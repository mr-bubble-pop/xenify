// Xenify Chat's screens. The state uses the chat server's shapes (see
// xenify-chat-proto). Signed in, it comes from the server (through the Rust
// side's `to_app` events); with XENIFY_SAMPLE=1, or in a plain browser, the
// sample chats are shown instead and changes stay on screen.

import { sample } from "./sample.js";
import { Call } from "./calls.js";
import { Recorder, BARS, clock, wavSamples, toBase64 } from "./voice.js";
import { Recording } from "./record.js";

const tauri = window.__TAURI__;

const state = {
  me: null,
  server: "",
  live: false, // talking to a server (not the sample chats)
  net: "online", // "online" | "connecting" | "offline"
  outdated: null, // { server, app }: the server is on another protocol level (see "outdated")
  conversations: [],
  messages: {}, // conv id -> [Message], oldest first
  sel: null,
  view: "loading", // "loading" | "signin" | "chat" | "settings"
  signin: { join: false, link: null, error: "", busy: false, invite: "" },
  people: [], // people on the server matching the search
  found: [], // groups and channels on the server matching the search
  browse: [], // public channels you could follow
  typing: {}, // conv id -> { user, until }
  presence: {}, // user id -> "online" | "away" | "busy" | "offline"
  kept: { q: "", list: [], sort: "best", more: "", total: 0, loading: false }, // messages the server found for the search: { msg, snippet }; the next page's key
  threads: false, // the server has threads (replies to one message, beside the chat)
  thread: null, // the thread open beside the chat: { conv, seq }
  threadMsgs: {}, // "conv:seq" -> [Message] the thread's first message and its replies, oldest first
  threadDrafts: {}, // "conv:seq" -> what you're writing in it
  ai: false, // you may ask the server's AI about your messages
  answer: null, // the AI's answer to a question: { q, busy, text, sources, error }
  due: [], // reminders showing now (the server's): { key, at, title, body, starts, link }
  info: null, // the server's /v1/info (its features: "mail"…)
  keepDays: null, // how many days the server keeps messages (null: for ever)
  autostart: false, // starts when you sign in to this computer (desktop)
  push: "", // this browser's notifications with the tab closed: "on", "off", "blocked", "unsupported"
  // Email: accounts; folders by account; the folder shown ("" every inbox,
  // "drafts", or "account\nwire"); its list; news by account.
  mail: { accounts: [], folders: {}, folder: "", list: [], status: {}, problems: {}, found: [], opened: null, loading: false, drafts: [], answered: {}, options: null },
  mailSel: null, // { account, folder, uid } the email open on the right
  compose: null, // an email being written: { to, cc, subject, text, quote, replyTo, title, busy, error }
  mailSetup: null, // adding an account in Settings
  cal: { today: [], week: [], weekStart: 0, more: false, snoozed: [] }, // the calendar (this computer); snoozed: Later
  meetings: [], // meetings you're invited to or organise (the Rust side's MeetingOut)
  meetingConvs: [], // their conversations (each meeting's call is held there)
  meetRemind: {}, // meeting conv id -> minutes before it reminds you (null: not at all)
  busy: {}, // user id -> { known, busy, work }: busy times for Find a time
  calSettings: null, // Settings › Calendar
  // Address books: which one is shown ("mine", a shared book's id,
  // "everyone" or "things"), who or what is picked, the find box, and what's in them.
  ab: { view: "mine", sel: null, q: "", list: [], books: [], directory: [] },
  // Rooms and things: what there is, whether you're an admin, bookings by id.
  things: { list: [], admin: false, byId: {} },
  away: { mine: null, of: {} }, // out of office: yours, and other people's by user id
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
  events: {}, // conv id -> [{ at, text }]: people added or gone, seen while open
  calls: {}, // conv id -> the call going on there (CallInfo)
  call: null, // the call you're in (a Call from calls.js)
  ringing: null, // a call ringing for you (CallInfo)
  declined: new Set(), // calls you said no to
  uploads: [], // files being sent: { id, conv, name, sent, total, error }
  bigPicture: null, // the message whose picture is shown big
  rec: null, // recording a voice message: { conv, r (a Recorder), sendText, where: "chat" | "call" }
  voice: { models: [], downloading: null, source: "" }, // the speech-to-text models (Rust side's voice_state)
  noAnswer: null, // a call nobody answered: { conv, name, declined }
  recording: null, // the call recording you're making: { round, conv, rec (a Recording), lines, … }
  recSave: null, // saving it after: { step: "saving" | "text", done, of }
  recAsk: null, // turning "mic", "cam" or "screen" on while someone records: asked first
  recRounds: {}, // call id → the recording round this app turned everything off for
  settings: {
    spelling: true, grammar: true, spotter: false, before: false,     desktop: true, preview: true, sound: true, mentions: true, quiet: "Off",
    digest: "mentions", meetingRemind: 10,
    theme: "system", size: 100,
    voiceModel: "better", voiceIncoming: true, voiceSendText: true, voiceLang: "", meetingTranscript: true,
  },
  words: [], // your dictionary (shared with Xenify Write)
  issues: {}, // conv id -> { text, list } what the checker found in the draft
  ignored: {}, // conv id -> keys of mistakes you chose to ignore
  spotter: "off", // the AI spotter: "on", "off", "not installed", "unavailable"
};

// ---------- Helpers ----------

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const nameOf = (u) => u.display_name || (u.app ? u.username : u.email || u.username);
/** The line under a name: an app's @name, a person's email address. */
const handleOf = (u) => (u.app ? "@" + u.username : u.email || "");
/** A sender's current name (messages keep the name from when they were sent). */
const senderName = (c, u) => nameOf(c?.members?.find((x) => x.id === u.id) || u);
const initials = (s) => s.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "?";
const palette = [["#FDE6D8", "#8A2C0A"], ["#DCE9FB", "#1D4F91"], ["#DDF3E4", "#1E6B3A"], ["#EDE4FA", "#5B3A91"], ["#FCE7F0", "#8E2557"], ["#E2F0F3", "#1D5C6B"], ["#FFF1D6", "#7A4B00"]];
const colours = (key) => palette[[...String(key)].reduce((a, c) => a + c.charCodeAt(0), 0) % palette.length];
const unread = (c) => c.last_seq > c.read_seq;
const muted = (c) => c.notify === "off" || (c.muted_until && c.muted_until > Date.now());
const others = (c) => c.members.filter((m) => m.id !== state.me.id);
const conv = (id) => state.conversations.find((c) => c.id === id) || state.meetingConvs.find((c) => c.id === id) || (state.preview?.id === id ? state.preview : undefined);
const current = () => conv(state.sel);
const findMsg = (id) => {
  for (const [cid, list] of Object.entries(state.messages)) {
    const m = list.find((x) => x.id === id);
    if (m) return [Number(cid), m];
  }
  for (const list of Object.values(state.threadMsgs)) {
    const m = list.find((x) => x.id === id);
    if (m) return [m.conv, m];
  }
  return [null, null];
};

function titleOf(c) {
  if (c.purpose === "meeting") return state.meetings.find((m) => m.conv === c.id)?.title || "Meeting";
  if (c.title) return c.title;
  if (c.kind === "direct") {
    const o = others(c);
    return o.length ? nameOf(o[0]) : "Notes to self";
  }
  return others(c).map(nameOf).join(", ") || "Just you";
}

