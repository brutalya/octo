//! Antigravity CLI: index in `conversation_summaries.db`, prompts in `history.jsonl`.
//!
//! The conversation steps (`conversations/<id>.db`, table `steps`) hold one schema-less
//! protobuf per step; user and assistant text are decoded from it (see `steps`). When a
//! conversation cannot be decoded, the prompts from `history.jsonl` plus one muted
//! assistant line are shown instead.
use super::{Session, SessionData, short_title, time, uri_to_path, valid_id, wire};
use crate::storage::{Role, SourceMessage};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value as Json;
use std::path::Path;

/// Muted (markdown italic) line that stands for the unreadable assistant side.
pub const UNAVAILABLE: &str = "*history unavailable*";

struct Row {
    id: String,
    title: String,
    preview: String,
    steps: i64,
    modified_raw: String,
    modified: i64,
    workspace: Option<String>,
}

struct Prompt {
    text: String,
    at: i64,
    workspace: String,
}

/// Top-level conversations with at least one step, newest first.
pub fn list(dir: &Path) -> Vec<Session> {
    let rows = rows(dir);
    let prompts = prompts(dir);
    let mut out: Vec<Session> = rows
        .iter()
        .map(|r| build(dir, r, &rows, &prompts).session)
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out
}

pub fn read(dir: &Path, id: &str) -> Option<SessionData> {
    if !valid_id(id) {
        return None;
    }
    let rows = rows(dir);
    let row = rows.iter().find(|r| r.id == id)?;
    Some(build(dir, row, &rows, &prompts(dir)))
}

fn norm(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

fn text_of(v: Value) -> String {
    match v {
        Value::Text(s) => s,
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        _ => String::new(),
    }
}

fn rows(dir: &Path) -> Vec<Row> {
    let Ok(conn) = Connection::open_with_flags(
        dir.join("conversation_summaries.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return vec![];
    };
    let sql = "SELECT conversation_id, COALESCE(title,''), COALESCE(preview,''), step_count,
                      last_modified_time, COALESCE(workspace_uris,'[]')
               FROM conversation_summaries
               WHERE COALESCE(parent_conversation_id,'') = '' AND step_count > 0";
    let Ok(mut st) = conn.prepare(sql) else {
        return vec![];
    };
    let Ok(mapped) = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            text_of(r.get::<_, Value>(4)?),
            r.get::<_, String>(5)?,
        ))
    }) else {
        return vec![];
    };
    mapped
        .flatten()
        .filter_map(|(id, title, preview, steps, modified_raw, uris)| {
            let modified = time::parse_ms(&modified_raw)?;
            let workspace = serde_json::from_str::<Vec<String>>(&uris)
                .ok()
                .and_then(|v| v.into_iter().next())
                .map(|u| uri_to_path(&u));
            Some(Row {
                id,
                title,
                preview,
                steps,
                modified_raw,
                modified,
                workspace,
            })
        })
        .collect()
}

/// User prompts (not slash commands) from `history.jsonl`.
fn prompts(dir: &Path) -> Vec<Prompt> {
    let Ok(text) = std::fs::read_to_string(dir.join("history.jsonl")) else {
        return vec![];
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Json>(l).ok())
        .filter(|v| v["type"].is_null())
        .filter_map(|v| {
            let text = v["display"].as_str()?.trim().to_string();
            let at = v["timestamp"].as_i64()?;
            let workspace = norm(&uri_to_path(v["workspace"].as_str()?));
            (!text.is_empty()).then_some(Prompt {
                text,
                at,
                workspace,
            })
        })
        .collect()
}

