// Text people type: issue summaries (one line), descriptions and comments
// (a little formatting, as in Write). All of it is checked for spelling and
// grammar with Xenify Write's checker: wavy lines (red: spelling, blue:
// grammar) drawn as highlights, so the text itself is never rewritten and
// the caret never jumps. Right-click a marked word for the fixes, Add to
// the dictionary, Ignore and Look up.

import { h, icon, item, menu, popup, closePopups, toast } from './ui.js';
import * as platform from './platform.js';

// ----- Formatted text: what's kept -----

const KEEP = new Set(['P', 'BR', 'B', 'STRONG', 'I', 'EM', 'U', 'S', 'UL', 'OL', 'LI', 'A', 'CODE', 'H2', 'H3', 'BLOCKQUOTE', 'PRE', 'HR']);
const BLOCKS = new Set(['P', 'DIV', 'UL', 'OL', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'BLOCKQUOTE', 'PRE', 'HR']);
// Enter makes paragraphs (not the browser's own <div>s).
try { document.execCommand('defaultParagraphSeparator', false, 'p'); } catch { /* fine */ }
const SAME = { DIV: 'P', H1: 'H2', H4: 'H3', H5: 'H3', H6: 'H3', DEL: 'S', STRIKE: 'S' };

/// Only paragraphs, headings, lists and checklists, quotes, code, bold,
/// italic, underline, strikethrough, lines and web links (pasted or someone else's text).
export function clean(html) {
  const t = document.createElement('template');
  t.innerHTML = String(html || '').replace(/​/g, '');
  const out = document.createElement('div');
  const copy = (from, to) => {
    for (const n of from.childNodes) {
      if (n.nodeType === Node.TEXT_NODE) { to.append(n.textContent); continue; }
      if (n.nodeType !== Node.ELEMENT_NODE) continue;
      const tag = SAME[n.tagName] || n.tagName;
      if (['SCRIPT', 'STYLE', 'TEMPLATE', 'IFRAME', 'OBJECT'].includes(tag)) continue;
      if (!KEEP.has(tag)) { copy(n, to); continue; }
      // A paragraph (or the browser's <div>) holding lists or other blocks: just its insides.
      if (tag === 'P' && [...n.children].some((k) => BLOCKS.has(k.tagName))) { copy(n, to); continue; }
      const e = document.createElement(tag);
      if (tag === 'A') {
        const href = n.getAttribute('href') || '';
        if (/^(https?:|mailto:)/i.test(href)) e.setAttribute('href', href);
        else { copy(n, to); continue; }
      }
      // A checklist: its list says so, each item whether it's ticked.
      if (tag === 'UL' && n.classList.contains('checks')) e.className = 'checks';
      if (tag === 'LI' && n.hasAttribute('data-done') && n.parentElement?.classList.contains('checks')) e.setAttribute('data-done', '');
      copy(n, e);
      to.append(e);
    }
  };
  copy(t.content, out);
  const s = out.innerHTML.replace(/<p><\/p>/g, '').replace(/^(<p><br><\/p>|<br>)+$/, '');
  return s.trim() === '' || (out.textContent.trim() === '' && !out.querySelector('hr, li')) ? '' : s;
}

const esc = (x) => x.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

/// Markdown words in a line: **bold**, *italic*, `code`, ~~struck~~, [links](https://…).
function mdInline(x) {
  return esc(x)
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/__([^_]+)__/g, '<b>$1</b>')
    .replace(/(^|[^*\w])\*([^*\s][^*]*)\*/g, '$1<i>$2</i>')
    .replace(/(^|[^_\w])_([^_\s][^_]*)_/g, '$1<i>$2</i>')
    .replace(/~~([^~]+)~~/g, '<s>$1</s>')
    .replace(/\[([^\]]+)\]\(((?:https?:|mailto:)[^)\s]+)\)/g, '<a href="$2">$1</a>');
}

