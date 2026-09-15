use crate::services::question::{Answer, OptionItem, Question, MAX_OTHER};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WizardOutcome {
    Continue,
    Submit,
    Cancel,
}

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
            toggles: questions
                .iter()
                .map(|q| vec![false; q.options.len()])
                .collect(),
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
    pub fn on_other(&self) -> bool {
        self.current()
            .map(|q| self.opt_idx >= q.options.len())
            .unwrap_or(false)
    }
    pub fn is_toggled(&self, option: usize) -> bool {
        self.toggles
            .get(self.idx)
            .and_then(|t| t.get(option))
            .copied()
            .unwrap_or(false)
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
        let Some(question) = self.current() else {
            return;
        };
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
        let Some(buf) = self.other.get_mut(self.idx) else {
            return;
        };
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
    pub fn back(&mut self) -> WizardOutcome {
        if self.idx == 0 {
            return WizardOutcome::Continue;
        }
        self.store_current();
        self.idx -= 1;
        self.load_current();
        WizardOutcome::Continue
    }
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
        let Some(question) = self.questions.get(self.idx).cloned() else {
            return;
        };
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
    use crate::services::question::{Answer, OptionItem, Question};

    fn question(header: &str, multi_select: bool, options: &[&str]) -> Question {
        Question {
            header: Some(header.to_string()),
            question: "Pick one".to_string(),
            multi_select,
            options: options
                .iter()
                .map(|l| OptionItem {
                    label: l.to_string(),
                    description: None,
                })
                .collect(),
        }
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
        assert_eq!(
            wizard.answers()[0].selected,
            vec!["a".to_string(), "c".to_string()]
        );
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
