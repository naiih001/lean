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

        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_double_star_end(&chars, i + 2) {
                let inner: String = chars[i + 2..end].iter().collect();
                let mut spans_inner = render_inline(&inner, base);
                for s in &mut spans_inner {
                    s.style = s.style.add_modifier(Modifier::BOLD).fg(ASHEN.bone);
                }
                spans.extend(spans_inner);
                i = end + 2;
                continue;
            }
        }

        // Italic: *...* (single star, not preceded/followed by *)
        if chars[i] == '*'
            && (i == 0 || chars[i - 1] != '*')
            && (i + 1 < len && chars[i + 1] != '*')
        {
            if let Some(end) = find_single_star_end(&chars, i + 1) {
                let inner: String = chars[i + 1..end].iter().collect();
                let mut spans_inner = render_inline(&inner, base);
                for s in &mut spans_inner {
                    s.style = s.style.add_modifier(Modifier::ITALIC);
                }
                spans.extend(spans_inner);
                i = end + 1;
                continue;
            }
        }

        // Link: [text](url)
        if chars[i] == '[' {
            if let Some(bracket_end) = find_char(&chars, i + 1, ']') {
                if bracket_end + 1 < len && chars[bracket_end + 1] == '(' {
                    if let Some(paren_end) = find_char(&chars, bracket_end + 2, ')') {
                        let link_text: String = chars[i + 1..bracket_end].iter().collect();
                        let url: String = chars[bracket_end + 2..paren_end].iter().collect();
                        spans.push(Span::styled(
                            link_text,
                            base.fg(ASHEN.frost).add_modifier(Modifier::UNDERLINED),
                        ));
                        spans.push(Span::styled(
                            format!(" ({})", url),
                            base.fg(ASHEN.deep_ash),
                        ));
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // Plain text: collect until next special char
        let start = i;
        while i < len && !is_inline_special(chars[i]) {
            i += 1;
        }
        if i > start {
            let text: String = chars[start..i].iter().collect();
            spans.push(Span::styled(text, base));
        }
    }

    spans
}

fn is_inline_special(c: char) -> bool {
    c == '*' || c == '`' || c == '['
}

fn find_inline_code_end(chars: &[char], start: usize) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == '`' {
            return Some(i);
        }
    }
    None
}

fn find_double_star_end(chars: &[char], start: usize) -> Option<usize> {
    for i in start..chars.len() - 1 {
        if chars[i] == '*' && chars[i + 1] == '*' {
            return Some(i);
        }
    }
    None
}

fn find_single_star_end(chars: &[char], start: usize) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == '*' && (i + 1 >= chars.len() || chars[i + 1] != '*') {
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

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();

        // Fenced code block start/end
        if trimmed.starts_with("```") {
            if in_code_block {
                // End code block
                let code_content = code_buf.join("\n");
                for cl in code_content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(
                            cl.to_string(),
                            Style::default()
                                .fg(ASHEN.pale_ash)
                                .bg(ASHEN.stone),
                        ),
                    ]));
                }
                if code_buf.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(
                            " ",
                            Style::default().bg(ASHEN.stone),
                        ),
                    ]));
                }
                in_code_block = false;
                code_buf.clear();
                code_lang.clear();
            } else {
                // Start code block
                in_code_block = true;
                code_lang = trimmed.trim_start_matches('`').trim().to_string();
                // Show language tag if present
                if !code_lang.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}  ", pad), Style::default()),
                        Span::styled(
                            code_lang.clone(),
                            Style::default()
                                .fg(ASHEN.deep_ash)
                                .add_modifier(Modifier::ITALIC)
                                .bg(ASHEN.stone),
                        ),
                    ]));
                }
            }
            continue;
        }

        if in_code_block {
            code_buf.push(raw_line.to_string());
            continue;
        }

        // Horizontal rule: --- or *** or ___
        if is_horizontal_rule(trimmed) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}   {}", pad, "─".repeat(40)),
                    Style::default().fg(ASHEN.charcoal),
                ),
            ]));
            continue;
        }

        // Headers: # ... ######
        if let Some(header) = parse_header(trimmed) {
            let (level, text) = header;
            let (fg, modifiers) = match level {
                1 => (ASHEN.moss, Modifier::BOLD),
                2 => (ASHEN.frost, Modifier::BOLD),
                3 => (ASHEN.ember_glow, Modifier::BOLD),
                _ => (ASHEN.slate, Modifier::BOLD),
            };
            let prefix = "#".repeat(level);
            let mut spans = vec![
                Span::styled(format!("{} {} ", pad, prefix), Style::default().fg(fg).add_modifier(modifiers)),
            ];
            spans.extend(render_inline(
                text,
                Style::default().fg(fg).add_modifier(modifiers),
            ));
            lines.push(Line::from(spans));
            if level <= 2 {
                // Underline for h1/h2
                lines.push(Line::from(Span::styled(
                    format!("{} {}", pad, "─".repeat(text.chars().count().min(50))),
                    Style::default().fg(ASHEN.charcoal),
                )));
            }
            continue;
        }

        // Blockquote: > ...
        if let Some(quote_text) = trimmed.strip_prefix('>') {
            let qt = quote_text.strip_prefix(' ').unwrap_or(quote_text);
            let mut spans = vec![
                Span::styled(
                    format!("{} │ ", pad),
                    Style::default().fg(ASHEN.frost).add_modifier(Modifier::ITALIC),
                ),
            ];
            let base = Style::default()
                .fg(ASHEN.smoke)
                .add_modifier(Modifier::ITALIC);
            spans.extend(render_inline(qt, base));
            lines.push(Line::from(spans));
            continue;
        }

        // Unordered list: - or *
        if let Some(item) = parse_unordered_list(trimmed) {
            let mut spans = vec![
                Span::styled(
                    format!("{}   • ", pad),
                    Style::default().fg(ASHEN.ember),
                ),
            ];
            spans.extend(render_inline(item, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
            continue;
        }

        // Ordered list: 1. or 1)
        if let Some((num, item)) = parse_ordered_list(trimmed) {
            let mut spans = vec![
                Span::styled(
                    format!("{}   {}. ", pad, num),
                    Style::default().fg(ASHEN.ember),
                ),
            ];
            spans.extend(render_inline(item, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
            continue;
        }

        // Plain text line
        if trimmed.is_empty() {
            lines.push(Line::from(""));
        } else {
            let mut spans = vec![Span::styled(format!("{} ", pad), Style::default())];
            spans.extend(render_inline(trimmed, Style::default().fg(ASHEN.smoke)));
            lines.push(Line::from(spans));
        }
    }

    // Flush any unclosed code block
    if in_code_block && !code_buf.is_empty() {
        let code_content = code_buf.join("\n");
        for cl in code_content.lines() {
            lines.push(Line::from(vec![
                Span::styled(format!("{}  ", pad), Style::default()),
                Span::styled(
                    cl.to_string(),
                    Style::default()
                        .fg(ASHEN.pale_ash)
                        .bg(ASHEN.stone),
                ),
            ]));
        }
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
            return Some((hash_count, text));
        }
    }
    None
}

fn parse_unordered_list(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("- ") {
        Some(&trimmed[2..])
    } else if trimmed.starts_with("* ") {
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
