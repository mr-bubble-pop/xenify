// Xenify Chat's screens. The state uses the chat server's shapes (see
// xenify-chat-proto). Signed in, it comes from the server (through the Rust
// side's `to_app` events); with XENIFY_SAMPLE=1, or in a plain browser, the
// sample chats are shown instead and changes stay on screen.

import { sample } from "./sample.js";

const tauri = window.__TAURI__;

const state = {
  me: null,
  server: "",
  live: false, // talking to a server (not the sample chats)
  net: "online", // "online" | "connecting" | "offline"
  conversations: [],
  messages: {}, // conv id -> [Message], oldest first
  sel: null,
  view: "loading", // "loading" | "signin" | "chat" | "settings"
  signin: { join: false, error: "", busy: false, invite: "" },
  people: [], // people on the server matching the search
  found: [], // groups and channels on the server matching the search
  browse: [], // public channels you could follow
  typing: {}, // conv id -> { user, until }
  presence: {}, // user id -> "online" | "away" | "busy" | "offline"
  kept: { q: "", list: [] }, // messages kept on this computer matching the search
  // Email: accounts; folders by account; the folder shown ("" every inbox,
  // "drafts", or "account\nwire"); its list; news by account.
  mail: { accounts: [], folders: {}, folder: "", list: [], status: {}, problems: {}, found: [], opened: null, loading: false, drafts: [], answered: {} },
  mailSel: null, // { account, folder, uid } the email open on the right
  compose: null, // an email being written: { to, cc, subject, text, quote, replyTo, title, busy, error }
  mailSetup: null, // adding an account in Settings
  cal: { today: [], week: [], weekStart: 0, more: false, snoozed: [] }, // the calendar (this computer); snoozed: Later
  apps: { list: [], shown: null }, // your apps; a key just made (shown once)
  appsIn: {}, // conv id -> the apps in it
  dialog: null, // { kind: "group" | "channel" | "add" | "server", ... }
  wantOpen: null, // a chat to open as soon as the server sends it
  toast: "",
  section: "writing",
  tab: "all",
  q: "",
  find: null, // { q, ids: [message id], at: index } inside the open chat
  flash: null, // a message id to highlight briefly
  menu: null, // "notify" | "more" | "status" | "style"
  ctx: null, // { x, y, items } a right-click menu
  confirm: null, // { title, text, action, run }
  editing: null, // message id being edited
  picker: null, // message id whose reaction row is open
  status: "available",
  readLine: {}, // conv id -> the seq you'd read up to when you opened it
  drafts: {},
  settings: {
    spelling: true, grammar: true, spotter: false, before: false,     desktop: true, preview: true, sound: true, mentions: true, quiet: "Off",
    theme: "system", size: 100,
  },
  words: [], // your dictionary (shared with Xenify Write)
  issues: {}, // conv id -> { text, list } what the checker found in the draft
  ignored: {}, // conv id -> keys of mistakes you chose to ignore
  spotter: "off", // the AI spotter: "on", "off", "not installed", "unavailable"
};

// ---------- Helpers ----------

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const nameOf = (u) => u.display_name || u.username;
/** A sender's current name (messages keep the name from when they were sent). */
const senderName = (c, u) => nameOf(c?.members?.find((x) => x.id === u.id) || u);
const initials = (s) => s.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "?";
const palette = [["#FDE6D8", "#8A2C0A"], ["#DCE9FB", "#1D4F91"], ["#DDF3E4", "#1E6B3A"], ["#EDE4FA", "#5B3A91"], ["#FCE7F0", "#8E2557"], ["#E2F0F3", "#1D5C6B"], ["#FFF1D6", "#7A4B00"]];
const colours = (key) => palette[[...String(key)].reduce((a, c) => a + c.charCodeAt(0), 0) % palette.length];
const unread = (c) => c.last_seq > c.read_seq;
const muted = (c) => c.notify === "off" || (c.muted_until && c.muted_until > Date.now());
const others = (c) => c.members.filter((m) => m.id !== state.me.id);
const conv = (id) => state.conversations.find((c) => c.id === id) || (state.preview?.id === id ? state.preview : undefined);
const current = () => conv(state.sel);
const findMsg = (id) => {
  for (const [cid, list] of Object.entries(state.messages)) {
    const m = list.find((x) => x.id === id);
    if (m) return [Number(cid), m];
  }
  return [null, null];
};

function titleOf(c) {
  if (c.title) return c.title;
  if (c.kind === "direct") {
    const o = others(c);
    return o.length ? nameOf(o[0]) : "Notes to self";
  }
  return others(c).map(nameOf).join(", ") || "Just you";
}

function subOf(c) {
  if (c.kind === "channel") return `Channel · ${c.member_count} following${c.open ? " · open to everyone" : ""}`;
  if (c.kind === "group") return others(c).map((m) => nameOf(m).split(" ")[0]).join(", ") + " and you";
  const o = others(c)[0];
  if (!o) return "Only you";
  if (o.app) return "App · answers automatically";
  return state.live ? presenceOf(o).label : "Direct chat";
}

function timeOf(ms) {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return "Yesterday";
  if (now - d < 6 * 864e5) return d.toLocaleDateString([], { weekday: "short" });
  return d.toLocaleDateString([], { day: "numeric", month: "short" });
}

function dayOf(ms) {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return "Today";
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
}

function lastLine(c) {
  if (!c.last) return "No messages yet";
  const who = c.last.sender.id === state.me.id ? "You: " : c.kind === "direct" ? "" : senderName(c, c.last.sender).split(" ")[0] + ": ";
  return c.last.deleted ? who + "deleted a message" : who + c.last.text;
}

function avatarFor(c, size = "") {
  const t = titleOf(c);
  // A direct chat wears the other person's colours, as their messages do.
  const o = c.kind === "direct" ? others(c)[0] : null;
  const [bg, fg] = o ? colours(o.id + nameOf(o)) : colours(c.id + t);
  const inner = c.kind === "channel" ? icons.channel : esc(initials(t));
  return `<span class="avatar ${c.kind === "direct" ? "" : "square"} ${size}" style="background:${bg};color:${fg}" aria-hidden="true">${inner}${o ? dot(o) : ""}</span>`;
}

/** Escapes `text`, marking where `q` appears (any case). */
function marked(text, q) {
  if (!q) return esc(text);
  const lower = text.toLowerCase();
  const needle = q.toLowerCase();
  let out = "";
  let i = 0;
  for (;;) {
    const j = lower.indexOf(needle, i);
    if (j < 0) break;
    out += esc(text.slice(i, j)) + "<mark>" + esc(text.slice(j, j + needle.length)) + "</mark>";
    i = j + needle.length;
  }
  return out + esc(text.slice(i));
}

/** A short piece of `text` around the first `q`, for a search result. */
function snippet(text, q) {
  const j = text.toLowerCase().indexOf(q.toLowerCase());
  const start = Math.max(0, j - 30);
  const end = Math.min(text.length, j + q.length + 50);
  return (start > 0 ? "…" : "") + marked(text.slice(start, end), q) + (end < text.length ? "…" : "");
}

const icons = {
  flame: `<svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3c1 3.5 5 5.5 5 10a5 5 0 0 1-10 0c0-2 1-3.5 2-4.5 0 1.5.8 2.5 2 3 0-3 .5-5.5 1-8.5z"/></svg>`,
  search: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--muted)" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>`,
  channel: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 11v2a1 1 0 0 0 1 1h2l5 4V6L6 10H4a1 1 0 0 0-1 1z"/><path d="M15 9a4 4 0 0 1 0 6"/><path d="M18 6a8 8 0 0 1 0 12"/></svg>`,
  bell: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.9 1.9 0 0 0 3.4 0"/></svg>`,
  bellOff: (size = 20) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M13.7 21a2 2 0 0 1-3.4 0"/><path d="M18.6 13A17.9 17.9 0 0 1 18 8"/><path d="M6.3 6.3A5.9 5.9 0 0 0 6 8c0 7-3 9-3 9h14"/><path d="M18 8a6 6 0 0 0-9.3-5"/><path d="m2 2 20 20"/></svg>`,
  more: (size = 20) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true"><path d="M12 5h.01M12 12h.01M12 19h.01"/></svg>`,
  people: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.9"/><path d="M16 3.1a4 4 0 0 1 0 7.8"/></svg>`,
  gear: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>`,
  smile: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M8.5 14a4 4 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/><path d="M19 3v4M17 5h4"/></svg>`,
  mail: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/></svg>`,
  reply: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 6 6v4"/></svg>`,
  forward: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0-6 6v4"/></svg>`,
  plus: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>`,
  clip: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21.4 11.1-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/></svg>`,
  check: (colour) => `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="${colour}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>`,
  send: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m22 2-7 20-4-9-9-4z"/><path d="M22 2 11 13"/></svg>`,
  up: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m18 15-6-6-6 6"/></svg>`,
  down: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>`,
  close: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg>`,
  back: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg>`,
  caret: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>`,
};

const NOTIFY = [
  { id: "all", label: "All new messages", short: "All messages" },
  { id: "mentions", label: "Only when I'm mentioned", short: "Mentions only" },
  { id: "hour", label: "Mute for 1 hour", short: "Muted for 1 hour" },
  { id: "tomorrow", label: "Mute until tomorrow", short: "Muted until tomorrow" },
  { id: "off", label: "Mute until I turn it back on", short: "Muted" },
];

function notifyLevel(c) {
  if (c.notify === "off") return "off";
  if (c.muted_until && c.muted_until > Date.now()) return c.muted_until - Date.now() <= 3_600_000 + 5_000 ? "hour" : "tomorrow";
  return c.notify;
}

function setNotify(c, id) {
  setNotifyHere(c, id);
  tell({ t: "set_notify", conv: c.id, notify: c.notify, muted_until: c.muted_until });
}

function setNotifyHere(c, id) {
  if (id === "hour") Object.assign(c, { notify: "all", muted_until: Date.now() + 3_600_000 });
  else if (id === "tomorrow") {
    const t = new Date();
    t.setDate(t.getDate() + 1);
    t.setHours(8, 0, 0, 0);
    Object.assign(c, { notify: "all", muted_until: t.getTime() });
  } else Object.assign(c, { notify: id, muted_until: null });
}

const STATUS = {
  available: { label: "Available", short: "Available", hint: "Notifications as you've set them", dot: "var(--ok)" },
  away: { label: "Away", short: "Away", hint: "Others see you're away; notifications as usual", dot: "var(--away)" },
  dnd: { label: "Do not disturb", short: "Do not disturb · notifications paused", hint: "Pause all notifications; others see you're busy", dot: "var(--danger)" },
};
/** What others see for each of your statuses. */
const STATUS_SHARED = { available: "online", away: "away", dnd: "busy" };

/** Someone else's presence: label and dot colour (nothing when offline). */
const PRESENCE = {
  online: { label: "Online", dot: "var(--ok)" },
  away: { label: "Away", dot: "var(--away)" },
  busy: { label: "Do not disturb", dot: "var(--danger)" },
  offline: { label: "Offline", dot: "" },
};
const presenceOf = (u) => PRESENCE[state.presence[u.id] || "offline"];
const dot = (u) => (state.live && presenceOf(u).dot ? `<span class="presence" style="background:${presenceOf(u).dot}" title="${presenceOf(u).label}"></span>` : "");

// Quick reactions: the emoji, and what a screen reader says.
const REACTIONS = [["👍", "thumbs up"], ["❤️", "heart"], ["😂", "laughing"], ["😮", "surprised"], ["😢", "sad"], ["🙏", "thank you"]];
const reactionName = (e) => (REACTIONS.find(([x]) => x === e) || [e, e])[1];

/** Adds or takes back your reaction `emoji` on message `m`. */
function react(m, emoji) {
  m.reactions ||= [];
  let r = m.reactions.find((x) => x.emoji === emoji);
  if (!r) m.reactions.push((r = { emoji, users: [] }));
  const i = r.users.findIndex((u) => u.id === state.me.id);
  if (i >= 0) r.users.splice(i, 1);
  else r.users.push(state.me);
  m.reactions = m.reactions.filter((x) => x.users.length);
}

function names(users) {
  const n = users.map((u) => (u.id === state.me.id ? "you" : nameOf(u)));
  return n.length <= 1 ? n.join("") : n.slice(0, -1).join(", ") + " and " + n[n.length - 1];
}

function reactionRow(m) {
  if (!m.reactions?.length) return "";
  return `<div class="reactions">${m.reactions.map((r) => {
    const mine = r.users.some((u) => u.id === state.me.id);
    const who = names(r.users);
    return `<button class="reaction${mine ? " own" : ""}" data-act="react" data-id="${m.id}" data-emoji="${r.emoji}" title="${esc(who)}" aria-pressed="${mine}" aria-label="${esc(reactionName(r.emoji))} from ${esc(who)}${mine ? ". Take yours back" : ". Add yours"}"><span aria-hidden="true">${r.emoji}</span>${r.users.length}</button>`;
  }).join("")}</div>`;
}

function reactionPicker(id) {
  return `<div class="picker" role="group" aria-label="React">${REACTIONS.map(([e, n]) => `<button data-act="react" data-id="${id}" data-emoji="${e}" aria-label="React with ${n}" title="${n}">${e}</button>`).join("")}</div>`;
}


// ---------- Search ----------

/** Every message, in every chat, containing `q`: from the messages kept on
 * this computer when signed in, else those loaded. */
function messageHits(q) {
  if (state.live && state.kept.q === q.trim()) {
    return state.kept.list.map((m) => ({ c: conv(m.conv), m })).filter((h) => h.c);
  }
  const hits = [];
  const needle = q.toLowerCase();
  for (const c of state.conversations) {
    for (const m of state.messages[c.id] || []) {
      if (!m.deleted && m.text.toLowerCase().includes(needle)) hits.push({ c, m });
    }
  }
  return hits.sort((a, b) => b.m.at - a.m.at);
}

function searchResults() {
  const q = state.q.trim();
  const chats = state.conversations.filter((c) => titleOf(c).toLowerCase().includes(q.toLowerCase()));
  const hits = messageHits(q);
  const chatRows = chats.map((c) => `<button class="row compact" data-act="open" data-id="${c.id}">${avatarFor(c, "small")}<span class="body"><span class="name">${marked(titleOf(c), q)}</span><span class="last">${esc(subOf(c))}</span></span></button>`).join("");
  const hitRows = hits.map(({ c, m }) => `<button class="hit" data-act="hit" data-id="${m.id}">
      <span class="where"><b>${esc(titleOf(c))}</b>${m.sender.id === state.me.id ? " · You" : c.kind === "direct" ? "" : ` · ${esc(senderName(c, m.sender))}`} · ${esc(timeOf(m.at))}</span>
      <span class="text">${snippet(m.text, q)}</span>
    </button>`).join("");
  return `<div class="results">
    ${chats.length ? `<div class="section">Chats</div>${chatRows}` : ""}
    ${peopleResults(q, chats)}
    <div class="section"><span>Messages${hits.length ? ` · ${hits.length}` : ""}</span><span class="note">Searched on this computer</span></div>
    ${hitRows || `<div class="empty">No messages with “${esc(q)}”.</div>`}
    ${hasMail() ? `<div class="section"><span>Email${state.mail.found.length ? ` · ${state.mail.found.length}` : ""}</span><span class="note">Searched on this computer</span></div>${state.mail.found.map(mailRow).join("") || `<div class="empty">No email with “${esc(q)}”.</div>`}` : ""}
  </div>`;
}

/** People, and channels you're not in, found on the server. */
function peopleResults(q, chats) {
  if (!state.live) return "";
  // Someone you already have a direct chat with shows under Chats.
  const talking = new Set(chats.filter((c) => c.kind === "direct").flatMap((c) => others(c).map((u) => u.id)));
  const people = state.people.filter((u) => !talking.has(u.id)).map((u) => `<button class="row compact" data-act="dm" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${marked(nameOf(u), q)}${u.app ? ` <span class="appbadge">APP</span>` : ""}</span><span class="last">@${esc(u.username)}</span></span></button>`).join("");
  const channels = state.found.filter((f) => !f.joined).map((f) => channelRow(f, marked(f.title, q))).join("");
  return `${people ? `<div class="section">People</div>${people}` : ""}
    ${channels ? `<div class="section">Channels you can follow</div>${channels}` : ""}`;
}

function personAvatar(u, size = "") {
  const name = nameOf(u);
  const [bg, fg] = colours(u.id + name);
  return `<span class="avatar ${size}" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(name))}${dot(u)}</span>`;
}

/** Opens the chat holding message `id`, with the find bar on its matches. */
async function jumpTo(id) {
  if (!findMsg(id)[1]) {
    // An older message: bring in it and everything after it from the store.
    const hit = state.kept.list.find((x) => x.id === id);
    if (!hit) return;
    const have = state.messages[hit.conv] || [];
    const upTo = have.length ? have[0].seq - 1 : (conv(hit.conv)?.last_seq ?? hit.seq + 50);
    const older = await tauri.core.invoke("messages_between", { conv: hit.conv, from: Math.max(1, hit.seq - 20), to: upTo });
    const byId = new Map([...older, ...have].map((x) => [x.id, x]));
    state.messages[hit.conv] = [...byId.values()].sort((a, b) => (a.pending ? 1 : 0) - (b.pending ? 1 : 0) || a.seq - b.seq);
  }
  const [cid, m] = findMsg(id);
  if (!m) return;
  const q = state.q.trim();
  open(cid, { keepSearch: true });
  const ids = (state.messages[cid] || []).filter((x) => !x.deleted && x.text.toLowerCase().includes(q.toLowerCase())).map((x) => x.id);
  state.find = { q, ids, at: Math.max(0, ids.indexOf(id)) };
  state.flash = id;
  render();
  scrollToMessage(id);
}

