use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};
use std::time::Duration;
use tokio::sync::oneshot;

pub const MAX_QUESTIONS: usize = 4;
pub const MAX_OPTIONS: usize = 6;
const MAX_HEADER: usize = 24;
const MAX_QUESTION: usize = 300;
const MAX_LABEL: usize = 60;
const MAX_DESCRIPTION: usize = 120;
const MAX_OTHER: usize = 200;

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionItem {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub header: Option<String>,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<OptionItem>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    pub selected: Vec<String>,
    pub other: Option<String>,
}

/// Parse an `ask_user` tool payload into validated questions.
/// Over-long fields are truncated; questions without a body or options are dropped.
pub fn parse_questions(args: &Value) -> Vec<Question> {
    let Some(raw_questions) = args.get("questions").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for raw in raw_questions.iter().take(MAX_QUESTIONS) {
        let question = raw.get("question").and_then(|v| v.as_str()).unwrap_or("").trim();
        if question.is_empty() {
            continue;
        }
        let mut options = Vec::new();
        if let Some(raw_options) = raw.get("options").and_then(|v| v.as_array()) {
            for raw_option in raw_options.iter().take(MAX_OPTIONS) {
                let label = raw_option.get("label").and_then(|v| v.as_str()).unwrap_or("").trim();
                if label.is_empty() {
                    continue;
                }
                let description = raw_option
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|d| truncate_chars(d.trim(), MAX_DESCRIPTION))
                    .filter(|d| !d.is_empty());
                options.push(OptionItem { label: truncate_chars(label, MAX_LABEL), description });
            }
        }
        if options.is_empty() {
            continue;
        }
        let header = raw
            .get("header")
            .and_then(|v| v.as_str())
            .map(|h| truncate_chars(h.trim(), MAX_HEADER))
            .filter(|h| !h.is_empty());
        out.push(Question {
            header,
            question: truncate_chars(question, MAX_QUESTION),
            multi_select: raw.get("multi_select").and_then(|v| v.as_bool()).unwrap_or(false),
            options,
        });
    }
    out
}

fn question_label(question: &Question, index: usize) -> String {
    question.header.clone().unwrap_or_else(|| format!("Q{}", index + 1))
}

pub fn format_answers(questions: &[Question], answers: &[Answer]) -> String {
    let mut lines = vec!["User answered:".to_string()];
    for (i, question) in questions.iter().enumerate() {
        let answer = answers.get(i).cloned().unwrap_or_default();
        let mut parts = answer.selected;
        if let Some(other) = answer.other.filter(|o| !o.trim().is_empty()) {
            parts.push(format!("Other: {}", other.trim()));
        }
        let rendered = if parts.is_empty() { "(no answer)".to_string() } else { parts.join(", ") };
        lines.push(format!("- {}: {}", question_label(question, i), rendered));
    }
    lines.join("\n")
}

pub fn skipped_message(questions: &[Question]) -> String {
    let mut out = String::from("User skipped the question(s) — proceed with your best judgment.");
    if !questions.is_empty() {
        out.push_str("\nUnanswered:");
        for (i, question) in questions.iter().enumerate() {
            out.push_str(&format!("\n- {}: {}", question_label(question, i), question.question));
        }
    }
    out
}

pub struct AskRequest {
    pub questions: Vec<Question>,
    pub tx: Option<oneshot::Sender<Vec<Answer>>>,
}

static PENDING: OnceLock<Mutex<VecDeque<AskRequest>>> = OnceLock::new();
static INTERACTIVE: AtomicBool = AtomicBool::new(false);

fn pending_lock() -> &'static Mutex<VecDeque<AskRequest>> {
    PENDING.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Mark whether a TUI is attached. Without one, `ask` skips immediately.
pub fn set_interactive(v: bool) {
    INTERACTIVE.store(v, Ordering::Relaxed);
}

pub fn is_interactive() -> bool {
    INTERACTIVE.load(Ordering::Relaxed)
}

/// Ask the user `questions` and block until answered (tool-side entrypoint).
/// Returns a formatted answer string, or the skip message on cancel/no UI/timeout.
pub async fn ask(questions: Vec<Question>) -> String {
    if questions.is_empty() {
        return "Error: ask_user requires at least one question with options.".to_string();
    }
    if !is_interactive() {
        return skipped_message(&questions);
    }
    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending_lock().lock().unwrap();
        lock.push_back(AskRequest { questions: questions.clone(), tx: Some(tx) });
    }
    match tokio::time::timeout(Duration::from_secs(300), rx).await {
        Ok(Ok(answers)) if !answers.is_empty() => format_answers(&questions, &answers),
        _ => skipped_message(&questions),
    }
}

