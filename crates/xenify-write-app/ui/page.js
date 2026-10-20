// The pages: drawn on canvases from the engine's display lists, with the
// caret and selection on top, and the keyboard and mouse sent back to the
// engine. Document units are points; `zoom` is screen pixels per point.

import { call } from './engine.js';

/// Replaces an element's children, skipping empty ones (null, false) and
/// flattening lists.
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

const PT = 96 / 72;
window.TRACE = false;
if (window.TRACE) {
  let last = performance.now(), worst = 0, n = 0;
  const tick = (t) => { worst = Math.max(worst, t - last); last = t; if (++n % 30 === 0) { call('log', { text: `frames: worst gap ${worst.toFixed(0)}ms` }); worst = 0; } requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
}
const EM = 1000;
const MARGIN = 28; // around the stack of pages, in pixels

// Keys the engine handles itself (the browser must not).
const KEYS = new Set(['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown',
  'Backspace', 'Delete', 'Enter', 'Tab']);
const COMMAND_KEYS = new Set(['a', 'z', 'y', 'b', 'i', 'u']);

export class PageView {
  constructor({ scroller, stack, overlay, ime, ruler, onView, onResult, isActive, onActivate }) {
    this.onResult = onResult;
    // Split view: the pane not being worked in shows `docId` (read only).
    this.isActive = isActive || (() => true);
    this.onActivate = onActivate;
    this.docId = null;
    this.scroller = scroller;
    this.stack = stack;
    this.overlay = overlay;
    this.ime = ime;
    this.ruler = ruler;
    this.onView = onView;
    this.zoomPercent = 100;
    this.view = null;
    this.canvases = new Map();   // page index → canvas
    this.drawn = new Map();      // page index → signature drawn
    this.paths = new Map();      // "font:glyph" → Path2D
    this.drawing = false;
    this.redrawAgain = false;
    this.paintSeq = 0;
    this.selectedText = null;

    this.caret = document.createElement('div');
    this.caret.className = 'caret';
    this.overlay.appendChild(this.caret);
    this.selLayer = document.createElement('div');
    this.overlay.appendChild(this.selLayer);
    this.markLayer = document.createElement('div');
    this.overlay.appendChild(this.markLayer);
    this.frameLayer = document.createElement('div');
    this.overlay.appendChild(this.frameLayer);
    this.marks = [];

    scroller.addEventListener('scroll', () => this.draw());
    // Laid out on the next frame (laying out inside the callback can resize the
    // scroller again, which the browser reports as a loop); a new height only
    // needs the newly visible pages drawn.
    let lastWidth = -1, pending = false;
    new ResizeObserver(() => {
      if (pending) return;
      pending = true;
      requestAnimationFrame(() => {
        pending = false;
        if (scroller.clientWidth !== lastWidth) { lastWidth = scroller.clientWidth; this.layoutPages(); } else this.draw();
      });
    }).observe(scroller);
    this.wireKeyboard();
    this.wireMouse();
  }

  get zoom() { return PT * this.zoomPercent / 100; }

  setZoom(percent) {
    const old = this.zoom;
    const anchor = this.scroller.scrollTop / old;
    this.zoomPercent = Math.max(25, Math.min(400, Math.round(percent)));
    this.drawn.clear();
    this.layoutPages();
    this.scroller.scrollTop = anchor * this.zoom;
  }

  focus() { this.ime.focus({ preventScroll: true }); }

  /// Shows the view of a tab that isn't in front (the inactive pane).
  async refresh() {
    if (this.isActive() || this.docId == null) return;
    try {
      const out = await call('view', { doc: this.docId });
      if (out?.view) { this.drawn.clear(); this.show(out.view); }
    } catch { /* the tab was closed */ }
  }

  /// A new view from the engine (after any call).
  /// Spelling and grammar underlines (from the engine's `marks`).
  setMarks(marks) {
    this.marks = marks || [];
    if (this.view) this.placeMarks();
  }

  show(view, { reveal = false } = {}) {
    this.view = view;
    this.selectedText = view.selected_text;
    this.mirror(view);
    this.layoutPages();
    if (reveal) this.revealCaret();
    if (this.isActive()) this.onView(view);
  }

  layoutPages() {
    const v = this.view;
    if (!v) return;
    const z = this.zoom;
    const width = v.width * z;
    const avail = this.scroller.clientWidth;
    // Room on the right for notes in the margin (comments, changes).
    const room = this.rightRoom || 0;
    const left = Math.max(MARGIN, (avail - width - room) / 2);
    this.originX = left;
    this.originY = MARGIN;
    this.stack.style.width = `${Math.max(avail, width + 2 * MARGIN + room)}px`;
    if (this.ruler) this.ruler.el.style.width = this.stack.style.width;
    this.stack.style.height = `${v.height * z + 2 * MARGIN}px`;
    const seen = new Set();
    for (const p of v.pages) {
      seen.add(p.index);
      let c = this.canvases.get(p.index);
      if (!c) {
        c = document.createElement('canvas');
        c.className = 'sheet';
        c.setAttribute('aria-hidden', 'true');
        c.addEventListener('picture', () => { this.drawn.delete(p.index); this.draw(); });
        this.stack.insertBefore(c, this.overlay);
        this.canvases.set(p.index, c);
      }
      c.style.left = `${left + p.x * z}px`;
      c.style.top = `${MARGIN + p.y * z}px`;
      c.style.width = `${p.w * z}px`;
      c.style.height = `${p.h * z}px`;
    }
    for (const [i, c] of this.canvases) {
      if (!seen.has(i)) { c.remove(); this.canvases.delete(i); this.drawn.delete(i); }
    }
    this.placeMarks();
    this.placeRuler();
    this.onLayout?.();
    this.draw();
  }

  // Caret, selection and the hidden text box (where input methods pop up).
  placeMarks() {
    const v = this.view, z = this.zoom;
    const at = (r) => [this.originX + r[0] * z, this.originY + r[1] * z, r[2] * z, r[3] * z];
    if (v.caret) {
      const [x, y, , h] = at(v.caret);
      Object.assign(this.caret.style, { left: `${x}px`, top: `${y}px`, height: `${h}px`, display: 'block' });
      // Restart the blink so the caret is visible right after moving.
      this.caret.classList.remove('blink');
      void this.caret.offsetWidth;
      this.caret.classList.add('blink');
      Object.assign(this.ime.style, { left: `${x}px`, top: `${y}px`, height: `${h}px` });
    } else {
      this.caret.style.display = 'none';
    }
    // Text boxes: a faint dashed outline (screen only); drag it to move the box.
    fill(this.frameLayer, ...(v.frames || []).map(([id, r]) => {
      const d = document.createElement('div');
      d.className = 'frame-edge';
      d.dataset.id = id;
      const [x, y, w, h] = at(r);
      Object.assign(d.style, { left: `${x}px`, top: `${y}px`, width: `${w}px`, height: `${h}px` });
      return d;
    }));
    fill(this.markLayer, ...this.marks.flatMap((m) => m.rects.map((r) => {
      const d = document.createElement('div');
      const [x, y, w, h] = at(r);
      if (m.kind.startsWith('find')) {
        d.className = `hit${m.kind === 'find on' ? ' on' : ''}`;
        Object.assign(d.style, { left: `${x}px`, top: `${y}px`, width: `${Math.max(w, 2)}px`, height: `${h}px` });
        return d;
      }
      d.className = `uline ${m.kind}`;
      Object.assign(d.style, { left: `${x}px`, top: `${y + h - 3 * Math.max(1, z / 1.3)}px`, width: `${Math.max(w, 4)}px` });
      return d;
    })));
    fill(this.selLayer, ...v.selection.map((r) => {
      const d = document.createElement('div');
      d.className = 'sel';
      const [x, y, w, h] = at(r);
      Object.assign(d.style, { left: `${x}px`, top: `${y}px`, width: `${Math.max(w, 2)}px`, height: `${h}px` });
      return d;
    }));
  }

  placeRuler() {
    const v = this.view;
    if (!this.ruler || !v.pages.length) return;
    const p = v.pages[Math.min(v.caret_page, v.pages.length - 1)];
    const z = this.zoom;
    this.ruler.update({
      left: this.originX + p.x * z,
      width: p.w * z,
      body: [p.body[0] * z, p.body[2] * z],
      zoom: z,
      indents: v.indents,
    });
  }

  revealCaret() {
    const v = this.view;
    const r = v.caret || v.selection[v.selection.length - 1];
    if (!r) return;
    const z = this.zoom;
    const top = this.stack.offsetTop + this.originY + r[1] * z, bottom = top + r[3] * z;
    const s = this.scroller, pad = 60;
    if (top < s.scrollTop + pad) s.scrollTop = top - pad;
    else if (bottom > s.scrollTop + s.clientHeight - pad) s.scrollTop = bottom - s.clientHeight + pad;
  }

  // ----- Drawing -----

  visiblePages() {
    const s = this.scroller, z = this.zoom;
    const off = this.stack.offsetTop;
    const top = s.scrollTop - off - 200, bottom = s.scrollTop - off + s.clientHeight + 200;
    return this.view.pages.filter((p) => {
      const y0 = this.originY + p.y * z, y1 = y0 + p.h * z;
      return y1 >= top && y0 <= bottom;
    });
  }

  async draw() {
    if (!this.view) return;
    if (this.drawing) { this.redrawAgain = true; return; }
    this.drawing = true;
    try {
      const want = this.visiblePages().filter((p) => this.drawn.get(p.index) !== p.sig);
      if (want.length) {
        const scenes = await call('scenes', { pages: want.map((p) => p.index), ...(this.isActive() ? {} : { doc: this.docId }) });
        await this.paintScenes(this.view, scenes);
      }
    } catch (e) {
      console.error('drawing pages', e);
    } finally {
      this.drawing = false;
      if (this.redrawAgain) { this.redrawAgain = false; this.draw(); }
    }
  }

  loadGlyphs(scenes) { return loadGlyphs(this.paths, scenes); }

  /// Paints scenes onto their canvases: only what changed since the last
  /// paint of the same page at the same zoom.
  async paintScenes(view, scenes) {
    // Paints are numbered: one that finishes after a newer one (its glyphs
    // took longer to load) must not paint older text over newer.
    const seq = ++this.paintSeq;
    await this.loadGlyphs(scenes);
    for (const sc of scenes) {
      const page = view.pages[sc.index];
      if (!page || page.sig !== sc.sig) continue;
      let c = this.canvases.get(sc.index);
      if (!c) continue;
      if ((c.paintedSeq || 0) > seq) continue;
      c.paintedSeq = seq;
      const before = c.drawnScene && c.drawnZoom === this.zoom && c.drawnW === page.w && c.drawnH === page.h ? c.drawnScene : null;
      paintScene(c, page, sc, this.zoom, this.paths, before);
      c.drawnScene = sc;
      c.drawnZoom = this.zoom;
      c.drawnW = page.w;
      c.drawnH = page.h;
      this.drawn.set(sc.index, sc.sig);
    }
  }

  // ----- Input -----

  /// What is drawn on the pages on screen (index → signature).
  have() {
    const have = {};
    if (!this.view) return have;
    for (const p of this.visiblePages()) if (this.drawn.has(p.index)) have[p.index] = this.drawn.get(p.index);
    return have;
  }

  async send(name, args, opts = { reveal: true }) {
    try {
      const out = await call(name, { ...args, have: this.have() });
      if (out) this.onResult?.(out);
      if (out && out.marks) this.marks = out.marks;
      if (out && out.scenes) await this.paintScenes(out.view, out.scenes);
      if (out && out.view) this.show(out.view, opts);
      return out;
    } catch (e) {
      console.error(name, e);
      return null;
    }
  }

  /// The hidden text box holds the caret's paragraph, its selection where
  /// the caret is: screen readers read it like any text field.
  mirror(view) {
    if (this.composing) return;
    const ime = this.ime;
    const text = (view.para_text || '').replace(/\uFFFC/g, ' ');
    if (ime.value !== text) ime.value = text;
    this.mirrored = text;
    const [a, b] = view.para_sel || [0, 0];
    try { ime.setSelectionRange(a, b); } catch { /* not focused */ }
  }

  wireKeyboard() {
    const ime = this.ime;
    let composing = false;
    ime.addEventListener('compositionstart', () => { composing = true; this.composing = true; });
    ime.addEventListener('compositionend', (e) => {
      composing = false;
      this.composing = false;
      ime.value = this.mirrored || '';
      if (e.data) this.send('text', { text: e.data });
    });
    ime.addEventListener('input', () => {
      if (composing) return;
      // Whatever the browser typed into the box (dead keys, some input
      // methods): what's new compared with the mirrored paragraph.
      const before = this.mirrored || '', now = ime.value;
      let a = 0;
      while (a < before.length && a < now.length && before[a] === now[a]) a++;
      let z = 0;
      while (z < before.length - a && z < now.length - a && before[before.length - 1 - z] === now[now.length - 1 - z]) z++;
      const text = now.slice(a, now.length - z);
      ime.value = before;
      if (text) this.send('text', { text });
    });
    ime.addEventListener('keydown', (e) => {
      if (e.isComposing || composing) return;
      const command = e.ctrlKey || e.metaKey;
      const key = e.key;
      // Ordinary characters go straight from the key press: the browser's
      // "input" event can arrive after a click that came later, and the
      // text would land in the wrong place. Input methods (accents, Chinese,
      // Japanese…) still use "input"/"composition" events.
      if (key.length > 0 && [...key].length === 1 && !command && !(e.altKey && !e.getModifierState('AltGraph'))) {
        e.preventDefault();
        const t0 = performance.now();
        const lag = t0 - (performance.timeOrigin + e.timeStamp - performance.timeOrigin);
        this.send('text', { text: key }).then(() => {
          const t1 = performance.now();
          requestAnimationFrame(() => { if (window.TRACE) call('log', { text: `key ${key}: event→now ${lag.toFixed(0)}ms, call ${(t1 - t0).toFixed(1)}ms, frame ${(performance.now() - t0).toFixed(1)}ms` }); });
        });
        return;
      }
      const mine = KEYS.has(key) || (command && COMMAND_KEYS.has(key.toLowerCase()));
      if (!mine) return;
      e.preventDefault();
      this.send('key', { key, mods: { shift: e.shiftKey, command, alt: e.altKey } });
    });
    // Copy: the text for other programs; Write keeps the formatting for its
    // own pastes. Paste: Write's own copy, else the clipboard's HTML (from a
    // browser or Word), else the text. Ctrl+Shift+V pastes just the text.
    let plainNext = false;
    ime.addEventListener('keydown', (e) => { plainNext = (e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === 'v'; }, true);
    ime.addEventListener('copy', (e) => {
      if (this.selectedText == null) return;
      e.preventDefault();
      e.clipboardData.setData('text/plain', this.selectedText);
      call('copy', {}).catch(() => {});
    });
    ime.addEventListener('cut', (e) => {
      if (this.selectedText == null) return;
      e.preventDefault();
      e.clipboardData.setData('text/plain', this.selectedText);
      this.send('cut', {});
    });
    ime.addEventListener('paste', (e) => {
      e.preventDefault();
      const text = e.clipboardData.getData('text/plain');
      const html = plainNext ? '' : e.clipboardData.getData('text/html');
      const plain = plainNext;
      plainNext = false;
      if (text || html) this.send('paste', { text, html, plain });
    });
  }

  /// The text box (id) whose edge is under the pointer (within a few pixels).
  frameEdgeAt(e) {
    const p = this.docPoint(e), slop = 6 / this.zoom;
    for (const [id, [x, y, w, h]] of this.view?.frames || []) {
      const inside = p.x > x - slop && p.x < x + w + slop && p.y > y - slop && p.y < y + h + slop;
      const deep = p.x > x + slop && p.x < x + w - slop && p.y > y + slop && p.y < y + h - slop;
      if (inside && !deep) return id;
    }
    return null;
  }

  docPoint(e) {
    const r = this.stack.getBoundingClientRect();
    return { x: (e.clientX - r.left - this.originX) / this.zoom, y: (e.clientY - r.top - this.originY) / this.zoom };
  }

  wireMouse() {
    let down = false, busy = false, pending = null;
    const flush = async () => {
      if (busy || !pending) return;
      busy = true;
      const p = pending;
      pending = null;
      await this.send('move', p);
      busy = false;
      flush();
    };
    this.stack.addEventListener('pointerdown', (e) => {
      // The other pane: a click there makes it the one being worked in.
      if (!this.isActive()) { e.preventDefault(); e.stopImmediatePropagation(); this.onActivate?.(); return; }
      if (e.button !== 0) return;
      // Controls laid over the page (header boxes, notes, labels, handles) handle their own clicks.
      if (e.target.closest?.('input, textarea, button, select, .hf-edit, .note, .frame-handle, .tbl-tools > *')) return;
      // On a text box's edge: the box is being grabbed, not the text.
      const fr = this.frameEdgeAt(e);
      if (fr != null && this.onFrameDown) { e.preventDefault(); e.stopImmediatePropagation(); this.onFrameDown(fr, e); return; }
      e.preventDefault();
      down = true;
      this.stack.setPointerCapture(e.pointerId);
      const p = this.docPoint(e);
      this.send('down', { ...p, clicks: e.detail || 1, mods: { shift: e.shiftKey, command: e.ctrlKey || e.metaKey, alt: e.altKey } }, { reveal: false });
      this.focus();
    }, true);
    this.stack.addEventListener('pointermove', (e) => {
      if (!down) return;
      pending = this.docPoint(e);
      flush();
    });
    const up = () => {
      if (!down) return;
      down = false;
      this.send('up', {}, { reveal: false });
    };
    this.stack.addEventListener('pointerup', up);
    this.stack.addEventListener('pointercancel', up);
  }
}

/// Fetches the outlines of glyphs in `scenes` that `paths` doesn't have yet.
export async function loadGlyphs(paths, scenes) {
  const missing = new Map();
  for (const sc of scenes) for (const op of sc.ops) {
    if (op.t !== 'g') continue;
    for (let i = 0; i < op.g.length; i += 3) {
      const key = `${op.f}:${op.g[i]}`;
      if (paths.has(key)) continue;
      if (!missing.has(op.f)) missing.set(op.f, new Set());
      missing.get(op.f).add(op.g[i]);
    }
  }
  for (const [font, set] of missing) {
    const ids = [...set];
    const out = await call('glyphs', { font, ids });
    ids.forEach((id, i) => paths.set(`${font}:${id}`, out[i] ? new Path2D(out[i]) : null));
  }
}

// A key for comparing operations between two paints, and the box (in
// points) an operation can touch.
function opKey(op) {
  if (!op._key) op._key = JSON.stringify(op);
  return op._key;
}
function opBox(op) {
  if (op._box) return op._box;
  let b;
  if (op.t === 'g') {
    let x0 = Infinity, x1 = -Infinity, y0 = Infinity, y1 = -Infinity;
    for (let i = 0; i < op.g.length; i += 3) {
      const x = op.g[i + 1], y = op.g[i + 2];
      if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
    }
    // Generous: ascenders, descenders, wide glyphs and slanted ones.
    b = [x0 - op.s * 0.5, y0 - op.s * 1.3, x1 + op.s * 1.6, y1 + op.s * 0.6];
  } else {
    b = [op.x - 1, op.y - 1, op.x + op.w + 1, op.y + op.h + 1];
  }
  op._box = b;
  return b;
}

/// The area (points) that differs between two display lists, or null.
function changedArea(before, after) {
  const counts = new Map();
  for (const op of before.ops) counts.set(opKey(op), (counts.get(opKey(op)) || 0) + 1);
  const added = [];
  for (const op of after.ops) {
    const k = opKey(op), n = counts.get(k);
    if (n) counts.set(k, n - 1); else added.push(op);
  }
  let box = null;
  const grow = (b) => { box = box ? [Math.min(box[0], b[0]), Math.min(box[1], b[1]), Math.max(box[2], b[2]), Math.max(box[3], b[3])] : b.slice(); };
  for (const op of added) grow(opBox(op));
  for (const op of before.ops) if (counts.get(opKey(op)) > 0) { grow(opBox(op)); counts.set(opKey(op), counts.get(opKey(op)) - 1); }
  return box;
}

/// Draws one page's display list at `zoom` pixels per point. With `before`
/// (what the canvas shows now), only the part that changed is painted.
export function paintScene(canvas, page, scene, zoom, paths, before = null) {
  const dpr = window.devicePixelRatio || 1;
  const k = zoom * dpr;
  const w = Math.round(page.w * k), h = Math.round(page.h * k);
  if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; before = null; }
  const ctx = canvas.getContext('2d');
  let clip = null;
  if (before && before.paper === scene.paper) {
    clip = changedArea(before, scene);
    if (!clip) return;
  }
  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  if (clip) {
    const x = Math.floor(clip[0] * k), y = Math.floor(clip[1] * k);
    const cw = Math.ceil(clip[2] * k) - x, ch = Math.ceil(clip[3] * k) - y;
    ctx.beginPath();
    ctx.rect(x, y, cw, ch);
    ctx.clip();
    ctx.fillStyle = scene.paper;
    ctx.fillRect(x, y, cw, ch);
  } else {
    ctx.fillStyle = scene.paper;
    ctx.fillRect(0, 0, w, h);
  }
  for (const op of scene.ops) {
    if (clip) {
      const b = opBox(op);
      if (b[2] < clip[0] || b[0] > clip[2] || b[3] < clip[1] || b[1] > clip[3]) continue;
    }
    switch (op.t) {
      case 'r':
        ctx.setTransform(k, 0, 0, k, 0, 0);
        ctx.fillStyle = op.c;
        ctx.fillRect(op.x, op.y, op.w, op.h);
        break;
      case 'g': {
        ctx.fillStyle = op.c;
        const s = k * op.s / EM;
        const slant = op.k ? -Math.tan(op.k * Math.PI / 180) * s : 0;
        for (let i = 0; i < op.g.length; i += 3) {
          const p = paths.get(`${op.f}:${op.g[i]}`);
          if (!p) continue;
          ctx.setTransform(s, 0, slant, s, op.g[i + 1] * k, op.g[i + 2] * k);
          ctx.fill(p);
        }
        break;
      }
      case 'e': {
        // The same picture the page and PDF use (the bundled Fluent set).
        ctx.setTransform(k, 0, 0, k, 0, 0);
        const img = emojiPics.get(op.e);
        if (img && img.complete && img.naturalWidth) ctx.drawImage(img, op.x, op.y, op.w, op.h);
        else if (!img) loadEmoji(op.e, canvas);
        break;
      }
      case 'i': {
        ctx.setTransform(k, 0, 0, k, 0, 0);
        const img = pictures.get(op.src);
        if (img && img.complete && img.naturalWidth) ctx.drawImage(img, op.x, op.y, op.w, op.h);
        else {
          ctx.fillStyle = '#EEEAE5';
          ctx.fillRect(op.x, op.y, op.w, op.h);
          if (!img) loadPicture(op.src, canvas);
        }
        break;
      }
    }
  }
  ctx.restore();
}

// Pictures, loaded once each; the page is redrawn when one arrives.
const pictures = new Map();
/// Forgets pictures shown from files (not those kept in the document), so
/// a file changed elsewhere is read again.
export function forgetLinkedPictures() {
  for (const src of [...pictures.keys()]) if (!src.startsWith('media/')) pictures.delete(src);
}
function loadPicture(src, canvas) {
  const img = new Image();
  pictures.set(src, img);
  call('asset', { src }).then((url) => {
    if (!url) return;
    img.onload = () => { canvas.drawnScene = null; canvas.dispatchEvent(new CustomEvent('picture')); };
    img.src = url;
  });
}

const emojiPics = new Map();
function loadEmoji(glyph, canvas) {
  const img = new Image();
  emojiPics.set(glyph, img);
  call('emoji_svg', { glyph }).then((svg) => {
    if (!svg) return;
    img.onload = () => { canvas.drawnScene = null; canvas.dispatchEvent(new CustomEvent('picture')); };
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  });
}
export { loadEmoji, emojiPics };
