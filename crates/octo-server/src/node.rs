//! The node service: identity, membership, relay link, IK sessions, stream muxes and the
//! pairing state machine. One actor task owns the relay handle and the sessions; HTTP handlers
//! talk to it through commands. Tunneled requests are dispatched into the local router in-process.
use crate::policy::{self, TunnelReq};
use crate::tunnel::{Framed, ReqHead, RespHead};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderName, HeaderValue, Method, Request, Response, StatusCode};
use octo_core::Hub;
use octo_core::files::private::{Action, SyncEntry, plan};
use octo_core::nodes::{NodeInfo, PairingInvite, PairingPending, TrustLevel};
use octo_core::settings::NetSettings;
use octo_net::NetError;
use octo_net::identity::{NodeId, NodeKey};
use octo_net::membership::{Membership, Trust};
use octo_net::mux::{Mux, Stream};
use octo_net::pairing::{self, PAIR_TTL, PairHost, PairingCode};
use octo_net::relay_client::{Event, RelayClientConfig, RelayHandle};
use octo_net::session::{Handled, Sessions};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_stream::StreamExt;
use tower::ServiceExt;

/// Request headers forwarded through the tunnel (the bearer token is deliberately absent).
const REQ_HEADERS: [&str; 2] = ["content-type", "accept"];
const RESP_HEADERS: [&str; 5] = [
    "content-type",
    "cache-control",
    "content-disposition",
    "x-content-type-options",
    "content-security-policy",
];
/// Largest request body accepted through the tunnel (uploads are limited to 50 MB).
const MAX_BODY: usize = 51 * 1024 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_NAME: &str = "octo";

#[derive(Debug, Clone)]
pub struct NodeOpts {
    /// How long a request waits for a session with the target node.
    pub connect_timeout: Duration,
}

impl Default for NodeOpts {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("relay_url is not configured")]
    NoRelay,
    #[error("a pairing is already in progress")]
    Busy,
    #[error("no pairing awaiting confirmation")]
    NoPending,
    #[error("invalid pairing code")]
    BadCode,
    #[error("unknown node")]
    UnknownNode,
    #[error("invalid path")]
    BadPath,
    #[error("node unreachable")]
    Unreachable,
    #[error("{0}")]
    Net(#[from] NetError),
}

type OpenReq = oneshot::Sender<Result<Stream, NetError>>;
type Link = mpsc::Sender<OpenReq>;

enum Cmd {
    Ensure(NodeId, oneshot::Sender<Result<Link, NetError>>),
    /// Drops the session; with a link, only if that link is still the current one (a session
    /// established since the failure is left alone).
    Reset(NodeId, Option<Link>),
    Reconfigure,
    Control(NodeId, Vec<u8>),
    /// Tell established full members that private files changed here.
    Hint,
}

enum Pair {
    Idle,
    Starting,
    Inviting,
    Joining,
    /// Decision taken; the exchange is being completed.
    Finishing,
    Pending {
        sas: String,
        peer_name: String,
        decide: Option<oneshot::Sender<(bool, Trust)>>,
    },
}

struct PairSlot {
    gen_: u64,
    st: Pair,
    abort: Option<AbortHandle>,
}

struct Shared {
    key: NodeKey,
    members: RwLock<Membership>,
    hub: Arc<Hub>,
    opts: NodeOpts,
    router: OnceLock<Router>,
    pair: Mutex<PairSlot>,
    cmd: mpsc::Sender<Cmd>,
    ts_path: std::path::PathBuf,
    hint: tokio::sync::Notify,
    sync: Mutex<HashMap<NodeId, SyncSlot>>,
}

/// At most one sync run per peer; `dirty` requests another run after the current one.
#[derive(Default)]
struct SyncSlot {
    running: bool,
    dirty: bool,
}

