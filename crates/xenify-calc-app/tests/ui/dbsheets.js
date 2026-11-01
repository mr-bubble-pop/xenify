// Calc UI test (scripts/test-calc-ui.sh): a database sheet's columns fit
// their values when it first loads, and numeric(10,2) shows two decimals.
window.XENIFY_TEST = async () => {
  const app = window.calc;
  const g = app.grid;
  const out = [];
  const check = (what, got, want) => out.push(`${got === want ? 'ok' : 'FAIL'} ${what}: ${got}${got === want ? '' : ` (want ${want})`}`);
  const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));

  // As the server answers a table editor's read (decimals from numeric(10,2)).
  const def = { server: 'http://office:7700', source: 3, source_name: 'Shop', table: 'shops', key: ['id'], add: true, delete: true };
  const answer = {
    columns: [['id', 'number'], ['name', 'text'], ['city', 'text'], ['price', 'number']],
    rows: [[1, 'Corner Bakery', 'Leeds', 21.5], [2, 'Mill & Loaf', 'York', 24], [3, 'The Long Street Patisserie', 'Bath', 1234.5]],
    truncated: false, notes: { total: 3 }, decimals: [null, null, null, 2],
  };
  const id = app.db.d.run('add_editor', { def, answer });
  app.db.d.goSheet(id);
  await frame();
  await frame();
  const shown = (r, c) => app.db.d.call('cell', { sheet: id, row: r, col: c })?.text;
  check('price 21.5', shown(2, 4), '21.50');
  check('price 24', shown(3, 4), '24.00');
  check('price 1234.5', shown(4, 4), '1,234.50');
  check('ids as they were', shown(2, 1), '1');
  check('name column wider than normal', g.cols.size(2) > 200, true);
  check('id column left as it is', g.cols.size(1), g.cols.def);

  const text = out.join('\n');
  const done = text.includes('FAIL') ? 'FAILED' : 'PASSED';
  document.body.append(Object.assign(document.createElement('pre'), { id: 'test-result', textContent: `${done}\n${text}` }));
  document.title = `TEST ${done}`;
};
