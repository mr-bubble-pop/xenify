// The AI menu: build a new animation or slide deck from a description (in a
// new tab), a new slide or a change to this slide, and an illustration
// painted by an image model (traced into shapes, or kept as a picture).

import { call } from './engine.js';
import { hasAi, serverPicture, aiServer, noAi } from './aiprovider.js';
import { ui } from './serverui.js';

function needsKey(app, openSettings) {
  if (hasAi()) return false;
  noAi(app.toast, openSettings);
  return true;
}

function describeBox(h, placeholder, rows = 4) {
  return h('textarea', { class: 'field', rows: String(rows), placeholder, 'aria-label': 'Description', style: 'resize: vertical; font: inherit; width: 100%',
    onKeydown: (e) => { e.stopPropagation(); if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); document.querySelector('.modal-back .btn.main')?.click(); } } });
}

/// AI › New from a description…: a new tab the assistant builds an animation or a deck in.
export async function aiNewDialog(app, h, { openSettings, newTab }) {
  if (needsKey(app, openSettings)) return;
  let kind = 'animation';
  const hint = { animation: 'e.g. “A 6-second logo reveal for a coffee shop called Ember, warm colours”', deck: 'e.g. “A 6-slide pitch for a community garden: problem, plan, budget, timeline, ask”' };
  const text = describeBox(h, hint.animation);
  const segs = h('div', { class: 'segs', role: 'radiogroup', 'aria-label': 'What to make', style: 'display: flex' });
  const draw = () => segs.replaceChildren(...[['animation', 'A picture or animation'], ['deck', 'A slide deck']].map(([k, l]) => h('button', { class: `seg${kind === k ? ' on' : ''}`, role: 'radio', 'aria-checked': String(kind === k), style: 'flex: 1 1 0',
    onClick: () => { kind = k; text.placeholder = hint[k]; draw(); } }, l)));
  draw();
  const ok = await app.ui.modal('New canvas from a description', h('div', { class: 'ai-dialog', style: 'display: flex; flex-direction: column; gap: 10px' }, segs, text,
    h('small', { class: 'muted' }, 'The assistant makes it as a new canvas, in a new tab. (To add one slide to this presentation: AI › Add a slide to this presentation.) Ctrl+Enter starts.')), [['Cancel', false], ['Build it', true, true]]);
  const description = text.value.trim();
  if (!ok || !description) return;
  newTab(kind === 'deck' ? 'New deck' : 'New animation', () => call('new', kind === 'deck' ? { width: 1920, height: 1080, duration: 6, name: 'Slide 1', deck: true } : { width: 1920, height: 1080, duration: 6, name: 'Animation' }));
  app.side = 'ai';
  app.renderSide();
  app.assistant.send(description, kind === 'deck' ? 'new_deck' : 'new_animation');
}

/// AI › New slide with AI… / Modify this slide with AI…
export async function aiSlideDialog(app, h, task, { openSettings }) {
  if (needsKey(app, openSettings)) return;
  const text = describeBox(h, task === 'new_slide' ? 'What should the slide say or show? e.g. “Three benefits of solar power, with an icon for each”' : 'What should change? e.g. “Make the title bigger and have the bullets fly in one by one”');
  const ok = await app.ui.modal(task === 'new_slide' ? 'Add a slide to this presentation' : 'Modify this slide with AI', h('div', { class: 'ai-dialog', style: 'display: flex; flex-direction: column; gap: 8px' }, text,
    h('small', { class: 'muted' }, 'The assistant builds it as one change you can keep or undo. Ctrl+Enter sends.')), [['Cancel', false], [task === 'new_slide' ? 'Create slide' : 'Apply', true, true]]);
  if (!ok || !text.value.trim()) return;
  app.side = 'ai';
  app.renderSide();
  app.assistant.send(text.value.trim(), task);
}

