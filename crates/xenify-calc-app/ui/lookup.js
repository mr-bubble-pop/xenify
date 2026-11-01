// Look up by example: three clicks on the sheet instead of typing a
// VLOOKUP. 1) the table to look in, 2) the column to match on (Calc finds
// which of your columns matches it), 3) the value you'd expect: its column
// comes back. Nothing is written until "Fill the column".

import { h } from './ui.js';
import { colName } from './grid.js';

const TEAL = '#0F766E';
const EMBER = '#C2410C';

export class Lookup {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet, goSheet, sheetName, toast }
    this.on = false;
  }

  /// Starts with the cursor where the answers go.
  start() {
    const { call, grid, sheet } = this.d;
    const a = grid.active;
    const yours = call('lookup_rows', { sheet: sheet(), row: a.r, col: a.c });
    if (!yours) {
      this.d.toast('Put the cursor in the column where the answers go, beside the rows they’re for');
      return;
    }
    this.s = { stage: 1, yours, target: a.c, look: null, on: null, key: null, ret: null, candidates: [], missing: '' };
    this.on = true;
    grid.onPick = (hit) => this.pick(hit);
    this.keys = (e) => { if (e.key === 'Escape' && !document.querySelector('.modal-back, .popup')) { e.preventDefault(); this.stop(); } };
    document.addEventListener('keydown', this.keys, true);
    document.getElementById('lookbar').hidden = false;
    grid.resize();
    this.show();
  }

  stop() {
    this.on = false;
    this.d.grid.onPick = null;
    this.d.grid.marks = [];
    document.removeEventListener('keydown', this.keys, true);
    document.getElementById('lookbar').hidden = true;
    document.getElementById('lookcard').hidden = true;
    this.d.grid.resize();
  }

  /// A sheet's tab was clicked while picking the table: its one table is taken.
  sheetShown(id) {
    if (!this.on || this.s.stage !== 1 || id === this.s.yours.sheet) { this.show(); return; }
    const look = this.d.call('lookup_in', { sheet: id });
    if (look) this.setLook(look);
    else this.show();
  }

  setLook(look) {
    this.s.look = look;
    this.s.stage = 2;
    this.s.on = this.s.key = this.s.ret = null;
    this.show();
  }

  /// A click on the sheet: the answer to the step it's on.
  pick(hit) {
    const s = this.s;
    const sheet = this.d.sheet();
    const inside = (r) => r && r.sheet === sheet && hit.r >= r.top && hit.r <= r.bottom && hit.c >= r.left && hit.c <= r.right;
    if (s.stage === 1 || (!inside(s.look) && sheet !== s.yours.sheet)) {
      const look = this.d.call('lookup_in', { sheet, row: hit.r, col: hit.c });
      if (!look) { this.d.toast('Click inside the table to look in'); return true; }
      if (look.sheet === s.yours.sheet && look.top === s.yours.top && look.left === s.yours.left) {
        this.d.toast('That’s the table the answers are for: pick the other table to look in');
        return true;
      }
      this.setLook(look);
      return true;
    }
    if (!inside(s.look)) return false;
    if (s.stage === 2) {
      s.on = hit.c;
      s.candidates = this.d.call('lookup_candidates', { yours: s.yours, look: s.look, on: hit.c });
      s.key = s.candidates[0]?.col ?? null;
      s.stage = 3;
      this.show();
      return true;
    }
    if (s.stage >= 3) {
      s.ret = hit.c;
      s.stage = 4;
      this.show();
      return true;
    }
    return false;
  }

  plan() {
    const s = this.s;
    return { yours: s.yours, key: s.key, look: s.look, on: s.on, ret: s.ret, target: s.target, missing: s.missing };
  }

  fill() {
    const s = this.s;
    if (s.stage < 4 || s.key == null) return;
    const ok = this.d.run('lookup_fill', { plan: this.plan() });
    if (ok === undefined) return;
    const yours = s.yours;
    const target = s.target;
    this.stop();
    if (this.d.sheet() !== yours.sheet) this.d.goSheet(yours.sheet);
    this.d.grid.selectRange({ top: yours.top + (yours.header ? 1 : 0), left: target, bottom: yours.bottom, right: target });
    this.d.toast('Filled. The formula is in each cell: change it there if you need to');
  }

  header(rows, col) {
    if (!rows) return colName(col);
    const name = rows.header ? this.d.call('cell', { sheet: rows.sheet, row: rows.top, col }).input : '';
    return name || colName(col);
  }

  show() {
    const s = this.s;
    const { grid, sheetName } = this.d;
    // Marks on the sheet: the table looked in, the column matched, the one that comes back.
    const marks = [{ sheet: s.yours.sheet, top: s.yours.top, bottom: s.yours.bottom, left: s.target, right: s.target, color: EMBER, dash: true, label: 'answers' }];
    if (s.look) {
      marks.push({ ...s.look, color: TEAL, dash: true });
      if (s.on != null) marks.push({ sheet: s.look.sheet, top: s.look.top, bottom: s.look.bottom, left: s.on, right: s.on, color: TEAL, fill: 'rgba(15, 118, 110, .10)', label: 'match' });
      if (s.ret != null) marks.push({ sheet: s.look.sheet, top: s.look.top, bottom: s.look.bottom, left: s.ret, right: s.ret, color: EMBER, fill: 'rgba(194, 65, 12, .10)', label: 'comes back' });
      if (s.key != null) marks.push({ sheet: s.yours.sheet, top: s.yours.top, bottom: s.yours.bottom, left: s.key, right: s.key, color: TEAL, fill: 'rgba(15, 118, 110, .08)' });
    }
    grid.marks = marks;
    grid.draw();

    const step = (n, title, detail, cls) => h('div', { class: `step ${cls}` },
      h('span', { class: 'n', 'aria-hidden': 'true' }, cls === 'done' ? '✓' : String(n)),
      h('div', { style: { minWidth: '0' } }, h('b', {}, title), detail));
    const st = (n) => (s.stage > n ? 'done' : s.stage === n ? 'now' : '');
    const lookName = s.look ? (s.look.table ? `${s.look.table} (the whole table, ${s.look.bottom - s.look.top} rows)` : `${sheetName(s.look.sheet)} ${colName(s.look.left)}${s.look.top}:${colName(s.look.right)}${s.look.bottom}`) : 'Click inside it, or on its sheet’s tab';
    let matchDetail = h('span', { class: 'd' }, 'In the table looked in');
    if (s.on != null) {
      const sel = h('select', { 'aria-label': 'Your column to match', onChange: (e) => { s.key = Number(e.target.value); this.show(); } },
        s.candidates.map((c) => h('option', { value: c.col, selected: c.col === s.key }, `${c.name} · ${c.matches} of ${c.rows} match`)));
      matchDetail = h('span', { class: 'd' }, `${this.header(s.look, s.on)} = ${sheetName(s.yours.sheet)} › `, sel);
      if (!s.candidates.length || s.candidates[0].matches === 0) matchDetail.append(h('span', { style: { color: '#B91C1C' } }, ' none match'));
    }
    const retDetail = h('span', { class: 'd' }, s.ret != null ? `You want the ${this.header(s.look, s.ret)}` : 'Click any value in the column you want back');
    const bar = document.getElementById('lookbar');
    bar.replaceChildren(
      h('div', { class: 'top' },
        h('h2', {}, `Look up for ${sheetName(s.yours.sheet)} › column ${colName(s.target)}`),
        h('span', { class: 'hint' }, 'Click on the sheet to answer each step. Esc cancels.'),
        h('button', { class: 'btn small', onClick: () => this.stop() }, 'Cancel'),
        h('button', { class: 'btn small main', disabled: s.stage < 4, onClick: () => this.fill() }, 'Fill the column')),
      h('div', { class: 'steps' },
        step(1, '1 · Select the table to look in', h('span', { class: 'd' }, lookName), st(1)),
        step(2, '2 · Click the column to match on', matchDetail, st(2) || (s.stage > 2 ? 'done' : '')),
        step(3, '3 · Click the value you’d expect in that row', retDetail, s.stage === 3 ? 'now' : s.stage > 3 ? 'done' : '')));

    const card = document.getElementById('lookcard');
    if (s.stage < 4) { card.hidden = true; return; }
    const prev = this.d.call('lookup_preview', { plan: this.plan(), rows: 4 });
    const seg = (label, value) => h('button', { class: 'seg', 'aria-pressed': String(s.missing === value), onClick: () => { s.missing = value; this.show(); } }, label);
    card.replaceChildren(
      h('h3', {}, `What ${sheetName(s.yours.sheet)} › ${colName(s.target)} will show`),
      h('table', {},
        h('thead', {}, h('tr', {}, h('th', {}, this.header(s.yours, s.key)), h('th', {}, this.header(s.look, s.ret)))),
        h('tbody', {}, prev.rows.map((r) => h('tr', {}, h('td', {}, r.key), r.answer == null ? h('td', { class: 'miss' }, s.missing || 'not found: left empty') : h('td', {}, r.answer))))),
      h('span', { class: 'lab' }, 'When it isn’t found'),
      h('div', { class: 'segs' }, seg('Leave empty', ''), seg('Show “not found”', 'not found')),
      h('details', {}, h('summary', { class: 'lab' }, 'Formula'), h('code', {}, prev.formula)));
    card.hidden = false;
  }
}
