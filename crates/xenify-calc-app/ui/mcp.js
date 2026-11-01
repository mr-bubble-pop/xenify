// MCP (the Model Context Protocol), both ways, on the desktop.
//
// Connecting in: AI programs on this computer (a desktop chat app, a coding
// tool) use Calc's tools on the open books. Off until the person turns it
// on; each program asks the first time; what they may do is one of Read
// only, Read and propose (proposals wait in Calc for Add it), or Read and
// change (changes are ordinary edits: Undo takes them back, and books
// edited together pass them on).
//
// In the program: Ask can use tools from MCP servers the person adds here
// (a command, or an address) and ones the server's admin adds for everyone.
// Ask asks before using any of these tools (showing what it sends), until
// the person ticks "Don't ask again" for that tool; Settings lists those.

import { h, toast, modal } from './ui.js';

const KEEP = { on: 'calc-mcp-in', level: 'calc-mcp-level', allowed: 'calc-mcp-allowed', tools: 'calc-mcp-tools', trust: 'calc-mcp-trust' };
export const LEVELS = [['read', 'Read only'], ['propose', 'Read and propose changes'], ['change', 'Read and change']];

const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} };
const invoke = (name, args = {}) => window.__TAURI__.core.invoke('mcp', { name, args });
const slug = (s) => String(s).toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '').slice(0, 24) || 'tool';

// The tools Calc offers to programs connecting in, by the level they need.
const OFFERED = [
  ['read', 'list_books', 'The books open in Xenify Calc (names), and which one is in front.', {}],
  ['read', 'describe_book', 'The sheets, tables (with their columns) and definitions of a book.', { book: { type: 'string', description: 'A book name from list_books (default: the one in front)' } }],
  ['read', 'look_at_columns', 'Each column of a table or sheet: what it holds, examples, totals for numbers, and its note.', { book: { type: 'string' }, table: { type: 'string' }, sheet: { type: 'string' } }],
  ['read', 'look_at_rows', 'Some rows of a table or sheet, as shown (at most 50).', { book: { type: 'string' }, table: { type: 'string' }, sheet: { type: 'string' }, from: { type: 'integer' }, count: { type: 'integer' } }],
  ['read', 'calculate', 'Works out a spreadsheet formula on a book (nothing changes).', { book: { type: 'string' }, formula: { type: 'string' }, sheet: { type: 'string' } }, ['formula']],
  ['propose', 'propose_cells', 'Proposes new contents for some cells. The person sees them in Calc and adds them (or not).', {
    book: { type: 'string' }, sheet: { type: 'string', description: 'Sheet name (default: the first)' },
    cells: { type: 'array', items: { type: 'object', properties: { cell: { type: 'string', description: 'e.g. B2' }, value: { type: 'string', description: 'What to type there (a formula starts with =)' } }, required: ['cell', 'value'] } },
    why: { type: 'string', description: 'One sentence the person sees' },
  }, ['cells']],
  ['change', 'set_cells', 'Types into some cells (as the person would; Undo takes it back).', {
    book: { type: 'string' }, sheet: { type: 'string' },
    cells: { type: 'array', items: { type: 'object', properties: { cell: { type: 'string' }, value: { type: 'string' } }, required: ['cell', 'value'] } },
  }, ['cells']],
];
const RANK = { read: 0, propose: 1, change: 2 };
/// More cells than this typed at once by a program wait for the person.
const MANY = 10;

export class Mcp {
  constructor(d) {
    this.d = d;   // { call, run, tabs: () => [{id, title}], frontTab, sheet, changed, desktop }
    this.box = h('div', { class: 'mcp-stack', 'aria-live': 'polite' });
    document.body.append(this.box);
    this.started = new Map(); // tool servers Ask uses: name -> { tools, kind, session }
  }

  get desktop() { return !!(this.d.desktop && window.__TAURI__); }

  // ----- Connecting in -----

  async boot() {
    if (!this.desktop) return;
    await window.__TAURI__.event.listen('mcp-request', (e) => this.request(e.payload));
    await window.__TAURI__.event.listen('mcp-programs', () => this.onPrograms?.());
    if (load(KEEP.on, false)) await this.setOn(true);
  }

  isOn() { return load(KEEP.on, false); }
  level() { return load(KEEP.level, 'propose'); }
  setLevel(l) { save(KEEP.level, l); }

  async setOn(on) {
    save(KEEP.on, on);
    try { await invoke(on ? 'in_on' : 'in_off'); } catch (e) { toast(String(e)); }
  }

