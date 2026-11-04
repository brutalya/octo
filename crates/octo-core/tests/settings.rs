use octo_core::settings::{Settings, SettingsPatch, mask_key};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn defaults() {
    let s = Settings::default();
    assert_eq!(s.ollama_url, "http://127.0.0.1:11434");
    assert_eq!(s.openrouter_api_key, None);
    assert_eq!(s.history_char_budget, 24_000);
}

#[test]
fn mask() {
    assert_eq!(mask_key("sk-or-v1-abcdef1234"), "sk-...1234");
    assert_eq!(mask_key("short"), "...");
    assert_eq!(mask_key(""), "...");
}

#[test]
fn view_never_contains_full_key() {
    let s = Settings {
        openrouter_api_key: Some("sk-or-v1-SECRETSECRET1234".into()),
        ..Settings::default()
    };
    let json = serde_json::to_string(&s.view()).unwrap();
    assert!(json.contains("sk-...1234"));
    assert!(!json.contains("SECRET"));
    assert!(!format!("{s:?}").contains("SECRET"));
}

#[test]
fn view_without_key_is_null() {
    let v = serde_json::to_value(Settings::default().view()).unwrap();
    assert!(v["openrouter_api_key"].is_null());
}

#[cfg(unix)]
#[test]
fn save_is_0600_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let s = Settings {
        openrouter_api_key: Some("sk-or-abcdefgh".into()),
        history_char_budget: 5000,
        ..Settings::default()
    };
    s.save(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(Settings::load(&path).unwrap(), s);
}

#[test]
fn load_missing_is_default_and_partial_file_fills_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    std::fs::write(&path, r#"{"history_char_budget": 100}"#).unwrap();
    let s = Settings::load(&path).unwrap();
    assert_eq!(s.history_char_budget, 100);
    assert_eq!(s.ollama_url, "http://127.0.0.1:11434");
}

#[test]
fn patch_applies_only_given_fields() {
    let mut s = Settings::default();
    let p: SettingsPatch =
        serde_json::from_str(r#"{"openrouter_api_key":"sk-x-12345678"}"#).unwrap();
    s.apply(p).unwrap();
    assert_eq!(s.openrouter_api_key.as_deref(), Some("sk-x-12345678"));
    assert_eq!(s.history_char_budget, 24_000);
    let p: SettingsPatch = serde_json::from_str(r#"{"openrouter_api_key":""}"#).unwrap();
    s.apply(p).unwrap();
    assert_eq!(s.openrouter_api_key, None);
}

#[test]
fn patch_validates() {
    let mut s = Settings::default();
    for body in [
        r#"{"ollama_url":"ftp://x"}"#,
        r#"{"ollama_url":""}"#,
        r#"{"history_char_budget":0}"#,
    ] {
        let p: SettingsPatch = serde_json::from_str(body).unwrap();
        assert!(s.apply(p).is_err(), "{body}");
    }
    assert_eq!(s, Settings::default());
    let p: SettingsPatch = serde_json::from_str(r#"{"ollama_url":"http://box:11434/"}"#).unwrap();
    s.apply(p).unwrap();
    assert_eq!(s.ollama_url, "http://box:11434");
}

#[test]
fn relay_url_and_node_name_patch() {
    let mut s = Settings::default();
    assert_eq!(s.relay_url, None);
    s.apply(SettingsPatch {
        relay_url: Some(" ws://h:1/relay ".into()),
        node_name: Some(" laptop ".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(s.relay_url.as_deref(), Some("ws://h:1/relay"));
    assert_eq!(s.node_name.as_deref(), Some("laptop"));
    assert!(
        s.apply(SettingsPatch {
            relay_url: Some("http://x".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(
        s.relay_url.as_deref(),
        Some("ws://h:1/relay"),
        "invalid patch changes nothing"
    );
    s.apply(SettingsPatch {
        relay_url: Some("".into()),
        node_name: Some("".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!((s.relay_url, s.node_name), (None, None));
}
