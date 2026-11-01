// Xenify Calc's screens: books in tabs, the menus and toolbar, the formula
// bar, the grid, sheet tabs and the status bar. Saving works as in Write and
// Draw: Save writes the file; unsaved work is only kept for recovery.

import { start, call, live, tabs as engineTabs, saveFile, recoveryCopy, bookKind } from './engine.js';
import { Grid, cellName, colName, parseCell } from './grid.js';
import { h, icon, menu, item, modal, ask, toast, popup, closePopups, colourPicker, browserCloseKey, dialogKeys } from './ui.js';
import * as platform from './platform.js';
import { setUpWindow } from './window.js';
import { Lookup } from './lookup.js';
import { AllSheets } from './allsheets.js';
import { Charts } from './charts.js';
import { printDialog } from './print.js';
import { Summarise } from './summarise.js';
import { ChartPanel } from './chartpanel.js';
import { FormulaHelp } from './formulahelp.js';
import { FormulaAI } from './formulaai.js';
import { Spelling } from './spelling.js';
import { Pictures } from './pictures.js';
import { Reports } from './reports.js';
import { Databases } from './database.js';
import { Editors } from './editors.js';
import { CsvLinks, csvDialog, SEPARATORS } from './csvlink.js';
import { Ask, aiTools } from './ask.js';
import { aboutColumn, definitions } from './notes.js';
import { Mcp } from './mcp.js';
import { Server } from './server.js';
import { ui as serverUi, serverRow, openFromServer, openLinked, saveToServer, shareDialog, historyDialog, checkpointDialog, offerUnsent } from './serverui.js';

const $ = (id) => document.getElementById(id);
const MAX_ROW = 1048576;
const MAX_COL = 16384;

const app = {
  tabs: [],      // { id, title, path, handle, sheet }
  grid: null,
  clip: null,    // what Copy kept: { clip, cut, range, sheet, tab }
  state: {},
  find: null,
};
window.calc = app;

const front = () => app.tabs.find((t) => t.id === engineTabs.front());
const sheet = () => front()?.sheet;

/// Runs an engine call that changes the book, then shows the result.
function run(name, args = {}) {
  try {
    const out = call(name, args);
    changed();
    return out;
  } catch (e) {
    toast(String(e.message || e));
    return undefined;
  }
}

function changed() {
  app.spelling?.soon();
  app.all?.render();
  app.reports?.render();
  app.db?.sync();
  app.editors?.sync();
  app.csvLinks?.sync();
  app.grid.refresh();
  app.charts?.refresh();
  app.pictures?.refresh();
  app.grid.changed();
  showState();
  showSheets();
  selectionChanged();
  keep();
}

// ----- Books (tabs) -----

function newTab(title, open, path = null, handle = null) {
  const id = open();
  const sheets = call('sheets');
  const t = { id, title, path, handle, sheet: (sheets.find((s) => !s.working) || sheets[0]).id };
  app.tabs.push(t);
  switchTab(id);
  return t;
}

/// The front tab is a new book nothing was typed in: the next one opened replaces it.
function untouched(t) {
  return !!t && !t.path && !t.handle && !app.state.unsaved && !app.state.can_undo && call('sheets').length === 1;
}

/// Opens a file's bytes (`name` with its ending says what kind of file it is).
function openBytes(name, bytes, path = null, handle = null) {
  // Text files: how to read them is asked first.
  if (/\.(csv|tsv|txt|psv|tab)$/i.test(name)) return importCsv(name, bytes, path, handle);
  const replace = untouched(front()) ? front() : null;
  // Calc reads .xls and .ods but saves an Excel file: Save asks where (the old file stays as it was).
  const readOnly = /\.(xls|ods)$/i.test(name);
  if (readOnly) { path = null; handle = null; }
  try {
    newTab(platform.baseName(name), () => engineTabs.openFile(bytes, name), path, handle);
  } catch (e) {
    toast(`Couldn’t open ${name}: ${e.message || e}`);
    return false;
  }
  if (replace) dropTab(replace.id);
  app.db?.sync();
  app.editors?.sync();
  app.csvLinks?.sync();
  app.db?.opened().then(() => app.editors?.opened()).then(() => app.csvLinks?.opened());
  const kept = call('about_file').kept;
  if (readOnly) toast('Opened the values and formulas (not formats). Save keeps it as an .xlsx file, beside the original.');
  else if (kept.length) toast(`Opened. Kept as they are (Calc doesn’t change them): ${kept.join(', ').toLowerCase()}`);
  return true;
}

/// Opening CSV or other separated text: how to read it is asked first
/// (csvlink.js), and on the desktop whether the sheet stays linked to the file.
async function importCsv(name, bytes, path, handle) {
  const text = new TextDecoder().decode(bytes);
  const how = await csvDialog(call, name, text, { linkable: platform.DESKTOP && !!path });
  if (!how) return false;
  // Linked: a new Calc book whose sheet follows the file (Save asks where to keep the book).
  const link = how.link ? { path, name: platform.fileName(name), sep: how.sep, values: how.values } : null;
  if (link) { path = null; handle = null; }
  const replace = untouched(front()) ? front() : null;
  const t = newTab(platform.baseName(name), () => engineTabs.open(), path, handle);
  try {
    call('import_csv', { text, name: platform.baseName(name), sep: how.sep || undefined, values: how.values, link, read: new Date().toISOString() });
  } catch (e) { toast(`Couldn’t open ${name}: ${e.message || e}`); dropTab(t.id); return false; }
  t.sheet = call('sheets')[0].id;
  app.grid.setSheet(t.sheet);
  if (replace) dropTab(replace.id);
  changed();
  if (link) app.csvLinks.noteRead(t.sheet);
  return true;
}

function dropTab(id) {
  app.tabs = app.tabs.filter((t) => t.id !== id);
  engineTabs.close(id);
  platform.autosave.clear(keyOf({ id })).catch(() => {});
  showTabs();
}

function switchTab(id) {
  app.grid.endEdit(true);
  if (app.summarise?.on) app.summarise.stop();
  app.chartPanel?.stop();
  app.all?.hide();
  if (app.lookup?.on) app.lookup.stop();
  engineTabs.switch(id);
  app.spelling?.soon(0);
  const t = front();
  if (!call('sheets').some((s) => s.id === t.sheet)) t.sheet = call('sheets')[0].id;
  app.grid.setSheet(t.sheet);
  app.charts?.refresh();
  app.pictures?.refresh();
  app.reports?.sync();
  app.db?.sync();
  app.editors?.sync();
  app.csvLinks?.sync();
  showTabs();
  showState();
  showSheets();
  document.title = `${t.title} — Xenify Calc`;
}

function showTabs() {
  const box = $('tabs');
  // On a server's site one file is open at a time, like a page (File › New adds tabs).
  document.querySelector('.tabstrip').hidden = servedHere && app.tabs.length <= 1;
  box.replaceChildren(...app.tabs.map((t) => {
    const on = t.id === engineTabs.front();
    const dirty = on ? app.state.unsaved : t.dirty;
    return h('div', { class: 'tab', role: 'tab', 'aria-selected': String(on), title: t.path || t.title, onClick: () => switchTab(t.id),
      onAuxclick: (e) => { if (e.button === 1) closeTab(t.id); } },
      h('span', { class: 'book', html: icon('book', 16) }),
      h('span', { class: 'name' }, t.title),
      server.nodeOf(t.id) != null ? h('span', { class: 'on-server', title: `On ${server.name}${server.here(t.id).length ? ', editing together' : ''}`, html: icon('database', 12) }) : null,
      dirty ? h('span', { class: 'dot', title: 'Not saved yet', 'aria-label': 'not saved yet' }) : null,
      h('button', { class: 'close', 'aria-label': `Close ${t.title}`, html: icon('close', 13), onClick: (e) => { e.stopPropagation(); closeTab(t.id); } }));
  }));
}

function newBook() {
  newTab('Untitled', () => engineTabs.open());
}

async function openBook() {
  let got;
  try { got = await platform.openBook(); } catch (e) { toast(String(e.message || e)); return; }
  if (got) openBytes(got.name, got.bytes, got.path, got.handle);
}

function unsavedPrompt(t) {
  return modal(`Save the changes to “${t.title}”?`, h('p', {}, 'If you don’t, the changes since it was last saved are lost.'),
    [['Cancel', null], ['Don’t save', 'discard'], ['Save', 'save', true]]);
}

function isDirty(t) {
  return t.id === engineTabs.front() ? !!app.state.unsaved : !!t.dirty;
}

async function closeTab(id) {
  const t = app.tabs.find((x) => x.id === id);
  if (!t) return true;
  if (isDirty(t)) {
    if (engineTabs.front() !== id) switchTab(id);
    const what = await unsavedPrompt(t);
    if (!what || (what === 'save' && !(await save()))) return false;
  }
  server.leave(id);
  dropTab(id);
  // The last one closed on a server's site: back to Files.
  if (!app.tabs.length && servedHere) { await quit(); return true; }
  if (!app.tabs.length) newBook();
  else switchTab(app.tabs[app.tabs.length - 1].id);
  return true;
}

const KIND_NAMES = { xcalc: 'Calc book (.xcalc)', xlsx: 'Spreadsheet (.xlsx)', xlsm: 'Spreadsheet with macros (.xlsm)', csv: 'CSV, this sheet (.csv)', tsv: 'Tab-separated, this sheet (.tsv)' };

/// Save as in the browser: which kind of file.
function askKind(current) {
  const choices = Object.entries(KIND_NAMES).filter(([k]) => k !== 'xlsm' || call('about_file').macros);
  return modal('Save as', h('p', {}, 'Which kind of file?'), [['Cancel', null], ...choices.map(([k, label]) => [label, k, k === current])]);
}

/// Before writing a CSV: what it can't hold. Whether to go on.
async function csvOk(t) {
  const a = call('about_file');
  const lost = [];
  if (a.sheets > 1) lost.push('the other sheets');
  if (a.formatted) lost.push('formats');
  if (a.tables) lost.push('tables and their totals');
  if (a.charts) lost.push('charts');
  lost.push('formulas (their results are written)');
  if (t.csvWarned) return true;
  const ok = await modal('Save this sheet as text?', h('p', {}, `A CSV or tab-separated file keeps only the values on this sheet. Not kept: ${lost.join(', ')}.`),
    [['Cancel', false], ['Save the sheet', true, true]]);
  if (ok) t.csvWarned = true;
  return ok;
}

/// Save as CSV: which character separates the values (the file's own, at first).
async function csvSeparator() {
  const now = call('csv_sep', {});
  const sel = h('select', { class: 'field', 'aria-label': 'Separated by' }, SEPARATORS.filter(([c]) => c !== '\t').map(([c, l]) => h('option', { value: c, selected: c === now }, l)));
  const ok = await modal('Save as CSV', h('label', { class: 'field-group' }, 'Separated by', sel, h('p', { class: 'hint' }, 'Numbers are written in full (no rounding); dates as they show.')), [['Cancel', false], ['Save', true, true]]);
  if (ok) call('csv_sep', { sep: sel.value });
  return ok;
}

