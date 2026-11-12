//! End-to-end: Hub against fake Ollama / OpenRouter servers (axum on a random port).
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use octo_core::providers::{AgentEvent, ProviderKind};
use octo_core::settings::SettingsPatch;
use octo_core::storage::{Approval, AttachmentRef, Role, Space};
use octo_core::{Config, CoreError, Hub};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Receiver;

const KEY: &str = "sk-or-v1-test-key-ABCD1234";

type ChatLog = Arc<Mutex<Vec<(Option<String>, Value)>>>;

#[derive(Default, Clone)]
struct Seen {
    /// (authorization header, json body) per chat request
    chats: ChatLog,
}

impl Seen {
    fn last_body(&self) -> Value {
        self.chats.lock().unwrap().last().unwrap().1.clone()
    }
    fn last_auth(&self) -> Option<String> {
        self.chats.lock().unwrap().last().unwrap().0.clone()
    }
}

fn streaming(chunks: Vec<&'static str>, hang: bool) -> Response {
    let items = tokio_stream::iter(
        chunks
            .into_iter()
            .map(|c| Ok::<Bytes, Infallible>(Bytes::from_static(c.as_bytes()))),
    );
    if hang {
        Body::from_stream(futures_util::StreamExt::chain(
            items,
            futures_util::stream::pending(),
        ))
        .into_response()
    } else {
        Body::from_stream(items).into_response()
    }
}

fn prompt_of(body: &Value) -> String {
    body["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn ollama_chat(State(seen): State<Seen>, h: HeaderMap, body: Bytes) -> Response {
    let v: Value = serde_json::from_slice(&body).unwrap();
    seen.chats.lock().unwrap().push((
        h.get("authorization")
            .and_then(|x| x.to_str().ok())
            .map(String::from),
        v.clone(),
    ));
    if v["model"] == "missing" {
        return (
            StatusCode::NOT_FOUND,
            json!({"error": "model 'missing' not found"}).to_string(),
        )
            .into_response();
    }
    if prompt_of(&v).contains("SLOW") {
        return streaming(
            vec![
                r#"{"message":{"role":"assistant","content":"part"},"done":false}"#,
                "\n",
            ],
            true,
        );
    }
    // chunk boundaries deliberately fall inside a JSON line and inside a multibyte char
    let l1 = "{\"message\":{\"role\":\"assistant\",\"content\":\"Hello \"},\"done\":false}\n";
    let l2 = "{\"message\":{\"role\":\"assistant\",\"content\":\"мир\"},\"done\":false}\n";
    let l3 = "{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n";
    let all: &'static str = Box::leak(format!("{l1}{l2}{l3}").into_boxed_str());
    let cut1 = l1.len() - 10;
    let cut2 = l1.len() + l2.len() - 20;
    let cut2 = cut2 + (0..4).find(|d| all.is_char_boundary(cut2 + d)).unwrap();
    streaming(vec![&all[..cut1], &all[cut1..cut2], &all[cut2..]], false)
}

async fn or_chat(State(seen): State<Seen>, h: HeaderMap, body: Bytes) -> Response {
    let v: Value = serde_json::from_slice(&body).unwrap();
    let auth = h
        .get("authorization")
        .and_then(|x| x.to_str().ok())
        .map(String::from);
    seen.chats.lock().unwrap().push((auth.clone(), v));
    if auth.as_deref() != Some(&format!("Bearer {KEY}")) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"error": {"message": "No auth credentials found", "code": 401}}).to_string(),
        )
            .into_response();
    }
    if prompt_of(&seen.chats.lock().unwrap().last().unwrap().1).contains("CUTOFF") {
        // connection closed mid-answer: no finish_reason, no [DONE]
        return streaming(
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"par\"}}]}\n\n"],
            false,
        );
    }
    if prompt_of(&seen.chats.lock().unwrap().last().unwrap().1).contains("NODONE") {
        // finished (finish_reason) but closed without [DONE]
        return streaming(
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"all\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            ],
            false,
        );
    }
    streaming(
        vec![
            ": OPENROUTER PROCESSING\n\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Hi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ],
        false,
    )
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

async fn fake_ollama() -> (String, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route(
            "/api/tags",
            get(|| async {
                json!({"models": [{"name": "llama3:8b"}, {"name": "qwen2"}]}).to_string()
            }),
        )
        .route("/api/chat", post(ollama_chat))
        .with_state(seen.clone());
    (serve(app).await, seen)
}

async fn fake_openrouter() -> (String, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route(
            "/models",
            get(|| async {
                json!({"data": [{"id": "openai/gpt-x", "name": "OpenAI: GPT X"}]}).to_string()
            }),
        )
        .route("/chat/completions", post(or_chat))
        .with_state(seen.clone());
    (serve(app).await, seen)
}

struct Env {
    hub: Hub,
    _dir: tempfile::TempDir,
}

fn env(ollama: &str, openrouter: &str, key: Option<&str>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(Some(dir.path().to_path_buf()));
    cfg.openrouter_base = openrouter.to_string();
    let hub = Hub::new(cfg, "test-node").unwrap();
    hub.update_settings(SettingsPatch {
        ollama_url: Some(ollama.to_string()),
        openrouter_api_key: key.map(String::from),
        ..Default::default()
    })
    .unwrap();
    Env { hub, _dir: dir }
}

async fn collect(mut rx: Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut v = vec![];
    while let Some(e) = rx.recv().await {
        v.push(e);
    }
    v
}

fn text(evs: &[AgentEvent]) -> String {
    evs.iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

async fn wait_idle(hub: &Hub, id: &str) {
    // the run entry is removed right after the final persist
    for _ in 0..300 {
        if !hub.is_running(id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("run did not finish");
}

#[tokio::test]
async fn ollama_streams_persists_and_resends_history() {
    let (url, seen) = fake_ollama().await;
    let e = env(&url, "", None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("llama3:8b"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "hi", &[]).await.unwrap()).await;
    assert_eq!(text(&evs), "Hello мир");
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
    let b = seen.last_body();
    assert_eq!(b["model"], "llama3:8b");
    assert_eq!(b["stream"], true);
    assert_eq!(b["messages"], json!([{"role": "user", "content": "hi"}]));
    assert_eq!(seen.last_auth(), None, "no key to the local server");
    wait_idle(&e.hub, &t.id).await;

    collect(e.hub.send_message(&t.id, "again", &[]).await.unwrap()).await;
    assert_eq!(
        seen.last_body()["messages"],
        json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "Hello мир"},
            {"role": "user", "content": "again"}
        ])
    );
    wait_idle(&e.hub, &t.id).await;
    let msgs = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[1].role, Role::Assistant);
    assert_eq!(e.hub.list_threads().unwrap()[0].title, "hi");
}

