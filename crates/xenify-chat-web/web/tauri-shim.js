// Xenify Chat in the browser: the screens call `window.__TAURI__` as in the
// desktop app; here the calls go to the shared core (WebAssembly: signing
// in, meetings, calendar, address books), the page keeps the WebSocket and
// does HTTP, files go up and down part by part, and spelling goes to the site's checker in a worker. Email is kept by the
// server (/v1/mail/…), which reaches the mail servers. Loaded before the screens.

import init, * as chat from './pkg/xenify_chat_web.js';

const CHECKER_AT = new URL(window.XENIFY_CHECKER_AT || 'checker/', import.meta.url);
const DRAW_AT = window.XENIFY_DRAW_AT || 'draw/';
window.XENIFY_WEB = true;
window.XENIFY_DRAW_AT = DRAW_AT;

// ----- Kept in this browser -----

const kept = (k, fallback) => {
  try {
    const v = localStorage.getItem(`xenify-chat-${k}`);
    return v == null ? fallback : JSON.parse(v);
  } catch {
    return fallback;
  }
};
const keep = (k, v) => {
  try {
    if (v == null) localStorage.removeItem(`xenify-chat-${k}`);
    else localStorage.setItem(`xenify-chat-${k}`, JSON.stringify(v));
  } catch { /* a private window */ }
};

// ----- Events for the screens -----

const listeners = new Map();
function emit(event, payload) {
  for (const cb of listeners.get(event) || []) {
    try { cb({ payload }); } catch (e) { console.error(e); }
  }
}

// ----- Which server -----

// Served by a Xenify server: that one. Otherwise (xenify.net) the one you
// type, or the one a link the server made names (?link=<server>/join/#…).

