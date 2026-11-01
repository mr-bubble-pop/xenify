// Raster layers: the paint tools for pictures (brush, eraser, fill, magic
// wand, selections, colour pick), the brush ring and the selection's
// marching ants on the canvas, and the toolbar while a raster layer is the
// one you're on. The pixels are edited in the engine (raster.rs).

import { call, engineRaw, afterOffscreen } from './engine.js';
import { icon } from './icons.js';

/// The tools the toolbar shows on a raster layer (instead of the vector ones).
export const RASTER_TOOLS = [
  { id: 'r-brush', label: 'Paint brush: paint pixels on the raster layer (Shift+click: a straight line from where the last stroke ended)', key: 'B', icon: 'brush' },
  { id: 'r-eraser', label: 'Eraser: rub pixels out', key: 'E', icon: 'eraser' },
  { id: 'r-smudge', label: 'Smudge or blur: drag colours along like a finger in wet paint, or soften them', key: 'R', icon: 'droplet' },
  { id: 'r-clone', label: 'Clone: Alt+click where to copy from, then paint those pixels somewhere else', key: 'S', icon: 'stamp' },
  { id: 'r-fill', label: 'Fill: click to fill the area of like colour; closes under line art (Grow)', key: 'G', icon: 'bucket' },
  { id: 'r-wand', label: 'Magic wand: click to select the area of like colour (Shift: add to the selection, Alt: take away)', key: 'W', icon: 'wand' },
  { id: 'r-select', label: 'Select an area: a rectangle, an ellipse or freehand (Shift: add, Alt: take away)', key: 'M', icon: 'marquee' },
  { id: 'r-pick', label: 'Pick colour: click the picture to take its colour', key: 'I', icon: 'pipette' },
];

/// The vector tools that stay on the raster toolbar.
export const SHARED_TOOLS = ['select', 'xform', 'mask', 'crop', 'zoom', 'hand'];  // mask and crop: by key or menu

const SELECT_SHAPES = [
  { id: 'rect', label: 'Rectangle', icon: 'marquee' },
  { id: 'ellipse', label: 'Ellipse', icon: 'marqueeEllipse' },
  { id: 'lasso', label: 'Freehand (lasso)', icon: 'lasso' },
];

const DEFAULTS = {
  brush: { size: 24, hardness: 0.75, opacity: 1, pressure: true, keep_alpha: false, steady: 0, pressure_opacity: false },
  eraser: { size: 40, hardness: 0.75, opacity: 1, pressure: true, steady: 0, pressure_opacity: false },
  smudge: { size: 30, hardness: 0.5, opacity: 0.6, pressure: true, mode: 'smudge' },
  clone: { size: 40, hardness: 0.6, opacity: 1, pressure: false, aligned: true, sample: 'layer' },
  /// Brush settings you saved: { name, size, hardness, opacity, pressure }.
  presets: [],
  fill: { tolerance: 20, contiguous: true, sample: 'layer', grow: 1, keep_alpha: false, mode: 'bucket', gradient: 'linear', color2: 'none' },
  wand: { tolerance: 20, contiguous: true, sample: 'layer' },
  select: { shape: 'rect' },
};

/// Ready-made brushes (any of the painting tools can take one).
const PRESETS = [
  { name: 'Pencil', size: 3, hardness: 1, opacity: 1, pressure: false },
  { name: 'Ink pen', size: 6, hardness: 0.9, opacity: 1, pressure: true },
  { name: 'Marker', size: 20, hardness: 0.85, opacity: 0.6, pressure: false },
  { name: 'Round brush', size: 24, hardness: 0.75, opacity: 1, pressure: true },
  { name: 'Soft airbrush', size: 120, hardness: 0, opacity: 0.2, pressure: true },
  { name: 'Big soft brush', size: 250, hardness: 0.2, opacity: 0.6, pressure: false },
];
const PRESET_KEYS = ['size', 'hardness', 'opacity', 'pressure'];

/// The painting tools: their settings in the options, and what the engine's stroke does.
const BRUSHES = { 'r-brush': 'brush', 'r-eraser': 'eraser', 'r-smudge': 'smudge', 'r-clone': 'clone' };
export const isBrush = (tool) => tool in BRUSHES;

const load = () => { try { return JSON.parse(localStorage.getItem('draw.raster') || '{}'); } catch { return {}; } };
const reduceMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

