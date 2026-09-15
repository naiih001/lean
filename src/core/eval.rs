//! Offline eval harness — deterministic, no LLM, no network.
//!
//! Simulates model turns / tool results to test the agent loop's state
//! machine, completion rules, and recovery prompts. Each fixture asserts
//! the expected evidence-driven transitions per `docs/behaviour-plans/agent-issue-solving-behaviour.md`.

use serde_json::json;

use crate::core::tracker::{PlanTracker, RequestMode};

/// Result shape for a single eval (easy to display + assert).
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub fixture: String,
    pub passed: bool,
    pub request_mode: String,
    pub tool_calls: Vec<String>,
    pub mutations_attempted: bool,
    pub mutations_succeeded: bool,
    pub verification_attempted: bool,
    pub verification_passed: bool,
    pub final_text: String,
    pub failure_reason: Option<String>,
}

impl EvalResult {
    fn ok(
        fixture: &str,
        tracker: &PlanTracker,
        tool_calls: Vec<String>,
        final_text: String,
    ) -> Self {
        Self {
            fixture: fixture.to_string(),
            passed: true,
            request_mode: format!("{:?}", tracker.request_mode()),
            tool_calls,
            mutations_attempted: tracker.mutation_attempted,
            mutations_succeeded: tracker.mutation_succeeded,
            verification_attempted: tracker.verification_attempted,
            verification_passed: tracker.verification_passed,
            final_text,
            failure_reason: None,
        }
    }
    fn fail(
        fixture: &str,
        tracker: &PlanTracker,
        tool_calls: Vec<String>,
        final_text: String,
        reason: String,
    ) -> Self {
        Self {
            fixture: fixture.to_string(),
            passed: false,
            request_mode: format!("{:?}", tracker.request_mode()),
            tool_calls,
            mutations_attempted: tracker.mutation_attempted,
            mutations_succeeded: tracker.mutation_succeeded,
            verification_attempted: tracker.verification_attempted,
            verification_passed: tracker.verification_passed,
            final_text,
            failure_reason: Some(reason),
        }
    }
}

/// Simulate a turn sequence: request + ordered (tool_name, args, result) + final assistant text.
/// Returns EvalResult with pass/fail based on `expect` closure.
fn run_fixture(
    fixture: &str,
    request: &str,
    steps: Vec<(&str, serde_json::Value, &str)>,
    final_text: &str,
    expect: impl Fn(&PlanTracker, &str) -> Option<String>,
) -> EvalResult {
    let mut tracker = PlanTracker::new(request);
    let mut tool_calls = Vec::new();
    for (name, args, result) in &steps {
        tool_calls.push(name.to_string());
        // record_tools covers inspected/mutation flags
        tracker.record_tools(&[name.to_string()]);
        tracker.note_tool_result(name, args, result);
        // record text as if model produced a reasoning chunk
        if !result.is_empty() {
            tracker.record_text(result);
        }
    }
    tracker.record_text(final_text);
    let tool_call_refs: Vec<String> = tool_calls.clone();
    if let Some(reason) = expect(&tracker, final_text) {
        EvalResult::fail(
            fixture,
            &tracker,
            tool_call_refs,
            final_text.to_string(),
            reason,
        )
    } else {
        EvalResult::ok(fixture, &tracker, tool_call_refs, final_text.to_string())
    }
}

// ---------------------------------------------------------------------------
// Fixtures (7 categories from the design doc)
// ---------------------------------------------------------------------------

/// Fixture 1: simple file edit — must read, edit, verify.
pub fn fixture_simple_file_edit() -> EvalResult {
    run_fixture(
        "simple file edit",
        "fix typo in src/main.rs",
        vec![
            (
                "read",
                json!({"path": "src/main.rs"}),
                "fn main() { let x = 1; }",
            ),
            (
                "edit",
                json!({"path": "src/main.rs", "oldText": "let x = 1", "newText": "let x = 2"}),
                "ok",
            ),
            (
                "bash",
                json!({"command": "cargo check"}),
                "Finished dev [unoptimized]",
            ),
        ],
        "Fixed typo and verified. All done.",
        |tracker, final_text| {
            if !tracker.inspected {
                return Some("should have inspected before edit".to_string());
            }
            if !tracker.mutation_succeeded {
                return Some("mutation should have succeeded".to_string());
            }
            if !tracker.verification_attempted || !tracker.verification_passed {
                return Some("verification should have passed".to_string());
            }
            if !tracker.looks_complete(final_text) {
                return Some(
                    "final text should be considered complete (evidence gate)".to_string(),
                );
            }
            None
        },
    )
}

