// Sound: plays the canvas's mix with the timeline, imports sound files,
// makes sound effects and music beds, and edits the presentation's
// soundtrack. The engine mixes (timing, speed, volume, fades); the page
// decodes files with Web Audio and plays the mix.

import { call, engineRaw } from './engine.js';
import * as platform from './platform.js';

/// A clip's bytes: inside the canvas file, or (desktop) read from where it's linked.
export async function clipBytes(id) {
  const got = call('media_bytes', { id });
  if (typeof got === 'string') return Uint8Array.from(atob(got), (c) => c.charCodeAt(0));
  return platform.readBytes(got.path);
}

const RATE = 48000;
let ctx = null;
const audio = () => (ctx ||= new (window.AudioContext || window.webkitAudioContext)({ sampleRate: RATE }));

/// Decodes `bytes` to 48 kHz stereo, interleaved; and its length in seconds.
export async function decode(bytes) {
  const buf = await audio().decodeAudioData(bytes.slice().buffer);
  const off = new OfflineAudioContext(2, Math.max(1, Math.ceil(buf.duration * RATE)), RATE);
  const src = off.createBufferSource();
  src.buffer = buf;
  src.connect(off.destination);
  src.start();
  const out = await off.startRendering();
  const [l, r] = [out.getChannelData(0), out.getChannelData(1)];
  const pcm = new Float32Array(l.length * 2);
  for (let i = 0; i < l.length; i++) { pcm[i * 2] = l[i]; pcm[i * 2 + 1] = r[i]; }
  return { pcm, duration: buf.duration };
}

/// Hands the engine every sound it can't read itself (it reads WAV).
export async function decodeSounds() { return decodeAll(); }
async function decodeAll() {
  const e = engineRaw();
  for (const s of call('sounds_to_decode')) {
    if (e.has_pcm(s.hash)) continue;
    try {
      const { pcm } = await decode(await clipBytes(s.id));
      e.set_pcm(s.hash, pcm);
    } catch { /* this browser can't play it: silent */ }
  }
}

export class Sound {
  constructor(app) {
    this.app = app;
    this.node = null;
  }

  /// Starts the sound from `t` (with the timeline's playback).
  async start(t) {
    this.stop();
    if (this.app.settings?.playSound === false || !call('has_sound')) return;
    // Unlocked while the click or key that started playing still counts (browsers' autoplay rule).
    const a0 = audio();
    if (a0.state === 'suspended') a0.resume().catch(() => {});
    const token = (this.token = Symbol('play'));
    await decodeAll();
    if (this.token !== token || !this.app.playing) return;
    const end = this.app.state.comp.duration;
    const pcm = engineRaw().mix(t, end, false);
    const frames = pcm.length / 2;
    if (!frames) return;
    const a = audio();
    const buf = a.createBuffer(2, frames, RATE);
    const [l, r] = [buf.getChannelData(0), buf.getChannelData(1)];
    for (let i = 0; i < frames; i++) { l[i] = pcm[i * 2]; r[i] = pcm[i * 2 + 1]; }
    const node = a.createBufferSource();
    node.buffer = buf;
    node.connect(a.destination);
    // Catch up with however long decoding and mixing took.
    const late = Math.max(0, this.app.state.time - t);
    node.start(0, late);
    node.onended = () => { if (this.node === node) this.node = null; };
    this.node = node;
  }

  /// Presenting: the presentation's sound from deck time `t` to `end`.
  async startDeck(t, end) {
    this.stop();
    if (this.app.settings?.playSound === false || !call('has_sound')) return;
    const a0 = audio();
    if (a0.state === 'suspended') a0.resume().catch(() => {});
    const token = (this.token = Symbol('deck'));
    await decodeAll();
    if (this.token !== token) return;
    const pcm = engineRaw().mix(t, Math.max(t, end), true);
    const frames = pcm.length / 2;
    if (!frames) return;
    const buf = a0.createBuffer(2, frames, RATE);
    const [l, r] = [buf.getChannelData(0), buf.getChannelData(1)];
    for (let i = 0; i < frames; i++) { l[i] = pcm[i * 2]; r[i] = pcm[i * 2 + 1]; }
    const node = a0.createBufferSource();
    node.buffer = buf;
    node.connect(a0.destination);
    node.start();
    this.node = node;
  }

  playing() {
    return !!this.node;
  }

  stop() {
    this.token = null;
    try { this.node?.stop(); } catch { /* already stopped */ }
    this.node = null;
  }
}

