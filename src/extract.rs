//! Pull dashboard fields (balance, due date) out of CLI output.
//!
//! The **utility/v1 profile fast path** comes first: a CLI emitting the
//! canonical `utility-summary/v1` DTO or a `<record>-list/v1` `Paged`
//! envelope (cli-common ≥ v0.2.0) needs no field configuration at all — the
//! schema tag is authoritative and the manifest's field lists are ignored.
//!
//! Otherwise, two manifest-driven formats, matching how the provider CLIs
//! print:
//! - `json`: fields are dot-paths into the stdout JSON (`balance.cents`,
//!   `services.0.due_date`). Numeric segments index arrays.
//! - `text`: fields are labels matched case-insensitively against
//!   `Key: value` lines — the text-block style the provider CLIs render.
//!
//! Each field list is a series of fallbacks tried in order, because some
//! upstream payloads (notably FPL's) vary by account type.

use serde_json::Value;

use crate::manifest::Query;

#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct SummaryFields {
    /// Amount due, normalized to dollars.
    pub balance: Option<f64>,
    pub due_date: Option<String>,
}

pub fn extract_summary(query: &Query, stdout: &str) -> SummaryFields {
    if let Some(fields) = profile_summary(stdout) {
        return fields;
    }
    let (balance_raw, due_date) = match query.format.as_str() {
        "text" => (
            first_text_field(stdout, &query.balance_fields),
            first_text_field(stdout, &query.due_date_fields),
        ),
        _ => match serde_json::from_str::<Value>(stdout) {
            Ok(v) => (
                first_json_field(&v, &query.balance_fields).map(json_scalar),
                first_json_field(&v, &query.due_date_fields).map(json_scalar),
            ),
            Err(_) => (None, None),
        },
    };

    let mut balance = balance_raw.as_deref().and_then(parse_money);
    if query.scale.as_deref() == Some("cents") {
        balance = balance.map(|b| b / 100.0);
    }
    SummaryFields { balance, due_date }
}

/// utility/v1 fast path: a `utility-summary/v1` payload carries `balance` as
/// a `Money` object (string-decimal `amount`) and an ISO `due_date` — no
/// per-provider field configuration needed, whatever the manifest says.
fn profile_summary(stdout: &str) -> Option<SummaryFields> {
    let v: Value = serde_json::from_str(stdout).ok()?;
    if v.get("schema")?.as_str()? != "utility-summary/v1" {
        return None;
    }
    let balance = v
        .get("balance")
        .and_then(|m| m.get("amount"))
        .and_then(Value::as_str)
        .and_then(parse_money);
    let due_date = v
        .get("due_date")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(SummaryFields { balance, due_date })
}

/// utility/v1 fast path for lists: a `<record>-list/v1` `Paged` envelope
/// keeps its records under `items`.
fn profile_items(root: &Value) -> Option<&Value> {
    let schema = root.get("schema")?.as_str()?;
    if !schema.ends_with("-list/v1") {
        return None;
    }
    root.get("items").filter(|v| v.is_array())
}

/// Walk a dot-path (`a.b.0.c`) into a JSON value; numeric segments index arrays.
pub fn json_path<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn first_json_field<'v>(root: &'v Value, paths: &[String]) -> Option<&'v Value> {
    paths
        .iter()
        .filter_map(|p| json_path(root, p))
        .find(|v| !v.is_null())
}

fn json_scalar(v: &Value) -> String {
    // A profile `Money` object scalarizes to its decimal amount, so
    // `value-fields = ["amount"]` works on utility/v1 records.
    if let Some(map) = v.as_object() {
        if map.len() == 2 && map.contains_key("currency") {
            if let Some(amount) = map.get("amount").and_then(Value::as_str) {
                return amount.to_string();
            }
        }
    }
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Find the first `Label: value` line whose label matches (case-insensitive).
fn first_text_field(stdout: &str, labels: &[String]) -> Option<String> {
    for label in labels {
        for line in stdout.lines() {
            if let Some((key, val)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case(label) && !val.trim().is_empty() {
                    return Some(val.trim().to_string());
                }
            }
        }
    }
    None
}

/// One extracted series point.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub label: String,
    pub value: f64,
}

