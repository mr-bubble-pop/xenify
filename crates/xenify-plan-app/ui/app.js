// Xenify Plan's screens: open plans in tabs, the menus and toolbar, and the
// views (Board, Backlog, Timeline, People; History from the View menu).
// Issues open over the view (issue.js); Create makes them (create.js). Every
// change goes through the engine (engine.js), which keeps the plan, its
// undo and History.

import { start, call, tabs as engineTabs, saveFile } from './engine.js';
import { h, icon, menu, item, modal, ask, toast, closePopups, sheetOpen, browserCloseKey } from './ui.js';
import * as platform from './platform.js';
import { setUpWindow } from './window.js';
import { todayIso, dayName, plural, initials } from './fmt.js';
import * as board from './board.js';
import * as backlog from './backlog.js';
import * as timeline from './timeline.js';
import * as people from './people.js';
import * as historyView from './history.js';
import { openTask, closeTask, renderTask, createMirror } from './issue.js';
import { openCreate } from './create.js';
import { deleteIssue } from './remove.js';
import { spelling } from './text.js';
import { estText, personColour, may, offers, ONLY } from './pieces.js';
import { planSettings, whichPerson } from './settings.js';
import { fieldsDialog } from './fields.js';
import { labelsDialog } from './labels.js';
import { Ask, openAsk } from './ai.js';
import { Mcp, programsSettings } from './mcp.js';
import { setUpServer, fileItems, onServer, saveVersion, saveCheckpoint, leave, showPeople, sayWhere, server as srv, openServer, serverPlans, linkFile, openLink, openPlanAt, syncRegister } from './serverplan.js';
import { openFromServer, serverRow, newPlace } from './serverui.js';

const VIEWS = {
  board: { label: 'Board', icon: 'board', mod: board },
  backlog: { label: 'Backlog', icon: 'list', mod: backlog },
  timeline: { label: 'Timeline', icon: 'timeline', mod: timeline },
  people: { label: 'People', icon: 'people', mod: people },
  history: { label: 'History', icon: 'clock', mod: historyView, hidden: true },
};

const RUN = Math.random().toString(36).slice(2, 10);

/// Things the screens share: the open plans and how to change them.
export const app = {
  tabs: [],
  get tab() { return this.tabs.find((t) => t.id === engineTabs.front()) || null; },
  view: null,
  /// An engine call that changes the plan: the screens are drawn again after it.
  edit(name, args = {}) {
    try {
      const out = call(name, args);
      app.render();
      return out;
    } catch (e) {
      toast(e.message);
      return null;
    }
  },
  /// An engine call that changes the plan, its error thrown (for messages under a field).
  attempt(name, args = {}) {
    const out = call(name, args);
    app.render();
    return out;
  },
  /// An engine call that only reads.
  read(name, args = {}) {
    try { return call(name, args); } catch (e) { toast(e.message); return null; }
  },
  render: () => render(),
  openTask: (id) => { app.tab.selected = id; openTask(app, id); },
  create: (preset) => openCreate(app, preset),
  setView: (v) => {
    closeTask();
    if (app.tab.viewName !== v) {
      const main = document.getElementById('main');
      main.scrollTop = 0;
      main.scrollLeft = 0;
      app.tab.scrolled = false;
      app.tab.flash = null;
    }
    app.tab.viewName = v;
    app.prefs.view = VIEWS[v]?.hidden ? app.prefs.view : v;
    app.savePrefs();
    render();
  },
  prefs: loadPrefs(),
  savePrefs() { try { localStorage.setItem('plan.prefs', JSON.stringify(app.prefs)); } catch { /* none */ } },
};
window.plan = { app, call, openAsk };
app.openBytes = (name, bytes) => openBytes(name, bytes);

function loadPrefs() {
  try { return JSON.parse(localStorage.getItem('plan.prefs')) || {}; } catch { return {}; }
}

function clock(extra = {}) {
  call('set_clock', { today: todayIso(), now: Date.now(), seed: Math.floor(Math.random() * 2 ** 52), ...extra });
  // A plan kept as a file has no names in it (no audit): only a server says who made a change.
  call('set_me', { name: '', resource: app.prefs.meResource || '' });
}

// ----- Tabs -----

function addTab(t) {
  const view = VIEWS[app.prefs.view] && !VIEWS[app.prefs.view].hidden ? app.prefs.view : 'board';
  const tab = { name: 'Untitled plan', path: null, handle: null, viewName: view, zoom: 'days', selected: null, filter: {}, ...t };
  app.tabs.push(tab);
  return tab;
}

/// A new plan: made on the server straight away (in My files), as a new
/// document is in any program; only its name is asked for, since issue
/// keys come from it ("Website relaunch" → WR-1).
export async function newPlan() {
  // Signed out: the sign-in is what's on screen.
  if (!srv?.ready) { render(); return; }
  const name = (await ask('New plan', 'Name', '', 'Create'))?.trim();
  if (!name) return;
  newLocalPlan(name);
  const tab = app.tab;
  try {
    await putOnServer(name);
  } catch (e) {
    toast(String(e.message || e));
    engineTabs.close(tab.id);
    app.tabs = app.tabs.filter((t) => t !== tab);
    if (app.tabs.length) engineTabs.switch(app.tabs[app.tabs.length - 1].id);
    render();
    return;
  }
  render();
}

/// Puts the front plan on the server, in My files, as `name`.
async function putOnServer(name) {
  const spaces = await srv.spaces();
  // Where New was asked for in Files (that folder), else My files.
  const asked = newPlace();
  const there = asked && spaces.find((x) => x.id === asked.space && ['editor', 'manager'].includes(x.role));
  const space = there ? asked.space : spaces.find((x) => x.kind === 'personal')?.id ?? spaces[0]?.id;
  if (space == null) throw new Error(`There’s nowhere on ${srv.name} you may put a plan.`);
  await srv.saveNew(space, there ? asked.parent : null, name);
  // Its people, holidays and other plans' bookings, as for a plan opened there.
  await syncRegister().catch(() => {});
}

