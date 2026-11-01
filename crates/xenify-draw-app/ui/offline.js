// The browser version keeps itself for offline use (sw.js). When a newer
// build has been fetched, `onUpdate` is told, and `applyUpdate` switches
// to it (the page reloads).

export function keepOffline(onUpdate) {
  if (window.__TAURI__ || !('serviceWorker' in navigator) || !window.isSecureContext) return;
  navigator.serviceWorker.register('sw.js').then((reg) => {
    const waiting = () => { if (reg.waiting && navigator.serviceWorker.controller) onUpdate(); };
    waiting();
    reg.addEventListener('updatefound', () => {
      reg.installing?.addEventListener('statechange', waiting);
    });
  }).catch(() => { /* not kept: works online as usual */ });
  let reloading = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (reloading) return;
    reloading = true;
    location.reload();
  });
}

export async function applyUpdate() {
  const reg = await navigator.serviceWorker.getRegistration();
  reg?.waiting?.postMessage('update');
}
