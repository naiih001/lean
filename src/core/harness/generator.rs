//! Generator helpers — the legacy agent tool loop viewed as a harness step.
//!
//! TODO(HARNESS-REPLACE-LOOP): the Responses/Chat step bodies in
//! `core/agent.rs` move here as `run_generator_turn`. Until that move lands,
//! the harness calls the legacy loop per sprint and only owns orchestration
//! (planner → generator → evaluator, blocking retries).

use serde_json::Value;

/// Complex-task gate: the Planner only runs for non-conversational goals.
/// Smalltalk (`hi`, `thanks`, …) skips plan+eval entirely.
pub fn is_complex_task(user_prompt: &str) -> bool {
    !crate::core::tracker::PlanTracker::new(user_prompt).is_conversational_goal()
}

/// Build the evaluator-visible transcript from chat-shaped history.
///
/// **Plan-blindness:** this must contain work product only — never the `Plan`.
/// Callers pass the post-sprint assistant/tool history; plan text is never
/// inserted here.
pub fn transcript_for_eval(history: &[Value], final_text: &str) -> String {
    const MAX_CHARS: usize = 12_000;
    let mut parts: Vec<String> = Vec::new();
    for m in history {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        // Only assistant + tool outputs carry work evidence.
        if role != "assistant" && role != "tool" {
            continue;
        }
        if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
            if !s.trim().is_empty() {
                parts.push(s.to_string());
            }
        } else if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
            for p in arr {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
        // tool_calls JSON shape carries no prose — skip.
    }
    if !final_text.trim().is_empty() && !parts.iter().any(|p| p.contains(final_text.trim())) {
        parts.push(final_text.to_string());
    }
    let mut out = parts.join("\n\n---\n\n");
    if out.chars().count() > MAX_CHARS {
        out = format!(
            "{}… [truncated]",
            out.chars().take(MAX_CHARS).collect::<String>()
        );
    }
    out
}

/// One-line plan summary injected into the generator prompt for a sprint.
/// The Evaluator never sees this — only the Generator prompt does.
pub fn plan_summary(plan: &crate::core::harness::types::Plan) -> String {
    let feats: Vec<String> = plan
        .features
        .iter()
        .map(|f| {
            format!(
                "- {}: {} (accept: {})",
                f.name,
                f.description,
                f.acceptance_criteria.join("; ")
            )
        })
        .collect();
    format!("Plan '{}':\n{}", plan.title, feats.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn complex_gate_skips_smalltalk() {
        assert!(!is_complex_task("hi"));
        assert!(!is_complex_task("thanks!"));
        assert!(is_complex_task("fix the login redirect bug"));
    }

    #[test]
    fn transcript_excludes_plan_and_caps_length() {
        let history = vec![
            json!({"role": "system", "content": "Plan 'x': secret-plan-marker"}),
            json!({"role": "assistant", "content": "did the thing"}),
            json!({"role": "tool", "tool_call_id": "1", "content": "ok"}),
        ];
        let t = transcript_for_eval(&history, "did the thing");
        assert!(t.contains("did the thing"));
        assert!(!t.contains("secret-plan-marker"));
    }
}
