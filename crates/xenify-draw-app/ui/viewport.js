// The canvas: the engine draws the artwork (WebGL2) and this draws what
// goes over it (selection, handles, what's being drawn, spelling and
// grammar underlines) on a 2D canvas on top. Pan with the Hand tool, the
// space bar or the wheel; zoom with Ctrl and the wheel.

import { call, render } from './engine.js';
import { PointEditor } from './points.js';
import { pumpFrames } from './video.js';

const ACCENT = '#C2410C';
const HANDLE = 8;
const ROTATE_GAP = 28;

const mul = (a, b) => [
  a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
  a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
  a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
];
const translate = (x, y) => [1, 0, 0, 1, x, y];
const scaleAbout = (cx, cy, sx, sy) => mul(translate(cx, cy), mul([sx, 0, 0, sy, 0, 0], translate(-cx, -cy)));
const rotateAbout = (cx, cy, a) => mul(translate(cx, cy), mul([Math.cos(a), Math.sin(a), -Math.sin(a), Math.cos(a), 0, 0], translate(-cx, -cy)));

export class Viewport {
  constructor({ stage, gl, overlay, textEdit, perf, app }) {
    Object.assign(this, { stage, gl, overlay, textEdit, perf, app });
    this.ctx = overlay.getContext('2d');
    this.k = 0.5;
    this.ox = 0;
    this.oy = 0;
    this.dirty = true;
    this.drag = null;
    this.hover = null;
    this.pen = null;
    this.marks = [];
    this.space = false;
    this.stats = { frames: [], last: null };
    this.editing = null;
    this.points = new PointEditor(this);
    this.maskPoints = new PointEditor(this, { masks: true });
    this.wire();
    // The stage changing size (a panel opened, a tablet turned): a canvas
    // that was fitted is fitted again; otherwise it keeps its middle.
    new ResizeObserver(() => {
      const W = stage.clientWidth, H = stage.clientHeight, [W0, H0] = this.size || [W, H];
      this.size = [W, H];
      if (this.fitted && this.fitted.k === this.k && this.fitted.ox === this.ox && this.fitted.oy === this.oy) this.fit();
      else if (this.k) { this.ox += (W - W0) / 2; this.oy += (H - H0) / 2; }
      this.redraw();
    }).observe(stage);
    const loop = (t) => {
      if ((this.dirty || this.app.playing) && !this.app.presenting) this.frame(t);
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  redraw() {
    this.dirty = true;
  }

  /// Draws the canvas straight away: after something else drew on the same renderer (thumbnails,
  /// previews), so the page never shows the canvas with that in it.
  drawNow() {
    if (this.app.presenting) return;
    this.frame(performance.now());
  }

  // ----- Coordinates -----

  /// Composition → screen (CSS px inside the stage).
  get screen() {
    return [this.k, 0, 0, this.k, this.ox, this.oy];
  }

  toComp(e) {
    const r = this.stage.getBoundingClientRect();
    return [(e.clientX - r.left - this.ox) / this.k, (e.clientY - r.top - this.oy) / this.k];
  }

  /// How many clicks in a row this press is. Browsers leave a pointerdown's count at 0
  /// (only some webviews fill it in), so quick presses at about the same spot are counted here.
  clickCount(e) {
    const l = this.lastClick || { t: 0, x: 0, y: 0, n: 0 };
    const near = Math.abs(e.clientX - l.x) < 5 && Math.abs(e.clientY - l.y) < 5;
    const n = e.detail > 0 ? e.detail : (near && e.timeStamp - l.t < 500 ? l.n + 1 : 1);
    this.lastClick = { t: e.timeStamp, x: e.clientX, y: e.clientY, n };
    return n;
  }

  toScreen([x, y]) {
    return [x * this.k + this.ox, y * this.k + this.oy];
  }

  fit() {
    const c = this.app.state?.comp;
    if (!c) return;
    const r = this.stage.getBoundingClientRect();
    const pad = 40;
    this.k = Math.max(0.02, Math.min((r.width - pad * 2) / c.width, (r.height - pad * 2) / c.height, 4));
    this.ox = (r.width - c.width * this.k) / 2;
    this.oy = (r.height - c.height * this.k) / 2;
    this.fitted = { k: this.k, ox: this.ox, oy: this.oy };
    this.app.onZoom?.();
    this.redraw();
  }

  /// Some of the canvas always stays on screen, so it can't be lost by panning.
  keepInView() {
    const c = this.app.state?.comp;
    if (!c) return;
    const W = this.stage.clientWidth, H = this.stage.clientHeight;
    const keep = Math.min(120, W / 3, H / 3);
    const [cw, ch] = [c.width * this.k, c.height * this.k];
    this.ox = Math.min(W - Math.min(keep, cw), Math.max(Math.min(keep, cw) - cw, this.ox));
    this.oy = Math.min(H - Math.min(keep, ch), Math.max(Math.min(keep, ch) - ch, this.oy));
  }

  zoomTo(k, at) {
    const r = this.stage.getBoundingClientRect();
    const [sx, sy] = at || [r.width / 2, r.height / 2];
    const nk = Math.max(0.02, Math.min(k, 64));
    this.ox = sx - ((sx - this.ox) / this.k) * nk;
    this.oy = sy - ((sy - this.oy) / this.k) * nk;
    this.k = nk;
    this.keepInView();
    this.app.onZoom?.();
    this.redraw();
  }

  // ----- Drawing -----

  frame(t) {
    this.dirty = false;
    const r = this.stage.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(r.width * dpr));
    const h = Math.max(1, Math.round(r.height * dpr));
    for (const c of [this.gl, this.overlay]) {
      if (c.width !== w || c.height !== h) {
        c.width = w;
        c.height = h;
      }
    }
    if (this.app.playing) this.app.advance(t);
    const ground = getComputedStyle(document.documentElement).getPropertyValue('--ground-2').trim() || '#E6E2DD';
    const t0 = performance.now();
    const view = [this.k * dpr, 0, 0, this.k * dpr, this.ox * dpr, this.oy * dpr];
    let times;
    try {
      times = render(view, w, h, ground);
    } catch (e) {
      this.app.problem?.(e);
      return;
    }
    const total = performance.now() - t0;
    this.track(t, total, times);
    // Video frames the drawing needed and didn't have yet: fetch them, then draw again.
    pumpFrames().then((n) => { if (n) this.redraw(); }).catch(() => {});
    this.drawOverlay(dpr);
  }

  track(t, total, times) {
    const s = this.stats;
    s.frames.push({ t, total, ...times });
    while (s.frames.length > 60) s.frames.shift();
    if (this.perf.hidden) return;
    const f = s.frames;
    const avg = (k) => f.reduce((n, x) => n + x[k], 0) / f.length;
    const span = f.length > 1 ? (f[f.length - 1].t - f[0].t) / (f.length - 1) : 0;
    const fps = this.app.playing && span > 0 ? Math.round(1000 / span) : null;
    const worst = Math.max(...f.map((x) => x.total));
    this.perf.textContent =
      `${this.app.renderer}\n` +
      `frame   ${avg('total').toFixed(1)} ms  (worst ${worst.toFixed(1)})\n` +
      `encode  ${avg('encode').toFixed(1)} ms\n` +
      `GPU     ${avg('gpu').toFixed(1)} ms\n` +
      `speed   ${fps == null ? 'press play' : `${fps} fps`}\n` +
      `layers  ${this.app.state?.layers?.length ?? 0}  ·  ${this.gl.width}×${this.gl.height} px`;
  }

  drawOverlay(dpr) {
    const g = this.ctx;
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.clearRect(0, 0, this.overlay.width, this.overlay.height);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.drawGuides(g);
    this.drawCameras(g);
    this.drawBones(g);
    // Spelling and grammar underlines (squiggles under the words).
    for (const m of this.marks) {
      for (const quad of m.quads) this.squiggle(g, quad, m.color);
    }
    // Others editing the same server canvas: the layers they have selected,
    // in their colour with their name (under the text caret).
    if (!this.app.playing) {
      for (const o of this.app.others || []) {
        if (o.rev !== this.app.rev) {
          try { o.boxes = o.ids.length ? call('bounds', { ids: o.ids }).filter((b) => b?.corners?.length) : []; } catch { o.boxes = []; }
          o.rev = this.app.rev;
        }
        for (const b of o.boxes) this.quad(g, b.corners, o.colour, 1.5);
        if (!o.boxes.length) continue;
        const top = o.boxes[0].corners.map((p) => this.toScreen(p)).reduce((a, p) => (p[1] < a[1] ? p : a));
        g.font = '600 11px Figtree, sans-serif';
        g.fillStyle = o.colour;
        g.fillRect(top[0], top[1] - 18, g.measureText(o.name).width + 10, 16);
        g.fillStyle = '#fff';
        g.fillText(o.name, top[0] + 5, top[1] - 6);
      }
    }
    if (this.editing) this.drawTyping(g);
    if (this.app.playing) return;
    // Hovered layer.
    if (this.hover && this.app.tool !== 'select') this.hover = null;
    if (this.hover && this.hover.rev !== this.app.rev) this.hover = { ...(call('bounds', { ids: [this.hover.id] })[0] || { corners: [] }), id: this.hover.id, rev: this.app.rev };
    if (this.hover && !this.drag && !this.app.state?.selection.includes(this.hover.id)) {
      this.quad(g, this.hover.corners, ACCENT, 1, [4, 3]);
    }
    // Selection.
    if (this.app.tool === 'crop') {
      this.drawCrop(g);
      return;
    }
    if (this.app.tool === 'mask') {
      for (const b of this.selectionBoxes()) this.quad(g, b.corners, ACCENT, 1, [3, 3]);
      this.maskPoints.draw(g);
      return;
    }
    if (this.app.tool === 'node') {
      this.points.draw(g);
      this.drawTool(g);
      return;
    }
    const boxes = this.selectionBoxes();
    for (const b of boxes) this.quad(g, b.corners, ACCENT, 1.25);
    const u = this.union(boxes);
    const tool = this.app.tool;
    if (u && (tool === 'select' || tool === 'xform' || tool === 'perspective') && !this.editing) {
      const pts = this.handles(u);
      g.strokeStyle = ACCENT;
      g.lineWidth = 1.25;
      if (tool === 'perspective') {
        const q = this.drag?.quad || pts.corners;
        g.beginPath();
        q.forEach((c, i) => (i ? g.lineTo(c[0], c[1]) : g.moveTo(c[0], c[1])));
        g.closePath();
        g.stroke();
        for (const c of q) { g.beginPath(); g.arc(c[0], c[1], 6, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.stroke(); }
      } else {
        g.beginPath();
        g.moveTo(pts.top[0], pts.top[1]);
        g.lineTo(pts.rot[0], pts.rot[1]);
        g.stroke();
        for (const p of [...pts.corners, ...pts.edges]) {
          g.fillStyle = '#fff';
          g.fillRect(p[0] - HANDLE / 2, p[1] - HANDLE / 2, HANDLE, HANDLE);
          g.strokeRect(p[0] - HANDLE / 2, p[1] - HANDLE / 2, HANDLE, HANDLE);
        }
        g.beginPath();
        g.arc(pts.rot[0], pts.rot[1], 5, 0, Math.PI * 2);
        g.fillStyle = ACCENT;
        g.fill();
        if (tool === 'xform') {
          const c = this.toScreen(this.pivotFor(u));
          g.beginPath(); g.arc(c[0], c[1], 7, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.stroke();
          g.beginPath(); g.moveTo(c[0] - 11, c[1]); g.lineTo(c[0] + 11, c[1]); g.moveTo(c[0], c[1] - 11); g.lineTo(c[0], c[1] + 11); g.stroke();
        }
      }
    }
    // A selected connector: its two ends, to drag onto another shape.
    if (tool === 'select' && this.oneInfo()?.connector) {
      const ends = this.connectorEnds();
      if (ends) {
        ends.forEach((p0, i) => {
          const q = this.drag?.connEnd && (i === 0) === (this.drag.connEnd === 'from') ? this.drag.at : p0;
          const sp = this.toScreen(q);
          g.beginPath(); g.arc(sp[0], sp[1], 6, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.strokeStyle = ACCENT; g.lineWidth = 2; g.stroke();
        });
      }
    }
    if (tool === 'select') this.drawGenerators(g);
    // Ports of the shape a connector end would glue to, the chosen one filled.
    const gl = this.glue || (tool === 'connector' && !this.drag ? this.hoverGlue : null);
    if (gl?.ports?.length) {
      gl.ports.forEach((q, i) => { const sq = this.toScreen(q); g.beginPath(); g.arc(sq[0], sq[1], i === gl.port ? 5.5 : 4, 0, Math.PI * 2); g.fillStyle = i === gl.port ? ACCENT : '#fff'; g.fill(); g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.stroke(); });
    }
    // A spot on the edge it would glue to.
    if (gl?.spot) {
      const sq = this.toScreen(gl.at);
      g.beginPath(); g.arc(sq[0], sq[1], 5, 0, Math.PI * 2); g.fillStyle = ACCENT; g.fill(); g.strokeStyle = '#fff'; g.lineWidth = 1.5; g.stroke();
    }
    const mh = tool === 'select' ? this.modifierHandle() : null;
    if (mh) {
      const c = this.toScreen(mh.at);
      if (mh.axis != null) {
        const a = (mh.axis * Math.PI) / 180, far = 4000;
        g.setLineDash([6, 4]); g.strokeStyle = ACCENT; g.lineWidth = 1;
        g.beginPath(); g.moveTo(c[0] - Math.cos(a) * far, c[1] - Math.sin(a) * far); g.lineTo(c[0] + Math.cos(a) * far, c[1] + Math.sin(a) * far); g.stroke(); g.setLineDash([]);
      }
      const r = HANDLE * 0.9;
      g.beginPath(); g.moveTo(c[0], c[1] - r); g.lineTo(c[0] + r, c[1]); g.lineTo(c[0], c[1] + r); g.lineTo(c[0] - r, c[1]); g.closePath();
      g.fillStyle = ACCENT; g.fill(); g.strokeStyle = '#fff'; g.lineWidth = 1.5; g.stroke();
    }
    if (tool === 'select' && !this.drag) {
      for (const hd of this.segmentHandles()) { const q = this.toScreen(hd.at); g.fillStyle = '#fff'; g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.fillRect(q[0] - 4, q[1] - 4, 8, 8); g.strokeRect(q[0] - 4, q[1] - 4, 8, 8); }
    }
    if (tool === 'select' && !this.drag) this.drawQuickArrows(g);
    if (tool === 'text') this.drawTextBox(g);
    // The gradient tool: the selected shape's gradient, its two ends to drag.
    if (tool === 'gradient' && !this.drag) {
      const ends = this.gradientEnds(false);
      if (ends) {
        const [a, b] = [this.toScreen(ends.a), this.toScreen(ends.b)];
        g.strokeStyle = '#fff'; g.lineWidth = 3; g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
        g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
        for (const [q, filled] of [[a, false], [b, true]]) { g.beginPath(); g.arc(q[0], q[1], 6, 0, Math.PI * 2); g.fillStyle = filled ? ACCENT : '#fff'; g.fill(); g.strokeStyle = ACCENT; g.lineWidth = 2; g.stroke(); }
      }
    }
    if (this.drag?.zoomBox) {
      const [a, b] = [this.drag.zoomBox, this.drag.to];
      g.setLineDash([4, 3]); g.strokeStyle = ACCENT; g.lineWidth = 1;
      g.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.setLineDash([]);
    }
    if (this.drag?.marquee) {
      const [a, b] = [this.drag.marquee.from, this.drag.marquee.to];
      g.setLineDash([4, 3]);
      g.strokeStyle = ACCENT;
      g.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.setLineDash([]);
    }
    if (tool === 'sculpt' && this.cursor) {
      const c = this.toScreen(this.cursor), r = this.app.sculpt.size * this.k;
      g.lineWidth = 1.5; g.strokeStyle = 'rgba(0,0,0,.5)';
      g.beginPath(); g.arc(c[0], c[1], r, 0, Math.PI * 2); g.stroke();
      g.lineWidth = 1; g.strokeStyle = '#fff';
      g.beginPath(); g.arc(c[0], c[1], r, 0, Math.PI * 2); g.stroke();
      if (this.app.sculpt.mode !== 'push') { g.beginPath(); g.arc(c[0], c[1], r / 2, 0, Math.PI * 2); g.stroke(); }
    }
    this.app.raster?.overlay(g);
    this.drawTool(g);
    // Sizes, distances and angles by the pointer while dragging.
    if (this.readout && this.drag) {
      const r0 = this.stage.getBoundingClientRect();
      const x = this.readout.at.clientX - r0.left + 16, y = this.readout.at.clientY - r0.top + 22;
      g.font = '600 12px Figtree, Inter, sans-serif';
      const w = g.measureText(this.readout.text).width + 14;
      g.fillStyle = 'rgba(28, 25, 23, .86)';
      g.beginPath(); g.roundRect(x, y - 15, w, 22, 6); g.fill();
      g.fillStyle = '#fff'; g.fillText(this.readout.text, x + 7, y);
    }
  }

  /// The grid, the guides, the lines a move snapped to, and the rulers.
  /// Every camera's frame (what it sees), dashed, with its name; a note while looking through one.
  cameraFrames() {
    if (!this.app.state?.layers.some((l) => l.kind === 'camera')) return [];
    const key = `${this.app.rev}:${this.app.state.time}`;
    if (this.camCache?.key !== key) this.camCache = { key, list: call('camera_frames') };
    return this.camCache.list;
  }
  drawCameras(g) {
    const looking = this.app.state?.layers.some((l) => l.kind === 'camera') && call('look_through').on;
    let note = document.getElementById('camera-note');
    if (looking && !note) {
      note = document.createElement('div');
      note.id = 'camera-note';
      note.className = 'camera-note';
      note.textContent = 'Looking through the camera. Turn off View › Look through the camera to edit.';
      this.stage.append(note);
    }
    if (note) note.hidden = !looking;
    if (looking || this.app.playing) return;
    for (const f of this.cameraFrames()) {
      if (!f.visible) continue;
      const on = this.app.state.selection.includes(f.id);
      const colour = on ? ACCENT : f.active ? '#0EA5E9' : '#8A8F98';
      this.quad(g, f.corners, colour, 1.5, [8, 5]);
      const at = this.toScreen(f.corners[0]);
      g.font = '12px Inter, system-ui, sans-serif';
      g.fillStyle = colour;
      g.fillText(`${f.name}${f.active ? ' (active)' : ''}`, at[0] + 5, at[1] + 15);
    }
  }
  /// The camera whose frame edge is under canvas point `p`.
  cameraAt(p) {
    const tol = 6 / this.k;
    const near = (a, b) => {
      const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
      const t = Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)));
      return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy) <= tol;
    };
    return [...this.cameraFrames()].reverse().find((f) => f.visible && !f.locked && f.corners.some((c, i) => near(c, f.corners[(i + 1) % 4]))) || null;
  }

