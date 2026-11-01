// A visual equation editor: fractions, roots, powers and the like are boxes
// you click into and type in, as they'll look on the page. The equation is
// kept as a small tree and written out as LaTeX; LaTeX it can show (what it
// writes, and the common commands) can be opened again.
//
// A row is a list of parts: { t: 'text', v } or a structure with rows inside
// (frac a/b, sqrt a, root n/a, sup a, sub a, sum lo/hi, int lo/hi, paren a, abs a).

const STRUCTS = {
  frac: ['a', 'b'], sqrt: ['a'], root: ['n', 'a'], sup: ['a'], sub: ['a'],
  sum: ['lo', 'hi'], int: ['lo', 'hi'], paren: ['a'], abs: ['a'],
};
// What typed text becomes in LaTeX.
const TEXT_TEX = [['<=', '\\le '], ['>=', '\\ge '], ['!=', '\\ne '], ['+-', '\\pm '], ['*', '\\times '], ['±', '\\pm '], ['×', '\\times '],
  ['≤', '\\le '], ['≥', '\\ge '], ['≠', '\\ne '], ['≈', '\\approx '], ['∞', '\\infty '], ['→', '\\to '], ['·', '\\cdot '], ['°', '^{\\circ}'], ['÷', '\\div ']];
const GREEK = { α: 'alpha', β: 'beta', γ: 'gamma', δ: 'delta', ε: 'epsilon', θ: 'theta', λ: 'lambda', μ: 'mu', π: 'pi', ρ: 'rho', σ: 'sigma', τ: 'tau', φ: 'phi', ω: 'omega', Δ: 'Delta', Σ: 'Sigma', Ω: 'Omega' };
const COMMAND_TEXT = { ...Object.fromEntries(Object.entries(GREEK).map(([g, n]) => [n, g])), le: '≤', ge: '≥', ne: '≠', pm: '±', times: '×', approx: '≈', infty: '∞', to: '→', cdot: '·', div: '÷', circ: '°' };

const text = (v = '') => ({ t: 'text', v });
const struct = (t) => ({ t, ...Object.fromEntries(STRUCTS[t].map((k) => [k, [text()]])) });

// ----- LaTeX out -----

function textTex(v) {
  let out = '';
  for (let i = 0; i < v.length;) {
    const hit = TEXT_TEX.find(([a]) => v.startsWith(a, i));
    if (hit) { out += hit[1]; i += hit[0].length; continue; }
    const g = GREEK[v[i]];
    if (g) { out += `\\${g} `; i += 1; continue; }
    out += '{}#$%&'.includes(v[i]) ? `\\${v[i]}` : v[i];
    i += 1;
  }
  return out;
}
export function toLatex(row) {
  return row.map((p) => {
    const r = (k) => toLatex(p[k]);
    switch (p.t) {
      case 'text': return textTex(p.v);
      case 'frac': return `\\frac{${r('a')}}{${r('b')}}`;
      case 'sqrt': return `\\sqrt{${r('a')}}`;
      case 'root': return `\\sqrt[${r('n')}]{${r('a')}}`;
      case 'sup': return `^{${r('a')}}`;
      case 'sub': return `_{${r('a')}}`;
      case 'sum': case 'int': return `\\${p.t}${toLatex(p.lo).trim() ? `_{${r('lo')}}` : ''}${toLatex(p.hi).trim() ? `^{${r('hi')}}` : ''} `;
      case 'paren': return `\\left( ${r('a')} \\right)`;
      case 'abs': return `\\left| ${r('a')} \\right|`;
      default: return '';
    }
  }).join('').trim();
}

// ----- LaTeX in (null when it uses something this editor can't show) -----

export function fromLatex(src) {
  let i = 0;
  const fail = () => { throw new Error('unsupported'); };
  const ws = () => { while (src[i] === ' ') i += 1; };
  const group = () => {
    ws();
    if (src[i] !== '{') {
      // A single character or command as the argument (x^2).
      if (src[i] === '\\') return row(null, true);
      const c = src[i++];
      if (c === undefined) fail();
      return [text(c)];
    }
    i += 1;
    return row('}');
  };
  const row = (end, one = false) => {
    const out = [];
    const push = (s) => { const last = out[out.length - 1]; if (last?.t === 'text') last.v += s; else out.push(text(s)); };
    while (i < src.length) {
      const c = src[i];
      if (end && src.startsWith(end, i)) { i += end.length; return out; }
      if (c === '}' && !end) fail();
      if (c === '\\') {
        const m = /^\\([a-zA-Z]+|.)/.exec(src.slice(i));
        i += m[0].length;
        const name = m[1];
        if (name === 'frac') { const a = group(); const b = group(); out.push({ t: 'frac', a, b }); }
        else if (name === 'sqrt') {
          ws();
          if (src[i] === '[') { i += 1; const n = row(']'); out.push({ t: 'root', n, a: group() }); } else out.push({ t: 'sqrt', a: group() });
        } else if (name === 'sum' || name === 'int') {
          const s = struct(name);
          for (let k = 0; k < 2; k += 1) {
            ws();
            if (src[i] === '_') { i += 1; s.lo = group(); } else if (src[i] === '^') { i += 1; s.hi = group(); }
          }
          out.push(s);
        } else if (name === 'left') {
          ws();
          const open = src[i++];
          if (open !== '(' && open !== '|') fail();
          const a = row(`\\right${open === '(' ? ')' : '|'}`);
          out.push({ t: open === '(' ? 'paren' : 'abs', a });
        } else if (COMMAND_TEXT[name]) push(COMMAND_TEXT[name]);
        else if ('{}#$%&'.includes(name)) push(name);
        else if (name === ' ' || name === ',') push(' ');
        else fail();
        if (one) return out;
        continue;
      }
      if (c === '^' || c === '_') {
        i += 1;
        ws();
        if (c === '^' && src.startsWith('{\\circ}', i)) { i += 7; push('°'); continue; }
        out.push({ t: c === '^' ? 'sup' : 'sub', a: group() });
        continue;
      }
      if (c === '{') { i += 1; out.push(...row('}')); continue; }
      push(c);
      i += 1;
      if (one) return out;
    }
    if (end) fail();
    return out;
  };
  try {
    return normalise(row(null));
  } catch {
    return null;
  }
}

