// Insert › Emoji (the bundled Fluent Emoji, added as editable shapes) and
// Insert › GIF (GIPHY searched through the Xenify server, with its admin's
// key; the GIF is added as an animated picture). Both are pickers in a dialog.

import * as platform from './platform.js';
import { call } from './engine.js';
import { icon } from './icons.js';
import { ui } from './serverui.js';

/// Dragging a picker's tile out of its dialog onto the canvas: the dialog fades while the tile follows the
/// pointer; let go over the canvas to call `onDrop([x, y])` with the canvas point (the dialog then closes).
export function dragToCanvas(app, tile, e, ghostSrc, onDrop) {
  if (e.button !== 0) return;
  const x0 = e.clientX, y0 = e.clientY;
  const back = tile.closest('.modal-back');
  let ghost = null;
  const move = (ev) => {
    if (!ghost && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 8) return;
    if (!ghost) {
      ghost = document.createElement('img');
      ghost.src = ghostSrc;
      ghost.className = 'drag-ghost';
      document.body.append(ghost);
      if (back) back.classList.add('dragging-out');
    }
    ghost.style.left = `${ev.clientX - 28}px`;
    ghost.style.top = `${ev.clientY - 28}px`;
  };
  const up = (ev) => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    if (!ghost) return;
    ghost.remove();
    back?.classList.remove('dragging-out');
    const stage = document.getElementById('stage').getBoundingClientRect();
    if (ev.clientX >= stage.left && ev.clientX < stage.right && ev.clientY >= stage.top && ev.clientY < stage.bottom) {
      tile.dataset.dropped = '1';
      back?.querySelector('.modal-actions .btn')?.click();
      onDrop(app.viewport.toComp(ev));
    }
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

const RECENT = 'draw.recentEmoji';
const recentEmoji = () => { try { return JSON.parse(localStorage.getItem(RECENT) || '[]'); } catch { return []; } };
const rememberEmoji = (glyph) => { try { localStorage.setItem(RECENT, JSON.stringify([glyph, ...recentEmoji().filter((g) => g !== glyph)].slice(0, 24))); } catch { /* not kept */ } };
const ANIM_CREDIT = 'Animated emoji: Noto Emoji Animation © Google LLC, CC BY 4.0';
let animatedList = null;

export async function emojiPicker(app, h, { modal, toast }) {
  const groups = call('emoji_groups');
  let group = recentEmoji().length ? 'Recent' : groups[0];
  let mode = 'add';
  let looped = true;
  const grid = h('div', { class: 'emoji-grid', role: 'listbox', 'aria-label': 'Emoji' });
  const search = h('input', { class: 'field', placeholder: 'Search emoji (party, rocket, cat…)', 'aria-label': 'Search emoji' });
  const tabs = h('div', { class: 'emoji-tabs' });
  const foot = h('div', { class: 'emoji-foot' });
  const credit = h('small', { class: 'muted' });
  let pick = null;
  const svgUrl = (svg) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  const finish = () => document.querySelector('.modal-back .btn.main')?.click();
  // Adds emoji `e` (at a canvas point, else the middle): as shapes, or a confetti burst of it.
  const addStill = (e, at) => {
    rememberEmoji(e.glyph);
    if (mode === 'confetti') { if (app.run('emoji_confetti', { glyph: e.glyph }) != null) toast(`Added a confetti burst of ${e.name}: press Space to watch it.`); }
    else if (app.run('add_emoji', { glyph: e.glyph, at }) != null) { app.setTool('select'); toast('Added the emoji as shapes (a group).'); }
  };
  const addAnimated = async (e, at) => {
    toast(`Getting ${e.name}…`);
    try {
      const res = await fetch(e.url);
      if (!res.ok) throw new Error(`the server said ${res.status}`);
      const text = await res.text();
      const s = app.state.comp;
      if (app.run('import_lottie', { text, name: `${e.glyph} ${e.name} (animated)`, rename: true, at, size: Math.round(Math.min(s.width, s.height) / 4), looped }) != null) {
        rememberEmoji(e.glyph);
        toast(`Added ${e.glyph} ${e.name} as a symbol (double-click to edit its layers). ${ANIM_CREDIT}`);
      }
    } catch (x) { toast(`Couldn’t add ${e.name}: ${x.message || x}`); }
  };
  const cell = (e, animated) => {
    const src = e.svg ? svgUrl(e.svg) : null;
    const b = h('button', { class: `emoji-cell${animated ? ' animated' : ''}`, title: `${e.name}${animated ? ' (animated)' : ''}: click to add, or drag onto the canvas`, 'aria-label': `${e.name}${animated ? ' (animated)' : ''}`,
      onClick: () => { if (b.dataset.dropped) return; pick = { e, animated }; finish(); },
      onPointerdown: (ev) => dragToCanvas(app, b, ev, src || svgUrl(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><text x="32" y="48" font-size="44" text-anchor="middle">${e.glyph}</text></svg>`), (at) => (animated ? addAnimated(e, at) : addStill(e, at))) },
    src ? h('img', { src, alt: '', draggable: 'false' }) : h('span', { class: 'emoji-glyph' }, e.glyph));
    return b;
  };
  const show = async () => {
    const q = search.value.trim();
    tabs.replaceChildren(...['Recent', 'Animated', ...groups].map((g) => h('button', { class: `seg${(!q || g === 'Animated') && g === group ? ' on' : ''}`, onClick: () => { group = g; if (g !== 'Animated') search.value = ''; show(); } }, g)));
    credit.textContent = group === 'Animated' ? ANIM_CREDIT : 'Fluent Emoji © Microsoft (MIT licence). Each is added as shapes you can edit.';
    foot.replaceChildren(group === 'Animated'
      ? h('label', { class: 'check-row', title: 'Else they play once, from the playhead' }, h('input', { type: 'checkbox', checked: looped, onChange: (ev) => { looped = ev.target.checked; } }), h('span', {}, 'Play over and over'))
      : h('div', { class: 'segs mini', role: 'radiogroup', 'aria-label': 'What a click adds' }, [['add', 'Add it'], ['confetti', 'Confetti burst of it']].map(([k, t]) =>
        h('button', { class: `seg${mode === k ? ' on' : ''}`, role: 'radio', 'aria-checked': String(mode === k), title: k === 'confetti' ? 'A burst of particles, each one this emoji' : 'As shapes you can edit', onClick: () => { mode = k; show(); } }, t))));
    if (group === 'Animated') {
      if (!animatedList) {
        grid.replaceChildren(h('p', { class: 'muted' }, 'Getting the list of animated emoji…'));
        try { animatedList = call('animated_emoji_list', { text: await (await fetch('https://googlefonts.github.io/noto-emoji-animation/data/api.json')).text() }); }
        catch (x) { grid.replaceChildren(h('p', { class: 'muted' }, `Couldn’t get the animated emoji: ${x.message || x}`), h('button', { class: 'small-btn', onClick: () => show() }, 'Try again')); return; }
      }
      const ql = q.toLowerCase();
      const list = (ql ? animatedList.filter((e) => e.name.includes(ql) || e.tags.some((t) => t.includes(ql))) : animatedList).slice(0, 240);
      // The still Fluent picture where there is one.
      const stills = call('emoji_find', { glyphs: list.map((e) => e.glyph) });
      grid.replaceChildren(...list.map((e, i) => cell({ ...e, svg: stills[i]?.svg || null }, true)));
      if (!list.length) grid.replaceChildren(h('p', { class: 'muted' }, 'No animated emoji match that.'));
      return;
    }
    let list;
    if (q) list = call('emoji_search', { query: q, group: null, limit: 240 });
    else if (group === 'Recent') list = call('emoji_find', { glyphs: recentEmoji() }).filter((e) => e);
    else list = call('emoji_search', { query: '', group, limit: 240 });
    grid.replaceChildren(...list.map((e) => cell(e, false)));
    if (!list.length) grid.replaceChildren(h('p', { class: 'muted' }, group === 'Recent' && !q ? 'The emoji you add show here.' : 'No emoji match that.'));
  };
  search.addEventListener('input', () => { if (group === 'Recent') group = groups[0]; show(); });
  search.addEventListener('keydown', (e) => e.stopPropagation());
  show();
  const ok = await modal('Emoji', h('div', { class: 'emoji-picker' }, search, tabs, grid, foot, credit), [['Close', false], ['Add', () => !!pick, true]]);
  if (ok && pick) await (pick.animated ? addAnimated(pick.e) : addStill(pick.e));
}

export async function gifPicker(app, h, { modal, toast, addPicture }) {
  const search = h('input', { class: 'field', placeholder: 'Search GIFs (empty: trending)', 'aria-label': 'Search GIFs' });
  const grid = h('div', { class: 'gif-grid' });
  const note = h('small', { class: 'muted' });
  const more = h('button', { class: 'btn', hidden: true, onClick: () => run(true) }, 'More');
  let pick = null, offset = 0;
  const run = async (again = false) => {
    offset = again ? offset + 24 : 0;
    note.textContent = search.value.trim() ? 'Searching…' : 'Loading what’s trending…';
    try {
      const srv = ui.server;
      if (!srv?.session) throw new Error('sign in to your Xenify server first');
      const answer = await srv.api('GET', `/v1/gifs?q=${encodeURIComponent(search.value.trim())}&offset=${offset}`);
      const list = call('giphy_parse', { text: JSON.stringify(answer) });
      note.textContent = list.length || again ? 'Powered by GIPHY' : 'Nothing found.';
      const cells = list.map((g) => h('button', { class: 'gif-cell', title: g.credit, onClick: () => { pick = g; document.querySelector('.modal-back .btn.main')?.click(); } }, h('img', { src: g.preview, alt: g.title, loading: 'lazy' })));
      if (again) grid.append(...cells); else grid.replaceChildren(...cells);
      more.hidden = list.length < 24;
    } catch (e) {
      note.textContent = `Couldn’t get GIFs: ${e.message || e}`;
    }
  };
  search.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') { e.preventDefault(); run(); } });
  run();
  const ok = await modal('GIFs', h('div', { class: 'emoji-picker' }, h('div', { style: 'display: flex; gap: 8px' }, search, h('button', { class: 'btn', onClick: () => run() }, 'Search')), grid, h('div', { style: 'display: flex; gap: 8px; align-items: center' }, note, h('span', { style: 'flex: 1' }), more)), [['Close', false], ['Add', () => !!pick, true]]);
  if (!ok || !pick) return;
  try {
    const bytes = new Uint8Array(await (await fetch(pick.full)).arrayBuffer());
    await addPicture(`${pick.title || 'GIF'}.gif`, bytes);
    toast(`Added ${pick.credit}.`);
  } catch (e) {
    toast(`Couldn’t download the GIF: ${e.message || e}`);
  }
}

/// Diagram canvases: the shapes to build with (flowchart, UML, BPMN, network, cloud), in the left panel.
/// Click one to add it (beside the selected shape, joined to it), or drag it onto the canvas. Made once and kept,
/// so the search and the open group stay as you left them.
export function shapesPanel(app, h, { toast, close }) {
  if (app.shapesPanelEl) return app.shapesPanelEl;
  const ORDER = ['Flowchart', 'UML', 'BPMN & swimlanes', 'Network', 'Cloud & software'];
  const lib = (app.library ||= call('library')).slice().sort((a, b) => (ORDER.indexOf(a.category) + 99) % 99 - (ORDER.indexOf(b.category) + 99) % 99);
  let cat = 0;
  const grid = h('div', { class: 'lib-grid side' });
  const tabs = h('div', { class: 'lib-tabs', role: 'tablist', 'aria-label': 'Kinds of shape' });
  const search = h('input', { class: 'field small', type: 'search', placeholder: 'Find a shape (decision, class, actor…)', 'aria-label': 'Find a shape' });
  const svgOf = (s, size = 34) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${s.box[0] - 2} ${s.box[1] - 2} ${s.box[2] + 4} ${s.box[3] + 4}" width="${size}" height="${size}">${s.parts.map((p) => `<path d="${p.d}" fill="${p.role === 'body' ? '#fff' : p.role === 'solid' ? '#374151' : 'none'}" stroke="${p.role === 'solid' ? 'none' : '#374151'}" stroke-width="1.5" stroke-linejoin="round"/>`).join('')}</svg>`;
  const added = () => { app.setTool('select'); app.hint?.('Drag the little arrows round it to add a joined shape, or join shapes with the Connector (F). Double-click to type its label.'); };
  const show = () => {
    const q = search.value.trim().toLowerCase();
    tabs.replaceChildren(...lib.map((c, i) => h('button', { class: `seg${!q && i === cat ? ' on' : ''}`, role: 'tab', 'aria-selected': String(!q && i === cat), onClick: () => { cat = i; search.value = ''; show(); } }, c.category.replace(' & swimlanes', '').replace(' & software', ''))));
    const list = q ? lib.flatMap((c) => c.shapes.filter((s) => s.name.toLowerCase().includes(q) || c.category.toLowerCase().includes(q))) : lib[cat].shapes;
    grid.replaceChildren(...list.map((s) => {
      const b = h('button', { class: 'lib-cell', title: `${s.name}: click to add${app.state?.selection.length ? ' beside the selected shape' : ''}, or drag onto the canvas`,
        onClick: () => { if (b.dataset.dropped) return; if (app.run('add_library_shape', { shape: s.id }) != null) added(); },
        onPointerdown: (e) => dragToCanvas(app, b, e, `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svgOf(s, 56))}`, (at) => { if (app.run('add_library_shape', { shape: s.id, at }) != null) added(); }) },
      h('span', { html: svgOf(s) }), h('span', { class: 'lib-name' }, s.name));
      return b;
    }));
    if (!list.length) grid.replaceChildren(h('p', { class: 'muted' }, 'No shapes match that.'));
  };
  search.addEventListener('input', show);
  search.addEventListener('keydown', (e) => e.stopPropagation());
  show();
  app.shapesPanelEl = h('div', { class: 'shapes-panel' },
    h('div', { class: 'panel-head' }, h('h2', {}, 'Diagram shapes'), close ? h('button', { class: 'icon-btn small', 'aria-label': 'Hide the shapes', title: 'Hide (the shapes button in the toolbar brings it back)', html: icon('x'), onClick: () => close() }) : null), search, tabs, grid);
  return app.shapesPanelEl;
}