function subOf(c) {
  if (c.kind === "channel") return `Channel · ${c.member_count} following${c.open ? " · open to everyone" : ""}`;
  if (c.kind === "group") {
    const names = others(c).map((m) => nameOf(m).split(" ")[0]);
    // A big group: a few names and how many more.
    const more = c.member_count - 1 - 3;
    if (names.length > 4 && more > 0) return `${names.slice(0, 3).join(", ")}, you and ${more} others`;
    return names.length ? names.join(", ") + " and you" : "Just you so far";
  }
  const o = others(c)[0];
  if (!o) return "Only you";
  if (o.app) return "App · answers automatically";
  const away = state.away.of[o.id];
  if (awayNow(away)) return `Away until ${awayUntil(away)}`;
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
  return c.last.deleted ? who + "deleted a message" : who + describe(c.last.text);
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
  secret: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="7.5" cy="15.5" r="4.5"/><path d="m10.7 12.3 9.8-9.8M17 6l3 3M14.5 8.5l2.5 2.5"/></svg>`,
  recdot: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="4" fill="#B42318" stroke="none"/></svg>`,
  screen: `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="13" rx="2"/><path d="M8 21h8M12 17v4"/></svg>`,
  play: `<svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z" fill="currentColor"/></svg>`,
  pause: `<svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 5h4v14H7zM13 5h4v14h-4z" fill="currentColor"/></svg>`,
  trash: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14"/></svg>`,
  plane: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17.8 19.2 16 11l3.5-3.5C21 6 21.5 4 21 3c-1-.5-3 0-4.5 1.5L13 8 4.8 6.2c-.5-.1-.9.1-1.1.5l-.3.5c-.2.5-.1 1 .3 1.3L9 12l-2 3H4l-1 1 3 2 2 3 1-1v-3l3-2 3.5 5.3c.3.4.8.5 1.3.3l.5-.2c.4-.3.6-.7.5-1.2z"/></svg>`,
  // The Xenify mark: an X on an ember square.
  phone: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 2.1 4.2 2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7c.1.9.4 1.8.7 2.7a2 2 0 0 1-.5 2.1L8 9.8a16 16 0 0 0 6 6l1.3-1.3a2 2 0 0 1 2.1-.4c.9.3 1.8.6 2.7.7a2 2 0 0 1 1.7 2z"/></svg>`,
  video: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="6" width="14" height="12" rx="2"/><path d="m16 10 6-3v10l-6-3"/></svg>`,
  videoOff: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2 2l20 20M16 16v1a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2m4 0h4a2 2 0 0 1 2 2v3.3l1 1L22 7v10"/></svg>`,
  mic: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v3"/></svg>`,
  micOff: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2 2l20 20M9 9v3a3 3 0 0 0 5.1 2.1M15 9.3V5a3 3 0 0 0-5.9-.6M17 16.9A7 7 0 0 1 5 12v-2M19 10v2a7 7 0 0 1-.1 1.2M12 19v3"/></svg>`,
  share: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4M9 10l3-3 3 3M12 7v7"/></svg>`,
  board: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4M7 12l3-3 3 3 4-4"/></svg>`,
  back: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"/></svg>`,
  chat: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>`,
  calendar: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/></svg>`,
  open: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 3h7v7M10 14 21 3M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5"/></svg>`,
  full: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3"/></svg>`,
  clip: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21 12-8.6 8.6a5 5 0 0 1-7-7l8.6-8.6a3.3 3.3 0 0 1 4.7 4.7l-8.6 8.6a1.7 1.7 0 0 1-2.4-2.4L16 8"/></svg>`,
  download: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12m0 0-4-4m4 4 4-4M4 21h16"/></svg>`,
  search: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--muted)" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>`,
  channel: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 11v2a1 1 0 0 0 1 1h2l5 4V6L6 10H4a1 1 0 0 0-1 1z"/><path d="M15 9a4 4 0 0 1 0 6"/><path d="M18 6a8 8 0 0 1 0 12"/></svg>`,
  bell: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.9 1.9 0 0 0 3.4 0"/></svg>`,
  bellOff: (size = 20) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M13.7 21a2 2 0 0 1-3.4 0"/><path d="M18.6 13A17.9 17.9 0 0 1 18 8"/><path d="M6.3 6.3A5.9 5.9 0 0 0 6 8c0 7-3 9-3 9h14"/><path d="M18 8a6 6 0 0 0-9.3-5"/><path d="m2 2 20 20"/></svg>`,
  more: (size = 20) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true"><path d="M12 5h.01M12 12h.01M12 19h.01"/></svg>`,
  addPerson: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M19 8v6M22 11h-6"/></svg>`,
  people: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.9"/><path d="M16 3.1a4 4 0 0 1 0 7.8"/></svg>`,
  gear: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>`,
  smile: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M8.5 14a4 4 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/><path d="M19 3v4M17 5h4"/></svg>`,
  mail: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/></svg>`,
  thread: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"/><path d="M8.5 10.5h7M8.5 13.5h4"/></svg>`,
  reply: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 6 6v4"/></svg>`,
  forward: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0-6 6v4"/></svg>`,
  plus: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>`,
  clip: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21.4 11.1-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/></svg>`,
  check: (colour) => `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="${colour}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>`,
  send: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m22 2-7 20-4-9-9-4z"/><path d="M22 2 11 13"/></svg>`,
  up: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m18 15-6-6-6 6"/></svg>`,
  down: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>`,
  spark: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/></svg>`,
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
    return `<button class="reaction${mine ? " own" : ""}" data-act="react" data-id="${m.id}" data-emoji="${esc(r.emoji)}" title="${esc(who)}" aria-pressed="${mine}" aria-label="${esc(reactionName(r.emoji))} from ${esc(who)}${mine ? ". Take yours back" : ". Add yours"}"><span aria-hidden="true">${esc(r.emoji)}</span>${r.users.length}</button>`;
  }).join("")}</div>`;
}

function reactionPicker(id) {
  return `<div class="picker" role="group" aria-label="React">${REACTIONS.map(([e, n]) => `<button data-act="react" data-id="${id}" data-emoji="${e}" aria-label="React with ${n}" title="${n}">${e}</button>`).join("")}</div>`;
}


// ---------- Search ----------

/** Every message, in every chat, containing `q`: found by the server when
 * signed in (best first), else among those loaded. */
function messageHits(q) {
  if (state.live) {
    if (state.kept.q !== q.trim()) return [];
    return state.kept.list.map((h) => ({ c: conv(h.msg.conv), m: h.msg, snippet: h.snippet })).filter((h) => h.c);
  }
  const hits = [];
  const needle = q.toLowerCase();
  for (const c of state.conversations) {
    for (const m of state.messages[c.id] || []) {
      if (!m.deleted && searchedText(m.text).toLowerCase().includes(needle)) hits.push({ c, m });
    }
  }
  return hits.sort((a, b) => b.m.at - a.m.at);
}

function searchResults() {
  const q = state.q.trim();
  const chats = state.conversations.filter((c) => titleOf(c).toLowerCase().includes(q.toLowerCase()));
  const hits = messageHits(q);
  const chatRows = chats.map((c) => `<button class="row compact" data-act="open" data-id="${c.id}">${avatarFor(c, "small")}<span class="body"><span class="name">${marked(titleOf(c), q)}</span><span class="last">${esc(subOf(c))}</span></span></button>`).join("");
  const hitRows = hits.map(({ c, m, snippet: found }) => `<button class="hit" data-act="hit" data-id="${m.id}">
      <span class="where"><b>${esc(titleOf(c))}</b>${m.sender.id === state.me.id ? " · You" : c.kind === "direct" ? "" : ` · ${esc(senderName(c, m.sender))}`} · ${esc(timeOf(m.at))}</span>
      <span class="text">${hitText(m.text, found, q)}</span>
    </button>`).join("");
  const looking = state.live && state.kept.q !== q;
  const k = state.kept;
  const count = state.live ? (k.total > 1000 ? "1,000+" : k.total ? String(k.total) : "") : hits.length ? String(hits.length) : "";
  const sortBox = state.live && hits.length > 1 ? `<label class="sortbox"><span class="sr">Order</span><select id="hitsort">${[["best", "Best match"], ["newest", "Newest"], ["oldest", "Oldest"]].map(([v, n]) => `<option value="${v}" ${k.sort === v ? "selected" : ""}>${n}</option>`).join("")}</select></label>` : "";
  const more = state.live && k.q === q && k.more ? `<button class="morehits" data-act="morehits" ${k.loading ? "disabled" : ""}>${k.loading ? "Loading…" : "More messages"}</button>` : "";
  return `<div class="results">
    ${chats.length ? `<div class="section">Chats</div>${chatRows}` : ""}
    ${peopleResults(q, chats)}
    ${askBox(q)}
    <div class="section"><span>Messages${count ? ` · ${count}` : ""}</span>${state.live ? sortBox : `<span class="note">Only those loaded</span>`}</div>
    ${hitRows || `<div class="empty">${looking ? "Looking…" : `No messages with “${esc(q)}”.`}</div>`}
    ${more}
    ${hasMail() ? `<div class="section"><span>Email${state.mail.found.length ? ` · ${state.mail.found.length}` : ""}</span><span class="note">${serverEmail() ? "" : "Searched on this computer"}</span></div>${state.mail.found.map(mailRow).join("") || `<div class="empty">No email with “${esc(q)}”.</div>`}` : ""}
  </div>`;
}

/** Search › Ask AI: a question about your messages, answered by the
 * server's AI from messages you may read (and nothing else). */
function askBox(q) {
  if (!state.live || !state.ai) return "";
  const a = state.answer?.q === q ? state.answer : null;
  if (!a) return `<button class="askai" data-act="askai">${icons.spark}<span><b>Ask AI</b><span class="hintline">Answers “${esc(q)}” from your messages</span></span></button>`;
  if (a.busy) return `<div class="answer" role="status">${icons.spark}<span class="hintline">Looking through your messages…</span></div>`;
  if (a.error) return `<div class="answer" role="alert">${icons.spark}<span>${esc(a.error)}</span></div>`;
  // [1], [2]… link to the messages it used.
  const cited = [];
  const text = esc(a.text).replace(/\[(\d+)\]/g, (all, n) => {
    const src = a.sources[Number(n) - 1];
    if (!src) return all;
    if (!cited.includes(src)) cited.push(src);
    return `<button class="cite" data-act="hit" data-id="${src.msg.id}" aria-label="Message ${n}">${n}</button>`;
  });
  const rows = cited.map((h) => {
    const c = conv(h.msg.conv);
    if (!c) return "";
    const n = a.sources.indexOf(h) + 1;
    return `<button class="hit" data-act="hit" data-id="${h.msg.id}"><span class="where"><b>${n}</b> · <b>${esc(titleOf(c))}</b>${h.msg.sender.id === state.me.id ? " · You" : c.kind === "direct" ? "" : ` · ${esc(senderName(c, h.msg.sender))}`} · ${esc(timeOf(h.msg.at))}</span><span class="text">${esc(describe(h.msg.text).slice(0, 160))}</span></button>`;
  }).join("");
  return `<div class="answer">${icons.spark}<div class="body"><div class="text">${text}</div><div class="hintline">From your messages only. AI can get things wrong: check the messages.</div></div></div>${rows}`;
}

/** What search looks in (as the server does): a file's name (and what a
 * voice message says), a whiteboard's title, nothing of other posts apps
 * keep in chats, or the text. */
function searchedText(text) {
  const f = fileOf(text);
  if (f) return [f.name, f.voice?.text].filter(Boolean).join(" ");
  const b = boardOf(text);
  if (b) return b.title || "Whiteboard";
  return /^~[a-z]+[0-9]*:[{[]/.test(text) ? "" : text;
}

/** A found message as HTML: files by their names, whiteboards by their
 * titles, the server's snippet when there is one. */
function hitText(text, found, q) {
  const f = fileOf(text);
  const b = !f && boardOf(text);
  const shown = searchedText(text);
  const label = f ? `${{ voice: "Voice message", recording: "Recording", picture: "Picture", video: "Clip" }[fileKind(f)] || "File"}: ` : b ? "Whiteboard: " : "";
  // The server marks whole words; a part of one (report_final.pdf) is marked here.
  const marked = found !== undefined && found.includes("\u0001");
  return esc(label) + (marked ? serverSnippet(shown, found) : snippet(shown, q.trim().split(/\s+/)[0] || q));
}

/** The server's snippet (matches between \u0001 and \u0002) as HTML, with
 * "…" where the message goes on. */
function serverSnippet(text, found) {
  const plain = found.replace(/[\u0001\u0002]/g, "");
  const html = esc(found).replace(/\u0001/g, "<mark>").replace(/\u0002/g, "</mark>");
  return (text.startsWith(plain) ? "" : "…") + html + (text.endsWith(plain) ? "" : "…");
}

/** People, and channels you're not in, found on the server. */
function peopleResults(q, chats) {
  if (!state.live) return "";
  // Someone you already have a direct chat with shows under Chats.
  const talking = new Set(chats.filter((c) => c.kind === "direct").flatMap((c) => others(c).map((u) => u.id)));
  const people = state.people.filter((u) => !talking.has(u.id)).map((u) => `<button class="row compact" data-act="dm" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${marked(nameOf(u), q)}${u.app ? ` <span class="appbadge">APP</span>` : ""}</span><span class="last">${esc(handleOf(u))}</span></span></button>`).join("");
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
  const found = (state.kept.list.find((x) => x.msg.id === id) || state.answer?.sources?.find((x) => x.msg.id === id))?.msg;
  // A reply: its thread opens beside the chat.
  if (found?.thread != null && state.threads) {
    open(found.conv, { keepSearch: true });
    await openThread({ conv: found.conv, seq: found.thread }, false);
    state.flash = id;
    render();
    document.querySelector(`#threadmsgs [data-msg="${id}"]`)?.scrollIntoView({ block: "center" });
    return;
  }
  if (!findMsg(id)[1]) {
    // An older message: bring in it and everything after it from the server.
    const hit = found;
    if (!hit) return;
    if (!state.messages[hit.conv]) await historyBefore(hit.conv, null, 100);
    for (let i = 0; i < 50; i++) {
      const first = (state.messages[hit.conv] || []).find((x) => !x.pending);
      if (!first || first.seq <= Math.max(1, hit.seq - 20)) break;
      await historyBefore(hit.conv, first.seq, 200);
    }
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

/** Asks for messages before `before` and waits for them (or 10 seconds). */
const historyWaits = new Map();
function historyBefore(convId, before, limit) {
  const tag = `jump${Math.random().toString(36).slice(2)}`;
  return new Promise((done) => {
    historyWaits.set(tag, done);
    setTimeout(() => historyWaits.delete(tag) && done(), 10_000);
    tell({ t: "history", conv: convId, before, limit, tag });
  });
}

function scrollToMessage(id) {
  const el = document.querySelector(`[data-msg="${id}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}

// ---------- Drawing ----------

/** A chat or group with no messages yet shows only to whoever started it
 * (and while open); the others see it when the first message arrives. */
const listed = (c) => c.kind === "channel" || c.last || c.id === state.sel || (c.kind === "group" && c.role === "owner");

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
  // Email: the server's when it keeps it, else the desktop app's own.
  const mailOn = !!tauri && (!window.XENIFY_WEB || serverEmail());
  const tabs = [["all", "All"], ["chats", "Chats"], ["channels", "Channels"], ...(mailOn ? [["mail", "Mail"]] : []), ...(tauri ? [["later", "Later"]] : [])].map(([id, label]) => {
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
    ? `<div class="empty">Nothing here. Use “Remind me…” on a chat, a message or an email (right-click it) to put it here until later.</div>`
    : state.tab === "mail"
    ? mailEmpty()
    : state.tab === "channels"
      ? (state.browse.length ? "" : `<div class="empty">No channels on the server yet.${can("chat.create_channel") ? " Press New channel to start one." : ""}</div>`)
      : `<div class="empty">No chats yet. Search for someone to message them; to talk with several people, add more people to a chat.</div>`;
  return `<nav class="side" aria-label="Chats, channels and email">
    <div class="brand">Xenify Chat${state.live && state.net !== "online" ? `<span class="netstate" role="status">${state.net === "connecting" ? "Connecting…" : "Offline · trying again"}</span>` : ""}${state.outdated ? (state.outdated.server > state.outdated.app ? `<span class="netstate" role="status" title="This server is newer than this Xenify Chat. Some things may not work until you update.">Update Chat</span>` : `<span class="netstate" role="status" title="This server is older than this Xenify Chat. Some things may not work until the server is updated.">Server needs updating</span>`) : ""}</div>
    ${callStrip(true)}
    ${viewSwitch()}
    <div class="searchrow">
      <label class="search"><span class="sr">Search chats, email and messages, or find someone to message</span>${icons.search}<input id="search" placeholder="Search or find someone" value="${esc(state.q)}" autocomplete="off">${searching ? `<button class="iconbtn tiny" data-act="clearsearch" aria-label="Clear search">${icons.close}</button>` : ""}</label>
    </div>
    ${searching ? searchResults() : `<div class="tabrow"><div class="tabs" role="tablist" aria-label="Show">${tabs}</div></div>${state.tab === "later" ? "" : invitesCard() + requestsCard() + todayPanel()}${state.tab === "mail" ? mailBar() : ""}<div class="list">${state.tab === "channels" ? channelsHead() : ""}${rows || listEmpty}${state.tab === "channels" ? browseList() : ""}</div>`}
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

/** Whether the server's roles let you do `p` ("chat.create_channel"…).
 * Sample chats (not signed in) may do everything. */
const can = (p) => !state.live || (state.perms || []).includes(p);

/** On a phone the list and what's open take turns: this goes back to the list. */
const phoneBack = (act = "phoneback") => `<button class="iconbtn phoneback" data-act="${act}" aria-label="Back">${icons.back}</button>`;

/** What makes a phone show the sheet instead of the list. */
const SHOWS_SHEET = new Set(["open", "preview", "hit", "dm", "mail", "newmail", "calendar", "people", "section", "goto", "abmessage", "abemail", "subscribe"]);

/** Chats, Calendar and Address books: where the sheet beside the list is. */
function viewSwitch() {
  if (!tauri) return "";
  const views = [["chat", "Chats", icons.chat], ["calendar", "Calendar", icons.calendar], ...(state.live ? [["people", "Contacts", icons.people]] : [])];
  const at = state.view === "calendar" || state.view === "people" ? state.view : "chat";
  return `<div class="viewswitch" role="tablist" aria-label="Show">${views.map(([id, label, icon]) => `<button role="tab" aria-selected="${at === id}" data-act="${id === "chat" ? "chats" : id}">${icon}<span>${label}</span></button>`).join("")}</div>`;
}

/** The Channels tab's heading: making a channel lives here. */
function channelsHead() {
  if (!can("chat.create_channel")) return "";
  return `<div class="section listhead"><span>Channels</span><button class="textbtn small-text" data-act="newchat" data-id="channel" title="Announcements and alerts that people follow">${icons.plus}New channel</button></div>`;
}

function statusMenu() {
  const items = Object.entries(STATUS).map(([id, s]) => `<button role="menuitemradio" aria-checked="${state.status === id}" data-act="status" data-id="${id}">
    <span style="display:flex;align-items:center;gap:10px"><span style="width:10px;height:10px;border-radius:50%;background:${s.dot}"></span><span style="display:flex;flex-direction:column"><span style="font-weight:600">${s.label}</span><span class="hintline">${s.hint}</span></span></span>
  </button>`).join("");
  const away = state.live && tauri ? `<div class="sep"></div><button role="menuitem" data-act="awayopen"><span style="display:flex;flex-direction:column"><span style="font-weight:600">Out of office…</span><span class="hintline">${awayNow(state.away.mine) ? `On · until ${esc(awayUntil(state.away.mine))}` : "Times away, an automatic reply"}</span></span></button>` : "";
  const quit = onServerPage ? `<div class="sep"></div><button role="menuitem" data-act="quit"><span style="font-weight:600">Quit</span></button>` : "";
  return `<div class="menu" role="menu" aria-label="Your status" style="left:4px;right:4px;width:auto;bottom:60px">${items}${away}${quit}</div>`;
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
  const f = fileOf(m.text);
  const board = boardOf(m.text);
  if (board) return `<div class="msg">${replyTo(m)}<div class="bubblewrap" data-msg="${m.id}" data-ctx="msg"><div class="filewrap${pending}">${boardCard(m, board)}</div></div>${reactionRow(m)}${threadLine(m)}</div>`;
  return `<div class="msg">
    ${replyTo(m)}
    <div class="bubblewrap" data-msg="${m.id}" data-ctx="msg">
      ${f ? `<div class="filewrap${pending}">${fileCard(m, f)}</div>` : `<div class="bubble${pending}${flash}">${marked(m.text, findQ)}`}${f ? "" : `${m.edited && !m.sender.app ? `<span class="edited"> · edited</span>` : ""}</div>`}
      <span class="msgtools${picking ? " open" : ""}">
        <button class="msgmore" data-act="picker" data-id="${m.id}" aria-label="React" aria-expanded="${picking}">${icons.smile}</button>
        ${canThread(m) ? `<button class="msgmore" data-act="thread" data-id="${m.id}" aria-label="Reply in thread" title="Reply in thread">${icons.thread}</button>` : ""}
        <button class="msgmore" data-act="msgmenu" data-id="${m.id}" aria-label="Message options">${icons.more(16)}</button>
      </span>
      ${picking ? reactionPicker(m.id) : ""}
    </div>
    ${reactionRow(m)}
    ${threadLine(m)}
  </div>`;
}

/** Whether you can reply to `m` in its thread (in the chat itself, sent,
 * and in a chat you can write in). */
function canThread(m) {
  if (!state.threads || m.thread != null || m.pending || m.deleted) return false;
  const c = conv(m.conv);
  return !!c && !c.preview && c.can_post && !c.purpose;
}

/** Under a message with replies: how many, when the last came, and
 * whether any are new since you last read the chat. */
function threadLine(m) {
  if (!state.threads || m.thread != null || !m.replies || inPane) return "";
  const open = state.thread?.conv === m.conv && state.thread.seq === m.seq;
  const line = state.readLine[m.conv];
  const fresh = line != null && m.last_reply_seq > line;
  const n = m.replies === 1 ? "1 reply" : `${m.replies} replies`;
  return `<button class="threadlink${open ? " open" : ""}" data-act="thread" data-id="${m.id}" aria-label="${n}${fresh ? ", new" : ""}: open the thread">${icons.thread}<b>${n}</b><span>${esc(timeOf(m.last_reply_at))}</span>${fresh ? `<span class="new">New</span>` : ""}</button>`;
}

/** Threads turned off: a reply shows in the chat, under what it answers. */
function replyTo(m) {
  if (m.thread == null || state.threads) return "";
  const root = (state.messages[m.conv] || []).find((x) => x.seq === m.thread);
  if (!root) return `<div class="replyto">${icons.reply}In reply to an earlier message</div>`;
  const c = conv(m.conv);
  const text = root.deleted ? "a deleted message" : describe(root.text);
  return `<div class="replyto">${icons.reply}${esc(senderName(c, root.sender))}: ${esc(text.length > 80 ? text.slice(0, 80) + "…" : text)}</div>`;
}

function messageList(c) {
  const msgs = state.messages[c.id] || [];
  const line = state.readLine[c.id];
  const findQ = state.find?.q || "";
  const out = [];
  let last = null;
  let day = null;
  const events = [...(state.events[c.id] || [])];
  msgs.forEach((m, i) => {
    while (events.length && events[0].at <= m.at) {
      out.push(`<div class="sysline">${esc(events.shift().text)}</div>`);
      last = null;
    }
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
    } else {
      const group = { sender: m.sender.id, mine, head: m, el: [bubble(m, findQ)] };
      out.push(group);
      last = group;
    }
    // Someone away: their automatic reply follows your first message.
    const auto = mine ? autoReply(c, m) : "";
    if (auto) {
      out.push(auto);
      last = null;
    }
  });
  for (const ev of events) out.push(`<div class="sysline">${esc(ev.text)}</div>`);
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
  const recording = state.rec?.conv === c.id && state.rec.where === "chat";
  const composer = recording && c.can_post ? `<div class="composer">${recBar()}${canMakeText() ? `<div class="hint">Your voice is turned into text on this computer as you talk.</div>` : ""}</div>` : c.preview ? `<div class="followbar"><span>You're looking at <b>${esc(titleOf(c))}</b> without following it. Follow it to get its posts.</span><button class="small primary" data-act="subscribe" data-id="${c.id}">Follow</button></div>` : c.can_post ? `<div class="composer">
      <div class="box">
        ${uploadChips(c)}
        <label class="sr" for="draft">Message</label>
        <div class="fieldwrap">
          <div class="backdrop" id="backdrop" aria-hidden="true">${underlined(draft, issuesFor(c.id, draft))}</div>
          <textarea id="draft" rows="1" placeholder="Write to ${esc(titleOf(c))}" aria-describedby="checkstate">${esc(draft)}</textarea>
        </div>
        <div class="tools">
          <div class="left">${tauri ? `<button class="iconbtn" data-act="attach" aria-label="Send files, pictures or videos" title="Send files, pictures or videos (or drop them here)">${icons.clip}</button><input type="file" id="filepick" multiple hidden><button class="iconbtn" data-act="newboard" aria-label="New whiteboard" title="New whiteboard: draw and make diagrams together">${icons.board}</button><button class="iconbtn" data-act="voicerec" aria-label="Record a voice message" title="Record a voice message">${icons.mic}</button>` : ""}${state.live ? `<button class="iconbtn" data-act="secret" data-id="chat" aria-label="Share a secret" title="Share a secret: a password or the like, as a link that opens once">${icons.secret}</button>` : ""}</div>
          <button class="send" data-act="send" title="Send (Ctrl+Enter)" ${draft.trim() ? "" : "disabled"}>Send ${icons.send}</button>
        </div>
      </div>
      <div class="hint" id="checkstate">${typingLine(c) || checkLine(c.id, draft)}</div>
    </div>` : `<div class="hint" style="padding:12px 24px 18px;border-top:1px solid var(--line)">Only the channel's owners post here.</div>`;
  return `<main class="sheet">
    <header class="sheet-head">
      ${phoneBack()}<div class="title">${avatarFor(c, "big")}<div><div class="titleline"><h1>${esc(titleOf(c))}</h1></div><div class="sub">${esc(subOf(c))}</div></div></div>
      ${c.preview ? "" : `<div class="actions">
        <button class="textbtn" data-act="menu" data-id="notify" aria-label="Notifications for this chat: ${short}" style="${isMuted ? "background:var(--tint)" : ""}">${isMuted ? icons.bellOff() : icons.bell}${short}</button>
        ${tauri && c.kind !== "channel" && !c.preview && others(c).some((u) => !u.app) ? `<button class="iconbtn" data-act="callstart" data-id="voice" aria-label="Voice call" title="Voice call">${icons.phone}</button><button class="iconbtn" data-act="callstart" data-id="video" aria-label="Video call" title="Video call">${icons.video}</button>` : ""}
        ${c.kind === "direct" ? (others(c).some((u) => !u.app) ? `<button class="iconbtn" data-act="addpeople" aria-label="Add people: a group chat with ${esc(titleOf(c))} and others" title="Add people (starts a group chat)">${icons.addPerson}</button>` : "") : `<button class="iconbtn" data-act="menu" data-id="people" aria-label="${c.kind === "channel" ? "About this channel" : "People in this chat"}">${icons.people}</button>`}
        <button class="iconbtn" data-act="menu" data-id="more" aria-label="More options">${icons.more()}</button>
      </div>`}
    </header>
    ${callBar(c)}${askPrivatelyBar(c)}${findBar}${notifyMenu}${moreMenu}${state.menu === "people" ? peopleMenu(c) : ""}
    <div class="messages" id="messages">${messageList(c)}</div>
    ${awayNotice(c)}${composer}
  </main>${threadPane(c)}`;
}

// Drawing the thread pane (its first message doesn't link to itself).
let inPane = false;

/** The thread open beside the chat: its first message, the replies, and
 * a box to reply. */
function threadPane(c) {
  inPane = true;
  try {
    return threadPaneHtml(c);
  } finally {
    inPane = false;
  }
}

function threadPaneHtml(c) {
  const t = state.thread;
  if (!t || t.conv !== c.id) return "";
  const key = `${t.conv}:${t.seq}`;
  const list = state.threadMsgs[key] || [];
  const root = list.find((m) => m.seq === t.seq) || (state.messages[c.id] || []).find((m) => m.seq === t.seq);
  const replies = list.filter((m) => m.thread === t.seq);
  const one = (m) => {
    const mine = m.sender.id === state.me.id;
    const t2 = esc(timeOf(m.at));
    if (mine) return `<div class="mine"><span class="meta"><span class="time">${t2}</span></span>${bubble(m, "")}</div>`;
    const name = senderName(c, m.sender);
    const [bg, fg] = colours(m.sender.id + name);
    return `<div class="theirs"><span class="avatar small" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(name))}</span>
      <div class="stack"><div class="meta"><b>${esc(name)}</b>${m.sender.app ? `<span class="appbadge">APP</span>` : ""}<span class="time">${t2}</span></div>${bubble(m, "")}</div></div>`;
  };
  const draft = state.threadDrafts[key] || "";
  const n = replies.length === 1 ? "1 reply" : `${replies.length} replies`;
  const box = c.can_post ? `<div class="composer">
      <div class="box">
        <label class="sr" for="tdraft">Reply in the thread</label>
        <textarea id="tdraft" rows="1" placeholder="Reply…">${esc(draft)}</textarea>
        <div class="tools"><div class="left"></div><button class="send" data-act="threadsend" title="Send (Ctrl+Enter)" ${draft.trim() ? "" : "disabled"}>Reply ${icons.send}</button></div>
      </div>
      <div class="hint">Only people in this thread are told about replies, and anyone you mention.</div>
    </div>` : "";
  return `<aside class="sheet threadpane" aria-label="Thread">
    <header class="sheet-head"><div class="title"><div><h1>Thread</h1><div class="sub">${esc(titleOf(c))}</div></div></div>
      <button class="iconbtn" data-act="threadclose" aria-label="Close the thread" title="Close (Esc)">${icons.close}</button></header>
    <div class="messages" id="threadmsgs">
      ${root ? one(root) : `<div class="empty">Loading…</div>`}
      ${root ? `<div class="divider">${replies.length ? n : "No replies yet"}</div>` : ""}
      ${replies.map(one).join("")}
    </div>
    ${box}
  </aside>`;
}

/** Opens message `m`'s thread beside the chat. */
function openThread(m, focus = true) {
  state.thread = { conv: m.conv, seq: m.seq };
  state.menu = null;
  state.picker = null;
  const tag = `thread${Math.random().toString(36).slice(2)}`;
  const loaded = new Promise((done) => {
    historyWaits.set(tag, done);
    setTimeout(() => historyWaits.delete(tag) && done(), 10_000);
  });
  tell({ t: "thread", conv: m.conv, thread: m.seq, tag });
  render();
  if (focus) document.getElementById("tdraft")?.focus();
  return loaded;
}

/** Sends the reply written in the open thread. */
function sendReply() {
  const t = state.thread;
  const c = t && conv(t.conv);
  if (!c) return;
  const key = `${t.conv}:${t.seq}`;
  const text = (state.threadDrafts[key] || "").trim();
  if (!text) return;
  const client_id = crypto.randomUUID();
  const msg = { id: -Date.now(), conv: c.id, seq: c.last_seq + 1, thread: t.seq, sender: state.me, text, at: Date.now(), client_id, edited: false, deleted: false, pending: true };
  (state.threadMsgs[key] ||= []).push(msg);
  tell({ t: "reply", conv: c.id, thread: t.seq, client_id, text });
  state.threadDrafts[key] = "";
  render();
  document.getElementById("tdraft")?.focus();
}

function typingLine(c) {
  const t = state.typing[c.id];
  return t && t.until > Date.now() ? `<span class="typing">${esc(nameOf(t.user).split(" ")[0])} is typing…</span>` : "";
}

function personRow(u) {
  return `<div class="person">${personAvatar(u, "small")}<span class="body"><b>${esc(u.id === state.me.id ? nameOf(u) + " (you)" : nameOf(u))}${u.app ? `<span class="appbadge">APP</span>` : ""}</b><span class="hintline">${esc(handleOf(u))}</span></span></div>`;
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
      <div class="people">${posters.map((u) => c.role === "owner" && u.id !== state.me.id && c.owners?.includes(u.id) ? personRow(u).replace(/<\/div>$/, `<button class="small quiet" data-act="setowner" data-id="${u.id}" data-on="">Stop posting</button></div>`) : personRow(u)).join("")}</div>
      ${invite ? `<button role="menuitem" class="withicon" data-act="addpeople">${icons.plus}${c.open ? "Add an app or people…" : "Add people or apps…"}</button>
      <button role="menuitem" class="withicon" data-act="addposter">${icons.plus}Let someone post…</button>` : ""}
    </div>`;
  }
  const owner = c.role === "owner";
  const takeOut = owner || can("chat.moderate");
  const row = (u) => {
    const isOwner = c.owners?.includes(u.id);
    const buttons = [
      owner && u.id !== state.me.id && !u.app ? `<button class="small quiet" data-act="setowner" data-id="${u.id}" data-on="${isOwner ? "" : "1"}">${isOwner ? "Not owner" : "Make owner"}</button>` : "",
      takeOut && u.id !== state.me.id ? `<button class="small quiet" data-act="takeout" data-id="${u.id}" aria-label="Take ${esc(nameOf(u))} out of this chat">Take out</button>` : "",
    ].join("");
    const r = isOwner ? personRow(u).replace(/(<span class="hintline">)/, `$1Owner · `) : personRow(u);
    return buttons ? r.replace(/<\/div>$/, `${buttons}</div>`) : r;
  };
  return `<div class="menu" role="dialog" aria-label="People in ${esc(titleOf(c))}" style="right:60px;top:66px;width:${owner ? 440 : 320}px">
    <div class="label">In this chat · ${c.member_count}</div>
    <div class="people">${c.members.map(row).join("")}</div>
    <button role="menuitem" class="withicon" data-act="addpeople">${icons.plus}Add people…</button>
  </div>`;
}

// ---------- How long messages are kept ----------

const KEEP = [[null, "As long as the server keeps them"], [7, "1 week"], [30, "30 days"], [90, "90 days"], [365, "1 year"]];
const keepWords = (days) => {
  const own = days ?? null;
  const server = state.keepDays;
  const d = own !== null && (server === null || own < server) ? own : server;
  if (d === null) return "For ever";
  return KEEP.find(([v]) => v === d)?.[1] || `${d} days`;
};

/** Owners of a group or channel: keep messages less long than the server. */
function keepDialog(d) {
  const c = conv(d.conv);
  const server = state.keepDays;
  const choices = KEEP.filter(([v]) => v === null || server === null || v < server);
  return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Keep messages">
    <h2>Keep messages</h2>
    <p>How long ${esc(titleOf(c))} keeps its messages, files and recordings. Older ones are deleted for everyone.</p>
    <div class="keepchoice">${choices.map(([v, l]) => `<label class="choice${d.days === v ? " on" : ""}"><input type="radio" name="keep" value="${v ?? ""}" ${d.days === v ? "checked" : ""}> ${v === null ? `${l}${server === null ? " (for ever)" : ` (${keepWords(null).toLowerCase()})`}` : l}</label>`).join("")}</div>
    ${server !== null ? `<p class="hintline">This server deletes messages after ${esc(keepWords(null).toLowerCase())}: chats can only choose less.</p>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Save</button></div>
  </form></div>`;
}

// ---------- Right-click menus ----------

function convItems(c) {
  const items = [];
  if (unread(c)) items.push({ label: "Mark as read", run: () => markRead(c) });
  else if (c.last_seq) items.push({ label: "Mark as unread", hint: "From the last message", run: () => markUnread(c, c.last_seq - 1) });
  if (tauri) items.push({ label: "Remind me about this chat…", run: () => remindAbout(chatAbout(c, null)) });
  items.push({ label: "Notification settings", hint: NOTIFY.find((n) => n.id === notifyLevel(c)).short, run: () => ((state.sel = c.id), (state.view = "chat"), (state.menu = "notify")) });
  if (c.kind !== "direct" && state.live && (c.kind === "group" || c.role === "owner" || can("chat.moderate"))) {
    items.push({ label: "Rename…", run: () => (state.dialog = { kind: "rename", conv: c.id, value: c.title }) });
  }
  if (c.kind !== "direct" && c.role === "owner" && state.live) {
    items.push({ label: "Keep messages…", hint: keepWords(c.keep_days ?? null), run: () => (state.dialog = { kind: "keep", conv: c.id, days: c.keep_days ?? null }) });
  }
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
  if (canThread(m)) items.push({ label: m.replies ? "Open thread" : "Reply in thread", run: () => openThread(m) });
  items.push({ label: "Copy text", run: () => navigator.clipboard?.writeText(m.text) });
  if (tauri && !m.deleted && !m.pending) items.push({ label: "Remind me…", hint: "Later today, tomorrow, next week…", run: () => remindAbout(chatAbout(c, m)) });
  items.push({
    label: "Mark unread from here",
    hint: "The new line goes above this message",
    run: () => markUnread(c, m.seq - 1),
  });
  const mine = m.sender.id === state.me.id;
  if (!mine && !m.deleted && c && c.kind !== "direct" && state.live && (c.role === "owner" || can("chat.moderate"))) {
    items.push({
      label: "Take down",
      hint: c.role === "owner" ? "You're an owner here" : "You moderate this server",
      danger: true,
      run: () => {
        state.confirm = {
          title: `Take down ${senderName(c, m.sender)}'s message?`,
          text: "Everyone in the chat will see “This message was deleted.” instead.",
          action: "Take down",
          run: () => {
            tell({ t: "delete", conv: m.conv, seq: m.seq });
            Object.assign(m, { deleted: true, text: "", reactions: [] });
          },
        };
      },
    });
  }
  if (mine && !m.deleted && !m.pending) {
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

/** The browser: notifications with this tab closed. */
function pushRow() {
  const st = state.push;
  const hint = st === "on" ? "On in this browser: new messages and reminders arrive even with Chat closed"
    : st === "blocked" ? "This browser blocks notifications from this site: allow them in its site settings"
    : st === "unsupported" ? "This browser can't show notifications with the tab closed"
    : "New messages and reminders, even with Chat closed";
  const button = st === "on" ? `<button class="small" data-act="pushoff">Turn off</button>` : st === "off" || st === "" ? `<button class="small primary" data-act="pushon">Turn on</button>` : "";
  return `<div class="setting"><div><div class="label">Notifications when this tab is closed</div><div class="hintline">${hint}</div></div>${button}</div>`;
}

function toggle(key, label, hint) {
  const on = !!state.settings[key];
  return `<div class="setting"><div><div class="label">${label}</div><div class="hintline">${hint}</div></div>
    <button class="switch" role="switch" aria-checked="${on}" aria-label="${esc(label)}" data-act="flip" data-id="${key}"><span></span></button></div>`;
}

function settingsPage() {
  const sections = [["account", "Account"], ...(tauri && (!window.XENIFY_WEB || serverEmail()) ? [["email", "Email"]] : []), ...(tauri && state.live ? [["calendar", "Calendar"]] : []), ...(state.live ? [["apps", "Apps"]] : []), ["writing", "Spelling and grammar"], ...(canMakeText() ? [["voice", "Voice and transcripts"]] : []), ["notifications", "Notifications"], ["appearance", "Appearance"]];
  const nav = `<nav class="side" aria-label="Settings sections">
    <button class="textbtn" data-act="back" style="margin:6px 0 10px">${icons.back}Back to chats</button>
    <div class="section" style="padding:0 10px 6px">Settings</div>
    ${sections.map(([id, label]) => `<button class="tab wide" aria-current="${state.section === id ? "page" : "false"}" aria-selected="${state.section === id}" data-act="section" data-id="${id}">${label}</button>`).join("")}
  </nav>`;
  let body = "";
  if (state.section === "account") {
    body = `<h1>Account</h1><p class="lede">Who you are and which server you chat on.</p>
      <div class="setting"><div style="display:flex;align-items:center;gap:14px"><span class="avatar" style="width:52px;height:52px;background:#FDE6D8;color:#8A2C0A;font-size:18px">${esc(initials(nameOf(state.me)))}</span><div><div class="label">${esc(nameOf(state.me))}</div><div class="hintline">${esc(state.me.email || "")}</div></div></div>${state.live ? `<button class="small" data-act="changename">Change name</button>` : ""}</div>
      <div class="setting"><div><div class="label">Server</div><div class="hintline">${esc(state.server)} · ${state.viaHub ? "through Xenify Hub: every app uses it, and signing out of the Hub signs them all out" : "remembered for next time"}</div></div>${state.live ? `<button class="small" data-act="changeserver">${state.viaHub ? "Use a different server" : "Change server"}</button>` : ""}</div>
      ${state.live ? twoStepRow() : ""}
      ${state.hub && !state.viaHub ? `<div class="setting"><div><div class="label">Xenify Hub</div><div class="hintline">The Hub is signed in to a server. Chat can use it too, like the other apps.</div></div><button class="small" data-act="usehub">Use the Hub's server</button></div>` : ""}
      ${state.live ? `<div style="margin-top:20px"><button class="small danger-outline" data-act="signout">${state.viaHub ? "Sign out of Chat only" : "Sign out"}</button></div>` : `<p class="hintline" style="margin-top:20px">These are sample chats. Start Xenify Chat without XENIFY_SAMPLE to sign in.</p>`}`;
  } else if (state.section === "email") {
    body = emailSettings();
  } else if (state.section === "calendar") {
    body = calendarSettings();
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
  } else if (state.section === "voice") {
    body = voiceSettings();
  } else if (state.section === "notifications") {
    body = `<h1>Notifications</h1><p class="lede">How Xenify Chat gets your attention. Each chat can also be muted from the bell at its top right.</p>
      ${toggle("desktop", "Desktop notifications", "Show new messages from chats that aren't muted")}
      ${toggle("preview", "Show the message in notifications", "Off: only who it's from, so nobody can read over your shoulder")}
      ${toggle("sound", "Play a sound", "A short chime for new messages")}
      ${toggle("mentions", "Always tell me when I'm mentioned", "Even in muted chats")}
      <div class="setting"><div><label class="label" for="quiet">Quiet hours</label><div class="hintline">Do not disturb turns itself on at these times</div></div>
        <select id="quiet">${["Off", "Evenings and weekends", "18:00 to 08:00", "Weekends only"].map((o) => `<option ${state.settings.quiet === o ? "selected" : ""}>${o}</option>`).join("")}</select></div>
      ${state.live && state.info?.features?.includes("mail") ? `<div class="setting"><div><label class="label" for="digest">Email me what I miss</label><div class="hintline">Messages still unread after 10 minutes, to the email address on your account. Mutes, Do not disturb and quiet hours count.</div></div>
        <select id="digest">${[["off", "Never"], ["mentions", "Direct messages and mentions"], ["all", "Everything"]].map(([v, l]) => `<option value="${v}" ${state.settings.digest === v ? "selected" : ""}>${l}</option>`).join("")}</select></div>` : ""}
      ${state.live ? `<div class="setting"><div><label class="label" for="meetremind">Remind me before meetings</label><div class="hintline">For new meetings; change one meeting's from the meeting itself</div></div>
        <select id="meetremind">${MEET_REMIND.filter(([v]) => v !== null).map(([v, l]) => `<option value="${v}" ${Number(state.settings.meetingRemind) === v ? "selected" : ""}>${l}</option>`).join("")}</select></div>` : ""}
      ${window.XENIFY_WEB ? pushRow() : tauri ? `<div class="setting"><div><div class="label">Start when you sign in to your computer</div><div class="hintline">Xenify Chat waits in the system tray; closing its window keeps it there</div></div>
        <button class="switch" role="switch" aria-checked="${state.autostart}" aria-label="Start when you sign in to your computer" data-act="autostart"><span></span></button></div>` : ""}
      ${state.live ? `<p class="hintline syncnote">These settings, spelling and grammar, and the theme are kept with your account, so your other computers and browsers use them too.</p>` : ""}
      <h2>Muted chats</h2>
      ${state.conversations.filter(muted).map((c) => `<div class="setting">${avatarFor(c, "small")}<div style="flex:1"><div class="label">${esc(titleOf(c))}</div><div class="hintline">${NOTIFY.find((n) => n.id === notifyLevel(c)).short}</div></div><button class="small" data-act="unmute" data-id="${c.id}">Unmute</button></div>`).join("") || `<p class="hintline">No chats are muted.</p>`}`;
  } else {
    body = `<h1>Appearance</h1><p class="lede">Make Xenify Chat comfortable to look at.</p>
      <h2>Theme</h2>
      <div class="choices">${[["system", "Same as the computer"], ["light", "Light"], ["dark", "Dark"]].map(([id, l]) => `<label class="choice"><input type="radio" name="theme" value="${id}" ${state.settings.theme === id ? "checked" : ""}> ${l}</label>`).join("")}</div>
      <h2>Text size</h2>
      <div style="display:flex;align-items:center;gap:12px;max-width:420px"><label class="sr" for="size">Text size</label><input id="size" type="range" min="80" max="150" step="10" value="${state.settings.size}" style="flex:1"><span>${state.settings.size}%</span></div>`;
  }
  return nav + `<main class="sheet settings"><div class="inner"><button class="textbtn phoneback" data-act="phoneback">${icons.back}Settings</button>${body}</div></main>`;
}

// ---------- Sign in ----------

const CODE_LABEL = { app: "Code from your authenticator app", email: "Code from the email", backup: "Backup code" };

/** The authenticator key to scan, or type. */
const keyPart = (key) => `<p class="hintline">Scan this with your authenticator app, or type the key</p>
  <div class="qrkey"><span class="qr">${key.qr_svg}</span><code>${esc(key.secret.replace(/(.{4})/g, "$1 ").trim())}</code></div>`;

/** Backup codes, shown once. */
const codesPart = (codes) => `<div class="backupcodes">${codes.map((c) => `<code>${esc(c)}</code>`).join("")}</div>`;

/** Signing in, password right: the second step (`k.step`), setting one up
 *  first when the server requires it, then the backup codes (`k.codes`). */
function stepForm(k) {
  const st = k.step;
  const m = k.mfa;
  const codeField = (method) => `<label class="field">${CODE_LABEL[method]}<input id="sc" autocomplete="one-time-code" autocapitalize="off" spellcheck="false" required></label>
        <label class="checkline" style="border:none;padding:0"><input type="checkbox" id="srem" ${m.remember ? "checked" : ""}><span><b>Remember this computer for 30 days</b></span></label>`;
  const error = k.error ? `<p class="formerror" role="alert">${esc(k.error)}</p>` : "";
  const again = `<button class="linkbtn" type="button" data-act="stepagain">Start again</button>`;
  if (k.codes) {
    return `<h2>Backup codes</h2>
        <p class="lede">If you can't get a code, each of these signs you in once. Keep them somewhere safe: they aren't shown again.</p>
        ${codesPart(k.codes)}
        <button class="small" type="button" data-act="copy" data-id="${esc(k.codes.join("\n"))}" style="align-self:flex-start">Copy</button>
        <button class="big primary" type="button" data-act="codeskept">I've kept them</button>`;
  }
  if (st.setup) {
    const ready = m.setup === "email" || (m.setup === "app" && m.key);
    return `<h2>Two-step sign-in</h2>
        <p class="lede">Your server asks for a code as well as your password when you sign in. Set that up to go on.</p>
        ${!m.setup ? `<button class="big" type="button" data-act="stepsetup" data-id="app">Use an authenticator app</button>
        ${st.setup_methods.includes("email") ? `<button class="big" type="button" data-act="stepsetup" data-id="email">Email me codes</button>` : ""}` : ""}
        ${m.setup === "app" && m.key ? keyPart(m.key) : ""}
        ${m.setup === "email" ? `<p class="hintline">A code is on its way to your email.</p>` : ""}
        ${ready ? codeField(m.setup) : ""}
        ${error}
        ${ready ? `<button class="big primary" type="submit" ${k.busy ? "disabled" : ""}>${k.busy ? "Checking…" : "Turn on"}</button>` : ""}
        ${again}`;
  }
  const has = (x) => st.methods.includes(x);
  const method = m.method || (has("app") ? "app" : has("email") ? "email" : "backup");
  return `<h2>Two-step sign-in</h2>
        <p class="lede">${method === "app" ? "Type the code your authenticator app shows." : method === "email" ? (m.sent ? "A code is on its way to your email." : "We can email you a code.") : "Type one of your backup codes."}</p>
        ${codeField(method)}
        ${error}
        <button class="big primary" type="submit" ${k.busy ? "disabled" : ""}>${k.busy ? "Signing in…" : "Sign in"}</button>
        ${has("app") && method !== "app" ? `<button class="linkbtn" type="button" data-act="stepuse" data-id="app">Use your authenticator app</button>` : ""}
        ${has("email") ? `<button class="linkbtn" type="button" data-act="stepemail">${m.sent ? "Email me another code" : "Email me a code"}</button>` : ""}
        ${has("backup") && method !== "backup" ? `<button class="linkbtn" type="button" data-act="stepuse" data-id="backup">Use a backup code</button>` : ""}
        ${again}`;
}

function signinPage() {
  const k = state.signin;
  if (k.step || k.codes) {
    return `<div class="signin">
    <section class="brandpanel" aria-hidden="true">
      <h1>Xenify Chat</h1>
      <p>Chat with your team. Spelling and grammar are checked on your own computer, the same way as in Xenify Write.</p>
    </section>
    <main class="signform">
      <form data-act="signin" autocomplete="on">
        ${stepForm(k)}
      </form>
    </main>
  </div>`;
  }
  // The user name first (k.typed: it's been checked); then the password,
  // or (k.join: an account without a password yet, or a forgotten one) a
  // code, emailed or from an admin, then picking a password. (k.join
  // without k.fresh: an older link Chat was opened with.)
  const l = k.join ? k.link : null;
  const who = l ? esc(l.display_name || l.email || l.username) : "";
  const named = !k.join && k.typed;
  const [title, lede] = !k.join ? ["Sign in", named ? "" : "Welcome back."]
    : !l ? (k.fresh ? [k.forgot ? "Forgot your password?" : "Pick your password", k.emailed ? "" : "This account doesn’t have a password yet. Type the code your admin gave you."] : ["Open your link", "Paste the link you were sent."])
    : l.kind === "reset" ? ["Choose a new password", `For ${who}.`]
    : [`Welcome, ${who}`, `Choose a password for ${esc(l.server_name)}.`];
  const whoLine = `<p class="signwho">${esc(k.username || "")} <button class="linkbtn" type="button" data-act="signback">Change</button></p>`;
  const fields = !k.join
    ? !named
      ? `${state.askServer ? `<label class="field">Server<input id="ss" value="${esc(k.server || "")}" placeholder="chat.example.org" autocomplete="url" autocapitalize="off" spellcheck="false" required><span class="hintline">Your organisation's Xenify server</span></label>` : ""}
        <label class="field">Email<input id="su" inputmode="email" value="${esc(k.username || "")}" autocomplete="username" autocapitalize="off" spellcheck="false" required></label>`
      : `${whoLine}<input type="hidden" id="su" value="${esc(k.username || "")}" autocomplete="username">
        <label class="field">Password<input id="sp" type="password" autocomplete="current-password" required></label>`
    : !l
      ? `${k.fresh ? whoLine : ""}<label class="field">${k.fresh ? "Code" : "Link"}<input id="si" value="${esc(k.invite || "")}" autocomplete="one-time-code" autocapitalize="off" spellcheck="false" required></label>`
      : `<input type="hidden" id="su" value="${esc(l.username)}" autocomplete="username">
        <label class="field">Password<input id="sp" type="password" autocomplete="new-password" required><span class="hintline">At least 8 characters</span></label>`;
  const button = k.busy ? (k.join && !l ? "Opening…" : k.join ? "Setting up…" : named ? "Signing in…" : "Checking…") : k.join ? "Continue" : named ? "Sign in" : "Next";
  const other = !k.join
    ? named ? `<button class="linkbtn" type="button" data-act="signforgot">Forgot password?</button>` : ""
    : k.fresh && !l ? (k.emailed ? `<button class="linkbtn" type="button" data-act="signforgot">Send a new code</button>` : "")
    : k.fresh ? "" : `<button class="linkbtn" type="button" data-act="signmode">Already have a password? Sign in</button>`;
  return `<div class="signin">
    <section class="brandpanel" aria-hidden="true">
      <h1>Xenify Chat</h1>
      <p>Chat with your team. Spelling and grammar are checked on your own computer, the same way as in Xenify Write.</p>
    </section>
    <main class="signform">
      <form data-act="signin" autocomplete="on">
        <h2>${title}</h2>
        <p class="lede">${lede}</p>
        ${fields}
        ${k.error ? `<p class="formerror" role="alert">${esc(k.error)}</p>` : ""}
        ${k.note ? `<p class="hintline" role="status">${esc(k.note)}</p>` : ""}
        <button class="big primary" type="submit" ${k.busy ? "disabled" : ""}>${button}</button>
        ${other}
        ${state.hub ? `<button class="linkbtn" type="button" data-act="usehub">Use the server Xenify Hub is signed in to</button>` : ""}
      </form>
    </main>
  </div>`;
}

/** Join mode, first step: what the pasted link (or the one Xenify Chat was
 *  opened with, `typed` "") is for. */
async function openLink(typed) {
  const k = state.signin;
  k.busy = true;
  k.error = "";
  render();
  try {
    k.link = await tauri.core.invoke("link_info", { link: typed, username: k.fresh ? k.username || "" : "" });
    k.invite = "";
  } catch (e) {
    k.error = String(e);
  }
  k.busy = false;
  render();
  document.getElementById(k.link ? "sp" : "si")?.focus();
}

/** Has the server email the person signing in a code, and asks for it. */
async function emailCode(note) {
  const k = state.signin;
  try {
    await tauri.core.invoke("forgot", { username: k.username || "" });
    Object.assign(k, { emailed: true, note, error: "", busy: false });
  } catch (e) {
    Object.assign(k, { error: String(e), busy: false });
  }
  render();
  document.getElementById("si")?.focus();
}

/** Signed in: on to the chats. */
function entered(user) {
  const k = state.signin;
  state.me = user;
  Object.assign(k, { busy: false, invite: "", join: false, link: null, step: null, mfa: {} });
  state.mfa = undefined;
  if (k.codes) return;
  state.view = "chat";
  if (state.net !== "online") state.net = "connecting";
}

/** An error during the second step: when it took too long, back to the password. */
function stepFailed(e) {
  const k = state.signin;
  k.busy = false;
  k.error = String(e);
  if (/sign in again/i.test(k.error)) Object.assign(k, { step: null, mfa: {} });
}

/** Signing in, password right: the code, or turning the second step on. */
async function stepIn() {
  const k = state.signin;
  const code = document.getElementById("sc").value;
  k.mfa.remember = !!document.getElementById("srem")?.checked;
  if (!code.trim()) return;
  k.busy = true;
  k.error = "";
  render();
  try {
    if (k.step.setup) {
      const out = await tauri.core.invoke("mfa_setup_finish", { ticket: k.step.ticket, method: k.mfa.setup, code, remember: k.mfa.remember });
      k.codes = out.backup_codes;
      entered(out.user);
    } else {
      const method = k.mfa.method || (k.step.methods.includes("app") ? "app" : k.step.methods.includes("email") ? "email" : "backup");
      entered(await tauri.core.invoke("second_code", { ticket: k.step.ticket, method, code, remember: k.mfa.remember }));
    }
  } catch (e) {
    stepFailed(e);
  }
  render();
  document.getElementById("sc")?.focus();
}

async function signIn() {
  const k = state.signin;
  if (k.step) return stepIn();
  if (k.join && !k.link) return openLink((k.invite = document.getElementById("si").value));
  const username = (k.username = document.getElementById("su").value);
  // Read before redrawing, which empties the field.
  const password = document.getElementById("sp")?.value ?? "";
  const server = state.askServer && !k.join ? (k.server = document.getElementById("ss")?.value ?? k.server) : "";
  k.busy = true;
  k.error = "";
  k.note = "";
  render();
  try {
    if (!k.join && !k.typed) {
      // The user name first: the password next, or picking one.
      const step = await tauri.core.invoke("sign_in_start", { username, server });
      k.busy = false;
      if (step.next === "pick_password") {
        Object.assign(k, { join: true, fresh: true, forgot: false, emailed: false, link: null, invite: "" });
        if (step.can_email) return emailCode("We’ve emailed you a code. Type it here, then pick your password.");
      } else k.typed = true;
      render();
      return document.getElementById(k.join ? "si" : "sp")?.focus();
    }
    const invite = k.join ? k.link.link : "";
    const out = await tauri.core.invoke("sign_in", { username, password, invite, server });
    if (out.step) {
      // The password was right: a code next (or setting one up).
      Object.assign(k, { busy: false, step: out.step, mfa: {} });
      render();
      const only = !out.step.setup && !out.step.methods.includes("app") && out.step.methods.includes("email");
      if (only) return stepAct("stepemail");
      return document.getElementById("sc")?.focus();
    }
    entered(out);
  } catch (e) {
    Object.assign(k, { busy: false, error: String(e) });
    // The server's own certificate: ask whether to trust it.
    if (/trust the server's certificate/.test(k.error) && !window.XENIFY_WEB) {
      try {
        const seen = await tauri.core.invoke("certificate", { fingerprint: null, remember: false });
        if (!seen.trusted && !seen.ignored) state.dialog = { kind: "certificate", seen };
      } catch {}
    }
  }
  render();
  if (state.view === "signin") document.getElementById(k.error.includes("assword") ? "sp" : "su")?.focus();
}

/** The second step's buttons on the sign-in page. */
async function stepAct(act, id) {
  const k = state.signin;
  k.mfa.remember = !!document.getElementById("srem")?.checked;
  k.error = "";
  try {
    if (act === "stepuse") k.mfa.method = id;
    else if (act === "stepagain") Object.assign(k, { step: null, mfa: {} });
    else if (act === "codeskept") {
      k.codes = null;
      state.view = "chat";
      if (state.net !== "online") state.net = "connecting";
    } else if (act === "stepemail") {
      await tauri.core.invoke("second_email", { ticket: k.step.ticket });
      Object.assign(k.mfa, { method: "email", sent: true });
    } else if (act === "stepsetup") {
      if (id === "app") k.mfa.key ||= await tauri.core.invoke("mfa_setup_app", { ticket: k.step.ticket });
      else await tauri.core.invoke("mfa_setup_email", { ticket: k.step.ticket });
      k.mfa.setup = id;
    }
  } catch (e) {
    stepFailed(e);
  }
  render();
  document.getElementById(k.step ? "sc" : "sp")?.focus();
}

// ---------- Settings › Account › Two-step sign-in ----------

async function loadMfa() {
  try { state.mfa = await tauri.core.invoke("mfa_state"); } catch { state.mfa = null; }
  render();
}

function twoStepRow() {
  // Asked once each time Settings shows it; servers from before two-step sign-in have none.
  if (state.mfa === undefined) { state.mfa = null; loadMfa(); }
  const m = state.mfa;
  if (!m) return "";
  const on = m.app || m.email;
  const how = [m.app && "your authenticator app", m.email && "email"].filter(Boolean).join(" or ");
  const hint = on
    ? `On: a code from ${how} as well as your password. ${m.backup_left} backup code${m.backup_left === 1 ? "" : "s"} left.${m.required ? " This server requires it." : ""}`
    : `Off. With it on, signing in takes a code as well as your password.${m.required ? " This server requires it: set it up." : ""}`;
  return `<div class="setting"><div><div class="label">Two-step sign-in</div><div class="hintline">${hint}</div></div>
    <span style="display:flex;gap:8px;flex:none">${on
      ? `<button class="small" data-act="mfabackup">New backup codes</button>${m.required ? "" : `<button class="small" data-act="mfaoff">Turn off</button>`}`
      : `<button class="small" data-act="mfaset">Set up</button>`}</span></div>`;
}

async function mfaAct(act, id) {
  const m = state.mfa;
  if (act === "mfaoff" || act === "mfabackup") {
    state.dialog = { kind: "mfa", step: act === "mfaoff" ? "off" : "backup", error: "" };
    render();
    return document.getElementById("mpass")?.focus();
  }
  const method = id || (m.setup_methods.length === 1 ? m.setup_methods[0] : "");
  state.dialog = { kind: "mfa", step: method || "choose", error: "" };
  try {
    if (method === "app") state.dialog.key = await tauri.core.invoke("mfa_setup_app", { ticket: "" });
    if (method === "email") await tauri.core.invoke("mfa_setup_email", { ticket: "" });
  } catch (e) {
    state.dialog.error = String(e);
  }
  render();
  document.getElementById("mcode")?.focus();
}

function mfaDialog(d) {
  const head = d.step === "off" ? "Turn off two-step sign-in" : d.step === "backup" ? "New backup codes" : d.step === "codes" ? "Backup codes" : "Two-step sign-in";
  const error = d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : "";
  let body = "";
  let ok = "";
  if (d.step === "choose") {
    body = `<p>Signing in will take a code as well as your password.</p>
      <button class="small" type="button" data-act="mfachoose" data-id="app">Use an authenticator app</button>
      <button class="small" type="button" data-act="mfachoose" data-id="email">Email me codes</button>`;
  } else if (d.step === "app" || d.step === "email") {
    body = `${d.step === "app" ? (d.key ? keyPart(d.key) : "") : `<p>A code is on its way to your email.</p>`}
      <label class="field">${CODE_LABEL[d.step]}<input id="mcode" autocomplete="one-time-code" autocapitalize="off" spellcheck="false" required></label>`;
    ok = "Turn on";
  } else if (d.step === "codes") {
    body = `<p>If you can't get a code, each of these signs you in once. Keep them somewhere safe: they aren't shown again.</p>
      ${codesPart(d.codes)}`;
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="${head}">
      <h2>${head}</h2>${body}
      <div class="buttons"><button class="small" type="button" data-act="copy" data-id="${esc(d.codes.join("\n"))}">Copy</button><button class="small primary" type="submit">I've kept them</button></div>
    </form></div>`;
  } else {
    body = `<p>${d.step === "off" ? "Signing in will take only your password." : "Your old backup codes stop working."}</p>
      <label class="field">Your password<input id="mpass" type="password" autocomplete="current-password" required></label>`;
    ok = d.step === "off" ? "Turn off" : "Make new codes";
  }
  return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="${head}">
    <h2>${head}</h2>${body}${error}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button>${ok ? `<button class="small primary" type="submit">${ok}</button>` : ""}</div>
  </form></div>`;
}

async function finishMfa(d) {
  d.error = "";
  try {
    if (d.step === "codes") {
      state.dialog = null;
      return loadMfa();
    }
    if (d.step === "app" || d.step === "email") {
      const out = await tauri.core.invoke("mfa_setup_finish", { ticket: "", method: d.step, code: document.getElementById("mcode").value, remember: false });
      Object.assign(d, { step: "codes", codes: out.backup_codes });
    } else if (d.step === "backup") {
      Object.assign(d, { step: "codes", codes: await tauri.core.invoke("mfa_backup", { password: document.getElementById("mpass").value }) });
    } else if (d.step === "off") {
      await tauri.core.invoke("mfa_off", { password: document.getElementById("mpass").value });
      state.dialog = null;
      showToast("Two-step sign-in is off.");
      return loadMfa();
    }
  } catch (e) {
    d.error = String(e);
  }
  render();
  document.getElementById(d.step === "codes" ? "" : d.step === "app" || d.step === "email" ? "mcode" : "mpass")?.focus();
}

// ---------- One-time secrets ----------
// A password (or the like) as a link that opens once, within 1 to 7 days
// (the server's src/secrets.rs). It's encrypted here; the key goes only in
// the link's # part, so the server keeps what it can't read.

function secretDialog(d) {
  const where = d.into === "mail" ? "your email" : "your message";
  return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Share a secret">
    <h2>Share a secret</h2>
    <p>A password or the like, as a link that opens once. Whoever opens it first sees it; after that, or when the time's up, the link stops working.</p>
    <label class="field">The secret<textarea id="stext" rows="3" autocomplete="off" spellcheck="false" maxlength="10000" required></textarea></label>
    <label class="field">Keep it for<select id="sdays">${[1, 2, 3, 4, 5, 6, 7].map((n) => `<option value="${n}" ${d.days === n ? "selected" : ""}>${n === 1 ? "1 day" : `${n} days`}</option>`).join("")}</select></label>
    <label class="field">Passphrase (optional)<input id="spass" type="text" autocomplete="off" spellcheck="false" maxlength="200" placeholder="Tell them this another way: say it, or text it"></label>
    <p class="hintline">It's encrypted on this computer; the server can't read it. The link goes in ${where} for you to send. With a passphrase, the link alone can't open it, and 5 wrong tries delete it.</p>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit" ${d.busy ? "disabled" : ""}>${d.busy ? "Making the link…" : "Make the link"}</button></div>
  </form></div>`;
}

