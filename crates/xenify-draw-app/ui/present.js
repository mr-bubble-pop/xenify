// The audience's window when presenting on another screen: its own engine,
// showing the presentation at the moment the presenter's window says
// (Tauri events on the desktop, a BroadcastChannel in the browser).

import { start, call, renderDeck } from './engine.js';

const canvas = document.getElementById('gl');
let t = null;
let dirty = false;
let black = false;

function draw() {
  if (dirty && t != null) {
    dirty = false;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(innerWidth * dpr), h = Math.round(innerHeight * dpr);
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    if (black) { const g = canvas.getContext('webgl2'); g?.clearColor(0, 0, 0, 1); g?.clear(g.COLOR_BUFFER_BIT); } else try { renderDeck(t, w, h); } catch { /* next time */ }
  }
  requestAnimationFrame(draw);
}

function onMessage(m) {
  if (m.project) { call('open', { json: m.project }); dirty = true; }
  if (m.t != null) { t = m.t; dirty = true; }
  if (m.black != null && m.black !== black) { black = m.black; dirty = true; }
  if (m.close) window.close();
}

async function main() {
  await start(canvas);
  addEventListener('resize', () => { dirty = true; });
  const tauri = window.__TAURI__;
  if (tauri && !window.XENIFY_WEB) {
    await tauri.event.listen('present', (e) => onMessage(e.payload));
    await tauri.event.emit('present-ready', {});
  } else {
    const ch = new BroadcastChannel('xenify-draw-present');
    ch.onmessage = (e) => onMessage(e.data);
    ch.postMessage({ ready: true });
  }
  requestAnimationFrame(draw);
}
main();
