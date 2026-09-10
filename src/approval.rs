use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};
use tokio::sync::oneshot;

static AUTO_ACCEPT: AtomicBool = AtomicBool::new(false);

pub fn is_auto_accept() -> bool {
    AUTO_ACCEPT.load(Ordering::Relaxed)
}

pub fn set_auto_accept(v: bool) {
    AUTO_ACCEPT.store(v, Ordering::Relaxed);
}

pub fn toggle_auto_accept() -> bool {
    let prev = AUTO_ACCEPT.fetch_xor(true, Ordering::Relaxed);
    !prev
}

#[derive(Debug)]
pub struct ApprovalRequest {
    pub cmd: String,
    pub severity: crate::bash_guard::Severity,
    pub reasons: Vec<String>,
    pub tx: Option<oneshot::Sender<bool>>,
}

static PENDING: OnceLock<Mutex<VecDeque<ApprovalRequest>>> = OnceLock::new();

fn pending_lock() -> &'static Mutex<VecDeque<ApprovalRequest>> {
    PENDING.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Called from tools::execute_tool (background tokio task) to request approval.
/// Returns true if approved, false if denied.
/// If no TUI is running (pending not consumed within 200ms), falls back to blocking behavior handled by caller.
pub async fn request(cmd: String, severity: crate::bash_guard::Severity, reasons: Vec<String>) -> bool {
    // Session auto-accept bypass — no queue, no prompt, silent
    if is_auto_accept() {
        return true;
    }
    // If bash guard disabled, auto-approve
    if crate::bash_guard::is_disabled() {
        return true;
    }
    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending_lock().lock().unwrap();
        lock.push_back(ApprovalRequest {
            cmd: cmd.clone(),
            severity,
            reasons,
            tx: Some(tx),
        });
    }
    // Wait for TUI to respond. Queue preserves FIFO; TUI pops front.
    match rx.await {
        Ok(v) => v,
        Err(_) => false, // channel dropped -> denied
    }
}

/// TUI side: take pending request if any (non-blocking) — FIFO
pub fn take_pending() -> Option<ApprovalRequest> {
    let mut lock = pending_lock().lock().unwrap();
    lock.pop_front()
}

/// TUI side: put back if user hasn't decided yet (e.g., keep showing) — push to front
pub fn put_back(req: ApprovalRequest) {
    let mut lock = pending_lock().lock().unwrap();
    lock.push_front(req);
}

/// Check if there's pending approval (for rendering)
pub fn has_pending() -> bool {
    let lock = pending_lock().lock().unwrap();
    !lock.is_empty()
}

/// Number of queued approvals (for "[1/N]" display)
pub fn queue_len() -> usize {
    let lock = pending_lock().lock().unwrap();
    lock.len()
}

/// Peek queue length without consuming — used to compute label before pop
pub fn pending_count() -> usize {
    queue_len()
}
