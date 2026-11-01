// The grid: drawn on a canvas over a scrolling area (which gives real
// scroll bars and touch scrolling), with a text box over the cell being
// typed in. Rows and columns are numbered from 1, as in formulas.
//
// Tables draw their header row in bold, their totals row under a line with
// a picker in each cell, and, just above the totals row, a line to type a
// new row into ("type to add a row") that isn't a row of the sheet.

const HDR_H = 26;   // column letters
const HDR_W = 46;   // row numbers
const MAX_ROW = 1048576;
const MAX_COL = 16384;

export function colName(c) {
  let s = '';
  while (c > 0) { const r = (c - 1) % 26; s = String.fromCharCode(65 + r) + s; c = Math.floor((c - 1) / 26); }
  return s;
}

export function cellName(r, c) {
  return `${colName(c)}${r}`;
}

/// "B12" → [12, 2]; "b$12" works too. Null when it isn't a cell.
export function parseCell(s) {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(s.trim());
  if (!m) return null;
  let c = 0;
  for (const ch of m[1].toUpperCase()) c = c * 26 + (ch.charCodeAt(0) - 64);
  const r = Number(m[2]);
  return r >= 1 && r <= MAX_ROW && c >= 1 && c <= MAX_COL ? [r, c] : null;
}

/// Where rows (or columns) are: a normal size, sizes that differ (0 when
/// hidden), and gaps after some of them (the "add a row" lines). Each
/// differing size and each gap moves the items after it.
class Axis {
  constructor() { this.def = 25; this.sizes = new Map(); this.gaps = new Map(); this.keys = []; this.runs = []; }

  set(def, sizes, gaps = new Map()) {
    this.def = def;
    this.sizes = sizes;
    this.gaps = gaps;
    const shift = new Map();
    for (const [i, s] of sizes) shift.set(i, (shift.get(i) || 0) + s - def);
    for (const [i, g] of gaps) shift.set(i, (shift.get(i) || 0) + g);
    this.keys = [...shift.keys()].sort((x, y) => x - y);
    let run = 0;
    this.runs = this.keys.map((k) => (run += shift.get(k)));
  }

  size(i) { return this.sizes.has(i) ? this.sizes.get(i) : this.def; }

  /// Where item `i` starts (the first item starts at 0).
  pos(i) {
    // The shifts of items before i.
    let lo = 0; let hi = this.keys.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (this.keys[mid] < i) lo = mid + 1; else hi = mid;
    }
    return (i - 1) * this.def + (lo > 0 ? this.runs[lo - 1] : 0);
  }

  /// The item at position `p` (`gap`: in the gap after it).
  at(p) {
    if (p <= 0) return { i: 1 };
    let lo = 1; let hi = 2100000;
    while (lo < hi) {
      const mid = Math.floor((lo + hi + 1) / 2);
      if (this.pos(mid) <= p) lo = mid; else hi = mid - 1;
    }
    // Hidden items (size 0) share a start with the next one: the visible one.
    if (this.gaps.has(lo) && p >= this.pos(lo) + this.size(lo)) return { i: lo, gap: true };
    return { i: lo };
  }
}


/// How table editor cell states look (the same in light and dark).
const STATE_LOOK = {
  new: { fill: 'rgba(237, 161, 0, 0.16)' },
  pending: { line: '#C2410C' },
  conflict: { fill: 'rgba(185, 28, 28, 0.10)', line: '#B91C1C' },
  sent: { fill: 'rgba(27, 175, 122, 0.14)' },
};
export class Grid {
  constructor(o) {
    Object.assign(this, o);
    this.ctx = this.canvas.getContext('2d');
    this.rows = new Axis();
    this.cols = new Axis();
    this.sheet = null;
    this.layout = null;
    this.active = { r: 1, c: 1 };
    this.anchor = { r: 1, c: 1 };
    this.cursor = { r: 1, c: 1 };
    this.cells = new Map();
    this.viewKey = '';
    this.editing = null;
    this.copied = null;
    this.buttons = [];
    this.limitR = 200;
    this.limitC = 30;
    this.scroller.tabIndex = 0;
    this.scroller.setAttribute('aria-label', 'Sheet cells');
    this.bind();
    new ResizeObserver(() => this.resize()).observe(this.host);
    this.resize();
  }

  // ----- What's on the sheet -----

  /// Shows sheet `id`. With `pointing`, a formula being typed stays open, so
  /// clicks on this sheet put references to it ('Sheet 2'!B3) into it.
  setSheet(id, pointing = false) {
    if (pointing && this.editing) {
      this.sheet = id;
      this.scroller.scrollTop = 0;
      this.scroller.scrollLeft = 0;
      this.point = null;
      this.refresh();
      this.placeEditor();
      this.editor.focus();
      return;
    }
    this.endEdit(false);
    this.sheet = id;
    this.active = { r: 1, c: 1 };
    this.anchor = { r: 1, c: 1 };
    this.cursor = { r: 1, c: 1 };
    this.scroller.scrollTop = 0;
    this.scroller.scrollLeft = 0;
    this.refresh();
    this.selected();
  }

  /// After any change: sizes, tables and the cells shown are read again.
  refresh() {
    if (this.sheet == null) return;
    const l = this.call('layout', { sheet: this.sheet });
    this.layout = l;
    const rs = new Map(l.rows.map(([r, h]) => [r, h]));
    const cs = new Map();
    for (const [a, b, w] of l.cols) for (let c = a; c <= Math.min(b, a + 2000); c++) cs.set(c, w);
    // The "type to add a row" line above each totals row.
    const gaps = new Map();
    this.ghosts = new Map();
    if (!['query', 'summary', 'linked_csv'].includes(l.kind)) {
      for (const t of l.tables) {
        if (t.totals) {
          const last = t.bottom - 1;
          gaps.set(last, l.row_height);
          this.ghosts.set(last, t);
        }
      }
    }
    this.rows.set(l.row_height, rs, gaps);
    this.cols.set(l.col_width, cs);
    this.frozen = { r: l.frozen[0], c: l.frozen[1] };
    this.merged = (l.merged || []).map(([r, c, w, h]) => ({ r, c, w, h }));
    this.viewKey = '';
    this.grow();
    this.draw();
  }

  /// The scroll area covers what's in use and a screen or two more.
  grow() {
    const used = this.layout?.used || [1, 1];
    const view = this.visible();
    this.limitR = Math.min(MAX_ROW, Math.max(used[0] + 60, 200, view.r2 + 120));
    this.limitC = Math.min(MAX_COL, Math.max(used[1] + 12, 30, view.c2 + 12));
    this.spacer.style.width = `${HDR_W + this.cols.pos(this.limitC + 1)}px`;
    this.spacer.style.height = `${HDR_H + this.rows.pos(this.limitR + 1)}px`;
  }

  resize() {
    const r = this.host.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    // The scroll bars aren't drawn over.
    const w = this.scroller.clientWidth || r.width;
    const hgt = this.scroller.clientHeight || r.height;
    // The same size: nothing to do (setting a canvas's size clears it).
    if (w === this.w && hgt === this.h && dpr === this.dpr) return;
    this.w = w;
    this.h = hgt;
    this.canvas.width = Math.round(this.w * dpr);
    this.canvas.height = Math.round(this.h * dpr);
    this.canvas.style.width = `${this.w}px`;
    this.canvas.style.height = `${this.h}px`;
    this.dpr = dpr;
    this.viewKey = '';
    if (this.layout) { this.grow(); this.draw(); }
  }

  // ----- Positions -----

  frozenH() { return this.frozen?.r ? this.rows.pos(this.frozen.r + 1) : 0; }
  frozenW() { return this.frozen?.c ? this.cols.pos(this.frozen.c + 1) : 0; }

  /// Where row `r` starts on screen.
  y(r) {
    const fr = this.frozen?.r || 0;
    return HDR_H + (r <= fr ? this.rows.pos(r) : this.rows.pos(r) - this.scroller.scrollTop);
  }

  x(c) {
    const fc = this.frozen?.c || 0;
    return HDR_W + (c <= fc ? this.cols.pos(c) : this.cols.pos(c) - this.scroller.scrollLeft);
  }

  /// The rows and columns on screen (outside the frozen ones).
  visible() {
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    const top = this.scroller.scrollTop + this.frozenH();
    const left = this.scroller.scrollLeft + this.frozenW();
    const r1 = Math.max(fr + 1, this.rows.at(top).i);
    const c1 = Math.max(fc + 1, this.cols.at(left).i);
    const r2 = Math.min(MAX_ROW, this.rows.at(top + (this.h || 800) - HDR_H).i + 1);
    const c2 = Math.min(MAX_COL, this.cols.at(left + (this.w || 1200) - HDR_W).i + 1);
    return { r1, c1, r2, c2 };
  }

