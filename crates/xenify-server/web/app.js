// The Xenify server's admin pages. One page, drawn from `state` by render()
// (like Write, Draw and Chat); clicks and typing go through data-act and
// data-bind.

import { planInit, planPage, loadPlan, planActs } from "./plan.js";
import { dataInit, dataPage, loadData, dataActs } from "./data.js";
import { rolesInit, rolesPage, loadRoles, rolesActs } from "./roles.js";
import { backupsInit, backupsPage, loadBackups, backupsActs } from "./backups.js";
import { emailInit, emailPage, loadEmail, emailActs } from "./email.js";

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const initials = (s) => String(s || "?").split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "?";
const palette = [["#FDE6D8", "#8A2C0A"], ["#DCE9FB", "#1D4F91"], ["#DDF3E4", "#1E6B3A"], ["#EDE4FA", "#5B3A91"], ["#FCE7F0", "#8E2557"], ["#E2F0F3", "#1D5C6B"]];
const colours = (key) => palette[[...String(key)].reduce((a, c) => a + c.charCodeAt(0), 0) % palette.length];
// Links and codes may break onto a new line only between parts.
const codeHtml = (c) => esc(c).replaceAll("-", "-<wbr>").replaceAll("/", "/<wbr>").replaceAll("#", "#<wbr>").replaceAll(".", ".<wbr>");
const nameOf = (p) => p.display_name || p.email || p.username;

// The email an admin sends from their own mail program with a person's
// code (when the server can't send email itself).
function codeMail(p, code, why) {
  const server = state.info?.name || location.host;
  const first = why === "join" ? `You've been invited to ${server} in Xenify.` : `Your password for ${server} in Xenify was reset.`;
  return {
    to: p.email || "",
    subject: why === "join" ? `You're invited to ${server}` : `New password for ${server}`,
    body: `Hi ${nameOf(p)},\n\n${first}\n\nSign in at ${location.origin}/files/ with your email address, ${p.email}. When asked, type this code, then pick your password:\n\n${code}\n\nThe Xenify apps sign in the same way. The code works once, for 14 days.\n`,
  };
}

// When the server couldn't email someone: the code for the admin to pass on.
function codeDialog(p, made, why) {
  const as = p.email || p.username;
  return {
    kind: "code",
    title: `Give ${nameOf(p)} this code`,
    text: `${made.mail_error ? `The email didn't go (${esc(made.mail_error)}), so tell them yourself` : mailOn() ? "They have no email address here, so tell them yourself" : `This server can't send email yet (<button class="link" type="button" data-act="to-mail">set it up</button>), so tell them yourself`}: sign in at <b>${esc(location.origin)}</b> as <b>${esc(as)}</b>, type this code when asked, then pick ${why === "join" ? "a" : "a new"} password. It works once, for 14 days.`,
    code: made.code,
    mail: codeMail(p, made.code, why),
  };
}

// Has the server email the person signing in a code, and asks for it.
async function emailCode(note) {
  const f = state.form;
  await api("POST", "/v1/forgot", { username: (f.username || "").trim().toLowerCase() });
  Object.assign(f, { stage: "code", note, code: "", emailed: true });
}

const ROLES = { viewer: "Can view", commenter: "Can comment", editor: "Can edit", manager: "Can manage" };
const KINDS = [
  ["text", "Writing and questions"],
  ["data", "Working with data"],
  ["vision", "Looking at pictures"],
  ["image", "Making pictures"],
  ["video", "Making video"],
  ["lora", "Picture styles (LoRA)"],
];

const state = {
  view: "loading",
  info: null,
  session: null, // { token, account }
  error: "",
  busy: false,
  toast: "",
  menu: null, // a ⋯ menu open: its id
  dialog: null,
  form: {},
  setup: { token: "" },
  people: [],
  groups: [],
  tab: "people",
  find: "",
  spaces: [],
  spaceSel: null,
  access: null,
  names: {}, // space id -> name shown
  ai: null,
  aiStatus: null,
  aiFind: { source: "hf", kind: "text", q: "", hits: null, files: null, open: null },
  // Making a picture style: { name, base, lora, strength, parts: { bases, loras } }.
  style: null,
  jobs: [],
  adminAccess: "lan",
};

// ---- Talking to the server --------------------------------------------------

async function api(method, path, body) {
  const headers = { "Content-Type": "application/json" };
  if (state.session?.token) headers.Authorization = `Bearer ${state.session.token}`;
  const r = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await r.text();
  const data = text ? (() => { try { return JSON.parse(text); } catch { return null; } })() : null;
  if (r.status === 401 && state.session && path !== "/v1/login") {
    signOut(false);
    throw new Error("Please sign in again.");
  }
  if (!r.ok) {
    const e = new Error(sentence(data?.error || `The server said no (${r.status}).`));
    e.code = data?.code;
    throw e;
  }
  return data;
}

const sentence = (s) => (s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".") : s);

// One sign-in for every page of this server (Files, these pages and the
// apps): signing in or out here does it there too.
const SHARED = "xenify-session";

function keep() {
  try {
    if (state.session) localStorage.setItem(SHARED, JSON.stringify(state.session));
    else localStorage.removeItem(SHARED);
  } catch {}
}

function sharedSession() {
  try {
    const s = JSON.parse(localStorage.getItem(SHARED) || "null");
    return s?.token && s.account ? { token: s.token, account: s.account } : null;
  } catch {
    return null;
  }
}

// Signed in or out on another page of this server.
window.addEventListener("storage", (e) => {
  if (e.key === SHARED && sharedSession()?.token !== state.session?.token) location.reload();
});

function signOut(tell = true) {
  if (tell && state.session) api("POST", "/v1/signout").catch(() => {});
  state.session = null;
  keep();
  go("signin");
}

function openSession(s) {
  state.session = { token: s.token, account: s.account };
  keep();
}

// "Remember this computer" after a second step: kept in this browser.
function remembered() {
  try {
    return localStorage.getItem("xenify-admin-remember") || "";
  } catch {
    return "";
  }
}

// Which page needs which part of running the server (Roles › Advanced);
// admins open them all, Spaces only admins.
const PAGE_POWER = { people: "admin.people", roles: "admin.roles", links: "admin.links", ai: "admin.ai", data: "admin.data", holidays: "admin.holidays", updates: "admin.updates", backups: "admin.backups", email: "admin.network", network: "admin.network" };

/// The parts of running the server `token`'s person has ([] for most).
async function powersOf(token) {
  try {
    const r = await fetch("/v1/me/perms", { headers: { Authorization: `Bearer ${token}` } });
    return r.ok ? (await r.json()).powers || [] : [];
  } catch {
    return [];
  }
}

function isAdmin() {
  return !!state.session?.account?.admin;
}

function mayOpen(view) {
  if (view === "account" || isAdmin()) return true;
  return !!PAGE_POWER[view] && (state.powers || []).includes(PAGE_POWER[view]);
}

/// The first page this person may open.
function firstPage() {
  return ["people", "roles", "spaces", "links", "ai", "data", "holidays", "updates", "backups", "email", "network"].find(mayOpen) || "account";
}

async function signedInAs(s) {
  state.powers = s.account.admin ? [] : await powersOf(s.token);
  if (!s.account.admin && !state.powers.length) {
    // Not an admin: signed in all the same, to their files.
    openSession(s);
    return void location.assign("/files/");
  }
  if (s.remember) {
    try {
      localStorage.setItem("xenify-admin-remember", s.remember);
    } catch {}
  }
  state.mfa = null;
  openSession(s);
  state.form = {};
  go(fromHash() || firstPage());
}

/// Two-step sign-in: the code while signing in, or setting it up (while
/// signing in, with a ticket, or from Your account).
function mfaHtml() {
  const m = state.mfa;
  const remember = m.ticket ? `<label class="check"><input type="checkbox" data-act="mfa-remember" ${state.form.remember ? "checked" : ""}>Remember this computer for 30 days</label>` : "";
  const cancel = `<button class="btn" type="button" data-act="mfa-cancel">${m.ticket ? "Back to sign in" : "Cancel"}</button>`;
  if (m.stage === "code") {
    const what = { app: "Code from your authenticator app", email: "Code from the email we sent", backup: "Backup code" }[m.method];
    const others = m.methods.filter((x) => x !== m.method).map((x) => `<button class="link" type="button" data-act="mfa-method" data-id="${x}">${{ app: "Use the authenticator app", email: "Email me a code", backup: "Use a backup code" }[x]}</button>`).join("");
    return `<form class="stack" data-act="mfa-code"><div><h1>Two-step sign-in</h1><p class="dim">Your password was right. Now the second step.</p></div>${errorBox()}
      ${field(what, "form.mfaCode", { auto: "one-time-code", placeholder: m.method === "backup" ? "xxxx-xxxx" : "123456" })}${remember}
      <div class="row">${cancel}<span class="grow"></span>${others}<button class="btn main" type="submit">Sign in</button></div></form>`;
  }
  if (m.stage === "choose") {
    return `<div class="stack"><div><h1>Set up two-step sign-in</h1><p class="dim">${m.ticket ? "This server asks for a code after your password. Pick how you'll get it." : "After your password, you'll also give a code, so a stolen password isn't enough."}</p></div>${errorBox()}
      <button class="choice" data-act="mfa-pick" data-id="app"><span><b>An authenticator app</b><span class="pill">Recommended</span><span class="why">Google Authenticator, Microsoft Authenticator, 1Password and others: a new code every 30 seconds.</span></span></button>
      ${m.setup_methods?.includes("email") ? '<button class="choice" data-act="mfa-pick" data-id="email"><span><b>A code by email</b><span class="why">Easier, but weaker: whoever gets into your email gets both steps.</span></span></button>' : ""}
      <div class="row">${cancel}</div></div>`;
  }
  if (m.stage === "app" || m.stage === "email") {
    const app = m.stage === "app";
    return `<form class="stack" data-act="mfa-finish"><div><h1>${app ? "Scan this with your authenticator app" : "Check your email"}</h1><p class="dim">${app ? "Or type the key in, then give the code it shows." : "We sent you a code. Type it here."}</p></div>${errorBox()}
      ${app ? `<div class="mfa-qr">${m.app.qr_svg}</div><div class="code link">${codeHtml(m.app.secret.replace(/(.{4})/g, "$1 ").trim())}</div>` : ""}
      ${field(app ? "Code from the app" : "Code from the email", "form.mfaCode", { auto: "one-time-code", placeholder: "123456" })}${remember}
      <div class="row">${cancel}<span class="grow"></span>${app ? "" : '<button class="link" type="button" data-act="mfa-email">Send it again</button>'}<button class="btn main" type="submit">Turn it on</button></div></form>`;
  }
  // The backup codes, shown once.
  return `<div class="stack"><div><h1>Keep these backup codes</h1><p class="dim">Each works once, when you can't get a code (a lost phone, say). Keep them somewhere safe: they won't be shown again.</p></div>
    <div class="code link backup">${m.codes.map(esc).join("<br>")}</div>
    <div class="row"><button class="btn" data-act="copy" data-text="${esc(m.codes.join("\n"))}">Copy</button><span class="grow"></span><button class="btn main" data-act="mfa-kept">I've kept them</button></div></div>`;
}

// Names of spaces: JSON text ({"name": …}), like files' details.
const nameMeta = (name) => JSON.stringify({ name });

function openName(meta) {
  try {
    return JSON.parse(meta).name || "";
  } catch {
    return "";
  }
}

// ---- Pages --------------------------------------------------------------------

function go(view) {
  if (state.session && view !== "signin" && !mayOpen(view)) view = firstPage();
  state.view = view;
  state.error = "";
  state.menu = null;
  const hash = { people: "#/people", roles: "#/roles", spaces: "#/spaces", links: "#/links", ai: "#/ai", data: "#/data", updates: "#/updates", email: "#/email", network: "#/network", account: "#/account" }[view];
  if (hash && location.hash !== hash) history.replaceState(null, "", hash);
  load();
  render();
}

async function load() {
  try {
    if (state.view === "people") {
      let set, roles;
      [state.people, state.groups, set, state.info, roles] = await Promise.all([
        api("GET", "/v1/admin/people"),
        api("GET", "/v1/admin/groups"),
        api("GET", "/v1/admin/settings"),
        api("GET", "/v1/info"),
        api("GET", "/v1/roles").catch(() => ({ roles: [] })),
      ]);
      state.serverRoles = roles.roles.filter((r) => r.kind === "server");
      state.mfaRequired = set.mfa_required;
    } else if (state.view === "spaces") {
      let roles;
      [state.spaces, state.people, state.groups, roles] = await Promise.all([api("GET", "/v1/spaces"), api("GET", "/v1/admin/people"), api("GET", "/v1/admin/groups"), api("GET", "/v1/roles")]);
      state.docRoles = roles.roles.filter((r) => r.kind === "document");
      for (const s of state.spaces) {
        state.names[s.id] = s.kind === "personal" ? "My files" : s.kind === "templates" ? "Templates" : (await openName(s.name)) || "Untitled space";
      }
      if (!state.spaceSel || !state.spaces.some((s) => s.id === state.spaceSel)) {
        state.spaceSel = state.spaces.find((s) => s.kind === "shared")?.id ?? state.spaces[0]?.id ?? null;
      }
      await loadAccess();
    } else if (state.view === "links") {
      state.publinks = await api("GET", "/v1/admin/public-links");
      for (const l of state.publinks.links) {
        l.name = (await openName(l.meta)) || (l.kind === "folder" ? "Untitled folder" : "Untitled file");
        l.place = l.space_kind === "personal" ? `${l.made_by || "Someone"}'s files` : l.space_kind === "templates" ? "Templates" : (await openName(l.space)) || "A space";
      }
    } else if (state.view === "ai") {
      state.ai = await api("GET", "/v1/admin/ai");
      state.form.ai = { ...state.ai };
      state.jobs = await api("GET", "/v1/admin/ai/jobs");
      state.mcpTools = await api("GET", "/v1/admin/mcp").catch(() => []);
      render();
      state.aiStatus = await api("GET", "/v1/admin/ai/status");
      state.comfy = await api("GET", "/v1/admin/ai/comfy").catch(() => null);
    } else if (state.view === "data") {
      await loadData();
    } else if (state.view === "holidays") {
      await loadPlan();
    } else if (state.view === "backups") {
      await loadBackups();
    } else if (state.view === "email") {
      await loadEmail();
    } else if (state.view === "roles") {
      await loadRoles();
    } else if (state.view === "updates") {
      [state.hub, state.info] = await Promise.all([api("GET", "/v1/admin/hub"), api("GET", "/v1/info")]);
      render();
      state.hubStatus = await api("GET", "/v1/admin/hub/status");
    } else if (state.view === "account") {
      state.mfaState = await api("GET", "/v1/me/mfa");
    } else if (state.view === "network") {
      state.info = await api("GET", "/v1/info");
      const set = await api("GET", "/v1/admin/settings");
      state.adminAccess = set.admin_access;
      state.form.chatLimit ??= String(set.chat_file_limit_mb);
      state.form.callRecording ??= set.call_recording;
      state.form.chatThreads ??= set.chat_threads;
      const keep = String(set.chat_keep_days || 0);
      state.form.keepChoice ??= KEEP_CHOICES.some(([v]) => v === keep) ? keep : "custom";
      state.form.keepDays ??= keep === "0" ? "" : keep;
      const m = await api("GET", "/v1/admin/mail");
      state.form.mail ??= { ...m, password: "", port: m.port ? String(m.port) : "", security: m.security || "starttls" };
      state.mailPasswordSet = m.password_set;
      state.scan = await api("GET", "/v1/admin/scan");
      state.form.scan ??= { on: state.scan.on, address: state.scan.address || "" };
      state.storage = await api("GET", "/v1/admin/storage");
      state.form.quotaGb ??= state.storage.quota_gb ? String(state.storage.quota_gb) : "";
      await loadHttps();
      await loadNet();
    }
  } catch (e) {
    state.error = e.message;
  }
  render();
}

