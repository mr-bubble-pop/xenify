// New canvas: only its size, on one page. The picture on the clipboard (its size,
// pasted in) or a custom size; screen and video sizes; paper. Drawn to scale.
// Everything else starts sensibly and is changed later: a presentation is the
// Slides panel's +, a diagram is File › New diagram or Canvas settings, and
// length, background and units are in Canvas settings and Settings.

import { icon } from './icons.js';

const px = (w, h) => ({ w, h }), mm = (w, h) => ({ w, h, unit: 'mm' }), inch = (w, h) => ({ w, h, unit: 'in' });
const PRESETS = [
  ['Screen', '1080p', px(1920, 1080), 'Full HD: most screens, video and slides'],
  ['Screen', '4K', px(3840, 2160), 'TVs and big screens (heavy to render)'],
  ['Screen', '720p', px(1280, 720), 'Smaller files; web and email'],
  ['Screen', 'Phone', px(1080, 1920), 'Upright, full-screen on a phone: stories, reels, shorts'],
  ['Screen', 'Square', px(1080, 1080), 'Feeds and avatars'],
  ['Paper', 'A5', mm(148, 210), 'Flyers, booklets'],
  ['Paper', 'A4', mm(210, 297), 'The standard sheet'],
  ['Paper', 'A3', mm(297, 420), 'Posters: two A4s side by side'],
  ['Paper', 'Letter', inch(8.5, 11), 'The standard sheet in the US and Canada'],
  ['Paper', 'Legal', inch(8.5, 14), 'A longer sheet, for forms'],
  ['Paper', 'Tabloid', inch(11, 17), 'Posters: two Letters side by side'],
].map(([family, name, dim, note]) => ({ family, name, dim, note }));

const PER_INCH = { mm: 25.4, in: 1 };
/// A preset's pixels (paper at `dpi`, turned for landscape).
function pixels(p, landscape, dpi) {
  let { w, h } = p.dim;
  if (p.dim.unit) {
    if (landscape !== w > h) [w, h] = [h, w];
    return [Math.round((w / PER_INCH[p.dim.unit]) * dpi), Math.round((h / PER_INCH[p.dim.unit]) * dpi)];
  }
  return [w, h];
}
function paperLabel(p, landscape) {
  let { w, h } = p.dim;
  if (landscape !== w > h) [w, h] = [h, w];
  return `${w} × ${h} ${p.dim.unit}`;
}
const gcd = (a, b) => (b ? gcd(b, a % b) : a);
function aspect(w, h) {
  const g = gcd(w, h);
  const [a, b] = [w / g, h / g];
  return a <= 32 && b <= 32 ? `${a}:${b}` : `${(w / h).toFixed(2)}:1`;
}

/// The canvas shape with a hint of what goes on it (an SVG string).
function thumb(family, w, h, box = [80, 62]) {
  const k = Math.min(box[0] / w, box[1] / h);
  const [W, H] = [w * k, h * k];
  const paper = family === 'Paper';
  const fill = paper ? '#EBEBEB' : '#262626', ink = paper ? '#969696' : '#787878', acc = '#C2410C';
  const line = (y, a, b) => `<line x1="${W * a}" y1="${H * y}" x2="${W * b}" y2="${H * y}" stroke="${ink}" stroke-width="1.5"/>`;
  let art = '';
  if (family === 'Screen') { const c = [W / 2, H / 2], q = Math.min(W, H) * 0.18; art = `<path d="M${c[0] - q * 0.7} ${c[1] - q} L${c[0] - q * 0.7} ${c[1] + q} L${c[0] + q} ${c[1]} Z" fill="${acc}"/>`; }
  if (paper) { const f = Math.min(W, H) * 0.18; const rows = Math.max(2, Math.min(7, Math.floor(H / 7))); art = `<path d="M${W - f} 0 L${W} ${f} L${W - f} ${f} Z" fill="#BEBEBE"/>` + Array.from({ length: rows }, (_, i) => line(0.22 + (0.62 * i) / rows, 0.14, i + 1 === rows ? 0.5 : 0.84)).join(''); }
  const r = 2;
  return `<svg aria-hidden="true" width="${W}" height="${H}" viewBox="-1 -1 ${W + 2} ${H + 2}"><rect width="${W}" height="${H}" rx="${r}" fill="${fill}" stroke="#8C8C8C"/>${art}</svg>`;
}

