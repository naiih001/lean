use crate::tui::theme::{ASHEN, THEME};
use crate::tui::widgets::messages::{merge_thinking, Msg};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw_running_indicator(f: &mut Frame, area: Rect, busy: bool, tick: usize) {
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

pub fn draw_header(f: &mut Frame, area: Rect, _model: &str, step_info: &str) {
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

pub fn draw_queue(f: &mut Frame, area: Rect, queue: &[String]) {
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

pub fn draw_separator(f: &mut Frame, area: Rect) {
    let sep = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(THEME.separator).bg(THEME.page_bg),
    )));
    f.render_widget(sep, area);
}

/// Wrap lines to fit within a given width. Preserves line.style bg (used for
/// full-width wash behind user/assistant blocks) and pads each washed line to
/// `width` with trailing spaces so the bg extends to the right edge.
pub fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
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

pub fn is_banner(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("mcp server running on stdio") || l.contains("github mcp server running on stdio")
}

/// Build the full list of styled lines from messages (without wrapping).
pub fn build_content_lines(messages: &[Msg]) -> Vec<Line<'static>> {
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
