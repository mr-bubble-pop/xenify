// AI › Change this picture: describe a change to a picture or raster layer
// and an image model paints it, only inside the pixel selection when there is
// one (the rest stays exactly as it was), optionally with reference pictures
// ("this jacket"). The model's own words on what it changed show in a strip
// over the canvas, with Keep, Undo and Try again.

import { call, engineRaw, afterOffscreen } from './engine.js';
import * as platform from './platform.js';
import { hasAi, aiServer, pictureEditOnServer, chatTarget, noAi } from './aiprovider.js';
import { icon } from './icons.js';

const toB64 = (bytes) => { let bin = ''; for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return btoa(bin); };
const mimeOf = (bytes) => (bytes[0] === 0xff && bytes[1] === 0xd8 ? 'image/jpeg' : bytes[0] === 0x52 && bytes[8] === 0x57 ? 'image/webp' : bytes[0] === 0x47 ? 'image/gif' : 'image/png');

/// The last change asked for, per layer (Try again asks it again).
const last = new Map();

export async function aiEditDialog(app, h, { openSettings, modal }, again = null) {
  const l = app.raster.layer() || (app.state.selection.length === 1 ? app.layerById(app.state.selection[0]) : null);
  if (!l || l.kind !== 'picture') { app.toast('Select a picture or a raster layer to change (or Layer › Convert to raster first).'); return; }
  const info = call('raster_info', { id: l.id });
  if (!info.paintable) { app.toast(info.why); return; }
  if (!hasAi()) return void noAi(app.toast, openSettings);
  const model = aiServer.info?.image_model || 'the server’s picture model';
  const prev = again || last.get(l.id) || {};
  const st = { only: prev.only ?? true, keepAlpha: prev.keepAlpha ?? true, refs: again ? prev.refs : [] };

  let req = again;
  if (!again) {
    const text = h('textarea', { class: 'field', rows: '3', placeholder: info.selection ? 'e.g. “a red knitted scarf”, “remove the person”, “make it night”' : 'e.g. “change his jacket to the one in the reference”, “make it look like a watercolour”',
      'aria-label': 'What should change', style: 'resize: vertical; font: inherit; width: 100%',
      onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); document.querySelector('.modal-back .btn.main')?.click(); } } });
    text.value = prev.change || '';
    const refBox = h('div', { class: 'ai-refs' });
    const drawRefs = () => refBox.replaceChildren(
      ...st.refs.map((r, i) => h('div', { class: 'ai-ref' },
        h('img', { src: `data:${r.mime};base64,${r.b64}`, alt: r.name }),
        h('input', { class: 'field small', value: r.note, placeholder: 'What it shows, e.g. “the jacket”', 'aria-label': `What reference ${i + 1} shows`, onKeydown: (e) => e.stopPropagation(), onInput: (e) => { r.note = e.target.value; } }),
        h('button', { class: 'small-btn', 'aria-label': `Remove reference ${i + 1}`, title: 'Remove', html: icon('x', 14), onClick: () => { st.refs.splice(i, 1); drawRefs(); } }))),
      st.refs.length < 4 ? h('div', { class: 'prop' },
        h('button', { class: 'small-btn', onClick: async () => {
          let got;
          try { got = await platform.openPicture(); } catch (e) { app.toast(String(e.message || e)); return; }
          if (!got || got.type === 'image/svg+xml') return;
          st.refs.push({ name: got.name, b64: toB64(got.bytes), mime: mimeOf(got.bytes), note: '' });
          drawRefs();
        } }, '+ A picture file…'),
        h('select', { class: 'field small', style: 'width: auto', 'aria-label': 'A reference from a layer', onChange: (e) => {
          const id = Number(e.target.value);
          e.target.value = '';
          if (!id) return;
          try {
            const png = engineRaw().layer_thumb(BigInt(id), 768);
            st.refs.push({ name: app.layerById(id)?.name || 'Layer', b64: toB64(png), mime: 'image/png', note: '' });
          } catch (err) { app.toast(String(err.message || err)); } finally { afterOffscreen(); }
          drawRefs();
        } }, h('option', { value: '' }, '+ From a layer…'), app.state.layers.filter((x) => x.id !== l.id && !x.folder && x.kind !== 'sound' && x.kind !== 'camera').map((x) => h('option', { value: String(x.id) }, x.name)))) : null);
    drawRefs();
    const check = (label, key, tip) => h('label', { class: 'check-row', title: tip }, h('input', { type: 'checkbox', checked: st[key], onChange: (e) => { st[key] = e.target.checked; } }), h('span', {}, label));
    const ok = await modal('Change this picture', h('div', { class: 'ai-dialog', style: 'display: flex; flex-direction: column; gap: 10px' },
      h('label', {}, 'What should change?', text),
      info.selection ? check('Only inside the selection', 'only', 'The rest of the picture stays exactly as it is') : h('small', { class: 'muted' }, 'Tip: select an area first (magic wand, Select with AI, or the selection tool) to change only that part.'),
      info.selection ? h('small', { class: 'muted' }, 'Only the selected part can change: select all the room the change needs (for a jacket with sleeves, the arms too; Selection › Grow adds a margin).') : null,
      info.see_through ? check('Keep the see-through parts', 'keepAlpha', 'The layer’s outline stays as it is; only its colours change') : null,
      h('div', { class: 'lab' }, 'Reference pictures (optional)'),
      h('small', { class: 'muted' }, 'Show what you mean: a jacket to put on, a style to copy, a face to keep.'),
      refBox,
      h('small', { class: 'muted' }, `Painted by ${model}. About 10–40 seconds; it’s one step you can undo. Ctrl+Enter starts.`)),
    [['Cancel', false], ['Change it', true, true]]);
    const change = text.value.trim();
    if (!ok || !change) return;
    req = { change, only: st.only && !!info.selection, keepAlpha: st.keepAlpha && info.see_through, refs: st.refs };
  }
  last.set(l.id, { ...req, only: req.only || st.only });
  await run(app, h, l, req, model, { openSettings, modal });
}

