// Insert › 3D model: turn the model in a preview (drag it, or a ready view),
// pick how big the picture is, and place it as a picture you can paint on
// like any other. The model stays with the picture, so right-click › Turn
// the 3D model again reopens this with it (the picture is drawn again).

import { call, engineRaw, afterOffscreen } from './engine.js';

const VIEWS = [['Front', 0, 0], ['Side', 0, -90], ['Back', 0, 180], ['Top', 90, 0], ['Three-quarter', 20, -30]];
const SIZES = [[512, 'Small (512 pixels)'], [1024, 'Medium (1024 pixels)'], [2048, 'Large (2048 pixels)'], [4096, 'Very large (4096 pixels)']];
const PREVIEW = 480;

const toB64 = (bytes) => { let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return btoa(s); };
// A PNG's size, from its header.
const pngSize = (b) => { const v = new DataView(b.buffer, b.byteOffset); return [v.getUint32(16), v.getUint32(20)]; };
const tidy = (d) => ((((Math.round(d) + 180) % 360) + 360) % 360) - 180;

/// `from`: { name, bytes } for a model file, or { layer } to turn a placed one again.
export async function modelDialog(app, h, { modal, toast }, from) {
  let staged;
  try {
    staged = call('model_stage', from.layer != null ? { id: from.layer } : { name: from.name, base64: toB64(from.bytes) });
  } catch (e) { toast(String(e.message || e)); return; }
  const again = from.layer != null;
  const pose = { rot_x: staged.rot_x, rot_y: staged.rot_y, rot_z: staged.rot_z };
  // Turned again: drawn as big as it is now.
  let size = again ? staged.side || 1024 : (app.modelSize || 1024);
  const img = h('img', { class: 'model-img', alt: 'The model, turned as it will be placed', draggable: 'false' });
  const stage = h('div', { class: 'model-stage', tabindex: '0', role: 'img', 'aria-label': 'The model: drag, or use the arrow keys, to turn it' }, img);
  const angles = h('small', { class: 'muted' });
  // One drawing at a time; the newest pose wins.
  let busy = false, pending = false, url = null;
  const show = () => {
    angles.textContent = `Tilted ${tidy(pose.rot_x)}°, turned ${tidy(pose.rot_y)}°${tidy(pose.rot_z) ? `, spun ${tidy(pose.rot_z)}°` : ''}`;
    if (busy) { pending = true; return; }
    busy = true;
    requestAnimationFrame(() => {
      try {
        const png = engineRaw().model_view(JSON.stringify(pose), PREVIEW);
        if (url) URL.revokeObjectURL(url);
        url = URL.createObjectURL(new Blob([png], { type: 'image/png' }));
        img.src = url;
      } catch (e) { angles.textContent = String(e.message || e); } finally { afterOffscreen(); }
      busy = false;
      if (pending) { pending = false; show(); }
    });
  };
  const turn = (dx, dy, dz = 0) => { pose.rot_y += dx; pose.rot_x += dy; pose.rot_z += dz; show(); };
  let last = null;
  stage.addEventListener('pointerdown', (e) => { last = [e.clientX, e.clientY]; stage.setPointerCapture(e.pointerId); });
  stage.addEventListener('pointermove', (e) => {
    if (!last) return;
    turn((e.clientX - last[0]) * 0.6, -(e.clientY - last[1]) * 0.6);
    last = [e.clientX, e.clientY];
  });
  stage.addEventListener('pointerup', () => { last = null; });
  stage.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 45 : 15;
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
    if (!d) return;
    e.preventDefault(); e.stopPropagation();
    turn(...d);
  });
  const views = h('div', { class: 'model-views' },
    VIEWS.map(([label, x, y]) => h('button', { class: 'small-btn', onClick: () => { Object.assign(pose, { rot_x: x, rot_y: y, rot_z: 0 }); show(); } }, label)),
    h('button', { class: 'small-btn', title: 'Spin it a quarter turn, flat on the screen', onClick: () => turn(0, 0, 90) }, 'Spin 90°'));
  const sizeRow = again ? null : h('label', { class: 'model-size' }, 'Picture size (its longer side)',
    h('select', { class: 'field', onChange: (e) => { size = Number(e.target.value); } }, SIZES.map(([v, l]) => h('option', { value: v, selected: v === size }, l))));
  const body = h('div', { class: 'model-dialog' }, stage, angles, views, sizeRow,
    h('p', { class: 'muted' }, again
      ? 'The picture is drawn again, turned like this. Anything painted on it is replaced.'
      : `${staged.summary}. Drag the model to turn it. It’s placed as a picture you can move, paint on and animate.`));
  show();
  setTimeout(() => stage.focus(), 0);
  const ok = await modal(again ? 'Turn the 3D model again' : 'Insert a 3D model', body, [['Cancel', false], [again ? 'Update the picture' : 'Place', true, true]]);
  if (url) URL.revokeObjectURL(url);
  if (!ok) return;
  if (!again) app.modelSize = size;
  let png;
  try {
    png = engineRaw().model_view(JSON.stringify(pose), size);
  } catch (e) { toast(`Couldn’t draw the model: ${e.message || e}`); return; } finally { afterOffscreen(); }
  const [width, height] = pngSize(png);
  if (app.run('model_place', { base64: toB64(png), width, height, pose }) == null) return;
  app.setTool('select');
  toast(again ? 'Turned the model and drew the picture again.' : 'Placed the model as a picture. To turn it later: right-click it › Turn the 3D model again.');
}
