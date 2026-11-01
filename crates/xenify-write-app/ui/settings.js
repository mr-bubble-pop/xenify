// Settings: sections down the left, plain-English rows on the right, and a
// search box that finds a setting by everyday words ("dark", "bigger",
// "autosave"), opens its section and highlights it.

import { call } from './engine.js';
import { icon } from './icons.js';
import { kindsDialog } from './kinds.js';
import { serverRow, ui } from './serverui.js';
import { words as myWords } from './words.js';

/// Replaces an element's children, skipping empty ones (null, false) and
/// flattening lists.
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (v === true) el.setAttribute(k, '');
    else if (v !== false && v != null) el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
}

// Section icons: single-path line drawings (Lucide shapes, ISC licence).
const D = {
  general: 'M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2 M12 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z',
  appearance: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 3v18 M12 8h6.5 M12 12h9 M12 16h6.5',
  access: 'M12 5.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3z M5 8.5l7 1.2 7-1.2 M12 9.7V14 M9 21l3-7 3 7',
  typing: 'M4 6h16a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z M6 10h.01 M10 10h.01 M14 10h.01 M18 10h.01 M8 14h8',
  spelling: 'M6 16 12 4l6 12 M8 12h8 M16 20l2 2 4-4',
  words: 'M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z',
  server: 'M4 2h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z M4 14h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-4a2 2 0 0 1 2-2z M6 6h.01 M6 18h.01',
  saving: 'M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7 M7 3v4a1 1 0 0 0 1 1h7',
};
const svg = (d) => `<svg aria-hidden="true" viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>`;

// A row: `get`/`set` read and change the settings object.
const row = (id, kind, label, desc, keys, extra = {}) => ({ id, kind, label, desc, keys, ...extra });
const path = (p) => ({ get: (s) => p.split('.').reduce((o, k) => o[k], s), set: (s, v) => { const ks = p.split('.'); const last = ks.pop(); ks.reduce((o, k) => o[k], s)[last] = v; } });

// Whether the mistake spotter loaded (filled in when Settings opens).
let spotterStatus = '';
// How much the browser build keeps for working offline.
let offlineStatus = '';

