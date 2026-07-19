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
# Optional for conforming CLIs (piekstra-cli/1): these default to
# ["self-update"] and ["self-update", "--check", "--json"].
self-update-args = ["self-update", "--yes"]     # CLI updates itself in place
update-check-args = ["self-update", "--check"]  # report-only check

[auth]
required = true
login-command = "acmew auth login"   # run in YOUR terminal; utiman never prompts
# Optional for conforming CLIs (piekstra-cli/1): these default to
# ["auth", "status", "--json"] and "authenticated" (auth-status/v1).
status-args = ["auth", "status", "--json"]   # non-secret status report (JSON)
authenticated-field = "authenticated"        # truthy dot-path = signed in

[summary]                            # fills the dashboard card
args = ["balance", "--json"]
format = "json"                      # "json" (default) or "text"
# Optional for utility/v1-profile CLIs (cli-common >= v0.2.0): a
# `utility-summary/v1` payload is parsed from its schema tag alone —
# `balance` (Money object) + `due_date` — and the fields below are ignored.
balance-fields = ["balance_due"]     # fallbacks, tried in order
due-date-fields = ["due_date"]
# scale = "cents"                    # divide the balance by 100

[pay]                                # hand off to the official payment page
open-args = ["pay", "--open"]        # run the CLI to open it; OR:
# url = "https://acme.example/pay"   # open a portal URL directly
label = "Pay bill"                   # button text (default "Pay bill")

[[series]]                           # charted in the provider's detail drawer
id = "usage"
name = "Water usage by period"
args = ["usage", "list", "--json"]
format = "json"                      # "json" (default) or "table" (pipe-table text)
# A `<record>-list/v1` Paged envelope (utility/v1 profile) is unwrapped
# automatically: records are read from `items`, and Money-object values
# resolve by their decimal amount (value-fields = ["amount"] just works).
items-path = ""                      # dot-path to the record array ("" = JSON root)
label-field = "period"               # each record's x label; also accepts a
                                     # fallback chain, e.g. ["payment_date", "date"]
value-fields = ["quantity"]          # fallbacks, tried in order
unit = "gallons"                     # "usd" formats as money; else shown as-is
# scale = "cents"                    # divide values by 100 (ignored for -list/v1 envelopes)
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
already installed, the Update button runs `<binary> <self-update-args...>`
instead of reinstalling, and the catalog gets a **Check for update** button
running `<binary> <update-check-args...>`. Like the `[auth]` defaults, these
follow the conforming-CLI convention (piekstra-cli/1) when omitted:
`["self-update"]` and `["self-update", "--check", "--json"]`. Set them
explicitly only when the CLI deviates — e.g. it needs a `--yes`-style flag to
skip confirmation, since utiman gives the CLI no TTY and no stdin.

### `[auth]`

Auth stays inside the CLI (typically the OS keychain); utiman never collects
credentials. Three levels of integration, all optional:

- `login-command`: shown as a hint when a summary fails, and (on macOS)
  behind an **Open login in Terminal** button — utiman opens Terminal with
  the command so the interactive login still happens entirely between you
  and the CLI.
- `status-args` + `authenticated-field` (optional; default to the
  piekstra-cli/1 conventions above): `status-args` must print non-secret
  JSON; `authenticated-field` is a dot-path into it whose truthy value means
  signed in (status commands conventionally exit 0 either way, so the answer
  has to come from the output). With both set, cards show a
  "Signed in" / "Sign-in needed" chip.
- `login-steps`: an ordered list of human steps shown before the command in
  the provider's **Setup & sign-in** drawer section — for flows that need
  browser work first (e.g. capturing a session cookie). Backtick spans render
  as inline code. The provider's own detail drawer always shows a sign-in
  section when `required = true`, not only when a fetch fails.

### `[[setup]]`

Non-secret setup **values** utiman collects in a form and applies by running
the CLI — e.g. saving an account number. utiman appends the entered value as
the final argument: `[[setup]]` with `args = ["config", "set-account"]` runs
`<binary> config set-account <value>`.

