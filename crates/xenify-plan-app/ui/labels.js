// Labels (tags): any words on an issue ("Front end", "Customer A"), typed
// where the issue is made or opened, suggested from the plan's other
// labels, and filtered by from the toolbar. A label is there while some
// issue has it; Issue › Labels renames or removes one everywhere.

import { h, modal, ask, toast } from './ui.js';

const COLOURS = ['#2a78d6', '#1baf7a', '#6250d6', '#C2410C', '#B45309', '#0F766E', '#d55181', '#57534E', '#1D4ED8', '#15803D'];

/// A label's colour: from its letters, so it is the same everywhere.
export function labelColour(name) {
  let n = 0;
  for (const c of String(name).toLowerCase()) n = (n * 31 + c.charCodeAt(0)) >>> 0;
  return COLOURS[n % COLOURS.length];
}

export function labelChip(name, remove = null) {
  return h('span', { class: 'label', style: { '--c': labelColour(name) } }, name,
    remove ? h('button', { class: 'x', type: 'button', 'aria-label': `Take off ${name}`, onClick: (e) => { e.stopPropagation(); remove(); } }, '×') : null);
}

export function labelChips(labels) {
  return labels.length ? h('span', { class: 'labels' }, labels.map((l) => labelChip(l))) : null;
}

/// The labels on an issue as chips and a box to type more: Enter or a
/// comma adds one, Backspace in the empty box takes the last off.
/// `onChange(list)` gets the new list.
export function labelEditor(app, labels, onChange, id = null) {
  let list = [...labels];
  const box = h('div', { class: 'label-edit' });
  const input = h('input', { class: 'label-input', id, placeholder: list.length ? '' : 'Add labels', 'aria-label': 'Add a label', autocomplete: 'off' });
  const sugg = h('div', { class: 'label-sugg', role: 'listbox', hidden: true });
  let at = 0;
  let moved = false;
  const known = () => (app.view?.labels || []).map((l) => l.name).filter((n) => !list.some((x) => x.toLowerCase() === n.toLowerCase()));
  const matches = () => {
    const q = input.value.trim().toLowerCase();
    return known().filter((n) => !q || n.toLowerCase().includes(q)).slice(0, 8);
  };
  const set = (next) => { list = next; onChange(list); draw(); input.focus(); };
  const add = (name) => {
    const n = name.replace(/,/g, ' ').replace(/\s+/g, ' ').trim();
    input.value = '';
    moved = false;
    at = 0;
    if (!n || list.some((x) => x.toLowerCase() === n.toLowerCase())) { draw(); return; }
    set([...list, n]);
  };
  const drawSugg = () => {
    const m = matches();
    const typed = input.value.trim();
    sugg.hidden = document.activeElement !== input || (!m.length && !typed);
    at = Math.min(at, Math.max(0, m.length - 1));
    sugg.replaceChildren(...m.map((n, i) => h('button', { type: 'button', class: `item${i === at ? ' active' : ''}`, role: 'option', onMousedown: (e) => e.preventDefault(), onClick: () => add(n) }, labelChip(n))),
      ...(typed && !m.some((n) => n.toLowerCase() === typed.toLowerCase()) ? [h('div', { class: 'f-hint pad' }, `Enter adds the new label “${typed}”`)] : []));
  };
  const draw = () => {
    input.placeholder = list.length ? '' : 'Add labels';
    box.replaceChildren(...list.map((l) => labelChip(l, () => set(list.filter((x) => x !== l)))), input, sugg);
    drawSugg();
  };
  input.addEventListener('input', () => { at = 0; moved = false; drawSugg(); });
  input.addEventListener('focus', drawSugg);
  input.addEventListener('blur', () => setTimeout(() => { if (input.value.trim()) add(input.value); sugg.hidden = true; }, 120));
  input.addEventListener('keydown', (e) => {
    const m = matches();
    if (e.key === 'ArrowDown' && m.length) { e.preventDefault(); moved = true; at = Math.min(m.length - 1, at + 1); drawSugg(); }
    else if (e.key === 'ArrowUp' && m.length) { e.preventDefault(); moved = true; at = Math.max(0, at - 1); drawSugg(); }
    else if (e.key === 'Enter' || e.key === ',' || e.key === 'Tab') {
      // The typed words, unless a suggestion was picked with the arrows (or nothing was typed).
      const typed = input.value.trim();
      const pick = (moved || !typed) && e.key === 'Enter' && m.length ? m[at] : typed;
      if (!pick) return;
      e.preventDefault();
      e.stopPropagation();
      add(pick);
    } else if (e.key === 'Backspace' && !input.value && list.length) set(list.slice(0, -1));
    else if (e.key === 'Escape' && input.value) { e.preventDefault(); e.stopPropagation(); input.value = ''; drawSugg(); }
  });
  box.addEventListener('click', (e) => { if (e.target === box) input.focus(); });
  draw();
  return box;
}

/// Issue › Labels: every label with how many issues have it; rename or remove.
export async function labelsDialog(app) {
  const draw = () => {
    const v = app.view;
    if (!v.labels.length) return h('p', { class: 'f-hint' }, 'No labels yet. Type them under Labels when you make or open an issue.');
    return h('div', { class: 'label-list' }, v.labels.map((l) => h('div', { class: 'label-row' },
      labelChip(l.name), h('span', { class: 'muted' }, `${l.count} ${l.count === 1 ? 'issue' : 'issues'}`), h('span', { class: 'grow' }),
      h('button', { class: 'btn plain small', onClick: async () => {
        const to = await ask('Rename the label', 'New name', l.name, 'Rename');
        if (to && to.trim() && to.trim() !== l.name) { app.edit('rename_label', { from: l.name, to: to.trim() }); body.replaceChildren(draw()); }
      } }, 'Rename'),
      h('button', { class: 'btn plain small', onClick: () => {
        app.edit('rename_label', { from: l.name, to: '' });
        toast(`Took “${l.name}” off ${l.count} ${l.count === 1 ? 'issue' : 'issues'}`, ['Undo', () => app.edit('undo')]);
        body.replaceChildren(draw());
      } }, 'Remove'))));
  };
  const body = h('div', {}, draw());
  await modal('Labels', body, [['Close', true, true]]);
}
