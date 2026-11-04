mod common;
use common::*;
use octo_net::identity::NodeKey;
use octo_net::membership::Membership;
use octo_net::pairing::{PairHost, PairingCode, join, sas_from_hash, xx_state};
use octo_net::relay::RelayLimits;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(300);

#[test]
fn code_roundtrip() {
    let k = NodeKey::generate();
    let t = octo_net::pairing::PairingToken::generate(k.node_id());
    let c = PairingCode {
        relay_url: "ws://h:1/relay".into(),
        token: t,
    };
    let s = c.to_string();
    assert!(s.starts_with("ws://h:1/relay#"));
    assert_eq!(PairingCode::parse(&s).unwrap().token, c.token);
    assert!(PairingCode::parse("no-hash").is_err());
    assert!(PairingCode::parse("ws://x#zzz").is_err());
}

/// Drive XX by hand. Returns (initiator hash, responder hash).
fn xx(
    secret: &[u8; 16],
    ki: &NodeKey,
    si: &[u8; 16],
    kr: &NodeKey,
    sr: &[u8; 16],
) -> Option<(Vec<u8>, Vec<u8>)> {
    let _ = secret;
    let mut i = xx_state(true, ki, si).unwrap();
    let mut r = xx_state(false, kr, sr).unwrap();
    let (mut buf, mut pl) = ([0u8; 1024], [0u8; 1024]);
    let n = i.write_message(&[], &mut buf).ok()?;
    r.read_message(&buf[..n], &mut pl).ok()?;
    let n = r.write_message(&[], &mut buf).ok()?;
    i.read_message(&buf[..n], &mut pl).ok()?;
    let n = i.write_message(b"name", &mut buf).ok()?;
    r.read_message(&buf[..n], &mut pl).ok()?;
    Some((
        i.get_handshake_hash().to_vec(),
        r.get_handshake_hash().to_vec(),
    ))
}

#[test]
fn sas_equal_on_both_sides() {
    let (a, b) = (NodeKey::generate(), NodeKey::generate());
    let s = [5u8; 16];
    let (hi, hr) = xx(&s, &b, &s, &a, &s).unwrap();
    let sas = sas_from_hash(&hi);
    assert_eq!(sas, sas_from_hash(&hr));
    assert_eq!(sas.len(), 6);
    assert!(sas.bytes().all(|c| c.is_ascii_digit()));
}

#[test]
fn sas_differs_under_mitm_that_swaps_keys() {
    // Relay M knows the (leaked) pairing secret, so it can complete a handshake with each side,
    // substituting its own key: B <-> M <-> A. The two legs must show different SAS.
    let (a, b, m) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let s = [9u8; 16];
    // Leg 1: B (initiator) <-> M (responder). Leg 2: M (initiator) <-> A (responder).
    let (b_hash, m_resp_hash) = xx(&s, &b, &s, &m, &s).unwrap();
    let (m_init_hash, a_hash) = xx(&s, &m, &s, &a, &s).unwrap();
    // Each leg is self-consistent, so neither side notices from the handshake alone...
    assert_eq!(sas_from_hash(&b_hash), sas_from_hash(&m_resp_hash));
    assert_eq!(sas_from_hash(&a_hash), sas_from_hash(&m_init_hash));
    // ...but the numbers the two humans compare differ.
    assert_ne!(sas_from_hash(&b_hash), sas_from_hash(&a_hash));
}

#[test]
fn handshake_without_secret_fails() {
    let (a, b) = (NodeKey::generate(), NodeKey::generate());
    assert!(xx(&[0; 16], &b, &[1; 16], &a, &[2; 16]).is_none());
}

#[tokio::test]
async fn full_pairing_over_relay() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let d = tempfile::tempdir().unwrap();
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut ma = Membership::load_or_create(&d.path().join("a.json")).unwrap();
    let mut mb = Membership::load_or_create(&d.path().join("b.json")).unwrap();
    let mut host = PairHost::start(&url(addr), &ka, "laptop", TTL)
        .await
        .unwrap();
    let code = host.code();
    let code_s = code.to_string();

    let joiner = tokio::spawn({
        let kb = kb.clone();
        async move {
            let code = PairingCode::parse(&code_s).unwrap();
            let p = join(&code, &kb, "phone", TTL).await.unwrap();
            let sas = p.sas().to_string();
            (p, sas)
        }
    });
    let hp = host.wait_peer().await.unwrap();
    let (jp, jsas) = joiner.await.unwrap();
    assert_eq!(hp.sas(), jsas);
    assert_eq!(hp.peer_name(), "phone");
    assert_eq!(hp.peer_node_id(), kb.node_id());

    let ja = tokio::spawn(async move {
        let mut mb = mb;
        jp.finish(true, &mut mb).await.map(|_| mb)
    });
    let member = hp
        .finish(true, octo_net::membership::Trust::Public, &mut ma)
        .await
        .unwrap();
    mb = ja.await.unwrap().unwrap();
    assert_eq!(member.node_id, kb.node_id());
    // Both sides now share the network key and know each other.
    assert_eq!(ma.network_id(), mb.network_id());
    assert_eq!(ma.get(&kb.node_id()).unwrap().pubkey, kb.public());
    assert_eq!(mb.get(&ka.node_id()).unwrap().pubkey, ka.public());
    assert_eq!(mb.get(&ka.node_id()).unwrap().name, "laptop");
    // Persisted.
    assert_eq!(
        Membership::load_or_create(&d.path().join("b.json"))
            .unwrap()
            .network_id(),
        ma.network_id()
    );
}

