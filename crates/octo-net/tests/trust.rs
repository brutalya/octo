mod common;
use common::*;
use octo_net::identity::NodeKey;
use octo_net::membership::{Member, Membership, NetworkKey, Trust};
use octo_net::pairing::{
    PairHost, PairingCode, announcement, apply_announcement, apply_control, join,
    private_key_message,
};
use octo_net::relay::RelayLimits;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(300);

async fn pair(trust: Trust) -> (Membership, Membership, NodeKey, NodeKey) {
    let (addr, _s) = spawn_relay(RelayLimits::default()).await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let mut ma = Membership::ephemeral(NetworkKey::generate());
    let mut host = PairHost::start(&url(addr), &ka, "laptop", TTL)
        .await
        .unwrap();
    let code = PairingCode::parse(&host.code().to_string()).unwrap();
    let j = tokio::spawn({
        let kb = kb.clone();
        async move {
            let p = join(&code, &kb, "phone", TTL).await.unwrap();
            let mut mb = Membership::ephemeral(NetworkKey::generate());
            p.finish(true, &mut mb).await.unwrap();
            mb
        }
    });
    let hp = host.wait_peer().await.unwrap();
    let m = hp.finish(true, trust, &mut ma).await.unwrap();
    assert_eq!(m.trust, trust);
    (ma, j.await.unwrap(), ka, kb)
}

#[tokio::test]
async fn welcome_without_private_key_for_public_trust() {
    let (ma, mb, ka, kb) = pair(Trust::Public).await;
    assert_eq!(ma.get(&kb.node_id()).unwrap().trust, Trust::Public);
    assert!(ma.private_key().is_some());
    assert!(mb.private_key().is_none());
    assert_eq!(mb.private_epoch(), 0);
    assert_eq!(mb.get(&ka.node_id()).unwrap().trust, Trust::Public);
}

#[tokio::test]
async fn welcome_with_private_key_for_full_trust() {
    let (ma, mb, ka, kb) = pair(Trust::Full).await;
    assert_eq!(ma.get(&kb.node_id()).unwrap().trust, Trust::Full);
    let (ka_, kb_) = (ma.private_key().unwrap(), mb.private_key().unwrap());
    assert_eq!(ka_.as_bytes(), kb_.as_bytes());
    assert_eq!(ma.private_epoch(), mb.private_epoch());
    assert_ne!(ka_.as_bytes(), ma.network_key().as_bytes());
    assert_eq!(mb.get(&ka.node_id()).unwrap().trust, Trust::Full);
}

#[test]
fn rotation_and_adoption_by_epoch() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.json");
    let mut m = Membership::load_or_create(&p).unwrap();
    let (e1, k1) = (m.private_epoch(), m.private_key().unwrap().clone());
    let e2 = m.rotate_private_key().unwrap().unwrap();
    assert_eq!(e2, e1 + 1);
    assert_ne!(m.private_key().unwrap().as_bytes(), k1.as_bytes());
    let again = Membership::load_or_create(&p).unwrap();
    assert_eq!(again.private_epoch(), e2);
    assert_eq!(
        again.private_key().unwrap().as_bytes(),
        m.private_key().unwrap().as_bytes()
    );
    // Older or equal epochs are ignored.
    assert!(!m.adopt_private(e1, &k1).unwrap());
    assert!(!m.adopt_private(e2, &k1).unwrap());
    assert!(m.adopt_private(e2 + 1, &k1).unwrap());
    assert_eq!(m.private_epoch(), e2 + 1);
}

#[test]
fn set_trust_changes_and_persists() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.json");
    let mut m = Membership::load_or_create(&p).unwrap();
    let a = NodeKey::generate();
    m.add(Member::new(a.public(), "a")).unwrap();
    assert_eq!(m.get(&a.node_id()).unwrap().trust, Trust::Public);
    assert!(m.set_trust(&a.node_id(), Trust::Full).unwrap());
    assert_eq!(
        Membership::load_or_create(&p)
            .unwrap()
            .get(&a.node_id())
            .unwrap()
            .trust,
        Trust::Full
    );
    assert!(
        !m.set_trust(&NodeKey::generate().node_id(), Trust::Full)
            .unwrap()
    );
}

#[test]
fn announced_trust_only_from_full_members() {
    let (a, b, c) = (
        NodeKey::generate(),
        NodeKey::generate(),
        NodeKey::generate(),
    );
    let mut m = Membership::ephemeral(NetworkKey::generate());
    let mut ma = Member::new(a.public(), "a");
    ma.trust = Trust::Full;
    m.add(ma).unwrap();
    m.add(Member::new(b.public(), "b")).unwrap();
    let mut cm = Member::new(c.public(), "c");
    cm.trust = Trust::Full;
    let msg = announcement(&cm).unwrap();
    // From a public member: ignored entirely.
    assert!(apply_control(&mut m, &b.node_id(), &msg).is_err());
    assert!(m.get(&c.node_id()).is_none());
    // From a full member: trust is taken over.
    apply_control(&mut m, &a.node_id(), &msg).unwrap();
    assert_eq!(m.get(&c.node_id()).unwrap().trust, Trust::Full);
}

