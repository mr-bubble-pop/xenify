// What AI can do with a plan, for Ask (in the app) and programs over MCP:
// look (overview, find, one issue, who is busy) and change (add issues,
// change fields, link, hold back, delete). Changes are listed first and made
// together as one step: one Undo, one line in History, passed on to anyone
// editing the plan together.

import { call } from './engine.js';
import { dayName, plural } from './fmt.js';
import { estText, parseEst } from './pieces.js';
import { plainText } from './text.js';

const lower = (s) => String(s ?? '').trim().toLowerCase();
const NOBODY = ['', 'nobody', 'no one', 'noone', 'none', 'unassigned'];

export function taskOf(v, ref) {
  const r = lower(ref);
  if (!r) throw new Error('Say which issue (its key, like WEB-6, or its summary)');
  const t = v.tasks.find((x) => x.id === ref) || v.tasks.find((x) => lower(x.key) === r) || v.tasks.find((x) => lower(x.title) === r)
    || v.tasks.find((x) => lower(x.title).includes(r));
  if (!t) throw new Error(`No issue called “${ref}”`);
  return t;
}

export function resourceOf(v, ref) {
  const r = lower(ref);
  const x = v.resources.find((p) => p.id === ref) || v.resources.find((p) => lower(p.name) === r) || v.resources.find((p) => lower(p.name).split(' ')[0] === r)
    || v.resources.find((p) => lower(p.name).includes(r));
  if (!x) throw new Error(`Nobody called “${ref}” in this plan. People: ${v.resources.map((p) => p.name).join(', ') || 'none yet'}`);
  return x;
}

const statusName = (v, id) => v.statuses.find((s) => s.id === id)?.name;
const typeName = (v, id) => v.types.find((s) => s.id === id)?.name;
const when = (t) => (t.placed ? (t.milestone || t.start === t.finish ? dayName(t.start) : `${dayName(t.start)} – ${dayName(t.finish)}`) : 'not on the Timeline (no estimate)');

const brief = (v, t) => ({
  key: t.key, summary: t.title, type: typeName(v, t.kind), status: statusName(v, t.status),
  epic: t.parent ? v.tasks.find((x) => x.id === t.parent)?.title : null, who: t.who_name || null, priority: t.priority,
  estimate: t.epic ? `${estText(t.total, v.settings.day_minutes)} in its issues` : t.milestone ? null : estText(t.work, v.settings.day_minutes) || null,
  when: when(t), due: t.due, late_by_working_days: t.late ? t.late_by : 0, held_until: t.held, done: t.is_done,
  waits_for: t.after.map((d) => v.tasks.find((x) => x.id === d.task)?.key).filter(Boolean),
  waits_for_unestimated: t.waits.map((id) => v.tasks.find((x) => x.id === id)?.key).filter(Boolean),
  warnings: t.warnings,
});

// ----- Looking -----

export function overview(tab) {
  const v = call('view', { tab });
  const day = v.settings.day_minutes;
  const issues = v.tasks.filter((t) => !t.epic);
  return {
    plan: v.title, today: v.today, hours_in_a_day: day / 60,
    how_dates_work: 'Issues have estimates (work), not dates. The Timeline does them in the Backlog order, each after what it waits for, when its person has time; time away splits work.',
    statuses: v.statuses.map((s) => s.name), types: v.types.map((t) => t.name),
    people: v.resources.map((r) => `${r.name} (${(r.day_minutes || day) / 60} h a day${(r.away || []).length ? `, away ${r.away.map(([a, b]) => `${a}–${b}`).join(', ')}` : ''})`),
    epics: v.tasks.filter((t) => t.epic).map((e) => `${e.key} ${e.title}: ${when(e)}, ${plural(issues.filter((t) => t.parent === e.id).length, 'issue')}`),
    milestones: issues.filter((t) => t.milestone).map((t) => `${t.key} ${t.title}: ${when(t)}${t.due ? `, due ${t.due}` : ''}${t.late ? ` (${t.late_by} working days late)` : ''}`),
    late: issues.filter((t) => t.late).map((t) => `${t.key} ${t.title}: finishes ${t.finish}, due ${t.due}`),
    without_estimate: issues.filter((t) => !t.milestone && t.work == null && !t.is_done).map((t) => `${t.key} ${t.title}`),
    issues: issues.map((t) => `${t.key} ${t.title} · ${statusName(v, t.status)} · ${t.who_name || 'no one'} · ${t.milestone ? 'milestone' : estText(t.work, day) || 'no estimate'} · ${when(t)}`).slice(0, 150),
  };
}

export function findTasks(tab, query) {
  return call('find', { tab, query: query || '' }).slice(0, 40);
}