const b64 = (bytes) => btoa(String.fromCharCode(...bytes));
const b64url = (bytes) => b64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

// With a passphrase, the key is made from the link's key and the
// passphrase, and a check lets the server refuse a wrong passphrase without
// giving the secret up (the secret page does the same: web/secret.js).
async function secretKeys(linkKey, pass, salt) {
  if (!pass) return { key: await crypto.subtle.importKey("raw", linkKey, "AES-GCM", false, ["encrypt"]), check: null };
  const base = await crypto.subtle.importKey("raw", new TextEncoder().encode(pass), "PBKDF2", false, ["deriveBits"]);
  const stretched = await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt, iterations: 600000 }, base, 256);
  const hk = await crypto.subtle.importKey("raw", linkKey, "HKDF", false, ["deriveKey", "deriveBits"]);
  const how = (info) => ({ name: "HKDF", hash: "SHA-256", salt: stretched, info: new TextEncoder().encode(info) });
  const key = await crypto.subtle.deriveKey(how("xenify secret key"), hk, { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
  const check = b64(new Uint8Array(await crypto.subtle.deriveBits(how("xenify secret check"), hk, 256)));
  return { key, check };
}

async function makeSecret(d) {
  const text = document.getElementById("stext").value;
  const pass = document.getElementById("spass").value;
  d.days = Number(document.getElementById("sdays").value) || 1;
  if (!text.trim()) return;
  if (!crypto.subtle) {
    d.error = "This page isn't on a secure (HTTPS) address, so it can't encrypt. Open Chat over HTTPS, or use the desktop app.";
    return render();
  }
  d.busy = true;
  d.error = "";
  render();
  try {
    const key = crypto.getRandomValues(new Uint8Array(32));
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const k = await secretKeys(key, pass, salt);
    const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, k.key, new TextEncoder().encode(text)));
    const made = await tauri.core.invoke("secret_new", { sealed: b64(new Uint8Array([...iv, ...sealed])), days: d.days, check: k.check });
    const until = new Date(made.expires).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
    const line = `A secret for you (opens once, until ${until}${pass ? ", with the passphrase I'll give you" : ""}): ${made.link}.${b64url(key)}${pass ? `.${b64url(salt)}` : ""}`;
    state.dialog = null;
    if (d.into === "mail" && state.compose) {
      state.compose.text = state.compose.text.trim() ? `${state.compose.text.trimEnd()}\n\n${line}\n` : `${line}\n`;
      render();
      scheduleDraft();
    } else if (d.conv) {
      const was = state.drafts[d.conv] || "";
      state.drafts[d.conv] = was.trim() ? `${was.trimEnd()}\n${line}` : line;
      render();
      const t = document.getElementById("draft");
      if (t) { t.focus(); t.dispatchEvent(new Event("input", { bubbles: true })); }
    }
  } catch (e) {
    d.busy = false;
    d.error = String(e);
    render();
  }
}

async function signOut() {
  await tauri.core.invoke("sign_out");
  toSignin("");
}

function toSignin(error) {
  Object.assign(state, { me: null, conversations: [], messages: {}, sel: null, view: "signin", menu: null, dialog: null, q: "" });
  // (An opened link, and the server typed, stay.)
  const was = state.signin || {};
  state.signin = { join: !!was.link && !state.me, link: was.link || null, error, busy: false, invite: "", server: was.server || "", username: was.username || "", step: null, mfa: {} };
  state.mfa = undefined;
  render();
  document.getElementById(state.signin.join ? "sp" : "su")?.focus();
}

// ---------- Dialogs: new group or channel, add people, change server ----------

