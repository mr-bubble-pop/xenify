// What differs between the desktop app (Tauri) and the browser: files and
// keeping unsaved work. The desktop asks the Rust side; the browser uses
// file pickers and downloads, and keeps unsaved work in this browser.

const tauri = window.__TAURI__;
export const DESKTOP = !!tauri && !window.XENIFY_WEB;
const invoke = DESKTOP ? tauri.core.invoke : null;

export function log(text) {
  if (DESKTOP) invoke('log', { text }).catch(() => {});
  else if (window.XENIFY_LOG) fetch('/log', { method: 'POST', body: text }).catch(() => {});
}

export const toB64 = (bytes) => {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
export const fromB64 = (b) => Uint8Array.from(atob(b), (c) => c.charCodeAt(0));
export const fileName = (path) => path.split(/[\\/]/).pop();
export const baseName = (path) => fileName(path).replace(/\.xplan$/i, '');
export const extensionOf = (path) => (/\.([a-z0-9]+)$/i.exec(path)?.[1] || '').toLowerCase();

const KINDS = {
  xplan: { description: 'Xenify Plan plan', mime: 'application/x-xenify-plan' },
};
const OPENABLE = '.xplan';

function pickFile(accept) {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.addEventListener('change', () => resolve(input.files[0] || null));
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

/// Asks for a plan to open: { name (with its ending), bytes, path, handle } or null.
export async function openPlan() {
  if (DESKTOP) {
    const path = await invoke('choose', { kind: 'open' });
    if (!path) return null;
    return { name: fileName(path), bytes: fromB64(await invoke('read_file', { path })), path };
  }
  if (window.showOpenFilePicker) {
    try {
      const [handle] = await window.showOpenFilePicker({ types: [{ description: 'Plans', accept: { 'application/octet-stream': OPENABLE.split(',') } }] });
      const file = await handle.getFile();
      return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), path: null, handle };
    } catch (e) {
      if (e.name === 'AbortError') return null;
    }
  }
  const file = await pickFile(OPENABLE);
  if (!file) return null;
  return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), path: null, handle: null };
}

/// A file named on the command line.
export async function readPath(path) {
  return { name: fileName(path), bytes: fromB64(await invoke('read_file', { path })), path };
}

/// Where to save a plan named `name`: the desktop's path (the
/// ending chosen there decides the kind), or null when cancelled.
export async function chooseSave(kind, name) {
  if (!DESKTOP) return null;
  const where = await invoke('choose', { kind, name });
  if (!where) return null;
  return KINDS[extensionOf(where)] ? where : `${where}.${kind}`;
}

/// Writes `bytes` (named `name`): to `path` (or `handle`) when
/// there is one, else asks where. The desktop gives the path; the browser the
/// file handle, `true` for a download, or null when cancelled.
export async function savePlan({ name, bytes, path, handle, kind = 'xplan' }) {
  if (DESKTOP) {
    const where = path || (await chooseSave(kind, name));
    if (!where) return null;
    await invoke('write_file', { path: where, base64: toB64(bytes) });
    return where;
  }
  const k = KINDS[kind] || KINDS.xplan;
  if (handle || window.showSaveFilePicker) {
    try {
      const h = handle || (await window.showSaveFilePicker({ suggestedName: name, types: [{ description: k.description, accept: { [k.mime]: [`.${kind}`] } }] }));
      const w = await h.createWritable();
      await w.write(bytes);
      await w.close();
      return h;
    } catch (e) {
      if (e.name === 'AbortError') return null;
    }
  }
  const url = URL.createObjectURL(new Blob([bytes], { type: k.mime }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
  return true;
}

// ----- Keeping unsaved work (recovery only; files are written by Save) -----

const KEPT = 'plan.unsaved.';
export const autosave = {
  async write(key, name, path, bytes) {
    if (DESKTOP) return invoke('autosave_write', { key, name, path: path || null, base64: toB64(bytes) });
    try { localStorage.setItem(KEPT + key, JSON.stringify({ name, base64: toB64(bytes), when: Math.floor(Date.now() / 1000) })); } catch { /* too big for this browser */ }
  },
  async clear(key) {
    if (DESKTOP) return invoke('autosave_clear', { key });
    try { localStorage.removeItem(KEPT + key); } catch { /* none */ }
  },
  /// Left over from an earlier run: [{ key, name, path, when, base64 }].
  async leftovers(run) {
    if (DESKTOP) return invoke('autosave_list', { keep: run });
    const out = [];
    try {
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (!k.startsWith(KEPT) || k.startsWith(KEPT + run)) continue;
        // One unreadable entry (damaged, or kept by another version) mustn't hide the rest.
        let v = null;
        try { v = JSON.parse(localStorage.getItem(k)); } catch { /* skip it */ }
        if (!v || typeof v.base64 !== 'string') continue;
        out.push({ key: k.slice(KEPT.length), name: v.name || 'Untitled', path: null, when: v.when, base64: v.base64 });
      }
    } catch { /* none */ }
    return out;
  },
};

export async function openedFiles() {
  return DESKTOP ? invoke('opened_files') : [];
}

export function quit() {
  if (DESKTOP) invoke('quit');
}

export async function bundled(name) {
  if (DESKTOP) return new TextDecoder().decode(new Uint8Array(await invoke('bundled', { name })));
  const r = await fetch(name);
  return r.ok ? r.text() : '';
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
