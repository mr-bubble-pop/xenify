// Spelling and grammar on the canvas: every text layer is checked with
// Xenify Write's checker (on this computer). Mistakes are underlined on
// the canvas and listed in the Writing panel, each with the layer it's in
// and when that text is on screen.

import { call } from './engine.js';
import * as platform from './platform.js';
import { fmtTime } from './timeline.js';

/// The kinds of writing a canvas can be checked as (Write's profiles).
export const KINDS = [
  { id: 'standard', name: 'Everyday writing', who: 'anyone' },
  { id: 'marketing', name: 'Poster or advert', who: 'the public' },
  { id: 'social', name: 'Social media post', who: 'followers' },
  { id: 'casual', name: 'Card or invitation', who: 'friends and family' },
  { id: 'creative', name: 'Story or comic', who: 'readers' },
  { id: 'school-essay', name: 'School work', who: 'a teacher' },
  { id: 'technical', name: 'Work slides or diagram', who: 'colleagues' },
];

const COLOURS = { Spelling: '#DC2626', Grammar: '#2563EB' };
const colour = (cat) => COLOURS[cat] || '#C2410C';
const label = (cat) => ({ Spelling: 'Spelling', Grammar: 'Grammar', Style: 'Clearer wording', Readability: 'Easier to read', Register: 'Tone' }[cat] || cat);

export class Writing {
  constructor(app) {
    this.app = app;
    this.issues = [];
    this.ignored = new Set();
    this.timer = null;
    this.seq = 0;
    this.kind = localStorage.getItem('draw.kind') || 'marketing';
    this.active = null;
    this.failed = null;
  }