/// Fixture 2: compile error fix — reads error, edits, reruns check successfully.
pub fn fixture_compile_error_fix() -> EvalResult {
    run_fixture(
        "compile error fix",
        "fix compile error",
        vec![
            (
                "bash",
                json!({"command": "cargo check"}),
                "error: expected `;` at src/lib.rs:10",
            ),
            ("read", json!({"path": "src/lib.rs"}), "let y = 5"),
            (
                "edit",
                json!({"path": "src/lib.rs", "oldText": "let y = 5", "newText": "let y = 5;"}),
                "ok",
            ),
            ("bash", json!({"command": "cargo check"}), "Finished dev"),
        ],
        "Fixed missing semicolon, cargo check passes. All done.",
        |tracker, final_text| {
            if tracker.verification_failed {
                return Some("verification should not be failed after fix".to_string());
            }
            if !tracker.verification_passed {
                return Some("verification should have passed after fix".to_string());
            }
            if !tracker.looks_complete(final_text) {
                return Some("should be complete after verification".to_string());
            }
            None
        },
    )
}

/// Fixture 3: failed unique edit recovery — stale oldText, then read + corrected edit.
pub fn fixture_failed_unique_edit_recovery() -> EvalResult {
    run_fixture(
        "failed unique edit recovery",
        "update handler in src/app.rs",
        vec![
            ("read", json!({"path": "src/app.rs"}), "handler old"),
            (
                "edit",
                json!({"path": "src/app.rs", "oldText": "stale text", "newText": "new"}),
                "Error: oldText not found",
            ),
            (
                "read",
                json!({"path": "src/app.rs"}),
                "handler current content",
            ),
            (
                "edit",
                json!({"path": "src/app.rs", "oldText": "current content", "newText": "new content"}),
                "ok",
            ),
            ("bash", json!({"command": "cargo check"}), "Finished"),
        ],
        "Recovered from stale edit, verified. All done.",
        |tracker, final_text| {
            if !tracker.tool_failed {
                return Some("should have recorded tool failure".to_string());
            }
            // failed_signatures should contain the failed edit
            if tracker.failed_signatures.is_empty() {
                return Some("should have stored failed signature to avoid retry".to_string());
            }
            // focus hint should have required re-read (we test via completion_blocker_hint after first failure would have suggested)
            if !tracker.mutation_succeeded {
                return Some("second edit should succeed".to_string());
            }
            if !tracker.looks_complete(final_text) {
                return Some("should be complete after recovery + verification".to_string());
            }
            None
        },
    )
}

/// Fixture 4: multi-file diagnosis — must search/read before editing correct file.
pub fn fixture_multi_file_diagnosis() -> EvalResult {
    run_fixture(
        "multi-file diagnosis",
        "fix auth bug where token expires",
        vec![
            (
                "grep",
                json!({"pattern": "token"}),
                "src/auth.rs: token = ...\nsrc/middleware.rs: token ...",
            ),
            ("read", json!({"path": "src/auth.rs"}), "fn validate() ..."),
            (
                "edit",
                json!({"path": "src/auth.rs", "oldText": "validate()", "newText": "validate_fixed()"}),
                "ok",
            ),
            ("bash", json!({"command": "cargo check"}), "Finished"),
        ],
        "Found bug in auth.rs via search, fixed and verified. All done.",
        |tracker, final_text| {
            if !tracker.inspected {
                return Some("should have inspected via grep/read before edit".to_string());
            }
            if !tracker.mutation_succeeded {
                return Some("should have edited correct file".to_string());
            }
            if !tracker.looks_complete(final_text) {
                return Some("should be complete after diagnosis + verification".to_string());
            }
            None
        },
    )
}

