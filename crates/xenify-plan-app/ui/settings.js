// Plan settings (the working week and day, public holidays) and the app's
// own (which person in the plan you are).

import { h, modal, menu, item } from './ui.js';

/// Which of the plan's people you are (for Only my issues): a short pick
/// list, nothing to type. On a server the server says who you are.
export function whichPerson(app, x, y, then) {
  const v = app.view;
  const people = v.resources.filter((r) => (r.kind || 'person') === 'person');
  menu([{ head: 'Which of these is you?' }, ...people.map((p) => item(p.name, { checked: p.id === v.me_resource, run: () => {
    app.prefs.meResource = p.id;
    app.savePrefs();
    app.read('set_me', { name: '', resource: p.id });
    then?.();
    app.render();
  } }))], x, y);
}

export async function planSettings(app) {
  const v = app.view;
  const s = v.settings;
  const list = app.read('countries') || [];
  const hours = h('input', { class: 'field num', type: 'number', step: '0.5', value: String(s.day_minutes / 60) });
  const usual = h('select', { class: 'field' }, [[0, 'None (estimate each one)'], [0.5, 'Half a day'], [1, '1 day'], [2, '2 days'], [3, '3 days'], [5, '5 days']]
    .map(([d, label]) => h('option', { value: String(d), selected: d === (s.default_days ?? 1) }, label)));
  const country = h('select', { class: 'field' }, h('option', { value: '' }, 'None'), list.map((c) => h('option', { value: c.code, selected: c.code === s.country }, c.name)));
  const region = h('select', { class: 'field' });
  const fill = () => { const c = list.find((x) => x.code === country.value); region.replaceChildren(h('option', { value: '' }, 'All of it'), (c?.regions || []).map(([k, n]) => h('option', { value: k, selected: k === s.region }, n))); };
  country.addEventListener('change', fill); fill();
  const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((d, i) => { const c = h('input', { type: 'checkbox', checked: s.work_days[i] }); return [c, h('label', { class: 'check-row' }, c, d)]; });
  const ok = await modal('Plan settings', h('div', { class: 'form' },
    h('label', {}, 'Working week', h('div', { class: 'days' }, days.map((d) => d[1]))),
    h('label', {}, 'Hours in a working day (a “1d” estimate)', hours),
    h('label', {}, 'New issues start with an estimate of', usual),
    h('div', { class: 'two' }, h('label', {}, 'Holidays', country), h('label', {}, 'Region', region))), [['Cancel', null], ['Save', true, true]]);
  if (!ok) return;
  const set = (field, value) => { if (JSON.stringify(s[field]) !== JSON.stringify(value)) app.read('set_plan', { field: `settings.${field}`, value }); };
  // One Save, one step (one Undo).
  app.read('group_start');
  set('day_minutes', Math.round((parseFloat(hours.value) || 7.5) * 60)); set('country', country.value); set('region', region.value);
  set('work_days', days.map((d) => d[0].checked)); set('default_days', parseFloat(usual.value));
  app.read('group_end', { what: 'Changed the plan’s settings' });
  app.prefs.country = country.value; app.prefs.region = region.value; app.savePrefs();
  app.render();
}
