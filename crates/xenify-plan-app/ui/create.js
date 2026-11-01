// Making issues: the Create dialog (C or the Create button: a summary first,
// everything else optional, "Create another" to keep going), the box at the
// foot of a Board column or Backlog group (type a summary, Enter, next), and
// the issue picker both use for links.

import { h, icon, popup, closePopups, sheet, toast, modal } from './ui.js';
import { oneLine, richText, Checker } from './text.js';
import { typeIcon, whoChoices, parseEst, estText, PRIORITIES, keyClass, dueControl } from './pieces.js';
import { labelEditor } from './labels.js';
import { fieldControl } from './fields.js';

/// The issue type as a button with its icon; a click lists the types, each
/// with its icon (arrows and Enter work too).
export function typePicker(v, kinds, value, set, { id, small = false } = {}) {
  const b = h('button', { type: 'button', class: `typepick${small ? ' small' : ''}`, id, 'aria-haspopup': 'listbox', 'aria-label': 'Issue type' });
  const draw = () => b.replaceChildren(typeIcon(v, value, small ? 14 : 16), h('span', {}, kinds.find((t) => t.id === value)?.name || value), h('span', { class: 'chev', html: icon('down', 14) }));
  draw();
  b.addEventListener('click', () => {
    let at = Math.max(0, kinds.findIndex((t) => t.id === value));
    const list = h('div', { class: 'type-list', role: 'listbox', 'aria-label': 'Issue type', tabindex: '-1' });
    const choose = (k) => { value = k; draw(); closePopups(); b.focus(); set(k); };
    const fill = () => list.replaceChildren(...kinds.map((t, i) => h('button', { type: 'button', class: `item${i === at ? ' active' : ''}`, role: 'option', 'aria-selected': String(t.id === value), onClick: () => choose(t.id) },
      typeIcon(v, t.id), h('span', {}, t.name), t.id === value ? h('span', { class: 'check', html: icon('check', 16) }) : null)));
    list.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); at = Math.min(kinds.length - 1, at + 1); fill(); }
      if (e.key === 'ArrowUp') { e.preventDefault(); at = Math.max(0, at - 1); fill(); }
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); choose(kinds[at].id); }
    });
    fill();
    const r = b.getBoundingClientRect();
    popup(list, r.left, r.bottom + 4, () => { if (!document.activeElement || document.activeElement === document.body) b.focus(); });
    list.focus();
  });
  return b;
}

/// After an issue is made: say so with a way to it, and show it where it is.
function made(app, out, title) {
  if (!out) return;
  app.tab.flash = out.id;
  app.render();
  toast(`${out.key} “${title}” created`, ['View', () => app.openTask(out.id)]);
  setTimeout(() => { if (app.tab.flash === out.id) app.tab.flash = null; }, 1800);
}

