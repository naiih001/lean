// herdr integration — mirrors pi/opencode `herdr-agent-state` but in Rust
// managed by lean; reinstalling herdr integration overwrites the *other* agents' shims, not this file.
// HERDR_INTEGRATION_ID=lean
// HERDR_INTEGRATION_VERSION=1

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE: &str = "herdr:lean";
const AGENT: &str = "lean";

static REPORT_SEQ: OnceLock<AtomicU64> = OnceLock::new();
static LAST_STATE: OnceLock<Mutex<Option<(String, Option<String>)>>> = OnceLock::new();
static SEND_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static QUEUED: OnceLock<Mutex<Option<QueuedState>>> = OnceLock::new();
static CURRENT_SESSION_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static CURRENT_SESSION_PATH: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct QueuedState {
    state: String,
    message: Option<String>,
    seq: u64,
    session_id: Option<String>,
    session_path: Option<String>,
}

fn seq_cell() -> &'static AtomicU64 {
    REPORT_SEQ.get_or_init(|| {
        let base = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            * 1000;
        AtomicU64::new(base)
    })
}

fn next_seq() -> u64 {
    seq_cell().fetch_add(1, Ordering::Relaxed) + 1
}

fn last_state_cell() -> &'static Mutex<Option<(String, Option<String>)>> {
    LAST_STATE.get_or_init(|| Mutex::new(None))
}

fn queued_cell() -> &'static Mutex<Option<QueuedState>> {
    QUEUED.get_or_init(|| Mutex::new(None))
}

fn current_id_cell() -> &'static Mutex<Option<String>> {
    CURRENT_SESSION_ID.get_or_init(|| Mutex::new(None))
}

fn current_path_cell() -> &'static Mutex<Option<String>> {
    CURRENT_SESSION_PATH.get_or_init(|| Mutex::new(None))
}

fn enabled() -> bool {
    std::env::var("HERDR_ENV")
        .map(|v| v == "1")
        .unwrap_or(false)
        && std::env::var("HERDR_PANE_ID")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        && std::env::var("HERDR_SOCKET_PATH")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
}

fn pane_id() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

fn socket_path() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("HERDR_SOCKET_PATH")
        .ok()
        .filter(|s| !s.is_empty())
}

fn request_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rand: u32 = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        ^ (std::process::id().wrapping_mul(0x9e3779b1)))
        % 1_000_000;
    format!("{}:{}:{:06}", SOURCE, millis, rand)
}

fn update_session_ref(id: Option<&str>, path: Option<&str>) {
    if let Some(v) = id {
        *current_id_cell().lock().unwrap() = Some(v.to_string());
    }
    if let Some(v) = path {
        *current_path_cell().lock().unwrap() = Some(v.to_string());
    }
}

fn current_session_ref() -> (Option<String>, Option<String>) {
    let id = current_id_cell().lock().unwrap().clone();
    let path = current_path_cell().lock().unwrap().clone();
    (id, path)
}

// Fire-and-forget socket write — matches pi's sendRequestAttempt with 500ms + 1500ms retry.
// We spawn a tokio task so the TUI loop is never blocked.
async fn send_once(request_json: String, timeout_ms: u64) -> bool {
    let sock = match socket_path() {
        Some(s) => s,
        None => return false,
    };
    let stream = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::net::UnixStream::connect(&sock),
    )
    .await
    {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    // Use the stream as async Read+Write
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;
    let payload = format!("{}\n", request_json);
    if tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        stream.write_all(payload.as_bytes()),
    )
    .await
    .is_err()
    {
        return false;
    }
    if stream.flush().await.is_err() {
        return false;
    }
    // Wait for any response byte (herdr sends JSON ack), or timeout. Even if no data, we delivered.
    let mut buf = [0u8; 4096];
    match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        stream.read(&mut buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => true,
        Ok(Ok(_)) => true, // connected + wrote, treat as delivered even if empty
        Ok(Err(_)) => false,
        Err(_) => true, // timeout after write = likely delivered, pi treats this as success on second attempt
    }
}

