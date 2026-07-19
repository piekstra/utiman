// utiman dashboard. Vanilla JS; all data comes from the local API, and all
// CLI output is inserted with textContent (never innerHTML) since it is
// arbitrary program output.

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];
const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

const state = {
  providers: [],
  summaries: new Map(),   // id -> summary response
  auth: new Map(),        // id -> "authenticated" | "unauthenticated" | "unknown"
  snapshots: new Map(),   // id -> [{ts, balance, due_date}]
  series: new Map(),      // "id/sid" -> series response (cached per refresh)
  checkedAt: new Map(),   // id -> Date.now() of last summary fetch
  refreshedAt: null,
  os: null,               // server host OS ("macos" | "linux" | ...)
  loading: false,         // a full load/refresh is fetching summaries
};

const isMac = () => state.os === "macos";

const KIND = {
  electric: { icon: "i-bolt", hue: "var(--kind-electric)" },
  water: { icon: "i-droplet", hue: "var(--kind-water)" },
  sewer: { icon: "i-waves", hue: "var(--kind-sewer)" },
  gas: { icon: "i-flame", hue: "var(--kind-gas)" },
  internet: { icon: "i-wifi", hue: "var(--kind-internet)" },
  trash: { icon: "i-trash", hue: "var(--kind-other)" },
  other: { icon: "i-box", hue: "var(--kind-other)" },
};

async function api(path, opts) {
  const res = await fetch(path, opts);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || `${res.status} ${res.statusText}`);
  return body;
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const c of children) if (c != null) node.append(c);
  return node;
}

function icon(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ic");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#${name}`);
  svg.append(use);
  return svg;
}

function badge(kind) {
  const k = KIND[kind] || KIND.other;
  const b = el("span", { class: "badge" });
  b.style.setProperty("--kind-c", k.hue);
  b.append(icon(k.icon));
  return b;
}

function pill(cls, iconName, text) {
  const p = el("span", { class: `pill ${cls}` });
  if (iconName) p.append(icon(iconName));
  p.append(text);
  return p;
}

function toast(message, kind = "ok") {
  const t = el("div", { class: `toast ${kind}` });
  t.append(icon(kind === "ok" ? "i-check" : "i-x"), message);
  $("#toasts").append(t);
  setTimeout(() => t.remove(), 4500);
}

function relTime(ts) {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 8) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  return `${Math.round(s / 3600)}h ago`;
}

/** Due dates come from scrapers and vary wildly; when the raw string isn't
 * itself a date, pull out the first date-looking substring (e.g. the portal
 * text "… (Saturday, July 11, 2026)" becomes "July 11, 2026"). */
function cleanDueDate(s) {
  if (!s) return s;
  if (!Number.isNaN(Date.parse(s))) return s;
  const m = s.match(/\d{1,2}\/\d{1,2}\/\d{2,4}|[A-Z][a-z]+ \d{1,2},? \d{4}/);
  return m ? m[0] : s;
}

function daysUntil(dateStr) {
  const t = Date.parse(dateStr);
  if (Number.isNaN(t)) return null;
  return Math.ceil((t - Date.now()) / 86400000);
}

function duePill(dateStr) {
  const d = daysUntil(dateStr);
  if (d == null) return null;
  if (d < 0) return pill("crit", "i-x", `overdue ${-d}d`);
  if (d === 0) return pill("warn", "i-check", "due today");
  if (d <= 7) return pill("warn", null, `in ${d}d`);
  return pill("", null, `in ${d}d`);
}

// ---------- routing ----------

