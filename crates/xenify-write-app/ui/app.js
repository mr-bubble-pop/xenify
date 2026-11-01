// The window around the pages: menus, document tabs, toolbar, the
// spelling and find bars, notes in the margin and the status bar.

import { call, live } from './engine.js';
import { Server } from './server.js';
import { ui as serverUi, openFromServer, openLinked, saveToServer, shareDialog, historyDialog, checkpointDialog, pickFile, offerUnsent } from './serverui.js';
import { icon } from './icons.js';
import { refreshAiServer, hasAi, noAi } from './aiserver.js';
import { PageView, forgetLinkedPictures, forgetPicture } from './page.js';
import { Ruler } from './ruler.js';
import { openSettings } from './settings.js';
import { words, syncWords } from './words.js';
import { aiDialog } from './ai.js';
import { citeDialog, sourcesDialog } from './refs.js';
import { setUpWindow } from './window.js';
import { mathEditor } from './mathedit.js';

/// Replaces an element's children, skipping empty ones (null, false) and
/// flattening lists.
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

const $ = (id) => document.getElementById(id);
const h = (tag, props = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (v === true) el.setAttribute(k, '');
    else if (v !== false && v != null) el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null) el.append(kid);
  return el;
};

const app = {
  view: null,
  tabs: null,
  styles: [],
  fonts: [],
  issues: [],
  checking: false,
  activeIssue: null,
};

// A Xenify server, if you connect to one (Settings › Server).
const server = new Server({
  call,
  // The server's own changes: never refused here.
  apply: (name, args) => run(name, args, true),
  live,
  app: 'write',
  ext: 'xwrite',
  onChange: () => { renderTabs(); refreshAiServer(); offerUnsent(); syncWords().then(() => proofSoon(0)); },
  onError: (e) => toast(String(e.message || e)),
  // Each saved version (and, while editing together, every little while)
  // puts a picture of its first page on the server, for
  // canvases that show it (Draw's Insert › Picture from a server).
  afterSave: async (id) => {
    if (id !== app.tabs?.front) return;
    const png = await pages.pagePng(0);
    if (png) await server.savePreview(id, png);
  },
});
serverUi.server = server;
serverUi.env = {
  h, toast, modalShell: (...a) => modalShell(...a),
  showSettings: (section) => showSettings(section),
  refresh: async () => { pages.drawn.clear(); pages.draw(); app.prefs = await call('prefs'); },
  // An earlier version, as a new document on this computer.
  openCopy: async (name, copy) => { await run('open_bytes', { name: `${name}.xwrite`, data: copy.data }); pages.drawn.clear(); pages.draw(); },
};

// The pages.
function makeRuler(el) {
  const r = new Ruler(el);
  r.onIndent = (patch) => run('set_para', { patch });
  r.onMargins = async ({ left, right }) => {
    const { page } = await call('page');
    await run('set_page', { page: { margins: { ...page.margins, left, right } } });
    pages.drawn.clear();
    pages.draw();
  };
  return r;
}
const pages = new PageView({
  scroller: $('scroller'), stack: $('stack'), overlay: $('overlay'), ime: $('ime'), ruler: makeRuler($('ruler')),
  onView: (v) => { app.view = v; refresh(); maybeProof(v); },
  onResult: (out) => {
    if (out.tabs) app.tabs = out.tabs;
    if (out.details) app.details = out.details;
    if (out.open_link) openLink(out.open_link);
    // Typing put back (a table from a plan or a Calc book, a template's part): say why.
    if (out.refused) toast(out.refused);
  },
});
const ruler = pages.ruler;
pages.onZoom = () => renderStatus();

function onStacks(type, fn, opts) { pages.stack.addEventListener(type, fn, opts); }
function onScrollers(type, fn, opts) { pages.scroller.addEventListener(type, fn, opts); }
const isIme = (el) => el === $('ime');
document.addEventListener('focusin', () => document.documentElement.classList.toggle('focused', isIme(document.activeElement)));
document.addEventListener('focusout', () => setTimeout(() => document.documentElement.classList.toggle('focused', isIme(document.activeElement))));

// ----- Commenting only -----

/// How far you may go with the front document, when it's on a server and
/// you may only comment on it ('commenter') or read it ('viewer'); else null.
function limited() {
  const role = server.docs.get(app.tabs?.front)?.role;
  return role === 'commenter' || role === 'viewer' ? role : null;
}

// What still works then: moving about, finding, the panels and tabs, and comments.
const LIMITED_OK = new Set(['view', 'key', 'down', 'up', 'move', 'select_range', 'find', 'find_next', 'find_close', 'go_to', 'panel', 'reading',
  'switch', 'close', 'open', 'open_bytes', 'new', 'recover', 'save', 'save_source', 'copy', 'select_issue', 'add_comment', 'reply', 'resolve', 'delete_comment']);
const COMMENT_ONLY = new Set(['add_comment', 'reply', 'resolve', 'delete_comment']);
// Keys that change the text (Ctrl with a letter: formatting) rather than move about.
const KEEP_KEYS = new Set(['a', 'c', 'd', 'f', 'g', 'h', 'p', 's', 'w', 'n', 'o']);
function changesText(name, args) {
  if (name === 'text' || name === 'cut' || name === 'paste') return true;
  if (name !== 'key') return false;
  const k = String(args?.key || '');
  return ['Backspace', 'Delete', 'Enter', 'Tab'].includes(k) || (!!args?.mods?.command && k.length === 1 && !KEEP_KEYS.has(k.toLowerCase()));
}
let limitedSaid = 0;
function refuseHere(name, args) {
  const role = limited();
  if (!role) return false;
  const no = !LIMITED_OK.has(name) || changesText(name, args) || (role === 'viewer' && COMMENT_ONLY.has(name));
  // Pictures and tables from elsewhere keeping up by themselves: quietly not.
  if (no && name.endsWith('_update')) return true;
  if (no && Date.now() - limitedSaid > 4000) {
    limitedSaid = Date.now();
    toast(role === 'viewer' ? 'You can read this document, not change it.' : 'You can comment on this document, not change it: select some text and press Ctrl+Alt+M.');
  }
  return no;
}
pages.refuse = refuseHere;
server.onJoined = () => refreshToolbar();

/// Greys out what changes the document, and says what you can do.
function syncLimited() {
  const role = limited();
  document.body.classList.toggle('limited', !!role);
  for (const [k, b] of Object.entries(tb)) {
    if (k === 'comment') b.disabled = role === 'viewer';
    else if (role) b.disabled = true;
    // Undo and redo follow what there is to undo (refreshToolbar).
    else if (k !== 'undo' && k !== 'redo') b.disabled = false;
  }
  let note = $('limited-note');
  if (!role) { note?.remove(); return; }
  if (!note) {
    note = h('span', { id: 'limited-note', class: 'limited-note', role: 'status' });
    $('toolbar').append(note);
  }
  note.textContent = role === 'viewer' ? 'You can read this' : 'You can comment';
  note.title = role === 'viewer' ? 'Whoever shared it with you can let you comment or edit' : 'Add comments, reply and resolve; change or delete your own. Whoever shared it with you can let you edit';
}

// In a menu, what stays when you may only comment (by menu; others keep all
// but `LIMITED_OFF`).
const LIMITED_MENU = {
  Edit: ['Copy', 'Select all', 'Select next match too', 'Add caret above', 'Add caret below', 'Back to one caret', 'Find…', 'Look up word'],
  Insert: ['Comment…'],
  Format: [],
  References: [],
  Review: ['Look up word', 'Comment…', 'Show resolved comments', 'Go through comments'],
};
const LIMITED_OFF = ['Mail merge…', 'Page setup…', 'Save as template…'];
function limitMenu(menu, items) {
  const role = limited();
  if (!role) return items;
  const keep = LIMITED_MENU[menu];
  for (const x of items) {
    if (!(x instanceof HTMLElement) || !x.classList.contains('item')) continue;
    const label = x.firstChild?.textContent || '';
    const ok = keep ? keep.includes(label) && !(role === 'viewer' && label === 'Comment…') : !LIMITED_OFF.includes(label) && !label.startsWith('Update from');
    if (!ok) { x.disabled = true; x.title = role === 'viewer' ? 'You can read this document, not change it' : 'You can comment on this document, not change it'; }
  }
  return items;
}

async function run(name, args = {}, fromServer = false) {
  if (!fromServer && refuseHere(name, args)) return null;
  try {
    const out = await call(name, { ...args, have: pages.have() });
    if (out && out.scenes) await pages.paintScenes(out.view, out.scenes);
    if (out && out.tabs) app.tabs = out.tabs;
    if (out && out.details) app.details = out.details;
    if (out && out.marks) pages.marks = out.marks;
    if (out && out.view) pages.show(out.view, { reveal: true });
    // May only read or comment here: the change was put back.
    if (out && out.refused) toast(out.refused);
    // A template was opened: a new document from it (it can be edited instead).
    if (out && out.made_from_template) madeFromTemplate(out.made_from_template);
    return out;
  } catch (e) {
    toast(String(e));
    return null;
  } finally {
    // Back to the page, unless you're typing somewhere else (find, a field).
    const a = document.activeElement;
    const elsewhere = a && a !== document.body && !isIme(a) && (a.tagName === 'INPUT' || a.tagName === 'SELECT' || a.tagName === 'TEXTAREA' || a.isContentEditable);
    if (!document.querySelector('.popup') && !elsewhere) pages.focus();
  }
}

/// Opens the plan or Calc book (server file `node`) a linked table shows,
/// in its app in the browser.
async function openLinkedSource(node) {
  try {
    const n = await server.api('GET', `/v1/nodes/${node}`);
    const app = n?.details?.app || (n?.meta && JSON.parse(n.meta).app);
    if (!['plan', 'calc'].includes(app)) { toast('It isn’t on the server any more, or you can’t open it.'); return; }
    openLink(`${server.url}/${app}/?open=${node}`);
  } catch (e) { toast(String(e.message || e)); }
}

function openLink(url) {
  if (WEB) {
    // Like the desktop app: only web and email links (never javascript: and the like).
    if (/^(https?:\/\/|mailto:)/i.test(url)) window.open(url, '_blank', 'noopener');
    else toast('Only web and email links open from here.');
    return;
  }
  window.__TAURI__.core.invoke('open_link', { url }).catch((e) => toast(String(e)));
}

/// The checker marks words in its messages with `backticks`: shown as quotes.
const quoted = (msg) => String(msg).replace(/`([^`]+)`/g, '“$1”');

function toast(text) {
  const t = h('div', { class: 'toast', role: 'status' }, text);
  document.body.append(t);
  setTimeout(() => t.remove(), 3200);
}

// ----- Popups -----

let openPopup = null;
function closePopup() {
  if (!openPopup) return;
  openPopup.el.remove();
  openPopup.anchor?.setAttribute('aria-expanded', 'false');
  openPopup = null;
}
function popup(anchor, content, { width } = {}) {
  closePopup();
  const r = anchor.getBoundingClientRect();
  const el = h('div', { class: 'popup', role: 'menu' }, content);
  if (width) el.style.width = `${width}px`;
  el.style.left = `${Math.min(r.left, window.innerWidth - (width || 240) - 8)}px`;
  el.style.top = `${r.bottom + 6}px`;
  $('popups').append(el);
  // Not enough room below: up, so it stays on screen.
  const ph = el.offsetHeight;
  if (r.bottom + 6 + ph > window.innerHeight - 8) {
    const above = (r.top ?? r.bottom) - 6 - ph;
    el.style.top = `${Math.max(8, above >= 8 ? above : window.innerHeight - ph - 8)}px`;
  }
  anchor.setAttribute('aria-expanded', 'true');
  openPopup = { el, anchor };
  const first = el.querySelector('input, .item:not([disabled])');
  first?.focus();
  return el;
}
document.addEventListener('pointerdown', (e) => {
  if (openPopup && !openPopup.el.contains(e.target) && !openPopup.anchor.contains(e.target)) closePopup();
}, true);
document.addEventListener('keydown', (e) => {
  if (!openPopup) return;
  const items = [...openPopup.el.querySelectorAll('.item:not([disabled])')];
  const i = items.indexOf(document.activeElement);
  if (e.key === 'Escape') { e.preventDefault(); closePopup(); pages.focus(); }
  else if (e.key === 'ArrowDown') { e.preventDefault(); items[(i + 1) % items.length]?.focus(); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); items[(i - 1 + items.length) % items.length]?.focus(); }
}, true);

// In a browser these belong to the browser (it never hands them over), so
// they aren't offered there.
const BROWSER_KEYS = new Set(['Ctrl+N', 'Ctrl+T', 'Ctrl+W', 'Ctrl+Q', 'Ctrl+Shift+N', 'Ctrl+Shift+T', 'Ctrl+Shift+W', 'Ctrl+Tab', 'Ctrl+Shift+Tab']);
const IN_BROWSER = /^https?:$/.test(location.protocol) && location.hostname !== 'tauri.localhost';
// Closing a tab and quitting are too common to go without: Alt there instead.
const IN_BROWSER_AS = { 'Ctrl+W': 'Alt+W', 'Ctrl+Q': 'Alt+Q' };
const keysHere = (keys) => (keys && IN_BROWSER && BROWSER_KEYS.has(keys) ? IN_BROWSER_AS[keys] || '' : keys);

function item(label, { keys, run: act, enabled = true, checked, style, title } = {}) {
  keys = keysHere(keys);
  return h('button', { title,
    class: 'item', role: checked === undefined ? 'menuitem' : 'menuitemcheckbox', 'aria-checked': checked === undefined ? null : String(!!checked),
    disabled: !enabled, style,
    onClick: () => { closePopup(); act?.(); },
  // The tick goes on the right, so ticked and plain items line up.
  }, h('span', {}, label), keys ? h('span', { class: 'keys' }, keys) : null, checked === undefined ? null : h('span', { class: 'check', html: checked ? icon('check', 16) : '' }));
}

/// In the browser on the website (not a Xenify server, not the desktop app):
/// File › Back to the website returns to the page that linked here, else
/// this app's page on the site. Your work stays kept in this browser.
function backToSite() {
  let same = false;
  try { const r = new URL(document.referrer); same = r.origin === location.origin && !r.pathname.startsWith('/write/'); } catch { /* */ }
  if (same && history.length > 1) history.back();
  else location.href = '/products/write/';
}

// ----- Menus -----

const st = () => app.view?.state || {};
const MENUS = [
  { label: 'File', items: () => [
    item('New document', { keys: 'Ctrl+N', run: () => showNew() }),
    WEB && !server.ready ? null : item('New from template…', { run: () => templatePicker() }),
    WEB && !server.ready ? null : item('Use a template for this document…', { enabled: !app.details?.is_template && !app.details?.template, title: 'This document keeps its text and takes a template’s styles, headers, footers and page setup, and follows the template from then on', run: () => useTemplate() }),
    item('Open…', { keys: 'Ctrl+O', run: () => openFile() }),
    item(server.session ? `Open from ${server.name}…` : 'Open from a server…', { run: () => openFromServer() }),
    ...recentItems(),
    '-',
    item('Save', { keys: 'Ctrl+S', run: () => save() }),
    item('Save as…', { keys: 'Ctrl+Shift+S', run: () => saveAs() }),
    item(server.session ? `Save to ${server.name}…` : 'Save to a server…', { run: () => saveToServer(app.tabs?.tabs.find((d) => d.id === app.tabs.front)?.title) }),
    app.tabs?.tabs.find((d) => d.id === app.tabs.front)?.remote != null ? item('Share…', { run: () => shareDialog(app.tabs.front, app.tabs.tabs.find((d) => d.id === app.tabs.front).title) }) : null,
    app.tabs?.tabs.find((d) => d.id === app.tabs.front)?.remote != null ? item('Save a checkpoint…', { run: () => checkpointDialog(app.tabs.front, app.tabs.tabs.find((d) => d.id === app.tabs.front).title) }) : null,
    app.tabs?.tabs.find((d) => d.id === app.tabs.front)?.remote != null ? item('Version history…', { run: () => historyDialog(app.tabs.front, app.tabs.tabs.find((d) => d.id === app.tabs.front).title) }) : null,
    WEB ? null : item('Save as template…', { run: () => saveTemplate() }),
    WEB ? null : item('Edit a template…', { run: () => templatePicker() }),
    item('This is a template', { checked: !!app.details?.is_template, title: 'Opening it makes a new document from it, with its title page, contents, headers, footers and styles kept as they are', run: () => app.details?.is_template ? run('template_set', { on: false }) : templateStarter() }),
    item('Detach from template…', { enabled: !!app.details?.template, title: 'The document becomes your own: changes to the template no longer come in', run: () => detachTemplate() }),
    app.details?.template ? item(`Update from “${app.details.template}”`, { run: () => checkTemplate() }) : null,
    item('Export as PDF…', { run: () => exportAs('pdf') }),
    item('Export as web page…', { run: () => exportAs('html') }),
    item('Mail merge…', { run: () => mailMerge() }),
    item('Page setup…', { run: () => pageSetup() }),
    WEB ? null : item('Print…', { keys: 'Ctrl+P', run: () => printDialog() }),
    '-',
    onServerDoc(app.tabs?.front) ? item('Discard my changes…', { enabled: !!st().can_undo, run: () => discardMine() }) : null,
    item('Close tab', { keys: 'Ctrl+W', run: () => closeTab(app.tabs.front) }),
    '-',
    item('Settings', { keys: 'Ctrl+,', run: () => showSettings() }),
    WEB && !servedHere ? item('Back to the website', { run: () => backToSite() }) : item('Quit', { keys: 'Ctrl+Q', run: () => quit() }),
  ] },
  { label: 'Edit', items: () => [
    item(st().undo_label ? `Undo ${st().undo_label.toLowerCase()}` : 'Undo', { keys: 'Ctrl+Z', enabled: st().can_undo, run: () => run('undo') }),
    item(st().redo_label ? `Redo ${st().redo_label.toLowerCase()}` : 'Redo', { keys: 'Ctrl+Y', enabled: st().can_redo, run: () => run('redo') }),
    '-',
    item('Cut', { keys: 'Ctrl+X', enabled: app.view?.selected_text != null, run: () => { pages.focus(); document.execCommand('cut'); } }),
    item('Copy', { keys: 'Ctrl+C', enabled: app.view?.selected_text != null, run: () => { pages.focus(); document.execCommand('copy'); } }),
    item('Paste', { keys: 'Ctrl+V', run: async () => { try { const t = await navigator.clipboard.readText(); if (t) run('paste', { text: t }); } catch { toast('Use Ctrl+V to paste'); } } }),
    item('Paste as plain text', { keys: 'Ctrl+Shift+V', run: async () => { try { const t = await navigator.clipboard.readText(); if (t) run('paste', { text: t, plain: true }); } catch { toast('Use Ctrl+Shift+V to paste'); } } }),
    '-',
    item('Select all', { keys: 'Ctrl+A', run: () => run('key', { key: 'a', mods: { command: true } }) }),
    item('Select next match too', { keys: 'Ctrl+D', run: () => run('key', { key: 'd', mods: { command: true } }) }),
    item('Add caret above', { keys: 'Ctrl+Alt+↑', run: () => run('key', { key: 'ArrowUp', mods: { command: true, alt: true } }) }),
    item('Add caret below', { keys: 'Ctrl+Alt+↓', run: () => run('key', { key: 'ArrowDown', mods: { command: true, alt: true } }) }),
    item('Back to one caret', { keys: 'Esc', enabled: st().carets > 1, run: () => run('key', { key: 'Escape', mods: {} }) }),
    '-',
    item('Find…', { keys: 'Ctrl+F', run: () => openFind(false) }),
    item('Find and replace…', { keys: 'Ctrl+H', run: () => openFind(true) }),
    '-',
    item('Look up word', { keys: 'Shift+F7', run: () => lookUp(null) }),
  ] },
  { label: 'View', items: () => [
    item('Zoom in', { keys: 'Ctrl++', run: () => zoomBy(1) }),
    item('Zoom out', { keys: 'Ctrl+−', run: () => zoomBy(-1) }),
    item('Actual size', { keys: 'Ctrl+0', run: () => setZoom(100) }),
    '-',
    item('Reading view', { keys: 'Ctrl+Alt+R', checked: !!app.reading, run: () => toggleReading() }),
    item('Navigation', { keys: 'Ctrl+Shift+N', checked: app.navOpen, run: () => toggleNav() }),
    item('Show resolved comments', { checked: !!app.showResolved, run: () => { app.showResolved = !app.showResolved; placeNotes(); } }),
    '-',
    item('Toolbar', { checked: app.prefs.toolbar, run: () => setPref('toolbar', !app.prefs.toolbar) }),
    item('Ruler', { checked: app.prefs.ruler, run: () => setPref('ruler', !app.prefs.ruler) }),
    item('Status bar', { checked: app.prefs.status_bar, run: () => setPref('status_bar', !app.prefs.status_bar) }),
  ] },
  { label: 'Insert', items: () => [
    item('Picture…', { run: () => insertPicture() }),
    item('From a server…', { title: 'A Draw canvas, a table, summary or chart from a Calc book, or a plan’s issues, milestones or timeline, from a Xenify server, kept up to date when they change', enabled: !!server.ready, run: () => insertFromServer() }),
    WEB ? null : item('From a Calc book on this computer…', { title: 'A table, summary or chart, kept up to date when the book is saved', run: () => insertFromCalc('calc', 'file') }),
    item('Table…', { run: () => tablePicker() }),
    item('Link…', { keys: 'Ctrl+K', run: () => linkEditor() }),
    item('Comment…', { keys: 'Ctrl+Alt+M', run: () => newComment() }),
    item('Symbols & emoji…', { keys: 'Ctrl+.', run: () => symbolsDialog() }),
    item('Equation…', { keys: 'Alt+=', run: () => equationEditor(null) }),
    item('Text box', { run: () => run('frame_insert') }),
    '-',
    item('Footnote', { keys: 'Ctrl+Alt+F', run: () => run('insert', { what: 'footnote' }) }),
    item('Endnote', { run: () => run('insert', { what: 'endnote' }) }),
    item('Title page', { run: () => run('insert', { what: 'title_page' }) }),
    item('Table of contents…', { run: () => tocDialog() }),
    item('Page number', { run: () => run('insert', { what: 'page_number' }) }),
    item('Date and time…', { run: () => dateTimeDialog() }),
    item('Change log', { run: () => run('insert', { what: 'change_log' }) }),
    '-',
    item('Header', { run: () => headerEditor('header', app.view?.caret_page ?? 0) }),
    item('Footer', { run: () => headerEditor('footer', app.view?.caret_page ?? 0) }),
    item('Page break', { keys: 'Ctrl+Enter', run: () => run('insert', { what: 'page_break' }) }),
    item('Divider line', { run: () => run('divider') }),
    '-',
    item('Template field…', { title: 'Something filled in in each document made from this template (a name, a date…)', run: () => templateField() }),
    item('Where the text goes (template)', { title: 'Each document made from this template has its text here; the rest stays the template’s', run: () => markBody() }),
  ] },
  { label: 'Format', items: () => [
    item('Bold', { keys: 'Ctrl+B', checked: st().bold, run: () => run('toggle', { what: 'bold' }) }),
    item('Italic', { keys: 'Ctrl+I', checked: st().italic, run: () => run('toggle', { what: 'italic' }) }),
    item('Underline', { keys: 'Ctrl+U', checked: st().underline, run: () => run('toggle', { what: 'underline' }) }),
    item('Strikethrough', { checked: st().strike, run: () => run('toggle', { what: 'strike' }) }),
    item('Superscript', { checked: st().superscript, run: () => run('toggle', { what: 'superscript' }) }),
    item('Subscript', { checked: st().subscript, run: () => run('toggle', { what: 'subscript' }) }),
    '-',
    item('Text colour…', { run: () => colourPicker('text') }),
    item('Highlight…', { run: () => colourPicker('highlight') }),
    item('Clear formatting', { keys: 'Ctrl+Space', run: () => run('clear_format') }),
    '-',
    item('Spacing and indents…', { run: () => spacingPicker() }),
    item('Number the headings (1, 1.1, 1.1.1)', { checked: !!app.details?.numbered_headings, run: async () => { await run('number_headings', { on: !app.details?.numbered_headings }); app.styles = await call('styles'); } }),
    item('Shading behind the paragraph…', { run: () => colourPicker('shade') }),
    '-',
    item(`Update ${styleName(st().style)} to match the selection`, { run: () => styleFromSelection(st().style) }),
    item('New style from the selection…', { run: () => styleFromSelection(null) }),
  ] },
  { label: 'References', items: () => [
    item('Insert citation…', { run: () => citeDialog(refsEnv()) }),
    item('Sources…', { run: () => sourcesDialog(refsEnv()) }),
    item('Insert bibliography', { run: () => run('bibliography') }),
    '-',
    item('Table of contents…', { run: () => tocDialog() }),
    item('Footnote', { keys: 'Ctrl+Alt+F', run: () => run('insert', { what: 'footnote' }) }),
    item('Endnote', { run: () => run('insert', { what: 'endnote' }) }),
  ] },
  { label: 'Review', items: () => [
    item('Check spelling and grammar', { keys: 'F7', run: () => openCheck() }),
    item('Kind of writing…', { run: () => kindEditor() }),
    item('Look up word', { keys: 'Shift+F7', run: () => lookUp(null) }),
    '-',
    item('Comment…', { keys: 'Ctrl+Alt+M', run: () => newComment() }),
    item('Show resolved comments', { checked: !!app.showResolved, run: () => { app.showResolved = !app.showResolved; placeNotes(); } }),
    '-',
    item('Go through comments', { run: () => openCheck('comments') }),
    '-',
    item('Track changes', { keys: 'Ctrl+Shift+E', checked: !!app.review?.tracking, run: () => toggleTracking() }),
    item('Go through changes', { run: () => openCheck('changes') }),
    item('Record a change…', { run: () => askChange() }),
    item('Accept all changes', { run: () => decide(true, true) }),
    item('Reject all changes', { run: () => decide(false, true) }),
    ...(WEB ? [] : ['-', item('Ask AI…', { run: () => openAi() })]),
  ] },
  { label: 'Help', items: () => [
    ...(updateReady ? [item(`Restart to use version ${updateReady}`, { title: 'Xenify Hub installed it; your documents are asked about first', run: () => quit(true) }), '-'] : []),
    item('Keyboard shortcuts', { keys: 'Ctrl+/', run: () => shortcutsDialog() }),
    '-',
    item('About Xenify Write', { run: () => aboutDialog() }),
  ] },
];

// Alt and a letter opens a menu (F10 the first); Left/Right move between them.
const MENU_KEYS = { f: 'File', e: 'Edit', v: 'View', i: 'Insert', o: 'Format', s: 'References', r: 'Review', h: 'Help' };
function openMenu(label) {
  const b = [...document.querySelectorAll('.menu-btn')].find((x) => x.textContent === label);
  if (b) b.click();
}
window.addEventListener('keydown', (e) => {
  if (e.key === 'F10' && !e.shiftKey) { e.preventDefault(); openMenu('File'); return; }
  if (e.altKey && !e.ctrlKey && !e.metaKey && MENU_KEYS[e.key.toLowerCase()] && !document.querySelector('.modal-back, .screen')) {
    e.preventDefault();
    openMenu(MENU_KEYS[e.key.toLowerCase()]);
    return;
  }
  if (openPopup && openPopup.anchor.classList?.contains('menu-btn') && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    const all = [...document.querySelectorAll('.menu-btn')];
    const i = all.indexOf(openPopup.anchor);
    const next = all[(i + (e.key === 'ArrowRight' ? 1 : all.length - 1)) % all.length];
    e.preventDefault();
    next.click();
  }
}, true);

function buildMenus() {
  const bar = $('menus');
  // Help comes last.
  const help = MENUS.findIndex((m) => m.label === 'Help');
  if (help >= 0) MENUS.push(MENUS.splice(help, 1)[0]);
  for (const m of MENUS) {
    const letter = Object.entries(MENU_KEYS).find(([, v]) => v === m.label)?.[0];
    const b = h('button', { class: 'menu-btn', 'aria-haspopup': 'menu', 'aria-expanded': 'false', 'aria-keyshortcuts': letter ? `Alt+${letter.toUpperCase()}` : null, title: letter ? `Alt+${letter.toUpperCase()}` : null }, m.label);
    const show = () => popup(b, limitMenu(m.label, m.items().filter((x) => x != null)).map((x) => (x === '-' ? h('hr') : x)));
    b.addEventListener('click', () => (openPopup?.anchor === b ? closePopup() : show()));
    b.addEventListener('pointerenter', () => { if (openPopup && openPopup.anchor.classList.contains('menu-btn') && openPopup.anchor !== b) show(); });
    bar.append(b);
  }
}

// ----- Tabs -----

let lastTitle = '';
function renderTabs() {
  const t = app.tabs;
  if (!t) return;
  syncAddress(t);
  // The window's title: the document's name (with a dot when it isn't saved).
  const front = t.tabs.find((d) => d.id === t.front);
  const title = front ? `${front.dirty ? '• ' : ''}${front.title} — Xenify Write` : 'Xenify Write';
  if (title !== lastTitle) {
    lastTitle = title;
    document.title = title;
    window.__TAURI__?.window?.getCurrentWindow?.().setTitle(title).catch(() => {});
  }
  // On a server's site one document is open at a time, like a page (File › New adds tabs).
  document.querySelector('.tabstrip').hidden = servedHere && t.tabs.length === 1;
  fill($('tabs'), ...t.tabs.map((d) => {
    const on = d.id === t.front;
    return h('div', {
      class: 'tab', role: 'tab', 'aria-selected': String(on), title: d.path || d.title,
      onPointerdown: (e) => { if (e.button === 0 && !e.target.closest('.close')) run('switch', { id: d.id }).then(() => pages.drawn.clear()); if (e.button === 1) closeTab(d.id); },
      onContextmenu: (e) => {
        e.preventDefault();
        const anchor = { getBoundingClientRect: () => ({ left: e.clientX, bottom: e.clientY, right: e.clientX }), setAttribute() {}, contains: () => false, classList: { contains: () => false } };
        popup(anchor, [
          item('Close', { run: () => closeTab(d.id) }),
          item('Close the others', { enabled: t.tabs.length > 1, run: async () => { for (const o of t.tabs.filter((x) => x.id !== d.id)) await closeTab(o.id); } }),
          d.path ? item('Copy the file’s location', { run: () => navigator.clipboard.writeText(d.path).then(() => toast('Copied')).catch(() => toast(d.path)) }) : null,
        ].filter(Boolean), { width: 240 });
      },
    },
    on ? h('span', { html: icon('file', 16), style: 'color: var(--accent)' }) : null,
    h('span', { class: 'name' }, d.title),
    d.remote != null ? h('span', { class: 'tab-server', html: icon('server', 14), title: `On ${server.name}`, 'aria-label': `On ${server.name}` }) : null,
    d.remote ? faces(server.here(d.id)) : null,
    d.dirty ? h('span', { class: 'dot', 'aria-label': 'Unsaved changes', title: 'Unsaved changes' }) : null,
    h('button', { class: 'close', 'aria-label': `Close ${d.title}`, html: icon('x', 14), onClick: () => closeTab(d.id) }));
  }));
}

/// The status bar's word on saving.
function saveState(s) {
  const tab = app.tabs?.tabs.find((t) => t.id === app.tabs.front);
  if (tab?.remote) {
    const others = server.here(tab.id).length;
    return others ? `Editing with ${others} other${others === 1 ? '' : 's'} · on ${server.name}` : `Saved on ${server.name} as you type`;
  }
  if (tab?.remote === false) return s.dirty ? 'Not saved yet' : `Saved on ${server.name}`;
  return s.dirty || !tab?.path ? 'Not saved yet' : 'Saved';
}

// ----- Pictures of canvases on a server (kept up to date) -----

const sid = () => server.id.replace(/[^A-Za-z0-9_]/g, '');
const linkSeen = new Map(); // node -> the version shown

/// Insert › From a server: a Draw canvas (as a picture), or something from a
/// Calc book or a plan; all of them kept up to date.
async function insertFromServer() {
  const n = await pickFile('Insert from a server', ['draw', 'calc', 'plan']);
  if (!n) return;
  if (n.details?.app === 'draw') await insertServerPicture(n);
  else if (n.details?.app === 'plan') await insertFromPlan(n);
  else if (n.details?.app === 'calc') await insertFromCalc('calc', 'server', n);
  else toast('Pick a Draw canvas, a Calc book or a plan.');
}

async function insertServerPicture(picked = null) {
  const n = picked || await pickFile('Picture from a server', 'draw');
  if (!n) return;
  try {
    const p = await server.preview(n.id);
    // Nothing drawn yet: an empty box in its place, which becomes the
    // picture once the canvas is saved in Draw.
    linkSeen.set(n.id, p ? p.version : (n.pictured || n.version));
    await run('link_insert', { server: sid(), node: n.id, data: p ? p.png : emptyPictureBox() });
    toast(p ? 'Inserted: it updates when the canvas is saved in Draw' : `Inserted: “${n.details?.name || 'the canvas'}” shows here once something is drawn on it and saved`);
  } catch (e) { toast(String(e.message || e)); }
}

/// A plain box with a picture sign (no words: it may be printed) for a
/// canvas with nothing on it yet, as a PNG in base64.
function emptyPictureBox() {
  const c = document.createElement('canvas');
  c.width = 640; c.height = 360;
  const x = c.getContext('2d');
  x.fillStyle = '#F5F5F4'; x.fillRect(0, 0, 640, 360);
  x.strokeStyle = '#D6D3D1'; x.lineWidth = 2; x.setLineDash([10, 8]); x.strokeRect(1, 1, 638, 358);
  x.setLineDash([]); x.strokeStyle = '#A8A29E'; x.lineWidth = 6; x.lineJoin = 'round';
  x.strokeRect(260, 130, 120, 100);
  x.beginPath(); x.moveTo(266, 222); x.lineTo(306, 178); x.lineTo(334, 204); x.lineTo(350, 190); x.lineTo(374, 222); x.stroke();
  x.beginPath(); x.arc(350, 158, 9, 0, Math.PI * 2); x.stroke();
  return c.toDataURL('image/png').split(',')[1];
}

// ----- Tables and charts from Calc books on a server (kept up to date) -----

const calcSeen = new Map(); // node -> the version shown

/// What a Calc book on the server shows to other documents (Calc puts it
/// there when the book is saved): { items: [...] }, or null.
async function calcSnapshot(node) {
  const p = await server.preview(node);
  if (!p) return null;
  try {
    const text = new TextDecoder().decode(Uint8Array.from(atob(p.png), (c) => c.charCodeAt(0)));
    const snap = JSON.parse(text);
    return snap.format === 'xenify-calc-link' ? { ...snap, version: p.version, name: p.name } : null;
  } catch { return null; }
}

/// Insert › From Calc (`app` 'calc') or From Plan ('plan'): both put up the same kind of snapshot.
async function insertFromCalc(app = 'calc', where = null, chosen = null) {
  const words = app === 'plan' ? { title: 'From Plan', thing: 'That plan', open: 'open it in Plan and save it once', where: 'the plan is saved in Plan', fallback: 'Plan' }
    : { title: 'From Calc', thing: 'That book', open: 'open it in Calc and save it once', where: 'the book is saved in Calc', fallback: 'Calc' };
  // A Calc book on this computer (the desktop app), or on the server.
  const desktop = app === 'calc' && !window.XENIFY_WEB;
  if (desktop && !where) {
    if (!server.ready) return insertFromCalc(app, 'file');
    modalShell('From Calc', [h('p', {}, 'Where is the book?')], [
      ['Cancel', () => true],
      ['On a Xenify server', () => { insertFromCalc(app, 'server'); return true; }],
      ['On this computer', () => { insertFromCalc(app, 'file'); return true; }, true],
    ]);
    return;
  }
  let n = null, file = null, snap;
  if (where === 'file') {
    const info = await call('save_info');
    const path = await window.__TAURI__.core.invoke('choose', { kind: 'calc', dir: info.dir });
    if (!path) return;
    try {
      file = await call('calc_file_name', { file: path });
      snap = await call('calc_file_snapshot', { file });
    } catch (e) { const m = String(e.message || e); toast(m[0].toUpperCase() + m.slice(1)); return; }
  } else {
    n = chosen || await pickFile(words.title, app);
    if (!n) return;
    try { snap = await calcSnapshot(n.id); } catch (e) { toast(String(e.message || e)); return; }
  }
  if (!snap?.items?.length) { toast(`“${n?.details?.name || snap?.name || words.thing}” has nothing to show yet: ${words.open}.`); return; }
  const kinds = [['table', 'Tables'], ['summary', 'Summaries'], ['chart', 'Charts'], ['sheet', 'Sheets'], ['task', 'Tasks']];
  let picked = snap.items.find((x) => x.kind === 'table') || snap.items[0];
  const preview = h('div', { class: 'calc-preview' });
  const list = h('div', { class: 'calc-items' });
  const showPreview = () => {
    if (picked.kind === 'task') {
      preview.replaceChildren(h('p', {}, h('span', { class: `task-chip c-${picked.color}` }, picked.text)), h('p', { class: 'hint' }, 'A chip in the text that keeps its status up to date.'));
      return;
    }
    if (picked.kind === 'chart') {
      preview.replaceChildren(picked.png ? h('img', { src: `data:image/png;base64,${picked.png}`, alt: picked.name }) : h('p', {}, 'No picture of this chart yet.'));
      return;
    }
    const rows = picked.rows.slice(0, 8);
    preview.replaceChildren(h('table', {}, rows.map((r, k) => h('tr', {}, r.map((c, j) => h(k === 0 && picked.header ? 'th' : 'td', { class: picked.numbers?.[j] && !(k === 0 && picked.header) ? 'n' : null }, c))))),
      ...(picked.rows.length > 8 ? [h('p', { class: 'hint' }, `and ${picked.rows.length - 8} more rows`)] : []));
  };
  const draw = () => {
    list.replaceChildren(...kinds.flatMap(([k, label]) => {
      const items = snap.items.filter((x) => x.kind === k);
      if (!items.length) return [];
      return [h('b', {}, label), ...items.map((x) => h('button', { class: `calc-item${x === picked ? ' on' : ''}`, onClick: () => { picked = x; draw(); showPreview(); } }, x.name, x.sheet && x.sheet !== x.name ? h('span', { class: 'hint' }, ` · ${x.sheet}`) : null))];
    }));
  };
  draw();
  showPreview();
  modalShell(`From ${snap.name || n?.details?.name || words.fallback}`, [h('div', { class: 'calc-pick' }, list, preview),
    h('p', { class: 'hint' }, `It goes in as a real table (or a picture of the chart) and updates when ${words.where}. Right-click it › Keep this copy to stop that.`)], [
    ['Cancel', () => true],
    ['Insert', async () => {
      try {
        if (file) {
          calcFileSeen.set(file, snap.version);
          if (picked.kind === 'chart') {
            if (!picked.png) { toast(`This chart has no picture yet: ${words.open.replace('once', 'once more')}.`); return false; }
            await run('calc_file_picture_insert', { file, item: picked.id });
            await copyInForServer();
          } else {
            await run('calc_link_insert', { file, item: picked.id, rows: picked.rows, header: !!picked.header, footer: !!picked.footer, numbers: picked.numbers || [] });
          }
          toast(`Inserted: it updates when ${words.where}`);
          return true;
        }
        calcSeen.set(n.id, snap.version);
        if (picked.kind === 'task') {
          await run('plan_task_insert', { server: sid(), node: n.id, task: picked.task, text: picked.text, color: picked.color });
        } else if (picked.kind === 'chart') {
          if (!picked.png) { toast(`This chart has no picture yet: ${words.open.replace('once', 'once more')}.`); return false; }
          await run('calc_picture_insert', { server: sid(), node: n.id, item: picked.id, data: picked.png });
        } else {
          await run('calc_link_insert', { server: sid(), node: n.id, item: picked.id, rows: picked.rows, header: !!picked.header, footer: !!picked.footer, numbers: picked.numbers || [] });
        }
        toast(`Inserted: it updates when ${words.where}`);
        return true;
      } catch (e) { toast(String(e.message || e)); return false; }
    }, true],
  ]);
}

/// Insert › From a server, a plan: tick what to show (its Timeline, its key
/// dates — expected, now, how far they moved — its issues, its milestones),
/// for the whole plan, a project or a group, and it all goes in at once,
/// kept up to date; or one issue as a chip in the text.
async function insertFromPlan(n) {
  let snap;
  try { snap = await calcSnapshot(n.id); } catch (e) { toast(String(e.message || e)); return; }
  const name = n.details?.name || snap?.name || 'That plan';
  if (!snap?.items?.length) { toast(`“${name}” has nothing to show yet: open it in Plan once.`); return; }
  const byId = new Map(snap.items.map((x) => [x.id, x]));
  // What each part is called in each scope: the whole plan, a project ("project-<id>"), a group ("group-<id>").
  const scopes = [{ id: '', name: 'The whole plan', parts: { timeline: 'gantt', keytable: 'key-dates', keytimeline: 'gantt-key', issues: 'breakdown', milestones: 'milestones' } }];
  for (const x of snap.items) {
    const m = /^(project|group)-(.+)$/.exec(x.id);
    if (!m) continue;
    const label = m[1] === 'project' ? x.name.replace(/ tasks$/, '') : x.name.replace(/^Group: /, '');
    scopes.push({ id: x.id, name: `${m[1] === 'project' ? 'Project' : 'Group'}: ${label}`,
      parts: { timeline: m[1] === 'project' ? `gantt-${m[2]}` : `gantt-group-${m[2]}`, issues: x.id } });
  }
  const PARTS = [['timeline', 'Timeline', 'a picture'], ['keytable', 'Key dates', 'groups and milestones: expected, now, how far they moved'],
    ['keytimeline', 'Key dates timeline', 'a picture of just the groups and milestones'], ['issues', 'Issues', 'a table: who, estimate, start, finish, status'], ['milestones', 'Milestones', 'a table']];
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem('write.planParts') || '{}'); } catch { /* none kept */ }
  const ticked = new Set(saved.parts || ['timeline', 'keytable']);
  let scope = scopes[0];
  let mode = 'parts';
  const tasks = snap.items.filter((x) => x.kind === 'task');
  let task = tasks[0] || null;

  const preview = h('div', { class: 'calc-preview plan-preview' });
  const side = h('div', { class: 'plan-parts' });
  const note = h('p', { class: 'hint' });
  const itemOf = (part) => byId.get(scope.parts[part]);
  const chosen = () => PARTS.filter(([k]) => ticked.has(k) && itemOf(k)).map(([k]) => itemOf(k));
  const tableOf = (it, max = 8) => h('div', {}, h('table', {}, it.rows.slice(0, max).map((r, k) => h('tr', {}, r.map((c, j) => h(k === 0 && it.header ? 'th' : 'td', { class: it.numbers?.[j] && !(k === 0 && it.header) ? 'n' : null }, c))))),
    it.rows.length > max ? h('p', { class: 'hint' }, `and ${it.rows.length - max} more rows`) : null);
  const showPreview = () => {
    if (mode === 'chip') {
      preview.replaceChildren(task ? h('p', {}, h('span', { class: `task-chip c-${task.color}` }, task.text)) : h('p', { class: 'hint' }, 'This plan has no issues yet.'),
        h('p', { class: 'hint' }, 'A chip in the text that keeps its status up to date.'));
      return;
    }
    const list = chosen();
    preview.replaceChildren(...(list.length ? list.map((it) => h('div', { class: 'plan-part' }, h('b', {}, it.name),
      it.kind === 'chart' ? (it.png ? h('img', { src: `data:image/png;base64,${it.png}`, alt: it.name }) : h('p', { class: 'hint' }, 'No picture yet.')) : tableOf(it)))
      : [h('p', { class: 'hint' }, 'Tick what to put in.')]));
  };
  const draw = () => {
    note.textContent = mode === 'chip' ? 'It goes in where the cursor is.'
      : 'They go in as real tables and pictures, one after the other, and update when the plan changes. Right-click one › Keep this copy to stop that.';
    const seg = h('div', { class: 'plan-mode', role: 'group', 'aria-label': 'What to insert' },
      [['parts', 'Parts of the plan'], ['chip', 'One issue']].map(([k, label]) => h('button', { class: `calc-item${mode === k ? ' on' : ''}`, 'aria-pressed': String(mode === k), onClick: () => { mode = k; draw(); showPreview(); } }, label)));
    if (mode === 'chip') {
      const sel = h('select', { class: 'field', 'aria-label': 'Issue' }, tasks.map((t, i) => h('option', { value: String(i), selected: t === task }, t.name)));
      sel.addEventListener('change', () => { task = tasks[Number(sel.value)]; showPreview(); });
      side.replaceChildren(...[seg, tasks.length ? h('label', { class: 'plan-field' }, 'Issue', sel) : null].filter(Boolean));
      return;
    }
    const of = h('select', { class: 'field', 'aria-label': 'Of' }, scopes.map((x) => h('option', { value: x.id, selected: x === scope }, x.name)));
    of.addEventListener('change', () => { scope = scopes.find((x) => x.id === of.value) || scopes[0]; draw(); showPreview(); });
    const boxes = PARTS.filter(([k]) => itemOf(k)).map(([k, label, what]) => {
      const c = h('input', { type: 'checkbox', checked: ticked.has(k) });
      c.addEventListener('change', () => { if (c.checked) ticked.add(k); else ticked.delete(k); showPreview(); });
      return h('label', { class: 'plan-tick' }, c, h('span', {}, label, h('span', { class: 'hint' }, ` · ${what}`)));
    });
    // (replaceChildren would show a null as the word.)
    side.replaceChildren(...[seg, scopes.length > 1 ? h('label', { class: 'plan-field' }, 'Of', of) : null, ...boxes,
      byId.has('key-dates') && !byId.get('key-dates').rows.slice(1).some((r) => r[1] !== '—')
        ? h('p', { class: 'hint' }, 'Key dates compare with the dates kept in Plan (Timeline › Expected dates), or with due dates.') : null].filter(Boolean));
  };
  draw();
  showPreview();
  modalShell(`From ${name}`, [h('div', { class: 'calc-pick plan-pick' }, side, preview), note], [
    ['Cancel', () => true],
    ['Insert', async () => {
      try {
        calcSeen.set(n.id, snap.version);
        if (mode === 'chip') {
          if (!task) return false;
          await run('plan_task_insert', { server: sid(), node: n.id, task: task.task, text: task.text, color: task.color });
          toast('Inserted: its status updates as the plan changes');
          return true;
        }
        const list = chosen();
        if (!list.length) { toast('Tick at least one thing to put in.'); return false; }
        try { localStorage.setItem('write.planParts', JSON.stringify({ parts: [...ticked] })); } catch { /* not kept */ }
        for (const it of list) {
          if (it.kind === 'chart') {
            if (it.png) await run('calc_picture_insert', { server: sid(), node: n.id, item: it.id, data: it.png });
          } else {
            await run('calc_link_insert', { server: sid(), node: n.id, item: it.id, rows: it.rows, header: !!it.header, footer: !!it.footer, numbers: it.numbers || [] });
          }
        }
        toast(`Inserted: ${list.length === 1 ? 'it updates' : 'they update'} when the plan changes`);
        return true;
      } catch (e) { toast(String(e.message || e)); return false; }
    }, true],
  ]);
}

/// Tables from Calc books on this computer that were saved since they were
/// shown (their charts are pictures read from the file, like Draw canvases).
const calcFileSeen = new Map();
async function refreshCalcFiles() {
  const links = (await call('calc_links').catch(() => [])).filter((l) => l.file);
  for (const file of new Set(links.map((l) => l.file))) {
    const snap = await call('calc_file_snapshot', { file }).catch(() => null);
    if (!snap || calcFileSeen.get(file) === snap.version) continue;
    const first = !calcFileSeen.has(file);
    calcFileSeen.set(file, snap.version);
    for (const l of links.filter((x) => x.file === file)) {
      const item = snap.items.find((x) => x.id === l.item);
      if (item && item.kind !== 'chart') await run('calc_link_update', { file, item: item.id, rows: item.rows, header: !!item.header, footer: !!item.footer, numbers: item.numbers || [] });
    }
    // Its charts: read from the file again.
    if (!first) { for (const it of snap.items) if (it.kind === 'chart') forgetPicture(`${file}#${it.id}`); pages.repaintAll(); }
  }
}
// Books saved in Calc while this window is beside it show up too.
if (!window.XENIFY_WEB) setInterval(() => { if (document.visibilityState === 'visible') refreshCalcFiles(); }, 4000);

