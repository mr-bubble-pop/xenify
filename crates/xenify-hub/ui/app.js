// Xenify Hub's window: drawn from the state the Hub reports (render()),
// clicks go through data-act. Downloads run in the Hub; the window asks
// for the state again while anything is happening.

const invoke = (name, args = {}) => window.__TAURI__.core.invoke('hub', { name, args });
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const COLOURS = { write: '#C2410C', draw: '#0F766E', calc: '#C2410C', chat: '#1D4ED8', plan: '#C2410C', server: '#1C1917', localai: '#7C3AED', postgres: '#475569' };
const CHANNELS = [['release', 'Release'], ['preview', 'Preview'], ['nightly', 'Nightly']];
const svg = (d, size = 18) => `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
// Lucide shapes (ISC licence): settings, more-horizontal, minus, square, x.
const ICON = {
  settings: svg('<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>'),
  more: svg('<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>'),
  min: svg('<path d="M5 12h14"/>', 16),
  max: svg('<rect width="14" height="14" x="5" y="5" rx="2"/>', 16),
  close: svg('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>', 16),
};

const state = { hub: null, menu: null, dialog: null, form: {}, error: '', mfa: {}, queue: [] };

const channelName = (c) => (c?.fixed ? `Fixed at ${c.fixed}` : CHANNELS.find(([k]) => k === c?.channel)?.[1] || 'Release');
const size = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(0)} MB` : `${Math.max(1, Math.round(b / 1e3))} KB`);

function status(p) {
  const j = p.job;
  if (j?.state === 'downloading') return `<span class="bar"><i style="width:${j.total ? Math.min(100, (100 * j.done) / j.total).toFixed(1) : 0}%"></i></span>Downloading ${esc(j.version)} · ${size(j.done)} of ${size(j.total)}`;
  if (j?.state === 'installing') return `Installing ${esc(j.version)}…`;
  if (j?.state === 'failed') return `<span class="new">Didn't install: ${esc(j.message)}</span>`;
  if (p.id === 'localai' && state.hub.localai_running && !p.to) return `${esc(p.installed)} · <span class="ok">Running</span> at ${esc(state.hub.localai.address)}`;
  if (p.id === 'server' && state.hub.server_service) return `<span class="${state.hub.server_running ? 'ok' : ''}">${state.hub.server_running ? 'Running' : 'Starting'}</span> with the computer · updates itself${state.hub.server_admin?.includes('/setup/') ? ' · waiting to be set up' : ''}`;
  if (p.id === 'server' && state.hub.server_running && !p.to) return `${esc(p.installed)} · <span class="ok">Running</span>${state.hub.server_admin?.includes('/setup/') ? ' · waiting to be set up' : ''}`;
  if (p.component && p.needed_by?.length && !p.to) return `${esc(p.installed)} · used by ${esc(p.needed_by.join(', '))}`;
  if (p.ready && p.installed) return `${esc(p.installed)} · <span class="new">Restart ${esc(p.name)} to use it</span>`;
  const by = state.hub.managed_by;
  if (p.locked && p.choice?.fixed) return `<span>Kept at ${esc(p.choice.fixed)} by ${esc(by)}</span>`;
  const head = `${esc(p.installed)} · ${esc(channelName(p.choice))}${p.locked ? ` (set by ${esc(by)})` : ''}`;
  if (p.to) return `${head} · <span class="new">${esc(p.to.version)} ready</span>`;
  return `${head} · up to date`;
}

