// One issue, open over the view: what it is on the left (summary,
// description, the issues in an epic, links, comments and its history), its
// details on the right (status, who, priority, estimate, epic, due date, and
// when the Timeline has it). Every change is made as you leave a field.

import { h, icon, menu, item, sheet, toast, modal } from './ui.js';
import { dayName, when, plural } from './fmt.js';
import { oneLine, richText, showHtml } from './text.js';
import { typeIcon, prioIcon, avatar, assignee, whoChoices, group, may, lock, lozenge, estText, parseEst, PRIORITIES, statusOf, epicColour, keyClass, dueControl } from './pieces.js';
import { pickButton, quickAdd } from './create.js';
import { labelEditor } from './labels.js';
import { fieldControl, fieldsFor, fieldsDialog } from './fields.js';
import { serverPlans, issuesOf, fetchOutside, openPlanAt, openRef, nodeOfTab, wouldLoop, issueLink } from './serverplan.js';
import { call } from './engine.js';
import { deleteItem } from './remove.js';
import { mirrorIcon, mirrorWhat } from './timeline.js';

const isOutside = (id) => /^n\d+:./.test(id || '');

let open = null; // { id, close, box, tab: 'comments' | 'history', draft }

export function openTask(app, id) {
  if (open?.id === id) return;
  if (open) { open.id = id; open.tab = 'comments'; draw(app); return; }
  const box = h('div', { class: 'dlg issue', role: 'dialog', 'aria-modal': 'true' });
  open = { id, box, tab: 'comments', dirty: false };
  open.close = sheet(box, () => { open = null; app.tab.selected = id; });
  box.addEventListener('focusout', () => setTimeout(() => { if (open?.dirty && !editing()) draw(app); }, 0));
  draw(app);
  setTimeout(() => box.querySelector('.is-close')?.focus({ preventScroll: true }), 20);
}

export function closeTask() {
  open?.close();
  open = null;
}

/// Called after every change: drawn again, unless something in it is being typed in.
export function renderTask(app) {
  if (!open) return;
  if (editing()) { open.dirty = true; return; }
  draw(app);
}

const editing = () => {
  const a = document.activeElement;
  return !!(open && a && open.box.contains(a) && (a.isContentEditable || (a.tagName === 'INPUT' && a.type !== 'checkbox')));
};

