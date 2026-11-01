// Where AI requests go: always the Xenify server Draw is signed in to (its
// admin picks the runner and the models). Without a server with AI, there's
// no AI.

import { ui } from './serverui.js';

/// What the server offers (null when not signed in or it has no AI).
/// `gifs`: the server searches GIPHY for people (its admin set a key).
export const aiServer = { info: null, gifs: false };

/// Asks the server what AI it has (again only after signing in or out, or `force`).
let seen = null;
export async function refreshAiServer(force = false) {
  const s = ui.server;
  const now = s?.ready && s.session ? `${s.url} ${s.session.token}` : '';
  if (!force && now === seen) return;
  seen = now;
  aiServer.info = null;
  aiServer.gifs = false;
  if (!now) return;
  try {
    const info = await s.api('GET', '/v1/ai');
    aiServer.gifs = !!info.gifs;
    if (info.available) aiServer.info = info;
  } catch { /* no AI there */ }
}

export const viaServer = () => !!aiServer.info;
export const hasAi = viaServer;

/// No AI here: says why, and opens Settings › Server when signing in to one would help.
export function noAi(toast, openSettings) {
  const s = ui.server;
  if (s?.session) return void toast(`${s.name || 'Your server'} has no AI yet: its admin can add it on the server's admin pages (AI).`);
  if (s?.servedHere) return void toast('AI comes from your Xenify server: sign in to use it.');
  toast('AI comes from a Xenify server: sign in to one that has AI.');
  openSettings('server');
}

/// A chat request's address, key and body, with the server's models.
export function chatTarget(body) {
  const s = ui.server;
  // Requests that show a frame need a model that can see.
  const sees = JSON.stringify(body.messages || []).includes('"image_url"');
  const model = sees ? aiServer.info.vision_model || '' : '';
  return { url: `${s.url}/v1/ai/chat/completions`, key: s.session.token, body: { ...body, model } };
}

/// Paints a picture with the server's image model: PNG as base64.
export async function serverPicture(prompt) {
  const s = ui.server;
  const out = await s.api('POST', '/v1/ai/images/generations', { prompt, model: '', size: '1024x1024', n: 1, response_format: 'b64_json' });
  const d = out?.data?.[0];
  if (d?.b64_json) return d.b64_json;
  if (d?.url) {
    const bytes = new Uint8Array(await (await fetch(d.url)).arrayBuffer());
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(bin);
  }
  throw new Error('the AI on the server sent no picture');
}

/// A picture asked for as a chat request (the assistant's, OpenRouter
/// style: `modalities: ["image"]`) made with the server's image model, and
/// answered the way OpenRouter answers.
export async function pictureAsChat(body) {
  // A change to a picture (the request shows one): the server's picture model answers the chat request as it is.
  if (JSON.stringify(body.messages || []).includes('"image_url"')) return pictureEditOnServer(body);
  const last = [...(body.messages || [])].reverse().find((m) => m.role === 'user');
  const text = typeof last?.content === 'string' ? last.content : (last?.content || []).filter((p) => p.type === 'text').map((p) => p.text).join('\n');
  const b64 = await serverPicture(text || 'an illustration');
  return { choices: [{ message: { role: 'assistant', content: '', images: [{ type: 'image_url', image_url: { url: `data:image/png;base64,${b64}` } }] } }] };
}

/// The request asks for a picture.
export const wantsPicture = (body) => Array.isArray(body?.modalities) && body.modalities.includes('image');

/// A change to a picture (an image-editing chat request), with the server's picture model.
export function pictureEditOnServer(body) {
  return ui.server.api('POST', '/v1/ai/chat/completions', { ...body, model: aiServer.info?.image_model || '' });
}
