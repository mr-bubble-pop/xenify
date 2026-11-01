// The engine: Xenify Plan's Rust core as WebAssembly, in this page on every
// platform. Calls are synchronous (it runs on this thread).

import init, { Engine } from './pkg/xenify_plan_web.js';

let engine = null;

export async function start() {
  await init({ module_or_path: window.XENIFY_WASM_URL || 'pkg/xenify_plan_web_bg.wasm' });
  engine = new Engine();
}

// Editing together: steps to send to the server (server.js sets `send` and `active`).
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
  openFile: (bytes) => engine.open_file(bytes),
  switch: (id) => engine.switch_tab(id),
  close: (id) => engine.close_tab(id),
  front: () => engine.front_tab(),
};

/// The front plan as a .xplan file.
export const saveFile = () => engine.save_file();
