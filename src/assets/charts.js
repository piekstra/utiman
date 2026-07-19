// Dependency-free SVG charts. Single-series only (per design: one measure,
// one chart — no dual axes, no legends for a lone series). Mark styling and
// interaction follow the dashboard's chart conventions: thin marks with
// rounded data-ends, hairline grid, hover tooltips, and a table view so the
// data is never color- or hover-only.

const SVG_NS = "http://www.w3.org/2000/svg";

function svgEl(tag, attrs = {}) {
  const n = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  return n;
}

function fmtVal(v, unit) {
  if (unit === "usd") {
    return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(v);
  }
  const n = Math.abs(v) >= 100 ? Math.round(v).toLocaleString("en-US")
    : v.toLocaleString("en-US", { maximumFractionDigits: 2 });
  return unit ? `${n} ${unit}` : n;
}

// "Nice" round max for the y scale so gridline labels aren't ragged.
function niceMax(max) {
  if (max <= 0) return 1;
  const pow = 10 ** Math.floor(Math.log10(max));
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (max <= m * pow) return m * pow;
  }
  return 10 * pow;
}

/**
 * Render a single-series chart with a table-view toggle.
 * spec: { points: [{label, value}], unit, chart: "bar"|"line", name }
 * Returns a container element.
 */
function renderChart(spec) {
  const box = document.createElement("div");
  box.className = "chart-box";

  const head = document.createElement("div");
  head.className = "chart-head";
  const title = document.createElement("strong");
  // Callers inside a titled section pass showTitle: false to avoid repeating it.
  title.textContent = spec.showTitle === false ? "" : spec.name;
  const toggle = document.createElement("button");
  toggle.className = "small";
  toggle.textContent = "Table";
  head.append(title, toggle);
  box.append(head);

  if (!spec.points.length) {
    const empty = document.createElement("p");
    empty.className = "sub";
    empty.textContent = "No data points.";
    box.append(empty);
    toggle.hidden = true;
    return box;
  }

  const body = document.createElement("div");
  body.className = "chart-body";
  body.append(plot(spec, body));
  // Table rows read top-down as most-recent-first.
  const tableSpec = spec.order === "chronological"
    ? { ...spec, points: [...spec.points].reverse() }
    : spec;
  box.append(body, dataTable(tableSpec));
  box.lastChild.hidden = true;

  toggle.addEventListener("click", () => {
    const showTable = box.lastChild.hidden;
    box.lastChild.hidden = !showTable;
    body.hidden = showTable;
    toggle.textContent = showTable ? "Chart" : "Table";
  });
  return box;
}

function dataTable(spec) {
  const wrap = document.createElement("div");
  wrap.className = "chart-table";
  const table = document.createElement("table");
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const h of ["Period", spec.unit === "usd" ? "Amount" : (spec.unit || "Value")]) {
    const th = document.createElement("th");
    th.textContent = h;
    hr.append(th);
  }
  thead.append(hr);
  const tbody = document.createElement("tbody");
  for (const p of spec.points) {
    const tr = document.createElement("tr");
    const td1 = document.createElement("td");
    td1.textContent = p.label;
    const td2 = document.createElement("td");
    td2.className = "num";
    td2.textContent = fmtVal(p.value, spec.unit);
    tr.append(td1, td2);
    tbody.append(tr);
  }
  table.append(thead, tbody);
  wrap.append(table);
  return wrap;
}