#[tokio::test]
async fn rejection_on_host_pairs_nothing() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut ma = Membership::ephemeral(octo_net::membership::NetworkKey::generate());
    let mb = Membership::ephemeral(octo_net::membership::NetworkKey::generate());
    let before = mb.network_id();
    let mut host = PairHost::start(&url(addr), &ka, "a", TTL).await.unwrap();
    let code = host.code();
    let j = tokio::spawn(async move {
        let p = join(&code, &kb, "b", TTL).await.unwrap();
        let mut mb2 = mb;
        let r = p.finish(true, &mut mb2).await;
        (r, mb2)
    });
    let hp = host.wait_peer().await.unwrap();
    assert!(
        hp.finish(false, octo_net::membership::Trust::Public, &mut ma)
            .await
            .is_err()
    );
    let (r, mb) = j.await.unwrap();
    assert!(r.is_err());
    assert!(ma.members().is_empty());
    assert!(mb.members().is_empty());
    assert_eq!(mb.network_id(), before);
}

#[tokio::test]
async fn token_is_one_time_and_expires() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb, kc) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let mut ma = Membership::ephemeral(octo_net::membership::NetworkKey::generate());
    let mut host = PairHost::start(&url(addr), &ka, "a", TTL).await.unwrap();
    let code = host.code();
    let c2 = code.clone();
    let j = tokio::spawn(async move { join(&c2, &kb, "b", TTL).await });
    let hp = host.wait_peer().await.unwrap();
    let _jp = j.await.unwrap().unwrap();
    // A second joiner using the same code after consumption gets nowhere.
    let r = join(&code, &kc, "c", Duration::from_millis(500)).await;
    assert!(r.is_err());
    drop(hp);
    let _ = &mut ma;

    // Expiry.
    let mut h2 = PairHost::start(&url(addr), &ka, "a", Duration::from_millis(200))
        .await
        .unwrap();
    assert!(h2.wait_peer().await.is_err());
    let k4 = NodeKey::generate();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        join(&h2.code(), &k4, "d", Duration::from_millis(500))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn wrong_token_cannot_pair() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut host = PairHost::start(&url(addr), &ka, "a", Duration::from_secs(2))
        .await
        .unwrap();
    let mut code = host.code();
    // Same host id, different secret: lands in a different relay network and handshake prologue.
    let t2 = octo_net::pairing::PairingToken::generate(ka.node_id());
    code.token = t2;
    assert!(
        join(&code, &kb, "b", Duration::from_millis(500))
            .await
            .is_err()
    );
    assert!(host.wait_peer().await.is_err());
}

#[tokio::test]
async fn already_paired_node_cannot_join() {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb, kx) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let host = PairHost::start(&url(addr), &ka, "a", TTL).await.unwrap();
    let code = host.code();
    let mut mb = Membership::ephemeral(octo_net::membership::NetworkKey::generate());
    mb.add(octo_net::membership::Member::new(kx.public(), "x"))
        .unwrap();
    let p = join(&code, &kb, "b", Duration::from_millis(500)).await;
    // Joining itself is allowed to start; finishing into a non-empty membership must fail.
    if let Ok(p) = p {
        assert!(p.finish(true, &mut mb).await.is_err());
    }
}

#[test]
fn announcement_applies_only_with_known_sender() {
    use octo_net::pairing::{announcement, apply_announcement};
    let (a, b, c) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let nk = octo_net::membership::NetworkKey::generate();
    let mut mb = Membership::ephemeral(nk);
    mb.add(octo_net::membership::Member::new(a.public(), "a"))
        .unwrap();
    let msg = announcement(&octo_net::membership::Member::new(c.public(), "c"));
    assert!(apply_announcement(&mut mb, &b.node_id(), &msg).is_err());
    apply_announcement(&mut mb, &a.node_id(), &msg).unwrap();
    assert!(mb.get(&c.node_id()).is_some());
}
