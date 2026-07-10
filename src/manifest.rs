//! Provider manifests: the extensibility contract.
//!
//! A provider is described entirely by a TOML manifest — which binary to run,
//! how to install it, and how to turn its output into dashboard data. utiman
//! ships a built-in catalog (embedded at compile time from `catalog/`), and
//! users extend it by dropping manifests into `~/.config/utiman/providers/`.
//! A user manifest with the same `id` as a built-in overrides it.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Built-in catalog, embedded so the binary is self-contained.
pub const BUILTIN: &[(&str, &str)] = &[
    ("fpl", include_str!("../catalog/fpl.toml")),
    ("tojfl", include_str!("../catalog/tojfl.toml")),
    ("lrfl", include_str!("../catalog/lrfl.toml")),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Manifest {
    /// Unique id (kebab-case). User manifests with a built-in id override it.
    pub id: String,
    /// Display name, e.g. "FPL Electric".
    pub name: String,
    /// electric | water | sewer | gas | internet | trash | other
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Binary name looked up on PATH (no path separators allowed).
    pub binary: String,
    /// Source repository (shown as a link in the catalog).
    pub repo: String,
    #[serde(default)]
    pub description: Option<String>,
    /// One-time, non-secret setup the user runs in their own terminal
    /// (e.g. saving a default account number).
    #[serde(default)]
    pub setup_command: Option<String>,
    #[serde(default)]
    pub install: Option<Install>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub summary: Option<Query>,
    #[serde(default)]
    pub operations: Vec<Operation>,
    /// Time-series data (usage, bill amounts, payments) rendered as charts.
    #[serde(default)]
    pub series: Vec<Series>,
    /// Files the CLI can produce (bill PDFs etc.), offered as downloads.
    #[serde(default)]
    pub documents: Vec<Document>,
}

fn default_kind() -> String {
    "other".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Install {
    /// "cargo-git" (cargo install --git), "cargo" (crates.io), or "custom".
    pub kind: String,
    #[serde(default)]
    pub git: Option<String>,
    /// Package to select within a workspace (cargo-git).
    #[serde(default)]
    pub package: Option<String>,
    /// Crate name on crates.io (kind = "cargo").
    #[serde(default, rename = "crate")]
    pub krate: Option<String>,
    /// Full argv for kind = "custom", e.g. ["brew", "install", "foo"].
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Args that make the installed CLI update itself in place
    /// (e.g. ["self-update", "--yes"]). Preferred over reinstalling
    /// when the CLI is already present.
    #[serde(default)]
    pub self_update_args: Option<Vec<String>>,
    /// Args that only report whether an update exists
    /// (e.g. ["self-update", "--check"]).
    #[serde(default)]
    pub update_check_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Auth {
    #[serde(default)]
    pub required: bool,
    /// Interactive login the user runs in their own terminal. utiman never
    /// collects or stores credentials itself.
    #[serde(default)]
    pub login_command: Option<String>,
    /// Non-secret args that report auth state as JSON
    /// (e.g. ["auth", "status", "--json"]).
    #[serde(default)]
    pub status_args: Option<Vec<String>>,
    /// Dot-path into the status JSON whose truthy value means "signed in"
    /// (e.g. "authenticated", "password_in_keychain").
    #[serde(default)]
    pub authenticated_field: Option<String>,
}

/// How to run the CLI and pull dashboard fields out of what it prints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Query {
    pub args: Vec<String>,
    /// "json": stdout is JSON, fields are dot-paths (`balance.cents`,
    /// `services.0.due_date`). "text": stdout is `Key: value` lines, fields
    /// are labels matched case-insensitively.
    #[serde(default = "default_format")]
    pub format: String,
    /// Candidate fields for the amount due, tried in order.
    #[serde(default)]
    pub balance_fields: Vec<String>,
    /// Candidate fields for the due date, tried in order.
    #[serde(default)]
    pub due_date_fields: Vec<String>,
    /// "cents" divides the extracted balance by 100.
    #[serde(default)]
    pub scale: Option<String>,
}

fn default_format() -> String {
    "json".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub name: String,
    pub args: Vec<String>,
}

/// A time series the CLI can report: one labelled number per record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Series {
    pub id: String,
    pub name: String,
    pub args: Vec<String>,
    /// "json" (default): stdout is JSON. "table": stdout contains a
    /// pipe-delimited text table with an ALL-CAPS-ish header row.
    #[serde(default = "default_format")]
    pub format: String,
    /// Dot-path to the array of records; empty string = the JSON root.
    /// Ignored for format = "table".
    #[serde(default)]
    pub items_path: String,
    /// Field holding each record's label (a period or date string).
    pub label_field: String,
    /// Candidate fields for the value, tried in order.
    pub value_fields: Vec<String>,
    /// Display unit: "usd" formats as money; anything else is shown as-is
    /// (e.g. "kWh", "gallons").
    #[serde(default)]
    pub unit: Option<String>,
    /// "cents" divides values by 100.
    #[serde(default)]
    pub scale: Option<String>,
    /// "bar" (default) or "line".
    #[serde(default = "default_chart")]
    pub chart: String,
}

fn default_chart() -> String {
    "bar".into()
}

/// A file-producing command: utiman appends `out-flag <temp path>`, runs it,
/// and streams the file back as a browser download.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub args: Vec<String>,
    /// Flag that names the output file, e.g. "--out".
    pub out_flag: String,
    /// Suggested download filename (extension sets the content type).
    pub filename: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Builtin,
    User,
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub manifest: Manifest,
    pub source: Source,
    /// Manifest file path for user providers (needed to delete them).
    pub path: Option<PathBuf>,
}

