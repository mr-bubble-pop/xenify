// Deleting issues, the same from everywhere (the issue's ⋯ menu, a right-click
// on a card, a row or a bar, the Delete key, Issue › Delete). An issue goes
// at once with Undo; an epic with issues in it asks first whether they go
// too or stay, moved out to where the epic was.

import { h, modal, toast, menu, item } from './ui.js';
import { plural } from './fmt.js';
import { may } from './pieces.js';
import { closeTask } from './issue.js';

/// Whether the Delete item is offered as usable.
export const mayDelete = (app) => may(app.view, 'delete');

export async function deleteIssue(app, id) {
  const v = app.view;
  const t = v?.tasks.find((x) => x.id === id);
  if (!t) return;
  if (!may(v, 'delete')) { toast('Your access to this plan doesn’t allow deleting issues'); return; }
  const inside = v.tasks.filter((x) => x.parent === t.id);
  let keep = false;
  if (t.epic && inside.length) {
    const upName = t.parent ? v.tasks.find((x) => x.id === t.parent)?.title : null;
    const body = h('p', {}, `It has ${plural(inside.length, 'issue')} in it. Keep ${inside.length === 1 ? 'it' : 'them'}`,
      upName ? ` (${inside.length === 1 ? 'it goes' : 'they go'} into ${upName})` : ` (${inside.length === 1 ? 'it’s' : 'they’re'} then in no epic)`,
      `, or delete ${inside.length === 1 ? 'it' : 'them'} with the epic?`);
    const a = await modal(`Delete the epic “${t.title || t.key}”?`, body,
      [['Cancel', null], [`Delete ${inside.length === 1 ? 'it' : 'them'} too`, 'all'], [`Keep ${inside.length === 1 ? 'the issue' : 'the issues'}`, 'keep', true]]);
    if (!a) return;
    keep = a === 'keep';
  }
  const all = keep ? 0 : app.view.tasks.filter((x) => x.id !== t.id && isIn(v, x, t.id)).length;
  if (app.tab?.selected === id) app.tab.selected = null;
  closeTask();
  app.edit('delete_tasks', { ids: [id], keep });
  const what = keep && inside.length ? `Deleted ${t.key}; ${plural(inside.length, 'issue')} kept`
    : `Deleted ${t.key}${all ? ` and ${plural(all, 'issue')} in it` : ` ${t.title || ''}`}`;
  toast(what.trim(), ['Undo', () => app.edit('undo')]);
}

function isIn(v, x, id) {
  for (let p = x.parent, n = 0; p && n < 50; n++) {
    if (p === id) return true;
    p = v.tasks.find((y) => y.id === p)?.parent;
  }
  return false;
}

/// Right-click on a card or a row: open it or delete it.
export function issueMenu(e, app, t, extra = []) {
  e.preventDefault();
  e.stopPropagation();
  menu([
    item('Open', { run: () => app.openTask(t.id) }),
    ...extra,
    null,
    deleteItem(app, t),
  ], e.clientX, e.clientY);
}

export const deleteItem = (app, t) => item(t.mirror ? 'Delete the mirror' : t.epic ? 'Delete the epic…' : 'Delete the issue', { keys: 'Delete', enabled: mayDelete(app), run: () => deleteIssue(app, t.id) });
