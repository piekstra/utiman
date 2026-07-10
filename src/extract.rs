//! Pull dashboard fields (balance, due date) out of CLI output.
//!
//! Two formats, matching how the provider CLIs print:
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

/// Parse "$1,234.56", "84.21", "-12", "($5.00)" into a float.
pub fn parse_money(s: &str) -> Option<f64> {
    let t = s.trim();
    let (t, negative) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (inner, true),
        None => (t, false),
    };
    let cleaned: String = t.chars().filter(|c| !"$,".contains(*c) && !c.is_whitespace()).collect();
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

    #[test]
    fn money_parsing() {
        assert_eq!(parse_money("$1,234.56"), Some(1234.56));
        assert_eq!(parse_money("84.21"), Some(84.21));
        assert_eq!(parse_money("($5.00)"), Some(-5.0));
        assert_eq!(parse_money("-12"), Some(-12.0));
        assert_eq!(parse_money("n/a"), None);
    }
}
