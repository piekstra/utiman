// utiman dashboard. Vanilla JS; all data comes from the local API, all
// CLI output is inserted with textContent (never innerHTML) since it is
// arbitrary program output.

const $ = (sel) => document.querySelector(sel);
const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

const state = {
  providers: [],
  summaries: new Map(), // id -> summary response
  auth: new Map(),      // id -> "authenticated" | "unauthenticated" | "unknown"
};

async function api(path, opts) {
  const res = await fetch(path, opts);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || `${res.status} ${res.statusText}`);
  return body;
}

// ---------- cards ----------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const c of children) {
    if (c == null) continue;
    node.append(c);
  }
  return node;
}

function statusLine(cls, icon, text) {
  const s = el("div", { class: `card-status ${cls}` });
  s.append(el("span", { class: "icon", "aria-hidden": "true" }, icon), text);
  return s;
}

function renderCards() {
  const cards = $("#cards");
  cards.replaceChildren();
  const withSummary = state.providers.filter((p) => p.summary);
  for (const p of withSummary) {
    const card = el("div", { class: "card" });
    const top = el("div", { class: "card-top" });
    top.append(
      el("span", { class: "card-name" }, p.name),
      el("span", { class: "kind" }, p.kind)
    );
    card.append(top);

    if (!p.detection.installed) {
      card.append(statusLine("", "○", "CLI not installed — see the catalog below."));
      cards.append(card);
      continue;
    }

    const authState = state.auth.get(p.id);
    if (authState === "authenticated") {
      card.append(statusLine("good", "●", "Signed in"));
    } else if (authState === "unauthenticated") {
      const line = statusLine("warn", "○", "Sign-in needed ");
      if (p.auth?.["login-command"]) line.append(loginTerminalButton(p));
      card.append(line);
    }

    const s = state.summaries.get(p.id);
    if (!s) {
      card.append(el("div", { class: "skeleton" }));
    } else if (s.state === "ok") {
      card.append(
        el("div", { class: "card-value" },
          s.balance == null ? "—" : usd.format(s.balance)),
        el("div", { class: "card-due" },
          s.due_date ? `Due ${s.due_date}` : "No due date reported")
      );
      if (s.raw) {
        const d = el("details");
        d.append(el("summary", {}, "Raw output"));
        const pre = el("pre");
        pre.textContent = pretty(s.raw);
        d.append(pre);
        card.append(d);
      }
    } else {
      // Error state: reserved status color + icon + label, never color alone.
      card.append(statusLine("crit", "✕", s.timed_out ? "Timed out" : "Couldn't fetch"));
      if (s.hint) {
        const hint = el("div", { class: "card-status warn" });
        hint.append(
          el("span", { class: "icon", "aria-hidden": "true" }, "▲"),
          "Try in your terminal: ",
          el("code", {}, s.hint),
          " "
        );
        if (p.auth?.["login-command"] === s.hint) {
          hint.append(loginTerminalButton(p, "Open in Terminal"));
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

    if (p.operations.length) {
      const ops = el("div", { class: "card-ops" });
      for (const op of p.operations) {
        ops.append(
          el("button", { class: "small", onclick: () => runOp(p, op) }, op.name)
        );
      }
      card.append(ops);
    }
    cards.append(card);
  }
  renderHero();
}

function renderHero() {
  const loaded = [...state.summaries.values()].filter(
    (s) => s.state === "ok" && s.balance != null
  );
  const hero = $("#hero");
  if (!loaded.length) { hero.hidden = true; return; }
  hero.hidden = false;
  $("#hero-value").textContent = usd.format(
    loaded.reduce((sum, s) => sum + s.balance, 0)
  );
  $("#hero-sub").textContent = `across ${loaded.length} account${loaded.length === 1 ? "" : "s"}`;
}

function pretty(text) {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

async function loadSummaries() {
  const installed = state.providers.filter((p) => p.summary && p.detection.installed);
  const authed = state.providers.filter(
    (p) => p.auth?.required && p.detection.installed
  );
  state.summaries.clear();
  state.auth.clear();
  renderCards();
  await Promise.all([
    ...installed.map(async (p) => {
      try {
        state.summaries.set(p.id, await api(`/api/providers/${p.id}/summary`));
      } catch (e) {
        state.summaries.set(p.id, { state: "error", stderr: String(e) });
      }
      renderCards();
    }),
    ...authed.map(async (p) => {
      try {
        const r = await api(`/api/providers/${p.id}/auth-status`);
        state.auth.set(p.id, r.state);
      } catch {
        state.auth.set(p.id, "unknown");
      }
      renderCards();
    }),
  ]);
}

function loginTerminalButton(p, label = "Open login in Terminal") {
  return el("button", {
    class: "small",
    onclick: async (ev) => {
      const btn = ev.currentTarget;
      btn.disabled = true;
      try {
        await api(`/api/providers/${p.id}/login-terminal`, { method: "POST" });
        btn.textContent = "Opened — refresh when done";
      } catch (e) {
        btn.textContent = String(e.message || e);
      }
    },
  }, label);
}

// ---------- operations ----------

async function runOp(p, op) {
  const modal = $("#output-modal");
  $("#modal-title").textContent = `${p.name} — ${op.name}`;
  $("#modal-body").textContent = "Running…";
  modal.showModal();
  try {
    const r = await api(`/api/providers/${p.id}/op/${op.id}`, { method: "POST" });
    $("#modal-body").textContent = r.ok
      ? pretty(r.stdout) || "(no output)"
      : `exit ${r.status ?? "?"}${r.timed_out ? " (timed out)" : ""}\n\n${r.stderr || r.stdout}`;
  } catch (e) {
    $("#modal-body").textContent = String(e);
  }
}

// ---------- catalog ----------

function renderCatalog() {
  const box = $("#catalog");
  box.replaceChildren();
  for (const p of state.providers) {
    const entry = el("div", { class: "entry" });
    const row = el("div", { class: "entry-row" });

    const grow = el("div", { class: "grow" });
    const title = el("div");
    title.append(el("strong", {}, p.name), " ", el("span", { class: "kind" }, p.kind));
    if (p.source === "user") title.append(" ", el("span", { class: "kind" }, "user"));
    grow.append(title);
    if (p.description) grow.append(el("div", { class: "entry-desc" }, p.description));
    const meta = el("div", { class: "entry-meta" });
    meta.append(`binary: ${p.binary}`);
    if (p.detection.version) meta.append(` · ${p.detection.version}`);
    grow.append(meta);
    row.append(grow);

    row.append(
      p.detection.installed
        ? el("span", { class: "installed" }, "✓ installed")
        : el("span", { class: "not-installed" }, "○ not installed")
    );
    row.append(el("a", { href: p.repo, target: "_blank", rel: "noreferrer" }, "GitHub"));

    if (p.install) {
      const selfUpdate = p.detection.installed && p.install["self-update-args"];
      const btn = el("button", {
        class: "small",
        onclick: () => install(p, entry, btn),
      }, p.installing ? "Installing…"
        : selfUpdate ? "Self-update"
        : p.detection.installed ? "Reinstall/update"
        : "Install");
      if (p.installing) btn.disabled = true;
      row.append(btn);
      if (p.detection.installed && p.install["update-check-args"]) {
        const chk = el("button", { class: "small", onclick: () => checkUpdate(p, entry, chk) },
          "Check for update");
        row.append(chk);
      }
    }
    if (p.source === "user") {
      row.append(el("button", { class: "small", onclick: () => removeProvider(p) }, "Remove"));
    }
    entry.append(row);

    if (p.auth?.required && p.auth["login-command"]) {
      const login = el("div", { class: "entry-meta" });
      login.append("login (in your terminal): ", el("code", {}, p.auth["login-command"]), " ");
      if (p.detection.installed) login.append(loginTerminalButton(p, "Open in Terminal"));
      entry.append(login);
    } else if (p["setup-command"]) {
      const setup = el("div", { class: "entry-meta" });
      setup.append("setup (in your terminal): ", el("code", {}, p["setup-command"]));
      entry.append(setup);
    }
    box.append(entry);
  }
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
        log.textContent += st.state === "succeeded" ? "\n✓ done" : "\n✕ failed";
        break;
      }
      await new Promise((r) => setTimeout(r, 1000));
    }
  } catch (e) {
    log.textContent += `\n${e}`;
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
    await refresh();
  } catch (e) {
    alert(String(e));
  }
}

// ---------- register ----------

async function register() {
  const msg = $("#register-msg");
  msg.className = "";
  msg.textContent = "…";
  try {
    const r = await api("/api/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        toml: $("#register-toml").value,
        overwrite: $("#register-overwrite").checked,
      }),
    });
    msg.className = "ok";
    msg.textContent = `Registered ${r.id}`;
    await refresh();
  } catch (e) {
    msg.className = "err";
    msg.textContent = String(e.message || e);
  }
}

// ---------- boot ----------

async function refresh() {
  const { providers } = await api("/api/providers");
  state.providers = providers;
  renderCatalog();
  await loadSummaries();
}

$("#refresh").addEventListener("click", refresh);
$("#register-btn").addEventListener("click", register);
$("#modal-close").addEventListener("click", () => $("#output-modal").close());

refresh().catch((e) => {
  $("#cards").textContent = `Failed to load: ${e}`;
});