function dialogBox() {
  const d = state.dialog;
  if (!d) return "";
  if (d.kind === "event") return eventDialog(d);
  if (d.kind === "meeting") return meetingDialog(d);
  if (d.kind === "contact") return contactDialog(d);
  if (d.kind === "book") return bookDialog(d);
  if (d.kind === "thing") return thingDialog(d);
  if (d.kind === "away") return awayDialog(d);
  if (d.kind === "record") return recordDialog(d);
  if (d.kind === "transcript") return transcriptDialog(d);
  if (d.kind === "board") {
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="New whiteboard">
      <h2>New whiteboard</h2>
      <p>Everyone in this chat can draw on it and make diagrams, together and live.</p>
      <label class="field">Name<input id="dtitle" value="${esc(d.title)}" placeholder="Whiteboard" autocomplete="off"></label>
      <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Make it</button></div>
    </form></div>`;
  }
  if (d.kind === "certificate") {
    const c = d.seen;
    const host = c.host.replace(/:443$/, "");
    return `<div class="scrim"><div class="dialog" role="alertdialog" aria-label="This server uses its own certificate">
      <h2>This server uses its own certificate</h2>
      ${c.changed ? `<p role="alert">${esc(host)} showed a different certificate before. If its admin didn't make a new one, something else may be answering in its place: don't connect until you've checked.</p>` : ""}
      <p>${esc(host)} has no domain name, so its HTTPS certificate is signed by the server itself${c.name ? ` (“${esc(c.name)}”)` : ""}. To be sure it's your server, check with its admin that this fingerprint matches the one on its admin pages (Admin › Network):</p>
      <p class="fingerprint">${esc(c.fingerprint)}</p>
      <p class="hintline">Trust this server: this computer remembers it, and asks again only if the certificate changes. Ignore: connect now and ask again next time.</p>
      <div class="buttons"><button class="small" data-act="closedialog">Cancel</button><button class="small" data-act="certonce">Ignore this time</button><button class="small primary" data-act="certtrust">Trust this server</button></div>
    </div></div>`;
  }
  if (d.kind === "app") return appDialog(d);
  if (d.kind === "remind") return remindDialog(d);
  if (d.kind === "keep") return keepDialog(d);
  if (d.kind === "name") {
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Change your name">
      <h2>Your name</h2>
      <p>The name others see in chats.</p>
      <label class="field">Name<input id="dname" value="${esc(d.value)}" autocomplete="name" maxlength="64"></label>
      <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Save</button></div>
    </form></div>`;
  }
  if (d.kind === "rename") {
    const c = d.cal ? null : conv(d.conv);
    return `<div class="scrim"><form class="dialog" data-act="dialog" role="dialog" aria-label="Rename ${esc(d.cal ? d.value : titleOf(c))}">
      <h2>Rename ${d.cal ? "calendar" : c?.kind === "channel" ? "channel" : "group"}</h2>
      <p>Everyone in it sees the new name.</p>
      <label class="field">Name<input id="dname" value="${esc(d.value)}" autocomplete="off" maxlength="100" required></label>
      <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Rename</button></div>
    </form></div>`;
  }
  if (d.kind === "mfa") return mfaDialog(d);
  if (d.kind === "secret") return secretDialog(d);
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
  const heading = d.kind === "add" && c?.kind === "direct" ? `A group chat with ${esc(titleOf(c))}` : d.kind === "group" ? "New group chat" : d.kind === "channel" ? "New channel" : d.owner ? `Who else posts in ${esc(titleOf(c))}` : `Add people to ${esc(titleOf(c))}`;
  const already = new Set([state.me.id, ...(c ? (d.owner ? c.owners || [] : c.members.map((u) => u.id)) : []), ...d.picked.map((u) => u.id)]);
  const results = d.q.trim() ? state.people.filter((u) => !already.has(u.id)) : [];
  const picker = d.kind === "channel" ? "" : `
      <label class="field">${d.kind === "add" ? "People" : "People to add"}<input id="pickq" value="${esc(d.q)}" placeholder="Search by name" autocomplete="off"></label>
      ${d.picked.length ? `<div class="words">${d.picked.map((u) => `<span class="word">${esc(nameOf(u))}<button type="button" class="iconbtn tiny" data-act="unpick" data-id="${u.id}" aria-label="Don't add ${esc(nameOf(u))}">${icons.close}</button></span>`).join("")}</div>` : ""}
      ${results.length ? `<div class="pickresults">${results.slice(0, 6).map((u) => `<button type="button" class="row compact" data-act="pick" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${esc(nameOf(u))}</span><span class="last">${esc(handleOf(u))}</span></span></button>`).join("")}</div>` : ""}`;
  const ready = d.kind === "add" ? d.picked.length > 0 : d.title.trim().length > 0;
  const toGroup = d.kind === "add" && c?.kind === "direct";
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${heading}">
    <h2>${heading}</h2>
    ${toGroup ? `<p>Pick who else to add. ${esc(titleOf(c))} is in it too; this chat with them stays as it is.</p>` : ""}
    ${d.kind === "add" ? "" : `<label class="field">Name<input id="dtitle" value="${esc(d.title)}" placeholder="${d.kind === "group" ? "e.g. Design team" : "e.g. Build alerts"}" autocomplete="off" required></label>`}
    ${picker}
    ${d.kind === "channel" && can("chat.create_open_channel") ? `<div class="setting" style="border:none;padding:4px 0"><div><div class="label">Anyone on the server can find and follow it</div><div class="hintline">Off: only people you add will see it</div></div><button type="button" class="switch" role="switch" aria-checked="${d.open}" aria-label="Anyone can follow" data-act="togglefield"><span></span></button></div>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" id="dcreate" type="submit" ${ready ? "" : "disabled"}>${toGroup ? "Start group chat" : d.kind === "add" ? (d.picked.length > 1 ? `Add ${d.picked.length} people` : "Add") : "Create"}</button></div>
  </form></div>`;
}

async function finishDialog() {
  const d = state.dialog;
  if (d.kind === "event") return saveEvent();
  if (d.kind === "contact") return saveContact();
  if (d.kind === "book") return saveBook();
  if (d.kind === "thing") return saveThing();
  if (d.kind === "away") return saveAway();
  if (d.kind === "board") {
    const title = (document.getElementById("dtitle")?.value || "").trim() || "Whiteboard";
    const c = conv(d.conv);
    state.dialog = null;
    try {
      const b = await tauri.core.invoke("board_new", { conv: d.conv, title });
      if (c) post(c, "~board:" + JSON.stringify(b));
      return openBoard(d.conv, b);
    } catch (e) {
      showToast(String(e));
      return render();
    }
  }
  if (d.kind === "app") return saveApp();
  if (d.kind === "keep") {
    const v = document.querySelector('input[name="keep"]:checked')?.value ?? "";
    tell({ t: "set_keep", conv: d.conv, days: v === "" ? null : Number(v) });
    state.dialog = null;
    return render();
  }
  if (d.kind === "name") {
    tell({ t: "set_name", display_name: document.getElementById("dname").value });
    state.dialog = null;
    return render();
  }
  if (d.kind === "rename") {
    const title = document.getElementById("dname").value.trim();
    if (!title) return;
    if (d.cal) tauri.core.invoke("shared_cal_rename", { conv: d.cal, name: title }).then(refreshCalendar, (e) => showToast(String(e)));
    else tell({ t: "rename", conv: d.conv, title });
    state.dialog = null;
    return render();
  }
  if (d.kind === "mfa") return finishMfa(d);
  if (d.kind === "secret") return makeSecret(d);
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
  const from = d.kind === "add" ? conv(d.conv) : null;
  if (from?.kind === "direct") {
    // A direct chat stays two people: adding someone starts a group.
    if (!d.picked.length) return;
    const members = [...others(from), ...d.picked].map((u) => u.username);
    state.wantOpen = { members: [...members, state.me.username].sort().join(" ") };
    tell({ t: "create_group", title: "", members });
  } else if (d.kind === "add") {
    if (!d.picked.length) return;
    tell({ t: "add_people", conv: d.conv, users: d.picked.map((u) => u.username) });
    // Posting in a channel: its owners (they follow it too).
    if (d.owner) for (const u of d.picked) tell({ t: "set_role", conv: d.conv, user: u.username, owner: true });
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
  if (!q.trim()) {
    state.people = [];
    state.found = [];
    if (state.dialog?.kind === "direct") tell({ t: "find_people", q: "" });
    return;
  }
  findTimer = setTimeout(() => {
    tell({ t: "find_people", q: q.trim() });
    if (!state.dialog) tell({ t: "find_conversations", q: q.trim() });
    // Messages are searched on the server.
    if (!state.dialog) tell({ t: "search", q: q.trim(), sort: state.kept.sort, tag: "find" });
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
  // Who came and went while you watched (groups; channels have followers).
  const was = i >= 0 ? state.conversations[i] : null;
  if (was && c.kind === "group" && c.member_count <= 50 && was.member_count <= 50) {
    const before = new Map(was.members.map((u) => [u.id, u]));
    const now = new Map(c.members.map((u) => [u.id, u]));
    const lines = (state.events[c.id] ||= []);
    for (const [id, u] of now) if (!before.has(id)) lines.push({ at: Date.now(), text: `${id === state.me.id ? "You were" : `${nameOf(u)} was`} added` });
    for (const [id, u] of before) if (!now.has(id)) lines.push({ at: Date.now(), text: `${nameOf(u)} left or was taken out` });
  }
  if (i >= 0) state.conversations[i] = c;
  else state.conversations.push(c);
  state.conversations.sort(byLatest);
  const w = state.wantOpen;
  if (w && ((w.user && c.kind === "direct" && c.members.some((u) => u.id === w.user)) || (w.username && c.kind === "direct" && c.members.some((u) => u.username === w.username)) || (w.id && c.id === w.id) || (w.title && c.title === w.title && c.role === "owner") || (w.members && c.kind === "group" && c.role === "owner" && c.members.map((u) => u.username).sort().join(" ") === w.members))) {
    state.wantOpen = null;
    open(c.id);
  }
}

function upsertMessage(msg) {
  // A reply: in its thread (when it's been opened), beside the chat.
  const key = `${msg.conv}:${msg.thread}`;
  const list = msg.thread != null && state.threads ? (state.threadMsgs[key] ||= []) : (state.messages[msg.conv] ||= []);
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
  trace(`to_app ${m.t}`);
  // Anything from the server means it's connected (its "online" can come
  // before sign-in finishes and be overwritten).
  if (state.net !== "online") state.net = "online";
  switch (m.t) {
    case "welcome":
      state.me = m.user;
      state.keepDays = m.keep_days ?? null;
      state.perms = m.perms || [];
      state.threads = !!m.threads;
      state.ai = !!m.ai;
      // How far the server's clock is ahead of this computer's.
      state.skew = m.now ? m.now - Date.now() : 0;
      onServerSettings(m.settings, true);
      tauri.core.invoke("server_info").then((info) => {
        const had = serverEmail();
        state.info = info;
        // The server keeps email: its accounts take over from this computer's.
        if (serverEmail() !== had) initMail();
        render();
      }).catch(() => {});
      break;
    case "settings":
      onServerSettings(m.settings, false);
      break;
    case "hits":
      if (m.tag === "find" && m.q === state.q.trim() && (m.sort || "best") === state.kept.sort) {
        // A next page adds to what's shown.
        const list = m.after ? [...state.kept.list, ...m.hits.filter((h) => !state.kept.list.some((x) => x.msg.id === h.msg.id))] : m.hits;
        state.kept = { q: m.q, list, sort: m.sort || "best", more: m.more || "", total: m.total || 0, loading: false };
      }
      break;
    case "answer":
      if (state.answer && m.q === state.answer.q) state.answer = { q: m.q, busy: false, text: m.text, sources: m.sources || [], error: m.error || "" };
      break;
    case "reminder":
      onReminder(m.reminder);
      break;
    case "reminder_gone":
      state.due = state.due.filter((r) => r.key !== m.key);
      break;
    case "conversations":
      // Meetings' conversations hold their calls; they aren't chats.
      state.meetingConvs = m.list.filter((c) => c.purpose === "meeting");
      state.conversations = m.list.filter((c) => !c.purpose).sort(byLatest);
      // Recent messages from every chat, so each opens straight away.
      for (const c of state.conversations) tell({ t: "history", conv: c.id, before: null, limit: 100 });
      // Start in the latest chat, as if you'd opened it.
      const first = state.conversations.find((c) => c.last || c.kind !== "direct");
      if (!conv(state.sel) && first && state.view === "chat") return open(first.id, { keepSearch: true });
      if (!conv(state.sel)) state.sel = null;
      break;
    case "conversation":
      if (m.conv.purpose === "meeting") state.meetingConvs = [...state.meetingConvs.filter((c) => c.id !== m.conv.id), m.conv];
      else upsertConv(m.conv);
      break;
    case "removed":
      state.meetingConvs = state.meetingConvs.filter((c) => c.id !== m.conv);
      removeConv(m.conv);
      break;
    case "message":
      upsertMessage(m.msg);
      break;
    case "history": {
      if (m.thread != null) {
        const key = `${m.conv}:${m.thread}`;
        const have = state.threadMsgs[key] || [];
        const byId = new Map(have.filter((x) => !x.pending).map((x) => [x.id, x]));
        for (const x of m.messages) byId.set(x.id, x);
        state.threadMsgs[key] = [...byId.values()].sort((a, b) => a.seq - b.seq).concat(have.filter((p) => p.pending && !m.messages.some((x) => x.client_id === p.client_id)));
        historyWaits.get(m.tag)?.();
        historyWaits.delete(m.tag);
        break;
      }
      const have = state.messages[m.conv] || [];
      const pending = have.filter((x) => x.pending);
      const byId = new Map(have.filter((x) => !x.pending).map((x) => [x.id, x]));
      for (const x of m.messages) byId.set(x.id, x);
      state.messages[m.conv] = [...byId.values()].sort((a, b) => a.seq - b.seq).concat(pending.filter((p) => !m.messages.some((x) => x.client_id === p.client_id)));
      historyWaits.get(m.tag)?.();
      historyWaits.delete(m.tag);
      break;
    }
    case "call":
      onCall(m.call);
      break;
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
      if (m.q === "" && state.view === "people") state.ab.directory = m.users;
      if (m.q === (state.dialog ? state.dialog.q : state.q).trim()) state.people = m.users;
      break;
    case "things": {
      const first = !state.things.list.length;
      Object.assign(state.things, { list: m.list, admin: m.admin });
      if (state.dialog?.kind === "thing") state.dialog = null;
      for (const [b, tag] of state.things.later || []) if (thingOf(b.thing)) onBooked(b, tag);
      state.things.later = [];
      // What's booked over the next month (for "free now" and the pages).
      if (m.list.length && (first || state.view === "people")) tell({ t: "bookings", things: m.list.map((t) => t.id), from: startOfDay(Date.now()) - DAY, to: startOfDay(Date.now()) + 30 * DAY });
      break;
    }
    case "bookings":
      for (const b of m.list) state.things.byId[b.id] = b;
      if (state.view === "calendar") loadWeek();
      break;
    case "booked":
      onBooked(m.booking, m.tag);
      if (state.view === "calendar") loadWeek();
      break;
    case "away_of":
      for (const a of m.list) {
        state.away.of[a.user] = a;
        if (a.user === state.me?.id) state.away.mine = a;
      }
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
      showToast(m.message.charAt(0).toUpperCase() + m.message.slice(1) + (/[.!?]$/.test(m.message) ? "" : "."));
      return;
    default:
      return;
  }
  render();
}

/** Test trace (only when the app was started with XENIFY_CHAT_LOG). */
function trace(text) {
  if (state.trace) tauri.core.invoke("log", { text }).catch(() => {});
}

function onNet(st) {
  trace(`net ${st}`);
  if (st === "signed_out") return toSignin("You've been signed out. Please sign in again.");
  state.net = st;
  if (st === "online") {
    tell({ t: "list" });
    tell({ t: "find_conversations", q: "" });
    tell({ t: "set_status", status: STATUS_SHARED[state.status] });
    tell({ t: "things" });
    tell({ t: "my_bookings" });
    // Anything sent while the connection was down goes again (the server
    // ignores repeats).
    for (const list of Object.values(state.messages)) for (const x of list) if (x.pending) tell({ t: "send", conv: x.conv, client_id: x.client_id, text: x.text });
  }
  render();
}

// In the browser on a Xenify server: Quit (in your status menu) goes back
// to its Files page.
let onServerPage = false;
if (window.XENIFY_WEB && /^https?:$/.test(location.protocol)) {
  fetch("/v1/info").then((r) => { onServerPage = r.ok && /json/.test(r.headers.get("content-type") || ""); }).catch(() => {});
}

async function boot() {
  let start = { sample: true, server: "https://chat.example.org", user: null, invite: "" };
  if (tauri) {
    await tauri.event.listen("to_app", (e) => onServer(e.payload));
    await tauri.event.listen("net", (e) => onNet(e.payload.state));
    // The server speaks another protocol level: { server, app }.
    await tauri.event.listen("outdated", (e) => { state.outdated = e.payload; render(); });
    // Files sent from this computer (dropped on the window): progress.
    await tauri.event.listen("upload", (e) => {
      const u = state.uploads.find((x) => x.id === e.payload.upload);
      if (u) Object.assign(u, { sent: e.payload.sent, total: e.payload.total });
      render();
    });
    // Files dragged onto the window go to the open chat.
    await tauri.event.listen("tauri://drag-enter", () => { state.dragging = true; render(); });
    await tauri.event.listen("tauri://drag-leave", () => { state.dragging = false; render(); });
    await tauri.event.listen("tauri://drag-drop", (e) => {
      state.dragging = false;
      const c = state.view === "chat" ? current() : null;
      const paths = e.payload?.paths || [];
      if (c?.can_post && paths.length) sendFiles(c, paths);
      else render();
    });
    // In the browser: files dropped on the window go to the open chat.
    if (window.XENIFY_WEB) {
      document.addEventListener("dragover", (e) => {
        if (![...(e.dataTransfer?.types || [])].includes("Files")) return;
        e.preventDefault();
        if (!state.dragging) { state.dragging = true; render(); }
      });
      document.addEventListener("dragleave", (e) => {
        if (e.relatedTarget) return;
        state.dragging = false;
        render();
      });
      document.addEventListener("drop", (e) => {
        if (!e.dataTransfer?.files?.length) return;
        e.preventDefault();
        state.dragging = false;
        const c = state.view === "chat" ? current() : null;
        if (c?.can_post) sendFiles(c, [...e.dataTransfer.files]);
        else render();
      });
    }
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
    await tauri.event.listen("voice_download", (e) => onVoiceDownload(e.payload));
    if (canMakeText()) {
      voiceTest = await tauri.core.invoke("voice_test_audio").catch(() => null);
      // Whether the model's here (received voice messages get their text by themselves).
      loadVoice();
    }
    start = await tauri.core.invoke("start");
    state.trace = !!start.trace;
    // The connection may come up before the rest of this is done: be ready
    // for what it says (and to ask it things) from now on.
    if (!start.sample && start.user) Object.assign(state, { live: true, me: start.user });
    trace("started");
  }
  state.server = start.server.replace(/^https?:\/\//, "");
  state.serverUrl = start.server;
  Object.assign(state, { viaHub: !!start.via_hub, hub: !!start.hub });
  if (tauri) {
    loadProofing();
    await tauri.event.listen("mail", (e) => onMail(e.payload));
    await tauri.event.listen("meetings", (e) => { state.meetings = e.payload; render(); });
    await tauri.event.listen("meeting_invite", (e) => notifyInvite(e.payload));
    await tauri.event.listen("busy", (e) => { for (const b of e.payload) state.busy[b.user] = b; if (state.dialog?.kind === "event") render(); });
    await tauri.event.listen("calendar", () => refreshCalendar());
    await tauri.event.listen("contacts", () => { if (state.view === "people") loadAb(); });
    tauri.core.invoke("meetings_list").then((list) => { state.meetings = list; render(); }).catch(() => {});
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
  // A server with its own certificate trusted here: the web view (calls,
  // pictures) is told again (the server makes a new one at every start).
  if (/^https:/.test(start.server || "") && !window.XENIFY_WEB) tauri.core.invoke("certificate", { fingerprint: null, remember: false }).catch(() => {});
  if (start.user) {
    // (Already online if the connection beat this: then ask again, in case
    // the first asking came too early.)
    const online = state.net === "online";
    Object.assign(state, { me: start.user, view: "chat", net: online ? "online" : "connecting" });
    if (online) onNet("online");
    else render();
  } else {
    state.askServer = !!start.ask_server;
    state.signin.server = start.server.replace(/^https?:\/\//, "");
    toSignin("");
    // Opened with a link: what it's for, then the password.
    if (start.invite) {
      state.signin.join = true;
      openLink("");
    }
  }
}

// ---------- Notifications ----------

// Settings kept with your account on the server, so all your devices
// share them (Settings › Calendar's too: who sees when you're busy and your
// working hours). The rest stay on each device: text size, the voice model and
// the AI spotter (downloaded here), sound and camera devices (chosen in a
// call), starting with the computer, and email accounts.
const SYNCED = ["spelling", "grammar", "before", "desktop", "preview", "sound", "mentions", "quiet", "digest", "meetingRemind", "theme", "voiceIncoming", "voiceSendText", "voiceLang", "meetingTranscript", "calendar", "hiddenCals", "lastCal"];

const syncedNow = () => {
  const out = { status: state.status, tz: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC" };
  for (const k of SYNCED) out[k] = state.settings[k];
  return JSON.stringify(out);
};

let savedSettings = "";
let savedOnServer = null; // what the server has (null: not heard yet)
function saveSettings() {
  if (!tauri || state.view === "loading") return;
  const now = JSON.stringify({ settings: state.settings, status: state.status });
  if (now !== savedSettings) {
    if (savedSettings) tauri.core.invoke("save_settings", { settings: JSON.parse(now) }).catch(() => {});
    savedSettings = now;
  }
  const synced = syncedNow();
  if (state.live && savedOnServer !== null && synced !== savedOnServer) {
    const was = savedOnServer ? JSON.parse(savedOnServer) : {};
    savedOnServer = synced;
    tell({ t: "save_settings", settings: JSON.parse(synced) });
    if (was.meetingRemind !== state.settings.meetingRemind) tauri.core.invoke("cal_meeting_remind", { minutes: Number(state.settings.meetingRemind) }).catch(() => {});
  }
}

/** Your settings from the server (signing in, or changed on another device);
 * the first time, this device's go up instead. */
function onServerSettings(v, connecting) {
  const has = v && typeof v === "object" && Object.keys(v).length > 0;
  // Calendar settings: the server's win; none there yet, this device's go up.
  if (has && v.calendar) tauri.core.invoke("calendar_settings_save", { settings: v.calendar }).catch(() => {});
  else tauri.core.invoke("calendar_settings").then((c) => { state.settings.calendar = c; saveSettings(); }).catch(() => {});
  if (!has) {
    if (connecting) savedOnServer = "";
    return;
  }
  for (const k of SYNCED) if (k in v) state.settings[k] = v[k];
  if (STATUS[v.status] && v.status !== state.status) {
    state.status = v.status;
    tell({ t: "set_status", status: STATUS_SHARED[state.status] });
  }
  // As the server has it (a different time zone here goes up: quiet hours
  // follow the device you used last).
  savedOnServer = JSON.stringify({ ...JSON.parse(syncedNow()), tz: v.tz || "" });
  tauri.core.invoke("cal_meeting_remind", { minutes: Number(state.settings.meetingRemind) }).catch(() => {});
}

/** Whether `text` mentions you (your name, or @ and the name underneath). */
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
  // A reply in a thread you're not in: only when it mentions you.
  if (m.thread != null && state.threads && !(m.thread_people || []).includes(state.me.id) && !mention) return;
  const chatWants = !muted(c) && (c.notify === "all" || (c.notify === "mentions" && mention));
  if (!chatWants && !(mention && st.mentions)) return;
  const who = senderName(c, m.sender);
  const title = c.kind === "direct" ? who : `${who} in ${titleOf(c)}`;
  const body = st.preview ? describe(m.text) : mention ? "Mentioned you" : "New message";
  tauri.core.invoke("notify", { title, body, conv: c.id, sound: !!st.sound }).catch(() => {});
}

// ---------- Email ----------

/** "Ana Ruiz <ana@x.org>" → "Ana Ruiz" (or the address when there's no name). */
function fromName(from) {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]+)>/.exec(from || "");
  return (m && (m[1].trim() || m[2])) || (from || "").trim() || "(no sender)";
}

const hasMail = () => state.mail.accounts.length > 0;
/** The server keeps everyone's email (and this app is signed in to it). */
const serverEmail = () => !!(state.live && state.info?.features?.includes("email"));
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
    const o = state.mail.options;
    if (serverEmail() && o && !o.microsoft && !o.google && !o.passwords) return `<div class="empty mailempty"><p>Email isn't set up on this server yet. Ask your admin to connect the company's email (Admin › Email).</p></div>`;
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
      <option value="drafts" ${sel === "drafts" ? "selected" : ""}>${serverEmail() ? "Drafts" : "Drafts on this computer"}${drafts ? ` (${drafts})` : ""}</option>
      ${groups}
    </select>
    <button class="textbtn" data-act="newmail" title="${several() ? "From any of your accounts" : `From ${esc(state.mail.accounts[0].email)}`}">${icons.plus}New email</button>
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
  // The email's own HTML (already cleaned): no scripts, and nothing loaded from elsewhere.
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'"><base target="_blank"><style>
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
      ${phoneBack()}<div class="title"><span class="avatar big square" style="background:${bg};color:${fg}" aria-hidden="true">${esc(initials(who))}</span><div><h1>${esc(o.subject || "(no subject)")}</h1><div class="sub">${esc(o.from)} · ${esc(o.date)}</div></div></div>
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
      ${phoneBack()}<div class="title"><span class="avatar big square" style="background:var(--tint);color:var(--ink-2)" aria-hidden="true">${icons.mail}</span><div><h1>${esc(k.title)}</h1><div class="sub" id="draftstate">${k.savedAt ? `Draft saved ${hm(k.savedAt)}` : "Kept in Drafts as you write"}</div></div></div>
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
            ${state.live ? `<button type="button" class="textbtn attachbtn" data-act="secret" data-id="mail" title="A password or the like, as a link that opens once">${icons.secret}Share a secret</button>` : ""}
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

/** Where an account's mail comes from, under its name in Settings › Email. */
function keptLine(a) {
  if (a.kind === "company") return "Your company's mailbox, connected by your admin.";
  if (a.kind === "microsoft") return "Signed in with Microsoft. Take Xenify's access back there at any time.";
  if (a.kind === "google") return "Signed in with Google. Take Xenify's access back there at any time.";
  if (a.kept === "on the server") return "Kept on your Xenify server, with its password locked away there.";
  return !a.kept ? "The password isn't saved on this computer: remove the account and add it again." : a.kept.startsWith("system") ? "Password in this computer's keyring." : "Password in a private file only you can read (no keyring here).";
}

function emailSettings() {
  const list = state.mail.accounts.map((a) => `<div class="setting"><div><div class="label">${esc(a.name)} &lt;${esc(a.email)}&gt;</div><div class="hintline">${state.mail.problems[a.email] ? `<span class="problem">${esc(state.mail.problems[a.email])}</span>` : esc(state.mail.status[a.email] || "Up to date")}</div><div class="hintline">${esc(keptLine(a))}</div></div>${a.kind === "company" ? "" : `<button class="small danger-outline" style="height:36px" data-act="mailremove" data-id="${esc(a.email)}">Remove</button>`}</div>`).join("");
  const o = serverEmail() ? state.mail.options || {} : { passwords: true };
  const connect = serverEmail() && (o.microsoft || o.google)
    ? `<div class="connectrow">${o.microsoft ? `<button class="small" data-act="mailconnect" data-id="microsoft">Connect Microsoft mail</button>` : ""}${o.google ? `<button class="small" data-act="mailconnect" data-id="google">Connect Google mail</button>` : ""}</div><p class="hintline">You sign in on Microsoft's or Google's own page; Xenify never sees your password.</p>`
    : "";
  const canAdd = connect || o.passwords;
  const adding = canAdd && (!hasMail() || state.mailSetup);
  const lede = serverEmail()
    ? "Your email sits next to your chats, checked for spelling and grammar like everything you write here. Your Xenify server keeps it, so it's the same in the browser and on every computer you sign in on."
    : "Your email sits next to your chats, checked for spelling and grammar like everything you write here. A copy is kept on this computer, so it opens instantly and can be searched offline.";
  const none = serverEmail() && !canAdd && !hasMail() ? `<div class="panel"><div class="label">Email isn't set up on this server yet</div><div class="hintline">Ask your admin to connect the company's email (Admin › Email).</div></div>` : "";
  return `<h1>Email</h1><p class="lede">${lede}</p>${none}
    ${list ? `${list}<div style="display:flex;gap:8px;margin:14px 0 6px"><button class="small" data-act="mailcheck">Check now</button>${adding || !canAdd ? "" : `<button class="small primary" data-act="mailaddmore">${icons.plus}Add another account</button>`}</div>` : ""}
    ${adding ? `${list ? "<h2>Add an account</h2>" : ""}${connect}${o.passwords ? `${connect ? `<h2 class="subhead">Or with an email password</h2>` : ""}${mailSetupForm()}` : ""}` : ""}`;
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
    state.mail.options = serverEmail() ? await tauri.core.invoke("mail_options").catch(() => null) : null;
  } catch {
    return;
  }
  // Another server's (or this computer's) mail was showing.
  const mine = new Set(state.mail.accounts.map((a) => a.email));
  state.mail.list = state.mail.list.filter((r) => mine.has(r.account));
  if (state.mailSel && !mine.has(state.mailSel.account)) state.mailSel = null;
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
  // Accounts added or taken away (here, in another window, by the admin).
  if (e.kind === "accounts") return initMail().then(render);
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
  if (e.kind === "signin_failed") {
    const a = state.mail.accounts.find((x) => x.email === e.account);
    state.mail.problems[e.account] = a?.kind === "company" ? `${e.text} Your admin can check Admin › Email.`
      : a?.kind === "microsoft" || a?.kind === "google" ? `${e.text} Remove it in Settings › Email and connect it again.`
      : `${e.text} Remove the account in Settings › Email and add it again with the right password.`;
  }
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
    ${can("chat.create_apps") ? `<div style="display:flex;gap:8px;margin:4px 0 8px"><button class="small primary" data-act="newapp">${icons.plus}Make an app</button><button class="small" data-act="apikey">Make an API key for scripts</button></div>` : `<p class="hintline">Your server's admins haven't let you make apps.</p>`}
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

// The server's days off (Admin › Holidays): public holidays and company
// closures, as all-day entries. Calendars › Holidays and closures (-1) hides them.
const HOLIDAYS = -1;
const holidayYears = new Map(); // year → a promise of [{ from, to, name, moved_from, company }]
const holidaysIn = new Map(); // the same, once they're in
async function holidaysOfYear(y) {
  if (!holidayYears.has(y)) {
    holidayYears.set(y, tauri.core.invoke("holidays", { year: y })
      .then((list) => (holidaysIn.set(y, list), list))
      .catch(() => (holidayYears.delete(y), [])));
  }
  return holidayYears.get(y);
}
/** The holiday or closure on the day starting at `day` (local midnight), if
 * the server's are in; asks for its year otherwise and draws again. Find a
 * time leaves these days out, hidden in the calendar or not. */
function holidayOn(day) {
  if (!state.live) return null;
  const y = new Date(day).getFullYear();
  if (!holidaysIn.has(y)) {
    if (!holidayYears.has(y)) holidaysOfYear(y).then(() => holidaysIn.has(y) && render());
    return null;
  }
  const d = dateInput(day);
  return holidaysIn.get(y).find((h) => h.from <= d && d <= h.to) || null;
}
async function holidayEvents(from, to) {
  if (!state.live || (state.settings.hiddenCals || []).includes(HOLIDAYS)) return [];
  const years = [...new Set([new Date(from).getFullYear(), new Date(to - 1).getFullYear()])];
  const day = (s) => fromInputs(s, "00:00");
  return (await Promise.all(years.map(holidaysOfYear))).flat()
    .map((h, i) => ({ id: -1e9 - i * 1e5 - Math.round(day(h.from) / DAY) % 1e5, title: h.name, start: day(h.from), end: addDays(day(h.to), 1), all_day: true, notes: "", kind: "holiday", company: !!h.company, moved_from: h.moved_from || "", link: "", remind: null, reminded: false, done: false, repeat: "", meeting: 0, answer: "" }))
    .filter((e) => e.start < to && e.end > from);
}

async function loadToday() {
  if (!tauri) return;
  const now = Date.now();
  try {
    const list = (await holidayEvents(startOfDay(now), startOfDay(now) + DAY)).concat(await tauri.core.invoke("cal_between", { from: startOfDay(now), to: startOfDay(now) + DAY }));
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
    const from = state.cal.weekStart, to = addDays(state.cal.weekStart, 7);
    state.cal.shared = state.live ? await tauri.core.invoke("shared_cals") : [];
    const hidden = new Set(state.settings.hiddenCals || []);
    const shared = state.live ? (await tauri.core.invoke("shared_cal_between", { from, to })).filter((e) => !hidden.has(e.shared)) : [];
    const mine = hidden.has(0) ? [] : (await tauri.core.invoke("cal_between", { from, to })).concat(bookingEvents(from, to));
    state.cal.week = (await holidayEvents(from, to)).concat(mine, shared);
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
  return `<button class="todayitem${e.kind === "reminder" ? " reminder" : ""}${e.meeting && !e.answer ? " unanswered" : ""}" data-act="calitem" data-id="${e.id}">
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

// Shared calendars' colours (by their place in the list).
const CAL_COLOURS = ["#2563EB", "#0F766E", "#7C3AED", "#B45309", "#BE185D", "#4D7C0F"];
const sharedCal = (conv) => (state.cal.shared || []).find((x) => x.conv === conv);
const calColour = (conv) => CAL_COLOURS[Math.max(0, (state.cal.shared || []).findIndex((x) => x.conv === conv)) % CAL_COLOURS.length];

/** The Calendars menu: yours, and the shared ones (show, share, rename, leave). */
function calsMenu() {
  const hidden = new Set(state.settings.hiddenCals || []);
  const list = state.cal.shared || [];
  return `<div class="menu" role="dialog" aria-label="Calendars" style="right:150px;top:66px;width:340px">
    <div class="label">Calendars</div>
    <div class="person"><input type="checkbox" class="calshow" data-act="calshow" data-id="0" ${hidden.has(0) ? "" : "checked"} aria-label="Show My calendar" style="accent-color:var(--accent)"><span class="body"><b>My calendar</b><span class="hintline">Only you</span></span></div>
    ${state.live ? `<div class="person"><input type="checkbox" class="calshow" data-act="calshow" data-id="${HOLIDAYS}" ${hidden.has(HOLIDAYS) ? "" : "checked"} aria-label="Show Holidays and closures" style="accent-color:#B91C1C"><span class="body"><b>Holidays and closures</b><span class="hintline">Public holidays and days the company is closed, set on the server</span></span></div>` : ""}
    ${list.map((c) => `<div class="person"><input type="checkbox" class="calshow" data-act="calshow" data-id="${c.conv}" ${hidden.has(c.conv) ? "" : "checked"} aria-label="Show ${esc(c.name)}" style="accent-color:${calColour(c.conv)}"><span class="body"><b>${esc(c.name)}</b><span class="hintline">${c.yours ? "You can change it" : "You can look"} · ${c.member_count} ${c.member_count === 1 ? "person" : "people"}</span></span>${c.yours ? `<button class="small quiet" data-act="calshare" data-id="${c.conv}">Share</button>` : ""}<button class="small quiet" data-act="calmore" data-id="${c.conv}" aria-label="More for ${esc(c.name)}">⋯</button></div>`).join("")}
    ${list.length ? "" : `<p class="hintline" style="padding:0 10px">A shared calendar is one several people see: a team's holidays, a rota, a project's dates. You choose who can change it.</p>`}
    <button role="menuitem" class="withicon" data-act="calsharednew">${icons.plus}New shared calendar…</button>
  </div>`;
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
      const sc = e.shared ? `;--cal:${calColour(e.shared)}` : "";
      return `<button class="calevent${e.shared ? " shared" : ""}${e.kind === "reminder" ? " reminder" : ""}${e.done ? " done" : ""}${e.meeting && !e.answer ? " unanswered" : ""}${e.answer === "maybe" ? " maybe" : ""}" style="top:${top}px;height:${h - 2}px;left:calc(${c * w}% + 2px);width:calc(${w}% - 4px)${sc}" data-act="calitem" data-id="${e.id}" title="${esc(e.title)}${e.shared ? ` · ${esc(sharedCal(e.shared)?.name || "Shared calendar")}` : ""}">
        <b>${e.repeat ? `<span class="repeaticon" title="Repeats" aria-label="Repeats">↻</span>` : ""}${e.kind === "reminder" ? icons.bell.replace('width="20" height="20"', 'width="12" height="12"') : ""}${esc(e.title)}</b>${h > 40 ? `<span>${hm(e.start)}${e.end > e.start ? `–${hm(e.end)}` : ""}${e.meeting && !e.answer ? " · not answered" : e.answer === "maybe" ? " · maybe" : ""}</span>` : ""}
      </button>`;
    }).join("");
    return `<div class="calday${d === today ? " istoday" : ""}" data-act="calslot" data-id="${i}">${blocks}${nowLine(d)}</div>`;
  }).join("");
  const hours = [...Array(24)].map((_, h) => `<div class="calhour"><span>${h === 0 ? "" : `${pad(h)}:00`}</span></div>`).join("");
  return `<main class="sheet calendar">
    <header class="sheet-head">
      ${phoneBack()}<div class="title"><div><h1>Calendar</h1><div class="sub">${esc(label)}</div></div></div>
      <div class="actions">
        <button class="iconbtn" data-act="calprev" aria-label="Previous week">${icons.back}</button>
        <button class="textbtn" data-act="caltoday">This week</button>
        <button class="iconbtn" data-act="calnext" aria-label="Next week" style="transform:scaleX(-1)">${icons.back}</button>
        ${state.live ? `<button class="textbtn" data-act="calsmenu" aria-expanded="${state.menu === "cals"}">${icons.people}Calendars</button>` : ""}
        ${state.live ? `<button class="textbtn" data-act="awayopen">${icons.plane}Out of office</button>` : ""}
        <button class="small primary" data-act="calnew">${icons.plus}New meeting</button>
        ${state.menu === "cals" ? calsMenu() : ""}
      </div>
    </header>
    <div class="calhead"><div class="calgutter"></div>${days.map((d) => `<div class="caldayname${d === today ? " istoday" : ""}"><span>${new Date(d).toLocaleDateString([], { weekday: "short" })}</span><b>${new Date(d).getDate()}</b></div>`).join("")}</div>
    ${hasAllDay ? `<div class="calallday"><div class="calgutter hintline">All day</div>${allDay.map((l) => `<div>${l.map((e) => `<button class="calchip${e.kind === "away" ? " away" : e.kind === "booking" ? " booking" : e.kind === "holiday" ? " holiday" : ""}${e.shared ? " shared" : ""}" ${e.shared ? `style="--cal:${calColour(e.shared)}" title="${esc(sharedCal(e.shared)?.name || "")}"` : ""} data-act="calitem" data-id="${e.id}"${e.kind === "holiday" ? ` title="${esc(holidayNote(e))}"` : ""}>${esc(e.title)}</button>`).join("")}</div>`).join("")}</div>` : ""}
    <div class="calscroll" id="calscroll"><div class="calgrid" style="height:${24 * HOUR_PX}px"><div class="calhours">${hours}</div>${cols}</div></div>
  </main>`;
}

/** What a holiday chip says when you point at it or click it. */
function holidayNote(e) {
  if (e.company) return `${e.title}: the company is closed${e.end - e.start > DAY ? ` ${new Date(e.start).toLocaleDateString([], { day: "numeric", month: "short" })} – ${new Date(e.end - DAY).toLocaleDateString([], { day: "numeric", month: "short" })}` : ""}`;
  return `${e.title}: a public holiday${e.moved_from ? ` (moved from ${new Date(fromInputs(e.moved_from)).toLocaleDateString([], { weekday: "long", day: "numeric", month: "short" })})` : ""}`;
}

function openCalendar(at = Date.now()) {
  holidayYears.clear(); // an admin may have changed them
  holidaysIn.clear();
  if (state.compose) return void parkCompose().then(() => openCalendar(at));
  Object.assign(state, { view: "calendar", mailSel: null, menu: null });
  state.cal.weekStart = startOfWeek(at);
  loadWeek();
}

/** The event editor (a dialog); with people in it, the meeting editor. */
function eventDialog(d) {
  if (isMeeting(d)) return meetingEditor(d);
  const e = d.event;
  const link = e.link ? JSON.parse(e.link) : null;
  // Shared calendars: which one (new events), or only looking.
  const sc = e.shared ? sharedCal(e.shared) : null;
  const lookOnly = !!sc && !sc.yours;
  const mine = (state.cal.shared || []).filter((x) => x.yours);
  const calPick = sc
    ? `<p class="hintline" style="margin:-4px 0 0"><span class="caldot" style="background:${calColour(sc.conv)}"></span> In ${esc(sc.name)}${lookOnly ? " · you can look, not change it" : ""}</p>`
    : !e.id && e.kind !== "reminder" && mine.length && !d.picked.length
      ? `<label class="field">Calendar<select id="ev_cal"><option value="0">My calendar</option>${mine.map((x) => `<option value="${x.conv}" ${d.cal === x.conv ? "selected" : ""}>${esc(x.name)}</option>`).join("")}</select></label>`
      : "";
  const remindOpts = [[null, "No reminder"], [0, "When it starts"], [1, "1 minute before"], [5, "5 minutes before"], [10, "10 minutes before"], [15, "15 minutes before"], [30, "30 minutes before"], [60, "1 hour before"], [1440, "1 day before"]];
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${e.id ? "Change" : "New"} ${e.kind === "reminder" ? "reminder" : "event"}">
    <h2>${e.id ? (e.kind === "reminder" ? "Reminder" : "Event") : e.kind === "reminder" ? "New reminder" : "New event"}</h2>
    ${link ? `<div class="panel" style="display:flex;align-items:center;justify-content:space-between;gap:10px"><span class="hintline">About ${esc(link.label || "")}</span><button class="small" type="button" data-act="calopenlink">Open</button></div>` : ""}
    <fieldset class="bare" ${lookOnly ? "disabled" : ""}>
    <label class="field">${e.kind === "reminder" ? "Remind me to" : "Name"}<input id="ev_title" value="${esc(e.title)}" autocomplete="off" required></label>
    ${calPick}
    ${e.kind === "reminder" || e.ical_uid || !state.live || e.shared ? "" : whoField(d)}
    <div class="grid3 evtime">
      <label class="field">Date<input id="ev_date" type="date" value="${dateInput(e.start)}" required></label>
      ${e.all_day ? "" : `<label class="field">${e.kind === "reminder" ? "At" : "From"}<input id="ev_from" type="time" value="${timeInput(e.start)}"></label>`}
      ${e.all_day || e.kind === "reminder" ? "" : `<label class="field">To<input id="ev_to" type="time" value="${timeInput(e.end)}"></label>`}
    </div>
    ${e.kind === "reminder" ? "" : `<label class="choice" style="border:none;padding:0"><input id="ev_allday" type="checkbox" ${e.all_day ? "checked" : ""}> All day</label>
    ${e.shared ? "" : `<label class="field">Remind me<select id="ev_remind">${remindOpts.map(([v, l]) => `<option value="${v ?? ""}" ${(e.remind ?? null) === v ? "selected" : ""}>${l}</option>`).join("")}</select></label>`}`}
    ${e.kind === "reminder" ? "" : `<label class="field">Repeat<select id="ev_repeat">${repeatChoices(e.repeat)}</select>${e.repeat ? `<span class="hintline">Changes here change every time it happens; the date is the first one.</span>` : ""}</label>`}
    <label class="field">Notes<textarea id="ev_notes" rows="2">${esc(e.notes)}</textarea></label>
    </fieldset>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons">
      ${lookOnly ? `<button class="small primary" type="button" data-act="closedialog">Close</button>` : `
      ${e.id ? `<button class="small danger-outline" type="button" data-act="caldelete" style="margin-right:auto;height:36px">Delete</button>` : ""}
      ${e.id && e.kind === "reminder" ? `<button class="small" type="button" data-act="caldone">${e.done ? "Not done" : "Done"}</button>` : ""}
      <button class="small" type="button" data-act="closedialog">Cancel</button>
      <button class="small primary" type="submit">Save</button>`}
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

function editEvent(event, meeting = null) {
  // One appearance of a repeating event: change the series.
  if (event.repeat && event.series_start) event = { ...event, start: event.series_start, end: event.series_end };
  state.dialog = {
    kind: "event", event: { ...event }, error: "", q: "", day: startOfDay(event.start),
    // A meeting: who's invited (people on the server, and by email), where.
    picked: meeting ? meeting.people.map((p) => p.user).filter((u) => u.id !== state.me.id) : [],
    guests: meeting ? meeting.guests.map((g) => ({ ...g })) : [],
    place: meeting?.place || "", where: meeting?.place ? "place" : "call", meeting: meeting?.conv || 0,
    things: meeting?.things ? meeting.things.map((t) => ({ ...t })) : [], also_call: meeting ? !!meeting.also_call : true, found: [],
    // A new event goes in the calendar used last (while you can still change it).
    cal: !event.id && sharedCal(state.settings.lastCal)?.yours ? state.settings.lastCal : 0,
  };
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
  const d = state.dialog;
  const place = document.getElementById("ev_place");
  if (place) d.place = place.value;
  const where = document.querySelector('input[name="ev_where"]:checked');
  if (where) d.where = where.value;
  const len = document.getElementById("ev_length");
  if (len) e.end = e.start + Number(len.value) * MIN;
  const also = document.getElementById("ev_alsocall");
  if (also) d.also_call = also.checked;
  return e;
}

async function saveEvent() {
  const d = state.dialog;
  const e = readEventDialog();
  if (isMeeting(d)) return saveMeeting(d, e);
  const into = e.shared || Number(document.getElementById("ev_cal")?.value || 0);
  if (into && !sharedCal(into)?.yours) return ((state.dialog = null), render());
  try {
    if (into) await tauri.core.invoke("shared_cal_save", { conv: into, event: { ...e, id: 0, shared: 0 } });
    else await tauri.core.invoke("cal_save", { event: e });
    if (!e.id && !e.shared && (state.settings.lastCal || 0) !== into) {
      state.settings.lastCal = into;
      saveSettings();
    }
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

// ---------- Address books, rooms and things ----------
// Beside your chats, like the calendar: My contacts, shared books, everyone
// on the server, and the rooms and things people book.

const KINDS = [["room", "Room", "Rooms"], ["equipment", "Equipment", "Equipment"], ["book", "Book", "Books"], ["vehicle", "Vehicle", "Vehicles"], ["other", "Something else", "Other things"]];
const thingIcons = {
  room: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 21h18M5 21V4a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v17M14 12h.01"/></svg>`,
  equipment: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="7" width="20" height="10" rx="2"/><circle cx="16" cy="12" r="2.5"/></svg>`,
  book: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"/></svg>`,
  vehicle: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 17h14v-5l-2-5H7l-2 5zM7 17v2M17 17v2"/></svg>`,
  other: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3"/></svg>`,
};
const thingIcon = (t) => thingIcons[t.kind] || thingIcons.other;
const thingOf = (id) => state.things.list.find((t) => t.id === id);
const live = (b) => b.status === "yes" || b.status === "asked";
const bookingsOf = (id) => Object.values(state.things.byId).filter((b) => b.thing === id && live(b)).sort((a, b) => a.start - b.start);
const dayLabel = (t) => new Date(t).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });

function openPeople(view = state.ab.view) {
  if (state.compose) return void parkCompose().then(() => openPeople(view));
  Object.assign(state, { view: "people", mailSel: null, menu: null });
  if (view !== state.ab.view) Object.assign(state.ab, { view, sel: null });
  loadAb();
}

/** Fetches what the shown book needs (and the list of books). */
async function loadAb() {
  if (!tauri) return;
  const ab = state.ab;
  try {
    ab.books = await tauri.core.invoke("books_list");
    if (ab.view === "mine" || typeof ab.view === "number") ab.list = await tauri.core.invoke("contacts_list", { book: ab.view === "mine" ? 0 : ab.view });
    if (ab.view === "mine") ab.mineCount = ab.list.length;
  } catch (e) {
    showToast(String(e));
  }
  if (ab.view === "everyone") tell({ t: "find_people", q: "" });
  if (ab.view === "things") {
    tell({ t: "things" });
    const ids = state.things.list.map((t) => t.id);
    if (ids.length) tell({ t: "bookings", things: ids, from: startOfDay(Date.now()) - DAY, to: startOfDay(Date.now()) + 30 * DAY });
  }
  render();
}

function peopleSheet() {
  const ab = state.ab;
  const q = ab.q.trim().toLowerCase();
  const book = typeof ab.view === "number" ? ab.books.find((b) => b.conv === ab.view) : null;
  const canChange = ab.view === "mine" || book?.yours;
  const navItem = (view, label, n) => `<button class="book${ab.view === view ? " sel" : ""}" data-act="abview" data-id="${view}">${esc(label)}<span class="n">${n ?? ""}</span></button>`;
  const nav = `<nav class="abnav" aria-label="Contacts">
    ${navItem("mine", "My contacts", ab.view === "mine" ? ab.list.length : ab.mineCount)}
    ${ab.books.length ? `<div class="grp">Shared</div>` : ""}
    ${ab.books.map((b) => navItem(b.conv, b.name, b.count)).join("")}
    <button class="book add" data-act="booknew">+ New contact list</button>
    <div class="grp">This server</div>
    ${navItem("everyone", "Everyone here", ab.directory.filter((u) => !u.app).length || "")}
    ${navItem("things", "Rooms and things", state.things.list.length || "")}
    <span class="hintline abnote">Contacts are kept on the server.</span>
  </nav>`;
  let rows = "";
  let detail = "";
  let actions = "";
  if (ab.view === "everyone") {
    const list = ab.directory.filter((u) => !u.app && (!q || nameOf(u).toLowerCase().includes(q) || (u.email || "").toLowerCase().includes(q)));
    rows = list.map((u) => `<button class="person${ab.sel === u.id ? " on" : ""}" data-act="absel" data-id="${u.id}">${personAvatar(u, "small")}<span class="who"><b>${esc(nameOf(u))}</b><span class="hintline">${esc(handleOf(u))}</span></span></button>`).join("") || `<p class="hintline abempty">${ab.directory.length ? "Nobody by that name." : "Looking…"}</p>`;
    const u = ab.directory.find((x) => x.id === ab.sel);
    if (u) detail = personDetail(u);
  } else if (ab.view === "things") {
    const list = state.things.list.filter((t) => !q || t.name.toLowerCase().includes(q) || t.place.toLowerCase().includes(q));
    rows = KINDS.map(([k, , plural]) => {
      const here = list.filter((t) => t.kind === k);
      return here.length ? `<div class="grp">${plural}</div>` + here.map((t) => `<button class="person${ab.sel === t.id ? " on" : ""}" data-act="absel" data-id="${t.id}"><span class="avatar small square thingav" aria-hidden="true">${thingIcon(t)}</span><span class="who"><b>${esc(t.name)}</b><span class="hintline">${esc(thingLine(t))}</span></span></button>`).join("") : "";
    }).join("") || `<p class="hintline abempty">${state.things.admin ? "Nothing to book yet. Add rooms, equipment, books or anything else people borrow." : "Nothing to book yet. Admins add rooms and things here."}</p>`;
    const t = thingOf(ab.sel);
    if (t) detail = thingDetail(t);
    if (state.things.admin) actions = `<button class="small primary" data-act="thingnew">${icons.plus}Add something to book</button>`;
  } else {
    const list = ab.list.filter((x) => !q || x.name.toLowerCase().includes(q) || x.email.toLowerCase().includes(q));
    rows = list.map((x) => `<button class="person${ab.sel === x.uid ? " on" : ""}" data-act="absel" data-id="${esc(x.uid)}">${personAvatar({ id: x.uid, username: x.name, display_name: x.name }, "small")}<span class="who"><b>${esc(x.name)}</b><span class="hintline">${esc(x.email || (x.username ? "On this server" : x.phone))}</span></span></button>`).join("")
      || `<p class="hintline abempty">${ab.list.length ? "Nobody by that name." : ab.view === "mine" ? "Nobody yet. People you email or invite to a meeting are added here by themselves." : "Nobody in this list yet."}</p>`;
    const x = ab.list.find((c) => c.uid === ab.sel);
    if (x) detail = contactDetail(x, canChange);
    if (canChange) actions = `<button class="small primary" data-act="contactnew">${icons.plus}Add</button>`;
    if (book) actions = (book.yours ? `<button class="small" data-act="bookshare" data-id="${book.conv}">Share</button>` : `<button class="small" data-act="bookleave" data-id="${book.conv}">Leave</button>`) + actions;
  }
  return `<main class="sheet abook">
    <header class="sheet-head">
      ${phoneBack(detail ? "abunsel" : "phoneback")}<div class="title"><div><h1>Contacts</h1>${book ? `<div class="sub">${esc(book.name)} · ${book.yours ? "you can change it" : "you can look"} · shared with ${book.people.length} ${book.people.length === 1 ? "person" : "people"}${book.groups?.length ? ` (${esc(book.groups.map((g) => g.title).join(", "))})` : ""}</div>` : ""}</div></div>
      <div class="actions"><label class="sr" for="abq">Find</label><input id="abq" class="abq" value="${esc(ab.q)}" placeholder="${ab.view === "things" ? "Find something" : "Find someone"}" autocomplete="off">${actions}</div>
    </header>
    <div class="abcols${detail ? " picked" : ""}">${nav}<section class="ablist" aria-label="List">${rows}</section><section class="abdetail" aria-label="Details">${detail || `<p class="hintline abempty">${ab.view === "things" ? "Pick something to see when it's free." : "Pick someone to see their details."}</p>`}</section></div>
  </main>`;
}

function contactDetail(x, canChange) {
  const user = x.username ? state.ab.directory.find((u) => u.username === x.username) || { id: x.username, username: x.username, display_name: x.name } : null;
  return `<div class="abhead">${personAvatar({ id: x.uid, username: x.name, display_name: x.name }, "huge")}<div style="flex:1;min-width:0"><h2>${esc(x.name)}</h2><div class="hintline">${esc([x.role, x.company].filter(Boolean).join(", "))}</div></div>${canChange ? `<button class="small" data-act="contactedit" data-id="${esc(x.uid)}">Edit</button>` : ""}</div>
    <div class="abbtns">
      ${x.email && hasMail() ? `<button class="small" data-act="abemail" data-id="${esc(x.email)}">${icons.mail}Email</button>` : ""}
      ${x.username ? `<button class="small" data-act="abmessage" data-id="${esc(x.username)}">${icons.chat}Message</button>` : ""}
      <button class="small" data-act="abinvite" data-id="${esc(x.uid)}">${icons.calendar.replace('width="20" height="20"', 'width="18" height="18"')}Invite to a meeting</button>
    </div>
    <div class="kv">${[["Email", x.email], ["Phone", x.phone], ["Company", x.company], ["Job", x.role], ["On this server", x.username ? "Yes" : ""], ["Notes", x.notes]].filter(([, v]) => v).map(([k, v]) => `<span>${k}</span><span>${esc(v)}</span>`).join("")}</div>
    ${x.book === 0 && state.ab.books.some((b) => b.yours) ? `<div class="abin"><b>Also put in</b><div class="words">${state.ab.books.filter((b) => b.yours).map((b) => `<button class="small" data-act="abcopy" data-id="${b.conv}">${esc(b.name)}</button>`).join("")}</div></div>` : ""}
    ${x.book !== 0 ? `<div class="abin"><button class="small" data-act="abcopy" data-id="0">Add to My contacts</button></div>` : ""}
    ${canChange ? `<button class="linkbtn danger" data-act="contactdelete" data-id="${esc(x.uid)}" style="align-self:flex-start;padding:0">Delete</button>` : ""}
    ${x.added ? `<span class="hintline" style="margin-top:auto">${esc(x.added)}</span>` : ""}
    ${user ? "" : ""}`;
}

function personDetail(u) {
  const known = state.ab.list.some((x) => x.username === u.username);
  return `<div class="abhead">${personAvatar(u, "huge")}<div style="flex:1;min-width:0"><h2>${esc(nameOf(u))}</h2><div class="hintline">${esc([u.email, presenceOf(u).label].filter(Boolean).join(" · "))}</div></div></div>
    <div class="abbtns">
      <button class="small" data-act="abmessage" data-id="${esc(u.username)}">${icons.chat}Message</button>
      <button class="small" data-act="abinviteuser" data-id="${u.id}">${icons.calendar.replace('width="20" height="20"', 'width="18" height="18"')}Invite to a meeting</button>
      ${known ? "" : `<button class="small" data-act="abadduser" data-id="${u.id}">${icons.plus}Add to My contacts</button>`}
    </div>`;
}

/** "8 seats · 2nd floor · free now" and the like. */
function thingLine(t) {
  const now = Date.now();
  const cur = bookingsOf(t.id).find((b) => b.status === "yes" && b.start <= now && b.end > now);
  const bits = [t.kind === "room" && t.seats ? `${t.seats} seats` : "", t.place];
  bits.push(cur ? (t.by_day ? `with ${nameOf(cur.user).split(" ")[0]} until ${dayLabel(cur.end - 1)}` : `booked until ${hm(cur.end)}`) : "free now");
  return bits.filter(Boolean).join(" · ");
}

function thingDetail(t) {
  const d = state.dialog?.kind === "booking" && state.dialog.thing === t.id ? state.dialog : null;
  const now = Date.now();
  const list = bookingsOf(t.id);
  const cur = list.find((b) => b.status === "yes" && b.start <= now && b.end > now);
  const today = startOfDay(now);
  // Two weeks for things lent by the day; today's bookings for the rest.
  const strip = t.by_day
    ? `<div class="strip">${[...Array(14)].map((_, i) => {
        const day = addDays(today, i);
        const b = list.find((x) => x.start < addDays(day, 1) && x.end > day);
        const cls = !b ? "" : b.status === "asked" ? " asked" : " taken";
        return `<span class="stripday${cls}" title="${esc(dayLabel(day))}${b ? ` · ${esc(nameOf(b.user))}${b.status === "asked" ? " (asked)" : ""}` : " · free"}"><i>${new Date(day).toLocaleDateString([], { weekday: "narrow" })}</i></span>`;
      }).join("")}</div><span class="hintline">Grey: booked. Orange: asked for, waiting for an answer.</span>`
    : `<div class="daylist">${list.filter((b) => b.end > now && b.start < addDays(today, 7)).slice(0, 8).map((b) => `<div class="whorow"><span>${esc(dayLabel(b.start))} · ${hm(b.start)}–${hm(b.end)}</span><span class="hintline">${esc(nameOf(b.user))}${b.status === "asked" ? " · asked" : ""}</span></div>`).join("") || `<span class="hintline">Not booked in the next week.</span>`}</div>`;
  const who = t.approver ? nameOf(t.approver) : "";
  const mine = list.filter((b) => b.user.id === state.me.id && b.end > now);
  const f = d || { from: t.by_day ? addDays(today, 1) : Math.ceil(now / HOUR) * HOUR, to: t.by_day ? addDays(today, 2) : Math.ceil(now / HOUR) * HOUR + HOUR };
  return `<div class="abhead"><span class="avatar huge square thingav" aria-hidden="true">${thingIcon(t)}</span><div style="flex:1;min-width:0"><h2>${esc(t.name)}</h2><div class="hintline">${esc([KINDS.find(([k]) => k === t.kind)?.[1], t.place, t.kind === "room" && t.seats ? `${t.seats} seats` : "", t.by_day ? "lent for days" : "booked by the hour"].filter(Boolean).join(" · "))}</div></div>${state.things.admin ? `<button class="small" data-act="thingedit" data-id="${t.id}">Change</button>` : ""}</div>
    ${cur ? `<div class="nowbanner"><b>${t.by_day ? `With ${esc(nameOf(cur.user))}` : `Booked by ${esc(nameOf(cur.user))}`}</b> until ${esc(t.by_day ? dayLabel(cur.end - 1) : hm(cur.end))}</div>` : ""}
    <div class="abblock"><b>${t.by_day ? "Next two weeks" : "This week"}</b>${strip}</div>
    ${mine.length ? `<div class="abblock"><b>Your bookings</b>${mine.map((b) => `<div class="bookingrow"><span class="when">${esc(bookingWhen(t, b))}</span><span class="answer ${b.status === "yes" ? "yes" : "maybe"}">${b.status === "yes" ? "Booked" : `Waiting for ${esc(who.split(" ")[0])}`}</span><button class="small" data-act="bookingcancel" data-id="${b.id}">Call off</button></div>`).join("")}</div>` : ""}
    <form class="bookit" data-act="bookit" data-id="${t.id}">
      <b>Book it</b>
      ${t.by_day
        ? `<div class="bookrow"><label>From<input type="date" id="bk_from" value="${dateInput(f.from)}" required></label><label>To<input type="date" id="bk_to" value="${dateInput(f.to - 1)}" required></label></div>`
        : `<div class="bookrow"><label>Day<input type="date" id="bk_day" value="${dateInput(f.from)}" required></label><label>From<input type="time" id="bk_from" value="${timeInput(f.from)}"></label><label>To<input type="time" id="bk_to" value="${timeInput(f.to)}"></label></div>`}
      <span class="hintline">${t.approver && t.approver.id !== state.me.id ? `${esc(who)} looks after it and says yes or no. You'll get a message either way.` : t.kind === "room" ? "Booked at once if it's free. Booking it in a meeting's Who books it with the meeting." : "Booked at once if it's free."}</span>
      <div class="buttons"><button class="small primary" type="submit">${t.approver && t.approver.id !== state.me.id ? `Ask ${esc(who.split(" ")[0])}` : "Book it"}</button></div>
    </form>
    <div class="kv" style="margin-top:auto">${who ? `<span>Looked after by</span><span>${esc(who)}</span>` : ""}<span>Who can book</span><span>Everyone on this server</span></div>`;
}

/** "Mon, 26 Oct", "Mon, 26 Oct – Wed, 28 Oct" or "Mon, 26 Oct · 10:00–11:00". */
function bookingWhen(t, b) {
  if (!t.by_day) return `${dayLabel(b.start)} · ${hm(b.start)}–${hm(b.end)}`;
  const last = b.end - 1;
  return startOfDay(last) === startOfDay(b.start) ? dayLabel(b.start) : `${dayLabel(b.start)} – ${dayLabel(last)}`;
}

function bookIt(id) {
  const t = thingOf(id);
  const v = (x) => document.getElementById(x)?.value;
  let start, end;
  if (t.by_day) {
    start = fromInputs(v("bk_from"), "00:00");
    end = addDays(fromInputs(v("bk_to"), "00:00"), 1);
  } else {
    start = fromInputs(v("bk_day"), v("bk_from"));
    end = fromInputs(v("bk_day"), v("bk_to"));
  }
  if (!(end > start)) return showToast("It needs to end after it starts.");
  tell({ t: "book", thing: id, start, end, meeting: null, tag: "page" });
}

/** A booking made, answered or called off. */
function onBooked(b, tag) {
  const was = state.things.byId[b.id];
  state.things.byId[b.id] = b;
  const t = thingOf(b.thing);
  // Something added since the list came: fetch it again (this comes again).
  if (!t) {
    delete state.things.byId[b.id];
    state.things.later = [...(state.things.later || []), [b, tag]];
    return tell({ t: "things" });
  }
  const me = state.me?.id;
  if (tag === "page") showToast(b.status === "yes" ? `Booked: ${t.name}.` : `Asked ${nameOf(t.approver || b.user)}. You'll hear back here.`);
  if (b.user.id === me && was?.status === "asked" && (b.status === "yes" || b.status === "no")) {
    const text = `${t.approver ? nameOf(t.approver) : "They"} said ${b.status === "yes" ? "yes" : "no"} to your booking of ${t.name}.`;
    showToast(text);
    if (tauri && state.settings.desktop && state.status !== "dnd") tauri.core.invoke("notify", { title: t.name, body: text, conv: -3, sound: false }).catch(() => {});
  }
  if (t.approver?.id === me && b.user.id !== me && b.status === "asked" && !was) {
    if (tauri && state.settings.desktop && state.status !== "dnd") tauri.core.invoke("notify", { title: `${nameOf(b.user)} asks for ${t.name}`, body: bookingWhen(t, b), conv: -3, sound: !!state.settings.sound }).catch(() => {});
  }
}

/** Requests waiting for you (you look after something), in the sidebar. */
function requestsCard() {
  if (!tauri || !state.me) return "";
  const list = Object.values(state.things.byId).filter((b) => b.status === "asked" && b.user.id !== state.me.id && thingOf(b.thing)?.approver?.id === state.me.id && b.end > Date.now());
  if (!list.length) return "";
  return `<section class="today invites" aria-label="Booking requests">
    <div class="todayhead"><b>Booking requests · ${list.length}</b></div>
    ${list.slice(0, 3).map((b) => {
      const t = thingOf(b.thing);
      return `<div class="invitecard"><span><b>${esc(nameOf(b.user))}</b> asks for <b>${esc(t.name)}</b></span><span class="hintline">${esc(bookingWhen(t, b))}</span>
        <div class="goingrow"><button class="small" data-act="bookingyes" data-id="${b.id}">Yes</button><button class="small" data-act="bookingno" data-id="${b.id}">No</button></div></div>`;
    }).join("")}
  </section>`;
}

function contactDialog(d) {
  const x = d.contact;
  const f = (id, label, v, type = "text") => `<label class="field">${label}<input id="${id}" type="${type}" value="${esc(v)}" autocomplete="off"></label>`;
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${x.uid ? "Change contact" : "New contact"}">
    <h2>${x.uid ? esc(x.name) : "New contact"}</h2>
    ${f("ct_name", "Name", x.name)}${f("ct_email", "Email", x.email, "email")}
    <div class="grid2">${f("ct_phone", "Phone", x.phone, "tel")}${f("ct_company", "Company", x.company)}</div>
    ${f("ct_role", "Job", x.role)}
    <label class="field">Notes<textarea id="ct_notes" rows="2">${esc(x.notes)}</textarea></label>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons"><button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">Save</button></div>
  </form></div>`;
}

async function saveContact() {
  const d = state.dialog;
  const v = (id) => document.getElementById(id)?.value ?? "";
  const contact = { ...d.contact, name: v("ct_name"), email: v("ct_email"), phone: v("ct_phone"), company: v("ct_company"), role: v("ct_role"), notes: v("ct_notes") };
  try {
    const saved = await tauri.core.invoke("contact_save", { contact });
    state.dialog = null;
    state.ab.sel = saved.uid;
    await loadAb();
  } catch (e) {
    d.error = String(e);
    render();
  }
}

/** New shared book, or sharing one: people, and who can change it. */
function bookDialog(d) {
  const already = new Set([state.me.id, ...d.picked.map((u) => u.id)]);
  const results = d.q.trim() ? state.people.filter((u) => !already.has(u.id) && !u.app) : [];
  const book = d.conv ? (d.cal ? state.cal.shared : state.ab.books).find((b) => b.conv === d.conv) : null;
  // Your group chats: share with everyone in one, now and as it changes.
  const q = d.q.trim().toLowerCase();
  const groupHits = q && !d.cal ? state.conversations.filter((c) => c.kind === "group" && titleOf(c).toLowerCase().includes(q) && !d.groups.some((g) => g.conv === c.id)) : [];
  const groupRow = (g) => `<div class="whorow"><span class="avatar small" style="background:#FFF1D6;color:#7A4A00" aria-hidden="true">${icons.people.replace('width="20" height="20"', 'width="16" height="16"')}</span><span>${esc(g.title)}<span class="hintline">Group · ${g.count} ${g.count === 1 ? "person" : "people"}, and whoever joins</span></span><select class="can" data-group="${g.conv}" aria-label="What people in ${esc(g.title)} can do"><option value="change" ${g.owner ? "selected" : ""}>Can change</option><option value="look" ${g.owner ? "" : "selected"}>Can look</option><option value="out">Take out</option></select></div>`;
  const row = (u, owner, gone) => `<div class="whorow">${personAvatar(u, "small")}<span>${esc(nameOf(u))}${u.id === state.me.id ? " (you)" : ""}</span>${u.id === state.me.id ? `<span class="hintline">Can change</span>` : `<select class="can" data-id="${u.id}" aria-label="What ${esc(nameOf(u))} can do"><option value="change" ${owner ? "selected" : ""}>Can change</option><option value="look" ${owner ? "" : "selected"}>Can look</option>${gone ? `<option value="out">Take out</option>` : ""}</select>`}</div>`;
  const what = d.cal ? "calendar" : "contact list";
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${book ? `Share ${esc(book.name)}` : `New ${what}`}">
    <h2>${book ? `Share “${esc(book.name)}”` : d.cal ? "New shared calendar" : "New contact list"}</h2>
    ${book ? "" : `<label class="field">Name<input id="dtitle" value="${esc(d.title)}" placeholder="${d.cal ? "e.g. Team holidays" : "e.g. Clients"}" autocomplete="off" required></label>`}
    <label class="field">${d.cal ? "People on this server" : "People or groups on this server"}<input id="pickq" value="${esc(d.q)}" placeholder="${d.cal ? "Add someone" : "Add someone or a group"}" autocomplete="off"></label>
    ${results.length || groupHits.length ? `<div class="pickresults">${groupHits.slice(0, 3).map((c) => `<button type="button" class="row compact" data-act="pickgroup" data-id="${c.id}"><span class="avatar small" style="background:#FFF1D6;color:#7A4A00" aria-hidden="true">${icons.people.replace('width="20" height="20"', 'width="16" height="16"')}</span><span class="body"><span class="name">${esc(titleOf(c))}</span><span class="last">Group · ${c.member_count} people</span></span></button>`).join("")}${results.slice(0, 5).map((u) => `<button type="button" class="row compact" data-act="pick" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${esc(nameOf(u))}</span><span class="last">${esc(handleOf(u))}</span></span></button>`).join("")}</div>` : ""}
    <div class="wholist">
      ${d.groups.filter((g) => !g.out).map(groupRow).join("")}
      ${row(state.me, true)}
      ${(book ? book.people.filter((p) => p.user.id !== state.me.id) : []).map((p) => row(p.user, d.roles[p.user.id] ?? p.owner, true)).join("")}
      ${d.picked.map((u) => row(u, d.roles[u.id] ?? false, false)).join("")}
    </div>
    <p class="hintline">${d.groups.length ? "People joining a group get it too; people leaving lose it (unless you shared it with them yourself). " : ""}Taking someone out stops them seeing changes from then on.</p>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons">${book && !d.cal ? `<button class="linkbtn" type="button" data-act="bookvcard" data-id="${book.conv}" style="margin-right:auto">Save as vCard</button>` : ""}<button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">${book ? "Save" : "Make it"}</button></div>
  </form></div>`;
}

async function saveBook() {
  const d = state.dialog;
  for (const sel of document.querySelectorAll("select.can[data-id]")) d.roles[Number(sel.dataset.id)] = sel.value === "out" ? "out" : sel.value === "change";
  for (const sel of document.querySelectorAll("select.can[data-group]")) {
    const g = d.groups.find((x) => x.conv === Number(sel.dataset.group));
    if (g) Object.assign(g, { owner: sel.value === "change", out: sel.value === "out" });
  }
  const groups = d.groups.filter((g) => !g.out).map((g) => ({ conv: g.conv, owner: g.owner }));
  if (d.cal) return saveSharedCal(d);
  try {
    if (!d.conv) {
      const name = (document.getElementById("dtitle")?.value || "").trim();
      await tauri.core.invoke("book_new", { name, people: d.picked.map((u) => ({ username: u.username, owner: d.roles[u.id] === true })), groups });
      showToast(`Made ${name}.`);
    } else {
      const book = state.ab.books.find((b) => b.conv === d.conv);
      const remove = book.people.filter((p) => d.roles[p.user.id] === "out").map((p) => p.user.username);
      const roles = book.people.filter((p) => typeof d.roles[p.user.id] === "boolean" && d.roles[p.user.id] !== p.owner).map((p) => ({ username: p.user.username, owner: d.roles[p.user.id] }));
      await tauri.core.invoke("book_share", { conv: d.conv, add: d.picked.map((u) => ({ username: u.username, owner: d.roles[u.id] === true })), remove, roles, groups });
      showToast("Saved.");
    }
    state.dialog = null;
    setTimeout(loadAb, 600);
    render();
  } catch (e) {
    d.error = String(e);
    render();
  }
}

/** A new shared calendar, or sharing one (the address book dialog's choices). */
async function saveSharedCal(d) {
  const people = d.picked.map((u) => ({ username: u.username, owner: d.roles[u.id] === true }));
  try {
    if (!d.conv) {
      const name = (document.getElementById("dtitle")?.value || "").trim();
      await tauri.core.invoke("shared_cal_new", { name, people });
      showToast(`Made ${name}.`);
    } else {
      const sc = sharedCal(d.conv);
      const others = sc.people.filter((p) => p.user.id !== state.me.id);
      const remove = others.filter((p) => d.roles[p.user.id] === "out").map((p) => p.user.username);
      const roles = others.filter((p) => typeof d.roles[p.user.id] === "boolean" && d.roles[p.user.id] !== p.owner).map((p) => ({ username: p.user.username, owner: d.roles[p.user.id] }));
      await tauri.core.invoke("shared_cal_share", { conv: d.conv, add: people, remove, roles });
      showToast("Saved.");
    }
    state.dialog = null;
    setTimeout(refreshCalendar, 600);
    render();
  } catch (e) {
    d.error = String(e);
    render();
  }
}

/** Admins: add or change something to book. */
function thingDialog(d) {
  const t = d.thing;
  const byDayDefault = (k) => k === "book" || k === "vehicle";
  return `<div class="scrim"><form class="dialog wide" data-act="dialog" role="dialog" aria-label="${t.id ? "Change" : "Add something to book"}" style="width:600px">
    <h2>${t.id ? `Change ${esc(t.name)}` : "Add something to book"}</h2>
    <div class="grid2">
      <label class="field">Name<input id="th_name" value="${esc(t.name)}" placeholder="Room 2" autocomplete="off" required></label>
      <label class="field">What it is<select id="th_kind">${KINDS.map(([k, l]) => `<option value="${k}" ${t.kind === k ? "selected" : ""}>${l}</option>`).join("")}</select></label>
      <label class="field">Where<input id="th_place" value="${esc(t.place)}" placeholder="2nd floor, by the kitchen" autocomplete="off"></label>
      ${t.kind === "room" ? `<label class="field">Seats<input id="th_seats" type="number" min="0" value="${t.seats || ""}"></label>` : `<label class="field">Booked<select id="th_byday"><option value="day" ${t.by_day ? "selected" : ""}>By the day</option><option value="hour" ${t.by_day ? "" : "selected"}>By the hour</option></select></label>`}
    </div>
    <b>Bookings</b>
    <label class="choice tall${d.wantApprover ? "" : " on"}"><input type="radio" name="th_ok" value="self" ${d.wantApprover ? "" : "checked"}><span><b>Yes by itself when it's free</b><span class="hintline">Like most meeting rooms.</span></span></label>
    <label class="choice tall${d.wantApprover ? " on" : ""}"><input type="radio" name="th_ok" value="person" ${d.wantApprover ? "checked" : ""}><span><b>Someone says yes or no</b><span class="hintline">${t.approver ? `${esc(nameOf(t.approver))} looks after it.` : "Pick who looks after it, e.g. the librarian."}</span></span></label>
    ${d.pickApprover ? `<label class="field">Who looks after it<input id="pickq" value="${esc(d.q)}" placeholder="Search by name" autocomplete="off"></label>${d.q.trim() ? `<div class="pickresults">${state.people.filter((u) => !u.app).slice(0, 5).map((u) => `<button type="button" class="row compact" data-act="thingapprover" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${esc(nameOf(u))}</span><span class="last">${esc(handleOf(u))}</span></span></button>`).join("")}</div>` : ""}` : ""}
    <p class="hintline">Everyone on this server can book it, and sees who booked it and when (not what for). ${byDayDefault(t.kind) ? "" : ""}</p>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons">${t.id ? `<button class="small danger-outline" type="button" data-act="thingremove" data-id="${t.id}" style="margin-right:auto;height:36px">Take away</button>` : ""}<button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">${t.id ? "Save" : "Add"}</button></div>
  </form></div>`;
}

/** Reads the thing dialog's boxes into it. */
function readThing() {
  const d = state.dialog;
  const t = d.thing;
  const v = (id) => document.getElementById(id)?.value;
  if (v("th_name") != null) t.name = v("th_name");
  if (v("th_place") != null) t.place = v("th_place");
  const kind = v("th_kind");
  if (kind && kind !== t.kind) {
    t.kind = kind;
    t.by_day = kind === "book" || kind === "vehicle";
  }
  if (v("th_seats") != null) t.seats = Number(v("th_seats")) || 0;
  if (v("th_byday") != null && t.kind !== "room") t.by_day = v("th_byday") === "day";
  if (t.kind === "room") t.by_day = false;
  return t;
}

function saveThing() {
  const d = state.dialog;
  const t = readThing();
  if (!t.name.trim()) {
    d.error = "Give it a name.";
    return render();
  }
  if (d.wantApprover && !t.approver) {
    d.error = "Pick who looks after it.";
    return render();
  }
  tell({ t: "save_thing", thing: t });
}

// ---------- Out of office ----------
// Times away, kept with your account on the server so people are told while
// your computer is off: a notice in chats with you, one automatic reply
// each, and (from this computer's mail) one by email.

/** The time away going on now, if any. */
const awayNow = (a) => a?.times?.find(([s, e]) => s <= Date.now() && Date.now() < e);
/** "Mon 9 Nov": the day you're back from the current (or next) time away. */
function awayUntil(a) {
  const t = awayNow(a) || a?.times?.find(([, e]) => e > Date.now());
  return t ? dayLabel(t[1]) : "";
}

async function openAway() {
  let a = state.away.mine;
  if (tauri) {
    try {
      a = await tauri.core.invoke("away_get");
    } catch {}
  }
  const tomorrow = addDays(startOfDay(Date.now()), 1);
  const times = a?.times?.filter(([, e]) => e > Date.now()).map(([s, e]) => [s, e]) || [];
  state.dialog = {
    kind: "away", error: "",
    times: times.length ? times : [[tomorrow, addDays(tomorrow, 5)]],
    on: times.length > 0,
    text: a?.text || "",
    notice: a?.notice ?? true, block: a?.block ?? true, decline: a?.decline ?? true, email_to: a?.email_to || "contacts",
  };
  render();
}

function awayDialog(d) {
  const check = (id, on, label, hint) => `<label class="checkline"><input type="checkbox" id="${id}" ${on ? "checked" : ""}><span><b>${label}</b><span class="hintline">${hint}</span></span></label>`;
  const first = nameOf(state.me).split(" ")[0];
  return `<div class="scrim"><form class="dialog wide awaydlg" data-act="dialog" role="dialog" aria-label="Out of office">
    <div><h2>Out of office</h2><span class="hintline">${d.on ? "On. Change it, or turn it off." : "When you're away, and what people are told."}</span></div>
    <b>When you're away</b>
    ${d.times.map(([s, e], i) => `<div class="awayrow"><label class="sr" for="aw_from${i}">From</label><input type="date" id="aw_from${i}" value="${dateInput(s)}"><span>to</span><label class="sr" for="aw_to${i}">To</label><input type="date" id="aw_to${i}" value="${dateInput(e - 1)}"><button class="iconbtn" type="button" data-act="awaydel" data-id="${i}" aria-label="Remove this time away" ${d.times.length > 1 ? "" : "disabled"}>${icons.close}</button></div>`).join("")}
    <button class="linkbtn" type="button" data-act="awayadd" style="align-self:flex-start;padding:0">+ Another time away</button>
    <div class="awaychecks">
      ${check("aw_block", d.block, "Block my calendar", "Find a time shows you as away, so new meetings aren't put then.")}
      ${check("aw_decline", d.decline, "Say no to invitations for those days", "With your automatic reply as the reason.")}
      ${check("aw_notice", d.notice, "Show a notice in chats", `People see “${esc(first)} is away until …” before they write to you.`)}
    </div>
    <label class="field">Automatic reply<textarea id="aw_text" rows="3" placeholder="I'm on holiday until Monday 9 November. For anything urgent, ask Sam Lee.">${esc(d.text)}</textarea></label>
    <div class="emailto"><span>By email to</span>${[["everyone", "Everyone"], ["contacts", "People in my contacts"], ["nobody", "Nobody"]].map(([v, l]) => `<label class="choice${d.email_to === v ? " on" : ""}"><input type="radio" name="aw_email" value="${v}" ${d.email_to === v ? "checked" : ""}> ${l}</label>`).join("")}</div>
    <span class="hintline">Sent once to each person while you're away, in chats and by email; never to mailing lists or other automatic replies. Email replies go while Xenify Chat is open on one of your computers. The server can read your notice and reply, like your status.</span>
    ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
    <div class="buttons">${d.on ? `<button class="small danger-outline" type="button" data-act="awayoff" style="margin-right:auto;height:36px">Turn off</button>` : ""}<button class="small" type="button" data-act="closedialog">Cancel</button><button class="small primary" type="submit">${d.on ? "Save" : "Turn on"}</button></div>
  </form></div>`;
}

function readAway() {
  const d = state.dialog;
  if (d?.kind !== "away") return;
  d.times = d.times.map(([s, e], i) => {
    const from = document.getElementById(`aw_from${i}`)?.value;
    const to = document.getElementById(`aw_to${i}`)?.value;
    const start = from ? fromInputs(from, "00:00") : s;
    const end = to ? addDays(fromInputs(to, "00:00"), 1) : e;
    return [start, Math.max(end, addDays(start, 1))];
  });
  for (const k of ["block", "decline", "notice"]) {
    const el = document.getElementById(`aw_${k}`);
    if (el) d[k] = el.checked;
  }
  const t = document.getElementById("aw_text");
  if (t) d.text = t.value;
  const em = document.querySelector('input[name="aw_email"]:checked');
  if (em) d.email_to = em.value;
}

async function saveAway(off = false) {
  const d = state.dialog;
  trace(`away: saving (${off ? "off" : "on"})`);
  readAway();
  if (!off && !d.text.trim()) {
    d.error = "Write the automatic reply people get.";
    return render();
  }
  const away = { user: 0, times: off ? [] : d.times, text: d.text.trim(), notice: d.notice, block: d.block, decline: d.decline, email_to: d.email_to };
  try {
    await tauri.core.invoke("away_save", { away });
    state.away.mine = off ? null : { ...away, user: state.me.id };
    state.dialog = null;
    showToast(off ? "Out of office is off." : `Out of office is on${awayNow(away) ? "" : ` from ${dayLabel(away.times[0][0])}`}.`);
    render();
  } catch (e) {
    trace(`away: ${e}`);
    d.error = String(e);
    render();
  }
}

/** The person you're chatting with directly, if they're away now. */
function awayIn(c) {
  if (!c || c.kind !== "direct") return null;
  const o = others(c)[0];
  const a = o && state.away.of[o.id];
  const now = awayNow(a);
  // Messages from when it was turned on (not earlier the same day).
  return now ? { who: o, a, from: Math.max(now[0], a.since || 0) } : null;
}

/** Above the message box: they're away (until you've written to them). */
function awayNotice(c) {
  const w = awayIn(c);
  if (!w || !w.a.notice || autoReplyAfter(c, w)) return "";
  const first = nameOf(w.who).split(" ")[0];
  return `<div class="awaynotice" role="status">${icons.plane}<span><b>${esc(first)} is away until ${esc(awayUntil(w.a))}.</b> ${esc(firstLine(w.a.text))}</span></div>`;
}

/** Your first message since they went away, which their automatic reply follows. */
function autoReplyAfter(c, w) {
  return (state.messages[c.id] || []).find((m) => m.sender.id === state.me.id && m.at >= w.from && !m.deleted);
}

function firstLine(text) {
  const t = (text || "").trim().split(/\n/)[0];
  return t.length > 160 ? t.slice(0, 157) + "…" : t;
}

/** Their automatic reply, once, after your first message while they're away. */
function autoReply(c, m) {
  const w = awayIn(c);
  if (!w || autoReplyAfter(c, w)?.id !== m.id || !w.a.text.trim()) return "";
  return `<div class="autoreply"><span class="autohead">Automatic reply · ${esc(nameOf(w.who))}</span><div class="bubble auto">${esc(w.a.text)}</div></div>`;
}

// ---------- Meetings ----------
// A meeting is an event with people invited: on the server (they answer in
// Xenify) or by email (from your email account). Find a time shows when
// everyone's busy, from the busy times each person's app shares.

const isMeeting = (d) => d.picked?.length > 0 || d.guests?.length > 0 || d.things?.length > 0 || d.meeting > 0;
const looksLikeEmail = (s) => /^[^\s@,;<>]+@[^\s@,;<>]+\.[^\s@,;<>]+$/.test(s.trim());
const meetingOf = (conv) => state.meetings.find((m) => m.conv === conv);
const ANSWERS = { yes: "Yes", maybe: "Maybe", no: "No" };

/** Who's invited: chips, and a box to find people or type an email address. */
function whoField(d) {
  const chips = d.picked.map((u) => `<span class="chip">${esc(nameOf(u))}<button type="button" class="chipx" data-act="unpick" data-id="${u.id}" aria-label="Don't invite ${esc(nameOf(u))}">×</button></span>`)
    .concat((d.things || []).map((t) => `<span class="chip room">${thingIcon(thingOf(t.id) || { kind: "room" }).replace('width="18" height="18"', 'width="14" height="14"')}${esc(t.name)}<button type="button" class="chipx" data-act="unthing" data-id="${t.id}" aria-label="Don't book ${esc(t.name)}">×</button></span>`))
    .concat(d.guests.map((g, i) => `<span class="chip guest">${icons.mail.replace('width="18" height="18"', 'width="14" height="14"')}${esc(g.email)}<button type="button" class="chipx" data-act="unguest" data-id="${i}" aria-label="Don't invite ${esc(g.email)}">×</button></span>`)).join("");
  const already = new Set([state.me.id, ...d.picked.map((u) => u.id)]);
  const results = d.q.trim() && !d.q.includes("@") ? state.people.filter((u) => !already.has(u.id) && !u.app) : [];
  const email = looksLikeEmail(d.q);
  const q = d.q.trim().toLowerCase();
  const rooms = q && !q.includes("@") ? state.things.list.filter((t) => !t.by_day && t.name.toLowerCase().includes(q) && !(d.things || []).some((x) => x.id === t.id)) : [];
  const fromBooks = q ? (d.found || []).filter((x) => x.email && !d.guests.some((g) => g.email.toLowerCase() === x.email.toLowerCase())) : [];
  const any = results.length || rooms.length || fromBooks.length;
  return `<div class="field">Who
      <div class="whobox">${chips}<input id="pickq" value="${esc(d.q)}" placeholder="${d.picked.length || d.guests.length ? "Add someone" : state.things.list.some((t) => !t.by_day) ? "Name, email address or room" : "Name or email address"}" autocomplete="off" aria-label="Invite people, email addresses or rooms"></div>
      ${any ? `<div class="pickresults">${results.slice(0, 5).map((u) => `<button type="button" class="row compact" data-act="pick" data-id="${u.id}">${personAvatar(u, "small")}<span class="body"><span class="name">${esc(nameOf(u))}</span><span class="last">${esc(handleOf(u))}</span></span></button>`).join("")}
        ${rooms.slice(0, 4).map((t) => `<button type="button" class="row compact" data-act="pickthing" data-id="${t.id}"><span class="avatar small square thingav" aria-hidden="true">${thingIcon(t)}</span><span class="body"><span class="name">${esc(t.name)}</span><span class="last">${esc(thingLine(t))}</span></span></button>`).join("")}
        ${fromBooks.slice(0, 4).map((x, i) => `<button type="button" class="row compact" data-act="pickcontact" data-id="${i}">${icons.mail}<span class="body"><span class="name">${esc(x.name)}</span><span class="last">${esc(x.email)}</span></span></button>`).join("")}</div>` : ""}
      ${email ? `<div class="pickresults"><button type="button" class="row compact" data-act="addguest">${icons.mail}<span class="body"><span class="name">Invite ${esc(d.q.trim())} by email</span></span></button></div>` : ""}
      ${d.guests.length ? `<span class="hintline">People on this server get it in Xenify. Email addresses get a normal invitation from your email account, and their answers come back by email.</span>` : ""}
      ${d.things?.length ? `<span class="hintline">Rooms and things are booked with the meeting${d.things.some((t) => thingOf(t.id)?.approver) ? "; some ask whoever looks after them" : ", at once if they're free"}.</span>` : ""}
    </div>`;
}

/** The meeting editor: the form on the left, Find a time on the right. */
function meetingEditor(d) {
  const e = d.event;
  const heading = d.meeting ? "Change meeting" : "New meeting";
  return `<div class="scrim"><form class="dialog meeting" data-act="dialog" role="dialog" aria-label="${heading}">
    <div class="meetform">
      <h2>${heading}</h2>
      <label class="field">Name<input id="ev_title" value="${esc(e.title)}" autocomplete="off" required></label>
      ${whoField(d)}
      <div class="meettime">
        <label class="field">Day<input id="ev_date" type="date" value="${dateInput(e.start)}" required></label>
        <label class="field">From<input id="ev_from" type="time" value="${timeInput(e.start)}"></label>
        <label class="field">To<input id="ev_to" type="time" value="${timeInput(e.end)}"></label>
      </div>
      ${d.things.length ? `<div class="field">Where
        <div class="roomwhere">${d.things.map((t) => `${thingIcon(thingOf(t.id) || { kind: "room" })}<span>${esc([t.name, thingOf(t.id)?.place, thingOf(t.id)?.seats ? `${thingOf(t.id).seats} seats` : ""].filter(Boolean).join(" · "))}</span>`).join("")}</div>
        <label class="checkline"><input type="checkbox" id="ev_alsocall" ${d.also_call ? "checked" : ""}><span><b>Also a Xenify call</b><span class="hintline">For anyone not in the room.</span></span></label>
      </div>` : `<div class="field">Where
        <div class="wherechoice">
          <label class="choice${d.where === "call" ? " on" : ""}"><input type="radio" name="ev_where" value="call" ${d.where === "call" ? "checked" : ""}> Xenify call</label>
          <label class="choice${d.where === "place" ? " on" : ""}"><input type="radio" name="ev_where" value="place" ${d.where === "place" ? "checked" : ""}> A place</label>
        </div>
        ${d.where === "place" ? `<input id="ev_place" value="${esc(d.place)}" placeholder="An address, or a room not in Xenify" autocomplete="off" aria-label="Where">` : `<span class="hintline">A Join button appears for everyone invited on this server at the time.${d.guests.length ? " People invited by email are told the call is in Xenify Chat." : ""}</span>`}
      </div>`}
      <label class="field">Notes<textarea id="ev_notes" rows="2">${esc(e.notes)}</textarea></label>
      ${d.guests.length && state.live ? `<p class="hintline">${d.guests.length === 1 ? esc(d.guests[0].email) + " gets" : "People invited by email get"} the meeting's name, time and notes by email.</p>` : ""}
      ${d.error ? `<p class="formerror" role="alert">${esc(d.error)}</p>` : ""}
      <div class="buttons">
        ${d.meeting ? `<button class="small danger-outline" type="button" data-act="meetcancel" data-id="${d.meeting}" style="margin-right:auto;height:36px">Call it off</button>` : ""}
        <button class="small" type="button" data-act="closedialog">Cancel</button>
        <button class="small primary" type="submit">${d.meeting ? "Save and tell everyone" : "Send invitations"}</button>
      </div>
    </div>
    ${findATime(d)}
  </form></div>`;
}

/** Everyone's busy times on one day, and times everyone can make. */
function findATime(d) {
  const e = d.event;
  const day = d.day;
  const len = Math.max(e.end - e.start, 15 * MIN);
  // 08:00–18:00, wider if the meeting is outside it.
  const from = Math.min(day + 8 * HOUR, e.start >= day && e.start < day + DAY ? e.start : Infinity);
  const to = Math.max(day + 18 * HOUR, e.start >= day && e.start < day + DAY ? e.end : 0);
  const pct = (t) => (100 * (Math.min(Math.max(t, from), to) - from)) / (to - from);
  const span = (a, b, cls) => (b <= from || a >= to ? "" : `<div class="${cls}" style="left:${pct(a)}%;width:${pct(b) - pct(a)}%"></div>`);
  const picked = e.start < day + DAY && e.end > day ? span(e.start, e.end, "lanepick") : "";
  const lane = (who, b, label) => {
    let inside;
    const away = b?.known && (b.away || []).some(([a, z]) => a < day + 18 * HOUR && z > day + 8 * HOUR);
    if (!b) inside = `<div class="lanenote">Looking…</div>`;
    else if (!b.known) inside = `<div class="lanenote">Doesn't share busy times with you</div>`;
    else if (away) inside = `<div class="lanenote awaylane">Away</div>`;
    else inside = b.busy.map(([a, z]) => span(a, z, "busy")).join("");
    return `<div class="lanerow"><div class="lanewho">${who}<span>${esc(label)}</span></div><div class="lane${b && !b.known ? " unknown" : ""}" data-act="lanepick" data-from="${from}" data-to="${to}" title="Pick a time here">${inside}${picked}</div></div>`;
  };
  const me = state.me;
  const rows = [lane(personAvatar(me, "tiny"), state.busy[me.id], "You")]
    .concat(d.picked.map((u) => lane(personAvatar(u, "tiny"), state.busy[u.id], nameOf(u))))
    .concat(d.things.map((t) => lane(`<span class="avatar tiny square thingav" aria-hidden="true">${thingIcon(thingOf(t.id) || { kind: "room" }).replace('width="18" height="18"', 'width="13" height="13"')}</span>`, roomBusy(t.id, d), t.name)))
    .concat(d.guests.map((g) => `<div class="lanerow"><div class="lanewho"><span class="avatar tiny" style="background:var(--tint);color:var(--muted)" aria-hidden="true">${icons.mail.replace('width="18" height="18"', 'width="12" height="12"')}</span><span>${esc(g.email)}</span></div><div class="lane unknown"><div class="lanenote">Outside the server: not known</div></div></div>`))
    .join("");
  const ticks = [];
  for (let t = Math.ceil(from / (2 * HOUR)) * 2 * HOUR; t <= to; t += 2 * HOUR) ticks.push(`<span style="left:${pct(t)}%">${hm(t)}</span>`);
  const slots = freeTimes(d, len);
  const hol = holidayOn(day);
  const lengths = [[30, "30 minutes"], [45, "45 minutes"], [60, "1 hour"], [90, "1½ hours"], [120, "2 hours"]];
  const lenMin = Math.round(len / MIN);
  return `<section class="findtime" aria-label="Find a time">
    <div class="findhead"><b>Find a time</b>
      <div class="daynav"><button type="button" class="iconbtn" data-act="meetday" data-id="-1" aria-label="Day before">${icons.back}</button><span>${esc(new Date(day).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }))}</span><button type="button" class="iconbtn" data-act="meetday" data-id="1" aria-label="Next day" style="transform:scaleX(-1)">${icons.back}</button></div>
    </div>
    ${hol ? `<p class="laneholiday" role="status">${esc(hol.name)}: ${hol.company ? "the company is closed" : "a public holiday"}. Times are suggested on other days.</p>` : ""}
    <div class="lanes"><div class="laneticks">${ticks.join("")}</div>${rows}</div>
    <p class="hintline">Grey is busy. Only the times are shared, never what anyone's doing.</p>
    <b class="slotshead">${d.picked.some((u) => state.busy[u.id] && !state.busy[u.id].known) ? "Times everyone who shares busy times can make" : "Times everyone on the server can make"}${d.things.length ? `, with ${esc(d.things.map((t) => t.name).join(" and "))} free` : ""}</b>
    <div class="slots">${slots.length ? slots.map(([a, z]) => `<button type="button" class="slot${a === e.start ? " on" : ""}" data-act="meetslot" data-id="${a}"><b>${hm(a)}–${hm(z)}</b><span>${esc(new Date(a).toLocaleDateString([], { weekday: "short", day: "numeric" }))}${a === e.start ? " · picked" : ""}</span></button>`).join("") : `<span class="hintline">${d.picked.some((u) => !state.busy[u.id]) ? "Looking at everyone's busy times…" : `No time free for everyone in the next week${[...Array(7)].some((_, i) => holidayOn(addDays(day, i))) ? " (holidays and days the company is closed are left out)" : ""}.`}</span>`}</div>
    <label class="lengthrow">Length<select id="ev_length">${lengths.map(([m, l]) => `<option value="${m}" ${m === lenMin ? "selected" : ""}>${l}</option>`).concat(lengths.some(([m]) => m === lenMin) ? [] : [`<option value="${lenMin}" selected>${lenMin} minutes</option>`]).join("")}</select><span class="hintline">Inside everyone's working hours</span></label>
  </section>`;
}

/** The first few times from the shown day on that everyone known is free,
 * inside their working hours, `len` long, on the half hour. */
function freeTimes(d, len) {
  // Only once everyone's busy times are in (or known not to be shared).
  const all = [state.me, ...d.picked].map((u) => state.busy[u.id]);
  if (all.some((b) => !b)) return [];
  const known = all.filter((b) => b.known).concat((d.things || []).map((t) => roomBusy(t.id, d)));
  if (!known.length) return [];
  const out = [];
  const now = Date.now();
  for (let day = d.day; day < d.day + 7 * DAY && out.length < 4; day = addDays(day, 1)) {
    if (holidayOn(day)) continue;
    let today = 0;
    for (let t = day; t + len <= addDays(day, 1) && out.length < 4 && today < 2; t += 30 * MIN) {
      if (t < now) continue;
      const end = t + len;
      const ok = known.every((b) => b.work.some(([a, z]) => a <= t && end <= z) && !b.busy.some(([a, z]) => a < end && z > t));
      if (ok) {
        out.push([t, end]);
        today++;
        t += len - 30 * MIN;
      }
    }
  }
  return out;
}

/** A room's bookings as busy times (not this meeting's own booking). */
function roomBusy(id, d) {
  return { known: true, busy: bookingsOf(id).filter((b) => !(d.meeting && b.meeting === d.meeting)).map((b) => [b.start, b.end]), work: [[0, 8e15]] };
}

/** Find a time needs the busy times of everyone invited, and rooms' bookings. */
function askBusy(d) {
  if (!tauri || !state.live) return;
  tauri.core.invoke("meeting_busy", { users: [state.me.id, ...d.picked.map((u) => u.id).filter((id) => id > 0)] }).catch(() => {});
  if (d.things?.length) tell({ t: "bookings", things: d.things.map((t) => t.id), from: d.day - DAY, to: d.day + 9 * DAY });
}

function addGuest(d) {
  const email = d.q.trim().replace(/[,;]$/, "");
  if (!looksLikeEmail(email)) return false;
  if (!d.guests.some((g) => g.email.toLowerCase() === email.toLowerCase())) d.guests.push({ email, name: "", answer: "" });
  d.q = "";
  return true;
}

async function saveMeeting(d, e) {
  const details = {
    title: e.title.trim(), start: e.start, end: e.end, place: d.where === "place" ? d.place.trim() : "", notes: e.notes,
    guests: d.guests, cancelled: false, uid: "", sequence: 0, mail_from: "",
    things: d.things, also_call: d.things.length ? !!d.also_call : false,
  };
  if (d.things.length) details.place = "";
  if (!d.things.length && d.where === "place" && !details.place) {
    d.error = "Say where it is, or choose a Xenify call.";
    return render();
  }
  try {
    if (d.meeting) {
      const was = meetingOf(d.meeting);
      const before = new Set((was?.people || []).map((p) => p.user.username));
      const now = new Set(d.picked.map((u) => u.username));
      await tauri.core.invoke("meeting_change", {
        conv: d.meeting, details,
        add: [...now].filter((u) => !before.has(u)),
        remove: [...before].filter((u) => !now.has(u) && u !== state.me.username),
        bookings: meetingBookings(d.meeting),
      });
      showToast("Saved. Everyone invited is told.");
    } else {
      await tauri.core.invoke("meeting_new", { details, people: d.picked.map((u) => u.username) });
      showToast(d.guests.length ? "Invitations sent, here and by email." : "Invitations sent.");
    }
    state.dialog = null;
    render();
  } catch (err) {
    d.error = String(err);
    render();
  }
}

/** Your bookings for a meeting (to make again when it moves, or call off). */
const meetingBookings = (conv) => Object.values(state.things.byId).filter((b) => b.meeting === conv && live(b)).map((b) => b.id);

/** A meeting: when, where, who's coming; your answer; Join at the time. */
function meetingDialog(d) {
  const m = meetingOf(d.conv);
  if (!m) return "";
  const when = `${new Date(m.start).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })} · ${hm(m.start)}–${hm(m.end)}`;
  const soon = Date.now() >= m.start - 5 * MIN && Date.now() <= m.end;
  const c = conv(m.conv);
  const going = c && state.calls[c.id];
  const answerPill = (a, email) => `<span class="answer ${a || "none"}">${a ? ANSWERS[a] + (email ? ", by email" : "") : "Not yet"}</span>`;
  // Someone who said no because they're away then: their automatic reply.
  const awayWhy = (u) => {
    const a = state.away.of[u.id];
    return a?.text && a.times.some(([s, e]) => s < m.end && e > m.start) ? `<span class="hintline awaywhy">${icons.plane.replace('width="18" height="18"', 'width="12" height="12"')}${esc(firstLine(a.text))}</span>` : "";
  };
  const people = m.people.map((p) => `<div class="whorow">${personAvatar(p.user, "small")}<span>${esc(nameOf(p.user))}${p.user.id === state.me.id ? " (you)" : ""}${m.organiser?.id === p.user.id ? " · organiser" : ""}${p.answer === "no" ? awayWhy(p.user) : ""}</span>${answerPill(p.answer)}</div>`)
    .concat(m.guests.map((g) => `<div class="whorow"><span class="avatar small" style="background:var(--tint);color:var(--muted)" aria-hidden="true">${icons.mail.replace('width="18" height="18"', 'width="14" height="14"')}</span><span>${esc(g.name || g.email)}</span>${answerPill(g.answer, true)}</div>`)).join("");
  return `<div class="scrim"><div class="dialog wide meetview" role="dialog" aria-label="${esc(m.title)}">
    <div><h2>${esc(m.title)}</h2><div class="hintline">${esc(when)}${m.organiser && !m.yours ? ` · from ${esc(nameOf(m.organiser))}` : ""}</div></div>
    ${m.cancelled ? `<p class="formerror">Called off.</p>` : ""}
    ${m.things?.length ? `<div class="wherebox">${thingIcon(thingOf(m.things[0].id) || { kind: "room" })}<span>${esc(m.things.map((t) => [t.name, thingOf(t.id)?.place].filter(Boolean).join(", ")).join(" · "))}</span></div>` : ""}
    ${m.place ? `<div class="wherebox"><span>${esc(m.place)}</span></div>` : ""}
    ${(!m.place && !m.things?.length) || m.also_call ? `<div class="wherebox">${icons.video}<span>Xenify call${soon ? "" : " · Join appears 5 minutes before"}</span>${soon && !m.cancelled ? `<button class="small primary" data-act="meetjoin" data-id="${m.conv}">${going ? "Join" : "Start the call"}</button>` : ""}</div>` : ""}
    ${m.notes ? `<p>${esc(m.notes)}</p>` : ""}
    <b class="whohead">Who's coming</b>
    <div class="wholist">${people}</div>
    ${meetingRecordings(m.conv)}
    ${m.cancelled || !(m.conv in state.meetRemind) ? "" : `<label class="field remindfield">Remind me<select id="onemeetremind" data-conv="${m.conv}">${MEET_REMIND.map(([v, l]) => `<option value="${v ?? ""}" ${state.meetRemind[m.conv] === v ? "selected" : ""}>${l}</option>`).join("")}</select></label>`}
    ${m.yours || m.cancelled ? "" : `<div class="goingrow"><b>Going?</b>${["yes", "maybe", "no"].map((a) => `<button class="small${m.mine === a ? " picked" : ""}" data-act="meetanswer" data-id="${m.conv}" data-answer="${a}" aria-pressed="${m.mine === a}">${ANSWERS[a]}</button>`).join("")}</div>`}
    <div class="buttons">
      ${m.yours && !m.cancelled ? `<button class="small" data-act="meetedit" data-id="${m.conv}">Change</button>` : ""}
      <button class="small" data-act="closedialog">Close</button>
    </div>
  </div></div>`;
}

const MEET_REMIND = [[null, "No reminder"], [0, "When it starts"], [1, "1 minute before"], [5, "5 minutes before"], [10, "10 minutes before"], [15, "15 minutes before"]];

function openMeeting(conv) {
  state.dialog = { kind: "meeting", conv };
  tauri.core.invoke("meeting_remind", { conv }).then((m) => { state.meetRemind[conv] = m; render(); }).catch(() => {});
  // Its recordings, if any.
  if (state.live && !state.messages[conv]) tell({ t: "history", conv, before: null, limit: 100 });
  // Whether anyone's away then (for why they said no).
  const m = meetingOf(conv);
  if (m && state.live) tell({ t: "away_of", users: m.people.map((p) => p.user.id).filter((id) => id !== state.me.id) });
  render();
}

/** Invitations waiting for your answer, in the sidebar. */
function invitesCard() {
  if (!tauri) return "";
  const list = state.meetings.filter((m) => !m.yours && !m.mine && !m.cancelled && m.end > Date.now());
  if (!list.length) return "";
  return `<section class="today invites" aria-label="Invitations">
    <div class="todayhead"><b>Invitations · ${list.length}</b></div>
    ${list.slice(0, 3).map((m) => `<div class="invitecard">
      <button class="linkish" data-act="openmeeting" data-id="${m.conv}"><b>${esc(m.title)}</b><span class="hintline">${esc(new Date(m.start).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }))} · ${hm(m.start)}–${hm(m.end)} · ${m.place ? esc(m.place) : "Xenify call"}${m.organiser ? ` · from ${esc(nameOf(m.organiser))}` : ""}</span></button>
      <div class="goingrow">${["yes", "maybe", "no"].map((a) => `<button class="small" data-act="meetanswer" data-id="${m.conv}" data-answer="${a}">${ANSWERS[a]}</button>`).join("")}</div>
    </div>`).join("")}
  </section>`;
}

function notifyInvite(m) {
  state.meetings = [...state.meetings.filter((x) => x.conv !== m.conv), m].sort((a, b) => a.start - b.start);
  render();
  const st = state.settings;
  if (!tauri || !st.desktop || state.status === "dnd" || quietNow()) return;
  const when = `${new Date(m.start).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}, ${hm(m.start)}`;
  tauri.core.invoke("notify", { title: `Invitation: ${m.title}`, body: `${when}${m.organiser ? ` · from ${nameOf(m.organiser)}` : ""}`, conv: -2, sound: !!st.sound }).catch(() => {});
}

async function answerMeeting(conv, answer) {
  try {
    await tauri.core.invoke("meeting_answer", { conv, answer });
  } catch (e) {
    showToast(String(e));
  }
}

/** Settings › Calendar. */
function calendarSettings() {
  const s = state.calSettings;
  if (!s) return `<h1>Calendar</h1><p class="hintline">Loading…</p>`;
  const days = [[1, "Mon"], [2, "Tue"], [3, "Wed"], [4, "Thu"], [5, "Fri"], [6, "Sat"], [7, "Sun"]];
  const share = [["everyone", "Everyone on this server", "Anyone here can find a time with you."], ["chats", "People I chat with", "Only people in a chat or group with you."], ["nobody", "Nobody", "You show as “not known” when others look for a time."]];
  return `<h1>Calendar</h1><p class="lede">Your calendar is kept on the server, and shared by all your computers.</p>
    <h2>Who can see when you're busy</h2>
    <p class="hintline" style="margin:0 0 10px">For Find a time. They see that you're busy, never what it is.</p>
    <div class="sharechoice">${share.map(([id, l, h]) => `<label class="choice tall${s.share === id ? " on" : ""}"><input type="radio" name="cal_share" value="${id}" ${s.share === id ? "checked" : ""}><span><b>${l}</b><span class="hintline">${h}</span></span></label>`).join("")}</div>
    <h2>Working hours</h2>
    <div class="workrow"><label class="sr" for="cal_from">From</label><input id="cal_from" type="time" value="${esc(s.work_from)}"><span>to</span><label class="sr" for="cal_to">To</label><input id="cal_to" type="time" value="${esc(s.work_to)}"></div>
    <div class="daypicks">${days.map(([n, l]) => `<label class="daypick"><input type="checkbox" data-day="${n}" ${s.work_days.includes(n) ? "checked" : ""}> ${l}</label>`).join("")}</div>
    <p class="hintline">Find a time only suggests times inside everyone's working hours.</p>
    <h2>Invitations to people outside the server</h2>
    ${state.mail.accounts.length
      ? `<div class="setting"><div><label class="label" for="cal_mail">Send from</label><div class="hintline">They get an ordinary invitation, and answer by email.</div></div><select id="cal_mail">${state.mail.accounts.map((a) => `<option ${s.mail_from === a.email ? "selected" : ""}>${esc(a.email)}</option>`).join("")}</select></div>`
      : `<div class="setting"><div><div class="label">No email account</div><div class="hintline">Add one under Email to invite people outside this server.</div></div><button class="small" data-act="section" data-id="email">Add one</button></div>`}`;
}

async function saveCalSettings() {
  const s = state.calSettings;
  const v = (id) => document.getElementById(id)?.value;
  s.share = document.querySelector('input[name="cal_share"]:checked')?.value || s.share;
  s.work_from = v("cal_from") || s.work_from;
  s.work_to = v("cal_to") || s.work_to;
  s.work_days = [...document.querySelectorAll(".daypick input:checked")].map((x) => Number(x.dataset.day));
  if (v("cal_mail")) s.mail_from = v("cal_mail");
  try {
    await tauri.core.invoke("calendar_settings_save", { settings: s });
    state.settings.calendar = { ...s };
  } catch (e) {
    showToast(String(e));
  }
  render();
}

/** Your bookings (not meetings' rooms) as calendar entries. */
function bookingEvents(from, to) {
  return Object.values(state.things.byId)
    .filter((b) => b.user.id === state.me?.id && live(b) && !b.meeting && b.start < to && b.end > from && thingOf(b.thing))
    .map((b) => {
      const t = thingOf(b.thing);
      return { id: -b.id, title: `${t.name}${b.status === "asked" ? " (asked)" : ""}`, start: b.start, end: b.end, all_day: t.by_day, notes: "", kind: "booking", link: JSON.stringify({ type: "thing", thing: t.id }), remind: null, reminded: false, done: false, repeat: "", meeting: 0, answer: b.status === "asked" ? "" : "yes" };
    });
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
      <span class="body"><span class="name">${esc(e.title)}</span><span class="last">${e.hides ? "Back" : "Reminds you"} ${esc(dayWord(e.start))} at ${esc(hm(e.start))}</span></span>
    </button>
    ${e.hides ? `<button class="small" data-act="bringback" data-id="${e.id}">Bring back</button>` : ""}
  </div>`;
}

// Reminders that are due: shown once, as a notification and on screen.

async function checkDue() {
  // Signed in, the server shows reminders (see onReminder).
  if (!tauri || state.live) return;
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

// Reminders from the server: shown on every device you're signed in on,
// with snoozing and dismissing there for all of them.

function onReminder(r) {
  const fresh = !state.due.some((x) => x.key === r.key);
  state.due = [...state.due.filter((x) => x.key !== r.key), r];
  if (!fresh || r.at < Date.now() + (state.skew || 0) - HOUR) return;
  const st = state.settings;
  if (st.desktop && state.status !== "dnd" && !quietNow()) {
    const link = r.link ? JSON.parse(r.link) : null;
    const title = r.starts ? `${r.title} · ${r.starts > Date.now() ? `at ${hm(r.starts)}` : "now"}` : "Reminder";
    tauri.core.invoke("notify", { title, body: r.starts ? r.body || "From your calendar" : r.title, conv: link?.type === "chat" ? link.conv : -2, sound: !!st.sound }).catch(() => {});
  }
  attend(true);
}

/** When a reminder about something starting at `starts` is, in words. */
function dueWhen(r) {
  if (!r.starts) {
    // `at` is in the server's time (`starts` is this computer's).
    const at = r.at - (state.skew || 0);
    const day = dayWord(at);
    return `${day.charAt(0).toUpperCase() + day.slice(1)} at ${hm(at)}`;
  }
  const mins = Math.round((r.starts - Date.now()) / MIN);
  if (mins > 0 && mins < 60) return `At ${hm(r.starts)} · in ${mins} minute${mins === 1 ? "" : "s"}`;
  return mins <= 0 && mins > -60 ? `At ${hm(r.starts)} · now` : `${dayWord(r.starts)} at ${hm(r.starts)}`;
}

/** Snoozing choices for reminder `r`: [label, until]. */
function snoozes(r) {
  const now = Date.now();
  const tomorrow = new Date(addDays(startOfDay(now), 1));
  tomorrow.setHours(9);
  const out = [["1 min", now + MIN], ["5 min", now + 5 * MIN], ["10 min", now + 10 * MIN]];
  if (r.starts && r.starts - MIN > now + 30_000) out.push(["1 min before it starts", r.starts - MIN]);
  out.push(["Tomorrow", tomorrow.getTime()]);
  return out;
}

function dueCards() {
  if (!state.due.length) return "";
  return `<div class="duecards" role="region" aria-label="Reminders">${state.due.slice(-3).map((r) => `<div class="duecard" role="alertdialog" aria-label="Reminder: ${esc(r.title)}">
    <div class="duehead">${icons.bell.replace('width="20" height="20"', 'width="16" height="16"')}<div><b>${esc(r.title)}</b><span class="hintline">${esc(dueWhen(r))}</span></div></div>
    ${r.starts && r.body ? `<p class="duebody">${esc(r.body)}</p>` : ""}
    <div class="duesnooze"><span class="hintline">Snooze</span>${snoozes(r).map(([l, t]) => `<button class="small" data-act="snooze" data-id="${esc(r.key)}" data-until="${t}">${esc(l)}</button>`).join("")}</div>
    <div class="buttons">${r.link ? `<button class="small" data-act="dueopen" data-id="${esc(r.key)}">Open</button>` : ""}<button class="small primary" data-act="duedismiss" data-id="${esc(r.key)}">Dismiss</button></div>
  </div>`).join("")}</div>`;
}

function snooze(key, until) {
  state.due = state.due.filter((r) => r.key !== key);
  tell({ t: "snooze", key, until: until + (state.skew || 0) });
  showToast(`I'll remind you again ${dayWord(until)} at ${hm(until)}.`);
}

function dismissDue(key) {
  state.due = state.due.filter((r) => r.key !== key);
  tell({ t: "dismiss", key });
}

function openDue(key) {
  const r = state.due.find((x) => x.key === key);
  if (!r) return;
  dismissDue(key);
  const link = r.link ? JSON.parse(r.link) : null;
  if (link?.type === "meeting") return openMeeting(link.conv);
  // `at` is in the server's time.
  const at = r.at - (state.skew || 0);
  openLinked({ link: r.link, title: r.title, start: at, end: at, kind: "reminder" });
}

// ---------- Getting your attention ----------

// Something new while you're elsewhere: the browser tab's title takes turns
// with "(3) New message" and its icon gets the count; the desktop app's
// taskbar entry flashes and its tray icon gets a dot. Seen, it stops. With
// reduce motion on, nothing flashes: the count stays in the title.

const reduceMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const seen = () => document.visibilityState === "visible" && document.hasFocus();
const attention = { on: false, count: 0, timer: 0, flip: false };

/** Chats with something unread that isn't muted. */
const unreadCount = () => state.conversations.filter((c) => unread(c) && !muted(c)).length;

/** Called after each change: starts when something new came in unseen. */
function attend(force = false) {
  if (!tauri || !state.live) return;
  const n = unreadCount();
  if (seen()) {
    attention.count = n;
    if (attention.on) stopAttention();
    return;
  }
  if (n > attention.count || force) startAttention(n);
  else if (attention.on && n !== attention.count) startAttention(n);
  attention.count = n;
}

function startAttention(n) {
  attention.on = true;
  if (window.XENIFY_WEB) {
    badgeIcon(n || "!");
    clearInterval(attention.timer);
    const words = n ? `(${n}) ${n === 1 ? "New message" : "New messages"}` : "Reminder";
    if (reduceMotion()) document.title = `${n ? `(${n}) ` : ""}Xenify Chat`;
    else {
      attention.timer = setInterval(() => {
        attention.flip = !attention.flip;
        document.title = attention.flip ? words : "Xenify Chat";
      }, 1000);
    }
  } else tauri.core.invoke("attention", { on: true, flash: !reduceMotion() }).catch(() => {});
}

function stopAttention() {
  attention.on = false;
  clearInterval(attention.timer);
  if (window.XENIFY_WEB) {
    document.title = "Xenify Chat";
    badgeIcon(0);
  } else tauri.core.invoke("attention", { on: false, flash: false }).catch(() => {});
}

let plainIcon = null;
/** The browser tab's icon, with `n` in a red circle (0: plain). */
function badgeIcon(n) {
  const link = document.querySelector('link[rel="icon"]');
  if (!link) return;
  plainIcon ??= link.href;
  if (!n) {
    link.href = plainIcon;
    return;
  }
  const img = new Image();
  img.onload = () => {
    const c = document.createElement("canvas");
    c.width = c.height = 64;
    const g = c.getContext("2d");
    g.drawImage(img, 0, 0, 64, 64);
    g.fillStyle = "#C43020";
    g.beginPath();
    g.arc(44, 20, 19, 0, 2 * Math.PI);
    g.fill();
    g.fillStyle = "#FFFFFF";
    g.font = "bold 26px sans-serif";
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.fillText(String(n).length > 2 ? "99" : String(n), 44, 21);
    if (attention.on) link.href = c.toDataURL("image/png");
  };
  img.src = plainIcon;
}

window.addEventListener("focus", () => attend());
document.addEventListener("visibilitychange", () => attend());

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
  // At the bottom (or nothing to scroll yet): stay at the bottom as messages come.
  const atEnd = box && box.scrollTop + box.clientHeight >= box.scrollHeight - 24;
  const tbox = document.getElementById("threadmsgs");
  const tKeep = tbox && tbox.scrollTop + tbox.clientHeight < tbox.scrollHeight - 24 ? tbox.scrollTop : null;
  const keepScroll = box && state.view === "chat" && !atEnd ? box.scrollTop : null;
  const calTop = document.getElementById("calscroll")?.scrollTop;
  menuRuns = [];
  applyLook();
  saveSettings();
  attend();
  const page = state.view === "loading" ? "" : state.view === "signin" ? signinPage() : state.view === "settings" ? settingsPage() : state.call && !state.call.away ? callSheet() : state.noAnswer ? noAnswerSheet() : state.board && state.view === "chat" ? boardSheet() : sidebar() + (state.view === "calendar" ? calendarSheet() : state.view === "people" ? peopleSheet() : state.compose ? composeSheet() : state.mailSel ? mailSheet() : chatSheet());
  // Players keep playing across a redraw: the same element is put back.
  const players = new Map([...document.querySelectorAll("video[data-keep], audio[data-keep]")].map((v) => [v.dataset.keep, v]));
  const app = document.getElementById("app");
  app.classList.toggle("phonesheet", !!state.phoneSheet);
  app.innerHTML = page + ctxMenu() + confirmBox() + dialogBox() + pictureView() + dropHint() + ringCard() + callStrip(false) + recSaveBar() + dueCards() + (state.toast ? `<div class="toast" role="alert">${esc(state.toast)}</div>` : "");
  for (const v of document.querySelectorAll("video[data-keep], audio[data-keep]")) {
    const old = players.get(v.dataset.keep);
    if (old && (old.getAttribute("src") || !v.getAttribute("src"))) v.replaceWith(old);
  }
  loadClips();
  placeBoard();
  placeCallTiles();
  syncTones();
  const nbox = document.getElementById("messages");
  if (nbox) nbox.scrollTop = keepScroll ?? nbox.scrollHeight;
  const ntbox = document.getElementById("threadmsgs");
  if (ntbox) ntbox.scrollTop = tKeep ?? ntbox.scrollHeight;
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
  sizeBox(document.getElementById("tdraft"));
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
  if (state.thread?.conv !== id) state.thread = null;
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
    // Whether the person you're writing to is away.
    if (state.live && c.kind === "direct" && others(c)[0]) tell({ t: "away_of", users: [others(c)[0].id] });
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
  state.drafts[c.id] = "";
  post(c, text);
  document.getElementById("draft")?.focus();
}

