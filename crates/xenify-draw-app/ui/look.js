// The Layer panel's look sections: fill and stroke of each shape (colour,
// gradient, pen, caps, dashes), the shape's own settings, draw-on, blend
// mode, effects, modifiers, masks and the mask layer. Animatable values are
// property rows (the diamond animates them); the rest edits the layer as
// stored (`layer_full`) and sends it back.

import { call } from './engine.js';
import { icon } from './icons.js';
import { easingLabel } from './timeline.js';

const clone = (v) => JSON.parse(JSON.stringify(v));
export const toHex = (c) => {
  const b = (v) => Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16).padStart(2, '0');
  return `#${b(c.r)}${b(c.g)}${b(c.b)}${c.a >= 1 ? '' : b(c.a)}`;
};
export const fromHex = (s) => {
  const n = (i) => parseInt(s.slice(i, i + 2), 16) / 255;
  return { r: n(1), g: n(3), b: n(5), a: s.length >= 9 ? n(7) : 1 };
};

const EFFECTS = [['drop_shadow', 'Drop shadow'], ['glow', 'Glow'], ['blur', 'Blur'], ['adjust_colors', 'Adjust colours']];
export const EFFECT_KINDS = EFFECTS;
const EFFECT_NAME = Object.fromEntries(EFFECTS);
const BLENDS = ['normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color_dodge', 'color_burn', 'hard_light', 'soft_light', 'difference', 'exclusion', 'hue', 'saturation', 'color', 'luminosity', 'add'];
const MASK_MODES = [['add', 'Add'], ['subtract', 'Subtract'], ['intersect', 'Intersect'], ['difference', 'Difference']];
const MATTE_MODES = [['alpha', 'Inside its shape'], ['alpha_inverted', 'Outside its shape'], ['luma', 'Where it’s bright'], ['luma_inverted', 'Where it’s dark']];
const MOD_NAME = { array: 'Array', jitter: 'Jitter', zig_zag: 'Zig zag', pucker_bloat: 'Pucker & bloat', twist: 'Twist', smooth: 'Smooth', subdivide: 'Subdivide', simplify: 'Simplify', offset_path: 'Offset path', mirror: 'Mirror', blend: 'Blend', path_array: 'Array along path', shatter: 'Shatter' };
const words = (s) => s.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase());

/// A property's name for people: "effects.0.radius" → "Blur" (for a shadow), "shapes.1.stroke_width" → "Stroke width".
export function labelFor(path, layer, short = false) {
  const own = /^(emitter|liquid|parts\.\d+)\.(.+)$/.exec(path);
  if (own) return { color: 'Colour', level: 'Level', tilt: 'Tilt', wave_height: 'Wave height' }[own[2]] || words(own[2]);
  const m = /^(shapes|effects|modifiers|masks)\.(\d+)\.(.+)$/.exec(path);
  if (!m) return { position: 'Position', scale: 'Scale', rotation: 'Rotation', opacity: 'Opacity', anchor: 'Transform origin', skew: 'Skew', skew_axis: 'Skew direction', font_size: 'Text size', text_color: 'Text colour', fill: 'Fill', stroke: 'Stroke', stroke_width: 'Stroke width', size: 'Size', corner_radius: 'Corners', distort: 'Perspective', volume: 'Volume', rotation_x: 'Turn X', rotation_y: 'Turn Y', rotation_z: 'Turn Z', solid_size: 'Size', solid_depth: 'Depth', solid_color: 'Colour', camera_zoom: 'Zoom', shake_amount: 'Shake', shake_rotation: 'Shake turn' }[path] || words(path);
  const [, list, i, field] = m;
  const f = { fill_color: 'Fill', stroke_color: 'Stroke', stroke_width: 'Stroke width', corner_radius: 'Corners', count: 'Count', inner: 'Inner size', hole: 'Hole', trim_start: 'Draw from', trim_end: 'Draw to', trim_offset: 'Draw offset', gradient_start: 'Gradient start', gradient_end: 'Gradient end', gradient_colors: 'Gradient colours', dash_offset: 'Dash offset', position: 'Offset', color: 'Colour', end_opacity: 'Last one’s opacity' }[field] || words(field);
  if (list === 'effects') {
    const kind = layer?.effects?.[i]?.effect;
    const fx = kind === 'drop_shadow' && field === 'radius' ? 'Blur' : f;
    if (short) return fx;
    return `${EFFECT_NAME[kind] || 'Effect'}: ${fx.toLowerCase()}`;
  }
  if (short) return f;
  if (list === 'modifiers') return `${MOD_NAME[layer?.modifiers?.[i]?.modifier] || 'Modifier'}: ${f.toLowerCase()}`;
  if (list === 'masks') return `${layer?.masks?.[i]?.name || `Mask ${+i + 1}`}: ${f.toLowerCase()}`;
  const n = layer?.shapes?.length || 1;
  return n > 1 ? `${layer.shapes[i]?.name || `Shape ${+i + 1}`}: ${f.toLowerCase()}` : f;
}