function draw(app) {
  const v = app.view;
  const t = v.tasks.find((x) => x.id === open.id);
  if (!t) { closeTask(); return; }
  open.dirty = false;
  const focusId = document.activeElement?.id;
  const box = open.box;
  box.setAttribute('aria-label', `${t.key} ${t.title}`);
  const day = v.settings.day_minutes;
  const epic = t.parent ? v.tasks.find((x) => x.id === t.parent) : null;

  const head = h('header', { class: 'is-head' },
    h('div', { class: 'crumb' },
      epic ? [typeIcon(v, 'epic', 16, epicColour(v, epic.id)), h('button', { class: 'crumb-link', onClick: () => app.openTask(epic.id) }, epic.title || epic.key), h('span', { 'aria-hidden': 'true' }, '/')] : null,
      t.mirror ? mirrorIcon() : typeIcon(v, t.kind), h('span', { class: 'key' }, t.key)),
    h('span', { class: 'grow' }),
    // A link to this issue, to send to someone (it opens the plan on the server, at this issue).
    issueLink(app.tab.id, t.key) ? h('button', { class: 'btn plain small', title: 'Copy a link to this issue: whoever opens it (and may see this plan) goes straight to it', onClick: () => copyLink(app, t) }, 'Copy link') : null,
    h('button', { class: 'icon-btn', 'aria-label': 'More', title: 'More', html: icon('more', 18), onClick: (e) => more(e, app, t) }),
    h('button', { class: 'icon-btn is-close', 'aria-label': 'Close (Esc)', title: 'Close (Esc)', html: icon('close', 16), onClick: () => closeTask() }));

  // ----- Left: the work -----
  const left = h('div', { class: 'is-main' });
  // Made with just a summary? What's still to fill in, and the next one like it.
  if (t.missing.length && !t.epic) {
    const goTo = (what) => () => {
      const f = v.fields.find((x) => x.name === what);
      const el = what === 'description' ? box.querySelector('.is-main .rte-text') : what === 'estimate' ? box.querySelector('#is-est')
        : f && box.querySelector(`#is-f-${f.id}`)?.querySelector('input, select, textarea, button');
      el?.focus();
      el?.scrollIntoView({ block: 'nearest' });
    };
    const next = nextNeedy(app, t.id);
    left.append(h('div', { class: 'is-needs', role: 'note' },
      h('span', { class: 'needs', html: '<svg aria-hidden="true" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M10.5 2.5l3 3L6 13H3v-3z"/></svg>' }),
      h('span', {}, 'Still to fill in: ', ...t.missing.flatMap((m, i) => [i ? ', ' : '', h('button', { class: 'gap', onClick: goTo(m) }, m)])),
      h('span', { class: 'grow' }),
      next ? h('button', { class: 'btn plain small', title: `${next.key} ${next.title}`, onClick: () => app.openTask(next.id) }, 'Next to fill in →') : null));
  }
  left.append(oneLine({ text: t.title, label: 'Summary', placeholder: 'What needs doing?', className: 'is-title',
    onCommit: (s) => { if (s) app.edit('set_task', { id: t.id, field: 'title', value: s }); else toast('An issue needs a summary'); } }));
  if (t.mirror) {
    left.append(h('div', { class: 'whole-note', role: 'note' }, mirrorIcon(),
      h('span', { class: 'grow' }, `Mirrors ${mirrorWhat(v, t)}: it shows when that’s done, with its dates, and moves nothing.`),
      h('button', { class: 'btn small', onClick: () => openRef(t.mirror) }, 'Go there')));
  }
  left.append(h('h4', {}, 'Description'));
  left.append(richText({ html: t.description, label: 'Description', placeholder: 'Add a description: what it is, and what “done” means. Markdown works: # heading, - list, [] checklist',
    onSave: (html) => app.edit('set_task', { id: t.id, field: 'description', value: html }) }).el);

  if (t.epic) {
    const kids = v.tasks.filter((x) => x.parent === t.id);
    const done = kids.filter((x) => x.is_done).length;
    left.append(h('h4', {}, `Issues in this epic`, h('span', { class: 'h-n' }, `${done} of ${kids.length} done`)));
    left.append(h('div', { class: 'links' }, kids.map((k) => linkRow(v, app, k, null)), quickAdd(app, { parent: t.id })));
  }

  left.append(h('h4', {}, 'Linked issues'));
  const by = new Map(v.tasks.map((x) => [x.id, x]));
  const waits = t.after.map((d) => by.get(d.task)).filter(Boolean);
  const away = t.after.filter((d) => isOutside(d.task)).map((d) => d.task);
  const holds = (t.blocks || []).map((id) => by.get(id)).filter(Boolean);
  const rel = (t.relates || []).map((id) => by.get(id)).filter(Boolean);
  const links = h('div', { class: 'links' });
  if (waits.length || away.length) links.append(h('div', { class: 'rel' }, 'Starts after'), ...waits.map((w) => linkRow(v, app, w, () => app.edit('unlink', { from: w.id, to: t.id }))),
    ...away.map((ref) => outsideRow(v, ref, () => app.edit('unlink', { from: ref, to: t.id }))));
  // Also what waits for it in other plans (the server says; the link is theirs to remove).
  const held = (app.tab.waiting?.[t.id] || []).filter((o) => o.ref);
  if (holds.length || held.length) links.append(h('div', { class: 'rel' }, 'Must finish before'), ...holds.map((b) => linkRow(v, app, b, () => app.edit('unlink', { from: t.id, to: b.id }))),
    ...held.map((o) => outsideRow(v, o.ref, null, o)));
  if (rel.length) links.append(h('div', { class: 'rel' }, 'Relates to'), ...rel.map((r) => linkRow(v, app, r, () => app.edit('relate', { a: t.id, b: r.id, on: false }))));
  if (!waits.length && !away.length && !holds.length && !held.length && !rel.length) links.append(h('p', { class: 'f-hint' }, t.mirror ? 'Pick “This must finish before…” to make issues here wait until it’s done.' : t.epic ? 'Link an epic and everything in it starts after (or must finish before) the other issue.' : 'Nothing linked. Link what has to be done first, and the Timeline starts this after it.'));
  const kinds = h('select', { class: 'field small', 'aria-label': 'Kind of link', id: 'is-link-kind' },
    // A mirror can't start after anything (it only shows the other one's dates).
    t.mirror ? null : h('option', { value: 'waits' }, 'This starts after…'), h('option', { value: 'holds' }, 'This must finish before…'), h('option', { value: 'relates' }, 'Relates to'));
  const taken = [t.id, ...waits.map((x) => x.id), ...holds.map((x) => x.id), ...rel.map((x) => x.id)];
  links.append(h('div', { class: 'link-add' }, kinds, pickButton(app, '+ Link an issue', taken, async (other) => {
    const k = kinds.value;
    // This plan, even if another tab is in front once the server answers.
    const tab = app.tab.id;
    // On a server, a loop can also run through other plans.
    const [a, b] = k === 'waits' ? [other, t.id] : [t.id, other];
    if (k !== 'relates' && v.on_server) {
      const chain = await wouldLoop(tab, a, b);
      if (chain) { showLoop(app, chain, by.get(b)); return; }
    }
    if (k === 'waits') app.edit('link', { tab, from: other, to: t.id });
    else if (k === 'holds') app.edit('link', { tab, from: t.id, to: other });
    else app.edit('relate', { tab, a: t.id, b: other, on: true });
  }, () => true), v.on_server && !t.mirror ? h('button', { class: 'btn plain small', type: 'button', onClick: () => pickOutside(app, t) }, '+ Starts after another plan, or an issue in it…') : null));
  left.append(links);

  // Activity: comments, and every change to this issue.
  left.append(h('h4', {}, 'Activity'));
  // History (who changed what) only for plans on a server: a plan kept as a file has no audit.
  if (!v.audit) open.tab = 'comments';
  const tabs = h('div', { class: 'acttabs', role: 'tablist' }, [['comments', `Comments${t.comments.length ? ` (${t.comments.length})` : ''}`], v.audit ? ['history', 'History'] : null].filter(Boolean).map(([id, label]) =>
    h('button', { role: 'tab', 'aria-selected': String(open.tab === id), class: open.tab === id ? 'on' : '', onClick: () => { open.tab = id; draw(app); } }, label)));
  left.append(tabs);
  if (open.tab === 'comments') {
    const box2 = richText({ label: 'Add a comment', placeholder: 'Add a comment…', compact: true });
    const send = () => {
      const html = box2.html;
      if (!html) return;
      box2.html = '';
      // Drawn again now: the box still has the focus, which otherwise holds the redraw back.
      app.edit('add_comment', { id: t.id, html });
      draw(app);
    };
    box2.editor.addEventListener('keydown', (e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); send(); } });
    left.append(h('div', { class: 'comment-new' }, avatar(v, v.me_resource || null, 30), h('div', { class: 'comment-col' }, box2.el,
      h('div', { class: 'comment-actions' }, h('button', { class: 'btn main small', onClick: send }, 'Save'), h('span', { class: 'f-hint' }, 'Ctrl+Enter')))));
    for (const c of [...t.comments].reverse()) left.append(comment(v, app, t, c));
  } else {
    const list = app.read('history', { task: t.id }) || [];
    left.append(h('div', { class: 'hist' }, list.length ? list.slice(0, 60).map((e) => h('div', { class: 'hist-row' },
      h('b', {}, e.who || 'Someone'), h('span', {}, e.what), h('span', { class: 'when' }, when(e.at)))) : h('p', { class: 'f-hint' }, 'No changes yet.')));
  }

  // ----- Right: details -----
  const sel = (id, label, options, value, set) => {
    const s = h('select', { id, 'aria-label': label }, options.map(([k2, l]) => h('option', { value: k2 ?? '', selected: (k2 ?? '') === (value ?? '') }, l)));
    s.addEventListener('change', () => set(s.value || null));
    return s;
  };
  const row = (label, ...control) => h('div', { class: 'dl' }, h('span', { class: 'dl-k' }, label), h('div', { class: 'dl-v' }, ...control));
  const st = statusOf(v, t.status);
  const status = sel('is-status', 'Status', v.statuses.map((s) => [s.id, s.name]), t.status, (x) => app.edit('set_task', { id: t.id, field: 'status', value: x }));
  status.className = 'statusbtn';
  status.style.setProperty('--c', st.color || '#A8A29E');
  const est = h('input', { id: 'is-est', value: estText(t.work, day), placeholder: 'e.g. 2d 4h', 'aria-label': 'Estimate', autocomplete: 'off' });
  est.addEventListener('keydown', (e) => { if (e.key === 'Enter') est.blur(); });
  est.addEventListener('change', () => {
    const m = parseEst(est.value, day);
    if (est.value.trim() && m == null) { toast('Write the estimate like 3d, 4h or 1d 4h'); est.value = estText(t.work, day); return; }
    app.edit('set_task', { id: t.id, field: 'work', value: m });
  });
  const due = dueControl({ value: t.due, id: 'is-due', onChange: (x) => { if ((x || null) !== (t.due || null)) app.edit('set_task', { id: t.id, field: 'due', value: x }); } });
  const epics = v.tasks.filter((x) => x.epic && x.id !== t.id);
  const details = h('div', { class: 'details' }, h('div', { class: 'dh' }, 'Details'),
    !t.epic && !t.mirror ? row('Assignee', assignee(v, t, 22), sel('is-who', 'Assignee', whoChoices(v), t.who, (x) => app.edit('set_task', { id: t.id, field: 'who', value: x }))) : null,
    // Group work: who in the group is free first, and handing it to them.
    !t.epic && t.who_group ? h('div', { class: 'f-hint group-by' }, t.by_name ? `${t.by_name} is free first in ${group(v, t.who)?.name || 'the group'}. ` : `Nobody in ${group(v, t.who)?.name || 'the group'} has time yet. `,
      t.by_name ? h('button', { class: 'linkish', id: 'is-give', onClick: () => app.edit('set_task', { id: t.id, field: 'who', value: t.by }) }, `Give it to ${t.by_name}`) : null) : null,
    t.mirror ? null : row('Priority', prioIcon(t.priority), sel('is-prio', 'Priority', PRIORITIES.map(([k2, n]) => [k2, n]), t.priority, (x) => app.edit('set_task', { id: t.id, field: 'priority', value: x || 'medium' }))),
    !t.epic && !t.milestone && !t.mirror ? row('Estimate', est) : null,
    !t.epic ? row('Epic', t.parent ? h('i', { class: 'dot', style: { background: epicColour(v, t.parent) } }) : null, sel('is-epic', 'Epic', [[null, 'None'], ...epics.map((e) => [e.id, e.title || e.key])], t.parent, (x) => app.edit('set_epic', { id: t.id, parent: x }))) : null,
    !t.epic && !t.mirror ? row('Type', sel('is-type', 'Issue type', v.types.filter((x) => x.id !== 'epic').map((x) => [x.id, x.name]), t.kind, (x) => app.edit('set_tasks', { what: `${t.title}: ${v.types.find((y) => y.id === x)?.name}`, changes: [{ id: t.id, field: 'kind', value: x }, { id: t.id, field: 'milestone', value: x === 'milestone' }] }))) : null,
    row('Due date', due),
    row('Labels', labelEditor(app, t.labels, (l) => app.edit('set_labels', { id: t.id, labels: l }), 'is-labels')),
    ...fieldsFor(v, t).map((f) => row(h('span', { title: f.help || null }, f.name, f.required ? h('span', { class: 'req', title: 'Needed' }, ' *') : null),
      h('div', { id: `is-f-${f.id}`, class: 'fwrap' }, fieldControl(v, f, t.fields?.[f.id], (x) => {
        try { app.attempt('set_field', { id: t.id, field: f.id, value: x }); return null; } catch (e) { return e.message; }
      })))),
    row('When', schedule(v, app, t)),
    h('button', { class: 'linkish small add-field', onClick: () => fieldsDialog(app) }, v.fields.length ? 'Fields…' : '+ Add a field of your own'));
  const made = v.audit && t.created ? `Created ${when(t.created)}${t.created_by ? ` by ${t.created_by}` : ''}` : '';
  const side = h('div', { class: 'is-side' },
    !t.epic && !t.mirror ? status : null,
    details,
    h('div', { class: 'meta' }, made ? h('span', {}, made) : null, v.audit && t.updated && t.updated !== t.created ? h('span', {}, `Updated ${when(t.updated)}`) : null));

  // Only what my access to the plan allows.
  if (v.may) {
    const gate = (what, ...sels) => { if (!may(v, what)) for (const s of sels) for (const el of [...left.querySelectorAll(s), ...side.querySelectorAll(s)]) lock(el.closest('.dl') || el); };
    gate('edit', '.is-title', '.is-main > .rte', '#is-prio', '#is-epic', '#is-type', '#is-due', '#is-labels', '.fwrap', '.add-field');
    gate('status', '#is-status');
    gate('assign', '#is-who', '#is-give');
    gate('schedule', '#is-est', '.links', '.sched');
    if (!may(v, 'comment')) for (const el of left.querySelectorAll('.comment-new')) el.remove();
  }
  box.replaceChildren(head, h('div', { class: 'is-cols' }, left, side));
  if (focusId) document.getElementById(focusId)?.focus({ preventScroll: true });
}

