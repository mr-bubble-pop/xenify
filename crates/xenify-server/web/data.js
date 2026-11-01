// Admin › Data sources: databases people can use in Calc (query sheets read
// them, table editor sheets change them). The server holds their passwords
// and sees every result, which the form says before one is added.
//
// Whoever may set up database connections (a role permission; admins
// too) sees and changes every source here, and picks who else may write
// queries on each. Once a query is in a file, anyone who has the file can
// load it again: the server keeps the file to that exact query.
//
// app.js hands over its helpers with dataInit(); its render() calls
// dataPage() and its click wiring runs dataActs.

let $ = null; // { state, api, esc, shell, field, errorBox, render, busy, fail, toast }

export function dataInit(env) {
  $ = env;
  $.state.ds = { list: [], dir: { people: [], groups: [] }, form: null, test: null, queries: [], log: null, logFor: null, folders: null, foldersText: "" };
}

const KIND_NAMES = { postgres: "PostgreSQL", mysql: "MySQL / MariaDB", mssql: "SQL Server", sqlite: "A SQLite file on this computer", csv: "CSV files on this computer" };
const SHORT = { postgres: "PostgreSQL", mysql: "MySQL", mssql: "SQL Server", sqlite: "SQLite file", csv: "CSV" };
const isFile = (k) => k === "sqlite" || k === "csv";
const words = (list) => (list || []).join(", ");
const split = (text) => String(text || "").split(",").map((s) => s.trim()).filter(Boolean);
const isAdmin = () => !!$.state.session?.account?.admin;
const TLS_NAMES = [["require", "Require (verify certificate)"], ["prefer", "Prefer (encrypted if offered, certificate not checked)"], ["off", "Off (only on a trusted network)"]];
/// The address or kind changed from what's saved: kept passwords aren't used there.
const moved = (f) => f.id && (f.kind !== f.orig?.kind || (f.address || "").trim() !== (f.orig?.address || "").trim());

export async function loadData() {
  const ds = $.state.ds;
  [ds.list, ds.dir, ds.folders] = await Promise.all([$.api("GET", "/v1/admin/data"), $.api("GET", "/v1/directory"), $.api("GET", "/v1/admin/data/folders")]);
  ds.foldersText = (ds.folders.folders || []).join("\n");
}

function tlsChip(s) {
  if (isFile(s.kind)) return "";
  if (s.tls === "require") return "";
  if (s.tls === "prefer") return '<span class="chip warn">Certificate not checked</span>';
  return '<span class="chip warn">Not encrypted</span>';
}

function foldersCard() {
  const { esc, state } = $;
  const f = state.ds.folders;
  if (!f) return "";
  const list = f.can_change
    ? `<label class="field"><span>Folders, one per line</span><textarea class="inp mono" rows="3" id="f-ds-folders" data-bind="ds.foldersText" spellcheck="false">${esc(state.ds.foldersText)}</textarea></label>
      <div class="row"><button class="btn small" data-act="ds-save-folders">Save folders</button></div>`
    : `${(f.folders || []).map((x) => `<code>${esc(x)}</code>`).join("<br>") || '<p class="dim">None.</p>'}<p class="dim">Only an admin can change these.</p>`;
  return `<section class="card stack" aria-label="Folders for files"><h2>Folders for files</h2>
    <p class="dim">SQLite files and CSV files must be in one of these folders on this computer. The data folder is <code>${esc(f.data || "")}</code>.</p>${list}</section>`;
}