export function newCanvasScreen({ h, recent, backTo = null, onCreate, onOpen, onRecent, onClose, readClipboard, settings = {}, serverTemplates = null, onServerTemplate, examples = [], onExample }) {
  // Only its size. Picked to begin with: the picture on the clipboard if there is one, else Custom.
  // `pick` is 'clip', 'custom' or a preset.
  const st = { pick: 'custom', landscape: false, w: 1920, h: 1080, copied: null };
  const dpi = settings.printDpi || 300;
  const preset = () => (typeof st.pick === 'object' ? st.pick : null);
  const choose = (pick) => {
    st.pick = pick;
    if (pick === 'clip') [st.w, st.h] = [st.copied.width, st.copied.height];
    else if (preset()) [st.w, st.h] = pixels(pick, st.landscape, dpi);
    draw();
  };
  const left = h('div', { class: 'nc-left' });
  const right = h('div', { class: 'nc-right' });
  const seg = (options, current, pick, label) => h('div', { class: 'segs', role: 'radiogroup', 'aria-label': label, style: 'display: inline-flex' },
    options.map(([v, l]) => h('button', { class: `seg${current === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String(current === v), onClick: () => pick(v) }, l)));
  const num = (value, set, aria) => h('input', { class: 'field', type: 'number', min: '1', max: '16384', step: '1', value: String(value), 'aria-label': aria, style: 'width: 96px',
    onChange: (e) => set(Math.max(1, Math.min(16384, Math.round(Number(e.target.value)) || 1))), onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') create(); } });
  const card = (on, title, family, [w, hh], name, sub, onClick) => h('button', { class: `nc-card${on ? ' on' : ''}`, 'aria-pressed': String(on), title, onClick, onDblclick: () => create() },
    h('span', { class: 'nc-thumb', html: thumb(family, w, hh) }), h('b', {}, name), h('small', {}, sub));
  const turned = (p) => { let { w, h: x } = p.dim; if (p.dim.unit && st.landscape !== w > x) [w, x] = [x, w]; return [w, x]; };

  function draw() {
    const mine = h('div', { class: 'nc-cards' },
      st.copied ? card(st.pick === 'clip', 'The size of the picture on the clipboard, with the picture pasted in', 'Screen', [st.copied.width, st.copied.height], 'Clipboard', `${st.copied.width} × ${st.copied.height}`, () => choose('clip')) : null,
      card(st.pick === 'custom', 'Type the width and height', 'Custom', [4, 3], 'Custom', st.pick === 'custom' ? `${st.w} × ${st.h}` : 'Any width and height', () => choose('custom')));
    const customRow = st.pick === 'custom' ? h('div', { style: 'display: flex; gap: 6px; align-items: center' },
      num(st.w, (v) => { st.w = v; draw(); }, 'Width in pixels'), '×', num(st.h, (v) => { st.h = v; draw(); }, 'Height in pixels'), h('span', { class: 'muted' }, 'px'),
      h('button', { class: 'small-btn', title: 'Swap width and height', onClick: () => { [st.w, st.h] = [st.h, st.w]; draw(); } }, 'Turn')) : null;
    const group = (family, label, extra) => [
      h('div', { class: 'nc-head' }, h('div', { class: 'lab' }, label), extra),
      h('div', { class: 'nc-cards' }, PRESETS.filter((p) => p.family === family).map((p) =>
        card(st.pick === p, p.note, family, turned(p), p.name, p.dim.unit ? paperLabel(p, st.landscape) : `${p.dim.w} × ${p.dim.h}`, () => choose(p)))),
    ];
    const way = seg([[false, 'Upright'], [true, 'Sideways']], st.landscape, (v) => { st.landscape = v; if (preset()?.dim.unit) choose(st.pick); else draw(); }, 'Which way the paper is');
    left.replaceChildren(mine, ...[customRow].filter(Boolean), ...group('Screen', 'Screens and video'), ...group('Paper', 'Paper', way));

    // ----- To scale, what it is, Create -----
    const p = preset();
    const fam = p ? PRESETS.filter((x) => x.family === p.family) : [];
    const sizes = fam.map((x) => pixels(x, st.landscape, dpi));
    const maxW = Math.max(st.w, ...sizes.map((x) => x[0])), maxH = Math.max(st.h, ...sizes.map((x) => x[1]));
    const k = Math.min(240 / maxW, 150 / maxH);
    const label = st.pick === 'clip' ? 'Clipboard' : st.pick === 'custom' ? 'Custom' : p.name;
    const scale = h('div', { class: 'nc-scale', style: `height: ${Math.ceil(maxH * k) + 4}px`, 'aria-hidden': 'true' },
      ...fam.filter((x) => x !== p).map((x) => { const [w, y] = pixels(x, st.landscape, dpi); return h('span', { class: 'ghost', title: x.name, style: `width: ${w * k}px; height: ${y * k}px` }); }),
      h('span', { class: 'mine', style: `width: ${st.w * k}px; height: ${st.h * k}px` }, label));
    const others = fam.filter((x) => x !== p).map((x) => x.name);
    const scaleNote = h('p', { class: 'muted nc-scale-note' }, others.length
      ? `Drawn to scale: this canvas (orange) over the other ${p.family === 'Paper' ? 'paper' : 'screen'} sizes (dashed): ${others.join(', ')}.` : 'Drawn to scale.');
    const paper = p?.dim.unit;
    const info = h('dl', { class: 'nc-info' },
      paper ? [h('dt', {}, 'Paper'), h('dd', {}, paperLabel(p, st.landscape))] : null,
      h('dt', {}, 'Pixels'), h('dd', {}, `${st.w} × ${st.h}${paper ? ` at ${dpi} ppi` : ''}`),
      h('dt', {}, 'Shape'), h('dd', {}, aspect(st.w, st.h)),
      h('dt', {}, 'It is'), h('dd', {}, paper ? 'A still picture for paper' : st.pick === 'clip' ? 'The copied picture, pasted in; animated (6 s) once you animate it' : 'Animated (6 s): nothing moves until you animate it'));
    const createBtn = h('button', { class: 'btn main big', style: 'align-self: flex-start', onClick: () => create() }, 'Create');
    right.replaceChildren(scale, scaleNote, info, createBtn,
      h('small', { class: 'muted' }, 'Slides: the Slides panel’s +. Length, background and units: Canvas settings, any time.'));
  }
  const create = () => {
    const p = preset();
    const unit = !settings.units || settings.units === 'auto' ? (p?.dim.unit || 'px') : settings.units;
    onCreate({
      name: st.pick === 'clip' ? 'Pasted picture' : p ? p.name : 'Untitled', width: st.w, height: st.h, duration: p?.dim.unit ? 0 : 6, fps: 30,
      units: { unit, dpi: unit === 'px' ? 96 : dpi }, kind: 'animation',
      clip: st.pick === 'clip' ? st.copied : null, ai: null,
    });
  };
  draw();
  // A picture on the clipboard is offered as a size of its own, picked. The desktop can look straight away;
  // a browser only once the page may read the clipboard, and Ctrl+V on this screen works everywhere.
  const offer = (got) => { if (got) { st.copied = got; choose('clip'); } };
  const look = () => readClipboard({ quiet: true }).then(offer).catch(() => {});
  if (settings.lookAtClipboard) look();
  else navigator.permissions?.query({ name: 'clipboard-read' }).then((q) => { if (q.state === 'granted') look(); }).catch(() => {});
  // The server's shared templates (signed in to a Xenify server), once they arrive.
  const fromServer = h('section', { 'aria-labelledby': 'srv-tpl-h', style: 'display: flex; flex-direction: column; gap: 10px', hidden: true });
  serverTemplates?.().then((list) => {
    if (!list.length) return;
    fromServer.append(h('h2', { id: 'srv-tpl-h' }, 'From your server'),
      h('div', { class: 'recent' }, list.map((n) => h('button', { onClick: () => onServerTemplate(n), title: 'A new canvas starting as a copy of this template' }, n.details?.name || 'Untitled', h('span', {}, 'Template')))));
    fromServer.hidden = false;
  }).catch(() => {});
  const screen = h('div', { class: 'screen nc-screen', role: 'tabpanel', 'aria-label': 'New canvas' },
    h('div', { class: 'screen-in' },
      // Back to your work, first thing on the screen (also its tab's ×, another tab, or Esc).
      backTo && onClose ? h('button', { class: 'nc-back', title: 'Back to the canvas you were on (Esc)', onClick: onClose }, h('span', { html: icon('left', 16), style: 'display: inline-flex' }), `Back to ${backTo}`) : null,
      h('div', { style: 'display: flex; align-items: flex-end; justify-content: space-between; gap: 12px; flex-wrap: wrap' },
        h('h1', {}, 'New canvas'),
        h('button', { class: 'btn', html: `${icon('folder', 16)}Open from your computer`, onClick: onOpen })),
      h('section', { class: 'blank nc', 'aria-label': 'Blank canvas' }, left, right),
      fromServer,
      examples.length && onExample ? h('section', { 'aria-labelledby': 'examples-h', style: 'display: flex; flex-direction: column; gap: 10px' },
        h('h2', { id: 'examples-h' }, 'Examples'),
        h('div', { class: 'recent' }, examples.map((x) => h('button', { onClick: () => onExample(x.name), title: 'A new canvas with a copy of this example, to look at, change or present' }, x.title, h('span', {}, 'Example'))))) : null,
      recent.length ? h('section', { 'aria-labelledby': 'recent-h', style: 'display: flex; flex-direction: column; gap: 10px' },
        h('h2', { id: 'recent-h' }, 'Recent'),
        h('div', { class: 'recent' }, recent.map((r) => h('button', { onClick: () => onRecent(r) }, r.name, h('span', {}, r.when))))) : null));
  // Esc goes back to the canvas, wherever the focus is (and stops listening once the screen is gone).
  const esc = (e) => {
    if (!screen.isConnected) { window.removeEventListener('keydown', esc, true); return; }
    if (e.key === 'Escape' && onClose && !document.querySelector('.popup, .modal-back')) { e.preventDefault(); e.stopPropagation(); window.removeEventListener('keydown', esc, true); onClose(); }
  };
  window.addEventListener('keydown', esc, true);
  // Ctrl+V: a pasted picture becomes the Clipboard size, picked.
  const paste = async (e) => {
    if (!screen.isConnected) { window.removeEventListener('paste', paste, true); return; }
    const file = [...(e.clipboardData?.files || [])].find((f) => f.type.startsWith('image/'));
    if (!file) return;
    e.preventDefault(); e.stopPropagation();
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const bmp = await createImageBitmap(file);
      const got = { name: 'Pasted picture.png', bytes, width: bmp.width, height: bmp.height };
      bmp.close();
      offer(got);
    } catch { /* not a picture the browser can read */ }
  };
  window.addEventListener('paste', paste, true);
  return screen;
}
