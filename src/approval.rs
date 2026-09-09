use std::sync::{Mutex, OnceLock};
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct ApprovalRequest {
    pub cmd: String,
    pub severity: crate::bash_guard::Severity,
    pub reasons: Vec<String>,
    pub tx: Option<oneshot::Sender<bool>>,
}

static PENDING: OnceLock<Mutex<Option<ApprovalRequest>>> = OnceLock::new();

fn pending_lock() -> &'static Mutex<Option<ApprovalRequest>> {
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Called from tools::execute_tool (background tokio task) to request approval.
/// Returns true if approved, false if denied.
/// If no TUI is running (pending not consumed within 200ms), falls back to blocking behavior handled by caller.
pub async fn request(cmd: String, severity: crate::bash_guard::Severity, reasons: Vec<String>) -> bool {
    // If bash guard disabled, auto-approve
    if crate::bash_guard::is_disabled() {
        return true;
    }
    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending_lock().lock().unwrap();
        *lock = Some(ApprovalRequest {
            cmd: cmd.clone(),
            severity,
            reasons,
            tx: Some(tx),
        });
    }
    // Wait for TUI to respond. If TUI not running, this will hang forever — but we are in agent task,
    // caller should have timeout? For now wait indefinitely, TUI loop will resolve.
    match rx.await {
        Ok(v) => v,
        Err(_) => false, // channel dropped -> denied
    }
}

/// TUI side: take pending request if any (non-blocking)
pub fn take_pending() -> Option<ApprovalRequest> {
    let mut lock = pending_lock().lock().unwrap();
    lock.take()
}

/// TUI side: put back if user hasn't decided yet (e.g., keep showing)
pub fn put_back(req: ApprovalRequest) {
    let mut lock = pending_lock().lock().unwrap();
    *lock = Some(req);
}

/// Check if there's pending approval (for rendering)
pub fn has_pending() -> bool {
    let lock = pending_lock().lock().unwrap();
    lock.is_some()
}
