//! Planner/Evaluator separation harness — GAN-inspired three-agent architecture.
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
pub mod planner;
pub mod runner;
pub mod types;

// Re-export core types used by the harness
pub use types::*;