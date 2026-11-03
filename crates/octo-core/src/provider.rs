use crate::config::Config;
use crate::storage::Approval;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 2] = [ProviderKind::Claude, ProviderKind::Antigravity];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Claude => "claude",
            ProviderKind::Antigravity => "antigravity",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "antigravity" {
            ProviderKind::Antigravity
        } else {
            ProviderKind::Claude
        }
    }

    /// Both CLIs send data to a cloud service.
    pub fn is_local(self) -> bool {
        false
    }

    fn bin(self, cfg: &Config) -> &Path {
        match self {
            ProviderKind::Claude => &cfg.claude_bin,
            ProviderKind::Antigravity => &cfg.agy_bin,
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

/// Command line plus the bytes to write on stdin. The prompt never goes in argv.
pub struct Invocation {
    pub command: Command,
    pub stdin: String,
}

/// Pure argument builder. Auto-approval flags appear only for `Approval::Auto`.
pub fn build_args(
    kind: ProviderKind,
    approval: Approval,
    external_session_id: Option<&str>,
    new_session_id: &str,
) -> Vec<String> {
    match kind {
        ProviderKind::Claude => {
            let mode = match approval {
                Approval::Auto => "bypassPermissions",
                // Default mode: in -p, tool uses needing permission are denied.
                Approval::Confirm => "default",
            };
            let mut a: Vec<String> = [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                mode,
            ]
            .map(String::from)
            .to_vec();
            match external_session_id {
                Some(id) => a.extend(["--resume".into(), id.into()]),
                None => a.extend(["--session-id".into(), new_session_id.into()]),
            }
            a
        }
        ProviderKind::Antigravity => {
            // `-p` requires a value; the real prompt is sent as NDJSON on stdin.
            // `request-review` makes headless agy auto-deny actions that need a prompt.
            let tool_permission = match approval {
                Approval::Auto => "always-proceed",
                Approval::Confirm => "request-review",
            };
            let settings = serde_json::json!({"toolPermission": tool_permission});
            let mut a: Vec<String> = [
                "-p",
                "",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--settings",
            ]
            .map(String::from)
            .to_vec();
            a.push(settings.to_string());
            if let Some(id) = external_session_id {
                a.extend(["--conversation".into(), id.into()]);
            }
            a
        }
    }
}

pub fn build_invocation(
    kind: ProviderKind,
    cfg: &Config,
    prompt: &str,
    external_session_id: Option<&str>,
    approval: Approval,
) -> Invocation {
    let mut command = Command::new(kind.bin(cfg));
    command.kill_on_drop(true);
    command.args(build_args(
        kind,
        approval,
        external_session_id,
        &uuid::Uuid::new_v4().to_string(),
    ));
    let stdin = match kind {
        ProviderKind::Claude => prompt.to_string(),
        ProviderKind::Antigravity => {
            let msg = serde_json::json!({"event": "user", "message": {"content": prompt}});
            format!("{msg}\n")
        }
    };
    Invocation { command, stdin }
}

pub async fn detect(kind: ProviderKind, cfg: &Config) -> ProviderStatus {
    let mut cmd = Command::new(kind.bin(cfg));
    cmd.arg("--version")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await;
    let (installed, version) = match out {
        Ok(Ok(o)) => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, (!v.is_empty()).then_some(v))
        }
        _ => (false, None),
    };
    ProviderStatus {
        kind,
        installed,
        version,
        is_local: kind.is_local(),
    }
}

pub fn parse_line(kind: ProviderKind, line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    match kind {
        ProviderKind::Claude => parse_claude(&v),
        ProviderKind::Antigravity => parse_agy(&v),
    }
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn parse_claude(v: &Value) -> Vec<AgentEvent> {
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => s(&v["session_id"])
            .map(|id| vec![AgentEvent::Session { id }])
            .unwrap_or_default(),
        Some("stream_event") if v["event"]["delta"]["type"] == "text_delta" => {
            s(&v["event"]["delta"]["text"])
                .map(|text| vec![AgentEvent::TextDelta { text }])
                .unwrap_or_default()
        }
        Some("assistant") => v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| AgentEvent::Tool {
                name: s(&b["name"]).unwrap_or_default(),
                summary: tool_summary(&b["input"]),
            })
            .collect(),
        Some("result") if v["is_error"] == true => {
            vec![AgentEvent::Error {
                message: s(&v["result"]).unwrap_or_else(|| "error".into()),
            }]
        }
        Some("result") => vec![AgentEvent::Done],
        _ => vec![],
    }
}

fn tool_summary(input: &Value) -> String {
    ["command", "file_path", "pattern", "url", "path", "query"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

fn parse_agy(v: &Value) -> Vec<AgentEvent> {
    match v["event"].as_str() {
        Some("init") => s(&v["conversation_id"])
            .map(|id| vec![AgentEvent::Session { id }])
            .unwrap_or_default(),
        Some("step_update") => {
            let u = &v["step_update"];
            match u["step_type"].as_str() {
                Some("agent_response") => s(&u["text_delta"])
                    .filter(|t| !t.is_empty())
                    .map(|text| vec![AgentEvent::TextDelta { text }])
                    .unwrap_or_default(),
                Some("user_input" | "system_message") | None => vec![],
                // Tool-like steps: report once, when finished.
                Some(other) if u["state"] == "DONE" => {
                    vec![AgentEvent::Tool {
                        name: other.into(),
                        summary: String::new(),
                    }]
                }
                Some(_) => vec![],
            }
        }
        Some("result") if v["result"]["status"] == "SUCCESS" => vec![AgentEvent::Done],
        Some("result") => {
            let message = s(&v["result"]["error"]).unwrap_or_else(|| "error".into());
            vec![AgentEvent::Error { message }]
        }
        _ => vec![],
    }
}