export function getTask(tab, ref) {
  const v = call('view', { tab });
  const t = taskOf(v, ref);
  const out = brief(v, t);
  out.holds_up = (t.blocks || []).map((id) => v.tasks.find((x) => x.id === id)?.key).filter(Boolean);
  if (t.epic) out.issues = v.tasks.filter((x) => x.parent === t.id).map((x) => `${x.key} ${x.title}`);
  out.description = plainText(t.description);
  out.labels = t.labels;
  for (const f of v.fields) if (t.fields?.[f.id] != null) out[f.name] = t.fields[f.id];
  out.comments = t.comments.slice(-5).map((c) => `${c.who}: ${plainText(c.html)}`);
  out.history = call('history', { tab, task: t.id }).slice(0, 8).map((e) => `${e.who}: ${e.what}`);
  return out;
}

export function whoIsBusy(tab, from, weeks = 6) {
  const v = call('view', { tab });
  const rows = call('load', { tab, from: from || v.today, weeks: Math.min(weeks || 6, 26) });
  const hrs = (m) => Math.round((m / 60) * 10) / 10;
  return rows.map((r) => ({ name: r.name, weeks: r.cells.map((c) => `week of ${c.week}: ${hrs(c.booked)} of ${hrs(c.have)} hours${c.away ? `, ${c.away} days away` : ''}`) }));
}

// ----- Changing -----

const FIELDS = ['summary', 'status', 'estimate', 'who', 'priority', 'due', 'epic', 'description', 'type', 'labels'];
const ref = { type: 'string', description: 'An issue: key like WEB-6, or its summary' };

/// Changes grouped by kind (easier for models than one mixed list).
export const PROPOSAL_SCHEMA = {
  why: { type: 'string', description: 'One sentence the person sees' },
  add: { type: 'array', description: 'New issues, each once, in order', items: { type: 'object', properties: {
    title: { type: 'string', description: 'A short summary, like "Payment page"' },
    type: { type: 'string', description: 'Story, Task, Bug, Epic or Milestone (default Task)' },
    epic: { ...ref, description: 'The epic it belongs to' },
    estimate: { type: 'string', description: 'Work, like 3d, 4h or 1d 4h' },
    who: { type: 'string', description: 'Who does it, a name from plan_overview' },
    waits_for: { type: 'array', items: ref, description: 'Issues it starts after' },
  }, required: ['title'] } },
  change: { type: 'array', items: { type: 'object', properties: { task: ref, field: { type: 'string', enum: FIELDS }, value: { type: 'string', description: 'Status and type by name, estimate like 2d, due like 2026-12-07, who by name ("" for no one), labels comma-separated (the whole list)' } }, required: ['task', 'field', 'value'] } },
  link: { type: 'array', items: { type: 'object', properties: { task: ref, waits_for: ref }, required: ['task', 'waits_for'] } },
  hold: { type: 'array', items: { type: 'object', properties: { task: ref, until: { type: 'string', description: '2026-12-07; empty: start as soon as it can' } }, required: ['task'] } },
  remove: { type: 'array', items: ref, description: 'Issues to delete (only when asked)' },
};

/// A grouped proposal (PROPOSAL_SCHEMA) as the list of changes `apply` makes.
export function fromProposal(a) {
  if (Array.isArray(a.changes)) return tidy(a.changes);
  const out = [];
  const titles = new Set();
  for (const t of a.add || []) {
    if (!t?.title || titles.has(t.title.toLowerCase())) continue;
    titles.add(t.title.toLowerCase());
    out.push({ do: 'add_task', task: t.title, type: t.type, epic: t.epic, estimate: t.estimate, who: t.who });
  }
  for (const t of a.add || []) for (const f of [].concat(t?.waits_for || [])) out.push({ do: 'link', task: t.title, from: f });
  for (const c of a.change || []) out.push({ do: 'set', task: c.task, field: c.field, value: c.value });
  for (const l of a.link || []) out.push({ do: 'link', task: l.task, from: l.waits_for ?? l.after });
  for (const x of a.hold || []) out.push({ do: 'hold', task: x.task, until: x.until || '' });
  for (const r of a.remove || []) out.push({ do: 'delete', task: r });
  return tidy(out);
}

/// The same change twice (models repeat themselves) counts once.
export function tidy(changes) {
  const seen = new Set();
  const adds = new Map();
  const out = [];
  for (const c of Array.isArray(changes) ? changes : []) {
    const k = JSON.stringify(c);
    if (seen.has(k)) continue;
    seen.add(k);
    // An issue added twice is one issue: the later mention fills in what the first left out.
    if (c?.do === 'add_task') {
      const t = String(c.task || '').toLowerCase();
      const first = adds.get(t);
      if (first) { for (const [f, val] of Object.entries(c)) if (val != null) first[f] = val; continue; }
      const copy = { ...c };
      adds.set(t, copy);
      out.push(copy);
      continue;
    }
    out.push(c);
  }
  return out;
}

