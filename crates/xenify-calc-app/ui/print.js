// File › Print: the sheet (or the selection, or every sheet) as pages,
// through the system's print dialog (which can also save a PDF). Formats,
// merged cells and tables print as they show; the frozen top rows repeat
// on every page; charts follow the cells.

import { h, modal, toast } from './ui.js';
import { colName } from './grid.js';

// Also for attribute values: a file's colours are its own text.
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/// One sheet's cells (rows top..bottom, columns left..right) as a table.
function sheetTable(call, sheet, rg, o) {
  const v = call('view', { sheet, top: rg.top, left: rg.left, bottom: rg.bottom, right: rg.right });
  const layout = call('layout', { sheet });
  const cells = new Map(v.cells.map((c) => [`${c.r},${c.c}`, c]));
  const merged = (layout.merged || []).map(([r, c, w, hh]) => ({ r, c, w, h: hh }));
  const covered = new Set();
  for (const m of merged) for (let r = m.r; r < m.r + m.h; r++) for (let c = m.c; c < m.c + m.w; c++) if (r !== m.r || c !== m.c) covered.add(`${r},${c}`);
  const tables = layout.tables;
  const cols = v.widths.map((w) => `<col style="width:${w}px">`).join('');
  const repeat = o.repeat ? Math.max(layout.frozen[0], 0) : 0;
  const row = (r) => {
    let out = `<tr style="height:${v.heights[r - rg.top]}px">`;
    for (let c = rg.left; c <= rg.right; c++) {
      if (covered.has(`${r},${c}`)) continue;
      const cell = cells.get(`${r},${c}`);
      const m = merged.find((x) => x.r === r && x.c === c);
      const t = tables.find((x) => r >= x.top && r <= x.bottom && c >= x.left && c <= x.right);
      const look = cell?.look || {};
      const st = [];
      if (look.b || (t && ((t.header && r === t.top) || (t.totals && r === t.bottom)))) st.push('font-weight:700');
      if (look.i) st.push('font-style:italic');
      if (look.u || look.s) st.push(`text-decoration:${look.u ? 'underline' : ''} ${look.s ? 'line-through' : ''}`);
      if (look.size) st.push(`font-size:${look.size * 1.125}px`);
      if (look.color) st.push(`color:${look.color}`);
      if (look.fill) st.push(`background:${look.fill}`);
      else if (t && t.header && r === t.top) st.push('background:#FAF9F7');
      const align = look.align || (cell?.number ? 'right' : 'left');
      st.push(`text-align:${align}`);
      if (look.valign) st.push(`vertical-align:${look.valign === 'center' ? 'middle' : look.valign}`);
      if (look.wrap) st.push('white-space:pre-wrap');
      if (t && t.totals && r === t.bottom) st.push('border-top:1.5px solid #1C1917');
      const b = look.border;
      if (b) {
        const side = (x, name) => { if (x) st.push(`border-${name}:${x[0]}px solid ${x[1]}`); };
        side(b[0], 'top'); side(b[1], 'right'); side(b[2], 'bottom'); side(b[3], 'left');
      }
      const span = m ? `${m.w > 1 ? ` colspan="${m.w}"` : ''}${m.h > 1 ? ` rowspan="${m.h}"` : ''}` : '';
      out += `<td${span} style="${esc(st.join(';'))}">${esc(cell?.text || '')}</td>`;
    }
    return `${out}</tr>`;
  };
  let head = '';
  for (let r = rg.top; r < rg.top + repeat && r <= rg.bottom; r++) head += row(r);
  let body = '';
  for (let r = rg.top + Math.min(repeat, rg.bottom - rg.top + 1); r <= rg.bottom; r++) body += row(r);
  const headers = o.headings
    ? `<tr class="hd"><th></th>${Array.from({ length: rg.right - rg.left + 1 }, (_, k) => `<th>${colName(rg.left + k)}</th>`).join('')}</tr>`
    : '';
  return `<table class="${o.grid ? 'grid' : ''}"><colgroup>${cols}</colgroup>${head || headers ? `<thead>${headers}${head}</thead>` : ''}<tbody>${body}</tbody></table>`;
}

