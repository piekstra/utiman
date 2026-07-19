//! Bill-anomaly detection: is the latest amount unusually high for this series?
//!
//! Utility bills swing seasonally, so a single high month isn't automatically
//! alarming — but a latest bill well above the recent norm is worth surfacing
//! ("bill shock"). We compare the newest value against the *median* of the
//! prior periods (robust to one past spike) and flag it when it exceeds that
//! baseline by more than a threshold. A few periods of history are required
//! before we'll call anything anomalous.

/// A flagged anomaly on a series: the latest value, the baseline it beat, and
/// how far over it is (fraction — `0.42` means 42% above the baseline).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Anomaly {
    pub latest: f64,
    pub baseline: f64,
    pub pct_over: f64,
}

/// Prior periods required before a verdict (so a brand-new account with one or
/// two bills never trips the alarm).
const MIN_HISTORY: usize = 3;
/// Fraction over the baseline that counts as anomalous (40%).
const THRESHOLD: f64 = 0.40;

/// Detect an unusually-high latest value. `values` are newest-first, matching
/// how the series pipeline emits points. Returns `None` when there's too little
/// history, the baseline is non-positive, or nothing stands out.
pub fn detect(values: &[f64]) -> Option<Anomaly> {
    let clean: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if clean.len() < MIN_HISTORY + 1 {
        return None;
    }
    let latest = clean[0];
    let baseline = median(&clean[1..]);
    if baseline <= 0.0 {
        return None;
    }
    let pct_over = (latest - baseline) / baseline;
    (pct_over > THRESHOLD).then_some(Anomaly {
        latest,
        baseline,
        pct_over,
    })
}

/// Median of a slice (already finite). Averages the middle pair when even.
fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_a_latest_spike() {
        // Baseline median ~100; latest 180 is 80% over -> anomaly.
        let a = detect(&[180.0, 100.0, 95.0, 105.0, 100.0]).expect("anomaly");
        assert_eq!(a.latest, 180.0);
        assert_eq!(a.baseline, 100.0);
        assert!((a.pct_over - 0.80).abs() < 1e-9);
    }

    #[test]
    fn steady_bills_are_not_anomalies() {
        assert_eq!(detect(&[104.0, 100.0, 98.0, 102.0, 101.0]), None);
    }

    #[test]
    fn a_mild_rise_under_threshold_is_ignored() {
        // 130 vs median 100 = 30% over, below the 40% threshold.
        assert_eq!(detect(&[130.0, 100.0, 100.0, 100.0, 100.0]), None);
    }

    #[test]
    fn needs_enough_history() {
        // Only two prior periods — not enough to judge.
        assert_eq!(detect(&[500.0, 100.0, 100.0]), None);
    }

    #[test]
    fn non_positive_baseline_never_flags() {
        // All-credit / zero history can't yield a meaningful percentage.
        assert_eq!(detect(&[50.0, 0.0, 0.0, 0.0, 0.0]), None);
    }

    #[test]
    fn median_is_robust_to_one_past_spike() {
        // A single historical spike (900) shouldn't raise the baseline enough
        // to mask a genuine latest anomaly against the typical ~100 bills.
        let a = detect(&[170.0, 100.0, 900.0, 100.0, 95.0, 105.0]).expect("anomaly");
        assert_eq!(a.baseline, 100.0); // median of [100,900,100,95,105]
    }
}
