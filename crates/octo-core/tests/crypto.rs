use octo_core::crypto::{CryptoError, Key, decrypt, encrypt};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn key() -> Key {
    Key::from_bytes([7u8; 32])
}

#[test]
fn roundtrip() {
    let ct = encrypt(&key(), b"secret text", b"aad").unwrap();
    assert_eq!(decrypt(&key(), &ct, b"aad").unwrap(), b"secret text");
}

#[test]
fn roundtrip_empty_and_large() {
    for n in [0usize, 1, 1_000_000] {
        let pt = vec![42u8; n];
        let ct = encrypt(&key(), &pt, b"").unwrap();
        assert_eq!(ct.len(), 24 + n + 16);
        assert_eq!(decrypt(&key(), &ct, b"").unwrap(), pt);
    }
}

#[test]
fn nonce_is_random_per_call() {
    let a = encrypt(&key(), b"same", b"").unwrap();
    let b = encrypt(&key(), b"same", b"").unwrap();
    assert_ne!(a[..24], b[..24]);
    assert_ne!(a, b);
}

#[test]
fn plaintext_not_visible_in_ciphertext() {
    let ct = encrypt(&key(), b"needle-needle-needle", b"").unwrap();
    assert!(!ct.windows(6).any(|w| w == b"needle"));
}

#[test]
fn tamper_detection_every_region() {
    let ct = encrypt(&key(), b"hello world", b"").unwrap();
    for i in [0, 23, 24, ct.len() - 1] {
        let mut bad = ct.clone();
        bad[i] ^= 1;
        assert!(
            matches!(decrypt(&key(), &bad, b""), Err(CryptoError::Decrypt)),
            "byte {i}"
        );
    }
}

#[test]
fn rejects_truncated_wrong_key_wrong_aad() {
    let ct = encrypt(&key(), b"hello", b"a").unwrap();
    assert!(decrypt(&key(), &ct[..10], b"a").is_err());
    assert!(decrypt(&key(), &[], b"a").is_err());
    assert!(decrypt(&Key::from_bytes([8u8; 32]), &ct, b"a").is_err());
    assert!(decrypt(&key(), &ct, b"b").is_err());
}

#[cfg(unix)]
#[test]
fn key_file_created_0600_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys/private.key");
    let k1 = Key::load_or_create(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 32);
    let ct = encrypt(&k1, b"x", b"").unwrap();
    let k2 = Key::load_or_create(&path).unwrap();
    assert_eq!(decrypt(&k2, &ct, b"").unwrap(), b"x");
}

#[test]
fn key_file_with_wrong_length_is_an_error_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("k");
    std::fs::write(&path, b"short").unwrap();
    assert!(Key::load_or_create(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"short");
}

#[test]
fn key_debug_does_not_leak() {
    assert!(!format!("{:?}", key()).contains('7'));
}

#[test]
fn concurrent_load_or_create_agrees_and_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys").join("k.key");
    let keys: Vec<Vec<u8>> = (0..16)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || {
                let k = Key::load_or_create(&p).unwrap();
                encrypt(&k, b"probe", b"").unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let k = Key::load_or_create(&path).unwrap();
    for ct in keys {
        assert_eq!(decrypt(&k, &ct, b"").unwrap(), b"probe");
    }
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}
