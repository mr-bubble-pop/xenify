// Report sheets: a printable page of blocks (a title, text, big numbers,
// cells or a table, a summary, a chart) instead of a grid. Click a block to
// change it; the bar under it moves or removes it; Add a block puts more on.

import { h, menu, item, modal, toast } from './ui.js';

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export class Reports {
  constructor(d) {
    this.d = d;   // { call, run, sheet, charts, sheetName, onShown }
    this.el = document.getElementById('reportpage');
    this.open = false;
    this.selected = null;
  }

  /// Shows the page when the sheet shown is a report (hides it otherwise).
  sync() {
    const kind = this.d.call('layout', { sheet: this.d.sheet() }).kind;
    if (kind === 'report') this.show(); else this.hide();
  }

  show() {
    this.open = true;
    document.querySelector('.app').classList.add('reporting');
    this.el.hidden = false;
    this.render();
  }

  hide() {
    if (!this.open) return;
    this.open = false;
    document.querySelector('.app').classList.remove('reporting');
    this.el.hidden = true;
    this.d.onHidden?.();
  }

  def() {
    return this.d.call('report', { sheet: this.d.sheet() });
  }

  save(def) {
    this.d.run('set_report', { sheet: this.d.sheet(), def });
    this.render();
  }

  render() {
    if (!this.open) return;
    let page;
    try { page = this.def(); } catch (e) { this.el.replaceChildren(h('p', {}, String(e.message || e))); return; }
    const def = page.def;
    const blocks = page.blocks.map((b, k) => this.block(b, k, def));
    const sheet = h('article', { class: `page${def.landscape ? ' wide' : ''}`, 'aria-label': 'The report page' }, blocks.length ? blocks : h('p', { class: 'empty' }, 'Nothing on the page yet: add a block.'));
    const add = h('button', { class: 'btn main', onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); menu(this.addItems(def), r.left, r.bottom + 4); } }, 'Add a block');
    this.el.replaceChildren(
      h('div', { class: 'page-tools' },
        add,
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: def.landscape, onChange: (e) => this.save({ ...def, landscape: e.target.checked }) }), 'Sideways page'),
        h('button', { class: 'btn', title: 'Paste a copied block, chart or cells (Ctrl+V)', onClick: () => { const c = this.d.clipped(); this.paste(c.o, c.cells); } }, 'Paste'),
        h('button', { class: 'btn', onClick: () => this.print() }, 'Print…'),
        h('span', { class: 'hint' }, 'Numbers, tables and charts follow the book as it changes.')),
      sheet);
  }

  block(b, k, def) {
    const kinds = def.blocks;
    const sel = this.selected === k;
    let body;
    switch (b.kind) {
      case 'title': body = this.editable('h1', b.text, (t) => this.setText(def, k, t)); break;
      case 'text': body = this.editable('p', b.text, (t) => this.setText(def, k, t)); break;
      case 'number': body = h('div', { class: 'number' }, h('span', { class: 'value' }, b.value || '—'), h('span', { class: 'label' }, b.label)); break;
      case 'cells': body = this.table(b); break;
      case 'chart': {
        body = h('div', { class: 'chartbox' });
        requestAnimationFrame(() => this.chart(body, b.chart));
        break;
      }
      default: body = h('p', { class: 'missing' }, b.text || 'Missing');
    }
    const bar = sel ? h('div', { class: 'block-bar' },
      kinds[k].kind === 'number' ? h('button', { class: 'btn small', onClick: () => this.editNumber(def, k) }, 'Change…') : null,
      h('button', { class: 'btn small', title: 'Copy (Ctrl+C): paste on another report (charts on any sheet too)', onClick: () => this.copy(k) }, 'Copy'),
      h('button', { class: 'btn small', disabled: k === 0, onClick: () => this.move(def, k, -1) }, 'Up'),
      h('button', { class: 'btn small', disabled: k === kinds.length - 1, onClick: () => this.move(def, k, 1) }, 'Down'),
      h('button', { class: 'btn small plain', onClick: () => this.remove(def, k) }, 'Remove')) : null;
    return h('section', { class: `block k-${b.kind}${sel ? ' on' : ''}`, onClick: () => { if (this.selected !== k) { this.selected = k; this.render(); } } }, body, bar);
  }

  editable(tag, text, done) {
    const el = h(tag, { contenteditable: 'true', spellcheck: 'true' }, text);
    el.addEventListener('blur', () => { if (el.textContent !== text) done(el.textContent); });
    el.addEventListener('keydown', (e) => { if (e.key === 'Enter' && tag === 'h1') { e.preventDefault(); el.blur(); } e.stopPropagation(); });
    return el;
  }

  table(b) {
    const [head, ...rows] = b.rows || [];
    if (!head) return h('p', { class: 'missing' }, 'Nothing there');
    const num = (v) => /^-?[\d,.]+%?$/.test(v);
    return h('div', { class: 'tablebox' }, b.name ? h('div', { class: 'cap' }, b.name) : null,
      h('table', {}, h('thead', {}, h('tr', {}, head.map((c) => h('th', {}, c)))),
        h('tbody', {}, rows.map((r, i) => h('tr', { class: b.totals && i === rows.length - 1 ? 'total' : null }, r.map((c) => h('td', { class: num(c) ? 'n' : null }, c)))))));
  }

  chart(box, id) {
    let c; let data;
    try {
      c = this.d.call('charts', { sheet: this.chartSheet(id) }).find((x) => x.id === id);
      data = this.d.call('chart_data', { id });
    } catch { return; }
    if (!c) return;
    box.replaceChildren(h('b', {}, c.title || ''), h('div', { class: 'plot' }));
    const plot = box.querySelector('.plot');
    this.d.charts.plot(plot, { ...c, w: plot.clientWidth + 32, h: 300 }, data, data.series.slice(0, 6));
  }

  chartSheet(id) {
    for (const s of this.d.call('sheets')) {
      if (this.d.call('charts', { sheet: s.id }).some((c) => c.id === id)) return s.id;
    }
    return this.d.sheet();
  }

  async copy(k) {
    const o = this.d.copy(k);
    if (!o) return;
    try { await navigator.clipboard.writeText(o.text); } catch { /* Calc's own paste still has it */ }
    toast('Copied: open another report and paste (Ctrl+V)');
  }

  /// Ctrl+V on a report: a copied block, a chart, or copied cells as a table
  /// block, put after the block picked (or at the end).
  paste(o, cells) {
    let b = null;
    if (o?.kind === 'block') b = structuredClone(o.block);
    else if (o?.kind === 'chart') b = { kind: 'chart', chart: o.chart.id };
    else if (cells) {
      const t = this.d.call('tables').find((x) => x.sheet === cells.sheet && x.top === cells.range.top && x.left === cells.range.left && x.bottom === cells.range.bottom && x.right === cells.range.right);
      b = t ? { kind: 'cells', sheet: t.sheet, table: t.name } : { kind: 'cells', sheet: cells.sheet, range: cells.range };
    }
    if (!b) { toast('Copy a chart, cells or a report block first, then paste it here'); return; }
    // Charts, tables and summaries show their own book's numbers: from another book only words go.
    if (o && o.tab !== this.d.tab() && !['title', 'text'].includes(b.kind)) { toast('Charts, tables and summaries can be pasted in the book they came from'); return; }
    const def = this.def().def;
    const at = this.selected != null ? this.selected + 1 : def.blocks.length;
    const blocks = [...def.blocks];
    blocks.splice(at, 0, b);
    this.selected = at;
    this.save({ ...def, blocks });
  }

  setText(def, k, text) {
    const blocks = def.blocks.map((b, i) => (i === k ? { ...b, text } : b));
    this.save({ ...def, blocks });
  }

  move(def, k, by) {
    const blocks = [...def.blocks];
    const [b] = blocks.splice(k, 1);
    blocks.splice(k + by, 0, b);
    this.selected = k + by;
    this.save({ ...def, blocks });
  }

  remove(def, k) {
    this.selected = null;
    this.save({ ...def, blocks: def.blocks.filter((_, i) => i !== k) });
  }

  async editNumber(def, k, fresh = false) {
    const b = def.blocks[k] || { kind: 'number', label: '', formula: '' };
    const label = h('input', { class: 'field', value: b.label, placeholder: 'What it is (Revenue this month)' });
    const formula = h('input', { class: 'field mono', value: b.formula, placeholder: '=SUM(Sales[Revenue])' });
    const ok = await modal(fresh ? 'A big number' : 'Change the number', h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } },
      h('label', {}, 'Label', label), h('label', {}, 'Worked out by', formula)), [['Cancel', false], [fresh ? 'Add' : 'Change', true, true]]);
    if (!ok) return;
    const nb = { kind: 'number', label: label.value, formula: formula.value.replace(/^=/, '') };
    const blocks = fresh ? [...def.blocks, nb] : def.blocks.map((x, i) => (i === k ? nb : x));
    this.save({ ...def, blocks });
  }

  addItems(def) {
    const { call } = this.d;
    const push = (b) => this.save({ ...def, blocks: [...def.blocks, b] });
    const tables = call('tables');
    const sheets = call('sheets');
    const summaries = sheets.filter((s) => s.kind === 'summary');
    const charts = sheets.flatMap((s) => call('charts', { sheet: s.id }).map((c) => ({ ...c, sheetName: s.name })));
    return [
      item('Title', { run: () => push({ kind: 'title', text: 'Title' }) }),
      item('Text', { run: () => push({ kind: 'text', text: 'Write here.' }) }),
      item('A big number…', { run: () => this.editNumber(def, def.blocks.length, true) }),
      null,
      item('A table', { enabled: tables.length > 0, sub: () => tables.map((t) => item(t.name, { run: () => push({ kind: 'cells', sheet: t.sheet, table: t.name }) })) }),
      item('A summary', { enabled: summaries.length > 0, sub: () => summaries.map((s) => item(s.name, { run: () => push({ kind: 'summary', sheet: s.id }) })) }),
      item('A chart', { enabled: charts.length > 0, sub: () => charts.map((c) => item(`${c.title || 'Chart'} (${c.sheetName})`, { run: () => push({ kind: 'chart', chart: c.id }) })) }),
    ];
  }

  /// Prints the page as shown.
  print() {
    const page = this.el.querySelector('.page');
    if (!page) return;
    const clone = page.cloneNode(true);
    clone.querySelectorAll('.block-bar').forEach((b) => b.remove());
    clone.querySelectorAll('[contenteditable]').forEach((e) => e.removeAttribute('contenteditable'));
    const css = [...document.styleSheets].flatMap((s) => { try { return [...s.cssRules].map((r) => r.cssText); } catch { return []; } }).join('\n');
    const landscape = page.classList.contains('wide');
    const frame = h('iframe', { style: { position: 'fixed', width: '0', height: '0', border: '0', right: '0', bottom: '0' }, 'aria-hidden': 'true' });
    document.body.append(frame);
    const d = frame.contentDocument;
    d.open();
    d.write(`<!doctype html><html><head><meta charset="utf-8"><title>${esc(this.d.sheetName(this.d.sheet()))}</title><style>${css}
      @page{size:A4 ${landscape ? 'landscape' : 'portrait'};margin:14mm} html,body{background:#fff!important;overflow:visible!important;height:auto!important}
      .page{box-shadow:none!important;margin:0!important;width:auto!important;max-width:none!important;padding:0!important}
      .block{break-inside:avoid}</style></head><body>${clone.outerHTML}</body></html>`);
    d.close();
    frame.contentWindow.addEventListener('afterprint', () => setTimeout(() => frame.remove(), 500));
    setTimeout(() => { try { frame.contentWindow.focus(); frame.contentWindow.print(); } catch (e) { toast(`Couldn’t print: ${e.message || e}`); } }, 200);
  }
}