export function openCreate(app, preset = {}) {
  const v = app.view;
  const day = v.settings.day_minutes;
  const epics = v.tasks.filter((t) => t.epic);
  const people = v.resources;
  const draft = {
    kind: preset.kind || app.prefs.lastKind || 'task',
    // The last epic again only for issues of the same kind (not a milestone or a bug after stories).
    parent: preset.parent !== undefined ? preset.parent
      : (epics.some((e) => e.id === app.tab.lastEpic) && (preset.kind || app.prefs.lastKind || 'task') === app.tab.lastEpicKind && preset.kind !== 'milestone' ? app.tab.lastEpic : null),
    who: preset.who ?? null,
    priority: 'medium',
    waits: [],
    labels: [],
    fields: {},
    status: preset.status || v.statuses[0]?.id,
  };
  let another = false;
  const err = h('div', { class: 'f-error', role: 'alert' });
  const summary = oneLine({ label: 'Summary', placeholder: 'What needs doing?', className: 'oneline big', text: preset.title || '' });
  const desc = richText({ label: 'Description', placeholder: 'What it is, and what “done” means. Markdown works: # heading, - list, [] checklist' });
  const labels = labelEditor(app, [], (l) => { draft.labels = l; }, 'c-labels');
  // The plan's own fields asked for here: those set to be, and required ones.
  const extra = h('div', { class: 'two' });
  const drawFields = () => {
    const asked = v.fields.filter((f) => (f.on_create || f.required) && (!f.types?.length || f.types.includes(draft.kind)));
    extra.hidden = !asked.length;
    extra.replaceChildren(...asked.map((f) => h('div', { class: 'f' },
      h('label', { class: 'f-label', for: `c-f-${f.id}` }, f.name, f.required ? h('span', { class: 'req', title: 'Needed' }, ' *') : null),
      fieldControl(v, f, draft.fields[f.id] ?? f.default, (x) => { draft.fields[f.id] = x; return null; }, `c-f-${f.id}`),
      f.help ? h('span', { class: 'f-hint' }, f.help) : null)));
  };
  // The plan's usual estimate to start with (Plan settings); emptied: none.
  const usual = (v.settings.default_days ?? 1) > 0 ? estText(Math.round(v.settings.default_days * day), day) : '';
  const est = h('input', { class: 'field', id: 'c-est', placeholder: 'e.g. 3d or 4h', autocomplete: 'off', value: usual });
  const estSays = h('span', { class: 'f-hint' }, 'Work, not dates: the Timeline works out when.');
  est.addEventListener('input', () => {
    const m = parseEst(est.value, day);
    estSays.textContent = est.value.trim() === '' ? 'Work, not dates: the Timeline works out when.' : m == null ? 'Write it like 3d, 4h or 1d 4h' : `${Math.round((m / 60) * 10) / 10} hours of work`;
  });
  let dueValue = null;
  const due = dueControl({ value: null, id: 'c-due', inputClass: 'field', onChange: (x) => { dueValue = x; } });
  const select = (id, options, value, set) => {
    const s = h('select', { class: 'field', id }, options.map(([k, label]) => h('option', { value: k ?? '', selected: (k ?? '') === (value ?? '') }, label)));
    s.addEventListener('change', () => set(s.value || null));
    return s;
  };
  const kinds = v.types.filter((t) => t.id !== 'epic' || !preset.parent);
  const typeSel = typePicker(v, kinds, draft.kind, (x) => { draft.kind = x; drawKind(); }, { id: 'c-type' });
  const epicSel = select('c-epic', [[null, 'None'], ...epics.map((e) => [e.id, `${e.key} ${e.title}`])], draft.parent, (x) => { draft.parent = x; });
  const whoSel = select('c-who', whoChoices(v, true), draft.who, (x) => { draft.who = x; });
  const prioSel = select('c-prio', PRIORITIES.map(([k, n]) => [k, n]), draft.priority, (x) => { draft.priority = x || 'medium'; });
  const waits = h('div', { class: 'chips' });
  const drawWaits = () => {
    waits.replaceChildren(
      ...draft.waits.map((id) => {
        const t = v.tasks.find((x) => x.id === id);
        return h('span', { class: 'chip big' }, h('span', { class: 'key' }, t.key), t.title,
          h('button', { class: 'x', 'aria-label': `Remove ${t.key}`, onClick: () => { draft.waits = draft.waits.filter((x) => x !== id); drawWaits(); } }, '×'));
      }),
      pickButton(app, '+ Add what it starts after', draft.waits, (id) => { draft.waits.push(id); drawWaits(); waits.querySelector('.add-link')?.focus(); }));
  };
  drawWaits();
  const field = (label, control, hint, forId) => h('div', { class: 'f' }, h('label', { class: 'f-label', for: forId || null }, label), control, hint || null);
  const epicRow = field('Epic', epicSel, null, 'c-epic');
  const estRow = field('Estimate', est, estSays, 'c-est');
  const drawKind = () => {
    epicRow.hidden = draft.kind === 'epic';
    estRow.hidden = draft.kind === 'epic' || draft.kind === 'milestone';
    drawFields();
  };
  drawKind();
  const anotherBox = h('input', { type: 'checkbox', id: 'c-another' });
  anotherBox.addEventListener('change', () => { another = anotherBox.checked; });

  const submit = () => {
    const title = summary.textContent.trim();
    if (!title) { err.textContent = 'Give it a summary, such as “Write the FAQ”.'; summary.focus(); return; }
    const work = parseEst(est.value, day);
    if (est.value.trim() && work == null) { err.textContent = 'Write the estimate like 3d, 4h or 1d 4h.'; est.focus(); return; }
    err.textContent = '';
    const fields = Object.fromEntries(Object.entries(draft.fields).filter(([id]) => v.fields.some((f) => f.id === id && (!f.types?.length || f.types.includes(draft.kind)))));
    const args = { title, kind: draft.kind, status: draft.status, who: draft.who, priority: draft.priority, description: desc.html, waits_for: draft.waits, labels: draft.labels, fields };
    if (draft.kind !== 'epic' && draft.parent) args.parent = draft.parent;
    if (work != null && draft.kind !== 'epic' && draft.kind !== 'milestone') args.work = work;
    // Emptied on purpose: no estimate (the engine would give it the usual one).
    if (!est.value.trim()) args.work = null;
    if (dueValue) args.due = dueValue;
    let out;
    try { out = app.attempt('add_task', args); } catch (e) { err.textContent = e.message; return; }
    app.prefs.lastKind = draft.kind === 'epic' || draft.kind === 'milestone' ? app.prefs.lastKind : draft.kind;
    app.savePrefs();
    if (draft.parent) { app.tab.lastEpic = draft.parent; app.tab.lastEpicKind = draft.kind; }
    made(app, out, title);
    if (another) {
      summary.textContent = '';
      desc.html = '';
      est.value = usual;
      dueValue = null;
      due.reset();
      // Labels and fields stay: the next issue is usually like this one.
      estSays.textContent = 'Work, not dates: the Timeline works out when.';
      summary.focus();
    } else close();
  };
  const box = h('div', { class: 'dlg create', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Create issue' },
    h('header', {}, h('h2', {}, 'Create issue'), h('span', { class: 'dlg-sub' }, v.title), h('span', { class: 'grow' }),
      h('button', { class: 'icon-btn', 'aria-label': 'Close', html: icon('close', 16), onClick: () => close() })),
    h('div', { class: 'dlg-body' },
      h('div', { class: 'two' }, field('Issue type', typeSel, null, 'c-type'), epicRow),
      h('div', { class: 'f' }, h('span', { class: 'f-label' }, 'Summary ', h('span', { class: 'req', title: 'Needed' }, '*')), summary, err),
      h('div', { class: 'f' }, h('span', { class: 'f-label' }, 'Description'), desc.el),
      h('div', { class: 'two' }, field('Assignee', whoSel, people.length ? null : h('span', { class: 'f-hint' }, v.on_server ? 'People are the server’s accounts.' : 'Connect to a server to give issues to people.'), 'c-who'), estRow),
      h('div', { class: 'f' }, h('span', { class: 'f-label' }, 'Starts after'), waits, h('span', { class: 'f-hint' }, 'It starts once these are done.')),
      h('div', { class: 'two' }, field('Priority', prioSel, null, 'c-prio'), field('Due date', due, h('span', { class: 'f-hint' }, 'Only when there’s a real deadline.'), 'c-due')),
      field('Labels', labels, h('span', { class: 'f-hint' }, 'Words to sort and filter by, like “Front end” or a customer. Enter after each.'), 'c-labels'),
      extra,
      h('div', { class: 'f-foot' }, h('button', { class: 'linkish small', type: 'button', onClick: () => { close(); import('./fields.js').then((m) => m.fieldsDialog(app)); } }, 'Add a field of your own…'))),
    h('footer', {},
      h('label', { class: 'check-row', for: 'c-another' }, anotherBox, 'Create another'),
      h('span', { class: 'grow' }),
      h('button', { class: 'btn plain', onClick: () => cancel() }, 'Cancel'),
      h('button', { class: 'btn main', onClick: submit, title: 'Create (Ctrl+Enter)' }, 'Create')));
  // Ctrl+Enter creates from anywhere while the dialog is open (focus may be on a picker that just closed).
  const keys = (e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); e.stopPropagation(); submit(); } };
  document.addEventListener('keydown', keys, true);
  const close = sheet(box, () => document.removeEventListener('keydown', keys, true));
  // Something typed: ask before throwing it away.
  const cancel = async () => {
    if ((summary.textContent.trim() || desc.html) && !(await modal('Throw away this issue?', h('p', {}, 'What you typed isn’t kept.'), [['Keep writing', null], ['Throw it away', true, true]]))) return;
    close();
  };
  setTimeout(() => { summary.focus(); if (preset.title) getSelection().selectAllChildren(summary), getSelection().collapseToEnd(); }, 20);
}

