//! The node service: identity, membership, relay link, IK sessions, stream muxes and the
//! pairing state machine. One actor task owns the relay handle and the sessions; HTTP handlers
//! talk to it through commands. Tunneled requests are dispatched into the local router in-process.
use crate::policy::{self, TunnelReq};
use crate::tunnel::{Framed, ReqHead, RespHead};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderName, HeaderValue, Method, Request, Response, StatusCode};
use octo_core::Hub;
use octo_core::files::FilesError;
use octo_core::files::private::{Action, SyncEntry, plan};
use octo_core::nodes::{
    AccessView, NetworkView, NodeInfo, PairingInvite, PairingPending, PairingStage, PairingState,
    RelayCheck, RelayStatus, TrustLevel,
};
use octo_core::settings::NetSettings;
use octo_net::NetError;
use octo_net::identity::{NodeId, NodeKey};
use octo_net::membership::{Membership, Trust};
use octo_net::mux::{Mux, Stream};
use octo_net::pairing::{self, PAIR_TTL, PairHost, PairingCode};
use octo_net::relay_client::{Event, LinkStatus, RelayClientConfig, RelayHandle};
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
/// Response headers the target puts on the wire (the receiving side sanitizes them again).
const RESP_HEADERS: [&str; 5] = [
    "content-type",
    "cache-control",
    "content-disposition",
    "x-content-type-options",
    "content-security-policy",
];
/// Largest request body accepted through the tunnel (same as the upload limit).
const MAX_BODY: usize = crate::UPLOAD_LIMIT;
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
    #[error("this device is already in a network; leave it first")]
    InNetwork,
    #[error("the relay code must be a ws:// or wss:// address")]
    BadRelay,
    #[error("invalid path")]
    BadPath,
    #[error("request body too large")]
    TooLarge,
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

/// The local user's choice in the SAS dialog: accept, trust level, may run CLI agents here.
type Decision = (bool, Trust, bool);

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
        decide: Option<oneshot::Sender<Decision>>,
    },
}

struct PairSlot {
    gen_: u64,
    st: Pair,
    abort: Option<AbortHandle>,
    /// The code handed out by the running invite (kept until that pairing ends).
    invite: Option<PairingInvite>,
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
    /// Every task spawned on behalf of the node (sync runs, streams, peer tasks, body pumps);
    /// aborted when the node is dropped.
    tasks: Mutex<tokio::task::JoinSet<()>>,
    /// Status view of the actor's current relay link (None while no link is kept).
    link: Mutex<Option<LinkStatus>>,
}

/// At most one sync run per peer; `dirty` requests another run after the current one.
#[derive(Default)]
struct SyncSlot {
    running: bool,
    dirty: bool,
}

pub struct Node {
    sh: Arc<Shared>,
    task: AbortHandle,
    hint_task: JoinHandle<()>,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.task.abort();
        self.hint_task.abort();
        if let Ok(mut t) = self.sh.tasks.lock() {
            t.abort_all();
        }
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

    /// Spawns a task owned by the node: it ends when the node is dropped.
    fn spawn(&self, fut: impl Future<Output = ()> + Send + 'static) -> AbortHandle {
        let mut set = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        while set.try_join_next().is_some() {} // reap finished tasks
        set.spawn(fut)
    }

    fn is_member(&self, id: &NodeId) -> bool {
        self.members
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .is_some()
    }

