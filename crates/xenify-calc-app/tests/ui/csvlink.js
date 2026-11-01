// Calc UI test (scripts/test-calc-ui.sh): a sheet linked to a CSV in your
// server Files. Linking asks how to read it, the bar above the sheet says
// which file and when it was read, a new version of the file updates the
// sheet (and a formula elsewhere), and a missing file keeps the last values.
// The server is played here (its answers to the few calls linking makes).
window.XENIFY_TEST = async () => {
  const app = window.calc;
  const out = [];
  const check = (what, got, want) => out.push(`${got === want ? 'ok' : 'FAIL'} ${what}: ${got}${got === want ? '' : ` (want ${want})`}`);
  const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  const L = app.csvLinks;
  const mode = new URLSearchParams(location.search).get('mode') || 'linked';

  // The file on the server: its versions, and whether it's there.
  const file = { version: 1, text: 'Region,Month,Sales\nNorth,Jan,1200\nSouth,Jan,950\nEast,Jan,1430\nWest,Jan,610\n', gone: false };
  const srv = app.server;
  srv.session = { token: 'test' };
  srv.url = 'http://office:7700';
  srv.api = async (method, path, body, opts) => {
    if (file.gone) { const e = new Error('There’s no such file.'); e.status = 404; throw e; }
    if (path === '/v1/nodes/42') return { id: 42, version: file.version };
    if (path === '/v1/nodes/42/content') return new TextEncoder().encode(file.text);
    throw new Error(`not played: ${method} ${path}`);
  };
  check('possible on a server', L.possible, true);

  const first = app.tabs[0].sheet;
  app.grid.setSheet(first);

  const linking = L.linkTo({ server: srv.url, node: 42, name: 'sales.csv' });
  await frame();
  await frame();
  const dialog = document.querySelector('.modal');
  check('asks how to read it', dialog?.querySelector('h2')?.textContent, 'Link sales.csv');
  if (mode === 'dialog') { finish(); return; }
  [...dialog.querySelectorAll('.btn')].find((b) => b.textContent === 'Link').click();
  await linking;
  await frame();
  const id = L.d.sheet();
  const shown = (s, r, c) => L.d.call('cell', { sheet: s, row: r, col: c })?.text;
  // A total on the first sheet that follows the linked one.
  L.d.call('type', { sheet: first, row: 1, col: 1, input: 'Total sales' });
  L.d.call('type', { sheet: first, row: 1, col: 2, input: '=SUM(sales!C:C)' });
  check('linked sheet shown', L.d.call('sheets').find((s) => s.id === id)?.kind, 'linked_csv');
  check('its values', shown(id, 3, 3), '950');
  check('bar shown', document.getElementById('linkbar').hidden, false);
  check('bar names the file', document.querySelector('#linkbar .q-src')?.textContent, 'sales.csv · linked · read-only');
  check('total follows', shown(first, 1, 2), '4190');
  check('locked', app.grid.locked(), true);

  // A new version of the file on the server.
  file.version = 2;
  file.text += 'Central,Jan,800\n';
  await L.watch();
  await frame();
  check('new row read', shown(id, 6, 1), 'Central');
  check('total follows the update', shown(first, 1, 2), '4990');

  if (mode === 'missing') {
    file.gone = true;
    await L.update(id, true);
    await frame();
    check('says the file is missing', !!document.querySelector('#linkbar .q-error'), true);
    check('keeps the last values', shown(id, 6, 1), 'Central');
  }
  if (mode === 'unlink') {
    [...document.querySelectorAll('#linkbar .btn')].find((b) => b.textContent.startsWith('Unlink')).click();
    await frame();
    check('a normal sheet', L.d.call('sheets').find((s) => s.id === id)?.kind, 'normal');
    check('bar gone', document.getElementById('linkbar').hidden, true);
    check('values kept', shown(id, 6, 1), 'Central');
    await new Promise((r) => setTimeout(r, 300));
    const cv = app.grid.canvas;
    const px = cv.getContext('2d').getImageData(0, 0, cv.width, cv.height).data;
    let ink = 0;
    for (let i = 0; i < px.length; i += 4) if (px[i + 3] && px[i] < 200) ink++;
    check('grid drawn', ink > 1000, true);
  }
  finish();

  function finish() {
    const text = out.join('\n');
    const done = text.includes('FAIL') ? 'FAILED' : 'PASSED';
    document.body.append(Object.assign(document.createElement('pre'), { id: 'test-result', textContent: `${done}\n${text}` }));
    document.title = `TEST ${done}`;
  }
};
