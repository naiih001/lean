use crate::agent::{self, AgentEvent};
use crate::theme::{ASHEN, THEME};
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use ratatui_textarea::{CursorMove, Input as TAInput, Key as TAKey, TextArea};
use std::io::Stdout;
use tokio::sync::broadcast;

mod markdown;

#[derive(Debug, Clone)]
pub struct RunOpts {
    pub model: String,
    pub continue_session: bool,
    pub resume_id: Option<String>,
    pub no_session: bool,
    pub bash_guard_disabled: bool,
    pub dir_guard_disabled: bool,
}

pub async fn run(opts: RunOpts) -> anyhow::Result<()> {
    // propagate guards globally
    crate::bash_guard::set_disabled(opts.bash_guard_disabled);
    crate::dir_guard::set_disabled(opts.dir_guard_disabled);
    crate::dir_guard::init(None);
    let _model = opts.model.clone();

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    // Best-effort: enable the kitty keyboard protocol so Shift+Enter (and other
    // modified keys) are distinguishable. Unsupported terminals ignore the
    // sequence; we skip the capability probe to avoid a blocking query.
    let _ = crossterm::execute!(
        stdout,
        crossterm::event::PushKeyboardEnhancementFlags(
            crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        )
    );
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let res = app_loop(&mut terminal, opts).await;

    let _ = crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::PopKeyboardEnhancementFlags
    );
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("TUI error: {}", e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_prefers_word_boundary_and_drops_break_space() {
        assert_eq!(hard_wrap_str("hello world foo", 10), "hello\nworld foo");
    }

    #[test]
    fn wrap_falls_back_to_exact_column_without_stripping_chars() {
        assert_eq!(hard_wrap_str("abcdefghij", 4), "abcd\nefgh\nij");
    }

    #[test]
    fn wrap_measures_display_width_for_wide_chars() {
        // "犬" and "猫" are two columns each, so only "ab犬" (four columns) fits.
        // A char-counting wrap would wrongly keep all four chars on one line.
        assert_eq!(hard_wrap_str("ab犬猫", 4), "ab犬\n猫");
        assert_eq!(hard_wrap_str("ab犬猫", 3), "ab\n犬\n猫");
    }

    #[test]
    fn wrap_leaves_fitting_text_untouched() {
        assert_eq!(hard_wrap_str("hello", 10), "hello");
        assert_eq!(hard_wrap_str("ab\ncd", 10), "ab\ncd");
    }

    #[test]
    fn wrap_cursor_line_wraps_tail_typing() {
        let mut ta = TextArea::from(["hello world"]);
        ta.move_cursor(CursorMove::End);
        wrap_cursor_line(&mut ta, 6);
        assert_eq!(ta.lines().to_vec(), vec!["hello", "world"]);
        assert_eq!(ta.cursor(), (1, 5));
    }

    #[test]
    fn wrap_cursor_line_ignores_cursor_away_from_line_end() {
        let mut ta = TextArea::from(["hello world"]);
        ta.move_cursor(CursorMove::Head);
        wrap_cursor_line(&mut ta, 6);
        assert_eq!(ta.lines().to_vec(), vec!["hello world"]);
    }

    #[test]
    fn wrap_cursor_line_uses_char_boundaries_for_long_tokens() {
        let mut ta = TextArea::from(["abcdefgh"]);
        ta.move_cursor(CursorMove::End);
        wrap_cursor_line(&mut ta, 4);
        assert_eq!(ta.lines().to_vec(), vec!["abcd", "efgh"]);
    }
}

// ── Message model ──────────────────────────────────────────────

#[derive(Clone)]
struct Msg {
    role: String,
    content: String,
    tool_id: Option<String>,
    tool_name: Option<String>,
    tool_args: Option<String>,
    elapsed_ms: Option<u64>,
}

impl Msg {
    fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None }
    }
    fn new_tool_start(name: String, args: String, id: String) -> Self {
        Self { role: "tool".into(), content: format!("{} {}", name, args), tool_id: Some(id), tool_name: Some(name.clone()), tool_args: Some(args), elapsed_ms: None }
    }
    fn format_elapsed(&self) -> String {
        if let Some(ms) = self.elapsed_ms {
            if ms < 1000 { format!("{}ms", ms) } else { format!("{:.2}s", ms as f64 / 1000.0) }
        } else { String::new() }
    }
}

impl Msg {
    /// Render this message as styled ratatui Lines.
    fn render_lines(&self) -> Vec<Line<'static>> {
        match self.role.as_str() {
            "user" => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        ">>",
                        Style::default()
                            .fg(ASHEN.slate)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " you",
                        Style::default()
                            .fg(ASHEN.slate)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])];
                for l in self.content.lines() {
                    // Highlight @file mentions in ember color
                    let mut spans: Vec<Span<'static>> = vec![Span::styled(
                        "     ".to_string(),
                        Style::default().fg(ASHEN.bone),
                    )];
                    let chars: Vec<char> = l.chars().collect();
                    let mut i = 0;
                    let mut buf = String::new();
                    let flush_buf = |spans: &mut Vec<Span<'static>>, buf: &mut String| {
                        if !buf.is_empty() {
                            spans.push(Span::styled(
                                std::mem::take(buf),
                                Style::default().fg(ASHEN.bone),
                            ));
                        }
                    };
                    while i < chars.len() {
                        if (chars[i] == '@' || chars[i] == '$')
                            && (i == 0 || chars[i - 1].is_whitespace() || "(\"'`".contains(chars[i - 1]))
                        {
                            let prefix_char = chars[i];
                            // for $ require next char to be letter to avoid $5 etc.
                            if prefix_char == '$' && i + 1 < chars.len() && !chars[i + 1].is_ascii_alphabetic() && chars[i+1] != '_' {
                                buf.push(chars[i]);
                                i += 1;
                                continue;
                            }
                            let mut j = i + 1;
                            while j < chars.len() && !chars[j].is_whitespace() {
                                j += 1;
                            }
                            if j > i + 1 {
                                flush_buf(&mut spans, &mut buf);
                                let m: String = chars[i..j].iter().collect();
                                let style = if prefix_char == '$' {
                                    Style::default().fg(ASHEN.moss).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(ASHEN.ember).add_modifier(Modifier::BOLD)
                                };
                                spans.push(Span::styled(m, style));
                                i = j;
                                continue;
                            }
                        }
                        buf.push(chars[i]);
                        i += 1;
                    }
                    flush_buf(&mut spans, &mut buf);
                    lines.push(Line::from(spans));
                }
                lines
            }
            "assistant" => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        "lean",
                        Style::default().fg(ASHEN.moss).add_modifier(Modifier::BOLD),
                    ),
                ])];
                lines.extend(markdown::render_markdown(&self.content, 5));
                lines
            }
            "thinking" => {
                // Render thinking as a single collapsed block.
                // If there's content, show first line as preview.
                let first = self.content.lines().next().unwrap_or("");
                if first.is_empty() {
                    vec![Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            "...",
                            Style::default()
                                .fg(ASHEN.deep_ash)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ])]
                } else {
                    let preview: String = first.chars().take(80).collect();
                    vec![Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            "...",
                            Style::default()
                                .fg(ASHEN.deep_ash)
                                .add_modifier(Modifier::ITALIC),
                        ),
                        Span::styled(
                            format!(" {}", preview),
                            Style::default()
                                .fg(ASHEN.deep_ash)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ])]
                }
            }
            "tool" => { if self.tool_id.is_some() { self.render_tool_box() } else { self.render_tool_lines() } },
            "system" => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        format!("   {}", l),
                        Style::default().fg(ASHEN.ember),
                    ))
                })
                .collect(),
            _ => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(ASHEN.frost),
                    ))
                })
                .collect(),
        }
    }

    fn render_tool_box(&self) -> Vec<Line<'static>> {
        let name = self.tool_name.clone().unwrap_or_else(|| {
            if let Some(pos) = self.content.find(" → ") { self.content[..pos].trim().to_string() }
            else if let Some(pos) = self.content.find(' ') { self.content[..pos].to_string() }
            else { self.content.clone() }
        });
        let args_str = self.tool_args.clone().unwrap_or_else(|| {
            if let Some(pos) = self.content.find(' ') { self.content[pos..].trim().to_string() } else { String::new() }
        });
        let has_result = self.content.contains(" → ");
        let result = if has_result {
            if let Some(pos) = self.content.find(" → ") { self.content[pos + " → ".len()..].to_string() } else { String::new() }
        } else { String::new() };
        let is_running = self.elapsed_ms.is_none();
        let elapsed = self.format_elapsed();
        let is_error = result.contains("Error:") || result.contains("BLOCKED") || result.to_lowercase().contains("error");
        let border_col = if is_running { ASHEN.charcoal } else if is_error { ASHEN.ember } else { ASHEN.moss };
        let timer_str = if is_running { "… running".to_string() } else { elapsed.clone() };
        let header_title = if timer_str.is_empty() { name.clone() } else { format!("{} · {}", name, timer_str) };
        let mut inner: Vec<Line<'static>> = Vec::new();
        if !args_str.is_empty() && args_str != "{}" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&args_str) {
                if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let display = if let Some(rest) = path.strip_prefix(&home) { format!("~{}", rest) } else { path.to_string() };
                    inner.push(Line::from(Span::styled(display, Style::default().fg(ASHEN.smoke))));
                } else if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                    inner.push(Line::from(Span::styled(format!("$ {}", cmd), Style::default().fg(ASHEN.whisper))));
                } else if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                    inner.push(Line::from(Span::styled(format!("\"{}\"", q), Style::default().fg(ASHEN.smoke))));
                } else {
                    inner.push(Line::from(Span::styled(args_str.clone(), Style::default().fg(ASHEN.smoke))));
                }
            } else {
                inner.push(Line::from(Span::styled(args_str.clone(), Style::default().fg(ASHEN.smoke))));
            }
        }
        if has_result {
            inner.push(Line::from(Span::styled("─".repeat(28), Style::default().fg(ASHEN.charcoal))));
            let result_lines: Vec<&str> = result.lines().collect();
            let is_diff = name == "edit_file" || result.contains("diff:") || result.contains("@@");
            let max_lines = if is_diff { 12 } else { 3 };
            let show = result_lines.len().min(max_lines);
            for l in &result_lines[..show] {
                let style = if l.starts_with('+') && !l.starts_with("+++") { Style::default().fg(ASHEN.moss) }
                    else if l.starts_with('-') && !l.starts_with("---") { Style::default().fg(ASHEN.ember) }
                    else if l.starts_with("@@") { Style::default().fg(ASHEN.frost).add_modifier(Modifier::BOLD) }
                    else if is_diff { Style::default().fg(ASHEN.deep_ash) }
                    else { Style::default().fg(ASHEN.smoke) };
                inner.push(Line::from(Span::styled(l.to_string(), style)));
            }
            if result_lines.len() > max_lines {
                inner.push(Line::from(Span::styled(format!("… +{} more lines", result_lines.len() - max_lines), Style::default().fg(ASHEN.charcoal))));
            }
        } else if is_running {
            inner.push(Line::from(Span::styled("  ● running", Style::default().fg(ASHEN.charcoal).add_modifier(Modifier::ITALIC))));
        }
        let width_est: usize = 60;
        let top = format!("┌─ {} ─", header_title);
        let top_border = {
            let rem = width_est.saturating_sub(top.chars().count() + 1);
            format!("{}{}┐", top, "─".repeat(rem))
        };
        let bottom = format!("└{}┘", "─".repeat(width_est));
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled(top_border, Style::default().fg(border_col))));
        for l in inner {
            let mut spans = vec![Span::styled("│ ".to_string(), Style::default().fg(border_col))];
            spans.extend(l.spans);
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(Span::styled(bottom, Style::default().fg(border_col))));
        lines
    }

    /// Render tool messages compactly.
    fn render_tool_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(pos) = self.content.find(" → ") {
            // Tool result: "tool_name → result"
            let name = self.content[..pos].trim();
            let result = &self.content[pos + " → ".len()..];

            // Truncate long results — show more for diffs (edit_file)
            let result_lines: Vec<&str> = result.lines().collect();
            let is_diff = name == "edit_file" || result.contains("diff:") || result.contains("@@");
            let max_lines = if is_diff { 12 } else { 3 };
            let show = result_lines.len().min(max_lines);

            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("▸", Style::default().fg(ASHEN.ember)),
                Span::styled(", ", Style::default().fg(ASHEN.charcoal)),
                Span::styled(
                    name.to_string(),
                    Style::default()
                        .fg(ASHEN.ember_glow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            for l in &result_lines[..show] {
                // Diff coloring for builder-native feel
                let style = if l.starts_with('+') && !l.starts_with("+++") {
                    Style::default().fg(ASHEN.moss)
                } else if l.starts_with('-') && !l.starts_with("---") {
                    Style::default().fg(ASHEN.ember)
                } else if l.starts_with("@@") {
                    Style::default().fg(ASHEN.frost).add_modifier(Modifier::BOLD)
                } else if is_diff {
                    Style::default().fg(ASHEN.deep_ash)
                } else {
                    Style::default().fg(ASHEN.smoke)
                };
                lines.push(Line::from(Span::styled(
                    format!("      {}", l),
                    style,
                )));
            }
            if result_lines.len() > max_lines {
                lines.push(Line::from(Span::styled(
                    format!("      … +{} more lines", result_lines.len() - max_lines),
                    Style::default().fg(ASHEN.charcoal),
                )));
            }
        } else {
            // Tool invocation: "tool_name {json}"
            let first_space = self.content.find(' ');
            if let Some(pos) = first_space {
                let name = &self.content[..pos];
                let args_str = self.content[pos..].trim();

                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("▸", Style::default().fg(ASHEN.ember)),
                    Span::styled(", ", Style::default().fg(ASHEN.charcoal)),
                    Span::styled(
                        name.to_string(),
                        Style::default()
                            .fg(ASHEN.ember_glow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                if let Ok(v) = serde_json::from_str::<serde_json::Value>(args_str) {
                    if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                        let home = std::env::var("HOME").unwrap_or_default();
                        let display = if let Some(rest) = path.strip_prefix(&home) {
                            format!("~{}", rest)
                        } else {
                            path.to_string()
                        };
                        lines.push(Line::from(Span::styled(
                            format!("      {}", display),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("      $ {}", cmd),
                            Style::default().fg(ASHEN.whisper),
                        )));
                    } else if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("      \"{}\"", q),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("      {}", args_str),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("      {}", args_str),
                        Style::default().fg(ASHEN.smoke),
                    )));
                }
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines
    }
}