function productMenu(p) {
  const by = state.hub.managed_by;
  const lock = p.locked ? 'disabled' : '';
  const radio = (on, label, act, extra = '') => `<button class="mi" role="menuitemradio" aria-checked="${on}" data-act="${act}" data-id="${p.id}" ${extra} ${lock}><span class="chk">${on ? '✓' : ''}</span>${label}</button>`;
  return `<div class="menu" role="menu" aria-label="${esc(p.name)} options">
    <div class="mh">Get updates from${p.locked ? ` · ${esc(by)} decides` : ''}</div>
    ${CHANNELS.map(([k, n]) => radio(p.choice?.channel === k, n, 'channel', `data-channel="${k}"`)).join('')}
    ${radio(!!p.choice?.fixed, p.choice?.fixed ? `Fixed version: ${esc(p.choice.fixed)}…` : 'Fixed version…', 'fixed')}
    <div class="sep"></div>
    <button class="mi" role="menuitemcheckbox" aria-checked="${p.mode === 'automatic'}" data-act="mode" data-id="${p.id}" data-mode="${p.mode === 'automatic' ? 'manual' : 'automatic'}" ${p.mode_locked ? 'disabled' : ''}><span class="chk">${p.mode === 'automatic' ? '✓' : ''}</span>Update automatically${p.mode_locked ? ` · ${esc(by)} decides` : ''}</button>
    <div class="sep"></div>
    ${p.previous ? `<button class="mi" role="menuitem" data-act="roll_back" data-id="${p.id}"><span class="chk"></span>Go back to ${esc(p.previous)}</button>` : ''}
    <button class="mi" role="menuitem" data-act="folder" data-id="${p.id}"><span class="chk"></span>Show in folder</button>
    <button class="mi" role="menuitem" data-act="remove" data-id="${p.id}" ${p.required ? `disabled title="${esc(by)} needs it on this computer"` : p.needed_by?.length ? `disabled title="${esc(p.needed_by.join(', '))} needs it"` : ''}><span class="chk"></span>Remove</button>
  </div>`;
}

function installedRow(p) {
  const busy = p.job && p.job.state !== 'failed';
  return `<div class="prod">
    <span class="app" style="background:${COLOURS[p.id] || '#57534E'}" aria-hidden="true">${esc(p.name[0])}</span>
    <div class="what"><b>${esc(p.name)}</b><div class="dim">${status(p)}</div></div>
    ${p.to && !busy && !p.ready && !(p.id === 'server' && state.hub.server_service) ? `<button class="btn main" data-act="update" data-id="${p.id}">Update</button>` : ''}
    ${p.id === 'localai' && state.hub.localai_running ? `<button class="btn" data-act="ai-how">How to use it</button>`
      : p.component ? '' : p.id === 'server' && state.hub.server_running
      ? `<button class="btn" data-act="admin">Admin pages</button>`
      : `<button class="btn" data-act="open" data-id="${p.id}">${p.id === 'server' ? 'Start' : 'Open'}</button>`}
    <div class="more-wrap"><button class="icon-btn ${state.menu === p.id ? 'on' : ''}" aria-label="More for ${esc(p.name)}" aria-haspopup="menu" aria-expanded="${state.menu === p.id}" data-act="menu" data-id="${p.id}">${ICON.more}</button>${state.menu === p.id ? productMenu(p) : ''}</div>
  </div>`;
}

/// The steadiest channel with a version of `p` for this computer.
const earliest = (p) => CHANNELS.map(([k]) => k).find((k) => p.versions.some((v) => v.channel === k));

/// Products the Hub could install now, and what "Install all apps" means:
/// the apps (the server and the AI runner are for running a server, so
/// they're only installed when asked for).
const installable = (p) => p.allowed && !p.component && !(p.job && p.job.state !== 'failed') && !state.queue.includes(p.id) && (p.to || (earliest(p) && !p.locked));
const isApp = (p) => p.id !== 'server' && p.id !== 'localai';

/// Installs one product (on the channel that has it when it isn't out on the
/// one chosen yet).
async function installOne(p) {
  if (p.to) return act('install', { product: p.id });
  return (await act('choose', { product: p.id, channel: earliest(p) })) && act('install', { product: p.id });
}

/// Installs several, one after another; the rest wait in the list.
async function installMany(ids) {
  state.queue.push(...ids.filter((id) => !state.queue.includes(id)));
  render();
  while (state.queue.length) {
    const id = state.queue[0];
    const p = state.hub.products.find((x) => x.id === id);
    if (p && !p.installed) await installOne(p);
    state.queue.shift();
  }
  render();
}

