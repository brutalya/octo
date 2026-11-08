use crate::providers::{AgentEvent, ProviderKind, parse_line};
use crate::registry::{ProcEvent, ProcessRegistry, RunHandle};
use crate::storage::{Role, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
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

/// How long a process may linger after its terminal event before it is cancelled.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// Turns the raw process output of a CLI provider into agent events. After a terminal
/// event the process gets `EXIT_GRACE` to exit on its own, then `key` is cancelled.
pub fn cli_events(
    kind: ProviderKind,
    mut handle: RunHandle,
    registry: ProcessRegistry,
    key: String,
) -> mpsc::Receiver<AgentEvent> {
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
            if finished {
                let drain = async { while handle.events.recv().await.is_some() {} };
                if tokio::time::timeout(EXIT_GRACE, drain).await.is_err() {
                    registry.cancel(&key);
                }
                return;
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
    while let Some(mut e) = events.recv().await {
        if finished {
            break;
        }
        if matches!(e, AgentEvent::Done | AgentEvent::Error { .. }) && ctl.is_cancelled() {
            break;
        }
        match &e {
            AgentEvent::Session { id } if !session_saved => {
                match storage.set_external_session(thread_id, id) {
                    Ok(()) => session_saved = true,
                    Err(err) => eprintln!("octo: cannot save session id: {err}"),
                }
            }
            AgentEvent::TextDelta { text: t } => text.push_str(t),
            AgentEvent::Done => {
                finished = true;
                if let Err(err) = persist(storage, thread_id, Role::Assistant, &text) {
                    e = save_failed(&err);
                }
            }
            AgentEvent::Error { message } => {
                finished = true;
                if let Err(err) = persist(storage, thread_id, Role::Error, message) {
                    eprintln!("octo: cannot save error message: {err}");
                }
            }
            _ => {}
        }
        let _ = out.send(e).await;
    }
    if !finished && let Err(err) = persist(storage, thread_id, Role::Assistant, &text) {
        // cancelled (or the source vanished): keep what was streamed so far
        let _ = out.send(save_failed(&err)).await;
    }
}

fn save_failed(err: &rusqlite::Error) -> AgentEvent {
    AgentEvent::Error {
        message: format!("could not save the reply: {err}"),
    }
}

fn persist(storage: &Storage, thread_id: &str, role: Role, content: &str) -> rusqlite::Result<()> {
    let content = content.trim_end();
    if !content.is_empty() {
        storage.add_message(thread_id, role, content)?;
    }
    Ok(())
}