/// Pasted text written in Markdown: headings, lists, checklists, quotes, code and lines.
export function markdown(text) {
  const lines = String(text).replace(/\r\n?/g, '\n').split('\n');
  const out = [];
  let list = null; // 'ul' | 'ol' | 'checks'
  const close = () => { if (list) out.push(list === 'ol' ? '</ol>' : '</ul>'); list = null; };
  const open = (k) => { if (list !== k) { close(); out.push(k === 'ol' ? '<ol>' : k === 'checks' ? '<ul class="checks">' : '<ul>'); list = k; } };
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    let m;
    if (/^\s*```/.test(l)) {
      close();
      const code = [];
      while (++i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i]);
      out.push(`<pre>${esc(code.join('\n'))}</pre>`);
    } else if ((m = l.match(/^\s*[-*+]\s+\[( |x|X)\]\s+(.*)$/)) || (m = l.match(/^\s*\[( |x|X)\]\s+(.*)$/))) { open('checks'); out.push(`<li${m[1] === ' ' ? '' : ' data-done'}>${mdInline(m[2])}</li>`); }
    else if ((m = l.match(/^\s*[-*+]\s+(.*)$/))) { open('ul'); out.push(`<li>${mdInline(m[1])}</li>`); }
    else if ((m = l.match(/^\s*\d+[.)]\s+(.*)$/))) { open('ol'); out.push(`<li>${mdInline(m[1])}</li>`); }
    else if ((m = l.match(/^(#{1,6})\s+(.*)$/))) { close(); out.push(`<h${m[1].length <= 2 ? 2 : 3}>${mdInline(m[2])}</h${m[1].length <= 2 ? 2 : 3}>`); }
    else if ((m = l.match(/^>\s?(.*)$/))) { close(); out.push(`<blockquote>${mdInline(m[1])}</blockquote>`); }
    else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(l)) { close(); out.push('<hr>'); }
    else if (!l.trim()) close();
    else { close(); out.push(`<p>${mdInline(l)}</p>`); }
  }
  close();
  return out.join('').replace(/<\/blockquote><blockquote>/g, '<br>');
}

/// Whether pasted text looks like Markdown (rather than plain lines).
const looksLikeMarkdown = (t) => /(^|\n)\s*(#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s|```|\[[ xX]\]\s)|\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\(https?:/.test(t);

/// The words without formatting (search, the AI, short previews).
export function plainText(html) {
  const t = document.createElement('template');
  t.innerHTML = clean(html).replace(/<\/(p|li)>/g, '$& ');
  return t.content.textContent.replace(/\s+/g, ' ').trim();
}

// ----- Checking -----

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

const HL = typeof CSS !== 'undefined' && CSS.highlights && typeof Highlight !== 'undefined';
const spell = HL ? new Highlight() : null;
const gram = HL ? new Highlight() : null;
if (HL) { CSS.highlights.set('xen-spell', spell); CSS.highlights.set('xen-gram', gram); }

const checked = new Set();
const ignored = [];
let on = (() => { try { return localStorage.getItem('plan-spelling') !== 'off'; } catch { return true; } })();
let nextPara = 1;

export const spelling = {
  get on() { return on; },
  toggle() {
    spelling.set(!on);
    toast(on ? 'Checking spelling and grammar' : 'Not checking spelling and grammar');
    // Your settings follow you (serverplan.js sends them to the server).
    document.dispatchEvent(new Event('plan-settings'));
  },
  set(yes) {
    if (yes === on) return;
    on = yes;
    try { localStorage.setItem('plan-spelling', on ? 'on' : 'off'); } catch { /* fine */ }
    for (const c of checked) c.soon(0);
  },
};

function redraw() {
  if (!HL) return;
  spell.clear();
  gram.clear();
  for (const c of [...checked]) {
    if (!c.el.isConnected) { checked.delete(c); continue; }
    for (const f of c.found) (f.spelling ? spell : gram).add(f.range);
  }
}

/// Watches an editable element and marks what the checker finds.
export class Checker {
  constructor(el) {
    this.el = el;
    this.found = [];
    this.timer = null;
    checked.add(this);
    el.addEventListener('input', () => this.soon());
    el.addEventListener('focus', () => this.soon(300));
    el.addEventListener('contextmenu', (e) => this.menu(e));
    this.soon(50);
  }

  soon(wait = 800) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.check(), wait);
  }

  /// The text by paragraph, with where each text node sits in it.
  paragraphs() {
    const paras = new Map();
    const walk = document.createTreeWalker(this.el, NodeFilter.SHOW_TEXT);
    while (walk.nextNode()) {
      const n = walk.currentNode;
      const block = n.parentElement.closest('p, li, div') || this.el;
      const at = this.el.contains(block) ? block : this.el;
      if (!paras.has(at)) paras.set(at, { id: nextPara++, text: '', parts: [] });
      const p = paras.get(at);
      p.parts.push([n, p.text.length]);
      p.text += n.textContent;
    }
    return [...paras.values()].filter((p) => p.text.trim());
  }

  async check() {
    // Not on the page yet (made before it's shown): wait; gone again: forget it.
    if (!this.el.isConnected) { if (this.shown) { checked.delete(this); redraw(); } return; }
    this.shown = true;
    checked.add(this);
    if (!on) { this.found = []; redraw(); return; }
    const paras = this.paragraphs();
    if (!paras.length) { this.found = []; redraw(); return; }
    let issues;
    try { issues = await platform.check(paras.map((p) => ({ id: p.id, text: p.text })), { ignoreWords: ignored }); } catch (e) { platform.log(`spelling: ${e.message || e}`); return; }
    const byId = new Map(paras.map((p) => [p.id, p]));
    const found = [];
    for (const i of issues || []) {
      if (i.category !== 'Spelling' && i.category !== 'Grammar') continue;
      const p = byId.get(i.para);
      if (!p) continue;
      const a = charAt(p.text, i.range.start);
      const b = charAt(p.text, i.range.end);
      const word = p.text.slice(a, b);
      // Codes (WEB-12, API, v2) and capitalised words (names of people,
      // products and places) aren't spelling mistakes here: better unmarked
      // than a false alarm. A sentence's first word is checked all the same.
      const before = p.text.slice(0, a);
      const opens = /^[\s"“‘'(]*$/.test(before) || /[.!?:][\s"“‘'(]+$/.test(before);
      const code = /\d/.test(word) || /^\p{Lu}[\p{Lu}\d-]+$/u.test(word);
      if (i.category === 'Spelling' && (code || (/^\p{Lu}/u.test(word) && !opens))) continue;
      const range = this.range(p, a, b);
      if (range) found.push({ ...i, range, word, spelling: i.category === 'Spelling', message: i.message.replace(/`([^`]*)`/g, '“$1”') });
    }
    this.found = found;
    redraw();
  }

  range(p, a, b) {
    const at = (k) => {
      for (let j = p.parts.length - 1; j >= 0; j--) if (p.parts[j][1] <= k) return [p.parts[j][0], k - p.parts[j][1]];
      return null;
    };
    const s = at(a);
    const e = at(b === a ? b : b - 1);
    if (!s || !e || !s[0].isConnected) return null;
    const r = document.createRange();
    try {
      r.setStart(s[0], Math.min(s[1], s[0].length));
      r.setEnd(e[0], Math.min(e[1] + (b === a ? 0 : 1), e[0].length));
    } catch { return null; }
    return r;
  }

  menu(e) {
    const pos = document.caretRangeFromPoint?.(e.clientX, e.clientY);
    const f = pos && this.found.find((x) => { try { return x.range.isPointInRange(pos.startContainer, pos.startOffset); } catch { return false; } });
    if (!f) return;
    e.preventDefault();
    const items = [{ head: f.message.length > 80 ? `${f.message.slice(0, 78)}…` : f.message }];
    for (const s of (f.suggestions || []).slice(0, 5)) items.push(item(s === '' ? `Take out “${f.word}”` : `Change to “${s}”`, { run: () => this.replace(f, s) }));
    if (f.spelling) items.push(item(`Add “${f.word}” to the dictionary`, { run: () => addWord(f.word, this) }));
    items.push(item('Ignore it', { run: () => { ignored.push(f.word); for (const c of checked) c.soon(0); } }));
    items.push(null, item(`Look up “${f.word}”…`, { run: () => lookUp(f.word, e.clientX, e.clientY, (w) => this.replace(f, w)) }));
    menu(items, e.clientX, e.clientY);
  }

  replace(f, s) {
    this.el.focus();
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(f.range);
    // As typing: one undo step, and the editor's own change events.
    document.execCommand('insertText', false, s);
    this.soon(0);
  }
}

async function addWord(word, c) {
  try {
    const now = await platform.words();
    await platform.setWords([...new Set([...(now || []), word])]);
    toast(`Added “${word}” to the dictionary (Write, Calc and Draw use it too)`);
  } catch (e) { toast(String(e.message || e)); }
  for (const x of checked) x.soon(0);
  c.soon(0);
}

async function lookUp(word, x, y, use) {
  let got;
  try { got = await platform.lookUp(word); } catch (e) { toast(String(e.message || e)); return; }
  const swap = (w) => { closePopups(); use(word[0] === word[0].toUpperCase() ? w[0].toUpperCase() + w.slice(1) : w); };
  const body = h('div', { class: 'lookup-word' },
    h('div', { class: 'lw-head' }, h('b', {}, got.base || word), got.pronunciation ? h('span', {}, got.pronunciation) : null),
    got.meanings?.length ? got.meanings.slice(0, 6).map((m) => h('div', { class: 'lw-meaning' },
      h('span', { class: 'pos' }, m.pos),
      h('p', {}, m.definition),
      m.example ? h('p', { class: 'ex' }, `“${m.example}”`) : null,
      m.swaps?.length ? h('div', { class: 'swaps' }, h('span', { class: 'k' }, 'Use instead:'), m.swaps.map((w) => h('button', { class: 'chip', onClick: () => swap(w) }, w))) : null))
      : h('p', {}, 'The dictionary doesn’t have this word.'));
  popup(body, x, y + 8);
}

// ----- Editors -----

/// One line of text (a summary): Enter keeps it, Escape puts back what was there.
export function oneLine({ text = '', label, placeholder = '', onCommit, className = 'oneline', check = true }) {
  const el = h('div', { class: className, contenteditable: 'plaintext-only', role: 'textbox', 'aria-label': label, 'data-ph': placeholder, spellcheck: 'false' });
  el.textContent = text;
  let was = text;
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); if (!(e.ctrlKey || e.metaKey)) el.blur(); }
    if (e.key === 'Escape' && el.textContent !== was) { e.preventDefault(); e.stopPropagation(); el.textContent = was; el.blur(); }
  });
  el.addEventListener('paste', (e) => {
    e.preventDefault();
    document.execCommand('insertText', false, (e.clipboardData.getData('text/plain') || '').replace(/\s*\n\s*/g, ' '));
  });
  el.addEventListener('focus', () => { was = el.textContent; });
  el.addEventListener('blur', () => {
    const now = el.textContent.replace(/\s+/g, ' ').trim();
    if (el.textContent !== now) el.textContent = now;
    if (now !== was.trim()) onCommit?.(now);
  });
  if (check) new Checker(el);
  return el;
}

/// Formatted text (a description or a comment) with a toolbar, and
/// Markdown as you type: "# " a heading, "- " a list, "[] " a checklist,
/// "> " a quote, "```" a code block, **bold**, *italic*, `code`, ~~struck~~.
/// `onSave(html)` runs when it changed and the editor loses focus (or a
/// checklist item is ticked).
export function richText({ html = '', label, placeholder = '', onSave, compact = false }) {
  const ed = h('div', { class: 'rte-text', contenteditable: 'true', role: 'textbox', 'aria-multiline': 'true', 'aria-label': label, 'data-ph': placeholder, spellcheck: 'false' });
  ed.innerHTML = clean(html);
  let was = ed.innerHTML;
  const changed = () => ed.dispatchEvent(new Event('input', { bubbles: true }));
  const cmd = (c, v) => () => { ed.focus(); document.execCommand(c, false, v); changed(); };
  const b = (svgOrText, title, run) => h('button', { type: 'button', class: 'rte-b', title, 'aria-label': title, onMousedown: (e) => e.preventDefault(), onClick: run },
    typeof svgOrText === 'string' && svgOrText.startsWith('<') ? h('span', { html: svgOrText }) : svgOrText);
  /// The block the caret is in (a paragraph, heading, list item …), right under the editor or in a list.
  const blockAt = () => {
    const sel = getSelection();
    if (!sel.rangeCount || !ed.contains(sel.anchorNode)) return null;
    let n = sel.anchorNode.nodeType === Node.TEXT_NODE ? sel.anchorNode.parentElement : sel.anchorNode;
    while (n && n !== ed && !/^(P|LI|H2|H3|BLOCKQUOTE|PRE|DIV)$/.test(n.tagName)) n = n.parentElement;
    return n && n !== ed ? n : null;
  };
  const listAt = () => blockAt()?.closest?.('ul, ol');
  const block = (tag) => {
    ed.focus();
    const now = blockAt()?.tagName;
    document.execCommand('formatBlock', false, now === tag ? 'P' : tag);
    changed();
  };
  const checklist = () => {
    ed.focus();
    const ul = listAt();
    if (ul?.classList.contains('checks')) { document.execCommand('insertUnorderedList'); changed(); return; }
    if (!ul || ul.tagName !== 'UL') document.execCommand('insertUnorderedList');
    listAt()?.classList.add('checks');
    changed();
  };
  const link = () => {
    const sel = getSelection();
    if (!sel.rangeCount || sel.isCollapsed) { toast('Select the words to link first'); return; }
    const saved = sel.getRangeAt(0).cloneRange();
    const input = h('input', { class: 'field', placeholder: 'https://…', 'aria-label': 'Web address' });
    const go = () => {
      let url = input.value.trim();
      closePopups();
      if (!url) return;
      if (!/^(https?:|mailto:)/i.test(url)) url = `https://${url}`;
      ed.focus();
      sel.removeAllRanges();
      sel.addRange(saved);
      document.execCommand('createLink', false, url);
      changed();
    };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); go(); } });
    const r = saved.getBoundingClientRect();
    popup(h('div', { class: 'link-box' }, input, h('button', { class: 'btn main small', onClick: go }, 'Link')), r.left, r.bottom + 6);
    input.focus();
  };
  const style = h('select', { class: 'rte-style', 'aria-label': 'Paragraph style', title: 'Paragraph style', onMousedown: () => { saveSel = getSelection().rangeCount ? getSelection().getRangeAt(0).cloneRange() : null; } },
    [['P', 'Text'], ['H2', 'Heading'], ['H3', 'Subheading'], ['BLOCKQUOTE', 'Quote'], ['PRE', 'Code block']].map(([v, n]) => h('option', { value: v }, n)));
  let saveSel = null;
  style.addEventListener('change', () => {
    ed.focus();
    if (saveSel) { const sel = getSelection(); sel.removeAllRanges(); sel.addRange(saveSel); }
    document.execCommand('formatBlock', false, style.value);
    changed();
  });
  const showStyle = () => { const t = blockAt()?.tagName; style.value = ['H2', 'H3', 'BLOCKQUOTE', 'PRE'].includes(t) ? t : 'P'; };
  document.addEventListener('selectionchange', () => { if (ed.isConnected && document.activeElement === ed) showStyle(); });
  const inlineCode = () => {
    ed.focus();
    const sel = getSelection();
    if (!sel.rangeCount) return;
    const r = sel.getRangeAt(0);
    const inCode = (sel.anchorNode.nodeType === Node.TEXT_NODE ? sel.anchorNode.parentElement : sel.anchorNode)?.closest('code');
    if (inCode && ed.contains(inCode)) { inCode.replaceWith(...inCode.childNodes); changed(); return; }
    if (r.collapsed) { toast('Select the words to show as code first'); return; }
    document.execCommand('insertHTML', false, `<code>${esc(r.toString())}</code>`);
    changed();
  };
  const bar = h('div', { class: 'rte-bar' },
    compact ? null : style,
    compact ? null : h('span', { class: 'rte-sep' }),
    b(h('b', {}, 'B'), 'Bold (Ctrl+B, or **words**)', cmd('bold')),
    b(h('i', {}, 'I'), 'Italic (Ctrl+I, or *words*)', cmd('italic')),
    b(h('u', {}, 'U'), 'Underline (Ctrl+U)', cmd('underline')),
    b(h('s', {}, 'S'), 'Strikethrough (~~words~~)', cmd('strikeThrough')),
    b(h('span', { class: 'mono' }, '</>'), 'Code (`words`)', inlineCode),
    h('span', { class: 'rte-sep' }),
    b(icon('bullets', 16), 'Bulleted list (- at the start of a line)', cmd('insertUnorderedList')),
    b(icon('numbers', 16), 'Numbered list (1. at the start of a line)', cmd('insertOrderedList')),
    b('<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="1.5" y="2.5" width="5" height="5" rx="1"/><path d="M2.8 5l1.2 1.2L5.8 4"/><rect x="1.5" y="9.5" width="5" height="5" rx="1"/><path d="M9 5h6M9 12h6"/></svg>', 'Checklist ([] at the start of a line)', checklist),
    compact ? null : b('<svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path d="M2 4h4v4H4.2c0 1.4.6 2.3 1.8 2.8l-.6 1.2C3.3 11.3 2 9.6 2 7zm7 0h4v4h-1.8c0 1.4.6 2.3 1.8 2.8l-.6 1.2C10.3 11.3 9 9.6 9 7z"/></svg>', 'Quote (> at the start of a line)', () => block('BLOCKQUOTE')),
    b(icon('weblink', 16), 'Link', link),
    compact ? null : b('<svg width="16" height="16" viewBox="0 0 16 16" stroke="currentColor" stroke-width="1.5"><path d="M2 8h12"/></svg>', 'Line across (--- on its own line)', cmd('insertHorizontalRule')));

  // Markdown at the start of a line, when the space after it is typed.
  const STARTS = [
    [/^#{1,2}$/, () => document.execCommand('formatBlock', false, 'H2')],
    [/^#{3,6}$/, () => document.execCommand('formatBlock', false, 'H3')],
    [/^[-*+]$/, () => { if (!listAt()) document.execCommand('insertUnorderedList'); }],
    [/^1[.)]$/, () => { if (!listAt()) document.execCommand('insertOrderedList'); }],
    [/^(-\s?)?\[\s?\]$/, () => checklist()],
    [/^(-\s?)?\[[xX]\]$/, () => { checklist(); blockAt()?.setAttribute('data-done', ''); }],
    [/^>$/, () => document.execCommand('formatBlock', false, 'BLOCKQUOTE')],
  ];
  const beforeCaret = () => {
    let blk = blockAt();
    const sel = getSelection();
    // Typing on the first line of an empty editor: the text isn't in a paragraph yet.
    if (!blk && sel.isCollapsed && sel.anchorNode && (sel.anchorNode === ed || sel.anchorNode.parentNode === ed)) { document.execCommand('formatBlock', false, 'P'); blk = blockAt(); }
    if (!blk || !sel.isCollapsed) return null;
    const r = document.createRange();
    r.setStart(blk, 0);
    r.setEnd(sel.anchorNode, sel.anchorOffset);
    return { blk, r, text: r.toString() };
  };
  ed.addEventListener('keydown', (e) => {
    if (e.key === 'Tab' && listAt()) { e.preventDefault(); document.execCommand(e.shiftKey ? 'outdent' : 'indent'); changed(); return; }
    if (e.key === ' ' && !e.ctrlKey && !e.metaKey) {
      const at = beforeCaret();
      if (!at || at.blk.tagName === 'PRE') return;
      const rule = STARTS.find(([re]) => re.test(at.text));
      if (!rule || (at.blk.tagName === 'LI' && !/\[/.test(at.text))) return;
      e.preventDefault();
      getSelection().removeAllRanges();
      getSelection().addRange(at.r);
      document.execCommand('delete');
      rule[1]();
      changed();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      const at = beforeCaret();
      if (!at || at.blk.tagName === 'LI') return;
      const t = at.blk.textContent;
      if (t.trim() === '```' && at.text.trim() === '```') { e.preventDefault(); at.blk.textContent = ''; document.execCommand('formatBlock', false, 'PRE'); changed(); return; }
      if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(t) && at.text === t) { e.preventDefault(); at.blk.textContent = ''; document.execCommand('insertHorizontalRule'); changed(); return; }
      // Enter on an empty line in a code block or quote: out of it.
      if ((at.blk.tagName === 'PRE' || at.blk.tagName === 'BLOCKQUOTE') && /\n?$/.test(t) && t.replace(/​/g, '').endsWith('\n') && at.text === t) {
        e.preventDefault(); at.blk.textContent = t.replace(/\n$/, ''); const p = h('p', {}, h('br')); at.blk.after(p);
        const r = document.createRange(); r.setStart(p, 0); getSelection().removeAllRanges(); getSelection().addRange(r); changed(); return;
      }
      if (at.blk.tagName === 'PRE') { e.preventDefault(); document.execCommand('insertText', false, '\n'); }
    }
  });
  // Markdown around words, when the closing mark is typed: **bold**, *italic*, `code`, ~~struck~~.
  const INLINE = [[/\*\*([^*\s][^*]*?)\*\*$/, 'b'], [/~~([^~\s][^~]*?)~~$/, 's'], [/`([^`\s][^`]*?)`$/, 'code'], [/(?:^|[^*\w])\*([^*\s][^*]*?)\*$/, 'i'], [/(?:^|[^_\w])_([^_\s][^_]*?)_$/, 'i']];
  ed.addEventListener('input', (e) => {
    // A new checklist item starts unticked.
    for (const li of ed.querySelectorAll('ul.checks > li[data-done]')) if (!li.textContent.trim() && !li.querySelector('ul, ol')) li.removeAttribute('data-done');
    if (e.inputType !== 'insertText' || !/[*`~_]/.test(e.data || '')) return;
    const sel = getSelection();
    const n = sel.anchorNode;
    if (!sel.isCollapsed || n?.nodeType !== Node.TEXT_NODE || n.parentElement.closest('code, pre')) return;
    const text = n.textContent.slice(0, sel.anchorOffset);
    for (const [re, tag] of INLINE) {
      const m = text.match(re);
      if (!m) continue;
      const whole = m[0].slice(m[0].indexOf(m[1]) - (tag === 'b' || tag === 's' ? 2 : 1));
      const from = sel.anchorOffset - whole.length;
      const r = document.createRange();
      r.setStart(n, from);
      r.setEnd(n, sel.anchorOffset);
      r.deleteContents();
      const el = h(tag, {}, m[1]);
      const after = document.createTextNode('​');
      r.insertNode(after);
      r.insertNode(el);
      const c = document.createRange();
      c.setStart(after, 1);
      sel.removeAllRanges();
      sel.addRange(c);
      break;
    }
  });
  ed.addEventListener('paste', (e) => {
    e.preventDefault();
    const html = e.clipboardData.getData('text/html');
    const text = e.clipboardData.getData('text/plain') || '';
    if (html) document.execCommand('insertHTML', false, clean(html));
    else if (looksLikeMarkdown(text) && !blockAt()?.closest('pre')) document.execCommand('insertHTML', false, markdown(text));
    else document.execCommand('insertText', false, text);
  });
  const save = () => {
    const now = clean(ed.innerHTML);
    if (now !== clean(was)) { was = ed.innerHTML; onSave?.(now); }
  };
  // A checklist item's box: clicking it ticks the item (and keeps it).
  ed.addEventListener('mousedown', (e) => {
    const li = e.target.closest?.('ul.checks > li');
    if (!li || !ed.contains(li)) return;
    const r = li.getBoundingClientRect();
    if (e.clientX > r.left + 2 || e.clientX < r.left - 26 || e.clientY > r.top + 24) return;
    e.preventDefault();
    li.toggleAttribute('data-done');
    changed();
    if (document.activeElement !== ed) save();
  });
  ed.addEventListener('click', (e) => {
    const a = e.target.closest('a');
    if (a && (e.ctrlKey || e.metaKey)) { e.preventDefault(); window.open(a.href, '_blank', 'noopener'); }
  });
  ed.addEventListener('blur', save);
  new Checker(ed);
  const box = h('div', { class: `rte${compact ? ' compact' : ''}` }, bar, ed);
  return {
    el: box,
    editor: ed,
    get html() { return clean(ed.innerHTML); },
    set html(v) { ed.innerHTML = clean(v); was = ed.innerHTML; },
    focus() { ed.focus(); },
    save,
  };
}

/// Shows saved formatted text (someone else's comment, a read-only view).
export function showHtml(html, className = 'rte-show') {
  return h('div', { class: className, html: clean(html) });
}
