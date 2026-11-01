// Talking to a Xenify server, for Write and Draw (the same file in both:
// edit crates/xenify-server/client/server.js, then run
// scripts/sync-server-client.sh).
//
// Everything that reads or changes documents stays in the app's engine
// (`call`): this file only moves bytes. The app and the server must work
// with each other's protocol level (see `mismatch`); until they do, nothing
// else works.
//
//   const srv = new Server({ call, live, app: 'write', ext: 'xwrite' });
//   await srv.connect('http://192.168.1.20:7700');
//   await srv.signIn('priya', 'password');
//   const list = await srv.children(space, folder);
//   await srv.open(node);           // opens it in the engine, editing together
//   await srv.saveNew(space, folder, 'Plan');
//   await srv.save();               // the front document, if it's on the server

const KEEP = 'xenify-server';
// The sign-in the pages a server serves share (Files, the admin pages and
// every browser app there): signing in or out on one does it on all.
const SHARED = 'xenify-session';

function sharedSession() {
  try {
    const s = JSON.parse(localStorage.getItem(SHARED) || 'null');
    return s?.token && s.account ? { token: s.token, account: s.account, origin: location.origin } : null;
  } catch { return null; }
}

/// Where a server address points ('' when it isn't one): a sign-in belongs
/// to one origin and its token goes nowhere else.
export function originOf(url) {
  try {
    const u = new URL(url);
    return /^https?:$/.test(u.protocol) ? u.origin : '';
  } catch { return ''; }
}

