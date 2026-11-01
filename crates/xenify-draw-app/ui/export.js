// File › Export: pictures, SVG, Lottie, GIF, video, PNG frames and PDF.
// The desktop app makes video, GIF, PNG frames and PDF natively (rendered
// offscreen and encoded by ffmpeg, as the egui app does); the browser draws
// each frame here and encodes GIF in the engine, WebM with WebCodecs and
// PNG frames into a zip.

import { call, render, gif, exportPdf, renderDeck } from './engine.js';
import * as platform from './platform.js';
import { pumpFrames } from './video.js';
import { mp4, webm, encodeAudio } from './mux.js';

export const FORMATS = [
  { id: 'png', label: 'PNG picture (this frame)', ext: 'png', still: true },
  { id: 'svg', label: 'SVG (this frame)', ext: 'svg', still: true },
  { id: 'pdf', label: 'PDF (this frame, or every slide)', ext: 'pdf', still: true },
  { id: 'mp4', label: 'MP4 video', ext: 'mp4', video: true },
  { id: 'webm', label: 'WebM video', ext: 'webm', video: true },
  { id: 'gif', label: 'GIF animation', ext: 'gif', video: true },
  { id: 'frames', label: 'PNG frames', ext: 'zip', video: true },
  { id: 'asvg', label: 'Animated SVG (plays in browsers)', ext: 'svg', video: true },
  { id: 'lottie', label: 'Lottie (JSON, for apps and websites)', ext: 'json', video: true },
  { id: 'lottie_html', label: 'Lottie web page', ext: 'html', video: true },
];

/// What this platform can export.
export function available() {
  const webm = typeof window.VideoEncoder === 'function';
  return FORMATS.filter((f) => (platform.DESKTOP ? true : !f.desktop && (!['webm', 'mp4'].includes(f.id) || webm)));
}

/// The export sizes besides the canvas's own (turned for portrait canvases).
const SIZES = [['720p', 1280, 720], ['1080p Full HD', 1920, 1080], ['1440p', 2560, 1440], ['4K UHD', 3840, 2160]];

