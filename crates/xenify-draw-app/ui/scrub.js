// Numbers you can drag, as in the old app: drag left or right on a number
// box (or on its name) to change it; Shift is ten times faster, Alt ten times
// finer. A click without dragging types in the box as usual. The wheel steps
// the box that has the focus. One drag is one undo step.

import { call } from './engine.js';

// Number boxes: type=number, and the text boxes for numbers (they take sums like 100+20).
const NUM = 'input[type=number], input[inputmode=decimal]';
const isNum = (el) => el?.tagName === 'INPUT' && el.matches(NUM) && !el.disabled && !el.readOnly;

/// The number box a press on `t` would drag: the box itself, its unit, or the first one of the name it's under.
function target(t) {
  if (isNum(t)) return t;
  const unit = t.closest?.('.num > small');
  if (unit) return unit.parentElement.querySelector(NUM);
  const name = t.closest?.('.pname, label > span:first-child, .lab');
  if (!name) return null;
  const box = name.closest('label, .prop')?.querySelector(NUM);
  return isNum(box) ? box : null;
}

const stepOf = (el) => { const s = Number(el.step); return Number.isFinite(s) && s > 0 ? s : 1; };
const decimals = (s) => (String(s).split('.')[1] || '').length;

function set(el, v) {
  const min = el.min === '' ? -Infinity : Number(el.min), max = el.max === '' ? Infinity : Number(el.max);
  const s = stepOf(el);
  v = Math.max(min, Math.min(max, v));
  el.value = String(Number(v.toFixed(Math.min(6, decimals(s) + 1))));
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

export function numberScrubbing() {
  let d = null, justScrubbed = false;
  document.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const el = target(e.target);
    // A box being typed in keeps its usual behaviour (selecting, moving the caret).
    if (!el || document.activeElement === el) return;
    d = { el, x: e.clientX, start: Number(el.value) || 0, moving: false, onBox: e.target === el, id: e.pointerId };
    if (!d.onBox) e.preventDefault();
  }, true);
  document.addEventListener('pointermove', (e) => {
    if (!d || e.pointerId !== d.id) return;
    const dx = e.clientX - d.x;
    if (!d.moving) {
      if (Math.abs(dx) < 4) return;
      d.moving = true;
      d.el.blur();
      document.body.classList.add('scrubbing');
      try { call('begin_gesture', { label: `Change ${(d.el.getAttribute('aria-label') || 'a value').replace(/ [XY%]$/, '').toLowerCase()}` }); d.group = true; } catch { /* no engine yet */ }
    }
    e.preventDefault();
    const k = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
    const v = d.start + Math.round(dx) * stepOf(d.el) * k;
    if (d.queued != null) { d.queued = v; return; }
    d.queued = v;
    requestAnimationFrame(() => { if (!d) return; const q = d.queued; d.queued = null; set(d.el, q); });
  }, true);
  const end = (e) => {
    if (!d || e.pointerId !== d.id) return;
    const was = d;
    d = null;
    document.body.classList.remove('scrubbing');
    if (was.moving) {
      justScrubbed = true;
      setTimeout(() => { justScrubbed = false; }, 0);
      if (was.queued != null) set(was.el, was.queued);
      if (was.group) { try { call('end_gesture'); } catch { /* fine */ } }
      e.preventDefault();
      // The drag's last change already refreshed the screens; this records the step.
      window.drawApp?.changed?.();
    } else if (!was.onBox && was.el.isConnected) { was.el.focus(); was.el.select(); }
  };
  document.addEventListener('pointerup', end, true);
  document.addEventListener('pointercancel', end, true);
  // A drag that changed a value isn't also a click (which could toggle what's under it).
  document.addEventListener('click', (e) => { if (justScrubbed) { justScrubbed = false; e.preventDefault(); e.stopPropagation(); } }, true);
  document.addEventListener('wheel', (e) => {
    const el = document.activeElement;
    if (!isNum(el) || !el.matches(':hover')) return;
    e.preventDefault();
    const k = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
    set(el, (Number(el.value) || 0) - Math.sign(e.deltaY || e.deltaX) * stepOf(el) * k);
  }, { passive: false });
}