/// The part of a sheet in use.
function used(call, sheet) {
  const l = call('layout', { sheet });
  return { top: 1, left: 1, bottom: Math.max(1, l.used[0]), right: Math.max(1, l.used[1]) };
}

export async function printDialog({ call, sheet, selection, sheetName, chartSvgs }) {
  const multi = selection.top !== selection.bottom || selection.left !== selection.right;
  const what = h('select', { class: 'field', 'aria-label': 'What to print' },
    h('option', { value: 'sheet' }, 'This sheet'),
    multi ? h('option', { value: 'selection' }, 'The cells selected') : null,
    h('option', { value: 'all' }, 'Every sheet (not working sheets)'));
  const orient = h('select', { class: 'field', 'aria-label': 'Page' }, h('option', { value: 'portrait' }, 'Upright (portrait)'), h('option', { value: 'landscape' }, 'Sideways (landscape)'));
  const fit = h('input', { type: 'checkbox', checked: true });
  const grid = h('input', { type: 'checkbox' });
  const repeat = h('input', { type: 'checkbox', checked: true });
  const body = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '12px' } },
    h('label', {}, 'Print', what),
    h('label', {}, 'Pages', orient),
    h('label', { class: 'check-row' }, fit, 'Fit the width to the page'),
    h('label', { class: 'check-row' }, grid, 'Gridlines'),
    h('label', { class: 'check-row' }, repeat, 'Repeat the frozen top rows on every page'),
    h('p', { class: 'muted' }, 'The system’s print dialog opens next; it can also save a PDF.'));
  const ok = await modal('Print', body, [['Cancel', false], ['Print…', true, true]]);
  if (!ok) return;
  const o = { grid: grid.checked, repeat: repeat.checked, headings: false };
  const parts = [];
  if (what.value === 'all') {
    for (const s of call('sheets').filter((x) => !x.working)) {
      parts.push(`<h1>${esc(s.name)}</h1>${sheetTable(call, s.id, used(call, s.id), o)}`);
    }
  } else {
    const rg = what.value === 'selection' ? selection : used(call, sheet);
    parts.push(sheetTable(call, sheet, rg, o));
    for (const svg of chartSvgs()) parts.push(`<figure>${svg}</figure>`);
  }
  const css = `@page{size:A4 ${orient.value};margin:12mm}
    body{font:400 12px Figtree,system-ui,sans-serif;color:#1C1917;margin:0}
    h1{font-size:15px;margin:0 0 8px;break-before:page}h1:first-child{break-before:auto}
    table{border-collapse:collapse;table-layout:fixed;${fit.checked ? 'width:100%;' : ''}margin-bottom:18px}
    td,th{padding:2px 6px;overflow:hidden;white-space:nowrap;text-overflow:clip;vertical-align:bottom}
    table.grid td{border:0.5px solid #D6D0C9}
    thead{display:table-header-group}tr{break-inside:avoid}
    th{font-weight:500;color:#57534E;background:#F7F5F2}
    figure{margin:0 0 18px;break-inside:avoid}figure svg{max-width:100%;height:auto}`;
  const frame = h('iframe', { style: { position: 'fixed', width: '0', height: '0', border: '0', right: '0', bottom: '0' }, 'aria-hidden': 'true' });
  document.body.append(frame);
  const d = frame.contentDocument;
  d.open();
  d.write(`<!doctype html><html><head><meta charset="utf-8"><title>${esc(sheetName)}</title><style>${css}</style></head><body>${parts.join('')}</body></html>`);
  d.close();
  frame.contentWindow.addEventListener('afterprint', () => setTimeout(() => frame.remove(), 500));
  setTimeout(() => {
    try {
      frame.contentWindow.focus();
      frame.contentWindow.print();
    } catch (e) {
      toast(`Couldn’t print: ${e.message || e}`);
    }
  }, 150);
}