async function loadAccess() {
  const sp = state.spaces.find((s) => s.id === state.spaceSel);
  state.access = sp && sp.kind !== "personal" ? await api("GET", `/v1/spaces/${sp.id}/access`) : null;
}

// ---- Drawing --------------------------------------------------------------------

const icon = {
  email: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/></svg>',
  backups: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5"/><path d="M4 11v6c0 1.7 3.6 3 8 3 1 0 2-.1 2.9-.2"/><path d="M18 15v6M15 18l3 3 3-3"/></svg>',
  holidays: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16v14H4zM4 10h16M8 3.5V7M16 3.5V7"/><path d="M9 15l2 2 4-4"/></svg>',
  roles: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5c0 4.5-3 8.2-7 10-4-1.8-7-5.5-7-10V6z"/><path d="M9 12l2 2 4-4"/></svg>',
  people: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>',
  spaces: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>',
  ai: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.9 5.8L20 10l-5 3.9L16.6 20 12 16.6 7.4 20 9 13.9 4 10l6.1-1.2z"/></svg>',
  updates: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/></svg>',
  network: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
  links: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
  // Lucide's "database" (ISC).
  data: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/></svg>',
  account: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/></svg>',
  search: '<svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>',
};

function top() {
  return `<header class="top"><span class="mark" aria-hidden="true">X</span><b>${esc(state.info?.name || "Xenify server")}</b></header>`;
}

function errorBox() {
  return state.error ? `<div class="error" role="alert">${esc(state.error)}</div>` : "";
}

function field(label, bind, { type = "text", placeholder = "", auto = "" } = {}) {
  const id = `f-${bind.replace(/\W/g, "-")}`;
  return `<label class="field" for="${id}"><span>${esc(label)}</span><input class="inp" id="${id}" type="${type}" data-bind="${bind}" value="${esc(getPath(bind) ?? "")}" placeholder="${esc(placeholder)}" ${auto ? `autocomplete="${auto}"` : ""}></label>`;
}

function getPath(path) {
  return path.split(".").reduce((o, k) => (o == null ? o : o[k]), state);
}

function setPath(path, value) {
  const parts = path.split(".");
  let o = state;
  for (const k of parts.slice(0, -1)) o = o[k] ??= {};
  o[parts.at(-1)] = value;
}

function setupPage() {
  const body = `<form class="sheet narrow" data-act="setup-finish">
      <div><h1>Set up your server</h1><p class="dim">You'll be its first admin. You can add people afterwards.</p></div>
      ${errorBox()}
      ${field("Your name", "form.display_name", { placeholder: "Alex Morgan", auto: "name" })}
      ${field("Email (you sign in with it)", "form.email", { type: "email", placeholder: "alex@example.org", auto: "username" })}
      ${field("Password", "form.password", { type: "password", auto: "new-password" })}
      ${field("Password again", "form.password2", { type: "password", auto: "new-password" })}
      <div class="row end"><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>${state.busy ? "Setting up…" : "Set up"}</button></div></form>`;
  return `<div class="solo">${top()}<main>${body}</main></div>`;
}

function signinPage() {
  // A plain sign-in page: nothing about what the server is or runs.
  if (state.mfa?.ticket) return `<div class="solo plain"><main><div class="sheet narrow">${mfaHtml()}</div></main></div>`;
  // The email address first; then the password, or (an account without a
  // password yet, or a forgotten one) a code, emailed or from an admin,
  // and a new password.
  const stage = state.form.stage || "user";
  const who = `<p class="dim">${esc(state.form.username || "")} · <button class="link" type="button" data-act="signin-back">Change</button></p>`;
  const go = (label) => `<button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>${label}</button>`;
  const body =
    stage === "user"
      ? `${field("Email", "form.username", { auto: "username" })}
    <div class="row">${go("Next")}</div>`
      : stage === "password"
        ? `${who}${field("Password", "form.password", { type: "password", auto: "current-password" })}
    <div class="row">${go(state.busy ? "Signing in…" : "Sign in")}<span class="grow"></span><button class="link" type="button" data-act="to-recover">Forgot password?</button></div>`
        : stage === "code"
          ? `${who}<p>${esc(state.form.note || "")}</p>
    ${field("Code", "form.code", { auto: "one-time-code" })}
    <div class="row">${go("Next")}<span class="grow"></span>${state.form.emailed ? '<button class="link" type="button" data-act="signin-email">Send a new code</button>' : ""}</div>`
          : `${who}<p>${state.form.kind === "reset" ? "Pick a new password." : `Welcome to ${esc(state.info?.name || "this server")}. Pick your password.`}</p>
    ${field("Password", "form.password", { type: "password", auto: "new-password" })}
    ${field("Password again", "form.password2", { type: "password", auto: "new-password" })}
    <div class="row">${go(state.busy ? "Signing in…" : "Set password and sign in")}</div>`;
  return `<div class="solo plain"><main><form class="sheet narrow" data-act="signin">
    <div><h1>Sign in</h1></div>
    ${errorBox()}
    ${body}
  </form></main></div>`;
}

function recoverPage() {
  return `<div class="solo plain"><main><div class="sheet narrow">
    <div><h1>Forgot your password?</h1><p class="dim">Ask an admin to reset it: you then pick a new one when you sign in. The only admin resets theirs on the server's computer (see its documentation).</p></div>
    <div class="row"><span class="grow"></span><button class="link" type="button" data-act="to-signin">Back to sign in</button></div>
  </div></main></div>`;
}

/// Welcome and reset links: `/join/#<id>.<secret>` (or the code typed in
/// when signing in).
function joinPage() {
  const l = state.link;
  const done = state.form.joined;
  let body;
  if (done) {
    body = `<div class="sheet narrow"><h1>You're in</h1><p>Open any Xenify app and sign in to <b>${esc(location.origin)}</b> with <b>${esc(done)}</b>, or <a href="/files/">open your files here</a>.</p></div>`;
  } else if (!l) {
    body = `<div class="sheet narrow"><h1>${state.error ? "This link doesn't work" : "Checking the link…"}</h1>${errorBox()}<p class="dim">Links work once, for a limited time. Ask whoever sent it for a new one.</p></div>`;
  } else {
    const title = l.kind === "reset" ? "Pick a new password" : `Welcome to ${esc(l.server_name || "this server")}`;
    const who = `You sign in with <b>${esc(l.email || l.username)}</b>.`;
    body = `<form class="sheet narrow" data-act="join">
    <div><h1>${title}</h1><p class="dim">${who}</p></div>
    ${errorBox()}
    ${field("Password", "form.password", { type: "password", auto: "new-password" })}
    ${field("Password again", "form.password2", { type: "password", auto: "new-password" })}
    <div class="row end"><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>${l.kind === "reset" ? "Save password" : "Join"}</button></div>
  </form>`;
  }
  return `<div class="solo">${top()}<main>${body}</main></div>`;
}

function shell(content) {
  const items = [["people", "People &amp; groups"], ["roles", "Roles"], ["spaces", "Spaces"], ["links", "Public links"], ["ai", "AI"], ["data", "Data sources"], ["holidays", "Holidays"], ["updates", "Apps &amp; updates"], ["backups", "Backups"], ["email", "Email"], ["network", "Network"], ["account", "Your account"]];
  return `<div class="admin"><nav class="nav" aria-label="Admin">
    <a href="/files/" class="brand" title="Back to Files"><span class="mark" aria-hidden="true">X</span>${esc(state.info?.name || "Xenify server")}</a>
    ${items.filter(([v]) => mayOpen(v)).map(([v, n]) => `<a href="#/${v}" class="${state.view === v ? "on" : ""}" ${state.view === v ? 'aria-current="page"' : ""}>${icon[v]}${n}</a>`).join("")}
    <a href="/files/" class="tofiles"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M15 18l-6-6 6-6"/></svg>Back to Files</a>
    <div class="foot">${esc(location.host)} · version ${esc(state.info?.version || "")}</div>
  </nav><main class="page"><div class="sheet">${content}</div></main></div>`;
}

// The roles someone has themselves (Admin first; not through groups).
const rolesOf = (p) => (state.serverRoles || []).filter((r) => !r.everyone && (r.admin ? p.admin : r.members?.some((m) => m.user === p.id)));

function personRow(p) {
  const [bg, fg] = colours(p.username);
  const groups = p.groups.map((g) => state.groups.find((x) => x.id === g)?.name).filter(Boolean);
  const seen = p.pending ? "—" : p.last_seen ? ago(p.last_seen) : "Never";
  const sub = p.disabled ? "Turned off" : p.pending ? "Hasn't picked a password yet" : p.email || p.username;
  // Someone must always be able to run the server.
  const lastAdmin = p.admin && !p.disabled && state.people.filter((x) => x.admin && !x.disabled).length === 1;
  return `<div class="tr" ${p.disabled ? 'style=""' : ""}>
    <span class="who"><span class="av" style="background:${bg};color:${fg}">${esc(initials(nameOf(p)))}</span><span><b>${esc(nameOf(p))}</b><br><span class="dim">${esc(sub)}</span></span></span>
    <span class="chips">${groups.map((g) => `<span class="chip">${esc(g)}</span>`).join("") || '<span class="dim">—</span>'}</span>
    <span class="dim">${esc(seen)}</span>
    <span class="chips">${rolesOf(p).map((r) => `<span class="chip${r.admin ? " ember" : ""}">${esc(r.name)}</span>`).join("") || '<span class="dim">Everyone</span>'}</span>
    <span class="anchor"><button class="more" aria-label="More for ${esc(nameOf(p))}" aria-haspopup="menu" data-act="menu" data-id="p${p.id}">⋯</button>${
      state.menu === `p${p.id}`
        ? `<div class="menu" role="menu">
        ${p.admin && !isAdmin() ? '<p class="dim menunote">Only admins change an admin’s account.</p>' : `<button role="menuitem" data-act="edit-person" data-id="${p.id}">Name, groups and roles…</button>
        <button role="menuitem" data-act="reset" data-id="${p.id}">Reset password…</button>
        <button role="menuitem" data-act="clear-mfa" data-id="${p.id}">Clear two-step sign-in…</button>
        ${lastAdmin ? "" : `<button role="menuitem" class="danger" data-act="toggle-off" data-id="${p.id}">${p.disabled ? "Turn back on" : "Turn off"}</button>`}${lastAdmin ? '<p class="dim menunote">The only admin: make someone else an admin to change this.</p>' : ""}`}</div>`
        : ""
    }</span></div>`;
}

// Admin › Public links: the switch for the whole server, and every link.
function linksPage() {
  const pl = state.publinks;
  if (!pl) return shell(`<h1>Public links</h1>${errorBox()}`);
  const on = pl.on;
  const rows = pl.links.map((l) => `<div class="tr links">
    <span><b>${esc(l.name)}</b>${l.kind === "folder" ? ` <span class="dim">(folder, ${l.files - 1} inside)</span>` : ""}<br><span class="dim">${esc(l.place)}</span></span>
    <span>${esc(l.made_by || "Someone")}<br><span class="dim">${esc(ago(l.made))}</span></span>
    <span class="dim">${l.opened ? esc(ago(l.opened)) : "Never"}</span>
    <span class="row">${l.token ? `<a class="btn" href="/p/${esc(l.token)}" target="_blank" rel="noopener" title="What anyone with the link sees">Open</a><button class="btn" data-act="copy" data-text="${esc(`${location.origin}/p/${l.token}`)}">Copy link</button>` : ""}${on ? `<button class="btn danger" data-act="stop-link" data-id="${l.id}">Stop link</button>` : '<span class="dim">Paused</span>'}</span>
  </div>`);
  return shell(`<h1>Public links</h1>${errorBox()}
    <section class="stack" aria-label="Whole server">
      <label class="check"><input type="checkbox" data-act="toggle-links" ${on ? "checked" : ""}><b>Let people share pages with anyone</b></label>
      <p class="dim">${on
        ? "People who can change a file or folder can give it a read-only link. Turning this off stops every link below at once."
        : "Off: no one can make a public link, and every link below has stopped working. Turning it back on brings back the ones nobody stopped."}</p>
    </section>
    <h2>Links people have made <span class="dim">· ${pl.links.length}</span></h2>
    ${pl.links.length
      ? `<div class="scroll"><div class="table"><div class="tr links th"><span>What it opens</span><span>Made by</span><span>Last opened</span><span></span></div>${rows.join("")}</div></div>`
      : '<p class="dim">None yet. People make them from Share… on the server\'s Files page.</p>'}`);
}

function ago(iso) {
  const s = (Date.now() - Date.parse(iso)) / 1000;
  if (s < 120) return "Now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  if (s < 172800) return "Yesterday";
  return `${Math.round(s / 86400)} days ago`;
}

function peoplePage() {
  const q = state.find.toLowerCase();
  const people = state.people.filter((p) => !q || nameOf(p).toLowerCase().includes(q) || (p.email || "").toLowerCase().includes(q));
  const tabs = `<div class="seg" role="tablist"><button role="tab" class="${state.tab === "people" ? "on" : ""}" data-act="tab" data-id="people">People · ${state.people.length}</button><button role="tab" class="${state.tab === "groups" ? "on" : ""}" data-act="tab" data-id="groups">Groups · ${state.groups.length}</button></div>`;
  let list;
  if (state.tab === "people") {
    list = `<label class="search">${icon.search}<input aria-label="Find people" placeholder="Find people" data-bind="find" value="${esc(state.find)}"></label>
      <div class="scroll"><div class="table"><div class="tr th"><span>Name</span><span>Groups</span><span>Last seen</span><span>Roles</span><span></span></div>
      ${people.map(personRow).join("")}</div></div>
      <div class="note">Forgot their password? ⋯ › <b>Reset password</b>. They pick a new one when they next sign in, with a code ${mailOn() ? "sent to their email (or one you give them, if they have no address)" : "you give them"}.</div>
      ${isAdmin() ? mfaRuleSection() : ""}`;
  } else {
    list = `<div class="stack">${state.groups
      .map((g) => `<div class="res"><div class="who"><span class="av group">${esc(initials(g.name))}</span><span><b>${esc(g.name)}</b><br><span class="dim">${g.members.length === 1 ? "1 person" : `${g.members.length} people`}</span></span></div>
        <div class="row"><button class="btn small" data-act="edit-group" data-id="${g.id}">Change</button><button class="btn small danger" data-act="delete-group" data-id="${g.id}">Remove</button></div></div>`)
      .join("") || '<p class="dim">No groups yet. Groups make giving access to spaces quick.</p>'}</div>`;
  }
  const add = state.tab === "people" ? `<button class="btn main" data-act="add-person">Add person</button>` : `<button class="btn main" data-act="add-group">New group</button>`;
  return shell(`<div class="head"><h1>People &amp; groups</h1>${tabs}${add}</div>${errorBox()}${list}`);
}