  /// What's under the pointer at (`px`, `py`) in the sheet.
  hit(px, py) {
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    const sy = py - HDR_H < this.frozenH() ? 0 : this.scroller.scrollTop;
    const sx = px - HDR_W < this.frozenW() ? 0 : this.scroller.scrollLeft;
    const rAt = this.rows.at(py - HDR_H + sy);
    const cAt = this.cols.at(px - HDR_W + sx);
    let r = rAt.i; let c = cAt.i;
    if (fr && py - HDR_H >= this.frozenH() && r <= fr) r = fr + 1;
    if (fc && px - HDR_W >= this.frozenW() && c <= fc) c = fc + 1;
    r = Math.min(Math.max(1, r), MAX_ROW);
    c = Math.min(Math.max(1, c), MAX_COL);
    if (py < HDR_H && px < HDR_W) return { kind: 'corner' };
    if (py < HDR_H) {
      // Near a column's right edge: resizing it.
      const right = this.x(c) + this.cols.size(c);
      const left = this.x(c);
      if (Math.abs(px - right) <= 4) return { kind: 'colborder', c };
      if (Math.abs(px - left) <= 4 && c > 1) return { kind: 'colborder', c: c - 1 };
      return { kind: 'colhead', c };
    }
    if (px < HDR_W) {
      const bottom = this.y(r) + this.rows.size(r);
      if (!rAt.gap && Math.abs(py - bottom) <= 3) return { kind: 'rowborder', r };
      if (rAt.gap) return { kind: 'rowhead', r: r + 1 };
      return { kind: 'rowhead', r };
    }
    if (rAt.gap) return { kind: 'ghost', r, c, table: this.ghosts.get(r) };
    const t = this.tableAt(r, c);
    if (t) {
      const k = c - t.left;
      if (t.totals && r === t.bottom && px - this.x(c) < 18) return { kind: 'totals', r, c, table: t, column: t.columns[k].name };
      if (t.header && r === t.top && this.x(c) + this.cols.size(c) - px < 22) return { kind: 'filter', r, c, table: t, column: t.columns[k].name };
    }
    return { kind: 'cell', r, c };
  }

  /// The merged cells (r, c) is part of, or null.
  mergeAt(r, c) {
    return this.merged?.find((m) => r >= m.r && r < m.r + m.h && c >= m.c && c < m.c + m.w) || null;
  }

  tableAt(r, c) {
    return this.layout?.tables.find((t) => r >= t.top && r <= t.bottom && c >= t.left && c <= t.right) || null;
  }

  locked() {
    return ['query', 'summary', 'linked_csv'].includes(this.layout?.kind);
  }

  // ----- Selection -----

  range() {
    const a = this.anchor; const b = this.cursor;
    const rg = { top: Math.min(a.r, b.r), left: Math.min(a.c, b.c), bottom: Math.max(a.r, b.r), right: Math.max(a.c, b.c) };
    // A selection takes in all of every merged cell it touches.
    for (let grew = true; grew && this.merged?.length;) {
      grew = false;
      for (const m of this.merged) {
        const touches = m.r <= rg.bottom && m.r + m.h - 1 >= rg.top && m.c <= rg.right && m.c + m.w - 1 >= rg.left;
        if (!touches) continue;
        const nt = Math.min(rg.top, m.r); const nl = Math.min(rg.left, m.c);
        const nb = Math.max(rg.bottom, m.r + m.h - 1); const nr = Math.max(rg.right, m.c + m.w - 1);
        if (nt !== rg.top || nl !== rg.left || nb !== rg.bottom || nr !== rg.right) {
          Object.assign(rg, { top: nt, left: nl, bottom: nb, right: nr });
          grew = true;
        }
      }
    }
    return rg;
  }

  select(r, c, extend = false) {
    r = Math.min(Math.max(1, r), MAX_ROW);
    c = Math.min(Math.max(1, c), MAX_COL);
    // In a merged cell: its top left.
    const m = !extend && this.mergeAt(r, c);
    if (m) { r = m.r; c = m.c; }
    if (extend) {
      this.cursor = { r, c };
    } else {
      this.active = { r, c };
      this.anchor = { r, c };
      this.cursor = { r, c };
    }
    this.reveal(extend ? this.cursor : this.active);
    this.selected();
    this.draw();
  }

  selectRange(rg) {
    this.active = { r: rg.top, c: rg.left };
    this.anchor = { r: rg.top, c: rg.left };
    this.cursor = { r: rg.bottom, c: rg.right };
    this.selected();
    this.draw();
  }

  selected() {
    this.onSelect?.(this.active, this.range());
  }

  /// Scrolls so cell `p` shows.
  reveal(p) {
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    const s = this.scroller;
    if (p.r > fr) {
      const top = this.rows.pos(p.r) - this.frozenH();
      const bottom = this.rows.pos(p.r) + this.rows.size(p.r) - (this.h - HDR_H);
      if (top < s.scrollTop) s.scrollTop = top;
      else if (bottom > s.scrollTop) s.scrollTop = bottom + 2;
    }
    if (p.c > fc) {
      const left = this.cols.pos(p.c) - this.frozenW();
      const right = this.cols.pos(p.c) + this.cols.size(p.c) - (this.w - HDR_W);
      if (left < s.scrollLeft) s.scrollLeft = left;
      else if (right > s.scrollLeft) s.scrollLeft = right + 2;
    }
    if (p.r > this.limitR - 20 || p.c > this.limitC - 5) this.grow();
  }

  // ----- Drawing -----

  /// The cells shown, read when the view moves or the sheet changes.
  load() {
    const v = this.visible();
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    const key = `${this.sheet}:${v.r1}:${v.c1}:${v.r2}:${v.c2}:${fr}:${fc}`;
    if (key === this.viewKey) return;
    this.viewKey = key;
    this.cells = new Map();
    this.widths = null;
    const parts = [[v.r1, v.c1, v.r2, v.c2]];
    if (fr) parts.push([1, v.c1, fr, v.c2]);
    if (fc) parts.push([v.r1, 1, v.r2, fc]);
    if (fr && fc) parts.push([1, 1, fr, fc]);
    for (const [top, left, bottom, right] of parts) {
      // Text that runs on into empty cells: read a little to the left too.
      const got = this.call('view', { sheet: this.sheet, top, left: Math.max(1, left - 4), bottom, right });
      for (const cell of got.cells) this.cells.set(`${cell.r},${cell.c}`, cell);
    }
  }

  /// Marks the cells shown as out of date and draws again.
  changed() {
    this.viewKey = '';
    this.draw();
  }

  draw() {
    if (this.queued) return;
    this.queued = requestAnimationFrame(() => { this.queued = null; this.paint(); });
  }