/// AI › New illustration…: an image model paints it; traced into shapes, or kept as a picture.
export async function illustrationDialog(app, h, { openSettings, addPicture }) {
  if (needsKey(app, openSettings)) return;
  const st = (app.illustrate ||= { vector: true, colors: 5 });
  const model = aiServer.info.image_model || 'the server’s';
  const prompt = describeBox(h, 'e.g. a cute sitting fox, flat style', 4);
  const note = h('small', { class: 'muted' });
  const colours = h('div', { class: 'prop', title: 'Lower: fewer colours and simpler shapes; higher: more detail' }, h('span', { class: 'pname' }, `Detail (${st.colors})`),
    h('input', { type: 'range', min: '2', max: '8', step: '1', value: String(st.colors), style: 'flex: 1; accent-color: var(--accent)', onInput: (e) => { st.colors = Number(e.target.value); e.target.previousSibling.textContent = `Detail (${st.colors})`; } }));
  const segs = h('div', { class: 'segs', role: 'radiogroup', 'aria-label': 'Result', style: 'display: flex' });
  const draw = () => {
    segs.replaceChildren(...[[true, 'Vector shapes', 'Traced into editable, scalable shapes'], [false, 'Picture', 'Kept as a picture (photos, detailed art)']].map(([v, l, t]) => h('button', { class: `seg${st.vector === v ? ' on' : ''}`, role: 'radio', 'aria-checked': String(st.vector === v), title: t, style: 'flex: 1 1 0',
      onClick: () => { st.vector = v; draw(); } }, l)));
    colours.hidden = !st.vector;
    note.textContent = st.vector ? `An image model (${model}) paints it, then Draw traces it into shapes. About 10–30 seconds.` : `An image model (${model}) paints it; it’s added as a picture. About 10–30 seconds.`;
  };
  draw();
  const ok = await app.ui.modal('New illustration', h('div', { class: 'ai-dialog', style: 'display: flex; flex-direction: column; gap: 10px' }, segs, prompt, colours, note), [['Cancel', false], ['Paint it', true, true]]);
  const subject = prompt.value.trim();
  if (!ok || !subject) return;
  app.toast('Painting… (about 10–30 seconds)');
  let b64;
  try {
    // The server's image model (an images endpoint, not chat).
    b64 = await serverPicture(st.vector ? `${subject}. Flat illustration, a few solid colours (no gradients or textures), clean shapes, on a plain background of one solid colour that the subject doesn’t use (not white if any part of the subject is white).` : subject);
  } catch (e) { app.toast(`Couldn’t paint it: ${e.message || e}`); return; }
  const name = subject.split(/\s+/).slice(0, 4).join(' ').replace(/[^\p{L}\p{N}]+$/u, '') || 'Illustration';
  if (st.vector) {
    if (app.run('trace', { base64: b64, name, colors: st.colors, noise: 8, smooth: true, remove_background: true }) != null) { app.setTool('select'); app.toast(`Painted and traced “${name}”.`); }
  } else {
    await addPicture(`${name}.png`, Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)));
  }
}

/// AI › New video clip…: a short clip made by the video model on the Xenify
/// server (LocalAI), added at the playhead.
export async function videoDialog(app, h, { openSettings, importVideo }) {
  const offered = aiServer.info?.offered?.video || [];
  if (!aiServer.info) return void noAi(app.toast, openSettings);
  if (!offered.length) return void app.toast('Video clips need a video model on your Xenify server: its admin can add one on the admin pages (AI).');
  const prompt = describeBox(h, 'e.g. paper boats drifting on a rainy street, slow camera pan', 3);
  const ok = await app.ui.modal('New video clip', h('div', { class: 'ai-dialog', style: 'display: flex; flex-direction: column; gap: 10px' }, prompt,
    h('small', { class: 'muted' }, 'Made on your Xenify server. It can take a few minutes; you can keep working.')), [['Cancel', false], ['Make it', true, true]]);
  const subject = prompt.value.trim();
  if (!ok || !subject) return;
  app.toast('Making the clip… (this can take a few minutes)');
  try {
    const out = await ui.server.api('POST', '/v1/ai/video', { prompt: subject, model: aiServer.info.video_model || '' });
    const b64 = out?.data?.[0]?.b64_json;
    if (!b64) throw new Error('the server sent no clip');
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    // Named by what it is (WebM starts 1A 45 DF A3; else MP4).
    const webm = bytes[0] === 0x1a && bytes[1] === 0x45 && bytes[2] === 0xdf && bytes[3] === 0xa3;
    const name = `${subject.split(/\s+/).slice(0, 4).join(' ').replace(/[^\p{L}\p{N}]+$/u, '') || 'Clip'}.${webm ? 'webm' : 'mp4'}`;
    await importVideo(name, bytes);
  } catch (e) { app.toast(`Couldn’t make it: ${e.message || e}`); }
}