/// Shows the export screen and does the export. `ui` gives the dialog helpers.
export async function exportDialog(app, h, { modal, toast, title }) {
  const st = app.state;
  const animated = st.comp.duration > 0;
  const list = available().filter((f) => animated || f.still || st.presentation);
  // A see-through canvas stays see-through where the format can keep it; elsewhere you pick what goes behind.
  const clear = String(st.comp.background).length === 9 && parseInt(String(st.comp.background).slice(7, 9), 16) < 255;
  const defaults = { format: animated ? 'mp4' : 'png', scale: 100, transparent: clear, behind: '#ffffff', loop: true, sound: true, size: null, letterbox: '#000000', notes: false, deck: false };
  const last = app.lastExport && list.find((f) => f.id === app.lastExport.format) ? { ...defaults, ...app.lastExport } : defaults;
  if (!list.find((f) => f.id === last.format)) last.format = list[0].id;
  const o = { ...last };
  if (!clear) o.transparent = !!last.transparent && !!app.lastExport;
  else if (!app.lastExport) o.transparent = true;
  const base = (title || st.comp.name || 'Canvas').replace(/[\\/:*?"<>|]/g, '-');
  const slidesInfo = st.presentation ? call('slides').list : [];
  const nSlides = slidesInfo.length;
  o.range = o.range && o.range[1] < nSlides ? o.range : [0, Math.max(0, nSlides - 1)];
  const slices = call('slices');
  const portrait = st.comp.height > st.comp.width;
  const fileName = h('input', { class: 'field', 'aria-label': 'File name', onKeydown: (e) => e.stopPropagation() });
  const extOf = (f) => FORMATS.find((x) => x.id === f).ext;
  const setName = () => { const stem = fileName.value.replace(/\.[a-z0-9]+$/i, '') || base; fileName.value = `${stem}.${extOf(o.format)}`; };
  fileName.value = `${base}.${extOf(o.format)}`;
  const body = h('div', { class: 'export-box' });
  // Typing a name with a known ending picks that format.
  fileName.addEventListener('input', () => {
    const m = fileName.value.match(/\.([a-z0-9]+)$/i);
    const f = m && list.find((x) => x.ext === m[1].toLowerCase() && !(x.id === 'asvg' && o.format === 'svg') && !(x.id === 'lottie_html' && o.format !== 'lottie_html'));
    if (f && f.id !== o.format && !(f.ext === extOf(o.format))) { o.format = f.id; draw(); fileName.focus(); }
  });
  const check = (label, key, title2) => h('label', { class: 'check-row', title: title2 || '' }, h('input', { type: 'checkbox', checked: !!o[key], onChange: (e) => { o[key] = e.target.checked; draw(); } }), h('span', {}, label));
  function draw() {
    const f = FORMATS.find((x) => x.id === o.format);
    const raster = ['png', 'mp4', 'webm', 'gif', 'frames', 'pdf'].includes(f.id);
    const fmt = h('select', { class: 'field', 'aria-label': 'Format', onChange: (e) => { o.format = e.target.value; setName(); draw(); } }, list.map((x) => h('option', { value: x.id, selected: x.id === o.format }, x.label)));
    // Size: the canvas's times a scale, a standard video size, or any.
    const sizeSegs = h('div', { class: 'add-row' },
      h('button', { class: `pick${!o.size ? ' on' : ''}`, onClick: () => { o.size = null; o.preset = null; draw(); } }, 'Canvas size'),
      SIZES.map(([n, w, hh], i) => h('button', { class: `pick${o.preset === i ? ' on' : ''}`, title: portrait ? `${hh} × ${w}` : `${w} × ${hh}`, onClick: () => { o.size = portrait ? [hh, w] : [w, hh]; o.preset = i; draw(); } }, n)),
      h('button', { class: `pick${o.size && o.preset == null ? ' on' : ''}`, onClick: () => { o.size = o.size || [st.comp.width, st.comp.height]; o.preset = null; draw(); } }, 'Custom'));
    const k = o.scale / 100;
    const sizeRow = !o.size
      ? h('div', { style: 'display: flex; gap: 8px; align-items: center' }, h('input', { class: 'field', type: 'number', min: '10', max: '800', step: '10', value: String(o.scale), style: 'width: 110px', 'aria-label': 'Scale (%)',
        onChange: (e) => { o.scale = Math.max(10, Math.min(800, Number(e.target.value) || 100)); draw(); }, onKeydown: (e) => e.stopPropagation() }), h('span', {}, '%'),
      h('small', { class: 'muted' }, `${Math.round(st.comp.width * k)} × ${Math.round(st.comp.height * k)} px${animated ? ` · ${st.comp.fps} frames a second` : ''}`))
      : h('div', { style: 'display: flex; gap: 8px; align-items: center' },
        ...[0, 1].map((i) => h('input', { class: 'field', type: 'number', min: '16', max: '16384', value: String(o.size[i]), disabled: o.preset != null, style: 'width: 110px', 'aria-label': i ? 'Height' : 'Width',
          onChange: (e) => { o.size[i] = Math.max(16, Math.min(16384, Math.round(Number(e.target.value)) || o.size[i])); draw(); }, onKeydown: (e) => e.stopPropagation() })), h('span', {}, 'px'));
    const otherShape = o.size && Math.abs((o.size[0] / o.size[1]) / (st.comp.width / st.comp.height) - 1) > 0.005;
    const deckVideo = nSlides > 1 && ['mp4', 'webm', 'gif', 'frames'].includes(f.id);
    const deckPng = nSlides > 1 && f.id === 'png';
    body.replaceChildren(...[
      h('label', {}, 'Format', fmt),
      h('label', {}, 'File name', fileName, h('small', { class: 'muted' }, 'Tip: the ending picks the format (.mp4, .gif, .png, .svg, .pdf…).')),
      slices.length && !o.deck ? h('label', {}, 'Of', h('select', { class: 'field', onChange: (e) => { o.of = e.target.value; } }, h('option', { value: 'canvas' }, 'The whole canvas'), h('option', { value: 'slices', selected: o.of === 'slices' }, `Each slice (${slices.length}), as its own file`))) : null,
      raster && f.id !== 'pdf' ? h('div', { class: 'lab' }, 'Size') : null,
      raster && f.id !== 'pdf' ? sizeSegs : null,
      raster && f.id !== 'pdf' ? sizeRow : null,
      raster && otherShape ? h('label', { class: 'check-row' }, h('input', { type: 'color', class: 'color-in', value: o.letterbox || '#000000', onInput: (e) => { o.letterbox = e.target.value; } }), h('span', {}, 'Another shape than the canvas: bars of this colour fill the sides')) : null,
      ALPHA.includes(f.id) ? check('See-through background', 'transparent', 'Leave the background out (MP4 and PDF can’t store it)') : null,
      // A see-through canvas in a format (or choice) without see-through: what goes behind it.
      clear && !(ALPHA.includes(f.id) && o.transparent) ? h('div', { class: 'lab', style: 'display: flex; flex-direction: column; gap: 6px' },
        ALPHA.includes(f.id) ? 'Background behind the drawing' : `${f.label.split(' ')[0]} can’t be see-through: what goes behind the drawing?`,
        h('div', { class: 'add-row' },
          [['#ffffff', 'White'], ['#000000', 'Black']].map(([v, l]) => h('button', { class: `pick${o.behind === v ? ' on' : ''}`, onClick: () => { o.behind = v; draw(); } }, l)),
          h('label', { class: `pick${!['#ffffff', '#000000'].includes(o.behind) ? ' on' : ''}`, style: 'display: inline-flex; flex-direction: row; align-items: center; gap: 8px' }, 'Colour',
            h('input', { type: 'color', class: 'color-in', value: o.behind, 'aria-label': 'Colour behind the drawing', onInput: (e) => { o.behind = e.target.value; }, onChange: () => draw() })))) : null,
      ['gif', 'asvg'].includes(f.id) ? check('Repeat forever', 'loop') : null,
      ['mp4', 'webm'].includes(f.id) ? check('Include the sound (clips, generated sounds and the soundtrack, mixed)', 'sound') : null,
      f.id === 'pdf' && nSlides ? check('Include speaker notes (a handout page per slide)', 'notes') : null,
      deckVideo ? check(`The whole presentation (${nSlides} slides with their transitions, no pauses)`, 'deck') : null,
      deckVideo && o.deck ? h('div', { style: 'display: flex; gap: 8px; align-items: center' }, h('span', {}, 'From slide'),
        ...[0, 1].map((i) => h('input', { class: 'field', type: 'number', min: '1', max: String(nSlides), value: String(o.range[i] + 1), style: 'width: 80px', 'aria-label': i ? 'To slide' : 'From slide',
          onChange: (e) => { o.range[i] = Math.max(0, Math.min(nSlides - 1, Math.round(Number(e.target.value)) - 1)); if (o.range[0] > o.range[1]) o.range = [o.range[1], o.range[0]]; draw(); }, onKeydown: (e) => e.stopPropagation() })).flatMap((x, i) => (i ? [h('span', {}, 'to'), x] : [x]))) : null,
      deckPng ? check(`Every slide (${nSlides} pictures, each finished)`, 'deck') : null,
      h('small', { class: 'muted' }, ({ frames: 'Frames go into a zip (the desktop app: a folder) with this name.', png: o.deck ? 'One picture per slide, named …-1.png, …-2.png.' : 'Saves the frame at the playhead.', svg: 'Saves the frame at the playhead.',
        asvg: 'Plays in web browsers: keyframes become CSS animations with their easing.', lottie: 'For lottie-web and the iOS and Android Lottie players.', lottie_html: 'A page that plays the animation (loads the Lottie player from the internet).',
        pdf: nSlides ? 'Every slide as it looks when finished, one per page.' : 'This frame as a page.' })[f.id] || ''),
    ].filter(Boolean));
  }
  draw();
  const ok = await modal('Export', body, [['Cancel', false], ['Export', true, true]]);
  if (!ok) return;
  const f = FORMATS.find((x) => x.id === o.format);
  const named = fileName.value.trim() || `${base}.${f.ext}`;
  const name = /\.[a-z0-9]+$/i.test(named) ? named : `${named}.${f.ext}`;
  const deck = o.deck && ((nSlides > 1 && ['mp4', 'webm', 'gif', 'frames'].includes(f.id)) || (nSlides > 1 && f.id === 'png'));
  const opts = { ...o, deck, scale: Math.max(10, Math.min(800, Number(o.scale) || 100)) };
  app.lastExport = { ...opts, range: null };
  const stem = name.replace(/\.[a-z0-9]+$/i, '');
  const regions = opts.of === 'slices' && !deck ? slices.map((sl) => ({ rect: sl.rect, name: `${stem}-${sl.name.replace(/[\\/:*?"<>|]/g, '-')}.${f.ext}` }))
    : [{ rect: null, name }];
  let folder = null;
  try {
    for (const r of regions) {
      // The desktop asks once where slices go; each is saved beside that name.
      if (platform.DESKTOP && regions.length > 1 && !folder) {
        const where = await platform.chooseSave(regions[0].name);
        if (!where) return;
        folder = where.replace(/[\\/][^\\/]*$/, '');
      }
      const path = folder ? `${folder}/${r.name}` : null;
      // No checkerboard in the export; a see-through canvas gets the chosen colour unless it stays see-through.
      const keep = ALPHA.includes(f.id) && opts.transparent;
      call('export_mode', { on: true, background: clear && !keep ? opts.behind : null });
      try { await run(app, h, opts, f, r.name, { modal, toast, rect: r.rect, path, behind: clear && !keep ? opts.behind : null }); } finally { call('export_mode', { on: false }); app.viewport.redraw(); }
    }
  } catch (e) {
    toast(`Export failed: ${e.message || e}`);
  }
}

/// Formats that can keep a see-through background.
const ALPHA = ['png', 'webm', 'gif', 'frames', 'svg', 'asvg', 'lottie', 'lottie_html'];

async function run(app, h, opts, f, name, { toast, rect, path, behind }) {
  const st = app.state;
  const k = opts.scale / 100;
  const R = rect || [0, 0, st.comp.width, st.comp.height];
  // The output: the region times the scale, or a given size with the region fitted in it.
  let w = Math.max(1, Math.round((R[2] - R[0]) * k)), hh = Math.max(1, Math.round((R[3] - R[1]) * k));
  let region = { x: R[0], y: R[1], k };
  if (opts.size && !rect) {
    [w, hh] = opts.size;
    const kk = Math.min(w / (R[2] - R[0]), hh / (R[3] - R[1]));
    region = { x: R[0], y: R[1], k: kk, ox: (w - (R[2] - R[0]) * kk) / 2, oy: (hh - (R[3] - R[1]) * kk) / 2, bars: opts.letterbox || '#000000' };
  }
  // Where it went (desktop), so the message can offer to show it in its folder.
  let savedTo = null;
  const save = async (data, type, as = name) => { const r = await platform.saveFile({ name: as, path, ...(typeof data === 'string' ? { text: data } : { bytes: data }), type }); if (typeof r === 'string') savedTo = r; return r; };
  const done = (msg) => toast(msg, savedTo ? { path: savedTo } : undefined);
  const cut = rect ? { rect } : {};
  // Made from the document alone (the same everywhere).
  if (f.id === 'svg' || f.id === 'asvg') { if (await save(call('export_svg', { animated: f.id === 'asvg', loop: opts.loop, ...cut }), 'image/svg+xml')) done(`Exported ${name}`); return; }
  if (f.id === 'lottie' || f.id === 'lottie_html') { if (await save(call('export_lottie', { html: f.id === 'lottie_html', ...cut }), f.id === 'lottie' ? 'application/json' : 'text/html')) done(`Exported ${name}`); return; }
  if (f.id === 'pdf' && !platform.DESKTOP) {
    const bytes = exportPdf(st.time, Math.max(1, k) * 2, !!opts.notes);
    app.viewport.redraw();
    if (await save(bytes, 'application/pdf')) done(`Exported ${name}`);
    return;
  }
  if (f.id === 'png' && !opts.deck) { const bytes = await framePng(app, st.time, w, hh, opts.transparent, region); if (await save(bytes, 'image/png')) done(`Exported ${name}`); return; }
  const deckInfo = opts.deck ? call('deck') : null;
  // The desktop makes these itself.
  if (platform.DESKTOP && ['mp4', 'webm', 'gif', 'frames', 'pdf'].includes(f.id) || (platform.DESKTOP && f.id === 'png' && opts.deck)) {
    if (!['pdf', 'png'].includes(f.id) && !(await platform.hasFfmpeg())) { toast('Video, GIF and PNG frames need ffmpeg: install it (for example: sudo apt install ffmpeg), then try again.'); return; }
    const where = path ? (f.id === 'frames' ? path.replace(/\.zip$/, '') : path) : await platform.chooseSave(f.id === 'frames' ? name.replace(/\.zip$/, '') : name);
    if (!where) return;
    const prog = progress(h, `Exporting ${f.label}`, () => platform.exportCancel());
    const off = await platform.onExportProgress((done, total) => prog.set(done, total));
    try {
      await platform.exportNative({ project: rect ? call('cropped_project', { rect }) : call('save_copy', behind ? { background: behind } : {}), comp: st.comp.id, path: where, format: f.id === 'frames' ? 'png' : f.id === 'png' ? 'slides_png' : f.id, scale: k, transparent: opts.transparent, sound: opts.sound, time: st.time,
        size: opts.size && !rect ? opts.size : null, letterbox: opts.letterbox || null, deck: !!opts.deck && f.id !== 'png', slides: opts.deck ? opts.range : null, notes: !!opts.notes });
      savedTo = where;
      done(`Exported ${where.split(/[\\/]/).pop()}`);
    } finally { off(); prog.close(); }
    return;
  }
  // The browser draws each frame here.
  const fps = st.comp.fps || 30;
  const from = deckInfo ? deckInfo.starts[opts.range[0]] : 0;
  const to = deckInfo ? deckInfo.ends[opts.range[1]] : st.comp.duration;
  const frames = Math.max(1, Math.round((to - from) * fps));
  const deck = deckInfo ? { from } : null;
  // The presentation draws letterboxed to the output size.
  if (deck && !opts.size) { const first = call('slides').list[0]; w = Math.max(1, Math.round((first?.width || st.comp.width) * k)); hh = Math.max(1, Math.round((first?.height || st.comp.height) * k)); }
  let cancelled = false;
  const prog = progress(h, `Exporting ${f.label}`, () => { cancelled = true; });
  const was = st.time;
  const pixelsAt = async (i) => (deck ? deckAt(from + i / fps, w, hh, (gl) => grab(gl, w, hh), app) : framePixels(app, i / fps, w, hh, opts.transparent, region));
  try {
    if (f.id === 'png' && opts.deck) {
      // Every slide, finished: a zip of pictures.
      const zip = new Zip();
      const list = call('slides').list;
      for (let i = 0; i < list.length && !cancelled; i++) {
        const t = Math.max(deckInfo.starts[i], deckInfo.ends[i] - 0.001);
        zip.add(`${name.replace(/\.png$/i, '')}-${i + 1}.png`, await deckAt(t, w, hh, (gl) => pngOf(gl), app));
        prog.set(i + 1, list.length);
      }
      if (!cancelled && await save(zip.finish(), 'application/zip', name.replace(/\.png$/i, '.zip'))) done(`Exported ${list.length} slides`);
    } else if (f.id === 'gif') {
      gif.start(w, hh, fps, opts.loop);
      for (let i = 0; i < frames && !cancelled; i++) {
        gif.frame(await pixelsAt(i));
        prog.set(i + 1, frames);
        await idle();
      }
      const bytes = gif.finish();
      if (!cancelled && await save(bytes, 'image/gif')) done(`Exported ${name}`);
    } else if (f.id === 'frames') {
      const zip = new Zip();
      for (let i = 0; i < frames && !cancelled; i++) {
        zip.add(`frame_${String(i).padStart(5, '0')}.png`, deck ? await deckAt(from + i / fps, w, hh, (gl) => pngOf(gl), app) : await framePng(app, i / fps, w, hh, opts.transparent, region));
        prog.set(i + 1, frames);
      }
      if (!cancelled && await save(zip.finish(), 'application/zip')) done(`Exported ${name}`);
    } else if (f.id === 'webm' || f.id === 'mp4') {
      const bytes = await encodeVideo(app, f.id, w, hh, fps, frames, opts.transparent, region, opts.sound, (i) => { prog.set(i, frames); return !cancelled; }, deck);
      if (!cancelled && bytes && await save(bytes, f.id === 'mp4' ? 'video/mp4' : 'video/webm')) done(`Exported ${name}`);
    }
  } finally {
    prog.close();
    app.setTime(was);
    app.viewport.redraw();
  }
}

/// The WebGL canvas's pixels (RGBA, top row first) and as PNG bytes.
function grab(gl, w, h) {
  scratch ||= document.createElement('canvas');
  scratch.width = w;
  scratch.height = h;
  const g = scratch.getContext('2d', { willReadFrequently: true });
  g.clearRect(0, 0, w, h);
  g.drawImage(gl, 0, 0);
  return new Uint8Array(g.getImageData(0, 0, w, h).data.buffer);
}
function pngOf(gl) {
  const url = gl.toDataURL('image/png');
  return Uint8Array.from(atob(url.split(',')[1]), (c) => c.charCodeAt(0));
}

const idle = () => new Promise((r) => setTimeout(r, 0));

/// Draws the canvas at time `t` into the WebGL canvas at `w`×`h` and hands the canvas to `use`.
function drawAt(app, t, w, h, transparent, use, region) {
  const st = app.state;
  // Exports show what the camera sees.
  const looked = call('look_through').on;
  if (!looked) call('look_through', { on: true });
  const r = region || { x: 0, y: 0, k: w / st.comp.width };
  call('set_time', { time: t });
  const gl = app.viewport.gl;
  const [ow, oh] = [gl.width, gl.height];
  gl.width = w;
  gl.height = h;
  try {
    // An output of another shape than the canvas: centred, the bars in the letterbox colour.
    render([r.k, 0, 0, r.k, (r.ox || 0) - r.x * r.k, (r.oy || 0) - r.y * r.k], w, h, transparent ? '#00000000' : (r.bars || st.comp.background));
    return use(gl);
  } finally {
    gl.width = ow;
    gl.height = oh;
    if (!looked) call('look_through', { on: false });
  }
}

/// Draws once and fetches any video frames that drawing needed (a few rounds), so the frame is complete.
const frames_ = (...a) => frames(...a);
async function frames(app, t, w, h, transparent, region) {
  for (let i = 0; i < 4; i++) {
    drawAt(app, t, w, h, transparent, () => null, region);
    if (!(await pumpFrames())) break;
  }
}

/// Draws the presentation at deck time `t` into the WebGL canvas and hands it to `use`.
async function deckAt(t, w, h, use, app) {
  const gl = app.viewport.gl;
  for (let i = 0; i < 4; i++) {
    const [ow, oh] = [gl.width, gl.height];
    gl.width = w; gl.height = h;
    try { renderDeck(t, w, h); } finally { if (i === 3 || !(await pumpFrames())) { const out = use(gl); gl.width = ow; gl.height = oh; return out; } gl.width = ow; gl.height = oh; }
  }
  return null;
}

let scratch = null;
/// The frame at `t` as RGBA pixels; with motion blur, the average of the moments the shutter is open.
async function framePixels(app, t, w, h, transparent, region) {
  const times = app.state.comp.motion_blur ? call('shutter_times', { time: t }) : [t];
  if (times.length < 2) return sharpPixels(app, t, w, h, transparent, region);
  const sum = new Float32Array(w * h * 4);
  for (const s of times) {
    const px = await sharpPixels(app, s, w, h, transparent, region);
    for (let i = 0; i < px.length; i++) sum[i] += px[i];
  }
  const out = new Uint8Array(sum.length);
  for (let i = 0; i < sum.length; i++) out[i] = Math.round(sum[i] / times.length);
  call('set_time', { time: t });
  return out;
}

async function sharpPixels(app, t, w, h, transparent, region) {
  await frames(app, t, w, h, transparent, region);
  return drawAt(app, t, w, h, transparent, (gl) => {
    scratch ||= document.createElement('canvas');
    scratch.width = w;
    scratch.height = h;
    const g = scratch.getContext('2d', { willReadFrequently: true });
    g.clearRect(0, 0, w, h);
    g.drawImage(gl, 0, 0);
    return new Uint8Array(g.getImageData(0, 0, w, h).data.buffer);
  }, region);
}

async function framePng(app, t, w, h, transparent, region) {
  if (app.state.comp.motion_blur) {
    const px = await framePixels(app, t, w, h, transparent, region);
    const c = document.createElement('canvas');
    c.width = w; c.height = h;
    c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(px.buffer), w, h), 0, 0);
    const url = c.toDataURL('image/png');
    return Uint8Array.from(atob(url.split(',')[1]), (ch) => ch.charCodeAt(0));
  }
  await frames(app, t, w, h, transparent, region);
  const url = drawAt(app, t, w, h, transparent, (gl) => gl.toDataURL('image/png'), region);
  return Uint8Array.from(atob(url.split(',')[1]), (c) => c.charCodeAt(0));
}

