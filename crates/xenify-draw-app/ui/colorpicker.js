// The colour picker: a saturation and brightness square, a hue strip, see-
// through (alpha), sliders in RGB, HSV, HSL, CMYK or grey, the hex code, an
// eyedropper, and the colours used lately. It opens for every colour field
// (an <input type="color">): the field gets the new colour (#rrggbb), and
// its data-rgba the colour with alpha (#rrggbbaa) for fields that take it.

const RECENT = 'draw.recentColours';
let open = null;

const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const hex2 = (v) => Math.round(clamp(v, 0, 1) * 255).toString(16).padStart(2, '0');
export function toHex({ r, g, b, a = 1 }, alpha = false) { return `#${hex2(r)}${hex2(g)}${hex2(b)}${alpha && a < 0.999 ? hex2(a) : ''}`; }
export function parseHex(s) {
  const m = String(s || '').trim().replace(/^#/, '');
  const full = m.length === 3 || m.length === 4 ? m.split('').map((c) => c + c).join('') : m;
  if (!/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(full)) return null;
  const n = (i) => parseInt(full.slice(i, i + 2), 16) / 255;
  return { r: n(0), g: n(2), b: n(4), a: full.length === 8 ? n(6) : 1 };
}
function rgb2hsv({ r, g, b }) {
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
  let h = 0;
  if (d) h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return { h: ((h * 60) + 360) % 360, s: mx ? d / mx : 0, v: mx };
}
function hsv2rgb({ h, s, v }) {
  const f = (n) => { const k = (n + h / 60) % 6; return v - v * s * Math.max(0, Math.min(k, 4 - k, 1)); };
  return { r: f(5), g: f(3), b: f(1) };
}
function rgb2hsl({ r, g, b }) {
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, d = mx - mn;
  return { h: rgb2hsv({ r, g, b }).h, s: d ? d / (1 - Math.abs(2 * l - 1)) : 0, l };
}
function hsl2rgb({ h, s, l }) {
  const a = s * Math.min(l, 1 - l);
  const f = (n) => { const k = (n + h / 30) % 12; return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)); };
  return { r: f(0), g: f(8), b: f(4) };
}

function recent() { try { return JSON.parse(localStorage.getItem(RECENT) || '[]'); } catch { return []; } }
function remember(hex) {
  const list = [hex, ...recent().filter((x) => x !== hex)].slice(0, 16);
  try { localStorage.setItem(RECENT, JSON.stringify(list)); } catch { /* not kept */ }
}

