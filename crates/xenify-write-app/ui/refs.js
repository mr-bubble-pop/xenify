// References: citations, the sources they point to, and the bibliography.

import { call } from './engine.js';

/// Opens the citation picker; `run` makes edits (and redraws), `h` builds elements.
export async function citeDialog({ h, run, toast, modalShell }) {
  let data = await call('sources');
  const chosen = new Set();
  const search = h('input', { class: 'field', type: 'search', placeholder: 'Find a source by author, title or year', 'aria-label': 'Find a source' });
  const locator = h('input', { class: 'field', placeholder: 'e.g. p. 12', 'aria-label': 'Page or chapter (optional)' });
  const list = h('div', { class: 'src-list' });
  const draw = () => {
    const q = search.value.trim().toLowerCase();
    const shown = data.sources.filter((x) => !q || x.entry.toLowerCase().includes(q));
    list.replaceChildren(...(shown.length ? shown.map((x) => {
      const box = h('input', { type: 'checkbox', checked: chosen.has(x.source.key) });
      box.addEventListener('change', () => { if (box.checked) chosen.add(x.source.key); else chosen.delete(x.source.key); });
      return h('label', { class: 'src-item' }, box, h('span', {}, x.entry));
    }) : [h('p', { class: 'nav-empty' }, data.sources.length ? `No source matches “${search.value}”.` : 'No sources yet. Add one below, or paste BibTeX from your reference manager.')]));
  };
  search.addEventListener('input', draw);
  draw();
  const { close, body } = modalShell('Insert citation', [
    search, list,
    h('div', { class: 'form-grid' }, h('div', {}, h('label', { class: 'lbl' }, 'Page or chapter (optional)'), locator),
      h('div', { style: 'display: flex; align-items: flex-end; gap: 8px' },
        h('button', { class: 'btn', onClick: async () => { await sourceForm({ h, run, toast, modalShell }, null); data = await call('sources'); draw(); } }, 'Add a source…'))),
  ], [
    ['Cancel', null],
    ['Insert citation', async () => {
      if (!chosen.size) { toast('Tick a source to cite'); return false; }
      await run('cite', { keys: [...chosen], locator: locator.value });
      return true;
    }, true],
  ]);
  void body;
  search.focus();
  return close;
}

const FIELDS = [
  ['title', 'Title'], ['year', 'Year'], ['container', 'Journal, book or website'], ['publisher', 'Publisher'],
  ['volume', 'Volume'], ['issue', 'Issue'], ['pages', 'Pages'], ['url', 'Web address'], ['doi', 'DOI'], ['accessed', 'Read on (for web pages)'],
];

/// Add (`src` null) or edit a source. Resolves when saved or cancelled.
export async function sourceForm({ h, run, modalShell }, src) {
  const data = await call('sources');
  const s = src ? structuredClone(src) : { key: '', kind: 'book', authors: [] };
  const kind = h('select', { class: 'field', 'aria-label': 'Kind of source' }, data.kinds.map((k) => h('option', { value: k.id, selected: k.id === s.kind }, k.name)));
  const authors = h('textarea', { class: 'field area', rows: 2, 'aria-label': 'Authors', placeholder: 'One per line, e.g. Smith, Jane' });
  authors.value = (s.authors || []).map((a) => (a.given ? `${a.family}, ${a.given}` : a.family)).join('\n');
  const inputs = Object.fromEntries(FIELDS.map(([k]) => [k, h('input', { class: 'field', value: s[k] || '', 'aria-label': k })]));
  return new Promise((resolve) => {
    modalShell(src ? 'Edit source' : 'Add a source', [
      h('div', { class: 'form-grid' },
        h('div', {}, h('label', { class: 'lbl' }, 'Kind'), kind),
        h('div', {}, h('label', { class: 'lbl' }, 'Authors'), authors),
        FIELDS.map(([k, label]) => h('div', {}, h('label', { class: 'lbl' }, label), inputs[k]))),
    ], [
      ['Cancel', () => { resolve(null); return true; }],
      ['Save source', async () => {
        const out = {
          ...s, kind: kind.value,
          authors: authors.value.split('\n').map((l) => l.trim()).filter(Boolean).map((l) => {
            const [family, given] = l.includes(',') ? l.split(',').map((x) => x.trim()) : [l.split(' ').slice(-1)[0], l.split(' ').slice(0, -1).join(' ')];
            return { family, given: given || '' };
          }),
        };
        for (const [k] of FIELDS) out[k] = inputs[k].value.trim();
        if (!out.title) return false;
        await run('save_source', { source: out });
        resolve(out);
        return true;
      }, true],
    ]);
    inputs.title.focus();
  });
}

/// References › Sources: everything the document can cite.
export async function sourcesDialog(env) {
  const { h, run, toast, modalShell } = env;
  let data = await call('sources');
  const list = h('div', { class: 'src-list tall' });
  const style = h('select', { class: 'field', 'aria-label': 'Citation style' }, data.styles.map((x) => h('option', { value: x.id, selected: x.id === data.style }, x.name)));
  style.addEventListener('change', async () => { await run('cite_style', { style: style.value }); data = await call('sources'); draw(); });
  const paste = h('textarea', { class: 'field area mono', rows: 3, placeholder: 'Paste BibTeX (@book{…}) or RIS (TY  - JOUR…) from your reference manager', 'aria-label': 'Paste sources' });
  const draw = () => {
    list.replaceChildren(...(data.sources.length ? data.sources.map((x) => h('div', { class: 'src-item' },
      h('span', {}, x.entry, x.cited ? h('span', { class: 'cited' }, ' · cited') : null),
      h('span', { class: 'acts' },
        h('button', { class: 'link', onClick: async () => { await sourceForm(env, x.source); data = await call('sources'); draw(); } }, 'Edit'),
        h('button', { class: 'link', onClick: async () => { await run('delete_source', { key: x.source.key }); data = await call('sources'); draw(); } }, 'Delete')))) : [h('p', { class: 'nav-empty' }, 'No sources yet.')]));
  };
  draw();
  modalShell('Sources', [
    h('div', { class: 'form-grid' }, h('div', {}, h('label', { class: 'lbl' }, 'Citation style'), style),
      h('div', { style: 'display: flex; align-items: flex-end' }, h('button', { class: 'btn', onClick: async () => { await sourceForm(env, null); data = await call('sources'); draw(); } }, 'Add a source…'))),
    list,
    h('label', { class: 'lbl', style: 'margin-top: 12px' }, 'Import'), paste,
    h('button', { class: 'btn', style: 'margin-top: 8px', onClick: async () => {
      try { const out = await call('import_sources', { text: paste.value }); toast(`Added ${out.added} ${out.added === 1 ? 'source' : 'sources'}`); paste.value = ''; await run('view'); data = await call('sources'); draw(); } catch (e) { toast(String(e)); }
    } }, 'Add these'),
  ], [['Close', null, true]]);
}
