use crate::attachments::{build_prompt, decode_text};
use crate::config::Config;
use crate::error::CoreError;
use crate::files::Files;
use crate::policy::{check_attachment, check_provider};
use crate::providers::{
    AgentEvent, ChatMessage, CliExtra, ModelInfo, ProviderKind, ProviderStatus, build_invocation,
    detect, history, ollama, openrouter,
};
use crate::registry::ProcessRegistry;
use crate::run::{RunCtl, cli_events, drive};
use crate::settings::{Settings, SettingsPatch, SettingsView};
use crate::storage::{Approval, AttachmentRef, Message, Role, Space, Storage, Thread};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application facade used by the HTTP server (and later embedded in Tauri).
pub struct Hub {
    config: Config,
    storage: Arc<Storage>,
    registry: ProcessRegistry,
    files: Files,
    settings: RwLock<Settings>,
    http: reqwest::Client,
    /// One entry per running thread (CLI and HTTP providers alike).
    runs: Arc<Mutex<HashMap<String, Arc<RunCtl>>>>,
}

impl Hub {
    pub fn new(config: Config) -> Result<Self, CoreError> {
        std::fs::create_dir_all(&config.data_dir)?;
        let storage = Arc::new(Storage::open(&config.data_dir.join("octo.db"))?);
        let settings = Settings::load(&config.settings_path())?;
        let files = Files::open(&config.data_dir)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| CoreError::Invalid("cannot build http client"))?;
        Ok(Self {
            config,
            storage,
            registry: ProcessRegistry::default(),
            files,
            settings: RwLock::new(settings),
            http,
            runs: Arc::default(),
        })
    }

    pub fn files(&self) -> &Files {
        &self.files
    }

    fn settings_snapshot(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn settings(&self) -> SettingsView {
        self.settings_snapshot().view()
    }

    pub fn update_settings(&self, patch: SettingsPatch) -> Result<SettingsView, CoreError> {
        let mut s = self.settings.write().unwrap_or_else(|e| e.into_inner());
        let mut next = s.clone();
        next.apply(patch)?;
        next.save(&self.config.settings_path())?;
        *s = next;
        Ok(s.view())
    }

    pub async fn providers(&self) -> Vec<ProviderStatus> {
        let settings = self.settings_snapshot();
        let mut out = vec![];
        for k in ProviderKind::ALL {
            out.push(detect(k, &self.config, &settings, &self.http).await);
        }
        out
    }

    /// Models of an HTTP provider; CLI providers have no model list.
    pub async fn models(&self, kind: ProviderKind) -> Result<Vec<ModelInfo>, CoreError> {
        let s = self.settings_snapshot();
        match kind {
            ProviderKind::Ollama => ollama::models(&self.http, &s.ollama_url).await,
            ProviderKind::OpenRouter => {
                openrouter::models(
                    &self.http,
                    &self.config.openrouter_base,
                    s.openrouter_api_key.as_deref(),
                )
                .await
            }
            ProviderKind::Claude | ProviderKind::Antigravity => Ok(vec![]),
        }
        .map_err(CoreError::Upstream)
    }

    pub fn list_threads(&self) -> Result<Vec<Thread>, CoreError> {
        Ok(self.storage.list_threads()?)
    }

    pub fn create_thread(
        &self,
        provider: ProviderKind,
        space: Space,
        approval: Approval,
        model: Option<&str>,
    ) -> Result<Thread, CoreError> {
        check_provider(space, provider)?;
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        if provider.is_http() && model.is_none() {
            return Err(CoreError::Invalid("model is required"));
        }
        Ok(self
            .storage
            .create_thread(provider, space, approval, model)?)
    }

    /// Applies from the next run on; a running process keeps its flags.
    pub fn set_approval(&self, id: &str, approval: Approval) -> Result<Thread, CoreError> {
        if !self.storage.set_approval(id, approval)? {
            return Err(CoreError::NotFound);
        }
        self.storage.get_thread(id)?.ok_or(CoreError::NotFound)
    }

    pub fn delete_thread(&self, id: &str) -> Result<(), CoreError> {
        self.cancel(id);
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

    pub fn is_running(&self, id: &str) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    pub fn cancel(&self, id: &str) {
        let ctl = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        if let Some(ctl) = ctl {
            ctl.cancel();
        }
        self.registry.cancel(id);
    }

    /// Reads the attached text files (policy first) and inlines them into the prompt.
    fn prompt_with_attachments(
        &self,
        thread: &Thread,
        content: &str,
        attachments: &[AttachmentRef],
    ) -> Result<String, CoreError> {
        let mut files = vec![];
        for a in attachments {
            check_attachment(thread.space, a.space)?;
            let data = self.files.read(a.space, &a.path)?;
            files.push((a.path.clone(), decode_text(&a.path, &data)?));
        }
        Ok(build_prompt(content, &files))
    }

    fn history(
        &self,
        id: &str,
        prompt: String,
        budget: usize,
    ) -> Result<Vec<ChatMessage>, CoreError> {
        let mut h: Vec<ChatMessage> = self
            .storage
            .list_messages(id)?
            .into_iter()
            .filter_map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Error => return None,
                };
                Some(ChatMessage {
                    role,
                    content: m.content,
                })
            })
            .collect();
        h.push(ChatMessage {
            role: "user",
            content: prompt,
        });
        Ok(history::budget(h, budget))
    }

    /// Starts a run; fails fast (policy, busy, spawn) before any event is produced.
    pub async fn send_message(
        &self,
        id: &str,
        content: &str,
        attachments: &[AttachmentRef],
    ) -> Result<mpsc::Receiver<AgentEvent>, CoreError> {
        let content = content.trim();
        if content.is_empty() {
            return Err(CoreError::Invalid("empty message"));
        }
        let thread = self.storage.get_thread(id)?.ok_or(CoreError::NotFound)?;
        check_provider(thread.space, thread.provider)?;
        let settings = self.settings_snapshot();
        let model = thread.model.clone();
        if thread.provider.is_http() && model.is_none() {
            return Err(CoreError::Invalid("model is required"));
        }
        let key = settings.openrouter_api_key.clone();
        if thread.provider == ProviderKind::OpenRouter && key.is_none() {
            return Err(CoreError::Invalid("OpenRouter API key is not configured"));
        }
        let prompt = self.prompt_with_attachments(&thread, content, attachments)?;

        let ctl = Arc::new(RunCtl::default());
        {
            let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            if runs.contains_key(id) {
                return Err(CoreError::Busy);
            }
            runs.insert(id.to_string(), ctl.clone());
        }
        let started = self
            .start(&thread, &settings, &prompt, key, model, &ctl)
            .and_then(|rx| {
                self.storage
                    .add_message_with(id, Role::User, content, attachments)?;
                self.storage.set_title_if_empty(id, content)?;
                Ok(rx)
            });
        let events = match started {
            Ok(rx) => rx,
            Err(e) => {
                self.runs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(id);
                return Err(e);
            }
        };

        let (out, rx) = mpsc::channel(64);
        let storage = self.storage.clone();
        let runs = self.runs.clone();
        let id = id.to_string();
        let has_session = thread.external_session_id.is_some();
        tokio::spawn(async move {
            drive(&storage, &id, &ctl, has_session, events, &out).await;
            // release the thread before the client sees the stream end
            runs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            drop(out);
        });
        Ok(rx)
    }

    /// Spawns the provider and returns its event source.
    fn start(
        &self,
        thread: &Thread,
        settings: &Settings,
        prompt: &str,
        key: Option<String>,
        model: Option<String>,
        ctl: &Arc<RunCtl>,
    ) -> Result<mpsc::Receiver<AgentEvent>, CoreError> {
        let kind = thread.provider;
        if !kind.is_http() {
            let extra = CliExtra {
                model: model.as_deref(),
                add_dir: (thread.space == Space::Public).then(|| self.files.public_dir()),
            };
            let inv = build_invocation(
                kind,
                &self.config,
                prompt,
                thread.external_session_id.as_deref(),
                thread.approval,
                extra,
            )
            .ok_or(CoreError::Invalid("not a CLI provider"))?;
            let handle = self.registry.spawn(&thread.id, inv.command, inv.stdin)?;
            return Ok(cli_events(kind, handle));
        }
        let history = self.history(&thread.id, prompt.to_string(), settings.history_char_budget)?;
        let (tx, rx) = mpsc::channel(64);
        let http = self.http.clone();
        let ctl = ctl.clone();
        let model = model.unwrap_or_default();
        let ollama_url = settings.ollama_url.clone();
        let or_base = self.config.openrouter_base.clone();
        tokio::spawn(async move {
            let run = async {
                match kind {
                    ProviderKind::OpenRouter => {
                        let key = key.unwrap_or_default();
                        openrouter::stream_chat(&http, &or_base, &key, &model, &history, &tx).await
                    }
                    _ => ollama::stream_chat(&http, &ollama_url, &model, &history, &tx).await,
                }
            };
            tokio::select! {
                () = run => {}
                () = ctl.cancelled() => {}
            }
        });
        Ok(rx)
    }
}
