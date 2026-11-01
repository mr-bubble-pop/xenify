// The chart panel: what a chart shows, beside the sheet. Pick the kind from
// its picture, then drag columns into the boxes (or use + add): the points
// along the bottom, the values (each row as it is, or added up: totals,
// averages, counts…), a colour or line for each value of another column,
// and only some rows. The chart on the sheet changes as you go; it's all
// one step of undo.

import { h, menu, item, toast } from './ui.js';
import { KINDS, EACH_ROW } from './charts.js';

const HOWS = [['each', 'Each row as it is'], ['sum', 'Total'], ['average', 'Average'], ['rows', 'How many rows'], ['min', 'Smallest'], ['max', 'Largest'], ['median', 'Middle value'], ['distinct', 'How many different']];
const TESTS = [['is', 'is'], ['is_not', 'isn’t'], ['contains', 'contains'], ['more', 'is more than'], ['less', 'is less than'], ['year', 'is a date in']];

/// What the "along the bottom" box is called for each kind (none: the kind has no points).
const X_LABEL = { bar: 'Along the bottom', stacked: 'Along the bottom', line: 'Along the bottom', area: 'Along the bottom', sideways: 'Down the side', pie: 'A slice for each', heatmap: 'Along the bottom', scatter: 'Along the bottom (numbers)' };
/// What the "split by" box is called (none: the kind can't split).
const SPLIT_LABEL = { bar: 'A colour for each', stacked: 'A colour for each', sideways: 'A colour for each', line: 'A line for each', area: 'A colour for each', heatmap: 'A row for each' };
/// Kinds that show one value.
const ONE_VALUE = ['pie', 'heatmap', 'histogram'];

// Small pictures of each kind (the accent colour and a lighter one).
const A = 'var(--series-1)';
const B = 'var(--series-2)';
const PICTURES = {
  bar: `<rect x="6" y="18" width="7" height="12" rx="1.5" fill="${A}"/><rect x="17" y="8" width="7" height="22" rx="1.5" fill="${A}"/><rect x="28" y="13" width="7" height="17" rx="1.5" fill="${A}"/><rect x="39" y="4" width="7" height="26" rx="1.5" fill="${A}"/>`,
  sideways: `<rect x="4" y="4" width="36" height="5" rx="1.5" fill="${A}"/><rect x="4" y="11.5" width="26" height="5" rx="1.5" fill="${A}"/><rect x="4" y="19" width="40" height="5" rx="1.5" fill="${A}"/><rect x="4" y="26.5" width="16" height="5" rx="1.5" fill="${A}"/>`,
  stacked: `<rect x="6" y="20" width="7" height="10" fill="${A}"/><rect x="6" y="13" width="7" height="6" rx="1.5" fill="${B}"/><rect x="17" y="14" width="7" height="16" fill="${A}"/><rect x="17" y="5" width="7" height="8" rx="1.5" fill="${B}"/><rect x="28" y="17" width="7" height="13" fill="${A}"/><rect x="28" y="10" width="7" height="6" rx="1.5" fill="${B}"/><rect x="39" y="12" width="7" height="18" fill="${A}"/><rect x="39" y="3" width="7" height="8" rx="1.5" fill="${B}"/>`,
  line: `<polyline points="4,26 14,18 24,21 34,9 46,5" fill="none" stroke="${A}" stroke-width="2.5" stroke-linejoin="round" stroke-linecap="round"/><polyline points="4,29 14,26 24,27 34,20 46,17" fill="none" stroke="${B}" stroke-width="2.5" stroke-linejoin="round" stroke-linecap="round"/>`,
  area: `<path d="M4,26 L14,16 L24,19 L34,8 L46,5 L46,30 L4,30Z" fill="${A}" fill-opacity=".22"/><polyline points="4,26 14,16 24,19 34,8 46,5" fill="none" stroke="${A}" stroke-width="2.5" stroke-linejoin="round"/>`,
  pie: `<circle cx="25" cy="17" r="11" fill="none" stroke="${A}" stroke-width="7" stroke-dasharray="40 100"/><circle cx="25" cy="17" r="11" fill="none" stroke="${B}" stroke-width="7" stroke-dasharray="0 40 20 100"/><circle cx="25" cy="17" r="11" fill="none" stroke="var(--series-3)" stroke-width="7" stroke-dasharray="0 60 9.2 100"/>`,
  scatter: `<circle cx="9" cy="24" r="2.6" fill="${A}"/><circle cx="15" cy="19" r="2.6" fill="${A}"/><circle cx="21" cy="21" r="2.6" fill="${A}"/><circle cx="27" cy="13" r="2.6" fill="${A}"/><circle cx="33" cy="15" r="2.6" fill="${A}"/><circle cx="40" cy="8" r="2.6" fill="${A}"/><circle cx="44" cy="11" r="2.6" fill="${A}"/>`,
  heatmap: [0.25, 0.6, 0.9, 0.4, 0.15, 0.75, 0.5, 1, 0.35, 0.55, 0.2, 0.8].map((o, k) => `<rect x="${5 + (k % 4) * 10.5}" y="${3 + Math.floor(k / 4) * 9.5}" width="9" height="8" rx="1.5" fill="${A}" fill-opacity="${o}"/>`).join(''),
  histogram: [5, 11, 19, 25, 17, 9, 4].map((v, k) => `<rect x="${4 + k * 6.2}" y="${30 - v}" width="5.4" height="${v}" rx="1" fill="${A}"/>`).join(''),
  box: `<line x1="5" y1="11" x2="45" y2="11" stroke="${A}" stroke-width="2"/><rect x="15" y="6" width="18" height="10" rx="2" fill="${A}" fill-opacity=".22" stroke="${A}" stroke-width="2"/><line x1="23" y1="6" x2="23" y2="16" stroke="${A}" stroke-width="2.5"/><line x1="9" y1="24" x2="41" y2="24" stroke="${B}" stroke-width="2"/><rect x="17" y="20" width="12" height="8" rx="2" fill="${B}" fill-opacity=".22" stroke="${B}" stroke-width="2"/>`,
};

