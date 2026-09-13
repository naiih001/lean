use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::ASHEN;

// ── Inline formatting ──────────────────────────────────────────

/// Parse inline markdown (bold, italic, code, links) into styled spans.
fn render_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Inline code: `...`
        if chars[i] == '`' {
            if let Some(end) = find_inline_code_end(&chars, i + 1) {
                let code: String = chars[i + 1..end].iter().collect();
                spans.push(Span::styled(
                    format!(" {} ", code),
                    base.fg(ASHEN.ember_glow).bg(ASHEN.stone),
                ));
                i = end + 1;
                continue;
            }
        }

        // Bold: **...**  (also __...__)
        if i + 1 < len && ((chars[i] == '*' && chars[i + 1] == '*') || (chars[i] == '_' && chars[i + 1] == '_')) {
            let marker = chars[i];
            if let Some(end) = find_double_marker_end(&chars, i + 2, marker) {
                let inner: String = chars[i + 2..end].iter().collect();
                if !inner.trim().is_empty() {
                    let mut inner_spans = render_inline(&inner, base);
                    for s in &mut inner_spans {
                        s.style = s.style.add_modifier(Modifier::BOLD).fg(ASHEN.bone);
                    }
                    spans.extend(inner_spans);
                    i = end + 2;
                    continue;
                }
            }
        }

        // Italic: *...* or _..._
        if (chars[i] == '*' || chars[i] == '_')
            && (i == 0 || chars[i - 1] != chars[i])
            && (i + 1 < len && chars[i + 1] != chars[i])
        {
            let marker = chars[i];
            if let Some(end) = find_single_marker_end(&chars, i + 1, marker) {
                let inner: String = chars[i + 1..end].iter().collect();
                if !inner.trim().is_empty() && !inner.contains('\n') {
                    let mut inner_spans = render_inline(&inner, base);
                    for s in &mut inner_spans {
                        s.style = s.style.add_modifier(Modifier::ITALIC).fg(ASHEN.pale_ash);
                    }
                    spans.extend(inner_spans);
                    i = end + 1;
                    continue;
                }
            }
        }

        // Link: [text](url) and ![alt](url)
        if chars[i] == '[' || (chars[i] == '!' && i + 1 < len && chars[i + 1] == '[') {
            let is_image = chars[i] == '!';
            let bracket_start = if is_image { i + 1 } else { i };
            if let Some(bracket_end) = find_char(&chars, bracket_start + 1, ']') {
                if bracket_end + 1 < len && chars[bracket_end + 1] == '(' {
                    if let Some(paren_end) = find_char(&chars, bracket_end + 2, ')') {
                        let link_text: String = chars[bracket_start + 1..bracket_end].iter().collect();
                        let url: String = chars[bracket_end + 2..paren_end].iter().collect();
                        let mut mods = Modifier::UNDERLINED;
                        if is_image { mods |= Modifier::ITALIC; }
                        spans.push(Span::styled(
                            link_text,
                            base.fg(ASHEN.frost).add_modifier(mods),
                        ));
                        spans.push(Span::styled(format!(" ({})", url), base.fg(ASHEN.deep_ash)));
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // Strikethrough: ~~...~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(end) = find_double_marker_end(&chars, i + 2, '~') {
                let inner: String = chars[i + 2..end].iter().collect();
                let mut inner_spans = render_inline(&inner, base);
                for s in &mut inner_spans {
                    s.style = s.style.add_modifier(Modifier::CROSSED_OUT).fg(ASHEN.deep_ash);
                }
                spans.extend(inner_spans);
                i = end + 2;
                continue;
            }
        }

        // Plain text
        let start = i;
        while i < len && !is_inline_special(chars[i]) {
            i += 1;
        }
        if i > start {
            let text: String = chars[start..i].iter().collect();
            spans.push(Span::styled(text, base));
        } else {
            spans.push(Span::styled(chars[i].to_string(), base));
            i += 1;
        }
    }

    spans
}

fn is_inline_special(c: char) -> bool {
    c == '*' || c == '`' || c == '[' || c == '!' || c == '_' || c == '~'
}

fn find_inline_code_end(chars: &[char], start: usize) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == '`' {
            return Some(i);
        }
    }
    None
}

fn find_double_marker_end(chars: &[char], start: usize, marker: char) -> Option<usize> {
    for i in start..chars.len() - 1 {
        if chars[i] == marker && chars[i + 1] == marker {
            return Some(i);
        }
    }
    None
}

fn find_single_marker_end(chars: &[char], start: usize, marker: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == marker && (i + 1 >= chars.len() || chars[i + 1] != marker) {
            return Some(i);
        }
    }
    None
}

fn find_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == target {
            return Some(i);
        }
    }
    None
}

// ── Block-level rendering ──────────────────────────────────────