pub struct Node {
    sh: Arc<Shared>,
    task: JoinHandle<()>,
    hint_task: JoinHandle<()>,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.task.abort();
        self.hint_task.abort();
        if let Ok(p) = self.sh.pair.lock()
            && let Some(a) = &p.abort
        {
            a.abort();
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Shared {
    fn membership(&self) -> Membership {
        self.members
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn is_member(&self, id: &NodeId) -> bool {
        self.members
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .is_some()
    }

    fn trust_of(&self, id: &NodeId) -> Option<Trust> {
        self.members
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .trust_of(id)
    }

    fn net(&self) -> NetSettings {
        self.hub.watch_net().borrow().clone()
    }

    fn name(&self) -> String {
        self.net().node_name.unwrap_or_else(|| DEFAULT_NAME.into())
    }

    fn pair(&self) -> std::sync::MutexGuard<'_, PairSlot> {
        self.pair.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn ensure(&self, peer: NodeId) -> Result<Link, NodeError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::Ensure(peer, tx))
            .await
            .map_err(|_| NodeError::Unreachable)?;
        match tokio::time::timeout(self.opts.connect_timeout, rx).await {
            Ok(Ok(Ok(l))) => Ok(l),
            _ => Err(NodeError::Unreachable),
        }
    }

    async fn open(&self, peer: NodeId) -> Result<(Stream, Link), NodeError> {
        let link = self.ensure(peer).await?;
        let (tx, rx) = oneshot::channel();
        link.send(tx).await.map_err(|_| NodeError::Unreachable)?;
        match tokio::time::timeout(self.opts.connect_timeout, rx).await {
            Ok(Ok(Ok(s))) => Ok((s, link)),
            _ => Err(NodeError::Unreachable),
        }
    }

    fn finish_pair(&self, gen_: u64) {
        let mut p = self.pair();
        if p.gen_ == gen_ {
            p.st = Pair::Idle;
            p.abort = None;
        }
    }
}

/// The id of this node (derived from `keys/node.key`, created if missing).
pub fn node_id(data_dir: &Path) -> Result<String, NetError> {
    Ok(
        NodeKey::load_or_create(&data_dir.join("keys").join("node.key"))?
            .node_id()
            .to_string(),
    )
}

impl Node {
    /// Loads (or creates) identity and membership under `data_dir` and starts the actor.
    /// Needs a tokio runtime.
    pub fn start(hub: Arc<Hub>, data_dir: &Path, opts: NodeOpts) -> Result<Arc<Self>, NetError> {
        let key = NodeKey::load_or_create(&data_dir.join("keys").join("node.key"))?;
        let members = Membership::load_or_create(&data_dir.join("membership.json"))?;
        let ts_path = data_dir.join("keys").join("ik_ts.json");
        let sessions = Sessions::with_store(key.clone(), &ts_path)?;
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let sh = Arc::new(Shared {
            key,
            members: RwLock::new(members),
            hub,
            opts,
            router: OnceLock::new(),
            pair: Mutex::new(PairSlot {
                gen_: 0,
                st: Pair::Idle,
                abort: None,
            }),
            cmd: cmd_tx,
            ts_path,
            hint: tokio::sync::Notify::new(),
            sync: Mutex::new(HashMap::new()),
        });
        let task = tokio::spawn(run(sh.clone(), sessions, cmd_rx));
        let hint_task = tokio::spawn(debounce_hints(sh.clone()));
        Ok(Arc::new(Self {
            sh,
            task,
            hint_task,
        }))
    }

    /// Private files changed locally: full members are told to pull (debounced).
    pub fn private_changed(&self) {
        self.sh.hint.notify_one();
    }

    /// The router that tunneled requests are dispatched into (set once by the server).
    pub fn set_router(&self, r: Router) {
        let _ = self.sh.router.set(r);
    }

    pub fn node_id(&self) -> NodeId {
        self.sh.key.node_id()
    }

    /// Self plus members; online = reachable right now (session + ping).
    pub async fn list(&self) -> Vec<NodeInfo> {
        let m = self.sh.membership();
        let mut out = vec![NodeInfo {
            node_id: self.node_id().to_string(),
            name: self.sh.name(),
            is_self: true,
            online: true,
            trust: None,
        }];
        let probes = m.members().iter().map(|mem| {
            let sh = self.sh.clone();
            let id = mem.node_id;
            tokio::spawn(async move { ping(&sh, id).await })
        });
        let handles: Vec<_> = probes.collect();
        for (mem, h) in m.members().iter().zip(handles) {
            out.push(NodeInfo {
                node_id: mem.node_id.to_string(),
                name: mem.name.clone(),
                is_self: false,
                online: h.await.unwrap_or(false),
                trust: Some(match mem.trust {
                    Trust::Full => TrustLevel::Full,
                    Trust::Public => TrustLevel::Public,
                }),
            });
        }
        out
    }

    // ------------------------------------------------------------ forwarding

    /// Forwards a request to `id` (a member, or this node) and streams the response back.
    pub async fn forward(
        &self,
        id: &str,
        rest: &str,
        req: Request<Body>,
    ) -> Result<Response<Body>, NodeError> {
        let peer = NodeId::parse(id).map_err(|_| NodeError::UnknownNode)?;
        if rest.split('/').any(|s| s == ".." || s == ".") {
            return Err(NodeError::BadPath);
        }
        let (parts, body) = req.into_parts();
        let uri = match parts.uri.query() {
            Some(q) => format!("/api/{rest}?{q}"),
            None => format!("/api/{rest}"),
        };
        let headers = REQ_HEADERS
            .iter()
            .filter_map(|n| {
                let v = parts.headers.get(*n)?.to_str().ok()?;
                Some(((*n).to_string(), v.to_string()))
            })
            .collect();
        let head = ReqHead {
            method: parts.method.to_string(),
            uri,
            headers,
        };
        let body = axum::body::to_bytes(body, MAX_BODY)
            .await
            .map_err(|_| NodeError::BadPath)?;
        if peer == self.node_id() {
            return Ok(dispatch(&self.sh, Trust::Full, head, body).await);
        }
        if !self.sh.is_member(&peer) {
            return Err(NodeError::UnknownNode);
        }
        let (stream, link) = self.sh.open(peer).await?;
        let mut f = Framed::new(stream);
        let sent = async {
            f.send_head(&head).await?;
            f.write(&body).await?;
            f.finish().await
        }
        .await;
        if sent.is_err() {
            let _ = self.sh.cmd.send(Cmd::Reset(peer, Some(link))).await;
            return Err(NodeError::Unreachable);
        }
        let resp: RespHead = match tokio::time::timeout(HEAD_TIMEOUT, f.read_head()).await {
            Ok(Ok(h)) => h,
            _ => {
                let _ = self.sh.cmd.send(Cmd::Reset(peer, Some(link))).await;
                return Err(NodeError::Unreachable);
            }
        };
        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
        tokio::spawn(async move {
            loop {
                match f.next_body().await {
                    Ok(Some(b)) => {
                        if tx.send(Ok(b.into())).await.is_err() {
                            return; // client went away: dropping the stream resets it
                        }
                    }
                    Ok(None) => return,
                    Err(_) => {
                        let _ = tx.send(Err(std::io::Error::other("tunnel reset"))).await;
                        return;
                    }
                }
            }
        });
        let mut b = Response::builder()
            .status(StatusCode::from_u16(resp.status).unwrap_or(StatusCode::BAD_GATEWAY));
        for (k, v) in &resp.headers {
            if RESP_HEADERS.contains(&k.as_str()) {
                b = b.header(k, v);
            }
        }
        b.body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .map_err(|_| NodeError::Unreachable)
    }

