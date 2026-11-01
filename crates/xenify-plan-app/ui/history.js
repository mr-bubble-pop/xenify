// History (View › History, Ctrl+H): every change to the plan with who made
// it and when, grouped by day, and how the milestones' days moved over time.
// Any change can be gone back before: that's a new change, so it can be
// undone, and nothing is ever rewritten. (Each issue's own changes are on
// its History tab.)

import { h, modal } from './ui.js';
import { days, short, when, dayOf, plural, initials } from './fmt.js';

export function tools() { return []; }

const SVG = 'http://www.w3.org/2000/svg';
const svg = (tag, attrs = {}) => { const el = document.createElementNS(SVG, tag); for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v); return el; };

const KINDS = { all: 'Every kind of change', dates: 'Estimates and dates', people: 'Who does what', status: 'Status', links: 'Links', comments: 'Comments', tasks: 'Issues added, removed, renamed', plan: 'People and the plan’s settings' };

export function render(main, v, tab, app) {
  const all = app.read('history') || [];
  const kind = tab.histKind || 'all';
  const who = tab.histWho || '';
  const people = [...new Set(all.map((e) => e.who).filter(Boolean))];
  const kindSel = h('select', { class: 'field small', 'aria-label': 'Kind of change' }, Object.entries(KINDS).map(([k, l]) => h('option', { value: k, selected: k === kind }, l)));
  kindSel.addEventListener('change', () => { tab.histKind = kindSel.value; app.render(); });
  const whoSel = h('select', { class: 'field small', 'aria-label': 'Who' }, h('option', { value: '' }, 'Everyone'), people.map((p) => h('option', { value: p, selected: p === who }, p)));
  whoSel.addEventListener('change', () => { tab.histWho = whoSel.value; app.render(); });
  const shown = all.filter((e) => (kind === 'all' || e.kinds.includes(kind)) && (!who || e.who === who));

  const list = h('div', { class: 'hist-list' });
  let lastDay = null;
  for (const e of shown.slice(0, 400)) {
    const d = dayOf(e.at);
    if (d !== lastDay) { list.append(h('div', { class: 'hist-day' }, d)); lastDay = d; }
    const task = e.tasks.length === 1 ? v.tasks.find((t) => t.id === e.tasks[0]) : null;
    list.append(h('div', { class: 'hist-item' },
      h('span', { class: 'av', style: { background: 'var(--ink-2)', width: '28px', height: '28px', fontSize: '11px' } }, initials(e.who || '?')),
      h('div', { class: 'hist-text' },
        h('div', {}, h('b', {}, e.who || 'Someone'), h('span', { class: 'when' }, ` ${when(e.at).replace(/^(Today|Yesterday) /, '')}`)),
        h('div', {}, e.what, task ? [' ', h('button', { class: 'linkish small', onClick: () => app.openTask(task.id) }, task.key)] : null)),
      e.index > 0 ? h('button', { class: 'btn plain small', onClick: async () => {
        const ok = await modal('Go back to before this change?', h('p', {}, `The plan is set as it was before “${e.what}”. That is a new change: Undo takes it back, and History keeps everything.`), [['Cancel', null], ['Go back', true, true]]);
        if (ok) app.edit('restore', { index: e.index });
      } }, 'Go back to before') : null));
  }
  if (!shown.length) list.append(h('p', { class: 'f-hint pad' }, 'No changes match.'));

  const all_slips = (app.read('slips') || []).filter((s) => s.points.length);
  const slips = all_slips.filter((s) => new Set(s.points.map((p) => p[1])).size > 1);
  const still = all_slips.filter((s) => !slips.includes(s));
  main.replaceChildren(h('div', { class: 'histpage' },
    h('div', { class: 'hist-head' }, h('h2', {}, 'History'), h('span', { class: 'f-hint' }, plural(all.length, 'change')), h('span', { class: 'grow' }), whoSel, kindSel),
    slips.length ? h('section', { class: 'hist-box' }, h('h3', {}, 'How the milestones moved'), drawSlips(slips),
      still.length ? h('p', { class: 'f-hint' }, `Not moved: ${still.map((m) => `${m.title} ${short(m.points[0][1])}`).join(', ')}`) : null)
      : still.length ? h('p', { class: 'f-hint' }, `No milestone has moved: ${still.map((m) => `${m.title} ${short(m.points[0][1])}`).join(', ')}.`) : null,
    h('section', { class: 'hist-box' }, list)));
}

function drawSlips(slips) {
  const pts = slips.flatMap((s) => s.points);
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => days(p[1]));
  const [x0, x1] = [Math.min(...xs), Math.max(...xs, Math.min(...xs) + 86400000)];
  const [y0, y1] = [Math.min(...ys) - 3, Math.max(...ys) + 3];
  const W = 720, H = 180, L = 70, B = 24;
  const X = (x) => L + ((x - x0) / (x1 - x0)) * (W - L - 110);
  const Y = (y) => (H - B) - ((y - y0) / (y1 - y0)) * (H - B - 10);
  const s = svg('svg', { width: '100%', viewBox: `0 0 ${W} ${H}`, role: 'img', class: 'slips', 'aria-label': slips.map((m) => `${m.title}: ${m.points.map((p) => short(p[1])).join(' then ')}`).join('. ') });
  for (let k = 0; k <= 3; k++) {
    const y = y0 + ((y1 - y0) * k) / 3;
    s.append(svg('line', { x1: String(L), x2: String(W - 110), y1: String(Y(y)), y2: String(Y(y)), class: 'gridline' }));
    const t = svg('text', { x: String(L - 8), y: String(Y(y) + 4), 'text-anchor': 'end', class: 'axis' });
    t.textContent = short(new Date(Math.round(y) * 86400000).toISOString().slice(0, 10));
    s.append(t);
  }
  const COLS = ['var(--series-1)', 'var(--series-2)', 'var(--series-3)', 'var(--series-4)', 'var(--series-5)'];
  slips.slice(0, 5).forEach((m, i) => {
    let d = '';
    m.points.forEach((p, k) => {
      const x = X(p[0]); const y = Y(days(p[1]));
      d += k === 0 ? `M${x} ${y}` : `H${x}V${y}`;
    });
    s.append(svg('path', { d, fill: 'none', stroke: COLS[i], 'stroke-width': '2.5' }));
    const last = m.points[m.points.length - 1];
    const t = svg('text', { x: String(X(last[0]) + 6), y: String(Y(days(last[1])) + 4), class: 'slipname', fill: COLS[i] });
    t.textContent = `${m.title} ${short(last[1])}`;
    s.append(t);
  });
  return s;
}
