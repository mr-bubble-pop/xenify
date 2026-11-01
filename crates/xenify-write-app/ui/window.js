// The window has no system border: the menu bar and tab strip move it
// (double-click maximises), the edges resize it, and the corners are
// rounded until it's maximised (where the desktop can draw that). The browser build keeps the browser's window.

const tw = window.__TAURI__?.window;

export function setUpWindow() {
  if (!tw || window.XENIFY_WEB) return;
  const win = tw.getCurrentWindow();
  const root = document.documentElement;
  root.classList.add('frameless');
  // Rounded corners need a see-through window (the app checks the desktop can draw one).
  if (window.XENIFY_ROUNDED) root.classList.add('rounded');

  // Moving: empty parts of the menu bar and tab strip.
  const menubar = document.querySelector('.menubar');
  const fillGap = document.createElement('div');
  fillGap.className = 'drag-gap';
  menubar.append(fillGap);
  for (const el of [menubar, fillGap, document.querySelector('.brand'), document.querySelector('.tabstrip'), document.getElementById('tabs')]) {
    el?.setAttribute('data-tauri-drag-region', '');
  }

  // Minimise, maximise, close.
  const svg = (d) => `<svg aria-hidden="true" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round">${d}</svg>`;
  const button = (label, d, act, cls = '') => {
    const b = document.createElement('button');
    b.className = `win-btn ${cls}`;
    b.setAttribute('aria-label', label);
    b.title = label;
    b.innerHTML = svg(d);
    b.addEventListener('click', act);
    return b;
  };
  const box = document.createElement('div');
  box.className = 'win-btns';
  box.append(
    button('Minimise', '<path d="M6 12h12"/>', () => win.minimize()),
    button('Maximise', '<rect x="6" y="6" width="12" height="12" rx="1.5"/>', () => win.toggleMaximize()),
    // Closing asks about unsaved work first (the app listens for the close request).
    button('Close', '<path d="M7 7l10 10M17 7 7 17"/>', () => win.close(), 'close'),
  );
  menubar.append(box);

  // Resizing: thin edges and corners.
  for (const dir of ['North', 'South', 'East', 'West', 'NorthEast', 'NorthWest', 'SouthEast', 'SouthWest']) {
    const edge = document.createElement('div');
    edge.className = `resize-edge ${dir.toLowerCase()}`;
    edge.addEventListener('mousedown', (e) => {
      if (e.button !== 0 || root.classList.contains('maximized')) return;
      e.preventDefault();
      win.startResizeDragging(dir).catch(() => {});
    });
    document.body.append(edge);
  }

  // Square corners while maximised.
  const check = async () => {
    try { root.classList.toggle('maximized', await win.isMaximized()); } catch { /* not allowed */ }
  };
  window.addEventListener('resize', check);
  check();
}