```toml
[[setup]]
id = "account"
name = "Account number"
description = "The NNNNNNN-N number from your bill."
args = ["config", "set-account"]
placeholder = "1234567-0"
```

This is **only for non-secret values** (account numbers, meter ids) — the kind
that live in plain config and appear in URLs. Credentials never go here; they
go through `[auth]` and the terminal, so utiman's "never handles secrets"
guarantee holds. The value is passed as a single argv element (no shell), and
the CLI's own validation decides whether it's accepted.

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

**utility/v1 fast path:** when the command's output carries a
`"schema": "utility-summary/v1"` tag (a CLI on cli-common ≥ v0.2.0), the
whole field-list mechanism is bypassed — `balance` (a Money object) and
`due_date` are read directly, and `balance-fields`/`due-date-fields`/`scale`
are ignored. Stale field config is safe to leave in place for older CLI
versions; it simply becomes a no-op once the CLI emits the canonical DTO.

### `[[series]]`

Time-series the CLI can report — usage per period, bill amounts, payments.
Each series becomes a chart (with a table view) in the provider's detail
drawer. Records come from a JSON array (`items-path` dot-path, root by
default) or, with `format = "table"`, from a pipe-delimited text table whose
header row names the fields (matched case-insensitively, so `label-field =
"Month"` finds a `MONTH` column). Points are assumed newest-first, matching
how the CLIs print.

**utility/v1 fast path:** a `"<record>-list/v1"`-tagged Paged envelope
(cli-common ≥ v0.2.0) is unwrapped automatically — records are read from its
`items` array regardless of `items-path`, Money-object values resolve by
their decimal amount (so `value-fields = ["amount"]` just works), and any
`scale = "cents"` is suppressed for those records (profile Money is already
decimal dollars). Keep pre-profile paths and `scale` in the manifest as
fallbacks for older CLI versions — they're ignored once the envelope
appears.

utiman also keeps a **local series archive**: every fetched series is
merged into `~/.local/share/utiman/series/<provider>__<series>.jsonl`
(upsert by period label), so charts extend past the CLI's own window the
longer utiman runs — and insight chips can compare a period to the **same
period last year** (the right baseline for seasonal utilities), falling
back to the prior period until a year of history accrues.

Separately from manifest series, utiman records its own **balance snapshot**
on every successful summary (to `~/.local/share/utiman/history/<id>.jsonl`),
so every provider gets a balance-over-time chart and a card sparkline even
when its portal only reports the current balance.

### `[[documents]]`

File-producing commands, offered as downloads in the detail drawer. utiman
appends `<out-flag> <temp path>` to `args`, runs the command (120s timeout),
streams the file back with `filename` as the download name, and deletes the
temp file. The extension picks the content type (pdf/csv/json/txt).

### `[pay]`

How to pay — utiman only ever opens the provider's **official** payment page,
never collecting or transmitting card data. Set exactly one of:

- `open-args`: CLI args that hand off to the pay page (utiman runs
  `<binary> <open-args>`; the CLI opens the browser). e.g. `["pay", "--open"]`.
- `url`: a payment-portal URL opened directly in the browser — for CLIs whose
  only payment command *makes* a payment (which utiman won't drive) or that
  need flags utiman can't supply.

`label` sets the button text (default "Pay bill").

```toml
[pay]
open-args = ["pay", "--open"]   # or: url = "https://provider.example/pay"
label = "Pay bill"
```

### `[[operations]]`

Read-only(ish) commands exposed as buttons on the provider's card; output is
shown raw in a modal (pretty-printed when it's JSON). Keep mutating commands
(payments!) out of manifests — those belong in your terminal, where the CLI
can prompt and confirm.

## Timeouts and errors

Every run is killed after 45 seconds. A non-zero exit shows the CLI's stderr
in the card, plus the `login-command`/`setup-command` hint when one is
declared.