/// Tables and charts from Calc books that changed since they were shown.
async function refreshCalcLinks() {
  const chips = (await call('plan_task_links')).filter((l) => l.server === sid());
  const links = [...(await call('calc_links')).filter((l) => l.server === sid()), ...chips.map((c) => ({ ...c, chip: true }))];
  const nodes = [...new Set(links.map((l) => l.node))];
  for (const node of nodes) {
    const n = await server.api('GET', `/v1/nodes/${node}`).catch(() => null);
    if (!n || calcSeen.get(node) === (n.pictured || n.version)) continue;
    const snap = await calcSnapshot(node).catch(() => null);
    if (!snap) continue;
    calcSeen.set(node, snap.version);
    for (const l of links.filter((x) => x.node === node)) {
      if (l.chip) {
        const it = snap.items.find((x) => x.kind === 'task' && x.task === l.task);
        if (it) await run('plan_task_update', { server: sid(), node, task: l.task, text: it.text, color: it.color });
        continue;
      }
      const want = l.item || l.picture;
      const item = snap.items.find((x) => x.id === want || x.id.replace(/[^A-Za-z0-9_]/g, '') === want);
      if (!item) continue;
      if (l.picture && item.png) {
        const out = await run('calc_picture_update', { server: sid(), node, item: item.id, data: item.png });
        if (out?.key) { forgetPicture(out.key); pages.repaintAll(); }
      } else if (l.item) {
        await run('calc_link_update', { server: sid(), node, item: item.id, rows: item.rows, header: !!item.header, footer: !!item.footer, numbers: item.numbers || [] });
      }
    }
  }
}

/// Fetches pictures of canvases that changed since they were shown.
let refreshing = false;
async function refreshLinks() {
  if (refreshing || !server.ready || !server.id) return;
  refreshing = true;
  try {
    for (const l of await call('links')) {
      if (l.server !== sid()) continue;
      const n = await server.api('GET', `/v1/nodes/${l.node}`).catch(() => null);
      if (!n || linkSeen.get(l.node) === (n.pictured || n.version)) continue;
      const p = await server.preview(l.node).catch(() => null);
      if (!p) continue;
      linkSeen.set(l.node, p.version);
      const out = await run('link_update', { server: sid(), node: l.node, data: p.png });
      if (out?.key) { forgetPicture(out.key); pages.repaintAll(); }
    }
    await refreshCalcLinks();
  } finally { refreshing = false; }
}
window.addEventListener('focus', () => refreshLinks());
setInterval(() => { if (document.visibilityState === 'visible') refreshLinks(); }, 20000);

// ----- Pictures in server documents -----

// Changes sent while editing together don't carry pictures: one added here
// goes up by itself (part of the document now, not a version), and one
// someone else added is fetched by itself (else it would show as an empty
// box); a picture only in a saved version comes from that version.
let picturing = false;
async function syncPictures() {
  if (picturing || !server.ready) return;
  picturing = true;
  const at = (d, name) => `/v1/nodes/${d.node}/pictures/${name.replace(/^media\//, '').split('/').map(encodeURIComponent).join('/')}`;
  const b64 = (data) => {
    let s = '';
    for (let i = 0; i < data.length; i += 0x8000) s += String.fromCharCode(...data.subarray(i, i + 0x8000));
    return btoa(s);
  };
  try {
    for (const [id, d] of server.docs) {
      if (!d.joined) continue;
      const need = await call('remote_needs', { id }).catch(() => null);
      if (!need) continue;
      const sent = [];
      for (const name of need.upload) {
        const p = await call('remote_picture', { id, name }).catch(() => null);
        if (!p) continue;
        const bytes = Uint8Array.from(atob(p.data), (c) => c.charCodeAt(0));
        if (await server.api('PUT', at(d, name), bytes).then(() => true, () => false)) sent.push(name);
      }
      if (sent.length) await call('remote_uploaded', { id, names: sent }).catch(() => {});
      if (!need.missing.length) continue;
      const taken = [];
      let older = false;
      for (const name of need.missing) {
        const data = await server.api('GET', at(d, name), undefined, { raw: true }).catch(() => null);
        if (!data) { older = true; continue; }
        const out = await call('remote_take_picture', { id, name, data: b64(data) }).catch(() => null);
        if (out) taken.push(name);
      }
      if (older) {
        const data = await server.api('GET', `/v1/nodes/${d.node}/content`, undefined, { raw: true }).catch(() => null);
        const out = data && await call('remote_take_pictures', { id, data: b64(data) }).catch(() => null);
        if (out?.taken) taken.push(...out.taken);
      }
      if (taken.length) { taken.forEach(forgetPicture); pages.repaintAll(); }
    }
  } finally { picturing = false; }
}
setInterval(syncPictures, 3000);

// ----- Where the others editing a server document are -----

/// A person's colour (the same for their face and their caret).
function colourOf(name) {
  const colours = ['#0F766E', '#1D4ED8', '#7C3AED', '#BE185D', '#B45309'];
  return colours[[...name].reduce((a, c) => a + c.charCodeAt(0), 0) % colours.length];
}

// Tell the others where this window's caret is when it moves.
let lastSaid = '';
let sayTimer = null;
function sayWhere() {
  const id = app.tabs?.front;
  if (!server.docs.get(id)?.joined || !pages.view) return;
  const now = JSON.stringify([id, pages.view.caret, pages.view.selection]);
  if (now === lastSaid) return;
  lastSaid = now;
  // At most every 120 ms, the latest place (typing keeps it moving).
  if (sayTimer) return;
  sayTimer = setTimeout(async () => {
    sayTimer = null;
    try { server.say(id, (await call('live_say', { id })).data); } catch { /* not on a server any more */ }
  }, 120);
}

// Show theirs, placed afresh (the text may have moved).
let placing = false;
let placeAgain = false;
async function placeOthers() {
  if (placing) { placeAgain = true; return; }
  placing = true;
  try {
    const list = [];
    for (const s of server.saidOf(app.tabs?.front)) {
      const r = await call('live_heard', { data: s.data }).catch(() => null);
      if (r) list.push({ name: s.who.name, colour: colourOf(s.who.name), caret: r.caret, selection: r.selection });
    }
    pages.showOthers(list);
  } finally {
    placing = false;
    if (placeAgain) { placeAgain = false; placeOthers(); }
  }
}
pages.onMarks = () => { sayWhere(); if (server.saidOf(app.tabs?.front).length || pages.others.length) placeOthers(); };
server.onSaid = (id, why) => {
  if (id !== app.tabs?.front) return;
  // Someone new: they need to know where this caret is too.
  if (why === 'arrived') { lastSaid = ''; sayWhere(); }
  placeOthers();
};

/// Who else has a server document open: their initials, in their colours.
function faces(people) {
  if (!people.length) return null;
  const names = people.map((p) => p.name).join(', ');
  return h('span', { class: 'faces', 'aria-label': `Here now: ${names}`, title: `Here now: ${names}` },
    people.slice(0, 4).map((p) => h('span', { class: 'face', style: `background: ${colourOf(p.name)}` },
      p.name.split(/\s+/).slice(0, 2).map((w) => w[0]?.toUpperCase()).join(''))));
}

/// Whether tab `id` is a server document: saved as it's typed, so there's
/// nothing to ask about when it closes.
const onServerDoc = (id) => id != null && server.nodeOf(id) != null;

/// File › Discard my changes (a server document): everything this window
/// changed since it was opened is undone, for everyone; the saved
/// document follows. A page that's empty then can go.
async function discardMine() {
  const id = app.tabs.front;
  const title = app.tabs.tabs.find((x) => x.id === id)?.title || 'this document';
  const c = await ask(`Discard your changes to “${title}”?`, 'What you changed since you opened it is undone. Changes other people made stay.',
    [['discard', 'Discard my changes', 'main'], ['cancel', 'Cancel']]);
  if (c !== 'discard') return;
  for (let i = 0; i < 100000 && st().can_undo; i++) await run('undo');
  await server.versionNow();
  pages.drawn.clear();
  pages.draw();
  if (!st().words) {
    const node = server.nodeOf(id);
    const d = await ask(`“${title}” is empty now`, 'Delete it? It goes in the bin, where it can be put back.', [['delete', 'Delete it', 'main'], ['keep', 'Keep it']]);
    if (d === 'delete' && node != null) {
      try { await server.api('DELETE', `/v1/nodes/${node}`); } catch (e) { toast(e.message); return; }
      server.leave(id);
      await run('close', { id });
      if (EMBED) return void window.parent.postMessage({ xenify: 'write', t: 'deleted' }, location.origin);
      if (WEB) return void (await server.quitToFiles());
    }
  }
  toast('Your changes were discarded.');
}

async function closeTab(id) {
  const t = app.tabs.tabs.find((x) => x.id === id);
  if (onServerDoc(id)) await server.versionNow();
  else if (t?.dirty) {
    if (id !== app.tabs.front) await run('switch', { id });
    const choice = await ask(`Save changes to “${t.title}”?`, 'If you don’t, what you changed since the last save is lost.',
      [['save', 'Save', 'main'], ['discard', 'Don’t save'], ['cancel', 'Cancel']]);
    if (choice === 'cancel' || !choice) return;
    if (choice === 'save' && !(await save())) return;
  }
  const last = app.tabs.tabs.length === 1;
  server.leave(id);
  await run('close', { id });
  // The last one closed: nothing left to do here (back to Files in the browser).
  if (last && (!WEB || servedHere)) return void (await quit());
  pages.drawn.clear();
  pages.draw();
}

// ----- Files -----

function recentItems() {
  const files = (app.prefs?.recent || []).slice(0, 8);
  if (!files.length) return [];
  return [h('div', { class: 'menu-label' }, 'Recent'), ...files.map((f) => item(f.split('/').pop(), { run: () => openPath(f), title: f }))];
}

async function openPath(path) {
  const out = await run('open', { path });
  if (out) { pages.drawn.clear(); pages.draw(); app.prefs = await call('prefs'); }
}

async function openFile() {
  const info = await call('save_info');
  const path = await window.__TAURI__.core.invoke('choose', { kind: 'open', dir: info.dir });
  if (path) await openPath(path);
}

/// Saves the front document; asks where for a new one. Returns whether it was saved.
async function save() {
  // On a server it saves itself as you type: Save sends what's waiting
  // (no new version each time; a checkpoint keeps a moment).
  const remote = await call('remote_info');
  if (remote) {
    try { await server.versionNow(app.tabs?.front); toast(`Saved on ${server.name}: it saves itself as you type. File › Save a checkpoint… keeps it as it is now.`); return true; } catch (e) { toast(String(e.message || e)); return false; }
  }
  const info = await call('save_info');
  // In the browser on a Xenify server, a new document (one made from a template, say) is saved there,
  // not downloaded as a file.
  if (!info.has_path && WEB && servedHere && server.ready) return !!(await saveToServer(app.tabs?.tabs.find((d) => d.id === app.tabs.front)?.title));
  if (!info.has_path) return saveAs();
  if (app.prefs.ask_change_note && st().dirty && !(await askChange())) return false;
  return !!(await run('save'));
}

/// What changed, for the change log (asked when saving, or File › Record a change).
async function askChange() {
  const { summary } = await call('change_suggest');
  return new Promise((resolve) => {
    const box = h('textarea', { class: 'field area', rows: 3, 'aria-label': 'What changed' });
    box.value = summary;
    const { close } = modalShell('What changed?', [h('p', {}, 'Goes in the document’s change log with your name and the date.'), box], [
      ['Skip', () => { resolve(true); return true; }],
      ['Cancel', () => { resolve(false); return true; }],
      ['Record it', async () => { if (box.value.trim()) await run('record_change', { summary: box.value }); resolve(true); return true; }, true],
    ]);
    void close;
    box.focus();
    box.select();
  });
}

/// Keeps the document as one of the person's own templates (New document lists them first).
function saveTemplate() {
  const name = h('input', { class: 'field', 'aria-label': 'Template name', placeholder: 'e.g. Weekly report' });
  const front = app.tabs?.tabs.find((d) => d.id === app.tabs.front);
  name.value = front && !/^Untitled/.test(front.title) ? front.title.replace(/\.[a-z]+$/i, '') : '';
  const msg = h('p', { class: 'set-desc', role: 'alert' });
  modalShell('Save as template', [
    h('p', {}, 'File › New from template will offer it, with everything in it now: text, styles, page setup, headers, footers and pictures.'),
    h('p', { class: 'hint-text' }, 'Type {{Your name}} (any words in double braces) where something should be filled in: it becomes a box to type over. Documents made from the template keep its styles, page setup, headers and footers; change them with File › Edit a template.'),
    name, msg], [
    ['Cancel', () => true],
    ['Save template', async () => {
      try { const r = await call('save_template', { name: name.value }); toast(`Saved “${r.name}” as a template`); return true; }
      catch (e) { msg.textContent = String(e); return false; }
    }, true],
  ]);
  name.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); name.closest('.modal').querySelector('.btn.main').click(); } });
  name.focus();
  name.select();
}

