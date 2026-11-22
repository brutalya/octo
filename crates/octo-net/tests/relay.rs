mod common;
use common::*;

use octo_net::identity::NodeKey;
use octo_net::relay::{MAX_PAYLOAD, RelayLimits};
use octo_net::relay_client::{ERR_PEER_UNAVAILABLE, Event, RelayHandle};
use octo_net::session::{Handled, Sessions};
use std::time::Duration;

const NET1: [u8; 32] = [1; 32];
const NET2: [u8; 32] = [2; 32];

async fn next(h: &mut RelayHandle) -> Event {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), h.recv()).await {
            Ok(Some(Event::Connected)) => continue,
            Ok(Some(e)) => return e,
            other => panic!("no event: {other:?}"),
        }
    }
}

async fn silent(h: &mut RelayHandle) {
    let r = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match h.recv().await {
                Some(Event::Frame { .. }) => return true,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await;
    assert!(r.is_err(), "unexpected frame delivered");
}

#[tokio::test]
async fn routes_between_same_network_nodes() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let a = online(addr, &ka, NET1).await;
    let mut b = online(addr, &kb, NET1).await;
    a.send(kb.node_id(), b"hello".to_vec()).await.unwrap();
    match next(&mut b).await {
        Event::Frame { from, payload } => {
            assert_eq!(from, ka.node_id());
            assert_eq!(payload, b"hello");
        }
        e => panic!("{e:?}"),
    }
}

#[tokio::test]
async fn isolation_between_networks() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb, kc) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let mut a = online(addr, &ka, NET1).await;
    let mut b = online(addr, &kb, NET1).await;
    let mut c = online(addr, &kc, NET2).await;
    // A -> C (other network): not delivered, A told the peer is unavailable.
    a.send(kc.node_id(), b"for c".to_vec()).await.unwrap();
    assert!(matches!(
        next(&mut a).await,
        Event::RelayError(ERR_PEER_UNAVAILABLE)
    ));
    silent(&mut c).await;
    // C -> A and C -> B: nothing arrives either.
    c.send(ka.node_id(), b"intrude".to_vec()).await.unwrap();
    c.send(kb.node_id(), b"intrude".to_vec()).await.unwrap();
    assert!(matches!(
        next(&mut c).await,
        Event::RelayError(ERR_PEER_UNAVAILABLE)
    ));
    silent(&mut a).await;
    silent(&mut b).await;
    // Same-network still fine.
    a.send(kb.node_id(), b"ok".to_vec()).await.unwrap();
    assert!(matches!(next(&mut b).await, Event::Frame { .. }));
}

#[tokio::test]
async fn same_node_id_in_two_networks_is_independent() {
    // The same key registered in two networks must receive only its own network's traffic.
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut a1 = online(addr, &ka, NET1).await;
    let mut a2 = online(addr, &ka, NET2).await;
    let b = online(addr, &kb, NET1).await;
    b.send(ka.node_id(), b"x".to_vec()).await.unwrap();
    assert!(matches!(next(&mut a1).await, Event::Frame { .. }));
    silent(&mut a2).await;
}

#[tokio::test]
async fn oversize_payload_rejected_client_side() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let k = NodeKey::generate();
    let a = online(addr, &k, NET1).await;
    assert!(a.send(k.node_id(), vec![0; MAX_PAYLOAD + 1]).await.is_err());
    a.send(k.node_id(), vec![0; MAX_PAYLOAD]).await.unwrap();
}

#[tokio::test]
async fn burst_over_limit_is_delayed_not_dropped() {
    let limits = RelayLimits {
        msgs_per_sec: 400.0,
        msg_burst: 10.0,
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let a = online(addr, &ka, NET1).await;
    let mut b = online(addr, &kb, NET1).await;
    const N: u32 = 600;
    let t0 = std::time::Instant::now();
    let tx = tokio::spawn(async move {
        for i in 0..N {
            a.send(kb.node_id(), i.to_be_bytes().to_vec())
                .await
                .unwrap();
        }
        a
    });
    for i in 0..N {
        match next(&mut b).await {
            Event::Frame { payload, .. } => assert_eq!(payload, i.to_be_bytes(), "frame {i}"),
            e => panic!("{e:?}"),
        }
    }
    // 600 frames at 400/s with burst 10 cannot finish instantly.
    assert!(t0.elapsed() >= Duration::from_secs(1), "{:?}", t0.elapsed());
    let mut a = tx.await.unwrap();
    // The sender never saw a rate-limit error.
    let r = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Some(Event::RelayError(c)) = a.recv().await {
                return c;
            }
        }
    })
    .await;
    assert!(r.is_err(), "unexpected relay error {r:?}");
}

