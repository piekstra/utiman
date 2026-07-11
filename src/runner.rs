//! Run a provider CLI and capture its output.
//!
//! Commands are spawned directly (argv array, no shell), inherit the user's
//! environment (the CLIs need HOME / keychain access), and are killed after a
//! timeout so one slow portal can't wedge the dashboard.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Serialize)]
pub struct RunOutcome {
    /// Process exit code; None if killed by a signal or timed out.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl RunOutcome {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

pub const DEFAULT_TIMEOUT_SECS: u64 = 45;

pub async fn run_cli(bin: &Path, args: &[String], timeout: Duration) -> RunOutcome {
    let child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return RunOutcome {
                status: None,
                stdout: String::new(),
                stderr: format!("failed to start {}: {e}", bin.display()),
                timed_out: false,
            }
        }
    };

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => RunOutcome {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            timed_out: false,
        },
        Ok(Err(e)) => RunOutcome {
            status: None,
            stdout: String::new(),
            stderr: format!("i/o error running {}: {e}", bin.display()),
            timed_out: false,
        },
        Err(_) => RunOutcome {
            status: None,
            stdout: String::new(),
            stderr: format!("timed out after {}s (killed)", timeout.as_secs()),
            timed_out: true,
        },
    }
}