function sourceRow(s) {
  const { esc, state } = $;
  const writers = (s.writers || []).map((x) => `<span class="chip ${x.group ? "ember" : ""}">${esc(x.name)}</span>`);
  const edit = s.write_user && s.editable.length ? `${s.editable.length === 1 ? "1 editable table" : `${s.editable.length} editable tables`}` : "read only";
  const log = state.ds.logFor === s.id && state.ds.log;
  return `<div class="res"><div class="stack"><b>${esc(s.name)}</b><div class="chips"><span class="chip">${esc(SHORT[s.kind] || s.kind)}</span><span class="chip">${esc(edit)}</span>${tlsChip(s)}${writers.length ? `<span class="dim">Queries by</span>${writers.join("")}` : ""}</div>
      ${log ? `<div class="ds-log">${log.length ? log.map((l) => `<div class="row"><span class="dim">${esc(new Date(l.at).toLocaleString())}</span><b>${esc(l.who || "—")}</b><span class="grow">${esc(describe(l))}</span></div>`).join("") : '<p class="dim">Nobody has used it yet.</p>'}</div>` : ""}</div>
    <div class="row"><button class="btn small" data-act="ds-log" data-id="${s.id}">${log ? "Hide" : "Who used it"}</button><button class="btn small" data-act="ds-edit" data-id="${s.id}">Change</button><button class="btn small danger" data-act="ds-delete" data-id="${s.id}">Remove</button></div></div>`;
}

function describe(l) {
  const d = l.detail || {};
  if (l.what === "read") return `${d.which?.query ? `ran ${d.which.query}` : `ran their own query`} · ${d.rows} rows`;
  if (l.what === "read table") return `opened ${d.table} to edit · ${d.rows} rows`;
  if (l.what === "add row") return `added a row to ${d.table}${d.outcome?.key ? ` (${Object.entries(d.outcome.key).map(([k, v]) => `${k} ${v}`).join(", ")})` : ""}`;
  if (l.what === "delete row") return `deleted ${d.table} ${Object.entries(d.key || {}).map(([k, v]) => `${k} ${v}`).join(", ")}${d.outcome?.outcome === "missing" ? " (it was gone already)" : ""}`;
  if (l.what === "change") {
    const key = Object.entries(d.key || {}).map(([k, v]) => `${k} ${v}`).join(", ");
    const how = { changed: "", conflict: " (refused: someone changed it first)", missing: " (refused: the row's gone)" }[d.outcome?.outcome] ?? "";
    return `${d.table} ${key}: ${d.column} ${JSON.stringify(d.old)} → ${JSON.stringify(d.new)}${how}`;
  }
  return l.what;
}

function trustNote() {
  return `<div class="note warn-note" role="note"><b>This computer will hold the database password and see every result.</b><br>
    Anyone who gets into this computer could use this database too.
    Give it a database user that can see only what Calc needs.</div>`;
}

function pwLabel(f, set) {
  if (set && moved(f)) return "Type the password again for the new address";
  return set ? "Its password (kept; type to change)" : "Its password";
}

function tlsFields(f) {
  const { esc } = $;
  const opts = [...TLS_NAMES]
    .map(([v, n]) => `<option value="${v}" ${f.tls === v ? "selected" : ""}>${esc(n)}</option>`).join("");
  const why = { "": "This connection isn’t encrypted: passwords and results cross the network as they are. Pick Require if the database offers encryption.",
    prefer: "Encrypted when the database offers it, but anyone in between could pretend to be it.",
    off: "Passwords and results cross the network as they are. A password is never sent unencrypted to another computer." }[f.tls];
  return `<label class="field"><span>Encryption</span><select class="inp" data-bind="ds.form.tls">${opts}</select></label>
    ${why ? `<p class="dim">${esc(why)}</p>` : ""}
    ${f.tls === "require" ? `<label class="field"><span>CA certificate (only for a private certificate authority)</span><textarea class="inp mono" rows="3" id="f-ds-ca" data-bind="ds.form.ca_cert" placeholder="-----BEGIN CERTIFICATE-----" spellcheck="false">${esc(f.ca_cert || "")}</textarea></label>` : ""}`;
}

