// The graph editor: one property's value over time as curves. Drag a
// keyframe to change its value (and, sideways, its time); drag the round
// handles between two keyframes to shape the easing (they're the easing's
// cubic-bézier control points, drawn in value space).

import { call } from './engine.js';

const DIM = ['#E0604F', '#3FA34D', '#2563EB', '#9CA3AF'];
const PAD = { l: 52, r: 16, t: 14, b: 22 };

export class Graph {
  constructor(app, h) {
    this.app = app;
    this.h = h;
    this.prop = null;
    this.canvas = h('canvas', { class: 'graph-canvas', 'aria-label': 'Graph of the property over time' });
    this.pick = h('select', { class: 'field small', 'aria-label': 'Property', onChange: (e) => { this.prop = e.target.value; this.draw(); } });
    this.el = h('div', { class: 'graph' }, h('div', { class: 'graph-bar' }, h('span', {}, 'Property'), this.pick,
      h('span', { class: 'graph-tip' }, 'Drag a keyframe to change it (Shift: its value only); drag the round handles to change how it speeds up and slows down.')), this.canvas);
    this.canvas.addEventListener('pointerdown', (e) => this.down(e));
    new ResizeObserver(() => this.draw()).observe(this.canvas);
  }

  /// The selected layer's animated properties, the chosen one first.
  props() {
    const info = this.app.layerInfo;
    if (!info) return [];
    return Object.entries(info.props).filter(([, v]) => v.animated && (typeof v.value === 'number' || Array.isArray(v.value))).map(([k]) => k);
  }

