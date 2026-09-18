use crate::tui::theme::{ASHEN, THEME};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use ratatui_textarea::{Input as TAInput, Key as TAKey, TextArea};

pub fn crossterm_key_to_input(k: crossterm::event::KeyEvent) -> TAInput {
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

/// Meta shown in the OpenCode-style input chrome (title + info rows).
pub struct InputMeta<'a> {
    pub mode_label: String,
    pub mode_color: Color,
    pub pretty_model: &'a str,
}

fn truncate_to(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    format!("{}…", chars[..max - 1].iter().collect::<String>())
}

// ── OpenCode-style input ─────────────────────────────────────────
// Layout inside `area` (height = text_rows + 2):
//   row 0:            title  "■ {Mode}"
//   rows 1..n:        textarea with a 1-col ember accent bar on the left
//   last row:         info   "{Model}"
pub fn draw_input(f: &mut Frame, area: Rect, textarea: &mut TextArea<'_>, meta: &InputMeta<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Apply styling every frame (cheap). Solid block cursor like the reference.
    textarea.set_style(Style::default().fg(ASHEN.bone).bg(THEME.input_bg));
    textarea.set_cursor_style(Style::default().fg(THEME.input_bg).bg(ASHEN.bone));
    textarea.set_cursor_line_style(Style::default().bg(THEME.input_bg));
    textarea.set_placeholder_text("▸ type a message…");
    textarea.set_placeholder_style(Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg));
    textarea.set_block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(THEME.input_bg)),
    );

    // Fill the whole block so title/info rows share the input bg.
    let bg = Block::default().style(Style::default().bg(THEME.input_bg));
    f.render_widget(bg, area);

    // Accent bar: 1-col "│" in ember spanning the full block height.
    let accent = Paragraph::new(
        (0..area.height)
            .map(|_| {
                Line::from(Span::styled(
                    "│",
                    Style::default().fg(ASHEN.ember).bg(THEME.input_bg),
                ))
            })
            .collect::<Vec<_>>(),
    );
    f.render_widget(
        accent,
        Rect {
            x: area.x,
            y: area.y,
            width: 1.min(area.width),
            height: area.height,
        },
    );

    let inner_x = area.x + 1.min(area.width);
    let inner_w = area.width.saturating_sub(1);
    if inner_w == 0 {
        return;
    }
    let text_rows = (area.height as usize).saturating_sub(2).max(1) as u16;

    // ── Title row ──
    let title_area = Rect {
        x: inner_x,
        y: area.y,
        width: inner_w,
        height: 1,
    };
    // "■ {Mode}" — model lives in the info row only.
    let title = Line::from(vec![
        Span::styled(
            "■ ",
            Style::default()
                .fg(meta.mode_color)
                .bg(THEME.input_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            meta.mode_label.to_string(),
            Style::default()
                .fg(meta.mode_color)
                .bg(THEME.input_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(
        Paragraph::new(title).style(Style::default().bg(THEME.input_bg)),
        title_area,
    );

    // ── Textarea ──
    let text_area = Rect {
        x: inner_x,
        y: area.y + 1,
        width: inner_w,
        height: text_rows.min(area.height.saturating_sub(2).max(1)),
    };
    f.render_widget(&*textarea, text_area);

    // ── Info row ──
    let info_area = Rect {
        x: inner_x,
        y: area.y + area.height - 1,
        width: inner_w,
        height: 1,
    };
    let info_line = info_line_spans(meta, inner_w as usize);
    f.render_widget(
        Paragraph::new(info_line).style(Style::default().bg(THEME.input_bg)),
        info_area,
    );
}

/// Build the bottom info row: "{Model}" only.
/// Mode lives in the title row only — no duplicate indicator down here.
fn info_line_spans(meta: &InputMeta<'_>, width: usize) -> Line<'static> {
    let bg = THEME.input_bg;
    Line::from(vec![Span::styled(
        truncate_to(meta.pretty_model, width.max(1)),
        Style::default().fg(ASHEN.bone).bg(bg),
    )])
}