function scrollToMessage(id) {
  const el = document.querySelector(`[data-msg="${id}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}

// ---------- Drawing ----------

/** A direct chat with no messages yet shows only while you have it open;
 * the other person sees it when the first message arrives. */
const listed = (c) => c.kind !== "direct" || c.last || c.id === state.sel;

/** The "Later" reminder hiding something, if any. */
const snoozedFor = (match) => state.cal.snoozed.find((e) => {
  const l = e.link ? JSON.parse(e.link) : null;
  return l && match(l);
});

function inTab(c) {
  if (!listed(c) || state.tab === "mail" || state.tab === "later") return false;
  // Put off until later (a whole chat, not one message).
  if (snoozedFor((l) => l.type === "chat" && l.conv === c.id && !l.msg)) return false;
  if (state.tab === "chats") return c.kind !== "channel";
  if (state.tab === "channels") return c.kind === "channel";
  return true;
}

function sidebar() {
  const searching = state.q.trim().length > 0;
  const mailOn = !!tauri;
  const tabs = [["all", "All"], ["chats", "Chats"], ["channels", "Channels"], ...(mailOn ? [["mail", "Mail"], ["later", "Later"]] : [])].map(([id, label]) => {
    const newMail = (id === "all" || id === "mail") && state.mail.list.some((r) => !r.read);
    const fresh = newMail || state.conversations.some((c) => listed(c) && id !== "mail" && id !== "later" && (id === "all" || (id === "chats" ? c.kind !== "channel" : c.kind === "channel")) && unread(c) && !muted(c));
    return `<button class="tab" role="tab" aria-selected="${state.tab === id}" data-act="tab" data-id="${id}">${label}${fresh ? `<span class="dot" aria-hidden="true"></span><span class="sr">, something new</span>` : ""}</button>`;
  }).join("");
  const chatRows = state.conversations.filter(inTab).map((c) => ({ at: c.last?.at || 0, html: chatRow(c) }));
  const laterRows = state.tab === "later" ? state.cal.snoozed.map((e) => ({ at: -e.start, html: laterRow(e) })) : [];
  const mailRows = state.tab === "later" ? laterRows : state.tab === "mail" && state.mail.folder === "drafts"
    ? state.mail.drafts.map((d) => ({ at: d.saved, html: draftRow(d) }))
    : state.tab === "all" || state.tab === "mail"
      ? state.mail.list.filter((r) => !snoozedFor((l) => l.type === "mail" && l.account === r.account && l.folder === r.folder && l.uid === r.uid)).map((r) => ({ at: r.at, html: mailRow(r) }))
      : [];
  const rows = [...chatRows, ...mailRows].sort((a, b) => b.at - a.at).map((x) => x.html).join("");
  const st = STATUS[state.status];
  const listEmpty = state.tab === "later"
    ? `<div class="empty">Nothing put off. Use “Remind me…” on a chat or an email to put it here until later.</div>`
    : state.tab === "mail"
    ? mailEmpty()
    : state.tab === "channels"
      ? (state.browse.length ? "" : `<div class="empty">No channels on the server yet. Press New to start one.</div>`)
      : `<div class="empty">No chats yet. Search for someone to talk to, or press New to start a group.</div>`;
  return `<nav class="side" aria-label="Chats, channels and email">
    <div class="brand">${icons.flame}Xenify Chat${state.live && state.net !== "online" ? `<span class="netstate" role="status">${state.net === "connecting" ? "Connecting…" : "Offline · trying again"}</span>` : ""}</div>
    <div class="searchrow">
      <label class="search"><span class="sr">Search chats, email and messages</span>${icons.search}<input id="search" placeholder="Search everything" value="${esc(state.q)}" autocomplete="off">${searching ? `<button class="iconbtn tiny" data-act="clearsearch" aria-label="Clear search">${icons.close}</button>` : ""}</label>
      <button class="textbtn new" data-act="menu" data-id="new" aria-haspopup="menu" title="A new chat, group, channel or email">${icons.plus}New</button>
    </div>
    ${state.menu === "new" ? newMenu() : ""}
    ${searching ? searchResults() : `<div class="tabrow"><div class="tabs" role="tablist" aria-label="Show">${tabs}</div></div>${state.tab === "later" ? "" : todayPanel()}${state.tab === "mail" ? mailBar() : ""}<div class="list">${rows || listEmpty}${state.tab === "channels" ? browseList() : ""}</div>`}
    ${state.menu === "status" ? statusMenu() : ""}
    <div class="me">
      <button class="me-button" data-act="menu" data-id="status" aria-label="Your status: ${st.label}. Change it">
        <span class="avatar small" style="position:relative;background:#FDE6D8;color:#8A2C0A">${esc(initials(nameOf(state.me)))}<span class="presence" style="background:${st.dot}"></span></span>
        <span class="who"><b>${esc(nameOf(state.me))}</b><span>${st.short}</span></span>
      </button>
      <button class="iconbtn" data-act="settings" aria-label="Settings" aria-pressed="${state.view === "settings"}">${icons.gear}</button>
    </div>
  </nav>`;
}

function chatRow(c) {
    const isUnread = unread(c);
    const spoken = `${c.kind === "channel" ? "Channel " : ""}${titleOf(c)}${muted(c) ? ", muted" : ""}${isUnread ? ", new messages" : ""}`;
  return `<button class="row${isUnread ? " unread" : ""}" aria-current="${c.id === state.sel && state.view === "chat" && !state.mailSel && !state.compose}" aria-label="${esc(spoken)}" data-act="open" data-id="${c.id}" data-ctx="conv">
      ${avatarFor(c)}
      <span class="body">
        <span class="top"><span class="flags"><span class="name">${esc(titleOf(c))}</span>${muted(c) ? `<span style="color:var(--muted);display:flex">${icons.bellOff(14)}</span>` : ""}</span><span class="when">${c.last ? esc(timeOf(c.last.at)) : ""}</span></span>
        <span class="bottom"><span class="last">${esc(lastLine(c))}</span></span>
      </span>
    </button>`;
}


/** A channel you don't follow: click it to look inside, or Follow. */
function channelRow(f, title) {
  return `<div class="row compact${state.preview?.id === f.id && state.sel === f.id ? " previewing" : ""}">
    <button class="rowmain" data-act="preview" data-id="${f.id}" aria-label="Look at ${esc(f.title)}">${avatarFor({ id: f.id, kind: f.kind, title: f.title, members: [] }, "small")}<span class="body"><span class="name">${title}</span><span class="last">${f.member_count} following</span></span></button>
    <button class="small" data-act="subscribe" data-id="${f.id}">Follow</button>
  </div>`;
}

/** Shows a public channel you don't follow, read-only, so you can decide. */
function preview(f) {
  state.preview = {
    id: f.id, kind: "channel", title: f.title, members: [], member_count: f.member_count, owners: [], role: "member",
    can_post: false, open: true, last: null, last_seq: 0, read_seq: 0, notify: "all", muted_until: null, preview: true,
  };
  delete state.messages[f.id];
  open(f.id, { keepSearch: true });
}

/** Public channels you don't follow yet, under your own in the Channels tab. */
function browseList() {
  if (!state.browse.length) return "";
  return `<div class="section">More channels to follow</div>${state.browse.map((f) => channelRow(f, esc(f.title))).join("")}`;
}

function newMenu() {
  return `<div class="menu newmenu" role="menu" aria-label="New" style="right:0;top:104px">
    <button role="menuitem" data-act="newchat" data-id="group"><span style="display:flex;flex-direction:column">New group chat<span class="hintline">Talk with several people; anyone in it can add others</span></span></button>
    <button role="menuitem" data-act="newchat" data-id="channel"><span style="display:flex;flex-direction:column">New channel<span class="hintline">Announcements and alerts that people follow</span></span></button>
    ${hasMail() ? `<button role="menuitem" data-act="newmail"><span style="display:flex;flex-direction:column">New email<span class="hintline">${several() ? "From any of your accounts" : `From ${esc(state.mail.accounts[0].email)}`}</span></span></button>` : ""}
    <button role="menuitem" data-act="focussearch"><span style="display:flex;flex-direction:column">Message someone<span class="hintline">Search for them by name</span></span></button>
  </div>`;
}

function statusMenu() {
  const items = Object.entries(STATUS).map(([id, s]) => `<button role="menuitemradio" aria-checked="${state.status === id}" data-act="status" data-id="${id}">
    <span style="display:flex;align-items:center;gap:10px"><span style="width:10px;height:10px;border-radius:50%;background:${s.dot}"></span><span style="display:flex;flex-direction:column"><span style="font-weight:600">${s.label}</span><span class="hintline">${s.hint}</span></span></span>
  </button>`).join("");
  return `<div class="menu" role="menu" aria-label="Your status" style="left:4px;right:4px;width:auto;bottom:60px">${items}</div>`;
}

function bubble(m, findQ) {
  const pending = m.pending ? " pending" : "";
  const flash = state.flash === m.id ? " flash" : "";
  if (m.deleted) return `<div class="bubble deleted" data-msg="${m.id}">This message was deleted.</div>`;
  if (state.editing === m.id) {
    const text = state.editText ?? m.text;
    const key = `edit:${m.id}`;
    const n = issuesFor(key, text).length;
    return `<div class="editbox" data-msg="${m.id}">
      <label class="sr" for="edit">Edit your message</label>
      <div class="fieldwrap">
        <div class="backdrop" id="editbackdrop" aria-hidden="true">${underlined(text, issuesFor(key, text))}</div>
        <textarea id="edit" rows="2">${esc(text)}</textarea>
      </div>
      <div class="editbar"><span class="hintline" id="editcheck">${n ? `<span class="checkcount">${n === 1 ? "1 thing" : `${n} things`} to look at</span> · ` : ""}Ctrl+Enter to save · Esc to cancel</span><span style="display:flex;gap:6px"><button class="small" data-act="canceledit">Cancel</button><button class="small primary" data-act="saveedit">Save</button></span></div>
    </div>`;
  }
  const picking = state.picker === m.id;
  return `<div class="msg">
    <div class="bubblewrap" data-msg="${m.id}" data-ctx="msg">
      <div class="bubble${pending}${flash}">${marked(m.text, findQ)}${m.edited && !m.sender.app ? `<span class="edited"> · edited</span>` : ""}</div>
      <span class="msgtools${picking ? " open" : ""}">
        <button class="msgmore" data-act="picker" data-id="${m.id}" aria-label="React" aria-expanded="${picking}">${icons.smile}</button>
        <button class="msgmore" data-act="msgmenu" data-id="${m.id}" aria-label="Message options">${icons.more(16)}</button>
      </span>
      ${picking ? reactionPicker(m.id) : ""}
    </div>
    ${reactionRow(m)}
  </div>`;
}

function messageList(c) {
  const msgs = state.messages[c.id] || [];
  const line = state.readLine[c.id];
  const findQ = state.find?.q || "";
  const out = [];
  let last = null;
  let day = null;
  msgs.forEach((m, i) => {
    const d = dayOf(m.at);
    if (d !== day) {
      out.push(`<div class="divider">${esc(d)}</div>`);
      day = d;
      last = null;
    }
    // (Not above the very first message: everything is new then.)
    if (line != null && m.seq > line && i > 0 && msgs[i - 1].seq <= line) {
      out.push(`<div class="divider new" role="separator">New since you last read</div>`);
      last = null;
    }
    const mine = m.sender.id === state.me.id;
    if (last && last.sender === m.sender.id) {
      last.el.push(bubble(m, findQ));
      return;
    }
    const group = { sender: m.sender.id, mine, head: m, el: [bubble(m, findQ)] };
    out.push(group);
    last = group;
  });
  if (!msgs.length) out.push(`<div class="empty" style="margin:auto">No messages yet. Say hello.</div>`);
  return out.map((g) => {
    if (typeof g === "string") return g;
    const t = esc(timeOf(g.head.at));
    if (g.mine) return `<div class="mine"><span class="meta"><span class="time">${t}</span></span>${g.el.join("")}</div>`;
    const name = senderName(c, g.head.sender);
    const [bg, fg] = colours(g.head.sender.id + name);
    return `<div class="theirs"><span class="avatar small" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(name))}</span>
      <div class="stack"><div class="meta"><b>${esc(name)}</b>${g.head.sender.app ? `<span class="appbadge" title="Posted by an app, not a person">APP</span>` : ""}<span class="time">${t}</span></div>${g.el.join("")}</div></div>`;
  }).join("");
}

function chatSheet() {
  const c = current();
  if (!c) return `<main class="sheet"><div class="empty" style="margin:auto">Pick a chat on the left.</div></main>`;
  const level = notifyLevel(c);
  const short = NOTIFY.find((n) => n.id === level).short;
  const isMuted = muted(c);
  const notifyMenu = state.menu === "notify" ? `<div class="menu" role="menu" aria-label="Notifications for this chat" style="right:16px;top:66px">
      <div class="label">Notify me about ${esc(titleOf(c))}</div>
      ${NOTIFY.map((n) => `<button role="menuitemradio" aria-checked="${n.id === level}" data-act="notify" data-id="${n.id}">${n.label}${n.id === level ? icons.check("var(--accent)") : ""}</button>`).join("")}
    </div>` : "";
  const moreMenu = state.menu === "more" ? `<div class="menu" role="menu" aria-label="${esc(titleOf(c))}" style="right:16px;top:66px">${convItems(c).map(menuItem).join("")}</div>` : "";
  const f = state.find;
  const findBar = f ? `<div class="findbar" role="search">
      <span>“${esc(f.q)}” · ${f.ids.length ? `${f.at + 1} of ${f.ids.length} in this chat` : "not in this chat"}</span>
      <span style="display:flex;gap:2px">
        <button class="iconbtn tiny" data-act="findprev" aria-label="Previous match" ${f.ids.length > 1 ? "" : "disabled"}>${icons.up}</button>
        <button class="iconbtn tiny" data-act="findnext" aria-label="Next match" ${f.ids.length > 1 ? "" : "disabled"}>${icons.down}</button>
        <button class="iconbtn tiny" data-act="findclose" aria-label="Close the search in this chat">${icons.close}</button>
      </span>
    </div>` : "";
  const draft = state.drafts[c.id] || "";
  const composer = c.preview ? `<div class="followbar"><span>You're looking at <b>${esc(titleOf(c))}</b> without following it. Follow it to get its posts.</span><button class="small primary" data-act="subscribe" data-id="${c.id}">Follow</button></div>` : c.can_post ? `<div class="composer">
      <div class="box">
        <label class="sr" for="draft">Message</label>
        <div class="fieldwrap">
          <div class="backdrop" id="backdrop" aria-hidden="true">${underlined(draft, issuesFor(c.id, draft))}</div>
          <textarea id="draft" rows="1" placeholder="Write to ${esc(titleOf(c))}" aria-describedby="checkstate">${esc(draft)}</textarea>
        </div>
        <div class="tools">
          <div class="left"></div>
          <button class="send" data-act="send" title="Send (Ctrl+Enter)" ${draft.trim() ? "" : "disabled"}>Send ${icons.send}</button>
        </div>
      </div>
      <div class="hint" id="checkstate">${typingLine(c) || checkLine(c.id, draft)}</div>
    </div>` : `<div class="hint" style="padding:12px 24px 18px;border-top:1px solid var(--line)">Only the channel's owners post here.</div>`;
  return `<main class="sheet">
    <header class="sheet-head">
      <div class="title">${avatarFor(c, "big")}<div><h1>${esc(titleOf(c))}</h1><div class="sub">${esc(subOf(c))}</div></div></div>
      ${c.preview ? "" : `<div class="actions">
        <button class="textbtn" data-act="menu" data-id="notify" aria-label="Notifications for this chat: ${short}" style="${isMuted ? "background:var(--tint)" : ""}">${isMuted ? icons.bellOff() : icons.bell}${short}</button>
        ${c.kind === "direct" ? "" : `<button class="iconbtn" data-act="menu" data-id="people" aria-label="${c.kind === "channel" ? "About this channel" : "People in this chat"}">${icons.people}</button>`}
        <button class="iconbtn" data-act="menu" data-id="more" aria-label="More options">${icons.more()}</button>
      </div>`}
    </header>
    ${askPrivatelyBar(c)}${findBar}${notifyMenu}${moreMenu}${state.menu === "people" ? peopleMenu(c) : ""}
    <div class="messages" id="messages">${messageList(c)}</div>
    ${composer}
  </main>`;
}

function typingLine(c) {
  const t = state.typing[c.id];
  return t && t.until > Date.now() ? `<span class="typing">${esc(nameOf(t.user).split(" ")[0])} is typing…</span>` : "";
}

function personRow(u) {
  return `<div class="person">${personAvatar(u, "small")}<span class="body"><b>${esc(u.id === state.me.id ? nameOf(u) + " (you)" : nameOf(u))}${u.app ? `<span class="appbadge">APP</span>` : ""}</b><span class="hintline">@${esc(u.username)}</span></span></div>`;
}

function peopleMenu(c) {
  if (c.kind === "channel") {
    // A channel has followers, not people in it: say how many, and who posts.
    const posters = c.members.filter((u) => c.owners?.includes(u.id) || u.app);
    const invite = c.role === "owner";
    return `<div class="menu" role="dialog" aria-label="About ${esc(titleOf(c))}" style="right:60px;top:66px;width:300px">
      <div class="label">${c.open ? "Public channel: anyone on the server can follow it" : "Private channel: only people invited can follow it"}</div>
      <div class="person"><span class="body"><b>${c.member_count} following</b></span></div>
      <div class="label">Posts from</div>
      <div class="people">${posters.map(personRow).join("")}</div>
      ${invite ? `<button role="menuitem" class="withicon" data-act="addpeople">${icons.plus}${c.open ? "Add an app or people…" : "Invite people or apps…"}</button>` : ""}
    </div>`;
  }
  return `<div class="menu" role="dialog" aria-label="People in ${esc(titleOf(c))}" style="right:60px;top:66px;width:300px">
    <div class="label">In this chat</div>
    <div class="people">${c.members.map(personRow).join("")}</div>
    <button role="menuitem" class="withicon" data-act="addpeople">${icons.plus}Add people…</button>
  </div>`;
}

// ---------- Right-click menus ----------

function convItems(c) {
  const items = [];
  if (unread(c)) items.push({ label: "Mark as read", run: () => markRead(c) });
  else if (c.last_seq) items.push({ label: "Mark as unread", hint: "From the last message", run: () => markUnread(c, c.last_seq - 1) });
  if (tauri) items.push({ label: "Remind me about this chat…", run: () => remindAbout(chatAbout(c, null)) });
  items.push({ label: "Notification settings", hint: NOTIFY.find((n) => n.id === notifyLevel(c)).short, run: () => ((state.sel = c.id), (state.view = "chat"), (state.menu = "notify")) });
  if (c.kind !== "direct") {
    const word = c.kind === "group" ? "Leave group" : "Unfollow";
    items.push({
      label: word,
      danger: true,
      run: () => {
        state.confirm = {
          title: `${word === "Leave group" ? "Leave" : "Unfollow"} ${titleOf(c)}?`,
          text: c.kind === "group" ? "You won't get its messages any more. Someone in the group can add you back." : c.open ? "Its posts will stop appearing here. It stays in the Channels tab if you want to follow it again." : "Its posts will stop appearing here. As it's private, an owner would have to invite you back.",
          action: word,
          run: () => {
            tell({ t: "leave", conv: c.id });
            removeConv(c.id);
          },
        };
      },
    });
  }
  return items;
}

function msgItems(m) {
  const [cid] = findMsg(m.id);
  const c = conv(cid);
  const items = [];
  if (!m.deleted && !m.pending) items.push({ reactions: m.id });
  items.push({ label: "Copy text", run: () => navigator.clipboard?.writeText(m.text) });
  if (tauri && !m.deleted && !m.pending) items.push({ label: "Remind me…", hint: "Later today, tomorrow, next week…", run: () => remindAbout(chatAbout(c, m)) });
  items.push({
    label: "Mark unread from here",
    hint: "The new line goes above this message",
    run: () => markUnread(c, m.seq - 1),
  });
  if (m.sender.id === state.me.id && !m.deleted && !m.pending) {
    items.push({
      label: "Edit",
      run: () => {
        state.editing = m.id;
        state.editText = m.text;
        scheduleCheck(0);
      },
    });
    items.push({
      label: "Delete",
      danger: true,
      run: () => {
        state.confirm = {
          title: "Delete this message?",
          text: "Everyone in the chat will see “This message was deleted.” instead.",
          action: "Delete",
          run: () => {
            tell({ t: "delete", conv: m.conv, seq: m.seq });
            Object.assign(m, { deleted: true, text: "", reactions: [] });
          },
        };
      },
    });
  }
  return items;
}

