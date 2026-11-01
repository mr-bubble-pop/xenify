// Line icons for Xenify Calc, drawn for it in the style of the other Xenify
// apps (24×24, 1.75 stroke). Listed in assets/SOURCES.toml.

const P = {
  plus: 'M12 5v14M5 12h14',
  close: 'M7 7l10 10M17 7 7 17',
  undo: 'M9 7 4.5 11.5 9 16M5 11.5h9.5a5 5 0 0 1 0 10H11',
  redo: 'M15 7l4.5 4.5L15 16M19 11.5H9.5a5 5 0 0 0 0 10H13',
  book: 'M3 4h18v16H3zM3 9h18M9 9v11',
  color: 'M7 16l5-11 5 11M8.8 12.5h6.4',
  fill: 'M5 12l6-6 6 6-6 6zM18.5 14.5s1.5 2 1.5 3a1.5 1.5 0 0 1-3 0c0-1 1.5-3 1.5-3z',
  borders: 'M4 4h16v16H4zM4 12h16M12 4v16',
  left: 'M4 6h16M4 10h10M4 14h16M4 18h10',
  center: 'M4 6h16M7 10h10M4 14h16M7 18h10',
  right: 'M4 6h16M10 10h10M4 14h16M10 18h10',
  wrap: 'M4 6h16M4 12h13a3 3 0 0 1 0 6h-4M15 15.5 12.5 18 15 20.5M4 18h5',
  sort: 'M7 4v16M3.5 16.5 7 20l3.5-3.5M13 6h8M13 11h6M13 16h4',
  filter: 'M4 5h16l-6 7.5V18l-4 2v-7.5z',
  grid: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  down: 'M6 9l6 6 6-6',
  search: 'M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM20 20l-4.8-4.8',
  check: 'M5 12.5l4.5 4.5L19 7.5',
  table: 'M4 5h16v14H4zM4 10h16M4 15h16M10 10v9',
  chart: 'M4 20V4M4 20h16M8 16v-4M12 16V8M16 16v-6',
  summary: 'M4 5h16M4 10h10M4 15h7M15 13l3 3 3-3M18 16V8',
  database: 'M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3',
  play: 'M8 5v14l11-7z',
  refresh: 'M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
};
export function icon(name, size = 18) {
  return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="${P[name] || ''}"/></svg>`;
}

