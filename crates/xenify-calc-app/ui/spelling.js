// Spelling and grammar in text cells, with Xenify Write's checker (shared
// with Write and Draw, so words added to the dictionary count in all of
// them). Words with something to say get a wavy line (red: spelling, blue:
// grammar); the pointer over the cell says what, and right-clicking it
// offers the fixes. Look up gives a word's meanings and words to use
// instead (the dictionary and thesaurus); clicking one puts it in the cell.
// Numbers and formulas aren't checked.

import { h, item, popup, closePopups, toast } from './ui.js';
import * as platform from './platform.js';

const enc = new TextEncoder();
/// A byte offset in `text` (as the checker counts) as a character offset (as JS does).
function charAt(text, byte) {
  let n = 0;
  for (let i = 0; i < text.length; i++) {
    if (n >= byte) return i;
    n += enc.encode(text[i]).length;
  }
  return text.length;
}

const ID = (r, c) => r * 20000 + c;

export class Spelling {
  constructor(d) {
    this.d = d;   // { call, run, grid, sheet }
    this.on = (() => { try { return localStorage.getItem('calc-spelling') !== 'off'; } catch { return true; } })();
    this.issues = new Map();   // "r,c" -> the checker's issues
    this.ignored = [];
    this.timer = null;
    this.asked = 0;   // the latest check (an older answer coming late is dropped)
  }

  toggle() {
    this.on = !this.on;
    try { localStorage.setItem('calc-spelling', this.on ? 'on' : 'off'); } catch { /* fine */ }
    if (this.on) this.soon(0); else { this.issues.clear(); this.show(); }
    toast(this.on ? 'Checking spelling and grammar in text cells' : 'Not checking spelling');
  }