#[test]
fn announcement_never_overwrites_an_existing_member() {
    let (a, b) = (NodeKey::generate(), NodeKey::generate());
    let mut m = Membership::ephemeral(NetworkKey::generate());
    let mut ma = Member::new(a.public(), "a");
    ma.trust = Trust::Full;
    m.add(ma).unwrap();
    let mut mb = Member::new(b.public(), "b");
    mb.trust = Trust::Full;
    m.add(mb).unwrap();
    // A full announcer cannot downgrade or rename an existing member.
    let mut other = Member::new(b.public(), "renamed");
    other.trust = Trust::Public;
    apply_announcement(&mut m, &a.node_id(), other).unwrap();
    let got = m.get(&b.node_id()).unwrap();
    assert_eq!((got.trust, got.name.as_str()), (Trust::Full, "b"));
    // `add` itself reports and ignores an existing id.
    assert!(!m.add(Member::new(b.public(), "x")).unwrap());
    assert_eq!(m.get(&b.node_id()).unwrap().name, "b");
}

#[test]
fn private_key_message_only_from_full_and_newer() {
    let (a, b) = (NodeKey::generate(), NodeKey::generate());
    let mut m = Membership::ephemeral(NetworkKey::generate());
    let mut ma = Member::new(a.public(), "a");
    ma.trust = Trust::Full;
    m.add(ma).unwrap();
    m.add(Member::new(b.public(), "b")).unwrap();
    let k = NetworkKey::generate();
    let msg = private_key_message(5, k.as_bytes()).unwrap();
    assert!(apply_control(&mut m, &b.node_id(), &msg).is_err());
    assert_eq!(m.private_epoch(), 1);
    apply_control(&mut m, &a.node_id(), &msg).unwrap();
    assert_eq!(m.private_epoch(), 5);
    assert_eq!(m.private_key().unwrap().as_bytes(), k.as_bytes());
    // Stale epoch ignored.
    let old = private_key_message(4, &[7; 32]).unwrap();
    let _ = apply_control(&mut m, &a.node_id(), &old);
    assert_eq!(m.private_epoch(), 5);
    // Announcements still route through apply_control.
    let c = NodeKey::generate();
    apply_control(
        &mut m,
        &a.node_id(),
        &announcement(&Member::new(c.public(), "c")).unwrap(),
    )
    .unwrap();
    assert!(m.get(&c.node_id()).is_some());
}

#[test]
fn agents_flag_is_local_only() {
    let (a, c) = (NodeKey::generate(), NodeKey::generate());
    let mut m = Membership::ephemeral(NetworkKey::generate());
    let mut ma = Member::new(a.public(), "a");
    ma.trust = Trust::Full;
    m.add(ma).unwrap();
    // Default is off; an announcement claiming agents=true is ignored for that flag.
    let mut cm = Member::new(c.public(), "c");
    cm.agents = true;
    assert!(
        !serde_json::from_slice::<serde_json::Value>(&announcement(&cm).unwrap()).unwrap()
            ["member"]["agents"]
            .as_bool()
            .unwrap_or(false),
        "announcements never carry agents"
    );
    let msg = serde_json::to_vec(&serde_json::json!({"t":"member_added","member":cm})).unwrap();
    apply_control(&mut m, &a.node_id(), &msg).unwrap();
    assert!(!m.get(&c.node_id()).unwrap().agents);
    // Local setter persists.
    assert!(m.set_agents(&c.node_id(), true).unwrap());
    assert!(m.get(&c.node_id()).unwrap().agents);
    assert!(!m.set_agents(&c.node_id(), true).unwrap());
}

#[test]
fn agents_flag_persists_and_old_files_read_false() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("membership.json");
    let a = NodeKey::generate();
    let mut m = Membership::load_or_create(&p).unwrap();
    m.add(Member::new(a.public(), "a")).unwrap();
    m.set_agents(&a.node_id(), true).unwrap();
    let m2 = Membership::load_or_create(&p).unwrap();
    assert!(m2.get(&a.node_id()).unwrap().agents);
    // A file written before the field existed.
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["members"][0].as_object_mut().unwrap().remove("agents");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(
        !Membership::load_or_create(&p)
            .unwrap()
            .get(&a.node_id())
            .unwrap()
            .agents
    );
}

/// Leaving a network: members gone, a new network key and private key, persisted.
#[test]
fn reset_starts_a_fresh_network() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.json");
    let mut m = Membership::load_or_create(&p).unwrap();
    m.add(Member::new(NodeKey::generate().public(), "a"))
        .unwrap();
    let (old_id, old_private) = (m.network_id(), *m.private_key().unwrap().as_bytes());
    m.reset().unwrap();
    let again = Membership::load_or_create(&p).unwrap();
    for x in [&m, &again] {
        assert!(x.members().is_empty());
        assert_ne!(x.network_id(), old_id);
        assert_ne!(*x.private_key().unwrap().as_bytes(), old_private);
    }
    assert_eq!(again.network_id(), m.network_id());
}