  drawGuides(g) {
    const pr = this.app.prefs;
    const c = this.app.state?.comp;
    if (!c) return;
    const [a, b] = [this.toScreen([0, 0]), this.toScreen([c.width, c.height])];
    if (pr.showGrid) {
      let gd = this.gridCache?.rev === this.app.rev ? this.gridCache.g : (this.gridCache = { rev: this.app.rev, g: call('guides') }).g;
      let step = gd.grid;
      while (step * this.k < 8) step *= 2;
      g.strokeStyle = 'rgba(37, 99, 235, 0.16)';
      g.lineWidth = 1;
      g.beginPath();
      for (let x = 0; x <= c.width + 1e-6; x += step) { const sx = Math.round(this.toScreen([x, 0])[0]) + 0.5; g.moveTo(sx, a[1]); g.lineTo(sx, b[1]); }
      for (let y = 0; y <= c.height + 1e-6; y += step) { const sy = Math.round(this.toScreen([0, y])[1]) + 0.5; g.moveTo(a[0], sy); g.lineTo(b[0], sy); }
      g.stroke();
    }
    const W = this.overlay.width / (window.devicePixelRatio || 1), H = this.overlay.height / (window.devicePixelRatio || 1);
    if (pr.showGuides) {
      const gd = this.gridCache?.rev === this.app.rev ? this.gridCache.g : (this.gridCache = { rev: this.app.rev, g: call('guides') }).g;
      g.strokeStyle = '#06B6D4';
      g.lineWidth = 1;
      g.beginPath();
      for (const gl of gd.guides) {
        if (gl.vertical) { const x = Math.round(this.toScreen([gl.pos, 0])[0]) + 0.5; g.moveTo(x, 0); g.lineTo(x, H); }
        else { const y = Math.round(this.toScreen([0, gl.pos])[1]) + 0.5; g.moveTo(0, y); g.lineTo(W, y); }
      }
      g.stroke();
    }
    if (this.newGuide) {
      g.strokeStyle = '#06B6D4'; g.setLineDash([4, 3]); g.beginPath();
      if (this.newGuide.vertical) { const x = this.toScreen([this.newGuide.pos, 0])[0]; g.moveTo(x, 0); g.lineTo(x, H); }
      else { const y = this.toScreen([0, this.newGuide.pos])[1]; g.moveTo(0, y); g.lineTo(W, y); }
      g.stroke(); g.setLineDash([]);
    }
    for (const l of this.snapLines || []) {
      g.strokeStyle = '#DB2777'; g.lineWidth = 1; g.beginPath();
      if (l.x != null) { const x = this.toScreen([l.x, 0])[0]; g.moveTo(x, 0); g.lineTo(x, H); }
      else { const y = this.toScreen([0, l.y])[1]; g.moveTo(0, y); g.lineTo(W, y); }
      g.stroke();
    }
    // Slices (outlined; the crop tool draws them itself).
    if (pr.showSlices !== false && this.app.tool !== 'crop') {
      const sl = this.slices();
      g.font = '12px Inter, sans-serif';
      for (const x of sl) {
        const [a, b] = [this.toScreen([x.rect[0], x.rect[1]]), this.toScreen([x.rect[2], x.rect[3]])];
        g.setLineDash([5, 4]); g.strokeStyle = 'rgba(37, 99, 235, .7)'; g.lineWidth = 1;
        g.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
        g.setLineDash([]); g.fillStyle = '#2563EB'; g.fillText(x.name, a[0] + 4, a[1] - 5);
      }
    }
    if (pr.showRulers) this.drawRulers(g, W, H);
  }