let menuRuns = [];
function menuItem(it) {
  if (it.note) return `<div class="label note">${esc(it.note)}</div>`;
  if (it.reactions) return reactionPicker(it.reactions).replace('class="picker"', 'class="picker inmenu"');
  menuRuns.push(it.run);
  return `<button role="menuitem" class="${it.danger ? "danger" : it.strong ? "strong" : ""}" data-act="run" data-id="${menuRuns.length - 1}"><span style="display:flex;flex-direction:column">${esc(it.label)}${it.hint ? `<span class="hintline">${esc(it.hint)}</span>` : ""}</span></button>`;
}

function ctxMenu() {
  if (!state.ctx) return "";
  const { x, y, items } = state.ctx;
  const left = Math.min(x, window.innerWidth - 270);
  const top = Math.min(y, window.innerHeight - 60 - items.length * 48);
  return `<div class="menu" role="menu" style="position:fixed;left:${left}px;top:${top}px;width:260px">${items.map(menuItem).join("")}</div>`;
}

function confirmBox() {
  const k = state.confirm;
  if (!k) return "";
  return `<div class="scrim"><div class="dialog" role="alertdialog" aria-label="${esc(k.title)}">
    <h2>${esc(k.title)}</h2><p style="white-space:pre-line">${esc(k.text)}</p>
    <div class="buttons"><button class="small" data-act="cancelconfirm">${esc(k.cancel || "Cancel")}</button><button class="small ${k.primary ? "primary" : "danger"}" data-act="doconfirm">${esc(k.action)}</button></div>
  </div></div>`;
}

// ---------- Settings ----------

function toggle(key, label, hint) {
  const on = !!state.settings[key];
  return `<div class="setting"><div><div class="label">${label}</div><div class="hintline">${hint}</div></div>
    <button class="switch" role="switch" aria-checked="${on}" aria-label="${esc(label)}" data-act="flip" data-id="${key}"><span></span></button></div>`;
}

function settingsPage() {
  const sections = [["account", "Account"], ...(tauri ? [["email", "Email"]] : []), ...(state.live ? [["apps", "Apps"]] : []), ["writing", "Spelling and grammar"], ["notifications", "Notifications"], ["appearance", "Appearance"]];
  const nav = `<nav class="side" aria-label="Settings sections">
    <button class="textbtn" data-act="back" style="margin:6px 0 10px">${icons.back}Back to chats</button>
    <div class="section" style="padding:0 10px 6px">Settings</div>
    ${sections.map(([id, label]) => `<button class="tab wide" aria-current="${state.section === id ? "page" : "false"}" aria-selected="${state.section === id}" data-act="section" data-id="${id}">${label}</button>`).join("")}
  </nav>`;
  let body = "";
  if (state.section === "account") {
    body = `<h1>Account</h1><p class="lede">Who you are and which server you chat on.</p>
      <div class="setting"><div style="display:flex;align-items:center;gap:14px"><span class="avatar" style="width:52px;height:52px;background:#FDE6D8;color:#8A2C0A;font-size:18px">${esc(initials(nameOf(state.me)))}</span><div><div class="label">${esc(nameOf(state.me))}</div><div class="hintline">@${esc(state.me.username)}</div></div></div>${state.live ? `<button class="small" data-act="changename">Change name</button>` : ""}</div>
      <div class="setting"><div><div class="label">Server</div><div class="hintline">${esc(state.server)} · remembered for next time</div></div>${state.live ? `<button class="small" data-act="changeserver">Change server</button>` : ""}</div>
      ${state.live ? `<div style="margin-top:20px"><button class="small danger-outline" data-act="signout">Sign out</button></div>` : `<p class="hintline" style="margin-top:20px">These are sample chats. Start Xenify Chat without XENIFY_SAMPLE to sign in.</p>`}`;
  } else if (state.section === "email") {
    body = emailSettings();
  } else if (state.section === "apps") {
    body = appsSettings();
  } else if (state.section === "writing") {
    body = `<h1>Spelling and grammar</h1><p class="lede">Messages are checked as you type, on this computer. Nothing you write is sent anywhere to be checked.</p>
      <div class="panel"><div><div class="label">Checking runs on this computer</div><div class="hintline">The same checker and dictionary as Xenify Write. The AI mistake spotter is shared by all your Xenify apps.</div></div></div>
      <h2>What to check</h2>
      ${toggle("spelling", "Spelling", "Underline words that aren't in the dictionary")}
      ${toggle("grammar", "Grammar and style", "Underline grammar slips, repeated words and unclear phrasing")}
      ${toggle("spotter", "Extra mistake spotter", spotterHint())}
      ${toggle("before", "Don't send until it's checked", "If anything is still underlined, or still being checked, Send shows it first: fix it or send anyway")}
      <h2>Your dictionary</h2><p class="hintline" style="margin:0 0 10px">Words you've added are never marked as mistakes, here or in Xenify Write.</p>
      <div class="words">${state.words.length ? "" : `<span class="hintline">No words added yet.</span>`}${state.words.map((w, i) => `<span class="word">${esc(w)}<button class="iconbtn tiny" data-act="unword" data-id="${i}" aria-label="Remove ${esc(w)}">${icons.close}</button></span>`).join("")}</div>
      <form class="addword" data-act="addword"><label class="sr" for="newword">Add a word</label><input id="newword" placeholder="Add a word" autocomplete="off"><button class="small primary" type="submit">Add</button></form>
      ${state.credits ? `<p class="hintline" style="margin-top:32px">${esc(state.credits)}</p>` : ""}`;
  } else if (state.section === "notifications") {
    body = `<h1>Notifications</h1><p class="lede">How Xenify Chat gets your attention. Each chat can also be muted from the bell at its top right.</p>
      ${toggle("desktop", "Desktop notifications", "Show new messages from chats that aren't muted")}
      ${toggle("preview", "Show the message in notifications", "Off: only who it's from, so nobody can read over your shoulder")}
      ${toggle("sound", "Play a sound", "A short chime for new messages")}
      ${toggle("mentions", "Always tell me when I'm mentioned", "Even in muted chats")}
      <div class="setting"><div><label class="label" for="quiet">Quiet hours</label><div class="hintline">Do not disturb turns itself on at these times</div></div>
        <select id="quiet">${["Off", "Evenings and weekends", "18:00 to 08:00", "Weekends only"].map((o) => `<option ${state.settings.quiet === o ? "selected" : ""}>${o}</option>`).join("")}</select></div>
      <h2>Muted chats</h2>
      ${state.conversations.filter(muted).map((c) => `<div class="setting">${avatarFor(c, "small")}<div style="flex:1"><div class="label">${esc(titleOf(c))}</div><div class="hintline">${NOTIFY.find((n) => n.id === notifyLevel(c)).short}</div></div><button class="small" data-act="unmute" data-id="${c.id}">Unmute</button></div>`).join("") || `<p class="hintline">No chats are muted.</p>`}`;
  } else {
    body = `<h1>Appearance</h1><p class="lede">Make Xenify Chat comfortable to look at.</p>
      <h2>Theme</h2>
      <div class="choices">${[["system", "Same as the computer"], ["light", "Light"], ["dark", "Dark"]].map(([id, l]) => `<label class="choice"><input type="radio" name="theme" value="${id}" ${state.settings.theme === id ? "checked" : ""}> ${l}</label>`).join("")}</div>
      <h2>Text size</h2>
      <div style="display:flex;align-items:center;gap:12px;max-width:420px"><label class="sr" for="size">Text size</label><input id="size" type="range" min="80" max="150" step="10" value="${state.settings.size}" style="flex:1"><span>${state.settings.size}%</span></div>`;
  }
  return nav + `<main class="sheet settings"><div class="inner">${body}</div></main>`;
}

// ---------- Sign in ----------

function signinPage() {
  const k = state.signin;
  return `<div class="signin">
    <section class="brandpanel" aria-hidden="true">
      <div class="bigmark">${icons.flame.replace('width="26" height="26"', 'width="56" height="56"').replace('stroke="var(--accent)"', 'stroke="#FFFFFF"')}</div>
      <h1>Xenify Chat</h1>
      <p>Chat with your team. Spelling and grammar are checked on your own computer, the same way as in Xenify Write.</p>
    </section>
    <main class="signform">
      <form data-act="signin" autocomplete="on">
        <h2>${k.join ? "Create your account" : "Sign in"}</h2>
        <p class="lede">${k.join ? "You've been invited. Choose how you'll sign in and how others will see you." : "Welcome back."}</p>
        <label class="field">User name<input id="su" value="${esc(k.username || "")}" autocomplete="username" autocapitalize="off" spellcheck="false" required></label>
        ${k.join ? `<label class="field">Your name<input id="sn" value="${esc(k.name || "")}" autocomplete="name" placeholder="As others will see it, e.g. Alex Lane"></label>` : ""}
        <label class="field">Password<input id="sp" type="password" autocomplete="${k.join ? "new-password" : "current-password"}" required>${k.join ? `<span class="hintline">At least 10 characters</span>` : ""}</label>
        ${k.join ? `<label class="field">Invite code or link<input id="si" value="${esc(k.invite)}" autocomplete="off" spellcheck="false" required></label>` : ""}
        ${k.error ? `<p class="formerror" role="alert">${esc(k.error)}</p>` : ""}
        <button class="big primary" type="submit" ${k.busy ? "disabled" : ""}>${k.busy ? (k.join ? "Creating your account…" : "Signing in…") : k.join ? "Create account" : "Sign in"}</button>
        <button class="linkbtn" type="button" data-act="signmode">${k.join ? "Already have an account? Sign in" : "Got an invite? Create your account"}</button>
      </form>
    </main>
  </div>`;
}

async function signIn() {
  const k = state.signin;
  const username = (k.username = document.getElementById("su").value);
  const password = document.getElementById("sp").value;
  const invite = k.join ? document.getElementById("si").value : "";
  const displayName = k.join ? (k.name = document.getElementById("sn").value) : "";
  k.busy = true;
  k.error = "";
  render();
  try {
    state.me = await tauri.core.invoke("sign_in", { username, password, invite, displayName });
    Object.assign(k, { busy: false, invite: "", join: false });
    state.view = "chat";
    state.net = "connecting";
  } catch (e) {
    Object.assign(k, { busy: false, error: String(e) });
  }
  render();
  if (state.view === "signin") document.getElementById(k.error.includes("assword") ? "sp" : "su")?.focus();
}

async function signOut() {
  await tauri.core.invoke("sign_out");
  toSignin("");
}

function toSignin(error) {
  Object.assign(state, { me: null, conversations: [], messages: {}, sel: null, view: "signin", menu: null, dialog: null, q: "" });
  state.signin = { join: false, error, busy: false, invite: "" };
  render();
  document.getElementById("su")?.focus();
}

// ---------- Dialogs: new group or channel, add people, change server ----------