export class ChartPanel {
  constructor(d) {
    this.d = d;   // { call, run, charts, sheet, host }
    this.panel = h('aside', { class: 'sumpanel chartpanel', 'aria-label': 'Chart', hidden: true });
    d.host.append(this.panel);
    this.on = false;
    document.addEventListener('keydown', (e) => {
      if (this.on && e.key === 'Escape' && !e.target.closest?.('.modal, .popup, .menu')) { e.preventDefault(); this.stop(); }
    });
  }

  /// Opens on chart `id` (of the sheet shown).
  start(id) {
    const c = this.d.call('charts', { sheet: this.d.sheet() }).find((x) => x.id === id);
    if (!c) return;
    this.c = structuredClone(c);
    this.again = false;
    try { this.cols = this.d.call('chart_columns', { source: c.source }); } catch (e) { toast(String(e.message || e)); return; }
    const s = c.source;
    this.x = s.x || (s.range && s.labels && !s.series?.length ? this.cols[0] : null);
    if (s.values?.length) this.vals = s.values.map((v) => ({ ...v }));
    else if (s.series?.length) this.vals = s.series.map((column) => ({ column, how: 'each' }));
    else this.vals = this.cols.slice(s.labels ? 1 : 0).map((column) => ({ column, how: 'each' }));
    // Only columns with numbers in them (words can't be drawn).
    try {
      const data = this.d.call('chart_data', { id });
      const numeric = new Set(data.series.filter((x) => x.values.some((v) => v != null)).map((x) => x.name));
      if (!s.values?.length) this.vals = this.vals.filter((v) => numeric.has(v.column));
    } catch { /* the chart shows why */ }
    this.split = s.split || null;
    this.only = (s.only || []).map((o) => ({ ...o }));
    this.on = true;
    this.panel.hidden = false;
    this.d.charts.shove = 420;
    this.d.charts.select(id);
    this.d.charts.place();
    this.render();
  }

  stop() {
    if (!this.on) return;
    this.on = false;
    this.panel.hidden = true;
    this.d.charts.shove = 0;
    this.d.charts.place();
  }

  /// Whether some value of column `x` comes more than once (so rows need adding up).
  repeats(x) {
    if (!x) return false;
    try {
      const p = this.d.call('summary_preview', { def: { from: this.from(), group: [x], show: [{ how: 'rows', column: '' }], only: [], order: null, desc: false, totals: false } });
      return p.rows.slice(1).some((r) => r[r.length - 1] !== '1');
    } catch { return false; }
  }

  from() {
    const s = this.c.source;
    return { sheet: s.sheet, ...(s.table ? { table: s.table } : { range: s.range }), ...(s.header === false ? { header: false } : {}) };
  }

  /// Puts the panel's choices into the chart and shows it.
  apply() {
    const c = this.c;
    const old = c.source;
    const raw = EACH_ROW.includes(c.kind) || (!this.split && !this.only.length && this.vals.every((v) => v.how === 'each'));
    const source = { sheet: old.sheet, labels: false, series: this.vals.map((v) => v.column).filter(Boolean) };
    if (old.table) source.table = old.table; else source.range = old.range;
    if (old.header === false) source.header = false;
    if (this.x) source.x = this.x;
    if (!raw) {
      source.values = this.vals.map((v) => ({ column: v.how === 'rows' ? '' : v.column, how: v.how === 'each' ? 'sum' : v.how }));
      if (this.split) source.split = this.split;
      if (this.only.length) source.only = this.only;
    }
    c.source = source;
    this.d.run('set_chart', { chart: c, again: this.again });
    this.again = true;
    this.d.charts.refresh();
    this.d.charts.select(c.id);
  }