#[tokio::test]
async fn history_budget_drops_oldest() {
    let (url, seen) = fake_ollama().await;
    let e = env(&url, "", None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    collect(
        e.hub
            .send_message(&t.id, "first-question", &[])
            .await
            .unwrap(),
    )
    .await;
    wait_idle(&e.hub, &t.id).await;
    e.hub
        .update_settings(SettingsPatch {
            history_char_budget: Some(20),
            ..Default::default()
        })
        .unwrap();
    collect(e.hub.send_message(&t.id, "second", &[]).await.unwrap()).await;
    assert_eq!(
        seen.last_body()["messages"],
        json!([
            {"role": "assistant", "content": "Hello мир"},
            {"role": "user", "content": "second"}
        ])
    );
}

#[tokio::test]
async fn ollama_error_is_streamed_and_stored_as_error() {
    let (url, _) = fake_ollama().await;
    let e = env(&url, "", None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("missing"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "hi", &[]).await.unwrap()).await;
    assert_eq!(
        evs,
        vec![AgentEvent::Error {
            message: "model 'missing' not found".into()
        }]
    );
    wait_idle(&e.hub, &t.id).await;
    let m = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(m.last().unwrap().role, Role::Error);
}

#[tokio::test]
async fn unreachable_ollama_reports_error_event() {
    let e = env("http://127.0.0.1:1", "", None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "hi", &[]).await.unwrap()).await;
    assert!(
        matches!(&evs[..], [AgentEvent::Error { message }] if message.contains("cannot reach Ollama"))
    );
}

#[tokio::test]
async fn openrouter_streams_with_bearer_key_and_ignores_comments() {
    let (url, seen) = fake_openrouter().await;
    let e = env("http://127.0.0.1:1", &url, Some(KEY));
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::OpenRouter,
            Space::Public,
            Approval::Confirm,
            Some("openai/gpt-x"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "yo", &[]).await.unwrap()).await;
    assert_eq!(text(&evs), "Hi there");
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
    assert_eq!(seen.last_auth().as_deref(), Some(&*format!("Bearer {KEY}")));
    assert_eq!(seen.last_body()["model"], "openai/gpt-x");
    wait_idle(&e.hub, &t.id).await;
    assert_eq!(e.hub.list_messages(&t.id).unwrap()[1].content, "Hi there");
}