/** `<server>/join/#<id>.<secret>` → [server, "<id>.<secret>"], else null. */
function parseLink(text) {
  const m = /^(https?:\/\/[^\s#]+?)\/join\/?#([^.\s]+\.[^\s]+)$/.exec(String(text || '').trim());
  return m ? [m[1], m[2]] : null;
}

const params = new URLSearchParams(location.search);
let server = kept('server', '');
const opened = parseLink(params.get('link'));
const linkServer = opened?.[0] || '';
const linkCode = opened?.[1] || '';
if (linkServer) server = linkServer;
const atHome = fetch('/v1/info', { cache: 'no-store' })
  .then((r) => r.ok)
  .catch(() => false)
  .then((ok) => {
    if (ok && !linkServer) server = location.origin;
    return ok;
  });

// ----- HTTP as the signed-in person -----

let session = kept('session', null); // { server, token, user }

// Served by a Xenify server, Chat shares the sign-in of its other pages
// (Files, the admin pages, the apps): signed in there is signed in here,
// and signing in happens on Files.
const SHARED = 'xenify-session';
function sharedSession() {
  try {
    const s = JSON.parse(localStorage.getItem(SHARED) || 'null');
    if (!s?.token || !s.account) return null;
    const a = s.account;
    return { server: location.origin, token: s.token, user: { id: a.id, username: a.username, display_name: a.display_name || '', app: false, email: a.email || '' } };
  } catch {
    return null;
  }
}
/** Off to Files to sign in; it comes back here. Never returns. */
function signInOnFiles() {
  location.replace(`/files/?back=${encodeURIComponent(location.pathname + location.search + location.hash)}`);
  return new Promise(() => {});
}
const servedHere = atHome.then((home) => home && !linkServer);
servedHere.then((here) => {
  if (!here) return;
  window.addEventListener('storage', (e) => {
    if (e.key === SHARED && sharedSession()?.token !== session?.token) location.reload();
  });
});

/** "wrong email address or password" → "Wrong email address or password." */
function said(text, status) {
  let why = '';
  try { why = JSON.parse(text).error || ''; } catch { /* */ }
  if (!why) return `The server said no (${status}).`;
  return why.charAt(0).toUpperCase() + why.slice(1) + (/[.!?]$/.test(why) ? '' : '.');
}

async function http(token, path, body, opts = {}) {
  const headers = {};
  const t = token || session?.token;
  if (t) headers.Authorization = `Bearer ${t}`;
  if (body !== undefined && !(body instanceof Uint8Array) && !(body instanceof ArrayBuffer)) headers['Content-Type'] = 'application/json';
  if (body instanceof Uint8Array || body instanceof ArrayBuffer) headers['Content-Type'] = 'application/octet-stream';
  let r;
  try {
    r = await fetch(`${server}${path}`, { method: opts.method || (body === undefined ? 'GET' : 'POST'), headers, body });
  } catch {
    throw "Can't reach the server. Check you're online and try again.";
  }
  if (opts.bytes && r.ok) return new Uint8Array(await r.arrayBuffer());
  const text = await r.text();
  if (r.status === 204) return '';
  if (r.ok) return text;
  throw said(text, r.status);
}

// ----- The connection -----

let ws = null;
let outbox = [];
let pause = 1000;
let wanted = false;

function status(state) {
  emit('net', { state });
}

function connect() {
  if (!session || ws) return;
  wanted = true;
  status('connecting');
  const sock = new WebSocket(`${session.server.replace(/^http/, 'ws')}/v1/ws`, ['xenify-chat', session.token]);
  ws = sock;
  let opened = false;
  sock.onopen = () => {
    opened = true;
    pause = 1000;
    status('online');
    const waiting = outbox;
    outbox = [];
    for (const m of waiting) sock.send(m);
  };
  sock.onmessage = (e) => {
    try { chat.heard(e.data); } catch (err) { console.error('heard', err); }
  };
  sock.onclose = async () => {
    // A connection already replaced (signed in again) or closed on purpose:
    // its end says nothing about the one in use.
    if (ws !== sock) return;
    ws = null;
    if (!wanted) return;
    // Never opened: is the sign-in still good?
    if (!opened) {
      try {
        await http('', '/v1/me');
      } catch (e) {
        if (String(e).includes('(401)') || /sign in/i.test(String(e))) return signedOut('signed_out');
      }
    }
    status('offline');
    setTimeout(connect, pause);
    pause = Math.min(pause * 2, 30000);
  };
}

function send(json) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(json);
  else outbox.push(json);
}

function disconnect() {
  wanted = false;
  const s = ws;
  ws = null;
  outbox = [];
  try { s?.close(); } catch { /* */ }
}

function signedOut(why) {
  disconnect();
  session = null;
  keep('session', null);
  servedHere.then((here) => {
    if (!here) return status(why);
    try { localStorage.removeItem(SHARED); } catch { /* */ }
    signInOnFiles();
  });
}

// ----- The core -----

const ready = (async () => {
  await init();
  chat.start(
    {
      emit: (event, json) => emit(event, JSON.parse(json)),
      save: (what, json) => { try { localStorage.setItem(`xenify-chat-${what}`, json); } catch { /* */ } },
      fetch: (token, path, body) => http(token, path, body),
      send,
      // A meeting's email invitations, sent by the server.
      invite: (json) => {
        const a = JSON.parse(json);
        mailCall('mail_invite', a).catch((e) => emit('to_app', { t: 'error', code: 'mail', message: `The email invitations weren't sent: ${e}` }));
      },
    },
    Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
    localStorage.getItem('xenify-chat-calendar') || '',
    localStorage.getItem('xenify-chat-contacts') || '',
  );
  if (session) {
    server = session.server;
    chat.session(session.server);
  }
})();

// ----- Files in chats -----

const uploads = new Map(); // id → { part }

async function fileBegin({ conv, size }) {
  const v = JSON.parse(await http('', '/v1/chat/files', JSON.stringify({ conv, size })));
  uploads.set(v.id, { part: v.part || 4 << 20 });
  return [v.id, v.part || 4 << 20];
}

async function filePart(bytes, headers) {
  const id = headers.upload;
  const n = Number(headers.part);
  const u = uploads.get(id);
  if (!u) throw "that upload isn't going";
  if (bytes.byteLength > u.part) throw 'that part is too big';
  for (let attempt = 0; ; attempt++) {
    try {
      await http('', `/v1/chat/files/${id}/${n}`, new Uint8Array(bytes), { method: 'PUT' });
      return null;
    } catch (e) {
      if (attempt >= 2 || !String(e).startsWith("Can't reach")) throw e;
      await new Promise((r) => setTimeout(r, 500 * (attempt + 1)));
    }
  }
}

const MIMES = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml', mp4: 'video/mp4', m4v: 'video/mp4', webm: 'video/webm', mov: 'video/quicktime', mp3: 'audio/mpeg', ogg: 'audio/ogg', wav: 'audio/wav', pdf: 'application/pdf', txt: 'text/plain' };

async function fileEnd({ id, name, size, mime }) {
  const u = uploads.get(id);
  if (!u) throw "that upload isn't going";
  uploads.delete(id);
  await http('', `/v1/chat/files/${id}/done`, '');
  const ext = (name.split('.').pop() || '').toLowerCase();
  return { id, name, size, mime: mime || MIMES[ext] || 'application/octet-stream', part: u.part };
}

/** Types a chat file opens as: pictures, sound and video. The type comes
 *  from the message (anyone in the chat could pick it), and SVG, HTML or XML
 *  opened in a tab would run scripts as this site: those are plain bytes. */
const SHOWN = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif', 'image/bmp', 'video/mp4', 'video/webm', 'video/ogg', 'video/quicktime', 'audio/mpeg', 'audio/mp4', 'audio/aac', 'audio/ogg', 'audio/opus', 'audio/wav', 'audio/x-wav', 'audio/webm', 'audio/flac']);
function shownType(mime) {
  const base = String(mime || '').split(';')[0].trim().toLowerCase();
  return SHOWN.has(base) ? base : 'application/octet-stream';
}

/** Whether a message's file details are ones the server could have given
 *  (as the desktop app's `fetchable`): its kind of id, which stays one
 *  piece of the address, a sane part size and file size. */
function fetchable(f) {
  return typeof f?.id === 'string' && /^[A-Za-z0-9_-]+$/.test(f.id)
    && Number.isInteger(f.part) && f.part >= 1 && f.part <= 16 * 1024 * 1024
    && Number.isInteger(f.size) && f.size >= 0 && f.size <= 100 * 1024 ** 3;
}

/** A whole file (pictures, clips and saving). */
const fetched = new Map(); // id → Promise<Blob>
function fileBlob(f) {
  if (!fetchable(f)) return Promise.reject("That file isn't on the server.");
  if (!fetched.has(f.id)) {
    fetched.set(f.id, (async () => {
      const parts = Math.max(1, Math.ceil(f.size / f.part));
      const out = [];
      for (let n = 0; n < parts; n++) {
        out.push(await http('', `/v1/chat/files/${f.id}/${n}`, undefined, { bytes: true }));
      }
      return new Blob(out, { type: shownType(f.mime) });
    })().catch((e) => {
      fetched.delete(f.id);
      throw e;
    }));
  }
  return fetched.get(f.id);
}

window.XENIFY_FILE_BLOB = fileBlob;

/** Pictures and clips: a blob address once the file is here; until then
 *  nothing, and the screens draw again when it's ready. */
const urls = new Map();
window.XENIFY_FILE_URL = (f, redraw) => {
  if (urls.has(f.id)) return urls.get(f.id);
  fileBlob(f).then((b) => {
    urls.set(f.id, URL.createObjectURL(b));
    redraw?.();
  }).catch(() => {});
  return '';
};

function download(blob, name) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name || 'download';
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 2000);
}

