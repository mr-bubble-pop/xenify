// Slides: the strip of slides (thumbnails, add, duplicate, delete, move),
// each slide's transitions and speaker notes, and presenting: the deck
// fills the screen and plays to each pause; → or Space goes on, ← goes back,
// P shows the presenter view (notes, the time, what's next), Esc stops.

import { layoutDialog, themeDialog, saveLayoutDialog } from './templates.js';
import { call, renderDeck, renderComp, transitionFrame, hasTransitionFrame, afterOffscreen } from './engine.js';
import { easingLabel } from './timeline.js';
import { curveDialog } from './curve.js';
import { icon } from './icons.js';

const KINDS = ['cut', 'fade', 'dip_to_color', 'push', 'cover', 'uncover', 'wipe', 'clock_wipe', 'iris', 'blinds', 'zoom_in', 'zoom_out', 'spin', 'flip', 'blur_dissolve', 'liquid', 'shatter', 'particle_dissolve', 'smart_morph'];
const words = (s) => s.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase());

export class Slides {
  constructor(app, h, ui) {
    Object.assign(this, { app, h, ui });
    this.thumbs = new Map();
  }

  list() {
    try { return call('slides'); } catch { return { presentation: false, list: [] }; }
  }

  /// A small picture of slide `s` (its first moment), cached until the document changes.
  thumb(s) {
    const key = `${s.comp}:${this.app.state.revision}`;
    if (this.thumbs.get(s.comp)?.key === key) return this.thumbs.get(s.comp).url;
    const gl = this.app.viewport.gl;
    const w = 160, hh = Math.max(1, Math.round((160 * s.height) / s.width));
    const [ow, oh] = [gl.width, gl.height];
    let url = '';
    try {
      gl.width = w; gl.height = hh;
      renderComp(s.comp, s.duration || 0, [w / s.width, 0, 0, hh / s.height, 0, 0], w, hh, '#ffffff');
      url = gl.toDataURL('image/png');
    } catch { /* drawn next time */ } finally {
      gl.width = ow; gl.height = oh;
      afterOffscreen();
    }
    this.thumbs.set(s.comp, { key, url });
    return url;
  }

  /// The strip above the layers (only for presentations).
  panel() {
    const { h, app } = this;
    const sl = this.list();
    // Not a presentation yet: the list says so, and its + makes this canvas the first slide.
    if (!sl.presentation) {
      if (this.app.state?.symbol) return null;
      return h('div', { class: 'slides-strip' },
        h('div', { class: 'panel-head' }, h('h2', {}, 'Slides'),
          h('button', { class: 'icon-btn small', 'aria-label': 'Add a slide', title: 'Add a slide: this canvas becomes slide 1 of a presentation', html: icon('plus'), onClick: () => this.add() })),
        h('div', { class: 'empty', style: 'font-size: 13px' }, 'This canvas is one picture. Press + to add a slide and make it a presentation (View › Panels hides this list).'));
    }
    return h('div', { class: 'slides-strip' },
      h('div', { class: 'panel-head' }, h('h2', {}, 'Slides'),
        h('button', { class: 'icon-btn small', 'aria-label': 'New slide', title: 'New slide after this one', html: icon('plus'), onClick: () => this.add() })),
      this.reorderable(h('div', { class: 'slide-list', role: 'listbox', 'aria-label': 'Slides (drag to reorder; Alt+↑ or ↓ moves the open one)' }, sl.list.map((s) => h('button', {
        class: `slide-card${s.open ? ' on' : ''}`, role: 'option', 'aria-selected': String(s.open), title: s.notes ? `Notes: ${s.notes}` : s.name, 'data-index': String(s.index),
        onClick: () => { if (!this.dragged) this.open(s.index); }, onContextmenu: (e) => { e.preventDefault(); this.menu(e, s, sl.list.length); },
        onKeydown: (e) => {
          if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
          e.preventDefault(); e.stopPropagation();
          const to = s.index + (e.key === 'ArrowUp' ? -1 : 1);
          if (to < 0 || to >= sl.list.length) return;
          app.run('move_slide', { index: s.index, to });
          this.refresh();
          requestAnimationFrame(() => document.querySelector(`.slide-card[data-index="${to}"]`)?.focus());
        },
      }, h('span', { class: 'slide-no' }, String(s.index + 1)), h('img', { src: this.thumb(s), alt: '', draggable: 'false' }), h('span', { class: 'slide-name' }, s.name))))));
  }

