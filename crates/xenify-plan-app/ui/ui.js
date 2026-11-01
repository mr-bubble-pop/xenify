// Small pieces the screens are built from: elements, icons, popup menus,
// dialogs and messages.

import { icon } from './icons.js';

export { icon };

/// An element: h('button', { class: 'x', onClick: f }, 'text', child…).
export function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v;
    else if (k === 'style' && typeof v === 'object') for (const [sk, sv] of Object.entries(v)) { if (sk.startsWith('--')) el.style.setProperty(sk, sv); else el.style[sk] = sv; }
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
}

// ----- Popups -----

const layer = () => document.getElementById('popups');
let openPopup = null;

export function closePopups() {
  if (openPopup) {
    const p = openPopup;
    openPopup = null;
    p.close();
  }
}

/// Shows `el` as a popup at (`x`, `y`) (kept on screen). `onClose` runs when it goes.
export function popup(el, x, y, onClose) {
  closePopups();
  const box = h('div', { class: 'popup', role: el.getAttribute?.('role') ? null : 'dialog' }, el);
  layer().append(box);
  const r = box.getBoundingClientRect();
  box.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
  box.style.top = `${Math.max(8, Math.min(y, innerHeight - r.height - 8))}px`;
  const away = (e) => { if (!box.contains(e.target)) closePopups(); };
  const esc = (e) => { if (e.key === 'Escape') { e.stopPropagation(); closePopups(); } };
  setTimeout(() => document.addEventListener('pointerdown', away, true));
  document.addEventListener('keydown', esc, true);
  const me = {
    box,
    close() {
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('keydown', esc, true);
      box.remove();
      onClose?.();
    },
  };
  openPopup = me;
  return me;
}

// In a browser these belong to the browser (it never hands them over), so
// they aren't offered there.
const BROWSER_KEYS = new Set(['Ctrl+N', 'Ctrl+T', 'Ctrl+W', 'Ctrl+Q', 'Ctrl+Shift+N', 'Ctrl+Shift+T', 'Ctrl+Shift+W', 'Ctrl+Tab', 'Ctrl+Shift+Tab']);
const IN_BROWSER = /^https?:$/.test(location.protocol) && location.hostname !== 'tauri.localhost';
// Closing and quitting are too common to go without: Alt there instead.
const IN_BROWSER_AS = { 'Ctrl+W': 'Alt+W', 'Ctrl+Q': 'Alt+Q' };
const keysHere = (keys) => (keys && IN_BROWSER && BROWSER_KEYS.has(keys) ? IN_BROWSER_AS[keys] || '' : keys);
/// Alt+W / Alt+Q in a browser tab (Ctrl+W and Ctrl+Q are the browser's there): 'close', 'quit' or null.
export const browserCloseKey = (e) => (IN_BROWSER && e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey ? { KeyW: 'close', KeyQ: 'quit' }[e.code] || null : null);

/// A menu entry: item('Save', { keys: 'Ctrl+S', run, enabled, checked, sub: () => [items] }).
export function item(label, o = {}) {
  return { label, ...o };
}

/// A list of items (null: a line between groups) as a menu at (`x`, `y`).
export function menu(items, x, y, onClose) {
  const list = h('div', { role: 'menu' });
  const buttons = [];
  for (const it of items) {
    // One line between groups, however many items in between are left out.
    if (it === null) { if (list.lastChild && list.lastChild.tagName !== 'HR') list.append(h('hr')); continue; }
    if (it === undefined || it === false) continue;
    if (it.head) { list.append(h('div', { class: 'head' }, it.head)); continue; }
    const b = h('button', {
      class: 'item', role: it.checked != null ? 'menuitemcheckbox' : 'menuitem', disabled: it.enabled === false,
      'aria-checked': it.checked != null ? String(!!it.checked) : null,
    }, h('span', {}, it.label));
    if (keysHere(it.keys)) b.append(h('span', { class: 'keys' }, keysHere(it.keys)));
    if (it.checked) b.append(h('span', { class: 'check', html: icon('check', 16) }));
    if (it.sub) b.append(h('span', { class: 'more', 'aria-hidden': 'true' }, '›'));
    b.addEventListener('click', (e) => {
      if (it.sub) {
        const r = b.getBoundingClientRect();
        const parent = openPopup;
        openPopup = null;
        menu(it.sub(), r.right + 4, r.top - 6, () => parent?.close());
        return;
      }
      closePopups();
      it.run?.(e);
    });
    buttons.push(b);
    list.append(b);
  }
  if (list.lastChild?.tagName === 'HR') list.lastChild.remove();
  list.addEventListener('keydown', (e) => {
    const k = buttons.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const live = buttons.filter((b) => !b.disabled);
      const at = live.indexOf(document.activeElement);
      const next = live[(at + (e.key === 'ArrowDown' ? 1 : -1) + live.length) % live.length];
      next?.focus();
    } else if (e.key === 'ArrowRight' && k >= 0 && items.filter((i) => i && !i.head)[k]?.sub) {
      buttons[k].click();
    }
  });
  const p = popup(list, x, y, onClose);
  buttons.find((b) => !b.disabled)?.focus({ preventScroll: true });
  return p;
}

// ----- Dialogs and messages -----

// Open dialogs (their dimmed pages), newest last: only the top one answers Enter and Escape.
const dialogStack = [];