    fn caller_of(&self, id: &NodeId) -> Option<Caller> {
        self.members
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|m| Caller::Member {
                trust: m.trust,
                agents: m.agents,
            })
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
            p.invite = None;
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
                invite: None,
            }),
            cmd: cmd_tx,
            ts_path,
            hint: tokio::sync::Notify::new(),
            sync: Mutex::new(HashMap::new()),
            tasks: Mutex::new(tokio::task::JoinSet::new()),
            link: Mutex::new(None),
        });
        let actor = tokio::spawn(run(sh.clone(), sessions, cmd_rx));
        let task = actor.abort_handle();
        // An actor that panics would silently leave the node deaf: say so.
        tokio::spawn(async move {
            if let Err(e) = actor.await
                && e.is_panic()
            {
                eprintln!("octo: node actor panicked: {e}");
            }
        });
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
        let fresh = self.sh.router.set(r).is_ok();
        debug_assert!(fresh, "set_router called twice");
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
            agents: None,
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
                agents: Some(mem.agents),
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
            .map_err(|_| NodeError::TooLarge)?;
        if peer == self.node_id() {
            return Ok(dispatch(&self.sh, Caller::Local, head, body).await);
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
        self.sh.spawn(async move {
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
        for (k, v) in policy::sanitize_forwarded_headers(&resp.headers) {
            b = b.header(k, v);
        }
        b.body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .map_err(|_| NodeError::Unreachable)
    }

    // ------------------------------------------------------------ trust

    /// Sends the current private key to `to` over IK.
    async fn send_private(&self, to: &[NodeId], epoch: u64, key: [u8; 32]) {
        let Ok(msg) = pairing::private_key_message(epoch, &key) else {
            return;
        };
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

    /// Changes whether a member may run CLI agents here (local opinion, nothing is sent).
    pub fn set_agents(&self, id: &str, agents: bool) -> Result<(), NodeError> {
        let peer = NodeId::parse(id).map_err(|_| NodeError::UnknownNode)?;
        let mut g = self.sh.members.write().unwrap_or_else(|e| e.into_inner());
        if g.get(&peer).is_none() {
            return Err(NodeError::UnknownNode);
        }
        g.set_agents(&peer, agents)?;
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

    /// Starts an invite, or returns the running one (Settings may be reopened meanwhile).
    pub async fn invite(&self) -> Result<PairingInvite, NodeError> {
        if let Some(i) = self.running_invite() {
            return Ok(i);
        }
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
        // Inviting is set before the task can run: a task that ends at once returns to Idle and
        // nothing afterwards may overwrite that.
        let invite = PairingInvite {
            code,
            expires_at: now_secs() + PAIR_TTL.as_secs(),
        };
        {
            let mut p = self.sh.pair();
            if p.gen_ == gen_ {
                p.st = Pair::Inviting;
                p.invite = Some(invite.clone());
            }
        }
        let task = tokio::spawn(async move {
            host_flow(&sh, gen_, &mut host).await;
            sh.finish_pair(gen_);
        });
        self.store_abort(gen_, &task);
        Ok(invite)
    }

    /// The invite whose code is still waiting to be used.
    fn running_invite(&self) -> Option<PairingInvite> {
        let p = self.sh.pair();
        match p.st {
            Pair::Inviting => p.invite.clone(),
            _ => None,
        }
    }

    pub fn pairing_state(&self) -> PairingState {
        let p = self.sh.pair();
        let stage = match p.st {
            Pair::Idle => PairingStage::Idle,
            Pair::Starting | Pair::Inviting => PairingStage::Inviting,
            Pair::Joining => PairingStage::Joining,
            Pair::Pending { .. } => PairingStage::Confirm,
            Pair::Finishing => PairingStage::Finishing,
        };
        PairingState {
            stage,
            invite: p.invite.clone(),
        }
    }

    /// Ends the running invite or join (its code stops working). Idle: nothing to do.
    pub fn cancel_pairing(&self) {
        let mut p = self.sh.pair();
        if let Some(a) = p.abort.take() {
            a.abort();
        }
        p.gen_ += 1;
        p.st = Pair::Idle;
        p.invite = None;
    }

    pub fn join(&self, code: &str, name: &str) -> Result<(), NodeError> {
        let code = PairingCode::parse(code).map_err(|_| NodeError::BadCode)?;
        // Adopting another network only works while this one has no members; refuse up front
        // instead of letting the host add a device that then never joins.
        if !self.sh.membership().members().is_empty() {
            return Err(NodeError::InNetwork);
        }
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
        self.store_abort(gen_, &task);
        Ok(())
    }

    /// Remembers how to cancel the pairing task, unless it already finished.
    fn store_abort(&self, gen_: u64, task: &JoinHandle<()>) {
        let mut p = self.sh.pair();
        if p.gen_ == gen_ && !matches!(p.st, Pair::Idle) {
            p.abort = Some(task.abort_handle());
        }
    }

    // ------------------------------------------------------------ network

    pub fn network(&self) -> NetworkView {
        let s = self.sh.hub.settings();
        let others = self.sh.membership().members().len();
        let relay_host = s.relay_url.as_deref().and_then(relay_host);
        let joined = s.relay_url.is_some() || others > 0;
        let name = joined.then(|| {
            s.network_name
                .clone()
                .or_else(|| relay_host.clone())
                .unwrap_or_else(|| "Network".into())
        });
        NetworkView {
            joined,
            name,
            relay_host,
            devices: u32::try_from(others + 1).unwrap_or(u32::MAX),
        }
    }

    /// Starts a network on `relay_url` (the relay code) with this device as its first member.
    pub async fn create_network(
        &self,
        relay_url: &str,
        name: &str,
    ) -> Result<NetworkView, NodeError> {
        if !self.sh.membership().members().is_empty() {
            return Err(NodeError::InNetwork);
        }
        let url = relay_url.trim();
        if url.is_empty() {
            return Err(NodeError::BadRelay);
        }
        let patch = octo_core::settings::SettingsPatch {
            relay_url: Some(url.to_string()),
            network_name: Some(name.to_string()),
            ..Default::default()
        };
        self.sh.hub.update_settings(patch).map_err(|e| match e {
            octo_core::CoreError::Settings(octo_core::settings::SettingsError::BadRelayUrl) => {
                NodeError::BadRelay
            }
            e => NodeError::Net(NetError::Relay(e.to_string())),
        })?;
        let _ = self.sh.cmd.send(Cmd::Reconfigure).await;
        Ok(self.network())
    }

    /// Leaves the network: ends any pairing, forgets members (new network and private keys),
    /// the relay and the network name. The other devices see this one go offline.
    pub async fn leave(&self) -> Result<(), NodeError> {
        self.cancel_pairing();
        let peers: Vec<NodeId> = {
            let mut g = self.sh.members.write().unwrap_or_else(|e| e.into_inner());
            let ids = g.members().iter().map(|m| m.node_id).collect();
            g.reset()?;
            ids
        };
        for p in peers {
            let _ = self.sh.cmd.send(Cmd::Reset(p, None)).await;
        }
        let patch = octo_core::settings::SettingsPatch {
            relay_url: Some(String::new()),
            network_name: Some(String::new()),
            ..Default::default()
        };
        self.sh
            .hub
            .update_settings(patch)
            .map_err(|e| NodeError::Net(NetError::Relay(e.to_string())))?;
        let _ = self.sh.cmd.send(Cmd::Reconfigure).await;
        Ok(())
    }

    pub fn relay_status(&self) -> RelayStatus {
        let link = self
            .sh
            .link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        RelayStatus {
            configured: self.sh.net().relay_url.is_some(),
            active: link.is_some(),
            online: link.as_ref().is_some_and(LinkStatus::online),
            error: link.and_then(|l| l.last_error()),
        }
    }

    /// One connection attempt to `url` (default: the configured relay), never touching this
    /// node's own link.
    pub async fn relay_check(&self, url: Option<String>) -> Result<RelayCheck, NodeError> {
        let url = match url {
            Some(u) => u,
            None => self.relay_url()?,
        };
        Ok(
            match octo_net::relay_client::probe(&url, Duration::from_secs(10)).await {
                Ok(()) => RelayCheck {
                    ok: true,
                    error: None,
                },
                Err(e) => RelayCheck {
                    ok: false,
                    error: Some(octo_net::relay_client::describe(&e)),
                },
            },
        )
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

    /// `trust_private` is the host user's choice for the new device (ignored on the joiner);
    /// `allow_agents` is each side's choice for the other device.
    pub fn decide(
        &self,
        accept: bool,
        trust_private: bool,
        allow_agents: bool,
    ) -> Result<(), NodeError> {
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
        tx.send((accept, trust, allow_agents))
            .map_err(|_| NodeError::NoPending)
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

fn set_pending(sh: &Shared, gen_: u64, sas: &str, peer_name: &str) -> oneshot::Receiver<Decision> {
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
    let Ok(mut pending) = host.wait_peer().await else {
        return;
    };
    pending.set_network_name(sh.hub.settings().network_name);
    let rx = set_pending(sh, gen_, pending.sas(), pending.peer_name());
    let (accept, trust, agents) = match tokio::time::timeout(PAIR_TTL, rx).await {
        Ok(Ok(d)) => d,
        _ => (false, Trust::Public, false),
    };
    // The exchange waits on the network, so it runs on a detached copy; the result is merged
    // into the live membership under its lock (trust changes made meanwhile are not lost).
    let mut scratch = sh.membership().detached();
    if !accept {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            pending.finish(false, trust, &mut scratch),
        )
        .await;
        return;
    }
    let Ok(mut member) = pending.finish(true, trust, &mut scratch).await else {
        return;
    };
    member.agents = agents;
    let before: Vec<NodeId> = {
        let mut g = sh.members.write().unwrap_or_else(|e| e.into_inner());
        let before = g
            .members()
            .iter()
            .map(|x| x.node_id)
            .filter(|id| *id != member.node_id)
            .collect();
        if let Err(e) = g.put(member.clone()) {
            eprintln!("octo: pairing: saving the new member failed: {e}");
            return;
        }
        before
    };
    let _ = sh.cmd.send(Cmd::Reconfigure).await;
    let Ok(msg) = pairing::announcement(&member) else {
        return;
    };
    for id in before {
        let _ = sh.cmd.send(Cmd::Control(id, msg.clone())).await;
    }
}

async fn join_flow(sh: &Arc<Shared>, gen_: u64, code: PairingCode, name: String) {
    let Ok(jp) = pairing::join(&code, &sh.key, &name, PAIR_TTL).await else {
        return;
    };
    let rx = set_pending(sh, gen_, jp.sas(), &code.token.host.to_string());
    let (accept, agents) = match tokio::time::timeout(PAIR_TTL, rx).await {
        Ok(Ok((true, _, agents))) => (true, agents),
        _ => (false, false),
    };
    let mut scratch = sh.membership().detached();
    if let Ok(network_name) = jp.finish(accept, &mut scratch).await {
        let adopted = {
            let mut g = sh.members.write().unwrap_or_else(|e| e.into_inner());
            match g.adopt_from(&scratch) {
                Ok(true) => g
                    .set_agents(&code.token.host, agents)
                    .map(|_| true)
                    .unwrap_or(true),
                Ok(false) => false,
                Err(e) => {
                    eprintln!("octo: pairing: saving the network failed: {e}");
                    false
                }
            }
        };
        if adopted {
            // The network lives on the relay of the code (its URL carries the relay key):
            // keep it, or the new members stay unreachable until the relay is typed in. The
            // host's name for the network comes with it (none: the UI shows the relay host).
            let relay_changed = sh.net().relay_url.as_deref() != Some(code.relay_url.as_str());
            let patch = octo_core::settings::SettingsPatch {
                relay_url: relay_changed.then(|| code.relay_url.clone()),
                network_name: Some(network_name.unwrap_or_default()),
                ..Default::default()
            };
            if let Err(e) = sh.hub.update_settings(patch) {
                eprintln!("octo: pairing: saving the relay failed: {e}");
            }
            let _ = sh.cmd.send(Cmd::Reconfigure).await;
        }
    }
}

// ---------------------------------------------------------------- tunnel target side

fn reply(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({"error": msg}).to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Who is calling the local API through the dispatcher.
#[derive(Clone, Copy)]
enum Caller {
    /// This node itself (`/api/nodes/<own id>/...`): no trust restrictions.
    Local,
    Member {
        trust: Trust,
        agents: bool,
    },
}

/// Runs a request against the local router (no TCP, no bearer token).
async fn dispatch(sh: &Shared, caller: Caller, head: ReqHead, body: Bytes) -> Response<Body> {
    // What the caller may do here; answered from the caller itself, never routed further.
    if head.method == "GET" && head.uri.split('?').next() == Some("/api/access") {
        let access = match caller {
            Caller::Local => AccessView {
                agents: true,
                private_files: true,
            },
            Caller::Member { trust, agents } => AccessView {
                agents,
                private_files: trust == Trust::Full,
            },
        };
        let json = serde_json::to_vec(&access).unwrap_or_default();
        return Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(json))
            .unwrap_or_else(|_| Response::new(Body::empty()));
    }
    if !policy::path_allowed(&head.uri) {
        return reply(StatusCode::FORBIDDEN, "not available through the tunnel");
    }
    let trust = match caller {
        Caller::Local => Trust::Full,
        Caller::Member { trust, agents } => {
            // Fail closed: if the thread cannot be looked up, nothing is decided from a guess.
            let meta = match policy::thread_id(&head.uri) {
                Some(id) => {
                    let hub = sh.hub.clone();
                    let id = id.to_owned();
                    match tokio::task::spawn_blocking(move || hub.thread_meta(&id)).await {
                        Ok(Ok(m)) => m,
                        Ok(Err(e)) => {
                            eprintln!("octo: thread lookup failed: {e}");
                            return reply(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
                        }
                        Err(e) => {
                            eprintln!("octo: thread lookup task failed: {e}");
                            return reply(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
                        }
                    }
                }
                None => None,
            };
            let req = TunnelReq {
                method: &head.method,
                path: &head.uri,
                thread_space: meta.map(|m| m.0),
                thread_provider: meta.map(|m| m.1),
                body_private: policy::body_touches_private(&body),
                body_agents: policy::body_agent_fields(&body),
            };
            if !policy::tunnel_allows(trust, agents, &req) {
                return reply(StatusCode::FORBIDDEN, "not permitted through the tunnel");
            }
            trust
        }
    };
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
    let Some(caller) = sh.caller_of(&peer) else {
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
                if body.len() + c.len() > MAX_BODY {
                    let _ = f.send_head(&respond(413)).await;
                    let _ = f.finish().await;
                    return;
                }
                body.extend_from_slice(&c);
            }
            Ok(None) => break,
            Err(_) => return,
        }
    }
    let resp = dispatch(&sh, caller, head, body.into()).await;
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
    task: AbortHandle,
}

/// Control messages waiting for a session with an offline peer. A newer message replaces an
/// older one of the same kind (a stale `private_key` is worthless once a newer one exists), so
/// the queue stays small however long the peer is away.
#[derive(Default)]
struct CtlQueue {
    q: HashMap<NodeId, Vec<(String, Vec<u8>)>>,
}

impl CtlQueue {
    /// Messages of one kind supersede each other; announcements are keyed by the member too.
    fn key(bytes: &[u8]) -> String {
        let v: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
        let t = v.get("t").and_then(|t| t.as_str()).unwrap_or("");
        match v.pointer("/member/node_id").and_then(|m| m.as_str()) {
            Some(m) => format!("{t}:{m}"),
            None => t.to_string(),
        }
    }

    fn push(&mut self, peer: NodeId, bytes: Vec<u8>) {
        let key = Self::key(&bytes);
        let list = self.q.entry(peer).or_default();
        match list.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = bytes,
            None => list.push((key, bytes)),
        }
    }

    fn take(&mut self, peer: &NodeId) -> Vec<Vec<u8>> {
        self.q
            .remove(peer)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, b)| b)
            .collect()
    }

    fn clear(&mut self, peer: &NodeId) {
        self.q.remove(peer);
    }

    fn peers(&self) -> impl Iterator<Item = &NodeId> {
        self.q.keys()
    }
}

struct Actor {
    sh: Arc<Shared>,
    sessions: Sessions,
    relay: Option<RelayHandle>,
    current: Option<(String, [u8; 32])>,
    peers: HashMap<NodeId, PeerEntry>,
    waiters: HashMap<NodeId, Vec<oneshot::Sender<Result<Link, NetError>>>>,
    ctl: CtlQueue,
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
        ctl: CtlQueue::default(),
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
        *self.sh.link.lock().unwrap_or_else(|e| e.into_inner()) =
            self.relay.as_ref().map(RelayHandle::status);
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
        let connect = {
            let m = self.sh.members.read().unwrap_or_else(|e| e.into_inner());
            self.sessions.connect(&m, peer)
        };
        if let Ok(f) = connect {
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
            .chain(self.ctl.peers())
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
                if link.is_none() {
                    // Removed member: nothing queued for it, no session or replay state kept.
                    self.ctl.clear(&peer);
                    if let Err(e) = self.sessions.forget(&peer) {
                        eprintln!("octo: forgetting session state failed: {e}");
                    }
                }
                self.drop_peer(&peer);
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
                let Ok(hint) = serde_json::to_vec(&serde_json::json!({"t": "sync_hint"})) else {
                    return;
                };
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
                    self.ctl.push(peer, bytes);
                    self.maybe_connect(peer).await;
                }
            }
        }
    }

    /// Ends the session and the mux of `peer`.
    fn drop_peer(&mut self, peer: &NodeId) {
        self.sessions.reset(peer);
        if let Some(p) = self.peers.remove(peer) {
            p.task.abort();
        }
        self.connecting.remove(peer);
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
        // Under the read lock only for the (synchronous) handling, never across an await.
        let handled = {
            let m = self.sh.members.read().unwrap_or_else(|e| e.into_inner());
            self.sessions.handle(&m, from, &payload)
        };
        match handled {
            Ok(Handled::Reply(f)) => {
                self.send(from, f).await;
                self.open_peer(from).await;
            }
            Ok(Handled::Established) => self.open_peer(from).await,
            Ok(Handled::Data(d)) => match d.split_first() {
                Some((&PLAIN_MUX, rest)) => {
                    let lost = self
                        .peers
                        .get(&from)
                        .is_some_and(|p| p.in_tx.try_send(rest.to_vec()).is_err());
                    if lost {
                        // A dropped mux frame stalls its streams for good: start over.
                        eprintln!(
                            "octo: inbound queue for a peer overflowed, resetting its session"
                        );
                        self.drop_peer(&from);
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
        let Ok(msg) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return; // not a control message
        };
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
                if let (true, true, Some(key)) = (full, epoch > theirs, key)
                    && let Ok(msg) = pairing::private_key_message(epoch, &key)
                {
                    self.send_ctl(from, &msg).await;
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
        let task = self.sh.spawn(peer_task(
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
        for c in self.ctl.take(&peer) {
            self.send_ctl(peer, &c).await;
        }
        if self.sh.trust_of(&peer) == Some(Trust::Full) {
            let epoch = self.sh.membership().private_epoch();
            if let Ok(msg) =
                serde_json::to_vec(&serde_json::json!({"t": "private_epoch", "epoch": epoch}))
            {
                self.send_ctl(peer, &msg).await;
            }
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
                    sh.spawn(serve_stream(sh.clone(), peer, s));
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
const MAX_BLOB: usize = crate::MAX_UPLOAD;
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
    let sh2 = sh.clone();
    sh.spawn(async move {
        let sh = sh2;
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

/// An entry with its downloaded content (`None` = already present locally).
type SyncItem = (SyncEntry, Option<Vec<u8>>);

/// Applies a group of entries with one index save, off the async runtime. Re-checks right
/// before applying that `peer` is still a full member (trust may have been withdrawn while
/// blobs were downloading). `None` = nothing applied (trust gone or the store failed).
async fn apply_batch(
    sh: &Arc<Shared>,
    peer: NodeId,
    items: Vec<SyncItem>,
) -> Option<Vec<Result<(), FilesError>>> {
    if items.is_empty() {
        return Some(vec![]);
    }
    if sh.trust_of(&peer) != Some(Trust::Full) {
        return None;
    }
    let files = sh.hub.files().clone();
    match tokio::task::spawn_blocking(move || files.private_apply_batch(&items)).await {
        Ok(Ok(r)) => Some(r),
        Ok(Err(e)) => {
            eprintln!("octo: sync: applying a batch failed: {e}");
            None
        }
        Err(e) => {
            eprintln!("octo: sync: apply task failed: {e}");
            None
        }
    }
}

/// Counts and reports entries the store refused (they are not retried).
fn report_rejected(
    peer: NodeId,
    entries: &[SyncEntry],
    results: &[Result<(), FilesError>],
) -> usize {
    let mut n = 0;
    for (e, r) in entries.iter().zip(results) {
        if let Err(err) = r {
            n += 1;
            eprintln!("octo: sync from {peer}: rejected {:?}: {err}", e.path);
        }
    }
    n
}

/// Downloaded blobs are applied in groups of about this many bytes.
const APPLY_GROUP_BYTES: usize = 64 * 1024 * 1024;

/// One pull run: manifest, plan, fetch missing blobs (hash-verified by `apply`), apply in
/// batches. Returns `Some` when nothing needs a retry: rejected entries are logged and counted
/// but not retried; failed downloads and oversized entries make the run incomplete.
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
    let (mut adopt, mut fetch) = (vec![], vec![]);
    for action in plan(&local, &remote) {
        match action {
            Action::Adopt(e) => adopt.push(e),
            Action::Fetch(e) => fetch.push(e),
        }
    }
    let mut rejected = 0;
    // Entries whose content is already here (or that need none).
    let results = apply_batch(sh, peer, adopt.iter().map(|e| (e.clone(), None)).collect()).await;
    let Some(results) = results else {
        return trust_gone_or_failed(sh, peer);
    };
    let mut missing = vec![];
    for (e, r) in adopt.iter().zip(&results) {
        if matches!(r, Err(FilesError::NotFound)) {
            missing.push(e.clone()); // content not present locally: download it
        }
    }
    let settled: Vec<_> = adopt
        .iter()
        .zip(&results)
        .filter(|(_, r)| !matches!(r, Err(FilesError::NotFound)))
        .map(|(e, _)| e.clone())
        .collect();
    let settled_results: Vec<_> = results
        .into_iter()
        .filter(|r| !matches!(r, Err(FilesError::NotFound)))
        .collect();
    rejected += report_rejected(peer, &settled, &settled_results);
    missing.extend(fetch);
    let mut group: Vec<SyncItem> = vec![];
    let mut group_bytes = 0usize;
    for entry in missing {
        if entry.size > MAX_BLOB as u64 {
            eprintln!(
                "octo: sync from {peer}: {:?} is too large ({} bytes), skipped",
                entry.path, entry.size
            );
            complete = false;
            continue;
        }
        let uri = format!("/api/sync/private/blob/{}", entry.hash);
        // A failed download is a transient failure (retried); a rejected entry is not.
        match peer_get(sh, peer, &uri, MAX_BLOB).await {
            Some(data) => {
                group_bytes += data.len();
                group.push((entry, Some(data)));
            }
            None => complete = false,
        }
        if group_bytes >= APPLY_GROUP_BYTES {
            let items = std::mem::take(&mut group);
            group_bytes = 0;
            let entries: Vec<_> = items.iter().map(|(e, _)| e.clone()).collect();
            let Some(r) = apply_batch(sh, peer, items).await else {
                return trust_gone_or_failed(sh, peer);
            };
            rejected += report_rejected(peer, &entries, &r);
        }
    }
    let entries: Vec<_> = group.iter().map(|(e, _)| e.clone()).collect();
    let Some(r) = apply_batch(sh, peer, group).await else {
        return trust_gone_or_failed(sh, peer);
    };
    rejected += report_rejected(peer, &entries, &r);
    if rejected > 0 {
        eprintln!("octo: sync from {peer}: {rejected} entries rejected");
    }
    complete.then_some(())
}

/// `apply_batch` yields nothing when trust was withdrawn (the run just ends) or on a store
/// failure (the run is retried).
fn trust_gone_or_failed(sh: &Arc<Shared>, peer: NodeId) -> Option<()> {
    (sh.trust_of(&peer) != Some(Trust::Full)).then_some(())
}

/// Host of a relay URL, for naming the network.
fn relay_host(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use octo_net::identity::NodeKey;

    fn id() -> NodeId {
        NodeKey::generate().node_id()
    }

    fn msg(t: &str, extra: &str) -> Vec<u8> {
        format!(r#"{{"t":"{t}"{extra}}}"#).into_bytes()
    }

    #[test]
    fn ctl_queue_keeps_only_the_latest_per_type() {
        let (a, b) = (id(), id());
        let mut q = CtlQueue::default();
        q.push(a, msg("private_key", r#","epoch":1"#));
        q.push(a, msg("private_key", r#","epoch":2"#));
        q.push(a, msg("private_key", r#","epoch":3"#));
        q.push(b, msg("private_key", r#","epoch":9"#));
        let got = q.take(&a);
        assert_eq!(got, vec![msg("private_key", r#","epoch":3"#)]);
        assert!(q.take(&a).is_empty());
        // Announcements about different members are different messages.
        q.push(a, msg("member_added", r#","member":{"node_id":"x"}"#));
        q.push(a, msg("member_added", r#","member":{"node_id":"y"}"#));
        q.push(a, msg("member_added", r#","member":{"node_id":"x","n":2}"#));
        assert_eq!(q.take(&a).len(), 2);
        // Removal clears the peer's queue.
        q.clear(&b);
        assert!(q.take(&b).is_empty());
    }
}