#[tokio::test]
async fn sustained_over_limit_closes_connection() {
    let limits = RelayLimits {
        msgs_per_sec: 20.0,
        msg_burst: 2.0,
        abuse_after: Duration::from_millis(500),
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut a = online(addr, &ka, NET1).await;
    let _b = online(addr, &kb, NET1).await;
    for _ in 0..2000 {
        if a.send(kb.node_id(), vec![1; 10]).await.is_err() {
            break;
        }
    }
    // The client reconnects within milliseconds, so watch for the event, not the state.
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = a.recv().await {
            if e == Event::Disconnected {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(closed, Ok(true), "abusive connection not closed");
}

#[tokio::test]
async fn garbage_before_auth_is_dropped() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(url(addr)).await.unwrap();
    ws.send(Message::Binary(vec![9; 40].into())).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                _ => continue,
            }
        }
    })
    .await;
    assert!(r.is_ok(), "relay must close unauthenticated garbage");
}

#[tokio::test]
async fn client_reconnects_after_relay_restart() {
    let (addr, mut srv) = RelayProc::start("127.0.0.1:0").unwrap();
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut a = online(addr, &ka, NET1).await;
    let b = online(addr, &kb, NET1).await;
    srv.kill();
    for _ in 0..100 {
        if !b.is_online() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!b.is_online());
    assert!(b.send(ka.node_id(), vec![1]).await.is_err());
    let mut srv2 = None;
    for _ in 0..100 {
        if let Ok(x) = RelayProc::start(&addr.to_string()) {
            srv2 = Some(x);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _srv2 = srv2.expect("rebind");
    a.wait_online(Duration::from_secs(10)).await.unwrap();
    b.wait_online(Duration::from_secs(10)).await.unwrap();
    b.send(ka.node_id(), b"back".to_vec()).await.unwrap();
    loop {
        match next(&mut a).await {
            Event::Disconnected => continue,
            Event::Frame { payload, .. } => {
                assert_eq!(payload, b"back");
                break;
            }
            e => panic!("{e:?}"),
        }
    }
}

#[tokio::test]
async fn noise_ik_session_end_to_end_over_relay() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let nid = ma.network_id();
    assert_eq!(nid, mb.network_id());
    let (mut ha, mut hb) = (online(addr, &ka, nid).await, online(addr, &kb, nid).await);
    let (mut sa, mut sb) = (Sessions::new(ka), Sessions::new(kb));
    ha.send(idb, sa.connect(&ma, idb).unwrap()).await.unwrap();
    let Event::Frame { from, payload } = next(&mut hb).await else {
        panic!()
    };
    let Handled::Reply(r) = sb.handle(&mb, from, &payload).unwrap() else {
        panic!()
    };
    hb.send(from, r).await.unwrap();
    let Event::Frame { from, payload } = next(&mut ha).await else {
        panic!()
    };
    assert!(matches!(
        sa.handle(&ma, from, &payload).unwrap(),
        Handled::Established
    ));
    ha.send(idb, sa.encrypt(idb, b"private thread").unwrap())
        .await
        .unwrap();
    let Event::Frame { from, payload } = next(&mut hb).await else {
        panic!()
    };
    assert_eq!(from, ida);
    assert!(
        matches!(sb.handle(&mb, from, &payload).unwrap(), Handled::Data(d) if d == b"private thread")
    );
}

// ---- raw protocol client (to act as a misbehaving peer)

type RawWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Raw {
    ws: RawWs,
    t: snow::TransportState,
    id: octo_net::identity::NodeId,
}

impl Raw {
    async fn connect(addr: std::net::SocketAddr, net: [u8; 32]) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let params = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
        let kp = snow::Builder::new(params.parse().unwrap())
            .generate_keypair()
            .unwrap();
        let id = octo_net::identity::NodeId::from_pubkey(&kp.public.clone().try_into().unwrap());
        let mut hs = snow::Builder::new(params.parse().unwrap())
            .local_private_key(&kp.private)
            .unwrap()
            .prologue(b"octo-relay-v1")
            .unwrap()
            .build_initiator()
            .unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(url(addr)).await.unwrap();
        let mut buf = vec![0u8; 65_535];
        let n = hs.write_message(&[], &mut buf).unwrap();
        let mut m1 = vec![1u8];
        m1.extend_from_slice(&buf[..n]);
        ws.send(Message::Binary(m1.into())).await.unwrap();
        let Some(Ok(Message::Binary(m2))) = ws.next().await else {
            panic!("no msg2")
        };
        hs.read_message(&m2, &mut buf).unwrap();
        let n = hs.write_message(&net, &mut buf).unwrap();
        ws.send(Message::Binary(buf[..n].to_vec().into()))
            .await
            .unwrap();
        let mut me = Self {
            ws,
            t: hs.into_transport_mode().unwrap(),
            id,
        };
        assert_eq!(me.recv().await.unwrap(), [0x13]);
        me
    }

    async fn send(&mut self, pt: &[u8]) {
        use futures_util::SinkExt;
        let mut out = vec![0u8; pt.len() + 16];
        let n = self.t.write_message(pt, &mut out).unwrap();
        self.ws
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                out[..n].to_vec().into(),
            ))
            .await
            .unwrap();
    }

    /// Next decrypted message; `None` once the relay closed the connection.
    async fn recv(&mut self) -> Option<Vec<u8>> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    let mut out = vec![0u8; b.len()];
                    let n = self.t.read_message(&b, &mut out).unwrap();
                    out.truncate(n);
                    return Some(out);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return None,
            }
        }
    }
}

