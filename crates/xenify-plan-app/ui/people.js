// People: who is in the plan and how busy they are, week by week: the hours
// the Timeline gives them (and other plans' work) against the hours they
// can work. Click a week to see what fills it; click a name to change their
// working day or time away.

import { h, icon, sheet, toast } from './ui.js';
import { days, iso, dayName, short, plural, initials } from './fmt.js';
import { typeIcon, estText, keyClass, personColour } from './pieces.js';
import { isServerResource, savePerson } from './serverplan.js';

// People are the server's: its accounts, with their hours and time away.
export function tools() {
  return [];
}

const hours = (m) => `${Math.round((m / 60) * 10) / 10}`;

export function render(main, v, tab, app) {
  const today = days(v.today);
  const monday = iso(today - ((today + 3) % 7));
  const rows = app.read('load', { from: monday, weeks: 6 }) || [];
  const wrap = h('div', { class: 'people-view' });
  if (!v.on_server) {
    wrap.append(h('div', { class: 'empty-state' },
      h('h2', {}, 'Connect to a server to plan people'),
      h('p', {}, 'The people who do the work, their hours and their time away come from the Xenify server, so every plan adds up the same people’s time. This is a copy of a plan: put it on a server to give its issues to people.'),
      h('button', { class: 'btn main', onClick: () => app.saveToServer?.() }, 'Put it on a server…')));
    main.replaceChildren(wrap);
    return;
  }
  if (!rows.length) {
    wrap.append(h('div', { class: 'empty-state' },
      h('h2', {}, 'No one on the server yet'),
      h('p', {}, 'People are the server’s accounts. Once someone has an account (Admin › People, or Invite someone in Settings › Server), they’re here: give them issues, and the Timeline plans their work in their hours, around their time away.')));
    main.replaceChildren(wrap);
    return;
  }
  const weeks = rows[0].cells.map((c) => c.week);
  // People with work in this plan (and me) unless asked for everyone: a
  // server can have hundreds of accounts.
  const inPlan = new Set([...v.tasks.flatMap((t) => [t.who, t.by]), v.me_resource].filter(Boolean));
  for (const g of v.groups || []) if (inPlan.has(g.id)) for (const m of g.members) inPlan.add(m.person);
  const q = (tab.peopleFind || '').toLowerCase();
  const shownRows = rows
    .filter((r) => (tab.peopleAll ? !q || (r.name || '').toLowerCase().includes(q) : inPlan.has(r.id)))
    .sort((a, b) => (a.name || '').localeCompare(b.name || '', undefined, { numeric: true, sensitivity: 'base' }));
  const find = h('input', { class: 'field small', type: 'search', placeholder: 'Find someone', 'aria-label': 'Find someone', value: tab.peopleFind || '' });
  find.addEventListener('input', () => { tab.peopleFind = find.value; app.render(); const i = document.querySelector('.pp-bar input'); i?.focus(); i?.setSelectionRange(i.value.length, i.value.length); });
  wrap.append(h('div', { class: 'pp-bar' },
    h('button', { class: `chipbtn${tab.peopleAll ? '' : ' on'}`, 'aria-pressed': String(!tab.peopleAll), onClick: () => { tab.peopleAll = false; app.render(); } }, `In this plan · ${rows.filter((r) => inPlan.has(r.id)).length}`),
    h('button', { class: `chipbtn${tab.peopleAll ? ' on' : ''}`, 'aria-pressed': String(!!tab.peopleAll), onClick: () => { tab.peopleAll = true; app.render(); } }, `Everyone · ${rows.length}`),
    tab.peopleAll ? find : null));
  if (!shownRows.length) wrap.append(h('p', { class: 'f-hint' }, tab.peopleAll ? 'Nobody by that name.' : 'Nobody has work in this plan yet. Give issues to people, or look at Everyone.'));
  const table = h('table', { class: 'pp-table' },
    h('thead', {}, h('tr', {}, h('th', {}, 'Person'), weeks.map((w) => h('th', {}, `Week of ${short(w)}`)))),
    h('tbody', {}, shownRows.map((r) => {
      const p = v.resources.find((x) => x.id === r.id) || { id: r.id, name: r.name };
      return h('tr', {},
        h('td', {}, h('button', { class: 'pp-who', title: 'Change their working day or time away', onClick: () => personDialog(app, p) },
          h('span', { class: 'av', style: { background: personColour(v, p.id), width: '30px', height: '30px', fontSize: '12px' } }, initials(p.name)),
          h('span', {}, h('b', {}, p.name), h('small', {}, `${hours(r.day)} h a day${(p.away || []).some(([, b]) => days(b) >= today) ? ' · time away' : ''}`)))),
        r.cells.map((c) => {
          const pct = c.have ? c.booked / c.have : c.booked ? 2 : 0;
          const sel = tab.week?.who === r.id && tab.week?.week === c.week;
          const cls = pct > 1.001 ? ' over' : '';
          return h('td', {}, h('button', {
            class: `load${cls}${sel ? ' sel' : ''}`, 'aria-pressed': String(sel),
            'aria-label': `${p.name}, week of ${short(c.week)}: ${hours(c.booked)} of ${hours(c.have)} hours`,
            onClick: () => { tab.week = sel ? null : { who: r.id, week: c.week }; app.render(); },
          },
          h('span', { class: 'track' }, h('i', { style: { width: `${Math.min(100, pct * 100)}%` } })),
          h('span', { class: 'nums' }, c.have ? `${hours(c.booked)} of ${hours(c.have)} h` : c.booked ? `${hours(c.booked)} h, away` : 'Away',
            c.away && c.have ? ` · ${plural(c.away, 'day')} off` : '')));
        }));
    })));
  wrap.append(h('div', { class: 'pp-box' }, table));
  if (tab.week) {
    const { who, week } = tab.week;
    const p = v.resources.find((x) => x.id === who);
    const items = app.read('week_detail', { resource: who, week }) || [];
    wrap.append(h('section', { class: 'pp-detail' },
      h('h3', {}, `${p?.name || 'Someone'}, week of ${short(week)}`),
      items.length ? items.map((it) => {
        const t = it.task ? v.tasks.find((x) => x.id === it.task) : null;
        return h(t ? 'button' : 'div', { class: 'pp-item', onClick: t ? () => app.openTask(t.id) : null },
          t ? typeIcon(v, t.kind) : h('span', { class: 'ty other', title: 'Another plan' }),
          t ? h('span', { class: keyClass(t) }, t.key) : h('span', { class: 'key' }, it.plan || 'Another plan'),
          h('span', { class: 'pp-title' }, it.title),
          h('span', { class: 'pp-when' }, `${it.from === it.to ? dayName(it.from) : `${dayName(it.from)} – ${dayName(it.to)}`} · ${hours(it.minutes)} h`));
      }) : h('p', { class: 'f-hint' }, 'Nothing planned this week.'),
      (p?.away || []).filter(([a, b]) => days(b) >= days(week) && days(a) <= days(week) + 6).map(([a, b]) => h('p', { class: 'f-hint' }, `Away ${dayName(a)} – ${dayName(b)}`))));
  }
  const nobody = v.tasks.filter((t) => !t.epic && !t.mirror && !t.milestone && !t.who && !t.is_done);
  if (nobody.length) {
    wrap.append(h('section', { class: 'pp-detail' },
      h('h3', {}, `No one yet (${nobody.length})`),
      nobody.map((t) => h('button', { class: 'pp-item', onClick: () => app.openTask(t.id) }, typeIcon(v, t.kind), h('span', { class: keyClass(t) }, t.key), h('span', { class: 'pp-title' }, t.title),
        h('span', { class: 'pp-when' }, t.work == null ? 'No estimate' : estText(t.work, v.settings.day_minutes))))));
  }
  main.replaceChildren(wrap);
}