  drawRulers(g, W, H) {
    const R = 20;
    const css = getComputedStyle(document.documentElement);
    g.fillStyle = css.getPropertyValue('--sheet').trim() || '#fff';
    g.fillRect(0, 0, W, R); g.fillRect(0, 0, R, H);
    g.strokeStyle = css.getPropertyValue('--line-2').trim() || '#ccc';
    g.fillStyle = css.getPropertyValue('--muted').trim() || '#777';
    g.font = '10px Inter, sans-serif';
    // Numbered in the canvas's units (mm, in…) when it has them.
    const u = this.app.units?.() || { k: 1 };
    const kk = this.k * u.k;
    // The smallest 1, 2 or 5 × 10ⁿ that keeps numbers 50 px apart.
    const raw = 50 / kk, e10 = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 5, 10].map((m) => m * e10).find((v) => v >= raw);
    const label = (v) => String(Math.round(v * 100) / 100);
    g.beginPath();
    const x0 = Math.floor(-this.ox / kk / step) * step;
    for (let x = x0; this.toScreen([x * u.k, 0])[0] < W; x += step) {
      const sx = Math.round(this.toScreen([x * u.k, 0])[0]) + 0.5;
      g.moveTo(sx, R - 8); g.lineTo(sx, R);
      g.fillText(label(x), sx + 2, 10);
    }
    const y0 = Math.floor(-this.oy / kk / step) * step;
    for (let y = y0; this.toScreen([0, y * u.k])[1] < H; y += step) {
      const sy = Math.round(this.toScreen([0, y * u.k])[1]) + 0.5;
      g.moveTo(R - 8, sy); g.lineTo(R, sy);
      g.save(); g.translate(10, sy - 2); g.rotate(-Math.PI / 2); g.fillText(label(y), 0, 0); g.restore();
    }
    g.moveTo(0, R + 0.5); g.lineTo(W, R + 0.5); g.moveTo(R + 0.5, 0); g.lineTo(R + 0.5, H);
    g.stroke();
  }

  /// Rulers and guides under the pointer: a new guide from a ruler, or a guide to move (screen point `s`).
  guideAt(s) {
    const pr = this.app.prefs;
    if (pr.showRulers && (s[0] < 20 || s[1] < 20)) return { fresh: true, vertical: s[0] < 20 && s[1] >= 20 };
    if (!pr.showGuides) return null;
    const gd = call('guides');
    const i = gd.guides.findIndex((gl) => (gl.vertical ? Math.abs(this.toScreen([gl.pos, 0])[0] - s[0]) : Math.abs(this.toScreen([0, gl.pos])[1] - s[1])) < 4);
    return i >= 0 ? { index: i, vertical: gd.guides[i].vertical } : null;
  }

  /// The canvas's slices (kept until the next change).
  slices() {
    if (this.sliceCache?.rev !== this.app.rev) {
      let list = [];
      try { list = call('slices'); } catch { /* none */ }
      this.sliceCache = { rev: this.app.rev, list };
    }
    return this.sliceCache.list;
  }

  /// Crop and slices: the slices (dashed, named), and the rectangle being set with the outside dimmed.
  drawCrop(g) {
    const slices = this.slices();
    g.font = '12px Inter, sans-serif';
    for (const sl of slices) {
      const [a, b] = [this.toScreen([sl.rect[0], sl.rect[1]]), this.toScreen([sl.rect[2], sl.rect[3]])];
      g.setLineDash([5, 4]); g.strokeStyle = '#2563EB'; g.lineWidth = 1.25;
      g.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
      g.setLineDash([]);
      g.fillStyle = '#2563EB'; g.fillText(sl.name, a[0] + 4, a[1] - 5);
    }
    const r = this.cropRect;
    if (!r) return;
    const [a, b] = [this.toScreen([r[0], r[1]]), this.toScreen([r[2], r[3]])];
    const W = this.overlay.width, H = this.overlay.height;
    g.fillStyle = 'rgba(28, 25, 23, 0.45)';
    g.beginPath(); g.rect(0, 0, W, H); g.rect(a[0], a[1], b[0] - a[0], b[1] - a[1]); g.fill('evenodd');
    g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
    g.fillStyle = '#fff';
    for (const [x, y] of [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]) { g.fillRect(x - 4, y - 4, 8, 8); g.strokeRect(x - 4, y - 4, 8, 8); }
    g.fillStyle = ACCENT;
    g.fillText(`${Math.round(r[2] - r[0])} × ${Math.round(r[3] - r[1])}`, a[0] + 4, b[1] + 16);
  }

  /// The selected text layer's box (layer space) and its layer → canvas transform, if it has one.
  textBox() {
    const sel = this.app.state?.selection || [];
    if (sel.length !== 1 || this.app.layerById(sel[0])?.kind !== 'text') return null;
    let tb;
    try { tb = call('text_box', { id: sel[0] }); } catch { return null; }
    return tb.frame ? { id: sel[0], affine: tb.affine, rect: tb.frame } : null;
  }

  /// The box's eight grips: [layer-space point, side x (-1, 0, 1), side y].
  static grips(r) {
    const [cx, cy] = [(r[0] + r[2]) / 2, (r[1] + r[3]) / 2];
    return [[[r[0], r[1]], -1, -1], [[cx, r[1]], 0, -1], [[r[2], r[1]], 1, -1], [[r[2], cy], 1, 0], [[r[2], r[3]], 1, 1], [[cx, r[3]], 0, 1], [[r[0], r[3]], -1, 1], [[r[0], cy], -1, 0]];
  }

  /// The text box grip under the pointer: { id, affine, rect, side, start, cursor }.
  textGripAt(e) {
    const tb = this.textBox();
    if (!tb) return null;
    const r0 = this.stage.getBoundingClientRect();
    const s = [e.clientX - r0.left, e.clientY - r0.top];
    for (const [pt, sx, sy] of Viewport.grips(tb.rect)) {
      const q = this.toScreen(applyAff(tb.affine, pt));
      if (Math.hypot(q[0] - s[0], q[1] - s[1]) <= HANDLE + 2) {
        const cursor = sx && sy ? (sx === sy ? 'nwse-resize' : 'nesw-resize') : sx ? 'ew-resize' : 'ns-resize';
        return { ...tb, side: [sx, sy], start: this.toComp(e), cursor };
      }
    }
    return null;
  }

  /// The Text tool's box being drawn, and the selected text box with its grips.
  drawTextBox(g) {
    const d = this.drag;
    g.strokeStyle = ACCENT;
    g.lineWidth = 1.25;
    if (d?.newText) {
      const [a, b] = [this.toScreen(d.newText), this.toScreen(d.to)];
      g.setLineDash([4, 3]);
      g.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.setLineDash([]);
      return;
    }
    const tb = this.textBox();
    if (!tb) return;
    const r = tb.rect;
    const c = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map((p) => this.toScreen(applyAff(tb.affine, p)));
    g.setLineDash([4, 3]);
    g.beginPath(); c.forEach((p, i) => (i ? g.lineTo(p[0], p[1]) : g.moveTo(p[0], p[1]))); g.closePath(); g.stroke();
    g.setLineDash([]);
    for (const [pt] of Viewport.grips(r)) {
      const q = this.toScreen(applyAff(tb.affine, pt));
      g.fillStyle = '#fff';
      g.fillRect(q[0] - HANDLE / 2, q[1] - HANDLE / 2, HANDLE, HANDLE);
      g.strokeRect(q[0] - HANDLE / 2, q[1] - HANDLE / 2, HANDLE, HANDLE);
    }
  }

  /// What's being drawn: the shape's box, the line, the pen's points and handles, the knife's cut.
  drawTool(g) {
    const d = this.drag;
    g.strokeStyle = ACCENT;
    if (d?.knife) {
      // The knife: a clear line, and where it will cut each shape it goes right through.
      const [a, b] = [this.toScreen(d.from), this.toScreen(d.to)];
      g.lineCap = 'round';
      g.strokeStyle = 'rgba(255,255,255,.9)'; g.lineWidth = 4; g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
      g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.setLineDash([6, 4]); g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke(); g.setLineDash([]);
      this.drawCuts(g, d.cuts || [], 1);
      g.lineCap = 'butt';
    } else if (this.cutFlash) {
      const age = (performance.now() - this.cutFlash.at) / 1200;
      if (age < 1) { this.drawCuts(g, this.cutFlash.cuts, 1 - age); this.redraw(); } else this.cutFlash = null;
    }
    if (d?.shape && !d.knife) {
      const [a, b] = [this.toScreen(d.from), this.toScreen(d.to)];
      g.lineWidth = 1.5;
      g.setLineDash([5, 4]);
      g.beginPath();
      if (d.shape === 'line' || d.knife) { g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); }
      else if (d.shape === 'ellipse') g.ellipse((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, Math.abs(b[0] - a[0]) / 2, Math.abs(b[1] - a[1]) / 2, 0, 0, Math.PI * 2);
      else g.rect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.stroke();
      g.setLineDash([]);
    }
    if (d?.points?.length) {
      g.lineWidth = d.brush ? Math.max(1, this.app.brushSize * this.k) : 2;
      g.lineCap = 'round';
      g.lineJoin = 'round';
      if (d.brush) g.strokeStyle = this.app.fill;
      g.beginPath();
      d.points.forEach((p, i) => { const s = this.toScreen(p); if (i) g.lineTo(s[0], s[1]); else g.moveTo(s[0], s[1]); });
      g.stroke();
      g.strokeStyle = ACCENT;
      g.lineCap = 'butt';
    }
    const pen = this.pen;
    if (pen?.nodes.length) {
      const S = (p) => this.toScreen(p);
      g.lineWidth = 2;
      g.beginPath();
      const seg = (a, b) => {
        const [p0, p3] = [S(a.p), S(b.p)];
        const c1 = S([a.p[0] + a.h[0], a.p[1] + a.h[1]]), c2 = S([b.p[0] - b.h[0], b.p[1] - b.h[1]]);
        g.bezierCurveTo(c1[0], c1[1], c2[0], c2[1], p3[0], p3[1]);
        return p0;
      };
      const first = S(pen.nodes[0].p);
      g.moveTo(first[0], first[1]);
      for (let i = 1; i < pen.nodes.length; i++) seg(pen.nodes[i - 1], pen.nodes[i]);
      if (pen.cursor && !pen.dragging) seg(pen.nodes[pen.nodes.length - 1], { p: pen.cursor, h: [0, 0] });
      g.stroke();
      g.lineWidth = 1.25;
      for (const n of pen.nodes) {
        const s = S(n.p);
        if (n.h[0] || n.h[1]) {
          const [o, i] = [S([n.p[0] + n.h[0], n.p[1] + n.h[1]]), S([n.p[0] - n.h[0], n.p[1] - n.h[1]])];
          g.beginPath(); g.moveTo(i[0], i[1]); g.lineTo(o[0], o[1]); g.stroke();
          for (const q of [o, i]) { g.beginPath(); g.arc(q[0], q[1], 3.5, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.stroke(); }
        }
        g.fillStyle = '#fff';
        g.fillRect(s[0] - 4, s[1] - 4, 8, 8);
        g.strokeRect(s[0] - 4, s[1] - 4, 8, 8);
      }
    }
  }

  /// The knife's cuts: a gap-coloured line across each shape, with a dot where it enters and leaves.
  drawCuts(g, list, alpha) {
    g.save();
    g.globalAlpha = alpha;
    g.lineCap = 'round';
    for (const { cuts } of list) {
      for (const [p, q] of cuts) {
        const [a, b] = [this.toScreen(p), this.toScreen(q)];
        g.strokeStyle = '#fff'; g.lineWidth = 5; g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
        g.strokeStyle = ACCENT; g.lineWidth = 2.5; g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
        for (const c of [a, b]) { g.beginPath(); g.arc(c[0], c[1], 4, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.lineWidth = 1.5; g.stroke(); }
      }
    }
    g.restore();
  }

  quad(g, corners, color, width, dash) {
    g.strokeStyle = color;
    g.lineWidth = width;
    g.setLineDash(dash || []);
    g.beginPath();
    corners.forEach((p, i) => { const s = this.toScreen(p); if (i) g.lineTo(s[0], s[1]); else g.moveTo(s[0], s[1]); });
    g.closePath();
    g.stroke();
    g.setLineDash([]);
  }

  /// A wavy line along the bottom edge of `quad` (corners: top-left, top-right, bottom-right, bottom-left).
  squiggle(g, quad, color) {
    const a = this.toScreen(quad[3]);
    const b = this.toScreen(quad[2]);
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (len < 2) return;
    const ux = (b[0] - a[0]) / len, uy = (b[1] - a[1]) / len;
    const amp = 1.6, wave = 4;
    g.strokeStyle = color;
    g.lineWidth = 1.5;
    g.beginPath();
    for (let s = 0; s <= len; s += 1) {
      const off = Math.sin((s / wave) * Math.PI) * amp + 2;
      const x = a[0] + ux * s - uy * off, y = a[1] + uy * s + ux * off;
      if (s) g.lineTo(x, y); else g.moveTo(x, y);
    }
    g.stroke();
  }

  selectionBoxes() {
    if (!this.app.state?.selection?.length) return [];
    if (this.boxCache?.rev === this.app.rev) return this.boxCache.boxes;
    const boxes = call('bounds');
    this.boxCache = { rev: this.app.rev, boxes };
    return boxes;
  }

  union(boxes) {
    if (!boxes.length) return null;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const b of boxes) for (const [x, y] of b.corners) { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
    return { x0, y0, x1, y1 };
  }

  /// Screen positions of the selection's corner handles and its turn handle.
  handles(u) {
    const tl = this.toScreen([u.x0, u.y0]), br = this.toScreen([u.x1, u.y1]);
    const corners = [[tl[0], tl[1]], [br[0], tl[1]], [br[0], br[1]], [tl[0], br[1]]];
    const top = [(tl[0] + br[0]) / 2, tl[1]];
    const mid = (tl[1] + br[1]) / 2;
    const edges = [top, [br[0], mid], [top[0], br[1]], [tl[0], mid]];
    return { corners, edges, top, rot: [top[0], top[1] - ROTATE_GAP] };
  }

  /// Free transform's centre: where it was put, else the box's middle.
  pivotFor(u) {
    const key = JSON.stringify(this.app.state.selection);
    if (this.pivot?.key === key) return this.pivot.at;
    return [(u.x0 + u.x1) / 2, (u.y0 + u.y1) / 2];
  }

  // ----- Pointer -----

  // ----- Fingers (tablets) -----
  // One finger works as the mouse does, whatever the tool; two pan and
  // pinch-zoom the canvas. The first finger waits a moment (or until it
  // moves) before it draws, so a two-finger gesture leaves no mark. A pen
  // never waits.

  touchDown(e) {
    if (e.pointerType !== 'touch') return false;
    this.fingers ??= new Map();
    this.fingers.set(e.pointerId, [e.clientX, e.clientY]);
    if (this.fingers.size === 1 && !this.pinch) {
      this.held = { e, timer: setTimeout(() => this.letGo(), 120) };
      try { this.overlay.setPointerCapture(e.pointerId); } catch { /* gone */ }
      return true;
    }
    if (this.fingers.size === 2) {
      if (this.held) { clearTimeout(this.held.timer); this.held = null; }
      // (Already drawing with the first finger: that ends where it is.)
      else if (this.drag) this.up(e);
      const [a, b] = [...this.fingers.values()];
      const r = this.stage.getBoundingClientRect();
      this.pinch = { mid: [(a[0] + b[0]) / 2 - r.left, (a[1] + b[1]) / 2 - r.top], dist: Math.hypot(a[0] - b[0], a[1] - b[1]) || 1, k: this.k, ox: this.ox, oy: this.oy };
    }
    return true;
  }

  /// The waiting first finger starts what the tool does.
  letGo(move) {
    const held = this.held;
    if (!held) return;
    clearTimeout(held.timer);
    this.held = null;
    this.down(held.e);
    if (move) this.move(move);
  }

  touchMove(e) {
    if (e.pointerType !== 'touch' || !this.fingers?.has(e.pointerId)) return false;
    this.fingers.set(e.pointerId, [e.clientX, e.clientY]);
    const p = this.pinch;
    if (p) {
      if (this.fingers.size < 2) return true;
      const [a, b] = [...this.fingers.values()];
      const r = this.stage.getBoundingClientRect();
      const mid = [(a[0] + b[0]) / 2 - r.left, (a[1] + b[1]) / 2 - r.top];
      const k = Math.max(0.02, Math.min(64, p.k * (Math.hypot(a[0] - b[0], a[1] - b[1]) / p.dist)));
      // The point under the fingers' middle stays under it.
      this.ox = mid[0] - ((p.mid[0] - p.ox) / p.k) * k;
      this.oy = mid[1] - ((p.mid[1] - p.oy) / p.k) * k;
      this.k = k;
      this.keepInView();
      this.app.onZoom?.();
      this.redraw();
      return true;
    }
    if (this.held) {
      const s = this.held.e;
      if (Math.hypot(e.clientX - s.clientX, e.clientY - s.clientY) > 6) this.letGo(e);
      return true;
    }
    return false;
  }

  touchUp(e) {
    if (e.pointerType !== 'touch' || !this.fingers?.has(e.pointerId)) return false;
    this.fingers.delete(e.pointerId);
    if (this.pinch) {
      if (!this.fingers.size) this.pinch = null;
      return true;
    }
    // A tap: pressed and let go before it started.
    if (this.held?.e.pointerId === e.pointerId) this.letGo();
    return false;
  }

  wire() {
    const o = this.overlay;
    // Fingers go through touchDown/Move/Up first: two of them pan and zoom.
    o.addEventListener('pointerdown', (e) => this.touchDown(e) || this.down(e));
    o.addEventListener('pointermove', (e) => this.touchMove(e) || this.move(e));
    o.addEventListener('pointerup', (e) => this.touchUp(e) || this.up(e));
    o.addEventListener('pointercancel', (e) => this.touchUp(e) || this.up(e));
    o.addEventListener('dblclick', (e) => this.double(e));
    o.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      if (this.editing) { this.typingMenu(e); return; }
      if (this.app.tool === 'zoom') { const r0 = this.stage.getBoundingClientRect(); this.zoomTo(this.k * 0.5, [e.clientX - r0.left, e.clientY - r0.top]); return; }
      if (this.app.tool !== 'node') {
        const p = this.toComp(e);
        const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
        if (hit != null && !this.app.state.selection.includes(hit)) this.app.select([hit]);
        this.app.canvasMenu?.(e, hit == null);
        return;
      }
      this.app.canvasMenu?.(e);
    });
    o.addEventListener('pointerleave', () => { if (this.hover) { this.hover = null; this.redraw(); } });
    this.stage.addEventListener('wheel', (e) => {
      e.preventDefault();
      // Wheels that count in lines or pages (many mice, WebKitGTK) are turned into pixels.
      const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? this.stage.clientHeight : 1;
      const [dx, dy] = [e.deltaX * unit, e.deltaY * unit];
      if (e.ctrlKey || e.metaKey || e.altKey) {
        // Zoom (as in the old app, Alt works too); a notch is about 20%.
        const r = this.stage.getBoundingClientRect();
        const d = Math.max(-120, Math.min(120, dy || dx));
        this.zoomTo(this.k * Math.exp(-d * 0.0018), [e.clientX - r.left, e.clientY - r.top]);
      } else {
        this.ox -= e.shiftKey ? dy || dx : dx;
        this.oy -= e.shiftKey ? 0 : dy;
        this.keepInView();
        this.redraw();
      }
    }, { passive: false });
    window.addEventListener('keydown', (e) => {
      if (e.code === 'Space' && !this.space && !isTyping(e)) { this.space = true; this.overlay.style.cursor = 'grab'; e.preventDefault(); }
      if (this.pen && e.key === 'Enter') { e.preventDefault(); this.finishPen(false); }
      if (this.pen && e.key === 'Escape') { e.preventDefault(); this.pen = null; this.redraw(); }
    });
    window.addEventListener('keyup', (e) => {
      if (e.code === 'Space') { this.space = false; this.overlay.style.cursor = this.cursorFor(this.app.tool); }
    });
    // Typing on the canvas: the words go into a hidden text field (so typing, IME, undo and the clipboard work
    // as usual) and the canvas draws them, with the caret and selection drawn over them here.
    this.textEdit.addEventListener('input', () => {
      if (!this.editing) return;
      const { id, label, edit } = this.editing;
      if (label) this.app.run('set_label', { id, text: this.textEdit.value, edit }, { quiet: true });
      else this.app.run('set_text', { id, text: this.textEdit.value });
      this.app.changed();
      this.caretMoved();
    });
    this.textEdit.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (!this.editing) return;
      if (e.key === 'Escape' || (e.key === 'Enter' && (e.ctrlKey || e.metaKey))) { e.preventDefault(); this.endEdit(); return; }
      // Up, down, Home and End go by the lines as the canvas lays them out.
      const step = e.key === 'ArrowUp' ? { lines: -1 } : e.key === 'ArrowDown' ? { lines: 1 } : !(e.ctrlKey || e.metaKey) && e.key === 'Home' ? { edge: -1 } : !(e.ctrlKey || e.metaKey) && e.key === 'End' ? { edge: 1 } : null;
      if (step) {
        e.preventDefault();
        const [a, f] = this.typedRange();
        let got;
        try { got = call('text_step', { id: this.editing.id, anchor: a, focus: f, extend: e.shiftKey, ...step }); } catch { return; }
        this.setTyped(got.anchor, got.focus);
      }
    });
    for (const ev of ['keyup', 'select', 'focus']) this.textEdit.addEventListener(ev, () => this.caretMoved());
    document.addEventListener('selectionchange', () => { if (this.editing && document.activeElement === this.textEdit) this.caretMoved(); });
    this.textEdit.addEventListener('blur', () => { if (!this.keepTyping) this.endEdit(); });
    // A click on the canvas while typing mustn't take the focus from the words.
    this.overlay.addEventListener('mousedown', (e) => { if (this.editing) e.preventDefault(); });
  }

  cursorFor(tool) {
    return { zoom: 'zoom-in', hand: 'grab', text: 'text', select: 'default', node: 'default', xform: 'default', perspective: 'default', eyedropper: 'copy', sculpt: 'none', 'r-brush': 'none', 'r-eraser': 'none', 'r-smudge': 'none', 'r-clone': 'none', mask: 'crosshair', crop: 'crosshair', orbit: 'grab', bones: 'default' }[tool] || 'crosshair';
  }

  down(e) {
    if (e.button === 1 || this.space || this.app.tool === 'hand') {
      this.drag = { pan: true, sx: e.clientX, sy: e.clientY, ox: this.ox, oy: this.oy };
      capture(this.overlay, e);
      this.overlay.style.cursor = 'grabbing';
      return;
    }
    if (e.button !== 0) return;
    // The colour picker's "Pick": this click gives the colour showing here.
    if (this.app.pickOnce) {
      const p0 = this.toComp(e);
      const sc = call('sample_color', { x: p0[0], y: p0[1], tolerance: 4 / this.k });
      const done = this.app.pickOnce;
      this.app.pickOnce = null;
      done(String(sc.visible).slice(0, 9));
      return;
    }
    if (this.editing) {
      // On the words being typed: places the caret (a double-click picks a word, a triple-click the line;
      // Shift extends), and dragging selects. Anywhere else finishes typing.
      const p0 = this.toComp(e);
      if (this.onTyped(p0)) {
        capture(this.overlay, e);
        this.textEdit.focus({ preventScroll: true });
        const unit = Math.min(3, this.clickCount(e));
        let got;
        try { got = call('text_pick', { id: this.editing.id, x: p0[0], y: p0[1], unit }); } catch { return; }
        const [a] = this.typedRange();
        if (e.shiftKey && unit === 1) this.setTyped(a, got.focus);
        else this.setTyped(got.anchor, got.focus);
        this.drag = { typing: true, anchor: e.shiftKey && unit === 1 ? a : got.anchor };
        return;
      }
      this.endEdit();
    }
    capture(this.overlay, e);
    const p = this.toComp(e);
    const tool = this.app.tool;
    const r = this.stage.getBoundingClientRect();
    const sp = [e.clientX - r.left, e.clientY - r.top];
    if (tool === 'zoom') {
      this.drag = { zoomBox: sp, to: sp, out: e.shiftKey || e.altKey || e.ctrlKey || e.metaKey };
      return;
    }
    // The raster tools (raster.js) paint and select pixels.
    if (tool.startsWith('r-')) {
      if (this.app.raster?.down(e, p, sp)) this.drag = { raster: true };
      return;
    }
    if (tool === 'node' || tool === 'mask') {
      const ed = tool === 'mask' ? this.maskPoints : this.points;
      if (tool === 'mask' && !this.app.state.selection.length) {
        const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
        if (hit != null) this.app.select([hit]);
        else { this.app.hint?.('Mask: click the layer to mask first, then drag on it to draw the mask.'); return; }
      }
      ed.down(e, p, sp);
      this.drag = { node: true, ed };
    } else if (tool === 'shape') {
      const snap = e.ctrlKey ? null : this.snapTargets([]);
      this.drag = { shape: this.app.shape, from: this.snapPoint(snap, p), to: p, snap };
    } else if (tool === 'orbit') {
      const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
      const id = hit != null && this.app.layerById(hit)?.kind === '3d' ? hit : this.app.state.selection.find((x) => this.app.layerById(x)?.kind === '3d');
      if (id != null) {
        if (!this.app.state.selection.includes(id)) this.app.select([id]);
        this.orbitDrag = (this.orbitDrag || 0) + 1;
        this.drag = { orbit: true, id, last: [e.clientX, e.clientY], n: this.orbitDrag };
      } else this.app.hint?.('Orbit turns 3D layers: drag one. Add one from Insert › 3D shape, or Extrude a shape.');
    } else if (tool === 'crop') {
      const r = this.cropRect;
      const inside = r && p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3];
      const corner = r && [[r[0], r[1], 0, 1], [r[2], r[1], 2, 1], [r[2], r[3], 2, 3], [r[0], r[3], 0, 3]].find(([x, y]) => Math.hypot(x - p[0], y - p[1]) * this.k < 9);
      if (corner) this.drag = { crop: 'corner', fixed: [r[2 - corner[2]], r[4 - corner[3]]] };
      else if (inside) this.drag = { crop: 'move', from: p, start: r.slice() };
      else this.drag = { crop: 'new', from: p };
    } else if (tool === 'gradient') {
      if (!this.app.state.selection.length) {
        const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
        if (hit != null) this.app.select([hit]);
        else { this.app.hint?.('Gradient: start on a filled shape (or select shapes first), then drag across it.'); return; }
      }
      this.gradDrag = (this.gradDrag || 0) + 1;
      // On an end of the selected shape's gradient: drag that end (the other stays).
      const ends = this.gradientEnds(e.altKey);
      const near = (q) => q && Math.hypot(q[0] - p[0], q[1] - p[1]) * this.k < 10;
      if (ends && (near(ends.a) || near(ends.b))) {
        const moveA = near(ends.a);
        this.drag = { shape: 'line', grad: true, from: moveA ? ends.b : ends.a, to: p, swap: moveA, n: this.gradDrag, stroke: e.altKey, ends: true };
        return;
      }
      this.drag = { shape: 'line', grad: true, from: p, to: p, n: this.gradDrag, stroke: e.altKey };
    } else if (tool === 'connector') {
      // Over a port: glued to that port; elsewhere on a shape: to the shape (the best side); Ctrl: loose.
      const g = this.glueAt(p, e);
      this.drag = { shape: 'line', connect: true, from: g.at, to: p, fromId: g.layer, fromPort: g.port };
    } else if (tool === 'knife') {
      this.drag = { shape: 'line', knife: true, from: p, to: p };
    } else if (tool === 'pencil' || tool === 'brush') {
      this.drag = { points: [p], brush: tool === 'brush' };
    } else if (tool === 'sculpt') {
      // It sculpts the selected shapes; starting on another shape picks that one.
      const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
      if (hit != null && !this.app.state.selection.includes(hit)) this.app.select([hit]);
      if (!this.app.state.selection.length) { this.app.hint?.('Sculpt: start on a shape (or select the shapes first), then drag over its outline.'); return; }
      this.sculptStroke = (this.sculptStroke || 0) + 1;
      this.drag = { sculpt: true, last: p, stroke: this.sculptStroke };
    } else if (tool === 'eyedropper') {
      this.app.eyedrop(p);
    } else if (tool === 'pen') {
      this.penDown(p);
    } else if (tool === 'text') {
      // A grip of the selected text box resizes it; text is edited; elsewhere a click makes point text and a drag a box.
      const grip = this.textGripAt(e);
      if (grip) {
        this.drag = { tbox: grip, n: (this.dragN = (this.dragN || 0) + 1) };
        return;
      }
      const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
      const layer = hit != null && this.app.layerById(hit);
      if (layer?.kind === 'text') {
        this.app.select([hit]);
        this.beginEdit(hit, { at: p, unit: 1 });
      } else {
        this.drag = { newText: p, to: p };
      }
    } else if (tool === 'bones') {
      this.bonesDown(e, p);
    } else {
      this.selectDown(e, p);
    }
  }

  /// The Bones tool's view of the canvas: every part's joint (where it turns), what it hangs from,
  /// and which joints to show (parts in a rig, the selected ones and the one under the pointer).
  bones() {
    let list = [];
    try { list = call('bones'); } catch { /* no canvas */ }
    const byId = new Map(list.map((b) => [b.id, b]));
    const isPart = (id) => { const l = this.app.layerById(id); return !!l && !!l.kind && l.kind !== 'group'; };
    const shown = new Set();
    const links = [];
    for (const b of list) {
      if (!isPart(b.id) || b.parent == null || !isPart(b.parent)) continue;
      const par = byId.get(b.parent);
      if (!par) continue;
      links.push([par, b]);
      shown.add(b.id); shown.add(par.id);
    }
    for (const id of this.app.state?.selection || []) if (isPart(id)) shown.add(id);
    if (this.boneHover != null && isPart(this.boneHover)) shown.add(this.boneHover);
    return { list, byId, isPart, shown, links };
  }

  /// Bones: on a joint, drag it to where the part should turn (dropped on another part, it hangs from that part);
  /// on a part, drag to turn it about its joint (what hangs from it follows); Shift: reach with the whole limb.
  bonesDown(e, p) {
    const B = this.bones();
    const near = [...B.shown].map((id) => B.byId.get(id)).filter(Boolean)
      .map((b) => ({ b, d: Math.hypot(b.at[0] - p[0], b.at[1] - p[1]) * this.k })).filter((x) => x.d < 10).sort((a, b) => a.d - b.d)[0];
    this.poseDrag = (this.poseDrag || 0) + 1;
    if (near && !e.shiftKey) {
      this.app.select([near.b.id]);
      this.drag = { joint: true, id: near.b.id, from: near.b.parent, n: this.poseDrag, moved: false };
      return;
    }
    const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k, group: false });
    if (hit == null || !B.isPart(hit)) {
      this.app.select([]);
      this.app.hint?.('Bones: drag a part to turn it about its joint (the dot). Drag the dot to the joint (a shoulder, an elbow); drop it on another part to hang it from that part.');
      return;
    }
    this.app.select([hit]);
    const b = B.byId.get(hit);
    if (!b) return;
    if (e.shiftKey) {
      if (b.parent == null || !B.isPart(b.parent)) { this.app.hint?.('Shift+drag moves a hand and bends the arm it hangs from: hang the parts from each other first (drag a part’s dot onto the part above it).'); return; }
      this.drag = { ik: true, id: hit, offset: [b.at[0] - p[0], b.at[1] - p[1]], n: this.poseDrag, chain: 0 };
      return;
    }
    const a0 = Math.atan2(p[1] - b.at[1], p[0] - b.at[0]);
    this.drag = { turn: true, id: hit, at: b.at, last: a0, sum: 0, r0: b.rotation, n: this.poseDrag };
  }

  /// The Bones tool's drawing: a bone from each joint to the joint of what hangs from it, and the joints as dots.
  drawBones(g) {
    if (this.app.tool !== 'bones' || !this.app.state) return;
    const B = this.bones();
    const sel = new Set(this.app.state.selection);
    g.save();
    for (const [par, b] of B.links) {
      const [a, c] = [this.toScreen(par.at), this.toScreen(b.at)];
      const len = Math.hypot(c[0] - a[0], c[1] - a[1]);
      if (len < 2) continue;
      const ux = (c[0] - a[0]) / len, uy = (c[1] - a[1]) / len;
      const w = Math.min(10, len * 0.12);
      const m = [a[0] + ux * len * 0.18, a[1] + uy * len * 0.18];
      g.beginPath();
      g.moveTo(a[0], a[1]);
      g.lineTo(m[0] - uy * w, m[1] + ux * w);
      g.lineTo(c[0], c[1]);
      g.lineTo(m[0] + uy * w, m[1] - ux * w);
      g.closePath();
      const on = sel.has(b.id) || sel.has(par.id);
      g.fillStyle = on ? 'rgba(194, 65, 12, .30)' : 'rgba(255, 255, 255, .45)';
      g.strokeStyle = on ? ACCENT : 'rgba(41, 37, 36, .75)';
      g.lineWidth = 1.5;
      g.fill(); g.stroke();
    }
    for (const id of B.shown) {
      const b = B.byId.get(id);
      if (!b) continue;
      const [x, y] = this.toScreen(b.at);
      g.beginPath(); g.arc(x, y, 6, 0, Math.PI * 2);
      g.fillStyle = sel.has(id) ? ACCENT : '#fff';
      g.strokeStyle = sel.has(id) ? '#fff' : 'rgba(41, 37, 36, .85)';
      g.lineWidth = 2; g.fill(); g.stroke();
    }
    // Dropping a joint: what it will hang from.
    const d = this.drag;
    if (d?.joint && d.target != null) {
      try { for (const bx of call('bounds', { ids: [d.target] })) if (bx?.corners?.length) this.quad(g, bx.corners, ACCENT, 2, [5, 4]); } catch { /* gone */ }
    }
    g.restore();
  }

  selectDown(e, p) {
    {
      const r0 = this.stage.getBoundingClientRect();
      const gHit = this.app.tool === 'select' && this.guideAt([e.clientX - r0.left, e.clientY - r0.top]);
      if (gHit) {
        this.drag = { guide: gHit };
        this.newGuide = { vertical: gHit.vertical, pos: gHit.vertical ? p[0] : p[1] };
        return;
      }
    }
    if (this.app.tool === 'select') {
      const hd = this.generatorHandle();
      const near = (q) => q && Math.hypot(q[0] - p[0], q[1] - p[1]) * this.k < 10;
      this.genDrag = (this.genDrag || 0) + 1;
      if (hd?.kind === 'emitter' && near(hd.tip)) { this.drag = { emitter: true, id: this.app.layerInfo.id, n: this.genDrag }; return; }
      if (hd?.kind === 'water' && near(hd.at)) { this.drag = { level: true, id: this.app.layerInfo.id, n: this.genDrag }; return; }
      // An Array, Twist or Mirror's origin (the diamond).
      const mh = this.modifierHandle();
      if (mh && near(mh.at)) { this.drag = { modOrigin: true, id: this.app.state.selection[0], index: mh.index, n: this.genDrag }; return; }
      const mark = (this.genCache?.marks || []).find((m) => near(m.at));
      if (mark && !this.app.state.selection.includes(mark.id)) { this.app.select([mark.id]); call('transform_begin'); this.drag = { move: true, from: p, moved: false }; return; }
    }
    if (this.app.tool === 'select') {
      const s = [e.clientX - this.stage.getBoundingClientRect().left, e.clientY - this.stage.getBoundingClientRect().top];
      const arrow = this.quickArrows()?.find((a) => Math.hypot(a.at[0] - s[0], a.at[1] - s[1]) <= 11);
      if (arrow) { this.app.run('quick_connect', { id: this.app.state.selection[0], dir: arrow.dir }); return; }
    }
    if (this.app.tool === 'select' && this.oneInfo()?.connector) {
      const ends = this.connectorEnds();
      const i = ends ? ends.findIndex((q) => Math.hypot(q[0] - p[0], q[1] - p[1]) * this.k < 9) : -1;
      if (i >= 0) { this.drag = { connEnd: i === 0 ? 'from' : 'to', id: this.app.layerInfo.id, at: p }; return; }
      const seg = this.segmentHandles().find((h) => Math.hypot(h.at[0] - p[0], h.at[1] - p[1]) * this.k < 9);
      if (seg) { this.drag = { seg: true, id: this.app.layerInfo.id, index: seg.index, route: seg.points, start: p, n: (this.segDrag = (this.segDrag || 0) + 1) }; return; }
    }
    const boxes = this.selectionBoxes();
    const u = this.union(boxes);
    const r = this.stage.getBoundingClientRect();
    const s = [e.clientX - r.left, e.clientY - r.top];
    const tool = this.app.tool;
    const near = (q, d = HANDLE) => Math.abs(s[0] - q[0]) < d && Math.abs(s[1] - q[1]) < d;
    if (u) {
      const h = this.handles(u);
      const cs = [[u.x0, u.y0], [u.x1, u.y0], [u.x1, u.y1], [u.x0, u.y1]];
      if (tool === 'perspective') {
        const corner = h.corners.findIndex((q) => near(q, 10));
        if (corner >= 0) {
          call('transform_begin');
          this.drag = { perspective: true, corner, from: cs, to: cs.map((c) => c.slice()), quad: h.corners.map((c) => c.slice()) };
          return;
        }
      }
      if (tool === 'xform') {
        const c = this.toScreen(this.pivotFor(u));
        if (near(c, 9)) { this.drag = { pivot: true }; return; }
      }
      if (tool !== 'perspective') {
        if (Math.hypot(s[0] - h.rot[0], s[1] - h.rot[1]) < 9) {
          const c = tool === 'xform' ? this.pivotFor(u) : [(u.x0 + u.x1) / 2, (u.y0 + u.y1) / 2];
          call('transform_begin');
          this.drag = { rotate: true, c, a0: Math.atan2(p[1] - c[1], p[0] - c[0]) };
          return;
        }
        // A text box's handles resize the box and the words reflow, as in most programs; Ctrl+drag scales the words too.
        const tbox = tool === 'select' && !(e.ctrlKey || e.metaKey) ? this.textBox() : null;
        const corner = h.corners.findIndex((q) => near(q));
        const edgeAt = corner < 0 ? h.edges.findIndex((q) => near(q)) : -1;
        if (tbox && (corner >= 0 || edgeAt >= 0)) {
          const side = corner >= 0 ? [[-1, -1], [1, -1], [1, 1], [-1, 1]][corner] : [[0, -1], [1, 0], [0, 1], [-1, 0]][edgeAt];
          this.drag = { tbox: { ...tbox, side, start: p }, n: (this.dragN = (this.dragN || 0) + 1) };
          return;
        }
        if (corner >= 0) {
          call('transform_begin');
          // Free transform: Shift+corner pins that corner (perspective); Alt scales about the centre.
          if (tool === 'xform' && e.shiftKey) {
            this.drag = { perspective: true, corner, from: cs, to: cs.map((c) => c.slice()), quad: h.corners.map((c) => c.slice()) };
            return;
          }
          const about = tool === 'xform' && e.altKey ? this.pivotFor(u) : cs[(corner + 2) % 4];
          this.drag = { scale: true, anchor: about, start: cs[corner] };
          return;
        }
        const edge = h.edges.findIndex((q) => near(q));
        if (edge >= 0) {
          call('transform_begin');
          const mids = [[(u.x0 + u.x1) / 2, u.y0], [u.x1, (u.y0 + u.y1) / 2], [(u.x0 + u.x1) / 2, u.y1], [u.x0, (u.y0 + u.y1) / 2]];
          const about = tool === 'xform' && e.altKey ? this.pivotFor(u) : mids[(edge + 2) % 4];
          this.drag = { scale: true, axis: edge % 2 ? 'x' : 'y', anchor: about, start: mids[edge] };
          return;
        }
      }
    }
    // A camera's frame: drag it to move the camera.
    const cam = this.cameraAt(p);
    if (cam) {
      this.app.select([cam.id]);
      this.drag = { camera: cam.id, from: p, start: call('layer', { id: cam.id }).props.position.value };
      return;
    }
    // A click picks the whole group; Ctrl+click reaches inside it.
    const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k, group: !(e.ctrlKey || e.metaKey) });
    if (hit == null) {
      // Drag on empty canvas: select what the box touches.
      this.drag = { marquee: { from: s, to: s }, add: e.shiftKey };
      return;
    }
    let sel = this.app.state.selection;
    if (e.shiftKey) sel = sel.includes(hit) ? sel.filter((x) => x !== hit) : [...sel, hit];
    else if (!sel.includes(hit)) sel = [hit];
    this.app.select(sel);
    if (!sel.includes(hit)) return;
    // The end of a joint chain: dragging bends the chain to follow.
    if (sel.length === 1 && this.app.layerInfo?.ik) {
      const b = this.union(call('bounds', { ids: [hit] }));
      const c = b ? [(b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2] : p;
      this.ikDrag = (this.ikDrag || 0) + 1;
      this.drag = { ik: true, id: hit, offset: [c[0] - p[0], c[1] - p[1]], n: this.ikDrag };
      return;
    }
    call('transform_begin');
    this.drag = { move: true, from: p, moved: false, snap: e.ctrlKey ? null : this.snapTargets() };
  }

  /// A step of a drag that may not apply here (no shape under it, no fill…): says why once, in the status bar, not as an error.
  quietRun(d, name, args) {
    try {
      call(name, args);
      this.app.light();
    } catch (e) {
      if (!d.told) { d.told = true; this.app.hint?.(String(e.message || e)); }
    }
  }

  /// Particle markers (every emitter), and the selected emitter's arrow or water's level handle.
  drawGenerators(g) {
    const S = (p) => this.toScreen(p);
    let marks = [];
    try { marks = this.genCache?.rev === this.app.rev ? this.genCache.marks : (this.genCache = { rev: this.app.rev, marks: call('emitters') }).marks; } catch { /* none */ }
    for (const m of marks) {
      const p = S(m.at);
      g.beginPath(); g.arc(p[0], p[1], 5, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.strokeStyle = ACCENT; g.lineWidth = 1.5; g.stroke();
      g.beginPath(); g.arc(p[0], p[1], 2, 0, Math.PI * 2); g.fillStyle = ACCENT; g.fill();
    }
    const h = this.generatorHandle();
    if (!h) return;
    g.strokeStyle = ACCENT; g.fillStyle = ACCENT; g.lineWidth = 2;
    if (h.kind === 'emitter') {
      const [o, t] = [S(h.origin), S(h.tip)];
      g.beginPath(); g.moveTo(o[0], o[1]); g.lineTo(t[0], t[1]); g.stroke();
      const a = Math.atan2(t[1] - o[1], t[0] - o[0]);
      g.beginPath(); g.moveTo(t[0], t[1]); g.lineTo(t[0] - 11 * Math.cos(a - 0.45), t[1] - 11 * Math.sin(a - 0.45)); g.lineTo(t[0] - 11 * Math.cos(a + 0.45), t[1] - 11 * Math.sin(a + 0.45)); g.closePath(); g.fill();
    } else if (h.kind === 'water') {
      g.setLineDash([6, 4]); g.lineWidth = 1;
      g.beginPath(); h.box.forEach((p, i) => { const q = S(p); if (i) g.lineTo(q[0], q[1]); else g.moveTo(q[0], q[1]); }); g.closePath(); g.stroke();
      g.setLineDash([]);
      const p = S(h.at);
      g.beginPath(); g.arc(p[0], p[1], 6, 0, Math.PI * 2); g.fill(); g.strokeStyle = '#fff'; g.lineWidth = 1.5; g.stroke();
      g.font = '12px Inter, sans-serif'; g.fillStyle = ACCENT;
      g.fillText(`Level ${Math.round(h.level * 100)}%`, p[0] + 10, p[1] - 10);
    }
  }

  /// Diagram canvases: four arrows around the one selected shape (screen points); a click makes a joined copy that way.
  quickArrows() {
    const info = this.oneInfo();
    if (this.app.state?.comp.kind !== 'diagram' || !info || info.connector || !['shape', 'precomp'].includes(info.kind)) return null;
    if (this.app.layerById(info.id)?.locked) return null;
    const b = call('bounds', { ids: [info.id] })[0];
    if (!b) return null;
    const pts = b.corners.map((c) => this.toScreen(c));
    const [x0, x1] = [Math.min(...pts.map((q) => q[0])), Math.max(...pts.map((q) => q[0]))];
    const [y0, y1] = [Math.min(...pts.map((q) => q[1])), Math.max(...pts.map((q) => q[1]))];
    const [cx, cy] = [(x0 + x1) / 2, (y0 + y1) / 2], OUT = 22;
    return [{ at: [cx, y0 - OUT], dir: [0, -1] }, { at: [x1 + OUT, cy], dir: [1, 0] }, { at: [cx, y1 + OUT], dir: [0, 1] }, { at: [x0 - OUT, cy], dir: [-1, 0] }];
  }

  drawQuickArrows(g) {
    const arrows = this.quickArrows();
    if (!arrows) return;
    for (const { at: p, dir: d } of arrows) {
      const on = this.pointer && Math.hypot(this.pointer[0] - p[0], this.pointer[1] - p[1]) <= 11;
      const s = on ? 9 : 7, n = [-d[1], d[0]];
      g.beginPath();
      g.moveTo(p[0] + d[0] * s, p[1] + d[1] * s);
      g.lineTo(p[0] - d[0] * s * 0.6 + n[0] * s * 0.8, p[1] - d[1] * s * 0.6 + n[1] * s * 0.8);
      g.lineTo(p[0] - d[0] * s * 0.6 - n[0] * s * 0.8, p[1] - d[1] * s * 0.6 - n[1] * s * 0.8);
      g.closePath();
      g.fillStyle = on ? ACCENT : 'rgba(194, 65, 12, .55)';
      g.fill();
      g.strokeStyle = '#fff'; g.lineWidth = 1; g.stroke();
    }
  }

  /// The selected layer's first Array/Twist/Mirror origin on the canvas ({ index, at, axis }).
  modifierHandle() {
    if (this.app.tool !== 'select' || this.app.state?.selection.length !== 1) return null;
    if (this.modCache?.rev !== this.app.rev) {
      let mh = null;
      try { mh = call('modifier_handle'); } catch { /* none */ }
      this.modCache = { rev: this.app.rev, mh };
    }
    return this.modCache.mh;
  }

  /// The selected layer's gradient ends on the canvas ({ a, b }), for the gradient tool.
  gradientEnds(stroke = false) {
    if (this.app.state?.selection.length !== 1) return null;
    const key = `${this.app.rev}:${stroke}`;
    if (this.gradCache?.key === key) return this.gradCache.ends;
    let ends = null;
    try { ends = call('gradient_ends', { stroke }); } catch { /* none */ }
    this.gradCache = { key, ends };
    return ends;
  }

  /// The selected layer's details when exactly one is selected.
  oneInfo() {
    return this.app.state?.selection.length === 1 ? this.app.layerInfo : null;
  }

  generatorHandle() {
    const info = this.oneInfo();
    if (!info || !['particles', 'water'].includes(info.kind)) return null;
    if (this.handleCache?.rev === this.app.rev && this.handleCache.id === info.id) return this.handleCache.h;
    let h = null;
    try { h = call('generator_handle', { id: info.id }); } catch { /* none */ }
    this.handleCache = { rev: this.app.rev, id: info.id, h };
    return h;
  }

  /// Where a connector end at `p` glues: { layer, port, at }. Ports first (within 9 px), then the shape under it; Ctrl: loose.
  glueAt(p, e, exclude = null) {
    if (e?.ctrlKey || e?.metaKey) return { layer: null, port: null, at: p, ports: [] };
    const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
    const cand = [...new Set([hit, this.glue?.layer, this.portLayer].filter((x) => x != null && x !== exclude))];
    let best = null;
    for (const id of cand) {
      let ports = [];
      try { ports = call('ports', { id }); } catch { /* none */ }
      ports.forEach((q, i) => { const dist = Math.hypot(q[0] - p[0], q[1] - p[1]) * this.k; if (dist <= 9 && (!best || dist < best.dist)) best = { dist, layer: id, port: i, at: q, ports }; });
    }
    if (best) { this.portLayer = best.layer; return best; }
    // On a shape's edge: glued to that very spot (it moves and turns with the shape).
    for (const id of cand) {
      let s = null;
      try { s = call('edge_spot', { id, x: p[0], y: p[1], tolerance: 8 / this.k }); } catch { /* not a shape */ }
      if (s) {
        let ports = [];
        try { ports = call('ports', { id }); } catch { /* none */ }
        this.portLayer = id;
        return { layer: id, port: { spot: s.spot }, at: s.at, ports, spot: true };
      }
    }
    if (hit != null && hit !== exclude) {
      let ports = [];
      try { ports = call('ports', { id: hit }); } catch { /* none */ }
      this.portLayer = hit;
      return { layer: hit, port: null, at: p, ports };
    }
    return { layer: null, port: null, at: p, ports: [] };
  }

  /// The selected elbow connector's middle segments (their midpoints), to drag.
  segmentHandles() {
    const id = this.oneInfo()?.connector ? this.app.layerInfo.id : null;
    if (id == null) return [];
    if (this.segCache?.rev !== this.app.rev || this.segCache.id !== id) {
      let r = null;
      try { r = call('connector_route', { id }); } catch { /* none */ }
      this.segCache = { rev: this.app.rev, id, r };
    }
    const r = this.segCache.r;
    if (!r || r.route !== 'orthogonal') return [];
    const pts = r.points, out = [];
    for (let k = 1; k < pts.length - 2; k++) out.push({ index: k, points: pts, at: [(pts[k][0] + pts[k + 1][0]) / 2, (pts[k][1] + pts[k + 1][1]) / 2] });
    return out;
  }

  connectorEnds() {
    const id = this.app.layerInfo?.id;
    if (this.connCache?.rev === this.app.rev && this.connCache.id === id) return this.connCache.ends;
    let ends = null;
    try { ends = call('connector_ends', { id }); } catch { ends = null; }
    this.connCache = { rev: this.app.rev, id, ends };
    return ends;
  }

  /// What a move can line up with (null when snapping is off).
  snapTargets(ids) {
    const pr = this.app.prefs;
    if (!pr.snapObjects && !pr.snapGuides && !pr.snapGrid) return null;
    try { return call('snap_targets', { ids, objects: pr.snapObjects, guides: pr.snapGuides && pr.showGuides, grid: pr.snapGrid, margin: pr.snapMargin }); } catch { return null; }
  }

  /// Moves `dx`, `dy` so the box's edges or middle meet a snap line within a few pixels; the lines met go to this.snapLines.
  snapDelta(t, dx, dy) {
    this.snapLines = [];
    if (!t?.box) return [dx, dy];
    const tol = 6 / this.k;
    const best = (vals, lines) => {
      let out = null;
      for (const v of vals) for (const l of lines) { const d = l - v; if (Math.abs(d) <= tol && (!out || Math.abs(d) < Math.abs(out.d))) out = { d, l }; }
      return out;
    };
    const [x0, y0, x1, y1] = t.box;
    const sx = best([x0 + dx, (x0 + x1) / 2 + dx, x1 + dx], t.xs);
    const sy = best([y0 + dy, (y0 + y1) / 2 + dy, y1 + dy], t.ys);
    if (sx) { dx += sx.d; this.snapLines.push({ x: sx.l }); }
    if (sy) { dy += sy.d; this.snapLines.push({ y: sy.l }); }
    return [dx, dy];
  }

  /// A point snapped to the lines (drawing shapes).
  snapPoint(t, p) {
    if (!t) return p;
    const box = { box: [p[0], p[1], p[0], p[1]], xs: t.xs, ys: t.ys };
    const [dx, dy] = this.snapDelta(box, 0, 0);
    return [p[0] + dx, p[1] + dy];
  }

  move(e) {
    const d = this.drag;
    if (d?.pan) {
      this.ox = d.ox + e.clientX - d.sx;
      this.oy = d.oy + e.clientY - d.sy;
      this.keepInView();
      this.redraw();
      return;
    }
    const p = this.toComp(e);
    if (this.app.tool === 'bones' && !d) {
      let hov = null;
      try { hov = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k, group: false }); } catch { /* none */ }
      if (hov !== this.boneHover) { this.boneHover = hov; this.redraw(); }
      return;
    }
    if (this.app.tool?.startsWith('r-') && (!d || d.raster)) {
      this.app.raster?.move(e, p, !!d?.raster);
      return;
    }
    if ((this.app.tool === 'node' || this.app.tool === 'mask') && (!d || d.node)) {
      const r = this.stage.getBoundingClientRect();
      (this.app.tool === 'mask' ? this.maskPoints : this.points).move(e, p, [e.clientX - r.left, e.clientY - r.top]);
      return;
    }
    if (this.pen?.dragging) {
      const n = this.pen.nodes[this.pen.nodes.length - 1];
      n.h = [p[0] - n.p[0], p[1] - n.p[1]];
      this.redraw();
      return;
    }
    if (!d) {
      if (this.app.tool === 'connector') { const was = JSON.stringify(this.hoverGlue?.ports), wasSpot = this.hoverGlue?.spot; this.hoverGlue = this.glueAt(p, e); if (JSON.stringify(this.hoverGlue.ports) !== was || this.hoverGlue.port != null || wasSpot) this.redraw(); }
      { const r0 = this.stage.getBoundingClientRect(); const was = this.pointer; this.pointer = [e.clientX - r0.left, e.clientY - r0.top]; if (this.app.state?.comp.kind === 'diagram' && this.app.tool === 'select' && was) this.redraw(); }
      if (this.app.tool === 'text') {
        const g = this.textGripAt(e);
        this.overlay.style.cursor = g ? g.cursor : 'text';
        return;
      }
      if (this.app.tool === 'sculpt') { this.cursor = p; this.redraw(); return; }
      if (this.pen) { this.pen.cursor = p; this.redraw(); return; }
      if (this.app.tool === 'select' && !this.app.playing) {
        const r0 = this.stage.getBoundingClientRect();
        const gh = this.guideAt([e.clientX - r0.left, e.clientY - r0.top]);
        this.overlay.style.cursor = gh ? (gh.vertical ? 'ew-resize' : 'ns-resize') : 'default';
        const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k, group: !(e.ctrlKey || e.metaKey) });
        const id = hit ?? null;
        if ((this.hover?.id ?? null) !== id) {
          const b = id == null ? null : call('bounds', { ids: [id] })[0];
          this.hover = b ? { ...b, rev: this.app.rev } : null;
          this.redraw();
        }
      }
      return;
    }
    if (d.newText) {
      d.to = p;
      this.redraw();
      return;
    }
    if (d.zoomBox) {
      const r0 = this.stage.getBoundingClientRect();
      d.to = [e.clientX - r0.left, e.clientY - r0.top];
      this.redraw();
      return;
    }
    if (d.typing) {
      let got;
      try { got = call('text_pick', { id: this.editing?.id, x: p[0], y: p[1], unit: 1 }); } catch { return; }
      this.setTyped(d.anchor, got.focus);
      return;
    }
    if (d.tbox) {
      const g = d.tbox, [sx, sy] = g.side, r = g.rect.slice();
      const [lx, ly] = applyInv(g.affine, p);
      const [ox, oy] = applyInv(g.affine, g.start);
      const dx = lx - ox, dy = ly - oy;
      if (sx < 0) r[0] = Math.min(g.rect[0] + dx, r[2] - 8); else if (sx > 0) r[2] = Math.max(g.rect[2] + dx, r[0] + 8);
      if (sy < 0) r[1] = Math.min(g.rect[1] + dy, r[3] - 8); else if (sy > 0) r[3] = Math.max(g.rect[3] + dy, r[1] + 8);
      this.app.run('set_text_style', { id: g.id, rect: r, merge: `resize${d.n}` }, { quiet: true });
      return;
    }
    if (d.shape === 'line') {
      d.to = e.shiftKey ? snapAngle(d.from, p) : p;
      if (d.knife) { try { d.cuts = call('knife_preview', { a: d.from, b: d.to }); } catch { d.cuts = []; } }
      if (d.connect) this.glue = this.glueAt(p, e, d.fromId);
      if (d.grad && (d.ends || Math.hypot(d.to[0] - d.from[0], d.to[1] - d.from[1]) * this.k > 3)) {
        this.quietRun(d, 'gradient_drag', { a: d.swap ? d.to : d.from, b: d.swap ? d.from : d.to, drag: d.n, stroke: d.stroke });
      }
    } else if (d.shape) {
      // Alt: from the centre (where the drag began); Shift: square.
      d.origin ||= d.from;
      const q = d.snap && !e.ctrlKey ? this.snapPoint(d.snap, p) : p;
      d.to = e.shiftKey ? square(d.origin, q) : q;
      d.from = e.altKey ? [2 * d.origin[0] - d.to[0], 2 * d.origin[1] - d.to[1]] : d.origin;
      const u = this.app.units?.() || { k: 1, unit: 'px' };
      this.readout = { at: e, text: `${fmtNum(Math.abs(d.to[0] - d.from[0]) / u.k)} × ${fmtNum(Math.abs(d.to[1] - d.from[1]) / u.k)} ${u.unit}` };
    } else if (d.points) {
      const last = d.points[d.points.length - 1];
      if (Math.hypot(p[0] - last[0], p[1] - last[1]) * this.k > 2) d.points.push(p);
    } else if (d.move) {
      let dx = p[0] - d.from[0], dy = p[1] - d.from[1];
      if (e.shiftKey) { if (Math.abs(dx) > Math.abs(dy)) dy = 0; else dx = 0; }
      if (d.snap && !e.ctrlKey) [dx, dy] = this.snapDelta(d.snap, dx, dy); else this.snapLines = [];
      d.moved = true;
      this.app.run('transform', { affine: translate(dx, dy) }, { quiet: true });
      { const u = this.app.units?.() || { k: 1, unit: 'px' }; this.readout = { at: e, text: `Δx ${fmtNum(dx / u.k)}  Δy ${fmtNum(dy / u.k)} ${u.unit}` }; }
    } else if (d.orbit) {
      const [dx, dy] = [e.clientX - d.last[0], e.clientY - d.last[1]];
      d.last = [e.clientX, e.clientY];
      this.app.run('turn_3d', { id: d.id, dx: dx * 0.5, dy: dy * 0.5, drag: d.n }, { quiet: true });
    } else if (d.crop) {
      const snap = (v) => Math.round(v);
      if (d.crop === 'move') {
        const dx = p[0] - d.from[0], dy = p[1] - d.from[1];
        this.cropRect = [d.start[0] + dx, d.start[1] + dy, d.start[2] + dx, d.start[3] + dy].map(snap);
      } else {
        const o = d.crop === 'new' ? d.from : d.fixed;
        this.cropRect = [Math.min(o[0], p[0]), Math.min(o[1], p[1]), Math.max(o[0], p[0]), Math.max(o[1], p[1])].map(snap);
      }
      this.app.onCrop?.();
    } else if (d.turn) {
      // Turning about the joint: the angle the pointer has gone round it (any number of turns).
      const a = Math.atan2(p[1] - d.at[1], p[0] - d.at[0]);
      let da = a - d.last;
      if (da > Math.PI) da -= 2 * Math.PI;
      if (da < -Math.PI) da += 2 * Math.PI;
      d.last = a; d.sum += da;
      this.app.run('pose_turn', { id: d.id, rotation: d.r0 + (d.sum * 180) / Math.PI, drag: d.n }, { quiet: true });
    } else if (d.joint) {
      d.moved = true;
      this.app.run('move_pivot', { id: d.id, to: p, drag: d.n }, { quiet: true });
      const under = call('hit', { x: p[0], y: p[1], tolerance: 2 / this.k, group: false, skip: [d.id] });
      d.target = under != null && this.bones().isPart(under) && under !== d.from ? under : null;
      this.redraw();
    } else if (d.ik) {
      this.app.run('ik_move', { id: d.id, to: [p[0] + d.offset[0], p[1] + d.offset[1]], drag: d.n, ...(d.chain != null ? { chain: d.chain } : {}) }, { quiet: true });
    } else if (d.emitter) {
      this.app.run('drag_emitter', { id: d.id, to: p, snap: e.shiftKey, drag: d.n }, { quiet: true });
    } else if (d.level) {
      this.app.run('drag_level', { id: d.id, to: p, drag: d.n }, { quiet: true });
    } else if (d.camera) {
      this.app.run('set_prop', { id: d.camera, prop: 'position', value: [d.start[0] + p[0] - d.from[0], d.start[1] + p[1] - d.from[1]] }, { quiet: true });
    } else if (d.modOrigin) {
      this.app.run('drag_modifier_origin', { id: d.id, index: d.index, to: p, drag: d.n }, { quiet: true });
    } else if (d.connEnd) {
      d.at = p;
      this.glue = this.glueAt(p, e, d.id);
    } else if (d.seg) {
      // An elbow line's middle segment moves sideways; its bends are kept as waypoints.
      const [a, b] = [d.route[d.index], d.route[d.index + 1]];
      const horizontal = Math.abs(a[1] - b[1]) < 1e-6;
      const shift = horizontal ? [0, p[1] - d.start[1]] : [p[0] - d.start[0], 0];
      const moved = d.route.map((q, i) => (i === d.index || i === d.index + 1 ? [q[0] + shift[0], q[1] + shift[1]] : q));
      const n = moved.length;
      const waypoints = [];
      for (let k = 1; k < n - 2; k++) waypoints.push([(moved[k][0] + moved[k + 1][0]) / 2, (moved[k][1] + moved[k + 1][1]) / 2]);
      this.app.run('set_waypoints', { id: d.id, waypoints, drag: d.n }, { quiet: true });
    } else if (d.guide) {
      this.newGuide = { vertical: d.guide.vertical, pos: Math.round(d.guide.vertical ? p[0] : p[1]) };
    } else if (d.marquee) {
      const r = this.stage.getBoundingClientRect();
      d.marquee.to = [e.clientX - r.left, e.clientY - r.top];
    } else if (d.pivot) {
      this.pivot = { key: JSON.stringify(this.app.state.selection), at: p };
    } else if (d.perspective) {
      d.to[d.corner] = p;
      d.quad[d.corner] = this.toScreen(p);
      this.app.run('distort', { from: d.from, to: d.to }, { quiet: true });
    } else if (d.sculpt) {
      const delta = [p[0] - d.last[0], p[1] - d.last[1]];
      d.last = p;
      this.cursor = p;
      // Alt swaps pulling in and pushing away.
      const m = this.app.sculpt.mode;
      const mode = e.altKey && m !== 'push' ? (m === 'attract' ? 'repel' : 'attract') : m;
      this.quietRun(d, 'sculpt', { center: p, delta, radius: this.app.sculpt.size, amount: this.app.sculpt.strength, mode, stroke: d.stroke });
    } else if (d.scale) {
      const [ax, ay] = d.anchor, [sx0, sy0] = d.start;
      let sx = Math.abs(sx0 - ax) > 1e-9 ? (p[0] - ax) / (sx0 - ax) : 1;
      let sy = Math.abs(sy0 - ay) > 1e-9 ? (p[1] - ay) / (sy0 - ay) : 1;
      if (d.axis === 'x') sy = e.shiftKey ? sx : 1;
      if (d.axis === 'y') sx = e.shiftKey ? sy : 1;
      if (e.shiftKey) { const m = Math.max(Math.abs(sx), Math.abs(sy)); sx = Math.sign(sx || 1) * m; sy = Math.sign(sy || 1) * m; }
      this.app.run('transform', { affine: scaleAbout(ax, ay, sx || 0.001, sy || 0.001) }, { quiet: true });
      this.readout = { at: e, text: Math.abs(sx - sy) < 1e-9 ? `${Math.round(sx * 1000) / 10}%` : `${Math.round(sx * 1000) / 10}% × ${Math.round(sy * 1000) / 10}%` };
    } else if (d.rotate) {
      let a = Math.atan2(p[1] - d.c[1], p[0] - d.c[0]) - d.a0;
      if (e.shiftKey) a = Math.round(a / (Math.PI / 12)) * (Math.PI / 12);
      this.app.run('transform', { affine: rotateAbout(d.c[0], d.c[1], a) }, { quiet: true });
      this.readout = { at: e, text: `${Math.round((a * 180) / Math.PI * 10) / 10}°` };
    }
    this.redraw();
  }

  up(e) {
    this.snapLines = [];
    this.readout = null;
    if (this.pen?.dragging) { this.pen.dragging = false; this.redraw(); }
    const d = this.drag;
    this.drag = null;
    if (d?.node) { d.ed.up(); return; }
    if (!d) return;
    if (d.raster) { this.app.raster?.up(e); return; }
    if (d.joint) {
      if (d.target != null) {
        const name = (id) => this.app.layerById(id)?.name || 'the part';
        if (this.app.run('attach', { id: d.id, parent: d.target }) !== null) this.app.toast?.(`${name(d.id)} now hangs from ${name(d.target)} and turns about this joint.`);
      }
      this.redraw();
      return;
    }
    if (d.pan) { this.overlay.style.cursor = this.space ? 'grab' : this.cursorFor(this.app.tool); return; }
    if (d.zoomBox) {
      // A click zooms in (out with a modifier); a box zooms to fit it.
      const [a, b] = [d.zoomBox, d.to];
      if (Math.abs(b[0] - a[0]) > 4 && Math.abs(b[1] - a[1]) > 4) {
        const r0 = this.stage.getBoundingClientRect();
        const c0 = [(Math.min(a[0], b[0]) - this.ox) / this.k, (Math.min(a[1], b[1]) - this.oy) / this.k];
        const c1 = [(Math.max(a[0], b[0]) - this.ox) / this.k, (Math.max(a[1], b[1]) - this.oy) / this.k];
        const k = Math.max(0.02, Math.min(64, Math.min(r0.width / (c1[0] - c0[0]), r0.height / (c1[1] - c0[1]))));
        this.k = k;
        this.ox = r0.width / 2 - ((c0[0] + c1[0]) / 2) * k;
        this.oy = r0.height / 2 - ((c0[1] + c1[1]) / 2) * k;
        this.app.onZoom?.();
        this.redraw();
      } else this.zoomTo(this.k * (d.out ? 0.5 : 2), a);
      return;
    }
    if (d.newText) {
      const [a, b] = [d.newText, d.to];
      const size = Math.max(12, Math.round(48 / Math.max(this.k, 0.25) * 0.5));
      const big = Math.abs(b[0] - a[0]) * this.k > 8 && Math.abs(b[1] - a[1]) * this.k > 8;
      const id = big
        ? this.app.run('add_text', { x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), text: 'Text', size, box: [Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1])] })
        : this.app.run('add_text', { x: a[0], y: a[1], text: 'Text', size });
      if (!big) this.app.setTool('select');
      if (id != null) this.beginEdit(id, { selectAll: true });
      return;
    }
    if (d.tbox) { this.app.changed(); return; }
    if (d.connect) {
      this.glue = null;
      const g = this.glueAt(d.to, e, d.fromId);
      if (Math.hypot(d.to[0] - d.from[0], d.to[1] - d.from[1]) * this.k > 6) {
        this.app.run('connect', { a: d.from, b: g.at, from: d.fromId, from_port: d.fromPort, to: g.layer, to_port: g.port });
      }
    } else if (d.grad) {
      this.app.changed();
    } else if (d.knife) {
      if (Math.hypot(d.to[0] - d.from[0], d.to[1] - d.from[1]) * this.k > 4) {
        let cuts = [];
        try { cuts = call('knife_preview', { a: d.from, b: d.to }); } catch { /* none */ }
        if (!cuts.length) this.app.hint?.('The knife cuts filled shapes: drag right across one, from outside it to outside it.');
        else {
          const n = this.app.run('knife', { a: d.from, b: d.to });
          // The cut stays marked for a moment, so you can see where the pieces part.
          if (n != null) { this.cutFlash = { cuts, at: performance.now() }; this.app.toast?.(`Cut into ${n} pieces.`); }
        }
      }
    } else if (d.shape === 'line') {
      let [a, b] = [d.from, d.to];
      // A click: the exact length and direction, typed.
      if (Math.hypot(b[0] - a[0], b[1] - a[1]) * this.k < 4) { this.app.shapeSizeDialog?.('line', a); return; }
      this.app.run('add_line', { a, b, stroke: this.app.stroke, stroke_width: Math.max(1, this.app.strokeWidth) });
    } else if (d.shape) {
      const [a, b] = [d.from, d.to];
      // A click: the exact size, typed (the shape goes centred there).
      if (Math.abs(b[0] - a[0]) * this.k < 4 && Math.abs(b[1] - a[1]) * this.k < 4) { this.app.shapeSizeDialog?.(d.shape, d.origin || a); return; }
      this.app.run('add_figure', { kind: d.shape, x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), w: Math.abs(b[0] - a[0]), h: Math.abs(b[1] - a[1]), ...this.app.paint(), ...this.app.shapeOpts });
    } else if (d.points) {
      if (d.points.length > 1) {
        // Alt: this stroke stays as drawn (no shape recognition).
        this.app.run('freehand', { points: d.points, brush: d.brush, recognize: this.app.recognize && !e.altKey, width: d.brush ? this.app.brushSize : Math.max(1, this.app.strokeWidth), fill: this.app.fill, stroke: this.app.stroke, smoothing: this.app.smoothing });
      }
    } else if (d.emitter || d.level || d.modOrigin || d.camera) {
      this.app.changed();
    } else if (d.connEnd) {
      this.glue = null;
      const g = this.glueAt(d.at, e, d.id);
      this.app.run('reglue', { id: d.id, end: d.connEnd, at: g.at, layer: g.layer, port: g.port });
    } else if (d.seg) {
      this.app.changed();
    } else if (d.guide) {
      const gd = call('guides');
      const list = gd.guides.slice();
      const r0 = this.stage.getBoundingClientRect();
      const s = [e.clientX - r0.left, e.clientY - r0.top];
      // Dropped back on the ruler (or off the canvas): removed.
      const gone = this.app.prefs.showRulers && (d.guide.vertical ? s[0] < 20 : s[1] < 20);
      if (d.guide.index != null) list.splice(d.guide.index, 1);
      if (!gone && this.newGuide) list.push({ vertical: d.guide.vertical, pos: this.newGuide.pos });
      this.newGuide = null;
      this.app.run('set_guides', { guides: list });
    } else if (d.marquee) {
      const [a, b] = [this.drag?.marquee?.from || d.marquee.from, d.marquee.to];
      const [x0, x1, y0, y1] = [Math.min(a[0], b[0]), Math.max(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[1], b[1])];
      if (x1 - x0 < 3 && y1 - y0 < 3) { if (!d.add) this.app.select([]); }
      else {
        // What a click would pick: the top layers, and what's in a layer (folder) rather than the layer itself.
        const top = [], stack = [];
        for (const l of this.app.state.layers) {
          stack.length = l.depth;
          const parent = stack[l.depth - 1];
          const off = !l.visible || l.locked || (parent && parent.off);
          if (!off && !l.folder && (l.depth === 0 || (parent?.folder && !parent.off))) top.push(l.id);
          stack[l.depth] = { folder: l.folder, off };
        }
        const hits = call('bounds', { ids: top }).filter((bx) => {
          const q = bx.corners.map((c) => this.toScreen(c));
          const [bx0, bx1] = [Math.min(...q.map((c) => c[0])), Math.max(...q.map((c) => c[0]))];
          const [by0, by1] = [Math.min(...q.map((c) => c[1])), Math.max(...q.map((c) => c[1]))];
          return bx1 >= x0 && bx0 <= x1 && by1 >= y0 && by0 <= y1;
        }).map((bx) => bx.id);
        this.app.select(d.add ? [...new Set([...this.app.state.selection, ...hits])] : hits);
      }
    } else if (d.orbit) {
      this.app.changed();
    } else if (d.crop) {
      this.app.onCrop?.();
    } else if (d.ik) {
      this.app.changed();
    } else if (d.perspective || d.sculpt) {
      call('transform_end');
      this.app.changed();
    } else if (d.move || d.scale || d.rotate) {
      call('transform_end');
      this.app.changed();
    }
    this.redraw();
  }

  double(e) {
    const p = this.toComp(e);
    // Free transform: a double-click on the centre puts it back in the middle.
    if (this.app.tool === 'xform' && this.pivot) {
      const u = this.union(this.selectionBoxes());
      if (u) { const c = this.toScreen(this.pivotFor(u)); const r0 = this.stage.getBoundingClientRect(); if (Math.hypot(c[0] - (e.clientX - r0.left), c[1] - (e.clientY - r0.top)) < 10) { this.pivot = null; this.redraw(); return; } }
    }
    if (this.app.tool === 'node' || this.app.tool === 'mask') {
      const r = this.stage.getBoundingClientRect();
      (this.app.tool === 'mask' ? this.maskPoints : this.points).double(p, [e.clientX - r.left, e.clientY - r.top]);
      return;
    }
    if (this.app.tool !== 'select') { if (this.pen) this.finishPen(false); return; }
    const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.k });
    const full = hit != null && this.app.layerById(hit)?.kind === 'shape' ? call('layer_full', { id: hit }).layer : null;
    // Labels: shapes that have one, connectors, and any shape on a diagram canvas.
    const labelled = full && (full.label || full.connector || this.app.state.comp.kind === 'diagram');
    if (labelled) this.beginEdit(hit, { label: true, text: full.label?.text || '', at: p });
    else if (hit != null && this.app.layerById(hit)?.kind === 'text') this.beginEdit(hit, { at: p });
    else if (hit != null && this.app.layerById(hit)?.kind === 'precomp') {
      const full = call('layer_full', { id: hit });
      if (full.layer.comp != null) this.app.editSymbol(full.layer.comp);
    }
  }

  /// Pen: click for a sharp point, drag for a curve; click the first point
  /// to close, Enter or double-click to finish. Starting on an end of the
  /// selected open line carries that line on.
  penDown(p) {
    const near = (q) => Math.hypot(q[0] - p[0], q[1] - p[1]) * this.k < 10;
    if (!this.pen) {
      this.pen = { nodes: [], into: null };
      const id = this.app.state.selection.length === 1 ? this.app.state.selection[0] : null;
      if (id != null) {
        let pts = null;
        try { pts = call('points', { id }); } catch { /* not a shape */ }
        for (const sh of pts?.shapes || []) {
          const end = sh.ends.find((x) => near(x.p));
          if (end) {
            this.pen.into = { id, shape: sh.shape, subpath: end.subpath, from_start: end.from_start, start: end.p };
            this.pen.nodes.push({ p: end.p, h: [0, 0], fixed: true });
            this.redraw();
            return;
          }
        }
      }
    }
    const nodes = this.pen.nodes;
    const first = this.pen.into ? null : nodes[0];
    if (first && nodes.length > 2 && near(first.p)) { this.finishPen(true); return; }
    nodes.push({ p, h: [0, 0] });
    this.pen.dragging = true;
    this.redraw();
  }

  finishPen(closed) {
    const pen = this.pen;
    this.pen = null;
    if (!pen) return;
    // A double-click to finish also clicked twice: drop the points it added on top of the last.
    const all = pen.nodes.filter((n, i, list) => i === 0 || Math.hypot(n.p[0] - list[i - 1].p[0], n.p[1] - list[i - 1].p[1]) * this.k > 3);
    const nodes = all.filter((n) => !n.fixed).map((n) => ({ p: n.p, h: n.h }));
    if (pen.into) {
      if (nodes.length) this.app.run('pen_path', { nodes, closed, ...pen.into });
    } else if (nodes.length > 1) {
      this.app.run('pen_path', { nodes, closed, ...this.app.paint(), fill: closed ? this.app.fill : null });
    }
    this.redraw();
  }

  // ----- Typing on the canvas -----

  /// Starts typing in text layer `id`, or in the label of shape or connector `id` (`label`, with its `text` now).
  /// `at`: the canvas point double-clicked (its word is picked); `selectAll`: everything is picked (new text).
  beginEdit(id, opts = {}) {
    if (opts === true) opts = { selectAll: true };
    const { label = false, at = null, selectAll = false, unit = 2 } = opts;
    if (this.editing) this.endEdit();
    let text = opts.text;
    if (text == null) { try { text = label ? (call('layer_full', { id }).layer.label?.text ?? '') : (call('layer', { id }).text?.text ?? ''); } catch { text = ''; } }
    this.editing = { id, label, before: text, edit: (this.labelEdits = (this.labelEdits || 0) + 1), moved: performance.now() };
    call('edit_text', { id });
    const ta = this.textEdit;
    ta.value = text;
    ta.hidden = false;
    ta.focus({ preventScroll: true });
    if (selectAll) ta.select();
    else if (at) {
      let got = null;
      try { got = call('text_pick', { id, x: at[0], y: at[1], unit }); } catch { /* not laid out yet */ }
      if (got) this.setTyped(got.anchor, got.focus); else ta.setSelectionRange(text.length, text.length);
    } else ta.setSelectionRange(text.length, text.length);
    clearInterval(this.blink);
    this.blink = setInterval(() => { if (this.editing) this.redraw(); }, 530);
    this.caretMoved();
    this.app.renderStatus?.();
  }

  /// The typed selection as byte offsets (anchor, focus), as the engine counts.
  typedRange() {
    const ta = this.textEdit;
    const back = ta.selectionDirection === 'backward';
    const [a, f] = back ? [ta.selectionEnd, ta.selectionStart] : [ta.selectionStart, ta.selectionEnd];
    return [bytesAt(ta.value, a), bytesAt(ta.value, f)];
  }

  /// Picks bytes `anchor`..`focus` of the typed words.
  setTyped(anchor, focus) {
    const ta = this.textEdit;
    const [a, f] = [charsAt(ta.value, anchor), charsAt(ta.value, focus)];
    ta.setSelectionRange(Math.min(a, f), Math.max(a, f), f < a ? 'backward' : 'forward');
    this.caretMoved();
  }

  /// The caret moved or the words changed: the caret shows (blinking restarts) and the hidden field follows it (for IMEs).
  caretMoved() {
    if (!this.editing) return;
    this.editing.moved = performance.now();
    const [a, f] = this.typedRange();
    try { this.editing.geom = call('text_caret', { id: this.editing.id, anchor: a, focus: f }); } catch { this.editing.geom = null; }
    const c = this.editing.geom?.caret;
    if (c) {
      const q = this.toScreen(c[0]), q2 = this.toScreen(c[3]);
      Object.assign(this.textEdit.style, { left: `${q[0]}px`, top: `${q[1]}px`, height: `${Math.max(8, Math.abs(q2[1] - q[1]))}px`, transform: 'none' });
    }
    this.redraw();
  }

  /// Whether canvas point `p` is on the words being typed (their box, or the shape a label is in).
  onTyped(p) {
    if (!this.editing) return false;
    let quads = [];
    try {
      const tb = call('text_box', { id: this.editing.id });
      const [x0, y0, x1, y1] = tb.frame ? [Math.min(tb.box[0], tb.frame[0]), Math.min(tb.box[1], tb.frame[1]), Math.max(tb.box[2], tb.frame[2]), Math.max(tb.box[3], tb.frame[3])] : tb.box;
      const pad = 6 / this.k;
      quads.push([[x0 - pad, y0 - pad], [x1 + pad, y0 - pad], [x1 + pad, y1 + pad], [x0 - pad, y1 + pad]].map((q) => applyAff(tb.affine, q)));
    } catch { /* none */ }
    if (this.editing.label) quads.push(...(call('bounds', { ids: [this.editing.id] }).map((b) => b.corners)));
    return quads.some((q) => insideQuad(q, p));
  }

  /// The selection, the caret (blinking) and a dashed outline of the words being typed.
  drawTyping(g) {
    const ed = this.editing;
    const geom = ed.geom;
    try {
      const tb = call('text_box', { id: ed.id });
      const r = tb.frame || tb.box;
      this.quad(g, [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map((q) => applyAff(tb.affine, q)), ACCENT, 1, [4, 3]);
    } catch { /* not on screen */ }
    if (!geom) {
      // An empty label: the caret in the middle of its shape.
      const b = ed.label && call('bounds', { ids: [ed.id] })[0];
      if (!b || Math.floor((performance.now() - ed.moved) / 530) % 2) return;
      const c = this.toScreen([b.corners.reduce((a, q) => a + q[0], 0) / b.corners.length, b.corners.reduce((a, q) => a + q[1], 0) / b.corners.length]);
      g.strokeStyle = '#1C1917'; g.lineWidth = 1.5;
      g.beginPath(); g.moveTo(c[0], c[1] - 10); g.lineTo(c[0], c[1] + 10); g.stroke();
      return;
    }
    g.fillStyle = 'rgba(194, 65, 12, 0.28)';
    for (const quad of geom.selection) {
      g.beginPath();
      quad.forEach((q, i) => { const sq = this.toScreen(q); if (i) g.lineTo(sq[0], sq[1]); else g.moveTo(sq[0], sq[1]); });
      g.closePath();
      g.fill();
    }
    const on = Math.floor((performance.now() - ed.moved) / 530) % 2 === 0;
    if (on && !geom.selection.length) {
      const [a, b] = [this.toScreen(geom.caret[0]), this.toScreen(geom.caret[3])];
      g.strokeStyle = '#1C1917';
      g.lineWidth = 1.5;
      g.beginPath(); g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]); g.stroke();
      g.strokeStyle = 'rgba(255, 255, 255, 0.8)';
      g.lineWidth = 0.75;
      g.beginPath(); g.moveTo(a[0] + 1.25, a[1]); g.lineTo(b[0] + 1.25, b[1]); g.stroke();
    }
  }

  /// Right-click while typing: the app's own menu, with spelling and grammar fixes for the word there.
  typingMenu(e) {
    const p = this.toComp(e);
    const ta = this.textEdit;
    const ui = this.app.ui;
    // Right-clicking on the words outside the selection moves the caret there first, as text fields do.
    if (this.onTyped(p)) {
      const [a, f] = this.typedRange();
      let got = null;
      try { got = call('text_pick', { id: this.editing.id, x: p[0], y: p[1], unit: 1 }); } catch { /* none */ }
      if (got && (got.focus < Math.min(a, f) || got.focus > Math.max(a, f))) this.setTyped(got.focus, got.focus);
    }
    const issue = this.app.writing?.at(p);
    const replace = (i, text) => {
      // Through the text field, so it's one more thing Ctrl+Z undoes.
      const [s0, e0] = i.chars;
      ta.focus({ preventScroll: true });
      ta.setSelectionRange(s0, e0);
      document.execCommand('insertText', false, text);
    };
    const keep = (f) => () => { this.keepTyping = true; try { f(); } finally { this.keepTyping = false; ta.focus({ preventScroll: true }); } };
    const items = [];
    if (issue && issue.layer === this.editing.id) {
      const span = this.app.writing.chars(issue);
      const i = { ...issue, chars: span };
      if (i.message && i.category !== 'Spelling') items.push(ui.heading(i.message.length > 70 ? `${i.message.slice(0, 67)}…` : i.message));
      for (const sug of i.suggestions.slice(0, 5)) items.push(ui.item(sug || '(remove it)', { run: keep(() => replace(i, sug)) }));
      if (!i.suggestions.length) items.push(ui.item('No suggestions', { enabled: false }));
      if (i.category === 'Spelling') items.push(ui.item(`Add “${i.word}” to my words`, { run: keep(() => this.app.writing.addWord(i)) }));
      items.push(ui.item('Ignore', { run: keep(() => this.app.writing.ignore(i)) }), document.createElement('hr'));
    }
    const has = ta.selectionStart !== ta.selectionEnd;
    items.push(
      ui.item('Cut', { keys: 'Ctrl+X', enabled: has, run: keep(() => document.execCommand('cut')) }),
      ui.item('Copy', { keys: 'Ctrl+C', enabled: has, run: keep(() => document.execCommand('copy')) }),
      ui.item('Paste', { keys: 'Ctrl+V', run: keep(async () => { let t = ''; try { t = await navigator.clipboard.readText(); } catch { /* not allowed here */ } if (t) { ta.focus({ preventScroll: true }); document.execCommand('insertText', false, t); } }) }),
      ui.item('Select all', { keys: 'Ctrl+A', run: keep(() => { ta.select(); this.caretMoved(); }) }),
      document.createElement('hr'),
      ui.item('Done typing', { keys: 'Esc', run: () => this.endEdit() }),
    );
    this.keepTyping = true;
    const menu = ui.popup(null, items, { at: [e.clientX, e.clientY] });
    setTimeout(() => { this.keepTyping = false; }, 0);
    // Closed without picking anything: back to typing.
    new MutationObserver((_, ob) => { if (!menu.isConnected) { ob.disconnect(); if (this.editing && document.activeElement !== ta) ta.focus({ preventScroll: true }); } }).observe(menu.parentNode, { childList: true });
  }

  /// A shape's or connector's label, typed where it is.
  beginLabel(id, now) {
    this.beginEdit(id, { label: true, text: now, selectAll: true });
  }

  endLabel() {
    if (this.editing?.label) this.endEdit();
  }

  endEdit() {
    if (!this.editing) return;
    const { id, label } = this.editing;
    this.editing = null;
    clearInterval(this.blink);
    this.textEdit.hidden = true;
    call('edit_text', { id: null });
    // Typed nothing: no empty text layer is left behind.
    if (!label && !this.textEdit.value.trim()) {
      this.app.select([id]);
      this.app.run('delete');
    }
    this.app.changed();
    this.redraw();
  }
}

