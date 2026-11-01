// Plan on a Xenify server: plans opened and saved there, edited together,
// and the server's register (people, groups, things, holidays, time away,
// every plan's bookings) so load adds up across plans nobody has open here.
// The server works every plan out itself (dates, bookings, what other plans
// wait for); the app asks it about other plans and shows what comes back.

import { Server } from './server.js';
import { ui as serverUi, serverRow, openFromServer, openLinked, saveToServer, shareDialog, historyDialog, checkpointDialog, offerUnsent } from './serverui.js';
import { call, live, tabs as engineTabs } from './engine.js';
import { h, toast, dialogKeys } from './ui.js';
import { initials, faceColour } from './fmt.js';
import * as platform from './platform.js';
import { snapshot } from './snapshot.js';
import { spelling } from './text.js';

let app = null;
export let server = null;
/// The last register the server sent (null: not signed in).
let reg = null;
let pushTimer = null;

export function setUpServer(theApp, { newTabFor }) {
  app = theApp;
  server = new Server({
    call: async (name, args) => call(name, args && 'id' in args ? { ...args, tab: args.id } : args),
    apply: async (name, args) => {
      if (name === 'remote_open') {
        // Opening one: a tab of its own. Putting this plan there: this tab.
        if (args.data || args.blank) newTabFor(args.name);
        // Nothing saved in it yet (made on the Files page): a new plan of that name.
        if (args.blank) call('new', { title: args.name });
        call('remote_open', args);
        // Read-only until the server says what this person may change (onJoined).
        call('set_access', { may: [] });
        const tab = app.tab;
        // Hidden until it has caught up with the server (live_start): what was
        // saved last can be older than what everyone has done since.
        Object.assign(tab, { remote: true, name: args.name, path: null, handle: null, loading: Date.now() });
        // Its people and the issues it waits for in other plans, from the server.
        setTimeout(() => { worldFor(tab).then(() => app.render()).catch(() => {}); }, 0);
        app.render();
        return { id: engineTabs.front() };
      }
      const out = call(name, 'id' in args ? { ...args, tab: args.id } : args);
      if (name === 'live_start') { const t = app.tabs.find((x) => x.id === args.id); if (t) t.loading = null; }
      if (name.startsWith('live_') || name === 'remote_saved') { app.render(); followLater(); }
      return out;
    },
    live,
    app: 'plan',
    ext: 'xplan',
    onChange: () => { app.render(); offerUnsent(); platform.serverWords(server); },
    onError: (e) => toast(String(e.message || e)),
    // Each version: what Write can show.
    afterSave: async (id) => {
      await server.savePreview(id, new TextEncoder().encode(snapshot(id)));
    },
  });
  server.onSaid = () => showPeople();
  // What I may change in this plan: everything as an editor, nothing as a
  // viewer, else what my document role (or "can comment") lists.
  // A change the server refused (your access to the plan): open the plan
  // again as the server has it, so nothing shows that isn't there.
  server.onRefused = async () => {
    for (const t of [...app.tabs]) {
      const node = server.nodeOf(t.id);
      if (node == null) continue;
      let unsent = false;
      try { unsent = call('remote_unsent', { tab: t.id }).unsent; } catch { /* closed */ }
      if (!unsent) continue;
      server.leave(t.id);
      engineTabs.close(t.id);
      app.tabs = app.tabs.filter((x) => x !== t);
      await openLinked(server.url, node).catch(() => {});
    }
    app.render();
  };
  server.onJoined = (id, d) => {
    const all = (d.role === 'editor' || d.role === 'manager') && !d.may;
    try { call('set_access', { tab: id, may: all ? null : (d.may || []) }); } catch { /* closed */ }
    app.render();
  };
  serverUi.server = server;
  serverUi.env = {
    h, toast,
    modalShell,
    showSettings: () => openServer(),
    refresh: async () => { await syncRegister(); app.render(); },
    openCopy: async (name, copy) => app.openBytes(`${name}.xplan`, Uint8Array.from(atob(copy.data), (ch) => ch.charCodeAt(0))),
  };
  app.server = server;
  app.serverBookings = (tab) => otherBookings(tab);
  setInterval(() => { if (server.ready) syncRegister().then(() => app.render()).catch(() => {}); }, 60000);
  // Without the server just now: the people as they were last time.
  const kept = keptRegister();
  if (kept && !server.ready) for (const t of app.tabs) call('set_world', { tab: t.id, resources: [...kept.people, ...kept.groups, ...kept.things] });
  if (server.session) syncRegister().then(() => app.render()).catch(() => {});
  return server;
}