function plot(spec, container) {
  // Chronological left→right. CLIs usually emit newest-first (the default);
  // pass order: "chronological" for data that is already oldest-first.
  const points = spec.order === "chronological"
    ? [...spec.points]
    : [...spec.points].reverse();
  const W = 640, H = 220;
  const M = { top: 12, right: 12, bottom: 26, left: 56 };
  const iw = W - M.left - M.right;
  const ih = H - M.top - M.bottom;
  // A series that's all non-positive (e.g. account credits, which parse to
  // negatives) would scale to an invisible chart. Scale by peak magnitude so
  // those bars/line are visible; signed values still show in tooltip + table,
  // and the axis labels carry the sign.
  const rawMax = Math.max(...points.map((p) => p.value));
  const allNonPositive = rawMax <= 0;
  const domain = niceMax(allNonPositive ? Math.max(...points.map((p) => Math.abs(p.value))) : rawMax);
  // Height fraction [0..1]: magnitude for an all-credit series, else clamp a
  // stray negative to the baseline.
  const frac = (v) => (allNonPositive ? Math.abs(v) : Math.max(0, v)) / domain;

  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, class: "chart", role: "img" });
  svg.setAttribute("aria-label", spec.name);

  // Hairline gridlines + y labels at 0/half/max.
  for (const g of [0, 0.5, 1]) {
    const y = M.top + ih - g * ih;
    svg.append(svgEl("line", {
      x1: M.left, x2: M.left + iw, y1: y, y2: y,
      class: g === 0 ? "axis-line" : "grid-line",
    }));
    const lbl = svgEl("text", { x: M.left - 6, y: y + 4, class: "tick", "text-anchor": "end" });
    // `|| 0` collapses the -0 that `-1 * domain * 0` yields at the baseline,
    // so an all-credit chart's zero gridline reads "$0.00", not "-$0.00".
    const labelVal = (allNonPositive ? -1 : 1) * domain * g || 0;
    lbl.textContent = fmtVal(labelVal, spec.unit === "usd" ? "usd" : undefined);
    svg.append(lbl);
  }

  const tooltip = document.createElement("div");
  tooltip.className = "chart-tip";
  tooltip.hidden = true;
  container.append(tooltip);

  const showTip = (evt, p) => {
    tooltip.hidden = false;
    tooltip.textContent = `${p.label}: ${fmtVal(p.value, spec.unit)}`;
    const r = container.getBoundingClientRect();
    tooltip.style.left = `${Math.min(evt.clientX - r.left + 12, r.width - 140)}px`;
    tooltip.style.top = `${evt.clientY - r.top - 30}px`;
  };
  const hideTip = () => { tooltip.hidden = true; };

  const n = points.length;
  const step = iw / n;

  if (spec.chart === "line") {
    const xy = points.map((p, i) => [
      M.left + step * (i + 0.5),
      M.top + ih - frac(p.value) * ih,
    ]);
    const d = xy.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
    const base = M.top + ih;
    svg.append(svgEl("path", {
      d: `${d} L${xy[xy.length - 1][0].toFixed(1)},${base} L${xy[0][0].toFixed(1)},${base} Z`,
      class: "series-area",
    }));
    svg.append(svgEl("path", { d, class: "series-line" }));
    xy.forEach(([x, y], i) => {
      const dot = svgEl("circle", { cx: x, cy: y, r: 4, class: "series-dot" });
      const hit = svgEl("rect", {
        x: M.left + step * i, y: M.top, width: step, height: ih,
        fill: "transparent",
      });
      hit.addEventListener("mousemove", (e) => { showTip(e, points[i]); dot.classList.add("hot"); });
      hit.addEventListener("mouseleave", () => { hideTip(); dot.classList.remove("hot"); });
      svg.append(dot, hit);
    });
  } else {
    // Bars: thin, rounded top (the data end), 2px gap on each side.
    const barW = Math.max(3, Math.min(28, step - 4));
    points.forEach((p, i) => {
      const h = Math.max(1, frac(p.value) * ih);
      const x = M.left + step * i + (step - barW) / 2;
      const y = M.top + ih - h;
      const r = Math.min(4, barW / 2, h);
      const d = `M${x},${y + h} V${y + r} Q${x},${y} ${x + r},${y} H${x + barW - r} Q${x + barW},${y} ${x + barW},${y + r} V${y + h} Z`;
      const bar = svgEl("path", { d, class: "series-fill" });
      const hit = svgEl("rect", {
        x: M.left + step * i, y: M.top, width: step, height: ih,
        fill: "transparent",
      });
      hit.addEventListener("mousemove", (e) => { showTip(e, p); bar.classList.add("hot"); });
      hit.addEventListener("mouseleave", () => { hideTip(); bar.classList.remove("hot"); });
      svg.append(bar, hit);
    });
  }

  // Sparse x labels: at most ~6, always first and last.
  const every = Math.max(1, Math.ceil(n / 6));
  points.forEach((p, i) => {
    if (i % every !== 0 && i !== n - 1) return;
    const t = svgEl("text", {
      x: M.left + step * (i + 0.5),
      y: H - 8,
      class: "tick",
      "text-anchor": "middle",
    });
    t.textContent = p.label.length > 11 ? p.label.slice(0, 10) + "…" : p.label;
    svg.append(t);
  });

  return svg;
}

