use octo_core::providers::ProviderKind;
use octo_core::storage::Space;
use octo_net::membership::Trust;
use octo_server::policy::{TunnelReq, path_allowed, tunnel_allows};

fn req<'a>(method: &'a str, path: &'a str, p: Option<ProviderKind>) -> TunnelReq<'a> {
    TunnelReq {
        method,
        path,
        thread_space: Some(Space::Public),
        thread_provider: p,
        body_private: false,
        body_agents: false,
    }
}

#[test]
fn approval_endpoints_need_the_agents_permission() {
    for (m, p) in [
        ("POST", "/api/threads/t1/approvals/a1"),
        ("GET", "/api/threads/t1/approvals"),
    ] {
        for prov in [Some(ProviderKind::Claude), None] {
            for trust in [Trust::Full, Trust::Public] {
                assert!(
                    !tunnel_allows(trust, false, &req(m, p, prov)),
                    "{m} {p} {trust:?} {prov:?}"
                );
            }
        }
        assert!(
            tunnel_allows(Trust::Full, true, &req(m, p, Some(ProviderKind::Claude))),
            "{m} {p}"
        );
    }
}

#[test]
fn approvals_of_private_threads_stay_private_for_public_trust() {
    let mut r = req(
        "POST",
        "/api/threads/t1/approvals/a1",
        Some(ProviderKind::Claude),
    );
    r.thread_space = Some(Space::Private);
    assert!(!tunnel_allows(Trust::Public, true, &r));
}

#[test]
fn approval_paths_are_reachable_but_the_mcp_route_is_not() {
    assert!(path_allowed("/api/threads/t1/approvals"));
    assert!(path_allowed("/api/threads/t1/approvals/a1"));
    assert!(!path_allowed("/mcp/abcd"));
    assert!(!path_allowed("/api/mcp/abcd"));
}