function newLocalPlan(title = 'Untitled plan') {
  const id = engineTabs.open();
  clock();
  call('new', { title });
  call('set_item', { list: 'projects', id: 'p1', value: { name: title === 'Untitled plan' ? 'Issues' : title, prefix: keyPrefix(title), color: '#C2410C' }, what: 'Added a project' });
  const hol = app.prefs.serverHolidays?.country ? app.prefs.serverHolidays : app.prefs;
  if (hol.country) {
    call('set_plan', { field: 'settings.country', value: hol.country });
    call('set_plan', { field: 'settings.region', value: hol.region || '' });
  }
  call('saved');
  addTab({ id, name: title });
  render();
}

/// Issue keys for a new plan: "Website relaunch" → WR-1, an untitled plan → PLAN-1.
function keyPrefix(title) {
  if (!title || title === 'Untitled plan') return 'PLAN';
  const w = title.toUpperCase().replace(/[^A-Z0-9 ]/g, '').split(/\s+/).filter(Boolean);
  const p = w.length > 1 ? w.map((x) => x[0]).join('').slice(0, 4) : (w[0] || 'PLAN').slice(0, 4);
  return p || 'PLAN';
}

/// A new tab for a plan coming from the server (its engine session is empty until opened).
function newTabFor(name) {
  const id = engineTabs.open();
  clock();
  addTab({ id, name });
}

async function openBytes(name, bytes, path = null, handle = null) {
  // A link file: the plan on its server (what Save a link writes).
  if (bytes[0] === 0x7b) {
    let link = null;
    try { link = JSON.parse(new TextDecoder().decode(bytes)); } catch { /* not one */ }
    if (link?.xenify === 'plan-link') {
      try { await openLink(link); } catch (e) { toast(e.message); }
      return;
    }
  }
  const id = engineTabs.openFile(bytes);
  clock();
  // A copy (Export a copy): not on the server until put there, and plans
  // only change on a server, so it's read-only until then.
  const tab = addTab({ id, name: platform.baseName(name), path, handle, copy: true });
  call('set_access', { may: [] });
  const v = call('view');
  tab.name = v.title ? (/\(copy\)$/.test(v.title) ? v.title : `${v.title} (copy)`) : tab.name;
  if (!app.prefs.recent) app.prefs.recent = [];
  if (path) {
    app.prefs.recent = [path, ...app.prefs.recent.filter((p) => p !== path)].slice(0, 8);
    app.savePrefs();
  }
  render();
}

async function openPlan() {
  const f = await platform.openPlan();
  if (!f) return;
  try { await openBytes(f.name, f.bytes, f.path, f.handle); } catch (e) { toast(e.message); }
}

async function openPath(path) {
  try {
    const f = await platform.readPath(path);
    await openBytes(f.name, f.bytes, f.path);
  } catch (e) { toast(e.message); }
}

/// Saves the front plan: a version on the server; a copy goes onto the
/// server (Save to server). `as`: a copy as a file (Export a copy).
async function save(as = false) {
  const tab = app.tab;
  if (!tab) return false;
  if (as) return exportCopy();
  if (onServer(tab.id)) {
    try { await saveVersion(tab.id); render(); return true; } catch (e) { toast(e.message); return false; }
  }
  if (srv?.ready) {
    // A copy is read-only here; naming it on the way up is allowed (the
    // server then says what this person may change).
    if (tab.copy) call('set_access', { may: null });
    const name = tab.name.replace(/ \(copy\)$/, '');
    let ok = true;
    try {
      if (name !== tab.name) call('set_plan', { field: 'title', value: name });
      await putOnServer(name);
      tab.name = name;
      toast(`Put it on ${srv.name}, in My files`);
    } catch (e) { toast(String(e.message || e)); ok = false; }
    if (ok) { tab.copy = false; call('saved'); platform.autosave.clear(`${RUN}-${tab.id}`); render(); }
    else if (tab.copy && !onServer(tab.id)) call('set_access', { may: [] });
    return ok;
  }
  if (!srv?.ready) { openServer(() => { if (srv.ready) save(); }); return false; }
  const bytes = saveFile();
  const name = `${tab.name}.xplan`;
  const where = await platform.savePlan({ name, bytes, path: as ? null : tab.path, handle: as ? null : tab.handle });
  if (!where) return false;
  if (typeof where === 'string') { tab.path = where; tab.name = platform.baseName(where); }
  else if (where !== true) tab.handle = where;
  call('saved');
  platform.autosave.clear(`${RUN}-${tab.id}`);
  toast('Saved');
  render();
  return true;
}

/// File › Export a copy: the plan as a file to keep or send, with the names
/// of its people and the other plans' issues it waits for as they are now.
async function exportCopy() {
  const tab = app.tab;
  if (!tab) return false;
  const bytes = platform.fromB64(call('export_copy'));
  const where = await platform.savePlan({ name: `${tab.name} (copy).xplan`, bytes, path: null, handle: null });
  if (!where) return false;
  toast('Exported a copy');
  return true;
}

/// File › Save a link: a small .xplan that opens this plan on its server.
async function saveLink() {
  const tab = app.tab;
  if (!onServer(tab?.id)) return;
  const bytes = new TextEncoder().encode(linkFile(tab.id, tab.name));
  const where = await platform.savePlan({ name: `${tab.name}.xplan`, bytes, path: null, handle: null });
  if (where) toast('Saved a link. Whoever opens it opens the plan on the server, if they may.');
}