  draw() {
    const { h } = this;
    const props = this.props();
    const info = this.app.layerInfo;
    if (!props.includes(this.prop)) this.prop = props[0] || null;
    this.pick.replaceChildren(...props.map((p) => h('option', { value: p, selected: p === this.prop }, this.app.labelFor(p))));
    const c = this.canvas;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, hh = c.clientHeight;
    if (!w || !hh) return;
    c.width = Math.round(w * dpr);
    c.height = Math.round(hh * dpr);
    const g = c.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, hh);
    const css = getComputedStyle(document.documentElement);
    const ink = css.getPropertyValue('--muted').trim() || '#78716C';
    const line = css.getPropertyValue('--line').trim() || '#E7E2DC';
    g.font = '11px Inter, sans-serif';
    if (!info || !this.prop) {
      g.fillStyle = ink;
      g.fillText(info ? 'Nothing animated on this layer yet: click a diamond in the Layer panel.' : 'Select a layer to see its animation.', PAD.l, hh / 2);
      this.data = null;
      return;
    }
    let data;
    try { data = call('graph', { id: info.id, prop: this.prop }); } catch { return; }
    const scale = this.prop === 'opacity' || /opacity|trim_/.test(this.prop) ? 100 : this.prop === 'scale' ? 100 : 1;
    const vals = data.samples.flatMap((r) => r.slice(1));
    let lo = Math.min(...vals), hi = Math.max(...vals);
    if (!(hi - lo > 1e-9)) { lo -= 1; hi += 1; }
    const span = hi - lo;
    lo -= span * 0.12; hi += span * 0.12;
    const dur = this.app.state.comp.duration || 1;
    const X = (t) => PAD.l + (t / dur) * (w - PAD.l - PAD.r);
    const Y = (v) => PAD.t + (1 - (v - lo) / (hi - lo)) * (hh - PAD.t - PAD.b);
    this.map = { X, Y, lo, hi, dur, w, hh, T: (x) => ((x - PAD.l) / (w - PAD.l - PAD.r)) * dur, V: (y) => lo + (1 - (y - PAD.t) / (hh - PAD.t - PAD.b)) * (hi - lo) };
    // Grid: a few values and seconds.
    g.strokeStyle = line;
    g.fillStyle = ink;
    g.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const v = lo + ((hi - lo) * i) / 4, y = Y(v);
      g.beginPath(); g.moveTo(PAD.l, y); g.lineTo(w - PAD.r, y); g.stroke();
      g.fillText(String(+(v * scale).toFixed(1)), 6, y + 4);
    }
    const stepT = [0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60].find((s) => (dur / s) <= 12) || 60;
    for (let t = 0; t <= dur + 1e-9; t += stepT) { const x = X(t); g.fillText(`${+t.toFixed(2)}s`, x - 8, hh - 6); }
    // Playhead.
    g.strokeStyle = '#C2410C';
    g.beginPath(); g.moveTo(X(this.app.state.time), PAD.t); g.lineTo(X(this.app.state.time), hh - PAD.b); g.stroke();
    // Curves.
    const n = data.samples[0]?.length - 1 || 0;
    for (let d = 0; d < n; d++) {
      g.strokeStyle = n === 1 ? '#C2410C' : DIM[d];
      g.lineWidth = 2;
      g.beginPath();
      data.samples.forEach((r, i) => (i ? g.lineTo(X(r[0]), Y(r[d + 1])) : g.moveTo(X(r[0]), Y(r[d + 1]))));
      g.stroke();
    }
    // Keyframes and the easing handles of each segment (on every value's curve).
    this.hits = [];
    data.keys.forEach((k, i) => {
      for (let d = 0; d < k.value.length; d++) {
        const [x, y] = [X(k.time), Y(k.value[d])];
        g.fillStyle = '#fff'; g.strokeStyle = n === 1 ? '#C2410C' : DIM[d]; g.lineWidth = 2;
        g.beginPath(); g.moveTo(x, y - 6); g.lineTo(x + 6, y); g.lineTo(x, y + 6); g.lineTo(x - 6, y); g.closePath(); g.fill(); g.stroke();
        this.hits.push({ kind: 'key', i, d, x, y, time: k.time });
      }
      const next = data.keys[i + 1];
      const e = k.easing || { type: 'linear' };
      if (!next) return;
      if (e.type === 'bezier') this.handlesFor(g, X, Y, k, next, e, i);
      else if (e.type === 'linear') this.handlesFor(g, X, Y, k, next, { x1: 1 / 3, y1: 1 / 3, x2: 2 / 3, y2: 2 / 3 }, i);
    });
    this.data = data;
  }

  /// The easing's two handles drawn on each value's curve (x and y of a position…); any of them shapes it.
  handlesFor(g, X, Y, a, b, bz, i) {
    for (let d = 0; d < a.value.length; d++) this.handlesOn(g, X, Y, a, b, bz, i, d);
  }

  handlesOn(g, X, Y, a, b, bz, i, d) {
    const dv = b.value[d] - a.value[d], dt = b.time - a.time;
    if (Math.abs(dv) < 1e-9) return;
    const p0 = [X(a.time), Y(a.value[d])], p3 = [X(b.time), Y(b.value[d])];
    const h1 = [X(a.time + bz.x1 * dt), Y(a.value[d] + bz.y1 * dv)];
    const h2 = [X(a.time + bz.x2 * dt), Y(a.value[d] + bz.y2 * dv)];
    g.strokeStyle = '#A8A29E'; g.lineWidth = 1;
    g.beginPath(); g.moveTo(...p0); g.lineTo(...h1); g.moveTo(...p3); g.lineTo(...h2); g.stroke();
    for (const [hx, hy, out] of [[...h1, true], [...h2, false]]) {
      g.beginPath(); g.arc(hx, hy, 4.5, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.strokeStyle = '#57534E'; g.stroke();
      this.hits.push({ kind: 'handle', i, d, out, x: hx, y: hy, bz, a, b });
    }
  }

  down(e) {
    if (!this.data || !this.hits) return;
    const r = this.canvas.getBoundingClientRect();
    const at = [e.clientX - r.left, e.clientY - r.top];
    const hit = this.hits.slice().reverse().find((h) => Math.hypot(h.x - at[0], h.y - at[1]) < 8);
    if (!hit) {
      this.app.setTime(Math.max(0, Math.min(this.map.dur, this.map.T(at[0]))));
      this.draw();
      return;
    }
    e.preventDefault();
    this.canvas.setPointerCapture(e.pointerId);
    const id = this.app.layerInfo.id, prop = this.prop;
    const fps = this.app.state.comp.fps || 30;
    let time = hit.time;
    const move = (ev) => {
      const p = [ev.clientX - r.left, ev.clientY - r.top];
      if (hit.kind === 'key') {
        const key = this.data.keys[hit.i];
        const value = key.value.slice();
        value[hit.d] = this.map.V(p[1]);
        // Sideways moves its time (between its neighbours), snapped to frames; Shift: the value only.
        const lo = this.data.keys[hit.i - 1]?.time ?? 0, hi = this.data.keys[hit.i + 1]?.time ?? this.map.dur;
        const t = Math.round(Math.max(lo + 1 / fps, Math.min(hi - 1 / fps, this.map.T(p[0]))) * fps) / fps;
        if (!ev.shiftKey && Math.abs(t - time) > 1e-6 && Math.abs(ev.clientX - e.clientX) > 6) { this.app.run('move_key', { id, prop, from: time, to: t }, { quiet: true }); time = t; }
        this.app.run('set_key', { id, prop, time, value: value.length === 1 ? value[0] : value }, { quiet: true });
      } else {
        const { a, b, bz, d } = hit;
        const dt = b.time - a.time, dv = b.value[d] - a.value[d];
        const x = Math.max(0, Math.min(1, (this.map.T(p[0]) - a.time) / dt));
        const y = (this.map.V(p[1]) - a.value[d]) / dv;
        const nb = hit.out ? { x1: x, y1: y, x2: bz.x2, y2: bz.y2 } : { x1: bz.x1, y1: bz.y1, x2: x, y2: y };
        this.app.run('set_easing', { id, prop, time: a.time, easing: { type: 'bezier', ...nb }, merge: true }, { quiet: true });
      }
      this.app.layerInfo = call('layer', { id });
      this.draw();
    };
    const up = () => { this.canvas.removeEventListener('pointermove', move); this.canvas.removeEventListener('pointerup', up); this.app.changed(); };
    this.canvas.addEventListener('pointermove', move);
    this.canvas.addEventListener('pointerup', up);
  }
}