const MONTHS3 = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/** Parse a period label into a *local* Date (or null). `Jun 2026` / `June 2026`
 * and ISO `2026-06`/`2026-06-26` are built from components so they land on local
 * midnight — `Date.parse("2026-06-26")` would be UTC midnight, which local
 * getMonth()/getFullYear() can read back a day (and month) early in zones west
 * of UTC, mis-bucketing a 1st-of-month bill. Other shapes fall back to parse. */
function labelToDate(label) {
  const s = String(label || "");
  if (!s) return null;
  const named = s.match(/([A-Za-z]{3,})\s+(\d{4})/);
  if (named) {
    const mi = MONTHS3.indexOf(named[1].slice(0, 3).toLowerCase());
    if (mi >= 0) return new Date(Number(named[2]), mi, 1);
  }
  const iso = s.match(/(\d{4})-(\d{1,2})(?:-(\d{1,2}))?/);
  if (iso) return new Date(Number(iso[1]), Number(iso[2]) - 1, Number(iso[3] || 1));
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : new Date(t);
}

/** Build a comparison {delta, pct, label, seasonal} between latest and a
 * reference point (prior period, or same period last year when found). */
function compareTo(latest, ref, seasonal) {
  if (!ref) return null;
  const delta = latest.value - ref.value;
  return {
    delta,
    pct: ref.value ? (delta / Math.abs(ref.value)) * 100 : null,
    label: ref.label,
    seasonal,
  };
}

/**
 * Descriptive stats over series points (newest-first, as the API returns
 * them). Everything here is directly computed from the data — no guesses.
 * `compare` prefers the same period last year (the right baseline for
 * seasonal utilities) and falls back to the prior period.
 */
function seriesStats(points) {
  const latest = points[0];
  const prev = points[1] || null;
  const values = points.map((p) => p.value);
  const avg = values.reduce((a, b) => a + b, 0) / values.length;
  const peak = points.reduce((m, p) => (p.value > m.value ? p : m), points[0]);

  // Same period last year: the point closest to ~365 days before latest,
  // accepted only within ~45 days of that mark.
  let seasonalRef = null;
  const latestDate = labelToDate(latest.label);
  if (latestDate) {
    const target = latestDate.getTime() - 365.25 * 864e5;
    let best = null;
    let bestDiff = 46 * 864e5;
    for (let i = 1; i < points.length; i++) {
      const d = labelToDate(points[i].label);
      if (!d) continue;
      const diff = Math.abs(d.getTime() - target);
      if (diff < bestDiff) { bestDiff = diff; best = points[i]; }
    }
    seasonalRef = best;
  }
  // Prefer the seasonal comparison only when it yields a usable percentage —
  // a zero-valued same-period-last-year reference makes pct null, and falling
  // back on object truthiness alone would then suppress a valid prior-period
  // delta. Fall back on pct == null instead.
  const seasonalCmp = compareTo(latest, seasonalRef, true);
  const compare = seasonalCmp && seasonalCmp.pct != null
    ? seasonalCmp
    : compareTo(latest, prev, false);

  let streak = 0;
  let dir = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    const d = Math.sign(points[i].value - points[i + 1].value);
    if (d === 0) break;
    if (dir === 0) dir = d;
    if (d !== dir) break;
    streak++;
  }
  return { latest, prev, compare, avg, peak, streak, dir, count: points.length };
}

