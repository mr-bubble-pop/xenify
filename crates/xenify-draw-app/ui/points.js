// Edit Points (A): a shape's points, handles and outline over the canvas.
// Click a point to select it (Shift adds), drag a box around several, drag
// selected points to move them, drag an edge to move both its ends, drag a
// handle to bend the curve (Alt moves one side only). The circles inside
// sharp corners round them: every corner, or with Alt just that one.
// Double-click a point to make it smooth or sharp, double-click the outline
// to add a point. Right-click for the point tools.

import { call } from './engine.js';

const ACCENT = '#C2410C';
const R = 4.5;
/// How far inside a sharp corner its rounding circle sits (screen px); shapes with more corners than MAX get none.
const INSET = 16, MAX = 48;

export class PointEditor {
  /// `masks`: edit the selected layer's masks (the mask tool) instead of its shapes.
  constructor(vp, { masks = false } = {}) {
    this.masks = masks;
    this.color = masks ? '#E0A800' : ACCENT;
    this.vp = vp;
    this.app = vp.app;
    this.sel = [];
    this.cache = null;
    this.drag = null;
  }

  /// The selected layer's points (composition space), or null.
  data() {
    const st = this.app.state;
    const id = st?.selection?.length === 1 ? st.selection[0] : null;
    if (id == null) return null;
    if (this.cache?.rev === this.app.rev && this.cache.id === id) return this.cache.data;
    let data = null;
    try { data = call('points', { id, masks: this.masks }); } catch { data = null; }
    if (this.cache?.id !== id) this.sel = [];
    this.cache = { rev: this.app.rev, id, data, corners: null };
    return data;
  }

  /// The sharp corners' rounding circles (screen points), when they fit.
  cornerCircles() {
    if (this.masks || !this.data()) return [];
    const c = this.cache;
    if (!c.corners) { try { c.corners = call('corners', { id: c.id }); } catch { c.corners = []; } }
    const k = this.vp.k;
    const list = c.corners.filter((x) => x.edge * k >= INSET * 1.5);
    if (list.length > MAX) return [];
    return list.map((x) => ({ ...x, s: this.vp.toScreen([x.at[0] + (x.dir[0] * INSET) / k, x.at[1] + (x.dir[1] * INSET) / k]) }));
  }

  isSel(s, i) {
    return this.sel.some(([a, b]) => a === s && b === i);
  }

