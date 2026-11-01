// Charts floating over the sheet, drawn as SVG from the numbers the engine
// gives (so they follow the table as it grows and its filters). Click one
// to move it, resize it from its corner, or change it from its bar.
//
// Colours: categorical slots in a fixed order (ember, blue, aqua, violet,
// yellow, magenta), validated for colour-blind separation in light and
// dark (see style.css --series-*). One axis; a legend from two series on,
// with the last value labelled; recessive grid; a tooltip on hover.

import { h, icon, menu, item } from './ui.js';

export const KINDS = [['bar', 'Bars'], ['sideways', 'Bars across'], ['stacked', 'Stacked bars'], ['line', 'Line'], ['area', 'Area'], ['pie', 'Pie'], ['scatter', 'Scatter'], ['heatmap', 'Heatmap'], ['histogram', 'Spread'], ['box', 'Box']];
/// Kinds that show each number as it is (they don't add rows up).
export const EACH_ROW = ['scatter', 'histogram', 'box'];
/// Names that say nothing (columns without a header: "Column B").
const unnamed = (n) => /^Column [A-Z]+$/.test(n);
const NS = 'http://www.w3.org/2000/svg';
const MAX_SERIES = 6;

const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const colour = (k) => css(`--series-${(k % MAX_SERIES) + 1}`);

/// 12,400 → "12.4k"; small numbers as they are.
function compact(n) {
  n += 0;   // no "-0"
  const a = Math.abs(n);
  if (a >= 1e9) return `${(n / 1e9).toLocaleString(undefined, { maximumFractionDigits: 1 })}bn`;
  if (a >= 1e6) return `${(n / 1e6).toLocaleString(undefined, { maximumFractionDigits: 1 })}m`;
  if (a >= 1e4) return `${(n / 1e3).toLocaleString(undefined, { maximumFractionDigits: 1 })}k`;
  return n.toLocaleString(undefined, { maximumFractionDigits: a < 10 ? 2 : 0 });
}
const full = (n) => n.toLocaleString(undefined, { maximumFractionDigits: 2 });

/// One way of writing an axis's numbers, chosen by its largest (10k, 20k… not 5,000 then 10k).
function axisFormat(tk) {
  const top = Math.max(...tk.map(Math.abs));
  const [div, unit] = top >= 1e9 ? [1e9, 'bn'] : top >= 1e6 ? [1e6, 'm'] : top >= 1e4 ? [1e3, 'k'] : [1, ''];
  return (n) => (n === 0 ? '0' : `${(n / div).toLocaleString(undefined, { maximumFractionDigits: div === 1 && top < 10 ? 2 : 1 })}${unit}`);
}

/// Round ticks from `lo` to `hi` (with `zero`, always reaching 0, as a bar's length needs).
function ticks(lo, hi, want = 5, zero = true) {
  if (zero && lo > 0) lo = 0;
  if (zero && hi < 0) hi = 0;
  if (lo === hi) hi = lo + 1;
  const raw = (hi - lo) / want;
  const p = 10 ** Math.floor(Math.log10(raw));
  const m = raw / p;
  const step = p * (m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10);
  const a = Math.floor(lo / step) * step;
  const b = Math.ceil(hi / step) * step;
  const out = [];
  for (let v = a; v <= b + step / 2; v += step) out.push(Math.round(v / step) * step);
  return out;
}

function el(tag, attrs = {}, text) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null) e.setAttribute(k, v);
  if (text != null) e.textContent = text;
  return e;
}

/// A bar's shape: square at the baseline, 4px rounded at its far end.
function barPath(x, y0, w, y1) {
  const up = y1 < y0;
  const len = Math.abs(y1 - y0);
  const r = Math.min(4, w / 2, len);
  if (len < 0.5) return '';
  if (up) return `M${x},${y0}V${y1 + r}Q${x},${y1} ${x + r},${y1}H${x + w - r}Q${x + w},${y1} ${x + w},${y1 + r}V${y0}Z`;
  return `M${x},${y0}V${y1 - r}Q${x},${y1} ${x + r},${y1}H${x + w - r}Q${x + w},${y1} ${x + w},${y1 - r}V${y0}Z`;
}

