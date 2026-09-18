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
    pub mode_label: &'a str,
    pub mode_color: Color,
    pub pretty_model: &'a str,
    pub provider_label: &'a str,
    pub variant_label: &'a str,
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
//   row 0:            title  "■ {Mode} · {Model}"
//   rows 1..n:        textarea with a 1-col ember accent bar on the left
//   last row:         info   "{Model}  {Provider} · {Variant}"
pub fn draw_input(f: &mut Frame, area: Rect, textarea: &mut TextArea<'_>, meta: &InputMeta<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
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
    let mode_w = meta.mode_label.chars().count();
    // "■ {Mode} · {Model}" — keep model, truncate it first on narrow widths.
    let fixed = 2 + mode_w + 3; // "■ " + mode + " · "
    let model_max = width.saturating_sub(1).saturating_sub(fixed);
    let model_txt = truncate_to(meta.pretty_model, model_max.max(1));
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
        Span::styled(
            " · ",
            Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg),
        ),
        Span::styled(
            model_txt,
            Style::default().fg(ASHEN.bone).bg(THEME.input_bg),
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

/// Build the bottom info row: "{Model}  {Provider} · {Variant}".
/// Mode lives in the title row only — no duplicate indicator down here.
/// Drops provider/variant first when space is tight.
fn info_line_spans(meta: &InputMeta<'_>, width: usize) -> Line<'static> {
    let dot = " · ";
    // Full: "Muse Spark 1.3 Free  OpenCode Zen · xhigh"
    let full_len = meta.pretty_model.chars().count()
        + 2
        + meta.provider_label.chars().count()
        + dot.chars().count()
        + meta.variant_label.chars().count();
    let bg = THEME.input_bg;
    let mut spans = vec![Span::styled(
        meta.pretty_model.to_string(),
        Style::default().fg(ASHEN.bone).bg(bg),
    )];
    if full_len <= width {
        spans.push(Span::styled("  ", Style::default().bg(bg)));
        spans.push(Span::styled(
            meta.provider_label.to_string(),
            Style::default().fg(ASHEN.deep_ash).bg(bg),
        ));
        spans.push(Span::styled(
            dot,
            Style::default().fg(ASHEN.charcoal).bg(bg),
        ));
        spans.push(Span::styled(
            meta.variant_label.to_string(),
            Style::default()
                .fg(ASHEN.ember)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }
    // Tight: try "Model  Provider" (drop variant).
    let no_variant_len = full_len - dot.chars().count() - meta.variant_label.chars().count();
    if no_variant_len <= width && !meta.provider_label.is_empty() {
        spans.push(Span::styled("  ", Style::default().bg(bg)));
        spans.push(Span::styled(
            truncate_to(
                meta.provider_label,
                width.saturating_sub(meta.pretty_model.chars().count() + 2),
            ),
            Style::default().fg(ASHEN.deep_ash).bg(bg),
        ));
        return Line::from(spans);
    }
    // Narrow: truncated model only.
    spans[0] = Span::styled(
        truncate_to(meta.pretty_model, width.max(1)),
        Style::default().fg(ASHEN.bone).bg(bg),
    );
    Line::from(spans)
}