/// Save (or Save as): writes the file, in the kind of file the book came
/// from (or the kind picked). Whether it's saved now.
async function save(as = false) {
  const t = front();
  if (!t) return false;
  app.grid.endEdit(true);
  if (!as && server.nodeOf(t.id) != null) {
    if (app.editors) await app.editors.send();
    // It saves itself as you type: Save sends what's waiting (no new
    // version each time; a checkpoint keeps a moment).
    try {
      await server.versionNow(t.id);
      toast(`Saved on ${server.name}: it saves itself as you type. File › Save a checkpoint… keeps it as it is now.`);
      showState();
      showTabs();
      return true;
    } catch (e) { toast(String(e.message || e)); return false; }
  }
  // On a server's site a new book is saved there (named now, where New was asked for).
  if (!as && servedHere && server.session) {
    const ok = !!(await saveToServer(t.title, { beforeSave: (name) => { t.title = name; } }));
    showTabs();
    document.title = `${t.title} — Xenify Calc`;
    return ok;
  }
  let kind = bookKind();
  let path = as ? null : t.path;
  let handle = as ? null : t.handle;
  const stem = t.title.replace(/[\\/:*?"<>|]/g, '-');
  if (as) {
    if (platform.DESKTOP) {
      path = await platform.chooseSave(kind, `${stem}.${kind}`);
      if (!path) return false;
      kind = platform.extensionOf(path);
    } else {
      kind = await askKind(kind);
      if (!kind) return false;
    }
  }
  if (kind === 'xlsx' && call('about_file').macros) {
    kind = 'xlsm';
    if (path) path = path.replace(/\.xlsx$/i, '.xlsm');
    toast('Saved with its macros as .xlsm (an .xlsx can’t hold them)');
  }
  const text = kind === 'csv' || kind === 'tsv';
  if (text && !(await csvOk(t))) return false;
  if (kind === 'csv' && as && !(await csvSeparator())) return false;
  // Table editor sheets: their changes go to the database first.
  if (!text && app.editors) await app.editors.send();
  // A Calc book carries what Write shows of it (Write documents can link to the file).
  if (kind === 'xcalc') {
    try { call('set_link', { tab: t.id, link: await linkSnapshot(t.id) }); } catch { call('set_link', { tab: t.id, link: null }); }
  }
  // Another book brought to the front while that was sent: this one is written.
  if (engineTabs.front() !== t.id) switchTab(t.id);
  let bytes;
  try { bytes = saveFile(kind, text ? sheet() : null); } catch (e) { toast(String(e.message || e)); return false; }
  const name = `${stem}.${kind}`;
  let where;
  try {
    where = await platform.saveBook({ name, bytes, path, handle, kind });
  } catch (e) {
    toast(`Couldn’t save: ${e.message || e}`);
    where = null;
  }
  if (!where) {
    // Not written after all: still unsaved.
    call('mark_unsaved');
    showState();
    return false;
  }
  const own = bookKind() === kind;
  if (own) {
    if (typeof where === 'string') { t.path = where; t.title = platform.baseName(where); }
    else if (where !== true) { t.handle = where; t.title = platform.baseName(where.name || name); }
    platform.autosave.clear(keyOf(t)).catch(() => {});
  }
  showState();
  showTabs();
  document.title = `${t.title} — Xenify Calc`;
  if (!own) toast(`Saved this sheet as ${name}. The book itself isn’t saved yet.`);
  else toast(where === true ? `Downloaded ${name}` : `Saved ${t.title}`);
  return true;
}

function printNow() {
  app.grid.endEdit(true);
  printDialog({
    call, sheet: sheet(), selection: app.grid.range(), sheetName: front()?.title || 'Sheet',
    // The charts on this sheet, as drawn.
    chartSvgs: () => [...document.querySelectorAll('.chart-layer .chart')].map((n) => {
      const svg = n.querySelector('svg');
      const title = n.querySelector('.chart-head b')?.textContent || '';
      return svg ? `<div style="font-weight:700;margin-bottom:4px">${title.replace(/&/g, '&amp;').replace(/</g, '&lt;')}</div>${svg.outerHTML}` : '';
    }).filter(Boolean),
  });
}

/// File › About this file: what kind of file it is and what Calc keeps unedited.
function aboutFile() {
  const a = call('about_file');
  const t = front();
  const body = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } },
    h('p', {}, `${KIND_NAMES[bookKind()] || bookKind()}${t?.path ? ` · ${t.path}` : ''}`),
    a.kept.length
      ? h('div', {}, h('p', {}, 'Kept exactly as they were (Calc shows or keeps these but doesn’t change them):'), h('ul', {}, a.kept.map((k) => h('li', {}, k))))
      : h('p', {}, 'Calc can change everything in this file.'),
    a.macros ? h('p', {}, 'Macros are kept in the file but never run here.') : null);
  modal('About this file', body, [['Close', true, true]]);
}

// ----- Keeping unsaved work for recovery (not saving) -----

const RUN = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
const keyOf = (t) => `${RUN}-${t.id}`;
let keepTimer = null;
/// Every 15 seconds after a change, the unsaved book is kept apart from its
/// file, offered back if Calc closes before it's saved.
function keep() {
  if (keepTimer) return;
  keepTimer = setTimeout(() => {
    keepTimer = null;
    const t = front();
    if (!t) return;
    if (app.state.unsaved) platform.autosave.write(keyOf(t), t.title, t.path, recoveryCopy()).catch(() => {});
    else platform.autosave.clear(keyOf(t)).catch(() => {});
  }, 15000);
}

