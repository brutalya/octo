use crate::providers::{AgentEvent, ProviderKind, parse_line};
use crate::registry::{ProcEvent, RunHandle};
use crate::storage::{Role, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, mpsc};

/// Cancel switch of one running thread.
#[derive(Default)]
pub struct RunCtl {
    cancelled: AtomicBool,
    notify: Notify,
}

impl RunCtl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves when cancel was requested (also if it was requested earlier).
    pub async fn cancelled(&self) {
        self.notify.notified().await
    }
}

/// Turns the raw process output of a CLI provider into agent events.
pub fn cli_events(kind: ProviderKind, mut handle: RunHandle) -> mpsc::Receiver<AgentEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut finished = false;
        while let Some(ev) = handle.events.recv().await {
            let events = match ev {
                ProcEvent::Line(l) => parse_line(kind, &l),
                ProcEvent::Exit { success, stderr } if !finished => {
                    let message = stderr.trim();
                    if !success || !message.is_empty() {
                        let message = if message.is_empty() {
                            "process exited with an error"
                        } else {
                            message
                        };
                        vec![AgentEvent::Error {
                            message: message.to_string(),
                        }]
                    } else {
                        vec![AgentEvent::Done]
                    }
                }
                ProcEvent::Exit { .. } => vec![],
            };
            for e in events {
                finished |= matches!(e, AgentEvent::Done | AgentEvent::Error { .. });
                if tx.send(e).await.is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// Forwards events to the client and persists the outcome. A completion that races with
/// a cancel is dropped: cancel never reports done, and the partial text is kept.
pub async fn drive(
    storage: &Storage,
    thread_id: &str,
    ctl: &RunCtl,
    mut session_saved: bool,
    mut events: mpsc::Receiver<AgentEvent>,
    out: &mpsc::Sender<AgentEvent>,
) {
    let mut text = String::new();
    let mut finished = false;
    while let Some(e) = events.recv().await {
        if finished {
            break;
        }
        if matches!(e, AgentEvent::Done | AgentEvent::Error { .. }) && ctl.is_cancelled() {
            break;
        }
        match &e {
            AgentEvent::Session { id } if !session_saved => {
                session_saved = storage.set_external_session(thread_id, id).is_ok();
            }
            AgentEvent::TextDelta { text: t } => text.push_str(t),
            AgentEvent::Done => {
                finished = true;
                persist(storage, thread_id, Role::Assistant, &text);
            }
            AgentEvent::Error { message } => {
                finished = true;
                persist(storage, thread_id, Role::Error, message);
            }
            _ => {}
        }
        let _ = out.send(e).await;
    }
    if !finished {
        // cancelled (or the source vanished): keep what was streamed so far
        persist(storage, thread_id, Role::Assistant, &text);
    }
}

fn persist(storage: &Storage, thread_id: &str, role: Role, content: &str) {
    let content = content.trim_end();
    if !content.is_empty() {
        let _ = storage.add_message(thread_id, role, content);
    }
}