async function saveAs() {
  const info = await call('save_info');
  let path = await window.__TAURI__.core.invoke('choose', { kind: 'save', name: info.name, dir: info.dir });
  if (!path) return false;
  if (!/\.[a-z0-9]+$/i.test(path.split('/').pop())) path += `.${info.ext}`;
  const ext = path.split('.').pop().toLowerCase();
  if (ext === 'pdf') return exportTo(path);
  const lost = await call('losses', { ext }).catch(() => []);
  if (lost.length) {
    const choice = await ask(`Saving as .${ext} leaves some things out`, `Not kept: ${lost.join(', ')}. A Xenify Write document (.xwrite) keeps everything.`,
      [['save', `Save as .${ext} anyway`, 'main'], ['xwrite', 'Save as .xwrite'], ['cancel', 'Cancel']]);
    if (choice === 'cancel' || !choice) return false;
    if (choice === 'xwrite') path = path.replace(/\.[^.]+$/, '.xwrite');
  }
  const out = await run('save', { path });
  if (out) app.prefs = await call('prefs');
  return !!out;
}

async function exportAs(ext) {
  const info = await call('save_info');
  let path = await window.__TAURI__.core.invoke('choose', { kind: 'export', name: `${info.stem}.${ext}`, dir: info.dir });
  if (!path) return;
  if (!path.toLowerCase().endsWith(`.${ext}`)) path += `.${ext}`;
  await exportTo(path);
}

async function exportTo(path) {
  try {
    await call('export', { path });
    toast(`Exported ${path.split('/').pop()}`);
    return true;
  } catch (e) {
    toast(String(e));
    return false;
  }
}

/// Asks about everything unsaved, then closes Write (`restart`: and starts
/// the newer version Xenify Hub installed).
async function quit(restart = false) {
  // Server documents are saved as they're typed: the last typing goes now.
  await server.versionNow();
  for (const t of await call('dirty_tabs')) {
    if (onServerDoc(t.id)) continue;
    await run('switch', { id: t.id });
    pages.drawn.clear();
    const choice = await ask(`Save changes to “${t.title}” before closing?`, 'If you don’t, what you changed since the last save is lost.',
      [['save', 'Save', 'main'], ['discard', 'Don’t save'], ['cancel', 'Cancel']]);
    if (choice === 'cancel' || !choice) return;
    if (choice === 'save' && !(await save())) return;
  }
  // Everything was saved or deliberately not: no unsaved work to offer back.
  await call('discard_all_autosaves');
  // In the browser, from a Xenify server: back to its Files page.
  leaving = true;
  if (WEB) return void (await server.quitToFiles());
  window.__TAURI__.core.invoke(restart ? 'restart_updated' : 'quit');
}

// Xenify Hub installed a newer version: Help offers to restart into it.
let updateReady = null;
async function checkUpdate() {
  if (window.XENIFY_WEB) return;
  try { updateReady = await window.__TAURI__.core.invoke('update_ready'); } catch { updateReady = null; }
}
checkUpdate();
window.addEventListener('focus', () => checkUpdate());
setInterval(() => checkUpdate(), 120000);
window.__TAURI__?.event.listen('close-requested', () => quit());

/// A question that needs an answer: resolves to the chosen button's id
/// (or null with Esc).
function ask(title, text, buttons) {
  return new Promise((resolve) => {
    closePopup();
    const back = h('div', { class: 'modal-back' });
    const done = (v) => { back.remove(); unkey(); resolve(v); pages.focus(); };
    // Escape: no answer; Enter: the main button.
    const unkey = dialogKeys(back, () => done(null));
    const box = h('div', { class: 'modal', role: 'alertdialog', 'aria-modal': 'true', 'aria-labelledby': 'ask-title', 'aria-describedby': 'ask-text' },
      h('h2', { id: 'ask-title' }, title),
      h('p', { id: 'ask-text' }, text),
      h('div', { class: 'modal-actions' }, buttons.slice().reverse().map(([id, label, kind]) => h('button', { class: `btn${kind ? ` ${kind}` : ''}`, onClick: () => done(id) }, label))));
    back.append(box);
    document.body.append(back);
    box.querySelector('.btn.main')?.focus();
  });
}

// ----- Toolbar -----

const tb = {};
function buildToolbar() {
  const bar = $('toolbar');
  const btn = (name, label, keys, act, content) => {
    const b = h('button', { class: 'icon-btn', 'aria-label': label, title: keys ? `${label} (${keys})` : label, html: content || icon(name), onClick: act });
    b.addEventListener('mousedown', (e) => e.preventDefault()); // keep the caret
    tb[name] = b;
    return b;
  };
  const sep = () => h('span', { class: 'sep' });

  tb.style = h('button', { class: 'pick', style: 'width: 132px', 'aria-label': 'Paragraph style', 'aria-haspopup': 'menu', onClick: () => stylePicker() },
    h('span', { class: 'label' }, 'Body text'), h('span', { html: icon('down', 14) }));
  tb.font = h('button', { class: 'pick', style: 'width: 140px; margin-left: 6px', 'aria-label': 'Font', 'aria-haspopup': 'menu', onClick: () => fontPicker() },
    h('span', { class: 'label' }, ''), h('span', { html: icon('down', 14) }));
  tb.size = h('input', { class: 'size-box', 'aria-label': 'Font size', inputmode: 'decimal', style: 'margin-left: 6px' });
  tb.size.addEventListener('change', () => { const v = parseFloat(tb.size.value); if (v > 0) run('size', { pt: v }); });
  tb.size.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); tb.size.blur(); } });

  bar.append(
    btn('undo', 'Undo', 'Ctrl+Z', () => run('undo')),
    btn('redo', 'Redo', 'Ctrl+Y', () => run('redo')),
    sep(), tb.style, tb.font, tb.size, sep(),
    btn('bold', 'Bold', 'Ctrl+B', () => run('toggle', { what: 'bold' }), '<span style="font-weight: 800">B</span>'),
    btn('italic', 'Italic', 'Ctrl+I', () => run('toggle', { what: 'italic' }), '<span style="font-style: italic; font-family: Georgia, serif; font-size: 17px">I</span>'),
    btn('underline', 'Underline', 'Ctrl+U', () => run('toggle', { what: 'underline' }), '<span style="text-decoration: underline; text-underline-offset: 3px">U</span>'),
    btn('colour', 'Text colour', null, () => colourPicker(), '<span style="display: flex; flex-direction: column; align-items: center; line-height: 1">A<span class="colour-bar" style="background: var(--accent)"></span></span>'),
    btn('highlighter', 'Highlight', null, () => colourPicker('highlight')),
    sep(),
    btn('alignLeft', 'Align left', null, () => run('align', { to: 'left' })),
    btn('alignCenter', 'Centre', null, () => run('align', { to: 'center' })),
    btn('alignRight', 'Align right', null, () => run('align', { to: 'right' })),
    btn('alignJustify', 'Justify', null, () => run('align', { to: 'justify' })),
    btn('spacing', 'Spacing and indents', null, () => spacingPicker()),
    sep(),
    btn('bullets', 'Bulleted list', null, () => run('list', { kind: 'bullets' })),
    btn('numbers', 'Numbered list', null, () => run('list', { kind: 'numbers' })),
    btn('checklist', 'Checklist', null, () => run('list', { kind: 'checklist' })),
    btn('outdent', 'Less indent', 'Shift+Tab', () => indentBy(-1)),
    btn('indent', 'More indent', 'Tab', () => indentBy(1)),
    sep(),
    btn('link', 'Link', 'Ctrl+K', () => linkEditor()),
    btn('image', 'Picture', null, () => insertPicture()),
    btn('table', 'Table', null, () => tablePicker(tb.table)),
    btn('comment', 'Comment', 'Ctrl+Alt+M', () => newComment()),
    btn('omega', 'Symbols & emoji', 'Ctrl+.', () => symbolsDialog()),
    sep(),
    btn('eraser', 'Clear formatting', 'Ctrl+Space', () => run('clear_format')),
  );
}

/// Review › Kind of writing: what spelling and grammar checking points out
/// in this document (a school essay is stricter about slang than a chat).
async function kindEditor() {
  const d = await call('details');
  const kind = h('select', { class: 'field', id: 'dt-kind' }, d.profiles.map((p) => h('option', { value: p.id, selected: p.id === d.profile }, p.name)));
  const about = h('p', { class: 'hint-text' }, d.profiles.find((p) => p.id === d.profile)?.about || '');
  kind.addEventListener('change', () => { about.textContent = d.profiles.find((p) => p.id === kind.value)?.about || ''; });
  modalShell('Kind of writing', h('div', { class: 'details-pop' },
    h('p', { class: 'hint-text' }, 'Decides what spelling and grammar checking points out in this document.'),
    h('div', {}, h('label', { class: 'lbl', for: 'dt-kind' }, 'Check it as'), kind, about)), [
    ['Cancel', null],
    ['Save', async () => { await run('set_details', { profile: kind.value }); proofSoon(0); }, true],
  ]);
  kind.focus();
}

/// Who Ask AI writes for and in what tone (only Ask AI uses these); `then` after saving or cancelling.
async function readersEditor(then = null) {
  const d = await call('details');
  const reader = h('input', { class: 'field', id: 'dt-reader', value: d.reader || '', placeholder: 'e.g. product and engineering' });
  const tone = h('div', { class: 'segs', role: 'radiogroup', 'aria-label': 'Tone' });
  let toneV = d.tone || 'Neutral';
  const drawTone = () => fill(tone, ...['Formal', 'Neutral', 'Friendly'].map((x) => h('button', {
    role: 'radio', 'aria-checked': String(x === toneV), class: `seg${x === toneV ? ' on' : ''}`, onClick: () => { toneV = x; drawTone(); },
  }, x)));
  drawTone();
  const { body } = modalShell('Who it’s for', h('div', { class: 'details-pop' },
    h('p', { class: 'hint-text' }, 'Ask AI writes for these readers in this tone. Nothing else uses them.'),
    h('div', {}, h('label', { class: 'lbl', for: 'dt-reader' }, 'Who will read it'), reader),
    h('div', {}, h('div', { class: 'lbl' }, 'Tone'), tone)), [
    // Opened from Ask AI: Cancel goes back there too.
    ['Cancel', () => { then?.(); }],
    ['Save', async () => { await run('set_details', { reader: reader.value, tone: toneV }); then?.(); }, true],
  ]);
  reader.focus();
}

const COLOURS = ['#1C1917', '#57534E', '#A8A29E', '#FFFFFF', '#B91C1C', '#C2410C', '#B45309', '#4D7C0F',
  '#047857', '#0E7490', '#1D4ED8', '#6D28D9', '#A21CAF', '#BE185D', '#7C2D12', '#1E3A8A'];
const HIGHLIGHTS = ['#FEF08A', '#BBF7D0', '#BAE6FD', '#FBCFE8', '#FED7AA', '#DDD6FE', '#E7E5E4', '#FECACA'];
const SHADES = ['#F5F5F4', '#FFF7ED', '#FEFCE8', '#F0FDF4', '#EFF6FF', '#FAF5FF', '#FDF2F8', '#E7E5E4'];
/// Text colour (the default), highlight or paragraph shading. Below the
/// usual ones, the document's own colours and + for any other (it joins
/// them, so it's there to pick again; right-click one to take it off).
async function colourPicker(kind = 'text') {
  const list = kind === 'highlight' ? HIGHLIGHTS : kind === 'shade' ? SHADES : COLOURS;
  const act = (c) => (kind === 'highlight' ? run('highlight', c ? { hex: c } : {}) : kind === 'shade' ? run('para_look', c ? { what: 'shade', hex: c } : { what: 'no_shade' }) : run('color', { hex: c }));
  const names = { text: 'Text colour', highlight: 'Highlight', shade: 'Shading' };
  const own = (await call('details').catch(() => null))?.colors || [];
  const swatch = (c, extra = {}) => h('button', {
    class: 'swatch item', style: `background: ${c}; padding: 0; min-height: 0`, 'aria-label': c, title: c,
    onClick: () => { closePopup(); act(c); }, ...extra,
  });
  const other = h('input', { type: 'color', class: 'swatch-input', 'aria-label': 'Another colour', value: own[0] || '#c2410c' });
  other.addEventListener('change', async () => {
    closePopup();
    await run('add_color', { hex: other.value });
    act(other.value);
  });
  const add = h('label', { class: 'swatch item swatch-add', title: 'Another colour: it’s kept with this document', html: icon('plus', 14) }, other);
  popup(kind === 'text' ? tb.colour : kind === 'highlight' ? tb.highlighter : tb.spacing, [h('div', { class: 'menu-label' }, names[kind]),
    h('div', { class: 'swatches' }, list.map((c) => swatch(c))),
    h('div', { class: 'menu-label' }, 'This document’s colours'),
    h('div', { class: 'swatches' }, ...own.map((c) => swatch(c, { onContextmenu: (e) => {
      e.preventDefault();
      closePopup();
      run('remove_color', { hex: c }).then(() => toast(`${c} taken off this document’s colours`));
    } })), add),
    kind !== 'text' ? item('None', { run: () => act(null) }) : null]);
}

function styleName(id) { return app.styles.find((s) => s.id === id)?.name || id; }

function stylePicker(anchor = tb.style) {
  const cur = st().style;
  const keysOf = (id) => STYLE_KEYS[id] || null;
  popup(anchor, [...app.styles.filter((s) => !['header', 'footer', 'footnote-text'].includes(s.id)).map((s) => {
    const size = s.level ? Math.max(14, 22 - s.level * 2) : 14;
    return item(s.name, { checked: s.id === cur, keys: keysOf(s.id), run: () => run('style', { id: s.id }), style: `font-size: ${size}px; font-weight: ${s.level || s.id === 'title' ? 700 : 400}` });
  }),
  h('hr'),
  // A numbered style: every paragraph in it is numbered, continuing (1, 2, 3; headings 1, 1.1).
  ...(() => {
    const def = app.styles.find((x) => x.id === cur);
    if (!def || ['title', 'subtitle', 'caption'].includes(cur)) return [];
    const numbered = !!def.list && def.list !== 'bullets';
    const toggle = (list) => async () => { await run('style_list', { id: cur, list }); app.styles = await call('styles'); };
    return [
      item(`Number every ${styleName(cur)} paragraph`, { checked: numbered, title: def.level ? '1, 1.1, 1.1.1 by heading level' : '1, 2, 3 through the document', run: toggle(numbered ? null : 'numbers') }),
      def.level ? null : item(`Bullet every ${styleName(cur)} paragraph`, { checked: def.list === 'bullets', run: toggle(def.list === 'bullets' ? null : 'bullets') }),
    ].filter(Boolean);
  })(),
  item(`Update ${styleName(cur)} to match the selection`, { run: () => styleFromSelection(cur) }),
  item('New style from the selection…', { run: () => styleFromSelection(null) })], { width: 340 });
}

/// Fonts to write in: not the maths symbols one.
const textFonts = (list) => list.filter((f) => !/^STIX Two Math$/i.test(f));

function fontPicker() {
  const cur = st().font;
  const list = h('div');
  const search = h('input', { class: 'search', placeholder: 'Find a font, e.g. Inter', 'aria-label': 'Find a font' });
  const drawList = () => {
    const q = search.value.trim().toLowerCase();
    const shown = app.fonts.filter((f) => !q || f.toLowerCase().includes(q)).slice(0, 80);
    fill(list, ...shown.map((f) => item(f, { checked: f === cur, run: () => run('font', { name: f }), style: `font-family: "${f}", Figtree, sans-serif; font-size: 15px` })));
  };
  search.addEventListener('input', drawList);
  search.addEventListener('keydown', (e) => { if (e.key === 'Enter') list.querySelector('.item')?.click(); });
  drawList();
  popup(tb.font, [search, list], { width: 280 });
}

function refreshToolbar() {
  const s = st();
  tb.undo.disabled = !s.can_undo;
  tb.redo.disabled = !s.can_redo;
  syncLimited();
  tb.style.querySelector('.label').textContent = styleName(s.style);
  // A style added since (a panel, a template's): the list is fetched again.
  if (s.style && !app.styles.some((x) => x.id === s.style) && !app.stylesLoading) {
    app.stylesLoading = true;
    call('styles').then((list) => { app.styles = list; app.stylesLoading = false; refreshToolbar(); });
  }
  tb.font.querySelector('.label').textContent = s.font || '';
  tb.font.querySelector('.label').style.fontFamily = `"${s.font}", Figtree, sans-serif`;
  if (document.activeElement !== tb.size) tb.size.value = s.size ? +s.size.toFixed(1) : '';
  for (const k of ['bold', 'italic', 'underline']) tb[k].setAttribute('aria-pressed', String(!!s[k]));
  for (const [k, v] of [['alignLeft', 'left'], ['alignCenter', 'center'], ['alignRight', 'right'], ['alignJustify', 'justify']]) {
    tb[k].setAttribute('aria-pressed', String(s.align === v));
  }
  for (const k of ['bullets', 'numbers', 'checklist']) tb[k].setAttribute('aria-pressed', String(s.list === k));
}

// ----- Popups next to the text -----

const WEB = !!window.XENIFY_WEB;
// Inside the server's Files page (editing a page where it's read): no menu
// bar or tabs, and Files asks for Done and Discard.
const EMBED = WEB && new URLSearchParams(location.search).get('embed') === '1' && window.parent !== window;
if (EMBED) {
  document.documentElement.classList.add('embed');
  window.addEventListener('message', async (e) => {
    if (e.origin !== location.origin || e.source !== window.parent) return;
    const tell = (t) => window.parent.postMessage({ xenify: 'write', t }, location.origin);
    if (e.data?.t === 'done') {
      await server.versionNow();
      tell('saved');
    } else if (e.data?.t === 'discard') {
      if (!st().can_undo) return toast('Nothing to discard: you haven’t changed it.');
      await discardMine();
    }
  });
}
// Closing the browser tab with unsaved work of this computer asks first
// (server documents are saved as they're typed).
let leaving = false;
if (WEB) window.addEventListener('beforeunload', (e) => {
  if (!leaving && app.tabs?.tabs.some((t) => t.dirty && !onServerDoc(t.id))) { e.preventDefault(); e.returnValue = ''; }
});


/// A point on screen to hang a popup from.
function anchorAt(left, bottom, top = bottom) {
  return { getBoundingClientRect: () => ({ left, right: left, bottom, top }), setAttribute() {}, contains: () => false, classList: { contains: () => false } };
}
/// Where document-space rect `r` ([x, y, w, h]) is on screen.
function screenRect(r) {
  const z = pages.zoom, sr = pages.stack.getBoundingClientRect();
  const left = sr.left + pages.originX + r[0] * z, top = sr.top + pages.originY + r[1] * z;
  return { left, top, right: left + r[2] * z, bottom: top + r[3] * z };
}
/// Below the selection (or the caret).
function selectionAnchor() {
  const v = app.view;
  const r = v?.selection?.length ? v.selection[v.selection.length - 1] : v?.caret;
  if (!r) return anchorAt(window.innerWidth / 2, window.innerHeight / 3);
  const at = screenRect(r);
  return anchorAt(at.left, at.bottom, at.top);
}

// ----- Ask AI: a dialog, with the signed-in Xenify server's AI -----

function openAi() {
  if (!hasAi()) return void noAi(toast, showSettings);
  aiDialog({ h, fill, run, ask, modalShell, hasSelection: app.view?.selected_text != null, selectedText: app.view?.selected_text || '',
    details: app.details || {}, tab: app.tabs?.front, changeDetails: () => readersEditor(() => openAi()) });
}

// ----- Symbols & emoji: a dialog; click one to type it -----

