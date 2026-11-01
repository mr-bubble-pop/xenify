// The Paint panel: colours for the fill or the stroke (of the selection, or
// of what you draw next): swatches, the canvas's own colours, recent ones,
// and your saved pens ("My pens").

import { call } from './engine.js';
import { icon } from './icons.js';

const SWATCHES = ['#000000', '#3a3a3a', '#7a7a7a', '#b5b5b5', '#e6e6e6', '#ffffff', '#ef476f', '#f78c6b', '#ffd166', '#06d6a0',
  '#118ab2', '#073b4c', '#8338ec', '#3a86ff', '#ff006e', '#fb5607', '#ffbe0b', '#8ac926', '#1982c4', '#6a4c93', '#e63946', '#f1faee', '#a8dadc', '#457b9d'];

const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k) || 'null') ?? d; } catch { return d; } };
const keep = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* not kept */ } };

export function paintPanel(app, h, { ask, toast }) {
  app.paintSlot ||= 'fill';
  const slot = app.paintSlot;
  const sel = app.state.selection;
  const shapes = sel.filter((id) => app.layerById(id)?.kind === 'shape');
  const texts = sel.filter((id) => app.layerById(id)?.kind === 'text');
  const P = app.layerInfo?.props || {};
  const fromLayer = slot === 'fill' ? (P.fill?.value || P.text_color?.value) : P.stroke?.value;
  const current = String(fromLayer || (slot === 'fill' ? app.fill : app.stroke));
  const apply = (hex) => {
    const recent = [hex, ...load('draw.recentColors', []).filter((c) => c !== hex)].slice(0, 12);
    keep('draw.recentColors', recent);
    if (slot === 'fill') { app.fill = hex.slice(0, 7); app.fillOn = true; } else { app.stroke = hex.slice(0, 7); app.strokeOn = true; }
    if (shapes.length) app.run('set_paint', slot === 'fill' ? { ids: shapes, fill: hex } : { ids: shapes, stroke: hex, stroke_width: Math.max(1, app.strokeWidth) });
    for (const id of texts) if (slot === 'fill') app.run('set_prop', { id, prop: 'text_color', value: hex });
    app.renderSide();
  };
  const grid = (list) => h('div', { class: 'swatches' }, list.map((c) => h('button', { class: `swatch${c.toLowerCase() === current.toLowerCase() ? ' on' : ''}`, title: c.toUpperCase(), 'aria-label': `Colour ${c}`, style: `background: ${c}`, onClick: () => apply(c) })));
  const picker = h('input', { type: 'color', class: 'color-in big', value: current.slice(0, 7), 'aria-label': 'Pick a colour', onChange: (e) => apply(e.target.value) });
  const hex = h('input', { class: 'field small', value: current.toUpperCase(), 'aria-label': 'Hex colour', onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') { const v = e.target.value.trim(); if (/^#?[0-9a-f]{6}([0-9a-f]{2})?$/i.test(v)) apply(v.startsWith('#') ? v : `#${v}`); } } });
  let docColours = [];
  try { docColours = call('doc_colors'); } catch { /* none */ }
  const pens = load('draw.pens', []);
  const strokeStyle = (() => { try { return app.full?.layer?.shapes?.find((s) => s.stroke)?.stroke?.style || null; } catch { return null; } })();
  return [h('div', { class: 'props' },
    h('div', { class: 'segs', role: 'tablist', style: 'display: flex' },
      ['fill', 'stroke'].map((k) => h('button', { class: `seg${slot === k ? ' on' : ''}`, style: 'flex: 1 1 0', role: 'tab', 'aria-selected': String(slot === k), onClick: () => { app.paintSlot = k; app.renderSide(); } }, k === 'fill' ? 'Fill' : 'Stroke'))),
    h('div', { class: 'empty', style: 'font-size: 13px' }, sel.length ? `Colours go on the ${sel.length === 1 ? 'selected layer' : `${sel.length} selected layers`}.` : 'Colours go on what you draw next.'),
    h('div', { class: 'prop' }, picker, hex),
    h('div', { class: 'lab' }, 'Swatches'), grid(SWATCHES),
    docColours.length ? [h('div', { class: 'lab' }, 'In this canvas'), grid(docColours)] : null,
    load('draw.recentColors', []).length ? [h('div', { class: 'lab' }, 'Recent'), grid(load('draw.recentColors', []))] : null,
    h('div', { class: 'lab', style: 'margin-top: 8px' }, 'My pens'),
    pens.length ? pens.map((p, i) => h('div', { class: 'prop' },
      h('button', { class: 'small-btn', style: 'flex: 1; justify-content: flex-start', title: 'Use this pen on the selected shapes', onClick: () => { if (app.run('set_pen', { ids: shapes, style: p.style }) !== null) toast(`Pen: ${p.name}`); } }, p.name),
      h('button', { class: 'small-btn', 'aria-label': `Delete ${p.name}`, html: icon('trash', 14), onClick: async () => { if (await ask(`Delete the pen “${p.name}”?`, 'Lines already drawn with it keep their look.', 'Delete')) { keep('draw.pens', pens.filter((_, j) => j !== i)); app.renderSide(); } } })))
      : h('div', { class: 'empty', style: 'font-size: 13px' }, 'Set up a pen on a stroke (Layer panel › Pen), then save it here to use again.'),
    h('button', { class: 'small-btn', disabled: !strokeStyle, title: strokeStyle ? 'Save the selected stroke’s pen' : 'Select a shape whose stroke has a pen', onClick: () => {
      const name = `${strokeStyle.kind.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase())} ${pens.length + 1}`;
      keep('draw.pens', [...pens, { name, style: { ...strokeStyle, name } }]);
      toast(`Saved the pen “${name}”.`);
      app.renderSide();
    } }, '+ Save the selected stroke’s pen'))];
}