  /// Drag a slide up or down the list to move it; a line shows where it goes.
  reorderable(list) {
    // The thumbnails would start the browser's own drag otherwise.
    list.addEventListener('dragstart', (e) => e.preventDefault());
    list.addEventListener('pointerdown', (e) => {
      const card = e.target.closest('.slide-card');
      if (!card || e.button !== 0) return;
      const from = Number(card.dataset.index);
      const y0 = e.clientY;
      let to = null;
      this.dragged = false;
      const line = document.createElement('div');
      line.className = 'drop-line';
      const move = (ev) => {
        if (!this.dragged && Math.abs(ev.clientY - y0) < 6) return;
        this.dragged = true;
        card.classList.add('dragging');
        const cards = [...list.querySelectorAll('.slide-card')];
        let at = cards.length;
        for (let i = 0; i < cards.length; i++) { const r = cards[i].getBoundingClientRect(); if (ev.clientY < r.top + r.height / 2) { at = i; break; } }
        to = at > from ? at - 1 : at;
        const ref = cards[at];
        if (ref) list.insertBefore(line, ref); else list.append(line);
      };
      const up = () => {
        window.removeEventListener('pointermove', move);
        window.removeEventListener('pointerup', up);
        line.remove();
        card.classList.remove('dragging');
        if (this.dragged && to != null && to !== from) { this.app.run('move_slide', { index: from, to }); this.refresh(); }
        setTimeout(() => { this.dragged = false; }, 0);
      };
      window.addEventListener('pointermove', move);
      window.addEventListener('pointerup', up);
    });
    return list;
  }

  open(index) {
    this.app.run('open_slide', { index });
    this.app.lastStructure = '';
    this.app.changed();
    this.app.viewport.fit();
    this.app.writing?.run();
  }

  current() {
    return this.list().list.find((s) => s.open);
  }

  add() {
    const cur = this.current();
    this.app.run('add_slide', { after: cur ? cur.index : null });
    this.refresh();
  }

  refresh() {
    this.app.lastStructure = '';
    this.app.changed();
    this.app.viewport.fit();
  }

  menu(e, s, n) {
    const { ui } = this;
    ui.popup(null, [
      ui.item('Open', { run: () => this.open(s.index) }),
      ui.item('Duplicate', { run: () => { this.app.run('duplicate_slide', { index: s.index }); this.refresh(); } }),
      ui.item('Transition…', { run: () => this.transitionDialog(s) }),
      ui.item('Speaker notes…', { run: () => this.notesDialog(s) }),
      this.h('hr'),
      ui.item('Move up', { enabled: s.index > 0, run: () => { this.app.run('move_slide', { index: s.index, to: s.index - 1 }); this.refresh(); } }),
      ui.item('Move down', { enabled: s.index < n - 1, run: () => { this.app.run('move_slide', { index: s.index, to: s.index + 1 }); this.refresh(); } }),
      this.h('hr'),
      ui.item('Delete slide', { enabled: n > 1, run: () => { this.app.run('delete_slide', { index: s.index }); this.refresh(); } }),
      ui.item('Present from here', { run: () => this.present(s.index) }),
    ], { at: [e.clientX, e.clientY] });
  }

