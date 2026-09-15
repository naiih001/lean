pub mod wrap;
pub use wrap::{
    char_display_width, hard_wrap_str, line_display_width, wrap_cursor_line, wrap_split,
};

use crate::tui::widgets::messages::Msg;

pub fn context_window_for_model(model: &str) -> usize {
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

pub fn estimate_tokens(messages: &[Msg], model: &str) -> usize {
    // ~4 chars per token + per-message overhead + system prompt
    let mut chars: usize = 4000; // system prompt estimate
    for m in messages {
        chars += m.content.chars().count() + 8;
    }
    // Also include model name overhead
    chars += model.len();
    (chars + 3) / 4
}

pub fn format_context_label(est: usize, window: usize) -> String {
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