function dialogBox() {
  const d = state.dialog;
  if (!d) return "";
  if (d.kind === "event") return eventDialog(d);
  if (d.kind === "app") return appDialog(d);
  if (d.kind === "remind") return remindDialog(d);
  if (d.kind === "name") {
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Change your name">
      <h2>Your name</h2>
      <p>The name others see in chats. Your user name (@${esc(state.me.username)}) stays the same.</p>
      <label class="field">Name<input id="dname" value="${esc(d.value)}" autocomplete="name" maxlength="64"></label>
      <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Save</button></div>
    </form></div>`;
  }
  if (d.kind === "server") {
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Change server">
      <h2>Change server</h2>
      <p>Only if your organisation has moved Xenify Chat to a new address. You'll need to sign in again.</p>
      <label class="field">Server address<input id="dserver" value="${esc(d.value)}" autocomplete="off" spellcheck="false" required></label>
      ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
      <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Change and sign out</button></div>
    </form></div>`;
  }
  const c = d.kind === "add" ? conv(d.conv) : null;
  const heading = d.kind === "group" ? "New group chat" : d.kind === "channel" ? "New channel" : `Add people to ${esc(titleOf(c))}`;
  const already = new Set([state.me.id, ...(c ? c.members.map((u) => u.id) : []), ...d.picked.map((u) => u.id)]);
  const results = d.q.trim() ? state.people.filter((u) => !already.has(u.id)) : [];
  const picker = d.kind === "channel" ? "" : `
      <label class="field">${d.kind === "add" ? "People" : "People to add"}<input id="pickq" value="${esc(d.q)}" placeholder="Search by name" autocomplete="off"></label>
      ${d.picked.length ? `<div class="words">${d.picked.map((u) => `<span class="word">${esc(nameOf(u))}<button type="button" class="iconbtn tiny" data-act="unpick" data-id="${u.id}" aria-label="Don't add ${esc(nameOf(u))}">${icons.close}</button></span>`).join("")}</div>` : ""}
      ${results.length ? `<div class="pickresults">${results.slice(0, 6).map((u) => `<button type="button" class="row compact" data-act="pick" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${esc(nameOf(u))}</span><span class="last">@${esc(u.username)}</span></span></button>`).join("")}</div>` : ""}`;
  const ready = d.kind === "add" ? d.picked.length > 0 : d.title.trim().length > 0;
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${heading}">
    <h2>${heading}</h2>
    ${d.kind === "add" ? "<p>They'll see the chat's earlier messages too.</p>" : `<label class="field">Name<input id="dtitle" value="${esc(d.title)}" placeholder="${d.kind === "group" ? "e.g. Design team" : "e.g. Build alerts"}" autocomplete="off" required></label>`}
    ${picker}
    ${d.kind === "channel" ? `<div class="setting" style="border:none;padding:4px 0"><div><div class="label">Anyone on the server can find and follow it</div><div class="hintline">Off: only people you add will see it</div></div><button type="button" class="switch" role="switch" aria-checked="${d.open}" aria-label="Anyone can follow" data-act="togglefield"><span></span></button></div>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" id="dcreate" type="submit" ${ready ? "" : "disabled"}>${d.kind === "add" ? "Add" : "Create"}</button></div>
  </form></div>`;
}

async function finishDialog() {
  const d = state.dialog;
  if (d.kind === "event") return saveEvent();
  if (d.kind === "app") return saveApp();
  if (d.kind === "name") {
    tell({ t: "set_name", display_name: document.getElementById("dname").value });
    state.dialog = null;
    return render();
  }
  if (d.kind === "server") {
    try {
      const server = await tauri.core.invoke("change_server", { server: document.getElementById("dserver").value });
      state.server = server.replace(/^https?:\/\//, "");
      state.dialog = null;
      toSignin("");
    } catch (e) {
      d.error = String(e);
      render();
    }
    return;
  }
  const title = d.title.trim();
  if (d.kind === "add") {
    if (!d.picked.length) return;
    tell({ t: "add_people", conv: d.conv, users: d.picked.map((u) => u.username) });
  } else {
    if (!title) return;
    state.wantOpen = { title };
    if (d.kind === "group") tell({ t: "create_group", title, members: d.picked.map((u) => u.username) });
    else tell({ t: "create_channel", title, open: !!d.open });
  }
  state.dialog = null;
  render();
}

// ---------- Talking to the server ----------

function tell(msg) {
  if (state.live) tauri.core.invoke("send", { msg }).catch((e) => showToast(`Couldn't send that to the server: ${e}`));
}

function markRead(c) {
  if (c.preview || c.read_seq >= c.last_seq) return;
  c.read_seq = c.last_seq;
  tell({ t: "read", conv: c.id, seq: c.last_seq });
}

function markUnread(c, seq) {
  c.read_seq = Math.max(0, seq);
  state.readLine[c.id] = c.read_seq;
  tell({ t: "unread", conv: c.id, seq: c.read_seq });
}

function removeConv(id) {
  state.conversations = state.conversations.filter((x) => x.id !== id);
  delete state.messages[id];
  if (state.sel === id) state.sel = state.conversations[0]?.id ?? null;
}

const byLatest = (a, b) => (b.last?.at || 0) - (a.last?.at || 0);

let findTimer = 0;
function findOnServer(q) {
  clearTimeout(findTimer);
  // Email is searched in its copy on this computer (no chat server needed).
  if (hasMail() && !state.dialog && q.trim()) {
    const want = q.trim();
    tauri.core.invoke("mail_search", { q: want }).then((list) => {
      if (state.q.trim() === want) {
        state.mail.found = list;
        render();
      }
    }).catch(() => {});
  }
  if (!state.live) return;
  // Messages are searched here, on this computer.
  if (!state.dialog && q.trim()) {
    const want = q.trim();
    tauri.core.invoke("search_messages", { q: want }).then((list) => {
      if (state.q.trim() === want) {
        state.kept = { q: want, list };
        render();
      }
    });
  }
  if (!q.trim()) {
    state.people = [];
    state.found = [];
    return;
  }
  findTimer = setTimeout(() => {
    tell({ t: "find_people", q: q.trim() });
    if (!state.dialog) tell({ t: "find_conversations", q: q.trim() });
  }, 200);
}

let lastTyping = 0;
function typing() {
  if (Date.now() - lastTyping < 3000 || !state.sel) return;
  lastTyping = Date.now();
  tell({ t: "typing", conv: state.sel });
}

function showToast(text) {
  state.toast = text;
  render();
  setTimeout(() => {
    if (state.toast === text) {
      state.toast = "";
      render();
    }
  }, 5000);
}

/** Keeps a chat that the server sent (new or changed). */
function upsertConv(c) {
  if (state.preview?.id === c.id) state.preview = null;
  const i = state.conversations.findIndex((x) => x.id === c.id);
  // Someone new to chat with: are they around?
  const unknown = c.kind === "channel" ? [] : c.members.filter((u) => u.id !== state.me.id && !(u.id in state.presence)).map((u) => u.id);
  if (unknown.length) tell({ t: "get_presence", users: unknown });
  if (i >= 0) state.conversations[i] = c;
  else state.conversations.push(c);
  state.conversations.sort(byLatest);
  const w = state.wantOpen;
  if (w && ((w.user && c.kind === "direct" && c.members.some((u) => u.id === w.user)) || (w.id && c.id === w.id) || (w.title && c.title === w.title && c.role === "owner"))) {
    state.wantOpen = null;
    open(c.id);
  }
}

function upsertMessage(msg) {
  const list = (state.messages[msg.conv] ||= []);
  const i = list.findIndex((x) => x.id === msg.id || (x.pending && msg.client_id && x.client_id === msg.client_id));
  const fresh = i < 0 && msg.sender.id !== state.me.id && !msg.deleted && msg.seq > (conv(msg.conv)?.last_seq ?? 0);
  if (i >= 0) list[i] = msg;
  else list.push(msg);
  if (fresh && conv(msg.conv)) maybeNotify(conv(msg.conv), msg);
  list.sort((a, b) => (a.pending ? 1 : 0) - (b.pending ? 1 : 0) || a.seq - b.seq);
  const c = conv(msg.conv);
  if (!c) return;
  if (msg.seq >= c.last_seq) Object.assign(c, { last: msg, last_seq: msg.seq });
  if (msg.sender.id === state.me.id) c.read_seq = Math.max(c.read_seq, msg.seq);
  else if (state.view === "chat" && state.sel === c.id && document.hasFocus()) markRead(c);
  if (state.typing[c.id]?.user.id === msg.sender.id) delete state.typing[c.id];
  state.conversations.sort(byLatest);
}

function onServer(m) {
  switch (m.t) {
    case "welcome":
      state.me = m.user;
      break;
    case "conversations":
      state.conversations = m.list.sort(byLatest);
      // Recent messages from every chat, so search finds them.
      for (const c of state.conversations) tell({ t: "history", conv: c.id, before: null, limit: 100 });
      // Start in the latest chat, as if you'd opened it.
      const first = state.conversations.find((c) => c.last || c.kind !== "direct");
      if (!conv(state.sel) && first && state.view === "chat") return open(first.id, { keepSearch: true });
      if (!conv(state.sel)) state.sel = null;
      break;
    case "conversation":
      upsertConv(m.conv);
      break;
    case "removed":
      removeConv(m.conv);
      break;
    case "message":
      upsertMessage(m.msg);
      break;
    case "history": {
      const have = state.messages[m.conv] || [];
      const pending = have.filter((x) => x.pending);
      const byId = new Map(have.filter((x) => !x.pending).map((x) => [x.id, x]));
      for (const x of m.messages) byId.set(x.id, x);
      state.messages[m.conv] = [...byId.values()].sort((a, b) => a.seq - b.seq).concat(pending.filter((p) => !m.messages.some((x) => x.client_id === p.client_id)));
      break;
    }
    case "typing":
      if (m.user.id === state.me?.id) return;
      state.typing[m.conv] = { user: m.user, until: Date.now() + 5000 };
      setTimeout(render, 5100);
      break;
    case "apps":
      if (m.conv) state.appsIn[m.conv] = m.list;
      else {
        state.apps.list = m.list;
        if (state.dialog?.kind === "app" && state.dialog.waiting) state.dialog = null;
      }
      break;
    case "app_key":
      state.apps.shown = { app: m.app, key: m.key, secret: m.secret };
      if (state.dialog?.kind === "app") state.dialog = null;
      tell({ t: "list_apps" });
      break;
    case "api_key":
      state.apps.shown = { key: m.key };
      break;
    case "presence":
      for (const p of m.list) state.presence[p.user] = p.state;
      break;
    case "read": {
      const c = conv(m.conv);
      if (c) c.read_seq = m.seq;
      break;
    }
    case "people":
      if (m.q === (state.dialog ? state.dialog.q : state.q).trim()) state.people = m.users;
      break;
    case "found":
      // An empty search is the list of every channel you could follow.
      if (m.q === "") state.browse = m.conversations.filter((f) => f.kind === "channel" && !f.joined);
      if (m.q === state.q.trim()) state.found = m.conversations;
      break;
    case "error":
      if (state.dialog?.kind === "app" && state.dialog.waiting) {
        Object.assign(state.dialog, { waiting: false, error: m.message.charAt(0).toUpperCase() + m.message.slice(1) + "." });
        break;
      }
      showToast(m.message.charAt(0).toUpperCase() + m.message.slice(1) + ".");
      return;
    default:
      return;
  }
  render();
}

function onNet(st) {
  if (st === "signed_out") return toSignin("You've been signed out. Please sign in again.");
  state.net = st;
  if (st === "online") {
    tell({ t: "list" });
    tell({ t: "find_conversations", q: "" });
    tell({ t: "set_status", status: STATUS_SHARED[state.status] });
    // Anything sent while the connection was down goes again (the server
    // ignores repeats).
    for (const list of Object.values(state.messages)) for (const x of list) if (x.pending) tell({ t: "send", conv: x.conv, client_id: x.client_id, text: x.text });
  }
  render();
}

async function boot() {
  let start = { sample: true, server: "https://chat.example.org", user: null, invite: "" };
  if (tauri) {
    await tauri.event.listen("to_app", (e) => onServer(e.payload));
    await tauri.event.listen("net", (e) => onNet(e.payload.state));
    // A notification was clicked: its chat, or (-1) the mail.
    await tauri.event.listen("open_chat", (e) => {
      if (e.payload === -1) {
        state.tab = "mail";
        return render();
      }
      if (e.payload === -2) return openCalendar();
      if (conv(e.payload)) open(e.payload);
    });
    try {
      const saved = await tauri.core.invoke("load_settings");
      Object.assign(state.settings, saved.settings || {});
      if (STATUS[saved.status]) state.status = saved.status;
    } catch {}
    start = await tauri.core.invoke("start");
  }
  state.server = start.server.replace(/^https?:\/\//, "");
  state.serverUrl = start.server;
  if (tauri) {
    loadProofing();
    await tauri.event.listen("mail", (e) => onMail(e.payload));
    initMail();
    loadToday();
    checkDue();
    setInterval(checkDue, 30_000);
    setInterval(loadToday, 60_000);
  }
  if (start.sample) {
    Object.assign(state, { me: sample.me, conversations: sample.conversations, messages: sample.messages, view: "chat" });
    return open(sample.conversations[0].id);
  }
  state.live = true;
  if (start.user) {
    Object.assign(state, { me: start.user, view: "chat", net: "connecting" });
    render();
  } else {
    state.signin.invite = start.invite;
    state.signin.join = !!start.invite;
    toSignin("");
  }
}

// ---------- Notifications ----------

let savedSettings = "";
function saveSettings() {
  if (!tauri || state.view === "loading") return;
  const now = JSON.stringify({ settings: state.settings, status: state.status });
  if (now === savedSettings) return;
  if (savedSettings) tauri.core.invoke("save_settings", { settings: JSON.parse(now) }).catch(() => {});
  savedSettings = now;
}

/** Whether `text` mentions you (@username, or your name). */
function mentionsMe(text) {
  const t = text.toLowerCase();
  const me = state.me;
  return t.includes("@" + me.username.toLowerCase()) || (!!me.display_name && t.includes(me.display_name.toLowerCase()));
}

/** Quiet hours, as chosen in Settings › Notifications. */
function quietNow(d = new Date()) {
  const h = d.getHours();
  const weekend = d.getDay() === 0 || d.getDay() === 6;
  const night = h >= 18 || h < 8;
  switch (state.settings.quiet) {
    case "Evenings and weekends": return weekend || night;
    case "18:00 to 08:00": return night;
    case "Weekends only": return weekend;
    default: return false;
  }
}

/** A desktop notification for message `m` in chat `c`, if it deserves one. */
function maybeNotify(c, m) {
  const st = state.settings;
  if (!tauri || !st.desktop || state.status === "dnd" || quietNow()) return;
  // Not for the chat you're looking at.
  if (document.hasFocus() && state.view === "chat" && state.sel === c.id) return;
  const mention = mentionsMe(m.text);
  const chatWants = !muted(c) && (c.notify === "all" || (c.notify === "mentions" && mention));
  if (!chatWants && !(mention && st.mentions)) return;
  const who = senderName(c, m.sender);
  const title = c.kind === "direct" ? who : `${who} in ${titleOf(c)}`;
  const body = st.preview ? m.text : mention ? "Mentioned you" : "New message";
  tauri.core.invoke("notify", { title, body, conv: c.id, sound: !!st.sound }).catch(() => {});
}

// ---------- Email ----------

/** "Ana Ruiz <ana@x.org>" → "Ana Ruiz" (or the address when there's no name). */
function fromName(from) {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]+)>/.exec(from || "");
  return (m && (m[1].trim() || m[2])) || (from || "").trim() || "(no sender)";
}

const hasMail = () => state.mail.accounts.length > 0;
const several = () => state.mail.accounts.length > 1;
/** The folder shown in the Mail tab: "" (every inbox), "drafts", or "account\nwire". */
const pickedFolder = () => {
  const f = state.mail.folder;
  if (!f || f === "drafts") return { account: "", wire: "" };
  const [account, wire] = f.split("\n");
  return { account, wire };
};

function mailEmpty() {
  if (!hasMail()) {
    return `<div class="empty mailempty"><p>Read and answer your email here, next to your chats.</p><button class="small primary" data-act="mailsetup">Add your email account</button></div>`;
  }
  if (state.mail.folder === "drafts") return `<div class="empty">No drafts. Emails you start and don't send are kept here.</div>`;
  return `<div class="empty">${esc(Object.values(state.mail.status).find(Boolean) || "No email in this folder.")}</div>`;
}

function mailBar() {
  if (!hasMail()) return "";
  const label = (x) => (x.role === "inbox" ? "Inbox" : x.name) + (x.unread ? ` (${x.unread})` : "");
  const sel = state.mail.folder || "";
  const groups = state.mail.accounts.map((a) => {
    const opts = (state.mail.folders[a.email] || []).map((x) => {
      const v = !several() && x.role === "inbox" ? "" : `${a.email}\n${x.wire}`;
      return `<option value="${esc(v)}" ${v === sel ? "selected" : ""}>${esc(label(x))}</option>`;
    }).join("");
    return several() ? `<optgroup label="${esc(a.email)}">${opts}</optgroup>` : opts;
  }).join("");
  const drafts = state.mail.drafts.length;
  const problem = Object.entries(state.mail.problems).find(([, p]) => p);
  const status = Object.values(state.mail.status).find(Boolean) || "";
  return `<div class="mailbar">
    <label class="sr" for="mailfolder">Folder</label>
    <select id="mailfolder">
      ${several() ? `<option value="" ${sel === "" ? "selected" : ""}>All inboxes</option>` : ""}
      <option value="drafts" ${sel === "drafts" ? "selected" : ""}>Drafts on this computer${drafts ? ` (${drafts})` : ""}</option>
      ${groups}
    </select>
    <span class="hintline">${problem ? `<span class="problem">${esc(several() ? `${problem[0]}: ${problem[1]}` : problem[1])}</span>` : esc(status)}</span>
  </div>`;
}

function mailRow(r) {
  const who = fromName(r.from);
  const [bg, fg] = colours(r.from_address + who);
  const s = state.mailSel;
  const current = s?.account === r.account && s?.folder === r.folder && s?.uid === r.uid && !state.compose;
  return `<button class="row mailrow${r.read ? "" : " unread"}" aria-current="${current}" aria-label="${esc(`Email from ${who}: ${r.subject}${r.read ? "" : ", unread"}`)}" data-act="mail" data-account="${esc(r.account)}" data-folder="${esc(r.folder)}" data-uid="${r.uid}" data-ctx="mail">
    <span class="avatar square" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(who))}<span class="mailbadge">${icons.mail}</span></span>
    <span class="body">
      <span class="top"><span class="flags"><span class="name">${esc(who)}</span></span><span class="when">${esc(timeOf(r.at))}</span></span>
      <span class="bottom"><span class="last">${esc(r.subject || "(no subject)")}</span>${several() ? `<span class="acct" title="${esc(r.account)}">${esc(r.account.split("@")[1] || r.account)}</span>` : ""}</span>
    </span>
  </button>`;
}

function draftRow(d) {
  return `<button class="row mailrow draftrow" data-act="opendraft" data-id="${esc(d.id)}" data-ctx="draft">
    <span class="avatar square" style="background:var(--tint);color:var(--ink-2)" aria-hidden="true">${icons.mail}</span>
    <span class="body">
      <span class="top"><span class="flags"><span class="name">${esc(d.to || "(no one yet)")}</span></span><span class="when">${esc(timeOf(d.saved))}</span></span>
      <span class="bottom"><span class="last">Draft · ${esc(d.subject || d.text.slice(0, 60) || "(empty)")}</span></span>
    </span>
  </button>`;
}

const sameMail = (r, account, folder, uid) => r.account === account && r.folder === folder && r.uid === uid;
const findRow = (account, folder, uid) => [...state.mail.list, ...state.mail.found].find((r) => sameMail(r, account, folder, uid));

function mailItems(account, folder, uid) {
  const r = findRow(account, folder, uid);
  const items = [];
  if (r) items.push({ label: r.read ? "Mark as unread" : "Mark as read", run: () => setRead(r, !r.read) });
  items.push({ label: "Reply", run: async () => { await openMail(account, folder, uid); if (state.mail.opened) startCompose(replyOf(state.mail.opened, false)); } });
  if (r) items.push({ label: "Remind me…", hint: "Later today, tomorrow, next week…", run: () => remindAbout(mailAbout(r)) });
  items.push({ label: "Delete", hint: "Moves it to Trash", danger: true, run: () => deleteMail(account, folder, uid) });
  return items;
}

function openedItems() {
  const o = state.mail.opened;
  if (!o) return [];
  const r = findRow(o.account, o.folder, o.uid);
  return [
    { label: "Mark as unread", run: () => { if (r) setRead(r, false); state.mailSel = null; } },
    ...(r ? [{ label: "Remind me…", hint: "Later today, tomorrow, next week…", run: () => remindAbout(mailAbout(r)) }] : []),
    { label: "Delete", hint: "Moves it to Trash", danger: true, run: () => deleteMail(o.account, o.folder, o.uid) },
  ];
}

function setRead(r, read) {
  r.read = read;
  tauri.core.invoke("mail_flag", { account: r.account, folder: r.folder, uid: r.uid, flag: "\\Seen", on: read });
}

function deleteMail(account, folder, uid) {
  tauri.core.invoke("mail_delete", { account, folder, uid });
  state.mail.list = state.mail.list.filter((r) => !sameMail(r, account, folder, uid));
  if (state.mailSel && sameMail(state.mailSel, account, folder, uid)) state.mailSel = null;
  showToast("Moved to Trash.");
}

async function openMail(account, folder, uid) {
  await parkCompose();
  Object.assign(state, { mailSel: { account, folder, uid }, compose: null, view: "chat", menu: null });
  state.mail.opened = null;
  const r = findRow(account, folder, uid);
  if (r) r.read = true;
  render();
  await loadOpened();
}

async function loadOpened() {
  const sel = state.mailSel;
  if (!sel) return;
  state.mail.loading = true;
  let o = null;
  try {
    o = await tauri.core.invoke("mail_open", { account: sel.account, folder: sel.folder, uid: sel.uid });
  } catch (e) {
    showToast(String(e));
  }
  if (state.mailSel !== sel) return;
  // Not downloaded yet: a "downloaded" event brings it.
  state.mail.opened = o;
  state.mail.loading = !o;
  render();
}

/** The email's own HTML, shown in a frame that runs no scripts. */
function mailDoc(o) {
  return `<!doctype html><html><head><meta charset="utf-8"><base target="_blank"><style>
    html { color-scheme: light; } body { margin: 0; padding: 14px 18px 18px; font: 15px/1.55 system-ui, sans-serif; color: #1E1C1A; background: #fff; overflow-wrap: break-word; }
    img { max-width: 100%; height: auto; } table { max-width: 100%; } a { color: #B23A0A; } blockquote { margin: 0 0 0 8px; padding-left: 12px; border-left: 3px solid #E5E0DA; color: #555; }
  </style></head><body>${o.html}</body></html>`;
}

/** Sizes the email's frame to its content, and sends its links to the browser. */
function fitMailFrame() {
  const f = document.getElementById("mailframe");
  if (!f) return;
  const fit = () => {
    const d = f.contentDocument;
    if (!d?.body) return;
    f.style.height = d.documentElement.scrollHeight + 8 + "px";
    if (!d.xenify) {
      d.xenify = true;
      d.addEventListener("click", (e) => {
        const a = e.target.closest?.("a[href]");
        if (!a) return;
        e.preventDefault();
        askOpenLink(a.getAttribute("href"));
      });
    }
  };
  f.addEventListener("load", fit);
  fit();
}

function askOpenLink(url) {
  if (!/^(https?:|mailto:)/i.test(url || "")) return;
  if (/^mailto:/i.test(url)) return startCompose({ to: decodeURIComponent(url.slice(7).split("?")[0]) });
  state.confirm = { title: "Open this link?", text: url, action: "Open in browser", primary: true, run: () => tauri.core.invoke("open_link", { url }) };
  render();
}

/** Plain text with web addresses as links (opened only after asking). */
function linkified(text) {
  return esc(text).replace(/\bhttps?:\/\/[^\s<>"']+/g, (u) => `<a role="link" tabindex="0" class="maillink" data-act="askopen" data-url="${u}">${u}</a>`);
}

function mailSheet() {
  const o = state.mail.opened;
  const s = state.mailSel;
  const r = s && findRow(s.account, s.folder, s.uid);
  if (!o) {
    return `<main class="sheet"><div class="empty" style="margin:auto">${state.mail.loading ? "Downloading this email…" : ""}${r ? `<br><b>${esc(r.subject)}</b>` : ""}</div></main>`;
  }
  const who = fromName(o.from);
  const [bg, fg] = colours(o.from_address + who);
  const size = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
  return `<main class="sheet mail">
    <header class="sheet-head">
      <div class="title"><span class="avatar big square" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(who))}</span><div><h1>${esc(o.subject || "(no subject)")}</h1><div class="sub">${esc(o.from)} · ${esc(o.date)}</div></div></div>
      <div class="actions">
        <button class="textbtn" data-act="reply">${icons.reply}Reply</button>
        <button class="textbtn" data-act="replyall">Reply all</button>
        <button class="textbtn" data-act="forward">${icons.forward}Forward</button>
        <button class="iconbtn" data-act="mailmenu" aria-label="More for this email">${icons.more()}</button>
      </div>
    </header>
    <div class="mailscroll">
      <div class="mailmeta"><span><b>To</b> ${esc(o.to || "you")}</span>${o.cc ? `<span><b>Cc</b> ${esc(o.cc)}</span>` : ""}${several() ? `<span><b>In</b> ${esc(o.account)}</span>` : ""}</div>
      ${o.invite ? inviteCard(o) : ""}
      ${o.blocked_images ? `<div class="mailnote">${o.blocked_images === 1 ? "A picture from the internet wasn't loaded" : `${o.blocked_images} pictures from the internet weren't loaded`}, so the sender can't tell you've read this.</div>` : ""}
      ${o.html ? `<iframe id="mailframe" class="mailframe" sandbox="allow-same-origin" title="The email" srcdoc="${esc(mailDoc(o))}"></iframe>` : `<div class="mailtext">${linkified(o.text)}</div>`}
      ${o.attachments.length ? `<div class="attachments"><div class="section">Attached</div>${o.attachments.map(([name, mime, n], i) => `<div class="attachment">${icons.clip}<span class="body"><b>${esc(name)}</b><span class="hintline">${esc(mime)} · ${size(n)}</span></span><button class="small" data-act="saveattachment" data-id="${i}">Save</button></div>`).join("")}</div>` : ""}
    </div>
  </main>`;
}

const myAddresses = () => state.mail.accounts.map((a) => a.email.toLowerCase());

function quoteOf(o) {
  return `\n\nOn ${o.date}, ${o.from} wrote:\n` + shownText(o).split("\n").map((l) => "> " + l).join("\n");
}

/** The email's words as shown (headings and paragraphs on their own lines). */
const shownText = (o) => document.getElementById("mailframe")?.contentDocument?.body?.innerText?.trim() || o.text;

function replyOf(o, all) {
  if (!o) return {};
  const subject = /^re:/i.test(o.subject) ? o.subject : `Re: ${o.subject}`;
  const mine = myAddresses();
  const others = all ? [o.to, o.cc].join(",").split(",").map((x) => x.trim()).filter((x) => x && !mine.some((m) => x.toLowerCase().includes(m))) : [];
  return { account: o.account, to: o.from, cc: others.join(", "), showCc: !!others.length, subject, quote: quoteOf(o), replyTo: [o.folder, o.uid], title: all ? "Reply all" : "Reply" };
}

function forwardOf(o) {
  if (!o) return {};
  const subject = /^(fwd?|fw):/i.test(o.subject) ? o.subject : `Fwd: ${o.subject}`;
  const head = `\n\n---------- Forwarded message ----------\nFrom: ${o.from}\nDate: ${o.date}\nSubject: ${o.subject}\nTo: ${o.to}\n\n`;
  return { account: o.account, subject, quote: head + shownText(o), title: "Forward" };
}

async function startCompose(c) {
  await parkCompose();
  const account = c.account || pickedFolder().account || state.mail.accounts[0]?.email || "";
  state.compose = { to: "", cc: "", subject: "", text: "", quote: "", replyTo: null, title: "New email", showCc: false, attachments: [], draftId: "", ...c, account };
  delete state.issues.compose;
  delete state.ignored.compose;
  state.view = "chat";
  state.menu = null;
  render();
  document.getElementById(state.compose.to ? "mailbody" : "mto")?.focus();
}

const composeHasText = () =>
  !!state.compose && !!(state.compose.text.trim() || state.compose.attachments.length || (!state.compose.replyTo && (state.compose.to.trim() || state.compose.subject.trim())));

// Drafts: kept on this computer as you write, and when you leave the email.

function draftOf(k) {
  return { id: k.draftId || "", account: k.account, title: k.title, to: k.to, cc: k.cc, subject: k.subject, text: k.text, quote: k.quote || "", reply_to: k.replyTo, attachments: k.attachments.map(({ name, mime, data }) => ({ name, mime, data })), saved: 0 };
}

async function saveDraft() {
  const k = state.compose;
  if (!k || !composeHasText() || k.busy) return;
  try {
    k.draftId = await tauri.core.invoke("mail_draft_save", { draft: draftOf(k) });
    k.savedAt = Date.now();
    loadDrafts();
  } catch (e) {
    k.error = String(e);
  }
}

let draftTimer = 0;
function scheduleDraft() {
  clearTimeout(draftTimer);
  draftTimer = setTimeout(async () => {
    await saveDraft();
    const line = document.getElementById("draftstate");
    if (line && state.compose?.savedAt) line.textContent = `Draft saved ${hm(state.compose.savedAt)}`;
  }, 1500);
}

/** Leaving an email being written keeps it in Drafts. */
async function parkCompose() {
  if (!state.compose) return;
  clearTimeout(draftTimer);
  if (composeHasText()) {
    await saveDraft();
    showToast("Kept in Drafts.");
  }
  state.compose = null;
}

async function loadDrafts() {
  try {
    state.mail.drafts = await tauri.core.invoke("mail_drafts");
  } catch {
    return;
  }
  render();
}

function openDraft(id) {
  const d = state.mail.drafts.find((x) => x.id === id);
  if (!d) return;
  startCompose({ account: d.account, to: d.to, cc: d.cc, showCc: !!d.cc, subject: d.subject, text: d.text, quote: d.quote, replyTo: d.reply_to, title: d.title || "Draft", attachments: d.attachments.map((a) => ({ ...a, size: Math.floor((a.data.length * 3) / 4) })), draftId: d.id });
}

function deleteDraft(id) {
  tauri.core.invoke("mail_draft_delete", { id }).then(loadDrafts);
}

function discardCompose() {
  const k = state.compose;
  const gone = () => {
    if (k.draftId) deleteDraft(k.draftId);
    clearTimeout(draftTimer);
    state.compose = null;
  };
  if (composeHasText()) {
    state.confirm = { title: "Delete this email?", text: "It won't be kept in Drafts.", action: "Delete", cancel: "Keep writing", run: gone };
    return render();
  }
  gone();
  render();
}

// Attaching files.

const MAX_ATTACHED = 25 * 1024 * 1024;
const fileSize = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

async function attachFiles(files) {
  const k = state.compose;
  if (!k) return;
  let total = k.attachments.reduce((t, a) => t + a.size, 0);
  for (const f of files) {
    if (total + f.size > MAX_ATTACHED) {
      showToast(`${f.name} wasn't attached: attachments can be 25 MB in all.`);
      continue;
    }
    const data = await new Promise((done, fail) => {
      const r = new FileReader();
      r.onload = () => done(String(r.result).split(",")[1] || "");
      r.onerror = fail;
      r.readAsDataURL(f);
    });
    total += f.size;
    k.attachments.push({ name: f.name, mime: f.type, data, size: f.size });
  }
  render();
  scheduleDraft();
}

function composeSheet() {
  const k = state.compose;
  const text = k.text;
  const from = state.mail.accounts.find((a) => a.email === k.account) || state.mail.accounts[0] || { name: "", email: "" };
  return `<main class="sheet compose">
    <header class="sheet-head">
      <div class="title"><span class="avatar big square" style="background:var(--tint);color:var(--ink-2)" aria-hidden="true">${icons.mail}</span><div><h1>${esc(k.title)}</h1><div class="sub" id="draftstate">${k.savedAt ? `Draft saved ${hm(k.savedAt)}` : "Kept in Drafts as you write"}</div></div></div>
      <div class="actions">
        <button class="textbtn" data-act="discard">Delete</button>
        <button class="send" data-act="sendmail" title="Send (Ctrl+Enter)" ${k.busy ? "disabled" : ""}>${k.busy ? "Sending…" : "Send"} ${icons.send}</button>
      </div>
    </header>
    <div class="mailscroll">
      <div class="mailfields">
        <label class="mfield"><span>From</span>${several() ? `<select id="mfrom">${state.mail.accounts.map((a) => `<option value="${esc(a.email)}" ${a.email === k.account ? "selected" : ""}>${esc(a.name)} &lt;${esc(a.email)}&gt;</option>`).join("")}</select>` : `<span class="fromline">${esc(from.name)} &lt;${esc(from.email)}&gt;</span>`}</label>
        <label class="mfield"><span>To</span><input id="mto" value="${esc(k.to)}" autocomplete="off" spellcheck="false" placeholder="name@example.com, …">${k.showCc ? "" : `<button class="linkbtn" data-act="showcc" type="button">Cc</button>`}</label>
        ${k.showCc ? `<label class="mfield"><span>Cc</span><input id="mcc" value="${esc(k.cc)}" autocomplete="off" spellcheck="false"></label>` : ""}
        <label class="mfield"><span>Subject</span><input id="msubject" value="${esc(k.subject)}" autocomplete="off"></label>
      </div>
      ${k.error ? `<p class="formerror" role="alert" style="margin:0 24px 8px">${esc(k.error)}</p>` : ""}
      <div class="box mailbox">
        <div class="fieldwrap">
          <div class="backdrop" id="mailbackdrop" aria-hidden="true">${underlined(text, issuesFor("compose", text))}</div>
          <textarea id="mailbody" rows="8" aria-label="Your email" aria-describedby="mailcheck" placeholder="Write your email">${esc(text)}</textarea>
        </div>
        <div class="tools">
          <div class="left">
            <label class="textbtn attachbtn">${icons.clip}Attach files<input id="mfile" type="file" multiple class="sr"></label>
            ${k.attachments.map((a, i) => `<span class="word attached">${icons.clip.replace('width="20" height="20"', 'width="14" height="14"')}${esc(a.name)} <span class="hintline">${fileSize(a.size)}</span><button type="button" class="iconbtn tiny" data-act="unattach" data-id="${i}" aria-label="Remove ${esc(a.name)}">${icons.close}</button></span>`).join("")}
          </div>
        </div>
      </div>
      <div class="hint" id="mailcheck" style="padding:6px 28px">${checkLine("compose", text)}</div>
      ${k.quote ? `<div class="mailquote">${esc(k.quote.trim())}</div>` : ""}
    </div>
  </main>`;
}

async function sendMail(anyway = false) {
  const k = state.compose;
  if (!k || k.busy) return;
  if (!k.to.trim() && !k.cc.trim()) {
    k.error = "Add at least one address in To.";
    render();
    return document.getElementById("mto")?.focus();
  }
  if (state.settings.before && !anyway) {
    await runCheck();
    const left = issuesFor("compose", k.text);
    if (left.length) {
      state.confirm = {
        title: left.length === 1 ? "One thing to check" : `${left.length} things to check`,
        text: left.slice(0, 4).map((i) => `“${i.word}”: ${tidy(i.message)}`).join("\n") + (left.length > 4 ? "\n…" : ""),
        action: "Send anyway", cancel: "Fix it", primary: true, run: () => sendMail(true),
      };
      return render();
    }
  }
  clearTimeout(draftTimer);
  Object.assign(k, { busy: true, error: "" });
  render();
  try {
    await tauri.core.invoke("mail_send", {
      account: k.account, to: k.to, cc: k.cc, subject: k.subject, text: k.text + (k.quote || ""), replyTo: k.replyTo,
      attachments: k.attachments.map(({ name, mime, data }) => ({ name, mime, data })),
    });
    if (k.draftId) deleteDraft(k.draftId);
    state.compose = null;
    showToast("Sent.");
  } catch (e) {
    Object.assign(k, { busy: false, error: String(e) });
    render();
  }
}

// Accounts (Settings › Email).

function emailSettings() {
  const list = state.mail.accounts.map((a) => `<div class="setting"><div><div class="label">${esc(a.name)} &lt;${esc(a.email)}&gt;</div><div class="hintline">${state.mail.problems[a.email] ? `<span class="problem">${esc(state.mail.problems[a.email])}</span>` : esc(state.mail.status[a.email] || "Up to date")}</div><div class="hintline">${!a.kept ? "The password isn't saved on this computer: remove the account and add it again." : a.kept.startsWith("system") ? "Password in this computer's keyring." : "Password in a private file only you can read (no keyring here)."}</div></div><button class="small danger-outline" style="height:36px" data-act="mailremove" data-id="${esc(a.email)}">Remove</button></div>`).join("");
  const adding = !hasMail() || state.mailSetup;
  return `<h1>Email</h1><p class="lede">Your email sits next to your chats, checked for spelling and grammar like everything you write here. A copy is kept on this computer, so it opens instantly and can be searched offline.</p>
    ${list ? `${list}<div style="display:flex;gap:8px;margin:14px 0 6px"><button class="small" data-act="mailcheck">Check now</button>${adding ? "" : `<button class="small primary" data-act="mailaddmore">${icons.plus}Add another account</button>`}</div>` : ""}
    ${adding ? `${list ? "<h2>Add an account</h2>" : ""}${mailSetupForm()}` : ""}`;
}

const newSetup = () => ({ step: "address", email: "", name: state.me ? nameOf(state.me) : "", password: "", user: "", imap: { host: "", port: 993, security: "tls" }, smtp: { host: "", port: 465, security: "tls" } });

function mailSetupForm() {
  const m = (state.mailSetup ||= newSetup());
  const sec = (key, v) => `<select id="ms_${key}">${[["tls", "Encrypted (TLS)"], ["starttls", "Encrypted (STARTTLS)"], ["none", "Not encrypted (this computer only)"]].map(([id, l]) => `<option value="${id}" ${v === id ? "selected" : ""}>${l}</option>`).join("")}</select>`;
  const servers = m.step === "servers" ? `
      <div class="panel" style="margin:16px 0 8px"><div class="label">Server settings ${m.source ? `<span class="hintline">· ${esc(m.source)}</span>` : ""}</div>${m.help ? `<div class="hintline" style="margin-top:4px">${esc(m.help)}</div>` : ""}</div>
      <button class="linkbtn" type="button" data-act="mailadvanced" style="align-self:flex-start;padding-left:0">${m.advanced ? "Hide server settings" : "Change server settings"}</button>
      ${m.advanced ? `<div class="servergrid">
        <label class="field">Sign-in name<input id="ms_user" value="${esc(m.user)}" spellcheck="false"></label>
        <div class="grid3"><label class="field">Incoming (IMAP)<input id="ms_imap_host" value="${esc(m.imap.host)}" spellcheck="false"></label><label class="field">Port<input id="ms_imap_port" value="${m.imap.port}" inputmode="numeric"></label><label class="field">Security${sec("imap_security", m.imap.security)}</label></div>
        <div class="grid3"><label class="field">Outgoing (SMTP)<input id="ms_smtp_host" value="${esc(m.smtp.host)}" spellcheck="false"></label><label class="field">Port<input id="ms_smtp_port" value="${m.smtp.port}" inputmode="numeric"></label><label class="field">Security${sec("smtp_security", m.smtp.security)}</label></div>
      </div>` : ""}` : "";
  return `<form class="mailform" data-act="mailadd">
      <label class="field">Email address<input id="ms_email" value="${esc(m.email)}" type="email" autocomplete="email" spellcheck="false" required></label>
      <label class="field">Your name<input id="ms_name" value="${esc(m.name)}" autocomplete="name" placeholder="As people you email will see it"></label>
      <label class="field">Password<input id="ms_password" value="${esc(m.password)}" type="password" autocomplete="current-password" required><span class="hintline">Some providers need an app password made on their website.</span></label>
      ${servers}
      ${m.error ? `<p class="formerror" role="alert">${esc(m.error)}</p>` : ""}
      <div style="display:flex;gap:8px">${hasMail() ? `<button class="small" type="button" data-act="mailaddcancel" style="height:48px">Cancel</button>` : ""}<button class="big primary" type="submit" style="flex:1" ${m.busy ? "disabled" : ""}>${m.busy ? (m.step === "address" ? "Finding the settings…" : "Signing in…") : m.step === "address" ? "Next" : "Add account"}</button></div>
    </form>`;
}

function setupField(key, v) {
  const m = state.mailSetup;
  if (!m) return;
  if (key === "email" && v !== m.email && m.step === "servers") m.step = "address";
  if (["email", "name", "password", "user"].includes(key)) m[key] = v;
  const [srv, part] = key.split("_");
  if ((srv === "imap" || srv === "smtp") && part) m[srv][part] = part === "port" ? Number(v) || 0 : v;
}

async function addMailAccount() {
  const m = state.mailSetup;
  Object.assign(m, { busy: true, error: "" });
  render();
  try {
    if (m.step === "address") {
      const found = await tauri.core.invoke("mail_look_up", { email: m.email.trim() });
      // Only a guess: show the settings to check.
      if (found) Object.assign(m, { imap: found.imap, smtp: found.smtp, user: found.user, source: found.source, help: found.help, advanced: found.source.startsWith("a guess") });
      else Object.assign(m, { user: m.email.trim(), advanced: true, source: "not found: enter them from your provider" });
      m.step = "servers";
    } else {
      const added = await tauri.core.invoke("mail_add", { email: m.email, name: m.name, user: m.user || m.email, password: m.password, imap: m.imap, smtp: m.smtp });
      state.mail.accounts.push(added);
      state.mailSetup = null;
      state.mail.status[added.email] = "Getting your email…";
      showToast("Email added.");
      reloadMail();
    }
  } catch (e) {
    m.error = String(e);
  }
  if (state.mailSetup) state.mailSetup.busy = false;
  render();
}

async function removeMail(email) {
  await tauri.core.invoke("mail_remove", { account: email });
  state.mail.accounts = state.mail.accounts.filter((a) => a.email !== email);
  state.mail.list = state.mail.list.filter((r) => r.account !== email);
  state.mail.found = state.mail.found.filter((r) => r.account !== email);
  delete state.mail.folders[email];
  if (state.mailSel?.account === email) state.mailSel = null;
  if (state.compose?.account === email) state.compose = null;
  if (pickedFolder().account === email) state.mail.folder = "";
  loadDrafts();
  render();
}

// Keeping up with the accounts.

async function initMail() {
  try {
    state.mail.accounts = await tauri.core.invoke("mail_accounts");
  } catch {
    return;
  }
  loadDrafts();
  if (hasMail()) reloadMail();
}

let mailTimer = 0;
function reloadMail() {
  clearTimeout(mailTimer);
  mailTimer = setTimeout(async () => {
    if (!hasMail()) return;
    try {
      const before = new Set(state.mail.list.map((r) => `${r.account}:${r.folder}:${r.uid}`));
      const first = !state.mail.list.length;
      for (const a of state.mail.accounts) state.mail.folders[a.email] = await tauri.core.invoke("mail_folders", { account: a.email }).catch(() => []);
      if (state.mail.folder === "drafts") {
        await loadDrafts();
      } else {
        const { account, wire } = pickedFolder();
        const list = await tauri.core.invoke("mail_list", { account, folder: wire, limit: 200 });
        // The inbox of an account (or all of them) in view: new mail gets a notification.
        const inboxes = !wire || (state.mail.folders[account] || []).find((f) => f.wire === wire)?.role === "inbox";
        if (!first && inboxes) for (const r of list) if (!r.read && !before.has(`${r.account}:${r.folder}:${r.uid}`)) notifyMail(r);
        state.mail.list = list;
      }
    } catch {
      return;
    }
    render();
  }, 250);
}

function notifyMail(r) {
  const st = state.settings;
  if (!st.desktop || state.status === "dnd" || quietNow()) return;
  if (document.hasFocus() && state.tab === "mail") return;
  const body = st.preview ? r.subject || "(no subject)" : "New email";
  tauri.core.invoke("notify", { title: `Email from ${fromName(r.from)}`, body, conv: -1, sound: !!st.sound }).catch(() => {});
}

function onMail(e) {
  if (e.kind === "changed") return reloadMail();
  if (e.kind === "downloaded") {
    if (state.mailSel && sameMail(state.mailSel, e.account, e.folder, e.uid)) loadOpened();
    return;
  }
  if (e.kind === "status") {
    state.mail.status[e.account] = e.text;
    if (!e.text) state.mail.problems[e.account] = "";
  }
  if (e.kind === "problem") state.mail.problems[e.account] = e.text;
  if (e.kind === "signin_failed") state.mail.problems[e.account] = `${e.text} Remove the account in Settings › Email and add it again with the right password.`;
  render();
}

/** A meeting invitation in an email: when, where, and your answer. */
function inviteCard(o) {
  const v = o.invite;
  const day = dayWord(v.start);
  const Day = day.charAt(0).toUpperCase() + day.slice(1);
  const when = v.all_day ? `${Day}, all day` : `${Day}, ${hm(v.start)}–${hm(v.end)}`;
  const answered = state.mail.answered[v.uid] || v.answer;
  const head = { request: "Invitation", publish: "Event", cancel: "Cancelled", reply: "Answer to an invitation" }[v.method];
  const answers = [["accept", "Going"], ["maybe", "Maybe"], ["decline", "Not going"]];
  return `<section class="invite${v.method === "cancel" ? " cancelled" : ""}" aria-label="${esc(head)}">
    <div class="invitehead"><span class="label">${esc(head)}</span>${v.in_calendar ? `<span class="hintline">In your calendar</span>` : ""}</div>
    <div class="invitetitle">${esc(v.title || "(no title)")}</div>
    <div class="invitemeta"><span>${esc(when)}${v.repeats ? " · repeats" : ""}</span>${v.location ? `<span>${esc(v.location)}</span>` : ""}${v.organizer ? `<span>From ${esc(v.organizer)}</span>` : ""}${v.url ? `<span><a role="link" tabindex="0" class="maillink" data-act="askopen" data-url="${esc(v.url)}">Join online</a></span>` : ""}</div>
    ${v.method === "cancel"
      ? (v.in_calendar ? `<div class="answers"><button class="small" data-act="forgetinvite" data-id="${esc(v.uid)}">Remove from calendar</button></div>` : `<div class="hintline">It isn't in your calendar.</div>`)
      : v.method === "reply"
        ? ""
        : v.method === "publish"
          ? `<div class="answers"><button class="small primary" data-act="answer" data-id="accept">${v.in_calendar ? "Update in calendar" : "Add to calendar"}</button></div>`
          : `<div class="answers">${answers.map(([id, label]) => `<button class="small${answered === label ? " primary" : ""}" aria-pressed="${answered === label}" data-act="answer" data-id="${id}">${label}</button>`).join("")}${answered && answered !== "No answer yet" ? `<span class="hintline">You said: ${esc(answered)}</span>` : ""}</div>`}
  </section>`;
}

async function answerInvite(answer) {
  const o = state.mail.opened;
  if (!o?.invite) return;
  try {
    await tauri.core.invoke("mail_answer_invite", { account: o.account, folder: o.folder, uid: o.uid, answer });
    state.mail.answered[o.invite.uid] = { accept: "Going", maybe: "Maybe", decline: "Not going" }[answer];
    showToast(answer === "decline" ? "Sent: not going." : o.invite.method === "publish" ? "Added to your calendar." : "Sent your answer, and it's in your calendar.");
    await refreshCalendar();
    await loadOpened();
  } catch (e) {
    showToast(String(e));
  }
}

async function saveAttachment(i) {
  const o = state.mail.opened;
  if (!o) return;
  try {
    const path = await tauri.core.invoke("mail_save_attachment", { account: o.account, folder: o.folder, uid: o.uid, index: i });
    showToast(`Saved to ${path}`);
  } catch (e) {
    showToast(String(e));
  }
}

// ---------- Apps (bots and feeds people make) ----------

const MODES = [
  ["personal", "Each person privately", "Everyone gets their own 1:1 chat with it; nobody sees anyone else's."],
  ["shared", "In groups and channels", "It posts where it's added, and answers when someone writes @its-name."],
  ["both", "Both", "Privately, and in the groups and channels it's added to."],
];
const modeLabel = (m) => (MODES.find(([id]) => id === m) || MODES[0])[1];

function appsSettings() {
  if (!state.live) return `<h1>Apps</h1><p class="lede">Sign in to a chat server to make apps.</p>`;
  const a = state.apps;
  const shown = a.shown
    ? `<div class="panel keypanel" role="alert">
        <div class="label">${a.shown.secret ? "Keep these somewhere safe: they're shown only now." : "Your API key: shown only now."}</div>
        ${a.shown.key ? `<div class="keyline"><span>${a.shown.app ? "App key" : "API key"}</span><code>${esc(a.shown.key)}</code><button class="small" data-act="copy" data-id="${esc(a.shown.key)}">Copy</button></div>` : ""}
        ${a.shown.secret ? `<div class="keyline"><span>Webhook secret</span><code>${esc(a.shown.secret)}</code><button class="small" data-act="copy" data-id="${esc(a.shown.secret)}">Copy</button></div>` : ""}
        ${a.shown.app ? `<p class="hintline">Post as ${esc(nameOf(a.shown.app.user))}:</p><pre class="example">curl -X POST ${esc(serverUrl())}/v1/post \\
  -H "Authorization: Bearer ${esc(a.shown.key)}" \\
  -H "Content-Type: application/json" \\
  -d '{"channel": "Build alerts", "text": "Hello"}'</pre>
        <p class="hintline">Webhooks arrive as JSON with <code>X-Xenify-Timestamp</code> and <code>X-Xenify-Signature: sha256=…</code>, the HMAC-SHA256 of “timestamp.body” with the secret. Answer with <code>{"text": "…"}</code> to reply, or post later. Update a post again and again with <code>/v1/edit</code> to stream an answer.</p>` : `<p class="hintline">Use it as <code>Authorization: Bearer …</code> with ${esc(serverUrl())}/v1/apps to make and change apps from a script.</p>`}
        <button class="small" data-act="hidekey">Done</button>
      </div>`
    : "";
  const list = a.list.map((x) => `<div class="setting appitem">
      <div style="display:flex;gap:12px;align-items:center;min-width:0">${personAvatar(x.user)}<div style="min-width:0"><div class="label">${esc(nameOf(x.user))}<span class="appbadge">APP</span></div><div class="hintline">@${esc(x.user.username)} · ${esc(modeLabel(x.mode))}${x.webhook ? ` · webhook ${esc(x.webhook)}` : " · no webhook"}</div>${x.description ? `<div class="hintline">${esc(x.description)}</div>` : ""}</div></div>
      <div style="display:flex;gap:6px;flex:none"><button class="small" data-act="editapp" data-id="${x.user.id}">Change</button><button class="small" data-act="appkey" data-id="${x.user.id}">New key</button><button class="small danger-outline" style="height:36px" data-act="deleteapp" data-id="${x.user.id}">Delete</button></div>
    </div>`).join("");
  return `<h1>Apps</h1><p class="lede">Bots, alerts and feeds you make. They post with a key, and can hear what's said to them through a webhook: a web address the server sends messages to.</p>
    ${shown}
    <div style="display:flex;gap:8px;margin:4px 0 8px"><button class="small primary" data-act="newapp">${icons.plus}Make an app</button><button class="small" data-act="apikey">Make an API key for scripts</button></div>
    <h2>Your apps</h2>
    ${list || `<p class="hintline">You haven't made any apps yet.</p>`}`;
}

const serverUrl = () => (state.serverUrl || `https://${state.server}`).replace(/\/$/, "");

function appDialog(d) {
  const x = d.app;
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${x.id ? "Change" : "Make"} an app">
    <h2>${x.id ? `Change ${esc(x.display_name || x.name)}` : "Make an app"}</h2>
    ${x.id ? "" : `<label class="field">Name for @mentions<input id="ap_name" value="${esc(x.name)}" placeholder="e.g. helper" autocomplete="off" spellcheck="false" required><span class="hintline">2–32 letters, digits, dots, dashes or underscores; can't be changed</span></label>`}
    <label class="field">Name people see<input id="ap_display" value="${esc(x.display_name)}" placeholder="e.g. Policy helper" autocomplete="off"></label>
    <label class="field">What it does<input id="ap_desc" value="${esc(x.description)}" placeholder="Shown to people who use it" autocomplete="off"></label>
    <fieldset class="modes"><legend class="label">Who it talks with</legend>${MODES.map(([id, label, hint]) => `<label class="choice mode"><input type="radio" name="ap_mode" value="${id}" ${x.mode === id ? "checked" : ""}><span><b>${label}</b><span class="hintline">${hint}</span></span></label>`).join("")}</fieldset>
    <label class="field">Webhook (optional)<input id="ap_hook" value="${esc(x.webhook)}" placeholder="https://your-bot.example.org/xenify" autocomplete="off" spellcheck="false"><span class="hintline">Where the server sends messages for it. Leave empty for an app that only posts.</span></label>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">${x.id ? "Save" : "Make it"}</button></div>
  </form></div>`;
}

function readAppDialog() {
  const x = state.dialog.app;
  const v = (id) => document.getElementById(id)?.value;
  if (!x.id) x.name = v("ap_name") ?? x.name;
  x.display_name = v("ap_display") ?? x.display_name;
  x.description = v("ap_desc") ?? x.description;
  x.webhook = v("ap_hook") ?? x.webhook;
  x.mode = document.querySelector('input[name="ap_mode"]:checked')?.value || x.mode;
  return x;
}

function saveApp() {
  const x = readAppDialog();
  state.dialog.waiting = true;
  if (x.id) tell({ t: "update_app", app: x.id, display_name: x.display_name, description: x.description, webhook: x.webhook, mode: x.mode });
  else tell({ t: "create_app", name: x.name, display_name: x.display_name, description: x.description, webhook: x.webhook, mode: x.mode });
}

/** The apps in a chat that people can also talk to privately. */
function privateApps(c) {
  return (state.appsIn[c.id] || []).filter((a) => a.mode !== "shared");
}

function askPrivatelyBar(c) {
  if (c.kind === "direct") return "";
  const apps = privateApps(c);
  if (!apps.length) return "";
  return `<div class="askbar">${apps.map((a) => `<span>${icons.people.replace('width="20" height="20"', 'width="16" height="16"')}<b>${esc(nameOf(a.user))}</b> can answer you privately${a.description ? ` · ${esc(a.description)}` : ""}</span><button class="small" data-act="askapp" data-id="${a.user.id}">Ask privately</button>`).join("")}</div>`;
}

// ---------- Calendar, reminders and Today ----------

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const HOUR_PX = 48;

const startOfDay = (t) => {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};
/** Monday 00:00 of the week holding `t`. */
const startOfWeek = (t) => {
  const d = new Date(startOfDay(t));
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d.getTime();
};
const addDays = (t, n) => {
  const d = new Date(t);
  d.setDate(d.getDate() + n);
  return d.getTime();
};
const hm = (t) => new Date(t).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const pad = (n) => String(n).padStart(2, "0");
const dateInput = (t) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};
const timeInput = (t) => {
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};
/** "2026-10-18" + "09:30" → Unix ms (this computer's time zone). */
const fromInputs = (date, time) => new Date(`${date}T${time || "00:00"}`).getTime();

async function loadToday() {
  if (!tauri) return;
  const now = Date.now();
  try {
    const list = await tauri.core.invoke("cal_between", { from: startOfDay(now), to: startOfDay(now) + DAY });
    // What's left of today: not over, not ticked off.
    state.cal.today = list.filter((e) => !e.done && (e.all_day || Math.max(e.end, e.start + 15 * MIN) > now));
    state.cal.snoozed = await tauri.core.invoke("cal_snoozed", { now });
  } catch {
    return;
  }
  render();
}

async function loadWeek() {
  try {
    state.cal.week = await tauri.core.invoke("cal_between", { from: state.cal.weekStart, to: addDays(state.cal.weekStart, 7) });
  } catch (e) {
    showToast(String(e));
  }
  render();
  // Start the day at 08:00.
  const g = document.getElementById("calscroll");
  if (g && !g.dataset.scrolled) {
    g.scrollTop = 8 * HOUR_PX - 8;
    g.dataset.scrolled = "1";
  }
}

function whenOf(e) {
  if (e.all_day) return "All day";
  const now = Date.now();
  if (e.start <= now && e.end > now) return "Now";
  return hm(e.start);
}

function todayItem(e) {
  return `<button class="todayitem${e.kind === "reminder" ? " reminder" : ""}" data-act="calitem" data-id="${e.id}">
    <span class="when">${esc(whenOf(e))}</span>
    <span class="what">${e.kind === "reminder" ? icons.bell.replace('width="20" height="20"', 'width="14" height="14"') : ""}<span class="t">${esc(e.title)}</span></span>
  </button>`;
}

/** The Today panel: the next thing, and the rest on request. */
function todayPanel() {
  if (!tauri) return "";
  const list = state.cal.today;
  const shown = state.cal.more ? list : list.slice(0, 1);
  const rest = list.length - 1;
  return `<section class="today" aria-label="Today">
    <div class="todayhead"><b>Today</b><button class="linkbtn" data-act="calendar">Open calendar</button></div>
    ${shown.map(todayItem).join("") || `<div class="hintline todayempty">Nothing else today.</div>`}
    ${rest > 0 ? `<button class="linkbtn more" data-act="todaymore">${state.cal.more ? "Show less" : `Show ${rest} more`}</button>` : ""}
  </section>`;
}

/** Side-by-side columns for events that overlap on one day. */
function layoutDay(events) {
  const cols = [];
  const out = [];
  for (const e of [...events].sort((a, b) => a.start - b.start)) {
    const end = Math.max(e.end, e.start + 30 * MIN);
    let c = cols.findIndex((last) => last <= e.start);
    if (c < 0) {
      c = cols.length;
      cols.push(end);
    } else cols[c] = end;
    out.push({ e, c });
  }
  return out.map((x) => ({ ...x, of: cols.length }));
}

function calendarSheet() {
  const ws = state.cal.weekStart;
  const days = [...Array(7)].map((_, i) => addDays(ws, i));
  const today = startOfDay(Date.now());
  const label = `${new Date(ws).toLocaleDateString([], { day: "numeric", month: "short" })} – ${new Date(addDays(ws, 6)).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" })}`;
  const allDay = days.map((d) => state.cal.week.filter((e) => e.all_day && e.start < d + DAY && Math.max(e.end, e.start + 1) > d));
  const hasAllDay = allDay.some((l) => l.length);
  const nowLine = (d) => (d === today ? `<div class="nowline" style="top:${((Date.now() - d) / HOUR) * HOUR_PX}px"></div>` : "");
  const cols = days.map((d, i) => {
    const timed = state.cal.week.filter((e) => !e.all_day && e.start >= d && e.start < d + DAY);
    const blocks = layoutDay(timed).map(({ e, c, of }) => {
      const top = ((e.start - d) / HOUR) * HOUR_PX;
      const h = Math.max(((e.end - e.start) / HOUR) * HOUR_PX, e.kind === "reminder" ? 24 : 22);
      const w = 100 / of;
      return `<button class="calevent${e.kind === "reminder" ? " reminder" : ""}${e.done ? " done" : ""}" style="top:${top}px;height:${h - 2}px;left:calc(${c * w}% + 2px);width:calc(${w}% - 4px)" data-act="calitem" data-id="${e.id}" title="${esc(e.title)}">
        <b>${e.repeat ? `<span class="repeaticon" title="Repeats" aria-label="Repeats">↻</span>` : ""}${e.kind === "reminder" ? icons.bell.replace('width="20" height="20"', 'width="12" height="12"') : ""}${esc(e.title)}</b>${h > 40 ? `<span>${hm(e.start)}${e.end > e.start ? `–${hm(e.end)}` : ""}</span>` : ""}
      </button>`;
    }).join("");
    return `<div class="calday${d === today ? " istoday" : ""}" data-act="calslot" data-id="${i}">${blocks}${nowLine(d)}</div>`;
  }).join("");
  const hours = [...Array(24)].map((_, h) => `<div class="calhour"><span>${h === 0 ? "" : `${pad(h)}:00`}</span></div>`).join("");
  return `<main class="sheet calendar">
    <header class="sheet-head">
      <div class="title"><div><h1>Calendar</h1><div class="sub">${esc(label)}</div></div></div>
      <div class="actions">
        <button class="iconbtn" data-act="calprev" aria-label="Previous week">${icons.back}</button>
        <button class="textbtn" data-act="caltoday">This week</button>
        <button class="iconbtn" data-act="calnext" aria-label="Next week" style="transform:scaleX(-1)">${icons.back}</button>
        <button class="small primary" data-act="calnew">${icons.plus}New event</button>
      </div>
    </header>
    <div class="calhead"><div class="calgutter"></div>${days.map((d) => `<div class="caldayname${d === today ? " istoday" : ""}"><span>${new Date(d).toLocaleDateString([], { weekday: "short" })}</span><b>${new Date(d).getDate()}</b></div>`).join("")}</div>
    ${hasAllDay ? `<div class="calallday"><div class="calgutter hintline">All day</div>${allDay.map((l) => `<div>${l.map((e) => `<button class="calchip" data-act="calitem" data-id="${e.id}">${esc(e.title)}</button>`).join("")}</div>`).join("")}</div>` : ""}
    <div class="calscroll" id="calscroll"><div class="calgrid" style="height:${24 * HOUR_PX}px"><div class="calhours">${hours}</div>${cols}</div></div>
  </main>`;
}

function openCalendar(at = Date.now()) {
  if (state.compose) return void parkCompose().then(() => openCalendar(at));
  Object.assign(state, { view: "calendar", mailSel: null, menu: null });
  state.cal.weekStart = startOfWeek(at);
  loadWeek();
}

/** The event editor (a dialog). */
function eventDialog(d) {
  const e = d.event;
  const link = e.link ? JSON.parse(e.link) : null;
  const remindOpts = [[null, "No reminder"], [0, "When it starts"], [5, "5 minutes before"], [10, "10 minutes before"], [30, "30 minutes before"], [60, "1 hour before"], [1440, "1 day before"]];
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${e.id ? "Change" : "New"} ${e.kind === "reminder" ? "reminder" : "event"}">
    <h2>${e.id ? (e.kind === "reminder" ? "Reminder" : "Event") : e.kind === "reminder" ? "New reminder" : "New event"}</h2>
    ${link ? `<div class="panel" style="display:flex;align-items:center;justify-content:space-between;gap:10px"><span class="hintline">About ${esc(link.label || "")}</span><button class="small" type="button" data-act="calopenlink">Open</button></div>` : ""}
    <label class="field">${e.kind === "reminder" ? "Remind me to" : "Name"}<input id="ev_title" value="${esc(e.title)}" autocomplete="off" required></label>
    <div class="grid3 evtime">
      <label class="field">Date<input id="ev_date" type="date" value="${dateInput(e.start)}" required></label>
      ${e.all_day ? "" : `<label class="field">${e.kind === "reminder" ? "At" : "From"}<input id="ev_from" type="time" value="${timeInput(e.start)}"></label>`}
      ${e.all_day || e.kind === "reminder" ? "" : `<label class="field">To<input id="ev_to" type="time" value="${timeInput(e.end)}"></label>`}
    </div>
    ${e.kind === "reminder" ? "" : `<label class="choice" style="border:none;padding:0"><input id="ev_allday" type="checkbox" ${e.all_day ? "checked" : ""}> All day</label>
    <label class="field">Remind me<select id="ev_remind">${remindOpts.map(([v, l]) => `<option value="${v ?? ""}" ${(e.remind ?? null) === v ? "selected" : ""}>${l}</option>`).join("")}</select></label>`}
    ${e.kind === "reminder" ? "" : `<label class="field">Repeat<select id="ev_repeat">${repeatChoices(e.repeat)}</select>${e.repeat ? `<span class="hintline">Changes here change every time it happens; the date is the first one.</span>` : ""}</label>`}
    <label class="field">Notes<textarea id="ev_notes" rows="2">${esc(e.notes)}</textarea></label>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons">
      ${e.id ? `<button class="small danger-outline" type="button" data-act="caldelete" style="margin-right:auto;height:36px">Delete</button>` : ""}
      ${e.id && e.kind === "reminder" ? `<button class="small" type="button" data-act="caldone">${e.done ? "Not done" : "Done"}</button>` : ""}
      <button class="small" type="button" data-act="closedialog">Cancel</button>
      <button class="small primary" type="submit">Save</button>
    </div>
  </form></div>`;
}

const REPEATS = [
  ["", "Doesn't repeat"],
  ["FREQ=DAILY", "Every day"],
  ["FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "Every weekday (Monday to Friday)"],
  ["FREQ=WEEKLY", "Every week"],
  ["FREQ=MONTHLY", "Every month"],
  ["FREQ=YEARLY", "Every year"],
];

function repeatChoices(current) {
  const known = REPEATS.some(([v]) => v === (current || ""));
  return REPEATS.map(([v, l]) => `<option value="${v}" ${(current || "") === v ? "selected" : ""}>${l}</option>`).join("")
    + (known ? "" : `<option value="${esc(current)}" selected>As the invitation says</option>`);
}

function editEvent(event) {
  // One appearance of a repeating event: change the series.
  if (event.repeat && event.series_start) event = { ...event, start: event.series_start, end: event.series_end };
  state.dialog = { kind: "event", event: { ...event }, error: "" };
  state.menu = null;
  state.ctx = null;
  render();
}

/** Reads the editor's boxes into the event being edited. */
function readEventDialog() {
  const e = state.dialog.event;
  const v = (id) => document.getElementById(id)?.value;
  e.title = v("ev_title") ?? e.title;
  e.notes = v("ev_notes") ?? e.notes;
  const allDay = document.getElementById("ev_allday");
  if (allDay) e.all_day = allDay.checked;
  const date = v("ev_date") || dateInput(e.start);
  const length = e.end - e.start;
  if (e.all_day) {
    e.start = fromInputs(date, "00:00");
    e.end = e.start + DAY;
  } else {
    e.start = fromInputs(date, v("ev_from") || timeInput(e.start));
    e.end = e.kind === "reminder" ? e.start : v("ev_to") ? fromInputs(date, v("ev_to")) : e.start + Math.max(length, 0);
  }
  const r = document.getElementById("ev_remind");
  if (r) e.remind = r.value === "" ? null : Number(r.value);
  const rp = document.getElementById("ev_repeat");
  if (rp) e.repeat = rp.value;
  return e;
}

async function saveEvent() {
  const d = state.dialog;
  const e = readEventDialog();
  try {
    await tauri.core.invoke("cal_save", { event: e });
    state.dialog = null;
    await refreshCalendar();
  } catch (err) {
    d.error = String(err);
    render();
  }
}

async function refreshCalendar() {
  await loadToday();
  if (state.view === "calendar") await loadWeek();
}

function findEvent(id) {
  return [...state.cal.today, ...state.cal.week, ...state.cal.snoozed].find((e) => e.id === id);
}

/** Follows a reminder back to its message or email. */
function openLinked(e) {
  const link = e.link ? JSON.parse(e.link) : null;
  if (!link) return editEvent(e);
  state.dialog = null;
  if (link.type === "mail") return openMail(link.account || state.mail.accounts[0]?.email, link.folder, link.uid);
  if (link.type === "chat" && conv(link.conv)) {
    open(link.conv);
    if (link.msg && findMsg(link.msg)[1]) {
      state.flash = link.msg;
      render();
      scrollToMessage(link.msg);
    }
  }
}

// "Remind me" on a message, chat or email.

/** The times offered: later today (if there's time), tomorrow and next week. */
function remindTimes(now = new Date()) {
  const at = (days, h) => {
    const d = new Date(now);
    d.setDate(d.getDate() + days);
    d.setHours(h, 0, 0, 0);
    return d.getTime();
  };
  const out = [];
  const later = Math.ceil((now.getTime() + 3 * HOUR) / (30 * MIN)) * 30 * MIN;
  if (startOfDay(later) === startOfDay(now.getTime()) && new Date(later).getHours() < 21) out.push(["Later today", later]);
  out.push(["Tomorrow morning", at(1, 9)]);
  // Next week's Monday (not tomorrow, when tomorrow is Monday).
  let toMonday = ((8 - now.getDay()) % 7) || 7;
  if (toMonday === 1) toMonday = 8;
  out.push(["Next week", at(toMonday, 9)]);
  return out;
}

/** "today", "tomorrow", "Thursday" (this week), or "Mon 26 Oct". */
function dayWord(t) {
  const n = Math.round((startOfDay(t) - startOfDay(Date.now())) / DAY);
  if (n === 0) return "today";
  if (n === 1) return "tomorrow";
  if (n > 1 && n < 7) return new Date(t).toLocaleDateString([], { weekday: "long" });
  return new Date(t).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
}

/** Opens the "Remind me" choices for `about` ({ title, link }). */
function remindAbout(about) {
  state.ctx = null;
  state.menu = null;
  state.dialog = { kind: "remind", about };
  render();
}

function remindDialog(d) {
  const day = (t) => dayWord(t);
  return `<div class="scrim"><div class="dialog" role="dialog" aria-label="Remind me">
    <h2>Remind me</h2>
    <p>${esc(d.about.title)}</p>
    <div class="remindopts">${remindTimes().map(([label, t], i) => `<button class="remindopt" data-act="remindat" data-id="${i}"><b>${label}</b><span>${day(t)} at ${hm(t)}</span></button>`).join("")}
      <button class="remindopt" data-act="remindpick"><b>Pick a time…</b><span>Any day and time</span></button></div>
    <div class="buttons"><button class="small" data-act="closedialog">Cancel</button></div>
  </div></div>`;
}

async function remindAt(t) {
  const { about } = state.dialog;
  // A whole chat or an email waits in Later; a message just gets a reminder.
  const hides = about.link.type === "mail" || (about.link.type === "chat" && !about.link.msg);
  const event = { id: 0, title: about.title, start: t, end: t, kind: "reminder", remind: 0, link: JSON.stringify(about.link), notes: "", hides };
  try {
    await tauri.core.invoke("cal_save", { event });
    state.dialog = null;
    showToast(hides ? `Put off until ${dayWord(t)} at ${hm(t)}: it's in Later till then.` : `I'll remind you ${dayWord(t)} at ${hm(t)}.`);
    await refreshCalendar();
  } catch (e) {
    showToast(String(e));
  }
}

const chatAbout = (c, m) => ({
  title: m ? `${m.sender.id === state.me?.id ? "Your message" : senderName(c, m.sender)} in ${titleOf(c)}: “${m.text.slice(0, 60)}${m.text.length > 60 ? "…" : ""}”` : `Chat: ${titleOf(c)}`,
  link: { type: "chat", conv: c.id, msg: m?.id, label: m ? `a message in ${titleOf(c)}` : titleOf(c) },
});
const mailAbout = (r) => ({
  title: `Email from ${fromName(r.from)}: ${r.subject || "(no subject)"}`,
  link: { type: "mail", account: r.account, folder: r.folder, uid: r.uid, label: `the email “${r.subject || "(no subject)"}”` },
});

function laterRow(e) {
  const l = e.link ? JSON.parse(e.link) : {};
  return `<div class="row compact laterrow">
    <button class="rowmain" data-act="calitem" data-id="${e.id}">
      <span class="avatar square small" style="background:var(--tint);color:var(--ink-2)" aria-hidden="true">${l.type === "mail" ? icons.mail : icons.bell.replace('width="20" height="20"', 'width="16" height="16"')}</span>
      <span class="body"><span class="name">${esc(e.title)}</span><span class="last">Back ${esc(dayWord(e.start))} at ${esc(hm(e.start))}</span></span>
    </button>
    <button class="small" data-act="bringback" data-id="${e.id}">Bring back</button>
  </div>`;
}

// Reminders that are due: shown once, as a notification and on screen.

async function checkDue() {
  if (!tauri) return;
  let due = [];
  try {
    due = await tauri.core.invoke("cal_due", { now: Date.now() });
  } catch {
    return;
  }
  for (const e of due) {
    await tauri.core.invoke("cal_reminded", { id: e.id, at: e.start }).catch(() => {});
    const st = state.settings;
    const link = e.link ? JSON.parse(e.link) : null;
    if (st.desktop && state.status !== "dnd" && !quietNow()) {
      const title = e.kind === "reminder" ? "Reminder" : `${e.title} · ${e.start > Date.now() ? `at ${hm(e.start)}` : "now"}`;
      tauri.core.invoke("notify", { title, body: e.kind === "reminder" ? e.title : e.notes || "From your calendar", conv: link?.type === "chat" ? link.conv : -2, sound: !!st.sound }).catch(() => {});
    }
    showToast(e.kind === "reminder" ? `Reminder: ${e.title}` : `${e.title} ${e.start > Date.now() ? `at ${hm(e.start)}` : "now"}`);
  }
  if (due.length) loadToday();
}

// ---------- Spelling and grammar ----------

const issuesFor = (id, text) => (state.issues[id]?.text === text ? state.issues[id].list : state.issues[id]?.shifted || []);

/** The checker's `code` marks shown as quotes. */
const tidy = (m) => m.replace(/`([^`]*)`/g, "“$1”");

/** `text` as HTML with the things to look at underlined (drawn behind the box). */
function underlined(text, list) {
  let out = "";
  let at = 0;
  list.forEach((f, n) => {
    if (f.start < at || f.end > text.length) return;
    out += esc(text.slice(at, f.start)) + `<mark class="issue ${f.kind}" data-i="${n}">${esc(text.slice(f.start, f.end))}</mark>`;
    at = f.end;
  });
  // A trailing new line needs something after it to take up room.
  return out + esc(text.slice(at)) + "\u200b";
}

/** The text box being written in: a chat message, or an email. */
function writing() {
  // A sent message being changed (checked as chat).
  if (state.editing && document.getElementById("edit")) {
    return {
      ta: "edit", bd: "editbackdrop", line: "editcheck", key: `edit:${state.editing}`, kind: "chat",
      get: () => state.editText ?? "",
      set: (v) => (state.editText = v),
    };
  }
  if (state.compose) {
    return {
      ta: "mailbody", bd: "mailbackdrop", line: "mailcheck", key: "compose", kind: "email",
      get: () => state.compose?.text || "",
      set: (v) => (state.compose.text = v),
    };
  }
  const c = current();
  if (!c || !c.can_post || state.mailSel) return null;
  return {
    ta: "draft", bd: "backdrop", line: "checkstate", key: c.id, kind: "chat",
    get: () => state.drafts[c.id] || "",
    set: (v) => (state.drafts[c.id] = v),
  };
}

function paintBackdrop() {
  const w = writing();
  const b = w && document.getElementById(w.bd);
  const t = w && document.getElementById(w.ta);
  if (!b || !t) return;
  b.innerHTML = underlined(t.value, issuesFor(w.key, t.value));
  b.scrollTop = t.scrollTop;
  const line = document.getElementById(w.line);
  if (line && w.ta === "edit") {
    const n = issuesFor(w.key, t.value).length;
    line.innerHTML = `${n ? `<span class="checkcount">${n === 1 ? "1 thing" : `${n} things`} to look at</span> · ` : ""}Ctrl+Enter to save · Esc to cancel`;
  } else if (line && !line.querySelector(".typing")) line.innerHTML = checkLine(w.key, t.value);
}

function checkLine(key, text) {
  const n = issuesFor(key, text).length;
  const keys = "Ctrl+Enter to send";
  if (!n) return `${keys} · Spelling and grammar are checked on this computer`;
  return `${keys} · <span class="checkcount">${n === 1 ? "1 thing" : `${n} things`} to look at</span> · right-click an underlined word`;
}

/** Keeps underlines the edit didn't touch, moved along with the text. */
function shiftIssues(id, before, after) {
  const was = state.issues[id];
  if (!was || was.text !== before) return;
  let p = 0;
  while (p < before.length && p < after.length && before[p] === after[p]) p++;
  let q = 0;
  while (q < before.length - p && q < after.length - p && before[before.length - 1 - q] === after[after.length - 1 - q]) q++;
  const delta = after.length - before.length;
  const kept = [];
  for (const f of was.list) {
    if (f.end <= p) kept.push(f);
    else if (f.start >= before.length - q) kept.push({ ...f, start: f.start + delta, end: f.end + delta });
  }
  state.issues[id] = { text: after, list: kept };
}

let checkTimer = 0;
function scheduleCheck(delay = 400) {
  clearTimeout(checkTimer);
  checkTimer = setTimeout(runCheck, delay);
}

async function runCheck() {
  clearTimeout(checkTimer);
  const w = writing();
  if (!w || !tauri) return;
  const text = w.get();
  const { spelling, grammar, spotter } = state.settings;
  let list = [];
  if (text.trim() && (spelling || grammar)) {
    try {
      list = await tauri.core.invoke("check", { text, options: { kind: w.kind, spelling, grammar, spotter, ignored: state.ignored[w.key] || [] } });
    } catch {
      return;
    }
  }
  // Typed more meanwhile (or moved on): the next check will catch up.
  const now = writing();
  if (!now || now.key !== w.key || now.get() !== text) return;
  state.issues[w.key] = { text, list };
  paintBackdrop();
}

/** The underlined mistake under the pointer, if any. */
function issueAt(x, y) {
  const w = writing();
  const t = w && document.getElementById(w.ta);
  if (!t) return null;
  // Which underlined word (drawn behind the text box) is at the pointer.
  const mark = [...document.querySelectorAll(`#${w.bd} mark.issue`)].find((m) =>
    [...m.getClientRects()].some((r) => x >= r.left - 2 && x <= r.right + 2 && y >= r.top - 2 && y <= r.bottom + 4),
  );
  if (!mark) return null;
  return issuesFor(w.key, t.value)[Number(mark.dataset.i)] || null;
}

function fixItems(f) {
  const w = writing();
  const items = [{ note: tidy(f.message) }];
  for (const s of f.suggestions) items.push({ label: s === "" ? `Delete “${f.word}”` : s, strong: true, run: () => replaceIn(w, f, s) });
  if (!f.suggestions.length) items.push({ note: "No suggestion: reword it, or ignore it." });
  if (f.kind === "spelling") items.push({ label: `Add “${f.word}” to your dictionary`, hint: "Also in Xenify Write", run: () => addWord(f.word) });
  items.push({
    label: "Ignore",
    hint: w.kind === "email" ? "Just here, in this email" : "Just here, in this message",
    run: () => {
      (state.ignored[w.key] ||= []).push(f.key);
      state.issues[w.key].list = state.issues[w.key].list.filter((x) => x.key !== f.key);
      scheduleCheck(0);
    },
  });
  return items;
}

function replaceIn(w, f, s) {
  const text = w.get();
  const next = text.slice(0, f.start) + s + text.slice(f.end);
  shiftIssues(w.key, text, next);
  w.set(next);
  setTimeout(() => {
    const t = document.getElementById(w.ta);
    if (t) {
      t.focus();
      t.setSelectionRange(f.start + s.length, f.start + s.length);
    }
  });
  scheduleCheck(0);
}

async function loadProofing() {
  if (!tauri) return;
  try {
    state.words = await tauri.core.invoke("words");
    state.spotter = await tauri.core.invoke("spotter_state");
    state.credits ||= `Word meanings: ${await tauri.core.invoke("checker_credits")}.`;
  } catch {
    return;
  }
  if (state.view === "settings") render();
}

async function addWord(w) {
  state.words = await tauri.core.invoke("add_word", { word: w });
  state.issues = {};
  render();
  scheduleCheck(0);
}

async function removeWord(w) {
  state.words = await tauri.core.invoke("remove_word", { word: w });
  state.issues = {};
  render();
}

function spotterHint() {
  const base = "A small AI model on this computer that catches more mistakes (uses more memory)";
  if (!state.settings.spotter) return base;
  if (state.spotter === "not installed") return "The model isn't on this computer yet. Xenify Write can download it (Proofing settings).";
  if (state.spotter === "unavailable") return "The shared checker couldn't start, so only the usual checks run.";
  return base;
}

// ---------- Render ----------

function applyLook() {
  const t = state.settings.theme;
  const dark = t === "dark" || (t === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.getElementById("app").style.zoom = state.settings.size / 100;
}

function render() {
  const active = document.activeElement;
  const focus = active?.id;
  // Text boxes keep their cursor across a redraw (other inputs have none).
  let caret = null;
  try {
    if (focus && typeof active.selectionStart === "number") caret = [active.selectionStart, active.selectionEnd];
  } catch {}
  const box = document.getElementById("messages");
  const keepScroll = box && state.view === "chat" ? box.scrollTop : null;
  const calTop = document.getElementById("calscroll")?.scrollTop;
  menuRuns = [];
  applyLook();
  saveSettings();
  const page = state.view === "loading" ? "" : state.view === "signin" ? signinPage() : state.view === "settings" ? settingsPage() : sidebar() + (state.view === "calendar" ? calendarSheet() : state.compose ? composeSheet() : state.mailSel ? mailSheet() : chatSheet());
  document.getElementById("app").innerHTML = page + ctxMenu() + confirmBox() + dialogBox() + (state.toast ? `<div class="toast" role="alert">${esc(state.toast)}</div>` : "");
  const nbox = document.getElementById("messages");
  if (nbox) nbox.scrollTop = keepScroll ?? nbox.scrollHeight;
  const ncal = document.getElementById("calscroll");
  if (ncal && calTop != null) {
    ncal.scrollTop = calTop;
    ncal.dataset.scrolled = "1";
  }
  if (focus) {
    const el = document.getElementById(focus);
    if (el) {
      el.focus();
      try {
        if (caret && el.setSelectionRange) el.setSelectionRange(caret[0], caret[1]);
      } catch {}
    }
  }
  sizeBox(document.getElementById("draft"));
  sizeBox(document.getElementById("edit"));
  sizeBox(document.getElementById("mailbody"), 420);
  fitMailFrame();
  // A dialog keeps the keyboard: start in its first box.
  const dlg = document.querySelector(".dialog");
  if (dlg && !dlg.contains(document.activeElement)) dlg.querySelector("input, button")?.focus();
}

function sizeBox(t, max = 200) {
  if (!t) return;
  t.style.height = "auto";
  t.style.height = Math.min(t.scrollHeight, max) + "px";
}

// ---------- Doing things ----------

function open(id, opts = {}) {
  // An email being written is kept in Drafts first.
  if (state.compose) return void parkCompose().then(() => open(id, opts));
  state.mailSel = null;
  const c = conv(id);
  state.sel = id;
  state.view = "chat";
  state.menu = null;
  state.editing = null;
  if (!opts.keepSearch) state.find = null;
  if (c) {
    state.readLine[id] = unread(c) ? c.read_seq : null;
    markRead(c);
    if (state.live && !state.messages[id]) tell({ t: "history", conv: id, before: null, limit: 100 });
    // Apps here that people can also ask privately.
    if (state.live && c.kind !== "direct" && c.members.some((u) => u.app)) tell({ t: "apps_in", conv: id });
  }
  render();
  const box = document.getElementById("messages");
  if (box) box.scrollTop = box.scrollHeight;
  if (!opts.keepSearch) document.getElementById("draft")?.focus();
  scheduleCheck(0);
}

async function send(anyway = false) {
  const c = current();
  const text = (state.drafts[c.id] || "").trim();
  if (!text) return;
  if (state.settings.before && !anyway && tauri) {
    // "Don't send until it's checked": check now if a check is due.
    await runCheck();
    const left = issuesFor(c.id, state.drafts[c.id] || "");
    if (left.length) {
      state.confirm = {
        title: left.length === 1 ? "One thing to check" : `${left.length} things to check`,
        text: left.slice(0, 4).map((i) => `“${i.word}”: ${tidy(i.message)}`).join("\n") + (left.length > 4 ? "\n…" : ""),
        action: "Send anyway",
        cancel: "Fix it",
        primary: true,
        run: () => send(true),
      };
      return render();
    }
  }
  delete state.issues[c.id];
  delete state.ignored[c.id];
  const seq = c.last_seq + 1;
  const client_id = crypto.randomUUID();
  const msg = { id: -Date.now(), conv: c.id, seq, sender: state.me, text, at: Date.now(), client_id, edited: false, deleted: false, pending: state.live };
  (state.messages[c.id] ||= []).push(msg);
  if (state.live) tell({ t: "send", conv: c.id, client_id, text });
  else Object.assign(c, { last: msg, last_seq: seq, read_seq: seq });
  state.drafts[c.id] = "";
  state.readLine[c.id] = null;
  state.conversations.sort((a, b) => (b.last?.at || 0) - (a.last?.at || 0));
  render();
  const box = document.getElementById("messages");
  if (box) box.scrollTop = box.scrollHeight;
  document.getElementById("draft")?.focus();
}

function saveEdit() {
  const t = document.getElementById("edit");
  const [, m] = findMsg(state.editing);
  if (t && m && t.value.trim() && t.value.trim() !== m.text) {
    tell({ t: "edit", conv: m.conv, seq: m.seq, text: t.value.trim() });
    Object.assign(m, { text: t.value.trim(), edited: true });
  }
  state.editing = null;
  state.editText = null;
  render();
}

function stepFind(by) {
  const f = state.find;
  if (!f || !f.ids.length) return;
  f.at = (f.at + by + f.ids.length) % f.ids.length;
  state.flash = f.ids[f.at];
  render();
  scrollToMessage(f.ids[f.at]);
}

document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-act]");
  if (!el || el.tagName === "FORM") {
    if ((state.menu || state.ctx || state.picker) && !e.target.closest(".menu, .picker")) {
      state.menu = null;
      state.ctx = null;
      state.picker = null;
      render();
    }
    return;
  }
  const { act, id } = el.dataset;
  const c = current();
  state.ctx = act === "run" ? state.ctx : null;
  switch (act) {
    case "open": return open(Number(id));
    case "calendar": return openCalendar();
    case "todaymore": state.cal.more = !state.cal.more; break;
    case "calitem": {
      const ev = findEvent(Number(id));
      if (!ev) break;
      return ev.kind === "reminder" && ev.link && state.view !== "calendar" ? openLinked(ev) : editEvent(ev);
    }
    case "calitemlater": {
      const ev = state.cal.snoozed.find((x) => x.id === Number(id));
      return ev && openLinked(ev);
    }
    case "calprev": state.cal.weekStart = addDays(state.cal.weekStart, -7); return loadWeek();
    case "calnext": state.cal.weekStart = addDays(state.cal.weekStart, 7); return loadWeek();
    case "caltoday": state.cal.weekStart = startOfWeek(Date.now()); return loadWeek();
    case "calnew": {
      const start = Math.ceil(Date.now() / HOUR) * HOUR;
      return editEvent({ id: 0, title: "", start, end: start + HOUR, all_day: false, notes: "", kind: "event", link: "", remind: 10, reminded: false, done: false });
    }
    case "calslot": {
      const r = el.getBoundingClientRect();
      const start = addDays(state.cal.weekStart, Number(id)) + Math.floor(((e.clientY - r.top) / HOUR_PX) * 2) * 30 * MIN;
      return editEvent({ id: 0, title: "", start, end: start + HOUR, all_day: false, notes: "", kind: "event", link: "", remind: 10, reminded: false, done: false });
    }
    case "caldelete": {
      const ev = state.dialog?.event;
      state.dialog = null;
      if (ev?.id) tauri.core.invoke("cal_delete", { id: ev.id }).then(refreshCalendar);
      break;
    }
    case "caldone": {
      const ev = readEventDialog();
      ev.done = !ev.done;
      return saveEvent();
    }
    case "calopenlink": return openLinked(state.dialog.event);
    case "remindat": return remindAt(remindTimes()[Number(id)][1]);
    case "bringback": {
      const ev = state.cal.snoozed.find((x) => x.id === Number(id));
      if (ev) tauri.core.invoke("cal_save", { event: { ...ev, done: true } }).then(refreshCalendar);
      return;
    }
    case "answer": return answerInvite(id);
    case "forgetinvite": tauri.core.invoke("mail_forget_invite", { uid: id }).then(() => { showToast("Taken out of your calendar."); refreshCalendar(); loadOpened(); }); return;
    case "remindpick": {
      const { about } = state.dialog;
      const t = remindTimes().find(([l]) => l === "Tomorrow morning")[1];
      return editEvent({ id: 0, title: about.title, start: t, end: t, all_day: false, notes: "", kind: "reminder", link: JSON.stringify(about.link), remind: 0, reminded: false, done: false });
    }
    case "mail": return openMail(el.dataset.account, el.dataset.folder, Number(el.dataset.uid));
    case "opendraft": return openDraft(id);
    case "unattach": state.compose.attachments.splice(Number(id), 1); scheduleDraft(); break;
    case "mailaddmore": state.mailSetup = newSetup(); break;
    case "mailaddcancel": state.mailSetup = null; break;
    case "newmail": state.menu = null; return startCompose({});
    case "reply": return startCompose(replyOf(state.mail.opened, false));
    case "replyall": return startCompose(replyOf(state.mail.opened, true));
    case "forward": return startCompose(forwardOf(state.mail.opened));
    case "discard": return discardCompose();
    case "sendmail": return sendMail();
    case "showcc": state.compose.showCc = true; break;
    case "mailsetup": state.view = "settings"; state.section = "email"; break;
    case "mailcheck": tauri.core.invoke("mail_sync_now"); break;
    case "mailremove": state.confirm = { title: `Remove ${id}?`, text: "Its settings, saved password, drafts and the copy of its mail on this computer are removed. Nothing changes on the mail server.", action: "Remove", run: () => removeMail(id) }; break;
    case "mailadvanced": state.mailSetup.advanced = !state.mailSetup.advanced; break;
    case "askopen": return askOpenLink(el.dataset.url);
    case "saveattachment": return saveAttachment(Number(id));
    case "hit": return jumpTo(Number(id));
    case "tab": state.tab = id; if (id === "channels") tell({ t: "find_conversations", q: "" }); break;
    case "menu": state.menu = state.menu === id ? null : id; break;
    case "status": state.status = id; state.menu = null; tell({ t: "set_status", status: STATUS_SHARED[id] }); break;
    case "notify": if (c) setNotify(c, id); state.menu = null; break;
    case "dm": {
      const u = state.people.find((x) => x.id === Number(id));
      if (u) {
        state.wantOpen = { user: u.id };
        tell({ t: "open_direct", user: u.username });
      }
      state.q = "";
      break;
    }
    case "preview": { const f = [...state.browse, ...state.found].find((x) => x.id === Number(id)); if (f) preview(f); return; }
    case "subscribe": state.wantOpen = { id: Number(id) }; tell({ t: "subscribe", conv: Number(id) }); state.browse = state.browse.filter((f) => f.id !== Number(id)); state.q = ""; break;
    case "newchat": state.menu = null; state.dialog = { kind: id, title: "", q: "", picked: [], open: true }; break;
    case "addpeople": state.menu = null; state.dialog = { kind: "add", conv: c.id, title: "", q: "", picked: [] }; break;
    case "pick": { const u = state.people.find((x) => x.id === Number(id)); if (u && !state.dialog.picked.some((x) => x.id === u.id)) state.dialog.picked.push(u); state.dialog.q = ""; state.people = []; break; }
    case "unpick": state.dialog.picked = state.dialog.picked.filter((x) => x.id !== Number(id)); break;
    case "closedialog": state.dialog = null; break;
    case "focussearch": state.menu = null; render(); document.getElementById("search")?.focus(); return;
    case "signmode": state.signin.join = !state.signin.join; state.signin.error = ""; break;
    case "signout": signOut(); return;
    case "changename": state.dialog = { kind: "name", value: state.me.display_name || "" }; break;
    case "changeserver": state.dialog = { kind: "server", value: state.server, error: "" }; break;
    case "goto": state.view = "settings"; state.section = id; state.menu = null; break;
    case "send": return send();
    case "clearsearch": state.q = ""; break;
    case "findprev": return stepFind(-1);
    case "findnext": return stepFind(1);
    case "findclose": state.find = null; state.flash = null; break;
    case "mailmenu": {
      const r = el.getBoundingClientRect();
      state.ctx = { x: r.left - 230, y: r.bottom + 4, items: openedItems() };
      state.menu = null;
      break;
    }
    case "msgmenu": {
      const [, m] = findMsg(Number(id));
      const r = el.getBoundingClientRect();
      state.ctx = { x: r.left - 230, y: r.bottom + 4, items: msgItems(m) };
      state.menu = null;
      break;
    }
    case "run": {
      const run = menuRuns[Number(id)];
      state.ctx = null;
      state.menu = null;
      run?.();
      break;
    }
    case "picker": state.picker = state.picker === Number(id) ? null : Number(id); state.menu = null; break;
    case "react": {
      const [, m] = findMsg(Number(id));
      if (m) {
        const mine = m.reactions?.find((r) => r.emoji === el.dataset.emoji)?.users.some((u) => u.id === state.me.id);
        tell({ t: "react", conv: m.conv, seq: m.seq, emoji: el.dataset.emoji, on: !mine });
        react(m, el.dataset.emoji);
      }
      state.picker = null;
      state.ctx = null;
      break;
    }
    case "saveedit": return saveEdit();
    case "canceledit": state.editing = null; state.editText = null; break;
    case "cancelconfirm": state.confirm = null; break;
    case "doconfirm": state.confirm?.run(); state.confirm = null; break;
    case "settings": state.view = state.view === "settings" ? "chat" : "settings"; if (state.view === "chat" && c) markRead(c); state.menu = null; if (state.view === "settings") loadProofing(); break;
    case "back": state.view = "chat"; if (c) markRead(c); break;
    case "section": state.section = id; if (id === "apps") tell({ t: "list_apps" }); break;
    case "newapp": state.dialog = { kind: "app", app: { id: 0, name: "", display_name: "", description: "", webhook: "", mode: "personal" } }; break;
    case "editapp": {
      const x = state.apps.list.find((a) => a.user.id === Number(id));
      if (x) state.dialog = { kind: "app", app: { id: x.user.id, name: x.user.username, display_name: x.user.display_name, description: x.description, webhook: x.webhook, mode: x.mode } };
      break;
    }
    case "appkey": state.confirm = { title: "Make a new key?", text: "The app's current key and webhook secret stop working at once. Anything using them needs the new ones.", action: "Make a new key", primary: true, run: () => tell({ t: "new_app_key", app: Number(id) }) }; break;
    case "deleteapp": state.confirm = { title: "Delete this app?", text: "It leaves every chat and its key stops working. Its earlier messages stay.", action: "Delete", run: () => tell({ t: "delete_app", app: Number(id) }) }; break;
    case "apikey": state.confirm = { title: "Make an API key?", text: "It lets a script do anything you can do in Xenify Chat, so keep it secret. It's shown only once.", action: "Make it", primary: true, run: () => tell({ t: "new_api_key" }) }; break;
    case "hidekey": state.apps.shown = null; break;
    case "copy": navigator.clipboard?.writeText(id); showToast("Copied."); return;
    case "askapp": {
      const a = Object.values(state.appsIn).flat().find((x) => x.user.id === Number(id));
      if (a) {
        state.wantOpen = { user: a.user.id };
        tell({ t: "open_direct", user: a.user.username });
      }
      break;
    }
    case "flip": state.settings[id] = !state.settings[id]; if (["spelling", "grammar", "spotter"].includes(id)) { state.issues = {}; if (id === "spotter") setTimeout(loadProofing, 1500); } break;
    case "unword": removeWord(state.words[Number(id)]); break;
    case "unmute": { const x = conv(Number(id)); if (x) setNotify(x, "all"); break; }
    case "togglefield": state.dialog.open = !state.dialog.open; break;
  }
  render();
  if (act === "pick" || act === "unpick") document.getElementById("pickq")?.focus();
  if (act === "run" && state.editing) {
    const t = document.getElementById("edit");
    t?.focus();
    t?.setSelectionRange(t.value.length, t.value.length);
  }
});

document.addEventListener("contextmenu", (e) => {
  // Text boxes keep cut, copy and paste; everything else gets ours or none
  // (never the web view's Back / Reload / Inspect).
  if (e.target.id === "draft" || e.target.id === "mailbody" || e.target.id === "edit") {
    const found = issueAt(e.clientX, e.clientY);
    if (!found) return;
    e.preventDefault();
    state.menu = null;
    state.ctx = { x: e.clientX, y: e.clientY, items: fixItems(found) };
    return render();
  }
  if (e.target.closest("textarea, input")) return;
  e.preventDefault();
  const target = e.target.closest("[data-ctx]");
  if (!target) return;
  let items = [];
  if (target.dataset.ctx === "conv") items = convItems(conv(Number(target.dataset.id)));
  if (target.dataset.ctx === "mail") items = mailItems(target.dataset.account, target.dataset.folder, Number(target.dataset.uid));
  if (target.dataset.ctx === "draft") items = [{ label: "Open", run: () => openDraft(target.dataset.id) }, { label: "Delete draft", danger: true, run: () => deleteDraft(target.dataset.id) }];
  if (target.dataset.ctx === "msg") items = msgItems(findMsg(Number(target.dataset.msg))[1]);
  state.menu = null;
  state.ctx = { x: e.clientX, y: e.clientY, items };
  render();
});

document.addEventListener("submit", (e) => {
  const act = e.target.dataset.act;
  if (act === "signin") {
    e.preventDefault();
    return signIn();
  }
  if (act === "dialog") {
    e.preventDefault();
    return finishDialog();
  }
  if (act === "mailadd") {
    e.preventDefault();
    return addMailAccount();
  }
  if (act === "addword") {
    e.preventDefault();
    const w = document.getElementById("newword").value.trim();
    if (w) addWord(w).then(() => document.getElementById("newword")?.focus());
  }
});

document.addEventListener("input", (e) => {
  const id = e.target.id;
  if (id === "search") {
    state.q = e.target.value;
    findOnServer(state.q);
    render();
  } else if (id === "mailbody") {
    const before = state.compose.text;
    state.compose.text = e.target.value;
    shiftIssues("compose", before, e.target.value);
    sizeBox(e.target, 420);
    paintBackdrop();
    scheduleCheck();
    scheduleDraft();
  } else if (["mto", "mcc", "msubject"].includes(id)) {
    state.compose[{ mto: "to", mcc: "cc", msubject: "subject" }[id]] = e.target.value;
    scheduleDraft();
  } else if (id.startsWith("ms_")) {
    setupField(id.slice(3), e.target.value);
  } else if (id === "su") {
    state.signin.username = e.target.value;
  } else if (id === "pickq") {
    state.dialog.q = e.target.value;
    findOnServer(state.dialog.q);
  } else if (id === "dtitle") {
    state.dialog.title = e.target.value;
    const b = document.getElementById("dcreate");
    if (b) b.disabled = !e.target.value.trim();
  } else if (id === "draft") {
    const before = state.drafts[state.sel] || "";
    state.drafts[state.sel] = e.target.value;
    shiftIssues(state.sel, before, e.target.value);
    paintBackdrop();
    scheduleCheck();
    typing();
    sizeBox(e.target);
    const btn = document.querySelector(".send");
    if (btn) btn.disabled = !e.target.value.trim();
  } else if (id === "edit") {
    const before = state.editText ?? "";
    state.editText = e.target.value;
    shiftIssues(`edit:${state.editing}`, before, e.target.value);
    sizeBox(e.target);
    paintBackdrop();
    scheduleCheck();
  }
  else if (id === "size") {
    state.settings.size = Number(e.target.value);
    render();
  }
});

document.addEventListener("change", (e) => {
  if (e.target.id === "mailfolder") {
    state.mail.folder = e.target.value;
    return reloadMail();
  }
  if (e.target.id === "mfrom" && state.compose) {
    state.compose.account = e.target.value;
    return scheduleDraft();
  }
  if (e.target.id === "mfile") {
    attachFiles([...e.target.files]);
    e.target.value = "";
    return;
  }
  if (e.target.id?.startsWith("ms_")) {
    setupField(e.target.id.slice(3), e.target.value);
    // Redrawing as a text box loses focus would swallow the click that
    // caused it (the button is replaced mid-click); only lists need it.
    if (e.target.tagName === "SELECT") render();
    return;
  }
  if (e.target.name === "theme") state.settings.theme = e.target.value;
  else if (e.target.id === "quiet") state.settings.quiet = e.target.value;
  else return;
  render();
});

document.addEventListener("keydown", (e) => {
  // Ctrl+Enter sends (and saves an edit); plain Enter is a new line.
  if (e.target.closest?.(".compose") && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    sendMail();
  } else if (e.target.id === "draft" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    send();
  } else if (e.target.id === "edit" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    saveEdit();
  } else if (e.target.id === "pickq" && e.key === "Enter") {
    // Enter picks the top match (it doesn't create the group yet).
    e.preventDefault();
    document.querySelector(".pickresults .row")?.click();
  } else if (e.key === "Tab" && document.querySelector(".dialog")) {
    // Tab goes round the dialog, not out to the page behind it.
    const items = [...document.querySelectorAll(".dialog input, .dialog button:not([disabled]), .dialog select")];
    const i = items.indexOf(document.activeElement);
    const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : i === items.length - 1 ? 0 : i + 1;
    e.preventDefault();
    items[next]?.focus();
  } else if (e.key === "Escape") {
    if (state.editing) (state.editing = null), (state.editText = null);
    else if (state.confirm) state.confirm = null;
    else if (state.dialog) state.dialog = null;
    else if (state.ctx || state.menu || state.picker) (state.ctx = null), (state.menu = null), (state.picker = null);
    else if (state.find) state.find = null;
    else return;
    render();
  }
});

window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => render());
// The underlines scroll with the text.
document.addEventListener("scroll", (e) => {
  if (e.target.id === "draft") {
    const b = document.getElementById("backdrop");
    if (b) b.scrollTop = e.target.scrollTop;
  }
}, true);

window.addEventListener("focus", () => {
  const c = current();
  if (c && state.view === "chat" && unread(c)) {
    markRead(c);
    render();
  }
});

// A script mistake shows up on screen rather than leaving a button dead.
window.addEventListener("error", (e) => showToast(`Something went wrong: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => showToast(`Something went wrong: ${e.reason}`));

boot();