async function offerRecovery() {
  const left = await platform.autosave.leftovers(RUN).catch(() => []);
  if (!left.length) return;
  const when = (s) => new Date(s * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const ok = await modal('Recover unsaved books?', h('div', { style: 'display: flex; flex-direction: column; gap: 8px' },
    h('p', {}, 'Calc closed last time before these were saved:'),
    h('ul', {}, left.map((l) => h('li', {}, `${l.name} (${when(l.when)})`)))), [['Discard them', false], ['Recover', true, true]]);
  for (const l of left) {
    if (ok) {
      try {
        if (openBytes(`${l.name}.xcalc`, platform.fromB64(l.base64), l.path || null)) {
          // Back to the kind of file it came from, and not in it yet.
          const ext = l.path ? platform.extensionOf(l.path) : '';
          if (['xlsx', 'xlsm', 'csv', 'tsv'].includes(ext)) call('set_format', { format: ext });
          call('mark_unsaved');
          showState();
        }
      } catch { /* unreadable */ }
    }
    platform.autosave.clear(l.key).catch(() => {});
  }
}

async function quit() {
  for (const t of [...app.tabs]) {
    if (!isDirty(t)) continue;
    switchTab(t.id);
    const what = await unsavedPrompt(t);
    if (!what || (what === 'save' && !(await save()))) return;
  }
  await Promise.all(app.tabs.map((t) => platform.autosave.clear(keyOf(t)).catch(() => {})));
  if (platform.DESKTOP) return platform.quit();
  // In the browser, from a Xenify server: back to its Files page.
  leaving = true;
  await server.quitToFiles();
}

// ----- Sheets -----

function showSheets() {
  const sheets = call('sheets');
  const box = $('sheettabs');
  const cur = sheet();
  const shown = sheets.filter((s) => !s.working || s.id === cur);
  box.replaceChildren(...shown.map((s) => h('button', {
    class: `stab${s.working ? ' working' : ''}`, role: 'tab', 'aria-selected': String(s.id === cur),
    onClick: () => goSheet(s.id), onDblclick: () => renameSheet(s.id),
    // While pointing in a formula the editor keeps the keyboard.
    onMousedown: (e) => { if (app.grid.editing && app.grid.pointing()) e.preventDefault(); },
    onContextmenu: (e) => { e.preventDefault(); sheetMenu(s, e.clientX, e.clientY); },
  }, s.name, s.working ? h('span', { class: 'tag' }, s.kind === 'normal' ? 'working' : `${kindName(s.kind)} · locked`) : null)));
  const working = sheets.filter((s) => s.working).length;
  $('all-sheets').setAttribute('aria-pressed', String(!!app.all?.open));
  $('all-sheets').replaceChildren(...[h('span', { html: icon('grid', 15) }), 'All sheets', working ? h('span', { class: 'tag' }, `${working} working`) : null].filter(Boolean));
}

function kindName(k) {
  return { normal: 'Sheet', query: 'Query', table_editor: 'Table editor', summary: 'Summary', report: 'Report', linked_csv: 'Linked file' }[k] || k;
}

function goSheet(id) {
  const t = front();
  app.all?.hide();
  if (t.sheet === id) return;
  t.sheet = id;
  app.chartPanel?.stop();
  app.spelling?.soon(0);
  // A formula being typed stays open: clicks on this sheet put references to it in.
  app.grid.setSheet(id, !!app.grid.editing && app.grid.pointing());
  app.charts?.refresh();
  app.pictures?.refresh();
  app.reports?.sync();
  app.db?.sync();
  app.editors?.sync();
  app.csvLinks?.sync();
  showSheets();
  if (app.lookup?.on) app.lookup.sheetShown(id);
}

const sheetName = (id) => call('sheets').find((s) => s.id === id)?.name || '';

function addSheet() {
  const id = run('add_sheet', {});
  if (id != null) goSheet(id);
}

async function renameSheet(id) {
  const s = call('sheets').find((x) => x.id === id);
  const name = await ask('Rename sheet', 'Name', s.name, 'Rename');
  if (name && name !== s.name) run('rename_sheet', { sheet: id, name });
}

function sheetMenu(s, x, y) {
  const sheets = call('sheets');
  const at = sheets.findIndex((q) => q.id === s.id);
  menu([
    item('Rename…', { run: () => renameSheet(s.id) }),
    item('Duplicate', { run: () => { const id = run('duplicate_sheet', { sheet: s.id }); if (id != null) goSheet(id); } }),
    null,
    item('Move left', { enabled: at > 0, run: () => run('move_sheet', { sheet: s.id, to: at - 1 }) }),
    item('Move right', { enabled: at < sheets.length - 1, run: () => run('move_sheet', { sheet: s.id, to: at + 1 }) }),
    null,
    s.working
      ? item('Show on the tab strip', { run: () => run('working', { sheet: s.id, working: false }) })
      : item('Make it a working sheet', { enabled: sheets.filter((q) => !q.working).length > 1, run: () => { run('working', { sheet: s.id, working: true }); goSheet(sheets.find((q) => !q.working && q.id !== s.id).id); } }),
    null,
    item('Delete…', { enabled: sheets.length > 1, run: () => deleteSheet(s) }),
  ], x, y);
}

async function deleteSheet(s) {
  const ok = await modal(`Delete “${s.name}”?`, h('p', {}, 'Formulas on other sheets that use it will show #REF!. Undo brings it back.'), [['Cancel', false], ['Delete', true, true]]);
  if (!ok) return;
  const rest = call('sheets').filter((q) => q.id !== s.id);
  if (sheet() === s.id) goSheet((rest.find((q) => !q.working) || rest[0]).id);
  run('remove_sheet', { sheet: s.id });
}

// ----- Selection: formula bar, toolbar, status -----

let lastSel = null;
function selectionChanged() {
  const g = app.grid;
  if (!g?.layout) return;
  app.fxai?.show();
  const a = g.active;
  const rg = g.range();
  lastSel = rg;
  const single = rg.top === rg.bottom && rg.left === rg.right;
  $('namebox').value = single ? cellName(a.r, a.c)
    : rg.top === 1 && rg.bottom === MAX_ROW ? `${colName(rg.left)}:${colName(rg.right)}`
      : rg.left === 1 && rg.right === MAX_COL ? `${rg.top}:${rg.bottom}`
        : `${cellName(rg.top, rg.left)}:${cellName(rg.bottom, rg.right)}`;
  if (!g.editing && document.activeElement !== $('formula')) {
    const c = call('cell', { sheet: sheet(), row: a.r, col: a.c });
    $('formula').value = c.input;
  }
  $('formula').readOnly = g.locked();
  showToolbarState();
  showStatus();
  if (single) app.editors?.cellSelected(a.r, a.c);
  sayWhere();
}

function activeLook() {
  const a = app.grid.active;
  return app.grid.cells.get(`${a.r},${a.c}`)?.look || {};
}

function showStatus() {
  const g = app.grid;
  const rg = g.range();
  const st = call('stats', { sheet: sheet(), range: rg });
  const fmt = (n) => (Math.abs(n) >= 1e15 || (Math.abs(n) < 1e-4 && n !== 0) ? n.toExponential(3) : n.toLocaleString(undefined, { maximumFractionDigits: 2 }));
  const bits = [];
  const t = g.tableAt(g.active.r, g.active.c);
  if (g.layout?.kind === 'summary') bits.push(h('button', { class: 'sum', onClick: () => app.summarise.start() }, 'Change this summary'));
  bits.push(h('span', {}, g.locked() ? 'This sheet shows what its source gives: it can’t be typed in'
    : t ? (t.totals ? `Table ${t.name}: totals grow as rows are added` : `Table ${t.name}`) : ''));
  if (st && st.numbers > 1) {
    bits.push(h('button', { class: 'sum', title: 'More about these numbers', 'aria-haspopup': 'dialog', onClick: (e) => summaryCard(st, e.currentTarget) }, `Sum ${fmt(st.sum)} · Average ${fmt(st.average)} · Count ${st.count}`));
  } else if (st && st.count > 1) {
    bits.push(h('span', {}, `Count ${st.count}`));
  }
  $('status').replaceChildren(...bits.filter((b) => b.textContent));
}

const num = (n) => (Math.abs(n) >= 1e15 || (Math.abs(n) < 1e-4 && n !== 0) ? n.toExponential(3) : n.toLocaleString(undefined, { maximumFractionDigits: 2 }));

/// Large numbers without their fractions (for ranges and spreads).
const round = (n) => (Math.abs(n) >= 100 ? Math.round(n).toLocaleString() : num(n));

/// What the selected numbers are called: their column's header, when there is one.
function selectionTitle(rg) {
  const t = app.grid.tableAt(rg.top, rg.left);
  if (t && rg.left === rg.right) return t.columns[rg.left - t.left].name;
  if (rg.left === rg.right && rg.top > 1) {
    const above = call('cell', { sheet: sheet(), row: rg.top - 1, col: rg.left });
    if (above.input && !/^[=\d.-]/.test(above.input)) return above.input;
  }
  return 'Selected numbers';
}

/// The number summary: the middle value, the spread and a histogram, to copy or put on the sheet.
function summaryCard(st, anchor) {
  const rg = app.grid.range();
  const where = $('namebox').value;
  const stats = [
    ['Total', st.sum], ['Average', st.average], ['Middle value', st.median], ['Typical spread', round(st.sd)],
    ['Smallest', st.min], ['Largest', st.max], ['Middle half', `${round(st.q1)} – ${round(st.q3)}`],
  ];
  const W = 396; const H = 64; const base = 46;
  const counts = st.bins.counts;
  const most = Math.max(...counts, 1);
  const bw = W / counts.length;
  const bars = counts.map((n, k) => {
    const hgt = Math.max(n ? 2 : 0, (n / most) * (base - 6));
    const x = k * bw + 1; const w = Math.max(2, bw - 2);
    const from = st.bins.start + k * st.bins.step;
    const r = Math.min(4, w / 2, hgt);
    const y = base - hgt;
    const d = hgt ? `M${x},${base}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${base}Z` : '';
    return `<path d="${d}" fill="var(--accent)"><title>${num(from)} to ${num(from + st.bins.step)}: ${n} ${n === 1 ? 'number' : 'numbers'}</title></path>`;
  }).join('');
  const end = st.bins.start + counts.length * st.bins.step;
  const svg = `<svg role="img" aria-label="How the numbers spread, from ${num(st.bins.start)} to ${num(end)}" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">${bars}<line x1="0" x2="${W}" y1="${base}" y2="${base}" stroke="var(--line-2)"/><text x="0" y="${H - 2}" font-size="10.5" fill="var(--muted)" font-family="Figtree">${num(st.bins.start)}</text><text x="${W}" y="${H - 2}" text-anchor="end" font-size="10.5" fill="var(--muted)" font-family="Figtree">${num(end)}</text></svg>`;
  const copyIt = async () => {
    const text = [...stats, ['Count', st.numbers]].map(([k, v]) => `${k}\t${v}`).join('\n');
    try { await navigator.clipboard.writeText(text); toast('The summary is copied'); } catch { toast('Couldn’t reach the clipboard'); }
    closePopups();
  };
  const body = h('div', { class: 'statcard' },
    h('div', { class: 'head' }, h('b', {}, selectionTitle(rg)), h('span', {}, `${where} · ${st.numbers} numbers`)),
    h('div', { class: 'stats' }, stats.map(([k, v]) => h('div', { class: k === 'Middle half' ? 'two' : null }, h('span', { class: 'k' }, k), h('span', { class: 'v' }, typeof v === 'number' ? num(v) : v)))),
    h('span', { class: 'k' }, 'How the numbers spread'),
    h('div', { html: svg }),
    h('div', { class: 'row-end' },
      h('button', { class: 'btn small', onClick: copyIt }, 'Copy'),
      h('button', { class: 'btn small main', onClick: () => { closePopups(); insertSummary(rg); } }, 'Insert summary')));
  const r = anchor.getBoundingClientRect();
  popup(body, r.left, r.top - 330);
}

/// Puts the summary on the sheet as formulas beside the selection (they keep up as the numbers change).
function insertSummary(rg) {
  const t = app.grid.tableAt(rg.top, rg.left);
  const ref = t && rg.left === rg.right ? `${t.name}[${t.columns[rg.left - t.left].name}]` : `${cellName(rg.top, rg.left)}:${cellName(rg.bottom, rg.right)}`;
  const right = t ? t.right : rg.right;
  // Two columns to the right of what's there, from the top of the selection.
  let col = right + 2;
  const top = t ? t.top : rg.top;
  const rows = [['Total', `=SUM(${ref})`], ['Average', `=AVERAGE(${ref})`], ['Middle value', `=MEDIAN(${ref})`], ['Smallest', `=MIN(${ref})`], ['Largest', `=MAX(${ref})`], ['Count', `=COUNT(${ref})`]];
  const free = (c) => rows.every((_, k) => call('cell', { sheet: sheet(), row: top + 1 + k, col: c }).input === '' && call('cell', { sheet: sheet(), row: top + 1 + k, col: c + 1 }).input === '');
  while (!free(col) && col < right + 40) col++;
  const cells = [{ row: top, col, data: { s: selectionTitle(rg) } }];
  rows.forEach(([k, f], i) => {
    cells.push({ row: top + 1 + i, col, data: { s: k } });
    cells.push({ row: top + 1 + i, col: col + 1, data: { f } });
  });
  run('edit', { ops: [{ op: 'cells', sheet: sheet(), cells }] });
  const look = activeLook();
  if (look.fmt) run('format', { sheet: sheet(), range: { top: top + 1, left: col + 1, bottom: top + 5, right: col + 1 }, set: { num_fmt: look.fmt } });
  run('format', { sheet: sheet(), range: { top, left: col, bottom: top, right: col }, set: { bold: true } });
  app.grid.selectRange({ top, left: col, bottom: top + rows.length, right: col + 1 });
}

function showState() {
  try { app.state = call('state'); } catch { app.state = {}; }
  const t = front();
  if (t) t.dirty = app.state.unsaved;
  for (const b of document.querySelectorAll('[data-needs]')) {
    b.disabled = !app.state[b.dataset.needs];
  }
  showTabs();
  if (app.grid?.layout) showStatus();
}

// ----- Editing cells -----

function typeIn(r, c, value) {
  try {
    call('type', { sheet: sheet(), row: r, col: c, input: value });
  } catch (e) {
    toast(String(e.message || e));
  }
  changed();
}

function addRow(table, col, value) {
  if (value === '') return;
  const column = table.columns[col - table.left]?.name || table.columns[0].name;
  const newRow = table.bottom - (table.totals ? 1 : 0) + 1;
  const ok = run('add_row', { table: table.name, values: { [column]: value } });
  if (ok === undefined) return;
  // On along the new row, so the rest of it can be typed.
  app.grid.select(newRow, Math.min(col + 1, table.right));
}

function clearCells() {
  run('clear', { sheet: sheet(), range: app.grid.range(), what: 'contents' });
}

function format(set) {
  run('format', { sheet: sheet(), range: app.grid.range(), set });
}

// ----- Clipboard -----

function copy(cut = false) {
  const rg = app.grid.range();
  let clip;
  try { clip = call('copy', { sheet: sheet(), range: rg }); } catch (e) { toast(String(e.message || e)); return null; }
  app.clip = { clip, cut, range: rg, sheet: sheet(), tab: front().id };
  app.objClip = null;
  app.textClip = null;
  app.grid.copied = { sheet: sheet(), range: rg };
  app.grid.draw();
  return clip.text;
}

function paste(text, values = false) {
  const a = app.grid.active;
  // Something Calc copied as text (a summary) when the system clipboard couldn't take it.
  if (!text && app.textClip) text = app.textClip;
  const mine = app.clip && (text == null || text === app.clip.clip.text);
  if (mine) {
    const c = app.clip;
    if (c.tab !== front().id) { pasteText(c.clip.text); return; }
    const rows = c.range.bottom - c.range.top; const cols = c.range.right - c.range.left;
    // Cut: the cells left behind are emptied (unless pasted over), in the same step of undo.
    const ops = [];
    if (c.cut) {
      const cells = [];
      for (let r = c.range.top; r <= c.range.top + rows; r++) {
        for (let cc = c.range.left; cc <= c.range.left + cols; cc++) {
          const inside = c.sheet === sheet() && r >= a.r && r <= a.r + rows && cc >= a.c && cc <= a.c + cols;
          if (!inside) cells.push({ row: r, col: cc, data: null });
        }
      }
      if (cells.length) ops.push({ op: 'cells', sheet: c.sheet, cells });
    }
    run('paste', { sheet: sheet(), row: a.r, col: a.c, clip: c.clip, values, then: ops });
    if (c.cut) { app.clip = null; app.grid.copied = null; }
    app.grid.selectRange({ top: a.r, left: a.c, bottom: a.r + rows, right: a.c + cols });
    return;
  }
  if (text != null) pasteText(text);
}

function pasteText(text) {
  const a = app.grid.active;
  run('paste', { sheet: sheet(), row: a.r, col: a.c, text });
  const lines = text.replace(/\n$/, '').split('\n');
  const width = Math.max(...lines.map((l) => l.split('\t').length));
  app.grid.selectRange({ top: a.r, left: a.c, bottom: a.r + lines.length - 1, right: a.c + width - 1 });
}

async function pasteFromMenu(values = false) {
  let text = null;
  try { text = await navigator.clipboard.readText(); } catch { /* not allowed: Calc's own copy */ }
  paste(text, values);
}

// ----- Tables, totals, sorting and filters -----

/// The table the selection is in, or a new one made from the cells selected
/// (or, for one cell, the block around it). Nothing moves: cells without a
/// header row make a table without one.
function tableHere() {
  const g = app.grid;
  const t = g.tableAt(g.active.r, g.active.c);
  if (t) return t;
  const rg = g.range();
  const many = rg.bottom > rg.top || rg.right > rg.left;
  const made = run('make_table', { sheet: sheet(), row: g.active.r, col: g.active.c, ...(many ? { range: rg } : {}) });
  if (made) toast(made.header ? `Made the table ${made.name}: its first row names the columns` : `Made the table ${made.name}`);
  return made || null;
}

const TOTALS = [['sum', 'Total'], ['average', 'Average'], ['count', 'Count'], ['min', 'Smallest'], ['max', 'Largest'], ['median', 'Middle value']];

/// Σ: a totals row under the selected columns of numbers (the table is made
/// first if needed, in the same step of undo).
function totalFor(kind) {
  const g = app.grid;
  const rg = g.range();
  const many = rg.bottom > rg.top || rg.right > rg.left;
  const got = run('totals_here', { sheet: sheet(), row: g.active.r, col: g.active.c, total: kind, ...(many ? { range: rg } : {}) });
  if (!got) return;
  const label = TOTALS.find((x) => x[0] === kind)?.[1] || 'Total';
  toast(`${label} of ${got.columns.join(', ')} under the table: it grows as rows are added`);
}

function totalsMenu(x, y) {
  menu(TOTALS.map(([k, label]) => item(label, { run: () => totalFor(k) })), x, y);
}

function totalsPicker(hit, x, y) {
  const t = hit.table;
  const now = t.columns[hit.c - t.left].total || null;
  menu([
    { head: `Under ${hit.column}` },
    ...TOTALS.map(([k, label]) => item(label, { checked: now === k ? true : null, run: () => run('total', { table: t.name, column: hit.column, total: k }) })),
    null,
    item(hit.c === t.left ? 'Row count' : 'Nothing', { checked: !now ? true : null, run: () => run('total', { table: t.name, column: hit.column, total: null }) }),
  ], x, y);
}

function sortBy(desc) {
  const g = app.grid;
  const rg = g.range();
  const t = g.tableAt(g.active.r, g.active.c);
  let area = rg;
  let header = false;
  if (!t && rg.top === rg.bottom && rg.left === rg.right) {
    // One cell: the block around it, keeping a header row on top.
    area = blockAround(g.active.r, g.active.c);
    header = true;
  }
  run('sort', { sheet: sheet(), range: area, by: [{ col: g.active.c, desc }], header });
}

/// The block of filled cells around a cell (what Ctrl+A picks).
function blockAround(r, c) {
  const down = call('edge', { sheet: sheet(), row: r, col: c, dr: 1, dc: 0 });
  const up = call('edge', { sheet: sheet(), row: r, col: c, dr: -1, dc: 0 });
  const right = call('edge', { sheet: sheet(), row: r, col: c, dr: 0, dc: 1 });
  const left = call('edge', { sheet: sheet(), row: r, col: c, dr: 0, dc: -1 });
  const filled = (rr, cc) => call('cell', { sheet: sheet(), row: rr, col: cc }).input !== '';
  return {
    top: filled(up.row, c) ? up.row : r, bottom: filled(down.row, c) ? down.row : r,
    left: filled(r, left.col) ? left.col : c, right: filled(r, right.col) ? right.col : c,
  };
}

function filterPicker(hit, x, y) {
  const t = hit.table;
  let got;
  try { got = call('column_values', { table: t.name, column: hit.column }); } catch (e) { toast(String(e.message || e)); return; }
  const keep = new Set(got.keep || got.values.map((v) => v.value));
  const search = h('input', { class: 'search', placeholder: 'Find a value', 'aria-label': 'Find a value' });
  const boxes = got.values.map((v) => {
    const cb = h('input', { type: 'checkbox', checked: keep.has(v.value) });
    return { v, cb, row: h('label', {}, cb, h('span', {}, v.value === '' ? '(empty)' : v.value), h('span', { class: 'n' }, String(v.rows))) };
  });
  const all = h('input', { type: 'checkbox', checked: boxes.every((b) => b.cb.checked) });
  all.addEventListener('change', () => { for (const b of boxes) if (b.row.style.display !== 'none') b.cb.checked = all.checked; });
  search.addEventListener('input', () => {
    const q = search.value.toLowerCase();
    for (const b of boxes) b.row.style.display = b.v.value.toLowerCase().includes(q) ? '' : 'none';
  });
  const apply = () => {
    const chosen = boxes.filter((b) => b.cb.checked).map((b) => b.v.value);
    closePopups();
    run('filter', { table: t.name, column: hit.column, keep: chosen.length === boxes.length ? null : chosen });
  };
  const sortCol = (desc) => {
    closePopups();
    run('sort', { sheet: sheet(), range: { top: t.top, left: t.left, bottom: t.bottom, right: t.right }, by: [{ col: hit.c, desc }] });
  };
  const body = h('div', { style: 'width: 250px' },
    h('button', { class: 'item', onClick: () => sortCol(false) }, 'Sort A to Z (smallest first)'),
    h('button', { class: 'item', onClick: () => sortCol(true) }, 'Sort Z to A (largest first)'),
    h('hr'), search,
    h('div', { class: 'checks' }, h('label', {}, all, h('b', {}, 'All')), boxes.map((b) => b.row)),
    h('div', { class: 'row-end' },
      h('button', { class: 'btn small', onClick: () => { closePopups(); run('filter', { table: t.name, column: hit.column, keep: null }); } }, 'Show all'),
      h('button', { class: 'btn small main', onClick: apply }, 'Apply')));
  popup(body, x, y);
  search.focus();
}

function filterHere() {
  const t = tableHere();
  if (!t) return;
  const g = app.grid;
  const c = Math.min(Math.max(g.active.c, t.left), t.right);
  const x = g.host.getBoundingClientRect().left + g.x(c) + g.cols.size(c) - 20;
  const y = g.host.getBoundingClientRect().top + g.y(t.top) + g.rows.size(t.top);
  filterPicker({ table: call('tables').find((q) => q.name === t.name), column: t.columns[c - t.left].name, c }, x, y);
}

// ----- Rows and columns -----

function insertLines(rows, after) {
  const rg = app.grid.range();
  const count = rows ? rg.bottom - rg.top + 1 : rg.right - rg.left + 1;
  const at = rows ? (after ? rg.bottom + 1 : rg.top) : (after ? rg.right + 1 : rg.left);
  run(rows ? 'insert_rows' : 'insert_columns', { sheet: sheet(), at, count: Math.min(count, 1000) });
}

function deleteLines(rows) {
  const rg = app.grid.range();
  const at = rows ? rg.top : rg.left;
  const count = rows ? rg.bottom - rg.top + 1 : rg.right - rg.left + 1;
  run(rows ? 'delete_rows' : 'delete_columns', { sheet: sheet(), at, count });
}

function resizeLines(rows, list, size) {
  run('sizes', { sheet: sheet(), rows, lines: list.map((at) => ({ at, size, hidden: false })) });
}

/// Double-clicking a column's edge: as wide as its widest text.
function autoWidth(c) {
  const used = app.grid.layout.used[0];
  const v = call('view', { sheet: sheet(), top: 1, left: c, bottom: Math.min(used, 5000), right: c });
  const ctx = app.grid.ctx;
  let w = 40;
  for (const cell of v.cells) {
    if (cell.c !== c) continue;
    ctx.font = app.grid.fontFor(cell.look);
    w = Math.max(w, ctx.measureText(cell.text).width + 18);
  }
  resizeLines(false, [c], Math.min(600, Math.ceil(w)));
}

async function columnWidth() {
  const rg = app.grid.range();
  const w = await ask('Column width', 'Width in pixels', String(Math.round(app.grid.cols.size(rg.left))), 'Set');
  const n = Number(w);
  if (w && n > 0) resizeLines(false, range(rg.left, Math.min(rg.right, rg.left + 500)), Math.min(2000, n));
}

function range(a, b) {
  const out = [];
  for (let i = a; i <= b; i++) out.push(i);
  return out;
}

/// Merge: one cell from the selection (the first value stays; asks when others would go).
async function mergeCells() {
  const rg = app.grid.range();
  let check;
  try { check = call('merge', { sheet: sheet(), range: rg, check: true }); } catch (e) { toast(String(e.message || e)); return; }
  if (check.lost > 0) {
    const ok = await modal('Merge these cells?', h('p', {}, `The merged cell keeps only the first value; the other ${check.lost === 1 ? 'one goes' : `${check.lost} go`}. Undo brings them back.`), [['Cancel', false], ['Merge', true, true]]);
    if (!ok) return;
  }
  run('merge', { sheet: sheet(), range: rg });
}

const commentTip = h('div', { class: 'chart-tip comment-tip', role: 'tooltip', hidden: true });
document.body.append(commentTip);
/// A cell's comment, beside the pointer.
function showComment(cm, x, y) {
  if (!cm) { commentTip.hidden = true; return; }
  commentTip.replaceChildren(...[cm.author ? h('b', {}, cm.author) : null, h('span', { style: { whiteSpace: 'pre-wrap' } }, cm.text)].filter(Boolean));
  commentTip.hidden = false;
  commentTip.style.left = `${Math.min(innerWidth - 340, x + 14)}px`;
  commentTip.style.top = `${y + 14}px`;
}

/// Insert › Report: a page to print or share. From a table (the cursor in
/// one), it starts with the totals of its number columns and the table.
function addReport(fromTable = false) {
  const g = app.grid;
  const t = fromTable ? g.tableAt(g.active.r, g.active.c) : null;
  const id = run('add_report', t ? { name: `${t.name} report`, table: t.name } : { name: 'Report' });
  if (id != null) {
    goSheet(id);
    toast('A report is a page made of blocks: use Add a block for titles, text, big numbers, tables and charts. They follow the book as it changes.');
  }
}

/// Sparklines for the numbers selected: one per row, in the column after them (or per column, under them).
function addSparklines(kind) {
  const rg = app.grid.range();
  const n = run('add_sparklines', { sheet: sheet(), range: rg, kind });
  if (n) toast(n === 1 ? 'Added a sparkline beside the numbers' : `Added ${n} sparklines beside the numbers`);
}

function freeze(rows, cols) {
  run('freeze', { sheet: sheet(), rows, columns: cols });
}

// ----- Find (at the bottom, as in Write) -----

function openFind(replace = false) {
  const bar = $('findbar');
  const q = h('input', { class: 'field', placeholder: 'Find', 'aria-label': 'Find' });
  const w = h('input', { class: 'field', placeholder: 'Replace with', 'aria-label': 'Replace with', hidden: !replace });
  const mc = h('input', { type: 'checkbox' });
  const whole = h('input', { type: 'checkbox' });
  const inF = h('input', { type: 'checkbox' });
  const count = h('span', { class: 'count', 'aria-live': 'polite' });
  let found = [];
  let at = -1;
  const opts = () => ({ match_case: mc.checked, whole: whole.checked, formulas: inF.checked });
  const search = () => {
    found = q.value ? call('find', { query: q.value, options: opts() }) : [];
    at = -1;
    count.textContent = q.value ? (found.length ? `${found.length} found` : 'Not found') : '';
  };
  const go = (step) => {
    if (!found.length) search();
    if (!found.length) return;
    at = (at + step + found.length) % found.length;
    const f = found[at];
    if (f.sheet !== sheet()) goSheet(f.sheet);
    app.grid.select(f.row, f.col);
    count.textContent = `${at + 1} of ${found.length}`;
  };
  const replaceAll = () => {
    const n = run('replace', { query: q.value, with: w.value, options: opts() });
    if (n !== undefined) toast(n === 1 ? 'Replaced 1 cell' : `Replaced ${n} cells`);
    search();
  };
  q.addEventListener('input', search);
  for (const c of [mc, whole, inF]) c.addEventListener('change', search);
  q.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); go(e.shiftKey ? -1 : 1); }
    if (e.key === 'Escape') { e.preventDefault(); closeFind(); }
  });
  w.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeFind(); });
  const showReplace = h('button', { class: 'btn small plain', onClick: () => { w.hidden = !w.hidden; rep.hidden = w.hidden; } }, 'Replace…');
  const rep = h('button', { class: 'btn small', hidden: !replace, onClick: replaceAll }, 'Replace all');
  bar.replaceChildren(
    h('span', { html: icon('search', 16), style: { color: 'var(--muted)' } }), q,
    h('button', { class: 'btn small', onClick: () => go(-1), 'aria-label': 'Previous' }, '↑'),
    h('button', { class: 'btn small', onClick: () => go(1), 'aria-label': 'Next' }, '↓'),
    count, w, rep, showReplace,
    h('label', {}, mc, 'Match case'), h('label', {}, whole, 'Whole cell'), h('label', {}, inF, 'In formulas'),
    h('button', { class: 'icon-btn small', 'aria-label': 'Close find', html: icon('close', 14), onClick: closeFind, style: { marginLeft: 'auto' } }));
  bar.hidden = false;
  app.grid.resize();
  q.focus();
}