  /// After a change: checked again a moment later.
  soon(wait = 600) {
    if (!this.on) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.check(), wait);
  }

  async check() {
    const { call, sheet } = this.d;
    const at = sheet();
    const n = ++this.asked;
    let cells;
    try { cells = call('texts', { sheet: at }); } catch { return; }
    if (!cells.length) { this.issues.clear(); this.show(); return; }
    let found;
    try {
      found = await platform.check(cells.map(([r, c, text]) => ({ id: ID(r, c), text })), { ignoreWords: this.ignored });
    } catch (e) { platform.log(`spelling: ${e.message || e}`); return; }
    if (n !== this.asked || !this.on || sheet() !== at) return;
    const text = new Map(cells.map(([r, c, t]) => [ID(r, c), t]));
    this.issues.clear();
    // Spelling and grammar only: style notes would be noise in short cells.
    for (const i of (found || []).filter((x) => x.category === 'Spelling' || x.category === 'Grammar')) {
      const t = text.get(i.para);
      if (t == null) continue;
      // Codes are left alone: CRO, MUF, A4, SKU-2201 (all capitals, or with digits).
      const w = t.slice(charAt(t, i.range.start), charAt(t, i.range.end));
      if (i.category === 'Spelling' && (/\d/.test(w) || (w.length > 1 && w === w.toUpperCase() && /\p{Lu}/u.test(w)))) continue;
      // Capitalised words are names and places (customers, suppliers, towns:
      // "Harbour Café"); every cell starts like a sentence, so the checker
      // can't tell them apart from slips. Better unflagged than a false alarm.
      if (i.category === 'Spelling' && /^\p{Lu}/u.test(w)) continue;
      const key = `${Math.floor(i.para / 20000)},${i.para % 20000}`;
      const a = charAt(t, i.range.start); const b = charAt(t, i.range.end);
      if (!this.issues.has(key)) this.issues.set(key, []);
      this.issues.get(key).push({ ...i, message: i.message.replace(/`([^`]*)`/g, '“$1”'), a, b, word: t.slice(a, b), spelling: i.category === 'Spelling' });
    }
    this.show();
  }

  show() {
    this.d.grid.issues = this.issues;
    this.d.grid.draw();
  }

  /// Right-click on a cell: its fixes, Add to the dictionary, Ignore, Look up.
  menuItems(r, c) {
    const list = this.issues.get(`${r},${c}`) || [];
    const out = [];
    for (const i of list.slice(0, 3)) {
      out.push({ head: i.message.length > 70 ? `${i.message.slice(0, 68)}…` : i.message });
      for (const s of i.suggestions.slice(0, 4)) out.push(item(s === '' ? `Take out “${i.word}”` : `Change to “${s}”`, { run: () => this.replace(r, c, i, s) }));
      if (i.spelling) out.push(item(`Add “${i.word}” to the dictionary`, { run: () => this.addWord(i.word) }));
      out.push(item('Ignore it', { run: () => { this.ignored.push(i.word); this.soon(0); } }));
    }
    const word = list[0]?.word || this.firstWord(r, c);
    if (word) out.push(item(`Look up “${word}”…`, { run: () => this.lookUp(word, r, c) }));
    return out.length ? [...out, null] : [];
  }

  firstWord(r, c) {
    const v = this.d.call('cell', { sheet: this.d.sheet(), row: r, col: c });
    if (v.input.startsWith('=')) return null;
    return (/[\p{L}][\p{L}'’-]*/u.exec(v.text) || [])[0] || null;
  }

  replace(r, c, i, s) {
    const { call, run, sheet } = this.d;
    const t = call('cell', { sheet: sheet(), row: r, col: c }).input;
    if (t.slice(i.a, i.b) !== i.word) { this.soon(0); return; }
    run('type', { sheet: sheet(), row: r, col: c, input: t.slice(0, i.a) + s + t.slice(i.b) });
    this.soon(0);
  }

  async addWord(word) {
    try {
      const now = await platform.words();
      await platform.setWords([...new Set([...(now || []), word])]);
      toast(`Added “${word}” to the dictionary (Write and Draw use it too)`);
    } catch (e) { toast(String(e.message || e)); }
    this.soon(0);
  }

  /// The dictionary and thesaurus for `word`; a word to use instead goes in cell (r, c).
  async lookUp(word, r = null, c = null) {
    let got;
    try { got = await platform.lookUp(word); } catch (e) { toast(String(e.message || e)); return; }
    const g = this.d.grid;
    const box = g.host.getBoundingClientRect();
    const at = r != null ? { x: box.left + g.x(c), y: box.top + g.y(r) + g.rows.size(r) + 4 } : { x: innerWidth / 2 - 180, y: 160 };
    const swap = (w) => {
      closePopups();
      if (r == null) { navigator.clipboard?.writeText(w).catch(() => {}); toast(`Copied “${w}”`); return; }
      const t = this.d.call('cell', { sheet: this.d.sheet(), row: r, col: c }).input;
      const k = t.toLowerCase().indexOf(word.toLowerCase());
      if (k < 0) return;
      const same = word[0] === word[0].toUpperCase() ? w[0].toUpperCase() + w.slice(1) : w;
      this.d.run('type', { sheet: this.d.sheet(), row: r, col: c, input: t.slice(0, k) + same + t.slice(k + word.length) });
      this.soon(0);
    };
    const body = h('div', { class: 'lookup-word' },
      h('div', { class: 'lw-head' }, h('b', {}, got.base || word), got.pronunciation ? h('span', {}, got.pronunciation) : null),
      got.meanings?.length ? got.meanings.slice(0, 6).map((m) => h('div', { class: 'lw-meaning' },
        h('span', { class: 'pos' }, m.pos),
        h('p', {}, m.definition),
        m.example ? h('p', { class: 'ex' }, `“${m.example}”`) : null,
        m.swaps?.length ? h('div', { class: 'swaps' }, h('span', { class: 'k' }, r != null ? 'Use instead:' : 'Words like it:'), m.swaps.map((w) => h('button', { class: 'chip', onClick: () => swap(w) }, w))) : null,
        m.opposites?.length ? h('div', { class: 'swaps' }, h('span', { class: 'k' }, 'Opposites:'), m.opposites.map((w) => h('button', { class: 'chip', onClick: () => swap(w) }, w))) : null))
        : h('p', {}, 'The dictionary doesn’t have this word.'));
    popup(body, at.x, at.y);
  }
}
