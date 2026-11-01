// Xenify Draw: the window around the canvas (menus, canvas tabs, toolbar,
// layers, the Layer and Writing panels, the timeline and the status bar).
// The same screens run in the desktop app and in the browser.

import { start, call, tabs as engineTabs, engineRaw, live, files, media } from './engine.js';
import { Server } from './server.js';
import { ui as serverUi, openFromServer, openLinked, saveToServer, shareDialog, historyDialog, checkpointDialog, offerUnsent, pickFile } from './serverui.js';
import { icon } from './icons.js';
import { Viewport, isTyping } from './viewport.js';
import { Timeline, propName, fmtTime, easingLabel } from './timeline.js';
import { Writing } from './writing.js';

import * as fonts from './fonts.js';
import { textSection } from './textpanel.js';
import { lookSections, labelFor, EFFECT_KINDS } from './look.js';
import { exportDialog, renderPng } from './export.js';
import { refreshAiServer, hasAi, aiServer } from './aiprovider.js';
import { Slides } from './slides.js';
import { Sound, importSound, clipBytes } from './sound.js';
import { probe as probeVideo } from './video.js';
import { emojiPicker, gifPicker, shapesPanel } from './pickers.js';
import { paintPanel } from './paint.js';
import { modelDialog } from './model3d.js';
import { settingsDialog, loadSettings, applySettings, toolKeys } from './settings.js';
import { Assistant } from './assistant.js';
import { takeOverColourFields } from './colorpicker.js';
import { aiNewDialog, aiSlideDialog, illustrationDialog, videoDialog } from './aimenu.js';
import { pensDialog } from './pens.js';
import { initRaster, RASTER_TOOLS, SHARED_TOOLS } from './raster.js';
import { aiEditDialog, aiSelectDialog, removeBackground } from './aiedit.js';
import { newCanvasScreen } from './newcanvas.js';
import * as platform from './platform.js';
import { setUpWindow } from './window.js';
import { keepOffline, applyUpdate } from './offline.js';
import { numberScrubbing } from './scrub.js';

const $ = (id) => document.getElementById(id);
/// Replaces an element's children, skipping empty ones (null, false) and flattening lists.
const put = (el, ...kids) => el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
const h = (tag, props = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'selected' || k === 'checked' || k === 'disabled') { if (v) el.setAttribute(k, ''); }
    else if (v === true) el.setAttribute(k, '');
    else if (v !== false && v != null) el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
};

const TOOLS = [
  { id: 'select', label: 'Select', key: 'V', icon: 'select' },
  { id: 'node', label: 'Edit points: drag points and handles, double-click to add a point or make one smooth', key: 'A', icon: 'node' },
  { id: 'xform', label: 'Free transform: scale and turn around a centre you can move (Alt: from the centre, Shift+corner: perspective)', key: 'Q', icon: 'xform' },
  { id: 'perspective', label: 'Perspective: drag the corners', key: 'W', icon: 'perspective' },
  { id: 'bones', label: 'Bones: drag a part to turn it about its joint (what hangs from it follows); Shift+drag reaches with the whole arm. Drag a joint dot to where it bends; drop it on another part to hang it from that part', key: 'J', icon: 'bones' },
  { id: 'shape', label: 'Shapes', icon: 'shapes' },
  { id: 'pen', label: 'Pen: exact lines and curves, point by point. Click for a corner, drag for a curve, keep clicking; Enter or double-click to finish, click the first point to close', key: 'P', icon: 'pen' },
  { id: 'pencil', label: 'Pencil: a freehand line, as you draw it (rough circles and boxes become clean ones; Alt: keep it as drawn)', key: 'N', icon: 'pencil' },
  { id: 'brush', label: 'Brush: freehand paint, a filled stroke in the fill colour that you can make wide', key: 'B', icon: 'brush' },
  { id: 'sculpt', label: 'Sculpt: push, pull or push away the outlines of the selected shapes (Alt: pull ↔ push away)', key: 'M', icon: 'sculpt' },
  { id: 'text', label: 'Text', key: 'T', icon: 'type' },
  { id: 'gradient', label: 'Gradient: drag across shapes to set where the colours start and end (Alt: the stroke)', key: 'G', icon: 'gradient' },
  { id: 'mask', label: 'Mask: drag on the selected layer to draw a mask; drag its points to reshape it', key: 'U', icon: 'mask' },
  { id: 'crop', label: 'Crop and slices: drag a rectangle, then crop the canvas or the selected layers, or make it a slice to export on its own', icon: 'crop' },
  { id: 'eyedropper', label: 'Eyedropper: click a layer to take its colours (onto the selection, if any)', key: 'I', icon: 'pipette' },
  { id: 'knife', label: 'Knife: drag across shapes to cut them in two', key: 'C', icon: 'knife' },
  { id: 'connector', label: 'Connector: drag from one shape to another to join them (the line finds its way round). Drop on a dot or on the edge to join at that spot; inside the shape, it picks the best side', key: 'F', icon: 'connector' },
  { id: 'zoom', label: 'Zoom: click to zoom in; right-click or Shift/Ctrl/Alt+click to zoom out; drag a box to zoom to it', key: 'Z', icon: 'zoom' },
  { id: 'hand', label: 'Hand (or hold Space)', key: 'H', icon: 'hand' },
];

/// Every tool with a key: the vector ones and the raster ones (raster.js; the same letters, on a raster layer).
const ALL_TOOLS = [...TOOLS, ...RASTER_TOOLS];

/// A diagram canvas's tools (Canvas settings › Kind: Diagram); the others still answer their keys.
const DIAGRAM_TOOLS = ['select', 'node', 'shape', 'connector', 'pen', 'text', 'eyedropper', 'gradient', 'zoom', 'hand'];

/// The shapes button's shapes, in its menu's order.
const SHAPES = [
  { id: 'rect', label: 'Rectangle', key: 'R', icon: 'rect' },
  { id: 'rounded', label: 'Rounded rectangle', icon: 'roundrect' },
  { id: 'ellipse', label: 'Ellipse', key: 'E', icon: 'ellipse' },
  { id: 'triangle', label: 'Triangle', icon: 'triangle' },
  { id: 'polygon', label: 'Polygon', key: 'Y', icon: 'polygon' },
  { id: 'star', label: 'Star', icon: 'star' },
  { id: 'gear', label: 'Gear', icon: 'gear' },
  { id: 'heart', label: 'Heart', icon: 'heart' },
  { id: 'arrow', label: 'Arrow', icon: 'arrow' },
  { id: 'bubble', label: 'Speech bubble', icon: 'bubble' },
  { id: 'line', label: 'Line', key: 'L', icon: 'line' },
];

const app = {
  state: null,
  rev: 0,
  layerInfo: null,
  tool: 'select',
  playing: false,
  loop: true,
  autoKey: true,
  fill: '#C2410C',
  stroke: '#1C1917',
  strokeWidth: 4,
  // New shapes: filled, and outlined when the stroke is on.
  fillOn: true,
  strokeOn: false,
  shape: 'rect',
  shapeOpts: { sides: 5, inner: 0.5, teeth: 12, depth: 0.2, hole: 0.3, radius: 24 },
  brushSize: 16,
  sculpt: { mode: 'push', size: 60, strength: 4 },
  maskShape: 'rect',
  recognize: true,
  smoothing: 3,
  side: 'layer',
  tabs: [],
  renderer: 'WebGL2 (Vello Hybrid)',
  lastTick: null,
  lastStructure: '',
  /// Which Layer panel sections are open (by key), as the person left them.
  openSections: {},
  full: null,
  /// Snapping, rulers, grid and guides (kept in this browser / on this computer).
  prefs: { snapObjects: true, snapGuides: true, snapGrid: false, snapMargin: 0, showGrid: false, showRulers: false, showGuides: true, showSlices: true },
};
try { Object.assign(app.prefs, JSON.parse(localStorage.getItem('draw.prefs') || '{}')); } catch { /* defaults */ }
/// The parts of the window you can hide (View › Panels): key, name.
const PANELS = [['layers', 'Layers'], ['side', 'Details (Layer, Paint, Writing, AI)'], ['timeline', 'Timeline'], ['toolbar', 'Toolbar'], ['status', 'Status bar'], ['slides', 'Slides list'], ['symbols', 'Symbols list'], ['shapes', 'Diagram shapes (UML, flowchart, network)']];
/// The Shapes panel: on for diagrams unless hidden; on other canvases when asked for.
const shapesShown = () => (app.state?.comp.kind === 'diagram' ? !app.prefs.hidden?.shapes : !!app.prefs.shapesOn);
function showShapes(on) {
  if (app.state?.comp.kind === 'diagram') app.prefs.hidden = { ...app.prefs.hidden, shapes: !on };
  else app.prefs.shapesOn = on;
  if (on && app.prefs.hidden?.layers) app.prefs.hidden = { ...app.prefs.hidden, layers: false };
  if (on && narrow.matches) document.body.classList.add('show-layers');
  app.savePrefs();
  applyLayout();
  renderLayers();
  renderToolbar();
}
/// Shows the panels that are on, at the widths they were dragged to.
function applyLayout() {
  const hid = app.prefs.hidden || {}, w = app.prefs.widths || {};
  for (const [key] of PANELS.slice(0, 5)) { const el = document.getElementById(key); if (el && key !== 'timeline') el.hidden = !!hid[key]; }
  const L = document.getElementById('layers'), S = document.getElementById('side');
  if (L) L.style.flexBasis = w.layers ? `${w.layers}px` : '';
  if (S) S.style.flexBasis = w.side ? `${w.side}px` : '';
  document.body.classList.toggle('no-slides-list', !!hid.slides);
  document.body.classList.toggle('no-symbols-list', !!hid.symbols);
  // The timeline: as tall as it was dragged to, or only its bar (minimised).
  const T = document.getElementById('timeline');
  if (T) {
    T.classList.toggle('min', !!app.prefs.tlMin);
    T.style.height = app.prefs.tlHeight && !app.prefs.tlMin ? `${app.prefs.tlHeight}px` : '';
    T.style.maxHeight = app.prefs.tlHeight && !app.prefs.tlMin ? '70vh' : '';
  }
  drawerButtons();
  if (app.state) { timelineShown(); renderLayers(); tl?.update(); }
  requestAnimationFrame(() => app.viewport?.fit());
}
app.applyLayout = applyLayout;

// ----- Tablets: the panels open beside the canvas when asked for -----

/// A narrow window (a tablet): the canvas gets the width, and Layers and
/// Details open from buttons at the end of the tabs.
const narrow = window.matchMedia('(max-width: 1100px)');
function drawerButtons() {
  const strip = document.querySelector('.tabstrip');
  if (!strip) return;
  let box = document.getElementById('drawers');
  if (!box) {
    const btn = (key, label, name) => h('button', { class: 'pick drawer-btn', 'data-drawer': key, 'aria-pressed': 'false', 'aria-label': label, title: `Show or hide ${label}`, onClick: () => openDrawer(key, !document.body.classList.contains(`show-${key}`)) },
      h('span', { html: icon(name, 16), style: 'display: inline-flex' }), h('span', {}, label));
    box = h('div', { id: 'drawers', class: 'drawers' }, btn('layers', 'Layers', 'folder'), btn('side', 'Details', 'sliders'));
    strip.append(box);
  }
  const hid = app.prefs.hidden || {};
  for (const b of box.children) {
    b.hidden = !!hid[b.dataset.drawer];
    b.setAttribute('aria-pressed', String(document.body.classList.contains(`show-${b.dataset.drawer}`)));
  }
}
/// Opens or closes a panel on a tablet (an upright one has room for one at a time).
function openDrawer(key, on = true) {
  if (on && window.innerWidth <= 900) document.body.classList.remove('show-layers', 'show-side');
  document.body.classList.toggle(`show-${key}`, on);
  drawerButtons();
  if (on && app.state) (key === 'side' ? app.renderSide() : renderLayers());
}
/// One of the Details tabs, shown (and opened on a tablet).
function showSide(which) {
  app.side = which;
  if (narrow.matches) openDrawer('side');
  app.renderSide();
}
narrow.addEventListener('change', () => drawerButtons());

/// The timeline: only its bar (and back).
app.minTimeline = (on = !app.prefs.tlMin) => { app.prefs.tlMin = on; app.savePrefs(); applyLayout(); };
/// The timeline: gone (View › Panels › Timeline brings it back).
app.hideTimeline = () => {
  app.prefs.hidden = { ...app.prefs.hidden, timeline: true };
  app.savePrefs();
  applyLayout();
  toast('Timeline hidden. View › Panels › Timeline brings it back.');
};
/// Dragging the timeline's top edge makes it taller or shorter.
function timelineResizer() {
  const el = $('timeline');
  const set = (px) => { app.prefs.tlHeight = Math.round(Math.max(96, Math.min(window.innerHeight * 0.7, px))); app.prefs.tlMin = false; applyLayout(); };
  const grip = h('div', { class: 'tl-grip', role: 'separator', 'aria-orientation': 'horizontal', 'aria-label': 'Timeline height', tabindex: '0', title: 'Drag to change its height (double-click: back to normal)',
    onDblclick: () => { delete app.prefs.tlHeight; app.savePrefs(); applyLayout(); },
    onKeydown: (e) => { if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return; e.preventDefault(); e.stopPropagation(); set(el.getBoundingClientRect().height + (e.key === 'ArrowUp' ? 16 : -16)); app.savePrefs(); },
    onPointerdown: (e) => {
      e.preventDefault();
      const y0 = e.clientY, h0 = el.getBoundingClientRect().height;
      const move = (ev) => set(h0 + y0 - ev.clientY);
      const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); app.savePrefs(); };
      window.addEventListener('pointermove', move);
      window.addEventListener('pointerup', up);
    } });
  el.before(grip);
}
/// Dragging a panel's inner edge makes it wider or narrower.
function panelResizers() {
  for (const [key, side] of [['layers', 'right'], ['side', 'left']]) {
    const el = document.getElementById(key);
    if (!el) continue;
    const grip = h('div', { class: `panel-grip ${side}`, role: 'separator', 'aria-orientation': 'vertical', 'aria-label': `${key === 'layers' ? 'Layers' : 'Details'} panel width`, tabindex: '0', title: 'Drag to change its width (double-click: back to normal)',
      onDblclick: () => { if (app.prefs.widths) delete app.prefs.widths[key]; app.savePrefs(); applyLayout(); },
      onKeydown: (e) => { if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return; e.preventDefault(); e.stopPropagation(); const now = el.getBoundingClientRect().width; const d = (e.key === 'ArrowRight') === (side === 'right') ? 16 : -16; app.prefs.widths = { ...app.prefs.widths, [key]: Math.max(180, Math.min(640, now + d)) }; app.savePrefs(); applyLayout(); },
      onPointerdown: (e) => {
        e.preventDefault();
        const x0 = e.clientX, w0 = el.getBoundingClientRect().width;
        const move = (ev) => { const d = (ev.clientX - x0) * (side === 'right' ? 1 : -1); app.prefs.widths = { ...app.prefs.widths, [key]: Math.round(Math.max(180, Math.min(640, w0 + d))) }; applyLayout(); };
        const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); app.savePrefs(); };
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', up);
      } });
    el.parentElement.insertBefore(grip, side === 'right' ? el.nextSibling : el);
  }
}
app.savePrefs = () => { try { localStorage.setItem('draw.prefs', JSON.stringify(app.prefs)); } catch { /* not kept */ } app.viewport?.redraw(); };
window.drawApp = app;

// ----- Engine calls -----

/// Runs an engine call and brings the screens up to date. `quiet` (while
/// dragging) refreshes only what moves.
app.run = (name, args = {}, { quiet = false } = {}) => {
  let out;
  try {
    out = call(name, args);
  } catch (e) {
    toast(String(e.message || e));
    return null;
  }
  if (quiet) app.light();
  else app.changed();
  return out;
};

app.light = () => {
  app.state = call('state');
  app.rev++;
  tl.update();
  app.viewport.redraw();
};

app.changed = () => {
  const before = app.state?.revision;
  app.state = call('state');
  app.rev++;
  const sel = app.state.selection;
  // Several selected: the first one's panel (values with a keyframe button change on all).
  app.layerInfo = sel.length >= 1 ? call('layer', { id: sel[0] }) : null;
  app.full = sel.length >= 1 ? call('layer_full', { id: sel[0] }) : null;
  renderLayers();
  renderA11yTree();
  app.renderSide();
  const structure = JSON.stringify([app.state.layers, sel, app.layerInfo && Object.entries(app.layerInfo.props).map(([k, v]) => [k, v.keys, v.animated]), app.state.comp]);
  if (structure !== app.lastStructure) { app.lastStructure = structure; tl.render(); } else tl.update();
  renderStatus();
  renderTabs();
  renderToolbar();
  timelineShown();
  app.viewport.redraw();
  // (Not before the server connection is set up, early in starting.)
  try { sayWhere(); } catch { /* not yet */ }
  if (before !== app.state.revision) {
    writing.soon();
    const tab = frontTab();
    if (tab) tab.dirty = app.state.dirty;
    autosave();
  }
};

app.select = (ids) => {
  call('select', { ids });
  app.changed();
};

app.layerById = (id) => app.state?.layers.find((l) => l.id === id);

/// The colours new shapes get.
app.paint = () => ({ fill: app.fillOn ? app.fill : null, stroke: app.strokeOn ? app.stroke : null, stroke_width: app.strokeOn ? app.strokeWidth : 0 });

/// Eyedropper: the colours under the pointer (gradients as they look there) become the current ones, and the
/// selection's. "Into the fill" or "the stroke" takes just the one colour showing there.
app.eyedrop = (p) => {
  const s = call('sample_color', { x: p[0], y: p[1], tolerance: 4 / app.viewport.k });
  const hex = (c) => (c ? String(c).slice(0, 7) : null);
  const into = app.eyeInto || 'both';
  const fill = into === 'stroke' ? null : hex(into === 'fill' ? s.visible : s.fill);
  const stroke = into === 'fill' ? null : hex(into === 'stroke' ? s.visible : s.stroke);
  if (!fill && !stroke) { app.hint('Eyedropper: click a shape or picture to take its colours.'); return; }
  if (fill) { app.fill = fill; app.fillOn = true; }
  if (stroke) { app.stroke = stroke; app.strokeOn = true; if (into === 'both' && s.stroke_width) app.strokeWidth = s.stroke_width; }
  if (into === 'both' && !s.stroke && s.fill) app.strokeOn = false;
  const hit = call('hit', { x: p[0], y: p[1], tolerance: 4 / app.viewport.k });
  for (const id of sel().filter((x) => x !== hit)) {
    const l = app.layerById(id);
    if (fill) app.run('set_prop', { id, prop: l?.kind === 'text' ? 'text_color' : 'fill', value: fill });
    if (stroke && l?.kind !== 'text') app.run('set_prop', { id, prop: 'stroke', value: stroke });
  }
  renderToolbar();
  toast(into === 'both' ? `Picked fill ${fill || 'none'} and stroke ${stroke || 'none'}.` : `Picked ${fill || stroke} into the ${into}.`);
};
app.toast = (t) => toast(t);
app.labelFor = (p) => labelFor(p, app.full?.layer);

app.setTool = (t, shape) => {
  if (shape) app.shape = shape;
  if (t === 'node' && sel().length !== 1) {
    const first = app.state?.layers.find((l) => sel().includes(l.id));
    if (first) call('select', { ids: [first.id] });
  }
  app.tool = t;
  if (t !== 'pen' && app.viewport.pen) app.viewport.finishPen(false);
  app.viewport.overlay.style.cursor = app.viewport.cursorFor(t);
  renderToolbar();
  if (app.state) renderStatus();
  app.raster?.tickAnts();  // undo may have brought a pixel selection back
  app.viewport.redraw();
};

app.setTime = (t) => {
  call('set_time', { time: t });
  app.state = call('state');
  app.rev++;
  const sel = app.state.selection;
  app.layerInfo = sel.length >= 1 ? call('layer', { id: sel[0] }) : null;
  app.full = sel.length === 1 ? call('layer_full', { id: sel[0] }) : null;
  tl.update();
  if (app.side === 'layer') app.renderSide();
  app.viewport.redraw();
  writing.paint();
};

// ----- Playback -----

app.togglePlay = () => {
  if (!app.state?.comp.duration) return;
  app.playing = !app.playing;
  app.lastTick = null;
  if (app.playing && app.state.time >= app.state.comp.duration - 1e-6) { call('set_time', { time: 0 }); app.state.time = 0; }
  if (!app.playing) app.setTime(app.state.time);
  if (app.playing) app.sound.start(app.state.time); else app.sound.stop();
  tl.update();
  app.viewport.marks = app.playing ? [] : app.viewport.marks;
  if (!app.playing) writing.paint();
  app.viewport.redraw();
};

app.advance = (now) => {
  const dur = app.state.comp.duration;
  const dt = app.lastTick == null ? 0 : (now - app.lastTick) / 1000;
  app.lastTick = now;
  let t = app.state.time + dt;
  if (t > dur) {
    if (app.loop) { t %= Math.max(dur, 1e-6); app.sound.start(t); }
    else { t = dur; app.playing = false; app.sound.stop(); }
  }
  call('set_time', { time: t });
  app.state.time = t;
  tl.update();
  if (!app.playing) app.setTime(t);
};

app.toggleAutoKey = () => {
  app.autoKey = !app.autoKey;
  tl.update();
};

// ----- Popups and menus -----

