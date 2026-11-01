// Slide templates: new slides from a layout (built in, or saved from one of
// your slides), themes for a whole presentation, and the gallery the New
// canvas screen shows. Saved layouts are kept in this program's storage.

import { call, layoutThumb } from './engine.js';

const KEY = 'draw.layouts';

export function savedLayouts() {
  try { return JSON.parse(localStorage.getItem(KEY) || '{}'); } catch { return {}; }
}

function keepLayouts(all) {
  localStorage.setItem(KEY, JSON.stringify(all));
}

/// Slide › New slide from layout…: the layouts in the deck's theme, and yours.
export async function layoutDialog(app, h) {
  const themes = call('themes');
  const layouts = call('layouts');
  const mine = savedLayouts();
  let picked = null;
  const body = h('div', { class: 'gallery' });
  const card = (label, src, run, extra) => h('button', { class: 'gallery-card', onClick: () => { picked = run; document.querySelector('.modal-back .btn.main')?.click(); } },
    src ? h('img', { src, alt: '', width: '160', height: '90' }) : h('div', { class: 'thumb-blank' }, label), h('span', {}, label), extra);
  body.append(
    h('div', { class: 'lab' }, `Layouts (${themes.current})`),
    h('div', { class: 'gallery-grid' }, layouts.map((l) => card(l, layoutThumb(themes.current, l), () => app.run('add_layout_slide', { layout: l })))));
  if (Object.keys(mine).length) {
    body.append(h('div', { class: 'lab', style: 'margin-top: 12px' }, 'My layouts'),
      h('div', { class: 'gallery-grid' }, Object.keys(mine).sort().map((name) => card(name, null, () => app.run('add_saved_layout', { name, json: mine[name] }),
        h('span', { class: 'gallery-x', role: 'button', tabindex: '0', title: `Forget “${name}”`, 'aria-label': `Forget ${name}`,
          onClick: (e) => { e.stopPropagation(); const all = savedLayouts(); delete all[name]; keepLayouts(all); e.target.closest('.gallery-card').remove(); } }, '×')))));
  }
  const ok = await app.ui.modal('New slide from layout', body, [['Cancel', false], ['Add', true, true]]);
  if (ok && picked) { picked(); app.slides?.refresh?.(); }
}

/// Slide › Apply theme…: recolours every templated layer and sets the transitions.
export async function themeDialog(app, h) {
  const themes = call('themes');
  let chosen = themes.current;
  const grid = themeGallery(h, chosen, (n) => { chosen = n; });
  const ok = await app.ui.modal('Apply a theme', h('div', { class: 'gallery' },
    h('p', { class: 'muted' }, 'Layers made from layouts (and any with a theme role) take the theme’s colours and type; slides take its background and transitions.'), grid),
  [['Cancel', false], ['Apply', true, true]]);
  if (ok) app.run('apply_theme', { name: chosen });
}

/// Slide › Save slide as layout…
export async function saveLayoutDialog(app, h) {
  const name = h('input', { class: 'field', placeholder: 'Layout name', 'aria-label': 'Layout name', onKeydown: (e) => e.stopPropagation() });
  if (!(await app.ui.modal('Save this slide as a layout', h('div', { style: 'display: flex; flex-direction: column; gap: 8px' }, name,
    h('p', { class: 'muted' }, 'It shows under My layouts in Slide › New slide from layout.')), [['Cancel', false], ['Save', true, true]]))) return;
  const n = name.value.trim().replace(/[^\p{L}\p{N} _-]/gu, '').trim();
  if (!n) { app.toast('Give the layout a name.'); return; }
  const all = savedLayouts();
  all[n] = call('save_layout');
  try { keepLayouts(all); app.toast(`Saved layout “${n}”.`); } catch { app.toast('That slide is too big to keep as a layout here (it has large pictures).'); }
}

/// The themes as cards (their title slide); `onPick(name)` when one is chosen.
export function themeGallery(h, chosen, onPick) {
  const themes = call('themes');
  const grid = h('div', { class: 'gallery-grid', role: 'radiogroup', 'aria-label': 'Theme' });
  const draw = () => grid.replaceChildren(...themes.list.map((t) => h('button', { class: `gallery-card${t.name === chosen ? ' on' : ''}`, role: 'radio', 'aria-checked': String(t.name === chosen), onClick: () => { chosen = t.name; onPick(t.name); draw(); } },
    h('img', { src: layoutThumb(t.name, 'Title'), alt: '', width: '160', height: '90' }), h('span', {}, t.name))));
  draw();
  return grid;
}
