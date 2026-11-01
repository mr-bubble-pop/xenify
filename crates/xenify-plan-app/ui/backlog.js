// Backlog: every issue in priority order, grouped by epic (or by label, or
// not at all). Drag a row up or down to change the order (the Timeline does
// what's higher first), or onto another group's heading to move it there.
// Type at the foot of a group to add an issue to it.

import { h, icon, toast, ask } from './ui.js';
import { typeIcon, prioIcon, assignee, may, lozenge, estPill, estText, epicColour, shown, keyClass, snippet, checksPill } from './pieces.js';
import { labelChips, labelChip, labelColour } from './labels.js';
import { fieldShow } from './fields.js';
import { quickAdd } from './create.js';
import { issueMenu } from './remove.js';
import { mirrorIcon, mirrorWhat } from './timeline.js';
import { dayName } from './fmt.js';

const DRAG = 'text/x-plan-task';

export function tools(app, v, tab) {
  const by = tab.groupBy || 'epic';
  return [h('span', { class: 'f-hint' }, 'Group by'), h('div', { class: 'segs', role: 'group', 'aria-label': 'Group by' },
    [['epic', 'Epic'], ['label', 'Label'], ['none', 'Nothing']].map(([k, label]) => h('button', { class: 'seg', 'aria-pressed': String(by === k), onClick: () => { tab.groupBy = k; app.render(); } }, label)))];
}

export function render(main, v, tab, app) {
  const by = tab.groupBy || 'epic';
  const wrap = h('div', { class: 'backlog' });
  const issues = v.tasks.filter((t) => !t.epic && !t.release);
  // Groups: { id, title, head (extra bits), all (every issue in it), drop (an issue dropped on the heading), add (quick-add preset) }
  let groups;
  if (by === 'label') {
    groups = [...v.labels.map((l) => ({ id: `l:${l.name}`, label: l.name, all: issues.filter((t) => t.labels.some((x) => x.toLowerCase() === l.name.toLowerCase())), add: { labels: [l.name] } })),
      { id: 'l:', label: null, all: issues.filter((t) => !t.labels.length), add: {} }];
  } else if (by === 'none') {
    groups = [{ id: 'all', all: issues, add: {} }];
  } else {
    groups = [...v.tasks.filter((t) => t.epic).map((e) => ({ id: e.id, epic: e, all: issues.filter((t) => t.parent === e.id), add: { parent: e.id } })),
      { id: null, epic: null, all: issues.filter((t) => !t.parent), add: { parent: null } }];
  }
  for (const g of groups) {
    const list = g.all.filter((t) => shown(app, t));
    const last = g === groups[groups.length - 1];
    if (last && by !== 'none' && !list.length && groups.length > 1) continue;
    wrap.append(group(v, tab, app, g, list, by));
  }
  if (by === 'epic') wrap.append(h('div', { class: 'bl-end' }, quickAdd(app, { kind: 'epic', parent: null }, 'Create epic')));
  if (v.releases.length) wrap.append(released(v, tab, app));
  main.replaceChildren(wrap);
}

/// Released: each release (newest first) and what went out in it, folded.
function released(v, tab, app) {
  const open = (tab.openReleases ||= new Set());
  return h('section', { class: 'bl-released', 'aria-label': 'Released' }, h('h3', {}, 'Released'),
    [...v.releases].reverse().map((r) => {
      const out = v.tasks.filter((t) => t.release === r.id);
      const on = open.has(r.id);
      return h('div', { class: 'rel-row' },
        h('button', { class: 'rel-head', 'aria-expanded': String(on), onClick: () => { if (on) open.delete(r.id); else open.add(r.id); app.render(); } },
          h('span', { class: 'twisty-i', html: icon('down', 14) }), h('b', {}, r.name),
          h('span', { class: 'f-hint' }, `${out.length} ${out.length === 1 ? 'issue' : 'issues'}${r.date ? ` · ${short(r.date)}` : ''}`)),
        h('button', { class: 'linkish small', onClick: async () => { const n = (await ask('Rename the release', 'Name', r.name, 'Rename'))?.trim(); if (n && n !== r.name) app.edit('release', { id: r.id, name: n }); } }, 'Rename'),
        on ? h('div', { class: 'rel-list' }, out.map((t) => h('button', { class: 'rel-issue', onClick: () => app.openTask(t.id) }, typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key), h('span', {}, t.title)))) : null);
    }));
}