function route() {
  const h = location.hash || "#/dashboard";
  const provider = h.match(/^#\/p\/([a-z0-9-]+)$/)?.[1];
  const tab = provider ? "dashboard" : (h.match(/^#\/(dashboard|catalog|add)$/)?.[1] || "dashboard");
  for (const sec of ["dashboard", "catalog", "add"]) {
    $(`#view-${sec}`).hidden = sec !== tab;
  }
  $$(".tabs a").forEach((a) => a.classList.toggle("active", a.dataset.tab === tab));
  const p = provider && state.providers.find((x) => x.id === provider);
  if (p) openDrawer(p);
  else hideDrawer();
}
window.addEventListener("hashchange", route);

// ---------- dashboard ----------

function summaryProviders() {
  return state.providers.filter((p) => p.summary && p.detection.installed);
}

function renderDashboard() {
  renderStats();
  renderHighlights();
  renderTimeline();
  renderCards();
  renderRollup();
  const any = summaryProviders().length > 0;
  // While a load is in flight, keep the stats strip up (it carries the loading
  // hero + progress) and never flash the "no providers yet" empty state before
  // we've even finished discovering them.
  $("#dash-empty").hidden = any || state.loading;
  $("#stats-strip").hidden = !any && !state.loading;
}

function renderStats() {
  const oks = [...state.summaries.entries()].filter(([, s]) => s.state === "ok" && s.balance != null);
  const { done, total } = loadProgress();
  const hero = $("#hero-value");
  $("#load-bar").hidden = !state.loading;

  if (state.loading && done === 0) {
    // Nothing read yet — a shimmer reads as "working", where "$0.00" would
    // look like a real (wrong) total.
    hero.textContent = "";
    hero.classList.add("loading");
  } else {
    hero.classList.remove("loading");
    hero.textContent = usd.format(oks.reduce((sum, [, s]) => sum + s.balance, 0));
  }
  // While a load is in flight, show how many accounts are still pending so the
  // total visibly builds up instead of jumping from nothing to the final value.
  const prog = $("#hero-progress");
  if (state.loading && done < total) {
    prog.textContent = `checking ${total - done} of ${total} account${total === 1 ? "" : "s"}…`;
    prog.hidden = false;
  } else {
    prog.hidden = true;
  }

  // The total only sums accounts we could read. If some failed (expired
  // session etc.), say so — an unqualified total would silently under-report.
  // Count only providers whose summary actually errored; ones still loading
  // (no summary entry yet) aren't "couldn't be read", so the caveat doesn't
  // flash during a refresh.
  const installed = summaryProviders().length;
  const failed = [...state.summaries.values()].filter((s) => s.state === "error").length;
  const note = $("#hero-caveat");
  if (failed > 0 && installed > 0) {
    note.textContent = `${failed} of ${installed} account${installed === 1 ? "" : "s"} couldn't be read`;
    note.hidden = false;
  } else {
    note.hidden = true;
  }

  const side = $("#stats-side");
  side.replaceChildren();

  // Next due across providers.
  const dues = oks
    .map(([id, s]) => {
      const due = s.due_date ? cleanDueDate(s.due_date) : null;
      return { id, s, due, days: due ? daysUntil(due) : null };
    })
    .filter((d) => d.days != null && d.days >= 0)
    .sort((a, b) => a.days - b.days);
  if (dues.length) {
    const d = dues[0];
    const p = state.providers.find((x) => x.id === d.id);
    const stat = el("div", { class: "stat" });
    stat.append(
      el("div", { class: "stat-label" }, "Next due"),
      el("div", { class: "stat-value" }, `${usd.format(d.s.balance)} · ${d.days === 0 ? "today" : `in ${d.days}d`}`),
      el("div", { class: "stat-sub" }, `${p?.name || d.id} — ${d.due}`)
    );
    side.append(stat);
  }

  const authed = summaryProviders().filter((p) => effectiveAuth(p) === "authenticated").length;
  const stat = el("div", { class: "stat" });
  stat.append(
    el("div", { class: "stat-label" }, "Providers"),
    el("div", { class: "stat-value" }, String(summaryProviders().length)),
    el("div", { class: "stat-sub" }, authed ? `${authed} signed in` : " ")
  );
  side.append(stat);
}

/** One top observation per provider, computed from its first series. */
function renderHighlights() {
  let box = $("#highlights");
  if (!box) {
    box = el("div", { class: "highlights", id: "highlights" });
    $("#stats-strip").after(box);
  }
  box.replaceChildren();
  let count = 0;
  for (const p of summaryProviders()) {
    for (const s of p.series || []) {
      const r = state.series.get(`${p.id}/${s.id}`);
      if (!r?.ok || r.points.length < 2) continue;
      const stats = seriesStats(r.points);
      const c = stats.compare;
      if (!c || c.pct == null) continue;
      const line = el("div", { class: "highlight" });
      const dir = c.delta >= 0 ? "▲" : "▼";
      const cls = c.delta >= 0 ? "delta-up" : "delta-down";
      const vs = c.seasonal ? `vs ${c.label} last year` : `vs ${c.label}`;
      line.append(
        el("strong", {}, p.name),
        `${s.name.toLowerCase()}: ${fmtVal(stats.latest.value, r.unit)} (`,
        el("span", { class: cls }, `${dir} ${Math.abs(c.pct).toFixed(1)}%`),
        ` ${vs})`
      );
      box.append(line);
      count += 1;
      break; // one highlight per provider
    }
  }
  // Collapse the box entirely when there's nothing to show, so it doesn't
  // reserve blank vertical space before series data has streamed in.
  box.hidden = count === 0;
}

// Bills we can actually place on a calendar: read OK, a positive balance owed,
// and a parseable due date. Soonest-first, overdue included (negative days).
function upcomingBills() {
  const out = [];
  for (const p of summaryProviders()) {
    const s = state.summaries.get(p.id);
    if (!s || s.state !== "ok" || !(s.balance > 0) || !s.due_date) continue;
    const due = cleanDueDate(s.due_date);
    const days = daysUntil(due);
    if (days == null) continue;
    out.push({ id: p.id, name: p.name, kind: p.kind, balance: s.balance, due, days });
  }
  return out.sort((a, b) => a.days - b.days);
}

// A horizontal "what's coming up" timeline: each dated bill as a marker placed
// by its due date, colored by urgency. Needs 2+ dated bills to beat the single
// "Next due" stat; otherwise it stays hidden.
function renderTimeline() {
  let box = $("#timeline");
  if (!box) {
    box = el("div", { class: "timeline", id: "timeline" });
    ($("#highlights") || $("#stats-strip")).after(box);
  }
  const bills = upcomingBills();
  if (bills.length < 2) {
    box.replaceChildren();
    box.hidden = true;
    return;
  }
  box.hidden = false;
  box.replaceChildren();

  // Window: today (0) on the left — or the most-overdue bill if earlier — out to
  // the furthest due date, but at least a 30-day runway so near bills aren't
  // crammed against the edge.
  const minDay = Math.min(0, ...bills.map((b) => b.days));
  const maxDay = Math.max(30, ...bills.map((b) => b.days));
  const span = maxDay - minDay || 1;
  // Keep every marker inside a 6%–94% band so a centered label at either end
  // still has room and can't push the page into a horizontal scroll.
  const frac = (d) => 6 + ((d - minDay) / span) * 88;
  const urgencyOf = (d) => (d < 0 ? "crit" : d <= 7 ? "warn" : "ok");
  const whenLabel = (d) => (d < 0 ? `overdue ${-d}d` : d === 0 ? "due today" : `in ${d}d`);

  const total = bills.reduce((sum, b) => sum + b.balance, 0);
  box.append(el("div", { class: "timeline-head" },
    el("strong", {}, "Upcoming bills"),
    el("span", { class: "timeline-sub" },
      `${bills.length} bills · ${usd.format(total)} over the next ${maxDay} days`)));

  const track = el("div", { class: "timeline-track" });
  track.append(el("div", { class: "tl-line" }));

  // "Today" tick.
  const today = el("div", { class: "tl-today" });
  today.style.left = `${frac(0)}%`;
  today.append(el("span", { class: "tl-today-label" }, "Today"));
  track.append(today);

  // Stagger labels above/below by *proximity*, not index parity: place each
  // marker on whichever side's last marker is farther away. Bills are sorted by
  // day, so a cluster in the same week spreads across both rows (e.g. 3 close
  // bills go above/below/above) instead of stacking two labels on one side.
  const lastX = { above: -Infinity, below: -Infinity };
  const sideFor = (x) => {
    const side = x - lastX.above >= x - lastX.below ? "above" : "below";
    lastX[side] = x;
    return side;
  };

  bills.forEach((b) => {
    const marker = el("div", {
      class: `tl-bill ${urgencyOf(b.days)} ${sideFor(frac(b.days))}`,
      role: "button", tabindex: "0",
      title: `${b.name}: ${usd.format(b.balance)} due ${b.due} (${whenLabel(b.days)})`,
    });
    marker.style.left = `${frac(b.days)}%`;
    marker.append(
      el("div", { class: "tl-label" },
        el("span", { class: "tl-amt" }, usd.format(b.balance)),
        el("span", { class: "tl-who" }, `${b.name} · ${whenLabel(b.days)}`)),
      el("div", { class: "tl-dot" }));
    const open = () => { location.hash = `#/p/${b.id}`; };
    marker.addEventListener("click", open);
    marker.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); open(); }
    });
    track.append(marker);
  });
  box.append(track);
}