function localField(f) {
  if (isFile(f.kind)) return "";
  const admin = isAdmin();
  return `<label class="check"><input type="checkbox" data-act="ds-local" ${f.allow_local ? "checked" : ""} ${admin ? "" : "disabled"}>Allow local addresses</label>
    <p class="dim">Addresses on this computer, link-local ones and this server’s own database are refused unless this is ticked.${admin ? "" : " Only an admin can change it."}</p>`;
}

function form() {
  const { esc, field, state } = $;
  const f = state.ds.form;
  const t = state.ds.test;
  const file = isFile(f.kind);
  const kinds = Object.entries(KIND_NAMES).map(([k, n]) => `<option value="${k}" ${f.kind === k ? "selected" : ""}>${esc(n)}</option>`).join("");
  let test = "";
  if (t?.busy) test = '<p class="dim">Trying it…</p>';
  else if (t && !t.ok) test = `<p class="chip warn">Didn’t work</p><p class="dim">${esc(t.message || "")}</p>`;
  else if (t) {
    const reader = file ? "Calc" : f.read_user || "The reading user";
    const lines = [`${esc(reader)} can see ${t.tables === 1 ? "1 table" : `${t.tables} tables`}.`];
    if (t.reader_can_change?.length) lines.push(`<b>${esc(reader)} can also change ${esc(words(t.reader_can_change))}.</b> Queries run in a read-only session anyway, but give it a user that can only read.`);
    if (t.changes?.ok === false) lines.push(`The user for changes couldn’t sign in: ${esc(t.changes.message)}`);
    else if (t.changes?.tables) lines.push(`${esc(f.write_user)} can change ${t.changes.tables.length ? esc(words(t.changes.tables)) : "no tables"}.`);
    lines.push(file ? "Queries always run read-only, with the limits below." : `Queries always run as ${esc(reader)}, in a read-only session, with the limits below.`);
    test = `<p><span class="chip ${t.reader_can_change?.length || t.changes?.ok === false ? "warn" : "ok"}">Connected</span></p><p class="dim">${lines.join(" ")}</p>`;
  }
  const dir = state.ds.dir;
  const has = (k, id) => f.writers.some((x) => (k === "g" ? x.group : x.user) === id);
  const who = [...dir.groups.filter((g) => !has("g", g.id)).map((g) => `<option value="g:${g.id}">${esc(g.name)} (group)</option>`),
    ...dir.people.filter((p) => !has("u", p.id)).map((p) => `<option value="u:${p.id}">${esc(p.display_name || p.username)}</option>`)].join("");
  const writers = `${f.writers.map((x, i) => `<div class="row ds-share"><span class="grow">${x.group ? "👥 " : ""}${esc(x.name)}</span>
      <button class="more" aria-label="${esc(x.name)} can't write queries" data-act="ds-unwriter" data-id="${i}">×</button></div>`).join("") || '<p class="dim">Nobody else yet.</p>'}
    ${who ? `<select class="inp auto" aria-label="Let a person or group write queries" data-act="ds-writer"><option value="">Add a person or group…</option>${who}</select>` : ""}
    <p class="dim">They write queries in Calc (their own SQL, or a saved one) and save queries for others. Whoever may set up database connections can too.
      Once a query is in a file, anyone who has the file can load new values for it, but not change it.</p>`;
  const offered = t?.changes?.tables || t?.names || [];
  const tables = f.editable.map((e, i) => `<div class="ds-table">
      ${field("Table", `ds.form.editable.${i}.table`, { placeholder: offered[0] || "products" })}
      ${field("Finds each row by", `ds.form.editable.${i}.keyText`, { placeholder: "id" })}
      ${field("Can’t be changed", `ds.form.editable.${i}.lockedText`, { placeholder: "created, sku" })}
      <button class="more" aria-label="Stop letting people edit ${esc(e.table || "this table")}" data-act="ds-untable" data-id="${i}">×</button>
      <div class="ds-rows"><label class="check"><input type="checkbox" data-act="ds-rowflag" data-id="${i}" data-flag="add" ${e.add !== false ? "checked" : ""}>People can add rows</label>
        <label class="check"><input type="checkbox" data-act="ds-rowflag" data-id="${i}" data-flag="delete" ${e.delete ? "checked" : ""}>People can delete rows</label></div></div>`).join("");
  const queries = f.id ? `<section class="stack" aria-label="Saved queries"><h2>Saved queries</h2>
      <p class="dim">Whoever writes queries picks these by name in Calc, and can save more there. Write <code>:name</code> for a value the sheet fills in, like <code>WHERE year = :year</code>.</p>
      ${state.ds.queries.map((q, i) => `<div class="ds-query">${q.by ? `<span class="dim">Saved by ${esc(q.by)}</span>` : ""}${field("Name", `ds.queries.${i}.name`, { placeholder: "Sales by month" })}
        <label class="field"><span>SQL</span><textarea class="inp mono" rows="3" id="f-dsq-${i}" data-bind="ds.queries.${i}.sql">${esc(q.sql)}</textarea></label>
        <div class="row end"><button class="btn small danger" data-act="ds-unquery" data-id="${i}">Remove</button><button class="btn small" data-act="ds-save-query" data-id="${i}">${q.id ? "Save" : "Add"}</button></div></div>`).join("")}
      <div><button class="btn small" data-act="ds-add-query">Add a saved query</button></div></section>` : "";
  return `<section class="card stack ds-form" aria-label="${f.id ? "Change the data source" : "Add a data source"}">
    <h2>${f.id ? `Change ${esc(f.name)}` : "Add a data source"}</h2>${trustNote()}
    <label class="field"><span>Kind</span><select class="inp" data-bind="ds.form.kind">${kinds}</select></label>
    ${field("Name people see", "ds.form.name", { placeholder: "Shop database" })}
    ${file ? field(f.kind === "csv" ? "The CSV file, or a folder of them, on this computer" : "The file on this computer", "ds.form.address", { placeholder: f.kind === "csv" ? "/srv/data/prices" : "/srv/data/stock.sqlite" })
      : `<div class="row">${field("Address", "ds.form.address", { placeholder: { postgres: "db.office.lan:5432", mysql: "db.office.lan:3306", mssql: "db.office.lan:1433" }[f.kind] })}${field("Database", "ds.form.database", { placeholder: "shop" })}</div>
      <div class="row">${field("User for reading", "ds.form.read_user", { placeholder: "calc_reader", auto: "off" })}${field(pwLabel(f, f.read_password_set), "ds.form.read_password", { type: "password", auto: "new-password" })}</div>
      <div class="row">${field("User for changes (only if tables can be edited)", "ds.form.write_user", { placeholder: "calc_editor", auto: "off" })}${field(pwLabel(f, f.write_password_set), "ds.form.write_password", { type: "password", auto: "new-password" })}</div>
      ${tlsFields(f)}
      ${f.kind === "mssql" ? '<p class="note warn-note" role="note">For SQL Server, the reading user’s own rights in the database are the only real protection. Give it a login that can only read.</p>' : ""}`}
    ${localField(f)}
    <div class="row"><button class="btn" data-act="ds-test">Test it</button></div>${test}
    <fieldset class="stack"><legend>Who else can write queries</legend>${writers}</fieldset>
    <div class="row">${field("Longest a query may run (seconds)", "ds.form.time_limit", { type: "number" })}${field("Most rows a sheet may load", "ds.form.row_limit", { type: "number" })}</div>
    ${file ? "" : `<fieldset class="stack"><legend>Tables people may edit${f.write_user ? ` (only those ${esc(f.write_user)} can change)` : ": set a user for changes first"}</legend>
      ${tables}${f.write_user ? `<div><button class="btn small" data-act="ds-table">Add a table</button></div>` : ""}
      ${f.editable.length ? '<p class="dim">Changes wait in Calc until Save. A change is refused if someone else changed that cell first.</p>' : ""}</fieldset>`}
    <div class="row end"><button class="btn" data-act="ds-cancel">Cancel</button><button class="btn main" data-act="ds-save">${f.id ? "Save" : "Add it"}</button></div>
  </section>${queries}`;
}

