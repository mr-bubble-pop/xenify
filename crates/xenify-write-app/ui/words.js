// Your words (spelling): the ones on this computer, or, signed in to a
// Xenify server, your list there (it follows you to every computer and is
// separate for each server). A document can also have words of its own,
// kept in it (doc_word).

import { call } from './engine.js';
import { ui } from './serverui.js';

const local = {
  get: () => window.__TAURI__.core.invoke('words'),
  set: (words) => window.__TAURI__.core.invoke('set_words', { words }),
};
// The list on the server, while signed in.
let onServer = null;

/// Signed in or out: which list is yours, and the checker told.
export async function syncWords() {
  const srv = ui.server;
  let list = null;
  try { list = srv?.ready ? await srv.myWords(await local.get()) : null; } catch { list = null; }
  onServer = list;
  await call('server_words', { words: list || [] }).catch(() => {});
}

export const words = {
  /// The server whose list it is ('' : this computer's).
  where: () => (onServer ? ui.server.name : ''),
  get: async () => onServer ?? local.get(),
  set: async (list) => {
    if (onServer) {
      onServer = await ui.server.setMyWords(list);
      await call('server_words', { words: onServer });
    } else await local.set(list);
  },
};
