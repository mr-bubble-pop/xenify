// The plan's own fields (Story points, Customer, Release …): a control for
// each kind of value, how a value reads on a card, and Issue › Fields, where
// fields are made and given rules (required, lowest and highest, length,
// a pattern, how many choices). The engine checks every value against its
// field's rules and says what's wrong; the control shows that under itself.

import { h, icon, sheet, toast, colourPicker, modal } from './ui.js';
import { dayName } from './fmt.js';

/// The fields an issue of this type has.
export const fieldsFor = (v, t) => v.fields.filter((f) => !f.types?.length || f.types.includes(t.kind));

/// A value as it reads on a card or in a list ("£1,200", "Yes", "Front, Back").
export function fieldText(v, f, value) {
  if (value == null || value === '' || (Array.isArray(value) && !value.length)) return '';
  switch (f.kind) {
    case 'number': {
      const n = Number(value);
      const s = f.decimals != null ? n.toLocaleString(undefined, { minimumFractionDigits: f.decimals, maximumFractionDigits: f.decimals }) : n.toLocaleString();
      return `${f.prefix || ''}${s}${f.suffix ? ` ${f.suffix}` : ''}`;
    }
    case 'date': return dayName(value);
    case 'check': return value ? 'Yes' : 'No';
    case 'choices': return value.join(', ');
    case 'person': return v.resources.find((p) => p.id === value)?.name || 'Someone not in this plan';
    default: return String(value);
  }
}

/// A choice as a coloured lozenge.
export function choiceChip(f, name) {
  const i = (f.choices || []).indexOf(name);
  const c = (f.colors || [])[i] || '#A8A29E';
  return h('span', { class: 'choice', style: { '--c': c } }, name);
}

/// The value on a card or row: choices as lozenges, the rest as text.
export function fieldShow(v, f, value) {
  const s = fieldText(v, f, value);
  if (!s) return null;
  if (f.kind === 'choice') return choiceChip(f, value);
  if (f.kind === 'choices') return h('span', { class: 'choices' }, value.map((c) => choiceChip(f, c)));
  return h('span', { class: 'fval' }, s);
}