/// Merge all consecutive/adjacent thinking messages into one.
/// This fixes fragmented thinking from interleaved reasoning + tool events.
fn merge_thinking(messages: &[Msg]) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::new();
    let mut thinking_buf = String::new();
    let mut in_thinking = false;

    for m in messages {
        if m.role == "thinking" {
            if in_thinking {
                thinking_buf.push_str(&m.content);
            } else {
                in_thinking = true;
                thinking_buf = m.content.clone();
            }
        } else {
            if in_thinking {
                // Flush accumulated thinking
                out.push(Msg::new("thinking", thinking_buf.clone()));
                thinking_buf.clear();
                in_thinking = false;
            }
            out.push(m.clone());
        }
    }
    // Flush trailing thinking
    if in_thinking && !thinking_buf.is_empty() {
        out.push(Msg::new("thinking", thinking_buf));
    }
    out
}

// ── Rendering helpers ──────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, _model: &str, step_info: &str) {
    let inner = area;
    let width = inner.width as usize;

    // Left: app name
    let left = " lean ";
    // Right: model + step
    let right = if step_info.is_empty() {
        format!("")
    } else {
        format!("{}", step_info)
    };
    let gap = width.saturating_sub(left.len() + right.len());
    let filler = " ".repeat(gap);
    let title = format!("{}{}{}", left, filler, right);

    let header = Paragraph::new(Line::from(Span::styled(
        title,
        Style::default()
            .fg(ASHEN.deep_ash)
            .bg(THEME.header_bg)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(header, inner);
}

fn draw_queue(f: &mut Frame, area: Rect, queue: &[String]) {
    let max_show = queue.len().min(2);
    let start = queue.len().saturating_sub(max_show);
    let visible = &queue[start..];
    let width = area.width as usize;

    let lines: Vec<Line<'static>> = visible
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let n = start + i + 1;
            let prefix = format!("  {:>2} queued  ", n);
            let display: String = msg
                .chars()
                .take(width.saturating_sub(prefix.chars().count()))
                .collect();
            Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg),
                ),
                Span::styled(
                    display,
                    Style::default().fg(ASHEN.deep_ash).bg(THEME.input_bg),
                ),
            ])
        })
        .collect();

    let para = Paragraph::new(lines).style(Style::default().bg(THEME.input_bg));
    f.render_widget(para, area);
}

fn draw_separator(f: &mut Frame, area: Rect) {
    let sep = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(THEME.separator).bg(THEME.page_bg),
    )));
    f.render_widget(sep, area);
}

/// Wrap lines to fit within a given width.
fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::new();
    for line in lines {
        // Calculate total text width of this line
        let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        if total <= width {
            out.push(line);
            continue;
        }
        // Split into segments that fit
        let mut remaining = width;
        let mut seg_spans: Vec<Span<'static>> = Vec::new();
        for span in &line.spans {
            let chars: Vec<char> = span.content.chars().collect();
            let mut pos = 0;
            while pos < chars.len() && remaining > 0 {
                let take = chars.len().min(pos + remaining);
                let chunk: String = chars[pos..take].iter().collect();
                seg_spans.push(Span::styled(chunk, span.style));
                remaining -= take - pos;
                pos = take;
                if remaining == 0 {
                    out.push(Line::from(seg_spans));
                    seg_spans = Vec::new();
                    remaining = width;
                }
            }
        }
        if !seg_spans.is_empty() {
            out.push(Line::from(seg_spans));
        }
    }
    out
}

fn is_banner(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("mcp server running on stdio") || l.contains("github mcp server running on stdio")
}

/// Build the full list of styled lines from messages (without wrapping).
fn build_content_lines(messages: &[Msg]) -> Vec<Line<'static>> {
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    // Filter out server banner noise anywhere — user asked to never show it
    let filtered: Vec<Msg> = messages.iter().filter(|m| !is_banner(&m.content)).cloned().collect();
    let merged = merge_thinking(&filtered);

    for (i, m) in merged.iter().enumerate() {
        // Add a thin separator between tool blocks and other messages
        if i > 0
            && (m.role == "tool" || merged[i - 1].role == "tool")
            && m.role != merged[i - 1].role
        {
            all_lines.push(Line::from(Span::styled(
                format!("   {}", "─".repeat(6)),
                Style::default().fg(ASHEN.charcoal),
            )));
        }
        all_lines.extend(m.render_lines());
        // Blank line between messages (but not after the last one)
        if i + 1 < merged.len() {
            all_lines.push(Line::from(""));
        }
    }

    // If empty, show a welcome line
    if messages.is_empty() {
        all_lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "    lean",
                Style::default().fg(ASHEN.moss).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "    light coding assistant",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "    type a message below. /help for commands.",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(""),
        ];
    }

    all_lines
}

fn crossterm_key_to_input(k: crossterm::event::KeyEvent) -> TAInput {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let key = match k.code {
        KeyCode::Char(c) => TAKey::Char(c),
        KeyCode::Backspace => TAKey::Backspace,
        KeyCode::Enter => TAKey::Enter,
        KeyCode::Left => TAKey::Left,
        KeyCode::Right => TAKey::Right,
        KeyCode::Up => TAKey::Up,
        KeyCode::Down => TAKey::Down,
        KeyCode::Tab => TAKey::Tab,
        KeyCode::BackTab => TAKey::Tab,
        KeyCode::Delete => TAKey::Delete,
        KeyCode::Home => TAKey::Home,
        KeyCode::End => TAKey::End,
        KeyCode::PageUp => TAKey::PageUp,
        KeyCode::PageDown => TAKey::PageDown,
        KeyCode::Esc => TAKey::Esc,
        KeyCode::F(n) => TAKey::F(n),
        _ => TAKey::Null,
    };
    TAInput {
        key,
        ctrl,
        alt,
        shift,
    }
}

// ── Hard wrap ──────────────────────────────────────────────────

fn char_display_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0)
}

fn line_display_width(chars: &[char]) -> usize {
    chars.iter().map(|&c| char_display_width(c)).sum()
}

/// Choose where to split an over-long line so the first part fits `max_cols`
/// columns. Prefers the last space that fits (the space is consumed by the
/// break), falling back to an exact column split at a char boundary.
/// Returns `(split_index, drop_space)`.
fn wrap_split(chars: &[char], max_cols: usize) -> (usize, bool) {
    let mut width = 0usize;
    let mut fitting = 0usize;
    let mut last_space: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        let w = char_display_width(c);
        if width + w > max_cols {
            break;
        }
        width += w;
        fitting = i + 1;
        if c == ' ' && i >= 1 {
            last_space = Some(i);
        }
    }
    match last_space {
        Some(i) => (i, true),
        None => (fitting.max(1), false),
    }
}

/// Insert hard newlines so the cursor's line stops exceeding `max_cols` columns.
/// Only acts when the cursor sits at the end of its line (the tail-typing case
/// that fills the input box).
fn wrap_cursor_line(ta: &mut TextArea<'_>, max_cols: usize) {
    if max_cols == 0 {
        return;
    }
    loop {
        let cur = ta.cursor();
        let (row, col) = (cur.0, cur.1);
        let chars: Vec<char> = ta.lines()[row].chars().collect();
        if col != chars.len() || line_display_width(&chars) <= max_cols {
            break;
        }
        let (split, drop_space) = wrap_split(&chars, max_cols);
        if split == 0 || split >= chars.len() {
            break;
        }
        ta.move_cursor(CursorMove::Jump(row as u16, split as u16));
        if drop_space {
            ta.delete_next_char();
        }
        ta.insert_newline();
        ta.move_cursor(CursorMove::End);
    }
}

/// Hard-wrap text before insertion (e.g. bracketed paste): each logical line is
/// wrapped independently at `max_cols` columns.
fn hard_wrap_str(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return text.to_string();
    }
    text.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut out = String::new();
            let mut rest: Vec<char> = line.chars().collect();
            while line_display_width(&rest) > max_cols {
                let (split, drop_space) = wrap_split(&rest, max_cols);
                if split == 0 || split >= rest.len() {
                    break;
                }
                out.extend(rest[..split].iter());
                out.push('\n');
                let skip = if drop_space { split + 1 } else { split };
                rest = rest[skip.min(rest.len())..].to_vec();
            }
            out.extend(rest.iter());
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn draw_input(f: &mut Frame, area: Rect, textarea: &mut TextArea<'_>) {
    // Apply ash styling every frame (cheap)
    textarea.set_style(Style::default().fg(ASHEN.bone).bg(THEME.input_bg));
    textarea.set_cursor_style(
        Style::default()
            .fg(ASHEN.bone)
            .bg(ASHEN.ember)
            .add_modifier(Modifier::REVERSED),
    );
    textarea.set_cursor_line_style(Style::default().bg(THEME.input_bg));
    textarea.set_placeholder_text(
        "  ▸  type a message…  (/help • Enter send • Shift+Enter newline)",
    );
    textarea.set_placeholder_style(Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg));
    // prompt gutter: we prepend via block title style instead of manual truncation
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(THEME.input_bg));
    textarea.set_block(block);

    // Render textarea as widget (single-line height, multiline expands via scrolling)
    f.render_widget(&*textarea, area);
}

