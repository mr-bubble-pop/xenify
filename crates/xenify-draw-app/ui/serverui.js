// The screens for a Xenify server, shared by Write and Draw (edit
// crates/xenify-server/client, then run scripts/sync-server-client.sh):
// Settings › Server (connect; sign in: the user name, then the password or
// a code from an admin; sign out), File › Open from server and File › Save
// to server. The talking is in server.js; `env` brings the app's helpers:
// h, toast, modalShell (Write's kind), showSettings, refresh.

import { originOf, hostOf } from './server.js';

export const ui = { server: null, env: null };

// The DOM's append and replaceChildren write "null" for empty slots: leave them out.
const kids = (...a) => a.flat().filter((x) => x != null && x !== false);

// Lucide's "server" icon (ISC).
const SERVER_ICON = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="8" x="2" y="2" rx="2"/><rect width="20" height="8" x="2" y="14" rx="2"/><path d="M6 6h.01"/><path d="M6 18h.01"/></svg>';
const fileName = (n) => n.details?.name || 'Untitled';
// A link from an admin pasted as the server's address: { link, info }.
let linkAt = null;
// What to say after a redraw (the second step ran out of time).
let carried = '';

/// A server with its own certificate (no domain name), first time here:
/// 'trust' (remembered), 'once' (until the app closes) or null.
function askCertificate(seen) {
  const { h, modalShell } = ui.env;
  const host = seen.host.replace(/:443$/, '');
  return new Promise((resolve) => {
    const shell = modalShell('This server uses its own certificate', kids(
      seen.changed ? h('p', { role: 'alert' }, `${host} showed a different certificate before. If its admin didn’t make a new one, something else may be answering in its place: don’t connect until you’ve checked.`) : null,
      h('p', {}, `${host} has no domain name, so its HTTPS certificate is signed by the server itself${seen.name ? ` (“${seen.name}”)` : ''}. To be sure it’s your server, check with its admin that this fingerprint matches the one on its admin pages (Admin › Network):`),
      h('p', { class: 'srv-fingerprint' }, seen.fingerprint),
      h('p', { class: 'set-desc' }, 'Trust this server: this computer remembers it, and asks again only if the certificate changes. Ignore: connect now and ask again next time.')),
    [['Cancel', () => { resolve(null); return true; }],
      ['Ignore this time', () => { resolve('once'); return true; }],
      ['Trust this server', () => { resolve('trust'); return true; }, true]]);
    shell.body.classList.add('srv-modal');
  });
}

/// The server's name for signing in, with its real host when the name it
/// gives itself is something else.
function signInName(srv) {
  const host = srv.host;
  return host && srv.name !== host ? `${srv.name} (${host})` : srv.name;
}

/// A link to a server this device hasn't signed in to: true to go there.
function askToLeave(url) {
  const { h, modalShell } = ui.env;
  const host = hostOf(url);
  const now = ui.server.host;
  return new Promise((resolve) => {
    let done = false;
    const end = (yes) => { if (!done) { done = true; resolve(yes); } };
    const shell = modalShell('Another server', [
      h('p', {}, 'Open a document on ', h('b', {}, host), `?${now && now !== host ? ` You’ll leave ${now}.` : ''}`)],
    [['Cancel', () => { end(false); return true; }],
      ['Open', () => { end(true); return true; }, true]]);
    shell.body.classList.add('srv-modal');
    // Closed with Escape: nothing opens.
    const watch = setInterval(() => { if (!shell.body.isConnected) { clearInterval(watch); end(false); } }, 300);
  });
}

/// Settings › Server: one row that changes with where you are.
/// "Priya Shah (priya@example.org)", or whichever of the two there is.
const whoIs = (a) => (a.display_name && a.email ? `${a.display_name} (${a.email})` : a.display_name || a.email || a.username);

/// The desktop app (not the browser build).
export const onDesktop = () => !!window.__TAURI__ && !window.XENIFY_WEB;

/// "Connect in Xenify Hub": what a desktop app shows instead of signing in.
/// While it shows, the app looks for the Hub's sign-in every few seconds
/// (and when it comes to the front), then `redraw`s.
export function hubAsk(redraw, text = 'Xenify Hub connects your apps to a server: sign in there once and every app follows.') {
  const { h } = ui.env;
  const srv = ui.server;
  const note = h('p', { class: 'set-desc', role: 'status' });
  const box = h('div', { class: 'srv-hub' },
    h('p', { class: 'set-desc' }, text),
    h('div', { class: 'words-add' }, h('button', {
      class: 'btn small main',
      onClick: async () => {
        note.textContent = '';
        try { await window.__TAURI__.core.invoke('open_hub'); } catch (e) { note.textContent = String(e?.message || e); }
      },
    }, 'Open Xenify Hub')),
    note);
  const tick = setInterval(async () => {
    if (!box.isConnected) return clearInterval(tick);
    await srv.followHub();
    if (srv.viaHub) { clearInterval(tick); redraw?.(); }
  }, 3000);
  return box;
}