/// Opens the picker by `anchor` for colour `value`; `onInput(hex, done)` as it changes. `alpha`: offer see-through.
export function openPicker(anchor, value, onInput, { alpha = true, pickFromCanvas } = {}) {
  closePicker();
  let c = parseHex(value) || { r: 0, g: 0, b: 0, a: 1 };
  let hsv = rgb2hsv(c);
  let mode = localStorage.getItem('draw.pickerMode') || 'hsv';
  const el = document.createElement('div');
  el.className = 'colour-picker';
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-label', 'Colour');
  el.innerHTML = `<canvas class="cp-sv" width="232" height="150" aria-label="Saturation and brightness: drag"></canvas>
    <canvas class="cp-hue" width="232" height="14" aria-label="Hue: drag"></canvas>
    ${alpha ? '<canvas class="cp-alpha" width="232" height="14" aria-label="See-through: drag"></canvas>' : ''}
    <div class="cp-row"><span class="cp-swatch"></span><input class="field small cp-hex" aria-label="Hex code" spellcheck="false">
      <button class="small-btn cp-eye" title="Pick a colour from the screen" aria-label="Pick a colour from the screen">Pick</button></div>
    <div class="segs cp-modes" role="radiogroup" aria-label="Sliders">${['hsv', 'rgb', 'hsl', 'cmyk', 'grey'].map((m) => `<button class="seg" data-mode="${m}" role="radio">${m === 'grey' ? 'Grey' : m.toUpperCase()}</button>`).join('')}</div>
    <div class="cp-sliders"></div><div class="cp-recent" aria-label="Recent colours"></div>`;
  document.body.append(el);
  const sv = el.querySelector('.cp-sv'), hue = el.querySelector('.cp-hue'), al = el.querySelector('.cp-alpha');
  const hexIn = el.querySelector('.cp-hex'), sw = el.querySelector('.cp-swatch'), sliders = el.querySelector('.cp-sliders');
  const emit = (done) => onInput(toHex(c, alpha), done);
  const fromHsv = () => { c = { ...hsv2rgb(hsv), a: c.a }; };
  function paint() {
    let g = sv.getContext('2d');
    g.fillStyle = toHex(hsv2rgb({ h: hsv.h, s: 1, v: 1 })); g.fillRect(0, 0, sv.width, sv.height);
    let gr = g.createLinearGradient(0, 0, sv.width, 0); gr.addColorStop(0, '#fff'); gr.addColorStop(1, 'rgba(255,255,255,0)'); g.fillStyle = gr; g.fillRect(0, 0, sv.width, sv.height);
    gr = g.createLinearGradient(0, 0, 0, sv.height); gr.addColorStop(0, 'rgba(0,0,0,0)'); gr.addColorStop(1, '#000'); g.fillStyle = gr; g.fillRect(0, 0, sv.width, sv.height);
    const x = hsv.s * sv.width, y = (1 - hsv.v) * sv.height;
    g.strokeStyle = hsv.v > 0.5 ? '#000' : '#fff'; g.lineWidth = 2; g.beginPath(); g.arc(x, y, 6, 0, Math.PI * 2); g.stroke();
    g = hue.getContext('2d');
    gr = g.createLinearGradient(0, 0, hue.width, 0);
    for (let i = 0; i <= 6; i++) gr.addColorStop(i / 6, toHex(hsv2rgb({ h: i * 60, s: 1, v: 1 })));
    g.fillStyle = gr; g.fillRect(0, 0, hue.width, hue.height);
    g.fillStyle = '#fff'; g.strokeStyle = '#000'; g.fillRect((hsv.h / 360) * hue.width - 2, 0, 4, hue.height); g.strokeRect((hsv.h / 360) * hue.width - 2, 0, 4, hue.height);
    if (al) {
      g = al.getContext('2d');
      for (let i = 0; i < al.width; i += 7) for (let j = 0; j < al.height; j += 7) { g.fillStyle = (i + j) / 7 % 2 ? '#ccc' : '#fff'; g.fillRect(i, j, 7, 7); }
      gr = g.createLinearGradient(0, 0, al.width, 0); gr.addColorStop(0, `${toHex(c)}00`); gr.addColorStop(1, toHex(c)); g.fillStyle = gr; g.fillRect(0, 0, al.width, al.height);
      g.fillStyle = '#fff'; g.strokeStyle = '#000'; g.fillRect(c.a * al.width - 2, 0, 4, al.height); g.strokeRect(c.a * al.width - 2, 0, 4, al.height);
    }
    if (document.activeElement !== hexIn) hexIn.value = toHex(c, alpha);
    sw.style.background = `linear-gradient(${toHex(c, true)}, ${toHex(c, true)}), repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 10px 10px`;
    el.querySelectorAll('.cp-modes .seg').forEach((b) => { const on = b.dataset.mode === mode; b.classList.toggle('on', on); b.setAttribute('aria-checked', String(on)); });
    drawSliders();
  }
  function drawSliders() {
    const rows = [];
    const add = (label, value, max, set) => rows.push([label, value, max, set]);
    if (mode === 'rgb') ['r', 'g', 'b'].forEach((k) => add(k.toUpperCase(), c[k] * 255, 255, (v) => { c[k] = v / 255; hsv = rgb2hsv(c); }));
    if (mode === 'hsv') { add('Hue', hsv.h, 360, (v) => { hsv.h = v; fromHsv(); }); add('Saturation %', hsv.s * 100, 100, (v) => { hsv.s = v / 100; fromHsv(); }); add('Brightness %', hsv.v * 100, 100, (v) => { hsv.v = v / 100; fromHsv(); }); }
    if (mode === 'hsl') { const l = rgb2hsl(c); add('Hue', l.h, 360, (v) => { c = { ...hsl2rgb({ ...l, h: v }), a: c.a }; hsv = rgb2hsv(c); }); add('Saturation %', l.s * 100, 100, (v) => { c = { ...hsl2rgb({ ...l, s: v / 100 }), a: c.a }; hsv = rgb2hsv(c); }); add('Lightness %', l.l * 100, 100, (v) => { c = { ...hsl2rgb({ ...l, l: v / 100 }), a: c.a }; hsv = rgb2hsv(c); }); }
    if (mode === 'cmyk') {
      const k = 1 - Math.max(c.r, c.g, c.b), cmy = ['r', 'g', 'b'].map((x) => (k < 1 ? (1 - c[x] - k) / (1 - k) : 0));
      const set = (vals) => { const [C, M, Y, K] = vals; c = { r: (1 - C) * (1 - K), g: (1 - M) * (1 - K), b: (1 - Y) * (1 - K), a: c.a }; hsv = rgb2hsv(c); };
      const vals = [...cmy, k];
      ['Cyan %', 'Magenta %', 'Yellow %', 'Black %'].forEach((label, i) => add(label, vals[i] * 100, 100, (v) => { const nv = vals.slice(); nv[i] = v / 100; set(nv); }));
    }
    if (mode === 'grey') add('Grey %', ((c.r + c.g + c.b) / 3) * 100, 100, (v) => { c = { r: v / 100, g: v / 100, b: v / 100, a: c.a }; hsv = rgb2hsv(c); });
    if (alpha) add('Opacity %', c.a * 100, 100, (v) => { c.a = v / 100; });
    sliders.replaceChildren(...rows.map(([label, value, max, set]) => {
      const row = document.createElement('label');
      row.className = 'cp-slider';
      row.innerHTML = `<span>${label}</span><input type="range" min="0" max="${max}" step="1"><input type="number" class="field small" min="0" max="${max}">`;
      const [range, num] = row.querySelectorAll('input');
      range.value = num.value = String(Math.round(value));
      const go = (v, done) => { set(clamp(Number(v) || 0, 0, max)); paint(); emit(done); };
      range.addEventListener('input', () => go(range.value, false));
      range.addEventListener('change', () => go(range.value, true));
      num.addEventListener('change', () => go(num.value, true));
      num.addEventListener('keydown', (e) => e.stopPropagation());
      return row;
    }));
  }
  const drag = (canvas, at) => canvas.addEventListener('pointerdown', (e) => {
    canvas.setPointerCapture(e.pointerId);
    const r = canvas.getBoundingClientRect();
    const go = (ev) => { at(clamp((ev.clientX - r.left) / r.width, 0, 1), clamp((ev.clientY - r.top) / r.height, 0, 1)); paint(); emit(false); };
    go(e);
    const up = () => { canvas.removeEventListener('pointermove', go); emit(true); remember(toHex(c, alpha)); drawRecent(); };
    canvas.addEventListener('pointermove', go);
    canvas.addEventListener('pointerup', up, { once: true });
  });
  drag(sv, (x, y) => { hsv.s = x; hsv.v = 1 - y; fromHsv(); });
  drag(hue, (x) => { hsv.h = x * 360; fromHsv(); });
  if (al) drag(al, (x) => { c.a = x; });
  hexIn.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') hexIn.blur(); if (e.key === 'Escape') closePicker(); });
  hexIn.addEventListener('change', () => { const p = parseHex(hexIn.value); if (p) { c = alpha ? p : { ...p, a: 1 }; hsv = rgb2hsv(c); paint(); emit(true); remember(toHex(c, alpha)); drawRecent(); } });
  el.querySelectorAll('.cp-modes .seg').forEach((b) => b.addEventListener('click', () => { mode = b.dataset.mode; try { localStorage.setItem('draw.pickerMode', mode); } catch { /* */ } paint(); }));
  el.querySelector('.cp-eye').addEventListener('click', async () => {
    // The browser's screen eyedropper where there is one; else a click on the canvas.
    if (window.EyeDropper) {
      try { const got = await new window.EyeDropper().open(); const p = parseHex(got.sRGBHex); if (p) { c = { ...p, a: c.a }; hsv = rgb2hsv(c); paint(); emit(true); } } catch { /* cancelled */ }
    } else if (pickFromCanvas) {
      el.style.visibility = 'hidden';
      const hexv = await pickFromCanvas();
      el.style.visibility = '';
      const p = parseHex(hexv);
      if (p) { c = { ...p, a: c.a }; hsv = rgb2hsv(c); paint(); emit(true); }
    }
  });
  function drawRecent() {
    const box = el.querySelector('.cp-recent');
    box.replaceChildren(...recent().map((x) => {
      const b = document.createElement('button');
      b.className = 'cp-dot'; b.title = x; b.setAttribute('aria-label', `Use ${x}`);
      b.style.background = `linear-gradient(${x}, ${x}), repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 8px 8px`;
      b.addEventListener('click', () => { const p = parseHex(x); if (p) { c = alpha ? p : { ...p, a: 1 }; hsv = rgb2hsv(c); paint(); emit(true); } });
      return b;
    }));
  }
  drawRecent();
  paint();
  // Beside the field, kept on screen.
  const r = anchor.getBoundingClientRect();
  const w = el.offsetWidth, hh = el.offsetHeight;
  el.style.left = `${clamp(r.left, 8, window.innerWidth - w - 8)}px`;
  el.style.top = `${r.bottom + 6 + hh > window.innerHeight - 8 ? Math.max(8, r.top - hh - 6) : r.bottom + 6}px`;
  open = { el, anchor, done: () => remember(toHex(c, alpha)) };
  setTimeout(() => document.addEventListener('pointerdown', outside, true), 0);
  document.addEventListener('keydown', esc, true);
  hexIn.focus();
  hexIn.select();
}