/// Fixture 5: verification required after mutation — must NOT finish after edit until verification.
pub fn fixture_verification_required_after_mutation() -> EvalResult {
    // First check: edit without verification should NOT be considered complete
    let mut tracker = PlanTracker::new("add feature to src/feature.rs");
    tracker.record_tools(&["read".to_string()]);
    tracker.note_tool_result("read", &json!({"path": "src/feature.rs"}), "content");
    tracker.record_tools(&["edit".to_string()]);
    tracker.note_tool_result(
        "edit",
        &json!({"path": "src/feature.rs", "oldText": "a", "newText": "b"}),
        "ok",
    );
    let final_text = "Edited file. All done.";
    tracker.record_text(final_text);
    let premature_complete = tracker.looks_complete(final_text);
    if premature_complete {
        return EvalResult::fail(
            "verification required after mutation",
            &tracker,
            vec!["read".to_string(), "edit".to_string()],
            final_text.to_string(),
            "should NOT be complete after edit without verification".to_string(),
        );
    }
    // Now verify and check it becomes complete
    tracker.note_tool_result("bash", &json!({"command": "cargo check"}), "Finished");
    tracker.record_tools(&["bash".to_string()]);
    let final_text2 = "Edited and verified. All done.";
    tracker.record_text(final_text2);
    if !tracker.looks_complete(final_text2) {
        return EvalResult::fail(
            "verification required after mutation",
            &tracker,
            vec!["read".to_string(), "edit".to_string(), "bash".to_string()],
            final_text2.to_string(),
            "should be complete after verification".to_string(),
        );
    }
    EvalResult::ok(
        "verification required after mutation",
        &tracker,
        vec!["read".to_string(), "edit".to_string(), "bash".to_string()],
        final_text2.to_string(),
    )
}

/// Fixture 6: blocked command recovery — guard denies, then allowed alternative or clear blocker.
pub fn fixture_blocked_command_recovery() -> EvalResult {
    run_fixture(
        "blocked command recovery",
        "remove temp file",
        vec![
            ("bash", json!({"command": "rm -rf /tmp/foo"}), "[bash-guard BLOCKED (HIGH): rm] Command: rm -rf /tmp/foo"),
        ],
        "Command was blocked by guard — need approval or use allowed alternative. Cannot proceed without permission.",
        |tracker, _| {
            if !tracker.has_blocker {
                return Some("should have recorded blocker".to_string());
            }
            if !tracker.tool_failed {
                return Some("should have recorded tool failure for blocked".to_string());
            }
            // With blocker, completion should be allowed (state-based)
            if !tracker.can_complete() {
                return Some("with blocker, can_complete should be true".to_string());
            }
            None
        },
    )
}

/// Fixture 7: malformed tool call recovery — invalid args, then retry with valid args.
pub fn fixture_malformed_tool_call_recovery() -> EvalResult {
    run_fixture(
        "malformed tool call recovery",
        "write config",
        vec![
            (
                "edit",
                json!({"path": "src/app.rs"}),
                "Error: malformed tool arguments: missing oldText",
            ),
            ("read", json!({"path": "src/app.rs"}), "current"),
            (
                "edit",
                json!({"path": "src/app.rs", "oldText": "current", "newText": "new"}),
                "ok",
            ),
            ("bash", json!({"command": "cargo check"}), "Finished"),
        ],
        "Fixed malformed args, retried correctly and verified. All done.",
        |tracker, final_text| {
            if !tracker.tool_failed {
                return Some("should have recorded malformed failure".to_string());
            }
            if !tracker.mutation_succeeded {
                return Some("retry should succeed".to_string());
            }
            if !tracker.looks_complete(final_text) {
                return Some("should be complete after corrected edit + verification".to_string());
            }
            None
        },
    )
}

/// Run all fixtures and return results (for display / CI).
pub fn run_all() -> Vec<EvalResult> {
    vec![
        fixture_simple_file_edit(),
        fixture_compile_error_fix(),
        fixture_failed_unique_edit_recovery(),
        fixture_multi_file_diagnosis(),
        fixture_verification_required_after_mutation(),
        fixture_blocked_command_recovery(),
        fixture_malformed_tool_call_recovery(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_evals_all_pass() {
        let results = run_all();
        for r in &results {
            assert!(
                r.passed,
                "fixture '{}' failed: {} (mode={:?}, tools={:?}, mutation={}/{}, verify={}/{}, text={:?})",
                r.fixture,
                r.failure_reason.as_deref().unwrap_or("unknown"),
                r.request_mode,
                r.tool_calls,
                r.mutations_attempted,
                r.mutations_succeeded,
                r.verification_attempted,
                r.verification_passed,
                r.final_text
            );
        }
    }

    #[test]
    fn eval_result_shape_has_required_fields() {
        let r = fixture_simple_file_edit();
        assert!(!r.fixture.is_empty());
        assert!(!r.request_mode.is_empty());
        assert!(!r.tool_calls.is_empty());
        assert!(r.verification_attempted);
        assert!(r.final_text.contains("All done"));
    }

    #[test]
    fn verification_gate_blocks_premature_completion() {
        let r = fixture_verification_required_after_mutation();
        assert!(r.passed, "{}", r.failure_reason.unwrap_or_default());
        assert!(r.verification_attempted);
        assert!(r.verification_passed);
    }
}
