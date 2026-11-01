// Table editor sheets: rows of one database table that people change like
// any cells. Changes wait in the book (kept in recovery copies like any
// edit) until Save sends them; a cell someone else changed first waits for
// a choice, theirs or mine. The bar above the sheet says where the rows
// come from, which rows are shown, and what's not sent yet.

import { h, icon, toast, popup, closePopups, menu, item, modal } from './ui.js';

const sentence = (s) => { s = String(s || '').trim(); return s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? '' : '.') : s; };
const show = (v) => (v == null || v === '' ? '(empty)' : typeof v === 'number' ? v.toLocaleString() : String(v));
const plural = (n, one, many) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
const orderWords = (kind) => (kind === 'date' ? ['oldest first', 'newest first'] : kind === 'number' ? ['smallest first', 'biggest first'] : ['A to Z', 'Z to A']);
const titleCase = (s) => String(s).replace(/[_.]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()).trim();

export class Editors {
  constructor(d) {
    this.d = d;   // { call, run, server, sheet, goSheet, openServer, grid, tab, save }
    this.el = document.getElementById('editbar');
    this.box = h('div', { class: 'unsent', role: 'status', hidden: true });
    document.getElementById('sheet').append(this.box);
    this.busy = new Set();
    this.st = null;
  }

  get server() { return this.d.server; }

  /// Shows the bar and the cell states when the sheet shown is a table editor.
  sync() {
    let st = null;
    try { st = this.d.call('editor', { sheet: this.d.sheet() }); } catch {}
    const was = this.el.hidden;
    this.st = st;
    this.el.hidden = !st;
    const sheet = this.d.sheet();
    this.d.grid.stateMarks = st ? st.marks.map((m) => ({ ...m, sheet })) : [];
    if (st) this.render(); else this.box.hidden = true;
    if (was !== this.el.hidden) this.d.grid.resize();
  }

  render() {
    const st = this.st;
    const def = st.def;
    const sheet = this.d.sheet();
    const src = h('span', { class: 'q-src' }, h('span', { html: icon('database', 15) }), `${def.source_name} · ${def.table} · editable`);
    const chips = def.only.map(([col, v], k) => h('span', { class: 'chip big' }, `${col} is ${show(v)}`,
      h('button', { class: 'x', 'aria-label': `Show every ${col}`, onClick: () => this.reload(sheet, { only: def.only.filter((_, i) => i !== k) }) }, '×')));
    const addOnly = h('button', { class: 'btn small plain', onClick: (e) => this.pickOnly(e.currentTarget) }, 'Only rows where…');
    const order = h('button', { class: 'btn small', onClick: (e) => this.pickOrder(e.currentTarget) },
      def.order ? `${def.order[0]}, ${orderWords(def.columns.find(([c]) => c === def.order[0])?.[1])[def.order[1] ? 1 : 0]}` : `In ${def.key.join(', ')} order`);
    const total = def.total ?? st.rows;
    const shown = def.truncated ? `The first ${plural(st.rows, 'row', 'rows')} of ${total.toLocaleString()} (the source’s limit: narrow it with Only rows where)` : plural(st.rows, 'row', 'rows');
    const locked = [...def.key, ...def.locked];
    const refresh = h('button', { class: 'btn small', disabled: this.busy.has(sheet) || null, onClick: () => this.reload(sheet) }, h('span', { html: icon('refresh', 15) }), 'Refresh');
    const add = def.add ? h('button', { class: 'btn small', onClick: () => this.addRow() }, h('span', { html: icon('plus', 15) }), 'Add a row') : null;
    this.el.replaceChildren(h('div', { class: 'q-bar' },
      src, ...chips, addOnly, order, h('span', { class: 'q-facts' }, shown), h('span', { class: 'grow' }),
      h('span', { class: 'q-hint' }, `Can’t be changed here: ${locked.join(', ')}${def.delete ? '' : ' · rows can’t be deleted'}`), add, refresh));
    this.renderBox();
  }

  /// "3 changes not sent yet. Save sends them to the database."
  renderBox() {
    const st = this.st;
    const n = st.updates + st.inserts + st.deletes;
    if (!n && !st.waiting) { this.box.hidden = true; return; }
    const parts = [];
    if (n) parts.push(`${plural(n, 'change', 'changes')} not sent yet${st.inserts ? ` (${plural(st.inserts, 'new row', 'new rows')}${st.deletes ? `, ${plural(st.deletes, 'deleted', 'deleted')}` : ''})` : st.deletes ? ` (${plural(st.deletes, 'row', 'rows')} deleted)` : ''}`);
    if (st.waiting) parts.push(`${plural(st.waiting, 'cell needs', 'cells need')} a choice`);
    this.box.replaceChildren(...[h('span', {}, `${parts.join(' · ')}. ${n ? 'Save sends them to the database.' : 'Click the red cells.'}`),
      n ? h('button', { class: 'btn small main', onClick: () => this.d.save() }, 'Save · Ctrl+S') : null,
      st.waiting ? h('button', { class: 'btn small', onClick: () => this.nextChoice() }, 'Show me') : null].filter(Boolean));
    this.box.hidden = false;
  }

