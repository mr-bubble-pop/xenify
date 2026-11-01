// Script errors go to the app's log: the terminal it was started from (desktop), or
// the test server when the page is opened with ?log.
window.__errors = [];
// Inside another Xenify app (a chat's whiteboard, embed.js): Draw works as in
// a browser there, not as the desktop app.
if (new URLSearchParams(location.search).get('embed') === 'board') window.XENIFY_WEB = true;
const report = (text) => {
  window.__errors.push(text);
  try {
    if (window.__TAURI__ && !window.XENIFY_WEB) window.__TAURI__.core.invoke('log', { text });
    else if (new URLSearchParams(location.search).has('log')) fetch('/log', { method: 'POST', body: text });
  } catch { /* no log */ }
};
window.addEventListener('error', (e) => report(`error: ${e.message} at ${e.filename}:${e.lineno}:${e.colno}`));
window.addEventListener('unhandledrejection', (e) => report(`unhandled: ${e.reason && (e.reason.stack || e.reason)}`));
const ce = console.error;
console.error = (...a) => { report(`console: ${a.join(' ')}`); ce(...a); };
