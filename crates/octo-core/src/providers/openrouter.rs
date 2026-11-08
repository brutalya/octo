use super::lines::{Pumped, pump};
use super::{AgentEvent, ChatMessage, ModelInfo, chat_body, join_url};
use crate::error::CoreError;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

/// True for a data line that carries a `finish_reason`: the model finished, even if the
/// gateway then closes the stream without `[DONE]`.
fn is_finish(line: &str) -> bool {
    let Some(data) = line.strip_prefix("data:") else {
        return false;
    };
    serde_json::from_str::<Value>(data.trim())
        .is_ok_and(|v| v["choices"][0]["finish_reason"].as_str().is_some())
}

/// One SSE line of `/chat/completions`. Comments (`: ...`) and other fields are ignored.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let Some(data) = line.strip_prefix("data:") else {
        return vec![];
    };
    let data = data.trim();
    if data == "[DONE]" {
        return vec![AgentEvent::Done];
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return vec![];
    };
    if !v["error"].is_null() {
        let message = v["error"]["message"].as_str().unwrap_or("error");
        return vec![AgentEvent::Error {
            message: message.into(),
        }];
    }
    match v["choices"][0]["delta"]["content"]
        .as_str()
        .filter(|t| !t.is_empty())
    {
        Some(text) => vec![AgentEvent::TextDelta { text: text.into() }],
        None => vec![],
    }
}

pub fn parse_models(body: &str) -> Vec<ModelInfo> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            Some(ModelInfo {
                id: id.into(),
                name: m["name"].as_str().unwrap_or(id).into(),
            })
        })
        .collect()
}

/// Message for a non-2xx response; never includes request headers.
pub fn error_message(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

pub async fn models(
    http: &reqwest::Client,
    base: &str,
    key: Option<&str>,
) -> Result<Vec<ModelInfo>, CoreError> {
    let mut req = http
        .get(join_url(base, "/models"))
        .timeout(Duration::from_secs(15));
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let resp = req
        .send()
        .await
        .map_err(|_| CoreError::Upstream("cannot reach OpenRouter".into()))?;
    let ok = resp.status().is_success();
    let code = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| CoreError::Upstream(e.without_url().to_string()))?;
    if !ok {
        return Err(CoreError::Upstream(error_message(code, &body)));
    }
    Ok(parse_models(&body))
}

/// Streams a chat turn. Always ends with `Done` or `Error` unless the receiver is gone.
pub async fn stream_chat(
    http: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    history: &[ChatMessage],
    tx: &mpsc::Sender<AgentEvent>,
) {
    let sent = http
        .post(join_url(base, "/chat/completions"))
        .bearer_auth(key)
        .json(&chat_body(model, history))
        .send()
        .await;
    let resp = match sent {
        Ok(r) => r,
        Err(_) => {
            let message = "cannot reach OpenRouter".to_string();
            let _ = tx.send(AgentEvent::Error { message }).await;
            return;
        }
    };
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let message = error_message(code, &body);
        let _ = tx.send(AgentEvent::Error { message }).await;
        return;
    }
    // Some gateways close the stream without [DONE]; that is complete only after a
    // finish_reason, otherwise the connection was cut.
    match pump(resp, parse_line, is_finish, tx).await {
        Pumped::Terminal => {}
        Pumped::Eof { complete: true } => {
            let _ = tx.send(AgentEvent::Done).await;
        }
        Pumped::Eof { complete: false } => {
            let message = "stream ended before completion".to_string();
            let _ = tx.send(AgentEvent::Error { message }).await;
        }
    }
}
