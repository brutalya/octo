#![cfg(unix)] // shell-script based (sh, sleep); unix only
use octo_core::CoreError;
use octo_core::registry::{ProcEvent, ProcessRegistry};
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

fn sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(script);
    c
}

async fn gone(pid: u32) -> bool {
    for _ in 0..50 {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn streams_lines_and_exit() {
    let r = ProcessRegistry::default();
    let mut h = r
        .spawn("a", sh("cat; echo done"), "hello\n".into())
        .unwrap();
    let mut got = vec![];
    while let Some(e) = h.events.recv().await {
        got.push(e);
    }
    assert_eq!(got[0], ProcEvent::Line("hello".into()));
    assert_eq!(got[1], ProcEvent::Line("done".into()));
    assert_eq!(
        got[2],
        ProcEvent::Exit {
            success: true,
            stderr: String::new()
        }
    );
    assert!(!r.is_running("a"));
}

#[tokio::test]
async fn busy_per_key() {
    let r = ProcessRegistry::default();
    let _h = r.spawn("a", sh("exec sleep 30"), String::new()).unwrap();
    assert!(matches!(
        r.spawn("a", sh("true"), String::new()),
        Err(CoreError::Busy)
    ));
    assert!(r.spawn("b", sh("true"), String::new()).is_ok());
    assert!(r.cancel("a"));
}

#[tokio::test]
async fn cancel_kills_and_reaps() {
    let r = ProcessRegistry::default();
    let mut h = r
        .spawn("a", sh("echo x; exec sleep 30"), String::new())
        .unwrap();
    let pid = h.pid.unwrap();
    assert_eq!(h.events.recv().await, Some(ProcEvent::Line("x".into())));
    assert!(r.cancel("a"));
    assert_eq!(h.events.recv().await, None, "no Exit after cancel");
    assert!(gone(pid).await, "process must be reaped, not zombie");
    assert!(!r.cancel("a"));
}

#[tokio::test]
async fn failure_reports_stderr() {
    let r = ProcessRegistry::default();
    let mut h = r
        .spawn("a", sh("echo oops >&2; exit 3"), String::new())
        .unwrap();
    let mut last = None;
    while let Some(e) = h.events.recv().await {
        last = Some(e);
    }
    assert_eq!(
        last,
        Some(ProcEvent::Exit {
            success: false,
            stderr: "oops\n".into()
        })
    );
}

fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => !s
            .rsplit(')')
            .next()
            .unwrap_or("")
            .trim_start()
            .starts_with('Z'),
        Err(_) => false,
    }
}

async fn all_gone(pids: &[u32]) -> bool {
    for _ in 0..100 {
        if pids.iter().all(|p| !alive(*p)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

async fn spawn_with_grandchild(
    r: &ProcessRegistry,
    script: &str,
) -> (u32, u32, octo_core::registry::RunHandle) {
    let mut h = r.spawn("g", sh(script), String::new()).unwrap();
    let pid = h.pid.unwrap();
    let Some(ProcEvent::Line(l)) = h.events.recv().await else {
        panic!("expected grandchild pid line");
    };
    (pid, l.trim().parse().unwrap(), h)
}

#[tokio::test]
async fn cancel_kills_whole_process_group() {
    let r = ProcessRegistry::default();
    let (pid, gc, mut h) = spawn_with_grandchild(&r, "sleep 300 & echo $!; wait").await;
    assert!(alive(gc));
    assert!(r.cancel("g"));
    assert_eq!(h.events.recv().await, None);
    assert!(all_gone(&[pid, gc]).await, "child and grandchild must die");
}

#[tokio::test]
async fn cancel_escalates_to_sigkill() {
    let r = ProcessRegistry::default();
    let (pid, gc, mut h) = spawn_with_grandchild(
        &r,
        "trap '' TERM; (trap '' TERM; exec sleep 300) & echo $!; while :; do sleep 1; done",
    )
    .await;
    let start = std::time::Instant::now();
    assert!(r.cancel("g"));
    assert_eq!(h.events.recv().await, None);
    assert!(all_gone(&[pid, gc]).await);
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn dropping_handle_kills_group() {
    let r = ProcessRegistry::default();
    let (pid, gc, h) = spawn_with_grandchild(&r, "sleep 300 & echo $!; wait").await;
    drop(h);
    assert!(all_gone(&[pid, gc]).await);
}