/// "+ Create issue" at the foot of a column or group: type, Enter, and the next.
export function quickAdd(app, preset, label = 'Create issue') {
  const holder = h('div', { class: 'quick' });
  // Only for people who may create issues here.
  if (app.view?.may && !app.view.may.includes('plan.create')) return holder;
  const button = h('button', { class: 'quick-btn', onClick: () => open() }, h('span', { html: icon('plus', 15) }), label);
  holder.append(button);
  const key = JSON.stringify(preset);
  // What's typed is kept in the tab, so the view being drawn again (a save,
  // someone else's change, the minute's refresh) doesn't throw it away.
  const shut = () => { app.tab.quick = null; app.tab.quickText = ''; holder.replaceChildren(button); };
  const open = (again = false) => {
    const v = app.view;
    let kind = preset.kind || (app.prefs.lastKind && app.prefs.lastKind !== 'epic' ? app.prefs.lastKind : 'task');
    if (again && app.tab.quickKind) kind = app.tab.quickKind;
    if (!again) app.tab.quickText = '';
    app.tab.quick = key;
    // One line of text, spell- and grammar-checked like every summary.
    const input = h('div', { class: 'quick-input', contenteditable: 'plaintext-only', role: 'textbox', spellcheck: 'false', 'data-ph': preset.kind === 'epic' ? 'Name of the epic' : 'What needs doing?', 'aria-label': preset.kind === 'epic' ? 'Name of the new epic' : 'Summary of the new issue' });
    Object.defineProperty(input, 'value', { get: () => input.textContent.replace(/\s+/g, ' '), set: (x) => { input.textContent = x; } });
    input.addEventListener('paste', (e) => { e.preventDefault(); document.execCommand('insertText', false, (e.clipboardData.getData('text/plain') || '').replace(/\s*\n\s*/g, ' ')); });
    if (again && app.tab.quickText) input.textContent = app.tab.quickText;
    input.addEventListener('input', () => { app.tab.quickText = input.textContent; });
    new Checker(input);
    const kinds = v.types.filter((t) => t.id !== 'epic' && t.id !== 'milestone');
    const typeSel = preset.kind ? null : typePicker(v, kinds, kind, (x) => { kind = x; app.tab.quickKind = x; input.focus(); }, { small: true });
    // Shift+Enter: the Create dialog, with what's typed so far.
    const full = () => { const title = input.value.trim(); shut(); openCreate(app, { ...preset, kind, title }); };
    holder.replaceChildren(h('div', { class: 'quick-box' }, input, h('div', { class: 'quick-row' }, typeSel, h('span', { class: 'f-hint' }, 'Enter to create · Esc to stop'))));
    // Drawn again: back where the typing was, unless the person is busy elsewhere.
    const free = !document.activeElement || document.activeElement === document.body || !document.activeElement.isConnected;
    if (!again || free) {
      input.focus();
      if (input.textContent) { getSelection().selectAllChildren(input); getSelection().collapseToEnd(); }
    }
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); shut(); }
      if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); full(); return; }
      if (e.key === 'Enter' && !input.value.trim()) { e.preventDefault(); return; }
      if (e.key === 'Enter' && input.value.trim()) {
        e.preventDefault();
        const title = input.value.trim();
        app.tab.quickText = '';
        made(app, app.edit('add_task', { title, kind, ...preset }), title);
      }
    });
    input.addEventListener('blur', () => setTimeout(() => {
      if (holder.isConnected && !holder.contains(document.activeElement) && !document.querySelector('.popup .type-list') && !input.value.trim()) { if (app.tab.quick === key) shut(); }
    }, 200));
  };
  // Drawn again after adding one: open again, ready for the next.
  if (app.tab.quick === key) setTimeout(() => { if (holder.isConnected) open(true); }, 0);
  return holder;
}

