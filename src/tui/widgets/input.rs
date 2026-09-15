use crate::tui::theme::{ASHEN, THEME};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::Frame;
use ratatui::widgets::{Block, Borders};
use ratatui_textarea::{TextArea, Input as TAInput, Key as TAKey};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

// ── Hard wrap ──────────────────────────────────────────────────


pub fn draw_input(f: &mut Frame, area: Rect, textarea: &mut TextArea<'_>) {
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

