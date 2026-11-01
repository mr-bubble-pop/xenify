// Draw inside Xenify Chat as a chat's whiteboard (index.html?embed=board).
//
// The chat page talks to this frame with messages:
//   chat → board  { t: 'open', url, token, account, node, name }
//                 { t: 'snapshot' }   a picture of the board, for the chat
//                 { t: 'download', format: 'png' | 'pdf' | 'xdraw' }
//   board → chat  { t: 'ready' }, { t: 'opened', others }, { t: 'error', message }
//                 { t: 'picture', name, bytes, purpose }  (purpose: 'snapshot' or 'download')
//                 { t: 'file', name, bytes, mime }
// The board is a file on the Xenify server, edited together like any other;
// the server lets only the chat's members open it.

import { exportPdf, files } from './engine.js';
import { renderPng } from './export.js';
import { originOf } from './server.js';

/// A board is a wide, still, white page (Draw's default is a portrait clip).
/// Every app opening one sets it the same way.
export function boardPage(app, name) {
  const c = app.state?.comp;
  if (c && (c.duration > 0 || c.width < c.height)) {
    app.run('set_comp', { name, width: 2400, height: 1500, duration: 0, background: '#ffffff' });
  }
}

export function start({ app, server, call, applyLayout, showShapes, renderTabs }) {
  document.body.classList.add('embedded');
  // A board: drawing and diagrams, no animation, no slides, no menus.
  app.prefs.hidden = { ...app.prefs.hidden, timeline: true, status: true, slides: true, symbols: true, layers: false };
  applyLayout();
  // Only to the chat page this frame belongs to (the same origin).
  const reply = (m, transfer) => window.parent.postMessage(m, location.origin, transfer);
  let opened = null;
  let name = 'Whiteboard';

  async function open(m) {
    name = m.name || name;
    server.url = m.url.replace(/\/$/, '');
    server.session = { token: m.token, account: m.account, origin: originOf(server.url) };
    server.keep = () => {};
    try {
      opened = await server.open(m.node);
      boardPage(app, name);
      showShapes(true);
      renderTabs();
      app.viewport?.fit();
      reply({ t: 'opened', others: server.here(opened.id).length });
    } catch (e) {
      reply({ t: 'error', message: String(e.message || e) });
    }
  }

  async function picture() {
    const c = app.state.comp;
    // The first frame, wherever the playhead is.
    return renderPng(app, 0, Math.min(2, 2400 / Math.max(c.width, c.height)));
  }

  window.addEventListener('message', async (e) => {
    if (e.source !== window.parent || e.origin !== location.origin) return;
    const m = e.data || {};
    try {
      if (m.t === 'open') await open(m);
      else if (m.t === 'snapshot') {
        const bytes = await picture();
        reply({ t: 'picture', name: `${name}.png`, bytes, purpose: 'snapshot' });
      } else if (m.t === 'download') {
        if (m.format === 'png') reply({ t: 'file', name: `${name}.png`, mime: 'image/png', bytes: await picture() });
        else if (m.format === 'pdf') reply({ t: 'file', name: `${name}.pdf`, mime: 'application/pdf', bytes: exportPdf(app.state.time, 2) });
        else if (m.format === 'xdraw') reply({ t: 'file', name: `${name}.xdraw`, mime: 'application/zip', bytes: files.save(undefined, false) });
      }
    } catch (err) {
      reply({ t: 'error', message: String(err.message || err) });
    }
  });
  reply({ t: 'ready' });
}