function spacesPage() {
  const sel = state.spaces.find((s) => s.id === state.spaceSel);
  const spaceItem = (s) => `<button class="space ${s.id === state.spaceSel ? "on" : ""}" data-act="pick-space" data-id="${s.id}"><span class="ico" aria-hidden="true">${esc(initials(state.names[s.id] || "?")[0])}</span><span><b>${esc(state.names[s.id] || "")}</b><span>${s.kind === "personal" ? "Each person's own · only they can open it" : s.kind === "templates" ? "Everyone can use · admins change" : ""}</span></span></button>`;
  let detail = "";
  if (sel && sel.kind === "personal") {
    detail = `<h2>My files</h2><p class="dim">Everyone has their own. Only they can open it; they can share single folders and files from it in the apps. Admins can't open anyone's My files.</p>`;
  } else if (sel && state.access) {
    const whoName = (w) => (w.user ? nameOf(state.people.find((p) => p.id === w.user) || { username: "?" }) : w.group ? state.groups.find((g) => g.id === w.group)?.name || "?" : "Everyone");
    const count = (n) => (n === 1 ? "1 person" : `${n} people`);
    const whoKind = (w) => (w.user ? "" : w.group ? `group · ${count(state.groups.find((g) => g.id === w.group)?.members.length ?? 0)}` : "everyone with an account");
    const options = [
      ...state.groups.map((g) => [`g${g.id}`, `${g.name} (group)`]),
      ...state.people.filter((p) => !p.disabled).map((p) => [`u${p.id}`, nameOf(p)]),
      ["everyone", "Everyone"],
    ];
    detail = `<div class="head"><h2>Who can open ${esc(state.names[sel.id])}</h2>${sel.kind === "shared" ? `<button class="btn small" data-act="rename-space">Rename</button><button class="btn small danger" data-act="delete-space">Remove</button>` : ""}</div>
      <p class="dim">Folders inside follow this list unless a folder has its own (set in the apps).</p>
      <div class="row"><select class="inp grow" aria-label="Add a person or group" data-bind="form.grantWho"><option value="">Add a person or group…</option>${options.map(([v, n]) => `<option value="${v}" ${state.form.grantWho === v ? "selected" : ""}>${esc(n)}</option>`).join("")}</select><button class="btn" data-act="add-grant">Add</button></div>
      <div>${state.access.grants
        .map((g, i) => `<div class="grant"><span class="who"><span class="av ${g.who.user ? "" : "group"}" style="${g.who.user ? (([bg, fg]) => `background:${bg};color:${fg}`)(colours(whoName(g.who))) : ""}">${esc(initials(whoName(g.who)))}</span><span><b>${esc(whoName(g.who))}</b> <span class="dim">${esc(whoKind(g.who))}</span></span></span>
          <select class="inp" aria-label="Access for ${esc(whoName(g.who))}" data-act="grant-role" data-id="${i}">${Object.entries(ROLES).map(([r, n]) => `<option value="${r}" ${!g.custom && g.role === r ? "selected" : ""}>${n}</option>`).join("")}${(state.docRoles || []).map((r) => `<option value="c${r.id}" ${g.custom === r.id ? "selected" : ""}>${esc(r.name)}</option>`).join("")}</select>
          <button class="more" aria-label="Remove ${esc(whoName(g.who))}" data-act="remove-grant" data-id="${i}">×</button></div>`)
        .join("") || '<p class="dim">Nobody yet (admins can always manage it).</p>'}</div>
      <div class="roles"><div><b>Can view</b> — open and download</div><div><b>Can comment</b> — and leave comments</div><div><b>Can edit</b> — change, add and move files</div><div><b>Can manage</b> — and change this list</div>${(state.docRoles || []).map((r) => `<div><b>${esc(r.name)}</b> — a role made on Roles</div>`).join("")}</div>`;
  }
  return shell(`<div class="split"><section class="list" aria-label="Spaces"><div class="head"><h1>Spaces</h1><button class="btn main" data-act="add-space">New space</button></div>${errorBox()}${state.spaces.map(spaceItem).join("")}</section>
    <section class="detail" aria-label="Access">${detail}</section></div>`);
}

/// ComfyUI: pictures and clips. Its address, whether it answers, and for each
/// model the server knows whether it's ready or which files it still needs.
function comfyCard(a) {
  const c = state.comfy;
  const ready = (c?.models || []).filter((m) => m.ready).length;
  const status = !(a.comfy_url || "").trim() ? "Not set up. Install ComfyUI (Stability Matrix is the easiest way), then type its address."
    : !c ? "Checking…" : c.reachable ? `Answering · ${ready} of ${c.models.length} models ready` : `Not answering: ${c.error || ""}`;
  const kind = { image: "Pictures", video: "Clips" };
  const rows = c?.reachable ? c.models.map((m) => `<div class="res"><div class="stack"><b>${esc(m.name)}</b><div class="chips"><span class="chip">${kind[m.kind] || m.kind}</span>${m.ready ? '<span class="chip ok">Ready: offer it below</span>' : '<span class="chip warn">Needs files</span>'}</div>
      ${m.ready ? "" : `<span class="dim">Download into ComfyUI's models folder (Stability Matrix's model browser can find them by name): ${m.missing.map((f) => `<a href="${esc(f.url)}" target="_blank" rel="noopener">${esc(f.file)}</a> in ${esc(f.folder)}`).join(", ")}</span>`}</div></div>`).join("") : "";
  return `<section class="card stack comfy" aria-label="ComfyUI">
        <span class="k">Pictures and clips · ComfyUI</span>
        ${field("ComfyUI's address", "form.ai.comfy_url", { placeholder: "http://localhost:8188" })}
        <p class="dim">${esc(status)}</p>${rows}
        <div class="row"><button class="btn main" data-act="ai-save">Save</button>${c?.set_up ? '<button class="btn" data-act="comfy-check">Check again</button>' : ""}</div>
      </section>`;
}

/// Addresses of well-known runners and OpenAI-style services, for the
/// Address field's help: [name, address, program].
const AI_ADDRESSES = [
  ["Ollama (this computer)", "http://localhost:11434/v1", "ollama"],
  ["LocalAI (this computer)", "http://localhost:8080/v1", "localai"],
  ["LM Studio (this computer)", "http://localhost:1234/v1", "openai"],
  ["OpenRouter", "https://openrouter.ai/api/v1", "openai"],
  ["OpenAI", "https://api.openai.com/v1", "openai"],
  ["Mistral", "https://api.mistral.ai/v1", "openai"],
  ["Groq", "https://api.groq.com/openai/v1", "openai"],
  ["DeepSeek", "https://api.deepseek.com/v1", "openai"],
  ["Together", "https://api.together.xyz/v1", "openai"],
];

function aiPage() {
  const a = state.form.ai;
  if (!a) return shell(`<h1>AI</h1>${errorBox()}<p class="dim">Loading…</p>`);
  const st = state.aiStatus;
  const runnerName = { localai: "LocalAI", ollama: "Ollama", openai: "Another service" }[a.runner] || "Ollama";
  const status = !a.url ? "Not set up yet" : !st ? "Checking…" : st.reachable ? `Answering · ${st.models.length === 1 ? "1 model" : `${st.models.length} models`}` : `Not answering: ${st.error || ""}`;
  const installed = st?.models || [];
  const kindRow = ([k, label]) => {
    const def = a[`${k}_model`];
    const offered = [...new Set([...(def ? [def] : []), ...((a.offered || {})[k] || [])])];
    // ComfyUI's ready models can be offered for pictures and clips.
    const comfy = (state.comfy?.models || []).filter((m) => m.ready && m.kind === k).map((m) => m.id);
    // The runner's list may say what each model does (OpenRouter's does).
    const fits = (m) => !st?.kinds?.[m] || st.kinds[m].includes(k);
    // A long list (a service's): in order, found by typing any part of the name.
    const addable = [...new Set([...installed.filter(fits), ...comfy])].filter((m) => !offered.includes(m))
      .sort((x, y) => x.localeCompare(y, undefined, { sensitivity: "base", numeric: true }));
    // The default (used when nobody picks) is labelled; the others can be made it.
    return `<div class="res"><div class="stack"><b>${label}</b><div class="chips">${offered
      .map((m) => {
        // Offered but not a model the runner has, or one that doesn't do this: said, so it can be swapped.
        const why = !st?.reachable || k === "lora" || comfy.includes(m) || (state.comfy?.models || []).some((x) => x.id === m) ? ""
          : !installed.includes(m) ? `${a.runner === "openai" ? "the service" : runnerName} doesn't have it` : st.kinds?.[m] && !st.kinds[m].includes(k) ? `it can't do ${label.toLowerCase()}` : "";
        return `<span class="chip ${why ? "warn" : m === def ? "ok" : ""}" ${why ? `title="${esc(why)}"` : ""}>${esc(m)}${why ? ` · ⚠ ${esc(why)}` : ""}${k === "lora" ? "" : m === def ? ' <span class="tag">default</span>' : ` <button class="link" data-act="ai-default" data-kind="${k}" data-model="${esc(m)}" title="Used when people don't pick one">Make default</button>`} <button class="link" data-act="ai-unoffer" data-kind="${k}" data-model="${esc(m)}" aria-label="Stop offering ${esc(m)}">×</button></span>`;
      })
      .join("") || '<span class="dim">None yet</span>'}</div></div>
      ${k === "lora" ? (state.style ? "" : '<button class="btn small" data-act="style-open">Make a style</button>') : addable.length ? `<input class="inp offer" list="offer-${k}" aria-label="Offer a model for ${label}: type to find it" placeholder="Offer a model (${addable.length}): type to find" data-act="ai-offer" data-kind="${k}"><datalist id="offer-${k}">${addable.map((m) => `<option value="${esc(m)}"></option>`).join("")}</datalist>` : ""}</div>${k === "lora" ? styleForm() : ""}${k === "data" ? thinkingRow(a) : ""}`;
  };
  const f = state.aiFind;
  const hits = f.hits
    ? f.hits.map((h) => `<div class="res"><div class="stack"><b>${esc(h.name)}</b><div class="chips"><span class="chip">${esc(KINDS.find((k) => k[0] === h.kind)?.[1] || h.kind)}</span>${h.base_model ? `<span class="chip">${esc(h.base_model)}</span>` : ""}${h.license ? `<span class="chip">Licence: ${esc(h.license)}</span>` : ""}${h.gated ? '<span class="chip warn">Sign-in needed at Hugging Face</span>' : ""}<span class="dim">${h.downloads.toLocaleString()} downloads</span></div>
        ${f.open === h.id && f.files ? `<div class="stack">${f.files.map((x) => `<div class="row"><span class="grow">${esc(x.name)} <span class="dim">${size(x.size)}${x.more?.length ? ` · ${x.more.length + 1} parts` : ""}${x.version ? ` · ${esc(x.version)}` : ""}</span></span>${x.safe ? `<button class="btn small main" data-act="ai-install" data-id="${esc(h.id)}" data-file="${esc(x.name)}" data-kind="${esc(h.kind)}">Install</button>` : '<span class="chip warn">Old file type: not installed (it could run programs)</span>'}</div>`).join("") || '<span class="dim">No files that can be installed.</span>'}</div>` : ""}</div>
        ${f.source === "hf" && (h.kind === "image" || h.kind === "video") ? `<button class="btn small main" data-act="ai-install" data-id="${esc(h.id)}" data-file="" data-kind="${esc(h.kind)}">Install</button>` : `<button class="btn small" data-act="ai-files" data-id="${esc(h.id)}">${f.open === h.id ? "Hide files" : "Files"}</button>`}</div>`).join("") || '<p class="dim">Nothing found.</p>'
    : "";
  const jobs = state.jobs.filter((j) => j.state !== "installed" || Date.now() - (j._seen ??= Date.now()) < 60000);
  return shell(`<h1>AI</h1>${errorBox()}
    <div class="cards">
      <section class="card stack runner" aria-label="Runner">
        <span class="k">Writing and questions · Ollama, LocalAI or a service</span>
        <label class="field"><span>Program</span><select class="inp" data-bind="form.ai.runner"><option value="ollama" ${a.runner === "ollama" ? "selected" : ""}>Ollama (recommended)</option><option value="localai" ${a.runner === "localai" ? "selected" : ""}>LocalAI</option><option value="openai" ${a.runner === "openai" ? "selected" : ""}>Another OpenAI-style service</option></select></label>
        ${field("Address", "form.ai.url", { placeholder: a.runner === "ollama" ? "http://localhost:11434/v1" : "http://localhost:8080/v1" })}
        <details class="keys"><summary>Common addresses</summary><div class="stack addrs">${AI_ADDRESSES.map(([name, url, runner]) => `<div class="row"><span class="grow"><b>${esc(name)}</b><br><span class="dim">${esc(url)}</span></span><button class="btn small" type="button" data-act="ai-address" data-url="${esc(url)}" data-runner="${runner}">Use</button></div>`).join("")}<p class="dim">Services on the internet need a key from their website.</p></div></details>
        ${a.runner === "localai" ? field("Its models folder on this computer", "form.ai.models_dir", { placeholder: "/home/you/localai/models" }) : ""}
        ${field(a.key_set ? (hostOf(a.url) !== hostOf(state.ai?.url) ? "Key (type it again for the new address)" : "Key (kept; type to change)") : "Key (if it needs one)", "form.ai.key", { type: "password" })}
        <p class="dim">${esc(runnerName)}: ${esc(status)}</p>
        <div class="row"><button class="btn main" data-act="ai-save">Save</button></div>
      </section>
      ${comfyCard(a)}
      <section class="card stack" aria-label="GIFs">
        <span class="k">GIFs · GIPHY</span>
        <p class="dim">With a GIPHY key, Draw offers Insert › GIF to everyone here (searched through this server; people need no key). Without one, GIFs aren't offered. A free key: developers.giphy.com.</p>
        ${field(a.giphy_key_set ? "GIPHY key (kept; type to change)" : "GIPHY key", "form.ai.giphy_key", { type: "password" })}
        <div class="row"><button class="btn main" data-act="ai-save">Save</button>${a.giphy_key_set ? '<button class="btn" data-act="giphy-forget">Stop offering GIFs</button>' : ""}</div>
      </section>
      ${limitsCard(a)}
      <section class="card offered" aria-label="Models people can use"><span class="k">Models people can use · they pick in the app; the default is used when they don't</span>${KINDS.map(kindRow).join("")}</section>
    </div>
    ${jobs.length ? `<section class="stack" aria-label="Installing"><h2>Installing</h2>${jobs.map((j) => `<div class="res"><div class="stack"><b>${esc(j.name)}</b><div class="row">${j.total ? `<div class="bar"><i style="width:${Math.min(100, (100 * j.done) / j.total).toFixed(1)}%"></i></div><span class="dim">${size(j.done)} of ${size(j.total)}</span>` : ""}<span class="dim">${esc({ downloading: "Downloading", checking: "Checking", loading: "Loading", installed: "Installed", failed: "Didn't work", stopped: "Stopped" }[j.state] || j.state)}${j.message ? `: ${esc(j.message)}` : ""}</span></div></div>${["downloading", "checking"].includes(j.state) ? `<button class="btn small" data-act="ai-stop" data-id="${esc(j.id)}">Stop</button>` : ""}</div>`).join("")}</section>` : ""}
    <section class="stack" aria-label="Add models">
      <div class="head"><h2>Add models</h2><div class="seg" role="tablist" aria-label="Where to look"><button role="tab" class="${f.source === "hf" ? "on" : ""}" data-act="ai-source" data-id="hf">Hugging Face</button><button role="tab" class="${f.source === "civitai" ? "on" : ""}" data-act="ai-source" data-id="civitai">Civitai</button></div></div>
      <form class="row" data-act="ai-search"><input class="inp q" aria-label="Search models" placeholder="Search models" data-bind="aiFind.q" value="${esc(f.q)}">
        <select class="inp auto" aria-label="Kind" data-bind="aiFind.kind">${KINDS.filter(([k]) => (f.source === "civitai" ? k === "image" || k === "lora" : k !== "lora")).map(([k, n]) => `<option value="${k}" ${f.kind === k ? "selected" : ""}>${n}</option>`).join("")}</select>
        <button class="btn" type="submit">Search</button></form>
      ${hits}
      <details class="keys"><summary>Keys for models that need you signed in${a.hf_token_set || a.civitai_token_set ? " (kept)" : ""}</summary>
        <div class="row">${field(a.hf_token_set ? "Hugging Face key (kept; type to change)" : "Hugging Face key", "form.ai.hf_token", { type: "password" })}${field(a.civitai_token_set ? "Civitai key (kept; type to change)" : "Civitai key", "form.ai.civitai_token", { type: "password" })}</div>
        <div class="row"><button class="btn small" data-act="ai-save">Save keys</button></div></details>
      <label class="check"><input type="checkbox" data-act="ai-adult" ${a.adult ? "checked" : ""}>Show Civitai models marked as adult content</label>
    </section>${mcpSection()}`);
}