// ----- Spelling and grammar (the site's checker, in a worker) -----

let checkerWorker = null;
let asked = 0;
const waiting = new Map();

function checker(name, args = {}) {
  if (!checkerWorker) {
    const src = `import { ready, call } from ${JSON.stringify(new URL('kept.js', CHECKER_AT).href)};
onmessage = async ({ data: { id, name, args } }) => {
  try { await ready; postMessage({ id, value: JSON.parse(call(name, JSON.stringify(args || {}))) }); }
  catch (e) { postMessage({ id, error: String(e.message || e) }); }
};`;
    checkerWorker = new Worker(URL.createObjectURL(new Blob([src], { type: 'text/javascript' })), { type: 'module' });
    checkerWorker.onmessage = ({ data }) => {
      const w = waiting.get(data.id);
      waiting.delete(data.id);
      if (w) data.error ? w.reject(new Error(data.error)) : w.resolve(data.value);
    };
    checker('set_words', { words: kept('words', []) });
  }
  return new Promise((resolve, reject) => {
    const id = ++asked;
    waiting.set(id, { resolve, reject });
    checkerWorker.postMessage({ id, name, args });
  });
}

const KINDS = { Spelling: 'spelling', Grammar: 'grammar', Style: 'style', Readability: 'readability', Register: 'register' };
const enc = new TextEncoder();
/** UTF-16 place of byte offset `b` in `line`. */
function u16(line, b) {
  const bytes = enc.encode(line);
  return new TextDecoder().decode(bytes.slice(0, b)).length;
}