async function closeTab(id) {
  const tab = app.tabs.find((t) => t.id === id);
  if (!tab) return;
  engineTabs.switch(id);
  const v = call('view');
  // A plan on a server keeps every change as it's made: nothing to lose.
  if (v.changed && !onServer(id)) {
    const a = await modal(`Save “${tab.name}”?`, h('p', {}, 'Changes you haven’t saved will be lost.'), [['Cancel', null], ['Don’t save', 'drop'], ['Save', 'save', true]]);
    if (!a) return;
    if (a === 'save' && !(await save())) return;
  }
  platform.autosave.clear(`${RUN}-${id}`);
  leave(id);
  engineTabs.close(id);
  app.tabs = app.tabs.filter((t) => t.id !== id);
  closeTask(app);
  // The last one closed on a server's site: back to Files.
  if (!app.tabs.length && servedHere) return void (await quit());
  render();
}

async function quit() {
  for (const t of [...app.tabs]) {
    engineTabs.switch(t.id);
    if (call('view').changed) {
      await closeTab(t.id);
      if (app.tabs.some((x) => x.id === t.id)) return;
    }
  }
  if (platform.DESKTOP) return platform.quit();
  // In the browser, from a Xenify server: back to its Files page.
  if (servedHere) await srv.quitToFiles();
}

// Unsaved plans are kept every 15 s, and offered back after a crash.
setInterval(() => {
  const front = engineTabs.front();
  for (const t of app.tabs) {
    try {
      engineTabs.switch(t.id);
      // A plan on a server: the server has every change (unsent ones are kept by server.js).
      if (onServer(t.id)) { platform.autosave.clear(`${RUN}-${t.id}`); continue; }
      if (call('view').changed) platform.autosave.write(`${RUN}-${t.id}`, t.name, t.path, saveFile());
    } catch { /* next time */ }
  }
  if (front) engineTabs.switch(front);
}, 15000);

async function offerRecovery() {
  const left = await platform.autosave.leftovers(RUN);
  if (!left.length) return;
  const body = h('div', {}, h('p', {}, `Plan closed before ${left.length === 1 ? 'this plan was' : 'these plans were'} saved:`),
    h('ul', {}, left.map((l) => h('li', {}, l.name))));
  const a = await modal('Recover unsaved plans?', body, [['Delete them', 'drop'], ['Recover', 'keep', true]]);
  // Recovering doesn't take you away from the plan you're in.
  const was = engineTabs.front();
  // The empty plan Plan opened with goes when plans come back in its place.
  const blank = a === 'keep' ? app.tabs.filter((t) => !t.path && !onServer(t.id) && (engineTabs.switch(t.id), call('view')).tasks.length === 0 && !call('view').changed) : [];
  for (const l of left) {
    if (a === 'keep') {
      await openBytes(l.name, platform.fromB64(l.base64), l.path);
      // Recovered, not saved: closing it asks.
      call('changed');
      render();
    }
    if (a) platform.autosave.clear(l.key);
  }
  for (const t of blank) {
    engineTabs.close(t.id);
    app.tabs = app.tabs.filter((x) => x !== t);
  }
  const back = app.tabs.find((t) => t.id === was && !blank.includes(t));
  const to = (back || app.tabs[app.tabs.length - 1])?.id ?? was;
  if (to != null) engineTabs.switch(to);
  render();
}

// ----- Menus -----

const MENUS = {
  File: () => [
    item('New plan', { keys: 'Ctrl+N', run: () => newPlan() }),
    item('Open a file…', { keys: 'Ctrl+O', run: openPlan }),
    app.prefs.recent?.length ? item('Open recent', { sub: () => app.prefs.recent.map((p) => item(platform.fileName(p), { run: () => openPath(p) })) }) : null,
    null,
    ...fileItems(item),
    null,
    // On a server it saves itself: Ctrl+S just sends what's waiting, and a
    // checkpoint keeps a moment (with a description) in Version history.
    app.tab && onServer(app.tab.id)
      ? item('Save a checkpoint…', { run: () => saveCheckpoint(app.tab.id, app.tab.name) })
      : item('Put it on the server…', { keys: 'Ctrl+S', enabled: !!app.tab, run: () => save() }),
    app.tab && onServer(app.tab.id) ? item('Save a link to it…', { run: saveLink }) : null,
    item('Export a copy…', { keys: 'Ctrl+Shift+S', enabled: !!app.tab, run: () => save(true) }),
    null,
    item('Plan settings…', { keys: 'Ctrl+,', run: () => planSettings(app) }),
    item('Settings', { sub: () => [
      // Which server, and signing in (as Settings › Server in the other apps);
      // served by the server, there's nothing to choose: its Files page signs in.
      servedHere ? null : item(srv?.session ? `Server: ${srv.name}…` : 'Server…', { run: () => openServer() }),
      item('Programs that can use Plan…', { run: () => programsSettings(app, app.mcp) }),
    ] }),
    null,
    item('Close plan', { keys: 'Ctrl+W', run: () => closeTab(app.tab.id) }),
    platform.DESKTOP || servedHere ? item('Quit', { keys: 'Ctrl+Q', run: quit }) : null,
  ],
  Edit: () => {
    const v = app.view;
    return [
      item(v?.undo ? `Undo “${v.undo}”` : 'Undo', { keys: 'Ctrl+Z', enabled: !!v?.undo, run: () => app.edit('undo') }),
      item(v?.redo ? `Redo “${v.redo}”` : 'Redo', { keys: 'Ctrl+Y', enabled: !!v?.redo, run: () => app.edit('redo') }),
      null,
      item('Find an issue…', { keys: 'Ctrl+F', run: findTask }),
      item('Search issues', { keys: '/', enabled: !!app.tab, run: searchIssues }),
      null,
      item('Check spelling and grammar', { checked: spelling.on, run: () => spelling.toggle() }),
    ];
  },
  View: () => [
    // History (who changed what) is for plans on a server only.
    ...Object.entries(VIEWS).filter(([k, x]) => (k !== 'history' || app.view?.audit) && (!x.when || (app.view && x.when(app.view)))).map(([k, x], i) => item(x.label, { checked: app.tab?.viewName === k, keys: x.hidden ? 'Ctrl+H' : `Alt+${i + 1}`, run: () => app.setView(k) })),
    null,
    item('Only my issues', { enabled: offers(app.tab, 'mine'), checked: offers(app.tab, 'mine') && !!app.tab.filter.mine, run: () => toggleMine() }),
  ],
  Issue: () => {
    if (!app.tab) return [item('New plan', { run: () => newPlan() })];
    const sel = app.tab?.selected;
    const t = app.view?.tasks.find((x) => x.id === sel);
    return [
      item('Create issue…', { keys: 'C', run: () => openCreate(app) }),
      item('Create epic…', { run: () => openCreate(app, { kind: 'epic', parent: null }) }),
      null,
      item('Fields…', { run: () => fieldsDialog(app) }),
      item('Labels…', { run: () => labelsDialog(app) }),
      item('Board columns…', { run: () => { app.setView('board'); board.columnsDialog(app); } }),
      null,
      item('Open the selected issue', { keys: 'Enter', enabled: !!t, run: () => app.openTask(sel) }),
      item('Mirror another plan, epic or issue…', { title: 'A task here that shows when it’s done, and goes there', enabled: !!app.view?.on_server && may(app.view, 'create'), run: () => createMirror(app) }),
      item('Delete the selected issue', { keys: 'Delete', enabled: !!t, run: () => deleteSelected() }),
    ];
  },
  AI: () => [
    // AI is the server's: without it, Ask says where it comes from.
    item('Ask AI…', { keys: 'Ctrl+J', enabled: !!app.tab, run: () => openAsk(app, app.ask) }),
    null,
    item('Programs that can use Plan…', { run: () => programsSettings(app, app.mcp) }),
  ],
  Help: () => [
    item('Keyboard shortcuts', { run: shortcuts }),
    item('About Xenify Plan', { run: about }),
  ],
};