  programs() { return this.desktop ? invoke('in_programs').catch(() => []) : Promise.resolve([]); }
  stopProgram(id) { return invoke('in_drop', { id }); }
  setup() { return invoke('in_setup'); }

  /// A request from a program: allowed? then answered.
  async request(r) {
    const out = (o) => invoke('in_reply', { id: r.id, out: o }).catch(() => {});
    const allowed = load(KEEP.allowed, {});
    if (allowed[r.program] === false) return out({ error: { code: -32001, message: 'The person at this computer said no. They can change it in Calc’s AI › Tools for AI (MCP).' } });
    if (allowed[r.program] !== true) {
      // Several requests at once from a new program: one question.
      this.asking ||= new Map();
      if (!this.asking.has(r.program)) {
        this.asking.set(r.program, this.askFirst(r.program).then((yes) => {
          this.asking.delete(r.program);
          const a = load(KEEP.allowed, {});
          a[r.program] = yes;
          save(KEEP.allowed, a);
          this.onPrograms?.();
          return yes;
        }));
      }
      const yes = await this.asking.get(r.program);
      if (!yes) return out({ error: { code: -32001, message: 'The person at this computer said no.' } });
    }
    try {
      if (r.method === 'tools/list') return out({ result: { tools: this.offered().map(([, name, description, properties, required = []]) => ({ name, description, inputSchema: { type: 'object', properties, required }, annotations: { readOnlyHint: name !== 'set_cells' && name !== 'propose_cells' } })) } });
      if (r.method === 'tools/call') {
        const res = await this.tool(r.params?.name, r.params?.arguments || {}, r.program);
        return out({ result: { content: [{ type: 'text', text: JSON.stringify(res) }], structuredContent: res, isError: !!res?.error } });
      }
      return out({ error: { code: -32601, message: `Calc doesn't do ${r.method}` } });
    } catch (e) {
      return out({ result: { content: [{ type: 'text', text: String(e.message || e) }], isError: true } });
    }
  }

  offered() {
    const l = RANK[this.level()] ?? 0;
    return OFFERED.filter(([need]) => RANK[need] <= l);
  }

  askFirst(program) {
    const what = LEVELS.find(([k]) => k === this.level())?.[1].toLowerCase();
    return modal(`${program} wants to use Calc`, h('div', { class: 'note-form' },
      h('p', {}, `An AI program on this computer asks to use your open books (${what}).`),
      h('p', { class: 'q-hint' }, 'Allow only programs you started yourself. You can stop it, or change what programs may do, in AI › Tools for AI (MCP).')),
    [['Don’t allow', false], ['Allow', true, true]]).then((v) => !!v);
  }

  /// Which book: by name, else the front one (its tab).
  tab(book) {
    const tabs = this.d.tabs();
    if (!book) return this.d.frontTab();
    const t = tabs.find((x) => x.title.toLowerCase() === String(book).toLowerCase()) || tabs.find((x) => x.title.toLowerCase().includes(String(book).toLowerCase()));
    if (!t) throw new Error(`No open book called ${book}. Open: ${tabs.map((x) => x.title).join(', ')}`);
    return t.id;
  }

  sheetId(tab, name) {
    const sheets = this.d.call('sheets', { tab });
    if (!name) return sheets.find((s) => !s.working)?.id ?? sheets[0].id;
    const s = sheets.find((x) => x.name.toLowerCase() === String(name).toLowerCase());
    if (!s) throw new Error(`No sheet called ${name}: ${sheets.map((x) => x.name).join(', ')}`);
    return s.id;
  }

  cells(list) {
    return (list || []).map((x) => {
      const m = /^\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(String(x.cell).trim());
      if (!m) throw new Error(`${x.cell} isn't a cell like B2`);
      const col = [...m[1].toUpperCase()].reduce((a, ch) => a * 26 + ch.charCodeAt(0) - 64, 0);
      return { row: Number(m[2]), col, data: String(x.value ?? '') === '' ? null : { i: String(x.value) } };
    });
  }

