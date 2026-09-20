//! Planner module — decomposes tasks into structured feature lists.
//!
//! The Planner is an LLM-powered component that takes a task description
//! and produces a structured `Plan` with atomic features, each having
//! acceptance criteria. This follows Anthropic's approach where the
//! Planner decomposes a task into a feature list.

use crate::core::harness::types::{HarnessConfig, Plan, Rubric};
use crate::integrations::llm::Client;
use serde_json::{json, Value};

/// Build a rubric for the Evaluator based on the plan structure.
///
/// The rubric is derived from the plan's features but is **independent**
/// of the plan details — it measures work against objective criteria.
pub fn build_rubric(plan: &Plan) -> Rubric {
    let mut dimensions: Vec<_> = plan
        .features
        .iter()
        .map(|f| crate::core::harness::types::Dimension {
            name: format!("{}", f.name),
            description: format!(
                "Implementation of {} — meets all acceptance criteria",
                f.name
            ),
            weight: if plan.features.is_empty() {
                1.0
            } else {
                1.0 / plan.features.len() as f32
            },
        })
        .collect();

    // Add general quality dimensions
    dimensions.push(crate::core::harness::types::Dimension {
        name: "correctness".to_string(),
        description: "Code compiles and passes tests".to_string(),
        weight: 0.15,
    });
    dimensions.push(crate::core::harness::types::Dimension {
        name: "safety".to_string(),
        description: "No allowlist violations or security issues".to_string(),
        weight: 0.1,
    });

    // Normalize weights
    let total: f32 = dimensions.iter().map(|d| d.weight).sum();
    let dimensions: Vec<_> = dimensions
        .into_iter()
        .map(|mut d| {
            d.weight = d.weight / total;
            d
        })
        .collect();

    Rubric {
        dimensions,
        pass_threshold: 0.7,
    }
}

/// Create an initial plan from a task description using the Planner LLM.
pub async fn plan_task(
    task: &str,
    config: &HarnessConfig,
    client: &Client,
) -> anyhow::Result<Plan> {
    let system_prompt = build_planner_prompt();
    let user_prompt = format!("Task: {}\n\nDecompose this into a structured plan with atomic features.", task);

    let messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_prompt}),
    ];

    let response = client
        .chat(&config.planner_model, &messages, 0.1, Some(2000))
        .await?;

    let plan_content = extract_json_from_response(&response);
    let plan: Plan = serde_json::from_str(&plan_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse plan: {}", e))?;

    Ok(plan)
}

/// Build the system prompt for the Planner.
fn build_planner_prompt() -> String {
    r#"You are a Planner. Decompose tasks into atomic, testable features.

Return a JSON object with this structure:
{
  "id": "unique-plan-id",
  "title": "Short descriptive title",
  "features": [
    {
      "name": "feature-name",
      "description": "Detailed description of what this feature does",
      "acceptance_criteria": ["criterion 1", "criterion 2"],
      "priority": 1,
      "affected_files": ["src/path/to/file.rs"]
    }
  ],
  "metadata": {
    "feature_count": 3,
    "complexity": 5,
    "notes": "Any constraints or risks"
  }
}

Rules:
- Each feature must be independently testable
- Acceptance criteria must be objective and verifiable
- Keep features small — one responsibility each
- Include affected file paths where relevant"#.to_string()
}

/// Extract JSON from an LLM response, handling markdown code blocks.
pub fn extract_json_from_response(response: &str) -> String {
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