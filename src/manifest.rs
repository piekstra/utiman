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
    ("xfin", include_str!("../catalog/xfin.toml")),
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
    /// (e.g. saving a default account number). Display-only hint; for a
    /// value utiman can collect and apply itself, use `[[setup]]`.
    #[serde(default)]
    pub setup_command: Option<String>,
    /// Non-secret setup inputs utiman collects in a form and applies by
    /// running the CLI (e.g. saving an account number). Never for secrets —
    /// those go through `[auth]` and the terminal.
    #[serde(default)]
    pub setup: Vec<SetupInput>,
    #[serde(default)]
    pub install: Option<Install>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub summary: Option<Query>,
    /// How to pay: hand off to the provider's official payment page (utiman
    /// never touches card data).
    #[serde(default)]
    pub pay: Option<Pay>,
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
    /// Non-secret args that report auth state as JSON. Defaults to the
    /// family-standard ["auth", "status", "--json"] (piekstra-cli/1).
    #[serde(default)]
    pub status_args: Option<Vec<String>>,
    /// Dot-path into the status JSON whose truthy value means "signed in".
    /// Defaults to "authenticated" (the auth-status/v1 field).
    #[serde(default)]
    pub authenticated_field: Option<String>,
    /// Ordered human steps shown before the login command, for flows that
    /// need browser work first (e.g. capturing a session cookie).
    #[serde(default)]
    pub login_steps: Vec<String>,
}

/// A non-secret setup value utiman collects in a form and applies by running
/// `<binary> <args...> <value>` (e.g. `lrfl config set-account 1234567-0`).
/// Deliberately not for secrets — credentials go through `[auth]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SetupInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// CLI args before the value, e.g. ["config", "set-account"]. utiman
    /// appends the user's value as the final argument.
    pub args: Vec<String>,
    /// Example value shown in the empty field.
    #[serde(default)]
    pub placeholder: Option<String>,
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

/// How to pay a provider. utiman only ever opens the provider's *official*
/// payment page — it never collects or transmits card data. Set exactly one
/// of `open-args` (run the CLI to hand off to its Pay page) or `url` (open a
/// payment portal directly in the browser).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Pay {
    /// CLI args that open the official pay page, e.g. ["pay", "--open"].
    #[serde(default)]
    pub open_args: Option<Vec<String>>,
    /// A payment-portal URL to open in the browser.
    #[serde(default)]
    pub url: Option<String>,
    /// Button label (default "Pay bill").
    #[serde(default)]
    pub label: Option<String>,
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
        bail!(
            "binary must be a bare command name, not a path (got {:?})",
            m.binary
        );
    }
    if let Some(q) = &m.summary {
        if !matches!(q.format.as_str(), "json" | "text") {
            bail!(
                "summary.format must be \"json\" or \"text\" (got {:?})",
                q.format
            );
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
    for s in &m.setup {
        if s.args.is_empty() {
            bail!("setup {}: args must not be empty", s.id);
        }
    }
    if let Some(pay) = &m.pay {
        match (&pay.open_args, &pay.url) {
            (None, None) => bail!("pay: set either open-args or url"),
            (Some(_), Some(_)) => bail!("pay: set only one of open-args or url"),
            (Some(a), None) if a.is_empty() => bail!("pay.open-args must not be empty"),
            (None, Some(u)) if !u.starts_with("https://") && !u.starts_with("http://") => {
                bail!("pay.url must be an http(s) URL")
            }
            _ => {}
        }
    }
    if let Some(i) = &m.install {
        match i.kind.as_str() {
            "cargo-git" if i.git.is_none() => {
                bail!("install.kind = cargo-git requires install.git")
            }
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

    #[test]
    fn parses_setup_inputs_and_login_steps() {
        let text = r#"
id = "x"
name = "X"
binary = "x"
repo = "https://example.com"

[auth]
required = true
login-command = "pbpaste | x auth login --stdin"
login-steps = ["Sign in", "Copy the `Cookie` header"]

[[setup]]
id = "account"
name = "Account number"
args = ["config", "set-account"]
placeholder = "1234567-0"
"#;
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.setup.len(), 1);
        assert_eq!(m.setup[0].args, ["config", "set-account"]);
        assert_eq!(m.auth.unwrap().login_steps.len(), 2);
    }

    #[test]
    fn pay_requires_exactly_one_of_open_args_or_url() {
        let base = "id=\"x\"\nname=\"X\"\nbinary=\"x\"\nrepo=\"https://e.com\"\n";
        assert!(parse_manifest(&format!("{base}[pay]\nopen-args=[\"pay\",\"--open\"]")).is_ok());
        assert!(parse_manifest(&format!("{base}[pay]\nurl=\"https://e.com/pay\"")).is_ok());
        assert!(
            parse_manifest(&format!("{base}[pay]\n")).is_err(),
            "neither"
        );
        assert!(
            parse_manifest(&format!(
                "{base}[pay]\nopen-args=[\"p\"]\nurl=\"https://e.com\""
            ))
            .is_err(),
            "both"
        );
        assert!(
            parse_manifest(&format!("{base}[pay]\nurl=\"ftp://e.com\"")).is_err(),
            "non-http url"
        );
    }

    #[test]
    fn rejects_setup_without_args() {
        let bad = r#"
id = "x"
name = "X"
binary = "x"
repo = "https://example.com"

[[setup]]
id = "account"
name = "Account"
args = []
"#;
        assert!(parse_manifest(bad).is_err());
    }
}
