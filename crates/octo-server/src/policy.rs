//! Authorization of tunneled requests on the target node, by the requesting member's trust.
//! Full-trust members may use everything the tunnel allow-list exposes; public-trust members
//! never touch the private space (threads, files, attachments, creation, import).
use octo_core::providers::ProviderKind;
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
    /// Provider of the thread addressed by the path, if any and if it exists.
    pub thread_provider: Option<ProviderKind>,
    /// The body names the private space (new thread, import target, attachment).
    pub body_private: bool,
    /// The body picks a CLI provider or sets approval / cwd (see [`body_agent_fields`]).
    pub body_agents: bool,
}

/// Routes reachable through the tunnel (allow-list by first path segment after `/api/`):
/// threads (incl. messages, cancel), providers (incl. models), files, import, sync. Everything
/// else (settings, pairing, nodes, unknown paths) is denied, so new routes are private by default.
const TUNNEL_ROUTES: [&str; 5] = ["threads", "providers", "files", "import", "sync"];

/// Path segments after `/api/` (query dropped); empty if the path is not under `/api/`.
fn segments(path: &str) -> Vec<&str> {
    let p = path.split('?').next().unwrap_or("");
    p.strip_prefix("/api/")
        .map(|r| r.split('/').collect())
        .unwrap_or_default()
}

/// Whether the tunnel exposes this path at all. Percent-escapes are refused outright: the
/// private-space checks below compare raw segments, so `/api/files/%70rivate` must never reach
/// a router that decodes it.
pub fn path_allowed(uri: &str) -> bool {
    let seg = segments(uri);
    seg.first().is_some_and(|f| TUNNEL_ROUTES.contains(f))
        && seg
            .iter()
            .all(|s| *s != ".." && *s != "." && !s.contains('%'))
}

fn is_cli(p: ProviderKind) -> bool {
    !p.is_http()
}

/// The single decision point: may a member with `trust` (and the local `agents` permission)
/// make this request?
pub fn tunnel_allows(trust: Trust, agents: bool, req: &TunnelReq) -> bool {
    let seg = segments(req.path);
    if !agents {
        let cli_thread = req.thread_provider.is_some_and(is_cli);
        let sends = req.method == "POST" && seg.get(2) == Some(&"messages");
        let patches = req.method == "PATCH";
        if seg.first() == Some(&"import") || req.body_agents || (cli_thread && (sends || patches)) {
            return false;
        }
    }
    if trust == Trust::Full {
        return true;
    }
    let private_files = seg.first() == Some(&"files") && seg.get(1) == Some(&"private");
    let sync = seg.first() == Some(&"sync");
    !(private_files || sync || req.thread_space == Some(Space::Private) || req.body_private)
}

/// True if a JSON body picks a CLI provider (`claude`, `antigravity`) or sets `approval` or
/// `cwd` (any non-null value): things that run or steer a local agent.
pub fn body_agent_fields(body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let set = |k: &str| v.get(k).is_some_and(|x| !x.is_null());
    let cli = matches!(
        v.get("provider").and_then(Value::as_str),
        Some("claude" | "antigravity")
    );
    cli || set("approval") || set("cwd")
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