/// "https://host:port" of an address, to see when a kept key would go somewhere new.
function hostOf(url) {
  const m = /^\s*(?:([a-z][a-z0-9+.-]*):\/\/)?([^/?#]*)/i.exec(url || "");
  return `${(m?.[1] || "http").toLowerCase()}://${(m?.[2] || "").replace(/^.*@/, "").toLowerCase()}`;
}

const LIMITS = [
  ["pictures_at_once", "Pictures one person makes at a time"],
  ["clips_at_once", "Clips one person makes at a time"],
  ["server_at_once", "Pictures and clips at a time on this server"],
  ["media_per_hour", "Pictures and clips per person per hour"],
  ["text_per_hour", "Questions and writing per person per hour"],
];

/// How much AI each person may use.
function limitsCard(a) {
  a.limits ??= {};
  return `<section class="card stack" aria-label="Limits">
    <span class="k">Limits · 0 means no limit</span>
    <div class="row">${LIMITS.map(([k, label]) => `<label class="field" for="f-limit-${k}"><span>${esc(label)}</span><input class="inp" id="f-limit-${k}" type="number" min="0" step="1" data-bind="form.ai.limits.${k}" value="${esc(String(a.limits[k] ?? ""))}"></label>`).join("")}</div>
    <div class="row"><button class="btn main" data-act="ai-limits">Save limits</button></div>
  </section>`;
}

/// Tools for Ask (MCP): tool servers by address, for everyone using Calc's Ask.
function mcpSection() {
  const list = state.mcpTools || [];
  state.form.mcp ??= { name: "", url: "", header: "", value: "" };
  return `<section class="stack" aria-label="Tools for Calc's questions"><h2>Tools for Calc's questions (MCP)</h2>
    <p class="dim">Tool servers Calc can use when people ask it about a spreadsheet (Ask…, Ctrl+J), for everyone here, such as your accounts system. Ask says which tool it used and asks the person before using a tool. Keys stay on this server. Only tools that say they only read are offered, unless you allow tools that change things.</p>
    ${list.map((t) => `<div class="res"><div class="stack"><b>${esc(t.name)}</b><span class="dim">${esc(t.url)}${t.headers.length ? ` · sends ${esc(t.headers.join(", "))}` : ""}</span>
      <label class="check"><input type="checkbox" data-act="mcp-changes" data-id="${t.id}" ${t.changes ? "checked" : ""}>Allow its tools that change things</label></div><button class="btn small danger" data-act="mcp-remove" data-id="${t.id}">Remove</button></div>`).join("") || '<p class="dim">None yet.</p>'}
    <div class="row">${field("Name", "form.mcp.name", { placeholder: "Accounts" })}${field("Address (Streamable HTTP)", "form.mcp.url", { placeholder: "https://accounts.office.lan/mcp" })}</div>
    <div class="row">${field("Header it needs (optional)", "form.mcp.header", { placeholder: "Authorization" })}${field("Its value", "form.mcp.value", { type: "password" })}</div>
    <label class="check"><input type="checkbox" data-act="mcp-new-changes" ${state.form.mcp.changes ? "checked" : ""}>Allow its tools that change things</label>
    <div class="row"><button class="btn" data-act="mcp-add">Add the tool</button></div></section>`;
}

/// Working with data (Calc's Ask): optional, and how long its model thinks.
function thinkingRow(a) {
  const t = a.data_thinking || "normal";
  return `<div class="res think"><span class="dim">Used when someone asks Calc about a spreadsheet (Ask…, Ctrl+J), such as "which customers paid late?". Pick a model that thinks before it answers and can use tools, such as Qwen3 30B. Optional: without one, Calc uses the writing model and handles only simpler questions.</span>
    <label class="row"><span class="dim">How long it thinks</span><select class="inp auto" aria-label="How long it thinks" data-act="ai-thinking">${[["quick", "Quick"], ["normal", "Normal"], ["thorough", "Thorough"]].map(([v, n]) => `<option value="${v}" ${t === v ? "selected" : ""}>${n}</option>`).join("")}</select></label></div>`;
}

/// A picture model with a LoRA, offered as a picture model of its own.
function styleForm() {
  const st = state.style;
  if (!st) return "";
  const { bases, loras } = st.parts;
  const why = !bases.length ? "Offer a picture model first: a style is made from one." : !loras.length ? "Install a LoRA first: Add models › Civitai › Picture styles (LoRA)." : "";
  const pick = (bind, list, label) => `<label class="field"><span>${label}</span><select class="inp" data-bind="${bind}">${list.map((m) => `<option ${getPath(bind) === m ? "selected" : ""}>${esc(m)}</option>`).join("")}</select></label>`;
  return `<div class="stack style-form" role="group" aria-label="Make a picture style">
    <p class="dim">A style is a picture model with a LoRA added. People pick it like any other picture model.</p>
    ${why ? `<p class="dim">${esc(why)}</p>` : `<div class="row">${field("Name", "style.name", { placeholder: "Ink wash" })}${pick("style.base", bases, "Made from")}${pick("style.lora", loras, "LoRA")}${field("Strength (0.1–2)", "style.strength", { type: "number" })}</div>`}
    <div class="row">${why ? "" : '<button class="btn small main" data-act="style-make">Make it</button>'}<button class="btn small" data-act="style-close">Cancel</button></div></div>`;
}

const size = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(0)} MB` : `${Math.max(1, Math.round(b / 1e3))} KB`);

// Apps people use, as Xenify Hub installs them.
const APPS = [["write", "Write", "#C2410C", "/write/"], ["draw", "Draw", "#0F766E", "/draw/"], ["calc", "Calc", "#C2410C", "/calc/"], ["plan", "Plan", "#C2410C", "/plan/"], ["chat", "Chat", "#1D4ED8", "/chat/"]];

function updatesPage() {
  const hub = state.hub;
  if (!hub) return shell(`<h1>Apps &amp; updates</h1>${errorBox()}<p class="dim">Loading…</p>`);
  const st = state.hubStatus || { products: {}, latest: {}, computers: 0 };
  const cur = state.info?.version || st.server || "";
  const next = st.latest?.server;
  const choiceOf = (p) => {
    const c = p?.choice;
    if (p && p.allowed === false) return "off";
    if (!c) return "";
    return c.fixed ? "fixed" : c.channel || "";
  };
  const whenOf = (p) => (!p?.mode ? "" : p.mode === "manual" ? "manual" : p.window ? "night" : "auto");
  const counts = (id) => {
    const v = st.products?.[id] || {};
    const total = Object.values(v).reduce((a, n) => a + n, 0);
    if (!total) return '<span class="dim">No computers yet</span>';
    const latest = st.latest?.[id];
    const on = latest ? v[latest] || 0 : 0;
    if (latest && on < total) return `<span class="tag new">${total - on} can update to ${esc(latest)}</span>`;
    const top = Object.entries(v).sort((a, b) => b[1] - a[1])[0];
    return `<span class="tag ok">${top[1]} of ${total} on ${esc(top[0])}</span>`;
  };
  // "All apps": a channel and when, for every app whose own row leaves it open.
  const all = hub.all_apps || {};
  const allC = all.choice?.channel || "";
  const allW = whenOf(all);
  const CH = [["release", "Release"], ["preview", "Preview"], ["nightly", "Nightly"]];
  const WH = [["auto", "Automatically"], ["night", "Automatically, at night"], ["manual", "People click Update"]];
  const allRow = `<div class="urow">
      <span class="row"><span class="app" style="background:#57534E" aria-hidden="true">★</span><b>All apps</b></span>
      <select class="inp" aria-label="All apps: which versions" data-act="hub-choice" data-id="*">
        ${[["", "Each person chooses"], ...CH].map(([v, n]) => `<option value="${v}" ${allC === v ? "selected" : ""}>${n}</option>`).join("")}
      </select>
      <select class="inp" aria-label="All apps: when they update" data-act="hub-when" data-id="*">
        ${[["", "Each person chooses"], ...WH].map(([v, n]) => `<option value="${v}" ${allW === v ? "selected" : ""}>${n}</option>`).join("")}
      </select>
      <span class="dim">Used by every app below set to “Same as all apps”</span>
      <span></span>
    </div>`;
  const row = ([id, name, colour, path]) => {
    const p = hub.products?.[id];
    const c = choiceOf(p);
    const fixed = p?.choice?.fixed;
    const w = whenOf(p);
    return `<div class="urow">
      <span class="row"><span class="app" style="background:${colour}" aria-hidden="true">${name[0]}</span><b>${name}</b></span>
      <select class="inp" aria-label="${name}: which versions" data-act="hub-choice" data-id="${id}">
        ${[["", id === "server" ? "Release" : allC ? "Same as all apps" : "Each person chooses"], ...(id === "server" ? [] : [["release", "Release"]]), ["preview", "Preview"], ["nightly", "Nightly"], ["fixed", `Fixed version${fixed ? `: ${esc(fixed)}` : "…"}`], ...(id === "server" ? [] : [["off", "Not allowed"]])].map(([v, n]) => `<option value="${v}" ${c === v ? "selected" : ""}>${n}</option>`).join("")}
      </select>
      <select class="inp" aria-label="${name}: when it updates" data-act="hub-when" data-id="${id}" ${c === "off" || c === "fixed" ? "disabled" : ""}>
        ${c === "fixed" ? `<option>Never — kept at ${esc(fixed)}</option>` : [["", id === "server" ? "Automatically" : allW ? "Same as all apps" : "Each person chooses"], ...(id === "server" ? [] : [["auto", "Automatically"]]), ["night", "Automatically, at night"], ["manual", id === "server" ? "When you click Update" : "People click Update"]].map(([v, n]) => `<option value="${v}" ${w === v ? "selected" : ""}>${n}</option>`).join("")}
      </select>
      <span>${id === "server" ? '<span class="dim">—</span>' : counts(id)}</span>
      <span class="dim">${path ? (st.hosting ? `<a href="${path}">${path}</a>` : "Not offered") : "—"}</span>
    </div>`;
  };
  const serverBox = !st.latest?.server
    ? ""
    : `<div class="banner"><span class="app" style="background:#1C1917" aria-hidden="true">S</span>
      <div class="grow"><b>This server: ${esc(cur)}${next && next !== cur ? ` → ${esc(next)} ready` : " · up to date"}</b>
      <div class="dim">${st.update_requested && st.update_requested !== cur ? `Moving to ${esc(st.update_requested)}: ${st.self_updating ? "it updates itself" : "Xenify Hub on this computer does it"}. ` : st.self_updating ? "It updates itself, as set in its row below. " : ""}No downtime: the new version takes over and open documents carry on.</div></div>
      ${next && next !== cur ? `<button class="btn main" data-act="hub-update-server" data-id="${esc(next)}">Update server now</button>` : ""}</div>`;
  return shell(`<h1>Apps &amp; updates</h1>${errorBox()}
    <p class="dim">Computers signed in to this server follow these choices in Xenify Hub${hub.mirror ? ". Updates download from this server, not the internet" : ""}.</p>
    ${serverBox}
    <div class="scroll"><div class="utable">
      <div class="urow head"><span>App</span><span>Which versions</span><span>When</span><span>On computers (${st.computers})</span><span>On this server's website</span></div>
      ${allRow}
      ${APPS.map(row).join("")}
      ${row(["server", "Server", "#1C1917", null])}
    </div></div>
    <div class="legend"><div><b>Release</b> — tested versions. Most people want this.</div><div><b>Preview</b> — next version early, may have rough edges.</div><div><b>Fixed version</b> — everyone stays on one version until you change it.</div></div>
    <section class="stack" aria-label="Downloads"><h2>Downloads</h2>
      <label class="check"><input type="checkbox" data-act="hub-mirror" ${hub.mirror ? "checked" : ""}>Keep copies of updates on this server (computers here download them once, from here)</label>
    </section>`);
}

async function hubChange(id, change) {
  return busy(async () => {
    if (id === "*") {
      const all = { choice: null, mode: null, window: null, ...(state.hub.all_apps || {}), ...change };
      await api("POST", "/v1/admin/hub", { ...state.hub, all_apps: all });
      state.hub = await api("GET", "/v1/admin/hub");
      state.hubStatus = await api("GET", "/v1/admin/hub/status");
      return toast("Saved");
    }
    const products = { ...(state.hub.products || {}) };
    const p = { allowed: true, required: false, choice: null, mode: null, window: null, ...(products[id] || {}), ...change };
    if (!p.choice && !p.mode && p.allowed && !p.required) delete products[id];
    else products[id] = p;
    await api("POST", "/v1/admin/hub", { ...state.hub, products });
    state.hub = await api("GET", "/v1/admin/hub");
    state.hubStatus = await api("GET", "/v1/admin/hub/status");
    toast("Saved");
  });
}

const mailOn = () => !!state.info?.features?.includes("mail");

/// People › whose sign-in needs a second step.
function mfaRuleSection() {
  const rules = [["off", "Nobody (each person can turn it on)"], ["admins", "Admins"], ["everyone", "Everyone"]];
  return `<section class="stack" aria-label="Two-step sign-in"><h2>Two-step sign-in</h2>
    <label class="row"><span class="dim">Who must sign in with a second step</span><select class="inp auto" aria-label="Who must sign in with a second step" data-act="require-mfa">${rules.map(([v, n]) => `<option value="${v}" ${(state.mfaRequired || "off") === v ? "selected" : ""}>${n}</option>`).join("")}</select></label>
    <p class="dim">They set it up the next time they sign in. Lost a phone? ⋯ › <b>Clear two-step sign-in</b>.</p></section>`;
}

/// Network › the mail server the server sends from.
function mailSection() {
  const f = state.form.mail || {};
  return `<section class="stack" aria-label="Outgoing mail"><h2>Outgoing mail</h2>
    <p class="dim">The server emails invitations and sign-in codes from here. Your email provider's settings for sending (SMTP) work, or a sending service.</p>
    <div class="row">${field("Mail server", "form.mail.host", { placeholder: "smtp.example.org" })}${field("Port", "form.mail.port", { type: "number", placeholder: f.security === "tls" ? "465" : f.security === "none" ? "25" : "587" })}</div>
    <label class="field"><span>Connection</span><select class="inp" data-bind="form.mail.security">${[["starttls", "STARTTLS (usually port 587)"], ["tls", "TLS (usually port 465)"], ["none", "None (a mail server on this computer)"]].map(([v, n]) => `<option value="${v}" ${f.security === v ? "selected" : ""}>${n}</option>`).join("")}</select></label>
    <div class="row">${field("User name", "form.mail.username", { auto: "off" })}${field(state.mailPasswordSet ? "Password (kept; type to change)" : "Password", "form.mail.password", { type: "password", auto: "new-password" })}</div>
    <div class="row">${field("From name", "form.mail.from_name", { placeholder: state.info?.name || "Office server" })}${field("From address", "form.mail.from_address", { type: "email", placeholder: "files@example.org" })}</div>
    <div class="row"><button class="btn" data-act="save-mail">Save</button><button class="btn" data-act="test-mail" ${mailOn() ? "" : "disabled"}>Send a test to me</button><span class="dim">${mailOn() ? "Set up" : "Not set up yet"}</span></div>
  </section>`;
}

/// Network › scanning uploads with ClamAV.
function scanSection() {
  const f = state.form.scan || {};
  const st = state.scan?.status;
  return `<section class="stack" aria-label="Virus scanning"><h2>Virus scanning</h2>
    <p class="dim">Every upload (files, pictures in documents, chat files) goes to ClamAV's scanner before it's kept; a file with a virus is refused. While the scanner doesn't answer, nothing can be uploaded.</p>
    <label class="check"><input type="checkbox" data-act="toggle-scan" ${f.on ? "checked" : ""}>Scan uploads for viruses</label>
    ${field("Scanner address", "form.scan.address", { placeholder: "127.0.0.1:3310 or /run/clamav/clamd.ctl" })}
    <div class="row"><button class="btn" data-act="save-scan">Save</button><button class="btn" data-act="test-scan">Test</button>${st ? `<span class="dim" ${st.ok ? "" : 'style="color:var(--bad)"'}>${esc(st.text)} · ${esc(ago(st.at))}</span>` : ""}</div>
  </section>`;
}

/// Network › how much each person can store.
function storageSection() {
  const free = state.storage?.free_gb;
  return `<section class="stack" aria-label="Storage"><h2>Storage</h2>
    ${field("Most each person can store (GB)", "form.quotaGb", { type: "number", placeholder: "No limit" })}
    <p class="dim">Counts the versions they save and the pictures and clips they put in Draw canvases. Empty: no limit.${free != null ? ` ${free} GB free on the server's disk.` : ""}</p>
    <div class="row"><button class="btn" data-act="save-storage">Save</button></div>
  </section>`;
}

/** How long chats keep messages (days; "0": for ever). */
const KEEP_CHOICES = [["0", "For ever"], ["30", "30 days"], ["90", "90 days"], ["365", "1 year"], ["custom", "A number of days…"]];

function networkPage() {
  const hs = state.https || {};
  const opts = [
    ["local", "This computer only", "Open them on the server itself."],
    ["lan", "Computers on this network", "Home or office addresses (192.168…, 10…) and VPNs, not the internet."],
    ["anywhere", "Anywhere", "Only over HTTPS. Use a strong admin password."],
  ];
  return shell(`<h1>Network</h1>${errorBox()}
    ${isAdmin() ? `<section class="stack" aria-label="Admin pages"><h2>Who can open these admin pages</h2>
      <p class="dim">Admins still sign in. The apps and files work wherever the server can be reached; this is only about admin pages.</p>
      <div class="radios">${opts.map(([v, n, why]) => `<label class="choice ${state.adminAccess === v ? "on" : ""}"><input type="radio" name="access" data-act="access" data-id="${v}" ${state.adminAccess === v ? "checked" : ""}><span><b>${n}</b>${v === "lan" ? '<span class="pill">Recommended</span>' : ""}<span class="why">${why}</span></span></label>`).join("")}</div>
    </section>` : ""}
    ${isAdmin() ? reachSection() : ""}
    ${netSection()}
    <section class="stack" aria-label="Domain name"><h2>Domain name</h2>
      <p class="dim">If this server can be reached from the internet at a domain name, it gets a free certificate from Let's Encrypt that every browser already trusts: nothing to install on each computer. It renews by itself.</p>
      ${hs.fixed ? `<p>Set when the server was started: <b>${esc(hs.domain)}</b>.</p>` : `${field("Domain name", "form.domain", { placeholder: "files.example.com" })}
      ${field("Email for warnings (optional)", "form.domainEmail", { type: "email", placeholder: "you@example.com", auto: "email" })}
      <div class="row"><button class="btn" data-act="save-domain" ${hs.port ? "" : "disabled"}>Save</button></div>`}
      ${httpsStatus(hs)}
      <p class="dim">The domain name must point at this computer's internet address, ${hs.port === 443 ? "with port 443 open to the internet" : `and the router must pass port 443 to port ${hs.port || 7443} on this computer (or set the HTTPS port to 443 above)`}.</p>
    </section>
    <section class="stack" aria-label="HTTPS on your network"><h2>HTTPS on your network</h2>
      <p class="dim">${hs.status === "ready" ? `Addresses other than ${esc(hs.domain)} (like this computer's network address) use the server's own certificate` : "Without a domain name, the server makes its own certificate"}, so people on your network can use HTTPS (browsers need it for the apps to work offline). Each computer trusts it once:</p>
      <div class="row"><a class="btn" href="/v1/ca.pem" download="xenify-server.crt">Download this server's certificate</a><span class="dim">then, on that computer:</span></div>
      <ul class="dim steps-list">
        <li><b>Windows:</b> double-click the file › Install certificate › Local machine › “Trusted Root Certification Authorities”.</li>
        <li><b>Mac:</b> double-click the file (Keychain Access opens) › double-click “Xenify server on …” › Trust › Always trust.</li>
        <li><b>Linux:</b> copy it to /usr/local/share/ca-certificates/ and run <code>sudo update-ca-certificates</code>; Firefox: Settings › Certificates › Import.</li>
        <li><b>Phones:</b> open the file, then allow it under the security settings (iPhone: Settings › General › About › Certificate Trust Settings).</li>
      </ul>
      <p class="dim">Then open the server at its HTTPS address (port ${hs.port || 7443}). The desktop apps don't need this: the first time they connect over HTTPS they ask whether to trust this server (or ignore it this time), showing this fingerprint to compare:</p>
      ${hs.authority ? `<p><code class="fingerprint">${esc(hs.authority.fingerprint)}</code></p>
      <p class="dim">${hs.authority.limited ? "The certificate can only vouch for names on your network and this computer's own addresses, so trusting it can't be used to pretend to be any other site." : "This certificate was made before certificates were limited to your network: trusting it would let anyone who copies it from this computer pretend to be any site. Make a new one (computers that trusted the old one must trust the new one)."}</p>
      <div class="row"><button class="btn${hs.authority.limited ? "" : " main"}" data-act="new-authority">Make a new certificate</button></div>` : ""}
    </section>
    <section class="stack" aria-label="Server name"><h2>Server name</h2>${field("What people see in the apps", "form.serverName", { placeholder: state.info?.name || "" })}<div class="row"><button class="btn" data-act="save-name">Save</button></div></section>
    ${mailSection()}
    ${scanSection()}
    ${storageSection()}
    <section class="stack" aria-label="Chat"><h2>Chat</h2>${field("Largest file people can send (MB)", "form.chatLimit", { type: "number" })}<p class="dim">Files, pictures and video clips sent in chats are kept in this server's data folder.</p><label class="field" for="f-keep"><span>Keep messages</span><select class="inp" id="f-keep" data-bind="form.keepChoice">${KEEP_CHOICES.map(([v, n]) => `<option value="${v}" ${state.form.keepChoice === v ? "selected" : ""}>${n}</option>`).join("")}</select></label>${state.form.keepChoice === "custom" ? field("Days", "form.keepDays", { type: "number", placeholder: "e.g. 180" }) : ""}<p class="dim">Older messages are deleted for everyone, with the files and recordings sent with them, about once an hour. Owners of a group or channel can choose less time for it, never more. Calendars, meetings and address books are kept.</p><label class="check"><input type="checkbox" data-act="toggle-threads" ${state.form.chatThreads ? "checked" : ""}>Threads</label><p class="dim">People can reply to one message in its thread, beside the chat. Off: replies go in the chat itself.</p><label class="check"><input type="checkbox" data-act="toggle-recording" ${state.form.callRecording ? "checked" : ""}>People can record calls</label><p class="dim">Starting a recording turns everyone's microphone, camera and shared screen off. Each person is only in it once they turn theirs back on. It's made on the recorder's computer and put in the call's chat.</p><div class="row"><button class="btn" data-act="save-chat-limit">Save</button></div></section>`);
}

