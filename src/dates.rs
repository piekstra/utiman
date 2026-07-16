//! Minimal, dependency-free date handling for due-date math.
//!
//! Provider CLIs emit due dates in a couple of shapes — ISO `2026-07-17` and
//! US `8/5/2026` — sometimes wrapped in free text. We only need "how many days
//! from today", so this parses those two shapes and counts days via the civil
//! calendar (Howard Hinnant's algorithm), with no external crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// A calendar date. `days_from_epoch` gives a total ordering and day counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Days since 1970-01-01 (negative before). Proleptic Gregorian.
    pub fn days_from_epoch(self) -> i64 {
        let (y, m, d) = (self.year, self.month as i64, self.day as i64);
        // Shift so March is month 0 (leap day lands at year's end).
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    /// Whole days from today (negative = already past).
    pub fn days_from_today(self) -> i64 {
        self.days_from_epoch() - today().days_from_epoch()
    }
}

/// Today's local-ish date, derived from the system clock. Uses the machine's
/// UTC day; good enough for "due in N days" at day granularity.
pub fn today() -> Date {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    from_days_since_epoch(secs.div_euclid(86_400))
}

/// Inverse of `days_from_epoch` — civil date from a day count.
fn from_days_since_epoch(z: i64) -> Date {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { y + 1 } else { y };
    Date { year, month, day }
}

/// Parse a due date out of CLI output. Accepts a bare `YYYY-MM-DD` or
/// `M/D/YYYY`, or those embedded in free text (e.g. a portal string like
/// "… (Saturday, July 11, 2026)" is not matched — only numeric forms are).
pub fn parse_due(s: &str) -> Option<Date> {
    // ISO: YYYY-MM-DD
    if let Some(d) = first_match(s, &['-'], |a, b, c| Date {
        year: a,
        month: b as u32,
        day: c as u32,
    }) {
        return Some(d);
    }
    // US: M/D/YYYY
    first_match(s, &['/'], |a, b, c| Date {
        year: c,
        month: a as u32,
        day: b as u32,
    })
}

/// Scan `s` for the first `N<sep>N<sep>N` triple and build a Date from it.
/// `build(first, second, third)` assigns the fields per format.
fn first_match(s: &str, seps: &[char], build: impl Fn(i64, i64, i64) -> Date) -> Option<Date> {
    let sep = seps[0];
    for token in s.split(|c: char| c != sep && !c.is_ascii_digit()) {
        let parts: Vec<&str> = token.split(sep).collect();
        if parts.len() == 3 {
            if let (Ok(a), Ok(b), Ok(c)) = (
                parts[0].parse::<i64>(),
                parts[1].parse::<i64>(),
                parts[2].parse::<i64>(),
            ) {
                let d = build(a, b, c);
                if (1..=12).contains(&d.month) && (1..=31).contains(&d.day) && d.year >= 1970 {
                    return Some(d);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_anchor() {
        assert_eq!(
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
            .days_from_epoch(),
            0
        );
        assert_eq!(
            Date {
                year: 2000,
                month: 1,
                day: 1
            }
            .days_from_epoch(),
            10957
        );
    }

    #[test]
    fn roundtrip_from_days() {
        for z in [0, 10957, 20000, -1, 19000] {
            assert_eq!(from_days_since_epoch(z).days_from_epoch(), z);
        }
    }

    #[test]
    fn parse_iso_and_us() {
        assert_eq!(
            parse_due("2026-07-17"),
            Some(Date {
                year: 2026,
                month: 7,
                day: 17
            })
        );
        assert_eq!(
            parse_due("8/5/2026"),
            Some(Date {
                year: 2026,
                month: 8,
                day: 5
            })
        );
        assert_eq!(
            parse_due("Due 07/18/2026 now"),
            Some(Date {
                year: 2026,
                month: 7,
                day: 18
            })
        );
        assert_eq!(parse_due("paid in full"), None);
        assert_eq!(parse_due("13/40/2026"), None); // out of range
    }

    #[test]
    fn day_delta_sign() {
        let t = today();
        assert_eq!(t.days_from_today(), 0);
        let tomorrow = from_days_since_epoch(t.days_from_epoch() + 1);
        assert_eq!(tomorrow.days_from_today(), 1);
    }
}
