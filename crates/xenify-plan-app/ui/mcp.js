// MCP: AI programs on this computer (a desktop chat app, a coding tool)
// use Plan's tools on the open plans. Off until the person turns it on;
// each program asks the first time; what they may do is one of Read only,
// Read and suggest (suggestions wait in Plan for Apply), or Read and change
// (changes are ordinary edits: one Undo takes each back, History shows the
// program's name, and plans edited together pass them on).
//
// Programs start `xenify-plan --mcp`, which passes their messages to the
// running Plan (src/mcp.rs); requests arrive here as `mcp-request` events.

import { h, toast, modal } from './ui.js';
import { call, tabs as engineTabs } from './engine.js';
import { overview, findTasks, getTask, whoIsBusy, describe, apply, fromProposal, PROPOSAL_SCHEMA } from './actions.js';

const KEEP = { on: 'plan-mcp-in', level: 'plan-mcp-level', allowed: 'plan-mcp-allowed' };
export const LEVELS = [['read', 'Read only', 'Look at tasks, dates and who is busy'], ['propose', 'Read and suggest', 'Changes wait in Plan until you apply them'], ['change', 'Read and change', 'Changes happen straight away; undo works. Deleting issues, or changing more than 10, waits for you']];
const RANK = { read: 0, propose: 1, change: 2 };
const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* none */ } };
const invoke = (name, args = {}) => window.__TAURI__.core.invoke('mcp', { name, args });

const plan = { type: 'string', description: 'A plan name from list_plans (default: the one in front)' };
const OFFERED = [
  ['read', 'list_plans', 'The plans open in Xenify Plan, and which is in front.', {}],
  ['read', 'plan_overview', 'A plan: today, people and their hours, epics, milestones, late issues, and every issue with status, who, estimate and dates (worked out from estimates and links).', { plan }],
  ['read', 'find_tasks', 'Issues whose key, summary, description or person match.', { plan, query: { type: 'string' } }],
  ['read', 'get_task', 'One issue in full: estimate, dates, links, description, comments, recent changes.', { plan, task: { type: 'string', description: 'Key like WEB-6, or the summary' } }, ['task']],
  ['read', 'who_is_busy', 'Each person’s hours booked against hours free, week by week, across all plans.', { plan, from: { type: 'string' }, weeks: { type: 'integer' } }],
  ['propose', 'suggest_changes', 'Suggests changes; the person sees them in Plan and applies them (or not). Waits for their answer.', { plan, ...PROPOSAL_SCHEMA }, ['why']],
  ['change', 'make_changes', 'Makes changes now, as one step (Undo takes it back).', { plan, ...PROPOSAL_SCHEMA }, ['why']],
];

export class Mcp {
  constructor(app) {
    this.app = app;
    this.box = h('div', { class: 'mcp-stack', 'aria-live': 'polite' });
    document.body.append(this.box);
  }

  get desktop() { return !!(window.__TAURI__ && !window.XENIFY_WEB); }

  async boot() {
    if (!this.desktop) return;
    await window.__TAURI__.event.listen('mcp-request', (e) => this.request(e.payload));
    if (this.isOn()) await this.setOn(true);
  }

  isOn() { return load(KEEP.on, false); }
  level() { return load(KEEP.level, 'propose'); }
  setLevel(l) { save(KEEP.level, l); }
  allowed() { return load(KEEP.allowed, {}); }
  forget(program) { const a = this.allowed(); delete a[program]; save(KEEP.allowed, a); }

  async setOn(on) {
    save(KEEP.on, on);
    try { await invoke(on ? 'in_on' : 'in_off'); } catch (e) { toast(String(e)); }
  }

  programs() { return this.desktop ? invoke('in_programs').catch(() => []) : Promise.resolve([]); }
  setup() { return invoke('in_setup'); }

  offered() { return OFFERED.filter(([need]) => RANK[need] <= (RANK[this.level()] ?? 0)); }

