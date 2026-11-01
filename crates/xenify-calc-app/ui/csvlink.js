// Linked CSV sheets: a sheet whose values come from a CSV (or other
// separated text) file and stay up to date with it. On the desktop the link
// is to a file on the computer: Calc reads it when the book opens and again
// whenever the file changes while it's open. In the browser on a Xenify
// server it's to a file in your Files there: read when the book opens, with
// Update now, and when a new version is saved there. The sheet can't be typed
// in; a bar above it (on screen only) says which file, when it was read,
// and has Update now and Unlink (keep as a normal sheet). Other sheets'
// formulas follow each update.

import { h, icon, toast, modal } from './ui.js';
import * as platform from './platform.js';
import { pickFile } from './serverui.js';

export const SEPARATORS = [[',', 'Comma ,'], ['\t', 'Tab'], [';', 'Semicolon ;'], ['|', 'Pipe |'], [' ', 'Space']];
const sepName = (c) => SEPARATORS.find((x) => x[0] === c)?.[1] || `“${c}”`;
const TEXT_FILE = /\.(csv|tsv|txt|psv|tab)$/i;

const ago = (iso) => {
  if (!iso) return 'not read yet';
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return 'read just now';
  if (s < 3600) return `read ${Math.round(s / 60)} min ago`;
  if (s < 86400) return `read ${Math.round(s / 3600)} h ago`;
  return `read ${new Date(iso).toLocaleString()}`;
};
const plural = (n, one, many) => `${(n || 0).toLocaleString()} ${n === 1 ? one : many}`;

/// How to read separated text: its first rows as they'll read, with the
/// separator (found, or picked) and whether values become numbers and dates
/// or stay text. `linkable`: also offers to keep the sheet linked to the
/// file (`linked`: always linked, as with Link a CSV file). Resolves to
/// { sep, values, link } or null.
export async function csvDialog(call, name, text, { title, okLabel = 'Open', linkable = false, linked = false } = {}) {
  const found = call('csv_preview', { text }).sep;
  const sep = h('select', { class: 'field', 'aria-label': 'Separated by' },
    h('option', { value: '' }, `${sepName(found)} (found in the file)`),
    SEPARATORS.filter(([c]) => c !== found).map(([c, l]) => h('option', { value: c }, l)),
    h('option', { value: 'other' }, 'Another character…'));
  const other = h('input', { class: 'field', maxlength: '1', placeholder: 'The character', style: { width: '130px' }, hidden: true, 'aria-label': 'Separator character' });
  const radio = (v, label, hint, on) => h('label', { class: 'choice' }, h('input', { type: 'radio', name: 'csv-values', value: v, checked: on }), h('span', {}, h('b', {}, label), h('small', {}, hint)));
  const values = h('div', { class: 'choices' },
    radio('typed', 'Numbers and dates', 'Read like typing. Long numbers keep every digit (up to 28); codes like 007 or +44 stay exactly as written.', true),
    radio('text', 'Everything as text', 'Every value exactly as in the file.', false));
  const preview = h('div', { class: 'csv-preview' });
  const chosen = () => (sep.value === 'other' ? other.value || undefined : sep.value || undefined);
  const how = () => values.querySelector('input:checked').value;
  const draw = () => {
    other.hidden = sep.value !== 'other';
    let p;
    try { p = call('csv_preview', { text, sep: chosen(), values: how() }); } catch (e) { preview.replaceChildren(h('p', {}, String(e.message || e))); return; }
    const width = Math.max(1, ...p.rows.map((r) => r.length));
    preview.replaceChildren(
      h('table', {}, h('tbody', {}, p.rows.map((r, i) => h('tr', {}, Array.from({ length: width }, (_, k) => {
        const kept = p.text[i]?.[k] && how() === 'typed' && r[k] !== '' && /\d/.test(r[k]);
        return h('td', { class: kept ? 'kept' : null, title: kept ? 'Kept exactly as written' : null }, r[k] ?? '');
      }))))),
      h('p', { class: 'hint' }, `${p.lines} lines · ${width} columns${p.lines > p.rows.length ? ` · the first ${p.rows.length} shown` : ''}`));
  };
  sep.addEventListener('change', draw);
  other.addEventListener('input', draw);
  values.addEventListener('change', draw);
  draw();
  const keep = linkable && !linked ? h('input', { type: 'checkbox' }) : null;
  const body = h('div', { class: 'csv-import' },
    h('div', { class: 'row-line' }, h('label', { class: 'field-group' }, 'Separated by', sep), other),
    values, preview,
    keep ? h('label', { class: 'check-row' }, keep, h('span', {}, 'Keep this sheet linked to the file, so it updates when the file changes')) : null,
    linked ? h('p', { class: 'hint' }, 'The sheet stays linked to the file: it updates when the file changes, and can’t be typed in (Unlink makes it a normal sheet).') : null);
  const ok = await modal(title || `Open ${platform.fileName(name)}`, body, [['Cancel', false], [okLabel, true, true]]);
  if (!ok) return null;
  return { sep: chosen() || '', values: how(), link: linked || !!keep?.checked };
}