/** Sends `text` to chat `c` (shown at once, marked as on its way). */
function post(c, text) {
  const seq = c.last_seq + 1;
  const client_id = crypto.randomUUID();
  const msg = { id: -Date.now(), conv: c.id, seq, sender: state.me, text, at: Date.now(), client_id, edited: false, deleted: false, pending: state.live };
  (state.messages[c.id] ||= []).push(msg);
  if (state.live) tell({ t: "send", conv: c.id, client_id, text });
  else Object.assign(c, { last: msg, last_seq: seq, read_seq: seq });
  state.readLine[c.id] = null;
  state.conversations.sort((a, b) => (b.last?.at || 0) - (a.last?.at || 0));
  render();
  const box = document.getElementById("messages");
  if (box) box.scrollTop = box.scrollHeight;
}

// ---------- Files, pictures and clips ----------

/** A whiteboard message's details ({node, title}), or null. */
function boardOf(text) {
  if (!text?.startsWith("~board:")) return null;
  try {
    return JSON.parse(text.slice(7));
  } catch {
    return null;
  }
}

function boardCard(m, b) {
  return `<div class="boardcard">
    <span class="boardicon">${icons.board}</span>
    <span class="filebody"><b>${esc(b.title)}</b><span class="hintline">Whiteboard · everyone in this chat can draw on it</span></span>
    <button class="small primary" data-act="openboard" data-id="${m.id}">Open board</button>
  </div>`;
}

