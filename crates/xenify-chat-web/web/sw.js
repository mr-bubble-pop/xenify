// Xenify Chat's service worker: shows what the server pushes (new messages
// and reminders) while no Chat tab is open. Reminders get Snooze and
// Dismiss buttons where the browser has them (Chrome, Edge); they work
// without the page, with the token the server put in the reminder. Clicking
// the notification opens Chat, which shows the reminder with every choice.

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

const MIN = 60_000;

self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data?.json() || {}; } catch { /* not ours */ }
  e.waitUntil((async () => {
    // A Chat tab is open: it shows things itself.
    const open = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    if (open.length) return;
    const reminder = d.kind === 'reminder';
    const actions = [];
    if (reminder && d.act) {
      actions.push({ action: 'snooze5', title: 'Snooze 5 min' });
      if (d.starts && d.starts - MIN > Date.now() + 30_000) actions.push({ action: 'before', title: '1 min before it starts' });
      else actions.push({ action: 'snooze10', title: 'Snooze 10 min' });
      actions.push({ action: 'snooze1', title: 'Snooze 1 min' }, { action: 'tomorrow', title: 'Tomorrow' }, { action: 'dismiss', title: 'Dismiss' });
    }
    const max = self.Notification?.maxActions ?? 2;
    await self.registration.showNotification(d.title || 'Xenify Chat', {
      body: d.body || '',
      icon: 'icon.png',
      badge: 'icon.png',
      tag: reminder ? `reminder:${d.key}` : d.conv ? `chat:${d.conv}` : undefined,
      renotify: true,
      requireInteraction: reminder,
      data: d,
      actions: actions.slice(0, max),
    });
  })());
});

self.addEventListener('notificationclick', (e) => {
  const d = e.notification.data || {};
  e.notification.close();
  const until = {
    snooze1: Date.now() + MIN,
    snooze5: Date.now() + 5 * MIN,
    snooze10: Date.now() + 10 * MIN,
    before: (d.starts || 0) - MIN,
    tomorrow: (() => {
      const t = new Date();
      t.setDate(t.getDate() + 1);
      t.setHours(9, 0, 0, 0);
      return t.getTime();
    })(),
  }[e.action];
  if (e.action && d.act) {
    const body = e.action === 'dismiss' ? { token: d.act } : { token: d.act, until };
    e.waitUntil(fetch(new URL('/v1/push/act', self.location.origin), { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }).catch(() => {}));
    return;
  }
  // Open Chat: the tab that's there, or a new one (on the chat it's about).
  e.waitUntil((async () => {
    const open = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const conv = d.conv || 0;
    if (open.length) {
      await open[0].focus();
      if (conv) open[0].postMessage({ open: conv });
      return;
    }
    await self.clients.openWindow(new URL(conv ? `./?open=${conv}` : './', self.registration.scope).href);
  })());
});
