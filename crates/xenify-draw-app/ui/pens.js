// The Pens panel: every pen drawn as a sample stroke, your own pens, and the
// chosen pen's settings. A pen goes on the selected strokes, and (ticked) on
// every stroke drawn from now on.

import { call, penThumb } from './engine.js';
import { penSettings } from './look.js';

const MINE = 'draw.myPens';
const mine = () => { try { return JSON.parse(localStorage.getItem(MINE) || '[]'); } catch { return []; } };
const keepMine = (list) => { try { localStorage.setItem(MINE, JSON.stringify(list)); } catch { /* not kept */ } };

export async function pensDialog(app, h) {
  const kinds = (app.pens ||= call('pens'));
  const presets = kinds.map((k) => ({ label: k.label, style: call('pen_preset', { kind: k.kind }) }));
  const first = app.layerInfo && app.full?.layer?.shapes?.find((s) => s.stroke)?.stroke;
  const ns = call('new_stroke');
  let cur = JSON.parse(JSON.stringify(first?.style || ns?.style || presets[0].style));
  let forNew = !!ns;
  const hasSel = !!first;
  const body = h('div', { class: 'pens-box' });
  const apply = (style) => {
    cur = JSON.parse(JSON.stringify(style));
    if (hasSel) app.run('set_pen', { style: cur.kind === 'solid' ? null : cur });
    if (forNew || !hasSel) { forNew = true; call('set_new_stroke', { stroke: { ...(call('new_stroke') || {}), style: cur } }); }
    draw();
  };
  const tile = (label, style, extra) => h('button', { class: `pen-tile${JSON.stringify(style) === JSON.stringify(cur) ? ' on' : ''}`, onClick: () => apply(style), title: label },
    h('img', { src: penThumb(style), alt: '', width: '120', height: '40' }), h('span', {}, label), extra);
  function draw() {
    const my = mine();
    body.replaceChildren(...[
      h('div', { class: 'pen-grid' }, presets.map((p) => tile(p.label, p.style))),
      my.length ? h('div', { class: 'lab' }, 'My pens') : null,
      my.length ? h('div', { class: 'pen-grid' }, my.map((p, i) => tile(p.name, p.style, h('span', { class: 'gallery-x', role: 'button', tabindex: '0', 'aria-label': `Forget ${p.name}`, title: `Forget ${p.name}`,
        onClick: (e) => { e.stopPropagation(); const l = mine(); l.splice(i, 1); keepMine(l); draw(); } }, '×')))) : null,
      h('label', { class: 'check-row', title: 'Strokes you draw from now on use this pen' }, h('input', { type: 'checkbox', checked: forNew, onChange: (e) => { forNew = e.target.checked; call('set_new_stroke', { stroke: forNew ? { ...(call('new_stroke') || {}), style: cur } : null }); } }), h('span', {}, 'Use it for new strokes too')),
      cur.kind !== 'solid' ? h('div', { class: 'lab' }, 'This pen') : null,
      cur.kind !== 'solid' ? penSettings(h, cur, (f) => { f(cur); apply(cur); }) : null,
      h('div', { class: 'add-row' },
        cur.kind !== 'solid' ? h('button', { class: 'small-btn', title: 'The same pen with a different random wobble', onClick: () => { cur.seed = Math.floor(Math.random() * 9999); apply(cur); } }, 'Shuffle') : null,
        h('button', { class: 'small-btn', title: 'This pen as it comes', onClick: () => apply(call('pen_preset', { kind: cur.kind })) }, 'Reset'),
        h('button', { class: 'small-btn', onClick: async () => {
          const name = h('input', { class: 'field', placeholder: 'Name', 'aria-label': 'Pen name', onKeydown: (e) => e.stopPropagation() });
          if (await app.ui.modal('Save this pen', name, [['Cancel', false], ['Save', true, true]]) && name.value.trim()) { keepMine([...mine(), { name: name.value.trim(), style: cur }]); draw(); }
        } }, 'Save as my pen'))].filter(Boolean));
  }
  draw();
  await app.ui.modal(hasSel ? 'Pens (for the selected strokes)' : 'Pens (for new strokes)', body, [['Done', true, true]]);
}