fn draw_footer(
    f: &mut Frame,
    area: Rect,
    model: &str,
    msg_count: usize,
    cwd: &str,
    agent_busy: bool,
    spinner_tick: usize,
) {
    // Always clear footer area first to avoid ghosting when popup was over it
    f.render_widget(Clear, area);
    let width = area.width as usize;
    // Footer never shows MCP status — check via /mcp only (as requested)
    // Narrow terminal (<10 cols) – just show truncated model to avoid overflow/wrap bleed
    if width < 15 {
        let txt = if model.chars().count() > width.saturating_sub(2) {
            format!(" {}… ", model.chars().take(width.saturating_sub(3)).collect::<String>())
        } else {
            format!(" {} ", model)
        };
        let para = Paragraph::new(Line::from(Span::styled(txt, Style::default().fg(ASHEN.smoke).bg(THEME.page_bg))));
        f.render_widget(para, area);
        return;
    }

    let spinner = [
        "\u{280b}", "\u{2819}", "\u{2813}", "\u{2827}", "\u{2836}", "\u{2834}", "\u{2826}",
        "\u{282e}",
    ];
    let spinner_char = if agent_busy {
        format!("{} ", spinner[spinner_tick % spinner.len()])
    } else {
        String::new()
    };
    let right = format!(" {} msgs ", msg_count);

    // Shorten cwd to show last 2 components
    let short_cwd = {
        let parts: Vec<&str> = cwd
            .rsplit('/')
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            parts.join("/")
        }
    };
    let center = format!(" {} ", short_cwd);

    let used = spinner_char.len() + 1 + model.len() + 2 + center.len() + right.len();
    let gap = if used < width { width - used } else { 0 };
    let gap_left = gap / 2;
    let gap_right = gap - gap_left;

    let mut spans = vec![Span::styled(
        format!(" {} ", model),
        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
    )];
    if !spinner_char.is_empty() {
        spans.insert(
            0,
            Span::styled(
                spinner_char,
                Style::default()
                    .fg(ASHEN.bone)
                    .bg(THEME.page_bg)
                    .add_modifier(Modifier::BOLD),
            ),
        );
    }
    spans.push(Span::styled(
        " ".repeat(gap_left),
        Style::default().bg(THEME.page_bg),
    ));
    spans.push(Span::styled(
        center,
        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
    ));
    spans.push(Span::styled(
        " ".repeat(gap_right),
        Style::default().bg(THEME.page_bg),
    ));
    spans.push(Span::styled(
        right,
        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
    ));

    let footer = Paragraph::new(Line::from(spans));
    f.render_widget(footer, area);
}

// ── Autocomplete + @-mentions ─────────────────────────────────

const COMMANDS: &[&str] = &["/help", "/new", "/clear", "/exit", "/quit", "/model", "/sessions", "/resume", "/allowlist", "/allowlist clear", "/mcp", "/memory", "/memory stats", "/memory consolidate"];

/// Filter commands matching the current input prefix.
/// Also completes model aliases after `/model `.
fn autocomplete_matches(input: &str) -> Vec<String> {
    if input.is_empty() || !input.starts_with('/') {
        return Vec::new();
    }
    // Model alias completion: "/model " or "/model <prefix>"
    if input.starts_with("/model ") {
        let prefix = input.strip_prefix("/model ").unwrap_or("");
        if let Ok(cfg) = crate::models::load() {
            let mut aliases: Vec<String> = cfg.models.keys().cloned().collect();
            aliases.sort();
            let filtered: Vec<String> = aliases
                .into_iter()
                .filter(|a| a.starts_with(prefix))
                .map(|a| format!("/model {}", a))
                .collect();
            return filtered;
        }
        return Vec::new();
    }
    // Handle "/model" partial -> command, but if user typed "/model" exactly and hits Tab we want alias list on next space
    // Normal command completion
    COMMANDS
        .iter()
        .filter(|cmd| cmd.starts_with(input))
        .map(|s| s.to_string())
        .collect()
}

// ── @-file mentions ─────────────────────────────────────────
#[derive(Debug, Clone)]
struct AtMention {
    prefix: String,
    row: usize,
    col: usize,
    at_col: usize,
}

fn detect_at_mention(textarea: &TextArea<'_>) -> Option<AtMention> {
    let c = textarea.cursor();
    let row = c.0;
    let col = c.1;
    let lines = textarea.lines();
    if row >= lines.len() {
        return None;
    }
    let line = &lines[row];
    let chars: Vec<char> = line.chars().collect();
    if col > chars.len() {
        return None;
    }
    // find last '@' before cursor with valid preceding boundary
    let mut at_col: Option<usize> = None;
    for i in (0..col).rev() {
        if chars[i] == '@' {
            let prev_ok = if i == 0 {
                true
            } else {
                let pc = chars[i - 1];
                pc.is_whitespace() || "(\"'`".contains(pc)
            };
            if prev_ok {
                at_col = Some(i);
                break;
            }
        }
        // stop scanning if we cross whitespace that already had no @? Keep searching for earlier @
    }
    let at = at_col?;
    let prefix_chars = &chars[at + 1..col];
    if prefix_chars.iter().any(|c| c.is_whitespace()) {
        return None;
    }
    let prefix: String = prefix_chars.iter().collect();
    Some(AtMention {
        prefix,
        row,
        col,
        at_col: at,
    })
}

fn collect_files() -> Vec<String> {
    let root = crate::dir_guard::project_root();
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if name == ".git" || name == "target" || name == "node_modules" || name == ".next" || name == "dist" || name == "build" {
                return false;
            }
            // skip hidden except .env-ish
            if name.starts_with('.') {
                return false;
            }
            true
        });
    for entry in walker.filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(rel) = entry.path().strip_prefix(&root) {
                let s = rel.display().to_string();
                if s.is_empty() {
                    continue;
                }
                files.push(s);
                if files.len() > 3000 {
                    break;
                }
            }
        }
    }
    files.sort();
    files
}

fn file_autocomplete_matches(prefix: &str) -> Vec<String> {
    let all = collect_files();
    if prefix.is_empty() {
        return all.into_iter().take(20).collect();
    }
    let lower = prefix.to_lowercase();
    let mut starts: Vec<String> = Vec::new();
    let mut contains: Vec<String> = Vec::new();
    for f in all {
        let fl = f.to_lowercase();
        if fl.starts_with(&lower) {
            starts.push(f);
        } else if fl.contains(&lower) {
            contains.push(f);
        }
        if starts.len() + contains.len() >= 20 {
            // keep collecting starts priority, but cap
            if starts.len() >= 20 {
                break;
            }
        }
    }
    starts.extend(contains);
    starts.truncate(20);
    starts
}

fn parse_skill_name(raw: &str) -> Option<String> {
    if !raw.starts_with("---") {
        return None;
    }
    let end = raw[3..].find("\n---").map(|i| i + 3)?;
    let fm = &raw[3..end];
    for line in fm.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == "name" {
                let val = v.trim().trim_matches('"').trim_matches('\'').trim().to_string();
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

fn list_skill_names_sync() -> Vec<String> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let bases = vec![
        cwd.join("skills"),
        cwd.join(".lean").join("skills"),
        home.join(".agents").join("skills"),
    ];
    let mut map: HashMap<String, String> = HashMap::new();
    for base in &bases {
        let is_local = base.starts_with(&cwd);
        if let Ok(rd) = std::fs::read_dir(base) {
            for entry in rd.filter_map(|e| e.ok()) {
                let ft = match entry.file_type() { Ok(ft) => ft, Err(_) => continue };
                if !ft.is_dir() { continue; }
                let skill_path = entry.path().join("SKILL.md");
                if !skill_path.exists() { continue; }
                let raw = std::fs::read_to_string(&skill_path).unwrap_or_default();
                let name = parse_skill_name(&raw).unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                if !map.contains_key(&name) || is_local {
                    map.insert(name.clone(), name);
                }
            }
        }
    }
    let mut v: Vec<String> = map.into_values().collect();
    v.sort();
    v
}

fn skill_autocomplete_matches(prefix: &str) -> Vec<String> {
    let all = list_skill_names_sync();
    if prefix.is_empty() {
        return all.into_iter().take(20).collect();
    }
    let lower = prefix.to_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for s in all {
        let sl = s.to_lowercase();
        if sl.starts_with(&lower) {
            starts.push(s);
        } else if sl.contains(&lower) {
            contains.push(s);
        }
    }
    starts.extend(contains);
    starts.truncate(20);
    starts
}

fn detect_skill_mention(textarea: &TextArea<'_>) -> Option<AtMention> {
    let c = textarea.cursor();
    let row = c.0;
    let col = c.1;
    let lines = textarea.lines();
    if row >= lines.len() { return None; }
    let line = &lines[row];
    let chars: Vec<char> = line.chars().collect();
    if col > chars.len() { return None; }
    let mut at_col: Option<usize> = None;
    for i in (0..col).rev() {
        if chars[i] == '$' {
            let prev_ok = if i == 0 { true } else { let pc = chars[i-1]; pc.is_whitespace() || "(\"'`".contains(pc) };
            if prev_ok {
                // require next char is alpha after $ if prefix empty? allow empty but check
                at_col = Some(i);
                break;
            }
        }
    }
    let at = at_col?;
    let prefix_chars = &chars[at + 1..col];
    if prefix_chars.iter().any(|c| c.is_whitespace()) { return None; }
    // prefix for skills should be alphanumeric/_- only; reject if contains other punctuation like @ prefix contains /
    // allow empty prefix to show all
    let prefix: String = prefix_chars.iter().collect();
    // filter out purely numeric or with symbols that can't be skill names
    if !prefix.is_empty() && prefix.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' )) {
        // still allow but we already filtered whitespace; keep it simple allow any but autocomplete will just not match
    }
    Some(AtMention { prefix, row, col, at_col: at })
}

fn find_skill_content(name: &str) -> Option<String> {
    use std::path::PathBuf;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let bases = vec![
        cwd.join("skills"),
        cwd.join(".lean").join("skills"),
        home.join(".agents").join("skills"),
    ];
    // scan with priority local
    let mut found: Option<PathBuf> = None;
    let mut found_is_local = false;
    for base in &bases {
        let is_local = base.starts_with(&cwd);
        if let Ok(rd) = std::fs::read_dir(base) {
            for entry in rd.filter_map(|e| e.ok()) {
                let ft = match entry.file_type() { Ok(ft) => ft, Err(_) => continue };
                if !ft.is_dir() { continue; }
                let skill_path = entry.path().join("SKILL.md");
                if !skill_path.exists() { continue; }
                let raw = std::fs::read_to_string(&skill_path).unwrap_or_default();
                let sname = parse_skill_name(&raw).unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                if sname == name {
                    if found.is_none() || (is_local && !found_is_local) {
                        found = Some(skill_path);
                        found_is_local = is_local;
                    }
                }
            }
        }
        let direct = base.join(name).join("SKILL.md");
        if direct.exists() {
            if found.is_none() || (is_local && !found_is_local) {
                found = Some(direct);
                found_is_local = is_local;
            }
        }
    }
    let path = found?;
    std::fs::read_to_string(path).ok()
}

fn current_completions(textarea: &TextArea<'_>) -> Vec<String> {
    if let Some(m) = detect_skill_mention(textarea) {
        return skill_autocomplete_matches(&m.prefix);
    }
    if let Some(m) = detect_at_mention(textarea) {
        return file_autocomplete_matches(&m.prefix);
    }
    let cur = textarea.lines().join("\n");
    autocomplete_matches(&cur)
}