/// The next issue (in backlog order, after this one) still to fill in.
function nextNeedy(app, id) {
  const list = app.view.tasks.filter((x) => !x.epic && x.missing.length);
  const i = app.view.tasks.findIndex((x) => x.id === id);
  return list.find((x) => app.view.tasks.indexOf(x) > i) || list.find((x) => x.id !== id) || null;
}

function schedule(v, app, t) {
  const day = v.settings.day_minutes;
  const go = () => { closeTask(); app.tab.focusDay = t.start; app.tab.selected = t.id; app.setView('timeline'); };
  if (t.epic) {
    return h('div', { class: 'sched' }, t.placed ? `${dayName(t.start)} – ${dayName(t.finish)}` : 'Nothing estimated yet',
      h('span', { class: 'f-hint' }, `${estText(t.total, day) || '0h'} of work in it`), t.placed ? h('button', { class: 'linkish', onClick: go }, 'Show on the Timeline') : null);
  }
  if (t.is_done) return h('div', { class: 'sched' }, t.placed ? `Done ${dayName(t.finish)}` : 'Done');
  if (!t.placed) {
    return h('div', { class: 'sched' }, h('span', { class: 'f-hint' }, t.milestone ? 'When what it starts after has an estimate' : 'Not on the Timeline until it has an estimate'));
  }
  return h('div', { class: 'sched' },
    t.milestone ? dayName(t.start) : t.start === t.finish ? dayName(t.start) : `${dayName(t.start)} – ${dayName(t.finish)}`,
    t.pieces.length > 1 ? h('span', { class: 'f-hint' }, `In ${t.pieces.length} pieces, around time away`) : null,
    t.held ? h('span', { class: 'f-hint' }, `Not before ${dayName(t.held)} (dragged there) · `, h('button', { class: 'linkish', onClick: () => app.edit('hold', { id: t.id }) }, 'start sooner')) : null,
    t.waits.length ? h('span', { class: 'warnline' }, `Also starts after ${plural(t.waits.length, 'issue')} without an estimate`) : null,
    t.late ? h('span', { class: 'lateline' }, `${plural(t.late_by, 'working day')} after the due date`) : null,
    ...t.warnings.map((w) => h('span', { class: 'warnline' }, w)),
    ...clashLines(app, t),
    h('button', { class: 'linkish', onClick: go }, 'Show on the Timeline'));
}

