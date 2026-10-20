// Spelling and grammar in a worker of its own, so typing never waits for
// it. Answers { id, name, args } with { id, value } or { id, error }; tells
// the page how the mistake spotter is getting on with { spotter }.

import init, { call, set_spotter } from './pkg/xenify_write_check_web.js';
import { FILES, cachedFetch } from './offline.js';

const ready = init({ module_or_path: cachedFetch('pkg/xenify_write_check_web_bg.wasm') });

const tell = (spotter) => postMessage({ spotter });

let spotterAsked = false;
async function loadSpotter() {
  if (spotterAsked) return;
  spotterAsked = true;
  // The model is in parts (some hosts limit a file's size).
  const parts = Object.keys(FILES).filter((p) => p.startsWith('spotter/model.part')).sort();
  const total = parts.reduce((n, p) => n + FILES[p].n, 0) || 1;
  const got = new Map();
  try {
    tell({ state: 'downloading', progress: 0 });
    const text = async (p) => new Uint8Array(await (await cachedFetch(p)).arrayBuffer());
    const [config, tokenizer] = await Promise.all([text('spotter/config.json'), text('spotter/tokenizer.json')]);
    const blobs = [];
    for (const p of parts) {
      const r = await cachedFetch(p, (n) => {
        got.set(p, n);
        let sum = 0;
        for (const v of got.values()) sum += v;
        tell({ state: 'downloading', progress: Math.min(1, sum / total) });
      });
      blobs.push(await r.blob());
    }
    tell({ state: 'starting' });
    const weights = new Uint8Array(await new Blob(blobs).arrayBuffer());
    await ready;
    set_spotter(config, tokenizer, weights);
    tell({ state: 'loaded', status: JSON.parse(call('spotter_status', '{}')) });
  } catch (e) {
    spotterAsked = false;
    tell({ state: 'failed', status: String(e.message || e) });
  }
}

onmessage = async ({ data: { id, name, args } }) => {
  try {
    await ready;
    if (name === 'load_spotter') {
      loadSpotter();
      postMessage({ id, value: null });
      return;
    }
    postMessage({ id, value: JSON.parse(call(name, JSON.stringify(args || {}))) });
  } catch (e) {
    postMessage({ id, error: String(e.message || e) });
  }
};
