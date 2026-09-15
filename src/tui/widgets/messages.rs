use crate::tui::markdown;
use crate::tui::theme::{ASHEN, THEME};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
    pub tool_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args: Option<String>,
    pub elapsed_ms: Option<u64>,
}

impl Msg {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_id: None,
            tool_name: None,
            tool_args: None,
            elapsed_ms: None,
        }
    }
    pub fn new_tool_start(name: String, args: String, id: String) -> Self {
        Self {
            role: "tool".into(),
            content: format!("{} {}", name, args),
            tool_id: Some(id),
            tool_name: Some(name.clone()),
            tool_args: Some(args),
            elapsed_ms: None,
        }
    }
    pub fn format_elapsed(&self) -> String {
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

pub fn shorten_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && p.starts_with(&home) {
        format!("~{}", &p[home.len()..])
    } else {
        p.to_string()
    }
}

pub fn format_tool_preview(name: &str, args_str: &str) -> String {
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

pub fn sanitize_display_content(s: &str) -> String {
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
    pub fn render_lines(&self) -> Vec<Line<'static>> {
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
                lines
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
                md
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

    pub fn render_tool_box(&self) -> Vec<Line<'static>> {
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
    pub fn render_tool_lines(&self) -> Vec<Line<'static>> {
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
pub fn merge_thinking(messages: &[Msg]) -> Vec<Msg> {
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