/// Its person booked in plans made before this one: it waits for them,
/// unless set to go ahead anyway (then both plans warn: over-booked).
function clashLines(app, t) {
  const waited = t.clashes.filter((c) => !c.over);
  if (t.overbook) {
    return [h('span', { class: 'f-hint' }, 'Set to go ahead even where they’re booked in other plans · ',
      h('button', { class: 'linkish sched-wait', onClick: () => app.edit('set_task', { id: t.id, field: 'overbook', value: false, what: `${t.title}: waits for other plans again` }) }, 'wait for them'))];
  }
  if (!waited.length) return [];
  const c = waited[0];
  const where = c.plan ? `“${c.plan}”` : 'another plan';
  const line = h('span', { class: 'f-hint clashline' },
    `Waits for ${c.who}: booked in ${where}${c.title ? ` (${c.title})` : ''} until ${dayName(c.to)}${waited.length > 1 ? `, and in ${plural(waited.length - 1, 'other issue')}` : ''} · `,
    h('button', { class: 'linkish sched-over', onClick: async () => {
      const yes = await modal(`Over-book ${c.who}?`, h('p', {}, `${c.who} is already booked in ${where} then. Going ahead anyway puts both on ${c.who} at once, and both plans show a warning.`),
        [['Cancel', null], ['Over-book', true, true]]);
      if (yes) app.edit('set_task', { id: t.id, field: 'overbook', value: true, what: `${t.title}: goes ahead over other plans` });
    } }, 'over-book anyway'));
  return [line];
}

