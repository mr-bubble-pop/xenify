// Timeline: when everything happens, worked out from estimates, the
// Backlog's order, "waits for" links and people's time. Nothing here is
// typed as a date: drag a bar later to hold it back, drag the dot at its end
// onto another bar to make that one wait for it. Work split by time off
// shows as pieces joined by a dotted line. Issues without an estimate wait
// in a tray below until they get one.

import { h, icon, menu, item, toast, ask, modal } from './ui.js';
import { days, iso, dayName, parts, monthShort, weekday, plural, when, moved } from './fmt.js';
import { typeIcon, avatar, assignee, estText, parseEst, epicColour, mirrorColour, shown, keyClass, person, personColour, may } from './pieces.js';
import { quickAdd } from './create.js';
import { deleteItem } from './remove.js';
import { openRef, wouldLoop } from './serverplan.js';
import { createMirror } from './issue.js';

/// A mirror's mark: it points somewhere else.
export const mirrorIcon = (size = 16) => h('span', { class: 'ty mirror', 'aria-hidden': 'true', style: { width: `${size}px`, height: `${size}px` },
  html: `<svg width="${size - 4}" height="${size - 4}" viewBox="0 0 16 16" fill="none" stroke="#fff" stroke-width="2"><path d="M6 3H3v10h10v-3M9 3h4v4M13 3L7 9"/></svg>` });
/// What a mirror shows, in words: "all of Website", "BRA-3 Logo in Brand".
export function mirrorWhat(v, t) {
  const o = v.outside?.[t.mirror] || {};
  if (!o.known) return 'Asking the server…';
  if (o.gone) return 'Not there any more';
  const plan = o.plan || (t.mirror.endsWith(':*') ? o.title : '') || 'another plan';
  return t.mirror.endsWith(':*') ? `all of “${plan}”` : `${o.key || ''}${o.readable && o.title ? ` ${o.title}` : ''} in “${plan}”`.trim();
}

const RH = 36;
const HEAD = 52;
const SVG = 'http://www.w3.org/2000/svg';
// A day's width at each zoom: Months fits half a year on a screen.
const WIDTH = { days: 28, weeks: 12, months: 4 };
const dayWidth = (tab) => WIDTH[tab.zoom] || WIDTH.days;

export function tools(app, v, tab) {
  const zoom = h('div', { class: 'segs', role: 'group', 'aria-label': 'Zoom' },
    [['days', 'Days'], ['weeks', 'Weeks'], ['months', 'Months']].map(([k, label]) => h('button', { class: 'seg', 'aria-pressed': String((tab.zoom || 'days') === k), onClick: () => { tab.zoom = k; tab.scrolled = false; app.render(); } }, label)));
  const expected = h('button', { class: 'tool', 'aria-haspopup': 'menu', title: 'Keep today’s dates to compare with later: documents show each key date’s expected and current date',
    onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); expectedMenu(app, v, tab, r.left, r.bottom + 4); } }, 'Expected dates');
  return [h('button', { class: 'tool', onClick: () => scrollToDay(v, tab, v.today, true) }, 'Today'), zoom, expected];
}

/// Timeline › Expected dates: keep the dates as they are now, to see later
/// how far groups, milestones and issues moved (Write's Key dates too).
function expectedMenu(app, v, tab, x, y) {
  const keep = () => {
    app.edit('save_baseline');
    toast('Kept: the Timeline and documents compare with these dates now', ['Undo', () => app.edit('undo')]);
  };
  const items = [
    { head: v.expected ? `Kept ${when(v.expected.saved)}${v.expected.by ? ` by ${v.expected.by}` : ''}` : 'None kept yet' },
    item(v.expected ? 'Keep today’s dates instead' : 'Keep today’s dates as expected', { enabled: may(v, 'schedule'), run: async () => {
      if (v.expected && !(await modal('Keep today’s dates instead?', h('p', {}, 'The dates kept before are replaced: the Timeline and key dates in documents then compare with today’s.'), [['Cancel', null], ['Keep them', true, true]]))) return;
      keep();
    } }),
  ];
  if (v.expected) {
    items.push(item('Show them on the Timeline', { checked: tab.hideExpected !== true, run: () => { tab.hideExpected = tab.hideExpected !== true; app.render(); } }),
      null,
      item('Forget the expected dates', { enabled: may(v, 'schedule'), run: () => { app.edit('clear_baseline'); toast('Forgot the expected dates', ['Undo', () => app.edit('undo')]); } }));
  }
  menu(items, x, y);
}