function otherRow(p) {
  if (state.queue.includes(p.id) && !(p.job && p.job.state !== 'failed')) {
    return `<div class="prod">
    <span class="app" style="background:${COLOURS[p.id] || '#57534E'}" aria-hidden="true">${esc(p.name[0])}</span>
    <div class="what"><b>${esc(p.name)}</b><div class="dim">Waiting to install…</div></div>
    <button class="btn" disabled>Install</button>
  </div>`;
  }
  const busy = p.job && p.job.state !== 'failed';
  const tag = p.id === 'server' ? 'Runs on this computer' : p.id === 'localai' ? 'AI runner' : p.component ? 'Part of the server' : '';
  return `<div class="prod">
    <span class="app" style="background:${COLOURS[p.id] || '#57534E'}" aria-hidden="true">${esc(p.name[0])}</span>
    <div class="what"><b>${esc(p.name)}</b>${tag ? `<span class="tag">${tag}</span>` : ''}<div class="dim">${busy || p.job ? status(p) : esc(p.about)}</div></div>
    ${!p.allowed ? `<span class="dim">Not allowed by ${esc(state.hub.managed_by)}</span>`
      : p.to ? `<button class="btn" data-act="install" data-id="${p.id}" ${busy ? 'disabled' : ''}>Install</button>`
      : earliest(p) && !p.locked ? `<button class="btn" data-act="install-on" data-id="${p.id}" data-channel="${earliest(p)}" ${busy ? 'disabled' : ''}>Install ${CHANNELS.find(([k]) => k === earliest(p))[1]}</button>`
      : '<span class="dim">Not out for this computer yet</span>'}
  </div>`;
}

const CODE_LABEL = { app: 'Code from your authenticator app', email: 'Code from the email', backup: 'Backup code' };

// Signing in: the server and the email address first; then the password, or
// (an account without a password yet, or a forgotten one) a code, emailed
// or from an admin, and a new password.
function signInFields(f) {
  const stage = f.stage || 'user';
  if (stage === 'user') {
    return `<label class="field">Server address<input class="inp" id="srv-url" placeholder="e.g. office-server:7700" value="${esc(f.url || '')}"></label>
      <label class="field">Email<input class="inp" id="srv-user" inputmode="email" autocomplete="username" spellcheck="false" value="${esc(f.user || '')}"></label>
      <div class="row"><button class="btn main" data-act="connect">Next</button></div>`;
  }
  const where = `<p class="dim" style="margin:0">${esc((f.url || '').replace(/^https?:\/\//, ''))} · ${esc(f.user || '')} · <button class="link" data-act="srv-back">Change</button></p>`;
  // Plain HTTP beyond this network: said before the password is typed.
  const plain = f.plain ? '<p role="alert" style="margin:0">This server doesn’t use HTTPS, so your password would travel unprotected. Sign in only if you trust every network between you and it.</p>' : '';
  if (stage === 'password') {
    return `${where}${plain}<label class="field">Password<input class="inp" id="srv-pass" type="password" autocomplete="current-password"></label>
      <div class="row"><button class="btn main" data-act="connect">Sign in</button><button class="link" data-act="srv-forgot">Forgot password?</button></div>`;
  }
  return `${where}${plain}<p class="dim" style="margin:0">${esc(f.note || '')}</p>
    <label class="field">Code<input class="inp" id="srv-code-admin" spellcheck="false" autocomplete="one-time-code"></label>
    <label class="field">Pick a password<input class="inp" id="srv-pass" type="password" autocomplete="new-password"></label>
    <label class="field">Password again<input class="inp" id="srv-pass2" type="password" autocomplete="new-password"></label>
    <div class="row"><button class="btn main" data-act="connect">Set password and sign in</button>${f.mails ? '<button class="link" data-act="srv-new-code">Send a new code</button>' : ''}</div>`;
}

// Has the server email the person signing in a code, and asks for it.
async function emailCode(note) {
  const f = state.form;
  try {
    await invoke('forgot', { url: f.url, username: f.user });
    Object.assign(f, { stage: 'code', note, mails: true });
    state.error = '';
  } catch (e) { state.error = String(e); }
  render();
  document.getElementById('srv-code-admin')?.focus();
}

