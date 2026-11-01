// The All sheets page: every sheet as a card, with the working sheets
// (off the tab strip: data and helper sums other sheets use) in their own
// area. Drag a card between the two to tuck it away or bring it back.

import { h, menu, item } from './ui.js';

const KIND = { normal: 'Sheet', query: 'Query · locked', table_editor: 'Table editor', summary: 'Summary', report: 'Report', linked_csv: 'Linked file · locked' };

export class AllSheets {
  constructor(d) {
    this.d = d;   // { call, run, goSheet, renameSheet, deleteSheet, addSheet, sheet, onClose }
    this.el = document.getElementById('allsheets');
    this.open = false;
  }

  show() {
    this.open = true;
    document.querySelector('.app').classList.add('paged');
    this.el.hidden = false;
    this.render();
    this.d.onOpen?.();
  }

  hide() {
    if (!this.open) return;
    this.open = false;
    document.querySelector('.app').classList.remove('paged');
    this.el.hidden = true;
    this.d.onClose?.();
  }

  render() {
    if (!this.open) return;
    const facts = this.d.call('sheet_facts');
    const sheets = facts.filter((f) => !f.working);
    const working = facts.filter((f) => f.working);
    const zone = (title, note, list, isWorking) => {
      const grid = h('div', { class: 'cards' }, list.map((f) => this.card(f)));
      const sec = h('section', { class: `zone${isWorking ? ' working' : ''}`, 'aria-label': title },
        h('div', { class: 'zone-head' }, h('h2', {}, title), h('span', {}, note)), grid);
      // Dropping a card here moves it between the tab strip and the working sheets.
      sec.addEventListener('dragover', (e) => { e.preventDefault(); sec.classList.add('drop'); });
      sec.addEventListener('dragleave', () => sec.classList.remove('drop'));
      sec.addEventListener('drop', (e) => {
        e.preventDefault();
        sec.classList.remove('drop');
        const id = Number(e.dataTransfer.getData('text/x-sheet'));
        const f = facts.find((x) => x.id === id);
        if (!f || f.working === isWorking) return;
        if (!isWorking || sheets.length > 1) this.d.run('working', { sheet: id, working: isWorking });
        this.render();
      });
      return sec;
    };
    this.el.replaceChildren(
      h('div', { class: 'page-head' },
        h('h1', {}, 'All sheets'),
        h('button', { class: 'btn', onClick: () => { this.d.addSheet(); this.render(); } }, 'New sheet'),
        h('button', { class: 'btn plain', onClick: () => this.hide() }, 'Back to the sheet')),
      zone('Sheets', 'On the tab strip. Drag one into Working sheets to tuck it away.', sheets, false),
      zone('Working sheets', working.length ? 'Not on the tab strip: data and helper sums the other sheets use.' : 'None yet. Drag a sheet here to keep it off the tab strip; it still works the same.', working, true));
  }

  card(f) {
    const describe = [];
    if (f.tables.length) describe.push(f.tables.length === 1 ? `table ${f.tables[0]}` : `${f.tables.length} tables`);
    describe.push(f.rows ? `${f.rows} ${f.rows === 1 ? 'row' : 'rows'}` : 'empty');
    const thumb = h('canvas', { class: 'thumb', width: 220, height: 118, 'aria-hidden': 'true' });
    this.paintThumb(thumb, f);
    const c = h('button', {
      class: `card${f.id === this.d.sheet() ? ' current' : ''}`, draggable: 'true',
      onClick: () => { this.hide(); this.d.goSheet(f.id); },
      onContextmenu: (e) => { e.preventDefault(); this.cardMenu(f, e.clientX, e.clientY); },
      onDragstart: (e) => { e.dataTransfer.setData('text/x-sheet', String(f.id)); e.dataTransfer.effectAllowed = 'move'; },
    },
    thumb,
    h('b', {}, f.name),
    h('span', { class: 'tags' }, h('span', { class: `tag k-${f.kind}` }, KIND[f.kind] || 'Sheet'), h('span', { class: 'dim' }, describe.join(', '))),
    f.used_by.length ? h('span', { class: 'dim' }, `Used by ${f.used_by.join(', ')}`) : null);
    return c;
  }

  /// A small picture of the sheet's top left: filled cells as bars.
  paintThumb(canvas, f) {
    const ctx = canvas.getContext('2d');
    const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
    ctx.fillStyle = css('--sheet');
    ctx.fillRect(0, 0, 220, 118);
    ctx.fillStyle = css('--soft-2');
    for (let x = 0; x <= 220; x += 44) ctx.fillRect(x, 0, 1, 118);
    for (let y = 0; y <= 118; y += 14) ctx.fillRect(0, y, 220, 1);
    if (!f.rows) return;
    let v;
    try { v = this.d.call('view', { sheet: f.id, top: 1, left: 1, bottom: 8, right: 5 }); } catch { return; }
    for (const t of v.tables) {
      ctx.fillStyle = css('--accent-soft');
      ctx.fillRect((t.left - 1) * 44, (t.top - 1) * 14, (Math.min(t.right, 5) - t.left + 1) * 44, 14);
    }
    for (const cell of v.cells) {
      if (!cell.text) continue;
      const x = (cell.c - 1) * 44; const y = (cell.r - 1) * 14;
      const w = Math.min(36, 6 + cell.text.length * 4);
      ctx.fillStyle = cell.look.fill || 'transparent';
      if (cell.look.fill) ctx.fillRect(x + 1, y + 1, 43, 13);
      ctx.fillStyle = cell.number ? css('--faint') : css('--line-2');
      ctx.fillRect(cell.number ? x + 40 - w : x + 4, y + 5, w, 4);
    }
  }

  cardMenu(f, x, y) {
    const facts = this.d.call('sheet_facts');
    const shown = facts.filter((s) => !s.working).length;
    menu([
      item('Open', { run: () => { this.hide(); this.d.goSheet(f.id); } }),
      f.working
        ? item('Show on the tab strip', { run: () => { this.d.run('working', { sheet: f.id, working: false }); this.render(); } })
        : item('Make it a working sheet', { enabled: shown > 1, run: () => { this.d.run('working', { sheet: f.id, working: true }); this.render(); } }),
      item('Rename…', { run: async () => { await this.d.renameSheet(f.id); this.render(); } }),
      item('Duplicate', { run: () => { this.d.run('duplicate_sheet', { sheet: f.id }); this.render(); } }),
      null,
      item('Delete…', { enabled: facts.length > 1, run: async () => { await this.d.deleteSheet(f); this.render(); } }),
    ], x, y);
  }
}
