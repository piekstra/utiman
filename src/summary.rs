//! Run a provider's summary query and extract its balance + due date.
//!
//! Shared by the server's `/summary` endpoint and the `utiman check` command
//! so both read a card the same way.

use std::time::Duration;

use crate::detect::find_binary;
use crate::extract::extract_summary;
use crate::manifest::Manifest;
use crate::runner::run_cli;

#[derive(Debug)]
pub enum Summary {
    /// CLI is not installed.
    NotInstalled,
    /// The manifest has no summary query.
    NoQuery,
    /// The command failed (auth expired, network, etc.).
    Error { stderr: String, timed_out: bool },
    /// Parsed balance / due date (either may be absent).
    Ok {
        balance: Option<f64>,
        due_date: Option<String>,
    },
}

pub async fn summarize(manifest: &Manifest, timeout: Duration) -> Summary {
    let Some(query) = &manifest.summary else {
        return Summary::NoQuery;
    };
    let Some(bin) = find_binary(&manifest.binary) else {
        return Summary::NotInstalled;
    };
    let out = run_cli(&bin, &query.args, timeout).await;
    if !out.ok() {
        return Summary::Error {
            stderr: out.stderr,
            timed_out: out.timed_out,
        };
    }
    let fields = extract_summary(query, &out.stdout);
    Summary::Ok {
        balance: fields.balance,
        due_date: fields.due_date,
    }
}