// ---------- Calls ----------

/** The server says a call started, changed or ended. */
function onCall(info) {
  const mine = info.people.some((u) => u.id === state.me?.id);
  if (info.people.length) state.calls[info.conv] = info;
  else delete state.calls[info.conv];
  const active = state.call;
  if (active && active.info.id === info.id) {
    // Who's still there.
    active.info = info;
    active.forget(info.people.map((u) => u.id));
    recordingChanged(active, info);
    if (!mine) {
      const c = conv(info.conv);
      const other = c?.kind === "direct" ? c.members.find((u) => u.id !== state.me.id) : null;
      const answered = active.answered;
      endCall();
      if (other && !answered) state.noAnswer = { conv: info.conv, name: nameOf(other), declined: !!info.declined };
      else if (other) showToast("The call ended.");
    } else if (info.people.length > 1) active.answered = true;
  } else if (mine && state.wantCall === info.conv && !active) {
    // Our start or join went through: connect.
    state.wantCall = null;
    begin(info);
  }
  // Ringing: someone else's call, not answered or declined here, and new.
  const fresh = Date.now() + (state.skew || 0) - info.at < 60_000;
  if (!mine && info.people.length && info.by.id !== state.me?.id && fresh && !state.declined.has(info.id) && state.call?.info.id !== info.id) {
    if (state.ringing?.id !== info.id) {
      const c = conv(info.conv);
      if (c && tauri) tauri.core.invoke("notify", { title: `${nameOf(info.by)} is calling`, body: c.kind === "direct" ? (info.video ? "Video call" : "Voice call") : `In ${titleOf(c)}`, conv: c.id, sound: true }).catch(() => {});
    }
    state.ringing = info;
  } else if (state.ringing?.id === info.id && (mine || !info.people.length)) {
    // It stopped ringing without you answering here.
    if (!mine && !state.declined.has(info.id)) showToast(`Missed call from ${nameOf(info.by)}.`);
    state.ringing = null;
  }
}

function startCall(c, video) {
  if (state.call) return showToast("You're in a call already.");
  const going = state.calls[c.id];
  state.wantCall = c.id;
  tell({ t: "call", conv: c.id, action: going ? "join" : "start", video });
  state.callVideo = video;
}

function joinCall(convId) {
  if (state.call) return showToast("You're in a call already.");
  state.ringing = null;
  state.wantCall = convId;
  state.callVideo = !!state.calls[convId]?.video;
  tell({ t: "call", conv: convId, action: "join" });
}

async function begin(info) {
  const call = new Call({ ...info, video: state.callVideo ?? info.video }, state.me.id, {
    testAudio: voiceTest,
    changed: () => render(),
    error: (text) => showToast(text),
    trace: (text) => trace(text),
  });
  state.call = call;
  // Calling one person: give up after 30 seconds without an answer.
  if (conv(info.conv)?.kind === "direct" && info.people.length === 1) {
    clearTimeout(state.callTimer);
    state.callTimer = setTimeout(() => {
      if (state.call !== call || call.info.people.length > 1) return;
      const other = conv(info.conv)?.members.find((u) => u.id !== state.me.id);
      leaveCall();
      state.noAnswer = { conv: info.conv, name: other ? nameOf(other) : "They", declined: false };
      render();
    }, 30_000);
  } else call.answered = true;
  render();
  try {
    const who = await tauri.core.invoke("board_session");
    await call.start(who.url, who.token);
    // Joining while someone records: you start with everything off too.
    recordingChanged(call, call.info);
  } catch (e) {
    showToast(`The call didn't start: ${e.message || e}`);
    leaveCall();
  }
}

function leaveCall() {
  const call = state.call;
  if (call) tell({ t: "call", conv: call.info.conv, action: "leave" });
  endCall();
}

function endCall() {
  clearTimeout(state.callTimer);
  if (state.recording) finishRecording();
  state.recAsk = null;
  state.call?.stop();
  state.call = null;
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
}

/** Ringing, made with the web view's own sound (no sound files). */
const tones = {
  ctx: null,
  kind: null,
  timer: null,
  /** "in" (someone's calling you), "out" (you're calling) or null. */
  set(kind) {
    if (kind === this.kind) return;
    clearInterval(this.timer);
    this.kind = kind;
    if (!kind) return;
    try {
      this.ctx ||= new AudioContext();
      this.ctx.resume().catch(() => {});
    } catch {
      return;
    }
    const beep = (freqs, start, length) => {
      const t = this.ctx.currentTime + start;
      const gain = this.ctx.createGain();
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(0.08, t + 0.02);
      gain.gain.setValueAtTime(0.08, t + length - 0.05);
      gain.gain.linearRampToValueAtTime(0, t + length);
      gain.connect(this.ctx.destination);
      for (const f of freqs) {
        const o = this.ctx.createOscillator();
        o.frequency.value = f;
        o.connect(gain);
        o.start(t);
        o.stop(t + length);
      }
    };
    // Incoming: a rising pair of chimes every 2 s. Outgoing: the usual
    // ringback (two tones together, on 1.5 s, off 3 s).
    const once = kind === "in" ? () => { beep([660], 0, 0.18); beep([880], 0.22, 0.28); } : () => beep([440, 480], 0, 1.5);
    once();
    this.timer = setInterval(once, kind === "in" ? 2000 : 4500);
  },
};

/** Which ring should be sounding now. */
function syncTones() {
  const call = state.call;
  const calling = call && !call.answered && conv(call.info.conv)?.kind === "direct";
  const r = state.ringing;
  const c = r && conv(r.conv);
  const quiet = !state.settings.sound || state.status === "dnd" || quietNow() || (c && muted(c));
  tones.set(calling ? "out" : r && !call && !quiet ? "in" : null);
}

/** In a chat with a call going: join it (or go back to it). */
function callBar(c) {
  const info = state.calls[c.id];
  if (!info) return "";
  // (Your own call: the strip at the top says so.)
  if (state.call?.info.id === info.id) return "";
  return `<div class="callbar" role="status">${icons.phone}<span><b>${esc(info.video ? "Video call" : "Call")} going</b> · ${info.people.map((u) => esc(nameOf(u).split(" ")[0])).join(", ")}</span><button class="small primary" data-act="calljoin" data-id="${c.id}">Join</button></div>`;
}

/** Someone's calling you. */
function ringCard() {
  const r = state.ringing;
  if (!r || state.view !== "chat") return "";
  const c = conv(r.conv);
  return `<div class="ring" role="alertdialog" aria-label="${esc(nameOf(r.by))} is calling">
    <div class="ringtitle">${r.video ? "Video call" : "Call"}${c && c.kind !== "direct" ? ` · ${esc(titleOf(c))}` : ""}</div>
    <div class="ringwho">${personAvatar(r.by, "big")}<b>${esc(nameOf(r.by))}</b></div>
    <div class="ringbuttons"><button class="small danger" data-act="calldecline">Decline</button><button class="small answer" data-act="calljoin" data-id="${r.conv}">Answer</button></div>
  </div>`;
}

/** Away from your call: back to it or hang up (atop the sidebar, or
 * floating where there's none). */
function callStrip(inSide) {
  const call = state.call;
  if (!call?.away || (!inSide && state.view !== "settings")) return "";
  const c = conv(call.info.conv);
  const others = call.info.people.filter((u) => u.id !== state.me.id).map((u) => esc(nameOf(u).split(" ")[0]));
  const meeting = c?.purpose === "meeting" ? meetingOf(c.id)?.title : null;
  const what = !call.answered && c?.kind === "direct" ? `Calling ${esc(titleOf(c))}…` : meeting ? `In ${esc(meeting)}` : others.length ? `In a call with ${others.join(", ")}` : "In a call";
  return `<div class="callstrip${inSide ? " inside" : ""}" role="status"><div class="callstripwhat">${icons.phone}<span><b>${what}</b><br>The call keeps going</span></div>
    <div class="callstripbuttons"><button class="small primary" data-act="callback">Back to the call</button>
    <button class="small danger" data-act="callleave">${c?.kind === "direct" ? "Hang up" : "Leave"}</button></div></div>`;
}

/** The call fills the window: a shared screen big, people as tiles. */
function callSheet() {
  const call = state.call;
  const info = call.info;
  const c = conv(info.conv);
  const peopleIds = info.people.map((u) => u.id);
  const sharing = call.sharing != null && (call.sharing === state.me.id ? call.screen : peopleIds.includes(call.sharing)) ? call.sharing : null;
  const tile = (u) => {
    const p = call.people.get(u.id) || {};
    const talking = p.talking && performance.now() - p.talking < 600;
    const camOn = u.id === state.me.id ? call.cam : p.lastCam && performance.now() - p.lastCam < 3000;
    const muted = u.id === state.me.id && !call.mic;
    const [bg, fg] = colours(u.id + nameOf(u));
    const rec = info.recording;
    const tag = rec ? (rec.in?.includes(u.id) ? `<span class="rectag in">In the recording</span>` : `<span class="rectag">Not in the recording</span>`) : "";
    return `<div class="calltile${talking ? " talking" : ""}" data-person="${u.id}">${tag}
      ${camOn ? `<div class="callslot" data-call-slot="${u.id}-cam"></div>` : `<span class="callface" style="background:${bg};color:${fg}">${esc(initials(nameOf(u)))}</span>`}
      <span class="callname">${muted ? icons.micOff : ""}${esc(u.id === state.me.id ? "You" : nameOf(u))}</span>
    </div>`;
  };
  // Calling one person: their face waits beside yours until they answer.
  const direct = c?.kind === "direct";
  const callee = direct && !call.answered ? c.members.find((u) => u.id !== state.me.id) : null;
  const waiting = callee
    ? (() => {
        const [bg, fg] = colours(callee.id + nameOf(callee));
        return `<div class="calltile calling"><span class="callface" style="background:${bg};color:${fg}">${esc(initials(nameOf(callee)))}</span><span class="callname">Calling ${esc(nameOf(callee))}…</span></div>`;
      })()
    : "";
  const stage = sharing != null
    ? `<div class="callstage"><div class="callslot screen" data-call-slot="${sharing}-screen"></div><div class="calltiles over">${info.people.map(tile).join("")}</div></div>`
    : `<div class="callgrid n${Math.min(info.people.length + (waiting ? 1 : 0), 4)}">${info.people.map(tile).join("")}${waiting}</div>`;
  const who = callee ? "Ringing…" : sharing != null ? `${sharing === state.me.id ? "You're" : `${esc(nameOf(info.people.find((u) => u.id === sharing) || { username: "" }))} is`} sharing ${sharing === state.me.id ? "your" : "their"} screen` : `${info.people.length} in the call`;
  return `<main class="callsheet" aria-label="Call">
    <header class="callhead"><b>${esc(c ? titleOf(c) : "Call")}</b><span class="callwho">${who}</span><span style="flex:1"></span>
      <button class="callctl small" data-act="callaway" title="The call keeps going">${icons.back}Back to chats</button>
      <button class="callctl small" data-act="callfull">${icons.full}Full screen</button></header>
    ${recBanner(info)}
    <div class="callbody">${stage}${call.chat && c ? `<div class="boarddrawer">${chatSheet()}</div>` : ""}</div>
    <footer class="callcontrols">${recAskBox(info)}
      <button class="callctl${call.mic ? "" : " off"}" data-act="callmic" aria-pressed="${!call.mic}" aria-label="${call.mic ? "Mute" : "Unmute"}">${call.mic ? icons.mic : icons.micOff}</button>
      <button class="callctl${call.cam ? "" : " off"}" data-act="callcam" aria-pressed="${call.cam}" aria-label="${call.cam ? "Turn the camera off" : "Turn the camera on"}">${call.cam ? icons.video : icons.videoOff}</button>
      <button class="callctl${call.screen ? " on" : ""}" data-act="callshare" aria-pressed="${call.screen}">${icons.share}${call.screen ? "Stop sharing" : "Share screen"}</button>
      ${c?.purpose === "meeting" ? "" : `<button class="callctl${call.chat ? " on" : ""}" data-act="callchat" aria-pressed="${!!call.chat}">${icons.chat}Chat</button>`}
      ${info.recording?.by.id === state.me.id ? `<button class="callctl recording" data-act="recstop">${icons.recdot}Stop recording</button>` : info.can_record && !info.recording && info.people.length > 1 ? `<button class="callctl" data-act="callrecord">${icons.recdot}Record</button>` : ""}
      <button class="callctl leave" data-act="callleave">${direct ? "Hang up" : "Leave"}</button>
    </footer>
  </main>`;
}

/** Puts each person's live picture (kept by calls.js) in its place. */
function placeCallTiles() {
  const call = state.call;
  if (!call) return;
  for (const slot of document.querySelectorAll("[data-call-slot]")) {
    const [id, what] = slot.dataset.callSlot.split("-");
    const canvas = call.people.get(+id)?.[what];
    // (A camera's video moves only once it plays: moved before, WebKitGTK
    // never starts it.)
    if (canvas && canvas.parentElement !== slot && !(canvas.tagName === "VIDEO" && canvas.readyState < 2)) {
      slot.append(canvas);
      // (Out of its corner; moving a video pauses it.)
      if (canvas.tagName === "VIDEO") {
        canvas.style.cssText = "";
        canvas.play().catch(() => {});
      }
    }
  }
}
// While in a call: talking rings change in place (a redraw would eat
// clicks); a camera going on or off redraws.
setInterval(() => {
  const call = state.call;
  if (!call || call.away) return;
  const now = performance.now();
  let cams = "";
  for (const u of call.info.people) {
    const p = call.people.get(u.id) || {};
    cams += u.id === state.me.id ? (call.cam ? 1 : 0) : p.lastCam && now - p.lastCam < 3000 ? 1 : 0;
    document.querySelector(`[data-person="${u.id}"]`)?.classList.toggle("talking", !!p.talking && now - p.talking < 600);
  }
  const sharing = call.sharing ?? "";
  if (cams !== call.drawnCams || sharing !== call.drawnSharing) {
    call.drawnCams = cams;
    call.drawnSharing = sharing;
    render();
  }
}, 300);

// ---------- Whiteboards (Draw inside the chat) ----------

/** The board fills the window; the chat is a drawer beside it. */
function boardSheet() {
  const b = state.board;
  const c = conv(b.conv);
  const save = state.menu === "boardsave" ? `<div class="menu" role="menu" aria-label="Save the board" style="right:140px;top:54px;width:260px">
      <button role="menuitem" data-act="boardsnap">Post a snapshot to the chat</button>
      <button role="menuitem" data-act="boarddl" data-id="png">Download as a picture (PNG)</button>
      <button role="menuitem" data-act="boarddl" data-id="pdf">Download as PDF</button>
      <button role="menuitem" data-act="boarddl" data-id="xdraw">Download as a Draw file (.xdraw)</button>
    </div>` : "";
  return `<main class="sheet boardsheet" id="boardsheet">
    <header class="boardhead">
      <button class="textbtn" data-act="closeboard" aria-label="Back to ${esc(c ? titleOf(c) : "the chat")}">${icons.back}${esc(c ? titleOf(c) : "Back")}</button>
      <div class="boardtitle"><b>${esc(b.title)}</b><span class="hintline">${b.status ? esc(b.status) : "Saved in the chat as you draw"}</span></div>
      <button class="textbtn" data-act="boardchat" aria-pressed="${!!b.chat}">${icons.chat}Chat${c && unread(c) ? `<span class="tabdot"></span>` : ""}</button>
      <button class="textbtn" data-act="menu" data-id="boardsave" aria-haspopup="menu">${icons.download}Save</button>
      ${window.XENIFY_WEB ? "" : `<button class="textbtn" data-act="boarddraw" title="Opens the board in Xenify Draw: everyone still sees changes live">${icons.open}Open in Draw</button>`}
      <button class="textbtn" data-act="boardfull">${icons.full}Full screen</button>
    </header>
    ${save}
    <div class="boardbody"><div class="boardslot" id="boardslot"></div>${b.chat && c ? `<div class="boarddrawer">${chatSheet()}</div>` : ""}</div>
  </main>`;
}

/** Keeps the Draw frame over the board's slot (it lives outside the page
 *  that's redrawn, so it never reloads). */
