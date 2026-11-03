use crate::config::Config;
use crate::error::CoreError;
use crate::policy::check_provider;
use crate::provider::{
    AgentEvent, ProviderKind, ProviderStatus, build_invocation, detect, parse_line,
};
use crate::registry::{ProcEvent, ProcessRegistry};
use crate::storage::{Message, Role, Space, Storage, Thread};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Application facade used by the HTTP server (and later embedded in Tauri).
pub struct Hub {
    config: Config,
    storage: Arc<Storage>,
    registry: ProcessRegistry,
}

impl Hub {
    pub fn new(config: Config) -> Result<Self, CoreError> {
        std::fs::create_dir_all(&config.data_dir)?;
        let storage = Arc::new(Storage::open(&config.data_dir.join("octo.db"))?);
        Ok(Self {
            config,
            storage,
            registry: ProcessRegistry::default(),
        })
    }

    pub async fn providers(&self) -> Vec<ProviderStatus> {
        let mut out = vec![];
        for k in ProviderKind::ALL {
            out.push(detect(k, &self.config).await);
        }
        out
    }

    pub fn list_threads(&self) -> Result<Vec<Thread>, CoreError> {
        Ok(self.storage.list_threads()?)
    }

    pub fn create_thread(&self, provider: ProviderKind, space: Space) -> Result<Thread, CoreError> {
        check_provider(space, provider)?;
        Ok(self.storage.create_thread(provider, space)?)
    }

    pub fn delete_thread(&self, id: &str) -> Result<(), CoreError> {
        self.registry.cancel(id);
        if self.storage.delete_thread(id)? {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }

    pub fn list_messages(&self, id: &str) -> Result<Vec<Message>, CoreError> {
        self.storage.get_thread(id)?.ok_or(CoreError::NotFound)?;
        Ok(self.storage.list_messages(id)?)
    }

    pub fn cancel(&self, id: &str) {
        self.registry.cancel(id);
    }

    /// Starts a run; fails fast (policy, busy, spawn) before any event is produced.
    pub async fn send_message(
        &self,
        id: &str,
        content: &str,
    ) -> Result<mpsc::Receiver<AgentEvent>, CoreError> {
        let content = content.trim();
        if content.is_empty() {
            return Err(CoreError::Invalid("empty message"));
        }
        let thread = self.storage.get_thread(id)?.ok_or(CoreError::NotFound)?;
        check_provider(thread.space, thread.provider)?;
        let inv = build_invocation(
            thread.provider,
            &self.config,
            content,
            thread.external_session_id.as_deref(),
        );
        let mut handle = self.registry.spawn(id, inv.command, inv.stdin)?;
        self.storage.add_message(id, Role::User, content)?;
        self.storage.set_title_if_empty(id, content)?;

        let (out, rx) = mpsc::channel(64);
        let storage = self.storage.clone();
        let id = id.to_string();
        let kind = thread.provider;
        let has_session = thread.external_session_id.is_some();
        tokio::spawn(async move {
            let mut text = String::new();
            let mut finished = false;
            let mut session_saved = has_session;
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
                    if finished {
                        break;
                    }
                    match &e {
                        AgentEvent::Session { id: sid } if !session_saved => {
                            session_saved = storage.set_external_session(&id, sid).is_ok();
                        }
                        AgentEvent::TextDelta { text: t } => text.push_str(t),
                        AgentEvent::Done => {
                            finished = true;
                            persist(&storage, &id, Role::Assistant, &text);
                        }
                        AgentEvent::Error { message } => {
                            finished = true;
                            persist(&storage, &id, Role::Error, message);
                        }
                        _ => {}
                    }
                    let _ = out.send(e).await;
                }
            }
            if !finished {
                // cancelled: keep what was streamed so far
                persist(&storage, &id, Role::Assistant, &text);
            }
        });
        Ok(rx)
    }
}

fn persist(storage: &Storage, thread_id: &str, role: Role, content: &str) {
    let content = content.trim_end();
    if !content.is_empty() {
        let _ = storage.add_message(thread_id, role, content);
    }
}