export class CsvLinks {
  constructor(d) {
    this.d = d;   // { call, run, server, sheet, goSheet, grid, tab, changed }
    this.el = document.getElementById('linkbar');
    // By book and sheet: at(sheet).
    this.status = new Map();   // { busy, missing, error }
    this.stamps = new Map();   // what the file was when last read (its change time and size, or its version on the server)
    this.watching = false;
    // The desktop looks at linked files every few seconds; a server's files are asked about every minute.
    setInterval(() => this.watch(), platform.DESKTOP ? 3000 : 60_000);
    // "read 2 min ago" stays true.
    setInterval(() => { if (!this.el.hidden) this.render(); }, 30_000);
  }

  get server() { return this.d.server; }
  at(sheet, tab = this.d.tab()) { return `${tab}:${sheet}`; }

  /// Whether a sheet can be linked here: on the desktop (files on the
  /// computer), or signed in to a Xenify server (files there).
  get possible() { return platform.DESKTOP || !!this.server.session; }

  /// Whether `def`'s file can be read here (and why not).
  why(def) {
    if (def.path) return platform.DESKTOP ? '' : 'It’s linked to a file on a computer: open the book in the Calc app to update it.';
    if (def.node != null) {
      if (!this.server.session || this.server.url !== def.server) return `Sign in to ${String(def.server).replace(/^https?:\/\//, '')} to update it.`;
      return '';
    }
    return 'It doesn’t say which file.';
  }

  /// The file's text and stamp now (throws when it can't be read).
  async read(def) {
    if (def.path) {
      const stamp = await platform.fileStamp(def.path);
      if (stamp == null) { const e = new Error('missing'); e.missing = true; throw e; }
      const { bytes } = await platform.readPath(def.path);
      return { text: new TextDecoder().decode(bytes), stamp };
    }
    let node;
    try { node = await this.server.api('GET', `/v1/nodes/${def.node}`); } catch (e) {
      if (e.status === 404 || e.status === 403) e.missing = true;
      throw e;
    }
    if (!(node.version > 0)) return { text: '', stamp: '0' };
    const bytes = await this.server.api('GET', `/v1/nodes/${def.node}/content`, undefined, { raw: true });
    return { text: new TextDecoder().decode(bytes), stamp: String(node.version) };
  }

  /// The stamp now, without reading the file (null: missing).
  async stamp(def) {
    if (def.path) return platform.fileStamp(def.path);
    try { return String((await this.server.api('GET', `/v1/nodes/${def.node}`)).version || 0); } catch (e) {
      if (e.status === 404 || e.status === 403) return null;
      throw e;
    }
  }

  // ----- Data › Link a CSV file… -----

  async add() {
    if (platform.DESKTOP) {
      const path = await platform.chooseCsv();
      if (!path) return;
      return this.linkTo({ path, name: platform.fileName(path) });
    }
    if (!this.server.session) { toast('Sign in to a Xenify server first: the file comes from your Files there.'); return; }
    // Uploaded files (not Calc books, Write documents…) and folders.
    const n = await pickFile('Link a CSV file', 'csv');
    if (!n) return;
    const name = n.details?.name || 'Linked';
    if (!TEXT_FILE.test(name)) { toast(`${name} isn’t a CSV or text file: pick one ending in .csv, .tsv or .txt.`); return; }
    return this.linkTo({ server: this.server.url, node: n.id, name });
  }

  async linkTo(def) {
    let got;
    try { got = await this.read(def); } catch (e) { toast(e.missing ? `Couldn’t find ${def.name}.` : `Couldn’t read ${def.name}: ${e.message || e}`); return; }
    const how = await csvDialog(this.d.call, def.name, got.text, { title: `Link ${def.name}`, okLabel: 'Link', linked: true });
    if (!how) return;
    const id = this.d.run('add_csv_link', { def: { ...def, sep: how.sep, values: how.values }, text: got.text, read: new Date().toISOString() });
    if (id == null) return;
    this.stamps.set(this.at(id), got.stamp);
    this.d.goSheet(id);
    toast(`Linked to ${def.name}: the sheet updates when the file changes.`);
  }

  /// A sheet just linked by opening its file (its stamp, so a change is seen).
  async noteRead(sheet) {
    const def = this.d.call('csv_link', { sheet });
    if (!def) return;
    try { this.stamps.set(this.at(sheet), await this.stamp(def)); } catch { /* seen next time */ }
  }

  // ----- Reading again -----

  /// When a book opens: every linked sheet reads its file (quietly; the
  /// bar says why when one couldn't).
  async opened() {
    let all = [];
    try { all = this.d.call('csv_links'); } catch {}
    if (!all.length) return;
    let failed = 0;
    for (const l of all) if (!(await this.update(l.sheet, true))) failed++;
    if (failed) toast(failed === 1 ? `${all.length === 1 ? 'The linked file' : 'A linked file'} couldn’t be read: the sheet keeps its last values and says why.` : `${failed} linked files couldn’t be read: their sheets keep their last values.`);
  }

