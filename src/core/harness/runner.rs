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

use crate::core::agent::AgentEvent;
use crate::core::harness::evaluator::{build_generator_feedback, build_planner_feedback, evaluate};
use crate::core::harness::generator::{is_complex_task, plan_summary, transcript_for_eval};
use crate::core::harness::planner;
use crate::core::harness::types::{Evaluation, HarnessConfig, Plan, SprintResult};
use crate::integrations::llm::Client;

/// Harness-core streaming loop: Planner → Generator → Evaluator sprints.
///
/// Flag-gated / blocking / complex-only: conversational goals fall through to
/// a single legacy pass with no planner/eval overhead. Complex tasks plan
/// once, then run the legacy tool loop as the `Generator` step per sprint,
/// grade the transcript (plan-blind) and retry with feedback until pass or
/// budget exhaustion. `AgentEvent` shapes are unchanged so TUI/subagent
/// consumers only add a `Sprint` status arm.
pub fn run_harness_stream(
    task: String,
    model: String,
    max_steps: usize,
    history: Vec<serde_json::Value>,
    config: HarnessConfig,
) -> impl futures::Stream<Item = AgentEvent> {
    async_stream::stream! {
        // Defense in depth: complex-only gate holds even if the caller forgets it.
        if !is_complex_task(&task) {
            let mut s = Box::pin(crate::core::agent::legacy_run_agent_with_history(
                task, model, max_steps, history,
            ));
            use futures::StreamExt;
            while let Some(ev) = s.next().await {
                yield ev;
            }
            return;
        }
        let resolved = match crate::integrations::models::resolve(Some(&model)) {
            Ok(r) => r,
            Err(e) => {
                yield AgentEvent::Text { delta: format!("\n[model resolve error: {}]", e) };
                yield AgentEvent::Done { text: String::new(), history: Vec::new() };
                return;
            }
        };
        let client = Client::from_resolved(&resolved);
        // Planner pass — on failure fall back to a single legacy pass.
        let plan = match planner::plan_task(&task, &config, &client).await {
            Ok(p) => p,
            Err(e) => {
                yield AgentEvent::Text { delta: format!("\n[harness: planner failed ({}), single-pass fallback]", e) };
                let mut s = Box::pin(crate::core::agent::legacy_run_agent_with_history(
                    task, model, max_steps, history,
                ));
                use futures::StreamExt;
                while let Some(ev) = s.next().await {
                    yield ev;
                }
                return;
            }
        };
        let rubric = planner::build_rubric(&plan);
        let summary = plan_summary(&plan);
        let sprints = config.max_sprints.max(1);
        // Bound worst-case tool steps across blocking retries.
        let per_sprint_steps = (max_steps / sprints).max(15);
        let mut generator_feedback = String::new();
        let mut carry_history = history;
        let mut best_text = String::new();
        let mut best_history: Vec<serde_json::Value> = Vec::new();
        let mut best_score: f32 = -1.0;
        let mut best_passed = false;
        let mut completed = 0usize;
        for sprint in 1..=sprints {
            yield AgentEvent::Step { n: sprint };
            let sprint_prompt = if generator_feedback.is_empty() {
                format!("{}\n\n[Plan context:\n{}]", task, summary)
            } else {
                format!(
                    "{}\n\n[Plan context:\n{}]\n\n[Feedback from sprint {} eval — address before finishing:\n{}]",
                    task, summary, sprint - 1, generator_feedback
                )
            };
            // Generator step: legacy single-pass tool loop.
            let mut s = Box::pin(crate::core::agent::legacy_run_agent_with_history(
                sprint_prompt,
                model.clone(),
                per_sprint_steps,
                carry_history.clone(),
            ));
            let mut final_text = String::new();
            let mut new_history: Vec<serde_json::Value> = Vec::new();
            use futures::StreamExt;
            while let Some(ev) = s.next().await {
                match ev {
                    AgentEvent::Done { text, history } => {
                        final_text = text;
                        new_history = history;
                    }
                    other => yield other,
                }
            }
            completed = sprint;
            let transcript = transcript_for_eval(&new_history, &final_text);
            if transcript.trim().is_empty() {
                // Nothing gradable (e.g. immediate conversational stop) — keep it.
                best_text = final_text;
                best_history = new_history;
                best_score = best_score.max(0.0);
                yield AgentEvent::Sprint { n: sprint, score: 0.0, passed: true };
                break;
            }
            // Evaluator step (plan-blind by signature). Eval failure is
            // pass-through: keep the best attempt, don't error the turn.
            let eval = match evaluate(&transcript, &rubric, &config, &client).await {
                Ok(e) => e,
                Err(e) => {
                    yield AgentEvent::Text { delta: format!("\n[harness: eval failed ({}), keeping best attempt]", e) };
                    best_text = final_text;
                    best_history = new_history;
                    best_score = best_score.max(0.0);
                    yield AgentEvent::Sprint { n: sprint, score: 0.0, passed: true };
                    break;
                }
            };
            let score = eval.overall_score;
            let passed = eval.passed;
            if score > best_score {
                best_score = score;
                best_passed = passed;
                best_text = final_text.clone();
                best_history = new_history.clone();
            }
            carry_history = new_history;
            yield AgentEvent::Sprint { n: sprint, score, passed };
            if passed && score >= config.min_score && config.early_termination {
                let _planner_fb = build_planner_feedback(&eval, &plan.features);
                break;
            }
            generator_feedback = build_generator_feedback(&eval);
        }
        if completed == sprints && (!best_passed || best_score < config.min_score) && best_score >= 0.0 {
            yield AgentEvent::Text { delta: format!("\n[harness: best of {} sprints, score {:.2}]", completed, best_score) };
        }
        yield AgentEvent::Done { text: best_text, history: best_history };
    }
}

/// Run the full sprint cycle: Plan → Generate → Evaluate → Repeat.
///
/// Non-streaming batch variant. Interactive turns use `run_harness_stream`
/// (the harness core); this shares planner/evaluator but grades without
/// tool execution — prefer the streaming path for agent turns.
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
        let work_output = plan
            .features
            .iter()
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
        .map(|f| {
            format!(
                "### {}\n{}\nAcceptance: {:?}",
                f.name, f.description, f.acceptance_criteria
            )
        })
        .collect();

    format!(
        "Implement the following plan features:\n\n{}\n\nReturn the complete implementation.",
        features.join("\n\n")
    )
}
