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

use crate::dates::parse_label;
use crate::extract::Point;

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
    // Upsert: label → value, existing first so a fresh value overrides.
    let mut by_label: Vec<(String, f64)> = read(provider, series)
        .into_iter()
        .map(|p| (p.label, p.value))
        .collect();
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

    if changed && fs::create_dir_all(dir()).is_ok() {
        let body: String = merged
            .iter()
            .filter_map(|p| serde_json::to_string(p).ok())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::write(file_for(provider, series), body + "\n");
    }
    merged
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
}
