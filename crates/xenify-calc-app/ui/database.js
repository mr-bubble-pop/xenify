// Data › From a database: sheets from a server's data sources. A query
// sheet shows what a query gave, locked, with the query in a panel above
// it (the SQL, or a saved query's name, and the values for its :names, from
// cells or typed in). The server runs it with the source's own user and
// limits; Calc only ever sees the answer. Query sheets load new values when
// the book opens (unless set to only with Refresh), and Refresh runs it again.
//
// Who writes queries on a source (their own SQL, or a saved one) is set on
// the server, by whoever sets up database connections. When they run a
// query (or load an editable table), the server hands back a ticket for
// exactly that, which the sheet keeps: anyone who has the file can then load
// new values with it, but not change the query. What a sheet last showed
// stays in the file for everyone.

import { ask as askText, h, icon, toast, dialogKeys } from './ui.js';
import { openLink } from './platform.js';

const KIND = { postgres: 'PostgreSQL', mysql: 'MySQL', mssql: 'SQL Server', sqlite: 'SQLite file on the server', csv: 'CSV on the server' };
const LEVEL = { write: 'you write queries', manage: 'you set it up' };
const LOADS = [['open', 'When the file opens, and with Refresh'], ['refresh', 'Only with Refresh'], ['every15', 'Every 15 minutes while open']];
const CELL = /^(('[^']+'|[\p{L}\p{N}_. ]+)!)?\$?[A-Za-z]{1,3}\$?\d{1,7}$/u;

/// The :names a query uses, in order (outside strings, quoted names and comments; not :: casts).
export function paramNames(sql) {
  const bare = String(sql || '').replace(/'(?:[^']|'')*'|"(?:[^"]|"")*"|`[^`]*`|--[^\n]*|\/\*[\s\S]*?\*\//g, ' ');
  const out = [];
  for (const m of bare.matchAll(/(^|[^:\w]):([A-Za-z_]\w*)/g)) if (!out.includes(m[2])) out.push(m[2]);
  return out;
}

/// A name quoted for `kind`'s SQL.
function quote(kind, name) {
  if (kind === 'mysql') return `\`${name.replace(/`/g, '``')}\``;
  if (kind === 'mssql') return `[${name.replace(/]/g, ']]')}]`;
  return /^[a-z_][a-z0-9_]*$/.test(name) ? name : `"${name.replace(/"/g, '""')}"`;
}
const quoteTable = (kind, t) => t.split('.').map((p) => quote(kind, p)).join('.');

const ago = (iso) => {
  if (!iso) return 'not loaded yet';
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return 'loaded just now';
  if (s < 3600) return `loaded ${Math.round(s / 60)} min ago`;
  if (s < 86400) return `loaded ${Math.round(s / 3600)} h ago`;
  return `loaded ${new Date(iso).toLocaleString()}`;
};
const sentence = (s) => { s = String(s || '').trim(); return s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? '' : '.') : s; };
const titleCase = (s) => String(s).replace(/[_.]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()).trim();

export class Databases {
  constructor(d) {
    this.d = d;   // { call, run, server, sheet, goSheet, openServer, grid, tab, editors }
    this.el = document.getElementById('querypanel');
    // By book and sheet (sheet numbers repeat from book to book): at(sheet).
    this.hiddenFor = new Set();   // sheets with their query tucked away
    this.status = new Map();      // sheet -> { ms, error, busy }
    this.drafts = new Map();      // sheet -> { sql, params, load } being changed
    this.levels = null;           // { server, by: source id -> level } once asked
    // "Every 15 minutes while open".
    setInterval(() => this.every15(), 60_000);
    // "loaded 2 min ago" stays true.
    setInterval(() => { if (!this.el.hidden) this.render(); }, 30_000);
  }

  get server() { return this.d.server; }

  /// The key for sheet `sheet` of book `tab` (the front one).
  at(sheet, tab = this.d.tab()) { return `${tab}:${sheet}`; }