fn expand_at_mentions(prompt: &str) -> String {
    // Find @<path> tokens that resolve to existing files and $skill tokens that force skills, appending contents.
    let root = crate::dir_guard::project_root();
    let mut files: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let chars: Vec<char> = prompt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            let prev_ok = if i == 0 { true } else { chars[i - 1].is_whitespace() || "(\"'`".contains(chars[i - 1]) };
            let next_is_alpha = i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_');
            if prev_ok && next_is_alpha {
                let mut j = i + 1;
                while j < chars.len() && !chars[j].is_whitespace() { j += 1; }
                let mut raw: String = chars[i + 1..j].iter().collect();
                while raw.ends_with(',') || raw.ends_with('.') || raw.ends_with(';') || raw.ends_with(':') || raw.ends_with('!') || raw.ends_with('?') || raw.ends_with(')') || raw.ends_with(']') || raw.ends_with('"') || raw.ends_with('\'') { raw.pop(); }
                // skill names allowed chars: alnum - _
                let cleaned: String = raw.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                let skill_name = if !cleaned.is_empty() { cleaned } else { raw.clone() };
                if !skill_name.is_empty() && !skills.contains(&skill_name) && find_skill_content(&skill_name).is_some() {
                    skills.push(skill_name);
                }
                i = j;
                continue;
            }
        }
        if chars[i] == '@' {
            let prev_ok = if i == 0 {
                true
            } else {
                chars[i - 1].is_whitespace() || "(\"'`".contains(chars[i - 1])
            };
            if prev_ok {
                let mut j = i + 1;
                while j < chars.len() && !chars[j].is_whitespace() {
                    j += 1;
                }
                let mut raw: String = chars[i + 1..j].iter().collect();
                // strip trailing punctuation that is unlikely part of path
                while raw.ends_with(',')
                    || raw.ends_with('.')
                    || raw.ends_with(';')
                    || raw.ends_with(':')
                    || raw.ends_with('!')
                    || raw.ends_with('?')
                    || raw.ends_with(')')
                    || raw.ends_with(']')
                    || raw.ends_with('"')
                    || raw.ends_with('\'')
                {
                    raw.pop();
                }
                if !raw.is_empty() && !files.contains(&raw) {
                    // check existence relative to root (or absolute)
                    let p = std::path::Path::new(&raw);
                    let full = if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        root.join(p)
                    };
                    if full.is_file() {
                        files.push(raw);
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if files.is_empty() && skills.is_empty() {
        return prompt.to_string();
    }
    let mut out = prompt.to_string();
    for rel in files {
        let p = std::path::Path::new(&rel);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        let content = std::fs::read_to_string(&full).unwrap_or_else(|e| format!("[read error: {}]", e));
        let truncated = if content.len() > 8000 {
            format!("{}… [truncated {} chars]", &content[..8000], content.len() - 8000)
        } else {
            content
        };
        let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("");
        out.push_str(&format!("\n\n[File: {}]\n```{}\n{}```", rel, ext, truncated));
    }
    for skill_name in skills {
        if let Some(content) = find_skill_content(&skill_name) {
            let truncated = if content.len() > 12000 { format!("{}… [truncated {} chars]", &content[..12000], content.len() - 12000) } else { content };
            out.push_str(&format!("\n\n[Forced skill: {} — follow its workflow explicitly]\n{}", skill_name, truncated));
        }
    }
    out
}

/// Draw a scrollable autocomplete popup above the input area.
fn draw_autocomplete(
    f: &mut Frame,
    input_area: Rect,
    matches: &[String],
    selected: usize,
    scroll_offset: usize,
) {
    if matches.is_empty() {
        return;
    }

    let max_cmd_len = matches.iter().map(|c| c.len()).max().unwrap_or(10);
    let popup_width = (max_cmd_len + 4) as u16;

    // Cap height to available space above input
    let max_visible = input_area.y.saturating_sub(1) as usize; // leave 1 row gap
    let visible_items = matches.len().min(max_visible.max(1));
    let popup_height = (visible_items + 2) as u16; // +2 for borders

    let x = input_area.x;
    let y = input_area.y.saturating_sub(popup_height);

    let area = Rect {
        x,
        y,
        width: popup_width.min(input_area.width),
        height: popup_height,
    };

    if area.height < 2 || area.width < 4 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.charcoal))
        .style(Style::default().bg(THEME.header_bg));

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    // Render only visible items
    let visible_h = inner.height as usize;
    let start = scroll_offset;
    let end = (start + visible_h).min(matches.len());

    let items: Vec<Line<'static>> = matches[start..end]
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let real_idx = start + i;
            let style = if real_idx == selected {
                Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            Line::from(Span::styled(format!(" {} ", cmd), style))
        })
        .collect();

    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}

fn draw_approval(f: &mut Frame, area: Rect, req: &crate::approval::ApprovalRequest) {
    let width = (area.width.saturating_sub(4)).min(90);
    let queued = crate::approval::queue_len();
    let queue_label = if queued > 0 { format!(" [{}/{}]", 1, queued + 1) } else { String::new() };
    let height = 9.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect { x, y, width, height };
    let is_dir = req.reasons.iter().any(|r| r.contains("outside CWD"));
    let is_mcp = req.reasons.iter().any(|r| r.contains("MCP tool"));
    let title = if is_mcp { format!(" MCP Guard — Approval Required{} ", queue_label) } else if is_dir { format!(" Dir Guard — Approval Required{} (outside CWD) ", queue_label) } else { format!(" Bash Guard — Approval Required{} ", queue_label) };
    let border_col = if is_mcp { ASHEN.moss } else if is_dir { ASHEN.frost } else { ASHEN.ember };
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let sev = match req.severity { crate::bash_guard::Severity::High => "HIGH", crate::bash_guard::Severity::Medium => "MEDIUM" };
    let sev_style = if sev == "HIGH" { Style::default().fg(ASHEN.ember).add_modifier(Modifier::BOLD) } else { Style::default().fg(ASHEN.frost) };
    let cmd_label = if is_mcp { format!("  MCP: {}", req.cmd) } else if is_dir && req.cmd.contains(' ') && !req.cmd.contains('/') { format!("  $ {}", req.cmd) } else if is_dir { format!("  path: {}", req.cmd) } else { format!("  $ {}", req.cmd) };
    let lines = vec![
        Line::from(vec![Span::styled(format!("  {} risk:  ", sev), sev_style), Span::styled(req.reasons.join("; "), Style::default().fg(ASHEN.smoke))]),
        Line::from(Span::styled(cmd_label, Style::default().fg(ASHEN.whisper))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [a] Allow once  ", Style::default().fg(ASHEN.moss).add_modifier(Modifier::BOLD)),
            Span::styled("[d] Deny  ", Style::default().fg(ASHEN.ember)),
            Span::styled("[A] Allow always  ", Style::default().fg(ASHEN.slate)),
            Span::styled("Esc deny", Style::default().fg(ASHEN.charcoal)),
        ]),
        Line::from(Span::styled("  Press a/d/A or Enter to allow, Esc to deny", Style::default().fg(ASHEN.deep_ash))),
    ];
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}


fn draw_sessions(f: &mut Frame, area: Rect, selected: usize, scroll: usize, filter: &str, show_all: bool, cwd: &str) {
    let all = crate::session::Session::list();
    // Per-directory filtering: default shows only sessions for current cwd, Tab toggles all
    let cwd_norm = cwd.trim_end_matches('/');
    let dir_filtered: Vec<crate::session::Session> = if show_all {
        all
    } else {
        all.into_iter().filter(|s| s.cwd.trim_end_matches('/') == cwd_norm).collect()
    };
    let filtered: Vec<crate::session::Session> = if filter.is_empty() {
        dir_filtered
    } else {
        let lower = filter.to_lowercase();
        dir_filtered.into_iter().filter(|s| {
            s.id.to_lowercase().contains(&lower)
                || s.cwd.to_lowercase().contains(&lower)
                || s.messages.iter().any(|m| m.content.to_lowercase().contains(&lower))
        }).collect()
    };
    let sessions = &filtered;
    let width = (area.width.saturating_sub(4)).min(80);
    let height = (14.min(sessions.len() + 6) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect { x, y, width, height };
    let mode_label = if show_all { "all" } else { "this dir" };
    let toggle_label = if show_all { "this dir" } else { "all" };
    let title = if filter.is_empty() {
        format!(" Sessions [{}] — Enter resume, Esc close, Tab:{} ", mode_label, toggle_label)
    } else {
        format!(" Sessions [{}] — filter: {} (Tab:{}) ", mode_label, filter, toggle_label)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.frost))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if sessions.is_empty() {
        let msg = if !filter.is_empty() {
            "  No match"
        } else if !show_all {
            "  No sessions in this dir — Tab for all"
        } else {
            "  No sessions"
        };
        let para = Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(ASHEN.deep_ash))));
        f.render_widget(para, inner);
        return;
    }
    let visible_h = inner.height as usize;
    let start = scroll.min(sessions.len().saturating_sub(1));
    let end = (start + visible_h).min(sessions.len());
    let items: Vec<Line> = sessions[start..end].iter().enumerate().map(|(i, sess)| {
        let idx = start + i;
        let sel = idx == selected;
        let style = if sel { Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD).bg(ASHEN.stone) } else { Style::default().fg(ASHEN.smoke) };
        let ts = sess.updated_at;
        let preview = sess.messages.first().map(|m| m.content.chars().take(30).collect::<String>().replace("\n", " ")).unwrap_or_else(|| "-".to_string());
        let id_short = sess.id.chars().take(8).collect::<String>();
        Line::from(Span::styled(format!(" {} {} {}m {} ", id_short, preview, sess.messages.len(), ts), style))
    }).collect();
    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}

