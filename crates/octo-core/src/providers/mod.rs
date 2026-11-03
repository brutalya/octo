pub mod antigravity;
pub mod claude;
pub mod history;
pub mod lines;
pub mod ollama;
pub mod openrouter;

use crate::config::Config;
use crate::settings::Settings;
use crate::storage::Approval;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderKind {
    Claude,
    Antigravity,
    Ollama,
    #[serde(rename = "openrouter")]
    #[ts(rename = "openrouter")]
    OpenRouter,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 4] = [
        ProviderKind::Claude,
        ProviderKind::Antigravity,
        ProviderKind::Ollama,
        ProviderKind::OpenRouter,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Claude => "claude",
            ProviderKind::Antigravity => "antigravity",
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenRouter => "openrouter",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "antigravity" => ProviderKind::Antigravity,
            "ollama" => ProviderKind::Ollama,
            "openrouter" => ProviderKind::OpenRouter,
            _ => ProviderKind::Claude,
        }
    }

    /// Only Ollama keeps data on the device; the CLIs and OpenRouter use a cloud service.
    pub fn is_local(self) -> bool {
        self == ProviderKind::Ollama
    }

    /// HTTP providers are stateless and need a model and the full history each turn.
    pub fn is_http(self) -> bool {
        matches!(self, ProviderKind::Ollama | ProviderKind::OpenRouter)
    }

    fn bin(self, cfg: &Config) -> Option<&Path> {
        match self {
            ProviderKind::Claude => Some(&cfg.claude_bin),
            ProviderKind::Antigravity => Some(&cfg.agy_bin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AgentEvent {
    Session { id: String },
    TextDelta { text: String },
    Tool { name: String, summary: String },
    Done,
    Error { message: String },
}

impl AgentEvent {
    /// SSE event name (variant in snake_case).
    pub fn name(&self) -> &'static str {
        match self {
            AgentEvent::Session { .. } => "session",
            AgentEvent::TextDelta { .. } => "text_delta",
            AgentEvent::Tool { .. } => "tool",
            AgentEvent::Done => "done",
            AgentEvent::Error { .. } => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProviderStatus {
    pub kind: ProviderKind,
    pub installed: bool,
    pub version: Option<String>,
    pub is_local: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// One turn of chat history sent to an HTTP provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// Extra CLI inputs that are not part of the session handling.
#[derive(Debug, Clone, Copy, Default)]
pub struct CliExtra<'a> {
    pub model: Option<&'a str>,
    /// Public space dir, only for public threads.
    pub add_dir: Option<&'a Path>,
}

/// Command line plus the bytes to write on stdin. The prompt never goes in argv.
pub struct Invocation {
    pub command: Command,
    pub stdin: String,
}

/// Pure argument builder for the CLI providers. Auto-approval flags appear only for
/// `Approval::Auto`. HTTP providers have no arguments.
pub fn build_args(
    kind: ProviderKind,
    approval: Approval,
    external_session_id: Option<&str>,
    new_session_id: &str,
    extra: CliExtra<'_>,
) -> Vec<String> {
    match kind {
        ProviderKind::Claude => claude::args(approval, external_session_id, new_session_id, extra),
        ProviderKind::Antigravity => antigravity::args(approval, external_session_id, extra),
        ProviderKind::Ollama | ProviderKind::OpenRouter => vec![],
    }
}

/// `None` for HTTP providers.
pub fn build_invocation(
    kind: ProviderKind,
    cfg: &Config,
    prompt: &str,
    external_session_id: Option<&str>,
    approval: Approval,
    extra: CliExtra<'_>,
) -> Option<Invocation> {
    let mut command = Command::new(kind.bin(cfg)?);
    command.kill_on_drop(true);
    command.args(build_args(
        kind,
        approval,
        external_session_id,
        &uuid::Uuid::new_v4().to_string(),
        extra,
    ));
    let stdin = match kind {
        ProviderKind::Antigravity => antigravity::stdin(prompt),
        _ => prompt.to_string(),
    };
    Some(Invocation { command, stdin })
}

pub async fn detect(
    kind: ProviderKind,
    cfg: &Config,
    settings: &Settings,
    http: &reqwest::Client,
) -> ProviderStatus {
    let (installed, version) = match kind {
        ProviderKind::Ollama => (ollama::reachable(http, &settings.ollama_url).await, None),
        ProviderKind::OpenRouter => (settings.openrouter_api_key.is_some(), None),
        _ => detect_cli(kind, cfg).await,
    };
    ProviderStatus {
        kind,
        installed,
        version,
        is_local: kind.is_local(),
    }
}

async fn detect_cli(kind: ProviderKind, cfg: &Config) -> (bool, Option<String>) {
    let Some(bin) = kind.bin(cfg) else {
        return (false, None);
    };
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    match tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
        Ok(Ok(o)) => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, (!v.is_empty()).then_some(v))
        }
        _ => (false, None),
    }
}

/// Parse one stdout line of a CLI provider. HTTP providers have their own parsers.
pub fn parse_line(kind: ProviderKind, line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return vec![];
    };
    match kind {
        ProviderKind::Claude => claude::parse(&v),
        ProviderKind::Antigravity => antigravity::parse(&v),
        ProviderKind::Ollama | ProviderKind::OpenRouter => vec![],
    }
}
