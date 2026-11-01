// The easing editor: a curve with two handles to drag (cubic Bézier), a
// spring (stiffness, damping) or steps (how many, where the jump is), with
// the ready-made easings to start from. Resolves with the easing as stored.

export function curveDialog(app, h, start, { title = 'Easing', presets = [], label = (n) => n } = {}) {
  let e = JSON.parse(JSON.stringify(start || { type: 'bezier', x1: 0.25, y1: 0.1, x2: 0.25, y2: 1 }));
  const kind = () => (e.type === 'spring' ? 'spring' : e.type === 'steps' ? 'steps' : 'curve');
  const body = h('div', { class: 'curve-edit', style: 'display: flex; flex-direction: column; gap: 12px' });
  const W = 300, H = 300, PAD = 40;
  const canvas = h('canvas', { width: String(W * devicePixelRatio), height: String(H * devicePixelRatio), style: `width: ${W}px; height: ${H}px; border: 1px solid var(--line); border-radius: 8px; background: var(--card); touch-action: none; align-self: center`, 'aria-label': 'The curve: drag the two round handles' });
  const g = canvas.getContext('2d');
  const sx = (x) => PAD + x * (W - 2 * PAD), sy = (y) => H - PAD - y * (H - 2 * PAD - 60) - 30;
  const ix = (px) => (px - PAD) / (W - 2 * PAD), iy = (py) => (H - PAD - 30 - py) / (H - 2 * PAD - 60);
  // The value at progress x (0..1), for drawing: as the engine eases, else worked out here.
  let sampled = null, sampledFor = '';
  function at(x) {
    const key = JSON.stringify(e);
    if (key !== sampledFor) { sampledFor = key; try { sampled = call('ease_sample', { easing: e, n: 200 }); } catch { sampled = null; } }
    if (sampled) return sampled[Math.round(x * 200)];
    return guess(x);
  }
  function guess(x) {
    if (e.type === 'bezier') {
      let lo = 0, hi = 1, t = x;
      for (let i = 0; i < 30; i++) {
        t = (lo + hi) / 2;
        const bx = 3 * (1 - t) * (1 - t) * t * e.x1 + 3 * (1 - t) * t * t * e.x2 + t * t * t;
        if (bx < x) lo = t; else hi = t;
      }
      return 3 * (1 - t) * (1 - t) * t * e.y1 + 3 * (1 - t) * t * t * e.y2 + t * t * t;
    }
    if (e.type === 'spring') {
      const w = Math.sqrt(Math.max(1, e.stiffness)), z = Math.max(0.01, e.damping) / (2 * w);
      const tt = x * 6 / Math.max(0.3, z * w / 4 + 0.6);
      if (z < 1) { const wd = w * Math.sqrt(1 - z * z); return 1 - Math.exp(-z * w * tt) * (Math.cos(wd * tt) + (z * w / wd) * Math.sin(wd * tt)); }
      return 1 - (1 + w * tt) * Math.exp(-w * tt);
    }
    if (e.type === 'steps') {
      const n = Math.max(1, e.count | 0), j = e.jump || 'end';
      const k = j === 'start' ? Math.ceil(x * n) : j === 'both' ? Math.floor(x * n) + 1 : j === 'none' ? Math.floor(x * (n - 1) + 1e-9) : Math.floor(x * n);
      const d = j === 'both' ? n + 1 : j === 'none' ? Math.max(1, n - 1) : n;
      return Math.min(1, k / d);
    }
    if (e.type === 'hold') return x >= 1 ? 1 : 0;
    return x;
  }
  function draw() {
    const d = devicePixelRatio;
    g.setTransform(d, 0, 0, d, 0, 0);
    g.clearRect(0, 0, W, H);
    g.strokeStyle = 'rgba(120,113,108,.35)'; g.lineWidth = 1;
    g.strokeRect(sx(0), sy(1), sx(1) - sx(0), sy(0) - sy(1));
    g.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent') || '#C2410C'; g.lineWidth = 2.5;
    g.beginPath();
    for (let i = 0; i <= 200; i++) { const x = i / 200, y = at(x); i ? g.lineTo(sx(x), sy(y)) : g.moveTo(sx(x), sy(y)); }
    g.stroke();
    if (e.type === 'bezier') {
      g.lineWidth = 1.25; g.strokeStyle = '#78716C';
      for (const [ax, ay, hx, hy] of [[0, 0, e.x1, e.y1], [1, 1, e.x2, e.y2]]) {
        g.beginPath(); g.moveTo(sx(ax), sy(ay)); g.lineTo(sx(hx), sy(hy)); g.stroke();
        g.beginPath(); g.arc(sx(hx), sy(hy), 7, 0, Math.PI * 2); g.fillStyle = '#fff'; g.fill(); g.stroke();
      }
    }
  }
  let dragging = null;
  canvas.addEventListener('pointerdown', (ev) => {
    if (e.type !== 'bezier') return;
    const r = canvas.getBoundingClientRect(), px = ev.clientX - r.left, py = ev.clientY - r.top;
    const d1 = Math.hypot(px - sx(e.x1), py - sy(e.y1)), d2 = Math.hypot(px - sx(e.x2), py - sy(e.y2));
    dragging = d1 < d2 ? 1 : 2;
    canvas.setPointerCapture(ev.pointerId);
    move(ev);
  });
  const move = (ev) => {
    if (!dragging) return;
    const r = canvas.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, ix(ev.clientX - r.left))), y = Math.max(-1, Math.min(2, iy(ev.clientY - r.top)));
    e[`x${dragging}`] = Math.round(x * 100) / 100; e[`y${dragging}`] = Math.round(y * 100) / 100;
    fields();
    draw();
  };
  canvas.addEventListener('pointermove', move);
  canvas.addEventListener('pointerup', () => { dragging = null; });
  const num = (label, key, { step = 0.01, min = -10, max = 10 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
    h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(e[key] ?? 0), 'aria-label': label,
      onKeydown: (ev) => ev.stopPropagation(), onChange: (ev) => { e[key] = Math.max(min, Math.min(max, Number(ev.target.value) || 0)); if (key === 'count') e[key] = Math.round(e[key]); draw(); } }));
  const extra = h('div', { class: 'form-grid' });
  function fields() {
    if (kind() === 'curve') extra.replaceChildren(num('Handle 1 X', 'x1', { min: 0, max: 1 }), num('Handle 1 Y', 'y1', { min: -1, max: 2 }), num('Handle 2 X', 'x2', { min: 0, max: 1 }), num('Handle 2 Y', 'y2', { min: -1, max: 2 }));
    else if (kind() === 'spring') extra.replaceChildren(num('Stiffness', 'stiffness', { step: 1, min: 1, max: 1000 }), num('Damping', 'damping', { step: 0.5, min: 0.1, max: 100 }));
    else extra.replaceChildren(num('Steps', 'count', { step: 1, min: 1, max: 100 }),
      h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Jumps'), h('select', { class: 'field small', onChange: (ev) => { e.jump = ev.target.value; draw(); } },
        [['end', 'At the end of each step'], ['start', 'At the start of each step'], ['both', 'At both ends'], ['none', 'Between (not at the ends)']].map(([v, l]) => h('option', { value: v, selected: (e.jump || 'end') === v }, l)))));
  }
  const tabs = h('div', { class: 'segs', role: 'radiogroup', style: 'display: flex' });
  function drawTabs() {
    tabs.replaceChildren(...[['curve', 'Curve'], ['spring', 'Spring'], ['steps', 'Steps']].map(([k, t]) => h('button', { class: `seg${kind() === k ? ' on' : ''}`, role: 'radio', 'aria-checked': String(kind() === k), style: 'flex: 1 1 0',
      onClick: () => {
        if (k === 'curve' && kind() !== 'curve') e = { type: 'bezier', x1: 0.25, y1: 0.1, x2: 0.25, y2: 1 };
        if (k === 'spring' && kind() !== 'spring') e = { type: 'spring', stiffness: 100, damping: 10 };
        if (k === 'steps' && kind() !== 'steps') e = { type: 'steps', count: 4, jump: 'end' };
        drawTabs(); fields(); draw();
      } }, t)));
  }
  const presetSel = h('select', { class: 'field small', 'aria-label': 'Start from', onChange: (ev) => { const p = presets.find((x) => x.name === ev.target.value); if (p) { e = JSON.parse(JSON.stringify(p.easing)); if (e.type === 'linear') e = { type: 'bezier', x1: 0, y1: 0, x2: 1, y2: 1 }; drawTabs(); fields(); draw(); } } },
    h('option', { value: '' }, 'Start from a ready-made one…'), presets.map((p) => h('option', { value: p.name }, label(p.name))));
  drawTabs(); fields();
  body.append(tabs, presetSel, canvas, extra);
  requestAnimationFrame(draw);
  return app.ui.modal(title, body, [['Cancel', null], ['Use it', () => e, true]]);
}