// Signing in, password right: the code (or, not set up yet, where to set it up).
function secondStep(st) {
  const m = state.mfa;
  const remember = `<label class="check"><input type="checkbox" id="srv-remember" data-act="remember" ${m.remember ? 'checked' : ''}>Remember this computer for 30 days</label>`;
  const code = (method) => `<label class="field">${CODE_LABEL[method]}<input class="inp" id="srv-code" autocomplete="one-time-code" spellcheck="false"></label>`;
  const again = '<button class="btn" data-act="step-cancel">Start again</button>';
  // Two-step sign-in is set up on the server's own pages, never in an app.
  if (st.setup) {
    const page = `${(state.form.url || '').replace(/\/+$/, '')}/files/`;
    return `<p class="dim" style="margin:0"><b>Two-step sign-in.</b> ${esc(st.name)} asks for a code as well as your password, and you haven’t set that up yet. Sign in once on its web page (${esc(page.replace(/^https?:\/\//, ''))}) to set it up, then sign in here again.</p>
      <div class="row">${again}</div>`;
  }
  const has = (x) => st.methods.includes(x);
  const method = m.method || (has('app') ? 'app' : has('email') ? 'email' : 'backup');
  return `<p class="dim" style="margin:0"><b>Two-step sign-in</b> for ${esc(st.name)}.${m.sent ? ' A code is on its way to your email.' : ''}</p>
    ${code(method)}${remember}
    <div class="row"><button class="btn main" data-act="step-code" data-m="${method}">Sign in</button>${again}</div>
    <div class="row">
      ${has('app') && method !== 'app' ? '<button class="btn" data-act="step-use" data-m="app">Use your authenticator app</button>' : ''}
      ${has('email') ? `<button class="btn" data-act="step-email">${m.sent ? 'Email me another code' : 'Email me a code'}</button>` : ''}
      ${has('backup') && method !== 'backup' ? '<button class="btn" data-act="step-use" data-m="backup">Use a backup code</button>' : ''}
    </div>`;
}