    // ------------------------------------------------------------ trust

    /// Sends the current private key to `to` over IK.
    async fn send_private(&self, to: &[NodeId], epoch: u64, key: [u8; 32]) {
        let msg = pairing::private_key_message(epoch, &key);
        for id in to {
            let _ = self.sh.cmd.send(Cmd::Control(*id, msg.clone())).await;
        }
    }

    /// Changes a member's trust (local policy). Full -> public rotates the private key and sends
    /// the new one to the remaining full members; public -> full sends the current key.
    pub async fn set_trust(&self, id: &str, trust: Trust) -> Result<(), NodeError> {
        let peer = NodeId::parse(id).map_err(|_| NodeError::UnknownNode)?;
        let (epoch, key, to) = {
            let mut g = self.sh.members.write().unwrap_or_else(|e| e.into_inner());
            let old = g.trust_of(&peer).ok_or(NodeError::UnknownNode)?;
            if old == trust {
                return Ok(());
            }
            g.set_trust(&peer, trust)?;
            if trust == Trust::Public {
                g.rotate_private_key()?;
            }
            let to = if trust == Trust::Public {
                full_members(&g)
            } else {
                vec![peer]
            };
            (
                g.private_epoch(),
                g.private_key().map(|k| *k.as_bytes()),
                to,
            )
        };
        if let Some(key) = key {
            self.send_private(&to, epoch, key).await;
        }
        Ok(())
    }

    /// Removes a member and rotates the private key for the remaining full members.
    pub async fn remove(&self, id: &str) -> Result<(), NodeError> {
        let peer = NodeId::parse(id).map_err(|_| NodeError::UnknownNode)?;
        let (epoch, key, to) = {
            let mut g = self.sh.members.write().unwrap_or_else(|e| e.into_inner());
            if !g.remove(&peer)? {
                return Err(NodeError::UnknownNode);
            }
            g.rotate_private_key()?;
            (
                g.private_epoch(),
                g.private_key().map(|k| *k.as_bytes()),
                full_members(&g),
            )
        };
        let _ = self.sh.cmd.send(Cmd::Reset(peer, None)).await;
        let _ = self.sh.cmd.send(Cmd::Reconfigure).await;
        if let Some(key) = key {
            self.send_private(&to, epoch, key).await;
        }
        Ok(())
    }

    // ------------------------------------------------------------ pairing

    fn reserve(&self, st: Pair) -> Result<u64, NodeError> {
        let mut p = self.sh.pair();
        if !matches!(p.st, Pair::Idle) {
            return Err(NodeError::Busy);
        }
        p.gen_ += 1;
        p.st = st;
        Ok(p.gen_)
    }

    fn release(&self, gen_: u64) {
        self.sh.finish_pair(gen_);
    }

    fn relay_url(&self) -> Result<String, NodeError> {
        self.sh.net().relay_url.ok_or(NodeError::NoRelay)
    }

    pub async fn invite(&self) -> Result<PairingInvite, NodeError> {
        let url = self.relay_url()?;
        let gen_ = self.reserve(Pair::Starting)?;
        let mut host = match PairHost::start(&url, &self.sh.key, &self.sh.name(), PAIR_TTL).await {
            Ok(h) => h,
            Err(e) => {
                self.release(gen_);
                return Err(e.into());
            }
        };
        let code = host.code().to_string();
        let sh = self.sh.clone();
        let task = tokio::spawn(async move {
            host_flow(&sh, gen_, &mut host).await;
            sh.finish_pair(gen_);
        });
        let mut p = self.sh.pair();
        if p.gen_ == gen_ {
            p.st = Pair::Inviting;
            p.abort = Some(task.abort_handle());
        }
        Ok(PairingInvite {
            code,
            expires_at: now_secs() + PAIR_TTL.as_secs(),
        })
    }

    pub fn join(&self, code: &str, name: &str) -> Result<(), NodeError> {
        let code = PairingCode::parse(code).map_err(|_| NodeError::BadCode)?;
        let name: String = name.trim().chars().take(64).collect();
        let name = if name.is_empty() {
            self.sh.name()
        } else {
            name
        };
        let gen_ = self.reserve(Pair::Joining)?;
        let sh = self.sh.clone();
        let task = tokio::spawn(async move {
            join_flow(&sh, gen_, code, name).await;
            sh.finish_pair(gen_);
        });
        let mut p = self.sh.pair();
        if p.gen_ == gen_ {
            p.abort = Some(task.abort_handle());
        }
        Ok(())
    }

