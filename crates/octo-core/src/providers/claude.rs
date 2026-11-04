use super::{AgentEvent, CliExtra};
use crate::storage::Approval;
use serde_json::Value;

pub fn args(
    approval: Approval,
    external_session_id: Option<&str>,
    new_session_id: &str,
    extra: CliExtra<'_>,
) -> Vec<String> {
    let mode = match approval {
        Approval::Auto => "auto",
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
    if let Some(m) = extra.model {
        a.extend(["--model".into(), m.into()]);
    }
    if let Some(d) = extra.add_dir {
        a.extend(["--add-dir".into(), d.to_string_lossy().into_owned()]);
    }
    match external_session_id {
        Some(id) => a.extend(["--resume".into(), id.into()]),
        None => a.extend(["--session-id".into(), new_session_id.into()]),
    }
    a
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

pub fn parse(v: &Value) -> Vec<AgentEvent> {
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

pub(crate) fn tool_summary(input: &Value) -> String {
    ["command", "file_path", "pattern", "url", "path", "query"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}
