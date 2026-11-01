// Data › Summarise: one row for each value of some columns, with counts,
// totals, averages… of others. A panel beside the sheet builds it (drag
// columns into the boxes, or use + add); the preview shows over the sheet
// until it's added as a Summary sheet, which then follows its source.

import { h, menu, item, toast } from './ui.js';

const HOWS = [['rows', 'How many rows'], ['sum', 'Total'], ['average', 'Average'], ['median', 'Middle value'], ['min', 'Smallest'], ['max', 'Largest'], ['distinct', 'How many different']];
const TESTS = [['is', 'is'], ['is_not', 'isn’t'], ['contains', 'contains'], ['more', 'is more than'], ['less', 'is less than'], ['year', 'is a date in']];

export class Summarise {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet, goSheet, sheetName, toast, host }
    this.panel = h('aside', { class: 'sumpanel', 'aria-label': 'Summarise', hidden: true });
    this.preview = h('section', { class: 'sumpreview', 'aria-label': 'What the summary shows', hidden: true });
    d.host.append(this.preview, this.panel);
    this.on = false;
    document.addEventListener('keydown', (e) => {
      if (this.on && e.key === 'Escape' && !e.target.closest?.('.modal, .popup, .menu')) { e.preventDefault(); this.stop(); }
    });
  }

  /// Opens for the table or cells the cursor is in (or, on a Summary sheet, to change it).
  start() {
    const { call, grid, sheet } = this.d;
    const existing = call('summary', { sheet: sheet() });
    if (existing) {
      this.editing = sheet();
      this.def = existing;
      this.columns = call('summary_columns', { from: existing.from });
    } else {
      let got;
      try { got = call('summary_from', { sheet: sheet(), row: grid.active.r, col: grid.active.c }); } catch (e) { toast(String(e.message || e)); return; }
      this.editing = null;
      this.columns = got.columns;
      // A first guess: grouped by the first column of words, the rows counted and the first numbers totalled.
      this.def = { from: got.from, group: [], show: [{ how: 'rows', column: '' }], only: [], order: null, desc: true, totals: true };
      const guess = this.guess(got.from);
      if (guess.words) this.def.group = [guess.words];
      if (guess.numbers) { this.def.show.push({ column: guess.numbers, how: 'sum' }); this.def.order = 1; }
      // Counting rows says nothing when every value comes once.
      if (guess.numbers && this.def.group.length && this.allDifferent()) { this.def.show.shift(); this.def.order = null; this.def.as_found = true; this.def.desc = false; }
    }
    this.on = true;
    this.panel.hidden = false;
    document.querySelector('.app').classList.add('summarising');
    this.d.grid.resize();
    this.render();
  }

  stop() {
    this.on = false;
    this.panel.hidden = true;
    this.preview.hidden = true;
    document.querySelector('.app').classList.remove('summarising');
    this.d.grid.resize();
  }

  /// The first column of words and of numbers in the source (by its first data row).
  guess(from) {
    const { call } = this.d;
    let top = 2; let left = 1;
    if (from.table) {
      const t = call('tables').find((x) => x.name === from.table);
      top = t.top + (t.header ? 1 : 0); left = t.left;
    } else { top = from.range.top + (from.header === false ? 0 : 1); left = from.range.left; }
    const out = {};
    this.columns.forEach((name, k) => {
      const v = call('cell', { sheet: from.sheet, row: top, col: left + k }).input;
      const num = v !== '' && !Number.isNaN(Number(v.replace(/,/g, '')));
      if (num && !out.numbers) out.numbers = name;
      if (!num && v !== '' && !out.words && k > 0) out.words = name;
    });
    if (!out.words) out.words = this.columns[0];
    return out;
  }

  /// Whether each group comes once in the data (one row each).
  allDifferent() {
    try {
      const p = this.d.call('summary_preview', { def: { ...this.def, show: [{ how: 'rows', column: '' }], totals: false } });
      return p.rows.slice(1).every((r) => r[r.length - 1] === '1');
    } catch { return false; }
  }

  /// What the summary shows, in words.
  says() {
    const d = this.def;
    const what = d.show.map((s) => ({
      rows: 'how many rows have it', sum: `the total of ${s.column}`, average: `the average ${s.column}`, median: `the middle value of ${s.column}`,
      min: `the smallest ${s.column}`, max: `the largest ${s.column}`, distinct: `how many different ${s.column} values it has`,
    }[s.how]));
    const list = what.length > 1 ? `${what.slice(0, -1).join(', ')} and ${what[what.length - 1]}` : what[0] || 'nothing yet';
    const each = d.group.length ? `One row for each different ${d.group.join(' and ')}, showing ${list}.` : `One row for all the rows, showing ${list}.`;
    const only = d.only.length ? ` Only rows where ${d.only.map((o) => `${o.column} ${TESTS.find((t) => t[0] === o.test)?.[1]} ${o.value}`).join(' and ')}.` : '';
    return `${each}${only}${d.totals ? ' The last row adds up all of them.' : ''}`;
  }

  sourceName() {
    const f = this.def.from;
    return f.table || this.d.sheetName(f.sheet);
  }

  chip(text, onRemove, extra) {
    return h('span', { class: 'chip' }, text, extra || null, onRemove ? h('button', { class: 'x', 'aria-label': `Take ${text} away`, onClick: onRemove }, '×') : null);
  }

  /// A box columns can be dropped into (or added with + add).
  box(label, kids, onDrop, addItems) {
    const add = h('button', { class: 'add', onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); menu(addItems(), r.left, r.bottom + 4); } }, '+ add');
    const box = h('div', { class: 'box' }, kids, add);
    box.addEventListener('dragover', (e) => { e.preventDefault(); box.classList.add('drop'); });
    box.addEventListener('dragleave', () => box.classList.remove('drop'));
    box.addEventListener('drop', (e) => { e.preventDefault(); box.classList.remove('drop'); const c = e.dataTransfer.getData('text/x-column'); if (c) onDrop(c); });
    return h('div', { class: 'field-group' }, h('span', { class: 'k' }, label), box);
  }

  change(f) {
    f(this.def);
    this.render();
  }

  render() {
    const d = this.def;
    const cols = this.columns;
    const howSelect = (s) => h('select', { 'aria-label': 'How', onChange: (e) => this.change(() => { s.how = e.target.value; }) },
      HOWS.map(([v, l]) => h('option', { value: v, selected: s.how === v }, l)));
    const orderSel = h('select', { class: 'pick wide', onChange: (e) => this.change((x) => { const [o, dir] = e.target.value.split(':'); x.as_found = o === 'found'; x.order = o === 'group' || o === 'found' ? null : Number(o); x.desc = dir === 'desc'; }) },
      h('option', { value: 'found:asc', selected: d.order == null && !!d.as_found }, 'As they come in the data'),
      h('option', { value: 'group:asc', selected: d.order == null && !d.as_found && !d.desc }, 'By the grouped values, A to Z'),
      h('option', { value: 'group:desc', selected: d.order == null && !d.as_found && d.desc }, 'By the grouped values, Z to A'),
      d.show.flatMap((s, k) => {
        const name = s.how === 'rows' ? 'Rows' : `${HOWS.find((x) => x[0] === s.how)?.[1]} ${s.column}`;
        return [h('option', { value: `${k}:desc`, selected: d.order === k && d.desc }, `${name}, biggest first`), h('option', { value: `${k}:asc`, selected: d.order === k && !d.desc }, `${name}, smallest first`)];
      }));
    const totals = h('input', { type: 'checkbox', checked: d.totals, onChange: (e) => this.change((x) => { x.totals = e.target.checked; }) });
    this.panel.replaceChildren(
      h('div', { class: 'head' }, h('h2', {}, this.editing ? 'Change the summary' : 'Summarise'), h('button', { class: 'btn small close', title: 'Close (Esc)', onClick: () => this.stop() }, '× Close')),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'From'), h('div', { class: 'from' }, `${this.sourceName()}${d.from.table ? ' (table, grows with it)' : ''}`)),
      h('div', { class: 'field-group' }, h('span', { class: 'k' }, 'Columns (drag into a box)'),
        h('div', { class: 'cols' }, cols.map((c) => h('span', { class: 'col', draggable: 'true', onDragstart: (e) => e.dataTransfer.setData('text/x-column', c) }, c)))),
      this.box('One row for each', d.group.map((g, k) => this.chip(g, () => this.change((x) => x.group.splice(k, 1)))),
        (c) => this.change((x) => { if (!x.group.includes(c)) x.group.push(c); }),
        () => cols.filter((c) => !d.group.includes(c)).map((c) => item(c, { run: () => this.change((x) => x.group.push(c)) }))),
      this.box('Show', d.show.map((s, k) => this.chip(s.how === 'rows' ? 'How many rows' : s.column, () => this.change((x) => { x.show.splice(k, 1); if (x.order === k) x.order = null; else if (x.order > k) x.order--; }), s.how === 'rows' ? null : howSelect(s))),
        (c) => this.change((x) => x.show.push({ column: c, how: 'sum' })),
        () => [item('Rows (how many)', { run: () => this.change((x) => x.show.push({ column: '', how: 'rows' })) }), null, ...cols.map((c) => item(c, { run: () => this.change((x) => x.show.push({ column: c, how: 'sum' })) }))]),
      this.box('Only rows where', d.only.map((o, k) => this.chip(`${o.column} ${TESTS.find((t) => t[0] === o.test)?.[1]} ${o.value}`, () => this.change((x) => x.only.splice(k, 1)))),
        (c) => this.askCondition(c),
        () => cols.map((c) => item(c, { run: () => this.askCondition(c) }))),
      h('div', { class: 'row2' },
        h('label', { class: 'field-group' }, h('span', { class: 'k' }, 'Order'), orderSel),
        h('label', { class: 'check' }, totals, 'Totals row')),
      h('div', { class: 'actions' },
        h('button', { class: 'btn plain', onClick: () => this.stop() }, 'Cancel'),
        this.editing ? null : h('button', { class: 'btn', title: 'Copy the numbers to paste on any sheet (they won’t follow changes)', onClick: () => this.copy() }, 'Copy'),
        h('button', { class: 'btn main', title: 'A sheet that keeps following the data', onClick: () => this.add() }, this.editing ? 'Change it' : 'Add as a sheet')));
    this.showPreview();
  }

  async askCondition(column) {
    const { modal } = await import('./ui.js');
    const test = h('select', { class: 'field' }, TESTS.map(([v, l]) => h('option', { value: v }, l)));
    const value = h('input', { class: 'field', placeholder: 'A value (a year for dates)' });
    const ok = await modal(`Only rows where ${column}…`, h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } }, test, value), [['Cancel', false], ['Add', true, true]]);
    if (ok && value.value.trim()) this.change((x) => x.only.push({ column, test: test.value, value: value.value.trim() }));
  }

  showPreview() {
    if (this.editing) { this.preview.hidden = true; return; }
    let p;
    try { p = this.d.call('summary_preview', { def: this.def }); } catch (e) {
      this.preview.replaceChildren(h('p', {}, String(e.message || e)));
      this.preview.hidden = false;
      return;
    }
    const [head, ...rows] = p.rows;
    const isNum = (v) => /^-?[\d,.]+%?$/.test(v);
    this.preview.replaceChildren(
      h('div', { class: 'label' }, 'Preview: nothing is added until you choose'),
      h('p', { class: 'says' }, this.says()),
      h('table', {},
        h('thead', {}, h('tr', {}, head.map((c) => h('th', {}, c)))),
        h('tbody', {}, rows.slice(0, 200).map((r, k) => h('tr', { class: p.totals && k === rows.length - 1 ? 'total' : null }, r.map((c) => h('td', { class: isNum(c) ? 'n' : null }, c)))))),
      h('p', { class: 'note' }, `Add as a sheet: it keeps following ${this.sourceName()} (new rows, changes and filters show up by themselves). Copy: the numbers as they are now, to paste anywhere.`));
    this.preview.hidden = false;
  }

  add() {
    if (this.editing) {
      const ok = this.d.run('set_summary', { sheet: this.editing, def: this.def });
      if (ok !== undefined) this.stop();
      return;
    }
    const name = this.def.group.length ? `${this.sourceName()} by ${this.def.group.join(' and ').toLowerCase()}` : `${this.sourceName()} summary`;
    const id = this.d.run('add_summary', { def: this.def, name });
    if (id != null) { this.stop(); this.d.goSheet(id); }
  }

  /// The summary's numbers on the clipboard, to paste on any sheet (they won't follow changes).
  async copy() {
    let p;
    try { p = this.d.call('summary_preview', { def: this.def }); } catch (e) { toast(String(e.message || e)); return; }
    const text = p.rows.map((r) => r.join('\t')).join('\n');
    this.d.copied?.(text);
    try { await navigator.clipboard.writeText(text); } catch { /* Calc's own paste still has it */ }
    this.stop();
    toast('Copied: click where it should go, on any sheet, and paste (Ctrl+V)');
  }
}
