use crate::tui::layout::wrap::hard_wrap_str;
use crate::tui::theme::{ASHEN, THEME};
use crate::tui::widgets::header::{build_content_lines, wrap_lines};
use crate::tui::widgets::messages::Msg;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw_subagent_summary(f: &mut Frame, area: Rect, tick: usize) {
    let subs: Vec<_> = crate::agents::list_subagents()
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


pub fn draw_subagent_list(f: &mut Frame, area: Rect, selected: usize, scroll: usize) {
    let mut subs: Vec<_> = crate::agents::list_subagents()
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


pub fn draw_subagent_detail(f: &mut Frame, area: Rect, idx: usize, scroll: u16) {
    let subs: Vec<_> = crate::agents::list_subagents()
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


pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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


pub fn draw_scrollbar(f: &mut Frame, area: Rect, total_lines: usize, viewport_h: usize, scroll: u16) {
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