/// A faint mark where row `t` was expected (Timeline › Expected dates), when it moved since.
function expectedMark(t, x, DW, ms) {
  if (!t.expected || !t.placed) return null;
  const [a, b] = t.expected;
  if (a === t.start && b === t.finish) return null;
  const title = `Expected ${ms ? dayName(b) : `${dayName(a)} – ${dayName(b)}`}; now ${moved(b, t.finish).toLowerCase()}`;
  return ms ? h('div', { class: 'ms-exp', title, style: { left: `${x(b) - 7}px` } })
    : h('div', { class: 'exp', title, style: { left: `${x(a) + 2}px`, width: `${Math.max(3, x(b) - x(a) + DW - 4)}px` } });
}

function scrollToDay(v, tab, date, smooth) {
  const main = document.getElementById('main');
  const DW = dayWidth(tab);
  main.scrollTo({ left: Math.max(0, (days(date) - days(v.from)) * DW - 140), behavior: smooth ? 'smooth' : 'auto' });
}

export function render(main, v, tab, app) {
  const DW = dayWidth(tab);
  const from = days(v.from);
  // At least the window's width: zoomed out, the dates go on past the work.
  const span = Math.max(days(v.to) - from + 1, Math.ceil((main.clientWidth || 1200) / DW));
  const W = span * DW;
  const x = (s) => (days(s) - from) * DW;
  const collapsed = (tab.collapsed ||= new Set());
  const day = v.settings.day_minutes;

  // Rows: each epic, then its issues; issues without an epic at the end.
  const rows = [];
  const issues = v.tasks.filter((t) => !t.epic && shown(app, t));
  for (const e of v.tasks.filter((t) => t.epic)) {
    const kids = issues.filter((t) => t.parent === e.id);
    if (!kids.length && app.tab.filter?.text) continue;
    rows.push({ epic: e, kids });
    if (!collapsed.has(e.id)) kids.forEach((t) => rows.push({ t }));
  }
  const loose = issues.filter((t) => !t.parent || !v.tasks.some((e) => e.id === t.parent && e.epic));
  if (loose.length) {
    if (rows.length) rows.push({ epic: null, kids: loose });
    loose.forEach((t) => rows.push({ t }));
  }

  const missing = v.tasks.filter((t) => !t.epic && !t.milestone && !t.placed && !t.is_done && shown(app, t)).length;
  const left = h('div', { class: 'tl-left' }, h('div', { class: 'tl-lhead' }, 'Issue',
    missing ? h('button', { class: 'tl-missing', title: 'Issues without an estimate aren’t on the Timeline: give them one below', onClick: () => document.querySelector('.tl-tray h3')?.scrollIntoView({ behavior: 'smooth', block: 'center' }) }, `${missing} not on it yet`) : null));
  const chart = h('div', { class: 'tl-chart', style: { width: `${W}px`, height: `${HEAD + rows.length * RH}px` } });

  // Dates along the top: months, then days (or week starts).
  const months = h('div', { class: 'tl-months' });
  const dayRow = h('div', { class: 'tl-days' });
  let m0 = null;
  let mStart = 0;
  for (let d = 0; d <= span; d++) {
    const p = d < span ? parts(from + d) : null;
    const key = p ? `${p.y}-${p.m}` : 'end';
    if (key !== m0) {
      if (m0 !== null) {
        const q = parts(from + mStart);
        const mw = (d - mStart) * DW;
        // The year with January (or the first month shown), else just the month.
        const label = mw > 60 && (q.m === 0 || mStart === 0) ? `${monthShort(q.m)} ${q.y}` : mw > 26 ? monthShort(q.m) : '';
        months.append(h('div', { style: { width: `${mw}px` } }, label));
      }
      m0 = key;
      mStart = d;
    }
    if (d < span) {
      const wd = weekday(from + d);
      // Days: each working day; Weeks: Mondays; Months: the month row says it all.
      const label = DW >= 20 ? (wd < 5 ? String(p.d) : '') : DW >= 10 && wd === 0 ? String(p.d) : '';
      dayRow.append(h('div', { class: wd === 0 && DW >= 10 ? 'mon' : '', style: { width: `${DW}px` } }, label));
    }
  }
  const grid = h('div', { class: 'tl-grid', style: { width: `${W}px` } });
  for (let d = 0; d < span && DW >= 10; d++) if (weekday(from + d) >= 5) grid.append(h('div', { class: 'we', style: { left: `${d * DW}px`, width: `${DW}px` } }));
  for (const off of v.days_off) {
    const d = days(off.date) - from;
    if (d >= 0 && d < span) grid.append(h('div', { class: 'hol', title: off.name, style: { left: `${d * DW}px`, width: `${DW}px` } }));
  }
  grid.append(h('div', { class: 'today', style: { left: `${x(v.today)}px` } }, h('span', {}, 'Today')));
  chart.append(h('div', { class: 'tl-dates' }, months, dayRow), grid);

  const pos = new Map();
  const byId = new Map(v.tasks.map((t) => [t.id, t]));
  rows.forEach((row, i) => {
    const y = HEAD + i * RH;
    if ('epic' in row && !row.t) {
      const e = row.epic;
      const open = e && !collapsed.has(e.id);
      left.append(h('div', { class: 'tl-row ep', onContextmenu: e ? (ev) => { ev.preventDefault(); menu([item('Open', { run: () => app.openTask(e.id) }), null, deleteItem(app, e)], ev.clientX, ev.clientY); } : null },
        e ? h('button', { class: 'twisty', 'aria-expanded': String(open), 'aria-label': open ? `Fold ${e.title}` : `Unfold ${e.title}`, html: icon('down', 14),
          onClick: () => { if (collapsed.has(e.id)) collapsed.delete(e.id); else collapsed.add(e.id); app.render(); } }) : null,
        e ? typeIcon(v, 'epic', 16, epicColour(v, e.id)) : null,
        h('button', { class: 'sum', onClick: () => e && app.openTask(e.id) }, e ? e.title || e.key : 'No epic'),
        h('span', { class: 'n' }, String(row.kids.length))));
      const lane = h('div', { class: 'tl-lane ep', style: { top: `${y}px` } });
      if (e?.placed) {
        const c = epicColour(v, e.id);
        if (!tab.hideExpected) { const m = expectedMark(e, x, DW, false); if (m) lane.append(m); }
        lane.append(h('div', { class: 'sumbar', title: `${e.title}: ${dayName(e.start)} – ${dayName(e.finish)}`, style: { left: `${x(e.start)}px`, width: `${Math.max(DW, x(e.finish) - x(e.start) + DW)}px`, '--c': c } }));
      }
      chart.append(lane);
      return;
    }
    const t = row.t;
    const p = (t.by || t.who) ? person(v, t.by || t.who) : null;
    left.append(h('div', { class: `tl-row${t.parent ? ' child' : ''}${tab.selected === t.id ? ' sel' : ''}${tab.flash === t.id ? ' flash' : ''}`, title: t.title, onClick: () => app.openTask(t.id),
      onContextmenu: (e) => rowMenu(e, t, v, app) },
      t.mirror ? mirrorIcon() : typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key), h('span', { class: 'sum' }, t.title), t.mirror ? null : assignee(v, t, 22)));
    const lane = h('div', { class: 'tl-lane', style: { top: `${y}px` } });
    if (t.mirror) {
      // A mirror: the other one's dates, nothing of its own. A click opens it there.
      if (t.placed) {
        const w = Math.max(6, x(t.finish) - x(t.start) + DW - 4);
        const done = t.done >= 100;
        const bar = h('div', {
          class: `bar mirror${done ? ' done' : ''}`, tabindex: '0', role: 'button', style: { left: `${x(t.start) + 2}px`, width: `${w}px`, '--c': mirrorColour(v, t.id) },
          'aria-label': `${t.title}: mirrors ${mirrorWhat(v, t)}, ${dayName(t.start)} to ${dayName(t.finish)}`,
          title: `${t.title}\nMirrors ${mirrorWhat(v, t)}: ${dayName(t.start)} – ${dayName(t.finish)}\nDouble-click to go there · drag the end dot onto another bar: that one then starts after this`,
        }, w > 70 ? h('span', { class: 'lbl' }, t.title) : null);
        if (!done) bar.append(h('span', { class: 'dot', title: 'Drag onto another issue: it then starts after this' }));
        bar.addEventListener('dblclick', () => openRef(t.mirror));
        drag(bar, t, chart, pos, v, tab, app, DW);
        bar.addEventListener('contextmenu', (e) => barMenu(e, t, app));
        lane.append(bar);
        if (w <= 70) lane.append(h('span', { class: 'ms-label', style: { left: `${x(t.start) + w + 22}px` } }, t.title));
        pos.set(t.id, { x0: x(t.start) + 2, x1: x(t.finish) + DW - 2, y: y + RH / 2 });
      } else {
        const o = v.outside?.[t.mirror];
        lane.append(h('div', { class: 'unplaced', style: { left: `${x(v.today) + 8}px` } }, !o?.known ? 'Asking the server…' : o.gone ? 'What it mirrors isn’t there any more' : o.done ? 'Done' : 'Not on its Timeline yet'));
      }
      chart.append(lane);
      return;
    }
    for (const [a, b] of p?.away || []) {
      if (days(b) < from || days(a) > from + span) continue;
      lane.append(h('div', { class: 'away', title: `${p.name} is away ${dayName(a)} – ${dayName(b)}`, style: { left: `${x(a)}px`, width: `${x(b) - x(a) + DW}px` } }));
    }
    if (t.due) lane.append(h('div', { class: `due${t.late ? ' late' : ''}`, title: `Due ${dayName(t.due)}`, style: { left: `${x(t.due) + DW}px` } }));
    if (!tab.hideExpected) { const m = expectedMark(t, x, DW, t.milestone); if (m) lane.append(m); }
    if (t.placed && t.milestone) {
      const m = h('div', { class: `ms${t.late ? ' late' : ''}`, tabindex: '0', role: 'button', 'aria-label': `${t.title}, ${dayName(t.start)}`,
        title: `${t.key} ${t.title}: ${dayName(t.start)}${t.late ? `, ${plural(t.late_by, 'working day')} after its due date` : ''}${t.waits.length ? `\nAlso starts after issues without an estimate` : ''}`,
        style: { left: `${x(t.start) - 9}px` } });
      m.addEventListener('click', () => app.openTask(t.id));
      lane.append(m, h('span', { class: 'ms-label', style: { left: `${x(t.start) + 14}px` } }, dayName(t.start)));
      pos.set(t.id, { x0: x(t.start) - 9, x1: x(t.start) + 9, y: y + RH / 2 });
    } else if (t.placed) {
      const colour = p ? personColour(v, p.id) : null;
      t.pieces.forEach(([a, b], k) => {
        if (k) { const prev = t.pieces[k - 1]; lane.append(h('div', { class: 'join', style: { left: `${x(prev[1]) + DW}px`, width: `${x(a) - x(prev[1]) - DW}px` } })); }
        const w = Math.max(3, x(b) - x(a) + DW - 4);
        const bar = h('div', {
          class: `bar${p ? '' : ' nobody'}${t.is_done ? ' done' : ''}${t.late ? ' late' : ''}`, tabindex: '0', role: 'button', style: { left: `${x(a) + 2}px`, width: `${w}px`, '--c': colour || 'var(--faint)' },
          'aria-label': `${t.key} ${t.title}, ${dayName(a)} to ${dayName(b)}`,
          title: `${t.key} ${t.title}\n${dayName(t.start)} – ${dayName(t.finish)}${t.pieces.length > 1 ? ` in ${t.pieces.length} pieces` : ''}\n${estText(t.work, day)} of work, ${p ? p.name : 'no one yet'}${t.held ? `\nNot before ${dayName(t.held)}` : ''}\nDrag to start it later · drag the end dot onto another bar: that one then starts after this`,
        }, k === 0 && w > 70 ? h('span', { class: 'lbl' }, t.title) : null);
        if (t.done > 0 && !t.is_done && k === 0) bar.style.setProperty('--done', `${t.done}%`);
        if (k === t.pieces.length - 1 && !t.is_done) bar.append(h('span', { class: 'dot', title: 'Drag onto another issue: it then starts after this one' }));
        drag(bar, t, chart, pos, v, tab, app, DW);
        bar.addEventListener('contextmenu', (e) => barMenu(e, t, app));
        lane.append(bar);
      });
      if (t.held && !t.is_done) lane.append(h('div', { class: 'held', title: `Not before ${dayName(t.held)} (dragged there)`, style: { left: `${x(t.held)}px` } }));
      pos.set(t.id, { x0: x(t.start) + 2, x1: x(t.finish) + DW - 2, y: y + RH / 2 });
    } else if (!t.is_done) {
      lane.append(h('div', { class: 'unplaced', style: { left: `${x(v.today) + 8}px` } }, t.milestone ? 'Starts after issues without an estimate' : 'Needs an estimate to go on the Timeline'));
    }
    chart.append(lane);
  });

  // Links: an arrow from what finishes first to what waits for it.
  const links = document.createElementNS(SVG, 'svg');
  links.setAttribute('class', 'tl-links');
  links.setAttribute('width', W);
  links.setAttribute('height', rows.length * RH);
  links.innerHTML = '<defs><marker id="tl-arr" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0L8 4 0 8z" class="arrhead"/></marker></defs>';
  for (const t of v.tasks) {
    for (const d of t.after) {
      const a = pos.get(d.task);
      const b = pos.get(t.id);
      if (!a || !b) continue;
      const [x1, y1, x2, y2] = [a.x1, a.y - HEAD, b.x0, b.y - HEAD];
      const path = document.createElementNS(SVG, 'path');
      // Room after the first one: out of its end, down, into the next. Back
      // to back (the next working day) or overlapping: down from under the
      // first bar, just before where the next one starts. Only one that
      // starts before the first even begins goes round.
      // Zoomed out, bars a few pixels wide: straight down (or up) where they meet.
      const xs = Math.max(a.x0 + 2, Math.min(x1 - 6, x2 - 8));
      const edge = y2 > y1 ? y1 + 10 : y1 - 10;
      const xv = Math.max(a.x0, b.x0) + 1;
      path.setAttribute('d', x2 >= x1 + 12 ? `M${x1} ${y1} H${x1 + 8} V${y2} H${x2 - 2}`
        : xs <= x2 - 4 && y1 !== y2 ? `M${xs} ${edge} V${y2} H${x2 - 2}`
        : y1 !== y2 && (xv <= Math.min(a.x1, b.x1) || DW < 10) ? `M${DW < 10 ? b.x0 + 1 : xv} ${edge} V${y2 > y1 ? y2 - 11 : y2 + 11}`
        : `M${x1} ${y1} h8 V${y2 - 12} H${x2 - 10} V${y2} H${x2 - 2}`);
      path.setAttribute('marker-end', 'url(#tl-arr)');
      const late = byId.get(t.id)?.late;
      if (late) path.setAttribute('class', 'late');
      links.append(path);
    }
  }
  chart.append(links);

  // A group (an epic) of its own, made here; right-click a row to move it into one.
  left.append(h('div', { class: 'tl-add-group' }, quickAdd(app, { kind: 'epic', parent: null }, 'Add a group'),
    v.on_server && may(v, 'create') ? h('button', { class: 'linkish small tl-mirror', title: 'A bar here that shows when a whole plan, an epic or an issue elsewhere is done', onClick: () => createMirror(app) }, '+ Mirror another plan…') : null));
  const tl = h('div', { class: 'tl' }, left, chart);

  // What isn't on the Timeline yet.
  const unplaced = v.tasks.filter((t) => !t.epic && !t.milestone && !t.mirror && !t.placed && !t.is_done && shown(app, t));
  const tray = h('div', { class: 'tl-tray' },
    h('div', { class: 'legend' },
      h('span', {}, h('i', { class: 'lg-bar' }), 'Colour: who does it'),
      h('span', {}, h('i', { class: 'lg-nobody' }), 'No one yet'),
      h('span', {}, h('i', { class: 'lg-join' }), 'Split by time off'),
      h('span', {}, h('i', { class: 'lg-away' }), 'Away'),
      h('span', {}, h('i', { class: 'lg-due' }), 'Due date'),
      h('span', {}, h('i', { class: 'lg-held' }), 'Not before (dragged)'),
      v.expected && !tab.hideExpected ? h('span', {}, h('i', { class: 'lg-exp' }), 'Where it was expected') : null),
    unplaced.length ? h('h3', {}, `Not on the Timeline (${unplaced.length})`) : null,
    unplaced.map((t) => {
      const inp = h('input', { class: 'field small', placeholder: 'e.g. 1d', 'aria-label': `Estimate for ${t.key}` });
      inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') inp.blur(); });
      inp.addEventListener('change', () => {
        const m = parseEst(inp.value, day);
        if (m == null) { toast('Write the estimate like 3d, 4h or 1d 4h'); return; }
        app.edit('set_task', { id: t.id, field: 'work', value: m });
        const now = app.view.tasks.find((x) => x.id === t.id);
        if (now?.placed) toast(`${t.key} is on the Timeline: ${dayName(now.start)} – ${dayName(now.finish)}`);
      });
      return h('div', { class: 'tray-item' }, typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key),
        h('button', { class: 'tray-title', onClick: () => app.openTask(t.id) }, t.title), avatar(v, t.who, 22), h('span', { class: 'f-hint' }, 'Estimate'), inp);
    }));

  main.replaceChildren(tl, tray);
  if (!tab.scrolled) { tab.scrolled = true; requestAnimationFrame(() => scrollToDay(v, tab, v.today, false)); }
  if (tab.focusDay) { const d = tab.focusDay; tab.focusDay = null; requestAnimationFrame(() => scrollToDay(v, tab, d, true)); }
}

