// Calc UI test (scripts/test-calc-ui.sh): Ask's propose sends a number that
// gives many values (an array) or an error back to the model instead of
// showing it, and a card's numbers keep their money columns' 0.00.
window.XENIFY_TEST = async () => {
  const app = window.calc;
  const out = [];
  const check = (what, got, want) => out.push(`${got === want ? 'ok' : 'FAIL'} ${what}: ${got}${got === want ? '' : ` (want ${want})`}`);
  const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  const { call, run } = app.ask.d;

  // Hours, Revenue and Costs (money, 0.00) as a table.
  const sheet = app.ask.d.sheet();
  const rows = [['Hour', 'Revenue', 'Costs'], [9, 120.2, 80.1], [10, 60, 49.2], [11, 64, 50]];
  const cells = [];
  rows.forEach((r, i) => r.forEach((v, c) => cells.push({ row: i + 1, col: c + 1, data: typeof v === 'number' ? { i: String(v) } : { s: v } })));
  run('edit', { ops: [{ op: 'cells', sheet, cells }] });
  run('make_table', { sheet, row: 1, col: 1 });
  run('format', { sheet, range: { top: 2, left: 2, bottom: 4, right: 3 }, set: { num_fmt: '0.00' } });
  const table = call('tables')[0].name;

  // No AI here: Ask opens as if it had one.
  app.ask.refresh = async () => true;
  app.ask.whose = () => 'Test';
  await app.ask.open();
  const holder = app.ask.log;

  // calculate says an array is an array.
  const calc = await app.ask.run('calculate', { formula: `=SORTBY(${table}[Hour],${table}[Revenue],-1)` }, holder);
  check('calculate: array of 3', calc.array?.values, 3);
  check('calculate: note', /array of 3 values/.test(calc.note || ''), true);

  // An array or an error goes back to the model, and no card shows.
  const busy = await app.ask.run('propose', { title: 'Busiest hour', numbers: [{ label: 'Busiest hour', formula: `=SORTBY(${table}[Hour],${table}[Revenue],-1)` }] }, holder);
  check('array: back as an error', /gives 3 values/.test(busy.error || ''), true);
  check('array: INDEX suggested', /INDEX\(…,1\)/.test(busy.error || ''), true);
  const div = await app.ask.run('propose', { title: 'Margin', numbers: [{ label: 'Margin', formula: `=SUM(${table}[Revenue])/0` }] }, holder);
  check('error: back as an error', /#DIV\/0!: something is divided by zero/.test(div.error || ''), true);
  check('no card shown', holder.querySelectorAll('.ask-card').length, 0);

  // Fixed: a card, with the money columns' two decimals.
  const ok = await app.ask.run('propose', { title: 'Takings', numbers: [
    { label: 'Busiest hour', formula: `=INDEX(SORTBY(${table}[Hour],${table}[Revenue],-1),1)` },
    { label: 'Revenue', formula: `=SUM(${table}[Revenue])` },
    { label: 'Profit', formula: `=SUM(${table}[Revenue])-SUM(${table}[Costs])` },
    { label: 'Best hour’s takings', formula: `=MAX(${table}[Revenue])` },
  ] }, holder);
  check('shown', ok.shown, true);
  await frame();
  const nums = [...holder.querySelectorAll('.ask-card .ask-num b')].map((b) => b.textContent);
  check('busiest hour', nums[0], '9');
  check('revenue', nums[1], '244.20');
  check('profit', nums[2], '64.90');
  check('max revenue', nums[3], '120.20');

  const text = out.join('\n');
  const done = text.includes('FAIL') ? 'FAILED' : 'PASSED';
  document.body.append(Object.assign(document.createElement('pre'), { id: 'test-result', textContent: `${done}\n${text}` }));
  document.title = `TEST ${done}`;
};