/// Directory user manifests live in: `~/.config/utiman/providers`.
/// Deliberately `~/.config` on every platform (not the OS-native config dir)
/// to match where the provider CLIs keep their own config.
pub fn user_providers_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("utiman")
        .join("providers")
}

pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let m: Manifest = toml::from_str(text).context("manifest is not valid TOML")?;
    validate(&m)?;
    Ok(m)
}

fn validate(m: &Manifest) -> Result<()> {
    if m.id.is_empty()
        || !m
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("id must be non-empty kebab-case (got {:?})", m.id);
    }
    if m.binary.is_empty() || m.binary.contains(['/', '\\']) {
        bail!("binary must be a bare command name, not a path (got {:?})", m.binary);
    }
    if let Some(q) = &m.summary {
        if !matches!(q.format.as_str(), "json" | "text") {
            bail!("summary.format must be \"json\" or \"text\" (got {:?})", q.format);
        }
    }
    for s in &m.series {
        if !matches!(s.format.as_str(), "json" | "table") {
            bail!("series {}: format must be \"json\" or \"table\"", s.id);
        }
        if !matches!(s.chart.as_str(), "bar" | "line") {
            bail!("series {}: chart must be \"bar\" or \"line\"", s.id);
        }
        if s.value_fields.is_empty() {
            bail!("series {}: value-fields must not be empty", s.id);
        }
    }
    for d in &m.documents {
        if d.filename.contains(['/', '\\']) || d.filename.starts_with('.') {
            bail!("document {}: filename must be a plain name", d.id);
        }
    }
    if let Some(i) = &m.install {
        match i.kind.as_str() {
            "cargo-git" if i.git.is_none() => bail!("install.kind = cargo-git requires install.git"),
            "cargo" if i.krate.is_none() => bail!("install.kind = cargo requires install.krate"),
            "custom" if i.command.as_ref().is_none_or(|c| c.is_empty()) => {
                bail!("install.kind = custom requires a non-empty install.command")
            }
            "cargo-git" | "cargo" | "custom" => {}
            other => bail!("unknown install.kind {other:?}"),
        }
    }
    Ok(())
}

/// Built-ins merged with user manifests; user ids override built-in ids.
/// Unparseable user manifests are skipped (reported on stderr) so one bad
/// file can't take the dashboard down.
pub fn load_providers() -> Vec<Provider> {
    let mut out: Vec<Provider> = Vec::new();
    for (name, text) in BUILTIN {
        match parse_manifest(text) {
            Ok(manifest) => out.push(Provider {
                manifest,
                source: Source::Builtin,
                path: None,
            }),
            Err(e) => eprintln!("utiman: built-in manifest {name} is invalid: {e}"),
        }
    }

    let dir = user_providers_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("utiman: cannot read {}: {e}", path.display());
                continue;
            }
        };
        match parse_manifest(&text) {
            Ok(manifest) => {
                out.retain(|p| p.manifest.id != manifest.id);
                out.push(Provider {
                    manifest,
                    source: Source::User,
                    path: Some(path),
                });
            }
            Err(e) => eprintln!("utiman: skipping {}: {e}", path.display()),
        }
    }
    out.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_parses() {
        for (name, text) in BUILTIN {
            let m = parse_manifest(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&m.id, name);
            assert!(m.summary.is_some(), "{name} should have a summary query");
        }
    }

    #[test]
    fn rejects_binary_paths() {
        let bad = r#"
id = "x"
name = "X"
binary = "/usr/bin/evil"
repo = "https://example.com"
"#;
        assert!(parse_manifest(bad).is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let bad = r#"
id = "x"
name = "X"
binary = "x"
repo = "https://example.com"
surprise = true
"#;
        assert!(parse_manifest(bad).is_err());
    }
}