/// File menu entries for the server.
export function fileItems(item) {
  const id = engineTabs.front();
  const on = server.nodeOf(id) != null;
  return [
    item(server.session ? `Open from ${server.name}…` : 'Open from a server…', { run: () => (server.session ? openFromServer() : openServer(() => server.session && openFromServer())) }),
    item(server.session ? `${on ? 'Save a copy to' : 'Save to'} ${server.name}…` : 'Save to a server…', { run: () => {
      // On a server already: a second copy. Otherwise: put this one there (named first).
      const go = () => (on ? saveToServer(app.tab?.name) : app.saveToServer());
      return server.session ? go() : openServer(() => server.session && go());
    } }),
    on ? item('Share…', { run: () => shareDialog(id, app.tab.name) }) : undefined,
    on ? item('Version history…', { run: () => historyDialog(id, app.tab.name) }) : undefined,
  ];
}

export const onServer = (id) => server?.nodeOf(id) != null;

/// Ctrl+S on a server plan: it saves itself as you type, so this only
/// makes sure what's waiting goes now (no new version each time; File ›
/// Save a checkpoint… keeps a moment, with a description).
export async function saveVersion(id) {
  await server.versionNow(id);
  toast(`Saved on ${server.name}: it saves itself as you type. File › Save a checkpoint… keeps it as it is now.`);
}

export const saveCheckpoint = (id, name) => checkpointDialog(id, name);

export function leave(id) { server?.leave(id); }

function modalShell(title, body, buttons) {
  const back = h('div', { class: 'modal-back' });
  const close = () => { back.remove(); unkey(); };
  const box = h('div', { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title }, h('h2', {}, title), body,
    h('div', { class: 'modal-actions' }, buttons.map(([label, act, main]) => h('button', { class: `btn${main ? ' main' : ''}`, onClick: async () => {
      if (!act) { close(); return; }
      if ((await act()) !== false) close();
    } }, label))));
  back.append(box);
  // Escape closes, Enter is the main button.
  const unkey = dialogKeys(back, box, close);
  back.addEventListener('keydown', (e) => e.stopPropagation());
  document.getElementById('popups').append(back);
  return { close, body: box };
}

/// File › Xenify server: connect, sign in, sign out.
export function openServer(then) {
  // Served by the server: no other server to pick; signing in is on its Files page.
  if (server?.servedHere) {
    if (!server.session) location.assign(server.signInHere);
    else then?.();
    return;
  }
  const holder = h('div', { class: 'srv-holder' });
  const redraw = () => holder.replaceChildren(serverRow(redraw));
  redraw();
  const shell = modalShell('Xenify server', [h('p', {}, 'Plans on a server are edited together, and People adds up everyone’s time across all of them.'), holder],
    [['Done', async () => { await syncRegister().catch(() => {}); app.render(); then?.(); }, true]]);
  shell.body.classList.add('srv-modal');
  shell.body.querySelector('input')?.focus();
}

// ----- The register -----

/// Fetches the register and hands it to every open plan.
export async function syncRegister() {
  if (!server?.ready) { reg = null; return; }
  const r = await server.api('GET', '/v1/plan/register');
  for (const t of r.things) t.name ||= 'A thing';
  reg = r;
  const resources = [...r.people, ...r.groups, ...r.things];
  const company = (r.holidays.company_days || []).map((d) => [d[0], d[1], d[2]]);
  for (const t of [...app.tabs]) {
    // Closed while the server was answering for the one before.
    if (!app.tabs.includes(t)) continue;
    call('set_world', { tab: t.id, resources, company_days: company, holidays: r.holidays });
    if (r.me) {
      const me = r.people.find((p) => p.id === r.me);
      call('set_me', { tab: t.id, name: me?.name || 'Someone', resource: r.me });
    }
    if (onServer(t.id)) await worldFor(t).catch(() => {});
  }
  app.prefs.serverHolidays = r.holidays;
  if (!mine.pulled) await pullSettings().catch(() => {});
  // Kept for when the server can't be reached (names on cards, people's hours).
  try { localStorage.setItem('plan.register', JSON.stringify({ server: server.url, people: r.people, groups: r.groups, things: r.things, holidays: r.holidays, me: r.me })); } catch { /* full */ }
}

