// The browser build: the screens call `window.__TAURI__.core.invoke` as on
// the desktop; here the calls go to WebAssembly, and files go through the
// browser (picking a file, downloading a copy). Loaded before the screens.
//
// Loading is split so the first page shows quickly: the document engine
// comes first; then the maths/serif/emoji fonts (fonts.bin); then the
// spelling and grammar checker in a worker of its own, and last the mistake
// spotter model. Every file is kept in this browser (Cache Storage, by
// content hash) so later visits download nothing that hasn't changed, and
// sw.js serves them when offline.

import init, { call, late_fonts } from './pkg/xenify_write_web.js';
import { FILES, cachedFetch, forgetOffline, keptSize } from './offline.js';

// ?debug: errors and loading steps are reported to the server's log (for
// testing without devtools; see scripts/serve-write-web.py).
const DEBUG = location.search.includes('debug');
const report = (m) => DEBUG && fetch(`/__error?${encodeURIComponent(m)}`).catch(() => {});
if (DEBUG) {
  window.addEventListener('error', (e) => report(`${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener('unhandledrejection', (e) => report(`unhandled: ${e.reason && (e.reason.stack || e.reason)}`));
  for (const ev of ['xenify-fonts', 'xenify-recheck', 'xenify-update-ready']) window.addEventListener(ev, () => report(`${ev} at ${Math.round(performance.now())} ms`));
}

const ready = init({ module_or_path: cachedFetch('pkg/xenify_write_web_bg.wasm') }).then(() => {
  // Settings are kept in this browser.
  try {
    const saved = localStorage.getItem('xenify-write-prefs');
    if (saved) wasm('set_prefs', { prefs: JSON.parse(saved) });
  } catch { /* private window */ }
});

function wasm(name, args) {
  return JSON.parse(call(name, JSON.stringify(args || {})));
}

// ----- Offline copy -----

// The service worker serves the kept files when there's no connection. A
// new version waits until it's asked to take over (Help › … or the status
// note), so a page never mixes files of two versions.
let updateReady = null;
const sw = (async () => {
  if (!('serviceWorker' in navigator) || location.search.includes('nosw')) return;
  try {
    const reg = await navigator.serviceWorker.register('sw.js');
    const waiting = () => {
      if (reg.waiting && navigator.serviceWorker.controller) {
        updateReady = reg.waiting;
        window.dispatchEvent(new Event('xenify-update-ready'));
      }
    };
    waiting();
    reg.addEventListener('updatefound', () => reg.installing?.addEventListener('statechange', waiting));
    let reloading = false;
    navigator.serviceWorker.addEventListener('controllerchange', () => {
      if (updateReady && !reloading) { reloading = true; location.reload(); }
    });
  } catch { /* not served over https, or blocked */ }
})();

/// Takes the new version (it's already downloaded) and reloads.
window.XENIFY_UPDATE = () => updateReady?.postMessage('take-over');

// ----- Fonts that come after the first page -----

let fontsAsked = false;
function fetchFonts() {
  if (fontsAsked) return;
  fontsAsked = true;
  cachedFetch('fonts.bin')
    .then((r) => r.arrayBuffer())
    .then((b) => {
      late_fonts(new Uint8Array(b));
      window.dispatchEvent(new Event('xenify-fonts'));
    })
    .catch((e) => console.error('fonts', e));
}

// ----- Spelling and grammar (a worker of its own) -----

let checkerWorker = null;
let asked = 0;
const waiting = new Map();
let spotter = { state: 'none', progress: 0, status: '' };

function startChecker() {
  if (checkerWorker) return;
  checkerWorker = new Worker('checker-worker.js', { type: 'module' });
  checkerWorker.onmessage = ({ data }) => {
    if (data.spotter) {
      if (data.spotter.state !== spotter.state) report(`spotter ${data.spotter.state} ${data.spotter.status || ''} at ${Math.round(performance.now())} ms`);
      spotter = data.spotter;
      if (spotter.state === 'loaded') window.dispatchEvent(new Event('xenify-recheck'));
      window.dispatchEvent(new Event('xenify-spotter'));
      return;
    }
    const w = waiting.get(data.id);
    waiting.delete(data.id);
    if (w) data.error ? w.reject(new Error(data.error)) : w.resolve(data.value);
  };
  let words = [];
  try { words = JSON.parse(localStorage.getItem('xenify-write-words') || '[]'); } catch { /* */ }
  checker('set_words', { words });
}

function checker(name, args = {}) {
  startChecker();
  return new Promise((resolve, reject) => {
    const id = ++asked;
    waiting.set(id, { resolve, reject });
    checkerWorker.postMessage({ id, name, args });
  });
}

/// Asks the worker to load the spotter model (once, when it's switched on).
function wantSpotter(prefs) {
  if (prefs?.proofing?.spotter?.enabled && spotter.state === 'none') {
    spotter = { ...spotter, state: 'asked' };
    checker('load_spotter').catch(() => {});
  }
}

function spotterText() {
  switch (spotter.state) {
    case 'downloading': return `Downloading it to keep in this browser: ${Math.round(spotter.progress * 100)}%`;
    case 'starting': return 'Starting it';
    case 'loaded': return spotter.status;
    case 'failed': return `Couldn’t get it: ${spotter.status}. It’s tried again next time you open Write`;
    default: return '';
  }
}

/// After the first page is on screen: fonts, then the checker, then the
/// model. Big downloads wait for the offline copy's worker, so nothing is
/// downloaded twice.
let started = false;
function afterFirstPage() {
  if (started) return;
  started = true;
  setTimeout(fetchFonts, 0);
  sw.finally(() => {
    startChecker();
    try { wantSpotter(JSON.parse(localStorage.getItem('xenify-write-prefs') || 'null') || wasm('prefs', {})); } catch { /* */ }
    // Ask the browser not to clear the kept copy when space runs low.
    navigator.storage?.persist?.().catch(() => {});
  });
}

// ----- Files -----

const picked = new Map(); // "web-file:N" → File
let next = 1;

function pickFile(accept) {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.addEventListener('change', () => {
      const f = input.files && input.files[0];
      if (!f) return resolve(null);
      const key = `web-file:${next++}`;
      picked.set(key, f);
      resolve(key);
    });
    input.click();
  });
}

async function base64(file) {
  const buf = new Uint8Array(await file.arrayBuffer());
  let s = '';
  for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(s);
}

function download(name, data) {
  const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
  const url = URL.createObjectURL(new Blob([bytes]));
  const a = Object.assign(document.createElement('a'), { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

// Unsaved work, kept in this browser in case the tab closes.
const KEPT = 'xenify-write-unsaved';
let kept = [];
try { kept = JSON.parse(localStorage.getItem(KEPT) || '[]'); } catch { kept = []; }
let offered = kept; // what was left by an earlier visit
function keep() {
  try {
    const now = wasm('snapshot', {}).map((d) => ({ ...d, time: Date.now() }));
    localStorage.setItem(KEPT, JSON.stringify(now));
  } catch { /* too big, or private window */ }
}

async function engine(name, args) {
  if (name === 'autosave') { keep(); return null; }
  if (name === 'recovery') {
    return { left: [], orphans: offered.map((d, i) => ({ key: String(i), title: d.title, words: d.words, when: new Date(d.time).toLocaleString() })) };
  }
  if (name === 'recover') {
    const d = offered[Number(args.key)];
    if (!d) return wasm('view', {});
    offered = offered.filter((x) => x !== d);
    return wasm('open_bytes', { name: `${d.title || 'Untitled'}.igdoc`, data: d.data, have: args.have });
  }
  if (name === 'forget_recovery') { offered = offered.filter((_, i) => String(i) !== args.key); return null; }
  if (name === 'discard_all_autosaves') { try { localStorage.removeItem(KEPT); } catch { /* */ } return null; }
  if (name === 'forget_offline') { await forgetOffline(); return wasm('prefs', {}); }
  // Files: paths are tokens for files picked in the browser.
  if (name === 'open' && picked.has(args.path)) {
    const f = picked.get(args.path);
    return wasm('open_bytes', { name: f.name, data: await base64(f), have: args.have });
  }
  if (name === 'insert_picture' && picked.has(args.path)) {
    const f = picked.get(args.path);
    return wasm('insert_picture_bytes', { name: f.name, data: await base64(f), have: args.have });
  }
  if (name === 'save' || name === 'export') {
    setTimeout(keep, 0);
    const ext = (args.path || '').split('.').pop() || 'md';
    const out = wasm('save_bytes', { ext, have: args.have });
    download(out.file.name, out.file.data);
    delete out.file;
    return name === 'export' ? { path: args.path } : out;
  }
  if (name === 'lookup') {
    // The meanings come from the checker's worker (the word list is there).
    const out = wasm('lookup', args);
    if (out && out.word) Object.assign(out, await checker('look_up', { word: out.word }));
    return out;
  }
  const out = wasm(name, args);
  if (name === 'view' && !started) report(`first page at ${Math.round(performance.now())} ms, controlled: ${!!navigator.serviceWorker?.controller}`);
  if (name === 'view') afterFirstPage();
  if (name === 'set_prefs' || name === 'clear_recent') {
    try { localStorage.setItem('xenify-write-prefs', JSON.stringify(out)); } catch { /* private window */ }
    wantSpotter(out);
  }
  return out;
}

// The rules answer at once; the spotter takes about half a second a
// paragraph here (no threads in a browser worker), so it goes through the
// paragraphs one at a time afterwards and its findings are added when it's
// done. A new check in the meantime only waits for one paragraph.
let spotRound = 0;
async function proofread() {
  const { doc, request } = wasm('proof_request', {});
  const spotting = request.prefs.spotter.enabled && spotter.state === 'loaded';
  const rulesOnly = { ...request, prefs: { ...request.prefs, spotter: { ...request.prefs.spotter, enabled: false } } };
  const issues = await checker('check', { request: spotting ? rulesOnly : request });
  wasm('set_issues', { doc, texts: request.paras, issues });
  if (spotting) spotAfter(doc, request);
  return null;
}

async function spotAfter(doc, request) {
  const round = ++spotRound;
  for (const para of request.paras) {
    if (round !== spotRound) return; // the text changed: a newer round has started
    await checker('check', { request: { ...request, paras: [para] } }); // kept in the checker's cache
  }
  if (round !== spotRound) return;
  const issues = await checker('check', { request });
  wasm('set_issues', { doc, texts: request.paras, issues });
  window.dispatchEvent(new Event('xenify-issues'));
}

window.XENIFY_WEB = true;
/// For Settings: how much this browser keeps, and the spotter's state.
window.XENIFY_OFFLINE = { keptSize, spotterText, files: FILES };
window.__TAURI__ = {
  core: {
    async invoke(cmd, args = {}) {
      await ready;
      switch (cmd) {
        case 'engine': return engine(args.name, args.args || {});
        case 'proofread': return proofread();
        case 'words': return checker('words');
        case 'set_words':
          try { localStorage.setItem('xenify-write-words', JSON.stringify(args.words)); } catch { /* */ }
          return checker('set_words', { words: args.words });
        case 'spotter_status': return spotterText();
        case 'choose':
          if (args.kind === 'open') return pickFile('.md,.markdown,.docx,.igdoc,.html,.htm,.txt');
          if (args.kind === 'picture') return pickFile('image/*');
          return args.name || 'Untitled.md'; // save: the browser downloads it
        case 'ai': return { has_key: false, web: true, presets: [], model: '' };
        case 'quit': return null;
        case 'open_link': window.open(args.url, '_blank', 'noopener'); return null;
        default: throw new Error(`not available in the browser: ${cmd}`);
      }
    },
  },
  event: { listen: async () => () => {} },
};