let openPopup = null;
function closePopup() {
  if (!openPopup) return;
  openPopup.sub?.remove();
  openPopup.el.remove();
  openPopup.anchor?.setAttribute?.('aria-expanded', 'false');
  openPopup = null;
}
function popup(anchor, content, { at } = {}) {
  closePopup();
  const r = at ? { left: at[0], bottom: at[1], top: at[1] } : anchor.getBoundingClientRect();
  const el = h('div', { class: 'popup', role: 'menu' }, content);
  $('popups').append(el);
  el.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - el.offsetWidth - 8))}px`;
  // Below the button; above it only when there's more room there. Never over the button: a long menu scrolls.
  const below = window.innerHeight - 8 - (r.bottom + 6), above = r.top - 6 - 8;
  el.style.maxHeight = 'none';
  const down = el.offsetHeight <= below || below >= above;
  el.style.maxHeight = `${Math.max(120, down ? below : above)}px`;
  el.style.top = down ? `${r.bottom + 6}px` : `${Math.max(8, r.top - 6 - el.offsetHeight)}px`;
  anchor?.setAttribute?.('aria-expanded', 'true');
  openPopup = { el, anchor };
  el.querySelector('.item:not([disabled]), button')?.focus();
  return el;
}
document.addEventListener('pointerdown', (e) => {
  if (openPopup && !openPopup.el.contains(e.target) && !openPopup.sub?.contains(e.target) && !(openPopup.anchor?.contains?.(e.target))) closePopup();
}, true);
document.addEventListener('keydown', (e) => {
  if (!openPopup) return;
  // In a submenu: its items; ← goes back to the menu.
  const inSub = openPopup.sub?.contains(document.activeElement);
  if (inSub && e.key === 'ArrowLeft') { e.preventDefault(); const from = openPopup.subFrom; openPopup.sub.remove(); openPopup.sub = null; from?.focus(); return; }
  if (!inSub && e.key === 'ArrowRight' && document.activeElement?.classList.contains('has-sub')) { e.preventDefault(); document.activeElement.click(); return; }
  const items = [...(inSub ? openPopup.sub : openPopup.el).querySelectorAll('.item:not([disabled])')];
  const i = items.indexOf(document.activeElement);
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closePopup(); }
  else if (e.key === 'ArrowDown') { e.preventDefault(); items[(i + 1) % items.length]?.focus(); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); items[(i - 1 + items.length) % items.length]?.focus(); }
}, true);

/// A menu item that opens `items()` beside it (hover, click or →).
function sub(label, items, { enabled = true } = {}) {
  const b = h('button', { class: 'item has-sub', role: 'menuitem', 'aria-haspopup': 'menu', disabled: !enabled,
    onClick: (e) => { e.stopPropagation(); openSub(b, items()); } }, h('span', {}, label), h('span', { class: 'sub-arrow', html: icon('right', 14) }));
  b.addEventListener('pointerenter', () => { if (enabled && openPopup?.subFrom !== b) openSub(b, items()); });
  b.subItems = items;
  return b;
}
function openSub(from, content) {
  if (!openPopup) return;
  openPopup.sub?.remove();
  const el = h('div', { class: 'popup sub', role: 'menu' }, content.filter((x) => x != null).map((x) => (x === '-' ? h('hr') : x)));
  $('popups').append(el);
  const r = from.getBoundingClientRect();
  const left = r.right + el.offsetWidth + 4 > window.innerWidth ? r.left - el.offsetWidth - 2 : r.right + 2;
  el.style.left = `${Math.max(8, left)}px`;
  el.style.maxHeight = `${window.innerHeight - 16}px`;
  el.style.top = `${Math.max(8, Math.min(r.top - 6, window.innerHeight - el.offsetHeight - 8))}px`;
  openPopup.sub = el;
  openPopup.subFrom = from;
}
// Moving onto another item of the menu closes an open submenu.
document.addEventListener('pointerover', (e) => {
  if (!openPopup?.sub) return;
  const it = e.target.closest?.('.item');
  if (it && openPopup.el.contains(it) && it !== openPopup.subFrom && !it.classList.contains('has-sub')) { openPopup.sub.remove(); openPopup.sub = null; openPopup.subFrom = null; }
});

/// A menu item; `pic` names an icon shown before the label (a picture of what it does).
// In a browser these belong to the browser (it never hands them over), so
// they aren't offered there.
const BROWSER_KEYS = new Set(['Ctrl+N', 'Ctrl+T', 'Ctrl+W', 'Ctrl+Q', 'Ctrl+Shift+N', 'Ctrl+Shift+T', 'Ctrl+Shift+W', 'Ctrl+Tab', 'Ctrl+Shift+Tab']);
const IN_BROWSER = /^https?:$/.test(location.protocol) && location.hostname !== 'tauri.localhost';
// Closing a tab and quitting are too common to go without: Alt there instead.
const CLOSE_KEYS = IN_BROWSER ? 'Alt+W' : 'Ctrl+W';
const QUIT_KEYS = IN_BROWSER ? 'Alt+Q' : 'Ctrl+Q';
const keysHere = (keys) => (keys && IN_BROWSER && BROWSER_KEYS.has(keys) ? '' : keys);

function item(label, { keys, run, enabled = true, checked, pic } = {}) {
  keys = keysHere(keys);
  return h('button', {
    class: 'item', role: checked === undefined ? 'menuitem' : 'menuitemcheckbox', 'aria-checked': checked === undefined ? null : String(!!checked),
    disabled: !enabled, onClick: () => { closePopup(); run?.(); },
  }, pic ? h('span', { class: 'with-icon' }, h('span', { html: icon(pic, 18) }), label) : h('span', {}, label), keys ? h('span', { class: 'keys' }, shown(keys)) : null,
  checked === undefined ? null : h('span', { class: 'check', html: checked ? icon('check', 16) : '' }));
}

const sel = () => app.state?.selection || [];
const one = () => (sel().length === 1 ? app.layerById(sel()[0]) : null);
const MENUS = [
  { label: 'File', items: () => [
    item('New canvas…', { keys: 'Ctrl+N', run: () => showNew() }),
    item('New diagram', { run: () => newTab('Diagram', () => call('new', { width: 1920, height: 1080, duration: 0, name: 'Diagram', kind: 'diagram' })) }),
    item('Open…', { keys: 'Ctrl+O', run: () => openCanvas() }),
    item(server.session ? `Open from ${server.name}…` : 'Open from a server…', { run: () => openFromServer() }),
    ...(recentList().length ? [heading('Open recent'), ...recentList().map((r) => item(r.name, { keys: r.when, run: () => openRecent(r) })),
      item('Clear the list', { run: () => { try { localStorage.removeItem('draw.recent'); } catch { /* */ } toast('Cleared the recent canvases.'); } })] : []),
    '-',
    item('Save', { keys: 'Ctrl+S', run: () => save() }),
    item('Save as…', { keys: 'Ctrl+Shift+S', run: () => save(true) }),
    item(server.session ? `Save to ${server.name}…` : 'Save to a server…', { run: () => saveToServer(frontTab()?.title, { beforeSave: nameCanvas }) }),
    frontTab()?.remote ? item('Share…', { run: () => shareDialog(frontTab().id, frontTab().title) }) : null,
    frontTab()?.remote ? item('Save a checkpoint…', { run: () => checkpointDialog(frontTab().id, frontTab().title) }) : null,
    frontTab()?.remote ? item('Version history…', { run: () => historyDialog(frontTab().id, frontTab().title) }) : null,
    item('Import…', { keys: 'Ctrl+I', run: () => importFile() }),
    item(one()?.kind === 'picture' ? 'Trace the selected picture into shapes…' : 'Trace a picture into shapes…', { run: () => traceDialog() }),
    item('Export…', { keys: 'Ctrl+E', run: () => openExport() }),
    item('Print…', { keys: 'Ctrl+P', run: () => printFrame() }),
    '-',
    item('Canvas settings…', { run: () => canvasDialog() }),
    '-',
    item('Close tab', { keys: shown(CLOSE_KEYS), run: () => closeTab(engineTabs.front()) }),
    '-',
    platform.DESKTOP || servedHere ? item('Quit', { keys: shown(QUIT_KEYS), run: () => quit() }) : item('Back to the website', { run: () => backToSite() }),
  ] },
  { label: 'Edit', items: () => [
    item(app.state?.undo ? `Undo ${app.state.undo.toLowerCase()}` : 'Undo', { keys: 'Ctrl+Z', enabled: !!app.state?.undo, run: () => app.run('undo') }),
    item(app.state?.redo ? `Redo ${app.state.redo.toLowerCase()}` : 'Redo', { keys: 'Ctrl+Shift+Z', enabled: !!app.state?.redo, run: () => app.run('redo') }),
    '-',
    item('Cut', { keys: 'Ctrl+X', enabled: sel().length > 0, run: () => copyLayers(true) }),
    item('Copy', { keys: 'Ctrl+C', enabled: sel().length > 0, run: () => copyLayers(false) }),
    item('Paste', { keys: 'Ctrl+V', run: () => pasteLayers() }),
    item('Duplicate', { keys: 'Ctrl+D', enabled: sel().length > 0, run: () => duplicateSel() }),
    item('Delete', { keys: 'Delete', enabled: sel().length > 0, run: () => app.run('delete') }),
    '-',
    item('Select all', { keys: 'Ctrl+A', run: () => selectAll() }),
    item('Select none', { keys: 'Esc', enabled: sel().length > 0, run: () => app.select([]) }),
    item('Select next layer', { keys: 'Tab', run: () => selectStep(1) }),
    item('Select previous layer', { keys: 'Shift+Tab', run: () => selectStep(-1) }),
    sub('Pixels (raster layer)', () => app.raster.selectionItems().map((x) => (x === '-' ? h('hr') : item(x[0], { enabled: x[1], keys: x[3], run: x[2] }))), { enabled: !!app.raster?.layer() }),
    '-',
    item('Settings…', { keys: 'Ctrl+,', run: () => openSettings() }),
  ] },
  { label: 'Insert', items: () => [
    sub('Shape', () => SHAPES.map((sh) => item(sh.label, { keys: sh.key, run: () => app.setTool('shape', sh.id) }))),
    item('Text', { keys: 'T', run: () => app.setTool('text') }),
    item('Path with the pen', { keys: 'P', run: () => app.setTool('pen') }),
    item('Freehand line', { keys: 'N', run: () => app.setTool('pencil') }),
    item('Brush stroke', { keys: 'B', run: () => app.setTool('brush') }),
    '-',
    item('Picture…', { run: () => insertPicture() }),
    item('Picture from a server…', { title: 'A page of a Write document on a Xenify server, kept up to date when it changes', run: () => insertServerPicture() }),
    item('Effect…', { run: () => effectGallery() }),
    item('Diagram shapes (UML, flowchart, network)', { pic: 'shapes', checked: shapesShown(), run: () => showShapes(!shapesShown()) }),
    item('Emoji…', { run: () => emojiPicker(app, h, { modal, toast }) }),
    // GIFs come through the server, when its admin gave it a GIPHY key.
    ...(aiServer.gifs ? [item('GIF (GIPHY)…', { run: () => gifPicker(app, h, { modal, toast, addPicture }) })] : []),
    item('Sound…', { run: () => insertSound() }),
    item('Video…', { run: () => insertVideo() }),
    item('Camera', { run: () => { if (app.run('add_camera') != null) toast('Added a camera framing the canvas: move, turn or zoom it. View › Look through the camera shows what it sees (exports always do).'); } }),
    '-',
    item('3D model…', { title: 'Turn a 3D model (STL, OBJ, glTF, 3MF) the way you want it and place it as a picture', run: () => insertModel() }),
  ] },
  { label: 'Layer', items: () => [
    item('New layer', { run: () => app.run('add_folder') }),
    item('New raster layer', { title: 'A layer of pixels to paint on (brush, eraser, fill, magic wand)', run: () => { if (app.run('add_raster_layer') != null) app.setTool('r-brush'); } }),
    item('Convert to raster', { title: 'Turn the selected layers into pixels, each its own raster layer', enabled: sel().some((id) => !app.layerById(id)?.raster), run: () => rasterize(false) }),
    item('Merge into one raster layer', { title: 'Flatten the selected layers into one layer of pixels', enabled: sel().length > 1, run: () => rasterize(true) }),
    item('Convert to vector shapes…', { title: 'Trace the selected picture into shapes', enabled: one()?.kind === 'picture', run: () => traceDialog() }),
    one()?.model ? item('Turn the 3D model again…', { title: 'Turn the model this picture was drawn from, and draw it again', run: () => modelDialog(app, h, { modal, toast }, { layer: one().id }) }) : null,
    item('Picture size…', { title: 'How many pixels the selected picture or raster layer has (it stays the same size on the canvas)', enabled: !!app.raster?.layer(), run: () => pictureSizeDialog() }),
    item('Extend the picture…', { title: 'Add pixels around the picture, as far as the canvas or by an amount; AI can fill them in', enabled: !!app.raster?.layer(), run: () => extendDialog() }),
    '-',
    item('Rename…', { keys: 'F2', enabled: !!one(), run: () => renameLayer(one().id) }),
    item(one()?.visible === false ? 'Show' : 'Hide', { enabled: !!one(), run: () => app.run('set_meta', { id: one().id, visible: !one().visible }) }),
    item(one()?.locked ? 'Unlock' : 'Lock', { enabled: !!one(), run: () => app.run('set_meta', { id: one().id, locked: !one().locked }) }),
    ...(one()?.linked ? [item('Keep this picture (stop updating)', { title: 'It then stays as it is now, even when the document on the server changes', run: () => app.run('link_keep', { id: one().id }) })] : []),
    '-',
    item('Group', { keys: 'Ctrl+G', enabled: sel().length > 0, run: () => app.run('group') }),
    item('Ungroup', { keys: 'Ctrl+Shift+G', enabled: sel().some((id) => app.layerById(id)?.kind === 'group'), run: () => app.run('ungroup') }),
    item('Clipping mask', { keys: 'Ctrl+7', enabled: sel().length > 1, run: () => app.run('clip', { make: true }) }),
    item('Release clipping mask', { keys: 'Ctrl+Alt+7', enabled: sel().length > 0, run: () => app.run('clip', { make: false }) }),
    item('Convert to symbol', { keys: 'Ctrl+Shift+C', enabled: sel().length > 0, run: () => app.run('precompose') }),
    item('New diagram symbol', { run: () => newDiagramSymbol() }),
    sub('Arrange', () => [
    item('Bring to front', { keys: 'Ctrl+Shift+]', enabled: sel().length > 0, run: () => app.run('arrange', { to: 'front' }) }),
    item('Bring forward', { keys: 'Ctrl+]', enabled: sel().length > 0, run: () => app.run('arrange', { to: 'forward' }) }),
    item('Send backward', { keys: 'Ctrl+[', enabled: sel().length > 0, run: () => app.run('arrange', { to: 'backward' }) }),
    item('Send to back', { keys: 'Ctrl+Shift+[', enabled: sel().length > 0, run: () => app.run('arrange', { to: 'back' }) }),
    item('Remove perspective', { enabled: sel().length > 0, run: () => app.run('clear_distort') }),
    ]),
    sub('Turn and flip', () => [
    item('Rotate 90° right', { enabled: sel().length > 0, run: () => app.run('rotate_by', { degrees: 90 }) }),
    item('Rotate 90° left', { enabled: sel().length > 0, run: () => app.run('rotate_by', { degrees: -90 }) }),
    item('Rotate 180°', { enabled: sel().length > 0, run: () => app.run('rotate_by', { degrees: 180 }) }),
    item('Flip horizontally', { pic: 'flipH', enabled: sel().length > 0, run: () => app.run('flip', { axis: 'horizontal' }) }),
    item('Flip vertically', { pic: 'flipV', enabled: sel().length > 0, run: () => app.run('flip', { axis: 'vertical' }) }),
    item('Rotate by…', { enabled: sel().length > 0, run: () => rotateDialog() }),
    ]),
    item('Position and size…', { keys: 'Ctrl+T', enabled: sel().length > 0, run: () => positionSizeDialog() }),
  ] },
  { label: 'Shape', items: () => shapeMenu() },
  { label: 'Slide', items: () => slides.menuItems() },
  { label: 'Animate', items: () => [
    ...(app.state?.comp.duration ? [] : [item('Animate this canvas (adds a timeline)', { run: () => animateStill() }), '-']),
    item(app.playing ? 'Pause' : 'Play', { keys: 'Space', enabled: !!app.state?.comp.duration, run: () => app.togglePlay() }),
    item('Go to start', { keys: 'Home', run: () => app.setTime(0) }),
    item('Loop', { checked: app.loop, run: () => { app.loop = !app.loop; } }),
    '-',
    item('Auto keyframes', { checked: app.autoKey, run: () => app.toggleAutoKey() }),
    '-',
    item('New drawing frame here', { title: 'Frame-by-frame drawing: the raster layer showing now ends at the playhead and a new, empty one starts there', enabled: !!app.raster?.layer(), run: () => newFrame(false) }),
    item('New drawing frame (a copy)', { title: 'Like New drawing frame, starting from a copy of this frame’s pixels', enabled: !!app.raster?.layer(), run: () => newFrame(true) }),
    item('Onion skin', { checked: !!app.onion, title: 'While drawing frame by frame, the frame before shows faintly under this one (only on screen)', run: () => { app.onion = app.run('raster_onion', { on: !app.onion }); app.viewport.redraw(); } }),
    item('Graph editor', { checked: !!tl?.showGraph, run: () => tl.toggleGraph() }),
    sub('Add animation (from the playhead)', () => (app.animPresets ||= call('anim_presets')).map((pr) => item(pr.name, { enabled: sel().length > 0, run: () => app.run('apply_preset', { name: pr.name, stagger: sel().length > 1 ? 0.15 : 0 }) })), { enabled: sel().length > 0 }),
    heading('Moving parts'),
    item('Bones (J)…', { title: 'Turn parts about their joints, like an arm at the shoulder and elbow', run: () => { app.setTool('bones'); app.hint?.('Bones: drag a part’s dot to its joint (shoulder, elbow), drop it on the part above to hang it there; then drag parts to pose them.'); } }),
    item('Stop hanging from its parent', { enabled: sel().length === 1 && !!app.layerInfo?.has_parent, title: 'It stays where it is but no longer moves with the part it hung from', run: () => app.run('attach', { id: sel()[0], parent: null }) }),
    item('Follow a path…', { enabled: sel().length > 0, title: 'Send the selected layer along a line or round a shape (a circle…)', run: () => followPathDialog() }),
    item('Fall and bounce…', { enabled: sel().length > 0, run: () => physicsDialog() }),
  ] },
  { label: 'Review', items: () => [
    item('AI assistant', { keys: 'Ctrl+K', run: () => showSide('ai') }),
    item('Spelling and grammar', { keys: 'F7', run: () => showSide('writing') }),
    item('Check again', { run: () => writing.run() }),
  ] },
  { label: 'AI', items: () => [
    item('New canvas from a description…', { run: () => aiNewDialog(app, h, aiCtx()) }),
    item('Add a slide to this presentation…', { keys: 'Ctrl+Alt+M', run: () => aiSlideDialog(app, h, 'new_slide', aiCtx()) }),
    item('Modify this slide with AI…', { keys: 'Ctrl+Alt+E', run: () => aiSlideDialog(app, h, 'modify_slide', aiCtx()) }),
    item('New illustration…', { run: () => illustrationDialog(app, h, aiCtx()) }),
    item('Select with AI…', { title: 'Say what to select in the picture (“the jacket”) and an image model finds it', enabled: one()?.kind === 'picture', run: () => aiSelectDialog(app, h, aiCtx()) }),
    item('Remove the background', { title: 'Keep what’s in front of the picture; the rest becomes see-through', enabled: one()?.kind === 'picture', run: () => removeBackground(app, h, aiCtx()) }),
    item('Change this picture…', { pic: 'sparkles', title: 'Describe a change to the selected picture or raster layer (only inside the selection, when there is one); reference pictures can show what you mean', enabled: one()?.kind === 'picture', run: () => aiEditDialog(app, h, aiCtx()) }),
    item('New video clip…', { title: 'Made by the video model on your Xenify server', run: () => videoDialog(app, h, { ...aiCtx(), openSettings: (tab) => openSettings(tab || 'server'), importVideo }) }),
    '-',
    item('AI assistant', { keys: 'Ctrl+K', run: () => showSide('ai') }),
  ] },
  { label: 'View', items: () => [
    item('Zoom in', { keys: 'Ctrl+=', run: () => app.viewport.zoomTo(app.viewport.k * 1.25) }),
    item('Zoom out', { keys: 'Ctrl+-', run: () => app.viewport.zoomTo(app.viewport.k / 1.25) }),
    item('Actual size', { keys: 'Ctrl+1', run: () => app.viewport.zoomTo(1) }),
    sub('Zoom to', () => [25, 50, 200, 400].map((p) => item(`${p}%`, { run: () => app.viewport.zoomTo(p / 100) }))),
    item('Fit on screen', { keys: 'Ctrl+0', run: () => app.viewport.fit() }),
    '-',
    item('Rulers', { keys: 'Ctrl+R', checked: app.prefs.showRulers, run: () => { app.prefs.showRulers = !app.prefs.showRulers; app.savePrefs(); } }),
    item('Grid', { keys: "Ctrl+'", checked: app.prefs.showGrid, run: () => { app.prefs.showGrid = !app.prefs.showGrid; app.savePrefs(); } }),
    item('Guides', { keys: 'Ctrl+;', checked: app.prefs.showGuides, run: () => { app.prefs.showGuides = !app.prefs.showGuides; app.savePrefs(); } }),
    item('Slices', { checked: app.prefs.showSlices !== false, run: () => { app.prefs.showSlices = app.prefs.showSlices === false; app.savePrefs(); } }),
    item('Clear guides', { run: () => app.run('set_guides', { guides: [] }) }),
    sub('Snap (hold Ctrl while dragging: off)', () => [
    item('To objects and the frame', { checked: app.prefs.snapObjects, run: () => { app.prefs.snapObjects = !app.prefs.snapObjects; app.savePrefs(); } }),
    item('To guides', { checked: app.prefs.snapGuides, run: () => { app.prefs.snapGuides = !app.prefs.snapGuides; app.savePrefs(); } }),
    item('To the grid', { checked: app.prefs.snapGrid, run: () => { app.prefs.snapGrid = !app.prefs.snapGrid; app.savePrefs(); } }),
    item('Grid size and snap margin…', { run: () => gridDialog() }),
    ]),
    '-',
    item('Show frame times', { checked: !$('perf').hidden, run: () => togglePerf() }),
    item('Run speed test', { enabled: !!app.state?.comp.duration, run: () => speedTest() }),
    item('Dark mode', { checked: document.documentElement.dataset.theme === 'dark', run: () => toggleDark() }),
    item('Look through the camera', { checked: call('look_through').on, enabled: call('look_through').has_camera, run: () => { const on = !call('look_through').on; call('look_through', { on }); app.viewport.redraw(); } }),
    '-',
    sub('Panels', () => [
      ...PANELS.map(([key, label]) => key === 'shapes' ? item(label, { checked: shapesShown(), run: () => showShapes(!shapesShown()) }) : item(label, { checked: !app.prefs.hidden?.[key], run: () => { app.prefs.hidden = { ...app.prefs.hidden, [key]: !app.prefs.hidden?.[key] }; app.savePrefs(); applyLayout(); } })),
      item('Timeline: only its bar', { checked: !!app.prefs.tlMin, title: 'Play and step buttons without the layer rows, for more room', run: () => app.minTimeline() }),
      h('hr'),
      item('Reset the layout', { run: () => { delete app.prefs.hidden; delete app.prefs.widths; delete app.prefs.tlHeight; delete app.prefs.tlMin; app.savePrefs(); applyLayout(); toast('Panels are back where they started.'); } }),
    ]),
  ] },
  { label: 'Help', items: () => [
    ...(updateReady ? [item(`Restart to use version ${updateReady}`, { title: 'Xenify Hub installed it; your canvases are asked about first', run: () => quit(true) }), '-'] : []),
    item('Find a command…', { keys: 'Ctrl+Space', run: () => commandSearch() }),
    item('Keyboard shortcuts', { keys: 'Ctrl+/', run: () => shortcuts() }),
    item('Credits and licences', { run: () => credits() }),
    item('About Xenify Draw', { run: () => about() }),
  ] },
];

function heading(label) {
  return h('div', { class: 'menu-head', role: 'presentation' }, label);
}

const COMBINE = [
  ['unite', 'Unite', 'unite', 'Merge into one shape'],
  ['minus_front', 'Minus front', 'minusFront', 'Cut the shapes on top out of the bottom one'],
  ['intersect', 'Intersect', 'intersect', 'Keep only where they all overlap'],
  ['exclude', 'Exclude', 'exclude', 'Remove where they overlap'],
];
const ALIGN = [
  ['left', 'Left edges', 'alignLeft'], ['center', 'Centres across', 'alignCenter'], ['right', 'Right edges', 'alignRight'],
  ['top', 'Tops', 'alignTop'], ['middle', 'Middles', 'alignMiddle'], ['bottom', 'Bottoms', 'alignBottom'],
];

/// Combines the selected shapes (Pathfinder).
function combine(op) {
  if (sel().length < 2) { toast('Select two or more shapes first (Shift+click).'); return; }
  app.run('path_op', { op });
}

function align(edge, to) {
  if (!sel().length) { toast('Select layers to align.'); return; }
  const moved = app.run('align', { edge, to: to || (sel().length > 1 ? 'selection' : 'canvas') });
  if (moved === false) toast('Already aligned.');
}

function pathTool(name, label) {
  const out = app.run(name);
  if (out?.before != null) toast(`${label}: ${out.before} → ${out.after} points.`);
}

function shapeMenu() {
  const n = sel().length;
  return [
    heading('Combine shapes'),
    ...COMBINE.map(([op, label, pic]) => item(label, { pic, enabled: n > 1, run: () => combine(op) })),
    heading(n > 1 ? 'Align to each other' : 'Align to the canvas'),
    ...ALIGN.map(([edge, label, pic]) => item(label, { pic, enabled: n > 0, run: () => align(edge) })),
    item('Distribute across', { pic: 'distH', enabled: n > 2, run: () => app.run('distribute', { axis: 'horizontal' }) }),
    item('Distribute down', { pic: 'distV', enabled: n > 2, run: () => app.run('distribute', { axis: 'vertical' }) }),
    heading('Path'),
    item('Edit points', { pic: 'node', keys: 'A', enabled: n === 1, run: () => app.setTool('node') }),
    item('Convert to path', { pic: 'toPath', enabled: n > 0, run: () => { if (app.run('convert_to_path') !== null) app.setTool('node'); } }),
    item('Simplify', { pic: 'simplify', enabled: n > 0, run: () => pathTool('simplify', 'Simplified') }),
    item('Subdivide', { pic: 'subdivide', enabled: n > 0, run: () => pathTool('subdivide', 'Subdivided') }),
    item('Outline stroke', { pic: 'outlineStroke', enabled: n > 0, run: () => app.run('outline_stroke') }),
    item('Offset path', { pic: 'offsetPath', enabled: n > 0, run: () => { if (app.run('add_modifier', { preset: 'OffsetPath' }) !== null) { app.openSections.modifiers = true; toast('Grows or shrinks the shapes by a distance: change it under Modifiers (Apply makes it permanent).'); } } }),
    item('Knife', { pic: 'knife', keys: 'C', run: () => app.setTool('knife') }),
  ];
}

// ----- Clipboard (layers as text, so they also paste into another canvas or window) -----

let clip = null;
async function copyLayers(cut) {
  const text = call('copy');
  if (!text) return;
  clip = text;
  try { await navigator.clipboard?.writeText(text); } catch { /* kept here only */ }
  if (cut) app.run('delete');
  toast(cut ? 'Cut.' : `Copied ${sel().length === 1 ? 'the layer' : 'the layers'}.`);
}
async function pasteLayers(text) {
  if (text == null) {
    try { text = await navigator.clipboard?.readText(); } catch { text = null; }
  }
  if (!text || !text.includes('xenify_draw_layers')) text = clip;
  if (!text) { toast('Nothing to paste.'); return; }
  app.run('paste', { text });
}
window.addEventListener('copy', (e) => { if (isTyping(e) || newScreen) return; if (tl.hovered && tl.selKeys.size) { e.preventDefault(); tl.copyKeys(); return; } if (!sel().length) return; e.preventDefault(); const t = call('copy'); if (t) { clip = t; e.clipboardData.setData('text/plain', t); } });
window.addEventListener('cut', (e) => { if (isTyping(e) || !sel().length || newScreen) return; e.preventDefault(); const t = call('copy'); if (t) { clip = t; e.clipboardData.setData('text/plain', t); app.run('delete'); } });

function selectAll() {
  if (app.tool === 'node') { app.viewport.points.selectAll(); return; }
  if (tl.hovered) { tl.selectAllKeys(); return; }
  app.select(app.state.layers.filter((l) => l.depth === 0 && !l.locked).map((l) => l.id));
}

/// Tab: the next (or previous) layer of the list.
/// Duplicate; once the copies are moved, turned or scaled, Duplicate again repeats that (step and repeat).
function duplicateSel() {
  const r = app.run('duplicate');
  if (r?.again) toast('Duplicated, with the same change again (step and repeat).');
  else if (r) toast('Duplicated. Move, turn or scale the copy, then Duplicate again to repeat that.');
}

/// An arrow key: nudges the selection (Shift: 10 px), or with nothing selected steps a frame.
let shiftHeld = false;
window.addEventListener('keydown', (e) => { shiftHeld = e.shiftKey; }, true);
function nudgeKey(x, y) {
  if (!sel().length) { if (x) tl.stepFrame(x); return; }
  const d = shiftHeld ? 10 : 1;
  app.run('nudge', { dx: x * d, dy: y * d });
}

function selectStep(step) {
  const ls = app.state.layers.filter((l) => !l.locked);
  if (!ls.length) return;
  const at = ls.findIndex((l) => l.id === sel()[0]);
  const next = at < 0 ? (step > 0 ? 0 : ls.length - 1) : (at + step + ls.length) % ls.length;
  app.select([ls[next].id]);
}

// ----- The canvas for screen readers -----
// A list of what's on the canvas (name, kind, where and how big, selected), kept in step with it;
// activating an item selects that object. Alt+] / Alt+[ walk the objects in reading order.
let a11yTimer = null;
let a11yBoxes = [];
function objectsInReadingOrder() {
  const byId = new Map(a11yBoxes.map((b) => [b.id, b.box]));
  return app.state.layers.filter((l) => byId.has(l.id) && l.visible).map((l) => ({ l, box: byId.get(l.id) }))
    // Top to bottom in bands, then left to right.
    .sort((a, b) => (Math.abs(a.box[1] - b.box[1]) > 24 ? a.box[1] - b.box[1] : a.box[0] - b.box[0]));
}
function describe(l, box) {
  const u = app.state.comp.units || { px_per_unit: 1, suffix: 'px', decimals: 0 };
  const n = (v) => (v / u.px_per_unit).toFixed(u.decimals ?? 0);
  return `${l.name}, ${l.kind}${box ? `, at ${n(box[0])}, ${n(box[1])}, ${n(box[2] - box[0])} by ${n(box[3] - box[1])} ${u.suffix}` : ''}${l.locked ? ', locked' : ''}${l.visible ? '' : ', hidden'}`;
}
function renderA11yTree() {
  clearTimeout(a11yTimer);
  a11yTimer = setTimeout(() => {
    if (app.playing || !app.state) return;
    try { a11yBoxes = call('layer_boxes'); } catch { a11yBoxes = []; }
    let tree = document.getElementById('canvas-objects');
    if (!tree) {
      tree = h('div', { id: 'canvas-objects', class: 'sr-only', role: 'tree', 'aria-label': 'Objects on the canvas', 'aria-multiselectable': 'true' });
      $('stage').append(tree, h('div', { id: 'canvas-say', class: 'sr-only', role: 'status', 'aria-live': 'polite' }));
    }
    const focused = document.activeElement?.closest?.('#canvas-objects') ? document.activeElement.dataset.id : null;
    const boxes = new Map(a11yBoxes.map((b) => [b.id, b.box]));
    const items = app.state.layers.map((l, i, all) => {
      const on = app.state.selection.includes(l.id);
      const it = h('div', { role: 'treeitem', 'aria-level': String(l.depth + 1), 'aria-selected': String(on), tabindex: (focused ? String(l.id) === focused : i === 0) ? '0' : '-1', 'data-id': String(l.id),
        onClick: (e) => app.select(e.shiftKey ? [...new Set([...app.state.selection, l.id])] : [l.id]),
        onKeydown: (e) => {
          const list = [...tree.querySelectorAll('[role=treeitem]')], k = list.indexOf(e.currentTarget);
          const go = (j) => { e.preventDefault(); e.stopPropagation(); const t = list[Math.max(0, Math.min(list.length - 1, j))]; list.forEach((x) => x.setAttribute('tabindex', '-1')); t.setAttribute('tabindex', '0'); t.focus(); };
          if (e.key === 'ArrowDown') go(k + 1); else if (e.key === 'ArrowUp') go(k - 1); else if (e.key === 'Home') go(0); else if (e.key === 'End') go(list.length - 1);
          else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); e.currentTarget.click(); }
        } }, describe(l, boxes.get(l.id)));
      return it;
    });
    tree.replaceChildren(...items);
    if (focused) tree.querySelector(`[data-id="${focused}"]`)?.focus();
  }, 350);
}
function readingStep(step) {
  const list = objectsInReadingOrder().filter((o) => !o.l.locked);
  if (!list.length) return;
  const at = list.findIndex((o) => o.l.id === sel()[0]);
  const o = list[at < 0 ? (step > 0 ? 0 : list.length - 1) : (at + step + list.length) % list.length];
  app.select([o.l.id]);
  const say = document.getElementById('canvas-say');
  if (say) say.textContent = `Selected ${describe(o.l, o.box)}`;
}

/// A keyframe's right-click menu: its easing towards the next one, and remove.
let easings = null;
app.keyMenu = (e, k) => {
  easings ||= call('easings');
  let now = null;
  try { now = call('key_easings', { id: k.id, prop: k.prop }).find((x) => Math.abs(x.time - k.time) < 1e-6)?.name; } catch { /* none */ }
  const nice = easingLabel;
  popup(null, [
    heading('Easing to the next keyframe'),
    ...easings.map((x) => item(nice(x.name), { checked: now === x.name, run: () => app.run('set_easing', { id: k.id, prop: k.prop, time: k.time, easing: x.name }) })),
    h('hr'),
    item('Remove keyframe', { run: () => app.run('remove_key', { id: k.id, prop: k.prop, time: k.time }) }),
  ], { at: [e.clientX, e.clientY] });
};

/// The canvas's right-click menu.
app.canvasMenu = (e, onNothing = false) => {
  if (app.tool === 'node' || app.tool === 'mask') {
    const items = (app.tool === 'mask' ? app.viewport.maskPoints : app.viewport.points).menuItems().map((x) => (x === '-' ? h('hr') : x[2] ? item(x[0], { keys: x[3], enabled: x[1], run: x[2] }) : heading(x[0])));
    popup(null, items, { at: [e.clientX, e.clientY] });
    return;
  }
  const at = app.viewport.toComp(e);
  popup(null, (onNothing || !sel().length ? emptyMenu(at) : selectionMenu()).filter((x) => x != null).map((x) => (x === '-' ? h('hr') : x)), { at: [e.clientX, e.clientY] });
};

/// Layer › Position and size (Ctrl+T): the selection's box typed exactly, in the canvas's units, and its turn.
async function positionSizeDialog() {
  const ids = sel();
  if (!ids.length) { toast('Select something first.'); return; }
  const b = app.viewport.union(call('bounds', { ids }));
  if (!b) return;
  const u = app.units();
  const turn0 = ids.length === 1 ? (app.layerInfo?.props?.rotation?.value ?? 0) : 0;
  const v = { x: b.x0 / u.k, y: b.y0 / u.k, w: (b.x1 - b.x0) / u.k, h: (b.y1 - b.y0) / u.k, turn: turn0, keep: false };
  const ratio = v.w / (v.h || 1);
  const round = (x) => String(Math.round(x * 100) / 100);
  const inputs = {};
  const num = (key, label, unit) => {
    const i = h('input', { class: 'field', type: 'number', step: String(key === 'turn' ? 1 : u.step), value: round(v[key]), 'aria-label': label,
      onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') { e.preventDefault(); document.querySelector('.modal-back .btn.main')?.click(); } },
      onInput: (e) => {
        v[key] = Number(e.target.value) || 0;
        if (v.keep && key === 'w') { v.h = v.w / ratio; inputs.h.value = round(v.h); }
        if (v.keep && key === 'h') { v.w = v.h * ratio; inputs.w.value = round(v.w); }
      } });
    inputs[key] = i;
    return h('label', {}, `${label} (${unit})`, i);
  };
  const ok = await modal('Position and size', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
    h('p', { class: 'muted' }, ids.length === 1 ? 'The layer’s box, from the canvas’s top-left corner.' : 'The box around the selected layers, from the canvas’s top-left corner.'),
    h('div', { class: 'grid2' }, num('x', 'X', u.unit), num('y', 'Y', u.unit), num('w', 'Width', u.unit), num('h', 'Height', u.unit), num('turn', 'Rotation', '°'),
      h('label', { class: 'check-row', style: 'align-self: end' }, h('input', { type: 'checkbox', onChange: (e) => { v.keep = e.target.checked; } }), h('span', {}, 'Keep proportions')))),
  [['Cancel', false], ['Apply', true, true]]);
  if (!ok) return;
  const from = [b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0];
  const to = [v.x * u.k, v.y * u.k, Math.max(1e-6, v.w * u.k), Math.max(1e-6, v.h * u.k)];
  const sx = from[2] > 1e-9 ? to[2] / from[2] : 1, sy = from[3] > 1e-9 ? to[3] / from[3] : 1;
  // Move and scale the box onto the new one, then turn it about its middle.
  const a = (v.turn - turn0) * Math.PI / 180, cos = Math.cos(a), sin = Math.sin(a);
  const cx = to[0] + to[2] / 2, cy = to[1] + to[3] / 2;
  const tx = to[0] - from[0] * sx, ty = to[1] - from[1] * sy;
  const m = [cos * sx, sin * sx, -sin * sy, cos * sy, cos * tx - sin * ty + cx - cos * cx + sin * cy, sin * tx + cos * ty + cy - sin * cx - cos * cy];
  call('transform_begin');
  call('transform', { affine: m });
  app.run('transform_end');
}

/// A click (no drag) with a shape tool: the shape's exact size, centred where you clicked.
app.shapeSizeDialog = async (kind, at) => {
  const u = app.units();
  const line = kind === 'line';
  const last = (app.lastShapeSize ||= {});
  const def = (last[kind] || (line ? [160, 0] : [160, 160])).map((v) => Math.round((v / u.k) * 100) / 100);
  let keep = !line && def[0] === def[1];
  const ins = [0, 1].map((i) => h('input', { class: 'field', type: 'number', step: String(u.step), value: String(def[i]), 'aria-label': line ? (i ? 'Down' : 'Across') : (i ? 'Height' : 'Width'),
    onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') { e.preventDefault(); document.querySelector('.modal-back .btn.main')?.click(); } },
    onInput: (e) => { if (keep) ins[1 - i].value = e.target.value; } }));
  const name = (SHAPES.find((x) => x.id === kind)?.label || 'Shape');
  const ok = await modal(name, h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
    h('div', { class: 'grid2' }, h('label', {}, `${line ? 'Across' : 'Width'} (${u.unit})`, ins[0]), h('label', {}, `${line ? 'Down' : 'Height'} (${u.unit})`, ins[1])),
    line ? null : h('label', { class: 'check-row' }, h('input', { type: 'checkbox', checked: keep, onChange: (e) => { keep = e.target.checked; if (keep) ins[1].value = ins[0].value; } }), h('span', {}, 'Keep proportions')),
    h('small', { class: 'muted' }, 'Tip: drag to draw instead (Shift: in proportion, Alt: from the centre).')), [['Cancel', false], ['Create', true, true]]);
  if (!ok) return;
  const [w, hh] = ins.map((i) => (Number(i.value) || 0) * u.k);
  last[kind] = [w, hh];
  if (line) app.run('add_line', { a: [at[0] - w / 2, at[1] - hh / 2], b: [at[0] + w / 2, at[1] + hh / 2], stroke: app.stroke, stroke_width: Math.max(1, app.strokeWidth) });
  else app.run('add_figure', { kind, x: at[0] - Math.abs(w) / 2, y: at[1] - Math.abs(hh) / 2, w: Math.max(1, Math.abs(w)), h: Math.max(1, Math.abs(hh)), ...app.paint(), ...app.shapeOpts });
};

/// Shift+X: the fill and stroke colours trade places (the selection's too).
function swapColours() {
  [app.fill, app.stroke] = [app.stroke, app.fill];
  [app.fillOn, app.strokeOn] = [app.strokeOn, app.fillOn];
  const ids = sel().filter((id) => app.layerById(id)?.kind === 'shape');
  if (ids.length) app.run('set_paint', { ids, fill: app.fillOn ? app.fill : false, stroke: app.strokeOn ? app.stroke : false });
  else renderToolbar();
}

/// D: white fill, black stroke (the selection's too).
function resetColours() {
  app.fill = '#ffffff'; app.stroke = '#000000'; app.fillOn = true; app.strokeOn = true;
  const ids = sel().filter((id) => app.layerById(id)?.kind === 'shape');
  if (ids.length) app.run('set_paint', { ids, fill: app.fill, stroke: app.stroke });
  else renderToolbar();
}

/// [ and ]: the brush (or pencil line, or sculpt) size.
function resizeBrush(k) {
  if (app.tool === 'brush') app.brushSize = Math.max(1, Math.min(500, Math.round(app.brushSize * k)));
  else if (app.tool === 'sculpt') app.sculpt.size = Math.max(5, Math.min(600, Math.round(app.sculpt.size * k)));
  else if (app.tool === 'pencil') app.strokeWidth = Math.max(0.5, Math.min(200, Math.round(app.strokeWidth * k * 2) / 2));
  else return;
  renderToolbar();
  app.viewport.redraw();
}

/// Layer › Rotate by…: any angle.
async function rotateDialog() {
  const deg = h('input', { class: 'field', type: 'number', step: '1', value: '45', 'aria-label': 'Degrees', onKeydown: (e) => e.stopPropagation() });
  if (await modal('Rotate by', h('label', {}, 'Degrees (negative: the other way)', deg), [['Cancel', false], ['Rotate', true, true]])) app.run('rotate_by', { degrees: Number(deg.value) || 0 });
}

/// Right-click on empty canvas: make something here, what's shown, the canvas.
function emptyMenu(at) {
  const c = app.state.comp;
  const side = Math.max(8, Math.round(Math.min(c.width, c.height) / 4));
  const tool = app.tool;
  const fig = (kind, w, hh, extra = {}) => () => { app.run('add_figure', { kind, x: at[0] - w / 2, y: at[1] - hh / 2, w, h: hh, ...app.paint(), ...app.shapeOpts, ...extra }); app.tool = tool; };
  const locked = app.state.layers.filter((l) => l.locked);
  const P = app.prefs;
  const tog = (label, key) => item(label, { checked: !!P[key], run: () => { P[key] = !P[key]; app.savePrefs(); } });
  return [
    sub('Create here', () => [
      item('Square', { run: fig('rect', side, side) }), item('Rectangle', { run: fig('rect', side * 1.5, side) }),
      item('Circle', { run: fig('ellipse', side, side) }), item('Ellipse', { run: fig('ellipse', side * 1.5, side) }),
      item('Triangle', { run: fig('triangle', side, side) }), item('Hexagon', { run: fig('polygon', side, side, { sides: 6 }) }),
      item('Star', { run: fig('star', side, side) }), item('Gear', { run: fig('gear', side, side) }), item('Heart', { run: fig('heart', side, side) }),
      item('Arrow', { run: fig('arrow', side * 1.5, side) }), item('Speech bubble', { run: fig('bubble', side * 1.5, side) }),
      item('Line', { run: () => app.run('add_line', { a: [at[0] - side / 2, at[1]], b: [at[0] + side / 2, at[1]], stroke: app.stroke, stroke_width: Math.max(1, app.strokeWidth) }) }),
      '-',
      item('Text', { run: () => { const id = app.run('add_text', { x: at[0], y: at[1], text: 'Text', size: Math.max(12, Math.round(c.height / 10)) }); if (id != null) app.viewport.beginEdit(id, true); } }),
      item('Text box', { run: () => { const id = app.run('add_text', { x: at[0] - side, y: at[1] - side / 2, text: 'Text', size: Math.max(12, Math.round(c.height / 20)), box: [side * 2, side] }); if (id != null) app.viewport.beginEdit(id, true); } }),
      item('Empty group', { run: () => { const id = app.run('add_group'); if (id != null) app.run('set_prop', { id, prop: 'position', value: at }); } }),
    ]),
    sub('Effect here', () => (app.effectCat ||= call('effect_gallery')).flatMap((sec) => [heading(sec.section),
      ...sec.effects.map((fx) => item(fx.name, { enabled: !(fx.needs_layer && !sel().length), run: () => { if (app.run('add_gallery_effect', { name: fx.name, at }) != null) toast(`Added ${fx.name}.`); } }))])),
    item('Import…', { keys: 'Ctrl+I', run: () => importFile() }),
    item('Paste', { keys: 'Ctrl+V', run: () => pasteLayers() }),
    '-',
    item('Fit on screen', { keys: 'Ctrl+0', run: () => app.viewport.fit() }),
    sub('Show', () => [tog('Rulers', 'showRulers'), tog('Grid', 'showGrid'), tog('Guides', 'showGuides'), tog('Slices', 'showSlices'), '-',
      tog('Snap to objects', 'snapObjects'), tog('Snap to guides', 'snapGuides'), tog('Snap to the grid', 'snapGrid')]),
    locked.length ? item(`Unlock all (${locked.length})`, { run: () => { for (const l of locked) call('set_meta', { id: l.id, locked: false }); app.changed(); } }) : null,
    item('Canvas settings…', { run: () => canvasDialog() }),
  ];
}

/// Right-click on the selection: everything that acts on it.
function selectionMenu() {
  const ids = sel();
  const n = ids.length;
  const first = app.layerById(ids[0]);
  const layers = ids.map((id) => app.layerById(id)).filter(Boolean);
  const allHidden = layers.every((l) => l.visible === false), allLocked = layers.every((l) => l.locked);
  const isShape = layers.some((l) => l.kind === 'shape');
  const info = app.layerInfo;
  return [
    heading(n === 1 ? first?.name || 'Layer' : `${n} layers`),
    item('Cut', { keys: 'Ctrl+X', run: () => copyLayers(true) }),
    item('Copy', { keys: 'Ctrl+C', run: () => copyLayers(false) }),
    item('Paste', { keys: 'Ctrl+V', run: () => pasteLayers() }),
    item('Duplicate', { keys: 'Ctrl+D', run: () => duplicateSel() }),
    item('Delete', { keys: 'Delete', run: () => app.run('delete') }),
    '-',
    item('Group', { keys: 'Ctrl+G', run: () => app.run('group') }),
    layers.some((l) => l.kind === 'group') ? item('Ungroup', { keys: 'Ctrl+Shift+G', run: () => app.run('ungroup') }) : null,
    item('Convert to symbol', { keys: 'Ctrl+Shift+C', run: () => app.run('precompose') }),
    n === 1 && first?.model ? item('Turn the 3D model again…', { run: () => modelDialog(app, h, { modal, toast }, { layer: first.id }) }) : null,
    sub('Mask', () => [
      item('Clipping mask (the top shape clips the others)', { keys: 'Ctrl+7', enabled: n > 1, run: () => app.run('clip', { make: true }) }),
      item('Release clipping mask', { keys: 'Ctrl+Alt+7', run: () => app.run('clip', { make: false }) }),
      '-',
      item('Add a rectangle mask', { enabled: n === 1, run: () => app.run('add_mask', { ellipse: false }) }),
      item('Add an ellipse mask', { enabled: n === 1, run: () => app.run('add_mask', { ellipse: true }) }),
      item('The shape above masks the other', { enabled: n === 2, run: () => app.run('shape_to_mask') }),
      item('Draw a mask (mask tool)', { keys: 'U', enabled: n === 1, run: () => app.setTool('mask') }),
    ]),
    sub('Combine', () => COMBINE.map(([op, label, pic]) => item(label, { pic, run: () => combine(op) })), { enabled: n > 1 }),
    sub('Align', () => [
      heading(n > 1 ? 'To each other' : 'To the canvas'), ...ALIGN.map(([edge, label, pic]) => item(label, { pic, run: () => align(edge) })),
      n > 1 ? heading('To the canvas') : null, ...(n > 1 ? ALIGN.map(([edge, label, pic]) => item(label, { pic, run: () => align(edge, 'canvas') })) : []),
      '-', item('Spread across', { pic: 'distH', enabled: n > 2, run: () => app.run('distribute', { axis: 'horizontal' }) }), item('Spread down', { pic: 'distV', enabled: n > 2, run: () => app.run('distribute', { axis: 'vertical' }) }),
    ]),
    sub('Arrange', () => [
      item('Bring to front', { keys: 'Ctrl+Shift+]', run: () => app.run('arrange', { to: 'front' }) }), item('Bring forward', { keys: 'Ctrl+]', run: () => app.run('arrange', { to: 'forward' }) }),
      item('Send backward', { keys: 'Ctrl+[', run: () => app.run('arrange', { to: 'backward' }) }), item('Send to back', { keys: 'Ctrl+Shift+[', run: () => app.run('arrange', { to: 'back' }) }),
    ]),
    sub('Transform', () => [
      item('Rotate 90° right', { run: () => app.run('rotate_by', { degrees: 90 }) }), item('Rotate 90° left', { run: () => app.run('rotate_by', { degrees: -90 }) }),
      item('Rotate 180°', { run: () => app.run('rotate_by', { degrees: 180 }) }), item('Rotate by…', { run: () => rotateDialog() }),
      item('Flip horizontally', { pic: 'flipH', run: () => app.run('flip', { axis: 'horizontal' }) }), item('Flip vertically', { pic: 'flipV', run: () => app.run('flip', { axis: 'vertical' }) }),
      '-', item('Position and size…', { keys: 'Ctrl+T', run: () => positionSizeDialog() }), item('Remove perspective', { run: () => app.run('clear_distort') }),
    ]),
    isShape ? sub('Path', () => shapeMenu().filter((x) => !(x?.classList?.contains('menu-head') && /Combine|Align/.test(x.textContent)))) : null,
    n === 1 && first?.kind === 'text' ? sub('Text', () => [
      item('Edit the text', { keys: 'Enter', run: () => app.viewport.beginEdit(first.id) }),
      info?.text && !info.text.path ? item(info.text.frame ? 'Make point text' : 'Make a text box', { run: () => { app.run('set_text_style', { id: first.id, box: !info.text.frame }); if (!info.text.frame) app.setTool('text'); } }) : null,
    ]) : null,
    sub('Effects', () => [...EFFECT_KINDS.map(([k, t]) => item(t, { run: () => app.run('add_effect', { ids, kind: k }) })), '-', item('Effect gallery…', { run: () => effectGallery() })]),
    sub('Modifiers', () => (app.modPresets ||= call('modifier_presets')).map((p) => item(p.pair ? `${p.label} (two layers)` : p.label, { enabled: !p.pair || n === 2, run: () => app.run('add_modifier', { preset: p.key }) }))),
    '-',
    n === 1 && first?.kind === 'picture' ? item('Trace the picture into shapes…', { run: () => traceDialog(first.id) }) : null,
    n === 1 && first?.kind === 'precomp' ? item('Edit the symbol', { run: () => { const f = call('layer_full', { id: first.id }); if (f.layer.comp != null) app.editSymbol(f.layer.comp); } }) : null,
    item('Crop the selection…', { run: () => { app.setTool('crop'); const b = app.viewport.union(call('bounds', { ids })); if (b) { app.viewport.cropRect = [b.x0, b.y0, b.x1, b.y1]; renderToolbar(); app.viewport.redraw(); } } }),
    item(allHidden ? 'Show' : 'Hide', { run: () => { for (const id of ids) call('set_meta', { id, visible: allHidden }); app.changed(); } }),
    item(allLocked ? 'Unlock' : 'Lock', { run: () => { for (const id of ids) call('set_meta', { id, locked: !allLocked }); if (!allLocked) call('select', { ids: [] }); app.changed(); } }),
    n === 1 ? item('Rename…', { keys: 'F2', run: () => renameLayer(first.id) }) : null,
  ];
}

function buildMenus() {
  const bar = $('menus');
  // (Built again once a server is found: not a second set beside the first.)
  bar.replaceChildren();
  for (const m of MENUS) {
    const b = h('button', { class: 'menu-btn', 'aria-haspopup': 'menu', 'aria-expanded': 'false' }, m.label);
    const show = () => popup(b, m.items().filter((x) => x != null).map((x) => (x === '-' ? h('hr') : x)));
    b.addEventListener('click', () => (openPopup?.anchor === b ? closePopup() : show()));
    b.addEventListener('pointerenter', () => { if (openPopup?.anchor?.classList?.contains('menu-btn') && openPopup.anchor !== b) show(); });
    bar.append(b);
  }
}

// ----- Toolbar -----

/// A toolbar too wide for the window takes a second row (rather than hiding tools).
function fitToolbar() {
  const tb = $('toolbar');
  if (!tb) return;
  tb.classList.remove('wrap');
  // A tablet held upright, or a phone: one row that scrolls sideways (fingers swipe it).
  if (window.innerWidth <= 900) return;
  // scrollWidth can miss a last button cut off by the window's edge: where the buttons end counts too.
  const right = tb.getBoundingClientRect().right - parseFloat(getComputedStyle(tb).paddingRight || '0');
  if (tb.scrollWidth > tb.clientWidth + 1 || [...tb.children].some((c) => c.getBoundingClientRect().right > right + 1)) tb.classList.add('wrap');
}
{
  // Only a change of the window's width matters (the second row changes the height).
  let lastW = 0;
  new ResizeObserver(() => { const w = window.innerWidth; if (w !== lastW) { lastW = w; requestAnimationFrame(fitToolbar); } }).observe(document.body);
}

/// The toolbar shows the raster tools: a raster layer is the one you're on, or a raster tool is out.
function rasterMode() {
  return !!app.raster?.layer() || !!app.tool?.startsWith('r-');
}

/// Layer › Convert to raster (`merge`: all the selected layers into one).
function rasterize(merge) {
  const ids = sel().filter((id) => !app.layerById(id)?.raster || merge);
  if (!ids.length) { toast('Select the layers to turn into pixels.'); return; }
  try {
    const made = JSON.parse(engineRaw().rasterize(JSON.stringify(ids), merge));
    app.changed();
    if (made.length) toast(merge && ids.length > 1 ? `Merged ${ids.length} layers into one raster layer.` : made.length > 1 ? `Turned ${made.length} layers into raster layers.` : 'Turned it into a raster layer: paint on it with the raster tools.');
  } catch (e) { toast(String(e.message || e)); }
  afterOffscreenDraw();
}
const afterOffscreenDraw = () => app.viewport.drawNow();

/// The toolbar's “Paint pixels”: the raster tools, on the selected picture or a new raster layer above what's selected.
function paintPixels() {
  if (!app.raster.layer()) {
    const above = sel().length ? Math.max(...sel().map((id) => app.state.layers.findIndex((l) => l.id === id))) : -1;
    const id = app.run('add_raster_layer', above >= 0 ? { above: app.state.layers[above].id } : {});
    if (id == null) return;
  }
  app.setTool('r-brush');
}

function renderToolbar() {
  // A raster layer has its own tools; picking another kind of layer puts the vector ones back.
  if (app.tool?.startsWith('r-') && sel().length && !app.raster?.layer()) app.tool = 'select';
  else if (app.raster?.layer() && !app.tool?.startsWith('r-') && !SHARED_TOOLS.includes(app.tool)) app.tool = 'select';
  if (app.viewport) app.viewport.overlay.style.cursor = app.viewport.cursorFor(app.tool);
  const st = app.state;
  const l = one();
  const info = app.layerInfo;
  const text = l?.kind === 'text';
  const paintOf = info?.paint;
  const fillOn = paintOf ? paintOf.fill : text ? true : app.fillOn;
  const strokeOn = paintOf ? paintOf.stroke : text ? false : app.strokeOn;
  const colour = (label, value, on, onPick, onToggle) => {
    const input = h('input', { type: 'color', value: String(value).slice(0, 7), 'data-rgba': String(value), 'aria-label': label, style: 'position: absolute; inset: 0; opacity: 0; width: 100%; height: 100%; cursor: default' });
    input.addEventListener('input', (e) => onPick(e.target.dataset.rgba || e.target.value, false));
    input.addEventListener('change', (e) => onPick(e.target.dataset.rgba || e.target.value, true));
    const name = label.split(' ')[0];
    const front = (app.activeSlot || 'fill') === (label.startsWith('Stroke') ? 'stroke' : 'fill');
    return h('span', { class: `pick-pair${front ? ' front' : ''}` },
      h('label', { class: 'pick', title: `${label} (X: switch, Shift+X: swap, D: white and black)`, style: 'position: relative' }, h('span', { class: `sw${on ? '' : ' none'}`, style: on ? `background: ${value}` : '' }), name, input),
      onToggle ? h('button', { class: 'mini-toggle', 'aria-pressed': String(!on), 'aria-label': `No ${name.toLowerCase()}`, title: on ? `No ${name.toLowerCase()}` : `Turn the ${name.toLowerCase()} on`, html: icon(on ? 'x' : 'plus', 14), onClick: onToggle }) : null);
  };
  const fillNow = info?.props.fill?.value || (text ? info?.props.text_color?.value : null) || app.fill;
  const strokeNow = info?.props.stroke?.value || app.stroke;
  const widthNow = info?.props.stroke_width?.value ?? app.strokeWidth;
  const targets = () => sel().filter((id) => app.layerById(id)?.kind === 'shape');
  const setFill = (v, done) => {
    app.fill = v;
    app.fillOn = true;
    if (text) app.run('set_prop', { id: l.id, prop: 'text_color', value: v }, { quiet: !done });
    else if (targets().length) app.run('set_paint', { ids: targets(), fill: v }, { quiet: !done });
    else renderToolbar();
  };
  const setStroke = (v, done) => {
    app.stroke = v;
    app.strokeOn = true;
    if (targets().length) app.run('set_paint', { ids: targets(), stroke: v, stroke_width: Math.max(1, app.strokeWidth) }, { quiet: !done });
    else renderToolbar();
  };
  const toggleFill = () => {
    if (targets().length) app.run('set_paint', { ids: targets(), fill: fillOn ? false : app.fill });
    else { app.fillOn = !app.fillOn; renderToolbar(); }
  };
  const toggleStroke = () => {
    if (targets().length) app.run('set_paint', { ids: targets(), stroke: strokeOn ? false : app.stroke, stroke_width: Math.max(1, app.strokeWidth) });
    else { app.strokeOn = !app.strokeOn; if (app.strokeOn && !app.strokeWidth) app.strokeWidth = 4; renderToolbar(); }
  };
  const num = (label, value, apply, { min = 0, max = 1000, step = 1, width = 56 } = {}) => h('label', { class: 'opt', title: label }, h('span', {}, label),
    h('input', { class: 'size-box', type: 'number', min: String(min), max: String(max), step: String(step), value: String(value), style: `width: ${width}px`, 'aria-label': label,
      onChange: (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) apply(Math.min(max, Math.max(min, v))); }, onKeydown: (e) => e.stopPropagation() }));
  const width = num('Width', Math.round(widthNow * 10) / 10, (v) => { app.strokeWidth = v; if (info?.props.stroke_width) app.run('set_prop', { id: l.id, prop: 'stroke_width', value: v }); else renderToolbar(); }, { max: 400, step: 1, width: 52 });
  const c = st?.comp;
  const kindNote = c ? `${c.width} × ${c.height}${c.duration ? ` · ${+c.duration.toFixed(2)} s` : ' · still'}` : '';
  const shape = SHAPES.find((x) => x.id === app.shape) || SHAPES[0];
  const toolBtn = (t) => {
    if (t.id === 'shape') {
      const on = app.tool === 'shape';
      return h('span', { class: 'split' },
        h('button', { class: 'icon-btn', 'aria-label': `${shape.label}${shape.key ? ` (${shape.key})` : ''}`, title: `${shape.label}${shape.key ? ` (${shape.key})` : ''}: drag on the canvas (Shift: even)`, 'aria-pressed': String(on), html: icon(shape.icon), onClick: () => app.setTool('shape') }),
        h('button', { class: 'icon-btn chev', 'aria-label': 'More shapes', title: 'More shapes', 'aria-haspopup': 'menu', html: icon('down', 14), onClick: (e) => popup(e.currentTarget,
          SHAPES.map((x) => item(h('span', { class: 'with-icon' }, h('span', { html: icon(x.icon, 16) }), x.label), { keys: x.key, checked: app.tool === 'shape' && app.shape === x.id, run: () => app.setTool('shape', x.id) }))) }));
    }
    const key = (app.keys || {})[t.id] || t.key;
    return h('button', { class: 'icon-btn', 'aria-label': key ? `${t.label} (${key})` : t.label, title: key ? `${t.label} (${key})` : t.label, 'aria-pressed': String(app.tool === t.id), html: icon(t.icon), onClick: () => app.setTool(t.id) });
  };
  // What the current tool can be set to, beside the colours.
  const o = app.shapeOpts;
  const opts = [];
  if (app.tool === 'shape') {
    if (app.shape === 'polygon') opts.push(num('Sides', o.sides, (v) => { o.sides = v; }, { min: 3, max: 64, width: 50 }));
    if (app.shape === 'star') opts.push(num('Points', o.sides, (v) => { o.sides = v; }, { min: 3, max: 64, width: 50 }), num('Inner %', Math.round(o.inner * 100), (v) => { o.inner = v / 100; }, { min: 5, max: 95, width: 50 }));
    if (app.shape === 'gear') opts.push(num('Teeth', o.teeth, (v) => { o.teeth = v; }, { min: 3, max: 96, width: 50 }), num('Depth %', Math.round(o.depth * 100), (v) => { o.depth = v / 100; }, { min: 2, max: 80, width: 50 }), num('Hole %', Math.round(o.hole * 100), (v) => { o.hole = v / 100; }, { min: 0, max: 90, width: 50 }));
    if (app.shape === 'rounded') opts.push(num('Corners', o.radius, (v) => { o.radius = v; }, { max: 2000, width: 56 }));
  }
  if (app.tool === 'sculpt') {
    opts.push(h('span', { class: 'segs', role: 'radiogroup', 'aria-label': 'Sculpt', style: 'display: inline-flex' },
      [['push', 'Push'], ['attract', 'Pull in'], ['repel', 'Push away']].map(([m, label]) => h('button', { class: `seg${app.sculpt.mode === m ? ' on' : ''}`, role: 'radio', 'aria-checked': String(app.sculpt.mode === m), onClick: () => { app.sculpt.mode = m; renderToolbar(); } }, label))),
    num('Size', app.sculpt.size, (v) => { app.sculpt.size = v; app.viewport.redraw(); }, { min: 2, max: 2000, width: 56 }),
    num('Strength', app.sculpt.strength, (v) => { app.sculpt.strength = v; }, { min: 0.1, max: 100, step: 0.5, width: 50 }));
  }
  if (app.tool === 'mask') {
    opts.push(h('span', { class: 'segs', role: 'radiogroup', 'aria-label': 'Mask shape', style: 'display: inline-flex' },
      [['rect', 'Rectangle'], ['ellipse', 'Ellipse']].map(([m, label]) => h('button', { class: `seg${app.maskShape === m ? ' on' : ''}`, role: 'radio', 'aria-checked': String(app.maskShape === m), onClick: () => { app.maskShape = m; renderToolbar(); } }, label))));
  }
  if (app.tool === 'crop') {
    const r = app.viewport.cropRect;
    const c = app.state.comp;
    opts.push(
      h('button', { class: 'pick', disabled: !r, title: 'Resize the canvas to the rectangle (Enter)', onClick: () => cropCanvas() }, 'Crop canvas'),
      h('button', { class: 'pick', disabled: !r || !sel().length, title: 'Clip the selected layers to the rectangle (a mask)', onClick: () => { if (app.run('crop_layers', { rect: r }) !== null) { app.viewport.cropRect = null; renderToolbar(); } } }, 'Crop layers'),
      h('button', { class: 'pick', disabled: !r, title: 'A named region that exports as its own file', onClick: () => { if (app.run('add_slice', { rect: r }) !== null) { app.viewport.cropRect = null; toast('Added a slice: export it with File › Export (Of: each slice).'); renderToolbar(); } } }, 'Make a slice'),
      h('button', { class: 'pick', onClick: () => { app.viewport.cropRect = [0, 0, c.width, c.height]; renderToolbar(); app.viewport.redraw(); } }, 'All'));
  }
  if (app.tool === 'perspective' || app.tool === 'xform') {
    opts.push(h('button', { class: 'pick', disabled: !sel().length, onClick: () => app.run('clear_distort') }, 'Remove perspective'));
  }
  if (app.tool === 'eyedropper') {
    opts.push(h('span', { class: 'segs', role: 'radiogroup', 'aria-label': 'Pick into', style: 'display: inline-flex' },
      [['both', 'Fill and stroke'], ['fill', 'Fill only'], ['stroke', 'Stroke only']].map(([v, l]) => h('button', { class: `seg${(app.eyeInto || 'both') === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String((app.eyeInto || 'both') === v),
        title: v === 'both' ? 'The fill and stroke of what you click' : `The one colour showing where you click, into the ${v}`, onClick: () => { app.eyeInto = v; renderToolbar(); } }, l))));
  }
  if (app.tool === 'node') {
    // The point tools on the selected corners (or every corner), and showing every handle.
    const pe = app.viewport.points;
    app.bevelSize ??= 20;
    opts.push(
      h('button', { class: 'pick', title: 'Round the selected corners (or every corner) with a curve', onClick: () => pe.edit('round', app.bevelSize) }, 'Round'),
      h('button', { class: 'pick', title: 'Cut the selected corners (or every corner) off with a straight edge', onClick: () => pe.edit('bevel', app.bevelSize) }, 'Bevel'),
      num('By', app.bevelSize, (v) => { app.bevelSize = v; }, { min: 0.5, max: 2000, step: 1, width: 52 }),
      h('label', { class: 'opt', title: 'Show the curve handles of every point, not just the selected ones' },
        h('input', { type: 'checkbox', checked: !!pe.allHandles, onChange: (e) => { pe.allHandles = e.target.checked; app.viewport.redraw(); } }), h('span', {}, 'All handles')));
  }
  if (app.tool === 'connector' && app.state) {
    // New connectors on this canvas: how they run and look.
    const dg = app.state.comp.diagram || {};
    const setD = (f) => { const d = JSON.parse(JSON.stringify(dg)); f(d); app.run('set_comp', { diagram: d }); };
    const sel2 = (label, value, options, apply) => h('label', { class: 'opt' }, h('span', {}, label), h('select', { class: 'field small', style: 'width: auto', 'aria-label': `New connectors: ${label}`, onChange: (e) => apply(e.target.value) },
      options.map(([v, l]) => h('option', { value: v, selected: v === value }, l))));
    opts.push(
      sel2('Route', dg.route || 'orthogonal', [['orthogonal', 'Elbows'], ['straight', 'Straight'], ['curved', 'Curved']], (v) => setD((d) => { d.route = v; })),
      sel2('Jumps', dg.jump || 'arc', [['arc', 'Arc'], ['gap', 'Gap'], ['square', 'Square'], ['none', 'None']], (v) => setD((d) => { d.jump = v; })),
      sel2('Arrow', dg.end_marker?.kind || 'triangle', [['none', 'None'], ['triangle', 'Triangle'], ['open', 'Open'], ['arrow', 'Arrow'], ['diamond', 'Diamond'], ['circle', 'Circle'], ['bar', 'Bar']], (v) => setD((d) => { d.end_marker = { ...(d.end_marker || { size: 1 }), kind: v }; })),
      num('Width', dg.line_width ?? 2, (v) => setD((d) => { d.line_width = v; }), { min: 0.5, max: 40, step: 0.5, width: 48 }),
      h('input', { type: 'color', class: 'color-in', title: 'Line colour of new connectors', 'aria-label': 'Line colour of new connectors', value: (() => { const c = dg.line_color; return c ? `#${[c.r, c.g, c.b].map((x) => Math.round(x * 255).toString(16).padStart(2, '0')).join('')}` : '#33383f'; })(),
        onChange: (e) => { const x = e.target.value; setD((d) => { d.line_color = { r: parseInt(x.slice(1, 3), 16) / 255, g: parseInt(x.slice(3, 5), 16) / 255, b: parseInt(x.slice(5, 7), 16) / 255, a: 1 }; }); } }),
      h('small', { class: 'muted' }, 'Ctrl: leave an end loose'));
  }
  if (app.tool === 'pen' || app.tool === 'pencil') opts.push(lineStyle());
  if (app.tool === 'brush') opts.push(num('Size', app.brushSize, (v) => { app.brushSize = v; }, { min: 1, max: 400, width: 52 }));
  if (app.tool === 'pencil' || app.tool === 'brush') {
    opts.push(h('label', { class: 'opt', title: 'Rough lines, circles, boxes and triangles become clean shapes' },
      h('input', { type: 'checkbox', checked: app.recognize, onChange: (e) => { app.recognize = e.target.checked; } }), h('span', {}, 'Clean shapes')),
    num('Smoothing', app.smoothing, (v) => { app.smoothing = v; }, { min: 1, max: 10, width: 44 }));
  }
  const n = sel().length;
  const drop = (label, iconName, items, enabled) => h('button', { class: 'pick', disabled: !enabled, 'aria-haspopup': 'menu', title: label, onClick: (e) => popup(e.currentTarget, items()) },
    h('span', { html: icon(iconName, 16), style: 'display: inline-flex' }), label, h('span', { html: icon('down', 14), style: 'display: inline-flex' }));
  const undoRedo = () => [
    h('button', { class: 'icon-btn', 'aria-label': 'Undo', title: `Undo${st?.undo ? ` ${st.undo.toLowerCase()}` : ''} (Ctrl+Z)`, disabled: !st?.undo, html: icon('undo'), onClick: () => app.run('undo') }),
    h('button', { class: 'icon-btn', 'aria-label': 'Redo', title: 'Redo (Ctrl+Shift+Z)', disabled: !st?.redo, html: icon('redo'), onClick: () => app.run('redo') })];
  if (rasterMode()) {
    // Raster: the paint tools, the paint colour, the tool's options, the selection and AI.
    const rl = app.raster.layer();
    const seg = (label, value, list, apply) => h('span', { class: 'segs', role: 'radiogroup', 'aria-label': label, style: 'display: inline-flex' },
      list.map(([v, l, tip]) => h('button', { class: `seg${value === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String(value === v), title: tip || l, onClick: () => apply(v) }, l)));
    const check = (label, value, apply, tip) => h('label', { class: 'opt', title: tip || label }, h('input', { type: 'checkbox', checked: !!value, onChange: (e) => apply(e.target.checked) }), h('span', {}, label));
    // A menu of [label, checked, run, tip] rows; '-' is a line, '#Name' a heading.
    const menu = (label, list) => drop(label, 'sliders', () => list.map((x) => (x === '-' ? h('hr') : typeof x === 'string' ? heading(x.slice(1)) : item(h('span', { title: x[3] }, x[0]), { checked: x[1], run: x[2] }))), true);
    const ropts = app.raster.options(h, { num, seg, check, menu, ask: askText }).filter(Boolean);
    const selItems = () => app.raster.selectionItems().map((x) => (x === '-' ? h('hr') : item(x[0], { enabled: x[1], keys: x[3], run: x[2] })));
    const setPaint = (v, done) => { app.fill = String(v).slice(0, 9); app.fillOn = true; if (done) renderToolbar(); };
    put($('toolbar'), undoRedo(), h('span', { class: 'sep' }),
      TOOLS.filter((t) => ['select', 'xform'].includes(t.id)).map(toolBtn),
      app.raster.toolButtons(h, app.keys || {}),
      TOOLS.filter((t) => ['zoom', 'hand'].includes(t.id)).map(toolBtn),
      h('button', { class: 'icon-btn', 'aria-label': 'Vector tools', title: 'Vector tools: the pen, shapes and text, to draw new shapes over the picture', html: icon('pen'), onClick: () => { app.select([]); app.setTool('select'); } }),
      // The tools that don't use a colour (they work with the pixels there) leave it out.
      ...(app.raster.quick() && ['r-brush', 'r-eraser'].includes(app.tool)
        ? [h('span', { class: 'sep' }), h('button', { class: 'pick quick-on', title: 'The brush adds to the selection and the eraser takes away. Click (or Q) to paint pixels again.', onClick: () => app.raster.setQuick(false) },
          h('span', { html: icon('marquee', 16), style: 'display: inline-flex' }), 'Painting selection', h('span', { html: icon('x', 14), style: 'display: inline-flex' }))]
        : ['r-eraser', 'r-smudge', 'r-clone'].includes(app.tool) ? [] : [h('span', { class: 'sep' }), colour('Paint colour', app.fill, true, setPaint, null)]),
      ropts.length ? h('span', { class: 'sep' }) : null, ropts,
      h('span', { class: 'sep' }),
      h('button', { class: 'pick', 'aria-haspopup': 'menu', 'aria-label': 'Selection', title: 'Selection: select all, invert, grow, soften, delete, fill, copy to a new layer (also Edit › Pixels)', disabled: !rl, onClick: (e) => popup(e.currentTarget, selItems()) },
        h('span', { html: icon('marquee', 16), style: 'display: inline-flex' }), h('span', { html: icon('down', 14), style: 'display: inline-flex' })),
      h('button', { class: 'pick', disabled: !rl, style: 'white-space: nowrap; flex: none', title: 'Describe a change and an AI model paints it (only inside the selection, when there is one)', onClick: () => aiEditDialog(app, h, aiCtx()) },
        h('span', { html: icon('sparkles', 16), style: 'display: inline-flex' }), 'Change with AI…'));
    app.raster.tickAnts();
    fitToolbar();
    return;
  }
  put($('toolbar'),
    h('button', { class: 'icon-btn', 'aria-label': 'Undo', title: `Undo${st?.undo ? ` ${st.undo.toLowerCase()}` : ''} (Ctrl+Z)`, disabled: !st?.undo, html: icon('undo'), onClick: () => app.run('undo') }),
    h('button', { class: 'icon-btn', 'aria-label': 'Redo', title: 'Redo (Ctrl+Shift+Z)', disabled: !st?.redo, html: icon('redo'), onClick: () => app.run('redo') }),
    h('span', { class: 'sep' }),
    (c?.kind === 'diagram' ? TOOLS.filter((t) => DIAGRAM_TOOLS.includes(t.id)) : TOOLS).map(toolBtn),
    h('button', { class: `icon-btn${shapesShown() ? ' on' : ''}`, 'aria-pressed': String(shapesShown()), 'aria-label': 'Diagram shapes', title: 'Diagram shapes: flowchart, UML, network, cloud and BPMN, to drag in and join with the Connector', html: icon('shapes'), onClick: () => showShapes(!shapesShown()) }),
    h('button', { class: 'icon-btn', 'aria-label': 'Paint pixels', title: 'Paint pixels: the raster tools (brush, eraser, fill, magic wand), on the selected picture or a new raster layer', html: icon('palette'), onClick: () => paintPixels() }),
    h('span', { class: 'sep' }),
    colour(text ? 'Text colour' : 'Fill colour', fillNow, fillOn, setFill, text ? null : toggleFill),
    colour('Stroke colour', strokeNow, strokeOn, setStroke, text ? null : toggleStroke),
    strokeOn && !text ? width : null,
    strokeOn && !text && !['pen', 'pencil'].includes(app.tool) ? h('button', { class: 'pick', title: 'Line style (calligraphy, ink, marker, dashes…): for the selected lines and new ones', onClick: () => pensDialog(app, h) }, 'Line style') : null,
    opts.length ? h('span', { class: 'sep' }) : null, opts,
    ['select', 'node', 'xform', 'perspective'].includes(app.tool) ? [h('span', { class: 'sep' }),
    drop('Combine', 'combine', () => COMBINE.map(([op, label, ic, tip]) => item(h('span', { class: 'with-icon', title: tip }, h('span', { html: icon(ic, 16) }), label), { run: () => combine(op) })), n > 1),
    drop('Align', 'alignLeft', () => [
      heading(n > 1 ? 'To each other' : 'To the canvas'),
      ...ALIGN.map(([edge, label, ic]) => item(h('span', { class: 'with-icon' }, h('span', { html: icon(ic, 16) }), label), { run: () => align(edge) })),
      n > 1 ? heading('To the canvas') : null,
      ...(n > 1 ? ALIGN.map(([edge, label, ic]) => item(h('span', { class: 'with-icon' }, h('span', { html: icon(ic, 16) }), label), { run: () => align(edge, 'canvas') })) : []),
      heading('Spread evenly'),
      item(h('span', { class: 'with-icon' }, h('span', { html: icon('distH', 16) }), 'Across'), { enabled: n > 2, run: () => app.run('distribute', { axis: 'horizontal' }) }),
      item(h('span', { class: 'with-icon' }, h('span', { html: icon('distV', 16) }), 'Down'), { enabled: n > 2, run: () => app.run('distribute', { axis: 'vertical' }) }),
    ].filter(Boolean), n > 0)] : null,
    h('div', { class: 'spacer' }),
    c ? h('button', { class: 'pick', title: 'Canvas settings', style: 'gap: 8px; border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-ink)', html: `${icon('canvas', 14)}<span class="label">${esc(c.name)} · ${kindNote}</span>`, onClick: () => canvasDialog() }) : null,
  );
  fitToolbar();
}

// ----- Layers -----

const KIND_ICON = { text: 'type', shape: 'square', group: 'group', null: 'group', picture: 'image', precomp: 'canvas', '3d': 'cube', sound: 'sound', video: 'video', camera: 'video', particles: 'star', water: 'drop' };

function renderLayers() {
  const st = app.state;
  const issues = new Map();
  for (const i of writing.issues) if (!issues.has(i.layer) || i.category === 'Spelling') issues.set(i.layer, i.category);
  const add = h('button', { class: 'icon-btn small', 'aria-label': 'Add', title: 'A new layer, or something to put on the canvas', html: icon('plus'), onClick: (e) => popup(e.currentTarget, [
    item('New layer', { run: () => { if (app.run('add_folder') != null) toast('New shapes go into this layer. Click a layer to draw into it instead.'); } }),
    '-',
    item('Shape', { run: () => app.setTool('shape') }), item('Text', { run: () => app.setTool('text') }),
    item('Picture…', { run: () => insertPicture() }), item('Sound…', { run: () => insertSound() }), item('Video…', { run: () => insertVideo() }),
    item('Effect…', { run: () => effectGallery() }), item('Camera', { run: () => app.run('add_camera') }),
    item('Empty group', { run: () => app.run('add_group') }),
  ]) });
  // Layers and groups are folded (what's in them hidden) until you open them.
  app.openLayers ||= new Set();
  const shown = [];
  let hideBelow = null;
  const within = new Map();
  for (const [i, l] of st.layers.entries()) {
    if (hideBelow != null && l.depth > hideBelow.depth) {
      if (st.selection.includes(l.id)) within.set(hideBelow.id, (within.get(hideBelow.id) || 0) + 1);
      continue;
    }
    hideBelow = null;
    const kids = st.layers[i + 1] && st.layers[i + 1].depth > l.depth;
    if (kids && !app.openLayers.has(l.id)) hideBelow = l;
    shown.push(l);
  }
  const FX_NAME = { drop_shadow: 'Drop shadow', glow: 'Glow', blur: 'Blur', adjust_colors: 'Adjust colours' };
  const rows = [];
  for (const l of shown) {
    const on = st.selection.includes(l.id);
    const cat = issues.get(l.id);
    const i = st.layers.indexOf(l);
    const hasKids = st.layers[i + 1] && st.layers[i + 1].depth > l.depth;
    const open = app.openLayers.has(l.id);
    const active = l.folder && st.active_layer === l.id;
    const badge = (text, title) => h('span', { class: 'lbadge', title }, text);
    const row = h('div', { class: `lrow${on ? ' on' : ''}${l.visible ? '' : ' hidden'}${app.pendingLayers?.has(l.id) ? ' pending' : ''}${l.folder ? ' folder' : ''}${active ? ' active' : ''}`,
      title: l.folder ? (active ? 'The layer you’re on: what you draw goes in here' : 'A layer: click to draw into it (and select what’s in it)') : null, role: 'option', 'aria-selected': String(on), tabindex: '0', style: `padding-left: ${4 + l.depth * 18}px`,
      onClick: (e) => { if (e.target.closest('.mini, .lcaret') || app.layerDragged) return; pickLayer(l.id, e, shown); },
      onDblclick: (e) => { if (!e.target.closest('.mini, .lcaret')) renameLayer(l.id); },
      onContextmenu: (e) => { e.preventDefault(); if (!on) app.select([l.id]); layerMenu(e, l); },
      onKeydown: (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); app.select([l.id]); } if (e.key === 'F2') renameLayer(l.id); } },
    hasKids ? h('button', { class: 'lcaret', 'aria-label': open ? `Hide what’s in ${l.name}` : `Show what’s in ${l.name}`, 'aria-expanded': String(open), html: icon(open ? 'down' : 'right', 14),
      onClick: () => { if (open) app.openLayers.delete(l.id); else app.openLayers.add(l.id); renderLayers(); } }) : h('span', { class: 'lcaret-gap' }),
    h('span', { class: 'lthumb', 'data-thumb': String(l.id) }, h('span', { html: icon(KIND_ICON[l.kind] || 'square', 16), style: 'display: inline-flex' })),
    h('span', { class: 'lname', title: l.name }, l.name),
    l.folder || l.group ? h('span', { class: 'lcount', title: `${l.members} inside` }, String(l.members)) : null,
    within.get(l.id) ? h('span', { class: 'lsel', title: `${within.get(l.id)} selected inside`, 'aria-label': `${within.get(l.id)} selected inside` }) : null,
    l.fx?.length ? badge('fx', `${l.fx.length} effect${l.fx.length === 1 ? '' : 's'}`) : null,
    l.masks ? badge('mask', `${l.masks} mask${l.masks === 1 ? '' : 's'}`) : null,
    l.matte ? badge('matte', 'Shown through another layer') : null,
    l.mods ? badge('mod', `${l.mods} modifier${l.mods === 1 ? '' : 's'}`) : null,
    cat ? h('span', { class: 'dot', style: `background: ${cat === 'Spelling' ? '#DC2626' : '#2563EB'}`, 'aria-label': cat === 'Spelling' ? 'Has a spelling mistake' : 'Has a grammar issue', title: cat === 'Spelling' ? 'Spelling' : 'Grammar' }) : null,
    h('button', { class: `mini${l.locked ? ' keep' : ''}`, 'aria-label': l.locked ? `Unlock ${l.name}` : `Lock ${l.name}`, title: l.locked ? 'Unlock' : 'Lock', html: icon('lock', 14), style: l.locked ? 'color: var(--ink)' : '', onClick: () => app.run('set_meta', { id: l.id, locked: !l.locked }) }),
    h('button', { class: `mini${l.visible ? '' : ' keep'}`, 'aria-label': l.visible ? `Hide ${l.name}` : `Show ${l.name}`, title: l.visible ? 'Hide' : 'Show', html: icon(l.visible ? 'eye' : 'eyeOff', 14), onClick: () => app.run('set_meta', { id: l.id, visible: !l.visible }) }));
    row.dataset.id = l.id;
    row.dataset.group = l.group ? '1' : '';
    rows.push(row);
    // Its effects, each with a switch; a click opens its settings.
    (l.fx || []).forEach((fx, k) => rows.push(h('div', { class: `lrow fxrow${fx.enabled ? '' : ' hidden'}`, style: `padding-left: ${4 + l.depth * 18 + 40}px`, title: 'Its settings are in the Layer panel',
      onClick: (e) => { if (e.target.closest('input')) return; openEffect(l.id, k); } },
    h('span', { class: 'lbadge' }, 'fx'), h('span', { class: 'lname' }, FX_NAME[fx.kind] || String(fx.kind).replace(/_/g, ' ')),
    h('input', { type: 'checkbox', checked: fx.enabled, 'aria-label': `${FX_NAME[fx.kind] || fx.kind} on`, title: fx.enabled ? 'On' : 'Off',
      onChange: (e) => { const meta = JSON.parse(JSON.stringify(call('layer_full', { id: l.id }).layer)); meta.effects[k].enabled = e.target.checked; app.run('set_meta_json', { id: l.id, meta, label: e.target.checked ? 'Effect on' : 'Effect off' }); } }))));
  }
  const sym = call('symbols');
  const symRows = sym.list.map((x) => h('div', { class: `lrow${x.open ? ' on' : ''}`, title: `${x.uses} cop${x.uses === 1 ? 'y' : 'ies'} placed` },
    h('span', { html: icon('canvas', 16), style: 'display: inline-flex' }),
    h('span', { class: 'lname', onDblclick: (e) => { const input = h('input', { value: x.name, 'aria-label': 'Symbol name' }); const done = () => app.run('rename_symbol', { id: x.id, name: input.value }); input.addEventListener('keydown', (ev) => { ev.stopPropagation(); if (ev.key === 'Enter') done(); if (ev.key === 'Escape') renderLayers(); }); input.addEventListener('blur', done); e.currentTarget.replaceWith(input); input.focus(); input.select(); } }, x.name),
    h('button', { class: 'mini keep', title: 'Place a copy on this canvas', 'aria-label': `Place ${x.name}`, html: icon('plus', 14), disabled: x.open, onClick: () => app.run('place_symbol', { id: x.id }) }),
    h('button', { class: 'mini keep', title: 'Edit its layers', 'aria-label': `Edit ${x.name}`, html: icon('pen', 14), onClick: () => editSymbol(x.id) }),
    h('button', { class: 'mini keep', title: 'Delete it and its copies', 'aria-label': `Delete ${x.name}`, html: icon('trash', 14), onClick: async () => { if (!x.uses || await ask(`Delete “${x.name}”?`, `Its ${x.uses} cop${x.uses === 1 ? 'y goes' : 'ies go'} too.`, 'Delete')) app.run('delete_symbol', { id: x.id }); } })));
  const diagram = st.comp.kind === 'diagram';
  put($('layers'),
    shapesShown() ? shapesPanel(app, h, { toast, close: () => showShapes(false) }) : null,
    !diagram || st.presentation ? h('div', { class: 'slides-list' }, slides.panel()) : null,
    st.symbol ? h('div', { class: 'symbol-banner' }, h('span', {}, 'Editing the symbol ', h('strong', {}, st.symbol)), h('button', { class: 'btn main', style: 'height: 30px', onClick: () => finishSymbol() }, 'Done')) : null,
    h('div', { class: 'panel-head' }, h('h2', {}, 'Layers'), add),
    h('div', { role: 'listbox', 'aria-label': 'Layers', 'aria-multiselectable': 'true', style: 'display: flex; flex-direction: column; gap: 2px' },
      rows.length ? layerDrag(rows) : h('div', { class: 'empty' }, 'No layers yet.')),
    h('div', { class: 'panel-head symbols-list', style: 'margin-top: 14px' }, h('h2', {}, 'Symbols'),
      h('button', { class: 'icon-btn small', 'aria-label': 'New symbol', title: st.selection.length ? 'Turn the selected layers into a symbol' : 'A new symbol: empty, or a diagram', html: icon('plus'), onClick: (e) => (st.selection.length ? (app.run('new_symbol') && editSymbolRefresh())
        : popup(e.currentTarget, [item('Empty symbol', { run: () => app.run('new_symbol') && editSymbolRefresh() }), item('Diagram symbol', { run: () => newDiagramSymbol() })])) })),
    symRows.length ? h('div', { class: 'symbols-list', style: 'display: flex; flex-direction: column; gap: 2px' }, symRows)
      : h('div', { class: 'empty symbols-list', style: 'font-size: 13px' }, 'Reusable drawings: select layers and press + to make one; every copy changes when you edit it.'));
  layerThumbs();
}

/// A click in the Layers list: just it; Ctrl: add or take it away; Shift: everything from the last one clicked.
function pickLayer(id, e, shown) {
  const st = app.state;
  if (e.shiftKey && app.layerAnchor != null) {
    const order = shown.map((l) => l.id);
    const [a, b] = [order.indexOf(app.layerAnchor), order.indexOf(id)];
    if (a >= 0 && b >= 0) { app.select(order.slice(Math.min(a, b), Math.max(a, b) + 1)); return; }
  }
  app.layerAnchor = id;
  if (e.ctrlKey || e.metaKey) app.select(st.selection.includes(id) ? st.selection.filter((x) => x !== id) : [...st.selection, id]);
  else app.select([id]);
}

/// Opens effect `k` of layer `id` in the Layer panel.
function openEffect(id, k) {
  app.select([id]);
  app.side = 'layer';
  app.openSections.effects = true;
  app.renderSide();
  requestAnimationFrame(() => document.querySelectorAll('.sect').forEach((d) => { if (d.querySelector('summary')?.textContent.startsWith('Effects')) { d.open = true; d.querySelectorAll('.item-card')[k]?.scrollIntoView({ block: 'center' }); } }));
}

/// Drag rows to restack: above or below another layer, or into a group (its middle).
function layerDrag(rows) {
  for (const row of rows) {
    if (!row.dataset.id) continue;
    row.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || e.target.closest('.mini, .lcaret, input')) return;
      const id = Number(row.dataset.id);
      const y0 = e.clientY;
      app.layerDragged = false;
      let drop = null;
      const line = h('div', { class: 'drop-line' });
      const move = (ev) => {
        if (!app.layerDragged && Math.abs(ev.clientY - y0) < 6) return;
        app.layerDragged = true;
        row.classList.add('dragging');
        document.querySelectorAll('.lrow.drop-into').forEach((x) => x.classList.remove('drop-into'));
        const target = [...row.parentElement.querySelectorAll('.lrow[data-id]')].find((r) => { const b = r.getBoundingClientRect(); return ev.clientY >= b.top && ev.clientY < b.bottom; });
        line.remove();
        if (!target || target === row) { drop = null; return; }
        const b = target.getBoundingClientRect(), t = (ev.clientY - b.top) / b.height;
        const into = target.dataset.group && t > 0.28 && t < 0.72;
        // The list is top first: the top half of a row is above it.
        drop = { target: Number(target.dataset.id), place: into ? 'into' : t < 0.5 ? 'above' : 'below' };
        if (into) target.classList.add('drop-into');
        else target.parentElement.insertBefore(line, t < 0.5 ? target : target.nextSibling);
      };
      const up = () => {
        window.removeEventListener('pointermove', move);
        window.removeEventListener('pointerup', up);
        line.remove();
        row.classList.remove('dragging');
        document.querySelectorAll('.lrow.drop-into').forEach((x) => x.classList.remove('drop-into'));
        if (app.layerDragged && drop) {
          const ids = app.state.selection.includes(id) ? app.state.selection : [id];
          app.run('move_layers', { ids, target: drop.target, place: drop.place });
        }
        setTimeout(() => { app.layerDragged = false; }, 0);
      };
      window.addEventListener('pointermove', move);
      window.addEventListener('pointerup', up);
    });
  }
  return rows;
}

/// Each row's little picture of its layer, drawn a few at a time once things settle.
let thumbTimer = null;
const thumbCache = new Map();
function layerThumbs() {
  clearTimeout(thumbTimer);
  const rev = app.state.revision;
  const paint = () => {
    for (const el of document.querySelectorAll('.lthumb[data-thumb]')) {
      const key = `${el.dataset.thumb}:${rev}`;
      const c = thumbCache.get(el.dataset.thumb);
      if (c?.key === key && c.url && el.querySelector('img')?.getAttribute('src') !== c.url) el.replaceChildren(h('img', { src: c.url, alt: '' }));
    }
  };
  paint();
  thumbTimer = setTimeout(() => {
    const els = [...document.querySelectorAll('.lthumb[data-thumb]')];
    let i = 0;
    const step = () => {
      if (app.state.revision !== rev || app.playing) return;
      const until = performance.now() + 8;
      while (i < els.length && performance.now() < until) {
        const el = els[i++];
        const id = el.dataset.thumb, key = `${id}:${rev}`;
        if (thumbCache.get(id)?.key === key) continue;
        let url = '';
        try { url = URL.createObjectURL(new Blob([engineRaw().layer_thumb(BigInt(id), 56)], { type: 'image/png' })); } catch { /* nothing to show */ }
        const old = thumbCache.get(id);
        if (old?.url) URL.revokeObjectURL(old.url);
        thumbCache.set(id, { key, url });
      }
      // The list may have been drawn again since: show them in the one on screen.
      paint();
      // Thumbnails draw on the canvas's renderer: the view goes back before the frame shows.
      app.viewport.drawNow();
      if (i < els.length) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }, 500);
}

/// A diagram (flowchart, network, workflow) as a symbol: it opens with the diagram tools; place it from the Symbols list.
function newDiagramSymbol() {
  const r = app.run('new_diagram_symbol');
  if (r == null) return;
  editSymbolRefresh();
  toast(`Editing “${r.name}”: place it on other canvases from the Symbols list.`);
}

app.editSymbol = (id) => editSymbol(id);
function editSymbol(id) {
  app.run('edit_symbol', { id });
  editSymbolRefresh();
}
function finishSymbol() {
  app.run('finish_symbol');
  editSymbolRefresh();
}
function editSymbolRefresh() {
  app.lastStructure = '';
  app.changed();
  app.viewport.fit();
  writing.run();
  return true;
}

function layerMenu(e, l) {
  popup(null, selectionMenu().filter((x) => x != null).map((x) => (x === '-' ? h('hr') : x)), { at: [e.clientX, e.clientY] });
  return;
  popup(null, [
    item('Rename…', { run: () => renameLayer(l.id) }),
    item('Duplicate', { run: () => duplicateSel() }),
    item(l.visible ? 'Hide' : 'Show', { run: () => app.run('set_meta', { id: l.id, visible: !l.visible }) }),
    item(l.locked ? 'Unlock' : 'Lock', { run: () => app.run('set_meta', { id: l.id, locked: !l.locked }) }),
    '-',
    item('Bring forward', { run: () => reorder(1) }),
    item('Send backward', { run: () => reorder(-1) }),
    '-',
    item('Delete', { run: () => app.run('delete') }),
  ].map((x) => (x === '-' ? h('hr') : x)), { at: [e.clientX, e.clientY] });
}

function renameLayer(id) {
  const row = document.querySelector(`.lrow[data-id="${id}"]`);
  const l = app.layerById(id);
  if (!row || !l) return;
  const input = h('input', { value: l.name, 'aria-label': 'Layer name' });
  const done = (keep) => { const v = input.value.trim(); if (keep && v && v !== l.name) app.run('set_meta', { id, name: v }); else renderLayers(); };
  input.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') done(true); if (e.key === 'Escape') done(false); });
  input.addEventListener('blur', () => done(true));
  row.querySelector('.lname').replaceWith(input);
  input.focus();
  input.select();
}

/// Moves the selected layer up (towards the front) or down in the paint order.
function reorder(dir) {
  const l = one();
  if (!l) return;
  const top = app.state.layers.filter((x) => x.depth === 0);
  // The list is front first; the engine counts from the back.
  const back = top.slice().reverse();
  const i = back.findIndex((x) => x.id === l.id);
  if (i < 0) return;
  const to = Math.max(0, Math.min(back.length - 1, i + dir));
  if (to !== i) app.run('reorder', { id: l.id, index: to });
}

// ----- Side panel: Layer and Writing -----

app.renderSide = () => {
  const n = writing.issues.length;
  const tabs = h('div', { class: 'segs', role: 'tablist', 'aria-label': 'Panel', style: 'display: flex' },
    h('button', { class: `seg${app.side === 'layer' ? ' on' : ''}`, role: 'tab', 'aria-selected': String(app.side === 'layer'), style: 'flex: 1 1 0', onClick: () => { app.side = 'layer'; app.renderSide(); } }, sel().length ? 'Layer' : 'Canvas'),
    h('button', { class: `seg${app.side === 'paint' ? ' on' : ''}`, role: 'tab', 'aria-selected': String(app.side === 'paint'), style: 'flex: 1 1 0', onClick: () => { app.side = 'paint'; app.renderSide(); } }, 'Paint'),
    h('button', { class: `seg${app.side === 'writing' ? ' on' : ''}`, role: 'tab', 'aria-selected': String(app.side === 'writing'), style: 'flex: 1 1 0', onClick: () => { app.side = 'writing'; app.renderSide(); } }, 'Writing', n ? h('span', { class: 'badge' }, String(n)) : null),
    h('button', { class: `seg${app.side === 'ai' ? ' on' : ''}`, role: 'tab', 'aria-selected': String(app.side === 'ai'), style: 'flex: 1 1 0', onClick: () => { app.side = 'ai'; app.renderSide(); } }, 'AI'));
  const body = app.side === 'writing' ? writing.panel(h) : app.side === 'paint' ? paintPanel(app, h, { ask, toast }) : app.side === 'ai' ? assistant.panel() : layerPanel();
  // Keep the focused number box focused while values update.
  const focused = document.activeElement?.dataset?.prop;
  put($('side'), tabs, body);
  if (focused) $('side').querySelector(`[data-prop="${focused}"]`)?.focus();
};

function layerPanel() {
  const st = app.state;
  if (!st) return [];
  const info = app.layerInfo;
  if (!info) return canvasPanel();
  const P = info.props;
  const out = [h('div', { class: 'props' })];
  const box = out[0];
  if (sel().length > 1) {
    box.append(h('div', { class: 'multi-note', role: 'note' }, h('strong', {}, `${sel().length} layers selected`),
      h('span', {}, `Values with a keyframe button change on all of them (“mixed”: they differ, so each moves by the same amount). Other settings: “${info.name}” only.`)));
  }
  box.append(h('label', { class: 'lab', style: 'display: flex; flex-direction: column; gap: 6px' }, 'Name',
    h('input', { class: 'field', value: info.name, onChange: (e) => app.run('set_meta', { id: info.id, name: e.target.value.trim() || info.name }), onKeydown: (e) => e.stopPropagation() })));
  // The theme role: how Slide › Apply theme colours this layer.
  if (app.state.presentation || info.role) {
    const roles = app.roles || (app.roles = call('roles'));
    box.append(h('label', { class: 'prop', title: 'How Slide › Apply theme colours this layer (None: leave it alone)' }, h('span', { class: 'pname' }, 'Theme role'),
      h('select', { class: 'field small', 'aria-label': 'Theme role', onChange: (e) => app.run('set_role', { id: info.id, role: e.target.value || null }) },
        h('option', { value: '', selected: !info.role }, 'None'), roles.map((r) => h('option', { value: r.id, selected: info.role === r.id }, r.label)))));
  }
  if (info.kind === 'text') box.append(...textSection(app, h, { info, group, propRow, colourRow, toast }));
  box.append(group('Position and size',
    propRow(info, 'position', P.position, { unit: ['X', 'Y'], step: 1 }),
    propRow(info, 'scale', P.scale, { unit: ['%', '%'], step: 1, k: 100 }),
    propRow(info, 'rotation', P.rotation, { unit: '°', step: 1 }),
    propRow(info, 'opacity', P.opacity, { unit: '%', step: 1, k: 100, min: 0, max: 100 })));
  const more = h('details', { class: 'sect', open: app.openSections.more ?? false }, h('summary', {}, h('span', {}, 'Skew and origin')),
    h('div', { class: 'sect-body' },
      propRow(info, 'skew', P.skew, { unit: '°', step: 1, label: 'Skew' }),
      propRow(info, 'skew_axis', P.skew_axis, { unit: '°', step: 1, label: 'Skew direction' }),
      propRow(info, 'anchor', P.anchor, { unit: ['X', 'Y'], step: 1, label: 'Transform origin' })));
  more.addEventListener('toggle', () => { app.openSections.more = more.open; });
  box.append(more);
  box.append(...lookSections(app, h, { propRow, keyButton, unitsFor }));
  if (st.comp.duration) {
    box.append(h('div', { class: 'empty', style: 'font-size: 13px' },
      `Click a diamond to animate a property: it keeps its value at ${fmtTime(st.time)}, and changes at other times become keyframes${app.autoKey ? '' : ' (auto keyframes are off)'}.`));
  }
  return out;
}

function canvasPanel() {
  const c = app.state.comp;
  const num = (label, value, apply, step = 1) => h('label', { class: 'lab', style: 'display: flex; flex-direction: column; gap: 6px' }, label,
    h('input', { class: 'field', type: 'number', value: String(value), step: String(step), onChange: (e) => apply(Number(e.target.value)), onKeydown: (e) => e.stopPropagation() }));
  return [h('div', { class: 'props' },
    h('div', { class: 'empty' }, 'Nothing selected. Click a layer on the canvas or in the list to change it.'),
    h('div', { class: 'muted', style: 'font-size: 13px' }, [c.name, c.duration ? `${Math.round(c.duration * c.fps)} frames` : 'A still picture', `${app.state.layers.length} layer${app.state.layers.length === 1 ? '' : 's'}`].join(' · ')),
    h('div', { class: 'grid2' },
      num(`Width (${app.units().unit})`, Math.round((c.width / app.units().k) * 100) / 100, (v) => app.run('set_comp', { width: Math.round(v * app.units().k) }), app.units().step),
      num(`Height (${app.units().unit})`, Math.round((c.height / app.units().k) * 100) / 100, (v) => app.run('set_comp', { height: Math.round(v * app.units().k) }), app.units().step),
      num('Length (s)', +c.duration.toFixed(2), (v) => app.run('set_comp', { duration: v }), 0.5),
      num('Frames a second', c.fps, (v) => app.run('set_comp', { fps: v }))),
    backgroundRow(c, (v) => app.run('set_comp', { background: v })),
    slicesList())];
}

/// Whether a canvas background ("#rrggbbaa") is see-through.
const seeThrough = (bg) => String(bg).length === 9 && parseInt(String(bg).slice(7, 9), 16) < 255;

/// The canvas's background: see-through (the default), or a colour. `set` gets "#rrggbb" or "#rrggbb00".
function backgroundRow(c, set) {
  const clear = seeThrough(c.background);
  const colour = h('input', { type: 'color', class: 'color-in', value: c.background.slice(0, 7), 'aria-label': 'Background colour', disabled: clear,
    onChange: (e) => set(e.target.value) });
  return h('div', { class: 'lab', style: 'display: flex; align-items: center; gap: 10px; flex-wrap: wrap' }, 'Background',
    h('label', { class: 'check-row', title: 'Nothing behind the drawing: PNG, WebM and GIF keep it see-through; MP4 and PDF ask for a colour when you export' },
      h('input', { type: 'checkbox', checked: clear, onChange: (e) => { set(e.target.checked ? `${c.background.slice(0, 7)}00` : c.background.slice(0, 7)); } }), h('span', {}, 'See-through')),
    colour);
}
app.seeThrough = seeThrough;

/// The canvas's slices: named regions exported as their own files.
function slicesList() {
  let list = [];
  try { list = call('slices'); } catch { /* none */ }
  return h('div', { class: 'sect-body', style: 'border-top: 1px solid var(--line); padding-top: 12px' },
    h('div', { class: 'lab' }, 'Slices'),
    list.length ? list.map((sl, i) => h('div', { class: 'prop' },
      h('input', { class: 'field small', value: sl.name, 'aria-label': `Slice ${i + 1} name`, onChange: (e) => app.run('set_slice', { index: i, name: e.target.value.trim() || sl.name }), onKeydown: (e) => e.stopPropagation() }),
      h('small', { class: 'unit' }, `${Math.round(sl.rect[2] - sl.rect[0])}×${Math.round(sl.rect[3] - sl.rect[1])}`),
      h('button', { class: 'small-btn', 'aria-label': `Remove ${sl.name}`, html: icon('trash', 14), onClick: () => app.run('remove_slice', { index: i }) })))
      : h('div', { class: 'empty', style: 'font-size: 13px' }, 'None yet. With the crop tool, drag a rectangle and press “Make a slice”.'),
    list.length ? h('button', { class: 'small-btn', onClick: () => openExport() }, 'Export…') : null);
}

function group(title, ...rows) {
  return h('div', { style: 'display: flex; flex-direction: column; gap: 6px' }, h('div', { class: 'lab' }, title), rows);
}

/// The diamond that animates a property (filled when it has keyframes; a
/// keyframe now when the playhead is on one).
function keyButton(info, prop, p, name) {
  const t = app.state.time;
  const here = p.keys?.some((k) => Math.abs(k - t) < 1e-4);
  const nm = (name || propName(prop)).toLowerCase();
  if (!p.animated) {
    const label = `Animate ${nm} (a keyframe at the playhead)`;
    return h('span', { class: 'key-nav' }, h('span', { class: 'key-step' }),
      h('button', { class: 'key-btn', 'aria-label': label, title: label, 'aria-pressed': 'false', onClick: () => app.run('toggle_anim', { id: info.id, prop }) }, h('span', { class: 'diamond' })), h('span', { class: 'key-step' }));
  }
  // Animated: previous key, a key here (add or remove), next key; right-click for easing.
  const keys = (p.keys || []).slice().sort((a, b) => a - b);
  const prev = [...keys].reverse().find((k) => k < t - 1e-4), next = keys.find((k) => k > t + 1e-4);
  const menu = (e) => {
    e.preventDefault();
    const at = here ? keys.find((k) => Math.abs(k - t) < 1e-4) : prev ?? keys[0];
    const k = { id: info.id, prop, time: at };
    popup(null, [...tl.easingItems([k]), h('hr'),
      item(`Stop animating ${nm}`, { run: () => app.run('toggle_anim', { id: info.id, prop }) })], { at: [e.clientX, e.clientY] });
  };
  const label = here ? `Remove the ${nm} keyframe here` : `Add a ${nm} keyframe here`;
  return h('span', { class: 'key-nav', onContextmenu: menu },
    h('button', { class: 'key-step', 'aria-label': `Previous ${nm} keyframe`, title: 'Previous keyframe', disabled: prev == null, html: icon('left', 12), onClick: () => app.setTime(prev) }),
    h('button', { class: 'key-btn', 'aria-label': label, title: `${label} (right-click: easing, stop animating)`, 'aria-pressed': String(!!here), onClick: () => app.run('key_here', { id: info.id, prop }) },
      h('span', { class: `diamond on${here ? '' : ' hollow'}` })),
    h('button', { class: 'key-step', 'aria-label': `Next ${nm} keyframe`, title: 'Next keyframe', disabled: next == null, html: icon('right', 12), onClick: () => app.setTime(next) }));
}

function propRow(info, prop, p, { unit, step = 1, k = 1, min, max, label } = {}) {
  if (!p) return null;
  const vals = Array.isArray(p.value) ? p.value : [p.value];
  if (vals.some((v) => typeof v !== 'number')) return null;
  const units = Array.isArray(unit) ? unit : [unit];
  const name = label || propName(prop);
  const boxes = vals.map((v, i) => numBox(info, prop, p, vals, i, { unit: units[i] || '', step, k, min, max, name }));
  return h('div', { class: 'prop' }, app.state.comp.duration ? keyButton(info, prop, p, name) : null,
    h('span', { class: `pname${p.mixed ? ' mixed' : ''}`, title: p.mixed ? `${name}: the selected layers differ. Numbers change each of them by the same amount.` : '' }, p.mixed ? `${name} (mixed)` : name), h('div', { class: 'vals' }, boxes));
}

/// Units and steps for a property, by its name.
function unitsFor(path) {
  if (/(liquid\.level)$/.test(path)) return { unit: '%', k: 100, step: 1, min: 0, max: 100 };
  if (/(zoom)$/.test(path)) return { unit: '×', step: 0.05, min: 0.01 };
  if (/(opacity|trim_|saturation|brightness|contrast|inner|hole|end_opacity|progress)$/.test(path)) return { unit: '%', k: 100, step: 1 };
  if (/(rotation|hue|angle|spin|tilt|direction|spread)$/.test(path)) return { unit: '°', step: 1 };
  if (/(offset|origin|position|gravity)$/.test(path)) return { unit: ['X', 'Y'], step: 1 };
  if (/(count|levels|ridges|detail|seed)$/.test(path)) return { unit: '', step: 1, min: 0 };
  if (/scale$/.test(path)) return { unit: ['%', '%'], k: 100, step: 1 };
  return { unit: 'px', step: 1 };
}

/// Pick a picture and fill shape `index` of layer `id` with it, repeated.
app.pickPattern = async (id, index) => {
  let got;
  try { got = await platform.openPicture(); } catch (e) { toast(String(e.message || e)); return; }
  if (!got) return;
  const pic = await readPicture(got.name, got.bytes);
  if (pic) app.run('set_pattern', { id, index, ...pic });
};

function numBox(info, prop, p, vals, i, { unit, step, k, min, max, name }) {
  const shown = (v) => String(Math.round(v * k * 100) / 100);
  const input = h('input', { type: 'text', inputmode: 'decimal', value: shown(vals[i]), 'aria-label': `${name || propName(prop)}${vals.length > 1 ? ` ${unit}` : ''}`, 'data-prop': `${prop}:${i}` });
  const send = (v) => {
    let x = v / k;
    if (min != null) x = Math.max(min / (k === 100 && max === 100 ? 100 : 1), x);
    if (max != null) x = Math.min(max / k, x);
    const next = vals.slice();
    next[i] = x;
    app.run('set_prop', { id: info.id, prop, value: vals.length > 1 ? next : next[0] });
  };
  input.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { const v = Number(input.value); if (Number.isFinite(v)) send(v); input.select(); }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') { e.preventDefault(); const v = (Number(input.value) || 0) + (e.key === 'ArrowUp' ? 1 : -1) * step * (e.shiftKey ? 10 : 1); send(v); }
  });
  input.addEventListener('change', () => { const v = Number(input.value); if (Number.isFinite(v)) send(v); });
  // Dragging the box, its unit or the property's name changes it (scrub.js).
  input.step = String(step);
  const handle = h('small', { 'aria-hidden': 'true', title: 'Drag sideways to change' }, unit);
  return h('div', { class: 'num' }, input, handle);
}

function colourRow(info, prop, p) {
  if (!p) return null;
  const input = h('input', { type: 'color', class: 'color-in', value: String(p.value).slice(0, 7), 'data-rgba': String(p.value), 'aria-label': propName(prop) });
  input.addEventListener('input', (e) => app.run('set_prop', { id: info.id, prop, value: e.target.dataset.rgba || e.target.value }, { quiet: true }));
  input.addEventListener('change', (e) => app.run('set_prop', { id: info.id, prop, value: e.target.dataset.rgba || e.target.value }));
  return h('div', { class: 'prop' }, app.state.comp.duration ? keyButton(info, prop, p) : null, h('span', { class: 'pname' }, propName(prop)), input,
    h('span', { style: 'font-size: 13px; color: var(--muted); font-variant-numeric: tabular-nums' }, String(p.value).slice(0, 7).toUpperCase()));
}

// ----- Status bar, tabs -----

app.renderStatus = () => renderStatus();
function renderStatus() {
  const st = app.state;
  const n = writing.issues.length;
  const tab = frontTab();
  const others = tab?.remote ? server.here(tab.id).length : 0;
  const saved = tab?.remote
    ? (others ? `Editing with ${others} other${others === 1 ? '' : 's'} · on ${server.name}` : `Saved on ${server.name} as you work`)
    : !st.dirty ? (tab?.path ? 'Saved' : platform.DESKTOP ? 'Not saved yet' : 'Kept in this browser') : 'Changes not saved';
  put($('status'),
    h('span', {}, `${st.layers.length} layer${st.layers.length === 1 ? '' : 's'}`),
    st.selection.length ? h('span', {}, `${st.selection.length} selected`) : null,
    h('button', { class: 'pick', style: 'height: 26px; border: 0; background: transparent; padding: 0 4px', onClick: () => showSide('writing') },
      h('span', { style: `width: 8px; height: 8px; border-radius: 50%; background: ${n ? 'var(--accent)' : '#15803D'}` }), n ? `${n} to look at` : 'Spelling and grammar look right'),
    app.hintNow ? h('span', { class: 'status-hint now', role: 'status', title: app.hintNow }, app.hintNow)
      : toolHint() ? h('span', { class: 'status-hint', title: toolHint() }, toolHint()) : null,
    h('span', { class: 'spacer' }),
    app.lastMessage ? h('button', { class: 'pick status-msg', title: 'Messages so far', onClick: (e) => messagesMenu(e) }, app.lastMessage.text) : null,
    app.lastMessage?.path ? h('button', { class: 'pick', style: 'height: 26px; border: 0; background: transparent; padding: 0 4px; color: var(--accent-ink)', onClick: () => platform.reveal(app.lastMessage.path).catch((x) => toast(String(x.message || x))) }, 'Show in folder') : null,
    h('span', {}, saved),
    h('button', { class: 'pick', style: 'height: 26px; border: 0; background: transparent; padding: 0 4px', title: 'Show frame times', onClick: () => togglePerf() }, app.renderer),
    h('span', {}, `${Math.round(app.viewport.k * 100)}%`));
}

/// The Pen and Pencil's line style: plain, or a pen (calligraphy, ink, marker…). What you draw next is in it.
function lineStyle() {
  const kinds = (app.pens ||= call('pens'));
  const now = call('new_stroke')?.style?.kind || 'solid';
  return h('label', { class: 'opt', title: 'How the line looks: what you draw next is drawn in this style (More…: its settings, your own pens)' }, h('span', {}, 'Line'),
    h('select', { class: 'field small', style: 'width: auto', 'aria-label': 'Line style', onChange: (e) => {
      const v = e.target.value;
      if (v === 'more') { e.target.value = now; pensDialog(app, h).then(() => renderToolbar()); return; }
      const ns = call('new_stroke') || {};
      call('set_new_stroke', { stroke: v === 'solid' ? (Object.keys(ns).length > 1 ? { ...ns, style: undefined } : null) : { ...ns, style: call('pen_preset', { kind: v }) } });
      // A line needs a stroke to show.
      if (!app.strokeOn) { app.strokeOn = true; app.strokeWidth = Math.max(app.strokeWidth, v === 'solid' ? 2 : 8); }
      renderToolbar();
    } },
    h('option', { value: 'solid', selected: now === 'solid' }, 'Plain'),
    kinds.filter((k) => k.kind !== 'solid').map((k) => h('option', { value: k.kind, selected: now === k.kind }, k.label)),
    h('option', { value: 'more' }, 'More…')));
}

/// A tip about the tool in hand (not an error): shown in the status bar for a few seconds.
app.hint = (text) => {
  app.hintNow = text;
  clearTimeout(app.hintTimer);
  app.hintTimer = setTimeout(() => { app.hintNow = null; if (app.state) renderStatus(); }, 6000);
  if (app.state) renderStatus();
};

/// What the tool in hand does, for the status bar.
function toolHint() {
  if (app.viewport?.editing) return 'Typing: Esc or click outside to finish';
  if (app.tool === 'select') return sel().length ? 'Drag to move, handles to size and turn · Shift: keep straight · Alt+drag: a copy · Double-click: edit' : 'Click a layer to select it · drag on empty canvas to select several';
  if (app.tool === 'shape') return `${(SHAPES.find((x) => x.id === app.shape)?.label || 'Shape').split(':')[0]}: drag to draw · Shift: even sides · Alt: from the centre`;
  if (app.tool?.startsWith('r-')) return app.raster.hint(app.tool);
  const t = TOOLS.find((x) => x.id === app.tool);
  const label = t?.label || '';
  return label.includes(':') ? label : label ? `${label}` : '';
}

/// The messages so far (newest first), from the status bar.
function messagesMenu(e) {
  popup(e.currentTarget, (app.messages || []).map((m) => item(m.text, { keys: m.at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }), run: () => { if (m.path) platform.reveal(m.path).catch(() => {}); } })));
}

app.onZoom = () => { if (app.state) renderStatus(); renderZoom(); };

/// The zoom in the canvas's corner: − / the zoom (a menu of sizes) / +.
function renderZoom() {
  const k = app.viewport?.k;
  if (!k) return;
  const z = $('zoom');
  if (!z.firstChild) {
    z.append(
      h('button', { class: 'pick', 'aria-label': 'Zoom out', title: 'Zoom out (Ctrl+-)', html: icon('minus', 14), onClick: () => app.viewport.zoomTo(app.viewport.k / 1.25) }),
      h('button', { class: 'pick zoom-level', 'aria-haspopup': 'menu', title: 'Zoom to a size', onClick: (e) => popup(e.currentTarget, [
        item('Fit on screen', { keys: 'Ctrl+0', run: () => app.viewport.fit() }),
        ...[25, 50, 100, 200, 400].map((p) => item(`${p}%`, { keys: p === 100 ? 'Ctrl+1' : null, checked: Math.abs(app.viewport.k * 100 - p) < 0.5, run: () => app.viewport.zoomTo(p / 100) })),
      ]) }),
      h('button', { class: 'pick', 'aria-label': 'Zoom in', title: 'Zoom in (Ctrl+=)', html: icon('plus', 14), onClick: () => app.viewport.zoomTo(app.viewport.k * 1.25) }),
      h('button', { class: 'pick', title: 'Show the whole canvas (Ctrl+0 or Shift+1)', onClick: () => app.viewport.fit() }, 'Fit'));
  }
  z.querySelector('.zoom-level').textContent = `${Math.round(k * 100)}%`;
}
app.onIssues = () => { renderLayers(); if (app.side === 'writing') app.renderSide(); else app.renderSide(); renderStatus(); };

function frontTab() {
  return app.tabs.find((t) => t.id === engineTabs.front());
}

function renderTabs() {
  const front = engineTabs.front();
  // On a server's site one file is open at a time, like a page (File › New adds tabs).
  document.querySelector('.tabstrip').hidden = servedHere && app.tabs.length <= 1;
  // What's open, to open again next time (Settings › Reopen…).
  if (platform.DESKTOP && app.ready) try { localStorage.setItem('draw.lastOpen', JSON.stringify(app.tabs.filter((t) => t.path).map((t) => ({ name: t.title, path: t.path })))); } catch { /* not kept */ }
  syncAddress();
  const title = frontTab()?.title || 'Untitled';
  document.title = `${app.state?.dirty ? '• ' : ''}${title} — Xenify Draw`;
  // While the New canvas screen is open it's a tab of its own (the untouched empty canvas behind it isn't shown),
  // so the canvases you were on are a click away.
  const shown = newScreen ? app.tabs.filter((t) => !untouched(t)) : app.tabs;
  const newTabEl = newScreen ? h('div', { class: 'tab', role: 'tab', 'aria-selected': 'true', title: 'New canvas' },
    h('span', { html: icon('plus', 16), style: 'display: inline-flex; color: var(--accent)' }),
    h('span', { class: 'name' }, 'New canvas'),
    h('button', { class: 'close', 'aria-label': 'Close New canvas', html: icon('x', 14), onClick: () => hideNew() })) : null;
  put($('tabs'), [...shown.map((t) => {
    const on = t.id === front && !newScreen;
    return h('div', { class: 'tab', role: 'tab', 'aria-selected': String(on), title: t.path || t.title, 'data-tab': String(t.id),
      onPointerdown: (e) => { if (e.button === 0 && !e.target.closest('.close')) { hideNew(); switchTab(t.id); dragTab(e, t); } if (e.button === 1) closeTab(t.id); },
      onContextmenu: (e) => { e.preventDefault(); tabMenu(e, t); } },
    on ? h('span', { html: icon('brush', 16), style: 'display: inline-flex; color: var(--accent)' }) : null,
    h('span', { class: 'name' }, t.title),
    t.remote ? h('span', { class: 'tab-server', html: icon('server', 14), title: `On ${server.name}`, 'aria-label': `On ${server.name}` }) : null,
    t.remote ? serverFaces(server.here(t.id)) : null,
    t.dirty && !on && !t.remote ? h('span', { class: 'dot', 'aria-label': 'Unsaved changes' }) : null,
    h('button', { class: 'close', 'aria-label': `Close ${t.title}`, html: icon('x', 14), onClick: () => closeTab(t.id) }));
  }), newTabEl].filter(Boolean));
}

/// Dragging a tab along the strip puts it somewhere else.
function dragTab(e, t) {
  const x0 = e.clientX;
  let moved = false;
  const move = (ev) => {
    if (!moved && Math.abs(ev.clientX - x0) < 6) return;
    moved = true;
    const over = [...document.querySelectorAll('#tabs .tab')].find((el) => { const b = el.getBoundingClientRect(); return ev.clientX >= b.left && ev.clientX < b.right; });
    const to = over ? app.tabs.findIndex((x) => String(x.id) === over.dataset.tab) : -1;
    const from = app.tabs.indexOf(t);
    if (to >= 0 && to !== from) { app.tabs.splice(from, 1); app.tabs.splice(to, 0, t); renderTabs(); }
  };
  const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

function tabMenu(e, t) {
  const i = app.tabs.indexOf(t);
  const closeAll = async (list) => { for (const x of list) { if (!(await closeTab(x.id))) return; } };
  popup(null, [
    item('Close', { run: () => closeTab(t.id) }),
    item('Close the others', { enabled: app.tabs.length > 1, run: () => closeAll(app.tabs.filter((x) => x !== t)) }),
    item('Close the ones to the right', { enabled: i < app.tabs.length - 1, run: () => closeAll(app.tabs.slice(i + 1)) }),
    item('Close the saved ones', { enabled: app.tabs.some((x) => !x.dirty), run: () => closeAll(app.tabs.filter((x) => !x.dirty)) }),
    h('hr'),
    item('Copy where it’s saved', { enabled: !!t.path, run: () => { navigator.clipboard?.writeText(t.path).then(() => toast('Copied the file’s path.'), () => toast(t.path)); } }),
    platform.DESKTOP ? item('Show in folder', { enabled: !!t.path, run: () => platform.reveal(t.path).catch((x) => toast(String(x.message || x))) }) : null,
  ].filter(Boolean), { at: [e.clientX, e.clientY] });
}

/// Ctrl+Tab / Ctrl+Shift+Tab: the next or previous tab.
function cycleTab(step) {
  if (app.tabs.length < 2) return;
  const i = app.tabs.findIndex((x) => x.id === engineTabs.front());
  switchTab(app.tabs[(i + step + app.tabs.length) % app.tabs.length].id);
}
app.cycleTab = cycleTab;

/// Asks what to do with a canvas's unsaved changes: 'save', 'discard', or null (cancel).
function unsavedPrompt(t) {
  return modal(`Save the changes to “${t.title}”?`, h('p', {}, 'If you don’t, the changes since it was last saved are lost.'),
    [['Cancel', null], ['Don’t save', 'discard'], ['Save', 'save', true]]);
}

/// Saves a tab (bringing it to the front first). Whether it's saved now.
async function saveTab(t) {
  if (engineTabs.front() !== t.id) switchTab(t.id);
  await save();
  return !frontTab()?.dirty && !app.state.dirty;
}

function switchTab(id) {
  if (app.viewport.editing) app.viewport.endEdit();
  engineTabs.switch(id);
  app.playing = false;
  app.lastStructure = '';
  app.changed();
  app.viewport.fit();
  writing.run();
}

/// Closes a tab, asking about unsaved changes first. Whether it closed.
async function closeTab(id) {
  const t = app.tabs.find((x) => x.id === id);
  if (t && isDirty(t)) {
    const what = await unsavedPrompt(t);
    if (!what || (what === 'save' && !(await saveTab(t)))) return false;
  }
  // On a server: what was changed in the last moments goes up first.
  if (t?.remote) await server.versionNow(id);
  server.leave(id);
  app.tabs = app.tabs.filter((x) => x.id !== id);
  if (t) platform.autosave.clear(keyOf(t)).catch(() => {});
  // The last one closed on a server's site: back to Files.
  if (servedHere && !app.tabs.length) { await quit(); return true; }
  const front = engineTabs.close(id);
  if (!app.tabs.some((x) => x.id === front)) {
    app.tabs.push({ id: front, title: 'Untitled', path: null, dirty: false });
    showNew();
  }
  switchTab(front);
  return true;
}

/// Whether a tab has changes that aren't saved (the front one's from the engine).
function isDirty(t) {
  // Canvases on a server are saved there as you work.
  if (t.remote) return false;
  return t.id === engineTabs.front() ? !!app.state?.dirty : !!t.dirty;
}

/// Opens a canvas in a new tab (or in this one, when it's untouched and empty).
/// The front tab is an untouched empty canvas (nothing but empty layers): the next canvas replaces it.
function untouched(t) {
  return !!t && t.id === engineTabs.front() && !t.path && !app.state?.dirty && !app.state?.undo && (app.state?.layers || []).every((l) => l.folder && !l.members);
}

// ----- A Xenify server (Settings › Server) -----

const server = new Server({
  call: (name, args) => call(name, args && 'id' in args ? { ...args, tab: args.id } : args),
  apply: async (name, args) => {
    if (name === 'remote_open') {
      // Opening one: a tab of its own. Putting this canvas there: this tab.
      hideNew();
      if (args.data || args.blank) newTab(args.name, () => { if (args.blank) call('new', { name: args.name }); call('remote_open', args); });
      else {
        call('remote_open', args);
        // Its copy kept from before isn't offered back after a reload
        // (it's on the server now).
        clearTimeout(saveTimer);
        clearTimeout(deskTimer);
        if (platform.DESKTOP) platform.autosave.clear(keyOf(frontTab())).catch(() => {});
        else try { localStorage.removeItem('draw.autosave'); } catch { /* */ }
      }
      Object.assign(frontTab(), { remote: true, title: args.name, path: null });
      app.changed();
      return { id: engineTabs.front() };
    }
    const out = call(name, 'id' in args ? { ...args, tab: args.id } : args);
    app.changed();
    return out;
  },
  live,
  media,
  app: 'draw',
  ext: 'xdraw',
  onChange: () => { renderTabs(); refreshAiServer(); offerUnsent(); platform.serverWords(server); },
  onError: (e) => toast(String(e.message || e)),
  // Each saved version (and, while editing together, every little while)
  // puts a picture of the canvas on the server, for
  // documents that show it (Write's Insert › Picture from a server).
  afterSave: async (id) => {
    if (id !== undefined && id !== engineTabs.front()) return;
    const c = app.state.comp;
    // The first frame: what the canvas is, wherever the playhead is.
    const png = await renderPng(app, 0, Math.min(1, 1600 / Math.max(c.width, c.height)));
    await server.savePreview(engineTabs.front(), png);
  },
});
serverUi.server = server;
// Tests (XENIFY_DRAW_TEST) reach it through the app.
app.server = server;
app.serverScreens = { openFromServer, saveToServer, openSettings: () => openSettings('server') };
serverUi.env = {
  h, toast,
  modalShell: serverModal,
  showSettings: () => openSettings('server'),
  refresh: async () => app.changed(),
  // An earlier version, as a new canvas on this computer.
  openCopy: async (name, copy) => openText({ name, text: copy.text }),
};

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
  const unkey = dialogKeys(back, close);
  back.addEventListener('keydown', (e) => e.stopPropagation());
  document.body.append(back);
  return { close, body: box };
}

// ----- Where the others editing a server canvas are (the layers they selected) -----

/// A person's colour (the same for their face and their outlines).
function colourOf(name) {
  const colours = ['#0F766E', '#1D4ED8', '#7C3AED', '#BE185D', '#B45309'];
  return colours[[...name].reduce((a, c) => a + c.charCodeAt(0), 0) % colours.length];
}

let lastSaid = '';
let sayTimer = null;
function sayWhere() {
  const id = engineTabs.front();
  if (!server.docs.get(id)?.joined) return;
  const now = JSON.stringify([id, app.state?.selection]);
  if (now === lastSaid) return;
  lastSaid = now;
  // At most every 120 ms, the latest selection.
  if (sayTimer) return;
  sayTimer = setTimeout(() => {
    sayTimer = null;
    try { server.say(id, call('live_say').data); } catch { /* not on a server any more */ }
  }, 120);
}

function placeOthers() {
  app.others = server.saidOf(engineTabs.front()).map((s) => {
    try { return { name: s.who.name, colour: colourOf(s.who.name), ids: call('live_heard', { data: s.data }).ids || [], rev: -1 }; } catch { return null; }
  }).filter(Boolean);
  app.viewport.redraw();
}
server.onSaid = (id, why) => {
  if (id !== engineTabs.front()) return;
  // Someone new: they need to know what's selected here too.
  if (why === 'arrived') { lastSaid = ''; sayWhere(); }
  placeOthers();
};

// ----- Pictures of documents on a server (kept up to date) -----

const sid = () => server.id.replace(/[^A-Za-z0-9_]/g, '');
const linkSeen = new Map(); // node -> the version shown
const b64bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
function toB64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

async function pictureSize(b64) {
  const bmp = await createImageBitmap(new Blob([b64bytes(b64)], { type: 'image/png' }));
  const size = { width: bmp.width, height: bmp.height };
  bmp.close();
  return size;
}

async function insertServerPicture() {
  const n = await pickFile('Picture from a server', 'write');
  if (!n) return;
  try {
    const p = await server.preview(n.id);
    if (!p) { toast(`“${n.details?.name || 'That document'}” has no picture yet: open it in Write and save it once.`); return; }
    linkSeen.set(n.id, p.version);
    app.run('link_insert', { server: sid(), node: n.id, data: p.png, ...(await pictureSize(p.png)) });
    app.setTool('select');
    toast('Inserted: it updates when the document is saved in Write');
  } catch (e) { toast(String(e.message || e)); }
}

/// Fetches pictures of documents that changed since they were shown.
let refreshingLinks = false;
async function refreshServerPictures() {
  if (refreshingLinks || !server.ready || !server.id) return;
  refreshingLinks = true;
  try {
    for (const l of call('links')) {
      if (l.server !== sid()) continue;
      const n = await server.api('GET', `/v1/nodes/${l.node}`).catch(() => null);
      if (!n || linkSeen.get(l.node) === (n.pictured || n.version)) continue;
      const p = await server.preview(l.node).catch(() => null);
      if (!p) continue;
      linkSeen.set(l.node, p.version);
      app.run('link_update', { server: sid(), node: l.node, data: p.png, ...(await pictureSize(p.png)) });
    }
  } finally { refreshingLinks = false; }
}
window.addEventListener('focus', () => refreshServerPictures());
setInterval(() => { if (document.visibilityState === 'visible') refreshServerPictures(); }, 20000);

/// Who else has a server canvas open: their initials, in their colours.
function serverFaces(people) {
  if (!people.length) return null;
  const names = people.map((p) => p.name).join(', ');
  return h('span', { class: 'faces', 'aria-label': `Here now: ${names}`, title: `Here now: ${names}` },
    people.slice(0, 4).map((p) => h('span', { class: 'face', style: `background: ${colourOf(p.name)}` },
      p.name.split(/\s+/).slice(0, 2).map((w) => w[0]?.toUpperCase()).join(''))));
}

function newTab(title, setup, path = null) {
  const cur = frontTab();
  const reuse = untouched(cur);
  const id = reuse ? cur.id : engineTabs.open();
  if (!reuse) app.tabs.push({ id, title, path, dirty: false });
  else Object.assign(cur, { title, path, dirty: false });
  try {
    setup();
  } catch (e) {
    toast(String(e.message || e));
  }
  app.lastStructure = '';
  call('select', { ids: [] });
  app.changed();
  flatten3d();
  app.viewport.fit();
  writing.run();
  loadFonts();
}

/// Loads the fonts the canvas uses that aren't in the engine yet (from the computer), then redraws.
async function loadFonts() {
  try { if (await fonts.ensureUsed(call('fonts_used'))) app.changed(); } catch { /* shown in Inter */ }
}

// ----- Files -----

let newScreen = null;
function showNew() {
  if (newScreen) return;
  // It can always be closed: there's always a canvas behind it (an empty one at first).
  const close = () => hideNew();
  const back = frontTab();
  newScreen = newCanvasScreen({
    h, recent: recentList(), backTo: untouched(back) ? null : back?.title,
    onCreate: async (o) => {
      hideNew();
      const { clip, ai, ...settings } = o;
      newTab(o.name, () => call('new', settings));
      if (settings.template || settings.deck) slides.refresh?.();
      // What was copied, in the middle of the new canvas.
      if (clip) await addPicture(clip.name, clip.bytes);
      if (ai) { showSide('ai'); assistant.send(ai.description, ai.kind === 'deck' ? 'new_deck' : 'new_animation'); }
    },
    readClipboard,
    settings: { units: loadSettings().units, printDpi: loadSettings().printDpi, lookAtClipboard: platform.DESKTOP },
    onOpen: () => openCanvas(),
    onRecent: (r) => openRecent(r),
    onClose: close,
    serverTemplates: server.ready ? () => server.templates() : null,
    onServerTemplate: async (n) => {
      try {
        const copy = await server.copyOf(n.id);
        hideNew();
        // A new canvas (not in Recent: the template stays on the server).
        newTab(n.details?.name || 'Untitled', () => call('open', { json: copy.text }));
      } catch (e) { toast(String(e.message || e)); }
    },
    examples: call('samples'),
    onExample: (name) => openExample(name),
  });
  document.body.append(newScreen);
  renderTabs();
  newScreen.querySelector('.nc-card.on')?.focus();
}

/// Opened from the website (?example=…, not on a Xenify server or the desktop):
/// File › Back to the website, and a bar that says so.
const fromSite = () => !platform.DESKTOP && !servedHere && new URLSearchParams(location.search).has('example');

/// Back to the page that linked here (or the site's Draw page): File ›
/// Back to the website in the browser on the website.
function backToSite() {
  let same = false;
  try { same = !!document.referrer && new URL(document.referrer).origin === location.origin && !new URL(document.referrer).pathname.startsWith('/draw/'); } catch { /* */ }
  if (same && history.length > 1) history.back();
  else location.href = '/products/draw/';
}

/// The bar at the top for visitors from the website: where they are, and the way back.
function siteBar() {
  if (!fromSite() || document.getElementById('site-bar')) return;
  const bar = document.createElement('div');
  bar.id = 'site-bar';
  bar.setAttribute('role', 'status');
  bar.style.cssText = 'position:fixed;left:50%;bottom:18px;transform:translateX(-50%);z-index:50;display:flex;align-items:center;gap:12px;padding:8px 8px 8px 16px;border-radius:999px;background:var(--panel,#fff);box-shadow:0 8px 28px rgba(0,0,0,.18);font-size:14px';
  const text = document.createElement('span');
  text.textContent = 'This is Xenify Draw, with the example open: look around and change anything.';
  const back = document.createElement('button');
  back.className = 'btn main';
  back.textContent = 'Back to the website';
  back.onclick = () => backToSite();
  bar.append(text, back);
  document.body.append(bar);
}

/// A built-in example as a new canvas (the New canvas screen's Examples, or
/// ?example=<name> on the site); `present`: a presentation starts playing.
/// From the website, stopping the presentation goes back to the page that
/// linked here (Open it in Draw is the way to look at how it's made).
async function openExample(name, present = false) {
  const title = call('samples').find((s) => s.name === name)?.title;
  if (!title) return toast('There’s no example by that name.');
  hideNew();
  newTab(title, () => call('sample', { name }));
  slides.refresh?.();
  await loadFonts();
  if (present && call('slides').presentation) {
    if (fromSite()) app.afterPresenting = () => backToSite();
    slides.present(0);
  } else siteBar();
}

/// Closes the New canvas screen (back to the canvas behind it).
function hideNew() {
  if (!newScreen) return;
  newScreen.remove();
  newScreen = null;
  renderTabs();
  app.viewport.fit();
}

/// A picture on the clipboard: { name, bytes, width, height }, or null (and says why).
async function readClipboard({ quiet = false } = {}) {
  try {
    // The desktop reads the system clipboard itself (the web view only would after a paste).
    if (platform.DESKTOP) {
      const got = await platform.clipboardPicture();
      if (got) return got;
      if (!quiet) toast('There’s no picture on the clipboard. Copy one, then try again.');
      return null;
    }
    for (const it of await navigator.clipboard.read()) {
      const type = it.types.find((t) => t.startsWith('image/'));
      if (!type) continue;
      const bytes = new Uint8Array(await (await it.getType(type)).arrayBuffer());
      const bmp = await createImageBitmap(new Blob([bytes], { type }));
      const got = { name: 'Pasted picture.png', bytes, width: bmp.width, height: bmp.height };
      bmp.close();
      return got;
    }
    if (!quiet) toast('There’s no picture on the clipboard. Copy one, then try again.');
  } catch (e) { if (!quiet) toast(`The clipboard couldn’t be read here: ${e.message || e}`); }
  return null;
}

async function openCanvas() {
  let got;
  try { got = await platform.openCanvas(); } catch (e) { toast(String(e.message || e)); return; }
  if (!got) return;
  openText(got);
}

/// A chat's whiteboard, from Xenify Chat's "Open in Draw" (a small file with the
/// server, the board and its title): opened from the server and edited live with
/// everyone in the chat.
async function openBoard(got) {
  let b;
  try { b = JSON.parse(got.text); } catch { return toast('That board link is damaged.'); }
  server.boards = { ...server.boards, [b.node]: { title: b.title } };
  try {
    if (server.url !== String(b.server).replace(/\/$/, '')) await server.connect(b.server);
    if (!server.session) {
      openSettings('server');
      return toast(`Sign in to ${server.name}, then press Open in Draw in the chat again.`);
    }
    hideNew();
    await server.open(b.node);
    (await import('./embed.js')).boardPage(app, b.title);
    renderTabs();
    app.viewport?.fit();
    toast(`${b.title}: drawing together with the chat`);
  } catch (e) { toast(`The board didn't open: ${e.message || e}`); }
}

/// Opens a canvas: a file's `bytes`, or a copy's `text` (from the server).
function openText(got) {
  hideNew();
  newTab(got.name, () => (got.bytes ? files.open(got.bytes) : call('open', { json: got.text })), got.path);
  remember(got);
}

/// A file's only canvas, still "Untitled", takes the file's name when it's
/// saved under one (a presentation's slides keep theirs). True if renamed.
function nameCanvas(name) {
  const st = app.state;
  if (!name || !st?.comp || st.presentation || st.symbol || st.comp.name !== 'Untitled' || name === 'Untitled') return false;
  app.run('set_comp', { name });
  return true;
}

async function save(as = false) {
  const tab = frontTab();
  if (!tab) return;
  // On a server it saves itself as you type: Save sends what's waiting
  // (no new version each time; a checkpoint keeps a moment).
  if (tab.remote && !as) {
    try { await server.versionNow(tab.id); toast(`Saved on ${server.name}: it saves itself as you type. File › Save a checkpoint… keeps it as it is now.`); } catch (e) { toast(String(e.message || e)); }
    return;
  }
  // On a server's site a new canvas is saved there (named now, where New was asked for).
  if (servedHere && server.session && !as) { await saveToServer(tab.title, { beforeSave: nameCanvas }); return; }
  let bytes;
  try { bytes = await canvasFile(); } catch (e) { toast(String(e.message || e)); return; }
  const name = `${tab.title.replace(/[\\/:*?"<>|]/g, '-')}.xdraw`;
  let where;
  try { where = await platform.saveFile({ name, bytes, path: as ? null : tab.path, type: 'application/zip' }); } catch (e) { toast(String(e.message || e)); return; }
  if (!where) { call('state'); return; }
  if (typeof where === 'string') {
    tab.path = where;
    tab.title = where.split(/[\\/]/).pop().replace(/\.(xdraw|xenify)$/i, '');
    // Named now: the canvas too, saved again at once so nothing shows as unsaved.
    if (nameCanvas(tab.title)) return save(false);
  }
  remember({ name: tab.title, bytes, path: tab.path });
  app.changed();
  // What's kept for a reload (or recovery) now has nothing unsaved.
  autosave();
  toast(platform.DESKTOP ? `Saved ${tab.title}` : `Downloaded ${name}`);
}

/// The canvas as an `.xdraw` file, with a picture of it (a Write document
/// shows the canvas and follows it as it changes) and whether something in
/// it moves (Write offers to play it; Draw makes the clip then).
async function canvasFile() {
  let png;
  try {
    const c = app.state.comp;
    // The first frame: what the canvas is, wherever the playhead is.
    png = await renderPng(app, 0, Math.min(1, 1600 / Math.max(c.width, c.height)));
  } catch { /* saved without one */ }
  const animated = (app.state.layers || []).some((l) => l.keys?.length || l.kind === 'video');
  return files.save(png, animated);
}

async function insertPicture() {
  let got;
  try { got = await platform.openPicture(); } catch (e) { toast(String(e.message || e)); return; }
  if (!got) return;
  await addPicture(got.name, got.bytes);
}

/// A picture file's type, size and bytes (base64), or null (with a message) when it isn't one.
async function readPicture(name, bytes) {
  const mime = sniff(bytes);
  if (!mime) { toast(`${name} isn’t a picture Draw can show (PNG, JPEG, WebP, GIF or BMP).`); return null; }
  let size;
  try {
    const bmp = await createImageBitmap(new Blob([bytes], { type: mime }));
    size = [bmp.width, bmp.height];
    bmp.close();
  } catch { toast(`${name} couldn’t be read.`); return null; }
  // Bigger than Settings › Largest picture side: kept smaller (as PNG).
  const max = app.settings?.maxImageSide || 4096;
  // Bigger than that, or BMP (kept as PNG): drawn again.
  if ((Math.max(size[0], size[1]) > max || mime === 'image/bmp') && mime !== 'image/gif') {
    const k = Math.min(1, max / Math.max(size[0], size[1]));
    const c = document.createElement('canvas');
    c.width = Math.round(size[0] * k); c.height = Math.round(size[1] * k);
    const bmp = await createImageBitmap(new Blob([bytes], { type: mime }));
    c.getContext('2d').drawImage(bmp, 0, 0, c.width, c.height);
    bmp.close();
    const url = c.toDataURL('image/png');
    return { name, mime: 'image/png', width: c.width, height: c.height, base64: url.split(',')[1] };
  }
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return { name, mime, width: size[0], height: size[1], base64: btoa(s) };
}

/// Pictures dropped on the window or pasted.
async function addPicture(name, bytes) {
  const pic = await readPicture(name, bytes);
  if (!pic) return;
  app.run('add_picture', pic);
  app.setTool('select');
}

function sniff(b) {
  if (b[0] === 0x42 && b[1] === 0x4d) return 'image/bmp';
  if (b[0] === 0x89 && b[1] === 0x50) return 'image/png';
  if (b[0] === 0xff && b[1] === 0xd8) return 'image/jpeg';
  if (b[0] === 0x47 && b[1] === 0x49) return 'image/gif';
  if (b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return 'image/webp';
  return null;
}

window.addEventListener('dragover', (e) => { if ([...(e.dataTransfer?.items || [])].some((i) => i.kind === 'file')) e.preventDefault(); });
window.addEventListener('drop', async (e) => {
  const files = [...(e.dataTransfer?.files || [])];
  if (!files.length) return;
  e.preventDefault();
  for (const f of files) {
    if (/\.(xdraw|xenify)$/i.test(f.name)) openText({ name: f.name.replace(/\.(xdraw|xenify)$/i, ''), bytes: new Uint8Array(await f.arrayBuffer()), path: null });
    else await importBytes(f.name, new Uint8Array(await f.arrayBuffer()));
  }
});
window.addEventListener('paste', async (e) => {
  if (isTyping(e) || newScreen) return;
  if (tl.hovered && app.keyClip?.length) { e.preventDefault(); tl.pasteKeys(); return; }
  const text = e.clipboardData?.getData('text/plain');
  if (text && text.includes('xenify_draw_layers')) { e.preventDefault(); pasteLayers(text); return; }
  // Pixels copied from a raster layer: back as a new layer where they were.
  if (rasterMode() && !text && app.raster.pastePixels([...(e.clipboardData?.files || [])])) { e.preventDefault(); return; }
  // SVG copied from another program (as text) becomes shapes.
  const t0 = (text || '').trimStart();
  if (t0.startsWith('<svg') || (t0.startsWith('<?xml') && t0.includes('<svg'))) {
    e.preventDefault();
    await importBytes('Pasted SVG.svg', new TextEncoder().encode(text));
    return;
  }
  // A picture copied in a browser (GIPHY and the like), or a link to one: an animated GIF is fetched from
  // where it's from, as the pixels copied with it are only its first frame.
  const files = [...(e.clipboardData?.files || [])];
  const url = pictureUrl(e.clipboardData?.getData('text/html') || '', t0);
  if (url && (!files.length || /\.gif|giphy\.com/i.test(url))) {
    e.preventDefault();
    if (await pasteFromWeb(url)) return;
    if (!files.length) return;
  }
  // On the desktop the web view may not see a picture another program copied: the system clipboard is asked.
  if (!files.length && !text && platform.DESKTOP) {
    e.preventDefault();
    try { const got = await platform.clipboardPicture(); if (got) { await importBytes(got.name, got.bytes); return; } } catch (err) { toast(`The clipboard couldn’t be read: ${err.message || err}`); return; }
    toast('There’s nothing on the clipboard that can be pasted here.');
    return;
  }
  // Files copied in a file manager: pictures, SVG, sound, video…
  if (files.length) {
    e.preventDefault();
    for (const f of files) await importBytes(f.name || (f.type === 'image/svg+xml' ? 'Pasted.svg' : 'Pasted picture.png'), new Uint8Array(await f.arrayBuffer()));
    return;
  }
  const uris = (e.clipboardData?.getData('text/uri-list') || (text && /^file:\/\//.test(t0) ? text : '')).split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith('file://'));
  if (uris.length && platform.DESKTOP) {
    e.preventDefault();
    for (const u of uris) {
      const path = decodeURIComponent(u.replace(/^file:\/\/(localhost)?/, ''));
      try { await importBytes(path.split('/').pop(), await platform.readBytes(path), path); } catch (err) { toast(`Couldn’t read ${path}: ${err.message || err}`); }
    }
  }
});

/// Where a copied picture is from: the copied HTML's first picture, else a pasted link to a picture
/// (a GIPHY page's link means its GIF).
function pictureUrl(html, text) {
  const src = html.match(/<img[^>]+src=["']([^"']+)["']/i)?.[1]?.replace(/&amp;/g, '&');
  if (src && /^https:\/\//.test(src)) return giphyGif(src);
  if (!/^https:\/\/\S+$/.test(text || '')) return null;
  const page = text.match(/^https:\/\/giphy\.com\/(?:gifs|stickers)\/(?:[^/?#]*-)?([A-Za-z0-9]+)(?:[/?#]|$)/);
  if (page) return `https://media.giphy.com/media/${page[1]}/giphy.gif`;
  return /\.(gif|png|jpe?g|webp)(\?|#|$)/i.test(text) || /giphy\.com\/media\//.test(text) ? giphyGif(text) : null;
}
/// GIPHY's WebP and MP4 versions of a GIF: the GIF itself.
function giphyGif(url) {
  const m = url.match(/^https:\/\/(?:media\d*|i)\.giphy\.com\/media\/(?:v1\.[^/]+\/)?([A-Za-z0-9]+)\//);
  return m ? `https://media.giphy.com/media/${m[1]}/giphy.gif` : url;
}
async function pasteFromWeb(url) {
  toast('Fetching the picture…');
  try {
    await importBytes(platform.pictureName(url), await platform.fetchPicture(url));
    toast('Pasted the picture.');
    return true;
  } catch (err) {
    toast(`Couldn’t fetch the picture (${err.message || err}). Save it, then drag the file in.`);
    return false;
  }
}

function cropCanvas() {
  const r = app.viewport.cropRect;
  if (!r) return;
  if (app.run('crop_canvas', { rect: r }) !== null) { app.viewport.cropRect = null; app.viewport.fit(); renderToolbar(); }
}
app.onCrop = () => { renderToolbar(); app.viewport.redraw(); };

async function gridDialog() {
  const gd = call('guides');
  const u = app.units();
  const [form, read] = settingsForm([
    ['grid', `Grid spacing (${u.unit})`, 'num', Math.round((gd.grid / u.k) * 100) / 100, { min: 0.01, max: 100000, step: u.step }],
    ['margin', 'Snap margin (px): also stop this far from edges (0: off)', 'num', app.prefs.snapMargin, { min: 0, max: 1000 }],
  ]);
  if (!(await modal('Grid and snapping', form, [['Cancel', false], ['Apply', true, true]]))) return;
  const v = read();
  app.prefs.snapMargin = v.margin;
  app.savePrefs();
  app.run('set_guides', { guides: gd.guides, grid: v.grid * u.k });
}

const aiCtx = () => ({ openSettings: () => openSettings('server'), newTab, addPicture, modal });
app.rasterHooks = { aiSelect: () => aiSelectDialog(app, h, aiCtx()), adjust: () => adjustDialog() };

/// Selection › Adjust colours…: brightness, contrast, hue, saturation, invert, shown live on the selection (or the layer).
async function adjustDialog() {
  const l = app.raster.layer();
  if (!l) return;
  const v = { brightness: 0, contrast: 0, hue: 0, saturation: 0, invert: false };
  const show = () => { try { call('raster_adjust', { id: l.id, brightness: v.brightness / 100, contrast: v.contrast / 100, hue: v.hue, saturation: v.saturation / 100, invert: v.invert, preview: true }); } catch (e) { toast(String(e.message || e)); } app.viewport.redraw(); };
  const slider = (label, k, min, max) => {
    const out = h('span', { class: 'muted', style: 'width: 3em; text-align: right' }, '0');
    return h('label', { style: 'display: grid; grid-template-columns: 7em 1fr 3em; align-items: center; gap: 8px' }, label,
      h('input', { type: 'range', min: String(min), max: String(max), value: '0', 'aria-label': label, onInput: (e) => { v[k] = Number(e.target.value); out.textContent = e.target.value; show(); } }), out);
  };
  const inSel = !!call('raster_info', { id: l.id }).selection;
  const ok = await modal('Adjust colours', h('div', { style: 'display: flex; flex-direction: column; gap: 10px; min-width: 340px' },
    h('small', { class: 'muted' }, inSel ? 'Changes the selected pixels.' : 'Changes the whole layer (select part of it first to change only that).'),
    slider('Brightness', 'brightness', -100, 100), slider('Contrast', 'contrast', -100, 100), slider('Hue', 'hue', -180, 180), slider('Saturation', 'saturation', -100, 100),
    h('label', { class: 'check-row' }, h('input', { type: 'checkbox', onChange: (e) => { v.invert = e.target.checked; show(); } }), h('span', {}, 'Invert the colours'))),
  [['Cancel', false], ['OK', true, true]]);
  try { call('raster_live_end', { keep: !!ok }); } catch (e) { toast(String(e.message || e)); }
  app.changed();
}

/// Animate › New drawing frame: the raster layer showing now ends at the playhead; a new one (blank or a copy) starts.
function newFrame(copy) {
  const l = app.raster.layer();
  if (!l) { toast('Select the raster layer showing now first.'); return; }
  if (!app.state?.comp.duration) { toast('A still picture has no frames: Animate › Animate this canvas first.'); return; }
  if (app.run('raster_new_frame', { id: l.id, copy }) == null) return;
  if (app.onion === undefined) app.onion = app.run('raster_onion', { on: true });
  toast(copy ? 'New frame, a copy of the one before. Move on in time and add the next.' : 'New, empty frame: the one before shows faintly (Animate › Onion skin).');
}

/// Layer › Picture size…: how many pixels a raster layer has.
async function pictureSizeDialog() {
  const l = app.raster.layer();
  if (!l) return;
  const i = call('raster_info', { id: l.id });
  const ratio = i.width / i.height;
  let lock = true;
  const w = h('input', { class: 'field', type: 'number', min: '1', max: '16384', value: String(i.width), 'aria-label': 'Width in pixels', onKeydown: (e) => e.stopPropagation(), onInput: () => { if (lock) hh.value = String(Math.max(1, Math.round(Number(w.value) / ratio))); } });
  const hh = h('input', { class: 'field', type: 'number', min: '1', max: '16384', value: String(i.height), 'aria-label': 'Height in pixels', onKeydown: (e) => e.stopPropagation(), onInput: () => { if (lock) w.value = String(Math.max(1, Math.round(Number(hh.value) * ratio))); } });
  const pct = (k) => h('button', { class: 'small-btn', onClick: () => { w.value = String(Math.round(i.width * k)); hh.value = String(Math.round(i.height * k)); } }, `${Math.round(k * 100)}%`);
  const ok = await modal('Picture size', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
    h('small', { class: 'muted' }, `Now ${i.width} × ${i.height} pixels. It stays the same size on the canvas; more pixels hold finer detail when you paint or zoom in.`),
    h('div', { style: 'display: flex; gap: 8px; align-items: end' }, h('label', {}, 'Width', w), h('label', {}, 'Height', hh)),
    h('div', { style: 'display: flex; gap: 6px' }, pct(0.5), pct(2), pct(4)),
    h('label', { class: 'check-row' }, h('input', { type: 'checkbox', checked: true, onChange: (e) => { lock = e.target.checked; } }), h('span', {}, 'Keep the proportions'))),
  [['Cancel', false], ['Resize', true, true]]);
  if (!ok) return;
  app.run('raster_resize', { id: l.id, width: Number(w.value), height: Number(hh.value) });
}

/// Layer › Extend the picture…: more pixels around it (as far as the canvas, or by an amount), filled in by AI if asked.
async function extendDialog() {
  const l = app.raster.layer();
  if (!l) return;
  let how = 'canvas';
  const n = (label) => h('input', { class: 'field', type: 'number', min: '0', max: '8000', value: '0', 'aria-label': label, onKeydown: (e) => e.stopPropagation() });
  const sides = { left: n('Left'), top: n('Top'), right: n('Right'), bottom: n('Bottom') };
  const byBox = h('div', { style: 'display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px; opacity: .5' }, Object.entries(sides).map(([k, el]) => h('label', {}, k[0].toUpperCase() + k.slice(1), el)));
  let fill = hasAi();
  const what = h('input', { class: 'field', placeholder: 'What should be there (optional), e.g. “more beach and sky”', 'aria-label': 'What the new part shows', onKeydown: (e) => e.stopPropagation() });
  const radio = (v, label) => h('label', { class: 'check-row' }, h('input', { type: 'radio', name: 'extend-how', checked: how === v, onChange: () => { how = v; byBox.style.opacity = v === 'by' ? '1' : '.5'; } }), h('span', {}, label));
  const ok = await modal('Extend the picture', h('div', { style: 'display: flex; flex-direction: column; gap: 10px; min-width: 360px' },
    radio('canvas', 'As far as the canvas goes'), radio('by', 'By this many pixels:'), byBox,
    h('label', { class: 'check-row' }, h('input', { type: 'checkbox', checked: fill, onChange: (e) => { fill = e.target.checked; } }), h('span', {}, 'Fill the new part with AI (it continues the picture)')),
    what),
  [['Cancel', false], ['Extend', true, true]]);
  if (!ok) return;
  const args = how === 'canvas' ? { id: l.id, to_canvas: true } : { id: l.id, ...Object.fromEntries(Object.entries(sides).map(([k, el]) => [k, Number(el.value) || 0])) };
  if (app.run('raster_extend', args) === null) return;
  if (fill) {
    const extra = what.value.trim();
    aiEditDialog(app, h, aiCtx(), { change: `Fill the empty, see-through border around the picture by continuing it naturally (the same scene, light, perspective and style), so it looks like one bigger picture${extra ? `: ${extra}` : ''}.`, only: true, keepAlpha: false, refs: [] });
  }
}
app.renderToolbar = () => renderToolbar();
async function askText(title, label, value = '') {
  const i = h('input', { class: 'field', value, 'aria-label': label, onKeydown: (e) => e.stopPropagation() });
  setTimeout(() => i.select(), 0);
  return (await modal(title, h('label', {}, label, i), [['Cancel', false], ['OK', true, true]])) ? i.value.trim() : null;
}

app.askNumber = async (title, label, value) => {
  const i = h('input', { class: 'field', type: 'number', value: String(value), min: '0', step: '1', 'aria-label': label, onKeydown: (e) => e.stopPropagation() });
  return (await modal(title, h('label', {}, label, i), [['Cancel', false], ['OK', true, true]])) ? Number(i.value) : null;
};

function openSettings(tab) {
  return settingsDialog(app, h, { modal, toast, tools: ALL_TOOLS, shapes: SHAPES, tab });
}
app.onSettings = () => { app.keys = toolKeys(ALL_TOOLS, SHAPES); app.shortcuts = null; renderToolbar(); };

function openExport() {
  return exportDialog(app, h, { modal, toast, title: frontTab()?.title });
}

/// File › Import: SVG becomes shapes, Lottie an animation (a symbol), a picture a picture layer.
async function importFile() {
  let got;
  try { got = await platform.openImport(); } catch (e) { toast(String(e.message || e)); return; }
  if (!got) return;
  await importBytes(got.name, got.bytes, got.path);
}

async function insertSound() {
  let got;
  try { got = await platform.openImport('sound'); } catch (e) { toast(String(e.message || e)); return; }
  if (got && (await importSound(app, got.name, got.bytes, { path: got.path })) != null) toast(`Added ${got.name} at the playhead.`);
}

/// A shape's label (diagram shapes), by double-click.
app.editLabel = (id, now) => app.viewport.beginLabel(id, now || '');

/// Slide › Soundtrack: sound across the whole presentation.
async function soundtrackDialog() {
  const st = call('soundtrack');
  const names = st.names || {};
  delete st.names;
  const rows = h('div', { style: 'display: flex; flex-direction: column; gap: 6px' });
  const draw = () => put(rows, st.clips.length ? st.clips.map((c, i) => h('div', { class: 'prop' },
    h('span', { class: 'pname st-name', title: names[c.audio.asset] || 'Sound' }, names[c.audio.asset] || 'Sound'),
    h('small', { class: 'unit' }, 'starts at'),
    h('input', { class: 'field tiny', type: 'number', step: '0.5', min: '0', value: String(c.start), onChange: (e) => { c.start = Math.max(0, Number(e.target.value) || 0); }, onKeydown: (e) => e.stopPropagation() }),
    h('small', { class: 'unit' }, 's'),
    h('label', { class: 'check-row', title: 'Play it again from the start when it ends' }, h('input', { type: 'checkbox', checked: !!c.audio.looped, onChange: (e) => { c.audio.looped = e.target.checked; } }), h('span', {}, 'Loop')),
    h('button', { class: 'small-btn', html: icon('trash', 14), 'aria-label': 'Remove', onClick: () => { st.clips.splice(i, 1); draw(); } }))) : h('p', { class: 'muted' }, 'No soundtrack yet.'));
  draw();
  const keep = h('input', { type: 'checkbox', checked: !!st.keep_playing });
  const add = h('button', { class: 'small-btn', onClick: async () => {
    const got = await platform.openImport('sound');
    if (!got) return;
    if ((await importSound(app, got.name, got.bytes, { soundtrack: true, start: 0, path: got.path })) !== null) { back.remove(); soundtrackDialog(); }
  } }, '+ Add a sound file');
  let back;
  const p = modal('Soundtrack', h('div', { class: 'soundtrack-box' },
    h('p', {}, 'Sound that plays across the whole presentation, from where each clip starts.'), rows, add,
    h('label', { class: 'check-row' }, keep, h('span', {}, 'Keep playing while the presentation waits at a pause'))), [['Cancel', false], ['Apply', true, true]]);
  back = document.querySelector('.modal-back:last-of-type');
  if (await p) app.run('set_soundtrack', { soundtrack: { ...st, keep_playing: keep.checked } });
}

async function importVideo(name, bytes, path = null) {
  let info;
  // The desktop reads any video ffmpeg reads (AVI too, with its real frame rate); the browser what it can play.
  try { info = platform.DESKTOP && path ? await platform.probeMedia(path) : await probeVideo(bytes); } catch {
    toast(/\.avi$/i.test(name) && !platform.DESKTOP ? `${name}: browsers can’t play AVI. The desktop app reads it, or convert it to MP4 or WebM.` : `${name} isn’t a video this program can play.`);
    return;
  }
  // The desktop links the file (the canvas stays small), unless Settings says to keep clips inside it.
  const link = path && !app.settings?.embedMedia;
  if (!link && bytes.length > 200e6) toast('That video is big: the canvas file will be big too.');
  let s = '';
  if (!link) for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  const mime = /\.webm$/i.test(name) ? 'video/webm' : /\.mov$/i.test(name) ? 'video/quicktime' : /\.avi$/i.test(name) ? 'video/x-msvideo' : /\.mkv$/i.test(name) ? 'video/x-matroska' : 'video/mp4';
  const src = link ? { path, size: bytes.length } : { base64: btoa(s) };
  if (app.run('add_video', { name: name.replace(/\.[a-z0-9]+$/i, ''), mime, ...src, ...info }) != null) toast(`Added ${name} at the playhead.`);
}

async function insertVideo() {
  let got;
  try { got = await platform.openImport('video'); } catch (e) { toast(String(e.message || e)); return; }
  if (got) await importVideo(got.name, got.bytes, got.path);
}

async function insertModel() {
  let got;
  try { got = await platform.openImport('model'); } catch (e) { toast(String(e.message || e)); return; }
  if (got) importModel(got.name, got.bytes);
}

function importModel(name, bytes) {
  return modelDialog(app, h, { modal, toast }, { name, bytes });
}

/// 3D layers of canvases from before Insert › 3D model made pictures: they become pictures, as they look now.
function flatten3d() {
  const ids = (app.state?.layers || []).filter((l) => l.kind === '3d').map((l) => l.id);
  if (!ids.length) return;
  try {
    engineRaw().rasterize(JSON.stringify(ids), false);
    app.changed();
    toast(ids.length > 1 ? `Its ${ids.length} 3D layers are pictures now.` : 'Its 3D layer is a picture now.');
  } catch { /* they stay as they were */ }
  afterOffscreenDraw();
}

async function importBytes(name, bytes, path = null) {
  if (/\.(stl|obj|gltf|glb|3mf)$/i.test(name)) { importModel(name, bytes); return; }
  if (/\.(mp4|webm|mov|m4v|mkv|avi)$/i.test(name)) { await importVideo(name, bytes, path); return; }
  if (/\.(wav|mp3|ogg|oga|opus|flac|m4a|aac)$/i.test(name)) { if ((await importSound(app, name, bytes, { path })) != null) toast(`Added ${name} at the playhead.`); return; }
  const head = new TextDecoder().decode(bytes.subarray(0, 400)).trimStart();
  const b64 = () => { let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return btoa(s); };
  if (/\.svg$/i.test(name) || head.startsWith('<svg') || (head.startsWith('<?xml') && head.includes('<svg'))) {
    if (app.run('import_svg', { base64: b64(), name: name.replace(/\.svg$/i, '') }) != null) { app.setTool('select'); toast(`Imported ${name}.`); }
  } else if (/\.(json|lottie)$/i.test(name) || head.startsWith('{')) {
    const r = app.run('import_lottie', { base64: b64(), name: name.replace(/\.(json|lottie)$/i, '') });
    if (r) toast(`Imported ${name} as an animation. ${r.summary || ''}`);
  } else {
    await addPicture(name, bytes);
  }
}

/// File › Trace: a picture's colours become editable shapes.
/// Trace a picture into shapes: the selected picture layer (shapes go over it), or a picture file.
async function traceDialog(layerId) {
  const pic = layerId ?? (one()?.kind === 'picture' ? one().id : null);
  let got = null;
  if (pic == null) {
    try { got = await platform.openPicture(); } catch (e) { toast(String(e.message || e)); return; }
    if (!got) return;
  }
  const t = app.traceSet ||= { colors: 5, noise: 8, simplify: 1, smooth: true, remove_background: true };
  const [form, read] = settingsForm([
    ['colors', 'Detail (1: few colours, 8: many)', 'num', t.colors, { min: 1, max: 8 }],
    ['noise', 'Ignore specks smaller than (px)', 'num', t.noise, { min: 1, max: 64 }],
    ['simplify', 'Simplify the outlines by (px)', 'num', t.simplify ?? 1, { min: 0, max: 5, step: 0.5 }],
    ['smooth', 'Smooth curves', 'check', t.smooth],
    ['remove_background', 'Leave out the background', 'check', t.remove_background],
  ]);
  const what = pic != null ? `“${app.layerById(pic)?.name || 'The picture'}” gets editable shapes over it, one colour each (the picture stays underneath).` : `“${got.name}” becomes shapes you can edit, one colour each.`;
  const ok = await modal('Trace a picture', h('div', {}, h('p', {}, what), h('p', { class: 'muted' }, 'Works best on flat artwork: logos, icons, illustrations.'), form), [['Cancel', false], ['Trace', true, true]]);
  if (!ok) return;
  Object.assign(t, read());
  if (pic != null) {
    if (app.run('trace_layer', { id: pic, ...t }) != null) { app.setTool('select'); toast('Traced the picture into shapes.'); }
    return;
  }
  const p = await readPicture(got.name, got.bytes);
  if (!p) return;
  const r = app.run('trace', { base64: p.base64, name: got.name.replace(/\.[a-z]+$/i, ''), ...t });
  if (r) { app.setTool('select'); toast(`Traced ${got.name}.`); }
}

// Recent canvases: paths on the desktop; on the web, small canvases kept in this browser.
function recentList() {
  try { return JSON.parse(localStorage.getItem('draw.recent') || '[]'); } catch { return []; }
}
function remember({ name, text, bytes, path }) {
  const when = new Date().toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  // Small ones are kept here whole (the browser has no paths).
  const small = bytes ? bytes.length < 300_000 && { file: toB64(bytes) } : text.length < 400_000 && { text };
  const entry = path ? { name, path, when } : small ? { name, ...small, when } : null;
  if (!entry) return;
  const list = [entry, ...recentList().filter((r) => (path ? r.path !== path : r.name !== name))].slice(0, app.settings?.recentMax || 6);
  try { localStorage.setItem('draw.recent', JSON.stringify(list)); } catch { /* full: not kept */ }
}
async function openRecent(r) {
  if (r.file) return openText({ name: r.name, bytes: b64bytes(r.file), path: null });
  if (r.text) return openText({ name: r.name, text: r.text, path: null });
  try {
    const got = await platform.openedFromPath(r.path);
    openText(got);
  } catch (e) {
    toast(`Couldn't open ${r.name}: ${e.message || e}`);
  }
}

// The desktop keeps each unsaved canvas in its data folder while Draw runs,
// offered back if Draw ends without them being saved (a crash, power cut).
const RUN = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
const keyOf = (tab) => `${RUN}-${tab.id}`;
let deskTimer = null;
function deskAutosave() {
  clearTimeout(deskTimer);
  deskTimer = setTimeout(() => {
    const tab = frontTab();
    if (!tab) return;
    if (app.state.dirty) platform.autosave.write(keyOf(tab), tab.title, call('save_copy')).catch(() => {});
    else platform.autosave.clear(keyOf(tab)).catch(() => {});
  }, (app.settings?.autosaveSecs || 3) * 1000);
}

async function offerRecovery() {
  const left = await platform.autosave.leftovers(RUN).catch(() => []);
  if (!left.length) return;
  const when = (s) => new Date(s * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const ok = await modal('Recover unsaved canvases?', h('div', { style: 'display: flex; flex-direction: column; gap: 8px' },
    h('p', {}, 'Draw closed last time before these were saved:'),
    h('ul', {}, left.map((l) => h('li', {}, `${l.name} (${when(l.when)})`)))), [['Discard them', false], ['Recover', true, true]]);
  for (const l of left) {
    if (ok) {
      try { newTab(l.name, () => { call('open', { json: l.text }); call('mark_unsaved'); }); } catch { /* unreadable */ }
    }
    platform.autosave.clear(l.key).catch(() => {});
  }
  if (ok) app.changed();
}

/// File › Print: the frame at the playhead, through the system's print dialog.
/// File › Print: the frame at the playhead, scaled to fit the page. The desktop prints straight to
/// a printer (CUPS) with copies; otherwise the system's print dialog, or a PNG to print elsewhere.
async function printFrame() {
  const st = app.state;
  const [why, printers] = platform.DESKTOP ? await platform.printers() : [null, []];
  const copies = h('input', { class: 'field', type: 'number', min: '1', max: '99', value: '1', style: 'width: 90px', 'aria-label': 'Copies', onKeydown: (e) => e.stopPropagation() });
  const printer = h('select', { class: 'field', 'aria-label': 'Printer' }, printers.map((p, i) => h('option', { value: p, selected: i === 0 }, p)));
  const direct = printers.length > 0;
  const body = h('div', { class: 'print-box', style: 'display: flex; flex-direction: column; gap: 10px' },
    h('p', {}, `Prints the frame at ${fmtTime(st.time)}, scaled to fit the page.`),
    direct ? h('div', { class: 'grid2' }, h('label', {}, 'Printer', printer), h('label', {}, 'Copies', copies)) : null,
    why && platform.DESKTOP ? h('p', { class: 'muted' }, `Can’t print straight to a printer: ${why}. The system’s print dialog may still work, or save the frame as a PNG and print it from an image viewer.`) : null);
  const choice = await modal('Print', body, [['Cancel', null], ['Save as PNG instead…', 'png'], ...(direct ? [['System dialog…', 'system'], ['Print', 'direct', true]] : [['Print…', 'system', true]])]);
  if (!choice) return;
  if (choice === 'png') { app.lastExport = { ...(app.lastExport || {}), format: 'png' }; openExport(); return; }
  const k = Math.min(3, 4000 / Math.max(st.comp.width, st.comp.height));
  const { renderPng } = await import('./export.js');
  const bytes = await renderPng(app, st.time, k);
  if (choice === 'direct') {
    try {
      let b = '';
      for (let i = 0; i < bytes.length; i += 0x8000) b += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      await platform.printPng(btoa(b), printer.value, Math.max(1, Math.min(99, Math.round(Number(copies.value)) || 1)));
      toast(`Sent to ${printer.value}.`);
    } catch (e) { toast(`Couldn’t print: ${e.message || e}`); }
    return;
  }
  const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
  const frame = h('iframe', { style: 'position: fixed; width: 0; height: 0; border: 0; right: 0; bottom: 0', 'aria-hidden': 'true' });
  document.body.append(frame);
  const d = frame.contentDocument;
  d.open();
  d.write(`<!doctype html><title>${esc(frontTab()?.title || 'Canvas')}</title><style>@page{margin:12mm}html,body{margin:0;height:100%}body{display:flex;align-items:center;justify-content:center}img{max-width:100%;max-height:100vh;object-fit:contain}</style><img src="${url}">`);
  d.close();
  frame.contentWindow.addEventListener('afterprint', () => setTimeout(() => { frame.remove(); URL.revokeObjectURL(url); }, 500));
  const img = d.querySelector('img');
  try { await img.decode(); } catch { await new Promise((r) => { img.onload = r; img.onerror = r; setTimeout(r, 3000); }); }
  frame.contentWindow.focus();
  frame.contentWindow.print();
}

// The browser keeps the front canvas as you work, offered back after a reload.
let saveTimer = null;
function autosave() {
  // Server canvases aren't kept here: their unsent changes are kept apart
  // (server.keepUnsent).
  if (frontTab()?.remote) return;
  if (platform.DESKTOP) { deskAutosave(); return; }
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try { localStorage.setItem('draw.autosave', JSON.stringify({ name: frontTab()?.title || 'Untitled', text: call('save_copy'), dirty: !!app.state?.dirty })); } catch { /* too big */ }
  }, 1500);
}

async function quit(restart = false) {
  // Canvases on a server are saved as you work: the last changes go now.
  await server.versionNow();
  for (const t of [...app.tabs]) {
    if (!isDirty(t)) continue;
    const what = await unsavedPrompt(t);
    if (!what || (what === 'save' && !(await saveTab(t)))) return;
  }
  await Promise.all(app.tabs.map((t) => platform.autosave.clear(keyOf(t)).catch(() => {})));
  // `restart`: into the newer version Xenify Hub installed.
  if (restart) window.__TAURI__.core.invoke('restart_updated');
  else if (platform.DESKTOP) platform.quit();
  // In the browser, from a Xenify server: back to its Files page.
  else if (servedHere) {
    leaving = true;
    await server.quitToFiles();
  }
}

// Xenify Hub installed a newer version: Help offers to restart into it.
let updateReady = null;
async function checkUpdate() {
  if (!platform.DESKTOP) return;
  try { updateReady = await window.__TAURI__.core.invoke('update_ready'); } catch { updateReady = null; }
}
checkUpdate();
window.addEventListener('focus', () => checkUpdate());
setInterval(() => checkUpdate(), 120000);

// ----- Dialogs -----

/// Enter and Escape for a dialog on `back` (a .modal-back): Escape runs
/// `cancel`, Enter clicks its main button (or its only one). Only the top
/// dialog answers, not while a popup is open, and Enter is left to what uses
/// it itself: a text area, a list, a button, a text being edited,
/// Ctrl/Alt/⌘, an accent being typed; a key a box in it took
/// (preventDefault) is left alone. Returns the function to call when it closes.
function dialogKeys(back, cancel) {
  const top = () => back === [...document.querySelectorAll('.modal-back')].pop();
  const wanted = (e) => (e.key === 'Escape' || (e.key === 'Enter' && !e.ctrlKey && !e.altKey && !e.metaKey)) && !e.isComposing;
  const act = (e) => {
    if (!back.isConnected || !top() || openPopup) return;
    if (e.key === 'Escape') { cancel(); return; }
    const btns = [...back.querySelectorAll(':scope > .modal > .modal-actions .btn')];
    const main = btns.find((b) => b.classList.contains('main')) || (btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) main.click();
  };
  const key = (e) => {
    if (!back.isConnected || !top() || !wanted(e) || openPopup) return;
    const t = e.target;
    if (!back.contains(t)) {
      // Focus left outside (the button that opened it); a text box out there is its own.
      if (t.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      e.preventDefault(); e.stopPropagation(); act(e);
      return;
    }
    if (e.key === 'Enter' && t.closest('textarea, select, button, a[href], [contenteditable]:not([contenteditable="false"]), [aria-expanded="true"]')) return;
    // After the dialog's own boxes have had it (most stop keys going further, so the canvas doesn't get them).
    setTimeout(() => { if (!e.defaultPrevented) act(e); });
  };
  document.addEventListener('keydown', key, true);
  return () => document.removeEventListener('keydown', key, true);
}

function modal(title, body, actions) {
  return new Promise((resolve) => {
    const back = h('div', { class: 'modal-back' });
    const done = (v) => { back.remove(); unkey(); resolve(v); };
    const box = h('div', { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
      h('h2', {}, title), body,
      h('div', { class: 'modal-actions' }, actions.map(([label, value, main]) => h('button', { class: `btn${main ? ' main' : ''}`, onClick: () => done(typeof value === 'function' ? value() : value) }, label))));
    back.append(box);
    // Escape: nothing; Enter: the main button. No keys on to the canvas underneath.
    const unkey = dialogKeys(back, () => done(null));
    back.addEventListener('keydown', (e) => e.stopPropagation());
    back.addEventListener('pointerdown', (e) => { if (e.target === back) done(null); });
    document.body.append(back);
    (box.querySelector('input, select') || box.querySelector('.btn.main') || box.querySelector('.btn'))?.focus();
  });
}

function ask(title, text, yes) {
  return modal(title, h('p', {}, text), [['Cancel', false], [yes, true, true]]);
}

async function canvasDialog() {
  const c = app.state.comp;
  // Sizes are typed in the canvas's units; the picture itself is pixels.
  const st = { unit: c.units?.unit || 'px', dpi: c.units?.dpi || 96, w: c.width, h: c.height, grid: c.grid || 0, blur: c.motion_blur ? { ...c.motion_blur } : null, kind: c.kind || 'animation' };
  const PER_INCH = { mm: 25.4, cm: 2.54, in: 1, pt: 72 };
  const perUnit = () => (st.unit === 'px' ? 1 : st.dpi / PER_INCH[st.unit]);
  const shown = (px) => Math.round((px / perUnit()) * 100) / 100;
  const name = h('input', { class: 'field', value: c.name });
  const num = (label, value, step, onChange, attrs = {}) => {
    const i = h('input', { class: 'field', type: 'number', value: String(value), step: String(step), ...attrs, onChange: (e) => onChange(Number(e.target.value)), onKeydown: (e) => e.stopPropagation() });
    return h('label', {}, label, i);
  };
  const d = h('input', { class: 'field', type: 'number', value: String(+c.duration.toFixed(2)), step: '0.5', min: '0' });
  const fps = h('input', { class: 'field', type: 'number', value: String(c.fps), step: '1', min: '1', max: '240' });
  let bgNow = c.background;
  const bg = h('div');
  const drawBg = () => bg.replaceChildren(backgroundRow({ background: bgNow }, (v) => { bgNow = v.length === 7 ? `${v}ff` : v; drawBg(); }));
  drawBg();
  const sizeBox = h('div', { style: 'display: flex; flex-direction: column; gap: 12px' });
  const warn = h('p', { class: 'warn-note', hidden: true });
  const checkEnd = () => {
    const len = Number(d.value);
    warn.hidden = !(c.last_key > len + 1e-6 && len > 0);
    warn.textContent = `Keyframes run to ${c.last_key.toFixed(2)} s: a ${len.toFixed(2)} s canvas cuts off what comes after.`;
  };
  d.addEventListener('input', checkEnd);
  const draw = () => {
    const u = st.unit, sfx = u === 'px' ? 'px' : u;
    sizeBox.replaceChildren(
      h('div', { class: 'grid2' },
        num(`Width (${sfx})`, shown(st.w), u === 'px' ? 1 : 0.1, (v) => { st.w = Math.max(1, Math.round(v * perUnit())); }),
        num(`Height (${sfx})`, shown(st.h), u === 'px' ? 1 : 0.1, (v) => { st.h = Math.max(1, Math.round(v * perUnit())); })),
      u === 'px' ? null : h('small', { class: 'muted' }, `${st.w} × ${st.h} pixels at ${st.dpi} ppi`),
      h('div', { class: 'grid2' },
        h('label', { title: 'Rulers, guides and Position and size use these (the picture is still made of pixels)' }, 'Units',
          h('select', { class: 'field', onChange: (e) => { st.unit = e.target.value; draw(); } },
            [['px', 'Pixels'], ['mm', 'Millimetres'], ['cm', 'Centimetres'], ['in', 'Inches'], ['pt', 'Points']].map(([v, l]) => h('option', { value: v, selected: st.unit === v }, l)))),
        u === 'px' ? h('span') : h('label', { title: 'Pixels per inch: 300 for print, 96 for screens' }, 'Resolution (ppi)',
          h('div', { style: 'display: flex; gap: 4px; align-items: center' },
            h('input', { class: 'field', type: 'number', value: String(st.dpi), min: '10', max: '2400', style: 'flex: 1; min-width: 0', onChange: (e) => { st.dpi = Math.max(10, Math.min(2400, Number(e.target.value) || 96)); draw(); }, onKeydown: (e) => e.stopPropagation() }),
            [72, 96, 150, 300].map((v) => h('button', { class: `small-btn${st.dpi === v ? ' on' : ''}`, onClick: () => { st.dpi = v; draw(); } }, String(v)))))),
      h('label', { title: 'The spacing of View › Grid' }, `Grid spacing (${sfx})`,
        h('div', { style: 'display: flex; gap: 6px; align-items: center' },
          h('input', { class: 'field', type: 'number', min: '0', step: u === 'px' ? '1' : '0.1', value: String(st.grid ? shown(st.grid) : ''), placeholder: `Auto (${shown(c.grid_spacing || 50)})`, style: 'flex: 1',
            onChange: (e) => { st.grid = Math.max(0, Number(e.target.value) || 0) * perUnit(); }, onKeydown: (e) => e.stopPropagation() }),
          h('button', { class: 'small-btn', onClick: () => { st.grid = 0; draw(); } }, 'Auto'))));
  };
  draw();
  const blurRow = h('div', { style: 'display: flex; flex-direction: column; gap: 8px' });
  const drawBlur = () => blurRow.replaceChildren(
    h('label', { class: 'check-row', title: 'Blurs fast motion in exported video (not the live canvas)' }, h('input', { type: 'checkbox', checked: !!st.blur, onChange: (e) => { st.blur = e.target.checked ? { shutter_angle: 180, samples: 8 } : null; drawBlur(); } }), h('span', {}, 'Motion blur in exports')),
    st.blur ? h('div', { class: 'grid2' },
      num('Shutter (°: 180 is film-like)', st.blur.shutter_angle, 10, (v) => { st.blur.shutter_angle = Math.max(10, Math.min(720, v)); }, { min: '10', max: '720' }),
      num('Samples (more: smoother, slower)', st.blur.samples, 1, (v) => { st.blur.samples = Math.max(2, Math.min(32, Math.round(v))); }, { min: '2', max: '32' })) : null);
  drawBlur();
  const kind = h('div', { class: 'segs', role: 'radiogroup', 'aria-label': 'Kind', style: 'display: flex' });
  const drawKind = () => kind.replaceChildren(...[['animation', 'Animation'], ['diagram', 'Diagram']].map(([v, l]) => h('button', { class: `seg${st.kind === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String(st.kind === v), style: 'flex: 1 1 0',
    title: v === 'diagram' ? 'Flowcharts, network and architecture diagrams: shapes joined by connectors, with the diagram tools' : 'Drawings and animation', onClick: () => { st.kind = v; drawKind(); } }, l)));
  drawKind();
  checkEnd();
  const ok = await modal('Canvas settings', h('div', { class: 'canvas-settings', style: 'display: flex; flex-direction: column; gap: 12px' },
    h('label', {}, 'Name', name), h('label', {}, 'Kind', kind), sizeBox,
    h('div', { class: 'grid2' }, h('label', {}, 'Length (seconds, 0 for still)', d), h('label', {}, 'Frames a second', fps)), warn,
    bg, blurRow),
  [['Cancel', false], ['Apply', true, true]]);
  if (!ok) return;
  app.run('set_comp', { name: name.value.trim() || c.name, width: st.w, height: st.h, duration: Number(d.value), fps: Number(fps.value), background: bgNow,
    units: { unit: st.unit, dpi: st.dpi }, grid: st.grid, motion_blur: st.blur, kind: st.kind });
  const tab = frontTab();
  if (tab && !tab.path) { tab.title = name.value.trim() || tab.title; renderTabs(); }
  app.viewport.fit();
}

/// The canvas's units: pixels per unit, the suffix and a sensible step.
app.units = () => {
  const u = app.state?.comp?.units;
  if (!u || u.unit === 'px') return { k: 1, unit: 'px', step: 1, decimals: 0 };
  return { k: u.px_per_unit, unit: u.suffix, step: u.unit === 'mm' || u.unit === 'pt' ? 1 : 0.1, decimals: u.decimals };
};

function settingsForm(fields) {
  const inputs = {};
  const rows = fields.map(([key, label, kind, value, extra = {}]) => {
    const input = kind === 'check'
      ? h('input', { type: 'checkbox', checked: value })
      : h('input', { class: 'field', type: 'number', value: String(value), step: String(extra.step ?? 1), min: extra.min != null ? String(extra.min) : null, max: extra.max != null ? String(extra.max) : null });
    inputs[key] = [input, kind, extra.k || 1];
    return kind === 'check' ? h('label', { class: 'check-row' }, input, h('span', {}, label)) : h('label', {}, label, input);
  });
  const read = () => Object.fromEntries(Object.entries(inputs).map(([k, [i, kind, mul]]) => [k, kind === 'check' ? i.checked : Number(i.value) / mul]));
  return [h('div', { class: 'form-grid' }, rows), read];
}

/// Animate › Fall and bounce: baked physics from the playhead.
async function physicsDialog() {
  const s = app.physics ||= { gravity: 2400, bounce: 60, friction: 15, squish: 50, floor: true, walls: true, ceiling: false, obstacles: true, squish_moving: true, squish_hit: false };
  const [form, read] = settingsForm([
    ['gravity', 'Gravity (px/s²)', 'num', s.gravity, { step: 100, min: 0 }],
    ['bounce', 'Bounce (%)', 'num', s.bounce, { min: 0, max: 100, step: 5 }],
    ['friction', 'Friction (%)', 'num', s.friction, { min: 0, max: 100, step: 5 }],
    ['squish', 'Squish on impact (%)', 'num', s.squish, { min: 0, max: 100, step: 5 }],
    ['floor', 'Bounce on the bottom edge', 'check', s.floor], ['walls', 'Bounce on the sides', 'check', s.walls], ['ceiling', 'Bounce on the top edge', 'check', s.ceiling],
    ['obstacles', 'Other layers get in the way', 'check', s.obstacles], ['squish_moving', 'Falling layers squish', 'check', s.squish_moving], ['squish_hit', 'What they hit squishes', 'check', s.squish_hit],
  ]);
  const ok = await modal('Fall and bounce', h('div', {}, h('p', {}, `The selected layers fall from where they are at ${fmtTime(app.state.time)}, written as keyframes to the end.`), form), [['Cancel', false], ['Make it fall', true, true]]);
  if (!ok) return;
  Object.assign(s, read());
  app.run('physics', { ...s, bounce: s.bounce / 100, friction: s.friction / 100, squish: s.squish / 100 });
}

/// Animate › Follow a path: the selected layer goes along a line or round a shape, from the playhead.
async function followPathDialog() {
  const id = sel()[0];
  const paths = app.state.layers.filter((l) => l.id !== id && l.kind === 'shape');
  if (!paths.length) { toast('Draw the path first: a line with the Pen or Pencil, or a shape such as a circle. Then select what should travel and pick Follow a path again.'); return; }
  const c = app.state.comp;
  const left = Math.max(0.1, +(c.duration - app.state.time).toFixed(2));
  const s = app.followSet ||= { turn: false, reverse: false };
  const pathSel = h('select', { class: 'field', 'aria-label': 'The path' }, paths.map((l) => h('option', { value: String(l.id), selected: l.id === s.path }, l.name)));
  const secs = h('input', { class: 'field', type: 'number', min: '0.1', max: '3600', step: '0.5', value: String(Math.min(left, s.seconds || left)), 'aria-label': 'Seconds' });
  const turn = h('input', { type: 'checkbox', checked: s.turn });
  const rev = h('input', { type: 'checkbox', checked: s.reverse });
  const name = app.layerById(id)?.name || 'The layer';
  const ok = await modal('Follow a path', h('div', { class: 'stack' },
    h('p', {}, `“${name}” goes along the path from ${fmtTime(app.state.time)}, at an even speed (its joint, the dot the Bones tool shows, stays on the line). Its earlier movement is replaced.`),
    h('label', {}, 'The path (a line, or a shape’s outline)', pathSel),
    h('label', {}, 'Taking (seconds)', secs),
    h('label', { class: 'check-row' }, turn, h('span', {}, 'Turn to face along the path')),
    h('label', { class: 'check-row' }, rev, h('span', {}, 'Go the other way'))), [['Cancel', false], ['Follow it', true, true]]);
  if (!ok) return;
  Object.assign(s, { path: Number(pathSel.value), seconds: Math.max(0.1, Number(secs.value) || left), turn: turn.checked, reverse: rev.checked });
  if (app.run('follow_path', { id, path: s.path, seconds: s.seconds, turn: s.turn, reverse: s.reverse }) !== null) toast('Press Space to watch it. The path stays a layer you can hide.');
}

/// Ready-made effects (confetti, fire, smoke, water, shatter…), added as layers.
async function effectGallery() {
  const cat = call('effect_gallery');
  const haveLayer = sel().length > 0;
  let pick = null;
  const body = h('div', { class: 'gallery' }, cat.map((sec) => [
    h('h3', {}, sec.section),
    h('div', { class: 'gallery-grid' }, sec.effects.map((e) => h('button', {
      class: 'gallery-card', disabled: e.needs_layer && !haveLayer, title: e.needs_layer && !haveLayer ? 'Select the layer first' : e.tip,
      onClick: () => { pick = e.name; document.querySelector('.modal-back .btn.main')?.click(); },
    }, h('span', { class: 'thumb-blank', 'data-fx': e.name }, '…'), h('strong', {}, e.name), h('span', {}, e.tip)))),
  ]));
  // Each card plays its effect: 12 frames drawn a few at a time, then looped (one still frame with reduce motion).
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches || app.settings?.reduceMotion;
  const frames = new Map();
  const cards = [...body.querySelectorAll('[data-fx]')];
  let alive = true, tick = 0, next = 0;
  const play = () => {
    if (!alive || !body.isConnected) { if (tick > 2) { alive = false; for (const fr of frames.values()) fr.forEach((u) => u && URL.revokeObjectURL(u)); return; } }
    tick++;
    const until = performance.now() + 10;
    const want = still ? [6] : [...Array(12).keys()];
    while (performance.now() < until && next < cards.length * want.length) {
      const el = cards[next % cards.length], f = want[Math.floor(next / cards.length)];
      next++;
      const name = el.dataset.fx;
      if (!frames.has(name)) frames.set(name, []);
      try { frames.get(name)[f] = URL.createObjectURL(new Blob([engineRaw().effect_preview(name, f, 352, 198)], { type: 'image/png' })); } catch { /* none */ }
    }
    app.viewport.drawNow();
    const f = still ? 6 : Math.floor(performance.now() / 275) % 12;
    for (const el of cards) {
      const fr = frames.get(el.dataset.fx);
      const url = fr?.[f] || fr?.[still ? 6 : 0];
      if (!url) continue;
      if (el.tagName !== 'IMG') { const img = h('img', { src: url, alt: '', 'data-fx': el.dataset.fx }); el.replaceWith(img); cards[cards.indexOf(el)] = img; } else if (el.getAttribute('src') !== url) el.src = url;
    }
    if (alive) setTimeout(() => requestAnimationFrame(play), 60);
  };
  requestAnimationFrame(play);
  const ok = await modal('Effects', body, [['Close', false], ['Add', () => !!pick, true]]);
  alive = false;
  if (ok && pick) {
    const at = app.viewport.lastPoint || null;
    if (app.run('add_gallery_effect', { name: pick, at }) != null) toast(`Added ${pick}. Its settings are in the Layer panel; press Space to watch it.`);
  }
}

function shortcuts() {
  // The tools by the keys they have now (Settings › Keys can change them).
  const keys = app.keys || toolKeys(ALL_TOOLS, SHAPES);
  const toolRows = [...TOOLS.map((t) => [keys[t.id], t.label.split(':')[0]]), ...SHAPES.map((x) => [keys[`shape:${x.id}`], x.label.split(':')[0]])].filter(([k]) => k);
  const rows = [
    ...toolRows,
    ['Ctrl+C / X / V', 'Copy, cut, paste layers'], ['Ctrl+G / Ctrl+Shift+G', 'Group / ungroup'],
    ['Ctrl+] / Ctrl+[', 'Forward / backward (with Shift: front / back)'], ['Ctrl+7', 'Clipping mask'],
    ['Tab', 'Next layer'], ['Right-click', 'Menus for the canvas, layers and points'],
    ['Space', 'Play or pause (hold and drag to move around)'],
    ['Wheel / Shift + wheel', 'Scroll up and down / sideways'], ['Ctrl or Alt + wheel', 'Zoom'],
    ['Middle-drag', 'Move around, with any tool'], ['Ctrl+0 or Shift+1 / Ctrl+1', 'Fit / actual size'],
    ['Drag a number', 'Change it (Shift: faster, Alt: finer); the wheel steps it'],
    ['Arrows', 'Nudge (Shift: 10 px)'], ['Shift while dragging', 'Keep straight or in proportion'],
    ['Ctrl+Z / Ctrl+Shift+Z', 'Undo / redo'], ['Ctrl+D', 'Duplicate'], ['Delete', 'Delete'],
    ['Double-click text', 'Type on the canvas'], ['F7', 'Spelling and grammar'],
    ['Ctrl+S / Ctrl+O / Ctrl+N', 'Save / open / new'],
  ];
  const map = app.shortcuts || (app.shortcuts = shortcutMap());
  const all = [...rows, ...COMMANDS.filter(([id]) => map[id]?.length).map(([id, label]) => [map[id].join(' or '), label])];
  modal('Keyboard shortcuts', h('div', { class: 'keys-list', style: 'max-height: 60vh; overflow-y: auto' }, all.map(([k, v]) => [h('kbd', {}, k), h('span', {}, v)]),
    h('span', {}), h('span', { class: 'muted' }, 'Change them in Settings › Keys.')), [['Close', null, true]]);
}

/// Help › Credits and licences: Draw's licence and everything it's built with.
async function credits() {
  const pre = h('pre', { class: 'credits', tabindex: '0', 'aria-label': 'Licences' }, 'Loading…');
  modal('Credits and licences', h('div', { class: 'credits-box' }, h('p', {}, 'Xenify Draw is free software under the MIT licence. It’s built with these, under their own licences:'), pre), [['Close', null, true]]);
  try {
    const [mine, theirs] = await Promise.all([platform.bundledText('LICENSE.txt'), platform.bundledText('THIRD_PARTY_LICENSES.txt')]);
    pre.textContent = `${mine.trim()}\n\n${'─'.repeat(40)}\n\n${theirs}`;
  } catch (e) { pre.textContent = `The licences couldn’t be read: ${e.message || e}`; }
}

function about() {
  modal('About Xenify Draw', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
    h('p', {}, 'Draw pictures, posters and animations. Words on the canvas are checked for spelling and grammar on this computer, with Xenify Write’s checker.'),
    h('p', {}, `Drawing with ${app.renderer}. ${platform.DESKTOP ? 'Desktop app.' : 'Browser version: canvases are saved as downloads.'}`),
    h('p', { style: 'font-size: 13.5px; color: var(--muted)' }, 'Free software under the MIT licence.')), [['Close', null, true]]);
}

/// Plays the canvas for five seconds and reports how fast it drew: for
/// checking Draw on a computer (the Linux web view draws with WebGL2).
async function speedTest({ quiet = false } = {}) {
  if (!app.state?.comp.duration) return null;
  const wasShown = !$('perf').hidden;
  $('perf').hidden = false;
  app.setTime(0);
  app.viewport.stats.frames = [];
  app.loop = true;
  if (!app.playing) app.togglePlay();
  const frames = [];
  const t0 = performance.now();
  await new Promise((resolve) => {
    const grab = () => {
      frames.push(...app.viewport.stats.frames.splice(0));
      if (performance.now() - t0 < 5000) setTimeout(grab, 250); else resolve();
    };
    grab();
  });
  if (app.playing) app.togglePlay();
  $('perf').hidden = !wasShown;
  const avg = (k) => frames.reduce((n, x) => n + x[k], 0) / Math.max(1, frames.length);
  const sorted = frames.map((x) => x.total).sort((a, b) => a - b);
  const p95 = sorted[Math.floor(sorted.length * 0.95)] || 0;
  const r = {
    renderer: app.renderer, fps: Math.round(frames.length / 5), frame: +avg('total').toFixed(2), encode: +avg('encode').toFixed(2),
    gpu: +avg('gpu').toFixed(2), p95: +p95.toFixed(2), pixels: `${app.viewport.gl.width}×${app.viewport.gl.height}`,
    layers: app.state.layers.length, browser: navigator.userAgent,
  };
  platform.log(`speed test: ${JSON.stringify(r)}`);
  if (!quiet) {
    modal('Speed test', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
      h('p', {}, `${r.fps} frames a second while playing (the screen shows at most its refresh rate, usually 60).`),
      h('div', { class: 'keys-list' },
        h('span', {}, 'Drawing a frame'), h('span', {}, `${r.frame} ms on average, ${r.p95} ms at worst (95%)`),
        h('span', {}, 'Preparing it'), h('span', {}, `${r.encode} ms`),
        h('span', {}, 'Handing it to the GPU'), h('span', {}, `${r.gpu} ms`),
        h('span', {}, 'Canvas'), h('span', {}, `${r.pixels} pixels, ${r.layers} layers`),
        h('span', {}, 'Drawn with'), h('span', {}, r.renderer)),
      h('p', { style: 'font-size: 13.5px; color: var(--muted)' }, 'Under 16 ms a frame keeps up with a 60 Hz screen.')), [['Close', null, true]]);
  }
  return r;
}

function togglePerf() {
  const p = $('perf');
  p.hidden = !p.hidden;
  localStorage.setItem('draw.perf', p.hidden ? '0' : '1');
  app.viewport.redraw();
  renderStatus();
}

function toggleDark() {
  const root = document.documentElement;
  const dark = root.dataset.theme !== 'dark';
  root.dataset.theme = dark ? 'dark' : 'light';
  localStorage.setItem('draw.theme', root.dataset.theme);
  app.viewport.redraw();
}

/// A short message at the bottom (kept as the status bar's last message); a saved file's can show it in its folder.
function toast(text, { path } = {}) {
  const reveal = path && platform.DESKTOP ? () => platform.reveal(path).catch((e) => toast(String(e.message || e))) : null;
  app.lastMessage = { text: String(text), path: reveal ? path : null, at: new Date() };
  (app.messages ||= []).unshift(app.lastMessage);
  app.messages.length = Math.min(app.messages.length, 30);
  if (app.state) renderStatus();
  const t = h('div', { class: 'toast', role: 'status', style: reveal ? 'display: flex; gap: 12px; align-items: center' : '' }, text,
    reveal ? h('button', { class: 'btn', style: 'height: 30px', onClick: () => { reveal(); t.remove(); } }, 'Show in folder') : null);
  // Above any toast already showing.
  const up = [...document.querySelectorAll('.toast')].reduce((y, x) => Math.max(y, window.innerHeight - x.getBoundingClientRect().top), 0);
  if (up) t.style.bottom = `${up + 8}px`;
  document.body.append(t);
  setTimeout(() => t.remove(), reveal ? 6000 : 3200);
}
app.problem = (e) => { platform.log(`render: ${e?.message || e}`); };

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

// ----- Keys -----

let spacePanned = false;
/// Every command with a shortcut: id, what it does, its built-in keys, and what it runs.
/// Settings › Keys can give each other keys (kept in this browser / on this computer).
const COMMANDS = [
  ['undo', 'Undo', 'Ctrl+Z', () => app.run('undo')],
  ['redo', 'Redo', 'Ctrl+Shift+Z', () => app.run('redo')],
  ['redo2', 'Redo (also)', 'Ctrl+Y', () => app.run('redo')],
  ['save', 'Save', 'Ctrl+S', () => save(false)],
  ['save_as', 'Save as', 'Ctrl+Shift+S', () => save(true)],
  ['open', 'Open', 'Ctrl+O', () => openCanvas()],
  ['new', 'New canvas', 'Ctrl+N', () => showNew()],
  ['close', 'Close tab', CLOSE_KEYS, () => closeTab(engineTabs.front())],
  ['quit', 'Quit', QUIT_KEYS, () => { if (platform.DESKTOP || servedHere) quit(); }],
  ['export', 'Export', 'Ctrl+E', () => openExport()],
  ['import', 'Import', 'Ctrl+I', () => importFile()],
  ['print', 'Print', 'Ctrl+P', () => printFrame()],
  ['settings', 'Settings', 'Ctrl+,', () => openSettings()],
  ['assistant', 'AI assistant', 'Ctrl+K', () => showSide('ai')],
  ['commands', 'Find a command', ['Ctrl+Space', 'F3'], () => commandSearch()],
  ['rulers', 'Rulers', 'Ctrl+R', () => { app.prefs.showRulers = !app.prefs.showRulers; app.savePrefs(); }],
  ['grid', 'Grid', "Ctrl+'", () => { app.prefs.showGrid = !app.prefs.showGrid; app.savePrefs(); }],
  ['guides', 'Guides', 'Ctrl+;', () => { app.prefs.showGuides = !app.prefs.showGuides; app.savePrefs(); }],
  ['duplicate', 'Duplicate', 'Ctrl+D', () => duplicateSel()],
  ['select_all', 'Select all', 'Ctrl+A', () => selectAll()],
  ['group', 'Group', 'Ctrl+G', () => app.run('group')],
  ['ungroup', 'Ungroup', 'Ctrl+Shift+G', () => app.run('ungroup')],
  ['clip', 'Clipping mask', 'Ctrl+7', () => app.run('clip', { make: true })],
  ['unclip', 'Release clipping mask', 'Ctrl+Alt+7', () => app.run('clip', { make: false })],
  ['symbol', 'Convert to symbol', 'Ctrl+Shift+C', () => app.run('precompose')],
  ['front', 'Bring to front', 'Ctrl+Shift+]', () => app.run('arrange', { to: 'front' })],
  ['back', 'Send to back', 'Ctrl+Shift+[', () => app.run('arrange', { to: 'back' })],
  ['forward', 'Bring forward', 'Ctrl+]', () => app.run('arrange', { to: 'forward' })],
  ['backward', 'Send backward', 'Ctrl+[', () => app.run('arrange', { to: 'backward' })],
  ['fit', 'Fit on screen', ['Ctrl+0', 'Shift+1'], () => app.viewport.fit()],
  ['actual', 'Actual size', 'Ctrl+1', () => app.viewport.zoomTo(1)],
  ['zoom_in', 'Zoom in', 'Ctrl+=', () => app.viewport.zoomTo(app.viewport.k * 1.25)],
  ['zoom_out', 'Zoom out', 'Ctrl+-', () => app.viewport.zoomTo(app.viewport.k / 1.25)],
  ['shortcuts', 'Keyboard shortcuts', 'Ctrl+/', () => shortcuts()],
  ['writing', 'Spelling and grammar', 'F7', () => showSide('writing')],
  ['present', 'Present', 'F5', () => slides.present(0)],
  ['present_here', 'Present from this slide', 'Shift+F5', () => slides.present(slides.current()?.index || 0)],
  ['present_on', 'Present on another screen', 'Alt+F5', () => slides.presentOnDialog(slides.current()?.index || 0)],
  ['rename', 'Rename layer', 'F2', () => { if (one()) renameLayer(one().id); }],
  ['delete', 'Delete', 'Delete', () => { if (!(tl.selKeys.size && tl.hovered && tl.removeSelectedKeys())) app.run('delete'); }],
  ['delete2', 'Delete (also)', 'Backspace', () => { if (!(tl.selKeys.size && tl.hovered && tl.removeSelectedKeys())) app.run('delete'); }],
  ['next_layer', 'Select next layer', 'Tab', () => selectStep(1)],
  ['prev_layer', 'Select previous layer', 'Shift+Tab', () => selectStep(-1)],
  ['next_object', 'Select the next object in reading order', 'Alt+]', () => readingStep(1)],
  ['prev_object', 'Select the previous object in reading order', 'Alt+[', () => readingStep(-1)],
  ['start', 'Go to start', 'Home', () => app.setTime(0)],
  ['end', 'Go to the end', 'End', () => app.setTime(app.state.comp.duration)],
  ['prev_key', 'Previous keyframe', 'J', () => tl.jumpKey(-1)],
  ['next_key', 'Next keyframe', 'K', () => tl.jumpKey(1)],
  ['frame_back', 'Back a frame', ',', () => tl.stepFrame(-1)],
  ['frame_fwd', 'Forward a frame', '.', () => tl.stepFrame(1)],
  ['frame_back2', 'Back a frame (also)', 'PageUp', () => tl.stepFrame(-1)],
  ['frame_fwd2', 'Forward a frame (also)', 'PageDown', () => tl.stepFrame(1)],
  ['pause_marker', 'Add or remove a pause at the playhead', 'Ctrl+Shift+P', () => app.run('toggle_pause', {})],
  ['position_size', 'Position and size', 'Ctrl+T', () => positionSizeDialog()],
  ['fill_or_stroke', 'Switch between fill and stroke', 'X', () => {
    app.activeSlot = app.activeSlot === 'stroke' ? 'fill' : 'stroke';
    if (app.eyeInto === 'fill' || app.eyeInto === 'stroke') app.eyeInto = app.activeSlot;
    renderToolbar();
  }],
  ['swap_colours', 'Swap the fill and stroke colours', 'Shift+X', () => swapColours()],
  ['reset_colours', 'White fill, black stroke', 'D', () => resetColours()],
  ['brush_smaller', 'Smaller brush', '[', () => resizeBrush(0.8)],
  ['brush_bigger', 'Bigger brush', ']', () => resizeBrush(1.25)],
  ['rotate_cw', 'Rotate 90° right', 'Ctrl+Shift+ArrowRight', () => { if (sel().length) app.run('rotate_by', { degrees: 90 }); }],
  ['rotate_ccw', 'Rotate 90° left', 'Ctrl+Shift+ArrowLeft', () => { if (sel().length) app.run('rotate_by', { degrees: -90 }); }],
  ['rotate_180', 'Rotate 180°', '', () => { if (sel().length) app.run('rotate_by', { degrees: 180 }); }],
  ['flip_h', 'Flip horizontally', 'Shift+H', () => { if (sel().length) app.run('flip', { axis: 'horizontal' }); }],
  ['flip_v', 'Flip vertically', 'Shift+V', () => { if (sel().length) app.run('flip', { axis: 'vertical' }); }],
  ['grid', 'Grid', "Ctrl+'", () => { app.prefs.showGrid = !app.prefs.showGrid; app.savePrefs(); }],
  ['new_slide', 'New slide', 'Ctrl+M', () => slides.add()],
  ['duplicate_slide', 'Duplicate slide', '', () => { const cur = slides.current(); if (cur) { app.run('duplicate_slide', { index: cur.index }); slides.refresh(); } }],
  ['delete_slide', 'Delete slide', '', () => { const cur = slides.current(); if (cur && slides.list().list.length > 1) { app.run('delete_slide', { index: cur.index }); slides.refresh(); } }],
  // Arrows nudge the selection (Shift: 10 px); with nothing selected they step frames.
  ['nudge_left', 'Nudge left', ['ArrowLeft', 'Shift+ArrowLeft'], () => nudgeKey(-1, 0)],
  ['nudge_right', 'Nudge right', ['ArrowRight', 'Shift+ArrowRight'], () => nudgeKey(1, 0)],
  ['nudge_up', 'Nudge up', ['ArrowUp', 'Shift+ArrowUp'], () => nudgeKey(0, -1)],
  ['nudge_down', 'Nudge down', ['ArrowDown', 'Shift+ArrowDown'], () => nudgeKey(0, 1)],
  ['ai_new_slide', 'New slide with AI', 'Ctrl+Alt+M', () => aiSlideDialog(app, h, 'new_slide', aiCtx())],
  ['ai_modify_slide', 'Modify this slide with AI', 'Ctrl+Alt+E', () => aiSlideDialog(app, h, 'modify_slide', aiCtx())],
];
app.commands = COMMANDS;
app.ui = { popup, item, heading, modal, closePopup, toast, sub };
app.newTab = (title, setup) => newTab(title, setup);
app.openSettings = (tab) => openSettings(tab);
app.closeTab = (id) => closeTab(id);
app.renameLayer = (id) => renameLayer(id);
app.pensDialog = () => pensDialog(app, h);

/// The keys of an event as written in the shortcut lists ("Ctrl+Shift+Z", "F5", "Ctrl+]").
function comboOf(e) {
  let key = e.code?.startsWith('Key') ? e.code.slice(3) : e.code?.startsWith('Digit') ? e.code.slice(5)
    : e.code === 'BracketRight' ? ']' : e.code === 'BracketLeft' ? '[' : e.code === 'Minus' ? '-' : e.code === 'Equal' ? '='
    : e.code === 'Comma' ? ',' : e.code === 'Slash' ? '/' : e.code === 'Space' ? 'Space' : e.code === 'Semicolon' ? ';' : e.code === 'Quote' ? "'" : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(key)) return '';
  return [(e.ctrlKey || e.metaKey) && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', key].filter(Boolean).join('+');
}

/// Each command's keys now (the built-in ones unless changed in Settings).
function shortcutMap() {
  let custom = {};
  try { custom = JSON.parse(localStorage.getItem('draw.shortcuts') || '{}'); } catch { /* none */ }
  return Object.fromEntries(COMMANDS.map(([id, , keys]) => [id, keysOf(custom[id] ?? keys)]));
}
/// A command's keys as a list (a command can have several; Settings keeps one or a list).
function keysOf(v) {
  return Array.isArray(v) ? v.filter(Boolean) : v ? [v] : [];
}
app.keysOf = keysOf;
app.shortcutMap = shortcutMap;
app.comboOf = (e) => comboOf(e);
/// A built-in combination as currently bound (for menus written with the built-in keys).
const shown = (keys) => {
  const c = COMMANDS.find((x) => x[2] === keys);
  return c ? (app.shortcuts?.[c[0]] ? app.shortcuts[c[0]][0] || '' : keys) : keys;
};

window.addEventListener('keydown', (e) => {
  if (isTyping(e) || document.querySelector('.modal-back') || newScreen) return;
  const ctrl = e.ctrlKey || e.metaKey;
  const k = e.key.toLowerCase();
  const go = (f) => { e.preventDefault(); f(); };
  // Keys that mean something only where you are (points, crop, typing) come first.
  if (app.tool === 'node' && !ctrl && app.viewport.points.key(e)) { e.preventDefault(); return; }
  if (app.tool === 'mask' && !ctrl && app.viewport.maskPoints.key(e)) { e.preventDefault(); return; }
  if (app.raster?.key(e)) { e.preventDefault(); return; }
  const combo = comboOf(e);
  const map = app.shortcuts || (app.shortcuts = shortcutMap());
  const hit = COMMANDS.find(([id]) => map[id]?.includes(combo));
  if (hit) return go(hit[3]);
  if (ctrl) return;
  if (e.key === 'Enter' && app.tool === 'crop' && app.viewport.cropRect) return go(() => cropCanvas());
  if (e.key === 'Escape' && app.tool === 'crop' && app.viewport.cropRect) return go(() => { app.viewport.cropRect = null; renderToolbar(); app.viewport.redraw(); });
  if (e.key === 'Escape') return go(() => { app.setTool('select'); app.select([]); });
  if (e.key === 'Enter' && one()?.kind === 'text' && app.tool === 'select') return go(() => app.viewport.beginEdit(one().id));
  if (e.code === 'Space') { spacePanned = false; return; }
  if (e.altKey) return;
  // On a raster layer the raster tools answer their letters first; elsewhere the vector ones.
  const onRaster = rasterMode();
  const bound = Object.entries(app.keys || toolKeys(ALL_TOOLS, SHAPES)).filter(([, key]) => key && key.toLowerCase() === k)
    .sort(([a], [b]) => (a.startsWith('r-') === onRaster ? 0 : 1) - (b.startsWith('r-') === onRaster ? 0 : 1))[0];
  if (bound) return go(() => (bound[0].startsWith('shape:') ? app.setTool('shape', bound[0].slice(6)) : app.setTool(bound[0])));
});
window.addEventListener('pointermove', (e) => { if (app.viewport?.space && e.buttons) spacePanned = true; });
window.addEventListener('keyup', (e) => {
  if (e.code !== 'Space' || isTyping(e) || document.querySelector('.modal-back') || newScreen) return;
  if (!spacePanned) app.togglePlay();
});

// ----- Find a command (Ctrl+Space or F3) -----

/// Everything the menus and tools can do, as { path, label, keys, enabled, run }: the menus as they are now
/// (submenus included), then the tools, then commands only a key reaches.
function allCommands() {
  const out = [];
  // A menu's headings ("Align to the canvas", "Path") count as part of the name, so "align left" finds "Left edges".
  const walk = (els, path) => {
    let head = null;
    for (const el of els.flat()) {
      if (!el?.classList) continue;
      if (el.classList.contains('menu-head')) { head = el.textContent; continue; }
      if (el.tagName === 'HR') { head = null; continue; }
      if (el.classList.contains('has-sub')) { walk(el.subItems?.() || [], [...path, el.firstChild.textContent]); continue; }
      if (!el.classList.contains('item')) continue;
      const label = el.firstChild.textContent;
      out.push({ path: head ? [...path, head] : path, label, keys: el.querySelector('.keys')?.textContent || '', enabled: !el.disabled, run: () => el.click() });
    }
  };
  for (const m of MENUS) { try { walk(m.items(), [m.label]); } catch { /* a menu that can't be built now */ } }
  for (const t of [...ALL_TOOLS, ...SHAPES.map((x) => ({ ...x, shape: true }))]) {
    out.push({ path: ['Tool'], label: t.label.split(':')[0], keys: t.key || '', enabled: true, run: () => (t.shape ? app.setTool('shape', t.id) : app.setTool(t.id)) });
  }
  const have = new Set(out.map((c) => c.label.toLowerCase()));
  for (const [id, label, , run] of COMMANDS) {
    if (id === 'commands' || have.has(label.toLowerCase())) continue;
    out.push({ path: ['Key'], label, keys: (app.shortcuts || shortcutMap())[id]?.[0] || '', enabled: true, run });
  }
  return out;
}

function commandSearch() {
  if (document.querySelector('.cmd-search')) return;
  closePopup();
  const all = allCommands();
  let recent = [];
  try { recent = JSON.parse(localStorage.getItem('draw.recentCommands') || '[]'); } catch { /* none */ }
  const name = (c) => [...c.path, c.label].join(' › ');
  const input = h('input', { class: 'field', type: 'search', placeholder: 'Type what you want to do: align left, knife, export, new layer…', 'aria-label': 'Find a command', autocomplete: 'off', spellcheck: 'false' });
  const list = h('div', { class: 'cmd-list', role: 'listbox', 'aria-label': 'Commands' });
  let shown = [], at = 0;
  const back = h('div', { class: 'modal-back cmd-back' });
  const close = () => back.remove();
  const run = (c) => {
    if (!c?.enabled) return;
    close();
    try { localStorage.setItem('draw.recentCommands', JSON.stringify([name(c), ...recent.filter((r) => r !== name(c))].slice(0, 8))); } catch { /* not kept */ }
    c.run();
  };
  const draw = () => {
    const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) shown = [...recent.map((r) => all.find((c) => name(c) === r)).filter(Boolean), ...all.filter((c) => !recent.includes(name(c)))];
    else {
      // Every word somewhere in the command's name; ones whose own label matches, and starts with the first word, first.
      shown = all.filter((c) => { const n = name(c).toLowerCase(); return words.every((w) => n.includes(w)); })
        .map((c) => { const l = c.label.toLowerCase(); return [c, (words.every((w) => l.includes(w)) ? 0 : 2) + (l.startsWith(words[0]) ? 0 : 1) + (c.enabled ? 0 : 4)]; })
        .sort((a, b) => a[1] - b[1]).map(([c]) => c);
    }
    shown = shown.slice(0, 60);
    at = Math.min(at, Math.max(0, shown.length - 1));
    list.replaceChildren(...(shown.length ? shown.map((c, i) => h('button', { class: `cmd${i === at ? ' on' : ''}`, role: 'option', 'aria-selected': String(i === at), disabled: !c.enabled,
      onPointerdown: (e) => e.preventDefault(), onClick: () => run(c) },
      h('span', { class: 'cmd-path' }, c.path.join(' › ')), h('span', { class: 'cmd-label' }, c.label), c.keys ? h('span', { class: 'keys' }, c.keys) : null))
      : [h('p', { class: 'muted', style: 'padding: 10px 12px; margin: 0' }, 'No command has that name.')]));
    list.querySelector('.cmd.on')?.scrollIntoView({ block: 'nearest' });
  };
  input.addEventListener('input', () => { at = 0; draw(); });
  input.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); close(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); at = Math.min(shown.length - 1, at + 1); draw(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); at = Math.max(0, at - 1); draw(); }
    else if (e.key === 'Enter') { e.preventDefault(); run(shown[at]); }
  });
  back.addEventListener('pointerdown', (e) => { if (e.target === back) close(); });
  back.append(h('div', { class: 'cmd-search', role: 'dialog', 'aria-label': 'Find a command' }, input, list,
    h('div', { class: 'muted cmd-foot' }, '↑ ↓ to pick · Enter runs it · Esc closes')));
  document.body.append(back);
  draw();
  input.focus();
}

// ----- Start -----

function timelineShown() {
  const still = !app.state?.comp.duration;
  // Hidden in View › Panels: neither the timeline nor the still picture's strip.
  const off = !!app.prefs.hidden?.timeline;
  $('timeline').hidden = still || off;
  // A still picture: a strip in the timeline's place, to make it move.
  let bar = $('still-bar');
  if (!bar) {
    bar = h('div', { id: 'still-bar', class: 'still-bar', role: 'region', 'aria-label': 'Timeline' },
      h('span', { html: icon('stopwatch', 16), style: 'display: inline-flex; color: var(--muted)' }),
      h('span', {}, 'A still picture: no timeline.'),
      h('button', { class: 'btn', title: 'Gives the canvas a 6-second timeline (change its length in Canvas settings)', onClick: () => animateStill() }, 'Animate it'));
    $('timeline').after(bar);
  }
  bar.hidden = !still || !app.state || off;
}

/// A phone: Draw is made for computers and tablets; it says so once, and
/// carries on if asked.
function phoneNote() {
  if (Math.min(screen.width, screen.height) >= 600 || window.innerWidth > 600 || new URLSearchParams(location.search).get('embed')) return;
  try { if (localStorage.getItem('draw.phoneOk')) return; } catch { /* asked each time */ }
  const note = h('div', { class: 'phone-note', role: 'dialog', 'aria-label': 'A small screen' },
    h('b', {}, 'Draw is made for computers and tablets'),
    h('span', {}, 'On a phone the canvas and its panels don’t fit well. You can still look at a canvas, change a little, or open Details › AI to make a picture by describing it.'),
    h('div', { class: 'acts' },
      servedHere ? h('button', { class: 'btn', onClick: () => quit() }, 'Back to Files') : null,
      h('button', { class: 'btn main', onClick: () => { try { localStorage.setItem('draw.phoneOk', '1'); } catch { /* */ } note.remove(); } }, 'Continue')));
  document.body.append(note);
}

/// A still canvas gets a timeline (6 s), to animate.
function animateStill() {
  app.run('set_comp', { duration: 6 });
  toast('This canvas now has a 6-second timeline. Canvas settings changes its length.');
}

const writing = new Writing(app);
const slides = new Slides(app, h, { popup, item, modal: (...a) => modal(...a), soundtrack: () => soundtrackDialog() });
app.sound = new Sound(app);
const assistant = new Assistant(app, h, { openSettings: () => openSettings('server') });
app.assistant = assistant;
app.clipBytes = (id) => clipBytes(id);
app.slides = slides;
app.writing = writing;
let tl;

// In the browser on a Xenify server: the address names the server canvas
// in front (#open=<server>,<file>), so reloading reopens it and the link
// can be passed on; a canvas of this computer leaves it plain.
let addressFrom = false;
function syncAddress() {
  if (platform.DESKTOP || !addressFrom) return;
  const t = frontTab();
  const node = t?.remote ? server.nodeOf(t.id) : null;
  // This server's document: ?open=<file>; another server's: #open=<server>,<file>.
  const u = new URL(location.href);
  u.searchParams.delete('open');
  u.hash = '';
  // Saved to the server now: where New was asked for no longer matters.
  if (node != null) for (const k of ['new', 'space', 'parent']) u.searchParams.delete(k);
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
let leaving = false;
function showHome() {
  if (!/^https?:$/.test(location.protocol)) return;
  fetch('/v1/info').then((r) => {
    servedHere = r.ok && /json/.test(r.headers.get('content-type') || '');
    if (servedHere) buildMenus();
  }).catch(() => {});
}

async function main() {
  app.settings = loadSettings();
  applySettings(app.settings);
  app.autoKey = app.settings.autoKey;
  app.keys = toolKeys(ALL_TOOLS, SHAPES);
  const saved = localStorage.getItem('draw.theme');
  if (saved) document.documentElement.dataset.theme = saved;
  setUpWindow();
  buildMenus();
  $('new-tab').innerHTML = icon('plus');
  $('new-tab').addEventListener('click', () => showNew());
  try {
    await start($('gl'));
  } catch (e) {
    document.body.replaceChildren(h('div', { class: 'screen' }, h('div', { class: 'screen-in' },
      h('h1', {}, 'Xenify Draw can’t start here'),
      h('p', { style: 'font-size: 16px; line-height: 1.5; max-width: 640px' }, 'Drawing needs WebGL2, which this browser or computer has turned off. Try a current Firefox, Chrome, Edge or Safari, or turn on hardware acceleration in the browser’s settings.'),
      h('p', { style: 'color: var(--muted)' }, String(e.message || e)))));
    platform.log(`start: ${e.stack || e}`);
    document.documentElement.classList.add('ready');
    return;
  }
  try { app.aiModelSets = call('ai_models'); app.aiModels = app.aiModelSets.brain; app.aiDefaults = { reviewer: app.aiModelSets.default_reviewer }; } catch { app.aiModels = []; }
  // The first tab is an empty canvas (behind the New canvas screen until something is picked).
  call('new', { name: 'Untitled' });
  app.tabs = [{ id: engineTabs.front(), title: 'Untitled', path: null, dirty: false }];
  app.viewport = new Viewport({ stage: $('stage'), gl: $('gl'), overlay: $('overlay'), textEdit: $('text-edit'), perf: $('perf'), app });
  app.raster = initRaster(app);
  tl = new Timeline($('timeline'), app, h);
  if (localStorage.getItem('draw.perf') === '1' || new URLSearchParams(location.search).has('perf')) $('perf').hidden = false;
  app.changed();
  requestAnimationFrame(() => app.viewport.fit());
  writing.run();
  takeOverColourFields({ pickFromCanvas: () => new Promise((resolve) => { app.pickOnce = resolve; toast('Click the canvas to pick its colour (Esc: cancel).'); }) });
  await fonts.restore();
  await platform.loadSecrets();
  panelResizers();
  timelineResizer();
  numberScrubbing();
  // Full-window screens (New canvas, Settings…) start below the menu bar, which stays usable (and moves the window).
  const bar = document.querySelector('.menubar');
  if (bar) new ResizeObserver(() => document.documentElement.style.setProperty('--menubar-h', `${bar.offsetHeight}px`)).observe(bar);
  // The New canvas screen sits below the tabs (it's one of them).
  const strip = document.querySelector('.tabstrip');
  if (strip) new ResizeObserver(() => document.documentElement.style.setProperty('--tabs-bottom', `${strip.getBoundingClientRect().bottom}px`)).observe(strip);
  applyLayout();
  document.documentElement.classList.add('ready');
  showHome();
  // Ctrl+Tab / Ctrl+Shift+Tab: the next or previous tab (before anything else sees Tab).
  window.addEventListener('keydown', (e) => { if (e.ctrlKey && e.key === 'Tab') { e.preventDefault(); e.stopPropagation(); cycleTab(e.shiftKey ? -1 : 1); } }, true);
  // Closing the window asks about unsaved canvases first.
  const tw = platform.DESKTOP && window.__TAURI__?.window?.getCurrentWindow?.();
  if (tw) tw.onCloseRequested((e) => { e.preventDefault(); quit(); });
  else window.addEventListener('beforeunload', (e) => { if (!leaving && app.tabs.some(isDirty)) { e.preventDefault(); e.returnValue = ''; } });
  if (!platform.DESKTOP) {
    platform.loadSpotter();
    keepOffline(async () => {
      // On a Xenify server Draw follows the server: a version found just
      // after opening, with nothing unsaved, takes over at once.
      const early = performance.now() < 20000;
      if (early && (await onServer()) && !app.tabs.some(isDirty)) return applyUpdate();
      const t = h('div', { class: 'toast', role: 'status', style: 'display: flex; gap: 12px; align-items: center' }, 'Update ready',
        h('button', { class: 'btn', style: 'height: 30px', onClick: () => applyUpdate() }, 'Restart Draw'));
      document.body.append(t);
    });
  }
  // Canvases named on the command line, else what the browser kept, else the New canvas screen.
  const opened = await platform.openedFiles();
  for (const f of opened) (f.name.endsWith('.xboard') ? openBoard(f) : openText(f));
  let reopened = [];
  if (!opened.length && platform.DESKTOP && app.settings.reopenLast) {
    try { reopened = JSON.parse(localStorage.getItem('draw.lastOpen') || '[]'); } catch { /* none */ }
    for (const r of reopened) await openRecent(r);
  }
  app.ready = true;
  if (!opened.length && !reopened.length) {
    let kept = null;
    try { kept = JSON.parse(localStorage.getItem('draw.autosave') || 'null'); } catch { /* none */ }
    // (Opened for a canvas on the server: that one, not the copy kept here; for an
    // example: that, unless the copy kept here has unsaved changes.)
    if (kept && !(kept.dirty === false && new URLSearchParams(location.search).has('example')) && !platform.DESKTOP && new URLSearchParams(location.search).get('start') !== 'new' && !/^#open=/.test(location.hash) && !new URLSearchParams(location.search).has('open')) {
      // Back where it was left, in the first tab.
      try {
        call('open', { json: kept.text });
        // (Untouched or already saved: back as it was, not as unsaved changes.)
        if (kept.dirty !== false) call('mark_unsaved');
        app.tabs[0].title = kept.name;
        app.lastStructure = '';
        app.changed();
        flatten3d();
        app.viewport.fit();
        writing.run();
        loadFonts();
      } catch { /* unreadable: the empty canvas stays */ }
    } else if (new URLSearchParams(location.search).get('start') === 'new' || (platform.DESKTOP && app.settings.showStart)) showNew();
  }
  // An example linked from the site: ?example=<name>, &present to play it at once. (In
  // the address still: the first visit reloads once the offline copy is kept. What you
  // changed comes back in a tab of its own, as above.)
  const example = new URLSearchParams(location.search).get('example');
  if (example) await openExample(example, new URLSearchParams(location.search).has('present'));
  if (platform.DESKTOP) offerRecovery();
  phoneNote();
  window.XENIFY_READY = true;
  // Inside Xenify Chat as a chat's whiteboard (embed.js).
  if (new URLSearchParams(location.search).get('embed') === 'board') {
    const embed = await import('./embed.js');
    embed.start({ app, server, call, applyLayout, showShapes, renderTabs });
    return;
  }
  // Opened from a Xenify server (the browser version): use that server, and
  // open #open=<server>,<canvas> from its Files page.
  server.connectHere().then((yes) => {
    // ?open=<file> (this server's) or #open=<server>,<file> (read before
    // the tabs are drawn, which writes the address).
    const here = new URLSearchParams(location.search).get('open');
    const linked = /^\d+$/.test(here || '') ? [null, location.origin, here] : /^#open=([^,]+),(\d+)$/.exec(location.hash);
    addressFrom = !!yes;
    if (yes && !linked) renderTabs();
    // New › Drawing in Files: pick its size and kind first.
    if (!linked && new URLSearchParams(location.search).has('new')) showNew();
    if (linked) {
      // Opened for this canvas: the blank one Draw starts with goes.
      const blank = app.tabs.find((t) => !t.remote && !t.path && !isDirty(t));
      openLinked(decodeURIComponent(linked[1]), +linked[2]).then(async (ok) => {
        if (ok && blank && app.tabs.length > 1 && app.tabs.some((t) => t.id === blank.id)) await closeTab(blank.id);
        renderTabs();
      });
    }
  }).catch(() => {});
  if (window.XENIFY_TEST) window.XENIFY_TEST().catch((e) => platform.log(`test: ${e.stack || e}`));
  if (window.XENIFY_BENCH || new URLSearchParams(location.search).has('bench')) {
    const r = await speedTest({ quiet: true });
    window.benchOut = r;
    if (!platform.DESKTOP) fetch('/log', { method: 'POST', body: JSON.stringify(r) }).catch(() => {});
    // And that spelling and grammar answer on this platform.
    try {
      const found = await platform.check([{ id: 1, text: 'Bring you frends on Saturdy.' }], { profile: writing.kind });
      platform.log(`checker: ${JSON.stringify(found.map((i) => [i.category, i.suggestions.slice(0, 3)]))}`);
    } catch (e) { platform.log(`checker failed: ${e.message || e}`); }
  }
}

main().catch((e) => { document.documentElement.classList.add('ready'); platform.log(`main: ${e.stack || e}`); });
