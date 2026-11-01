// Ask AI (Review › Ask AI), with the AI of the Xenify server Write is signed
// in to. Ask anything about the document, or rewrite, summarise or
// get feedback on the selection, the paragraph the cursor is in, or the
// whole document. The document changes only when the person presses Apply or
// Replace on something the AI proposed; nothing is sent until they've agreed once.

import { aiServer } from './aiserver.js';

const invoke = (name, args = {}) => window.__TAURI__.core.invoke('ai', { name, args });

const HOW = [
  ['Shorter', 'Make it shorter. Keep every fact.'],
  ['Clearer', 'Make it clearer and easier to follow.'],
  ['More formal', 'Make it more formal.'],
  ['Friendlier', 'Make it friendlier and warmer.'],
  ['Simpler words', 'Use simpler, everyday words.'],
];
// Questions to start from (any question can be typed).
const ASK = ['Is this clear to my readers?', 'What’s missing?', 'Make this paragraph clearer', 'Is the tone right?', 'Suggest a better title'];

let agreed = false;
// The conversation, kept while Write is open (per document tab).
const chats = new Map();

/// A reply as tidy text: **bold**, *italic*, "> " quotes and "- " lists drawn
/// as such (built from elements, never as HTML, so a reply can't add anything
/// to the page).
function replyNodes(h, text) {
  const inline = (line) => {
    const out = [];
    const re = /\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*/g;
    let at = 0, m;
    while ((m = re.exec(line))) {
      out.push(line.slice(at, m.index), m[1] != null ? h('b', {}, m[1]) : h('i', {}, m[2]));
      at = m.index + m[0].length;
    }
    out.push(line.slice(at));
    return out;
  };
  const blocks = [];
  let list = null;
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd();
    if (/^\s*[-*•] /.test(line)) {
      if (!list) { list = h('ul', { class: 'ai-list' }); blocks.push(list); }
      list.append(h('li', {}, inline(line.replace(/^\s*[-*•] /, ''))));
      continue;
    }
    list = null;
    if (!line.trim()) continue;
    if (line.startsWith('>')) blocks.push(h('blockquote', { class: 'ai-quote' }, inline(line.replace(/^>\s?/, ''))));
    else blocks.push(h('p', {}, inline(line.replace(/^#+\s*/, ''))));
  }
  return blocks;
}

export async function aiDialog(env) {
  const { h, fill, run, ask, modalShell, hasSelection, selectedText, details, changeDetails, tab } = env;
  const status = await invoke('status');
  const chat = chats.get(tab) || [];
  chats.set(tab, chat);
  const st = { scope: hasSelection ? 'selection' : 'paragraph', what: 'ask', how: HOW[1][1], busy: false, result: null, error: null };
  const body = h('div', { class: 'ai-dialog' });
  let shell = null;

  const consent = async () => {
    if (agreed) return true;
    const where = status.server ? ` (${status.server.url.replace(/^https?:\/\//, '')})` : '';
    const to = `The text you chose goes to the AI on your Xenify server${where}, which the people running it can read. Nothing else leaves your computer.`;
    const choice = await ask('Send text to the AI service?', to,
      [['yes', 'Send it', 'main'], ['no', 'Cancel']]);
    agreed = choice === 'yes';
    return agreed;
  };
  const seg = (label, options, cur, set) => h('div', {},
    h('div', { class: 'lbl' }, label),
    h('div', { class: 'segs', role: 'radiogroup', 'aria-label': label }, options.map(([v, n, disabled]) => h('button', {
      role: 'radio', 'aria-checked': String(v === cur), class: `seg${v === cur ? ' on' : ''}`, disabled: !!disabled,
      onClick: () => { set(v); st.result = null; st.error = null; draw(); } }, n))));

  const go = async (question) => {
    if (st.busy || !(await consent())) return;
    st.busy = true; st.result = null; st.error = null; draw();
    try {
      if (st.what === 'ask') {
        // Your message shows straight away, with the reply on its way under it.
        chat.push({ role: 'user', text: question });
        draw();
        const r = await invoke('ask', { history: chat.filter((m) => !m.error).map(({ role, text }) => ({ role, text })), scope: st.scope });
        chat.push({ role: 'assistant', text: r.reply || (r.proposals.length ? 'Here’s a change you could make:' : '…'),
          proposals: r.proposals.map((p) => ({ ...p, table: r.table })) });
      } else if (st.what === 'rewrite') st.result = { kind: 'rewrite', ...(await invoke('rewrite', { how: st.how })) };
      else if (st.what === 'summary') st.result = { kind: 'summary', text: (await invoke('summarise', { scope: st.scope })).reply };
      else st.result = { kind: 'feedback', list: (await invoke('review', { scope: st.scope })).suggestions };
    } catch (e) { st.error = String(e); }
    st.busy = false;
    draw();
  };

  // A table change: the new rows or changed cells as a small table, applied on Apply.
  const tableProposal = (p, done) => h('div', { class: 'ai-result' },
    p.rows?.length
      ? h('table', { class: 'ai-table' }, h('tbody', {}, p.rows.map((r) => h('tr', {}, r.map((c) => h('td', {}, c))))))
      : h('div', { class: 'diff' }, p.cells.map(([r, c, t]) => h('div', {}, `Row ${r}, column ${c}: `, h('b', {}, t)))),
    p.reason ? h('div', { class: 'ai-note' }, p.reason) : null,
    h('div', { class: 'acts' }, h('button', { class: 'btn small main', disabled: p.table == null, onClick: async () => {
      if (await run('ai_table', { table: p.table, rows: p.rows || [], cells: p.cells || [], at: p.at ?? null })) done();
    } }, p.rows?.length ? (p.rows.length === 1 ? 'Add the row' : `Add the ${p.rows.length} rows`) : 'Change the cells')));

  // A change the AI proposed: shown, applied only on Apply.
  const proposal = (p, done) => (p.rows?.length || p.cells?.length) ? tableProposal(p, done) : h('div', { class: 'ai-result' },
    p.quote ? h('div', { class: 'diff' }, h('s', {}, p.quote), ' → ', h('b', {}, p.replacement)) : h('div', { class: 'diff' }, h('b', {}, p.replacement)),
    p.reason ? h('div', { class: 'ai-note' }, p.reason) : null,
    h('div', { class: 'acts' }, h('button', { class: 'btn small main', onClick: async () => {
      if (p.quote) await run('ai_apply_quote', { quote: p.quote, new: p.replacement });
      else await run('paste', { text: p.replacement });
      done();
    } }, p.quote ? 'Apply' : 'Put it in')));

  const result = () => {
    const r = st.result;
    if (!r) return null;
    if (r.kind === 'rewrite') {
      return h('div', { class: 'ai-result' },
        h('div', { class: 'diff' }, r.diff.map((p) => (p.same != null ? p.same : p.removed != null ? h('s', {}, p.removed) : h('b', {}, p.added)))),
        h('div', { class: 'acts' },
          h('button', { class: 'btn main', onClick: async () => { shell.close(); await run('ai_apply', { para: r.para, start: r.start, end: r.end, old: r.old, new: r.new }); } }, 'Replace it'),
          h('button', { class: 'btn', onClick: () => go() }, 'Try again')));
    }
    if (r.kind === 'summary') {
      return h('div', { class: 'ai-result' }, h('div', { class: 'ai-text' }, replyNodes(h, r.text)),
        h('div', { class: 'acts' }, h('button', { class: 'btn', onClick: () => navigator.clipboard.writeText(r.text) }, 'Copy')));
    }
    if (!r.list.length) return h('p', { class: 'ai-note' }, 'Nothing to change. Nice writing.');
    return h('div', { class: 'ai-feedback' }, r.list.map((s, k) => h('div', { class: 'ai-result' },
      s.quote ? h('div', { class: 'quote' }, `“${s.quote}”`) : null,
      h('div', {}, s.explanation),
      s.replacement ? h('div', { class: 'acts' }, h('div', { class: 'diff' }, '→ ', h('b', {}, s.replacement)),
        h('button', { class: 'btn small main', onClick: async () => { await run('ai_apply', { para: s.para, start: s.start, end: s.end, old: s.quote, new: s.replacement }); r.list.splice(k, 1); draw(); } }, 'Apply')) : null)));
  };

  const askPart = () => {
    const q = h('textarea', { class: 'field area', rows: 2, placeholder: 'Ask about your writing, e.g. “Is the second paragraph clear?” or “Make the intro friendlier”', 'aria-label': 'Your question' });
    const send = () => { const t = q.value.trim(); if (t) go(t); };
    q.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } });
    setTimeout(() => q.focus(), 0);
    return [
      chat.length ? h('div', { class: 'ai-chat' }, chat.map((m) => h('div', { class: `bubble ${m.role}` }, m.role === 'assistant' ? replyNodes(h, m.text) : m.text,
        (m.proposals || []).map((p, k) => proposal(p, () => { m.proposals.splice(k, 1); m.applied = (m.applied || 0) + 1; draw(); })),
        m.applied ? h('div', { class: 'ai-note' }, m.applied === 1 ? 'Changed in the document.' : `${m.applied} changes made in the document.`) : null)),
        st.busy && st.what === 'ask' ? h('div', { class: 'bubble assistant thinking', 'aria-live': 'polite' }, h('span', { class: 'dots', 'aria-hidden': 'true' }, h('i'), h('i'), h('i')), 'Thinking…') : null) : null,
      chat.length ? null : h('div', { class: 'chips' }, ASK.map((t) => h('button', { class: 'chip', onClick: () => go(t) }, t))),
      h('div', { class: 'ai-row' }, q, h('button', { class: 'btn main', disabled: st.busy, onClick: send }, st.busy ? 'Waiting…' : 'Ask')),
      chat.length ? h('button', { class: 'link', onClick: () => { chat.length = 0; draw(); } }, 'Start again') : null,
    ];
  };

  // Which text will be sent, so it's never a surprise (the paragraph above
  // when the cursor is on a new empty line).
  const sending = h('p', { class: 'ai-sending' });
  // In a table, "this paragraph" is the whole table.
  let inTable = false;
  const showSending = async () => {
    const p = await invoke('preview', { scope: st.scope }).catch(() => null);
    if (!p) return;
    if (st.scope === 'paragraph' && !!p.table !== inTable) { inTable = !!p.table; draw(); }
    sending.textContent = !p.words ? 'There’s no text there yet.'
      : st.scope === 'whole' ? `The whole document (${p.words} words).`
      : p.table && st.scope !== 'selection' ? `The table (${p.table} rows, ${p.words} words).`
      : `“${p.start}${p.more ? '…' : ''}” (${p.words} ${p.words === 1 ? 'word' : 'words'})`;
  };
  let shownScope = null;
  const draw = () => {
    if (shownScope !== st.scope) { shownScope = st.scope; showSending(); }
    let custom = null;
    const words = selectedText ? selectedText.trim().split(/\s+/).length : 0;
    const reader = details.reader ? details.reader.toLowerCase() : 'anyone';
    fill(body,
      // Answers and rewrites suit these readers and tone (only Ask AI uses them).
      h('p', { class: 'ai-note' }, 'Writing for ', h('b', {}, reader), details.tone ? [', in a ', h('b', {}, details.tone.toLowerCase()), ' tone'] : null, '. ',
        h('button', { class: 'link', onClick: () => { shell.close(); changeDetails(); } }, 'Change')),
      seg('Do', [['ask', 'Ask'], ['rewrite', 'Rewrite', st.scope === 'whole'], ['summary', 'Summarise'], ['feedback', 'Feedback']], st.what, (v) => { st.what = v; }),
      seg('On', [['selection', words ? `The selection (${words} ${words === 1 ? 'word' : 'words'})` : 'The selection', !hasSelection], ['paragraph', inTable ? 'This table' : 'This paragraph'], ['whole', 'The whole document']],
        st.scope, (v) => { st.scope = v; if (v === 'whole' && st.what === 'rewrite') st.what = 'feedback'; }),
      sending,
      st.what === 'ask' ? askPart() : [
        st.what === 'rewrite' ? h('div', {},
          h('div', { class: 'lbl' }, 'How'),
          h('div', { class: 'chips' }, HOW.map(([n, how]) => h('button', { class: `chip${st.how === how ? ' on' : ''}`, onClick: () => { st.how = how; draw(); } }, n))),
          custom = h('input', { class: 'field', style: 'margin-top: 8px', placeholder: 'Or say how, e.g. “for a 10-year-old”', 'aria-label': 'How to rewrite it' })) : null,
        h('div', { class: 'ai-go' },
          h('button', { class: 'btn main', disabled: st.busy, onClick: () => { if (custom?.value.trim()) st.how = custom.value.trim(); go(); } },
            st.busy ? 'Waiting for the reply…' : st.what === 'rewrite' ? 'Rewrite' : st.what === 'summary' ? 'Summarise' : 'Get feedback')),
        result()],
      st.error ? h('div', { class: 'ai-error', role: 'alert' }, st.error) : null,
      h('p', { class: 'ai-foot' }, `${aiServer.info?.text_model ? `Model: ${aiServer.info.text_model}, chosen by your server. ` : ''}Only the text you choose is sent, and only when you press a button. Your document changes only when you press Apply or Replace.`));
  };
  draw();
  shell = modalShell('Ask AI', body, [['Close', null]]);
}