/// Network › the address in emailed links, and a reverse proxy in front.
function reachSection() {
  if (!state.form.reach) return "";
  return `<section class="stack" aria-label="This server's address"><h2>This server's address</h2>
      ${field("Address in links sent by email", "form.reach.address", { placeholder: state.https?.domain ? `https://${state.https.domain}` : location.origin })}
      <p class="dim">Welcome emails, password codes and other links point here. Empty: ${state.https?.domain ? `https://${esc(state.https.domain)}` : "the address each one was made at"}.</p>
      ${field("Trusted proxies", "form.reach.trusted_proxies", { placeholder: "e.g. 127.0.0.1" })}
      <p class="dim">Only when a reverse proxy (nginx, Caddy) passes requests on to this server: its addresses or networks (10.0.0.0/8), separated by commas. The server then goes by the address the proxy says each request came from, for who can open these pages and for slowing down password guessing.</p>
      <div class="row"><button class="btn" data-act="save-reach">Save</button></div>
    </section>`;
}

function netSection() {
  const n = state.form.net;
  if (!n) return "";
  const fx = state.netFixed || {};
  const fixedNote = '<span class="dim">set when the server was started</span>';
  return `<section class="stack" aria-label="How the server is reached"><h2>How the server is reached</h2>
      <label class="field" for="f-net-http"><span>Port</span><input class="inp narrow-num" id="f-net-http" type="number" min="1" max="65535" data-bind="form.net.http_port" value="${esc(n.http_port)}" ${fx.http ? "disabled" : ""}></label>
      ${fx.http ? fixedNote : ""}
      <label class="check"><input type="checkbox" data-act="net-https" ${n.https_port ? "checked" : ""} ${fx.https ? "disabled" : ""}>HTTPS</label>
      ${n.https_port ? `<label class="field" for="f-net-https"><span>HTTPS port</span><input class="inp narrow-num" id="f-net-https" type="number" min="1" max="65535" data-bind="form.net.https_port" value="${esc(n.https_port)}" ${fx.https ? "disabled" : ""}></label>` : '<p class="dim">Turn HTTPS off only when something in front of the server (a reverse proxy) does it.</p>'}
      ${fx.https ? fixedNote : ""}
      <label class="check"><input type="checkbox" data-act="net-flag" data-id="announce" ${n.announce ? "checked" : ""} ${fx.announce ? "disabled" : ""}>Let apps on this network find the server</label>
      <h3>Apps in the browser</h3>
      <p class="dim">Write, Draw, Calc, Plan and Chat open in the browser at this server's address, for people without Xenify Hub (on a phone, say). They sign in to this server by themselves.</p>
      ${state.netApps ? `<p><a href="/" target="_blank" rel="noopener">Open them</a></p>` : webAppsGet()}
      <div class="row"><button class="btn" data-act="save-net">Save</button></div>
    </section>
`;
}

// The apps for the browser aren't here yet: download them from Xenify's
// release list (checked against its signature, like Xenify Hub does).
function webAppsGet() {
  const job = state.netJob;
  if (job === "downloading" || !job) return '<div class="row"><span class="dim" role="status">Getting the apps for the browser…</span></div>';
  return `<p class="err" role="alert">Couldn't get them yet: ${esc(job)}. The server tries again every few hours.</p>
    <div class="row"><button class="btn" data-act="net-get-apps">Try again now${state.netAppsSize ? ` (${size(state.netAppsSize)})` : ""}</button></div>`;
}

async function loadNet() {
  if (isAdmin()) state.form.reach = await api("GET", "/v1/admin/network/reach");
  const r = await api("GET", "/v1/admin/network");
  state.form.net = { ...r.net };
  state.net = { ...r.net };
  state.netFixed = r.fixed;
  state.netApps = r.apps_available;
  state.netJob = r.apps_job;
  state.netAppsSize = r.apps_size;
  pollWebApps();
}

// While the apps for the browser download, look again every few seconds.
function pollWebApps() {
  clearTimeout(pollWebApps.timer);
  if (state.netJob !== "downloading" && (state.netApps || state.netJob)) return;
  pollWebApps.timer = setTimeout(async () => {
    if (state.view !== "network") return;
    try {
      await loadNet();
    } catch {}
    render();
    if (state.netApps) toast("The apps are ready in the browser");
  }, 3000);
}

function httpsStatus(hs) {
  if (!hs.port) return '<p class="dim">This server was started without HTTPS.</p>';
  if (!hs.status) return "";
  if (hs.status === "ready") return `<p>Certificate ready: <a href="https://${esc(hs.domain)}${hs.port === 443 ? "" : `:${hs.port}`}/">https://${esc(hs.domain)}</a></p>`;
  if (hs.status === "waiting") return '<p class="dim">Getting a certificate from Let\'s Encrypt…</p>';
  return `<p class="err" role="alert">Let's Encrypt couldn't give a certificate yet: ${esc(hs.status)}</p>`;
}