export function serverRow(redraw) {
  const { h, toast } = ui.env;
  const srv = ui.server;
  srv.onCertificate ??= askCertificate;
  const box = h('div', { class: 'srv-row' });
  const msg = h('p', { class: 'set-desc', role: 'status' });
  const fail = (e) => { msg.textContent = String(e.message || e); };
  if (carried) { msg.textContent = carried; carried = ''; }
  // The server and this app speak different protocols: nothing works until one is updated.
  const wrong = srv.info && srv.mismatch ? h('p', { class: 'set-desc', role: 'alert' }, srv.mismatch) : null;
  // Set to a server of its own while the Hub is signed in to one.
  const backToHub = srv.own && srv.hub
    ? h('button', { class: 'link', onClick: async () => { await srv.useHub(true); redraw(); } }, 'Use the Hub’s server')
    : null;
  if (srv.viaHub) {
    const me = srv.session.account;
    box.append(...kids(
      h('p', { class: 'set-desc' }, `Signed in to ${srv.name} as ${whoIs(me)} through Xenify Hub. To sign out or change server, use the Hub: every app follows.`), wrong, msg));
    return box;
  }
  // The desktop apps never ask for a server or a password: Xenify Hub
  // signs every app in (and sets up a server on this computer).
  if (onDesktop()) {
    box.append(...kids(hubAsk(redraw), msg));
    return box;
  }
  if (!srv.info && !srv.session) {
    const addr = h('input', { class: 'field', placeholder: 'e.g. 192.168.1.20 or docs.example.org', 'aria-label': 'Server address', value: srv.url || '' });
    const go = async (url = addr.value) => {
      try {
        // A link from an admin: its server, then picking a password.
        if (url.includes('#')) linkAt = { link: url.trim(), info: await srv.checkLink(url) };
        else await srv.connect(url);
        redraw();
      } catch (e) { fail(e); }
    };
    addr.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    // Servers on this network, one click each.
    const found = h('div', { class: 'srv-found' }, h('p', { class: 'set-desc' }, 'Looking for servers on this network…'));
    srv.discover().then((list) => {
      found.replaceChildren(...(list.length
        ? list.map((f) => h('div', { class: 'srv-found-row' },
          h('span', { class: 'srv-ico server', 'aria-hidden': 'true', html: SERVER_ICON }),
          h('span', { class: 'srv-name' }, h('b', {}, f.name), h('br'), h('span', { class: 'set-desc' }, f.url.replace(/^https?:\/\//, ''))),
          h('button', { class: 'btn small main', onClick: () => go(f.url) }, 'Connect')))
        : [h('p', { class: 'set-desc' }, window.__TAURI__ && !window.XENIFY_WEB ? 'None found on this network. Type its address below.' : 'Type the server’s address below.')]));
    });
    box.append(...kids(h('b', {}, 'Found on this network'), found, h('b', {}, 'Somewhere else'),
      h('div', { class: 'words-add' }, addr, h('button', { class: 'btn small', onClick: () => go() }, 'Connect')), msg,
      h('p', { class: 'set-desc' }, 'Got a link from an admin? Paste it as the address.'), backToHub));
    return box;
  }
  if (!srv.session && wrong) {
    box.append(...kids(wrong, h('button', { class: 'link', onClick: () => { srv.info = null; srv.url = ''; srv.keep(); redraw(); } }, 'Use a different server'), backToHub));
    return box;
  }
  // Served by this server: its Files page signs in for every app here.
  if (!srv.session && srv.servedHere && !wrong) {
    box.append(...kids(h('p', { class: 'set-desc' }, `Sign in to ${srv.name} to open and save files there.`),
      h('div', { class: 'words-add' }, h('button', { class: 'btn small main', onClick: () => location.assign(srv.signInHere) }, 'Sign in')), msg));
    return box;
  }
  if (!srv.session && srv.step) {
    box.append(...kids(h('p', { class: 'set-desc' }, `Sign in to ${signInName(srv)}.`),
      // Two-step sign-in is set up on the server's own pages, never in an app.
      srv.step.setup ? [h('b', {}, 'Two-step sign-in'),
        h('p', { class: 'set-desc' }, `${srv.name} asks for a code as well as your password, and you haven’t set that up yet. Sign in once on its web page to set it up, then sign in here again: `,
          h('a', { href: `${srv.url}/files/`, target: '_blank', rel: 'noopener' }, `${srv.url.replace(/^https?:\/\//, '')}/files/`))]
        : secondStep(msg, redraw),
      msg, h('button', { class: 'link', onClick: () => { srv.step = null; redraw(); } }, 'Start again')));
    return box;
  }
  if (!srv.session) {
    // The user name first; then the password, or (an account with no
    // password yet, or a forgotten one) a code, emailed or from an admin,
    // and a new password. `got`: what the code is for, once checked.
    let stage = linkAt ? 'pick' : 'user';
    let got = linkAt?.info || null;
    let note = '';
    let emailed = false;
    const user = h('input', { class: 'field', inputmode: 'email', 'aria-label': 'Email', autocomplete: 'username', spellcheck: 'false' });
    const pass = h('input', { class: 'field', type: 'password', 'aria-label': 'Password', autocomplete: 'current-password' });
    const code = h('input', { class: 'field', 'aria-label': 'Code', autocomplete: 'one-time-code', spellcheck: 'false', value: linkAt?.link || '' });
    const pass2 = h('input', { class: 'field', type: 'password', 'aria-label': 'Password again', autocomplete: 'new-password' });
    linkAt = null;
    const form = h('div', { class: 'srv-signin' });
    const to = (next, focus) => { stage = next; msg.textContent = ''; draw(); focus?.focus(); };
    const who = () => h('p', { class: 'set-desc' }, user.value.trim(), ' · ',
      h('button', { class: 'link', onClick: () => { got = null; pass.value = ''; code.value = ''; to('user', user); } }, 'Change'));
    const draw = () => {
      if (stage === 'pick' && got) pass.setAttribute('autocomplete', 'new-password');
      form.replaceChildren(...kids(
        stage === 'user' ? [h('label', { class: 'srv-lbl' }, 'Email', user),
          h('div', { class: 'words-add' }, h('button', { class: 'btn small main', onClick: next }, 'Next'))] : null,
        stage === 'password' ? [who(), h('label', { class: 'srv-lbl' }, 'Password', pass),
          h('div', { class: 'words-add' }, h('button', { class: 'btn small main', onClick: signIn }, 'Sign in'),
            h('button', { class: 'link', onClick: forgot }, 'Forgot password?'))] : null,
        stage === 'code' ? [who(), h('p', { class: 'set-desc' }, note),
          h('label', { class: 'srv-lbl' }, 'Code', code),
          h('div', { class: 'words-add' }, h('button', { class: 'btn small main', onClick: checkCode }, 'Next'),
            emailed ? h('button', { class: 'link', onClick: () => emailMe(true) }, 'Send a new code') : null)] : null,
        stage === 'pick' && got ? [h('p', { class: 'set-desc' }, `${got.kind === 'reset' ? 'A new password' : 'Welcome'} for ${got.display_name || got.email || user.value.trim()} on ${got.server_name}.`),
          h('label', { class: 'srv-lbl' }, 'Pick a password', pass), h('label', { class: 'srv-lbl' }, 'Password again', pass2),
          h('div', { class: 'words-add' }, h('button', { class: 'btn small main', onClick: pick }, 'Set password and sign in'))] : null,
        msg));
    };
    const next = async () => {
      if (!user.value.trim()) return void user.focus();
      try {
        const step = await srv.startSignIn(user.value);
        if (step.next !== 'pick_password') return void to('password', pass);
        if (step.can_email) await emailMe(false);
        else { note = 'This account doesn’t have a password yet. Type the code your admin gave you, then pick one.'; to('code', code); }
      } catch (e) { fail(e); }
    };
    const forgot = async () => {
      if (!srv.mails) return void (msg.textContent = 'Ask an admin to reset your password: you then pick a new one when you sign in.');
      await emailMe(false, 'If this account has an email address, a code is on its way to it. Type it here, then pick a new password.');
    };
    /// Has the server email a code, and asks for it.
    const emailMe = async (again, text = 'We’ve emailed you a code. Type it here, then pick your password.') => {
      try {
        await srv.forgot(user.value);
        note = again ? 'A new code is on its way to your email.' : text;
        emailed = true;
        to('code', code);
      } catch (e) { fail(e); }
    };
    const checkCode = async () => {
      if (!code.value.trim()) return void code.focus();
      try {
        got = await srv.checkLink(code.value, user.value);
        to('pick', pass);
      } catch (e) { fail(e); }
    };
    const signIn = async () => {
      try {
        msg.textContent = 'Signing in…';
        await srv.signIn(user.value, pass.value);
        toast(`Signed in to ${srv.name}`);
        redraw();
      } catch (e) { fail(e); }
    };
    const pick = async () => {
      try {
        if (pass.value.length < 8) throw new Error('Use a password of at least 8 characters.');
        if (pass.value !== pass2.value) throw new Error('The two passwords aren’t the same.');
        msg.textContent = 'Signing in…';
        await srv.claim(code.value, pass.value, user.value);
        toast(`Signed in to ${srv.name}`);
        redraw();
      } catch (e) { fail(e); }
    };
    user.addEventListener('keydown', (e) => { if (e.key === 'Enter') next(); });
    pass.addEventListener('keydown', (e) => { if (e.key === 'Enter' && stage === 'password') signIn(); });
    pass2.addEventListener('keydown', (e) => { if (e.key === 'Enter') pick(); });
    code.addEventListener('keydown', (e) => { if (e.key === 'Enter') checkCode(); });
    draw();
    box.append(...kids(h('p', { class: 'set-desc' }, `Sign in to ${signInName(srv)}.`), form,
      h('button', { class: 'link', onClick: () => { srv.info = null; srv.url = ''; srv.keep(); redraw(); } }, 'Use a different server'), backToHub));
    return box;
  }
  const me = srv.session.account;
  box.append(...kids(
    h('p', { class: 'set-desc' }, `Signed in to ${srv.name} as ${whoIs(me)}.`), wrong,
    h('div', { class: 'words-add' },
      h('button', { class: 'btn small', onClick: () => { srv.forget(); redraw(); } }, 'Sign out'),
      h('button', { class: 'btn small', onClick: () => { srv.forget(); srv.info = null; srv.url = ''; srv.keep(); redraw(); } }, 'Use a different server')),
    msg, backToHub));
  return box;
}

// ---- Two-step sign-in ------------------------------------------------------

const CODE_LABEL = { app: 'Code from your authenticator app', email: 'Code from the email', backup: 'Backup code' };

// A field for a code (6 digits, or a backup code).
const codeField = () => ui.env.h('input', { class: 'field', autocomplete: 'one-time-code', spellcheck: 'false' });

/// An error from the second step: when it ran out of time, back to signing in.
function stepFailed(e, msg, redraw) {
  if (ui.server.session || ui.server.step) msg.textContent = String(e.message || e);
  else { carried = String(e.message || e); redraw(); }
}

/// Signing in, password right: the code.
function secondStep(msg, redraw) {
  const { h, toast } = ui.env;
  const srv = ui.server;
  const step = srv.step;
  const has = (m) => step.methods.includes(m);
  let method = has('app') ? 'app' : has('email') ? 'email' : 'backup';
  const code = codeField();
  const remember = h('input', { type: 'checkbox' });
  const form = h('div', { class: 'srv-signin' });
  const use = (m) => { method = m; code.value = ''; msg.textContent = ''; draw(); code.focus(); };
  const email = async () => {
    try {
      msg.textContent = 'Sending…';
      await srv.stepEmail();
      use('email');
      msg.textContent = 'A code is on its way to your email.';
    } catch (e) { stepFailed(e, msg, redraw); }
  };
  const go = async () => {
    if (!code.value.trim()) return void code.focus();
    try {
      msg.textContent = 'Signing in…';
      await srv.stepCode(method, code.value, remember.checked);
      toast(`Signed in to ${srv.name}`);
      redraw();
    } catch (e) { stepFailed(e, msg, redraw); }
  };
  code.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
  const draw = () => {
    form.replaceChildren(...kids(
      h('b', {}, 'Two-step sign-in'),
      h('label', { class: 'srv-lbl' }, CODE_LABEL[method], code),
      h('label', { class: 'srv-check' }, remember, 'Remember this computer for 30 days'),
      h('div', { class: 'words-add' },
        h('button', { class: 'btn small main', onClick: go }, 'Sign in'),
        has('app') && method !== 'app' ? h('button', { class: 'link', onClick: () => use('app') }, 'Use your authenticator app') : null,
        has('email') ? h('button', { class: 'link', onClick: email }, method === 'email' ? 'Email me another code' : 'Email me a code') : null,
        has('backup') && method !== 'backup' ? h('button', { class: 'link', onClick: () => use('backup') }, 'Use a backup code') : null)));
  };
  draw();
  // Codes by email only: the first one goes straight away.
  if (method === 'email' && !step.sent) { step.sent = true; email(); }
  return form;
}

/// Signed in to a server this app can use; otherwise Settings › Server
/// shows what's needed.
function signedIn() {
  const srv = ui.server;
  if (srv.ready) return true;
  if (srv.session) ui.env.toast(srv.mismatch);
  else if (srv.servedHere) location.assign(srv.signInHere);
  else ui.env.showSettings('server');
  return false;
}

/// A browser of the server's spaces, folders and files. `pick(file)` for
/// Open; with `folders` it picks a place to save.
/// Where a file made from Files' New goes (/app/?new=1&space=…&parent=…):
/// { space, parent }, or null.
export function newPlace() {
  const q = new URLSearchParams(location.search);
  if (!q.has('new') || !/^\d+$/.test(q.get('space') || '')) return null;
  return { space: +q.get('space'), parent: /^\d+$/.test(q.get('parent') || '') ? +q.get('parent') : null };
}

function browser({ forSave = false, onPick, onChoose, app = null, start = null }) {
  const { h } = ui.env;
  const srv = ui.server;
  const root = h('div', { class: 'srv-browser' });
  const places = h('nav', { class: 'srv-places', 'aria-label': 'Places' });
  const list = h('div', { class: 'srv-list', role: 'listbox', 'aria-label': 'Files' });
  const crumbs = h('div', { class: 'srv-crumbs' });
  const find = h('input', { class: 'field', placeholder: `Search ${srv.name}`, 'aria-label': `Search ${srv.name}` });
  const at = { space: null, path: [], items: [], place: forSave ? null : 'recent', sel: null };
  let spaces = [];
  // `app`: the app (or apps) whose files can be chosen; others aren't shown.
  const allowed = [app || srv.app].flat();
  const ours = (n) => n.kind === 'folder' || !n.details?.app || allowed.includes(n.details.app);
  const showItems = (all) => {
    const items = forSave ? all : all.filter(ours);
    list.replaceChildren(...(items.length ? items.map((n) => {
      const folder = n.kind === 'folder';
      const row = h('button', { class: `srv-item${at.sel === n.id ? ' on' : ''}`, role: 'option', 'aria-selected': at.sel === n.id, disabled: !ours(n) || (forSave && !folder) },
        h('span', { class: `srv-ico ${folder ? 'folder' : n.details?.app || 'doc'}`, 'aria-hidden': 'true' }, folder ? '' : ({ draw: 'DRW', calc: 'CLC', plan: 'PLN' }[n.details?.app] || 'DOC')),
        h('span', { class: 'srv-name' }, fileName(n)),
        h('span', { class: 'srv-when' }, folder ? '' : new Date(n.changed).toLocaleDateString()));
      row.addEventListener('click', () => {
        if (folder) { at.path.push(n); at.sel = null; load(); return; }
        // Marked where it is (drawing the list again would lose a double-click).
        at.sel = n.id; onPick?.(n);
        for (const r of list.querySelectorAll('.srv-item')) { r.classList.toggle('on', r === row); r.setAttribute('aria-selected', String(r === row)); }
      });
      row.addEventListener('dblclick', () => { if (!folder) onChoose?.(n); });
      return row;
    }) : [h('p', { class: 'set-desc' }, forSave ? 'No folders here: it goes at the top of the space.' : 'Nothing here yet.')]));
  };
  const load = async () => {
    try {
      if (at.place === 'recent') at.items = await srv.recent();
      else if (at.place === 'shared') at.items = await srv.shared();
      else at.items = await srv.children(at.space, at.path.at(-1)?.id ?? null);
      crumbs.replaceChildren(...kids(at.place ? [] : [h('button', { class: 'link', onClick: () => { at.path = []; load(); } }, spaces.find((s) => s.id === at.space)?.name || ''),
        ...at.path.flatMap((p, i) => [' › ', h('button', { class: 'link', onClick: () => { at.path = at.path.slice(0, i + 1); load(); } }, fileName(p))])]));
      showItems(at.items);
      onChoose && forSave && onPick?.({ space: at.space, parent: at.path.at(-1)?.id ?? null, place: at.place });
    } catch (e) { list.replaceChildren(h('p', { class: 'set-desc', role: 'alert' }, String(e.message || e))); }
  };
  const placeBtn = (label, on, go) => {
    const b = h('button', { class: `srv-place${on ? ' on' : ''}` }, label);
    b.addEventListener('click', () => { go(); drawPlaces(); load(); });
    return b;
  };
  const drawPlaces = () => {
    places.replaceChildren(...kids(
      forSave ? null : placeBtn('Recent', at.place === 'recent', () => { at.place = 'recent'; }),
      forSave ? null : placeBtn('Shared with me', at.place === 'shared', () => { at.place = 'shared'; }),
      h('div', { class: 'srv-label' }, 'Spaces'),
      ...spaces.filter((s) => !forSave || (s.role && ['editor', 'manager'].includes(s.role))).map((s) => placeBtn(s.name, !at.place && at.space === s.id, () => { at.place = null; at.space = s.id; at.path = []; }))));
  };
  find.addEventListener('input', async () => {
    const q = find.value.trim().toLowerCase();
    if (!q) { showItems(at.items); return; }
    // The server searches names and the words in files.
    const found = await srv.search(q).catch(() => []);
    if (find.value.trim().toLowerCase() === q) showItems(found.filter((n) => n.kind === 'file'));
  });
  root.append(...kids(forSave ? null : find, h('div', { class: 'srv-panes' }, places, h('div', { class: 'srv-right' }, crumbs, list))));
  (async () => {
    try {
      spaces = await srv.spaces();
      if (forSave) {
        at.place = null;
        // Where New was asked for (that folder), else My files.
        const there = start && spaces.find((s) => s.id === start.space && ['editor', 'manager'].includes(s.role));
        at.space = there ? start.space : spaces.find((s) => s.kind === 'personal')?.id ?? spaces[0]?.id;
        if (there && start.parent != null) {
          const f = await srv.api('GET', `/v1/nodes/${start.parent}`).then((n) => srv.named([n])[0]).catch(() => null);
          if (f) at.path = [f];
        }
      }
      drawPlaces();
      load();
    } catch (e) { list.replaceChildren(h('p', { class: 'set-desc', role: 'alert' }, String(e.message || e))); }
  })();
  return { root, here: () => ({ space: at.space, parent: at.path.at(-1)?.id ?? null }) };
}

export async function openFromServer() {
  const { modalShell, toast, refresh } = ui.env;
  if (!signedIn()) return;
  const srv = ui.server;
  let picked = null;
  const open = async (n) => {
    try {
      const out = await srv.open(n.id);
      await refresh();
      const others = srv.here(out.id).length;
      toast(others ? `Editing together with ${others} other${others === 1 ? '' : 's'}` : `Opened from ${srv.name}`);
      return true;
    } catch (e) { toast(String(e.message || e)); return false; }
  };
  let shell;
  const b = browser({ onPick: (n) => { picked = n; }, onChoose: (n) => { shell.close(); open(n); } });
  shell = modalShell(`Open from ${srv.name}`, [b.root], [['Cancel', () => true], ['Open', () => (picked ? open(picked) : false), true]]);
  shell.body.classList.add('srv-modal');
}

/// Opens file `node` on the server at `url`: a link from the server's
/// Files page (Edit in …). Signs in first when needed, then opens it.
/// A page a server serves opens only that server's links; elsewhere a
/// server this device hasn't signed in to asks first.
export async function openLinked(url, node) {
  const { toast, refresh } = ui.env;
  const srv = ui.server;
  try {
    srv.onCertificate ??= askCertificate;
    // Not opened on purpose: null (nothing else to offer).
    const to = originOf(url);
    if (!to) { toast('That link to a server document is broken.'); return null; }
    if (srv.hostedHere && to !== location.origin) {
      toast(`This link is for another server (${hostOf(url)}).`);
      return null;
    }
    if (!srv.hostedHere && to !== originOf(srv.url) && !(await srv.knows(to)) && !(await askToLeave(url))) return null;
    if (srv.url !== url.replace(/\/+$/, '') || !srv.info) await srv.connect(url);
    if (!signedIn()) {
      if (srv.session) return false;
      // Signing in from Settings: open it once that's done (or give up after a while).
      for (let i = 0; i < 600 && !srv.ready; i++) await new Promise((r) => setTimeout(r, 500));
      if (!srv.ready) return false;
    }
    const out = await srv.open(node);
    await refresh();
    const others = srv.here(out.id).length;
    toast(others ? `Editing together with ${others} other${others === 1 ? '' : 's'}` : `Opened from ${srv.name}`);
    return true;
  } catch (e) {
    toast(String(e.message || e));
    return false;
  }
}

/// `beforeSave(name)`: runs before the document goes up (Plan names itself
/// after it, so the first version has the name).
export async function saveToServer(title, { beforeSave } = {}) {
  const { h, modalShell, toast, refresh } = ui.env;
  if (!signedIn()) return false;
  const srv = ui.server;
  const name = h('input', { class: 'field', 'aria-label': 'Name', value: title || '' });
  const b = browser({ forSave: true, start: newPlace() });
  return new Promise((resolve) => {
    const shell = modalShell(`Save to ${srv.name}`, [h('label', { class: 'srv-lbl' }, 'Name', name), b.root], [
      ['Cancel', () => { resolve(false); return true; }],
      ['Save', async () => {
        const where = b.here();
        if (!name.value.trim()) { name.focus(); return false; }
        try {
          await beforeSave?.(name.value.trim());
          const out = await srv.saveNew(where.space, where.parent, name.value.trim());
          // Saved: the dialog closes even if bringing the lists up to date fails.
          try { await refresh(); } catch (e) { toast(String(e.message || e)); }
          // Write: pictures shown from files on this computer went in as copies.
          toast(out?.copied ? `Saved to ${srv.name}. ${out.copied === 1 ? 'A picture from a file on this computer was' : `${out.copied} pictures from files on this computer were`} copied in, so everyone sees ${out.copied === 1 ? 'it' : 'them'}.` : `Saved to ${srv.name}`);
          // The name it was saved under (Plan names the plan after it).
          resolve(name.value.trim());
          return true;
        } catch (e) { toast(String(e.message || e)); return false; }
      }, true],
    ]);
    shell.body.classList.add('srv-modal');
    name.focus();
    name.select();
  });
}

const ROLES = [['viewer', 'Can view'], ['commenter', 'Can comment'], ['editor', 'Can edit'], ['manager', 'Can manage']];

/// File › Share…: who can open the server document in tab `id`.
export async function shareDialog(id, title) {
  const { h, modalShell, toast } = ui.env;
  const srv = ui.server;
  const node = srv.nodeOf(id);
  if (!node) { toast('Share works for documents on a server: File › Save to a server first.'); return; }
  let access, dir, custom = [];
  try { [access, dir] = await Promise.all([srv.access(node), srv.directory()]); }
  catch (e) { toast(String(e.message || e)); return; }
  // Roles an admin made for sharing (Admin › Roles), after the built-in ones.
  try { custom = (await srv.api('GET', '/v1/roles')).roles.filter((r) => r.kind === 'document'); } catch { /* an older server */ }
  const body = h('div', { class: 'srv-share' });
  const nameOf = (w) => (w.user ? (dir.people.find((p) => p.id === w.user)?.display_name || dir.people.find((p) => p.id === w.user)?.email || dir.people.find((p) => p.id === w.user)?.username || 'Someone')
    : w.group ? `${dir.groups.find((g) => g.id === w.group)?.name || 'A group'} (group)` : 'Everyone');
  const draw = () => {
    const pick = h('select', { class: 'field', 'aria-label': 'Add a person or group' }, h('option', { value: '' }, 'Add a person or group…'),
      dir.groups.map((g) => h('option', { value: `g${g.id}` }, `${g.name} (group)`)),
      dir.people.filter((p) => p.id !== srv.session.account.id).map((p) => h('option', { value: `u${p.id}` }, p.display_name ? (p.email ? `${p.display_name} (${p.email})` : p.display_name) : p.email || p.username)));
    pick.addEventListener('change', () => {
      const v = pick.value;
      if (!v) return;
      const who = v[0] === 'g' ? { group: +v.slice(1) } : { user: +v.slice(1) };
      if (!access.grants.some((g) => JSON.stringify(g.who) === JSON.stringify(who))) access.grants.push({ who, role: 'editor' });
      draw();
    });
    body.replaceChildren(...kids(
      h('label', { class: 'srv-check' }, h('input', { type: 'radio', name: 'srv-own', checked: !access.own, onChange: () => { access.own = false; draw(); } }), 'The same people as its folder'),
      h('label', { class: 'srv-check' }, h('input', { type: 'radio', name: 'srv-own', checked: access.own, onChange: () => { access.own = true; draw(); } }), 'Only these people'),
      access.own ? [pick, ...access.grants.map((g, i) => {
        const role = h('select', { class: 'field', 'aria-label': `Access for ${nameOf(g.who)}` },
          // Draw and Calc have no comments: "Can comment" would only be "Can view".
          ROLES.filter(([v]) => v !== 'commenter' || !['draw', 'calc'].includes(srv.app) || g.role === v).map(([v, l]) => h('option', { value: v, selected: !g.custom && g.role === v }, l)),
          custom.map((r) => h('option', { value: `c${r.id}`, selected: g.custom === r.id, title: r.about || null }, r.name)));
        role.addEventListener('change', () => {
          const c = custom.find((r) => `c${r.id}` === role.value);
          if (c) Object.assign(g, { role: c.base, custom: c.id });
          else { g.role = role.value; delete g.custom; }
        });
        return h('div', { class: 'srv-grant' }, h('span', { class: 'srv-name' }, nameOf(g.who)), role,
          h('button', { class: 'btn small', 'aria-label': `Remove ${nameOf(g.who)}`, onClick: () => { access.grants.splice(i, 1); draw(); } }, 'Remove'));
      })] : null,
      access.own && !access.grants.length ? h('p', { class: 'set-desc' }, 'Nobody but you yet.') : null));
  };
  draw();
  modalShell(`Share “${title}”`, [h('p', {}, `On ${srv.name}. People can open it once it's shared with them.`), body], [
    ['Cancel', () => true],
    ['Save', async () => {
      try {
        // Keep yourself on your own list.
        if (access.own && !access.grants.some((g) => g.who.user === srv.session.account.id)) access.grants.unshift({ who: { user: srv.session.account.id }, role: 'manager' });
        await srv.setAccess(node, access);
        toast('Sharing saved');
        return true;
      } catch (e) { toast(String(e.message || e)); return false; }
    }, true],
  ]).body.classList.add('srv-modal');
}

/// File › Save a checkpoint…: the document as it is now, kept as a version
/// of its own with a description. (It saves itself as you type: these are
/// the moments worth going back to.)
export async function checkpointDialog(id, title) {
  const { h, modalShell, toast } = ui.env;
  const srv = ui.server;
  if (srv.nodeOf(id) == null) { toast('Checkpoints are for documents on a server.'); return false; }
  const what = h('input', { class: 'field', 'aria-label': 'What it is', placeholder: 'Sent to the client, say' });
  const err = h('p', { class: 'set-desc', role: 'alert' });
  return new Promise((resolve) => {
    const shell = modalShell(`Save a checkpoint of “${title}”`, [
      h('p', {}, 'It saves itself as you type. A checkpoint keeps it as it is now, under a description, so you can go back to it from Version history.'),
      h('label', { class: 'srv-lbl' }, 'What it is', what), err], [
      ['Cancel', () => { resolve(false); return true; }],
      ['Save the checkpoint', async () => {
        const label = what.value.trim();
        if (!label) { err.textContent = 'Say what it is: it’s how you’ll find it again.'; what.focus(); return false; }
        try { await srv.save(id, { label }); } catch (e) { err.textContent = String(e.message || e); return false; }
        toast(`Saved the checkpoint “${label}”`);
        resolve(true);
        return true;
      }, true],
    ]);
    shell.body.classList.add('srv-modal');
    what.focus();
  });
}

/// File › Version history: checkpoints (kept, with what they are) and the
/// latest versions saved as you type; any can be opened as a copy, named,
/// or (managers) deleted.
export async function historyDialog(id, title) {
  const { h, modalShell, toast } = ui.env;
  const srv = ui.server;
  const node = srv.nodeOf(id);
  if (!node) { toast('Version history is for documents on a server.'); return; }
  let dir;
  try { dir = await srv.directory(); } catch (e) { toast(String(e.message || e)); return; }
  const role = srv.docs?.get(id)?.role;
  const manage = role === 'manager';
  const edit = manage || role === 'editor';
  const who = (u) => { const p = dir.people.find((x) => x.id === u); return p ? p.display_name || p.email || p.username : 'Someone'; };
  const msg = h('p', { class: 'set-desc', role: 'status' });
  const listBox = h('div', { class: 'srv-versions' });
  const tidy = h('div', { class: 'srv-tidy' });
  let shell;
  const fail = (e) => { msg.textContent = String(e.message || e); };
  const draw = async () => {
    let list;
    try { list = await srv.versions(node); } catch (e) { fail(e); return; }
    const current = list[0]?.n;
    const name = (v) => v.label || `Version ${v.n}`;
    listBox.replaceChildren(...(list.length ? list.map((v) => h('div', { class: `srv-version${v.label ? ' checkpoint' : ''}` },
      h('div', { class: 'srv-vtext' },
        h('span', { class: 'srv-name' }, name(v), v.n === current ? h('span', { class: 'srv-tag' }, 'Current') : null),
        h('span', { class: 'srv-when' }, `${new Date(v.created).toLocaleString()} · ${who(v.author)}${v.label ? '' : ' · saved automatically'}`)),
      h('div', { class: 'srv-vacts' },
        h('button', { class: 'btn small', onClick: async () => {
          try {
            const copy = await srv.version(node, v.n);
            await ui.env.openCopy(`${title} (${v.label || `version ${v.n}`})`, copy);
            shell.close();
          } catch (e) { fail(e); }
        } }, 'Open a copy'),
        edit ? h('button', { class: 'btn small plain', title: v.label ? 'Change what it’s called' : 'Keep it, under a description', onClick: () => rename(v) }, v.label ? 'Rename…' : 'Make it a checkpoint…') : null,
        manage && v.n !== current ? h('button', { class: 'btn small plain', title: 'Delete this version', onClick: async () => {
          try { await srv.forgetVersion(node, v.n); msg.textContent = `Deleted ${name(v)}.`; draw(); } catch (e) { fail(e); }
        } }, 'Delete') : null)))
      : [h('p', { class: 'set-desc' }, 'No saved versions yet.')]));
    const old = list.filter((v) => v.n !== current);
    const autos = old.filter((v) => !v.label).length;
    tidy.replaceChildren(...(manage && old.length ? [
      autos ? h('button', { class: 'btn small plain', onClick: () => forget(false, autos) }, `Delete the ${autos === 1 ? 'one' : autos} saved automatically`) : null,
      h('button', { class: 'btn small plain', onClick: () => forget(true, old.length) }, `Delete all but the current one (${old.length})`),
    ].filter(Boolean) : []));
  };
  const rename = (v) => {
    const input = h('input', { class: 'field', 'aria-label': 'What it is', value: v.label || '' });
    const box = modalShell(v.label ? 'Rename the checkpoint' : `Make version ${v.n} a checkpoint`, [
      h('p', {}, v.label ? 'Leave it empty to make it an ordinary version again (only the latest 20 of those are kept).' : 'It’s kept under a description, however many versions are saved after it.'),
      h('label', { class: 'srv-lbl' }, 'What it is', input)], [
      ['Cancel', () => true],
      [v.label ? 'Rename' : 'Make it a checkpoint', async () => {
        if (!v.label && !input.value.trim()) { input.focus(); return false; }
        try { await srv.labelVersion(node, v.n, input.value.trim()); } catch (e) { toast(String(e.message || e)); return false; }
        draw();
        return true;
      }, true]]);
    box.body.classList.add('srv-modal');
    input.focus();
    input.select();
  };
  const forget = (checkpoints, n) => {
    const box = modalShell('Delete versions?', [h('p', {}, checkpoints
      ? `All ${n} versions but the current one go, checkpoints too. This can’t be undone.`
      : `The ${n === 1 ? 'version' : `${n} versions`} saved automatically go; checkpoints and the current version stay. This can’t be undone.`)], [
      ['Cancel', () => true],
      ['Delete them', async () => {
        try { const r = await srv.forgetVersions(node, { checkpoints }); msg.textContent = `Deleted ${r?.deleted ?? n} ${(r?.deleted ?? n) === 1 ? 'version' : 'versions'}.`; } catch (e) { toast(String(e.message || e)); return false; }
        draw();
        return true;
      }, true]]);
    box.body.classList.add('srv-modal');
  };
  shell = modalShell(`Versions of “${title}”`, [
    h('p', {}, 'It saves itself as you type, and the latest 20 of those versions are kept. Checkpoints (File › Save a checkpoint…) stay until someone deletes them. Opening one gives you a copy; the document itself doesn’t change.'),
    listBox, msg, tidy], [['Close', () => true, true]]);
  shell.body.classList.add('srv-modal', 'srv-history');
  await draw();
}

/// Picks a file on the server made by `app` ('draw', 'write'); resolves to
/// its node, or null.
export async function pickFile(title, app) {
  const { modalShell } = ui.env;
  if (!signedIn()) return null;
  const srv = ui.server;
  return new Promise((resolve) => {
    let picked = null;
    let shell;
    const done = (n) => resolve(n);
    // Only that app's files can be chosen.
    const b = browser({ app, onPick: (n) => { picked = n; }, onChoose: (n) => { shell.close(); done(n); } });
    shell = modalShell(title, [b.root], [['Cancel', () => { done(null); return true; }], ['Choose', () => { if (!picked) return false; done(picked); return true; }, true]]);
    shell.body.classList.add('srv-modal');
  });
}

/// Changes from an earlier run that never reached the server (it closed
/// while offline): offer to send them, once per sign-in.
let offered = '';
export async function offerUnsent() {
  const srv = ui.server;
  if (!srv?.ready || offered === `${srv.url} ${srv.session.token}`) return;
  offered = `${srv.url} ${srv.session.token}`;
  const { h, modalShell, toast } = ui.env;
  for (const k of await srv.unsent()) {
    const name = k.name || 'A document';
    await new Promise((resolve) => {
      const shell = modalShell('Changes not sent yet', [
        h('p', {}, `“${name}” was changed on ${new Date(k.at).toLocaleString()} while this computer couldn’t reach the server, and closed before the changes were sent.`)],
      [['Discard them', async () => { await srv.forgetKept(k); resolve(); return true; }],
        ['Send them', async () => {
          try {
            const r = await srv.sendKept(k);
            if (r.saved) toast(`Sent: “${name}” has a new version with your changes.`);
            else {
              // Someone saved since: both kept, to compare by eye.
              await ui.env.openCopy(`${name} (your unsent changes)`, r.copy);
              await srv.forgetKept(k);
              toast(`Someone saved “${name}” since. Your changes opened as a copy beside it: compare and copy what you need.`);
            }
          } catch (e) { toast(String(e.message || e)); }
          resolve();
          return true;
        }, true]]);
      shell.body.classList.add('srv-modal');
    });
  }
}