function placeBoard() {
  const host = document.getElementById("boardhost");
  if (!host) return;
  const slot = document.getElementById("boardslot");
  const b = state.board;
  if (!b || !slot) {
    host.hidden = true;
    if (!b && host.firstChild) host.replaceChildren();
    return;
  }
  if (host.dataset.node !== String(b.node)) {
    host.dataset.node = String(b.node);
    const f = document.createElement("iframe");
    f.className = "boardframe";
    f.title = `Whiteboard: ${b.title}`;
    f.src = `${window.XENIFY_DRAW_AT || "draw/"}index.html?embed=board`;
    host.replaceChildren(f);
  }
  const r = slot.getBoundingClientRect();
  Object.assign(host.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
  host.hidden = false;
}
window.addEventListener("resize", () => placeBoard());

async function openBoard(convId, b) {
  state.board = { conv: convId, node: b.node, title: b.title, chat: false, status: "Opening…" };
  state.menu = null;
  render();
}

/** Where messages to the Draw frame may go: only the page it was opened
 *  from ("/" when that's this app's own origin), never anything it went to. */
function boardTarget(frame) {
  try {
    const at = new URL(frame.src, location.href).origin;
    return at === location.origin ? "/" : at;
  } catch {
    return "/";
  }
}

/** What the Draw frame says. */
window.addEventListener("message", async (e) => {
  const frame = document.querySelector("#boardhost iframe");
  if (!frame || e.source !== frame.contentWindow) return;
  const m = e.data || {};
  const b = state.board;
  if (!b) return;
  if (m.t === "ready") {
    const who = await tauri.core.invoke("board_session");
    frame.contentWindow.postMessage({ t: "open", url: who.url, token: who.token, account: who.account, node: b.node, name: b.title }, boardTarget(frame));
  } else if (m.t === "opened") {
    b.status = m.others ? `Drawing together with ${m.others} other${m.others === 1 ? "" : "s"}` : "Saved in the chat as you draw";
    render();
  } else if (m.t === "error") {
    showToast(`The board: ${m.message}`);
  } else if (m.t === "picture" && m.purpose === "snapshot") {
    const c = conv(b.conv);
    if (c) sendFiles(c, [new File([m.bytes], m.name, { type: "image/png" })]);
    showToast("Posting a snapshot to the chat…");
  } else if (m.t === "file") {
    try {
      const p = await tauri.core.invoke("save_download", m.bytes.buffer ?? m.bytes, { headers: { name: encodeURIComponent(m.name) } });
      showToast(`Saved in Downloads: ${p.split(/[\\/]/).pop()}`);
    } catch (err) {
      showToast(String(err));
    }
  }
});

/** A file message's details, or null for text. */
function fileOf(text) {
  if (!text?.startsWith("~file:")) return null;
  try {
    return JSON.parse(text.slice(6));
  } catch {
    return null;
  }
}

function fileKind(f) {
  if (f.voice) return "voice";
  if (f.recording) return "recording";
  return f.mime.startsWith("image/") ? "picture" : f.mime.startsWith("video/") ? "video" : f.mime.startsWith("audio/") ? "audio" : "file";
}

/** What a message says in the list, notifications and search. */
function describe(text) {
  const b = boardOf(text);
  if (b) return `Shared a whiteboard: ${b.title}`;
  const f = fileOf(text);
  if (!f) return text;
  if (f.voice) return `Voice message · ${clock(f.voice.secs)}`;
  if (f.recording) return `Recorded the call · ${clock(f.recording.secs)}`;
  return { picture: "Sent a picture", video: `Sent a video: ${f.name}`, audio: `Sent a recording: ${f.name}`, file: `Sent a file: ${f.name}` }[fileKind(f)];
}

/** Where pictures and players read a chat file (opened as they go). */
function fileUrl(f) {
  // In the browser: the opened file, once it's here (drawn again then).
  if (window.XENIFY_FILE_URL) return window.XENIFY_FILE_URL(f, () => render());
  return tauri.core.convertFileSrc(`${f.id}/${f.size}/${f.part}`, "xfile") + "?t=" + encodeURIComponent(f.mime);
}

function sizeText(n) {
  if (n < 1024) return `${n} bytes`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function fileCard(m, f) {
  const k = fileKind(f);
  if (k === "voice") return voiceCard(m, f);
  if (k === "recording") return recordingCard(m, f);
  const save = `<button class="iconbtn" data-act="savefile" data-id="${m.id}" aria-label="Save ${esc(f.name)} in Downloads" title="Save in Downloads">${icons.download}</button>`;
  if (!tauri) return `<div class="filecard"><span class="fileicon">${esc((f.name.split(".").pop() || "").slice(0, 4).toUpperCase())}</span><span class="filebody"><b>${esc(f.name)}</b><span class="hintline">${sizeText(f.size)}</span></span></div>`;
  if (k === "picture") return `<button class="picture" data-act="viewpic" data-id="${m.id}" aria-label="Show ${esc(f.name)} bigger"><img src="${fileUrl(f)}" alt="${esc(f.name)}" loading="lazy"></button>`;
  if (k === "video") return `<div class="clip">${f.size > PLAY_LIMIT ? `<div class="cliptoobig">Too big to play here: save it to watch.</div>` : `<video controls preload="metadata" data-keep="${esc(f.id)}" data-file="${m.id}" ${clips.get(f.id) ? `src="${clips.get(f.id)}"` : ""} aria-label="${esc(f.name)}"></video>`}<div class="clipbar"><b>${esc(f.name)}</b><span class="hintline">${sizeText(f.size)}</span>${save}</div></div>`;
  if (k === "audio") return `<div class="filecard"><audio controls preload="metadata" data-keep="${esc(f.id)}" data-file="${m.id}" ${clips.get(f.id) ? `src="${clips.get(f.id)}"` : ""} aria-label="${esc(f.name)}"></audio>${save}</div>`;
  return `<div class="filecard"><span class="fileicon">${esc((f.name.split(".").pop() || "file").slice(0, 4).toUpperCase())}</span><span class="filebody"><b>${esc(f.name)}</b><span class="hintline">${sizeText(f.size)}</span></span>${save}</div>`;
}

/** Clips play from memory (this web view can't stream from the app's own
 *  addresses): file id → a local address, made the first time it's shown. */
const clips = new Map();
const PLAY_LIMIT = 512 * 1024 * 1024;

function loadClips() {
  for (const v of document.querySelectorAll("video[data-file]:not([src]), audio[data-file]:not([src])")) {
    const [, m] = findMsg(+v.dataset.file);
    const f = m && fileOf(m.text);
    if (!f || clips.has(f.id)) continue;
    clips.set(f.id, "");
    (window.XENIFY_FILE_BLOB ? window.XENIFY_FILE_BLOB(f) : fetch(fileUrl(f)).then((r) => (r.ok ? r.blob() : Promise.reject(new Error(r.status)))))
      .then((b) => {
        clips.set(f.id, URL.createObjectURL(new Blob([b], { type: f.mime })));
        render();
      })
      .catch((e) => {
        clips.delete(f.id);
        showToast(`This clip didn't load: ${e?.message || e}`);
      });
  }
}

/** Uploads a File to `c`: its details, not posted. */
async function uploadFile(c, file) {
  const [id, part] = await tauri.core.invoke("file_begin", { conv: c.id, size: file.size });
  for (let at = 0, n = 0; at < file.size; at += part, n++) {
    const bytes = new Uint8Array(await file.slice(at, at + part).arrayBuffer());
    await tauri.core.invoke("file_part", bytes.buffer, { headers: { upload: id, part: String(n) } });
  }
  return tauri.core.invoke("file_end", { id, name: file.name, size: file.size, mime: file.type || "" });
}

/** Sends files picked or dropped: uploads each, then posts it. `files` are
 *  File objects (picked) or paths (dropped on the window). */
async function sendFiles(c, files, extra = {}) {
  for (const file of files) {
    const upload = crypto.randomUUID();
    const name = typeof file === "string" ? file.split(/[\\/]/).pop() : file.name;
    const u = { id: upload, conv: c.id, name, sent: 0, total: typeof file === "string" ? 0 : file.size, error: "" };
    state.uploads.push(u);
    render();
    try {
      let info;
      if (typeof file === "string") {
        info = await tauri.core.invoke("file_send_path", { conv: c.id, path: file, upload });
      } else {
        const [id, part] = await tauri.core.invoke("file_begin", { conv: c.id, size: file.size });
        let n = 0;
        for (let at = 0; at < file.size || (at === 0 && n === 0); at += part, n++) {
          if (u.cancelled) throw new Error("Cancelled.");
          const bytes = new Uint8Array(await file.slice(at, at + part).arrayBuffer());
          await tauri.core.invoke("file_part", bytes.buffer, { headers: { upload: id, part: String(n) } });
          u.sent = Math.min(file.size, at + part);
          render();
          if (file.size === 0) break;
        }
        info = await tauri.core.invoke("file_end", { id, name: file.name, size: file.size, mime: file.type || "" });
      }
      state.uploads = state.uploads.filter((x) => x !== u);
      if (!u.cancelled) post(c, "~file:" + JSON.stringify({ ...info, ...extra }));
    } catch (e) {
      u.error = String(e?.message || e);
      render();
    }
  }
}

/** A picture shown big, over everything (Esc or a click closes it). */
function pictureView() {
  if (state.bigPicture == null) return "";
  const [, m] = findMsg(state.bigPicture);
  const f = m && fileOf(m.text);
  if (!f) return "";
  return `<div class="bigpic" role="dialog" aria-label="${esc(f.name)}" data-act="closepic">
    <img src="${fileUrl(f)}" alt="${esc(f.name)}">
    <div class="bigbar"><b>${esc(f.name)}</b><span>${sizeText(f.size)}</span>
      <button class="small" data-act="savefile" data-id="${m.id}">${icons.download}Save</button>
      <button class="small" data-act="closepic">Close</button></div>
  </div>`;
}

/** Files dragged over the window: where they'll go. */
function dropHint() {
  const c = state.view === "chat" ? current() : null;
  if (!state.dragging || !c || !c.can_post) return "";
  return `<div class="drophint" aria-hidden="true"><div>${icons.clip}Drop to send to ${esc(titleOf(c))}</div></div>`;
}

function uploadChips(c) {
  const list = state.uploads.filter((u) => u.conv === c.id);
  if (!list.length) return "";
  return `<div class="uploads">${list.map((u) => {
    const pct = u.total ? Math.round((u.sent / u.total) * 100) : 0;
    const what = u.error ? `<span class="uperror">${esc(u.error)}</span>` : `<span class="hintline">Sending · ${pct}%</span><span class="upbar"><span style="width:${pct}%"></span></span>`;
    return `<span class="upchip">${icons.clip}<b>${esc(u.name)}</b>${what}<button class="iconbtn tiny" data-act="upcancel" data-id="${u.id}" aria-label="${u.error ? "Dismiss" : "Cancel"}">${icons.close}</button></span>`;
  }).join("")}</div>`;
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
  if (SHOWS_SHEET.has(act)) state.phoneSheet = true;
  switch (act) {
    case "phoneback": state.phoneSheet = false; state.menu = null; break;
    case "abunsel": state.ab.sel = null; break;
    case "open": return open(Number(id));
    case "calendar": return state.view === "calendar" ? undefined : openCalendar();
    case "todaymore": state.cal.more = !state.cal.more; break;
    case "calitem": {
      const ev = findEvent(Number(id));
      if (!ev) break;
      if (ev.kind === "booking") { state.ab.sel = JSON.parse(ev.link).thing; return openPeople("things"); }
      if (ev.kind === "away") return openAway();
      if (ev.kind === "holiday") return showToast(holidayNote(ev));
      if (ev.meeting && meetingOf(ev.meeting)) return openMeeting(ev.meeting);
      return ev.kind === "reminder" && ev.link && state.view !== "calendar" ? openLinked(ev) : editEvent(ev);
    }
    case "openmeeting": openMeeting(Number(id)); return;
    case "people": return state.view === "people" ? undefined : openPeople();
    case "chats": state.view = "chat"; state.menu = null; state.phoneSheet = false; break;
    case "abview": return openPeople(/^\d+$/.test(id) ? Number(id) : id);
    case "absel": state.ab.sel = state.ab.view === "everyone" || state.ab.view === "things" ? Number(id) : id; break;
    case "contactnew": state.dialog = { kind: "contact", contact: { book: state.ab.view === "mine" ? 0 : state.ab.view, uid: "", name: "", email: "", phone: "", company: "", role: "", notes: "", username: "", added: "" }, error: "" }; break;
    case "contactedit": { const x = state.ab.list.find((c) => c.uid === id); if (x) state.dialog = { kind: "contact", contact: { ...x }, error: "" }; break; }
    case "contactdelete": {
      const x = state.ab.list.find((c) => c.uid === id);
      if (x) state.confirm = { title: `Delete ${x.name}?`, text: state.ab.view === "mine" ? "From My contacts, on all your computers." : "From this contact list, for everyone it's shared with.", action: "Delete", run: () => tauri.core.invoke("contact_delete", { book: x.book, uid: x.uid }).then(() => { state.ab.sel = null; loadAb(); }).catch((err) => showToast(String(err))) };
      break;
    }
    case "abcopy": {
      const x = state.ab.list.find((c) => c.uid === state.ab.sel);
      if (!x) return;
      const to = Number(id);
      const where = to === 0 ? "My contacts" : state.ab.books.find((b) => b.conv === to)?.name || "that contact list";
      tauri.core.invoke("contacts_list", { book: to }).then((there) => {
        const same = (c) => (x.email && c.email.toLowerCase() === x.email.toLowerCase()) || (x.username && c.username === x.username);
        if (there.some(same)) return showToast(`${x.name} is in ${where} already.`);
        return tauri.core.invoke("contact_save", { contact: { ...x, book: to, uid: "", added: "" } }).then(() => { showToast(`Added to ${where}.`); loadAb(); });
      }).catch((err) => showToast(String(err)));
      return;
    }
    case "abadduser": {
      const u = state.ab.directory.find((x) => x.id === Number(id));
      if (u) tauri.core.invoke("contact_save", { contact: { book: 0, uid: "", name: nameOf(u), email: "", phone: "", company: "", role: "", notes: "", username: u.username, added: "" } }).then(() => { showToast(`${nameOf(u)} is in My contacts.`); loadAb(); });
      return;
    }
    case "abemail": state.view = "chat"; return startCompose({ to: id });
    case "abmessage": state.wantOpen = { username: id }; tell({ t: "open_direct", user: id }); state.view = "chat"; break;
    case "abinvite": case "abinviteuser": {
      const x = act === "abinvite" ? state.ab.list.find((c) => c.uid === id) : null;
      const u = act === "abinviteuser" ? state.ab.directory.find((y) => y.id === Number(id)) : x?.username ? state.ab.directory.find((y) => y.username === x.username) || { id: -1, username: x.username, display_name: x.name } : null;
      const start = Math.ceil(Date.now() / HOUR) * HOUR;
      editEvent({ id: 0, title: "", start, end: start + HOUR, all_day: false, notes: "", kind: "event", link: "", remind: 10, reminded: false, done: false });
      if (u) state.dialog.picked.push(u);
      else if (x?.email) state.dialog.guests.push({ email: x.email, name: x.name, answer: "" });
      askBusy(state.dialog);
      return render();
    }
    case "booknew": state.dialog = { kind: "book", conv: 0, title: "", q: "", picked: [], groups: [], roles: {}, error: "" }; break;
    case "bookshare": {
      const b = state.ab.books.find((x) => x.conv === Number(id));
      state.dialog = { kind: "book", conv: Number(id), title: "", q: "", picked: [], groups: (b?.groups || []).map((g) => ({ ...g })), roles: {}, error: "" };
      break;
    }
    case "pickgroup": {
      const c = conv(Number(id));
      if (c) state.dialog.groups.push({ conv: c.id, title: titleOf(c), count: c.member_count, owner: false });
      state.dialog.q = "";
      break;
    }
    case "bookvcard": {
      const b = state.ab.books.find((x) => x.conv === Number(id));
      tauri.core.invoke("contacts_vcard_text", { book: Number(id) })
        .then((text) => tauri.core.invoke("save_download", new TextEncoder().encode(text).buffer, { headers: { name: encodeURIComponent(`${b?.name || "Contacts"}.vcf`) } }))
        .then((path) => showToast(path ? `Saved to ${path}` : "Saved."))
        .catch((err) => showToast(String(err)));
      return;
    }
    case "bookleave": {
      const b = state.ab.books.find((x) => x.conv === Number(id));
      state.confirm = { title: `Leave ${b?.name || "this contact list"}?`, text: "It leaves your computers. Someone who can change it can share it with you again.", action: "Leave", run: () => { tauri.core.invoke("book_leave", { conv: Number(id) }); state.ab.view = "mine"; setTimeout(loadAb, 600); } };
      break;
    }
    case "thingnew": state.dialog = { kind: "thing", thing: { id: 0, name: "", kind: "room", place: "", seats: 0, by_day: false, approver: null }, q: "", pickApprover: false, wantApprover: false, error: "" }; break;
    case "thingedit": { const t = thingOf(Number(id)); if (t) state.dialog = { kind: "thing", thing: { ...t }, q: "", pickApprover: false, wantApprover: !!t.approver, error: "" }; break; }
    case "thingapprover": { const u = state.people.find((x) => x.id === Number(id)); if (u) Object.assign(state.dialog, { pickApprover: false, q: "" }), (state.dialog.thing.approver = u); break; }
    case "thingremove": {
      const t = thingOf(Number(id));
      state.confirm = { title: `Take ${t?.name} away?`, text: "Its bookings go with it, and it can't be booked any more.", action: "Take away", run: () => { tell({ t: "remove_thing", id: Number(id) }); state.dialog = null; state.ab.sel = null; } };
      break;
    }
    case "bookingyes": tell({ t: "answer_booking", booking: Number(id), yes: true }); break;
    case "bookingno": tell({ t: "answer_booking", booking: Number(id), yes: false }); break;
    case "bookingcancel": tell({ t: "cancel_booking", booking: Number(id) }); break;
    case "awayopen": state.menu = null; return openAway();
    case "awayadd": { readAway(); const d = state.dialog; const last = d.times[d.times.length - 1]; const from = last ? addDays(startOfDay(last[1]), 7) : addDays(startOfDay(Date.now()), 1); d.times.push([from, addDays(from, 1)]); break; }
    case "awaydel": readAway(); state.dialog.times.splice(Number(id), 1); break;
    case "awayoff": return saveAway(true);
    case "meetanswer": answerMeeting(Number(id), el.dataset.answer); break;
    case "meetjoin": {
      const mc = conv(Number(id));
      state.dialog = null;
      if (!mc) { showToast("The meeting's call isn't ready yet: try again in a moment."); break; }
      if (state.calls[mc.id]) return joinCall(mc.id);
      return startCall(mc, true);
    }
    case "meetedit": {
      const m = meetingOf(Number(id));
      if (!m) break;
      editEvent({ id: 0, title: m.title, start: m.start, end: m.end, all_day: false, notes: m.notes, kind: "event", link: "", remind: 10, reminded: false, done: false }, m);
      askBusy(state.dialog);
      return;
    }
    case "meetcancel": {
      const mid = Number(id);
      state.confirm = { title: "Call the meeting off?", text: "Everyone invited is told, here and by email, and it leaves their calendars.", action: "Call it off", run: () => tauri.core.invoke("meeting_cancel", { conv: mid, bookings: meetingBookings(mid) }).then(() => { state.dialog = null; showToast("Called off. Everyone invited is told."); }).catch((err) => showToast(String(err))) };
      break;
    }
    case "meetday": { const d = state.dialog; readEventDialog(); d.day = addDays(d.day, Number(id)); break; }
    case "meetslot": {
      const d = state.dialog;
      const ev = readEventDialog();
      const len = ev.end - ev.start;
      ev.start = Number(id);
      ev.end = ev.start + len;
      d.day = startOfDay(ev.start);
      break;
    }
    case "lanepick": {
      const d = state.dialog;
      const ev = readEventDialog();
      const r = el.getBoundingClientRect();
      const from = Number(el.dataset.from), to = Number(el.dataset.to);
      const t = Math.floor((from + ((e.clientX - r.left) / r.width) * (to - from)) / (30 * MIN)) * 30 * MIN;
      const len = ev.end - ev.start;
      ev.start = t;
      ev.end = t + len;
      d.day = startOfDay(t);
      break;
    }
    case "addguest": addGuest(state.dialog); break;
    case "pickthing": {
      const d = state.dialog;
      const t = thingOf(Number(id));
      if (t && !d.things.some((x) => x.id === t.id)) d.things.push({ id: t.id, name: t.name });
      d.q = "";
      askBusy(d);
      break;
    }
    case "unthing": state.dialog.things = state.dialog.things.filter((t) => t.id !== Number(id)); break;
    case "pickcontact": {
      const d = state.dialog;
      const x = (d.found || []).filter((c) => c.email && !d.guests.some((g) => g.email.toLowerCase() === c.email.toLowerCase()))[Number(id)];
      if (x) d.guests.push({ email: x.email, name: x.name, answer: "" });
      d.q = "";
      break;
    }
    case "unguest": state.dialog.guests.splice(Number(id), 1); break;
    case "calitemlater": {
      const ev = state.cal.snoozed.find((x) => x.id === Number(id));
      return ev && openLinked(ev);
    }
    case "calprev": state.cal.weekStart = addDays(state.cal.weekStart, -7); return loadWeek();
    case "calsmenu": state.menu = state.menu === "cals" ? null : "cals"; break;
    case "calshow": {
      const hidden = new Set(state.settings.hiddenCals || []);
      if (el.checked) hidden.delete(Number(id));
      else hidden.add(Number(id));
      state.settings.hiddenCals = [...hidden];
      return loadWeek();
    }
    case "calsharednew": state.menu = null; state.dialog = { kind: "book", cal: true, conv: 0, title: "", q: "", picked: [], groups: [], roles: {}, error: "" }; break;
    case "calshare": state.menu = null; state.dialog = { kind: "book", cal: true, conv: Number(id), title: "", q: "", picked: [], groups: [], roles: {}, error: "" }; break;
    case "calmore": {
      const sc = sharedCal(Number(id));
      if (!sc) break;
      const r = el.getBoundingClientRect();
      state.menu = null;
      state.ctx = { x: r.left - 200, y: r.bottom + 4, items: [
        ...(sc.yours ? [{ label: "Rename…", run: () => (state.dialog = { kind: "rename", cal: sc.conv, value: sc.name }) }] : []),
        // The only one who can change it: leaving would hand it to someone
        // who only looks, so it's deleted instead.
        ...(sc.yours && sc.people.filter((p) => p.owner).length <= 1 ? [] : [{ label: "Leave", hint: "It goes from your calendar", danger: true, run: () => (state.confirm = { title: `Leave ${sc.name}?`, text: sc.yours ? "You won't see it any more. The others who can change it keep it." : "You won't see it any more. Someone who can change it would have to share it with you again.", action: "Leave", run: () => tauri.core.invoke("shared_cal_leave", { conv: sc.conv }).then(refreshCalendar) }) }]),
        ...(sc.yours ? [{ label: "Delete calendar…", hint: "For everyone in it", danger: true, run: () => (state.confirm = { title: `Delete ${sc.name}?`, text: sc.member_count > 1 ? `It goes, with its events, from everyone's calendar (${sc.member_count} people). This can't be undone.` : "It goes, with its events. This can't be undone.", action: "Delete", run: () => tauri.core.invoke("shared_cal_remove", { conv: sc.conv }).then(refreshCalendar, (e) => showToast(String(e))) }) }] : []),
      ] };
      break;
    }
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
      if (ev?.shared) tauri.core.invoke("shared_cal_delete", { conv: ev.shared, uid: ev.uid }).then(refreshCalendar, (err) => showToast(String(err)));
      else if (ev?.id) tauri.core.invoke("cal_delete", { id: ev.id }).then(refreshCalendar);
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
    case "mailremove": state.confirm = { title: `Remove ${id}?`, text: serverEmail() ? "Its settings, sign-in, drafts and the copy of its mail on your Xenify server are removed. Nothing changes in the mailbox itself." : "Its settings, saved password, drafts and the copy of its mail on this computer are removed. Nothing changes on the mail server.", action: "Remove", run: () => removeMail(id) }; break;
    case "mailconnect":
      tauri.core.invoke("mail_connect", { provider: id }).then((r) => {
        // The desktop app gets the page's address and opens it in the browser.
        if (r?.url) tauri.core.invoke("open_link", { url: r.url });
        showToast("Sign in on the page that opened. Your email shows here when you're done.");
      }).catch((e) => showToast(String(e)));
      break;
    case "mailadvanced": state.mailSetup.advanced = !state.mailSetup.advanced; break;
    case "askopen": return askOpenLink(el.dataset.url);
    case "saveattachment": return saveAttachment(Number(id));
    case "hit": return jumpTo(Number(id));
    case "thread": { const [, m] = findMsg(Number(id)); if (m) openThread(m); return; }
    case "threadclose": state.thread = null; break;
    case "threadsend": return sendReply();
    case "morehits":
      if (!state.kept.more || state.kept.loading) return;
      state.kept.loading = true;
      tell({ t: "search", q: state.kept.q, sort: state.kept.sort, after: state.kept.more, tag: "find" });
      break;
    case "askai": {
      const q = state.q.trim();
      if (!q) return;
      state.answer = { q, busy: true, text: "", sources: [], error: "" };
      tell({ t: "ask", q, tag: "find" });
      break;
    }
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
    case "newchat":
      state.menu = null;
      state.dialog = { kind: id, title: "", q: "", picked: [], open: can("chat.create_open_channel") };
      break;
    case "attach": document.getElementById("filepick")?.click(); return;
    case "secret": state.dialog = { kind: "secret", into: id, conv: state.sel, days: 1, error: "", busy: false }; render(); return document.getElementById("stext")?.focus();
    case "voicerec": if (c) startRecording(c.id, "chat"); return;
    case "voicepause": state.rec?.r.pause(); return;
    case "voicecancel": cancelRecording(); return;
    case "voicesend": sendVoice(); return;
    case "voiceplay": playVoice(Number(id)); return;
    case "voicespeed": voiceSpeed(Number(id)); return;
    case "voicetext": { const [, m] = findMsg(Number(id)); const f = m && fileOf(m.text); if (f) makeVoiceText(f, true); return; }
    case "voicesettings": state.view = "settings"; state.section = "voice"; loadVoice(); break;
    case "voiceget": getModel(state.settings.voiceModel); break;
    case "voicestopget": tauri.core.invoke("voice_stop_download"); return;
    case "voiceremove": tauri.core.invoke("voice_remove", { id: state.settings.voiceModel }).then(loadVoice).catch((err) => showToast(String(err))); return;
    case "leavevoice": if (state.noAnswer) startRecording(state.noAnswer.conv, "call"); return;
    case "callagain": { const x = state.noAnswer && conv(state.noAnswer.conv); cancelRecording(); state.noAnswer = null; if (x) startCall(x, false); break; }
    case "noanswerclose": cancelRecording(); state.noAnswer = null; break;
    case "callstart": return startCall(c, id === "video");
    case "calljoin": return joinCall(+id);
    case "calldecline": {
      const r = state.ringing;
      if (r) { state.declined.add(r.id); tell({ t: "call", conv: r.conv, action: "decline" }); }
      state.ringing = null;
      break;
    }
    case "callleave": leaveCall(); break;
    case "callmic": if (!state.call.mic && askFirst("mic")) break; state.call.setMic(!state.call.mic); return;
    case "callcam": if (!state.call.cam && askFirst("cam")) break; state.call.setCamera(!state.call.cam); return;
    case "callshare": if (!state.call.screen && askFirst("screen")) break; state.call.setScreen(!state.call.screen); return;
    case "callrecord": state.dialog = { kind: "record", transcript: state.settings.meetingTranscript }; break;
    case "recstart": {
      const d = state.dialog;
      state.recWantText = canMakeText() && !!document.getElementById("rectext")?.checked;
      state.dialog = null;
      if (d && state.call) tell({ t: "call", conv: state.call.info.conv, action: "record", video: false });
      break;
    }
    case "recstop": if (state.call) tell({ t: "call", conv: state.call.info.conv, action: "stop_recording", video: false }); break;
    case "recaskno": state.recAsk = null; break;
    case "recaskyes": {
      const what = state.recAsk;
      state.recAsk = null;
      const call = state.call;
      if (!call) break;
      tell({ t: "call", conv: call.info.conv, action: "in_recording", video: false });
      if (what === "mic") call.setMic(true);
      else if (what === "cam") call.setCamera(true);
      else if (what === "screen") call.setScreen(true);
      break;
    }
    case "transcript": { const [, tm] = findMsg(+id); const tf = tm && fileOf(tm.text); if (tf) { state.dialog = { kind: "transcript", msg: +id }; loadTranscript(tf.recording.transcript); } break; }
    case "transcriptsave": { const [, tm] = findMsg(+id); const tf = tm && fileOf(tm.text); if (tf) saveTranscript(tf); return; }
    case "callchat": state.call.chat = !state.call.chat; break;
    case "callaway": state.call.away = true; state.sel = state.call.info.conv; break;
    case "callback": state.call.away = false; break;
    case "callfull":
      (document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen()).catch(() => showToast("Full screen isn't available here."));
      return;
    case "newboard": state.dialog = { kind: "board", conv: c.id, title: "" }; break;
    case "openboard": {
      const [cid, bm] = findMsg(+id);
      const bb = bm && boardOf(bm.text);
      if (bb) return openBoard(bm.conv ?? cid, bb);
      return;
    }
    case "closeboard": state.board = null; state.menu = null; break;
    case "boardchat": state.board.chat = !state.board.chat; break;
    case "boarddraw": {
      const bd = state.board;
      tauri.core.invoke("board_open_in_draw", { info: { node: bd.node, title: bd.title } })
        .then(() => showToast("Opening the board in Xenify Draw: changes there reach everyone live."), (e) => showToast(String(e)));
      return;
    }
    case "boardfull":
      // The whole screen: the board and its bar.
      (document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen()).catch(() => showToast("Full screen isn't available here."));
      return;
    case "boardsnap": state.menu = null; document.querySelector("#boardhost iframe")?.contentWindow.postMessage({ t: "snapshot" }, "*"); break;
    case "boarddl": state.menu = null; document.querySelector("#boardhost iframe")?.contentWindow.postMessage({ t: "download", format: id }, "*"); break;
    case "upcancel": {
      const u = state.uploads.find((x) => x.id === id);
      if (u) u.cancelled = true;
      state.uploads = state.uploads.filter((x) => x.id !== id);
      break;
    }
    case "savefile": {
      const [, fm] = findMsg(+id);
      const sf = fm && fileOf(fm.text);
      if (sf) tauri.core.invoke("file_save", { info: sf }).then((p) => showToast(`Saved in Downloads: ${p.split(/[\\/]/).pop()}`), (e) => showToast(String(e)));
      return;
    }
    case "viewpic": state.bigPicture = +id; break;
    case "closepic": state.bigPicture = null; break;
    case "takeout": {
      const u = c.members.find((x) => x.id === +id);
      if (!u) return;
      state.menu = null;
      state.confirm = { title: `Take ${nameOf(u)} out of ${titleOf(c)}?`, text: "They can't read anything sent here from now on. You can add them again later.", action: "Take out", run: () => tell({ t: "remove_person", conv: c.id, user: u.username }) };
      break;
    }
    case "addpeople": state.menu = null; state.dialog = { kind: "add", conv: c.id, title: "", q: "", picked: [] }; break;
    case "addposter": state.menu = null; state.dialog = { kind: "add", conv: c.id, title: "", q: "", picked: [], owner: true }; break;
    case "setowner": {
      const u = c.members.find((x) => x.id === +id);
      if (u) tell({ t: "set_role", conv: c.id, user: u.username, owner: !!el.dataset.on });
      break;
    }
    case "pick": { const u = state.people.find((x) => x.id === Number(id)); if (u && !state.dialog.picked.some((x) => x.id === u.id)) state.dialog.picked.push(u); state.dialog.q = ""; state.people = []; if (state.dialog.kind === "event") askBusy(state.dialog); break; }
    case "unpick": state.dialog.picked = state.dialog.picked.filter((x) => x.id !== Number(id)); break;
    case "closedialog": state.dialog = null; break;
    case "certtrust": case "certonce": {
      const seen = state.dialog?.seen;
      state.dialog = null;
      if (!seen) break;
      tauri.core.invoke("certificate", { fingerprint: seen.fingerprint, remember: act === "certtrust" })
        .then(() => signIn())
        .catch((e) => { state.signin.error = String(e); render(); });
      break;
    }
    case "snooze": return snooze(id, Number(el.dataset.until));
    case "duedismiss": dismissDue(id); break;
    case "dueopen": return openDue(id);
    case "autostart":
      tauri.core.invoke("autostart_set", { on: !state.autostart }).then(() => { state.autostart = !state.autostart; render(); }).catch((err) => showToast(String(err)));
      return;
    case "pushon":
      tauri.core.invoke("push_on").then((st) => { state.push = st; render(); }).catch((err) => showToast(String(err)));
      return;
    case "pushoff":
      tauri.core.invoke("push_off").then((st) => { state.push = st; render(); }).catch(() => {});
      return;
    case "focussearch": state.menu = null; render(); document.getElementById("search")?.focus(); return;
    case "signmode":
      Object.assign(state.signin, { join: !state.signin.join, fresh: false, typed: false, link: null, error: "", note: "" });
      render();
      document.getElementById(state.signin.join ? "si" : "su")?.focus();
      return;
    case "signback":
      Object.assign(state.signin, { join: false, fresh: false, forgot: false, emailed: false, typed: false, link: null, error: "", note: "" });
      render();
      document.getElementById("su")?.focus();
      return;
    case "signforgot": {
      const k = state.signin;
      if (k.emailed) return void emailCode("A new code is on its way to your email.");
      Object.assign(k, { join: true, fresh: true, forgot: true, link: null, invite: "" });
      emailCode("If this account has an email address, a code is on its way to it: type it here, then pick a new password. If it has none, ask an admin to reset your password.");
      return;
    }
    case "signout": signOut(); return;
    case "quit": backToFiles(); return;
    case "stepuse": case "stepemail": case "stepsetup": case "stepagain": case "codeskept": stepAct(act, id); return;
    case "mfaset": case "mfaoff": case "mfabackup": case "mfachoose": mfaAct(act, id); return;
    case "usehub": tauri.core.invoke("use_hub").then(() => location.reload()); return;
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
    case "settings": state.phoneSheet = false; state.view = state.view === "settings" ? "chat" : "settings"; if (state.view === "chat" && c) markRead(c); state.menu = null; if (state.view === "settings") loadProofing(); break;
    case "back": state.view = "chat"; state.phoneSheet = false; if (c) markRead(c); break;
    case "section":
      state.section = id;
      if (id === "apps") tell({ t: "list_apps" });
      if (id === "voice") loadVoice();
      if (id === "calendar") tauri.core.invoke("calendar_settings").then((s) => { state.calSettings = s; render(); });
      if (id === "notifications" && !window.XENIFY_WEB) tauri.core.invoke("autostart_get").then((on) => { state.autostart = on; render(); }).catch(() => {});
      if (id === "notifications" && window.XENIFY_WEB) tauri.core.invoke("push_state").then((st) => { state.push = st; render(); }).catch(() => {});
      break;
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
  if (act === "bookit") {
    e.preventDefault();
    return bookIt(Number(e.target.dataset.id));
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
  } else if (id.startsWith("ev_") && state.dialog?.kind === "event") {
    readEventDialog();
  } else if (id === "abq") {
    state.ab.q = e.target.value;
    render();
  } else if (id.startsWith("th_") && state.dialog?.kind === "thing") {
    readThing();
  } else if (id.startsWith("aw_") && state.dialog?.kind === "away") {
    readAway();
  } else if (id === "pickq") {
    state.dialog.q = e.target.value;
    findOnServer(state.dialog.q);
    // These dialogs show matches as you type.
    if (["event", "book", "thing"].includes(state.dialog.kind)) render();
    // Who in a meeting: people in your address books too.
    if (state.dialog.kind === "event" && tauri) {
      const want = state.dialog.q;
      tauri.core.invoke("contacts_find", { q: want }).then((list) => { if (state.dialog?.q === want) { state.dialog.found = list; render(); } }).catch(() => {});
    }
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
  } else if (id === "tdraft" && state.thread) {
    state.threadDrafts[`${state.thread.conv}:${state.thread.seq}`] = e.target.value;
    typing();
    sizeBox(e.target);
    const btn = document.querySelector(".threadpane .send");
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
  if (e.target.id === "hitsort") {
    state.kept = { ...state.kept, sort: e.target.value, list: [], more: "", q: "" };
    findOnServer(state.q);
    return render();
  }
  if (e.target.id === "mailfolder") {
    state.mail.folder = e.target.value;
    return reloadMail();
  }
  if (e.target.id === "mfrom" && state.compose) {
    state.compose.account = e.target.value;
    return scheduleDraft();
  }
  if (e.target.id === "rectext" && state.dialog?.kind === "record") {
    state.dialog.transcript = e.target.checked;
    return;
  }
  if (e.target.id === "vsendtext" && state.rec) {
    state.rec.sendText = e.target.checked;
    return;
  }
  if (e.target.id === "vmodel" || e.target.id === "vlang") {
    state.settings[e.target.id === "vmodel" ? "voiceModel" : "voiceLang"] = e.target.value;
    return render();
  }
  if (e.target.id === "filepick") {
    const c = current();
    if (c) sendFiles(c, [...e.target.files]);
    e.target.value = "";
    return;
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
  if (e.target.name === "ev_where" || e.target.id === "ev_length" || e.target.id === "ev_date" || e.target.id === "ev_alsocall") {
    readEventDialog();
    if (e.target.id === "ev_date") state.dialog.day = startOfDay(state.dialog.event.start);
    return render();
  }
  if (state.dialog?.kind === "thing" && (e.target.id === "th_kind" || e.target.id === "th_byday" || e.target.name === "th_ok")) {
    readThing();
    if (e.target.name === "th_ok") {
      state.dialog.wantApprover = e.target.value === "person";
      state.dialog.pickApprover = e.target.value === "person";
      if (e.target.value === "self") state.dialog.thing.approver = null;
    }
    return render();
  }
  if (e.target.name === "aw_email" || e.target.id?.startsWith("aw_")) return readAway();
  if (e.target.name === "cal_share" || e.target.closest?.(".daypicks") || ["cal_from", "cal_to", "cal_mail"].includes(e.target.id)) return saveCalSettings();
  if (e.target.name === "theme") state.settings.theme = e.target.value;
  else if (e.target.id === "quiet") state.settings.quiet = e.target.value;
  else if (e.target.id === "digest") state.settings.digest = e.target.value;
  else if (e.target.id === "meetremind") state.settings.meetingRemind = Number(e.target.value);
  else if (e.target.id === "onemeetremind") {
    const conv = Number(e.target.dataset.conv);
    const minutes = e.target.value === "" ? null : Number(e.target.value);
    tauri.core.invoke("meeting_remind_set", { conv, minutes }).then(() => { state.meetRemind[conv] = minutes; render(); }).catch((err) => showToast(String(err)));
    return;
  } else if (e.target.name === "keep" && state.dialog?.kind === "keep") state.dialog.days = e.target.value === "" ? null : Number(e.target.value);
  else return;
  render();
});

/// Quit in the browser: back where the server's Files page opened Chat
/// from (it notes that), else to Files.
function filesBack() {
  try {
    const back = sessionStorage.getItem("xenify-back") || "";
    if (back.startsWith(`${location.origin}/files/`)) return back;
  } catch { /* no storage here */ }
  return "/files/";
}

/// Quit on the server's site: back where Files opened Chat from (as it was), else Files.
function backToFiles() {
  let len = 0;
  try { len = +(sessionStorage.getItem("xenify-back-len") || 0); } catch { /* no storage here */ }
  if (len && history.length === len + 1 && filesBack() !== "/files/") {
    history.back();
    setTimeout(() => location.assign(filesBack()), 600);
  } else location.assign(filesBack());
}

document.addEventListener("keydown", (e) => {
  // Ctrl+W closes the window (Chat stays in the tray), Ctrl+Q quits. In a
  // browser those keys are the browser's: Alt+W and Alt+Q go back to Files.
  const leave = !e.shiftKey && { KeyW: "close", KeyQ: "quit" }[e.code];
  if (leave && tauri && !window.XENIFY_WEB && (e.ctrlKey || e.metaKey) && !e.altKey) {
    e.preventDefault();
    tauri.core.invoke(leave === "close" ? "close_window" : "quit").catch(() => {});
    return;
  }
  if (leave && onServerPage && e.altKey && !e.ctrlKey && !e.metaKey) {
    e.preventDefault();
    backToFiles();
    return;
  }
  // Ctrl+Enter sends (and saves an edit); plain Enter is a new line.
  if (e.target.closest?.(".compose") && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    sendMail();
  } else if (e.target.id === "draft" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    send();
  } else if (e.target.id === "tdraft" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    sendReply();
  } else if (e.target.id === "edit" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    saveEdit();
  } else if (e.target.id === "pickq" && state.dialog?.kind === "event" && (e.key === "Enter" || e.key === ",") && looksLikeEmail(state.dialog.q.replace(/,$/, ""))) {
    // An email address: invite them by email.
    e.preventDefault();
    addGuest(state.dialog);
    render();
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
  } else if (e.key === "Enter" && (state.confirm || state.dialog || state.recAsk) && !e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing && !e.defaultPrevented
    && !e.target.closest?.("textarea, select, button, a[href], [contenteditable]:not([contenteditable='false'])")
    && !(e.target.form && e.target.matches?.("input:not([type=checkbox]):not([type=radio])"))) {
    // Enter in a dialog is its main button (a form's text boxes send it themselves); with none, Close or the only button.
    // The one Escape would close: a question first, then a dialog.
    const box = state.confirm ? document.querySelector('[data-act="doconfirm"]')?.closest(".dialog")
      : state.dialog ? [...document.querySelectorAll(".scrim .dialog")].pop() : document.querySelector(".recask");
    const btns = box ? [...box.querySelectorAll(":scope > .buttons button")] : [];
    const last = btns[btns.length - 1];
    const main = btns.filter((b) => b.classList.contains("primary") || b.classList.contains("danger")).pop()
      || (last?.dataset.act === "closedialog" || (btns.length === 1 && !/^cancel$/i.test(last.textContent.trim())) ? last : null);
    if (main && !main.disabled) {
      e.preventDefault();
      main.click();
    }
  } else if (e.key === "Escape") {
    if (state.bigPicture != null) state.bigPicture = null;
    else if (state.editing) (state.editing = null), (state.editText = null);
    else if (state.confirm) state.confirm = null;
    else if (state.dialog) state.dialog = null;
    else if (state.recAsk) state.recAsk = null;
    else if (state.ctx || state.menu || state.picker) (state.ctx = null), (state.menu = null), (state.picker = null);
    else if (state.find) state.find = null;
    else if (state.thread) state.thread = null;
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

// ---------- Voice messages ----------

/** Speech is turned into text here (the desktop app; not in the browser). */
function canMakeText() {
  return !!tauri && !window.XENIFY_WEB && state.voice.available !== false;
}

/** Test sound to "hear" instead of the microphone (XENIFY_VOICE_TEST_WAV). */
let voiceTest = null;

const MODEL_NAMES = { quick: "Quick", better: "Better", best: "Best, for fast computers" };

function loadVoice() {
  if (!canMakeText()) return Promise.resolve();
  return tauri.core.invoke("voice_state").then((v) => { state.voice = v; render(); }).catch(() => {});
}

const haveModel = (id = state.settings.voiceModel) => !!state.voice.models.find((m) => m.id === id)?.have;

function getModel(id) {
  if (state.voice.downloading) return;
  const m = state.voice.models.find((x) => x.id === id);
  state.voice.downloading = [id, 0, m?.bytes || 0];
  tauri.core.invoke("voice_download", { id });
}

function onVoiceDownload(p) {
  if (p.finished == null) {
    state.voice.downloading = [p.id, p.done, p.of];
  } else {
    state.voice.downloading = null;
    if (p.finished && p.finished !== "Stopped.") showToast(p.finished);
    loadVoice().then(() => {
      if (!haveModel()) return;
      // Text that waited for the model.
      if (state.rec) state.rec.r.noModel = false;
      for (const [id, t] of voiceTexts) if (t.waiting) { voiceTexts.delete(id); const f = waitingFiles.get(id); if (f) makeVoiceText(f, true); }
    });
  }
  if (state.view === "settings" || state.rec) render();
}

/** The text of what's said, with the model chosen in Settings (null: no model). */
function speechToText(audio) {
  if (!canMakeText()) return Promise.resolve(null);
  return tauri.core.invoke("voice_text", { id: state.settings.voiceModel, audio, language: state.settings.voiceLang }).catch((e) => {
    if (String(e).includes("no_model")) return null;
    throw e;
  });
}

async function startRecording(convId, where) {
  if (state.rec) return;
  if (state.call) return showToast("You're in a call.");
  const r = new Recorder({
    changed: () => redrawRec(),
    text: speechToText,
    error: (t) => showToast(t),
    testAudio: voiceTest,
  });
  state.rec = { conv: convId, r, where, sendText: state.settings.voiceSendText };
  render();
  try {
    await r.start();
    // The first time: the model comes now, and the text catches up once it's here.
    if (canMakeText()) {
      await loadVoice();
      if (!haveModel()) getModel(state.settings.voiceModel);
    }
  } catch (e) {
    state.rec = null;
    showToast(`The microphone didn't start: ${e?.message || e}`);
    render();
  }
}

function cancelRecording() {
  state.rec?.r.cancel();
  state.rec = null;
}

async function sendVoice() {
  const rec = state.rec;
  if (!rec || rec.sending) return;
  rec.sending = true;
  redrawRec();
  const out = await rec.r.finish();
  state.rec = null;
  const c = conv(rec.conv);
  if (!c || out.secs < 1) {
    if (out.secs < 1) showToast("That was too short to send.");
    return render();
  }
  if (rec.where === "call") {
    state.noAnswer = null;
    open(c.id);
  }
  const file = new File([out.wav], "Voice message.wav", { type: "audio/wav" });
  sendFiles(c, [file], { voice: { secs: out.secs, wave: out.wave, text: rec.sendText ? out.text : "" } });
}

/** Only the recording bar is drawn again while recording (it changes often). */
function redrawRec() {
  const el = document.getElementById("recbar");
  if (el) el.outerHTML = recBar();
  else render();
}

function recBar() {
  const rec = state.rec;
  if (!rec) return "";
  const r = rec.r;
  const levels = r.levels.slice(-48);
  const bars = levels.map((l) => `<span style="height:${Math.max(4, Math.round(l * 26))}px"></span>`).join("") + `<span style="height:4px"></span>`.repeat(Math.max(0, 48 - levels.length));
  const dl = state.voice.downloading;
  const said = r.text
    ? `<p class="rectext">${esc(r.text)}</p>`
    : !canMakeText() ? ""
    : r.noModel && dl ? `<p class="rectext hintline">Getting the speech model for the text (${Math.round((dl[1] / (dl[2] || 1)) * 100)}%). It's only needed once.</p>`
    : r.noModel ? `<p class="rectext hintline">No text: the speech model isn't here. <button class="linkbtn" data-act="voicesettings">Get it</button></p>`
    : `<p class="rectext hintline">${r.secs < 3 ? "Start talking…" : "Listening…"}</p>`;
  const where = rec.where === "call" ? ` for ${esc(state.noAnswer?.name || "")}` : "";
  return `<div class="recbar" id="recbar" role="group" aria-label="Recording a voice message">
    <div class="recline"><span class="recdot${r.paused ? " paused" : ""}" aria-hidden="true"></span><b class="rectime" aria-live="off">${rec.sending ? (canMakeText() && !r.noModel ? "Finishing the text…" : "Sending…") : `${r.paused ? "Paused" : "Recording"}${where} ${clock(r.secs)}`}</b><span class="recwave" aria-hidden="true">${bars}</span></div>
    ${said}
    <div class="recbuttons">
      <button class="textbtn" data-act="voicecancel" ${rec.sending ? "disabled" : ""}>${icons.trash}Throw away</button>
      <button class="textbtn" data-act="voicepause" ${rec.sending ? "disabled" : ""}>${r.paused ? icons.mic + "Go on" : icons.pause + "Pause"}</button>
      <span style="flex:1"></span>
      ${canMakeText() ? `<label class="check"><input type="checkbox" id="vsendtext" ${rec.sendText ? "checked" : ""}>Send the text too</label>` : ""}
      <button class="send" data-act="voicesend" ${rec.sending ? "disabled" : ""}>Send ${icons.send}</button>
    </div>
  </div>`;
}

function noAnswerSheet() {
  const n = state.noAnswer;
  const c = conv(n.conv);
  const other = c?.members.find((u) => u.id !== state.me.id);
  const [bg, fg] = colours(other ? other.id + nameOf(other) : n.name);
  const recording = state.rec?.where === "call";
  return `<main class="callsheet noanswer" aria-label="No answer">
    <header class="callhead"><button class="callctl small" data-act="noanswerclose">${icons.back}Back to chats</button><b>${esc(n.name)}</b></header>
    <div class="noanswerbody">
      <span class="callface big" style="background:${bg};color:${fg}">${esc(initials(n.name))}</span>
      <h2>${esc(n.name)} ${n.declined ? "can't talk now" : "didn't answer"}</h2>
      <p>You can leave a voice message.${canMakeText() ? ` ${esc(n.name)} will see what you said as text too.` : ""}</p>
      ${recording ? `<div class="noanswerrec">${recBar()}</div>` : `<div class="noanswerbuttons">
        <button class="callctl primary" data-act="leavevoice">${icons.mic}Leave a voice message</button>
        <button class="callctl" data-act="callagain">${icons.phone}Call again</button>
        <button class="callctl" data-act="noanswerclose">Close</button>
      </div>`}
    </div>
  </main>`;
}

/** Received voice messages' text, made here: file id → { text } | { busy } | { waiting }. */
const voiceTexts = new Map();
const waitingFiles = new Map();
try {
  for (const [id, text] of Object.entries(JSON.parse(localStorage.getItem("voiceTexts") || "{}"))) voiceTexts.set(id, { text });
} catch {}

function keepVoiceTexts() {
  try {
    const all = {};
    for (const [id, t] of voiceTexts) if (t.text != null) all[id] = t.text;
    localStorage.setItem("voiceTexts", JSON.stringify(all));
  } catch {}
}

function fileBlob(f) {
  return window.XENIFY_FILE_BLOB ? window.XENIFY_FILE_BLOB(f) : fetch(fileUrl(f)).then((r) => (r.ok ? r.blob() : Promise.reject(new Error(r.status))));
}

/** Makes a received voice message's text (`asked`: someone clicked for it). */
async function makeVoiceText(f, asked = false) {
  if (voiceTexts.has(f.id) || !canMakeText()) return;
  if (!haveModel()) {
    if (!asked) return;
    voiceTexts.set(f.id, { waiting: true });
    waitingFiles.set(f.id, f);
    await loadVoice();
    if (!haveModel()) getModel(state.settings.voiceModel);
    else voiceTexts.delete(f.id), makeVoiceText(f, true);
    return render();
  }
  voiceTexts.set(f.id, { busy: true });
  render();
  try {
    const audio = wavSamples(await (await fileBlob(f)).arrayBuffer());
    const text = await speechToText(audio);
    voiceTexts.set(f.id, { text: text || "" });
    keepVoiceTexts();
  } catch (e) {
    voiceTexts.delete(f.id);
    showToast(`The text wasn't made: ${e?.message || e}`);
  }
  render();
}

/** Voice messages on screen: their players (file id → { audio, frac, speed }). */
const voicePlayers = new Map();

async function playVoice(msgId) {
  const [, m] = findMsg(msgId);
  const f = m && fileOf(m.text);
  if (!f) return;
  let p = voicePlayers.get(f.id);
  if (!p) {
    p = { audio: null, frac: 0, speed: 1, loading: true };
    voicePlayers.set(f.id, p);
    render();
    try {
      p.audio = new Audio(URL.createObjectURL(await fileBlob(f)));
    } catch (e) {
      voicePlayers.delete(f.id);
      showToast(`This voice message didn't load: ${e?.message || e}`);
      return render();
    }
    p.loading = false;
    p.audio.playbackRate = p.speed;
    p.audio.ontimeupdate = () => {
      p.frac = p.audio.duration ? p.audio.currentTime / p.audio.duration : 0;
      const el = document.querySelector(`[data-voicewave="${CSS.escape(f.id)}"]`);
      if (el) [...el.children].forEach((b, i) => b.classList.toggle("played", i < p.frac * BARS));
      const t = document.querySelector(`[data-voicetime="${CSS.escape(f.id)}"]`);
      if (t) t.textContent = clock(p.audio.currentTime);
    };
    p.audio.onended = () => {
      p.frac = 0;
      render();
    };
  }
  if (!p.audio) return;
  // One at a time.
  for (const [id, o] of voicePlayers) if (id !== f.id && o.audio && !o.audio.paused) o.audio.pause();
  if (p.audio.paused) await p.audio.play().catch((e) => showToast(`It didn't play: ${e?.message || e}`));
  else p.audio.pause();
  render();
}

function voiceSpeed(msgId) {
  const [, m] = findMsg(msgId);
  const f = m && fileOf(m.text);
  if (!f) return;
  let p = voicePlayers.get(f.id);
  if (!p) voicePlayers.set(f.id, (p = { audio: null, frac: 0, speed: 1 }));
  p.speed = p.speed === 1 ? 1.5 : p.speed === 1.5 ? 2 : 1;
  if (p.audio) p.audio.playbackRate = p.speed;
  render();
}

function voiceCard(m, f) {
  const v = f.voice;
  const p = voicePlayers.get(f.id);
  const playing = p?.audio && !p.audio.paused;
  const played = p?.frac || 0;
  const mine = m.sender.id === state.me?.id;
  const wave = (v.wave || []).map((h, i) => `<span class="${i < played * BARS ? "played" : ""}" style="height:${6 + h * 2.6}px"></span>`).join("");
  const local = voiceTexts.get(f.id);
  if (!v.text && !local && canMakeText() && state.settings.voiceIncoming && haveModel() && !mine) setTimeout(() => makeVoiceText(f), 0);
  const note = (where) => `<div class="voicenote">${icons.screen}Text made on ${where}, may have mistakes</div>`;
  let said = "";
  if (v.text) said = `<p class="voicetext">${esc(v.text)}</p>${note(mine ? "your computer" : `${esc(senderName(conv(m.conv), m.sender))}'s computer`)}`;
  else if (local?.text) said = `<p class="voicetext">${esc(local.text)}</p>${note("this computer")}`;
  else if (local?.busy) said = `<div class="voicenote">Making the text…</div>`;
  else if (local?.waiting) said = `<div class="voicenote">Getting the speech model for the text…</div>`;
  else if (local && local.text === "") said = `<div class="voicenote">No words heard.</div>`;
  else if (canMakeText() && !mine) said = `<button class="linkbtn voicenote" data-act="voicetext" data-id="${m.id}">Read it as text</button>`;
  return `<div class="voice${mine ? " mine" : ""}">
    <div class="voicerow">
      <button class="voiceplay" data-act="voiceplay" data-id="${m.id}" aria-label="${playing ? "Pause" : "Play"} voice message, ${v.secs} seconds">${p?.loading ? "…" : playing ? icons.pause : icons.play}</button>
      <span class="voicewave" data-voicewave="${esc(f.id)}" aria-hidden="true">${wave}</span>
      <span class="voicetime" data-voicetime="${esc(f.id)}">${clock(p?.audio && played ? p.audio.currentTime : v.secs)}</span>
      <button class="voicespeed" data-act="voicespeed" data-id="${m.id}" aria-label="Play speed ${p?.speed || 1} times">${p?.speed || 1}×</button>
    </div>
    ${said}
  </div>`;
}

function voiceSettings() {
  const v = state.voice;
  const id = state.settings.voiceModel;
  const m = v.models.find((x) => x.id === id);
  const mb = (n) => `${Math.round(n / 1e6)} MB`;
  const dl = v.downloading;
  const status = dl
    ? `<div class="vprogress"><span class="upbar"><span style="width:${Math.round((dl[1] / (dl[2] || 1)) * 100)}%"></span></span><span class="hintline">Downloading ${MODEL_NAMES[dl[0]] || ""} · ${mb(dl[1])} of ${mb(dl[2])}</span><button class="small" data-act="voicestopget">Stop</button></div>`
    : m?.have
      ? `<div class="vprogress"><span class="hintline">Downloaded · ${mb(m.bytes)}</span><button class="small" data-act="voiceremove">Remove it</button></div>`
      : `<div class="vprogress"><span class="hintline">Not downloaded yet. It downloads by itself the first time you record or read a voice message.</span><button class="small primary" data-act="voiceget">Download now${m ? ` (${mb(m.bytes)})` : ""}</button></div>`;
  const langs = [["", "Work it out each time"], ["en", "English"], ["de", "Deutsch"], ["es", "Español"], ["fr", "Français"], ["it", "Italiano"], ["nl", "Nederlands"], ["pt", "Português"], ["pl", "Polski"], ["sv", "Svenska"], ["ja", "日本語"], ["zh", "中文"]];
  return `<h1>Voice and transcripts</h1><p class="lede">Speech is turned into text on this computer. Nothing you say is sent anywhere to be turned into text.</p>
    <div class="panel vmodel">
      <div class="setting" style="border:none;padding:0"><div><label class="label" for="vmodel">Speech-to-text model</label><div class="hintline">Needed once, for voice messages and meeting transcripts. Works in English and most other languages.</div></div>
        <select id="vmodel">${v.models.map((x) => `<option value="${x.id}" ${x.id === id ? "selected" : ""}>${MODEL_NAMES[x.id]}: ${mb(x.bytes)}${x.have ? " (downloaded)" : ""}</option>`).join("")}</select></div>
      ${status}
      <div class="hintline">Whisper, by OpenAI, run with whisper.cpp. MIT licence. From ${esc((v.source || "").replace(/^https:\/\//, "").replace(/\/resolve\/main\/$/, ""))}.</div>
    </div>
    <h2>Voice messages</h2>
    ${toggle("voiceIncoming", "Show the text of voice messages I get", "Made here when one arrives, so you can read it instead of listening")}
    ${toggle("voiceSendText", "Send the text with my voice messages", "Saves the other person's computer the work. You can untick it for one message")}
    <div class="setting"><div><label class="label" for="vlang">Language</label><div class="hintline">What people usually speak in your voice messages and meetings</div></div>
      <select id="vlang">${langs.map(([code, name]) => `<option value="${code}" ${state.settings.voiceLang === code ? "selected" : ""}>${name}</option>`).join("")}</select></div>
    <h2>Meeting recordings</h2>
    ${toggle("meetingTranscript", "Make a transcript when I record a meeting", "Each person's words under their name. Long meetings take a while on a slow computer")}`;
}

// ---------- Recording calls ----------

// Someone records: everyone's app turns everything off, and each person is
// only in the recording once they turn something back on (asked first). The
// recording is made on the recorder's computer from what it already gets.

/** The server says how the call's recording stands. */
function recordingChanged(call, info) {
  const r = info.recording;
  if (r && state.recRounds[info.id] !== r.round) {
    state.recRounds[info.id] = r.round;
    call.allOff().then(() => tell({ t: "call", conv: info.conv, action: "recording_off", video: false }));
  }
  if (r && r.by.id === state.me.id && state.recording?.round !== r.round) {
    if (state.recording) finishRecording();
    state.recording = { round: r.round, conv: info.conv, callId: info.id, rec: null, lines: [], queue: Promise.resolve(), pieces: 0, done: 0, everIn: new Set(), transcript: !!state.recWantText, startBy: Date.now() + 5000 };
    if (state.recording.transcript && !haveModel()) loadVoice().then(() => { if (!haveModel()) getModel(state.settings.voiceModel); });
    setTimeout(startRecordingWhenQuiet, 5100);
  }
  if (!r) {
    state.recAsk = null;
    if (state.recording?.callId === info.id) finishRecording();
  }
  startRecordingWhenQuiet();
}

/** The recording starts once every app in the call has turned everything
 *  off (or after 5 s: an app that doesn't answer is simply not in it). */
function startRecordingWhenQuiet() {
  const rc = state.recording;
  const call = state.call;
  if (!rc || rc.rec || !call || call.info.id !== rc.callId) return;
  const r = call.info.recording;
  if (!r) return;
  const quiet = call.info.people.every((u) => r.off.includes(u.id));
  if (!quiet && Date.now() < rc.startBy) return;
  const names = new Map(call.info.people.map((u) => [u.id, nameOf(u)]));
  rc.rec = new Recording(call, {
    included: () => {
      const now = state.call?.info.recording;
      const here = new Set((state.call?.info.people || []).map((u) => u.id));
      const ids = new Set((now?.in || []).filter((id) => here.has(id)));
      for (const id of ids) rc.everIn.add(id);
      return ids;
    },
    name: (id) => names.get(id) || nameOf(state.call?.info.people.find((u) => u.id === id) || { username: "?" }),
    piece: (id, atMs, samples) => textPiece(rc, id, atMs, samples),
    error: (t) => showToast(t),
  });
  for (const u of call.info.people) names.set(u.id, nameOf(u));
  rc.rec.start();
  rc.startedAt = Date.now();
  render();
}

/** One person's piece of the call, turned into text in the background. */
function textPiece(rc, id, atMs, samples) {
  if (!rc.transcript || !canMakeText()) return;
  const who = rc.rec?.hooks.name(id) || "";
  rc.pieces++;
  rc.queue = rc.queue.then(async () => {
    const audio = toBase64(samples);
    const ask = () => tauri.core.invoke("voice_pieces", { id: state.settings.voiceModel, audio, language: state.settings.voiceLang });
    let out = await ask().catch((e) => (String(e).includes("no_model") ? null : []));
    // The model's still coming: wait for it.
    while (out === null && state.voice.downloading) {
      await new Promise((ok) => setTimeout(ok, 1000));
      if (haveModel()) out = await ask().catch(() => []);
    }
    for (const p of out || []) rc.lines.push({ at: atMs + p.from_ms, who, text: p.text });
    rc.done++;
    if (state.recSave?.rc === rc) {
      state.recSave.done = rc.done;
      render();
    }
  });
}

/** Stopped (or the call ended): the video, then the transcript, into the call's chat. */
async function finishRecording() {
  const rc = state.recording;
  state.recording = null;
  if (!rc?.rec) return render();
  if (!rc.everIn.size) {
    rc.rec.cancel();
    showToast("Nobody turned their microphone or camera on, so nothing was recorded.");
    return render();
  }
  state.recSave = { rc, step: "saving", done: 0, of: 0 };
  render();
  try {
    const out = await rc.rec.finish();
    const c = conv(rc.conv);
    if (!c) throw new Error("its chat isn't here any more");
    const names = [...rc.everIn].map((id) => rc.rec.hooks.name(id));
    let transcript = null;
    if (rc.transcript) {
      Object.assign(state.recSave, { step: "text", done: rc.done, of: rc.pieces });
      render();
      await rc.queue;
      rc.lines.sort((a, b) => a.at - b.at);
      const json = new TextEncoder().encode(JSON.stringify({ lines: rc.lines }));
      transcript = await uploadFile(c, new File([json], "Transcript.json", { type: "application/json" }));
    }
    state.recSave.step = "sending";
    render();
    // (A call between two people is just "Call recording": the other's
    // name would be wrong on their side.)
    const title = c.purpose === "meeting" ? `${meetingOf(c.id)?.title || "Meeting"} recording` : c.kind === "direct" ? "Call recording" : `${titleOf(c)} recording`;
    const file = new File([out.webm], `${title}.webm`, { type: "video/webm" });
    await sendFiles(c, [file], { recording: { secs: out.secs, people: names, transcript, at: rc.startedAt } });
    showToast(c.purpose === "meeting" ? "The recording is in the meeting (open it from your calendar)." : `The recording is in your chat ${c.kind === "direct" ? "with" : "in"} ${titleOf(c)}.`);
  } catch (e) {
    showToast(`The recording wasn't saved: ${e?.message || e}`);
  }
  state.recSave = null;
  render();
}

/** Turning something on while someone records and you're not in it: ask. */
function askFirst(what) {
  const r = state.call?.info.recording;
  if (!r || r.in?.includes(state.me.id)) return false;
  state.recAsk = what;
  return true;
}

function recBanner(info) {
  const r = info.recording;
  if (!r) return "";
  const mine = r.by.id === state.me.id;
  const rc = state.recording;
  const since = mine && rc?.startedAt ? rc.startedAt : r.since;
  const time = clock((Date.now() - since) / 1000);
  const inIt = r.in?.includes(state.me.id);
  const count = (r.in || []).length;
  const text = mine && !rc?.rec
    ? "Turning everyone's microphone and camera off…"
    : mine
      ? `${count === 0 ? "Nobody's in it yet: each person is once they turn their microphone or camera back on." : `${count} ${count === 1 ? "person is" : "people are"} in it.`}${inIt ? "" : " Turn yours on to be in it too."}`
      : inIt
        ? "You're in the recording."
        : "Your microphone and camera were turned off. You're not in the recording unless you turn one back on.";
  return `<div class="recbanner" role="status"><span class="recbannerdot" aria-hidden="true"></span><b>${mine ? "You're recording" : `${esc(nameOf(r.by))} is recording`} · ${time}</b><span>${text}</span></div>`;
}

function recAskBox(info) {
  const r = info.recording;
  const what = state.recAsk;
  if (!r || !what) return "";
  const mine = r.by.id === state.me.id;
  const title = { mic: "Turn your microphone on?", cam: "Turn your camera on?", screen: "Share your screen?" }[what];
  const thing = { mic: "what you say is", cam: "your camera is", screen: "your screen is" }[what];
  const transcript = what === "mic" ? " and its transcript" : "";
  return `<div class="recask" role="dialog" aria-label="${title}">
    <b>${title}</b>
    <p>${mine ? "You're recording this call." : `${esc(nameOf(r.by))} is recording this call.`} Once it's on, ${thing} in the recording${transcript}, which everyone in the ${conv(info.conv)?.purpose === "meeting" ? "meeting" : "chat"} can open.</p>
    <div class="buttons"><button class="small" data-act="recaskno">Keep it off</button><button class="small primary" data-act="recaskyes">Turn on and be recorded</button></div>
  </div>`;
}

function recordDialog(d) {
  const c = state.call && conv(state.call.info.conv);
  const what = c?.purpose === "meeting" ? "meeting" : "call";
  return `<div class="scrim over"><div class="dialog wide" role="dialog" aria-labelledby="rectitle">
    <h2 id="rectitle">Record this ${what}?</h2>
    <p>Every microphone, camera and shared screen in the call will be turned off, yours too. Recording starts once they're all off.</p>
    <p>Each person is in the recording only after they turn theirs back on. Anyone who doesn't can still listen and write in the chat. People who join later start with theirs off.</p>
    ${canMakeText() ? `<label class="checkline"><input type="checkbox" id="rectext" ${d.transcript ? "checked" : ""}>Make a transcript too</label>` : ""}
    <p class="hintline">Recorded${canMakeText() ? " and transcribed" : ""} on this computer, then put in the ${what}'s chat.</p>
    <div class="buttons"><button class="small" data-act="closedialog">Cancel</button><button class="small danger" data-act="recstart">Start recording</button></div>
  </div></div>`;
}

function recSaveBar() {
  const s = state.recSave;
  if (!s) return "";
  const text = s.step === "text" ? `Making the transcript · ${Math.min(s.done, s.of)} of ${s.of}` : s.step === "sending" ? "Sending the recording…" : "Saving the recording…";
  return `<div class="recsave" role="status"><span class="recbannerdot" aria-hidden="true"></span>${text}</div>`;
}

/** Recordings' transcripts: file id → { lines } | { loading } | { error }. */
const transcripts = new Map();

function loadTranscript(info) {
  if (!info || transcripts.has(info.id)) return;
  transcripts.set(info.id, { loading: true });
  fileBlob(info)
    .then((b) => b.text())
    .then((t) => transcripts.set(info.id, { lines: JSON.parse(t).lines || [] }))
    .catch((e) => transcripts.set(info.id, { error: String(e?.message || e) }))
    .finally(() => render());
}

function transcriptText(f) {
  const t = transcripts.get(f.recording.transcript?.id);
  return (t?.lines || []).map((l) => `[${clock(l.at / 1000)}] ${l.who}: ${l.text}`).join("\n");
}

async function saveTranscript(f) {
  const info = f.recording.transcript;
  if (!info) return;
  loadTranscript(info);
  for (let i = 0; i < 50 && transcripts.get(info.id)?.loading; i++) await new Promise((ok) => setTimeout(ok, 100));
  const name = f.name.replace(/\.webm$/, "") + " transcript.txt";
  tauri.core.invoke("save_download", new TextEncoder().encode(transcriptText(f)).buffer, { headers: { name: encodeURIComponent(name) } })
    .then((p) => showToast(`Saved in Downloads: ${p.split(/[\\/]/).pop()}`), (e) => showToast(String(e)));
}

function transcriptLines(lines) {
  return lines.map((l) => `<div class="tline"><span class="tat">${clock(l.at / 1000)}</span><b>${esc(l.who)}</b><span>${esc(l.text)}</span></div>`).join("");
}

function recordingCard(m, f) {
  const r = f.recording;
  const mine = m.sender.id === state.me?.id;
  const t = r.transcript ? transcripts.get(r.transcript.id) : null;
  if (r.transcript && !t) setTimeout(() => loadTranscript(r.transcript), 0);
  const people = r.people?.length ? `In it: ${r.people.map(esc).join(", ")}` : "Nobody turned anything on";
  const video = f.size > PLAY_LIMIT ? `<div class="cliptoobig">Too big to play here: save it to watch.</div>` : `<video controls preload="metadata" data-keep="${esc(f.id)}" data-file="${m.id}" ${clips.get(f.id) ? `src="${clips.get(f.id)}"` : ""} aria-label="${esc(f.name)}"></video>`;
  const said = !r.transcript ? "" : t?.lines
    ? `<div class="tcard"><div class="thead"><b>Transcript</b><span class="hintline">Made on ${mine ? "your" : `${esc(senderName(conv(m.conv), m.sender))}'s`} computer, may have mistakes</span></div>
        ${t.lines.length ? transcriptLines(t.lines.slice(0, 3)) : `<p class="hintline">Nothing said that could be turned into text.</p>`}
        <div class="tbuttons">${t.lines.length > 3 ? `<button class="small" data-act="transcript" data-id="${m.id}">Open the whole transcript</button>` : ""}${t.lines.length ? `<button class="small" data-act="transcriptsave" data-id="${m.id}">${icons.download}Save as text</button>` : ""}</div></div>`
    : `<div class="tcard"><span class="hintline">${t?.error ? `The transcript didn't open: ${esc(t.error)}` : "Opening the transcript…"}</span></div>`;
  return `<div class="reccard">
    <div class="clip">${video}<div class="clipbar"><b>${esc(f.name.replace(/\.webm$/, ""))}</b><span class="hintline">${clock(r.secs)} · ${sizeText(f.size)}</span><button class="iconbtn" data-act="savefile" data-id="${m.id}" aria-label="Save ${esc(f.name)} in Downloads" title="Save in Downloads">${icons.download}</button></div></div>
    <div class="hintline recpeople">${people}</div>
    ${said}
  </div>`;
}

function transcriptDialog(d) {
  const [, m] = findMsg(d.msg);
  const f = m && fileOf(m.text);
  if (!f?.recording?.transcript) return "";
  const t = transcripts.get(f.recording.transcript.id);
  return `<div class="scrim"><div class="dialog transcript" role="dialog" aria-label="Transcript">
    <h2>Transcript</h2><div class="hintline">${esc(f.name.replace(/\.webm$/, ""))} · made on a computer, may have mistakes</div>
    <div class="tall">${t?.lines ? transcriptLines(t.lines) : `<span class="hintline">Opening…</span>`}</div>
    <div class="buttons"><button class="small" data-act="transcriptsave" data-id="${m.id}">${icons.download}Save as text</button><button class="small primary" data-act="closedialog">Close</button></div>
  </div></div>`;
}

/** A meeting's recordings, in its window. */
function meetingRecordings(convId) {
  const list = (state.messages[convId] || []).filter((x) => fileOf(x.text)?.recording);
  if (!list.length) return "";
  return `<b class="whohead">Recordings</b>${list.map((x) => recordingCard(x, fileOf(x.text))).join("")}`;
}

// The time in the recording banner.
setInterval(() => {
  if (state.call?.info.recording && !state.call.away) render();
}, 1000);