/// Extract labelled points from a series command's output.
pub fn extract_series(series: &crate::manifest::Series, stdout: &str) -> Vec<Point> {
    let mut profile_envelope = false;
    let records: Vec<serde_json::Map<String, Value>> = match series.format.as_str() {
        "table" => parse_pipe_table(stdout),
        _ => {
            let Ok(root) = serde_json::from_str::<Value>(stdout) else {
                return Vec::new();
            };
            let items = if let Some(items) = profile_items(&root) {
                profile_envelope = true;
                Some(items)
            } else if series.items_path.is_empty() {
                Some(&root)
            } else {
                json_path(&root, &series.items_path)
            };
            match items {
                Some(Value::Array(arr)) => {
                    arr.iter().filter_map(|v| v.as_object().cloned()).collect()
                }
                _ => Vec::new(),
            }
        }
    };

    // Profile semantics are canonical: Money is decimal dollars and
    // quantities are plain numbers, so a stale manifest `scale = "cents"`
    // must not divide values that came from a profile envelope.
    let scale = if !profile_envelope && series.scale.as_deref() == Some("cents") {
        100.0
    } else {
        1.0
    };
    records
        .iter()
        .filter_map(|rec| {
            let obj = Value::Object(rec.clone());
            let label = record_field(&obj, &series.label_field)?;
            let value = series
                .value_fields
                .iter()
                .find_map(|f| record_field(&obj, f).as_deref().and_then(parse_money))?;
            Some(Point {
                label,
                value: value / scale,
            })
        })
        .collect()
}

/// Look a field up inside one record: exact dot-path first, then a
/// case-insensitive top-level match (so table headers like "AMOUNT" or
/// "Due date" find manifest fields written naturally).
fn record_field(rec: &Value, field: &str) -> Option<String> {
    if let Some(v) = json_path(rec, field) {
        if !v.is_null() {
            return Some(json_scalar(v));
        }
    }
    let obj = rec.as_object()?;
    obj.iter()
        .find(|(k, v)| k.eq_ignore_ascii_case(field) && !v.is_null())
        .map(|(_, v)| json_scalar(v))
}

/// Parse the pipe-delimited text tables the provider CLIs render:
/// a `HEADER | HEADER` line followed by `value | value` rows. Lines before
/// the header (titles, key/value preamble) are skipped.
pub fn parse_pipe_table(stdout: &str) -> Vec<serde_json::Map<String, Value>> {
    let mut lines = stdout.lines().filter(|l| l.contains('|'));
    let Some(header_line) = lines.next() else {
        return Vec::new();
    };
    let headers: Vec<String> = header_line
        .split('|')
        .map(|h| h.trim().to_string())
        .collect();
    lines
        .map(|line| {
            let cells = line.split('|').map(str::trim);
            headers
                .iter()
                .zip(cells)
                .map(|(h, c)| (h.clone(), Value::String(c.to_string())))
                .collect()
        })
        .collect()
}

