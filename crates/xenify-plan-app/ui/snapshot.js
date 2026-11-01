// What a plan on a server shows to other documents (Write's Insert › From
// Plan): the issues and milestones as tables, one table per project, the
// key dates (groups and milestones: expected, now, how far they moved), and
// the Timeline as a picture. The same shape Calc puts up for its books,
// so Write keeps them up to date the same way. Put up at each saved version
// and now and then while editing together.

import { call } from './engine.js';
import { days, dayName, short, parts, monthShort, weekday, moved } from './fmt.js';
import { estText } from './pieces.js';

const W = 1100;
const ROW = 26;
const LEFT = 300;

/// Whether issue `t` is group (epic) `g` or somewhere inside it.
function inGroup(v, t, g) {
  for (let x = t, n = 0; x && n < 64; x = v.tasks.find((y) => y.id === x.parent), n++) if (x.id === g) return true;
  return false;
}

/// When row `t` was expected: the dates kept with Timeline › Expected
/// dates, else (for its finish) its due date.
const expectedEnd = (t) => t.expected?.[1] || t.due || null;

/// The Timeline of tab `tab` (`project`: only its issues; `group`: only
/// that epic and what's in it; `key`: only groups and milestones) as a PNG,
/// base64. Where a row was expected shows faintly under it when it moved.
export function ganttPng(v, project = null, group = null, key = false) {
  const tasks = v.tasks.filter((t) => t.placed && !t.release && (!project || t.summary || t.projects.includes(project)) && (!group || inGroup(v, t, group))
    && (!key || t.epic || t.milestone));
  if (!tasks.length) return null;
  const was = (t) => (t.expected && (t.expected[0] !== t.start || t.expected[1] !== t.finish) ? t.expected : null);
  const from = Math.min(...tasks.flatMap((t) => [days(t.start), ...(was(t) ? [days(was(t)[0])] : [])])) - 3;
  const to = Math.max(...tasks.flatMap((t) => [days(t.finish), ...(was(t) ? [days(was(t)[1])] : [])]), days(v.today)) + 5;
  const D = (W - LEFT - 20) / (to - from + 1);
  const H = 44 + tasks.length * ROW + 12;
  const c = document.createElement('canvas');
  const scale = 2;
  c.width = W * scale;
  c.height = H * scale;
  const g = c.getContext('2d');
  g.scale(scale, scale);
  g.fillStyle = '#ffffff';
  g.fillRect(0, 0, W, H);
  const x = (s) => LEFT + (days(s) - from) * D;
  // Weekends, holidays and the month/week scale.
  for (let d = from; d <= to; d++) {
    if (weekday(d) >= 5 && D >= 4) { g.fillStyle = '#F7F5F2'; g.fillRect(LEFT + (d - from) * D, 44, D, H - 44); }
    const p = parts(d);
    // The first label only when the next month isn't right after it.
    const roomy = d !== from || (Math.round(Date.UTC(p.y, p.m + 1, 1) / 86400000) - from) * D > 80;
    if (p.d === 1 || (d === from && roomy)) {
      g.fillStyle = '#44403C';
      g.font = '600 12px Figtree, sans-serif';
      g.fillText(`${monthShort(p.m)} ${p.y}`, LEFT + (d - from) * D + 4, 16);
      g.fillStyle = '#E2DDD7';
      g.fillRect(LEFT + (d - from) * D, 4, 1, H - 4);
    }
    if (weekday(d) === 0 && D * 7 > 34) { g.fillStyle = '#78716C'; g.font = '500 10.5px Figtree, sans-serif'; g.fillText(String(p.d), LEFT + (d - from) * D + 3, 36); }
  }
  for (const off of v.days_off) {
    const d = days(off.date);
    if (d < from || d > to) continue;
    g.fillStyle = '#FBE3D5';
    g.fillRect(LEFT + (d - from) * D, 44, Math.max(D, 1.5), H - 44);
  }
  g.fillStyle = '#E2DDD7';
  g.fillRect(0, 43, W, 1);
  tasks.forEach((t, i) => {
    const y = 44 + i * ROW;
    g.fillStyle = '#1C1917';
    g.font = `${t.summary ? 700 : 500} 12.5px Figtree, sans-serif`;
    const label = `${t.summary ? '' : `${t.key} `}${t.title}`;
    let text = label;
    while (g.measureText(text).width > LEFT - 20 - t.depth * 14 && text.length > 3) text = `${text.slice(0, -2)}…`;
    g.fillText(text, 10 + t.depth * 14, y + 17);
    const e = was(t);
    if (e) {
      // Where it was expected: a faint line under the bar, or a hollow diamond.
      g.save();
      if (t.milestone) {
        g.translate(x(e[1]) + D / 2, y + 13); g.rotate(Math.PI / 4); g.strokeStyle = '#A8A29E'; g.lineWidth = 1.5; g.setLineDash([2, 2]); g.strokeRect(-5, -5, 10, 10);
      } else {
        g.fillStyle = '#D6D3D1';
        g.fillRect(x(e[0]), y + 22, (days(e[1]) - days(e[0]) + 1) * D, 3);
      }
      g.restore();
    }
    const sx = x(t.start);
    const w = (days(t.finish) - days(t.start) + 1) * D;
    if (t.summary) {
      g.fillStyle = '#44403C';
      g.fillRect(sx, y + 10, w, 6);
    } else if (t.milestone) {
      g.save(); g.translate(sx + D / 2, y + 13); g.rotate(Math.PI / 4); g.fillStyle = '#1C1917'; g.fillRect(-5, -5, 10, 10); g.restore();
      g.fillStyle = '#44403C'; g.font = '500 11px Figtree, sans-serif'; g.fillText(short(t.finish), sx + D / 2 + 10, y + 17);
    } else {
      for (const [a, b] of t.pieces.length ? t.pieces : [[t.start, t.finish]]) {
        const px = x(a);
        const pw = (days(b) - days(a) + 1) * D;
        g.fillStyle = t.is_done ? '#E7E5E4' : '#FBE3D5';
        g.fillRect(px, y + 6, pw, 14);
        g.strokeStyle = t.late ? '#B91C1C' : '#E8A27F';
        g.lineWidth = t.late ? 2 : 1;
        g.strokeRect(px + 0.5, y + 6.5, pw - 1, 13);
      }
      if (t.who_name) { g.fillStyle = '#57534E'; g.font = '500 10.5px Figtree, sans-serif'; g.fillText(t.who_name.split(' ')[0], x(t.finish) + D + 6, y + 17); }
    }
  });
  const tx = x(v.today);
  g.fillStyle = '#C2410C';
  g.fillRect(tx, 40, 2, H - 40);
  return c.toDataURL('image/png').split(',')[1];
}