/// A control for field `f` holding `value`. `set(value)` changes it and
/// gives back what's wrong (a sentence) or null; the message shows under it.
export function fieldControl(v, f, value, set, id = null) {
  const wrap = h('div', { class: 'fctl' });
  const err = h('div', { class: 'f-error', role: 'alert' });
  const put = (x) => {
    const e = set(x);
    err.textContent = e || '';
    wrap.classList.toggle('bad', !!e);
    return !e;
  };
  let control;
  // Empty: a ghost saying what goes there, so there's something to click.
  const ghost = `Add ${f.name.toLowerCase()}`;
  switch (f.kind) {
    case 'long_text': {
      control = h('textarea', { class: 'field', id, rows: '3', placeholder: f.help || ghost });
      control.value = value || '';
      control.addEventListener('change', () => put(control.value));
      break;
    }
    case 'number': {
      const input = h('input', { class: 'field num', id, inputmode: 'decimal', value: value == null ? '' : String(value), autocomplete: 'off',
        placeholder: f.min != null && f.max != null ? `${ghost} (${f.min} to ${f.max})` : ghost });
      input.addEventListener('keydown', (e) => { if (e.key === 'Enter') input.blur(); });
      input.addEventListener('change', () => put(input.value.trim() === '' ? null : input.value));
      control = h('div', { class: 'affix' }, f.prefix ? h('span', { class: 'pre' }, f.prefix) : null, input, f.suffix ? h('span', { class: 'suf' }, f.suffix) : null);
      break;
    }
    case 'date': {
      const draw = (x, focus) => {
        if (!x && !focus) { control.replaceChildren(h('button', { class: 'due-add', type: 'button', id, onClick: () => draw('', true) }, `Add ${f.name.toLowerCase()}`)); return; }
        const input = h('input', { type: 'date', class: 'field', id, value: x || '', min: f.min_date || null, max: f.max_date || null, 'aria-label': f.name });
        input.addEventListener('change', () => put(input.value || null));
        control.replaceChildren(input, h('button', { class: 'icon-btn', type: 'button', 'aria-label': `No ${f.name}`, title: 'Clear', html: icon('close', 13), onClick: () => { if (put(null)) draw(null); } }));
        if (focus) input.focus();
      };
      control = h('div', { class: 'due-box' });
      draw(value, false);
      break;
    }
    case 'choice': {
      control = h('select', { class: 'field', id }, h('option', { value: '' }, '—'), (f.choices || []).map((c) => h('option', { value: c, selected: c === value }, c)));
      control.addEventListener('change', () => put(control.value || null));
      break;
    }
    case 'choices': {
      let picked = Array.isArray(value) ? [...value] : [];
      control = h('div', { class: 'pick-chips', role: 'group', 'aria-label': f.name, id });
      const draw = () => control.replaceChildren(...(f.choices || []).map((c, i) => h('button', {
        type: 'button', class: `pick-chip${picked.includes(c) ? ' on' : ''}`, 'aria-pressed': String(picked.includes(c)), style: { '--c': (f.colors || [])[i] || '#A8A29E' },
        onClick: () => {
          const next = picked.includes(c) ? picked.filter((x) => x !== c) : [...picked, c];
          if (put(next)) { picked = next; draw(); }
        },
      }, c)));
      draw();
      break;
    }
    case 'person': {
      control = h('select', { class: 'field', id }, h('option', { value: '' }, 'No one'), v.resources.map((p) => h('option', { value: p.id, selected: p.id === value }, p.name)));
      control.addEventListener('change', () => put(control.value || null));
      break;
    }
    case 'check': {
      const box = h('input', { type: 'checkbox', id, checked: !!value });
      box.addEventListener('change', () => { if (!put(box.checked)) box.checked = !box.checked; });
      control = h('label', { class: 'check-row' }, box, 'Yes');
      break;
    }
    default: {
      const type = f.kind === 'email' ? 'email' : f.kind === 'link' ? 'url' : 'text';
      const input = h('input', { class: 'field', id, type, value: value || '', autocomplete: 'off', maxlength: f.max_len || null,
        placeholder: f.kind === 'link' ? 'https://…' : f.kind === 'email' ? 'name@example.com' : ghost });
      input.addEventListener('keydown', (e) => { if (e.key === 'Enter') input.blur(); });
      input.addEventListener('change', () => put(input.value));
      // Only web and email links open (never javascript: and the like).
      control = f.kind === 'link' && /^(https?:|mailto:)/i.test(String(value || '').trim())
        ? h('div', { class: 'affix' }, input, h('a', { class: 'icon-btn', href: String(value).trim(), target: '_blank', rel: 'noopener', title: 'Open the link', 'aria-label': 'Open the link', html: icon('weblink', 15) }))
        : input;
    }
  }
  wrap.append(control, err);
  return wrap;
}

/// What a field's rules come to, in a line (under its name in the list).
function rulesLine(v, f) {
  const kind = v.field_kinds.find((k) => k.id === f.kind)?.name || f.kind;
  const bits = [kind];
  if (f.required) bits.push('required');
  if (f.kind === 'number') {
    if (f.min != null && f.max != null) bits.push(`${f.min} to ${f.max}`);
    else if (f.min != null) bits.push(`at least ${f.min}`);
    else if (f.max != null) bits.push(`at most ${f.max}`);
    if (f.decimals === 0) bits.push('whole numbers');
  }
  if (f.kind === 'choice' || f.kind === 'choices') bits.push(`${(f.choices || []).length} choices`);
  if (f.pattern) bits.push('pattern');
  if (f.types?.length) bits.push(`for ${f.types.map((t) => v.types.find((x) => x.id === t)?.name || t).join(', ')}`);
  return bits.join(' · ');
}