/// Text between and around every structure, so there's always somewhere to type.
function normalise(r) {
  const out = [];
  for (const p of r) {
    const last = out[out.length - 1];
    if (p.t === 'text') { if (last?.t === 'text') last.v += p.v; else out.push(p); continue; }
    if (last?.t !== 'text') out.push(text());
    for (const k of STRUCTS[p.t]) p[k] = normalise(p[k]);
    out.push(p);
  }
  if (out[out.length - 1]?.t !== 'text') out.push(text());
  return out;
}

// ----- The editor -----

/// An editor in `host` starting from `latex`; `onChange(latex)` after each edit.
/// Returns { insert(kind or text), focus(), latex() }.
export function mathEditor(host, latex, onChange) {
  let tree = normalise(fromLatex(latex || '') || [text()]);
  // Where to go back to after drawing: the path of the text part and the offset in it.
  let caret = null;

  const changed = () => onChange(toLatex(tree));

  const draw = () => {
    host.replaceChildren(drawRow(tree, []));
    restore();
  };
  // A path is a list of [index, slot key] steps down from the top row, then the part's index.
  const at = (path) => {
    let r = tree;
    for (let k = 0; k < path.length - 1; k += 2) r = r[path[k]][path[k + 1]];
    return r;
  };

  const drawRow = (r, path) => {
    const box = document.createElement('span');
    box.className = `mrow${r.length === 1 && r[0].t === 'text' && !r[0].v && path.length ? ' empty' : ''}`;
    r.forEach((p, i) => box.append(drawPart(p, [...path, i])));
    return box;
  };
  const slot = (p, key, path, cls) => {
    const el = drawRow(p[key], [...path, key]);
    el.classList.add(cls);
    return el;
  };
  const drawPart = (p, path) => {
    if (p.t === 'text') {
      const el = document.createElement('span');
      el.className = 'mtext';
      el.contentEditable = 'true';
      el.spellcheck = false;
      el.textContent = p.v;
      el.dataset.path = JSON.stringify(path);
      el.addEventListener('input', () => {
        p.v = el.textContent;
        // An empty box stops looking empty once typed in.
        const row = el.parentElement;
        if (row.classList.contains('empty') || (!p.v && row.children.length === 1)) row.classList.toggle('empty', !p.v);
        // Typing ^, _ or / makes a power, a lowered part or a fraction.
        const m = /[\^_/]/.exec(p.v);
        if (m) {
          const before = p.v.slice(0, m.index);
          const after = p.v.slice(m.index + 1);
          if (m[0] === '/') {
            // The number or name just before becomes the top of the fraction.
            const top = /([\w.]+)$/.exec(before);
            const f = struct('frac');
            if (top) f.a = [text(top[1])];
            placeStruct(path, top ? before.slice(0, -top[1].length) : before, after, f, top ? 'b' : 'a');
          } else placeStruct(path, before, after, struct(m[0] === '^' ? 'sup' : 'sub'), 'a');
          return;
        }
        changed();
      });
      el.addEventListener('focus', () => { caret = { path, offset: 0 }; });
      el.addEventListener('keyup', () => { caret = { path, offset: offsetIn(el) }; });
      el.addEventListener('click', () => { caret = { path, offset: offsetIn(el) }; });
      el.addEventListener('keydown', (e) => keys(e, el, path));
      return el;
    }
    const box = document.createElement('span');
    box.className = `mpart m-${p.t}`;
    const add = (...kids) => box.append(...kids);
    const sign = (s, cls = 'msign') => { const x = document.createElement('span'); x.className = cls; x.textContent = s; return x; };
    switch (p.t) {
      case 'frac': add(slot(p, 'a', path, 'mnum'), slot(p, 'b', path, 'mden')); break;
      case 'sqrt': add(sign('√'), slot(p, 'a', path, 'mrad')); break;
      case 'root': add(slot(p, 'n', path, 'mindex'), sign('√'), slot(p, 'a', path, 'mrad')); break;
      case 'sup': add(slot(p, 'a', path, 'msup')); break;
      case 'sub': add(slot(p, 'a', path, 'msub')); break;
      case 'sum': case 'int': {
        const limits = document.createElement('span');
        limits.className = 'mlimits';
        limits.append(slot(p, 'hi', path, 'mhi'), sign(p.t === 'sum' ? '∑' : '∫', 'mbig'), slot(p, 'lo', path, 'mlo'));
        add(limits);
        break;
      }
      case 'paren': add(sign('(', 'mfence'), slot(p, 'a', path, 'minner'), sign(')', 'mfence')); break;
      case 'abs': add(sign('|', 'mfence'), slot(p, 'a', path, 'minner'), sign('|', 'mfence')); break;
      default: break;
    }
    return box;
  };

  const offsetIn = (el) => {
    const s = window.getSelection();
    return s && s.anchorNode && el.contains(s.anchorNode) ? s.anchorOffset : el.textContent.length;
  };
  const editables = () => [...host.querySelectorAll('.mtext')];
  const focusEl = (el, offset) => {
    el.focus();
    const r = document.createRange();
    const node = el.firstChild || el;
    const n = Math.min(offset, (node.textContent || '').length);
    r.setStart(node, node === el ? 0 : n);
    r.collapse(true);
    const s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
    caret = { path: JSON.parse(el.dataset.path), offset: n };
  };
  const restore = () => {
    if (!caret) return;
    const el = editables().find((x) => x.dataset.path === JSON.stringify(caret.path));
    if (el) focusEl(el, caret.offset);
  };

  /// Splits the text at `path` into before / the new structure / after, and
  /// puts the cursor in the structure's slot `into`.
  const placeStruct = (path, before, after, s, into) => {
    const r = at(path);
    const i = path[path.length - 1];
    r.splice(i, 1, text(before), s, text(after));
    caret = { path: [...path.slice(0, -1), i + 1, into, 0], offset: (s[into][0]?.v || '').length };
    tree = normalise(tree);
    draw();
    changed();
  };

  const keys = (e, el, path) => {
    const list = editables();
    const k = list.indexOf(el);
    const off = offsetIn(el);
    if (e.key === 'ArrowLeft' && off === 0 && k > 0) { e.preventDefault(); focusEl(list[k - 1], list[k - 1].textContent.length); }
    else if (e.key === 'ArrowRight' && off === el.textContent.length && k < list.length - 1) { e.preventDefault(); focusEl(list[k + 1], 0); }
    else if (e.key === 'Tab') {
      // The next box (Shift: the one before); stops at the ends.
      e.preventDefault();
      const next = list[Math.max(0, Math.min(list.length - 1, k + (e.shiftKey ? -1 : 1)))];
      focusEl(next, e.shiftKey ? next.textContent.length : 0);
    }
    else if (e.key === 'Backspace' && off === 0 && window.getSelection().isCollapsed) {
      // Just after a structure: take it out (keeping what was typed inside it).
      const r = at(path);
      const i = path[path.length - 1];
      const prev = r[i - 1];
      if (prev && prev.t !== 'text') {
        e.preventDefault();
        const inner = STRUCTS[prev.t].map((key) => toPlain(prev[key])).join('');
        const before = r[i - 2]?.t === 'text' ? r[i - 2] : null;
        const keep = before ? before.v.length : 0;
        r.splice(i - 1, 1, text(inner));
        tree = normalise(tree);
        caret = { path: [...path.slice(0, -1), Math.max(0, i - 2)], offset: keep + inner.length };
        draw();
        changed();
      }
    } else if (e.key === 'Enter') e.preventDefault();
  };
  const toPlain = (r) => r.map((p) => (p.t === 'text' ? p.v : STRUCTS[p.t].map((key) => toPlain(p[key])).join(''))).join('');

  draw();
  return {
    /// A structure (by name) or some text at the cursor.
    insert(what) {
      if (!caret) { const first = editables()[0]; caret = { path: JSON.parse(first.dataset.path), offset: first.textContent.length }; }
      const p = at(caret.path)[caret.path[caret.path.length - 1]];
      const before = p.v.slice(0, caret.offset);
      const after = p.v.slice(caret.offset);
      if (STRUCTS[what]) {
        placeStruct(caret.path, before, after, struct(what), STRUCTS[what][0]);
      } else {
        p.v = before + what + after;
        caret.offset += what.length;
        draw();
        changed();
      }
    },
    focus() { restore(); if (!caret) focusEl(editables()[0], 0); },
    latex: () => toLatex(tree),
    /// Starts again from LaTeX (after editing it as LaTeX); false when it can't be shown.
    load(src) { const t = fromLatex(src); if (!t) return false; tree = normalise(t); caret = null; draw(); return true; },
  };
}
