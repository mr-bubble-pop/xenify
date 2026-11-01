// Small pieces every view uses: issue type icons, priority, faces, status
// lozenges, epic chips, estimates, and filtering by the toolbar.

import { h } from './ui.js';
import { initials } from './fmt.js';

const GLYPH = {
  epic: '<path d="M8.5 2L4 9h3.5L7 14l5-7H8.5z" fill="#fff"/>',
  story: '<path d="M5 3h6v10l-3-2.2L5 13z" fill="#fff"/>',
  task: '<path d="M4.5 8.2l2.3 2.3 4.7-5" stroke="#fff" stroke-width="2" fill="none"/>',
  bug: '<circle cx="8" cy="8" r="3" fill="#fff"/>',
  milestone: '<path d="M8 3l5 5-5 5-5-5z" fill="#fff"/>',
};

export const typeOf = (v, kind) => v.types.find((x) => x.id === kind) || { id: kind, name: kind || 'Issue', color: '#78716C', shape: 'square' };

/// The issue type's little square (round for bugs).
export function typeIcon(v, kind, size = 16, colour = null) {
  const t = typeOf(v, kind);
  const s = h('span', { class: `ty${t.shape === 'round' ? ' round' : ''}`, title: t.name, style: { background: colour || t.color, width: `${size}px`, height: `${size}px` } });
  s.innerHTML = `<svg aria-hidden="true" width="${size - 4}" height="${size - 4}" viewBox="0 0 16 16">${GLYPH[t.id] || '<rect x="5" y="5" width="6" height="6" rx="1" fill="#fff"/>'}</svg>`;
  return s;
}

export const PRIORITIES = [['highest', 'Highest', '#B91C1C'], ['high', 'High', '#C2410C'], ['medium', 'Medium', '#D97706'], ['low', 'Low', '#2a78d6']];

export function prioIcon(p) {
  const [id, name, colour] = PRIORITIES.find((x) => x[0] === p) || PRIORITIES[2];
  const d = { highest: 'M3 9l5-4 5 4M3 13l5-4 5 4', high: 'M3 11l5-5 5 5', medium: 'M3 6h10M3 10h10', low: 'M3 6l5 5 5-5' }[id];
  return h('span', { class: 'prio', title: `Priority: ${name}`, html: `<svg aria-hidden="true" width="14" height="14" viewBox="0 0 16 16"><path d="${d}" stroke="${colour}" stroke-width="2" fill="none"/></svg>` });
}

/// Whether I may make this kind of change here (the server's say: a
/// document role, "can comment" or "can view").
export const may = (v, what) => !v.may || v.may.includes(`plan.${what}`);

/// Turns off the controls in `el` (left readable): for changes I may not make.
export function lock(el, why = 'Your access to this plan doesn’t allow this') {
  if (!el) return el;
  el.classList.add('locked');
  for (const x of el.querySelectorAll('input, select, textarea, button')) { x.disabled = true; x.title = why; }
  for (const x of el.querySelectorAll('[contenteditable]')) x.setAttribute('contenteditable', 'false');
  if (el.matches('input, select, textarea, button')) { el.disabled = true; el.title = why; }
  return el;
}

export const person = (v, id) => v.resources.find((r) => r.id === id);
export const group = (v, id) => (v.groups || []).find((g) => g.id === id);

/// Assignee choices: people, then groups (whoever in it is free first does it).
export function whoChoices(v, me = false) {
  // People with work in this plan (and me) first: a server can have hundreds.
  const busy = new Set([...v.tasks.flatMap((t) => [t.who, t.by]), v.me_resource].filter(Boolean));
  const byName = (a, b) => (busy.has(b.id) - busy.has(a.id)) || a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
  const people = [...v.resources].sort(byName).map((p) => [p.id, me && p.id === v.me_resource ? `${p.name} (me)` : p.name]);
  const groups = (v.groups || []).map((g) => [g.id, `${g.name} (any of ${g.members.length})`]);
  return [[null, 'No one yet'], ...people, ...groups];
}

/// An issue's assignee: a face, or for group work the group's square with
/// the face of the member who gets it.
export function assignee(v, t, size = 24) {
  const g = t.who && group(v, t.who);
  if (!g) return avatar(v, t.who, size);
  const style = { width: `${size}px`, height: `${size}px`, fontSize: `${Math.round(size * 0.42)}px` };
  const by = t.by && person(v, t.by);
  const title = by ? `${g.name}: ${by.name} is free first` : `${g.name}: nobody free yet`;
  return h('span', { class: 'av-group', title },
    h('span', { class: 'av grp', style }, initials(g.name)),
    by ? h('span', { class: 'av', style: { ...style, background: personColour(v, by.id) } }, initials(by.name)) : null);
}

