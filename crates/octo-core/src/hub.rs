use crate::attachments::{build_prompt, decode_text};
use crate::config::Config;
use crate::error::CoreError;
use crate::files::Files;
use crate::import::{self, SessionData, SessionSummary};
use crate::policy::{check_attachment, check_provider};
use crate::providers::{
    AgentEvent, ChatMessage, CliExtra, ModelInfo, ProviderKind, ProviderStatus, build_invocation,
    detect, history, ollama, openrouter,
};
use crate::registry::ProcessRegistry;
use crate::run::{RunCtl, cli_events, drive};
use crate::settings::{Settings, SettingsPatch, SettingsView};
use crate::storage::{
    Approval, AttachmentRef, Message, NewImport, Origin, Role, Space, Storage, Thread,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application facade used by the HTTP server (and later embedded in Tauri).
pub struct Hub {
    config: Config,
    storage: Arc<Storage>,
    registry: ProcessRegistry,
    files: Arc<Files>,
    settings: RwLock<Settings>,
    net: tokio::sync::watch::Sender<crate::settings::NetSettings>,
    http: reqwest::Client,
    /// One entry per running thread (CLI and HTTP providers alike).
    runs: Arc<Mutex<HashMap<String, Arc<RunCtl>>>>,
    /// Serializes imports so the same session is never imported twice.
    import_lock: Mutex<()>,
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
            files: Arc::new(files),
            net: tokio::sync::watch::channel(settings.net()).0,
            settings: RwLock::new(settings),
            http,
            runs: Arc::default(),
            import_lock: Mutex::new(()),
        })
    }

    pub fn files(&self) -> &Arc<Files> {
        &self.files
    }

    fn settings_snapshot(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Notifies when relay URL or node name change.
    pub fn watch_net(&self) -> tokio::sync::watch::Receiver<crate::settings::NetSettings> {
        self.net.subscribe()
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
        self.net.send_replace(s.net());
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
        self.create_thread_in(provider, space, approval, model, None)
    }

    /// CLI threads run in `cwd`, by default the public space dir. HTTP providers have none.
    pub fn create_thread_in(
        &self,
        provider: ProviderKind,
        space: Space,
        approval: Approval,
        model: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<Thread, CoreError> {
        check_provider(space, provider)?;
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        if provider.is_http() && model.is_none() {
            return Err(CoreError::Invalid("model is required"));
        }
        let cwd = (!provider.is_http()).then(|| {
            cwd.map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| self.files.public_dir().to_string_lossy().into_owned())
        });
        Ok(self
            .storage
            .create_thread_in(provider, space, approval, model, cwd.as_deref())?)
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
        let thread = self.storage.get_thread(id)?.ok_or(CoreError::NotFound)?;
        self.refresh_from_source(&thread)?;
        Ok(self.storage.list_messages(id)?)
    }

    /// Imported Claude threads: replace the cache when the session file changed.
    /// (Antigravity history is lossy, so its cache is never replaced.) A vanished or
    /// unreadable source keeps the cache; a thread with a run in progress is left alone.
    fn refresh_from_source(&self, t: &Thread) -> Result<(), CoreError> {
        let Some(ext) = t.external_session_id.as_deref() else {
            return Ok(());
        };
        if t.origin != Origin::Imported
            || t.provider != ProviderKind::Claude
            || self.is_running(&t.id)
        {
            return Ok(());
        }
        let Some(data) = import::claude::read(&self.config.claude_projects_dir, ext) else {
            return Ok(());
        };
        if self.storage.sync_stamp(&t.id)?.as_deref() == Some(data.session.stamp.as_str()) {
            return Ok(());
        }
        self.storage.replace_messages(
            &t.id,
            &data.messages,
            &data.session.stamp,
            data.session.updated_at,
        )?;
        Ok(())
    }

    fn require_cli(kind: ProviderKind) -> Result<(), CoreError> {
        if kind.is_http() {
            return Err(CoreError::Invalid("provider has no CLI sessions"));
        }
        Ok(())
    }

    /// Sessions in the CLI's own storage, newest first, filtered by `q` (title or cwd).
    /// Blocking file and SQLite I/O.
    pub fn import_sessions(
        &self,
        kind: ProviderKind,
        q: &str,
    ) -> Result<Vec<SessionSummary>, CoreError> {
        Self::require_cli(kind)?;
        let sessions = match kind {
            ProviderKind::Claude => import::claude::list(&self.config.claude_projects_dir),
            _ => import::antigravity::list(&self.config.agy_dir),
        };
        let linked: HashMap<String, String> = self
            .storage
            .list_threads()?
            .into_iter()
            .filter(|t| t.provider == kind)
            .filter_map(|t| t.external_session_id.map(|e| (e, t.id)))
            .collect();
        let q = q.trim().to_lowercase();
        Ok(sessions
            .into_iter()
            .filter(|s| {
                q.is_empty()
                    || s.title.to_lowercase().contains(&q)
                    || s.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&q))
            })
            .map(|s| SessionSummary {
                imported_thread_id: linked.get(&s.external_id).cloned(),
                external_id: s.external_id,
                title: s.title,
                cwd: s.cwd,
                updated_at: s.updated_at,
                messages: s.messages,
            })
            .collect())
    }

    /// Imports a CLI session as a public thread; returns the existing thread if the
    /// session is already linked. Blocking file and SQLite I/O.
    pub fn import_session(
        &self,
        kind: ProviderKind,
        external_id: &str,
    ) -> Result<Thread, CoreError> {
        Self::require_cli(kind)?;
        let _guard = self.import_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = self.storage.find_by_external(kind, external_id)? {
            return Ok(t);
        }
        let data: SessionData = match kind {
            ProviderKind::Claude => {
                import::claude::read(&self.config.claude_projects_dir, external_id)
            }
            _ => import::antigravity::read(&self.config.agy_dir, external_id),
        }
        .ok_or(CoreError::NotFound)?;
        let s = &data.session;
        Ok(self.storage.import_thread(&NewImport {
            provider: kind,
            external_id,
            title: &s.title,
            cwd: s.cwd.as_deref(),
            updated_at: s.updated_at,
            stamp: &s.stamp,
            messages: &data.messages,
        })?)
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

    /// Cancels every running thread; their processes are killed and reaped.
    pub fn shutdown(&self) {
        let ids: Vec<String> = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.cancel(&id);
        }
    }

    /// Reads the attached text files (policy first) and inlines them into the prompt.
    async fn prompt_with_attachments(
        &self,
        thread: &Thread,
        content: &str,
        attachments: &[AttachmentRef],
    ) -> Result<String, CoreError> {
        for a in attachments {
            check_attachment(thread.space, a.space)?;
        }
        let store = self.files.clone();
        let refs = attachments.to_vec();
        let files = tokio::task::spawn_blocking(move || {
            let mut files = vec![];
            for a in &refs {
                let data = store.read(a.space, &a.path)?;
                files.push((a.path.clone(), decode_text(&a.path, &data)?));
            }
            Ok::<_, CoreError>(files)
        })
        .await
        .map_err(|_| CoreError::Invalid("file task failed"))??;
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
        let prompt = self
            .prompt_with_attachments(&thread, content, attachments)
            .await?;

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
            let cwd: PathBuf = thread
                .cwd
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.files.public_dir().to_path_buf());
            if !cwd.is_dir() {
                return Err(CoreError::Invalid("working directory does not exist"));
            }
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
                &cwd,
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