  /// The Slide menu's items.
  menuItems() {
    const { ui } = this;
    const sl = this.list();
    const cur = sl.list.find((s) => s.open) || sl.list[0];
    const n = sl.list.length;
    return [
      ui.item(sl.presentation ? 'New slide' : 'Make this a presentation (add a slide)', { run: () => this.add() }),
      ui.item('New slide from layout…', { run: () => layoutDialog(this.app, this.h).then(() => this.refresh()) }),
      ui.item('Apply theme…', { run: () => themeDialog(this.app, this.h) }),
      ui.item('Save slide as layout…', { enabled: !!cur, run: () => saveLayoutDialog(this.app, this.h) }),
      ui.item('Duplicate slide', { enabled: !!cur && sl.presentation, run: () => { this.app.run('duplicate_slide', { index: cur.index }); this.refresh(); } }),
      ui.item('Delete slide', { enabled: n > 1, run: () => { this.app.run('delete_slide', { index: cur.index }); this.refresh(); } }),
      ui.item('Move slide up', { enabled: cur?.index > 0, run: () => { this.app.run('move_slide', { index: cur.index, to: cur.index - 1 }); this.refresh(); } }),
      ui.item('Move slide down', { enabled: cur && cur.index < n - 1, run: () => { this.app.run('move_slide', { index: cur.index, to: cur.index + 1 }); this.refresh(); } }),
      '-',
      ui.item('Transition…', { enabled: !!cur, run: () => this.transitionDialog(cur) }),
      ui.item('Speaker notes…', { enabled: !!cur, run: () => this.notesDialog(cur) }),
      ui.item('Soundtrack…', { run: () => ui.soundtrack() }),
      (() => {
        const t = this.app.state.time, fps = this.app.state.comp.fps || 30;
        const here = (this.app.state.comp.pauses || []).some((p) => Math.abs(p - Math.round(t * fps) / fps) < 0.5 / fps);
        return ui.item(here ? 'Remove the pause here' : 'Add a pause here (presenting waits)', { keys: 'Ctrl+Shift+P', enabled: this.app.state.comp.duration > 0, run: () => this.app.run('toggle_pause', {}) });
      })(),
      '-',
      ui.item('Present', { keys: 'F5', run: () => this.present(0) }),
      ui.item('Present from this slide', { keys: 'Shift+F5', run: () => this.present(cur?.index || 0) }),
      ui.item('Rehearse with presenter view', { run: () => this.present(cur?.index || 0, true) }),
      ui.item('Present on…', { run: () => this.presentOnDialog(cur?.index || 0) }),
    ];
  }