/// Right-click an issue's row: open it, or put it in a group (an epic).
function rowMenu(e, t, v, app) {
  e.preventDefault();
  const epics = v.tasks.filter((x) => x.epic && x.id !== t.id);
  const put = (parent, name) => { app.edit('set_epic', { id: t.id, parent }); toast(`${t.key} is ${parent ? `in ${name}` : 'in no group'} now`, ['Undo', () => app.edit('undo')]); };
  menu([
    item('Open', { run: () => app.openTask(t.id) }),
    null,
    { head: 'Put it in a group' },
    ...epics.map((g) => item(g.title || g.key, { checked: t.parent === g.id, run: () => put(g.id, g.title || g.key) })),
    item('No group', { checked: !t.parent, run: () => put(null) }),
    item('A new group…', { run: async () => {
      const name = (await ask('A new group', 'Name', '', 'Make it'))?.trim();
      if (!name) return;
      // Made and moved in one step (one Undo).
      app.read('group_start');
      const out = app.read('add_task', { title: name, kind: 'epic' });
      if (out?.id) app.read('set_epic', { id: t.id, parent: out.id });
      app.edit('group_end', { what: `Put ${t.key} in a new group, ${name}` });
      toast(`${t.key} is in ${name} now`, ['Undo', () => app.edit('undo')]);
    } }),
    null,
    deleteItem(app, t),
  ], e.clientX, e.clientY);
}