/// UTF-16 offset `i` of `text` as a byte offset (UTF-8, as the engine and the checker count).
function bytesAt(text, i) {
  return new TextEncoder().encode(text.slice(0, i)).length;
}

/// Byte offset `b` of `text` as a UTF-16 offset.
function charsAt(text, b) {
  const enc = new TextEncoder();
  let bytes = 0, i = 0;
  for (const ch of text) {
    if (bytes >= b) return i;
    bytes += enc.encode(ch).length;
    i += ch.length;
  }
  return i;
}

function insideQuad(q, [x, y]) {
  let inside = false;
  for (let i = 0, j = q.length - 1; i < q.length; j = i++) {
    const [xi, yi] = q[i], [xj, yj] = q[j];
    if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

function capture(el, e) {
  try { el.setPointerCapture(e.pointerId); } catch { /* a synthetic or ended pointer */ }
}

function snapAngle(from, p) {
  const dx = p[0] - from[0], dy = p[1] - from[1];
  const step = Math.PI / 12;
  const a = Math.round(Math.atan2(dy, dx) / step) * step, len = Math.hypot(dx, dy);
  return [from[0] + Math.cos(a) * len, from[1] + Math.sin(a) * len];
}

function square(from, p) {
  const dx = p[0] - from[0], dy = p[1] - from[1];
  const m = Math.max(Math.abs(dx), Math.abs(dy));
  return [from[0] + Math.sign(dx || 1) * m, from[1] + Math.sign(dy || 1) * m];
}

export function isTyping(e) {
  const t = e.target;
  return t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
}

/// Applies affine [a, b, c, d, e, f] to a point.
function applyAff(m, [x, y]) {
  return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
}

/// The inverse of affine `m` applied to a point.
function applyInv(m, [x, y]) {
  const det = m[0] * m[3] - m[1] * m[2] || 1;
  const px = x - m[4], py = y - m[5];
  return [(m[3] * px - m[2] * py) / det, (-m[1] * px + m[0] * py) / det];
}

/// A number for the readouts: whole when it's big, else two decimals.
function fmtNum(v) {
  return Math.abs(v) >= 100 ? String(Math.round(v)) : String(Math.round(v * 100) / 100);
}