const SECTIONS = [
  { id: 'general', name: 'General', about: 'Who you are and how Write starts.', groups: [
    { title: 'You', rows: [row('name', 'text', 'Your name', 'Shown on your comments, tracked changes and change-log entries.', 'author user me signature', { eg: 'e.g. Alex Morgan', ...path('name') })] },
    { title: 'Starting up', rows: [
      row('start', 'select', 'When Write opens', 'What you see first.', 'startup launch open reopen session tabs', { options: [['blank', 'A blank page'], ['reopen', 'Reopen what I had open']], ...path('start') }),
      row('recent', 'button', 'Recent documents', (s) => `${s.recent.length} ${s.recent.length === 1 ? 'document' : 'documents'} in File › Open recent.`, 'history recent clear list', { btn: 'Clear the list', run: 'clear_recent' })] },
    { title: 'Units', rows: [row('units', 'seg', 'Show lengths in', 'Used by the ruler.', 'measurement cm centimetres inches mm points ruler margins', { options: [['cm', 'Centimetres'], ['in', 'Inches'], ['pt', 'Points']], ...path('units') })] },
  ] },
  { id: 'appearance', name: 'Appearance', about: 'How Write looks around your pages.', groups: [
    { title: 'Look', rows: [row('theme', 'seg', 'Theme', 'Pages stay white. This changes everything around them.', 'dark mode night light colour color theme', { options: [['system', 'Match computer'], ['light', 'Light'], ['dark', 'Dark']], ...path('theme') })] },
    { title: 'Show', rows: [
      row('toolbar', 'toggle', 'Toolbar', 'The row of formatting buttons under the tabs.', 'buttons ribbon formatting bar', path('toolbar')),
      row('ruler', 'toggle', 'Ruler', 'Shows the page width and its margins above the page.', 'margins indent tabs', path('ruler')),
      row('status', 'toggle', 'Status bar', 'Page number, word count, mistakes found and zoom along the bottom.', 'word count page number bottom bar zoom spelling', path('status_bar'))] },
  ] },
  { id: 'access', name: 'Accessibility', about: 'Make Write easier to see, read and use.', groups: [
    { title: 'Size and contrast', rows: [
      row('uisize', 'slider', 'Size of menus and bars', 'The page has its own zoom: Ctrl and + or −.', 'zoom bigger larger text font size scale interface', { min: 80, max: 160, step: 10, unit: '%', ...path('ui_size') }),
      row('contrast', 'toggle', 'High contrast', 'Black and white with stronger outlines.', 'contrast vision see colours', path('high_contrast'))] },
    { title: 'Movement', rows: [row('motion', 'toggle', 'Reduce motion', 'A steady cursor, and nothing slides or fades.', 'animation blink caret cursor flashing', path('reduce_motion'))] },
  ] },
  { id: 'typing', name: 'Typing', about: 'What Write does for you as you type.', groups: [
    { title: 'Format as you type', note: 'Press Ctrl+Z straight after to undo the formatting and keep what you typed.', rows: [
      row('bullets', 'toggle', 'Type * and a space to start a bulleted list', '', 'markdown bullet list asterisk dash', path('autoformat.bullets')),
      row('numbers', 'toggle', 'Type 1. and a space to start a numbered list', '', 'markdown numbered list', path('autoformat.numbers')),
      row('checklist', 'toggle', 'Type [ ] and a space to start a checklist', '', 'markdown checkbox todo tick', path('autoformat.checklist')),
      row('headings', 'toggle', 'Type # and a space to make a heading', '## for a level 2 heading, and so on.', 'markdown hash heading title', path('autoformat.headings')),
      row('quote', 'toggle', 'Type > and a space to make a quote', '', 'markdown blockquote', path('autoformat.quote')),
      row('inline', 'toggle', 'Type **bold**, *italic* or ~~struck~~ to format it', '***both*** for bold and italic; _italic_ works too.', 'markdown bold italic strikethrough emphasis asterisk underscore', path('autoformat.inline'))] },
    { title: 'Autocorrect', rows: [
      row('quotes', 'toggle', 'Curly quotes', 'Straight " and \' become “ ” and ‘ ’ as you type.', 'smart quotes apostrophe typographic curly straight', path('autocorrect.smart_quotes')),
      row('caps', 'toggle', 'Capital letter at the start of a sentence', 'Off by default: it guesses wrong after abbreviations like “e.g.”.', 'capitalise capitalize uppercase sentence', path('autocorrect.capitalize_sentences')),
      row('emoticons', 'toggle', 'Turn :) into a smiley', 'Also ;) and <3, typed on their own.', 'emoji smiley emoticon face', path('autocorrect.emoticons')),
      row('replace-on', 'toggle', 'Replace as I type', 'Fixes your usual typos and expands shortcuts, using the list below.', 'autocorrect replace typo shortcut abbreviation expand', path('autocorrect.replace')),
      row('replace', 'pairs', 'Your replacements', '', 'autocorrect replace typo shortcut list entries', { wide: true, ...path('autocorrect.entries') })] },
  ] },
  { id: 'spelling', name: 'Spelling and grammar', about: 'What gets underlined, and how strict to be.', groups: [
    { title: 'Checking', rows: [
      row('check', 'toggle', 'Check spelling and grammar as I type', 'Underlines show on screen only; they’re never printed.', 'proofing underline red squiggle spellcheck', path('proofing.enabled')),
      row('lang', 'select', 'Language', 'Which spellings count as right, like colour or color.', 'dialect british american english uk us australian canadian indian language locale', { options: [['auto', 'Match this computer'], ['British', 'English (UK)'], ['American', 'English (US)'], ['Australian', 'English (Australia)'], ['Canadian', 'English (Canada)'], ['Indian', 'English (India)']], ...path('proofing.dialect') }),
      row('spotter', 'toggle', 'Also find harder mistakes', () => `An offline model, run on this computer, that finds what the rules can’t: missing words (“pass me salt”), wrong words and some word order. ${spotterStatus}`, 'model spotter missing words ai machine learning offline harder mistakes', path('proofing.spotter.enabled')),
      row('names', 'toggle', 'Don’t flag names', 'Unknown words with a capital letter mid-sentence are left alone.', 'proper nouns names capital people places', path('proofing.ignore_names'))] },
    { title: 'Kinds of writing', note: 'A kind of writing decides what gets pointed out. A school essay is stricter about slang than a work chat. New documents get theirs from the type you pick, and you can change it in the document’s details.', rows: [
      row('profile', 'select', 'Documents without a kind are checked as', '', 'profile default document type style formality strict kind', { options: [], dynamic: 'profiles', ...path('proofing.default_profile') }),
      row('kinds', 'kinds', 'What each kind points out', 'See the checks for each kind, or copy one and make it your own.', 'profile rules edit custom checks passive voice slang contractions', {})] },
  ] },
  { id: 'words', name: 'Your words', about: 'Words that are right even though Write doesn’t know them.', groups: [
    { title: 'Words you’ve added', note: 'These are never underlined as spelling mistakes, in Write or the other Xenify apps. To add one while writing, right-click it and choose Add to my words.', rows: [
      row('wordlist', 'words', 'Your word list', '', 'dictionary custom words spelling add learn ignore', { wide: true })] },
  ] },
  { id: 'server', name: 'Server', about: 'Optional. Connect to a Xenify server to open shared files, edit together and use its AI (Review › Ask AI). Without one, Write keeps working on this computer as it does now.', groups: [
    { title: 'Your server', rows: [
      row('server', 'server', 'Xenify server', '', 'server connect account sign in log in share shared files office team network together collaborate cloud ai ask assistant', { wide: true })] },
  ] },
  { id: 'saving', name: 'Saving', about: 'File types, change notes and getting work back.', groups: [
    { title: 'Files', rows: [
      row('format', 'select', 'Save new documents as', 'Markdown opens anywhere; Word keeps every layout detail.', 'default format file type docx markdown word save as xwrite igdoc', { options: [['md', 'Markdown (.md)'], ['docx', 'Word (.docx)'], ['xwrite', 'Xenify Write document (.xwrite)']], ...path('save_as') }),
      row('changelog', 'toggle', 'Ask what changed each time I save', 'Adds who changed what, and when, to the document. Insert › Change log shows the list.', 'change log history version notes record', path('ask_change_note'))] },
    { title: 'If Write closes unexpectedly', rows: [
      row('recovery', 'info', 'Unsaved work is kept every minute', 'Kept apart from your file and offered back the next time you open Write.', 'autosave backup recovery crash restore lost')] },
  ] },
];