fn draw_mcp(f: &mut Frame, area: Rect, selected: usize, scroll: usize) {
    let servers = crate::mcp::snapshot();
    let width = (area.width.saturating_sub(4)).min(90);
    let height = (18.min(servers.len() * 2 + 7) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect { x, y, width, height };
    let connected = servers.iter().filter(|s| matches!(s.status, crate::mcp::ServerStatus::Connected)).count();
    let total = servers.len();
    let global_off = crate::mcp::is_global_disabled();
    let mut title = if total == 0 {
        " MCP Servers — No servers configured ".to_string()
    } else if global_off {
        format!(" MCP Servers [GLOBAL OFF] ({}/{} connected) — g/enable, Esc close ", connected, total)
    } else {
        format!(" MCP Servers ({}/{} connected) — d/toggle, g/global, r/reconnect, Esc close ", connected, total)
    };
    // Truncate title to fit popup width for narrow terminals (<10 cols)
    if title.chars().count() > width as usize - 4 {
        title = format!("{}… ", title.chars().take(width as usize - 5).collect::<String>());
    }
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.moss))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if servers.is_empty() {
        let para = Paragraph::new(vec![
            Line::from(Span::styled("  No MCP servers configured", Style::default().fg(ASHEN.deep_ash))),
            Line::from(Span::styled("  Create ~/.lean/mcp.json or .lean/mcp.json", Style::default().fg(ASHEN.charcoal))),
            Line::from(Span::styled("  See mcp.json.example for format", Style::default().fg(ASHEN.charcoal))),
        ]);
        f.render_widget(para, inner);
        return;
    }
    let visible_h = inner.height as usize;
    let start = scroll.min(servers.len().saturating_sub(1));
    let end = (start + visible_h).min(servers.len());
    let mut lines: Vec<Line> = Vec::new();
    for (i, srv) in servers[start..end].iter().enumerate() {
        let idx = start + i;
        let sel = idx == selected;
        let style = if sel { Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD).bg(ASHEN.stone) } else { Style::default().fg(ASHEN.smoke) };
        let (status_str, status_style) = match srv.status {
            crate::mcp::ServerStatus::Connected => ("connected", Style::default().fg(ASHEN.moss)),
            crate::mcp::ServerStatus::Connecting => ("connecting", Style::default().fg(ASHEN.frost)),
            crate::mcp::ServerStatus::Error(_) => ("error", Style::default().fg(ASHEN.ember)),
            crate::mcp::ServerStatus::Disabled => ("disabled", Style::default().fg(ASHEN.charcoal)),
        };
        let tool_cnt = srv.tools.len();
        let header = Line::from(vec![
            Span::styled(format!("  {} ", srv.name), style),
            Span::styled(format!("[{}] ", status_str), status_style),
            Span::styled(format!("{} tools", tool_cnt), Style::default().fg(ASHEN.charcoal)),
        ]);
        lines.push(header);
        // detail line — truncated to inner width to avoid wrapping/bleed, especially for <10 cols
        let detail_max = inner.width.saturating_sub(4) as usize;
        let detail = if let Some(err) = &srv.error_detail {
            let t = err.chars().take(detail_max).collect::<String>();
            Span::styled(format!("    {}", t), Style::default().fg(ASHEN.ember))
        } else if srv.status.as_str() == "connected" && tool_cnt > 0 {
            let preview: Vec<String> = srv.tools.iter().take(3).map(|t| t.name.clone()).collect();
            let mut txt = preview.join(", ");
            if txt.chars().count() > detail_max { txt = format!("{}…", txt.chars().take(detail_max.saturating_sub(1)).collect::<String>()); }
            Span::styled(format!("    {}", txt), Style::default().fg(ASHEN.deep_ash))
        } else if let Some(url) = &srv.config.url {
            let t = url.chars().take(detail_max).collect::<String>();
            Span::styled(format!("    {}", t), Style::default().fg(ASHEN.deep_ash))
        } else if let Some(cmd) = &srv.config.command {
            let args = srv.config.args.clone().unwrap_or_default().join(" ");
            let full = format!("{} {}", cmd, args);
            let t = full.chars().take(detail_max).collect::<String>();
            Span::styled(format!("    {}", t), Style::default().fg(ASHEN.deep_ash))
        } else {
            Span::styled("    ".to_string(), Style::default().fg(ASHEN.deep_ash))
        };
        lines.push(Line::from(detail));
    }
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn draw_allowlist(f: &mut Frame, area: Rect, selected: usize, scroll: usize) {
    let bash_list = crate::bash_guard::allowlist_list();
    let dir_list = crate::dir_guard::allowlist_list();
    let mut combined: Vec<(String, String)> = Vec::new(); // (kind, pat)
    for p in bash_list { combined.push(("bash".into(), p)); }
    for p in dir_list { combined.push(("dir".into(), p)); }
    combined.sort_by(|a,b| a.1.cmp(&b.1));
    let width = (area.width.saturating_sub(4)).min(80);
    let height = (16.min(combined.len() + 7) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect { x, y, width, height };
    let title = format!(" Allowlist (bash:{}, dir:{}) — Enter keep, d delete, c clear all, Esc close ", crate::bash_guard::allowlist_list().len(), crate::dir_guard::allowlist_list().len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.moss))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if combined.is_empty() {
        let para = Paragraph::new(vec![
            Line::from(Span::styled("  No allowlisted patterns", Style::default().fg(ASHEN.deep_ash))),
            Line::from(Span::styled("  Approve a bash/dir command with 'A' to add", Style::default().fg(ASHEN.charcoal))),
        ]);
        f.render_widget(para, inner);
        return;
    }
    let visible_h = inner.height as usize;
    let start = scroll.min(combined.len().saturating_sub(1));
    let end = (start + visible_h).min(combined.len());
    let items: Vec<Line> = combined[start..end].iter().enumerate().map(|(i, (kind, pat))| {
        let idx = start + i;
        let sel = idx == selected;
        let style = if sel { Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD).bg(ASHEN.stone) } else { Style::default().fg(ASHEN.smoke) };
        let kind_style = if kind == "dir" { Style::default().fg(ASHEN.frost) } else { Style::default().fg(ASHEN.moss) };
        Line::from(vec![
            Span::styled(format!("  [{}] ", kind), kind_style),
            Span::styled(pat.clone(), style),
        ])
    }).collect();
    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}


/// Spawn the agent task, always sending a done signal on completion (including panics).
fn spawn_agent(
    prompt: String,
    model: String,
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    done_tx: broadcast::Sender<()>,
) -> tokio::task::JoinHandle<()> {
    spawn_agent_with_history(prompt, model, Vec::new(), tx, done_tx)
}

fn spawn_agent_with_history(
    prompt: String,
    model: String,
    history: Vec<serde_json::Value>,
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    done_tx: broadcast::Sender<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use futures::FutureExt;
        let result = std::panic::AssertUnwindSafe(async move {
            let stream = agent::run_agent_with_history(prompt, model, 100, history);
            use futures::StreamExt;
            let mut s = Box::pin(stream);
            while let Some(ev) = s.next().await {
                let _ = tx.send(ev);
            }
        });
        let _ = result.catch_unwind().await;
        let _ = done_tx.send(());
    })
}

fn history_from_messages(msgs: &[Msg]) -> Vec<serde_json::Value> {
    msgs.iter().map(|m| {
        // Preserve role mapping for LLM replay; tool msgs become user context
        match m.role.as_str() {
            "user" => serde_json::json!({"role": "user", "content": m.content}),
            "assistant" => serde_json::json!({"role": "assistant", "content": m.content}),
            "system" => serde_json::json!({"role": "system", "content": m.content}),
            "tool" => serde_json::json!({"role": "user", "content": format!("[tool] {}", m.content)}),
            _ => serde_json::json!({"role": "user", "content": format!("[{}] {}", m.role, m.content)}),
        }
    }).collect()
}

fn llm_history_for_spawn(session: &Option<crate::session::Session>, msgs: &[Msg]) -> Vec<serde_json::Value> {
    if let Some(sess) = session {
        if let Some(ref hist) = sess.llm_history {
            if !hist.is_empty() { return hist.clone(); }
        }
    }
    history_from_messages(msgs)
}

// ── App loop ───────────────────────────────────────────────────