  /// Slide › Transition…: how the slide comes in (one transition, as people expect), and only when asked
  /// for, a different one as it goes out. The cards are stills that play when pointed at; the big preview
  /// plays the chosen one slowly, with a pause before and after, so you can see what it does.
  async transitionDialog(s, which = 'in') {
    const { h, ui, app } = this;
    const kinds = (this.kinds ||= call('transition_kinds'));
    const easings = (app.easingList ||= call('easings'));
    const t = { in: { kind: 'cut', duration: 0.5, ...(s.transition_in || {}) }, out: { kind: 'cut', duration: 0.5, ...(s.transition_out || {}) } };
    let side = which;
    let both = side === 'out' || t.out.kind !== 'cut';
    const body = h('div', { class: 'transition-edit' });
    // Small cards: 12 frames, only the one under the pointer moves. The big preview: 30 frames.
    const FR = 12, BIG = 30;
    const still = document.documentElement.dataset.motion === 'reduce' || matchMedia('(prefers-reduced-motion: reduce)').matches;
    let hovered = null, tick = 0;
    const bigImg = h('img', { class: 'transition-big', alt: '' });
    // One loop of the big preview: hold on the first slide, play over the chosen seconds, hold on the second.
    const timer = setInterval(() => {
      tick++;
      const cur = t[side];
      const STEP = 50;
      const hold = 600 / STEP, play = Math.max(8, Math.min(60, Math.round(((cur.kind === 'cut' ? 0.2 : cur.duration ?? 0.5) * 1000) / STEP)));
      const n = tick % (hold + play + hold);
      const u = still ? 0.5 : n < hold ? 0 : n >= hold + play ? 1 : (n - hold) / play;
      let f = Math.round(u * (BIG - 1));
      // Frames are made a few at a time: until one is ready, the nearest made one shows.
      if (!hasTransitionFrame(cur.kind, f, BIG, 448)) { for (let k = 0; k < BIG && !hasTransitionFrame(cur.kind, f, BIG, 448); k++) f = (f + 1) % BIG; }
      const make = [...Array(BIG).keys()].find((k) => !hasTransitionFrame(cur.kind, k, BIG, 448));
      if (make != null) transitionFrame(cur.kind, make, BIG, 448, 252);
      const url = hasTransitionFrame(cur.kind, f, BIG, 448) ? transitionFrame(cur.kind, f, BIG, 448, 252) : '';
      if (url && bigImg.src !== url) bigImg.src = url;
      // The card under the pointer plays (slower than before: a frame every 100 ms).
      if (hovered && !still && tick % 2 === 0) {
        const img = body.querySelector(`img[data-kind="${hovered}"]`);
        const g = (tick / 2) % (FR + 6);
        const fr = Math.max(0, Math.min(FR - 1, g - 3));
        const u2 = transitionFrame(hovered, fr, FR);
        if (img && u2 && img.src !== u2) img.src = u2;
      }
    }, 50);
    const toHex = (c) => (c ? `#${[c.r, c.g, c.b].map((v) => Math.round(v * 255).toString(16).padStart(2, '0')).join('')}` : '#000000');
    const draw = () => {
      if (!both) side = 'in';
      const cur = t[side];
      const info = kinds.find((k) => k.id === cur.kind) || kinds[0];
      const tabs = both ? h('div', { class: 'segs', role: 'tablist', style: 'display: flex' }, [['in', 'Comes in'], ['out', 'Goes out']].map(([k, l]) => h('button', { class: `seg${side === k ? ' on' : ''}`, role: 'tab', 'aria-selected': String(side === k), style: 'flex: 1 1 0', onClick: () => { side = k; draw(); } }, l))) : null;
      const mid = Math.floor(FR / 2);
      const grid = h('div', { class: 'transition-grid', role: 'radiogroup', 'aria-label': 'Transition' }, kinds.map((k) => h('button', { class: `transition-card${k.id === cur.kind ? ' on' : ''}`, role: 'radio', 'aria-checked': String(k.id === cur.kind), title: `${k.label}: point at it to see it move`,
        onClick: () => { cur.kind = k.id; tick = 0; draw(); },
        onPointerenter: () => { hovered = k.id; },
        onPointerleave: (e) => { if (hovered === k.id) hovered = null; const img = e.currentTarget.querySelector('img'); const u = transitionFrame(k.id, mid, FR); if (img && u) img.src = u; } },
        h('img', { 'data-kind': k.id, alt: '', width: '112', height: '63', src: transitionFrame(k.id, mid, FR) }), h('span', {}, k.label))));
      const preview = h('div', { class: 'transition-preview' }, bigImg,
        h('small', { class: 'muted' }, cur.kind === 'cut' ? 'Cut: the slide simply appears.' : `${info.label}: the slide before, then ${+(cur.duration ?? 0.5).toFixed(2)} s of transition, then this slide.`));
      const row = (label, el) => h('div', { class: 'prop' }, h('span', { class: 'pname' }, label), el);
      const settings = cur.kind === 'cut' ? [] : [
        row('Seconds', h('input', { class: 'field small', type: 'number', min: '0.1', max: '10', step: '0.1', value: String(cur.duration ?? 0.5), title: 'How long the transition itself takes', onChange: (e) => { cur.duration = Math.max(0.1, Math.min(10, Number(e.target.value) || 0.5)); tick = 0; draw(); }, onKeydown: (e) => e.stopPropagation() })),
        info.direction ? row('Towards', h('select', { class: 'field small', onChange: (e) => { cur.direction = e.target.value; } }, ['left', 'right', 'up', 'down'].map((d) => h('option', { value: d, selected: d === (cur.direction || 'left') }, words(d))))) : null,
        info.color ? row('Colour', h('input', { type: 'color', class: 'color-in', value: toHex(cur.color), onInput: (e) => { const x = e.target.value; cur.color = { r: parseInt(x.slice(1, 3), 16) / 255, g: parseInt(x.slice(3, 5), 16) / 255, b: parseInt(x.slice(5, 7), 16) / 255, a: 1 }; } })) : null,
        info.softness ? row('Soft edge (%)', h('input', { class: 'field small', type: 'number', min: '0', max: '100', step: '5', value: String(Math.round((cur.softness ?? 0) * 100)), onChange: (e) => { cur.softness = Math.max(0, Math.min(100, Number(e.target.value) || 0)) / 100; }, onKeydown: (e) => e.stopPropagation() })) : null,
        row('Easing', h('div', { style: 'display: flex; gap: 6px; flex: 1' },
          h('select', { class: 'field small', onChange: (e) => { const x = easings.find((q) => q.name === e.target.value); if (x) cur.easing = x.easing; } },
            (() => { const now = easings.find((q) => JSON.stringify(q.easing) === JSON.stringify(cur.easing || { type: 'linear' }))?.name; return [...easings.map((q) => h('option', { value: q.name, selected: q.name === now }, easingLabel(q.name))), now ? null : h('option', { value: '', selected: true }, 'Custom')]; })()),
          h('button', { class: 'small-btn', onClick: async () => { const e = await curveDialog(app, h, cur.easing, { presets: easings, label: easingLabel }); if (e) { cur.easing = e; draw(); } } }, 'Edit curve…'))),
      ];
      // Smart Morph: what glides from the slide before, and pairing the rest by name.
      let morph = null;
      if (cur.kind === 'smart_morph' && side === 'in') {
        const m = call('morph_pairs', { index: s.index });
        morph = m ? h('div', { class: 'morph' },
          h('div', { class: 'lab', title: 'Layers with the same name glide from the slide before; the rest fade' }, `Match (${m.pairs.length} gliding)`),
          m.pairs.map((n) => h('div', { class: 'muted' }, `↔ ${n}`)),
          m.lone_in.map((l) => h('div', { class: 'prop' }, h('span', { class: 'muted', style: 'flex: 1' }, `fades in: ${l.name}`),
            m.lone_prev.length ? h('select', { class: 'field small', 'aria-label': `Pair ${l.name} with`, onChange: (e) => { if (e.target.value) { app.run('set_meta', { id: l.id, name: e.target.value }); draw(); } } },
              h('option', { value: '' }, 'Pair with…'), m.lone_prev.map((n) => h('option', { value: n }, n))) : null)))
          : h('p', { class: 'muted' }, 'Smart Morph needs a slide before this one.');
      }
      const second = h('label', { class: 'check-row', title: 'Most slides need only the way they come in; the next slide’s transition takes over from there' },
        h('input', { type: 'checkbox', checked: both, onChange: (e) => { both = e.target.checked; if (!both) t.out = { kind: 'cut', duration: 0.5 }; side = both ? 'out' : 'in'; tick = 0; draw(); } }),
        h('span', {}, 'Also a different transition as it goes out'));
      body.replaceChildren(...[tabs, both ? h('div', { class: 'lab' }, side === 'in' ? 'How this slide comes in' : 'How this slide goes out (before the next slide comes in)') : null, preview, grid, ...settings.filter(Boolean), morph || '', second].filter(Boolean));
    };
    draw();
    const ok = await ui.modal(`Slide ${s.index + 1}: ${both ? "transitions" : "how it comes in"}`, body, [['Cancel', false], ['Apply', true, true]]);
    clearInterval(timer);
    if (ok) app.run('set_slide', { index: s.index, transition_in: t.in, transition_out: t.out });
  }