async function check({ text, options: o }) {
  const lines = text.split('\n');
  const request = {
    prefs: { enabled: true, dialect: 'auto', ignore_names: true, default_profile: 'standard', spotter: { enabled: false, path: '', threshold: 0.97 } },
    profile: o.kind === 'email' ? 'workplace-email' : 'workplace-chat',
    paras: lines.map((l, i) => [i, l]),
    ignore_words: [],
    ignore_issues: [],
  };
  const issues = await checker('check', { request });
  const out = [];
  let starts = [];
  let at = 0;
  for (const l of lines) { starts.push(at); at += l.length + 1; }
  for (const i of issues) {
    const line = lines[i.para] ?? '';
    const kind = KINDS[i.category] || 'grammar';
    if (kind === 'spelling' ? !o.spelling : !o.grammar) continue;
    const s = u16(line, i.range.start);
    const e = u16(line, i.range.end);
    const word = line.slice(s, e);
    const key = `${i.rule}:${word}:${line}`;
    if ((o.ignored || []).includes(key)) continue;
    out.push({ start: starts[i.para] + s, end: starts[i.para] + e, kind, message: i.message, suggestions: (i.suggestions || []).slice(0, 5), key, word });
  }
  return out.sort((a, b) => a.start - b.start);
}

// ----- The calls -----

/** A pasted link's `<id>.<secret>` (its server becomes this page's), the
 *  one the page was opened with when nothing's pasted, or just the part
 *  after `#`. */
function takeLink(text) {
  const t = String(text || '').trim();
  if (!t) {
    if (linkCode) return linkCode;
    throw 'Paste the link you were sent.';
  }
  const l = parseLink(t);
  if (l) {
    server = l[0];
    return l[1];
  }
  // A code, or the part of an older link after #.
  if (!/[/\s]/.test(t)) return t;
  throw 'That isn’t the code you were sent.';
}

/** What this browser is called in the server's list of sign-ins. */
const device = () => `Xenify Chat in ${/Firefox/.test(navigator.userAgent) ? 'Firefox' : /Edg\//.test(navigator.userAgent) ? 'Edge' : /Chrome/.test(navigator.userAgent) ? 'Chrome' : /Safari/.test(navigator.userAgent) ? 'Safari' : 'a browser'}`;

/** Signed in (`{ session, remember }`): keeps it and connects; the user. */
function signedIn({ session: s, remember }) {
  session = { server, token: s.token, user: s.user };
  keep('session', session);
  keep('server', server);
  // "Remember this computer": no second step here for 30 days (kept after signing out).
  if (remember) keep('remember', { ...kept('remember', {}), [server]: remember });
  disconnect();
  chat.session(server);
  connect();
  pushAgain();
  return s.user;
}

// ----- Notifications with the tab closed (Web Push) -----

// The service worker (sw.js) shows what the server pushes while no Chat tab
// is open; its Snooze and Dismiss buttons work without the page.

const pushable = () => 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window && window.isSecureContext;

async function worker() {
  return navigator.serviceWorker.register(new URL('sw.js', import.meta.url), { scope: './' });
}

async function pushState() {
  if (!pushable()) return 'unsupported';
  if (Notification.permission === 'denied') return 'blocked';
  const reg = await navigator.serviceWorker.getRegistration(new URL('./', import.meta.url).href);
  const sub = await reg?.pushManager.getSubscription();
  return sub && Notification.permission === 'granted' ? 'on' : 'off';
}