function breakdown(v, list, statusName) {
  const rows = [['Issue', 'Who', 'Estimate', 'Start', 'Finish', 'Status']];
  for (const t of list) {
    rows.push([
      `${'    '.repeat(t.depth)}${t.key} ${t.title}`,
      t.summary ? '' : t.who_name || '',
      // A mirror has no estimate of its own: its dates are the other plan's.
      t.milestone || t.mirror ? '' : estText(t.summary ? t.total : t.work, v.settings.day_minutes),
      t.placed ? dayName(t.start) : '—',
      t.placed ? (t.milestone ? `◆ ${dayName(t.finish)}` : dayName(t.finish)) : '—',
      t.summary ? '' : `${statusName(t.status)}${t.late ? ` (${t.late_by} days late)` : ''}`,
    ]);
  }
  return rows;
}

/// The snapshot for tab `tab`, as JSON text.
export function snapshot(tab) {
  const v = call('view', { tab });
  const statusName = (id) => v.statuses.find((s) => s.id === id)?.name || '';
  const nums = [false, false, false, false, false, false];
  const items = [
    { id: 'breakdown', kind: 'table', name: 'Breakdown', rows: breakdown(v, v.tasks, statusName), header: true, footer: false, numbers: nums },
  ];
  const ms = v.tasks.filter((t) => t.milestone);
  if (ms.length) {
    items.push({ id: 'milestones', kind: 'table', name: 'Milestones', header: true, footer: false, numbers: [false, false, false, false],
      rows: [['Milestone', 'Date', 'Due', 'Late'], ...ms.map((t) => [t.title, t.placed ? dayName(t.finish) : '—', t.due ? dayName(t.due) : '—', t.late ? `${t.late_by} working days` : '—'])] });
  }
  if (v.projects.length > 1) {
    for (const p of v.projects) {
      const list = v.tasks.filter((t) => !t.summary && t.projects.includes(p.id));
      if (list.length) items.push({ id: `project-${p.id}`, kind: 'table', name: `${p.name} tasks`, rows: breakdown(v, list.map((t) => ({ ...t, depth: 0 })), statusName), header: true, footer: false, numbers: nums });
    }
  }
  // Key dates: each group's finish and each milestone, expected and now.
  const key = v.tasks.filter((t) => !t.release && (t.epic || t.milestone));
  if (key.length) {
    items.push({ id: 'key-dates', kind: 'table', name: 'Key dates', header: true, footer: false, numbers: [false, false, false, false],
      rows: [['Key date', 'Expected', 'Now', 'Difference'], ...key.map((t) => {
        const exp = expectedEnd(t);
        const now = t.placed ? t.finish : null;
        return [`${t.milestone ? '◆ ' : ''}${t.title || t.key}`, exp ? dayName(exp) : '—', now ? dayName(now) : '—', t.is_done ? 'Done' : moved(exp, now) || '—'];
      })] });
  }
  const png = ganttPng(v);
  if (png) items.push({ id: 'gantt', kind: 'chart', name: 'Timeline', png });
  const kp = ganttPng(v, null, null, true);
  if (kp) items.push({ id: 'gantt-key', kind: 'chart', name: 'Key dates timeline', png: kp });
  if (v.projects.length > 1) {
    for (const p of v.projects) {
      const pp = ganttPng(v, p.id);
      if (pp) items.push({ id: `gantt-${p.id}`, kind: 'chart', name: `Timeline: ${p.name}`, png: pp });
    }
  }
  // Each group (epic) on its own: its issues as a table and its Timeline.
  for (const e of v.tasks.filter((t) => t.epic)) {
    const list = v.tasks.filter((t) => t.id !== e.id && !t.release && inGroup(v, t, e.id));
    if (!list.length) continue;
    const name = e.title || e.key;
    items.push({ id: `group-${e.id}`, kind: 'table', name: `Group: ${name}`, rows: breakdown(v, list.map((t) => ({ ...t, depth: Math.max(0, t.depth - e.depth - 1) })), statusName), header: true, footer: false, numbers: nums });
    const gp = ganttPng(v, null, e.id);
    if (gp) items.push({ id: `gantt-group-${e.id}`, kind: 'chart', name: `Timeline: ${name}`, png: gp });
  }
  // Tasks, for chips in a document: "WEB-6 Checkout pages · In progress", coloured by how it's going.
  for (const t of v.tasks.filter((x) => !x.summary).slice(0, 400)) {
    const st = v.statuses.find((x) => x.id === t.status);
    const colour = t.is_done ? 'green' : t.late ? 'red' : t.is_started ? 'blue' : 'grey';
    items.push({ id: `task-${t.id}`, kind: 'task', task: t.id, name: `${t.key} ${t.title}`, text: `${t.key} ${t.title} · ${t.late ? 'Late' : st?.name || ''}`, color: colour });
  }
  // Calc's format, so Write treats both alike.
  return JSON.stringify({ format: 'xenify-calc-link', app: 'plan', items });
}
