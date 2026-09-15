use std::sync::atomic::{AtomicBool, Ordering};

static PLAN_MODE: AtomicBool = AtomicBool::new(false);
static ASK_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Norm,
    Plan,
    Ask,
    Auto,
}

pub fn current_mode() -> Mode {
    if is_plan_mode() {
        Mode::Plan
    } else if is_ask_mode() {
        Mode::Ask
    } else if crate::approval::is_auto_accept() {
        Mode::Auto
    } else {
        Mode::Norm
    }
}

pub fn set_mode(m: Mode) {
    match m {
        Mode::Norm => {
            set_plan_mode(false);
            set_ask_mode(false);
            crate::approval::set_auto_accept(false);
        }
        Mode::Plan => {
            set_plan_mode(true);
            set_ask_mode(false);
            crate::approval::set_auto_accept(false);
        }
        Mode::Ask => {
            set_plan_mode(false);
            set_ask_mode(true);
            crate::approval::set_auto_accept(false);
        }
        Mode::Auto => {
            set_plan_mode(false);
            set_ask_mode(false);
            crate::approval::set_auto_accept(true);
        }
    }
}

pub fn cycle_mode() -> Mode {
    let next = match current_mode() {
        Mode::Norm => Mode::Plan,
        Mode::Plan => Mode::Ask,
        Mode::Ask => Mode::Auto,
        Mode::Auto => Mode::Norm,
    };
    set_mode(next);
    next
}

pub fn set_plan_mode(v: bool) {
    PLAN_MODE.store(v, Ordering::Relaxed);
}
pub fn is_plan_mode() -> bool {
    PLAN_MODE.load(Ordering::Relaxed)
}
pub fn set_ask_mode(v: bool) {
    ASK_MODE.store(v, Ordering::Relaxed);
}
pub fn is_ask_mode() -> bool {
    ASK_MODE.load(Ordering::Relaxed)
}


