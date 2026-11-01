// The browser version works offline after the first visit: every file of
// this build (FILES, filled in by scripts/build-draw-web.sh) is kept in this
// browser by content hash. A new build is fetched in the background and used from the next visit (the
// page says "Update ready").
//
// The spelling and grammar checker and its model (SHARED, at SHARED_AT) are
// kept in a cache of their own that the site's other apps (Write) use too,
// so they're downloaded once for all of them. Each app notes in that cache
// which of its files it needs, and a file goes when no app needs it.

const FILES = /*FILES*/ {};
const CACHE = 'xenify-draw';
const SHARED = /*SHARED*/ {};
const SHARED_AT = /*SHARED_AT*/ 'checker/';
const SHARED_CACHE = 'xenify-checker';

const at = (path) => new URL(path, self.registration.scope);
const keyed = (url, hash) => {
  url.searchParams.set('v', hash);
  return url.href;
};
const keyFor = (path) => keyed(at(path), FILES[path].h);
const sharedKeyFor = (path) => keyed(at(SHARED_AT + path), SHARED[path].h);
// Address → [cache, key] for every file this build serves.
const SERVED = new Map([
  ...Object.keys(FILES).map((p) => [at(p).pathname, [CACHE, keyFor(p)]]),
  ...Object.keys(SHARED).map((p) => [at(SHARED_AT + p).pathname, [SHARED_CACHE, sharedKeyFor(p)]]),
]);
// Where this app notes the shared files it needs.
const NEEDS = at('.xenify-checker-needs').href;

/// A redirect's answer as a plain one. Hosts like Cloudflare Pages send
/// index.html on to the folder (/write/index.html → /write/); an answer that
/// came through a redirect can't be used for a page load (Chrome then shows
/// "This site can't be reached"), so what it brought is kept without it.
async function plain(r) {
  if (!r || !r.redirected) return r;
  return new Response(await r.blob(), { status: r.status, statusText: r.statusText, headers: r.headers });
}

/// Whether a page of this app is open: an update waits for it to say so;
/// with none (or only a page that couldn't load) it takes over at once.
async function noPageOpen() {
  return (await self.clients.matchAll({ type: 'window' })).length === 0;
}

async function keep(cacheName, keys) {
  const cache = await caches.open(cacheName);
  for (const key of keys) {
    if (await cache.match(key)) continue;
    // Keys carry the content hash, so the browser's own copy (the page has
    // just loaded most of them) is always the right one.
    const r = await plain(await fetch(key, { cache: 'force-cache' }));
    if (!r.ok) throw new Error(`${key}: ${r.status}`);
    await cache.put(key, r);
  }
}

/// Deletes the shared files that no app (with a note) needs any more.
async function pruneShared() {
  const cache = await caches.open(SHARED_CACHE);
  const reqs = await cache.keys();
  const need = new Set();
  for (const req of reqs) {
    if (req.url.endsWith('/.xenify-checker-needs')) for (const k of await (await cache.match(req)).json()) need.add(k);
  }
  for (const req of reqs) if (!req.url.endsWith('/.xenify-checker-needs') && !need.has(req.url)) await cache.delete(req);
}

self.addEventListener('install', (e) => {
  e.waitUntil((async () => {
    await keep(CACHE, Object.keys(FILES).map(keyFor));
    // The model is kept as it's downloaded (the checker's worker does that).
    await keep(SHARED_CACHE, Object.keys(SHARED).filter((p) => !p.startsWith('spotter/')).map(sharedKeyFor));
    if (await noPageOpen()) self.skipWaiting();
  })());
});

self.addEventListener('activate', (e) => {
  e.waitUntil((async () => {
    // Forget files of older builds.
    const cache = await caches.open(CACHE);
    const now = new Set(Object.keys(FILES).map(keyFor));
    for (const req of await cache.keys()) if (!now.has(req.url)) await cache.delete(req);
    const shared = await caches.open(SHARED_CACHE);
    await shared.put(NEEDS, new Response(JSON.stringify(Object.keys(SHARED).map(sharedKeyFor))));
    await pruneShared();
    await self.clients.claim();
  })());
});

self.addEventListener('message', (e) => {
  if (e.data === 'update') self.skipWaiting();
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  const scope = at('');
  if (url.origin !== scope.origin) return;
  const inScope = url.pathname.startsWith(scope.pathname);
  const served = SERVED.get(inScope && (req.mode === 'navigate' || url.pathname === scope.pathname) ? at('index.html').pathname : url.pathname);
  if (!served) return;
  const [cacheName, key] = served;
  e.respondWith((async () => {
    const cache = await caches.open(cacheName);
    let hit = await cache.match(key);
    // Kept by an older version straight from a redirect: put right.
    if (hit?.redirected) {
      hit = await plain(hit);
      await cache.put(key, hit.clone()).catch(() => {});
    }
    if (hit) return hit;
    const r = await plain(await fetch(key));
    if (r.ok) await cache.put(key, r.clone()).catch(() => {});
    return r;
  })());
});
