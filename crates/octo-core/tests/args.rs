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
        ..CliExtra::default()
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
        ..CliExtra::default()
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

const URL: &str = "http://127.0.0.1:4242/mcp/tok";

fn claude_with(approval: Approval, kind: ProviderKind, url: Option<&str>) -> Vec<String> {
    let extra = CliExtra {
        mcp_url: url,
        ..CliExtra::default()
    };
    build_args(kind, approval, None, "u1", extra)
}

#[test]
fn claude_confirm_with_endpoint_uses_the_mcp_permission_prompt() {
    let a = claude_with(Approval::Confirm, ProviderKind::Claude, Some(URL));
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("default"));
    assert_eq!(
        pair(&a, "--permission-prompt-tool").as_deref(),
        Some("mcp__octo__approve")
    );
    let cfg: serde_json::Value = serde_json::from_str(&pair(&a, "--mcp-config").unwrap()).unwrap();
    assert_eq!(cfg["mcpServers"]["octo"]["type"], "http");
    assert_eq!(cfg["mcpServers"]["octo"]["url"], URL);
    assert!(a.iter().any(|x| x == "--strict-mcp-config"));
    // --mcp-config is variadic: the next argument must be a flag
    let i = a.iter().position(|x| x == "--mcp-config").unwrap();
    assert!(a[i + 2].starts_with("--"), "{a:?}");
    assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions"));
}

#[test]
fn claude_auto_never_uses_the_prompt_tool() {
    let a = claude_with(Approval::Auto, ProviderKind::Claude, Some(URL));
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("auto"));
    for f in [
        "--permission-prompt-tool",
        "--mcp-config",
        "--strict-mcp-config",
    ] {
        assert!(!a.iter().any(|x| x == f), "{f}");
    }
}

#[test]
fn claude_confirm_without_endpoint_stays_non_interactive() {
    let a = claude_with(Approval::Confirm, ProviderKind::Claude, None);
    assert_eq!(pair(&a, "--permission-mode").as_deref(), Some("default"));
    assert!(!a.iter().any(|x| x == "--permission-prompt-tool"));
}

#[test]
fn agy_ignores_the_endpoint() {
    let with = claude_with(Approval::Confirm, ProviderKind::Antigravity, Some(URL));
    let without = claude_with(Approval::Confirm, ProviderKind::Antigravity, None);
    assert_eq!(with, without);
}
