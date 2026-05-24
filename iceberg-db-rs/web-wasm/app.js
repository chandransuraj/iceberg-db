/** Snowsight-style SQL workspace over idb-wasm (Horizon IRC or demo). */

const DEMO_SCHEMA = {
  catalog: "local",
  schemas: [
    {
      name: "demo",
      tables: [{ name: "customers", rows: 3, columns: ["id", "name", "region"] }],
    },
  ],
};

const HORIZON_SCHEMA = {
  catalog: "snowflake_horizon",
  schemas: [
    {
      name: "iceberg_test",
      tables: [{ name: "employee", rows: "?", columns: ["(Iceberg table)"] }],
    },
  ],
};

const DEMO_QUERIES = [
  { label: "Count customers", sql: "SELECT COUNT(*) AS n FROM demo.customers" },
  { label: "All rows", sql: "SELECT * FROM demo.customers ORDER BY id" },
  {
    label: "By region",
    sql: "SELECT region, COUNT(*) AS n FROM demo.customers GROUP BY region ORDER BY n DESC",
  },
];

const HORIZON_QUERIES = [
  { label: "Sample rows", sql: "SELECT * FROM iceberg_test.employee LIMIT 10" },
  { label: "Show tables", sql: "SHOW TABLES IN iceberg_test" },
  { label: "Count", sql: "SELECT COUNT(*) AS n FROM iceberg_test.employee" },
];

let currentMode = "horizon";
let SCHEMA = HORIZON_SCHEMA;
let SAMPLE_QUERIES = HORIZON_QUERIES;

function $(id) {
  return document.getElementById(id);
}

async function loadWasm() {
  if (window.wasmBindings) return window.wasmBindings;
  return new Promise((resolve) => {
    window.addEventListener("TrunkApplicationStarted", () => resolve(window.wasmBindings), {
      once: true,
    });
  });
}

function syncLineNumbers() {
  const sql = $("sql");
  const gutter = $("line-gutter");
  const lines = sql.value.split("\n").length;
  const nums = Array.from({ length: Math.max(lines, 1) }, (_, i) => i + 1).join("\n");
  gutter.textContent = nums;
  gutter.scrollTop = sql.scrollTop;
}

function setSql(text) {
  $("sql").value = text;
  syncLineNumbers();
}

/** YAML double-quoted scalar (PAT/scope contain `:` and break unquoted YAML). */
function yamlScalar(value) {
  return JSON.stringify(String(value));
}

function buildHorizonYaml() {
  const account = $("sf-account").value.trim();
  const warehouse = $("sf-warehouse").value.trim();
  const schema = $("sf-schema").value.trim();
  const username = $("sf-username").value.trim();
  const scope = $("sf-scope").value.trim();
  const pat = $("sf-pat").value.trim();

  if (!account) throw new Error("Account host is required (e.g. qtfneqx-er54214)");
  if (!warehouse) throw new Error("Database (warehouse) is required");
  if (!pat || pat.length < 32) throw new Error("Paste a valid Snowflake PAT (32+ characters)");

  // Call idb-sf-proxy directly (:8787). Trunk /sf/ rewrite can drop OAuth POST bodies → 403.
  const host = window.location.hostname;
  const useDevProxy =
    host === "127.0.0.1" || host === "localhost" || host === "::1";
  const SF_PROXY = "http://127.0.0.1:8787";
  const uri = useDevProxy
    ? `${SF_PROXY}/${account}/polaris/api/catalog`
    : `https://${account}.snowflakecomputing.com/polaris/api/catalog`;

  return `default-catalog: snowflake_horizon

catalogs:
  snowflake_horizon:
    type: rest
    profile: snowflake-horizon
    uri: ${yamlScalar(uri)}
    warehouse: ${yamlScalar(warehouse)}
    default-schema: ${yamlScalar(schema)}
    token: ${yamlScalar(pat)}
    scope: ${yamlScalar(scope)}
${username ? `    username: ${yamlScalar(username)}\n` : ""}`;
}

function saveHorizonSettings() {
  localStorage.setItem(
    "idb-horizon-settings",
    JSON.stringify({
      account: $("sf-account").value.trim(),
      warehouse: $("sf-warehouse").value.trim(),
      schema: $("sf-schema").value.trim(),
      username: $("sf-username").value.trim(),
      scope: $("sf-scope").value.trim(),
    })
  );
}