  /// Reads linked sheet `sheet`'s file again (of the front book). True when it could.
  async update(sheet, quiet = false) {
    const def = this.d.call('csv_link', { sheet });
    if (!def) return false;
    const tab = this.d.tab();
    const k = this.at(sheet, tab);
    const why = this.why(def);
    if (why) { this.status.set(k, { error: why }); this.after(sheet); return false; }
    this.status.set(k, { ...this.status.get(k), busy: true });
    this.after(sheet);
    let st = {};
    try {
      const got = await this.read(def);
      if (this.d.tab() !== tab) { this.status.delete(k); return false; }
      const changed = this.d.call('fill_csv_link', { sheet, text: got.text, read: new Date().toISOString() });
      this.stamps.set(k, got.stamp);
      if (changed) this.d.changed();
      else if (!quiet) toast(`${def.name} is the same as before.`);
    } catch (e) {
      st = e.missing ? { missing: true } : { error: `Couldn’t read ${def.name}: ${e.message || e}` };
      // Seen again when it's back (or changes).
      this.stamps.delete(k);
    }
    this.status.set(k, st);
    this.after(sheet);
    return !st.missing && !st.error;
  }

  /// Every few seconds: a linked file of the front book that changed is read again.
  async watch() {
    if (this.watching || document.hidden) return;
    this.watching = true;
    try {
      let all = [];
      try { all = this.d.call('csv_links'); } catch {}
      const tab = this.d.tab();
      for (const l of all) {
        if (this.why(l.def)) continue;
        const k = this.at(l.sheet, tab);
        if (this.status.get(k)?.busy) continue;
        let now;
        try { now = await this.stamp(l.def); } catch { continue; }
        if (this.d.tab() !== tab) break;
        if (now == null) {
          if (!this.status.get(k)?.missing) { this.status.set(k, { missing: true }); this.after(l.sheet); }
          continue;
        }
        if (now !== this.stamps.get(k)) await this.update(l.sheet, true);
      }
    } finally {
      this.watching = false;
    }
  }

  unlink(sheet) {
    const def = this.d.call('csv_link', { sheet });
    if (this.d.run('unlink_csv', { sheet }) == null) return;
    this.status.delete(this.at(sheet));
    this.stamps.delete(this.at(sheet));
    toast(`A normal sheet now, with the values it had: it no longer follows ${def?.name || 'the file'}.`);
  }

  // ----- The bar above a linked sheet -----

  after(sheet) {
    if (this.d.sheet() === sheet) this.sync();
  }

  /// Shows the bar when the sheet shown is linked to a file (hides it otherwise).
  sync() {
    let def = null;
    try { def = this.d.call('csv_link', { sheet: this.d.sheet() }); } catch {}
    const was = this.el.hidden;
    this.def = def;
    this.el.hidden = !def;
    if (def) this.render();
    if (was !== this.el.hidden) this.d.grid.resize();
  }

  render() {
    const def = this.def;
    if (!def) return;
    const sheet = this.d.sheet();
    const st = this.status.get(this.at(sheet)) || {};
    const where = def.path || `${def.name} in your files on ${String(def.server).replace(/^https?:\/\//, '')}`;
    const src = h('span', { class: 'q-src', title: where }, h('span', { html: icon('file', 15) }), `${def.name} · linked · read-only`);
    const facts = h('span', { class: 'q-facts', role: 'status' },
      st.busy ? 'Reading…' : [st.missing || st.error ? `last ${ago(def.read)}` : ago(def.read), plural(def.rows, 'row', 'rows'), plural(def.columns, 'column', 'columns')].join(' · '));
    const problem = st.missing
      ? h('span', { class: 'q-error', role: 'alert', title: where }, `Can’t find ${def.name} any more: the sheet keeps its last values.`)
      : st.error ? h('span', { class: 'q-error', role: 'alert' }, st.error) : null;
    const can = !this.why(def);
    const update = can ? h('button', { class: 'btn small', disabled: st.busy || null, onClick: () => this.update(sheet) }, h('span', { html: icon('refresh', 15) }), 'Update now') : null;
    const unlink = h('button', { class: 'btn small', title: 'Stop following the file: type in the sheet like any other', onClick: () => this.unlink(sheet) }, 'Unlink (keep as a normal sheet)');
    // The problem, when there is one, says more than how it updates.
    const hint = problem ? null : h('span', { class: 'q-hint' }, platform.DESKTOP && def.path ? 'Updates by itself when the file changes.' : can ? 'Updates when the book opens, and when a new version of the file is saved.' : '');
    this.el.replaceChildren(h('div', { class: 'q-bar' }, src, facts, problem, h('span', { class: 'grow' }), hint, update, unlink));
  }
}