function group(v, tab, app, g, list, by) {
  const day = v.settings.day_minutes;
  const all = g.all;
  // Mirrors aren't work here: not counted.
  const work = all.filter((t) => !t.mirror);
  const total = work.reduce((n, t) => n + (t.work || 0), 0);
  const done = all.filter((t) => t.is_done).reduce((n, t) => n + (t.work || 0), 0);
  const missing = work.filter((t) => t.work == null && !t.milestone && !t.is_done).length;
  const colour = g.epic ? epicColour(v, g.id) : g.label ? labelColour(g.label) : 'var(--series-3)';
  const name = g.epic ? g.epic.title || 'Untitled epic' : g.label ? g.label : by === 'label' ? 'No label' : by === 'none' ? 'All issues' : v.tasks.some((t) => t.epic) ? 'No epic' : 'Issues';
  const head = h('div', { class: 'bl-head' },
    g.epic ? typeIcon(v, 'epic', 16, colour) : null,
    g.epic ? h('button', { class: 'bl-title', onClick: () => app.openTask(g.id) }, name) : g.label ? labelChip(g.label) : h('span', { class: 'bl-title plain' }, name),
    g.epic ? h('span', { class: 'key' }, g.epic.key) : null,
    g.epic?.labels.length ? labelChips(g.epic.labels) : null,
    h('span', { class: 'bl-meta' },
      h('span', {}, `${work.length} ${work.length === 1 ? 'issue' : 'issues'}`),
      h('span', {}, `${estText(total, day) || '0h'} estimated`),
      missing ? h('span', { class: 'warn' }, `${missing} without an estimate`) : null,
      g.epic?.placed ? h('span', {}, `${short(g.epic.start)} – ${short(g.epic.finish)}`) : null,
      h('span', { class: 'bar100', title: `${total ? Math.round((done / total) * 100) : 0}% of the estimated work is done` }, h('i', { style: { width: `${total ? (done / total) * 100 : 0}%`, background: colour } }))));
  const box = h('section', { class: 'bl-group', 'aria-label': name }, head);
  if (g.epic) head.addEventListener('contextmenu', (e) => issueMenu(e, app, g.epic));
  // Dropped on the heading: into this group (an epic: after its last issue; a label: given it).
  if (by !== 'none') {
    head.addEventListener('dragover', (e) => { if (e.dataTransfer.types.includes(DRAG)) { e.preventDefault(); head.classList.add('over'); } });
    head.addEventListener('dragleave', () => head.classList.remove('over'));
    head.addEventListener('drop', (e) => {
      e.preventDefault();
      head.classList.remove('over');
      const id = e.dataTransfer.getData(DRAG);
      const t = v.tasks.find((x) => x.id === id);
      if (!t) return;
      if (by === 'label') {
        if (!g.label) { app.edit('set_labels', { id, labels: [] }); toast(`${t.key} has no labels now`, ['Undo', () => app.edit('undo')]); return; }
        if (t.labels.some((x) => x.toLowerCase() === g.label.toLowerCase())) return;
        app.edit('set_labels', { id, labels: [...t.labels, g.label] });
        toast(`${t.key} is labelled ${g.label}`);
        return;
      }
      if ((t.parent ?? null) === g.id) return;
      // Its rank (priority) stays: only its epic changes.
      app.edit('set_epic', { id, parent: g.id });
      toast(`${t.key} moved to ${g.epic ? g.epic.title : 'no epic'}`);
    });
  }
  for (const t of list) box.append(row(v, t, tab, app, by));
  box.append(h('div', { class: 'bl-foot' }, quickAdd(app, g.add)));
  return box;
}