/// TUI side: take the next pending question set (FIFO, non-blocking).
pub fn take_pending() -> Option<AskRequest> {
    pending_lock().lock().unwrap().pop_front()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WizardOutcome {
    Continue,
    Submit,
    Cancel,
}

/// Pure, terminal-agnostic state machine for the one-question-at-a-time wizard.
/// The cursor ranges over `0..=options.len()`, where the final row is "Other".
#[derive(Debug, Clone)]
pub struct Wizard {
    questions: Vec<Question>,
    idx: usize,
    opt_idx: usize,
    toggles: Vec<Vec<bool>>,
    other: Vec<String>,
    answers: Vec<Answer>,
}

impl Wizard {
    pub fn new(questions: Vec<Question>) -> Self {
        let count = questions.len();
        let mut wizard = Self {
            toggles: questions.iter().map(|q| vec![false; q.options.len()]).collect(),
            questions,
            idx: 0,
            opt_idx: 0,
            other: vec![String::new(); count],
            answers: vec![Answer::default(); count],
        };
        wizard.load_current();
        wizard
    }

    pub fn total(&self) -> usize {
        self.questions.len()
    }

    pub fn idx(&self) -> usize {
        self.idx
    }

    pub fn opt_idx(&self) -> usize {
        self.opt_idx
    }

    pub fn current(&self) -> Option<&Question> {
        self.questions.get(self.idx)
    }

    /// True when the cursor sits on the always-present "Other…" row.
    pub fn on_other(&self) -> bool {
        self.current().map(|q| self.opt_idx >= q.options.len()).unwrap_or(false)
    }

    pub fn is_toggled(&self, option: usize) -> bool {
        self.toggles.get(self.idx).and_then(|t| t.get(option)).copied().unwrap_or(false)
    }

    pub fn other_text(&self) -> &str {
        self.other.get(self.idx).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn answers(&self) -> Vec<Answer> {
        self.answers.clone()
    }

    pub fn move_up(&mut self) {
        self.opt_idx = self.opt_idx.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let last = self.current().map(|q| q.options.len()).unwrap_or(0);
        if self.opt_idx < last {
            self.opt_idx += 1;
        }
    }

    pub fn toggle(&mut self) {
        let Some(question) = self.current() else { return };
        if !question.multi_select || self.on_other() {
            return;
        }
        if let Some(row) = self.toggles.get_mut(self.idx) {
            if let Some(cell) = row.get_mut(self.opt_idx) {
                *cell = !*cell;
            }
        }
    }

    pub fn push_char(&mut self, c: char) {
        if !self.on_other() {
            return;
        }
        let Some(buf) = self.other.get_mut(self.idx) else { return };
        if buf.chars().count() >= MAX_OTHER || c.is_control() {
            return;
        }
        buf.push(c);
    }

    pub fn backspace(&mut self) {
        if !self.on_other() {
            return;
        }
        if let Some(buf) = self.other.get_mut(self.idx) {
            buf.pop();
        }
    }

    /// Step back to the previous question, preserving the current draft.
    pub fn back(&mut self) -> WizardOutcome {
        if self.idx == 0 {
            return WizardOutcome::Continue;
        }
        self.store_current();
        self.idx -= 1;
        self.load_current();
        WizardOutcome::Continue
    }

    /// Confirm the current question and advance, or submit when it was the last.
    pub fn confirm(&mut self) -> WizardOutcome {
        self.store_current();
        if self.idx + 1 >= self.questions.len() {
            return WizardOutcome::Submit;
        }
        self.idx += 1;
        self.load_current();
        WizardOutcome::Continue
    }

    pub fn cancel(&self) -> WizardOutcome {
        WizardOutcome::Cancel
    }

    fn store_current(&mut self) {
        let Some(question) = self.questions.get(self.idx).cloned() else { return };
        let mut selected = Vec::new();
        let mut other = None;
        if question.multi_select {
            for (i, option) in question.options.iter().enumerate() {
                if self.is_toggled(i) {
                    selected.push(option.label.clone());
                }
            }
            let text = self.other_text().trim();
            if !text.is_empty() {
                other = Some(text.to_string());
            }
        } else if self.on_other() {
            let text = self.other_text().trim();
            if !text.is_empty() {
                other = Some(text.to_string());
            }
        } else if let Some(option) = question.options.get(self.opt_idx) {
            selected.push(option.label.clone());
        }
        if let Some(slot) = self.answers.get_mut(self.idx) {
            *slot = Answer { selected, other };
        }
    }

    fn load_current(&mut self) {
        let Some(question) = self.questions.get(self.idx).cloned() else {
            self.opt_idx = 0;
            return;
        };
        let answer = self.answers.get(self.idx).cloned().unwrap_or_default();
        if question.multi_select {
            for (i, option) in question.options.iter().enumerate() {
                let on = answer.selected.iter().any(|s| s == &option.label);
                if let Some(row) = self.toggles.get_mut(self.idx) {
                    if let Some(cell) = row.get_mut(i) {
                        *cell = on;
                    }
                }
            }
            if let Some(text) = &answer.other {
                if let Some(buf) = self.other.get_mut(self.idx) {
                    *buf = text.clone();
                }
            }
            self.opt_idx = 0;
        } else if answer.other.is_some() {
            self.opt_idx = question.options.len();
        } else if let Some(pos) = answer
            .selected
            .first()
            .and_then(|s| question.options.iter().position(|o| &o.label == s))
        {
            self.opt_idx = pos;
        } else {
            self.opt_idx = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question(header: &str, multi_select: bool, options: &[&str]) -> Question {
        Question {
            header: Some(header.to_string()),
            question: "Pick one".to_string(),
            multi_select,
            options: options
                .iter()
                .map(|l| OptionItem { label: l.to_string(), description: None })
                .collect(),
        }
    }

    #[test]
    fn parse_clamps_question_and_option_counts() {
        let options: Vec<Value> = (0..10).map(|i| json!({"label": format!("opt{i}")})).collect();
        let questions: Vec<Value> = (0..6)
            .map(|i| json!({"question": format!("q{i}"), "options": options}))
            .collect();
        let parsed = parse_questions(&json!({"questions": questions}));
        assert_eq!(parsed.len(), MAX_QUESTIONS);
        assert_eq!(parsed[0].options.len(), MAX_OPTIONS);
        assert!(!parsed[0].multi_select);
    }

    #[test]
    fn parse_drops_questions_without_options() {
        let parsed = parse_questions(&json!({
            "questions": [
                {"question": "no options", "options": []},
                {"question": "valid", "options": [{"label": "a"}]}
            ]
        }));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].question, "valid");
    }

    #[test]
    fn format_answers_renders_single_multi_and_other() {
        let questions = vec![
            question("Scope", false, &["small", "large"]),
            question("Extras", true, &["a", "b"]),
            question("Name", false, &["x"]),
        ];
        let answers = vec![
            Answer { selected: vec!["small".into()], other: None },
            Answer { selected: vec!["a".into(), "b".into()], other: Some("other thing".into()) },
            Answer::default(),
        ];
        let text = format_answers(&questions, &answers);
        assert!(text.contains("- Scope: small"));
        assert!(text.contains("- Extras: a, b, Other: other thing"));
        assert!(text.contains("- Name: (no answer)"));
    }

    #[test]
    fn skipped_message_lists_unanswered_questions() {
        let questions = vec![question("Scope", false, &["small"])];
        let text = skipped_message(&questions);
        assert!(text.contains("skipped"));
        assert!(text.contains("- Scope: Pick one"));
    }

    #[tokio::test]
    async fn ask_without_tui_skips_immediately() {
        set_interactive(false);
        let questions = vec![question("Scope", false, &["small"])];
        let result = ask(questions).await;
        assert!(result.contains("skipped"));
    }

    #[test]
    fn wizard_single_select_confirms_and_advances() {
        let mut wizard = Wizard::new(vec![
            question("One", false, &["a", "b"]),
            question("Two", false, &["c"]),
        ]);
        wizard.move_down();
        assert_eq!(wizard.confirm(), WizardOutcome::Continue);
        assert_eq!(wizard.answers()[0].selected, vec!["b".to_string()]);
        assert_eq!(wizard.confirm(), WizardOutcome::Submit);
        assert_eq!(wizard.answers()[1].selected, vec!["c".to_string()]);
    }

    #[test]
    fn wizard_multi_select_toggles_and_records_all() {
        let mut wizard = Wizard::new(vec![question("Extras", true, &["a", "b", "c"])]);
        wizard.toggle();
        wizard.move_down();
        wizard.move_down();
        wizard.toggle();
        assert_eq!(wizard.confirm(), WizardOutcome::Submit);
        assert_eq!(wizard.answers()[0].selected, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn wizard_typing_other_records_free_text() {
        let mut wizard = Wizard::new(vec![question("Name", false, &["x"])]);
        wizard.move_down();
        assert!(wizard.on_other());
        for c in "custom".chars() {
            wizard.push_char(c);
        }
        wizard.backspace();
        assert_eq!(wizard.other_text(), "custo");
        assert_eq!(wizard.confirm(), WizardOutcome::Submit);
        assert!(wizard.answers()[0].selected.is_empty());
        assert_eq!(wizard.answers()[0].other.as_deref(), Some("custo"));
    }

    #[test]
    fn wizard_back_preserves_prior_answers() {
        let mut wizard = Wizard::new(vec![
            question("One", false, &["a", "b"]),
            question("Two", false, &["c", "d"]),
        ]);
        wizard.move_down();
        wizard.confirm();
        wizard.move_down();
        wizard.back();
        assert_eq!(wizard.idx(), 0);
        assert_eq!(wizard.opt_idx(), 1);
        assert_eq!(wizard.answers()[0].selected, vec!["b".to_string()]);
    }
}
