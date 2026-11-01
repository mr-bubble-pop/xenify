// Board: a column for each status; drag a card to change its status, type
// at the foot of a column to add an issue there. Epics aren't cards (they
// group issues: see the Backlog). The columns are the plan's own: ⋯ on a
// column (or Issue › Board columns) renames, colours, moves and deletes
// them, and says whether issues there count as started or done.

import { h, icon, toast, menu, item, ask, modal, colourPicker } from './ui.js';
import { typeIcon, prioIcon, assignee, may, epicChip, estPill, shown, keyClass, snippet, checksPill } from './pieces.js';
import { labelChips } from './labels.js';
import { fieldShow } from './fields.js';
import { quickAdd } from './create.js';
import { issueMenu } from './remove.js';

export function render(main, v, tab, app) {
  const wrap = h('div', { class: 'board' });
  for (const s of v.statuses) {
    // Mirrors aren't work here: they're on the Backlog and the Timeline.
    const list = v.tasks.filter((t) => !t.epic && !t.mirror && t.status === s.id && shown(app, t));
    const more = h('button', { class: 'icon-btn col-more', 'aria-label': `${s.name}: column settings`, title: 'Column settings', 'aria-haspopup': 'menu', html: icon('more', 16) });
    more.hidden = !may(v, 'setup');
    more.addEventListener('click', () => { const r = more.getBoundingClientRect(); more.setAttribute('aria-expanded', 'true'); columnMenu(app, s, r.left, r.bottom + 4, () => more.setAttribute('aria-expanded', 'false')); });
    const col = h('section', { class: 'bcol', 'aria-label': s.name },
      h('div', { class: 'bcol-head' }, h('i', { style: { background: s.color || '#A8A29E' } }), s.name, h('span', { class: 'n' }, String(list.length)),
        s.done ? h('span', { class: 'tag-done', title: 'Issues here count as done' }, '✓') : null,
        s.done && list.length && may(v, 'status') ? h('button', { class: 'btn plain small col-release', title: 'Send the done issues out under a name (a version, a launch): they leave the Board, and the Backlog lists them under it', onClick: () => releaseDone(app) }, 'Release…') : null,
        more));
    col.addEventListener('dragover', (e) => { if (e.dataTransfer.types.includes('text/x-plan-task')) { e.preventDefault(); col.classList.add('over'); } });
    col.addEventListener('dragleave', (e) => { if (!col.contains(e.relatedTarget)) col.classList.remove('over'); });
    col.addEventListener('drop', (e) => {
      e.preventDefault();
      col.classList.remove('over');
      const id = e.dataTransfer.getData('text/x-plan-task');
      const t = v.tasks.find((x) => x.id === id);
      if (t && t.status !== s.id) {
        app.edit('set_task', { id, field: 'status', value: s.id });
        toast(`${t.key} is now ${s.name}`);
      }
    });
    for (const t of list) col.append(card(v, t, tab, app));
    col.append(quickAdd(app, { status: s.id }));
    wrap.append(col);
  }
  if (may(v, 'setup')) wrap.append(h('section', { class: 'bcol add-col', 'aria-label': 'Add a column' }, h('button', { class: 'btn plain small', title: 'Add a column', onClick: () => addColumn(app) }, h('span', { html: icon('plus', 14) }), 'Add a column')));
  main.replaceChildren(wrap);
}

