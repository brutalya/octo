use octo_core::provider::{ProviderKind, build_args};
use octo_core::storage::Approval;

fn pair(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

#[test]
fn claude_auto_uses_auto_permission_mode() {
    let a = build_args(ProviderKind::Claude, Approval::Auto, None, "u1");
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("auto"));
    assert_eq!(pair(&a, "--session-id").as_deref(), Some("u1"));
    assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions"));
}

#[test]
fn claude_confirm_uses_default_mode() {
    let a = build_args(ProviderKind::Claude, Approval::Confirm, Some("s9"), "u1");
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
        let a = build_args(ProviderKind::Antigravity, ap, Some("c1"), "u1");
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
