// Formulas that go wrong, and formulas written for you. When the cell
// picked shows an error (#NAME?, #DIV/0!…) a line under the formula bar
// says what it means in plain words, with "did you mean" for a misspelt
// function. ✦ (by the formula bar) asks the AI: to write a formula from
// what you describe, or to fix the one that's wrong. Its answer is worked
// out first and shown with its result; nothing goes in the cell until
// you choose Use it.

import { h, modal, toast } from './ui.js';

/// What each error means, and what usually causes it.
const ERRORS = {
  '#NAME?': 'Calc doesn’t know a name in this formula: a function or table spelt differently, or text without quotes around it.',
  '#DIV/0!': 'Something is divided by zero, or by an empty cell.',
  '#VALUE!': 'A value is the wrong kind: text where a number is needed, or a range where one value is.',
  '#REF!': 'It points at cells that aren’t there any more (deleted, or off the edge of the sheet).',
  '#N/A': 'A look-up didn’t find what it looked for, or a value isn’t there yet.',
  '#NUM!': 'The numbers can’t work: too big, a square root of a minus number, or a calculation that never settles.',
  '#NULL!': 'Two ranges that don’t meet: a space between ranges where a comma or colon was meant.',
  '#SPILL!': 'The answer needs more cells than are free: clear the cells under or beside it.',
  '#CALC!': 'The calculation gave nothing to show, like a FILTER that matched no rows.',
  '#CIRC!': 'The formula uses its own cell (directly or through others), so it can’t finish.',
  '#ERROR!': 'Calc can’t read the formula: check the brackets, commas and quotes.',
  '#N/IMPL!': 'Calc can’t work this function out yet.',
};

/// Edit distance, for "did you mean".
function distance(a, b) {
  const d = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) d[0][j] = j;
  for (let i = 1; i <= a.length; i++) for (let j = 1; j <= b.length; j++) {
    d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  }
  return d[a.length][b.length];
}

