//! Local series archive.
//!
//! Provider CLIs return a limited window of history (a year of bills, a dozen
//! usage cycles). utiman keeps its own append-only archive so charts extend
//! past that window the longer the app runs — the series analog of the balance
//! snapshots in `snapshots.rs`.
//!
//! Each (provider, series) has a JSONL file under
//! `~/.local/share/utiman/series/<provider>__<series>.jsonl`, one
//! `{label, value}` per line. Merging is an upsert keyed by the period label:
//! a re-fetched period overwrites its stored value (estimates finalize),
//! genuinely new periods are added. Reads return the union, newest-first, so
//! the merged history drops straight into the existing chart pipeline.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::dates::parse_label;
use crate::extract::Point;

/// Serializes the archive read-modify-write. axum services `/series` fetches
/// concurrently on tokio's multi-thread runtime, so two overlapping calls for
/// the same provider+series would otherwise both read the pre-merge file and
/// the second write would clobber the first, silently dropping a period. One
/// global lock is ample for this low write volume and keeps merge trivially
/// correct; the critical section is a couple of small file operations.
fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("utiman")
        .join("series")
}

/// Sanitize an id into a safe filename segment (ids are already kebab-case,
/// but be defensive since a user manifest supplies them).
fn safe(seg: &str) -> String {
    seg.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn file_for(provider: &str, series: &str) -> PathBuf {
    dir().join(format!("{}__{}.jsonl", safe(provider), safe(series)))
}

pub fn read(provider: &str, series: &str) -> Vec<Point> {
    let Ok(text) = fs::read_to_string(file_for(provider, series)) else {
        return Vec::new();
    };
    let mut points: Vec<Point> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    sort_newest_first(&mut points);
    points
}

/// Merge freshly-fetched points into the archive (upsert by label) and return
/// the full merged history, newest-first. When nothing new is learned, the
/// file is left untouched.
pub fn merge(provider: &str, series: &str, fresh: &[Point]) -> Vec<Point> {
    // Hold the lock across the read and the write so a concurrent merge for the
    // same series can't slip in between and lose an update.
    let _guard = write_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (merged, changed) = merge_points(read(provider, series), fresh);
    if changed {
        if let Err(e) = write_series(provider, series, &merged) {
            eprintln!("utiman: could not persist series {provider}/{series}: {e}");
        }
    }
    merged
}

/// Pure upsert-by-label: returns the merged history (newest-first) and whether
/// anything changed. Split from I/O so it's unit-testable and so the file write
/// can be skipped when a re-fetch taught us nothing new.
fn merge_points(existing: Vec<Point>, fresh: &[Point]) -> (Vec<Point>, bool) {
    // label → value, existing first so a fresh value overrides.
    let mut by_label: Vec<(String, f64)> =
        existing.into_iter().map(|p| (p.label, p.value)).collect();
    let mut changed = false;
    for f in fresh {
        if let Some(slot) = by_label.iter_mut().find(|(l, _)| *l == f.label) {
            if (slot.1 - f.value).abs() > f64::EPSILON {
                slot.1 = f.value;
                changed = true;
            }
        } else {
            by_label.push((f.label.clone(), f.value));
            changed = true;
        }
    }
    let mut merged: Vec<Point> = by_label
        .into_iter()
        .map(|(label, value)| Point { label, value })
        .collect();
    sort_newest_first(&mut merged);
    (merged, changed)
}

/// Serialize the merged history to the archive file via a temp file + rename,
/// so a crash mid-write can't leave a half-written archive whose bad lines
/// `read()` would silently drop. Callers hold `write_lock`.
fn write_series(provider: &str, series: &str, merged: &[Point]) -> std::io::Result<()> {
    fs::create_dir_all(dir())?;
    let body: String = merged
        .iter()
        .filter_map(|p| serde_json::to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let final_path = file_for(provider, series);
    let mut tmp = final_path.clone().into_os_string();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    fs::write(&tmp_path, body + "\n")?;
    fs::rename(&tmp_path, &final_path)
}

/// Newest-first, matching how provider CLIs emit series. Points whose label
/// isn't a recognizable date sort to the end but keep a stable relative order.
fn sort_newest_first(points: &mut [Point]) {
    points.sort_by(|a, b| {
        let ka = parse_label(&a.label).map(|d| d.days_from_epoch());
        let kb = parse_label(&b.label).map(|d| d.days_from_epoch());
        match (ka, kb) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(label: &str, value: f64) -> Point {
        Point {
            label: label.into(),
            value,
        }
    }

    #[test]
    fn sort_is_newest_first_dates_before_unparseable() {
        let mut v = vec![
            pt("2026-04-01", 1.0),
            pt("mystery", 9.0),
            pt("2026-06-01", 3.0),
            pt("2026-05-01", 2.0),
        ];
        sort_newest_first(&mut v);
        let labels: Vec<&str> = v.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            ["2026-06-01", "2026-05-01", "2026-04-01", "mystery"]
        );
    }

    #[test]
    fn merge_updates_existing_label() {
        let existing = vec![pt("2026-05-01", 100.0), pt("2026-06-01", 110.0)];
        let (merged, changed) = merge_points(existing, &[pt("2026-06-01", 125.0)]);
        assert!(changed, "a changed value must flag a write");
        assert_eq!(merged[0], pt("2026-06-01", 125.0)); // newest-first, updated
        assert_eq!(merged[1], pt("2026-05-01", 100.0));
    }

    #[test]
    fn merge_appends_new_label() {
        let existing = vec![pt("2026-05-01", 100.0)];
        let (merged, changed) = merge_points(existing, &[pt("2026-06-01", 110.0)]);
        assert!(changed);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], pt("2026-06-01", 110.0));
    }

    #[test]
    fn merge_unchanged_is_idempotent() {
        let existing = vec![pt("2026-05-01", 100.0), pt("2026-06-01", 110.0)];
        // Re-merging identical values learns nothing → no write.
        let (merged, changed) = merge_points(existing.clone(), &existing);
        assert!(!changed, "no new data must not flag a write");
        // ...and a second pass over the result is stable.
        let (merged2, changed2) = merge_points(merged.clone(), &merged);
        assert!(!changed2);
        assert_eq!(merged, merged2);
    }
}