function buildMenus() {
  const box = document.getElementById('menus');
  box.replaceChildren();
  for (const [name, items] of Object.entries(MENUS)) {
    const b = h('button', { class: 'menu-btn', 'aria-haspopup': 'menu', 'aria-expanded': 'false' }, name);
    b.addEventListener('click', () => {
      const r = b.getBoundingClientRect();
      b.setAttribute('aria-expanded', 'true');
      menu(items().filter((x) => x !== undefined), r.left, r.bottom + 4, () => b.setAttribute('aria-expanded', 'false'));
    });
    box.append(b);
  }
}

// ----- Actions -----

function deleteSelected() {
  const id = app.tab?.selected;
  if (id) deleteIssue(app, id);
}

function toggleMine(e) {
  const tab = app.tab;
  if (!tab.filter.mine && !app.view.me_resource) {
    if (!app.view.resources.length) { toast('Add yourself on the People view first'); return; }
    const r = (e?.currentTarget || document.getElementById('toolbar')).getBoundingClientRect();
    whichPerson(app, r.left, r.bottom + 4, () => { tab.filter.mine = true; });
    return;
  }
  tab.filter.mine = !tab.filter.mine;
  render();
}

/// The search box, in a view that has one; elsewhere the Backlog's.
function searchIssues() {
  if (!app.tab) return;
  if (!offers(app.tab, 'search')) app.setView('backlog');
  search.focus();
}

function findTask() {
  const input = h('input', { class: 'search', placeholder: 'Find an issue', 'aria-label': 'Find an issue' });
  const box = h('div', { role: 'menu' }, input);
  const results = h('div');
  box.append(results);
  const show = () => {
    const hits = call('find', { query: input.value }).slice(0, 12);
    results.replaceChildren(...hits.map((t) => h('button', { class: 'item', onClick: () => { closePopups(); app.openTask(t.id); } },
      h('span', { class: 'key' }, t.key), h('span', {}, t.title))));
  };
  input.addEventListener('input', show);
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') results.querySelector('button')?.click(); });
  const r = document.getElementById('toolbar').getBoundingClientRect();
  import('./ui.js').then(({ popup }) => { popup(box, r.right - 380, r.bottom + 4); input.focus(); show(); });
}

function shortcuts() {
  const rows = [['Create an issue', 'C'], ['Search issues (in the Backlog)', '/'], ['Open the selected issue', 'Enter'], ['Close the issue', 'Esc'], ['Delete the selected issue', 'Delete'],
    ['Find an issue', 'Ctrl+F'], ['Undo / redo', 'Ctrl+Z / Ctrl+Y'], ['Board … People', 'Alt+1 … Alt+4'], ['History', 'Ctrl+H'],
    ['Save a description or comment', 'Ctrl+Enter'], ['Save', 'Ctrl+S'], ['New plan', 'Ctrl+N'], ['Open', 'Ctrl+O']];
  modal('Keyboard shortcuts', h('div', { class: 'keys-list' }, rows.map(([a, k]) => [h('span', {}, a), h('kbd', {}, k)])), [['Close', true, true]]);
}

async function about() {
  const lic = await platform.bundled('THIRD_PARTY_LICENSES.txt').catch(() => '');
  modal('Xenify Plan', h('div', {}, h('p', {}, 'Issues on a board and a backlog, a timeline worked out from estimates and links, and who is busy. Free and open source (MIT).'),
    h('p', { class: 'muted' }, 'Public holidays from python-holidays (MIT), worked into rules for this program.'),
    lic ? h('details', {}, h('summary', {}, 'Licences of the parts it uses'), h('pre', { class: 'licences' }, lic.slice(0, 200000))) : null),
  [['Close', true, true]]);
}

// ----- Drawing -----