async function run(app, h, l, req, model, ctx) {
  const id = l.id;
  // A see-through layer whose outline isn't kept: the model sees what the canvas shows there.
  const source = req.keepAlpha || !call('raster_info', { id }).see_through ? 'layer' : 'canvas';
  if (source === 'canvas') { try { engineRaw().raster_sample(BigInt(id)); } finally { afterOffscreen(); } }
  const inputs = call('raster_ai_inputs', { id, max: 1024, source });
  const strip = review(app, h, `Changing “${l.name}”… (about 10–40 seconds)`, null);
  let b64, said = '';
  try {
    // An image model that edits pictures (OpenRouter style): the Xenify server's picture model.
    const r = call('image_edit_request', { model: '', change: req.change, image: inputs.image, mask: req.only ? inputs.mask : null,
      references: req.refs.map((x) => ({ image: `data:${x.mime};base64,${x.b64}`, note: x.note })) });
    const json = await pictureCall(r);
    b64 = call('image_from_response', { response: json });
    said = call('image_reply_text', { response: json });
  } catch (e) {
    strip.done(`Couldn’t change it: ${e.message || e}`, [['Try again', () => run(app, h, l, req, model, ctx)], ['Close', null]]);
    return;
  }
  if (!app.layerById(id)) { strip.done('The layer was deleted while the AI worked.', [['Close', null]]); return; }
  if (app.run('raster_ai_apply', { id, base64: b64, only_selection: req.only, keep_alpha: req.keepAlpha }) === null) { strip.close(); return; }
  app.viewport.redraw();
  const undoIt = () => { if (app.state.undo === 'AI change') app.run('undo'); };
  const actions = [
    ['Keep', null, true],
    ['Undo', undoIt],
    ['Try again', () => { undoIt(); run(app, h, l, req, model, ctx); }],
    ['Change it more…', () => aiEditDialog(app, h, ctx)],
  ];
  if (said) { strip.done(said, actions); return; }
  // Some image models answer with the picture alone: a model that sees compares before and after.
  strip.done('Changed it. Looking at what changed…', actions);
  const told = await whatChanged(inputs.image, b64, req.change);
  if (strip.open()) strip.done(told || `Changed it as asked: “${req.change}”.`, actions);
}

/// One or two sentences on how the picture changed, from the server's model that sees; '' when there's none.
async function whatChanged(before, after, change) {
  const url = (b) => { const head = atob(b.slice(0, 16)); return `data:${mimeOf([...head].map((c) => c.charCodeAt(0)))};base64,${b}`; };
  const body = {
    model: '', max_tokens: 150,
    messages: [{ role: 'user', content: [
      { type: 'text', text: `The first picture is before, the second after this change was asked for: “${change}”. In one or two short sentences, say what changed (what it looks like now), plainly, for the person who asked. Don't mention “first” or “second picture”.` },
      { type: 'image_url', image_url: { url: url(before) } },
      { type: 'image_url', image_url: { url: url(after) } }] }],
  };
  try {
    const to = chatTarget(body);
    const res = await fetch(to.url, { method: 'POST', headers: { Authorization: `Bearer ${to.key}`, 'Content-Type': 'application/json', 'HTTP-Referer': 'https://github.com/xenify-app', 'X-Title': 'Xenify' }, body: JSON.stringify(to.body) });
    if (!res.ok) return '';
    return call('image_reply_text', { response: await res.json() });
  } catch { return ''; }
}

/// Sends a picture request (`r`: what image_edit_request / image_mask_request made) to the image model:
/// the Xenify server's. The answer as JSON.
function pictureCall(r) {
  return pictureEditOnServer(r.body);
}