/// One line per change, for people to read before Apply.
export function describe(tab, changes) {
  const v = call('view', { tab });
  return changes.map((c) => {
    try {
      switch (c.do) {
        case 'add_task': return ['Add', [c.type && lower(c.type) !== 'task' ? `${c.type}: ` : '', c.task, c.epic ? ` in ${taskOf(v, c.epic).title}` : '', c.estimate ? `, ${c.estimate}` : '', c.who ? `, ${c.who}` : ''].join('')];
        case 'set': return ['Change', `${c.task}: ${c.field} → ${c.value === '' ? 'none' : c.value}`];
        case 'link': return ['Link', `${c.task} waits for ${c.from}`];
        case 'unlink': return ['Link', `${c.task} no longer waits for ${c.from}`];
        case 'hold': return ['Hold', c.until ? `${c.task} starts no earlier than ${dayName(c.until)}` : `${c.task} starts as soon as it can`];
        case 'delete': return ['Delete', c.task];
        default: return ['?', JSON.stringify(c)];
      }
    } catch (e) { return ['?', `${JSON.stringify(c)}: ${e.message}`]; }
  });
}

/// A field change in the engine's terms.
function fieldChange(v, id, field, value) {
  const day = v.settings.day_minutes;
  switch (lower(field)) {
    case 'summary': case 'title': return ['title', String(value).trim()];
    case 'status': return ['status', v.statuses.find((s) => lower(s.name) === lower(value))?.id ?? (() => { throw new Error(`No status called ${value}: ${v.statuses.map((s) => s.name).join(', ')}`); })()];
    case 'estimate': case 'work': { const m = parseEst(value, day); if (m == null && lower(value) !== '' && lower(value) !== 'none') throw new Error(`Estimates look like 3d or 4h, not “${value}”`); return ['work', m]; }
    case 'who': case 'assignee': return ['who', NOBODY.includes(lower(value)) ? null : resourceOf(v, value).id];
    case 'priority': { const p = lower(value); if (!['highest', 'high', 'medium', 'low'].includes(p)) throw new Error('Priority is Highest, High, Medium or Low'); return ['priority', p]; }
    case 'due': return ['due', /^\d{4}-\d{2}-\d{2}$/.test(String(value).trim()) ? String(value).trim() : null];
    case 'epic': return ['parent', NOBODY.includes(lower(value)) ? null : taskOf(v, value).id];
    case 'description': return ['description', String(value).split(/\n{2,}/).map((p) => `<p>${p.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]))}</p>`).join('')];
    case 'type': { const t = v.types.find((x) => lower(x.name) === lower(value) || x.id === lower(value)); if (!t) throw new Error(`No type ${value}: ${v.types.map((x) => x.name).join(', ')}`); return ['kind', t.id]; }
    case 'labels': case 'label': case 'tags': return ['labels', [...new Set(String(value).split(',').map((x) => x.trim()).filter(Boolean))]];
    default: throw new Error(`Can't change “${field}”: ${FIELDS.join(', ')}`);
  }
}

/// Makes `changes` as one step called `what`. Returns what happened.
export function apply(tab, changes, what) {
  const made = [];
  const added = new Map();
  const v0 = () => call('view', { tab });
  const refOf = (r) => { const v = v0(); return added.get(lower(r)) || taskOf(v, r).id; };
  call('group_start', { tab });
  try {
    for (const c of changes) {
      const v = v0();
      switch (c.do) {
        case 'add_task': {
          const type = c.type ? v.types.find((x) => lower(x.name) === lower(c.type) || x.id === lower(c.type))?.id || 'task' : 'task';
          const args = { tab, title: c.task, kind: type };
          if (c.epic) args.parent = refOf(c.epic);
          if (c.estimate && type !== 'epic' && type !== 'milestone') args.work = fieldChange(v, null, 'estimate', c.estimate)[1];
          if (c.who && type !== 'epic') args.who = fieldChange(v, null, 'who', c.who)[1];
          const id = call('add_task', args).id;
          added.set(lower(c.task), id);
          made.push(`added ${c.task}`);
          break;
        }
        case 'set': {
          const id = refOf(c.task);
          const [field, value] = fieldChange(v, id, c.field, c.value);
          // Into an epic keeps its rank, as on the Backlog (an epic's issues come after it).
          if (field === 'parent') call('set_epic', { tab, id, parent: value });
          else call('set_task', { tab, id, field, value });
          if (field === 'kind') call('set_task', { tab, id, field: 'milestone', value: value === 'milestone' });
          made.push(`changed ${c.task}`);
          break;
        }
        case 'link': call('link', { tab, from: refOf(c.from), to: refOf(c.task) }); made.push(`linked ${c.task}`); break;
        case 'unlink': call('unlink', { tab, from: refOf(c.from), to: refOf(c.task) }); made.push(`unlinked ${c.task}`); break;
        case 'hold': call('hold', { tab, id: refOf(c.task), start: /^\d{4}-\d{2}-\d{2}$/.test(String(c.until || '').trim()) ? c.until.trim() : null }); made.push(`held ${c.task}`); break;
        case 'delete': call('delete_tasks', { tab, ids: [refOf(c.task)] }); made.push(`deleted ${c.task}`); break;
        default: throw new Error(`Can't do “${c.do}”`);
      }
    }
  } catch (e) {
    // Nothing half done: what went in comes out again.
    call('group_end', { tab, what });
    if (made.length) call('undo', { tab });
    throw e;
  }
  call('group_end', { tab, what });
  return { done: made };
}