export function lookSections(app, h, ui) {
  const info = app.layerInfo;
  const full = app.full;
  if (!info || !full) return [];
  const L = full.layer;
  const P = info.props;
  const id = info.id;
  const out = [];
  const row = (path, opts, short) => (P[path] ? ui.propRow(info, path, P[path], { ...opts, label: labelFor(path, L, short) }) : null);
  const crow = (path, short) => (P[path] ? colourRow(app, h, ui, info, path, P[path], labelFor(path, L, short)) : null);
  const editMeta = (label, f, merge) => { const meta = clone(L); f(meta); app.run('set_meta_json', { id, meta, label, merge }); };
  const editShape = (i, label, f, merge) => { const s = clone(L.shapes[i]); f(s); app.run('set_shape_json', { id, index: i, shape: s, label, merge }); };
  const select = (label, value, options, onChange) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
    h('select', { class: 'field small', 'aria-label': label, onChange: (e) => onChange(e.target.value) }, options.map(([v, t]) => h('option', { value: v, selected: v === value }, t))));
  const check = (label, on, onChange, tip) => h('label', { class: 'check-row', title: tip || '' }, h('input', { type: 'checkbox', checked: on, onChange: (e) => onChange(e.target.checked) }), h('span', {}, label));
  const section = (key, title, kids, { open, extra } = {}) => {
    const d = h('details', { class: 'sect', open: app.openSections[key] ?? open ?? true }, h('summary', {}, h('span', {}, title), extra || null), h('div', { class: 'sect-body' }, kids));
    d.addEventListener('toggle', () => { app.openSections[key] = d.open; });
    return d;
  };
  const small = (label, run, tip, iconName) => h('button', { class: 'small-btn', title: tip || label, 'aria-label': tip || label, onClick: run, html: iconName ? icon(iconName, 14) : null }, iconName ? null : label);

  // ----- Connectors: how the line runs, its ends, arrowheads -----
  const hexOf = (c) => (c ? `#${[c.r, c.g, c.b].map((v) => Math.round(v * 255).toString(16).padStart(2, '0')).join('')}` : '#000000');
  const colorOf = (x) => ({ r: parseInt(x.slice(1, 3), 16) / 255, g: parseInt(x.slice(3, 5), 16) / 255, b: parseInt(x.slice(5, 7), 16) / 255, a: 1 });
  const num2 = (label, value, apply, { step = 1, min = 0, max = 1000, title } = {}) => h('label', { class: 'prop', title: title || '' }, h('span', { class: 'pname' }, label),
    h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(Math.round(value * 100) / 100), 'aria-label': label,
      onChange: (e) => apply(Math.max(min, Math.min(max, Number(e.target.value) || 0))), onKeydown: (e) => e.stopPropagation() }));
  if (L.connector) {
    const c = L.connector;
    const setC = (label, f) => editMeta(label, (m) => { f(m.connector); });
    const endRow = (label, key) => {
      const end = c[key];
      const a = end.attach;
      const name = a ? (app.layerById(a.layer)?.name || '(deleted)') : null;
      return h('div', { class: 'prop' }, h('span', { class: 'pname' }, label),
        h('span', { class: 'muted', style: 'flex: 1; font-size: 13px' }, a ? `${name}, ${a.port === 'auto' ? 'best side' : `port ${(a.port.port ?? 0) + 1}`}` : 'not glued'),
        a && a.port !== 'auto' ? small('Any side', () => setC('Any side', (x) => { x[key].attach.port = 'auto'; }), 'Let the route pick the side') : null,
        a ? small('Detach', () => {
          const r = call('connector_ends', { id });
          setC('Detach', (x) => { x[key].attach = null; if (r) x[key].point = { x: r[key === 'from' ? 0 : 1][0], y: r[key === 'from' ? 0 : 1][1] }; });
        }, 'Unglue this end; it stays where it is') : null);
    };
    out.push(section('connector', 'Connector', [
      select('Route', c.route || 'orthogonal', [['orthogonal', 'Elbows'], ['straight', 'Straight'], ['curved', 'Curved']], (v) => setC('Route', (x) => { x.route = v; })),
      (c.route || 'orthogonal') === 'orthogonal' ? num2('Corners (px)', c.corner_radius ?? 0, (v) => setC('Corners', (x) => { x.corner_radius = v; }), { max: 200, title: 'Rounds the bends (0: sharp)' }) : null,
      select('Jumps', c.jump || 'arc', [['arc', 'Arc'], ['gap', 'Gap'], ['square', 'Square'], ['none', 'None']], (v) => setC('Jumps', (x) => { x.jump = v; })),
      c.jump !== 'none' ? num2('Jump size (px)', c.jump_size ?? 5, (v) => setC('Jump size', (x) => { x.jump_size = v; }), { min: 1, max: 50, step: 0.5, title: 'Half the width of a jump where lines cross' }) : null,
      check('Go around shapes', c.avoid !== false, (on) => setC('Avoid', (x) => { x.avoid = on; }), 'Find a way around shapes in between (off: straight through them)'),
      num2('Spacing (px)', c.margin ?? 16, (v) => setC('Spacing', (x) => { x.margin = v; }), { max: 200, title: 'Space kept around shapes, and how far the line leaves a shape before turning' }),
      endRow('Start', 'from'), endRow('End', 'to'),
      check('Flow (marks moving along it)', !!c.flow, (on) => setC('Flow', (x) => { x.flow = on ? { speed: 60, kind: 'dashes' } : null; })),
      c.flow ? select('Marks', c.flow.kind || 'dashes', [['dashes', 'Dashes'], ['dots', 'Dots']], (v) => setC('Flow', (x) => { x.flow.kind = v; })) : null,
      c.flow ? num2('Speed (px/s)', c.flow.speed ?? 60, (v) => setC('Flow speed', (x) => { x.flow.speed = v; }), { min: -2000, max: 2000, title: 'How fast (negative: backwards)' }) : null,
      L.label ? num2('Label along (%)', (c.label_at ?? 0.5) * 100, (v) => setC('Label place', (x) => { x.label_at = v / 100; }), { max: 100, title: 'Where the label sits along the line' }) : null,
      h('div', { class: 'add-row' },
        small('Swap ends', () => setC('Swap ends', (x) => { [x.from, x.to] = [x.to, x.from]; }), 'Run the line the other way'),
        (c.waypoints || []).length ? small('Reset route', () => setC('Reset route', (x) => { x.waypoints = []; }), 'Forget dragged bends; route automatically') : null),
    ], { open: true }));
  }
  // Arrowheads on open lines.
  const lines = (L.shapes || []).map((sh, i) => [sh, i]).filter(([sh]) => sh.stroke && (L.connector || (sh.geometry?.type === 'path' && !/[Zz]\s*$/.test(sh.geometry?.path?.static ?? sh.geometry?.path ?? ''))));
  if (lines.length) {
    const MARKS = [['none', 'None'], ['triangle', 'Triangle'], ['open_triangle', 'Hollow triangle'], ['open', 'Open (→)'], ['arrow', 'Arrow'], ['diamond', 'Diamond'], ['open_diamond', 'Open diamond'], ['circle', 'Circle'], ['open_circle', 'Open circle'], ['bar', 'Bar'], ['crows_foot', 'Crow’s foot']];
    const setStroke = (i, f) => { const sh = clone(L.shapes[i]); f(sh.stroke); app.run('set_shape_json', { id, index: i, shape: sh, label: 'Arrowheads' }); };
    out.push(section('arrows', 'Arrowheads', [
      ...lines.flatMap(([sh, i]) => [
      lines.length > 1 ? h('div', { class: 'lab' }, sh.name || `Line ${i + 1}`) : null,
      ...[['Start', 'start_marker'], ['End', 'end_marker']].flatMap(([label, key]) => {
        const m = sh.stroke[key] || { kind: 'none', size: 1 };
        return [select(label, m.kind || 'none', MARKS, (v) => setStroke(i, (st) => { st[key] = { ...(st[key] || { size: 1 }), kind: v }; })),
          m.kind && m.kind !== 'none' ? num2(`${label} size (×)`, m.size ?? 1, (v) => setStroke(i, (st) => { st[key].size = v; }), { min: 0.2, max: 6, step: 0.1, title: 'Size, relative to the line’s width' }) : null];
      }),
    ])], { open: !!L.connector }));
  }

  // ----- The label: text inside a shape or along a connector -----
  if (L.label || L.connector || (L.type === 'shape' && app.state.comp.kind === 'diagram')) {
    const lb = L.label || null;
    const setLb = (label, f) => editMeta(label, (m) => { m.label ||= { text: '', font_family: 'Inter', font_size: 16, color: { r: 0.1, g: 0.1, b: 0.12, a: 1 }, align: 'center', inset: 6 }; f(m.label); if (!m.label.text) m.label = null; });
    out.push(section('label', 'Label', [
      h('textarea', { class: 'field', rows: '2', 'aria-label': 'Label', placeholder: 'Text inside the shape (or double-click it on the canvas)', style: 'resize: vertical; font: inherit',
        onKeydown: (e) => e.stopPropagation(), onChange: (e) => app.run('set_label', { id, text: e.target.value }) }, lb?.text || ''),
      lb ? num2('Size (px)', lb.font_size ?? 16, (v) => setLb('Label size', (x) => { x.font_size = v; }), { min: 4, max: 400 }) : null,
      lb ? h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Colour'), h('input', { type: 'color', class: 'color-in', value: hexOf(lb.color), onChange: (e) => setLb('Label colour', (x) => { x.color = colorOf(e.target.value); }) })) : null,
      lb ? select('Align', lb.align || 'center', [['left', 'Left'], ['center', 'Centre'], ['right', 'Right']], (v) => setLb('Label align', (x) => { x.align = v; })) : null,
      lb && L.connector ? check('A box behind it', !!lb.background, (on) => setLb('Label box', (x) => { x.background = on ? colorOf(app.state.comp.background.slice(0, 7)) : null; }), 'A box behind the label, which then sits on the line') : null,
      lb?.background ? h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Box colour'), h('input', { type: 'color', class: 'color-in', value: hexOf(lb.background), onChange: (e) => setLb('Label box', (x) => { x.background = colorOf(e.target.value); }) })) : null,
    ], { open: !!lb || app.state.comp.kind === 'diagram' }));
  }

  // ----- Ports: where connectors glue to a shape -----
  if (!L.connector && ['shape', 'precomp', 'image'].includes(L.type) && app.state.comp.kind === 'diagram') {
    const ports = (L.ports && L.ports.length ? L.ports : [{ pos: { x: 0.5, y: 0 }, side: 'top' }, { pos: { x: 1, y: 0.5 }, side: 'right' }, { pos: { x: 0.5, y: 1 }, side: 'bottom' }, { pos: { x: 0, y: 0.5 }, side: 'left' }]);
    const setP = (f) => editMeta('Ports', (m) => { m.ports = clone(ports); f(m.ports); });
    out.push(section('ports', L.ports?.length ? 'Ports (your own)' : 'Ports', [
      h('div', { class: 'muted', style: 'font-size: 13px' }, 'Where connectors glue: across and down the shape’s box (0–100%), and which way lines leave.'),
      ...ports.map((pt, i) => h('div', { class: 'prop' }, h('span', { class: 'pname', style: 'width: 24px' }, String(i + 1)),
        ...['x', 'y'].map((k) => h('input', { class: 'field small', type: 'number', min: '0', max: '100', step: '5', value: String(Math.round(pt.pos[k] * 100)), 'aria-label': `Port ${i + 1} ${k === 'x' ? 'across' : 'down'} (%)`,
          onChange: (e) => setP((list) => { list[i].pos[k] = Math.max(0, Math.min(100, Number(e.target.value) || 0)) / 100; }), onKeydown: (e) => e.stopPropagation() })),
        h('select', { class: 'field small', 'aria-label': `Port ${i + 1} side`, onChange: (e) => setP((list) => { list[i].side = e.target.value; }) }, ['top', 'right', 'bottom', 'left'].map((sd) => h('option', { value: sd, selected: pt.side === sd }, words(sd)))),
        small('Remove', () => setP((list) => { list.splice(i, 1); }), 'Remove this port', 'trash'))),
      h('div', { class: 'add-row' }, small('+ Port', () => setP((list) => { list.push({ pos: { x: 0.5, y: 0.5 }, side: 'top' }); })),
        L.ports?.length ? small('Back to the sides’ middles', () => editMeta('Ports', (m) => { m.ports = []; })) : null),
    ]));
  }

  // ----- Look: blend mode -----
  out.push(section('look', 'Blending', [
    select('Blend', L.blend || 'normal', BLENDS.map((b) => [b, words(b)]), (v) => editMeta('Blend mode', (m) => { m.blend = v; })),
  ], { open: (L.blend || 'normal') !== 'normal' }));

  // ----- Animation: ready-made animations, wiggle, parent, expressions -----
  if (app.state.comp.duration) {
    const presets = app.animPresets || (app.animPresets = call('anim_presets'));
    const n = app.state.selection.length;
    const wig = L.transform?.wiggle;
    const wnum = (label, key, { k = 1, step = 1, min = 0, max = 10000 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
      h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(Math.round((wig?.[key] ?? 0) * k * 100) / 100), 'aria-label': label,
        onChange: (e) => app.run('set_wiggle', { id, wiggle: { ...wig, [key]: Math.max(min, Math.min(max, Number(e.target.value))) / k } }), onKeydown: (e) => e.stopPropagation() }));
    const exprs = L.expressions || [];
    const animatable = Object.keys(P).filter((p) => typeof P[p].value === 'number' || (Array.isArray(P[p].value) && typeof P[p].value[0] === 'number') || /^#/.test(String(P[p].value)));
    const exprRow = (ex) => {
      const out = h('small', { class: 'expr-out' });
      const show = (src) => { const r = call('expr_preview', { id, prop: ex.path, source: src }); out.textContent = r.error ? r.error : `→ ${JSON.stringify(r.ok)}`; out.classList.toggle('bad', !!r.error); };
      const input = h('input', { class: 'field small mono', value: ex.source, 'aria-label': `Expression on ${labelFor(ex.path, L)}`, placeholder: 'value + wiggle(2, 20)',
        onInput: (e) => show(e.target.value), onChange: (e) => app.run('set_expression', { id, prop: ex.path, source: e.target.value }), onKeydown: (e) => e.stopPropagation() });
      show(ex.source);
      return h('div', { class: 'item-card' }, h('div', { class: 'card-head' }, h('strong', {}, labelFor(ex.path, L)), h('span', { class: 'spacer' }),
        small('Remove', () => app.run('set_expression', { id, prop: ex.path, source: '' }), 'Remove the expression', 'trash')), input, out);
    };
    // A ready-made animation with its settings (kept between uses), and the gap between layers.
    function presetEditor() {
      const st = (app.presetUI ||= { name: presets[0].name, preset: clone(presets[0].preset), stagger: 0.15 });
      const LABELS = { duration: ['Takes', 's', 0.05, 0.01, 60], overshoot: ['Overshoot', '%', 1, 0, 200, 100], distance: ['Distance', 'px', 10, 0, 10000], height: ['Height', 'px', 5, 0, 10000],
        bounces: ['Bounces', '', 1, 0, 6], strength: ['Squash', '%', 1, 0, 100, 100], amount: ['Amount', 'px', 1, 0, 1000], frequency: ['Wiggles a second', '', 0.5, 0.1, 60] };
      const rows = Object.entries(st.preset).filter(([k]) => k !== 'preset').map(([k, v]) => {
        if (k === 'from') return select('From', v, [['left', 'The left'], ['right', 'The right'], ['up', 'Above'], ['down', 'Below']], (x) => { st.preset.from = x; });
        const [label, unit, step, min, max, mult = 1] = LABELS[k] || [k, '', 0.1, 0, 10000];
        return h('label', { class: 'prop' }, h('span', { class: 'pname' }, unit ? `${label} (${unit})` : label),
          h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(Math.round(v * mult * 100) / 100), 'aria-label': label,
            onKeydown: (e) => e.stopPropagation(), onChange: (e) => { const x = Math.max(min, Math.min(max, Number(e.target.value) || 0)) / mult; st.preset[k] = k === 'bounces' ? Math.round(x) : x; } }));
      });
      return [
        select('Animation', st.name, presets.map((pr) => [pr.name, pr.name]), (v) => { const pr = presets.find((x) => x.name === v); st.name = v; st.preset = clone(pr.preset); app.renderSide(); }),
        rows,
        n > 1 ? h('label', { class: 'prop', title: 'Time between one layer starting and the next' }, h('span', { class: 'pname' }, 'Gap (s)'),
          h('input', { class: 'field small', type: 'number', step: '0.05', min: '0', max: '10', value: String(st.stagger), 'aria-label': 'Gap between layers',
            onKeydown: (e) => e.stopPropagation(), onChange: (e) => { st.stagger = Math.max(0, Math.min(10, Number(e.target.value) || 0)); } })) : null,
        h('button', { class: 'small-btn', title: 'Replaces the animation of the properties this one uses', onClick: () => app.run('apply_preset', { preset: st.preset, stagger: n > 1 ? st.stagger : 0 }) },
          n > 1 ? `Apply to ${n} layers` : 'Apply'),
      ];
    }
    out.push(section('anim', 'Animation', [
      h('div', { class: 'lab' }, n > 1 ? `Ready-made, from the playhead (${n} layers, one after another)` : 'Ready-made, from the playhead'),
      presetEditor(),
      h('div', { class: 'sub-head' }, check('Wiggle', !!wig, (on) => app.run('set_wiggle', { id, wiggle: on ? { frequency: 2, position: 12, rotation: 0, scale: 0, seed: id } : null }), 'Random drifting, like a hand-held camera')),
      wig ? [wnum('Wiggles a second', 'frequency', { step: 0.5, max: 60 }), wnum('Drift px', 'position'), wnum('Turn °', 'rotation', { max: 360 }), wnum('Grow %', 'scale', { k: 100, max: 100 }), wnum('Pattern', 'seed', { max: 999999 })] : null,
      select('Parent', L.parent != null ? String(L.parent) : '', [['', 'None'], ...full.others.map((o) => [String(o.id), o.name])], (v) => app.run('set_parent', { id, parent: v ? Number(v) : null })),
      h('div', { class: 'lab', title: 'Names: time, value, frame, fps, duration, index, pi. Functions: wiggle(freq, amount), random(), noise(x), value_at(t), loop_out(), loop_in(), ping_pong(), sin, cos, abs, min, max, clamp, lerp, smoothstep, round, floor. Vectors: [x, y], value.x.' }, 'Expressions (hover for what you can write)'),
      exprs.map(exprRow),
      h('select', { class: 'field small', 'aria-label': 'Add an expression to a property', onChange: (e) => { const v = e.target.value; e.target.value = ''; if (v) app.run('set_expression', { id, prop: v, source: 'value' }); } },
        h('option', { value: '' }, '+ Expression on…'), animatable.filter((p) => !exprs.some((x) => x.path === p)).map((p) => h('option', { value: p }, labelFor(p, L)))),
    ], { open: !!wig || exprs.length > 0 }));
  }

  // ----- A sound -----
  if (L.type === 'audio' || L.type === 'video') {
    const setL = (label, f) => { const l = clone(L); f(l); app.run('set_layer_json', { layer: l, label }); };
    const num = (label, key, { min = 0, max = 600, step = 0.1 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
      h('input', { class: 'field small', type: 'number', min: String(min), max: String(max), step: String(step), value: String(L[key] ?? 0), 'aria-label': label,
        onChange: (e) => setL(label, (l) => { l[key] = Math.max(min, Math.min(max, Number(e.target.value) || 0)); }), onKeydown: (e) => e.stopPropagation() }));
    const kids = [
      row('volume', { unit: '%', k: 100, step: 1, min: 0, max: 400 }),
      check('Muted', !!L.muted, (on) => setL('Mute', (l) => { l.muted = on; })),
      check('Play it over and over', !!L.looped, (on) => setL('Loop sound', (l) => { l.looped = on; })),
      L.type === 'audio' ? [num('Fade in (s)', 'fade_in'), num('Fade out (s)', 'fade_out')] : null,
      num('Speed', 'speed', { min: 0.1, max: 4, step: 0.05 }), num('Start in the sound (s)', 'source_in'),
    ];
    // The clip: what it is, where it's kept, and fitting it to the frame or its length.
    let m = null;
    try { m = call('media_info', { id }); } catch { /* none */ }
    if (m) {
      const facts = [m.video ? `${m.width} × ${m.height}` : null, m.video ? `${Math.round(m.fps * 100) / 100} frames a second` : null, `${m.duration.toFixed(2)} s`, m.video ? (m.has_audio ? 'with sound' : 'no sound') : null].filter(Boolean);
      kids.unshift(h('div', { class: 'muted', style: 'font-size: 13px' }, `${m.name} · ${facts.join(' · ')}`),
        m.linked ? h('div', { class: 'muted', style: 'font-size: 12.5px; overflow-wrap: anywhere', title: 'The canvas refers to this file: keep it where it is' }, `Linked: ${m.linked}`)
          : h('div', { class: 'muted', style: 'font-size: 12.5px' }, `Inside the canvas file (${(m.size / 1e6).toFixed(1)} MB)`),
        m.linked ? small('Keep inside the canvas file', async () => {
          try {
            const bytes = await app.clipBytes(m.asset);
            let b = '';
            for (let i = 0; i < bytes.length; i += 0x8000) b += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            app.run('embed_media', { id: m.asset, base64: btoa(b) });
          } catch (e) { app.toast?.(`Couldn’t read ${m.linked}: ${e.message || e}`); }
        }, 'The clip travels with the canvas (the file gets bigger)') : null);
      kids.push(h('div', { class: 'add-row' },
        m.video ? small('Fit frame', () => app.run('media_fit', { id, fill: false }), 'The whole picture inside the frame') : null,
        m.video ? small('Fill frame', () => app.run('media_fit', { id, fill: true }), 'Cover the frame (edges may be cut off)') : null,
        small('Trim to clip', () => app.run('media_trim', { id }), 'Make the layer as long as the clip plays')));
    }
    out.push(section('sound', L.type === 'video' ? 'Video' : 'Sound', kids));
  }

  // ----- 3D -----
  if (L.type === 'solid') {
    const setS = (label, f) => { const sol = clone(L); delete sol.type; for (const k of ['id', 'name', 'visible', 'locked', 'in_point', 'out_point', 'parent', 'transform', 'effects', 'masks', 'modifiers', 'blend', 'matte', 'expressions']) delete sol[k]; f(sol); app.run('set_solid_json', { id, solid: sol, label }); };
    const kids = [
      h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Perspective %'),
        h('input', { class: 'field small', type: 'number', min: '0', max: '100', step: '5', value: String(Math.round((L.perspective ?? 0) * 100)), 'aria-label': 'Perspective (%)',
          onChange: (e) => setS('Perspective', (x) => { x.perspective = Math.max(0, Math.min(100, Number(e.target.value))) / 100; }), onKeydown: (e) => e.stopPropagation() })),
      row('rotation_x', { unit: '°', step: 1 }), row('rotation_y', { unit: '°', step: 1 }), row('rotation_z', { unit: '°', step: 1 }),
      row('solid_size', { unit: 'px', step: 1, min: 1 }), row('solid_depth', { unit: 'px', step: 1, min: 0 }), crow('solid_color'),
    ];
    // Its shape, or the model it is: colours and parts.
    const si = call('solid_info', { id });
    const op = (args) => app.run('solid_op', { id, ...args });
    if (!si.model) {
      if (si.shape !== 'Extrusion') kids.unshift(select('Shape', si.shape, (app.solidShapes ||= call('solid_shapes')).map((n) => [n, n]), (v) => op({ shape: v })));
      if (si.thickness != null) kids.splice(1, 0, h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Thickness %'),
        h('input', { class: 'field small', type: 'number', min: '5', max: '50', step: '1', value: String(Math.round(si.thickness * 100)), 'aria-label': 'Ring thickness (%)',
          onChange: (e) => op({ thickness: Number(e.target.value) / 100 }), onKeydown: (e) => e.stopPropagation() })));
    } else {
      const m = si.model;
      kids.unshift(h('div', { class: 'muted', title: `${m.faces} faces. Size sets its largest dimension in pixels.` }, `Model: ${m.name}${m.parts.length > 1 ? ` · ${m.parts.length} parts` : ''}`),
        m.has_colors ? check('Use the model’s colours', m.own_colors, (on) => op({ own_colors: on }), 'Off: every part takes the Colour property (which can be animated)') : null);
      if (m.parts.length > 1) {
        kids.push(h('div', { class: 'lab' }, `Parts (${m.parts.length})`),
          h('div', { class: 'add-row' },
            h('button', { class: 'small-btn', title: 'Animates every part outward from the middle over the next second (keyframes from the playhead)', onClick: () => { if (op({ explode: true }) !== null) app.toast?.('Exploded the parts over one second from the playhead: play to see it; Undo removes it.'); } }, 'Explode'),
            m.any_moved ? h('button', { class: 'small-btn', title: 'Put every part back where it was', onClick: () => op({ reset_parts: true }) }, 'Reset parts') : null),
          m.parts.map((part, i) => h('div', { class: 'item-card' },
            h('div', { class: 'card-head' }, check(part.name, part.visible, (on) => op({ part: i, visible: on }), 'Show / hide this part'), h('span', { class: 'spacer' }),
              part.moved ? null : small('Move…', () => op({ part: i }), 'Move or turn this part on its own')),
            part.moved ? Object.keys(P).filter((p) => p.startsWith(`parts.${i}.`)).map((p) => row(p, ui.unitsFor(p), true)) : null)));
      }
    }
    out.push(section('solid', '3D', kids));
  }

  // ----- A placed symbol -----
  if (L.type === 'precomp') {
    const sym = call('symbols').list.find((x) => x.id === L.comp);
    out.push(section('symbol', 'Symbol', [
      h('div', { class: 'empty', style: 'font-size: 13px' }, `Shows “${sym?.name || 'a symbol'}”. Edit it to change every copy.`),
      h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Starts at (s)'),
        h('input', { class: 'field small', type: 'number', step: '0.1', value: String(L.start ?? 0), 'aria-label': 'Starts at (seconds)',
          onChange: (e) => app.run('set_symbol_timing', { id, start: Number(e.target.value) || 0 }), onKeydown: (e) => e.stopPropagation() })),
      check('Play it over and over', !!L.looped, (on) => app.run('set_symbol_timing', { id, loop: on })),
      h('button', { class: 'small-btn', onClick: () => app.editSymbol(L.comp) }, 'Edit the symbol'),
    ]));
  }

  // ----- Animated pictures (GIF, WebP, PNG): speed, where it starts, loop -----
  if (info.kind === 'picture') {
    let pb = null;
    try { pb = call('image_playback', { id }); } catch { /* a still picture */ }
    if (pb) {
      const num = (label, key, { step, min, max, unit, title }) => h('label', { class: 'prop', title }, h('span', { class: 'pname' }, `${label} (${unit})`),
        h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(Math.round(pb[key] * 100) / 100), 'aria-label': label,
          onChange: (e) => app.run('set_image_playback', { id, [key]: Math.max(min, Math.min(max, Number(e.target.value) || 0)) }), onKeydown: (e) => e.stopPropagation() }));
      out.push(section('imganim', 'Animated picture', [
        h('div', { class: 'muted', style: 'font-size: 13px' }, `An animated picture: ${pb.frames} frames, ${pb.length.toFixed(2)} s a pass.`),
        num('Speed', 'speed', { step: 0.1, min: 0, max: 16, unit: '×', title: '1× as made; 2× twice as fast; 0 stops on the first frame' }),
        num('Starts at', 'offset', { step: 0.05, min: 0, max: Math.max(0, pb.length), unit: 's', title: 'How far into the animation it is when the layer comes in' }),
        check('Loop the animation', pb.looped, (on) => app.run('set_image_playback', { id, looped: on }), 'Play over and over (else once, then hold the last frame)'),
        small('Reset playback', () => app.run('set_image_playback', { id, reset: true })),
      ], { open: true }));
    }
  }

  // ----- Text animations (letters, words or lines one after another) -----
  if (L.type === 'text') {
    const anims = L.animators || [];
    const names = app.textAnimPresets || (app.textAnimPresets = call('text_anim_presets'));
    const setAnims = (f) => { const list = clone(anims); f(list); app.run('set_text_anims', { id, animators: list }); };
    const num = (i, label, key, { step = 0.05, min = 0, max = 60 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
      h('input', { class: 'field small', type: 'number', step: String(step), min: String(min), max: String(max), value: String(anims[i][key] ?? 0), 'aria-label': label,
        onChange: (e) => setAnims((l) => { l[i][key] = Math.max(min, Math.min(max, Number(e.target.value))); }), onKeydown: (e) => e.stopPropagation() }));
    const easings = app.easingList || (app.easingList = call('easings'));
    const easingName = (e) => easings.find((x) => JSON.stringify(x.easing) === JSON.stringify(e))?.name || 'custom';
    // A "from" value: offset.x / offset.y (an {x, y} or [x, y]), or a number; shown × k.
    const fromRow = (i, label, key, unit, title, k = 1) => {
      const f = anims[i].from || {};
      const [top, sub] = key.split('.');
      const raw = sub ? (Array.isArray(f[top]) ? f[top][sub === 'x' ? 0 : 1] : f[top]?.[sub]) : f[top];
      return h('label', { class: 'prop', title }, h('span', { class: 'pname' }, `${label} (${unit})`),
        h('input', { class: 'field small', type: 'number', step: '1', value: String(Math.round((raw ?? (key === 'scale' || key === 'opacity' ? 1 : 0)) * k * 100) / 100), 'aria-label': `From: ${label}`,
          onKeydown: (e) => e.stopPropagation(),
          onChange: (e) => setAnims((l) => {
            const v = Number(e.target.value) / k;
            if (!Number.isFinite(v)) return;
            const fr = (l[i].from ||= {});
            if (sub) {
              const o = Array.isArray(fr[top]) ? { x: fr[top][0], y: fr[top][1] } : { x: 0, y: 0, ...(fr[top] || {}) };
              o[sub] = v;
              fr[top] = Array.isArray(fr[top]) ? [o.x, o.y] : o;
            } else fr[top] = key === 'opacity' ? Math.max(0, Math.min(1, v)) : key === 'blur' || key === 'scale' ? Math.max(0, v) : v;
          }) }));
    };
    out.push(section('textanim', anims.length ? `Text animation (${anims.length})` : 'Text animation', [
      ...anims.map((a, i) => h('div', { class: 'item-card' },
        h('div', { class: 'card-head' }, check(a.name || `Animation ${i + 1}`, a.enabled !== false, (on) => setAnims((l) => { l[i].enabled = on; })), h('span', { class: 'spacer' }),
          small('Remove', () => setAnims((l) => { l.splice(i, 1); }), 'Remove', 'trash')),
        select('Each', a.unit || 'letter', [['letter', 'Letter'], ['word', 'Word'], ['line', 'Line']], (v) => setAnims((l) => { l[i].unit = v; })),
        select('Order', a.order || 'forward', [['forward', 'First to last'], ['reverse', 'Last to first'], ['center', 'From the middle'], ['random', 'Random']], (v) => setAnims((l) => { l[i].order = v; })),
        num(i, 'Starts at (s)', 'start'), num(i, 'Each takes (s)', 'duration', { min: 0.01 }), num(i, 'Gap (s)', 'stagger'),
        a.order === 'random' ? num(i, 'Variation', 'seed', { step: 1, max: 9999 }) : null,
        select('Easing', easingName(a.easing), easings.map((x) => [x.name, easingLabel(x.name)]).concat(easingName(a.easing) === 'custom' ? [['custom', 'Custom']] : []),
          (v) => setAnims((l) => { const e = easings.find((x) => x.name === v); if (e) l[i].easing = e.easing; })),
        check('Animate out (leave) instead of in', !!a.out, (on) => setAnims((l) => { l[i].out = on; })),
        h('div', { class: 'lab', title: 'How each one starts; it animates to rest' }, 'From'),
        fromRow(i, 'Move X', 'offset.x', 'px', 'Starts this far to the side'), fromRow(i, 'Move Y', 'offset.y', 'px', 'Starts this far down (negative: up)'),
        fromRow(i, 'Size', 'scale', '%', 'Starts at this size', 100), fromRow(i, 'Turn', 'rotation', '°', 'Starts turned by'),
        fromRow(i, 'Opacity', 'opacity', '%', 'Starts this visible', 100), fromRow(i, 'Blur', 'blur', 'px', 'Starts this blurred'),
        fromRow(i, 'Spacing', 'tracking', 'px', 'Starts spread apart by this much per letter'))),
      h('div', { class: 'add-row' }, names.map((n) => h('button', { class: 'small-btn', onClick: () => app.run('add_text_anim', { id, name: n }) }, `+ ${n}`))),
    ], { open: anims.length > 0 }));
  }

  // A rectangle's corners one by one (and its size while a path is still a rectangle or ellipse);
  // a path's point count, with simplifying.
  function smartControls(i) {
    let sm = null;
    try { sm = call('smart_shape', { id, index: i }); } catch { return null; }
    if (!sm) return null;
    const small2 = (v, label, apply) => h('input', { class: 'field small', type: 'number', min: '0', step: '1', value: String(Math.round(v * 10) / 10), 'aria-label': label, title: label,
      onChange: (e) => apply(Math.max(0, Number(e.target.value) || 0)), onKeydown: (e) => e.stopPropagation() });
    const out = [];
    if (sm.kind === 'rect' || sm.kind === 'ellipse') {
      const [x0, y0, x1, y1] = sm.rect;
      if (!sm.live) {
        out.push(h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Size'), h('div', { class: 'vals' },
          small2(x1 - x0, 'Width', (v) => app.run('set_smart_shape', { id, index: i, w: v, h: y1 - y0 })), small2(y1 - y0, 'Height', (v) => app.run('set_smart_shape', { id, index: i, w: x1 - x0, h: v })))));
      }
      if (sm.kind === 'rect') {
        const same = sm.radii.every((r) => Math.abs(r - sm.radii[0]) < 1e-6);
        const each = app.eachCorner?.[`${id}:${i}`] ?? !same;
        if (!sm.live) out.push(h('div', { class: 'prop' }, h('span', { class: 'pname' }, same ? 'Corners' : 'Corners (mixed)'), h('div', { class: 'vals' },
          small2(Math.max(...sm.radii), 'Every corner', (v) => app.run('set_smart_shape', { id, index: i, radii: [v, v, v, v] })))));
        out.push(check('Each corner on its own', each, (on) => { (app.eachCorner ||= {})[`${id}:${i}`] = on; app.renderSide(); }, 'Round corners separately (a live rectangle becomes a path; it keeps these controls while it’s a rectangle)'));
        if (each) {
          out.push(h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Corners'), h('div', { class: 'vals corner4' },
            ['top left', 'top right', 'bottom right', 'bottom left'].map((n, k) => small2(sm.radii[k], `The ${n} corner’s radius (px)`, (v) => { const r = sm.radii.slice(); r[k] = v; app.run('set_smart_shape', { id, index: i, radii: r }); })))));
        }
      }
    } else if (sm.kind === 'path') {
      const tol = (app.simplifyTol ||= 1);
      const count = h('span', { class: 'muted', style: 'font-size: 13px; width: 74px' }, `${sm.points} points`);
      const slider = h('input', { type: 'range', min: '-1', max: '1.3', step: '0.01', value: String(Math.log10(tol)), disabled: sm.animated, style: 'flex: 1; accent-color: var(--accent)',
        'aria-label': 'Simplify: how far the outline may stray', title: sm.animated ? 'Simplifying an animated path would break its morph: remove its keyframes first' : 'Fewer points, the same look: drag right to simplify more',
        onPointerdown: () => call('points_begin'),
        onInput: (e) => { app.simplifyTol = 10 ** Number(e.target.value); const n = app.run('set_smart_shape', { id, index: i, simplify: app.simplifyTol }, { quiet: true }); if (n != null) count.textContent = `${n} points`; },
        onChange: () => app.changed() });
      out.push(h('div', { class: 'prop' }, h('span', { class: 'pname' }, 'Simplify'), count, slider));
    }
    return out;
  }

  // ----- Each shape: fill, stroke, its settings -----
  (L.shapes || []).forEach((s, i) => {
    const pre = `shapes.${i}.`;
    const kids = [];
    const g = s.geometry || {};
    // The shape's own settings.
    kids.push(row(`${pre}size`, { unit: ['W', 'H'], step: 1, min: 0 }), row(`${pre}corner_radius`, { unit: 'px', step: 1, min: 0 }));
    kids.push(smartControls(i));
    if (g.type === 'figure') {
      kids.push(row(`${pre}count`, { unit: g.kind === 'gear' ? 'teeth' : g.kind === 'star' ? 'points' : 'sides', step: 1, min: 3 }),
        g.kind !== 'polygon' ? row(`${pre}inner`, { unit: '%', k: 100, step: 1, min: 0 }) : null,
        g.kind === 'gear' ? row(`${pre}hole`, { unit: '%', k: 100, step: 1, min: 0 }) : null);
    }
    // Fill.
    const fillPaint = s.fill?.pattern ? 'pattern' : s.fill?.gradient ? s.fill.gradient.kind : 'solid';
    kids.push(h('div', { class: 'sub-head' }, check('Fill', !!s.fill, (on) => app.run('set_paint', { ids: [id], fill: on ? app.fill : false })),
      s.fill ? h('span', { class: 'segs mini' }, [['solid', 'Solid'], ['linear', 'Linear'], ['radial', 'Radial'], ['pattern', 'Picture']].map(([k, t]) => h('button', {
        class: `seg${fillPaint === k ? ' on' : ''}`, onClick: () => {
          if (k !== 'pattern') return app.run('set_gradient', { id, index: i, kind: k });
          const pics = call('pictures');
          if (!pics.length) return app.pickPattern(id, i);
          editShape(i, 'Pattern', (x) => { x.fill.pattern = { asset: pics[pics.length - 1].id, scale: 1, rotation: 0, offset: { x: 0, y: 0 } }; x.fill.gradient = null; });
        } }, t))) : null));
    if (s.fill) {
      if (fillPaint === 'solid') kids.push(crow(`${pre}fill_color`));
      if (s.fill.gradient) kids.push(stopsEditor(app, h, ui, info, `${pre}gradient_colors`, P[`${pre}gradient_colors`]), row(`${pre}gradient_start`, { unit: ['X', 'Y'] }), row(`${pre}gradient_end`, { unit: ['X', 'Y'] }));
      if (s.fill.pattern) {
        // Which picture (one this canvas has, or a file), and how its tiles sit.
        const pics = call('pictures');
        const pt = s.fill.pattern;
        const tile = (label, unit, value, set, step = 1) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, `${label} (${unit})`),
          h('input', { class: 'field small', type: 'number', step: String(step), value: String(Math.round(value * 100) / 100), 'aria-label': label,
            onChange: (e) => editShape(i, `Pattern ${label.toLowerCase()}`, (x) => { set(x.fill.pattern, Number(e.target.value) || 0); }), onKeydown: (e) => e.stopPropagation() }));
        kids.push(select('Picture', String(pt.asset), [...pics.map((pc) => [String(pc.id), `${pc.name} (${pc.width} × ${pc.height})`]), ['file', 'From a file…']],
          (v) => (v === 'file' ? app.pickPattern(id, i) : editShape(i, 'Pattern picture', (x) => { x.fill.pattern.asset = Number(v); }))),
          tile('Size', '%', (pt.scale ?? 1) * 100, (q, v) => { q.scale = Math.max(0.01, v / 100); }, 5),
          tile('Turn', '°', pt.rotation ?? 0, (q, v) => { q.rotation = v; }),
          tile('Shift across', 'px', pt.offset?.x ?? 0, (q, v) => { q.offset = { x: v, y: q.offset?.y ?? 0 }; }),
          tile('Shift down', 'px', pt.offset?.y ?? 0, (q, v) => { q.offset = { x: q.offset?.x ?? 0, y: v }; }),
        h('button', { class: 'small-btn', onClick: () => editShape(i, 'No picture', (x) => { delete x.fill.pattern; }) }, 'Remove picture'));
      }
    }
    // Stroke.
    const st = s.stroke;
    const strokePaint = st?.gradient ? st.gradient.kind : 'solid';
    kids.push(h('div', { class: 'sub-head' }, check('Stroke', !!st, (on) => app.run('set_paint', { ids: [id], stroke: on ? app.stroke : false, stroke_width: Math.max(1, app.strokeWidth) })),
      st ? h('span', { class: 'segs mini' }, [['solid', 'Solid'], ['linear', 'Linear'], ['radial', 'Radial']].map(([k, t]) => h('button', {
        class: `seg${strokePaint === k ? ' on' : ''}`, onClick: () => app.run('set_gradient', { id, index: i, kind: k, stroke: true }) }, t))) : null));
    if (st) {
      if (!st.gradient) kids.push(crow(`${pre}stroke_color`));
      else kids.push(stopsEditor(app, h, ui, info, `${pre}stroke_gradient_colors`, P[`${pre}stroke_gradient_colors`]), row(`${pre}stroke_gradient_start`, { unit: ['X', 'Y'] }), row(`${pre}stroke_gradient_end`, { unit: ['X', 'Y'] }));
      kids.push(row(`${pre}stroke_width`, { unit: 'px', step: 0.5, min: 0 }));
      // Ends: flat, round, square, or a square with rounded corners (how round).
      const capNow = st.cap === 'square' && (st.cap_roundness ?? 0) > 0 ? 'rounded' : st.cap || 'butt';
      kids.push(select('Ends', capNow, [['butt', 'Flat'], ['round', 'Round'], ['square', 'Square'], ['rounded', 'Rounded square']], (v) => editShape(i, 'Line ends', (x) => {
        if (v === 'rounded') { x.stroke.cap = 'square'; x.stroke.cap_roundness = x.stroke.cap_roundness || 0.5; } else { x.stroke.cap = v; x.stroke.cap_roundness = 0; }
      })));
      if (capNow === 'rounded') kids.push(h('label', { class: 'prop' }, h('span', { class: 'pname' }, 'Roundness (%)'), h('input', { class: 'field small', type: 'number', min: '0', max: '100', step: '5', value: String(Math.round((st.cap_roundness ?? 0) * 100)), 'aria-label': 'End roundness (%)',
        onChange: (e) => editShape(i, 'End roundness', (x) => { x.stroke.cap_roundness = Math.max(0, Math.min(100, Number(e.target.value) || 0)) / 100; }), onKeydown: (e) => e.stopPropagation() })));
      kids.push(select('Corners', st.join || 'miter', [['miter', 'Sharp'], ['round', 'Round'], ['bevel', 'Cut off']], (v) => editShape(i, 'Line corners', (x) => { x.stroke.join = v; })));
      kids.push(h('label', { class: 'prop', title: 'Dash and gap lengths in pixels, e.g. 12 6 (empty: a solid line)' }, h('span', { class: 'pname' }, 'Dashes'),
        h('input', { class: 'field small', value: (st.dashes || []).join(' '), placeholder: 'e.g. 12 6', 'aria-label': 'Dashes',
          onChange: (e) => editShape(i, 'Dashes', (x) => { x.stroke.dashes = e.target.value.split(/[\s,]+/).map(Number).filter((n) => n > 0); }), onKeydown: (e) => e.stopPropagation() })));
      { const w = Math.max(1, P[`${pre}stroke_width`]?.value ?? st.width ?? 2);
        kids.push(h('div', { class: 'add-row' },
          h('button', { class: 'small-btn', title: 'A solid line', onClick: () => editShape(i, 'Dashes', (x) => { x.stroke.dashes = []; }) }, 'Solid'),
          h('button', { class: 'small-btn', title: 'Dashes three times as long as the line is wide', onClick: () => editShape(i, 'Dashes', (x) => { x.stroke.dashes = [w * 3, w * 2]; }) }, 'Dashed'),
          h('button', { class: 'small-btn', title: 'Round dots (round ends, no dash length)', onClick: () => editShape(i, 'Dots', (x) => { x.stroke.dashes = [0.01, w * 2]; x.stroke.cap = 'round'; x.stroke.cap_roundness = 0; }) }, 'Dotted'))); }
      if ((st.dashes || []).length) kids.push(row(`${pre}dash_offset`, { unit: 'px', step: 1 }));
      const pens = app.pens || (app.pens = call('pens'));
      const kind = st.style?.kind || 'solid';
      kids.push(select('Pen', kind, pens.map((p) => [p.kind, p.label]), (v) => {
        const preset = call('pen_preset', { kind: v });
        editShape(i, 'Pen', (x) => { if (v === 'solid') delete x.stroke.style; else x.stroke.style = preset; });
      }));
      if (kind !== 'solid') kids.push(penSettings(h, st.style, (f) => editShape(i, 'Pen settings', (x) => f(x.stroke.style), 'pen')));
      kids.push(h('div', { class: 'add-row' },
        kind !== 'solid' ? h('button', { class: 'small-btn', title: 'The same pen with a different random wobble', onClick: () => editShape(i, 'Shuffle pen', (x) => { x.stroke.style.seed = Math.floor(Math.random() * 9999); }) }, 'Shuffle') : null,
        kind !== 'solid' ? h('button', { class: 'small-btn', title: 'This pen as it comes', onClick: () => { const p = call('pen_preset', { kind }); editShape(i, 'Reset pen', (x) => { x.stroke.style = { ...p, seed: x.stroke.style.seed }; }); } }, 'Reset') : null,
        h('button', { class: 'small-btn', title: 'Every pen drawn as a sample stroke; your own pens', onClick: () => app.pensDialog?.() }, 'Pens…')));
    }
    // Draw on (trim).
    kids.push(h('div', { class: 'sub-head' }, check('Draw on', !!s.trim, (on) => editShape(i, on ? 'Draw on' : 'No draw on', (x) => { if (on) x.trim = { start: { static: 0 }, end: { static: 1 }, offset: { static: 0 } }; else delete x.trim; }),
      'Only part of the outline is drawn: animate Draw to from 0% to 100% to draw it on')));
    if (s.trim) kids.push(row(`${pre}trim_start`, { unit: '%', k: 100, step: 1, min: 0, max: 100 }), row(`${pre}trim_end`, { unit: '%', k: 100, step: 1, min: 0, max: 100 }), row(`${pre}trim_offset`, { unit: '%', k: 100, step: 1 }));
    const title = (L.shapes.length > 1 ? s.name || `Shape ${i + 1}` : 'Fill and stroke');
    out.push(section(`shape${i}`, title, kids, { extra: i === L.shapes.length - 1 ? small('+', (e) => { e.preventDefault(); app.run('add_shape_item', { id }); }, 'Add another shape to this layer', 'plus') : null }));
  });

  // ----- Effects -----
  const fx = L.effects || [];
  const fxKids = fx.map((e, i) => h('div', { class: 'item-card' },
    h('div', { class: 'card-head' },
      check(EFFECT_NAME[e.effect] || words(e.effect), e.enabled !== false, (on) => editMeta(on ? 'Effect on' : 'Effect off', (m) => { m.effects[i].enabled = on; })),
      h('span', { class: 'spacer' }),
      i > 0 ? small('Up', () => editMeta('Move effect', (m) => { [m.effects[i - 1], m.effects[i]] = [m.effects[i], m.effects[i - 1]]; }), 'Move up', 'up') : null,
      small('Remove', () => editMeta('Remove effect', (m) => { m.effects.splice(i, 1); }), 'Remove', 'trash')),
    Object.keys(P).filter((p) => p.startsWith(`effects.${i}.`)).map((p) => (/color$/.test(p) ? crow(p, true) : row(p, ui.unitsFor(p), true)))));
  out.push(section('effects', fx.length ? `Effects (${fx.length})` : 'Effects', [
    ...fxKids,
    h('div', { class: 'add-row' }, EFFECTS.map(([k, t]) => h('button', { class: 'small-btn', onClick: () => app.run('add_effect', { ids: [id], kind: k }) }, `+ ${t}`))),
  ], { open: fx.length > 0 }));

  // ----- Modifiers -----
  const mods = L.modifiers || [];
  const presets = app.modPresets || (app.modPresets = call('modifier_presets'));
  const modKids = mods.map((m, i) => h('div', { class: 'item-card' },
    h('div', { class: 'card-head' },
      check(MOD_NAME[m.modifier] || words(m.modifier), m.enabled !== false, (on) => editMeta(on ? 'Modifier on' : 'Modifier off', (x) => { x.modifiers[i].enabled = on; })),
      h('span', { class: 'spacer' }),
      i === 0 ? small('Apply', () => app.run('apply_modifier', { id }), 'Make it permanent: copies become layers, outline changes become points') : null,
      i > 0 ? small('Up', () => editMeta('Move modifier', (x) => { [x.modifiers[i - 1], x.modifiers[i]] = [x.modifiers[i], x.modifiers[i - 1]]; }), 'Move up', 'up') : null,
      small('Remove', () => editMeta('Remove modifier', (x) => { x.modifiers.splice(i, 1); }), 'Remove', 'trash')),
    Object.keys(P).filter((p) => p.startsWith(`modifiers.${i}.`)).map((p) => (/color$/.test(p) ? crow(p, true) : row(p, ui.unitsFor(p), true))),
    modOptions(m, i)));
  // The settings a modifier has that aren't numbers: switches, how corners join, the other layer it uses.
  function modOptions(m, i) {
    const set = (label, f) => editMeta(label, (x) => { f(x.modifiers[i]); });
    const flag = { jitter: ['smooth', 'Keep curves', 'Else straight lines join the moved points'], zig_zag: ['smooth', 'Smooth (wave)', 'Rounded waves instead of sharp ridges'],
      mirror: ['merge', 'Merge halves', 'Join the shape and its mirror image into one outline'], path_array: ['align', 'Turn with the path', 'Each copy turns to follow the line'] }[m.modifier];
    const out = [];
    if (flag) out.push(check(flag[1], !!m[flag[0]], (on) => set(flag[1], (x) => { x[flag[0]] = on; }), flag[2]));
    if (m.modifier === 'offset_path') out.push(select('Corners', m.join || 'round', [['miter', 'Sharp'], ['round', 'Round'], ['bevel', 'Cut']], (v) => set('Offset corners', (x) => { x.join = v; })));
    const key = m.modifier === 'blend' ? 'target' : m.modifier === 'path_array' ? 'path' : null;
    if (key) out.push(select(m.modifier === 'blend' ? 'Into' : 'Path', String(m[key]), full.others.map((o) => [String(o.id), o.name]), (v) => set('Modifier layer', (x) => { x[key] = Number(v); })));
    if (!['array', 'path_array'].includes(m.modifier) && (L.type === 'text' || L.type === 'image')) out.push(h('div', { class: 'muted', style: 'font-size: 12.5px' }, 'Text and pictures keep their outline: this changes shapes only.'));
    return out;
  }
  const addMod = h('select', { class: 'field small', 'aria-label': 'Add a modifier', onChange: (e) => { const v = e.target.value; e.target.value = ''; if (v) app.run('add_modifier', { preset: v }); } },
    h('option', { value: '' }, '+ Add a modifier…'), presets.map((p) => h('option', { value: p.key, title: p.tip }, p.pair ? `${p.label} (select two layers)` : p.label)));
  out.push(section('modifiers', mods.length ? `Modifiers (${mods.length})` : 'Modifiers', [...modKids, addMod], { open: mods.length > 0 }));

  // ----- Everything else the layer has (particles, water, camera, 3D…) -----
  const shown = /^(position|scale|rotation|opacity|anchor|skew|skew_axis|font_size|text_color|fill|stroke|stroke_width|size|corner_radius|distort|volume|shapes\.|effects\.|modifiers\.|masks\.)/;
  const solidShown = /^(rotation_[xyz]|solid_size|solid_depth|solid_color)$/;
  const rest = Object.keys(P).filter((p) => !shown.test(p) && !(L.type === 'solid' && (solidShown.test(p) || /^parts\./.test(p))));
  if (rest.length && !['particles', 'liquid', 'camera'].includes(L.type)) {
    out.push(section('settings', words(L.type || 'Settings'), rest.map((p) => (/color$/.test(p) ? crow(p) : row(p, ui.unitsFor(p))))));
  }

  // ----- A particle layer's emitter: how and where they come out, what each is, its colours, how they end -----
  if (L.type === 'particles') {
    const setL = (label, f) => { const l = clone(L); f(l); app.run('set_layer_json', { layer: l, label }); };
    const others = full.others.map((o) => [String(o.id), o.name]);
    const num = (label, value, onSet, { min = 0, max = 1e6, step = 1, tip = '' } = {}) => h('label', { class: 'prop', title: tip }, h('span', { class: 'pname' }, label),
      h('input', { class: 'field small', type: 'number', min: String(min), max: String(max), step: String(step), value: String(Math.round(value * 1000) / 1000), 'aria-label': label,
        onChange: (e) => onSet(Math.max(min, Math.min(max, Number(e.target.value) || 0))), onKeydown: (e) => e.stopPropagation() }));
    const colourIn = (c, label, onSet) => h('input', { type: 'color', class: 'color-in', value: toHex(c).slice(0, 7), 'data-rgba': toHex(c), 'aria-label': label, onChange: (e) => onSet(fromHex(e.target.dataset.rgba || e.target.value)) });
    const sh = L.shape || { type: 'point' };
    const pa = L.particle || { type: 'confetti' };
    const end = L.end || { type: 'fade' };
    const firstOther = full.others[0]?.id;
    const kids = [
      select('Emit', L.mode || 'burst', [['burst', 'Burst: all at once'], ['stream', 'Stream: a steady flow']], (v) => setL('Emit', (l) => { l.mode = v; })),
      check('Already going at the start', !!L.prewarm, (on) => setL('Prewarm', (l) => { l.prewarm = on; }), 'Prewarm: the flow is in full swing on the first frame'),
      check('Stops', L.stop != null, (on) => setL('Stops', (l) => { l.stop = on ? Math.max((l.start || 0) + 1, app.state.comp.duration * 0.75) : null; }), 'Stop making new ones at a time (those out finish their life)'),
      L.stop != null ? num('Stops at (s)', L.stop, (v) => setL('Stop time', (l) => { l.stop = v; }), { step: 0.1, max: 3600 }) : null,
      L.mode === 'burst' ? [num('Bursts again', L.repeat || 0, (v) => setL('Repeat', (l) => { l.repeat = Math.round(v); }), { max: 1000, tip: 'How many more bursts after the first' }),
        (L.repeat || 0) > 0 ? num('Every (s)', L.interval || 1, (v) => setL('Interval', (l) => { l.interval = v; }), { min: 0.05, step: 0.1, max: 600 }) : null] : null,
      select('From', sh.type, [['point', 'A point'], ['line', 'A line'], ['rect', 'A rectangle'], ['ellipse', 'An ellipse'], ...(firstOther != null ? [['layer', 'A layer’s outline']] : [])], (v) => setL('Emitter shape', (l) => {
        l.shape = v === 'line' ? { type: 'line', length: 200 } : v === 'rect' || v === 'ellipse' ? { type: v, size: { x: 200, y: 120 } } : v === 'layer' ? { type: 'layer', layer: firstOther } : { type: 'point' };
      })),
      rest.map((p) => (/color$/.test(p) ? crow(p) : row(p, ui.unitsFor(p)))),
      sh.type === 'line' ? num('Length', sh.length, (v) => setL('Emitter length', (l) => { l.shape.length = v; })) : null,
      sh.size ? [num('Width', sh.size.x, (v) => setL('Emitter size', (l) => { l.shape.size.x = v; })), num('Height', sh.size.y, (v) => setL('Emitter size', (l) => { l.shape.size.y = v; }))] : null,
      sh.type === 'layer' ? select('Layer', String(sh.layer), others, (v) => setL('Emitter layer', (l) => { l.shape.layer = Number(v); })) : null,
      num('Swirl speed', L.turbulence_speed ?? 1, (v) => setL('Swirl speed', (l) => { l.turbulence_speed = v; }), { step: 0.1, max: 50, tip: 'How fast the turbulence changes' }),
      select('Each is a', pa.type, [['confetti', 'Confetti'], ['circle', 'Circle'], ['square', 'Square'], ['star', 'Star'], ['streak', 'Streak'], ['soft', 'Soft dot'], ['ring', 'Ring'], ...(firstOther != null ? [['layer', 'A layer (sprite)']] : [])],
        (v) => setL('Particle', (l) => { l.particle = v === 'layer' ? { type: 'layer', layer: firstOther } : { type: v }; })),
      pa.type === 'layer' ? select('Sprite', String(pa.layer), others, (v) => setL('Sprite', (l) => { l.particle.layer = Number(v); })) : null,
      num('Life ± %', (L.lifetime_variation || 0) * 100, (v) => setL('Life variation', (l) => { l.lifetime_variation = v / 100; }), { max: 100 }),
      num('Speed ± %', (L.speed_variation || 0) * 100, (v) => setL('Speed variation', (l) => { l.speed_variation = v / 100; }), { max: 100 }),
      num('Size ± %', (L.size_variation || 0) * 100, (v) => setL('Size variation', (l) => { l.size_variation = v / 100; }), { max: 100 }),
      num('Size at the end %', (L.size_end ?? 1) * 100, (v) => setL('End size', (l) => { l.size_end = v / 100; }), { max: 2000, step: 10, tip: 'Relative to the start: 300% grows like smoke' }),
      h('div', { class: 'lab' }, 'Colours (each particle picks one)'),
      h('div', { class: 'add-row' }, (L.colors || []).map((c, k) => h('span', { class: 'swatch-pair' },
        colourIn(c, `Colour ${k + 1}`, (v) => setL('Particle colour', (l) => { l.colors[k] = v; })),
        L.colors.length > 1 ? h('button', { class: 'mini keep', title: 'Remove this colour', 'aria-label': `Remove colour ${k + 1}`, html: '−', onClick: () => setL('Remove colour', (l) => { l.colors.splice(k, 1); }) }) : null)),
        h('button', { class: 'small-btn', title: 'Add a colour', onClick: () => setL('Add colour', (l) => { l.colors = [...(l.colors || []), (l.colors || []).at(-1) || { r: 1, g: 1, b: 1, a: 1 }]; }) }, '+')),
      check('Change over their life', !!L.color_over_life, (on) => setL('Colour over life', (l) => {
        l.color_over_life = on ? [{ offset: 0, color: (l.colors || [])[0] || { r: 1, g: 1, b: 1, a: 1 } }, { offset: 1, color: { r: 0.2, g: 0.2, b: 0.2, a: 0 } }] : null;
      }), 'Every particle goes through these colours from birth to the end'),
      L.color_over_life ? h('div', { class: 'add-row' }, L.color_over_life.map((st, k) => h('span', { class: 'swatch-pair', title: `${Math.round(st.offset * 100)}% through its life` },
        colourIn(st.color, `Life colour ${k + 1}`, (v) => setL('Life colour', (l) => { l.color_over_life[k].color = v; })),
        L.color_over_life.length > 2 ? h('button', { class: 'mini keep', 'aria-label': `Remove life colour ${k + 1}`, title: 'Remove', html: '−', onClick: () => setL('Remove life colour', (l) => { l.color_over_life.splice(k, 1); l.color_over_life.forEach((x, j, a) => { x.offset = j / (a.length - 1); }); }) }) : null)),
        h('button', { class: 'small-btn', title: 'Another colour at the end', onClick: () => setL('Add life colour', (l) => { const a = l.color_over_life; a.push({ offset: 1, color: a.at(-1).color }); a.forEach((x, j) => { x.offset = j / (a.length - 1); }); }) }, '+')) : null,
      select('Finally', end.type, [['fade', 'Fade out'], ['shrink', 'Shrink away'], ['settle', 'Settle on the floor'], ...(firstOther != null ? [['form', 'Form a shape']] : [])], (v) => setL('Particle end', (l) => {
        l.end = v === 'settle' ? { type: 'settle', floor: app.state.comp.height - 20 } : v === 'form' ? { type: 'form', target: firstOther } : { type: v };
      })),
      end.type === 'settle' ? num('Floor at (px)', end.floor, (v) => setL('Floor', (l) => { l.end.floor = v; }), { min: -1e5, max: 1e5, tip: 'Canvas y where they come to rest' }) : null,
      end.type === 'form' ? [select('Shape of', String(end.target), others, (v) => setL('Form target', (l) => { l.end.target = Number(v); })), h('div', { class: 'muted', style: 'font-size: 12.5px' }, 'They fly into its outline and stay there.')] : null,
      num('Variation', L.seed || 0, (v) => setL('Variation', (l) => { l.seed = Math.round(v); }), { max: 1e9, tip: 'A different random pattern' }),
    ];
    out.push(section('particles', 'Particles', kids));
  }

  // ----- Water: its container, waves, bubbles, foam and the deeper colour -----
  if (L.type === 'liquid' || L.type === 'camera') {
    const setL = (label, f) => { const l = clone(L); f(l); app.run('set_layer_json', { layer: l, label }); };
    const num = (label, value, onSet, { min = 0, max = 1e5, step = 1, tip = '' } = {}) => h('label', { class: 'prop', title: tip }, h('span', { class: 'pname' }, label),
      h('input', { class: 'field small', type: 'number', min: String(min), max: String(max), step: String(step), value: String(Math.round(value * 1000) / 1000), 'aria-label': label,
        onChange: (e) => onSet(Math.max(min, Math.min(max, Number(e.target.value) || 0))), onKeydown: (e) => e.stopPropagation() }));
    const rows = rest.map((p) => (/color$/.test(p) ? crow(p) : row(p, ui.unitsFor(p))));
    if (L.type === 'liquid') {
      const field = (label, key, opts) => num(label, L[key] ?? 0, (v) => setL(label, (l) => { l[key] = opts?.int ? Math.round(v) : v; }), opts);
      const deep = L.deep_color;
      out.push(section('water', 'Water', [
        h('div', { class: 'muted', style: 'font-size: 12.5px' }, 'Drag the level handle on the canvas, or keyframe Level to fill and drain.'),
        rows,
        num('Width', L.size.x, (v) => setL('Water width', (l) => { l.size.x = Math.max(1, v); }), { min: 1, tip: 'The container’s width' }),
        num('Height', L.size.y, (v) => setL('Water height', (l) => { l.size.y = Math.max(1, v); }), { min: 1, tip: 'The container’s height' }),
        field('Rises towards °', 'direction', { min: -360, max: 360, tip: '-90° = up' }),
        field('Slosh °', 'slosh', { max: 45, step: 0.5, tip: 'Rocking back and forth' }),
        field('Slosh speed', 'slosh_speed', { max: 10, step: 0.05, tip: 'Rocks per second' }),
        field('Wave trains', 'waves', { max: 6, int: true, tip: 'Layers of waves (0: flat)' }),
        field('Wave length', 'wave_length', { min: 1, tip: 'Crest to crest (px)' }),
        field('Wave speed', 'wave_speed', { min: -10, max: 10, step: 0.05, tip: 'Wavelengths per second' }),
        field('Bubbles', 'bubbles', { max: 500, int: true, tip: 'How many rise at once' }),
        field('Foam px', 'foam', { max: 100, step: 0.5, tip: 'A bright line on the surface (0: none)' }),
        h('div', { class: 'prop', title: 'A gradient down to this colour' },
          check('Deeper colour', !!deep, (on) => setL('Deeper colour', (l) => { l.deep_color = on ? { r: 0.02, g: 0.2, b: 0.45, a: 0.95 } : null; })),
          deep ? h('input', { type: 'color', class: 'color-in', value: toHex(deep).slice(0, 7), 'data-rgba': toHex(deep), 'aria-label': 'Deeper colour', onChange: (e) => setL('Deeper colour', (l) => { l.deep_color = fromHex(e.target.dataset.rgba || e.target.value); }) }) : null),
      ]));
    } else {
      const active = call('camera_info', { id }).active;
      const look = call('look_through');
      const sk = L.shake;
      const shakeNum = (label, key, opts) => num(label, sk[key] ?? 0, (v) => app.run('camera_shake', { id, shake: { ...sk, [key]: opts?.int ? Math.round(v) : v } }), opts);
      out.push(section('camera', 'Camera', [
        h('div', { class: 'add-row' }, active ? h('span', { style: 'color: var(--accent-ink); font-weight: 600', title: 'The topmost visible camera is the one that’s used' }, '● Active camera')
          : [h('span', { class: 'muted' }, 'Not the active camera'), h('button', { class: 'small-btn', title: 'Move it above the other cameras and show it', onClick: () => app.run('camera_make_active', { id }) }, 'Make active')]),
        check('Look through camera', !!look.on, (on) => { call('look_through', { on }); app.changed(); }, 'View › Look through camera'),
        rows.filter((r, k) => !/^shake_/.test(rest[k])),
        h('div', { class: 'muted', style: 'font-size: 12.5px' }, 'Follow a layer: set Parent to it. Keyframe Position and Rotation to move and turn.'),
        check('Shake', !!sk, (on) => app.run('camera_shake', { id, shake: on }), 'Handheld wobble, or a jolt that dies away'),
        sk ? [rows.filter((r, k) => /^shake_/.test(rest[k])),
          shakeNum('Shakes a second', 'frequency', { min: 0.1, max: 60, step: 0.5 }), shakeNum('Starts at (s)', 'start', { max: 3600, step: 0.1, tip: 'When the shake begins' }),
          shakeNum('Dies away', 'decay', { max: 50, step: 0.5, tip: '0 keeps shaking, 3 is a short jolt' }), shakeNum('Variation', 'seed', { max: 9999, int: true, tip: 'A different random pattern' })] : null,
      ]));
    }
  }

  // ----- Masks and the mask layer -----
  const masks = L.masks || [];
  const maskKids = masks.map((m, i) => h('div', { class: 'item-card' },
    h('div', { class: 'card-head' },
      check(m.name || `Mask ${i + 1}`, m.enabled !== false, (on) => editMeta(on ? 'Mask on' : 'Mask off', (x) => { x.masks[i].enabled = on; })),
      h('span', { class: 'spacer' }),
      small('Remove', () => editMeta('Remove mask', (x) => { x.masks.splice(i, 1); }), 'Remove', 'trash')),
    select('Mode', m.mode || 'add', MASK_MODES, (v) => editMeta('Mask mode', (x) => { x.masks[i].mode = v; })),
    check('Show outside instead', !!m.inverted, (on) => editMeta('Invert mask', (x) => { x.masks[i].inverted = on; })),
    row(`masks.${i}.feather`, { unit: 'px', step: 1, min: 0 }, true), row(`masks.${i}.opacity`, { unit: '%', k: 100, step: 1, min: 0, max: 100 }, true)));
  const matte = L.matte;
  out.push(section('masks', masks.length ? `Masks (${masks.length})` : 'Masks', [
    ...maskKids,
    h('div', { class: 'add-row' },
      h('button', { class: 'small-btn', onClick: () => app.run('add_mask', { ellipse: false }) }, '+ Rectangle'),
      h('button', { class: 'small-btn', onClick: () => app.run('add_mask', { ellipse: true }) }, '+ Ellipse'),
      h('button', { class: 'small-btn', title: 'Select two layers: the shape on top becomes a mask on the other', disabled: app.state.selection.length !== 2, onClick: () => app.run('shape_to_mask') }, '+ From the shape above')),
    select('Mask layer', matte ? String(matte.layer) : '', [['', 'None'], ...full.others.map((o) => [String(o.id), o.name])], (v) => app.run('set_matte', { id, layer: v ? Number(v) : null, mode: matte?.mode || 'alpha' })),
    matte ? select('Show', matte.mode || 'alpha', MATTE_MODES, (v) => app.run('set_matte', { id, layer: matte.layer, mode: v })) : null,
  ], { open: masks.length > 0 || !!matte }));
  return out;
}