const fromB64 = (t) => Uint8Array.from(atob(t.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (t.length % 4)) % 4)), (c) => c.charCodeAt(0));

async function pushOn() {
  if (!pushable()) return 'unsupported';
  if ((await Notification.requestPermission()) !== 'granted') return Notification.permission === 'denied' ? 'blocked' : 'off';
  const reg = await worker();
  await navigator.serviceWorker.ready;
  let sub = await reg.pushManager.getSubscription();
  if (!sub) {
    const { key } = JSON.parse(await http('', '/v1/push/key'));
    sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: fromB64(key) });
  }
  await http('', '/v1/push/subscribe', JSON.stringify(sub.toJSON()));
  keep('push', true);
  return 'on';
}

/** Stops this browser's notifications (`forget`: and doesn't turn them back
 * on at the next sign-in). */
async function pushOff(forget = true) {
  const reg = await navigator.serviceWorker?.getRegistration(new URL('./', import.meta.url).href);
  const sub = await reg?.pushManager.getSubscription();
  if (sub) {
    await http('', '/v1/push/unsubscribe', JSON.stringify({ endpoint: sub.endpoint })).catch(() => {});
    await sub.unsubscribe();
  }
  if (forget) keep('push', false);
  return 'off';
}

// Turned on before: subscribed again for whoever's signed in now.
async function pushAgain() {
  if (session && kept('push', false) && pushable() && Notification.permission === 'granted') pushOn().catch(() => {});
}

// A notification clicked while a Chat tab is open: its chat comes forward.
navigator.serviceWorker?.addEventListener('message', (e) => {
  if (e.data?.open) emit('open_chat', e.data.open);
});
// …or opened this page (?open=<chat>).
if (Number(params.get('open'))) setTimeout(() => emit('open_chat', Number(params.get('open'))), 3000);

// ----- Email, kept by the server -----

const TZ = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';

async function mailCall(cmd, args = {}) {
  const out = await http('', `/v1/mail/${cmd}`, JSON.stringify({ ...args, tz: TZ }));
  return out ? JSON.parse(out) : null;
}

// What changes (new mail, sign-in trouble, accounts, answers to meetings),
// heard while signed in.
let hearing = null;
async function hearMail() {
  if (hearing || !session) return;
  const mine = (hearing = new AbortController());
  let pause = 2000;
  while (hearing === mine && session) {
    try {
      const r = await fetch(`${server}/v1/mail/events`, { headers: { Authorization: `Bearer ${session.token}` }, signal: mine.signal });
      if (r.ok && r.body) {
        pause = 2000;
        const reader = r.body.pipeThrough(new TextDecoderStream()).getReader();
        let buf = '';
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += value;
          let i;
          while ((i = buf.indexOf('\n\n')) >= 0) {
            const block = buf.slice(0, i);
            buf = buf.slice(i + 2);
            const line = block.split('\n').find((l) => l.startsWith('data: '));
            if (!line) continue;
            let v;
            try { v = JSON.parse(line.slice(6)); } catch { continue; }
            if (v.kind === 'invite_answer') {
              try { chat.guest_answered(v.uid, v.email, v.answer); } catch (e) { console.error(e); }
            } else emit('mail', v);
          }
        }
      }
    } catch { /* offline, or stopped */ }
    if (hearing !== mine) return;
    await new Promise((res) => setTimeout(res, pause));
    pause = Math.min(pause * 2, 60000);
  }
}
function stopHearingMail() {
  hearing?.abort();
  hearing = null;
}