/// A button that opens a search over the plan's issues; `pick(id)` gets the one chosen.
export function pickButton(app, label, exclude, pick, filter = (t) => !t.epic) {
  const b = h('button', { class: 'btn plain small add-link', type: 'button' }, label);
  b.addEventListener('click', () => {
    const v = app.view;
    const input = h('input', { class: 'search', placeholder: 'Key or words', 'aria-label': 'Find an issue' });
    const list = h('div', { class: 'pick-list', role: 'listbox' });
    let hits = [];
    let at = 0;
    const draw = () => {
      const q = input.value.trim().toLowerCase();
      hits = v.tasks.filter((t) => filter(t) && !exclude.includes(t.id) && (!q || `${t.key} ${t.title}`.toLowerCase().includes(q))).slice(0, 12);
      at = Math.min(at, Math.max(0, hits.length - 1));
      list.replaceChildren(...(hits.length ? hits.map((t, i) => h('button', { class: `item${i === at ? ' active' : ''}`, role: 'option', onClick: () => { closePopups(); pick(t.id); } },
        typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key), h('span', { class: 'pick-title' }, t.title))) : [h('div', { class: 'f-hint pad' }, 'No issue matches')]));
    };
    input.addEventListener('input', () => { at = 0; draw(); });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); at = Math.min(hits.length - 1, at + 1); draw(); }
      if (e.key === 'ArrowUp') { e.preventDefault(); at = Math.max(0, at - 1); draw(); }
      if (e.key === 'Enter' && hits[at]) { e.preventDefault(); closePopups(); pick(hits[at].id); }
    });
    const r = b.getBoundingClientRect();
    popup(h('div', { class: 'picker' }, input, list), r.left, r.bottom + 4);
    draw();
    input.focus();
  });
  return b;
}