const FACES = ['#2a78d6', '#1baf7a', '#6250d6', '#C2410C', '#D97706', '#0F766E', '#d55181', '#57534E', '#0E7490', '#A16207', '#7C3AED', '#BE123C'];
// The plan's people in order (people with work in it first), per view.
const placeOf = new WeakMap();
/// A person's colour: by their place among the people with work in this
/// plan (then everyone else), so people on the same Timeline don't share one.
export function personColour(v, id) {
  let order = placeOf.get(v);
  if (!order) {
    const busy = new Set([...(v.tasks || []).flatMap((t) => [t.who, t.by]), v.me_resource].filter(Boolean));
    order = [...v.resources.filter((r) => busy.has(r.id)), ...v.resources.filter((r) => !busy.has(r.id))].map((r) => r.id);
    placeOf.set(v, order);
  }
  const k = order.indexOf(id);
  if (k >= 0) return FACES[k % FACES.length];
  let n = 0;
  for (const c of id || '') n = (n * 31 + c.charCodeAt(0)) >>> 0;
  return FACES[n % FACES.length];
}

/// Someone's face (initials on their colour); a dashed circle for no one yet.
export function avatar(v, id, size = 24) {
  const p = id && person(v, id);
  const style = { width: `${size}px`, height: `${size}px`, fontSize: `${Math.round(size * 0.42)}px` };
  if (!p) return h('span', { class: 'av none', title: id ? 'Someone not in this plan' : 'No one yet', style }, id ? '?' : '');
  return h('span', { class: 'av', title: p.name, style: { ...style, background: personColour(v, p.id) } }, initials(p.name));
}

export const statusOf = (v, id) => v.statuses.find((s) => s.id === id) || { id, name: '—', color: '#A8A29E' };

export function lozenge(v, id) {
  const s = statusOf(v, id);
  return h('span', { class: 'lozenge', style: { '--c': s.color || '#A8A29E' } }, s.name);
}

const EPIC = ['#6250d6', '#1baf7a', '#2a78d6', '#C2410C', '#D97706', '#0F766E', '#d55181'];
/// A mirror's colour: from where it sits among the mirrors.
export function mirrorColour(v, id) {
  const k = v.tasks.filter((t) => t.mirror).findIndex((t) => t.id === id);
  return EPIC[(k < 0 ? 0 : k) % EPIC.length];
}
/// An epic's colour: picked from where it sits among the epics, so they differ.
export function epicColour(v, id) {
  const epics = v.tasks.filter((t) => t.epic);
  const k = epics.findIndex((t) => t.id === id);
  return EPIC[(k < 0 ? 0 : k) % EPIC.length];
}

export function epicChip(v, id) {
  const e = v.tasks.find((t) => t.id === id);
  if (!e) return null;
  return h('span', { class: 'epic-chip', style: { '--c': epicColour(v, id) } }, e.title || e.key);
}

/// Minutes as "2d 4h" (a day is the plan's working day).
export function estText(min, dayMinutes = 450) {
  if (min == null) return '';
  if (min === 0) return '0h';
  const d = Math.floor(min / dayMinutes + 1e-9);
  const rest = Math.round(((min - d * dayMinutes) / 60) * 10) / 10;
  return [d ? `${d}d` : '', rest ? `${rest}h` : ''].filter(Boolean).join(' ') || '0h';
}

/// "3d", "4h", "1d 4h", "1.5d", "90m", "2w" → minutes (null when empty or unreadable).
export function parseEst(s, dayMinutes = 450) {
  const t = String(s ?? '').trim().toLowerCase();
  if (!t) return null;
  let total = 0;
  let any = false;
  for (const m of t.matchAll(/(\d+(?:[.,]\d+)?)\s*(w|d|h|m)?[a-z]*/g)) {
    const n = parseFloat(m[1].replace(',', '.'));
    any = true;
    total += m[2] === 'w' ? n * 5 * dayMinutes : m[2] === 'd' ? n * dayMinutes : m[2] === 'm' ? n : n * 60;
  }
  return any ? Math.round(total) : null;
}

export function estPill(v, t) {
  if (t.milestone || t.epic) return null;
  if (t.work == null) return h('span', { class: 'est missing', title: 'No estimate yet, so it isn’t on the Timeline' }, 'No estimate');
  return h('span', { class: 'est', title: 'Estimate' }, estText(t.work, v.settings.day_minutes));
}