/// The frame at `t` as PNG bytes at `k` times the canvas size (for printing,
/// and the pictures other documents show). A see-through canvas stays
/// see-through, without the editor's squares.
export function renderPng(app, t, k) {
  const st = app.state;
  const w = Math.max(1, Math.round(st.comp.width * k)), h = Math.max(1, Math.round(st.comp.height * k));
  const was = st.time;
  call('export_mode', { on: true });
  return framePng(app, t, w, h, false, { x: 0, y: 0, k }).finally(() => { call('export_mode', { on: false }); app.setTime(was); app.viewport.redraw(); });
}

function progress(h, title, onCancel) {
  const bar = h('div', { class: 'progress-bar' }, h('span'));
  const text = h('p', {}, 'Getting ready…');
  const back = h('div', { class: 'modal-back' }, h('div', { class: 'modal', role: 'dialog', 'aria-label': title },
    h('h2', {}, title), text, bar,
    h('div', { class: 'modal-actions' }, h('button', { class: 'btn', onClick: () => { onCancel(); text.textContent = 'Stopping…'; } }, 'Cancel'))));
  document.body.append(back);
  return {
    set: (done, total) => { bar.firstChild.style.width = `${Math.round((done / Math.max(1, total)) * 100)}%`; text.textContent = `Frame ${done} of ${total}`; },
    close: () => back.remove(),
  };
}