/// An issue in another plan that this one waits for: its key and dates; its
/// summary and plan only for someone who may open that plan.
function outsideRow(v, ref, remove, known = null, why = '') {
  const o = known ? { ...known, known: true } : v.outside?.[ref] || {};
  const [n, task] = ref.slice(1).split(':');
  if (task === '*') {
    // A whole plan: its name (shown to anyone, as for a plan you can't open) and its dates.
    const when = o.placed && o.finish ? `${o.start ? `${dayName(o.start)} – ` : ''}${dayName(o.finish)}` : o.done ? 'Done' : 'Nothing scheduled yet';
    const r = h('div', { class: `linkrow outside${o.readable ? '' : ' locked'}`, role: o.readable ? 'button' : null, tabindex: o.readable ? '0' : null,
      title: why || (o.readable ? 'The whole plan, start to finish' : `You can’t open “${o.plan}”: only its dates are shown. ${o.ask ? `Ask ${o.ask} for access.` : ''}`) },
      mirrorIcon(), h('span', { class: 'key' }, 'Plan'),
      h('span', { class: 'sum' }, !o.known ? 'Asking the server…' : o.gone ? 'Not there any more' : `“${o.plan || o.title || 'A plan'}”`, h('span', { class: 'snip' }, why ? ` · ${why}` : ' · all of it')),
      h('span', { class: 'when' }, when), o.done ? h('span', { class: 'lozenge', style: { '--c': '#1baf7a' } }, 'Done') : null,
      remove ? h('button', { class: 'rm', 'aria-label': 'Remove the link to that plan', title: 'Remove the link', html: icon('close', 13), onClick: (e) => { e.stopPropagation(); remove(); } }) : null);
    if (o.readable) r.addEventListener('click', () => openPlanAt(Number(n)));
    return r;
  }
  const when = o.placed && o.finish ? (o.milestone ? dayName(o.finish) : `${o.start ? `${dayName(o.start)} – ` : ''}${dayName(o.finish)}`) : o.done ? 'Done' : 'Not scheduled yet';
  const title = !o.known ? 'Asking the server…' : o.gone ? 'Not there any more (deleted, or its plan was)' : o.readable ? o.title || 'No summary' : 'No access';
  const r = h('div', { class: `linkrow outside${o.readable ? '' : ' locked'}`, role: o.readable ? 'button' : null, tabindex: o.readable ? '0' : null,
    title: o.readable ? `In ${o.plan}` : `You can’t open “${o.plan}”: only this issue’s key and dates are shown. ${o.ask ? `Ask ${o.ask} for access.` : ''}` },
    h('span', { class: 'ty out', 'aria-hidden': 'true', html: '<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="#fff" stroke-width="2"><path d="M6 3H3v10h10v-3M9 3h4v4M13 3L7 9"/></svg>' }),
    h('span', { class: 'key' }, o.key || `#${task}`),
    h('span', { class: 'sum' }, title, o.plan ? h('span', { class: 'snip' }, o.readable ? ` · ${o.plan}` : ` to “${o.plan}”${o.ask ? `: ask ${o.ask}` : ''}`) : null),
    h('span', { class: 'when' }, when), o.done ? h('span', { class: 'lozenge', style: { '--c': '#1baf7a' } }, 'Done') : null,
    remove ? h('button', { class: 'rm', 'aria-label': `Remove the link to ${o.key || 'it'}`, title: 'Remove the link', html: icon('close', 13), onClick: (e) => { e.stopPropagation(); remove(); } }) : null);
  if (o.readable) r.addEventListener('click', () => openPlanAt(Number(n), task));
  return r;
}

