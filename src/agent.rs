// Shim for backward compat — single responsibility refactor (C1)
// This file re-exports the split core modules and keeps old crate::agent paths alive.
// New code should use crate::core::agent / ::prompts / ::modes / ::tracker / ::history.
pub use crate::core::agent::{run_agent, run_agent_with_history, AgentEvent};
pub use crate::core::modes::{
    current_mode, cycle_mode, is_ask_mode, is_plan_mode, set_ask_mode, set_mode, set_plan_mode,
    Mode,
};
pub use crate::core::prompts::{
    build_system_prompt, ASK_READONLY_DENY_MSG, ASK_SYSTEM_PROMPT, PLAN_SYSTEM_PROMPT,
    REGULAR_SYSTEM_PROMPT, SYSTEM_PROMPT,
};
pub use crate::core::tracker::PlanTracker;