function drawTabs() {
  const box = document.getElementById('tabs');
  // On a server's site one file is open at a time, like a page (File › New adds tabs).
  document.querySelector('.tabstrip').hidden = servedHere && app.tabs.length <= 1;
  const front = engineTabs.front();
  box.replaceChildren(...app.tabs.map((t) => {
    const on = t.id === front;
    const el = h('div', { class: 'tab', role: 'tab', 'aria-selected': String(on), tabindex: on ? '0' : '-1', title: t.path || t.name },
      h('span', { class: 'book', html: icon('plan', 16) }),
      h('span', { class: 'name' }, t.name),
      // A plan on a server keeps every change as it's made.
      on && app.view?.changed && !onServer(t.id) ? h('span', { class: 'dot', title: 'Not saved' }) : null,
      h('button', { class: 'close', 'aria-label': `Close ${t.name}`, html: icon('close', 12), onClick: (e) => { e.stopPropagation(); closeTab(t.id); } }));
    el.addEventListener('click', () => { if (!on) { engineTabs.switch(t.id); closeTask(); render(); } });
    el.addEventListener('dblclick', async () => {
      const name = await ask('Rename the plan', 'Name', t.name, 'Rename');
      if (name && name.trim()) { app.edit('set_plan', { field: 'title', value: name.trim() }); t.name = name.trim(); render(); }
    });
    return el;
  }));
}

// The search box stays the same element, so typing in it isn't interrupted.
const search = h('input', { class: 'search-box', id: 'search', type: 'search', placeholder: 'Search issues', 'aria-label': 'Search issues', autocomplete: 'off' });
const searchWrap = h('div', { class: 'search-wrap' }, h('span', { class: 'search-icon', html: icon('search', 15) }), search);
search.addEventListener('input', () => { app.tab.filter.text = search.value; render(); });
search.addEventListener('keydown', (e) => { if (e.key === 'Escape' && search.value) { e.preventDefault(); search.value = ''; app.tab.filter.text = ''; render(); } else if (e.key === 'Escape') search.blur(); });

function toolbar(v, tab) {
  const bar = document.getElementById('toolbar');
  if (!bar.contains(searchWrap)) bar.replaceChildren(h('div', { class: 'tb-left' }), searchWrap, h('div', { class: 'tb-right' }));
  if (document.activeElement !== search) search.value = tab.filter.text || '';
  // (Not .hidden: the wrap's display: flex would beat it.)
  searchWrap.style.display = offers(tab, 'search') ? '' : 'none';
  const seg = h('div', { class: 'segs views', role: 'tablist', 'aria-label': 'View' },
    Object.entries(VIEWS).filter(([k, x]) => !x.hidden && (!x.when || x.when(v) || tab.viewName === k)).map(([k, x]) => h('button', { class: 'seg', role: 'tab', 'aria-selected': String(tab.viewName === k), 'aria-pressed': String(tab.viewName === k), onClick: () => app.setView(k) },
      h('span', { html: icon(x.icon, 15) }), x.label)));
  bar.querySelector('.tb-left').replaceChildren(...[
    h('button', { class: 'icon-btn', 'aria-label': 'Undo', title: v.undo ? `Undo “${v.undo}” (Ctrl+Z)` : 'Undo', disabled: !v.undo, html: icon('undo'), onClick: () => app.edit('undo') }),
    h('button', { class: 'icon-btn', 'aria-label': 'Redo', title: 'Redo (Ctrl+Y)', disabled: !v.redo, html: icon('redo'), onClick: () => app.edit('redo') }),
    h('span', { class: 'sep' }),
    seg,
    h('span', { class: 'sep' }),
    !may(v, 'create') ? accessChip(v) : offers(tab, 'create') ? h('button', { class: 'btn main small', title: 'Create an issue (C)', onClick: () => openCreate(app) }, h('span', { html: icon('plus', 15) }), 'Create') : null,
    !may(v, 'create') || offers(tab, 'create') ? h('span', { class: 'sep' }) : null].filter(Boolean));
  const sel = tab.filter.people || [];
  // Faces: the people with work in this plan (and me, and whoever is picked),
  // the rest behind "+n".
  const busy = new Set([...v.tasks.flatMap((t) => [t.who, t.by]), v.me_resource, ...sel].filter(Boolean));
  const inPlan = v.resources.filter((p) => busy.has(p.id));
  const pick = (p) => { tab.filter.people = sel.includes(p.id) ? sel.filter((x) => x !== p.id) : [...sel, p.id]; render(); };
  const face = (p) => h('button', {
    class: `av${sel.includes(p.id) ? ' on' : ''}`, 'aria-pressed': String(sel.includes(p.id)), title: p.name, style: { background: personColour(v, p.id) },
    onClick: () => pick(p),
  }, initials(p.name));
  const rest = [...inPlan.slice(8), ...v.resources.filter((p) => !busy.has(p.id))];
  const faces = h('div', { class: 'face-filter', role: 'group', 'aria-label': 'Show the issues of' },
    ...inPlan.slice(0, 8).map(face),
    rest.length ? h('button', { class: 'av more-faces', title: 'Everyone else', 'aria-label': `${rest.length} more people`, onClick: (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      menu(rest.slice(0, 40).map((p) => item(p.name, { checked: sel.includes(p.id), run: () => pick(p) })), r.left, r.bottom + 4);
    } }, `+${rest.length > 99 ? '99' : rest.length}`) : null);
  const extra = VIEWS[tab.viewName].mod.tools?.(app, v, tab) || [];
  // (Missing pieces are left out: replaceChildren would show null as text.)
  bar.querySelector('.tb-right').replaceChildren(...[
    offers(tab, 'people') && v.resources.length ? faces : null,
    offers(tab, 'mine') ? h('button', { class: `chipbtn${tab.filter.mine ? ' on' : ''}`, 'aria-pressed': String(!!tab.filter.mine), onClick: toggleMine }, 'Only my issues') : null,
    offers(tab, 'labels') && v.labels.length ? labelFilter(v, tab) : null,
    offers(tab, 'only') ? onlyFilter(v, tab) : null,
    (offers(tab, 'search') && tab.filter.text) || (offers(tab, 'mine') && tab.filter.mine) || (offers(tab, 'only') && tab.filter.only?.length)
      || (offers(tab, 'labels') && tab.filter.labels?.length) || (offers(tab, 'people') && sel.length) ? h('button', { class: 'tool small', onClick: () => { tab.filter = {}; search.value = ''; render(); } }, 'Clear') : null,
    h('span', { class: 'grow' }),
    ...extra].filter(Boolean));
}