async function mail(cmd, args) {
  hearMail();
  switch (cmd) {
    case 'mail_forget_invite': chat.forget_invite(args.uid); return null;
    case 'mail_save_attachment': {
      const q = new URLSearchParams({ account: args.account, folder: args.folder, uid: args.uid, index: args.index });
      const r = await fetch(`${server}/v1/mail/attachment?${q}`, { headers: { Authorization: `Bearer ${session?.token}` } });
      if (!r.ok) throw 'The attachment couldn’t be fetched from the server.';
      const cd = r.headers.get('content-disposition') || '';
      const m = /filename\*=UTF-8''([^;]+)/.exec(cd);
      const name = m ? decodeURIComponent(m[1]) : 'attachment';
      download(await r.blob(), name);
      return 'your Downloads';
    }
    case 'mail_connect': {
      const { url } = await mailCall(cmd, args);
      // Blocked as a pop-up: this tab goes there (the server's page says when it's done).
      if (!window.open(url, '_blank')) location.assign(url);
      return null;
    }
  }
  const out = await mailCall(cmd, args);
  switch (cmd) {
    case 'mail_accounts': chat.mail_accounts(JSON.stringify((out || []).map((a) => a.email))); break;
    case 'mail_open': if (out?.invite?.uid) out.invite.in_calendar = chat.in_calendar(out.invite.uid); break;
    case 'mail_send': chat.emailed(JSON.stringify(out?.people || [])); return null;
    case 'mail_answer_invite': chat.invite_answered(JSON.stringify(out || {})); return null;
  }
  return out;
}

