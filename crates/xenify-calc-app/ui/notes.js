// What columns mean, kept in the book: About this column (right-click a
// column header) and Data › Definitions (named measures like Profit, with
// their formula). Everyone, and Ask, uses the same ones.

import { h, modal, toast, dialogKeys } from './ui.js';

const TAX = [['', 'Not known'], ['no', 'No (net)'], ['yes', 'Yes (gross)']];

/// The note's target for column `c` at row `r`: the table's column, else the sheet's.
export function targetAt(d, r, c) {
  const t = d.call('tables').find((x) => x.sheet === d.sheet() && r >= x.top && r <= x.bottom && c >= x.left && c <= x.right);
  if (t) return { on: 'table', table: t.name, column: t.columns[c - t.left].name };
  return { on: 'column', sheet: d.sheet(), column: c };
}

const same = (a, b) => a.on === b.on && (a.on === 'table' ? a.table.toLowerCase() === b.table.toLowerCase() && a.column.toLowerCase() === b.column.toLowerCase() : a.sheet === b.sheet && a.column === b.column);

/// About this column: what it means, its unit, tax, how it's worked out.
export async function aboutColumn(d, r, c, colName) {
  const target = targetAt(d, r, c);
  const note = d.call('notes').find((n) => same(n.target, target)) || {};
  const title = target.on === 'table' ? `${target.column} (${target.table})` : `Column ${colName(c)}`;
  const f = (label, value, placeholder) => {
    const input = h('input', { class: 'field', value: value || '', placeholder });
    return [h('label', {}, label, input), input];
  };
  const [lMean, mean] = f('Means', note.meaning, 'Money in from orders, after discounts');
  const [lUnit, unit] = f('Unit or currency', note.unit, 'GBP, kg, hours');
  const tax = h('select', { class: 'field' }, TAX.map(([v, n]) => h('option', { value: v, selected: (note.with_tax === true ? 'yes' : note.with_tax === false ? 'no' : '') === v }, n)));
  const [lHow, how] = f('Worked out as', note.how, 'Price × Quantity − Discount');
  const [lOut, out] = f('Rows to leave out of totals', note.leave_out, 'Status is Cancelled');
  const [lSrc, src] = f('Comes from', note.source, 'The shop database, orders.total');
  const key = h('input', { type: 'checkbox', checked: note.key || null });
  // A query sheet's column: what the database says about it.
  let db = null;
  try {
    const q = d.call('query', { sheet: d.sheet() });
    const n = q?.def?.notes?.[target.column] ?? q?.def?.notes?.columns?.[target.column];
    if (n) db = h('p', { class: 'q-hint' }, `From the database: ${typeof n === 'string' ? n : JSON.stringify(n)}`);
  } catch {}
  const body = h('div', { class: 'note-form' }, lMean, h('div', { class: 'two' }, lUnit, h('label', {}, 'Includes VAT or sales tax', tax)), lHow, lOut, lSrc,
    h('label', { class: 'check-row' }, key, 'It identifies rows (an order number, a customer id)'), db,
    h('p', { class: 'q-hint' }, 'Kept in the book. Ask reads it before working anything out, so “profit” or “revenue” mean what your business means.'));
  const what = await modal(`About ${title}`, body, [['Cancel', null], ...(note.target ? [['Remove', 'remove']] : []), ['Save', 'save', true]]);
  if (what === 'remove') d.run('set_note', { target, note: null });
  if (what !== 'save') return;
  const n = { target, meaning: mean.value.trim(), unit: unit.value.trim(), how: how.value.trim(), with_tax: tax.value === '' ? null : tax.value === 'yes', key: key.checked, leave_out: out.value.trim(), source: src.value.trim() };
  const empty = !n.meaning && !n.unit && !n.how && n.with_tax == null && !n.key && !n.leave_out && !n.source;
  d.run('set_note', { target, note: empty ? null : n });
  toast(empty ? 'Note taken away' : 'Saved with the book');
}

/// Data › Definitions: named measures everyone uses (Profit, Margin…),
/// in one dialog: the list, and a form for a new or changed one.
export function definitions(d) {
  const back = h('div', { class: 'modal-back' });
  const close = () => { back.remove(); unkey(); };
  const list = h('div', { class: 'def-list' });
  const name = h('input', { class: 'field', placeholder: 'Profit', 'aria-label': 'Name' });
  const formula = h('input', { class: 'field mono', placeholder: '=SUM(Sales[Revenue])-SUM(Sales[Costs])', 'aria-label': 'Worked out as' });
  const about = h('input', { class: 'field', placeholder: 'Before VAT; refunds taken off', 'aria-label': 'In words' });
  const result = h('p', { class: 'q-hint' });
  let editing = null;
  const check = () => {
    if (!formula.value.trim()) { result.textContent = ''; return; }
    try { const v = d.call('ai_calc', { formula: formula.value }); result.textContent = v.error ? `Doesn’t work out: ${v.error}` : `Now: ${v.shows}`; } catch (e) { result.textContent = String(e.message || e); }
  };
  formula.addEventListener('input', check);
  const fill = (x) => { editing = x?.name || null; name.value = x?.name || ''; formula.value = x?.formula || ''; about.value = x?.about || ''; check(); name.focus(); };
  const draw = () => {
    const defs = d.call('definitions');
    list.replaceChildren(...(defs.length ? defs.map((x) => h('div', { class: 'def-row' },
      h('div', { class: 'grow' }, h('b', {}, x.name), h('code', {}, x.formula), x.about ? h('span', { class: 'q-hint' }, x.about) : null),
      h('button', { class: 'btn small', onClick: () => fill(x) }, 'Change'),
      h('button', { class: 'btn small plain', onClick: () => { d.run('set_definition', { name: x.name, definition: null }); draw(); } }, 'Remove')))
      : [h('p', { class: 'q-hint' }, 'None yet. A definition says how your business works something out, so everyone (and Ask) gets the same answer.')]));
  };
  const save = () => {
    const n = name.value.trim();
    const f0 = formula.value.trim();
    if (!n || !f0) { toast('A definition needs a name and a formula'); return; }
    if (editing && editing !== n) d.run('set_definition', { name: editing, definition: null });
    const f = f0.startsWith('=') ? f0 : `=${f0}`;
    d.run('set_definition', { name: n, definition: { name: n, formula: f, about: about.value.trim() } });
    fill(null);
    draw();
  };
  const box = h('div', { class: 'modal defs', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Definitions' },
    h('h2', {}, 'Definitions'), list,
    h('div', { class: 'note-form def-form' }, h('b', {}, 'A new one, or the one you’re changing'),
      h('div', { class: 'two' }, h('label', {}, 'Name', name), h('label', {}, 'In words', about)), h('label', {}, 'Worked out as', formula), result),
    h('div', { class: 'modal-actions' }, h('button', { class: 'btn', onClick: close }, 'Close'), h('button', { class: 'btn main', onClick: save }, 'Save it')));
  draw();
  back.append(box);
  back.addEventListener('pointerdown', (e) => { if (e.target === back) close(); });
  // Escape closes, Enter is Save it; other keys stay in the dialog too.
  const unkey = dialogKeys(back, box, close);
  back.addEventListener('keydown', (e) => e.stopPropagation());
  document.getElementById('popups').append(back);
  name.focus();
}
