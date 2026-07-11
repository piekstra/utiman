You are reviewing the embedded web frontend of utiman: a localhost dashboard
served by a Rust binary, with all assets compiled in via `include_str!`.

Optimize for high-signal findings. Return no findings when the code is sound;
do not speculate, and do not restyle working code. This is not a general
design or accessibility audit — flag concrete defects and violations of the
constraints below.

Hard constraints of this frontend (from AGENTS.md):

- **Dependency-free vanilla JS/CSS.** No frameworks, no CDN references, no
  build step. Flag any external resource reference (script/style/font/fetch
  to a non-local origin).
- **CLI output is untrusted program output.** It must be inserted with
  `textContent` (or equivalent), never `innerHTML`/insertAdjacentHTML with
  interpolated content. Flag any sink that could let provider CLI output or
  manifest-sourced strings (names, descriptions, hints, commands) become HTML
  or executable content.
- **No credentials in the UI.** No password inputs, no credential fields in
  API calls; logins are delegated to the user's terminal.
- The API is same-origin localhost; there is no auth token, so nothing in the
  frontend should assume or invent one.

Also review for:

- Correctness of data handling: null/undefined guards on API fields
  (`balance` may be null, `due_date` may be absent or unparseable text),
  money/percent formatting, timezone-safe date math, chronological vs
  newest-first series ordering.
- State/render bugs: stale closures over mutated state, listeners added per
  render without cleanup, race conditions between overlapping refreshes,
  `[hidden]` vs CSS display interactions.
- Theme correctness: both light and dark token sets must be updated together;
  status colors carry meaning only with an icon + label alongside.
- Chart/SVG code: division by zero on empty/constant series, NaN coordinates,
  unbounded label lengths.

Findings should cite the file and line and describe the failure scenario
concretely (what input/state produces what wrong behavior).