if (window.XENIFY_WEB) {
  // Unsaved work is kept in this browser; files can't be reopened by themselves.
  const saving = SECTIONS.find((x) => x.id === 'saving');
  const kept = saving.groups.find((g) => g.title.startsWith('If Write closes'));
  if (kept) { kept.title = 'If the tab closes'; kept.rows[0].desc = 'Kept in this browser (not on any server) and offered back next time you open Write here.'; }
  const start = SECTIONS[0].groups.find((g) => g.title === 'Starting up').rows[0];
  start.options = start.options.filter(([v]) => v !== 'reopen');
  // The mistake spotter runs in this browser; it's downloaded once and kept.
  const checking = SECTIONS.find((x) => x.id === 'spelling').groups[0];
  checking.rows.find((r) => r.id === 'spotter').desc = () => `An offline model, run in this browser, that finds what the rules can’t: missing words (“pass me salt”), wrong words and some word order. It’s a 67 MB download, kept in this browser. ${spotterStatus}`;
  // Working offline: what's kept in this browser.
  SECTIONS[0].groups.push({ title: 'Working offline', rows: [
    row('offline', 'button', 'Keep Write in this browser', () => `Write works without a connection once it has been opened here. ${offlineStatus}`, 'offline internet connection storage space cache download install', { btn: 'Free up space', run: 'forget_offline' })] });
}

const INDEX = [];
for (const s of SECTIONS) for (const g of s.groups) for (const r of g.rows) INDEX.push({ r, s, g });

function score(e, words, prefs) {
  const label = e.r.label.toLowerCase();
  const desc = typeof e.r.desc === 'function' ? e.r.desc(prefs) : e.r.desc;
  const hay = `${e.r.keys} ${desc} ${e.g.title} ${e.s.name}`.toLowerCase();
  let total = 0;
  for (const w of words) {
    let s = 0;
    if (label.startsWith(w)) s = 4; else if (label.includes(` ${w}`)) s = 3; else if (label.includes(w)) s = 2;
    else if (` ${hay}`.includes(` ${w}`)) s = 1.5; else if (hay.includes(w)) s = 0.5;
    if (!s) return 0;
    total += s;
  }
  return total;
}