export class Charts {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet, host }
    this.layer = h('div', { class: 'chart-layer' });
    d.host.append(this.layer);
    this.list = [];
    this.nodes = new Map();
    this.selected = null;
    this.tip = h('div', { class: 'chart-tip', role: 'tooltip', hidden: true });
    document.body.append(this.tip);
    document.addEventListener('pointerdown', (e) => {
      if (this.selected != null && !e.target.closest?.('.chart, .popup, .menu, .modal, .chartpanel')) this.select(null);
    });
    document.addEventListener('keydown', (e) => {
      if (this.selected == null || e.target.closest?.('input, textarea')) return;
      if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); e.stopPropagation(); this.remove(this.selected); }
      if (e.key === 'Escape') this.select(null);
    }, true);
  }

  /// The charts (and pictures from an Excel file) on the sheet shown, read again (after any change).
  refresh() {
    try { this.list = this.d.call('charts', { sheet: this.d.sheet() }); } catch { this.list = []; }
    this.pictures();
    const ids = new Set(this.list.map((c) => c.id));
    for (const [id, n] of this.nodes) if (!ids.has(id)) { n.remove(); this.nodes.delete(id); }
    for (const c of this.list) this.draw(c);
    if (this.selected != null && !ids.has(this.selected)) this.selected = null;
    // A tooltip over a chart that's gone (or redrawn) goes too.
    this.tip.hidden = true;
    this.place();
  }

  /// Pictures from the Excel file: shown where they were (kept as they are, not edited here).
  pictures() {
    const sheet = this.d.sheet();
    // By book and sheet (books number their sheets alike).
    const key = `${this.d.tab?.()}:${sheet}`;
    if (this.picKey !== key) {
      this.picKey = key;
      for (const p of this.pics || []) p.el.remove();
      let list = [];
      try { list = this.d.call('pictures', { sheet }); } catch { list = []; }
      this.pics = list.filter((p) => p.mime).map((p) => {
        const el = h('img', { class: 'picture', src: `data:${p.mime};base64,${p.data}`, alt: '', title: 'A picture from the .xlsx file (kept as it is)', draggable: 'false' });
        this.layer.append(el);
        return { ...p, el };
      });
    }
  }

  /// Keeps each chart over its cell as the sheet scrolls.
  place() {
    const g = this.d.grid;
    for (const p of this.pics || []) {
      p.el.style.left = `${g.x(p.col) + p.dx}px`;
      p.el.style.top = `${g.y(p.row) + p.dy}px`;
      p.el.style.width = `${p.w}px`;
      p.el.style.height = `${p.h}px`;
    }
    for (const c of this.list) {
      const n = this.nodes.get(c.id);
      if (!n) continue;
      let left = g.x(c.col) + c.dx;
      // While the chart panel is open, the chart it changes moves out from under it.
      if (this.shove && c.id === this.selected) {
        const room = g.host.clientWidth - this.shove - 16;
        if (left + c.w > room) left = Math.max(56, room - c.w);
      }
      n.style.left = `${left}px`;
      n.style.top = `${g.y(c.row) + c.dy}px`;
      n.style.width = `${c.w}px`;
      n.style.height = `${c.h}px`;
      // Near the top its bar goes under it (above, the column letters would cover it).
      n.classList.toggle('bar-below', g.y(c.row) + c.dy < 76);
    }
  }

  /// Insert › Chart: one for the selection, beside it, with the chart panel open on it.
  add() {
    const g = this.d.grid;
    const rg = g.range();
    const t = g.tableAt(rg.top, rg.left);
    const right = t ? t.right : rg.right;
    const top = t ? t.top : rg.top;
    const c = this.d.run('add_chart', { sheet: this.d.sheet(), range: rg, row: top, col: right + 2 });
    if (c) { this.refresh(); this.select(c.id); this.d.edit?.(c.id); }
  }

  /// Copy button: the chart to paste elsewhere.
  async copy(id) {
    const o = this.d.copy?.(id);
    if (!o) return;
    try { await navigator.clipboard.writeText(o.text); } catch { /* Calc's own paste still has it */ }
    import('./ui.js').then(({ toast }) => toast('Copied: click a cell on any sheet (or a report) and paste (Ctrl+V)'));
  }

  /// A copy of chart `c` with its top left at a cell of the sheet shown.
  paste(c, row, col) {
    const got = this.d.run('paste_chart', { chart: c, sheet: this.d.sheet(), row, col });
    if (got) { this.refresh(); this.select(got.id); }
    return got;
  }

  remove(id) {
    this.d.run('remove_chart', { id });
    this.selected = null;
    this.tip.hidden = true;
    this.d.onRemoved?.(id);
    this.refresh();
  }

  save(c) {
    this.d.run('set_chart', { chart: c });
    this.refresh();
  }

  select(id) {
    this.selected = id;
    for (const [k, n] of this.nodes) n.classList.toggle('on', k === id);
  }

  draw(c) {
    let n = this.nodes.get(c.id);
    if (!n) {
      n = h('figure', { class: 'chart', tabindex: '0' });
      this.layer.append(n);
      this.nodes.set(c.id, n);
      this.bind(n, c.id);
    }
    n.classList.toggle('on', this.selected === c.id);
    let data;
    try { data = this.d.call('chart_data', { id: c.id }); } catch (e) {
      n.replaceChildren(h('div', { class: 'chart-empty' }, String(e.message || e)));
      return;
    }
    const series = data.series.slice(0, MAX_SERIES);
    const legend = (series.length > 1 && c.kind !== 'heatmap') || c.kind === 'pie'
      ? h('div', { class: 'legend' }, (c.kind === 'pie' ? pieSlices(data).map((s) => s.name) : series.map((s) => s.name)).map((name, k) => h('span', {}, h('i', { style: { background: colour(k) } }), name)))
      : null;
    const title = c.title || (series.length === 1 && !unnamed(series[0].name) ? series[0].name : '');
    const head = h('div', { class: 'chart-head' }, title ? h('b', {}, title) : null, legend);
    const bar = h('div', { class: 'chart-bar' },
      h('button', { class: 'btn small', onClick: (e) => this.kindMenu(c, e) }, KINDS.find((k) => k[0] === c.kind)?.[1] || c.kind, h('span', { html: icon('down', 14) })),
      h('button', { class: 'btn small', title: 'Kind, columns, adding up, title', onClick: () => this.d.edit?.(c.id) }, 'Change…'),
      h('button', { class: 'btn small', title: 'Copy (Ctrl+C): paste on any sheet or report of this book', onClick: () => this.copy(c.id) }, 'Copy'),
      h('button', { class: 'btn small plain', 'aria-label': 'Delete chart', onClick: () => this.remove(c.id) }, 'Delete'));
    const box = h('div', { class: 'chart-body' });
    const grip = h('span', { class: 'grip', 'aria-hidden': 'true' });
    n.replaceChildren(bar, head, box, grip);
    n.setAttribute('aria-label', `Chart: ${c.title || series.map((s) => s.name).join(', ')}`);
    // Drawn once laid out (it needs its size).
    requestAnimationFrame(() => this.plot(box, c, data, series));
  }

  bind(n, id) {
    n.addEventListener('pointerdown', (e) => {
      if (e.target.closest('button')) return;
      e.stopPropagation();
      this.select(id);
      const c = this.list.find((x) => x.id === id);
      if (!c) return;
      const g = this.d.grid;
      const resizing = e.target.classList.contains('grip');
      const start = { x: e.clientX, y: e.clientY, w: c.w, h: c.h, left: n.offsetLeft, top: g.y(c.row) + c.dy };
      n.setPointerCapture(e.pointerId);
      let moved = false;
      const move = (ev) => {
        const dx = ev.clientX - start.x; const dy = ev.clientY - start.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
        if (resizing) {
          n.style.width = `${Math.max(240, start.w + dx)}px`;
          n.style.height = `${Math.max(160, start.h + dy)}px`;
        } else {
          n.style.left = `${start.left + dx}px`;
          n.style.top = `${start.top + dy}px`;
        }
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
        const dx = ev.clientX - start.x; const dy = ev.clientY - start.y;
        if (resizing) {
          this.save({ ...c, w: Math.round(Math.max(240, start.w + dx)), h: Math.round(Math.max(160, start.h + dy)) });
        } else {
          // Anchored to the cell its top left is over now.
          const hit = g.hit(Math.max(47, start.left + dx), Math.max(27, start.top + dy));
          const r = hit.r || c.row; const col = hit.c || c.col;
          this.save({ ...c, row: r, col, dx: Math.round(start.left + dx - g.x(col)), dy: Math.round(start.top + dy - g.y(r)) });
        }
      };
      n.addEventListener('pointermove', move);
      n.addEventListener('pointerup', up);
      n.addEventListener('pointercancel', cancel);
    });
  }

  kindMenu(c, e) {
    const r = e.currentTarget.getBoundingClientRect();
    menu(KINDS.map(([k, label]) => item(label, { checked: c.kind === k ? true : null, run: () => this.save({ ...c, kind: k }) })), r.left, r.bottom + 4);
  }


  // ----- Tooltips -----

  hover(target, text) {
    target.addEventListener('pointerenter', (e) => {
      this.tip.textContent = text;
      this.tip.hidden = false;
      this.moveTip(e);
    });
    target.addEventListener('pointermove', (e) => this.moveTip(e));
    target.addEventListener('pointerleave', () => { this.tip.hidden = true; });
  }

  moveTip(e) {
    const r = this.tip.getBoundingClientRect();
    this.tip.style.left = `${Math.min(innerWidth - r.width - 8, e.clientX + 12)}px`;
    this.tip.style.top = `${Math.max(8, e.clientY - r.height - 10)}px`;
  }

  // ----- Drawing -----

  plot(box, c, data, series) {
    const W = box.clientWidth || c.w - 32;
    const H = box.clientHeight || c.h - 70;
    const svg = el('svg', { width: W, height: H, viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': `${c.kind} chart of ${series.map((s) => s.name).join(', ')}` });
    box.replaceChildren(svg);
    if (!series.length || !data.labels.length) {
      svg.append(el('text', { x: W / 2, y: H / 2, 'text-anchor': 'middle', fill: css('--muted'), 'font-size': 13 }, 'No numbers to show'));
      return;
    }
    if (c.kind === 'pie') return this.pie(svg, W, H, data);
    if (c.kind === 'histogram') return this.histogram(svg, W, H, series[0]);
    if (c.kind === 'box') return this.boxes(svg, W, H, series);
    if (c.kind === 'scatter') return this.scatter(svg, W, H, data, series.slice(0, 3));
    if (c.kind === 'sideways') return this.sideways(svg, W, H, data, series);
    if (c.kind === 'heatmap') return this.heatmap(svg, W, H, data, data.series);
    return this.axes(svg, W, H, c.kind, data, series);
  }

  /// Bars, lines and areas: points along the bottom, one value axis.
  axes(svg, W, H, kind, data, series) {
    const stacked = kind === 'stacked';
    // Stacked: each point's pile above zero (and below, for negatives).
    const piles = data.labels.map((_, i) => series.reduce((a, s) => { const v = s.values[i]; if (v != null) a[v >= 0 ? 0 : 1] += v; return a; }, [0, 0]));
    const all = stacked ? piles.flat() : series.flatMap((s) => s.values.filter((v) => v != null));
    const tk = ticks(Math.min(...all), Math.max(...all));
    const fmt = axisFormat(tk);
    const left = 8 + Math.max(...tk.map((t) => fmt(t).length)) * 6.6;
    const right = 44; const top = 18; const bottom = 22;
    const pw = W - left - right; const ph = H - top - bottom;
    const y = (v) => top + ph - ((v - tk[0]) / (tk[tk.length - 1] - tk[0])) * ph;
    const muted = css('--muted');
    for (const t of tk) {
      svg.append(el('line', { x1: left, x2: left + pw, y1: y(t), y2: y(t), stroke: t === 0 ? css('--line-2') : css('--soft-2'), 'stroke-width': 1 }));
      svg.append(el('text', { x: left - 6, y: y(t) + 4, 'text-anchor': 'end', 'font-size': 11, fill: muted, 'font-family': 'Figtree' }, fmt(t)));
    }
    const n = data.labels.length;
    const band = pw / n;
    // Point names, thinned so they don't overlap.
    const every = Math.max(1, Math.ceil(n / Math.max(1, Math.floor(pw / 46))));
    data.labels.forEach((l, i) => {
      if (i % every) return;
      const text = l.length > 9 && band < 70 ? `${l.slice(0, 3)}` : l;
      svg.append(el('text', { x: left + band * (i + 0.5), y: H - 6, 'text-anchor': 'middle', 'font-size': 11, fill: muted, 'font-family': 'Figtree' }, text));
    });
    const zero = y(Math.max(tk[0], Math.min(0, tk[tk.length - 1])));
    const per = stacked ? 1 : series.length;
    const inner = Math.min(band * 0.72, 22 * per + 2 * (per - 1));
    const bw = (inner - 2 * (per - 1)) / per;
    const barX = (i, k) => left + band * i + (band - inner) / 2 + (stacked ? 0 : k * (bw + 2));
    if (stacked) {
      data.labels.forEach((l, i) => {
        let up = 0; let down = 0;
        series.forEach((s, k) => {
          const v = s.values[i];
          if (v == null || v === 0) return;
          const from = v >= 0 ? up : down;
          const to = from + v;
          if (v >= 0) up = to; else down = to;
          // Square joins with a 2px gap of the surface between pieces; the end of the pile rounded.
          const top = v >= 0 ? up === piles[i][0] : down === piles[i][1];
          const y0 = y(from) + (from === 0 ? 0 : (v >= 0 ? -1 : 1));
          const y1 = y(to);
          const d = top ? barPath(barX(i, k), y0, bw, y1) : `M${barX(i, k)},${y0}V${y1}H${barX(i, k) + bw}V${y0}Z`;
          const p = el('path', { d, fill: colour(k), stroke: from === 0 ? null : css('--sheet'), 'stroke-width': from === 0 ? null : 0 });
          if (from !== 0) { const gap = el('rect', { x: barX(i, k), y: Math.min(y(from) - 1, y(from) + 1), width: bw, height: 2, fill: css('--sheet') }); svg.append(p, gap); } else svg.append(p);
          this.hover(p, `${l} · ${s.name}: ${full(v)} (all: ${full(piles[i][0] + piles[i][1])})`);
        });
      });
      // The last pile's total, over it.
      const i = data.labels.length - 1;
      const tot = piles[i][0] + piles[i][1];
      svg.append(el('text', { x: barX(i, 0) + bw / 2, y: y(piles[i][0]) - 5, 'text-anchor': 'middle', 'font-size': 11.5, 'font-weight': 700, fill: css('--ink'), 'font-family': 'Figtree' }, compact(tot)));
      return;
    }
    if (kind === 'bar') {
      series.forEach((s, k) => {
        s.values.forEach((v, i) => {
          if (v == null) return;
          const p = el('path', { d: barPath(barX(i, k), zero, bw, y(v)), fill: colour(k) });
          this.hover(p, `${data.labels[i]} · ${s.name}: ${full(v)}`);
          svg.append(p);
        });
      });
    } else {
      series.forEach((s, k) => {
        const pts = s.values.map((v, i) => (v == null ? null : [left + band * (i + 0.5), y(v)]));
        let d = ''; let started = false;
        for (const p of pts) {
          if (!p) { started = false; continue; }
          d += `${started ? 'L' : 'M'}${p[0]},${p[1]}`;
          started = true;
        }
        if (kind === 'area') {
          const first = pts.find(Boolean); const last = [...pts].reverse().find(Boolean);
          if (first) svg.append(el('path', { d: `${d}L${last[0]},${zero}L${first[0]},${zero}Z`, fill: colour(k), 'fill-opacity': 0.14 }));
        }
        svg.append(el('path', { d, fill: 'none', stroke: colour(k), 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }));
        pts.forEach((p, i) => {
          if (!p) return;
          // Big, invisible targets; a dot shows on hover.
          const hit = el('circle', { cx: p[0], cy: p[1], r: Math.max(6, Math.min(12, band / 2)), fill: 'transparent' });
          const dot = el('circle', { cx: p[0], cy: p[1], r: 4, fill: colour(k), stroke: css('--sheet'), 'stroke-width': 2, opacity: 0 });
          hit.addEventListener('pointerenter', () => dot.setAttribute('opacity', 1));
          hit.addEventListener('pointerleave', () => dot.setAttribute('opacity', 0));
          this.hover(hit, `${data.labels[i]} · ${s.name}: ${full(s.values[i])}`);
          svg.append(dot, hit);
        });
      });
    }
    // The last value of each series, labelled at its end.
    series.forEach((s, k) => {
      const i = s.values.map((v, j) => (v == null ? -1 : j)).filter((j) => j >= 0).pop();
      if (i == null) return;
      const v = s.values[i];
      // Over its bar, or just right of the line's end.
      const bar = kind === 'bar';
      const lx = bar ? barX(i, k) + bw / 2 : left + band * (i + 0.5) + 6;
      const ly = bar ? (v >= 0 ? y(v) - 5 : y(v) + 13) : y(v) + 4;
      svg.append(el('text', { x: lx, y: ly, 'text-anchor': bar ? 'middle' : 'start', 'font-size': 11.5, 'font-weight': 700, fill: css('--ink'), 'font-family': 'Figtree' }, compact(v)));
    });
  }

  /// Bars across: names down the left, values along the bottom, each bar's value at its end.
  sideways(svg, W, H, data, series) {
    const all = series.flatMap((s) => s.values.filter((v) => v != null));
    const tk = ticks(Math.min(...all), Math.max(...all));
    const fmt = axisFormat(tk);
    const muted = css('--muted');
    const names = data.labels.map((l) => (l.length > 18 ? `${l.slice(0, 17)}…` : l));
    const left = 10 + Math.min(150, Math.max(...names.map((l) => l.length)) * 6.6);
    const right = 40; const top = 4; const bottom = 22;
    const pw = W - left - right; const ph = H - top - bottom;
    const x = (v) => left + ((v - tk[0]) / (tk[tk.length - 1] - tk[0])) * pw;
    for (const t of tk) {
      svg.append(el('line', { x1: x(t), x2: x(t), y1: top, y2: top + ph, stroke: t === 0 ? css('--line-2') : css('--soft-2') }));
      svg.append(el('text', { x: x(t), y: H - 6, 'text-anchor': 'middle', 'font-size': 11, fill: muted, 'font-family': 'Figtree' }, fmt(t)));
    }
    const n = data.labels.length;
    const band = ph / n;
    const inner = Math.min(band * 0.72, 22 * series.length + 2 * (series.length - 1));
    const bh = (inner - 2 * (series.length - 1)) / series.length;
    const zero = x(Math.max(tk[0], Math.min(0, tk[tk.length - 1])));
    const labelled = n * series.length <= 24;
    names.forEach((l, i) => svg.append(el('text', { x: left - 8, y: top + band * (i + 0.5) + 4, 'text-anchor': 'end', 'font-size': 11.5, fill: css('--ink-2'), 'font-family': 'Figtree' }, l)));
    series.forEach((s, k) => {
      s.values.forEach((v, i) => {
        if (v == null) return;
        const by = top + band * i + (band - inner) / 2 + k * (bh + 2);
        const len = Math.abs(x(v) - zero);
        if (len < 0.5) return;
        const r = Math.min(4, bh / 2, len);
        const x0 = zero; const x1 = x(v);
        const d = x1 >= x0
          ? `M${x0},${by}H${x1 - r}Q${x1},${by} ${x1},${by + r}V${by + bh - r}Q${x1},${by + bh} ${x1 - r},${by + bh}H${x0}Z`
          : `M${x0},${by}H${x1 + r}Q${x1},${by} ${x1},${by + r}V${by + bh - r}Q${x1},${by + bh} ${x1 + r},${by + bh}H${x0}Z`;
        const p = el('path', { d, fill: colour(k) });
        this.hover(p, `${data.labels[i]} · ${s.name}: ${full(v)}`);
        svg.append(p);
        if (labelled) svg.append(el('text', { x: x1 + (v >= 0 ? 5 : -5), y: by + bh / 2 + 4, 'text-anchor': v >= 0 ? 'start' : 'end', 'font-size': 11, 'font-weight': 600, fill: css('--ink'), 'font-family': 'Figtree' }, compact(v)));
      });
    });
  }

  /// A grid of squares: points along, series down, darker for bigger (one colour).
  heatmap(svg, W, H, data, series) {
    const all = series.flatMap((s) => s.values.filter((v) => v != null));
    if (!all.length) return;
    const lo = Math.min(...all); const hi = Math.max(...all);
    const muted = css('--muted');
    const rowNames = series.map((s) => (s.name.length > 16 ? `${s.name.slice(0, 15)}…` : s.name));
    const left = 10 + Math.min(130, Math.max(...rowNames.map((l) => l.length)) * 6.6);
    const top = 4; const bottom = 22; const right = 4;
    const cw = (W - left - right) / data.labels.length; const ch = (H - top - bottom) / series.length;
    const every = Math.max(1, Math.ceil(data.labels.length / Math.max(1, Math.floor((W - left) / 46))));
    data.labels.forEach((l, i) => {
      if (i % every) return;
      const text = l.length > 9 && cw < 70 ? l.slice(0, 3) : l;
      svg.append(el('text', { x: left + cw * (i + 0.5), y: H - 6, 'text-anchor': 'middle', 'font-size': 11, fill: muted, 'font-family': 'Figtree' }, text));
    });
    rowNames.forEach((n, k) => svg.append(el('text', { x: left - 8, y: top + ch * (k + 0.5) + 4, 'text-anchor': 'end', 'font-size': 11.5, fill: css('--ink-2'), 'font-family': 'Figtree' }, n)));
    const base = colour(0);
    series.forEach((s, k) => {
      s.values.forEach((v, i) => {
        const x = left + cw * i; const yy = top + ch * k;
        const share = v == null ? 0 : hi === lo ? 1 : (v - lo) / (hi - lo);
        // 2px of the surface between squares.
        const r = el('rect', { x: x + 1, y: yy + 1, width: Math.max(1, cw - 2), height: Math.max(1, ch - 2), rx: 3, fill: v == null ? css('--soft') : base, 'fill-opacity': v == null ? 1 : 0.1 + 0.9 * share });
        this.hover(r, `${data.labels[i]} · ${s.name}: ${v == null ? 'nothing' : full(v)}`);
        svg.append(r);
        if (v != null && cw >= 34 && ch >= 18) {
          svg.append(el('text', { x: x + cw / 2, y: yy + ch / 2 + 4, 'text-anchor': 'middle', 'font-size': 11, 'font-weight': 600, fill: share > 0.55 ? '#fff' : css('--ink'), 'font-family': 'Figtree', 'pointer-events': 'none' }, compact(v)));
        }
      });
    });
  }

  pie(svg, W, H, data) {
    const slices = pieSlices(data);
    const total = slices.reduce((a, s) => a + s.value, 0);
    if (!total) return;
    const cx = W / 2; const cy = H / 2; const R = Math.min(W, H) / 2 - 8; const r = R * 0.58;
    let a = -Math.PI / 2;
    slices.forEach((s, k) => {
      const da = (s.value / total) * Math.PI * 2;
      const large = da > Math.PI ? 1 : 0;
      const p = (rad, ang) => `${cx + rad * Math.cos(ang)},${cy + rad * Math.sin(ang)}`;
      const d = slices.length === 1
        ? `M${cx - R},${cy}A${R},${R} 0 1 1 ${cx + R},${cy}A${R},${R} 0 1 1 ${cx - R},${cy}M${cx - r},${cy}A${r},${r} 0 1 0 ${cx + r},${cy}A${r},${r} 0 1 0 ${cx - r},${cy}Z`
        : `M${p(R, a)}A${R},${R} 0 ${large} 1 ${p(R, a + da)}L${p(r, a + da)}A${r},${r} 0 ${large} 0 ${p(r, a)}Z`;
      // 2px of the surface between slices.
      const path = el('path', { d, fill: colour(k), stroke: css('--sheet'), 'stroke-width': 2, 'fill-rule': 'evenodd' });
      this.hover(path, `${s.name}: ${full(s.value)} (${Math.round((s.value / total) * 100)}%)`);
      svg.append(path);
      a += da;
    });
    svg.append(el('text', { x: cx, y: cy + 5, 'text-anchor': 'middle', 'font-size': 15, 'font-weight': 700, fill: css('--ink'), 'font-family': 'Figtree' }, compact(total)));
  }

  histogram(svg, W, H, s) {
    const vals = s.values.filter((v) => v != null).sort((x, y) => x - y);
    if (!vals.length) return;
    const lo = vals[0]; const hi = vals[vals.length - 1];
    const edges = ticks(lo, hi, Math.min(10, Math.max(3, Math.round(Math.sqrt(vals.length) * 2))), false);
    const counts = edges.slice(0, -1).map((e, i) => vals.filter((v) => v >= e && (i === edges.length - 2 ? v <= edges[i + 1] : v < edges[i + 1])).length);
    const left = 32; const bottom = 22; const top = 8;
    const pw = W - left - 8; const ph = H - top - bottom;
    const most = Math.max(...counts, 1);
    const ct = ticks(0, most, 4).filter((t) => Number.isInteger(t));
    const y = (v) => top + ph - (v / ct[ct.length - 1]) * ph;
    for (const t of ct) {
      svg.append(el('line', { x1: left, x2: left + pw, y1: y(t), y2: y(t), stroke: t === 0 ? css('--line-2') : css('--soft-2') }));
      svg.append(el('text', { x: left - 6, y: y(t) + 4, 'text-anchor': 'end', 'font-size': 11, fill: css('--muted'), 'font-family': 'Figtree' }, String(t)));
    }
    const bw = pw / counts.length;
    counts.forEach((n, i) => {
      const p = el('path', { d: barPath(left + i * bw + 1, y(0), bw - 2, y(n)), fill: colour(0) });
      this.hover(p, `${full(edges[i])} to ${full(edges[i + 1])}: ${n} ${n === 1 ? 'number' : 'numbers'}`);
      svg.append(p);
    });
    for (const [i, e] of [[0, edges[0]], [counts.length, edges[edges.length - 1]]]) {
      svg.append(el('text', { x: left + i * bw, y: H - 6, 'text-anchor': i ? 'end' : 'start', 'font-size': 11, fill: css('--muted'), 'font-family': 'Figtree' }, compact(e)));
    }
  }

  boxes(svg, W, H, series) {
    const stats = series.map((s) => {
      const v = s.values.filter((x) => x != null).sort((a, b) => a - b);
      const q = (p) => { if (!v.length) return 0; const k = p * (v.length - 1); const lo = Math.floor(k); return v[lo] + (v[Math.ceil(k)] - v[lo]) * (k - lo); };
      return { name: s.name, min: v[0], q1: q(0.25), med: q(0.5), q3: q(0.75), max: v[v.length - 1] };
    }).filter((s) => s.min != null);
    if (!stats.length) return;
    const tk = ticks(Math.min(...stats.map((s) => s.min)), Math.max(...stats.map((s) => s.max)), 5, false);
    const left = 8 + Math.max(...stats.map((s) => s.name.length)) * 6.5; const right = 16;
    const pw = W - left - right; const ph = H - 24;
    const x = (v) => left + ((v - tk[0]) / (tk[tk.length - 1] - tk[0])) * pw;
    const fmt = axisFormat(tk);
    for (const t of tk) {
      svg.append(el('line', { x1: x(t), x2: x(t), y1: 4, y2: ph, stroke: css('--soft-2') }));
      svg.append(el('text', { x: x(t), y: H - 6, 'text-anchor': 'middle', 'font-size': 11, fill: css('--muted'), 'font-family': 'Figtree' }, fmt(t)));
    }
    const band = ph / stats.length;
    stats.forEach((s, k) => {
      const cy = 4 + band * (k + 0.5); const bh = Math.min(28, band * 0.5);
      svg.append(el('text', { x: left - 8, y: cy + 4, 'text-anchor': 'end', 'font-size': 12, fill: css('--ink-2'), 'font-family': 'Figtree' }, s.name));
      svg.append(el('line', { x1: x(s.min), x2: x(s.max), y1: cy, y2: cy, stroke: colour(k), 'stroke-width': 2 }));
      const rect = el('rect', { x: x(s.q1), y: cy - bh / 2, width: Math.max(2, x(s.q3) - x(s.q1)), height: bh, rx: 4, fill: colour(k), 'fill-opacity': 0.22, stroke: colour(k), 'stroke-width': 2 });
      this.hover(rect, `${s.name}: smallest ${full(s.min)}, middle half ${full(s.q1)} to ${full(s.q3)}, middle value ${full(s.med)}, largest ${full(s.max)}`);
      svg.append(rect, el('line', { x1: x(s.med), x2: x(s.med), y1: cy - bh / 2, y2: cy + bh / 2, stroke: colour(k), 'stroke-width': 2.5 }));
    });
  }

  scatter(svg, W, H, data, series) {
    if (!data.x) {
      svg.append(el('text', { x: W / 2, y: H / 2, 'text-anchor': 'middle', fill: css('--muted'), 'font-size': 13 }, 'A scatter chart needs numbers along the bottom: pick a number column first'));
      return;
    }
    const xs = data.x.filter((v) => v != null);
    const ys = series.flatMap((s) => s.values.filter((v) => v != null));
    const tx = ticks(Math.min(...xs), Math.max(...xs), 5, false); const ty = ticks(Math.min(...ys), Math.max(...ys), 5, false);
    const fx = axisFormat(tx); const fy = axisFormat(ty);
    const left = 8 + Math.max(...ty.map((t) => fy(t).length)) * 6.6; const bottom = 22; const top = 8; const right = 12;
    const pw = W - left - right; const ph = H - top - bottom;
    const X = (v) => left + ((v - tx[0]) / (tx[tx.length - 1] - tx[0])) * pw;
    const Y = (v) => top + ph - ((v - ty[0]) / (ty[ty.length - 1] - ty[0])) * ph;
    for (const t of ty) {
      svg.append(el('line', { x1: left, x2: left + pw, y1: Y(t), y2: Y(t), stroke: css('--soft-2') }));
      svg.append(el('text', { x: left - 6, y: Y(t) + 4, 'text-anchor': 'end', 'font-size': 11, fill: css('--muted'), 'font-family': 'Figtree' }, fy(t)));
    }
    for (const t of tx) svg.append(el('text', { x: X(t), y: H - 6, 'text-anchor': 'middle', 'font-size': 11, fill: css('--muted'), 'font-family': 'Figtree' }, fx(t)));
    series.forEach((s, k) => {
      s.values.forEach((v, i) => {
        const xv = data.x[i];
        if (v == null || xv == null) return;
        const dot = el('circle', { cx: X(xv), cy: Y(v), r: 4.5, fill: colour(k), stroke: css('--sheet'), 'stroke-width': 2 });
        this.hover(dot, `${data.labels[i]}: ${full(xv)}, ${s.name} ${full(v)}`);
        svg.append(dot);
      });
    });
  }
}

/// A pie's slices: the first series by point, the smallest folded into "Other" past six.
function pieSlices(data) {
  const s = data.series[0];
  const all = data.labels.map((name, i) => ({ name, value: Math.max(0, s.values[i] || 0) })).filter((x) => x.value > 0);
  all.sort((a, b) => b.value - a.value);
  if (all.length <= MAX_SERIES) return all;
  const top = all.slice(0, MAX_SERIES - 1);
  top.push({ name: 'Other', value: all.slice(MAX_SERIES - 1).reduce((a, x) => a + x.value, 0) });
  return top;
}
