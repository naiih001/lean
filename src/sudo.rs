use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct SudoRequest {
    pub cmd: String,
    pub tx: Option<oneshot::Sender<Option<String>>>,
}

static PENDING: OnceLock<Mutex<VecDeque<SudoRequest>>> = OnceLock::new();

fn pending_lock() -> &'static Mutex<VecDeque<SudoRequest>> {
    PENDING.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub async fn request(cmd: String) -> Option<String> {
    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending_lock().lock().unwrap();
        lock.push_back(SudoRequest { cmd, tx: Some(tx) });
    }
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(v)) => v,
        _ => None,
    }
}

pub fn take_pending() -> Option<SudoRequest> {
    pending_lock().lock().unwrap().pop_front()
}

pub fn has_pending() -> bool {
    !pending_lock().lock().unwrap().is_empty()
}

pub fn queue_len() -> usize {
    pending_lock().lock().unwrap().len()
}