/// What each view's toolbar offers: Create, the search and the filters
/// belong where the issues are a list (the Backlog); the Board only has
/// Only my issues; the Timeline and People show everything.
export const OFFERS = {
  board: ['mine'],
  backlog: ['create', 'search', 'mine', 'people', 'labels', 'only'],
};
export const offers = (tab, what) => !!OFFERS[tab?.viewName]?.includes(what);

/// Backlog › Filters: issues that still need something.
export const ONLY = {
  unassigned: { label: 'Unassigned', test: (t) => !t.who && !t.epic && !t.mirror && !t.is_done },
  noest: { label: 'No estimate', test: (t) => t.work == null && !t.epic && !t.mirror && !t.milestone && !t.is_done },
  late: { label: 'Late (after its due date)', test: (t) => !!t.late && !t.is_done },
  fields: { label: 'Required fields empty', test: (t) => !t.epic && !t.mirror && t.missing.some((m) => m !== 'estimate') },
};

/// The toolbar's search, faces and Only mine (those this view offers: one
/// set in another view doesn't hide anything here).
export function shown(app, t) {
  // Released: gone out, off the Board, the Backlog and the Timeline.
  if (t.release) return false;
  const all = app.tab.filter || {};
  const f = Object.fromEntries(Object.entries(all).filter(([k]) => offers(app.tab, k === 'text' ? 'search' : k)));
  if (f.text) {
    const q = f.text.toLowerCase();
    if (!`${t.key} ${t.title} ${t.labels.join(' ')}`.toLowerCase().includes(q)) return false;
  }
  if (f.mine && (!app.view.me_resource || (t.who !== app.view.me_resource && t.by !== app.view.me_resource))) return false;
  if (f.only?.length && !f.only.every((k) => ONLY[k]?.test(t))) return false;
  if (f.labels?.length && !f.labels.some((l) => t.labels.some((x) => x.toLowerCase() === l.toLowerCase()))) return false;
  if (f.people?.length && !f.people.includes(t.who || '') && !(t.by && f.people.includes(t.by))) return false;
  return true;
}

export const keyClass = (t) => `key${t.is_done ? ' done' : ''}`;

/// A due date: "Add a due date" until there is one (an empty date box would
/// show today's date in some web views), then the date and a × to clear it.
export function dueControl({ value, id, onChange, inputClass = '' }) {
  const box = h('div', { class: 'due-box' });
  const show = (v, focus) => {
    if (!v && !focus) {
      box.replaceChildren(h('button', { class: 'due-add', type: 'button', id, onClick: () => show('', true) }, 'Add a due date'));
      return;
    }
    const input = h('input', { type: 'date', class: inputClass, id, value: v || '', 'aria-label': 'Due date' });
    input.addEventListener('change', () => onChange(input.value || null));
    input.addEventListener('blur', () => { if (!input.value && !v) setTimeout(() => { if (!box.contains(document.activeElement)) show(null); }, 150); });
    box.replaceChildren(input, h('button', { class: 'icon-btn', type: 'button', 'aria-label': 'No due date', title: 'No due date', html: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75"><path d="M7 7l10 10M17 7 7 17"/></svg>',
      onClick: () => { onChange(null); show(null); } }));
    if (focus) input.focus();
  };
  show(value, false);
  box.reset = () => show(null);
  return box;
}

/// A description's checklist: how many items, and how many are ticked.
export function checks(html) {
  if (!html || !html.includes('checks')) return { all: 0, done: 0 };
  const t = document.createElement('template');
  t.innerHTML = html;
  const items = [...t.content.querySelectorAll('ul.checks > li')];
  return { all: items.length, done: items.filter((li) => li.hasAttribute('data-done')).length };
}

export function checksPill(t) {
  const c = checks(t.description);
  if (!c.all) return null;
  return h('span', { class: `checks-pill${c.done === c.all ? ' all' : ''}`, title: `${c.done} of ${c.all} checklist items done`,
    html: `<svg aria-hidden="true" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"><rect x="2" y="2" width="12" height="12" rx="2.5"/><path d="M5 8.2l2 2 4-4.4"/></svg>${c.done}/${c.all}` });
}

/// The start of a description, as plain words (cards and backlog rows).
export function snippet(html, n = 140) {
  if (!html) return '';
  const t = document.createElement('template');
  t.innerHTML = html;
  // The words people wrote as text first (not headings or checklist items, which show as their own counts).
  const prose = [...t.content.querySelectorAll('p, blockquote, ul:not(.checks) > li, ol > li')].map((e) => e.textContent).join(' ');
  const s = (prose.trim() ? prose : t.content.textContent).replace(/\s+/g, ' ').trim();
  return s.length > n ? `${s.slice(0, n - 1).trimEnd()}…` : s;
}

