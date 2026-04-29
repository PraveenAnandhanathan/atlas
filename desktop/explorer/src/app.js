/**
 * ATLAS Explorer — frontend application (T6.6).
 *
 * Five tabs: Browser · Search · Lineage · Version · Policy.
 *
 * In production this file is compiled from TypeScript (src/app.ts) by
 * Vite.  The JavaScript version here works standalone for development
 * and matches the compiled output structure.
 *
 * IPC: calls Tauri's `invoke()` when running inside the desktop app, or
 * falls back to a mock backend for browser-based development.
 */

// ---------------------------------------------------------------------------
// Tauri IPC bridge
// ---------------------------------------------------------------------------

const isTauri = typeof window.__TAURI__ !== "undefined";

async function invoke(cmd, args = {}) {
  if (isTauri) {
    return window.__TAURI__.core.invoke(cmd, args);
  }
  // Mock backend for browser-only development.
  return mockInvoke(cmd, args);
}

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

function mockInvoke(cmd, args) {
  switch (cmd) {
    case "browse":
      return Promise.resolve({
        path: args.path ?? "/",
        breadcrumbs: (args.path ?? "/").split("/").filter(Boolean),
        entries: [
          { path: "/datasets", name: "datasets", kind: "Directory", size: 0,  hash_hex: "a1b2c3d4e5f6", modified_ms: 0, content_type: "directory" },
          { path: "/models",   name: "models",   kind: "Directory", size: 0,  hash_hex: "b2c3d4e5f6a1", modified_ms: 0, content_type: "directory" },
          { path: "/README.md",name: "README.md",kind: "File",      size: 1024, hash_hex: "c3d4e5f6a1b2", modified_ms: 0, content_type: "binary" },
        ],
        error: null,
      });

    case "search":
      return Promise.resolve({
        query: args.request?.query ?? "",
        results: [
          { path: "/models/gpt2.safetensors", snippet: "SafeTensors model checkpoint", score: 0.92, kind: "file" },
          { path: "/datasets/train.parquet",  snippet: "Training dataset",              score: 0.84, kind: "file" },
        ],
        total: 2,
        took_ms: 12,
        error: null,
      });

    case "lineage":
      return Promise.resolve({
        path: args.request?.path ?? "/",
        edges: [
          { from_path: "/datasets/raw/data.jsonl", to_path: "/datasets/train.parquet", kind: "derived_from", timestamp_ms: 1700000000000, actor: "pipeline@atlas" },
          { from_path: "/datasets/train.parquet",  to_path: "/models/gpt2.safetensors", kind: "trained_on",  timestamp_ms: 1700001000000, actor: "trainer@atlas" },
        ],
        error: null,
      });

    case "version_log":
      return Promise.resolve({
        commits: [
          { hash_hex: "deadbeef0001", short_hash: "deadbeef", message: "Add GPT-2 checkpoint", author: "alice", timestamp_ms: 1700001000000, parent_hashes: ["deadbeef0000"] },
          { hash_hex: "deadbeef0000", short_hash: "deadbee0", message: "init: empty store",   author: "atlas", timestamp_ms: 1700000000000, parent_hashes: [] },
        ],
        branches: [
          { name: "main",     head_hash: "deadbeef0001", is_current: true },
          { name: "training", head_hash: "deadbeef0000", is_current: false },
        ],
        current_branch: "main",
        error: null,
      });

    case "policy_view":
      return Promise.resolve({
        view: {
          path: args.request?.path ?? "/",
          rules: [
            { permission: "read",  principal: "*",     effect: "allow" },
            { permission: "write", principal: "admin", effect: "allow" },
            { permission: "write", principal: "*",     effect: "deny"  },
          ],
          redaction_enabled: true,
          capability_scope: "atlas://localhost/myvol/**",
        },
        error: null,
      });

    default:
      return Promise.reject(new Error(`Unknown command: ${cmd}`));
  }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let currentPath = "/";
let currentTab = "browser";

// ---------------------------------------------------------------------------
// Tab switching
// ---------------------------------------------------------------------------

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".tab-panel").forEach((p) => p.classList.add("hidden"));
    btn.classList.add("active");
    const tab = btn.dataset.tab;
    currentTab = tab;
    document.getElementById(`tab-${tab}`).classList.remove("hidden");
    renderTab(tab);
  });
});

