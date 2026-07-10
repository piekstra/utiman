# Provider manifests

A provider is described entirely by a TOML manifest. utiman ships a built-in
catalog (embedded in the binary from [`catalog/`](../catalog)), and you extend
it by dropping manifests into `~/.config/utiman/providers/` — via the
dashboard's **Register a CLI** panel, `utiman register <file.toml>`, or just
copying a file there. A user manifest with the same `id` as a built-in
overrides it; that's also how you customize a built-in (copy, edit, register).

utiman only ever runs the CLI named by `binary` (resolved on `PATH`, spawned
directly with the argv from the manifest — no shell), plus the manifest's
install command. It never handles credentials: interactive logins happen in
your terminal, and the dashboard just shows you which command to run.

## Full example

```toml
id = "acme-water"                    # unique, kebab-case
name = "Acme Water"                  # display name
kind = "water"                       # electric | water | sewer | gas | internet | trash | other
binary = "acmew"                     # command looked up on PATH (never a path)
repo = "https://github.com/you/acme-water-cli"
description = "Acme municipal water portal: balance, usage, bills."

# Optional non-secret one-time setup, shown as a hint in the UI.
setup-command = "acmew config set-account <your-account>"

[install]
kind = "cargo-git"                   # cargo-git | cargo | custom
git = "https://github.com/you/acme-water-cli"
package = "acmew-cli"                # optional: select a workspace package
# crate = "acmew-cli"                # for kind = "cargo" (crates.io)
# command = ["brew", "install", "acmew"]   # for kind = "custom"
self-update-args = ["self-update", "--yes"]     # optional: CLI updates itself
update-check-args = ["self-update", "--check"]  # optional: report-only check

[auth]
required = true
login-command = "acmew auth login"   # run in YOUR terminal; utiman never prompts
status-args = ["auth", "status", "--json"]   # non-secret status report (JSON)
authenticated-field = "authenticated"        # truthy dot-path = signed in

[summary]                            # fills the dashboard card
args = ["balance", "--json"]
format = "json"                      # "json" (default) or "text"
balance-fields = ["balance_due"]     # fallbacks, tried in order
due-date-fields = ["due_date"]
# scale = "cents"                    # divide the balance by 100

[[series]]                           # charted in the provider's detail drawer
id = "usage"
name = "Water usage by period"
args = ["usage", "list", "--json"]
format = "json"                      # "json" (default) or "table" (pipe-table text)
items-path = ""                      # dot-path to the record array ("" = JSON root)
label-field = "period"               # each record's x label
value-fields = ["quantity"]          # fallbacks, tried in order
unit = "gallons"                     # "usd" formats as money; else shown as-is
# scale = "cents"                    # divide values by 100
chart = "bar"                        # "bar" (default) or "line"

[[documents]]                        # downloadable files (bill PDFs etc.)
id = "bill"
name = "Latest bill (PDF)"
args = ["bills", "download"]         # utiman appends: <out-flag> <temp path>
out-flag = "--out"
filename = "acme-bill.pdf"           # download name; extension sets content type

[[operations]]                       # raw-output buttons in the drawer
id = "history"
name = "Payment history"
args = ["history", "--json"]
```

## Field reference

### Top level

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Unique kebab-case identifier; the user-manifest filename becomes `<id>.toml` |
| `name` | yes | Display name |
| `kind` | no | Utility category chip (`other` if omitted) |
| `binary` | yes | Command name resolved on `PATH`. Bare names only — paths are rejected |
| `repo` | yes | Source repository URL, linked from the catalog |
| `description` | no | One-liner for the catalog |
| `setup-command` | no | Non-secret setup hint (e.g. saving an account number) |

### `[install]`

| `kind` | Uses | Runs |
|---|---|---|
| `cargo-git` | `git`, optional `package` | `cargo install --force --git <git> [package]` |
| `cargo` | `crate` | `cargo install --force <crate>` |
| `custom` | `command` | the argv as given |

`self-update-args` / `update-check-args` (optional, any kind): when the CLI is
already installed and supports updating itself, the Update button runs
`<binary> <self-update-args...>` instead of reinstalling, and the catalog gets
a **Check for update** button running `<binary> <update-check-args...>`.
Use a non-interactive form (`--yes`-style flags) — utiman gives the CLI no TTY
and no stdin.

### `[auth]`

Auth stays inside the CLI (typically the OS keychain); utiman never collects
credentials. Three levels of integration, all optional:

- `login-command`: shown as a hint when a summary fails, and (on macOS)
  behind an **Open login in Terminal** button — utiman opens Terminal with
  the command so the interactive login still happens entirely between you
  and the CLI.
- `status-args` + `authenticated-field`: `status-args` must print non-secret
  JSON; `authenticated-field` is a dot-path into it whose truthy value means
  signed in (status commands conventionally exit 0 either way, so the answer
  has to come from the output). With both set, cards show a
  "Signed in" / "Sign-in needed" chip.

### `[summary]`

How the dashboard card gets its numbers. `args` is run with the binary; then:

- `format = "json"`: fields are **dot-paths** into the JSON output —
  `balance.cents`, `services.0.due_date` (numeric segments index arrays).
- `format = "text"`: fields are **labels** matched case-insensitively against
  `Key: value` lines of text output.

Both field lists are fallback chains, tried in order — useful when the
upstream payload shape varies. Balances accept numbers or strings
(`"$1,234.56"`, `(5.00)` for credits); `scale = "cents"` divides by 100.
Extracted balances are displayed as USD.

### `[[series]]`

Time-series the CLI can report — usage per period, bill amounts, payments.
Each series becomes a chart (with a table view) in the provider's detail
drawer. Records come from a JSON array (`items-path` dot-path, root by
default) or, with `format = "table"`, from a pipe-delimited text table whose
header row names the fields (matched case-insensitively, so `label-field =
"Month"` finds a `MONTH` column). Points are assumed newest-first, matching
how the CLIs print.

Separately from manifest series, utiman records its own **balance snapshot**
on every successful summary (to `~/.local/share/utiman/history/<id>.jsonl`),
so every provider gets a balance-over-time chart and a card sparkline even
when its portal only reports the current balance.

### `[[documents]]`

File-producing commands, offered as downloads in the detail drawer. utiman
appends `<out-flag> <temp path>` to `args`, runs the command (120s timeout),
streams the file back with `filename` as the download name, and deletes the
temp file. The extension picks the content type (pdf/csv/json/txt).

### `[[operations]]`

Read-only(ish) commands exposed as buttons on the provider's card; output is
shown raw in a modal (pretty-printed when it's JSON). Keep mutating commands
(payments!) out of manifests — those belong in your terminal, where the CLI
can prompt and confirm.

## Timeouts and errors

Every run is killed after 45 seconds. A non-zero exit shows the CLI's stderr
in the card, plus the `login-command`/`setup-command` hint when one is
declared.
