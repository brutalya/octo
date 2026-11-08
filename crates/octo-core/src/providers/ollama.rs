use super::lines::{Pumped, pump};
use super::{AgentEvent, ChatMessage, ModelInfo, chat_body, join_url};
use crate::error::CoreError;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

/// Detect: the tags endpoint answers within 1s.
pub async fn reachable(http: &reqwest::Client, base: &str) -> bool {
    http.get(join_url(base, "/api/tags"))
        .timeout(Duration::from_secs(1))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

pub fn parse_models(body: &str) -> Vec<ModelInfo> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["name"].as_str().or(m["model"].as_str()))
        .map(|n| ModelInfo {
            id: n.into(),
            name: n.into(),
        })
        .collect()
}

pub async fn models(http: &reqwest::Client, base: &str) -> Result<Vec<ModelInfo>, CoreError> {
    let resp = http
        .get(join_url(base, "/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| CoreError::Upstream(format!("cannot reach Ollama at {base}")))?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        return Err(CoreError::Upstream(format!("Ollama answered HTTP {code}")));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| CoreError::Upstream(e.without_url().to_string()))?;
    Ok(parse_models(&body))
}

/// One NDJSON line of `/api/chat`.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    if let Some(message) = v["error"].as_str() {
        return vec![AgentEvent::Error {
            message: message.into(),
        }];
    }
    let mut out = vec![];
    if let Some(text) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) {
        out.push(AgentEvent::TextDelta { text: text.into() });
    }
    if v["done"] == true {
        out.push(AgentEvent::Done);
    }
    out
}

/// Streams a chat turn. Always ends with `Done` or `Error` unless the receiver is gone.
pub async fn stream_chat(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    history: &[ChatMessage],
    tx: &mpsc::Sender<AgentEvent>,
) {
    let sent = http
        .post(join_url(base, "/api/chat"))
        .json(&chat_body(model, history))
        .send()
        .await;
    let resp = match sent {
        Ok(r) => r,
        Err(_) => {
            let message = format!("cannot reach Ollama at {base}");
            let _ = tx.send(AgentEvent::Error { message }).await;
            return;
        }
    };
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {code}"));
        let _ = tx.send(AgentEvent::Error { message }).await;
        return;
    }
    let end = pump(resp, parse_line, |_| false, tx).await;
    if end == (Pumped::Eof { complete: false }) {
        let message = "stream ended before completion".to_string();
        let _ = tx.send(AgentEvent::Error { message }).await;
    }
}
