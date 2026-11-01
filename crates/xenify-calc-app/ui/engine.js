// The engine: Xenify Calc's Rust core as WebAssembly, in this page on every
// platform. Calls are synchronous (it runs on this thread).

import init, { Engine } from './pkg/xenify_calc_web.js';

let engine = null;

export async function start() {
  await init({ module_or_path: window.XENIFY_WASM_URL || 'pkg/xenify_calc_web_bg.wasm' });
  engine = new Engine();
}

// Editing together: changes to send to the server (server.js sets `send`
// and `active`).
export const live = { send: null, active: false };

/// `name` with `args`; throws the engine's message when it says no.
export function call(name, args = {}) {
  const out = JSON.parse(engine.call(name, JSON.stringify(args)));
  if (live.active && live.send) {
    const batches = JSON.parse(engine.live_out());
    if (batches.length) live.send(batches);
  }
  if (out && typeof out === 'object' && !Array.isArray(out) && typeof out.error === 'string' && Object.keys(out).length === 1) {
    throw new Error(out.error);
  }
  return out;
}

export const tabs = {
  open: () => engine.open_tab(),
  openFile: (bytes, name) => engine.open_file(bytes, name),
  switch: (id) => engine.switch_tab(id),
  close: (id) => engine.close_tab(id),
  front: () => engine.front_tab(),
};

/// The front book as a file of `kind` (xcalc, xlsx, xlsm, csv, tsv; CSV and TSV:
/// sheet `sheet`). Saving in its own kind counts as saved.
export const saveFile = (kind, sheet) => engine.save_file(kind, sheet ?? undefined);
/// What Save writes for the front book.
export const bookKind = () => engine.format();
/// The front book as a .xcalc file for the recovery copy (still unsaved).
export const recoveryCopy = () => engine.recovery_copy();
