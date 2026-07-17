//! `utiman check` — a headless due-date report.
//!
//! Runs every installed provider's summary, prints what's due (and how soon),
//! and can raise a macOS notification for anything due within a window. Built
//! for cron: a stable exit code (2 = something due soon/overdue) lets a
//! wrapper act on it.

use std::time::Duration;

use anyhow::Result;

use crate::dates::parse_due;
use crate::manifest::load_providers;
use crate::runner::DEFAULT_TIMEOUT_SECS;
use crate::summary::{summarize, Summary};

struct Due {
    name: String,
    balance: Option<f64>,
    due_raw: String,
    days: Option<i64>,
}

struct Failed {
    name: String,
    reason: String,
}

/// Run the report. `within` is the notify/urgency window in days; `notify`
/// raises a macOS notification for due-soon items. Returns the process exit
/// code (2 when something is due within the window or overdue).
pub async fn run(within: i64, notify: bool, json: bool) -> Result<i32> {
    let providers = load_providers();
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);

    let mut dues = Vec::new();
    let mut errors = Vec::new();
    for p in providers {
        if p.manifest.summary.is_none() {
            continue;
        }
        match summarize(&p.manifest, timeout).await {
            Summary::Ok { balance, due_date } => {
                // Only report a positive balance; a $0 / credit line isn't "due".
                let owes = balance.map(|b| b > 0.005).unwrap_or(false);
                if let Some(raw) = due_date.filter(|_| owes) {
                    let days = parse_due(&raw).map(|d| d.days_from_today());
                    dues.push(Due {
                        name: p.manifest.name,
                        balance,
                        due_raw: raw,
                        days,
                    });
                }
            }
            Summary::Error { stderr, timed_out } => {
                let reason = if timed_out {
                    "timed out".to_string()
                } else {
                    stderr
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .map(|l| l.trim().trim_start_matches("error:").trim().to_string())
                        .unwrap_or_else(|| "couldn't fetch".to_string())
                };
                errors.push(Failed {
                    name: p.manifest.name,
                    reason,
                });
            }
            Summary::NotInstalled | Summary::NoQuery => {}
        }
    }

    triage(&mut dues); // soonest-first, undated last
    let soon: Vec<&Due> = dues.iter().filter(|d| is_soon(d, within)).collect();

    if json {
        print_json(&dues, &errors, within);
    } else {
        print_text(&dues, &errors, within);
    }

    if notify && !soon.is_empty() {
        raise_notification(&soon);
    }

    Ok(exit_code(soon.is_empty()))
}

/// Order the report soonest-first; undated items sort last.
fn triage(dues: &mut [Due]) {
    dues.sort_by_key(|d| d.days.unwrap_or(i64::MAX));
}

/// A bill is "due soon" when it has a date within the window (or overdue).
/// Undated bills are never "soon" — we can't say when they're due.
fn is_soon(d: &Due, within: i64) -> bool {
    d.days.map(|n| n <= within).unwrap_or(false)
}

/// Process exit code: 2 when anything is due soon/overdue (so cron can act),
/// else 0.
fn exit_code(none_soon: bool) -> i32 {
    if none_soon {
        0
    } else {
        2
    }
}

fn money(b: Option<f64>) -> String {
    b.map(|v| format!("${v:.2}")).unwrap_or_else(|| "—".into())
}

fn when(days: Option<i64>) -> String {
    match days {
        Some(n) if n < 0 => format!("overdue {}d", -n),
        Some(0) => "due today".into(),
        Some(n) => format!("in {n}d"),
        None => "date unknown".into(),
    }
}

fn print_text(dues: &[Due], errors: &[Failed], within: i64) {
    if dues.is_empty() && errors.is_empty() {
        println!("Nothing due — all accounts clear.");
        return;
    }
    for d in dues {
        let flag = match d.days {
            Some(n) if n < 0 => "! ",
            Some(n) if n <= within => "* ",
            _ => "  ",
        };
        println!(
            "{flag}{:<26} {:>10}   due {} ({})",
            d.name,
            money(d.balance),
            d.due_raw,
            when(d.days)
        );
    }
    for e in errors {
        println!("  {:<26} {:>10}   {}", e.name, "?", e.reason);
    }
}

fn print_json(dues: &[Due], errors: &[Failed], within: i64) {
    let items: Vec<serde_json::Value> = dues
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name,
                "balance": d.balance,
                "due": d.due_raw,
                "days": d.days,
                "soon": d.days.map(|n| n <= within).unwrap_or(false),
            })
        })
        .collect();
    let errs: Vec<serde_json::Value> = errors
        .iter()
        .map(|e| serde_json::json!({ "name": e.name, "reason": e.reason }))
        .collect();
    let out = serde_json::json!({ "due": items, "errors": errs, "within_days": within });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

/// Post a macOS notification summarizing due-soon items. Best-effort.
fn raise_notification(soon: &[&Due]) {
    if std::env::consts::OS != "macos" {
        return;
    }
    let total: f64 = soon.iter().filter_map(|d| d.balance).sum();
    let lead = soon
        .iter()
        .map(|d| format!("{} {}", d.name, when(d.days)))
        .collect::<Vec<_>>()
        .join(", ");
    let title = format!("utiman: {} bill(s) due soon", soon.len());
    let body = format!("${total:.2} total — {lead}");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        applescript_escape(&body),
        applescript_escape(&title),
    );
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output();
}

fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_phrasing() {
        assert_eq!(when(Some(-3)), "overdue 3d");
        assert_eq!(when(Some(0)), "due today");
        assert_eq!(when(Some(5)), "in 5d");
        assert_eq!(when(None), "date unknown");
    }

    #[test]
    fn money_formatting() {
        assert_eq!(money(Some(84.2)), "$84.20");
        assert_eq!(money(None), "—");
    }

    #[test]
    fn applescript_escaping() {
        assert_eq!(applescript_escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    fn due(name: &str, days: Option<i64>) -> Due {
        Due {
            name: name.into(),
            balance: Some(10.0),
            due_raw: "x".into(),
            days,
        }
    }

    #[test]
    fn triage_sorts_soonest_first_undated_last() {
        let mut v = vec![
            due("later", Some(20)),
            due("undated", None),
            due("overdue", Some(-3)),
            due("soon", Some(2)),
        ];
        triage(&mut v);
        let order: Vec<&str> = v.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, ["overdue", "soon", "later", "undated"]);
    }

    #[test]
    fn is_soon_window_boundary_and_overdue() {
        assert!(is_soon(&due("", Some(-1)), 5), "overdue is soon");
        assert!(is_soon(&due("", Some(0)), 5), "due today is soon");
        assert!(
            is_soon(&due("", Some(5)), 5),
            "exactly at the window is soon"
        );
        assert!(
            !is_soon(&due("", Some(6)), 5),
            "past the window is not soon"
        );
        assert!(!is_soon(&due("", None), 5), "undated is never soon");
    }

    #[test]
    fn exit_code_maps_soonness() {
        assert_eq!(exit_code(true), 0, "nothing soon → 0");
        assert_eq!(exit_code(false), 2, "something soon → 2 (cron acts)");
    }
}
