// Fonts. The engine has Inter built in; other fonts are loaded into it: the
// desktop's installed fonts (read by the Rust side), the browser's local
// fonts where it lets pages read them, and font files you add (kept in this
// browser's storage so they're there next time).

import { call, engineRaw } from './engine.js';
import * as platform from './platform.js';

const loaded = new Set();
let local = null; // family → [FontData] (browsers with local font access)
let system = null; // installed family names

const DB = 'draw-fonts';
function db() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore('files');
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}
async function stored() {
  try {
    const d = await db();
    return await new Promise((resolve) => {
      const out = [];
      const c = d.transaction('files').objectStore('files').openCursor();
      c.onsuccess = () => { const cur = c.result; if (!cur) return resolve(out); out.push([cur.key, cur.value]); cur.continue(); };
      c.onerror = () => resolve(out);
    });
  } catch { return []; }
}
async function keep(name, bytes) {
  try {
    const d = await db();
    d.transaction('files', 'readwrite').objectStore('files').put(bytes, name);
  } catch { /* not kept */ }
}

function add(bytes) {
  const got = JSON.parse(engineRaw().add_font(bytes) || '[]');
  for (const f of got) loaded.add(f);
  return got;
}

/// Font files added before (browser storage); call once at start.
export async function restore() {
  for (const f of call('fonts')) loaded.add(f);
  for (const [, bytes] of await stored()) add(bytes);
}

/// Every family you can pick: loaded ones, then installed ones.
export async function families() {
  if (system === null) {
    system = await platform.systemFonts();
    if (!system.length && typeof window.queryLocalFonts === 'function') {
      try {
        // Only asks once the person has opened the font list (the browser shows its own prompt).
        local = new Map();
        for (const f of await window.queryLocalFonts()) {
          if (!local.has(f.family)) local.set(f.family, []);
          local.get(f.family).push(f);
        }
        system = [...local.keys()];
      } catch { system = []; local = null; }
    }
  }
  const all = new Set([...loaded, ...system]);
  return [...all].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
}

// Fonts that ship with Draw (ui/fonts/), loaded the first time a canvas uses one.
const SHIPPED = ['Figtree'];
const shipping = new Map();
function shipped(family) {
  if (!shipping.has(family)) {
    shipping.set(family, fetch(`fonts/${family}.ttf`).then((r) => (r.ok ? r.arrayBuffer() : null)).then((b) => { if (b) add(new Uint8Array(b)); }).catch(() => {}));
  }
  return shipping.get(family);
}

/// Makes sure `family` is in the engine (loading it from the computer if need be). False if it can't be found.
export async function ensure(family) {
  if (!family || loaded.has(family)) return true;
  if (SHIPPED.includes(family)) { await shipped(family); if (loaded.has(family)) return true; }
  let files = [];
  if (platform.DESKTOP) files = await platform.fontFiles(family);
  else if (local?.has(family)) files = await Promise.all(local.get(family).map(async (f) => new Uint8Array(await (await f.blob()).arrayBuffer())));
  for (const b of files) add(b);
  return loaded.has(family);
}

/// Loads every font the canvas uses that the engine doesn't have yet.
export async function ensureUsed(families) {
  let any = false;
  for (const f of new Set(families)) if (!loaded.has(f) && (await ensure(f))) any = true;
  return any;
}

/// Asks for font files and adds them; the families they hold.
export async function addFile() {
  const file = await platform.pickFile('.ttf,.otf,.ttc,font/ttf,font/otf');
  if (!file) return [];
  const bytes = new Uint8Array(await file.arrayBuffer());
  const got = add(bytes);
  if (got.length && !platform.DESKTOP) await keep(file.name, bytes);
  return got;
}
