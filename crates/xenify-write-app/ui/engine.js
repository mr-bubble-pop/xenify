// Calls into the document engine. On the desktop they go to the Rust side
// through Tauri; a browser build answers the same calls from WebAssembly.
// Calls run one after another, in the order they were made, so typing
// fast never applies keys out of order.

const invoke = window.__TAURI__ ? window.__TAURI__.core.invoke : null;
let chain = Promise.resolve();

export function call(name, args = {}) {
  const p = chain.then(() => invoke('engine', { name, args }));
  chain = p.catch(() => {});
  return p;
}
