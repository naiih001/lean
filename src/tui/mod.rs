use crate::agent::{self, AgentEvent};
use crate::integrations::dictate;
use crate::tui::theme::{ASHEN, THEME};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use ratatui_textarea::{CursorMove, Input as TAInput, Key as TAKey, TextArea};
use std::io::Stdout;
use tokio::sync::broadcast;

pub mod layout;
pub mod markdown;
pub mod theme;
pub mod widgets;

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
    crate::guards::bash::set_disabled(opts.bash_guard_disabled);
    crate::guards::dir::set_disabled(opts.dir_guard_disabled);
    crate::guards::dir::init(None);
    crate::services::question::set_interactive(true);
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
        Self {
            role: role.into(),
            content: content.into(),
            tool_id: None,
            tool_name: None,
            tool_args: None,
            elapsed_ms: None,
        }
    }
    fn new_tool_start(name: String, args: String, id: String) -> Self {
        Self {
            role: "tool".into(),
            content: format!("{} {}", name, args),
            tool_id: Some(id),
            tool_name: Some(name.clone()),
            tool_args: Some(args),
            elapsed_ms: None,
        }
    }
    fn format_elapsed(&self) -> String {
        if let Some(ms) = self.elapsed_ms {
            if ms < 1000 {
                format!("{}ms", ms)
            } else {
                format!("{:.2}s", ms as f64 / 1000.0)
            }
        } else {
            String::new()
        }
    }
}

fn shorten_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && p.starts_with(&home) {
        format!("~{}", &p[home.len()..])
    } else {
        p.to_string()
    }
}

fn format_tool_preview(name: &str, args_str: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(args_str).unwrap_or(serde_json::Value::Null);
    let obj = v.as_object();
    let s = shorten_path;
    match name {
        "bash" => {
            let cmd = obj
                .and_then(|o| o.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            let preview = if cmd.len() > 60 {
                format!("{}…", &cmd[..60])
            } else {
                cmd.to_string()
            };
            format!("$ {}", preview)
        }
        "read" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            let off = obj.and_then(|o| o.get("offset")).and_then(|v| v.as_u64());
            let lim = obj.and_then(|o| o.get("limit")).and_then(|v| v.as_u64());
            let mut txt = s(path);
            if off.is_some() || lim.is_some() {
                let start = off.unwrap_or(1);
                let end = lim
                    .map(|l| start + l - 1)
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                txt = format!(
                    "{}:{}{}",
                    txt,
                    start,
                    if end.is_empty() {
                        "".to_string()
                    } else {
                        format!("-{}", end)
                    }
                );
            }
            format!("read {}", txt)
        }
        "write" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            let content = obj
                .and_then(|o| o.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let lines = content.lines().count();
            if lines > 1 {
                format!("write {} ({} lines)", s(path), lines)
            } else {
                format!("write {}", s(path))
            }
        }
        "edit" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            format!("edit {}", s(path))
        }
        "ls" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            format!("ls {}", s(path))
        }
        "find" => {
            let pat = obj
                .and_then(|o| o.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            if path == "." {
                format!("find {}", pat)
            } else {
                format!("find {} in {}", pat, s(path))
            }
        }
        "grep" => {
            let pat = obj
                .and_then(|o| o.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            if path == "." {
                format!("grep /{}/", pat)
            } else {
                format!("grep /{}/ in {}", pat, s(path))
            }
        }
        "web_fetch" => {
            let url = obj
                .and_then(|o| o.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            let short = if url.len() > 50 {
                format!("{}…", &url[..50])
            } else {
                url.to_string()
            };
            format!("fetch {}", short)
        }
        "web_search" => {
            let q = obj
                .and_then(|o| o.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let short = if q.len() > 40 {
                format!("{}…", &q[..40])
            } else {
                q.to_string()
            };
            format!("search {}", short)
        }
        _ => {
            let preview = if args_str.len() > 50 {
                format!("{}…", &args_str[..50])
            } else {
                args_str.to_string()
            };
            if preview == "null" || preview == "{}" {
                name.to_string()
            } else {
                format!("{} {}", name, preview)
            }
        }
    }
}

fn sanitize_display_content(s: &str) -> String {
    // Replace base64 image markers with short placeholder for display (keep LLM data but don't render it)
    let mut out = s.to_string();
    // Replace <<IMAGE:...>> and <<IMAGE_URL:...>>
    while let Some(start) = out.find("<<IMAGE") {
        if let Some(end) = out[start..].find(">>") {
            let end_idx = start + end + 2;
            let marker = &out[start..end_idx];
            let placeholder = if marker.starts_with("<<IMAGE_URL:") {
                let url = marker
                    .trim_start_matches("<<IMAGE_URL:")
                    .trim_end_matches(">>");
                let short = if url.len() > 40 {
                    format!("{}…", &url[..40])
                } else {
                    url.to_string()
                };
                format!("[img: {}]", short)
            } else if marker.starts_with("<<IMAGE:") {
                let inner = marker.trim_start_matches("<<IMAGE:").trim_end_matches(">>");
                let mime = inner.split(':').next().unwrap_or("image");
                format!("[img: {}]", mime)
            } else {
                "[img]".to_string()
            };
            out.replace_range(start..end_idx, &placeholder);
        } else {
            break;
        }
    }
    out
}

impl Msg {
    /// Render this message as styled ratatui Lines.
    fn render_lines(&self) -> Vec<Line<'static>> {
        match self.role.as_str() {
            "user" => {
                let display = sanitize_display_content(&self.content);
                let wash = THEME.user_bg;
                let mut lines: Vec<Line<'static>> = Vec::new();
                // No header — full-width wash block instead (subtle slate tint)
                for l in display.lines() {
                    // Highlight @file mentions in ember color, $mentions in moss — all on wash bg
                    let mut spans: Vec<Span<'static>> = vec![Span::styled(
                        "     ".to_string(),
                        Style::default().fg(ASHEN.bone).bg(wash),
                    )];
                    let chars: Vec<char> = l.chars().collect();
                    let mut i = 0;
                    let mut buf = String::new();
                    let flush_buf = |spans: &mut Vec<Span<'static>>, buf: &mut String| {
                        if !buf.is_empty() {
                            spans.push(Span::styled(
                                std::mem::take(buf),
                                Style::default().fg(ASHEN.bone).bg(wash),
                            ));
                        }
                    };
                    while i < chars.len() {
                        if (chars[i] == '@' || chars[i] == '$')
                            && (i == 0
                                || chars[i - 1].is_whitespace()
                                || "(\"'`".contains(chars[i - 1]))
                        {
                            let prefix_char = chars[i];
                            // for $ require next char to be letter to avoid $5 etc.
                            if prefix_char == '$'
                                && i + 1 < chars.len()
                                && !chars[i + 1].is_ascii_alphabetic()
                                && chars[i + 1] != '_'
                            {
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
                                    Style::default()
                                        .fg(ASHEN.moss)
                                        .add_modifier(Modifier::BOLD)
                                        .bg(wash)
                                } else {
                                    Style::default()
                                        .fg(ASHEN.ember)
                                        .add_modifier(Modifier::BOLD)
                                        .bg(wash)
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
                    let mut line = Line::from(spans);
                    line.style = Style::default().bg(wash);
                    lines.push(line);
                }
                // Empty user message: still show an empty wash line so block is visible
                if lines.is_empty() {
                    let mut line = Line::from(vec![Span::styled(
                        "     ".to_string(),
                        Style::default().bg(wash),
                    )]);
                    line.style = Style::default().bg(wash);
                    lines.push(line);
                }
                // Top/bottom padding by one row — keeps highlight block breathing
                let pad = {
                    let mut l = Line::from(vec![Span::styled(
                        " ".to_string(),
                        Style::default().bg(wash),
                    )]);
                    l.style = Style::default().bg(wash);
                    l
                };
                let mut padded = Vec::with_capacity(lines.len() + 2);
                padded.push(pad.clone());
                padded.extend(lines);
                padded.push(pad);
                padded
            }
            "assistant" => {
                let wash = THEME.assistant_bg;
                let mut md = markdown::render_markdown(&self.content, 5);
                // Apply subtle moss wash to every non-code line; keep stone bg for code blocks nested inside
                for line in &mut md {
                    let is_code = line.spans.iter().any(|s| s.style.bg == Some(ASHEN.stone));
                    if is_code {
                        // Keep stone full-width for code lines (nested inside wash)
                        line.style = Style::default().bg(ASHEN.stone);
                        for span in &mut line.spans {
                            if span.style.bg.is_none() {
                                // Pad/indent spans inside code block should be stone too
                                span.style = span.style.bg(ASHEN.stone);
                            }
                        }
                        continue;
                    }
                    // Non-code: give wash — blank lines inside the block also get wash for solid block look
                    line.style = Style::default().bg(wash);
                    for span in &mut line.spans {
                        if span.style.bg.is_none() {
                            span.style = span.style.bg(wash);
                        }
                    }
                    // Empty line inside markdown (Line::from("")) has no spans — keep its line.style wash
                }
                // Top/bottom padding by one row for the highlight block
                let pad = {
                    let mut l = Line::from(vec![Span::styled(
                        " ".to_string(),
                        Style::default().bg(wash),
                    )]);
                    l.style = Style::default().bg(wash);
                    l
                };
                let mut padded = Vec::with_capacity(md.len() + 2);
                padded.push(pad.clone());
                padded.extend(md);
                padded.push(pad);
                padded
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
            "tool" => {
                if self.tool_id.is_some() {
                    self.render_tool_box()
                } else {
                    self.render_tool_lines()
                }
            }
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
            if let Some(pos) = self.content.find(" → ") {
                self.content[..pos].trim().to_string()
            } else if let Some(pos) = self.content.find(' ') {
                self.content[..pos].to_string()
            } else {
                self.content.clone()
            }
        });
        let args_str = self.tool_args.clone().unwrap_or_else(|| {
            if let Some(pos) = self.content.find(' ') {
                self.content[pos..].trim().to_string()
            } else {
                String::new()
            }
        });
        let has_result = self.content.contains(" → ");
        let result = if has_result {
            if let Some(pos) = self.content.find(" → ") {
                self.content[pos + " → ".len()..].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let is_running = self.elapsed_ms.is_none();
        let elapsed = self.format_elapsed();
        let is_error = result.contains("Error:")
            || result.contains("BLOCKED")
            || result.to_lowercase().contains("error");
        let border_col = if is_running {
            ASHEN.charcoal
        } else if is_error {
            ASHEN.ember
        } else {
            ASHEN.moss
        };
        let timer_str = if is_running {
            "… running".to_string()
        } else {
            elapsed.clone()
        };
        let header_title = if timer_str.is_empty() {
            name.clone()
        } else {
            format!("{} · {}", name, timer_str)
        };
        let mut inner: Vec<Line<'static>> = Vec::new();
        if !args_str.is_empty() && args_str != "{}" {
            let preview = format_tool_preview(&name, &args_str);
            inner.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(preview, Style::default().fg(ASHEN.smoke)),
            ]));
        }
        if has_result {
            inner.push(Line::from(Span::styled(
                "─".repeat(28),
                Style::default().fg(ASHEN.charcoal),
            )));
            let result_lines: Vec<&str> = result.lines().collect();
            let is_diff = (name == "edit" || name == "edit_file")
                || result.contains("diff:")
                || result.contains("@@");
            let max_lines = if is_diff { 12 } else { 3 };
            let show = result_lines.len().min(max_lines);
            for l in &result_lines[..show] {
                let style = if l.starts_with('+') && !l.starts_with("+++") {
                    Style::default().fg(ASHEN.moss)
                } else if l.starts_with('-') && !l.starts_with("---") {
                    Style::default().fg(ASHEN.ember)
                } else if l.starts_with("@@") {
                    Style::default()
                        .fg(ASHEN.frost)
                        .add_modifier(Modifier::BOLD)
                } else if is_diff {
                    Style::default().fg(ASHEN.deep_ash)
                } else {
                    Style::default().fg(ASHEN.smoke)
                };
                inner.push(Line::from(Span::styled(l.to_string(), style)));
            }
            if result_lines.len() > max_lines {
                inner.push(Line::from(Span::styled(
                    format!("… +{} more lines", result_lines.len() - max_lines),
                    Style::default().fg(ASHEN.charcoal),
                )));
            }
        } else if is_running {
            inner.push(Line::from(Span::styled(
                "  ● running",
                Style::default()
                    .fg(ASHEN.charcoal)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
        let width_est: usize = 60;
        let top = format!("┌─ {} ─", header_title);
        let top_border = {
            let rem = width_est.saturating_sub(top.chars().count() + 1);
            format!("{}{}┐", top, "─".repeat(rem))
        };
        let bottom = format!("└{}┘", "─".repeat(width_est));
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled(
            top_border,
            Style::default().fg(border_col),
        )));
        for l in inner {
            let mut spans = vec![Span::styled(
                "│ ".to_string(),
                Style::default().fg(border_col),
            )];
            spans.extend(l.spans);
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(Span::styled(
            bottom,
            Style::default().fg(border_col),
        )));
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
            let is_diff = (name == "edit" || name == "edit_file")
                || result.contains("diff:")
                || result.contains("@@");
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
                    Style::default()
                        .fg(ASHEN.frost)
                        .add_modifier(Modifier::BOLD)
                } else if is_diff {
                    Style::default().fg(ASHEN.deep_ash)
                } else {
                    Style::default().fg(ASHEN.smoke)
                };
                lines.push(Line::from(Span::styled(format!("      {}", l), style)));
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

fn draw_running_indicator(f: &mut Frame, area: Rect, busy: bool, tick: usize) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    if !busy {
        // Reserved 1-row slot but blank when idle — no layout jump
        let blank = " ".repeat(w);
        let para = Paragraph::new(Line::from(Span::styled(
            blank,
            Style::default().bg(THEME.page_bg),
        )));
        f.render_widget(para, area);
        return;
    }
    // Busy: "━" line with ping-pong pulse sliding left→right→left
    let seg = 8usize.min(w);
    if w <= seg {
        let line = "━".repeat(w);
        let para = Paragraph::new(Line::from(Span::styled(
            line,
            Style::default()
                .fg(THEME.accent)
                .bg(THEME.page_bg)
                .add_modifier(Modifier::BOLD),
        )));
        f.render_widget(para, area);
        return;
    }
    let cycle = (w - seg) * 2;
    let pos = tick % cycle.max(1);
    let offset = if pos < w - seg { pos } else { cycle - pos };
    let before = offset;
    let after = w - offset - seg;
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(3);
    if before > 0 {
        spans.push(Span::styled(
            "━".repeat(before),
            Style::default().fg(ASHEN.charcoal).bg(THEME.page_bg),
        ));
    }
    spans.push(Span::styled(
        "━".repeat(seg),
        Style::default()
            .fg(THEME.accent)
            .bg(THEME.page_bg)
            .add_modifier(Modifier::BOLD),
    ));
    if after > 0 {
        spans.push(Span::styled(
            "━".repeat(after),
            Style::default().fg(ASHEN.charcoal).bg(THEME.page_bg),
        ));
    }
    let para = Paragraph::new(Line::from(spans));
    f.render_widget(para, area);
}

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

/// Wrap lines to fit within a given width. Preserves line.style bg (used for
/// full-width wash behind user/assistant blocks) and pads each washed line to
/// `width` with trailing spaces so the bg extends to the right edge.
fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::new();
    for line in lines {
        // Preserve the line-level bg (wash) for wrapped segments.
        let line_bg = line.style.bg;
        // Calculate total text width of this line
        let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        if total <= width {
            let mut padded = line;
            let pad_len = width.saturating_sub(total);
            if pad_len > 0 {
                // Only pad lines that are part of a washed block (user/assistant).
                // Blank separator lines have no bg — leave them unpadded (page_bg).
                let wash_bg = padded
                    .style
                    .bg
                    .or_else(|| padded.spans.iter().find_map(|s| s.style.bg));
                if let Some(bg) = wash_bg {
                    padded
                        .spans
                        .push(Span::styled(" ".repeat(pad_len), Style::default().bg(bg)));
                    if padded.style.bg.is_none() {
                        padded.style = Style::default().bg(bg);
                    }
                }
            }
            out.push(padded);
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
                    let mut seg_line = Line::from(seg_spans);
                    if let Some(bg) = line_bg {
                        seg_line.style = Style::default().bg(bg);
                    } else if let Some(bg) = seg_line.spans.iter().find_map(|s| s.style.bg) {
                        seg_line.style = Style::default().bg(bg);
                    }
                    // Pad segment already exactly `width`, no extra filler needed
                    out.push(seg_line);
                    seg_spans = Vec::new();
                    remaining = width;
                }
            }
        }
        if !seg_spans.is_empty() {
            let total_seg: usize = seg_spans.iter().map(|s| s.content.chars().count()).sum();
            let pad_len = width.saturating_sub(total_seg);
            // Pad last wrapped segment to full width if it was part of a washed block
            let wash_bg = line_bg.or_else(|| seg_spans.iter().find_map(|s| s.style.bg));
            let mut seg_line = Line::from(seg_spans);
            if let Some(bg) = wash_bg {
                if pad_len > 0 {
                    seg_line
                        .spans
                        .push(Span::styled(" ".repeat(pad_len), Style::default().bg(bg)));
                }
                seg_line.style = Style::default().bg(bg);
            } else if let Some(bg) = line_bg {
                seg_line.style = Style::default().bg(bg);
            }
            out.push(seg_line);
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
    let filtered: Vec<Msg> = messages
        .iter()
        .filter(|m| !is_banner(&m.content))
        .cloned()
        .collect();
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
        "  ▸  type a message…  (/help • Enter send • Shift+Enter newline • Shift+Tab NORM/PLAN/ASK/AUTO)",
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

fn context_window_for_model(model: &str) -> usize {
    let lower = model.to_lowercase();
    if lower.contains("128k") {
        128_000
    } else if lower.contains("200k") {
        200_000
    } else if lower.contains("32k") {
        32_000
    } else if lower.contains("1m") || lower.contains("1000k") || lower.contains("1.0m") {
        1_000_000
    } else {
        1_000_000
    }
}

fn estimate_tokens(messages: &[Msg], model: &str) -> usize {
    // ~4 chars per token + per-message overhead + system prompt
    let mut chars: usize = 4000; // system prompt estimate
    for m in messages {
        chars += m.content.chars().count() + 8;
    }
    // Also include model name overhead
    chars += model.len();
    (chars + 3) / 4
}

fn format_context_label(est: usize, window: usize) -> String {
    let pct = ((est as f64 / window as f64) * 100.0).min(100.0);
    // Format window as 1.0M, 128k, etc.
    let window_str = if window >= 1_000_000 {
        format!("{:.1}M", window as f64 / 1_000_000.0)
    } else if window >= 1000 {
        format!("{}k", window / 1000)
    } else {
        format!("{}", window)
    };
    // Show pct as integer, but keep one decimal if <10%
    let pct_str = if pct < 10.0 {
        format!("{:.1}%", pct)
    } else {
        format!("{:.0}%", pct)
    };
    format!("{} / {}", pct_str, window_str)
}

fn draw_subagent_summary(f: &mut Frame, area: Rect, tick: usize) {
    let subs: Vec<_> = crate::services::agents::list_subagents()
        .into_iter()
        .filter(|s| s.status == "running")
        .collect();
    if subs.is_empty() {
        return;
    }
    let running = subs.len();
    let total = running;
    let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][tick % 10];
    let txt = if running > 0 {
        format!(
            " {}  ⬢ {} agents • {} running  {}",
            spinner, total, running, spinner
        )
    } else {
        format!(" ⬢ {} agents • all done ", total)
    };
    let style = if running > 0 {
        Style::default().fg(ASHEN.frost).bg(THEME.page_bg)
    } else {
        Style::default().fg(ASHEN.moss).bg(THEME.page_bg)
    };
    let para = Paragraph::new(Line::from(Span::styled(txt, style)));
    f.render_widget(para, area);
}

fn draw_subagent_list(f: &mut Frame, area: Rect, selected: usize, scroll: usize) {
    let mut subs: Vec<_> = crate::services::agents::list_subagents()
        .into_iter()
        .filter(|s| s.status == "running")
        .collect();
    // only running shown — done/killed are out of current session
    subs.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    let area = centered_rect(70, 60, area);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Subagents ({}) — Enter: view  Esc: close  Shift+K: kill ",
            subs.len()
        ))
        .style(Style::default().bg(THEME.page_bg))
        .border_style(Style::default().fg(ASHEN.frost));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if subs.is_empty() {
        let para = Paragraph::new(Line::from(Span::styled(
            "  No subagents — spawn via worker",
            Style::default().fg(ASHEN.charcoal),
        )));
        f.render_widget(para, inner);
        return;
    }
    // 2-row cards: header + single-line task (compact for 20-50 agents)
    let row_h: u16 = 2;
    let visible = ((inner.height / row_h) as usize).max(1).min(subs.len());
    let max_scroll = subs.len().saturating_sub(visible);
    let clamped_scroll = scroll.min(max_scroll);
    let start = clamped_scroll;
    let end = (start + visible).min(subs.len());
    let total = subs.len();
    let mut y = inner.y;
    for (idx, sub) in subs[start..end].iter().enumerate() {
        let abs_idx = start + idx;
        let is_sel = abs_idx == selected;
        let bg = if is_sel {
            THEME.input_bg
        } else {
            THEME.page_bg
        };
        let row_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: row_h,
        };
        let icon = if sub.status == "running" {
            "●"
        } else if sub.status == "done" {
            "✓"
        } else if sub.status == "killed" {
            "✕"
        } else {
            "✗"
        };
        let status_style = if sub.status == "running" {
            Style::default()
                .fg(ASHEN.frost)
                .add_modifier(Modifier::BOLD)
        } else if sub.status == "done" {
            Style::default().fg(ASHEN.moss)
        } else {
            Style::default().fg(ASHEN.ember)
        };
        let style = if is_sel {
            Style::default()
                .fg(ASHEN.bone)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ASHEN.smoke).bg(bg)
        };
        let elapsed = std::time::SystemTime::now()
            .duration_since(sub.started_at)
            .unwrap_or_default()
            .as_secs();
        let elapsed_str = if elapsed < 60 {
            format!("{}s", elapsed)
        } else {
            format!("{}m", elapsed / 60)
        };
        let short_id = &sub.id[..8.min(sub.id.len())];
        let label = if sub.label.is_empty() {
            sub.agent.clone()
        } else {
            sub.label.clone()
        };
        let header = Line::from(vec![
            Span::styled(format!(" {} ", icon), status_style.bg(bg)),
            Span::styled(
                label,
                Style::default()
                    .fg(ASHEN.bone)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" [{}]", sub.agent),
                Style::default().fg(ASHEN.smoke).bg(bg),
            ),
            Span::styled(format!(" [{}]", sub.status), status_style.bg(bg)),
            Span::styled(
                format!("  {}  {}", short_id, elapsed_str),
                Style::default().fg(ASHEN.deep_ash).bg(bg),
            ),
            Span::styled(
                format!(" {}", if is_sel { "◀" } else { "" }),
                Style::default().fg(ASHEN.charcoal).bg(bg),
            ),
        ]);
        f.render_widget(
            Paragraph::new(header).style(style),
            Rect {
                x: row_area.x,
                y: row_area.y,
                width: row_area.width,
                height: 1,
            },
        );
        let task_w = (inner.width as usize).saturating_sub(4);
        let task_one = sub.task.lines().next().unwrap_or("").trim();
        let task_disp = if task_one.chars().count() > task_w {
            format!(
                "{}…",
                task_one
                    .chars()
                    .take(task_w.saturating_sub(1))
                    .collect::<String>()
            )
        } else {
            task_one.to_string()
        };
        let task_line = Line::from(Span::styled(
            format!("  {}", task_disp),
            Style::default().fg(ASHEN.deep_ash).bg(bg),
        ));
        f.render_widget(
            Paragraph::new(task_line),
            Rect {
                x: row_area.x,
                y: row_area.y + 1,
                width: row_area.width,
                height: 1,
            },
        );
        y += row_h;
    }
    if total > visible {
        draw_scrollbar(f, inner, total, visible, clamped_scroll as u16);
    }
}