function closeFind() {
  $('findbar').hidden = true;
  app.grid.resize();
  app.grid.scroller.focus();
}

// ----- Name box and formula bar -----

function goTo(text) {
  const s = text.trim();
  if (!s) return;
  const [a, b] = s.split(':');
  const p = parseCell(a);
  const q = b ? parseCell(b) : null;
  if (p && (!b || q)) {
    const rg = q ? { top: Math.min(p[0], q[0]), left: Math.min(p[1], q[1]), bottom: Math.max(p[0], q[0]), right: Math.max(p[1], q[1]) } : null;
    app.grid.select(p[0], p[1]);
    if (rg) app.grid.selectRange(rg);
    app.grid.scroller.focus();
    return;
  }
  // A table's name.
  const t = call('tables').find((x) => x.name.toLowerCase() === s.toLowerCase());
  if (t) {
    if (t.sheet !== sheet()) goSheet(t.sheet);
    app.grid.select(t.top, t.left);
    app.grid.selectRange({ top: t.top, left: t.left, bottom: t.bottom, right: t.right });
    app.grid.scroller.focus();
    return;
  }
  toast(`“${s}” isn’t a cell or a table`);
}

function bindFormulaBar() {
  const nb = $('namebox');
  nb.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); goTo(nb.value); }
    if (e.key === 'Escape') { e.preventDefault(); selectionChanged(); app.grid.scroller.focus(); }
  });
  nb.addEventListener('focus', () => nb.select());
  const f = $('formula');
  let original = '';
  f.addEventListener('focus', () => { original = f.value; });
  f.addEventListener('keydown', (e) => {
    const a = app.grid.active;
    if (e.key === 'Enter') { e.preventDefault(); if (f.value !== original) typeIn(a.r, a.c, f.value); app.grid.select(a.r + 1, a.c); app.grid.scroller.focus(); }
    if (e.key === 'Escape') { e.preventDefault(); f.value = original; app.grid.scroller.focus(); }
    if (e.key === 'Tab') { e.preventDefault(); if (f.value !== original) typeIn(a.r, a.c, f.value); app.grid.select(a.r, a.c + 1); app.grid.scroller.focus(); }
  });
}