/// Picks a whole plan, an epic or an issue in another plan on the server you
/// can open: { ref ("n12:*" or "n12:t3"), picked, plan (its name), tab }, or null.
async function pickOnServer(app, { title, label, hint, ok }) {
  // This plan, even if another tab is in front by the time it's picked.
  const tab = app.tab.id;
  let plans;
  try { plans = (await serverPlans()).filter((p) => p.node !== nodeOfTab(tab)); } catch (e) { toast(String(e.message || e)); return null; }
  if (!plans.length) { toast('There are no other plans on the server you can open'); return null; }
  const planSel = h('select', { class: 'field', 'aria-label': 'Plan' }, plans.map((p) => h('option', { value: p.node }, p.name || 'Untitled plan')));
  const q = h('input', { class: 'field', placeholder: 'Key or words', 'aria-label': 'Find an epic or issue' });
  const list = h('div', { class: 'pick-list tall', role: 'listbox' });
  let issues = [];
  let picked = null;
  const draw = () => {
    const w = q.value.trim().toLowerCase();
    const hits = issues.filter((i) => !w || `${i.key} ${i.title}`.toLowerCase().includes(w)).slice(0, 60);
    // First, the whole plan: start to finish of everything in it.
    const real = issues.filter((i) => i.epic !== '(epic)');
    const placed = real.filter((i) => i.placed && i.start);
    const all = { task: '*', key: '', title: planSel.selectedOptions[0]?.textContent || 'The plan', readable: true, placed: placed.length > 0,
      start: placed.map((i) => i.start).sort()[0] || null, finish: placed.map((i) => i.finish).sort().pop() || null, done: real.length > 0 && real.every((i) => i.done) };
    const wholeBtn = !w && issues.length ? h('button', { type: 'button', class: `item${picked?.task === '*' ? ' active' : ''}`, role: 'option', onClick: () => { picked = all; draw(); } },
      mirrorIcon(), h('span', { class: 'pick-title' }, h('b', {}, 'The whole plan'), ' — done when everything in it is'), h('span', { class: 'muted' }, all.finish ? dayName(all.finish) : '')) : null;
    list.replaceChildren(...[wholeBtn].filter(Boolean), ...(hits.length ? hits.map((i) => h('button', { type: 'button', class: `item${picked?.task === i.task ? ' active' : ''}`, role: 'option', onClick: () => { picked = i; draw(); } },
      h('span', { class: 'key' }, i.key), h('span', { class: 'pick-title' }, i.title || 'No summary', i.epic === '(epic)' ? h('span', { class: 'muted' }, ' · epic') : null),
      h('span', { class: 'muted' }, i.placed && i.finish ? dayName(i.finish) : i.done ? 'Done' : ''))) : [h('div', { class: 'f-hint pad' }, issues.length ? 'Nothing matches' : 'Getting its issues…')]));
  };
  const load = async () => {
    issues = [];
    picked = null;
    draw();
    try { issues = (await issuesOf(Number(planSel.value))).issues; } catch (e) { toast(String(e.message || e)); }
    draw();
  };
  planSel.addEventListener('change', load);
  q.addEventListener('input', draw);
  load();
  const yes = await modal(title, h('div', { class: 'form' },
    h('label', {}, 'Plan', planSel), h('label', {}, label, q), list, h('p', { class: 'f-hint' }, hint)), [['Cancel', null], [ok, true, true]]);
  if (!yes || !picked) return null;
  const plan = planSel.selectedOptions[0].textContent;
  const ref = `n${planSel.value}:${picked.task}`;
  // What's known already, so it reads right straight away (the server says the rest).
  try {
    const known = Object.fromEntries(Object.entries(call('view', { tab }).outside || {}).filter(([, o]) => o.known));
    call('set_world', { tab, outside: { ...known, [ref]: { ...picked, key: picked.key || '', readable: true, plan } } });
  } catch (e) { toast(e.message); return null; }
  return { ref, picked, plan, tab };
}

