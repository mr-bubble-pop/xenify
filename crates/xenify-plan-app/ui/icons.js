// Line icons for Xenify Plan, drawn for it in the style of the other Xenify
// apps (24×24, 1.75 stroke). Listed in assets/SOURCES.toml.

const P = {
  plus: 'M12 5v14M5 12h14',
  close: 'M7 7l10 10M17 7 7 17',
  undo: 'M9 7 4.5 11.5 9 16M5 11.5h9.5a5 5 0 0 1 0 10H11',
  redo: 'M15 7l4.5 4.5L15 16M19 11.5H9.5a5 5 0 0 0 0 10H13',
  plan: 'M4 6h9M8 12h11M5 18h8',
  filter: 'M4 5h16l-6 7.5V18l-4 2v-7.5z',
  down: 'M6 9l6 6 6-6',
  search: 'M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM20 20l-4.8-4.8',
  check: 'M5 12.5l4.5 4.5L19 7.5',
  indent: 'M4 5h16M10 12h10M10 19h10M4 9.5l3 2.5-3 2.5',
  outdent: 'M4 5h16M10 12h10M10 19h10M7 9.5 4 12l3 2.5',
  link: 'M3 4h8v5H3zM13 15h8v5h-8zM11 6.5h4v8.5M13 13l2 2 2-2',
  move: 'M3 12h18M16 7l5 5-5 5',
  baseline: 'M4 9h12M4 15h16',
  clock: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 7.5V12l3 2',
  pin: 'M9 3h6l-.7 5.5 2.7 2.7H7l2.7-2.7zM12 11.2V20',
  person: 'M12 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM4.5 20a7.5 7.5 0 0 1 15 0',
  group: 'M9 5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7zM2.5 19.5a6.5 6.5 0 0 1 13 0M16 5.5a3 3 0 0 1 0 6M18 14a5.5 5.5 0 0 1 3.5 5.5',
  thing: 'M4 5h16v11H4zM9 20h6M12 16v4',
  trash: 'M5 7h14M10 4h4M7 7l1 13h8l1-13',
  // Three round dots (each a little ring with no hole): plain .01 lines were dots too small to see.
  more: 'M4 12a1 1 0 1 0 2 0a1 1 0 1 0-2 0M11 12a1 1 0 1 0 2 0a1 1 0 1 0-2 0M18 12a1 1 0 1 0 2 0a1 1 0 1 0-2 0',
  warn: 'M12 3.5 21.5 20h-19zM12 10v4.5M12 17.5v.01',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1',
  calendar: 'M4 6h16v14H4zM4 10h16M8 3.5V7M16 3.5V7',
  board: 'M4 4h4.5v16H4zM10 4h4.5v10H10zM16 4h4v13h-4z',
  list: 'M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01',
  gantt: 'M4 6h8M8 12h10M6 18h6',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  bullets: 'M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01',
  numbers: 'M10 6h10M10 12h10M10 18h10M4 4.5h1.5V9M4 15.5c.5-1 2.5-1 2.5.3 0 1-2.5 1.7-2.5 3.2h2.6',
  weblink: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  people: 'M9 5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7zM2.5 19.5a6.5 6.5 0 0 1 13 0M16 5.5a3 3 0 0 1 0 6M18 14a5.5 5.5 0 0 1 3.5 5.5',
  timeline: 'M4 6h7M7 12h10M11 18h9',
  comment: 'M4 5h16v11H9l-5 4z',
  sparkle: 'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8z',
};

export function icon(name, size = 18) {
  return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="${P[name] || ''}"/></svg>`;
}