function barMenu(e, t, app) {
  e.preventDefault();
  menu([
    item('Open', { run: () => app.openTask(t.id) }),
    t.mirror ? item('Go to what it mirrors', { run: () => openRef(t.mirror) }) : false,
    t.held ? item('Start as soon as it can', { run: () => app.edit('hold', { id: t.id }) }) : false,
    null,
    deleteItem(app, t),
  ].filter((x) => x !== false), e.clientX, e.clientY);
}

/// Dragging a bar holds the issue back; dragging its end dot links it.
function drag(bar, t, chart, pos, v, tab, app, DW) {
  bar.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const linking = e.target.classList.contains('dot');
    if ((t.is_done || t.mirror) && !linking) return;
    const x0 = e.clientX;
    let moved = false;
    const ghost = linking ? document.createElementNS(SVG, 'path') : null;
    const svg = chart.querySelector('svg.tl-links');
    if (ghost) { ghost.setAttribute('class', 'drawing'); svg.append(ghost); }
    bar.setPointerCapture(e.pointerId);
    const lane = bar.parentElement;
    const others = [...lane.querySelectorAll('.bar')].filter((b) => b !== bar);
    const move = (ev) => {
      const dx = ev.clientX - x0;
      if (Math.abs(dx) > 3) moved = true;
      if (linking) {
        const r = chart.getBoundingClientRect();
        const p = pos.get(t.id);
        ghost.setAttribute('d', `M${p.x1} ${p.y - HEAD} L${ev.clientX - r.left} ${ev.clientY - r.top - HEAD}`);
      } else if (moved) {
        const snap = Math.round(dx / DW) * DW;
        for (const b of [bar, ...others]) b.style.transform = `translateX(${snap}px)`;
      }
    };
    const up = (ev) => {
      bar.removeEventListener('pointermove', move);
      bar.removeEventListener('pointerup', up);
      bar.removeEventListener('pointercancel', cancel);
      if (linking) {
        ghost.remove();
        const hit = document.elementsFromPoint(ev.clientX, ev.clientY).find((el) => (el.classList?.contains('bar') || el.classList?.contains('ms')) && el !== bar);
        const laneEl = hit?.closest('.tl-lane');
        const top = laneEl ? parseFloat(laneEl.style.top) : null;
        const other = top == null ? null : [...pos.entries()].find(([, p]) => Math.abs(p.y - (top + RH / 2)) < 1)?.[0];
        if (!other || other === t.id) return;
        const o = v.tasks.find((x) => x.id === other);
        const name = (x) => x.key;
        if (o.after.some((d) => d.task === t.id)) { toast(`${name(o)} already starts after ${name(t)}`); return; }
        const here = app.tab.id;
        const link = () => { if (app.edit('link', { tab: here, from: t.id, to: other }) !== null) toast(`${name(o)} now starts after ${name(t)}`, ['Undo', () => app.edit('undo')]); };
        if (o.mirror) { toast(`${o.key} is a mirror: it only shows when the other one is done, so it can’t start after anything`); return; }
        // From a mirror: a loop can run through other plans.
        if (v.on_server && t.mirror) {
          const ref = (x) => (x.mirror || x.id);
          wouldLoop(here, ref(t), ref(o)).then((chain) => {
            if (chain) toast(`That would make a loop: ${chain.map((c) => c.key || c.plan || c.title || '?').join(' → ')}`);
            else link();
          });
          return;
        }
        link();
        return;
      }
      if (!moved) { app.openTask(t.id); return; }
      const shift = Math.round((ev.clientX - x0) / DW);
      for (const b of [bar, ...others]) b.style.transform = '';
      if (!shift) return;
      const target = days(t.start) + shift;
      const today = days(v.today);
      if (shift < 0 && !t.held) {
        toast(`${t.key} already starts as soon as it can: ${t.after.length ? 'when what it starts after is done and ' : ''}${t.who ? 'when its person is free' : 'today'}`);
        return;
      }
      const hold = target > today ? iso(target) : null;
      app.edit('hold', { id: t.id, start: hold });
      const now = app.view.tasks.find((x) => x.id === t.id);
      if (!now) return;
      // A day off moves on to the next working day (the engine does that); later than that: say why.
      const held = now.held || hold;
      if (hold && held !== hold && days(now.start) === days(held)) toast(`${t.key} starts no sooner than ${dayName(held)} (${dayName(hold)} isn’t a working day)`, ['Undo', () => app.edit('undo')]);
      else if (hold && days(now.start) > days(held)) toast(`${t.key} can’t start before ${dayName(now.start)}: ${why(app.view, now, held)}`, ['Undo', () => app.edit('undo')]);
      else toast(hold ? `${t.key} starts no sooner than ${dayName(hold)}` : `${t.key} starts as soon as it can`, ['Undo', () => app.edit('undo')]);
    };
    const cancel = () => {
      bar.removeEventListener('pointermove', move);
      bar.removeEventListener('pointerup', up);
      bar.removeEventListener('pointercancel', cancel);
      ghost?.remove();
      for (const b of [bar, ...others]) b.style.transform = '';
    };
    bar.addEventListener('pointermove', move);
    bar.addEventListener('pointerup', up);
    bar.addEventListener('pointercancel', cancel);
  });
}

/// Why an issue starts later than it was held back to.
function why(v, t, hold) {
  const by = new Map(v.tasks.map((x) => [x.id, x]));
  const blocker = t.after.map((d) => by.get(d.task)).find((b) => b?.placed && days(b.finish) >= days(hold));
  if (blocker) return `it starts after ${blocker.key}`;
  // Group work: the member who got it.
  const who = t.by || t.who;
  const p = who && person(v, who);
  if (!p) return `${dayName(hold)} isn’t a working day`;
  const busy = v.tasks.find((x) => x.id !== t.id && (x.by || x.who) === who && x.placed && x.pieces.some(([a, b]) => days(a) <= days(hold) && days(hold) <= days(b)));
  if (busy) return `${p.name} is on ${busy.key} until then`;
  if ((p.away || []).some(([a, b]) => days(a) <= days(hold) && days(hold) <= days(b))) return `${p.name} is away`;
  return `${dayName(hold)} isn’t a working day for ${p.name}`;
}