/// A colour row that keeps the colour's transparency (the picker has none) and shows it as a percentage.
function colourRow(app, h, ui, info, prop, p, label) {
  const hex = String(p.value);
  const alpha = hex.length >= 9 ? parseInt(hex.slice(7, 9), 16) / 255 : 1;
  const withAlpha = (rgb, a) => (a >= 0.999 ? rgb : rgb + Math.round(a * 255).toString(16).padStart(2, '0'));
  // The picker gives the colour with its see-through (data-rgba); the number beside it sets that alone.
  const input = h('input', { type: 'color', class: 'color-in', value: hex.slice(0, 7), 'data-rgba': hex, 'aria-label': label });
  const picked = (e) => e.target.dataset.rgba || withAlpha(e.target.value, alpha);
  input.addEventListener('input', (e) => app.run('set_prop', { id: info.id, prop, value: picked(e) }, { quiet: true }));
  input.addEventListener('change', (e) => app.run('set_prop', { id: info.id, prop, value: picked(e) }));
  const a = h('input', { class: 'field tiny', type: 'number', min: '0', max: '100', step: '5', value: String(Math.round(alpha * 100)), 'aria-label': `${label} opacity (%)`, title: 'Opacity (%)',
    onChange: (e) => app.run('set_prop', { id: info.id, prop, value: withAlpha(hex.slice(0, 7), Math.max(0, Math.min(100, Number(e.target.value))) / 100) }), onKeydown: (e) => e.stopPropagation() });
  return h('div', { class: 'prop' }, app.state.comp.duration ? ui.keyButton(info, prop, p, label) : null, h('span', { class: 'pname' }, label), input, a, h('small', { class: 'unit' }, '%'));
}