fn draw_subagent_detail(f: &mut Frame, area: Rect, idx: usize, scroll: u16) {
    let subs: Vec<_> = crate::services::agents::list_subagents()
        .into_iter()
        .filter(|s| s.status == "running")
        .collect();
    if idx >= subs.len() {
        return;
    }
    let sub = &subs[idx];
    f.render_widget(Clear, area);
    let elapsed_tmp = std::time::SystemTime::now()
        .duration_since(sub.started_at)
        .unwrap_or_default()
        .as_secs();
    let elapsed_str_tmp = if elapsed_tmp < 60 {
        format!("{}s", elapsed_tmp)
    } else {
        format!("{}m{}s", elapsed_tmp / 60, elapsed_tmp % 60)
    };
    let display_label_tmp = if sub.label.is_empty() {
        sub.agent.clone()
    } else {
        sub.label.clone()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {} [{}] — {} [{}] {} — Esc: back  Shift+K: kill ",
            display_label_tmp,
            sub.agent,
            &sub.id[..8.min(sub.id.len())],
            sub.status,
            elapsed_str_tmp
        ))
        .style(Style::default().bg(THEME.page_bg))
        .border_style(Style::default().fg(ASHEN.frost));
    let inner = block.inner(area);
    f.render_widget(block, area);
    // sticky header: detailed, always visible while transcript scrolls
    let mut header_raw: Vec<Line> = Vec::new();
    header_raw.push(Line::from(vec![
        Span::styled(
            format!("{} ", display_label_tmp),
            Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{}]", sub.agent), Style::default().fg(ASHEN.frost)),
        Span::styled(
            format!("  [{}]", sub.status),
            Style::default()
                .fg(ASHEN.frost)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}  {}", &sub.id[..8.min(sub.id.len())], elapsed_str_tmp),
            Style::default().fg(ASHEN.deep_ash),
        ),
    ]));
    for l in hard_wrap_str(&sub.task, (inner.width as usize).saturating_sub(6)).lines() {
        header_raw.push(Line::from(vec![
            Span::styled("Task: ", Style::default().fg(ASHEN.charcoal)),
            Span::styled(l.to_string(), Style::default().fg(ASHEN.bone)),
        ]));
    }
    header_raw.push(Line::from(Span::styled(
        format!(
            "Started: {:?}  Elapsed: {}",
            sub.started_at, elapsed_str_tmp
        ),
        Style::default().fg(ASHEN.deep_ash),
    )));
    header_raw.push(Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(ASHEN.charcoal),
    )));
    let header_wrapped = wrap_lines(header_raw, inner.width as usize);
    let header_h = (header_wrapped.len() as u16).min(inner.height);
    let header_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: header_h,
    };
    let content_area = Rect {
        x: inner.x,
        y: inner.y + header_h,
        width: inner.width,
        height: inner.height.saturating_sub(header_h),
    };
    f.render_widget(Paragraph::new(header_wrapped), header_area);
    let mut transcript_lines: Vec<Line> = Vec::new();
    if sub.transcript.is_empty() {
        transcript_lines.push(Line::from(Span::styled(
            "(no transcript yet — waiting for updates…)",
            Style::default()
                .fg(ASHEN.charcoal)
                .add_modifier(Modifier::ITALIC),
        )));
    } else {
        // Convert SubagentMsg -> Msg and reuse regular message rendering pipeline
        let msgs: Vec<Msg> = sub
            .transcript
            .iter()
            .map(|m| Msg {
                role: m.role.clone(),
                content: m.content.clone(),
                tool_id: m.tool_id.clone(),
                tool_name: m.tool_name.clone(),
                tool_args: m.tool_args.clone(),
                elapsed_ms: m.elapsed_ms,
            })
            .collect();
        let content_lines = build_content_lines(&msgs);
        transcript_lines.extend(content_lines);
    }
    let wrapped = wrap_lines(transcript_lines, content_area.width as usize);
    let total = wrapped.len();
    let viewport_h = content_area.height as usize;
    let clamped = (scroll as usize).min(total.saturating_sub(viewport_h)) as u16;
    let para = Paragraph::new(wrapped).scroll((clamped, 0));
    f.render_widget(para, content_area);
    if total > viewport_h {
        draw_scrollbar(f, content_area, total, viewport_h, clamped);
    }
}