/// The real host of a server address, as the address bar would show it.
export function hostOf(url) {
  try { return new URL(url).host; } catch { return String(url || '').replace(/^https?:\/\//, ''); }
}
/// The protocol this app speaks (xenify-server-proto's VERSION).
const PROTOCOL = 2;
/// The oldest server protocol this app still works with (xenify-server-proto's OLDEST).
const OLDEST = 2;

/// Where File › Quit goes in the browser: back where the server's Files
/// page opened this app from (it notes that), else to Files.
export function filesBack() {
  try {
    const back = sessionStorage.getItem('xenify-back') || '';
    if (back.startsWith(`${location.origin}/files/`)) return back;
  } catch { /* no storage here */ }
  return '/files/';
}

/// Goes back to the Files page: a step back when Files opened this app in
/// this tab (the page, its place and the history as they were), else to
/// where Files noted, else Files.
export function leaveForFiles() {
  let len = 0;
  try { len = +(sessionStorage.getItem('xenify-back-len') || 0); } catch { /* no storage here */ }
  // Files noted how long the history was as it opened this app here.
  if (len && history.length === len + 1 && filesBack() !== '/files/') {
    history.back();
    // A history that can't go back (opened in a new tab): Files itself.
    setTimeout(() => location.assign(filesBack()), 600);
    return;
  }
  location.assign(filesBack());
}

export class Server {
  /// `call`: the app's engine; `apply`: the same for calls that change
  /// what's shown (the app repaints after them; by default `call`).
  /// `live`: where the engine hands over changes to send. `onChange`:
  /// called when anything shown here may have changed.
  /// `afterSave(id)`: after each version is saved, and now and then while
  /// editing together (the apps put a picture of the document up then).
  /// `media`: where the app keeps the pictures and clips its documents name
  /// by hash (Draw): { has(hash), get(hash) → bytes or null, put(bytes) }.
  constructor({ call, apply = call, live, app, ext, onChange = () => {}, onError = () => {}, afterSave = null, media = null }) {
    this.call = call;
    this.media = media;
    // Node -> the pictures and clips it's known to have.
    this.onServer = new Map();
    this.cachedHere = new Set();
    // Messages heard and batches sent, one after another (pictures load
    // and go up between them).
    this.inbox = Promise.resolve();
    this.outbox = Promise.resolve();
    this.afterSave = afterSave;
    this.apply = apply;
    this.app = app;
    this.ext = ext;
    this.onChange = onChange;
    this.onError = onError;
    this.url = '';
    this.info = null;
    this.session = null; // { token, account, origin }
    // Signing in, password right: { ticket, methods, setup, setup_methods }.
    this.step = null;
    // Server address -> token from "Remember this computer" (no second step).
    this.remembered = {};
    // The servers (origins) this device has signed in to: links to them
    // open without asking first.
    this.known = [];
    this.docs = new Map(); // engine id -> { node, joined, seq }
    this.people = new Map(); // engine id -> [{ conn, name }]
    setInterval(() => { if (this.docs.size) this.keepUnsent(); }, 5000);
    this.said = new Map(); // engine id -> conn -> { who, data } (where their carets are)
    this.ws = null;
    this.wsReady = null;
    this.retry = 1000;
    // The desktop apps follow the Hub's sign-in (Hub › Settings › Xenify
    // server) unless set to use a different server (`own`). `hubId`: the
    // Hub sign-in in use; `hub`: whether the Hub has one now.
    this.own = false;
    this.hubId = null;
    this.hub = false;
    try {
      const kept = JSON.parse(localStorage.getItem(KEEP) || 'null');
      if (kept) {
        this.url = kept.url || '';
        // A sign-in names the server it belongs to (one that doesn't is dropped).
        this.session = kept.session?.origin ? kept.session : null;
        this.own = !!kept.own;
        this.hubId = kept.hubId || null;
        this.remembered = kept.remembered || {};
        this.known = Array.isArray(kept.known) ? kept.known : [];
        for (const o of [this.session?.origin, ...Object.keys(this.remembered).map(originOf)]) if (o && !this.known.includes(o)) this.known.push(o);
      }
    } catch {}
    if (this.servedHere) this.session = sharedSession();
    // Signed in or out on another page of this server.
    window.addEventListener('storage', (e) => {
      if (e.key !== SHARED || !this.servedHere) return;
      const s = sharedSession();
      if (s?.token === this.session?.token) return;
      this.session = s;
      this.step = null;
      this.ws?.close();
      this.ws = null;
      this.onChange();
    });
    this.live = live;
    live.send = (list) => list.forEach((l) => this.sendBatch(l.batch));
    // Which protocol the server kept from last time speaks now.
    if (this.url) this.api('GET', '/v1/info').then((i) => { this.info = i; this.onChange(); }).catch(() => {});
    this.followHub();
    window.addEventListener('focus', () => this.followHub());
  }

  keep() {
    try {
      const here = this.servedHere;
      localStorage.setItem(KEEP, JSON.stringify({ url: this.url, session: here ? null : this.session, own: this.own, hubId: this.hubId, remembered: this.remembered, known: this.known }));
      if (here) {
        if (this.session) localStorage.setItem(SHARED, JSON.stringify(this.session));
        else localStorage.removeItem(SHARED);
      }
    } catch {}
  }

  /// A browser app served by this server: it shares the sign-in of the
  /// server's own pages (signing in happens on Files).
  get servedHere() {
    return (!window.__TAURI__ || !!window.XENIFY_WEB) && /^https?:$/.test(location.protocol) && !!this.url && this.url === location.origin;
  }

  /// Where to sign in when served here: Files, which comes back to this page.
  get signInHere() {
    return `/files/?back=${encodeURIComponent(location.pathname + location.search + location.hash)}`;
  }

  /// The sign-in is for the server in use (its token goes to no other).
  get sessionFits() {
    return !!this.session?.token && !!this.session.origin && this.session.origin === originOf(this.url);
  }

  /// The real host of the server in use (not the name it gives itself).
  get host() {
    return this.url ? hostOf(this.url) : '';
  }

  /// Whether this device has signed in to the server at `url` before
  /// (here, kept, or through the Hub on the desktop).
  async knows(url) {
    const o = originOf(url);
    if (!o) return false;
    if (this.known.includes(o) || (this.sessionFits && this.session.origin === o)) return true;
    if (!window.__TAURI__ || window.XENIFY_WEB) return false;
    try { return originOf((await window.__TAURI__.core.invoke('hub_server'))?.url) === o; } catch { return false; }
  }

  /// Signed in through the Hub (signing out is done there, for every app).
  get viaHub() {
    return !this.own && !!this.hubId && !!this.session;
  }

  /// Takes up the Hub's sign-in, a new one, or none (the Hub signed out:
  /// so does the app). At start and whenever the app comes to the front.
  async followHub() {
    if (!window.__TAURI__ || window.XENIFY_WEB) return;
    let h = null;
    try { h = await window.__TAURI__.core.invoke('hub_server'); } catch { return; }
    this.hub = !!h;
    // A sign-in of the app's own (from before the Hub signed every app
    // in): signed out of, the Hub's sign-in is the one.
    if (this.own || (this.session && !this.hubId)) {
      this.own = false;
      this.forget();
      this.url = '';
      this.info = null;
      this.keep();
    }
    if (!h) {
      if (this.hubId) {
        this.hubId = null;
        this.forget(false);
        this.url = '';
        this.info = null;
        this.keep();
        this.onChange();
      }
      return;
    }
    // Already in use.
    if (h.id === this.hubId && this.session) return;
    // The server turned it down earlier (signed out from the admin pages).
    if (h.id === this.hubGone) return;
    if (h.id !== this.hubId || h.url !== this.url) {
      if (this.session && !this.hubId) this.forget();
      else this.forget(false);
      this.url = h.url;
      this.info = null;
      try { this.info = await this.api('GET', '/v1/info'); } catch {}
    }
    this.hubId = h.id;
    await this.opened(h.session);
  }

  /// Settings › Server: back to the Hub's server, or (`own`) a server of
  /// this app's own, leaving the Hub's signed in for the other apps.
  async useHub(yes) {
    this.own = !yes;
    if (this.own && this.hubId) {
      this.hubId = null;
      this.forget(false);
      this.url = '';
      this.info = null;
    }
    this.keep();
    this.onChange();
    if (yes) await this.followHub();
  }

  /// Signed in to a server this app can use.
  get ready() {
    return this.sessionFits && !this.mismatch;
  }

  /// Why this app can't use the server ('' when it can), as
  /// xenify-server-proto's works_with(): the server serves apps down to its
  /// `oldest_protocol` (servers from before it said served only their own
  /// level), and this app works with servers down to OLDEST.
  get mismatch() {
    const p = this.info?.protocol;
    if (p == null) return '';
    if (PROTOCOL < (this.info.oldest_protocol ?? p)) return 'This server is newer than this app: update Xenify (Xenify Hub can do it).';
    if (p < OLDEST) return 'This server needs updating before this app can use it.';
    return '';
  }

  get name() {
    return this.info?.name || this.url.replace(/^https?:\/\//, '');
  }

  // ---- HTTP -------------------------------------------------------------

  async api(method, path, body, { raw = false } = {}) {
    if (this.mismatch && path !== '/v1/info') throw new Error(this.mismatch);
    const headers = {};
    // Only to the server the sign-in is for.
    if (this.sessionFits) headers.Authorization = `Bearer ${this.session.token}`;
    let payload;
    if (body instanceof Uint8Array) {
      headers['Content-Type'] = 'application/octet-stream';
      payload = body;
    } else if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
      payload = JSON.stringify(body);
    }
    let r;
    try {
      r = await fetch(this.url + path, { method, headers, body: payload });
    } catch {
      // The desktop apps: a server with its own certificate, trusted here
      // (it makes a new one at every start, so the web view is told again).
      const seen = await this.certificate();
      if (seen && (seen.trusted || seen.ignored)) {
        try { r = await fetch(this.url + path, { method, headers, body: payload }); } catch {}
      }
      if (!r) {
        const e = new Error(`Can't reach ${this.name}. Check you're on its network and that it's running.`);
        e.unreachable = true;
        if (seen && !seen.trusted && !seen.ignored) e.certificate = seen;
        throw e;
      }
    }
    if (raw && r.ok) return new Uint8Array(await r.arrayBuffer());
    const text = await r.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch {}
    // A wrong code or password is a 401 too: only a session that ended signs out.
    if (r.status === 401 && this.sessionFits && !path.startsWith('/v1/login') && (!data?.code || data.code === 'signed_out')) {
      if (this.viaHub) this.hubGone = this.hubId;
      this.forget();
      throw new Error('Please sign in to the server again.');
    }
    if (!r.ok) {
      const e = new Error(sentence(data?.error || `The server said no (${r.status}).`));
      e.code = data?.code;
      e.status = r.status;
      e.data = data;
      throw e;
    }
    return data;
  }

  /// The desktop apps, over HTTPS: what certificate the server shows and
  /// whether it's trusted here (null: not a desktop app, or no answer).
  /// With `fingerprint` (what the person was shown): trusts it from now on
  /// (`remember`), or until the app closes.
  async certificate(fingerprint = null, remember = false) {
    if (!window.__TAURI__ || window.XENIFY_WEB || !this.url.startsWith('https:')) return null;
    try {
      return await window.__TAURI__.core.invoke('certificate', { url: this.url, fingerprint, remember });
    } catch (e) {
      if (fingerprint) throw new Error(String(e?.message || e));
      return null;
    }
  }

  // ---- Your words (spelling) ----------------------------------------------

  /// Your words on this server (they follow you to every computer), or null
  /// when not signed in. The first time, the words on this computer
  /// (`local`) start the list.
  async myWords(local = []) {
    if (!this.ready) return null;
    const v = await this.api('GET', '/v1/me/settings/words');
    if (Array.isArray(v?.words)) return v.words;
    if (local.length) await this.setMyWords(local).catch(() => {});
    return local.slice();
  }

  async setMyWords(words) {
    const list = [...new Set(words.map((w) => w.trim()).filter(Boolean))].sort((a, b) => a.localeCompare(b));
    await this.api('PUT', '/v1/me/settings/words', { words: list });
    return list;
  }

  // ---- Connecting and signing in -----------------------------------------

  /// Checks `url` is a Xenify server and remembers it.
  async connect(url) {
    url = url.trim().replace(/\/+$/, '');
    if (!/^https?:\/\//.test(url)) {
      // Just an address ("files.example.com", "192.168.1.20:7443"): try
      // HTTPS first (a domain name's certificate on 443, or the server's own
      // on 7443), then plain HTTP on the network (7700), which a page loaded
      // over HTTPS isn't allowed to use.
      const port = /:\d+$/.test(url);
      const tries = port ? [`https://${url}`, `http://${url}`] : [`https://${url}`, `https://${url}:7443`, `http://${url}:7700`];
      const allowed = tries.filter((u) => !(location.protocol === 'https:' && u.startsWith('http:')));
      let first;
      for (const u of allowed) {
        try { return await this.connect(u); } catch (e) {
          if (e.cancelled) throw e;
          first ??= e;
        }
      }
      throw first;
    }
    const before = this.url;
    // Another server: the sign-in stays behind before anything is asked
    // there (its token is for its own server only), and comes back if
    // this one can't be reached.
    const kept = this.session;
    const keptStep = this.step;
    const other = !!kept && kept.origin !== originOf(url);
    if (other) {
      this.session = null;
      this.step = null;
      this.ws?.close();
      this.ws = null;
    }
    const back = () => {
      this.url = before;
      if (!other) return;
      this.session = kept;
      this.step = keptStep;
      if (this.docs.size) this.rejoin();
    };
    this.url = url;
    try {
      this.info = await this.api('GET', '/v1/info');
    } catch (e) {
      // Its own certificate (desktop apps): ask, then try again.
      if (e.certificate && this.onCertificate) {
        const choice = await this.onCertificate(e.certificate);
        if (choice) {
          try {
            await this.certificate(e.certificate.fingerprint, choice === 'trust');
            back();
            return await this.connect(url);
          } catch (again) { back(); throw again; }
        }
        back();
        const no = new Error('Not connected: the server’s certificate wasn’t trusted.');
        no.cancelled = true;
        throw no;
      }
      back();
      // Over HTTPS, a server without a domain name uses its own certificate,
      // which looks the same as not being there until this computer trusts it.
      if (e.unreachable && url.startsWith('https:')) {
        const host = url.replace(/^https:\/\//, '');
        e.message = `Can't reach ${host}. Check you're on its network and that it's running. If it has no domain name, this computer must trust its certificate first: ask its admin for it (admin pages › Network › Download this server's certificate).`;
      }
      throw e;
    }
    if (before !== url) {
      // Another server's sign-in doesn't carry over (this server's pages
      // may have one of their own).
      this.session = this.servedHere ? sharedSession() : null;
      this.step = null;
      this.ws?.close();
      this.ws = null;
    }
    this.keep();
    this.onChange();
    return this.info;
  }

  /// Xenify servers on this network (desktop apps; the browser can't look).
  async discover() {
    if (!window.__TAURI__ || window.XENIFY_WEB) return [];
    try { return await window.__TAURI__.core.invoke('discover'); } catch { return []; }
  }

  /// Served by a Xenify server (the browser apps): connects to it.
  async connectHere() {
    if (window.__TAURI__ && !window.XENIFY_WEB) return false;
    if (!/^https?:$/.test(location.protocol)) return false;
    try {
      const r = await fetch(`${location.origin}/v1/info`);
      if (!r.ok) return false;
      // A page this server serves: links to other servers don't open here.
      this.hostedHere = /json/.test(r.headers.get('content-type') || '');
      // Kept from another server: that one stays.
      if (this.session && this.url && this.url !== location.origin) return false;
      if (this.url !== location.origin) await this.connect(location.origin);
      // The sign-in of this server's pages, whoever that is now (never one
      // kept from before).
      const s = sharedSession();
      if (s?.token !== this.session?.token) {
        this.session = s;
        this.step = null;
        this.keep();
        this.onChange();
      }
      return true;
    } catch { return false; }
  }

  /// Signs in, or (the password was right) starts the second step: then
  /// `step` is set and the code goes to stepCode (or, with `step.setup`,
  /// setting one up comes first).
  /// Signing in starts with the user name: { next: 'password' |
  /// 'pick_password', can_email }. 'pick_password': an account an admin
  /// added that has no password yet (the code from the admin picks one).
  async startSignIn(username) {
    try {
      const step = await this.api('POST', '/v1/login/start', { username: username.trim().toLowerCase() });
      return step?.next ? step : { next: 'password', can_email: false };
    } catch (e) {
      // A server from before this step.
      if (e.status === 404 || e.status === 405) return { next: 'password', can_email: false };
      throw e;
    }
  }

  async signIn(username, password) {
    username = username.trim().toLowerCase();
    const s = await this.api('POST', '/v1/login', { username, password, device: `Xenify ${cap(this.app)}`, remember: this.remembered[this.url] || '' });
    await this.answered(s);
  }

  async answered(s) {
    if (s.ticket) {
      this.step = s;
      this.onChange();
    } else await this.opened(s);
  }

  /// Calls of the second step; a ticket that ran out starts again.
  async stepApi(path, body) {
    try {
      return await this.api('POST', path, { ticket: this.step.ticket, ...body });
    } catch (e) {
      if (e.code === 'bad_ticket') { this.step = null; this.onChange(); }
      throw e;
    }
  }

  /// The second step's code: `method` 'app', 'email' or 'backup'.
  async stepCode(method, code, remember) {
    const s = await this.stepApi('/v1/login/code', { method, code: code.trim(), remember, device: `Xenify ${cap(this.app)}` });
    await this.opened(s);
  }

  /// "Email me a code" while signing in.
  stepEmail() {
    return this.stepApi('/v1/login/email', {});
  }

  /// Setting up a second step, while signing in (`step.setup`) or signed
  /// in. setupApp: { secret, uri, qr_svg } to scan.
  setupApp() {
    return this.step ? this.stepApi('/v1/mfa/app/start', {}) : this.api('POST', '/v1/mfa/app/start', {});
  }

  /// Codes by email: without `code`, emails one. Both finish with the
  /// backup codes (shown once); while signing in, that signs in too.
  async setupFinish(method, code, remember = false) {
    const path = method === 'app' ? '/v1/mfa/app/finish' : '/v1/mfa/email';
    const out = this.step ? await this.stepApi(path, { code: code.trim(), remember }) : await this.api('POST', path, { code: code.trim() });
    if (out.session) await this.opened(out.session);
    return out.backup_codes;
  }

  setupEmail() {
    return this.step ? this.stepApi('/v1/mfa/email', {}) : this.api('POST', '/v1/mfa/email', {});
  }

  /// Signed in: { app, email, backup_left, required, setup_methods }.
  mfa() {
    return this.api('GET', '/v1/me/mfa');
  }

  mfaOff(password) {
    return this.api('POST', '/v1/me/mfa/off', { password });
  }

  /// New backup codes (the old ones stop working).
  async newBackupCodes(password) {
    return (await this.api('POST', '/v1/me/mfa/backup', { password })).backup_codes;
  }

  /// What `username`'s one-time code (emailed, or from an admin) is for,
  /// before using it: { kind: 'welcome' | 'reset', server_name, username,
  /// display_name, email }. (An older whole link connects to its server.)
  async checkLink(link, username = '') {
    const at = link.trim().match(/^(https?:\/\/[^#]+?)\/join\/?#/);
    if (at && at[1].replace(/\/+$/, '') !== this.url) await this.connect(at[1]);
    return this.api('POST', '/v1/link', { link: linkId(link), username: username.trim().toLowerCase() });
  }

  /// A new account's first sign-in, or a forgotten password: picks the
  /// password with the code, and signs in.
  async claim(link, password, username = '') {
    const s = await this.api('POST', '/v1/link/use', { link: linkId(link), username: username.trim().toLowerCase(), password, device: `Xenify ${cap(this.app)}` });
    await this.answered(s);
  }

  /// The server can send email (codes to pick a password).
  get mails() {
    return !!this.info?.features?.includes('mail');
  }

  /// "Forgot password?", or a new account: the server emails a code to
  /// pick one, if the account has an address. Says the same either way.
  forgot(username) {
    return this.api('POST', '/v1/forgot', { username: username.trim().toLowerCase() });
  }

  async opened(s) {
    const origin = originOf(this.url);
    this.session = { token: s.token, account: s.account, origin };
    this.step = null;
    if (s.remember) this.remembered[this.url] = s.remember;
    if (origin && !this.known.includes(origin)) this.known.push(origin);
    this.keep();
    this.onChange();
  }

  forget(tell = true) {
    if (tell && this.sessionFits) fetch(`${this.url}/v1/signout`, { method: 'POST', headers: { Authorization: `Bearer ${this.session.token}` } }).catch(() => {});
    this.session = null;
    this.step = null;
    this.ws?.close();
    this.ws = null;
    this.keep();
    this.onChange();
  }

  // ---- Files ----------------------------------------------------------

  /// Spaces, named: [{ id, kind, name, role }].
  async spaces() {
    const list = await this.api('GET', '/v1/spaces');
    for (const s of list) {
      // A shared space's name is its details as JSON ({ name, home… }).
      s.name = s.kind === 'personal' ? 'My files' : s.kind === 'templates' ? 'Templates' : spaceName(s.name) || 'Untitled space';
    }
    return list;
  }

  named(list) {
    for (const n of list) n.details = detailsOf(n.meta);
    return list;
  }

  /// Folders and files in a space (or folder), named.
  async children(space, parent = null) {
    return this.named(await this.api('GET', `/v1/nodes?space=${space}${parent ? `&parent=${parent}` : ''}`));
  }

  /// Everything in a space you may see (the templates, say).
  async everything(space) {
    return this.named(await this.api('GET', `/v1/everything?space=${space}`));
  }

  /// Files whose name or words match `q`, best first; the server searches.
  async search(q) {
    return this.named(await this.api('GET', `/v1/search?q=${encodeURIComponent(q)}`));
  }

  async recent() {
    return this.named(await this.api('GET', '/v1/recent'));
  }

  async shared() {
    return this.named(await this.api('GET', '/v1/shared'));
  }

  /// Opens a file from the server in the engine, then edits it together.
  async open(id) {
    const node = (await this.api('GET', `/v1/nodes/${id}`));
    node.details = this.boards?.[id] ? { name: this.boards[id].title } : detailsOf(node.meta);
    let data = '';
    let version = 0;
    let upto = 0;
    if (node.version > 0) {
      const u8 = await this.api('GET', `/v1/nodes/${id}/content`, undefined, { raw: true });
      await this.fetchMedia(id, u8);
      data = b64(u8);
      version = node.version;
      upto = (await this.api('GET', `/v1/nodes/${id}/versions`)).find((v) => v.n === version)?.upto ?? 0;
    }
    // Nothing saved in it yet (made on the Files page): a new, empty one.
    const out = await this.apply('remote_open', { server: this.url, node: id, name: node.details.name || 'Untitled', data, version, upto, user: this.author(), blank: !data });
    await this.join(out.id, id, upto);
    return out;
  }

  /// Puts the front document on the server as a new file.
  async saveNew(space, parent, name) {
    const created = await this.api('POST', '/v1/nodes', { space, parent, kind: 'file', meta: JSON.stringify({ name, app: this.app, ext: this.ext }) });
    const out = await this.apply('remote_open', { server: this.url, node: created.id, name, data: '', user: this.author() });
    await this.save(out.id);
    await this.join(out.id, created.id, 0);
    return out;
  }

  /// Saves a version of a server document (the front one by default).
  /// `auto`: saved as you type, which the server folds into your latest
  /// version while it's recent (see versionLater). `label`: a checkpoint,
  /// a version of its own with that description, kept.
  async save(id, { auto = false, label = '' } = {}) {
    // Changes made here that the server hasn't taken yet: a version now
    // would be its copy without them. They're kept on this computer and go
    // when they can; a version saved by hand waits a little for them.
    const args = id === undefined ? {} : { id };
    const unsent = async () => {
      try { return !!(await this.call('remote_unsent', args))?.unsent; } catch { return false; }
    };
    for (let i = 0; await unsent(); i++) {
      if (auto) return null;
      if (i === 20) throw new Error("Some changes haven't reached the server yet, so no version was saved. They're kept on this computer and are sent when they can.");
      await new Promise((r) => setTimeout(r, 250));
    }
    const s = await this.call('remote_save', args);
    await this.sendMedia(s.node, bytes(s.data));
    const a = (auto ? '&auto=true' : '') + (label ? `&label=${encodeURIComponent(label)}` : '');
    let saved;
    try {
      saved = await this.api('POST', `/v1/nodes/${s.node}/content?base=${s.base}&upto=${s.upto}${a}`, bytes(s.data));
    } catch (e) {
      // Someone saved a version meanwhile: editing together keeps you up
      // to date, so this one simply goes on top.
      if (e.code !== 'behind') throw e;
      saved = await this.api('POST', `/v1/nodes/${s.node}/content?upto=${s.upto}${a}`, bytes(s.data));
    }
    // The words, for the server's search (only once it has the version).
    if (s.text !== undefined) {
      try { await this.api('PUT', `/v1/nodes/${s.node}/text`, new TextEncoder().encode(s.text)); } catch (e) { this.onError(e); }
    }
    const out = await this.apply('remote_saved', { id: s.id, version: saved.version });
    try { await this.afterSave?.(s.id ?? id); } catch (e) { this.onError(e); }
    return out;
  }

  // ---- Pictures and clips (Draw) --------------------------------------
  // A canvas names each picture or clip by its SHA-256 ("sha256:<hex>");
  // the bytes come down and go up beside it, once each.

  known(node) {
    if (!this.onServer.has(node)) this.onServer.set(node, new Set());
    return this.onServer.get(node);
  }

  /// Gives the app every picture and clip `doc` (text or bytes) names that
  /// it hasn't got: from this computer's copies, else from file `node`.
  async fetchMedia(node, doc) {
    if (!this.media) return;
    for (const hash of mediaRefs(doc)) {
      if (this.media.has(hash)) continue;
      let got = await mediaCache.get(hash).catch(() => null);
      if (!got) {
        try { got = await this.api('GET', `/v1/nodes/${node}/media/${hash}`, undefined, { raw: true }); } catch (e) { this.onError(e); continue; }
        this.known(node).add(hash);
        this.cachedHere.add(hash);
        mediaCache.put(hash, got).catch(() => {});
      }
      this.media.put(got);
    }
  }

  /// Sends file `node` the pictures and clips `doc` names that it hasn't
  /// got, in parts, carrying on where an earlier try stopped.
  async sendMedia(node, doc) {
    if (!this.media) return;
    const known = this.known(node);
    for (const hash of mediaRefs(doc)) {
      if (known.has(hash)) continue;
      const got = this.media.get(hash) || (await mediaCache.get(hash).catch(() => null));
      // Never here either (it didn't arrive when the canvas opened): it stays named only.
      if (!got) continue;
      const path = `/v1/nodes/${node}/media/${hash}`;
      let { have = 0, part = 4 << 20, done = false } = await this.api('POST', path, { size: got.length });
      while (!done) {
        try {
          ({ have, done = false } = await this.api('PUT', `${path}?at=${have}&size=${got.length}`, got.subarray(have, have + part)));
        } catch (e) {
          if (e.code !== 'elsewhere') throw e;
          have = e.data.have;
        }
      }
      known.add(hash);
    }
  }

  /// Copies on this computer of the pictures and clips `doc` names, for
  /// changes kept to send later (the app may close first).
  async keepMedia(doc) {
    if (!this.media) return;
    for (const hash of mediaRefs(doc)) {
      if (this.cachedHere.has(hash)) continue;
      const got = this.media.get(hash);
      if (got) await mediaCache.put(hash, got);
      this.cachedHere.add(hash);
    }
  }

  // ---- Changes not sent yet (kept if the app closes offline) ----------

  /// Keeps, on this computer, the changes the server hasn't got
  /// yet; forgets them once they're sent. Runs every few seconds.
  async keepUnsent() {
    for (const [id, d] of this.docs) {
      const key = `${this.app}|${this.url}|${d.node}`;
      let unsent = false;
      try { unsent = (await this.call('remote_unsent', { id })).unsent; } catch { continue; }
      try {
        if (unsent) {
          const s = await this.call('remote_save', { id, mine: true });
          await this.keepMedia(bytes(s.data));
          await kept.put(key, { key, app: this.app, url: this.url, node: d.node, base: s.base, upto: s.upto, data: s.data, at: Date.now() });
        } else if (d.kept) await kept.del(key);
        d.kept = unsent;
      } catch { /* no room to keep it */ }
    }
  }

  /// Changes kept from an earlier run, for this server: [{ node, name, at }].
  async unsent() {
    let list = [];
    try { list = (await kept.all()).filter((k) => k.app === this.app && k.url === this.url && !this.idOf(k.node)[1]); } catch { return []; }
    for (const k of list) {
      try { k.name = detailsOf((await this.api('GET', `/v1/nodes/${k.node}`)).meta).name; } catch { k.name = ''; }
    }
    return list;
  }

  /// Sends kept changes as a new version, if nobody saved one since they
  /// were made: { saved: true }; otherwise { copy } to compare.
  async sendKept(k) {
    try {
      await this.sendMedia(k.node, bytes(k.data));
      await this.api('POST', `/v1/nodes/${k.node}/content?base=${k.base}&upto=${k.upto}`, bytes(k.data));
      await kept.del(k.key);
      return { saved: true };
    } catch (e) {
      if (e.code !== 'behind') throw e;
      await this.fetchMedia(k.node, bytes(k.data));
      return { copy: copyOf(bytes(k.data)) };
    }
  }

  async forgetKept(k) {
    await kept.del(k.key).catch(() => {});
  }

  /// Puts a picture of server document `id` (PNG bytes) on the server, for
  /// showing it inside other documents.
  async savePreview(id, png) {
    const node = this.nodeOf(id) ?? (await this.call('remote_info', id === undefined ? {} : { id }))?.node;
    await this.api('POST', `/v1/nodes/${node}/preview`, png);
  }

  /// The picture of file `node` (PNG, base64), or null if it has none yet.
  async preview(node) {
    const n = await this.api('GET', `/v1/nodes/${node}`);
    let png;
    try { png = await this.api('GET', `/v1/nodes/${node}/preview`, undefined, { raw: true }); }
    catch (e) { if (e.code === 'no_preview') return null; throw e; }
    // `version` says which picture this is: renewed while people edit
    // together, so it can change between saved versions.
    return { png: b64(png), version: n.pictured || n.version, name: detailsOf(n.meta).name };
  }

  /// This server's id (links between documents name it).
  get id() {
    return this.info?.server_id || '';
  }

  async versions(node) {
    return this.api('GET', `/v1/nodes/${node}/versions`);
  }

  /// Names version `n` (a checkpoint, kept) or, with '', unnames it.
  async labelVersion(node, n, label) {
    return this.api('POST', `/v1/nodes/${node}/versions/${n}`, { label: label || null });
  }

  /// Deletes version `n` (never the current one).
  async forgetVersion(node, n) {
    return this.api('DELETE', `/v1/nodes/${node}/versions/${n}`);
  }

  /// Deletes every version but the current one; checkpoints too if asked.
  async forgetVersions(node, { checkpoints = false } = {}) {
    return this.api('DELETE', `/v1/nodes/${node}/versions${checkpoints ? '?checkpoints=true' : ''}`);
  }

  /// Version `n` of a file: { data } (base64, Write, Calc and Plan) and
  /// { text } (Draw).
  async version(id, n) {
    const u8 = await this.api('GET', `/v1/nodes/${id}/content?version=${n}`, undefined, { raw: true });
    await this.fetchMedia(id, u8);
    return copyOf(u8);
  }

  /// This app's templates on the server (the shared Templates space).
  async templates() {
    const space = (await this.api('GET', '/v1/spaces')).find((x) => x.kind === 'templates');
    if (!space) return [];
    return (await this.everything(space.id)).filter((n) => n.kind === 'file' && (!n.details?.app || n.details.app === this.app));
  }

  /// The latest saved version of `id`, to start a new document from: as
  /// version() gives it.
  async copyOf(id) {
    const u8 = await this.api('GET', `/v1/nodes/${id}/content`, undefined, { raw: true });
    await this.fetchMedia(id, u8);
    return copyOf(u8);
  }

  /// The server document open in engine tab `id`: its node, or null.
  nodeOf(id) {
    return this.docs.get(id)?.node ?? null;
  }

  async access(node) {
    return this.api('GET', `/v1/nodes/${node}/access`);
  }

  async setAccess(node, access) {
    return this.api('POST', `/v1/nodes/${node}/access`, access);
  }

  async directory() {
    return this.api('GET', '/v1/directory');
  }

  // ---- Editing together -------------------------------------------------

  socket() {
    if (this.wsReady) return this.wsReady;
    if (!this.sessionFits) return Promise.reject(new Error('not connected'));
    this.wsReady = new Promise((resolve, reject) => {
      const ws = new WebSocket(`${this.url.replace(/^http/, 'ws')}/v1/live`);
      this.ws = ws;
      // Only to the server the sign-in is for.
      ws.onopen = () => (this.sessionFits ? ws.send(JSON.stringify({ t: 'hello', token: this.session.token })) : ws.close());
      ws.onmessage = (e) => {
        const m = JSON.parse(e.data);
        if (m.t === 'welcome') {
          // The server was updated while this app was open: check it again.
          if (this.info && m.protocol != null && m.protocol !== this.info.protocol) {
            this.api('GET', '/v1/info').then((i) => { this.info = i; this.onChange(); if (this.mismatch) ws.close(); }).catch(() => {});
          }
          this.conn = m.conn;
          this.retry = 1000;
          resolve(ws);
        } else if (this.media) this.inbox = this.inbox.then(() => this.heard(m)).catch((err) => this.onError(err));
        else this.heard(m).catch((err) => this.onError(err));
      };
      ws.onclose = () => {
        this.ws = null;
        this.wsReady = null;
        reject(new Error('not connected'));
        if (this.session && this.docs.size) setTimeout(() => this.rejoin(), this.retry);
        this.retry = Math.min(this.retry * 2, 30000);
      };
    });
    return this.wsReady;
  }

  async rejoin() {
    if (!this.session || this.ws) return;
    try {
      await this.socket();
      for (const [id, d] of this.docs) {
        d.joined = false;
        // A document whose first join never got through starts as usual
        // (from its saved version); the others catch up from where they were.
        this.ws.send(JSON.stringify({ t: 'join', node: d.node, since: d.seq ?? d.since }));
        d.rejoining = d.seq == null ? null : id;
      }
    } catch {}
  }

  async join(id, node, since) {
    this.docs.set(id, { node, since, joined: false });
    this.live.active = true;
    try {
      const ws = await this.socket();
      ws.send(JSON.stringify({ t: 'join', node, since }));
    } catch {}
  }

  /// The name comments made here carry (the server checks it's yours).
  author() {
    const a = this.session?.account;
    return a ? a.display_name || a.email || a.username : '';
  }

  idOf(node) {
    for (const [id, d] of this.docs) if (d.node === node) return [id, d];
    return [null, null];
  }

  async heard(m) {
    const [id, d] = this.idOf(m.node);
    // Pictures and clips the changes name, before the changes go in.
    if (d && m.ops?.length) await this.fetchMedia(d.node, m.ops.map((o) => o.data).join('\n'));
    if (m.t === 'joined' && d) {
      this.people.set(id, m.people.filter((p) => p.conn !== this.conn));
      if (d.rejoining) {
        // Back after a lost connection: catch up, then send what was waiting.
        await this.apply('live_ops', { id, ops: m.ops });
        const r = await this.call('live_resend', { id });
        if (r.batch) this.sendBatch(r.batch);
        d.rejoining = null;
      } else if (m.role === 'editor' || m.role === 'manager' || m.role === 'commenter' || m.may?.length) {
        // Comments take ids too.
        this.ws.send(JSON.stringify({ t: 'lease', node: d.node, count: 100000, at_least: 1 }));
        d.joinedMsg = m;
      } else {
        // Reading only: no ids to lease (the server gives editors only), but
        // the changes since the saved version still apply.
        await this.apply('live_start', { id, seq: m.seq, ops: m.ops, conn: this.conn, from: 0, to: 0, role: m.role });
        d.seq = m.seq;
      }
      d.joined = true;
      // A plan: what this person may change there (null: everything their role allows).
      d.may = m.may || null;
      d.role = m.role;
      this.onJoined?.(id, d);
    } else if (m.t === 'leased' && d?.leasing) {
      // More ids, the earlier range nearly used up.
      d.leasing = false;
      await this.call('live_lease', { id, from: m.from, to: m.to });
    } else if (m.t === 'leased' && d?.joinedMsg) {
      const j = d.joinedMsg;
      const early = d.early || [];
      d.joinedMsg = null;
      d.early = null;
      // Started, then the changes that came while waiting for the ids, in
      // order: both asked for before anything else that arrives meanwhile.
      const started = this.apply('live_start', { id, seq: j.seq, ops: j.ops, conn: this.conn, from: m.from, to: m.to, role: j.role });
      const caught = early.map((ops) => this.apply('live_ops', { id, ops }));
      await started;
      await Promise.all(caught);
      d.seq = Math.max(j.seq, ...early.flat().map((o) => o.seq));
    } else if (m.t === 'ops' && d?.joinedMsg) {
      // Others' changes between joining and getting ids (not editing
      // together yet): they go in once it starts, above.
      (d.early ||= []).push(m.ops);
      return;
    } else if (m.t === 'ops' && d) {
      await this.apply('live_ops', { id, ops: m.ops });
      d.seq = Math.max(d.seq || 0, ...m.ops.map((o) => o.seq));
    } else if (m.t === 'accepted' && d) {
      // The next batch comes back here (Draw) or through `live` (Write).
      const out = await this.apply('live_accepted', { id, client: m.client, from: m.from, to: m.to });
      if (out?.batch) this.sendBatch(out.batch);
      d.seq = Math.max(d.seq || 0, m.to);
      this.moreIds(id, d);
      this.versionLater(id);
    } else if (m.t === 'behind' && d) {
      const out = await this.apply('live_behind', { id, client: m.client, ops: m.ops });
      if (out?.batch) this.sendBatch(out.batch);
    } else if (m.t === 'said' && id !== null) {
      // Where someone's caret is: kept until they say again or leave.
      if (!this.said.has(id)) this.said.set(id, new Map());
      this.said.get(id).set(m.from.conn, { who: m.from, data: m.data });
      this.onSaid?.(id);
      return;
    } else if (m.t === 'arrived' && id !== null) {
      this.people.set(id, [...(this.people.get(id) || []), m.who]);
      this.onSaid?.(id, 'arrived');
    } else if (m.t === 'left' && id !== null) {
      this.people.set(id, (this.people.get(id) || []).filter((p) => p.conn !== m.conn));
      this.said.get(id)?.delete(m.conn);
      this.onSaid?.(id);
    } else if (m.t === 'error' && m.code === 'restarting') {
      this.ws?.close();
    } else if (m.t === 'error') {
      this.onError(new Error(sentence(m.error)));
      // A change the server won't take (your access): the app can put things back.
      if (m.code === 'not_allowed') this.onRefused?.(m);
    }
    this.onChange();
  }

  /// Asks for a new id range before this window's runs out (new paragraphs
  /// and layers take their ids from it).
  async moreIds(id, d) {
    if (d.leasing || this.ws?.readyState !== 1) return;
    d.leasing = true;
    const { left, next } = await this.call('live_ids', { id }).catch(() => ({ left: Infinity }));
    if (left > 2000 || this.ws?.readyState !== 1) { d.leasing = false; return; }
    // Above every id used here.
    this.ws.send(JSON.stringify({ t: 'lease', node: d.node, count: 100000, at_least: next }));
  }

  sendBatch(b) {
    if (!this.media) return this.sendNow(b);
    // The pictures and clips it names go up first. One that can't still
    // lets the change go (the others see it once it can).
    this.outbox = this.outbox
      .then(() => this.sendMedia(b.node, b.ops.join('\n')))
      .catch((e) => this.onError(e))
      .then(() => this.sendNow(b));
  }

  sendNow(b) {
    if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'send', ...b }));
    // Otherwise it goes again after reconnecting (live_resend).
    for (const [id, d] of this.docs) if (d.node === b.node) this.pictureLater(id);
  }

  /// While someone edits a document together with others, its picture
  /// (what documents linking to it show) is renewed without saving a
  /// version: 4 s after the edits pause, and at least every 20 s while
  /// they go on.
  pictureLater(id) {
    const d = this.docs.get(id);
    if (!d || !this.afterSave) return;
    clearTimeout(d.pictureTimer);
    const now = Date.now();
    d.pictureDue ??= now + 20000;
    d.pictureTimer = setTimeout(async () => {
      d.pictureDue = null;
      try { await this.afterSave(id); } catch (e) { this.onError(e); }
    }, Math.max(0, Math.min(4000, d.pictureDue - now)));
  }

  /// What you type reaches the saved document (what readers, search and
  /// the history see) without Save: 3 s after your changes pause, and at
  /// least every 30 s while they go on. Editors only.
  versionLater(id) {
    const d = this.docs.get(id);
    if (!d || !['editor', 'manager'].includes(d.role)) return;
    clearTimeout(d.versionTimer);
    const now = Date.now();
    d.versionDue ??= now + 30000;
    d.versionTimer = setTimeout(async () => {
      d.versionDue = null;
      if (!this.docs.has(id)) return;
      try { await this.save(id, { auto: true }); this.onChange(); } catch (e) { this.onError(e); }
    }, Math.max(0, Math.min(3000, d.versionDue - now)));
  }

  /// Leaves for the Files page (File › Quit in the browser), what was
  /// typed saved first.
  async quitToFiles() {
    await this.versionNow();
    leaveForFiles();
  }

  /// Saves at once what versionLater is waiting to (before leaving the page,
  /// or closing document `only`).
  async versionNow(only) {
    for (const [id, d] of this.docs) {
      if (only !== undefined && id !== only) continue;
      if (!d.versionTimer || d.versionDue === null) continue;
      clearTimeout(d.versionTimer);
      d.versionDue = null;
      try { await this.save(id, { auto: true }); } catch (e) { this.onError(e); }
    }
  }

  /// Stops editing a document together (its tab closed).
  leave(id) {
    const d = this.docs.get(id);
    if (!d) return;
    this.docs.delete(id);
    this.people.delete(id);
    this.said.delete(id);
    if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'leave', node: d.node }));
  }

  /// Tells the others editing document `id` where this window's caret is
  /// (`data`: from the engine's live_say).
  say(id, data) {
    const d = this.docs.get(id);
    if (d?.joined && this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'say', node: d.node, data }));
  }

  /// Where the others' carets are in document `id`: [{ who, data }].
  saidOf(id) {
    return [...(this.said.get(id)?.values() || [])];
  }

  /// Who else has document `id` open.
  here(id) {
    return this.people.get(id) || [];
  }
}