  async notesDialog(s) {
    const { h, ui } = this;
    const box = h('textarea', { class: 'field', rows: '8', style: 'width: 100%; height: auto; padding: 10px', 'aria-label': 'Speaker notes' }, s.notes || '');
    box.addEventListener('keydown', (e) => e.stopPropagation());
    const ok = await ui.modal(`Speaker notes for slide ${s.index + 1}`, box, [['Cancel', false], ['Save', true, true]]);
    if (ok) this.app.run('set_slide', { index: s.index, notes: box.value });
  }

  // ----- Presenting on another screen -----

  /// The screens to present on: this window, then each monitor (desktop) or screen (browsers that say).
  async screens() {
    const out = [{ label: 'This window (full screen)', here: true }];
    const tauri = window.__TAURI__;
    try {
      if (tauri?.window?.availableMonitors && !window.XENIFY_WEB) {
        const cur = await tauri.window.currentMonitor();
        for (const m of await tauri.window.availableMonitors()) {
          const k = m.scaleFactor || 1;
          out.push({ label: `${m.name || 'Screen'} — ${m.size.width}×${m.size.height}${cur && m.name === cur.name ? ' (this one)' : ''}`,
            x: m.position.x / k, y: m.position.y / k, w: m.size.width / k, h: m.size.height / k, other: !(cur && m.name === cur.name) });
        }
      } else if (window.getScreenDetails) {
        const d = await window.getScreenDetails();
        for (const sc of d.screens) out.push({ label: `${sc.label || 'Screen'} — ${sc.width}×${sc.height}${sc === d.currentScreen ? ' (this one)' : ''}`, x: sc.left, y: sc.top, w: sc.width, h: sc.height, screen: sc, other: sc !== d.currentScreen });
      }
    } catch { /* only this window */ }
    return out;
  }

