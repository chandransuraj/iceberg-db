/** Snowsight-style SQL workspace over idb-wasm */

const SAMPLE_QUERIES = [
  {
    label: "Count customers",
    sql: "SELECT COUNT(*) AS n FROM demo.customers",
  },
  {
    label: "All rows",
    sql: "SELECT * FROM demo.customers ORDER BY id",
  },
  {
    label: "By region",
    sql: "SELECT region, COUNT(*) AS n FROM demo.customers GROUP BY region ORDER BY n DESC",
  },
];

const SCHEMA = {
  catalog: "local",
  schemas: [
    {
      name: "demo",
      tables: [{ name: "customers", rows: 3, columns: ["id", "name", "region"] }],
    },
  ],
};

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
        setSql(`SELECT *\nFROM ${fq}\nORDER BY 1\nLIMIT 100`);
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
  el.className = kind === "ok" ? "status-ok" : kind === "err" ? "status-err" : "";
  el.textContent = message;
  $("status-rows").textContent = rows != null ? `${rows} row(s)` : "—";
  $("status-time").textContent = ms != null ? `${ms} ms` : "—";
}

function setRunning(running) {
  $("run-btn").disabled = running || !window.__idbReady;
  $("loading").classList.toggle("hidden", !running);
}

async function runQuery() {
  const { idb_query } = window.__idb;
  const sql = $("sql").value;
  setRunning(true);
  setStatus({ message: "Running…", kind: "muted" });
  try {
    const result = await idb_query(sql);
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
    setRunning(false);
  }
}

async function boot() {
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

  syncLineNumbers();

  const { idb_init, idb_query, idb_wasm_version } = await loadWasm();
  window.__idb = { idb_init, idb_query, idb_wasm_version };

  setStatus({ message: "Starting engine…", kind: "muted" });
  try {
    await idb_init();
    window.__idbReady = true;
    $("run-btn").disabled = false;
    $("engine-version").textContent = `iceberg-db wasm ${idb_wasm_version()}`;
    setStatus({ message: "Ready", kind: "ok" });
    $("results-grid-wrap").innerHTML =
      '<div class="empty-state">Run a query or pick a sample from the left.</div>';
  } catch (e) {
    setStatus({ message: "Init failed", kind: "err" });
    $("results-grid-wrap").innerHTML = `<div class="empty-state" style="color:var(--error)">${escapeHtml(String(e))}</div>`;
    console.error(e);
  } finally {
    $("loading").classList.add("hidden");
  }
}

boot().catch((e) => {
  console.error(e);
  document.getElementById("loading")?.classList.add("hidden");
  const status = document.getElementById("status-message");
  if (status) status.textContent = "Failed to start";
});