    pub fn pending(&self) -> Option<PairingPending> {
        match &self.sh.pair().st {
            Pair::Pending { sas, peer_name, .. } => Some(PairingPending {
                sas: sas.clone(),
                peer_name: peer_name.clone(),
            }),
            _ => None,
        }
    }

    /// `trust_private` is the host user's choice for the new device (ignored on the joiner).
    pub fn decide(&self, accept: bool, trust_private: bool) -> Result<(), NodeError> {
        let mut p = self.sh.pair();
        let Pair::Pending { decide, .. } = &mut p.st else {
            return Err(NodeError::NoPending);
        };
        let tx = decide.take().ok_or(NodeError::NoPending)?;
        p.st = Pair::Finishing;
        let trust = if trust_private {
            Trust::Full
        } else {
            Trust::Public
        };
        tx.send((accept, trust)).map_err(|_| NodeError::NoPending)
    }
}

fn full_members(m: &Membership) -> Vec<NodeId> {
    m.members()
        .iter()
        .filter(|x| x.trust == Trust::Full)
        .map(|x| x.node_id)
        .collect()
}

async fn ping(sh: &Arc<Shared>, peer: NodeId) -> bool {
    let Ok((stream, link)) = sh.open(peer).await else {
        return false;
    };
    let probe = async {
        let mut f = Framed::new(stream);
        f.send_head(&ReqHead {
            method: "PING".into(),
            uri: String::new(),
            headers: vec![],
        })
        .await
        .ok()?;
        f.finish().await.ok()?;
        let r: RespHead = f.read_head().await.ok()?;
        Some(r.status == 204)
    };
    match tokio::time::timeout(PING_TIMEOUT, probe).await {
        Ok(Some(true)) => true,
        _ => {
            let _ = sh.cmd.send(Cmd::Reset(peer, Some(link))).await;
            false
        }
    }
}

// ---------------------------------------------------------------- pairing flows

fn set_pending(
    sh: &Shared,
    gen_: u64,
    sas: &str,
    peer_name: &str,
) -> oneshot::Receiver<(bool, Trust)> {
    let (tx, rx) = oneshot::channel();
    let mut p = sh.pair();
    if p.gen_ == gen_ {
        p.st = Pair::Pending {
            sas: sas.to_owned(),
            peer_name: peer_name.to_owned(),
            decide: Some(tx),
        };
    }
    rx
}

async fn host_flow(sh: &Arc<Shared>, gen_: u64, host: &mut PairHost) {
    let Ok(pending) = host.wait_peer().await else {
        return;
    };
    let rx = set_pending(sh, gen_, pending.sas(), pending.peer_name());
    let (accept, trust) = match tokio::time::timeout(PAIR_TTL, rx).await {
        Ok(Ok((a, t))) => (a, t),
        _ => (false, Trust::Public),
    };
    let mut m = sh.membership();
    if !accept {
        let _ = tokio::time::timeout(Duration::from_secs(2), pending.finish(false, trust, &mut m))
            .await;
        return;
    }
    let before: Vec<NodeId> = m.members().iter().map(|x| x.node_id).collect();
    let Ok(member) = pending.finish(true, trust, &mut m).await else {
        return;
    };
    *sh.members.write().unwrap_or_else(|e| e.into_inner()) = m;
    let _ = sh.cmd.send(Cmd::Reconfigure).await;
    let msg = pairing::announcement(&member);
    for id in before {
        let _ = sh.cmd.send(Cmd::Control(id, msg.clone())).await;
    }
}

async fn join_flow(sh: &Arc<Shared>, gen_: u64, code: PairingCode, name: String) {
    let Ok(jp) = pairing::join(&code, &sh.key, &name, PAIR_TTL).await else {
        return;
    };
    let rx = set_pending(sh, gen_, jp.sas(), &code.token.host.to_string());
    let accept = matches!(tokio::time::timeout(PAIR_TTL, rx).await, Ok(Ok((true, _))));
    let mut m = sh.membership();
    if jp.finish(accept, &mut m).await.is_ok() {
        *sh.members.write().unwrap_or_else(|e| e.into_inner()) = m;
        let _ = sh.cmd.send(Cmd::Reconfigure).await;
    }
}

// ---------------------------------------------------------------- tunnel target side

fn reply(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({"error": msg}).to_string()))
        .unwrap_or_default()
}

/// Routes reachable through the tunnel (allow-list by first path segment after `/api/`):
/// threads (incl. messages, cancel), providers (incl. models), files, import, sync. Everything else
/// (settings, pairing, nodes, unknown paths) is denied, so new routes are private by default.
const TUNNEL_ROUTES: [&str; 5] = ["threads", "providers", "files", "import", "sync"];

fn path_allowed(uri: &str) -> bool {
    let path = uri.split('?').next().unwrap_or("");
    let Some(rest) = path.strip_prefix("/api/") else {
        return false;
    };
    let mut segs = rest.split('/');
    let first_ok = segs.next().is_some_and(|f| TUNNEL_ROUTES.contains(&f));
    first_ok && segs.all(|s| s != ".." && s != "." && !s.contains('%'))
}

