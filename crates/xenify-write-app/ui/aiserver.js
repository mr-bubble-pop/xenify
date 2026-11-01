// Where Ask AI's requests go: always the Xenify server Write is signed in to
// (its admin picks the runner and the models). Without a server with AI,
// there's no AI.

import { ui } from './serverui.js';

// Left over from when Write could use the person's own OpenRouter key.
try { localStorage.removeItem('xenify-write-ai-use'); localStorage.removeItem('xenify-write-ai-use-model'); } catch { /* not kept */ }

/// What the server offers (null when not signed in or it has no AI).
export const aiServer = { info: null };

/// Asks the server what AI it has and tells the engine to use it (again
/// only after signing in or out, or `force`).
let seen = null;
export async function refreshAiServer(force = false) {
  const s = ui.server;
  const now = s?.ready && s.session ? `${s.url} ${s.session.token}` : '';
  if (!force && now === seen) return;
  seen = now;
  aiServer.info = null;
  if (now) {
    try {
      const info = await s.api('GET', '/v1/ai');
      if (info.available) aiServer.info = info;
    } catch { /* no AI there */ }
  }
  // The model is left empty: the server's admin chooses it.
  const on = !!aiServer.info;
  await window.__TAURI__.core.invoke('ai', { name: 'use_server', args: on ? { url: s.url, token: s.session.token } : {} });
}

export const hasAi = () => !!aiServer.info;

/// No AI here: says why, and opens Settings › Server when signing in to one would help.
export function noAi(toast, openSettings) {
  const s = ui.server;
  if (s?.session) return void toast(`${s.name || 'Your server'} has no AI yet: its admin can add it on the server's admin pages (AI).`);
  if (s?.servedHere) return void toast('AI comes from your Xenify server: sign in to use it.');
  toast('AI comes from a Xenify server: sign in to one that has AI.');
  openSettings('server');
}
