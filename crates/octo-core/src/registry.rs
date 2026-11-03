use crate::error::CoreError;
use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
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
struct GroupGuard(Option<Pid>);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if let Some(pg) = self.0 {
            let _ = killpg(pg, Signal::SIGKILL);
        }
    }
}

fn group_alive(pg: Pid) -> bool {
    !matches!(killpg(pg, None), Err(Errno::ESRCH))
}

/// SIGTERM the group, wait up to `TERM_GRACE`, then SIGKILL; always reaps the leader.
async fn kill_group(child: &mut tokio::process::Child, pg: Option<Pid>) {
    let Some(pg) = pg else {
        let _ = child.kill().await;
        return;
    };
    let _ = killpg(pg, Signal::SIGTERM);
    let deadline = tokio::time::Instant::now() + TERM_GRACE;
    loop {
        let reaped = matches!(child.try_wait(), Ok(Some(_)) | Err(_));
        if reaped && !group_alive(pg) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let _ = killpg(pg, Signal::SIGKILL);
    let _ = child.wait().await;
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
            .kill_on_drop(true)
            .process_group(0);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let pgid = pid.and_then(|p| i32::try_from(p).ok()).map(Pid::from_raw);
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
            if let Some(mut si) = stdin_pipe {
                tokio::spawn(async move {
                    let _ = si.write_all(stdin.as_bytes()).await;
                });
            }
            let stderr_task = tokio::spawn(async move {
                let mut buf = String::new();
                if let Some(mut e) = stderr_pipe {
                    let _ = e.read_to_string(&mut buf).await;
                }
                buf
            });
            let Some(stdout) = stdout else {
                kill_group(&mut child, pgid).await;
                registry.map().remove(&key);
                return;
            };
            let mut lines = BufReader::new(stdout).lines();
            let mut cancelled = false;
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
            if cancelled {
                kill_group(&mut child, pgid).await;
            } else {
                let success = child.wait().await.map(|s| s.success()).unwrap_or(false);
                let stderr = stderr_task.await.unwrap_or_default();
                let _ = tx.send(ProcEvent::Exit { success, stderr }).await;
            }
            registry.map().remove(&key);
        });
        Ok(RunHandle { pid, events: rx })
    }
}