/// Runs a request against the local router (no TCP, no bearer token).
async fn dispatch(sh: &Shared, trust: Trust, head: ReqHead, body: Bytes) -> Response<Body> {
    if !path_allowed(&head.uri) {
        return reply(StatusCode::FORBIDDEN, "not available through the tunnel");
    }
    let thread_space = match policy::thread_id(&head.uri) {
        Some(id) => {
            let hub = sh.hub.clone();
            let id = id.to_owned();
            tokio::task::spawn_blocking(move || hub.thread_space(&id))
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
        }
        None => None,
    };
    let req = TunnelReq {
        method: &head.method,
        path: &head.uri,
        thread_space,
        body_private: policy::body_touches_private(&body),
    };
    if !policy::tunnel_allows(trust, &req) {
        return reply(StatusCode::FORBIDDEN, "private space is not available");
    }
    let filter_list = trust != Trust::Full
        && head.method == "GET"
        && head.uri.split('?').next() == Some("/api/threads");
    let Ok(method) = Method::from_bytes(head.method.as_bytes()) else {
        return reply(StatusCode::BAD_REQUEST, "bad method");
    };
    let mut b = Request::builder().method(method).uri(&head.uri);
    for (k, v) in &head.headers {
        if REQ_HEADERS.contains(&k.to_ascii_lowercase().as_str())
            && let (Ok(k), Ok(v)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            )
        {
            b = b.header(k, v);
        }
    }
    let Ok(req) = b.body(Body::from(body)) else {
        return reply(StatusCode::BAD_REQUEST, "bad request");
    };
    let Some(router) = sh.router.get() else {
        return reply(StatusCode::SERVICE_UNAVAILABLE, "not ready");
    };
    let resp = match router.clone().oneshot(req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if !filter_list || !resp.status().is_success() {
        return resp;
    }
    // Thread list for a public-trust member: private threads never leave this node.
    let (mut parts, body) = resp.into_parts();
    let filtered = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(b) => policy::filter_thread_list(&b),
        Err(_) => None,
    };
    match filtered {
        Some(out) => {
            parts.headers.remove("content-length");
            Response::from_parts(parts, Body::from(out))
        }
        None => reply(StatusCode::FORBIDDEN, "private space is not available"),
    }
}

async fn serve_stream(sh: Arc<Shared>, peer: NodeId, stream: Stream) {
    let mut f = Framed::new(stream);
    let Ok(head) = f.read_head::<ReqHead>().await else {
        return;
    };
    let respond = |status: u16| RespHead {
        status,
        headers: vec![],
    };
    let Some(trust) = sh.trust_of(&peer) else {
        let _ = f.send_head(&respond(403)).await;
        let _ = f.finish().await;
        return;
    };
    if head.method == "PING" {
        let _ = f.send_head(&respond(204)).await;
        let _ = f.finish().await;
        return;
    }
    let mut body = Vec::new();
    loop {
        match f.next_body().await {
            Ok(Some(c)) => {
                body.extend_from_slice(&c);
                if body.len() > MAX_BODY {
                    let _ = f.send_head(&respond(413)).await;
                    let _ = f.finish().await;
                    return;
                }
            }
            Ok(None) => break,
            Err(_) => return,
        }
    }
    let resp = dispatch(&sh, trust, head, body.into()).await;
    let out = RespHead {
        status: resp.status().as_u16(),
        headers: resp
            .headers()
            .iter()
            .filter(|(k, _)| RESP_HEADERS.contains(&k.as_str()))
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
            .collect(),
    };
    if f.send_head(&out).await.is_err() {
        return;
    }
    let mut data = resp.into_body().into_data_stream();
    while let Some(chunk) = data.next().await {
        match chunk {
            Ok(c) => {
                if f.write(&c).await.is_err() {
                    return;
                }
            }
            Err(_) => {
                f.reset().await;
                return;
            }
        }
    }
    let _ = f.finish().await;
}

// ---------------------------------------------------------------- actor

const CONNECT_RETRY: Duration = Duration::from_secs(3);
const PLAIN_MUX: u8 = 0;
const PLAIN_CTL: u8 = 1;

struct PeerEntry {
    in_tx: mpsc::Sender<Vec<u8>>,
    link: Link,
    task: JoinHandle<()>,
}

struct Actor {
    sh: Arc<Shared>,
    sessions: Sessions,
    relay: Option<RelayHandle>,
    current: Option<(String, [u8; 32])>,
    peers: HashMap<NodeId, PeerEntry>,
    waiters: HashMap<NodeId, Vec<oneshot::Sender<Result<Link, NetError>>>>,
    ctl: HashMap<NodeId, Vec<Vec<u8>>>,
    connecting: HashMap<NodeId, Instant>,
    out_tx: mpsc::Sender<(NodeId, Vec<u8>)>,
}

async fn next_event(r: &mut Option<RelayHandle>) -> Option<Event> {
    match r {
        Some(h) => h.recv().await,
        None => std::future::pending().await,
    }
}