async function loadHttps() {
  state.https = await api("GET", "/v1/admin/https");
  state.form.domain = state.https.domain;
  state.form.domainEmail = state.https.email;
  pollHttps();
}

// While Let's Encrypt is still being asked, look again every few seconds.
function pollHttps() {
  clearTimeout(pollHttps.timer);
  if (state.https?.status !== "waiting") return;
  pollHttps.timer = setTimeout(async () => {
    if (state.view !== "network") return;
    try {
      state.https = await api("GET", "/v1/admin/https");
    } catch {}
    render();
    pollHttps();
  }, 5000);
}

function accountPage() {
  const me = state.session?.account || {};
  return shell(`<h1>Your account</h1>${errorBox()}
    <p>Signed in as <b>${esc(nameOf(me))}</b>${me.email && me.display_name ? ` (${esc(me.email)})` : ""}.</p>
    <form class="stack" data-act="change-password"><h2>Change password</h2>
      ${field("Current password", "form.old", { type: "password", auto: "current-password" })}
      ${field("New password", "form.password", { type: "password", auto: "new-password" })}
      ${field("New password again", "form.password2", { type: "password", auto: "new-password" })}
      <div class="row"><button class="btn main" type="submit">Change password</button></div></form>
    ${mfaSection()}
    <div class="row"><button class="btn" data-act="sign-out">Sign out</button><button class="btn" data-act="sign-out-all">Sign out everywhere</button></div>`);
}

function mfaSection() {
  const st = state.mfaState;
  if (!st) return "";
  const on = st.app || st.email;
  const how = st.app ? "with an authenticator app" : "with codes by email";
  return `<section class="stack" aria-label="Two-step sign-in"><h2>Two-step sign-in</h2>
    <p class="dim">${on ? `On, ${how}. ${st.backup_left} backup code${st.backup_left === 1 ? "" : "s"} left.` : "Off. After your password, you'd also give a code, so a stolen password isn't enough."}${st.required ? " This server requires it." : ""}</p>
    <div class="row">${on ? `<button class="btn" data-act="mfa-backup">New backup codes</button>${st.required ? "" : '<button class="btn" data-act="mfa-off">Turn off</button>'}` : '<button class="btn main" data-act="mfa-setup">Set up</button>'}</div></section>`;
}

// The roles part of the person dialog: Admin, then the server's own roles
// (Everyone has the Everyone role anyway). Only admins give out Admin, or
// roles with parts of running the server.
function personRoles(d) {
  const roles = (state.serverRoles || []).filter((r) => !r.everyone);
  if (!roles.length) return "";
  const box = (r) => {
    const on = r.admin ? d.admin : d.roles.includes(r.id);
    const locked = !isAdmin() && (r.admin || r.perms.some((p) => p.startsWith("admin.")));
    const about = r.admin ? "can do everything, also make admins" : r.about || r.perms.length ? r.about || `${r.perms.length} permission${r.perms.length === 1 ? "" : "s"}` : "";
    return `<label class="check"><input type="checkbox" data-act="dialog-role" data-id="${r.id}" ${on ? "checked" : ""} ${locked ? "disabled" : ""}>${esc(r.name)}${about ? ` <span class="dim">— ${esc(about)}</span>` : ""}</label>`;
  };
  return `<fieldset class="stack"><legend class="dim">Roles (everyone also has Everyone's)</legend>${roles.map(box).join("")}</fieldset>`;
}

function dialogBox() {
  const d = state.dialog;
  if (!d) return "";
  let inner = "";
  if (d.kind === "person") {
    inner = `<h2>${d.id ? "Change" : "Add a person"}</h2>${errorBox()}
      ${field("Name", "dialog.display_name", { placeholder: "Priya Shah" })}
      ${field("Email (they sign in with it)", "dialog.email", { type: "email", placeholder: "priya@example.org" })}
      <fieldset class="stack"><legend class="dim">Groups</legend>${state.groups.map((g) => `<label class="check"><input type="checkbox" data-act="dialog-group" data-id="${g.id}" ${d.groups.includes(g.id) ? "checked" : ""}>${esc(g.name)}</label>`).join("") || '<span class="dim">No groups yet.</span>'}</fieldset>
      ${personRoles(d)}
      ${d.id ? "" : `<p class="dim">${mailOn() ? "With an email address, they get an invitation and pick their password when they first sign in." : `This server can't send email yet (<button class="link" type="button" data-act="to-mail">set it up</button>), so you get a code to give them: they sign in with their email address and that code, then pick a password.`}</p>`}
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="save-person">${d.id ? "Save" : "Add"}</button></div>`;
  } else if (d.kind === "mfa") {
    inner = mfaHtml();
  } else if (d.kind === "mfa-password") {
    inner = `<form class="stack" data-act="mfa-password"><h2>${esc(d.title)}</h2>${d.text ? `<p>${esc(d.text)}</p>` : ""}${errorBox()}
      ${field("Your password", "form.mfaPassword", { type: "password", auto: "current-password" })}
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit">${esc(d.yes)}</button></div></form>`;
  } else if (d.kind === "code") {
    // A mailto hands the link to the admin's own mail program.
    const mail = d.mail ? `mailto:${encodeURIComponent(d.mail.to)}?subject=${encodeURIComponent(d.mail.subject)}&body=${encodeURIComponent(d.mail.body)}` : "";
    inner = `<h2>${esc(d.title)}</h2><p>${d.text}</p><div class="code">${codeHtml(d.code)}</div>
      <p class="dim">Send it privately, by email or chat: whoever has it can sign in as them. Once you close this, it can't be shown again.</p>
      <div class="row">${mail ? `<a class="btn" href="${esc(mail)}">Write an email</a>` : ""}<button class="btn" data-act="copy" data-text="${esc(d.copy || d.code)}">Copy</button><span class="grow"></span><button class="btn main" data-act="close">Done</button></div>`;
  } else if (d.kind === "group") {
    inner = `<h2>${d.id ? "Change group" : "New group"}</h2>${errorBox()}
      ${field("Name", "dialog.name", { placeholder: "Design" })}
      <fieldset class="stack"><legend class="dim">People in it</legend>${state.people.filter((p) => !p.disabled).map((p) => `<label class="check"><input type="checkbox" data-act="dialog-member" data-id="${p.id}" ${d.members.includes(p.id) ? "checked" : ""}>${esc(nameOf(p))}</label>`).join("")}</fieldset>
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="save-group">${d.id ? "Save" : "Make group"}</button></div>`;
  } else if (d.kind === "space") {
    inner = `<h2>${d.id ? "Rename space" : "New space"}</h2>${errorBox()}${field("Name", "dialog.name", { placeholder: "Design" })}
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="save-space">${d.id ? "Save" : "Make space"}</button></div>`;
  } else if (d.kind === "role-name") {
    inner = `<form class="stack" data-act="confirm-yes"><h2>${esc(d.title)}</h2>${errorBox()}${field("Name", "dialog.name", { placeholder: d.placeholder })}
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit">${esc(d.yes)}</button></div></form>`;
  } else if (d.kind === "confirm") {
    inner = `<h2>${esc(d.title)}</h2><p>${esc(d.text)}</p><div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main" data-act="confirm-yes">${esc(d.yes)}</button></div>`;
  }
  return `<div class="backdrop" data-act="backdrop"><div class="dialog" role="dialog" aria-modal="true">${inner}</div></div>`;
}

function render() {
  const active = document.activeElement;
  const focus = active?.id;
  let caret = null;
  try {
    if (focus && typeof active.selectionStart === "number") caret = [active.selectionStart, active.selectionEnd];
  } catch {}
  const v = state.view;
  if (v !== "loading") document.title = v === "signin" || v === "recover" || v === "join" ? "Sign in" : state.info?.name || "Xenify server";
  const page =
    v === "loading" ? "" : v === "setup" ? setupPage() : v === "signin" ? signinPage() : v === "recover" ? recoverPage() : v === "join" ? joinPage()
    : v === "people" ? peoplePage() : v === "spaces" ? spacesPage() : v === "links" ? linksPage() : v === "ai" ? aiPage() : v === "data" ? dataPage() : v === "holidays" ? planPage() : v === "roles" ? rolesPage() : v === "updates" ? updatesPage() : v === "backups" ? backupsPage() : v === "email" ? emailPage() : v === "network" ? networkPage() : accountPage();
  document.getElementById("app").innerHTML = page + dialogBox() + (state.toast ? `<div class="toast" role="status">${esc(state.toast)}</div>` : "");
  if (focus) {
    const el = document.getElementById(focus);
    if (el) {
      el.focus();
      try {
        if (caret && el.setSelectionRange) el.setSelectionRange(caret[0], caret[1]);
      } catch {}
    }
  }
  focusOpened();
  placeMenu();
}

// A dialog that just opened: its first field (or button) has the keys, so
// typing goes straight in.
let shownDialog = null;
function focusOpened() {
  const opened = state.dialog && state.dialog !== shownDialog;
  shownDialog = state.dialog;
  if (!opened) return;
  const box = document.querySelector(".dialog");
  const el = box?.querySelector("[autofocus], input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([readonly]):not([disabled]), textarea, select") || box?.querySelector(".btn.main, button");
  el?.focus();
}

// An open ⋯ menu sits above the page: a table that scrolls sideways would
// otherwise cut it off. It opens below its button, or above near the bottom.
function placeMenu() {
  const menu = document.querySelector(".menu");
  const button = menu?.parentElement.querySelector(".more");
  if (!button) return;
  const b = button.getBoundingClientRect();
  const h = menu.offsetHeight;
  menu.style.position = "fixed";
  menu.style.left = `${Math.max(8, b.right - menu.offsetWidth)}px`;
  menu.style.right = "auto";
  menu.style.top = `${b.bottom + 4 + h > innerHeight - 8 ? Math.max(8, b.top - 4 - h) : b.bottom + 4}px`;
}
addEventListener("resize", () => state.menu && ((state.menu = null), render()));
addEventListener("scroll", placeMenu, true);

function toast(msg) {
  state.toast = msg;
  render();
  setTimeout(() => {
    if (state.toast === msg) {
      state.toast = "";
      render();
    }
  }, 2600);
}

// ---- What clicks do -------------------------------------------------------------

function passwordProblem(p, again) {
  if ((p || "").length < 8) return "Use a password of at least 8 characters.";
  if (p !== again) return "The two passwords aren't the same.";
  return "";
}

async function busy(f) {
  if (state.busy) return;
  state.busy = true;
  state.error = "";
  render();
  try {
    await f();
  } catch (e) {
    state.error = e.message;
  } finally {
    state.busy = false;
    render();
  }
}