function loadHorizonSettings() {
  try {
    const raw = localStorage.getItem("idb-horizon-settings");
    if (!raw) return;
    const s = JSON.parse(raw);
    if (s.account) $("sf-account").value = s.account;
    if (s.warehouse) $("sf-warehouse").value = s.warehouse;
    if (s.schema) $("sf-schema").value = s.schema;
    if (s.username) $("sf-username").value = s.username;
    if (s.scope) $("sf-scope").value = s.scope;
  } catch (_) {
    /* ignore */
  }
}

function setMode(mode) {
  currentMode = mode;
  if (mode === "demo") {
    SCHEMA = DEMO_SCHEMA;
    SAMPLE_QUERIES = DEMO_QUERIES;
    $("mode-pill").textContent = "demo";
    $("sql-hint").textContent = "SELECT * FROM demo.customers";
    setSql("SELECT COUNT(*) AS n FROM demo.customers");
  } else {
    SCHEMA = HORIZON_SCHEMA;
    SAMPLE_QUERIES = HORIZON_QUERIES;
    $("mode-pill").textContent = "horizon";
    const schema = $("sf-schema").value.trim() || "iceberg_test";
    $("sql-hint").textContent = `SELECT * FROM ${schema}.employee LIMIT 10`;
    setSql(`SELECT * FROM ${schema}.employee LIMIT 10`);
  }
  renderSchemaTree();
  renderSamples();
}

function renderSchemaTree() {
  const root = $("schema-tree");
  root.innerHTML = "";

  const catLi = document.createElement("li");
  catLi.className = "tree-item";
  catLi.innerHTML = `<div class="tree-row"><span class="tree-icon">▾</span><span>${SCHEMA.catalog}</span></div>`;
  const catUl = document.createElement("ul");
  catUl.className = "tree-children";

  for (const schema of SCHEMA.schemas) {
    const schLi = document.createElement("li");
    schLi.className = "tree-item";
    schLi.innerHTML = `<div class="tree-row"><span class="tree-icon">▾</span><span>${schema.name}</span></div>`;
    const tblUl = document.createElement("ul");
    tblUl.className = "tree-children";

    for (const table of schema.tables) {
      const tblLi = document.createElement("li");
      tblLi.className = "tree-item";
      const fq = `${schema.name}.${table.name}`;
      tblLi.innerHTML = `<div class="tree-row" data-fq="${fq}" title="${table.columns.join(", ")}"><span class="tree-icon">◇</span><span>${table.name}</span><span style="margin-left:auto;opacity:.6;font-size:11px">${table.rows}</span></div>`;
      tblLi.querySelector(".tree-row").addEventListener("click", () => {
        setSql(`SELECT *\nFROM ${fq}\nLIMIT 100`);
        document.querySelectorAll(".tree-row.active").forEach((el) => el.classList.remove("active"));
        tblLi.querySelector(".tree-row").classList.add("active");
      });
      tblUl.appendChild(tblLi);
    }
    schLi.appendChild(tblUl);
    catUl.appendChild(schLi);
  }
  catLi.appendChild(catUl);
  root.appendChild(catLi);
}

function renderSamples() {
  const list = $("sample-list");
  list.innerHTML = "";
  for (const sample of SAMPLE_QUERIES) {
    const li = document.createElement("li");
    li.textContent = sample.label;
    li.title = sample.sql;
    li.addEventListener("click", () => setSql(sample.sql));
    list.appendChild(li);
  }
}

function pushHistory(sql) {
  const trimmed = sql.trim();
  if (!trimmed) return;
  let history = JSON.parse(localStorage.getItem("idb-query-history") || "[]");
  history = history.filter((q) => q !== trimmed);
  history.unshift(trimmed);
  history = history.slice(0, 12);
  localStorage.setItem("idb-query-history", JSON.stringify(history));
  renderHistory();
}

function renderHistory() {
  const list = $("history-list");
  const history = JSON.parse(localStorage.getItem("idb-query-history") || "[]");
  list.innerHTML = "";
  if (!history.length) {
    list.innerHTML = '<li style="cursor:default;opacity:.6">No runs yet</li>';
    return;
  }
  for (const sql of history) {
    const li = document.createElement("li");
    li.textContent = sql.replace(/\s+/g, " ").slice(0, 80);
    li.title = sql;
    li.addEventListener("click", () => setSql(sql));
    list.appendChild(li);
  }
}

function switchResultsTab(name) {
  document.querySelectorAll(".results-tab").forEach((tab) => {
    tab.classList.toggle("active", tab.dataset.tab === name);
  });
  document.querySelectorAll(".panel").forEach((panel) => {
    panel.classList.toggle("active", panel.dataset.panel === name);
  });
}