export function initRaster(app) {
  const saved = load();
  const o = Object.fromEntries(Object.entries(DEFAULTS).map(([k, v]) => [k, Array.isArray(v) ? (Array.isArray(saved[k]) ? saved[k] : v) : { ...v, ...(saved[k] || {}) }]));
  const keep = () => { try { localStorage.setItem('draw.raster', JSON.stringify(o)); } catch { /* not kept */ } };
  const vp = () => app.viewport;
  let drag = null;
  let pointer = null;
  let ants = { key: '', segs: [] };
  let antsTimer = null;
  let lastEnd = null;
  let keyShift = false;
  /// Clone: where to copy from (canvas), and the distance kept from the brush once a stroke started.
  const clone = { source: null, offset: null };
  /// Painting the selection (quick mask, Q): the brush adds to it and the eraser takes away.
  let quick = false;
  /// Pixels copied with Ctrl+C: where they were, to paste them back there.
  let pixelClip = null;

  /// The raster layer you're on: the one selected layer, when it's a still picture.
  const layer = () => {
    const s = app.state?.selection || [];
    if (s.length !== 1) return null;
    const l = app.layerById(s[0]);
    return l?.raster ? l : null;
  };
  const info = () => {
    const l = layer();
    if (!l) return null;
    try { return call('raster_info', { id: l.id }); } catch { return null; }
  };
  const mode = (e) => (e.shiftKey && e.altKey ? 'intersect' : e.shiftKey ? 'add' : e.altKey ? 'subtract' : 'replace');

  /// The layer to work on, making a raster layer when there's none and the tool paints.
  const target = (tool) => {
    const l = layer();
    if (l) {
      const i = info();
      if (!i?.paintable) { app.hint?.(i?.why || 'This layer can’t be painted on.'); return null; }
      return l.id;
    }
    if (tool === 'r-brush' || tool === 'r-fill') {
      const id = app.run('add_raster_layer', {});
      if (id != null) app.toast?.('Added a raster layer to paint on.');
      return id;
    }
    app.hint?.('Select a raster layer first (Layer › New raster layer, or Convert to raster).');
    return null;
  };

  /// What the canvas shows, on the layer's grid (fill and wand on "all layers", and the AI).
  const sample = (id) => {
    try { engineRaw().raster_sample(BigInt(id)); } finally { afterOffscreen(); }
  };

  /// The pointer's points since the last event, steadied when the brush asks for it (each point follows
  /// the pointer only part of the way, which irons out shaky hands).
  const points = (e) => {
    const list = e.getCoalescedEvents?.().length ? e.getCoalescedEvents() : [e];
    const k = drag?.steady || 0;
    return list.map((ev) => {
      const p = vp().toComp(ev);
      const pr = ev.pointerType === 'mouse' ? 0.5 : ev.pressure || 0.5;
      if (!k) return [p[0], p[1], pr];
      const s = drag.sm || (drag.sm = [p[0], p[1]]);
      s[0] += (p[0] - s[0]) * (1 - k); s[1] += (p[1] - s[1]) * (1 - k);
      return [s[0], s[1], pr];
    });
  };
  const STEADY = [0, 0.6, 0.85];

  // A selection change is a step you can undo: the whole app catches up (Undo's label, the ants).
  const selChanged = () => { ants.key = ''; app.changed(); };

  function down(e, p) {
    const tool = app.tool;
    if (isBrush(tool)) {
      const b = o[BRUSHES[tool]];
      if (tool === 'r-clone' && e.altKey) {
        // Alt+click: where to copy from (the next stroke starts the copying there).
        if (!layer()) { app.hint?.('Select a raster layer first.'); return false; }
        clone.source = p; clone.offset = null;
        app.hint?.('Now paint where the copy should go.');
        vp().redraw();
        return false;
      }
      const id = target(tool);
      if (id == null) return false;
      let extra = {};
      if (tool === 'r-clone') {
        if (!clone.source) { app.hint?.('Alt+click where to copy from first.'); return false; }
        if (!clone.offset || !b.aligned) clone.offset = [clone.source[0] - p[0], clone.source[1] - p[1]];
        extra = { brush: 'clone', clone_offset: clone.offset };
        if (b.sample === 'canvas') { sample(id); extra.sample = 'canvas'; }
      } else if (tool === 'r-smudge') extra = { brush: b.mode === 'blur' ? 'blur' : 'smudge' };
      const toSel = quick && (tool === 'r-brush' || tool === 'r-eraser');
      if (toSel) extra.target = 'selection';
      drag = { paint: true, id, last: p, quick: toSel, steady: STEADY[b.steady || 0] || 0 };
      try {
        call('raster_begin', { id, color: app.fill, erase: tool === 'r-eraser', size: b.size, hardness: b.hardness, opacity: b.opacity, pressure: b.pressure, pressure_opacity: !!b.pressure_opacity, keep_alpha: !!b.keep_alpha, ...extra });
        // Shift: a straight line on from where the last stroke ended.
        const first = e.shiftKey && lastEnd?.id === id ? [lastEnd.at, ...points(e)] : points(e);
        call('raster_paint', { points: first });
      } catch (err) { drag = null; app.hint?.(String(err.message || err)); return false; }
      vp().redraw();
      return true;
    }
    if (tool === 'r-fill' || tool === 'r-wand') {
      const id = target(tool);
      if (id == null) return false;
      const opts = tool === 'r-fill' ? o.fill : o.wand;
      if (tool === 'r-fill' && opts.mode === 'gradient') { drag = { grad: true, id, from: p, to: p }; return true; }
      if (opts.sample === 'canvas') sample(id);
      if (tool === 'r-fill') {
        if (app.run('raster_fill', { id, x: p[0], y: p[1], color: app.fill, tolerance: opts.tolerance, contiguous: opts.contiguous, sample: opts.sample, grow: opts.grow, keep_alpha: !!opts.keep_alpha }) !== null) vp().redraw();
      } else {
        try { call('raster_wand', { id, x: p[0], y: p[1], tolerance: opts.tolerance, contiguous: opts.contiguous, sample: opts.sample, mode: mode(e) }); } catch (err) { app.hint?.(String(err.message || err)); }
        selChanged();
      }
      return false;
    }
    if (tool === 'r-select') {
      const id = target(tool);
      if (id == null) return false;
      // On a corner of the selection's box: scale the selected pixels; inside the selection: move them.
      const i = !e.shiftKey && !e.altKey ? info() : null;
      const corner = i?.selection_box ? cornerAt(i.selection_box, p) : -1;
      const inside = corner < 0 && i?.selection && call('raster_sel_at', { id, x: p[0], y: p[1] });
      if (corner >= 0 || inside) {
        try { call('raster_float_begin', { id }); } catch (err) { app.hint?.(String(err.message || err)); return false; }
        drag = { float: true, id, from: p, to: p, box: i.selection_box, corner };
        return true;
      }
      drag = { select: o.select.shape, id, from: p, to: p, pts: [p], mode: mode(e) };
      return true;
    }
    if (tool === 'r-pick') {
      const l = layer();
      let hex = null;
      if (l) { try { hex = call('raster_pick', { id: l.id, x: p[0], y: p[1] }); } catch { /* not paintable */ } }
      if (!hex) { app.hint?.('Click on the picture’s colours (that spot is see-through).'); return false; }
      app.fill = hex;
      app.fillOn = true;
      app.renderToolbar?.();
      app.toast?.(`Picked ${hex.toUpperCase()}.`);
      return false;
    }
    return false;
  }

  function move(e, p, dragging) {
    pointer = p;
    keyShift = e.shiftKey;
    if (!dragging || !drag) {
      if (isBrush(app.tool)) vp().redraw();
      // The cursor says what a drag would do.
      if (app.tool === 'r-select') {
        const i = layer() && info();
        const c = i?.selection_box ? cornerAt(i.selection_box, p) : -1;
        vp().overlay.style.cursor = c >= 0 ? (c === 0 || c === 3 ? 'nwse-resize' : 'nesw-resize') : i?.selection && call('raster_sel_at', { id: layer().id, x: p[0], y: p[1] }) ? 'move' : 'crosshair';
      }
      return;
    }
    if (drag.paint) {
      try { call('raster_paint', { points: points(e) }); } catch { /* the layer went away */ }
      drag.last = p;
      // Painting the selection: its edges follow (a few times a second).
      if (drag.quick && performance.now() - (drag.antsAt || 0) > 90) { drag.antsAt = performance.now(); ants.v = (ants.v || 0) + 1; }
    } else if (drag.grad) {
      drag.to = p;
    } else if (drag.float) {
      drag.to = p;
      // One update a frame, however fast the pointer moves (big selections take a while to redraw).
      drag.free = e.shiftKey;
      if (!drag.queued) {
        drag.queued = true;
        const d = drag;
        requestAnimationFrame(() => { d.queued = false; if (drag !== d) return; try { call('raster_float_set', floatArgs(d, d.free)); } catch { /* the layer went away */ } vp().redraw(); });
      }
      return;
    } else if (drag.select) {
      drag.to = p;
      if (drag.select === 'lasso') drag.pts.push(p);
    }
    vp().redraw();
  }

  function up() {
    const d = drag;
    drag = null;
    if (!d) return;
    if (d.paint) {
      try { call('raster_end'); } catch (err) { app.toast?.(String(err.message || err)); }
      lastEnd = { id: d.id, at: [d.last[0], d.last[1], 0.5] };
      if (d.quick) selChanged(); else app.changed();
      return;
    }
    if (d.grad) {
      if (Math.hypot(d.to[0] - d.from[0], d.to[1] - d.from[1]) * vp().k < 3) { app.hint?.('Drag from where the gradient starts to where it ends.'); return; }
      const f = o.fill;
      app.run('raster_gradient', { id: d.id, from: d.from, to: d.to, kind: f.gradient, color: app.fill, color2: f.color2, keep_alpha: !!f.keep_alpha });
      return;
    }
    if (d.float) {
      try { call('raster_float_set', floatArgs(d, d.free)); call('raster_live_end', { keep: true }); } catch (err) { app.toast?.(String(err.message || err)); }
      selChanged();
      return;
    }
    if (d.select) {
      const small = Math.hypot(d.to[0] - d.from[0], d.to[1] - d.from[1]) * vp().k < 3;
      try {
        if (small && d.select !== 'lasso') {
          // A click: nothing selected (or, with Shift/Alt, nothing changed).
          if (d.mode === 'replace') call('raster_select_none', { id: d.id });
        } else if (d.select === 'lasso') {
          if (d.pts.length > 2) call('raster_select', { id: d.id, shape: 'lasso', points: d.pts, mode: d.mode });
        } else {
          call('raster_select', { id: d.id, shape: d.select, rect: [d.from[0], d.from[1], d.to[0], d.to[1]], mode: d.mode });
        }
      } catch (err) { app.hint?.(String(err.message || err)); }
      selChanged();
    }
  }

  /// Which corner of box [x0, y0, x1, y1] (canvas) is under `p`: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right, -1 none.
  function cornerAt(b, p) {
    const r = 7 / vp().k;
    const cs = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]];
    return cs.findIndex((c) => Math.abs(c[0] - p[0]) <= r && Math.abs(c[1] - p[1]) <= r);
  }

  /// The move (and, from a corner, the scale about the box's middle) a float drag asks for. Shift: change the proportions.
  function floatArgs(d, free) {
    if (d.corner < 0) return { dx: d.to[0] - d.from[0], dy: d.to[1] - d.from[1], sx: 1, sy: 1 };
    const b = d.box, c = [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2];
    const k = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]][d.corner];
    let sx = (d.to[0] - c[0]) / (k[0] - c[0] || 1), sy = (d.to[1] - c[1]) / (k[1] - c[1] || 1);
    if (!free) { const m = Math.max(Math.abs(sx), Math.abs(sy)); sx = Math.sign(sx || 1) * m; sy = Math.sign(sy || 1) * m; }
    return { dx: 0, dy: 0, sx, sy };
  }

  /// The box a float drag shows (canvas).
  function floatBox(d, free) {
    const a = floatArgs(d, free), b = d.box, c = [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2];
    const x = (v) => c[0] + (v - c[0]) * a.sx + a.dx, y = (v) => c[1] + (v - c[1]) * a.sy + a.dy;
    return [x(b[0]), y(b[1]), x(b[2]), y(b[3])];
  }

  // ----- Copying and pasting pixels -----

  async function copyPixels(cut) {
    const l = layer();
    if (!l) return;
    let c;
    try { c = call('raster_copy', { id: l.id }); } catch (err) { app.hint?.(String(err.message || err)); return; }
    const bytes = Uint8Array.from(atob(c.base64), (ch) => ch.charCodeAt(0));
    pixelClip = { ...c, size: bytes.length, layer: l.id };
    try { await navigator.clipboard?.write?.([new ClipboardItem({ 'image/png': new Blob([bytes], { type: 'image/png' }) })]); } catch { /* kept here only */ }
    if (cut) app.run('raster_clear', { id: l.id });
    app.toast?.(cut ? 'Cut the selected pixels.' : 'Copied the selected pixels. Ctrl+V pastes them as a new layer in the same place.');
  }

  /// Pastes the pixels copied here as a new layer where they were (when the clipboard still has them). True when it did.
  function pastePixels(files) {
    if (!pixelClip) return false;
    const f = files?.[0];
    if (f && !(f.type === 'image/png' && f.size === pixelClip.size)) return false;
    const id = app.run('add_picture', { base64: pixelClip.base64, name: 'Pasted pixels.png', mime: 'image/png', width: pixelClip.width, height: pixelClip.height, at: [pixelClip.x, pixelClip.y], scale: pixelClip.scale, above: layer()?.id ?? pixelClip.layer });
    if (id != null) { app.setTool('r-select'); app.toast?.('Pasted as a new layer. Drag it with the arrow tool, or paint on it.'); }
    return id != null;
  }

  /// Arrow keys: nudge the selected pixels (Shift: 10 at a time).
  function nudge(id, dx, dy) {
    try {
      call('raster_float_begin', { id });
      call('raster_float_set', { dx, dy, sx: 1, sy: 1 });
      call('raster_live_end', { keep: true });
    } catch (err) { app.hint?.(String(err.message || err)); }
    selChanged();
  }

  function setQuick(on) {
    quick = on;
    if (on && !isBrush(app.tool)) app.setTool('r-brush');
    app.hint?.(on ? 'Painting the selection: the brush adds to it, the eraser takes away (Q: back to painting pixels).' : 'Painting pixels again.');
    app.renderToolbar?.();
  }

  // ----- On the canvas -----

  /// The selection's edges (segments in canvas space), fetched again when the selection or layer changes.
  const antsFor = (l) => {
    const key = `${l.id}/${app.rev}/${ants.v || 0}`;
    if (ants.key !== key) {
      try { ants.segs = call('raster_outline', { id: l.id }); } catch { ants.segs = []; }
      ants.key = key;
    }
    return ants.segs;
  };

  function tickAnts() {
    const l = layer();
    const has = l && antsFor(l).length;
    if (has && !antsTimer && !reduceMotion()) antsTimer = setInterval(() => { if (!layer()) { clearInterval(antsTimer); antsTimer = null; } vp().redraw(); }, 140);
    if (!has && antsTimer) { clearInterval(antsTimer); antsTimer = null; }
  }

  function overlay(g) {
    const v = vp();
    const l = layer();
    if (l) {
      const segs = antsFor(l);
      if (segs.length) {
        g.save();
        g.beginPath();
        for (let i = 0; i + 3 < segs.length; i += 4) {
          const a = v.toScreen([segs[i], segs[i + 1]]), b = v.toScreen([segs[i + 2], segs[i + 3]]);
          g.moveTo(a[0], a[1]);
          g.lineTo(b[0], b[1]);
        }
        g.lineWidth = 1;
        g.strokeStyle = '#fff';
        g.setLineDash([]);
        g.stroke();
        g.strokeStyle = '#000';
        g.setLineDash([4, 4]);
        g.lineDashOffset = reduceMotion() ? 0 : -((performance.now() / 70) % 8);
        g.stroke();
        g.restore();
      }
    }
    const d = drag;
    if (app.tool === 'r-select' && l && (d?.float || !d)) {
      // The selection's box, with corners to scale it by.
      const i = d?.float ? null : info();
      const box = d?.float ? floatBox(d, keyShift) : i?.selection_box;
      if (box) {
        const a = v.toScreen([box[0], box[1]]), b = v.toScreen([box[2], box[3]]);
        g.save();
        g.lineWidth = 1; g.strokeStyle = 'rgba(37,99,235,.9)'; g.setLineDash([]);
        g.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
        g.fillStyle = '#fff';
        for (const [x, y] of [[a[0], a[1]], [b[0], a[1]], [a[0], b[1]], [b[0], b[1]]]) { g.fillRect(x - 4, y - 4, 8, 8); g.strokeRect(x - 4, y - 4, 8, 8); }
        g.restore();
      }
    }
    if (d?.grad) {
      const a = v.toScreen(d.from), b = v.toScreen(d.to);
      g.save();
      for (const [col, wd] of [['rgba(0,0,0,.55)', 3], ['#fff', 1.5]]) {
        g.lineWidth = wd; g.strokeStyle = col;
        g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
        if (o.fill.gradient === 'radial') { g.beginPath(); g.arc(a[0], a[1], Math.hypot(b[0] - a[0], b[1] - a[1]), 0, Math.PI * 2); g.stroke(); }
      }
      g.fillStyle = app.fill; g.beginPath(); g.arc(a[0], a[1], 5, 0, Math.PI * 2); g.fill(); g.stroke();
      g.fillStyle = o.fill.color2 === 'none' ? '#fff' : o.fill.color2; g.beginPath(); g.arc(b[0], b[1], 5, 0, Math.PI * 2); g.fill(); g.stroke();
      g.restore();
    }
    if (d?.select) {
      g.save();
      g.lineWidth = 1;
      g.setLineDash([4, 3]);
      g.strokeStyle = '#000';
      const a = v.toScreen(d.from), b = v.toScreen(d.to);
      g.beginPath();
      if (d.select === 'lasso') d.pts.forEach((q, i) => { const s = v.toScreen(q); if (i) g.lineTo(s[0], s[1]); else g.moveTo(s[0], s[1]); });
      else if (d.select === 'ellipse') g.ellipse((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, Math.abs(b[0] - a[0]) / 2, Math.abs(b[1] - a[1]) / 2, 0, 0, Math.PI * 2);
      else g.rect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.stroke();
      g.strokeStyle = '#fff';
      g.lineDashOffset = 3.5;
      g.stroke();
      g.restore();
    }
    if (app.tool === 'r-clone') {
      // Where the copy comes from: a cross at the source (or where it is now, following the brush).
      const at = clone.offset && pointer ? [pointer[0] + clone.offset[0], pointer[1] + clone.offset[1]] : clone.source;
      if (at) {
        const c = v.toScreen(at);
        g.save();
        for (const [col, wd] of [['rgba(0,0,0,.6)', 3], ['#fff', 1]]) {
          g.lineWidth = wd; g.strokeStyle = col;
          g.beginPath(); g.moveTo(c[0] - 8, c[1]); g.lineTo(c[0] + 8, c[1]); g.moveTo(c[0], c[1] - 8); g.lineTo(c[0], c[1] + 8); g.stroke();
        }
        g.restore();
      }
    }
    if (isBrush(app.tool) && pointer) {
      const b = o[BRUSHES[app.tool]];
      const c = v.toScreen(pointer), r = Math.max(1.5, (b.size / 2) * v.k);
      g.save();
      g.lineWidth = 1.5; g.strokeStyle = 'rgba(0,0,0,.55)';
      g.beginPath(); g.arc(c[0], c[1], r, 0, Math.PI * 2); g.stroke();
      g.lineWidth = 1; g.strokeStyle = '#fff';
      g.beginPath(); g.arc(c[0], c[1], r, 0, Math.PI * 2); g.stroke();
      if (b.hardness < 0.95) { g.setLineDash([2, 3]); g.beginPath(); g.arc(c[0], c[1], Math.max(1, r * b.hardness), 0, Math.PI * 2); g.stroke(); }
      g.restore();
    }
  }

  // ----- Selection commands -----

  const selCall = (name, args = {}) => {
    const l = layer();
    if (!l) return;
    try { call(name, { id: l.id, ...args }); } catch (err) { app.hint?.(String(err.message || err)); }
    selChanged();
  };
  const hasSel = () => { const i = info(); return !!i?.selection; };
  const sizeAsk = async (title, label, value) => {
    const v = await app.askNumber?.(title, label, value);
    return v == null ? null : Number(v);
  };

  /// The Selection menu's items (also the toolbar's Selection button): [label, enabled, run, keys].
  function selectionItems() {
    const l = layer();
    const s = !!l && hasSel();
    return [
      ['Select all pixels', !!l, () => selCall('raster_select_all'), 'Ctrl+A'],
      ['Select none', s, () => selCall('raster_select_none'), 'Esc'],
      ['Invert the selection', !!l, () => selCall('raster_invert'), 'Ctrl+Shift+I'],
      ['Select with AI…', !!l, () => app.rasterHooks?.aiSelect()],
      [quick ? 'Stop painting the selection' : 'Paint the selection', !!l, () => setQuick(!quick), 'Q'],
      '-',
      ['Grow…', s, async () => { const v = await sizeAsk('Grow the selection', 'By (pixels)', 4); if (v) selCall('raster_grow', { by: Math.abs(v) }); }],
      ['Shrink…', s, async () => { const v = await sizeAsk('Shrink the selection', 'By (pixels)', 4); if (v) selCall('raster_grow', { by: -Math.abs(v) }); }],
      ['Soften the edges…', s, async () => { const v = await sizeAsk('Soften the selection’s edges', 'By (pixels)', 6); if (v) selCall('raster_feather', { by: Math.abs(v) }); }],
      '-',
      ['Delete the selected pixels', s, () => { if (app.run('raster_clear', { id: l.id }) !== null) vp().redraw(); }, 'Delete'],
      ['Fill with the colour', s, () => app.run('raster_fill_selection', { id: l.id, color: app.fill })],
      ['Adjust colours…', !!l, () => app.rasterHooks?.adjust()],
      '-',
      ['Copy pixels', s, () => copyPixels(false), 'Ctrl+C'],
      ['Cut pixels', s, () => copyPixels(true), 'Ctrl+X'],
      ['Paste as a new layer', !!pixelClip, () => pastePixels(null), 'Ctrl+V'],
      ['Copy to a new layer', s, () => { if (app.run('raster_layer_from_selection', { id: l.id }) != null) app.toast?.('Copied the selected pixels to a new layer above.'); }],
      ['Cut to a new layer', s, () => { if (app.run('raster_layer_from_selection', { id: l.id, cut: true }) != null) app.toast?.('Moved the selected pixels to a new layer above.'); }],
    ];
  }

  /// Keys while a raster tool is out: true when one was used.
  function key(e) {
    const l = layer();
    if (!l || !app.tool?.startsWith('r-')) return false;
    const ctrl = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (ctrl && !e.shiftKey && k === 'a') { selCall('raster_select_all'); return true; }
    if (ctrl && e.shiftKey && k === 'i') { selCall('raster_invert'); return true; }
    if (!ctrl && (e.key === 'Delete' || e.key === 'Backspace') && hasSel()) { app.run('raster_clear', { id: l.id }); return true; }
    if (!ctrl && e.key === 'Escape' && drag?.float) { drag = null; call('raster_live_end', { keep: false }); selChanged(); return true; }
    if (!ctrl && e.key === 'Escape' && quick) { setQuick(false); return true; }
    if (!ctrl && e.key === 'Escape' && hasSel()) { selCall('raster_select_none'); return true; }
    if (!ctrl && !e.altKey && k === 'q') { setQuick(!quick); return true; }
    if (ctrl && !e.shiftKey && (k === 'c' || k === 'x') && hasSel()) { copyPixels(k === 'x'); return true; }
    const arrows = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    if (arrows && !ctrl && app.tool === 'r-select' && hasSel()) {
      const n = (e.shiftKey ? 10 : 1) / (info()?.px_per_unit || 1);
      nudge(l.id, arrows[0] * n, arrows[1] * n);
      return true;
    }
    // [ and ]: a smaller or bigger brush.
    if (!ctrl && (e.key === '[' || e.key === ']') && isBrush(app.tool)) {
      const b = o[BRUSHES[app.tool]];
      b.size = Math.max(1, Math.min(1000, Math.round(b.size * (e.key === ']' ? 1.2 : 1 / 1.2))));
      keep(); app.renderToolbar?.(); vp().redraw();
      return true;
    }
    return false;
  }

  // ----- The toolbar -----

  /// The options for the raster tool that's out, built with the toolbar's own controls.
  /// The Brush menu: ready-made and saved brushes, saving the settings as one, and the brush's switches.
  function brushMenu(t, b, menu, ask) {
    const toggle = (k) => () => { b[k] = !b[k]; keep(); app.renderToolbar?.(); };
    const same = (p) => PRESET_KEYS.every((k) => (typeof p[k] === 'number' ? Math.abs(p[k] - b[k]) < 1e-6 : p[k] === b[k]));
    const use = (p) => () => { for (const k of PRESET_KEYS) b[k] = p[k]; keep(); app.renderToolbar?.(); vp().redraw(); };
    const save = async () => {
      const name = await ask('Save the brush', 'Name', `My brush ${o.presets.length + 1}`);
      if (!name) return;
      o.presets = [...o.presets.filter((p) => p.name !== name), { name, ...Object.fromEntries(PRESET_KEYS.map((k) => [k, b[k]])) }];
      keep(); app.renderToolbar?.();
      app.toast?.(`Saved “${name}”. It’s in the Brush menu of every painting tool.`);
    };
    const tipOf = (p) => `${p.size} px · hardness ${Math.round(p.hardness * 100)}% · ${t === 'r-smudge' ? 'strength' : 'opacity'} ${Math.round(p.opacity * 100)}%${p.pressure ? ' · pen pressure' : ''}`;
    return menu('Brush', [
      '#Ready-made',
      ...PRESETS.map((p) => [p.name, same(p), use(p), tipOf(p)]),
      ...(o.presets.length ? ['#Yours', ...o.presets.map((p) => [p.name, same(p), use(p), tipOf(p)])] : []),
      ['Save these settings as a brush…', undefined, save, 'Keep the size, hardness, opacity and pen pressure under a name'],
      ...(o.presets.length ? ['#Remove one of yours', ...o.presets.map((p) => [`Remove “${p.name}”`, undefined, () => { o.presets = o.presets.filter((x) => x !== p); keep(); app.renderToolbar?.(); }, ''])] : []),
      '-',
      ['Pen pressure', b.pressure, toggle('pressure'), 'A pen tablet’s pressure makes the line thinner when you press lightly'],
      t === 'r-brush' ? ['Keep the see-through parts', !!b.keep_alpha, toggle('keep_alpha'), 'Paint only where there are pixels already (colour in line art without going over its edge)'] : null,
      t === 'r-brush' || t === 'r-eraser' ? ['Pen pressure makes it lighter', !!b.pressure_opacity, toggle('pressure_opacity'), 'A light press paints fainter, as well as thinner'] : null,
      ...(t === 'r-brush' || t === 'r-eraser' ? ['#Steady lines', ...['Off', 'A little', 'A lot'].map((n, i) => [n, (b.steady || 0) === i, () => { b.steady = i; keep(); app.renderToolbar?.(); }, 'Smooths shaky strokes: the line follows the pointer more slowly'])] : []),
      t === 'r-brush' || t === 'r-eraser' ? '-' : null,
      t === 'r-brush' || t === 'r-eraser' ? ['Paint the selection (Q)', quick, () => setQuick(!quick), 'The brush adds to the selection and the eraser takes away, instead of painting pixels'] : null,
      t === 'r-clone' ? ['Aligned', !!b.aligned, toggle('aligned'), 'On: the copy keeps the same distance from where you paint for every stroke. Off: each stroke copies from the spot you Alt+clicked again.'] : null,
    ].filter(Boolean));
  }

  function options(h, { num, seg, check, menu, ask }) {
    const t = app.tool;
    const set = (obj, k) => (v) => { obj[k] = v; keep(); vp().redraw(); };
    const sampleSeg = (obj) => seg('Look at', obj.sample, [['layer', 'This layer', 'Only this layer’s pixels'], ['canvas', 'All layers', 'What the canvas shows (lines on another layer)']], (v) => { obj.sample = v; keep(); app.renderToolbar?.(); });
    if (isBrush(t)) {
      const b = o[BRUSHES[t]];
      return [
        t === 'r-smudge' ? seg('Smudge or blur', b.mode, [['smudge', 'Smudge', 'Drag colours along'], ['blur', 'Blur', 'Soften what you paint over']], (v) => { b.mode = v; keep(); app.renderToolbar?.(); }) : null,
        num('Size', b.size, set(b, 'size'), { min: 1, max: 1000, width: 56 }),
        num('Hardness', Math.round(b.hardness * 100), (v) => set(b, 'hardness')(v / 100), { min: 0, max: 100, width: 56 }),
        num(t === 'r-smudge' ? 'Strength' : 'Opacity', Math.round(b.opacity * 100), (v) => set(b, 'opacity')(v / 100), { min: 1, max: 100, width: 56 }),
        t === 'r-clone' ? sampleSeg(b) : null,
        brushMenu(t, b, menu, ask),
      ];
    }
    if (t === 'r-fill') {
      const f = o.fill;
      const fillMode = seg('Fill with', f.mode, [['bucket', 'Bucket', 'Click to fill the area of like colour'], ['gradient', 'Gradient', 'Drag to lay a gradient from the paint colour to a second one (inside the selection, if any)']], (v) => { f.mode = v; keep(); app.renderToolbar?.(); });
      if (f.mode === 'gradient') {
        return [
          fillMode,
          seg('Shape', f.gradient, [['linear', 'Linear', 'Straight bands along the drag'], ['radial', 'Radial', 'Rings out from where the drag starts']], (v) => { f.gradient = v; keep(); app.renderToolbar?.(); }),
          h('label', { class: 'opt', title: 'The colour the gradient ends in' }, h('span', {}, 'To'),
            h('input', { type: 'color', class: 'color-in', 'aria-label': 'The gradient’s second colour', value: f.color2 === 'none' ? '#ffffff' : f.color2, disabled: f.color2 === 'none', onChange: (e) => { f.color2 = e.target.value; keep(); } })),
          check('See-through', f.color2 === 'none', (v) => { f.color2 = v ? 'none' : '#ffffff'; keep(); app.renderToolbar?.(); }, 'The gradient fades out to see-through instead of a second colour'),
        ];
      }
      return [
        fillMode,
        num('Tolerance', f.tolerance, set(f, 'tolerance'), { min: 0, max: 100, width: 56 }),
        num('Grow', f.grow, set(f, 'grow'), { min: 0, max: 20, width: 52 }),
        check('Joined only', f.contiguous, set(f, 'contiguous'), 'Fill only the area joined to where you click (off: every pixel of that colour)'),
        sampleSeg(f),
      ];
    }
    if (t === 'r-wand') {
      const w = o.wand;
      return [
        num('Tolerance', w.tolerance, set(w, 'tolerance'), { min: 0, max: 100, width: 56 }),
        check('Joined only', w.contiguous, set(w, 'contiguous'), 'Select only the area joined to where you click (off: every pixel of that colour)'),
        sampleSeg(w),
      ];
    }
    if (t === 'r-select') {
      return [seg('Shape', o.select.shape, SELECT_SHAPES.map((x) => [x.id, x.label.split(' ')[0], x.label]), (v) => { o.select.shape = v; keep(); app.renderToolbar?.(); })];
    }
    return [];
  }

  /// The raster tool buttons (the select tool's shape shows on its button).
  function toolButtons(h, keys) {
    return RASTER_TOOLS.map((t) => {
      const k = keys[t.id] || t.key;
      const ic = t.id === 'r-select' ? SELECT_SHAPES.find((x) => x.id === o.select.shape)?.icon || 'marquee' : t.icon;
      return h('button', { class: 'icon-btn', 'aria-label': k ? `${t.label} (${k})` : t.label, title: k ? `${t.label} (${k})` : t.label, 'aria-pressed': String(app.tool === t.id), html: icon(ic), onClick: () => app.setTool(t.id) });
    });
  }

  /// Status-bar hint for a raster tool.
  function hint(tool) {
    return {
      'r-brush': 'Drag to paint · [ ] size · Shift+click: straight line · X: swap colours',
      'r-eraser': 'Drag to rub out · [ ] size',
      'r-smudge': o.smudge.mode === 'blur' ? 'Drag to soften · [ ] size' : 'Drag to smear colours along · [ ] size · Strength: how far they go',
      'r-clone': clone.source ? 'Drag to paint the copy · Alt+click: copy from somewhere else · [ ] size' : 'Alt+click where to copy from, then drag where it goes',
      'r-fill': o.fill.mode === 'gradient' ? 'Drag from where the gradient starts to where it ends · inside the selection, if any' : 'Click to fill · Tolerance: how alike colours must be · Grow: fill under the edges of lines',
      'r-wand': 'Click to select like colours · Shift: add · Alt: take away · Delete: delete them',
      'r-select': 'Drag to select · Shift: add · Alt: take away · drag inside: move the pixels · corners: scale · arrows: nudge · Esc: select none',
      'r-pick': 'Click to take a colour from the picture',
    }[tool];
  }

  return { down, move, up, overlay, key, options, toolButtons, hint, selectionItems, layer, info, sample, tickAnts, isPainting: () => !!drag?.paint, opts: o,
    copyPixels, pastePixels, quick: () => quick, setQuick };
}
