// Write's offline copy: answers for every file of this version from the
// copy kept in this browser (see offline.js), so Write opens with no
// connection. A new version downloads what changed before it takes over,
// and takes over only when the page asks (after "Update ready").

const FILES = /*FILES*/ {};
const CACHE = 'xenify-write';
// Small files the page loads before it can keep them itself.
const SHELL = Object.keys(FILES).filter((p) => !p.endsWith('.wasm') && !p.startsWith('spotter/') && p !== 'fonts.bin');

const keyFor = (path) => {
  const url = new URL(path, self.registration.scope);
  url.searchParams.set('v', FILES[path].h);
  return url.href;
};

async function keep(cache, paths) {
  for (const p of paths) {
    const key = keyFor(p);
    if (await cache.match(key)) continue;
    const r = await fetch(key);
    if (!r.ok) throw new Error(`${p}: ${r.status}`);
    await cache.put(key, r);
  }
}

self.addEventListener('install', (e) => {
  e.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // The first time, the page itself keeps the big files as it loads them;
    // an update gets everything that changed now, so it works offline at once.
    const update = !!self.registration.active;
    await keep(cache, update ? Object.keys(FILES) : SHELL);
    if (!update) self.skipWaiting();
  })());
});

self.addEventListener('activate', (e) => {
  e.waitUntil((async () => {
    // Forget files of older versions.
    const cache = await caches.open(CACHE);
    const now = new Set(Object.keys(FILES).map(keyFor));
    for (const req of await cache.keys()) if (!now.has(req.url)) await cache.delete(req);
    await self.clients.claim();
  })());
});

self.addEventListener('message', (e) => {
  if (e.data === 'take-over') self.skipWaiting();
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  const scope = new URL(self.registration.scope);
  if (url.origin !== scope.origin || !url.pathname.startsWith(scope.pathname)) return;
  let path = decodeURIComponent(url.pathname.slice(scope.pathname.length));
  if (req.mode === 'navigate' || path === '') path = 'index.html';
  if (!FILES[path]) return;
  e.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const key = keyFor(path);
    const hit = await cache.match(key);
    if (hit) return hit;
    const r = await fetch(key);
    if (r.ok) await cache.put(key, r.clone()).catch(() => {});
    return r;
  })());
});