function card(v, t, tab, app) {
  const snip = snippet(t.description, 120);
  const onCards = v.fields.filter((f) => f.on_cards && t.fields?.[f.id] != null && (!f.types?.length || f.types.includes(t.kind)));
  const c = h('article', {
    class: `bcard${tab.flash === t.id ? ' flash' : ''}${tab.selected === t.id ? ' sel' : ''}`, draggable: String(may(v, 'status')), tabindex: '0',
    'aria-label': `${t.key} ${t.title}`, 'data-id': t.id,
  },
  h('div', { class: 'sum' }, t.title || h('span', { class: 'faint' }, 'No summary')),
  snip ? h('div', { class: 'snip' }, snip) : null,
  t.parent || t.labels.length ? h('div', { class: 'tags' }, t.parent ? epicChip(v, t.parent) : null, labelChips(t.labels)) : null,
  onCards.length ? h('div', { class: 'fvals' }, onCards.map((f) => h('span', {}, h('span', { class: 'k' }, `${f.name} `), fieldShow(v, f, t.fields[f.id])))) : null,
  h('div', { class: 'foot' },
    typeIcon(v, t.kind), prioIcon(t.priority), h('span', { class: keyClass(t) }, t.key), estPill(v, t), checksPill(t),
    t.late ? h('span', { class: 'flag late', title: `Finishes ${t.late_by} working days after its due date` }, 'Late') : null,
    t.comments.length ? h('span', { class: 'ncomments', title: `${t.comments.length} comments`, html: `${icon('comment', 13)}${t.comments.length}` }) : null,
    h('span', { class: 'grow' }), assignee(v, t)));
  c.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/x-plan-task', t.id); e.dataTransfer.effectAllowed = 'move'; });
  c.addEventListener('click', () => app.openTask(t.id));
  c.addEventListener('contextmenu', (e) => { tab.selected = t.id; issueMenu(e, app, t); });
  c.addEventListener('keydown', (e) => { if (e.key === 'Enter') app.openTask(t.id); });
  c.addEventListener('focus', () => { tab.selected = t.id; });
  return c;
}

// ----- Columns -----

const SAYS = [['todo', 'Not started yet'], ['started', 'Started'], ['done', 'Done']];
const says = (s) => (s.done ? 'done' : s.started ? 'started' : 'todo');
const asSays = (k) => ({ started: k !== 'todo', done: k === 'done' });

function setColumn(app, s, change, what) {
  const { id, ...rest } = { ...s, ...change };
  app.edit('set_item', { list: 'statuses', id, value: rest, what });
}

async function addColumn(app, before = null) {
  const name = await ask('Add a column', 'Name', '', 'Add');
  if (!name || !name.trim()) return;
  // Before Done (new columns are usually a step on the way), unless said.
  const done = app.view.statuses.find((x) => x.done);
  app.edit('set_item', { list: 'statuses', id: '', value: { name: name.trim(), color: '#A8A29E', started: true, done: false }, before: before ?? done?.id ?? null, what: `Added the column ${name.trim()}` });
}

function move(app, s, by) {
  const list = app.view.statuses;
  const i = list.findIndex((x) => x.id === s.id);
  const j = i + by;
  if (j < 0 || j >= list.length) return;
  // Placed before the one after its new place (or at the end).
  const before = by < 0 ? list[j].id : list[j + 1]?.id ?? null;
  const { id, ...rest } = s;
  app.edit('set_item', { list: 'statuses', id, value: rest, before, what: `Moved the column ${s.name}` });
}

/// Done › Release…: every done issue goes out under a name and off the Board.
async function releaseDone(app) {
  const v = app.view;
  const n = v.tasks.filter((t) => t.is_done && !t.epic && !t.release).length;
  const last = v.releases.at(-1)?.name || '';
  // "1.2" suggests "1.3"; anything else, "Release 2".
  const m = last.match(/^(.*?)(\d+)$/);
  const next = m ? `${m[1]}${Number(m[2]) + 1}` : `Release ${v.releases.length + 1}`;
  const name = (await ask(`Release ${n} done ${n === 1 ? 'issue' : 'issues'}`, 'Name (a version or a launch)', next, 'Release'))?.trim();
  if (!name) return;
  if (app.edit('release', { name }) === null) return;
  toast(`Released ${name}: ${n} ${n === 1 ? 'issue' : 'issues'} off the Board (the Backlog lists them under Released)`, ['Undo', () => app.edit('undo')]);
}

async function removeColumn(app, s) {
  const v = app.view;
  const others = v.statuses.filter((x) => x.id !== s.id);
  if (!others.length) { toast('A board needs at least one column'); return; }
  const here = v.tasks.filter((t) => t.status === s.id);
  let to = others[Math.max(0, v.statuses.findIndex((x) => x.id === s.id) - 1)] || others[0];
  let body = h('p', {}, 'No issues are in it.');
  if (here.length) {
    const sel = h('select', { class: 'field' }, others.map((o) => h('option', { value: o.id, selected: o.id === to.id }, o.name)));
    sel.addEventListener('change', () => { to = others.find((o) => o.id === sel.value); });
    body = h('div', { class: 'form' }, h('p', {}, `${here.length} ${here.length === 1 ? 'issue is' : 'issues are'} in it. Move ${here.length === 1 ? 'it' : 'them'} to:`), sel);
  }
  const ok = await modal(`Delete the column “${s.name}”?`, body, [['Cancel', null], ['Delete', true, true]]);
  if (!ok) return;
  app.read('group_start');
  // One by one, as a status change: each then says whether it's started or done there.
  for (const t of here) app.read('set_task', { id: t.id, field: 'status', value: to.id });
  app.read('set_item', { list: 'statuses', id: s.id, value: null });
  app.edit('group_end', { what: `Deleted the column ${s.name}` });
  toast(`Deleted ${s.name}`, ['Undo', () => app.edit('undo')]);
}

