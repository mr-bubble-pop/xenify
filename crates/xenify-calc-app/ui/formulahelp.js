// Help with formulas. While a formula is typed (in a cell or the formula
// bar): the functions whose names start with what's typed, to pick with
// the arrows and Tab or Enter; inside a function's brackets, its arguments
// with the one being typed in bold and what it wants. The function
// dictionary (Help › Functions, or fx by the formula bar) explains each
// function with an example: its data drawn as a little sheet, the formula
// and what it gives, which can be put on a new sheet to try.

import { h, toast } from './ui.js';

/// Where the caret is in a formula: the name being typed, or the function
/// whose brackets it's in and which argument.
export function where(text, caret) {
  if (!text.startsWith('=')) return null;
  const before = text.slice(0, caret);
  // Not inside a quoted string.
  if ((before.match(/"/g) || []).length % 2) return null;
  const typing = /(^|[=(,;+\-*/^&<>:\s])([A-Za-z][A-Za-z0-9._]*)$/.exec(before);
  const after = text.slice(caret);
  const name = typing && !/^[A-Za-z0-9._]*\(/.test(after) && !/^[A-Za-z0-9._]*!/.test(after) ? typing[2] : null;
  // The innermost open bracket before the caret, and the commas since it.
  let depth = 0; let arg = 0; let quoted = false;
  for (let i = before.length - 1; i >= 0; i--) {
    const ch = before[i];
    if (ch === '"') { quoted = !quoted; continue; }
    if (quoted) continue;
    if (ch === ')' ) depth++;
    else if (ch === '(') {
      if (depth === 0) {
        const fn = /([A-Za-z][A-Za-z0-9._]*)$/.exec(before.slice(0, i));
        return { name, call: fn ? fn[1].toUpperCase() : null, arg };
      }
      depth--;
    } else if ((ch === ',' || ch === ';') && depth === 0) arg++;
  }
  return { name, call: null, arg: 0 };
}

export class FormulaHelp {
  constructor(d) {
    this.d = d;   // { call, run, host, sheet, goSheet, grid }
    this.box = h('div', { class: 'fx-help', role: 'listbox', 'aria-label': 'Functions', hidden: true });
    document.body.append(this.box);
    this.panel = h('aside', { class: 'sumpanel funcpanel', 'aria-label': 'Functions', hidden: true });
    d.host.append(this.panel);
    this.list = null;
    this.pick = 0;
    this.shown = [];
    document.addEventListener('keydown', (e) => {
      if (!this.panel.hidden && e.key === 'Escape' && !e.target.closest?.('.modal, .popup, .menu, input')) { e.preventDefault(); this.close(); }
    });
  }

  names() {
    if (!this.list) { try { this.list = this.d.call('functions'); } catch { this.list = []; } }
    return this.list;
  }

  /// Follows typing in `input` (a textarea or input): suggestions and hints under it.
  attach(input) {
    const update = () => this.update(input);
    input.addEventListener('input', update);
    input.addEventListener('click', update);
    input.addEventListener('keyup', (e) => { if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) update(); });
    input.addEventListener('blur', () => setTimeout(() => { if (document.activeElement !== input) this.hide(); }, 150));
    // Before the cell's own keys: arrows, Tab and Enter pick a suggestion.
    input.addEventListener('keydown', (e) => {
      if (this.box.hidden || !this.shown.length || this.input !== input) return;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault(); e.stopImmediatePropagation();
        this.pick = (this.pick + (e.key === 'ArrowDown' ? 1 : -1) + this.shown.length) % this.shown.length;
        this.draw();
      } else if (e.key === 'Tab' || (e.key === 'Enter' && !e.altKey)) {
        e.preventDefault(); e.stopImmediatePropagation();
        this.take(this.shown[this.pick].name);
      } else if (e.key === 'Escape') {
        e.preventDefault(); e.stopImmediatePropagation();
        this.hide();
      }
    }, true);
  }

  hide() {
    this.box.hidden = true;
    this.shown = [];
  }

  update(input) {
    this.input = input;
    const w = where(input.value, input.selectionStart ?? input.value.length);
    if (!w) { this.hide(); return; }
    this.at = w;
    if (w.name) {
      const q = w.name.toUpperCase();
      const all = this.names();
      const starts = all.filter((f) => f.name.startsWith(q));
      const has = all.filter((f) => !f.name.startsWith(q) && f.name.includes(q));
      this.shown = [...starts.sort((a, b) => a.name.length - b.name.length || a.name.localeCompare(b.name)), ...has].slice(0, 8);
      this.pick = 0;
      this.mode = 'names';
      if (this.shown.length) { this.draw(); return; }
    }
    if (w.call) {
      let e;
      try { e = this.d.call('function', { name: w.call }); } catch { e = null; }
      if (e) { this.mode = 'args'; this.entry = e; this.shown = []; this.draw(); return; }
    }
    this.hide();
  }

  /// Puts NAME( in place of the name being typed.
  take(name) {
    const input = this.input;
    const caret = input.selectionStart;
    const typed = this.at.name || '';
    const start = caret - typed.length;
    input.value = `${input.value.slice(0, start)}${name}(${input.value.slice(caret)}`;
    const at = start + name.length + 1;
    input.setSelectionRange(at, at);
    input.dispatchEvent(new Event('input'));
  }

  draw() {
    const box = this.box;
    if (this.mode === 'names') {
      box.setAttribute('role', 'listbox');
      box.replaceChildren(...this.shown.map((f, k) => h('div', {
        class: `fx-item${k === this.pick ? ' on' : ''}`, role: 'option', 'aria-selected': String(k === this.pick),
        onMousedown: (e) => { e.preventDefault(); this.take(f.name); },
      }, h('b', {}, f.name), h('span', {}, f.about))),
      h('div', { class: 'fx-foot' }, '↑↓ to choose · Tab to use · ', h('a', { href: '#', onMousedown: (e) => { e.preventDefault(); this.open(this.shown[this.pick]?.name); } }, 'More about it')));
    } else {
      const e = this.entry;
      const n = this.at.arg;
      const args = e.args.map((a, k) => {
        const on = k === n || (e.more && k === e.args.length - 1 && n >= k);
        const label = a.optional ? `[${a.name}]` : a.name;
        return on ? h('b', { class: 'cur' }, label) : h('span', {}, label);
      });
      const parts = [];
      args.forEach((a, k) => { if (k) parts.push(', '); parts.push(a); });
      if (e.more) parts.push(', …');
      const cur = e.args[Math.min(n, e.args.length - 1)];
      box.setAttribute('role', 'tooltip');
      box.replaceChildren(
        h('div', { class: 'fx-sig' }, h('b', {}, e.name), '(', ...parts, ')'),
        cur && (n < e.args.length || e.more) ? h('div', { class: 'fx-arg' }, h('b', {}, cur.name), `: ${cur.about}`) : h('div', { class: 'fx-arg' }, 'No more arguments: close the bracket.'),
        h('div', { class: 'fx-foot' }, e.about, ' ', h('a', { href: '#', onMousedown: (ev) => { ev.preventDefault(); this.open(e.name); } }, 'Example…')));
    }
    box.hidden = false;
    const r = this.input.getBoundingClientRect();
    const bw = 420;
    box.style.left = `${Math.max(8, Math.min(innerWidth - bw - 8, r.left))}px`;
    box.style.top = `${r.bottom + 4}px`;
  }

  // ----- The dictionary -----

  /// Opens the dictionary (on function `name`, if given).
  open(name = null) {
    this.hide();
    this.panel.hidden = false;
    this.search = this.search || '';
    if (name) this.showEntry(name); else this.showList();
  }

  close() {
    this.panel.hidden = true;
  }

  head(title, back) {
    return h('div', { class: 'head' },
      back ? h('button', { class: 'btn small plain', onClick: back }, '‹ All functions') : h('h2', {}, title),
      h('button', { class: 'btn small close', title: 'Close (Esc)', onClick: () => this.close() }, '× Close'));
  }

  showList() {
    const all = this.names();
    const q = h('input', { class: 'field', type: 'search', placeholder: 'Find a function (sum, date, text…)', value: this.search, 'aria-label': 'Find a function' });
    const body = h('div', { class: 'fx-list' });
    const fill = () => {
      const s = q.value.trim().toLowerCase();
      this.search = q.value;
      const hits = s ? all.filter((f) => f.name.toLowerCase().includes(s) || f.about.toLowerCase().includes(s)) : all;
      const groups = new Map();
      for (const f of hits) { if (!groups.has(f.group)) groups.set(f.group, []); groups.get(f.group).push(f); }
      body.replaceChildren(...(hits.length ? [...groups].flatMap(([g, fs]) => [
        h('div', { class: 'k' }, `${g} (${fs.length})`),
        ...fs.sort((a, b) => a.name.localeCompare(b.name)).map((f) => h('button', { class: 'fx-row', onClick: () => this.showEntry(f.name) }, h('b', {}, f.name), h('span', {}, f.about))),
      ]) : [h('p', { class: 'says' }, 'No function matches that.')]));
    };
    q.addEventListener('input', fill);
    q.addEventListener('keydown', (e) => e.stopPropagation());
    fill();
    this.panel.replaceChildren(this.head('Functions'), q, body);
    q.focus();
  }

  showEntry(name) {
    let e;
    try { e = this.d.call('function', { name }); } catch { toast(`No function called ${name}`); return; }
    const ex = e.example;
    this.panel.replaceChildren(
      this.head(e.name, () => this.showList()),
      h('h2', { class: 'fx-name' }, e.name),
      h('code', { class: 'fx-syntax' }, e.syntax),
      h('p', { class: 'says' }, e.about),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'What it takes'),
        h('dl', { class: 'fx-args' }, e.args.flatMap((a) => [h('dt', {}, a.name, a.optional ? h('span', { class: 'opt' }, ' (can be left out)') : null), h('dd', {}, a.about)]),
          e.more ? [h('dt', {}, '…'), h('dd', {}, `more of the last one, as many as needed`)] : [])),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'Example'), exampleSheet(ex), h('p', { class: 'fx-why' }, ex.why)),
      e.notes ? h('p', { class: 'fx-notes' }, e.notes) : null,
      h('div', { class: 'actions' },
        h('button', { class: 'btn', title: 'A new sheet with the example data and formula', onClick: () => this.tryIt(e) }, 'Try it on a new sheet')));
  }

  /// The example on a new sheet: its data from A1, the formula beside it.
  tryIt(e) {
    const ex = e.example;
    const id = this.d.run('add_sheet', { name: `${e.name} example` });
    if (id == null) return;
    const cells = [];
    ex.cells.forEach((row, r) => row.forEach((v, c) => { if (v !== '') cells.push({ row: r + 1, col: c + 1, data: { i: v } }); }));
    const col = Math.max(0, ...ex.cells.map((r) => r.length)) + 2;
    cells.push({ row: 1, col, data: { i: ex.formula } });
    this.d.run('edit', { ops: [{ op: 'cells', sheet: id, cells }] });
    this.d.goSheet(id);
    this.d.grid.select(1, col);
  }
}

