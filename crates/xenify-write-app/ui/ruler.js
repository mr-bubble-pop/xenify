// The ruler above the pages: the page's width and its margins (the white
// part is where text goes), with the caret paragraph's indents. Drag the
// triangles to indent (top: first line; bottom left: the whole paragraph;
// bottom right: from the right), or the edges of the white part to change
// the margins.

// Points per unit, and how many ticks between numbers.
const UNITS = { cm: [72 / 2.54, 2], in: [72, 8], pt: [72, 6] };
const SNAP = { cm: 72 / 2.54 / 4, in: 72 / 16, pt: 6 };

export class Ruler {
  constructor(el) {
    this.el = el;
    this.key = '';
    this.units = 'cm';
    this.onIndent = null; // (patch in points) → Promise
    this.onMargins = null; // ({left, right} in points) → Promise
  }

  update({ left, width, body, zoom, indents, pageWidth }) {
    const ind = indents || [0, 0, 0];
    const key = [left, width, body[0], body[1], zoom, ...ind, this.units].map((v) => (typeof v === 'number' ? v.toFixed(1) : v)).join();
    if (key === this.key || this.dragging) return;
    this.key = key;
    this.geom = { left, width, body, zoom, ind, pageWidth };
    const track = document.createElement('div');
    track.className = 'ruler-track';
    Object.assign(track.style, { left: `${left}px`, width: `${width}px` });
    const b = document.createElement('div');
    b.className = 'ruler-body';
    Object.assign(b.style, { left: `${body[0]}px`, width: `${body[1]}px` });
    track.append(b);
    // Numbers count from the left margin, like a printed ruler laid on the text.
    const [unit, ticks] = UNITS[this.units] || UNITS.cm;
    const step = unit * zoom, tick = step / ticks;
    const label = (n) => (this.units === 'pt' ? n * 72 : n);
    for (let x = body[0] % tick; x < width; x += tick) {
      const n = Math.round((x - body[0]) / tick);
      const fromMargin = n / ticks;
      const whole = n % ticks === 0;
      const t = document.createElement('div');
      t.className = 'ruler-tick';
      Object.assign(t.style, { left: `${x}px`, height: whole ? '7px' : '4px' });
      track.append(t);
      if (whole && fromMargin > 0 && x < body[0] + body[1] + 1) {
        const num = document.createElement('div');
        num.className = 'ruler-num';
        num.style.left = `${x}px`;
        num.textContent = label(fromMargin);
        track.append(num);
      }
    }
    // Margin edges (drag to change the page's margins).
    for (const side of ['left', 'right']) {
      const e = document.createElement('div');
      e.className = 'ruler-edge';
      e.title = side === 'left' ? 'Left margin' : 'Right margin';
      e.style.left = `${(side === 'left' ? body[0] : body[0] + body[1]) - 4}px`;
      e.addEventListener('pointerdown', (ev) => this.drag(ev, e, (dx) => {
        const pt = dx / zoom;
        const l = side === 'left' ? body[0] / zoom + pt : body[0] / zoom;
        const r = (width - body[0] - body[1]) / zoom - (side === 'right' ? pt : 0);
        return { left: Math.max(0, this.snap(l)), right: Math.max(0, this.snap(r)) };
      }, (v) => this.onMargins?.(v), side === 'left' ? 'margin-left' : 'margin-right'));
      track.append(e);
    }
    // Indents: first line (top), left (bottom left), right (bottom right).
    const z = zoom, x0 = body[0], x1 = body[0] + body[1];
    const mk = (cls, x, title, value) => {
      const m = document.createElement('div');
      m.className = `ruler-mark ${cls}`;
      m.title = title;
      m.style.left = `${x - 5}px`;
      m.addEventListener('pointerdown', (ev) => this.drag(ev, m, (dx) => value(dx / z), (patch) => this.onIndent?.(patch)));
      track.append(m);
    };
    mk('top', x0 + (ind[0] + ind[2]) * z, 'First line', (d) => ({ first_line: this.snap(ind[2] + d) }));
    mk('bottom', x0 + ind[0] * z, 'Left indent', (d) => ({ indent_left: this.snap(ind[0] + d), first_line: ind[2] }));
    mk('bottom', x1 - ind[1] * z, 'Right indent', (d) => ({ indent_right: this.snap(ind[1] - d) }));
    this.el.replaceChildren(track);
  }

  snap(pt) {
    const s = SNAP[this.units] || SNAP.cm;
    return Math.round(pt / s) * s;
  }

  /// Drags `el` sideways; `value(dx)` turns the distance into what to apply.
  drag(ev, el, value, apply) {
    ev.preventDefault();
    ev.stopPropagation();
    el.setPointerCapture(ev.pointerId);
    this.dragging = true;
    const x0 = ev.clientX, left0 = parseFloat(el.style.left);
    let last = null;
    const move = (e) => {
      const dx = e.clientX - x0;
      el.style.left = `${left0 + dx}px`;
      last = value(dx);
    };
    const up = async () => {
      el.removeEventListener('pointermove', move);
      this.dragging = false;
      this.key = '';
      if (last) await apply(last);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up, { once: true });
  }
}