/// What's wrong with `formula`, found without the AI: unknown functions (and
/// the nearest known ones), brackets that don't pair up.
export function check(formula, names) {
  const out = [];
  const known = new Set(names);
  const text = formula.replace(/"[^"]*"/g, '""');
  for (const m of text.matchAll(/([A-Za-z_][A-Za-z0-9._]*)\s*\(/g)) {
    const n = m[1].toUpperCase();
    if (known.has(n) || /^_XLFN\./.test(n)) continue;
    const near = names.map((k) => [k, distance(n, k)]).filter(([, d]) => d <= Math.max(1, Math.floor(n.length / 3))).sort((a, b) => a[1] - b[1]).slice(0, 2).map(([k]) => k);
    out.push(near.length ? `${m[1]} isn’t a function: did you mean ${near.join(' or ')}?` : `${m[1]} isn’t a function Calc knows.`);
  }
  const open = (text.match(/\(/g) || []).length; const close = (text.match(/\)/g) || []).length;
  if (open > close) out.push(`${open - close} bracket${open - close > 1 ? 's' : ''} opened but not closed.`);
  if (close > open) out.push(`${close - open} bracket${close - open > 1 ? 's' : ''} closed that weren’t opened.`);
  if ((formula.match(/"/g) || []).length % 2) out.push('A quote mark (") isn’t closed.');
  return out;
}

const SYSTEM = `You write spreadsheet formulas for Xenify Calc (the standard spreadsheet functions and syntax used in .xlsx files: commas between arguments, A1 references, Table[Column] for tables, 'Sheet name'!A1 for other sheets).
Reply with exactly two lines and nothing else:
line 1: the formula, starting with =
line 2: one plain sentence saying what it works out.
Use the cells and columns described. Never type numbers from the sheet into the formula: refer to their cells.`;

export class FormulaAI {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet, sheetName, ask, help }
    this.note = document.getElementById('fxnote');
  }

  /// Under the formula bar: what the picked cell's error means (or nothing).
  show() {
    const { call, grid, sheet } = this.d;
    if (!grid?.layout || grid.editing) { this.note.hidden = true; return; }
    const a = grid.active;
    let c;
    try { c = call('cell', { sheet: sheet(), row: a.r, col: a.c }); } catch { this.note.hidden = true; return; }
    const err = Object.keys(ERRORS).find((e) => c.text === e);
    if (!err || !c.input.startsWith('=')) { this.note.hidden = true; return; }
    const found = check(c.input, this.d.help.names().map((f) => f.name));
    this.note.replaceChildren(
      h('span', { class: 'err' }, err),
      h('span', {}, [ERRORS[err], ...found].join(' ')),
      h('button', { class: 'btn small', onClick: () => this.fix(c.input, err, found) }, '✦ Fix it with AI'));
    this.note.hidden = false;
  }

  /// What the AI is told about where the formula goes: the cell, the tables, the data around it.
  context() {
    const { call, grid, sheet, sheetName } = this.d;
    const a = grid.active;
    const lines = [`The formula goes in cell ${cellName(a.r, a.c)} on the sheet "${sheetName(sheet())}".`];
    const tables = call('tables').filter((t) => t.sheet === sheet());
    for (const t of tables) lines.push(`Table ${t.name} (${cellName(t.top, t.left)}:${cellName(t.bottom, t.right)}), columns: ${t.columns.map((x) => x.name).join(', ')}.`);
    // The filled cells near the top left of the sheet, as they show.
    const rows = [];
    for (let r = 1; r <= 12; r++) {
      const row = [];
      for (let col = 1; col <= 10; col++) {
        const v = call('cell', { sheet: sheet(), row: r, col }).text;
        if (v !== '') row.push(`${cellName(r, col)}=${v}`);
      }
      if (row.length) rows.push(row.join('  '));
    }
    if (rows.length) lines.push('Cells (as shown):', ...rows);
    return lines.join('\n');
  }

  /// ✦ by the formula bar: write a formula from words (or fix the cell's).
  async write() {
    const { call, grid, sheet } = this.d;
    const a = grid.active;
    const c = call('cell', { sheet: sheet(), row: a.r, col: a.c });
    if (c.input.startsWith('=') && /^#/.test(c.text)) return this.fix(c.input, c.text, check(c.input, this.d.help.names().map((f) => f.name)));
    const what = h('textarea', { class: 'field', rows: '3', placeholder: 'e.g. the total of Sales where Region is North', style: { height: 'auto', padding: '8px 10px' } });
    const ok = await modal(`A formula for ${cellName(a.r, a.c)}`, h('label', {}, 'What should it work out?', what), [['Cancel', false], ['Write it', true, true]]);
    if (!ok || !what.value.trim()) return;
    await this.propose(`${this.context()}\n\nWrite a formula that works out: ${what.value.trim()}`);
  }

  async fix(formula, err, found) {
    await this.propose(`${this.context()}\n\nThis formula shows ${err}:\n${formula}\n${found.length ? `Found so far: ${found.join(' ')}\n` : ''}Write a corrected formula that does what it was meant to.`);
  }

  /// Asks, works the answer out, and shows it: Use it puts it in the cell.
  async propose(question, tries = 2) {
    const { call, grid, sheet, sheetName } = this.d;
    // The cell picked when asked (another may be picked while the AI answers).
    const a = { ...grid.active };
    const at = sheet();
    toast('Asking the AI…');
    let answer;
    try { answer = await this.d.ask.once(SYSTEM, question); } catch (e) { if (!e.said) toast(String(e.message || e)); return; }
    const lines = answer.trim().split('\n').map((l) => l.trim().replace(/^`+|`+$/g, '')).filter(Boolean);
    const formula = lines.find((l) => l.startsWith('='));
    const why = lines.filter((l) => l !== formula).join(' ');
    if (!formula) { toast('The AI didn’t give a formula. Try saying it another way.'); return; }
    let got;
    try { got = call('ai_calc', { formula, sheet: sheetName(at) }); } catch (e) { got = { error: String(e.message || e) }; }
    const shown = got?.shows ?? got?.error ?? '';
    // A first answer that gives an error goes back once, with the error.
    if (tries > 1 && /^#/.test(String(shown))) return this.propose(`${question}\n\nYour formula ${formula} gives ${shown}. Write one that works.`, tries - 1);
    const problems = check(formula, this.d.help.names().map((f) => f.name));
    const use = await modal('The AI’s formula', h('div', { class: 'ai-formula' },
      h('code', {}, formula),
      why ? h('p', {}, why) : null,
      h('p', { class: 'hint' }, 'It gives: ', h('b', {}, String(shown))),
      problems.length ? h('p', { class: 'hint' }, problems.join(' ')) : null,
      h('p', { class: 'hint' }, 'Check it does what you meant before using it.')),
    [['Cancel', false], [`Use it in ${cellName(a.r, a.c)}`, true, true]]);
    if (!use) return;
    this.d.run('type', { sheet: at, row: a.r, col: a.c, input: formula });
    if (sheet() === at) grid.select(a.r, a.c);
  }
}

function letters(c) { let s = ''; while (c > 0) { const r = (c - 1) % 26; s = String.fromCharCode(65 + r) + s; c = Math.floor((c - 1) / 26); } return s; }
const cellName = (r, c) => `${letters(c)}${r}`;
