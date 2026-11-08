use crate::error::CoreError;
use crate::procgroup::{self, Group};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, PartialEq, Eq)]
pub enum ProcEvent {
    Line(String),
    /// Sent when the process ended on its own. Not sent after cancel.
    Exit {
        success: bool,
        stderr: String,
    },
}

pub struct RunHandle {
    pub pid: Option<u32>,
    pub events: mpsc::Receiver<ProcEvent>,
}

type Running = Arc<Mutex<HashMap<String, Option<oneshot::Sender<()>>>>>;

const TERM_GRACE: Duration = Duration::from_secs(2);

/// Kills the whole process group when dropped (task aborted, runtime shutdown).
struct GroupGuard(Option<Group>);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if let Some(pg) = self.0 {
            pg.kill();
        }
    }
}

/// SIGTERM the group, wait up to `TERM_GRACE`, then SIGKILL; always reaps the leader.
async fn kill_group(child: &mut tokio::process::Child, pg: Option<Group>) {
    let Some(pg) = pg else {
        let _ = child.kill().await;
        return;
    };
    pg.terminate();
    let deadline = tokio::time::Instant::now() + TERM_GRACE;
    loop {
        let reaped = matches!(child.try_wait(), Ok(Some(_)) | Err(_));
        if reaped && !pg.alive() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    pg.kill();
    let _ = child.wait().await;
}

const STDERR_CAP: usize = 64 * 1024;

/// Reads stderr up to `STDERR_CAP` bytes; the rest is drained and dropped so the child
/// never blocks on a full pipe.
async fn read_capped(pipe: Option<tokio::process::ChildStderr>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = STDERR_CAP.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Owns every child process: one per key, cancellable, always reaped.
#[derive(Default, Clone)]
pub struct ProcessRegistry {
    running: Running,
}

impl ProcessRegistry {
    pub fn is_running(&self, key: &str) -> bool {
        self.map().contains_key(key)
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<oneshot::Sender<()>>>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Returns true if a run existed and was asked to stop.
    pub fn cancel(&self, key: &str) -> bool {
        match self.map().get_mut(key).and_then(Option::take) {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }

    pub fn spawn(
        &self,
        key: &str,
        mut cmd: Command,
        stdin: String,
    ) -> Result<RunHandle, CoreError> {
        let mut map = self.map();
        if map.contains_key(key) {
            return Err(CoreError::Busy);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        procgroup::configure(&mut cmd);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let pgid = pid.and_then(Group::from_pid);
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        map.insert(key.to_string(), Some(cancel_tx));
        drop(map);

        let stdin_pipe = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let (tx, rx) = mpsc::channel(64);
        let registry = self.clone();
        let key = key.to_string();

        tokio::spawn(async move {
            let _guard = GroupGuard(pgid);
            let stdin_task = stdin_pipe.map(|mut si| {
                tokio::spawn(async move {
                    let _ = si.write_all(stdin.as_bytes()).await;
                })
            });
            let mut stderr_task = tokio::spawn(read_capped(stderr_pipe));
            let mut cancelled = false;
            if let Some(stdout) = stdout {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    tokio::select! {
                        _ = &mut cancel_rx => { cancelled = true; break; }
                        _ = tx.closed() => { cancelled = true; break; }
                        line = lines.next_line() => match line {
                            Ok(Some(l)) => {
                                if tx.send(ProcEvent::Line(l)).await.is_err() { cancelled = true; break; }
                            }
                            _ => break,
                        },
                    }
                }
            }
            // stdout is done (EOF or error); the process may still run: wait for it, but
            // stay cancellable.
            let mut status = None;
            if !cancelled {
                tokio::select! {
                    _ = &mut cancel_rx => cancelled = true,
                    _ = tx.closed() => cancelled = true,
                    s = child.wait() => status = Some(s.map(|s| s.success()).unwrap_or(false)),
                }
            }
            if let Some(t) = &stdin_task {
                t.abort();
            }
            if cancelled {
                kill_group(&mut child, pgid).await;
                stderr_task.abort();
            } else {
                // a grandchild may keep stderr open: do not wait for it forever
                let stderr = match tokio::time::timeout(TERM_GRACE, &mut stderr_task).await {
                    Ok(r) => r.unwrap_or_default(),
                    Err(_) => {
                        stderr_task.abort();
                        String::new()
                    }
                };
                let success = status.unwrap_or(false);
                let _ = tx.send(ProcEvent::Exit { success, stderr }).await;
            }
            registry.map().remove(&key);
        });
        Ok(RunHandle { pid, events: rx })
    }
}
