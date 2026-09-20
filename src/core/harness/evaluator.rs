//! Evaluator module — grades work against a rubric WITHOUT seeing the plan.
//!
//! This is the critical separation: the Evaluator receives the rubric and
//! the generated output, but **never** the plan. This prevents the
//! generator from gaming the evaluator by tailoring its output to match
//! the plan's structure rather than doing genuinely good work.
//!
//! The Evaluator is LLM-powered and uses a structured rubric to produce
//! dimension-by-dimension scores.

use crate::core::harness::types::{Evaluation, Issue, IssueSeverity, Rubric, HarnessConfig};
use crate::integrations::llm::Client;
use serde_json::{json, Value};

/// Evaluate generated work against a rubric.
///
/// **Critical invariant**: `work_output` must NOT contain the plan.
/// The evaluator function itself enforces this — it never receives
/// plan data, only the rubric and the work product.
pub async fn evaluate(
    work_output: &str,
    rubric: &Rubric,
    config: &HarnessConfig,
    client: &Client,
) -> anyhow::Result<Evaluation> {
    let system_prompt = build_evaluator_prompt();
    let user_prompt = build_eval_user_prompt(work_output, rubric);

    let messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_prompt}),
    ];

    let response = client
        .chat(&config.evaluator_model, &messages, 0.0, Some(2000))
        .await?;

    let eval = parse_evaluation_response(&response)?;
    Ok(eval)
}

/// Build the system prompt for the Evaluator.
///
/// The prompt explicitly states that the evaluator must NOT see the plan.
fn build_evaluator_prompt() -> String {
    r#"You are an Evaluator. Grade the generated work against the rubric below.

CRITICAL RULES:
1. You do NOT have access to the plan. Do not ask about it. Do not use plan details.
2. Grade ONLY against the rubric dimensions and the work output provided.
3. Be objective. Use the acceptance criteria as evidence.
4. Score each dimension from 0.0 to 1.0.
5. Provide constructive feedback for each dimension.
6. Identify specific issues with severity (critical/major/minor).

Return a JSON object:
{
  "overall_score": 0.85,
  "dimension_scores": {"feature_name": 0.9, "correctness": 0.8, "safety": 1.0},
  "passed": true,
  "feedback": "Summary of evaluation results",
  "issues": [
    {"dimension": "feature_name", "description": "Specific problem", "severity": "minor"}
  ]
}"#.to_string()
}

/// Build the user prompt containing the work output and rubric.
fn build_eval_user_prompt(work_output: &str, rubric: &Rubric) -> String {
    let rubric_text = rubric
        .dimensions
        .iter()
        .map(|d| format!("- {} (weight {:.2}): {}", d.name, d.weight, d.description))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Work Output:\n\n---\n{}\n---\n\nRubric:\n\n{}\n\nGrade the work against each dimension. Be objective and evidence-driven.",
        work_output, rubric_text
    )
}

/// Parse the LLM's evaluation response into an Evaluation struct.
pub fn parse_evaluation_response(response: &str) -> anyhow::Result<Evaluation> {
    let json_str = extract_json(response);
    let value: Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse eval response: {}", e))?;

    let overall_score = value
        .get("overall_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;

    let dimension_scores = value
        .get("dimension_scores")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| {
                    let score = v.as_f64().unwrap_or(0.0) as f32;
                    (k.clone(), score)
                })
                .collect()
        })
        .unwrap_or_default();

    let passed = value
        .get("passed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let feedback = value
        .get("feedback")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let issues = value
        .get("issues")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let obj = item.as_object()?;
                    let dimension = obj.get("dimension")?.as_str()?.to_string();
                    let description = obj.get("description")?.as_str()?.to_string();
                    let severity_str = obj.get("severity")?.as_str()?;
                    let severity = match severity_str {
                        "critical" => IssueSeverity::Critical,
                        "major" => IssueSeverity::Major,
                        _ => IssueSeverity::Minor,
                    };
                    Some(Issue {
                        dimension,
                        description,
                        severity,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Evaluation {
        overall_score,
        dimension_scores,
        passed,
        feedback,
        issues,
    })
}

/// Extract JSON from an LLM response, handling markdown code blocks.
pub fn extract_json(response: &str) -> String {
    if let Some(start) = response.find("```json") {
        let after = &response[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = response.find("```") {
        let after = &response[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    response.trim().to_string()
}

/// Build the feedback to pass back to the Generator.
///
/// This feedback is plan-agnostic — it tells the generator what to fix
/// without revealing the plan details.
pub fn build_generator_feedback(evaluation: &Evaluation) -> String {
    let mut feedback = String::new();
    feedback.push_str("Evaluating your work against the rubric:\n\n");

    if !evaluation.issues.is_empty() {
        feedback.push_str("Issues found:\n");
        for issue in &evaluation.issues {
            feedback.push_str(&format!(
                "  [{}] {}: {}\n",
                match issue.severity {
                    IssueSeverity::Critical => "CRITICAL",
                    IssueSeverity::Major => "MAJOR",
                    IssueSeverity::Minor => "MINOR",
                },
                issue.dimension,
                issue.description
            ));
        }
        feedback.push('\n');
    }

    feedback.push_str(&format!(
        "Overall score: {:.2}\n\n",
        evaluation.overall_score
    ));

    if evaluation.passed {
        feedback.push_str("Work passed the rubric threshold.\n");
    } else {
        feedback.push_str("Work did not pass the rubric threshold. Address the issues above and try again.\n");
    }

    feedback.push_str("\nFeedback:\n");
    feedback.push_str(&evaluation.feedback);

    feedback
}

/// Build the feedback to pass back to the Planner.
///
/// This feedback helps the planner adjust future plans based on
/// what the evaluator found about the work quality.
pub fn build_planner_feedback(evaluation: &Evaluation, plan_features: &[crate::core::harness::types::Feature]) -> String {
    let mut feedback = String::new();
    feedback.push_str("Plan Evaluation Summary:\n\n");

    feedback.push_str(&format!(
        "Overall score: {:.2}/{:.2}\n\n",
        evaluation.overall_score,
        1.0
    ));

    feedback.push_str("Per-feature assessment:\n");
    for feature in plan_features {
        let score = evaluation.dimension_scores.get(&feature.name).copied().unwrap_or(0.0);
        feedback.push_str(&format!("  {}: {:.2}\n", feature.name, score));
    }

    if !evaluation.issues.is_empty() {
        feedback.push_str("\nAreas needing improvement:\n");
        for issue in &evaluation.issues {
            let severity_str = match issue.severity {
                IssueSeverity::Critical => "critical",
                IssueSeverity::Major => "major",
                IssueSeverity::Minor => "minor",
            };
            feedback.push_str(&format!("  - {} ({}): {}\n", issue.dimension, severity_str, issue.description));
        }
    }

    feedback
}