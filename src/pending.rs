//! Payment-pending markers.
//!
//! When you hit Pay, the money leaves but the portal balance often doesn't
//! update for hours or days. utiman records a marker (when, and the balance at
//! the time) so the card can show a "payment pending" state in that gap. The
//! marker clears itself once the balance actually drops, or after a window.
//!
//! One JSON file per provider under `~/.local/share/utiman/pending/<id>.json`.
//! Non-secret (just a timestamp and a dollar amount), same as snapshots.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long a marker lingers before it's assumed stale (payment failed, or the
/// portal simply never reflected it) and stops showing as pending.
const PENDING_WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    /// Unix seconds when the user initiated payment.
    pub initiated_at: u64,
    /// Balance shown at the moment of payment; the marker clears once the
    /// live balance drops below this (the payment posted).
    pub balance_at_init: f64,
}

fn dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("utiman")
        .join("pending")
}

fn file_for(id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir().join(format!("{safe}.json"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record that a payment was just initiated at `balance`.
pub fn record(id: &str, balance: f64) {
    let p = Pending {
        initiated_at: now(),
        balance_at_init: balance,
    };
    if fs::create_dir_all(dir()).is_ok() {
        if let Ok(text) = serde_json::to_string(&p) {
            let _ = fs::write(file_for(id), text);
        }
    }
}

pub fn read(id: &str) -> Option<Pending> {
    let text = fs::read_to_string(file_for(id)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn clear(id: &str) {
    let _ = fs::remove_file(file_for(id));
}

/// Whether a marker still counts as pending, given the live balance and time.
/// Pure so it's unit-testable; `is_pending` wraps it with file I/O.
fn still_pending(p: &Pending, current_balance: Option<f64>, now: u64) -> bool {
    // Balance dropped by at least a cent → the payment posted.
    if let Some(bal) = current_balance {
        if bal <= p.balance_at_init - 0.01 {
            return false;
        }
    }
    // Too old → stop showing it (payment failed or never reflected).
    now.saturating_sub(p.initiated_at) <= PENDING_WINDOW_SECS
}

/// Given the current balance, decide whether a payment is still pending.
/// Clears the marker (and returns false) once the balance has dropped — the
/// payment posted — or the window elapsed.
pub fn is_pending(id: &str, current_balance: Option<f64>) -> bool {
    let Some(p) = read(id) else {
        return false;
    };
    if still_pending(&p, current_balance, now()) {
        true
    } else {
        clear(id);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_roundtrips() {
        let p = Pending {
            initiated_at: 1_700_000_000,
            balance_at_init: 84.21,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: Pending = serde_json::from_str(&s).unwrap();
        assert_eq!(back.balance_at_init, 84.21);
        assert_eq!(back.initiated_at, 1_700_000_000);
    }

    #[test]
    fn still_pending_logic() {
        let t = 1_700_000_000;
        let p = Pending {
            initiated_at: t,
            balance_at_init: 84.21,
        };
        // Same balance, just made → pending.
        assert!(still_pending(&p, Some(84.21), t));
        // Balance clearly dropped → posted, not pending.
        assert!(!still_pending(&p, Some(84.00), t));
        assert!(!still_pending(&p, Some(0.0), t));
        // Balance unknown but recent → still pending.
        assert!(still_pending(&p, None, t + 3600));
        // Past the window → stop showing.
        assert!(!still_pending(&p, Some(84.21), t + PENDING_WINDOW_SECS + 1));
    }
}