async fn run(sh: Arc<Shared>, sessions: Sessions, mut cmd_rx: mpsc::Receiver<Cmd>) {
    let (out_tx, mut out_rx) = mpsc::channel::<(NodeId, Vec<u8>)>(256);
    let mut net_rx = sh.hub.watch_net();
    let mut a = Actor {
        sh,
        sessions,
        relay: None,
        current: None,
        peers: HashMap::new(),
        waiters: HashMap::new(),
        ctl: HashMap::new(),
        connecting: HashMap::new(),
        out_tx,
    };
    a.reconfigure();
    let mut net_open = true;
    let mut periodic = tokio::time::interval(SYNC_EVERY);
    periodic.tick().await;
    loop {
        tokio::select! {
            ev = next_event(&mut a.relay) => match ev {
                Some(Event::Frame { from, payload }) => a.on_frame(from, payload).await,
                Some(Event::Connected) => a.on_connected().await,
                Some(_) => {}
                None => a.relay = None,
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(c) => a.on_cmd(c).await,
                None => return,
            },
            Some((peer, plain)) = out_rx.recv() => a.send_plain(peer, &plain).await,
            _ = periodic.tick() => {
                let online: Vec<NodeId> = a.peers.keys().copied().collect();
                for p in online {
                    trigger_sync(&a.sh, p);
                }
            }
            r = net_rx.changed(), if net_open => {
                if r.is_err() { net_open = false } else { a.reconfigure() }
            }
        }
    }
}

impl Actor {
    fn reconfigure(&mut self) {
        let net = self.sh.net();
        let m = self.sh.membership();
        let want = net
            .relay_url
            .filter(|_| !m.members().is_empty())
            .map(|u| (u, m.network_id()));
        if want == self.current {
            return;
        }
        for (_, p) in self.peers.drain() {
            p.task.abort();
        }
        self.waiters.clear();
        self.connecting.clear();
        self.relay = None;
        self.current = want.clone();
        if let Ok(s) = Sessions::with_store(self.sh.key.clone(), &self.sh.ts_path) {
            self.sessions = s;
        }
        if let Some((url, network_id)) = want {
            self.relay = Some(RelayHandle::connect(RelayClientConfig {
                url,
                key: self.sh.key.clone(),
                network_id,
                backoff_base: Duration::from_millis(250),
                backoff_max: Duration::from_secs(10),
            }));
        }
    }

    async fn send(&self, to: NodeId, frame: Vec<u8>) {
        if let Some(r) = &self.relay {
            let _ = r.send(to, frame).await;
        }
    }

    async fn send_plain(&mut self, peer: NodeId, plain: &[u8]) {
        if let Ok(f) = self.sessions.encrypt(peer, plain) {
            self.send(peer, f).await;
        }
    }

    async fn start_connect(&mut self, peer: NodeId) {
        let m = self.sh.membership();
        if let Ok(f) = self.sessions.connect(&m, peer) {
            self.connecting.insert(peer, Instant::now());
            self.send(peer, f).await;
        }
    }

    async fn on_connected(&mut self) {
        for p in full_members(&self.sh.membership()) {
            trigger_sync(&self.sh, p);
        }
        let want: Vec<NodeId> = self
            .waiters
            .keys()
            .chain(self.ctl.keys())
            .filter(|p| !self.sessions.is_established(p))
            .copied()
            .collect();
        for p in want {
            self.start_connect(p).await;
        }
    }

    async fn on_cmd(&mut self, c: Cmd) {
        match c {
            Cmd::Reconfigure => self.reconfigure(),
            Cmd::Reset(peer, link) => {
                if let (Some(l), Some(p)) = (&link, self.peers.get(&peer))
                    && !p.link.same_channel(l)
                {
                    return;
                }
                self.sessions.reset(&peer);
                if let Some(p) = self.peers.remove(&peer) {
                    p.task.abort();
                }
                self.connecting.remove(&peer);
            }
            Cmd::Ensure(peer, reply) => {
                if let (Some(p), true) =
                    (self.peers.get(&peer), self.sessions.is_established(&peer))
                {
                    let _ = reply.send(Ok(p.link.clone()));
                    return;
                }
                if self.relay.is_none() || !self.sh.is_member(&peer) {
                    let _ = reply.send(Err(NetError::Offline));
                    return;
                }
                let w = self.waiters.entry(peer).or_default();
                w.retain(|s| !s.is_closed());
                w.push(reply);
                self.maybe_connect(peer).await;
            }
            Cmd::Hint => {
                let hint =
                    serde_json::to_vec(&serde_json::json!({"t": "sync_hint"})).unwrap_or_default();
                let to: Vec<NodeId> = full_members(&self.sh.membership())
                    .into_iter()
                    .filter(|p| self.sessions.is_established(p))
                    .collect();
                for p in to {
                    self.send_ctl(p, &hint).await;
                }
            }
            Cmd::Control(peer, bytes) => {
                if self.relay.is_none() || !self.sh.is_member(&peer) {
                    return;
                }
                if self.sessions.is_established(&peer) {
                    self.send_ctl(peer, &bytes).await;
                } else {
                    self.ctl.entry(peer).or_default().push(bytes);
                    self.maybe_connect(peer).await;
                }
            }
        }
    }

