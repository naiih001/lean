// Agent modes — opencode-like configurable modes.
//
// The active mode is a name (norm/plan/ask + customs from modes.json) with a
// gate behavior (Norm/Plan/Ask). Builtin semantics are unchanged: plan gates
// mutations behind the 5-phase flow, ask is read-only, auto is norm with
// approvals bypassed. Custom modes map onto one gate behavior.
//
// Modes are process-global EXCEPT under a thread-local override (used by
// subagents): a child thread snapshots the parent mode and any set_mode call
// inside the child only rewrites its own snapshot — a child can never flip
// the parent's mode.

use crate::core::mode_config::{self, GateBehavior, ResolvedMode};
use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Norm,
    Plan,
    Ask,
    Auto,
    Custom(String),
}

/// Effective mode: resolved config + approval bypass flag.
#[derive(Debug, Clone)]
pub struct EffectiveMode {
    pub resolved: ResolvedMode,
    pub auto: bool,
}

fn effective_cell() -> &'static Mutex<EffectiveMode> {
    static EFFECTIVE: OnceLock<Mutex<EffectiveMode>> = OnceLock::new();
    EFFECTIVE.get_or_init(|| {
        Mutex::new(EffectiveMode {
            resolved: mode_config::builtin_norm(),
            auto: false,
        })
    })
}

thread_local! {
    static OVERRIDE: RefCell<Option<EffectiveMode>> = const { RefCell::new(None) };
}

/// Snapshot the current effective mode (for subagent inheritance).
pub fn capture_effective() -> EffectiveMode {
    effective()
}

/// Run with `eff` as this thread's mode; restores on drop.
/// Guards must not nest — dropping an inner guard clears the override.
pub fn with_override(eff: EffectiveMode) -> ModeOverrideGuard {
    OVERRIDE.with(|o| *o.borrow_mut() = Some(eff));
    ModeOverrideGuard
}

pub struct ModeOverrideGuard;

impl Drop for ModeOverrideGuard {
    fn drop(&mut self) {
        OVERRIDE.with(|o| *o.borrow_mut() = None);
    }
}

fn override_active() -> bool {
    OVERRIDE.with(|o| o.borrow().is_some())
}

/// Crate-visible: does this thread hold a mode override? (guards/approval.rs)
pub(crate) fn override_active_for_write() -> bool {
    override_active()
}

/// Write the auto flag into this thread's override (no-op without one).
pub(crate) fn set_override_auto(v: bool) {
    OVERRIDE.with(|o| {
        if let Some(eff) = o.borrow_mut().as_mut() {
            eff.auto = v;
        }
    });
}

/// Auto flag visible to guards/approval.rs (override wins over the global).
pub(crate) fn override_auto() -> Option<bool> {
    OVERRIDE.with(|o| o.borrow().as_ref().map(|e| e.auto))
}

pub fn effective() -> EffectiveMode {
    if let Some(eff) = OVERRIDE.with(|o| o.borrow().clone()) {
        return eff;
    }
    effective_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn store_effective(eff: EffectiveMode) {
    if override_active() {
        OVERRIDE.with(|o| *o.borrow_mut() = Some(eff));
    } else {
        *effective_cell().lock().unwrap_or_else(|e| e.into_inner()) = eff;
    }
}

fn apply_name(name: &str, auto: bool) -> bool {
    match mode_config::resolve(name) {
        Ok(resolved) => {
            store_effective(EffectiveMode { resolved, auto });
            crate::guards::approval::set_auto_accept_raw(auto);
            true
        }
        Err(e) => {
            eprintln!("[modes] {}", e);
            false
        }
    }
}

/// Lowercase effective mode name ("norm", "plan", "review", ...).
pub fn mode_name() -> String {
    effective().resolved.name.clone()
}

/// Display name for badges (capitalized).
pub fn mode_display() -> String {
    let name = mode_name();
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
        None => name,
    }
}

pub fn gate_behavior() -> GateBehavior {
    effective().resolved.behavior
}

pub fn current_mode() -> Mode {
    let eff = effective();
    let name = eff.resolved.name.as_str();
    // Auto badge wins only for norm-behavior modes (today: norm+auto).
    if eff.auto && eff.resolved.behavior == GateBehavior::Norm {
        if name == "norm" {
            return Mode::Auto;
        }
        return Mode::Custom(eff.resolved.name.clone());
    }
    match name {
        "norm" => Mode::Norm,
        "plan" => Mode::Plan,
        "ask" => Mode::Ask,
        _ => Mode::Custom(eff.resolved.name.clone()),
    }
}

pub fn set_mode(m: Mode) {
    match m {
        Mode::Norm => {
            apply_name("norm", false);
        }
        Mode::Plan => {
            apply_name("plan", false);
        }
        Mode::Ask => {
            apply_name("ask", false);
        }
        Mode::Auto => {
            // Today's semantics: auto is norm + bypass (leaves plan/ask).
            apply_name("norm", true);
        }
        Mode::Custom(name) => {
            apply_name(&name, false);
        }
    }
}

/// Set by name (for /mode). Returns false when the name is unknown.
pub fn set_mode_by_name(name: &str) -> bool {
    let key = name.trim().to_lowercase();
    if key == "auto" {
        set_mode(Mode::Auto);
        return true;
    }
    apply_name(&key, false)
}

pub fn cycle_mode() -> Mode {
    // norm → plan → ask → customs… → auto → norm
    let mut order = mode_config::mode_names();
    order.push("auto".to_string());
    let eff = effective();
    let cur_key = if eff.auto && eff.resolved.name == "norm" {
        "auto".to_string()
    } else {
        eff.resolved.name.clone()
    };
    let next = order
        .iter()
        .position(|n| *n == cur_key)
        .map(|i| order[(i + 1) % order.len()].clone())
        .unwrap_or_else(|| "norm".to_string());
    if next == "auto" {
        set_mode(Mode::Auto);
    } else {
        set_mode_by_name(&next);
    }
    current_mode()
}

pub fn set_plan_mode(v: bool) {
    if v {
        if gate_behavior() != GateBehavior::Plan {
            let auto = effective().auto;
            apply_name("plan", auto);
        }
    } else if mode_name() == "plan" {
        let auto = effective().auto;
        apply_name("norm", auto);
    }
}

pub fn is_plan_mode() -> bool {
    gate_behavior() == GateBehavior::Plan
}

pub fn set_ask_mode(v: bool) {
    if v {
        if gate_behavior() != GateBehavior::Ask {
            let auto = effective().auto;
            apply_name("ask", auto);
        }
    } else if mode_name() == "ask" {
        let auto = effective().auto;
        apply_name("norm", auto);
    }
}

pub fn is_ask_mode() -> bool {
    gate_behavior() == GateBehavior::Ask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_cycle_order() {
        // mode_names starts with the builtins even without config files.
        let names = mode_config::mode_names();
        assert!(names.starts_with(&["norm".to_string(), "plan".to_string(), "ask".to_string()]));
    }

    #[test]
    fn display_capitalizes() {
        let eff = EffectiveMode {
            resolved: mode_config::builtin_norm(),
            auto: false,
        };
        let _guard = with_override(eff);
        assert_eq!(mode_name(), "norm");
        assert_eq!(mode_display(), "Norm");
    }

    #[test]
    fn override_is_thread_local() {
        let eff = EffectiveMode {
            resolved: mode_config::builtin_norm(),
            auto: true,
        };
        {
            let _guard = with_override(eff);
            assert_eq!(override_auto(), Some(true));
        }
        assert_eq!(override_auto(), None);
    }
}