const sym = { data: null, tab: 'symbols', query: '', recent: [] };
async function symbolsDialog() {
  if (!sym.data) sym.data = await call('symbols');
  let shell = null;
  const search = h('input', { class: 'field', type: 'search', 'aria-label': 'Find a symbol' });
  const tabs = h('div', { class: 'segs', role: 'radiogroup', 'aria-label': 'Kind' });
  const recent = h('div');
  const grid = h('div', { class: 'sym-grid', role: 'list' });
  const put = async (ch) => {
    sym.recent = [ch, ...sym.recent.filter((x) => x !== ch)].slice(0, 16);
    shell.close();
    await pages.send('text', { text: ch });
    pages.focus();
  };
  const cell = (ch, name, isEmoji) => {
    const b = h('button', { class: `sym${isEmoji ? ' emoji' : ''}`, title: name, 'aria-label': name, role: 'listitem', onClick: () => put(ch) });
    if (isEmoji) {
      const img = h('img', { alt: '', width: 26, height: 26 });
      call('emoji_svg', { glyph: ch }).then((svg) => { if (svg) img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`; });
      b.append(img);
    } else b.append(ch);
    return b;
  };
  const drawGrid = () => {
    const q = sym.query.trim().toLowerCase();
    const out = [];
    if (sym.tab === 'symbols') {
      for (const g of sym.data.symbols) {
        const items = g.items.filter(([c, n]) => !q || n.includes(q) || c === q);
        if (!items.length) continue;
        out.push(h('div', { class: 'sym-group' }, g.group), ...items.map(([c, n]) => cell(c, n, false)));
      }
    } else {
      const emo = sym.data.emoticons.filter(([t]) => q && t.startsWith(q)).map(([, e]) => e);
      let last = '';
      let n = 0;
      for (const e of sym.data.emoji) {
        const hit = !q || e.name.toLowerCase().includes(q) || e.keywords.some((k) => k.toLowerCase().includes(q)) || emo.includes(e.glyph);
        if (!hit) continue;
        if (e.group !== last && !q) { out.push(h('div', { class: 'sym-group' }, e.group)); last = e.group; }
        out.push(cell(e.glyph, e.name, true));
        if (++n > 400) break;
      }
    }
    fill(grid, ...(out.length ? out : [h('p', { class: 'nav-empty' }, `Nothing called “${sym.query}”.`)]));
  };
  const drawAll = () => {
    search.placeholder = sym.tab === 'emoji' ? 'e.g. smile, heart, :)' : 'e.g. arrow, euro, degree';
    fill(tabs, ...[['symbols', 'Symbols'], ['emoji', 'Emoji']].map(([id, n]) => h('button', {
      role: 'radio', 'aria-checked': String(sym.tab === id), class: `seg${sym.tab === id ? ' on' : ''}`, onClick: () => { sym.tab = id; drawAll(); search.focus(); } }, n)));
    fill(recent, sym.recent.length ? [h('div', { class: 'sym-group' }, 'Recent'), h('div', { class: 'sym-grid' }, sym.recent.map((c) => cell(c, c, sym.data.emoji.some((e) => e.glyph === c))))] : null);
    drawGrid();
  };
  search.addEventListener('input', () => { sym.query = search.value; drawGrid(); });
  search.addEventListener('keydown', (e) => { if (e.key === 'Enter') grid.querySelector('.sym')?.click(); });
  drawAll();
  shell = modalShell('Symbols & emoji', h('div', { class: 'sym-dialog' }, h('div', { class: 'sym-head' }, tabs, search), h('div', { class: 'sym-scroll' }, recent, grid)), [['Cancel', null]]);
  search.focus();
}

// ----- Comments and tracked changes: notes in the margin beside the page -----

async function refreshReview() {
  app.review = await call('review');
  placeNotes();
  if (check.open && check.mode !== 'spelling') renderCheck();
}
let reviewTimer = null;
function refreshReviewSoon() { clearTimeout(reviewTimer); reviewTimer = setTimeout(refreshReview, 300); }

async function toggleTracking() {
  await run('set_tracking', { on: !app.review?.tracking });
  await refreshReview();
  toast(app.review.tracking ? 'Tracking changes: what you type and delete is marked' : 'No longer tracking changes');
}
async function decide(accept, all) {
  const out = await run('decide', { accept, all });
  if (out) toast(out.decided ? `${accept ? 'Accepted' : 'Rejected'} ${out.decided} ${out.decided === 1 ? 'change' : 'changes'}` : 'No change here');
  refreshReview();
}

/// A comment on the selection (or the word at the cursor), typed in a box next to it.
function newComment() {
  const v = app.view;
  const quote = v?.selected_text?.trim();
  const box = h('textarea', { class: 'field area', rows: 3, placeholder: 'Your comment', 'aria-label': 'Your comment' });
  const add = async () => { const t = box.value.trim(); if (!t) return; closePopup(); await run('add_comment', { text: t }); await refreshReview(); };
  box.addEventListener('keydown', (e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); add(); } });
  popup(selectionAnchor(), h('div', { class: 'link-pop', role: 'dialog', 'aria-label': 'New comment' },
    h('label', { class: 'lbl' }, quote ? `Comment on “${quote.length > 40 ? `${quote.slice(0, 40)}…` : quote}”` : 'Comment on the word at the cursor'), box,
    h('div', { class: 'pop-actions' }, h('span', { class: 'hint' }, 'Ctrl+Enter'),
      h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: add }, 'Comment'))), { width: 320 });
  box.focus();
}

// The notes: one card per open comment and tracked change, level with its
// text (pushed down so they don't overlap). Comments are tinted on the page.
const notes = h('div', { class: 'notes' });
pages.overlay.append(notes);
pages.onLayout = () => { placeNotes(); placeContents(); placeGuides(); placeTables(); };
const NOTE_ROOM = 300;
function placeNotes() {
  const r = app.review;
  const shown = r ? [...r.comments.filter((c) => c.y != null && (!c.resolved || app.showResolved)).map((c) => ({ ...c, kind: 'comment', key: `c${c.id}` })),
    ...r.changes.filter((c) => c.y != null).slice(0, 80).map((c, k) => ({ ...c, note: c.kind, kind: 'change', key: `t${k}` }))] : [];
  const room = shown.length ? NOTE_ROOM : 0;
  if ((pages.rightRoom || 0) !== room) { pages.rightRoom = room; pages.layoutPages(); return; }
  const v = app.view;
  if (!v) return;
  const z = pages.zoom;
  const pageRight = pages.originX + v.width * z;
  const x = pageRight + 20;
  shown.sort((a, b) => a.y - b.y);
  const btn = (label, act, main) => h('button', { class: `btn small${main ? ' main' : ''}`, onClick: (e) => { e.stopPropagation(); act(); } }, label);
  const select = (c) => run('select_range', { para: c.para, start: c.start, end: c.end });
  const at = (q) => [pages.originX + q[0] * z, pages.originY + q[1] * z, q[2] * z, q[3] * z];
  const tints = [];
  const cards = shown.map((c) => {
    const on = app.activeNote === c.key;
    if (c.kind === 'comment') {
      for (const q of c.rects || []) {
        const [l, t, w, hh] = at(q);
        tints.push(h('div', { class: `comment-tint${on ? ' on' : ''}`, 'data-key': c.key, style: `left: ${l}px; top: ${t}px; width: ${w}px; height: ${hh}px` }));
      }
      const reply = h('input', { class: 'field', placeholder: 'Reply…', 'aria-label': `Reply to ${c.author || 'comment'}` });
      reply.addEventListener('keydown', async (e) => { if (e.key === 'Enter' && reply.value.trim()) { await run('reply', { id: c.id, text: reply.value }); refreshReview(); } });
      return [c, h('div', { class: `note comment${on ? ' on' : ''}${c.resolved ? ' resolved' : ''}`, role: 'note', 'data-key': c.key,
        onClick: (e) => { if (e.target.closest('button, input')) return; app.activeNote = c.key; select(c); placeNotes(); } },
        h('div', { class: 'who' }, h('b', {}, c.author || 'Someone'), c.date ? h('span', {}, c.date) : null),
        c.quote ? h('div', { class: 'quote' }, `On “${c.quote.length > 60 ? `${c.quote.slice(0, 60)}…` : c.quote}”`) : null,
        h('div', { class: 'msg' }, c.text),
        c.replies.map((x) => h('div', { class: 'reply' }, h('b', {}, x.author), ` ${x.text}`)),
        c.resolved ? null : reply,
        h('div', { class: 'acts' },
          btn(c.resolved ? 'Reopen' : 'Resolve', async () => { await run('resolve', { id: c.id, resolved: !c.resolved }); refreshReview(); }),
          btn('Delete', async () => { await run('delete_comment', { id: c.id }); refreshReview(); })))];
    }
    return [c, h('div', { class: `note change ${c.note}${on ? ' on' : ''}`, role: 'note', 'data-key': c.key,
      onClick: (e) => { if (e.target.closest('button')) return; app.activeNote = c.key; select(c); placeNotes(); } },
      h('div', { class: 'who' }, h('b', {}, c.author || 'Someone'), h('span', {}, c.note === 'inserted' ? 'added' : 'deleted')),
      h('div', { class: 'ctx' }, c.note === 'inserted' ? h('u', {}, c.is_break ? 'a new paragraph' : c.text) : h('s', {}, c.is_break ? 'a paragraph break' : c.text)),
      h('div', { class: 'acts' },
        btn('Accept', async () => { await run('decide', { accept: true, para: c.para, start: c.start, end: c.end }); refreshReview(); }, true),
        btn('Reject', async () => { await run('decide', { accept: false, para: c.para, start: c.start, end: c.end }); refreshReview(); })))];
  });
  // A line from the end of each one's text to its note (so two comments side by side stay clear).
  const svgNS = 'http://www.w3.org/2000/svg';
  const lines = document.createElementNS(svgNS, 'svg');
  lines.setAttribute('class', 'note-lines');
  fill(notes, tints, lines, cards.map(([, el]) => el));
  let below = 0;
  for (const [c, el] of cards) {
    const top = Math.max(pages.originY + c.y * z, below);
    Object.assign(el.style, { left: `${x}px`, top: `${top}px` });
    below = top + el.offsetHeight + 8;
    const last = (c.rects || []).at(-1);
    const [tx, ty] = last ? [at(last)[0] + at(last)[2], at(last)[1] + at(last)[3]] : [pageRight - 40, pages.originY + c.y * z + 14];
    const path = document.createElementNS(svgNS, 'path');
    path.setAttribute('d', `M ${tx} ${ty} L ${pageRight + 6} ${ty} L ${x} ${top + 16}`);
    path.setAttribute('class', `${c.kind}${app.activeNote === c.key ? ' on' : ''}`);
    path.dataset.key = c.key;
    lines.append(path);
  }
  // Pointing at a note lights up its text and line.
  for (const el of notes.querySelectorAll('.note')) {
    el.addEventListener('pointerdown', (e) => e.stopPropagation());
    const light = (on) => { for (const t of notes.querySelectorAll(`[data-key="${el.dataset.key}"]`)) t.classList.toggle('hover', on); };
    el.addEventListener('mouseenter', () => light(true));
    el.addEventListener('mouseleave', () => light(false));
  }
}

// ----- Styles: the picker in the toolbar, with keys and typed shortcuts -----

/// Ctrl+Alt+0 is body text, Ctrl+Alt+1–3 the headings.
const STYLE_KEYS = { normal: 'Ctrl+Alt+0', heading1: 'Ctrl+Alt+1', heading2: 'Ctrl+Alt+2', heading3: 'Ctrl+Alt+3' };

async function styleFromSelection(id) {
  if (id) {
    await run('style_from_selection', { id });
    toast(`${styleName(id)} now looks like the selection`);
  } else {
    const input = h('input', { class: 'field', placeholder: 'e.g. Quote with a line', 'aria-label': 'Style name' });
    const go = async () => { const n = input.value.trim(); closePopup(); if (n) { await run('style_from_selection', { name: n }); app.styles = await call('styles'); } };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    popup(tb.style, h('div', { class: 'link-pop' }, h('label', { class: 'lbl' }, 'New style from the selection'), input,
      h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: go }, 'Make style'))), { width: 320 });
    input.focus();
  }
  app.styles = await call('styles');
}

// ----- Spelling and grammar: a bar like find, one thing at a time on the page -----

// `mode`: 'spelling', or going through tracked 'changes' or 'comments' the same way.
const check = { open: false, at: 0, mode: 'spelling' };
function checkItems() {
  if (check.mode === 'changes') return app.review?.changes || [];
  if (check.mode === 'comments') return (app.review?.comments || []).filter((c) => !c.resolved);
  return app.issues || [];
}
function openCheck(mode = 'spelling') {
  check.open = true;
  check.mode = mode;
  if (find.open) closeFind();
  check.at = 0;
  renderCheck();
  showIssue();
  if (mode === 'spelling') proofSoon(0);
  else refreshReview().then(() => { renderCheck(); showIssue(); });
}
function closeCheck() { check.open = false; app.activeNote = null; renderCheck(); placeNotes(); pages.focus(); }
function showIssue() {
  const i = checkItems()[check.at];
  if (!i) return;
  if (check.mode === 'spelling') {
    app.activeIssue = i.i;
    run('select_issue', { i: i.i });
    return;
  }
  app.activeNote = check.mode === 'comments' ? `c${i.id}` : `t${app.review.changes.indexOf(i)}`;
  run('select_range', { para: i.para, start: i.start, end: i.end }).then(() => placeNotes());
}
function stepIssue(back) {
  const n = checkItems().length;
  if (!n) return;
  check.at = (check.at + (back ? n - 1 : 1)) % n;
  renderCheck();
  showIssue();
}
/// The bar for going through tracked changes or comments.
function renderReviewBar(bar, close, btn, nav) {
  const list = checkItems();
  const changes = check.mode === 'changes';
  if (!list.length) {
    fill(bar, h('span', { class: 'check-ok', html: icon('check', 18) }), h('span', { class: 'check-msg' }, changes ? 'No tracked changes left.' : 'No open comments.'),
      h('span', { class: 'spacer', style: 'flex: 1 1 0' }), close);
    return;
  }
  check.at = Math.min(check.at, list.length - 1);
  const c = list[check.at];
  const after = async () => { await refreshReview(); renderCheck(); showIssue(); };
  if (changes) {
    fill(bar,
      h('div', { class: `check-what ${c.kind === 'inserted' ? 'added' : 'removed'}` }, h('span', { class: 'dot' }), c.kind === 'inserted' ? 'Added' : 'Deleted'),
      h('div', { class: 'check-text' },
        h('div', { class: 'ctx' }, c.kind === 'inserted' ? h('u', {}, c.is_break ? 'a new paragraph' : c.text) : h('s', {}, c.is_break ? 'a paragraph break' : c.text)),
        h('div', { class: 'msg' }, `${c.author || 'Someone'}${c.date ? `, ${c.date}` : ''}`)),
      h('div', { class: 'check-acts' },
        btn('Accept', async () => { await run('decide', { accept: true, para: c.para, start: c.start, end: c.end }); after(); }, true),
        btn('Reject', async () => { await run('decide', { accept: false, para: c.para, start: c.start, end: c.end }); after(); }),
        btn('Accept all', async () => { await decide(true, true); after(); })),
      h('div', { class: 'check-nav' }, nav(true), h('span', { class: 'count', 'aria-live': 'polite' }, `${check.at + 1} of ${list.length}`), nav(false)), close);
    return;
  }
  const reply = h('input', { class: 'field', placeholder: 'Reply…', 'aria-label': 'Reply' });
  reply.addEventListener('keydown', async (e) => { if (e.key === 'Enter' && reply.value.trim()) { await run('reply', { id: c.id, text: reply.value }); after(); } });
  fill(bar,
    h('div', { class: 'check-what comment' }, h('span', { class: 'dot' }), c.author || 'Comment'),
    h('div', { class: 'check-text' },
      h('div', { class: 'ctx' }, c.text),
      h('div', { class: 'msg' }, c.quote ? `On “${c.quote.length > 60 ? `${c.quote.slice(0, 60)}…` : c.quote}”` : '', c.replies.length ? ` · ${c.replies.length} ${c.replies.length === 1 ? 'reply' : 'replies'}` : '')),
    h('div', { class: 'check-acts' }, reply,
      btn('Resolve', async () => { await run('resolve', { id: c.id, resolved: true }); after(); }, true),
      btn('Delete', async () => { await run('delete_comment', { id: c.id }); after(); })),
    h('div', { class: 'check-nav' }, nav(true), h('span', { class: 'count', 'aria-live': 'polite' }, `${check.at + 1} of ${list.length}`), nav(false)), close);
}
function renderCheck() {
  const bar = $('checkbar');
  bar.hidden = !check.open;
  if (!check.open) return;
  const close = h('button', { class: 'icon-btn', 'aria-label': 'Close', title: 'Close (Esc)', html: icon('x', 18), onClick: closeCheck });
  const kind = h('button', { class: 'link', title: 'What gets pointed out (Review › Kind of writing)', onClick: () => kindEditor() }, `Checked as ${app.details?.profile_name || 'Standard'}`);
  const keep = (b) => { b.addEventListener('mousedown', (e) => e.preventDefault()); return b; };
  const btn = (label, act, main) => keep(h('button', { class: `btn${main ? ' main' : ''}`, onClick: act }, label));
  const nav = (back) => keep(h('button', { class: 'icon-btn', 'aria-label': back ? 'Previous' : 'Next', title: back ? 'Previous' : 'Next',
    html: back ? '<svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m15 18-6-6 6-6"/></svg>'
      : '<svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m9 18 6-6-6-6"/></svg>',
    onClick: () => stepIssue(back) }));
  if (check.mode !== 'spelling') { renderReviewBar(bar, close, btn, nav); return; }
  if (!app.prefs?.proofing?.enabled) {
    fill(bar, h('span', { class: 'check-msg' }, 'Spelling and grammar checking is turned off.'), btn('Turn it on', () => setProof('enabled', true), true), h('span', { class: 'spacer', style: 'flex: 1 1 0' }), close);
    return;
  }
  const list = app.issues || [];
  if (!list.length) {
    fill(bar, h('span', { class: 'check-ok', html: `${icon('check', 18)}` }), h('span', { class: 'check-msg' }, app.checking ? 'Checking…' : 'Nothing to look at. Nice writing.'),
      h('span', { class: 'spacer', style: 'flex: 1 1 0' }), kind, close);
    return;
  }
  check.at = Math.min(check.at, list.length - 1);
  const i = list[check.at];
  const same = list.filter((x) => x.message === i.message && x.text === i.text);
  fill(bar,
    h('div', { class: `check-what ${i.kind}` }, h('span', { class: 'dot' }), i.label),
    h('div', { class: 'check-text' },
      h('div', { class: 'ctx' }, i.before ? `…${i.before.trimStart()}` : '', h('mark', {}, i.text), i.after ? `${i.after.trimEnd()}…` : ''),
      h('div', { class: 'msg' }, quoted(i.message))),
    h('div', { class: 'check-acts' },
      i.suggestions.slice(0, 3).map((sug, k) => btn(sug === '' ? 'Remove it' : sug, () => fixIssue(i.i, sug), k === 0)),
      btn('Ignore', () => ignoreIssue(i.i)),
      same.length > 1 ? btn(`Ignore all ${same.length}`, async () => { for (const x of same) await call('ignore', { i: x.i }); await run('view'); await refreshIssues(); showIssue(); }) : null,
      i.spelling ? btn('Add to my words', () => addWord(i.text)) : null,
      i.spelling ? btn('Add to this document', () => addDocWord(i.text)) : null),
    h('div', { class: 'check-nav' }, nav(true), h('span', { class: 'count', 'aria-live': 'polite' }, `${check.at + 1} of ${list.length}`), nav(false)),
    kind, close);
}
$('checkbar').addEventListener('keydown', (e) => {
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeCheck(); }
});

async function fixIssue(n, text) {
  await run('fix', { i: n, with: text });
  await proofNow();
  if (check.open) showIssue();
}
async function ignoreIssue(n) {
  await run('ignore', { i: n });
  await refreshIssues();
  if (check.open) showIssue();
}
async function addWord(word) {
  try {
    await words.set([...(await words.get()), word]);
  } catch (e) { toast(String(e.message || e)); return; }
  toast(words.where() ? `Added “${word}” to your words on ${words.where()}` : `Added “${word}” to your words`);
  await proofNow();
  if (check.open) showIssue();
}
/// Right in this document only (kept in it, so everyone editing it shares it).
async function addDocWord(word) {
  await run('doc_word', { word });
  toast(`Added “${word}” to this document’s words`);
  await proofNow();
  if (check.open) showIssue();
}
async function setProof(key, value) {
  applyPrefs(await call('set_prefs', { prefs: { ...app.prefs, proofing: { ...app.prefs.proofing, [key]: value } } }));
  proofSoon(0);
}

// Checks after a pause in typing, and when switching documents.
let proofTimer = null, proofRev = null, proofDoc = null;
async function proofNow() {
  clearTimeout(proofTimer);
  app.checking = true;
  try {
    await window.__TAURI__.core.invoke('proofread');
    const out = await call('view', { have: pages.have() });
    if (out) { pages.setMarks(out.marks); }
    await refreshIssues();
  } catch (e) {
    console.error('proofread', e);
  } finally {
    app.checking = false;
    renderCheck();
  }
}
function proofSoon(delay = 450) {
  clearTimeout(proofTimer);
  proofTimer = setTimeout(proofNow, delay);
}
async function refreshIssues() {
  app.issues = await call('issues');
  renderCheck();
  renderStatus();
}
function maybeProof(view) {
  const doc = app.tabs?.front;
  if (view.rev !== proofRev || doc !== proofDoc) {
    const switched = doc !== proofDoc;
    proofRev = view.rev;
    proofDoc = doc;
    proofSoon(switched ? 0 : 450);
  }
}

// ----- Find and replace -----

const find = { open: false, replace: false, query: '', with: '', case: false, whole: false, count: 0, current: null };
function openFind(replace) {
  find.open = true;
  find.replace = replace || find.replace;
  // The selected text is what you're looking for, as in a browser.
  const sel = app.view?.selected_text;
  if (sel && !sel.includes('\n') && sel.length < 80) find.query = sel;
  renderFind();
  const input = $('findbar').querySelector('input');
  input.focus();
  input.select();
  if (find.query) doFind(true);
}
function closeFind() {
  find.open = false;
  renderFind();
  run('find_close');
}
async function doFind(stay = false) {
  const out = await run('find', { query: find.query, case: find.case, whole: find.whole, stay });
  if (out?.find) Object.assign(find, out.find);
  renderFindCount();
  $('findbar').querySelector('input')?.focus();
}
async function step(back) {
  const out = await run('find_next', { back });
  if (out?.find) Object.assign(find, out.find);
  renderFindCount();
}
function renderFindCount() {
  const el = $('findbar').querySelector('.count');
  if (!el) return;
  el.textContent = !find.query ? '' : find.count === 0 ? 'No matches' : find.current == null ? `${find.count} found` : `${find.current + 1} of ${find.count}`;
}
function renderFind() {
  const bar = $('findbar');
  bar.hidden = !find.open;
  if (!find.open) return;
  const q = h('input', { class: 'field', type: 'search', placeholder: 'Find in the document', 'aria-label': 'Find', value: find.query });
  let timer;
  q.addEventListener('input', () => { find.query = q.value; clearTimeout(timer); timer = setTimeout(() => doFind(), 150); });
  q.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey); }
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeFind(); pages.focus(); }
  });
  const opt = (label, key) => {
    const c = h('input', { type: 'checkbox', checked: find[key] });
    c.addEventListener('change', () => { find[key] = c.checked; doFind(); });
    return h('label', { class: 'opt' }, c, label);
  };
  const w = h('input', { class: 'field', placeholder: 'Replace with', 'aria-label': 'Replace with', value: find.with });
  w.addEventListener('input', () => { find.with = w.value; });
  w.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); run('replace', { with: find.with }).then((o) => { if (o?.find) Object.assign(find, o.find); renderFindCount(); }); }
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeFind(); pages.focus(); }
  });
  fill(bar, ...[
    q, h('span', { class: 'count', 'aria-live': 'polite' }),
    h('button', { class: 'icon-btn', 'aria-label': 'Previous match', title: 'Previous (Shift+Enter)', html: '<svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m18 15-6-6-6 6"/></svg>', onClick: () => step(true) }),
    h('button', { class: 'icon-btn', 'aria-label': 'Next match', title: 'Next (Enter)', html: icon('down', 18), onClick: () => step(false) }),
    opt('Match case', 'case'), opt('Whole words', 'whole'),
    find.replace ? [w,
      h('button', { class: 'btn', onClick: () => run('replace', { with: find.with }).then((o) => { if (o?.find) Object.assign(find, o.find); renderFindCount(); }) }, 'Replace'),
      h('button', { class: 'btn', onClick: async () => { const o = await run('replace_all', { with: find.with }); if (o) { toast(`Replaced ${o.replaced} ${o.replaced === 1 ? 'time' : 'times'}`); Object.assign(find, o.find || {}); renderFindCount(); } } }, 'Replace all')]
      : h('button', { class: 'link', onClick: () => { find.replace = true; renderFind(); } }, 'Replace…'),
    h('span', { class: 'spacer', style: 'flex: 1 1 0' }),
    h('button', { class: 'icon-btn', 'aria-label': 'Close find', title: 'Close (Esc)', html: icon('x', 18), onClick: () => { closeFind(); pages.focus(); } })].flat());
  renderFindCount();
}

// ----- Print -----

async function printDialog() {
  const { printers, why_not: whyNot } = await call('printers');
  const back = h('div', { class: 'modal-back' });
  const done = () => { back.remove(); unkey(); pages.focus(); };
  // Escape closes, Enter prints (or opens the PDF viewer where printing can't be done).
  const unkey = dialogKeys(back, done);
  const printer = h('select', { class: 'field', id: 'pr-printer', disabled: !printers.length }, printers.length ? printers.map((p) => h('option', { value: p }, p)) : h('option', {}, 'No printers found'));
  const copies = h('input', { class: 'field', id: 'pr-copies', type: 'number', min: 1, max: 999, value: 1 });
  const range = h('input', { class: 'field', id: 'pr-pages', placeholder: 'All pages, or e.g. 1-3, 5' });
  const viewer = async () => { done(); try { await call('print', {}); } catch (e) { toast(String(e)); } };
  const go = async () => {
    done();
    try {
      await call('print', { printer: printer.value, copies: Number(copies.value) || 1, pages: range.value });
      toast(`Sent to ${printer.value}`);
    } catch (e) { toast(String(e)); }
  };
  back.append(h('div', { class: 'modal wide', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'pr-title' },
    h('h2', { id: 'pr-title' }, 'Print'),
    whyNot ? h('p', {}, `Printing isn’t possible here: ${whyNot}. You can open the document as a PDF and print it from there.`) : null,
    !whyNot ? h('div', { class: 'form-grid' },
      h('div', { style: 'grid-column: 1 / -1' }, h('label', { class: 'lbl', for: 'pr-printer' }, 'Printer'), printer),
      h('div', {}, h('label', { class: 'lbl', for: 'pr-copies' }, 'Copies'), copies),
      h('div', {}, h('label', { class: 'lbl', for: 'pr-pages' }, 'Pages'), range)) : null,
    h('div', { class: 'modal-actions', style: 'margin-top: 18px' },
      h('button', { class: 'btn', onClick: done }, 'Cancel'),
      h('button', { class: `btn${whyNot ? ' main' : ''}`, onClick: viewer }, 'Open in PDF viewer'),
      !whyNot ? h('button', { class: 'btn main', onClick: go, disabled: !printers.length }, 'Print') : null)));
  document.body.append(back);
  (back.querySelector('.btn.main') || printer).focus();
}

// ----- Equations: a visual editor (boxes to type in), LaTeX underneath as
// an advanced option; the page shows it as you type -----

/// [button, what it adds (a structure or text), its name].
const EQ_PALETTE = [
  ['a⁄b', 'frac', 'Fraction'], ['x²', 'sup', 'Power'], ['xᵢ', 'sub', 'Small, lowered'], ['√x', 'sqrt', 'Square root'], ['ⁿ√x', 'root', 'Root'],
  ['∑', 'sum', 'Sum'], ['∫', 'int', 'Integral'], ['( )', 'paren', 'Brackets'], ['|x|', 'abs', 'Absolute value'], ['π', 'π', 'pi'],
  ['α', 'α', 'alpha'], ['β', 'β', 'beta'], ['θ', 'θ', 'theta'], ['Δ', 'Δ', 'Delta'], ['±', '±', 'plus or minus'],
  ['×', '×', 'times'], ['÷', '÷', 'divided by'], ['≤', '≤', 'at most'], ['≥', '≥', 'at least'], ['≠', '≠', 'not equal'],
];

async function equationEditor(point) {
  const found = await call('equation_get', point || {});
  if (point && !found) return false;
  const original = found ? { ...found } : null;
  let at = found ? { para: found.para, offset: found.offset } : null;
  const latexBox = h('textarea', { class: 'field area mono', rows: 3, 'aria-label': 'Equation in LaTeX' });
  latexBox.value = found ? found.latex : '';
  const flat = h('p', { class: 'eq-flat', 'aria-live': 'polite' });
  const note = h('p', { class: 'hint-text' });
  const advanced = h('details', { class: 'eq-advanced' }, h('summary', {}, 'Advanced: edit as LaTeX'),
    h('p', { class: 'hint-text' }, 'For anything the buttons can’t do.'), latexBox, note);
  const display = h('input', { type: 'checkbox', id: 'eq-display', checked: found ? found.display : false });
  const box = h('div', { class: 'eq-box', role: 'group', 'aria-label': 'Equation' });
  let timer = null;
  const put = async () => {
    if (!latexBox.value.trim() && !at) return;
    const out = await run('equation_set', { latex: latexBox.value, display: display.checked, ...(at || {}) });
    if (out?.at) at = out.at;
  };
  const showFlat = async () => {
    const easy = latexBox.value.trim() ? await call('math_to_easy', { latex: latexBox.value }) : '';
    flat.textContent = easy ? `Written out: ${easy}` : '';
  };
  const soon = () => { clearTimeout(timer); timer = setTimeout(() => { put(); showFlat(); }, 200); };
  const ed = mathEditor(box, latexBox.value, (latex) => { latexBox.value = latex; note.textContent = ''; soon(); });
  if (found && !ed.latex() && found.latex.trim()) {
    advanced.open = true;
    note.textContent = 'This equation uses LaTeX the boxes can’t show, so it’s edited as LaTeX.';
  }
  latexBox.addEventListener('input', () => {
    note.textContent = ed.load(latexBox.value) ? '' : 'The boxes can’t show all of this; it’s kept as you wrote it.';
    clearTimeout(timer);
    timer = setTimeout(() => { put(); showFlat(); latexBox.focus(); }, 300);
  });
  display.addEventListener('change', soon);
  const palette = h('div', { class: 'eq-palette' }, EQ_PALETTE.map(([label, what, name]) => {
    const b = h('button', { class: 'sym', title: name, 'aria-label': name, onClick: () => ed.insert(what) }, label);
    b.addEventListener('mousedown', (e) => e.preventDefault());
    return b;
  }));
  const back = h('div', { class: 'modal-back clear' });
  const done = () => { clearTimeout(timer); back.remove(); document.removeEventListener('keydown', keys, true); unkey(); pages.focus(); };
  const cancel = async () => {
    done();
    if (!at) return;
    if (original) await run('equation_set', { latex: original.latex, display: original.display, ...at });
    else await run('equation_remove', at);
  };
  // Escape cancels, Enter is Done; in the boxes too (Enter makes nothing there), and Ctrl+Enter from anywhere.
  const unkey = dialogKeys(back, cancel);
  const keys = (e) => {
    if (e.key === 'Enter' && !e.isComposing && !e.altKey && (e.ctrlKey || e.metaKey || box.contains(e.target))) { e.preventDefault(); e.stopPropagation(); done(); }
  };
  document.addEventListener('keydown', keys, true);
  back.append(h('div', { class: 'modal wide eq', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'eq-title' },
    h('h2', { id: 'eq-title' }, found ? 'Edit equation' : 'Equation'),
    h('p', {}, 'Click a button, then click into its boxes and type. Typing ^ makes a power, / a fraction. Tab moves to the next box.'),
    palette, box, flat,
    h('label', { class: 'check-line', for: 'eq-display' }, display, 'On a line of its own, centred'),
    advanced,
    h('div', { class: 'modal-actions', style: 'margin-top: 16px' },
      found ? h('button', { class: 'btn', onClick: async () => { done(); await run('equation_remove', at); } }, 'Delete') : null,
      h('span', { style: 'flex: 1 1 0' }),
      h('button', { class: 'btn', onClick: cancel }, 'Cancel'), h('button', { class: 'btn main', onClick: done }, 'Done'))));
  document.body.append(back);
  showFlat();
  if (advanced.open) latexBox.focus(); else ed.focus();
  return true;
}

onStacks('dblclick', async (e) => {
  const p = pages.docPoint(e);
  await equationEditor(p);
});

// ----- Pictures: click to select, drag the corner to resize -----

const picSel = { el: null, info: null };
function clearPicture() { picSel.el?.remove(); picSel.el = null; picSel.info = null; }
function showPicture(info) {
  clearPicture();
  picSel.info = info;
  const z = pages.zoom;
  const [x, y, w, hgt] = info.rect;
  const box = h('div', { class: 'pic-sel', style: `left: ${pages.originX + x * z}px; top: ${pages.originY + y * z}px; width: ${w * z}px; height: ${hgt * z}px` });
  const handle = h('div', { class: 'pic-handle', title: 'Drag to resize' });
  box.append(handle);
  // An animated Draw canvas: play it here (Draw makes the clip the first time).
  if (info.animated && info.canvas && !WEB) addPlay(box, info, () => window.__TAURI__.core.invoke('canvas_clip', { path: info.canvas }));
  // One on a server: Write fetches it and Draw makes the clip the same way.
  const onServer = /^media\/xen-(\w+)-(\d+)\.png$/.exec(info.src || '');
  if (onServer && onServer[1] === sid() && !WEB) {
    serverCanvas(+onServer[2]).then((c) => {
      if (c?.animated && picSel.info?.id === info.id && picSel.el === box) addPlay(box, info, () => window.__TAURI__.core.invoke('canvas_clip', { path: '', data: c.data }));
    });
  }
  pages.overlay.append(box);
  picSel.el = box;
  handle.addEventListener('pointerdown', (e) => {
    e.preventDefault(); e.stopPropagation();
    handle.setPointerCapture(e.pointerId);
    const start = { x: e.clientX, w: w * z, h: hgt * z };
    const move = (ev) => {
      const nw = Math.max(16, start.w + ev.clientX - start.x);
      box.style.width = `${nw}px`;
      box.style.height = `${nw * start.h / start.w}px`;
    };
    const up = async () => {
      handle.removeEventListener('pointermove', move);
      const nw = box.offsetWidth / z, nh = box.offsetHeight / z;
      clearPicture();
      await run('image_set', { id: info.id, w: nw, h: nh });
      const again = await call('image_at', { x: x + 1, y: y + 1 });
      if (again) showPicture(again);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up, { once: true });
  });
}
/// The Play button on an animated Draw canvas: `clip()` gives the clip
/// (WebM, base64).
function addPlay(box, info, clip) {
  // Lucide play and square (ISC licence).
  const PLAY = '<svg aria-hidden="true" width="20" height="20" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="1.75" stroke-linejoin="round"><path d="M7 4.5v15l12-7.5z"/></svg>';
  const STOP = '<svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>';
  const play = h('button', { class: 'pic-play', 'aria-label': 'Play the animation', title: 'Play the animation' });
  play.innerHTML = PLAY;
  let video = null;
  play.addEventListener('pointerdown', (e) => e.stopPropagation());
  play.addEventListener('click', async (e) => {
    e.stopPropagation();
    if (video) { video.remove(); video = null; play.innerHTML = PLAY; play.setAttribute('aria-label', 'Play the animation'); return; }
    play.disabled = true;
    play.classList.add('busy');
    play.title = 'Xenify Draw is making the clip';
    try {
      const b64 = await clip();
      if (!picSel.el || picSel.info?.id !== info.id) return;
      const url = URL.createObjectURL(new Blob([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], { type: 'video/webm' }));
      video = h('video', { class: 'pic-video', src: url, autoplay: true, loop: true, muted: true, playsinline: true, 'aria-label': 'The animation' });
      box.insertBefore(video, play);
      play.innerHTML = STOP;
      play.setAttribute('aria-label', 'Stop the animation');
      play.title = 'Stop';
    } catch (err) {
      play.title = 'Play the animation';
      toast(String(err));
    } finally { play.disabled = false; play.classList.remove('busy'); }
  });
  box.append(play);
}

/// Whether Draw canvas `node` on the server moves, and (if it does) the
/// canvas as a file with its pictures and clips (base64) for Draw to make
/// the clip from; kept per version.
const canvases = new Map();
async function serverCanvas(node) {
  const n = await server.api('GET', `/v1/nodes/${node}`).catch(() => null);
  if (!n) return null;
  const had = canvases.get(node);
  if (had?.version === n.version) return had;
  const doc = await server.api('GET', `/v1/nodes/${node}/content`, undefined, { raw: true }).catch(() => null);
  if (!doc) return null;
  // Keyframes are kept as "animated": [...].
  const animated = /"animated"\s*:\s*\[/.test(new TextDecoder().decode(doc));
  const file = animated ? await server.api('GET', `/v1/nodes/${node}/content?file=true`, undefined, { raw: true }).catch(() => null) : null;
  let s = '';
  for (let i = 0; file && i < file.length; i += 0x8000) s += String.fromCharCode(...file.subarray(i, i + 0x8000));
  const c = { version: n.version, animated: !!file, data: btoa(s) };
  canvases.set(node, c);
  return c;
}

onStacks('pointerdown', async (e) => {
  if (e.button !== 0 || e.target.closest('.pic-sel')) return;
  const p = pages.docPoint(e);
  const info = await call('image_at', p);
  if (info) showPicture(info); else clearPicture();
}, true);
window.addEventListener('keydown', async (e) => {
  if (!picSel.info) return;
  if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); e.stopPropagation(); const id = picSel.info.id; clearPicture(); await run('delete_block', { id }); }
  else if (e.key === 'Escape') clearPicture();
}, true);
onStacks('contextmenu', async (e) => {
  const info = await call('image_at', pages.docPoint(e));
  if (!info) return;
  e.stopImmediatePropagation();
  e.preventDefault();
  showPicture(info);
  const anchor = { getBoundingClientRect: () => ({ left: e.clientX, bottom: e.clientY, right: e.clientX }), setAttribute() {}, contains: () => false, classList: { contains: () => false } };
  popup(anchor, [
    item('Picture options…', { run: () => pictureOptions(info.id, anchor) }),
    item(info.caption ? 'Remove the caption' : 'Add a caption', { run: () => run('image_caption', { id: info.id, place: info.caption ? null : 'bottom' }) }),
    h('hr'),
    /^media\/xenc?-/.test(info.src || '')
      ? item('Keep this copy (stop updating)', { title: 'It then stays as it is now, even when the original on the server changes', run: () => run('link_keep', { id: info.id }) })
      : info.linked
      ? item('Copy it into the document', { title: 'It then stays as it is now, even if the file changes', run: () => run('image_link', { id: info.id, embed: true }) })
      : WEB ? null : item('Show it from a file instead…', { title: 'The picture then updates when the file does', run: async () => {
        const path = await window.__TAURI__.core.invoke('choose', { kind: 'picture', dir: (await call('save_info')).dir });
        if (path) await run('image_link', { id: info.id, path });
      } }),
    item('Description for screen readers…', { run: () => {
      const input = h('input', { class: 'field', value: info.alt, placeholder: 'e.g. A chart of sales by month', 'aria-label': 'Description' });
      const go = () => { closePopup(); run('image_set', { id: info.id, alt: input.value }); };
      input.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') go(); });
      popup(anchor, h('div', { class: 'link-pop' }, h('label', { class: 'lbl' }, 'What the picture shows'), input,
        h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: go }, 'Save'))), { width: 340 });
      input.focus();
    } }),
    item('Delete picture', { run: () => { clearPicture(); run('delete_block', { id: info.id }); } }),
  ], { width: 280 });
}, true);

/// Picture options: in line with the text or with text around it (in a box
/// like a text box), where it sits, a caption above or below, and a line.
async function pictureOptions(id) {
  let info = null;
  // Found again by id (it may have moved into or out of a box).
  const reload = async () => { info = await call('image_info', { id }); };
  await reload();
  if (!info) return;
  let changes = 0;
  const step = async (name, args) => { if (await run(name, { id, ...args })) changes += 1; await reload(); draw(); };
  const body = h('div', { class: 'pick-pop' });
  const wrapPic = (kind) => {
    // Text lines and a picture (a filled block) in each arrangement.
    const pic = (x, y, w, hh, fill = 'var(--accent-soft)') => `<rect x="${x}" y="${y}" width="${w}" height="${hh}" rx="2" fill="${fill}" stroke="var(--accent)" stroke-width="1.2"/>`;
    const text = (rows) => rows.map(([x, y, w]) => `<rect x="${x}" y="${y}" width="${w}" height="2.2" fill="currentColor" opacity=".55"/>`).join('');
    const svg = (inner) => `<svg aria-hidden="true" viewBox="0 0 64 36" width="64" height="36">${inner}</svg>`;
    if (kind === 'Inline') return svg(text([[4, 4, 56]]) + pic(18, 9, 28, 18) + text([[4, 31, 56]]));
    if (kind === 'Around') return svg(pic(4, 6, 24, 20) + text([[32, 6, 28], [32, 12, 28], [32, 18, 28], [32, 24, 28], [4, 30, 56]]));
    if (kind === 'TopBottom') return svg(text([[4, 4, 56]]) + pic(4, 9, 24, 18) + text([[4, 31, 56]]));
    if (kind === 'InFront') return svg(text([[4, 6, 56], [4, 12, 56], [4, 18, 56], [4, 24, 56], [4, 30, 56]]) + pic(18, 9, 28, 18, 'var(--sheet)'));
    return svg(pic(18, 9, 28, 18) + text([[4, 6, 56], [4, 12, 56], [4, 18, 56], [4, 24, 56], [4, 30, 56]]));
  };
  const chips = (label, options, cur, act) => [h('div', { class: 'lbl' }, label), h('div', { class: 'chips' }, options.map(([v, n]) =>
    h('button', { class: `chip${cur === v ? ' on' : ''}`, 'aria-pressed': String(cur === v), onClick: () => act(v) }, n)))];
  const draw = () => {
    const inline = !info.frame;
    fill(body,
      h('div', { class: 'lbl' }, 'Text and the picture'),
      h('div', { class: 'tiles five' }, [['inline', 'Inline', 'In line'], ['around', 'Around', 'Beside it'], ['top_bottom', 'TopBottom', 'Above and below'], ['in_front', 'InFront', 'In front'], ['behind', 'Behind', 'Behind']].map(([v, k, n]) =>
        tile(wrapPic(k), n, info.wrap === k, () => step('image_wrap', { wrap: v })))),
      inline ? chips('Where it sits', [['left', 'Left'], ['center', 'Centre'], ['right', 'Right']], info.align, (v) => step('image_set', { align: v }))
        : [chips('Space between it and the text', [[0, 'None'], [6, 'Small'], [12, 'Medium'], [24, 'Large']],
          [0, 6, 12, 24].reduce((b, v) => (Math.abs(v - info.gap) < Math.abs(b - info.gap) ? v : b)), (v) => step('frame_set', { id: info.frame, gap: v })),
          h('p', { class: 'hint-text' }, 'Drag the picture’s edge to move it; drag a corner to resize it.')],
      chips('Caption', [[null, 'None'], ['bottom', 'Below'], ['top', 'Above']], info.caption, (v) => step('image_caption', { place: v })),
      chips('Line around it', [[true, 'Thin line'], [false, 'No line']], !!info.border, (v) => step('image_set', { border: v })));
  };
  draw();
  const undo = async () => { for (; changes > 0; changes -= 1) await run('undo'); };
  modalShell('Picture options', body, [['Delete picture', async () => { clearPicture(); await run('delete_block', { id }); }], ['Cancel', 'cancel'], ['Done', null, true]], { side: true, onCancel: undo });
}

// Text boxes: grab an edge to move the box; the corner handle resizes it.
const frameSel = { el: null, info: null };
function clearFrame() { frameSel.el?.remove(); frameSel.el = null; frameSel.info = null; }
/// The box's rect from the last view (so grabbing it needs no round trip).
function frameRect(id) { return (pages.view?.frames || []).find(([fid]) => fid === id)?.[1]; }
/// Outlines the selected text box with handles on its corners and sides.
function showFrame(info) {
  clearFrame();
  frameSel.info = info;
  const z = pages.zoom;
  const [x, y, w, hh] = info.rect;
  const box = h('div', { class: 'pic-sel frame-sel', style: `left: ${pages.originX + x * z}px; top: ${pages.originY + y * z}px; width: ${w * z}px; height: ${hh * z}px` });
  for (const side of ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw']) {
    const handle = h('div', { class: `frame-handle ${side}`, title: 'Drag to resize' });
    handle.addEventListener('pointerdown', (e) => resizeFrame(e, handle, side, box, info));
    box.append(handle);
  }
  pages.overlay.append(box);
  frameSel.el = box;
}
function resizeFrame(e, handle, side, box, info) {
  e.preventDefault();
  e.stopPropagation();
  handle.setPointerCapture(e.pointerId);
  const z = pages.zoom;
  const [x0, y0, w0, h0] = info.rect;
  const sx = e.clientX, sy = e.clientY;
  let r = [x0, y0, w0, h0];
  const move = (ev) => {
    const dx = (ev.clientX - sx) / z, dy = (ev.clientY - sy) / z;
    let [x, y, w, hh] = [x0, y0, w0, h0];
    if (side.includes('e')) w = Math.max(24, w0 + dx);
    if (side.includes('s')) hh = Math.max(16, h0 + dy);
    if (side.includes('w')) { w = Math.max(24, w0 - dx); x = x0 + w0 - w; }
    if (side.includes('n')) { hh = Math.max(16, h0 - dy); y = y0 + h0 - hh; }
    r = [x, y, w, hh];
    Object.assign(box.style, { left: `${pages.originX + x * z}px`, top: `${pages.originY + y * z}px`, width: `${w * z}px`, height: `${hh * z}px` });
  };
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', async () => {
    handle.removeEventListener('pointermove', move);
    const [x, y, w, hh] = r;
    if (x === x0 && y === y0 && w === w0 && hh === h0) return;
    await run('frame_set', { id: info.id, dx: x - x0, dy: y - y0, w, h: hh });
    showFrame({ ...info, rect: r });
  }, { once: true });
}
/// Moving a box by its edge: the outline follows the pointer at once; the
/// text moves when you let go.
function grabFrame(view, id, e) {
  const z = view.zoom, sx = e.clientX, sy = e.clientY;
  const rect = frameRect(id);
  if (!rect) return;
  showFrame({ id, rect });
  const box = frameSel.el;
  const move = (ev) => {
    box.style.left = `${view.originX + rect[0] * z + ev.clientX - sx}px`;
    box.style.top = `${view.originY + rect[1] * z + ev.clientY - sy}px`;
  };
  const up = async (ev) => {
    window.removeEventListener('pointermove', move);
    const dx = (ev.clientX - sx) / z, dy = (ev.clientY - sy) / z;
    if (Math.abs(dx) + Math.abs(dy) < 0.5) return;
    await run('frame_set', { id, dx, dy });
    showFrame({ id, rect: frameRect(id) || [rect[0] + dx, rect[1] + dy, rect[2], rect[3]] });
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up, { once: true });
}
pages.onFrameDown = (id, e) => grabFrame(pages, id, e);
pages.onDropPictures = async (files) => {
  for (const f of files) {
    const bytes = new Uint8Array(await f.arrayBuffer());
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    await run('insert_picture_bytes', { name: f.name || 'picture.png', data: btoa(bin) });
  }
};
onStacks('pointerdown', (e) => { if (!e.target.closest('.pic-sel')) clearFrame(); }, true);
// The pointer shows "move" over a box's edge.
onStacks('pointermove', (e) => {
  if (e.buttons) return;
  pages.stack.style.cursor = pages.frameEdgeAt(e) != null ? 'move' : '';
});
window.addEventListener('keydown', async (e) => {
  if (!frameSel.info || document.querySelector('.modal-back')) return;
  if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); e.stopPropagation(); const id = frameSel.info.id; clearFrame(); await run('frame_delete', { id }); }
  else if (e.key === 'Escape') clearFrame();
}, true);

/// Text box options: how text goes around it, its line, background and margins.
async function frameOptions(id) {
  let info = await call('frame_at', (() => { const [x, y] = frameRect(id) || [0, 0]; return { x, y, slop: 3 }; })());
  if (!info) return;
  let changes = 0;
  const set = async (patch) => { if (await run('frame_set', { id, ...patch })) changes += 1; info = await call('frame_at', { x: info.rect[0], y: info.rect[1], slop: 3 }) || info; draw(); };
  const body = h('div', { class: 'pick-pop' });
  const wrapPic = (kind) => {
    const lines = kind === 'Around' ? [[4, 6, 22], [4, 12, 22], [4, 18, 22], [4, 24, 22], [4, 30, 56]] : kind === 'TopBottom' ? [[4, 6, 56], [4, 30, 56]] : [[4, 6, 56], [4, 12, 56], [4, 18, 56], [4, 24, 56], [4, 30, 56]];
    const boxSvg = `<rect x="${kind === 'Around' ? 30 : 16}" y="10" width="${kind === 'Around' ? 30 : 32}" height="16" rx="2" fill="${kind === 'Behind' ? 'var(--accent-soft)' : 'var(--sheet)'}" stroke="var(--accent)" stroke-width="1.2" ${kind === 'Behind' ? 'opacity=".9"' : ''}/>`;
    return kind === 'Behind' ? `<svg aria-hidden="true" viewBox="0 0 64 36" width="64" height="36">${boxSvg}${lines.map(([x, y, w]) => `<rect x="${x}" y="${y}" width="${w}" height="2.2" fill="currentColor" opacity=".55"/>`).join('')}</svg>`
      : sketch(lines.map(([x, y, w]) => [x, y, w, false]), { h: 36, extra: boxSvg });
  };
  const chips = (label, options, cur, key) => {
    // Numbers: the nearest choice is the one shown as on.
    const on = typeof cur === 'number' ? options.reduce((b, o) => (Math.abs(o[0] - cur) < Math.abs(b[0] - cur) ? o : b))[0] : cur;
    return [h('div', { class: 'lbl' }, label), h('div', { class: 'chips' }, options.map(([v, n]) =>
      h('button', { class: `chip${on === v ? ' on' : ''}`, 'aria-pressed': String(on === v), onClick: () => set({ [key]: v }) }, n)))];
  };
  const draw = () => fill(body,
    h('div', { class: 'lbl' }, 'Text around the box'),
    h('div', { class: 'tiles' }, [['around', 'Around', 'Beside it'], ['top_bottom', 'TopBottom', 'Above and below'], ['in_front', 'InFront', 'Box in front'], ['behind', 'Behind', 'Box behind']].map(([v, k, n]) =>
      tile(wrapPic(k), n, info.wrap === k, () => set({ wrap: v })))),
    info.wrap === 'Around' || info.wrap === 'TopBottom' ? chips('Space between it and the text', [[0, 'None'], [6, 'Small'], [12, 'Medium'], [24, 'Large']], info.gap, 'gap') : null,
    chips('Margin inside the box', [[0, 'None'], [4, 'Small'], [8, 'Medium'], [14, 'Large']], info.padding, 'padding'),
    chips('Line around it', [[true, 'Thin line'], [false, 'No line']], info.border, 'border'),
    h('div', { class: 'lbl' }, 'Background'),
    h('div', { class: 'swatches' }, [...SHADES, '#FEF08A', '#BAE6FD'].map((c) => h('button', {
      class: `swatch${info.fill?.toLowerCase() === c.toLowerCase() ? ' on' : ''}`, style: `background: ${c}`, 'aria-label': c, onClick: () => set({ fill: c }) }))),
    h('div', {}, h('button', { class: `btn small${info.fill ? '' : ' main'}`, onClick: () => set({ fill: null }) }, 'No background')));
  draw();
  const undo = async () => { for (; changes > 0; changes -= 1) await run('undo'); };
  modalShell('Text box options', body, [['Delete box', async () => { clearFrame(); await run('frame_delete', { id }); }], ['Cancel', 'cancel'], ['Done', null, true]], { side: true, onCancel: undo });
}

onStacks('contextmenu', async (e) => {
  const id = pages.frameEdgeAt(e);
  if (id == null) return;
  e.stopImmediatePropagation();
  e.preventDefault();
  showFrame({ id, rect: frameRect(id) });
  popup(anchorAt(e.clientX, e.clientY), [
    item('Text box options…', { run: () => frameOptions(id) }),
    item('Delete text box', { run: () => { clearFrame(); run('frame_delete', { id }); } }),
  ], { width: 240 });
}, true);
// Double-clicking a box's edge opens its options.
onStacks('dblclick', (e) => {
  const id = pages.frameEdgeAt(e);
  if (id == null) return;
  e.stopImmediatePropagation();
  frameOptions(id);
}, true);

// In the browser, a file dropped off the page mustn't replace Write with it
// (the page itself takes pictures: page.js).
window.addEventListener('dragover', (e) => { if (e.dataTransfer?.types?.includes('Files')) e.preventDefault(); });
window.addEventListener('drop', (e) => { if (e.dataTransfer?.types?.includes('Files')) e.preventDefault(); });
// Files dropped on the window: pictures go in where they land, documents open.
window.__TAURI__?.event.listen('tauri://drag-drop', async (ev) => {
  const { paths, position } = ev.payload || {};
  for (const path of paths || []) {
    if (/\.(png|jpe?g|webp|gif|bmp)$/i.test(path)) {
      if (position) {
        const r = pages.stack.getBoundingClientRect();
        const z = pages.zoom, dpr = window.devicePixelRatio || 1;
        const x = (position.x / dpr - r.left - pages.originX) / z, y = (position.y / dpr - r.top - pages.originY) / z;
        await run('down', { x, y, clicks: 1, mods: {} });
        await run('up');
      }
      await run('insert_picture', { path });
      await copyInForServer();
    } else {
      await openPath(path);
    }
  }
});

/// Enter and Escape for a dialog on `back` (a .modal-back): Escape runs
/// `cancel`, Enter clicks its main button (or its only one; `mainButton`
/// finds it in a dialog laid out its own way). Only the top dialog answers,
/// not while a popup is open, and Enter is left to what uses it itself: a
/// text area, a list, a button, a text being edited, Ctrl/Alt/⌘, an accent
/// being typed; a key a box in it took (preventDefault) is left alone.
/// Returns the function to call when the dialog closes.
function dialogKeys(back, cancel, mainButton = null) {
  const top = () => back === [...document.querySelectorAll('.modal-back')].pop();
  const wanted = (e) => (e.key === 'Escape' || (e.key === 'Enter' && !e.ctrlKey && !e.altKey && !e.metaKey)) && !e.isComposing;
  const act = (e) => {
    if (!back.isConnected || !top() || openPopup) return;
    if (e.key === 'Escape') { cancel(); return; }
    const btns = [...back.querySelectorAll(':scope > .modal > .modal-actions .btn')];
    // With only Close at the bottom, a dialog's one main button above it (Ask AI's Rewrite) is what Enter means.
    const inside = [...back.querySelectorAll('.modal .btn.main')].filter((x) => !btns.includes(x));
    // A lone Cancel isn't something to commit (Mail merge): Enter leaves it alone.
    const main = mainButton ? mainButton() : btns.find((x) => x.classList.contains('main')) || (inside.length === 1 ? inside[0] : btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) main.click();
  };
  const key = (e) => {
    if (!back.isConnected || !top() || !wanted(e) || openPopup) return;
    const t = e.target;
    if (!back.contains(t)) {
      // Focus left outside (the button that opened it); the page beside a side dialog is its own.
      if (t.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      e.preventDefault(); e.stopPropagation(); act(e);
      return;
    }
    if (e.key === 'Enter' && t.closest('textarea, select, button, a[href], [contenteditable]:not([contenteditable="false"]), [aria-expanded="true"]')) return;
    // After the dialog's own boxes have had it (and may have taken it).
    setTimeout(() => { if (!e.defaultPrevented) act(e); });
  };
  // Not on to the shortcuts underneath.
  back.addEventListener('keydown', (e) => { if (wanted(e)) e.stopPropagation(); });
  document.addEventListener('keydown', key, true);
  return () => document.removeEventListener('keydown', key, true);
}

// A dialog with buttons: [label, action (returns false to stay open), main?].
function modalShell(title, body, buttons, { side = false, onCancel = null } = {}) {
  // `side`: off to the right, the page stays visible (changes show as you make them).
  // `onCancel`: what Esc and a button with the action 'cancel' do first (put back what changed).
  const back = h('div', { class: `modal-back${side ? ' side' : ''}` });
  const close = () => { back.remove(); unkey(); if (!document.querySelector('.modal-back')) pages.focus(); };
  const cancel = async () => { close(); await onCancel?.(); };
  // Escape cancels, Enter is the main button.
  const unkey = dialogKeys(back, cancel);
  const box = h('div', { class: 'modal wide', role: 'dialog', 'aria-modal': 'true', 'aria-label': title }, h('h2', {}, title), body,
    h('div', { class: 'modal-actions', style: 'margin-top: 16px' }, buttons.map(([label, act, main]) => h('button', { class: `btn${main ? ' main' : ''}`, onClick: async () => {
      if (!act) { close(); return; }
      if (act === 'cancel') { cancel(); return; }
      const ok = await act();
      if (ok !== false) close();
    } }, label))));
  back.append(box);
  document.body.append(back);
  return { close, body: box };
}
const refsEnv = () => ({ h, run, toast, modalShell });

// ----- Mail merge: one letter (or label, invoice…) per row of a spreadsheet -----

async function mailMerge() {
  let info = await call('merge_info');
  const printers = WEB ? [] : (await call('printers').catch(() => ({ printers: [] }))).printers;
  const body = h('div', { class: 'merge' });
  let n = 1;
  const choose = async () => {
    const path = await window.__TAURI__.core.invoke('choose', { kind: 'data', dir: (await call('save_info')).dir });
    if (!path) return;
    try { info = await call('merge_load', { path }); draw(); } catch (e) { toast(String(e)); }
  };
  const draw = () => {
    const ext = h('select', { class: 'field', 'aria-label': 'Make' }, [['pdf', 'PDF files'], ['docx', '.docx files'], ['md', 'Markdown files']].map(([v, l]) => h('option', { value: v }, l)));
    const combined = h('input', { type: 'checkbox', id: 'mm-one' });
    const range = h('input', { class: 'field', placeholder: 'All, or e.g. 1-10, 15', 'aria-label': 'Which records' });
    const name = h('input', { class: 'field', placeholder: info.columns[0] ? `e.g. {{${info.columns[0]}}}` : 'e.g. {{name}}', 'aria-label': 'File names' });
    const printRange = h('input', { class: 'field', placeholder: 'All, or e.g. 1-10, 15', 'aria-label': 'Which rows to print' });
    const printer = h('select', { class: 'field', 'aria-label': 'Printer' }, h('option', { value: '' }, 'Open in the PDF viewer'),
      printers.map((p) => h('option', { value: p, selected: p === printers[0] }, p)));
    fill(body,
      h('div', { class: 'merge-step' }, h('b', {}, '1. Your list'),
        info.file ? h('p', {}, `${info.file}: ${info.records} ${info.records === 1 ? 'row' : 'rows'}. `, h('button', { class: 'link', onClick: choose }, 'Choose another')) : [h('p', {}, 'A spreadsheet saved as CSV, or a JSON file. Each row makes one copy.'), h('button', { class: 'btn', onClick: choose }, 'Choose the list…')]),
      h('div', { class: 'merge-step' }, h('b', {}, '2. Put the fields in'),
        h('p', {}, 'Click a column to put it in at the cursor. It’s filled in for each row.'),
        info.columns.length ? h('div', { class: 'chips' }, info.columns.map((c) => {
          const b = h('button', { class: 'chip', onClick: () => pages.send('text', { text: `{{${c}}}` }) }, c);
          b.addEventListener('mousedown', (e) => e.preventDefault());
          return b;
        })) : h('p', { class: 'nav-empty' }, 'Choose the list first.'),
        info.used.length ? h('p', { class: 'ai-note' }, `In the document now: ${info.used.join(', ')}`) : null),
      h('div', { class: 'merge-step' }, h('b', {}, '3. Check one'),
        h('div', { class: 'ai-row' }, h('button', { class: 'btn', disabled: !info.records, onClick: async () => { await run('merge_preview', { n }); pages.drawn.clear(); toast(`Showing row ${n} in a new tab`); } }, `Preview row ${n}`),
          h('button', { class: 'btn', disabled: !info.records, onClick: () => { n = n % Math.max(1, info.records) + 1; draw(); } }, 'Next row'))),
      h('div', { class: 'merge-step' }, h('b', {}, '4. Print them'),
        WEB ? h('p', {}, 'In the browser, make one PDF with every copy (below) and print that.') : [
          h('div', { class: 'form-grid' }, h('div', {}, h('label', { class: 'lbl' }, 'Printer'), printer), h('div', {}, h('label', { class: 'lbl' }, 'Which rows'), printRange)),
          h('button', { class: 'btn main', style: 'margin-top: 12px', disabled: !info.records, onClick: async () => {
            try {
              await call('merge_print', { printer: printer.value || null, range: printRange.value });
              toast(printer.value ? `Sent ${info.records === 1 ? 'the copy' : 'the copies'} to ${printer.value}` : 'Opened them in your PDF viewer to print');
            } catch (e) { toast(String(e)); }
          } }, 'Print them')]),
      h('div', { class: 'merge-step' }, h('b', {}, '5. Or make files'),
        h('div', { class: 'form-grid' }, h('div', {}, h('label', { class: 'lbl' }, 'Make'), ext), h('div', {}, h('label', { class: 'lbl' }, 'Which rows'), range),
          h('div', { style: 'grid-column: 1 / -1' }, h('label', { class: 'lbl' }, 'File names'), name)),
        h('label', { class: 'check-line', for: 'mm-one' }, combined, 'One file with every copy, instead of a file each'),
        h('button', { class: 'btn main', style: 'margin-top: 12px', disabled: !info.records || WEB, onClick: async () => {
          const dir = await window.__TAURI__.core.invoke('choose', { kind: 'folder', dir: (await call('save_info')).dir });
          if (!dir) return;
          try {
            const out = await call('merge_run', { dir, ext: ext.value, combined: combined.checked, range: range.value, name: name.value });
            toast(`Made ${out.written} ${out.written === 1 ? 'file' : 'files'} in ${out.dir.split('/').pop()}`);
          } catch (e) { toast(String(e)); }
        } }, 'Choose a folder and make them…')));
  };
  draw();
  modalShell('Mail merge', body, [['Cancel', null]]);
}

// ----- Help -----

const SHORTCUTS = [
  ['Files', [['New document', 'Ctrl+N'], ['Open', 'Ctrl+O'], ['Save', 'Ctrl+S'], ['Save as', 'Ctrl+Shift+S'], ['Print', 'Ctrl+P'], ['Close tab', 'Ctrl+W'], ['Next tab', 'Ctrl+Tab'], ['Settings', 'Ctrl+,'], ['Quit', 'Ctrl+Q']]],
  ['Editing', [['Undo', 'Ctrl+Z'], ['Redo', 'Ctrl+Y'], ['Cut, copy, paste', 'Ctrl+X, C, V'], ['Select all', 'Ctrl+A'], ['Select a word, a paragraph', 'Double-click, triple-click'], ['Find', 'Ctrl+F'], ['Find and replace', 'Ctrl+H'], ['Look up a word', 'Shift+F7'], ['Page break', 'Ctrl+Enter']]],
  ['Several carets', [['Add a caret', 'Alt+click'], ['Add a caret above or below', 'Ctrl+Alt+↑ ↓ or Shift+Alt+↑ ↓'], ['Select the next match too', 'Ctrl+D'], ['Select a column', 'Shift+Alt+drag'], ['Back to one caret', 'Esc']]],
  ['Formatting', [['Bold', 'Ctrl+B'], ['Italic', 'Ctrl+I'], ['Underline', 'Ctrl+U'], ['Clear formatting', 'Ctrl+Space'], ['Link', 'Ctrl+K'], ['Bold, italic', 'Type **bold** or *italic*'], ['Bulleted list', 'Type * and a space'], ['Numbered list', 'Type 1. and a space'], ['Heading 1, 2, 3', 'Ctrl+Alt+1, 2, 3'], ['Body text', 'Ctrl+Alt+0']]],
  ['Moving', [['Word by word', 'Ctrl+← →'], ['Start or end of line', 'Home, End'], ['Start or end of document', 'Ctrl+Home, Ctrl+End'], ['Select as you move', 'Shift + any of these'], ['Next table cell', 'Tab']]],
  ['Insert and review', [['Equation', 'Alt+='], ['Footnote', 'Ctrl+Alt+F'], ['Symbols & emoji', 'Ctrl+.'], ['New comment', 'Ctrl+Alt+M'], ['Track changes', 'Ctrl+Shift+E']]],
  ['Window', [['Menus', 'Alt+letter, or F10'], ['Navigation', 'Ctrl+Shift+N'], ['Zoom', 'Ctrl+ +, Ctrl+ −, Ctrl+0']]],
];
function modal(title, body, { wide = true } = {}) {
  const back = h('div', { class: 'modal-back' });
  const done = () => { back.remove(); unkey(); pages.focus(); };
  // Escape and Enter close it.
  const unkey = dialogKeys(back, done);
  back.append(h('div', { class: `modal${wide ? ' wide' : ''}`, role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
    h('h2', {}, title), body, h('div', { class: 'modal-actions', style: 'margin-top: 16px' }, h('button', { class: 'btn main', onClick: done }, 'Close'))));
  document.body.append(back);
  back.querySelector('input, .btn.main')?.focus();
  return done;
}
function shortcutsDialog() {
  const search = h('input', { class: 'field', type: 'search', placeholder: 'Find a shortcut, e.g. bold', 'aria-label': 'Find a shortcut' });
  const list = h('div', { class: 'shortcuts' });
  const draw = () => {
    const q = search.value.trim().toLowerCase();
    fill(list, ...SHORTCUTS.map(([g, items]) => {
      const shown = items.filter(([n, k]) => keysHere(k) && (!q || n.toLowerCase().includes(q) || k.toLowerCase().includes(q)));
      return shown.length ? h('div', {}, h('div', { class: 'menu-label' }, g), shown.map(([n, k]) => h('div', { class: 'sc-row' }, h('span', {}, n), h('kbd', {}, keysHere(k))))) : null;
    }));
  };
  search.addEventListener('input', draw);
  draw();
  modal('Keyboard shortcuts', [search, list]);
}
function aboutDialog() {
  modal('About Xenify Write', h('div', { class: 'about-box' },
    h('p', {}, 'A word processor that checks your writing on your own computer. Free and open source (MIT licence).'),
    h('p', {}, 'Made with Rust, Tauri, parley and Vello. Spelling and grammar by Harper and Xenify’s own rules; dictionary from Open English WordNet (CC BY 4.0).'),
    h('p', {}, 'Fonts: Figtree, Inter, STIX Two Math, CMU Serif (SIL Open Font License). Emoji: Fluent Emoji © Microsoft (MIT). Icons: Lucide (ISC).')));
}

// ----- Tables -----

const TABLE_ACTIONS = [
  ['row_above', 'Insert row above'], ['row_below', 'Insert row below'], ['col_left', 'Insert column left'], ['col_right', 'Insert column right'],
  '-', ['delete_rows', 'Delete row'], ['delete_cols', 'Delete column'], ['delete_table', 'Delete table'],
  '-', ['merge', 'Merge cells'], ['split', 'Split cell'], ['header_row', 'Header row on or off'], ['header_col', 'First column as headers on or off'], ['distribute', 'Even column widths'],
];
function tableItems() {
  const on = !!st().in_table;
  return [...TABLE_ACTIONS.map((a) => (a === '-' ? h('hr') : item(a[1], { enabled: on, run: () => run('table', { what: a[0] }) }))),
    h('hr'), item('Table options…', { enabled: on, run: () => tableDialog() })];
}

async function tableDialog() {
  let info = await call('table_info');
  if (!info) { toast('Put the cursor in a table'); return; }
  let own = (await call('details').catch(() => null))?.colors || [];
  // Each change is one undo step, so Cancel undoes them all.
  let changes = 0;
  const body = h('div', { class: 'pick-pop table-opts' });
  const t = async (what, extra = {}) => { if (await run('table', { what, ...extra })) changes += 1; info = (await call('table_info')) || info; draw(); };
  const undo = async () => { for (; changes > 0; changes -= 1) await run('undo'); };
  // A small table drawn as the choice would make it: 4 columns, 4 rows.
  const mini = ({ head = true, col = false, foot = false, fill = '#CBD5E1', band = null, grid = true, rowLines = false, w = 56, x = 4 } = {}) => {
    const cw = w / 4, rh = 8, y0 = 4;
    let out = '';
    for (let r = 0; r < 4; r++) {
      for (let c = 0; c < 4; c++) {
        const isHead = (head && r === 0) || (col && c === 0) || (foot && r === 3);
        const bg = isHead ? fill : band && r % 2 === 0 && r > 0 ? band : 'var(--sheet)';
        out += `<rect x="${x + c * cw}" y="${y0 + r * rh}" width="${cw}" height="${rh}" fill="${bg}"${grid ? ' stroke="#9CA3AF" stroke-width=".6"' : ''}/>`;
        // The text in it: bold (thicker) in headers.
        out += `<rect x="${x + c * cw + 2}" y="${y0 + r * rh + 3.2}" width="${cw - 5}" height="${isHead ? 2 : 1.2}" fill="currentColor" opacity="${isHead ? 0.8 : 0.4}"/>`;
      }
      if (rowLines) out += `<rect x="${x}" y="${y0 + (r + 1) * rh - 0.4}" width="${w}" height=".7" fill="#9CA3AF"/>`;
    }
    return `<svg aria-hidden="true" viewBox="0 0 64 40" width="64" height="40">${out}</svg>`;
  };
  // A single tall cell, its text at the top, middle or bottom.
  const cellPic = (where) => {
    const y = where === 'top' ? 9 : where === 'middle' ? 18 : 27;
    return `<svg aria-hidden="true" viewBox="0 0 64 40" width="64" height="40"><rect x="14" y="4" width="36" height="32" fill="var(--sheet)" stroke="#9CA3AF"/><rect x="19" y="${y}" width="26" height="2" fill="currentColor" opacity=".7"/><rect x="19" y="${y + 4}" width="18" height="2" fill="currentColor" opacity=".7"/></svg>`;
  };
  // The table on the page: its width and where it sits.
  const placePic = (w, where) => {
    const W = 48 * w, x = 8 + (where === 'center' ? (48 - W) / 2 : where === 'right' ? 48 - W : 0);
    let lines = '';
    for (let y = 6; y < 36; y += 5) if (y < 12 || y > 28) lines += `<rect x="8" y="${y}" width="48" height="1.6" fill="currentColor" opacity=".35"/>`;
    return `<svg aria-hidden="true" viewBox="0 0 64 40" width="64" height="40">${lines}<rect x="${x}" y="14" width="${W}" height="12" fill="var(--accent-soft, #FDE3D3)" stroke="var(--accent)" stroke-width="1"/><rect x="${x + W / 2}" y="14" width=".8" height="12" fill="var(--accent)"/><rect x="${x}" y="19.6" width="${W}" height=".8" fill="var(--accent)"/></svg>`;
  };
  const LOOKS = [
    { grid: true, fill: 'var(--sheet)' },
    { grid: false, rowLines: true, fill: '#F1F3F5' },
    { grid: false, fill: '#DDE6F3', band: '#F4F7FB' },
    { grid: true, fill: '#CFDDFB' },
    { grid: false, head: false },
  ];
  const sec = (title, ...kids) => [h('div', { class: 'lbl' }, title), ...kids];
  const draw = () => {
    const hr = info.header_row, hc = info.header_col;
    const look = { grid: info.lines, band: info.band ? '#F4F7FB' : null };
    fill(body,
      sec('Headers', h('div', { class: 'tiles' },
        [[false, false, 'None'], [true, false, 'Top row'], [false, true, 'First column'], [true, true, 'Both']].map(([r, c, name]) =>
          tile(mini({ ...look, head: r, col: c }), name, hr === r && hc === c, () => t('headers', { row: r, col: c }))))),
      sec('Footer row (e.g. totals)', h('div', { class: 'tiles' },
        tile(mini({ ...look, head: hr, col: hc }), 'No footer', !info.footer_row, () => t('footer', { on: false })),
        tile(mini({ ...look, head: hr, col: hc, foot: true }), 'Last row', info.footer_row, () => t('footer', { on: true })))),
      (hr || info.footer_row) ? sec('When the table runs over pages', h('div', { class: 'chips' },
        hr ? h('button', { class: `chip${info.repeat_header ? ' on' : ''}`, 'aria-pressed': String(info.repeat_header), onClick: () => t('repeat', { header: !info.repeat_header }) }, 'Header row on every page') : null,
        info.footer_row ? h('button', { class: `chip${info.repeat_footer ? ' on' : ''}`, 'aria-pressed': String(info.repeat_footer), onClick: () => t('repeat', { footer: !info.repeat_footer }) }, 'Footer row on every page') : null)) : null,
      sec('Look', h('div', { class: 'tiles five' }, info.styles.map((name, n) => tile(mini({ ...LOOKS[n], head: LOOKS[n].head !== false && hr, col: hc && LOOKS[n].head !== false }), name, false, () => t('style', { n }))))),
      sec('Colours', colourRows()),
      sec('Rows and lines', h('div', { class: 'tiles' },
        tile(mini({ head: hr, col: hc, grid: true, band: look.band }), 'Lines', info.lines, () => t(info.lines ? 'no_lines' : 'lines')),
        tile(mini({ head: hr, col: hc, grid: false, band: look.band }), 'No lines', !info.lines, () => t('no_lines')),
        tile(mini({ head: hr, col: hc, grid: look.grid, band: '#E8EEF8' }), 'Banded rows', info.band, () => t('band', { on: !info.band })),
        tile(mini({ head: hr, col: hc, grid: look.grid }), 'Plain rows', !info.band, () => t('band', { on: false })))),
      sec('Width and position', h('div', { class: 'tiles five' },
        [[1, 'left', 'Full width'], [0.6, 'left', 'Left'], [0.6, 'center', 'Centre'], [0.6, 'right', 'Right']].map(([w, a, name]) => {
          const on = w === 1 ? info.width > 0.95 : info.width <= 0.95 && info.align === a;
          return tile(placePic(w, a), name, on, () => t('layout', { width: w === 1 ? 1 : Math.min(info.width < 0.95 ? info.width : 0.6, 0.95), align: a }));
        }))),
      sec('Text in the cells', h('div', { class: 'tiles three' },
        [['top', 'Top'], ['middle', 'Middle'], ['bottom', 'Bottom']].map(([v, name]) => tile(cellPic(v), name, info.v_align === v, () => t(v))))),
      sec('Shading of the selected cells', h('div', { class: 'swatches' }, [...SHADES, '#FDE68A', '#C7D2FE'].map((c) => h('button', {
        class: 'swatch', style: `background: ${c}`, 'aria-label': c, onClick: () => t('shade', { hex: c }) }))),
        h('button', { class: 'btn small', onClick: () => t('shade') }, 'No shading')),
      info.can_sort ? sec(`Sort by column ${info.col + 1}`, h('div', { class: 'chips' },
        h('button', { class: 'chip', onClick: () => t('sort') }, 'A to Z, 1 to 9'), h('button', { class: 'chip', onClick: () => t('sort_desc') }, 'Z to A, 9 to 1'))) : null);
  };
  // Header, bands and lines: the usual colours, the document's own and + for
  // another (kept with the document); then all the document's tables alike.
  const HEADS = ['#F1F3F5', '#DDE6F3', '#CFDDFB', '#E7E5E4', '#FFEDD5', '#DCFCE7', '#1E3A8A', '#1C1917'];
  const BANDS = ['#F4F7FB', '#F5F5F4', '#FFF7ED', '#F0FDF4', '#FAF5FF', '#FEFCE8'];
  const LINES = ['#C4C9D0', '#807A73', '#1C1917', '#1D4ED8', '#C2410C', '#047857'];
  function colourRows() {
    const row = (label, key, list, cur, none) => {
      const set = (hex) => t('colors', { [key]: hex });
      const other = h('input', { type: 'color', class: 'swatch-input', 'aria-label': `Another ${label.toLowerCase()} colour`, value: cur || '#c2410c' });
      other.addEventListener('change', async () => { await run('add_color', { hex: other.value }); own = [other.value, ...own.filter((c) => c !== other.value)]; set(other.value); });
      const swatches = [...new Set([...list, ...own].map((c) => c.toLowerCase()))].map((c) => h('button', {
        class: `swatch${cur && cur.toLowerCase() === c ? ' on' : ''}`, style: `background: ${c}`, 'aria-label': c, title: c, onClick: () => set(c) }));
      return h('div', { class: 'tcolour-row' }, h('span', { class: 'tcolour-name' }, label),
        h('div', { class: 'swatches' }, ...swatches, h('label', { class: 'swatch swatch-add', title: 'Another colour: it’s kept with this document', html: icon('plus', 14) }, other)),
        none ? h('button', { class: `btn small${cur ? '' : ' on'}`, onClick: () => set('none') }, 'None') : null);
    };
    return [row('Header', 'header', HEADS, info.header_color, true), row('Banded rows', 'band', BANDS, info.band_color, true),
      info.line_color ? row('Lines', 'lines', LINES, info.line_color, false) : null,
      h('div', { class: 'chips' }, h('button', { class: 'chip', title: 'Every table in this document gets these colours, and the looks above use them', onClick: () => t('theme', { on: true }).then(() => toast('Every table in this document uses these colours')) }, 'Use for every table in this document'),
        info.themed ? h('button', { class: 'chip', title: 'The looks go back to their usual colours (tables keep theirs)', onClick: () => t('theme', { on: false }) }, 'Looks in their usual colours') : null)];
  }
  draw();
  modalShell('Table options', body, [['Delete table', async () => { await run('table', { what: 'delete_table' }); }], ['Cancel', 'cancel'], ['Done', null, true]], { side: true, onCancel: undo });
}

// ----- Page setup -----

const SIZES = [['A4', 595.28, 841.89], ['Letter', 612, 792], ['A5', 419.53, 595.28], ['Legal', 612, 1008], ['A3', 841.89, 1190.55]];
const UNIT = { cm: [72 / 2.54, 'cm', 0.1], in: [72, 'in', 0.05], pt: [1, 'pt', 1] };


const MARGINS = [['Normal', 72, 72], ['Narrow', 36, 36], ['Moderate', 72, 54], ['Wide', 72, 144]];

/// Page › Page setup: paper, orientation, margins and columns, as pictures;
/// each choice applies straight away.
async function pageSetup() {
  let { page, sections } = await call('page');
  const original = structuredClone(page);
  const [k, unit, step] = UNIT[app.prefs.units] || UNIT.cm;
  const show = (pt) => +(pt / k).toFixed(unit === 'pt' ? 0 : 2);
  const body = h('div', { class: 'pick-pop page-setup' });
  const set = async (patch) => { await run('set_page', { page: patch }); pages.drawn.clear(); ({ page } = await call('page')); draw(); };
  const land = () => page.width > page.height;
  const sizeOf = () => SIZES.find(([, sw, sh]) => Math.abs(Math.min(page.width, page.height) - sw) < 1 && Math.abs(Math.max(page.width, page.height) - sh) < 1)?.[0];
  // A page drawn to scale, with its margins and columns.
  const sheet = (w, hh, m = null, cols = 1) => {
    const sc = 40 / Math.max(w, hh);
    const W = w * sc, H = hh * sc;
    const x0 = (48 - W) / 2, y0 = (44 - H) / 2;
    let inner = '';
    if (m) {
      const l = m.left * sc, r = m.right * sc, t = m.top * sc, bt = m.bottom * sc;
      const cw = (W - l - r - (cols - 1) * 2) / cols;
      for (let c = 0; c < cols; c++) {
        for (let y = y0 + t; y < y0 + H - bt - 1; y += 3.2) inner += `<rect x="${x0 + l + c * (cw + 2)}" y="${y}" width="${cw}" height="1.6" fill="currentColor" opacity=".45"/>`;
      }
    }
    return `<svg aria-hidden="true" viewBox="0 0 48 44" width="48" height="44"><rect x="${x0}" y="${y0}" width="${W}" height="${H}" rx="1.5" fill="var(--sheet)" stroke="currentColor" stroke-width="1"/>${inner}</svg>`;
  };
  const margin = (key, label) => {
    const input = h('input', { class: 'field', type: 'number', min: 0, step, value: show(page.margins[key]), id: `ps-${key}` });
    input.addEventListener('change', () => set({ margins: { ...page.margins, [key]: Math.max(0, parseFloat(input.value) || 0) * k } }));
    return h('div', {}, h('label', { class: 'lbl', for: `ps-${key}` }, `${label} (${unit})`), input);
  };
  const custom = h('details', { class: 'eq-advanced' });
  const draw = () => {
    const [pw, ph] = [Math.min(page.width, page.height), Math.max(page.width, page.height)];
    const m = page.margins;
    const isMargin = ([, tb, side]) => near(m.top, tb, 2) && near(m.bottom, tb, 2) && near(m.left, side, 2) && near(m.right, side, 2);
    const wasOpen = custom.open;
    fill(custom, h('summary', {}, 'Exact margins'), h('div', { class: 'form-grid' }, margin('top', 'Top'), margin('bottom', 'Bottom'), margin('left', 'Left'), margin('right', 'Right')));
    custom.open = wasOpen || !MARGINS.some(isMargin);
    fill(body,
      sections > 1 ? h('p', { class: 'hint-text' }, 'For the section the cursor is in.') : null,
      h('div', { class: 'lbl' }, 'Paper'),
      h('div', { class: 'tiles five' }, SIZES.map(([n, sw, sh]) => tile(sheet(sw, sh), n === 'Letter' ? 'Letter (US)' : n === 'Legal' ? 'Legal (US)' : n, sizeOf() === n,
        () => set(land() ? { width: sh, height: sw } : { width: sw, height: sh })))),
      h('div', { class: 'lbl' }, 'Orientation'),
      h('div', { class: 'tiles' }, tile(sheet(pw, ph, m), 'Portrait', !land(), () => set({ width: pw, height: ph })),
        tile(sheet(ph, pw, m), 'Landscape', land(), () => set({ width: ph, height: pw }))),
      h('div', { class: 'lbl' }, 'Margins'),
      h('div', { class: 'tiles' }, MARGINS.map(([n, tb, side]) => tile(sheet(page.width, page.height, { top: tb, bottom: tb, left: side, right: side }), n, isMargin([n, tb, side]),
        () => set({ margins: { top: tb, bottom: tb, left: side, right: side } })))),
      custom,
      h('div', { class: 'lbl' }, 'Columns'),
      h('div', { class: 'tiles' }, [1, 2, 3].map((n) => tile(sheet(page.width, page.height, m, n), n === 1 ? 'One' : n === 2 ? 'Two' : 'Three', (page.columns || 1) === n, () => set({ columns: n })))),
      (page.columns || 1) > 1 ? [
        h('div', { class: 'lbl' }, 'Space between columns'),
        h('div', { class: 'tiles' }, choose([[12, 'Narrow'], [24, 'Normal'], [48, 'Wide']], page.column_gap, ([v, n], on) => tile(gapSketch(v), n, on, () => set({ column_gap: v })))),
        h('div', { class: 'chips' }, h('button', { class: `chip${page.column_line ? ' on' : ''}`, 'aria-pressed': String(!!page.column_line), onClick: () => set({ column_line: !page.column_line }) }, 'A line between them')),
      ] : null,
      h('div', { class: 'lbl' }, 'Text on each page'),
      h('div', { class: 'tiles three' }, [['top', 'At the top'], ['center', 'Centred'], ['bottom', 'At the bottom']].map(([v, n]) =>
        tile(alignSketch(v), n, (page.v_align || 'top') === v, () => set({ v_align: v })))));
  };
  // Two columns of lines with a gap of `g` points.
  const gapSketch = (g) => {
    const gap = 2 + g / 8;
    const w = (40 - gap) / 2;
    return sketch([0, 1].flatMap((c) => [0, 1, 2, 3, 4].map((k) => [4 + c * (w + gap), 6 + k * 5, w, false])), { h: 34 });
  };
  // A page with a short block of text at the top, middle or bottom.
  const alignSketch = (v) => {
    const y0 = v === 'top' ? 7 : v === 'center' ? 16 : 25;
    return `<svg aria-hidden="true" viewBox="0 0 48 44" width="48" height="44"><rect x="12" y="3" width="24" height="38" rx="1.5" fill="var(--sheet)" stroke="currentColor"/>${[0, 1, 2].map((k) => `<rect x="16" y="${y0 + k * 4}" width="${k === 2 ? 10 : 16}" height="1.8" fill="currentColor" opacity=".55"/>`).join('')}</svg>`;
  };
  draw();
  const undo = async () => { await run('set_page', { page: original }); pages.drawn.clear(); };
  modalShell('Page setup', body, [['Cancel', 'cancel'], ['Done', null, true]], { side: true, onCancel: undo });
}

// ----- Headers, footers and page numbers -----

const FIELDS = [['{page}', 'Page number'], ['{pages}', 'Number of pages'], ['{title}', 'Title'], ['{date}', 'Date']];

/// Double-click a page's top or bottom margin: its header or footer opens
/// there to type in (left, centre and right), with a bar for page numbers
/// and where it applies.
async function headerEditor(where, pageIndex) {
  closeHeaderEditor();
  const { page, section } = await call('page');
  const v = app.view;
  const pg = v?.pages.find((p) => p.index === pageIndex);
  if (!pg) return;
  const z = pages.zoom;
  let oddEven = !!page.different_odd_even;
  // Even pages have their own when odd and even differ.
  const key = () => (oddEven && pg.number % 2 === 0 ? `even_${where}` : where);
  const line = { left: '', center: '', right: '', ...(page[key()] || {}) };
  let first = !!page.different_first;
  let scope = 'all';
  const [bx, by, bw, bh] = pg.body;
  // The band between the page edge and the text, where it's drawn.
  const top = where === 'header' ? pg.y + Math.max(8, by / 2 - 14) : pg.y + by + bh + Math.max(8, (pg.h - by - bh) / 2 - 14);
  const band = h('div', { class: 'hf-edit', style: `left: ${pages.originX + (pg.x + bx) * z}px; top: ${pages.originY + top * z}px; width: ${bw * z}px` });
  let lastInput = null;
  const inputs = ['left', 'center', 'right'].map((k) => {
    const i = h('input', { class: `hf-in ${k}`, value: line[k] || '', 'aria-label': `${where === 'header' ? 'Header' : 'Footer'}, ${k === 'center' ? 'centre' : k}`,
      placeholder: k === 'left' ? 'Left' : k === 'center' ? 'Centre' : 'Right' });
    i.addEventListener('focus', () => { lastInput = i; });
    i.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); save(); }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeHeaderEditor(); }
    });
    i.dataset.key = k;
    return i;
  });
  const keep = (b) => { b.addEventListener('mousedown', (e) => e.preventDefault()); return b; };
  const chip = (label, act, on) => keep(h('button', { class: `chip${on ? ' on' : ''}`, onClick: act }, label));
  const bar = h('div', { class: 'hf-bar' });
  const drawBar = () => fill(bar,
    FIELDS.map(([t, n]) => chip(`+ ${n}`, () => {
      const i = lastInput || inputs[where === 'footer' ? 1 : 2];
      const at = i.selectionStart ?? i.value.length;
      i.value = i.value.slice(0, at) + t + i.value.slice(i.selectionEnd ?? at);
      i.focus();
    })),
    h('span', { class: 'sep' }),
    chip('Not on the first page', () => { first = !first; drawBar(); }, first),
    chip('Odd and even pages differ', () => { oddEven = !oddEven; drawBar(); }, oddEven),
    pageIndex > 0 || section > 0 ? chip(`From page ${pageIndex + 1} on`, () => { scope = scope === 'all' ? 'here' : 'all'; drawBar(); }, scope === 'here') : null,
    h('span', { style: 'flex: 1 1 0' }),
    keep(h('button', { class: 'btn small', onClick: () => closeHeaderEditor() }, 'Cancel')),
    keep(h('button', { class: 'btn small main', onClick: () => save() }, 'Done')));
  drawBar();
  band.append(h('div', { class: 'hf-inputs' }, inputs), bar);
  if (where === 'footer') band.classList.add('footer');
  band.addEventListener('pointerdown', (e) => e.stopPropagation());
  pages.overlay.append(band);
  hfEdit = band;
  const save = async () => {
    const value = Object.fromEntries(inputs.map((i) => [i.dataset.key, i.value.trim()]));
    closeHeaderEditor();
    const patch = { [key()]: value, different_first: first, different_odd_even: oddEven, link_to_previous: false };
    if (scope === 'here') {
      await run('section_from_page');
      await run('set_page', { page: patch });
    } else {
      await run('set_page', { page: patch, all: true });
    }
    pages.drawn.clear();
  };
  inputs[1].focus();
}
let hfEdit = null;
function closeHeaderEditor() { hfEdit?.remove(); hfEdit = null; }
// Double-click in the top or bottom margin: edit that page's header or footer.
onStacks('dblclick', (e) => {
  const p = pages.docPoint(e);
  const pg = app.view?.pages.find((x) => p.x >= x.x && p.x <= x.x + x.w && p.y >= x.y && p.y <= x.y + x.h);
  if (!pg) return;
  const y = p.y - pg.y;
  const where = y < pg.body[1] ? 'header' : y > pg.body[1] + pg.body[3] ? 'footer' : null;
  if (!where) return;
  e.stopImmediatePropagation();
  headerEditor(where, pg.index);
}, true);
onStacks('pointerdown', (e) => { if (hfEdit && !hfEdit.contains(e.target)) closeHeaderEditor(); }, true);


// ----- Visual pickers: pictures of the choice, not numbers -----

/// A small drawing of a page's text: `lines` is a list of [x, y, width, bold].
function sketch(lines, { w = 64, h = 44, extra = '' } = {}) {
  const bars = lines.map(([x, y, lw, bold]) => `<rect x="${x}" y="${y}" width="${lw}" height="${bold ? 3.2 : 2.2}" rx="1" fill="currentColor" opacity="${bold ? 0.9 : 0.55}"/>`).join('');
  return `<svg aria-hidden="true" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}">${bars}${extra}</svg>`;
}
/// One choice: its picture and name; `on` when it's the current one.
function tile(pic, label, on, act) {
  const b = h('button', { class: `tile${on ? ' on' : ''}`, 'aria-pressed': String(!!on), onClick: act }, h('span', { class: 'pic', html: pic }), h('span', { class: 'name' }, label));
  b.addEventListener('mousedown', (e) => e.preventDefault());
  return b;
}
const near = (a, b, d = 0.6) => Math.abs(a - b) < d;
/// The choices, the one nearest `value` marked as on.
function choose(options, value, make) {
  let best = 0;
  options.forEach(([v], k) => { if (Math.abs(v - value) < Math.abs(options[best][0] - value)) best = k; });
  return options.map((o, k) => make(o, k === best));
}

// Spacing and indents for the selected paragraphs; each choice applies straight away.
async function spacingPicker(anchor = tb.spacing) {
  let p = await call('para');
  const body = h('div', { class: 'pick-pop', role: 'dialog', 'aria-label': 'Spacing and indents' });
  const set = async (patch) => { await run('set_para', { patch }); p = await call('para'); draw(); };
  const lines = (gap) => sketch([0, 1, 2, 3].map((k) => [4, 6 + k * gap, 56, false]), { h: 8 + 3 * gap + 6 });
  const paras = (gap) => sketch([[4, 4, 56], [4, 9, 40], [4, 14 + gap, 56], [4, 19 + gap, 44]], { h: 26 + gap });
  const draw = () => fill(body,
    h('div', { class: 'lbl' }, 'Line spacing'),
    h('div', { class: 'tiles' }, choose([[1, 'Single', 6], [1.15, '1.15', 7], [1.5, '1.5', 9], [2, 'Double', 12]], p.line_height, ([v, n, gap], on) =>
      tile(lines(gap), n, on, () => set({ line_height: v, line_exact: null })))),
    h('div', { class: 'lbl' }, 'Space after each paragraph'),
    h('div', { class: 'tiles' }, choose([[0, 'None', 1], [6, 'Small', 4], [12, 'Medium', 8], [24, 'Large', 14]], p.space_after, ([v, n, gap], on) =>
      tile(paras(gap), n, on, () => set({ space_after: v })))),
    h('div', { class: 'lbl' }, 'First line'),
    h('div', { class: 'tiles' },
      tile(sketch([[4, 6, 56], [4, 12, 56], [4, 18, 40]], { h: 26 }), 'Even', near(p.first_line, 0), () => set({ first_line: 0 })),
      tile(sketch([[14, 6, 46], [4, 12, 56], [4, 18, 40]], { h: 26 }), 'Indented', p.first_line > 0.5, () => set({ first_line: 18 })),
      tile(sketch([[4, 6, 56], [14, 12, 46], [14, 18, 30]], { h: 26 }), 'Hanging', p.first_line < -0.5, () => set({ first_line: -18, indent_left: Math.max(p.indent_left, 18) }))),
    h('div', { class: 'lbl' }, 'On the page'),
    h('div', { class: 'chips' },
      h('button', { class: `chip${p.keep_with_next ? ' on' : ''}`, 'aria-pressed': String(!!p.keep_with_next), onClick: () => set({ keep_with_next: !p.keep_with_next }) }, 'Keep with the next paragraph'),
      h('button', { class: `chip${p.keep_together ? ' on' : ''}`, 'aria-pressed': String(!!p.keep_together), onClick: () => set({ keep_together: !p.keep_together }) }, 'Keep its lines together'),
      h('button', { class: `chip${p.page_break_before ? ' on' : ''}`, 'aria-pressed': String(!!p.page_break_before), onClick: () => set({ page_break_before: !p.page_break_before }) }, 'Start on a new page')),
    h('p', { class: 'hint-text' }, 'Drag the markers on the ruler for exact indents.'));
  draw();
  popup(anchor, body, { width: 360 });
}

/// More or less indent: a list nests (like Tab), other text moves by 1.27 cm.
async function indentBy(dir) {
  if (st().list) { await run('key', { key: 'Tab', mods: { shift: dir < 0 } }); return; }
  const p = await call('para');
  await run('set_para', { patch: { indent_left: Math.max(0, p.indent_left + dir * 36) } });
}

// ----- "/" commands: type / and a word to put something in -----

/// [word, what it shows, more words to find it by, what it does after the typed word is gone, group].
/// The common ones first: a bare "/" shows them all, in groups, to scroll through.
const cleared = (f) => async () => { await run('slash', { cmd: 'clear' }); await f(); };
const SLASH = [
  ['table', 'Table', 'grid rows columns', () => run('slash', { cmd: 'table' }), 'Common'],
  ['image', 'Picture', 'photo image picture', cleared(() => insertPicture()), 'Common'],
  ['server', 'From a server (canvas, table, chart, plan)', 'draw canvas calc table chart summary plan timeline issues milestones embed link picture', cleared(() => insertFromServer()), 'Common'],
  ['h1', 'Heading 1', 'title heading', cleared(() => run('style', { id: 'heading1' })), 'Common'],
  ['h2', 'Heading 2', 'heading', cleared(() => run('style', { id: 'heading2' })), 'Common'],
  ['h3', 'Heading 3', 'heading', cleared(() => run('style', { id: 'heading3' })), 'Common'],
  ['bullets', 'Bulleted list', 'list', cleared(() => run('list', { kind: 'bullets' })), 'Common'],
  ['numbers', 'Numbered list', 'list ordered', cleared(() => run('list', { kind: 'numbers' })), 'Common'],
  ['todo', 'Checklist', 'task tick checkbox', cleared(() => run('list', { kind: 'checklist' })), 'Common'],
  ['link', 'Link', 'url web address', cleared(() => linkEditor()), 'Common'],
  ['toc', 'Table of contents', 'contents', () => run('slash', { cmd: 'toc' }), 'Common'],
  ['divider', 'Divider line', 'rule line separator hr', cleared(() => run('divider')), 'Layout'],
  ['pagebreak', 'Page break', 'new page break', cleared(() => run('insert', { what: 'page_break' })), 'Layout'],
  ['textbox', 'Text box', 'frame box', cleared(() => run('frame_insert')), 'Layout'],
  ['quote', 'Quote', 'quotation', cleared(() => run('style', { id: 'quote' })), 'Layout'],
  ['code', 'Code', 'monospace program', cleared(() => run('style', { id: 'code' })), 'Layout'],
  ['info', 'Info panel', 'callout box tip', () => run('slash', { cmd: 'info' }), 'Panels'],
  ['note', 'Note panel', 'callout box', () => run('slash', { cmd: 'note' }), 'Panels'],
  ['success', 'Success panel', 'callout done ok green', () => run('slash', { cmd: 'success' }), 'Panels'],
  ['warning', 'Warning panel', 'callout caution', () => run('slash', { cmd: 'warning' }), 'Panels'],
  ['alert', 'Alert panel', 'callout error danger red', () => run('slash', { cmd: 'alert' }), 'Panels'],
  ['date', 'Today’s date', 'day today', () => run('slash', { cmd: 'date' }), 'Dates and labels'],
  ['time', 'The time now', 'clock hour', () => run('slash', { cmd: 'time' }), 'Dates and labels'],
  ['datetime', 'Date and time', 'now timestamp', () => run('slash', { cmd: 'datetime' }), 'Dates and labels'],
  ['badge', 'Badge (a coloured label)', 'status lozenge tag label', () => run('slash', { cmd: 'badge' }), 'Dates and labels'],
  ['equation', 'Equation', 'maths math formula', cleared(() => equationEditor(null)), 'Insert'],
  ['symbol', 'Symbol or emoji', 'emoji character', cleared(() => symbolsDialog()), 'Insert'],
  ['comment', 'Comment', 'review', cleared(() => newComment()), 'Insert'],
  ['footnote', 'Footnote', 'note reference', () => run('slash', { cmd: 'footnote' }), 'References'],
  ['endnote', 'Endnote', 'note reference end', cleared(() => run('insert', { what: 'endnote' })), 'References'],
  ['cite', 'Citation', 'reference source cite', cleared(() => citeDialog(refsEnv())), 'References'],
  ['bibliography', 'Bibliography', 'references sources works cited', cleared(() => run('bibliography')), 'References'],
  ['page', 'Page number', 'number', () => run('slash', { cmd: 'page' }), 'Pages'],
  ['header', 'Header', 'top of page', cleared(() => headerEditor('header', app.view?.caret_page ?? 0)), 'Pages'],
  ['footer', 'Footer', 'bottom of page', cleared(() => headerEditor('footer', app.view?.caret_page ?? 0)), 'Pages'],
  ['titlepage', 'Title page', 'cover front', cleared(() => run('insert', { what: 'title_page' })), 'Pages'],
  ['changelog', 'Change log', 'history versions', cleared(() => run('insert', { what: 'change_log' })), 'Pages'],
  ['field', 'Template field', 'template placeholder fill in blank {{', cleared(() => templateField()), 'Templates'],
  ['body', 'Where the text goes (template)', 'template body content slot', cleared(() => markBody()), 'Templates'],
];
const slash = { el: null, typed: '', items: [], at: 0 };
function closeSlash() { slash.el?.remove(); slash.el = null; }
/// After each change: "/word" just before the cursor (at the start or after a space) opens the list.
function checkSlash(v) {
  const [a, b] = v.para_sel || [0, 0];
  const before = a === b ? (v.para_text || '').slice(0, a) : '';
  const m = /(?:^|\s)(\/([a-z0-9]*))$/i.exec(before);
  if (!m || document.querySelector('.modal-back, .screen')) { closeSlash(); return; }
  const word = m[2].toLowerCase();
  const items = SLASH.filter(([w, label, more]) => !word || w.startsWith(word) || `${label} ${more}`.toLowerCase().split(/\s+/).some((x) => x.startsWith(word)));
  if (!items.length) { closeSlash(); return; }
  if (slash.typed !== m[1]) slash.at = 0;
  slash.typed = m[1];
  slash.items = items;
  drawSlash(v);
}
function drawSlash(v) {
  if (!slash.el) {
    slash.el = h('div', { class: 'popup slash-menu', role: 'listbox', 'aria-label': 'Put in' });
    $('popups').append(slash.el);
  }
  const grouped = slash.typed === '/';
  let last = null;
  fill(slash.el, slash.items.flatMap(([w, label, , , group], k) => {
    const b = h('button', { class: `item${k === slash.at ? ' active' : ''}`, role: 'option', 'aria-selected': String(k === slash.at), tabindex: '-1',
      onClick: () => doSlash(k) }, h('span', {}, label), h('span', { class: 'keys' }, `/${w}`));
    b.addEventListener('mousedown', (e) => e.preventDefault());
    const head = grouped && group !== last ? h('div', { class: 'menu-label' }, group) : (k === 0 ? h('div', { class: 'menu-label' }, 'Put in') : null);
    last = group;
    return head ? [head, b] : [b];
  }));
  slash.el.querySelector('.item.active')?.scrollIntoView({ block: 'nearest' });
  const at = screenRect(v.caret || [0, 0, 0, 0]);
  Object.assign(slash.el.style, { left: `${Math.min(at.left, window.innerWidth - 300)}px`, top: `${at.bottom + 6}px`, width: '280px' });
  if (at.bottom + 6 + slash.el.offsetHeight > window.innerHeight - 8) slash.el.style.top = `${Math.max(8, at.top - 6 - slash.el.offsetHeight)}px`;
}
async function doSlash(k) {
  const it = slash.items[k];
  closeSlash();
  // Each command's "slash" call takes the typed "/word" away first.
  if (!it) return;
  await it[3]();
  // Keys typed just before may still have been painting: draw it all again.
  pages.drawn.clear();
  await run('view');
}
// While the list is open: arrows move in it, Enter or Tab picks, Esc closes; typing carries on.
window.addEventListener('keydown', (e) => {
  if (!slash.el) return;
  const n = slash.items.length;
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault(); e.stopPropagation();
    slash.at = (slash.at + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
    drawSlash(app.view);
  } else if (e.key === 'Enter' || e.key === 'Tab') {
    e.preventDefault(); e.stopPropagation();
    doSlash(slash.at);
  } else if (e.key === 'Escape') {
    e.preventDefault(); e.stopPropagation();
    closeSlash();
  }
}, true);

/// Start a numbered list at any number (the item the cursor is in, and on from there).
function listStart(anchor, li) {
  const n = h('input', { class: 'field', type: 'number', min: 0, value: li.restart ?? (parseInt(li.label, 10) || 1), 'aria-label': 'Start at' });
  const go = () => { closePopup(); run('list_start', { at: Math.max(0, parseInt(n.value, 10) || 1) }); };
  n.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
  popup(anchor, h('div', { class: 'link-pop' }, h('label', { class: 'lbl' }, 'Start this list at'), n,
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: go }, 'Set'))), { width: 240 });
  n.focus();
  n.select();
}