// ----- Your settings, on the server so they follow you -----

const MINE = { spelling: 'plan-spelling' };
// Left from when Plan could use your own AI key; AI is the server's now.
try { localStorage.removeItem('plan-ai-use'); localStorage.removeItem('plan-ai-model'); } catch { /* none */ }
const mine = { pulled: false };

async function pullSettings() {
  const got = await server.api('GET', '/v1/me/settings/plan');
  mine.pulled = true;
  if (!Object.keys(got).length) { await pushSettings(); return; }
  for (const [k, key] of Object.entries(MINE)) {
    if (typeof got[k] === 'string') try { localStorage.setItem(key, got[k]); } catch { /* none */ }
  }
  if (got.spelling) spelling.set(got.spelling !== 'off');
}

async function pushSettings() {
  if (!server?.ready) return;
  const out = {};
  for (const [k, key] of Object.entries(MINE)) {
    const v = localStorage.getItem(key);
    if (v != null) out[k] = v;
  }
  await server.api('PUT', '/v1/me/settings/plan', out).catch(() => {});
}
document.addEventListener('plan-settings', () => { pushSettings(); });

/// The register as it was last time, for a server that can't be reached now.
export function keptRegister() {
  try {
    const k = JSON.parse(localStorage.getItem('plan.register'));
    return k && k.server === server?.url ? k : null;
  } catch { return null; }
}

/// A server plan's view of the world: the issues it waits for in other plans.
async function worldFor(tab) {
  if (!onServer(tab.id) || !server.ready) return;
  await fetchOutside(tab.id);
  // What other plans wait for here ("Holds up" in the issue sheet).
  try { tab.waiting = await server.api('GET', `/v1/plan/waiting/${server.nodeOf(tab.id)}`); } catch { tab.waiting = null; }
}

// Each plan's latest question about other plans: an older answer that comes
// back after it (without an issue linked since) is dropped.
const asked = new Map();

/// Asks the server about the issues in other plans this one waits for.
export async function fetchOutside(id) {
  const node = server?.ready ? server.nodeOf(id) : null;
  const refs = call('outside_refs', { tab: id });
  if (!refs.length || !server.ready) return;
  const n = (asked.get(id) || 0) + 1;
  asked.set(id, n);
  // With this plan's id: links that come back round to it are marked (a loop).
  const got = await server.api('POST', '/v1/plan/outside', node != null ? { refs, node } : { refs });
  if (asked.get(id) !== n) return;
  call('set_world', { tab: id, outside: got });
}

/// Opens what a mirror or a link points at ("n12:*" a whole plan, "n12:t3" one of its issues).
export function openRef(ref) {
  const [n, task] = String(ref).slice(1).split(':');
  return openPlanAt(Number(n), task && task !== '*' ? task : undefined);
}

/// Plans on the server you can open (to wait for or mirror).
export const serverPlans = () => server.api('GET', '/v1/plan/plans');
export const issuesOf = (node) => server.api('GET', `/v1/plan/issues/${node}`);
export const nodeOfTab = (id) => server?.nodeOf(id) ?? null;
/// A link to issue `key` of the plan in tab `id` on its server (opens the plan there at that issue), or null.
export function issueLink(id, key) {
  const node = server?.nodeOf(id);
  return node == null ? null : `${server.url}/plan/?open=${node}&issue=${encodeURIComponent(key)}`;
}

/// Would "`to` waits for `from`" make a loop through other plans? (Both
/// "n<plan>:<issue>" or this tab's issue ids.) The loop, issue by issue, or null.
export async function wouldLoop(id, from, to) {
  const node = server?.nodeOf(id);
  if (node == null || !server.ready) return null;
  const ref = (x) => (/^n\d+:/.test(x) ? x : `n${node}:${x}`);
  try {
    const r = await server.api('POST', '/v1/plan/loop', { from: ref(from), to: ref(to) });
    return r.loop ? r.chain : null;
  } catch { return null; }
}