function statusLine(cls, iconName, text) {
  const s = el("div", { class: `card-status ${cls}` });
  s.append(icon(iconName), text);
  return s;
}

// Reconcile the standalone auth-status ("is there a session?") with the actual
// summary fetch ("does the session still work?"). A CLI can report a session
// exists while the portal has already expired it — which surfaced as a green
// "signed in" chip sitting right next to a "Couldn't fetch → run auth login"
// error. When a fetch fails in a way that points back to re-authenticating
// (its hint is the login command, and it wasn't just a timeout), trust the
// fetch and treat the session as expired rather than claiming it's signed in.
function effectiveAuth(p) {
  const s = state.summaries.get(p.id);
  const stale =
    s?.state === "error" && !s.timed_out && s.hint && s.hint === p.auth?.["login-command"];
  if (stale) return "expired";
  return state.auth.get(p.id) ?? null;
}

function authChip(p) {
  switch (effectiveAuth(p)) {
    case "authenticated":
      return pill("good", "i-check", "signed in");
    case "expired":
      return pill("warn", "i-clock", "session expired");
    case "unauthenticated":
      return pill("warn", null, "sign-in needed");
    default:
      return null;
  }
}

// A card whose summary hasn't arrived yet: a shimmering value placeholder plus
// a clearly-labelled "Checking…" line, so a loading card reads as in-progress
// rather than blank or broken.
function skeletonBody() {
  const wrap = el("div", { class: "card-loading" });
  const spin = icon("i-refresh");
  spin.classList.add("spin");
  const status = el("div", { class: "card-status checking" });
  status.append(spin, "Checking…");
  wrap.append(el("div", { class: "skeleton skel-value" }), status);
  return wrap;
}

// "2026-06" -> "Jun 2026" for compact month labels.
function monthLabel(key) {
  const M3 = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const [y, m] = key.split("-");
  return `${M3[Number(m) - 1]} ${y}`;
}

// Aggregate each provider's dollar-denominated history by calendar month into
// stacked per-provider segments — the cross-provider "money out for utilities"
// view the per-account charts can't give. One series per provider (a billed
// amount is preferred over a payments series so semantics don't mix within a
// provider, and two usd series can never double-count); kWh/gallon series are
// skipped. A provider that only reports payments (e.g. a quarterly sewer bill)
// still contributes its best-available money signal rather than being dropped.
function spendRollup() {
  const provMap = new Map();          // id -> { id, name, kind }
  const byMonth = new Map();          // "YYYY-MM" -> Map(id -> summed value)
  // Prefer a billed-cost series over a payments one when a provider has both.
  const isCost = (s) => /bill|cost|charge|amount/i.test(`${s.id} ${s.name || ""}`);
  for (const p of summaryProviders()) {
    // (named usdSeries, not usd, so it can't shadow the module-level currency
    // formatter and turn a future usd.format(...) into a runtime error.)
    const usdSeries = (p.series || [])
      .map((s) => ({ s, r: state.series.get(`${p.id}/${s.id}`) }))
      .filter((x) => x.r?.ok && x.r.unit === "usd");
    if (!usdSeries.length) continue;
    const chosen = usdSeries.find((x) => isCost(x.s)) || usdSeries[0];
    provMap.set(p.id, { id: p.id, name: p.name, kind: p.kind });
    for (const pt of chosen.r.points) {
      const d = labelToDate(pt.label);
      if (!d) continue;
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
      if (!byMonth.has(key)) byMonth.set(key, new Map());
      const mm = byMonth.get(key);
      // Count charges only: a credit/refund (negative) is money back, not
      // spend — clamp to 0 so it never inflates the total the way abs() would.
      mm.set(p.id, (mm.get(p.id) || 0) + Math.max(0, pt.value));
    }
  }
  const providers = [...provMap.values()];
  const months = [...byMonth.entries()]
    .sort((a, b) => (a[0] < b[0] ? -1 : 1))
    .slice(-12) // a year is plenty of bars
    .map(([key, mm]) => {
      const segments = [...mm.entries()].map(([id, value]) => ({
        id, value, name: provMap.get(id).name, kind: provMap.get(id).kind,
      }));
      return { label: monthLabel(key), total: segments.reduce((a, s) => a + s.value, 0), segments };
    });
  return { months, providers };
}

// Stacked "Total monthly spend" chart. Injected after the timeline/highlights;
// only shown with 2+ providers and 2+ months, where it adds something the
// single-account charts don't.
function renderRollup() {
  let box = $("#rollup");
  if (!box) {
    box = el("div", { class: "rollup", id: "rollup" });
    ($("#timeline") || $("#highlights") || $("#stats-strip")).after(box);
  }
  const { months, providers } = spendRollup();
  if (months.length < 2 || providers.length < 2) {
    box.replaceChildren();
    box.hidden = true;
    return;
  }
  box.hidden = false;
  box.replaceChildren();
  const latest = months[months.length - 1];
  box.append(el("div", { class: "rollup-head" },
    el("strong", {}, "Total monthly spend"),
    el("span", { class: "rollup-sub" },
      `${providers.length} utilities · latest ${latest.label}: ${usd.format(latest.total)}`)));
  box.append(renderSpendChart(months, providers));
}