  addRow() {
    const t = this.d.call('tables').find((x) => x.name === this.st.def.calc_table);
    if (!t) return;
    this.d.grid.select(t.bottom + 1, t.left);
    this.d.grid.scroller.focus();
    toast('Type the new row here: it joins the table');
  }

  // ----- Which rows -----

  pickOnly(anchor) {
    const def = this.st.def;
    const r = anchor.getBoundingClientRect();
    menu(def.columns.map(([col]) => item(col, { run: async () => {
      const a = this.d.grid.active;
      const t = this.d.call('tables').find((x) => x.name === def.calc_table);
      // The value under the cursor in that column, as a start.
      let start = '';
      if (t && a.r > t.top && a.r <= t.bottom) {
        const k = t.columns.findIndex((c) => c.name === col);
        if (k >= 0) start = this.d.call('cell', { sheet: this.d.sheet(), row: a.r, col: t.left + k }).text || '';
      }
      const value = h('input', { class: 'field', value: start, placeholder: 'A value' });
      const ok = await modal(`Only rows where ${col} is…`, h('label', {}, 'Value', value), [['Cancel', false], ['Show them', true, true]]);
      if (!ok) return;
      const kind = def.columns.find(([c]) => c === col)?.[1];
      const v = value.value.trim();
      const typed = v === '' ? null : kind === 'number' && !Number.isNaN(Number(v)) ? Number(v) : v;
      this.reload(this.d.sheet(), { only: [...def.only.filter(([c]) => c !== col), [col, typed]] });
    } })), r.left, r.bottom + 4);
  }

  pickOrder(anchor) {
    const def = this.st.def;
    const r = anchor.getBoundingClientRect();
    menu([
      item(`In ${def.key.join(', ')} order`, { checked: !def.order || null, run: () => this.reload(this.d.sheet(), { order: null }) }),
      null,
      ...def.columns.flatMap(([col, kind]) => {
        const [up, down] = orderWords(kind);
        return [
          item(`${col}, ${down}`, { checked: (def.order?.[0] === col && def.order[1]) || null, run: () => this.reload(this.d.sheet(), { order: [col, true] }) }),
          item(`${col}, ${up}`, { checked: (def.order?.[0] === col && !def.order[1]) || null, run: () => this.reload(this.d.sheet(), { order: [col, false] }) }),
        ];
      }),
    ], r.left, r.bottom + 4);
  }

  // ----- Loading -----

  ready(def) {
    if (!this.server.session || this.server.url !== def.server) return `Sign in to ${def.server.replace(/^https?:\/\//, '')} first.`;
    return '';
  }

  /// Reads the table (with `def`'s choices) from the server.
  async read(def) {
    const body = { table: def.table };
    if (def.ticket) body.ticket = def.ticket;
    // The server file it's in: its rows are changed only by people who may edit that file.
    const node = this.server.nodeOf(this.d.tab());
    if (node != null) body.node = node;
    if (def.picked?.length) body.columns = def.picked;
    if (def.only?.length) body.only = def.only.map(([column, value]) => ({ column, value }));
    if (def.order) body.order = { column: def.order[0], desc: def.order[1] };
    return this.server.api('POST', `/v1/data/${def.source}/table`, body);
  }

  /// Loads the rows again (with `change` to which rows). Unsent changes are asked about first.
  async reload(sheet, change = {}, quiet = false) {
    const st = this.d.call('editor', { sheet });
    if (!st) return false;
    const def = { ...st.def, ...change };
    const why = this.ready(def);
    if (why) { if (!quiet) { toast(why); this.d.openServer(); } return false; }
    const unsent = st.updates + st.inserts + st.deletes;
    let force = false;
    if (unsent) {
      if (quiet) return false;
      const ok = await modal('Load the rows again?', h('p', {}, `${plural(unsent, 'change', 'changes')} not sent yet would go. Save first to send them.`), [['Cancel', false], ['Save first', 'save'], ['Drop them and load', 'drop', true]]);
      if (!ok) return false;
      if (ok === 'save') { if (!(await this.d.save())) return false; } else force = true;
    }
    this.busy.add(sheet);
    if (this.d.sheet() === sheet) this.sync();
    const tab = this.d.tab();
    try {
      const answer = await this.read(def);
      if (this.d.tab() !== tab) return false;
      // Its whole definition, so a different set of rows takes effect.
      const full = this.d.call('editor', { sheet }).def;
      // A new ticket (when you write queries) is kept for whoever has the file.
      this.d.run('fill_editor', { sheet, def: { ...full, ...change, loaded: [], ticket: answer.ticket || full.ticket || '' }, answer, refreshed: new Date().toISOString(), force });
      return true;
    } catch (e) {
      if (!quiet) toast(sentence(e.message || e));
      return false;
    } finally {
      this.busy.delete(sheet);
      if (this.d.sheet() === sheet) this.sync();
    }
  }

