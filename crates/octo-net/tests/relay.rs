mod common;
use common::*;

use octo_net::identity::NodeKey;
use octo_net::relay::{MAX_PAYLOAD, RelayLimits};
use octo_net::relay_client::{ERR_PEER_UNAVAILABLE, ERR_RATE_LIMITED, Event, RelayHandle};
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
async fn per_connection_rate_limit() {
    let limits = RelayLimits {
        msgs_per_sec: 1.0,
        msg_burst: 5.0,
        ..RelayLimits::default()
    };
    let (addr, _s) = spawn_relay(limits).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut a = online(addr, &ka, NET1).await;
    let b = online(addr, &kb, NET1).await;
    for _ in 0..30 {
        a.send(kb.node_id(), vec![1; 10]).await.unwrap();
    }
    let mut limited = false;
    for _ in 0..40 {
        if matches!(next(&mut a).await, Event::RelayError(ERR_RATE_LIMITED)) {
            limited = true;
            break;
        }
    }
    assert!(limited);
    // The other connection is unaffected.
    b.send(ka.node_id(), b"fine".to_vec()).await.unwrap();
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
