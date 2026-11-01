// Settings: General (new paper canvases, how the screen looks), Keys (record new keys for the tools and commands, put them back) and
// Server (a Xenify server: see serverui.js; not shown when the server itself
// serves Draw). AI comes from the server, so there's nothing to set for it.

import { serverRow, ui } from './serverui.js';

const DEFAULTS = {
  autosaveSecs: 3, recentMax: 6, maxImageSide: 4096, modelTriangles: 30000, uiScale: 100,
  highContrast: false, reduceMotion: false, autoKey: true, showStart: true, playSound: true, embedMedia: false, reopenLast: false,
  units: 'auto', printDpi: 300,
};

export function loadSettings() {
  let s = {};
  try { s = JSON.parse(localStorage.getItem('draw.settings') || '{}'); } catch { /* defaults */ }
  return { ...DEFAULTS, ...s };
}

export function saveSettings(s) {
  try { localStorage.setItem('draw.settings', JSON.stringify(s)); } catch { /* not kept */ }
}

/// Puts the look settings on the page (text size, contrast, motion).
export function applySettings(s) {
  const root = document.documentElement;
  root.style.fontSize = `${s.uiScale}%`;
  root.style.zoom = s.uiScale === 100 ? '' : String(s.uiScale / 100);
  root.dataset.contrast = s.highContrast ? 'high' : '';
  root.dataset.motion = s.reduceMotion ? 'reduce' : '';
}

/// The tool keys as set (tool id → key), the built-in ones filled in.
export function toolKeys(tools, shapes) {
  let custom = {};
  try { custom = JSON.parse(localStorage.getItem('draw.keys') || '{}'); } catch { /* none */ }
  const out = {};
  for (const t of [...tools, ...shapes.map((x) => ({ ...x, id: `shape:${x.id}` }))]) if (t.key || custom[t.id]) out[t.id] = (custom[t.id] ?? t.key ?? '').toUpperCase();
  return out;
}

// Keys the browser keeps for itself in a tab: a page never sees them.
const IN_BROWSER = /^https?:$/.test(location.protocol) && location.hostname !== 'tauri.localhost';
const BROWSER_KEYS = new Set(['Ctrl+N', 'Ctrl+Shift+N', 'Ctrl+T', 'Ctrl+Shift+T', 'Ctrl+W', 'Ctrl+Shift+W', 'Ctrl+Q', 'Ctrl+Shift+Q',
  'Ctrl+Tab', 'Ctrl+Shift+Tab', 'Ctrl+PageUp', 'Ctrl+PageDown', 'Alt+F4', 'F11']);
const browserKeeps = (c) => IN_BROWSER && BROWSER_KEYS.has(c);
// Keys Draw uses without a command of their own.
const BUILT_IN = { Space: 'Play or pause', Enter: 'Edit the selected text', Escape: 'Stop what you’re doing', 'Ctrl+C': 'Copy', 'Ctrl+X': 'Cut', 'Ctrl+V': 'Paste' };