/** Stat-chip row for a series: latest (with delta), average, peak, streak. */
function insightChips(stats, unit) {
  const row = document.createElement("div");
  row.className = "insight-chips";
  const chip = (label, value, subNode) => {
    const c = document.createElement("div");
    c.className = "insight-chip";
    const v = document.createElement("strong");
    v.textContent = value;
    c.append(label, v);
    if (subNode) c.append(subNode);
    row.append(c);
  };

  let deltaNode = null;
  const c = stats.compare;
  if (c && c.pct != null) {
    deltaNode = document.createElement("span");
    deltaNode.className = c.delta >= 0 ? "delta-up" : "delta-down";
    const vs = c.seasonal ? `vs ${c.label} (last year)` : `vs ${c.label}`;
    deltaNode.textContent = `${c.delta >= 0 ? "▲" : "▼"} ${Math.abs(c.pct).toFixed(1)}% ${vs}`;
  }
  chip(`Latest (${stats.latest.label})`, fmtVal(stats.latest.value, unit), deltaNode);
  chip(`Average of ${stats.count}`, fmtVal(stats.avg, unit));
  chip(`Peak (${stats.peak.label})`, fmtVal(stats.peak.value, unit));
  if (stats.streak >= 3) {
    chip("Trend", `${stats.streak} periods ${stats.dir > 0 ? "rising" : "falling"}`);
  }
  return row;
}

/** Small inline balance sparkline for a card: de-emphasized line, accent end dot. */
function renderSparkline(snapshots) {
  const pts = snapshots.slice(-12);
  if (pts.length < 2) return null;
  const W = 110, H = 28, pad = 3;
  const min = Math.min(...pts.map((s) => s.balance));
  const max = Math.max(...pts.map((s) => s.balance));
  const span = max - min || 1;
  const xy = pts.map((s, i) => [
    pad + (i / (pts.length - 1)) * (W - 2 * pad),
    H - pad - ((s.balance - min) / span) * (H - 2 * pad),
  ]);
  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, class: "sparkline", role: "img" });
  svg.setAttribute("aria-label", "balance trend");
  const d = xy.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  svg.append(svgEl("path", { d, class: "spark-line" }));
  const [lx, ly] = xy[xy.length - 1];
  svg.append(svgEl("circle", { cx: lx, cy: ly, r: 2.5, class: "spark-dot" }));
  return svg;
}

// ---------- cross-provider spend rollup ----------

// Provider kind → chart hue, reusing the same CSS custom props as the badges so
// the rollup's stacked segments match each provider's identity color.
function kindHue(kind) {
  // Only kinds with a defined --kind-* token; "trash" (and anything unknown)
  // falls back to --kind-other, matching the badge KIND map in app.js. Using
  // an undefined var would resolve to the SVG fill initial value (black).
  const known = ["electric", "water", "sewer", "gas", "internet"];
  return `var(--kind-${known.includes(kind) ? kind : "other"})`;
}

/**
 * Stacked monthly-spend chart across providers.
 *   months:    [{ label, total, segments: [{ id, name, kind, value }] }] oldest→newest
 *   providers: ordered [{ id, name, kind }] — fixed color/legend order
 * Legend is always present (>=2 series); a table view mirrors the data so it's
 * never color- or hover-only.
 */
function renderSpendChart(months, providers) {
  const box = document.createElement("div");
  box.className = "chart-box";

  const head = document.createElement("div");
  head.className = "chart-head";
  head.append(document.createElement("strong"));
  const toggle = document.createElement("button");
  toggle.className = "small";
  toggle.textContent = "Table";
  head.append(toggle);
  box.append(head);

  const legend = document.createElement("div");
  legend.className = "chart-legend";
  for (const p of providers) {
    const item = document.createElement("span");
    item.className = "legend-item";
    const sw = document.createElement("span");
    sw.className = "legend-swatch";
    sw.style.background = kindHue(p.kind);
    item.append(sw, p.name);
    legend.append(item);
  }
  box.append(legend);

  const body = document.createElement("div");
  body.className = "chart-body";
  body.append(spendPlot(months, providers, body));
  box.append(body, spendTable(months, providers));
  box.lastChild.hidden = true;

  toggle.addEventListener("click", () => {
    const showTable = box.lastChild.hidden;
    box.lastChild.hidden = !showTable;
    body.hidden = showTable;
    legend.hidden = showTable;
    toggle.textContent = showTable ? "Chart" : "Table";
  });
  return box;
}

