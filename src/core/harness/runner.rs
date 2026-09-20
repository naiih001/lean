//! Sprint cycle orchestration — Planner → Generator → Evaluator → feedback.
//!
//! The runner manages the sprint cycle, coordinating the three roles
//! and handling the feedback loop. Each sprint cycle:
//!
//! 1. **Planner** decomposes the task into a structured plan
//! 2. **Generator** (lean's existing agent loop) works on the plan
//! 3. **Evaluator** grades the work against the rubric (without seeing the plan)
//! 4. **Feedback** loops back to Planner and Generator if needed
//!
//! This implements Anthropic's "Harness Design for Long-Running
//! Application Development" approach where the Evaluator grades
//! work without seeing the plan.

use crate::core::harness::evaluator::{build_generator_feedback, build_planner_feedback, evaluate};
use crate::core::harness::planner;
use crate::core::harness::types::{Evaluation, HarnessConfig, Plan, SprintResult};
use crate::integrations::llm::Client;

/// Run the full sprint cycle: Plan → Generate → Evaluate → Repeat.
///
/// TODO(HARNESS-REPLACE-LOOP): this is the intended replacement for the
/// legacy tool loop in `core/agent.rs::run_agent_with_history`. Wiring goal
/// (opt-in `/harness`, advisory-only, complex tasks only): keep
/// `run_agent_with_history`'s signature, move its tool-loop body behind a
/// `Generator` step called from here, and stream `AgentEvent`s through.
///
/// Returns the final evaluation result after all iterations
/// or when the score exceeds the minimum threshold.
pub async fn run_sprint(
    task: &str,
    config: &HarnessConfig,
    client: &Client,
) -> anyhow::Result<SprintResult> {
    let mut iteration_count = 0;
    let plan = planner::plan_task(task, config, client).await?;
    let rubric = planner::build_rubric(&plan);
    let mut evaluation: Option<Evaluation> = None;

    while iteration_count < config.max_sprints {
        iteration_count += 1;

        // Evaluate the current work against the rubric
        let work_output = plan.features.iter()
            .map(|f| format!("{}: {}", f.name, f.description))
            .collect::<Vec<_>>()
            .join("\n");

        evaluation = Some(evaluate(&work_output, &rubric, config, client).await?);

        let eval = evaluation.as_ref().unwrap();
        if eval.passed && eval.overall_score >= config.min_score && config.early_termination {
            break;
        }

        // Get feedback for the next iteration
        let planner_fb = build_planner_feedback(eval, &plan.features);
        let generator_fb = build_generator_feedback(eval);

        // Update the plan based on planner feedback
        // (In a full implementation, this would re-plan based on feedback)
        eprintln!("[harness] Planner feedback: {}", planner_fb);
        eprintln!("[harness] Generator feedback: {}", generator_fb);
    }

    let eval = evaluation.ok_or_else(|| anyhow::anyhow!("No evaluation produced"))?;
    let planner_feedback = build_planner_feedback(&eval, &plan.features);
    let generator_feedback = build_generator_feedback(&eval);

    Ok(SprintResult {
        plan_id: plan.id,
        evaluation: eval.clone(),
        continue_sprint: !eval.passed || eval.overall_score < config.min_score,
        planner_feedback,
        generator_feedback,
        iterations: iteration_count,
    })
}

/// Run a single sprint step: generate work output for a plan.
///
/// This delegates to lean's existing agent loop for the actual
/// generation. The harness orchestrates the high-level flow.
pub async fn generate_work(
    plan: &Plan,
    config: &HarnessConfig,
    client: &Client,
) -> anyhow::Result<String> {
    let prompt = build_generation_prompt(plan);
    let messages: Vec<serde_json::Value> = vec![
        serde_json::json!({"role": "system", "content": prompt}),
        serde_json::json!({"role": "user", "content": "Generate code for this plan."}),
    ];

    let response = client
        .chat(&config.planner_model, &messages, 0.1, Some(4000))
        .await?;

    Ok(response)
}

/// Build the system prompt for the Generator (lean's existing agent loop).
fn build_generation_prompt(plan: &Plan) -> String {
    let features: Vec<String> = plan
        .features
        .iter()
        .map(|f| format!("### {}\n{}\nAcceptance: {:?}", f.name, f.description, f.acceptance_criteria))
        .collect();

    format!(
        "Implement the following plan features:\n\n{}\n\nReturn the complete implementation.",
        features.join("\n\n")
    )
}