/// Adding someone (`p` empty) or changing their working day and time away.
/// Someone's working day and time away (kept on the server, for every plan).
export function personDialog(app, p) {
  const v = app.view;
  if (!p || !isServerResource(p.id)) return;
  const name = h('input', { class: 'field', id: 'p-name', value: p.name || '', autocomplete: 'off', disabled: true });
  const day = h('input', { class: 'field', id: 'p-hours', value: hours(p?.day_minutes || v.settings.day_minutes), inputmode: 'decimal' });
  let away = (p?.away || []).map(([a, b]) => [a, b]);
  const awayList = h('div', { class: 'away-list' });
  const from = h('input', { class: 'field', type: 'date', 'aria-label': 'First day away' });
  const to = h('input', { class: 'field', type: 'date', 'aria-label': 'Last day away' });
  const err = h('div', { class: 'f-error', role: 'alert' });
  const drawAway = () => awayList.replaceChildren(...(away.length ? away.map(([a, b], i) => h('div', { class: 'away-row' },
    h('span', {}, a === b ? dayName(a) : `${dayName(a)} – ${dayName(b)}`),
    h('button', { class: 'icon-btn small', 'aria-label': `Remove ${dayName(a)}`, html: icon('close', 14), onClick: () => { away.splice(i, 1); drawAway(); } })))
    : [h('p', { class: 'f-hint' }, 'None')]));
  drawAway();
  const addAway = () => {
    if (!from.value) { err.textContent = 'Pick the first day away.'; return; }
    const b = to.value && to.value >= from.value ? to.value : from.value;
    away.push([from.value, b]);
    away.sort((x, y) => x[0].localeCompare(y[0]));
    from.value = ''; to.value = ''; err.textContent = '';
    drawAway();
  };
  const save = async () => {
    const n = p.name;
    const hrs = parseFloat(day.value.replace(',', '.'));
    if (!(hrs > 0 && hrs <= 24)) { err.textContent = 'Hours a day: a number like 7.5'; day.focus(); return; }
    if (from.value) addAway();
    const day_minutes = Math.round(hrs * 60) === v.settings.day_minutes ? 0 : Math.round(hrs * 60);
    try { await savePerson(p.id, { day_minutes, country: p.country || '', region: p.region || '', away }); } catch (e) { err.textContent = String(e.message || e); return; }
    close();
    toast(`Saved ${n}`);
  };
  const box = h('div', { class: 'dlg narrow', role: 'dialog', 'aria-modal': 'true', 'aria-label': p.name },
    h('header', {}, h('h2', {}, p.name), h('span', { class: 'grow' }), h('button', { class: 'icon-btn', 'aria-label': 'Close', html: icon('close', 16), onClick: () => close() })),
    h('div', { class: 'dlg-body' },
      h('div', { class: 'f' }, h('label', { class: 'f-label', for: 'p-name' }, 'Name'), name, h('span', { class: 'f-hint' }, 'Their name comes from their account on the server.')),
      h('div', { class: 'f' }, h('label', { class: 'f-label', for: 'p-hours' }, 'Hours they work a day'), day, h('span', { class: 'f-hint' }, 'Their issues fill this many hours each working day.')),
      h('div', { class: 'f' }, h('span', { class: 'f-label' }, 'Time away'), awayList,
        h('div', { class: 'away-add' }, from, h('span', { class: 'f-hint' }, 'to'), to, h('button', { class: 'btn small', onClick: addAway }, 'Add')),
        h('span', { class: 'f-hint' }, 'The same as their out of office in Chat.')),
      err),
    h('footer', {},
      h('span', { class: 'grow' }),
      h('button', { class: 'btn plain', onClick: () => close() }, 'Cancel'),
      h('button', { class: 'btn main', onClick: save }, 'Save')));
  box.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.tagName === 'INPUT' && e.target.type !== 'date') { e.preventDefault(); save(); } });
  const close = sheet(box);
  setTimeout(() => day.focus(), 20);
}