  /// What you may do with source `id` on the signed-in server ('write',
  /// 'manage'; '' when you don't write queries on it; null until known).
  levelOf(id) {
    const url = this.server.session && this.server.url;
    if (!url) return null;
    if (this.levels?.server === url) return this.levels.by[id] || '';
    if (!this.levels?.asking) {
      this.levels = { asking: true };
      this.server.api('GET', '/v1/data').then((list) => {
        this.levels = { server: url, by: Object.fromEntries(list.map((x) => [x.id, x.level])) };
        if (!this.el.hidden) this.render();
      }).catch(() => { this.levels = null; });
    }
    return null;
  }

  /// Keeps sheet `sheet`'s own SQL on the server as a saved query, for
  /// whoever writes queries on the source to pick; the sheet then uses it.
  async saveForOthers(sheet) {
    const q = this.d.call('query', { sheet });
    if (!q || q.def.sql == null) return;
    const def = this.defWith(sheet, q.def);
    if (!def.sql.trim()) { toast('Write the query first.'); return; }
    const name = (await askText('Save the query for others', `Its name in ${def.source_name}`, def.query_name === 'Query' ? '' : def.query_name, 'Save'))?.trim();
    if (!name) return;
    try {
      const { id } = await this.server.api('POST', `/v1/data/${def.source}/queries`, { name, sql: def.sql });
      const saved = { ...def, query: id, query_name: name, sql: undefined, asks: undefined };
      this.d.run('set_query', { sheet, def: saved });
      this.drafts.delete(this.at(sheet));
      this.levels = null;
      toast(`Saved in ${def.source_name}: whoever writes queries there can pick it now.`);
      // Its ticket, for whoever has the file.
      await this.refresh(sheet);
    } catch (e) {
      toast(sentence(e.message || e));
    }
  }

  /// Shows the panel when the sheet shown is a query sheet (hides it otherwise).
  sync() {
    let q = null;
    try { q = this.d.call('query', { sheet: this.d.sheet() }); } catch {}
    const was = this.el.hidden;
    this.q = q;
    this.el.hidden = !q;
    if (q) this.render();
    if (was !== this.el.hidden) this.d.grid.resize();
  }

  // ----- The panel -----

  draft(sheet, def) {
    const k = this.at(sheet);
    if (!this.drafts.has(k)) {
      this.drafts.set(k, {
        sql: def.sql ?? null,
        params: Object.fromEntries(Object.entries(def.params || {}).map(([k, p]) => [k, p.cell ?? (p.value == null ? '' : String(p.value))])),
      });
    }
    return this.drafts.get(k);
  }

  /// The definition with what's typed in the panel.
  defWith(sheet, def) {
    const dr = this.draft(sheet, def);
    const names = def.sql != null ? paramNames(dr.sql) : Object.keys(def.params || {}).concat(def.asks || []);
    const params = {};
    for (const n of new Set(names)) {
      const v = String(dr.params[n] ?? '').trim();
      params[n] = CELL.test(v) ? { cell: v } : { value: v === '' ? null : (/^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v) };
    }
    return { ...def, sql: def.sql != null ? dr.sql : def.sql, params };
  }

