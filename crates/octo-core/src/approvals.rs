//! Open approval requests of running threads (Claude `confirm` mode).
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::oneshot;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PendingApproval {
    pub id: String,
    pub tool: String,
    pub summary: String,
}

const SUMMARY_MAX: usize = 200;

/// Tool name plus a short form of the input (command / file path / url), at most 200 chars.
pub fn summarize(tool: &str, input: &Value) -> String {
    let key = ["command", "file_path", "url", "pattern", "path", "query"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .filter(|s| !s.is_empty());
    let full = match key {
        Some(k) => format!("{tool}: {k}"),
        None => tool.to_string(),
    };
    full.chars().take(SUMMARY_MAX).collect()
}

struct Entry {
    approval: PendingApproval,
    tx: oneshot::Sender<bool>,
}

type Open = HashMap<String, Vec<Entry>>;

/// Per hub. Requests are denied on timeout, on `cancel_thread` and when the run ends.
pub struct ApprovalBroker {
    timeout: Duration,
    open: Arc<Mutex<Open>>,
}

fn lock(m: &Mutex<Open>) -> MutexGuard<'_, Open> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn take(open: &Mutex<Open>, thread: &str, id: &str) -> Option<Entry> {
    let mut g = lock(open);
    let list = g.get_mut(thread)?;
    let i = list.iter().position(|e| e.approval.id == id)?;
    let e = list.remove(i);
    if list.is_empty() {
        g.remove(thread);
    }
    Some(e)
}

impl ApprovalBroker {
    /// Another handle on the same open requests.
    pub fn clone_handle(&self) -> Self {
        Self {
            timeout: self.timeout,
            open: self.open.clone(),
        }
    }

    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            open: Arc::default(),
        }
    }

    /// Registers a request; the receiver yields the decision (`false` = deny). Must be
    /// called inside a tokio runtime (the timeout runs as a task).
    pub fn request(
        &self,
        thread: &str,
        tool: &str,
        input: &Value,
    ) -> (PendingApproval, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let approval = PendingApproval {
            id: uuid::Uuid::new_v4().to_string(),
            tool: tool.to_string(),
            summary: summarize(tool, input),
        };
        lock(&self.open)
            .entry(thread.to_string())
            .or_default()
            .push(Entry {
                approval: approval.clone(),
                tx,
            });
        let open = self.open.clone();
        let (thread, id, timeout) = (thread.to_string(), approval.id.clone(), self.timeout);
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if let Some(e) = take(&open, &thread, &id) {
                let _ = e.tx.send(false);
            }
        });
        (approval, rx)
    }

    pub fn pending(&self, thread: &str) -> Vec<PendingApproval> {
        lock(&self.open)
            .get(thread)
            .map(|l| l.iter().map(|e| e.approval.clone()).collect())
            .unwrap_or_default()
    }

    /// `false` if the request is unknown, already decided, or belongs to another thread.
    pub fn resolve(&self, thread: &str, id: &str, allow: bool) -> bool {
        match take(&self.open, thread, id) {
            Some(e) => {
                let _ = e.tx.send(allow);
                true
            }
            None => false,
        }
    }

    /// Denies every open request of the thread.
    pub fn cancel_thread(&self, thread: &str) {
        let list = lock(&self.open).remove(thread).unwrap_or_default();
        for e in list {
            let _ = e.tx.send(false);
        }
    }
}
