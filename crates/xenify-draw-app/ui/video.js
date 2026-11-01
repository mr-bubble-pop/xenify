// Video layers: the engine asks for frames it doesn't have; this decodes
// them with a hidden <video> element per clip (seeking to each frame) and
// hands them over, then redraws.

import { engineRaw, call } from './engine.js';
import * as platform from './platform.js';
import { clipBytes } from './sound.js';

const clips = new Map();
let busy = false;

async function clip(id) {
  if (clips.has(id)) return clips.get(id);
  const bytes = await clipBytes(id);
  if (clips.has(id)) return clips.get(id);
  const v = document.createElement('video');
  v.muted = true;
  v.preload = 'auto';
  v.playsInline = true;
  v.src = URL.createObjectURL(new Blob([bytes]));
  const c = { v, ready: new Promise((r) => { v.addEventListener('loadeddata', r, { once: true }); v.addEventListener('error', r, { once: true }); }), canvas: document.createElement('canvas') };
  clips.set(id, c);
  return c;
}

/// The desktop decodes with ffmpeg (the system web view can't hand video pixels to a page).
const native = new Map();
async function nativeFrame(asset, index, t) {
  let m = native.get(asset);
  if (!m) {
    const info = call('media_list').find((x) => x.id === asset);
    if (!info) return false;
    let path = info.path;
    if (!path) {
      const b64 = call('media_bytes', { id: asset });
      path = await window.__TAURI__.core.invoke('media_file', { hash: info.hash, ext: (info.mime.split('/')[1] || 'mp4').replace('quicktime', 'mov'), base64: b64 });
    }
    m = { ...info, path };
    native.set(asset, m);
  }
  const buf = await window.__TAURI__.core.invoke('video_frame', { hash: m.hash, path: m.path, mime: m.mime, fps: m.fps, width: m.width, height: m.height, duration: m.duration, t });
  const bytes = new Uint8Array(buf);
  const dv = new DataView(bytes.buffer, bytes.byteOffset, 8);
  const w = dv.getUint32(0, true), h = dv.getUint32(4, true);
  engineRaw().set_frame(BigInt(asset), BigInt(index), w, h, bytes.subarray(8));
  return true;
}

/// Decodes the frames the engine asked for. Resolves to how many it handed over.
export async function pumpFrames() {
  if (busy) return 0;
  const e = engineRaw();
  const wanted = JSON.parse(e.frames_wanted());
  if (!wanted.length) return 0;
  busy = true;
  let n = 0;
  try {
    for (const [asset, index, t] of wanted) {
      if (platform.DESKTOP) {
        try { if (await nativeFrame(asset, index, t)) n++; } catch { /* that frame can't be read */ }
        continue;
      }
      let c;
      try { c = await clip(asset); } catch { continue; }
      await c.ready;
      const v = c.v;
      if (!v.videoWidth) continue;
      await new Promise((r) => {
        const done = () => { v.removeEventListener('seeked', done); r(); };
        v.addEventListener('seeked', done);
        v.currentTime = Math.min(Math.max(0, t), Math.max(0, v.duration - 0.001));
        setTimeout(done, 1500);
      });
      // Big videos are kept at most 1920 pixels wide.
      const k = Math.min(1, 1920 / v.videoWidth);
      const w = Math.max(1, Math.round(v.videoWidth * k)), h = Math.max(1, Math.round(v.videoHeight * k));
      c.canvas.width = w; c.canvas.height = h;
      const g = c.canvas.getContext('2d', { willReadFrequently: true });
      g.drawImage(v, 0, 0, w, h);
      e.set_frame(BigInt(asset), BigInt(index), w, h, new Uint8Array(g.getImageData(0, 0, w, h).data.buffer));
      n++;
    }
  } finally {
    busy = false;
  }
  return n;
}

/// A video file's size, length and frame rate (frame rate guessed: browsers don't say).
export function probe(bytes) {
  return new Promise((resolve, reject) => {
    const v = document.createElement('video');
    v.muted = true;
    v.preload = 'metadata';
    v.src = URL.createObjectURL(new Blob([bytes]));
    v.addEventListener('loadedmetadata', () => resolve({ width: v.videoWidth, height: v.videoHeight, duration: v.duration, fps: 30 }), { once: true });
    v.addEventListener('error', () => reject(new Error('not a video this program can play')), { once: true });
  });
}