  /// When a book opens: table editors with nothing waiting load the rows again.
  async opened() {
    let all = [];
    try { all = this.d.call('editors'); } catch {}
    for (const e of all) if (!e.changes.length && !e.waiting) await this.reload(e.sheet, {}, true);
  }

  /// Adds a table editor sheet for `table` of source `src`.
  /// A table editor sheet for `table` (with `picked`: only those columns, and the key).
  async add(src, table, picked = []) {
    const ed = src.editable.find((e) => e.table.toLowerCase() === table.toLowerCase());
    const def = { server: this.server.url, source: src.id, source_name: src.name, table: ed.table, key: ed.key, locked: ed.locked, add: ed.add !== false, delete: !!ed.delete, only: [], picked, refreshed: new Date().toISOString() };
    const answer = await this.read(def);
    def.total = answer.notes?.total ?? null;
    def.ticket = answer.ticket || '';
    const id = this.d.run('add_editor', { def, answer, name: titleCase(ed.table.split('.').pop()) });
    if (id == null) throw new Error('Calc couldn’t add the sheet.');
    this.d.goSheet(id);
    this.sync();
  }

  // ----- Save: sending -----

  /// Sends every table editor's changes (before the file is written).
  /// Returns false when sending failed in a way that should stop Save.
  async send() {
    let all = [];
    try { all = this.d.call('editors'); } catch { return true; }
    const due = all.filter((e) => e.changes.length);
    if (!due.length) return true;
    let sent = 0; let waiting = 0; let gone = 0;
    // The answers go to this book, even if another is in front by then.
    const tab = this.d.tab();
    for (const e of due) {
      const why = this.ready({ server: e.server });
      if (why) {
        toast(`${plural(e.changes.length, 'change', 'changes')} to ${e.table} not sent: ${why.toLowerCase()} They’re kept in the book.`);
        continue;
      }
      try {
        const { outcomes } = await this.server.api('POST', `/v1/data/${e.source}/changes`, { changes: e.changes, ticket: e.ticket || null, node: this.server.nodeOf(tab) });
        this.d.call('editor_sent', { tab, sheet: e.sheet, changes: e.changes, outcomes });
        for (const o of outcomes) {
          if (o.outcome === 'conflict') waiting++;
          else if (o.outcome === 'missing') gone++;
          else sent++;
        }
      } catch (err) {
        toast(`Couldn’t send the changes to ${e.table}: ${sentence(err.message || err)} They’re kept in the book.`);
      }
    }
    if (sent || waiting || gone) {
      const bits = [sent ? `Sent ${plural(sent, 'change', 'changes')} to the database` : null,
        waiting ? `${plural(waiting, 'cell', 'cells')} changed by someone else first: pick theirs or yours (red)` : null,
        gone ? `${plural(gone, 'row was', 'rows were')} deleted in the database meanwhile` : null].filter(Boolean);
      toast(bits.join('. '));
    }
    this.sync();
    return true;
  }

  // ----- Choices -----

  /// When the cursor lands on a cell waiting for a choice: theirs or mine.
  cellSelected(r, c) {
    const st = this.st;
    if (!st?.waiting) return;
    const ch = st.choices.find((x) => x.row === r && x.col === c);
    if (!ch) return;
    const g = this.d.grid;
    const host = g.host.getBoundingClientRect();
    const x = host.left + g.x(c);
    const y = host.top + g.y(r) + g.rows.size(r) + 6;
    const choose = (choice) => { closePopups(); this.d.run('editor_resolve', { sheet: this.d.sheet(), key: ch.key, column: ch.column, choice }); };
    const card = ch.gone
      ? h('div', { class: 'choice' }, h('b', {}, 'Not sent: this row was deleted in the database since you loaded it'),
        h('div', { class: 'modal-actions' }, h('button', { class: 'btn small', onClick: () => choose('mine') }, 'Keep it here'), h('button', { class: 'btn small main', onClick: () => choose('theirs') }, 'Take it off the sheet')))
      : h('div', { class: 'choice' }, h('b', {}, 'Not sent: changed in the database since you loaded it'),
        h('dl', {}, h('dt', {}, 'Now'), h('dd', {}, show(ch.now)), h('dt', {}, 'Yours'), h('dd', {}, show(ch.mine))),
        h('div', { class: 'modal-actions' }, h('button', { class: 'btn small', onClick: () => choose('theirs') }, 'Keep theirs'), h('button', { class: 'btn small main', onClick: () => choose('mine') }, 'Use mine')));
    popup(card, x, y);
  }

  nextChoice() {
    const ch = this.st?.choices.find((x) => x.row != null);
    if (!ch) return;
    this.d.grid.select(ch.row, ch.col);
    this.cellSelected(ch.row, ch.col);
  }
}