async fn send_request(value: serde_json::Value) {
    let json = serde_json::to_string(&value).unwrap_or_default();
    if json.is_empty() {
        return;
    }
    if send_once(json.clone(), 500).await {
        return;
    }
    let _ = send_once(json, 1500).await;
}

fn spawn_send(value: serde_json::Value) {
    // If we're inside a tokio runtime, spawn async; otherwise fallback to blocking thread.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            send_request(value).await;
        });
    } else {
        // Fallback: blocking UnixStream in a thread (for non-tokio contexts like tests)
        std::thread::spawn(move || {
            let json = serde_json::to_string(&value).unwrap_or_default();
            if json.is_empty() || pane_id().is_none() || socket_path().is_none() {
                return;
            }
            if let Some(p) = socket_path() {
                if let Ok(mut s) = std::os::unix::net::UnixStream::connect(&p) {
                    let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                    let _ = s.set_write_timeout(Some(std::time::Duration::from_millis(500)));
                    let payload = format!("{}\n", json);
                    let _ = std::io::Write::write_all(&mut s, payload.as_bytes());
                    let mut buf = [0u8; 1024];
                    let _ = std::io::Read::read(&mut s, &mut buf);
                }
            }
        });
    }
}

fn build_session_request(
    session_id: Option<&str>,
    session_path: Option<&str>,
    start_source: Option<&str>,
) -> Option<serde_json::Value> {
    let pane = pane_id()?;
    let seq = next_seq();
    let mut params = serde_json::json!({
        "pane_id": pane,
        "source": SOURCE,
        "agent": AGENT,
        "seq": seq,
    });
    if let Some(id) = session_id {
        params["agent_session_id"] = serde_json::Value::String(id.to_string());
    }
    if let Some(p) = session_path {
        params["agent_session_path"] = serde_json::Value::String(p.to_string());
    }
    if let Some(s) = start_source {
        params["session_start_source"] = serde_json::Value::String(s.to_string());
    }
    // Must have at least one session ref
    if session_id.is_none() && session_path.is_none() {
        return None;
    }
    Some(serde_json::json!({
        "id": request_id(),
        "method": "pane.report_agent_session",
        "params": params
    }))
}

fn build_state_request(
    state: &str,
    message: Option<&str>,
    seq: u64,
    session_id: Option<String>,
    session_path: Option<String>,
) -> Option<serde_json::Value> {
    let pane = pane_id()?;
    let mut params = serde_json::json!({
        "pane_id": pane,
        "source": SOURCE,
        "agent": AGENT,
        "state": state,
        "seq": seq,
    });
    if let Some(m) = message {
        if !m.is_empty() {
            params["message"] = serde_json::Value::String(m.to_string());
        }
    }
    if let Some(id) = session_id {
        params["agent_session_id"] = serde_json::Value::String(id);
    } else if let Some(path) = session_path {
        params["agent_session_path"] = serde_json::Value::String(path);
    } else {
        // fall back to global current ref
        let (cid, cpath) = current_session_ref();
        if let Some(cid) = cid {
            params["agent_session_id"] = serde_json::Value::String(cid);
        } else if let Some(cpath) = cpath {
            params["agent_session_path"] = serde_json::Value::String(cpath);
        }
    }
    Some(serde_json::json!({
        "id": request_id(),
        "method": "pane.report_agent",
        "params": params
    }))
}

// Public API — mirrors pi's queueState/drainStateQueue with dedup

fn queue_state(state: &str, message: Option<String>) {
    if !enabled() {
        return;
    }
    let (sid, spath) = current_session_ref();
    let seq = next_seq();
    let mut q = queued_cell().lock().unwrap();
    *q = Some(QueuedState {
        state: state.to_string(),
        message,
        seq,
        session_id: sid,
        session_path: spath,
    });
    // Try to drain if not already in-flight
    if !SEND_IN_FLIGHT.load(Ordering::Relaxed) {
        drop(q);
        drain_queue();
    }
}

