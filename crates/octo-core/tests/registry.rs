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
