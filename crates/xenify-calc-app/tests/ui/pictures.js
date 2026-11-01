// Calc UI test (scripts/test-calc-ui.sh): a picture keeps the place it's
// dragged to, and new pictures go in at most 240 px wide.
window.XENIFY_TEST = async () => {
  const app = window.calc;
  const g = app.grid;
  const out = [];
  const check = (what, got, want) => out.push(`${got === want ? 'ok' : 'FAIL'} ${what}: ${got}${got === want ? '' : ` (want ${want})`}`);
  const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));

  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="520" height="520" viewBox="0 0 520 520"><rect width="520" height="520" fill="#4a8"/><circle cx="260" cy="260" r="200" fill="#fd5"/></svg>';
  const file = new File([svg], 'disc.svg', { type: 'image/svg+xml' });
  const id = await app.pictures.addFile(file, 2, 2);
  await frame();
  let p = app.pictures.list.find((x) => x.id === id);
  check('new picture width', p.w, 240);
  check('new picture height', p.h, 240);

  // Drag it by its middle two columns right and one row down.
  const n = app.pictures.nodes.get(id);
  n.setPointerCapture = () => {};
  const box = n.getBoundingClientRect();
  const x0 = box.left + box.width / 2; const y0 = box.top + box.height / 2;
  const mx = g.cols.size(2) + g.cols.size(3); const my = g.rows.size(2);
  const ev = (type, x, y) => new PointerEvent(type, { bubbles: true, clientX: x, clientY: y, pointerId: 1, button: 0, buttons: type === 'pointerup' ? 0 : 1 });
  n.dispatchEvent(ev('pointerdown', x0, y0));
  n.dispatchEvent(ev('pointermove', x0 + mx / 2, y0 + my / 2));
  n.dispatchEvent(ev('pointermove', x0 + mx, y0 + my));
  const during = n.getBoundingClientRect();
  n.dispatchEvent(ev('pointerup', x0 + mx, y0 + my));
  await frame();
  p = app.pictures.list.find((x) => x.id === id);
  check('row after the move', p.row, 3);
  check('column after the move', p.col, 4);
  check('dx after the move', p.dx, 4);
  check('dy after the move', p.dy, 4);
  const after = n.getBoundingClientRect();
  check('left kept on the drop', Math.round(after.left - during.left), 0);
  check('top kept on the drop', Math.round(after.top - during.top), 0);

  // Resize by its corner: the place stays, the size follows.
  const grip = n.querySelector('.grip');
  const gb = grip.getBoundingClientRect();
  const gx = gb.left + gb.width / 2; const gy = gb.top + gb.height / 2;
  grip.dispatchEvent(ev('pointerdown', gx, gy));
  n.dispatchEvent(ev('pointermove', gx + 60, gy + 30));
  n.dispatchEvent(ev('pointerup', gx + 60, gy + 30));
  await frame();
  p = app.pictures.list.find((x) => x.id === id);
  check('width after resizing', p.w, 300);
  check('height after resizing', p.h, 270);
  check('row after resizing', p.row, 3);
  check('column after resizing', p.col, 4);
  const resized = n.getBoundingClientRect();
  check('left kept on resizing', Math.round(resized.left - after.left), 0);
  check('top kept on resizing', Math.round(resized.top - after.top), 0);

  // A window resize (place() again) keeps it where it is.
  window.dispatchEvent(new Event('resize'));
  await frame();
  app.pictures.refresh();
  const again = n.getBoundingClientRect();
  check('left kept after redrawing', Math.round(again.left - resized.left), 0);
  check('top kept after redrawing', Math.round(again.top - resized.top), 0);

  // Let go over the column letters and row numbers: it goes into A1, not
  // under the letters.
  const b2 = n.getBoundingClientRect();
  const cx = b2.left + 20; const cy = b2.top + 20;
  n.dispatchEvent(ev('pointerdown', cx, cy));
  n.dispatchEvent(ev('pointermove', cx - 600, cy - 600));
  n.dispatchEvent(ev('pointerup', cx - 600, cy - 600));
  await frame();
  p = app.pictures.list.find((x) => x.id === id);
  check('row after dropping on the letters', p.row, 1);
  check('column after dropping on the letters', p.col, 1);
  check('dx after dropping on the letters', p.dx, 0);
  check('dy after dropping on the letters', p.dy, 0);
  const host = g.host.getBoundingClientRect();
  check('top below the letters', Math.round(n.getBoundingClientRect().top - host.top), g.y(1));

  const text = out.join('\n');
  const done = text.includes('FAIL') ? 'FAILED' : 'PASSED';
  document.body.append(Object.assign(document.createElement('pre'), { id: 'test-result', textContent: `${done}\n${text}` }));
  document.title = `TEST ${done}`;
};
