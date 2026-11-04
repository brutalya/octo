use octo_net::identity::{NodeId, NodeKey};
use octo_net::membership::{Member, Membership};

#[test]
fn node_key_persists_with_0600_and_id_is_stable() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("keys/node.key");
    let k1 = NodeKey::load_or_create(&p).unwrap();
    let k2 = NodeKey::load_or_create(&p).unwrap();
    assert_eq!(k1.public(), k2.public());
    assert_eq!(k1.node_id(), k2.node_id());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(std::fs::metadata(&p).unwrap().len(), 32);
}

#[test]
fn bad_key_file_never_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("node.key");
    std::fs::write(&p, b"short").unwrap();
    assert!(NodeKey::load_or_create(&p).is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b"short");
}

#[test]
fn node_id_is_blake2s_prefix_base32() {
    use blake2::{Blake2s256, Digest};
    let k = NodeKey::generate();
    let h = Blake2s256::digest(k.public());
    assert_eq!(k.node_id().as_bytes(), &h[..16]);
    let s = k.node_id().to_string();
    assert_eq!(s.len(), 26);
    assert_eq!(NodeId::parse(&s).unwrap(), k.node_id());
    assert!(NodeId::parse("nonsense!").is_err());
}

#[test]
fn debug_does_not_leak_secret() {
    let k = NodeKey::generate();
    assert!(!format!("{k:?}").contains("secret"));
}

#[test]
fn membership_persists_and_network_id_is_commitment() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("net/membership.json");
    let mut m = Membership::load_or_create(&p).unwrap();
    let id = m.network_id();
    let peer = NodeKey::generate();
    m.add(Member::new(peer.public(), "phone")).unwrap();
    let m2 = Membership::load_or_create(&p).unwrap();
    assert_eq!(m2.network_id(), id);
    assert_eq!(m2.members().len(), 1);
    assert_eq!(m2.get(&peer.node_id()).unwrap().name, "phone");
    // network_id must not equal or contain the key itself.
    assert!(
        !id.windows(8)
            .any(|w| m.network_key().as_bytes().windows(8).any(|k| k == w))
    );
    let mut m3 = m2;
    m3.remove(&peer.node_id()).unwrap();
    assert!(Membership::load_or_create(&p).unwrap().members().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
