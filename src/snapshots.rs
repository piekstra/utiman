//! Local balance history.
//!
//! The provider portals mostly report only the current balance, so utiman
//! records its own trail: every successful summary appends a snapshot to
//! `~/.local/share/utiman/history/<id>.jsonl`. Appends are skipped while the
//! balance is unchanged and the last snapshot is recent, so refresh-spamming
//! doesn't bloat the file. This gives every provider a balance-over-time
//! chart even when its CLI has no history command.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    /// Unix seconds.
    pub ts: u64,
    pub balance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
}

/// Re-record an unchanged balance only after this long (keeps a heartbeat
/// so charts show flat periods without one row per refresh).
const UNCHANGED_REFRESH_SECS: u64 = 23 * 60 * 60;

pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("utiman")
        .join("history")
}

fn file_for(id: &str) -> PathBuf {
    data_dir().join(format!("{id}.jsonl"))
}

pub fn read(id: &str) -> Vec<Snapshot> {
    let Ok(text) = fs::read_to_string(file_for(id)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append a snapshot unless it duplicates a recent, unchanged balance.
pub fn record(id: &str, balance: f64, due_date: Option<&str>) {
    let history = read(id);
    if let Some(last) = history.last() {
        let unchanged = (last.balance - balance).abs() < 0.005
            && last.due_date.as_deref() == due_date;
        if unchanged && now().saturating_sub(last.ts) < UNCHANGED_REFRESH_SECS {
            return;
        }
    }
    let snap = Snapshot {
        ts: now(),
        balance,
        due_date: due_date.map(String::from),
    };
    let dir = data_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(line) = serde_json::to_string(&snap) else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().append(true).create(true).open(file_for(id)) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrip() {
        let s = Snapshot { ts: 1_700_000_000, balance: 84.21, due_date: Some("07/18/2026".into()) };
        let line = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Snapshot>(&line).unwrap(), s);
    }
}
