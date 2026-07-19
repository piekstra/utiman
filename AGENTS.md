# AGENTS.md — utiman

Canonical agent entrypoint for this repo. `CLAUDE.md` is a one-line pointer here.

## What this is

A local-only web dashboard (`utiman`, Rust/axum) over per-provider utility
CLIs. The server binds 127.0.0.1, serves an embedded vanilla-JS frontend, and
shells out to provider binaries described by TOML manifests.

## Local map

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | clap CLI: `serve` (default), `list`, `register`, `check`, `self-update` |
| `build.rs` | bakes `BUILD_TARGET` (target triple) for self-update asset selection |
| `src/manifest.rs` | manifest schema, validation, builtin+user loading/merge |
| `src/extract.rs` | balance/due-date extraction: JSON dot-paths, text labels, money parsing |
| `src/runner.rs` | spawn a provider CLI (no shell), capture output, timeout kill |
| `src/detect.rs` | PATH lookup + `--version` probe |
| `src/install.rs` | background install/update tasks with streamed logs |
| `src/snapshots.rs` | local balance history (`~/.local/share/utiman/history/*.jsonl`) |
| `src/archive.rs` | local series archive (`~/.local/share/utiman/series/*.jsonl`) so charts outlive the CLI window |
| `src/summary.rs` | shared "run summary → balance + due" helper (server + `check`) |
| `src/check.rs` | `utiman check`: due-date report, urgency triage, macOS notify; `--anomalies` bill-shock report |
| `src/anomaly.rs` | flags an unusually-high latest bill (>40% over the prior-periods median) |
| `src/dates.rs` | dependency-free date parsing (ISO/US dates + cycle labels like `Jun 2026` / `2026-06` via `parse_label`) + days-from-today math |
| `src/server.rs` | axum routes, Host-header guard, API handlers |
| `src/assets/` | embedded frontend (index.html / app.js / charts.js / style.css) |
| `catalog/` | built-in provider manifests (embedded via `include_str!`) |
| `docs/manifests.md` | the manifest format reference |

## Durable conventions (do not drift)

- **This is a public repo.** Never commit real account numbers, addresses,
  balances, credentials, or captured portal output. Examples use placeholder
  values (`1234567-0` style). Test fixtures are synthetic.
- utiman never handles credentials; logins are delegated to the provider CLIs
  in the user's own terminal. Don't add password fields to the UI or API.
- The server must stay loopback-only: keep the 127.0.0.1 bind and the Host
  header check in `server.rs`.
- Extensibility is data, not code: new providers are manifests. Don't
  special-case a provider in Rust; extend the manifest schema instead, with
  validation and a test.
- CLI invocations are argv arrays spawned without a shell, with a timeout.
  Manifest `binary` is a bare name (validation rejects paths).
- Frontend is dependency-free vanilla JS; CLI output is inserted with
  `textContent`, never `innerHTML`.

## Verify

`cargo test && cargo clippy --all-targets` must be clean. For behavior
changes, run `cargo run -- --no-open` and exercise the affected API route
with curl.