// ----- Toolbar -----

const currency = (() => {
  const loc = Intl.NumberFormat().resolvedOptions().locale || 'en-US';
  return loc.endsWith('GB') ? '£' : /^(de|fr|es|it|nl|pt|fi|el)/.test(loc) || loc.endsWith('IE') ? '€' : loc.endsWith('JP') ? '¥' : loc.endsWith('IN') ? '₹' : '$';
})();
const dateFmt = (() => {
  const loc = Intl.DateTimeFormat().resolvedOptions().locale || 'en-US';
  return loc === 'en-US' ? 'mm/dd/yyyy' : /^(en|fr|es|it|pt|de|nl)/.test(loc) ? 'dd/mm/yyyy' : 'yyyy-mm-dd';
})();
const FORMATS = [
  ['general', 'Automatic'], ['#,##0.00', 'Number'], ['#,##0', 'Whole number'], [`${currency}#,##0.00`, 'Money'],
  ['0%', 'Percent'], ['0.00%', 'Percent (2 places)'], [dateFmt, 'Date'], ['hh:mm', 'Time'], [`${dateFmt} hh:mm`, 'Date and time'], ['@', 'Text'],
];
const FONTS = ['Figtree', 'Inter', 'Georgia', 'Courier New', 'Arial', 'Times New Roman'];
const SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48];

function buildToolbar() {
  const tb = $('toolbar');
  const btn = (label, html, onClick, extra = {}) => h('button', { class: 'icon-btn', 'aria-label': label, title: label, html, onClick, ...extra });
  const tool = (label, html, onClick, extra = {}) => h('button', { class: 'tool', title: extra.title || label, 'aria-label': label, onClick, ...extra }, h('span', { html }), h('span', { class: 'label' }, label));
  const fmt = h('select', { class: 'pick', 'aria-label': 'Number format', style: { width: '128px' }, onChange: (e) => format({ num_fmt: e.target.value }) },
    FORMATS.map(([v, l]) => h('option', { value: v }, l)));
  const font = h('select', { class: 'pick wide', 'aria-label': 'Font', style: { width: '120px' }, onChange: (e) => format({ font: e.target.value === 'Figtree' ? 'Inter' : e.target.value }) },
    FONTS.map((f) => h('option', { value: f }, f)));
  const size = h('select', { class: 'pick', 'aria-label': 'Size', style: { width: '60px' }, onChange: (e) => format({ size: Number(e.target.value) }) },
    SIZES.map((s) => h('option', { value: s }, String(s))));
  const colourBtn = (label, ico, key, bar) => {
    const b = btn(label, ico, (e) => { const r = b.getBoundingClientRect(); colourPicker(r.left, r.bottom + 4, (c) => { app.toolColours[key] = c || bar; b.querySelector('.bar').style.background = c || bar; format({ [key]: c }); }); }, { class: 'icon-btn colour-btn' });
    b.append(h('span', { class: 'bar', style: { background: bar } }));
    return b;
  };
  app.toolColours = { color: '#C2410C', fill: '#FACC15' };
  app.tools = {
    bold: btn('Bold (Ctrl+B)', '<b style="font-size: 15px">B</b>', () => format({ bold: !activeLook().b }), { 'aria-pressed': 'false' }),
    italic: btn('Italic (Ctrl+I)', '<i style="font: italic 600 15px Georgia, serif">I</i>', () => format({ italic: !activeLook().i }), { 'aria-pressed': 'false' }),
    underline: btn('Underline (Ctrl+U)', '<u style="font-size: 15px">U</u>', () => format({ underline: !activeLook().u }), { 'aria-pressed': 'false', class: 'icon-btn wide' }),
    fmt, font, size,
  };
  const borders = btn('Borders', icon('borders'), (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    menu([['all', 'All borders'], ['outside', 'Outside border'], ['bottom', 'Bottom border'], ['top', 'Top border'], ['left', 'Left border'], ['right', 'Right border'], null, ['none', 'No borders']]
      .map((x) => (x ? item(x[1], { run: () => format({ border: x[0] }) }) : null)), r.left, r.bottom + 4);
  });
  const align = btn('Align', icon('left'), (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    const now = activeLook().align;
    menu([
      item('Automatic (words left, numbers right)', { checked: !now ? true : null, run: () => format({ align: 'general' }) }),
      item('Left', { checked: now === 'left' ? true : null, run: () => format({ align: 'left' }) }),
      item('Centre', { checked: now === 'center' ? true : null, run: () => format({ align: 'center' }) }),
      item('Right', { checked: now === 'right' ? true : null, run: () => format({ align: 'right' }) }),
      null,
      item('Wrap text', { checked: activeLook().wrap ? true : null, run: () => format({ wrap: !activeLook().wrap }) }),
    ], r.left, r.bottom + 4);
  });
  app.tools.align = align;
  tb.replaceChildren(
    btn('Undo (Ctrl+Z)', icon('undo'), () => doUndo(), { 'data-needs': 'can_undo' }),
    btn('Redo (Ctrl+Y)', icon('redo'), () => doRedo(), { 'data-needs': 'can_redo' }),
    h('span', { class: 'sep' }),
    fmt, font, size,
    h('span', { class: 'sep' }),
    app.tools.bold, app.tools.italic, app.tools.underline,
    colourBtn('Text colour', icon('color'), 'color', '#C2410C'),
    colourBtn('Fill colour', icon('fill'), 'fill', '#FACC15'),
    borders, align,
    h('span', { class: 'sep' }),
    btn('Fewer decimal places', '<span style="font: 600 12px Figtree, sans-serif">.0</span>', () => format({ decimals: -1 })),
    btn('More decimal places', '<span style="font: 600 12px Figtree, sans-serif">.00</span>', () => format({ decimals: 1 })),
    btn('Percent', '<span style="font: 600 14px Figtree, sans-serif">%</span>', () => format({ num_fmt: '0%' })),
    h('span', { class: 'sep' }),
    tool('Sort', icon('sort'), (e) => { const r = e.currentTarget.getBoundingClientRect(); menu([item('A to Z (smallest first)', { run: () => sortBy(false) }), item('Z to A (largest first)', { run: () => sortBy(true) })], r.left, r.bottom + 4); }),
    tool('Filter', icon('filter'), () => filterHere()),
    h('span', { class: 'sep' }),
    tool('Look up', icon('search'), () => app.lookup.start(), { title: 'Fill a column from another table, by example' }),
    tool('Summarise', icon('summary'), () => app.summarise.start(), { title: 'One row for each value, with totals' }),
    tool('Totals', '<span class="sigma">Σ</span>', (e) => { const r = e.currentTarget.getBoundingClientRect(); totalsMenu(r.left, r.bottom + 4); }, { title: 'A totals row that grows with the table' }),
    tool('Table', icon('table'), () => tableHere(), { title: 'Make the selected cells a table: stripes, a filter on each column, totals that grow with it (Ctrl+T)' }),
    tool('Chart', icon('chart'), () => app.charts.add(), { title: 'A chart of the selected cells: pick its kind and what it shows' }),
  );
}

function showToolbarState() {
  const look = activeLook();
  const t = app.tools;
  if (!t) return;
  t.bold.setAttribute('aria-pressed', String(!!look.b));
  t.italic.setAttribute('aria-pressed', String(!!look.i));
  t.underline.setAttribute('aria-pressed', String(!!look.u));
  const f = look.fmt || 'general';
  if (![...t.fmt.options].some((o) => o.value === f)) t.fmt.append(h('option', { value: f }, f));
  t.fmt.value = f;
  t.font.value = look.font && FONTS.includes(look.font) ? look.font : 'Figtree';
  t.size.value = String(look.size || 12);
  t.align.innerHTML = icon(look.align === 'center' ? 'center' : look.align === 'right' ? 'right' : 'left');
}

// ----- Menus -----

function doUndo() {
  app.grid.endEdit(false);
  run('undo');
}

function doRedo() {
  app.grid.endEdit(false);
  run('redo');
}

function selectAll() {
  const g = app.grid;
  const rg = g.range();
  const block = blockAround(g.active.r, g.active.c);
  const same = rg.top === block.top && rg.left === block.left && rg.bottom === block.bottom && rg.right === block.right;
  if (same || (block.top === block.bottom && block.left === block.right)) g.selectRange({ top: 1, left: 1, bottom: MAX_ROW, right: MAX_COL });
  else g.selectRange(block);
}

/// In the browser on the website (not a Xenify server, not the desktop app):
/// File › Back to the website returns to the page that linked here, else
/// this app's page on the site. Your work stays kept in this browser.
function backToSite() {
  let same = false;
  try { const r = new URL(document.referrer); same = r.origin === location.origin && !r.pathname.startsWith('/calc/'); } catch { /* */ }
  if (same && history.length > 1) history.back();
  else location.href = '/products/calc/';
}