/// The raster layer to work on with AI, or null (with a note why).
function aiLayer(app, openSettings) {
  const l = app.raster.layer() || (app.state.selection.length === 1 ? app.layerById(app.state.selection[0]) : null);
  if (!l || l.kind !== 'picture') { app.toast('Select a picture or a raster layer first.'); return null; }
  const info = call('raster_info', { id: l.id });
  if (!info.paintable) { app.toast(info.why); return null; }
  if (!hasAi()) { noAi(app.toast, openSettings); return null; }
  return l;
}

/// Asks the image model to draw a mask of `what` on layer `l` and selects it (`mode`: replace, add, subtract).
async function maskOf(app, h, l, what, mode) {
  const inputs = call('raster_ai_inputs', { id: l.id, max: 1024, source: 'layer' });
  const r = call('image_mask_request', { model: '', what, image: inputs.image });
  const json = await pictureCall(r);
  const b64 = call('image_from_response', { response: json });
  call('raster_select_mask', { id: l.id, base64: b64, mode });
}

/// Selection › Select with AI…: “the jacket”, “the sky”, “the person on the left”.
export async function aiSelectDialog(app, h, { openSettings, modal }) {
  const l = aiLayer(app, openSettings);
  if (!l) return;
  const what = h('input', { class: 'field', placeholder: 'e.g. “the jacket”, “the sky”, “the dog”', 'aria-label': 'What to select', onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter') { e.preventDefault(); document.querySelector('.modal-back .btn.main')?.click(); } } });
  what.value = aiSelectDialog.last || '';
  let mode = 'replace';
  const has = !!call('raster_info', { id: l.id }).selection;
  const ok = await modal('Select with AI', h('div', { style: 'display: flex; flex-direction: column; gap: 10px' },
    h('label', {}, 'What should be selected?', what),
    has ? h('label', { class: 'check-row' }, h('input', { type: 'checkbox', onChange: (e) => { mode = e.target.checked ? 'add' : 'replace'; } }), h('span', {}, 'Add to what’s selected')) : null,
    h('small', { class: 'muted' }, 'An image model draws where it is; check the edges (Grow, Shrink, Soften, or paint the selection with Q). About 10–30 seconds.')),
  [['Cancel', false], ['Select', true, true]]);
  const text = what.value.trim();
  if (!ok || !text) return;
  aiSelectDialog.last = text;
  const strip = review(app, h, `Finding ${text}…`, null);
  try {
    await maskOf(app, h, l, text, mode);
  } catch (e) {
    strip.done(`Couldn’t select it: ${e.message || e}`, [['Close', null]]);
    return;
  }
  app.changed();
  const sel = call('raster_info', { id: l.id }).selection;
  if (!sel) { strip.done(`The model didn’t find ${text}.`, [['Close', null]]); return; }
  strip.done(`Selected ${text}.`, [['OK', null, true], ['Undo', () => app.run('undo')], ['Change it with AI…', () => aiEditDialog(app, h, { openSettings, modal })]]);
}

/// AI › Remove the background: the main subject stays, everything else becomes see-through.
export async function removeBackground(app, h, { openSettings }) {
  const l = aiLayer(app, openSettings);
  if (!l) return;
  const strip = review(app, h, `Finding what’s in front in “${l.name}”…`, null);
  try {
    await maskOf(app, h, l, 'the main subject (the people, animals or things in front), not the background', 'replace');
    if (!call('raster_info', { id: l.id }).selection) throw new Error('the model found nothing in front');
    call('raster_feather', { id: l.id, by: 1 });
    call('raster_invert', { id: l.id });
    app.run('raster_clear', { id: l.id });
    call('raster_select_none', { id: l.id });
  } catch (e) {
    strip.done(`Couldn’t remove the background: ${e.message || e}`, [['Close', null]]);
    return;
  }
  app.changed();
  // Undo: back to before the deletion (the selection steps after it first).
  const undoIt = () => { for (let i = 0; i < 3 && app.state.undo; i++) { const u = app.state.undo; app.run('undo'); if (u === 'Delete pixels') break; } };
  strip.done('Removed the background. Rub out or paint back the edges with the eraser if needed.', [['Keep', null, true], ['Undo', undoIt]]);
}

/// The strip over the canvas while the AI works and after: what it said and what to do next.
function review(app, h, text, actions) {
  document.querySelector('.ai-change')?.remove();
  const el = h('div', { class: 'ai-change', role: 'status', 'aria-live': 'polite' });
  const host = document.getElementById('stage') || document.body;
  host.append(el);
  const fill = (msg, list, busy) => el.replaceChildren(
    h('span', { class: 'ai-change-icon', html: icon('sparkles', 16) }),
    h('span', { class: `ai-change-text${busy ? ' busy' : ''}` }, msg),
    list?.length ? h('div', { class: 'ai-change-actions' },
      list.map(([label, f, main]) => h('button', { class: `small-btn${main ? ' main' : ''}`, onClick: () => { el.remove(); f?.(); } }, label))) : null);
  fill(text, actions, true);
  return { done: (msg, list) => fill(msg, list, false), close: () => el.remove(), open: () => el.isConnected };
}