/// A prompt belongs to the first session of its workspace that was modified after it:
/// the window is (previous session's last change, this session's last change].
fn build(dir: &Path, row: &Row, rows: &[Row], prompts: &[Prompt]) -> SessionData {
    let ws = row.workspace.as_deref().map(norm);
    let lower = rows
        .iter()
        .filter(|o| o.workspace.as_deref().map(norm) == ws && o.modified < row.modified)
        .map(|o| o.modified)
        .max()
        .unwrap_or(i64::MIN);
    if let Some(messages) = steps(dir, &row.id) {
        return data(row, messages);
    }
    let mut messages: Vec<SourceMessage> = match &ws {
        Some(ws) => prompts
            .iter()
            .filter(|p| &p.workspace == ws && p.at > lower && p.at <= row.modified)
            .map(|p| SourceMessage {
                role: Role::User,
                content: p.text.clone(),
                created_at: p.at,
            })
            .collect(),
        None => vec![],
    };
    messages.sort_by_key(|m| m.created_at);
    messages.push(SourceMessage {
        role: Role::Assistant,
        content: UNAVAILABLE.into(),
        created_at: row.modified,
    });
    data(row, messages)
}

fn data(row: &Row, messages: Vec<SourceMessage>) -> SessionData {
    let title = [row.title.as_str(), row.preview.as_str()]
        .into_iter()
        .map(short_title)
        .find(|t| !t.is_empty())
        .or_else(|| messages.first().map(|m| short_title(&m.content)))
        .unwrap_or_else(|| "Antigravity session".into());
    SessionData {
        session: Session {
            external_id: row.id.clone(),
            title,
            cwd: row.workspace.clone(),
            updated_at: row.modified,
            messages: messages.len(),
            stamp: format!("{}:{}", row.steps, row.modified_raw),
        },
        messages,
    }
}

/// Step types in `conversations/<id>.db` that carry chat text (payload field 1).
const STEP_USER: u64 = 14;
const STEP_REPLY: u64 = 15;

/// User and assistant messages decoded from `<dir>/conversations/<id>.db`, in step order.
///
/// Payload layout (found by inspecting real sessions, no official schema):
/// `1` step type, `5.1` timestamp `{1: seconds, 2: nanos}`, user step: `19.2` prompt text,
/// reply step: `20.1` reply text. Other steps (tool calls, system notices) are skipped,
/// and consecutive replies are merged into one message. `None` when the database is
/// missing, unreadable or yields no text, so the caller can fall back.
fn steps(dir: &Path, id: &str) -> Option<Vec<SourceMessage>> {
    let conn = Connection::open_with_flags(
        dir.join("conversations").join(format!("{id}.db")),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let mut st = conn
        .prepare("SELECT step_payload FROM steps WHERE step_payload IS NOT NULL ORDER BY idx")
        .ok()?;
    let payloads = st.query_map([], |r| r.get::<_, Vec<u8>>(0)).ok()?;
    let mut out: Vec<SourceMessage> = Vec::new();
    for payload in payloads.flatten() {
        let Some(msg) = step_message(&payload) else {
            continue;
        };
        match out.last_mut() {
            Some(prev) if prev.role == Role::Assistant && msg.role == Role::Assistant => {
                prev.content.push_str("\n\n");
                prev.content.push_str(&msg.content);
            }
            _ => out.push(msg),
        }
    }
    (!out.is_empty()).then_some(out)
}

fn step_message(payload: &[u8]) -> Option<SourceMessage> {
    let (role, text) = match wire::varint_field(payload, 1)? {
        STEP_USER => (
            Role::User,
            wire::bytes_field(wire::bytes_field(payload, 19)?, 2)?,
        ),
        STEP_REPLY => (
            Role::Assistant,
            wire::bytes_field(wire::bytes_field(payload, 20)?, 1)?,
        ),
        _ => return None,
    };
    let content = std::str::from_utf8(text).ok()?.trim();
    if content.is_empty() {
        return None;
    }
    let created_at = wire::bytes_field(payload, 5)
        .and_then(|m| wire::bytes_field(m, 1))
        .and_then(|t| {
            let secs = i64::try_from(wire::varint_field(t, 1)?).ok()?;
            let nanos = i64::try_from(wire::varint_field(t, 2).unwrap_or(0)).ok()?;
            secs.checked_mul(1000)?.checked_add(nanos / 1_000_000)
        })
        .unwrap_or(0);
    Some(SourceMessage {
        role,
        content: content.to_string(),
        created_at,
    })
}
