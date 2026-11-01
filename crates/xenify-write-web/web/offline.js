// Every file of this version, kept in this browser by content hash (Cache
// Storage), shared by the page, the checker's worker and sw.js. A file is
// downloaded once per version it changes in; later visits and offline use
// read the kept copy.

/// Path → { h: content hash, n: size in bytes }, filled in by
/// scripts/build-write-web.sh.
export const FILES = /*FILES*/ {};

export const CACHE = 'xenify-write';

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

/// How much this browser keeps for Write, in bytes (null: not known). That
/// includes the spelling and grammar checker, which the site's other apps
/// share.
export async function keptSize() {
  try {
    let n = 0;
    for (const name of [CACHE, 'xenify-checker']) {
      const cache = await caches.open(name);
      for (const req of await cache.keys()) {
        const r = await cache.match(req);
        n += Number(r.headers.get('content-length')) || 0;
      }
    }
    return n;
  } catch {
    return null;
  }
}

/// Removes the kept copy (it's downloaded again next time Write opens). The
/// checker stays while another app on the site still uses it.
export async function forgetOffline() {
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    const sw = reg?.active;
    if (sw) {
      const { port1, port2 } = new MessageChannel();
      const done = new Promise((resolve) => { port1.onmessage = resolve; setTimeout(resolve, 5000); });
      sw.postMessage('forget', [port2]);
      await done;
    }
  } catch { /* */ }
  try { await caches.delete(CACHE); } catch { /* */ }
}