    async fn maybe_connect(&mut self, peer: NodeId) {
        let stale = self
            .connecting
            .get(&peer)
            .is_none_or(|t| t.elapsed() > CONNECT_RETRY);
        if stale && self.relay.as_ref().is_some_and(|r| r.is_online()) {
            self.start_connect(peer).await;
        }
    }

    async fn send_ctl(&mut self, peer: NodeId, bytes: &[u8]) {
        let plain = [&[PLAIN_CTL][..], bytes].concat();
        self.send_plain(peer, &plain).await;
    }

    async fn on_frame(&mut self, from: NodeId, payload: Vec<u8>) {
        let m = self.sh.membership();
        match self.sessions.handle(&m, from, &payload) {
            Ok(Handled::Reply(f)) => {
                self.send(from, f).await;
                self.open_peer(from).await;
            }
            Ok(Handled::Established) => self.open_peer(from).await,
            Ok(Handled::Data(d)) => match d.split_first() {
                Some((&PLAIN_MUX, rest)) => {
                    if let Some(p) = self.peers.get(&from) {
                        let _ = p.in_tx.try_send(rest.to_vec());
                    }
                }
                Some((&PLAIN_CTL, rest)) => self.on_ctl(from, rest).await,
                _ => {}
            },
            Ok(Handled::Ignored) => {}
            Ok(Handled::Rehandshake(f)) => {
                if let Some(p) = self.peers.remove(&from) {
                    p.task.abort();
                }
                self.send(from, f).await;
            }
            Err(_) => {}
        }
    }

    async fn on_ctl(&mut self, from: NodeId, bytes: &[u8]) {
        let msg: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
        let full = self.sh.trust_of(&from) == Some(Trust::Full);
        match msg.get("t").and_then(|t| t.as_str()) {
            Some("sync_hint") => {
                if full {
                    trigger_sync(&self.sh, from);
                }
            }
            Some("private_epoch") => {
                let theirs = msg.get("epoch").and_then(|e| e.as_u64()).unwrap_or(0);
                let (epoch, key) = {
                    let g = self.sh.members.read().unwrap_or_else(|e| e.into_inner());
                    (g.private_epoch(), g.private_key().map(|k| *k.as_bytes()))
                };
                if let (true, true, Some(key)) = (full, epoch > theirs, key) {
                    self.send_ctl(from, &pairing::private_key_message(epoch, &key))
                        .await;
                }
            }
            _ => {
                let mut g = self.sh.members.write().unwrap_or_else(|e| e.into_inner());
                let _ = pairing::apply_control(&mut g, &from, bytes);
            }
        }
    }

    /// A fresh session replaces the mux (and with it every stream) of the peer.
    async fn open_peer(&mut self, peer: NodeId) {
        self.connecting.remove(&peer);
        if let Some(old) = self.peers.remove(&peer) {
            old.task.abort();
        }
        let (mux_out_tx, mux_out_rx) = mpsc::channel(64);
        let (in_tx, in_rx) = mpsc::channel(4096);
        let (open_tx, open_rx) = mpsc::channel(16);
        let initiator = self.sh.key.node_id() < peer;
        let mux = Mux::new(mux_out_tx, in_rx, initiator);
        let task = tokio::spawn(peer_task(
            self.sh.clone(),
            peer,
            mux,
            mux_out_rx,
            open_rx,
            self.out_tx.clone(),
        ));
        for w in self.waiters.remove(&peer).unwrap_or_default() {
            let _ = w.send(Ok(open_tx.clone()));
        }
        self.peers.insert(
            peer,
            PeerEntry {
                in_tx,
                link: open_tx,
                task,
            },
        );
        for c in self.ctl.remove(&peer).unwrap_or_default() {
            self.send_ctl(peer, &c).await;
        }
        if self.sh.trust_of(&peer) == Some(Trust::Full) {
            let epoch = self.sh.membership().private_epoch();
            let msg =
                serde_json::to_vec(&serde_json::json!({"t": "private_epoch", "epoch": epoch}))
                    .unwrap_or_default();
            self.send_ctl(peer, &msg).await;
            trigger_sync(&self.sh, peer);
        }
    }
}

async fn peer_task(
    sh: Arc<Shared>,
    peer: NodeId,
    mut mux: Mux,
    mut mux_out: mpsc::Receiver<Vec<u8>>,
    mut open_rx: mpsc::Receiver<OpenReq>,
    out: mpsc::Sender<(NodeId, Vec<u8>)>,
) {
    loop {
        tokio::select! {
            Some(f) = mux_out.recv() => {
                let plain = [&[PLAIN_MUX][..], &f].concat();
                if out.send((peer, plain)).await.is_err() {
                    return;
                }
            }
            s = mux.accept() => match s {
                Some(s) => {
                    tokio::spawn(serve_stream(sh.clone(), peer, s));
                }
                None => return,
            },
            Some(req) = open_rx.recv() => {
                let _ = req.send(mux.open().await);
            }
        }
    }
}

// ---------------------------------------------------------------- private files sync

const SYNC_EVERY: Duration = Duration::from_secs(60);
const HINT_DEBOUNCE: Duration = Duration::from_secs(1);
/// Same as the upload limit.
const MAX_BLOB: usize = 50 * 1024 * 1024;
const MAX_MANIFEST: usize = 32 * 1024 * 1024;
const SYNC_RETRIES: u32 = 6;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(10);

