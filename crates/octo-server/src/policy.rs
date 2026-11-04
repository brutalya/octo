//! Authorization of tunneled requests on the target node, by the requesting member's trust.
//! Full-trust members may use everything the tunnel allow-list exposes; public-trust members
//! never touch the private space (threads, files, attachments, creation, import).
use octo_core::storage::Space;
use octo_net::membership::Trust;
use serde_json::Value;

/// What the policy needs to know about a request (resolved by the caller from the path, the
/// thread store and the body).
pub struct TunnelReq<'a> {
    pub method: &'a str,
    /// Path with optional query, e.g. `/api/threads/abc/messages`.
    pub path: &'a str,
    /// Space of the thread addressed by the path, if any and if it exists.
    pub thread_space: Option<Space>,
    /// The body names the private space (new thread, import target, attachment).
    pub body_private: bool,
}

fn segments(path: &str) -> Vec<&str> {
    let p = path.split('?').next().unwrap_or("");
    p.strip_prefix("/api/")
        .map(|r| r.split('/').collect())
        .unwrap_or_default()
}

/// The single decision point: may a member with `trust` make this request?
pub fn tunnel_allows(trust: Trust, req: &TunnelReq) -> bool {
    if trust == Trust::Full {
        return true;
    }
    let _ = req.method;
    let seg = segments(req.path);
    let private_files = seg.first() == Some(&"files") && seg.get(1) == Some(&"private");
    !(private_files || req.thread_space == Some(Space::Private) || req.body_private)
}

/// Id of the thread a path addresses (`/api/threads/{id}[/...]`).
pub fn thread_id(path: &str) -> Option<&str> {
    let seg = segments(path);
    match seg.as_slice() {
        ["threads", id, ..] if !id.is_empty() => Some(id),
        _ => None,
    }
}

/// True if a JSON body names the private space at top level or in any attachment.
pub fn body_touches_private(body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let is_private = |x: &Value| x.get("space").and_then(Value::as_str) == Some("private");
    is_private(&v)
        || v.get("attachments")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(is_private))
}

/// Drops private threads from a `GET /api/threads` body. `None` if the body is not a thread
/// array (it is then refused rather than passed through unfiltered).
pub fn filter_thread_list(body: &[u8]) -> Option<Vec<u8>> {
    let Value::Array(items) = serde_json::from_slice(body).ok()? else {
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    for t in items {
        match t.get("space").and_then(Value::as_str)? {
            "private" => {}
            "public" => out.push(t),
            _ => return None,
        }
    }
    serde_json::to_vec(&out).ok()
}
