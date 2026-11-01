// Spelling and grammar in a worker of its own (the browser version), so
// drawing never waits for it: Xenify Write's checker as WebAssembly, shared
// with the site's other apps (checker/kept.js; scripts/build-draw-web.sh
// points this at the shared copy). Answers { id, name, args } with
// { id, value } or { id, error }; tells the page how the mistake spotter is
// getting on with { spotter }.

import { ready, call, loadSpotter } from './checker/kept.js';

const tell = (spotter) => postMessage({ spotter });

let spotterAsked = false;
async function startSpotter() {
  if (spotterAsked) return;
  spotterAsked = true;
  try {
    await loadSpotter((progress) => tell({ state: 'downloading', progress }), () => tell({ state: 'starting' }));
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
      startSpotter();
      postMessage({ id, value: null });
      return;
    }
    postMessage({ id, value: JSON.parse(call(name, JSON.stringify(args || {}))) });
  } catch (e) {
    postMessage({ id, error: String(e.message || e) });
  }
};