async fn debounce_hints(sh: Arc<Shared>) {
    loop {
        sh.hint.notified().await;
        tokio::time::sleep(HINT_DEBOUNCE).await;
        let _ = sh.cmd.send(Cmd::Hint).await;
    }
}

/// Starts a pull from `peer` (full members only) unless one is running; then it runs once more.
fn trigger_sync(sh: &Arc<Shared>, peer: NodeId) {
    if sh.trust_of(&peer) != Some(Trust::Full) {
        return;
    }
    {
        let mut g = sh.sync.lock().unwrap_or_else(|e| e.into_inner());
        let slot = g.entry(peer).or_default();
        if slot.running {
            slot.dirty = true;
            return;
        }
        slot.running = true;
    }
    let sh = sh.clone();
    tokio::spawn(async move {
        let mut failures = 0u32;
        loop {
            let ok = sync_once(&sh, peer).await.is_some();
            // A failed run (session churn, peer restarting) is retried with backoff.
            let retry = !ok && failures < SYNC_RETRIES;
            if retry {
                tokio::time::sleep(Duration::from_secs(1 << failures.min(4))).await;
                failures += 1;
            } else {
                failures = 0;
            }
            let mut g = sh.sync.lock().unwrap_or_else(|e| e.into_inner());
            let slot = g.entry(peer).or_default();
            let full = sh.trust_of(&peer) == Some(Trust::Full);
            if full && (retry || slot.dirty) {
                slot.dirty = false;
            } else {
                *slot = SyncSlot::default();
                return;
            }
        }
    });
}

/// GET `uri` on `peer` through the tunnel; the body is capped at `cap` bytes. `None` unless 200.
async fn peer_get(sh: &Arc<Shared>, peer: NodeId, uri: &str, cap: usize) -> Option<Vec<u8>> {
    let (stream, link) = sh.open(peer).await.ok()?;
    let r = read_response(stream, uri, cap).await;
    if matches!(r, Err(true)) {
        let _ = sh.cmd.send(Cmd::Reset(peer, Some(link))).await;
    }
    r.ok()
}

/// `Err(true)` = transport failure (the session may be dead), `Err(false)` = refused or too big.
async fn read_response(stream: Stream, uri: &str, cap: usize) -> Result<Vec<u8>, bool> {
    let mut f = Framed::new(stream);
    let head = ReqHead {
        method: "GET".into(),
        uri: uri.into(),
        headers: vec![],
    };
    f.send_head(&head).await.map_err(|_| true)?;
    f.finish().await.map_err(|_| true)?;
    let resp: RespHead = tokio::time::timeout(HEAD_TIMEOUT, f.read_head())
        .await
        .map_err(|_| true)?
        .map_err(|_| true)?;
    if resp.status != 200 {
        return Err(false);
    }
    let mut out = Vec::new();
    while let Some(c) = tokio::time::timeout(CHUNK_TIMEOUT, f.next_body())
        .await
        .map_err(|_| true)?
        .map_err(|_| true)?
    {
        out.extend_from_slice(&c);
        if out.len() > cap {
            return Err(false);
        }
    }
    Ok(out)
}

async fn apply_entry(
    sh: &Arc<Shared>,
    e: SyncEntry,
    data: Option<Vec<u8>>,
) -> Result<(), octo_core::files::FilesError> {
    let files = sh.hub.files().clone();
    tokio::task::spawn_blocking(move || files.private_apply(&e, data.as_deref()))
        .await
        .unwrap_or(Err(octo_core::files::FilesError::Invalid))
}

/// One pull run: manifest, plan, fetch missing blobs (hash-verified by `apply`), apply.
async fn sync_once(sh: &Arc<Shared>, peer: NodeId) -> Option<()> {
    let remote: Vec<SyncEntry> = serde_json::from_slice(
        &peer_get(sh, peer, "/api/sync/private/manifest", MAX_MANIFEST).await?,
    )
    .ok()?;
    let files = sh.hub.files().clone();
    let local = tokio::task::spawn_blocking(move || files.private_manifest())
        .await
        .ok()?
        .ok()?;
    let mut complete = true;
    for action in plan(&local, &remote) {
        if sh.trust_of(&peer) != Some(Trust::Full) {
            return Some(());
        }
        let (entry, fetch) = match action {
            Action::Fetch(e) => (e, true),
            Action::Adopt(e) => (e, false),
        };
        let first = if fetch {
            Err(octo_core::files::FilesError::NotFound)
        } else {
            apply_entry(sh, entry.clone(), None).await
        };
        // Content not present locally (or gone meanwhile): download it.
        if matches!(first, Err(octo_core::files::FilesError::NotFound))
            && entry.size <= MAX_BLOB as u64
        {
            let uri = format!("/api/sync/private/blob/{}", entry.hash);
            // A failed download is a transient failure; a rejected entry is not retried.
            let data = peer_get(sh, peer, &uri, MAX_BLOB).await;
            match data {
                Some(data) => {
                    let _ = apply_entry(sh, entry, Some(data)).await;
                }
                None => complete = false,
            }
        }
    }
    complete.then_some(())
}
