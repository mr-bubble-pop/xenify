// Kinds of writing: what the checker points out for an essay, an email, a
// story… The built-in kinds can be copied and the copy changed.

import { call } from './engine.js';

const CHECKS = [
  ['passive_voice', 'Passive voice', '“The report was written” instead of “We wrote the report”'],
  ['adverbs', 'Adverbs', 'really, very, extremely'],
  ['wordy_phrases', 'Wordy phrases', '“in order to”, “at this point in time”'],
  ['clarity', 'Clearer sentences', 'Long lead-ins, piled-up nouns, vague “this”'],
  ['main_point_first', 'Main point first', 'Says so when the request comes late'],
  ['define_acronyms', 'Spell out abbreviations', 'The first time an abbreviation is used'],
  ['consistency', 'Consistency', '“organise” and “organize”, “3” and “three” in one document'],
  ['word_choice', 'Word choice', 'Clichés, weak intensifiers, sentences that all start the same'],
  ['optional_commas', 'Optional commas', 'After “However”, after a greeting, the Oxford comma'],
  ['sentence_sense', 'Mistakes the rules miss', 'The small model on this computer (if installed)'],
];
const LEVELS = [['allow', 'Fine'], ['note', 'Gentle note'], ['flag', 'Point out']];

function alertBox(text) {
  const t = document.createElement('div');
  t.className = 'toast';
  t.setAttribute('role', 'alert');
  t.textContent = text;
  document.body.append(t);
  setTimeout(() => t.remove(), 4000);
}

export async function kindsDialog({ h, fill, onClose }) {
  let data = await call('kinds');
  let sel = data.kinds[0].id;
  let draft = null; // the kind being changed (a copy of it)
  const back = h('div', { class: 'modal-back' });
  const close = () => { back.remove(); document.removeEventListener('keydown', keys, true); onClose?.(); };
  // Escape closes; Enter saves the kind being changed, or is Done. Only when no question is open on top.
  const keys = (e) => {
    if (back !== [...document.querySelectorAll('.modal-back')].pop() || e.isComposing) return;
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === 'Enter' && !e.ctrlKey && !e.altKey && !e.metaKey && !e.target.closest?.('textarea, select, button, [contenteditable]:not([contenteditable="false"])')) {
      e.preventDefault(); e.stopPropagation();
      (pane.querySelector('.btn.main') || box.querySelector(':scope > .modal-actions .btn.main'))?.click();
    }
  };
  document.addEventListener('keydown', keys, true);
  const list = h('div', { class: 'kind-list', role: 'listbox', 'aria-label': 'Kinds of writing' });
  const pane = h('div', { class: 'kind-pane' });
  const box = h('div', { class: 'modal kinds', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Kinds of writing' },
    h('h2', {}, 'Kinds of writing'),
    h('p', {}, 'Each kind decides what gets pointed out. Built-in kinds can’t be changed, but you can copy one and change the copy.'),
    h('div', { class: 'kind-body' }, list, pane),
    h('div', { class: 'modal-actions', style: 'margin-top: 14px' }, h('button', { class: 'btn main', onClick: close }, 'Done')));
  back.append(box);
  document.body.append(back);

  const current = () => draft || data.kinds.find((k) => k.id === sel);
  const drawList = () => fill(list, data.kinds.map((k) => h('button', {
    class: `kind-item${k.id === sel ? ' on' : ''}`, role: 'option', 'aria-selected': String(k.id === sel),
    onClick: () => { sel = k.id; draft = null; drawAll(); },
  }, h('span', {}, k.name), k.built_in ? h('span', { class: 'tag' }, 'built in') : null)));

  const drawPane = () => {
    const k = current();
    const ro = k.built_in && !draft;
    const set = (fn) => { if (ro) return; if (!draft) draft = structuredClone(k); fn(draft); drawPane(); };
    const name = h('input', { class: 'field', value: k.name, disabled: ro, 'aria-label': 'Name' });
    name.addEventListener('input', () => { if (!draft) draft = structuredClone(k); draft.name = name.value; });
    const about = h('input', { class: 'field', value: k.description, disabled: ro, 'aria-label': 'What it’s for' });
    about.addEventListener('input', () => { if (!draft) draft = structuredClone(k); draft.description = about.value; });
    const long = h('input', { class: 'field', type: 'number', min: 10, max: 80, value: k.long_sentence, disabled: ro, style: 'width: 90px' });
    long.addEventListener('change', () => set((d) => { d.long_sentence = Number(long.value) || 25; }));
    fill(pane,
      h('div', { class: 'form-grid' }, h('div', {}, h('label', { class: 'lbl' }, 'Name'), name), h('div', {}, h('label', { class: 'lbl' }, 'What it’s for'), about)),
      h('div', { class: 'modal-actions', style: 'margin-top: 10px; justify-content: flex-start' },
        ro ? h('button', { class: 'btn', onClick: () => {
          const taken = data.kinds.map((x) => x.id);
          let n = `${k.name} (my copy)`, i = 2;
          while (data.kinds.some((x) => x.name === n)) n = `${k.name} (my copy ${i++})`;
          draft = { ...structuredClone(k), id: '', name: n, built_in: false };
          void taken;
          drawPane();
        } }, 'Make a copy to change') : null,
        !k.built_in && !draft ? h('button', { class: 'btn', onClick: async () => { await call('delete_kind', { id: k.id }); data = await call('kinds'); sel = data.kinds[0].id; drawAll(); } }, 'Delete') : null,
        draft ? h('button', { class: 'btn', onClick: () => { draft = null; drawAll(); } }, 'Discard changes') : null,
        draft ? h('button', { class: 'btn main', onClick: async () => {
          let out;
          try { out = await call('save_kind', { kind: draft }); } catch (e) { alertBox(String(e)); return; }
          data = await call('kinds');
          sel = out.id;
          draft = null;
          drawAll();
        } }, 'Save') : null),
      h('h3', {}, 'Informal writing'),
      data.features.map((f) => h('div', { class: 'kind-row' },
        h('div', {}, h('div', { class: 'set-name' }, f.name), h('div', { class: 'set-desc' }, f.about)),
        h('div', { class: 'segs', role: 'radiogroup', 'aria-label': f.name }, LEVELS.map(([lv, label]) => {
          const on = (k.features[f.id] || 'allow') === lv;
          return h('button', { role: 'radio', 'aria-checked': String(on), class: `seg${on ? ' on' : ''}`, disabled: ro, onClick: () => set((d) => { d.features[f.id] = lv; }) }, label);
        })))),
      h('h3', {}, 'Style'),
      CHECKS.map(([key, label, about2]) => h('div', { class: 'kind-row' },
        h('div', {}, h('div', { class: 'set-name' }, label), h('div', { class: 'set-desc' }, about2)),
        h('button', { class: `switch${k[key] ? ' on' : ''}`, role: 'switch', 'aria-checked': String(!!k[key]), 'aria-label': label, disabled: ro, onClick: () => set((d) => { d[key] = !d[key]; }) }))),
      h('div', { class: 'kind-row' }, h('div', {}, h('div', { class: 'set-name' }, 'A sentence is long after'), h('div', { class: 'set-desc' }, 'words')), long));
  };
  const drawAll = () => { drawList(); drawPane(); };
  drawAll();
}