  css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  paint() {
    if (!this.layout) return;
    this.load();
    const ctx = this.ctx;
    const dpr = this.dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const C = {
      sheet: this.css('--sheet'), soft: this.css('--soft'), line: this.css('--line'), line2: this.css('--line-2'),
      ground: this.css('--ground'), ink: this.css('--ink'), ink2: this.css('--ink-2'), muted: this.css('--muted'),
      faint: this.css('--faint'), accent: this.css('--accent'), accentInk: this.css('--accent-ink'),
      accentSoft: this.css('--accent-soft'), sel: this.css('--sel'), dark: document.documentElement.dataset.theme === 'dark' || (matchMedia('(prefers-color-scheme: dark)').matches && document.documentElement.dataset.theme !== 'light'),
    };
    this.colours = C;
    ctx.fillStyle = C.sheet;
    ctx.fillRect(0, 0, this.w, this.h);
    const v = this.visible();
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    const fh = this.frozenH(); const fw = this.frozenW();
    // Four parts: frozen corner, frozen rows, frozen columns, the rest.
    const parts = [
      { r1: v.r1, r2: v.r2, c1: v.c1, c2: v.c2, clip: [HDR_W + fw, HDR_H + fh, this.w, this.h] },
    ];
    if (fr) parts.push({ r1: 1, r2: fr, c1: v.c1, c2: v.c2, clip: [HDR_W + fw, HDR_H, this.w, HDR_H + fh] });
    if (fc) parts.push({ r1: v.r1, r2: v.r2, c1: 1, c2: fc, clip: [HDR_W, HDR_H + fh, HDR_W + fw, this.h] });
    if (fr && fc) parts.push({ r1: 1, r2: fr, c1: 1, c2: fc, clip: [HDR_W, HDR_H, HDR_W + fw, HDR_H + fh] });
    this.cut = new Set();
    for (const p of parts) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(p.clip[0], p.clip[1], p.clip[2] - p.clip[0], p.clip[3] - p.clip[1]);
      ctx.clip();
      this.paintCells(p, C);
      this.paintSelection(p, C);
      ctx.restore();
    }
    this.paintHeaders(v, C);
    if (fr || fc) {
      ctx.fillStyle = C.line2;
      if (fr) ctx.fillRect(0, HDR_H + fh - 1, this.w, 1.5);
      if (fc) ctx.fillRect(HDR_W + fw - 1, 0, 1.5, this.h);
    }
    this.placeEditor();
    this.onPaint?.();
  }

  rowList(r1, r2) {
    const out = [];
    for (let r = r1; r <= r2; r++) {
      if (this.rows.size(r) > 0) out.push(r);
    }
    return out;
  }

  fontFor(look) {
    const size = (look.size || 12) * 1.125;
    const family = look.font && !/^(Inter|Calibri|Arial|Aptos)$/i.test(look.font) ? `"${look.font}", Figtree, sans-serif` : 'Figtree, system-ui, sans-serif';
    return `${look.i ? 'italic ' : ''}${look.b ? 700 : 400} ${size}px ${family}`;
  }

  paintCells(p, C) {
    const ctx = this.ctx;
    const grid = this.layout.grid;
    const rows = this.rowList(p.r1, p.r2);
    const cols = [];
    for (let c = p.c1; c <= p.c2; c++) if (this.cols.size(c) > 0) cols.push(c);
    // Gridlines.
    if (grid) {
      ctx.fillStyle = C.dark ? C.line : '#EFECE8';
      for (const r of rows) ctx.fillRect(HDR_W, this.y(r) + this.rows.size(r) - 1, this.w, 1);
      for (const c of cols) ctx.fillRect(this.x(c) + this.cols.size(c) - 1, HDR_H, 1, this.h);
    }
    // Tables: header and totals rows, stripes, the add-a-row line.
    for (const t of this.layout.tables) {
      if (t.bottom < p.r1 || t.top > p.r2 || t.right < p.c1 || t.left > p.c2) continue;
      const x1 = this.x(t.left); const x2 = this.x(t.right) + this.cols.size(t.right);
      if (t.header) {
        ctx.fillStyle = C.dark ? 'rgba(255,255,255,.07)' : '#EFEBE5';
        ctx.fillRect(x1, this.y(t.top), x2 - x1, this.rows.size(t.top) - 1);
      }
      if (t.stripes) {
        ctx.fillStyle = C.dark ? 'rgba(255,255,255,.035)' : 'rgba(28,25,23,.035)';
        for (let r = t.top + (t.header ? 2 : 1); r <= t.bottom - (t.totals ? 1 : 0); r += 2) {
          if (r >= p.r1 && r <= p.r2) ctx.fillRect(x1, this.y(r), x2 - x1, this.rows.size(r) - 1);
        }
      }
      const ghost = this.ghosts.get(t.bottom - 1) === t ? t.bottom - 1 : null;
      if (ghost != null && ghost >= p.r1 - 1 && ghost <= p.r2) {
        const gy = this.y(ghost) + this.rows.size(ghost);
        ctx.fillStyle = C.dark ? C.soft : '#FDFCFB';
        ctx.fillRect(x1, gy, x2 - x1, this.rows.def - 1);
        ctx.font = 'italic 400 13px Figtree, system-ui, sans-serif';
        ctx.fillStyle = '#A8A29E';
        ctx.textBaseline = 'middle';
        ctx.fillText('type to add a row', x1 + 8, gy + this.rows.def / 2);
        ctx.fillStyle = C.dark ? C.line : '#EFECE8';
        ctx.fillRect(x1, gy + this.rows.def - 1, x2 - x1, 1);
      }
      if (t.totals && t.bottom >= p.r1 && t.bottom <= p.r2) {
        ctx.fillStyle = C.ink;
        ctx.fillRect(x1, this.y(t.bottom) - 1, x2 - x1, 1.5);
      }
      // An outline, so a table reads as one thing (the add-a-row line is inside it).
      const y1 = this.y(t.top); const y2 = this.y(t.bottom) + this.rows.size(t.bottom);
      ctx.strokeStyle = C.dark ? 'rgba(255,255,255,.22)' : '#CFC8BF';
      ctx.lineWidth = 1;
      ctx.strokeRect(x1 - 0.5, y1 - 0.5, x2 - x1, y2 - y1);
    }
    // Merged cells: one cell (the gridlines inside them go).
    const merges = (this.merged || []).filter((m) => m.r <= p.r2 && m.r + m.h - 1 >= p.r1 && m.c <= p.c2 && m.c + m.w - 1 >= p.c1);
    for (const m of merges) {
      const box = this.mergeBox(m);
      const anchor = this.cells.get(`${m.r},${m.c}`);
      ctx.fillStyle = anchor?.look.fill || C.sheet;
      ctx.fillRect(box.x, box.y, box.w - 1, box.h - 1);
    }
    // Fills first, then text, then borders (so text running on isn't covered).
    const cellsShown = [];
    for (const r of rows) {
      for (const c of cols) {
        const cell = this.cells.get(`${r},${c}`);
        if (!cell) continue;
        cellsShown.push(cell);
        if (cell.look.fill) {
          ctx.fillStyle = cell.look.fill;
          ctx.fillRect(this.x(c), this.y(r), this.cols.size(c) - 1, this.rows.size(r) - 1);
        }
      }
    }
    // Data bars, under the text.
    for (const cell of cellsShown) if (cell.bar) this.paintBar(cell);
    ctx.textBaseline = 'middle';
    for (const r of rows) {
      const t0 = this.layout.tables.find((t) => r === t.top && t.header) || null;
      const tTot = this.layout.tables.find((t) => r === t.bottom && t.totals) || null;
      for (const c of cols) {
        const cell = this.cells.get(`${r},${c}`);
        const inTable = (t0 && c >= t0.left && c <= t0.right) ? t0 : (tTot && c >= tTot.left && c <= tTot.right ? tTot : null);
        if (inTable && tTot === inTable) this.paintTotalsMark(r, c, C);
        if (!cell || cell.text === '') continue;
        const m = merges.find((mm) => r >= mm.r && r < mm.r + mm.h && c >= mm.c && c < mm.c + mm.w);
        if (m && (m.r !== r || m.c !== c)) continue;
        this.paintText(cell, r, c, C, !!inTable, tTot === inTable && !!inTable, m ? this.mergeBox(m) : null);
      }
      if (t0) for (let c = Math.max(t0.left, p.c1); c <= Math.min(t0.right, p.c2); c++) this.paintFilterMark(t0, r, c, C);
    }
    for (const cell of cellsShown) if (cell.look.border) this.paintBorders(cell, C);
    // Sparklines, inside their cells.
    for (const cell of cellsShown) if (cell.spark) this.paintSpark(cell, C);
    // Comments: a mark in the top right corner (shown on hover).
    for (const cell of cellsShown) {
      if (!cell.comment) continue;
      const x = this.x(cell.c) + this.cols.size(cell.c) - 1; const y = this.y(cell.r);
      ctx.fillStyle = '#C2410C';
      ctx.beginPath();
      ctx.moveTo(x - 7, y); ctx.lineTo(x, y); ctx.lineTo(x, y + 7); ctx.closePath();
      ctx.fill();
    }
    // A list of choices for the active cell: its button.
    const a = this.active;
    const ch = this.choicesAt(a.r, a.c);
    if (ch && a.r >= p.r1 && a.r <= p.r2 && a.c >= p.c1 && a.c <= p.c2) {
      const bx = this.x(a.c) + this.cols.size(a.c) + 1; const by = this.y(a.r);
      ctx.fillStyle = C.sheet;
      ctx.strokeStyle = C.line2;
      ctx.lineWidth = 1;
      roundRect(ctx, bx, by + 2, 18, this.rows.size(a.r) - 4, 4);
      ctx.fill();
      ctx.stroke();
      ctx.strokeStyle = C.muted;
      ctx.lineWidth = 1.4;
      const cy = by + this.rows.size(a.r) / 2;
      ctx.beginPath(); ctx.moveTo(bx + 5, cy - 2); ctx.lineTo(bx + 9, cy + 2); ctx.lineTo(bx + 13, cy - 2); ctx.stroke();
    }
  }

  /// The list of choices (from the Excel file) for cell (r, c), or null.
  choicesAt(r, c) {
    return this.layout?.choices?.find((ch) => r >= ch.top && r <= ch.bottom && c >= ch.left && c <= ch.right) || null;
  }

  /// A sparkline: a small line, bars, or ups and downs across the cell.
  paintSpark(cell, C) {
    const ctx = this.ctx;
    const vals = cell.spark.values;
    const nums = vals.filter((v) => v != null);
    if (!nums.length) return;
    const x = this.x(cell.c) + 4; const y = this.y(cell.r) + 4;
    const w = this.cols.size(cell.c) - 9; const hgt = this.rows.size(cell.r) - 9;
    const colour = getComputedStyle(document.documentElement).getPropertyValue('--series-1').trim() || '#C2410C';
    const lo = Math.min(0, ...nums); const hi = Math.max(0, ...nums);
    const kind = cell.spark.kind;
    const step = w / Math.max(1, vals.length - (kind === 'line' ? 1 : 0));
    ctx.save();
    if (kind === 'line') {
      const min = Math.min(...nums); const max = Math.max(...nums);
      const yv = (v) => y + hgt - ((v - min) / ((max - min) || 1)) * hgt;
      ctx.strokeStyle = colour;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = 'round';
      ctx.beginPath();
      let started = false;
      vals.forEach((v, k) => {
        if (v == null) { started = false; return; }
        const px = x + k * step;
        if (started) ctx.lineTo(px, yv(v)); else ctx.moveTo(px, yv(v));
        started = true;
      });
      ctx.stroke();
      // The last point marked.
      const last = vals.length - 1 - [...vals].reverse().findIndex((v) => v != null);
      ctx.fillStyle = colour;
      ctx.beginPath(); ctx.arc(x + last * step, yv(vals[last]), 2.2, 0, Math.PI * 2); ctx.fill();
    } else {
      const zero = kind === 'win_loss' ? y + hgt / 2 : y + hgt - ((0 - lo) / ((hi - lo) || 1)) * hgt;
      vals.forEach((v, k) => {
        if (v == null) return;
        const bw = Math.max(1, step - 1.5);
        let top; let len;
        if (kind === 'win_loss') { len = hgt / 2 - 1; top = v >= 0 ? zero - len : zero; if (v === 0) return; }
        else { len = Math.abs(v) / ((hi - lo) || 1) * hgt; top = v >= 0 ? zero - len : zero; }
        ctx.fillStyle = v < 0 ? '#B91C1C' : colour;
        ctx.fillRect(x + k * step, top, bw, Math.max(1, len));
      });
    }
    ctx.restore();
  }

  /// A data bar: part of the cell filled, from its zero line.
  paintBar(cell) {
    const ctx = this.ctx;
    const b = cell.bar;
    const x = this.x(cell.c) + 2; const y = this.y(cell.r) + 3;
    const w = this.cols.size(cell.c) - 5; const hgt = this.rows.size(cell.r) - 7;
    const axis = x + w * Math.max(0, Math.min(1, b.axis || 0));
    const negative = cell.text.startsWith('-');
    const len = w * Math.max(0, Math.min(1, b.value)) * (b.axis > 0 ? (negative ? b.axis : 1 - b.axis) : 1);
    const left = negative && b.axis > 0 ? axis - len : (b.axis > 0 ? axis : x);
    const colour = negative ? (b.negative || '#FF0000') : (b.color || '#638EC6');
    if (b.gradient) {
      const g = ctx.createLinearGradient(left, 0, left + len, 0);
      g.addColorStop(negative ? 1 : 0, colour);
      g.addColorStop(negative ? 0 : 1, 'rgba(255,255,255,0.15)');
      ctx.fillStyle = g;
    } else {
      ctx.fillStyle = colour;
      ctx.globalAlpha = 0.55;
    }
    ctx.fillRect(left, y, Math.max(1, len), hgt);
    ctx.globalAlpha = 1;
    if (b.axis > 0) {
      ctx.fillStyle = 'rgba(0,0,0,.35)';
      for (let yy = y; yy < y + hgt; yy += 3) ctx.fillRect(axis, yy, 1, 1.5);
    }
  }

  /// A conditional formatting icon (arrow, light, flag, star…) at (x, y), `s` across.
  paintIcon(name, colour, x, y, s) {
    const ctx = this.ctx;
    ctx.save();
    ctx.fillStyle = colour || '#808080';
    ctx.strokeStyle = colour || '#808080';
    ctx.lineWidth = 1.6;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    const cx = x + s / 2; const cy = y + s / 2; const r = s / 2 - 1;
    const path = (pts, close = true) => { ctx.beginPath(); pts.forEach(([px, py], i) => (i ? ctx.lineTo(x + px * s, y + py * s) : ctx.moveTo(x + px * s, y + py * s))); if (close) ctx.closePath(); };
    switch (name) {
      case 'ArrowUp': path([[0.5, 0.08], [0.92, 0.55], [0.66, 0.55], [0.66, 0.92], [0.34, 0.92], [0.34, 0.55], [0.08, 0.55]]); ctx.fill(); break;
      case 'ArrowDown': path([[0.5, 0.92], [0.92, 0.45], [0.66, 0.45], [0.66, 0.08], [0.34, 0.08], [0.34, 0.45], [0.08, 0.45]]); ctx.fill(); break;
      case 'ArrowRight': path([[0.92, 0.5], [0.45, 0.08], [0.45, 0.34], [0.08, 0.34], [0.08, 0.66], [0.45, 0.66], [0.45, 0.92]]); ctx.fill(); break;
      case 'ArrowAngleUp': path([[0.2, 0.8], [0.8, 0.2]], false); ctx.stroke(); path([[0.42, 0.2], [0.8, 0.2], [0.8, 0.58]], false); ctx.stroke(); break;
      case 'ArrowAngleDown': path([[0.2, 0.2], [0.8, 0.8]], false); ctx.stroke(); path([[0.42, 0.8], [0.8, 0.8], [0.8, 0.42]], false); ctx.stroke(); break;
      case 'TriangleUp': case 'TriangleUpFilled': path([[0.5, 0.15], [0.9, 0.82], [0.1, 0.82]]); ctx.fill(); break;
      case 'TriangleDown': case 'TriangleDownFilled': path([[0.5, 0.85], [0.9, 0.18], [0.1, 0.18]]); ctx.fill(); break;
      case 'FlatRectangle': ctx.fillRect(x + 0.1 * s, y + 0.38 * s, 0.8 * s, 0.24 * s); break;
      case 'Rhombus': path([[0.5, 0.08], [0.92, 0.5], [0.5, 0.92], [0.08, 0.5]]); ctx.fill(); break;
      case 'Flag': path([[0.25, 0.92], [0.25, 0.1]], false); ctx.stroke(); path([[0.27, 0.12], [0.85, 0.3], [0.27, 0.5]]); ctx.fill(); break;
      case 'Check': ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.fill(); ctx.strokeStyle = '#fff'; path([[0.28, 0.52], [0.44, 0.68], [0.74, 0.34]], false); ctx.stroke(); break;
      case 'Cross': ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.fill(); ctx.strokeStyle = '#fff'; path([[0.32, 0.32], [0.68, 0.68]], false); ctx.stroke(); path([[0.68, 0.32], [0.32, 0.68]], false); ctx.stroke(); break;
      case 'Exclamation': path([[0.5, 0.06], [0.95, 0.9], [0.05, 0.9]]); ctx.fill(); ctx.fillStyle = '#fff'; ctx.fillRect(cx - 0.8, y + 0.35 * s, 1.6, 0.3 * s); ctx.fillRect(cx - 0.8, y + 0.72 * s, 1.6, 1.6); break;
      case 'Star': {
        ctx.beginPath();
        for (let k = 0; k < 10; k++) { const ang = -Math.PI / 2 + (k * Math.PI) / 5; const rr = k % 2 ? r * 0.45 : r; const px = cx + rr * Math.cos(ang); const py = cy + rr * Math.sin(ang); if (k) ctx.lineTo(px, py); else ctx.moveTo(px, py); }
        ctx.closePath(); ctx.fill(); break;
      }
      case 'Heart': ctx.beginPath(); ctx.moveTo(cx, y + 0.85 * s); ctx.bezierCurveTo(x - 0.1 * s, y + 0.45 * s, x + 0.25 * s, y + 0.05 * s, cx, y + 0.3 * s); ctx.bezierCurveTo(x + 0.75 * s, y + 0.05 * s, x + 1.1 * s, y + 0.45 * s, cx, y + 0.85 * s); ctx.fill(); break;
      case 'ThumbsUp': path([[0.5, 0.15], [0.9, 0.75], [0.1, 0.75]]); ctx.fill(); break;
      case 'ThumbsDown': path([[0.5, 0.85], [0.9, 0.25], [0.1, 0.25]]); ctx.fill(); break;
      default: ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.fill();
    }
    ctx.restore();
  }

  /// Where a merged cell is on screen.
  mergeBox(m) {
    const x = this.x(m.c); const y = this.y(m.r);
    return { x, y, w: this.x(m.c + m.w - 1) + this.cols.size(m.c + m.w - 1) - x, h: this.y(m.r + m.h - 1) + this.rows.size(m.r + m.h - 1) - y };
  }

  paintText(cell, r, c, C, bold, totals = false, box = null) {
    const ctx = this.ctx;
    const look = cell.look;
    const x = box ? box.x : this.x(c); const y = box ? box.y : this.y(r);
    const w = box ? box.w : this.cols.size(c); const hgt = box ? box.h : this.rows.size(r);
    ctx.font = this.fontFor(bold && !look.b ? { ...look, b: true } : look);
    const align = look.align || (cell.number ? 'right' : (/^(TRUE|FALSE|#[A-Z/0!?]+)$/.test(cell.text) && !cell.formula ? 'center' : 'left'));
    let text = cell.text;
    let width = Math.max(...text.split('\n').map((l) => ctx.measureText(l).width));
    const pad = 7;
    // Numbers that don't fit show ### (as everywhere); words run on into empty cells to the right.
    let room = w - pad * 2;
    if (width > room && cell.number) {
      // A plain number shrinks to its short form (1.23457E+19), as Excel does; others show ###.
      let short = null;
      if (/^-?\d+(\.\d+)?$/.test(text)) {
        const n = Number(text);
        for (let k = 5; k >= 0 && short == null; k--) {
          const t = n.toExponential(k).replace(/\.?0+e/, 'e').replace('e', 'E').replace(/E\+?(-?)(\d)$/, 'E+$10$2').replace('E+-', 'E-');
          if (ctx.measureText(t).width <= room) short = t;
        }
      }
      text = short || '#'.repeat(Math.max(1, Math.floor(room / ctx.measureText('#').width)));
      width = ctx.measureText(text).width;
    }
    let clipW = w;
    if (width > room && align === 'left' && !look.wrap && !box) {
      let k = c + 1;
      while (clipW - pad * 2 < width && k <= c + 30) {
        const next = this.cells.get(`${r},${k}`);
        if (next && next.text !== '') break;
        clipW += this.cols.size(k);
        k++;
      }
    }
    ctx.save();
    ctx.beginPath();
    ctx.rect(x, y, clipW - 1, hgt - 1);
    ctx.clip();
    ctx.fillStyle = look.color || (this.colours.dark ? C.ink : '#1C1917');
    const ty = look.valign === 'top' ? y + 11 : look.valign === 'center' ? y + hgt / 2 : y + hgt - 12;
    // Totals cells keep room for their picker on the left; icons take the left too.
    let lead = totals ? 10 : 0;
    const s = Math.min(14, hgt - 6);
    if (cell.icon) { this.paintIcon(cell.icon.name, cell.icon.color, x + 4, y + (hgt - s) / 2, s); lead = s + 4; }
    if (cell.rating) {
      for (let k = 0; k < cell.rating.max; k++) {
        this.ctx.globalAlpha = k < cell.rating.count ? 1 : 0.22;
        this.paintIcon(cell.rating.name, cell.rating.color, x + 4 + k * (s + 1), y + (hgt - s) / 2, s);
      }
      this.ctx.globalAlpha = 1;
      lead = cell.rating.max * (s + 1) + 4;
    }
    const hide = (cell.bar && cell.bar.show === false) || (cell.icon && cell.icon.show === false) || (cell.rating && cell.rating.show === false);
    if (hide) { ctx.restore(); return; }
    let tx = x + pad + lead;
    if (align === 'right') tx = x + w - pad - width;
    else if (align === 'center') tx = x + (w - width) / 2;
    let cut = false;
    if (look.wrap || text.includes('\n')) {
      cut = !this.paintWrapped(text, x + pad, y + 4, (look.wrap ? w : clipW) - pad * 2, hgt - 4, align, !!look.wrap);
    } else {
      cut = !cell.number && width > clipW - pad * 2 + 1;
      ctx.fillText(text, tx, ty);
      // Spelling and grammar: a wavy line under each word with something to say.
      for (const m of this.issues?.get(`${r},${c}`) || []) {
        const x0 = tx + ctx.measureText(text.slice(0, m.a)).width;
        const x1 = tx + ctx.measureText(text.slice(0, m.b)).width;
        this.paintWave(x0, x1, ty + 7, m.spelling ? '#DC2626' : '#2A78D6');
      }
      if (look.u || look.s) {
        const lw = Math.max(1, (look.size || 12) / 12);
        ctx.fillRect(tx, look.u ? ty + 7 : ty, width, lw);
      }
    }
    if (cut) {
      this.cut.add(`${r},${c}`);
      // Where the text stops showing: its own cell (or the last one it runs into).
      this.paintCut(x, y, look.wrap || align !== 'left' ? w : clipW, hgt, look.fill || C.sheet);
    }
    ctx.restore();
  }

  /// Lines of text in a cell: its own line breaks (Alt+Enter), and with
  /// `wrap` words wrapped to the width. Whether they all fit.
  paintWrapped(text, x, y, w, hgt, align, wrap = true) {
    const ctx = this.ctx;
    const lines = [];
    let wide = false;
    for (const para of text.split('\n')) {
      if (!wrap) { lines.push(para); if (ctx.measureText(para).width > w) wide = true; continue; }
      let line = '';
      for (const word of para.split(/(\s+)/)) {
        const test = line + word;
        if (ctx.measureText(test).width > w && line.trim()) { lines.push(line.trim()); line = word.trimStart(); } else line = test;
      }
      lines.push(line.trim());
    }
    const lh = 17;
    lines.forEach((l, i) => {
      const lw = ctx.measureText(l).width;
      const lx = align === 'right' ? x + w - lw : align === 'center' ? x + (w - lw) / 2 : x;
      if ((i + 1) * lh <= hgt + 4) ctx.fillText(l, lx, y + i * lh + 9);
    });
    return !wide && lines.length * lh <= hgt + 4;
  }

  paintWave(x0, x1, y, colour) {
    const ctx = this.ctx;
    ctx.save();
    ctx.strokeStyle = colour;
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    for (let x = x0, k = 0; x <= x1; x += 2, k++) ctx[k ? 'lineTo' : 'moveTo'](x, y + (k % 2 ? 1.5 : 0));
    ctx.stroke();
    ctx.restore();
  }

  /// A cell whose text doesn't all show: a fade and "…" at its right edge
  /// (the pointer over it shows the whole text).
  paintCut(x, y, w, hgt, fill) {
    const ctx = this.ctx;
    // The fade: the cell's own colour, from see-through to solid.
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(fill);
    const clear = m ? `rgba(${parseInt(m[1], 16)},${parseInt(m[2], 16)},${parseInt(m[3], 16)},0)` : 'transparent';
    const g = ctx.createLinearGradient(x + w - 26, 0, x + w - 1, 0);
    g.addColorStop(0, clear);
    g.addColorStop(0.55, fill);
    ctx.save();
    ctx.fillStyle = g;
    ctx.fillRect(x + w - 26, y, 25, hgt - 1);
    ctx.fillStyle = '#A8A29E';
    ctx.font = '700 12px Figtree, system-ui, sans-serif';
    ctx.textAlign = 'right';
    ctx.fillText('…', x + w - 4, y + hgt / 2);
    ctx.restore();
  }

  paintBorders(cell, C) {
    const ctx = this.ctx;
    const [t, r, b, l] = cell.look.border;
    const x = this.x(cell.c); const y = this.y(cell.r);
    const w = this.cols.size(cell.c); const hgt = this.rows.size(cell.r);
    const line = (side, x0, y0, x1, y1) => {
      if (!side) return;
      ctx.fillStyle = side[1];
      ctx.fillRect(x0, y0, Math.max(x1 - x0, side[0]), Math.max(y1 - y0, side[0]));
    };
    line(t, x - 1, y - 1, x + w, y - 1);
    line(b, x - 1, y + hgt - 1, x + w, y + hgt - 1);
    line(l, x - 1, y - 1, x - 1, y + hgt);
    line(r, x + w - 1, y - 1, x + w - 1, y + hgt);
  }

  paintTotalsMark(r, c, C) {
    const ctx = this.ctx;
    ctx.fillStyle = '#A8A29E';
    ctx.font = '500 11px Figtree, sans-serif';
    ctx.textBaseline = 'middle';
    ctx.fillText('▾', this.x(c) + 4, this.y(r) + this.rows.size(r) / 2);
  }

  paintFilterMark(t, r, c, C) {
    const ctx = this.ctx;
    const name = t.columns[c - t.left]?.name;
    const on = (this.layout.filters[t.name] || []).some((f) => f.column.toLowerCase() === name?.toLowerCase());
    const x = this.x(c) + this.cols.size(c) - 20; const y = this.y(r) + (this.rows.size(r) - 16) / 2;
    ctx.fillStyle = on ? C.accentSoft : (C.dark ? C.soft : '#F2EFEB');
    roundRect(ctx, x, y, 16, 16, 4);
    ctx.fill();
    ctx.strokeStyle = on ? C.accent : C.muted;
    ctx.lineWidth = 1.4;
    ctx.beginPath();
    if (on) {
      ctx.moveTo(x + 4, y + 5); ctx.lineTo(x + 12, y + 5); ctx.lineTo(x + 9, y + 9); ctx.lineTo(x + 9, y + 12); ctx.lineTo(x + 7, y + 11); ctx.lineTo(x + 7, y + 9); ctx.closePath();
    } else {
      ctx.moveTo(x + 5, y + 7); ctx.lineTo(x + 8, y + 10); ctx.lineTo(x + 11, y + 7);
    }
    ctx.stroke();
  }

  paintSelection(p, C) {
    const ctx = this.ctx;
    const rg = this.range();
    const clampR = (r) => Math.min(Math.max(r, p.r1 - 1), p.r2 + 1);
    const clampC = (c) => Math.min(Math.max(c, p.c1 - 1), p.c2 + 1);
    // Cell states a table editor sheet shows: new rows, not sent yet, needs a choice, sent.
    for (const m of this.stateMarks || []) {
      const right = m.right ?? m.col;
      if (m.sheet !== this.sheet || m.row < p.r1 || m.row > p.r2 || right < p.c1 || m.col > p.c2) continue;
      const mx = this.x(Math.max(m.col, p.c1)); const my = this.y(m.row);
      const mw = this.x(Math.min(right, p.c2)) + this.cols.size(Math.min(right, p.c2)) - mx; const mh = this.rows.size(m.row);
      const look = STATE_LOOK[m.kind];
      if (!look) continue;
      if (look.fill) { ctx.fillStyle = look.fill; ctx.fillRect(mx, my, mw, mh); }
      if (look.line) { ctx.strokeStyle = look.line; ctx.lineWidth = 1.5; ctx.strokeRect(mx + 1.5, my + 1.5, mw - 3, mh - 3); }
    }
    const r1 = clampR(rg.top); const r2 = clampR(rg.bottom); const c1 = clampC(rg.left); const c2 = clampC(rg.right);
    const x1 = this.x(c1); const y1 = this.y(r1);
    const x2 = this.x(c2) + this.cols.size(c2); const y2 = this.y(r2) + this.rows.size(r2);
    const am0 = this.mergeAt(this.active.r, this.active.c);
    const multi = (rg.top !== rg.bottom || rg.left !== rg.right) && !(am0 && rg.top === am0.r && rg.left === am0.c && rg.bottom === am0.r + am0.h - 1 && rg.right === am0.c + am0.w - 1);
    if (multi) {
      // A wash over the selection, except the active cell.
      const a = this.active;
      const ax = this.x(a.c); const ay = this.y(a.r);
      ctx.save();
      ctx.beginPath();
      ctx.rect(x1, y1, x2 - x1, y2 - y1);
      ctx.rect(ax + this.cols.size(a.c), ay, -this.cols.size(a.c), this.rows.size(a.r));
      ctx.clip('evenodd');
      ctx.fillStyle = C.sel;
      ctx.globalAlpha = 0.5;
      ctx.fillRect(x1, y1, x2 - x1, y2 - y1);
      ctx.restore();
    }
    ctx.strokeStyle = C.accent;
    ctx.lineWidth = 2;
    ctx.strokeRect(x1 + 0.5, y1 + 0.5, x2 - x1 - 2, y2 - y1 - 2);
    const am = this.mergeAt(this.active.r, this.active.c);
    const multiCells = multi && !(am && rg.top === am.r && rg.left === am.c && rg.bottom === am.r + am.h - 1 && rg.right === am.c + am.w - 1);
    if (multiCells) {
      const a = this.active;
      const b = am ? this.mergeBox(am) : { x: this.x(a.c), y: this.y(a.r), w: this.cols.size(a.c), h: this.rows.size(a.r) };
      ctx.lineWidth = 1;
      ctx.strokeRect(b.x + 1.5, b.y + 1.5, b.w - 4, b.h - 4);
    }
    // Marks other screens put on the sheet (look up's table and columns).
    for (const m of [...(this.marks || []), ...(this.peers || [])]) {
      if (m.sheet !== this.sheet) continue;
      const mr1 = clampR(m.top); const mr2 = clampR(m.bottom); const mc1 = clampC(m.left); const mc2 = clampC(m.right);
      const mx1 = this.x(mc1); const my1 = this.y(mr1);
      const mx2 = this.x(mc2) + this.cols.size(mc2); const my2 = this.y(mr2) + this.rows.size(mr2);
      if (m.fill) { ctx.fillStyle = m.fill; ctx.fillRect(mx1, my1, mx2 - mx1, my2 - my1); }
      ctx.strokeStyle = m.color;
      ctx.lineWidth = 2;
      if (m.dash) ctx.setLineDash([6, 4]);
      ctx.strokeRect(mx1 + 1, my1 + 1, mx2 - mx1 - 2, my2 - my1 - 2);
      ctx.setLineDash([]);
      // A tag under the marked cells (someone's cursor: above, over the cell's top edge).
      if (m.label && (m.above ? m.top >= p.r1 && m.top <= p.r2 : m.bottom >= p.r1 - 1 && m.bottom <= p.r2)) {
        ctx.font = m.above ? '600 10px Figtree, sans-serif' : '600 11px Figtree, sans-serif';
        const tw = ctx.measureText(m.label).width + (m.above ? 8 : 10);
        // Someone's cursor: at its top right corner, astride the edge (it hides the least).
        const tx = m.above ? Math.max(mx1, mx2 - tw) : mx1;
        const ty = m.above ? Math.max(my1 - 7, 0) : my2;
        ctx.fillStyle = m.color;
        ctx.fillRect(tx, ty, tw, m.above ? 13 : 17);
        ctx.fillStyle = '#fff';
        ctx.textBaseline = 'middle';
        ctx.fillText(m.label, tx + (m.above ? 4 : 5), ty + (m.above ? 7 : 9));
      }
    }
    // Where a copy came from: a dashed line around it.
    if (this.copied && this.copied.sheet === this.sheet) {
      const q = this.copied.range;
      const qx1 = this.x(clampC(q.left)); const qy1 = this.y(clampR(q.top));
      const qx2 = this.x(clampC(q.right)) + this.cols.size(clampC(q.right)); const qy2 = this.y(clampR(q.bottom)) + this.rows.size(clampR(q.bottom));
      ctx.setLineDash([5, 3]);
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = C.accentInk;
      ctx.strokeRect(qx1 + 1, qy1 + 1, qx2 - qx1 - 3, qy2 - qy1 - 3);
      ctx.setLineDash([]);
    }
  }

  paintHeaders(v, C) {
    const ctx = this.ctx;
    const rg = this.range();
    const fr = this.frozen?.r || 0; const fc = this.frozen?.c || 0;
    // Column letters.
    ctx.fillStyle = C.soft;
    ctx.fillRect(0, 0, this.w, HDR_H);
    ctx.fillRect(0, 0, HDR_W, this.h);
    ctx.font = '500 12.5px Figtree, system-ui, sans-serif';
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'center';
    const colsShown = [];
    for (let c = 1; c <= fc; c++) colsShown.push(c);
    for (let c = v.c1; c <= v.c2; c++) colsShown.push(c);
    ctx.save();
    for (const c of colsShown) {
      const w = this.cols.size(c);
      if (!w) continue;
      const x = this.x(c);
      if (c > fc && x < HDR_W + this.frozenW()) continue;
      const on = c >= rg.left && c <= rg.right;
      if (on) { ctx.fillStyle = C.accentSoft; ctx.fillRect(x, 0, w, HDR_H); }
      ctx.fillStyle = on ? C.accentInk : C.muted;
      ctx.font = `${on ? 700 : 500} 12.5px Figtree, system-ui, sans-serif`;
      ctx.fillText(colName(c), x + w / 2, HDR_H / 2 + 1);
      ctx.fillStyle = C.line;
      ctx.fillRect(x + w - 1, 0, 1, HDR_H);
    }
    // Row numbers (and the add-a-row lines).
    const rowsShown = [];
    for (let r = 1; r <= fr; r++) rowsShown.push(r);
    for (let r = v.r1; r <= v.r2; r++) rowsShown.push(r);
    for (const r of rowsShown) {
      const hgt = this.rows.size(r);
      const y = this.y(r);
      if (r > fr && y < HDR_H + this.frozenH()) continue;
      if (hgt) {
        const on = r >= rg.top && r <= rg.bottom;
        if (on) { ctx.fillStyle = C.accentSoft; ctx.fillRect(0, y, HDR_W, hgt); }
        ctx.fillStyle = on ? C.accentInk : C.muted;
        ctx.font = `${on ? 700 : 500} 12.5px Figtree, system-ui, sans-serif`;
        ctx.fillText(String(r), HDR_W / 2, y + hgt / 2 + 1);
        ctx.fillStyle = C.line;
        ctx.fillRect(0, y + hgt - 1, HDR_W, 1);
      }
      if (this.ghosts?.has(r)) {
        ctx.fillStyle = '#A8A29E';
        ctx.font = '600 14px Figtree, sans-serif';
        ctx.fillText('+', HDR_W / 2, y + hgt + this.rows.def / 2);
        ctx.fillStyle = C.line;
        ctx.fillRect(0, y + hgt + this.rows.def - 1, HDR_W, 1);
      }
    }
    ctx.restore();
    ctx.textAlign = 'left';
    ctx.fillStyle = C.line2;
    ctx.fillRect(0, HDR_H - 1, this.w, 1);
    ctx.fillRect(HDR_W - 1, 0, 1, this.h);
    ctx.fillStyle = C.soft;
    ctx.fillRect(0, 0, HDR_W - 1, HDR_H - 1);
  }

  // ----- Typing in a cell -----

  /// Starts typing in the active cell (or the add-a-row line `ghost`), with
  /// `text` there (typing a character replaces; F2 keeps what's there).
  startEdit(text = null, ghost = null) {
    if (this.locked() && !ghost) { this.onLocked?.(); return; }
    const a = this.active;
    const current = ghost ? '' : this.call('cell', { sheet: this.sheet, row: a.r, col: a.c }).input;
    this.editing = { r: a.r, c: a.c, ghost, original: current, sheet: this.sheet };
    const ed = this.editor;
    ed.value = text != null ? text : current;
    ed.hidden = false;
    this.placeEditor();
    ed.focus();
    ed.setSelectionRange(ed.value.length, ed.value.length);
    this.onEditing?.(ed.value);
  }

  placeEditor() {
    if (!this.editing) return;
    // On another sheet the cell isn't shown: the formula bar shows what's typed.
    this.editor.classList.toggle('away', this.editing.sheet !== this.sheet);
    this.onAway?.(this.editing.sheet !== this.sheet);
    const { r, c, ghost } = this.editing;
    const ed = this.editor;
    let x = this.x(c); let y = this.y(r);
    if (ghost) { x = this.x(ghost.left); y = this.y(r) + this.rows.size(r); }
    const m = !ghost && this.mergeAt(r, c);
    const box = m ? this.mergeBox(m) : null;
    const w = ghost ? this.x(ghost.right) + this.cols.size(ghost.right) - x : box ? box.w : this.cols.size(c);
    ed.style.left = `${x - 1}px`;
    ed.style.top = `${y - 1}px`;
    ed.style.minWidth = `${w + 1}px`;
    ed.style.height = `${(ghost ? this.rows.def : box ? box.h : this.rows.size(r)) + 1}px`;
    // Grows to fit what's typed.
    ed.style.width = 'auto';
    ed.style.width = `${Math.max(w + 1, Math.min(600, ed.scrollWidth + 4))}px`;
  }

  /// Finishes typing: `commit` keeps it; then moves by (`dr`, `dc`).
  endEdit(commit = true, dr = 0, dc = 0) {
    if (!this.editing) return;
    const e = this.editing;
    const value = this.editor.value;
    this.editing = null;
    this.point = null;
    this.editor.hidden = true;
    this.onAway?.(false);
    // Typed on another sheet's cell (references were picked here): back to it first.
    if (e.sheet !== this.sheet) {
      this.onSheetBack?.(e.sheet);
      this.active = { r: e.r, c: e.c }; this.anchor = { ...this.active }; this.cursor = { ...this.active };
      this.reveal(this.active);
    }
    this.scroller.focus({ preventScroll: true });
    if (commit) {
      if (e.ghost) this.onAddRow?.(e.ghost, e.c, value);
      else if (value !== e.original) this.onCommit?.(e.r, e.c, value);
    }
    this.onEditing?.(null);
    if (dr || dc) this.select(this.active.r + dr, this.active.c + dc);
    else this.draw();
  }

  /// Whether a click should put a reference into the formula being typed.
  pointing() {
    if (!this.editing || this.editing.ghost) return false;
    const v = this.editor.value;
    if (!v.startsWith('=')) return false;
    const at = this.editor.selectionStart;
    if (this.point && this.point.end === at) return true;
    const before = v.slice(0, at).trimEnd();
    return /[=(,;+\-*/^&<>:]$/.test(before);
  }

  /// Puts a reference to `rg` where the caret is (replacing the one put there
  /// by the last click while pointing).
  putReference(rg) {
    const ed = this.editor;
    let ref = rg.top === rg.bottom && rg.left === rg.right ? cellName(rg.top, rg.left) : `${cellName(rg.top, rg.left)}:${cellName(rg.bottom, rg.right)}`;
    if (this.editing && this.editing.sheet !== this.sheet) {
      const name = this.call('sheets').find((x) => x.id === this.sheet)?.name || '';
      ref = `${/^[A-Za-z_][A-Za-z0-9_.]*$/.test(name) ? name : `'${name.replace(/'/g, "''")}'`}!${ref}`;
    }
    const start = this.point ? this.point.start : ed.selectionStart;
    const end = this.point ? this.point.end : ed.selectionEnd;
    ed.value = ed.value.slice(0, start) + ref + ed.value.slice(end);
    this.point = { start, end: start + ref.length, from: this.point?.from || { r: rg.top, c: rg.left } };
    ed.setSelectionRange(this.point.end, this.point.end);
    ed.focus();
    this.placeEditor();
    this.onEditing?.(ed.value);
  }

  // ----- Pointer and keys -----

  bind() {
    const s = this.scroller;
    s.addEventListener('scroll', () => { this.grow(); this.draw(); this.onScroll?.(); });
    s.addEventListener('pointerdown', (e) => this.down(e));
    s.addEventListener('pointermove', (e) => this.move(e));
    s.addEventListener('dblclick', (e) => this.dbl(e));
    s.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const h = this.hitEvent(e);
      if (h.kind === 'cell' || h.kind === 'totals' || h.kind === 'filter') {
        const rg = this.range();
        if (!(h.r >= rg.top && h.r <= rg.bottom && h.c >= rg.left && h.c <= rg.right)) this.select(h.r, h.c);
      } else if (h.kind === 'colhead') {
        const rg = this.range();
        if (!(rg.top === 1 && rg.bottom === MAX_ROW && h.c >= rg.left && h.c <= rg.right)) this.selectRange({ top: 1, left: h.c, bottom: MAX_ROW, right: h.c });
      } else if (h.kind === 'rowhead') {
        const rg = this.range();
        if (!(rg.left === 1 && rg.right === MAX_COL && h.r >= rg.top && h.r <= rg.bottom)) this.selectRange({ top: h.r, left: 1, bottom: h.r, right: MAX_COL });
      }
      this.onContext?.(h, e.clientX, e.clientY);
    });
    s.addEventListener('keydown', (e) => this.key(e));
    this.editor.addEventListener('keydown', (e) => this.editKey(e));
    this.editor.addEventListener('input', () => { this.point = null; this.placeEditor(); this.onEditing?.(this.editor.value); });
    this.editor.addEventListener('blur', () => {
      // Clicking cells while pointing keeps typing; anything else finishes.
      setTimeout(() => { if (this.editing && document.activeElement !== this.editor && !this.keepEditing) this.endEdit(true); }, 0);
    });
  }

  hitEvent(e) {
    const r = this.host.getBoundingClientRect();
    return this.hit(e.clientX - r.left, e.clientY - r.top);
  }

  down(e) {
    if (e.button !== 0 && e.button !== undefined) return;
    const px = e.clientX - this.host.getBoundingClientRect().left;
    const py = e.clientY - this.host.getBoundingClientRect().top;
    // The scroll bars are the scroller's own.
    if (px > this.w || py > this.h) return;
    const h = this.hit(px, py);
    if (this.editing && this.pointing() && h.kind === 'cell') {
      e.preventDefault();
      this.keepEditing = true;
      // Shift+click: from the cell picked before to this one (B3, then Shift+click B6: B3:B6).
      const from = e.shiftKey && this.point?.from ? this.point.from : { r: h.r, c: h.c };
      if (this.point && !e.shiftKey) this.point.from = from;
      this.putReference({ top: Math.min(from.r, h.r), left: Math.min(from.c, h.c), bottom: Math.max(from.r, h.r), right: Math.max(from.c, h.c) });
      this.drag(e, (hh) => {
        if (hh.kind !== 'cell') return;
        this.putReference({ top: Math.min(from.r, hh.r), left: Math.min(from.c, hh.c), bottom: Math.max(from.r, hh.r), right: Math.max(from.c, hh.c) });
      }, () => { this.keepEditing = false; this.editor.focus(); });
      return;
    }
    if (this.editing) this.endEdit(true);
    e.preventDefault();
    this.scroller.focus({ preventScroll: true });
    // Look up and other steps that ask for a click on the sheet take it.
    if (this.onPick && (h.kind === 'cell' || h.kind === 'totals' || h.kind === 'filter') && this.onPick(h)) return;
    if (h.kind === 'colborder' || h.kind === 'rowborder') { this.resizeLine(e, h); return; }
    // The active cell's list button (just right of it).
    const a = this.active;
    const ch = this.choicesAt(a.r, a.c);
    if (ch && h.kind === 'cell' && h.r === a.r && h.c === a.c + 1 && px - (this.x(a.c) + this.cols.size(a.c)) < 20) {
      this.onChoices?.(ch, a, e.clientX, e.clientY);
      return;
    }
    if (h.kind === 'totals') { this.select(h.r, h.c); this.onTotals?.(h, e.clientX, e.clientY); return; }
    if (h.kind === 'filter') { this.select(h.r, h.c); this.onFilter?.(h, e.clientX, e.clientY); return; }
    if (h.kind === 'ghost') {
      this.select(h.r, h.c);
      this.startEdit('', h.table);
      return;
    }
    if (h.kind === 'corner') { this.selectRange({ top: 1, left: 1, bottom: MAX_ROW, right: MAX_COL }); return; }
    if (h.kind === 'colhead') {
      const start = e.shiftKey ? this.anchor.c : h.c;
      this.selectRange({ top: 1, left: Math.min(start, h.c), bottom: MAX_ROW, right: Math.max(start, h.c) });
      this.active = { r: 1, c: start };
      this.drag(e, (hh) => { if (hh.c) { this.anchor = { r: 1, c: start }; this.cursor = { r: MAX_ROW, c: hh.c }; this.selected(); this.draw(); } });
      return;
    }
    if (h.kind === 'rowhead') {
      const start = e.shiftKey ? this.anchor.r : h.r;
      this.selectRange({ top: Math.min(start, h.r), left: 1, bottom: Math.max(start, h.r), right: MAX_COL });
      this.active = { r: start, c: 1 };
      this.drag(e, (hh) => { if (hh.r) { this.anchor = { r: start, c: 1 }; this.cursor = { r: hh.r, c: MAX_COL }; this.selected(); this.draw(); } });
      return;
    }
    if (e.shiftKey) { this.select(h.r, h.c, true); return; }
    this.select(h.r, h.c);
    this.drag(e, (hh) => {
      if (hh.kind === 'cell' || hh.kind === 'totals' || hh.kind === 'filter' || hh.kind === 'ghost') {
        this.cursor = { r: hh.r, c: hh.c };
        this.selected();
        this.draw();
      }
    });
  }

  /// Follows a drag (scrolling when it leaves the sheet) until the button is let go.
  drag(e, onMove, onEnd) {
    const s = this.scroller;
    s.setPointerCapture(e.pointerId);
    let last = e;
    const tick = setInterval(() => {
      const r = this.host.getBoundingClientRect();
      const x = last.clientX - r.left; const y = last.clientY - r.top;
      let moved = false;
      if (y > this.h - 4) { s.scrollTop += 20; moved = true; } else if (y < HDR_H && s.scrollTop > 0) { s.scrollTop -= 20; moved = true; }
      if (x > this.w - 4) { s.scrollLeft += 20; moved = true; } else if (x < HDR_W && s.scrollLeft > 0) { s.scrollLeft -= 20; moved = true; }
      if (moved) onMove(this.hit(Math.min(Math.max(x, HDR_W + 1), this.w - 2), Math.min(Math.max(y, HDR_H + 1), this.h - 2)));
    }, 50);
    const move = (ev) => { last = ev; onMove(this.hitEvent(ev)); };
    const up = () => {
      clearInterval(tick);
      s.removeEventListener('pointermove', move);
      s.removeEventListener('pointerup', up);
      s.removeEventListener('pointercancel', up);
      onEnd?.();
    };
    s.addEventListener('pointermove', move);
    s.addEventListener('pointerup', up);
    s.addEventListener('pointercancel', up);
  }

  move(e) {
    if (e.buttons) return;
    const h = this.hitEvent(e);
    // A comment shows while the pointer is over its cell.
    const cell = h.kind === 'cell' ? this.cells.get(`${h.r},${h.c}`) : null;
    // So does the whole text of a cell that doesn't show all of it.
    const said = cell && this.issues?.get(`${h.r},${h.c}`);
    const whole = cell && !cell.comment && said ? { text: said.map((m) => m.message).join('\n'), r: h.r, c: h.c }
      : cell && !cell.comment && this.cut?.has(`${h.r},${h.c}`) ? { text: cell.text, r: h.r, c: h.c } : null;
    this.onHoverComment?.(cell?.comment ? { ...cell.comment, r: h.r, c: h.c } : whole, e.clientX, e.clientY);
    const cur = h.kind === 'colborder' ? 'col-resize' : h.kind === 'rowborder' ? 'row-resize'
      : (h.kind === 'totals' || h.kind === 'filter') ? 'pointer' : h.kind === 'ghost' ? 'text' : 'cell';
    if (this.scroller.style.cursor !== cur) this.scroller.style.cursor = cur;
  }

  dbl(e) {
    const h = this.hitEvent(e);
    if (h.kind === 'cell') { this.select(h.r, h.c); this.startEdit(); }
    if (h.kind === 'colborder') this.onAutoWidth?.(h.c);
  }

  resizeLine(e, h) {
    const cols = h.kind === 'colborder';
    const i = cols ? h.c : h.r;
    const axis = cols ? this.cols : this.rows;
    const start = cols ? e.clientX : e.clientY;
    const was = axis.size(i);
    let now = was;
    // Every selected column (or row) takes the new size when this one is among them.
    const rg = this.range();
    const many = cols ? (rg.top === 1 && rg.bottom === MAX_ROW && i >= rg.left && i <= rg.right) : (rg.left === 1 && rg.right === MAX_COL && i >= rg.top && i <= rg.bottom);
    this.drag(e, () => {}, () => {
      if (now !== was) {
        const list = many ? range(cols ? rg.left : rg.top, cols ? rg.right : rg.bottom) : [i];
        this.onResize?.(cols, list, now);
      }
    });
    const move = (ev) => {
      now = Math.max(cols ? 20 : 12, Math.round(was + (cols ? ev.clientX : ev.clientY) - start));
      const sizes = new Map(axis.sizes);
      sizes.set(i, now);
      axis.set(axis.def, sizes, axis.gaps);
      this.viewKey = '';
      this.grow();
      this.draw();
    };
    this.scroller.addEventListener('pointermove', move);
    const up = () => { this.scroller.removeEventListener('pointermove', move); this.scroller.removeEventListener('pointerup', up); };
    this.scroller.addEventListener('pointerup', up);
  }

  /// Ctrl+arrow: to the end of the run of filled cells.
  jump(dr, dc, extend) {
    const from = extend ? this.cursor : this.active;
    const to = this.call('edge', { sheet: this.sheet, row: from.r, col: from.c, dr, dc });
    this.select(to.row, to.col, extend);
  }

  pageRows() {
    return Math.max(1, Math.floor((this.h - HDR_H - this.frozenH()) / this.rows.def) - 1);
  }

  key(e) {
    if (this.editing) return;
    // Alt+Down: the active cell's list of choices.
    if (e.altKey && e.key === 'ArrowDown') {
      const ch = this.choicesAt(this.active.r, this.active.c);
      if (ch) {
        e.preventDefault();
        const x = this.host.getBoundingClientRect().left + this.x(this.active.c);
        const y = this.host.getBoundingClientRect().top + this.y(this.active.r) + this.rows.size(this.active.r);
        this.onChoices?.(ch, this.active, x, y);
        return;
      }
    }
    const ctrl = e.ctrlKey || e.metaKey;
    const ext = e.shiftKey;
    const cur = ext ? this.cursor : this.active;
    const go = (dr, dc) => {
      e.preventDefault();
      if (ctrl) { this.jump(dr, dc, ext); return; }
      // From a merged cell, the first step goes past all of it.
      const m = this.mergeAt(cur.r, cur.c);
      const r = m && dr > 0 ? m.r + m.h : cur.r + dr;
      const c = m && dc > 0 ? m.c + m.w : cur.c + dc;
      this.select(r, c, ext);
    };
    switch (e.key) {
      case 'ArrowDown': return go(1, 0);
      case 'ArrowUp': return go(-1, 0);
      case 'ArrowRight': return go(0, 1);
      case 'ArrowLeft': return go(0, -1);
      case 'PageDown': e.preventDefault(); return this.select(cur.r + this.pageRows(), cur.c, ext);
      case 'PageUp': e.preventDefault(); return this.select(cur.r - this.pageRows(), cur.c, ext);
      case 'Home': e.preventDefault(); return ctrl ? this.select(1, 1, ext) : this.select(cur.r, 1, ext);
      case 'End': {
        e.preventDefault();
        const used = this.layout.used;
        return ctrl ? this.select(used[0], used[1], ext) : this.jump(0, 1, ext);
      }
      case 'Enter': {
        e.preventDefault();
        // In a table, Enter in the last data row goes on to the add-a-row line.
        const t = this.tableAt(this.active.r, this.active.c);
        if (!ext && t && t.totals && this.active.r === t.bottom - 1) { this.startEdit('', t); return; }
        return this.select(this.active.r + (ext ? -1 : 1), this.active.c);
      }
      case 'Tab': e.preventDefault(); return this.select(this.active.r, this.active.c + (ext ? -1 : 1));
      case 'F2': e.preventDefault(); return this.startEdit();
      case 'Delete':
      case 'Backspace':
        e.preventDefault();
        if (e.key === 'Backspace' && !ext) return this.startEdit('');
        return this.onClear?.();
      case 'Escape':
        if (this.copied) { this.copied = null; this.draw(); }
        return;
      default:
    }
    if (!ctrl && !e.altKey && e.key.length === 1) {
      e.preventDefault();
      this.startEdit(e.key);
    }
  }

  editKey(e) {
    const ed = this.editor;
    if (e.key === 'Enter' && e.altKey) {
      e.preventDefault();
      const at = ed.selectionStart;
      ed.value = `${ed.value.slice(0, at)}\n${ed.value.slice(ed.selectionEnd)}`;
      ed.setSelectionRange(at + 1, at + 1);
      return;
    }
    if (e.key === 'Enter') { e.preventDefault(); this.endEdit(true, this.editing?.ghost ? 0 : (e.shiftKey ? -1 : 1), 0); return; }
    if (e.key === 'Tab') { e.preventDefault(); this.endEdit(true, 0, e.shiftKey ? -1 : 1); return; }
    if (e.key === 'Escape') { e.preventDefault(); this.endEdit(false); return; }
    // Arrows finish typing and move, unless the caret has somewhere to go (F2 editing) or a formula is pointing.
    if (['ArrowUp', 'ArrowDown'].includes(e.key) && !ed.value.includes('\n') && !this.editing?.ghost && this.editing?.original === '' ) {
      e.preventDefault();
      this.endEdit(true, e.key === 'ArrowUp' ? -1 : 1, 0);
    }
  }
}

function range(a, b) {
  const out = [];
  for (let i = a; i <= b; i++) out.push(i);
  return out;
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}