const acts = {
  "setup-finish"() {
    return busy(async () => {
      const f = state.form;
      if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test((f.email || "").trim())) throw new Error("Type your email address: you sign in with it.");
      const p = passwordProblem(f.password, f.password2);
      if (p) throw new Error(p);
      const s = await api("POST", "/v1/setup", {
        token: state.setup.token,
        server_name: "",
        display_name: f.display_name || "",
        email: (f.email || "").trim(),
        password: f.password,
        device: "Admin pages",
      });
      openSession(s);
      state.form = {};
      state.info = await api("GET", "/v1/info");
      history.replaceState(null, "", "#/people");
      go("people");
    });
  },
  async copy(el) {
    try {
      await navigator.clipboard.writeText(el.dataset.text);
      toast("Copied");
    } catch {
      toast("Select it and copy it yourself: this browser won't let pages copy here");
    }
  },
  signin() {
    return busy(async () => {
      const f = state.form;
      const username = (f.username || "").trim().toLowerCase();
      if (!f.stage || f.stage === "user") {
        if (!username) throw new Error("Type your email address.");
        let step = { next: "password" };
        try {
          step = (await api("POST", "/v1/login/start", { username })) || step;
        } catch (e) {
          if (!/404|405/.test(e.message)) throw e;
        }
        if (step.next !== "pick_password") return void (f.stage = "password");
        if (step.can_email) return emailCode("We’ve emailed you a code. Type it here, then pick your password.");
        f.note = "This account doesn’t have a password yet. Type the code your admin gave you, then pick one.";
        f.stage = "code";
        return;
      }
      if (f.stage === "code") {
        const code = (f.code || "").trim();
        if (!code) throw new Error("Type the code.");
        const l = await api("POST", "/v1/link", { link: code, username });
        Object.assign(f, { stage: "pick", kind: l.kind, password: "", password2: "" });
        return;
      }
      let s;
      if (f.stage === "pick") {
        const p = passwordProblem(f.password, f.password2);
        if (p) throw new Error(p);
        s = await api("POST", "/v1/link/use", { link: f.code.trim(), username, password: f.password, device: "Web browser" });
      } else s = await api("POST", "/v1/login", { username, password: f.password || "", device: "Admin pages", remember: remembered() });
      if (s.ticket) {
        // A second step first: a code, or setting one up.
        state.mfa = { ticket: s.ticket, methods: s.methods, setup: s.setup, setup_methods: s.setup_methods, method: s.methods[0] || "", stage: s.setup ? "choose" : "code" };
        state.form = { remember: true };
        // Only an app to choose from: straight to it.
        if (s.setup && s.setup_methods.length === 1) {
          state.mfa.app = await api("POST", "/v1/mfa/app/start", { ticket: s.ticket });
          state.mfa.stage = "app";
        }
        return;
      }
      await signedInAs(s);
    });
  },
  "mfa-remember"(el) {
    state.form.remember = el.checked;
  },
  "mfa-method"(el) {
    state.mfa.method = el.dataset.id;
    state.error = "";
    if (state.mfa.method === "email") return acts["mfa-email"]();
    render();
  },
  "mfa-email"() {
    return busy(async () => {
      const m = state.mfa;
      await api("POST", m.ticket ? (m.stage === "code" ? "/v1/login/email" : "/v1/mfa/email") : "/v1/mfa/email", m.ticket ? { ticket: m.ticket } : {});
      if (m.stage !== "code") m.stage = "email";
      toast("Code sent: look in your email");
    });
  },
  "mfa-code"() {
    return busy(async () => {
      const m = state.mfa;
      const code = state.form.mfaCode || "";
      const s = await api("POST", "/v1/login/code", { ticket: m.ticket, method: m.method, code, remember: !!state.form.remember, device: "Admin pages" });
      await signedInAs(s);
    });
  },
  "mfa-pick"(el) {
    return busy(async () => {
      const m = state.mfa;
      if (el.dataset.id === "app") {
        m.app = await api("POST", "/v1/mfa/app/start", m.ticket ? { ticket: m.ticket } : {});
        m.stage = "app";
      } else {
        await api("POST", "/v1/mfa/email", m.ticket ? { ticket: m.ticket } : {});
        m.stage = "email";
        toast("Code sent: look in your email");
      }
    });
  },
  "mfa-finish"() {
    return busy(async () => {
      const m = state.mfa;
      const body = { code: state.form.mfaCode || "", remember: !!state.form.remember, ...(m.ticket ? { ticket: m.ticket } : {}) };
      const r = await api("POST", m.stage === "app" ? "/v1/mfa/app/finish" : "/v1/mfa/email", body);
      m.codes = r.backup_codes;
      m.session = r.session;
      m.stage = "codes";
      state.form.mfaCode = "";
    });
  },
  "mfa-kept"() {
    return busy(async () => {
      const m = state.mfa;
      state.mfa = null;
      state.dialog = null;
      if (m.session) return signedInAs(m.session);
      await load();
    });
  },
  "mfa-cancel"() {
    state.mfa = null;
    state.dialog = null;
    state.form = {};
    render();
  },
  "mfa-setup"() {
    const st = state.mfaState;
    state.mfa = { stage: "choose", setup_methods: st.setup_methods };
    state.dialog = { kind: "mfa" };
    if (st.setup_methods.length === 1) return acts["mfa-pick"]({ dataset: { id: "app" } });
    render();
  },
  "mfa-off"() {
    state.dialog = { kind: "mfa-password", title: "Turn two-step sign-in off?", yes: "Turn off", path: "/v1/me/mfa/off" };
    state.form.mfaPassword = "";
    render();
  },
  "mfa-backup"() {
    state.dialog = { kind: "mfa-password", title: "New backup codes?", yes: "Make new codes", path: "/v1/me/mfa/backup", text: "The codes you have now stop working." };
    state.form.mfaPassword = "";
    render();
  },
  "mfa-password"() {
    return busy(async () => {
      const d = state.dialog;
      const r = await api("POST", d.path, { password: state.form.mfaPassword || "" });
      state.form.mfaPassword = "";
      if (r?.backup_codes) {
        state.mfa = { stage: "codes", codes: r.backup_codes };
        state.dialog = { kind: "mfa" };
      } else {
        state.dialog = null;
        toast("Two-step sign-in is off");
        await load();
      }
    });
  },
  "require-mfa"(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/settings", { mfa_required: el.value });
      state.mfaRequired = el.value;
      toast("Saved");
    });
  },
  "clear-mfa"(el) {
    const p = state.people.find((x) => x.id === +el.dataset.id);
    state.menu = null;
    state.dialog = { kind: "confirm", title: `Clear ${nameOf(p)}'s two-step sign-in?`, text: "For a lost phone: they sign in with just their password, and set it up again if this server requires it.", yes: "Clear it", then: async () => {
      await api("DELETE", `/v1/admin/people/${p.id}/mfa`);
      state.dialog = null;
      toast("Cleared");
    } };
    render();
  },
  "signin-back"() {
    state.form = { username: state.form.username };
    state.error = "";
    render();
  },
  "signin-email"() {
    return busy(() => emailCode("A new code is on its way to your email."));
  },
  "to-recover"() {
    if (mailOn()) return busy(() => emailCode("If this account has an email address, a code is on its way to it. Type it here, then pick a new password."));
    state.form = {};
    go("recover");
  },
  "to-mail"() {
    state.dialog = null;
    go("network");
    setTimeout(() => document.querySelector('[aria-label="Outgoing mail"]')?.scrollIntoView({ block: "start" }), 300);
  },
  "to-signin"() {
    state.form = {};
    go("signin");
  },
  join() {
    return busy(async () => {
      const f = state.form;
      const p = passwordProblem(f.password, f.password2);
      if (p) throw new Error(p);
      const as = state.link.email || state.link.username;
      const s = await api("POST", "/v1/link/use", { link: state.joinCode, password: f.password, device: "Web browser" });
      // Signed in already: straight to their files (a second step first, if the server asks).
      if (s.token) {
        openSession(s);
        return void location.assign("/files/");
      }
      state.form = { joined: as };
    });
  },
  tab(el) {
    state.tab = el.dataset.id;
    render();
  },
  menu(el) {
    state.menu = state.menu === el.dataset.id ? null : el.dataset.id;
    render();
  },
  "add-person"() {
    state.dialog = { kind: "person", display_name: "", email: "", groups: [], roles: [], admin: false };
    state.error = "";
    render();
  },
  "edit-person"(el) {
    const p = state.people.find((x) => x.id === +el.dataset.id);
    state.menu = null;
    const roles = (state.serverRoles || []).filter((r) => !r.admin && r.members?.some((m) => m.user === p.id)).map((r) => r.id);
    state.dialog = { kind: "person", id: p.id, display_name: p.display_name, email: p.email || "", groups: [...p.groups], roles, admin: p.admin, wasAdmin: p.admin };
    render();
  },
  "dialog-group"(el) {
    const id = +el.dataset.id;
    const g = state.dialog.groups;
    state.dialog.groups = el.checked ? [...g, id] : g.filter((x) => x !== id);
  },
  "dialog-role"(el) {
    const d = state.dialog, id = +el.dataset.id;
    if (id === 0) d.admin = el.checked;
    else d.roles = el.checked ? [...d.roles, id] : d.roles.filter((r) => r !== id);
  },
  "dialog-member"(el) {
    const id = +el.dataset.id;
    const m = state.dialog.members;
    state.dialog.members = el.checked ? [...m, id] : m.filter((x) => x !== id);
  },
  "save-person"() {
    return busy(async () => {
      const d = state.dialog;
      if (d.id) {
        const change = { display_name: d.display_name, email: (d.email || "").trim(), groups: d.groups, roles: d.roles };
        if (isAdmin() && d.admin !== d.wasAdmin) change.admin = d.admin;
        await api("POST", `/v1/admin/people/${d.id}`, change);
        state.dialog = null;
      } else {
        const email = (d.email || "").trim().toLowerCase();
        if (!email) throw new Error("Type their email address: they sign in with it.");
        const made = await api("POST", "/v1/admin/people", { email, display_name: d.display_name, admin: d.admin, groups: d.groups, roles: d.roles });
        const p = { email, display_name: d.display_name };
        if (made.emailed) {
          state.dialog = null;
          toast(`Added. ${nameOf(p)} gets an invitation by email`);
        } else state.dialog = codeDialog(p, made, "join");
      }
      await load();
    });
  },
  reset(el) {
    const p = state.people.find((x) => x.id === +el.dataset.id);
    state.menu = null;
    state.dialog = { kind: "confirm", title: `Reset ${nameOf(p)}'s password?`, text: `They're signed out everywhere and pick a new password when they next sign in${mailOn() && p.email ? ", with a code sent to their email" : ", with a code you give them"}.`, yes: "Reset password", then: async () => {
      const made = await api("POST", `/v1/admin/people/${p.id}/reset`);
      if (made.emailed) {
        state.dialog = null;
        toast(`${nameOf(p)} is told by email to pick a new password`);
        return load();
      }
      state.dialog = codeDialog(p, made, "reset");
      await load();
    } };
    render();
  },
  "toggle-off"(el) {
    const p = state.people.find((x) => x.id === +el.dataset.id);
    state.menu = null;
    if (p.disabled) return busy(async () => (await api("POST", `/v1/admin/people/${p.id}`, { disabled: false }), load()));
    state.dialog = { kind: "confirm", title: `Turn ${nameOf(p)} off?`, text: "They're signed out and can't sign in until you turn them back on. Their files stay.", yes: "Turn off", then: async () => {
      await api("POST", `/v1/admin/people/${p.id}`, { disabled: true });
      state.dialog = null;
      await load();
    } };
    render();
  },
  "add-group"() {
    state.dialog = { kind: "group", name: "", members: [] };
    render();
  },
  "edit-group"(el) {
    const g = state.groups.find((x) => x.id === +el.dataset.id);
    state.dialog = { kind: "group", id: g.id, name: g.name, members: [...g.members] };
    render();
  },
  "save-group"() {
    return busy(async () => {
      const d = state.dialog;
      await api("POST", d.id ? `/v1/admin/groups/${d.id}` : "/v1/admin/groups", { name: d.name, members: d.members });
      state.dialog = null;
      await load();
    });
  },
  "delete-group"(el) {
    const g = state.groups.find((x) => x.id === +el.dataset.id);
    state.dialog = { kind: "confirm", title: `Remove the group ${g.name}?`, text: "People in it stay; they lose what the group gave them.", yes: "Remove group", then: async () => {
      await api("DELETE", `/v1/admin/groups/${g.id}`);
      state.dialog = null;
      await load();
    } };
    render();
  },
  "pick-space"(el) {
    state.spaceSel = +el.dataset.id;
    busy(loadAccess);
  },
  "add-space"() {
    state.dialog = { kind: "space", name: "" };
    render();
  },
  "rename-space"() {
    state.dialog = { kind: "space", id: state.spaceSel, name: state.names[state.spaceSel] };
    render();
  },
  "save-space"() {
    return busy(async () => {
      const d = state.dialog;
      const name = (d.name || "").trim();
      if (!name) throw new Error("Give the space a name.");
      const meta = nameMeta(name);
      if (d.id) await api("POST", `/v1/spaces/${d.id}`, { name: meta });
      else state.spaceSel = (await api("POST", "/v1/spaces", { name: meta })).id;
      state.dialog = null;
      await load();
    });
  },
  "delete-space"() {
    const id = state.spaceSel;
    state.dialog = { kind: "confirm", title: `Remove ${state.names[id]}?`, text: "Everything in it goes, for everyone. This can't be undone.", yes: "Remove space", then: async () => {
      await api("DELETE", `/v1/spaces/${id}`);
      state.dialog = null;
      state.spaceSel = null;
      await load();
    } };
    render();
  },
  "add-grant"() {
    const w = state.form.grantWho;
    if (!w) return;
    const who = w === "everyone" ? "everyone" : w[0] === "g" ? { group: +w.slice(1) } : { user: +w.slice(1) };
    const same = (a) => JSON.stringify(a.who) === JSON.stringify(who);
    if (!state.access.grants.some(same)) state.access.grants.push({ who, role: "editor" });
    state.form.grantWho = "";
    saveAccess();
  },
  "grant-role"(el) {
    const g = state.access.grants[+el.dataset.id];
    const c = (state.docRoles || []).find((r) => `c${r.id}` === el.value);
    if (c) Object.assign(g, { role: c.base, custom: c.id });
    else {
      g.role = el.value;
      delete g.custom;
    }
    saveAccess();
  },
  "remove-grant"(el) {
    state.access.grants.splice(+el.dataset.id, 1);
    saveAccess();
  },
  "comfy-check"() {
    return busy(async () => {
      state.comfy = await api("GET", "/v1/admin/ai/comfy");
    });
  },
  "ai-save"() {
    return busy(async () => {
      const a = state.form.ai;
      state.ai = await api("POST", "/v1/admin/ai", { runner: a.runner, url: (a.url || "").trim(), key: a.key || "", models_dir: (a.models_dir || "").trim(), comfy_url: (a.comfy_url || "").trim(), hf_token: a.hf_token || "", civitai_token: a.civitai_token || "", giphy_key: (a.giphy_key || "").trim() });
      state.form.ai = { ...state.ai };
      toast(state.ai.cleared?.length ? "Saved. The address changed, so the kept key was cleared: type the key again for the new address" : "Saved");
      state.aiStatus = await api("GET", "/v1/admin/ai/status");
      state.comfy = await api("GET", "/v1/admin/ai/comfy").catch(() => null);
    });
  },
  "giphy-forget"() {
    return busy(async () => {
      // An empty key keeps the old one: a space clears it.
      state.ai = await api("POST", "/v1/admin/ai", { giphy_key: " " });
      state.form.ai = { ...state.ai };
      toast("GIFs are no longer offered");
    });
  },
  "mcp-add"() {
    return busy(async () => {
      const f = state.form.mcp;
      const headers = f.header?.trim() ? { [f.header.trim()]: f.value || "" } : {};
      await api("POST", "/v1/admin/mcp", { name: f.name, url: f.url, headers, changes: !!f.changes });
      state.form.mcp = { name: "", url: "", header: "", value: "" };
      state.mcpTools = await api("GET", "/v1/admin/mcp");
      toast("Added: Calc can use it when people ask");
    });
  },
  "mcp-new-changes"(el) {
    state.form.mcp.changes = el.checked;
  },
  "mcp-changes"(el) {
    const t = (state.mcpTools || []).find((x) => String(x.id) === el.dataset.id);
    if (!t) return;
    return busy(async () => {
      await api("POST", "/v1/admin/mcp", { id: t.id, name: t.name, url: t.url, headers: {}, changes: el.checked });
      state.mcpTools = await api("GET", "/v1/admin/mcp");
      toast(el.checked ? "Its tools that change things can be used" : "Only its tools that read can be used");
    });
  },
  "ai-limits"() {
    const l = state.form.ai.limits || {};
    const limits = Object.fromEntries(LIMITS.map(([k]) => [k, Math.max(0, Math.round(Number(l[k]) || 0))]));
    return aiChange({ limits }).then(() => toast("Limits saved"));
  },
  "mcp-remove"(el) {
    return busy(async () => {
      await api("DELETE", `/v1/admin/mcp/${el.dataset.id}`);
      state.mcpTools = await api("GET", "/v1/admin/mcp");
    });
  },
  "ai-thinking"(el) {
    return aiChange({ data_thinking: el.value });
  },
  "ai-default"(el) {
    return aiChange({ [`${el.dataset.kind}_model`]: el.dataset.model });
  },
  "ai-offer"(el) {
    // Only a model from the list (not something half typed).
    if (!el.value || ![...(el.list?.options || [])].some((o) => o.value === el.value)) return;
    const offered = { ...(state.ai.offered || {}) };
    offered[el.dataset.kind] = [...(offered[el.dataset.kind] || []), el.value];
    const change = { offered };
    if (!state.ai[`${el.dataset.kind}_model`] && el.dataset.kind !== "lora") change[`${el.dataset.kind}_model`] = el.value;
    return aiChange(change);
  },
  "ai-unoffer"(el) {
    const k = el.dataset.kind;
    const offered = { ...(state.ai.offered || {}) };
    offered[k] = (offered[k] || []).filter((m) => m !== el.dataset.model);
    const change = { offered };
    if (state.ai[`${k}_model`] === el.dataset.model) change[`${k}_model`] = offered[k][0] || "";
    // A style is offered as a picture model too.
    if (k === "lora") {
      offered.image = (offered.image || []).filter((m) => m !== el.dataset.model);
      if (state.ai.image_model === el.dataset.model) change.image_model = offered.image[0] || "";
    }
    return aiChange(change);
  },
  "style-open"() {
    return busy(async () => {
      const parts = await api("GET", "/v1/admin/ai/styles");
      state.style = { name: "", base: parts.bases[0] || "", lora: parts.loras[0] || "", strength: "1", parts };
    });
  },
  "style-close"() {
    state.style = null;
    render();
  },
  "style-make"() {
    const st = state.style;
    return busy(async () => {
      state.ai = await api("POST", "/v1/admin/ai/styles", { name: st.name, base: st.base, lora: st.lora, strength: parseFloat(st.strength) || 1 });
      state.form.ai = { ...state.form.ai, offered: state.ai.offered };
      state.style = null;
    });
  },
  "ai-adult"(el) {
    return aiChange({ adult: el.checked });
  },
  "ai-address"(el) {
    Object.assign(state.form.ai, { runner: el.dataset.runner, url: el.dataset.url });
    render();
  },
  "ai-source"(el) {
    const f = state.aiFind;
    f.source = el.dataset.id;
    if (f.source === "civitai" && !["image", "lora"].includes(f.kind)) f.kind = "image";
    if (f.source === "hf" && f.kind === "lora") f.kind = "text";
    f.hits = null;
    render();
  },
  "ai-search"() {
    return busy(async () => {
      const f = state.aiFind;
      f.open = null;
      f.hits = await api("GET", `/v1/admin/ai/search?source=${f.source}&kind=${f.kind}&q=${encodeURIComponent(f.q)}`);
    });
  },
  "ai-files"(el) {
    const f = state.aiFind;
    if (f.open === el.dataset.id) {
      f.open = null;
      return render();
    }
    return busy(async () => {
      f.files = await api("GET", `/v1/admin/ai/files?source=${f.source}&id=${encodeURIComponent(el.dataset.id)}`);
      f.open = el.dataset.id;
    });
  },
  "ai-install"(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/ai/install", { source: state.aiFind.source, id: el.dataset.id, file: el.dataset.file, kind: el.dataset.kind });
      state.jobs = await api("GET", "/v1/admin/ai/jobs");
      followJobs();
    });
  },
  "ai-stop"(el) {
    return busy(async () => {
      await api("DELETE", `/v1/admin/ai/jobs/${el.dataset.id}`);
      state.jobs = await api("GET", "/v1/admin/ai/jobs");
    });
  },
  access(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/settings", { admin_access: el.dataset.id });
      state.adminAccess = el.dataset.id;
      toast("Saved");
    });
  },
  "hub-choice"(el) {
    const v = el.value;
    const id = el.dataset.id;
    if (v === "off") return hubChange(id, { allowed: false, choice: null, mode: null, window: null });
    if (v === "") return hubChange(id, { allowed: true, choice: null });
    if (v === "fixed") {
      return (async () => {
        try { state.hubStatus = await api("GET", "/v1/admin/hub/status"); } catch {}
        return acts["hub-fixed"](id);
      })();
    }
    return hubChange(id, { allowed: true, choice: { channel: v } });
  },
  "hub-fixed"(id) {
    // The version most computers have now (else the newest).
    const seen = Object.entries(state.hubStatus?.products?.[id] || {}).sort((a, b) => b[1] - a[1])[0]?.[0];
    const version = seen || state.hubStatus?.latest?.[id];
    if (!version) return fail("No version of it is known yet: set where updates come from first.");
    return hubChange(id, { allowed: true, choice: { fixed: version }, mode: null, window: null });
  },
  "hub-when"(el) {
    const v = el.value;
    return hubChange(el.dataset.id, v === "" ? { mode: null, window: null } : v === "manual" ? { mode: "manual", window: null } : { mode: "automatic", window: v === "night" ? [22, 6] : null });
  },
  "hub-mirror"(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/hub", { ...state.hub, mirror: el.checked });
      state.hub = await api("GET", "/v1/admin/hub");
      toast("Saved");
    });
  },
  "hub-update-server"(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/hub/update-server", { version: el.dataset.id });
      state.hubStatus = await api("GET", "/v1/admin/hub/status");
      toast("Xenify Hub on this computer will move the server to the new version");
    });
  },
  "new-authority"() {
    state.dialog = { kind: "confirm", title: "Make a new certificate?", text: "Computers and apps that trusted this server must trust it again.", yes: "Make a new one", then: async () => {
      await api("POST", "/v1/admin/https/authority");
      state.dialog = null;
      await loadHttps();
      toast("New certificate made");
    } };
    render();
  },
  "net-https"(el) {
    state.form.net.https_port = el.checked ? 7443 : null;
    render();
  },
  "net-flag"(el) {
    state.form.net[el.dataset.id] = el.checked;
  },
  "net-get-apps"() {
    return busy(async () => {
      await api("POST", "/v1/admin/network/apps");
      await loadNet();
    });
  },
  "save-net"() {
    return busy(async () => {
      const n = state.form.net;
      const net = { ...n, http_port: Number(n.http_port), https_port: n.https_port ? Number(n.https_port) : null };
      await api("POST", "/v1/admin/network", net);
      // These pages came through a port that has just changed: follow it.
      const here = Number(location.port || (location.protocol === "https:" ? 443 : 80));
      const was = location.protocol === "https:" ? state.net?.https_port : state.net?.http_port;
      const now = location.protocol === "https:" ? net.https_port : net.http_port;
      if (was === here && now && now !== here) {
        location.port = String(now);
        return;
      }
      await loadNet();
      state.net = { ...state.form.net };
      await loadHttps();
      toast("Saved");
    });
  },
  "save-reach"() {
    return busy(async () => {
      const r = state.form.reach || {};
      await api("POST", "/v1/admin/network/reach", { address: r.address || "", trusted_proxies: r.trusted_proxies || "" });
      state.form.reach = await api("GET", "/v1/admin/network/reach");
      toast("Saved");
    });
  },
  "save-domain"() {
    return busy(async () => {
      await api("POST", "/v1/admin/https", { domain: state.form.domain || "", email: state.form.domainEmail || "" });
      await loadHttps();
      toast("Saved");
    });
  },
  "toggle-links"(el) {
    return busy(async () => {
      await api("POST", "/v1/admin/settings", { public_links: el.checked });
      await load();
    });
  },
  "stop-link"(el) {
    const l = state.publinks.links.find((x) => x.id === +el.dataset.id);
    state.dialog = { kind: "confirm", title: `Stop the link to ${l.name}?`, text: "It stops working at once for everyone who has it. Whoever shared it can make a new one.", yes: "Stop link", then: async () => {
      await api("DELETE", `/v1/admin/public-links/${l.id}`);
      state.dialog = null;
      await load();
    } };
    render();
  },
  "save-mail"() {
    return busy(async () => {
      const f = state.form.mail;
      await api("POST", "/v1/admin/mail", { ...f, port: Number(f.port) || 0 });
      state.info = await api("GET", "/v1/info");
      state.mailPasswordSet ||= !!f.password;
      f.password = "";
      toast("Saved");
    });
  },
  "save-storage"() {
    return busy(async () => {
      const v = String(state.form.quotaGb ?? "").trim();
      const gb = v === "" ? 0 : Math.round(Number(v));
      if (!(gb >= 0)) throw new Error("Give a number of GB, like 50, or leave it empty for no limit.");
      await api("POST", "/v1/admin/storage", { quota_gb: gb });
      state.storage = await api("GET", "/v1/admin/storage");
      toast("Saved");
    });
  },
  "toggle-scan"(el) {
    state.form.scan.on = el.checked;
  },
  "save-scan"() {
    return busy(async () => {
      const f = state.form.scan;
      await api("POST", "/v1/admin/scan", { on: !!f.on, address: f.address || "" });
      state.scan = await api("GET", "/v1/admin/scan");
      toast(f.on ? "Saved: uploads are scanned" : "Saved");
    });
  },
  "test-scan"() {
    return busy(async () => {
      const r = await api("POST", "/v1/admin/scan/test", { address: state.form.scan.address || "" });
      state.scan = await api("GET", "/v1/admin/scan");
      if (!r.ok) throw new Error(r.error);
      toast("The scanner works: it found the test file");
    });
  },
  "test-mail"() {
    return busy(async () => {
      const to = state.session.account.email;
      if (!to) throw new Error("Add your own email address first (People › ⋯ › Name and groups).");
      await api("POST", "/v1/admin/mail/test", { to });
      toast(`Sent to ${to}`);
    });
  },
  "toggle-recording"(el) {
    state.form.callRecording = el.checked;
  },
  "toggle-threads"(el) {
    state.form.chatThreads = el.checked;
  },
  "save-chat-limit"() {
    return busy(async () => {
      const mb = Math.round(Number(state.form.chatLimit));
      if (!(mb >= 1)) throw new Error("Give a size in MB, like 2048.");
      const custom = state.form.keepChoice === "custom";
      const days = Math.round(Number(custom ? state.form.keepDays : state.form.keepChoice));
      if (custom && !(days >= 1)) throw new Error("Keep messages for 1 day or more.");
      await api("POST", "/v1/admin/settings", { chat_file_limit_mb: mb, call_recording: !!state.form.callRecording, chat_threads: !!state.form.chatThreads, chat_keep_days: days });
      toast("Saved");
    });
  },
  "save-name"() {
    return busy(async () => {
      await api("POST", "/v1/admin/settings", { name: state.form.serverName || "" });
      state.info = await api("GET", "/v1/info");
      state.form.serverName = "";
      toast("Saved");
    });
  },
  "change-password"() {
    return busy(async () => {
      const f = state.form;
      const p = passwordProblem(f.password, f.password2);
      if (p) throw new Error(p);
      await api("POST", "/v1/password", { old_password: f.old || "", password: f.password });
      state.form = {};
      toast("Password changed. Your other devices sign in again.");
    });
  },
  "sign-out"() {
    signOut();
  },
  "sign-out-all"() {
    api("POST", "/v1/signout-everywhere").catch(() => {}).finally(() => signOut(false));
  },
  close() {
    state.dialog = null;
    state.error = "";
    render();
  },
  backdrop(el, e) {
    if (e.target === el) acts.close();
  },
  "confirm-yes"() {
    const then = state.dialog.then;
    return busy(then);
  },
};