/// A badge's text and colour, from its right-click menu.
function badgeEditor(b, anchor) {
  const text = h('input', { class: 'field', value: b.text, 'aria-label': 'Badge text' });
  let color = b.color;
  const body = h('div', { class: 'link-pop' });
  const save = () => { closePopup(); run('badge_set', { para: b.para, offset: b.offset, text: text.value || 'Status', color }); };
  const draw = () => fill(body, h('label', { class: 'lbl' }, 'Badge'), text,
    h('div', { class: 'chips' }, BADGES.map(([name, bg, fg]) => {
      const c = h('button', { class: `badge-pick${name === color ? ' on' : ''}`, style: `background: ${bg}; color: ${fg}`, 'aria-label': name, onClick: () => { color = name; draw(); } }, (text.value || 'Status').toUpperCase());
      c.addEventListener('mousedown', (e) => e.preventDefault());
      return c;
    })),
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: save }, 'Save')));
  text.addEventListener('input', () => { for (const c of body.querySelectorAll('.badge-pick')) c.textContent = (text.value || 'Status').toUpperCase(); });
  text.addEventListener('keydown', (e) => { if (e.key === 'Enter') save(); });
  draw();
  popup(anchor, body, { width: 320 });
  text.focus();
  text.select();
}
const BADGES = [['grey', '#E7E5E4', '#44403C'], ['blue', '#DBEAFE', '#1E40AF'], ['green', '#DCFCE7', '#166534'], ['yellow', '#FEF3C7', '#92400E'], ['red', '#FEE2E2', '#991B1B'], ['purple', '#EDE9FE', '#5B21B6']];
const PANEL_KINDS = [['info', 'Info'], ['note', 'Note'], ['success', 'Success'], ['warning', 'Warning'], ['alert', 'Alert']];

