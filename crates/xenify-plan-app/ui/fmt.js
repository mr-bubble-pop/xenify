// Dates, people and colours as the screens show them.

const W = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const M = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/// "2026-11-16" → days since 1970-01-01 (what the engine counts in).
export function days(s) {
  if (!s) return null;
  const [y, m, d] = s.split('-').map(Number);
  return Math.round(Date.UTC(y, m - 1, d) / 86400000);
}
export function iso(n) {
  return new Date(n * 86400000).toISOString().slice(0, 10);
}
/// 0 = Monday.
export const weekday = (n) => ((n % 7) + 10) % 7;
export function parts(n) {
  const d = new Date(n * 86400000);
  return { y: d.getUTCFullYear(), m: d.getUTCMonth(), d: d.getUTCDate() };
}
/// "Mon 16 Nov".
export function dayName(s) {
  if (!s) return '—';
  const n = days(s);
  const p = parts(n);
  return `${W[weekday(n)]} ${p.d} ${M[p.m]}`;
}
/// "16 Nov".
export function short(s) {
  if (!s) return '—';
  const p = parts(days(s));
  return `${p.d} ${M[p.m]}`;
}
export const monthShort = (m) => M[m];

/// How far a date moved from what was expected: "3 days later",
/// "2 days earlier", "On time"; '' when either is missing.
export function moved(expected, now) {
  if (!expected || !now) return '';
  const n = days(now) - days(expected);
  if (!n) return 'On time';
  return `${Math.abs(n)} day${Math.abs(n) === 1 ? '' : 's'} ${n > 0 ? 'later' : 'earlier'}`;
}

/// "Today", "Yesterday", "Fri 13 Nov 16:20".
export function when(ms) {
  if (!ms) return '';
  const d = new Date(ms);
  const t = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  const today = new Date();
  const same = (a, b) => a.toDateString() === b.toDateString();
  if (same(d, today)) return `Today ${t}`;
  const y = new Date(today);
  y.setDate(y.getDate() - 1);
  if (same(d, y)) return `Yesterday ${t}`;
  return `${W[(d.getDay() + 6) % 7]} ${d.getDate()} ${M[d.getMonth()]} ${t}`;
}
export function dayOf(ms) {
  if (!ms) return '';
  const d = new Date(ms);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return 'Today';
  return `${W[(d.getDay() + 6) % 7]} ${d.getDate()} ${M[d.getMonth()]}`;
}

/// Today here, as "2026-11-16".
export function todayIso() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/// "Priya Shah" → "PS".
export function initials(name) {
  const w = (name || '?').trim().split(/\s+/);
  return ((w[0]?.[0] || '?') + (w.length > 1 ? w[w.length - 1][0] : '')).toUpperCase();
}

// Avatar colours: picked from the id so a person keeps theirs.
const FACES = ['#2a78d6', '#1baf7a', '#6250d6', '#C2410C', '#eda100', '#0F766E', '#e87ba4', '#57534E'];
export function faceColour(id) {
  let h = 0;
  for (const c of id || '') h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return FACES[h % FACES.length];
}

export const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
