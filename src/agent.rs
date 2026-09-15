// Shim for backward compat — single responsibility refactor (C1)
// This file re-exports the split core modules and keeps old crate::agent paths alive.
// New code should use crate::core::agent / ::prompts / ::modes / ::tracker / ::history.
pub use crate::core::agent::{run_agent, run_agent_with_history, AgentEvent};
pub(crate) use crate::core::history::{
    build_user_content, drop_orphaned_tool_outputs, history_slice_for_api, prune_context_messages,
    strip_images_for_non_vision, MAX_IMAGES_PER_TURN,
};
pub use crate::core::modes::{
    current_mode, cycle_mode, is_ask_mode, is_plan_mode, set_ask_mode, set_mode, set_plan_mode,
    Mode,
};
pub use crate::core::prompts::{
    build_system_prompt, ASK_READONLY_DENY_MSG, ASK_SYSTEM_PROMPT, PLAN_SYSTEM_PROMPT,
    REGULAR_SYSTEM_PROMPT, SYSTEM_PROMPT,
};
pub(crate) use crate::core::prompts::{
    truncate_chars, truncate_for_llm, truncate_str, truncate_to_bytes,
};
pub use crate::core::tracker::PlanTracker;
pub(crate) use crate::core::tracker::{
    is_conversational_str, is_mcp_read, is_mutating_tool, is_permission_to_leave_plan,
    is_plan_exempt_write, is_readonly_bash, is_stay_in_plan, MAX_NOCALL_STREAK,
};