  async request(r) {
    const out = (o) => invoke('in_reply', { id: r.id, out: o }).catch(() => {});
    const allowed = this.allowed();
    if (allowed[r.program] === false) return out({ error: { code: -32001, message: 'The person at this computer said no. They can change it in Plan’s Settings › Programs that can use Plan.' } });
    if (allowed[r.program] !== true) {
      const what = LEVELS.find(([k]) => k === this.level())?.[1].toLowerCase();
      const yes = !!(await modal(`${r.program} wants to use Plan`, h('div', {}, h('p', {}, `An AI program on this computer asks to use your open plans (${what}).`),
        h('p', { class: 'muted small' }, 'Allow only programs you started yourself. Settings › Programs that can use Plan changes it later.')), [['Don’t allow', false], ['Allow', true, true]]));
      const a = this.allowed();
      a[r.program] = yes;
      save(KEEP.allowed, a);
      if (!yes) return out({ error: { code: -32001, message: 'The person at this computer said no.' } });
    }
    try {
      if (r.method === 'tools/list') {
        return out({ result: { tools: this.offered().map(([need, name, description, properties, required = []]) => ({ name, description, inputSchema: { type: 'object', properties, required }, annotations: { readOnlyHint: need === 'read' } })) } });
      }
      if (r.method === 'tools/call') {
        const res = await this.tool(r.params?.name, r.params?.arguments || {}, r.program);
        return out({ result: { content: [{ type: 'text', text: JSON.stringify(res) }], structuredContent: res, isError: !!res?.error } });
      }
      return out({ error: { code: -32601, message: `Plan doesn't do ${r.method}` } });
    } catch (e) {
      return out({ result: { content: [{ type: 'text', text: String(e.message || e) }], isError: true } });
    }
  }

  tab(name) {
    const tabs = this.app.tabs;
    if (!name) return engineTabs.front();
    const n = String(name).toLowerCase();
    const t = tabs.find((x) => x.name.toLowerCase() === n) || tabs.find((x) => x.name.toLowerCase().includes(n));
    if (!t) throw new Error(`No open plan called ${name}. Open: ${tabs.map((x) => x.name).join(', ')}`);
    return t.id;
  }

  async tool(name, a, program) {
    if (!this.offered().some(([, n]) => n === name)) return { error: `${name} isn't available: the person allows ${LEVELS.find(([k]) => k === this.level())?.[1].toLowerCase()}` };
    try {
      switch (name) {
        case 'list_plans': return { plans: this.app.tabs.map((t) => t.name), front: this.app.tab?.name };
        case 'plan_overview': return overview(this.tab(a.plan));
        case 'find_tasks': return { tasks: findTasks(this.tab(a.plan), a.query) };
        case 'get_task': return getTask(this.tab(a.plan), a.task);
        case 'who_is_busy': return { load: whoIsBusy(this.tab(a.plan), a.from, a.weeks) };
        case 'make_changes': {
          const tab = this.tab(a.plan);
          // Deleting, or changing many issues at once: the person says yes first, even at this level.
          if (needsAsking(fromProposal(a))) return this.suggestion(program, tab, a, 'Plan asks before deleting issues or changing more than 10 at once.');
          const res = apply(tab, fromProposal(a), `${program}: ${a.why || 'changes'}`.slice(0, 120));
          this.app.render();
          return res;
        }
        case 'suggest_changes': return this.suggestion(program, this.tab(a.plan), a);
        default: return { error: `No tool called ${name}` };
      }
    } catch (e) {
      return { error: String(e.message || e) };
    }
  }