#[tokio::test]
async fn openrouter_auth_failure_is_an_error_without_leaking_the_key() {
    let (url, _) = fake_openrouter().await;
    let e = env("http://127.0.0.1:1", &url, Some("sk-or-wrong-key-0000"));
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::OpenRouter,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "yo", &[]).await.unwrap()).await;
    assert_eq!(
        evs,
        vec![AgentEvent::Error {
            message: "No auth credentials found".into()
        }]
    );
    wait_idle(&e.hub, &t.id).await;
    let all = format!("{:?}", e.hub.list_messages(&t.id).unwrap());
    assert!(!all.contains("wrong-key"));
}

#[tokio::test]
async fn openrouter_without_key_fails_fast_and_stores_nothing() {
    let (url, seen) = fake_openrouter().await;
    let e = env("http://127.0.0.1:1", &url, None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::OpenRouter,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let r = e.hub.send_message(&t.id, "yo", &[]).await;
    assert!(matches!(r, Err(CoreError::Invalid(_))));
    assert!(e.hub.list_messages(&t.id).unwrap().is_empty());
    assert!(seen.chats.lock().unwrap().is_empty());
}

#[tokio::test]
async fn policy_private_thread_rejects_openrouter_at_create() {
    let e = env("http://127.0.0.1:1", "http://127.0.0.1:1", Some(KEY));
    for p in [
        ProviderKind::OpenRouter,
        ProviderKind::Claude,
        ProviderKind::Antigravity,
    ] {
        let r = e
            .hub
            .create_thread_in(p, Space::Private, Approval::Confirm, Some("m"), None);
        assert!(matches!(r, Err(CoreError::Policy(_))), "{p:?}");
    }
}

#[tokio::test]
async fn http_provider_requires_a_model() {
    let e = env("http://127.0.0.1:1", "http://127.0.0.1:1", Some(KEY));
    for model in [None, Some(""), Some("  ")] {
        let r = e.hub.create_thread_in(
            ProviderKind::Ollama,
            Space::Public,
            Approval::Confirm,
            model,
            None,
        );
        assert!(matches!(r, Err(CoreError::Invalid(_))), "{model:?}");
    }
}

#[tokio::test]
async fn providers_and_models_lists() {
    let (ou, _) = fake_ollama().await;
    let (ru, _) = fake_openrouter().await;
    let e = env(&ou, &ru, Some(KEY));
    let st = e.hub.providers().await;
    let get = |k| st.iter().find(|s| s.kind == k).unwrap();
    assert!(get(ProviderKind::Ollama).installed && get(ProviderKind::Ollama).is_local);
    assert!(get(ProviderKind::OpenRouter).installed && !get(ProviderKind::OpenRouter).is_local);
    let o = e.hub.models(ProviderKind::Ollama).await.unwrap();
    assert_eq!(
        o.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["llama3:8b", "qwen2"]
    );
    let r = e.hub.models(ProviderKind::OpenRouter).await.unwrap();
    assert_eq!(
        (r[0].id.as_str(), r[0].name.as_str()),
        ("openai/gpt-x", "OpenAI: GPT X")
    );
    assert!(e.hub.models(ProviderKind::Claude).await.unwrap().is_empty());

    let down = env("http://127.0.0.1:1", "http://127.0.0.1:1", None);
    let st = down.hub.providers().await;
    assert!(
        !st.iter()
            .find(|s| s.kind == ProviderKind::Ollama)
            .unwrap()
            .installed
    );
    assert!(
        !st.iter()
            .find(|s| s.kind == ProviderKind::OpenRouter)
            .unwrap()
            .installed
    );
    assert!(matches!(
        down.hub.models(ProviderKind::Ollama).await,
        Err(CoreError::Upstream(_))
    ));
}

#[tokio::test]
async fn cancel_stops_http_stream_keeps_partial_and_never_reports_done() {
    let (url, _) = fake_ollama().await;
    let e = env(&url, "", None);
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let mut rx = e.hub.send_message(&t.id, "SLOW", &[]).await.unwrap();
    assert_eq!(
        rx.recv().await,
        Some(AgentEvent::TextDelta {
            text: "part".into()
        })
    );
    assert!(matches!(
        e.hub.send_message(&t.id, "second", &[]).await,
        Err(CoreError::Busy)
    ));
    e.hub.cancel(&t.id);
    assert_eq!(rx.recv().await, None);
    wait_idle(&e.hub, &t.id).await;
    let m = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(
        (m[1].role, m[1].content.as_str()),
        (Role::Assistant, "part")
    );
}

#[tokio::test]
async fn attachments_are_inlined_and_stored_as_refs() {
    let (url, seen) = fake_ollama().await;
    let e = env(&url, "", None);
    e.hub
        .files()
        .write(Space::Private, "docs/a.txt", "секрет".as_bytes())
        .unwrap();
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let refs = vec![AttachmentRef {
        space: Space::Private,
        path: "docs/a.txt".into(),
    }];
    collect(e.hub.send_message(&t.id, "read it", &refs).await.unwrap()).await;
    assert_eq!(
        prompt_of(&seen.last_body()),
        "read it\n\ndocs/a.txt:\n```\nсекрет\n```"
    );
    wait_idle(&e.hub, &t.id).await;
    let m = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(m[0].content, "read it");
    assert_eq!(m[0].attachments, refs);
}

#[tokio::test]
async fn attachment_policy_and_media_checks_happen_before_anything_is_sent_or_stored() {
    let (url, seen) = fake_ollama().await;
    let e = env(&url, "", None);
    e.hub
        .files()
        .write(Space::Private, "p.txt", b"private")
        .unwrap();
    e.hub
        .files()
        .write(Space::Public, "bin.dat", &[0xff, 0x00, 0x01])
        .unwrap();
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let cross = [AttachmentRef {
        space: Space::Private,
        path: "p.txt".into(),
    }];
    assert!(matches!(
        e.hub.send_message(&t.id, "x", &cross).await,
        Err(CoreError::Policy(_))
    ));
    let bin = [AttachmentRef {
        space: Space::Public,
        path: "bin.dat".into(),
    }];
    assert!(matches!(
        e.hub.send_message(&t.id, "x", &bin).await,
        Err(CoreError::UnsupportedMedia(_))
    ));
    let missing = [AttachmentRef {
        space: Space::Public,
        path: "nope.txt".into(),
    }];
    assert!(e.hub.send_message(&t.id, "x", &missing).await.is_err());
    let evil = [AttachmentRef {
        space: Space::Public,
        path: "../x".into(),
    }];
    assert!(e.hub.send_message(&t.id, "x", &evil).await.is_err());
    // a private thread cannot pull in public files either
    let pt = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let pubref = [AttachmentRef {
        space: Space::Public,
        path: "bin.dat".into(),
    }];
    assert!(matches!(
        e.hub.send_message(&pt.id, "x", &pubref).await,
        Err(CoreError::Policy(_))
    ));
    assert!(e.hub.list_messages(&t.id).unwrap().is_empty());
    assert!(seen.chats.lock().unwrap().is_empty());
    // and the failed attempts did not leave the thread busy
    assert!(!e.hub.is_running(&t.id));
    collect(e.hub.send_message(&t.id, "fine", &[]).await.unwrap()).await;
}

#[cfg(unix)]
#[tokio::test]
async fn settings_persist_with_0600_and_reload() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::new(Some(dir.path().to_path_buf()));
    let hub = Hub::new(cfg.clone(), "test-node").unwrap();
    let v = hub
        .update_settings(SettingsPatch {
            openrouter_api_key: Some(KEY.into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.openrouter_api_key.as_deref(), Some("sk-...1234"));
    assert_eq!(
        std::fs::metadata(cfg.settings_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(hub);
    let again = Hub::new(cfg, "test-node").unwrap();
    assert_eq!(
        again.settings().openrouter_api_key.as_deref(),
        Some("sk-...1234")
    );
    let bad = again.update_settings(SettingsPatch {
        ollama_url: Some("nope".into()),
        ..Default::default()
    });
    assert!(matches!(bad, Err(CoreError::Settings(_))));
}

#[tokio::test]
async fn openrouter_eof_without_finish_is_an_error_but_finish_without_done_is_done() {
    let (url, _seen) = fake_openrouter().await;
    let e = env("http://127.0.0.1:1", &url, Some(KEY));
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::OpenRouter,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let evs = collect(e.hub.send_message(&t.id, "CUTOFF", &[]).await.unwrap()).await;
    assert!(
        matches!(evs.last(), Some(AgentEvent::Error { .. })),
        "{evs:?}"
    );
    assert!(!evs.contains(&AgentEvent::Done));
    wait_idle(&e.hub, &t.id).await;
    let evs = collect(e.hub.send_message(&t.id, "NODONE", &[]).await.unwrap()).await;
    assert_eq!(text(&evs), "all");
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
}

// ---- stage 6: images and PDF ----
use base64::Engine as _;

type Shows = Arc<Mutex<Vec<Value>>>;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01";

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

async fn fake_ollama_caps(caps: &'static [&'static str]) -> (String, Seen, Shows) {
    let seen = Seen::default();
    let shows = Shows::default();
    let s2 = shows.clone();
    let app = Router::new()
        .route(
            "/api/show",
            post(move |body: Bytes| {
                let shows = s2.clone();
                async move {
                    shows
                        .lock()
                        .unwrap()
                        .push(serde_json::from_slice(&body).unwrap());
                    json!({"capabilities": caps}).to_string()
                }
            }),
        )
        .route("/api/chat", post(ollama_chat))
        .with_state(seen.clone());
    (serve(app).await, seen, shows)
}

fn thread(e: &Env, kind: ProviderKind, space: Space, model: &str) -> String {
    e.hub
        .create_thread_in(kind, space, Approval::Confirm, Some(model), None)
        .unwrap()
        .id
}

fn aref(space: Space, path: &str) -> AttachmentRef {
    AttachmentRef {
        space,
        path: path.into(),
    }
}

#[tokio::test]
async fn ollama_gets_images_as_base64_and_vision_is_checked_once() {
    let (url, seen, shows) = fake_ollama_caps(&["completion", "vision"]).await;
    let e = env(&url, "", None);
    e.hub.files().write(Space::Private, "p/a.png", PNG).unwrap();
    let id = thread(&e, ProviderKind::Ollama, Space::Private, "llava");
    let refs = [aref(Space::Private, "p/a.png")];
    let evs = collect(e.hub.send_message(&id, "describe", &refs).await.unwrap()).await;
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
    let msgs = seen.last_body()["messages"].clone();
    assert_eq!(
        msgs,
        json!([{"role": "user", "content": "describe", "images": [b64(PNG)]}])
    );
    assert_eq!(shows.lock().unwrap().len(), 1);
    assert_eq!(shows.lock().unwrap()[0]["model"], "llava");
    wait_idle(&e.hub, &id).await;

    // second image turn: capability is cached, the earlier turn is re-sent without its image
    collect(e.hub.send_message(&id, "and again", &refs).await.unwrap()).await;
    let msgs = seen.last_body()["messages"].clone();
    assert_eq!(msgs[0], json!({"role": "user", "content": "describe"}));
    assert_eq!(msgs[2]["images"], json!([b64(PNG)]));
    assert_eq!(shows.lock().unwrap().len(), 1, "cached per model");
    wait_idle(&e.hub, &id).await;
    // the stored message keeps the plain text and the ref
    let m = e.hub.list_messages(&id).unwrap();
    assert_eq!(
        (m[0].content.as_str(), m[0].attachments.clone()),
        ("describe", refs.to_vec())
    );
}

#[tokio::test]
async fn ollama_text_only_turns_never_call_show() {
    let (url, _seen, shows) = fake_ollama_caps(&["completion"]).await;
    let e = env(&url, "", None);
    let id = thread(&e, ProviderKind::Ollama, Space::Private, "m");
    collect(e.hub.send_message(&id, "hi", &[]).await.unwrap()).await;
    assert!(shows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_without_vision_rejects_images_and_stores_nothing() {
    let (url, seen, shows) = fake_ollama_caps(&["completion"]).await;
    let e = env(&url, "", None);
    e.hub.files().write(Space::Private, "a.png", PNG).unwrap();
    let id = thread(&e, ProviderKind::Ollama, Space::Private, "llama3");
    let r = e
        .hub
        .send_message(&id, "describe", &[aref(Space::Private, "a.png")])
        .await;
    assert!(matches!(r, Err(CoreError::UnsupportedMedia(_))));
    assert_eq!(
        shows.lock().unwrap().len(),
        1,
        "rejected because of the capability check"
    );
    assert!(seen.chats.lock().unwrap().is_empty());
    assert!(e.hub.list_messages(&id).unwrap().is_empty());
    assert!(!e.hub.is_running(&id));
}

#[tokio::test]
async fn pdf_text_is_inlined_like_a_text_attachment() {
    let (url, seen, shows) = fake_ollama_caps(&["completion"]).await;
    let e = env(&url, "", None);
    let pdf = crate_pdf(&["Hemoglobin 14.2 g/dL"]);
    e.hub
        .files()
        .write(Space::Private, "lab.pdf", &pdf)
        .unwrap();
    let id = thread(&e, ProviderKind::Ollama, Space::Private, "llama3");
    collect(
        e.hub
            .send_message(&id, "summarise", &[aref(Space::Private, "lab.pdf")])
            .await
            .unwrap(),
    )
    .await;
    let p = prompt_of(&seen.last_body());
    assert!(p.starts_with("summarise\n\nlab.pdf:\n```\n"), "{p}");
    assert!(p.contains("Hemoglobin 14.2 g/dL"), "{p}");
    assert!(
        !p.contains("%PDF") && !p.contains("Tj"),
        "raw PDF syntax leaked: {p}"
    );
    assert!(seen.last_body()["messages"][0].get("images").is_none());
    assert!(
        shows.lock().unwrap().is_empty(),
        "a pdf needs no vision model"
    );
}

#[tokio::test]
async fn scanned_pdf_is_rejected_before_anything_is_stored() {
    let (url, seen, _s) = fake_ollama_caps(&["completion"]).await;
    let e = env(&url, "", None);
    e.hub
        .files()
        .write(Space::Public, "scan.pdf", &crate_pdf(&[]))
        .unwrap();
    let id = thread(&e, ProviderKind::Ollama, Space::Public, "m");
    let r = e
        .hub
        .send_message(&id, "read", &[aref(Space::Public, "scan.pdf")])
        .await;
    assert!(matches!(r, Err(CoreError::UnsupportedMedia(_))));
    assert!(seen.chats.lock().unwrap().is_empty());
    assert!(e.hub.list_messages(&id).unwrap().is_empty());
}

/// Same minimal PDF as tests/attachments.rs; no lines means a page without a text layer.
fn crate_pdf(lines: &[&str]) -> Vec<u8> {
    let mut content = String::new();
    if !lines.is_empty() {
        content.push_str("BT /F1 10 Tf 12 TL 40 800 Td\n");
        for l in lines {
            content.push_str(&format!("({l}) Tj T*\n"));
        }
        content.push_str("ET");
    }
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[tokio::test]
async fn openrouter_gets_image_content_parts() {
    let seen = Seen::default();
    let app = Router::new()
        .route(
            "/chat/completions",
            post(|State(seen): State<Seen>, body: Bytes| async move {
                let v: Value = serde_json::from_slice(&body).unwrap();
                seen.chats.lock().unwrap().push((None, v));
                streaming(
                    vec![
                        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n",
                    ],
                    false,
                )
            }),
        )
        .with_state(seen.clone());
    let url = serve(app).await;
    let e = env("http://127.0.0.1:1", &url, Some(KEY));
    e.hub.files().write(Space::Public, "a.png", PNG).unwrap();
    let id = thread(&e, ProviderKind::OpenRouter, Space::Public, "vendor/vision");
    let evs = collect(
        e.hub
            .send_message(&id, "look", &[aref(Space::Public, "a.png")])
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
    assert_eq!(
        seen.last_body()["messages"],
        json!([{"role": "user", "content": [
            {"type": "text", "text": "look"},
            {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64(PNG))}}
        ]}])
    );
}

#[cfg(unix)] // the fake CLI is a shell script
#[tokio::test]
async fn cli_provider_accepts_a_public_image() {
    // the CLI gets "Attached image: <path>" in the prompt (unit-tested); here it must not be refused
    let dir = tempfile::tempdir().unwrap();
    let fake = std::path::PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_cli.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake.clone(),
        agy_bin: fake,
        ..Config::new(None)
    };
    let hub = Hub::new(cfg, "test-node").unwrap();
    hub.files().write(Space::Public, "a.png", PNG).unwrap();
    let t = hub
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    let mut rx = hub
        .send_message(&t.id, "look", &[aref(Space::Public, "a.png")])
        .await
        .unwrap();
    while rx.recv().await.is_some() {}
}
