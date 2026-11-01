// The timeline: play, the playhead, each layer's span with its keyframes,
// and the selected layer's animated properties with keyframes you can drag.

import { icon } from './icons.js';
import { Graph } from './graph.js';
import { call } from './engine.js';
import { curveDialog } from './curve.js';

const NAMES = { position: 'Position', scale: 'Scale', rotation: 'Rotation', opacity: 'Opacity', anchor: 'Anchor',
  font_size: 'Text size', text_color: 'Text colour', fill: 'Fill', stroke: 'Stroke', stroke_width: 'Stroke width', size: 'Size', corner_radius: 'Corners' };

export function propName(p) {
  return NAMES[p] || p;
}

export function fmtTime(t) {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(2).padStart(5, '0')}`;
}

const NAME_W = 220;
const keyId = (k) => `${k.id}|${k.prop}|${k.time}`;

/// Seconds and frames: "0:03.00 · frame 90".
export function timecode(t, fps) {
  return `${fmtTime(t)} · frame ${Math.round(t * fps)}`;
}

export class Timeline {
  constructor(el, app, h) {
    this.el = el;
    this.app = app;
    this.h = h;
    this.zoom = 1;
    this.onlyAnimated = false;
    this.collapsed = new Set();
    this.selKeys = new Map();
    this.waves = new Map();
    this.hovered = false;
    this.build();
  }

  build() {
    const { h, app } = this;
    this.timeEl = h('span', { class: 'tl-time', title: 'Seconds, and the frame number' });
    this.playBtn = h('button', { class: 'icon-btn play', 'aria-label': 'Play (Space)', title: 'Play (Space)', onClick: () => app.togglePlay() });
    this.loopBtn = h('button', { class: 'icon-btn', 'aria-label': 'Loop', title: 'Loop', 'aria-pressed': 'true', html: icon('loop'),
      onClick: () => { app.loop = !app.loop; this.loopBtn.setAttribute('aria-pressed', String(app.loop)); } });
    this.zoomIn = h('input', { type: 'range', min: '0.5', max: '12', step: '0.1', value: '1', 'aria-label': 'Timeline zoom', style: 'width: 110px; accent-color: var(--accent)',
      onInput: (e) => { this.zoom = Number(e.target.value); this.render(); } });
    // How long the canvas runs: typed or dragged here (also Details › Canvas).
    this.lenIn = h('input', { class: 'field tiny tl-len-in', type: 'number', min: '0.1', max: '3600', step: '0.5', 'aria-label': 'Length in seconds',
      onChange: (e) => { const v = Number(e.target.value); if (v > 0) app.run('set_comp', { duration: Math.min(3600, v) }); else e.target.value = String(+app.state.comp.duration.toFixed(2)); },
      onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') e.target.blur(); } });
    this.lenEl = h('label', { class: 'tl-len', title: 'How long the animation runs, in seconds: type it, or drag the end of the timeline' }, h('span', {}, 'Length'), this.lenIn, h('small', { class: 'unit' }, 's'));
    const btn = (iconName, label, run) => h('button', { class: 'icon-btn', 'aria-label': label, title: label, html: icon(iconName), onClick: run });
    this.onlyBtn = h('button', { class: 'pick', 'aria-pressed': 'false', title: 'Show only the layers that have keyframes', onClick: () => { this.onlyAnimated = !this.onlyAnimated; this.onlyBtn.setAttribute('aria-pressed', String(this.onlyAnimated)); this.render(); } }, 'Only animated');
    this.bar = h('div', { class: 'tl-bar' },
      btn('start', 'Go to start (Home)', () => app.setTime(0)),
      btn('left', 'Back a frame (, or Page Up)', () => this.stepFrame(-1)),
      this.playBtn,
      btn('right', 'Forward a frame (. or Page Down)', () => this.stepFrame(1)),
      btn('end', 'Go to the end (End)', () => app.setTime(app.state.comp.duration)),
      this.loopBtn, this.timeEl, this.lenEl,
      h('span', { class: 'sep' }),
      h('button', { class: 'pick', title: 'New values on animated properties make keyframes', onClick: () => app.toggleAutoKey() },
        this.autoLabel = h('span', {}, 'Auto keyframes on')),
      this.keysBtn = h('button', { class: 'pick', 'aria-haspopup': 'menu', title: 'Select, key and ease keyframes', onClick: (e) => this.keysMenu(e.currentTarget) }, 'Keys'),
      this.graphBtn = h('button', { class: 'pick', 'aria-pressed': 'false', title: 'Show the selected layer’s animation as curves', onClick: () => this.toggleGraph() }, 'Graph'),
      this.onlyBtn,
      h('div', { class: 'spacer' }),
      h('button', { class: 'pick', title: 'Fit the whole timeline in view', onClick: () => this.fit() }, 'Fit'),
      h('span', { style: 'font-size: 13px; color: var(--muted)' }, 'Zoom'), this.zoomIn,
      h('span', { class: 'sep' }),
      this.minBtn = h('button', { class: 'icon-btn', onClick: () => app.minTimeline() }),
      btn('x', 'Hide the timeline (View › Panels brings it back)', () => app.hideTimeline()));
    this.graph = new Graph(app, h);
    this.body = h('div', { class: 'tl-body', tabindex: '-1' });
    this.el.replaceChildren(this.bar, this.body);
    this.body.addEventListener('contextmenu', (e) => this.context(e));
    this.body.addEventListener('pointerdown', (e) => this.down(e));
    this.body.addEventListener('dblclick', (e) => {
      const key = e.target.closest('[data-key]');
      if (key) { const k = JSON.parse(key.dataset.key); this.selKeys.delete(keyId(k)); app.run('remove_key', { id: k.id, prop: k.prop, time: k.time }); }
    });
    this.el.addEventListener('pointerenter', () => { this.hovered = true; });
    this.el.addEventListener('pointerleave', () => { this.hovered = false; });
    // Ctrl + wheel zooms around the pointer; Shift + wheel scrolls sideways.
    this.body.addEventListener('wheel', (e) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const r = this.body.getBoundingClientRect();
      const L = this.lane();
      const atX = e.clientX - r.left - NAME_W + this.body.scrollLeft;
      const t = (atX / L.w) * L.dur;
      this.zoom = Math.max(0.5, Math.min(12, this.zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
      this.zoomIn.value = String(this.zoom);
      this.render();
      const L2 = this.lane();
      this.body.scrollLeft = Math.max(0, L2.x(t) - (e.clientX - r.left - NAME_W));
    }, { passive: false });
  }

  fit() {
    this.zoom = 1;
    this.zoomIn.value = '1';
    this.body.scrollLeft = 0;
    this.render();
  }

  /// Lane width in px and seconds → x.
  lane() {
    const w = Math.max(200, (this.body.clientWidth - NAME_W) * this.zoom);
    const dur = Math.max(0.1, this.app.state?.comp?.duration || 1);
    return { w, dur, x: (t) => (t / dur) * w };
  }

  fps() {
    return this.app.state?.comp?.fps || 30;
  }

  stepFrame(n) {
    const fps = this.fps();
    const t = Math.round(this.app.state.time * fps + n) / fps;
    this.app.setTime(Math.max(0, Math.min(this.app.state.comp.duration, t)));
  }

  /// Every key time of the selected layers (or all layers), sorted.
  keyTimes() {
    const st = this.app.state;
    const sel = st.selection;
    const layers = st.layers.filter((l) => !sel.length || sel.includes(l.id));
    return [...new Set(layers.flatMap((l) => l.keys))].sort((a, b) => a - b);
  }

  /// J / K: the previous or next keyframe of the selected layers.
  jumpKey(dir) {
    const t = this.app.state.time, times = this.keyTimes();
    const next = dir > 0 ? times.find((k) => k > t + 1e-6) : [...times].reverse().find((k) => k < t - 1e-6);
    if (next != null) this.app.setTime(next);
  }

  selectedKeys() {
    return [...this.selKeys.values()];
  }

  selectKeys(list, add = false) {
    if (!add) this.selKeys.clear();
    for (const k of list) this.selKeys.set(keyId(k), k);
    this.render();
  }

  /// Every keyframe (of `ids`, or of the whole canvas).
  selectAllKeys(ids) {
    this.selectKeys(call('all_keys', ids ? { ids } : {}));
    this.app.toast?.(`Selected ${this.selKeys.size} keyframes.`);
  }

  removeSelectedKeys() {
    const keys = this.selectedKeys();
    if (!keys.length) return false;
    this.selKeys.clear();
    this.app.run('remove_keys', { keys });
    return true;
  }

  copyKeys() {
    const keys = this.selectedKeys();
    if (!keys.length) return false;
    this.app.keyClip = call('copy_keys', { keys });
    this.app.toast?.(keys.length === 1 ? 'Copied 1 keyframe.' : `Copied ${keys.length} keyframes.`);
    return true;
  }

  pasteKeys() {
    if (!this.app.keyClip?.length) return false;
    const got = this.app.run('paste_keys', { clip: this.app.keyClip });
    if (got) this.app.toast?.(`Pasted ${got.pasted} keyframe${got.pasted === 1 ? '' : 's'} at the playhead${got.skipped ? ` (${got.skipped} didn’t fit this layer)` : ''}.`);
    return true;
  }

  /// The easing all `keys` share (its name), or null.
  sharedEasing(keys) {
    let name;
    for (const k of keys) {
      let e;
      try { e = call('key_easings', { id: k.id, prop: k.prop }).find((x) => Math.abs(x.time - k.time) < 1e-6); } catch { return null; }
      if (!e) return null;
      if (name === undefined) name = e;
      else if (JSON.stringify(name.easing) !== JSON.stringify(e.easing)) return null;
    }
    return name || null;
  }

  /// Easing for `keys`: the ready-made ones, and the curve editor.
  easingItems(keys) {
    const { app } = this;
    const { item, heading } = app.ui;
    const list = app.easingList || (app.easingList = call('easings'));
    const now = this.sharedEasing(keys);
    return [
      heading(keys.length === 1 ? 'Easing to the next keyframe' : `Easing for ${keys.length} keyframes`),
      ...list.map((x) => item(easingLabel(x.name), { checked: now?.name === x.name, run: () => app.run('set_easings', { keys, easing: x.name }) })),
      item('Edit curve…', { run: async () => {
        const e = await curveDialog(app, this.h, now?.easing, { presets: list, label: easingLabel });
        if (e) app.run('set_easings', { keys, easing: e });
      } }),
    ];
  }

  keysMenu(anchor) {
    const { app, h } = this;
    const { item, popup } = app.ui;
    const sel = app.state.selection;
    const at = timecode(app.state.time, this.fps());
    const keys = this.selectedKeys();
    popup(anchor, [
      item('Select all keyframes', { keys: 'Ctrl+A', run: () => this.selectAllKeys() }),
      item('Select keyframes of the selected layers', { enabled: sel.length > 0, run: () => this.selectAllKeys(sel) }),
      h('hr'),
      item(`Key all properties at ${at}`, { enabled: sel.length > 0, run: () => { if (!app.run('key_all', { ids: sel, all: true })) app.toast?.('Nothing to key.'); } }),
      item(`Key animated properties at ${at}`, { enabled: sel.length > 0, run: () => { if (app.run('key_all', { ids: sel, all: false }) === false) app.toast?.('None of the selected layers’ properties are animated.'); } }),
      h('hr'),
      item('Copy keyframes', { keys: 'Ctrl+C', enabled: keys.length > 0, run: () => this.copyKeys() }),
      item('Paste keyframes at the playhead', { keys: 'Ctrl+V', enabled: !!app.keyClip?.length && sel.length === 1, run: () => this.pasteKeys() }),
      item('Remove the selected keyframes', { keys: 'Delete', enabled: keys.length > 0, run: () => this.removeSelectedKeys() }),
      h('hr'),
      item(`Add or remove a pause at ${at}`, { keys: 'Ctrl+Shift+P', run: () => app.run('toggle_pause', {}) }),
      ...(keys.length ? [h('hr'), ...this.easingItems(keys)] : [h('div', { class: 'menu-note' }, 'Select keyframes to set their easing.')]),
    ]);
  }

  toggleGraph() {
    this.showGraph = !this.showGraph;
    this.graphBtn.setAttribute('aria-pressed', String(this.showGraph));
    this.el.classList.toggle('graphing', this.showGraph);
    this.render();
  }

  update() {
    const st = this.app.state;
    if (!st) return;
    if (this.showGraph) this.graph.draw();
    this.timeEl.replaceChildren(fmtTime(st.time), this.h('span', {}, ` · frame ${Math.round(st.time * this.fps())}`));
    if (document.activeElement !== this.lenIn) this.lenIn.value = String(+st.comp.duration.toFixed(2));
    this.playBtn.innerHTML = icon(this.app.playing ? 'pause' : 'play');
    this.playBtn.setAttribute('aria-label', this.app.playing ? 'Pause (Space)' : 'Play (Space)');
    const min = !!this.app.prefs?.tlMin, minLabel = min ? 'Show the layer rows' : 'Only the bar (more room for the canvas)';
    if (this.minBtn.title !== minLabel) Object.assign(this.minBtn, { title: minLabel, innerHTML: icon(min ? 'up' : 'down') }), this.minBtn.setAttribute('aria-label', minLabel);
    this.autoLabel.textContent = this.app.autoKey ? 'Auto keyframes on' : 'Auto keyframes off';
    if (this.playhead) this.playhead.style.left = `${NAME_W + this.lane().x(st.time)}px`;
  }

  /// The rows shown: collapsed groups hide what's in them; "Only animated" hides layers without keys.
  visibleLayers() {
    const out = [];
    let hideBelow = null;
    for (const l of this.app.state.layers) {
      if (hideBelow != null && l.depth > hideBelow) continue;
      hideBelow = null;
      if (this.collapsed.has(l.id)) hideBelow = l.depth;
      if (this.onlyAnimated && !l.keys.length && !(l.kind === 'group' && this.app.state.layers.some((x) => x.depth > l.depth && x.keys.length))) continue;
      out.push(l);
    }
    return out;
  }

  render() {
    const { h, app } = this;
    const st = app.state;
    if (!st) return;
    if (this.showGraph) {
      this.body.replaceChildren(this.graph.el);
      this.graph.draw();
      this.update();
      return;
    }
    // Keys that no longer exist drop out of the selection.
    if (this.selKeys.size) {
      const live = new Set(call('all_keys').map(keyId));
      for (const k of [...this.selKeys.keys()]) if (!live.has(k)) this.selKeys.delete(k);
    }
    const L = this.lane();
    const rows = [];
    const width = `width: ${NAME_W + L.w}px`;
    // Seconds along the top (click or drag to move the playhead; right-click for pauses), and the pauses.
    const fps = this.fps();
    const step = L.dur / Math.max(1, Math.min(60, Math.floor(L.w / 70)));
    const nice = [...[1 / fps, 2 / fps, 5 / fps].filter((s) => s < 0.1), 0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60].find((s) => s >= step) || 60;
    const ticks = [];
    for (let t = 0; t <= L.dur + 1e-9; t += nice) ticks.push(h('span', { class: 'tick', style: `left: ${L.x(t)}px` }, nice < 0.1 ? `f${Math.round(t * fps)}` : `${+t.toFixed(2)}s`));
    const pauses = (st.comp.pauses || []).map((t) => h('span', { class: 'pause-flag', style: `left: ${L.x(t)}px`, title: `Pause at ${t.toFixed(2)} s: presenting waits here. Right-click to remove.` }));
    // The end: drag it to make the animation longer or shorter.
    const endGrip = h('span', { class: 'tl-end', 'data-end': '', style: `left: ${L.x(L.dur)}px`, title: `Ends at ${+L.dur.toFixed(2)} s: drag to make it longer or shorter` });
    rows.push(h('div', { class: 'trow head', style: width }, h('div', { class: 'nm', style: 'font-size: 12px; color: var(--faint)' }, 'Layer'), h('div', { class: 'lane', 'data-scrub': '' }, ticks, pauses, endGrip)));
    const sel = st.selection;
    const all = st.layers;
    for (const l of this.visibleLayers()) {
      const on = sel.includes(l.id);
      const end = l.out ?? st.comp.duration;
      const i = all.indexOf(l);
      const isGroup = all[i + 1] && all[i + 1].depth > l.depth;
      const caret = isGroup ? h('button', { class: 'tl-caret', 'aria-label': this.collapsed.has(l.id) ? `Show what’s in ${l.name}` : `Hide what’s in ${l.name}`, 'aria-expanded': String(!this.collapsed.has(l.id)),
        html: icon(this.collapsed.has(l.id) ? 'right' : 'down', 14),
        onClick: (e) => { e.stopPropagation(); if (this.collapsed.has(l.id)) this.collapsed.delete(l.id); else this.collapsed.add(l.id); this.render(); } }) : h('span', { class: 'tl-caret-gap' });
      const animated = l.keys.length > 0;
      const watch = h('button', { class: `tl-watch${animated ? ' on' : ''}`, 'aria-label': `Track all of ${l.name}`, title: animated ? 'Click: animate every property of this layer (a keyframe on each at the playhead), or, when all are, stop animating it' : 'Track everything: animate all of this layer’s properties with a keyframe at the playhead',
        html: icon('stopwatch', 14), onClick: (e) => { e.stopPropagation(); const now = app.run('track_all', { id: l.id }); if (now === false) app.toast?.(`${l.name} isn’t animated any more.`); } });
      const sumKeys = (on && sel.length === 1 ? [] : l.keys).map((t) => {
        const picked = [...this.selKeys.values()].some((k) => k.id === l.id && Math.abs(k.time - t) < 1e-6);
        return h('span', { class: `key small${picked ? ' picked' : ''}`, style: `left: ${L.x(t)}px`, 'data-sum': JSON.stringify({ id: l.id, time: t }), title: `Keyframes at ${t.toFixed(2)} s — click to select, drag to move` });
      });
      const bar = h('span', { class: 'span-bar', 'data-bar': String(l.id), style: `left: ${L.x(l.in)}px; width: ${Math.max(2, L.x(end) - L.x(l.in))}px`, title: 'Drag to move the layer in time; drag an end to trim it' });
      if (l.kind === 'sound' || l.kind === 'video') this.waveInto(bar, l, L);
      rows.push(h('div', { class: `trow${on ? ' on' : ''}`, style: width, 'data-layer': String(l.id) },
        h('div', { class: 'nm', style: `padding-left: ${6 + l.depth * 16}px`, onClick: (e) => app.select(e.shiftKey ? [...sel, l.id] : [l.id]) }, caret, watch, h('span', { class: 'tl-name' }, l.name)),
        h('div', { class: 'lane', 'data-lane': '' }, bar, sumKeys)));
      if (on && sel.length === 1) {
        const info = app.layerInfo;
        for (const [p, v] of Object.entries(info?.props || {})) {
          if (!v.animated) continue;
          rows.push(h('div', { class: 'trow', style: width },
            h('div', { class: 'nm sub' }, app.labelFor ? app.labelFor(p) : propName(p)),
            h('div', { class: 'lane', 'data-lane': '' }, v.keys.map((t) => {
              const k = { id: l.id, prop: p, time: t };
              return h('button', {
                class: `key${this.selKeys.has(keyId(k)) ? ' picked' : ''}`, style: `left: ${L.x(t)}px`, 'aria-label': `${propName(p)} keyframe at ${t.toFixed(2)} s`, 'aria-pressed': String(this.selKeys.has(keyId(k))),
                title: `${propName(p)} at ${t.toFixed(2)} s — click to select (Shift: add), drag to move, double-click to remove, right-click for easing`,
                'data-key': JSON.stringify(k),
              });
            }))));
        }
      }
    }
    if (!st.layers.length) rows.push(h('div', { class: 'empty', style: 'padding: 12px 16px' }, 'Nothing drawn yet. Pick a tool above and draw on the canvas.'));
    this.playhead = h('div', { class: 'playhead', style: `left: ${NAME_W + L.x(st.time)}px` });
    this.boxEl = h('div', { class: 'tl-box', hidden: true });
    this.body.replaceChildren(...rows.filter(Boolean), this.playhead, this.boxEl);
    this.update();
  }

  /// A sound's waveform drawn along its bar.
  waveInto(bar, l, L) {
    let w = this.waves.get(l.id);
    if (!w || w.rev !== this.app.state.revision) {
      let data = null;
      try { data = call('wave', { id: l.id, bins: 800 }); } catch { /* none */ }
      w = { rev: this.app.state.revision, data };
      this.waves.set(l.id, w);
    }
    const d = w.data;
    if (!d?.peaks?.length || !(d.duration > 0)) return;
    const end = l.out ?? this.app.state.comp.duration;
    const px = Math.max(2, Math.round(L.x(end) - L.x(l.in)));
    const c = this.h('canvas', { class: 'wave', width: String(Math.min(px, 4000)), height: '14', 'aria-hidden': 'true' });
    const g = c.getContext('2d');
    g.fillStyle = 'rgba(37, 99, 235, .55)';
    for (let x = 0; x < c.width; x += 2) {
      const t = (x / c.width) * (end - l.in);
      let src = d.source_in + t * d.speed;
      if (d.looped) src = ((src % d.duration) + d.duration) % d.duration;
      else if (src < 0 || src > d.duration) continue;
      const v = d.peaks[Math.min(d.peaks.length - 1, Math.floor((src / d.duration) * d.peaks.length))] || 0;
      const hh = Math.max(0.5, Math.min(1, v) * 6.5);
      g.fillRect(x, 7 - hh, 1.5, hh * 2);
    }
    bar.append(c);
  }

  /// "Add animation" at time `t` on layer `id` (or the selected layers, when it's one of them): the ready-made animations.
  animationMenu(id, t) {
    const { app } = this;
    const sel = app.state.selection;
    const ids = sel.includes(id) ? sel : [id];
    const at = timecode(t, this.fps());
    return app.ui.sub(`Add animation from ${at}`, () => (app.animPresets ||= call('anim_presets')).map((pr) => app.ui.item(pr.name, { run: () => {
      if (app.run('apply_preset', { name: pr.name, ids, time: t, stagger: ids.length > 1 ? 0.15 : 0 }) !== null) app.toast?.(`${pr.name} from ${at}.`);
    } })));
  }

  context(e) {
    const { app } = this;
    const key = e.target.closest('[data-key]');
    const sum = e.target.closest('[data-sum]');
    if (sum && !key) {
      // A layer's keyframe mark: its time, for a ready-made animation.
      e.preventDefault();
      const k = JSON.parse(sum.dataset.sum);
      app.ui.popup(null, [this.animationMenu(k.id, k.time), app.ui.item('Go here', { run: () => app.setTime(k.time) })], { at: [e.clientX, e.clientY] });
      return;
    }
    if (key) {
      e.preventDefault();
      const k = JSON.parse(key.dataset.key);
      if (!this.selKeys.has(keyId(k))) this.selectKeys([k]);
      const keys = this.selectedKeys();
      app.ui.popup(null, [this.animationMenu(k.id, k.time), this.h('hr'), ...this.easingItems(keys), this.h('hr'),
        app.ui.item(keys.length === 1 ? 'Remove keyframe' : `Remove ${keys.length} keyframes`, { run: () => this.removeSelectedKeys() }),
        app.ui.item('Copy keyframes', { run: () => this.copyKeys() })], { at: [e.clientX, e.clientY] });
      return;
    }
    if (e.target.closest('[data-scrub]')) {
      e.preventDefault();
      const t = this.timeAt(e.clientX);
      const fps = this.fps();
      const snapped = Math.round(t * fps) / fps;
      const has = (app.state.comp.pauses || []).some((p) => Math.abs(p - snapped) < 0.5 / fps + 0.05);
      app.ui.popup(null, [app.ui.item(has ? `Remove the pause at ${snapped.toFixed(2)} s` : `Add a pause at ${snapped.toFixed(2)} s`, { run: () => {
        const near = (app.state.comp.pauses || []).find((p) => Math.abs(p - snapped) < 0.05 + 0.5 / fps);
        app.run('toggle_pause', { time: near ?? snapped });
      } }), app.ui.item('Go here', { run: () => app.setTime(snapped) })], { at: [e.clientX, e.clientY] });
      return;
    }
    const row = e.target.closest('[data-layer]');
    if (row && e.target.closest('[data-lane]')) {
      // Anywhere on a layer's row: an animation from that moment.
      e.preventDefault();
      const id = Number(row.dataset.layer);
      const t = Math.round(this.timeAt(e.clientX) * this.fps()) / this.fps();
      app.ui.popup(null, [this.animationMenu(id, t),
        app.ui.item(`Keyframe what’s animated at ${timecode(t, this.fps())}`, { run: () => { app.setTime(t); app.run('key_all', { ids: [id], all: false }); } }),
        app.ui.item('Go here', { run: () => app.setTime(t) })], { at: [e.clientX, e.clientY] });
      return;
    }
    if (row && e.target.closest('.nm')) {
      e.preventDefault();
      const id = Number(row.dataset.layer);
      if (!app.state.selection.includes(id)) app.select([id]);
      const at = timecode(app.state.time, this.fps());
      const t = app.state.time;
      app.ui.popup(null, [
        this.animationMenu(id, t),
        this.h('hr'),
        app.ui.item(`Key all properties at ${at}`, { run: () => app.run('key_all', { ids: [id], all: true }) }),
        app.ui.item(`Key animated properties at ${at}`, { run: () => app.run('key_all', { ids: [id], all: false }) }),
        app.ui.item('Select this layer’s keyframes', { run: () => this.selectAllKeys([id]) }),
        app.ui.item('Track everything (stopwatch)', { run: () => app.run('track_all', { id }) }),
        this.h('hr'),
        app.ui.item('Start the layer here (trim the start)', { run: () => app.run('set_in_out', { id, in: t }) }),
        app.ui.item('End the layer here (trim the end)', { run: () => app.run('set_in_out', { id, out: t }) }),
        app.ui.item('Show it to the end', { run: () => app.run('set_in_out', { id, out: null }) }),
        this.h('hr'),
        app.ui.item('Rename…', { keys: 'F2', run: () => app.renameLayer?.(id) }),
      ], { at: [e.clientX, e.clientY] });
    }
  }

  timeAt(x) {
    const L = this.lane();
    const left = this.body.getBoundingClientRect().left + NAME_W - this.body.scrollLeft;
    return Math.max(0, Math.min(L.dur, ((x - left) / L.w) * L.dur));
  }

  down(e) {
    if (e.button !== 0) return;
    const { app } = this;
    const endGrip = e.target.closest('[data-end]');
    if (endGrip) {
      // Seconds per pixel stay as they were while dragging; the length is set on letting go.
      e.preventDefault();
      e.stopPropagation();
      const L0 = this.lane(), fps0 = this.fps(), x0 = e.clientX, d0 = L0.dur;
      const len = (ev) => Math.max(1 / fps0, Math.min(3600, Math.round((d0 + ((ev.clientX - x0) / L0.w) * d0) * fps0) / fps0));
      const move = (ev) => { endGrip.style.left = `${L0.x(len(ev))}px`; endGrip.dataset.len = `${+len(ev).toFixed(2)} s`; };
      const up = (ev) => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); const v = len(ev); if (Math.abs(v - d0) > 1e-6) app.run('set_comp', { duration: v }); else this.render(); };
      window.addEventListener('pointermove', move);
      window.addEventListener('pointerup', up);
      return;
    }
    const key = e.target.closest('[data-key]');
    const sum = e.target.closest('[data-sum]');
    const scrub = e.target.closest('[data-scrub]');
    const bar = e.target.closest('[data-bar]');
    const lane = e.target.closest('[data-lane]');
    if (!key && !sum && !scrub && !bar && !lane) return;
    e.preventDefault();
    this.body.focus({ preventScroll: true });
    const L = this.lane();
    const fps = this.fps();
    const snap = (t) => Math.round(t * fps) / fps;
    const drag = (move, up) => {
      const m = (ev) => move(ev);
      const u = (ev) => { window.removeEventListener('pointermove', m); window.removeEventListener('pointerup', u); up?.(ev); };
      window.addEventListener('pointermove', m);
      window.addEventListener('pointerup', u);
    };
    if (scrub) {
      app.setTime(snap(this.timeAt(e.clientX)));
      drag((ev) => app.setTime(snap(this.timeAt(ev.clientX))));
      return;
    }
    if (key || sum) {
      // Pick the key (Shift: add to the picked ones), then drag every picked key together.
      let picked;
      if (key) picked = [JSON.parse(key.dataset.key)];
      else { const s = JSON.parse(sum.dataset.sum); picked = call('all_keys', { ids: [s.id] }).filter((k) => Math.abs(k.time - s.time) < 1e-6); }
      const already = picked.every((k) => this.selKeys.has(keyId(k)));
      if (e.shiftKey && already) { for (const k of picked) this.selKeys.delete(keyId(k)); this.render(); return; }
      if (!already) { if (!e.shiftKey) this.selKeys.clear(); for (const k of picked) this.selKeys.set(keyId(k), k); }
      const keys = this.selectedKeys();
      const t0 = snap(this.timeAt(e.clientX));
      let delta = 0;
      const els = [...this.body.querySelectorAll('[data-key], [data-sum]')];
      const base = new Map(els.map((el) => [el, JSON.parse(el.dataset.key || el.dataset.sum)]));
      const ids = new Set(keys.map(keyId));
      const times = new Set(keys.map((k) => `${k.id}|${k.time}`));
      for (const el of els) {
        const k = base.get(el);
        if (el.dataset.key ? ids.has(keyId(k)) : times.has(`${k.id}|${k.time}`)) el.classList.add('picked');
        else if (!e.shiftKey && !already) el.classList.remove('picked');
      }
      drag((ev) => {
        const earliest = Math.min(...keys.map((k) => k.time));
        delta = Math.max(-earliest, snap(this.timeAt(ev.clientX)) - t0);
        for (const el of els) {
          const k = base.get(el);
          if (el.dataset.key ? ids.has(keyId(k)) : times.has(`${k.id}|${k.time}`)) el.style.left = `${L.x(k.time + delta)}px`;
        }
      }, () => {
        if (Math.abs(delta) > 1e-6) {
          const moved = app.run('move_keys', { keys, delta });
          this.selKeys.clear();
          for (const k of keys) { const n = { ...k, time: k.time + (moved ?? delta) }; this.selKeys.set(keyId(n), n); }
          this.render();
        } else if (keys.length === 1 || key) { app.setTime(picked[0].time); this.render(); } else this.render();
      });
      return;
    }
    if (bar) {
      // Drag the middle to slide the layer in time; drag an end to trim it.
      const id = Number(bar.dataset.bar);
      const l = app.state.layers.find((x) => x.id === id);
      const r = bar.getBoundingClientRect();
      const edge = e.clientX - r.left < 6 ? 'in' : r.right - e.clientX < 6 ? 'out' : null;
      const t0 = snap(this.timeAt(e.clientX));
      const n = (this.dragN = (this.dragN || 0) + 1);
      let last = 0;
      if (!app.state.selection.includes(id)) app.select([id]);
      drag((ev) => {
        const t = snap(this.timeAt(ev.clientX));
        if (edge === 'in') app.run('set_in_out', { id, in: t, drag: n }, { quiet: true });
        else if (edge === 'out') app.run('set_in_out', { id, out: t, drag: n }, { quiet: true });
        else {
          const d = t - t0;
          if (Math.abs(d - last) > 1e-6) { app.run('shift_layer', { id, delta: d - last, drag: n }, { quiet: true }); last = d; }
        }
        this.render();
      }, () => app.changed());
      void l;
      return;
    }
    // Empty lane: drag a box around keyframes (Shift: add to the picked ones).
    const br = this.body.getBoundingClientRect();
    const from = [e.clientX, e.clientY];
    const add = e.shiftKey;
    let moved = false;
    drag((ev) => {
      moved = true;
      const [x0, x1] = [Math.min(from[0], ev.clientX), Math.max(from[0], ev.clientX)];
      const [y0, y1] = [Math.min(from[1], ev.clientY), Math.max(from[1], ev.clientY)];
      Object.assign(this.boxEl.style, { left: `${x0 - br.left + this.body.scrollLeft}px`, top: `${y0 - br.top + this.body.scrollTop}px`, width: `${x1 - x0}px`, height: `${y1 - y0}px` });
      this.boxEl.hidden = false;
      this.box = [x0, y0, x1, y1];
    }, () => {
      this.boxEl.hidden = true;
      if (!moved) {
        if (!add) { this.selKeys.clear(); app.setTime(snap(this.timeAt(e.clientX))); this.render(); }
        return;
      }
      const [x0, y0, x1, y1] = this.box;
      const got = [];
      for (const el of this.body.querySelectorAll('[data-key], [data-sum]')) {
        const r = el.getBoundingClientRect();
        const cx = (r.left + r.right) / 2, cy = (r.top + r.bottom) / 2;
        if (cx < x0 || cx > x1 || cy < y0 || cy > y1) continue;
        if (el.dataset.key) got.push(JSON.parse(el.dataset.key));
        else { const s = JSON.parse(el.dataset.sum); got.push(...call('all_keys', { ids: [s.id] }).filter((k) => Math.abs(k.time - s.time) < 1e-6)); }
      }
      this.selectKeys(got, add);
    });
  }
}

/// An easing preset's name for people ("ease_in" → "Ease in (start slow)").
export function easingLabel(n) {
  return { linear: 'Steady', step_end: 'Hold (jump at the next)', ease: 'Ease', ease_in: 'Ease in (start slow)', ease_out: 'Ease out (end slow)', ease_in_out: 'Ease in and out', smooth: 'Smooth' }[n] || n.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase());
}