  change(f) {
    f();
    this.apply();
    this.render();
  }

  chip(text, onRemove, extra) {
    return h('span', { class: 'chip' }, text, extra || null, onRemove ? h('button', { class: 'x', 'aria-label': `Take ${text} away`, onClick: onRemove }, '×') : null);
  }

  box(label, kids, onDrop, addItems, full = false) {
    const add = full ? null : h('button', { class: 'add', onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); menu(addItems(), r.left, r.bottom + 4); } }, '+ add');
    const box = h('div', { class: 'box' }, kids, add);
    box.addEventListener('dragover', (e) => { e.preventDefault(); box.classList.add('drop'); });
    box.addEventListener('dragleave', () => box.classList.remove('drop'));
    box.addEventListener('drop', (e) => { e.preventDefault(); box.classList.remove('drop'); const col = e.dataTransfer.getData('text/x-column'); if (col) onDrop(col); });
    return h('div', { class: 'field-group' }, h('span', { class: 'k' }, label), box);
  }

  /// Adds a value: added up when the points repeat, else as it is.
  addValue(column) {
    const how = column === '' ? 'rows' : (this.repeats(this.x) && !EACH_ROW.includes(this.c.kind) ? 'sum' : 'each');
    if (ONE_VALUE.includes(this.c.kind)) this.vals = [{ column, how }]; else this.vals.push({ column, how });
  }

  setX(col) {
    this.x = col;
    // Points that repeat: their rows are added up (not shown one by one).
    if (this.repeats(col) && this.vals.every((v) => v.how === 'each') && !EACH_ROW.includes(this.c.kind)) {
      for (const v of this.vals) v.how = 'sum';
      toast(`Rows with the same ${col} are added up (change Total to Average… on each value)`);
    }
  }

  render() {
    const c = this.c;
    const kind = c.kind;
    const cols = this.cols;
    const eachRow = EACH_ROW.includes(kind);
    const s = c.source;
    const from = s.table ? `${s.table} (table, grows with it)` : `${this.d.sheetName(s.sheet)} ${cellRange(s.range)}`;
    const tiles = h('div', { class: 'kinds', role: 'radiogroup', 'aria-label': 'Kind of chart' }, KINDS.map(([k, label]) => h('button', {
      class: `kind${k === kind ? ' on' : ''}`, role: 'radio', 'aria-checked': String(k === kind), title: label,
      onClick: () => this.change(() => {
        c.kind = k;
        if (ONE_VALUE.includes(k)) this.vals = this.vals.slice(0, 1);
        if (!SPLIT_LABEL[k]) this.split = null;
        if (EACH_ROW.includes(k)) for (const v of this.vals) v.how = 'each';
        else if (this.repeats(this.x)) for (const v of this.vals) if (v.how === 'each') v.how = 'sum';
      }),
    }, h('span', { class: 'pic', html: `<svg viewBox="0 0 50 34" width="50" height="34" aria-hidden="true">${PICTURES[k]}</svg>` }), h('span', {}, label))));
    const howSelect = (v) => h('select', { 'aria-label': `How ${v.column || 'rows'} is shown`, onChange: (e) => this.change(() => {
      const how = e.target.value;
      // Each row as it is applies to all the values; adding up too.
      if (how === 'each') { for (const w of this.vals) w.how = 'each'; this.split = null; this.only = []; } else {
        for (const w of this.vals) if (w.how === 'each') w.how = 'sum';
        v.how = how;
      }
    }) }, HOWS.filter(([k]) => k !== 'rows' || v.how === 'rows').map(([k, l]) => h('option', { value: k, selected: v.how === k }, l)));
    const xBox = X_LABEL[kind] ? this.box(X_LABEL[kind],
      this.x ? [this.chip(this.x, () => this.change(() => { this.x = null; }))] : [],
      (col) => this.change(() => this.setX(col)),
      () => cols.map((col) => item(col, { run: () => this.change(() => this.setX(col)) })), !!this.x) : null;
    const valsBox = this.box(eachRow ? 'Numbers' : 'Values',
      this.vals.map((v, k) => this.chip(v.how === 'rows' ? 'Rows' : v.column, () => this.change(() => this.vals.splice(k, 1)), eachRow || v.how === 'rows' ? null : howSelect(v))),
      (col) => this.change(() => this.addValue(col)),
      () => [...(eachRow ? [] : [item('How many rows', { run: () => this.change(() => this.addValue('')) }), null]), ...cols.map((col) => item(col, { run: () => this.change(() => this.addValue(col)) }))],
      ONE_VALUE.includes(kind) && this.vals.length > 0);
    const splitBox = SPLIT_LABEL[kind] ? this.box(SPLIT_LABEL[kind],
      this.split ? [this.chip(this.split, () => this.change(() => { this.split = null; }))] : [],
      (col) => this.change(() => this.setSplit(col)),
      () => cols.map((col) => item(col, { run: () => this.change(() => this.setSplit(col)) })), !!this.split) : null;
    const onlyBox = eachRow ? null : this.box('Only rows where',
      this.only.map((o, k) => this.chip(`${o.column} ${TESTS.find((t) => t[0] === o.test)?.[1]} ${o.value}`, () => this.change(() => this.only.splice(k, 1)))),
      (col) => this.askCondition(col),
      () => cols.map((col) => item(col, { run: () => this.askCondition(col) })));
    const title = h('input', { class: 'field', value: c.title || '', placeholder: 'A title (optional)', 'aria-label': 'Title' });
    title.addEventListener('change', () => this.change(() => { c.title = title.value.trim(); }));
    title.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') title.blur(); });
    this.panel.replaceChildren(
      h('div', { class: 'head' }, h('h2', {}, 'Chart'), h('button', { class: 'btn small close', title: 'Close (Esc)', onClick: () => this.stop() }, '× Close')),
      tiles,
      h('p', { class: 'says' }, this.says()),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'From'), h('div', { class: 'from' }, from)),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'Columns (drag into a box)'),
        h('div', { class: 'cols' }, cols.map((col) => h('span', { class: 'col', draggable: 'true', onDragstart: (e) => e.dataTransfer.setData('text/x-column', col) }, col)))),
      xBox, valsBox, splitBox, onlyBox,
      h('label', { class: 'field-group' }, h('span', { class: 'k' }, 'Title'), title),
      h('div', { class: 'actions' },
        h('button', { class: 'btn plain', onClick: () => { this.d.charts.remove(c.id); this.stop(); } }, 'Delete chart'),
        h('button', { class: 'btn main', onClick: () => this.stop() }, 'Done')));
  }

  setSplit(col) {
    this.split = col;
    for (const v of this.vals) if (v.how === 'each') v.how = 'sum';
    if (this.vals.length > 1 && this.c.kind !== 'heatmap') this.vals = this.vals.slice(0, 1);
  }

  /// What the chart shows, in words.
  says() {
    const c = this.c;
    if (!this.vals.length) return 'Add a value: a column of numbers (or how many rows).';
    const names = { sum: 'the total of', average: 'the average', median: 'the middle value of', min: 'the smallest', max: 'the largest', distinct: 'how many different' };
    const what = this.vals.map((v) => (v.how === 'rows' ? 'how many rows' : v.how === 'each' ? v.column : `${names[v.how]} ${v.column}`)).join(' and ');
    if (c.kind === 'histogram') return `How the numbers in ${this.vals[0].column} spread: how many fall in each band.`;
    if (c.kind === 'box') return `The middle half and the range of ${what}.`;
    if (c.kind === 'scatter') return this.x ? `${what} against ${this.x}: one dot for each row.` : 'Put a column of numbers along the bottom.';
    const each = this.x ? ` for each ${this.x}` : this.vals.every((v) => v.how === 'each') ? ' for each row' : ' for all the rows';
    const split = this.split ? `, ${c.kind === 'heatmap' ? 'a row' : c.kind === 'line' ? 'a line' : 'a colour'} for each ${this.split}` : '';
    const only = this.only.length ? `, only rows where ${this.only.map((o) => `${o.column} ${TESTS.find((t) => t[0] === o.test)?.[1]} ${o.value}`).join(' and ')}` : '';
    return `${capital(what)}${each}${split}${only}.`;
  }

  async askCondition(column) {
    const { modal } = await import('./ui.js');
    const test = h('select', { class: 'field' }, TESTS.map(([v, l]) => h('option', { value: v }, l)));
    const value = h('input', { class: 'field', placeholder: 'A value (a year for dates)' });
    const ok = await modal(`Only rows where ${column}…`, h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } }, test, value), [['Cancel', false], ['Add', true, true]]);
    if (ok && value.value.trim()) this.change(() => {
      this.only.push({ column, test: test.value, value: value.value.trim() });
      for (const v of this.vals) if (v.how === 'each') v.how = 'sum';
    });
  }
}

const capital = (t) => t.charAt(0).toUpperCase() + t.slice(1);

function letters(c) { let s = ''; while (c > 0) { const r = (c - 1) % 26; s = String.fromCharCode(65 + r) + s; c = Math.floor((c - 1) / 26); } return s; }
function cellRange(r) { return r ? `${letters(r.left)}${r.top}:${letters(r.right)}${r.bottom}` : ''; }