  async presentOnDialog(from) {
    const { h, ui } = this;
    const list = await this.screens();
    const pick = h('select', { class: 'field' }, list.map((sc, i) => h('option', { value: String(i), selected: sc.other && !list.slice(0, i).some((x) => x.other) }, sc.label)));
    const note = list.length > 1 ? 'The audience sees the slides there; this window shows your notes, the time and what’s next.'
      : 'This computer shows one screen (or this browser can’t list them): the slides fill this window.';
    if (!(await ui.modal('Present on', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' }, pick, h('p', { class: 'muted' }, note)), [['Cancel', false], ['Present', true, true]]))) return;
    const sc = list[Number(pick.value)];
    if (sc.here) this.present(from);
    else this.present(from, true, sc);
  }

  /// Opens the audience's window on screen `sc`; a function that sends it messages, and one that closes it.
  async audience(sc) {
    const tauri = window.__TAURI__;
    const project = call('save_copy');
    if (tauri?.webviewWindow && !window.XENIFY_WEB) {
      const ready = new Promise((r) => { tauri.event.once('present-ready', r); setTimeout(r, 4000); });
      const win = new tauri.webviewWindow.WebviewWindow('present', { url: 'present.html', title: 'Xenify Draw — presentation', x: sc.x, y: sc.y, width: sc.w, height: sc.h, decorations: false, fullscreen: true, focus: false });
      await ready;
      const send = (m) => tauri.event.emit('present', m).catch(() => {});
      send({ project });
      return { send, close: () => { send({ close: true }); win.destroy?.().catch?.(() => {}); } };
    }
    const ch = new BroadcastChannel('xenify-draw-present');
    const w = window.open('present.html', 'xenify-present', `popup,left=${sc.x},top=${sc.y},width=${sc.w},height=${sc.h}`);
    await new Promise((r) => { ch.onmessage = (e) => { if (e.data?.ready) r(); }; setTimeout(r, 4000); });
    ch.postMessage({ project });
    try { await w?.document.documentElement.requestFullscreen({ screen: sc.screen }); } catch { /* the audience presses F11 */ }
    return { send: (m) => ch.postMessage(m), close: () => { ch.postMessage({ close: true }); try { w?.close(); } catch { /* gone */ } ch.close(); } };
  }

  // ----- Presenting -----

  async present(from = 0, presenter = false, screen = null) {
    const out = screen ? await this.audience(screen) : null;
    const deck = call('deck');
    const t0 = deck.starts[from] ?? 0;
    const { h, app } = this;
    const canvas = h('canvas', { class: 'present-canvas' });
    const notes = h('div', { class: 'present-notes', hidden: !presenter });
    const el = h('div', { class: 'present', tabindex: '0' }, canvas, notes);
    document.body.append(el);
    if (!out) el.requestFullscreen?.().catch(() => {});
    el.focus();
    // The deck draws into the app's WebGL canvas, then is copied onto this one.
    const gl = app.viewport.gl;
    const g = canvas.getContext('2d');
    const P = { t: t0, playing: true, stop: this.nextStop(deck, t0), last: null, started: performance.now(), presenter, black: false, ended: false };
    app.presenting = true;
    // Sound: the deck's mix while it plays; while it waits, the soundtrack carries on if it's set to.
    const sound = () => {
      if (P.playing && !P.black) app.sound.startDeck(P.t, deck.keep_playing ? deck.duration : P.stop);
      else if (!(deck.keep_playing && !P.ended && app.sound.playing())) app.sound.stop();
    };
    const draw = (now) => {
      if (!el.isConnected) return;
      if (P.playing) {
        const dt = P.last == null ? 0 : (now - P.last) / 1000;
        P.t = Math.min(P.stop, P.t + dt);
        if (P.t >= P.stop - 1e-6) { P.playing = false; sound(); }
      }
      P.last = now;
      const dpr = window.devicePixelRatio || 1;
      const W = Math.round(el.clientWidth * (P.presenter ? 0.62 : 1) * dpr), H = Math.round(el.clientHeight * (P.presenter ? 0.62 : 1) * dpr);
      if (canvas.width !== W || canvas.height !== H) { canvas.width = W; canvas.height = H; }
      if (P.black || P.ended) { g.fillStyle = '#000'; g.fillRect(0, 0, W, H); } else {
        const [ow, oh] = [gl.width, gl.height];
        gl.width = W; gl.height = H;
        try { renderDeck(P.t, W, H); g.drawImage(gl, 0, 0); } catch { /* next frame */ }
        gl.width = ow; gl.height = oh;
      }
      if (P.presenter) this.presenterView(notes, deck, P, actions);
      const msg = { t: P.t, black: P.black || P.ended };
      if (out && JSON.stringify(msg) !== P.sent) { P.sent = JSON.stringify(msg); out.send(msg); }
      requestAnimationFrame(draw);
    };
    const go = (dir) => {
      P.black = false;
      if (dir > 0) {
        if (P.ended) { stop(); return; }
        if (P.playing) { P.t = P.stop; P.playing = false; sound(); return; }
        // Past the last stop: a black end screen, then out.
        if (P.stop >= deck.duration - 1e-6) { P.ended = true; sound(); return; }
        P.stop = this.nextStop(deck, P.t + 1e-4);
        P.playing = true;
      } else {
        P.ended = false;
        // Back to where the previous step ended (or the start).
        const prev = [0, ...deck.stops].filter((s) => s < P.t - 0.05).pop() ?? 0;
        P.t = prev; P.stop = prev; P.playing = false;
      }
      sound();
    };
    const stop = () => {
      out?.close();
      app.sound.stop();
      app.presenting = false;
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      el.remove();
      app.viewport.redraw();
      // Something waits for the end (back to the website after its example).
      const after = app.afterPresenting;
      app.afterPresenting = null;
      after?.();
    };
    const actions = {
      next: () => go(1), back: () => go(-1), stop,
      black: () => { P.black = !P.black; sound(); },
      view: () => { if (!out) { P.presenter = !P.presenter; notes.hidden = !P.presenter; } },
      reset: () => { P.started = performance.now(); },
      first: () => { P.black = false; P.ended = false; P.t = 0; P.stop = this.nextStop(deck, 0); P.playing = true; sound(); },
      last: () => { P.black = false; P.ended = false; P.t = deck.duration; P.stop = deck.duration; P.playing = false; sound(); },
    };
    el.addEventListener('keydown', (e) => {
      if (e.target.closest?.('button')) { if (e.key === 'Enter' || e.key === ' ') return; }
      e.preventDefault(); e.stopPropagation();
      if (['ArrowRight', 'ArrowDown', ' ', 'PageDown', 'Enter', 'n'].includes(e.key)) go(1);
      else if (['ArrowLeft', 'ArrowUp', 'PageUp', 'Backspace'].includes(e.key)) go(-1);
      else if (e.key === 'p' || e.key === 'P' || e.key === 'v' || e.key === 'V') actions.view();
      else if (e.key === 'b' || e.key === 'B' || e.key === '.') actions.black();
      else if (e.key === 'Escape') stop();
      else if (e.key === 'Home') actions.first();
      else if (e.key === 'End') actions.last();
    });
    canvas.addEventListener('click', () => go(1));
    el.addEventListener('contextmenu', (e) => { e.preventDefault(); go(-1); });
    if (!out) document.addEventListener('fullscreenchange', () => { if (!document.fullscreenElement && el.isConnected && !P.presenter) stop(); }, { once: true });
    sound();
    requestAnimationFrame(draw);
    this.P = P;
    return P;
  }

  nextStop(deck, t) {
    return deck.stops.find((s) => s > t + 1e-6) ?? deck.duration;
  }

  /// The presenter's view: the time (reset), where you are, the notes, the next slide, and buttons.
  presenterView(el, deck, P, A) {
    const i = Math.max(0, deck.starts.filter((s) => s <= P.t + 1e-6).length - 1);
    const secs = Math.floor((performance.now() - P.started) / 1000);
    const key = `${i}:${secs}:${P.black}:${P.ended}`;
    if (el.dataset.key === key) return;
    const fresh = el.dataset.slide !== String(i);
    el.dataset.key = key;
    el.dataset.slide = String(i);
    const { h } = this;
    const btn = (label, run, title) => h('button', { class: 'pv-btn', title, onClick: (e) => { e.stopPropagation(); run(); el.closest('.present')?.focus(); } }, label);
    // The next slide as a picture (drawn once per slide).
    if (fresh || !this.nextPic) {
      this.nextPic = null;
      const nt = deck.starts[i + 1];
      if (nt != null) {
        const gl = this.app.viewport.gl;
        const [ow, oh] = [gl.width, gl.height];
        const c = document.createElement('canvas');
        c.width = 384; c.height = 216;
        gl.width = 384; gl.height = 216;
        try { renderDeck(Math.max(nt, Math.min(this.nextStop(deck, nt + 1e-4), deck.ends[i + 1] ?? nt) - 0.01), 384, 216); c.getContext('2d').drawImage(gl, 0, 0); this.nextPic = c.toDataURL(); } catch { /* none */ }
        gl.width = ow; gl.height = oh;
      }
    }
    el.replaceChildren(
      h('div', { class: 'pv-top' },
        h('div', { class: 'pv-time' }, `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`),
        btn('Reset', A.reset, 'Restart the timer'),
        h('div', { class: 'pv-slide' }, P.ended ? 'End of the presentation' : `Slide ${i + 1} of ${deck.starts.length}${P.playing ? '' : ' · waiting'}`)),
      h('div', { class: 'pv-btns' },
        btn('◀ Back', A.back, '←, Page Up'), btn('Next ▶', A.next, '→, Space, Page Down'),
        btn(P.black ? 'Show slide' : 'Black screen', A.black, 'B or .'), btn('Audience view', A.view, 'V: what the audience sees'), btn('End show', A.stop, 'Esc')),
      h('div', { class: 'pv-notes' }, deck.notes[i] || 'No notes for this slide.'),
      deck.starts[i + 1] != null
        ? h('div', { class: 'pv-next' }, h('div', { class: 'lab' }, 'Next'), this.nextPic ? h('img', { src: this.nextPic, alt: `Slide ${i + 2}` }) : null, deck.notes[i + 1] ? h('div', {}, deck.notes[i + 1].slice(0, 120)) : null)
        : h('div', { class: 'pv-next' }, 'Last slide'),
      h('div', { class: 'pv-keys' }, '→ or Space: next · ←: back · B: black · V: audience view · Home / End · Esc: stop'));
  }
}
