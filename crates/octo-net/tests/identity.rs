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

#[test]
fn concurrent_load_or_create_agree_and_never_see_partial_key() {
    for _ in 0..20 {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("node.key");
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let p = p.clone();
                std::thread::spawn(move || NodeKey::load_or_create(&p).unwrap().public())
            })
            .collect();
        let keys: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(keys.iter().all(|k| *k == keys[0]));
        let left: Vec<_> = std::fs::read_dir(d.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "temp files must be cleaned up");
    }
}

#[test]
fn failed_save_leaves_membership_unchanged() {
    use octo_net::membership::Trust;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.json");
    let mut m = Membership::load_or_create(&p).unwrap();
    let (a, b) = (NodeKey::generate(), NodeKey::generate());
    m.add(Member::new(a.public(), "a")).unwrap();
    let epoch = m.private_epoch();
    // Make saving impossible: the target becomes a non-empty directory.
    std::fs::remove_file(&p).unwrap();
    std::fs::create_dir_all(p.join("x")).unwrap();
    assert!(m.add(Member::new(b.public(), "b")).is_err());
    assert!(m.get(&b.node_id()).is_none());
    assert!(m.set_trust(&a.node_id(), Trust::Full).is_err());
    assert_eq!(m.get(&a.node_id()).unwrap().trust, Trust::Public);
    assert!(m.remove(&a.node_id()).is_err());
    assert!(m.get(&a.node_id()).is_some());
    assert!(m.rotate_private_key().is_err());
    assert_eq!(m.private_epoch(), epoch);
    let k = octo_net::membership::PrivateKey::generate();
    assert!(m.adopt_private(epoch + 5, &k).is_err());
    assert_eq!(m.private_epoch(), epoch);
}

#[test]
fn display_names_are_validated_by_chars() {
    use octo_net::membership::valid_display_name;
    assert!(valid_display_name(&"é".repeat(64)));
    assert!(!valid_display_name(&"é".repeat(65)));
    assert!(!valid_display_name("a\nb"));
    assert!(!valid_display_name("a\u{7}"));
    let mut m = Member::new(NodeKey::generate().public(), "x");
    m.name = "bad\u{0}".into();
    assert!(m.validate().is_err());
    assert!(Member::new(NodeKey::generate().public(), "ok\nname").name == "okname");
}
