// The engine: Xenify Draw's Rust core as WebAssembly, in this page on every
// platform. Calls are synchronous (it runs on this thread); the canvas is
// drawn with WebGL2 by Vello Hybrid.

import init, { Engine } from './pkg/xenify_draw_web.js';

let engine = null;

/// Loads the engine and gives it the canvas to draw on. Throws when this
/// browser can't (no WebGL2).
export async function start(canvas) {
  // The engine uses WebAssembly SIMD: a browser without it gets a plain message, not a blank page.
  if (!WebAssembly.validate(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11]))) {
    const msg = 'This browser is too old for Xenify: please update it (Chrome, Edge or Firefox from 2021 on, Safari 16.4 or later).';
    document.body.replaceChildren(Object.assign(document.createElement('p'), { textContent: msg, style: 'font: 16px system-ui, sans-serif; margin: 40px auto; max-width: 520px; padding: 0 16px' }));
    throw new Error(msg);
  }
  await init({ module_or_path: window.XENIFY_WASM_URL || 'pkg/xenify_draw_web_bg.wasm' });
  engine = new Engine(canvas);
}

// Editing together: changes to send to the server (see server.js, which
// sets `send` and `active`).
export const live = { send: null, active: false };

export function call(name, args = {}) {
  const out = JSON.parse(engine.call(name, JSON.stringify(args)));
  if (live.active && live.send) {
    const batches = JSON.parse(engine.live_out());
    if (batches.length) live.send(batches);
  }
  return out;
}

/// Canvas files (`.xdraw`: a zip, the canvas with its pictures and clips
/// beside it): one opened in the canvas in front (its state), and the
/// canvas in front as one (with a picture of it, PNG, and whether it moves).
export const files = {
  open: (bytes) => JSON.parse(engine.open_file(bytes)),
  save: (preview, animated) => engine.save_file(preview, animated),
};

/// Pictures and clips by hash: canvases on a server name them that way and
/// the server client fetches and sends them (see server.js).
export const media = {
  has: (hash) => engine.media_has(hash),
  get: (hash) => engine.media_get(hash) ?? null,
  put: (bytes) => engine.media_put(bytes),
};

export const tabs = {
  open: () => engine.open_tab(),
  switch: (id) => engine.switch_tab(id),
  close: (id) => engine.close_tab(id),
  front: () => engine.front_tab(),
};

/// Draws the front canvas; how long encoding and the GPU submit took (ms).
export function render(view, width, height, pasteboard) {
  const t = engine.render(Float64Array.from(view), width, height, pasteboard);
  return { encode: t[0], gpu: t[1] };
}

/// Animated GIFs made in the engine (the browser version; the desktop uses ffmpeg).
export const gif = {
  start: (w, h, fps, looping) => engine.gif_start(w, h, fps, looping),
  frame: (rgba) => engine.gif_frame(rgba),
  finish: () => engine.gif_finish(),
};

/// Draws the presentation at deck time `t` (presenting).
export function renderDeck(t, width, height) {
  return engine.render_deck(t, width, height);
}

/// Draws canvas `comp` at `time` (slide thumbnails).
export function renderComp(comp, time, view, width, height, pasteboard) {
  return engine.render_comp(BigInt(comp), time, Float64Array.from(view), width, height, pasteboard);
}

/// The engine itself, for calls that move raw samples (sound).
export const engineRaw = () => engine;

/// The front canvas as a PDF (layers with effects drawn as pictures here).
export function exportPdf(time, scale, notes = false) {
  return engine.export_pdf(time, scale, notes);
}

/// Pictures made offscreen draw on the canvas's own renderer: the canvas is drawn again (once, before the
/// page shows the frame) so it never shows them.
let restorePending = false;
function restoreCanvas() {
  if (restorePending) return;
  restorePending = true;
  queueMicrotask(() => { restorePending = false; window.drawApp?.viewport?.drawNow?.(); });
}
export const afterOffscreen = restoreCanvas;

/// A slide layout in a theme as a picture URL (gallery thumbnails; kept).
const thumbs = new Map();
export function layoutThumb(theme, layout, w = 320, h = 180) {
  const key = `${theme}/${layout}/${w}`;
  if (!thumbs.has(key)) {
    try { thumbs.set(key, URL.createObjectURL(new Blob([engine.layout_thumb(theme, layout, w, h)], { type: 'image/png' }))); } catch { thumbs.set(key, ''); }
    restoreCanvas();
  }
  return thumbs.get(key);
}

/// Frame `f` of `frames` of a transition's preview (two sample slides), as a picture URL (kept).
const tframes = new Map();
export function transitionFrame(kind, f, frames = 10, w = 224, h = 126) {
  const key = `${kind}/${f}/${frames}/${w}`;
  if (!tframes.has(key)) {
    try { tframes.set(key, URL.createObjectURL(new Blob([engine.transition_frame(kind, f / (frames - 1), w, h)], { type: 'image/png' }))); } catch { tframes.set(key, ''); }
    restoreCanvas();
  }
  return tframes.get(key);
}
export const hasTransitionFrame = (kind, f, frames = 10, w = 224) => tframes.has(`${kind}/${f}/${frames}/${w}`);

/// A pen drawing a sample stroke, as a picture URL (kept per pen).
const pthumbs = new Map();
export function penThumb(style, w = 240, h = 80) {
  const key = JSON.stringify(style);
  if (!pthumbs.has(key)) {
    try { pthumbs.set(key, URL.createObjectURL(new Blob([engine.pen_thumb(key, w, h)], { type: 'image/png' }))); } catch { pthumbs.set(key, ''); }
    restoreCanvas();
  }
  return pthumbs.get(key);
}