// ----- Date and time: pick how it's written and whether it keeps up to date -----

/// Insert › Date and time, or the options of one already there (`at`).
async function dateTimeDialog(at = null) {
  const presets = await call('datetime_presets');
  let format = at?.format || presets[0].format;
  // Dates are fixed (an old document's self-updating one keeps doing so until changed here).
  const auto = false;
  const body = h('div', { class: 'pick-pop dt-pop' });
  const custom = h('input', { class: 'field mono', 'aria-label': 'Format', placeholder: 'e.g. dddd d MMMM yyyy' });
  custom.value = presets.some((p) => p.format === format) ? '' : format;
  custom.addEventListener('input', () => { if (custom.value.trim()) { format = custom.value; draw(); } });
  const draw = () => fill(body,
    h('div', { class: 'lbl' }, 'How it’s written'),
    h('div', { class: 'dt-list' }, presets.map((p) => {
      const b = h('button', { class: `dt-item${p.format === format ? ' on' : ''}`, 'aria-pressed': String(p.format === format), onClick: () => { format = p.format; custom.value = ''; draw(); } }, p.example);
      b.addEventListener('mousedown', (e) => e.preventDefault());
      return b;
    })),
    h('details', { class: 'eq-advanced', open: !!custom.value || null }, h('summary', {}, 'Your own format'),
      h('p', { class: 'hint-text' }, 'd day, dddd weekday, MMMM month, yyyy year, HH:mm 24-hour time, h:mm am/pm.'), custom),
    at ? null : h('p', { class: 'hint-text' }, 'It stays as it is now. To change it later, right-click it and choose Update it to now.'));
  draw();
  modalShell(at ? 'Date and time options' : 'Date and time', body, [
    ['Cancel', null],
    [at ? 'Save' : 'Insert', async () => {
      if (at) await run('datetime_set', { para: at.para, offset: at.offset, format, auto });
      else await run('insert', { what: 'datetime', format, auto });
    }, true],
  ]);
}

