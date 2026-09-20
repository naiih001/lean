//! Planner/Evaluator separation harness — GAN-inspired three-agent architecture.
//!
//! HARNESS-AS-CORE (flag-gated / blocking / complex-only):
//! `run_agent_with_history` in `core/agent.rs` routes here when the harness
//! flag is on and the goal is non-conversational. New orchestration belongs in
//! `harness::runner::run_harness_stream`; the legacy single-pass tool loop in
//! `core/agent.rs::legacy_run_agent_with_history` is the `Generator` step.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────┐     plan     ┌──────────┐     work      ┌────────────┐
//! │ Planner  │─────────────>│ Generator│───────────────>│ Evaluator  │
//! │ (LLM)    │              │(agent    │                │ (LLM, no   │
//! │          │<─────────────│  loop)   │<───────────────│  plan)     │
//! └──────────┘   feedback  └──────────┘   feedback     └────────────┘
//! ```
//!
//! The **Evaluator grades against a rubric without seeing the plan** —
//! the critical separation that prevents gaming (see Anthropic's
//! "Harness Design for Long-Running Application Development", March 2026).
//!
//! The **Generator** is lean's existing agent loop.
//! **Sprint cycles** orchestrate the three roles with iterative feedback.

pub mod evaluator;
pub mod generator;
pub mod planner;
pub mod runner;
pub mod types;

// Re-export core types used by the harness
pub use types::*;

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-global harness kill-switch (flag-gated rollout, default OFF).
/// Set via `--harness` CLI or `/harness on|off` in the TUI. Subagent threads
/// inherit the parent value through the existing mode-override snapshot only
/// insofar as they read this flag at spawn time — the flag itself is global.
static HARNESS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable or disable harness-core orchestration for new turns.
pub fn set_harness_enabled(v: bool) {
    HARNESS_ENABLED.store(v, Ordering::SeqCst);
}

/// Whether new agent turns should route through `runner::run_harness_stream`.
pub fn harness_enabled() -> bool {
    HARNESS_ENABLED.load(Ordering::SeqCst)
}