function renderCards() {
  const cards = $("#cards");
  cards.replaceChildren();
  for (const p of state.providers.filter((x) => x.summary)) {
    const card = el("article", { class: "card" });
    const top = el("div", { class: "card-top" });
    const title = el("div", { class: "card-title" });
    const checked = state.checkedAt.get(p.id);
    title.append(
      el("div", { class: "card-name" }, p.name),
      el("div", { class: "card-sub" },
        checked ? `${p.kind} · checked ${relTime(checked)}` : p.kind)
    );
    top.append(badge(p.kind), title);

    const chip = authChip(p);
    if (chip) top.append(chip);

    // Per-card refresh: re-hit just this provider, so the full Refresh (which
    // re-queries every portal) isn't the only way to update one — and it's
    // gentler on the portals the README promises to be polite to.
    if (p.detection.installed) {
      const rb = el("button", { class: "icon-btn card-refresh", title: `Refresh ${p.name}` });
      rb.append(icon("i-refresh"));
      rb.addEventListener("click", (ev) => {
        ev.stopPropagation();
        refreshOne(p, rb);
      });
      top.append(rb);
    }
    card.append(top);

    if (!p.detection.installed) {
      card.append(statusLine("", "i-box", "CLI not installed"));
      const go = el("a", { class: "card-more", href: "#/catalog" }, "Install from catalog");
      go.append(icon("i-chevron"));
      card.append(go);
      cards.append(card);
      continue;
    }

    const s = state.summaries.get(p.id);
    if (!s) {
      card.append(skeletonBody());
    } else if (s.state === "ok") {
      card.append(el("div", { class: "card-value" },
        s.balance == null ? "—" : usd.format(s.balance)));
      const due = el("div", { class: "due-row" });
      if (s.due_date) {
        const cleaned = cleanDueDate(s.due_date);
        due.append(`Due ${cleaned}`);
        const dp = duePill(cleaned);
        if (dp) due.append(dp);
      } else {
        due.append("No due date reported");
      }
      card.append(due);
      // A payment you made hasn't posted to the portal balance yet.
      if (s.pending) {
        const pend = el("div", { class: "card-status pending" });
        pend.append(icon("i-clock"), "Payment pending — not yet reflected in the balance");
        card.append(pend);
      }
      const pay = payButton(p);
      if (pay) card.append(el("div", { class: "card-pay" }, pay));
    } else {
      card.append(statusLine("crit", "i-x", s.timed_out ? "Timed out" : "Couldn't fetch"));
      if (s.hint) {
        const hint = el("div", { class: "card-status warn" });
        hint.append("Run: ", el("code", {}, s.hint));
        if (p.auth?.["login-command"] === s.hint) {
          const tb = loginTerminalButton(p, "Open in Terminal");
          if (tb) hint.append(tb);
        }
        card.append(hint);
      }
      if (s.stderr) {
        const d = el("details");
        d.append(el("summary", {}, "Details"));
        const pre = el("pre");
        pre.textContent = s.stderr;
        d.append(pre);
        card.append(d);
      }
    }

    const snaps = state.snapshots.get(p.id) || [];
    const spark = typeof renderSparkline === "function" ? renderSparkline(snaps) : null;
    if (spark) {
      const wrap = el("div", { class: "card-spark" });
      wrap.append(spark, el("span", { class: "spark-note" }, "balance trend"));
      card.append(wrap);
    }

    const more = el("div", { class: "card-more" }, "Details & charts");
    more.append(icon("i-chevron"));
    card.append(more);

    card.classList.add("clickable");
    card.tabIndex = 0;
    card.setAttribute("role", "button");
    card.addEventListener("click", (ev) => {
      if (ev.target.closest("button, a, details, code")) return;
      location.hash = `#/p/${p.id}`;
    });
    card.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && ev.target === card) location.hash = `#/p/${p.id}`;
    });
    cards.append(card);
  }
}

// ---------- data loading ----------

// Fetch one provider's summary, snapshots, auth, and series into state and
// re-render. Shared by the full refresh and the per-card refresh.
async function loadProvider(p) {
  try {
    state.summaries.set(p.id, await api(`/api/providers/${p.id}/summary`));
  } catch (e) {
    state.summaries.set(p.id, { state: "error", stderr: String(e) });
  }
  state.checkedAt.set(p.id, Date.now());
  renderDashboard();
  // Snapshots after the summary (which records one).
  try {
    state.snapshots.set(p.id, (await api(`/api/providers/${p.id}/snapshots`)).snapshots);
  } catch { /* keep old */ }
  await Promise.all([
    (async () => {
      if (!p.auth?.required) return;
      try {
        state.auth.set(p.id, (await api(`/api/providers/${p.id}/auth-status`)).state);
      } catch {
        state.auth.set(p.id, "unknown");
      }
    })(),
    ...(p.series || []).map(async (s) => {
      try {
        state.series.set(`${p.id}/${s.id}`, await api(`/api/providers/${p.id}/series/${s.id}`));
      } catch (e) {
        state.series.set(`${p.id}/${s.id}`, { ok: false, stderr: String(e) });
      }
    }),
  ]);
  renderDashboard();
}

async function loadAll() {
  state.summaries.clear();
  state.auth.clear();
  state.series.clear();
  state.loading = true;
  renderDashboard();
  try {
    await Promise.all(summaryProviders().map(loadProvider));
  } finally {
    state.loading = false;
  }
  state.refreshedAt = Date.now();
  updateRefreshedNote();
  renderDashboard(); // clear the loading affordances once every card has resolved
}

// How far along the current load is: how many installed providers have a
// summary in hand vs. the total we're waiting on.
function loadProgress() {
  const total = summaryProviders().length;
  let done = 0;
  for (const p of summaryProviders()) if (state.summaries.has(p.id)) done += 1;
  return { done, total };
}

// Re-fetch a single provider (per-card refresh), with a spinning button.
async function refreshOne(p, btn) {
  btn.classList.add("busy");
  try {
    await loadProvider(p);
    reopenDrawerIfOpen();
  } finally {
    btn.classList.remove("busy");
  }
}