/// The gradient's colour stops: a preview, then each stop's colour and place.
/// A gradient's colours: a bar with a marker per colour (click the bar to add one, drag a marker
/// to move it, down off the bar to remove it), the chosen colour, and the gradient's tools.
function stopsEditor(app, h, ui, info, prop, p) {
  if (!p) return null;
  const stops = p.value;
  const [, idx, stroke] = prop.match(/^shapes\.(\d+)\.(stroke_)?gradient_colors$/) || [];
  const index = Number(idx || 0);
  const isStroke = !!stroke;
  const pre = `shapes.${index}.${isStroke ? 'stroke_' : ''}gradient_`;
  const P = info.props;
  const shape = app.full?.layer?.shapes?.[index];
  const kind = (isStroke ? shape?.stroke?.gradient?.kind : shape?.fill?.gradient?.kind) || 'linear';
  const key = `${info.id}:${prop}`;
  app.gradSel ||= {};
  const selI = Math.min(app.gradSel[key] ?? 0, stops.length - 1);
  const sorted = stops.map((st, i) => ({ ...st, i })).sort((a, b) => a.offset - b.offset);
  const css = sorted.map((st) => `${toHex(st.color)} ${Math.round(st.offset * 100)}%`).join(', ');
  const send = (next, quiet) => app.run('set_prop', { id: info.id, prop, value: next }, quiet ? { quiet: true } : {});
  const colourAt = (t) => {
    const lo = [...sorted].reverse().find((st) => st.offset <= t) || sorted[0], hi = sorted.find((st) => st.offset >= t) || sorted[sorted.length - 1];
    const k = hi.offset > lo.offset ? (t - lo.offset) / (hi.offset - lo.offset) : 0;
    const mix = (a, b) => a + (b - a) * k;
    return { r: mix(lo.color.r, hi.color.r), g: mix(lo.color.g, hi.color.g), b: mix(lo.color.b, hi.color.b), a: mix(lo.color.a ?? 1, hi.color.a ?? 1) };
  };
  const bar = h('div', { class: 'grad-bar', style: `background: linear-gradient(to right, ${css}), repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 10px 10px`, title: 'Click to add a colour here' });
  const lane = h('div', { class: 'grad-lane' });
  stops.forEach((st, i) => {
    const m = h('button', { class: `grad-stop${i === selI ? ' on' : ''}`, style: `left: ${st.offset * 100}%; --c: ${toHex(st.color).slice(0, 7)}`, 'aria-label': `Colour ${i + 1} at ${Math.round(st.offset * 100)}% (arrows move it, Delete removes it)`, title: 'Drag to move; drag down off the bar to remove',
      onKeydown: (e) => {
        e.stopPropagation();
        if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); const n = clone(stops); n[i].offset = Math.max(0, Math.min(1, n[i].offset + (e.key === 'ArrowRight' ? 1 : -1) * (e.shiftKey ? 0.1 : 0.01))); send(n); }
        if ((e.key === 'Delete' || e.key === 'Backspace') && stops.length > 2) { send(stops.filter((_, j) => j !== i)); app.gradSel[key] = 0; }
      } });
    m.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      app.gradSel[key] = i;
      m.setPointerCapture(e.pointerId);
      const r = bar.getBoundingClientRect();
      let gone = false;
      const move = (ev) => {
        gone = stops.length > 2 && ev.clientY - r.bottom > 28;
        m.style.opacity = gone ? '0.3' : '';
        const n = clone(stops);
        n[i].offset = Math.max(0, Math.min(1, (ev.clientX - r.left) / r.width));
        m.style.left = `${n[i].offset * 100}%`;
        if (!gone) send(n, true);
      };
      const up = () => {
        m.removeEventListener('pointermove', move);
        if (gone) { app.gradSel[key] = 0; send(stops.filter((_, j) => j !== i)); } else app.changed();
      };
      m.addEventListener('pointermove', move);
      m.addEventListener('pointerup', up, { once: true });
      lane.querySelectorAll('.grad-stop').forEach((x) => x.classList.toggle('on', x === m));
    });
    lane.append(m);
  });
  bar.addEventListener('click', (e) => {
    const r = bar.getBoundingClientRect();
    const t = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    app.gradSel[key] = stops.length;
    send([...stops, { offset: t, color: colourAt(t) }]);
  });
  const cur = stops[selI];
  const op = (name, value) => app.run('gradient_op', { id: info.id, index, stroke: isStroke, op: name, ...(value != null ? { value } : {}) });
  const startV = P[`${pre}start`]?.value, endV = P[`${pre}end`]?.value;
  const angle = startV && endV ? Math.round((Math.atan2(endV[1] - startV[1], endV[0] - startV[0]) * 180) / Math.PI) : 0;
  const radius = startV && endV ? Math.round(Math.hypot(endV[0] - startV[0], endV[1] - startV[1])) : 0;
  const numIn = (label, value, apply, { min = -100000, max = 100000, unit = '' } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, unit ? `${label} (${unit})` : label),
    h('input', { class: 'field small', type: 'number', value: String(value), min: String(min), max: String(max), 'aria-label': label, onChange: (e) => apply(Math.max(min, Math.min(max, Number(e.target.value) || 0))), onKeydown: (e) => e.stopPropagation() }));
  return h('div', { class: 'stops' },
    h('div', { class: 'prop' }, app.state.comp.duration ? ui.keyButton(info, prop, p, 'Gradient colours') : null, h('span', { class: 'pname' }, 'Colours')),
    h('div', { class: 'grad-edit' }, bar, lane),
    cur ? h('div', { class: 'stop-row' }, h('span', { class: 'muted', style: 'font-size: 12.5px' }, `Colour ${selI + 1}`),
      h('input', { type: 'color', class: 'color-in', value: toHex(cur.color).slice(0, 7), 'data-rgba': toHex(cur.color), 'aria-label': `Colour ${selI + 1}`,
        onInput: (e) => { const n = clone(stops); n[selI].color = fromHex(e.target.dataset.rgba || e.target.value); send(n, true); },
        onChange: (e) => { const n = clone(stops); n[selI].color = fromHex(e.target.dataset.rgba || e.target.value); send(n); } }),
      h('input', { class: 'field tiny', type: 'number', min: '0', max: '100', value: String(Math.round(cur.offset * 100)), 'aria-label': `Colour ${selI + 1} place (%)`,
        onChange: (e) => { const n = clone(stops); n[selI].offset = Math.max(0, Math.min(100, Number(e.target.value))) / 100; send(n); }, onKeydown: (e) => e.stopPropagation() }),
      h('small', { class: 'unit' }, '%'),
      stops.length > 2 ? h('button', { class: 'small-btn', 'aria-label': `Remove colour ${selI + 1}`, html: icon('x', 14), onClick: () => { app.gradSel[key] = 0; send(stops.filter((_, j) => j !== selI)); } }) : null) : null,
    h('div', { class: 'add-row' },
      h('button', { class: 'small-btn', title: 'Swap the ends', onClick: () => op('reverse') }, 'Reverse'),
      h('button', { class: 'small-btn', title: 'The colour fading to see-through', onClick: () => op('fade') }, 'Fade out'),
      h('button', { class: 'small-btn', title: 'A radial highlight lit from the top left, so the shape looks rounded', onClick: () => op('shine') }, 'Shine'),
      h('button', { class: 'small-btn', title: 'Span the shape’s box again', onClick: () => op('fit') }, 'Fit to shape')),
    kind === 'radial' ? numIn('Radius', radius, (v) => op('radius', v), { min: 1, unit: 'px' }) : numIn('Angle', angle, (v) => op('angle', v), { min: -180, max: 180, unit: '°' }));
}