export function dataPage() {
  const { state, shell, errorBox } = $;
  const ds = state.ds;
  const add = ds.form ? "" : '<button class="btn main" data-act="ds-add">Add a data source</button>';
  return shell(`<div class="head"><h1>Data sources</h1>${add}</div><p class="dim">Database connections for Calc</p>${errorBox()}
    ${ds.form ? form() : `<div class="stack">${ds.list.map(sourceRow).join("") || '<p class="dim">None yet. A data source lets Calc sheets show what’s in a database, and change the tables you allow.</p>'}</div>${foldersCard()}`}`);
}

function body(f) {
  const out = {
    id: f.id ?? null, name: f.name, kind: f.kind, address: f.address || "", database: f.database || "",
    read_user: f.read_user || "", read_password: f.read_password || "", write_user: f.write_user || "", write_password: f.write_password || "",
    time_limit: Number(f.time_limit) || 15, row_limit: Number(f.row_limit) || 50000,
    writers: f.writers.map((x) => ({ user: x.user ?? undefined, group: x.group ?? undefined })),
    editable: f.editable.filter((e) => e.table?.trim()).map((e) => ({ table: e.table.trim(), key: split(e.keyText), locked: split(e.lockedText), add: e.add !== false, delete: !!e.delete })),
    notes: f.notes || {}, tls: f.tls ?? "require", ca_cert: f.ca_cert || "",
  };
  if (isAdmin()) out.allow_local = !!f.allow_local;
  if (isFile(f.kind)) Object.assign(out, { database: "", read_user: "", write_user: "", editable: [] });
  return out;
}