function updateRefreshedNote() {
  const note = $("#refreshed-note");
  if (!state.refreshedAt) return;
  note.hidden = false;
  note.textContent = `updated ${relTime(state.refreshedAt)}`;
}
setInterval(() => {
  updateRefreshedNote();
  $$("[data-rel]").forEach((n) => { n.textContent = relTime(Number(n.dataset.rel)); });
}, 30000);

// ---------- detail drawer ----------

function drawerSection(title) {
  const s = el("div", { class: "drawer-section" });
  s.append(el("h3", {}, title));
  return s;
}

/**
 * "Setup & sign-in" drawer section. Shown when a provider needs a non-secret
 * setup value (rendered as an in-app form utiman runs itself) or an
 * interactive login (guidance + Open-in-Terminal; credentials never touch the
 * app). Returns null when there's nothing to set up.
 */
function renderSetupSection(p) {
  const inputs = p.setup || [];
  const needsAuth = p.auth?.required;
  if (!inputs.length && !needsAuth) return null;

  const sec = drawerSection("Setup & sign-in");
  const authState = effectiveAuth(p);
  if (needsAuth) {
    // Only show a definite state — "unknown"/loading (null) renders nothing, so
    // a pending or failed auth-status check isn't mistaken for a confirmed
    // sign-out (mirrors renderCards). "expired" comes from the fetch itself
    // failing back to re-auth, so it's a definite, actionable state.
    if (authState === "authenticated") {
      sec.append(statusLine("good", "i-check", "Signed in"));
    } else if (authState === "expired") {
      sec.append(statusLine("warn", "i-clock", "Session expired — sign in again"));
    } else if (authState === "unauthenticated") {
      sec.append(statusLine("warn", "i-x", "Not signed in"));
    }
  }

  // Non-secret setup inputs: a form utiman applies by running the CLI.
  for (const input of inputs) {
    const form = el("div", { class: "setup-form" });
    form.append(el("label", { class: "setup-label" }, input.name));
    if (input.description) form.append(el("div", { class: "entry-meta" }, input.description));
    const row = el("div", { class: "row" });
    const field = el("input", { type: "text", class: "setup-input", placeholder: input.placeholder || "" });
    const save = el("button", { class: "small primary" }, "Save");
    const msg = el("span", { class: "setup-msg" });
    save.addEventListener("click", async () => {
      const value = field.value.trim();
      if (!value) { field.focus(); return; }
      save.disabled = true;
      msg.textContent = "…";
      msg.className = "setup-msg";
      try {
        const r = await api(`/api/providers/${p.id}/setup/${input.id}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ value }),
        });
        if (r.ok) {
          msg.textContent = "✓ saved";
          msg.className = "setup-msg ok";
          toast(`${p.name}: ${input.name} saved`);
          refresh();
        } else {
          msg.textContent = (r.stderr || "failed").split("\n")[0].replace(/^error:\s*/, "");
          msg.className = "setup-msg err";
        }
      } catch (e) {
        msg.textContent = String(e.message || e);
        msg.className = "setup-msg err";
      }
      save.disabled = false;
    });
    field.addEventListener("keydown", (ev) => { if (ev.key === "Enter") save.click(); });
    row.append(field, save, msg);
    form.append(row);
    sec.append(form);
  }

  // Interactive login: numbered steps (if any) + the command + Open-in-Terminal.
  // Show once auth is resolved and not usable ("unauthenticated", "unknown", or
  // "expired"); skip while still loading (null) so it doesn't flash.
  if (needsAuth && authState !== null && authState !== "authenticated") {
    const cmd = p.auth["login-command"];
    const steps = p.auth["login-steps"] || [];
    if (steps.length) {
      const ol = el("ol", { class: "login-steps" });
      for (const step of steps) ol.append(el("li", {}, stepWithCode(step)));
      sec.append(ol);
    }
    if (cmd) {
      const cmdRow = el("div", { class: "row" });
      cmdRow.append(el("code", {}, cmd));
      if (p.detection.installed) {
        const tb = loginTerminalButton(p, "Open in Terminal");
        if (tb) cmdRow.append(tb);
      }
      sec.append(cmdRow);
    }
    sec.append(el("div", { class: "entry-meta" },
      "utiman never sees your password — sign-in happens in your terminal, and the CLI keeps the credential in your OS keychain."));
  }
  return sec;
}

// Render a guidance step, turning `backtick spans` into inline <code>.
function stepWithCode(text) {
  const frag = document.createDocumentFragment();
  text.split(/`([^`]+)`/).forEach((part, i) => {
    frag.append(i % 2 ? el("code", {}, part) : document.createTextNode(part));
  });
  return frag;
}

let drawerOpenFor = null;

function openDrawer(p) {
  if (drawerOpenFor === p.id) return;
  drawerOpenFor = p.id;
  $("#drawer-title").textContent = p.name;
  $("#drawer-kind").textContent = p.kind;
  const b = $("#drawer-badge");
  b.replaceChildren(icon((KIND[p.kind] || KIND.other).icon));
  b.style.setProperty("--kind-c", (KIND[p.kind] || KIND.other).hue);
  const body = $("#drawer-body");
  body.replaceChildren();
  $("#drawer").hidden = false;
  $("#drawer-backdrop").hidden = false;

  const setupSec = renderSetupSection(p);
  if (setupSec) body.append(setupSec);

  // Pay: hand off to the official payment page.
  const drawerPay = payButton(p, "small pay-btn");
  if (drawerPay) {
    const paySec = drawerSection("Pay");
    const note = el("div", { class: "entry-meta" },
      "Opens the provider's official payment page — utiman never sees your card.");
    paySec.append(drawerPay, note);
    body.append(paySec);
  }

  // Provider-reported series, each with insight chips + chart.
  for (const s of p.series || []) {
    const sec = drawerSection(s.name);
    body.append(sec);
    const render = (r) => {
      if (!r.ok) {
        const fail = el("div", { class: "card-status crit" });
        fail.append(icon("i-x"), "Couldn't fetch ");
        const d = el("details");
        d.append(el("summary", {}, "Details"));
        const pre = el("pre");
        pre.textContent = r.stderr || "(no stderr)";
        d.append(pre);
        sec.append(fail, d);
        return;
      }
      if (r.points.length >= 2) sec.append(insightChips(seriesStats(r.points), r.unit));
      sec.append(renderChart({
        name: s.name, unit: r.unit, chart: r.chart, points: r.points, showTitle: false,
      }));
    };
    const cached = state.series.get(`${p.id}/${s.id}`);
    if (cached) {
      render(cached);
    } else {
      const holder = el("div", { class: "chart-box" }, "Loading…");
      sec.append(holder);
      api(`/api/providers/${p.id}/series/${s.id}`)
        .then((r) => { state.series.set(`${p.id}/${s.id}`, r); holder.remove(); render(r); })
        .catch((e) => { holder.textContent = String(e); });
    }
  }

  // Balance trend from utiman's own snapshots.
  const trend = drawerSection("Balance history");
  const snaps = state.snapshots.get(p.id) || [];
  if (snaps.length >= 2) {
    trend.append(renderChart({
      name: "Balance over time (recorded locally at each refresh)",
      unit: "usd",
      chart: "line",
      order: "chronological",
      points: snaps.map((s) => ({
        label: new Date(s.ts * 1000).toLocaleDateString(),
        value: s.balance,
      })),
    }));
  } else {
    trend.append(el("p", { class: "sub" },
      "Not much history yet — utiman records a snapshot at every successful refresh, so this chart builds itself over time."));
  }
  body.append(trend);

  if ((p.documents || []).length) {
    const docs = drawerSection("Documents");
    for (const d of p.documents) {
      const a = el("a", {
        class: "doc-link",
        href: `/api/providers/${p.id}/doc/${d.id}`,
        download: d.filename,
      });
      a.append(icon("i-download"), d.name);
      // The CLI can take up to ~2 minutes to produce a PDF; a bare link gives
      // no feedback. Fetch it with a spinner + toast and trigger the save.
      a.addEventListener("click", (ev) => {
        ev.preventDefault();
        downloadDocument(p, d, a);
      });
      docs.append(a);
    }
    body.append(docs);
  }

  if (p.operations.length) {
    const ops = drawerSection("Commands");
    const row = el("div", { class: "entry-actions" });
    for (const op of p.operations) {
      row.append(el("button", { class: "small", onclick: () => runOp(p, op) }, op.name));
    }
    ops.append(row);
    body.append(ops);
  }
}

function hideDrawer() {
  drawerOpenFor = null;
  $("#drawer").hidden = true;
  $("#drawer-backdrop").hidden = true;
}

function closeDrawer() {
  if (location.hash.startsWith("#/p/")) location.hash = "#/dashboard";
  else hideDrawer();
}

// ---------- operations (structured output) ----------

async function runOp(p, op) {
  const modal = $("#output-modal");
  $("#modal-title").textContent = `${p.name} — ${op.name}`;
  const body = $("#modal-body");
  body.replaceChildren("Running…");
  modal.showModal();
  try {
    const r = await api(`/api/providers/${p.id}/op/${op.id}`, { method: "POST" });
    body.replaceChildren();
    if (r.ok) {
      renderOutput(body, r.stdout);
    } else {
      const pre = el("pre");
      pre.textContent = `exit ${r.status ?? "?"}${r.timed_out ? " (timed out)" : ""}\n\n${r.stderr || r.stdout}`;
      body.append(pre);
    }
  } catch (e) {
    body.replaceChildren(String(e));
  }
}

/** Render CLI output as data, not a JSON dump: arrays of records become
 * tables, objects become key/value grids, pipe-tables are parsed, and a Raw
 * toggle always offers the exact bytes. */
function renderOutput(container, text) {
  const raw = el("pre");
  raw.textContent = text || "(no output)";
  raw.hidden = true;

  let structured = null;
  try {
    structured = structureJson(JSON.parse(text));
  } catch {
    structured = structureText(text);
  }
  if (!structured) {
    raw.hidden = false;
    container.append(raw);
    return;
  }
  const toggle = el("button", { class: "small" }, "Raw");
  toggle.addEventListener("click", () => {
    const showRaw = raw.hidden;
    raw.hidden = !showRaw;
    structured.hidden = showRaw;
    toggle.textContent = showRaw ? "Formatted" : "Raw";
  });
  const bar = el("div", { class: "row", style: "justify-content:flex-end;margin:0 0 8px" });
  bar.append(toggle);
  container.append(bar, structured, raw);
}

function cellText(v) {
  if (v == null) return "";
  if (typeof v === "object") {
    if (typeof v.cents === "number" && Object.keys(v).length === 1) return usd.format(v.cents / 100);
    return JSON.stringify(v);
  }
  if (typeof v === "number") return v.toLocaleString("en-US", { maximumFractionDigits: 2 });
  return String(v);
}

function recordsTable(arr) {
  const cols = [];
  for (const row of arr) {
    for (const k of Object.keys(row)) if (!cols.includes(k)) cols.push(k);
  }
  const table = el("table", { class: "data" });
  const thead = el("thead");
  const hr = el("tr");
  for (const c of cols) hr.append(el("th", {}, c.replaceAll("_", " ")));
  thead.append(hr);
  const tbody = el("tbody");
  for (const row of arr) {
    const tr = el("tr");
    for (const c of cols) {
      const v = row[c];
      const td = el("td", {}, cellText(v));
      if (typeof v === "number" || (v && typeof v === "object" && "cents" in v)) td.className = "num";
      tr.append(td);
    }
    tbody.append(tr);
  }
  table.append(thead, tbody);
  const wrap = el("div", { class: "chart-table" });
  wrap.append(table);
  return wrap;
}

function structureJson(v) {
  if (Array.isArray(v) && v.length && v.every((x) => x && typeof x === "object" && !Array.isArray(x))) {
    return recordsTable(v);
  }
  if (v && typeof v === "object" && !Array.isArray(v)) {
    // Objects like {account, payments: [...]}: table for the array part,
    // key/value grid for the scalars.
    const box = el("div");
    const kv = el("dl", { class: "kv" });
    let hasScalars = false;
    for (const [k, val] of Object.entries(v)) {
      if (Array.isArray(val) && val.length && val.every((x) => x && typeof x === "object")) continue;
      kv.append(el("dt", {}, k.replaceAll("_", " ")), el("dd", {}, cellText(val)));
      hasScalars = true;
    }
    if (hasScalars) box.append(kv);
    for (const [k, val] of Object.entries(v)) {
      if (Array.isArray(val) && val.length && val.every((x) => x && typeof x === "object")) {
        box.append(el("h3", { style: "margin:10px 0 6px" }, k.replaceAll("_", " ")), recordsTable(val));
      }
    }
    return box.childNodes.length ? box : null;
  }
  return null;
}

function structureText(text) {
  const lines = (text || "").split("\n").filter((l) => l.includes("|"));
  if (lines.length >= 2) {
    const headers = lines[0].split("|").map((s) => s.trim());
    const records = lines.slice(1).map((l) => {
      const cells = l.split("|").map((s) => s.trim());
      return Object.fromEntries(headers.map((h, i) => [h, cells[i] ?? ""]));
    });
    return recordsTable(records);
  }
  return null;
}

// ---------- catalog ----------

function renderCatalog() {
  const box = $("#catalog");
  box.replaceChildren();
  for (const p of state.providers) {
    const entry = el("article", { class: "entry" });
    const top = el("div", { class: "entry-top" });
    const title = el("div", { class: "card-title" });
    const name = el("div", { class: "card-name" }, p.name);
    title.append(name, el("div", { class: "card-sub" }, `binary: ${p.binary}${p.detection.version ? ` · ${p.detection.version}` : ""}`));
    top.append(badge(p.kind), title);
    top.append(
      p.detection.installed
        ? pill("good", "i-check", "installed")
        : pill("", null, "not installed")
    );
    if (p.source === "user") top.append(pill("", null, "user"));
    entry.append(top);
    if (p.description) entry.append(el("div", { class: "entry-desc" }, p.description));

    if (p.auth?.required && p.auth["login-command"]) {
      const meta = el("div", { class: "entry-meta" });
      meta.append("login: ", el("code", {}, p.auth["login-command"]));
      entry.append(meta);
    } else if (p["setup-command"]) {
      const meta = el("div", { class: "entry-meta" });
      meta.append("setup: ", el("code", {}, p["setup-command"]));
      entry.append(meta);
    }

    const actions = el("div", { class: "entry-actions" });
    if (p.install) {
      const selfUpdate = p.detection.installed && p.install["self-update-args"];
      const btn = el("button", { class: "small", onclick: () => install(p, entry, btn) },
        p.installing ? "Installing…" : selfUpdate ? "Self-update" : p.detection.installed ? "Reinstall" : "Install");
      if (p.installing) btn.disabled = true;
      actions.append(btn);
      if (p.detection.installed && p.install["update-check-args"]) {
        const chk = el("button", { class: "small", onclick: () => checkUpdate(p, entry, chk) }, "Check for update");
        actions.append(chk);
      }
    }
    if (p.detection.installed && p.auth?.["login-command"]) {
      const tb = loginTerminalButton(p, "Login in Terminal");
      if (tb) actions.append(tb);
    }
    const gh = el("a", { class: "btn small", href: p.repo, target: "_blank", rel: "noreferrer" });
    gh.append(icon("i-github"), "GitHub");
    actions.append(gh);
    if (p.source === "user") {
      actions.append(el("button", { class: "small", onclick: () => removeProvider(p) }, "Remove"));
    }
    entry.append(actions);
    box.append(entry);
  }
}

// The Open-in-Terminal flow uses macOS `osascript`, so it's only offered on
// macOS. Off-mac, callers show the copyable command instead (null → skip).
// Fetch a provider document (the CLI can run ~120s) with visible progress,
// then trigger a browser save. Falls back to nothing destructive on error.
async function downloadDocument(p, d, anchor) {
  // Re-entrancy guard: these run the CLI for up to ~120s, so ignore repeat
  // clicks while one is already in flight (else duplicate fetches + saves).
  if (anchor.classList.contains("loading")) return;
  anchor.classList.add("loading");
  const spin = icon("i-refresh");
  spin.classList.add("spin");
  anchor.prepend(spin);
  toast(`${p.name}: preparing ${d.name}…`);
  try {
    const res = await fetch(`/api/providers/${p.id}/doc/${d.id}`);
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new Error(body.error || `${res.status} ${res.statusText}`);
    }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const tmp = el("a", { href: url, download: d.filename });
    document.body.append(tmp);
    tmp.click();
    tmp.remove();
    URL.revokeObjectURL(url);
    toast(`${p.name}: ${d.filename} downloaded`);
  } catch (e) {
    toast(String(e.message || e), "err");
  } finally {
    spin.remove();
    anchor.classList.remove("loading");
  }
}

function loginTerminalButton(p, label = "Open login in Terminal") {
  if (!isMac()) return null;
  const btn = el("button", { class: "small" });
  btn.append(icon("i-terminal"), label);
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    try {
      await api(`/api/providers/${p.id}/login-terminal`, { method: "POST" });
      toast(`Terminal opened — refresh when you're signed in.`);
    } catch (e) {
      toast(String(e.message || e), "err");
    }
    btn.disabled = false;
  });
  return btn;
}

/** A "Pay bill" button that hands off to the provider's official payment page:
 * a link for the `url` form, or a POST that runs the CLI for `open-args`.
 * utiman never sees card data. Returns null when the provider has no pay
 * config or isn't installed. */
// Record that a payment was just initiated, with the balance at the time, so
// the card can show a "payment pending" state until the balance drops.
function markPaymentInitiated(p) {
  const balance = state.summaries.get(p.id)?.balance ?? null;
  api(`/api/providers/${p.id}/payment-initiated`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ balance }),
  }).catch(() => {});
}

