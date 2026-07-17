//! utiman — a local dashboard over your utility-provider CLIs.
//!
//! `utiman` starts a localhost web server, opens the dashboard, and shells
//! out to provider CLIs (fpl, tojfl, lrfl, ...) behind the scenes. It holds
//! no credentials: each CLI manages its own (typically in the OS keychain),
//! and anything interactive (logins) happens in your terminal, not here.

mod check;
mod dates;
mod detect;
mod extract;
mod install;
mod manifest;
mod runner;
mod server;
mod snapshots;
mod summary;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "utiman", version, about = "Local utility-account manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Port for the dashboard (default 7877).
    #[arg(long, global = true, default_value_t = 7877)]
    port: u16,
    /// Don't open the browser automatically.
    #[arg(long, global = true)]
    no_open: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Start the dashboard (the default when no command is given).
    Serve,
    /// List known providers and whether their CLIs are installed.
    List,
    /// Copy a provider manifest into ~/.config/utiman/providers/.
    Register {
        /// Path to a manifest TOML file.
        file: PathBuf,
    },
    /// Report which bills are due (and how soon). Exit code 2 if anything is
    /// due within --within days or overdue — handy from cron.
    Check {
        /// Days-ahead window for "due soon" (and notifications).
        #[arg(long, default_value_t = 5)]
        within: i64,
        /// Raise a macOS notification for due-soon bills.
        #[arg(long)]
        notify: bool,
        /// Emit JSON instead of a text table.
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(cli.port, !cli.no_open).await,
        Command::List => list().await,
        Command::Register { file } => register(&file),
        Command::Check {
            within,
            notify,
            json,
        } => {
            let code = check::run(within, notify, json).await?;
            std::process::exit(code);
        }
    }
}

async fn serve(port: u16, open_browser: bool) -> Result<()> {
    let app = Arc::new(server::App {
        tasks: Arc::new(install::InstallTasks::default()),
    });
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind {addr} (is another utiman running?)"))?;
    let url = format!("http://127.0.0.1:{port}");
    eprintln!("utiman dashboard: {url}");
    if open_browser {
        let _ = open::that(&url);
    }
    axum::serve(listener, server::router(app)).await?;
    Ok(())
}

async fn list() -> Result<()> {
    println!("ID | NAME | KIND | BINARY | INSTALLED | VERSION | SOURCE");
    for p in manifest::load_providers() {
        let d = detect::detect(&p.manifest.binary).await;
        println!(
            "{} | {} | {} | {} | {} | {} | {:?}",
            p.manifest.id,
            p.manifest.name,
            p.manifest.kind,
            p.manifest.binary,
            if d.installed { "yes" } else { "no" },
            d.version.as_deref().unwrap_or("-"),
            p.source,
        );
    }
    Ok(())
}

fn register(file: &PathBuf) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let m = manifest::parse_manifest(&text)?;
    let dir = manifest::user_providers_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{}.toml", m.id));
    std::fs::write(&dest, &text)?;
    println!("registered {} -> {}", m.id, dest.display());
    Ok(())
}