function outside(e) { if (open && !open.el.contains(e.target) && e.target !== open.anchor) closePicker(); }
function esc(e) { if (open && e.key === 'Escape') { e.stopPropagation(); closePicker(); open?.anchor?.focus?.(); } }

export function closePicker() {
  if (!open) return;
  open.done?.();
  open.el.remove();
  document.removeEventListener('pointerdown', outside, true);
  document.removeEventListener('keydown', esc, true);
  open = null;
}

/// Every colour field opens this picker instead of the system one. A field with
/// data-alpha="no" leaves see-through out; the colour with alpha is in data-rgba.
export function takeOverColourFields({ pickFromCanvas } = {}) {
  const handler = (e) => {
    const input = e.target.closest?.('input[type="color"]');
    if (!input || input.disabled) return;
    e.preventDefault();
    e.stopPropagation();
    if (open?.anchor === input) { closePicker(); return; }
    const anchor = input.closest('label, .pick, .prop') || input;
    openPicker(anchor, input.dataset.rgba || input.value, (hex, done) => {
      input.dataset.rgba = hex;
      input.value = hex.slice(0, 7);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      if (done) input.dispatchEvent(new Event('change', { bubbles: true }));
    }, { alpha: input.dataset.alpha !== 'no', pickFromCanvas });
    open.anchor = input;
  };
  document.addEventListener('click', handler, true);
  document.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target?.matches?.('input[type="color"]')) handler(e); }, true);
}