/// Changes not sent yet, kept in this app's own storage (IndexedDB).
const kept = {
  db: null,
  open() {
    return (this.db ||= new Promise((resolve, reject) => {
      const r = indexedDB.open('xenify-unsent', 1);
      r.onupgradeneeded = () => r.result.createObjectStore('docs');
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    }));
  },
  async tx(mode, fn) {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const t = db.transaction('docs', mode);
      const req = fn(t.objectStore('docs'));
      t.oncomplete = () => resolve(req.result);
      t.onerror = () => reject(t.error);
      // No room left aborts without an error event on the transaction.
      t.onabort = () => reject(t.error);
    });
  },
  put(k, v) { return this.tx('readwrite', (s) => s.put(v, k)); },
  del(k) { return this.tx('readwrite', (s) => s.delete(k)); },
  all() { return this.tx('readonly', (s) => s.getAll()); },
};

/// Pictures and clips by hash, kept on this computer (IndexedDB): the ones
/// changes kept to send later name, and ones fetched (not fetched again).
const mediaCache = {
  db: null,
  open() {
    return (this.db ||= new Promise((resolve, reject) => {
      const r = indexedDB.open('xenify-media', 1);
      r.onupgradeneeded = () => r.result.createObjectStore('files');
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    }));
  },
  async tx(mode, fn) {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const t = db.transaction('files', mode);
      const req = fn(t.objectStore('files'));
      t.oncomplete = () => resolve(req.result);
      t.onerror = () => reject(t.error);
      t.onabort = () => reject(t.error);
    });
  },
  async get(hash) {
    const got = await this.tx('readonly', (s) => s.get(hash));
    return got ? new Uint8Array(got) : null;
  },
  put(hash, bytes) { return this.tx('readwrite', (s) => s.put(bytes, hash)); },
};