// ---------------------------------------------------------------------------
// Open store
// ---------------------------------------------------------------------------

document.getElementById("open-store-btn").addEventListener("click", async () => {
  if (isTauri) {
    const { open } = window.__TAURI__.dialog;
    const path = await open({ directory: true, title: "Open ATLAS Store" });
    if (path) {
      await invoke("open_store", { path });
      document.getElementById("store-path").textContent = path;
      renderTab(currentTab);
    }
  } else {
    document.getElementById("store-path").textContent = "/mock/atlas-store";
    renderTab(currentTab);
  }
});

// ---------------------------------------------------------------------------
// Render dispatch
// ---------------------------------------------------------------------------

async function renderTab(tab) {
  switch (tab) {
    case "browser":  return renderBrowser();
    case "search":   return renderSearch();
    case "lineage":  return renderLineage();
    case "version":  return renderVersion();
    case "policy":   return renderPolicy();
  }
}

// ---------------------------------------------------------------------------
// Browser tab
// ---------------------------------------------------------------------------

async function renderBrowser() {
  const panel = document.getElementById("tab-browser");
  panel.innerHTML = `<p class="text-dim" style="color:var(--text-dim)">Loading…</p>`;
  const resp = await invoke("browse", { request: { path: currentPath } });

  const crumbs = ["Root", ...resp.breadcrumbs]
    .map((c, i, arr) => {
      const path = "/" + arr.slice(1, i + 1).join("/");
      return `<span onclick="navigateTo('${path}')" style="cursor:pointer;color:var(--accent)">${c}</span>`;
    })
    .join(" / ");

  const rows = resp.entries.map((e) => {
    const icon = e.kind === "Directory" ? "📁" : "📄";
    const sizeStr = e.kind === "Directory" ? "—" : formatBytes(e.size);
    const clickable = e.kind === "Directory" ? `onclick="navigateTo('${e.path}')"` : "";
    return `<tr class="${e.kind === "Directory" ? "clickable" : ""}" ${clickable}>
      <td>${icon} <span class="${e.kind === "Directory" ? "kind-dir" : "kind-file"}">${e.name}</span></td>
      <td class="size">${sizeStr}</td>
      <td class="hash">${e.hash_hex.slice(0, 12)}</td>
      <td>${e.content_type}</td>
    </tr>`;
  }).join("");

  panel.innerHTML = `
    <div class="breadcrumb">${crumbs}</div>
    <table class="file-table">
      <thead><tr><th>Name</th><th>Size</th><th>Hash</th><th>Type</th></tr></thead>
      <tbody>${rows || '<tr><td colspan="4" class="empty-state">Empty directory</td></tr>'}</tbody>
    </table>`;
}

window.navigateTo = async (path) => {
  currentPath = path || "/";
  await renderBrowser();
};

// ---------------------------------------------------------------------------
// Search tab
// ---------------------------------------------------------------------------

function renderSearch() {
  const panel = document.getElementById("tab-search");
  if (panel.querySelector(".search-bar")) return; // already initialised
  panel.innerHTML = `
    <div class="section-title">Hybrid Search</div>
    <div class="search-bar">
      <input type="text" id="search-input" placeholder="Type a query…" />
      <button id="search-btn">Search</button>
    </div>
    <div id="search-results"></div>`;
  document.getElementById("search-btn").addEventListener("click", runSearch);
  document.getElementById("search-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") runSearch();
  });
}