function payButton(p, cls = "small pay-btn") {
  const pay = p.pay;
  if (!pay || !p.detection.installed) return null;
  const label = pay.label || "Pay bill";
  if (pay.url) {
    const a = el("a", { class: `btn ${cls}`, href: pay.url, target: "_blank", rel: "noreferrer" });
    a.append(icon("i-card"), label);
    a.addEventListener("click", () => markPaymentInitiated(p));
    return a;
  }
  const btn = el("button", { class: cls });
  btn.append(icon("i-card"), label);
  btn.addEventListener("click", async (ev) => {
    ev.stopPropagation();
    btn.disabled = true;
    try {
      const r = await api(`/api/providers/${p.id}/pay`, { method: "POST" });
      if (r.ok) markPaymentInitiated(p);
      toast(r.ok ? `${p.name}: opened payment page` : (r.stderr || "couldn't open pay page"),
        r.ok ? "ok" : "err");
    } catch (e) {
      toast(String(e.message || e), "err");
    }
    btn.disabled = false;
  });
  return btn;
}

async function install(p, entry, btn) {
  btn.disabled = true;
  btn.textContent = "Installing…";
  const log = el("pre", { class: "install-log" });
  log.textContent = "starting…";
  entry.append(log);
  try {
    const { task } = await api(`/api/providers/${p.id}/install`, { method: "POST" });
    for (;;) {
      const st = await api(`/api/install/${task}`);
      log.textContent = st.log;
      log.scrollTop = log.scrollHeight;
      if (st.state !== "running") {
        toast(st.state === "succeeded" ? `${p.name}: install finished` : `${p.name}: install failed`,
          st.state === "succeeded" ? "ok" : "err");
        break;
      }
      await new Promise((r) => setTimeout(r, 1000));
    }
  } catch (e) {
    toast(String(e.message || e), "err");
  }
  await refresh();
}

