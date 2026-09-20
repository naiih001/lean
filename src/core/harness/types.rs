use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A structured plan decomposed by the Planner into atomic features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Unique identifier for this plan.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Feature list — each item is an independent, testable chunk.
    pub features: Vec<Feature>,
    /// Metadata generated during planning.
    pub metadata: PlanMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feature {
    /// Short name for this feature.
    pub name: String,
    /// Detailed description of what this feature should do.
    pub description: String,
    /// Acceptance criteria — how to verify this feature works.
    pub acceptance_criteria: Vec<String>,
    /// Priority ordering (lower = earlier).
    pub priority: u32,
    /// Optional file paths this feature touches.
    pub affected_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanMetadata {
    /// Number of features in the plan.
    pub feature_count: usize,
    /// Estimated total complexity score (1-10).
    pub complexity: u8,
    /// Any notes from the planner about constraints or risks.
    pub notes: String,
}

/// The rubric the Evaluator uses to grade work.
/// The Evaluator has no access to `Plan` — only this rubric and the
/// generated output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rubric {
    /// Dimension names for scoring.
    pub dimensions: Vec<Dimension>,
    /// Minimum score to pass (0.0–1.0).
    pub pass_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    /// Dimension name (e.g., "correctness", "coverage").
    pub name: String,
    /// What this dimension measures.
    pub description: String,
    /// Weight (0.0–1.0, sums to 1.0).
    pub weight: f32,
}

/// The Evaluator's output — grades each dimension against the rubric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    /// Overall score (0.0–1.0).
    pub overall_score: f32,
    /// Per-dimension scores.
    pub dimension_scores: HashMap<String, f32>,
    /// Whether the work passed the rubric threshold.
    pub passed: bool,
    /// Detailed feedback for the Generator and Planner.
    pub feedback: String,
    /// Specific issues found, if any.
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Which dimension the issue relates to.
    pub dimension: String,
    /// Description of the problem.
    pub description: String,
    /// Severity.
    pub severity: IssueSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Critical,
    Major,
    Minor,
}

/// A single sprint cycle: Planner → Generator → Evaluator → feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintCycle {
    /// The plan for this cycle.
    pub plan: Plan,
    /// The rubric used for evaluation.
    pub rubric: Rubric,
    /// The evaluation result (populated after Generator runs).
    pub evaluation: Option<Evaluation>,
    /// Number of iterations in this sprint.
    pub iteration_count: usize,
}

/// Configuration for the harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    /// Maximum number of sprint cycles.
    pub max_sprints: usize,
    /// Minimum score to consider a sprint successful.
    pub min_score: f32,
    /// Whether to allow early termination if score exceeds threshold.
    pub early_termination: bool,
    /// Model to use for the Planner.
    pub planner_model: String,
    /// Model to use for the Evaluator.
    pub evaluator_model: String,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            max_sprints: 5,
            min_score: 0.7,
            early_termination: true,
            planner_model: crate::integrations::llm::DEFAULT_MODEL.to_string(),
            evaluator_model: crate::integrations::llm::DEFAULT_MODEL.to_string(),
        }
    }
}

/// The role a component plays in the sprint cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Planner,
    Generator,
    Evaluator,
}

/// Result of a single sprint iteration.
#[derive(Debug, Clone)]
pub struct SprintResult {
    /// The plan that was worked on.
    pub plan_id: String,
    /// The evaluation result for this iteration.
    pub evaluation: Evaluation,
    /// Whether the sprint cycle should continue.
    pub continue_sprint: bool,
    /// Feedback to pass back to the Planner.
    pub planner_feedback: String,
    /// Feedback to pass back to the Generator.
    pub generator_feedback: String,
    /// Total iterations used.
    pub iterations: usize,
}

/// Error type for harness operations.
pub type HarnessError = anyhow::Error;