  async tool(name, a, program) {
    if (!this.offered().some(([, n]) => n === name)) return { error: `${name} isn't available: the person allows ${LEVELS.find(([k]) => k === this.level())?.[1].toLowerCase()}` };
    const { call } = this.d;
    switch (name) {
      case 'list_books': return { books: this.d.tabs().map((t) => t.title), front: this.d.tabs().find((t) => t.id === this.d.frontTab())?.title };
      case 'describe_book': return call('ai_describe', { tab: this.tab(a.book) });
      case 'look_at_columns': return call('ai_columns', { tab: this.tab(a.book), table: a.table, sheet: a.sheet });
      case 'look_at_rows': return call('ai_rows', { tab: this.tab(a.book), table: a.table, sheet: a.sheet, from: a.from || 0, count: Math.min(a.count || 20, 50) });
      case 'calculate': return call('ai_calc', { tab: this.tab(a.book), formula: a.formula, sheet: a.sheet });
      case 'set_cells': {
        const tab = this.tab(a.book);
        const sheet = this.sheetId(tab, a.sheet);
        const cells = this.cells(a.cells);
        // Many cells at once: the person says yes first, even at this level.
        if (cells.length > MANY) {
          const title = this.d.tabs().find((t) => t.id === tab)?.title;
          return this.proposal(program, title, { ...a, why: `${a.why ? `${a.why} ` : ''}Calc asks before more than ${MANY} cells change at once.` }, () => { call('edit', { tab, ops: [{ op: 'cells', sheet, cells }] }); this.d.changed(); });
        }
        call('edit', { tab, ops: [{ op: 'cells', sheet, cells }] });
        this.d.changed();
        return { done: true, cells: a.cells.length };
      }
      case 'propose_cells': {
        const tab = this.tab(a.book);
        const sheet = this.sheetId(tab, a.sheet);
        const cells = this.cells(a.cells);
        const title = this.d.tabs().find((t) => t.id === tab)?.title;
        return this.proposal(program, title, a, () => { call('edit', { tab, ops: [{ op: 'cells', sheet, cells }] }); this.d.changed(); });
      }
      default: return { error: `No tool called ${name}` };
    }
  }

  /// A program's proposal, waiting in Calc for Add it.
  proposal(program, book, a, apply) {
    return new Promise((resolve) => {
      const card = h('div', { class: 'mcp-card', role: 'dialog', 'aria-label': `${program} proposes a change` },
        h('b', {}, `${program} proposes`), a.why ? h('p', {}, a.why) : null,
        h('div', { class: 'mcp-cells' }, a.cells.slice(0, 12).map((x) => h('div', {}, h('code', {}, x.cell), ' ', String(x.value)))),
        a.cells.length > 12 ? h('p', { class: 'q-hint' }, `and ${a.cells.length - 12} more`) : null,
        h('p', { class: 'q-hint' }, `In ${book}${a.sheet ? ` › ${a.sheet}` : ''}`),
        h('div', { class: 'ask-row' },
          h('button', { class: 'btn small', onClick: () => { card.remove(); resolve({ added: false, note: 'The person dismissed it.' }); } }, 'Dismiss'),
          h('button', { class: 'btn small main', onClick: () => { try { apply(); resolve({ added: true }); } catch (e) { resolve({ error: String(e.message || e) }); } card.remove(); } }, 'Add it')));
      this.box.append(card);
    });
  }

  // ----- Tools Ask can use -----

  /// The tool servers added on this computer.
  mine() { return load(KEEP.tools, []); }
  setMine(list) { save(KEEP.tools, list); this.started.clear(); }

  /// Reads a setup pasted from another program: {"mcpServers": {name: {command, args, env} | {url, headers}}}.
  parseSetup(text) {
    let v;
    try { v = JSON.parse(text); } catch { throw new Error('That isn’t a setup Calc can read (JSON with "mcpServers").'); }
    const servers = v.mcpServers || v.servers || v;
    const out = [];
    for (const [name, s] of Object.entries(servers || {})) {
      if (s?.command) out.push({ name, kind: 'command', command: s.command, args: s.args || [], env: s.env || {} });
      else if (s?.url) out.push({ name, kind: 'url', url: s.url, headers: s.headers || {} });
    }
    if (!out.length) throw new Error('No tool servers found in it.');
    return out;
  }