pub fn render_markdown(content: &str, indent: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let pad = " ".repeat(indent);
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf: Vec<String> = Vec::new();
    let mut prev_was_header = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();

        // Fenced code block
        if trimmed.starts_with("```") {
            if in_code_block {
                let code_content = code_buf.join("\n");
                if code_content.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(" ", Style::default().bg(ASHEN.stone)),
                    ]));
                } else {
                    for cl in code_content.lines() {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{}  ", pad), Style::default()),
                            Span::styled(
                                cl.to_string(),
                                Style::default().fg(ASHEN.pale_ash).bg(ASHEN.stone),
                            ),
                        ]));
                    }
                }
                in_code_block = false;
                code_buf.clear();
                code_lang.clear();
                lines.push(Line::from(""));
                prev_was_header = false;
            } else {
                in_code_block = true;
                code_lang = trimmed.trim_start_matches('`').trim().to_string();
                if !code_lang.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(
                            format!("{} ", code_lang),
                            Style::default()
                                .fg(ASHEN.deep_ash)
                                .add_modifier(Modifier::ITALIC)
                                .bg(ASHEN.stone),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(" ", Style::default().bg(ASHEN.stone)),
                    ]));
                }
                prev_was_header = false;
            }
            continue;
        }

        if in_code_block {
            code_buf.push(raw_line.to_string());
            continue;
        }

        // Horizontal rule
        if is_horizontal_rule(trimmed) {
            lines.push(Line::from(vec![Span::styled(
                format!("{}  {}", pad, "─".repeat(42)),
                Style::default().fg(ASHEN.charcoal),
            )]));
            lines.push(Line::from(""));
            prev_was_header = false;
            continue;
        }

        // Headers
        if let Some(header) = parse_header(trimmed) {
            let (level, text) = header;
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            let (fg, mods) = match level {
                1 => (ASHEN.moss, Modifier::BOLD),
                2 => (ASHEN.bone, Modifier::BOLD),
                3 => (ASHEN.ember_glow, Modifier::BOLD),
                _ => (ASHEN.slate, Modifier::BOLD),
            };
            let prefix = match level {
                1 => "█".to_string(),
                2 => "▓".to_string(),
                _ => "#".repeat(level),
            };
            let mut spans = vec![Span::styled(
                format!("{} {} ", pad, prefix),
                Style::default().fg(fg).add_modifier(mods),
            )];
            spans.extend(render_inline(
                text,
                Style::default().fg(fg).add_modifier(mods),
            ));
            lines.push(Line::from(spans));
            if level <= 2 {
                lines.push(Line::from(Span::styled(
                    format!("{} {}", pad, "─".repeat(text.chars().count().min(48))),
                    Style::default().fg(ASHEN.charcoal),
                )));
            }
            lines.push(Line::from(""));
            prev_was_header = true;
            continue;
        }

        // Blockquote
        if let Some(quote_text) = trimmed.strip_prefix('>') {
            let qt = quote_text.strip_prefix(' ').unwrap_or(quote_text);
            let mut spans = vec![Span::styled(
                format!("{} │ ", pad),
                Style::default()
                    .fg(ASHEN.frost)
                    .add_modifier(Modifier::ITALIC),
            )];
            let base = Style::default()
                .fg(ASHEN.smoke)
                .add_modifier(Modifier::ITALIC);
            spans.extend(render_inline(qt, base));
            lines.push(Line::from(spans));
            prev_was_header = false;
            continue;
        }

        // Task list
        if let Some(task) = parse_task_list(trimmed) {
            let (checked, item) = task;
            let box_char = if checked { "☑" } else { "☐" };
            let mut spans = vec![
                Span::styled(format!("{}   ", pad), Style::default()),
                Span::styled(
                    format!("{} ", box_char),
                    Style::default().fg(if checked { ASHEN.moss } else { ASHEN.deep_ash }),
                ),
            ];
            let style = if checked {
                Style::default().fg(ASHEN.deep_ash).add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            spans.extend(render_inline(item, style));
            lines.push(Line::from(spans));
            prev_was_header = false;
            continue;
        }

        // Unordered list
        if let Some(item) = parse_unordered_list(trimmed) {
            let mut spans = vec![Span::styled(
                format!("{}   • ", pad),
                Style::default().fg(ASHEN.ember),
            )];
            spans.extend(render_inline(item, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
            prev_was_header = false;
            continue;
        }

        // Ordered list
        if let Some((num, item)) = parse_ordered_list(trimmed) {
            let mut spans = vec![Span::styled(
                format!("{}  {:>2}. ", pad, num),
                Style::default().fg(ASHEN.ember),
            )];
            spans.extend(render_inline(item, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
            prev_was_header = false;
            continue;
        }

        // Plain text
        if trimmed.is_empty() {
            if lines.last().map(|l| l.width() == 0).unwrap_or(false) {
                continue;
            }
            lines.push(Line::from(""));
        } else {
            let mut spans = vec![Span::styled(format!("{} ", pad), Style::default())];
            spans.extend(render_inline(trimmed, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
        }
        prev_was_header = false;
    }

    if in_code_block && !code_buf.is_empty() {
        for cl in code_buf.join("\n").lines() {
            lines.push(Line::from(vec![
                Span::styled(format!("{}  ", pad), Style::default()),
                Span::styled(
                    cl.to_string(),
                    Style::default().fg(ASHEN.pale_ash).bg(ASHEN.stone),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    while lines.len() > 1 && lines.last().map(|l| l.width() == 0).unwrap_or(false) && lines[lines.len() - 2].width() == 0 {
        lines.pop();
    }

    lines
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed.chars().next().unwrap();
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    trimmed.chars().all(|c| c == first || c == ' ')
}

fn parse_header(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let mut hash_count = 0;
    for c in trimmed.chars() {
        if c == '#' {
            hash_count += 1;
        } else {
            break;
        }
    }
    if hash_count >= 1 && hash_count <= 6 {
        let rest = &trimmed[hash_count..];
        if let Some(text) = rest.strip_prefix(' ') {
            if !text.is_empty() {
                return Some((hash_count, text));
            }
        }
    }
    None
}

fn parse_task_list(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim();
    for prefix in ["- [ ] ", "* [ ] ", "+ [ ] ", "- [x] ", "- [X] ", "* [x] ", "* [X] "] {
        if trimmed.starts_with(prefix) {
            let checked = prefix.contains("[x]") || prefix.contains("[X]");
            return Some((checked, &trimmed[prefix.len()..]));
        }
    }
    None
}

fn parse_unordered_list(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("- ") {
        Some(&trimmed[2..])
    } else if trimmed.starts_with("* ") {
        if trimmed.starts_with("**") { None } else { Some(&trimmed[2..]) }
    } else if trimmed.starts_with("+ ") {
        Some(&trimmed[2..])
    } else {
        None
    }
}

fn parse_ordered_list(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim();
    let mut digits = 0;
    for c in trimmed.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        } else {
            break;
        }
    }
    if digits > 0 && digits <= 4 {
        let after_digits = &trimmed[digits..];
        if after_digits.starts_with(". ") {
            let num = trimmed[..digits].parse::<usize>().unwrap_or(0);
            return Some((num, &after_digits[2..]));
        } else if after_digits.starts_with(") ") {
            let num = trimmed[..digits].parse::<usize>().unwrap_or(0);
            return Some((num, &after_digits[2..]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_bold() {
        let spans = render_inline("hello **world** foo", Style::default());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "hello ");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, "world");
        assert_eq!(spans[2].content, " foo");
    }

    #[test]
    fn test_inline_italic() {
        let spans = render_inline("a *b* c", Style::default());
        assert_eq!(spans.len(), 3);
        assert!(spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_inline_code() {
        let spans = render_inline("use `foo` here", Style::default());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content, " foo ");
    }

    #[test]
    fn test_unmatched_bracket_terminates() {
        let spans = render_inline("hello [unmatched", Style::default());
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello [unmatched");
    }

    #[test]
    fn test_bracketed_error_message_terminates() {
        let spans = render_inline("[LLM HTTP error: boom]", Style::default());
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[LLM HTTP error: boom]");
    }

    #[test]
    fn test_unmatched_star_and_backtick_terminate() {
        let a = render_inline("2 * 3", Style::default());
        let at: String = a.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(at, "2 * 3");

        let b = render_inline("a `b", Style::default());
        let bt: String = b.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(bt, "a `b");
    }

    #[test]
    fn test_valid_link_still_renders() {
        let spans = render_inline("[text](https://x.test)", Style::default());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "text");
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(spans[1].content, " (https://x.test)");
    }

    #[test]
    fn test_screenshot_sample() {
        let md = "## Key Code\n- **Agent prompt** (agent.rs): `You are Lean, a coding assistant` → Defines\n1. **TUI (Ratatui)** → renders UI";
        let lines = render_markdown(md, 2);
        let flat: String = lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.to_string())).collect::<Vec<_>>().join("\n");
        assert!(!flat.contains("**"), "stars should be stripped, got: {}", flat);
        assert!(!flat.contains("##"), "hashes should be styled not raw, got: {}", flat);
        assert!(flat.contains("Agent prompt"));
        assert!(flat.contains("TUI"));
    }

    #[test]
    fn test_header_parsing() {
        assert_eq!(parse_header("## Hello"), Some((2, "Hello")));
        assert_eq!(parse_header("# H1"), Some((1, "H1")));
        assert_eq!(parse_header("### H3"), Some((3, "H3")));
        assert_eq!(parse_header("no header"), None);
    }

    #[test]
    fn test_horizontal_rule() {
        assert!(is_horizontal_rule("---"));
        assert!(is_horizontal_rule("***"));
        assert!(is_horizontal_rule("___"));
        assert!(!is_horizontal_rule("--"));
        assert!(!is_horizontal_rule("text"));
    }

    #[test]
    fn test_unordered_list() {
        assert_eq!(parse_unordered_list("- item"), Some("item"));
        assert_eq!(parse_unordered_list("* item"), Some("item"));
        assert_eq!(parse_unordered_list("not list"), None);
    }

    #[test]
    fn test_ordered_list() {
        assert_eq!(parse_ordered_list("1. first"), Some((1, "first")));
        assert_eq!(parse_ordered_list("12. twelfth"), Some((12, "twelfth")));
        assert_eq!(parse_ordered_list("not list"), None);
    }
}