fn drain_queue() {
    if SEND_IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    // Spawn the drain loop — fire-and-forget, keeps ordering like pi
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            loop {
                let next = { queued_cell().lock().unwrap().take() };
                let Some(item) = next else { break };
                if let Some(req) = build_state_request(
                    &item.state,
                    item.message.as_deref(),
                    item.seq,
                    item.session_id.clone(),
                    item.session_path.clone(),
                ) {
                    send_request(req).await;
                }
            }
            SEND_IN_FLIGHT.store(false, Ordering::Relaxed);
            // If something was queued while we were sending, drain again
            if queued_cell().lock().unwrap().is_some() {
                drain_queue();
            }
        });
    } else {
        // No runtime — send synchronously in a thread
        std::thread::spawn(|| {
            loop {
                let next = { queued_cell().lock().unwrap().take() };
                let Some(item) = next else { break };
                if let Some(req) = build_state_request(
                    &item.state,
                    item.message.as_deref(),
                    item.seq,
                    item.session_id.clone(),
                    item.session_path.clone(),
                ) {
                    let json = serde_json::to_string(&req).unwrap_or_default();
                    if let Some(p) = socket_path() {
                        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(&p) {
                            let _ =
                                s.set_write_timeout(Some(std::time::Duration::from_millis(500)));
                            let payload = format!("{}\n", json);
                            let _ = std::io::Write::write_all(&mut s, payload.as_bytes());
                        }
                    }
                }
            }
            SEND_IN_FLIGHT.store(false, Ordering::Relaxed);
        });
    }
}

fn publish_state(state: &str, message: Option<String>, force: bool) {
    if !enabled() {
        return;
    }
    {
        let mut last = last_state_cell().lock().unwrap();
        if !force {
            if let Some((ls, lm)) = last.as_ref() {
                if ls == state && lm == &message {
                    return;
                }
            }
        }
        *last = Some((state.to_string(), message.clone()));
    }
    queue_state(state, message);
}

// ── Public helpers called from TUI/app_loop ────────────────────────

/// Report a session — call on startup, resume, /new, and whenever Session::new() creates a file.
/// `start_source` mirrors hermes: "startup" | "resume" | "new" | "continue"
pub fn report_session(session_id: &str, session_path: &Path, start_source: Option<&str>) {
    if !enabled() || session_id.is_empty() {
        return;
    }
    let path_str = session_path.display().to_string();
    update_session_ref(Some(session_id), Some(&path_str));
    if let Some(req) = build_session_request(Some(session_id), Some(&path_str), start_source) {
        spawn_send(req);
    }
    // Also ensure last state is published with this session ref (idle by default)
    // Don't force — app_loop will explicitly publish idle/working after
}

/// Convenience: report session from &Session
pub fn report_session_obj(sess: &crate::session::Session, start_source: Option<&str>) {
    report_session(&sess.id, &sess.file_path(), start_source);
}

/// Update the global session ref without sending (e.g. when session file changes)
pub fn set_current_session(id: &str, path: &Path) {
    update_session_ref(Some(id), Some(&path.display().to_string()));
}

pub fn report_working() {
    publish_state("working", None, false);
}

pub fn report_working_with_msg(msg: &str) {
    publish_state("working", Some(msg.to_string()), false);
}

pub fn report_idle() {
    publish_state("idle", None, false);
}

pub fn report_blocked(msg: Option<&str>) {
    let m = msg.map(|s| s.to_string());
    publish_state("blocked", m, false);
}

/// Force publish — used on startup to ensure herdr sees us even if state == last
pub fn report_idle_force() {
    publish_state("idle", None, true);
}

pub fn report_working_force() {
    publish_state("working", None, true);
}

/// For tests / diagnostics
pub fn is_enabled() -> bool {
    enabled()
}