/// Parse "$1,234.56", "84.21", "-12", "($5.00)" into a float.
pub fn parse_money(s: &str) -> Option<f64> {
    let t = s.trim();
    let (t, negative) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (inner, true),
        None => (t, false),
    };
    let cleaned: String = t
        .chars()
        .filter(|c| !"$,".contains(*c) && !c.is_whitespace())
        .collect();
    let v: f64 = cleaned.parse().ok()?;
    Some(if negative { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn query(format: &str, balance: &[&str], due: &[&str], scale: Option<&str>) -> Query {
        Query {
            args: vec![],
            format: format.into(),
            balance_fields: balance.iter().map(|s| s.to_string()).collect(),
            due_date_fields: due.iter().map(|s| s.to_string()).collect(),
            scale: scale.map(String::from),
        }
    }

    #[test]
    fn json_paths_with_array_index() {
        let v = json!({"services": [{"due_date": "08/01/2026"}]});
        assert_eq!(
            json_path(&v, "services.0.due_date"),
            Some(&json!("08/01/2026"))
        );
        assert_eq!(json_path(&v, "services.1.due_date"), None);
    }

    #[test]
    fn lrfl_shape() {
        let q = query("json", &["balance_due"], &["services.0.due_date"], None);
        let out = r#"{"account":"1234567-0","balance_due":61.75,
                      "services":[{"service":"SEWER","amount_due":61.75,"due_date":"08/01/2026"}]}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(61.75));
        assert_eq!(s.due_date.as_deref(), Some("08/01/2026"));
    }

    #[test]
    fn tojfl_shape_cents() {
        let q = query("json", &["balance.cents"], &["due_date"], Some("cents"));
        let out = r#"{"balance":{"cents":8421},"due_date":"07/18/2026","name":null}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(84.21));
        assert_eq!(s.due_date.as_deref(), Some("07/18/2026"));
    }

    #[test]
    fn fpl_text_shape() {
        let q = query("text", &["Balance"], &["Due date"], None);
        let out = "Balance: $142.10\nDue date: 07/22/2026\nPast due: $0.00\n";
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(142.10));
        assert_eq!(s.due_date.as_deref(), Some("07/22/2026"));
    }

    #[test]
    fn fallback_paths_in_order() {
        let q = query("json", &["data.amount", "data.balance"], &[], None);
        let s = extract_summary(&q, r#"{"data":{"balance":"$50.00"}}"#);
        assert_eq!(s.balance, Some(50.0));
    }

    fn series(
        format: &str,
        items_path: &str,
        label: &str,
        values: &[&str],
        scale: Option<&str>,
    ) -> crate::manifest::Series {
        crate::manifest::Series {
            id: "s".into(),
            name: "S".into(),
            args: vec![],
            format: format.into(),
            items_path: items_path.into(),
            label_field: label.into(),
            value_fields: values.iter().map(|s| s.to_string()).collect(),
            unit: None,
            scale: scale.map(String::from),
            chart: "bar".into(),
        }
    }

    #[test]
    fn tojfl_bills_series() {
        let s = series("json", "", "date", &["amount.cents"], Some("cents"));
        let out = r#"[{"date":"06/01/2026","amount":{"cents":8421},"due_date":"06/18/2026"},
                      {"date":"05/01/2026","amount":{"cents":7900}}]"#;
        let pts = extract_series(&s, out);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].label, "06/01/2026");
        assert_eq!(pts[0].value, 84.21);
    }

    #[test]
    fn lrfl_payments_series() {
        let s = series("json", "payments", "payment_date", &["amount"], None);
        let out = r#"{"account":"1234567-0","payments":[
            {"transaction_id":"T1","amount":61.75,"payment_date":"05/28/2026"}]}"#;
        let pts = extract_series(&s, out);
        assert_eq!(
            pts,
            vec![Point {
                label: "05/28/2026".into(),
                value: 61.75
            }]
        );
    }

    #[test]
    fn pipe_table_series_with_case_insensitive_headers() {
        let s = series("table", "", "Month", &["Amount"], None);
        let out = "Bill history\n\nMONTH | AMOUNT | KWH\nJun 2026 | $142.10 | 1120\nMay 2026 | $128.33 | 1044\n";
        let pts = extract_series(&s, out);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1].label, "May 2026");
        assert_eq!(pts[1].value, 128.33);
    }

    #[test]
    fn missing_values_are_skipped() {
        let s = series("json", "", "date", &["amount.cents"], Some("cents"));
        let out = r#"[{"date":"06/01/2026"},{"date":"05/01/2026","amount":{"cents":100}}]"#;
        let pts = extract_series(&s, out);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].value, 1.0);
    }

    #[test]
    fn profile_summary_needs_no_field_config() {
        // Manifest has no field lists at all — the schema tag drives it.
        let q = query("json", &[], &[], None);
        let out = r#"{"schema":"utility-summary/v1",
                      "balance":{"amount":"84.21","currency":"USD"},
                      "due_date":"2026-07-18","account":"12345-0"}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(84.21));
        assert_eq!(s.due_date.as_deref(), Some("2026-07-18"));
    }

    #[test]
    fn profile_summary_overrides_manifest_fields() {
        // Stale manifest config (cents scaling, wrong paths) is ignored once
        // the CLI emits the canonical DTO.
        let q = query(
            "json",
            &["balance.cents"],
            &["services.0.due_date"],
            Some("cents"),
        );
        let out = r#"{"schema":"utility-summary/v1",
                      "balance":{"amount":"84.21","currency":"USD"},
                      "due_date":"2026-07-18"}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(84.21));
        assert_eq!(s.due_date.as_deref(), Some("2026-07-18"));
    }

    #[test]
    fn profile_summary_without_due_date() {
        let q = query("json", &[], &[], None);
        let out = r#"{"schema":"utility-summary/v1",
                      "balance":{"amount":"0.00","currency":"USD"}}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(0.0));
        assert_eq!(s.due_date, None);
    }

    #[test]
    fn other_schemas_fall_through_to_manifest_fields() {
        let q = query("json", &["balance.cents"], &["due_date"], Some("cents"));
        let out =
            r#"{"schema":"tojfl-summary/v1","balance":{"cents":8421},"due_date":"07/18/2026"}"#;
        let s = extract_summary(&q, out);
        assert_eq!(s.balance, Some(84.21));
    }

    #[test]
    fn paged_envelope_items_fast_path() {
        // items-path unset; the -list/v1 envelope finds records anyway, and
        // Money objects scalarize so value-fields = ["amount"] works.
        let s = series("json", "", "date", &["amount"], None);
        let out = r#"{"schema":"statement-list/v1","items":[
            {"id":"2026-06","date":"2026-06-15","amount":{"amount":"84.21","currency":"USD"}},
            {"id":"2026-05","date":"2026-05-15","amount":{"amount":"79.00","currency":"USD"}}]}"#;
        let pts = extract_series(&s, out);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].label, "2026-06-15");
        assert_eq!(pts[0].value, 84.21);
        assert_eq!(pts[1].value, 79.00);
    }

    #[test]
    fn paged_envelope_ignores_stale_cents_scale() {
        // A manifest written for the pre-profile CLI (`amount.cents` +
        // scale="cents") must still chart correctly against profile output:
        // the fallback chain reaches `amount`, and the envelope suppresses
        // the cents division.
        let s = series(
            "json",
            "",
            "date",
            &["amount.cents", "amount"],
            Some("cents"),
        );
        let out = r#"{"schema":"statement-list/v1","items":[
            {"id":"1","date":"2026-06-15","amount":{"amount":"84.21","currency":"USD"}}]}"#;
        let pts = extract_series(&s, out);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].value, 84.21);

        // …while the same manifest against pre-profile output still scales.
        let old = r#"[{"date":"06/15/2026","amount":{"cents":8421}}]"#;
        let pts = extract_series(&s, old);
        assert_eq!(pts[0].value, 84.21);
    }

    #[test]
    fn money_parsing() {
        assert_eq!(parse_money("$1,234.56"), Some(1234.56));
        assert_eq!(parse_money("84.21"), Some(84.21));
        assert_eq!(parse_money("($5.00)"), Some(-5.0));
        assert_eq!(parse_money("-12"), Some(-12.0));
        assert_eq!(parse_money("n/a"), None);
    }
}