// ----- Table of contents: pick how many heading levels; shown outlined on screen -----

function contentsChoices(cur, act) {
  const pic = (levels, nums) => sketch([[4, 5, 30, true], ...(levels > 1 ? [[12, 11, 26]] : []), ...(levels > 2 ? [[20, 17, 18]] : []), [4, levels > 2 ? 23 : levels > 1 ? 17 : 11, 34, true]],
    { h: 30, extra: nums ? '<g fill="currentColor" opacity=".5"><rect x="50" y="5" width="8" height="2.2"/><rect x="50" y="11" width="8" height="2.2"/><rect x="50" y="17" width="8" height="2.2"/><rect x="50" y="23" width="8" height="2.2"/></g>' : '' });
  return [
    h('div', { class: 'lbl' }, 'Which headings'),
    h('div', { class: 'tiles three' }, [[1, 'Level 1'], [2, 'Levels 1–2'], [3, 'Levels 1–3']].map(([n, label]) => tile(pic(n, cur.page_numbers), label, cur.levels === n, () => act({ levels: n })))),
    h('div', { class: 'lbl' }, 'Page numbers'),
    h('div', { class: 'tiles three' }, tile(pic(cur.levels, true), 'Shown', cur.page_numbers, () => act({ page_numbers: true })), tile(pic(cur.levels, false), 'Left off', !cur.page_numbers, () => act({ page_numbers: false }))),
  ];
}
/// Insert › Table of contents.
function tocDialog() {
  const cur = { levels: 3, page_numbers: true };
  const body = h('div', { class: 'pick-pop', role: 'dialog', 'aria-label': 'Table of contents' });
  const draw = () => fill(body, h('h2', {}, 'Table of contents'),
    h('p', { class: 'hint-text' }, 'Lists the headings in the document and keeps itself up to date as you write.'),
    contentsChoices(cur, (patch) => { Object.assign(cur, patch); draw(); }),
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'),
      h('button', { class: 'btn main', onClick: async () => {
        closePopup();
        await run('insert', { what: 'toc', levels: cur.levels });
        if (!cur.page_numbers) { const b = (await call('contents_boxes')).find((x) => x.kind === 'contents' && x.page_numbers); if (b) await run('contents_set', { id: b.id, page_numbers: false }); }
        refreshContents();
      } }, 'Insert')));
  draw();
  popup(selectionAnchor(), body, { width: 360 });
}
/// Settings of a table of contents already in the document.
function contentsOptions(box, anchor) {
  const cur = { levels: box.levels, page_numbers: box.page_numbers };
  const body = h('div', { class: 'pick-pop', role: 'dialog', 'aria-label': 'Table of contents' });
  const draw = () => fill(body, h('h2', {}, 'Table of contents'),
    contentsChoices(cur, async (patch) => { Object.assign(cur, patch); await run('contents_set', { id: box.id, ...patch }); draw(); refreshContents(); }),
    h('div', { class: 'pop-actions' },
      h('button', { class: 'btn', onClick: async () => { closePopup(); await run('delete_block', { id: box.id }); refreshContents(); } }, 'Remove it'),
      h('button', { class: 'btn main', onClick: () => closePopup() }, 'Done')));
  draw();
  popup(anchor, body, { width: 360 });
}
// Column guides: with two or more columns each one is outlined faintly on
// screen, so you can see them before there's enough text to fill them.
const guideLayer = h('div', { class: 'gen-layer' });
pages.overlay.append(guideLayer);
function placeGuides() {
  const v = app.view;
  if (!v) return;
  const z = pages.zoom;
  fill(guideLayer, v.pages.filter((p) => p.columns > 1).flatMap((p) => {
    const [bx, by, bw, bh] = p.body;
    const w = (bw - p.column_gap * (p.columns - 1)) / p.columns;
    return [...Array(p.columns).keys()].map((c) => h('div', { class: 'col-guide', style:
      `left: ${pages.originX + (p.x + bx + c * (w + p.column_gap)) * z}px; top: ${pages.originY + (p.y + by) * z}px; width: ${w * z}px; height: ${bh * z}px` }));
  }));
}

// ----- Tables: handles on the edges, + to add, drag a line to resize -----

const tableLayer = h('div', { class: 'gen-layer tbl-tools' });
pages.overlay.append(tableLayer);
let tablesOnScreen = [];
let tableHover = null;
let tablesTimer = null;
async function refreshTables() { tablesOnScreen = await call('tables_on_screen').catch(() => []); placeTables(); }
function refreshTablesSoon() { clearTimeout(tablesTimer); tablesTimer = setTimeout(refreshTables, 150); }
// Pointing at a table shows its controls (as being in it does).
onStacks('pointermove', (e) => {
  if (e.buttons) return;
  const p = pages.docPoint(e);
  const t = tablesOnScreen.find((x) => p.x >= x.rect[0] - 24 && p.x <= x.rect[0] + x.rect[2] + 8 && p.y >= x.rect[1] - 24 && p.y <= x.rect[1] + x.rect[3] + 8);
  const id = t?.id ?? null;
  if (id !== tableHover) { tableHover = id; placeTables(); }
});
function placeTables() {
  const z = pages.zoom;
  const X = (x) => pages.originX + x * z, Y = (y) => pages.originY + y * z;
  const keep = (el) => { el.addEventListener('pointerdown', (e) => { e.stopPropagation(); e.preventDefault(); }); return el; };
  const plus = (title, x, y, act, col = false) => keep(h('button', { class: `tbl-plus${col ? ' col' : ''}`, title, 'aria-label': title, style: `left: ${x - 8}px; top: ${y - 8}px`, onClick: act }, '+'));
  fill(tableLayer, tablesOnScreen.filter((t) => t.here || t.id === tableHover).map((t) => {
    const [tx, ty, tw, th] = t.rect;
    const out = [];
    // Where the menu goes is read first: picking redraws these controls.
    const at = (el) => { const r = el.getBoundingClientRect(); return { getBoundingClientRect: () => r, setAttribute() {}, contains: () => false }; };
    const menu = async (anchorEl, pick, items) => {
      const anchor = at(anchorEl);
      await run('table_pick', { id: t.id, ...pick });
      popup(anchor, items(), { width: 240 });
    };
    // Columns: a handle above each, + between them, and the lines between to drag.
    for (const [i, a, b] of t.cols) {
      const handle = keep(h('button', { class: 'tbl-handle col', title: 'Column', 'aria-label': `Column ${i + 1}`, style: `left: ${X(a) + 1}px; top: ${Y(ty) - 14}px; width: ${(b - a) * z - 2}px` }));
      handle.addEventListener('click', () => menu(handle, { col: i }, () => [
        item('Insert a column left', { run: () => run('table_add', { id: t.id, col: i }) }),
        item('Insert a column right', { run: () => run('table_add', { id: t.id, col: i + 1 }) }),
        item('Delete this column', { run: () => run('table', { what: 'delete_cols' }) }),
        h('hr'),
        item('Sort A to Z by this column', { run: () => run('table', { what: 'sort' }) }),
        item('Sort Z to A', { run: () => run('table', { what: 'sort_desc' }) }),
        item('Even column widths', { run: () => run('table', { what: 'distribute' }) }),
        ...(i === 0 ? [h('hr'), item('First column as headers on or off', { run: () => run('table', { what: 'header_col' }) })] : []),
      ]));
      out.push(handle, plus('Add a column here', X(a), Y(ty) - 22, () => run('table_add', { id: t.id, col: i }), true));
    }
    const lastCol = t.cols[t.cols.length - 1];
    if (lastCol) out.push(plus('Add a column at the end', X(lastCol[2]), Y(ty) - 22, () => run('table_add', { id: t.id, col: lastCol[0] + 1 }), true));
    for (const [i, a, b] of t.rows) {
      const handle = keep(h('button', { class: 'tbl-handle row', title: 'Row', 'aria-label': `Row ${i + 1}`, style: `left: ${X(tx) - 14}px; top: ${Y(a) + 1}px; height: ${(b - a) * z - 2}px` }));
      handle.addEventListener('click', () => menu(handle, { row: i }, () => [
        item('Insert a row above', { run: () => run('table_add', { id: t.id, row: i }) }),
        item('Insert a row below', { run: () => run('table_add', { id: t.id, row: i + 1 }) }),
        item('Delete this row', { run: () => run('table', { what: 'delete_rows' }) }),
        h('hr'),
        item('Header row on or off', { run: () => run('table', { what: 'header_row' }) }),
      ]));
      out.push(handle, plus('Add a row here', X(tx) - 22, Y(a), () => run('table_add', { id: t.id, row: i })));
    }
    const lastRow = t.rows[t.rows.length - 1];
    if (lastRow) out.push(plus('Add a row at the end', X(tx) - 22, Y(lastRow[2]), () => run('table_add', { id: t.id, row: lastRow[0] + 1 })));
    // The lines between columns: drag to make one wider and its neighbour narrower.
    t.cols.slice(0, -1).forEach(([i, , b]) => {
      const line = keep(h('div', { class: 'tbl-resize', title: 'Drag to resize', style: `left: ${X(b) - 3}px; top: ${Y(ty)}px; height: ${th * z}px` }));
      line.addEventListener('pointerdown', (e) => {
        line.setPointerCapture(e.pointerId);
        const sx = e.clientX;
        const total = t.widths.reduce((s2, w) => s2 + w, 0);
        const per = total / (tw * z);
        let widths = t.widths.slice();
        const move = (ev) => {
          const d = (ev.clientX - sx) * per;
          const min = total * 0.04;
          const a2 = Math.max(min, t.widths[i] + d), b2 = Math.max(min, t.widths[i + 1] - (a2 - t.widths[i]));
          widths = t.widths.slice();
          widths[i] = t.widths[i] + t.widths[i + 1] - b2;
          widths[i + 1] = b2;
          line.style.left = `${X(b) - 3 + (widths[i] - t.widths[i]) / per}px`;
        };
        line.addEventListener('pointermove', move);
        line.addEventListener('pointerup', async () => {
          line.removeEventListener('pointermove', move);
          if (widths.some((w, k) => Math.abs(w - t.widths[k]) > 1e-6)) await run('table_widths', { id: t.id, widths });
          refreshTables();
        }, { once: true });
      });
      out.push(line);
    });
    // The whole table: options and delete.
    const more = keep(h('button', { class: 'tbl-more', title: 'Table', 'aria-label': 'Table options', style: `left: ${X(tx) - 16}px; top: ${Y(ty) - 16}px`, html: '<svg aria-hidden="true" viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>' }));
    more.addEventListener('click', async () => {
      const anchor = at(more);
      await run('table_pick', { id: t.id, row: 0 });
      const info = await call('table_info');
      const linked = info?.linked;
      // A table from a plan or a Calc book is theirs: open it there to change it.
      const node = /^[^|]+\|(\d+)\|/.exec(info?.link || '')?.[1];
      popup(anchor, [linked ? null : item('Table options…', { run: () => tableDialog() }),
        node ? item('Open it (to change it there)', { title: 'The plan or Calc book this table shows: what changes there shows here', run: () => openLinkedSource(Number(node)) }) : null,
        linked ? item('Keep this copy (stop updating)', { title: 'It becomes the document’s own, to change here; it then stays as it is now, even when the plan or Calc book changes', run: () => run('calc_link_keep', { id: t.id }) }) : null,
        item('Delete the table', { run: () => run('table', { what: 'delete_table' }) })].filter(Boolean), { width: 260 });
    });
    out.push(more);
    return out;
  }));
}

const contentsLayer = h('div', { class: 'gen-layer' });
pages.overlay.append(contentsLayer);
let contentsBoxes = [];
async function refreshContents() {
  contentsBoxes = await call('contents_boxes').catch(() => []);
  placeContents();
}
let contentsTimer = null;
function refreshContentsSoon() { clearTimeout(contentsTimer); contentsTimer = setTimeout(refreshContents, 300); }
const GEN_NAMES = { contents: 'Table of contents', bibliography: 'Bibliography', change_log: 'Change log', figures: 'List of figures' };
function placeContents() {
  const z = pages.zoom;
  fill(contentsLayer, contentsBoxes.map((b) => {
    const [x, y, w, hh] = [pages.originX + b.rect[0] * z - 8, pages.originY + b.rect[1] * z - 14, b.rect[2] * z + 16, b.rect[3] * z + 22];
    const label = h('button', { class: 'gen-label', onClick: (e) => {
      e.stopPropagation();
      if (b.kind === 'contents') contentsOptions(b, label);
      else popup(label, [item(`Remove the ${GEN_NAMES[b.kind].toLowerCase()}`, { run: async () => { await run('delete_block', { id: b.id }); refreshContents(); } })], { width: 260 });
    } }, GEN_NAMES[b.kind], b.kind === 'contents' ? h('span', { html: icon('down', 12) }) : null);
    label.addEventListener('pointerdown', (e) => e.stopPropagation());
    return h('div', { class: `gen-box${b.empty ? ' empty' : ''}`, style: `left: ${x}px; top: ${y}px; width: ${w}px; height: ${hh}px` }, label);
  }));
}

// ----- Insert -----

async function insertPicture() {
  const info = await call('save_info');
  const path = await window.__TAURI__.core.invoke('choose', { kind: 'picture', dir: info.dir });
  if (path) await run('insert_picture', { path });
  if (path) await copyInForServer();
}

/// A picture from a file on this computer, put in a document on a server:
/// copied in, since the others don't have the file.
async function copyInForServer() {
  if (app.tabs?.tabs.find((t) => t.id === app.tabs.front)?.remote == null) return;
  const out = await run('remote_copy_in');
  if (out?.copied) toast('Copied into the document, so everyone on the server sees it (it won’t follow changes to the file)');
  else if (out?.not_copied?.length) toast(out.not_copied[0]);
}

/// The size grid: under the toolbar button (`from`), else at the cursor.
function tablePicker(from = null) {
  const anchor = from || selectionAnchor();
  let r = 3, c = 3;
  const label = h('div', { class: 'table-size' }, '3 × 3 table');
  const grid = h('div', { class: 'table-grid', role: 'grid', 'aria-label': 'Table size' });
  const cells = [];
  for (let i = 0; i < 8; i++) for (let j = 0; j < 8; j++) {
    const b = h('button', { 'aria-label': `${i + 1} rows by ${j + 1} columns`, onClick: () => { closePopup(); run('insert', { what: 'table', rows: i + 1, cols: j + 1 }); } });
    b.addEventListener('pointerenter', () => { r = i + 1; c = j + 1; paint(); });
    b.addEventListener('focus', () => { r = i + 1; c = j + 1; paint(); });
    cells.push([i, j, b]);
    grid.append(b);
  }
  const paint = () => { for (const [i, j, b] of cells) b.classList.toggle('on', i < r && j < c); label.textContent = `${r} × ${c} table`; };
  paint();
  popup(anchor, [h('div', { class: 'menu-label' }, 'Insert a table'), grid, label], { width: 220 });
}

function linkEditor() {
  const input = h('input', { class: 'field', placeholder: 'e.g. https://example.com', 'aria-label': 'Web address' });
  // A page picked below: its address goes in the box, and its name is the
  // link's words when nothing is selected.
  let picked = null;
  const go = () => {
    const u = input.value.trim();
    closePopup();
    if (!u) return;
    const url = /^[a-z]+:/i.test(u) ? u : `https://${u}`;
    run('link', picked?.url === u ? { url, text: picked.text } : { url });
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
  // A document on a server: a link to one of its space's pages, picked by name.
  const node = server.nodeOf(app.tabs?.front);
  const pages = h('div', { class: 'link-pages' });
  const el = popup(selectionAnchor(), h('div', { class: 'link-pop' }, h('label', { class: 'lbl' }, 'Link to'), input, pages,
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onClick: () => closePopup() }, 'Cancel'), h('button', { class: 'btn main', onClick: go }, 'Add link'))), { width: 360 });
  input.focus();
  if (node != null) pagePicker(node, pages, (url, text, el) => {
    picked = { url, text };
    input.value = url;
    for (const b of pages.querySelectorAll('.link-page')) b.classList.toggle('on', b === el);
    input.focus();
  });
  return el;
}

/// The pages on the server (this document's space first), to link to:
/// typing in the box narrows them. A link names the page by its number, so
/// it still finds the page after it's moved or renamed.
async function pagePicker(node, box, pick) {
  try {
    const me = await server.api('GET', `/v1/nodes/${node}`);
    const named = (x) => { try { return JSON.parse(x.meta || '{}'); } catch { return {}; } };
    const spaces = (await server.api('GET', '/v1/spaces')).filter((sp) => sp.kind === 'shared' || sp.kind === 'personal');
    spaces.sort((a, b) => (b.id === me.space) - (a.id === me.space));
    const label = (sp) => (sp.kind === 'personal' ? 'My files' : (() => { try { return JSON.parse(sp.name).name; } catch { return ''; } })() || 'Untitled space');
    const list = [];
    for (const sp of spaces) {
      const all = await server.api('GET', `/v1/everything?space=${sp.id}`).catch(() => []);
      list.push(...all.filter((x) => x.id !== node && x.kind === 'file' && named(x).page === true)
        .map((x) => ({ id: x.id, name: String(named(x).name || 'Untitled').replace(/\.(xwrite|igdoc)$/i, ''), space: sp.id === me.space ? '' : label(sp) }))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true })));
    }
    if (!list.length) return;
    const input = box.parentElement.querySelector('input');
    const draw = () => {
      const q = input.value.trim().toLowerCase();
      // A web address being typed: no pages.
      // A page's address in the box (one was picked): all the pages again.
      const chosen = /\/files\/(#\/)?(wiki|page)\//.test(q);
      const shown = chosen ? list.slice(0, 8) : /^[a-z]+:|\./i.test(q) ? [] : list.filter((x) => !q || x.name.toLowerCase().includes(q) || x.space.toLowerCase().includes(q)).slice(0, 8);
      if (!shown.length) return fill(box);
      fill(box, h('div', { class: 'lbl' }, 'Or a page'), ...shown.map((x) => h('button', { class: 'link-page', onClick: (e) => pick(`${server.url}/files/page/${x.id}`, x.name, e.currentTarget) },
        x.name, x.space ? h('span', { class: 'hint' }, ` · ${x.space}`) : null)));
    };
    input.placeholder = 'A web address, or a page’s name';
    input.addEventListener('input', draw);
    draw();
  } catch { /* no pages to offer */ }
}

// ----- Look up: a word card next to the word -----

async function lookUp(point) {
  const w = await call('lookup', point || {});
  if (!w) { toast('Put the cursor on a word to look it up'); return; }
  const z = pages.zoom;
  const sr = pages.stack.getBoundingClientRect();
  const r = w.rect || [0, 0, 0, 0];
  const anchor = {
    getBoundingClientRect: () => ({ left: sr.left + pages.originX + r[0] * z, right: sr.left + pages.originX + (r[0] + r[2]) * z, bottom: sr.top + pages.originY + (r[1] + r[3]) * z }),
    setAttribute() {}, contains: () => false, classList: { contains: () => false },
  };
  let more = false;
  const body = h('div', { class: 'wordcard', role: 'dialog', 'aria-label': `Look up ${w.word}` });
  const swapBtn = (x) => {
    const b = h('button', { class: 'chip', onClick: async () => { closePopup(); await run('swap', { para: w.para, start: w.start, end: w.end, with: x }); } }, x);
    return b;
  };
  const meaning = (m, first) => h('div', { class: `meaning${first ? ' first' : ''}` },
    first ? null : h('div', { class: 'pos' }, m.pos),
    h('p', { class: 'def' }, m.definition),
    m.example ? h('p', { class: 'eg' }, `“${m.example}”`) : null,
    m.swaps.length ? [h('div', { class: 'swap-label' }, 'Swap for'), h('div', { class: 'chips' }, m.swaps.map(swapBtn))] : null,
    m.opposites.length ? h('div', { class: 'opp' }, 'Opposite: ', m.opposites.map((o, i) => [i ? ', ' : '', h('button', { class: 'link', onClick: () => { closePopup(); run('swap', { para: w.para, start: w.start, end: w.end, with: o }); } }, o)])) : null);
  const draw = () => {
    const ms = w.meanings;
    fill(body, ...[
      h('div', { class: 'head' }, h('span', { class: 'wc-word' }, w.base), ms[0] ? h('span', { class: 'pos' }, ms[0].pos) : null,
        w.pronunciation ? h('span', { class: 'say' }, w.pronunciation) : null),
      ms.length ? meaning(ms[0], true) : h('p', { class: 'def' }, 'Not in the dictionary. If it’s spelled right, right-click it and choose Add to my words.'),
      more ? ms.slice(1).map((m) => meaning(m, false)) : null,
      h('div', { class: 'foot' },
        ms.length > 1 ? h('button', { class: 'link', onClick: () => { more = !more; draw(); } }, more ? 'Fewer meanings' : `${ms.length - 1} more ${ms.length === 2 ? 'meaning' : 'meanings'}`) : h('span'),
        h('span', { class: 'hint' }, 'Shift+F7 or right-click a word'))].flat().filter((x) => x != null));
  };
  draw();
  popup(anchor, body, { width: 340 });
}

// Right-click: fixes for the mistake under the pointer, then what you can
// do to the text there (clipboard, comment, link, formatting, the table).
onStacks('contextmenu', async (e) => {
  e.preventDefault();
  const p = pages.docPoint(e);
  // A picture has its own menu (above).
  if (await call('image_at', p)) return;
  // Without a selection, right-click puts the cursor where you clicked first.
  if (app.view?.selected_text == null) { await run('down', { ...p, clicks: 1, mods: {} }); await run('up'); }
  const i = await call('issue_at', p);
  const anchor = anchorAt(e.clientX, e.clientY);
  const s = st();
  const hasSel = app.view?.selected_text != null;
  const items = [];
  if (i) {
    items.push(h('div', { class: 'menu-label' }, quoted(i.message)));
    for (const sug of i.suggestions.slice(0, 5)) items.push(item(sug === '' ? 'Remove it' : sug, { run: () => fixIssue(i.i, sug), style: 'font-weight: 600' }));
    items.push(item('Ignore', { run: () => ignoreIssue(i.i) }));
    if (i.spelling) items.push(item(`Add “${i.text}” to my words`, { run: () => addWord(i.text) }));
    if (i.spelling) items.push(item(`Add “${i.text}” to this document’s words`, { run: () => addDocWord(i.text) }));
    items.push(h('hr'));
  }
  // B, I, U, highlight and clear in one row.
  const quick = (name, label, act, pressed, content) => {
    const b = h('button', { class: 'icon-btn item', 'aria-label': label, title: label, 'aria-pressed': pressed == null ? null : String(!!pressed), html: content || icon(name), onClick: () => { closePopup(); act(); } });
    b.addEventListener('mousedown', (ev) => ev.preventDefault());
    return b;
  };
  items.push(h('div', { class: 'ctx-row' },
    quick('bold', 'Bold', () => run('toggle', { what: 'bold' }), s.bold, '<span style="font-weight: 800">B</span>'),
    quick('italic', 'Italic', () => run('toggle', { what: 'italic' }), s.italic, '<span style="font-style: italic; font-family: Georgia, serif; font-size: 17px">I</span>'),
    quick('underline', 'Underline', () => run('toggle', { what: 'underline' }), s.underline, '<span style="text-decoration: underline; text-underline-offset: 3px">U</span>'),
    quick('highlighter', 'Highlight', () => colourPicker('highlight')),
    quick('bullets', 'Bulleted list', () => run('list', { kind: 'bullets' }), s.list === 'bullets'),
    quick('numbers', 'Numbered list', () => run('list', { kind: 'numbers' }), s.list === 'numbers'),
    quick('eraser', 'Clear formatting', () => run('clear_format'))));
  items.push(h('hr'));
  items.push(item('Cut', { keys: 'Ctrl+X', enabled: hasSel, run: () => document.execCommand('cut') }));
  items.push(item('Copy', { keys: 'Ctrl+C', enabled: hasSel, run: () => document.execCommand('copy') }));
  items.push(item('Paste', { keys: 'Ctrl+V', run: async () => {
    try { const text = await navigator.clipboard.readText(); if (text) run('paste', { text }); } catch { toast('Use Ctrl+V to paste'); }
  } }));
  items.push(h('hr'));
  items.push(item(hasSel ? 'Comment on this…' : 'Comment…', { keys: 'Ctrl+Alt+M', run: () => newComment() }));
  items.push(item(s.link ? 'Edit link…' : 'Link…', { keys: 'Ctrl+K', run: () => linkEditor() }));
  items.push(item('Look up this word', { keys: 'Shift+F7', run: () => lookUp(p) }));
  items.push(h('hr'));
  items.push(item(`Style: ${styleName(s.style)}…`, { run: () => stylePicker(anchor) }));
  items.push(item('Spacing and indents…', { run: () => spacingPicker(anchor) }));
  const badge = await call('badge_at', p);
  if (badge) items.push(item('Change badge…', { run: () => badgeEditor(badge, anchor) }));
  if ((s.style || '').startsWith('panel-')) {
    items.push(h('div', { class: 'menu-label' }, 'Panel'), h('div', { class: 'ctx-row' }, PANEL_KINDS.map(([k, n]) => {
      const b = h('button', { class: `chip${s.style === `panel-${k}` ? ' on' : ''}`, onClick: () => { closePopup(); run('panel', { kind: k }); } }, n);
      b.addEventListener('mousedown', (e) => e.preventDefault());
      return b;
    })), item('Back to plain text', { run: () => run('panel', {}) }));
  }
  // A numbered item: where its numbering starts.
  const li = s.list ? await call('list_info') : null;
  if (li?.numbered && !li.follow && !li.headings) {
    items.push(h('div', { class: 'menu-label' }, 'Numbering'),
      item('Start this list at…', { run: () => listStart(anchor, li) }),
      li.restart == null ? item('Restart at 1', { run: () => run('list_start', { at: 1 }) }) : item('Continue numbering from the list before', { run: () => run('list_start', { at: null }) }));
  }
  const dt = await call('datetime_at', p);
  if (dt) {
    items.push(item('Date and time options…', { run: () => dateTimeDialog(dt) }));
    items.push(item('Update it to now', { run: () => run('datetime_set', { ...dt, auto: false }) }));
  }
  const gen = contentsBoxes.find((b) => p.x >= b.rect[0] && p.x <= b.rect[0] + b.rect[2] && p.y >= b.rect[1] && p.y <= b.rect[1] + b.rect[3]);
  if (gen?.kind === 'contents') items.push(item('Table of contents options…', { run: () => contentsOptions(gen, anchor) }));
  if (s.in_table) items.push(h('hr'), h('div', { class: 'menu-label' }, 'Table'), ...tableItems().filter((x) => x.tagName !== 'HR'));
  pages.focus();
  popup(anchor, items, { width: 300 });
});

// ----- Status bar -----