  /// A program's suggestion, waiting in Plan for Apply.
  suggestion(program, tab, a, note = '') {
    const changes = fromProposal(a);
    const lines = describe(tab, changes);
    return new Promise((resolve) => {
      const card = h('div', { class: 'mcp-card', role: 'dialog', 'aria-label': `${program} suggests changes` },
        h('b', {}, `${program} suggests ${changes.length} change${changes.length === 1 ? '' : 's'}`), a.why ? h('p', {}, a.why) : null,
        note ? h('p', { class: 'muted small' }, note) : null,
        h('div', { class: 'mcp-lines' }, lines.slice(0, 12).map(([k, t]) => h('div', {}, h('span', { class: `kind k-${k.toLowerCase()}` }, k), ' ', t))),
        lines.length > 12 ? h('p', { class: 'muted small' }, `and ${lines.length - 12} more`) : null,
        h('div', { class: 'ask-row' },
          h('button', { class: 'btn small', onClick: () => { card.remove(); resolve({ applied: false, note: 'The person dismissed it.' }); } }, 'Dismiss'),
          h('button', { class: 'btn small main', onClick: () => {
            try { const res = apply(tab, changes, `${program}: ${a.why || 'suggested changes'}`.slice(0, 120)); this.app.render(); resolve({ applied: true, ...res }); } catch (e) { resolve({ error: String(e.message || e) }); }
            card.remove();
          } }, 'Apply')));
      this.box.append(card);
    });
  }
}

/// More issues than this changed at once wait for the person, as deletions do.
const MANY = 10;

/// Whether changes delete issues or change many: then the person is asked first.
export function needsAsking(changes) {
  if (changes.some((c) => c.do === 'delete')) return true;
  const touched = new Set(changes.filter((c) => c.do !== 'add_task').map((c) => String(c.task ?? '').toLowerCase()));
  return touched.size > MANY;
}

/// Settings › Programs that can use Plan.
export async function programsSettings(app, mcp) {
  if (!mcp.desktop) { modal('Programs that can use Plan', h('p', {}, 'AI programs connect to Plan on your computer: use the desktop app for this.'), [['OK', true, true]]); return; }
  const on = h('input', { type: 'checkbox', checked: mcp.isOn() });
  const levels = LEVELS.map(([k, label, desc]) => {
    const r = h('input', { type: 'radio', name: 'lv', value: k, checked: mcp.level() === k });
    return h('label', { class: `opt${mcp.level() === k ? ' on' : ''}` }, r, h('span', {}, h('div', { class: 'ot' }, label), h('div', { class: 'od' }, desc)));
  });
  const setup = await mcp.setup().catch(() => null);
  const conf = setup ? JSON.stringify(setup, null, 2) : '';
  const allowed = Object.entries(mcp.allowed());
  const progs = h('div', { class: 'progs' }, allowed.length ? allowed.map(([name, yes]) => h('div', { class: 'res' }, h('b', { class: 'grow' }, name), h('span', { class: 'muted small' }, yes ? 'Allowed' : 'Said no'),
    h('button', { class: 'btn small', onClick: (e) => { mcp.forget(name); e.currentTarget.parentElement.remove(); } }, 'Remove'))) : h('p', { class: 'muted small' }, 'No program has asked yet.'));
  const ok = await modal('Programs that can use Plan', h('div', { class: 'form' },
    h('p', {}, 'AI programs on this computer can read your open plans and suggest or make changes. Every change they make shows in History under their name.'),
    h('label', { class: 'check-row' }, on, 'Let programs connect'),
    h('div', { class: 'opts' }, levels),
    h('p', { class: 'h' }, 'Programs that asked'), progs,
    conf ? h('p', { class: 'h' }, 'Connect a program: add this to its tool servers') : null,
    conf ? h('pre', { class: 'mcp-conf' }, conf) : null,
    conf ? h('button', { class: 'btn small', onClick: () => { navigator.clipboard?.writeText(conf); toast('Copied'); } }, 'Copy') : null,
    h('p', { class: 'muted small' }, 'Plan has to be open. Programs work through this computer, so they also work on servers that keep files unreadable.')),
  [['Cancel', null], ['Save', true, true]]);
  if (!ok) return;
  const lv = levels.map((l) => l.querySelector('input')).find((x) => x.checked)?.value;
  if (lv) mcp.setLevel(lv);
  if (on.checked !== mcp.isOn()) await mcp.setOn(on.checked);
}

void call;
