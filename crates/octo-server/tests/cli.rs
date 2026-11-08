#![cfg(unix)]
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_octo-server"))
}

fn fails(args: &[&str], data: &std::path::Path) -> String {
    let out = bin()
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .output()
        .unwrap();
    assert!(!out.status.success(), "{args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn bad_configuration_is_an_error_at_startup() {
    let d = tempfile::tempdir().unwrap();
    let e = fails(&["--token", ""], d.path());
    assert!(e.contains("token"), "{e}");
    let f = d.path().join("empty-token");
    std::fs::write(&f, "\n").unwrap();
    let e = fails(&["--token-file", f.to_str().unwrap()], d.path());
    assert!(e.contains("empty"), "{e}");
    let e = fails(
        &["--web-dir", d.path().join("nope").to_str().unwrap()],
        d.path(),
    );
    assert!(e.contains("web-dir"), "{e}");
    // Bad API config must not leave a relay listener bound: fail before binding anything.
    let e = fails(
        &["--addr", "0.0.0.0:0", "--relay-addr", "127.0.0.1:0"],
        d.path(),
    );
    assert!(e.contains("--token"), "{e}");
}

#[test]
fn sigterm_stops_the_server_cleanly() {
    let d = tempfile::tempdir().unwrap();
    let mut child = bin()
        .args(["--addr", "127.0.0.1:0", "--data-dir"])
        .arg(d.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    assert!(line.starts_with("Octo: http://"), "{line}");
    let pid = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        status.success(),
        "exit via signal handler expected, got {status:?}"
    );
}
