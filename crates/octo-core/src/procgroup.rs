//! Cross-platform "kill the whole process tree". Unix: own process group + killpg.
//! Windows: `taskkill /T /F` (no unsafe, no Job Object FFI).

use tokio::process::Command;

#[cfg(unix)]
mod imp {
    use super::Command;
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    #[derive(Clone, Copy)]
    pub struct Group(Pid);

    pub fn configure(cmd: &mut Command) {
        cmd.process_group(0);
    }

    impl Group {
        pub fn from_pid(pid: u32) -> Option<Self> {
            i32::try_from(pid).ok().map(|p| Self(Pid::from_raw(p)))
        }
        /// Polite stop.
        pub fn terminate(&self) {
            let _ = killpg(self.0, Signal::SIGTERM);
        }
        pub fn kill(&self) {
            let _ = killpg(self.0, Signal::SIGKILL);
        }
        pub fn alive(&self) -> bool {
            !matches!(killpg(self.0, None), Err(Errno::ESRCH))
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::Command;
    use std::process::Stdio;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[derive(Clone, Copy)]
    pub struct Group(u32);

    pub fn configure(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    impl Group {
        pub fn from_pid(pid: u32) -> Option<Self> {
            Some(Self(pid))
        }
        /// No graceful tree stop on Windows; same as `kill`.
        pub fn terminate(&self) {
            self.kill();
        }
        pub fn kill(&self) {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &self.0.to_string()])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        /// The leader is reaped by the caller; descendants are already killed by `kill`.
        pub fn alive(&self) -> bool {
            false
        }
    }
}

pub use imp::{Group, configure};