function spendPlot(months, providers, container) {
  const W = 640, H = 240;
  const M = { top: 14, right: 12, bottom: 26, left: 58 };
  const iw = W - M.left - M.right, ih = H - M.top - M.bottom;
  const domain = niceMax(Math.max(1, ...months.map((m) => m.total)));
  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, class: "chart", role: "img" });
  svg.setAttribute("aria-label", "Total monthly spend across providers");

  for (const g of [0, 0.5, 1]) {
    const y = M.top + ih - g * ih;
    svg.append(svgEl("line", {
      x1: M.left, x2: M.left + iw, y1: y, y2: y,
      class: g === 0 ? "axis-line" : "grid-line",
    }));
    const lbl = svgEl("text", { x: M.left - 6, y: y + 4, class: "tick", "text-anchor": "end" });
    lbl.textContent = fmtVal(domain * g, "usd");
    svg.append(lbl);
  }

  const tooltip = document.createElement("div");
  tooltip.className = "chart-tip";
  tooltip.hidden = true;
  container.append(tooltip);

  const n = months.length;
  const step = iw / n;
  const barW = Math.max(6, Math.min(34, step - 8));

  months.forEach((mo, i) => {
    const x = M.left + step * i + (step - barW) / 2;
    // Draw in the providers' fixed order so a color always means the same
    // utility; stack from the baseline up.
    const segs = providers
      .map((p) => mo.segments.find((s) => s.id === p.id))
      .filter((s) => s && s.value > 0);
    let baseline = M.top + ih;
    segs.forEach((seg, si) => {
      const h = (seg.value / domain) * ih;
      const yb = baseline, yt = baseline - h;
      const isTop = si === segs.length - 1;
      const r = isTop ? Math.min(4, barW / 2, h) : 0;
      const d = isTop
        ? `M${x},${yb} V${yt + r} Q${x},${yt} ${x + r},${yt} H${x + barW - r} Q${x + barW},${yt} ${x + barW},${yt + r} V${yb} Z`
        : `M${x},${yb} V${yt} H${x + barW} V${yb} Z`;
      const path = svgEl("path", { d, class: "spend-seg" });
      path.style.fill = kindHue(seg.kind);
      svg.append(path);
      baseline = yt - 2; // 2px surface gap between stacked segments
    });

    const hit = svgEl("rect", {
      x: M.left + step * i, y: M.top, width: step, height: ih, fill: "transparent",
    });
    hit.addEventListener("mousemove", (e) => {
      tooltip.hidden = false;
      const parts = [`${mo.label} — ${fmtVal(mo.total, "usd")}`];
      for (const s of mo.segments) parts.push(`${s.name}: ${fmtVal(s.value, "usd")}`);
      tooltip.textContent = parts.join("   ·   ");
      const rb = container.getBoundingClientRect();
      tooltip.style.left = `${Math.min(e.clientX - rb.left + 12, rb.width - 200)}px`;
      tooltip.style.top = `${e.clientY - rb.top - 30}px`;
    });
    hit.addEventListener("mouseleave", () => { tooltip.hidden = true; });
    svg.append(hit);
  });

  const every = Math.max(1, Math.ceil(n / 6));
  months.forEach((mo, i) => {
    if (i % every !== 0 && i !== n - 1) return;
    const t = svgEl("text", {
      x: M.left + step * (i + 0.5), y: H - 8, class: "tick", "text-anchor": "middle",
    });
    t.textContent = mo.label;
    svg.append(t);
  });
  return svg;
}

function spendTable(months, providers) {
  const wrap = document.createElement("div");
  wrap.className = "chart-table";
  const table = document.createElement("table");
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const h of ["Month", ...providers.map((p) => p.name), "Total"]) {
    const th = document.createElement("th");
    th.textContent = h;
    hr.append(th);
  }
  thead.append(hr);
  const tbody = document.createElement("tbody");
  for (const mo of [...months].reverse()) {
    const tr = document.createElement("tr");
    const td0 = document.createElement("td");
    td0.textContent = mo.label;
    tr.append(td0);
    for (const p of providers) {
      const seg = mo.segments.find((s) => s.id === p.id);
      const td = document.createElement("td");
      td.className = "num";
      td.textContent = seg ? fmtVal(seg.value, "usd") : "—";
      tr.append(td);
    }
    const tot = document.createElement("td");
    tot.className = "num";
    tot.textContent = fmtVal(mo.total, "usd");
    tr.append(tot);
    tbody.append(tr);
  }
  table.append(thead, tbody);
  wrap.append(table);
  return wrap;
}
