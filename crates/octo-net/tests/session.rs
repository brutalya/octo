mod common;
use common::two_nodes;
use octo_net::NetError;
use octo_net::session::{Handled, Sessions};

fn establish(
    a: &mut Sessions,
    ma: &octo_net::membership::Membership,
    b: &mut Sessions,
    mb: &octo_net::membership::Membership,
    ida: octo_net::identity::NodeId,
    idb: octo_net::identity::NodeId,
) {
    let hs1 = a.connect(ma, idb).unwrap();
    let Handled::Reply(hs2) = b.handle(mb, ida, &hs1).unwrap() else {
        panic!("expected reply")
    };
    assert!(matches!(
        a.handle(ma, idb, &hs2).unwrap(),
        Handled::Established
    ));
    assert!(a.is_established(&idb) && b.is_established(&ida));
}

#[test]
fn ik_handshake_and_transport_roundtrip() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    establish(&mut a, &ma, &mut b, &mb, ida, idb);
    for i in 0..5u8 {
        let f = a.encrypt(idb, &[i; 100]).unwrap();
        assert!(matches!(b.handle(&mb, ida, &f).unwrap(), Handled::Data(d) if d == vec![i; 100]));
        let f = b.encrypt(ida, b"pong").unwrap();
        assert!(matches!(a.handle(&ma, idb, &f).unwrap(), Handled::Data(d) if d == b"pong"));
    }
}

#[test]
fn ciphertext_hides_plaintext() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    establish(&mut a, &ma, &mut b, &mb, ida, idb);
    let f = a.encrypt(idb, b"super secret plaintext").unwrap();
    assert!(!f.windows(6).any(|w| w == b"secret"));
}

#[test]
fn tampered_frame_rejected_and_session_survives() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    establish(&mut a, &ma, &mut b, &mb, ida, idb);
    let f = a.encrypt(idb, b"hello").unwrap();
    for pos in [2, f.len() / 2, f.len() - 1] {
        let mut bad = f.clone();
        bad[pos] ^= 1;
        assert!(b.handle(&mb, ida, &bad).is_err(), "pos {pos}");
    }
    // The genuine frame still decrypts (failed attempts must not advance state).
    assert!(matches!(b.handle(&mb, ida, &f).unwrap(), Handled::Data(d) if d == b"hello"));
}

#[test]
fn replayed_frame_rejected() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    establish(&mut a, &ma, &mut b, &mb, ida, idb);
    let f1 = a.encrypt(idb, b"one").unwrap();
    let f2 = a.encrypt(idb, b"two").unwrap();
    b.handle(&mb, ida, &f1).unwrap();
    assert!(b.handle(&mb, ida, &f1).is_err(), "replay");
    b.handle(&mb, ida, &f2).unwrap();
    assert!(b.handle(&mb, ida, &f1).is_err(), "old replay");
}

#[test]
fn replayed_handshake_rejected() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    let hs1 = a.connect(&ma, idb).unwrap();
    let Handled::Reply(hs2) = b.handle(&mb, ida, &hs1).unwrap() else {
        panic!()
    };
    a.handle(&ma, idb, &hs2).unwrap();
    let f = a.encrypt(idb, b"x").unwrap();
    // Relay replays the old first message: must not replace the live session.
    assert!(b.handle(&mb, ida, &hs1).is_err());
    assert!(matches!(b.handle(&mb, ida, &f).unwrap(), Handled::Data(_)));
}

#[test]
fn non_member_initiator_rejected() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let _ = ka;
    let pb = kb.public();
    let (idb, mut b) = (kb.node_id(), Sessions::new(kb));
    // Stranger knows B's key but is not in B's membership.
    let stranger = octo_net::identity::NodeKey::generate();
    let ids = stranger.node_id();
    let mut s = Sessions::new(stranger);
    let mut ms = octo_net::membership::Membership::ephemeral(ma.network_key().clone());
    ms.add(octo_net::membership::Member::new(pb, "b")).unwrap();
    let hs1 = s.connect(&ms, idb).unwrap();
    assert!(matches!(b.handle(&mb, ids, &hs1), Err(NetError::NotMember)));
}

#[test]
fn spoofed_sender_id_rejected() {
    // Frame claims to come from C but is signed by A's static key.
    let ((ka, ma), (kb, mut mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let kc = octo_net::identity::NodeKey::generate();
    mb.add(octo_net::membership::Member::new(kc.public(), "c"))
        .unwrap();
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    let hs1 = a.connect(&ma, idb).unwrap();
    assert!(b.handle(&mb, kc.node_id(), &hs1).is_err());
    let _ = ida;
}

#[test]
fn wrong_network_prologue_rejected() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    let mut ma2 =
        octo_net::membership::Membership::ephemeral(octo_net::membership::NetworkKey::generate());
    ma2.add(ma.get(&idb).unwrap().clone()).unwrap();
    let hs1 = a.connect(&ma2, idb).unwrap();
    assert!(b.handle(&mb, ida, &hs1).is_err());
}

#[test]
fn oversized_plaintext_rejected() {
    let ((ka, ma), (kb, mb)) = two_nodes();
    let (ida, idb) = (ka.node_id(), kb.node_id());
    let (mut a, mut b) = (Sessions::new(ka), Sessions::new(kb));
    establish(&mut a, &ma, &mut b, &mb, ida, idb);
    let big = vec![0u8; octo_net::session::MAX_PLAINTEXT + 1];
    assert!(matches!(a.encrypt(idb, &big), Err(NetError::TooLarge)));
    let ok = vec![0u8; octo_net::session::MAX_PLAINTEXT];
    let f = a.encrypt(idb, &ok).unwrap();
    assert!(f.len() <= octo_net::relay::MAX_PAYLOAD);
}