function renderGrid(result) {
  const wrap = $("results-grid-wrap");
  if (!result.columns?.length) {
    wrap.innerHTML = '<div class="empty-state">Query returned no columns.</div>';
    return;
  }

  const table = document.createElement("table");
  table.className = "results-grid";

  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  for (const col of result.columns) {
    const th = document.createElement("th");
    th.innerHTML = `${escapeHtml(col.name)}<span class="type-badge">${escapeHtml(col.data_type)}</span>`;
    headRow.appendChild(th);
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = document.createElement("tbody");
  const rows = result.rows || [];
  if (!rows.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = result.columns.length;
    td.textContent = "(no rows)";
    tr.appendChild(td);
    tbody.appendChild(tr);
  } else {
    for (const row of rows) {
      const tr = document.createElement("tr");
      for (const cell of row) {
        const td = document.createElement("td");
        td.textContent = cell ?? "";
        td.title = cell ?? "";
        tr.appendChild(td);
      }
      tbody.appendChild(tr);
    }
  }
  table.appendChild(tbody);
  wrap.innerHTML = "";
  wrap.appendChild(table);
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function setStatus({ message, kind = "muted", rows, ms }) {
  const el = $("status-message");
  el.className = `status-message${kind === "ok" ? " status-ok" : kind === "err" ? " status-err" : ""}`;
  el.textContent = message;
  el.title = message;
  $("status-rows").textContent = rows != null ? `${rows} row(s)` : "—";
  $("status-time").textContent = ms != null ? `${ms} ms` : "—";
}

let queryRunning = false;

function setQueryStep(step) {
  if (!queryRunning) return;
  const message = `Running… · ${step}`;
  const el = $("status-message");
  el.className = "status-message";
  el.textContent = message;
  el.title = step;
}

function installQueryLogHook() {
  const origLog = console.log.bind(console);
  console.log = (...args) => {
    origLog(...args);
    const msg = args.map((a) => (typeof a === "string" ? a : String(a))).join(" ");
    if (msg.startsWith("idb_query: ")) {
      setQueryStep(msg.slice("idb_query: ".length));
    }
  };
}

function setLoadingOverlay(visible, message) {
  const card = $("loading").querySelector(".loading-card");
  const label = card?.querySelector(".loading-label");
  if (label && message) label.textContent = message;
  $("loading").classList.toggle("hidden", !visible);
}

function setRunning(running) {
  $("run-btn").disabled = running || !window.__idbReady;
  // Queries use the status bar only — do not reuse the full-screen init overlay.
}

/** Selected text when non-empty, otherwise the full editor contents. */
function getSqlToRun() {
  const sqlEl = $("sql");
  const { value, selectionStart, selectionEnd } = sqlEl;
  const selected = value.slice(selectionStart, selectionEnd).trim();
  if (selectionStart !== selectionEnd && selected) {
    return selected;
  }
  return value.trim();
}

async function runQuery() {
  if (!window.__idbReady || !window.__idb?.idb_query) {
    console.error("[iceberg-db] Run ignored — connect or demo first");
    setStatus({ message: "Connect or Demo first", kind: "err" });
    return;
  }
  const { idb_query } = window.__idb;
  const sql = getSqlToRun();
  if (!sql) {
    setStatus({ message: "Nothing to run", kind: "err" });
    return;
  }
  const ver = window.__idb?.idb_wasm_version?.() ?? "?";
  console.info("[iceberg-db] runQuery", { ver, sql: sql.slice(0, 120) });
  queryRunning = true;
  setRunning(true);
  setStatus({ message: "Running…", kind: "muted" });
  try {
    const result = await Promise.race([
      idb_query(sql),
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error("Query timed out after 120s — check console for idb_query: logs")),
          120_000
        )
      ),
    ]);
    console.info("[iceberg-db] runQuery ok", result?.row_count, "rows");
    pushHistory(sql);
    renderGrid(result);
    $("out-text").textContent = result.text || "";
    setStatus({
      message: "Completed",
      kind: "ok",
      rows: result.row_count,
      ms: result.elapsed_ms,
    });
    switchResultsTab("grid");
  } catch (e) {
    const msg = String(e);
    $("results-grid-wrap").innerHTML = `<div class="empty-state" style="color:var(--error)">${escapeHtml(msg)}</div>`;
    $("out-text").textContent = msg;
    setStatus({ message: "Failed", kind: "err" });
    switchResultsTab("text");
  } finally {
    queryRunning = false;
    setRunning(false);
  }
}