/// A plan's link file (File › Save a link): what an .xplan holds now that
/// plans live on a server. Opening it opens the plan there, for whoever may.
export function linkFile(id, title) {
  return JSON.stringify({ xenify: 'plan-link', version: 1, server: server.url, node: server.nodeOf(id), title }, null, 2);
}

/// Opens what a link file points at (signing in first when needed).
export async function openLink(link) {
  if (!link?.server || !link?.node) throw new Error('This link doesn’t say which plan');
  const ok = await openLinked(link.server, link.node);
  // Another server's link this page doesn't open, or one turned down.
  if (ok === null) return false;
  if (!ok && !server.ready) openServer();
  // Signed in, but not theirs to open: whom to ask, as elsewhere in Plan.
  if (!ok && server.ready && server.url === link.server.replace(/\/+$/, '')) {
    try {
      const a = await server.api('GET', `/v1/plan/ask/${link.node}`);
      toast(`No access to “${a.name || link.title || 'this plan'}”: ask ${a.ask || 'whoever shared it'}`);
    } catch { /* gone: the server said so */ }
  }
  return ok;
}

/// Opens plan `node` on this server, then issue `task` in it.
export async function openPlanAt(node, task) {
  const have = app.tabs.find((t) => server.nodeOf(t.id) === node);
  if (have) { engineTabs.switch(have.id); app.render(); }
  else if (!(await openLinked(server.url, node))) return;
  if (task) setTimeout(() => app.openTask(task), 50);
}

/// The server's bookings for plans other than tab `tab`'s own.
/// Plans open here are left out: their tabs give their bookings as they are now.
/// A plan made earlier goes first (Booking.first).
function otherBookings(tab) {
  if (!reg) return [];
  const node = server.nodeOf(tab.id);
  // An exported copy keeps the bookings it was made with.
  if (node == null) return [];
  const open = new Set(app.tabs.map((t) => server.nodeOf(t.id)).filter((n) => n != null));
  return reg.bookings.filter((b) => b.node !== node && !open.has(b.node)).map((b) => ({
    resource: b.resource, plan: b.readable ? (b.plan || 'Another plan') : 'A plan you can’t open', task: `${b.node}:${b.task}`,
    title: b.title || 'A task', from: b.from, to: b.to, percent: b.percent, first: b.node < node, over: !!b.over,
  }));
}

/// A little after changes stop: the server has worked the plans out again,
/// so ask it about other plans' issues (their dates may have moved).
export function followLater() {
  clearTimeout(pushTimer);
  pushTimer = setTimeout(() => { for (const t of app.tabs) if (onServer(t.id)) fetchOutside(t.id).then(() => app.render()).catch(() => {}); }, 3000);
}

// ----- People and things kept on the server -----

export const isServerResource = (id) => !!reg && /^[ugt]\d+$/.test(id || '');

export async function savePerson(id, { day_minutes, country, region, away }) {
  await server.api('PUT', `/v1/plan/people/${id}`, { day_minutes, country, region });
  await server.api('PUT', `/v1/plan/away/${id}`, { times: away });
  await syncRegister();
}

export const serverHolidays = () => reg?.holidays || null;

// ----- Who else is here -----

export function showPeople() {
  const box = document.getElementById('people');
  const id = engineTabs.front();
  const here = server?.here(id) || [];
  const on = {};
  for (const s of server?.saidOf(id) || []) {
    try { const at = call('live_heard', { tab: id, data: s.data }); if (at.task) on[s.who.conn] = at.task; } catch { /* not yet */ }
  }
  app.peopleOn = Object.values(on);
  box.replaceChildren(...here.slice(0, 5).map((p) => {
    const who = p.name || p.username || 'Someone';
    return h('span', { class: 'face person', style: { background: faceColour(who), width: '26px', height: '26px', fontSize: '10.5px' }, title: `${who} is here` }, initials(who));
  }));
}

let lastSaid = '';
/// Tells the others which task this window is on.
export function sayWhere(task, view) {
  const id = engineTabs.front();
  if (!server?.docs.get(id)?.joined) return;
  const now = JSON.stringify([id, task, view]);
  if (now === lastSaid) return;
  lastSaid = now;
  try { server.say(id, call('live_say', { tab: id, task, view }).data); } catch { /* later */ }
}
