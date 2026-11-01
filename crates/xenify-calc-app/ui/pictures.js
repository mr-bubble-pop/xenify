// Pictures on a sheet (Insert › Picture…, pasting a picture, or dropping
// picture files on the sheet). Click one to pick it: drag it to move it,
// drag its corner to resize it (Shift keeps its shape), and its bar gives
// Describe… (what it shows, for screen readers), Copy and Delete. It moves
// with its cell as rows and columns are added or taken away.

import { h, toast, ask } from './ui.js';

const KINDS = /^image\/(png|jpeg|gif|webp|svg\+xml)$/;
const MAX_W = 480;   // Original size: no wider than this
const NEW_W = 240;   // a new picture: no wider than this (its shape kept)

export class Pictures {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet, layer, copy }
    this.nodes = new Map();
    this.list = [];
    this.selected = null;
    document.addEventListener('pointerdown', (e) => {
      if (this.selected != null && !e.target.closest?.('.mypicture, .popup, .menu, .modal')) this.select(null);
    });
    document.addEventListener('keydown', (e) => {
      if (this.selected == null || e.target.closest?.('input, textarea')) return;
      if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); e.stopPropagation(); this.remove(this.selected); }
      if (e.key === 'Escape') this.select(null);
    }, true);
    // Picture files dropped on the sheet.
    const host = d.grid.host;
    host.addEventListener('dragover', (e) => { if ([...(e.dataTransfer?.items || [])].some((i) => KINDS.test(i.type))) e.preventDefault(); });
    host.addEventListener('drop', (e) => {
      const files = [...(e.dataTransfer?.files || [])].filter((f) => KINDS.test(f.type));
      if (!files.length) return;
      e.preventDefault();
      const r = host.getBoundingClientRect();
      const hit = d.grid.hit(e.clientX - r.left, e.clientY - r.top);
      files.forEach((f, k) => this.addFile(f, hit.r || d.grid.active.r, (hit.c || d.grid.active.c) + k * 3));
    });
  }

  refresh() {
    try { this.list = this.d.call('my_pictures', { sheet: this.d.sheet() }); } catch { this.list = []; }
    const ids = new Set(this.list.map((p) => p.id));
    for (const [id, n] of this.nodes) if (!ids.has(id)) { n.remove(); this.nodes.delete(id); }
    for (const p of this.list) this.draw(p);
    if (this.selected != null && !ids.has(this.selected)) this.selected = null;
    this.place();
  }

  place() {
    const g = this.d.grid;
    for (const p of this.list) {
      const n = this.nodes.get(p.id);
      if (!n) continue;
      n.style.left = `${g.x(p.col) + p.dx}px`;
      n.style.top = `${g.y(p.row) + p.dy}px`;
      n.style.width = `${p.w}px`;
      n.style.height = `${p.h}px`;
      n.classList.toggle('bar-below', g.y(p.row) + p.dy < 76);
    }
  }

  draw(p) {
    let n = this.nodes.get(p.id);
    if (!n) {
      n = h('figure', { class: 'mypicture', tabindex: '0' });
      this.d.layer.append(n);
      this.nodes.set(p.id, n);
      this.bind(n, p.id);
    }
    n.classList.toggle('on', this.selected === p.id);
    n.setAttribute('aria-label', p.alt ? `Picture: ${p.alt}` : `Picture${p.name ? `: ${p.name}` : ''}`);
    const src = `data:${p.mime};base64,${p.data}`;
    const img = n.querySelector('img');
    if (!img || img.dataset.src !== String(p.data.length) + p.mime) {
      n.replaceChildren(
        h('img', { src, alt: p.alt || '', draggable: 'false', 'data-src': String(p.data.length) + p.mime }),
        h('div', { class: 'chart-bar' },
          h('button', { class: 'btn small', onClick: () => this.describe(p.id) }, 'Describe…'),
          h('button', { class: 'btn small', title: 'Original size', onClick: () => this.natural(p.id) }, 'Original size'),
          h('button', { class: 'btn small', onClick: () => this.d.copy?.(p.id) }, 'Copy'),
          h('button', { class: 'btn small plain', onClick: () => this.remove(p.id) }, 'Delete')),
        h('span', { class: 'grip', 'aria-hidden': 'true' }));
    }
  }

  select(id) {
    this.selected = id;
    for (const [k, n] of this.nodes) n.classList.toggle('on', k === id);
    if (id != null) this.nodes.get(id)?.focus({ preventScroll: true });
  }

  save(p) {
    this.d.run('set_picture', { picture: p });
    this.refresh();
  }

  remove(id) {
    this.d.run('remove_picture', { id });
    this.selected = null;
    this.refresh();
  }

  async describe(id) {
    const p = this.list.find((x) => x.id === id);
    if (!p) return;
    const alt = await ask('What the picture shows', 'For people who can’t see it (screen readers, and in other spreadsheet apps)', p.alt || '', 'Keep');
    if (alt != null) this.save({ ...p, alt: alt.trim() });
  }

  /// Back to the picture's own size (no wider than MAX_W).
  natural(id) {
    const p = this.list.find((x) => x.id === id);
    const img = this.nodes.get(id)?.querySelector('img');
    if (!p || !img?.naturalWidth) return;
    const k = Math.min(1, MAX_W / img.naturalWidth);
    this.save({ ...p, w: Math.round(img.naturalWidth * k), h: Math.round(img.naturalHeight * k) });
  }

  bind(n, id) {
    n.addEventListener('pointerdown', (e) => {
      if (e.target.closest('button')) return;
      e.stopPropagation();
      e.preventDefault();
      this.select(id);
      const p = this.list.find((x) => x.id === id);
      if (!p) return;
      const g = this.d.grid;
      const resizing = e.target.classList.contains('grip');
      const start = { x: e.clientX, y: e.clientY, left: n.offsetLeft, top: n.offsetTop };
      n.setPointerCapture(e.pointerId);
      let moved = false;
      const size = (ev) => {
        let w = Math.max(24, p.w + ev.clientX - start.x);
        let hh = Math.max(24, p.h + ev.clientY - start.y);
        // Shift: keeps its shape.
        if (ev.shiftKey) { const k = Math.max(w / p.w, hh / p.h); w = p.w * k; hh = p.h * k; }
        return [Math.round(w), Math.round(hh)];
      };
      const move = (ev) => {
        const dx = ev.clientX - start.x; const dy = ev.clientY - start.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
        if (resizing) { const [w, hh] = size(ev); n.style.width = `${w}px`; n.style.height = `${hh}px`; } else { n.style.left = `${start.left + dx}px`; n.style.top = `${start.top + dy}px`; }
      };
      const stop = () => {
        n.removeEventListener('pointermove', move);
        n.removeEventListener('pointerup', up);
        n.removeEventListener('pointercancel', cancel);
      };
      // Taken away mid-drag (the system took the pointer): back where it was.
      const cancel = () => { stop(); this.place(); };
      const up = (ev) => {
        stop();
        if (!moved) return;
        if (resizing) { const [w, hh] = size(ev); this.save({ ...p, w, h: hh }); return; }
        const left = start.left + ev.clientX - start.x; const top = start.top + ev.clientY - start.y;
        // The cell under its top left (the first one showing when it's let go
        // over the row numbers or column letters), and how far into it: never
        // above or left of that cell, so its top can't hide under the letters.
        const hit = g.hit(Math.max(47, left), Math.max(27, top));
        const r = hit.r || p.row; const c = hit.c || p.col;
        this.save({ ...p, row: r, col: c, dx: Math.max(0, Math.round(left - g.x(c))), dy: Math.max(0, Math.round(top - g.y(r))) });
      };
      n.addEventListener('pointermove', move);
      n.addEventListener('pointerup', up);
      n.addEventListener('pointercancel', cancel);
    });
  }

  // ----- Adding -----

  /// Insert › Picture…: picks picture files, each put at the cursor.
  choose() {
    const input = h('input', { type: 'file', accept: 'image/png,image/jpeg,image/gif,image/webp,image/svg+xml', multiple: true, style: { display: 'none' } });
    input.addEventListener('change', () => {
      const a = this.d.grid.active;
      [...input.files].forEach((f, k) => this.addFile(f, a.r, a.c + k * 3));
      input.remove();
    });
    document.body.append(input);
    input.click();
  }

  /// A picture file at cell (r, c), at its own size (no wider than NEW_W).
  async addFile(file, r, c) {
    if (!KINDS.test(file.type)) { toast('Calc takes PNG, JPEG, GIF, WebP and SVG pictures'); return null; }
    if (file.size > 15 * 1024 * 1024) { toast('That picture is too big (15 MB at most)'); return null; }
    const bytes = new Uint8Array(await file.arrayBuffer());
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    const data = btoa(bin);
    const [w, hh] = await new Promise((resolve) => {
      const img = new Image();
      img.onload = () => resolve([img.naturalWidth || 240, img.naturalHeight || 160]);
      img.onerror = () => resolve([240, 160]);
      img.src = `data:${file.type};base64,${data}`;
    });
    const k = Math.min(1, NEW_W / w);
    const id = this.d.run('add_picture', { picture: { sheet: this.d.sheet(), row: r, col: c, dx: 4, dy: 4, w: Math.round(w * k), h: Math.round(hh * k), mime: file.type, data, name: file.name || 'Picture', alt: '' } });
    if (id != null) { this.refresh(); this.select(id); }
    return id;
  }

  /// A copy of picture `p` with its top left at a cell of the sheet shown.
  paste(p, r, c) {
    const id = this.d.run('add_picture', { picture: { ...p, sheet: this.d.sheet(), row: r, col: c } });
    if (id != null) { this.refresh(); this.select(id); }
  }
}