const MENUS = {
  File: () => [
    item('New book', { keys: 'Ctrl+N', run: newBook }),
    item('Open…', { keys: 'Ctrl+O', run: openBook }),
    null,
    item('Save', { keys: 'Ctrl+S', run: () => save() }),
    item('Save as…', { keys: 'Ctrl+Shift+S', run: () => save(true) }),
    item('About this file', { run: aboutFile }),
    null,
    item(server.session ? `Open from ${server.name}…` : 'Open from a server…', { run: () => (server.session ? openFromServer() : openServer(() => server.session && openFromServer())) }),
    item(server.session ? `Save to ${server.name}…` : 'Save to a server…', { run: () => (server.session ? saveToServer(front()?.title) : openServer(() => server.session && saveToServer(front()?.title))) }),
    server.nodeOf(engineTabs.front()) != null ? item('Share…', { run: () => shareDialog(engineTabs.front(), front().title) }) : undefined,
    server.nodeOf(engineTabs.front()) != null ? item('Save a checkpoint…', { run: () => checkpointDialog(engineTabs.front(), front().title) }) : undefined,
    server.nodeOf(engineTabs.front()) != null ? item('Version history…', { run: () => historyDialog(engineTabs.front(), front().title) }) : undefined,
    // Served by the server: the app already uses it (sign-in is on its Files page).
    server.servedHere ? undefined : item(server.session ? `Xenify server: ${server.name}…` : 'Xenify server…', { run: () => openServer() }),
    null,
    item('Print…', { keys: 'Ctrl+P', run: printNow }),
    null,
    item('Close book', { keys: 'Ctrl+W', run: () => closeTab(engineTabs.front()) }),
    platform.DESKTOP || servedHere ? item('Quit', { keys: 'Ctrl+Q', run: quit }) : item('Back to the website', { run: backToSite }),
  ],
  Edit: () => [
    item('Undo', { keys: 'Ctrl+Z', enabled: !!app.state.can_undo, run: doUndo }),
    item('Redo', { keys: 'Ctrl+Y', enabled: !!app.state.can_redo, run: doRedo }),
    null,
    item('Cut', { keys: 'Ctrl+X', run: () => { const t = copy(true); if (t != null) navigator.clipboard?.writeText(t).catch(() => {}); } }),
    item('Copy', { keys: 'Ctrl+C', run: () => { const t = copy(false); if (t != null) navigator.clipboard?.writeText(t).catch(() => {}); } }),
    item('Paste', { keys: 'Ctrl+V', run: () => pasteFromMenu(false) }),
    item('Paste values only', { keys: 'Ctrl+Shift+V', run: () => pasteFromMenu(true) }),
    null,
    item('Clear contents', { keys: 'Delete', run: clearCells }),
    item('Clear formats', { run: () => run('clear', { sheet: sheet(), range: app.grid.range(), what: 'formats' }) }),
    item('Clear all', { run: () => run('clear', { sheet: sheet(), range: app.grid.range(), what: 'all' }) }),
    null,
    item('Look up a word…', { keys: 'Shift+F7', run: lookUpWord }),
    null,
    item('Find…', { keys: 'Ctrl+F', run: () => openFind(false) }),
    item('Replace…', { keys: 'Ctrl+H', run: () => openFind(true) }),
    item('Select all', { keys: 'Ctrl+A', run: selectAll }),
  ],
  View: () => {
    const fr = app.grid.frozen || { r: 0, c: 0 };
    const a = app.grid.active;
    return [
      item('Freeze the top row', { checked: fr.r === 1 && fr.c === 0 ? true : null, run: () => freeze(1, 0) }),
      item('Freeze the first column', { checked: fr.r === 0 && fr.c === 1 ? true : null, run: () => freeze(0, 1) }),
      item(`Freeze above and left of ${cellName(a.r, a.c)}`, { enabled: a.r > 1 || a.c > 1, run: () => freeze(a.r - 1, a.c - 1) }),
      item('Unfreeze', { enabled: !!(fr.r || fr.c), run: () => freeze(0, 0) }),
      null,
      item('Check spelling and grammar', { checked: app.spelling?.on ? true : null, run: () => app.spelling.toggle() }),
      null,
      item('All sheets', { run: () => app.all.show() }),
      item('Next sheet', { keys: 'Ctrl+PgDn', run: () => cycleSheet(1) }),
      item('Previous sheet', { keys: 'Ctrl+PgUp', run: () => cycleSheet(-1) }),
    ];
  },
  Insert: () => [
    item('Rows above', { run: () => insertLines(true, false) }),
    item('Rows below', { run: () => insertLines(true, true) }),
    item('Columns to the left', { run: () => insertLines(false, false) }),
    item('Columns to the right', { run: () => insertLines(false, true) }),
    null,
    item('Sheet', { run: addSheet }),
    item('Table', { keys: 'Ctrl+T', run: () => tableHere() }),
    item('Totals row', { sub: () => TOTALS.map(([k, label]) => item(label, { run: () => totalFor(k) })) }),
    item('Chart', { run: () => app.charts.add() }),
    item('Picture…', { run: () => app.pictures.choose() }),
    item('Report (a page to print or share)', { run: () => addReport(false) }),
    ...(app.grid.tableAt(app.grid.active.r, app.grid.active.c) ? [item('Report from this table', { run: () => addReport(true) })] : []),
    item('Sparklines', { sub: () => [
      item('Line', { run: () => addSparklines('line') }),
      item('Columns', { run: () => addSparklines('column') }),
      item('Ups and downs', { run: () => addSparklines('win_loss') }),
      null,
      item('Take sparklines away', { run: () => run('remove_sparklines', { sheet: sheet(), range: app.grid.range() }) }),
    ] }),
  ],
  Format: () => {
    const look = activeLook();
    return [
      item('Bold', { keys: 'Ctrl+B', checked: look.b ? true : null, run: () => format({ bold: !look.b }) }),
      item('Italic', { keys: 'Ctrl+I', checked: look.i ? true : null, run: () => format({ italic: !look.i }) }),
      item('Underline', { keys: 'Ctrl+U', checked: look.u ? true : null, run: () => format({ underline: !look.u }) }),
      item('Strikethrough', { checked: look.s ? true : null, run: () => format({ strike: !look.s }) }),
      null,
      item('Number', { sub: () => FORMATS.map(([v, l]) => item(l, { checked: (look.fmt || 'general') === v ? true : null, run: () => format({ num_fmt: v }) })) }),
      item('Wrap text', { checked: look.wrap ? true : null, run: () => format({ wrap: !look.wrap }) }),
      item('Merge cells', { run: mergeCells }),
      item('Unmerge cells', { enabled: !!app.grid.mergeAt(app.grid.active.r, app.grid.active.c), run: () => run('unmerge', { sheet: sheet(), range: app.grid.range() }) }),
      null,
      item('Column width…', { run: columnWidth }),
      item('Fit column width', { run: () => autoWidth(app.grid.active.c) }),
      null,
      item('Clear formats', { run: () => run('clear', { sheet: sheet(), range: app.grid.range(), what: 'formats' }) }),
    ];
  },
  Data: () => [
    item('From a database…', { run: () => app.db.add() }),
    item('Refresh from databases', { enabled: call('queries').length > 0, run: () => app.db.refreshAll() }),
    // Files on the computer (desktop), or in your Files on a Xenify server.
    app.csvLinks?.possible ? item('Link a CSV file…', { run: () => app.csvLinks.add() }) : undefined,
    null,
    item('Summarise…', { run: () => app.summarise.start() }),
    item('Look up from another table…', { run: () => app.lookup.start() }),
    null,
    item('Sort A to Z', { run: () => sortBy(false) }),
    item('Sort Z to A', { run: () => sortBy(true) }),
    item('Filter', { run: filterHere }),
    null,
    item('Make a table', { keys: 'Ctrl+T', run: () => tableHere() }),
    item('Totals row', { sub: () => TOTALS.map(([k, label]) => item(label, { run: () => totalFor(k) })) }),
    null,
    item('Delete rows', { run: () => deleteLines(true) }),
    item('Delete columns', { run: () => deleteLines(false) }),
    null,
    item('About this column…', { run: () => aboutColumn(notesEnv(), app.grid.active.r, app.grid.active.c, colName) }),
    item('Definitions…', { run: () => definitions(notesEnv()) }),
  ],
  AI: () => [
    item('Ask…', { keys: 'Ctrl+J', run: () => app.ask.open() }),
    // On the desktop: tools to add, and AI programs that may use Calc (in the browser there's nothing to set).
    app.mcp?.desktop ? item('Tools for AI (MCP)…', { run: () => aiTools(app.ask) }) : undefined,
  ],
  Help: () => [
    item('Functions…', { run: () => app.help.open() }),
    item('Keyboard shortcuts', { run: shortcuts }),
    item('Licences', { run: licences }),
    updateReady ? item(`Restart to use version ${updateReady}`, { run: restartUpdated }) : undefined,
  ],
};

function buildMenus() {
  const box = $('menus');
  // Asked again once a server is found: the menus are there already (their items are read when opened).
  if (box.childElementCount) return;
  for (const name of Object.keys(MENUS)) {
    const b = h('button', { class: 'menu-btn', 'aria-haspopup': 'menu', 'aria-expanded': 'false' }, name);
    b.addEventListener('click', () => {
      const r = b.getBoundingClientRect();
      b.setAttribute('aria-expanded', 'true');
      menu(MENUS[name](), r.left, r.bottom + 4, () => b.setAttribute('aria-expanded', 'false'));
    });
    box.append(b);
  }
}

// ----- A Xenify server (File › Xenify server): data sources for query sheets -----

const server = new Server({
  // Server documents are named by their tab.
  // A promise, as server.js expects (it may .catch it).
  call: async (name, args) => call(name, args && 'id' in args ? { ...args, tab: args.id } : args),
  apply: async (name, args) => {
    if (name === 'remote_open') {
      // Opening one: a tab of its own. Putting this book there: this tab.
      if (args.data || args.blank) newTab(args.name, () => engineTabs.open());
      call('remote_open', args);
      Object.assign(front(), { remote: true, title: args.name, path: null, handle: null });
      changed();
      showTabs();
      app.csvLinks?.opened();
      return { id: engineTabs.front() };
    }
    const out = call(name, 'id' in args ? { ...args, tab: args.id } : args);
    changed();
    return out;
  },
  live,
  app: 'calc',
  ext: 'xcalc',
  onChange: () => { app.db?.sync(); showTabs(); showPeople(); offerUnsent(); platform.serverWords(server); },
  onError: (e) => toast(String(e.message || e)),
  // Each saved version (and, while editing together, every little while)
  // puts what Write can show of the book on the server: its tables,
  // summaries and sheets as shown, and its charts as pictures.
  afterSave: async (id) => {
    const snap = await linkSnapshot(id);
    await server.savePreview(id, new TextEncoder().encode(JSON.stringify(snap)));
  },
});

/// What Write shows of book `tab`: its tables, summaries and sheets as
/// shown, and its charts as pictures (for a server book's preview, and as
/// link.json inside a local .xcalc, so Write can link to either).
async function linkSnapshot(tab) {
  const snap = call('link_snapshot', { tab });
  for (const item of snap.items) {
    if (item.kind !== 'chart') continue;
    try { item.png = await chartPicture(tab, item.chart); } catch {}
  }
  return snap;
}