const BLANK = { name: '', kind: 'text', choices: [], colors: [], types: [], on_cards: false, in_list: false, on_create: false, help: '', required: false,
  min: null, max: null, decimals: null, prefix: '', suffix: '', min_len: null, max_len: null, pattern: '', pattern_says: '',
  min_date: null, max_date: null, not_past: false, min_picks: null, max_picks: null, default: null };

/// Issue › Fields: the plan's fields on the left, the one picked on the
/// right with its rules. Saved with Save (the engine checks the rules make sense).
export function fieldsDialog(app, pick = null) {
  let at = pick || app.view.fields[0]?.id || null;
  let draft = null;
  const list = h('div', { class: 'fd-list', role: 'listbox', 'aria-label': 'Fields' });
  const form = h('div', { class: 'fd-form' });
  const choose = (id) => { at = id; draft = id ? structuredClone({ ...BLANK, ...app.view.fields.find((f) => f.id === id) }) : { ...structuredClone(BLANK) }; draw(); };
  const draw = () => {
    const v = app.view;
    // (replaceChildren would show a missing piece as the text "null".)
    list.replaceChildren(...[
      ...v.fields.map((f) => h('button', { class: `fd-item${f.id === at ? ' on' : ''}`, role: 'option', 'aria-selected': String(f.id === at), onClick: () => choose(f.id) },
        h('b', {}, f.name), h('span', { class: 'muted' }, rulesLine(v, f)))),
      at === null && draft ? h('div', { class: 'fd-item on' }, h('b', {}, draft.name || 'New field'), h('span', { class: 'muted' }, 'Not saved yet')) : null,
      h('button', { class: 'btn plain small fd-add', onClick: () => choose(null) }, h('span', { html: icon('plus', 14) }), 'Add a field')].filter(Boolean));
    form.replaceChildren(draft ? editor(app, draft, at, (id) => { at = id; draft = structuredClone({ ...BLANK, ...app.view.fields.find((f) => f.id === id) }); draw(); }, () => { at = app.view.fields[0]?.id || null; draft = at ? structuredClone({ ...BLANK, ...app.view.fields.find((f) => f.id === at) }) : null; draw(); })
      : h('div', { class: 'fd-empty' }, h('p', {}, 'Fields are things every issue can have that Plan doesn’t ask for itself: story points, a customer, a release, a link to the design.'),
        h('button', { class: 'btn main', onClick: () => choose(null) }, 'Add a field')));
  };
  if (at) draft = structuredClone({ ...BLANK, ...app.view.fields.find((f) => f.id === at) });
  const box = h('div', { class: 'dlg fields-dlg', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Fields' },
    h('header', {}, h('h2', {}, 'Fields'), h('span', { class: 'dlg-sub' }, 'What issues in this plan can hold'), h('span', { class: 'grow' }),
      h('button', { class: 'icon-btn', 'aria-label': 'Close', html: icon('close', 16), onClick: () => close() })),
    h('div', { class: 'fd-cols' }, list, form));
  const close = sheet(box, () => app.render());
  draw();
}

function editor(app, d, id, saved, removed) {
  const v = app.view;
  const err = h('div', { class: 'f-error', role: 'alert' });
  const f = (label, control, hint) => h('div', { class: 'f' }, h('label', { class: 'f-label' }, label), control, hint ? h('span', { class: 'f-hint' }, hint) : null);
  const text = (key, props = {}) => { const i = h('input', { class: 'field', value: d[key] ?? '', autocomplete: 'off', ...props }); i.addEventListener('input', () => { d[key] = i.value; }); return i; };
  const num = (key, props = {}) => {
    const i = h('input', { class: 'field num', inputmode: 'decimal', value: d[key] == null ? '' : String(d[key]), autocomplete: 'off', ...props });
    i.addEventListener('input', () => { const t = i.value.trim(); d[key] = t === '' ? null : Number(t); });
    return i;
  };
  const tick = (key, label) => { const c = h('input', { type: 'checkbox', checked: !!d[key] }); c.addEventListener('change', () => { d[key] = c.checked; }); return h('label', { class: 'check-row' }, c, label); };
  const date = (key) => { const i = h('input', { type: 'date', class: 'field', value: d[key] || '' }); i.addEventListener('change', () => { d[key] = i.value || null; }); return i; };
  const body = h('div', { class: 'fd-body' });
  const redraw = () => body.replaceChildren(...rules());
  const kind = h('select', { class: 'field' }, v.field_kinds.map((k) => h('option', { value: k.id, selected: k.id === d.kind }, k.name)));
  kind.addEventListener('change', () => { d.kind = kind.value; d.default = null; redraw(); });

  const choices = () => {
    const rows = h('div', { class: 'fd-choices' });
    const drawRows = () => rows.replaceChildren(...d.choices.map((c, i) => {
      const name = h('input', { class: 'field', value: c, 'aria-label': `Choice ${i + 1}` });
      name.addEventListener('input', () => { d.choices[i] = name.value; });
      const sw = h('button', { class: 'swatch', type: 'button', style: { background: d.colors[i] || '#A8A29E' }, 'aria-label': 'Colour', title: 'Colour',
        onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); colourPicker(r.left, r.bottom + 4, (col) => { d.colors[i] = col; drawRows(); }, 'Grey'); } });
      const mv = (k) => () => { const j = i + k; if (j < 0 || j >= d.choices.length) return; [d.choices[i], d.choices[j]] = [d.choices[j], d.choices[i]]; [d.colors[i], d.colors[j]] = [d.colors[j], d.colors[i]]; drawRows(); };
      return h('div', { class: 'fd-choice' }, sw, name,
        h('button', { class: 'icon-btn', type: 'button', 'aria-label': 'Move up', title: 'Move up', disabled: i === 0, onClick: mv(-1) }, '↑'),
        h('button', { class: 'icon-btn', type: 'button', 'aria-label': 'Move down', title: 'Move down', disabled: i === d.choices.length - 1, onClick: mv(1) }, '↓'),
        h('button', { class: 'icon-btn', type: 'button', 'aria-label': `Remove ${c || 'the choice'}`, title: 'Remove', html: icon('close', 13), onClick: () => { d.choices.splice(i, 1); d.colors.splice(i, 1); drawRows(); } }));
    }), h('button', { class: 'btn plain small', type: 'button', onClick: () => { d.choices.push(''); d.colors.push(''); drawRows(); rows.querySelectorAll('input')[d.choices.length - 1]?.focus(); } }, '+ Add a choice'));
    drawRows();
    return rows;
  };

  const rules = () => {
    const out = [];
    const k = d.kind;
    if (k === 'text' || k === 'long_text' || k === 'link' || k === 'email') {
      out.push(h('div', { class: 'two' }, f('Fewest characters', num('min_len', { placeholder: 'Any' })), f('Most characters', num('max_len', { placeholder: 'Any' }))));
      if (k === 'text') {
        out.push(f('Pattern', text('pattern', { placeholder: 'e.g. [A-Z]{2,5}-\\d+', spellcheck: 'false' }), 'Optional. A regular expression the whole value must match.'),
          f('When it doesn’t match, say', text('pattern_says', { placeholder: 'e.g. Like OPS-12' })));
      }
    }
    if (k === 'number') {
      const dec = h('select', { class: 'field' }, [['', 'Any'], ['0', 'Whole numbers'], ['1', '1 place'], ['2', '2 places'], ['3', '3 places']].map(([x, n]) => h('option', { value: x, selected: String(d.decimals ?? '') === x }, n)));
      dec.addEventListener('change', () => { d.decimals = dec.value === '' ? null : Number(dec.value); });
      out.push(h('div', { class: 'two' }, f('Lowest', num('min', { placeholder: 'Any' })), f('Highest', num('max', { placeholder: 'Any' }))),
        h('div', { class: 'three' }, f('Places after the point', dec), f('Before it', text('prefix', { placeholder: 'e.g. £' })), f('After it', text('suffix', { placeholder: 'e.g. pts' }))));
    }
    if (k === 'date') out.push(h('div', { class: 'two' }, f('Earliest', date('min_date')), f('Latest', date('max_date'))), tick('not_past', 'Not before today'));
    if (k === 'choice' || k === 'choices') {
      out.push(f('Choices', choices()));
      if (k === 'choices') out.push(h('div', { class: 'two' }, f('Fewest picks', num('min_picks', { placeholder: 'Any' })), f('Most picks', num('max_picks', { placeholder: 'Any' }))));
    }
    if (k !== 'choice' && k !== 'choices' || d.choices.some((c) => c.trim())) {
      const probe = { ...d, choices: d.choices.filter((c) => c.trim()) };
      out.push(f('New issues start with', fieldControl(v, probe, d.default, (x) => { d.default = x; return null; }), 'Optional.'));
    }
    return out;
  };

  const types = h('div', { class: 'fd-types' }, v.types.map((t) => {
    const c = h('input', { type: 'checkbox', checked: !d.types.length || d.types.includes(t.id) });
    c.addEventListener('change', () => {
      const on = [...types.querySelectorAll('input')].map((x, i) => (x.checked ? v.types[i].id : null)).filter(Boolean);
      d.types = on.length === v.types.length ? [] : on;
    });
    return h('label', { class: 'check-row' }, c, t.name);
  }));
  const save = () => {
    const value = { ...d, choices: d.choices.map((c) => c.trim()).filter(Boolean), name: d.name.trim() };
    value.colors = d.choices.map((c, i) => (c.trim() ? d.colors[i] || '' : null)).filter((x) => x !== null);
    delete value.id;
    try {
      const out = app.attempt('set_item', { list: 'fields', id: id || '', value, what: id ? `Changed the field ${value.name}` : `Added the field ${value.name}` });
      err.textContent = '';
      toast(id ? 'Field saved' : `Added “${value.name}”`);
      saved(out.id);
    } catch (e) { err.textContent = e.message; }
  };
  const remove = async () => {
    const n = v.tasks.filter((t) => t.fields?.[id] != null).length;
    const ok = await modal(`Delete “${d.name}”?`, h('p', {}, n ? `${n} ${n === 1 ? 'issue has' : 'issues have'} a value in it; those go too (Undo brings them back).` : 'No issue has a value in it yet.'), [['Cancel', null], ['Delete', true, true]]);
    if (!ok) return;
    app.read('group_start');
    // The issues as they are now (someone editing together may have changed them meanwhile).
    for (const t of app.view.tasks.filter((x) => x.fields?.[id] != null)) {
      const rest = { ...t.fields };
      delete rest[id];
      app.read('set_task', { id: t.id, field: 'fields', value: rest });
    }
    app.read('set_item', { list: 'fields', id, value: null });
    app.edit('group_end', { what: `Deleted the field ${d.name}` });
    removed();
  };
  redraw();
  return h('div', { class: 'fd-edit' },
    h('div', { class: 'two' }, f('Name', text('name', { placeholder: 'e.g. Story points' })), f('Kind', kind)),
    f('Help', text('help', { placeholder: 'What to put in (shown under it)' })),
    body,
    f('For', types),
    h('div', { class: 'fd-ticks' }, tick('required', 'Required: an issue without it needs details'), tick('on_create', 'Ask for it in Create'), tick('on_cards', 'Show it on Board cards'), tick('in_list', 'Show it in the Backlog')),
    err,
    h('div', { class: 'fd-actions' }, id ? h('button', { class: 'btn plain', onClick: remove }, 'Delete the field') : null, h('span', { class: 'grow' }), h('button', { class: 'btn main', onClick: save }, id ? 'Save' : 'Add the field')));
}