  /// Checks again shortly (after typing pauses).
  soon() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.run(), 450);
  }

  async run() {
    const seq = ++this.seq;
    let texts;
    try { texts = call('texts'); } catch { return; }
    this.texts = new Map(texts.map((t) => [t.id, t]));
    if (!texts.length) { this.issues = []; this.paint(); return; }
    let found;
    try {
      found = await platform.check(texts.map((t) => ({ id: t.id, text: t.text })), { profile: this.kind });
      this.failed = null;
    } catch (e) {
      this.failed = String(e.message || e);
      this.paint();
      return;
    }
    this.lastRaw = found;
    if (seq !== this.seq) return;
    this.issues = found
      .map((i) => {
        const t = this.texts.get(i.para);
        if (!t) return null;
        const word = t.text.slice(...byteRange(t.text, i.range));
        return { ...i, layer: i.para, name: t.name, from: t.from, word, key: `${i.para}:${i.rule}:${word}` };
      })
      .filter((i) => i && !this.ignored.has(i.key));
    this.paint();
  }

  /// Underlines on the canvas and the panel.
  paint() {
    const byLayer = new Map();
    for (const i of this.issues) {
      if (!byLayer.has(i.layer)) byLayer.set(i.layer, []);
      byLayer.get(i.layer).push(i);
    }
    const marks = [];
    for (const [id, list] of byLayer) {
      let rects;
      try { rects = call('text_rects', { id, ranges: list.map((i) => [i.range.start, i.range.end]) }); } catch { continue; }
      list.forEach((i, n) => {
        i.quads = rects[n] || [];
        marks.push({ quads: i.quads, color: colour(i.category) });
      });
    }
    this.app.viewport.marks = marks;
    this.app.viewport.redraw();
    this.app.onIssues?.();
  }

  /// The issue under a canvas point (composition space), for right-click.
  at(p) {
    return this.issues.find((i) => (i.quads || []).some((q) => inside(q, p)));
  }

  /// Where issue `i` is in its text, as UTF-16 offsets.
  chars(i) {
    return byteRange(this.texts.get(i.layer)?.text || '', i.range);
  }

  /// Puts new words in a text layer or a shape's label.
  setText(id, text) {
    if (this.texts.get(id)?.label) this.app.run('set_label', { id, text });
    else this.app.run('set_text', { id, text });
  }

  async fix(i, replacement) {
    const t = this.texts.get(i.layer);
    if (!t) return;
    const [s, e] = byteRange(t.text, i.range);
    this.setText(i.layer, t.text.slice(0, s) + replacement + t.text.slice(e));
    this.app.changed();
    await this.run();
  }

  ignore(i) {
    this.ignored.add(i.key);
    this.issues = this.issues.filter((x) => x !== i);
    this.paint();
  }

  async addWord(i) {
    const list = await platform.words();
    if (!list.includes(i.word)) await platform.setWords([...list, i.word]);
    await this.run();
  }

  async fixAllSpelling() {
    // Last first, so earlier ranges stay where they are.
    const spelling = this.issues.filter((i) => i.category === 'Spelling' && i.suggestions.length).sort((a, b) => b.range.start - a.range.start);
    const texts = new Map([...this.texts].map(([id, t]) => [id, t.text]));
    for (const i of spelling) {
      const text = texts.get(i.layer);
      const [s, e] = byteRange(text, i.range);
      texts.set(i.layer, text.slice(0, s) + i.suggestions[0] + text.slice(e));
    }
    for (const [id, text] of texts) if (text !== this.texts.get(id).text) this.setText(id, text);
    this.app.changed();
    await this.run();
  }

  setKind(id) {
    this.kind = id;
    localStorage.setItem('draw.kind', id);
    this.run();
  }

  /// Selects the layer and, if it isn't on screen now, moves to when it is.
  show(i) {
    this.active = i.key;
    this.app.select([i.layer]);
    const st = this.app.state;
    const layer = this.app.layerById(i.layer);
    if (layer && (st.time < i.from - 1e-6 || (layer.out != null && st.time > layer.out))) this.app.setTime(i.from);
    this.app.renderSide();
  }

  /// The Writing panel.
  panel(h) {
    const kind = KINDS.find((k) => k.id === this.kind) || KINDS[0];
    const head = h('div', { class: 'card', style: 'display: flex; flex-direction: column; gap: 6px' },
      h('label', { class: 'small', style: 'display: flex; flex-direction: column; gap: 6px' }, 'Checking this as',
        h('select', { class: 'field', style: 'font-weight: 600; font-size: 15px', 'aria-label': 'Kind of writing',
          onChange: (e) => this.setKind(e.target.value) },
          KINDS.map((k) => h('option', { value: k.id, selected: k.id === this.kind }, k.name)))),
      h('div', { class: 'mid' }, `For ${kind.who} · checked on this computer`));
    const n = this.issues.length;
    const spellingFixable = this.issues.some((i) => i.category === 'Spelling' && i.suggestions.length);
    const title = h('div', { style: 'display: flex; justify-content: space-between; align-items: center; gap: 8px' },
      h('h2', { style: 'font-size: 16px' }, n ? `${n} thing${n === 1 ? '' : 's'} to look at` : 'Nothing to look at'),
      spellingFixable ? h('button', { class: 'btn', style: 'height: 32px; font-size: 13.5px', onClick: () => this.fixAllSpelling() }, 'Fix all spelling') : null);
    const cards = this.issues.map((i) => this.card(h, i));
    const note = this.failed
      ? h('div', { class: 'empty', role: 'alert' }, `The checker couldn't run: ${this.failed}`)
      : !n ? h('div', { class: 'empty' }, this.texts?.size ? 'The words on this canvas look right.' : 'Add text to the canvas and it’s checked here as you type.') : null;
    return [head, title, ...cards, note,
      h('div', { class: 'empty', style: 'font-size: 13px' }, 'Every text layer and shape label is checked. Picking a card selects the layer and moves to the moment it’s on screen.')];
  }

  card(h, i) {
    const fix = i.suggestions[0];
    const on = this.active === i.key;
    return h('div', { class: `issue${on ? ' on' : ''}`, role: 'group', 'aria-label': `${label(i.category)}: ${i.word}`, onClick: (e) => { if (!e.target.closest('button')) this.show(i); } },
      h('div', { class: 'kind', style: `color: ${colour(i.category)}` }, h('span', { class: 'kdot', style: `background: ${colour(i.category)}` }), label(i.category),
        h('span', { class: 'where' }, i.from > 0.001 ? `${i.name} · from ${fmtTime(i.from)}` : i.name)),
      h('div', { class: 'what' }, fix != null ? [h('s', {}, i.word), ' → ', h('b', {}, fix || '(remove)')] : h('b', {}, i.word)),
      i.category !== 'Spelling' && i.message ? h('div', { class: 'why' }, i.message.replace(/`([^`]+)`/g, '“$1”')) : null,
      h('div', { class: 'acts' },
        fix != null ? h('button', { class: 'btn main', onClick: () => this.fix(i, fix) }, i.category === 'Spelling' ? 'Fix' : `Use “${fix}”`) : null,
        i.suggestions.slice(1, 3).map((s) => h('button', { class: 'chip', onClick: () => this.fix(i, s) }, s)),
        i.category === 'Spelling' ? h('button', { class: 'btn', onClick: () => this.addWord(i) }, 'Add to my words') : null,
        h('button', { class: 'btn', onClick: () => this.ignore(i) }, 'Ignore')));
  }
}

/// The checker's byte range as string (UTF-16) offsets.
function byteRange(text, r) {
  const enc = new TextEncoder();
  let bytes = 0, s = null, e = null, i = 0;
  for (const ch of text) {
    if (s == null && bytes >= r.start) s = i;
    if (e == null && bytes >= r.end) e = i;
    bytes += enc.encode(ch).length;
    i += ch.length;
  }
  return [s ?? i, e ?? i];
}

function inside(q, [x, y]) {
  let w = false;
  for (let a = 0, b = q.length - 1; a < q.length; b = a++) {
    const [xa, ya] = q[a], [xb, yb] = q[b];
    if ((ya > y) !== (yb > y) && x < ((xb - xa) * (y - ya)) / (yb - ya) + xa) w = !w;
  }
  return w;
}