export async function settingsDialog(app, h, { modal, tools, shapes, toast, tab: start }) {
  const s = loadSettings();
  // Served by the server: the app is already on it (signing in is on its Files page).
  const withServer = !ui.server?.servedHere;
  let tab = start === 'server' && !withServer ? 'general' : start === 'keys' || start === 'server' ? start : 'general';
  const body = h('div', { class: 'settings', style: 'display: flex; flex-direction: column; gap: 12px' });
  const keys = toolKeys(tools, shapes);
  const defaultsOf = () => Object.fromEntries((app.commands || []).map(([id, , d]) => [id, d]));
  let shortcuts = {};
  try { shortcuts = JSON.parse(localStorage.getItem('draw.shortcuts') || '{}'); } catch { /* none */ }
  let filter = '';
  // The row recording keys, what went wrong there, and whether "Restore" is being confirmed.
  let rec = null, err = null, restoring = false;
  const num = (label, key, min, max, step = 1) => h('label', {}, label, h('input', { class: 'field', type: 'number', min: String(min), max: String(max), step: String(step), value: String(s[key]),
    onChange: (e) => { s[key] = Math.max(min, Math.min(max, Number(e.target.value) || s[key])); }, onKeydown: (e) => e.stopPropagation() }));
  const check = (label, key, title) => h('label', { class: 'check-row', title }, h('input', { type: 'checkbox', checked: s[key], onChange: (e) => { s[key] = e.target.checked; } }), h('span', {}, label));
  const select = (label, key, aria, title, options, conv = String) => h('label', { title }, label,
    h('select', { class: 'field', 'aria-label': aria, onChange: (e) => { s[key] = conv(e.target.value); } }, options.map(([v, l]) => h('option', { value: v, selected: s[key] === v }, l))));

  // ---- Keys: every tool, shape and command, with the keys it has now and the ones it came with.
  const defs = defaultsOf();
  const builtTool = (id) => ([...tools, ...shapes.map((x) => ({ ...x, id: `shape:${x.id}` }))].find((t) => t.id === id)?.key || '').toUpperCase();
  const entries = [
    ...tools.filter((t) => t.id !== 'shape').map((t) => ({ uid: t.id, kind: 'tool', raster: t.id.startsWith('r-'), label: t.label.split(':')[0], group: t.id.startsWith('r-') ? 'Painting tools (on a raster layer)' : 'Tools' })),
    ...shapes.map((x) => ({ uid: `shape:${x.id}`, kind: 'tool', raster: false, label: x.label.split(':')[0], group: 'Shapes' })),
    ...(app.commands || []).map(([id, label]) => ({ uid: id, kind: 'cmd', label, group: 'Commands' })),
  ];
  const visible = (list) => list.filter((k) => !browserKeeps(k));
  const current = (e) => (e.kind === 'tool' ? (keys[e.uid] ? [keys[e.uid]] : []) : visible(app.keysOf(shortcuts[e.uid] ?? defs[e.uid])));
  const builtIn = (e) => (e.kind === 'tool' ? (builtTool(e.uid) ? [builtTool(e.uid)] : []) : visible(app.keysOf(defs[e.uid])));
  const setKeys = (e, list) => { if (e.kind === 'tool') keys[e.uid] = list[0] || ''; else shortcuts[e.uid] = list; };
  const changed = (e) => current(e).join('\n') !== builtIn(e).join('\n');
  // Who else answers `combo`: a single letter is shared by a vector and a raster tool (each works on its own kind of layer).
  const userOf = (combo, me) => entries.find((o) => o !== me && current(o).includes(combo) && !(me.kind === 'tool' && o.kind === 'tool' && me.raster !== o.raster));
  const nice = (k) => k.replace(/^Arrow/, '').replace(/\+Arrow/g, '+');

  const record = (e, ev) => {
    ev.preventDefault(); ev.stopPropagation();
    if (ev.key === 'Escape') { rec = null; err = null; draw(); return; }
    const c = app.comboOf(ev);
    if (!c) return; // Ctrl, Shift… alone: wait for the rest.
    const fail = (text, clash = null) => { err = { uid: e.uid, text, combo: c, clash }; draw(); };
    if (e.kind === 'tool' && !/^[A-Z0-9]$/.test(c)) return fail('A tool takes one letter or number, without Ctrl, Alt or Shift.');
    if (browserKeeps(c)) return fail(`The browser keeps ${c} for itself, so Draw never gets it here. Pick other keys.`);
    if (BUILT_IN[c]) return fail(`${c} is already used to ${BUILT_IN[c].toLowerCase()}. Pick other keys.`);
    const other = userOf(c, e);
    if (other) return fail(`${c} is already used for “${other.label}”.`, other);
    setKeys(e, [c]);
    rec = null; err = null;
    draw();
  };

  const keyRow = (e) => {
    const now = current(e);
    const recording = rec === e.uid;
    const bad = err?.uid === e.uid ? err : null;
    const keysCell = recording
      ? h('input', { class: 'field key-capture', readonly: true, placeholder: 'Press the new keys…', 'aria-label': `Press the new keys for ${e.label}`, onKeydown: (ev) => record(e, ev),
        onBlur: (ev) => { if (!ev.relatedTarget?.closest?.('.key-row')) setTimeout(() => { if (rec === e.uid && !body.contains(document.activeElement)) { rec = null; err = null; draw(); } }, 0); } })
      : h('span', { class: 'key-chips' }, now.length ? now.map((k) => h('kbd', { class: 'key-cap' }, nice(k))) : h('span', { class: 'muted' }, 'No key'));
    const revertTo = builtIn(e).map(nice).join(' or ') || 'no key';
    return h('div', { class: `key-row${recording ? ' recording' : ''}${changed(e) ? ' changed' : ''}` },
      h('span', { class: 'key-name' }, e.label),
      keysCell,
      h('span', { class: 'key-acts' },
        recording
          ? [now.length ? h('button', { class: 'small-btn', onClick: () => { setKeys(e, []); rec = null; err = null; draw(); } }, 'No key') : null,
            h('button', { class: 'small-btn', onClick: () => { rec = null; err = null; draw(); } }, 'Cancel')]
          : [h('button', { class: 'small-btn', 'aria-label': `Change the keys for ${e.label}`, onClick: () => { rec = e.uid; err = null; restoring = false; draw(); } }, 'Change'),
            changed(e) ? h('button', { class: 'small-btn', 'aria-label': `Put back the keys ${e.label} came with`, title: `Back to ${revertTo}`, onClick: () => { setKeys(e, builtIn(e)); if (e.kind === 'cmd') delete shortcuts[e.uid]; err = null; draw(); } }, 'Revert') : null]),
      bad ? h('p', { class: 'key-error', role: 'alert' }, bad.text,
        bad.clash ? h('button', { class: 'link', onClick: () => { setKeys(bad.clash, current(bad.clash).filter((k) => k !== bad.combo)); setKeys(e, [bad.combo]); rec = null; err = null; draw(); } }, ` Use it here instead (“${bad.clash.label}” loses it)`) : null) : null);
  };

  const draw = () => {
    const pages = [['general', 'General'], ['keys', 'Keys'], withServer ? ['server', 'Server'] : null].filter(Boolean);
    const tabs = h('div', { class: 'segs', style: 'display: flex' }, pages.map(([k, t]) =>
      h('button', { class: `seg${tab === k ? ' on' : ''}`, style: 'flex: 1 1 0', onClick: () => { tab = k; rec = null; err = null; draw(); } }, t)));
    let page;
    if (tab === 'general') {
      const desktop = window.__TAURI__ && !window.XENIFY_WEB;
      page = h('div', { style: 'display: flex; flex-direction: column; gap: 14px' },
        h('div', { class: 'lab' }, 'New canvases'),
        h('div', { class: 'form-grid' },
          select('Measure new canvases in', 'units', 'Units for new canvases', 'Rulers, guides and typed sizes on new canvases (each canvas can change it in Canvas settings)',
            [['auto', 'Automatic: paper in mm or inches, the rest in pixels'], ['px', 'Pixels'], ['mm', 'Millimetres'], ['cm', 'Centimetres'], ['in', 'Inches'], ['pt', 'Points']]),
          select('Paper sizes made for', 'printDpi', 'Resolution of new paper canvases', 'How many pixels an inch of paper gets on new print canvases',
            [[300, 'Printing (300 pixels an inch)'], [150, 'Draft printing (150)'], [96, 'Reading on screen (96)']], Number)),
        h('div', { class: 'lab' }, 'Screen'),
        h('div', { class: 'form-grid' },
          num('Size of the menus and panels (%)', 'uiScale', 75, 200, 5), h('span'),
          check('Stronger colours, easier to see', 'highContrast', 'Darker text and clearer edges everywhere'),
          check('No sliding or fading panels', 'reduceMotion', 'Menus and panels appear at once instead of moving'),
          check('Play sound when the timeline plays', 'playSound'),
          desktop ? check('Show the New canvas screen at start', 'showStart') : null,
          desktop ? check('Open the canvases that were open last time', 'reopenLast') : null));
    } else if (tab === 'keys') {
      const q = filter.trim().toLowerCase();
      const hit = (e) => !q || e.label.toLowerCase().includes(q) || current(e).some((k) => k.toLowerCase().includes(q));
      const shown = entries.filter(hit);
      const groups = [...new Set(shown.map((e) => e.group))];
      const search = h('input', { class: 'field', type: 'search', placeholder: 'Find a tool, command or key', value: filter, 'aria-label': 'Find a shortcut',
        onInput: (e) => { filter = e.target.value; const at = e.target.selectionStart; draw(); const again = body.querySelector('input[type=search]'); again?.focus(); again?.setSelectionRange(at, at); }, onKeydown: (e) => e.stopPropagation() });
      const anyChanged = entries.some(changed);
      page = h('div', { style: 'display: flex; flex-direction: column; gap: 8px' },
        h('p', { class: 'muted' }, 'Press Change, then the keys you want (for example Ctrl+Shift+D). Nothing changes until you press Save.'),
        search,
        h('div', { class: 'key-list' },
          groups.map((g) => [h('div', { class: 'lab key-group' }, g), shown.filter((e) => e.group === g).map(keyRow)]),
          !shown.length ? h('p', { class: 'muted' }, 'Nothing matches.') : null),
        restoring
          ? h('div', { class: 'key-restore', role: 'alert' }, h('span', {}, 'Put every key back the way Draw came?'),
            h('button', { class: 'small-btn', onClick: () => {
              Object.assign(keys, Object.fromEntries(Object.keys(keys).map((k) => [k, builtTool(k)])));
              for (const k of Object.keys(shortcuts)) delete shortcuts[k];
              restoring = false; rec = null; err = null; draw();
            } }, 'Restore'), h('button', { class: 'small-btn', onClick: () => { restoring = false; draw(); } }, 'Cancel'))
          : h('button', { class: 'small-btn', style: 'align-self: flex-start', disabled: !anyChanged, title: anyChanged ? 'Every tool and command gets the keys it came with' : 'All the keys are as Draw came', onClick: () => { restoring = true; draw(); } }, 'Restore default keys'));
    } else {
      page = h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
        h('p', { class: 'muted' }, 'Optional. Connect to a Xenify server to open shared canvases, edit them together and use its AI. Without one, Draw keeps working on this computer as it does now.'),
        serverRow(draw));
    }
    // Redrawn after each change: the list stays where it was scrolled to.
    const scrolled = body.querySelector('.key-list')?.scrollTop || 0;
    body.replaceChildren(tabs, page);
    const list = body.querySelector('.key-list');
    if (list) list.scrollTop = scrolled;
    if (rec) body.querySelector('.key-capture')?.focus({ preventScroll: true });
  };
  draw();
  const ok = await modal('Settings', body, [['Cancel', false], ['Save', true, true]]);
  if (!ok) return;
  saveSettings(s);
  applySettings(s);
  const custom = {};
  for (const [k, v] of Object.entries(keys)) if (v !== builtTool(k)) custom[k] = v;
  try {
    localStorage.setItem('draw.keys', JSON.stringify(custom));
    localStorage.setItem('draw.shortcuts', JSON.stringify(Object.fromEntries(Object.entries(shortcuts).filter(([id, k]) => app.keysOf(k).join('\n') !== app.keysOf(defs[id]).join('\n')))));
    localStorage.removeItem('draw.ai');
  } catch { /* not kept */ }
  app.settings = s;
  app.onSettings?.();
  toast('Settings saved.');
}