fn detail_total_for_width(sub: &crate::services::agents::SubagentStatus, width: usize) -> usize {
    if sub.transcript.is_empty() {
        return 1;
    }
    let msgs: Vec<Msg> = sub
        .transcript
        .iter()
        .map(|m| Msg {
            role: m.role.clone(),
            content: m.content.clone(),
            tool_id: m.tool_id.clone(),
            tool_name: m.tool_name.clone(),
            tool_args: m.tool_args.clone(),
            elapsed_ms: m.elapsed_ms,
        })
        .collect();
    let lines = build_content_lines(&msgs);
    wrap_lines(lines, width).len()
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_footer(
    f: &mut Frame,
    area: Rect,
    model: &str,
    messages: &[Msg],
    cwd: &str,
    agent_busy: bool,
    spinner_tick: usize,
    dictate_state: crate::integrations::dictate::State,
    dictate_meter: &[f32; 6],
) {
    // Always clear footer area first to avoid ghosting when popup was over it
    f.render_widget(Clear, area);
    let width = area.width as usize;
    // Split footer into 2 rows: top = main footer, bottom = context window
    let top_area = if area.height >= 2 {
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        }
    } else {
        area
    };
    let bottom_area = if area.height >= 2 {
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        }
    } else {
        Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: 0,
        }
    };
    // Footer never shows MCP status — check via /mcp only (as requested)
    // Narrow terminal (<10 cols) – just show truncated model to avoid overflow/wrap bleed
    if width < 15 {
        let txt = if model.chars().count() > width.saturating_sub(2) {
            format!(
                " {}… ",
                model
                    .chars()
                    .take(width.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            format!(" {} ", model)
        };
        let para = Paragraph::new(Line::from(Span::styled(
            txt,
            Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
        )));
        f.render_widget(para, top_area);
        if area.height >= 2 {
            let est = estimate_tokens(messages, model);
            let window = context_window_for_model(model);
            let ctx = format_context_label(est, window);
            let ctx_txt = if ctx.chars().count() > width.saturating_sub(2) {
                format!(
                    " {}… ",
                    ctx.chars()
                        .take(width.saturating_sub(3))
                        .collect::<String>()
                )
            } else {
                format!(" {} ", ctx)
            };
            let cpara = Paragraph::new(Line::from(Span::styled(
                ctx_txt,
                Style::default().fg(ASHEN.deep_ash).bg(THEME.page_bg),
            )));
            f.render_widget(cpara, bottom_area);
        }
        return;
    }

    let spinner = dictate::SPINNER_FRAMES;
    let (mode_badge, mode_style) = match crate::agent::current_mode() {
        crate::agent::Mode::Norm => (
            format!(" NORM "),
            Style::default()
                .fg(ASHEN.slate)
                .bg(THEME.header_bg)
                .add_modifier(Modifier::BOLD),
        ),
        crate::agent::Mode::Plan => (
            format!(" PLAN "),
            Style::default()
                .fg(ASHEN.bone)
                .bg(ASHEN.frost)
                .add_modifier(Modifier::BOLD),
        ),
        crate::agent::Mode::Ask => (
            format!(" ASK "),
            Style::default()
                .fg(ASHEN.bone)
                .bg(ASHEN.moss)
                .add_modifier(Modifier::BOLD),
        ),
        crate::agent::Mode::Auto => (
            format!(" AUTO "),
            Style::default()
                .fg(ASHEN.bone)
                .bg(ASHEN.ember)
                .add_modifier(Modifier::BOLD),
        ),
    };
    let spinner_char = if agent_busy {
        format!("{} ", spinner[spinner_tick % spinner.len()])
    } else {
        String::new()
    };
    let dictate_right: Option<String> = match dictate_state {
        crate::integrations::dictate::State::Recording => {
            let meter_str = dictate::meter_string(dictate_meter);
            Some(format!("● {} listening…", meter_str))
        }
        crate::integrations::dictate::State::Transcribing => {
            let frame = spinner[spinner_tick % spinner.len()];
            Some(format!("{} transcribing…", frame))
        }
        crate::integrations::dictate::State::Idle => None,
    };
    let dictate_len = dictate_right
        .as_ref()
        .map(|s| s.chars().count() + 2)
        .unwrap_or(0);

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

    let used = spinner_char.chars().count()
        + mode_badge.chars().count()
        + 1
        + model.chars().count()
        + 2
        + center.chars().count()
        + dictate_len;
    let gap = if used < width { width - used } else { 0 };
    let gap_left = gap / 2;
    let gap_right = gap - gap_left;

    let mut spans = vec![Span::styled(
        format!(" {} ", model),
        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
    )];
    spans.push(Span::styled(mode_badge.clone(), mode_style));
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
    if let Some(prefix) = dictate_right.clone() {
        let is_recording = dictate_state == crate::integrations::dictate::State::Recording;
        if is_recording {
            if let Some(pos) = prefix.find('●') {
                let before = &prefix[..pos];
                let rest_start = pos + '●'.len_utf8();
                let after = &prefix[rest_start..];
                if !before.is_empty() {
                    spans.push(Span::styled(
                        format!(" {}", before),
                        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
                    ));
                } else {
                    spans.push(Span::styled(
                        " ".to_string(),
                        Style::default().bg(THEME.page_bg),
                    ));
                }
                spans.push(Span::styled(
                    "●".to_string(),
                    Style::default()
                        .fg(ASHEN.ember)
                        .bg(THEME.page_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                if !after.is_empty() {
                    spans.push(Span::styled(
                        format!("{} ", after),
                        Style::default().fg(ASHEN.smoke).bg(THEME.page_bg),
                    ));
                } else {
                    spans.push(Span::styled(
                        " ".to_string(),
                        Style::default().bg(THEME.page_bg),
                    ));
                }
            } else {
                spans.push(Span::styled(
                    format!(" {} ", prefix),
                    Style::default()
                        .fg(ASHEN.ember)
                        .bg(THEME.page_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        } else {
            spans.push(Span::styled(
                format!(" {} ", prefix),
                Style::default()
                    .fg(ASHEN.frost)
                    .bg(THEME.page_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    let footer = Paragraph::new(Line::from(spans));
    f.render_widget(footer, top_area);
    // Second row: context window
    if area.height >= 2 {
        let est = estimate_tokens(messages, model);
        let window = context_window_for_model(model);
        let ctx_label = format_context_label(est, window);
        let pct = ((est as f64 / window as f64) * 100.0).min(100.0);
        let bar_width = width.saturating_sub(ctx_label.chars().count() + 4).max(6);
        let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
        let ctx_line = format!(" {} {} ", bar, ctx_label);
        let display = if ctx_line.chars().count() > width {
            ctx_line.chars().take(width).collect::<String>()
        } else {
            ctx_line
        };
        let ctx_style = if pct > 85.0 {
            Style::default().fg(ASHEN.ember).bg(THEME.page_bg)
        } else if pct > 60.0 {
            Style::default().fg(ASHEN.frost).bg(THEME.page_bg)
        } else {
            Style::default().fg(ASHEN.deep_ash).bg(THEME.page_bg)
        };
        let cpara = Paragraph::new(Line::from(Span::styled(display, ctx_style)));
        f.render_widget(cpara, bottom_area);
    }
}

fn draw_scrollbar(f: &mut Frame, area: Rect, total_lines: usize, viewport_h: usize, scroll: u16) {
    if total_lines <= viewport_h {
        return;
    }
    let max_scroll = total_lines.saturating_sub(viewport_h);
    if scroll as usize >= max_scroll {
        return;
    } // hide when at bottom — only while not at bottom per spec
    let track_h = area.height as usize;
    if track_h == 0 {
        return;
    }
    let thumb_h = (((viewport_h as f64 / total_lines as f64) * track_h as f64)
        .max(1.0)
        .min(track_h as f64))
    .round() as usize;
    let thumb_y = if max_scroll == 0 {
        0
    } else {
        ((scroll as f64 / max_scroll as f64) * (track_h.saturating_sub(thumb_h)) as f64).round()
            as usize
    };
    for y in 0..track_h {
        let is_thumb = y >= thumb_y && y < thumb_y + thumb_h;
        let ch = if is_thumb { "█" } else { "░" };
        let style = if is_thumb {
            Style::default().fg(ASHEN.frost).bg(THEME.page_bg)
        } else {
            Style::default().fg(ASHEN.charcoal).bg(THEME.page_bg)
        };
        let cell = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y + y as u16,
            width: 1,
            height: 1,
        };
        f.render_widget(Paragraph::new(ch).style(style), cell);
    }
}

// ── Autocomplete + @-mentions ─────────────────────────────────

const COMMANDS: &[&str] = &[
    "/help",
    "/new",
    "/clear",
    "/exit",
    "/quit",
    "/model",
    "/sessions",
    "/resume",
    "/allowlist",
    "/allowlist clear",
    "/mcp",
    "/memory",
    "/memory stats",
    "/memory consolidate",
    "/auto-accept",
    "/plan",
    "/init",
];

/// Filter commands matching the current input prefix.
/// Also completes model aliases after `/model `.
fn autocomplete_matches(input: &str) -> Vec<String> {
    if input.is_empty() || !input.starts_with('/') {
        return Vec::new();
    }
    // Model alias completion: "/model " or "/model <prefix>"
    if input.starts_with("/model ") {
        let prefix = input.strip_prefix("/model ").unwrap_or("");
        if let Ok(cfg) = crate::integrations::models::load() {
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
    let root = crate::guards::dir::project_root();
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if name == ".git"
                || name == "target"
                || name == "node_modules"
                || name == ".next"
                || name == "dist"
                || name == "build"
            {
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
                let val = v
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim()
                    .to_string();
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
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if !ft.is_dir() {
                    continue;
                }
                let skill_path = entry.path().join("SKILL.md");
                if !skill_path.exists() {
                    continue;
                }
                let raw = std::fs::read_to_string(&skill_path).unwrap_or_default();
                let name = parse_skill_name(&raw)
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
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

fn collect_agent_names_sync() -> Vec<(String, String)> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let bases = vec![
        cwd.join("agents"),
        cwd.join(".lean").join("agents"),
        home.join(".lean").join("agents"),
    ];
    let mut map: HashMap<String, String> = HashMap::new();
    for base in &bases {
        if let Ok(rd) = std::fs::read_dir(base) {
            for entry in rd.filter_map(|e| e.ok()) {
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if !ft.is_dir() {
                    continue;
                }
                let ag = entry.path().join("AGENTS.md");
                if !ag.exists() {
                    continue;
                }
                let raw = std::fs::read_to_string(&ag).unwrap_or_default();
                let mut name = entry.file_name().to_string_lossy().to_string();
                let mut desc = String::new();
                for line in raw.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        let k = k.trim();
                        let v = v.trim().trim_matches('"').trim_matches('\'');
                        if k == "name" && !v.is_empty() {
                            name = v.to_string();
                        }
                        if k == "description" && !v.is_empty() && desc.is_empty() {
                            desc = v.to_string();
                        }
                    }
                }
                if !map.contains_key(&name) {
                    map.insert(name.clone(), desc);
                }
            }
        }
    }
    let mut v: Vec<(String, String)> = map.into_iter().collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn agent_autocomplete_matches(prefix: &str) -> Vec<String> {
    let all = collect_agent_names_sync();
    let lower = prefix.to_lowercase();
    let mut out = Vec::new();
    for (name, desc) in all {
        if lower.is_empty()
            || name.to_lowercase().starts_with(&lower)
            || name.to_lowercase().contains(&lower)
        {
            let display = if desc.is_empty() {
                format!("#{}", name)
            } else {
                let d = if desc.len() > 40 {
                    format!("{}…", &desc[..40])
                } else {
                    desc
                };
                format!("#{} — {}", name, d)
            };
            out.push(display);
            if out.len() >= 20 {
                break;
            }
        }
    }
    out
}

fn detect_agent_mention(textarea: &TextArea<'_>) -> Option<AtMention> {
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
    let target = "#";
    let mut at_col: Option<usize> = None;
    for i in (0..col).rev() {
        if i + target.len() <= chars.len() {
            let slice: String = chars[i..i + target.len()].iter().collect();
            if slice == target {
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
        }
    }
    let at = at_col?;
    let prefix_start = at + target.len();
    if prefix_start > col {
        return None;
    }
    let prefix_chars = &chars[prefix_start..col];
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

fn parse_all_forced_agents(input: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let known: std::collections::HashSet<String> = collect_agent_names_sync()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let mut i = 0usize;
    let chars: Vec<char> = input.chars().collect();
    while i < chars.len() {
        if chars[i] == '#' {
            let prev_ok = if i == 0 {
                true
            } else {
                let pc = chars[i - 1];
                pc.is_whitespace() || "(\"'`".contains(pc)
            };
            if prev_ok {
                let mut j = i + 1;
                while j < chars.len()
                    && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_')
                {
                    j += 1;
                }
                if j > i + 1 {
                    let name: String = chars[i + 1..j].iter().collect();
                    if known.contains(&name) {
                        // task is the rest of the sentence after name, but for inline hint we just want name
                        out.push((name, String::new()));
                        i = j;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
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
    if row >= lines.len() {
        return None;
    }
    let line = &lines[row];
    let chars: Vec<char> = line.chars().collect();
    if col > chars.len() {
        return None;
    }
    let mut at_col: Option<usize> = None;
    for i in (0..col).rev() {
        if chars[i] == '$' {
            let prev_ok = if i == 0 {
                true
            } else {
                let pc = chars[i - 1];
                pc.is_whitespace() || "(\"'`".contains(pc)
            };
            if prev_ok {
                // require next char is alpha after $ if prefix empty? allow empty but check
                at_col = Some(i);
                break;
            }
        }
    }
    let at = at_col?;
    let prefix_chars = &chars[at + 1..col];
    if prefix_chars.iter().any(|c| c.is_whitespace()) {
        return None;
    }
    // prefix for skills should be alphanumeric/_- only; reject if contains other punctuation like @ prefix contains /
    // allow empty prefix to show all
    let prefix: String = prefix_chars.iter().collect();
    // filter out purely numeric or with symbols that can't be skill names
    if !prefix.is_empty()
        && prefix
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        // still allow but we already filtered whitespace; keep it simple allow any but autocomplete will just not match
    }
    Some(AtMention {
        prefix,
        row,
        col,
        at_col: at,
    })
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
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if !ft.is_dir() {
                    continue;
                }
                let skill_path = entry.path().join("SKILL.md");
                if !skill_path.exists() {
                    continue;
                }
                let raw = std::fs::read_to_string(&skill_path).unwrap_or_default();
                let sname = parse_skill_name(&raw)
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
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

use std::sync::{Mutex, OnceLock};
static PASTED_IMAGES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
fn pasted_store() -> &'static Mutex<Vec<String>> {
    PASTED_IMAGES.get_or_init(|| Mutex::new(Vec::new()))
}

fn clipboard_image_marker() -> Option<String> {
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    // Windows: PowerShell Get-Clipboard -Format Image -> temp file
    #[cfg(target_os = "windows")]
    {
        let tmp = std::env::temp_dir().join(format!("lean_clip_{}.png", std::process::id()));
        let ps = format!("$img = Get-Clipboard -Format Image -ErrorAction SilentlyContinue; if ($img -ne $null) {{ $img.Save('{}'); Write-Host 'ok' }} else {{ Write-Host 'no' }}", tmp.display().to_string().replace('\\', "/"));
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .ok()?;
        if String::from_utf8_lossy(&out.stdout).contains("ok") {
            if let Ok(bytes) = std::fs::read(&tmp) {
                let _ = std::fs::remove_file(&tmp);
                if !bytes.is_empty() && bytes.len() <= MAX_BYTES {
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    return Some(format!("<<IMAGE:image/png:{}>>", b64));
                } else if bytes.len() > MAX_BYTES {
                    return Some(format!(
                        "[image skipped: clipboard {} — over 4MB]",
                        crate::tools::human_size(bytes.len())
                    ));
                }
            }
        }
        // also try pwsh
        let out2 = std::process::Command::new("pwsh")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .ok();
        if let Some(o) = out2 {
            if String::from_utf8_lossy(&o.stdout).contains("ok") {
                if let Ok(bytes) = std::fs::read(&tmp) {
                    let _ = std::fs::remove_file(&tmp);
                    if !bytes.is_empty() {
                        let b64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &bytes,
                        );
                        return Some(format!("<<IMAGE:image/png:{}>>", b64));
                    }
                }
            }
        }
        return None;
    }
    // macOS: pngpaste or osascript -> temp file
    #[cfg(target_os = "macos")]
    {
        let tmp = std::env::temp_dir().join(format!("lean_clip_{}.png", std::process::id()));
        for prog in [
            "pngpaste",
            "/opt/homebrew/bin/pngpaste",
            "/usr/local/bin/pngpaste",
        ] {
            let out = std::process::Command::new(prog).arg(&tmp).output().ok();
            if let Some(o) = out {
                if o.status.success() && tmp.exists() {
                    if let Ok(bytes) = std::fs::read(&tmp) {
                        let _ = std::fs::remove_file(&tmp);
                        if !bytes.is_empty() && bytes.len() <= MAX_BYTES {
                            let b64 = base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &bytes,
                            );
                            return Some(format!("<<IMAGE:image/png:{}>>", b64));
                        } else if bytes.len() > MAX_BYTES {
                            return Some(format!(
                                "[image skipped: clipboard {} — over 4MB]",
                                crate::tools::human_size(bytes.len())
                            ));
                        }
                    }
                }
            }
        }
        // osascript fallback: save clipboard PNG to temp
        let osa = format!("set f to \"{}\"\ntry\n  set img to the clipboard as «class PNGf»\n  set out to open for access f with write permission\n  set eof of out to 0\n  write img to out\n  close access out\n  return \"ok\"\non error\n  return \"no\"\nend try", tmp.display());
        let out = std::process::Command::new("osascript")
            .args(["-e", &osa])
            .output()
            .ok()?;
        if String::from_utf8_lossy(&out.stdout).contains("ok") {
            if let Ok(bytes) = std::fs::read(&tmp) {
                let _ = std::fs::remove_file(&tmp);
                if !bytes.is_empty() {
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    return Some(format!("<<IMAGE:image/png:{}>>", b64));
                }
            }
        }
        return None;
    }
    // Linux / other Unix
    let try_bytes = |prog: &str, args: &[&str]| -> Option<Vec<u8>> {
        let mut cmd = std::process::Command::new(prog);
        cmd.args(args);
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            // Ghostty on Wayland often has wayland-0/1 but agent env may lack it
            if std::path::Path::new("/run/user/1000/wayland-0").exists() {
                cmd.env("WAYLAND_DISPLAY", "wayland-0");
            } else if std::path::Path::new("/run/user/1000/wayland-1").exists() {
                cmd.env("WAYLAND_DISPLAY", "wayland-1");
            }
            if std::env::var("XDG_RUNTIME_DIR").is_err()
                && std::path::Path::new("/run/user/1000").exists()
            {
                cmd.env("XDG_RUNTIME_DIR", "/run/user/1000");
            }
        } else if std::env::var("XDG_RUNTIME_DIR").is_err() {
            cmd.env("XDG_RUNTIME_DIR", "/run/user/1000");
        }
        let out = cmd.output().ok()?;
        if !out.status.success() || out.stdout.is_empty() {
            return None;
        }
        if out.stdout.len() > 8
            && (out.stdout.starts_with(&[0x89, b'P', b'N', b'G'])
                || out.stdout[0] == 0xFF && out.stdout[1] == 0xD8
                || out.stdout.starts_with(b"GIF"))
        {
            Some(out.stdout)
        } else if out.stdout.len() > 100 {
            Some(out.stdout)
        } else {
            None
        }
    };
    let try_types = || -> Vec<String> {
        let mut cmd = std::process::Command::new("wl-paste");
        cmd.arg("--list-types");
        if std::env::var("WAYLAND_DISPLAY").is_err()
            && std::path::Path::new("/run/user/1000/wayland-0").exists()
        {
            cmd.env("WAYLAND_DISPLAY", "wayland-0");
        }
        cmd.output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default()
            .lines()
            .map(|s| s.trim().to_string())
            .collect()
    };
    let types = try_types();
    let has_png = types.iter().any(|t| t == "image/png");
    let has_jpeg = types
        .iter()
        .any(|t| t.contains("jpeg") || t.contains("jpg"));
    // Wayland first, then X11 variants — prefer detected mime
    let mut candidates: Vec<Option<Vec<u8>>> = Vec::new();
    if has_png || types.is_empty() {
        candidates.push(try_bytes("wl-paste", &["--type", "image/png"]));
    }
    if has_jpeg {
        candidates.push(try_bytes("wl-paste", &["--type", "image/jpeg"]));
    }
    candidates.push(try_bytes("wl-paste", &["-t", "image/png"]));
    candidates.push(try_bytes("wl-paste", &["--type", "image/jpeg"]));
    candidates.push(try_bytes("wl-paste", &[])); // raw, let compositor pick
    candidates.push(try_bytes(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
    ));
    candidates.push(try_bytes("xsel", &["--clipboard", "--output"]));
    for maybe in candidates {
        if let Some(bytes) = maybe {
            if bytes.is_empty() {
                continue;
            }
            if bytes.len() > MAX_BYTES {
                // over cap — skip with hint instead of marker
                return Some(format!(
                    "[image skipped: clipboard {} — over 4MB]",
                    crate::tools::human_size(bytes.len())
                ));
            }
            // sniff mime
            let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                "image/png"
            } else if bytes.starts_with(&[0xFF, 0xD8]) {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF") {
                "image/gif"
            } else if bytes.len() > 12 && bytes[8..12] == *b"WEBP" {
                "image/webp"
            } else {
                "image/png"
            };
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            return Some(format!("<<IMAGE:{}:{}>>", mime, b64));
        }
    }
    None
}

fn current_completions(textarea: &TextArea<'_>) -> Vec<String> {
    if let Some(m) = detect_agent_mention(textarea) {
        return agent_autocomplete_matches(&m.prefix);
    }
    if let Some(m) = detect_skill_mention(textarea) {
        return skill_autocomplete_matches(&m.prefix);
    }
    if let Some(m) = detect_at_mention(textarea) {
        return file_autocomplete_matches(&m.prefix);
    }
    let cur = textarea
        .lines()
        .get(textarea.cursor().0)
        .cloned()
        .unwrap_or_default();
    autocomplete_matches(&cur)
}

fn expand_at_mentions(prompt: &str) -> String {
    // Find @<path> tokens that resolve to existing files and $skill tokens that force skills, appending contents.
    // Images (@*.png etc.) are encoded as <<IMAGE:mime:b64>> (handled by agent.rs, shown as placeholder in TUI).
    // Remote @https:// URLs are fetched (4MB cap) or left as URL for vision models.
    let root = crate::guards::dir::project_root();
    let mut files: Vec<String> = Vec::new();
    let mut remotes: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let chars: Vec<char> = prompt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            let prev_ok = if i == 0 {
                true
            } else {
                chars[i - 1].is_whitespace() || "(\"'`".contains(chars[i - 1])
            };
            let next_is_alpha =
                i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_');
            if prev_ok && next_is_alpha {
                let mut j = i + 1;
                while j < chars.len() && !chars[j].is_whitespace() {
                    j += 1;
                }
                let mut raw: String = chars[i + 1..j].iter().collect();
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
                // skill names allowed chars: alnum - _
                let cleaned: String = raw
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                let skill_name = if !cleaned.is_empty() {
                    cleaned
                } else {
                    raw.clone()
                };
                if !skill_name.is_empty()
                    && !skills.contains(&skill_name)
                    && find_skill_content(&skill_name).is_some()
                {
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
                if raw.starts_with("http://") || raw.starts_with("https://") {
                    if !remotes.contains(&raw) && !files.contains(&raw) {
                        remotes.push(raw);
                    }
                } else if !raw.is_empty() && !files.contains(&raw) {
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
    // Expand pasted [[IMAGE #N]] placeholders (Ctrl+V) — replace with actual marker before counting
    let mut pasted_expanded = prompt.to_string();
    if prompt.contains("[[IMAGE #") {
        if let Ok(store) = pasted_store().lock() {
            for (idx, marker) in store.iter().enumerate() {
                let ph = format!("[[IMAGE #{}]]", idx + 1);
                if pasted_expanded.contains(&ph) {
                    pasted_expanded = pasted_expanded.replace(&ph, marker);
                }
            }
        }
    }
    if files.is_empty() && remotes.is_empty() && skills.is_empty() && pasted_expanded == prompt {
        return prompt.to_string();
    }
    let mut out = pasted_expanded;
    const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
    const MAX_IMAGES_PER_TURN: usize = 5;
    let mut image_count = out.matches("<<IMAGE").count(); // count already-expanded pasted images
                                                          // If pasted images already hit cap, warn
    if image_count >= MAX_IMAGES_PER_TURN && (!files.is_empty() || !remotes.is_empty()) {
        out.push_str(&format!(
            "\n\n[image note: already {} images from paste — max {} /turn, @files may be skipped]",
            image_count, MAX_IMAGES_PER_TURN
        ));
    }
    for rel in files {
        if image_count >= MAX_IMAGES_PER_TURN {
            out.push_str(&format!(
                "\n\n[image skipped: {} — max {} images/turn]",
                rel, MAX_IMAGES_PER_TURN
            ));
            continue;
        }
        let p = std::path::Path::new(&rel);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        if crate::tools::is_image_file(&rel) {
            image_count += 1;
            match std::fs::read(&full) {
                Ok(bytes) => {
                    if bytes.len() > MAX_IMAGE_BYTES {
                        out.push_str(&format!("\n\n[image skipped: {} ({}) — over 4MB, compress or use smaller image]", rel, crate::tools::human_size(bytes.len())));
                        continue;
                    }
                    let ext = full
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let mime = crate::tools::image_mime_type(&ext).unwrap_or("image/png");
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    out.push_str(&format!(
                        "\n\n[File: {} ({}, {})]\n<<IMAGE:{}:{}>>",
                        rel,
                        mime,
                        crate::tools::human_size(bytes.len()),
                        mime,
                        b64
                    ));
                }
                Err(e) => out.push_str(&format!("\n\n[read error: {}: {}]", rel, e)),
            }
        } else {
            let content =
                std::fs::read_to_string(&full).unwrap_or_else(|e| format!("[read error: {}]", e));
            let truncated = if content.len() > 8000 {
                format!(
                    "{}… [truncated {} chars]",
                    &content[..8000],
                    content.len() - 8000
                )
            } else {
                content
            };
            let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("");
            out.push_str(&format!(
                "\n\n[File: {}]\n```{}\n{}```",
                rel, ext, truncated
            ));
        }
    }
    for url in remotes {
        if image_count >= MAX_IMAGES_PER_TURN {
            out.push_str(&format!(
                "\n\n[image skipped: {} — max {} images/turn]",
                url, MAX_IMAGES_PER_TURN
            ));
            continue;
        }
        // For @https:// URLs, pass through as image_url placeholder (agent will send as input_image URL).
        // If extension looks like image or URL is likely image, treat as image; otherwise as file reference.
        let lower = url.to_lowercase();
        let is_image_url = lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".gif")
            || lower.ends_with(".webp")
            || lower.ends_with(".bmp")
            || lower.contains("images.unsplash")
            || lower.contains("image")
            || true; // default to image for vision UX
        if is_image_url {
            image_count += 1;
            // Keep URL as-is; agent.rs will forward as image_url without base64 if we embed marker with URL.
            // We use a lightweight marker that llm.rs can map to input_image url.
            out.push_str(&format!(
                "\n\n[Remote image: {}]\n<<IMAGE_URL:{}>>",
                url, url
            ));
        } else {
            out.push_str(&format!("\n\n[Remote file: {}]", url));
        }
    }
    for skill_name in skills {
        if let Some(content) = find_skill_content(&skill_name) {
            let truncated = if content.len() > 12000 {
                format!(
                    "{}… [truncated {} chars]",
                    &content[..12000],
                    content.len() - 12000
                )
            } else {
                content
            };
            out.push_str(&format!(
                "\n\n[Forced skill: {} — follow its workflow explicitly]\n{}",
                skill_name, truncated
            ));
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

fn draw_approval(f: &mut Frame, area: Rect, req: &crate::guards::approval::ApprovalRequest) {
    let width = (area.width.saturating_sub(4)).min(90);
    let queued = crate::guards::approval::queue_len();
    let queue_label = if queued > 0 {
        format!(" [{}/{}]", 1, queued + 1)
    } else {
        String::new()
    };
    let height = 9.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let is_dir = req.reasons.iter().any(|r| r.contains("outside CWD"));
    let is_mcp = req.reasons.iter().any(|r| r.contains("MCP tool"));
    let title = if is_mcp {
        format!(" MCP Guard — Approval Required{} ", queue_label)
    } else if is_dir {
        format!(
            " Dir Guard — Approval Required{} (outside CWD) ",
            queue_label
        )
    } else {
        format!(" Bash Guard — Approval Required{} ", queue_label)
    };
    let border_col = if is_mcp {
        ASHEN.moss
    } else if is_dir {
        ASHEN.frost
    } else {
        ASHEN.ember
    };
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let sev = match req.severity {
        crate::guards::bash::Severity::High => "HIGH",
        crate::guards::bash::Severity::Medium => "MEDIUM",
    };
    let sev_style = if sev == "HIGH" {
        Style::default()
            .fg(ASHEN.ember)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ASHEN.frost)
    };
    let cmd_label = if is_mcp {
        format!("  MCP: {}", req.cmd)
    } else if is_dir && req.cmd.contains(' ') && !req.cmd.contains('/') {
        format!("  $ {}", req.cmd)
    } else if is_dir {
        format!("  path: {}", req.cmd)
    } else {
        format!("  $ {}", req.cmd)
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("  {} risk:  ", sev), sev_style),
            Span::styled(req.reasons.join("; "), Style::default().fg(ASHEN.smoke)),
        ]),
        Line::from(Span::styled(cmd_label, Style::default().fg(ASHEN.whisper))),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [a] Allow once  ",
                Style::default().fg(ASHEN.moss).add_modifier(Modifier::BOLD),
            ),
            Span::styled("[d] Deny  ", Style::default().fg(ASHEN.ember)),
            Span::styled("[A] Allow always  ", Style::default().fg(ASHEN.slate)),
            Span::styled("Esc deny", Style::default().fg(ASHEN.charcoal)),
        ]),
        Line::from(Span::styled(
            "  Press a/d/A or Enter to allow, Esc to deny",
            Style::default().fg(ASHEN.deep_ash),
        )),
    ];
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn draw_sudo(
    f: &mut Frame,
    area: Rect,
    cmd: &str,
    input: &str,
    error: Option<&str>,
    attempt: usize,
) {
    let width = (area.width.saturating_sub(6)).min(72);
    let height = if error.is_some() { 9 } else { 8 };
    let height = height.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let block = Block::default()
        .title(format!(
            " Sudo — Password Required (attempt {}/3) ",
            attempt
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.ember))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let short_cmd = if cmd.len() > width as usize - 4 {
        format!("{}…", &cmd[..(width as usize - 5)])
    } else {
        cmd.to_string()
    };
    let masked = "•".repeat(input.chars().count());
    let display = if masked.is_empty() {
        "▏".to_string()
    } else {
        format!("{}▏", masked)
    };
    let mut lines = vec![
        Line::from(Span::styled(
            format!("  $ {}", short_cmd),
            Style::default().fg(ASHEN.whisper),
        )),
        Line::from(Span::styled(
            "  Password (masked, not logged):",
            Style::default().fg(ASHEN.smoke),
        )),
        Line::from(vec![
            Span::styled(
                "  > ",
                Style::default()
                    .fg(ASHEN.frost)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(display, Style::default().fg(ASHEN.bone)),
        ]),
    ];
    if let Some(err) = error {
        lines.push(Line::from(Span::styled(
            format!("  ✗ {}", err),
            Style::default().fg(ASHEN.ember),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Enter submit · Esc cancel",
        Style::default().fg(ASHEN.deep_ash),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_question(
    f: &mut Frame,
    area: Rect,
    req: &crate::services::question::AskRequest,
    wizard: &crate::services::question::Wizard,
) {
    let _ = req;
    let width = (area.width.saturating_sub(6)).min(84);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let wrap_cols = width.saturating_sub(6) as usize;

    let mut body: Vec<Line> = Vec::new();
    if let Some(q) = wizard.current() {
        if let Some(h) = &q.header {
            body.push(Line::from(Span::styled(
                format!("  {}", h),
                Style::default()
                    .fg(ASHEN.frost)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        for l in hard_wrap_str(&q.question, wrap_cols).lines() {
            body.push(Line::from(Span::styled(
                format!("  {}", l),
                Style::default().fg(ASHEN.bone),
            )));
        }
        body.push(Line::from(""));
        if q.multi_select {
            body.push(Line::from(Span::styled(
                "  select all that apply",
                Style::default()
                    .fg(ASHEN.deep_ash)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
        for (i, opt) in q.options.iter().enumerate() {
            let current = wizard.opt_idx() == i;
            let cursor = if current { "›" } else { " " };
            let marker = if q.multi_select {
                if wizard.is_toggled(i) {
                    "[x]"
                } else {
                    "[ ]"
                }
            } else if current {
                "◉"
            } else {
                "○"
            };
            let style = if current {
                Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            body.push(Line::from(vec![
                Span::styled(
                    format!("  {} {} ", cursor, marker),
                    Style::default().fg(ASHEN.frost),
                ),
                Span::styled(opt.label.clone(), style),
            ]));
            if let Some(desc) = &opt.description {
                for l in hard_wrap_str(desc, wrap_cols.saturating_sub(6)).lines() {
                    body.push(Line::from(Span::styled(
                        format!("       {}", l),
                        Style::default().fg(ASHEN.deep_ash),
                    )));
                }
            }
        }
        let other_current = wizard.on_other();
        let cursor = if other_current { "›" } else { " " };
        let other_text = wizard.other_text();
        let marker = if q.multi_select {
            if other_text.trim().is_empty() {
                "[ ]"
            } else {
                "[x]"
            }
        } else if other_current {
            "◉"
        } else {
            "○"
        };
        let shown = if other_current {
            format!("{}▏", other_text)
        } else if other_text.is_empty() {
            "Other…".to_string()
        } else {
            other_text.to_string()
        };
        let style = if other_current {
            Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ASHEN.smoke)
        };
        body.push(Line::from(vec![
            Span::styled(
                format!("  {} {} ", cursor, marker),
                Style::default().fg(ASHEN.frost),
            ),
            Span::styled(shown, style),
        ]));
    }
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        "  ↑/↓ choose · Space toggle · Enter confirm · ← back · Esc skip",
        Style::default().fg(ASHEN.charcoal),
    )));

    let height = ((body.len() as u16) + 2)
        .min(area.height.saturating_sub(4))
        .max(5);
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };

    let title = format!(
        " Question [{}/{}] ",
        wizard.idx() + 1,
        wizard.total().max(1)
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.frost))
        .style(Style::default().bg(THEME.header_bg).fg(ASHEN.bone));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(body), inner);
}

fn draw_sessions(
    f: &mut Frame,
    area: Rect,
    selected: usize,
    scroll: usize,
    filter: &str,
    show_all: bool,
    cwd: &str,
) {
    let all = crate::services::session::Session::list();
    // Per-directory filtering: default shows only sessions for current cwd, Tab toggles all
    let cwd_norm = cwd.trim_end_matches('/');
    let dir_filtered: Vec<crate::services::session::Session> = if show_all {
        all
    } else {
        all.into_iter()
            .filter(|s| s.cwd.trim_end_matches('/') == cwd_norm)
            .collect()
    };
    let filtered: Vec<crate::services::session::Session> = if filter.is_empty() {
        dir_filtered
    } else {
        let lower = filter.to_lowercase();
        dir_filtered
            .into_iter()
            .filter(|s| {
                s.id.to_lowercase().contains(&lower)
                    || s.cwd.to_lowercase().contains(&lower)
                    || s.messages
                        .iter()
                        .any(|m| m.content.to_lowercase().contains(&lower))
            })
            .collect()
    };
    let sessions = &filtered;
    let width = (area.width.saturating_sub(4)).min(80);
    let height = (14.min(sessions.len() + 6) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let mode_label = if show_all { "all" } else { "this dir" };
    let toggle_label = if show_all { "this dir" } else { "all" };
    let title = if filter.is_empty() {
        format!(
            " Sessions [{}] — Enter resume, Esc close, Tab:{} ",
            mode_label, toggle_label
        )
    } else {
        format!(
            " Sessions [{}] — filter: {} (Tab:{}) ",
            mode_label, filter, toggle_label
        )
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
        let para = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(ASHEN.deep_ash),
        )));
        f.render_widget(para, inner);
        return;
    }
    let visible_h = inner.height as usize;
    let start = scroll.min(sessions.len().saturating_sub(1));
    let end = (start + visible_h).min(sessions.len());
    let items: Vec<Line> = sessions[start..end]
        .iter()
        .enumerate()
        .map(|(i, sess)| {
            let idx = start + i;
            let sel = idx == selected;
            let style = if sel {
                Style::default()
                    .fg(ASHEN.bone)
                    .add_modifier(Modifier::BOLD)
                    .bg(ASHEN.stone)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            let ts = sess.updated_at;
            let preview = sess
                .messages
                .first()
                .map(|m| {
                    m.content
                        .chars()
                        .take(30)
                        .collect::<String>()
                        .replace("\n", " ")
                })
                .unwrap_or_else(|| "-".to_string());
            let id_short = sess.id.chars().take(8).collect::<String>();
            Line::from(Span::styled(
                format!(" {} {} {}m {} ", id_short, preview, sess.messages.len(), ts),
                style,
            ))
        })
        .collect();
    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}

fn draw_mcp(f: &mut Frame, area: Rect, selected: usize, scroll: usize) {
    let servers = crate::integrations::mcp::snapshot();
    let width = (area.width.saturating_sub(4)).min(90);
    let height = (18.min(servers.len() * 2 + 7) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let connected = servers
        .iter()
        .filter(|s| matches!(s.status, crate::integrations::mcp::ServerStatus::Connected))
        .count();
    let total = servers.len();
    let global_off = crate::integrations::mcp::is_global_disabled();
    let mut title = if total == 0 {
        " MCP Servers — No servers configured ".to_string()
    } else if global_off {
        format!(
            " MCP Servers [GLOBAL OFF] ({}/{} connected) — g/enable, Esc close ",
            connected, total
        )
    } else {
        format!(
            " MCP Servers ({}/{} connected) — d/toggle, g/global, r/reconnect, Esc close ",
            connected, total
        )
    };
    // Truncate title to fit popup width for narrow terminals (<10 cols)
    if title.chars().count() > width as usize - 4 {
        title = format!(
            "{}… ",
            title.chars().take(width as usize - 5).collect::<String>()
        );
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
            Line::from(Span::styled(
                "  No MCP servers configured",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(Span::styled(
                "  Create ~/.lean/mcp.json or .lean/mcp.json",
                Style::default().fg(ASHEN.charcoal),
            )),
            Line::from(Span::styled(
                "  See mcp.json.example for format",
                Style::default().fg(ASHEN.charcoal),
            )),
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
        let style = if sel {
            Style::default()
                .fg(ASHEN.bone)
                .add_modifier(Modifier::BOLD)
                .bg(ASHEN.stone)
        } else {
            Style::default().fg(ASHEN.smoke)
        };
        let (status_str, status_style) = match srv.status {
            crate::integrations::mcp::ServerStatus::Connected => {
                ("connected", Style::default().fg(ASHEN.moss))
            }
            crate::integrations::mcp::ServerStatus::Connecting => {
                ("connecting", Style::default().fg(ASHEN.frost))
            }
            crate::integrations::mcp::ServerStatus::Error(_) => {
                ("error", Style::default().fg(ASHEN.ember))
            }
            crate::integrations::mcp::ServerStatus::Disabled => {
                ("disabled", Style::default().fg(ASHEN.charcoal))
            }
        };
        let tool_cnt = srv.tools.len();
        let header = Line::from(vec![
            Span::styled(format!("  {} ", srv.name), style),
            Span::styled(format!("[{}] ", status_str), status_style),
            Span::styled(
                format!("{} tools", tool_cnt),
                Style::default().fg(ASHEN.charcoal),
            ),
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
            if txt.chars().count() > detail_max {
                txt = format!(
                    "{}…",
                    txt.chars()
                        .take(detail_max.saturating_sub(1))
                        .collect::<String>()
                );
            }
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
    let bash_list = crate::guards::bash::allowlist_list();
    let dir_list = crate::guards::dir::allowlist_list();
    let mut combined: Vec<(String, String)> = Vec::new(); // (kind, pat)
    for p in bash_list {
        combined.push(("bash".into(), p));
    }
    for p in dir_list {
        combined.push(("dir".into(), p));
    }
    combined.sort_by(|a, b| a.1.cmp(&b.1));
    let width = (area.width.saturating_sub(4)).min(80);
    let height = (16.min(combined.len() + 7) as u16).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let title = format!(
        " Allowlist (bash:{}, dir:{}) — Enter keep, d delete, c clear all, Esc close ",
        crate::guards::bash::allowlist_list().len(),
        crate::guards::dir::allowlist_list().len()
    );
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
            Line::from(Span::styled(
                "  No allowlisted patterns",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(Span::styled(
                "  Approve a bash/dir command with 'A' to add",
                Style::default().fg(ASHEN.charcoal),
            )),
        ]);
        f.render_widget(para, inner);
        return;
    }
    let visible_h = inner.height as usize;
    let start = scroll.min(combined.len().saturating_sub(1));
    let end = (start + visible_h).min(combined.len());
    let items: Vec<Line> = combined[start..end]
        .iter()
        .enumerate()
        .map(|(i, (kind, pat))| {
            let idx = start + i;
            let sel = idx == selected;
            let style = if sel {
                Style::default()
                    .fg(ASHEN.bone)
                    .add_modifier(Modifier::BOLD)
                    .bg(ASHEN.stone)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            let kind_style = if kind == "dir" {
                Style::default().fg(ASHEN.frost)
            } else {
                Style::default().fg(ASHEN.moss)
            };
            Line::from(vec![
                Span::styled(format!("  [{}] ", kind), kind_style),
                Span::styled(pat.clone(), style),
            ])
        })
        .collect();
    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}

/// Spawn the agent task, always sending a done signal on completion (including panics).
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

fn llm_history_for_spawn(
    session: &Option<crate::services::session::Session>,
    msgs: &[Msg],
) -> Vec<serde_json::Value> {
    if let Some(sess) = session {
        if let Some(ref hist) = sess.llm_history {
            if !hist.is_empty() {
                return hist.clone();
            }
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
        let list = crate::services::session::Session::list();
        let found = list.into_iter().find(|sess| sess.id.starts_with(rid));
        match found {
            Some(sess) => {
                messages = sess
                    .messages
                    .iter()
                    .map(|m| Msg {
                        role: m.role.clone(),
                        content: m.content.clone(),
                        tool_id: None,
                        tool_name: None,
                        tool_args: None,
                        elapsed_ms: None,
                    })
                    .collect();
                Some(sess)
            }
            None => {
                // not found, start new but warn
                let m = Msg {
                    role: "system".into(),
                    content: format!("[session {} not found, started new]", rid),
                    tool_id: None,
                    tool_name: None,
                    tool_args: None,
                    elapsed_ms: None,
                };
                messages.push(m);
                Some(crate::services::session::Session::new(&model))
            }
        }
    } else if opts.continue_session {
        if let Some(sess) = crate::services::session::Session::latest() {
            messages = sess
                .messages
                .iter()
                .map(|m| Msg {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    tool_id: None,
                    tool_name: None,
                    tool_args: None,
                    elapsed_ms: None,
                })
                .collect();
            Some(sess)
        } else {
            Some(crate::services::session::Session::new(&model))
        }
    } else {
        Some(crate::services::session::Session::new(&model))
    };
    // TODO: remove if unnecessary
    // Warn if resumed session has a legacy alias not in models.json
    if let Some(ref sess) = session {
        if crate::integrations::models::resolve(Some(&sess.model)).is_err() {
            messages.push(Msg {
                role: "system".into(),
                content: format!(
                    "[warn: session model '{}' not in {} — use /model to switch]",
                    sess.model,
                    crate::integrations::models::path_display()
                ),
                tool_id: None,
                tool_name: None,
                tool_args: None,
                elapsed_ms: None,
            });
        }
    }

    // ── Herdr: report session identity ──
    {
        let start_source = if opts.resume_id.is_some() {
            Some("resume")
        } else if opts.continue_session {
            Some("continue")
        } else if opts.no_session {
            None
        } else {
            Some("startup")
        };
        if let Some(ref sess) = session {
            crate::integrations::herdr::report_session_obj(sess, start_source);
            crate::integrations::herdr::report_idle_force();
        } else {
            crate::integrations::herdr::report_idle_force();
        }
    }

    // helper to persist (cur_model passed explicitly to avoid borrow across mutation)
    let persist =
        |msgs: &Vec<Msg>, sess: &mut Option<crate::services::session::Session>, cur_model: &str| {
            if let Some(s) = sess {
                s.model = cur_model.to_string();
                s.cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                s.messages = msgs
                    .iter()
                    .map(|m| crate::services::session::SavedMsg {
                        role: m.role.clone(),
                        content: m.content.clone(),
                    })
                    .collect();
                let _ = s.save();
                crate::services::session::Session::prune(50);
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
            "  ▸  type a message…  (/help • Enter send • Shift+Enter newline • Shift+Tab NORM/PLAN/ASK/AUTO)",
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
    let mut pending_approval: Option<crate::guards::approval::ApprovalRequest> = None;
    let mut pending_question: Option<crate::services::question::AskRequest> = None;
    let mut question_wizard: Option<crate::services::question::Wizard> = None;
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
    let mut show_subagents = false;
    let mut subagent_selected: usize = 0;
    let mut subagent_scroll: usize = 0;
    let mut subagent_detail: Option<usize> = None;
    let mut pending_sudo: Option<crate::guards::sudo::SudoRequest> = None;
    let mut sudo_input = String::new();
    let mut sudo_error: Option<String> = None;
    let mut sudo_attempt: usize = 1;
    let mut subagent_detail_scroll: u16 = 0;
    let mut subagent_detail_auto_scroll = true;
    let mut subagent_detail_total: usize = 0;
    let mut subagent_detail_viewport_h: usize = 0;
    let mut subagent_list_auto_scroll = true;
    let mut subagent_kill_confirm = false;
    // ── Dictate (voice) state ──────────────────────────────────
    let mut dictate_state = crate::integrations::dictate::State::Idle;
    let mut dictate_chunks: Vec<Vec<u8>> = Vec::new();
    let mut dictate_child: Option<tokio::process::Child> = None;
    let mut dictate_meter: [f32; 6] = [0.0; 6];
    let mut dictate_current_level: f32 = 0.0;
    let mut dictate_generation: u64 = 0;
    let mut dictate_task: Option<tokio::task::JoinHandle<()>> = None;
    let (dictate_audio_tx, mut dictate_audio_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (dictate_result_tx, mut dictate_result_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let mut dictate_meter_last = std::time::Instant::now();
    // Eager MCP init (background)
    tokio::spawn(async move {
        crate::integrations::mcp::init().await;
    });

    // Redraw only when something changed; the spinner forces redraws while busy.
    let mut dirty = true;
    // CPU fix: cache rendered content so idle ticks (250ms) don't rebuild markdown 20×/sec
    let mut cached_wrapped_content: Vec<Line<'static>> = Vec::new();
    let mut cached_total_lines: usize = 0;
    let mut cached_content_height: usize = 0;
    let mut cached_chunks: Vec<Rect> = Vec::new();
    let mut cached_content_area: Rect = Rect {
        x: 0,
        y: 3,
        width: 80,
        height: 10,
    };
    let mut cached_term_size_width: u16 = 0;
    let mut cached_content_fingerprint: usize = 0; // sum of content lens + msg count
    let mut last_persist_fingerprint: usize = 0;

    loop {
        // Poll for approval requests — suppressed while auto-accept is ON
        if !crate::guards::approval::is_auto_accept() && pending_approval.is_none() {
            if let Some(req) = crate::guards::approval::take_pending() {
                pending_approval = Some(req);
                dirty = true;
            }
        }
        // Poll for question requests — never suppressed, even in auto-accept
        if pending_question.is_none() {
            if let Some(req) = crate::services::question::take_pending() {
                question_wizard = Some(crate::services::question::Wizard::new(
                    req.questions.clone(),
                ));
                pending_question = Some(req);
                dirty = true;
            }
        }
        // Poll for sudo password requests
        if pending_sudo.is_none() {
            if let Some(req) = crate::guards::sudo::take_pending() {
                sudo_input.clear();
                sudo_error = None;
                sudo_attempt = 1;
                pending_sudo = Some(req);
                dirty = true;
            }
        }
        // ── Herdr: publish derived agent state (blocked > working > idle) ──
        {
            let herdr_state = if pending_approval.is_some() {
                let label = pending_approval
                    .as_ref()
                    .map(|r| r.cmd.chars().take(60).collect::<String>())
                    .unwrap_or_else(|| "approval".to_string());
                Some(("blocked", Some(label)))
            } else if pending_sudo.is_some() {
                Some(("blocked", Some("sudo password".to_string())))
            } else if pending_question.is_some() {
                Some(("blocked", Some("ask_user".to_string())))
            } else if agent_busy {
                Some(("working", None))
            } else {
                Some(("idle", None))
            };
            if let Some((st, msg)) = herdr_state {
                match st {
                    "blocked" => crate::integrations::herdr::report_blocked(msg.as_deref()),
                    "working" => crate::integrations::herdr::report_working(),
                    _ => crate::integrations::herdr::report_idle(),
                }
            }
        }
        // Poll for subagent wake messages — render as tool call on master list (like pi)
        let wakes = crate::services::agents::take_wake_messages();
        if !wakes.is_empty() {
            for w in wakes {
                if w.id.is_empty() && w.agent.is_empty() {
                    // legacy string wake (fallback)
                    messages.push(Msg {
                        role: "system".into(),
                        content: w.result,
                        tool_id: None,
                        tool_name: None,
                        tool_args: None,
                        elapsed_ms: None,
                    });
                } else {
                    // tool-like subagent completion: looks identical to bash/read tool boxes
                    let header = if w.agent.is_empty() || w.agent == w.label {
                        w.label.clone()
                    } else {
                        format!("{}:{}", w.agent, w.label)
                    };
                    // result is "[subagent:label]\n{actual}" — strip prefix for cleaner body
                    let clean = if let Some(pos) = w.result.find('\n') {
                        let after = &w.result[pos + 1..];
                        // also strip leading "[subagent" line if present
                        if after.starts_with("[subagent") {
                            after
                                .find('\n')
                                .map(|p| after[p + 1..].trim_start().to_string())
                                .unwrap_or(after.to_string())
                        } else {
                            after.trim_start().to_string()
                        }
                    } else {
                        w.result.clone()
                    };
                    let body = if clean.trim().is_empty() {
                        "(no output)".to_string()
                    } else {
                        clean
                    };
                    // keep args JSON-ish like original tool call for preview
                    let args_json = if w.task.is_empty() {
                        format!(r#"{{"agent":"{}","task":"{}"}}"#, w.agent, w.label)
                    } else {
                        serde_json::json!({"agent": w.agent, "task": w.task}).to_string()
                    };
                    messages.push(Msg {
                        role: "tool".into(),
                        content: format!("subagent:{} → {}", header, body),
                        tool_id: Some(w.id.clone()),
                        tool_name: Some(format!("subagent:{}", header)),
                        tool_args: Some(args_json),
                        elapsed_ms: Some(w.elapsed_ms),
                    });
                }
            }
            dirty = true;
        }
        // ── Dictate: drain audio chunks + update meter ─────────
        while let Ok(chunk) = dictate_audio_rx.try_recv() {
            let lvl = crate::integrations::dictate::rms_from_pcm16(&chunk);
            dictate_current_level = lvl;
            dictate_chunks.push(chunk);
        }
        // meter tick every 60ms while recording
        if dictate_state == crate::integrations::dictate::State::Recording
            && dictate_meter_last.elapsed().as_millis() >= 60
        {
            dictate_meter_last = std::time::Instant::now();
            // shift left and push current level
            for i in 0..dictate_meter.len() - 1 {
                dictate_meter[i] = dictate_meter[i + 1];
            }
            dictate_meter[dictate_meter.len() - 1] = dictate_current_level;
            dirty = true;
        }
        // drain dictate results (transcription done)
        while let Ok(res) = dictate_result_rx.try_recv() {
            let gen_at_result = dictate_generation;
            match res {
                Ok(text) => {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        // Focus-aware delivery: if question wizard on Other -> push into wizard, else textarea, else clipboard
                        if let Some(wizard) = question_wizard.as_mut() {
                            if wizard.on_other() {
                                for c in trimmed.chars() {
                                    wizard.push_char(c);
                                }
                                // also keep a space separator if needed
                            } else {
                                // wizard not on Other: try textarea as fallback, else notify
                                let cur = textarea.lines().join("\n");
                                let sep = if cur.is_empty()
                                    || cur.ends_with(' ')
                                    || cur.ends_with('\n')
                                {
                                    ""
                                } else {
                                    " "
                                };
                                textarea.insert_str(format!("{}{}", sep, trimmed));
                                if !wizard.on_other() {
                                    messages.push(Msg { role: "system".into(), content: "Dictation: wizard not on Other field — also inserted into main input (Tab into Other first for direct entry)".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None });
                                }
                            }
                        } else {
                            let cur = textarea.lines().join("\n");
                            let sep = if cur.is_empty() || cur.ends_with(' ') || cur.ends_with('\n')
                            {
                                ""
                            } else {
                                " "
                            };
                            textarea.insert_str(format!("{}{}", sep, trimmed));
                        }
                    }
                    // always reset to idle after delivery
                    dictate_state = crate::integrations::dictate::State::Idle;
                    dictate_chunks.clear();
                    dictate_current_level = 0.0;
                    dictate_meter = [0.0; 6];
                    dirty = true;
                }
                Err(e) => {
                    messages.push(Msg {
                        role: "system".into(),
                        content: format!("Dictation failed: {}", e),
                        tool_id: None,
                        tool_name: None,
                        tool_args: None,
                        elapsed_ms: None,
                    });
                    dictate_state = crate::integrations::dictate::State::Idle;
                    dictate_chunks.clear();
                    dictate_current_level = 0.0;
                    dictate_meter = [0.0; 6];
                    dirty = true;
                    // generation already incremented on cleanup logic above; no extra
                    let _ = gen_at_result;
                }
            }
        }
        // Check if ffmpeg child exited unexpectedly while recording
        if dictate_state == crate::integrations::dictate::State::Recording {
            if let Some(child) = dictate_child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            messages.push(Msg {
                                role: "system".into(),
                                content: format!(
                                    "Dictation: ffmpeg exited unexpectedly (status {:?})",
                                    status
                                ),
                                tool_id: None,
                                tool_name: None,
                                tool_args: None,
                                elapsed_ms: None,
                            });
                        }
                        // cleanup handles dictate reset; keep chunks for transcribe if any
                        if dictate_chunks.is_empty() {
                            dictate_state = crate::integrations::dictate::State::Idle;
                            dictate_child = None;
                            dictate_task = None;
                            dictate_generation += 1;
                        } else {
                            // treat as stop -> transcribe what we have
                            dictate_state = crate::integrations::dictate::State::Transcribing;
                            let wav = crate::integrations::dictate::build_wav(&dictate_chunks);
                            let tx = dictate_result_tx.clone();
                            let gen = dictate_generation;
                            tokio::spawn(async move {
                                let res =
                                    crate::integrations::dictate::transcribe_with_groq(wav).await;
                                let _ = tx.send(res);
                                let _ = gen;
                            });
                            dictate_child = None;
                            dictate_task = None;
                        }
                        dirty = true;
                    }
                    Ok(None) => {}
                    Err(_) => {}
                }
            }
        }
        let term_size = terminal.size()?;
        let queue_rows = msg_queue.len().min(2) as u16;
        // Dynamic input height 1..6 (auto-grow like pi/jcode, clamped)
        let input_height = (textarea.lines().len() as u16).clamp(1, 6);
        let overhead = 7 + queue_rows + input_height; // indicator(1) + header + sep + sep + queue + summary(1) + input + footer(2 rows)
                                                      // Content area: starts after indicator + header + sep
        let content_area = Rect {
            x: 0,
            y: 3,
            width: term_size.width,
            height: term_size.height.saturating_sub(overhead).max(1),
        };

        // Pre-compute layout so we know content area dimensions — indicator always reserved (blank when idle, "━" + pulse when busy)
        let chunks = {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // running indicator (top, touches borders, always reserved)
                    Constraint::Length(1), // header
                    Constraint::Length(1), // separator
                    Constraint::Min(1),    // content
                    Constraint::Length(1), // separator
                    Constraint::Length(queue_rows), // queue (0-2)
                    Constraint::Length(1), // subagent summary bar (always reserved, empty when none)
                    Constraint::Length(input_height), // input (auto-grow 1..5)
                    Constraint::Length(2), // footer (2 rows: main + context)
                ])
                .split(Rect {
                    x: 0,
                    y: 0,
                    width: term_size.width,
                    height: term_size.height,
                });
            chunks
        };
        let content_height = chunks[3].height as usize;

        // Only rebuild wrapped content when dirty and fingerprint/size changed; reuse cached otherwise
        let fingerprint: usize = messages
            .len()
            .wrapping_add(messages.iter().map(|m| m.content.len()).sum::<usize>());
        let needs_rebuild = dirty
            && (cached_wrapped_content.is_empty()
                || fingerprint != cached_content_fingerprint
                || cached_term_size_width != term_size.width
                || cached_content_height != content_height
                || cached_chunks.len() != chunks.len());
        let (wrapped_content, total_lines) = if needs_rebuild {
            let content_lines = build_content_lines(&messages);
            let wrapped = wrap_lines(content_lines, chunks[3].width as usize);
            let total = wrapped.len();
            cached_wrapped_content = wrapped.clone();
            cached_total_lines = total;
            cached_content_height = content_height;
            cached_term_size_width = term_size.width;
            cached_chunks = chunks.to_vec();
            cached_content_area = content_area;
            cached_content_fingerprint = fingerprint;
            (wrapped, total)
        } else {
            (cached_wrapped_content.clone(), cached_total_lines)
        };
        if auto_scroll {
            scroll = total_lines.saturating_sub(content_height) as u16;
        }
        if subagent_detail.is_some() {
            if let Some(idx) = subagent_detail {
                let subs = crate::services::agents::list_subagents();
                if let Some(sub) = subs.get(idx) {
                    let inner_w = (term_size.width as usize).saturating_sub(4).max(10);
                    let inner_h = (term_size.height as usize).saturating_sub(6).max(5);
                    // sticky header: viewport is content area only (inner - header)
                    let header_raw: Vec<Line> = {
                        let display_label = if sub.label.is_empty() {
                            sub.agent.clone()
                        } else {
                            sub.label.clone()
                        };
                        let elapsed = std::time::SystemTime::now()
                            .duration_since(sub.started_at)
                            .unwrap_or_default()
                            .as_secs();
                        let elapsed_str = if elapsed < 60 {
                            format!("{}s", elapsed)
                        } else {
                            format!("{}m{}s", elapsed / 60, elapsed % 60)
                        };
                        let mut h = Vec::new();
                        h.push(Line::from(vec![
                            Span::styled(display_label, Style::default()),
                            Span::styled(sub.agent.clone(), Style::default()),
                        ]));
                        for l in hard_wrap_str(&sub.task, inner_w.saturating_sub(6)).lines() {
                            h.push(Line::from(l.to_string()));
                        }
                        h.push(Line::from(format!(
                            "Started: {:?}  Elapsed: {}",
                            sub.started_at, elapsed_str
                        )));
                        h.push(Line::from("─".repeat(inner_w)));
                        h
                    };
                    let header_h = wrap_lines(header_raw, inner_w).len().min(inner_h);
                    let content_h = inner_h.saturating_sub(header_h);
                    let total = detail_total_for_width(sub, inner_w);
                    // detail_total_for_width now returns transcript-only length (see function), so compare to content_h
                    subagent_detail_total = total;
                    subagent_detail_viewport_h = content_h;
                    if subagent_detail_auto_scroll {
                        subagent_detail_scroll = total.saturating_sub(inner_h) as u16;
                    } else {
                        // clamp if content shrank
                        let max = total.saturating_sub(inner_h) as u16;
                        if subagent_detail_scroll > max {
                            subagent_detail_scroll = max;
                        }
                    }
                }
            }
        } else if show_subagents {
            if subagent_list_auto_scroll {
                let total = crate::services::agents::list_subagents().len();
                let popup_h = (term_size.height as usize * 60 / 100).max(6);
                let inner_h = popup_h.saturating_sub(2);
                let vis = (inner_h / 2).max(1);
                let max = total.saturating_sub(vis);
                subagent_scroll = max;
            }
        }

        // Single render pass with correct scroll (only when state changed)
        if dirty {
            terminal.draw(|f| {
                let bg_block = Block::default().style(Style::default().bg(THEME.page_bg));
                f.render_widget(bg_block, f.area());

                // Running indicator — top row, touches borders, always reserved (blank when idle, "━" pulse when busy)
                draw_running_indicator(f, chunks[0], agent_busy, spinner_tick);

                // Header
                draw_header(f, chunks[1], &model, &step_info);

                // Separator
                draw_separator(f, chunks[2]);

                // Content — use pre-wrapped lines
                let para = Paragraph::new(wrapped_content.clone()).scroll((scroll, 0));
                f.render_widget(para, chunks[3]);
                // Scrollbar overlay — thin, read-only, only while not at bottom (A + #2), thumb encodes % (no footer text)
                if cached_total_lines > cached_content_height && !auto_scroll {
                    draw_scrollbar(
                        f,
                        chunks[3],
                        cached_total_lines,
                        cached_content_height,
                        scroll,
                    );
                }

                // Separator
                draw_separator(f, chunks[4]);

                // Queue
                draw_queue(f, chunks[5], &msg_queue);

                // Subagent summary bar (above input, visible when subagents exist)
                draw_subagent_summary(f, chunks[6], spinner_tick);

                // Input (textarea renders with cursor)
                draw_input(f, chunks[7], &mut textarea);

                // Autocomplete popup
                if !ac_matches.is_empty() {
                    draw_autocomplete(f, chunks[7], &ac_matches, ac_idx, ac_scroll);
                }
                if show_sessions {
                    draw_sessions(
                        f,
                        f.area(),
                        sessions_selected,
                        sessions_scroll,
                        &sessions_filter,
                        sessions_show_all,
                        &cwd,
                    );
                }
                if show_allowlist {
                    draw_allowlist(f, f.area(), allowlist_selected, allowlist_scroll);
                }
                if show_mcp {
                    draw_mcp(f, f.area(), mcp_selected, mcp_scroll);
                }
                if show_subagents {
                    if let Some(idx) = subagent_detail {
                        draw_subagent_detail(f, f.area(), idx, subagent_detail_scroll);
                        if subagent_kill_confirm {
                            let area = centered_rect(50, 20, f.area());
                            f.render_widget(Clear, area);
                            let block = Block::default()
                                .borders(Borders::ALL)
                                .title(" Confirm Kill ")
                                .border_style(Style::default().fg(ASHEN.ember));
                            let para = Paragraph::new(vec![
                                Line::from(Span::styled(
                                    "Kill this subagent? (y/n)",
                                    Style::default().fg(ASHEN.bone).add_modifier(Modifier::BOLD),
                                )),
                                Line::from(Span::styled(
                                    "This will mark it as killed (no process kill yet).",
                                    Style::default().fg(ASHEN.charcoal),
                                )),
                            ])
                            .block(block)
                            .style(Style::default().bg(THEME.page_bg));
                            f.render_widget(para, area);
                        }
                    } else {
                        draw_subagent_list(f, f.area(), subagent_selected, subagent_scroll);
                    }
                }
                // Approval / question / sudo on top of all overlays (visible even inside subagent view)
                if !crate::guards::approval::is_auto_accept() {
                    if let Some(ref req) = pending_approval {
                        draw_approval(f, f.area(), req);
                    }
                }
                if let Some(ref req) = pending_question {
                    if let Some(ref wizard) = question_wizard {
                        draw_question(f, f.area(), req, wizard);
                    }
                }
                if let Some(ref req) = pending_sudo {
                    draw_sudo(
                        f,
                        f.area(),
                        &req.cmd,
                        &sudo_input,
                        sudo_error.as_deref(),
                        sudo_attempt,
                    );
                }

                // Footer (2 rows) — includes dictate meter/spinner (shifts bar as requested)
                draw_footer(
                    f,
                    chunks[8],
                    &model,
                    &messages,
                    &cwd,
                    agent_busy,
                    spinner_tick,
                    dictate_state,
                    &dictate_meter,
                );
            })?;
            dirty = false;
        }

        // Handle keyboard and mouse events
        // Poll faster while dictate is active (60ms for meter, 80ms for transcribing spinner), else 10fps when busy, 4fps idle
        let poll_ms = if dictate_state == crate::integrations::dictate::State::Recording {
            60
        } else if dictate_state == crate::integrations::dictate::State::Transcribing {
            80
        } else if agent_busy {
            100
        } else {
            250
        };
        if event::poll(std::time::Duration::from_millis(poll_ms))? {
            let term_event = event::read()?;
            dirty = true;
            match term_event {
                Event::Paste(data) => {
                    crate::support::telemetry::record("paste");
                    let max_cols = (chunks[7].width as usize).saturating_sub(1);
                    textarea.insert_str(hard_wrap_str(&data, max_cols));
                    wrap_cursor_line(&mut textarea, max_cols);
                    ac_matches = current_completions(&textarea);
                    ac_idx = 0;
                }
                Event::Mouse(m) => {
                    // When subagent overlay is visible, wheel scrolls the overlay (not background chat) — exact regular behavior
                    if show_subagents {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                if let Some(_) = subagent_detail {
                                    subagent_detail_scroll =
                                        subagent_detail_scroll.saturating_sub(3);
                                    subagent_detail_auto_scroll = false;
                                } else {
                                    if subagent_scroll > 0 {
                                        subagent_scroll = subagent_scroll.saturating_sub(1);
                                    }
                                    subagent_list_auto_scroll = false;
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if let Some(_) = subagent_detail {
                                    let max = subagent_detail_total
                                        .saturating_sub(subagent_detail_viewport_h)
                                        as u16;
                                    subagent_detail_scroll = (subagent_detail_scroll + 3).min(max);
                                    if subagent_detail_scroll >= max {
                                        subagent_detail_auto_scroll = true;
                                    }
                                } else {
                                    let total = crate::services::agents::list_subagents().len();
                                    let popup_h = (term_size.height as usize * 60 / 100).max(6);
                                    let vis_rows =
                                        (popup_h.saturating_sub(2) / 2).max(1).min(total.max(1));
                                    let max = total.saturating_sub(vis_rows);
                                    subagent_scroll = (subagent_scroll + 1).min(max);
                                    if subagent_scroll >= max {
                                        subagent_list_auto_scroll = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
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
                    // Global dictate hotkeys: Alt+M toggle, Alt+N cancel — before any modal hijack so they work inside dialogs
                    // Filter Kitty release/repeat for dictate only — one physical press = one toggle (PI parity)
                    let is_alt_m =
                        k.code == KeyCode::Char('m') && k.modifiers.contains(KeyModifiers::ALT);
                    let is_alt_n =
                        k.code == KeyCode::Char('n') && k.modifiers.contains(KeyModifiers::ALT);
                    if (is_alt_m || is_alt_n) && k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if is_alt_m {
                        // Alt+M toggle
                        match dictate_state {
                            crate::integrations::dictate::State::Idle => {
                                // Start guard: GROQ key required
                                if std::env::var("GROQ_API_KEY")
                                    .unwrap_or_default()
                                    .trim()
                                    .is_empty()
                                {
                                    messages.push(Msg { role: "system".into(), content: "Dictation: GROQ_API_KEY not set in environment — add to .env or export".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None });
                                } else {
                                    match crate::integrations::dictate::spawn_ffmpeg() {
                                        Ok(mut child) => {
                                            dictate_generation = dictate_generation.wrapping_add(1);
                                            let gen = dictate_generation;
                                            dictate_chunks.clear();
                                            dictate_meter = [0.0; 6];
                                            dictate_current_level = 0.0;
                                            dictate_meter_last = std::time::Instant::now();
                                            dictate_state =
                                                crate::integrations::dictate::State::Recording;
                                            // Spawn reader task for stdout
                                            if let Some(stdout) = child.stdout.take() {
                                                dictate_child = Some(child);
                                                let tx = dictate_audio_tx.clone();
                                                let err_tx = dictate_audio_tx.clone(); // not used but keep gen
                                                let _ = err_tx; // silence unused
                                                let _ = gen;
                                                dictate_task = Some(tokio::spawn(async move {
                                                    use tokio::io::AsyncReadExt;
                                                    let mut reader = stdout;
                                                    let mut buf = [0u8; 4096];
                                                    loop {
                                                        match reader.read(&mut buf).await {
                                                            Ok(0) => break,
                                                            Ok(n) => {
                                                                let chunk = buf[..n].to_vec();
                                                                if tx.send(chunk).is_err() {
                                                                    break;
                                                                }
                                                            }
                                                            Err(_) => break,
                                                        }
                                                    }
                                                }));
                                            } else {
                                                dictate_child = Some(child);
                                            }
                                        }
                                        Err(e) => {
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("Dictation: {}", e),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        }
                                    }
                                }
                            }
                            crate::integrations::dictate::State::Recording => {
                                // Stop -> transcribe
                                dictate_generation = dictate_generation.wrapping_add(1);
                                dictate_state = crate::integrations::dictate::State::Transcribing;
                                // Kill ffmpeg
                                if let Some(mut child) = dictate_child.take() {
                                    let _ = child.kill().await;
                                    let _ = child.wait().await;
                                }
                                if let Some(t) = dictate_task.take() {
                                    t.abort();
                                }
                                // Drain any pending audio still in channel before building wav
                                while let Ok(chunk) = dictate_audio_rx.try_recv() {
                                    dictate_chunks.push(chunk);
                                }
                                if dictate_chunks.is_empty() {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "Dictation: no audio captured".into(),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                    dictate_state = crate::integrations::dictate::State::Idle;
                                } else {
                                    let wav =
                                        crate::integrations::dictate::build_wav(&dictate_chunks);
                                    let tx = dictate_result_tx.clone();
                                    tokio::spawn(async move {
                                        let res =
                                            crate::integrations::dictate::transcribe_with_groq(wav)
                                                .await;
                                        let _ = tx.send(res);
                                    });
                                }
                            }
                            crate::integrations::dictate::State::Transcribing => {
                                // Ignore Alt+M while transcribing
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    if is_alt_n {
                        // Alt+N cancel
                        if dictate_state != crate::integrations::dictate::State::Idle {
                            dictate_generation = dictate_generation.wrapping_add(1);
                            if let Some(mut child) = dictate_child.take() {
                                let _ = child.kill().await;
                            }
                            if let Some(t) = dictate_task.take() {
                                t.abort();
                            }
                            dictate_chunks.clear();
                            dictate_meter = [0.0; 6];
                            dictate_current_level = 0.0;
                            dictate_state = crate::integrations::dictate::State::Idle;
                            // drain channels
                            while dictate_audio_rx.try_recv().is_ok() {}
                            while dictate_result_rx.try_recv().is_ok() {}
                            messages.push(Msg {
                                role: "system".into(),
                                content: "Dictation cancelled".into(),
                                tool_id: None,
                                tool_name: None,
                                tool_args: None,
                                elapsed_ms: None,
                            });
                            dirty = true;
                        }
                        continue;
                    }
                    // Global mode cycle: Shift+Tab circles NORM → PLAN → ASK → AUTO → NORM — must be before any modal hijack, wins over wizard BackTab
                    let is_shift_tab = k.code == KeyCode::BackTab
                        || (k.code == KeyCode::Tab && k.modifiers.contains(KeyModifiers::SHIFT));
                    if is_shift_tab {
                        let next = crate::agent::cycle_mode();
                        match next {
                            crate::agent::Mode::Auto => {
                                // AUTO takes effect immediately: drain pending approvals
                                if let Some(mut req) = pending_approval.take() {
                                    if let Some(tx) = req.tx.take() {
                                        let _ = tx.send(true);
                                    }
                                }
                                while let Some(mut req) = crate::guards::approval::take_pending() {
                                    if let Some(tx) = req.tx.take() {
                                        let _ = tx.send(true);
                                    }
                                }
                                crate::support::telemetry::record("mode_auto");
                            }
                            crate::agent::Mode::Plan => {
                                crate::support::telemetry::record("mode_plan");
                            }
                            crate::agent::Mode::Ask => {
                                crate::support::telemetry::record("mode_ask");
                            }
                            crate::agent::Mode::Norm => {
                                crate::support::telemetry::record("mode_norm");
                            }
                        }
                        continue;
                    }
                    // Question/Approval modal: hijack all keys while the agent waits — top priority even inside subagent/mcp overlays
                    if let (Some(mut req), Some(mut wizard)) =
                        (pending_question.take(), question_wizard.take())
                    {
                        let q_ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        let q_alt = k.modifiers.contains(KeyModifiers::ALT);
                        let outcome = match k.code {
                            KeyCode::Up => {
                                wizard.move_up();
                                crate::services::question::WizardOutcome::Continue
                            }
                            KeyCode::Down => {
                                wizard.move_down();
                                crate::services::question::WizardOutcome::Continue
                            }
                            KeyCode::BackTab | KeyCode::Left => wizard.back(),
                            KeyCode::Backspace => {
                                wizard.backspace();
                                crate::services::question::WizardOutcome::Continue
                            }
                            KeyCode::Char('c') if q_ctrl => wizard.cancel(),
                            KeyCode::Char(c) if !q_ctrl && !q_alt => {
                                if wizard.on_other() {
                                    wizard.push_char(c);
                                } else if c == ' ' {
                                    wizard.toggle();
                                }
                                crate::services::question::WizardOutcome::Continue
                            }
                            KeyCode::Enter => wizard.confirm(),
                            KeyCode::Esc => wizard.cancel(),
                            _ => crate::services::question::WizardOutcome::Continue,
                        };
                        match outcome {
                            crate::services::question::WizardOutcome::Continue => {
                                pending_question = Some(req);
                                question_wizard = Some(wizard);
                            }
                            crate::services::question::WizardOutcome::Submit => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(wizard.answers());
                                }
                                crate::support::telemetry::record("ask_user_answered");
                            }
                            crate::services::question::WizardOutcome::Cancel => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(Vec::new());
                                }
                                crate::support::telemetry::record("ask_user_skipped");
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    // Guard approval modal: hijack all keys (covers bash + dir + mcp guard)
                    if pending_approval.is_some() {
                        let mut req = pending_approval.take().unwrap();
                        let is_dir = req.reasons.iter().any(|r| r.contains("outside CWD"));
                        let is_mcp = req.reasons.iter().any(|r| r.contains("MCP tool"));
                        match k.code {
                            KeyCode::Char('a') if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(true);
                                }
                                crate::support::telemetry::record(if is_mcp {
                                    "mcp_guard_allow_once"
                                } else if is_dir {
                                    "dir_guard_allow_once"
                                } else {
                                    "bash_guard_allow_once"
                                });
                            }
                            KeyCode::Char('A') => {
                                if is_mcp {
                                    // MCP: every call needs approval, 'A' is treated as allow once (no persist)
                                    if let Some(tx) = req.tx.take() {
                                        let _ = tx.send(true);
                                    }
                                    crate::support::telemetry::record("mcp_guard_allow_once");
                                } else if is_dir {
                                    // Extract offending paths from reasons "outside CWD (path → resolved)"
                                    for r in &req.reasons {
                                        if let Some(s) = r.find('(') {
                                            if let Some(e) = r.find(" →") {
                                                let raw = r[s + 1..e].trim();
                                                if !raw.is_empty() {
                                                    crate::guards::dir::allowlist_add(raw);
                                                }
                                            }
                                        }
                                    }
                                    // Also allowlist the raw cmd/path as fallback
                                    // For file tools cmd is "read_file /path", extract last token
                                    let fallback = if req.cmd.contains(' ') {
                                        req.cmd
                                            .split_whitespace()
                                            .last()
                                            .unwrap_or(&req.cmd)
                                            .to_string()
                                    } else {
                                        req.cmd.clone()
                                    };
                                    if !fallback.is_empty() {
                                        crate::guards::dir::allowlist_add(&fallback);
                                    }
                                    // For bash, also allowlist the full command for exact match
                                    if req.cmd.contains('/') || req.cmd.contains(' ') {
                                        crate::guards::dir::allowlist_add(&req.cmd);
                                    }
                                } else {
                                    crate::guards::bash::allowlist_add(&req.cmd);
                                }
                                if !is_mcp {
                                    if let Some(tx) = req.tx.take() {
                                        let _ = tx.send(true);
                                    }
                                    crate::support::telemetry::record(if is_dir {
                                        "dir_guard_allow_always"
                                    } else {
                                        "bash_guard_allow_always"
                                    });
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(false);
                                }
                                crate::support::telemetry::record(if is_mcp {
                                    "mcp_guard_deny"
                                } else {
                                    "bash_guard_deny"
                                });
                            }
                            KeyCode::Enter => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(true);
                                }
                                crate::support::telemetry::record(if is_mcp {
                                    "mcp_guard_allow_once"
                                } else if is_dir {
                                    "dir_guard_allow_once"
                                } else {
                                    "bash_guard_allow_once"
                                });
                            }
                            KeyCode::Esc => {
                                if let Some(tx) = req.tx.take() {
                                    let _ = tx.send(false);
                                }
                                crate::support::telemetry::record(if is_mcp {
                                    "mcp_guard_deny"
                                } else {
                                    "bash_guard_deny"
                                });
                            }
                            _ => {
                                pending_approval = Some(req);
                            }
                        }
                        continue;
                    }
                    // Sudo password modal: hijack keys while waiting for password
                    if pending_sudo.is_some() {
                        let is_ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        let is_alt = k.modifiers.contains(KeyModifiers::ALT);
                        match k.code {
                            KeyCode::Esc => {
                                if let Some(mut req) = pending_sudo.take() {
                                    if let Some(tx) = req.tx.take() {
                                        let _ = tx.send(None);
                                    }
                                }
                                sudo_input.clear();
                                sudo_error = None;
                                sudo_attempt = 1;
                            }
                            KeyCode::Enter => {
                                if sudo_input.is_empty() {
                                    sudo_error = Some("password cannot be empty".to_string());
                                    let req = pending_sudo.take().unwrap();
                                    pending_sudo = Some(req);
                                } else {
                                    if let Some(mut req) = pending_sudo.take() {
                                        if let Some(tx) = req.tx.take() {
                                            let _ = tx.send(Some(sudo_input.clone()));
                                        }
                                    }
                                    sudo_input.clear();
                                    sudo_error = None;
                                }
                            }
                            KeyCode::Backspace => {
                                sudo_input.pop();
                                sudo_error = None;
                            }
                            KeyCode::Char(c) if !is_ctrl && !is_alt => {
                                sudo_input.push(c);
                                sudo_error = None;
                            }
                            _ => {
                                let req = pending_sudo.take().unwrap();
                                pending_sudo = Some(req);
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    // Subagent overlay: hijack keys (highest priority after Shift+Tab)
                    if show_subagents {
                        if subagent_kill_confirm {
                            match k.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    if let Some(idx) = subagent_detail {
                                        let subs = crate::services::agents::list_subagents();
                                        if let Some(sub) = subs.get(idx) {
                                            crate::services::agents::kill_subagent(&sub.id);
                                        }
                                    }
                                    subagent_kill_confirm = false;
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    subagent_kill_confirm = false;
                                }
                                _ => {}
                            }
                            continue;
                        }
                        if let Some(idx) = subagent_detail {
                            let max = subagent_detail_total
                                .saturating_sub(subagent_detail_viewport_h)
                                as u16;
                            match k.code {
                                KeyCode::Esc => {
                                    // back to list, not close entirely
                                    subagent_detail = None;
                                    subagent_detail_scroll = 0;
                                    subagent_detail_auto_scroll = true;
                                }
                                KeyCode::Char('K') if k.modifiers.contains(KeyModifiers::SHIFT) => {
                                    subagent_kill_confirm = true;
                                }
                                KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::SHIFT) => {
                                    subagent_kill_confirm = true;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    subagent_detail_scroll =
                                        subagent_detail_scroll.saturating_sub(1);
                                    subagent_detail_auto_scroll = false;
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    subagent_detail_scroll = (subagent_detail_scroll + 1).min(max);
                                    if subagent_detail_scroll >= max {
                                        subagent_detail_auto_scroll = true;
                                    } else {
                                        subagent_detail_auto_scroll = false;
                                    }
                                }
                                KeyCode::PageUp => {
                                    subagent_detail_scroll =
                                        subagent_detail_scroll.saturating_sub(10);
                                    subagent_detail_auto_scroll = false;
                                }
                                KeyCode::PageDown => {
                                    subagent_detail_scroll = (subagent_detail_scroll + 10).min(max);
                                    if subagent_detail_scroll >= max {
                                        subagent_detail_auto_scroll = true;
                                    } else {
                                        subagent_detail_auto_scroll = false;
                                    }
                                }
                                KeyCode::Home => {
                                    subagent_detail_scroll = 0;
                                    subagent_detail_auto_scroll = false;
                                }
                                KeyCode::End => {
                                    subagent_detail_scroll = max;
                                    subagent_detail_auto_scroll = true;
                                }
                                _ => {}
                            }
                            continue;
                        }
                        // List mode — exact regular scroll (clamped, Home/End, Page, wheel)
                        match k.code {
                            KeyCode::Esc => {
                                show_subagents = false;
                                subagent_detail = None;
                                subagent_list_auto_scroll = true;
                            }
                            KeyCode::Up => {
                                if subagent_selected > 0 {
                                    subagent_selected -= 1;
                                    if subagent_selected < subagent_scroll {
                                        subagent_scroll = subagent_selected;
                                    }
                                }
                                subagent_list_auto_scroll = false;
                            }
                            KeyCode::Down => {
                                let popup_h = (term_size.height as usize * 60 / 100).max(6);
                                let vis = (popup_h.saturating_sub(2) / 2).max(1);
                                let len = crate::services::agents::list_subagents().len();
                                if subagent_selected + 1 < len {
                                    subagent_selected += 1;
                                    if subagent_selected >= subagent_scroll + vis {
                                        subagent_scroll += 1;
                                    }
                                }
                                let total = len;
                                let max = total.saturating_sub(vis);
                                if subagent_scroll >= max {
                                    subagent_list_auto_scroll = true;
                                } else {
                                    subagent_list_auto_scroll = false;
                                }
                            }
                            KeyCode::PageUp => {
                                let popup_h = (term_size.height as usize * 60 / 100).max(6);
                                let vis = (popup_h.saturating_sub(2) / 2).max(1);
                                let step = vis;
                                subagent_selected = subagent_selected.saturating_sub(step);
                                subagent_scroll = subagent_scroll.saturating_sub(step);
                                subagent_list_auto_scroll = false;
                            }
                            KeyCode::PageDown => {
                                let popup_h = (term_size.height as usize * 60 / 100).max(6);
                                let vis = (popup_h.saturating_sub(2) / 2).max(1);
                                let step = vis;
                                let len = crate::services::agents::list_subagents().len();
                                subagent_selected =
                                    (subagent_selected + step).min(len.saturating_sub(1));
                                let total = len;
                                let max = total.saturating_sub(vis);
                                subagent_scroll = (subagent_scroll + step).min(max);
                                if subagent_scroll >= max {
                                    subagent_list_auto_scroll = true;
                                }
                            }
                            KeyCode::Home => {
                                subagent_selected = 0;
                                subagent_scroll = 0;
                                subagent_list_auto_scroll = false;
                            }
                            KeyCode::End => {
                                let len = crate::services::agents::list_subagents().len();
                                if len > 0 {
                                    subagent_selected = len - 1;
                                }
                                let total = len;
                                let popup_h = (term_size.height as usize * 60 / 100).max(6);
                                let vis = (popup_h.saturating_sub(2) / 2).max(1);
                                let max = total.saturating_sub(vis);
                                subagent_scroll = max;
                                subagent_list_auto_scroll = true;
                            }
                            KeyCode::Enter => {
                                subagent_detail = Some(subagent_selected);
                                subagent_detail_scroll = 0;
                                subagent_detail_auto_scroll = true;
                            }
                            _ => {}
                        }
                        continue;
                    }
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
                                    if allowlist_selected < allowlist_scroll {
                                        allowlist_scroll = allowlist_selected;
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let bash_len = crate::guards::bash::allowlist_list().len();
                                let dir_len = crate::guards::dir::allowlist_list().len();
                                let len = bash_len + dir_len;
                                if allowlist_selected + 1 < len {
                                    allowlist_selected += 1;
                                    if allowlist_selected >= allowlist_scroll + 10 {
                                        allowlist_scroll += 1;
                                    }
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Delete => {
                                // rebuild combined as in draw
                                let bash_list = crate::guards::bash::allowlist_list();
                                let dir_list = crate::guards::dir::allowlist_list();
                                let mut combined: Vec<(String, String)> = Vec::new();
                                for p in &bash_list {
                                    combined.push(("bash".into(), p.clone()));
                                }
                                for p in &dir_list {
                                    combined.push(("dir".into(), p.clone()));
                                }
                                combined.sort_by(|a, b| a.1.cmp(&b.1));
                                if let Some((kind, pat)) = combined.get(allowlist_selected).cloned()
                                {
                                    if kind == "dir" {
                                        crate::guards::dir::allowlist_remove(&pat);
                                    } else {
                                        crate::guards::bash::allowlist_remove(&pat);
                                    }
                                    let new_len = crate::guards::bash::allowlist_list().len()
                                        + crate::guards::dir::allowlist_list().len();
                                    if allowlist_selected >= new_len && allowlist_selected > 0 {
                                        allowlist_selected -= 1;
                                    }
                                }
                            }
                            KeyCode::Char('c') => {
                                crate::guards::bash::allowlist_clear();
                                crate::guards::dir::allowlist_clear();
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
                            KeyCode::Tab => {
                                sessions_show_all = !sessions_show_all;
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Enter => {
                                // Apply same ordering as draw: dir filter + text filter
                                let all = crate::services::session::Session::list();
                                let cwd_norm = cwd.trim_end_matches('/');
                                let dir_filtered: Vec<crate::services::session::Session> =
                                    if sessions_show_all {
                                        all
                                    } else {
                                        all.into_iter()
                                            .filter(|s| s.cwd.trim_end_matches('/') == cwd_norm)
                                            .collect()
                                    };
                                let filtered: Vec<crate::services::session::Session> =
                                    if sessions_filter.is_empty() {
                                        dir_filtered
                                    } else {
                                        let lower = sessions_filter.to_lowercase();
                                        dir_filtered
                                            .into_iter()
                                            .filter(|s| {
                                                s.id.to_lowercase().contains(&lower)
                                                    || s.cwd.to_lowercase().contains(&lower)
                                                    || s.messages.iter().any(|m| {
                                                        m.content.to_lowercase().contains(&lower)
                                                    })
                                            })
                                            .collect()
                                    };
                                if let Some(sess) = filtered.get(sessions_selected).cloned() {
                                    messages = sess
                                        .messages
                                        .iter()
                                        .map(|m| Msg {
                                            role: m.role.clone(),
                                            content: m.content.clone(),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        })
                                        .collect();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("[resumed session {}]", sess.id),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                    session = Some(sess);
                                    if let Some(ref s) = session {
                                        crate::integrations::herdr::report_session_obj(
                                            s,
                                            Some("resume"),
                                        );
                                    }
                                }
                                show_sessions = false;
                                sessions_filter.clear();
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Up => {
                                if sessions_selected > 0 {
                                    sessions_selected -= 1;
                                    if sessions_selected < sessions_scroll {
                                        sessions_scroll = sessions_selected;
                                    }
                                }
                            }
                            KeyCode::Down => {
                                // Compute filtered len with dir + text filters
                                let all = crate::services::session::Session::list();
                                let cwd_norm = cwd.trim_end_matches('/');
                                let dir_filtered: Vec<crate::services::session::Session> =
                                    if sessions_show_all {
                                        all
                                    } else {
                                        all.into_iter()
                                            .filter(|s| s.cwd.trim_end_matches('/') == cwd_norm)
                                            .collect()
                                    };
                                let filtered_len = if sessions_filter.is_empty() {
                                    dir_filtered.len()
                                } else {
                                    let lower = sessions_filter.to_lowercase();
                                    dir_filtered
                                        .iter()
                                        .filter(|s| {
                                            s.id.to_lowercase().contains(&lower)
                                                || s.cwd.to_lowercase().contains(&lower)
                                                || s.messages.iter().any(|m| {
                                                    m.content.to_lowercase().contains(&lower)
                                                })
                                        })
                                        .count()
                                };
                                if sessions_selected + 1 < filtered_len {
                                    sessions_selected += 1;
                                    let visible = 10;
                                    if sessions_selected >= sessions_scroll + visible {
                                        sessions_scroll += 1;
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                sessions_filter.pop();
                                sessions_selected = 0;
                                sessions_scroll = 0;
                            }
                            KeyCode::Char(c)
                                if !k.modifiers.contains(KeyModifiers::CONTROL)
                                    && !k.modifiers.contains(KeyModifiers::ALT) =>
                            {
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
                                let servers = crate::integrations::mcp::snapshot();
                                if let Some(srv) = servers.get(mcp_selected).cloned() {
                                    if crate::integrations::mcp::is_server_disabled(&srv.name)
                                        || matches!(
                                            srv.status,
                                            crate::integrations::mcp::ServerStatus::Disabled
                                        )
                                    {
                                        // stay open, show [disabled] instantly — no chat spam
                                    } else {
                                        let name = srv.name.clone();
                                        tokio::spawn(async move {
                                            let _ =
                                                crate::integrations::mcp::reconnect(&name).await;
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Char('t') | KeyCode::Char(' ') => {
                                let servers = crate::integrations::mcp::snapshot();
                                if let Some(srv) = servers.get(mcp_selected).cloned() {
                                    let now_disabled =
                                        crate::integrations::mcp::toggle_server_disabled(&srv.name);
                                    if !now_disabled {
                                        let name = srv.name.clone();
                                        tokio::spawn(async move {
                                            let _ =
                                                crate::integrations::mcp::reconnect(&name).await;
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('g') => {
                                let was = crate::integrations::mcp::is_global_disabled();
                                crate::integrations::mcp::set_global_disabled(!was);
                                if was {
                                    tokio::spawn(async move {
                                        crate::integrations::mcp::init().await;
                                    });
                                }
                            }
                            KeyCode::Up => {
                                if mcp_selected > 0 {
                                    mcp_selected -= 1;
                                    if mcp_selected < mcp_scroll {
                                        mcp_scroll = mcp_selected;
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let len = crate::integrations::mcp::snapshot().len();
                                if mcp_selected + 1 < len {
                                    mcp_selected += 1;
                                    if mcp_selected >= mcp_scroll + 8 {
                                        mcp_scroll += 1;
                                    }
                                }
                            }
                            _ => {}
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
                                    crate::support::telemetry::record("interrupt");
                                    handle.abort();
                                }
                                if !msg_queue.is_empty() {
                                    // don't flicker busy off -> braille keeps ticking
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "[interrupted → next queued]".into(),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                    let next = msg_queue.remove(0);
                                    let expanded_next = expand_at_mentions(&next);
                                    let hist = llm_history_for_spawn(&session, &messages);
                                    messages.push(Msg {
                                        role: "user".into(),
                                        content: next.clone(),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
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
                                        content: "[interrupted]".into(),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
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
                        KeyCode::Char('o') if ctrl => {
                            if !crate::services::agents::list_subagents().is_empty() {
                                show_subagents = true;
                                subagent_selected = 0;
                                subagent_scroll = 0;
                                subagent_detail = None;
                            }
                        }
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
                            crate::support::telemetry::record("kill_line");
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('k') if ctrl => {
                            textarea.delete_line_by_end();
                            crate::support::telemetry::record("kill_line_end");
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('z') if ctrl => {
                            crate::support::telemetry::record("input_undo");
                            textarea.undo();
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Char('v') | KeyCode::Char('V')
                            if ctrl || k.modifiers.contains(KeyModifiers::SUPER) =>
                        {
                            // Ctrl+V / Cmd+V (macOS) / Ctrl+Shift+V: try image clipboard first, fallback to text paste
                            if let Some(marker) = clipboard_image_marker() {
                                if marker.starts_with("[image skipped") {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: marker.clone(),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                } else {
                                    // Store actual marker, insert visible placeholder [[IMAGE #N]]
                                    let idx = {
                                        let mut store = pasted_store().lock().unwrap();
                                        store.push(marker);
                                        store.len()
                                    };
                                    let placeholder = format!("[[IMAGE #{}]] ", idx);
                                    textarea.insert_str(&placeholder);
                                }
                                crate::support::telemetry::record("image_paste");
                                ac_matches = current_completions(&textarea);
                                ac_idx = 0;
                            } else {
                                let before = textarea.lines().join("\n");
                                textarea.paste();
                                let after = textarea.lines().join("\n");
                                if before == after {
                                    // No text pasted and no image found — likely image clipboard without tool support
                                    messages.push(Msg { role: "system".into(), content: "no image in clipboard (and no text). Tip: copy screenshot as PNG (Flameshot/Spectacle), ensure wl-clipboard installed, or use @path/to/image.png — see wl-paste --list-types to debug.".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                crate::support::telemetry::record("paste");
                                ac_matches = current_completions(&textarea);
                                ac_idx = 0;
                            }
                        }
                        KeyCode::Char('y') if ctrl => {
                            textarea.paste();
                            crate::support::telemetry::record("paste");
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
                            crate::support::telemetry::record("newline");
                            let inp = crossterm_key_to_input(k);
                            textarea.input(inp);
                            ac_matches = current_completions(&textarea);
                            ac_idx = 0;
                        }
                        KeyCode::Enter => {
                            // Enter: if autocomplete visible, accept it first
                            if !ac_matches.is_empty() {
                                let chosen = ac_matches[ac_idx].clone();
                                if let Some(m) = detect_agent_mention(&textarea) {
                                    // # completion — chosen is like "#scout — desc"
                                    let agent_name = chosen
                                        .split('—')
                                        .next()
                                        .unwrap_or(&chosen)
                                        .trim()
                                        .trim_start_matches("#")
                                        .trim()
                                        .to_string();
                                    let full = format!("#{}", agent_name);
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line2 = format!("{}{} {}", before, full, after);
                                        lines[m.row] = new_line2;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + full.len() + 1;
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(full);
                                    }
                                } else if let Some(m) = detect_skill_mention(&textarea) {
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
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
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
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
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
                                crate::support::telemetry::record("prompt_recall");
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
                                crate::support::telemetry::record("prompt_jump_down");
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
                                if let Some(m) = detect_agent_mention(&textarea) {
                                    // # completion — chosen is like "#scout — desc"
                                    let agent_name = chosen
                                        .split('—')
                                        .next()
                                        .unwrap_or(&chosen)
                                        .trim()
                                        .trim_start_matches("#")
                                        .trim()
                                        .to_string();
                                    let full = format!("#{}", agent_name);
                                    let mut lines = textarea.lines().to_vec();
                                    if m.row < lines.len() {
                                        let line = &lines[m.row];
                                        let chars: Vec<char> = line.chars().collect();
                                        let before: String = chars[..m.at_col].iter().collect();
                                        let after: String = chars[m.col..].iter().collect();
                                        let new_line2 = format!("{}{} {}", before, full, after);
                                        lines[m.row] = new_line2;
                                        let new_text = lines.join("\n");
                                        let new_col = m.at_col + full.len() + 1;
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(new_text);
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
                                    } else {
                                        textarea.select_all();
                                        textarea.cut();
                                        textarea.insert_str(full);
                                    }
                                } else if let Some(m) = detect_skill_mention(&textarea) {
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
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
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
                                        textarea.move_cursor(CursorMove::Jump(
                                            m.row as u16,
                                            new_col as u16,
                                        ));
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
                            crate::support::telemetry::record("word_back");
                        }
                        KeyCode::Right if alt => {
                            textarea.move_cursor(CursorMove::WordForward);
                            crate::support::telemetry::record("word_forward");
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
                        let max_cols = (chunks[7].width as usize).saturating_sub(1);
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
                        // --- Inline # hint: keep text + add system hint, let main agent delegate via subagent tool ---
                        let forced_agents = parse_all_forced_agents(&prompt_raw);
                        let mut inline_hint = String::new();
                        if !forced_agents.is_empty() {
                            let names: Vec<String> =
                                forced_agents.iter().map(|(n, _)| n.clone()).collect();
                            inline_hint = format!("\n\n[hint: user included #{} — please delegate relevant subtasks to those agents via `subagent` tool (agent field). You may run them in parallel and keep working; you will be woken when they return.]", names.join(", #"));
                        }
                        // Keep display as raw, but expand @files for LLM and append inline # hint
                        let prompt = prompt_raw.clone();
                        let mut expanded = expand_at_mentions(&prompt_raw);
                        if !inline_hint.is_empty() {
                            expanded.push_str(&inline_hint);
                        }
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
                                        session =
                                            Some(crate::services::session::Session::new(&model));
                                        if let Some(ref s) = session {
                                            crate::integrations::herdr::report_session_obj(
                                                s,
                                                Some("new"),
                                            );
                                        }
                                    } else {
                                        crate::integrations::herdr::report_idle_force();
                                    }
                                }
                                "/help" => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "/help /new /sessions /resume <id> /allowlist /allowlist clear /mcp /memory stats|consolidate /model <name> /clear /exit /auto-accept /plan /init  ·  Enter send · Shift+Enter newline · Shift+Tab NORM/PLAN/ASK/AUTO · @file $skill · Ctrl+C clear · Ctrl+U kill · Ctrl+Z undo · Up/Down history · PgUp/PgDn scroll — /plan toggles PLAN, /auto-accept toggles AUTO, Shift+Tab cycles — /init creates AGENTS.md (MEMORY.md is global-only, auto-updated silently in ~/.lean/)".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                }
                                "/plan" => {
                                    let next = if crate::agent::current_mode()
                                        == crate::agent::Mode::Plan
                                    {
                                        crate::agent::Mode::Norm
                                    } else {
                                        crate::agent::Mode::Plan
                                    };
                                    crate::agent::set_mode(next);
                                    match next {
                                        crate::agent::Mode::Plan => {
                                            crate::support::telemetry::record("mode_plan")
                                        }
                                        _ => crate::support::telemetry::record("mode_norm"),
                                    }
                                }
                                "/init" => {
                                    // Fast deterministic fallback + LLM enrichment
                                    let created = crate::core::context::ensure_init_files();
                                    if !created.is_empty() {
                                        messages.push(Msg { role: "system".into(), content: format!("init: created {} — now enriching with project analysis…", created.join(", ")), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    } else {
                                        messages.push(Msg { role: "system".into(), content: "init: AGENTS.md already exists — refreshing via agent…".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    }
                                    let cwd =
                                        crate::guards::dir::project_root().display().to_string();
                                    let init_prompt = format!(
                                        r#"Initialize project memory for lean. Current directory: {cwd}

Tasks:
1. Explore the codebase: run `ls -la`, read README.md, Cargo.toml / package.json / pyproject.toml / go.mod if present, and list src/ structure. Use read and bash (read-only) to gather facts.
2. Create or update AGENTS.md at ./AGENTS.md. Include: Project Overview (what it does), Tech Stack, Commands (build/run/test/lint exactly as found), Project Structure (key dirs), Conventions (style, commits), Architecture Notes, Gotchas. Keep concise, actionable, 1-2 pages. Use only facts you found — don't invent. Do NOT create CLAUDE.md or any MEMORY.md file in the project — AGENTS.md only.
3. MEMORY.md is global-only (~/.lean/MEMORY.md) and is updated silently in the background via the memory system — do NOT create ./MEMORY.md, do NOT use ask_user for persona, do NOT mention it in the project.
4. After writing, verify by reading ./AGENTS.md and summarize what was created/updated. End with "All done."

Be thorough but concise. Read before write; use unique oldText for edits."#
                                    );
                                    let hist = llm_history_for_spawn(&session, &messages);
                                    let expanded = expand_at_mentions(&init_prompt);
                                    history.push("/init".to_string());
                                    hist_idx = None;
                                    step_info = "...".into();
                                    auto_scroll = true;
                                    ac_matches.clear();
                                    ac_idx = 0;
                                    if agent_busy {
                                        msg_queue.push("/init".to_string());
                                    } else {
                                        messages.push(Msg {
                                            role: "user".into(),
                                            content: "/init (initialize AGENTS.md)".into(),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                        persist(&messages, &mut session, &model);
                                        agent_busy = true;
                                        agent_handle = Some(spawn_agent_with_history(
                                            expanded,
                                            model.clone(),
                                            hist,
                                            tx.clone(),
                                            done_tx.clone(),
                                        ));
                                    }
                                    crate::support::telemetry::record("init");
                                    // already handled textarea clear below; mark so we don't double-clear
                                    textarea.select_all();
                                    textarea.cut();
                                    hist_idx = None;
                                    ac_matches.clear();
                                    ac_idx = 0;
                                    continue;
                                }
                                _ if prompt.starts_with("/init") => {
                                    let rest = prompt.strip_prefix("/init").unwrap_or("").trim();
                                    if rest == "--help" || rest == "-h" || rest == "help" {
                                        messages.push(Msg { role: "system".into(), content: "usage: /init — analyze project and create/update AGENTS.md. Files are auto-loaded on startup (project AGENTS.md + global ~/.lean/MEMORY.md updated silently in background).".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    } else if !rest.is_empty() {
                                        // /init with extra text — treat as note for the agent
                                        let created = crate::core::context::ensure_init_files();
                                        if !created.is_empty() {
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!(
                                                    "init: created {} — enriching…",
                                                    created.join(", ")
                                                ),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        }
                                        let cwd2 = crate::guards::dir::project_root()
                                            .display()
                                            .to_string();
                                        let extra = format!("\nAdditional user note: {}", rest);
                                        let init_prompt2 = format!(
                                            r#"Initialize project memory for lean. CWD: {cwd2}{extra}

Explore codebase (ls, README, Cargo.toml etc.), then create/update ./AGENTS.md (project: overview, stack, commands, structure, conventions, architecture, gotchas — concise). Do NOT create CLAUDE.md or any MEMORY.md in the project — AGENTS.md only. MEMORY.md is global-only (~/.lean/MEMORY.md, silent background). Verify by reading ./AGENTS.md."#
                                        );
                                        let hist = llm_history_for_spawn(&session, &messages);
                                        let expanded = expand_at_mentions(&init_prompt2);
                                        if agent_busy {
                                            msg_queue.push(prompt.clone());
                                        } else {
                                            messages.push(Msg {
                                                role: "user".into(),
                                                content: prompt.clone(),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                            persist(&messages, &mut session, &model);
                                            agent_busy = true;
                                            agent_handle = Some(spawn_agent_with_history(
                                                expanded,
                                                model.clone(),
                                                hist,
                                                tx.clone(),
                                                done_tx.clone(),
                                            ));
                                        }
                                        crate::support::telemetry::record("init");
                                    } else {
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: format!("unknown command: {}", prompt),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                    }
                                    textarea.select_all();
                                    textarea.cut();
                                    hist_idx = None;
                                    ac_matches.clear();
                                    ac_idx = 0;
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
                                    let stats = crate::services::memory::api_stats();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: stats,
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                }
                                "/memory stats" => {
                                    let stats = crate::services::memory::api_stats();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: stats,
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                }
                                "/memory consolidate" => {
                                    let out = crate::services::memory::api_consolidate();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: out,
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
                                }
                                "/auto-accept" => {
                                    let next = if crate::agent::current_mode()
                                        == crate::agent::Mode::Auto
                                    {
                                        crate::agent::Mode::Norm
                                    } else {
                                        crate::agent::Mode::Auto
                                    };
                                    crate::agent::set_mode(next);
                                    match next {
                                        crate::agent::Mode::Auto => {
                                            if let Some(mut req) = pending_approval.take() {
                                                if let Some(tx) = req.tx.take() {
                                                    let _ = tx.send(true);
                                                }
                                            }
                                            while let Some(mut req) =
                                                crate::guards::approval::take_pending()
                                            {
                                                if let Some(tx) = req.tx.take() {
                                                    let _ = tx.send(true);
                                                }
                                            }
                                            crate::support::telemetry::record("mode_auto");
                                        }
                                        _ => crate::support::telemetry::record("mode_norm"),
                                    }
                                }
                                _ if prompt.starts_with("/auto-accept ") => {
                                    let rest = prompt
                                        .strip_prefix("/auto-accept ")
                                        .unwrap()
                                        .trim()
                                        .to_lowercase();
                                    let target_on = match rest.as_str() {
                                        "on" | "enable" | "enabled" => Some(true),
                                        "off" | "disable" | "disabled" => Some(false),
                                        _ => None,
                                    };
                                    if let Some(want_on) = target_on {
                                        let target = if want_on {
                                            crate::agent::Mode::Auto
                                        } else {
                                            crate::agent::Mode::Norm
                                        };
                                        let cur = crate::agent::current_mode();
                                        if target != cur {
                                            crate::agent::set_mode(target);
                                            if want_on {
                                                if let Some(mut req) = pending_approval.take() {
                                                    if let Some(tx) = req.tx.take() {
                                                        let _ = tx.send(true);
                                                    }
                                                }
                                                while let Some(mut req) =
                                                    crate::guards::approval::take_pending()
                                                {
                                                    if let Some(tx) = req.tx.take() {
                                                        let _ = tx.send(true);
                                                    }
                                                }
                                                crate::support::telemetry::record("mode_auto");
                                            } else {
                                                crate::support::telemetry::record("mode_norm");
                                            }
                                        }
                                    } else {
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: "usage: /auto-accept [on|off]".into(),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                    }
                                }
                                _ if prompt.starts_with("/allowlist ") => {
                                    let rest = prompt.strip_prefix("/allowlist ").unwrap().trim();
                                    if rest == "clear" {
                                        crate::guards::bash::allowlist_clear();
                                        crate::guards::dir::allowlist_clear();
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: "allowlists cleared (bash + dir)".into(),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                    } else if rest.starts_with("add ") {
                                        let pat = rest.strip_prefix("add ").unwrap().trim();
                                        // if pat looks like a path, add to dir allowlist, else bash
                                        if pat.contains('/')
                                            || pat.starts_with('~')
                                            || pat.starts_with('.')
                                        {
                                            crate::guards::dir::allowlist_add(pat);
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("dir-allowlisted: {}", pat),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        } else {
                                            crate::guards::bash::allowlist_add(pat);
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("allowlisted: {}", pat),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        }
                                    } else if rest.starts_with("rm ") || rest.starts_with("remove ")
                                    {
                                        let pat = rest
                                            .split_once(' ')
                                            .map(|(_, p)| p.trim())
                                            .unwrap_or(rest);
                                        crate::guards::bash::allowlist_remove(pat);
                                        crate::guards::dir::allowlist_remove(pat);
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: format!("removed: {}", pat),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                    } else {
                                        // heuristics: path-like → dir
                                        if rest.contains('/') || rest.starts_with('~') {
                                            crate::guards::dir::allowlist_add(rest);
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("dir-allowlisted: {}", rest),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        } else {
                                            crate::guards::bash::allowlist_add(rest);
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("allowlisted: {}", rest),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        }
                                    }
                                }
                                _ if prompt.starts_with("/resume ") => {
                                    let rid = prompt.strip_prefix("/resume ").unwrap().trim();
                                    let list = crate::services::session::Session::list();
                                    if let Some(sess) =
                                        list.into_iter().find(|sess| sess.id.starts_with(rid))
                                    {
                                        let sid = sess.id.clone();
                                        messages = sess
                                            .messages
                                            .iter()
                                            .map(|m| Msg {
                                                role: m.role.clone(),
                                                content: m.content.clone(),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            })
                                            .collect();
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: format!("[resumed session {}]", sid),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                        session = Some(sess);
                                        if let Some(ref s) = session {
                                            crate::integrations::herdr::report_session_obj(
                                                s,
                                                Some("resume"),
                                            );
                                        }
                                    } else {
                                        messages.push(Msg {
                                            role: "system".into(),
                                            content: format!("session {} not found", rid),
                                            tool_id: None,
                                            tool_name: None,
                                            tool_args: None,
                                            elapsed_ms: None,
                                        });
                                    }
                                }
                                "/model" => {
                                    // List current alias + available
                                    match crate::integrations::models::load() {
                                        Ok(cfg) => {
                                            let entry = cfg.models.get(&model);
                                            let detail = if let Some(e) = entry {
                                                let api_tag = match e.api_mode() {
                                                    crate::integrations::models::ApiMode::Responses => {
                                                        "responses"
                                                    }
                                                    crate::integrations::models::ApiMode::Anthropic => {
                                                        "anthropic"
                                                    }
                                                    _ => "chat",
                                                };
                                                format!(
                                                    " → {} @ {} [{}]",
                                                    e.model,
                                                    e.base_url
                                                        .as_deref()
                                                        .unwrap_or("env: OPENCODE_BASE_URL"),
                                                    api_tag
                                                )
                                            } else {
                                                String::new()
                                            };
                                            let mut aliases: Vec<String> =
                                                cfg.models.keys().cloned().collect();
                                            aliases.sort();
                                            let decorated: Vec<String> = aliases
                                                .iter()
                                                .map(|a| {
                                                    if let Some(e) = cfg.models.get(a) {
                                                        let tag = match e.api_mode() {
                                                            crate::integrations::models::ApiMode::Responses => {
                                                                "responses"
                                                            }
                                                            crate::integrations::models::ApiMode::Anthropic => {
                                                                "anthropic"
                                                            }
                                                            _ => "chat",
                                                        };
                                                        format!("{} ({})", a, tag)
                                                    } else {
                                                        a.clone()
                                                    }
                                                })
                                                .collect();
                                            let available = decorated.join(", ");
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("current model: {}{} (available: {})\nconfig: {} — use /model <alias> to switch", model, detail, available, crate::integrations::models::path_display()), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                        }
                                        Err(e) => {
                                            messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("models.json error: {}", e),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                        }
                                    }
                                }
                                _ if prompt.starts_with("/model ") => {
                                    let m = prompt.strip_prefix("/model ").unwrap().trim();
                                    if m.is_empty() {
                                        // bare /model with trailing space → list
                                        match crate::integrations::models::load() {
                                            Ok(cfg) => {
                                                let mut aliases: Vec<String> =
                                                    cfg.models.keys().cloned().collect();
                                                aliases.sort();
                                                messages.push(Msg {
                                                    role: "system".into(),
                                                    content: format!(
                                                        "current model: {} (available: {})",
                                                        model,
                                                        aliases.join(", ")
                                                    ),
                                                    tool_id: None,
                                                    tool_name: None,
                                                    tool_args: None,
                                                    elapsed_ms: None,
                                                });
                                            }
                                            Err(e) => messages.push(Msg {
                                                role: "system".into(),
                                                content: format!("models.json error: {}", e),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            }),
                                        }
                                    } else {
                                        match crate::integrations::models::resolve(Some(m)) {
                                            Ok(r) => {
                                                model = r.alias.clone();
                                                if let Some(s) = &mut session {
                                                    s.model = model.clone();
                                                    let _ = s.save();
                                                }
                                                let api_tag = match r.api_mode {
                                                    crate::integrations::models::ApiMode::Responses => {
                                                        "responses"
                                                    }
                                                    crate::integrations::models::ApiMode::Anthropic => {
                                                        "anthropic"
                                                    }
                                                    _ => "chat",
                                                };
                                                messages.push(Msg {
                                                    role: "system".into(),
                                                    content: format!("switched to {} ({} @ {} — {}) — live + persisted", r.alias, r.model, r.base_url, api_tag), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                            }
                                            Err(e) => {
                                                messages.push(Msg {
                                                    role: "system".into(),
                                                    content: format!("model switch failed: {}", e),
                                                    tool_id: None,
                                                    tool_name: None,
                                                    tool_args: None,
                                                    elapsed_ms: None,
                                                });
                                            }
                                        }
                                    }
                                }
                                _ if prompt.starts_with("/plan ") => {
                                    let task =
                                        prompt.strip_prefix("/plan ").unwrap().trim().to_string();
                                    if task.is_empty() {
                                        messages.push(Msg { role: "system".into(), content: "usage: /plan <task> — runs task in plan mode, or /plan to toggle".into(), tool_id: None, tool_name: None, tool_args: None, elapsed_ms: None});
                                    } else {
                                        if crate::agent::current_mode() != crate::agent::Mode::Plan
                                        {
                                            crate::agent::set_mode(crate::agent::Mode::Plan);
                                            crate::support::telemetry::record("mode_plan");
                                        }
                                        // Treat task as normal prompt but in plan mode
                                        let expanded_task = expand_at_mentions(&task);
                                        // Defer actual spawn to after textarea clear — set pending plan task
                                        // We push history and spawn directly here to avoid going through unknown path
                                        history.push(task.clone());
                                        hist_idx = None;
                                        step_info = "...".into();
                                        textarea.select_all();
                                        textarea.cut();
                                        auto_scroll = true;
                                        ac_matches.clear();
                                        ac_idx = 0;
                                        if agent_busy {
                                            msg_queue.push(task.clone());
                                        } else {
                                            let hist = llm_history_for_spawn(&session, &messages);
                                            messages.push(Msg {
                                                role: "user".into(),
                                                content: task.clone(),
                                                tool_id: None,
                                                tool_name: None,
                                                tool_args: None,
                                                elapsed_ms: None,
                                            });
                                            persist(&messages, &mut session, &model);
                                            agent_busy = true;
                                            agent_handle = Some(spawn_agent_with_history(
                                                expanded_task,
                                                model.clone(),
                                                hist,
                                                tx.clone(),
                                                done_tx.clone(),
                                            ));
                                        }
                                        // Skip generic textarea clear below by marking handled
                                        // (we already cleared, but set a flag to avoid double handling)
                                    }
                                }
                                _ => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("unknown command: {}", prompt),
                                        tool_id: None,
                                        tool_name: None,
                                        tool_args: None,
                                        elapsed_ms: None,
                                    });
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
                                    content: prompt.clone(),
                                    tool_id: None,
                                    tool_name: None,
                                    tool_args: None,
                                    elapsed_ms: None,
                                });
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
                                content: delta,
                                tool_id: None,
                                tool_name: None,
                                tool_args: None,
                                elapsed_ms: None,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "assistant".into(),
                            content: delta,
                            tool_id: None,
                            tool_name: None,
                            tool_args: None,
                            elapsed_ms: None,
                        });
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
                                content: delta,
                                tool_id: None,
                                tool_name: None,
                                tool_args: None,
                                elapsed_ms: None,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "thinking".into(),
                            content: delta,
                            tool_id: None,
                            tool_name: None,
                            tool_args: None,
                            elapsed_ms: None,
                        });
                    }
                }
                AgentEvent::TextDone { text } => {
                    let _ = text;
                }
                AgentEvent::ToolStart { name, args, id } => {
                    let args_str = if name == "ask_user" {
                        " waiting for your answer…".to_string()
                    } else {
                        serde_json::to_string(&args).unwrap_or_default()
                    };
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
                            if msg.tool_name.is_none() {
                                msg.tool_name = Some(name.clone());
                            }
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
                        let chunk_pairs: Vec<(String, String)> = messages
                            .iter()
                            .rev()
                            .take(12)
                            .rev()
                            .map(|m| (m.role.clone(), m.content.clone()))
                            .collect();
                        let chunk = crate::integrations::observer::build_chunk_text(&chunk_pairs);
                        let obs_model = model.clone();
                        tokio::spawn(async move {
                            crate::integrations::observer::observe_chunk(chunk, obs_model).await;
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
                            content: next.clone(),
                            tool_id: None,
                            tool_name: None,
                            tool_args: None,
                            elapsed_ms: None,
                        });
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
                    content: next.clone(),
                    tool_id: None,
                    tool_name: None,
                    tool_args: None,
                    elapsed_ms: None,
                });
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
                    content: "[agent crashed]".into(),
                    tool_id: None,
                    tool_name: None,
                    tool_args: None,
                    elapsed_ms: None,
                });
            }
        }

        // Persist session only when content changed (was every tick ~20Hz)
        let persist_fp: usize = messages
            .len()
            .wrapping_add(messages.iter().map(|m| m.content.len()).sum::<usize>());
        if persist_fp != last_persist_fingerprint {
            persist(&messages, &mut session, &model);
            last_persist_fingerprint = persist_fp;
        }
        // Advance spinner; force a redraw while it or any subagent is animating — now 10fps not 60fps
        let has_running_subagent = crate::services::agents::list_subagents()
            .iter()
            .any(|s| s.status == "running");
        if agent_busy || has_running_subagent || show_subagents {
            spinner_tick = spinner_tick.wrapping_add(1);
            dirty = true;
        }
    }

    Ok(())
}
