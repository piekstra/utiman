//! Detect installed provider CLIs: PATH lookup plus a `--version` probe.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::runner::run_cli;

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub installed: bool,
    /// Resolved binary path, when installed.
    pub path: Option<String>,
    /// First line of `<binary> --version`, when it answers.
    pub version: Option<String>,
}

pub fn find_binary(binary: &str) -> Option<PathBuf> {
    which::which(binary).ok()
}

pub async fn detect(binary: &str) -> Detection {
    let Some(path) = find_binary(binary) else {
        return Detection {
            installed: false,
            path: None,
            version: None,
        };
    };
    let out = run_cli(&path, &["--version".into()], Duration::from_secs(5)).await;
    let version = out
        .ok()
        .then(|| out.stdout.lines().next().unwrap_or("").trim().to_string())
        .filter(|v| !v.is_empty());
    Detection {
        installed: true,
        path: Some(path.display().to_string()),
        version,
    }
}
