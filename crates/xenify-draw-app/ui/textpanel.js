// The Text section of the Layer panel: the words, font, size, colour,
// alignment, text box, spacing and text on a path.

import * as fonts from './fonts.js';

export function textSection(app, h, { info, group, propRow, colourRow, toast }) {
  const t = info.text || {};
  const P = info.props;
  const id = info.id;
  const stop = (e) => e.stopPropagation();
  const style = (args, merge) => app.run('set_text_style', { id, ...args, ...(merge ? { merge } : {}) }, merge ? { quiet: true } : {});
  const field = (label, value, unit, apply, { step = 1, min = -1000, max = 100000 } = {}) => h('div', { class: 'num' }, h('input', { type: 'text', inputmode: 'decimal', value: String(Math.round(value * 100) / 100), 'aria-label': label,
      onKeydown: (e) => {
        stop(e);
        if (e.key === 'ArrowUp' || e.key === 'ArrowDown') { e.preventDefault(); const v = (Number(e.target.value) || 0) + (e.key === 'ArrowUp' ? step : -step) * (e.shiftKey ? 10 : 1); e.target.value = String(Math.round(v * 100) / 100); apply(Math.max(min, Math.min(max, v))); }
      },
      onChange: (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) apply(Math.max(min, Math.min(max, v))); } }), h('small', {}, unit));
  const num = (label, value, unit, apply, opts, title) => h('div', { class: 'prop', title: title || '' }, h('span', { class: 'pname' }, label), h('div', { class: 'vals' }, field(label, value, unit, apply, opts)));
  const segs = (label, options, current, pick) => h('div', { class: 'prop' }, h('span', { class: 'pname' }, label),
    h('div', { class: 'segs', role: 'radiogroup', 'aria-label': label, style: 'display: flex; flex: 1' }, options.map(([v, text, title]) =>
      h('button', { class: `seg${current === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String(current === v), title: title || text, style: 'flex: 1 1 0; padding: 4px 0', onClick: () => pick(v) }, text))));

  // The words: Enter makes a new line here (on the canvas too).
  const words = h('textarea', { class: 'field', rows: String(Math.min(6, Math.max(2, (t.text || '').split('\n').length))), 'aria-label': 'Text', style: 'resize: vertical; font: inherit',
    onKeydown: stop, onInput: (e) => app.run('set_text', { id, text: e.target.value }, { quiet: true }), onChange: () => app.changed() });
  words.value = t.text || '';

  // The font: any loaded or installed family, or a font file.
  const fontIn = h('input', { class: 'field', value: t.family || 'Inter', list: 'font-families', 'aria-label': 'Font', style: 'flex: 1; min-width: 0',
    onKeydown: (e) => { stop(e); if (e.key === 'Enter') e.target.blur(); },
    onFocus: async () => { list.replaceChildren(...(await fonts.families()).map((f) => h('option', { value: f }))); },
    onChange: async (e) => {
      const family = e.target.value.trim();
      if (!family || family === t.family) return;
      if (!(await fonts.ensure(family))) toast(`${family} isn’t on this computer. Add its font file with the + button; until then it shows in Inter.`);
      app.run('set_font', { id, family });
    } });
  const list = h('datalist', { id: 'font-families' });
  const addFont = h('button', { class: 'small-btn', title: 'Add a font file (TTF or OTF)', 'aria-label': 'Add a font file', onClick: async () => {
    try {
      const got = await fonts.addFile();
      if (!got.length) return;
      app.run('set_font', { id, family: got[0] });
      toast(got.length > 1 ? `Added ${got.join(', ')}.` : `Added ${got[0]}.`);
    } catch (e) { toast(`That font file can’t be read: ${e.message || e}`); }
  } }, '+');

  const boxed = !!t.frame && !t.path;
  const rows = [
    words,
    h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Font'), fontIn, addFont, list),
    propRow(info, 'font_size', P.font_size, { unit: 'pt', step: 1, min: 1 }),
    colourRow(info, 'text_color', P.text_color),
    segs('Align', [['left', 'Left'], ['center', 'Centre'], ['right', 'Right']], t.align, (v) => style({ align: v })),
    boxed
      ? segs('In the box', [['top', 'Top'], ['middle', 'Middle'], ['bottom', 'Bottom']], t.valign === 'baseline' ? 'top' : t.valign, (v) => style({ valign: v }))
      : h('div', { class: 'prop', title: 'Which part of the text sits on its position' }, h('span', { class: 'pname' }, 'Sits on'),
        h('select', { class: 'field small', 'aria-label': 'Sits on', onChange: (e) => style({ valign: e.target.value }) },
          [['baseline', 'The first line’s baseline'], ['top', 'Its top'], ['middle', 'Its middle'], ['bottom', 'Its bottom']].map(([v, l]) => h('option', { value: v, selected: t.valign === v }, l)))),
  ];
  if (!t.path) {
    rows.push(h('label', { class: 'check-row', title: 'Wrap and align the text inside a box (drag its edges with the Text tool). Off: point text that grows as you type' },
      h('input', { type: 'checkbox', checked: boxed, onChange: (e) => style({ box: e.target.checked }) }), h('span', {}, 'Text box')));
    if (boxed) {
      const u = app.units?.() || { k: 1, unit: 'px', step: 1 };
      rows.push(h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Box'), h('div', { class: 'vals' },
        field('Box width', t.frame.w / u.k, u.unit === 'px' ? 'W' : `W ${u.unit}`, (v) => style({ box_w: v * u.k }), { step: u.step, min: 1 / u.k }),
        field('Box height', t.frame.h / u.k, u.unit === 'px' ? 'H' : `H ${u.unit}`, (v) => style({ box_h: v * u.k }), { step: u.step, min: 1 / u.k }))),
      h('div', { style: 'display: flex; gap: 14px' },
        h('label', { class: 'check-row', title: 'Break lines that are wider than the box' }, h('input', { type: 'checkbox', checked: t.frame.wrap, onChange: (e) => style({ wrap: e.target.checked }) }), h('span', {}, 'Wrap lines')),
        h('label', { class: 'check-row', title: 'Don’t draw text that doesn’t fit in the box' }, h('input', { type: 'checkbox', checked: t.frame.clip, onChange: (e) => style({ clip: e.target.checked }) }), h('span', {}, 'Hide overflow'))));
    }
  }
  const spacing = h('details', { class: 'sect', open: app.openSections.textSpacing ?? false, onToggle: (e) => { app.openSections.textSpacing = e.target.open; } }, h('summary', {}, h('span', {}, 'Spacing and path')),
    h('div', { class: 'sect-body' },
      num('Letter spacing', t.letter_spacing || 0, 'px', (v) => style({ letter_spacing: v }), { step: 0.5, min: -50, max: 500 }),
      num('Word spacing', t.word_spacing || 0, 'px', (v) => style({ word_spacing: v }), { step: 1, min: -50, max: 1000 }),
      num('Line height', t.line_height || 0, '×', (v) => style({ line_height: v }), { step: 0.05, min: 0, max: 5 }, 'Distance between lines, as a multiple of the font size (0: the font’s own)'),
      pathRows()));
  function pathRows() {
    const shapes = (app.state.layers || []).filter((l) => l.kind === 'shape' && l.id !== id);
    const pick = h('select', { class: 'field small', 'aria-label': 'Text on a path', onChange: (e) => style({ path: e.target.value ? { layer: Number(e.target.value) } : null }) },
      h('option', { value: '', selected: !t.path }, 'None (straight lines)'),
      shapes.map((l) => h('option', { value: String(l.id), selected: t.path?.layer === l.id }, l.name)));
    const out = [h('div', { class: 'prop', title: 'Set the text along the outline of a shape layer' }, h('span', { class: 'pname' }, 'On path'), pick)];
    if (t.path) {
      out.push(num('Start', t.path.offset, 'px', (v) => style({ path: { offset: v } }), { step: 1, min: -100000 }),
        h('label', { class: 'check-row', title: 'Follow the outline the other way round (e.g. text inside a circle)' }, h('input', { type: 'checkbox', checked: t.path.flip, onChange: (e) => style({ path: { flip: e.target.checked } }) }), h('span', {}, 'Other way round')));
    }
    return out;
  }
  return [group('Text', ...rows), spacing];
}