/// "Starts after another plan, or an issue in it…".
async function pickOutside(app, t) {
  const got = await pickOnServer(app, { title: `${t.key} starts after another plan`, label: 'All of it, or one epic or issue', ok: 'Link',
    hint: 'It starts once that plan (or that issue) is finished, as the plan has it. People who can’t open that plan see only its name and dates.' });
  if (!got) return;
  const chain = await wouldLoop(got.tab, got.ref, t.id);
  if (chain) { showLoop(app, chain, t); return; }
  app.edit('link', { tab: got.tab, from: got.ref, to: t.id });
  fetchOutside(got.tab).then(() => app.render()).catch(() => {});
}

/// Issue › Mirror…: a task here that shows when a whole plan, an epic or an
/// issue elsewhere is done (its dates), and goes there when opened. It moves nothing.
export async function createMirror(app) {
  if (!app.view?.on_server) { toast('Mirrors show plans on the server: put this plan on the server first'); return; }
  const got = await pickOnServer(app, { title: 'Mirror another plan', label: 'All of it, or one epic or issue', ok: 'Mirror it',
    hint: 'A task appears here with its dates, kept up to date. It doesn’t move anything; link to it if something here has to wait for it.' });
  if (!got) return;
  const title = got.picked.task === '*' ? got.plan : got.picked.title || got.picked.key || 'Mirror';
  const out = app.edit('add_mirror', { tab: got.tab, ref: got.ref, title });
  if (out?.id) toast(`Mirroring ${got.picked.task === '*' ? `all of “${got.plan}”` : `${got.picked.key} ${title}`}`, ['Undo', () => app.edit('undo')]);
  fetchOutside(got.tab).then(() => app.render()).catch(() => {});
}