function letters(c) { let s = ''; while (c > 0) { const r = (c - 1) % 26; s = String.fromCharCode(65 + r) + s; c = Math.floor((c - 1) / 26); } return s; }

/// The example as a little sheet: column letters, row numbers, its data and
/// the formula's cell (showing the formula and what it gives).
function exampleSheet(ex) {
  const width = Math.max(0, ...ex.cells.map((r) => r.length));
  const col = width + 2;
  const rows = Math.max(1, ex.cells.length);
  const num = (v) => /^-?[\d.,]+%?$/.test(v);
  const head = h('tr', {}, h('th', {}), ...Array.from({ length: col }, (_, k) => h('th', { class: k + 1 === col ? 'fcol' : null }, letters(k + 1))));
  const body = Array.from({ length: rows }, (_, r) => h('tr', {}, h('th', {}, String(r + 1)),
    ...Array.from({ length: col }, (_, k) => {
      if (k + 1 === col) return r === 0 ? h('td', { class: 'res' }, ex.result ?? '(changes)') : h('td', {});
      const v = ex.cells[r]?.[k] ?? '';
      return h('td', { class: num(v) ? 'n' : null }, v);
    })));
  return h('div', { class: 'fx-ex' },
    h('div', { class: 'fx-formula' }, h('span', { class: 'cell' }, `${letters(col)}1`), h('code', {}, ex.formula)),
    h('table', {}, h('thead', {}, head), h('tbody', {}, body)));
}