#[tokio::test]
async fn bidirectional_bulk_does_not_deadlock() {
    let limits = RelayLimits {
        msgs_per_sec: 1e9,
        msg_burst: 1e9,
        bytes_per_sec: 1e12,
        byte_burst: 1e12,
        queue: 1024,
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut a = online(addr, &ka, NET1).await;
    let mut b = online(addr, &kb, NET1).await;
    const N: usize = 500;
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let all = async {
        let (ra, rb) = (&a, &b);
        tokio::join!(
            async {
                for _ in 0..N {
                    ra.send(idb, vec![1; 60_000]).await.unwrap();
                }
            },
            async {
                for _ in 0..N {
                    rb.send(ida, vec![2; 60_000]).await.unwrap();
                }
            }
        );
        for _ in 0..N {
            assert!(matches!(next(&mut a).await, Event::Frame { .. }));
            assert!(matches!(next(&mut b).await, Event::Frame { .. }));
        }
    };
    tokio::time::timeout(Duration::from_secs(90), all)
        .await
        .expect("bulk both ways must complete");
}

#[tokio::test]
async fn stalled_reader_is_closed_by_write_timeout() {
    let limits = RelayLimits {
        msgs_per_sec: 1e9,
        msg_burst: 1e9,
        bytes_per_sec: 1e12,
        byte_burst: 1e12,
        write_timeout: Duration::from_millis(300),
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    let kb = NodeKey::generate();
    // `stalled` authenticates, then never reads again.
    let stalled = Raw::connect(addr, NET1).await;
    let mut b = online(addr, &kb, NET1).await;
    let big = vec![3u8; 60_000];
    let end = std::time::Instant::now() + Duration::from_secs(15);
    let mut seen = false;
    while !seen && std::time::Instant::now() < end {
        b.send(stalled.id, big.clone()).await.unwrap();
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(5), b.recv()).await {
            seen |= ev == Event::RelayError(ERR_PEER_UNAVAILABLE);
        }
    }
    assert!(seen, "relay must drop the stalled connection");
}

#[tokio::test]
async fn silent_connections_time_out_and_keepalive_keeps_alive() {
    let limits = RelayLimits {
        idle_timeout: Duration::from_millis(400),
        handshake_timeout: Duration::from_millis(300),
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    // Pre-auth silence.
    let (mut ws, _) = tokio_tungstenite::connect_async(url(addr)).await.unwrap();
    let closed = tokio::time::timeout(
        Duration::from_secs(3),
        futures_util::StreamExt::next(&mut ws),
    )
    .await;
    assert!(matches!(
        closed,
        Ok(None | Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
    ));
    // Authenticated silence.
    let mut quiet = Raw::connect(addr, NET1).await;
    let r = tokio::time::timeout(Duration::from_secs(3), quiet.recv()).await;
    assert!(matches!(r, Ok(None)), "idle connection must be closed");
    // Keepalives hold the connection open well past the idle timeout and still route.
    let mut live = Raw::connect(addr, NET1).await;
    for _ in 0..8 {
        live.send(&[0x14]).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let mut send = vec![0x10];
    send.extend_from_slice(&[9; 16]);
    send.push(1);
    live.send(&send).await;
    assert_eq!(live.recv().await.unwrap(), [0x12, ERR_PEER_UNAVAILABLE]);
}

#[tokio::test]
async fn failed_upgrades_do_not_leak_connection_permits() {
    use tokio::io::AsyncWriteExt;
    let limits = RelayLimits {
        max_connections: 3,
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    for _ in 0..6 {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "GET /relay HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        drop(s);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let k = NodeKey::generate();
    online(addr, &k, NET1).await;
}

/// `wss://` must speak TLS: the first byte on the wire is a TLS handshake record (0x16).
#[tokio::test]
async fn wss_url_starts_tls_handshake() {
    use tokio::io::AsyncReadExt;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let key = NodeKey::generate();
    let mut cfg = fast_cfg(l.local_addr().unwrap(), &key, NET1);
    cfg.url = format!("wss://localhost:{port}/relay");
    let _h = RelayHandle::connect(cfg);
    let (mut s, _) = tokio::time::timeout(Duration::from_secs(5), l.accept())
        .await
        .expect("client never connected")
        .unwrap();
    let mut b = [0u8; 1];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(b[0], 0x16, "expected a TLS ClientHello");
}