/// Chart `chart` of book `tab` as a PNG (base64), drawn as on the sheet.
async function chartPicture(tab, chart) {
  const sheets = call('sheets', { tab });
  let c = null;
  for (const s of sheets) { c = call('charts', { tab, sheet: s.id }).find((x) => x.id === chart); if (c) break; }
  if (!c) return null;
  const data = call('chart_data', { tab, id: chart });
  const W = 720; const H = 400;
  const box = h('div', { style: { position: 'fixed', left: '-10000px', top: '0', width: `${W}px`, height: `${H - 40}px` } });
  document.body.append(box);
  try {
    app.charts.plot(box, { ...c, w: W + 32, h: H }, data, data.series.slice(0, 6));
    const svg = box.querySelector('svg');
    if (!svg) return null;
    svg.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
    const url = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(new XMLSerializer().serializeToString(svg))))}`;
    const img = new Image();
    await new Promise((ok, no) => { img.onload = ok; img.onerror = no; img.src = url; });
    const canvas = document.createElement('canvas');
    const scale = 2;
    canvas.width = svg.width.baseVal.value * scale;
    canvas.height = (svg.height.baseVal.value + (c.title ? 40 : 0)) * scale;
    const g = canvas.getContext('2d');
    g.scale(scale, scale);
    g.fillStyle = '#ffffff';
    g.fillRect(0, 0, canvas.width, canvas.height);
    if (c.title) { g.fillStyle = '#1c1917'; g.font = '600 16px sans-serif'; g.fillText(c.title, 4, 24); }
    g.drawImage(img, 0, c.title ? 40 : 0);
    return canvas.toDataURL('image/png').split(',')[1];
  } finally {
    box.remove();
  }
}
server.onSaid = (id) => { if (id === engineTabs.front()) showPeople(); };
serverUi.server = server;
app.server = server;
serverUi.env = {
  h, toast,
  modalShell: serverModal,
  showSettings: () => openServer(),
  refresh: async () => changed(),
  // An earlier version (or changes kept from before), as a book of its own.
  openCopy: async (name, copy) => openBytes(`${name}.xcalc`, Uint8Array.from(atob(copy.data), (ch) => ch.charCodeAt(0))),
};

// ----- Who else is here (editing together): their cursors on the grid -----

const PEOPLE_COLOURS = ['#0F766E', '#1D4ED8', '#7C3AED', '#BE185D', '#B45309'];
const colourOf = (name) => PEOPLE_COLOURS[[...name].reduce((a, ch) => a + ch.charCodeAt(0), 0) % PEOPLE_COLOURS.length];

function showPeople() {
  const id = engineTabs.front();
  const marks = [];
  for (const s of server.saidOf(id)) {
    try {
      const at = call('live_heard', { tab: id, data: s.data });
      if (at.sheet == null) continue;
      const who = s.who.name || s.who.username || 'Someone';
      marks.push({ sheet: at.sheet, top: at.row, left: at.col, bottom: at.bottom ?? at.row, right: at.right ?? at.col, color: colourOf(who), label: who.split(' ')[0], above: true });
    } catch {}
  }
  app.grid.peers = marks;
  app.grid.draw();
  const here = server.here(id);
  $('people').replaceChildren(...here.slice(0, 5).map((p) => {
    const who = p.name || p.username || 'Someone';
    return h('span', { class: 'face', title: `${who} is here`, style: { background: colourOf(who) } }, who.split(/\s+/).map((w) => w[0]).join('').slice(0, 2).toUpperCase());
  }));
}

let lastSaid = '';
let sayTimer = null;
/// Tells the others where this window's cursor is (at most every 150 ms).
function sayWhere() {
  const id = engineTabs.front();
  if (!server.docs.get(id)?.joined) return;
  const rg = app.grid.range();
  const at = { sheet: sheet(), row: rg.top, col: rg.left, bottom: Math.min(rg.bottom, rg.top + 500), right: Math.min(rg.right, rg.left + 100) };
  const now = JSON.stringify([id, at]);
  if (now === lastSaid) return;
  lastSaid = now;
  clearTimeout(sayTimer);
  sayTimer = setTimeout(() => {
    try { server.say(id, call('live_say', { tab: id, ...at }).data); } catch {}
  }, 150);
}

/// A dialog that stays open until its action says so (the server screens'
/// kind: an action returning false keeps it open).
function serverModal(title, body, buttons) {
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
  $('popups').append(back);
  return { close, body: box };
}

/// File › Xenify server: connect, sign in, sign out. `then` runs after it closes.
/// Served by the server, there's nothing to choose: not signed in goes to
/// its Files page to sign in (which comes back here).
function openServer(then) {
  if (server.servedHere) {
    if (!server.session) location.assign(server.signInHere);
    else { then?.(); app.db?.sync(); }
    return;
  }
  const holder = h('div', { class: 'srv-holder' });
  const redraw = () => holder.replaceChildren(serverRow(redraw));
  redraw();
  const shell = serverModal('Xenify server', [h('p', {}, 'Data sources (databases an admin shares with you) come through a Xenify server.'), holder], [['Done', async () => { then?.(); app.db?.sync(); }, true]]);
  shell.body.classList.add('srv-modal');
  shell.body.querySelector('input')?.focus();
}

const notesEnv = () => ({ call, run, sheet });

/// Edit › Look up a word: the picked cell's first word (or one typed in).
async function lookUpWord() {
  const a = app.grid.active;
  let word = app.spelling.firstWord(a.r, a.c);
  if (!word) word = await ask('Look up a word', 'Word', '', 'Look up');
  if (word) app.spelling.lookUp(word.trim(), app.spelling.firstWord(a.r, a.c) ? a.r : null, a.c);
}

function cellMenu(hit, x, y) {
  const g = app.grid;
  const rg = g.range();
  const t = hit.r ? g.tableAt(hit.r, hit.c || g.active.c) : null;
  const rows = hit.kind === 'rowhead';
  const cols = hit.kind === 'colhead';
  menu([
    ...(hit.kind === 'cell' && app.spelling ? app.spelling.menuItems(hit.r, hit.c) : []),
    item('Cut', { keys: 'Ctrl+X', run: () => { const s = copy(true); if (s != null) navigator.clipboard?.writeText(s).catch(() => {}); } }),
    item('Copy', { keys: 'Ctrl+C', run: () => { const s = copy(false); if (s != null) navigator.clipboard?.writeText(s).catch(() => {}); } }),
    item('Paste', { keys: 'Ctrl+V', run: () => pasteFromMenu(false) }),
    item('Paste values only', { run: () => pasteFromMenu(true) }),
    null,
    !cols ? item(rows ? `Insert ${rg.bottom - rg.top + 1 > 1 ? `${rg.bottom - rg.top + 1} rows` : 'a row'} above` : 'Insert rows above', { run: () => insertLines(true, false) }) : undefined,
    !rows ? item(cols ? `Insert ${rg.right - rg.left + 1 > 1 ? `${rg.right - rg.left + 1} columns` : 'a column'} to the left` : 'Insert columns to the left', { run: () => insertLines(false, false) }) : undefined,
    !cols ? item('Delete rows', { run: () => deleteLines(true) }) : undefined,
    !rows ? item('Delete columns', { run: () => deleteLines(false) }) : undefined,
    cols ? item('Column width…', { run: columnWidth }) : undefined,
    cols || (t && hit.r === t.top) ? item('About this column…', { run: () => aboutColumn(notesEnv(), t ? t.top : hit.r || 1, hit.c || g.active.c, colName) }) : undefined,
    null,
    item('Clear contents', { keys: 'Delete', run: clearCells }),
    null,
    item('Sort A to Z', { run: () => sortBy(false) }),
    item('Sort Z to A', { run: () => sortBy(true) }),
    t ? item('Filter this column…', { run: filterHere }) : item('Make a table', { run: () => tableHere() }),
    item('Totals row', { sub: () => TOTALS.map(([k, label]) => item(label, { run: () => totalFor(k) })) }),
  ], x, y);
}

function cycleSheet(step) {
  const sheets = call('sheets').filter((s) => !s.working);
  const i = sheets.findIndex((s) => s.id === sheet());
  if (i >= 0 && sheets.length > 1) goSheet(sheets[(i + step + sheets.length) % sheets.length].id);
}

function shortcuts() {
  const keys = [
    ['Arrows, Ctrl+arrows', 'Move, jump to the end of the filled cells'], ['Shift+arrows', 'Select'],
    ['Enter, Tab', 'Down, right (Shift: back)'], ['F2', 'Change what’s in the cell'], ['Alt+Enter', 'A new line in a cell'],
    ['Delete', 'Clear the cells'], ['Ctrl+C / X / V', 'Copy, cut, paste'], ['Ctrl+Shift+V', 'Paste values only'],
    ['Ctrl+Z / Ctrl+Y', 'Undo, redo'], ['Ctrl+B / I / U', 'Bold, italic, underline'], ['Ctrl+F, Ctrl+H', 'Find, replace'], ['Ctrl+J', 'Ask about the data'],
    ['Ctrl+T', 'Make a table'], ['Ctrl+A', 'Select the block, then everything'], ['Ctrl+PgUp / PgDn', 'Previous, next sheet'],
    ['Ctrl+S / O / N', 'Save, open, new book'],
  ];
  modal('Keyboard shortcuts', h('div', { class: 'keys-list' }, keys.flatMap(([k, d]) => [h('kbd', {}, k), h('span', {}, d)])), [['Close', true, true]]);
}

async function licences() {
  const text = await platform.bundled('THIRD_PARTY_LICENSES.txt').catch(() => '');
  const own = await platform.bundled('LICENSE.txt').catch(() => '');
  const pre = h('pre', { style: { maxHeight: '50vh', overflow: 'auto', whiteSpace: 'pre-wrap', fontSize: '12px', background: 'var(--soft)', padding: '10px', borderRadius: '10px' } }, `${own}\n\n${text}`);
  modal('Licences', h('div', {}, h('p', {}, 'Xenify Calc is free under the MIT licence. It uses IronCalc (MIT or Apache-2.0) to work out formulas, and these other parts:'), pre), [['Close', true, true]]);
}

let updateReady = null;
async function checkUpdate() {
  if (!platform.DESKTOP) return;
  try { updateReady = await window.__TAURI__.core.invoke('update_ready'); } catch { updateReady = null; }
}

async function restartUpdated() {
  for (const t of [...app.tabs]) {
    if (!isDirty(t)) continue;
    switchTab(t.id);
    const what = await unsavedPrompt(t);
    if (!what || (what === 'save' && !(await save()))) return;
  }
  window.__TAURI__.core.invoke('restart_updated');
}

// ----- Keys for the whole window -----

function bindKeys() {
  document.addEventListener('keydown', (e) => {
    const ctrl = e.ctrlKey || e.metaKey;
    const inField = e.target instanceof HTMLInputElement || (e.target instanceof HTMLTextAreaElement && e.target !== app.grid.editor);
    if (e.key === 'F7' && e.shiftKey && !inField && !app.grid.editing) { e.preventDefault(); lookUpWord(); return; }
    const leave = browserCloseKey(e);
    if (leave === 'close') { e.preventDefault(); closeTab(engineTabs.front()); return; }
    if (leave === 'quit' && servedHere) { e.preventDefault(); quit(); return; }
    if (!ctrl) return;
    const k = e.key.toLowerCase();
    const onGrid = !inField && !app.grid.editing;
    const take = (f) => { e.preventDefault(); f(); };
    if (k === 's') return take(() => save(e.shiftKey));
    if (k === 'o') return take(openBook);
    if (k === 'n') return take(newBook);
    if (k === 'w') return take(() => closeTab(engineTabs.front()));
    if (k === 'q' && (platform.DESKTOP || servedHere)) return take(quit);
    if (k === 'f') return take(() => openFind(false));
    if (k === 'j') return take(() => app.ask.open());
    if (k === 'p') return take(printNow);
    if (k === 'h') return take(() => openFind(true));
    if (e.key === 'PageDown') return take(() => cycleSheet(1));
    if (e.key === 'PageUp') return take(() => cycleSheet(-1));
    if (e.key === 'Tab') return take(() => { const i = app.tabs.findIndex((t) => t.id === engineTabs.front()); if (app.tabs.length > 1) switchTab(app.tabs[(i + (e.shiftKey ? -1 : 1) + app.tabs.length) % app.tabs.length].id); });
    if (!onGrid) return;
    if (k === 'z') return take(e.shiftKey ? doRedo : doUndo);
    if (k === 'y') return take(doRedo);
    if (k === 'b') return take(() => format({ bold: !activeLook().b }));
    if (k === 'i') return take(() => format({ italic: !activeLook().i }));
    if (k === 'u') return take(() => format({ underline: !activeLook().u }));
    if (k === 'a') return take(selectAll);
    if (k === 't') return take(() => tableHere());
    if (k === 'v' && e.shiftKey) return take(() => pasteFromMenu(true));
  });
  // The system clipboard: copy and paste events carry it (no permission asked).
  document.addEventListener('copy', (e) => {
    if (!typing(e.target)) {
      const o = copyObject();
      if (o) { e.clipboardData.setData('text/plain', o.text); e.preventDefault(); return; }
    }
    if (!gridFocused()) return;
    const text = copy(false);
    if (text != null) { e.clipboardData.setData('text/plain', text); e.preventDefault(); }
  });
  document.addEventListener('cut', (e) => {
    if (!gridFocused()) return;
    const text = copy(true);
    if (text != null) { e.clipboardData.setData('text/plain', text); e.preventDefault(); }
  });
  document.addEventListener('paste', (e) => {
    if (typing(e.target)) return;
    const text = e.clipboardData.getData('text/plain');
    const o = app.objClip && (!text || text === app.objClip.text) ? app.objClip : null;
    if (app.reports?.open) {
      e.preventDefault();
      const cells = app.clip && (!text || text === app.clip.clip.text) && app.clip.tab === front().id ? app.clip : null;
      app.reports.paste(o, cells);
      return;
    }
    const chartOn = app.charts.selected != null || app.pictures.selected != null;
    if (!gridFocused() && !(o && chartOn)) return;
    e.preventDefault();
    // A picture on the clipboard (copied from another program): on the sheet at the cursor.
    const image = [...(e.clipboardData.files || [])].find((f) => /^image\//.test(f.type));
    if (image && !o) { const a = app.grid.active; app.pictures.addFile(image, a.r, a.c); return; }
    if (o) { pasteObject(o); return; }
    paste(text);
  });
}

/// Whether the key or clipboard event is for something being typed in.
function typing(target) {
  return !!target?.closest?.('input, textarea, select, [contenteditable="true"]') && target !== app.grid.scroller;
}

// ----- Copying charts and report blocks -----

/// What Ctrl+C copies when a chart or a report block is picked (none: cells as usual).
function copyObject() {
  if (app.pictures?.selected != null && !app.reports?.open) return copyPicture(app.pictures.selected);
  if (app.charts?.selected != null && !app.reports?.open) return copyChart(app.charts.selected);
  if (app.reports?.open && app.reports.selected != null) return copyBlock(app.reports.selected);
  return null;
}

function copyChart(id) {
  const chart = call('charts', { sheet: sheet() }).find((c) => c.id === id);
  if (!chart) return null;
  app.objClip = { kind: 'chart', chart, tab: front().id, text: `Chart: ${chart.title || 'untitled'}` };
  app.clip = null; app.grid.copied = null; app.textClip = null;
  return app.objClip;
}

function copyPicture(id, button = false) {
  const picture = call('my_pictures', { sheet: sheet() }).find((p) => p.id === id);
  if (!picture) return null;
  app.objClip = { kind: 'picture', picture, tab: front().id, text: `Picture: ${picture.name || 'picture'}` };
  app.clip = null; app.grid.copied = null; app.textClip = null;
  if (button) {
    navigator.clipboard?.writeText(app.objClip.text).catch(() => {});
    toast('Copied: click a cell on any sheet and paste (Ctrl+V)');
  }
  return app.objClip;
}

function copyBlock(k) {
  const def = call('report', { sheet: sheet() }).def;
  const block = def.blocks[k];
  if (!block) return null;
  const words = block.text || block.label || ({ chart: 'Chart', cells: 'Table', summary: 'Summary' }[block.kind]) || 'Block';
  app.objClip = { kind: 'block', block, tab: front().id, text: `Report block: ${words}` };
  app.clip = null; app.grid.copied = null; app.textClip = null;
  return app.objClip;
}

/// Ctrl+V of a copied chart (or a report's chart block) on a sheet: a copy at the cursor.
function pasteObject(o) {
  if (o.kind === 'picture') { const a = app.grid.active; app.pictures.paste(o.picture, a.r, a.c); return; }
  if (o.tab !== front().id) { toast('Charts can be pasted in the book they came from (their numbers are there)'); return; }
  let chart = o.kind === 'chart' ? o.chart : null;
  if (o.kind === 'block' && o.block.kind === 'chart') {
    for (const s of call('sheets')) { chart = call('charts', { sheet: s.id }).find((c) => c.id === o.block.chart); if (chart) break; }
  }
  if (!chart) { toast('Only charts can go on a sheet: paste this on a report'); return; }
  const a = app.grid.active;
  app.charts.paste({ ...chart, dx: 8, dy: 8 }, a.r, a.c);
}

function gridFocused() {
  return document.activeElement === app.grid.scroller;
}

// ----- Start -----

// In the browser on a Xenify server: File › Quit goes back to its Files page.
let servedHere = false;
let leaving = false;
function showHome() {
  if (!/^https?:$/.test(location.protocol)) return;
  fetch('/v1/info').then((r) => {
    servedHere = r.ok && /json/.test(r.headers.get('content-type') || '');
    if (servedHere) buildMenus();
  }).catch(() => {});
}

async function boot() {
  setUpWindow();
  await start();
  $('new-tab').innerHTML = icon('plus', 16);
  $('new-tab').addEventListener('click', newBook);
  $('add-sheet').innerHTML = icon('plus', 15);
  $('add-sheet').addEventListener('click', addSheet);
  $('all-sheets').addEventListener('click', () => (app.all.open ? app.all.hide() : app.all.show()));
  buildMenus();
  buildToolbar();
  bindFormulaBar();
  bindKeys();
  app.grid = new Grid({
    host: $('sheet'), scroller: $('scroller'), spacer: $('spacer'), canvas: $('grid'), editor: $('editor'),
    call,
    onSelect: () => selectionChanged(),
    onCommit: (r, c, v) => typeIn(r, c, v),
    onAddRow: (t, c, v) => addRow(t, c, v),
    onClear: clearCells,
    onContext: (hit, x, y) => cellMenu(hit, x, y),
    onTotals: (hit, x, y) => totalsPicker(hit, x, y),
    onFilter: (hit, x, y) => filterPicker(hit, x, y),
    onResize: (rows, list, size) => resizeLines(!rows, list, size),
    onAutoWidth: autoWidth,
    onLocked: () => toast('This sheet shows what its source gives: it can’t be typed in'),
    onEditing: (v) => { if (v != null) $('formula').value = v; else app.help?.hide(); },
    onSheetBack: (id) => goSheet(id),
    onAway: (away) => $('formula').classList.toggle('pointing', away),
    onChoices: (ch, a, x, y) => menu(ch.values.map((v) => item(v, { run: () => typeIn(a.r, a.c, v) })).concat(ch.values.length ? [] : [item('No choices (the list is empty)', { enabled: false })]), x, y),
    onHoverComment: (cm, x, y) => showComment(cm, x, y),
  });
  app.lookup = new Lookup({ call, run, grid: app.grid, sheet, goSheet, sheetName, toast });
  app.reports = new Reports({ call, run, sheet, sheetName, get charts() { return app.charts; }, onHidden: () => { app.grid.resize(); app.grid.draw(); },
    tab: () => front()?.id, copy: (k) => copyBlock(k), clipped: () => ({ o: app.objClip, cells: app.clip && app.clip.tab === front()?.id ? app.clip : null }) });
  app.summarise = new Summarise({ call, run, grid: app.grid, sheet, goSheet, sheetName, toast, host: $('sheet'), copied: (text) => { app.clip = null; app.grid.copied = null; app.textClip = text; app.grid.draw(); } });
  app.charts = new Charts({ call, run, grid: app.grid, sheet, host: $('sheet'), tab: () => front()?.id, edit: (id) => app.chartPanel.start(id), copy: (id) => copyChart(id), onRemoved: () => app.chartPanel?.stop() });
  app.help = new FormulaHelp({ call, run, sheet, goSheet, grid: app.grid, host: $('sheet') });
  app.help.attach(app.grid.editor);
  app.help.attach($('formula'));
  $('fx').addEventListener('click', () => { const w = app.grid.editing || document.activeElement === $('formula') ? $('formula').value : ''; const m = /([A-Za-z][A-Za-z0-9.]*)\(/.exec(w); app.help.open(m ? m[1].toUpperCase() : null); });
  app.chartPanel = new ChartPanel({ call, run, charts: app.charts, sheet, sheetName, host: $('sheet') });
  app.pictures = new Pictures({ call, run, grid: app.grid, sheet, layer: app.charts.layer, copy: (id) => copyPicture(id, true) });
  app.grid.onPaint = () => { app.charts.place(); app.pictures.place(); };
  app.db = new Databases({ call, run, server, sheet, goSheet, openServer, grid: app.grid, tab: () => engineTabs.front(), editors: () => app.editors });
  app.mcp = new Mcp({ call, run, changed, desktop: platform.DESKTOP, tabs: () => app.tabs.map((t) => ({ id: t.id, title: t.title })), frontTab: () => engineTabs.front() });
  app.mcp.boot().catch((e) => platform.log(`MCP: ${e}`));
  app.ask = new Ask({ call, run, server, sheet, goSheet, grid: app.grid, get charts() { return app.charts; }, openServer, mcp: app.mcp });
  app.spelling = new Spelling({ call, run, grid: app.grid, sheet });
  app.spelling.soon(0);
  app.fxai = new FormulaAI({ call, run, grid: app.grid, sheet, sheetName, ask: app.ask, help: app.help });
  $('fx-ai').addEventListener('click', () => app.fxai.write());
  app.csvLinks = new CsvLinks({ call, run, server, sheet, goSheet, grid: app.grid, tab: () => engineTabs.front(), changed });
  app.editors = new Editors({ call, run, server, sheet, goSheet, openServer, grid: app.grid, tab: () => engineTabs.front(), save: () => save() });
  app.all = new AllSheets({
    call, run, goSheet, renameSheet, deleteSheet, addSheet, sheet,
    onOpen: () => showSheets(),
    onClose: () => { showSheets(); app.grid.resize(); app.grid.scroller.focus(); },
  });
  // Closing the window asks about unsaved work first.
  if (platform.DESKTOP) {
    window.__TAURI__.window.getCurrentWindow().onCloseRequested((ev) => { ev.preventDefault(); quit(); });
  } else {
    window.addEventListener('beforeunload', (e) => { if (!leaving && app.tabs.some(isDirty)) { e.preventDefault(); e.returnValue = ''; } });
  }
  const files = await platform.openedFiles().catch(() => []);
  newBook();
  for (const f of files) {
    try { const got = await platform.readPath(f); openBytes(got.name, got.bytes, got.path); } catch (e) { toast(`Couldn’t open ${f}: ${e.message || e}`); }
  }
  app.grid.scroller.focus();
  checkUpdate();
  setInterval(checkUpdate, 120000);
  document.documentElement.classList.add('ready');
  showHome();
  await offerRecovery();
  // Opened from a Xenify server (the browser version): use that server, and
  // open #open=<server>,<book> from its Files page.
  server.connectHere().then(() => {
    // ?open=<file> (this server's) or #open=<server>,<file>.
    const here = new URLSearchParams(location.search).get('open');
    const linked = /^\d+$/.test(here || '') ? [null, location.origin, here] : /^#open=([^,]+),(\d+)$/.exec(location.hash);
    if (linked) {
      history.replaceState(null, '', location.pathname + location.search);
      openLinked(decodeURIComponent(linked[1]), +linked[2]);
    }
  }).catch(() => {});
  if (window.XENIFY_TEST) {
    try { await window.XENIFY_TEST(); } catch (e) { platform.log(`test failed: ${e.stack || e}`); }
  }
}

// For tests (XENIFY_TEST scripts) and the browser's console.
Object.assign(app, { call, run, changed, goSheet, format, save, tableHere, totalFor, openFind, sheet, front, sheetName, openBytes, aboutFile });

boot().catch((e) => {
  document.documentElement.classList.add('ready');
  platform.log(`Calc failed to start: ${e.stack || e}`);
  document.body.append(h('p', { style: { padding: '24px' } }, `Calc couldn’t start: ${e.message || e}`));
});