Object.assign(acts, dataActs);
dataInit({ state, api, esc, shell, field, errorBox, render: () => render(), busy, fail: (m) => fail(m), toast });

Object.assign(acts, rolesActs);
rolesInit({ state, api, esc, shell, field, errorBox, render: () => render(), busy, fail: (m) => fail(m), toast });

Object.assign(acts, planActs);
Object.assign(acts, backupsActs);
Object.assign(acts, emailActs);
emailInit({ state, api, esc, shell, errorBox, render: () => render(), fail: (m) => fail(m), toast });
backupsInit({ state, api, esc, shell, errorBox, render: () => render(), fail: (m) => fail(m), toast });
planInit({ state, api, esc, shell, field, errorBox, render: () => render(), busy, fail: (m) => fail(m), toast });

function fail(msg) {
  state.error = msg;
  render();
}

async function aiChange(change) {
  return busy(async () => {
    state.ai = await api("POST", "/v1/admin/ai", change);
    state.form.ai = { ...state.form.ai, ...change, offered: state.ai.offered };
  });
}

let following = null;
function followJobs() {
  if (following) return;
  following = setInterval(async () => {
    if (state.view !== "ai") return;
    try {
      state.jobs = await api("GET", "/v1/admin/ai/jobs");
      if (!state.jobs.some((j) => ["downloading", "checking", "loading"].includes(j.state))) {
        clearInterval(following);
        following = null;
        state.aiStatus = await api("GET", "/v1/admin/ai/status");
      }
      render();
    } catch {}
  }, 1000);
}

async function saveAccess() {
  render();
  try {
    await api("POST", `/v1/spaces/${state.spaceSel}/access`, { grants: state.access.grants });
  } catch (e) {
    state.error = e.message;
    await loadAccess();
    render();
  }
}

// ---- Wiring ---------------------------------------------------------------------

document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-act]");
  if (!el || el.tagName === "FORM" || el.tagName === "SELECT") {
    if (state.menu && !e.target.closest(".menu")) {
      state.menu = null;
      render();
    }
    return;
  }
  if (el.type === "radio" || el.type === "checkbox") return; // on change
  e.preventDefault();
  acts[el.dataset.act]?.(el, e);
});

document.addEventListener("change", (e) => {
  const el = e.target;
  if (el.dataset.bind && el.tagName === "SELECT") {
    setPath(el.dataset.bind, el.value);
    render();
  } else if (el.dataset.act) acts[el.dataset.act]?.(el, e);
});

document.addEventListener("input", (e) => {
  const el = e.target;
  // A model picked from the Offer list's suggestions: offered at once.
  if (el.dataset.act === "ai-offer" && e.inputType !== "insertText" && e.inputType !== "deleteContentBackward") return void acts["ai-offer"](el, e);
  if (el.dataset.bind) {
    setPath(el.dataset.bind, el.value);
    if (el.dataset.bind === "find") render();
  }
});

document.addEventListener("submit", (e) => {
  e.preventDefault();
  acts[e.target.dataset.act]?.(e.target, e);
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && (state.dialog || state.menu)) {
    state.dialog = null;
    state.menu = null;
    render();
  }
  // Enter in a dialog is its main button (or its only one); a form sends itself, and a box that uses Enter keeps it.
  // (Not on a one-time code: Done there closes it for good, so it takes a click.)
  else if (e.key === "Enter" && state.dialog && state.dialog.kind !== "code" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing && !e.defaultPrevented
    && !e.target.closest?.("form, textarea, select, button, a[href], [contenteditable]:not([contenteditable='false'])")) {
    const btns = [...document.querySelectorAll(".backdrop .dialog > .row:last-child .btn, .backdrop .dialog > .stack > .row:last-child .btn")];
    const main = btns.find((b) => b.classList.contains("main")) || (btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) {
      e.preventDefault();
      main.click();
    }
  }
});

function fromHash() {
  const v = location.hash.replace(/^#\//, "").split("/")[0];
  return ["people", "roles", "spaces", "links", "ai", "data", "holidays", "updates", "backups", "email", "network", "account"].includes(v) ? v : "";
}

window.addEventListener("hashchange", () => {
  const v = fromHash();
  if (v && state.session && v !== state.view) go(v);
});

async function start() {
  try {
    state.info = await api("GET", "/v1/info");
  } catch (e) {
    state.view = "signin";
    state.error = "Can't reach the server.";
    return render();
  }
  const hash = location.hash.replace(/^#\//, "").split("/");
  if (state.info.setup_needed) {
    state.setup.token = hash[0] === "setup" ? hash[1] || "" : "";
    if (!state.setup.token) {
      state.view = "signin";
      state.error = "This server isn't set up yet. Open the setup address the server printed when it started.";
      return render();
    }
    return go("setup");
  }
  if (location.pathname.startsWith("/join")) {
    state.joinCode = decodeURIComponent(location.hash.slice(1));
    state.view = "join";
    try {
      if (!state.joinCode.includes(".")) throw new Error("This link isn't complete: ask for it again.");
      state.link = await api("POST", "/v1/link", { link: state.joinCode });
    } catch (e) {
      state.error = e.message;
    }
    return render();
  }
  try {
    state.session = sharedSession();
  } catch {}
  if (state.session) {
    try {
      const me = await api("GET", "/v1/me");
      state.powers = me.admin ? [] : await powersOf(state.session.token);
      if (!me.admin && !state.powers.length) throw new Error();
      state.session.account = me;
      return go(fromHash() || firstPage());
    } catch {
      state.session = null;
      keep();
    }
  }
  go("signin");
}

start();
