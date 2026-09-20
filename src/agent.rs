// Shim for backward compat — single responsibility refactor (C1)
// This file re-exports the split core modules and keeps old crate::agent paths alive.
// New code should use crate::core::agent / ::prompts / ::modes / ::tracker / ::history.
// TODO(HARNESS-REPLACE-LOOP): `run_agent*` must delegate to
// `crate::core::harness::runner` — the harness replaces the current agent loop.
pub use crate::core::agent::{run_agent, run_agent_with_history, AgentEvent};
pub use crate::core::modes::{
    capture_effective, current_mode, cycle_mode, effective, gate_behavior, is_ask_mode,
    is_plan_mode, mode_display, mode_name, set_ask_mode, set_mode, set_mode_by_name, set_plan_mode,
    with_override, EffectiveMode, Mode,
};
pub use crate::core::prompts::{
    build_system_prompt, ASK_READONLY_DENY_MSG, ASK_SYSTEM_PROMPT, PLAN_SYSTEM_PROMPT,
    REGULAR_SYSTEM_PROMPT, SYSTEM_PROMPT,
};
pub use crate::core::tracker::PlanTracker;