/// "That would make a loop": every issue in it, in order, back to the one
/// being linked, so you can see where to break it (issues in plans you
/// can't open show their key, dates and plan).
function showLoop(app, chain, t) {
  const v = app.view;
  const here = nodeOfTab(app.tab.id);
  const rows = chain.map((o, i) => {
    const [n, task] = (o.ref || '').slice(1).split(':');
    const mine = Number(n) === here;
    const local = mine ? v.tasks.find((x) => x.id === task) : null;
    const whole = task === '*';
    const name = whole ? `All of “${o.plan || o.title || 'a plan'}”` : local ? local.title : o.readable ? o.title || 'No summary' : 'No access';
    const go = local ? () => { closeTask(); app.openTask(local.id); } : o.readable && !(whole && mine) ? () => openPlanAt(Number(n), whole ? undefined : task) : null;
    return h('li', { class: 'loop-step' },
      whole ? mirrorIcon() : h('span', { class: 'key' }, local?.key || o.key || '?'),
      h('span', { class: `t${o.readable || local ? '' : ' faint'}` }, name),
      h('span', { class: 'muted' }, whole ? (mine ? 'this plan' : '') : mine ? 'this plan' : o.plan ? `in ${o.plan}${!o.readable && o.ask ? ` (ask ${o.ask} for access)` : ''}` : ''),
      go ? h('button', { class: 'linkish small', onClick: go }, 'Open') : null,
      h('span', { class: 'arrow', 'aria-hidden': 'true' }, i < chain.length - 1 ? 'starts after ↓' : ''));
  });
  const first = chain[0] || {};
  const firstKey = first.ref?.endsWith(':*') ? `“${first.plan || first.title || 'that plan'}”` : first.key || 'it';
  modal('That would make a loop', h('div', { class: 'loop-box' },
    h('p', {}, `${t?.key || 'This issue'} can’t start after ${firstKey}: ${firstKey} already starts after ${t?.key || 'it'}, like this. Take one of these links out first, or don’t add this one.`),
    h('ol', { class: 'loop-list' }, rows)), [['OK', true, true]]);
}

function linkRow(v, app, x, remove) {
  const r = h('div', { class: 'linkrow', role: 'button', tabindex: '0' },
    typeIcon(v, x.kind), h('span', { class: keyClass(x) }, x.key), h('span', { class: 'sum' }, x.title), prioIcon(x.priority), assignee(v, x, 22), lozenge(v, x.status),
    remove ? h('button', { class: 'rm', 'aria-label': `Remove the link to ${x.key}`, title: 'Remove the link', html: icon('close', 13), onClick: (e) => { e.stopPropagation(); remove(); } }) : null);
  r.addEventListener('click', () => app.openTask(x.id));
  r.addEventListener('keydown', (e) => { if (e.key === 'Enter') app.openTask(x.id); });
  return r;
}

function comment(v, app, t, c) {
  // Only the author may change it; the author or whoever may edit the plan
  // may delete it (the server checks too).
  const mine = !!c.by && c.by === v.me_resource;
  const remove = mine || (may(v, 'edit') && !!v.me_resource);
  const body = h('div', { class: 'comment-body' }, showHtml(c.html));
  const edit = () => {
    const ed = richText({ html: c.html, label: 'Change the comment', compact: true });
    body.replaceChildren(ed.el, h('div', { class: 'comment-actions' },
      // Saved or not, the comment shows again as text (the issue is drawn again).
      h('button', { class: 'btn main small', onClick: () => { app.edit('set_comment', { id: t.id, comment: c.id, html: ed.html }); draw(app); } }, 'Save'),
      h('button', { class: 'btn plain small', onClick: () => draw(app) }, 'Cancel')));
    ed.focus();
  };
  // The name the server knows the author by, not what the comment says.
  const person = c.by ? v.resources.find((p) => p.id === c.by) : null;
  const name = person?.name || c.who;
  return h('div', { class: 'comment' }, avatar(v, person?.id || null, 30),
    h('div', { class: 'comment-col' },
      h('div', { class: 'comment-head' }, name ? h('b', {}, name) : null, h('span', { class: 'when' }, when(c.at)),
        mine ? h('button', { class: 'linkish small', onClick: edit }, 'Edit') : null,
        remove ? h('button', { class: 'linkish small', onClick: () => { app.edit('delete_comment', { id: t.id, comment: c.id }); toast('Comment deleted', ['Undo', () => app.edit('undo')]); } }, 'Delete') : null),
      body));
}

function copyLink(app, t) {
  const url = issueLink(app.tab.id, t.key);
  if (!url) return;
  navigator.clipboard?.writeText(url).then(() => toast(`Copied a link to ${t.key}`)).catch(() => toast(`Couldn’t copy: ${url}`));
}

function more(e, app, t) {
  const r = e.currentTarget.getBoundingClientRect();
  menu([
    item(`Copy “${t.key}”`, { run: () => { navigator.clipboard?.writeText(`${t.key} ${t.title}`).then(() => toast('Copied')).catch(() => toast('Couldn’t copy')); } }),
    issueLink(app.tab.id, t.key) ? item('Copy a link to it', { run: () => copyLink(app, t) }) : null,
    null,
    deleteItem(app, t),
  ], r.right - 240, r.bottom + 4);
}