async function loadQueries(id) {
  $.state.ds.queries = id ? await $.api("GET", `/v1/admin/data/${id}/queries`) : [];
}

export const dataActs = {
  "ds-add"() {
    const ds = $.state.ds;
    ds.form = { kind: "postgres", name: "", writers: [], time_limit: "15", row_limit: "50000", editable: [], tls: "require", ca_cert: "", allow_local: false };
    ds.test = null;
    ds.queries = [];
    $.render();
  },
  async "ds-edit"(el) {
    const ds = $.state.ds;
    const s = ds.list.find((x) => x.id === Number(el.dataset.id));
    ds.form = { ...s, orig: { kind: s.kind, address: s.address }, read_password: "", write_password: "", time_limit: String(s.time_limit), row_limit: String(s.row_limit), writers: (s.writers || []).map((x) => ({ ...x })), editable: s.editable.map((e) => ({ table: e.table, keyText: words(e.key), lockedText: words(e.locked), add: e.add !== false, delete: !!e.delete })) };
    ds.test = null;
    await loadQueries(s.id).catch((e) => ($.state.error = e.message));
    $.render();
  },
  "ds-local"(el) {
    $.state.ds.form.allow_local = el.checked;
    $.render();
  },
  "ds-save-folders"() {
    return $.busy(async () => {
      const ds = $.state.ds;
      ds.folders = await $.api("PUT", "/v1/admin/data/folders", { folders: ds.foldersText.split("\n").map((x) => x.trim()).filter(Boolean) });
      ds.foldersText = ds.folders.folders.join("\n");
      $.toast("Folders saved");
    });
  },
  "ds-cancel"() {
    $.state.ds.form = null;
    $.state.error = "";
    $.render();
  },
  "ds-writer"(el) {
    const [k, raw] = el.value.split(":");
    if (!raw) return;
    const id = Number(raw);
    const dir = $.state.ds.dir;
    const name = k === "g" ? dir.groups.find((g) => g.id === id)?.name : (({ display_name, username }) => display_name || username)(dir.people.find((p) => p.id === id) || {});
    $.state.ds.form.writers.push(k === "g" ? { group: id, name } : { user: id, name });
    $.render();
  },
  "ds-unwriter"(el) {
    $.state.ds.form.writers.splice(Number(el.dataset.id), 1);
    $.render();
  },
  "ds-table"() {
    const f = $.state.ds.form;
    f.editable.push({ table: "", keyText: "id", lockedText: "", add: true, delete: false });
    $.render();
  },
  "ds-rowflag"(el) {
    $.state.ds.form.editable[Number(el.dataset.id)][el.dataset.flag] = el.checked;
    $.render();
  },
  "ds-untable"(el) {
    $.state.ds.form.editable.splice(Number(el.dataset.id), 1);
    $.render();
  },
  async "ds-test"() {
    const ds = $.state.ds;
    ds.test = { busy: true };
    $.render();
    try {
      ds.test = await $.api("POST", "/v1/admin/data/test", body(ds.form));
    } catch (e) {
      ds.test = { ok: false, message: e.message };
    }
    $.render();
  },
  "ds-save"() {
    return $.busy(async () => {
      const ds = $.state.ds;
      const f = ds.form;
      if (!f.name?.trim()) throw new Error("Give it a name people will recognise.");
      if (!f.address?.trim()) throw new Error(isFile(f.kind) ? "Which file on this computer?" : "What’s the database’s address?");
      if (!isFile(f.kind) && moved(f) && ((f.read_password_set && !f.read_password) || (f.write_password_set && f.write_user && !f.write_password))) throw new Error("Type the password again for the new address.");
      if (f.editable.some((e) => e.table?.trim() && !split(e.keyText).length)) throw new Error("Each editable table needs the column (or columns) that find its rows, like id.");
      await $.api("POST", "/v1/admin/data", body(f));
      ds.form = null;
      await loadData();
      $.toast(f.id ? "Saved" : "Added: whoever writes queries on it sees it in Calc");
    });
  },
  "ds-delete"(el) {
    const s = $.state.ds.list.find((x) => x.id === Number(el.dataset.id));
    $.state.dialog = { kind: "confirm", title: `Remove ${s.name}?`, text: "Its saved queries go too. Calc sheets made from it keep what they last showed but can’t refresh.", yes: "Remove", then: async () => {
      await $.api("DELETE", `/v1/admin/data/${s.id}`);
      $.state.dialog = null;
      await loadData();
    } };
    $.render();
  },
  async "ds-log"(el) {
    const ds = $.state.ds;
    const id = Number(el.dataset.id);
    if (ds.logFor === id) {
      ds.logFor = null;
      ds.log = null;
    } else {
      ds.logFor = id;
      ds.log = null;
      try { ds.log = await $.api("GET", `/v1/admin/data/${id}/log`); } catch (e) { $.state.error = e.message; }
    }
    $.render();
  },
  "ds-add-query"() {
    $.state.ds.queries.push({ name: "", sql: "" });
    $.render();
  },
  "ds-save-query"(el) {
    return $.busy(async () => {
      const ds = $.state.ds;
      const q = ds.queries[Number(el.dataset.id)];
      if (!q.name?.trim() || !q.sql?.trim()) throw new Error("A saved query needs a name and its SQL.");
      await $.api("POST", `/v1/admin/data/${ds.form.id}/queries`, { id: q.id ?? null, name: q.name.trim(), sql: q.sql });
      await loadQueries(ds.form.id);
      $.toast("Query saved");
    });
  },
  "ds-unquery"(el) {
    return $.busy(async () => {
      const ds = $.state.ds;
      const i = Number(el.dataset.id);
      const q = ds.queries[i];
      if (q.id) await $.api("DELETE", `/v1/admin/data/${ds.form.id}/queries/${q.id}`);
      ds.queries.splice(i, 1);
    });
  },
};