async fn app_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<Stdout>>,
    opts: RunOpts,
) -> anyhow::Result<()> {
    let mut model = opts.model.clone();
    let mut messages: Vec<Msg> = Vec::new();
    // ── Session restore ──
    let mut session = if opts.no_session {
        None
    } else if let Some(ref rid) = opts.resume_id {
        // prefix match
        let list = crate::session::Session::list();
        let found = list.into_iter().find(|sess| sess.id.starts_with(rid));
        match found {
            Some(sess) => {
                messages = sess.messages.iter().map(|m| Msg { role: m.role.clone(), content: m.content.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None}).collect();
                Some(sess)
            }
            None => {
                // not found, start new but warn
                let m = Msg { role: "system".into(), content: format!("[session {} not found, started new]", rid), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None};
                messages.push(m);
                Some(crate::session::Session::new(&model))
            }
        }
    } else if opts.continue_session {
        if let Some(sess) = crate::session::Session::latest() {
            messages = sess.messages.iter().map(|m| Msg { role: m.role.clone(), content: m.content.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None}).collect();
            Some(sess)
        } else {
            Some(crate::session::Session::new(&model))
        }
    } else {
        Some(crate::session::Session::new(&model))
    };
    // Warn if resumed session has a legacy alias not in models.json
    if let Some(ref sess) = session {
        if crate::models::resolve(Some(&sess.model)).is_err() {
            messages.push(Msg {
                role: "system".into(),
                content: format!("[warn: session model '{}' not in {} — use /model to switch]", sess.model, crate::models::path_display()), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
        }
    }
    // helper to persist (cur_model passed explicitly to avoid borrow across mutation)
    let persist = |msgs: &Vec<Msg>, sess: &mut Option<crate::session::Session>, cur_model: &str| {
        if let Some(s) = sess {
            s.model = cur_model.to_string();
            s.cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
            s.messages = msgs.iter().map(|m| crate::session::SavedMsg { role: m.role.clone(), content: m.content.clone() }).collect();
            let _ = s.save();
            crate::session::Session::prune(50);
        }
    };
    let mut scroll: u16 = 0;
    let mut auto_scroll = true;
    let mut step_info = String::new();
    let mut cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // input history
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut textarea = {
        let mut ta = TextArea::default();
        ta.set_style(Style::default().fg(ASHEN.bone).bg(THEME.input_bg));
        ta.set_cursor_style(
            Style::default()
                .fg(ASHEN.bone)
                .bg(ASHEN.ember)
                .add_modifier(Modifier::REVERSED),
        );
        ta.set_cursor_line_style(Style::default().bg(THEME.input_bg));
        ta.set_placeholder_text(
            "  ▸  type a message…  (/help • Enter send • Shift+Enter newline)",
        );
        ta.set_placeholder_style(Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg));
        ta.set_block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(THEME.input_bg)),
        );
        ta
    };

    // autocomplete
    let mut ac_matches: Vec<String> = Vec::new();
    let mut ac_idx: usize = 0;
    let mut ac_scroll: usize = 0;

    // message queue
    let mut msg_queue: Vec<String> = Vec::new();
    let mut agent_busy = false;
    let mut agent_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut spinner_tick: usize = 0;
    let (done_tx, _) = broadcast::channel::<()>(4);
    let mut pending_approval: Option<crate::approval::ApprovalRequest> = None;
    let mut show_sessions = false;
    let mut sessions_scroll: usize = 0;
    let mut sessions_selected: usize = 0;
    let mut sessions_filter = String::new();
    let mut sessions_show_all = false;
    let mut show_allowlist = false;
    let mut allowlist_selected: usize = 0;
    let mut allowlist_scroll: usize = 0;
    let mut show_mcp = false;
    let mut mcp_selected: usize = 0;
    let mut mcp_scroll: usize = 0;
    // Eager MCP init (background)
    tokio::spawn(async move { crate::mcp::init().await; });

    // Redraw only when something changed; the spinner forces redraws while busy.
    let mut dirty = true;

    loop {
        // Poll for bash-guard approval requests from agent
        if pending_approval.is_none() {
            if let Some(req) = crate::approval::take_pending() {
                pending_approval = Some(req);
                dirty = true;
            }
        }
        let term_size = terminal.size()?;
        let queue_rows = msg_queue.len().min(2) as u16;
        // Dynamic input height 1..6 (auto-grow like pi/jcode, clamped)
        let input_height = (textarea.lines().len() as u16).clamp(1, 6);
        let overhead = 4 + queue_rows + input_height; // header + sep + sep + queue + input + footer
                                       // Content area: starts at row 2 (after header + sep), height is the rest
        let content_area = Rect {
            x: 0,
            y: 2,
            width: term_size.width,
            height: term_size.height.saturating_sub(overhead).max(1),
        };

        // Pre-compute layout so we know content area dimensions
        let chunks = {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),              // header
                    Constraint::Length(1),              // separator
                    Constraint::Min(1),                 // content
                    Constraint::Length(1),              // separator
                    Constraint::Length(queue_rows),     // queue (0-2)
                    Constraint::Length(input_height),   // input (auto-grow 1..5)
                    Constraint::Length(1),              // footer
                ])
                .split(Rect {
                    x: 0,
                    y: 0,
                    width: term_size.width,
                    height: term_size.height,
                });
            chunks
        };
        let content_height = chunks[2].height as usize;

        // Build content lines once, wrap once, use for both counting and rendering
        let content_lines = build_content_lines(&messages);
        let wrapped_content = wrap_lines(content_lines, chunks[2].width as usize);
        let total_lines = wrapped_content.len();
        if auto_scroll {
            scroll = total_lines.saturating_sub(content_height) as u16;
        }

        // Single render pass with correct scroll (only when state changed)
        if dirty {
            terminal.draw(|f| {
                let bg_block = Block::default().style(Style::default().bg(THEME.page_bg));
                f.render_widget(bg_block, f.area());

                // Header
                draw_header(f, chunks[0], &model, &step_info);

                // Separator
                draw_separator(f, chunks[1]);

                // Content — use pre-wrapped lines
                let para = Paragraph::new(wrapped_content.clone()).scroll((scroll, 0));
                f.render_widget(para, chunks[2]);

                // Separator
                draw_separator(f, chunks[3]);

                // Queue
                draw_queue(f, chunks[4], &msg_queue);

                // Input (textarea renders with cursor)
                draw_input(f, chunks[5], &mut textarea);

                // Autocomplete popup
                if !ac_matches.is_empty() {
                    draw_autocomplete(f, chunks[5], &ac_matches, ac_idx, ac_scroll);
                }
                // Bash guard approval overlay (takes precedence)
                if let Some(ref req) = pending_approval {
                    draw_approval(f, f.area(), req);
                }
                if show_sessions {
                    draw_sessions(f, f.area(), sessions_selected, sessions_scroll, &sessions_filter, sessions_show_all, &cwd);
                }
                if show_allowlist {
                    draw_allowlist(f, f.area(), allowlist_selected, allowlist_scroll);
                }
                if show_mcp {
                    draw_mcp(f, f.area(), mcp_selected, mcp_scroll);
                }

                // Footer
                draw_footer(
                    f,
                    chunks[6],
                    &model,
                    messages.len(),
                    &cwd,
                    agent_busy,
                    spinner_tick,
                );
            })?;
            dirty = false;
        }

        // Handle keyboard and mouse events
        // Poll fast while the spinner is animating, slower when idle.
        let poll_ms = if agent_busy { 16 } else { 50 };
        if event::poll(std::time::Duration::from_millis(poll_ms))? {
            let term_event = event::read()?;
            dirty = true;
            match term_event {
                Event::Paste(data) => {
                    crate::telemetry::record("paste");
                    let max_cols = (chunks[5].width as usize).saturating_sub(1);
                    textarea.insert_str(hard_wrap_str(&data, max_cols));
                    wrap_cursor_line(&mut textarea, max_cols);
                    ac_matches = current_completions(&textarea);
                    ac_idx = 0;
                }
                Event::Mouse(m) => {
                    let in_content =
                        m.row >= content_area.y && m.row < content_area.y + content_area.height;
                    if in_content {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                scroll = scroll.saturating_sub(3);
                                auto_scroll = false;
                            }
                            MouseEventKind::ScrollDown => {
                                let max_scroll = total_lines.saturating_sub(content_height) as u16;
                                scroll = (scroll + 3).min(max_scroll);
                                if scroll >= max_scroll {
                                    auto_scroll = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Key(k) => {
                    // Allowlist picker modal: hijack keys
                    if show_allowlist {
                        match k.code {
                            KeyCode::Esc => {
                                show_allowlist = false;
                                allowlist_scroll = 0;
                            }
                            KeyCode::Enter => {
                                show_allowlist = false;
                            }
                            KeyCode::Up => {
                                if allowlist_selected > 0 {
                                    allowlist_selected -= 1;
                                    if allowlist_selected < allowlist_scroll { allowlist_scroll = allowlist_selected; }
                                }
                            }
                            KeyCode::Down => {
                                let bash_len = crate::bash_guard::allowlist_list().len();
                                let dir_len = crate::dir_guard::allowlist_list().len();
                                let len = bash_len + dir_len;
                                if allowlist_selected + 1 < len {
                                    allowlist_selected += 1;
                                    if allowlist_selected >= allowlist_scroll + 10 { allowlist_scroll += 1; }
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Delete => {
                                // rebuild combined as in draw
                                let bash_list = crate::bash_guard::allowlist_list();
                                let dir_list = crate::dir_guard::allowlist_list();
                                let mut combined: Vec<(String, String)> = Vec::new();
                                for p in &bash_list { combined.push(("bash".into(), p.clone())); }
                                for p in &dir_list { combined.push(("dir".into(), p.clone())); }
                                combined.sort_by(|a,b| a.1.cmp(&b.1));
                                if let Some((kind, pat)) = combined.get(allowlist_selected).cloned() {
                                    if kind == "dir" { crate::dir_guard::allowlist_remove(&pat); } else { crate::bash_guard::allowlist_remove(&pat); }
                                    let new_len = crate::bash_guard::allowlist_list().len() + crate::dir_guard::allowlist_list().len();
                                    if allowlist_selected >= new_len && allowlist_selected > 0 { allowlist_selected -= 1; }
                                }
                            }
                            KeyCode::Char('c') => {
                                crate::bash_guard::allowlist_clear();
                                crate::dir_guard::allowlist_clear();
                                allowlist_selected = 0;
                                allowlist_scroll = 0;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Sessions picker modal: hijack keys
                    if show_sessions {
                        match k.code {
                            KeyCode::Esc => {
                                show_sessions = false;
                                sessions_scroll = 0;
                                sessions_filter.clear();
                                sessions_selected = 0;
                            }
                            KeyCode::Tab | KeyCode::BackTab => {
                                sessions_show_all = !sessions_show_all;
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Enter => {
                                // Apply same ordering as draw: dir filter + text filter
                                let all = crate::session::Session::list();
                                let cwd_norm = cwd.trim_end_matches('/');
                                let dir_filtered: Vec<crate::session::Session> = if sessions_show_all {
                                    all
                                } else {
                                    all.into_iter().filter(|s| s.cwd.trim_end_matches('/') == cwd_norm).collect()
                                };
                                let filtered: Vec<crate::session::Session> = if sessions_filter.is_empty() { dir_filtered } else {
                                    let lower = sessions_filter.to_lowercase();
                                    dir_filtered.into_iter().filter(|s| s.id.to_lowercase().contains(&lower) || s.cwd.to_lowercase().contains(&lower) || s.messages.iter().any(|m| m.content.to_lowercase().contains(&lower))).collect()
                                };
                                if let Some(sess) = filtered.get(sessions_selected).cloned() {
                                    messages = sess.messages.iter().map(|m| Msg { role: m.role.clone(), content: m.content.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None}).collect();
                                    messages.push(Msg { role: "system".into(), content: format!("[resumed session {}]", sess.id), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    session = Some(sess);
                                }
                                show_sessions = false;
                                sessions_filter.clear();
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Up => {
                                if sessions_selected > 0 {
                                    sessions_selected -= 1;
                                    if sessions_selected < sessions_scroll { sessions_scroll = sessions_selected; }
                                }
                            }
                            KeyCode::Down => {
                                // Compute filtered len with dir + text filters
                                let all = crate::session::Session::list();
                                let cwd_norm = cwd.trim_end_matches('/');
                                let dir_filtered: Vec<crate::session::Session> = if sessions_show_all {
                                    all
                                } else {
                                    all.into_iter().filter(|s| s.cwd.trim_end_matches('/') == cwd_norm).collect()
                                };
                                let filtered_len = if sessions_filter.is_empty() { dir_filtered.len() } else {
                                    let lower = sessions_filter.to_lowercase();
                                    dir_filtered.iter().filter(|s| s.id.to_lowercase().contains(&lower) || s.cwd.to_lowercase().contains(&lower) || s.messages.iter().any(|m| m.content.to_lowercase().contains(&lower))).count()
                                };
                                if sessions_selected + 1 < filtered_len {
                                    sessions_selected += 1;
                                    let visible = 10;
                                    if sessions_selected >= sessions_scroll + visible { sessions_scroll += 1; }
                                }
                            }
                            KeyCode::Backspace => {
                                sessions_filter.pop();
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && !k.modifiers.contains(KeyModifiers::ALT) => {
                                sessions_filter.push(c);
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // MCP picker modal: hijack keys
                    if show_mcp {
                        match k.code {
                            KeyCode::Esc => {
                                show_mcp = false;
                                mcp_scroll = 0;
                            }
                            KeyCode::Enter | KeyCode::Char('r') => {
                                let servers = crate::mcp::snapshot();
                                if let Some(srv) = servers.get(mcp_selected).cloned() {
                                    if crate::mcp::is_server_disabled(&srv.name) || matches!(srv.status, crate::mcp::ServerStatus::Disabled) {
                                        // stay open, show [disabled] instantly — no chat spam
                                    } else {
                                        let name = srv.name.clone();
                                        tokio::spawn(async move {
                                            let _ = crate::mcp::reconnect(&name).await;
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Char('t') | KeyCode::Char(' ') => {
                                let servers = crate::mcp::snapshot();
                                if let Some(srv) = servers.get(mcp_selected).cloned() {
                                    let now_disabled = crate::mcp::toggle_server_disabled(&srv.name);
                                    if !now_disabled {
                                        let name = srv.name.clone();
                                        tokio::spawn(async move {
                                            let _ = crate::mcp::reconnect(&name).await;
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('g') => {
                                let was = crate::mcp::is_global_disabled();
                                crate::mcp::set_global_disabled(!was);
                                if was {
                                    tokio::spawn(async move { crate::mcp::init().await; });
                                }
                            }
                            KeyCode::Up => {
                                if mcp_selected > 0 {
                                    mcp_selected -= 1;
                                    if mcp_selected < mcp_scroll { mcp_scroll = mcp_selected; }
                                }
                            }
                            KeyCode::Down => {
                                let len = crate::mcp::snapshot().len();
                                if mcp_selected + 1 < len {
                                    mcp_selected += 1;
                                    if mcp_selected >= mcp_scroll + 8 { mcp_scroll += 1; }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Guard approval modal: hijack all keys (covers bash + dir + mcp guard)
                    if pending_approval.is_some() {
                        let mut req = pending_approval.take().unwrap();
                        let is_dir = req.reasons.iter().any(|r| r.contains("outside CWD"));
                        let is_mcp = req.reasons.iter().any(|r| r.contains("MCP tool"));
                        match k.code {
                            KeyCode::Char('a') if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(tx) = req.tx.take() { let _ = tx.send(true); }
                                crate::telemetry::record(if is_mcp { "mcp_guard_allow_once" } else if is_dir { "dir_guard_allow_once" } else { "bash_guard_allow_once" });
                            }
                            KeyCode::Char('A') => {
                                if is_mcp {
                                    // MCP: every call needs approval, 'A' is treated as allow once (no persist)
                                    if let Some(tx) = req.tx.take() { let _ = tx.send(true); }
                                    crate::telemetry::record("mcp_guard_allow_once");
                                } else if is_dir {
                                    // Extract offending paths from reasons "outside CWD (path → resolved)"
                                    for r in &req.reasons {
                                        if let Some(s) = r.find('(') {
                                            if let Some(e) = r.find(" →") {
                                                let raw = r[s+1..e].trim();
                                                if !raw.is_empty() { crate::dir_guard::allowlist_add(raw); }
                                            }
                                        }
                                    }
                                    // Also allowlist the raw cmd/path as fallback
                                    // For file tools cmd is "read_file /path", extract last token
                                    let fallback = if req.cmd.contains(' ') { req.cmd.split_whitespace().last().unwrap_or(&req.cmd).to_string() } else { req.cmd.clone() };
                                    if !fallback.is_empty() { crate::dir_guard::allowlist_add(&fallback); }
                                    // For bash, also allowlist the full command for exact match
                                    if req.cmd.contains('/') || req.cmd.contains(' ') { crate::dir_guard::allowlist_add(&req.cmd); }
                                } else {
                                    crate::bash_guard::allowlist_add(&req.cmd);
                                }
                                if !is_mcp {
                                    if let Some(tx) = req.tx.take() { let _ = tx.send(true); }
                                    crate::telemetry::record(if is_dir { "dir_guard_allow_always" } else { "bash_guard_allow_always" });
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                if let Some(tx) = req.tx.take() { let _ = tx.send(false); }
                                crate::telemetry::record(if is_mcp { "mcp_guard_deny" } else { "bash_guard_deny" });
                            }
                            KeyCode::Enter => {
                                if let Some(tx) = req.tx.take() { let _ = tx.send(true); }
                                crate::telemetry::record(if is_mcp { "mcp_guard_allow_once" } else if is_dir { "dir_guard_allow_once" } else { "bash_guard_allow_once" });
                            }
                            KeyCode::Esc => {
                                if let Some(tx) = req.tx.take() { let _ = tx.send(false); }
                                crate::telemetry::record(if is_mcp { "mcp_guard_deny" } else { "bash_guard_deny" });
                            }
                            _ => {
                                pending_approval = Some(req);
                            }
                        }
                        continue;
                    }
                    let mut submit_pending = false;
                    let lines_before = textarea.lines().to_vec();
                    // Ctrl+ base clearing handled before textarea
                    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                    let alt = k.modifiers.contains(KeyModifiers::ALT);
                    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
                    match k.code {
                        KeyCode::Esc => {
                            if !ac_matches.is_empty() {
                                ac_matches.clear();
                                ac_idx = 0;
                            } else if agent_busy {
                                // Abort the running agent - keep braille/spinner alive if queue pending
                                if let Some(handle) = agent_handle.take() {
                                    crate::telemetry::record("interrupt");
                                    handle.abort();
                                }
                                if !msg_queue.is_empty() {
                                    // don't flicker busy off -> braille keeps ticking
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "[interrupted → next queued]".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    let next = msg_queue.remove(0);
                                    let expanded_next = expand_at_mentions(&next);
                                    let hist = llm_history_for_spawn(&session, &messages);
                                    messages.push(Msg {
                                        role: "user".into(),
                                        content: next.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    // agent_busy stays true
                                    step_info = "...".into();
                        agent_handle = Some(spawn_agent_with_history(
                            expanded_next,
                            model.clone(),
                            hist,
                            tx.clone(),
                            done_tx.clone(),
                        ));
                                } else {
                                    agent_busy = false;
                                    step_info.clear();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "[interrupted]".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                            } else {
                                // Esc on empty input clears textarea
                                if textarea.is_empty() {
                                    // no-op
                                } else {
                                    textarea.select_all();
                                    textarea.cut();
                                }
                                ac_matches.clear();
                                ac_idx = 0;
                            }
                        }
                        KeyCode::Char('d') if ctrl => break,
                        KeyCode::Char('c') if ctrl => {
                            // Ctrl+C: clear current input (Emacs kill) — keep textarea undo
                            if !textarea.is_empty() {
                                textarea.select_all();
                                textarea.cut();
                            }
                            ac_matches.clear();
                            ac_idx = 0;
                        }
                        KeyCode::Char('u') if ctrl => {
                            // Emacs Ctrl+U kill to start of line (override textarea undo)
                            // delete from head using textarea API
                            textarea.delete_line_by_head();
                            crate::telemetry::record("kill_line");
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('k') if ctrl => {
                            textarea.delete_line_by_end();
                            crate::telemetry::record("kill_line_end");
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('z') if ctrl => {
                            crate::telemetry::record("input_undo");
                            textarea.undo();
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('y') if ctrl => {
                            textarea.paste();
                            crate::telemetry::record("paste");
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Enter if ctrl && !shift && !alt => {
                            // Ctrl+Enter: always submit
                            ac_matches.clear();
                            ac_idx = 0;
                            if !textarea.is_empty()
                                && !textarea.lines().join("\n").trim().is_empty()
                            {
                                submit_pending = true;
                            }
                        }
                        KeyCode::Enter if shift && !ctrl && !alt => {
                            crate::telemetry::record("newline");
                            let inp = crossterm_key_to_input(k);
                            textarea.input(inp);
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Enter => {
                            // Enter: if autocomplete visible, accept it first
                            if !ac_matches.is_empty() {
                                let chosen = ac_matches[ac_idx].clone();
                                if let Some(m) = detect_skill_mention(&textarea) {
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col + 1].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line2 = format!("{}{}{}", before, chosen, after);
                                        lines[m.row] = new_line2;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + 1 + chosen.chars().count();
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(m.row as u16, new_col as u16));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(chosen);
                                    }
                                } else if let Some(m) = detect_at_mention(&textarea) {
                                    // Replace @prefix with @chosen
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col + 1].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line2 = format!("{}{}{}", before, chosen, after);
                                        lines[m.row] = new_line2;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + 1 + chosen.chars().count();
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(m.row as u16, new_col as u16));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(chosen);
                                    }
                                } else {
                                    textarea.select_all();
                                    textarea.cut();
                                    textarea.insert_str(chosen);
                                }
                                ac_matches.clear();
                                ac_idx = 0;
                                ac_scroll = 0;
                            } else {
                                // Normal Enter submits (single line). If multiline (contains newline), submit still
                                let cur = textarea.lines().join("\n");
                                if !cur.trim().is_empty() {
                                    submit_pending = true;
                                }
                                ac_matches.clear();
                                ac_idx = 0;
                            }
                        }
                        KeyCode::Up => {
                            if !ac_matches.is_empty() {
                                ac_idx = if ac_idx == 0 {
                                    ac_matches.len() - 1
                                } else {
                                    ac_idx - 1
                                };
                            } else if !history.is_empty() {
                                crate::telemetry::record("prompt_recall");
                                // Only hijack Up for history when cursor at top line (native fish-like)
                                let r = textarea.cursor().0;
                                if r == 0 {
                                    let idx = hist_idx
                                        .map(|i| if i == 0 { 0 } else { i - 1 })
                                        .unwrap_or(history.len() - 1);
                                    hist_idx = Some(idx);
                                    let hist = history[idx].clone();
                                    textarea.select_all();
                                    textarea.cut();
                                    textarea.insert_str(hist);
                                    textarea.move_cursor(CursorMove::Bottom);
                                    ac_matches.clear();
                                } else {
                                    let inp: TAInput = crossterm_key_to_input(k);
                                    textarea.input(inp);
                                }
                            } else {
                                let inp = crossterm_key_to_input(k);
                                textarea.input(inp);
                            }
                        }
                        KeyCode::Down => {
                            // telemetry for history nav handled inside
                            if !ac_matches.is_empty() {
                                ac_idx = (ac_idx + 1) % ac_matches.len();
                                crate::telemetry::record("prompt_jump_down");
                            } else if let Some(idx) = hist_idx {
                                if idx + 1 < history.len() {
                                    hist_idx = Some(idx + 1);
                                    let hist = history[idx + 1].clone();
                                    textarea.select_all();
                                    textarea.cut();
                                    textarea.insert_str(hist);
                                    textarea.move_cursor(CursorMove::Bottom);
                                } else {
                                    hist_idx = None;
                                    textarea.select_all();
                                    textarea.cut();
                                }
                            } else {
                                let inp = crossterm_key_to_input(k);
                                textarea.input(inp);
                                // if textarea moved down internally, keep
                            }
                        }
                        KeyCode::Tab => {
                            if !ac_matches.is_empty() {
                                let chosen = ac_matches[ac_idx].clone();
                                if let Some(m) = detect_skill_mention(&textarea) {
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col + 1].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line = format!("{}{}{}", before, chosen, after);
                                        lines[m.row] = new_line;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + 1 + chosen.chars().count();
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(m.row as u16, new_col as u16));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(chosen);
                                    }
                                } else if let Some(m) = detect_at_mention(&textarea) {
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col + 1].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line = format!("{}{}{}", before, chosen, after);
                                        lines[m.row] = new_line;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + 1 + chosen.chars().count();
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(m.row as u16, new_col as u16));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(chosen);
                                    }
                                } else {
                                    textarea.select_all();
                                    textarea.cut();
                                    textarea.insert_str(chosen);
                                }
                                ac_matches.clear();
                                ac_idx = 0;
                                ac_scroll = 0;
                            } else {
                                // Tab inserts 2 spaces (or delegate)
                                let inp: TAInput = crossterm_key_to_input(k);
                                // prevent tab from inserting inside textarea as literal tab; insert spaces
                                if textarea.lines().join("\n").starts_with('/') {
                                    // in command mode, cycle autocomplete
                                } else {
                                    textarea.input(inp);
                                }
                            }
                        }
                        KeyCode::Backspace if alt => {
                            // Alt+Backspace word delete (textarea already handles but ensure)
                            textarea.delete_word();
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Left if alt => {
                            // Alt+Left word jump
                            textarea.move_cursor(CursorMove::WordBack);
                            crate::telemetry::record("word_back");
                        }
                        KeyCode::Right if alt => {
                            textarea.move_cursor(CursorMove::WordForward);
                            crate::telemetry::record("word_forward");
                        }
                        KeyCode::PageUp => {
                            scroll = scroll.saturating_sub(content_height as u16);
                            auto_scroll = false;
                        }
                        KeyCode::PageDown => {
                            let max_scroll = total_lines.saturating_sub(content_height) as u16;
                            scroll = (scroll + content_height as u16).min(max_scroll);
                            if scroll >= max_scroll {
                                auto_scroll = true;
                            }
                        }
                        KeyCode::Home => {
                            // Home in input: go to head; if with ctrl, scroll to top? Keep input head
                            if ctrl {
                                scroll = 0;
                                auto_scroll = false;
                            } else {
                                textarea.move_cursor(CursorMove::Head);
                            }
                        }
                        KeyCode::End => {
                            if ctrl {
                                auto_scroll = true;
                            } else {
                                textarea.move_cursor(CursorMove::End);
                            }
                        }
                        _ => {
                            // Delegate everything else to textarea (handles Char, Backspace, Delete, arrows, Ctrl+A/E/F/B etc)
                            // Special: Ctrl+W already handled as delete_word via textarea mapping, but we ensure
                            let inp: TAInput = crossterm_key_to_input(k);
                            let modified = textarea.input(inp);
                            if modified
                                || matches!(
                                    k.code,
                                    KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
                                )
                            {
                                // update autocomplete on content change
                                let new_matches = current_completions(&textarea);
                                if new_matches != ac_matches {
                                    ac_matches = new_matches;
                                    ac_idx = 0;
                                    ac_scroll = 0;
                                } else if !ac_matches.is_empty() {
                                    // keep selection
                                }
                            } else {
                                // Even if not modified, still recompute for typing
                                let cur = textarea.lines().join("\n");
                                ac_matches = current_completions(&textarea);
                                if ac_matches.is_empty() {
                                    ac_idx = 0;
                                }
                            }
                        }
                    }
                    // Hard-wrap the input line when typing reached the right edge
                    if textarea.lines() != lines_before.as_slice() {
                        let max_cols = (chunks[5].width as usize).saturating_sub(1);
                        wrap_cursor_line(&mut textarea, max_cols);
                        ac_matches = current_completions(&textarea);
                        ac_idx = 0;
                        ac_scroll = 0;
                    }
                    // Handle bracketed-paste style: ac_matches index scroll sync
                    if !ac_matches.is_empty() {
                        // sync scroll offset to keep selected visible
                        if ac_idx < ac_scroll {
                            ac_scroll = ac_idx;
                        } else if ac_idx >= ac_scroll + 5 {
                            ac_scroll = ac_idx - 4;
                        }
                    }
                    // Handle pending submit (Enter / Ctrl+Enter)
                    if submit_pending {
                        let raw = textarea.lines().join("\n");
                        let prompt_raw = raw.trim().to_string();
                        // Keep display as raw, but expand @files for LLM
                        let prompt = prompt_raw.clone();
                        let expanded = expand_at_mentions(&prompt_raw);
                        if prompt.is_empty() {
                            // skip
                        } else if prompt.starts_with('/') {
                            match prompt.as_str() {
                                "/exit" | "/quit" => break,
                                "/new" | "/clear" => {
                                    if let Some(handle) = agent_handle.take() {
                                        handle.abort();
                                    }
                                    while rx.try_recv().is_ok() {}
                                    messages.clear();
                                    scroll = 0;
                                    auto_scroll = true;
                                    step_info.clear();
                                    msg_queue.clear();
                                    agent_busy = false;
                                    if !opts.no_session {
                                        session = Some(crate::session::Session::new(&model));
                                    }
                                }
                                "/help" => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "/help /new /sessions /resume <id> /allowlist /allowlist clear /mcp /memory stats|consolidate /model <name> /clear /exit  ·  Enter send · Shift+Enter newline · @file $skill · Ctrl+C clear · Ctrl+U kill · Ctrl+Z undo · Up/Down history · PgUp/PgDn scroll".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                "/mcp" => {
                                    show_mcp = true;
                                    mcp_selected = 0;
                                    mcp_scroll = 0;
                                }
                                "/sessions" => {
                                    show_sessions = true;
                                    sessions_selected = 0;
                                    sessions_scroll = 0;
                                    sessions_filter.clear();
                                    sessions_show_all = false;
                                }
                                "/allowlist" => {
                                    show_allowlist = true;
                                    allowlist_selected = 0;
                                    allowlist_scroll = 0;
                                }
                                "/memory" => {
                                    let stats = crate::memory::api_stats();
                                    messages.push(Msg { role: "system".into(), content: stats, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                "/memory stats" => {
                                    let stats = crate::memory::api_stats();
                                    messages.push(Msg { role: "system".into(), content: stats, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                "/memory consolidate" => {
                                    let out = crate::memory::api_consolidate();
                                    messages.push(Msg { role: "system".into(), content: out, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                _ if prompt.starts_with("/allowlist ") => {
                                    let rest = prompt.strip_prefix("/allowlist ").unwrap().trim();
                                    if rest == "clear" {
                                        crate::bash_guard::allowlist_clear();
                                        crate::dir_guard::allowlist_clear();
                                        messages.push(Msg { role: "system".into(), content: "allowlists cleared (bash + dir)".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    } else if rest.starts_with("add ") {
                                        let pat = rest.strip_prefix("add ").unwrap().trim();
                                        // if pat looks like a path, add to dir allowlist, else bash
                                        if pat.contains('/') || pat.starts_with('~') || pat.starts_with('.') {
                                            crate::dir_guard::allowlist_add(pat);
                                            messages.push(Msg { role: "system".into(), content: format!("dir-allowlisted: {}", pat), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        } else {
                                            crate::bash_guard::allowlist_add(pat);
                                            messages.push(Msg { role: "system".into(), content: format!("allowlisted: {}", pat), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        }
                                    } else if rest.starts_with("rm ") || rest.starts_with("remove ") {
                                        let pat = rest.split_once(' ').map(|(_, p)| p.trim()).unwrap_or(rest);
                                        crate::bash_guard::allowlist_remove(pat);
                                        crate::dir_guard::allowlist_remove(pat);
                                        messages.push(Msg { role: "system".into(), content: format!("removed: {}", pat), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    } else {
                                        // heuristics: path-like → dir
                                        if rest.contains('/') || rest.starts_with('~') {
                                            crate::dir_guard::allowlist_add(rest);
                                            messages.push(Msg { role: "system".into(), content: format!("dir-allowlisted: {}", rest), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        } else {
                                            crate::bash_guard::allowlist_add(rest);
                                            messages.push(Msg { role: "system".into(), content: format!("allowlisted: {}", rest), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        }
                                    }
                                }
                                _ if prompt.starts_with("/resume ") => {
                                    let rid = prompt.strip_prefix("/resume ").unwrap().trim();
                                    let list = crate::session::Session::list();
                                    if let Some(sess) = list.into_iter().find(|sess| sess.id.starts_with(rid)) {
                                        let sid = sess.id.clone();
                                        messages = sess.messages.iter().map(|m| Msg { role: m.role.clone(), content: m.content.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None}).collect();
                                        messages.push(Msg { role: "system".into(), content: format!("[resumed session {}]", sid), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        session = Some(sess);
                                    } else {
                                        messages.push(Msg { role: "system".into(), content: format!("session {} not found", rid), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    }
                                }
                                "/model" => {
                                    // List current alias + available
                                    match crate::models::load() {
                                        Ok(cfg) => {
                                            let entry = cfg.models.get(&model);
                                            let detail = if let Some(e) = entry {
                                                format!(" → {} @ {}", e.model, e.base_url.as_deref().unwrap_or("env: OPENCODE_BASE_URL"))
                                            } else {
                                                String::new()
                                            };
                                            let mut aliases: Vec<String> = cfg.models.keys().cloned().collect();
                                            aliases.sort();
                                            let available = aliases.join(", ");
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("current model: {}{} (available: {})\nconfig: {} — use /model <alias> to switch", model, detail, available, crate::models::path_display()), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        }
                                        Err(e) => {
                                            messages.push(Msg { role: "system".into(), content: format!("models.json error: {}", e), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        }
                                    }
                                }
                                _ if prompt.starts_with("/model ") => {
                                    let m = prompt.strip_prefix("/model ").unwrap().trim();
                                    if m.is_empty() {
                                        // bare /model with trailing space → list
                                        match crate::models::load() {
                                            Ok(cfg) => {
                                                let mut aliases: Vec<String> = cfg.models.keys().cloned().collect();
                                                aliases.sort();
                                                messages.push(Msg { role: "system".into(), content: format!("current model: {} (available: {})", model, aliases.join(", ")), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                            }
                                            Err(e) => messages.push(Msg { role: "system".into(), content: format!("models.json error: {}", e), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None}),
                                        }
                                    } else {
                                        match crate::models::resolve(Some(m)) {
                                            Ok(r) => {
                                                model = r.alias.clone();
                                                if let Some(s) = &mut session {
                                                    s.model = model.clone();
                                                    let _ = s.save();
                                                }
                                                messages.push(Msg {
                                                    role: "system".into(),
                                                    content: format!("switched to {} ({} @ {}) — live + persisted", r.alias, r.model, r.base_url), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                            }
                                            Err(e) => {
                                                messages.push(Msg { role: "system".into(), content: format!("model switch failed: {}", e), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("unknown command: {}", prompt), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                            }
                            textarea.select_all();
                            textarea.cut();
                            hist_idx = None;
                            ac_matches.clear();
                            ac_idx = 0;
                        } else {
                            // Send user message
                            history.push(prompt.clone());
                            hist_idx = None;
                            step_info = "...".into();
                            textarea.select_all();
                            textarea.cut();
                            auto_scroll = true;
                            ac_matches.clear();
                            ac_idx = 0;

                            if agent_busy {
                                msg_queue.push(prompt.clone());
                            } else {
                                let hist = llm_history_for_spawn(&session, &messages);
                                messages.push(Msg {
                                    role: "user".into(),
                                    content: prompt.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                agent_busy = true;
                                agent_handle = Some(spawn_agent_with_history(
                                    expanded.clone(),
                                    model.clone(),
                                    hist,
                                    tx.clone(),
                                    done_tx.clone(),
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Poll agent events (non-blocking)
        while let Ok(ev) = rx.try_recv() {
            dirty = true;
            match ev {
                AgentEvent::Text { delta } => {
                    if delta.is_empty() {
                        continue;
                    }
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "assistant".into(),
                                content: delta, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                        }
                    } else {
                        messages.push(Msg {
                            role: "assistant".into(),
                            content: delta, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                    }
                    step_info = "".into();
                }
                AgentEvent::Reasoning { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "thinking" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "thinking".into(),
                                content: delta, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                        }
                    } else {
                        messages.push(Msg {
                            role: "thinking".into(),
                            content: delta, tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                    }
                }
                AgentEvent::TextDone { text } => {
                    let _ = text;
                }
                AgentEvent::ToolStart { name, args, id } => {
                    let args_str = serde_json::to_string(&args).unwrap_or_default();
                    messages.push(Msg::new_tool_start(name, args_str, id));
                }
                AgentEvent::ToolResult {
                    name,
                    result,
                    id,
                    elapsed_ms,
                } => {
                    let truncated: String = result.chars().take(2000).collect();
                    let display = if result.chars().count() > 2000 {
                        format!(
                            "{}… [truncated {} chars]",
                            truncated,
                            result.chars().count() - 2000
                        )
                    } else {
                        result
                    };
                    let mut found = false;
                    for msg in messages.iter_mut().rev() {
                        if msg.tool_id.as_deref() == Some(&id) && msg.elapsed_ms.is_none() {
                            msg.content = format!("{} → {}", name, display);
                            msg.elapsed_ms = Some(elapsed_ms);
                            if msg.tool_name.is_none() { msg.tool_name = Some(name.clone()); }
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        let mut m = Msg::new_tool_start(name.clone(), String::new(), id.clone());
                        m.content = format!("{} → {}", name, display);
                        m.elapsed_ms = Some(elapsed_ms);
                        messages.push(m);
                    }
                    if name == "bash" {
                        if let Ok(new_cwd) = std::env::current_dir() {
                            cwd = new_cwd.display().to_string();
                        }
                    }
                }
                AgentEvent::Step { n } => {
                    step_info = format!("step {}", n);
                }
                AgentEvent::Done { text, history } => {
                    let _ = text;
                    // Persist llm_history for exact replay
                    if let Some(sess) = session.as_mut() {
                        sess.llm_history = Some(history.clone());
                        let _ = sess.save();
                    }
                    // Observer: distill recent chunk into memories (async, non-blocking)
                    if std::env::var("LEAN_OBSERVER_DISABLED").unwrap_or_default() != "1" {
                        let chunk_pairs: Vec<(String, String)> = messages.iter().rev().take(12).rev().map(|m| (m.role.clone(), m.content.clone())).collect();
                        let chunk = crate::observer::build_chunk_text(&chunk_pairs);
                        let obs_model = model.clone();
                        tokio::spawn(async move {
                            crate::observer::observe_chunk(chunk, obs_model).await;
                        });
                    }
                    agent_handle.take();

                    // Keep braille/spinner alive continuously when queue has items:
                    // don't flash "done" or clear agent_busy between sessions.
                    if !msg_queue.is_empty() {
                        let next = msg_queue.remove(0);
                        let expanded_next = expand_at_mentions(&next);
                        let hist = llm_history_for_spawn(&session, &messages);
                        messages.push(Msg {
                            role: "user".into(),
                            content: next.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                        // stay busy — braille spinner keeps ticking
                        step_info = "...".into();
                        // agent_busy stays true
                        agent_handle = Some(spawn_agent_with_history(
                            expanded_next,
                            model.clone(),
                            hist,
                            tx.clone(),
                            done_tx.clone(),
                        ));
                    } else {
                        step_info = "done".into();
                        agent_busy = false;
                    }
                }
            }
            auto_scroll = true;
        }

        // If agent is busy but no events arrived, check if the task died (panic/error)
        if agent_busy && agent_handle.as_ref().map_or(false, |h| h.is_finished()) {
            dirty = true;
            agent_handle.take();
            // Drain stale done signals
            let mut done_rx = done_tx.subscribe();
            while done_rx.try_recv().is_ok() {}
            // Reset busy state
            if !msg_queue.is_empty() {
                let next = msg_queue.remove(0);
                let expanded_next = expand_at_mentions(&next);
                let hist = llm_history_for_spawn(&session, &messages);
                messages.push(Msg {
                    role: "user".into(),
                    content: next.clone(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                step_info = "...".into();
                        agent_handle = Some(spawn_agent_with_history(
                            expanded_next,
                            model.clone(),
                            hist,
                            tx.clone(),
                            done_tx.clone(),
                        ));
            } else {
                agent_busy = false;
                step_info = "error".into();
                messages.push(Msg {
                    role: "system".into(),
                    content: "[agent crashed]".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
            }
        }

        // Persist session (fire-and-forget, cheap json write)
        persist(&messages, &mut session, &model);
        // Advance spinner; force a redraw while it is animating.
        if agent_busy {
            spinner_tick = spinner_tick.wrapping_add(1);
            dirty = true;
        }
    }

    Ok(())
}