const short = (s) => { const [, m, d] = s.split('-').map(Number); return `${d} ${['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][m - 1]}`; };

function row(v, t, tab, app, by) {
  const inList = v.fields.filter((f) => f.in_list && t.fields?.[f.id] != null && (!f.types?.length || f.types.includes(t.kind)));
  const snip = snippet(t.description, 160);
  const r = h('div', { class: `bl-row${tab.flash === t.id ? ' flash' : ''}${tab.selected === t.id ? ' sel' : ''}`, draggable: String(may(v, 'status')), tabindex: '0', role: 'button', 'aria-label': `${t.key} ${t.title}`, 'data-id': t.id },
    h('span', { class: 'grip', 'aria-hidden': 'true', html: '<svg width="10" height="14" viewBox="0 0 10 14"><g fill="currentColor"><circle cx="3" cy="3" r="1.2"/><circle cx="7" cy="3" r="1.2"/><circle cx="3" cy="7" r="1.2"/><circle cx="7" cy="7" r="1.2"/><circle cx="3" cy="11" r="1.2"/><circle cx="7" cy="11" r="1.2"/></g></svg>' }),
    t.mirror ? mirrorIcon() : typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key),
    h('span', { class: 'sum' }, h('span', { class: 't' }, t.title || h('span', { class: 'faint' }, 'No summary')),
      by !== 'label' ? labelChips(t.labels) : null,
      snip ? h('span', { class: 'snip' }, snip) : null),
    h('span', { class: 'xtra' }, inList.map((f) => h('span', { title: f.name }, fieldShow(v, f, t.fields[f.id]))), checksPill(t)),
    h('span', { class: 'cell' }, t.late ? h('span', { class: 'flag late', title: `Finishes ${t.late_by} working days after its due date` }, 'Late') : null),
    ...(t.mirror
      ? [h('span', { class: 'cell mirror-when', title: `Mirrors ${mirrorWhat(v, t)}` }, t.done >= 100 ? 'Done' : t.placed ? `${dayName(t.start)} – ${dayName(t.finish)}` : '—')]
      : [h('span', { class: 'cell' }, lozenge(v, t.status)), h('span', { class: 'cell' }, estPill(v, t)), prioIcon(t.priority), assignee(v, t)]));
  r.addEventListener('click', () => app.openTask(t.id));
  r.addEventListener('contextmenu', (e) => { tab.selected = t.id; issueMenu(e, app, t); });
  r.addEventListener('keydown', (e) => { if (e.key === 'Enter') app.openTask(t.id); });
  r.addEventListener('focus', () => { tab.selected = t.id; });
  r.addEventListener('dragstart', (e) => { e.dataTransfer.setData(DRAG, t.id); e.dataTransfer.effectAllowed = 'move'; r.classList.add('dragging'); });
  r.addEventListener('dragend', () => r.classList.remove('dragging'));
  r.addEventListener('dragover', (e) => { if (e.dataTransfer.types.includes(DRAG)) { e.preventDefault(); r.classList.add('drop-above'); } });
  r.addEventListener('dragleave', () => r.classList.remove('drop-above'));
  r.addEventListener('drop', (e) => {
    e.preventDefault();
    e.stopPropagation();
    r.classList.remove('drop-above');
    const id = e.dataTransfer.getData(DRAG);
    const m = v.tasks.find((x) => x.id === id);
    if (!m || m.id === t.id) return;
    // Grouped by epic, a row dropped here joins this row's epic; otherwise it keeps its own.
    app.edit('place', { id, before: t.id, parent: by === 'epic' ? t.parent ?? null : m.parent ?? null });
    toast(`${m.key} now comes before ${t.key}: the Timeline does it first`);
  });
  return r;
}