/// "Filters ▾": only issues that are unassigned, without an estimate, late…
function onlyFilter(v, tab) {
  const on = tab.filter.only || [];
  const kinds = Object.entries(ONLY).filter(([k]) => k !== 'fields' || v.fields.some((f) => f.required));
  const count = (k) => v.tasks.filter((t) => ONLY[k].test(t)).length;
  const b = h('button', { class: `chipbtn${on.length ? ' on' : ''}`, 'aria-haspopup': 'menu' },
    on.length ? on.map((k) => ONLY[k].label).join(', ') : 'Filters', h('span', { class: 'caret', 'aria-hidden': 'true' }, ' ▾'));
  b.addEventListener('click', () => {
    const r = b.getBoundingClientRect();
    menu(kinds.map(([k, x]) => item(`${x.label}  ·  ${count(k)}`, { checked: on.includes(k), run: () => {
      tab.filter.only = on.includes(k) ? on.filter((y) => y !== k) : [...on, k];
      render();
    } })), r.left, r.bottom + 4);
  });
  return b;
}

/// "Labels ▾": show only issues with any of the labels picked.
function labelFilter(v, tab) {
  const on = tab.filter.labels || [];
  const b = h('button', { class: `chipbtn${on.length ? ' on' : ''}`, 'aria-haspopup': 'menu' }, on.length ? `Labels: ${on.join(', ')}` : 'Labels', h('span', { class: 'caret', 'aria-hidden': 'true' }, ' ▾'));
  b.addEventListener('click', () => {
    const r = b.getBoundingClientRect();
    menu([
      ...v.labels.map((l) => item(`${l.name}  ·  ${l.count}`, { checked: on.some((x) => x.toLowerCase() === l.name.toLowerCase()), run: () => {
        tab.filter.labels = on.some((x) => x.toLowerCase() === l.name.toLowerCase()) ? on.filter((x) => x.toLowerCase() !== l.name.toLowerCase()) : [...on, l.name];
        render();
      } })),
      null,
      item('Rename or remove labels…', { run: () => labelsDialog(app) }),
    ], r.left, r.bottom + 4);
  });
  return b;
}

function status(v, tab) {
  const el = document.getElementById('status');
  const hol = v.settings.country ? v.settings : v.holidays_from;
  const region = hol ? `${hol.country}${hol.region ? ` (${hol.region})` : ''}${v.holidays_from ? ' from the server' : ''}` : 'none';
  const on = onServer(tab.id);
  const others = on ? srv.here(tab.id).length : 0;
  const open = v.tasks.filter((t) => !t.epic && !t.mirror && !t.is_done);
  const left = open.reduce((n, t) => n + (t.work || 0), 0);
  const ms = v.tasks.filter((t) => t.milestone && !t.is_done && t.placed).sort((a, b) => a.start.localeCompare(b.start))[0];
  el.replaceChildren(...[
    on ? h('span', {}, `On ${srv.name}${others ? `, with ${plural(others, 'other')}` : ''}`)
      : h('span', { class: 'copy-note' }, 'A copy, read only until it’s on a server · ', h('button', { class: 'linkish', onClick: () => save() }, srv?.ready ? `Put it on ${srv.name}` : 'Connect to a server')),
    h('span', {}, h('b', {}, String(open.length)), ` open ${open.length === 1 ? 'issue' : 'issues'}`),
    left ? h('span', {}, h('b', {}, estText(left, v.settings.day_minutes)), ' of work left') : null,
    ms ? h('button', { class: `linkish${ms.late ? ' late' : ''}`, onClick: () => app.openTask(ms.id) }, `${ms.title}: ${dayName(ms.start)}${ms.late ? `, ${plural(ms.late_by, 'day')} late` : ''}`) : null,
    h('button', { class: 'linkish quiet', onClick: () => planSettings(app) }, `Holidays: ${region}`),
    h('span', { class: 'right' }, `Today ${dayName(v.today)}`),
  ].filter(Boolean));
}

/// Every open plan's bookings go to the others, so People adds them all up
/// (plus, on a server, the bookings it keeps for plans not open here).
function shareBookings() {
  if (app.tabs.length < 2 && !app.serverBookings) return;
  const all = call('all_tabs', { name: 'bookings' });
  // Which goes first: the plan made earlier (on a server), else the tab opened earlier.
  const rank = (id) => app.server?.nodeOf?.(id) ?? 1e15 + app.tabs.findIndex((y) => y.id === id);
  for (const t of app.tabs) {
    const others = all.filter((x) => x.tab !== t.id && Array.isArray(x.out)).flatMap((x) => {
      const name = app.tabs.find((y) => y.id === x.tab)?.name || 'Another plan';
      return x.out.map((b) => ({ ...b, plan: name, first: rank(x.tab) < rank(t.id) }));
    });
    call('set_world', { tab: t.id, bookings: [...others, ...(app.serverBookings?.(t) || [])] });
  }
}

// The plan last drawn: a toast's Undo is for that plan's change, so it goes
// when another plan comes to the front (Undo would undo that one's instead).
let drawnTab = null;