async function invoke(cmd, args = {}, opts = {}) {
  await ready;
  switch (cmd) {
    case 'start': {
      const home = await atHome;
      if (await servedHere) {
        // Whoever is signed in on this server's pages now (never one kept from before).
        const s = sharedSession();
        if (!s) return signInOnFiles();
        if (s.token !== session?.token) {
          disconnect();
          session = s;
          keep('session', session);
          server = location.origin;
          chat.session(server);
        }
      }
      if (session) connect();
      pushAgain();
      return {
        server: session?.server || server || '',
        user: session?.user || null,
        invite: linkCode,
        sample: false,
        trace: false,
        web: true,
        // Not served by a Xenify server (and no invite link): ask which.
        ask_server: !home && !linkServer,
      };
    }
    case 'link_info': {
      const link = takeLink(args.link);
      return { ...JSON.parse(await chat.link_info(link, (args.username || '').trim())), link };
    }
    case 'sign_in_start': {
      const typed = (args.server || '').trim().replace(/\/+$/, '');
      if (typed) server = /^https?:\/\//.test(typed) ? typed : `https://${typed}`;
      if (!server) throw 'Which server? Type its address.';
      try {
        return JSON.parse(await http('', '/v1/login/start', JSON.stringify({ username: (args.username || '').trim() })));
      } catch (e) {
        // A server from before this step: the password, as always.
        if (/404|405/.test(String(e))) return { next: 'password', can_email: false };
        throw e;
      }
    }
    case 'sign_in': {
      const link = (args.invite || '').trim() ? takeLink(args.invite) : '';
      const typed = (args.server || '').trim().replace(/\/+$/, '');
      if (typed) server = /^https?:\/\//.test(typed) ? typed : `https://${typed}`;
      if (!server) throw 'Which server? Type its address.';
      const out = JSON.parse(await chat.sign_in(server, (args.username || '').trim(), args.password, link, device(), kept('remember', {})[server] || ''));
      // The password was right: the second step next.
      if (out.ticket) return { step: out };
      return signedIn(out);
    }
    case 'second_code':
      return signedIn(JSON.parse(await chat.second_step(cmd, '', JSON.stringify({ ...args, device: device() }))));
    case 'mfa_setup_finish': {
      const out = JSON.parse(await chat.second_step(cmd, args.ticket ? '' : session?.token || '', JSON.stringify(args)));
      return { backup_codes: out.backup_codes, user: out.signed_in ? signedIn(out.signed_in) : null };
    }
    case 'second_email': case 'mfa_setup_app': case 'mfa_setup_email': case 'mfa_state': case 'mfa_off': case 'mfa_backup':
      return JSON.parse(await chat.second_step(cmd, args.ticket ? '' : session?.token || '', JSON.stringify(args)));
    case 'send': chat.send(JSON.stringify(args.msg)); return null;
    case 'sign_out':
      stopHearingMail();
      await pushOff(false).catch(() => {});
      if (await servedHere) {
        // Signed out of every page of this server.
        if (session) await http('', '/v1/signout', '{}').catch(() => {});
        disconnect();
        session = null;
        keep('session', null);
        try { localStorage.removeItem(SHARED); } catch { /* */ }
        return signInOnFiles();
      }
      disconnect();
      session = null;
      keep('session', null);
      return null;
    case 'change_server': {
      const s = String(args.server || '').trim().replace(/\/+$/, '');
      disconnect();
      session = null;
      keep('session', null);
      server = /^https?:\/\//.test(s) ? s : `https://${s}`;
      keep('server', server);
      return server;
    }
    case 'log': console.debug(args.text); return null;
    case 'load_settings': return kept('settings', {});
    case 'save_settings': keep('settings', { settings: args.settings, status: args.status }); return null;
    case 'notify':
      if ('Notification' in window) {
        if (Notification.permission === 'default') Notification.requestPermission();
        if (Notification.permission === 'granted' && document.visibilityState !== 'visible') {
          const n = new Notification(args.title, { body: args.body, silent: !args.sound });
          n.onclick = () => { window.focus(); emit('open_chat', args.conv); n.close(); };
        }
      }
      return null;
    case 'board_session': return { url: session?.server, token: session?.token, account: session?.user };
    case 'board_new': {
      const space = JSON.parse(await http('', '/v1/chat/boards', JSON.stringify({ conv: args.conv }))).space;
      const title = (args.title || '').trim() || 'Whiteboard';
      const meta = JSON.stringify({ name: title, app: 'draw', ext: 'xdraw' });
      const node = JSON.parse(await http('', '/v1/nodes', JSON.stringify({ space, parent: null, kind: 'file', meta }))).id;
      return { node, title };
    }
    case 'board_open_in_draw': throw 'In the browser the board is right here; Open in Draw is for the desktop apps.';
    case 'file_begin': return fileBegin(args);
    case 'file_part': return filePart(args, opts.headers || {});
    case 'file_end': return fileEnd(args);
    case 'file_save': download(await fileBlob(args.info), args.info.name); return '';
    case 'file_send_path': throw 'Drop files on the chat, or use the paper clip.';
    case 'save_download': download(new Blob([args]), decodeURIComponent(opts.headers?.name || 'download')); return '';
    case 'open_link':
      // Like the desktop app: only web and email links (never javascript: and the like).
      if (!/^(https?:\/\/|mailto:)/i.test(String(args.url || ''))) throw 'Only web and email links open from here.';
      window.open(args.url, '_blank', 'noopener');
      return null;
    case 'check': return check(args);
    case 'words': return checker('words');
    case 'add_word':
    case 'remove_word': {
      const words = new Set(await checker('words'));
      if (cmd === 'add_word') words.add(args.word); else words.delete(args.word);
      keep('words', [...words]);
      await checker('set_words', { words: [...words] });
      return [...words];
    }
    case 'spotter_state': return 'unavailable';
    case 'checker_credits': return 'Word meanings: Open English WordNet (CC BY 4.0), based on Princeton WordNet.';
    case 'server_info': return JSON.parse(await http('', '/v1/info'));
    case 'holidays': return JSON.parse(await http('', `/v1/plan/holidays/${Number(args.year)}`));
    case 'secret_new': return JSON.parse(await http('', '/v1/secrets', JSON.stringify({ sealed: args.sealed, days: args.days, check: args.check })));
    case 'forgot': await http('', '/v1/forgot', JSON.stringify({ username: (args.username || '').trim() })); return null;
    case 'push_state': return pushState();
    case 'push_on': return pushOn();
    case 'push_off': return pushOff();
    case 'cal_due': case 'cal_between': case 'cal_snoozed':
    default: {
      if (cmd.startsWith('mail_')) return mail(cmd, args);
      const out = chat.call(cmd, JSON.stringify(args || {}));
      if (out === undefined) throw `not available in the browser: ${cmd}`;
      return JSON.parse(out);
    }
  }
}

window.__TAURI__ = {
  core: {
    invoke,
    // Chat files show through XENIFY_FILE_URL instead.
    convertFileSrc: (path) => path,
  },
  event: {
    async listen(event, cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event).add(cb);
      return () => listeners.get(event).delete(cb);
    },
  },
};