async function checkUpdate(p, entry, btn) {
  btn.disabled = true;
  const prior = entry.querySelector(".update-note");
  if (prior) prior.remove();
  const note = el("div", { class: "entry-meta update-note" }, "checking…");
  entry.append(note);
  try {
    const r = await api(`/api/providers/${p.id}/update-check`, { method: "POST" });
    note.textContent = (r.ok ? r.stdout : r.stderr || r.stdout).trim() || "(no output)";
  } catch (e) {
    note.textContent = String(e);
  }
  btn.disabled = false;
}

async function removeProvider(p) {
  if (!confirm(`Remove ${p.name} from your registered providers?`)) return;
  try {
    await api(`/api/providers/${p.id}`, { method: "DELETE" });
    toast(`Removed ${p.name}`);
    await refresh();
  } catch (e) {
    toast(String(e.message || e), "err");
  }
}

// ---------- register ----------

async function register() {
  try {
    const r = await api("/api/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        toml: $("#register-toml").value,
        overwrite: $("#register-overwrite").checked,
      }),
    });
    toast(`Registered ${r.id}`);
    await refresh();
    location.hash = "#/catalog";
  } catch (e) {
    toast(String(e.message || e), "err");
  }
}

// ---------- theme ----------

function currentTheme() {
  return localStorage.getItem("utiman-theme") || "system";
}