export function render() {
  const tab = app.tab;
  if ((tab?.id ?? null) !== drawnTab) {
    document.querySelector('.toast .toast-act')?.closest('.toast')?.remove();
    drawnTab = tab?.id ?? null;
  }
  if (!tab) { home(); return; }
  document.body.classList.remove('at-home', 'signed-out');
  if (tab.loading) { opening(tab); return; }
  if (tab.viewName === 'people' || tab.viewName === 'timeline') {
    try { shareBookings(); } catch (e) { console.error(e); }
  }
  let v;
  try { v = call('view'); } catch (e) { toast(e.message); return; }
  app.view = v;
  if (tab.viewName === 'history' && !v.audit) tab.viewName = 'board';
  if (v.title && v.title !== tab.name && !tab.path && !tab.copy) tab.name = v.title;
  document.title = `${tab.name} – Xenify Plan`;
  drawTabs();
  toolbar(v, tab);
  const main = document.getElementById('main');
  main.dataset.view = tab.viewName;
  try {
    VIEWS[tab.viewName].mod.render(main, v, tab, app);
  } catch (e) {
    console.error(e);
    main.replaceChildren(h('p', { class: 'empty' }, `Couldn’t draw this view: ${e.message}`));
  }
  if (tab.flash) requestAnimationFrame(() => main.querySelector(`[data-id="${tab.flash}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' }));
  renderTask(app);
  status(v, tab);
  if (srv) { showPeople(); sayWhere(tab.selected, tab.viewName); }
}

/// A plan from the server until it's all there: its name and a spinner, no
/// half-loaded issues.
function opening(tab) {
  document.title = `${tab.name} – Xenify Plan`;
  drawTabs();
  document.getElementById('toolbar').replaceChildren();
  document.getElementById('status').replaceChildren();
  const slow = Date.now() - tab.loading > 8000;
  document.getElementById('main').replaceChildren(h('div', { class: 'opening', role: 'status' },
    h('span', { class: 'spinner', 'aria-hidden': 'true' }),
    h('p', {}, `Opening ${tab.name}…`),
    slow ? h('p', { class: 'f-hint' }, 'The server is taking a while to answer. It opens as soon as it does.') : null));
  // Once, to say so when it's slow.
  if (!slow && !tab.slowTimer) tab.slowTimer = setTimeout(() => { tab.slowTimer = null; if (app.tab === tab && tab.loading) render(); }, 8100);
}

// The sign-in on the first screen: kept while it's on screen, so drawing
// again doesn't wipe what's being typed.
let signIn = null;

/// No plan open: signed out, only the sign-in (Plan keeps plans on a
/// server, so there's nothing else to do); signed in, the plans there.
function home() {
  document.body.classList.add('at-home');
  document.body.classList.toggle('signed-out', !srv?.ready);
  drawTabs();
  document.title = 'Xenify Plan';
  document.getElementById('toolbar').replaceChildren();
  document.getElementById('status').replaceChildren(h('span', {}, srv?.ready ? `On ${srv.name}` : 'Not signed in'));
  const main = document.getElementById('main');
  main.dataset.view = 'home';
  if (!srv?.ready) {
    // Drawn again only when where the server is at has changed (connected, a second step…).
    const state = () => JSON.stringify([srv?.url, !!srv?.info, !!srv?.session, !!srv?.step, srv?.mismatch || '']);
    const at = state();
    if (signIn?.isConnected && signIn.dataset.at === at) return;
    const holder = h('div', { class: 'srv-holder' });
    const redraw = () => {
      if (srv.ready) { signIn = null; syncRegister().catch(() => {}).then(() => render()); return; }
      holder.replaceChildren(serverRow(redraw));
      if (signIn) signIn.dataset.at = state();
    };
    redraw();
    signIn = h('div', { class: 'home' },
      h('div', { class: 'home-card signin' },
        h('h1', {}, 'Xenify Plan'),
        h('p', {}, 'Plans are kept on a Xenify server, where everyone plans together.'),
        holder));
    signIn.dataset.at = at;
    main.replaceChildren(signIn);
    signIn.querySelector('input')?.focus();
    return;
  }
  signIn = null;
  const list = h('div', { class: 'home-list' }, h('p', { class: 'f-hint' }, 'Getting your plans…'));
  main.replaceChildren(h('div', { class: 'home' },
    h('div', { class: 'home-card wide' },
      h('div', { class: 'home-top' }, h('h1', {}, `Plans on ${srv.name}`), h('span', { class: 'grow' }),
        h('button', { class: 'btn plain', onClick: () => openFromServer() }, 'Browse folders…'),
        h('button', { class: 'btn main', onClick: () => newPlan() }, h('span', { html: icon('plus', 15) }), 'New plan')),
      list)));
  serverPlans().then((ps) => {
    list.replaceChildren(...(ps.length ? ps.slice(0, 40).map((p) => h('button', { class: 'home-plan', onClick: () => openPlanAt(p.node) },
      h('span', { class: 'book', html: icon('plan', 18) }), h('b', {}, p.name || 'Untitled plan'), h('span', { class: 'muted' }, plural(p.issues, 'issue'))))
      : [h('p', {}, 'No plans yet. Make the first one with New plan.')]));
  }).catch((e) => list.replaceChildren(h('p', { class: 'f-error' }, String(e.message || e))));
}

// ----- Keys -----

document.addEventListener('keydown', (e) => {
  const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement || e.target?.isContentEditable;
  const k = e.key.toLowerCase();
  const ctrl = e.ctrlKey || e.metaKey;
  if (!srv?.ready && !app.tabs.length) return;
  if (ctrl && k === 's') { e.preventDefault(); save(e.shiftKey); return; }
  if (ctrl && k === 'n') { e.preventDefault(); newPlan(); return; }
  if (ctrl && k === 'o') { e.preventDefault(); openPlan(); return; }
  if (!app.tab) return;
  if (ctrl && k === 'w') { e.preventDefault(); closeTab(app.tab.id); return; }
  if (ctrl && k === 'q') { e.preventDefault(); quit(); return; }
  const leave = browserCloseKey(e);
  if (leave === 'close') { e.preventDefault(); closeTab(app.tab.id); return; }
  if (leave === 'quit' && servedHere) { e.preventDefault(); quit(); return; }
  if (ctrl && k === ',') { e.preventDefault(); planSettings(app); return; }
  if (ctrl && k === 'f') { e.preventDefault(); findTask(); return; }
  if (ctrl && k === 'h' && app.view?.audit) { e.preventDefault(); app.setView('history'); return; }
  if (ctrl && k === 'j' && app.tab && app.ask) { e.preventDefault(); openAsk(app, app.ask); return; }
  if (e.altKey && /^[1-9]$/.test(e.key)) {
    // As the View menu numbers them.
    const k = Object.entries(VIEWS).filter(([, x]) => !x.hidden && (!x.when || (app.view && x.when(app.view))))[Number(e.key) - 1]?.[0];
    if (k) { e.preventDefault(); app.setView(k); }
    return;
  }
  if (typing) return;
  if (ctrl && k === 'z') { e.preventDefault(); app.edit(e.shiftKey ? 'redo' : 'undo'); return; }
  if (ctrl && k === 'y') { e.preventDefault(); app.edit('redo'); return; }
  if (sheetOpen() || ctrl || e.altKey) return;
  if (k === 'c' || e.key === 'Insert') { e.preventDefault(); openCreate(app); return; }
  if (e.key === '/') { e.preventDefault(); searchIssues(); return; }
  const sel = app.tab?.selected;
  if (!sel || !app.view?.tasks.some((t) => t.id === sel)) return;
  if (e.key === 'Delete') { e.preventDefault(); deleteSelected(); }
  else if (e.key === 'Enter' && !(e.target instanceof HTMLButtonElement)) { e.preventDefault(); app.openTask(sel); }
});

/// Instead of Create when my access is limited: what I may do here.
function accessChip(v) {
  const names = { comment: 'comment', create: 'create issues', edit: 'edit issues', status: 'move issues', assign: 'assign', schedule: 'change estimates', delete: 'delete', setup: 'change the set-up' };
  const can = (v.may || []).map((p) => names[p.replace('plan.', '')]).filter(Boolean);
  return h('span', { class: 'access-chip', title: can.length ? `You can ${can.join(', ')}` : 'You can read this plan, not change it' }, can.length ? (can.length === 1 && can[0] === 'comment' ? 'Can comment' : 'Limited access') : 'View only');
}

// ----- Start -----

// In the browser on a Xenify server: File › Quit goes back to its Files page.
let servedHere = false;
function showHome() {
  if (!/^https?:$/.test(location.protocol)) return;
  fetch('/v1/info').then((r) => {
    servedHere = r.ok && /json/.test(r.headers.get('content-type') || '');
    if (servedHere) buildMenus();
  }).catch(() => {});
}

async function main() {
  setUpWindow();
  document.getElementById('new-tab').innerHTML = icon('plus', 16);
  document.getElementById('new-tab').addEventListener('click', () => newPlan());
  buildMenus();
  await start();
  setUpServer(app, { newTabFor });
  showHome();
  // Opened from a Xenify server (the browser version): use that server, and
  // open #open=<server>,<plan> from its Files page. Shown once that's known,
  // so a signed-out visit goes to sign in without a glimpse of Plan.
  srv.connectHere().catch(() => false).then((yes) => {
    // Plans need the server: signing in is on its Files page, which comes back here.
    if (yes && !srv.session) return void location.replace(srv.signInHere);
    document.documentElement.classList.add('ready');
    if (yes) render();
    // ?open=<file> (this server's) or #open=<server>,<file>.
    const here = new URLSearchParams(location.search).get('open');
    const linked = /^\d+$/.test(here || '') ? [null, location.origin, here] : /^#open=([^,]+),(\d+)$/.exec(location.hash);
    if (linked) {
      history.replaceState(null, '', location.pathname + location.search);
      // &issue=<key> (Copy link on an issue): that issue, opened in the plan.
      const issue = new URLSearchParams(location.search).get('issue');
      openLink({ server: decodeURIComponent(linked[1]), node: +linked[2] }).then(() => {
        if (!issue || !app.tab) return;
        const t = call('find', { query: issue }).find((x) => x.key === issue);
        if (t) app.openTask(t.id);
        else toast(`${issue} isn’t in this plan any more.`);
      }).catch(() => {});
    }
  }).catch(() => {});
  app.newLocalPlan = newLocalPlan;
  app.saveToServer = () => save();
  app.ask = new Ask(app);
  app.mcp = new Mcp(app);
  app.mcp.boot().catch(() => {});
  app.ask.refresh().then(buildMenus).catch(() => {});
  const files = await platform.openedFiles();
  for (const f of files) {
    // xenify-plan://open?server=…&node=… (the server's Files page, through Xenify Hub).
    if (/^xenify-plan:/i.test(f)) {
      try {
        const u = new URL(f);
        await openLink({ server: u.searchParams.get('server'), node: Number(u.searchParams.get('node')) });
      } catch { toast('That link to a plan on a server is broken.'); }
    } else await openPath(f);
  }
  if (!app.tabs.length) render();
  if (!window.XENIFY_TEST) await offerRecovery();
  // A new day: dates move on.
  setInterval(() => { const t = engineTabs.front(); for (const x of app.tabs) { engineTabs.switch(x.id); clock(); } if (t) engineTabs.switch(t); render(); }, 3600 * 1000);
  if (platform.DESKTOP) {
    const w = window.__TAURI__.window.getCurrentWindow();
    w.onCloseRequested(async (e) => { e.preventDefault(); await quit(); });
  }
  if (window.XENIFY_TEST) {
    try { await window.XENIFY_TEST(); } catch (e) { platform.log(`test: ${e.stack || e}`); }
  }
}

main().catch((e) => { document.documentElement.classList.add('ready'); console.error(e.stack || e); });
