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

/// Compact token estimate for the footer, e.g. 999, 118.8K, 1.5M.
pub fn format_est(est: usize) -> String {
    if est >= 1_000_000 {
        format!("{:.1}M", est as f64 / 1_000_000.0)
    } else if est >= 1000 {
        format!("{:.1}K", est as f64 / 1000.0)
    } else {
        format!("{}", est)
    }
}

pub fn format_context_label(est: usize, window: usize) -> String {
    let pct = ((est as f64 / window as f64) * 100.0).min(100.0);
    // Show pct as integer, but keep one decimal if <10%
    let pct_str = if pct < 10.0 {
        format!("{:.1}%", pct)
    } else {
        format!("{:.0}%", pct)
    };
    format!("{} ({})", format_est(est), pct_str)
}

/// Right-align a short footer label with a 1-col right margin. Truncates from
/// the left on extreme widths so the row never bleeds past the edge.
pub fn right_align_label(label: &str, width: usize) -> String {
    let label_w = label.chars().count();
    if label_w + 1 > width {
        label
            .chars()
            .skip(label_w + 1 - width.max(1))
            .collect::<String>()
    } else {
        format!("{}{}", " ".repeat(width - label_w - 1), label)
    }
}