// ----- PNG frames: a zip with no compression (PNGs are compressed already) -----

const CRC = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; }
  return t;
})();
const crc32 = (b) => { let c = 0xFFFFFFFF; for (let i = 0; i < b.length; i++) c = CRC[(c ^ b[i]) & 0xFF] ^ (c >>> 8); return (c ^ 0xFFFFFFFF) >>> 0; };

class Zip {
  constructor() { this.parts = []; this.dir = []; this.at = 0; }
  add(name, data) {
    const n = new TextEncoder().encode(name);
    const crc = crc32(data);
    const head = new DataView(new ArrayBuffer(30));
    head.setUint32(0, 0x04034b50, true); head.setUint16(4, 20, true); head.setUint32(14, crc, true);
    head.setUint32(18, data.length, true); head.setUint32(22, data.length, true); head.setUint16(26, n.length, true);
    const entry = new DataView(new ArrayBuffer(46));
    entry.setUint32(0, 0x02014b50, true); entry.setUint16(4, 20, true); entry.setUint16(6, 20, true); entry.setUint32(16, crc, true);
    entry.setUint32(20, data.length, true); entry.setUint32(24, data.length, true); entry.setUint16(28, n.length, true); entry.setUint32(42, this.at, true);
    this.parts.push(new Uint8Array(head.buffer), n, data);
    this.dir.push(new Uint8Array(entry.buffer), n);
    this.at += 30 + n.length + data.length;
  }
  finish() {
    const size = this.dir.reduce((s, p) => s + p.length, 0);
    const end = new DataView(new ArrayBuffer(22));
    end.setUint32(0, 0x06054b50, true); end.setUint16(8, this.dir.length / 2, true); end.setUint16(10, this.dir.length / 2, true);
    end.setUint32(12, size, true); end.setUint32(16, this.at, true);
    return concat([...this.parts, ...this.dir, new Uint8Array(end.buffer)]);
  }
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((s, p) => s + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

// ----- Video: WebCodecs encoders, files written by mux.js -----

/// Encodes the frames (and, with `sound`, the canvas's mix) as WebM (VP9/VP8 + Opus) or MP4 (H.264 + AAC or Opus).
async function encodeVideo(app, kind, w, h, fps, frames, transparent, region, sound, step, deck = null) {
  const ew = w + (w % 2), eh = h + (h % 2);
  const chunks = [];
  let failed = null, description = null;
  const enc = new VideoEncoder({ output: (c, meta) => {
    const b = new Uint8Array(c.byteLength); c.copyTo(b);
    chunks.push({ t: c.timestamp, d: c.duration || Math.round(1e6 / fps), key: c.type === 'key', b });
    if (meta?.decoderConfig?.description) description = new Uint8Array(meta.decoderConfig.description);
  }, error: (e) => { failed = e; } });
  const candidates = kind === 'mp4' ? ['avc1.640028', 'avc1.4d0028', 'avc1.42E01F'] : ['vp09.00.10.08', 'vp8'];
  let config = null;
  for (const codec of candidates) {
    const c = { codec, width: ew, height: eh, bitrate: Math.max(1e6, ew * eh * fps * 0.12), framerate: fps, ...(kind === 'mp4' ? { avc: { format: 'avc' } } : { alpha: transparent ? 'keep' : 'discard' }) };
    if ((await VideoEncoder.isConfigSupported(c).catch(() => ({ supported: false }))).supported) { config = c; break; }
  }
  if (!config) throw new Error(kind === 'mp4' ? 'this browser can’t make H.264 video (the desktop app can)' : 'this browser can’t make WebM video');
  enc.configure(config);
  const canvas = document.createElement('canvas');
  canvas.width = ew; canvas.height = eh;
  const g = canvas.getContext('2d');
  for (let i = 0; i < frames; i++) {
    if (!step(i)) { enc.close(); return null; }
    if (deck) {
      await deckAt(deck.from + i / fps, w, h, (gl) => { g.clearRect(0, 0, ew, eh); g.drawImage(gl, 0, 0); }, app);
    } else if (app.state.comp.motion_blur) {
      const px = await framePixels(app, i / fps, w, h, transparent, region);
      g.clearRect(0, 0, ew, eh);
      g.putImageData(new ImageData(new Uint8ClampedArray(px.buffer), w, h), 0, 0);
    } else {
      await frames_(app, i / fps, w, h, transparent, region);
      drawAt(app, i / fps, w, h, transparent, (gl) => { g.clearRect(0, 0, ew, eh); g.drawImage(gl, 0, 0); }, region);
    }
    const frame = new VideoFrame(canvas, { timestamp: Math.round((i * 1e6) / fps), duration: Math.round(1e6 / fps) });
    enc.encode(frame, { keyFrame: i % Math.max(1, Math.round(fps * 2)) === 0 });
    frame.close();
    if (enc.encodeQueueSize > 8) await new Promise((r) => setTimeout(r, 5));
    if (failed) throw failed;
  }
  await enc.flush();
  enc.close();
  if (failed) throw failed;
  // The sound: the canvas's mix, as long as the video.
  let audio = null;
  if (sound && call('has_sound')) {
    const { engineRaw } = await import('./engine.js');
    const { decodeSounds } = await import('./sound.js');
    await decodeSounds();
    audio = await encodeAudio(deck ? engineRaw().mix(deck.from, deck.from + frames / fps, true) : engineRaw().mix(0, frames / fps, false), kind === 'mp4' ? 'aac' : 'opus');
  }
  if (kind === 'mp4') return mp4({ width: ew, height: eh, fps, video: chunks, videoConfig: description, audio: audio?.chunks, audioCodec: audio?.codec, audioConfig: audio?.config });
  return webm({ width: ew, height: eh, video: chunks, codec: config.codec.startsWith('vp09') ? 'V_VP9' : 'V_VP8', audio: audio?.codec === 'opus' ? audio.chunks : null });
}
