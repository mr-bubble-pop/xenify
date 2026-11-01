// Script errors go to the app's log (the terminal it was started from).
const report = (text) => { try { window.__TAURI__?.core.invoke('engine', { name: 'log', args: { text } }); } catch { /* no log */ } };
window.addEventListener('error', (e) => report(`error: ${e.message} at ${e.filename}:${e.lineno}:${e.colno}`));
window.addEventListener('unhandledrejection', (e) => report(`unhandled: ${e.reason && (e.reason.stack || e.reason)}`));
