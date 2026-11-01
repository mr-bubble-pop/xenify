// What differs between the desktop app (Tauri) and the browser: files and
// spelling/grammar. The desktop asks the Rust side; the browser uses file
// pickers and downloads, and runs Xenify Write's checker in a worker.

const tauri = window.__TAURI__;
export const DESKTOP = !!tauri && !window.XENIFY_WEB;
const invoke = DESKTOP ? tauri.core.invoke : null;

export function log(text) {
  if (DESKTOP) invoke('log', { text }).catch(() => {});
  else if (window.XENIFY_LOG) fetch('/log', { method: 'POST', body: text }).catch(() => {});
}

// ----- Files -----

/// A picture's bytes from the web (the desktop downloads it itself, so any site works).
export async function fetchPicture(url) {
  if (DESKTOP) return Uint8Array.from(atob(await invoke('fetch_picture', { url })), (c) => c.charCodeAt(0));
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return new Uint8Array(await res.arrayBuffer());
}

/// A file name for a picture from `url` (GIPHY's are all called giphy.gif: named by their id).
export function pictureName(url) {
  const giphy = url.match(/giphy\.com\/media\/(?:v1\.[^/]+\/)?([A-Za-z0-9]+)\//);
  if (giphy) return `GIPHY ${giphy[1]}.gif`;
  const last = decodeURIComponent(url.split(/[?#]/)[0].split('/').pop() || '');
  return /\.(gif|png|jpe?g|webp|bmp)$/i.test(last) ? last : 'Pasted picture.gif';
}

/// The desktop: the picture on the system clipboard as { name, bytes (an image file), width, height }, or null.
export async function clipboardPicture() {
  if (!DESKTOP) return null;
  const got = await invoke('clipboard_picture');
  if (!got) return null;
  const raw = (b) => Uint8Array.from(atob(b), (c) => c.charCodeAt(0));
  if (got.url) {
    // An animated GIF copied in a browser: fetched from where it's from.
    const bytes = await fetchPicture(got.url);
    const bmp = await createImageBitmap(new Blob([bytes]));
    const out = { name: pictureName(got.url), bytes, width: bmp.width, height: bmp.height };
    bmp.close();
    return out;
  }
  if (got.rgba) {
    // Raw pixels: made into a PNG.
    const c = document.createElement('canvas');
    c.width = got.width; c.height = got.height;
    c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(raw(got.rgba).buffer), got.width, got.height), 0, 0);
    const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
    return { name: 'Pasted picture.png', bytes: new Uint8Array(await blob.arrayBuffer()), width: got.width, height: got.height };
  }
  const bytes = raw(got.bytes);
  const bmp = await createImageBitmap(new Blob([bytes]));
  const out = { name: got.name || 'Pasted picture.png', bytes, width: bmp.width, height: bmp.height };
  bmp.close();
  return out;
}

/// Asks for a canvas to open: { name, bytes, path } or null.
export async function openCanvas() {
  if (DESKTOP) {
    const path = await invoke('choose', { kind: 'open' });
    if (!path) return null;
    return { name: baseName(path), bytes: fileBytes(await invoke('read_file', { path })), path };
  }
  const file = await pickFile('.xdraw,.xenify,application/json');
  if (!file) return null;
  return { name: file.name.replace(/\.(xdraw|xenify)$/i, ''), bytes: new Uint8Array(await file.arrayBuffer()), path: null, handle: file.handle || null };
}

/// Asks for a picture: { name, bytes: Uint8Array, type } or null.
export async function openPicture() {
  if (DESKTOP) {
    const path = await invoke('choose', { kind: 'picture' });
    if (!path) return null;
    const got = await invoke('read_file', { path });
    const bytes = got.base64 != null ? Uint8Array.from(atob(got.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(got.text);
    return { name: baseName(path), bytes, type: path.toLowerCase().endsWith('.svg') ? 'image/svg+xml' : '' };
  }
  const file = await pickFile('image/*,.svg');
  if (!file) return null;
  return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), type: file.type };
}

/// Asks for a file to import (SVG, Lottie, a picture, a sound; `kind` 'sound' for sound only): { name, bytes } or null.
export async function openImport(kind = 'import') {
  if (DESKTOP) {
    const path = await invoke('choose', { kind });
    if (!path) return null;
    const got = await invoke('read_file', { path });
    const bytes = got.base64 != null ? Uint8Array.from(atob(got.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(got.text);
    return { name: path.split(/[\\/]/).pop(), bytes, path };
  }
  const file = await pickFile(kind === 'sound' ? 'audio/*' : kind === 'video' ? 'video/*,.avi,.mkv' : kind === 'model' ? '.stl,.obj,.gltf,.glb,.3mf' : '.svg,.json,.lottie,image/*,audio/*,video/*');
  if (!file) return null;
  return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
}

/// Saves `text` (or `bytes`). `path` saves straight there (desktop); without
/// one the person picks where. Returns the path saved to (desktop), or true
/// (browser download), or null if cancelled.
export async function saveFile({ name, text, bytes, path, type = 'application/octet-stream' }) {
  if (DESKTOP) {
    const to = path || await invoke('choose', { kind: 'save', name });
    if (!to) return null;
    if (text != null) await invoke('write_file', { path: to, text });
    else await invoke('write_file', { path: to, base64: toBase64(bytes) });
    return to;
  }
  const blob = new Blob([text != null ? text : bytes], { type });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
  return true;
}

/// Desktop: a file's bytes by its path (linked sound and video).
export async function readBytes(path) {
  const got = await invoke('read_file', { path });
  return got.base64 != null ? Uint8Array.from(atob(got.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(got.text);
}

/// Desktop: asks where to save `name`; the path, or null.
export async function chooseSave(name) {
  return DESKTOP ? invoke('choose', { kind: 'save', name }) : null;
}

/// Desktop: whether ffmpeg is there (video, GIF and PNG frames need it).
export async function hasFfmpeg() {
  return DESKTOP ? invoke('has_ffmpeg') : false;
}

/// Desktop: exports natively (see export_native in main.rs).
export async function exportNative(args) {
  return invoke('export_native', args);
}

export async function exportCancel() {
  if (DESKTOP) await invoke('export_cancel');
}

/// Desktop: calls `f(done, total)` as an export goes; returns a function that stops listening.
export async function onExportProgress(f) {
  if (!DESKTOP || !tauri.event?.listen) return () => {};
  const off = await tauri.event.listen('export-progress', (e) => f(e.payload[0], e.payload[1]));
  return off;
}

/// Desktop: keeps an unsaved canvas so it can be recovered after a crash.
export const autosave = {
  write: (key, name, text) => (DESKTOP ? invoke('autosave_write', { key, name, text }) : Promise.resolve()),
  clear: (key) => (DESKTOP ? invoke('autosave_clear', { key }) : Promise.resolve()),
  leftovers: (keep) => (DESKTOP ? invoke('autosave_list', { keep }) : Promise.resolve([])),
};

/// A canvas by its path (desktop: the Recent list).
export async function openedFromPath(path) {
  return { name: baseName(path), bytes: fileBytes(await invoke('read_file', { path })), path };
}

export async function openedFiles() {
  if (!DESKTOP) return [];
  const paths = await invoke('opened_files');
  const out = [];
  for (const path of paths) {
    try {
      const got = await invoke('read_file', { path });
      // A board link (.xboard) is read as text.
      out.push({ name: baseName(path), text: got.text, bytes: fileBytes(got), path });
    } catch { /* unreadable: skipped */ }
  }
  return out;
}

/// Shows a saved file in the file manager (desktop).
export async function reveal(path) {
  if (DESKTOP) await invoke('reveal', { path });
}

export function quit() {
  if (DESKTOP) invoke('quit');
}

/// A file read by the desktop: its bytes.
function fileBytes(got) {
  return got.base64 != null ? Uint8Array.from(atob(got.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(got.text);
}

function baseName(path) {
  return path.split(/[\\/]/).pop().replace(/\.(xdraw|xenify)$/i, '');
}

function toBase64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

export function pickFile(accept) {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.addEventListener('change', () => resolve(input.files[0] || null));
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

/// A text shipped with Draw (the licences).
export async function bundledText(name) {
  if (DESKTOP) return new TextDecoder().decode(new Uint8Array(await invoke('bundled', { name })));
  const r = await fetch(name);
  if (!r.ok) throw new Error(`${name} is missing`);
  return r.text();
}

// ----- Keys -----
// AI keys live on the Xenify server now; keys an older Draw kept in this
// page's storage are forgotten.

export async function loadSecrets() {
  try {
    localStorage.removeItem('draw.giphy_key');
    const ai = JSON.parse(localStorage.getItem('draw.ai') || '{}');
    if (ai.key) { delete ai.key; localStorage.setItem('draw.ai', JSON.stringify(ai)); }
  } catch { /* nothing kept */ }
}

// ----- Printing (desktop: CUPS) -----

/// A video's size, length, frame rate and sound (desktop: ffprobe).
export async function probeMedia(path) {
  return invoke('probe_media', { path });
}

export async function printers() {
  return DESKTOP ? invoke('printers').catch((e) => [String(e), []]) : [null, []];
}

export async function printPng(base64, printer, copies) {
  return invoke('print_png', { base64, printer, copies });
}

// ----- Fonts -----

/// The desktop's installed font families ([] in the browser).
export async function systemFonts() {
  return DESKTOP ? invoke('system_fonts').catch(() => []) : [];
}

/// The desktop's files for font family `family` (Uint8Arrays).
export async function fontFiles(family) {
  if (!DESKTOP) return [];
  const buf = new Uint8Array(await invoke('font_files', { family }));
  const out = [];
  for (let o = 0; o + 4 <= buf.length;) {
    const n = buf[o] | (buf[o + 1] << 8) | (buf[o + 2] << 16) | (buf[o + 3] << 24);
    out.push(buf.slice(o + 4, o + 4 + n));
    o += 4 + n;
  }
  return out;
}

// ----- Spelling and grammar -----

let worker = null;
let nextId = 1;
const waiting = new Map();
export const checker = { state: 'ready', onState: null };

function web(name, args) {
  if (!worker) {
    worker = new Worker('checker-worker.js', { type: 'module' });
    worker.onmessage = ({ data }) => {
      if (data.spotter) {
        checker.state = data.spotter.state;
        checker.onState?.(data.spotter);
        return;
      }
      const w = waiting.get(data.id);
      if (!w) return;
      waiting.delete(data.id);
      if (data.error) w.reject(new Error(data.error));
      else w.resolve(data.value);
    };
    worker.onerror = (e) => { checker.state = 'failed'; checker.onState?.({ state: 'failed', status: e.message }); };
  }
  const id = nextId++;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    worker.postMessage({ id, name, args });
  });
}

/// Checks texts ({ id, text } each) as `profile`; Write's findings back.
export async function check(texts, { profile = 'standard', dialect = 'auto', ignoreWords = [], ignoreIssues = [] } = {}) {
  const request = {
    prefs: { enabled: true, dialect, ignore_names: true, default_profile: 'standard' },
    profile,
    paras: texts.map((t) => [t.id, t.text]),
    ignore_words: mine ? [...ignoreWords, ...mine] : ignoreWords,
    ignore_issues: ignoreIssues,
  };
  if (DESKTOP) return invoke('check', { request });
  return web('check', { request });
}

function localWords() {
  return DESKTOP ? invoke('words') : web('words', {});
}

// Signed in to a Xenify server: your words there (one list per server; it
// follows you to every computer) instead of this computer's.
let mine = null;
let mineAt = null;
export async function serverWords(server) {
  try { mine = server?.ready ? await server.myWords(await localWords()) : null; } catch { mine = null; }
  mineAt = mine ? server : null;
}

export async function words() {
  return mine ?? localWords();
}

export async function setWords(list) {
  if (mine) { mine = await mineAt.setMyWords(list); return; }
  return DESKTOP ? invoke('set_words', { words: list }) : web('set_words', { words: list });
}

export async function lookUp(word) {
  return DESKTOP ? invoke('look_up', { word }) : web('look_up', { word });
}

/// The browser downloads the mistake-spotter model when asked (the desktop has it installed).
export function loadSpotter() {
  if (!DESKTOP) web('load_spotter', {}).catch(() => {});
}