  render() {
    const sheet = this.d.sheet();
    const q = this.q;
    if (!q) return;
    const def = q.def;
    const st = this.status.get(this.at(sheet)) || {};
    const dr = this.draft(sheet, def);
    const mine = this.server.url === def.server && this.server.session;
    const srvName = mine ? this.server.name : def.server.replace(/^https?:\/\//, '');
    const src = h('span', { class: 'q-src', title: `${def.source_name} on ${srvName}` }, h('span', { html: icon('database', 15) }), `${def.source_name} · read-only`);
    const tableName = h('input', { class: 'field q-table', value: def.table, 'aria-label': 'Table name in formulas', spellcheck: 'false' });
    tableName.addEventListener('change', () => {
      const to = tableName.value.trim();
      if (to && to !== def.table) this.d.run('rename_table', { name: def.table, to });
      this.sync();
    });
    tableName.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') tableName.blur(); if (e.key === 'Escape') { tableName.value = def.table; tableName.blur(); } });
    const hidden = this.hiddenFor.has(this.at(sheet));
    const toggle = h('button', { class: 'btn small', onClick: () => { if (hidden) this.hiddenFor.delete(this.at(sheet)); else this.hiddenFor.add(this.at(sheet)); this.render(); this.d.grid.resize(); } }, hidden ? 'Show query' : 'Hide query');
    const refresh = h('button', { class: 'btn small', disabled: st.busy || null, onClick: () => this.refresh(sheet) }, h('span', { html: icon('refresh', 15) }), 'Refresh');
    const facts = [`${def.rows === 1 ? '1 row' : `${(def.rows || 0).toLocaleString()} rows`}`, st.ms != null ? `${(st.ms / 1000).toFixed(2)} s` : null, ago(def.refreshed)].filter(Boolean).join(' · ');
    const line = h('p', { class: 'q-facts', role: 'status' },
      st.busy ? 'Running…' : facts,
      def.truncated && !st.busy ? h('span', { class: 'q-warn' }, ` · only the first ${(def.rows || 0).toLocaleString()} rows (the source’s limit)`) : null);
    const problem = st.error ? h('p', { class: 'q-error', role: 'alert' }, st.error,
      st.signIn ? h('button', { class: 'btn small', onClick: () => this.d.openServer() }, 'Sign in…') : null) : null;

    if (hidden) {
      this.el.replaceChildren(...[h('div', { class: 'q-bar' }, src, h('span', { class: 'q-label' }, 'Table'), h('b', { class: 'q-tname' }, def.table), line, h('span', { class: 'grow' }), refresh, toggle), problem].filter(Boolean));
      return;
    }

    // The query: SQL to change, or a saved query by name.
    let query;
    if (def.sql != null) {
      const sql = h('textarea', { class: 'q-sql', spellcheck: 'false', 'aria-label': 'The query (SQL)', rows: Math.min(10, Math.max(3, (dr.sql || '').split('\n').length + 1)), placeholder: 'SELECT … FROM …' });
      sql.value = dr.sql || '';
      sql.addEventListener('input', () => { dr.sql = sql.value; this.renderValues(sheet, def); });
      sql.addEventListener('keydown', (e) => {
        e.stopPropagation();
        if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); this.refresh(sheet, true); }
        if (e.key === 'Tab') { e.preventDefault(); sql.setRangeText('  ', sql.selectionStart, sql.selectionEnd, 'end'); }
      });
      const lvl = mine ? this.levelOf(def.source) : null;
      if (lvl === '') {
        sql.readOnly = true;
        query = h('div', { class: 'q-sqlbox' }, sql, h('span', { class: 'q-hint' }, `Only people who write queries on ${def.source_name} can change it; you can load new values.`));
      } else {
        query = lvl ? h('div', { class: 'q-sqlbox' }, sql, h('button', { class: 'btn small', title: `Others who write queries on ${def.source_name} can then pick it by name`, onClick: () => this.saveForOthers(sheet) }, 'Save as a query…')) : sql;
      }
    } else {
      query = h('div', { class: 'q-saved' }, h('span', { class: 'q-label' }, 'Saved query'), h('b', {}, def.query_name),
        h('span', { class: 'q-hint' }, `Saved in ${def.source_name}; whoever made it, or sets up ${def.source_name}, can change it.`));
    }
    this.valuesBox = h('div', { class: 'q-values' });
    const load = h('select', { class: 'field', 'aria-label': 'Load new values', onChange: (e) => this.setLoad(sheet, e.target.value) },
      LOADS.map(([v, l]) => h('option', { value: v, selected: (def.load || 'open') === v }, l)));
    const runBtn = h('button', { class: 'btn main', disabled: st.busy || null, onClick: () => this.refresh(sheet, true) }, h('span', { html: icon('play', 14) }), def.sql != null ? 'Run · Ctrl+Enter' : 'Run');
    this.el.replaceChildren(
      h('div', { class: 'q-main' },
        h('div', { class: 'q-head' }, src, h('label', { class: 'q-label' }, 'Table name in formulas', tableName), h('span', { class: 'grow' }), toggle),
        query,
        h('p', { class: 'q-hint' }, `Results pass through ${srvName}, which can see them. Other sheets use them like any table: =SUM(${def.table}[…]).`)),
      h('div', { class: 'q-side' },
        this.valuesBox,
        h('div', { class: 'q-group' }, h('span', { class: 'q-label' }, 'Load new values'), load),
        h('div', { class: 'q-run' }, runBtn, refresh),
        line, problem));
    this.renderValues(sheet, def);
  }

  /// The values for the query's :names: a cell (shown with what it holds now) or typed in.
  renderValues(sheet, def) {
    if (!this.valuesBox) return;
    const dr = this.draft(sheet, def);
    const d2 = this.defWith(sheet, def);
    const names = Object.keys(d2.params);
    let now = {};
    try { now = this.d.call('query_values', { def: d2 }); } catch {}
    this.valuesBox.replaceChildren(...(names.length ? [h('span', { class: 'q-label' }, 'Values for the query'), ...names.map((n) => {
      const inp = h('input', { class: 'field', value: dr.params[n] ?? '', placeholder: 'a value, or a cell like B2', 'aria-label': `Value for :${n}`, spellcheck: 'false' });
      const shown = h('span', { class: 'q-now' }, CELL.test(String(dr.params[n] || '').trim()) ? String(now[n] ?? '') : '');
      inp.addEventListener('input', () => { dr.params[n] = inp.value; const d3 = this.defWith(sheet, def); let v = {}; try { v = this.d.call('query_values', { def: d3 }); } catch {} shown.textContent = CELL.test(inp.value.trim()) ? String(v[n] ?? '') : ''; });
      inp.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') this.refresh(sheet, true); });
      return h('label', { class: 'q-param' }, h('code', {}, `:${n}`), inp, shown);
    })] : [h('span', { class: 'q-label' }, 'Values for the query'), h('span', { class: 'q-hint' }, def.sql != null ? 'Write :name in the query for a value from a cell, like WHERE year = :year.' : 'This query asks for none.')]));
  }

  setLoad(sheet, load) {
    const q = this.d.call('query', { sheet });
    if (!q) return;
    // Recorded with the answer it has (no new run).
    this.d.run('set_query', { sheet, def: { ...q.def, load } });
  }

  // ----- Running -----

  /// Whether this server is the one `def` came from, signed in; or why not.
  ready(def) {
    if (!this.server.session || this.server.url !== def.server) {
      return `Sign in to ${def.server.replace(/^https?:\/\//, '')} to load new values.`;
    }
    return '';
  }

  /// Runs `def` on the server: { answer, ms }.
  async ask(def, params) {
    const t0 = performance.now();
    const body = def.query != null ? { query: def.query, params } : { sql: def.sql, params };
    if (def.ticket) body.ticket = def.ticket;
    const answer = await this.server.api('POST', `/v1/data/${def.source}/query`, body);
    return { answer, ms: performance.now() - t0 };
  }

  /// Runs the query of query sheet `sheet` again (with what's typed in the panel when `edited`).
  async refresh(sheet, edited = false, quiet = false) {
    const q = this.d.call('query', { sheet });
    if (!q) return false;
    const def = edited ? this.defWith(sheet, q.def) : q.def;
    const why = this.ready(def);
    const st = { busy: false };
    const tab = this.d.tab();
    const k = this.at(sheet, tab);
    if (why) {
      this.status.set(k, { error: why, signIn: true });
      if (!quiet) this.after(sheet);
      return false;
    }
    if (def.sql != null && !def.sql.trim()) {
      this.status.set(k, { error: 'Write a query first.' });
      this.after(sheet);
      return false;
    }
    this.status.set(k, { busy: true });
    this.after(sheet);
    try {
      const params = edited ? this.d.call('query_values', { def }) : q.params;
      const { answer, ms } = await this.ask(def, params);
      // The book was switched away from meanwhile (its Refresh works again).
      if (this.d.tab() !== tab) { this.status.delete(k); return false; }
      // A new ticket (when you write queries) is kept for whoever has the file.
      this.d.run('fill_query', { sheet, def: { ...def, ticket: answer.ticket || def.ticket || '' }, answer, refreshed: new Date().toISOString() });
      st.ms = ms;
      this.drafts.delete(k);
    } catch (e) {
      st.error = sentence(e.message || e);
      if (e.status === 403) st.error += ' The sheet keeps what it last showed.';
    }
    this.status.set(k, st);
    this.after(sheet);
    return !st.error;
  }

  after(sheet) {
    if (this.d.sheet() === sheet) this.sync();
  }

  /// When a book opens: its query sheets set to load then (quietly; a
  /// sheet says why when it couldn't).
  async opened() {
    let all = [];
    try { all = this.d.call('queries'); } catch {}
    const due = all.filter((q) => (q.def.load || 'open') !== 'refresh');
    if (!due.length) return;
    let failed = 0;
    for (const q of due) if (!(await this.refresh(q.sheet, false, true))) failed++;
    if (failed && !this.server.session) toast(`This book has data from ${due[0].def.server.replace(/^https?:\/\//, '')}: sign in (File › Xenify server) to load new values.`);
    else if (failed) toast(failed === 1 ? 'A query sheet couldn’t load new values: it says why.' : `${failed} query sheets couldn’t load new values: they say why.`);
  }

  async every15() {
    let all = [];
    try { all = this.d.call('queries'); } catch { return; }
    for (const q of all) {
      if (q.def.load !== 'every15' || this.ready(q.def)) continue;
      if (Date.now() - new Date(q.def.refreshed || 0).getTime() >= 15 * 60_000) await this.refresh(q.sheet, false, true);
    }
  }

  /// Refreshes every query sheet in the book.
  async refreshAll() {
    const all = this.d.call('queries');
    if (!all.length) { toast('This book has no query sheets.'); return; }
    let ok = 0;
    for (const q of all) if (await this.refresh(q.sheet)) ok++;
    toast(ok === all.length ? `Loaded new values for ${ok === 1 ? 'the query sheet' : `${ok} query sheets`}` : `${all.length - ok} of ${all.length} couldn’t load: they say why`);
  }

  // ----- Data › From a database -----

  /// Which columns of the table picked come in (all at first); key columns
  /// stay in when editing (changes find their row by them).
  columnPicker(pick, keys) {
    const cols = pick.choice.columns;
    const isKey = (n) => keys.some((k) => k.toLowerCase() === n.toLowerCase());
    if (!pick.cols) pick.cols = new Set(cols.map((c) => c.name));
    const set = (names) => { pick.cols = new Set([...names, ...cols.filter((c) => isKey(c.name)).map((c) => c.name)]); this.redraw(); };
    return h('div', { class: 'db-cols' },
      h('div', { class: 'db-cols-head' }, h('span', { class: 'q-label' }, `Columns (${pick.cols.size} of ${cols.length})`),
        h('button', { class: 'btn small plain', onClick: () => set(cols.map((c) => c.name)) }, 'All'),
        h('button', { class: 'btn small plain', onClick: () => set([]) }, 'None')),
      h('div', { class: 'db-col-list' }, cols.map((c) => {
        const key = isKey(c.name);
        const box = h('input', { type: 'checkbox', checked: pick.cols.has(c.name), disabled: key || null });
        box.addEventListener('change', () => { if (box.checked) pick.cols.add(c.name); else pick.cols.delete(c.name); this.redraw(); });
        return h('label', { class: 'db-col', title: key ? 'A key column: it stays, so changes can find their row' : (c.note || null) }, box, h('span', {}, c.name), key ? h('span', { class: 'chip blue' }, 'key') : c.type ? h('span', { class: 'q-hint' }, c.type) : null);
      })));
  }

  async add() {
    if (!this.server.session) {
      toast('Sign in to a Xenify server first: data sources are set up there.');
      this.d.openServer(() => { if (this.server.session) this.add(); });
      return;
    }
    let sources;
    // Whoever may set up database connections (admins too) gets a way to the server's Data sources page.
    const [listed, setUp] = await Promise.allSettled([this.server.api('GET', '/v1/data'), this.server.api('GET', '/v1/admin/data')]);
    if (listed.status === 'rejected') { toast(sentence(listed.reason?.message || listed.reason)); return; }
    sources = listed.value;
    const canSetUp = setUp.status === 'fulfilled';
    const setUpPage = () => openLink(`${this.server.url}/admin/#/data`);
    const lookAgain = async () => {
      try { sources = await this.server.api('GET', '/v1/data'); } catch (e) { msg.textContent = sentence(e.message || e); return; }
      pick.src = sources.find((s) => s.id === pick.src?.id) || sources[0] || null;
      pick.choice = null; pick.tables = null;
      drawList(); drawRight();
    };
    const pick = { src: sources[0] || null, how: 'read', choice: null, tables: null, values: {} };
    const back = h('div', { class: 'modal-back' });
    const close = () => { back.remove(); unkey(); };
    const list = h('nav', { class: 'db-sources', 'aria-label': 'Data sources' });
    const right = h('div', { class: 'db-right' });
    const msg = h('p', { class: 'q-error', role: 'alert' });
    const addBtn = h('button', { class: 'btn main', onClick: () => go() }, 'Add sheet');
    const box = h('div', { class: 'modal db-modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Add a sheet from a database' },
      h('div', { class: 'db-top' }, h('h2', {}, 'Add a sheet from a database'), h('button', { class: 'icon-btn small', 'aria-label': 'Close', html: icon('close', 15), onClick: close })),
      h('div', { class: 'db-body' }, list, right));

    const drawList = () => {
      list.replaceChildren(h('span', { class: 'q-label' }, `On ${this.server.name}`),
        ...sources.map((s) => {
          const what = [KIND[s.kind] || s.kind, LEVEL[s.level] || (s.own_queries ? 'your queries' : 'saved queries'), s.can_change && s.editable.length ? `${s.editable.length} editable` : (s.can_change ? null : 'read-only')].filter(Boolean).join(' · ');
          return h('button', { class: `db-src${pick.src === s ? ' on' : ''}`, 'aria-pressed': String(pick.src === s), onClick: () => { pick.src = s; pick.choice = null; pick.tables = null; drawList(); drawRight(); } },
            h('span', { class: 'ico', html: icon(s.kind === 'csv' || s.kind === 'sqlite' ? 'file' : 'database', 17) }),
            h('span', {}, h('b', {}, s.name), h('br'), h('span', { class: 'q-hint' }, what)));
        }),
        !sources.length && canSetUp ? ''
          : canSetUp ? h('div', { class: 'db-setup' }, h('button', { class: 'btn small', onClick: setUpPage }, 'Set up connections…'), h('button', { class: 'btn small plain', onClick: lookAgain }, 'Look again'))
          : h('p', { class: 'q-hint' }, 'Sources you write queries on. Whoever sets up database connections picks who does.'));
    };

    const drawRight = () => {
      const s = pick.src;
      if (!s && canSetUp) {
        right.replaceChildren(h('p', {}, `${this.server.name} has no database connections yet. You can set them up: add one on the server’s Data sources page (it opens in your browser), pick who may write queries on it, then come back and press Look again.`),
          msg,
          h('div', { class: 'modal-actions' }, h('button', { class: 'btn', onClick: close }, 'Close'), h('button', { class: 'btn', onClick: lookAgain }, 'Look again'), h('button', { class: 'btn main', onClick: setUpPage }, 'Set up a connection…')));
        return;
      }
      if (!s) {
        right.replaceChildren(h('p', {}, `You don’t write queries on any of ${this.server.name}’s data sources. Ask whoever sets up database connections: they add them in the server’s admin pages (Data sources) and pick who writes queries. Sheets someone else adds to a file load new values for you anyway.`),
          h('div', { class: 'modal-actions' }, h('button', { class: 'btn', onClick: close }, 'Close')));
        return;
      }
      const card = (how, ico, title, text, enabled = true) => h('button', { class: `db-card${pick.how === how ? ' on' : ''}`, disabled: !enabled || null, 'aria-pressed': String(pick.how === how),
        onClick: () => { pick.how = how; pick.choice = null; drawRight(); } },
      h('span', { class: 'db-card-head' }, h('span', { class: 'ico', html: icon(ico, 17) }), h('b', {}, title)), h('span', { class: 'q-hint' }, text));
      const editable = s.can_change && s.editable.length > 0;
      const cards = h('div', { class: 'db-cards' },
        card('read', 'search', 'Read with a query', 'A read-only sheet from SQL. Totals and groups (SUM, GROUP BY) are fine. Loads when the file opens.'),
        card('edit', 'table', 'Edit a table', editable ? 'Rows of one table. Changes wait until you save, then go to the database with your name on them.' : 'No tables here can be edited. Whoever sets up the source chooses which can.', editable));
      const kids = [cards];
      if (pick.how === 'edit') {
        if (!pick.tables) {
          pick.tables = 'loading';
          this.server.api('GET', `/v1/data/${s.id}/tables`).then((t) => { pick.tables = t; drawRight(); }).catch(() => { pick.tables = []; drawRight(); });
        }
        const info = Array.isArray(pick.tables) ? pick.tables : [];
        kids.push(h('span', { class: 'q-label' }, 'Tables you can edit'), h('div', { class: 'db-tables' }, s.editable.map((e) => {
          const t = info.find((x) => x.table.toLowerCase() === e.table.toLowerCase());
          const on = pick.choice?.edit === e.table;
          return h('button', { class: `db-table${on ? ' on' : ''}`, title: t?.note || null, onClick: () => { pick.choice = { edit: e.table, columns: t?.columns || [], key: e.key || [] }; pick.cols = null; drawRight(); } },
            h('b', {}, e.table), t ? h('span', { class: 'chip' }, t.columns.slice(0, 4).map((c) => c.name).join(', ') + (t.columns.length > 4 ? '…' : '')) : null,
            h('span', { class: 'chip blue' }, e.delete ? 'edit, add, delete' : e.add !== false ? 'edit, add' : 'edit'));
        })));
        if (pick.choice?.edit && pick.choice.columns.length) kids.push(this.columnPicker(pick, pick.choice.key));
      }
      if (pick.how === 'read' && s.own_queries) {
        if (!pick.tables) {
          pick.tables = 'loading';
          this.server.api('GET', `/v1/data/${s.id}/tables`).then((t) => { pick.tables = t; drawRight(); }).catch((e) => { pick.tables = []; msg.textContent = sentence(e.message || e); drawRight(); });
        }
        const tables = Array.isArray(pick.tables) ? pick.tables : [];
        kids.push(h('span', { class: 'q-label' }, 'Start from a table (writes the query for you, change it after)'),
          pick.tables === 'loading' ? h('p', { class: 'q-hint' }, 'Looking…') : h('div', { class: 'db-tables' }, tables.slice(0, 60).map((t) => {
            const on = pick.choice?.table === t.table;
            const ed = s.editable.some((e) => e.table.toLowerCase() === t.table.toLowerCase());
            return h('button', { class: `db-table${on ? ' on' : ''}`, title: t.note || null, onClick: () => { pick.choice = { table: t.table, columns: t.columns }; pick.cols = null; drawRight(); } },
              h('b', {}, t.table), h('span', { class: 'chip' }, t.columns.slice(0, 4).map((c) => c.name).join(', ') + (t.columns.length > 4 ? '…' : '')), ed ? h('span', { class: 'chip blue' }, 'editable') : null);
          })));
        if (pick.choice?.table) kids.push(this.columnPicker(pick, []));
      }
      if (pick.how === 'read') {
        if (s.queries.length) {
          kids.push(h('span', { class: 'q-label' }, s.own_queries ? 'Or a saved query' : 'Saved queries'));
          for (const q of s.queries) {
            const on = pick.choice?.query === q.id;
            kids.push(h('button', { class: `db-query${on ? ' on' : ''}`, onClick: () => { pick.choice = { query: q.id, name: q.name, params: q.params, sql: q.sql }; drawRight(); } },
              h('b', {}, q.name), q.params.length ? h('span', { class: 'q-hint' }, `asks for: ${q.params.join(', ')}`) : null));
            if (on && q.params.length) {
              kids.push(h('div', { class: 'db-asks' }, q.params.map((n) => {
                const inp = h('input', { class: 'field', value: pick.values[n] ?? '', placeholder: 'a value, or a cell like Summary!B1', 'aria-label': `Value for :${n}` });
                inp.addEventListener('input', () => { pick.values[n] = inp.value; });
                return h('label', { class: 'q-param' }, h('code', {}, `:${n}`), inp);
              })));
            }
          }
        }
        if (s.own_queries) {
          kids.push(h('button', { class: `db-query blank${pick.choice?.blank ? ' on' : ''}`, onClick: () => { pick.choice = { blank: true }; drawRight(); } }, h('b', {}, 'Blank query')));
        }
      }
      addBtn.disabled = !pick.choice || (pick.cols && pick.cols.size === 0) || null;
      kids.push(msg, h('div', { class: 'db-foot' }, h('span', { class: 'q-hint' }, pick.how === 'edit' ? 'Changes wait until you save; then they go to the database with your name on them.' : 'Calc adds the sheet and runs it. Rename it on its tab.'), h('span', { class: 'grow' }),
        h('button', { class: 'btn', onClick: close }, 'Cancel'), addBtn));
      right.replaceChildren(...kids);
    };

    const go = async () => {
      const s = pick.src;
      const c = pick.choice;
      if (!c) return;
      if (c.edit) {
        addBtn.disabled = true;
        msg.textContent = 'Loading the rows…';
        const all = c.columns.length && pick.cols && pick.cols.size < c.columns.length ? null : [];
        const picked = all ? all : c.columns.filter((x) => pick.cols?.has(x.name)).map((x) => x.name);
        try { await this.d.editors().add(s, c.edit, picked); close(); } catch (e) { msg.textContent = sentence(e.message || e); addBtn.disabled = false; }
        return;
      }
      const def = { server: this.server.url, source: s.id, source_name: s.name, params: {}, load: 'open' };
      let name;
      if (c.query != null) {
        Object.assign(def, { query: c.query, query_name: c.name, table: c.name });
        for (const n of c.params) {
          const v = String(pick.values[n] ?? '').trim();
          def.params[n] = CELL.test(v) ? { cell: v } : { value: v === '' ? null : (/^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v) };
        }
        name = c.name;
      } else if (c.table) {
        const chosen = c.columns.filter((x) => !pick.cols || pick.cols.has(x.name));
        const cols = chosen.length === c.columns.length && c.columns.length > 12 ? '*' : chosen.map((x) => quote(s.kind, x.name)).join(', ');
        Object.assign(def, { sql: `SELECT ${cols}\nFROM ${quoteTable(s.kind, c.table)}`, query_name: titleCase(c.table.split('.').pop()), table: titleCase(c.table.split('.').pop()) });
        name = def.query_name;
      } else {
        Object.assign(def, { sql: '', query_name: 'Query', table: 'Query' });
        name = 'Query';
      }
      addBtn.disabled = true;
      msg.textContent = 'Running…';
      try {
        let answer;
        if (def.sql === '') answer = { columns: [['Answer', 'text']], rows: [], truncated: false };
        else {
          const values = this.d.call('query_values', { def });
          answer = (await this.ask(def, values)).answer;
        }
        const id = this.d.run('add_query', { def: { ...def, ticket: answer.ticket || '', refreshed: def.sql === '' ? '' : new Date().toISOString() }, answer, name, working: true });
        if (id == null) throw new Error('Calc couldn’t add the sheet.');
        close();
        this.d.goSheet(id);
        this.sync();
        if (def.sql === '') this.el.querySelector('.q-sql')?.focus();
      } catch (e) {
        msg.textContent = sentence(e.message || e);
        addBtn.disabled = false;
      }
    };

    this.redraw = drawRight;
    drawList();
    drawRight();
    back.append(box);
    back.addEventListener('pointerdown', (e) => { if (e.target === back) close(); });
    // Escape closes, Enter is Add sheet; other keys stay in the dialog too.
    const unkey = dialogKeys(back, box, close, () => addBtn);
    back.addEventListener('keydown', (e) => e.stopPropagation());
    document.getElementById('popups').append(back);
  }
}
