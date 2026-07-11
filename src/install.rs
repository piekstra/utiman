//! Background install/update tasks.
//!
//! Installing a provider CLI shells out to `cargo install` (or the manifest's
//! custom command) in a background task; the API polls task state and the
//! accumulated log. Install and update are the same operation: `--force`
//! reinstalls, picking up whatever is now at the source.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::manifest::Install;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug)]
pub struct InstallTask {
    pub provider_id: String,
    pub state: Mutex<TaskState>,
    pub log: Mutex<String>,
}

#[derive(Default)]
pub struct InstallTasks {
    next_id: AtomicU64,
    tasks: Mutex<HashMap<u64, Arc<InstallTask>>>,
}

/// The argv `cargo install ...` (or custom) equivalent of this install spec.
pub fn install_argv(spec: &Install) -> Result<Vec<String>> {
    let argv = match spec.kind.as_str() {
        "cargo-git" => {
            let git = spec.git.as_deref().unwrap_or_default();
            let mut v = vec![
                "cargo".to_string(),
                "install".to_string(),
                "--force".to_string(),
                "--git".to_string(),
                git.to_string(),
            ];
            if let Some(pkg) = &spec.package {
                v.push(pkg.clone());
            }
            v
        }
        "cargo" => vec![
            "cargo".to_string(),
            "install".to_string(),
            "--force".to_string(),
            spec.krate.clone().unwrap_or_default(),
        ],
        "custom" => spec.command.clone().unwrap_or_default(),
        other => bail!("unknown install kind {other:?}"),
    };
    if argv.is_empty() {
        bail!("empty install command");
    }
    Ok(argv)
}

impl InstallTasks {
    pub fn get(&self, id: u64) -> Option<Arc<InstallTask>> {
        self.tasks.lock().unwrap().get(&id).cloned()
    }

    /// True if an install for this provider is currently running.
    pub fn running_for(&self, provider_id: &str) -> bool {
        self.tasks
            .lock()
            .unwrap()
            .values()
            .any(|t| t.provider_id == provider_id && *t.state.lock().unwrap() == TaskState::Running)
    }

    /// Spawn an install/update command in the background; returns the task id
    /// to poll. `argv` is either the manifest's install command or, when the
    /// CLI supports it, its own self-update invocation.
    pub fn start(self: &Arc<Self>, provider_id: &str, argv: Vec<String>) -> Result<u64> {
        if self.running_for(provider_id) {
            bail!("an install for {provider_id} is already running");
        }
        if argv.is_empty() {
            bail!("empty install command");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let task = Arc::new(InstallTask {
            provider_id: provider_id.to_string(),
            state: Mutex::new(TaskState::Running),
            log: Mutex::new(format!("$ {}\n", argv.join(" "))),
        });
        self.tasks.lock().unwrap().insert(id, task.clone());

        tokio::spawn(async move {
            let end = match run_logged(&argv, &task).await {
                Ok(true) => TaskState::Succeeded,
                Ok(false) => TaskState::Failed,
                Err(e) => {
                    task.log.lock().unwrap().push_str(&format!("error: {e}\n"));
                    TaskState::Failed
                }
            };
            *task.state.lock().unwrap() = end;
        });
        Ok(id)
    }
}

/// Run the command, streaming stdout+stderr lines into the task log.
async fn run_logged(argv: &[String], task: &InstallTask) -> Result<bool> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut err = BufReader::new(child.stderr.take().unwrap()).lines();
    let (mut out_done, mut err_done) = (false, false);
    while !(out_done && err_done) {
        let line = tokio::select! {
            l = out.next_line(), if !out_done => l?.or_else(|| { out_done = true; None }),
            l = err.next_line(), if !err_done => l?.or_else(|| { err_done = true; None }),
        };
        if let Some(l) = line {
            let mut log = task.log.lock().unwrap();
            log.push_str(&l);
            log.push('\n');
        }
    }
    let status = child.wait().await?;
    task.log.lock().unwrap().push_str(&format!(
        "exit: {}\n",
        status.code().map_or("signal".into(), |c| c.to_string())
    ));
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_git_argv() {
        let spec = Install {
            kind: "cargo-git".into(),
            git: Some("https://example.com/repo".into()),
            package: Some("pkg-cli".into()),
            krate: None,
            command: None,
            self_update_args: None,
            update_check_args: None,
        };
        assert_eq!(
            install_argv(&spec).unwrap(),
            [
                "cargo",
                "install",
                "--force",
                "--git",
                "https://example.com/repo",
                "pkg-cli"
            ]
        );
    }
}
