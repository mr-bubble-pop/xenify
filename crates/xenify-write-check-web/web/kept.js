// Xenify Write's spelling and grammar checker for the browser, shared by
// every Xenify app on the site (Write, Draw). The WebAssembly and the
// mistake-spotter model are downloaded once and kept in this browser by
// content hash, in a cache of their own (CACHE) that each app's offline
// copy (its sw.js) also serves from, so opening a second app downloads
// none of it again. Imported by the apps' checker workers.

import init, { call, set_spotter } from './xenify_write_check_web.js';

/// Path in this folder → { h: content hash, n: size in bytes }, filled in by
/// scripts/build-checker-web.sh.
export const FILES = /*FILES*/ {};

export const CACHE = 'xenify-checker';

export { call };

/// The kept file's address: its path plus its hash (so the browser's own
/// cache can never hand back an older copy).
export function keyFor(path) {
  const url = new URL(path, import.meta.url);
  const hash = FILES[path]?.h;
  if (hash) url.searchParams.set('v', hash);
  return url.href;
}

/// `path` from the kept copy, or downloaded and kept. `onBytes(n)` is told
/// how much has arrived (for big files).
export async function cachedFetch(path, onBytes) {
  const key = keyFor(path);
  let cache = null;
  try { cache = await caches.open(CACHE); } catch { /* not https, or a private window */ }
  const hit = cache && (await cache.match(key));
  if (hit) {
    onBytes?.(Number(hit.headers.get('content-length')) || 0);
    return hit;
  }
  const r = await fetch(key);
  if (!r.ok) throw new Error(`${path}: ${r.status} ${r.statusText}`);
  if (!cache && !onBytes) return r;
  // Read it once: count it as it arrives, then keep it.
  const reader = r.body.getReader();
  const parts = [];
  let n = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    parts.push(value);
    n += value.length;
    onBytes?.(n);
  }
  const body = new Blob(parts);
  const headers = { 'content-type': r.headers.get('content-type') || 'application/octet-stream', 'content-length': String(body.size) };
  const kept = new Response(body, { headers });
  if (cache) await cache.put(key, kept.clone()).catch(() => {});
  return kept;
}

/// Resolves once the checker is ready for `call`.
export const ready = init({ module_or_path: cachedFetch('xenify_write_check_web_bg.wasm') });

/// Downloads (or reads the kept copy of) the mistake spotter and starts it.
/// `onProgress(0…1)` is told how the download is getting on, `onStarting()`
/// when it's all here.
export async function loadSpotter(onProgress, onStarting) {
  // The model is in parts (some hosts limit a file's size).
  const parts = Object.keys(FILES).filter((p) => p.startsWith('spotter/model.part')).sort();
  const total = parts.reduce((n, p) => n + FILES[p].n, 0) || 1;
  const got = new Map();
  onProgress?.(0);
  const bytes = async (p) => new Uint8Array(await (await cachedFetch(p)).arrayBuffer());
  const [config, tokenizer] = await Promise.all([bytes('spotter/config.json'), bytes('spotter/tokenizer.json')]);
  const blobs = [];
  for (const p of parts) {
    const r = await cachedFetch(p, (n) => {
      got.set(p, n);
      let sum = 0;
      for (const v of got.values()) sum += v;
      onProgress?.(Math.min(1, sum / total));
    });
    blobs.push(await r.blob());
  }
  onStarting?.();
  const weights = new Uint8Array(await new Blob(blobs).arrayBuffer());
  await ready;
  set_spotter(config, tokenizer, weights);
}
