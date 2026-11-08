use super::{AgentEvent, CliExtra, str_of};
use crate::storage::Approval;
use serde_json::Value;

pub fn args(
    approval: Approval,
    external_session_id: Option<&str>,
    extra: CliExtra<'_>,
) -> Vec<String> {
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
    if let Some(m) = extra.model {
        a.extend(["--model".into(), m.into()]);
    }
    if let Some(d) = extra.add_dir {
        a.extend(["--add-dir".into(), d.to_string_lossy().into_owned()]);
    }
    if let Some(id) = external_session_id {
        a.extend(["--conversation".into(), id.into()]);
    }
    a
}

pub fn stdin(prompt: &str) -> String {
    let msg = serde_json::json!({"event": "user", "message": {"content": prompt}});
    format!("{msg}\n")
}

pub fn parse(v: &Value) -> Vec<AgentEvent> {
    match v["event"].as_str() {
        Some("init") => str_of(&v["conversation_id"])
            .map(|id| vec![AgentEvent::Session { id }])
            .unwrap_or_default(),
        Some("step_update") => {
            let u = &v["step_update"];
            match u["step_type"].as_str() {
                Some("agent_response") => str_of(&u["text_delta"])
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
            let message = str_of(&v["result"]["error"]).unwrap_or_else(|| "error".into());
            vec![AgentEvent::Error { message }]
        }
        _ => vec![],
    }
}
