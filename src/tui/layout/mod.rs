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

/// Split token estimate into OpenCode-style in/out numbers.
/// in ≈ system prompt + user messages, out ≈ assistant + tool + thinking.
pub fn estimate_in_out(messages: &[Msg], model: &str) -> (usize, usize) {
    let mut in_chars: usize = 4000; // system prompt estimate
    let mut out_chars: usize = 0;
    for m in messages {
        let n = m.content.chars().count() + 8;
        match m.role.as_str() {
            "user" | "system" => in_chars += n,
            _ => out_chars += n,
        }
    }
    in_chars += model.len();
    (in_chars.div_ceil(4), out_chars.div_ceil(4))
}

/// Pretty display name for a model id/alias, e.g.
/// "muse-spark-1.3-contributor-free" → "Muse Spark 1.3 Free".
pub fn pretty_model_display(raw: &str) -> String {
    // Use the part after '/' for aliases like "opencode/muse".
    let base = raw.rsplit('/').next().unwrap_or(raw);
    let mut s = base.replace(['-', '_'], " ");
    // Drop filler words like "contributor" (screenshot shows "Muse Spark 1.3 Free").
    s = s
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case("contributor"))
        .collect::<Vec<_>>()
        .join(" ");
    // Title-case each word, leaving version numbers alone.
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().collect::<String>();
                    out.push_str(&chars.collect::<String>());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Provider label for the input info row. Maps zen base URLs to
/// "OpenCode Zen" (matches the reference screenshot), else provider id.
pub fn provider_display(base_url: &str, provider: &str) -> String {
    let lower = base_url.to_lowercase();
    if lower.contains("opencode.ai/zen") || lower.contains("/zen/v1") || lower.contains("zen") {
        return "OpenCode Zen".to_string();
    }
    if provider.is_empty() {
        return "openai".to_string();
    }
    provider.to_string()
}

/// Resolve (pretty_model, provider_label, variant_label) for an alias.
/// Falls back to the alias itself when models.json can't be read.
pub fn resolve_model_meta(alias: &str) -> (String, String, String) {
    let (model_id, base_url, provider, api) =
        match crate::integrations::models::resolve(Some(alias)) {
            Ok(r) => (
                r.model.clone(),
                r.base_url.clone(),
                r.provider.as_str().to_string(),
                match r.api_mode {
                    crate::integrations::models::ApiMode::Responses => "responses",
                    crate::integrations::models::ApiMode::Anthropic => "anthropic",
                    _ => "chat",
                }
                .to_string(),
            ),
            Err(_) => {
                if let Ok(cfg) = crate::integrations::models::load() {
                    if let Some(e) = cfg.models.get(alias) {
                        let base = e.base_url.clone().unwrap_or_default();
                        let prov = e.provider().as_str().to_string();
                        let api = match e.api_mode() {
                            crate::integrations::models::ApiMode::Responses => "responses",
                            crate::integrations::models::ApiMode::Anthropic => "anthropic",
                            _ => "chat",
                        }
                        .to_string();
                        (e.model.clone(), base, prov, api)
                    } else {
                        (
                            alias.to_string(),
                            String::new(),
                            "openai".to_string(),
                            "chat".to_string(),
                        )
                    }
                } else {
                    (
                        alias.to_string(),
                        String::new(),
                        "openai".to_string(),
                        "chat".to_string(),
                    )
                }
            }
        };
    let pretty = pretty_model_display(&model_id);
    let provider_label = provider_display(&base_url, &provider);
    (pretty, provider_label, api)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_model_drops_contributor_and_title_cases() {
        assert_eq!(
            pretty_model_display("muse-spark-1.3-contributor-free"),
            "Muse Spark 1.3 Free"
        );
        assert_eq!(pretty_model_display("opencode/muse"), "Muse");
        assert_eq!(pretty_model_display("gpt-4o"), "Gpt 4o");
    }

    #[test]
    fn provider_display_maps_zen_urls() {
        assert_eq!(
            provider_display("https://opencode.ai/zen/v1", "generic"),
            "OpenCode Zen"
        );
        assert_eq!(
            provider_display("http://127.0.0.1:8080/zen/v1", "generic"),
            "OpenCode Zen"
        );
        assert_eq!(
            provider_display("https://api.openai.com/v1", "openai"),
            "openai"
        );
    }

    #[test]
    fn estimate_in_out_splits_roles() {
        let msgs = vec![Msg::new("user", "hello"), Msg::new("assistant", "world")];
        let (input, out) = estimate_in_out(&msgs, "m");
        // in ≈ (4000 + user + model overhead) / 4, out ≈ assistant / 4
        assert!(input > out);
        assert_eq!(out, ("world".len() + 8 + 3) / 4);
    }

    #[test]
    fn format_context_label_is_est_plus_pct() {
        assert_eq!(format_context_label(118800, 1_000_000), "118.8K (12%)");
        assert_eq!(format_context_label(5000, 1_000_000), "5.0K (0.5%)");
        // pct clamps at 100
        assert_eq!(format_context_label(2_000_000, 1_000_000), "2.0M (100%)");
    }

    #[test]
    fn right_align_label_pins_to_right_with_margin() {
        assert_eq!(right_align_label("118.8K (12%)", 20), "       118.8K (12%)");
        // extreme widths truncate from the left instead of overflowing
        assert_eq!(right_align_label("118.8K (12%)", 6), "(12%)");
    }
}
