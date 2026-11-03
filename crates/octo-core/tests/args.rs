use octo_core::providers::{CliExtra, ProviderKind, build_args};
use octo_core::storage::Approval;

fn pair(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

#[test]
fn claude_auto_uses_auto_permission_mode() {
    let a = build_args(
        ProviderKind::Claude,
        Approval::Auto,
        None,
        "u1",
        CliExtra::default(),
    );
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("auto"));
    assert_eq!(pair(&a, "--session-id").as_deref(), Some("u1"));
    assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions"));
}

#[test]
fn claude_confirm_uses_default_mode() {
    let a = build_args(
        ProviderKind::Claude,
        Approval::Confirm,
        Some("s9"),
        "u1",
        CliExtra::default(),
    );
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("default"));
    assert_eq!(pair(&a, "--resume").as_deref(), Some("s9"));
    assert!(!a.iter().any(|x| x == "--session-id"));
}

#[test]
fn agy_modes_set_tool_permission_via_settings() {
    for (ap, want) in [
        (Approval::Auto, "always-proceed"),
        (Approval::Confirm, "request-review"),
    ] {
        let a = build_args(
            ProviderKind::Antigravity,
            ap,
            Some("c1"),
            "u1",
            CliExtra::default(),
        );
        let s: serde_json::Value = serde_json::from_str(&pair(&a, "--settings").unwrap()).unwrap();
        assert_eq!(s["toolPermission"], want);
        assert_eq!(pair(&a, "--conversation").as_deref(), Some("c1"));
        assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions"));
    }
}

#[test]
fn approval_default_is_confirm() {
    assert_eq!(Approval::default(), Approval::Confirm);
}

#[test]
fn model_and_add_dir_are_passed_to_both_clis() {
    let dir = std::path::Path::new("/data/public");
    let extra = CliExtra {
        model: Some("opus"),
        add_dir: Some(dir),
    };
    for kind in [ProviderKind::Claude, ProviderKind::Antigravity] {
        let a = build_args(kind, Approval::Confirm, None, "u1", extra);
        assert_eq!(pair(&a, "--model").as_deref(), Some("opus"), "{kind:?}");
        assert_eq!(
            pair(&a, "--add-dir").as_deref(),
            Some("/data/public"),
            "{kind:?}"
        );
        let plain = build_args(kind, Approval::Confirm, None, "u1", CliExtra::default());
        assert!(!plain.iter().any(|x| x == "--model" || x == "--add-dir"));
    }
}

#[test]
fn add_dir_is_never_the_last_variadic_swallowing_a_flag() {
    // claude's --add-dir is variadic: it must be followed by another flag
    let extra = CliExtra {
        model: None,
        add_dir: Some(std::path::Path::new("/p")),
    };
    let a = build_args(ProviderKind::Claude, Approval::Confirm, None, "u1", extra);
    let i = a.iter().position(|x| x == "--add-dir").unwrap();
    assert!(a[i + 2].starts_with("--"), "{a:?}");
}

#[test]
fn http_providers_have_no_cli_args() {
    for kind in [ProviderKind::Ollama, ProviderKind::OpenRouter] {
        assert!(build_args(kind, Approval::Auto, None, "u", CliExtra::default()).is_empty());
    }
}