async function initEngine(initFn, label) {
  setStatus({ message: `${label}…`, kind: "muted" });
  setLoadingOverlay(true, label.endsWith("…") ? label : `${label}…`);
  window.__idbReady = false;
  $("run-btn").disabled = true;
  try {
    await initFn();
    window.__idbReady = true;
    $("run-btn").disabled = false;
    setStatus({ message: "Ready", kind: "ok" });
    $("results-grid-wrap").innerHTML =
      '<div class="empty-state">Run a query or pick a sample from the left.</div>';
  } catch (e) {
    setStatus({ message: "Init failed", kind: "err" });
    $("results-grid-wrap").innerHTML = `<div class="empty-state" style="color:var(--error)">${escapeHtml(e)}</div>`;
    console.error(e);
  } finally {
    setLoadingOverlay(false);
  }
}

async function connectHorizon() {
  const { idb_init_horizon } = window.__idb;
  if (!idb_init_horizon) {
    throw new Error("WASM build missing idb_init_horizon — rebuild with horizon feature");
  }
  saveHorizonSettings();
  const yaml = buildHorizonYaml();
  const account = $("sf-account").value.trim();
  const scope = $("sf-scope").value.trim();
  const patLen = $("sf-pat").value.trim().length;
  const host = window.location.hostname;
  const useDevProxy =
    host === "127.0.0.1" || host === "localhost" || host === "::1";
  const oauthUri = useDevProxy
    ? `http://127.0.0.1:8787/${account}/polaris/api/catalog/v1/oauth/tokens`
    : `https://${account}.snowflakecomputing.com/polaris/api/catalog/v1/oauth/tokens`;
  console.info("Horizon connect", { oauthUri, scope, patLen, username: $("sf-username").value.trim() || "(none)" });
  setMode("horizon");
  await initEngine(() => idb_init_horizon(yaml), "Connecting to Horizon");
}

async function connectDemo() {
  const { idb_init_demo } = window.__idb;
  if (!idb_init_demo) {
    throw new Error("WASM build missing idb_init_demo");
  }
  setMode("demo");
  await initEngine(() => idb_init_demo(), "Starting demo");
}

async function boot() {
  installQueryLogHook();
  loadHorizonSettings();
  setMode("horizon");
  renderSchemaTree();
  renderSamples();
  renderHistory();

  const sqlEl = $("sql");
  sqlEl.addEventListener("input", syncLineNumbers);
  sqlEl.addEventListener("scroll", () => {
    $("line-gutter").scrollTop = sqlEl.scrollTop;
  });
  sqlEl.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      runQuery();
    }
    if (e.key === "Tab") {
      e.preventDefault();
      const start = sqlEl.selectionStart;
      const end = sqlEl.selectionEnd;
      sqlEl.value = `${sqlEl.value.slice(0, start)}  ${sqlEl.value.slice(end)}`;
      sqlEl.selectionStart = sqlEl.selectionEnd = start + 2;
      syncLineNumbers();
    }
  });

  document.querySelectorAll(".results-tab").forEach((tab) => {
    tab.addEventListener("click", () => switchResultsTab(tab.dataset.tab));
  });

  $("run-btn").addEventListener("click", runQuery);
  $("clear-btn").addEventListener("click", () => {
    $("results-grid-wrap").innerHTML =
      '<div class="empty-state">Run a query to see results.</div>';
    $("out-text").textContent = "";
    setStatus({ message: "Ready", kind: "ok" });
  });

  $("connect-horizon-btn").addEventListener("click", () => connectHorizon().catch(console.error));
  $("demo-btn").addEventListener("click", () => connectDemo().catch(console.error));

  syncLineNumbers();

  const bindings = await loadWasm();
  const { idb_init_horizon, idb_init_demo, idb_query, idb_wasm_version } = bindings;
  window.__idb = { idb_init_horizon, idb_init_demo, idb_query, idb_wasm_version };

  const wasmVer = idb_wasm_version();
  console.info("[iceberg-db] wasm build", wasmVer);
  $("engine-version").textContent = `iceberg-db wasm ${wasmVer}`;
  setStatus({ message: "Enter PAT and click Connect", kind: "muted" });
  $("results-grid-wrap").innerHTML =
    '<div class="empty-state">Connect to Snowflake Horizon or use Demo data.</div>';
  setLoadingOverlay(false);
}

boot().catch((e) => {
  console.error(e);
  setLoadingOverlay(false);
  const status = document.getElementById("status-message");
  if (status) status.textContent = "Failed to start";
});