function dialog() {
  const d = state.dialog;
  if (!d) return '';
  if (d.kind === 'fixed') {
    const p = state.hub.products.find((x) => x.id === d.id);
    return `<div class="back" data-act="close-bg"><div class="dialog" role="dialog" aria-modal="true" aria-label="Fixed version of ${esc(p.name)}">
      <h2>Keep ${esc(p.name)} at one version</h2>
      <p class="dim" style="margin:0">It stays at this version until you choose a channel again.</p>
      <label class="field">Version<select class="inp" id="fixed-version">${p.versions.map((v) => `<option value="${esc(v.version)}" ${v.version === (p.choice?.fixed || p.installed) ? 'selected' : ''}>${esc(v.version)} · ${esc(CHANNELS.find(([k]) => k === v.channel)?.[1] || v.channel)} · ${esc(v.date)}</option>`).join('')}</select></label>
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="fixed-ok" data-id="${p.id}">Keep it at this version</button></div>
    </div></div>`;
  }
  if (d.kind === 'certificate') {
    const c = d.seen;
    const host = c.host.replace(/:443$/, '');
    return `<div class="back"><div class="dialog" role="alertdialog" aria-modal="true" aria-label="This server uses its own certificate">
      <h2>This server uses its own certificate</h2>
      ${c.changed ? `<p role="alert" style="margin:0">${esc(host)} showed a different certificate before. If its admin didn't make a new one, something else may be answering in its place: don't connect until you've checked.</p>` : ''}
      <p style="margin:0">${esc(host)} has no domain name, so its HTTPS certificate is signed by the server itself${c.name ? ` (“${esc(c.name)}”)` : ''}. To be sure it's your server, check with its admin that this fingerprint matches the one on its admin pages (Admin › Network):</p>
      <p class="fingerprint">${esc(c.fingerprint)}</p>
      <p class="dim" style="margin:0">Trust this server: this computer remembers it, and asks again only if the certificate changes. Ignore: connect now and ask again next time.</p>
      <div class="row end"><button class="btn" data-act="cert-cancel">Cancel</button><button class="btn" data-act="cert-once">Ignore this time</button><button class="btn main" data-act="cert-trust">Trust this server</button></div>
    </div></div>`;
  }
  if (d.kind === 'choose') {
    const list = state.hub.products.filter((p) => !p.installed && installable(p));
    const n = list.filter((p) => d.picked.includes(p.id)).length;
    return `<div class="back" data-act="close-bg"><div class="dialog" role="dialog" aria-modal="true" aria-label="Choose what to install">
      <h2>Choose what to install</h2>
      ${list.map((p) => `<label class="check"><input type="checkbox" data-act="pick" data-id="${p.id}" ${d.picked.includes(p.id) ? 'checked' : ''}><span><b>${esc(p.name)}</b><br><span class="dim">${esc(p.about)}</span></span></label>`).join('')}
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="install-picked" ${n ? '' : 'disabled'}>${n === 1 ? 'Install 1' : `Install ${n}`}</button></div>
    </div></div>`;
  }
  if (d.kind === 'ai') {
    const a = state.hub.localai;
    return `<div class="back" data-act="close-bg"><div class="dialog" role="dialog" aria-modal="true" aria-label="Using the AI runner">
      <h2>Using the AI runner</h2>
      <p style="margin:0">It runs AI models on this computer for the Xenify server. In the server's admin pages, open <b>AI</b> and set:</p>
      <label class="field">Program<input class="inp" readonly value="LocalAI"></label>
      <label class="field">Address<input class="inp" readonly value="${esc(a.address)}"></label>
      <label class="field">Its models folder on this computer<input class="inp" readonly value="${esc(a.models)}"></label>
      <p class="dim" style="margin:0">Then add models there from Hugging Face or Civitai. Only this computer can reach the runner.</p>
      <div class="row end"><button class="btn" data-act="close">Done</button></div>
    </div></div>`;
  }
  const h = state.hub;
  const f = state.form;
  return `<div class="back" data-act="close-bg"><div class="dialog" role="dialog" aria-modal="true" aria-label="Hub settings">
    <h2>Settings</h2>
    <section aria-label="Xenify server"><h3>Xenify server</h3>
      ${h.step ? secondStep(h.step) : h.server
        ? `<p class="dim" style="margin:0">Signed in to <b>${esc(h.server.name)}</b> (${esc(h.server.url.replace(/^https?:\/\//, ''))}) as ${esc(h.server.user)}. Write, Draw, Calc, Plan and Chat use it too, and its admin decides which apps and versions this computer gets.</p>
           <div class="row"><button class="btn" data-act="disconnect">Sign out</button></div>
           <p class="dim" style="margin:0">Signing out signs every app out of it. An app set to use a different server (in its own settings) keeps that one.</p>`
        : `<p class="dim" style="margin:0">Optional. Sign in to your Xenify server once here and every app uses it; its admin can also look after updates here.</p>
           ${signInFields(f)}`}
    </section>
    <section aria-label="Updates"><h3>Updates</h3>
      <div class="row"><button class="btn" data-act="check">Check for updates now</button></div>
    </section>
    <section aria-label="This computer"><h3>This computer</h3>
      <label class="check"><input type="checkbox" data-act="autostart" ${state.autostart ? 'checked' : ''}>Start Xenify Hub when I sign in (it waits in the tray and keeps apps up to date)</label>
      ${h.products.some((p) => p.id === 'server' && p.installed) ? `<label class="check"><input type="checkbox" data-act="server-service" ${h.server_service ? 'checked' : ''} ${h.service_busy ? 'disabled' : ''}>Start the Xenify server when this computer starts, even before anyone signs in${h.service_busy ? ' · waiting for your password…' : ' (asks for your computer\'s password once)'}</label>` : ''}
      ${h.products.some((p) => p.id === 'localai' && p.installed) ? `<label class="check"><input type="checkbox" data-act="localai-autostart" ${h.localai_autostart ? 'checked' : ''}>Start the AI runner with the Hub</label>` : ''}
    </section>
    ${state.error ? `<p class="err" role="alert">${esc(state.error)}</p>` : ''}
    <div class="row end"><button class="btn" data-act="close">Done</button></div>
  </div></div>`;
}

function render() {
  const h = state.hub;
  if (!h) return;
  const focus = document.activeElement?.id;
  const keep = {};
  for (const id of ['srv-url', 'srv-user', 'srv-pass', 'srv-code', 'fixed-version']) {
    const el = document.getElementById(id);
    if (el) keep[id] = el.value;
  }
  const installed = h.products.filter((p) => p.installed);
  const others = h.products.filter((p) => !p.installed && (!p.component || p.job));
  const updates = installed.filter((p) => p.to && !p.job && !p.ready).length;
  document.getElementById('app').innerHTML = `
    <header class="top" data-tauri-drag-region>
      <span class="mark" aria-hidden="true" data-tauri-drag-region>X</span><b data-tauri-drag-region>Xenify Hub</b>
      <button class="pill" data-act="settings">${h.managed_by || h.server ? `<span class="dot"></span>Managed by ${esc(h.managed_by || h.server.name)}` : '<span class="dot off"></span>Not managed by a server'}</button>
      <button class="icon-btn" aria-label="Settings" data-act="settings">${ICON.settings}</button>
      <span class="win"><button class="icon-btn" aria-label="Minimise" data-act="win-min">${ICON.min}</button><button class="icon-btn" aria-label="Maximise" data-act="win-max">${ICON.max}</button><button class="icon-btn" aria-label="Close" data-act="win-close">${ICON.close}</button></span>
    </header>
    <main><div class="col">
      ${h.error ? `<div class="banner" role="alert">${esc(h.error)}</div>` : ''}
      <section class="card" aria-label="Installed">
        <div class="card-head"><h1>Installed</h1>${updates > 1 ? `<button class="btn" data-act="update_all">Update all</button>` : ''}</div>
        ${installed.map(installedRow).join('') || `<p class="dim empty">${h.listed ? 'Nothing yet: install an app below.' : h.error ? 'The list of apps couldn\'t be fetched.' : 'Looking for Xenify apps…'}</p>`}
      </section>
      ${others.length ? `<section class="card" aria-label="More apps"><div class="card-head"><h2>More from Xenify</h2>${others.filter(installable).length > 1 ? `<span class="row"><button class="btn" data-act="choose-apps">Choose…</button>${others.filter((p) => installable(p) && isApp(p)).length > 1 ? `<button class="btn main" data-act="install-all">Install all apps</button>` : ''}</span>` : ''}</div>${others.map(otherRow).join('')}</section>` : ''}
      <p class="dim note">Every download is checked against Xenify's signature before it's installed.</p>
    </div></main>
    ${dialog()}`;
  for (const [id, v] of Object.entries(keep)) {
    const el = document.getElementById(id);
    if (el) el.value = v;
  }
  if (focus) document.getElementById(focus)?.focus();
}

async function refresh() {
  try { state.hub = await invoke('state'); } catch (e) { state.error = String(e); }
  render();
}

async function certChosen(remember) {
  const d = state.dialog;
  state.dialog = d.back;
  try {
    await invoke('certificate', { url: d.url, fingerprint: d.seen.fingerprint, remember });
  } catch (e) { state.error = String(e); render(); return; }
  render();
  await acts.connect();
  state.certPassword = null;
}

async function act(name, args) {
  state.error = '';
  try {
    const out = await invoke(name, args);
    if (out && out.products) state.hub = out;
    return true;
  } catch (e) {
    state.error = String(e);
    if (!state.dialog) state.hub.error = state.error;
    return false;
  } finally {
    render();
  }
}

const win = () => window.__TAURI__.window.getCurrentWindow();
const acts = {
  menu: (el) => { state.menu = state.menu === el.dataset.id ? null : el.dataset.id; render(); },
  open: (el) => act('open', { product: el.dataset.id }),
  install: (el) => act('install', { product: el.dataset.id }),
  'install-all': () => installMany(state.hub.products.filter((p) => !p.installed && installable(p) && isApp(p)).map((p) => p.id)),
  // The apps start ticked; the server and the AI runner only when wanted.
  'choose-apps': () => {
    state.dialog = { kind: 'choose', picked: state.hub.products.filter((p) => !p.installed && installable(p) && isApp(p)).map((p) => p.id) };
    render();
  },
  pick: (el) => {
    const d = state.dialog;
    d.picked = el.checked ? [...d.picked, el.dataset.id] : d.picked.filter((id) => id !== el.dataset.id);
    render();
  },
  'install-picked': () => {
    const ids = state.dialog.picked;
    state.dialog = null;
    installMany(ids);
  },
  update: (el) => act('update', { product: el.dataset.id }),
  update_all: () => act('update_all'),
  // Not out on Release yet: on the channel that has it.
  'install-on': async (el) => {
    if (await act('choose', { product: el.dataset.id, channel: el.dataset.channel })) await act('install', { product: el.dataset.id });
  },
  channel: (el) => { state.menu = null; return act('choose', { product: el.dataset.id, channel: el.dataset.channel }); },
  mode: (el) => { state.menu = null; return act('choose', { product: el.dataset.id, mode: el.dataset.mode }); },
  fixed: (el) => { state.menu = null; state.dialog = { kind: 'fixed', id: el.dataset.id }; render(); },
  'fixed-ok': async (el) => {
    const v = document.getElementById('fixed-version').value;
    if (await act('choose', { product: el.dataset.id, fixed: v })) { state.dialog = null; render(); }
  },
  roll_back: (el) => { state.menu = null; return act('roll_back', { product: el.dataset.id }); },
  folder: (el) => { state.menu = null; return act('folder', { product: el.dataset.id }); },
  remove: (el) => { state.menu = null; return act('remove', { product: el.dataset.id }); },
  settings: async () => {
    try { state.autostart = (await invoke('autostart')).on; } catch {}
    state.dialog = { kind: 'settings' };
    render();
    document.getElementById('srv-url')?.focus();
  },
  autostart: async (el) => {
    try { state.autostart = (await invoke('set_autostart', { on: el.checked })).on; } catch (e) { state.error = String(e); }
    render();
  },
  'server-service': (el) => act('server_service', { on: el.checked }),
  'localai-autostart': (el) => act('localai_autostart', { on: el.checked }),
  'ai-how': () => { state.dialog = { kind: 'ai' }; render(); },
  close: () => { state.dialog = null; state.error = ''; render(); },
  'close-bg': (el, e) => { if (e.target === el) acts.close(); },
  'srv-back': () => { state.form = { url: state.form.url, user: state.form.user }; state.error = ''; render(); },
  'srv-forgot': () => emailCode('If this account has an email address, a code is on its way to it: type it here and pick a new password. If it has none, ask an admin to reset your password.'),
  'srv-new-code': () => emailCode('A new code is on its way to your email.'),
  connect: async () => {
    const f = state.form;
    const stage = f.stage || 'user';
    if (stage === 'user') {
      f.url = document.getElementById('srv-url')?.value ?? f.url;
      f.user = document.getElementById('srv-user')?.value ?? f.user;
    }
    state.mfa = {};
    const password = document.getElementById('srv-pass')?.value || state.certPassword || '';
    let done;
    if (stage === 'user') {
      if (!f.user.trim()) return void document.getElementById('srv-user')?.focus();
      state.error = '';
      try {
        const step = await invoke('login_start', { url: f.url, username: f.user });
        f.plain = !!step.plain;
        if (step.next === 'pick_password' && step.can_email) return emailCode('We’ve emailed you a code. Type it here and pick your password.');
        f.stage = step.next === 'pick_password' ? 'code' : 'password';
        f.note = 'This account doesn’t have a password yet. Type the code your admin gave you and pick a password.';
        render();
        document.getElementById(f.stage === 'code' ? 'srv-code-admin' : 'srv-pass')?.focus();
        return;
      } catch (e) { state.error = String(e); done = false; }
    } else if (stage === 'code') {
      const code = document.getElementById('srv-code-admin')?.value || f.code || '';
      const again = document.getElementById('srv-pass2')?.value ?? password;
      if (password.length < 8) { state.error = 'Use a password of at least 8 characters.'; return render(); }
      if (password !== again) { state.error = 'The two passwords aren’t the same.'; return render(); }
      f.code = code;
      done = await act('claim', { url: f.url, username: f.user, code, password, plain_ok: !!f.plain });
    } else done = await act('connect', { url: f.url, username: f.user, password, plain_ok: !!f.plain });
    if (done) { state.form = {}; return; }
    // The server's own certificate: ask whether to trust it, then sign in again.
    if (state.error.startsWith('certificate:')) {
      const url = state.error.slice('certificate:'.length);
      state.error = '';
      try {
        const seen = await invoke('certificate', { url });
        state.certPassword = password;
        state.dialog = { kind: 'certificate', seen, url, back: state.dialog };
      } catch (e) { state.error = String(e); }
      render();
    }
  },
  'cert-cancel': () => { state.dialog = state.dialog.back; state.certPassword = null; state.error = 'Not signed in: the server’s certificate wasn’t trusted.'; render(); },
  'cert-trust': () => certChosen(true),
  'cert-once': () => certChosen(false),
  disconnect: () => act('disconnect'),
  remember: (el) => { state.mfa.remember = el.checked; },
  'step-use': (el) => { state.mfa.method = el.dataset.m; state.error = ''; render(); document.getElementById('srv-code')?.focus(); },
  'step-email': async () => {
    if (await act('step_email')) { state.mfa.method = 'email'; state.mfa.sent = true; render(); document.getElementById('srv-code')?.focus(); }
  },
  'step-code': async (el) => {
    const code = document.getElementById('srv-code').value;
    if (!code.trim()) return document.getElementById('srv-code').focus();
    if (await act('step_code', { method: el.dataset.m, code, remember: !!state.mfa.remember })) state.mfa = {};
  },
  'step-cancel': async () => { state.mfa = {}; await act('step_cancel'); },
  admin: () => act('admin'),
  check: () => act('check'),
  'win-min': () => win().minimize(),
  'win-max': () => win().toggleMaximize(),
  'win-close': () => win().close(),
};

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (!el) {
    if (state.menu) { state.menu = null; render(); }
    return;
  }
  if (el.disabled) return;
  acts[el.dataset.act]?.(el, e);
});
document.addEventListener('keydown', (e) => {
  // Ctrl+W closes the window (the Hub stays in the tray), Ctrl+Q quits.
  if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.code === 'KeyW' || e.code === 'KeyQ')) {
    e.preventDefault();
    if (e.code === 'KeyW') win().close();
    else window.__TAURI__.core.invoke('quit');
    return;
  }
  if (e.key === 'Escape' && !e.isComposing) {
    // A menu first; a dialog as its Cancel does (the certificate question goes back to where it was asked).
    if (state.menu) { state.menu = null; render(); return; }
    if (state.dialog?.kind === 'certificate') acts['cert-cancel']();
    else if (state.dialog) acts.close();
    return;
  }
  if (e.key === 'Enter' && e.target.id === 'srv-pass') acts.connect();
  else if (e.key === 'Enter' && e.target.id === 'srv-code') document.querySelector('[data-act="step-code"], [data-act="setup-finish"]')?.click();
  // Enter in a dialog is its main button (or its only one), except where Enter means something itself (the sign-in boxes too).
  else if (e.key === 'Enter' && state.dialog && !e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing && !e.target.closest?.('textarea, select, button, a[href], [id^="srv-"]')) {
    const btns = [...document.querySelectorAll('.dialog > .row.end .btn')];
    const main = btns.find((b) => b.classList.contains('main')) || (btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) { e.preventDefault(); main.click(); }
  }
});
document.querySelector('#app').addEventListener('dblclick', (e) => { if (e.target.closest('[data-tauri-drag-region]') && !e.target.closest('button')) win().toggleMaximize(); });

// While something is happening, ask again often; otherwise now and then.
async function loop() {
  await refresh();
  const busy = state.queue.length || state.hub && (state.hub.checking || state.hub.service_busy || state.hub.products.some((p) => p.job && p.job.state !== 'failed'));
  setTimeout(loop, busy ? 400 : 5000);
}
loop();
if (window.XENIFY_TEST) setTimeout(() => window.XENIFY_TEST({ state, invoke, render, acts, log: (text) => window.__TAURI__.core.invoke('log', { text }) }), 300);