function columnMenu(app, s, x, y, onClose) {
  const list = app.view.statuses;
  const i = list.findIndex((c) => c.id === s.id);
  menu([
    item('Rename…', { run: async () => { const n = await ask('Rename the column', 'Name', s.name, 'Rename'); if (n && n.trim() && n.trim() !== s.name) setColumn(app, s, { name: n.trim() }, `Renamed the column ${s.name} to ${n.trim()}`); } }),
    item('Colour…', { run: () => colourPicker(x, y, (c) => setColumn(app, s, { color: c || '#A8A29E' }, `Changed the colour of ${s.name}`), 'Grey') }),
    null,
    { head: 'Issues here count as' },
    ...SAYS.map(([k, n]) => item(n, { checked: says(s) === k, run: () => setColumn(app, s, asSays(k), `${s.name}: counts as ${n.toLowerCase()}`) })),
    null,
    item('Move left', { enabled: i > 0, run: () => move(app, s, -1) }),
    item('Move right', { enabled: i < list.length - 1, run: () => move(app, s, 1) }),
    item('Add a column after this', { run: () => addColumn(app, list[i + 1]?.id ?? null) }),
    null,
    item('Delete the column…', { enabled: list.length > 1, run: () => removeColumn(app, s) }),
  ], x, y, onClose);
}

/// Issue › Board columns: every column in a list, changed in place.
export function columnsDialog(app) {
  const body = h('div', { class: 'cols-list' });
  const draw = () => {
    const v = app.view;
    body.replaceChildren(...v.statuses.map((s, i) => {
      const name = h('input', { class: 'field', value: s.name, 'aria-label': 'Column name' });
      name.addEventListener('change', () => { if (name.value.trim()) { setColumn(app, s, { name: name.value.trim() }, `Renamed the column ${s.name} to ${name.value.trim()}`); draw(); } });
      const kind = h('select', { class: 'field', 'aria-label': 'Issues here count as' }, SAYS.map(([k, n]) => h('option', { value: k, selected: says(s) === k }, n)));
      kind.addEventListener('change', () => { setColumn(app, s, asSays(kind.value), `${s.name}: counts as ${kind.selectedOptions[0].textContent.toLowerCase()}`); draw(); });
      return h('div', { class: 'col-row' },
        h('button', { class: 'swatch', style: { background: s.color || '#A8A29E' }, 'aria-label': 'Colour', title: 'Colour', onClick: (e) => { const r = e.currentTarget.getBoundingClientRect(); colourPicker(r.left, r.bottom + 4, (c) => { setColumn(app, s, { color: c || '#A8A29E' }, `Changed the colour of ${s.name}`); draw(); }, 'Grey'); } }),
        name, kind,
        h('button', { class: 'icon-btn', 'aria-label': 'Move left', title: 'Move left (up here)', disabled: i === 0, onClick: () => { move(app, s, -1); draw(); } }, '↑'),
        h('button', { class: 'icon-btn', 'aria-label': 'Move right', title: 'Move right (down here)', disabled: i === v.statuses.length - 1, onClick: () => { move(app, s, 1); draw(); } }, '↓'),
        h('button', { class: 'icon-btn', 'aria-label': `Delete ${s.name}`, title: 'Delete', disabled: v.statuses.length < 2, html: icon('close', 13), onClick: async () => { await removeColumn(app, s); draw(); } }));
    }), h('button', { class: 'btn plain small', onClick: async () => { await addColumn(app); draw(); } }, '+ Add a column'));
  };
  draw();
  modal('Board columns', h('div', {}, h('p', { class: 'f-hint' }, 'Columns are the steps an issue goes through. “Counts as” tells the Timeline when work has started and when it’s finished.'), body), [['Close', true, true]]);
}