/// A pen's settings (the ones every pen uses; each pen ignores what it doesn't use).
export function penSettings(h, style, edit) {
  const num = (label, key, { k = 100, min = 0, max = 100, step = 5 } = {}) => h('label', { class: 'prop' }, h('span', { class: 'pname' }, label),
    h('input', { class: 'field small', type: 'number', min: String(min), max: String(max), step: String(step), value: String(Math.round((style[key] ?? 0) * k * 10) / 10), 'aria-label': label,
      onChange: (e) => edit((s) => { const v = Math.max(min, Math.min(max, Number(e.target.value))) / k; s[key] = step >= 1 && k === 1 ? Math.round(v) : v; }), onKeydown: (e) => e.stopPropagation() }));
  return h('div', { class: 'pen-settings' },
    num('Taper start %', 'taper_start'), num('Taper end %', 'taper_end'), num('Roughness %', 'roughness'), num('Grain %', 'grain'),
    num('Nib angle °', 'angle', { k: 1, min: -180, max: 180, step: 5 }), num('Thinnest %', 'thinning'), num('Flow %', 'flow'),
    num('Dash', 'dash', { k: 1, min: 0.1, max: 50, step: 0.5 }), num('Gap', 'gap', { k: 1, min: 0, max: 50, step: 0.5 }),
    num('Count', 'count', { k: 1, min: 1, max: 64, step: 1 }), num('Seed', 'seed', { k: 1, min: 0, max: 9999, step: 1 }));
}