/// Opens Settings. `prefs` is the current settings; `onChange(prefs)` is
/// called after every change (already saved).
export function openSettings({ prefs, onChange, onClose, section = 'general' }) {
  const extra = { profiles: [], words: [] };
  window.XENIFY_OFFLINE?.keptSize().then((n) => {
    offlineStatus = n ? `${(n / 1e6).toFixed(0)} MB is kept now. Freeing it up means downloading it again next time.` : 'Nothing is kept yet.';
    renderMain();
  });
  window.__TAURI__.core.invoke('spotter_status').then((t) => { spotterStatus = t ? `(${t}.)` : ''; renderMain(); }).catch(() => {});
  Promise.all([call('details'), myWords.get()]).then(([d, w]) => {
    extra.profiles = d.profiles;
    extra.words = w;
    extra.docWords = d.words || [];
    renderMain();
  });
  // The browser app served by a server is already using it (signing in is on
  // its Files page): no Server section there.
  const sections = SECTIONS.filter((s) => !(s.id === 'server' && ui.server?.servedHere));
  const index = INDEX.filter((e) => sections.includes(e.s));
  if (!sections.some((s) => s.id === section)) section = 'general';
  const state = { section, hl: null, query: '', open: false, active: 0 };
  let p = structuredClone(prefs);

  const root = h('div', { class: 'screen settings', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Settings' });
  const nav = h('div', { class: 'set-nav-list' });
  const results = h('div', { class: 'set-results', role: 'listbox', id: 'set-results', 'aria-label': 'Matching settings', hidden: true });
  const search = h('input', { class: 'field set-search', type: 'search', role: 'combobox', 'aria-expanded': 'false', 'aria-controls': 'set-results', autocomplete: 'off', placeholder: 'Search, e.g. dark, quotes, bigger', 'aria-label': 'Search settings' });
  const main = h('main', { class: 'set-main' });

  const close = () => { root.remove(); document.removeEventListener('keydown', keys, true); onClose?.(); };
  const keys = (e) => {
    // (A dialog open over Settings, like Edit kinds, has Escape first.)
    if (e.key === 'Escape' && !state.open && !document.querySelector('.modal-back')) { e.preventDefault(); e.stopPropagation(); close(); }
  };
  document.addEventListener('keydown', keys, true);

  root.append(
    h('nav', { class: 'set-nav', 'aria-label': 'Settings sections' },
      h('div', { class: 'set-title' }, h('h1', {}, 'Settings'), h('button', { class: 'btn small', onClick: close }, 'Done')),
      h('div', { class: 'set-search-wrap' }, h('span', { class: 'search-icon', html: icon('search', 18) }), search, results),
      nav,
      h('p', { class: 'set-foot' }, 'Changes save as you make them. Settings that belong to one document are in its details, not here.')),
    main);
  document.body.append(root);

  async function commit() {
    p = await call('set_prefs', { prefs: p });
    onChange(p);
  }

  function renderNav() {
    fill(nav, ...sections.map((s) => h('button', {
      class: `set-nav-btn${s.id === state.section ? ' on' : ''}`, 'aria-current': s.id === state.section ? 'page' : 'false',
      html: `${svg(D[s.id])}<span>${s.name}</span>`,
      onClick: () => { state.section = s.id; state.hl = null; renderAll(); },
    })));
  }

  function control(r) {
    const v = r.get ? r.get(p) : null;
    const set = (x) => { r.set(p, x); if (state.hl !== r.id) state.hl = null; renderMain(); commit(); };
    switch (r.kind) {
      case 'toggle':
        return h('button', { class: `switch${v ? ' on' : ''}`, role: 'switch', 'aria-checked': String(!!v), 'aria-label': r.label, onClick: () => set(!v) });
      case 'select': {
        const opts = r.dynamic === 'profiles' ? extra.profiles.map((x) => [x.id, x.name]) : r.options;
        const sel = h('select', { class: 'field set-select', 'aria-label': r.label }, opts.map(([val, name]) => h('option', { value: val, selected: val === v }, name)));
        sel.addEventListener('change', () => set(sel.value));
        return sel;
      }
      case 'seg':
        return h('div', { class: 'segs set-segs', role: 'radiogroup', 'aria-label': r.label }, r.options.map(([val, name]) => h('button', {
          role: 'radio', 'aria-checked': String(val === v), class: `seg${val === v ? ' on' : ''}`, onClick: () => set(val),
        }, name)));
      case 'text': {
        const input = h('input', { class: 'field set-text', 'aria-label': r.label, placeholder: r.eg, value: v || '' });
        input.addEventListener('change', () => { r.set(p, input.value); commit(); });
        return input;
      }
      case 'slider': {
        const out = h('span', { class: 'slider-val' }, `${v}${r.unit}`);
        const input = h('input', { type: 'range', min: r.min, max: r.max, step: r.step, value: v, 'aria-label': r.label });
        input.addEventListener('input', () => { out.textContent = `${input.value}${r.unit}`; });
        input.addEventListener('change', () => { r.set(p, Number(input.value)); commit(); });
        return h('div', { class: 'slider' }, input, out);
      }
      case 'button':
        return h('button', { class: 'btn small', onClick: async () => { p = await call(r.run); onChange(p); renderMain(); } }, r.btn);
      case 'info':
        return null;
      case 'kinds':
        return h('button', { class: 'btn small', onClick: () => kindsDialog({ h, fill, onClose: async () => { const d = await call('details'); extra.profiles = d.profiles; renderMain(); } }) }, 'Edit kinds…');
      case 'server':
        return serverRow(renderMain);
      case 'words': {
        const input = h('input', { class: 'field', placeholder: 'Add a word, e.g. Okafor', 'aria-label': 'Add a word', style: 'flex: 1 1 220px' });
        const add = async () => {
          const w = input.value.trim();
          if (!w) return;
          if (!extra.words.includes(w)) extra.words = [w, ...extra.words];
          input.value = '';
          await myWords.set(extra.words);
          renderMain();
          main.querySelector('#row-wordlist input')?.focus();
        };
        input.addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });
        const list = extra.words.slice().sort((a, b) => a.localeCompare(b));
        const chips = (ws, remove) => h('div', { class: 'word-chips' }, ws.map((w) => h('span', { class: 'word' }, w, h('button', {
          class: 'icon-btn', 'aria-label': `Remove ${w}`, html: icon('x', 15), onClick: () => remove(w),
        }))));
        const docList = (extra.docWords || []).slice().sort((a, b) => a.localeCompare(b));
        const where = myWords.where();
        return h('div', { class: 'words' },
          where ? h('p', { class: 'set-desc' }, `Your words on ${where}: they follow you to every computer you sign in on.`) : null,
          h('div', { class: 'words-add' }, input, h('button', { class: 'btn small', onClick: add }, 'Add word')),
          list.length
            ? chips(list, async (w) => { extra.words = extra.words.filter((x) => x !== w); await myWords.set(extra.words); renderMain(); })
            : h('p', { class: 'set-desc' }, 'No words yet.'),
          docList.length ? h('b', {}, 'This document’s words') : null,
          docList.length ? h('p', { class: 'set-desc' }, 'Right in this document only, for everyone who edits it.') : null,
          docList.length ? chips(docList, async (w) => { await call('doc_word', { word: w, remove: true }); extra.docWords = extra.docWords.filter((x) => x !== w); renderMain(); }) : null);
      }
      case 'pairs': {
        const list = v;
        const a = h('input', { class: 'field', placeholder: 'e.g. adn', 'aria-label': 'When I type' });
        const b = h('input', { class: 'field', placeholder: 'e.g. and', 'aria-label': 'Change it to' });
        const add = () => {
          if (!a.value.trim() || !b.value) return;
          r.set(p, [[a.value.trim(), b.value], ...list.filter(([x]) => x !== a.value.trim())]);
          renderMain(); commit();
        };
        b.addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });
        return h('div', { class: 'pairs' },
          h('div', { class: 'pair head' }, h('span', {}, 'When I type'), h('span', {}, 'Change it to'), h('span')),
          h('div', { class: 'pair' }, a, b, h('button', { class: 'btn small', 'aria-label': 'Add replacement', onClick: add }, 'Add')),
          h('div', { class: 'pair-list' }, list.map(([x, y], i) => h('div', { class: 'pair' },
            h('span', { class: 'cell mono' }, x), h('span', { class: 'cell' }, y),
            h('button', { class: 'icon-btn', 'aria-label': `Remove ${x}`, html: icon('x', 16), onClick: () => { r.set(p, list.filter((_, j) => j !== i)); renderMain(); commit(); } })))));
      }
      default:
        return null;
    }
  }

  function renderMain() {
    const sec = sections.find((s) => s.id === state.section);
    fill(main, h('div', { class: 'set-inner' },
      h('header', {}, h('h2', {}, sec.name), h('p', { class: 'set-about' }, sec.about)),
      sec.groups.map((g) => h('section', { class: 'set-group' },
        h('h3', {}, g.title),
        g.note ? h('p', { class: 'set-note' }, g.note) : null,
        h('div', { class: 'set-rows' }, g.rows.map((r) => {
          const desc = typeof r.desc === 'function' ? r.desc(p) : r.desc;
          return h('div', { id: `row-${r.id}`, class: `set-row${r.wide ? ' wide' : ''}${state.hl === r.id ? ' hl' : ''}` },
            h('div', { class: 'set-label' }, h('div', { class: 'set-name' }, r.label), desc ? h('div', { class: 'set-desc' }, desc) : null),
            control(r));
        }))))));
    if (state.hl) {
      const el = main.querySelector(`#row-${state.hl}`);
      el?.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches || p.reduce_motion ? 'auto' : 'smooth' });
    }
  }

  function found() {
    const words = state.query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return [];
    return index.map((e) => ({ e, s: score(e, words, p) })).filter((x) => x.s > 0).sort((a, b) => b.s - a.s).slice(0, 7).map((x) => x.e);
  }

  function pick(e) {
    state.section = e.s.id;
    state.hl = e.r.id;
    state.open = false;
    renderAll();
    // Then focus the setting itself, so the keyboard can change it at once.
    main.querySelector(`#row-${e.r.id} button, #row-${e.r.id} input, #row-${e.r.id} select`)?.focus({ preventScroll: true });
  }

  function renderResults() {
    const list = found();
    const q = state.query.trim().toLowerCase();
    const show = state.open && q.length > 0;
    results.hidden = !show;
    search.setAttribute('aria-expanded', String(show));
    if (!show) return;
    if (!list.length) {
      fill(results, h('div', { class: 'res-none' }, `No setting matches “${state.query}”.`, h('br'), 'Try ', h('b', {}, 'theme'), ', ', h('b', {}, 'quotes'), ' or ', h('b', {}, 'bigger'), '.'));
      return;
    }
    state.active = Math.min(state.active, list.length - 1);
    fill(results, ...list.map((e, i) => {
      const at = e.r.label.toLowerCase().indexOf(q);
      const label = at >= 0
        ? [e.r.label.slice(0, at), h('mark', {}, e.r.label.slice(at, at + q.length)), e.r.label.slice(at + q.length)]
        : [e.r.label];
      const b = h('button', { class: `res${i === state.active ? ' on' : ''}`, role: 'option', 'aria-selected': String(i === state.active) },
        h('span', { class: 'res-label' }, ...label), h('span', { class: 'res-path' }, `${e.s.name} › ${e.g.title}`));
      b.addEventListener('mousedown', (ev) => { ev.preventDefault(); pick(e); });
      return b;
    }), h('div', { class: 'res-keys' }, h('span', {}, '↑ ↓ to move'), h('span', {}, 'Enter to open'), h('span', {}, 'Esc to close')));
  }

  search.addEventListener('input', () => { state.query = search.value; state.open = true; state.active = 0; renderResults(); });
  search.addEventListener('focus', () => { state.open = true; renderResults(); });
  search.addEventListener('blur', () => { state.open = false; renderResults(); });
  search.addEventListener('keydown', (e) => {
    const list = found();
    if (e.key === 'ArrowDown' && list.length) { e.preventDefault(); state.active = (state.active + 1) % list.length; state.open = true; renderResults(); }
    else if (e.key === 'ArrowUp' && list.length) { e.preventDefault(); state.active = (state.active - 1 + list.length) % list.length; renderResults(); }
    else if (e.key === 'Enter' && list.length) { e.preventDefault(); pick(list[state.active] || list[0]); }
    else if (e.key === 'Escape' && state.open && state.query) { e.preventDefault(); e.stopPropagation(); state.open = false; renderResults(); }
  });

  function renderAll() { renderNav(); renderMain(); renderResults(); }
  renderAll();
  setTimeout(() => search.focus(), 0);
  return { search: (q) => { search.value = q; state.query = q; state.open = true; renderResults(); } };
}