/// Insert › Sound…: an audio file becomes a sound layer at the playhead.
export async function importSound(app, name, bytes, { soundtrack = false, start = 0, path = null } = {}) {
  let duration;
  try { ({ duration } = await decode(bytes)); } catch { app.toast(`${name} isn’t a sound this program can play.`); return null; }
  // The desktop links the file (the canvas stays small), unless Settings says to keep clips inside it.
  const link = path && !app.settings?.embedMedia;
  let s = '';
  if (!link) for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  const mime = /\.wav$/i.test(name) ? 'audio/wav' : /\.ogg$/i.test(name) ? 'audio/ogg' : /\.(m4a|aac)$/i.test(name) ? 'audio/mp4' : /\.flac$/i.test(name) ? 'audio/flac' : /\.opus$/i.test(name) ? 'audio/opus' : 'audio/mpeg';
  return app.run(soundtrack ? 'add_soundtrack_clip' : 'add_sound', { name: name.replace(/\.[a-z0-9]+$/i, ''), mime, duration, start, ...(link ? { path, size: bytes.length } : { base64: btoa(s) }) });
}

const MOODS = [['uplifting', 'Uplifting'], ['calm', 'Calm'], ['epic', 'Epic'], ['dark', 'Dark'], ['smooth', 'Smooth']];
const KEYS = ['C', 'C♯', 'D', 'E♭', 'E', 'F', 'F♯', 'G', 'A♭', 'A', 'B♭', 'B'];

/// The settings of a generated sound (effect or music bed), editing `recipe` in place; `onChange` after each change.
export function recipeEditor(h, recipe, onChange) {
  const num = (label, key, { min = 0, max = 1, step = 0.05, k = 1 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
    h('input', { class: 'field small', type: 'number', min: String(min * k), max: String(max * k), step: String(step * k), value: String(Math.round((recipe[key] ?? 0) * k * 100) / 100), 'aria-label': label,
      onChange: (e) => { recipe[key] = Math.max(min, Math.min(max, Number(e.target.value) / k)); if (['bars', 'seed', 'key'].includes(key)) recipe[key] = Math.round(recipe[key]); onChange(); }, onKeydown: (e) => e.stopPropagation() }));
  const check = (label, key) => h('label', { class: 'check-row' }, h('input', { type: 'checkbox', checked: !!recipe[key], onChange: (e) => { recipe[key] = e.target.checked; onChange(); } }), h('span', {}, label));
  if (recipe.type === 'music') {
    return h('div', { class: 'form-grid' },
      num('Tempo (beats a minute)', 'tempo', { min: 30, max: 300, step: 1 }),
      h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Key'), h('select', { class: 'field small', onChange: (e) => { recipe.key = Number(e.target.value); onChange(); } }, KEYS.map((k, i) => h('option', { value: String(i), selected: i === recipe.key }, k)))),
      h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Mood'), h('select', { class: 'field small', onChange: (e) => { recipe.mood = e.target.value; onChange(); } }, MOODS.map(([v, t]) => h('option', { value: v, selected: v === recipe.mood }, t)))),
      num('Bars', 'bars', { min: 1, max: 64, step: 1 }), num('Volume %', 'volume', { k: 100 }), num('Variation (seed)', 'seed', { min: 0, max: 9999, step: 1 }),
      check('Pad', 'pad'), check('Arpeggio', 'arp'), check('Bass', 'bass'), check('Kick drum', 'kick'), check('Hi-hat', 'hat'), check('Loops smoothly', 'looped'));
  }
  return h('div', { class: 'form-grid' },
    num('Pitch (semitones)', 'pitch', { min: -24, max: 24, step: 1 }), num('Length (s)', 'length', { min: 0.02, max: 10 }), num('Brightness %', 'brightness', { k: 100 }),
    num('Ring (0.25–2)', 'decay', { min: 0.25, max: 2 }), num('Variation %', 'variation', { k: 100 }), num('Seed', 'seed', { min: 0, max: 9999, step: 1 }), num('Volume %', 'volume', { k: 100 }));
}

let preview = null;
/// Plays a generated sound (its recipe) on its own; again stops it.
export function previewRecipe(recipe) {
  try { preview?.stop(); } catch { /* ended */ }
  if (preview) { preview = null; return false; }
  const pcm = engineRaw().recipe_pcm(JSON.stringify(recipe));
  const frames = pcm.length / 2;
  if (!frames) return false;
  const a = audio();
  if (a.state === 'suspended') a.resume().catch(() => {});
  const buf = a.createBuffer(2, frames, RATE);
  const [l, r] = [buf.getChannelData(0), buf.getChannelData(1)];
  for (let i = 0; i < frames; i++) { l[i] = pcm[i * 2]; r[i] = pcm[i * 2 + 1]; }
  const node = a.createBufferSource();
  node.buffer = buf;
  node.connect(a.destination);
  node.onended = () => { if (preview === node) preview = null; };
  node.start();
  preview = node;
  return true;
}