/// Enter and Escape for the dialog `box` on its dimmed page `back`: Escape
/// runs `cancel`, Enter clicks its main button (or its only one). Not while a
/// popup is open, and Enter is left to what uses it itself: a text area, a
/// list, a button, a text being edited, Ctrl/Alt/⌘, an accent being typed;
/// a key a box in it took (preventDefault) is left alone. `mainButton`, for
/// a dialog laid out its own way, finds the button. Call the function it
/// returns when the dialog closes.
export function dialogKeys(back, box, cancel, mainButton = null) {
  const top = () => dialogStack[dialogStack.length - 1] === back;
  const wanted = (e) => (e.key === 'Escape' || (e.key === 'Enter' && !e.ctrlKey && !e.altKey && !e.metaKey)) && !e.isComposing;
  const act = (e) => {
    if (!back.isConnected || !top() || openPopup) return;
    if (e.key === 'Escape') { cancel(); return; }
    const btns = [...box.querySelectorAll(':scope > .modal-actions .btn, :scope > footer .btn')];
    const main = mainButton ? mainButton() : btns.find((b) => b.classList.contains('main')) || (btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) main.click();
  };
  const key = (e) => {
    if (!top() || !wanted(e) || openPopup) return;
    const t = e.target;
    if (!back.contains(t)) {
      // Focus left outside (on the page, or the button that opened it); a text box out there is its own.
      if (t.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      e.preventDefault(); e.stopPropagation(); act(e);
      return;
    }
    if (e.key === 'Enter' && t.closest('textarea, select, button, a[href], [contenteditable]:not([contenteditable="false"]), [aria-expanded="true"]')) return;
    // After the dialog's own boxes have had it (and may have taken it).
    setTimeout(() => { if (!e.defaultPrevented) act(e); });
  };
  // Not on to the shortcuts underneath.
  const stay = (e) => { if (wanted(e)) e.stopPropagation(); };
  dialogStack.push(back);
  document.addEventListener('keydown', key, true);
  back.addEventListener('keydown', stay);
  return () => {
    const i = dialogStack.indexOf(back);
    if (i >= 0) dialogStack.splice(i, 1);
    document.removeEventListener('keydown', key, true);
  };
}

/// A dialog with `title`, `body` and `actions` [[label, value, main?]]: the
/// value of the button pressed (null for Escape or clicking outside).
export function modal(title, body, actions) {
  return new Promise((resolve) => {
    const back = h('div', { class: 'modal-back' });
    const done = (v) => { back.remove(); unkey(); resolve(v); };
    const box = h('div', { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
      h('h2', {}, title), body,
      h('div', { class: 'modal-actions' }, actions.map(([label, value, main]) => h('button', { class: `btn${main ? ' main' : ''}`, onClick: () => done(typeof value === 'function' ? value() : value) }, label))));
    const unkey = dialogKeys(back, box, () => done(null));
    // Other keys typed in it don't reach the shortcuts underneath either.
    back.addEventListener('keydown', (e) => e.stopPropagation());
    back.addEventListener('pointerdown', (e) => { if (e.target === back) done(null); });
    back.append(box);
    layer().append(back);
    (box.querySelector('input, select, textarea') || box.querySelector('.btn.main'))?.focus();
  });
}

/// Asks for one line of text: it, or null.
export async function ask(title, label, value = '', okLabel = 'OK') {
  const input = h('input', { class: 'field', value, 'aria-label': label });
  const body = h('label', {}, label, input);
  setTimeout(() => input.select());
  const ok = await modal(title, body, [['Cancel', null], [okLabel, true, true]]);
  return ok ? input.value : null;
}

let toastTimer = null;
/// A short message at the bottom; `action` [label, run] adds a button (View, Undo).
export function toast(text, action = null) {
  document.querySelector('.toast')?.remove();
  const t = h('div', { class: 'toast', role: 'status' }, h('span', {}, text),
    action ? h('button', { class: 'toast-act', onClick: () => { t.remove(); action[1](); } }, action[0]) : null);
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.remove(), action ? 6500 : 3200);
}

/// The colours offered for text and fill, then "automatic".
export const SWATCHES = [
  '#1C1917', '#57534E', '#A8A29E', '#E7E5E4', '#FFFFFF', '#C2410C', '#B91C1C', '#B45309',
  '#15803D', '#0F766E', '#1D4ED8', '#6D28D9', '#FCEBDF', '#FEE2E2', '#FEF3C7', '#DCFCE7',
  '#CCFBF1', '#DBEAFE', '#EDE9FE', '#F5F5F4',
];

/// A swatch picker at (`x`, `y`): calls `pick` with a colour, or '' for automatic.
export function colourPicker(x, y, pick, autoLabel = 'Automatic') {
  const grid = h('div', { class: 'swatches' },
    SWATCHES.map((c) => h('button', { class: 'swatch', style: { background: c }, 'aria-label': c, title: c, onClick: () => { closePopups(); pick(c); } })),
    h('button', { class: 'swatch none', 'aria-label': autoLabel, title: autoLabel, onClick: () => { closePopups(); pick(''); } }));
  return popup(grid, x, y);
}

const dialogs = [];

/// A large dialog of the screens' own (Create, an issue): `el` on a dimmed
/// page. Escape and clicking outside close it, Enter is its footer's main
/// button (Create, Save); returns the close function.
export function sheet(el, onClose) {
  const back = h('div', { class: 'sheet-back' }, el);
  const close = () => {
    if (!back.isConnected) return;
    back.remove();
    dialogs.splice(dialogs.indexOf(close), 1);
    unkey();
    onClose?.();
  };
  // (Escape in a text box being changed belongs to the text box first.)
  const unkey = dialogKeys(back, el, close);
  back.addEventListener('pointerdown', (e) => { if (e.target === back) back.dataset.down = '1'; });
  back.addEventListener('pointerup', (e) => { if (e.target === back && back.dataset.down) close(); delete back.dataset.down; });
  dialogs.push(close);
  layer().append(back);
  return close;
}

export const sheetOpen = () => dialogs.length > 0;