async function runSearch() {
  const query = document.getElementById("search-input").value.trim();
  if (!query) return;
  const resp = await invoke("search", { request: { query, path_prefix: "", limit: 50, vector: true, keyword: true } });
  const resultsEl = document.getElementById("search-results");
  if (resp.error) {
    resultsEl.innerHTML = `<p style="color:var(--danger)">${resp.error}</p>`;
    return;
  }
  resultsEl.innerHTML = resp.results.map((r) => `
    <div class="search-result">
      <span class="score">${(r.score * 100).toFixed(0)}%</span>
      <div class="path">${r.path}</div>
      <div class="snippet">${r.snippet}</div>
    </div>`).join("") || `<p class="empty-state">No results for "${query}"</p>`;
}

// ---------------------------------------------------------------------------
// Lineage tab
// ---------------------------------------------------------------------------

async function renderLineage() {
  const panel = document.getElementById("tab-lineage");
  const resp = await invoke("lineage", { request: { path: currentPath, depth: 5 } });
  const edges = resp.edges ?? [];
  const edgeHtml = edges.map((e) => `
    <div class="commit-entry">
      <div class="hash">${e.kind}</div>
      <div class="message">${e.from_path} → ${e.to_path}</div>
      <div class="meta">${new Date(e.timestamp_ms).toLocaleString()} · ${e.actor}</div>
    </div>`).join("") || '<p class="empty-state">No lineage edges recorded for this path.</p>';

  panel.innerHTML = `
    <div class="section-title">Lineage Graph — ${currentPath}</div>
    <div class="graph-placeholder" id="lineage-graph">
      ⬡ Interactive graph rendered by d3-dag (connect to lineage service)
    </div>
    ${edgeHtml}`;
}

// ---------------------------------------------------------------------------
// Version tab
// ---------------------------------------------------------------------------

async function renderVersion() {
  const panel = document.getElementById("tab-version");
  const resp = await invoke("version_log", { request: { path: "", limit: 50, after: null } });

  const branchHtml = (resp.branches ?? []).map((b) => `
    <span style="padding:4px 10px;border-radius:12px;font-size:0.75rem;
      background:${b.is_current ? "var(--accent)" : "var(--surface2)"};
      color:${b.is_current ? "#fff" : "var(--text-dim)"}">
      ${b.name}${b.is_current ? " ✓" : ""}
    </span>`).join(" ");

  const commitHtml = (resp.commits ?? []).map((c) => `
    <div class="commit-entry">
      <div class="hash">${c.short_hash}</div>
      <div class="message">${c.message}</div>
      <div class="meta">${c.author} · ${new Date(c.timestamp_ms).toLocaleString()}</div>
    </div>`).join("") || '<p class="empty-state">No commits yet.</p>';

  panel.innerHTML = `
    <div class="section-title">Current branch: ${resp.current_branch}</div>
    <div style="display:flex;gap:8px;flex-wrap:wrap;margin-bottom:20px">${branchHtml}</div>
    <div class="section-title">Commit History</div>
    ${commitHtml}`;
}

// ---------------------------------------------------------------------------
// Policy tab
// ---------------------------------------------------------------------------

async function renderPolicy() {
  const panel = document.getElementById("tab-policy");
  const resp = await invoke("policy_view", { request: { path: currentPath } });
  if (!resp.view) {
    panel.innerHTML = `<p class="empty-state">No policy attached to ${currentPath}</p>`;
    return;
  }
  const v = resp.view;
  const rulesHtml = v.rules.map((r) => `
    <div class="rule effect-${r.effect}">
      <strong>${r.effect.toUpperCase()}</strong>
      <span>${r.permission}</span>
      <span>→</span>
      <span style="color:var(--text-dim)">${r.principal}</span>
    </div>`).join("");

  panel.innerHTML = `
    <div class="section-title">Policy — ${v.path}</div>
    <div class="policy-view">
      <p style="margin-bottom:12px;font-size:0.8rem;color:var(--text-dim)">
        Redaction: <strong>${v.redaction_enabled ? "ON" : "OFF"}</strong>
        ${v.capability_scope ? `· Scope: <code>${v.capability_scope}</code>` : ""}
      </p>
      ${rulesHtml || '<p class="empty-state">No rules defined.</p>'}
    </div>`;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

renderTab("browser");