function renderStatus() {
  const v = app.view;
  if (!v) return;
  const n = app.issues?.length || 0;
  const s = v.state;
  const words = s.selected_words != null ? `${s.selected_words.toLocaleString()} of ${s.words.toLocaleString()} words` : `${s.words.toLocaleString()} ${s.words === 1 ? 'word' : 'words'}`;
  fill($('status'), 
    h('span', {}, `Page ${v.caret_page + 1} of ${v.pages.length}`),
    h('span', { class: 'status-text' }, words),
    s.carets > 1 ? h('button', { class: 'status-btn', title: 'Back to one caret (Esc)', onClick: () => run('key', { key: 'Escape', mods: {} }) }, `${s.carets} carets · Esc for one`) : null,
    app.prefs?.proofing?.enabled ? h('button', { class: 'status-btn issues', title: 'Check spelling and grammar (F7)', onClick: () => openCheck() },
      h('span', { html: icon(n ? 'spell' : 'check', 15) }), n ? `${n} to look at` : 'No mistakes found') : null,
    h('span', { class: 'spacer' }),
    app.updateReady ? h('button', { class: 'status-btn update', title: 'A new version of Write has been downloaded', onClick: takeUpdate }, 'Update ready: restart Write') : null,
    h('span', {}, saveState(s)),
    h('span', { class: 'zoom' },
      h('button', { 'aria-label': 'Zoom out', html: icon('minus', 16), onClick: () => zoomBy(-1) }),
      h('button', { class: 'pct status-btn', title: 'Actual size (Ctrl+0)', onClick: () => setZoom(100) }, `${pages.zoomPercent}%`),
      h('button', { 'aria-label': 'Zoom in', html: icon('plus', 16), onClick: () => zoomBy(1) })),
  );
}

const STEPS = [25, 33, 50, 67, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400];
function zoomBy(dir) {
  const z = pages.zoomPercent;
  const next = dir > 0 ? STEPS.find((s) => s > z) : [...STEPS].reverse().find((s) => s < z);
  if (next) setZoom(next);
}
function setZoom(p) { pages.setZoom(p); renderStatus(); }

// ----- Reading view: the text like a web page, no toolbars or page edges -----

async function toggleReading() {
  app.reading = !app.reading;
  document.documentElement.classList.toggle('reading', app.reading);
  if (app.reading && !$('exit-reading')) {
    const b = h('button', { id: 'exit-reading', class: 'btn small', onClick: () => toggleReading() }, 'Exit reading view');
    document.body.append(b);
  } else if (!app.reading) $('exit-reading')?.remove();
  await run('reading', { on: app.reading });
  pages.drawn.clear();
  pages.layoutPages();
}

// ----- Navigation pane -----

function toggleNav() {
  app.navOpen = !app.navOpen;
  $('nav').hidden = !app.navOpen;
  if (app.navOpen) renderNav();
  pages.layoutPages();
}
let navTimer = null;
function renderNavSoon() { if (!app.navOpen) return; clearTimeout(navTimer); navTimer = setTimeout(renderNav, 300); }
async function renderNav() {
  const o = await call('outline');
  const nav = $('nav');
  fill(nav, 
    h('h2', {}, 'Headings', h('button', { class: 'icon-btn small', 'aria-label': 'Close navigation', html: icon('x', 16), onClick: () => toggleNav() })),
    ...(o.headings.length ? o.headings.map((x, i) => {
      const b = h('button', { class: `nav-item l${x.level}${i === o.current ? ' on' : ''}`, style: `padding-left: ${8 + (x.level - 1) * 14}px`,
        'aria-current': i === o.current ? 'location' : null,
        onClick: async () => { await run('go_to', { para: x.para }); pages.revealCaret(); } },
      h('span', { class: 't' }, x.text), x.page ? h('span', { class: 'pg' }, x.page) : null);
      b.addEventListener('mousedown', (e) => e.preventDefault());
      return b;
    }) : [h('p', { class: 'nav-empty' }, 'No headings yet. Use the Heading styles and they’ll show here.')]));
}

function refresh() {
  renderNavSoon();
  refreshReviewSoon();
  refreshContentsSoon();
  refreshTablesSoon();
  placeGuides();
  if (app.view) checkSlash(app.view);
  if (app.tabs?.front !== app.templateFor) { app.templateFor = app.tabs?.front; checkTemplate(); }
  renderTabs();
  refreshToolbar();
  renderStatus();
}

// ----- Shortcuts that aren't the page's -----

// Ctrl and the mouse wheel zoom the pages.
onScrollers('wheel', (e) => {
  if (!(e.ctrlKey || e.metaKey)) return;
  e.preventDefault();
  zoomBy(e.deltaY < 0 ? 1 : -1);
}, { passive: false });

window.addEventListener('keydown', (e) => {
  // A dialog or full screen is open: its own keys only.
  if (document.querySelector('.modal-back, .screen')) return;
  if (e.key === 'F7' && e.shiftKey) { e.preventDefault(); lookUp(null); return; }
  if (e.key === 'F7') { e.preventDefault(); openCheck(); return; }
  if ((e.ctrlKey || e.metaKey) && e.altKey && e.key.toLowerCase() === 'r') { e.preventDefault(); toggleReading(); return; }
  if (e.key === 'Escape' && app.reading && !openPopup) { e.preventDefault(); toggleReading(); return; }
  if (e.key === 'Escape' && check.open && !openPopup && !document.querySelector('.screen, .modal-back')) { e.preventDefault(); closeCheck(); return; }
  // Ctrl+Alt+0 body text, Ctrl+Alt+1–3 headings.
  if ((e.ctrlKey || e.metaKey) && e.altKey && /^Digit[0-6]$/.test(e.code)) {
    e.preventDefault();
    const n = Number(e.code.slice(5));
    run('style', { id: n ? `heading${n}` : 'normal' });
    return;
  }
  if (e.key === '=' && e.altKey && !e.ctrlKey) { e.preventDefault(); equationEditor(null); return; }
  if (IN_BROWSER && e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.code === 'KeyW') { e.preventDefault(); closeTab(app.tabs.front); return; }
  if (IN_BROWSER && e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.code === 'KeyQ' && servedHere) { e.preventDefault(); quit(); return; }
  if (e.key.toLowerCase() === 'f' && e.altKey && e.ctrlKey) { e.preventDefault(); run('insert', { what: 'footnote' }); return; }
  if (e.key === 'Escape' && find.open && !openPopup && !document.querySelector('.screen, .modal-back')) { e.preventDefault(); closeFind(); pages.focus(); return; }
  const command = e.ctrlKey || e.metaKey;
  if (!command) return;
  const k = e.key.toLowerCase();
  const act = {
    n: () => (e.shiftKey ? toggleNav() : showNew()),
    w: () => closeTab(app.tabs.front),
    '=': () => zoomBy(1), '+': () => zoomBy(1), '-': () => zoomBy(-1), '0': () => setZoom(100),
    ' ': () => run('clear_format'),
    ',': () => showSettings(),
    e: () => { if (e.shiftKey) toggleTracking(); },
    m: () => { if (e.altKey) newComment(); },
    o: () => openFile(),
    f: () => openFind(false),
    h: () => openFind(true),
    k: () => linkEditor(),
    '/': () => shortcutsDialog(),
    '.': () => symbolsDialog(),
    s: () => (e.shiftKey ? saveAs() : save()),
    q: () => { if (!WEB || servedHere) quit(); },
    p: () => (WEB ? exportAs('pdf') : printDialog()),
  }[k];
  if (k === 'tab' && app.tabs) {
    const i = app.tabs.tabs.findIndex((t) => t.id === app.tabs.front);
    const n = app.tabs.tabs.length;
    const next = app.tabs.tabs[(i + (e.shiftKey ? n - 1 : 1)) % n];
    e.preventDefault();
    run('switch', { id: next.id }).then(() => pages.drawn.clear());
    return;
  }
  if (act) { e.preventDefault(); act(); }
});

// Back to the window: pictures shown from files are read again (they may have been edited).
window.addEventListener('focus', () => { forgetLinkedPictures(); pages.repaintAll(); refreshCalcFiles(); });

$('new-tab').innerHTML = icon('plus', 18);
$('new-tab').addEventListener('click', () => showNew());

// ----- Settings -----

function applyPrefs(p) {
  const proofChanged = app.prefs && JSON.stringify(app.prefs.proofing) !== JSON.stringify(p.proofing);
  app.prefs = p;
  if (proofChanged) proofSoon(0);
  const root = document.documentElement;
  if (p.theme === 'system') delete root.dataset.theme; else root.dataset.theme = p.theme;
  if (p.high_contrast) root.dataset.contrast = 'high'; else delete root.dataset.contrast;
  if (p.reduce_motion) root.dataset.motion = 'reduce'; else delete root.dataset.motion;
  $('toolbar').hidden = !p.toolbar;
  $('ruler').hidden = !p.ruler;
  $('status').hidden = !p.status_bar;
  // Menus and panels grow; the pages keep their own zoom.
  for (const el of [document.querySelector('.menubar'), document.querySelector('.tabstrip'), $('toolbar'), $('checkbar'), $('findbar'), $('status'), $('popups')]) {
    el.style.zoom = p.ui_size === 100 ? '' : `${p.ui_size}%`;
  }
  pages.ruler.units = p.units;
  pages.ruler.key = '';
  if (app.view) pages.layoutPages();
}

async function setPref(key, value) {
  applyPrefs(await call('set_prefs', { prefs: { ...app.prefs, [key]: value } }));
}

function showSettings(section) {
  closePopup();
  if (document.querySelector('.settings')) return;
  return openSettings({ prefs: app.prefs, section, onChange: applyPrefs, onClose: () => { pages.focus(); proofSoon(0); } });
}

/// A new blank document.
async function showNew() {
  closePopup();
  await run('new');
  pages.drawn.clear();
  pages.draw();
}

/// A document made from a template follows it: when it's shown, the
/// template as it is now comes in (its parts, styles and page setup; what's
/// typed in its fields stays). A template opened from a server makes a new
/// document from it, unless it's being edited.
async function checkTemplate() {
  const front = app.tabs?.front;
  const info = await call('template_info').catch(() => null);
  if (!info || app.tabs?.front !== front) return;
  const node = server.nodeOf(front);
  if (info.is_template && node != null && app.editingTemplate !== node) { await newFromServerTemplate(node, front); return; }
  if (!info.name) return;
  let args = {};
  const at = /^server:(.*)\|(\d+)$/.exec(info.from || '');
  if (at) {
    if (!server.ready || server.url !== at[1]) return;
    try { args = { data: (await server.copyOf(Number(at[2]))).data }; } catch { return; }
  }
  const out = await run('template_refresh', args);
  if (at) await templatePictures(Number(at[2]));
  if (out?.changed) { pages.drawn.clear(); toast(`Updated from the template “${info.name}”`); }
}

/// A document that took in a server template (`node`) names its pictures
/// (a logo, say), but a server copy carries no pictures: they're fetched
/// from the template.
async function templatePictures(node) {
  const missing = await call('template_pictures_missing').catch(() => []);
  if (!missing?.length || !server.ready) return;
  const b64 = (data) => {
    let s = '';
    for (let i = 0; i < data.length; i += 0x8000) s += String.fromCharCode(...data.subarray(i, i + 0x8000));
    return btoa(s);
  };
  for (const name of missing) {
    const path = `/v1/nodes/${node}/pictures/${name.replace(/^media\//, '').split('/').map(encodeURIComponent).join('/')}`;
    const data = await server.api('GET', path, undefined, { raw: true }).catch(() => null);
    if (data) await call('template_picture', { name, data: b64(data) }).catch(() => {});
  }
  pages.drawn.clear(); pages.draw();
  syncPictures();
}

/// Server document `node` (open in tab `tab`) is a template: a new document
/// from it takes the tab's place.
async function newFromServerTemplate(node, tab) {
  const name = String(app.tabs?.tabs.find((t) => t.id === tab)?.title || 'Template').replace(/\.[a-z]+$/i, '');
  try {
    const copy = await server.copyOf(node);
    await run('open_bytes', { name: `${name}.xwrite`, data: copy.data, template: true, from: `server:${server.url}|${node}`, template_name: name });
    await templatePictures(node);
    await closeTab(tab);
    pages.drawn.clear(); pages.draw();
  } catch (e) { toast(String(e.message || e)); }
}

/// Says a new document was made from template `name` (at `from`), with a way to edit the template instead.
function madeFromTemplate({ name, from }) {
  $('tpl-note')?.remove();
  const made = app.tabs?.front;
  const edit = async () => {
    note.remove();
    const doc = app.tabs?.tabs.find((t) => t.id === made);
    if (doc && !doc.dirty) await closeTab(made);
    const at = /^server:(.*)\|(\d+)$/.exec(from || '');
    if (at) { app.editingTemplate = Number(at[2]); await server.open(Number(at[2])); await refresh(); }
    else if (from?.startsWith('file:')) await run('template_edit', { path: from.slice(5) });
    else await run('template_edit', { name: from?.startsWith('mine:') ? from.slice(5) : name });
    pages.drawn.clear(); pages.draw();
    toast(`Editing the template “${name}”: documents made from it take your changes when they’re next opened`);
  };
  const editable = from?.startsWith('file:') || from?.startsWith('mine:') || /^server:/.test(from || '');
  const note = h('div', { id: 'tpl-note', class: 'tpl-note', role: 'status' },
    h('span', {}, `A new document from the template “${name}”.`),
    editable ? h('button', { class: 'btn small', onClick: edit }, 'Edit the template instead') : null,
    h('button', { class: 'btn small main', onClick: () => note.remove() }, 'OK'));
  document.querySelector('.doc-area').append(note);
  setTimeout(() => note.remove(), 12000);
}

/// File › Detach from template: the document becomes its own (like a copy downloaded).
async function detachTemplate() {
  const name = app.details?.template;
  modalShell('Detach from template', [h('p', {}, `The document becomes your own: everything in it can be changed, and changes to the template “${name}” no longer come in. Fields keep what’s typed in them.`)], [
    ['Cancel', () => true],
    ['Detach', async () => { await run('template_unlink'); toast('Detached: the document is your own'); return true; }, true],
  ]);
}

/// "This is a template": what to start it with. Nothing is ticked at first
/// (the document as it is becomes the template).
function templateStarter() {
  // A little page for each part, drawn in CSS (no words: the card says what it is).
  const PARTS = [
    ['title', 'Title page', 'A page of its own with the title, subtitle, author and date to fill in', 'title'],
    ['contents', 'Contents', 'A contents page, filled in from the headings, with page numbers', 'contents'],
    ['numbered', 'Numbered headings', 'Headings count themselves: 1, 1.1, 1.1.1', 'numbered'],
    ['headers', 'Header and footer', 'The title at the top of each page, “Page 2 of 5” at the bottom', 'headers'],
  ];
  const ticked = new Set();
  const main = h('button', { class: 'btn main' }, 'Make it a template');
  const note = h('p', { class: 'hint-text' });
  const say = () => {
    main.textContent = ticked.size ? `Add ${ticked.size === 1 ? 'it' : `these ${ticked.size}`} and make it a template` : 'Make it a template';
    note.textContent = ticked.has('title') || ticked.has('contents')
      ? 'Each document made from it has its own text after these pages; the rest stays the template’s.'
      : 'Nothing ticked: the document as it is becomes the template.';
  };
  const cards = h('div', { class: 'tpl-starter' }, PARTS.map(([id, name, about, pic]) => {
    const box = h('input', { type: 'checkbox', 'aria-label': name });
    const card = h('label', { class: 'tpl-part' }, box,
      h('span', { class: `tpl-pic ${pic}`, 'aria-hidden': 'true' }, h('i'), h('i'), h('i'), h('i')),
      h('span', { class: 'tpl-words' }, h('b', {}, name), h('span', {}, about)));
    box.addEventListener('change', () => { box.checked ? ticked.add(id) : ticked.delete(id); card.classList.toggle('on', box.checked); say(); });
    return card;
  }));
  say();
  const shell = modalShell('Make it a template', [
    h('p', {}, 'Opening the template makes a new document from it. Start it with:'), cards, note,
  ], [['Cancel', null]]);
  main.addEventListener('click', async () => {
    shell.close();
    const order = PARTS.map(([id]) => id).filter((id) => ticked.has(id));
    const out = await run('template_starter', { parts: order });
    if (!out) return;
    pages.drawn.clear(); pages.draw();
    toast(order.length ? 'It’s a template: words in {{double braces}} are filled in in each document made from it. Save it when it’s ready' : 'It’s a template: save it, and opening it makes a new document');
  });
  shell.body.querySelector('.modal-actions').append(main);
}

/// Insert › Template field: `{{label}}`, a box to fill in in each document made from the template.
function templateField() {
  const label = h('input', { class: 'field', 'aria-label': 'What goes in it', placeholder: 'e.g. Client, Date, Quote number' });
  const go = async () => {
    const l = label.value.trim().replace(/[{}]/g, '');
    if (!l) return false;
    if (!app.details?.is_template) await run('template_set', { on: true });
    await run('text', { text: `{{${l}}}` });
    return true;
  };
  modalShell('Template field', [h('p', {}, 'Each document made from this template shows it in grey, to type over. It isn’t printed until something is typed in it.'), label], [['Cancel', () => true], ['Insert', go, true]]);
  label.addEventListener('keydown', async (e) => { if (e.key === 'Enter') { e.preventDefault(); label.closest('.modal').querySelector('.btn.main').click(); } });
  label.focus();
}

/// Insert › Where the text goes: the paragraph the cursor is in is where each
/// document's text goes (shown faded here until then); the rest is the template's.
async function markBody() {
  const out = await run('template_mark_body');
  if (out) toast('Documents made from this template have their text here. Save it, and open it to make one');
}

/// File › New from template: the person's own templates (File › Save as template).
async function templatePicker() {
  const mine = (await call('templates')).filter((t) => t.mine);
  // The server's shared templates, when signed in to one.
  const shared = server.ready ? await server.templates().catch(() => []) : [];
  if (!mine.length && !shared.length) { toast('No templates yet. Make one with File › Save as template.'); return; }
  const list = h('div', { class: 'recover-list' });
  let shell = null;
  const fromServer = shared.map((n) => h('div', { class: 'recover-item' },
    h('div', {}, h('div', { class: 'set-name' }, n.details?.name || 'Untitled'), h('div', { class: 'set-desc' }, `On ${server.name || 'your server'}`)),
    h('div', { class: 'acts' }, h('button', { class: 'btn small main', onClick: async () => {
      shell.close();
      try {
        const copy = await server.copyOf(n.id);
        await run('open_bytes', { name: `${n.details?.name || 'Untitled'}.xwrite`, data: copy.data, template: true, from: `server:${server.url}|${n.id}`, template_name: n.details?.name || 'Untitled' });
        await templatePictures(n.id);
        pages.drawn.clear(); pages.draw();
      } catch (e) { toast(String(e.message || e)); }
    } }, 'Use it'))));
  const draw = () => fill(list, ...fromServer, mine.map((t, k) => h('div', { class: 'recover-item' },
    h('div', { class: 'set-name' }, t.name),
    h('div', { class: 'acts' },
      h('button', { class: 'btn small', onClick: async () => { await call('delete_template', { id: t.id }); mine.splice(k, 1); if (mine.length) draw(); else shell.close(); } }, 'Remove'),
      h('button', { class: 'btn small', onClick: async () => { shell.close(); await run('template_edit', { name: t.name }); pages.drawn.clear(); toast(`Editing the template “${t.name}”. Save it to update it; documents made from it can then take its changes.`); } }, 'Edit'),
      h('button', { class: 'btn small main', onClick: async () => { shell.close(); await run('new', { template: t.id }); pages.drawn.clear(); pages.draw(); } }, 'Use it')))));
  draw();
  shell = modalShell('New from template', [h('p', {}, shared.length ? 'Your server’s templates, and your own made with File › Save as template.' : 'Your own templates, made with File › Save as template.'), list], [['Cancel', null]]);
}

/// File › Use a template for this document: the document keeps its text and
/// takes the template's look (styles, page setup, headers and footers, and
/// any parts around the text). Change the template later and the document
/// follows the next time it's opened.
async function useTemplate() {
  const mine = WEB ? [] : (await call('templates')).filter((t) => t.mine);
  const shared = server.ready ? await server.templates().catch(() => []) : [];
  if (!mine.length && !shared.length) { toast(WEB ? 'No templates on the server yet: put one in its Templates space.' : 'No templates yet. Open a document you like the look of and use File › Save as template.'); return; }
  let shell = null;
  const done = (name) => {
    pages.drawn.clear(); pages.draw();
    toast(`This document now uses the template “${name}”. Change the template and it follows.`);
  };
  const row = (name, where, use) => h('div', { class: 'recover-item' },
    h('div', {}, h('div', { class: 'set-name' }, name), where ? h('div', { class: 'set-desc' }, where) : null),
    h('div', { class: 'acts' }, h('button', { class: 'btn small main', onClick: async () => {
      shell.close();
      try { await use(); done(name); } catch (e) { toast(String(e.message || e)); }
    } }, 'Use it')));
  const list = h('div', { class: 'recover-list' },
    ...shared.map((n) => {
      const name = n.details?.name || 'Untitled';
      return row(name, `On ${server.name || 'your server'}`, async () => {
        const copy = await server.copyOf(n.id);
        await run('template_use', { data: copy.data, name, from: `server:${server.url}|${n.id}` });
        await templatePictures(n.id);
      });
    }),
    ...mine.map((t) => row(t.name, 'Your own', () => run('template_use', { name: t.name }))));
  shell = modalShell('Use a template for this document', [
    h('p', {}, 'Your text stays as it is. It takes the template’s fonts and styles, headers and footers (a logo, say) and page setup, and follows the template from then on: when the template changes, so does this document.'),
    list], [['Cancel', null]]);
}

/// Work left by a run that closed without saving: offer it back.
async function offerRecovery() {
  const r = await call('recovery');
  const items = [...r.left.map((x) => ({ ...x, kind: 'left' })), ...r.orphans.map((x) => ({ ...x, kind: 'orphan' }))];
  if (!items.length) return false;
  return new Promise((resolve) => {
    const back = h('div', { class: 'modal-back' });
    let any = false;
    // Esc closes it (it's offered again next time), unless a question is open on top; Enter is Recover all.
    const unkey = dialogKeys(back, () => done(), () => back.querySelector(':scope > .modal > .modal-actions .btn.main') || (items.length === 1 ? list.querySelector('.btn.main') : null));
    const done = () => { back.remove(); unkey(); pages.focus(); resolve(any); };
    const list = h('div', { class: 'recover-list' });
    const forget = (x) => call('forget_recovery', x.kind === 'left' ? { id: x.id } : { key: x.key });
    const recover = async (x) => { any = true; await run('recover', x.kind === 'left' ? { id: x.id } : { key: x.key }); pages.drawn.clear(); };
    const all = async (fn) => { for (const x of items.splice(0)) await fn(x); done(); };
    const draw = () => {
      fill(list, ...items.map((x, k) => h('div', { class: 'recover-item' },
        h('div', {}, h('div', { class: 'set-name' }, x.title || x.start || 'Untitled'),
          x.title && x.start ? h('div', { class: 'set-desc recover-start' }, x.start) : null,
          h('div', { class: 'set-desc' }, x.kind === 'left' ? `Unsaved changes from ${x.when}, newer than the file` : `Never saved · ${x.when}${x.words ? ` · ${x.words.toLocaleString()} words` : ''}`)),
        h('div', { class: 'acts' },
          h('button', { class: 'btn small', onClick: async () => { await forget(x); items.splice(k, 1); items.length ? draw() : done(); } }, 'Discard'),
          h('button', { class: 'btn small main', onClick: async () => { await recover(x); items.splice(k, 1); items.length ? draw() : done(); } }, 'Recover')))));
    };
    draw();
    const many = items.length > 1;
    back.append(h('div', { class: 'modal wide', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'rc-title' },
      h('h2', { id: 'rc-title' }, many ? 'Write closed before these were saved' : 'Write closed before this was saved'),
      h('p', {}, many ? 'Your work was kept. Recover what you want to keep, then save it. Anything you discard is gone for good.' : 'Your work was kept. Recover it to carry on, then save it.'),
      list,
      many ? h('div', { class: 'modal-actions', style: 'margin-top: 16px' },
        h('button', { class: 'btn', onClick: async () => {
          const c = await ask(`Discard all ${items.length}?`, 'They can’t be got back.', [['yes', 'Discard all', 'danger'], ['no', 'Cancel']]);
          if (c === 'yes') await all(forget);
        } }, 'Discard all'),
        h('button', { class: 'btn main', onClick: () => all(recover) }, 'Recover all')) : null));
    document.body.append(back);
    back.querySelector('.btn.main')?.focus();
  });
}

// ----- The browser build: things that arrive after the first page -----

if (window.XENIFY_WEB) {
  // The maths, serif and emoji fonts: pages are drawn again with them.
  // The serif (and maths) fonts arrived: they're offered too.
  window.addEventListener('xenify-fonts', async () => { pages.drawn.clear(); app.fonts = textFonts(await call('fonts')); await run('view'); });
  // The mistake spotter is ready: check again with it.
  window.addEventListener('xenify-recheck', () => proofSoon(0));
  // The spotter's findings came in after the rules'.
  window.addEventListener('xenify-issues', async () => {
    const out = await call('view', { have: pages.have() });
    if (out) pages.setMarks(out.marks);
    await refreshIssues();
  });
  window.addEventListener('xenify-update-ready', async () => {
    // On a Xenify server Write follows the server: a version found just
    // after opening, with nothing unsaved, takes over at once.
    const early = performance.now() < 20000;
    if (early && (await onServer()) && !(app.tabs?.tabs || []).some((t) => t.dirty)) return window.XENIFY_UPDATE();
    app.updateReady = true;
    renderStatus();
  });
}

/// Restarts into the downloaded version, asking about unsaved work first.
async function takeUpdate() {
  const dirty = app.tabs?.tabs.filter((t) => t.dirty) || [];
  if (dirty.length) {
    const c = await ask('Restart with unsaved changes?', 'Unsaved work is kept in this browser and offered back after the restart.', [['yes', 'Restart'], ['no', 'Cancel']]);
    if (c !== 'yes') return;
    await call('autosave').catch(() => {});
  }
  window.XENIFY_UPDATE();
}

// In the browser on a Xenify server: the address names the server document
// in front (#open=<server>,<file>), so reloading reopens it and the link
// can be passed on; a document of this computer leaves it plain.
let addressFrom = false;
function syncAddress(t) {
  if (!WEB || !addressFrom) return;
  const front = t.tabs.find((d) => d.id === t.front);
  const node = front?.remote != null ? server.nodeOf(front.id) : null;
  // This server's document: ?open=<file>; another server's: #open=<server>,<file>.
  const u = new URL(location.href);
  u.searchParams.delete('open');
  u.hash = '';
  if (node != null && server.url === location.origin) u.searchParams.set('open', node);
  else if (node != null && server.url) u.hash = `open=${encodeURIComponent(server.url)},${node}`;
  const want = u.pathname + u.search + u.hash;
  if (location.pathname + location.search + location.hash !== want) history.replaceState(null, '', want);
}

// In the browser on a Xenify server: File › Quit goes back to its Files page.
let servedHere = false;
// Whether a Xenify server serves this page (asked once).
let served = null;
const onServer = () => (served ??= /^https?:$/.test(location.protocol)
  ? fetch('/v1/info').then((r) => r.ok && /json/.test(r.headers.get('content-type') || ''), () => false)
  : Promise.resolve(false));
function showHome() {
  if (!/^https?:$/.test(location.protocol)) return;
  fetch('/v1/info').then((r) => {
    servedHere = r.ok && /json/.test(r.headers.get('content-type') || '');
  }).catch(() => {});
}

async function start() {
  setUpWindow();
  if (IN_BROWSER) document.getElementById('new-tab').title = 'New document';
  buildMenus();
  buildToolbar();
  let prefs;
  [app.styles, app.fonts, prefs] = await Promise.all([call('styles'), call('fonts').then(textFonts), call('prefs')]);
  applyPrefs(prefs);
  await run('view');
  document.getElementById('web-loading')?.remove();
  document.documentElement.classList.add('ready');
  showHome();
  pages.focus();
  // Unsaved work is kept every minute (checked every 15 seconds).
  setInterval(() => call('autosave').catch(() => {}), 15000);
  await offerRecovery();
  // Documents opened from the system while Write was starting (macOS).
  const openWaiting = async () => { for (const p of await window.__TAURI__.core.invoke('opened_files').catch(() => [])) await openPath(p); };
  if (!window.XENIFY_WEB) { window.__TAURI__?.event.listen('open-files', openWaiting); await openWaiting(); }
  // Screenshots and tests: ?settings=word opens Settings searching for it.
  const q = new URLSearchParams(location.search);
  if (q.has('settings')) showSettings()?.search(q.get('settings'));
  // Opened from a Xenify server (the browser version): use that server.
  // Editing a page where the wiki shows it: one column as wide as the page's text, no paper.
  if (EMBED) {
    pages.atStart = true;
    run('reading', { on: true }).then(() => { pages.drawn.clear(); pages.layoutPages(); });
  }
  server.connectHere().then((yes) => {
    if (yes) renderTabs();
    // From the server's Files page: #open=<server>,<file> in the browser,
    // xenify-write://open?server=…&node=… on the desktop.
    // ?open=<file> (this server's) or #open=<server>,<file>.
    const here = new URLSearchParams(location.search).get('open');
    const linked = /^\d+$/.test(here || '') ? [null, location.origin, here] : /^#open=([^,]+),(\d+)$/.exec(location.hash);
    addressFrom = !!yes;
    if (linked) {
      // Opened for this document: the blank one Write starts with goes.
      const blank = app.tabs?.tabs.find((d) => d.remote == null && !d.path && !d.dirty);
      openLinked(decodeURIComponent(linked[1]), +linked[2]).then(async (ok) => {
        if (ok && blank && app.tabs.tabs.length > 1 && app.tabs.tabs.some((d) => d.id === blank.id)) await closeTab(blank.id);
        renderTabs();
      });
    }
    if (!WEB) {
      const openLinks = async () => {
        for (const l of await window.__TAURI__.core.invoke('opened_links').catch(() => [])) {
          try {
            const u = new URL(l);
            const node = Number(u.searchParams.get('node'));
            if (u.searchParams.get('server') && node) await openLinked(u.searchParams.get('server'), node);
          } catch { toast('That link to a server document is broken.'); }
        }
      };
      // macOS also hands links to a Write that's already running.
      window.__TAURI__.event.listen('open-links', openLinks);
      openLinks();
    }
  });
  // XENIFY_WRITE_TEST=file.js (desktop tests): a script run now, with Write's insides.
  if (window.XENIFY_TEST) window.XENIFY_TEST({ app, server, run, call, openFromServer, saveToServer, showSettings, pages, refreshLinks, showPicture }).catch((e) => call('log', { text: `test: ${e.stack || e}` }));
}
start();