// Reflect the active theme on the toggle: icon (auto/sun/moon), tooltip, aria.
function updateThemeToggle() {
  const cur = currentTheme();
  const glyph = { system: "i-theme", light: "i-sun", dark: "i-moon" }[cur];
  const btn = $("#theme-toggle");
  btn.replaceChildren(icon(glyph));
  const label = `Theme: ${cur} (click to change)`;
  btn.title = label;
  btn.setAttribute("aria-label", label);
}

function cycleTheme() {
  const cur = currentTheme();
  const next = cur === "system" ? "light" : cur === "light" ? "dark" : "system";
  if (next === "system") {
    localStorage.removeItem("utiman-theme");
    delete document.documentElement.dataset.theme;
  } else {
    localStorage.setItem("utiman-theme", next);
    document.documentElement.dataset.theme = next;
  }
  updateThemeToggle();
  toast(`Theme: ${next}`);
}

// ---------- boot ----------

let refreshing = false;
async function refresh() {
  // Non-re-entrant: a second refresh (double-click, or a click during the boot
  // load) would run its own loadAll with its own state.loading try/finally, so
  // whichever finished first would flip loading off and hide the load bar/hero
  // shimmer while the other's cards still showed "Checking…". One load at a time.
  if (refreshing) return;
  refreshing = true;
  const btn = $("#refresh");
  btn.classList.add("busy");
  btn.disabled = true;
  state.loading = true; // so the first paint already reads as "loading"
  try {
    const { providers, host } = await api("/api/providers");
    state.providers = providers;
    state.os = host?.os ?? null;
    renderCatalog();
    renderDashboard();
    await loadAll();
    // Rebuild an open drawer against the fresh data — otherwise a saved setup
    // value or a completed sign-in leaves its Setup & sign-in section stale
    // (openDrawer's same-id guard skips a rebuild while it stays open).
    reopenDrawerIfOpen();
  } finally {
    btn.classList.remove("busy");
    btn.disabled = false;
    refreshing = false;
  }
}

function reopenDrawerIfOpen() {
  if (!drawerOpenFor) return;
  const p = state.providers.find((x) => x.id === drawerOpenFor);
  if (!p) {
    hideDrawer();
    return;
  }
  drawerOpenFor = null; // bypass openDrawer's same-id early return
  openDrawer(p);
}

$("#refresh").addEventListener("click", refresh);
$("#register-btn").addEventListener("click", register);
$("#theme-toggle").addEventListener("click", cycleTheme);
updateThemeToggle();
$("#modal-close").addEventListener("click", () => $("#output-modal").close());
$("#drawer-close").addEventListener("click", closeDrawer);
$("#drawer-backdrop").addEventListener("click", closeDrawer);
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape" && !$("#drawer").hidden) closeDrawer();
});

// Reveal the target section and paint the loading shell *before* the (slow)
// data fetch — otherwise the whole dashboard stays hidden until every summary
// resolves, so the user stares at a blank page with no sign of progress.
state.loading = true;
route();
renderDashboard();
refresh()
  .then(route) // re-run once data is in to resolve a #/p/<id> drawer deep link
  .catch((e) => {
    $("#cards").textContent = `Failed to load: ${e}`;
    route();
  });