  draw(g) {
    const d = this.data();
    if (!d) return;
    const S = (p) => this.vp.toScreen(p);
    g.lineWidth = 1.25;
    g.strokeStyle = this.color;
    for (const sh of d.shapes) {
      g.beginPath();
      for (const seg of sh.segments) {
        const a = S(seg[0]);
        g.moveTo(a[0], a[1]);
        if (seg.length === 2) { const b = S(seg[1]); g.lineTo(b[0], b[1]); }
        else { const [c1, c2, b] = [S(seg[1]), S(seg[2]), S(seg[3])]; g.bezierCurveTo(c1[0], c1[1], c2[0], c2[1], b[0], b[1]); }
      }
      g.stroke();
    }
    for (const sh of d.shapes) {
      sh.anchors.forEach((a, i) => {
        const on = this.isSel(sh.shape, i);
        const near = this.allHandles || on || this.isSel(sh.shape, i - 1) || this.isSel(sh.shape, i + 1);
        if (near) {
          for (const hd of [a.in, a.out]) {
            if (!hd) continue;
            const [p, q] = [S(a.p), S(hd)];
            g.beginPath(); g.moveTo(p[0], p[1]); g.lineTo(q[0], q[1]); g.stroke();
            g.beginPath(); g.arc(q[0], q[1], 3.5, 0, Math.PI * 2);
            g.fillStyle = '#fff'; g.fill(); g.stroke();
          }
        }
        const p = S(a.p);
        g.fillStyle = on ? this.color : '#fff';
        g.fillRect(p[0] - R, p[1] - R, R * 2, R * 2);
        g.strokeRect(p[0] - R, p[1] - R, R * 2, R * 2);
      });
    }
    // Rounding circles inside sharp corners.
    if (!this.drag || this.drag.corner) {
      g.lineWidth = 1.25;
      for (const cc of this.cornerCircles()) {
        g.beginPath(); g.arc(cc.s[0], cc.s[1], 4.5, 0, Math.PI * 2);
        g.fillStyle = 'rgba(255,255,255,.9)'; g.fill(); g.strokeStyle = this.color; g.stroke();
      }
    }
    if (this.drag?.newMask) {
      const [a, b] = [S(this.drag.newMask.from), S(this.drag.newMask.to)];
      g.setLineDash([5, 4]);
      g.beginPath();
      if (this.app.maskShape === 'ellipse') g.ellipse((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, Math.abs(b[0] - a[0]) / 2, Math.abs(b[1] - a[1]) / 2, 0, 0, Math.PI * 2);
      else g.rect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.stroke();
      g.setLineDash([]);
    }
    if (this.drag?.box) {
      const [a, b] = [this.drag.box.from, this.drag.box.to];
      g.setLineDash([4, 3]);
      g.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
      g.setLineDash([]);
    }
  }

  /// What's under screen point `s`: a handle, a point, or nothing.
  hitAt(s) {
    const d = this.data();
    if (!d) return null;
    const S = (p) => this.vp.toScreen(p);
    const near = (p, r) => Math.hypot(s[0] - p[0], s[1] - p[1]) <= r;
    for (const sh of d.shapes) {
      for (let i = 0; i < sh.anchors.length; i++) {
        const a = sh.anchors[i];
        const shown = this.allHandles || this.isSel(sh.shape, i) || this.isSel(sh.shape, i - 1) || this.isSel(sh.shape, i + 1);
        if (!shown) continue;
        if (a.in && near(S(a.in), 7)) return { handle: true, shape: sh.shape, index: i, incoming: true };
        if (a.out && near(S(a.out), 7)) return { handle: true, shape: sh.shape, index: i, incoming: false };
      }
    }
    for (const sh of d.shapes) {
      for (let i = 0; i < sh.anchors.length; i++) if (near(S(sh.anchors[i].p), R + 3)) return { point: true, shape: sh.shape, index: i };
    }
    return null;
  }

  down(e, p, s) {
    const d = this.data();
    const hit = this.hitAt(s);
    // A corner's circle: drag it in to round the corner (Alt: only that one).
    const cc = !hit ? this.cornerCircles().find((x) => Math.hypot(x.s[0] - s[0], x.s[1] - s[1]) <= 7) : null;
    if (cc && d) {
      call('points_begin', { masks: this.masks });
      this.drag = { corner: cc, one: e.altKey, id: d.id };
      return;
    }
    if (hit?.handle) {
      call('points_begin', { masks: this.masks });
      this.drag = { handle: hit };
      return;
    }
    if (hit?.point) {
      const key = [hit.shape, hit.index];
      if (e.shiftKey) this.sel = this.isSel(...key) ? this.sel.filter(([a, b]) => !(a === key[0] && b === key[1])) : [...this.sel, key];
      else if (!this.isSel(...key)) this.sel = [key];
      call('points_begin', { masks: this.masks });
      this.drag = { points: true, from: p, moved: false };
      this.vp.redraw();
      return;
    }
    // An edge: both its ends move together.
    const edge = !hit && d ? this.edgeAt(s) : null;
    if (edge) {
      if (!(this.isSel(edge.shape, edge.a) && this.isSel(edge.shape, edge.b))) this.sel = e.shiftKey ? [...this.sel, [edge.shape, edge.a], [edge.shape, edge.b]] : [[edge.shape, edge.a], [edge.shape, edge.b]];
      call('points_begin', { masks: this.masks });
      this.drag = { points: true, from: p, moved: false };
      this.vp.redraw();
      return;
    }
    if (this.masks && d) {
      // Mask tool: dragging where there's no point draws a new mask on this layer.
      this.drag = { newMask: { from: p, to: p }, id: d.id };
      return;
    }
    // Another layer: edit its points instead.
    const other = call('hit', { x: p[0], y: p[1], tolerance: 4 / this.vp.k });
    if (other != null && other !== d?.id) {
      this.sel = [];
      this.app.select([other]);
      return;
    }
    if (other == null && !d) { this.app.select([]); return; }
    this.drag = { box: { from: s, to: s }, add: e.shiftKey };
  }

  move(e, p, s) {
    const d = this.drag;
    if (!d) {
      const hit = this.hitAt(s);
      this.vp.overlay.style.cursor = hit ? 'pointer' : 'default';
      return;
    }
    const id = this.data()?.id;
    if (d.corner) {
      // How far in from where the circle rests: the radius.
      const c = d.corner, k = this.vp.k;
      const proj = (p[0] - c.at[0]) * c.dir[0] + (p[1] - c.at[1]) * c.dir[1] - INSET / k;
      // The rounding starts this far along each edge: the drag's depth, seen from the edges.
      this.app.run('corner_round', { id: d.id, shape: c.shape, index: d.one ? c.index : null, radius: Math.min(Math.max(0, proj) * c.cos_half, c.edge / 2) }, { quiet: true });
    } else if (d.handle && id != null) {
      this.app.run('handle_move', { masks: this.masks, id, shape: d.handle.shape, index: d.handle.index, incoming: d.handle.incoming, to: p, mirror: !e.altKey }, { quiet: true });
    } else if (d.points && id != null) {
      let dx = p[0] - d.from[0], dy = p[1] - d.from[1];
      if (e.shiftKey) { if (Math.abs(dx) > Math.abs(dy)) dy = 0; else dx = 0; }
      d.moved = true;
      this.app.run('points_move', { masks: this.masks, id, points: this.sel, dx, dy }, { quiet: true });
    } else if (d.box) {
      d.box.to = s;
    } else if (d.newMask) {
      d.newMask.to = e.shiftKey ? [d.newMask.from[0] + Math.max(Math.abs(p[0] - d.newMask.from[0]), Math.abs(p[1] - d.newMask.from[1])) * Math.sign(p[0] - d.newMask.from[0] || 1), d.newMask.from[1] + Math.max(Math.abs(p[0] - d.newMask.from[0]), Math.abs(p[1] - d.newMask.from[1])) * Math.sign(p[1] - d.newMask.from[1] || 1)] : p;
    }
    this.vp.redraw();
  }

  up() {
    const d = this.drag;
    this.drag = null;
    if (!d) return;
    if (d.newMask) {
      const [a, b] = [d.newMask.from, d.newMask.to];
      if (Math.abs(b[0] - a[0]) * this.vp.k > 4 && Math.abs(b[1] - a[1]) * this.vp.k > 4) {
        const i = this.app.run('add_mask_at', { id: d.id, a, b, ellipse: this.app.maskShape === 'ellipse' });
        if (i != null) this.sel = [];
      } else this.sel = [];
      this.vp.redraw();
      return;
    }
    if (d.box) {
      const [a, b] = [d.box.from, d.box.to];
      const [x0, x1, y0, y1] = [Math.min(a[0], b[0]), Math.max(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[1], b[1])];
      if (x1 - x0 < 3 && y1 - y0 < 3) {
        if (!d.add) this.sel = [];
      } else {
        const inside = [];
        for (const sh of this.data()?.shapes || []) {
          sh.anchors.forEach((an, i) => {
            const q = this.vp.toScreen(an.p);
            if (q[0] >= x0 && q[0] <= x1 && q[1] >= y0 && q[1] <= y1) inside.push([sh.shape, i]);
          });
        }
        this.sel = d.add ? [...this.sel, ...inside.filter(([s2, i2]) => !this.isSel(s2, i2))] : inside;
      }
    } else if (d.points || d.handle || d.corner) {
      this.app.changed();
    }
    this.vp.redraw();
  }

  /// The edge under screen point `s` (within 6 px), as the indexes of its two ends.
  edgeAt(s) {
    const d = this.data();
    if (!d) return null;
    const S = (q) => this.vp.toScreen(q);
    const same = (a, b) => Math.abs(a[0] - b[0]) < 1e-6 && Math.abs(a[1] - b[1]) < 1e-6;
    let best = null;
    for (const sh of d.shapes) {
      for (const seg of sh.segments) {
        // The distance to the segment, sampled along it.
        const pts = seg.length === 2 ? [seg[0], seg[1]] : Array.from({ length: 13 }, (_, j) => { const t = j / 12, u = 1 - t; return [0, 1].map((ax) => u * u * u * seg[0][ax] + 3 * u * u * t * seg[1][ax] + 3 * u * t * t * seg[2][ax] + t * t * t * seg[3][ax]); });
        for (let j = 0; j + 1 < pts.length; j++) {
          const [a, b] = [S(pts[j]), S(pts[j + 1])];
          const dx = b[0] - a[0], dy = b[1] - a[1], len2 = dx * dx + dy * dy || 1;
          const t = Math.max(0, Math.min(1, ((s[0] - a[0]) * dx + (s[1] - a[1]) * dy) / len2));
          const dist = Math.hypot(s[0] - (a[0] + t * dx), s[1] - (a[1] + t * dy));
          if (dist <= 6 && (!best || dist < best.dist)) {
            const ia = sh.anchors.findIndex((an) => same(an.p, seg[0])), ib = sh.anchors.findIndex((an) => same(an.p, seg[seg.length - 1]));
            if (ia >= 0 && ib >= 0) best = { dist, shape: sh.shape, a: ia, b: ib };
          }
        }
      }
    }
    return best;
  }

  double(p, s) {
    const d = this.data();
    if (!d) return;
    const hit = this.hitAt(s);
    if (hit?.point) {
      this.sel = [[hit.shape, hit.index]];
      this.edit('smooth');
      return;
    }
    const at = this.app.run('point_insert', { masks: this.masks, id: d.id, at: p, tolerance: 8 / this.vp.k });
    if (Array.isArray(at)) this.sel = [at];
    this.vp.redraw();
  }

  /// One of the point tools on the selected points (or the whole shape).
  edit(op, amount) {
    const d = this.data();
    if (!d) return;
    const out = this.app.run('points_edit', { masks: this.masks, id: d.id, op, points: this.sel, amount: amount ?? (op === 'simplify' ? 1 / this.vp.k : op === 'merge' ? 3 / this.vp.k : op === 'extrude' ? 40 / this.vp.k : 10) });
    if (Array.isArray(out)) this.sel = op === 'delete' || op === 'bevel' || op === 'round' || op === 'merge' || op === 'simplify' ? [] : out.length ? out : this.sel;
    this.vp.redraw();
  }

  selectAll() {
    this.sel = (this.data()?.shapes || []).flatMap((sh) => sh.anchors.map((_, i) => [sh.shape, i]));
    this.vp.redraw();
  }

  /// Keys while editing points: Delete, arrows. True when used.
  key(e) {
    const d = this.data();
    if (!d || !this.sel.length) return false;
    if (e.key === 'Delete' || e.key === 'Backspace') { this.edit('delete'); return true; }
    if (e.key.startsWith('Arrow')) {
      const k = e.shiftKey ? 10 : 1;
      call('points_begin', { masks: this.masks });
      this.app.run('points_move', { masks: this.masks, id: d.id, points: this.sel, dx: e.key === 'ArrowLeft' ? -k : e.key === 'ArrowRight' ? k : 0, dy: e.key === 'ArrowUp' ? -k : e.key === 'ArrowDown' ? k : 0 });
      return true;
    }
    return false;
  }

  /// The right-click menu's items: [label, enabled, run].
  menuItems() {
    const n = this.sel.length;
    return [
      [n ? `${n} point${n === 1 ? '' : 's'} selected` : 'No points selected: the tools act on the whole shape', false],
      ['Smooth or sharp', n > 0, () => this.edit('smooth'), 'Double-click'],
      ['Delete points', n > 0, () => this.edit('delete'), 'Delete'],
      ['Subdivide', true, () => this.edit('subdivide')],
      ['Break here', n === 1, () => this.edit('break')],
      ['Join ends', n === 2, () => this.edit('join')],
      ['Extrude', n === 1, () => this.edit('extrude')],
      '-',
      ['Round corners', true, () => this.edit('round')],
      ['Bevel corners', true, () => this.edit('bevel')],
      ['Merge by distance', true, () => this.edit('merge')],
      ['Simplify', true, () => this.edit('simplify')],
      '-',
      ['Select all points', true, () => this.selectAll(), 'Ctrl+A'],
    ];
  }
}