  /// Every tool Ask may use: [{ name (for the model), server, tool, description, inputSchema, readOnly, via }].
  async askTools(server) {
    const out = [];
    const add = (srvName, via, tools) => {
      for (const t of tools || []) {
        out.push({ name: `${slug(srvName)}__${slug(t.name)}`.slice(0, 64), server: srvName, tool: t.name, description: `${srvName}: ${t.description || t.name}`, inputSchema: t.inputSchema || { type: 'object', properties: {} }, readOnly: t.annotations?.readOnlyHint === true, via });
      }
    };
    // From the server, for everyone.
    if (server?.session) {
      try {
        for (const s of await server.api('GET', '/v1/mcp')) {
          const key = `server:${s.id}`;
          if (!this.started.has(key)) {
            await this.serverRpc(server, s.id, 'initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'Xenify Calc', version: '1' } });
            const r = await this.serverRpc(server, s.id, 'tools/list', {});
            this.started.set(key, { tools: r.tools });
          }
          add(s.name, { kind: 'server', id: s.id }, this.started.get(key).tools);
        }
      } catch {}
    }
    if (!this.desktop) return out;
    for (const s of this.mine()) {
      try {
        if (!this.started.has(s.name)) {
          if (s.kind === 'command') {
            const r = await invoke('out_start', { server: s.name, command: s.command, args: s.args, env: s.env });
            this.started.set(s.name, { tools: r.tools });
          } else {
            const init = await invoke('out_http', { url: s.url, headers: s.headers, body: { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'Xenify Calc', version: '1' } } } });
            const session = init.session || null;
            await invoke('out_http', { url: s.url, headers: s.headers, session, body: { jsonrpc: '2.0', method: 'notifications/initialized' } }).catch(() => {});
            const r = await invoke('out_http', { url: s.url, headers: s.headers, session, body: { jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} } });
            this.started.set(s.name, { tools: r.message?.result?.tools || [], session });
          }
        }
        add(s.name, { kind: s.kind, s }, this.started.get(s.name).tools);
      } catch (e) {
        toast(`${s.name} didn’t start: ${e.message || e}`);
      }
    }
    return out;
  }

  async serverRpc(server, id, method, params) {
    const r = await server.api('POST', `/v1/mcp/${id}`, { jsonrpc: '2.0', id: Date.now(), method, params });
    if (r?.error) throw new Error(r.error.message || 'the tool said no');
    return r?.result ?? r;
  }

  /// Calls tool `t` (from askTools) with `args`: its result as text.
  async callTool(server, t, args) {
    const params = { name: t.tool, arguments: args };
    let res;
    if (t.via.kind === 'server') res = await this.serverRpc(server, t.via.id, 'tools/call', params);
    else if (t.via.kind === 'command') res = await invoke('out_call', { server: t.server, method: 'tools/call', params });
    else {
      const st = this.started.get(t.server);
      const r = await invoke('out_http', { url: t.via.s.url, headers: t.via.s.headers, session: st?.session, body: { jsonrpc: '2.0', id: Date.now(), method: 'tools/call', params } });
      if (r.message?.error) throw new Error(r.message.error.message);
      res = r.message?.result;
    }
    const text = (res?.content || []).map((c) => (c.type === 'text' ? c.text : `[${c.type}]`)).join('\n');
    return { text: text || JSON.stringify(res?.structuredContent ?? res ?? null), isError: !!res?.isError };
  }

  /// The tools Ask may use without asking (the person ticked "Don't ask again").
  trusted() { return load(KEEP.trust, {}); }
  untrust(name) { const tr = this.trusted(); delete tr[name]; save(KEEP.trust, tr); }

  /// Whether Ask may use `t` now with `args`: asks, showing what it sends,
  /// unless the person said not to ask again for this tool. A tool's own
  /// word that it only reads isn't enough: words in a shared sheet could
  /// steer Ask into sending data out through it.
  async mayUse(t, args, holder) {
    if (this.trusted()[t.name]) return true;
    return new Promise((resolve) => {
      const again = h('input', { type: 'checkbox' });
      const card = h('div', { class: 'ask-choice' },
        h('p', {}, h('b', {}, `Ask wants to use ${t.server} › ${t.tool}`)),
        h('p', { class: 'q-hint' }, t.readOnly ? 'It says it only reads. This is what Ask sends it:' : 'It may change something there. This is what Ask sends it:'),
        h('pre', { class: 'ask-args' }, JSON.stringify(args, null, 1).slice(0, 2000)),
        h('label', { class: 'check-row' }, again, 'Don’t ask again for this tool'),
        h('div', { class: 'ask-row' },
          h('button', { class: 'btn small', onClick: () => { card.replaceChildren(h('p', { class: 'ask-picked' }, 'You said no')); resolve(false); } }, 'Don’t'),
          h('button', { class: 'btn small main', onClick: () => {
            if (again.checked) { const tr = this.trusted(); tr[t.name] = `${t.server} › ${t.tool}`; save(KEEP.trust, tr); }
            card.replaceChildren(h('p', { class: 'ask-picked' }, again.checked ? 'Allowed from now on (AI › Tools for AI changes it)' : 'Allowed once'));
            resolve(true);
          } }, 'Allow')));
      holder.append(card);
    });
  }
}
