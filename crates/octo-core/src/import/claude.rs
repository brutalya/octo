//! Claude Code: `<projects>/<encoded-cwd>/<session-id>.jsonl`.
use super::{Session, SessionData, short_title, valid_id};
use crate::providers::claude::tool_summary;
use crate::storage::{Role, SourceMessage};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// All non-empty sessions, newest first.
pub fn list(projects: &Path) -> Vec<Session> {
    let mut out: Vec<Session> = session_files(projects)
        .into_iter()
        .filter_map(|f| parse_file(&f))
        .map(|d| d.session)
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out
}

pub fn read(projects: &Path, id: &str) -> Option<SessionData> {
    if !valid_id(id) {
        return None;
    }
    let name = format!("{id}.jsonl");
    subdirs(projects)
        .into_iter()
        .map(|d| d.join(&name))
        .find(|f| f.is_file())
        .and_then(|f| parse_file(&f))
}

fn subdirs(projects: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(projects) else {
        return vec![];
    };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

fn session_files(projects: &Path) -> Vec<PathBuf> {
    let mut files = vec![];
    for dir in subdirs(projects) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        files.extend(
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl")),
        );
    }
    files
}

/// Text of a user entry; tool results and command envelopes yield `None`.
fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let t = text.trim();
    let envelope = ["<local-command-", "<command-", "<system-reminder>"];
    if t.is_empty() || envelope.iter().any(|p| t.starts_with(p)) {
        return None;
    }
    Some(t.to_string())
}

fn assistant_parts(content: &Value) -> Vec<String> {
    let Some(blocks) = content.as_array() else {
        return vec![];
    };
    blocks
        .iter()
        .filter_map(|b| match b["type"].as_str() {
            Some("text") => b["text"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string),
            Some("tool_use") => {
                let name = b["name"].as_str().unwrap_or("tool");
                let summary = tool_summary(&b["input"]);
                Some(if summary.is_empty() {
                    format!("> Tool: {name}")
                } else {
                    format!("> Tool: {name} {summary}")
                })
            }
            _ => None,
        })
        .collect()
}

fn parse_file(path: &Path) -> Option<SessionData> {
    let id = path.file_stem()?.to_str()?.to_string();
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let reader = BufReader::new(std::fs::File::open(path).ok()?);

    let mut cwd: Option<String> = None;
    let mut messages: Vec<SourceMessage> = vec![];
    // consecutive assistant entries form one message
    let mut pending: Option<(i64, Vec<String>)> = None;
    let flush = |pending: &mut Option<(i64, Vec<String>)>, out: &mut Vec<SourceMessage>| {
        if let Some((created_at, parts)) = pending.take()
            && !parts.is_empty()
        {
            out.push(SourceMessage {
                role: Role::Assistant,
                content: parts.join("\n\n"),
                created_at,
            });
        }
    };
    for line in reader.split(b'\n').map_while(Result::ok) {
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if cwd.is_none()
            && let Some(c) = v["cwd"].as_str()
        {
            cwd = Some(c.to_string());
        }
        if v["isSidechain"] == true || v["isMeta"] == true {
            continue;
        }
        let ts = v["timestamp"]
            .as_str()
            .and_then(super::time::parse_ms)
            .unwrap_or(mtime);
        match v["type"].as_str() {
            Some("user") => {
                if let Some(text) = user_text(&v["message"]["content"]) {
                    flush(&mut pending, &mut messages);
                    messages.push(SourceMessage {
                        role: Role::User,
                        content: text,
                        created_at: ts,
                    });
                }
            }
            Some("assistant") => {
                let parts = assistant_parts(&v["message"]["content"]);
                if !parts.is_empty() {
                    pending.get_or_insert((ts, vec![])).1.extend(parts);
                }
            }
            _ => {}
        }
    }
    flush(&mut pending, &mut messages);
    let title = messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| short_title(&m.content))?;
    Some(SessionData {
        session: Session {
            external_id: id,
            title,
            cwd,
            updated_at: mtime,
            messages: messages.len(),
            stamp: format!("{mtime}:{}", meta.len()),
        },
        messages,
        lossy: false,
    })
}
