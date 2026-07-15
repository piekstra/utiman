# utiman

One local dashboard for your utility accounts — electric, water, sewer, and
whatever comes next — built on top of per-provider **CLIs** instead of a pile
of portal logins.

```
┌─ utiman ── localhost:7877 ──────────────────────┐
│  Utilities                         ⟳ Refresh    │
│                                    Total due    │
│  Acme Electric   $142.10  due 07/22  $288.06    │
│  Acme Water       $84.21  due 07/18             │
│  Acme Sewer       $61.75  due 08/01             │
│                                                 │
│  Catalog · install / self-update / check ⤓      │
│  Register a CLI · paste a manifest, done        │
└─────────────────────────────────────────────────┘
```

`utiman` runs a small web server on `127.0.0.1`, opens your browser, and
shells out to provider CLIs behind the scenes. Everything stays on your
machine.

Each provider card drills into a detail view with **charts and history**:
usage per period, bill amounts, payment history (whatever its CLI reports),
plus a **balance-over-time chart** utiman records locally at every refresh —
so there's a trend line even for portals that only show the current balance.
Providers can also expose **downloadable documents** (bill PDFs) and raw
commands. Auth state shows as a signed-in chip per card, with one-click
"open login in Terminal" (credentials never pass through utiman).

## Why CLIs underneath?

Utility portals rarely have APIs. Purpose-built CLIs already solve the hard
parts — auth, scraping, rate-limiting etiquette, keychain storage — and are
independently useful in a terminal or a script. utiman just gives them one
pane of glass. The built-in catalog:

| Provider | CLI | What it is |
|---|---|---|
| [FPL Electric](https://github.com/piekstra/fpl-cli) | `fpl` | Florida Power & Light |
| [Town of Jupiter Water](https://github.com/piekstra/town-of-jupiter-fl-cli) | `tojfl` | Town of Jupiter, FL utility billing |
| [Loxahatchee River Sewer](https://github.com/piekstra/loxahatchee-river-fl-cli) | `lrfl` | Loxahatchee River District |
| [Xfinity Internet](https://github.com/piekstra/xfinity-cli) | `xfin` | Xfinity/Comcast internet |

## Install

```sh
cargo install --git https://github.com/piekstra/utiman
utiman                       # serves http://127.0.0.1:7877 and opens it
```

Provider CLIs can be installed from the dashboard's catalog (it runs
`cargo install` for you and streams the log), or however you prefer — utiman
detects anything already on your `PATH`.

Logins happen **in your terminal**, not in the dashboard: when a provider
needs auth, its card tells you what to run (e.g. `fpl init`). utiman never
sees or stores credentials — each CLI keeps its own, typically in the OS
keychain.

## Extending it

Any CLI can become a provider with a small TOML manifest — which binary to
run, how to install/update it, and how to read a balance and due date out of
its output (JSON dot-paths or `Key: value` text labels). Register manifests
from the dashboard, with `utiman register <file>`, or by dropping a file in
`~/.config/utiman/providers/`.

See **[docs/manifests.md](docs/manifests.md)** for the full format, including
self-update support (`self-update-args`) for CLIs that can update themselves.

## CLI

The dashboard is the main interface, but the basics work headless too:

```sh
utiman                        # serve + open browser
utiman --port 9000 --no-open  # serve only
utiman list                   # providers + installed/version status
utiman register acme.toml     # add a provider manifest
```

## Security posture

- Binds to `127.0.0.1` only, and rejects requests whose `Host` header isn't
  local (DNS-rebinding guard) — a web page you visit can't drive your CLIs.
- No credentials: auth lives in each CLI (OS keychain); interactive logins
  never pass through utiman.
- Manifests can only name a bare binary (resolved on `PATH`, spawned without
  a shell) — but a manifest *does* choose what runs, so only register
  manifests you trust, same as anything you install.
- Nothing is persisted server-side: account data lives only in the API
  responses your own browser requests.

## Troubleshooting

**A provider card hangs or shows "unknown" auth status on macOS** — the first
time utiman runs a freshly (re)installed CLI that reads the OS keychain,
macOS shows a one-time "allow keychain access" prompt for that binary. Click
**Always Allow** and refresh. (Each `cargo install` update produces a new
binary, so the prompt can reappear after updates.)

## Development

```sh
cargo test                    # manifest parsing, extraction, money handling
cargo run -- --no-open        # local dev server
```

The built-in catalog lives in [`catalog/`](catalog); adding a provider to it
is a PR with one TOML file.

## License

MIT or Apache-2.0, at your option.