/// The hashes of the pictures and clips a document (text or bytes) names.
function mediaRefs(doc) {
  const text = typeof doc === 'string' ? doc : new TextDecoder().decode(doc);
  return [...new Set([...text.matchAll(/sha256:([0-9a-f]{64})/g)].map((m) => m[1]))];
}

const cap = (s) => s[0].toUpperCase() + s.slice(1);
/// A folder's or file's details (its `meta`, JSON): { name, app, ext, … }.
function spaceName(text) {
  try { const d = JSON.parse(text); return typeof d === 'object' && d ? d.name : String(d ?? ''); } catch { return text; }
}

function detailsOf(meta) {
  try { return JSON.parse(meta) || { name: '' }; } catch { return { name: '' }; }
}

/// The part of a one-time link (`<server>/join/#<id>.<secret>`) the server
/// needs; a bare `<id>.<secret>` works too.
const linkId = (link) => link.trim().replace(/^[^#]*#/, '');

/// A saved file's bytes, as the apps open a copy: { data } (base64) and
/// { text }.
function copyOf(u8) {
  return { data: b64(u8), get text() { return new TextDecoder().decode(u8); } };
}

const sentence = (s) => (s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? '' : '.') : s);

function b64(u8) {
  let s = '';
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000));
  return btoa(s);
}

function bytes(b) {
  const s = atob(b);
  const u8 = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u8[i] = s.charCodeAt(i);
  return u8;
}
