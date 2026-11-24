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

/// First line containing `needle` on stdout; the child is killed afterwards.
fn stdout_line(args: &[&str], needle: &str) -> String {
    let mut child = bin().args(args).stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut found = String::new();
    for _ in 0..10 {
        let mut line = String::new();
        if lines.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.contains(needle) {
            found = line;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    found
}

/// A private relay prints the code that Settings, Create a network takes: its public URL
/// (or the local one) with the key.
#[test]
fn private_relay_prints_its_relay_code() {
    println!("[Step 1/3] relay with a secret and a public URL");
    let d = tempfile::tempdir().unwrap();
    let secret = d.path().join("relay-secret");
    std::fs::write(&secret, "s3cr3t\n").unwrap();
    let s = secret.to_str().unwrap();
    let line = stdout_line(
        &[
            "--relay-only",
            "--relay-addr",
            "127.0.0.1:0",
            "--relay-secret-file",
            s,
            "--relay-public-url",
            "wss://relay.example.com/relay",
        ],
        "relay code",
    );
    assert_eq!(
        line.trim(),
        "Octo relay code: wss://relay.example.com/relay?key=s3cr3t"
    );

    println!("[Step 2/3] without a public URL the local address is used");
    let line = stdout_line(
        &[
            "--relay-only",
            "--relay-addr",
            "127.0.0.1:0",
            "--relay-secret-file",
            s,
        ],
        "relay code",
    );
    assert!(
        line.starts_with("Octo relay code: ws://127.0.0.1:"),
        "{line}"
    );
    assert!(line.trim().ends_with("/relay?key=s3cr3t"), "{line}");

    println!("[Step 3/3] a public URL that is not ws:// or wss:// is a startup error");
    let e = fails(
        &[
            "--relay-only",
            "--relay-addr",
            "127.0.0.1:0",
            "--relay-public-url",
            "https://relay.example.com",
        ],
        d.path(),
    );
    assert!(e.contains("relay-public-url"), "{e}");
}
