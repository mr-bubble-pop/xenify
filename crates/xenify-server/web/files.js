// The server's Files page (/files/) and pages opened with a public link
// (/p/<token>). One page, drawn from `state` by render() like the admin
// pages; clicks go through data-act.
//
// A Write document is read as a web page the server makes from it
// (/v1/nodes/<id>/page), shown in a frame that can't run scripts.

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const initials = (s) => String(s || "?").split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "?";
const sentence = (s) => (s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".") : s);
const APPS = { write: "Write", draw: "Draw", calc: "Calc", plan: "Plan" };
// What New makes, and the app that opens it.
const NEW_KINDS = [
  { app: "write", ext: "xwrite", what: "Document", letter: "W" },
  { app: "draw", ext: "xdraw", what: "Drawing", letter: "D" },
  { app: "calc", ext: "xcalc", what: "Spreadsheet", letter: "C" },
  { app: "plan", ext: "xplan", what: "Plan", letter: "P" },
];
const EXT_APP = { xwrite: "write", igdoc: "write", xdraw: "draw", xenify: "draw", xcalc: "calc", xplan: "plan" };
// The largest file sent from this page (it's read into memory first).
const MAX_UPLOAD = 200 << 20;

const icon = {
  chat: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/></svg>',
  recent: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/></svg>',
  shared: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>',
  folder: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>',
  file: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>',
  search: '<svg aria-hidden="true" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>',
  read: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>',
  edit: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.38 3.62a1 1 0 0 1 3 3L7.37 18.64a2 2 0 0 1-.86.5l-2.87.84.84-2.87a2 2 0 0 1 .5-.86z"/></svg>',
  down: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/></svg>',
  up: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m17 8-5-5-5 5"/><path d="M12 3v12"/></svg>',
  page: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z"/><path d="M14 3v5h5M9 13h6M9 17h4"/></svg>',
  plus: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/><path d="M12 5v14"/></svg>',
  // A folder, filled: unlike any file's mark.
  folderFill: '<svg aria-hidden="true" viewBox="0 0 24 24"><path fill="currentColor" d="M3 6.5A2.5 2.5 0 0 1 5.5 4h3.6c.66 0 1.3.26 1.77.73L12.3 6.2h6.2A2.5 2.5 0 0 1 21 8.7v8.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5Z"/><path fill="#fff" fill-opacity=".28" d="M3 9.2h18v1.3H3z"/></svg>',
  list: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/></svg>',
  grid: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>',
  home: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10"/><path d="M10 20v-6h4v6"/></svg>',
  link: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/></svg>',
  bin: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/></svg>',
};

/// What a file or folder is, at a glance: a filled folder, or the file's
/// app in its colour and letter (other files: a plain page).
function mark(n, big = false) {
  const cls = `fmark${big ? " big" : ""}`;
  if (n.kind === "folder") return `<span class="${cls} is-folder" aria-hidden="true">${icon.folderFill}</span>`;
  const k = NEW_KINDS.find((x) => x.app === appOf(n));
  if (k) return `<span class="${cls} app-${k.app}" aria-hidden="true">${k.letter}</span>`;
  return `<span class="${cls} other" aria-hidden="true">${icon.file}</span>`;
}

const state = {
  mode: location.pathname.startsWith("/p/") ? "public" : "files",
  view: "loading", // signin, browse, read
  info: null,
  session: null, // { token, account }
  powers: [], // the parts of running the server this person has (admins: all)
  perms: [], // what their roles let them do (files.spaces: make spaces)
  menu: null, // the open menu: "new", "me"
  error: "",
  busy: false,
  toast: "",
  dialog: null,
  form: {},
  spaces: [],
  people: {}, // id -> name
  groups: {}, // id -> name
  place: { kind: "recent" }, // recent, shared, or { kind: "space", space, path: [{ id, name }] }
  items: [],
  sel: null, // the file or folder whose details are shown (Details)
  about: null, // details of `sel`
  ctx: null, // the open menu of a file, folder or page: { node, x, y, from }
  find: "",
  findScope: null, // search only this space (when it's the one shown)
  everything: null, // what the last search found
  thumbs: {}, // node id -> data: URL, or "" when it has none
  dir: { people: [], groups: [] }, // everyone and every group, for sharing
  filter: "", // narrows the folder shown to names with this in them
  sort: { key: "", desc: false }, // the list's order: "" (as given), name, kind, changed, by
  // A space's pages (a wiki): the tree, the page shown, the pages open.
  wiki: null, // { space, node, path: [pages above], tree: { page id or 0: its pages }, open: Set, page, drawer }
  reading: null, // { node, html, toc, picture, siblings, error }
  pub: null, // { token, key, items, root, server }
};

// ---- Talking to the server --------------------------------------------------

async function api(method, path, body, { raw = false, bytes = null } = {}) {
  const headers = {};
  if (state.session?.token) headers.Authorization = `Bearer ${state.session.token}`;
  let payload;
  if (bytes) {
    headers["Content-Type"] = "application/octet-stream";
    payload = bytes;
  } else if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const r = await fetch(path, { method, headers, body: payload });
  if (raw && r.ok) return new Uint8Array(await r.arrayBuffer());
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

// URL-safe base64 without padding.
function b64(u8) {
  let s = "";
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000));
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// Write's engine (and data: addresses) want standard base64, padded.
const std64 = (u8) => {
  const s = b64(u8).replace(/-/g, "+").replace(/_/g, "/");
  return s + "=".repeat((4 - (s.length % 4)) % 4);
};

// "Remember this computer" after a second step: kept in this browser.
function remembered() {
  try {
    return localStorage.getItem("xenify-files-remember") || "";
  } catch {
    return "";
  }
}

async function signedIn(s) {
  if (s.remember) {
    try {
      localStorage.setItem("xenify-files-remember", s.remember);
    } catch {}
  }
  state.step = null;
  state.session = { token: s.token, account: s.account };
  state.form = {};
  keep();
  if (backTo()) return void location.replace(backTo());
  await afterSignIn();
}

// One sign-in for every page of this server (Files, the admin pages and the
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
    return s?.token && s.account ? s : null;
  } catch {
    return null;
  }
}

// Signed in or out on another page of this server.
window.addEventListener("storage", (e) => {
  if (e.key === SHARED && sharedSession()?.token !== state.session?.token && state.mode !== "public") location.reload();
});

/// Where to go after signing in: back to the app that sent you here.
function backTo() {
  const b = new URLSearchParams(location.search).get("back") || "";
  if (!b.startsWith("/")) return "";
  // Only a page of this server ("/\elsewhere" and the like lead away).
  let u;
  try { u = new URL(b, location.origin); } catch { return ""; }
  if (u.origin !== location.origin || u.pathname.startsWith("/files")) return "";
  // A path such as "//elsewhere" (from "/.//elsewhere") would leave too.
  return u.pathname.startsWith("//") ? "" : u.pathname + u.search + u.hash;
}

/// A chart's picture from a Calc book: base64 only (it goes into a page).
const chartPng = (x) => (typeof x?.png === "string" && /^[A-Za-z0-9+/]+={0,2}$/.test(x.png) ? x.png : "");

function signOut(tell = true) {
  if (tell && state.session) api("POST", "/v1/signout").catch(() => {});
  state.session = null;
  state.everything = null;
  state.powers = [];
  state.menu = null;
  keep();
  state.view = "signin";
  render();
}

// ---- Names and contents ----------------------------------------------------

/// A node's details ({ name, app, ext }) from its `meta` JSON text.
function details(meta) {
  try {
    const d = JSON.parse(meta);
    return typeof d === "object" && d ? d : { name: "" };
  } catch {
    return { name: "" };
  }
}

async function name(n) {
  n.d ??= details(n.meta);
  return n.d;
}

const nameOf = (n) => n.d?.name || (n.kind === "folder" ? "Untitled folder" : "Untitled");
/// The name a download is saved under: with its kind's ending (names here
/// don't need one, but Windows and the apps go by it).
const fileNameOf = (n) => {
  const name = nameOf(n);
  const ext = String(n.d?.ext || "").replace(/^\./, "").toLowerCase();
  return ext && !name.toLowerCase().endsWith(`.${ext}`) ? `${name}.${ext}` : name;
};

function appOf(n) {
  if (n.kind === "folder") return null;
  const ext = (n.d?.ext || n.d?.name?.split(".").pop() || "").toLowerCase();
  return n.d?.app || EXT_APP[ext] || null;
}

// What a file is, in plain words.
const KIND_WORD = { write: "Document", draw: "Drawing", calc: "Spreadsheet", plan: "Plan" };
const EXT_WORD = {
  pdf: "PDF", png: "Picture", jpg: "Picture", jpeg: "Picture", gif: "Picture", webp: "Picture", svg: "Picture", heic: "Picture",
  txt: "Text", md: "Text", csv: "Table (CSV)", zip: "Zip archive", docx: "Document", doc: "Document",
  xlsx: "Spreadsheet", xls: "Spreadsheet", pptx: "Slides", mp4: "Video", mov: "Video", mp3: "Audio", wav: "Audio",
};
function kindLabel(n) {
  if (n.kind === "folder") return "Folder";
  if (isPage(n)) return "Page";
  const app = appOf(n);
  if (app) return KIND_WORD[app] || APPS[app] || app;
  const ext = n.d?.name?.includes(".") ? n.d.name.split(".").pop().toLowerCase() : "";
  return EXT_WORD[ext] || (ext ? `${ext.toUpperCase()} file` : "File");
}

/// A page of a space's wiki: a Write document marked as one.
const isPage = (n) => n?.kind === "file" && n.d?.page === true;
const pageName = (n) => nameOf(n).replace(/\.(xwrite|igdoc)$/i, "");
/// Shared spaces are wikis (Pages, and Files); My files and Templates are files.
const wikiSpace = (id) => state.spaces.find((s) => s.id === id)?.kind === "shared";
/// A space's details (its name, and its home page).
const spaceDetails = (id) => details(state.spaces.find((s) => s.id === id)?.name);
const spaceRole = (id) => state.spaces.find((s) => s.id === id)?.role;
/// Pages in their order: as put there with Move up / Move down, else by name.
const byOrder = (x, y) => (x.d?.order ?? Infinity) - (y.d?.order ?? Infinity) || pageName(x).localeCompare(pageName(y), undefined, { numeric: true });
/// A link to a page (or file) for people on this server.
const linkTo = (n) => `${location.origin}${placePath(isPage(n) && wikiSpace(n.space) ? `#/wiki/${n.space}/${n.id}` : appOf(n) || readable(n) ? `#/read/${n.id}` : `#/space/${n.space}${n.parent ? `/${n.parent}` : ""}`)}`;
const spaceHome = (id) => (wikiSpace(id) ? `#/wiki/${id}` : `#/space/${id}`);

/// A file's picture as a data: URL. A Calc book's "picture" is what other
/// documents can show of it (its tables and charts, as JSON): its first
/// chart then, else none.
function pictureOf(bytes) {
  const b = new Uint8Array(bytes);
  if (b[0] === 0x89 && b[1] === 0x50) return `data:image/png;base64,${std64(bytes)}`;
  const chart = calcSnapshot(bytes)?.items.find((x) => x.kind === "chart" && chartPng(x));
  return chart ? `data:image/png;base64,${chartPng(chart)}` : "";
}

/// A Calc book's tables, summaries and charts as Calc put them up, or null.
function calcSnapshot(bytes) {
  try {
    const s = JSON.parse(new TextDecoder().decode(bytes));
    return s.format === "xenify-calc-link" && Array.isArray(s.items) ? s : null;
  } catch {
    return null;
  }
}

/// A Calc book as a page: each table, summary and sheet as Calc shows it,
/// charts as pictures.
function calcPage(snap, title) {
  const kinds = { table: "Table", summary: "Summary", sheet: "Sheet", chart: "Chart" };
  const parts = snap.items.filter((x) => kinds[x.kind]).map((x) => {
    const head = `<h2>${esc(x.name)}${x.sheet && x.sheet !== x.name ? ` <small>· ${esc(x.sheet)}</small>` : ""}</h2>`;
    if (x.kind === "chart") return head + (chartPng(x) ? `<p><img src="data:image/png;base64,${chartPng(x)}" alt="${esc(x.name)}"></p>` : "<p><i>No picture of this chart yet.</i></p>");
    const rows = (x.rows || []).map((r, k) => {
      const th = (k === 0 && x.header) || (k === x.rows.length - 1 && x.footer);
      return `<tr>${r.map((c, j) => `<${th ? "th" : "td"}${x.numbers?.[j] && !th ? ' class="n"' : ""}>${esc(c)}</${th ? "th" : "td"}>`).join("")}</tr>`;
    });
    return `${head}<table>${rows.join("")}</table>`;
  });
  return `<!doctype html><html><head><meta charset="utf-8"><title>${esc(title)}</title><style>
    body { font: 15px/1.5 system-ui, sans-serif; color: #1c1917; margin: 32px 40px; }
    h1 { font-size: 26px; margin: 0 0 18px; } h2 { font-size: 17px; margin: 28px 0 8px; } small { color: #78716c; font-weight: 400; }
    table { border-collapse: collapse; font-size: 14px; } td, th { border: 1px solid #e7e5e4; padding: 4px 10px; text-align: left; }
    th { background: #f5f5f4; } td.n { text-align: right; font-variant-numeric: tabular-nums; } img { max-width: 100%; }
  </style></head><body><h1>${esc(title)}</h1>${parts.join("") || "<p><i>Nothing to show yet: open it in Calc and save it once.</i></p>"}</body></html>`;
}

/// Whether file `n` can be read on this page.
function readable(n) {
  const app = appOf(n);
  return app === "write" || ((app === "calc" || app === "draw") && n.pictured) || !!viewAs(n);
}

const PICTURE_TYPE = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", svg: "image/svg+xml" };
const TEXT_EXT = ["txt", "md", "csv", "log", "json"];
/// How a file without an app is shown here: "picture", "text", or "" (downloaded).
function viewAs(n) {
  if (n?.kind !== "file" || appOf(n)) return "";
  const ext = n.d?.name?.includes(".") ? n.d.name.split(".").pop().toLowerCase() : "";
  return PICTURE_TYPE[ext] ? "picture" : TEXT_EXT.includes(ext) ? "text" : "";
}

/// A file's latest contents, as it's kept on a computer (a Draw canvas
/// comes with its pictures and clips).
function contents(n) {
  return api("GET", `/v1/nodes/${n.id}/content?file=true`, undefined, { raw: true });
}

async function thumb(n) {
  if (n.id in state.thumbs || !n.pictured) return;
  state.thumbs[n.id] = "";
  try {
    const png = await api("GET", `/v1/nodes/${n.id}/preview`, undefined, { raw: true });
    state.thumbs[n.id] = pictureOf(png);
    render();
  } catch {}
}

function ago(iso) {
  const s = (Date.now() - Date.parse(iso)) / 1000;
  if (s < 120) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  if (s < 172800) return "yesterday";
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short", year: s > 300 * 86400 ? "numeric" : undefined });
}

function save(bytes, filename) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([bytes]));
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 60000);
}

// ---- Loading -------------------------------------------------------------

// ---- Where you are -------------------------------------------------------
// A place is a path (/files/wiki/3/4); the code names it as "#/wiki/3/4".
// Old links (/files/#/wiki/3/4) still work. Public links keep their #.
const placePath = (h) => (h && h !== "#" ? `/files${h.slice(1)}` : "/files/");

function where() {
  if (state.mode === "public") return location.hash;
  const rest = location.pathname.replace(/^\/files/, "");
  return rest && rest !== "/" ? `#${rest}` : "";
}

/// Goes to place `h` ("#/…"), a new step in the history (or in its place).
function go(h, replace = false) {
  if (state.mode === "public") {
    if (replace) location.replace(h);
    else location.hash = h;
    return;
  }
  history[replace ? "replaceState" : "pushState"](null, "", placePath(h));
  route();
}

/// An old #/ address: the same place as a path.
function cleanAddress() {
  if (state.mode !== "public" && location.hash.startsWith("#/")) history.replaceState(null, "", placePath(location.hash) + location.search);
}

async function start() {
  cleanAddress();
  try {
    state.info = await api("GET", "/v1/info");
  } catch (e) {
    state.error = e.message;
  }
  if (state.mode === "public") document.title = "Shared with you";
  if (state.mode === "public") return openPublic();
  try {
    state.session = sharedSession();
  } catch {}
  if (state.session && backTo()) return void location.replace(backTo());
  if (!state.session) {
    // From an invitation's "Sign in": the email address is filled in.
    state.form.username ||= new URLSearchParams(location.search).get("user") || "";
    state.view = "signin";
    return render();
  }
  await afterSignIn();
}

async function afterSignIn() {
  state.view = "browse";
  try {
    const mine = await api("GET", "/v1/me/perms").catch(() => null);
    state.powers = state.session.account.admin ? ["admin"] : mine?.powers || [];
    state.perms = mine?.perms || [];
  } catch {}
  try {
    const [spaces, dir] = await Promise.all([api("GET", "/v1/spaces"), api("GET", "/v1/directory")]);
    state.dir = dir;
    for (const p of dir.people) state.people[p.id] = p.display_name || p.email || p.username;
    for (const g of dir.groups) state.groups[g.id] = g.name;
    state.spaces = [];
    for (const s of spaces.filter((s) => s.kind !== "chat")) {
      s.label = s.kind === "personal" ? "My files" : s.kind === "templates" ? "Templates" : details(s.name).name || "Untitled space";
      state.spaces.push(s);
    }
  } catch (e) {
    state.error = e.message;
  }
  await route();
}

/// Follows the address: #/space/<id>/<folder>, #/wiki/<space>/<page>,
/// #/read/<id>, #/recent, #/shared.
async function route() {
  state.ctx = null;
  if (pageEd.node) await endEdit({ reload: false });
  const [, what, a, b] = where().split("/");
  // Leaving a search: a document opened from it still shows the words found.
  if (what !== "search") {
    state.lastFind = state.view === "search" ? state.find : "";
    state.find = "";
  }
  if (what === "read" && a) return read(+a);
  if (what === "wiki" && a) return wiki(+a, b ? +b : null);
  if (what === "changes" && a) return wiki(+a, null, true);
  if (what === "bin" && a) return bin(+a);
  if (what === "search") return found(decodeURIComponent(a || ""), b ? +b : null);
  if (what === "pages" && a) return void go(`#/wiki/${a}${b ? `/${b}` : ""}`, true);
  // A link to a document by its id alone (from inside a document): its page, or read here.
  if (what === "page" && a) {
    const n = await api("GET", `/v1/nodes/${+a}`).catch(() => null);
    if (n) await name(n);
    if (n) return void location.replace(isPage(n) ? `#/wiki/${n.space}/${n.id}` : `#/read/${n.id}`);
  }
  if (what === "space" && a) {
    const space = +a;
    const path = [];
    // The folders on the way down, for the path shown.
    for (let id = b ? +b : null; id; ) {
      const n = await api("GET", `/v1/nodes/${id}`).catch(() => null);
      if (!n) break;
      await name(n);
      path.unshift({ id: n.id, name: nameOf(n) });
      id = n.parent;
    }
    return show({ kind: "space", space, path });
  }
  return show({ kind: what === "shared" ? "shared" : "recent" });
}

async function show(place) {
  state.view = "browse";
  state.place = place;
  state.reading = null;
  state.error = "";
  state.find = "";
  state.filter = "";
  try {
    if (place.kind === "recent") state.items = await api("GET", "/v1/recent");
    else if (place.kind === "shared") state.items = await api("GET", "/v1/shared");
    else {
      const folder = place.path.at(-1)?.id;
      state.items = await api("GET", `/v1/nodes?space=${place.space}${folder ? `&parent=${folder}` : ""}`);
    }
    for (const n of state.items) await name(n);
    if (place.kind === "space") state.items = state.items.filter((n) => !isPage(n));
    state.items.sort((x, y) => (x.kind === y.kind ? (place.kind === "space" ? nameOf(x).localeCompare(nameOf(y)) : 0) : x.kind === "folder" ? -1 : 1));
  } catch (e) {
    state.items = [];
    state.error = e.message;
  }
  if (state.sel && !state.items.some((n) => n.id === state.sel.id)) {
    state.sel = null;
    state.about = null;
  }
  render();
  state.items.forEach(thumb);
}

function placeHash(p) {
  if (p.kind !== "space") return `#/${p.kind}`;
  const folder = p.path.at(-1)?.id;
  return `#/space/${p.space}${folder ? `/${folder}` : ""}`;
}

async function choose(n) {
  state.sel = n;
  state.about = null;
  render();
  if (!n) return;
  const about = {};
  try {
    const [versions, access, pub] = await Promise.all([
      n.kind === "file" ? api("GET", `/v1/nodes/${n.id}/versions`) : null,
      api("GET", `/v1/nodes/${n.id}/access`).catch(() => null),
      api("GET", `/v1/nodes/${n.id}/public`).catch(() => null),
    ]);
    about.versions = versions?.length ?? 0;
    about.who = access ? whoCan(access, n) : "";
    about.pub = pub;
  } catch {}
  if (state.sel?.id === n.id) {
    state.about = about;
    render();
  }
}

const ROLE = { viewer: "can view", commenter: "can comment", editor: "can edit", manager: "can manage" };

function whoCan(access, n) {
  if (!access.own) return `The same people as ${!n.parent ? "the space" : isPage(n) ? "the page above it" : "the folder it's in"}.`;
  const parts = access.grants.map((g) => {
    const w = g.who === "everyone" ? `Everyone on ${state.info?.name || "this server"}` : g.who.user ? state.people[g.who.user] || "Someone" : state.groups[g.who.group] || "A group";
    return `${w} ${ROLE[g.role] || g.role}`;
  });
  return parts.length ? `${parts.join(" · ")}.` : "Only you.";
}

const webApps = () => !!state.info?.features?.includes("web_apps");

/// Search, from the box at the top: its own screen (#/search/<words>[/<space>]),
/// so Back returns from it and it can be linked to.
function search(q) {
  const want = q.trim() ? `#/search/${encodeURIComponent(q.trim())}${state.findScope ? `/${state.findScope}` : ""}` : "";
  if (!want) {
    if (state.view === "search") history.back();
    return;
  }
  go(want, state.view === "search");
}

/// The search screen: what has these words in its name or text, everywhere
/// or in one space, each with where it is.
async function found(q, scope) {
  state.view = "search";
  state.find = q;
  state.findScope = scope;
  state.sel = null;
  state.everything = null;
  render();
  const hits = await api("GET", `/v1/search?q=${encodeURIComponent(q)}${scope ? `&space=${scope}` : ""}`).catch(() => []);
  if (state.find !== q || state.view !== "search") return;
  for (const n of hits) {
    await name(n);
    n.spaceLabel = state.spaces.find((s) => s.id === n.space)?.label || "";
  }
  state.everything = hits;
  if (state.aiAvailable === undefined) state.aiAvailable = (await api("GET", "/v1/ai").catch(() => null))?.available && true;
  render();
  // Where each is: the pages or folders above it.
  const named = new Map();
  for (const n of hits.slice(0, 30)) {
    const path = [];
    for (let up = n.parent; up; ) {
      let f = named.get(up);
      if (!f) {
        f = await api("GET", `/v1/nodes/${up}`).catch(() => null);
        if (!f) break;
        await name(f);
        named.set(up, f);
      }
      path.unshift(isPage(f) ? pageName(f) : nameOf(f));
      up = f.parent;
    }
    n.path = path;
  }
  if (state.find === q && state.view === "search") render();
}

/// Search by meaning with the server's AI, in steps shown as they go: words
/// to look for, the files that have them (up to 40), then the AI reads
/// them 8 at a time and keeps the ones that answer the question.
async function aiSearch() {
  const q = state.find.trim(), scope = state.findScope;
  if (!q) return;
  const run = (state.aiFind = { q, scope, step: "words", read: 0, total: 0, found: [], error: "" });
  const live = () => state.aiFind === run && state.view === "search" && state.find.trim() === q;
  render();
  try {
    const got = (await api("POST", "/v1/search/ai/words", { q })).terms || [];
    if (!live()) return;
    // A phrase finds only files with all its words: its words go on their own too.
    const terms = [...new Set([...got, ...got.flatMap((t) => t.split(/\s+/)).filter((w) => w.length > 2)])].slice(0, 16);
    run.terms = terms;
    run.step = "search";
    render();
    // The files with any of those words (those the ordinary search found first).
    const seen = new Map();
    for (const n of state.everything || []) if (n.kind === "file") seen.set(n.id, n);
    for (const t of terms) {
      if (seen.size >= 40) break;
      const hits = await api("GET", `/v1/search?q=${encodeURIComponent(t)}${scope ? `&space=${scope}` : ""}`).catch(() => []);
      for (const n of hits) if (n.kind === "file" && !seen.has(n.id) && seen.size < 40) seen.set(n.id, n);
      if (!live()) return;
    }
    const all = [...seen.values()];
    run.total = all.length;
    run.step = "read";
    render();
    for (let i = 0; i < all.length; i += 8) {
      const batch = all.slice(i, i + 8);
      const { found } = await api("POST", "/v1/search/ai/read", { q, ids: batch.map((n) => n.id) });
      if (!live()) return;
      for (const f of found) {
        const n = batch.find((x) => x.id === f.id);
        if (!n) continue;
        await name(n);
        n.spaceLabel = state.spaces.find((s) => s.id === n.space)?.label || "";
        run.found.push({ n, why: f.why });
      }
      run.read = Math.min(all.length, i + 8);
      render();
    }
    run.step = "done";
  } catch (e) {
    if (!live()) return;
    run.step = "error";
    run.error = e.message || String(e);
  }
  render();
}

function aiFindHtml() {
  if (!state.aiAvailable) return "";
  const a = state.aiFind?.q === state.find.trim() && state.aiFind.scope === state.findScope ? state.aiFind : null;
  if (!a) {
    return `<div class="aifind"><div class="row"><span class="grow dim">Not what you’re after? AI can read the files that might be about it and find them by what they mean, not only the same words.</span><button class="btn small" data-act="ai-search">Look with AI</button></div></div>`;
  }
  const steps = { words: "Thinking of words to look for…", search: `Looking for files with ${(a.terms || []).map((t) => `“${esc(t)}”`).join(", ")}…`, read: `Reading the files: ${a.read} of ${a.total}…` };
  const busy = steps[a.step];
  const bar = a.step === "read" && a.total ? `<div class="aibar"><i style="width:${Math.round((a.read / a.total) * 100)}%"></i></div>` : "";
  const found = a.found.map(({ n, why }) => hitHtml({ ...n, snippet: "" }).replace("</span></button>", `<br><span class="dim snippet">${esc(why)}</span></span></button>`)).join("");
  const end = a.step === "error" ? `<div class="error" role="alert">${esc(a.error)}</div>`
    : a.step === "done" ? `<p class="dim">${a.found.length ? `Read ${a.total} ${a.total === 1 ? "file" : "files"}.` : `Read ${a.total} ${a.total === 1 ? "file" : "files"}; none of them seems to be about it.`}</p>` : "";
  return `<div class="aifind" aria-live="polite"><h2>Found with AI</h2>${busy ? `<p class="dim">${busy}</p>${bar}` : ""}${found ? `<div class="hits">${found}</div>` : ""}${end}</div>`;
}

function searchPage() {
  document.title = `Search: ${state.find}`;
  return `<div class="files">${header()}<div class="fbody">${places()}
    <main class="folder" aria-label="Found">${foundHtml()}</main></div></div>`;
}

/// A search hit's bit of text, with the matching words marked.
const snippet = (n) => (n.snippet ? `<br><span class="dim snippet">…${esc(n.snippet).replace(/&lt;&lt;/g, "<mark>").replace(/&gt;&gt;/g, "</mark>")}…</span>` : "");

// ---- Reading ------------------------------------------------------------

/// The page as HTML for a frame that can't run scripts: links to other
/// sites open in a new tab, anything else that isn't a web or mail link
/// is dropped. Returns { html, toc }.
function forFrame(html, find = state.find, { reading = true } = {}) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  markWords(doc, find);
  for (const a of doc.querySelectorAll("a[href]")) {
    const href = a.getAttribute("href");
    const page = pageLink(href);
    if (page) {
      a.setAttribute("href", `#/page/${page}`);
      a.dataset.page = page;
      continue;
    }
    if (href.startsWith("#")) continue;
    if (/^(https?:|mailto:)/i.test(href)) {
      a.target = "_blank";
      a.rel = "noopener noreferrer";
    } else a.removeAttribute("href");
  }
  const toc = [];
  doc.querySelectorAll("h1, h2, h3").forEach((h, i) => {
    if (!h.id) h.id = `h${i}`;
    if (h.tagName !== "H1" || i > 0) toc.push({ id: h.id, text: h.textContent.trim(), level: +h.tagName[1] });
  });
  // Every heading can be linked to: an anchor named after it (…#getting-started)
  // and a # beside it, seen when pointed at, that copies the link.
  if (reading) {
    const used = new Set();
    for (const h of doc.querySelectorAll("h1, h2, h3, h4, h5, h6")) {
      const base = h.textContent.trim().toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "").replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "").slice(0, 60) || "section";
      let slug = base;
      for (let n = 2; used.has(slug); n++) slug = `${base}-${n}`;
      used.add(slug);
      const mark = doc.createElement("a");
      mark.className = "hl";
      mark.id = slug;
      mark.href = `#${slug}`;
      mark.textContent = "#";
      mark.setAttribute("aria-label", `Copy a link to “${h.textContent.trim()}”`);
      mark.title = "Copy a link to this heading";
      h.append(mark);
    }
  }
  const csp = doc.createElement("meta");
  csp.httpEquiv = "Content-Security-Policy";
  csp.content = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data: 'self'";
  doc.head.prepend(csp);
  // The document's own fonts (Write's built-in ones), as it looks in Write.
  const faces = doc.createElement("style");
  const face = (family, file, weight = 400, style = "normal") =>
    `@font-face { font-family: "${family}"; src: url("${location.origin}/v1/fonts/${file}"); font-weight: ${weight}; font-style: ${style}; font-display: swap; }`;
  faces.textContent = [face("Inter", "inter.ttf", "100 900"), face("CMU Serif", "cmu-serif.otf"), face("CMU Serif", "cmu-serif-italic.otf", 400, "italic"),
    face("CMU Serif", "cmu-serif-bold.otf", 700), face("CMU Serif", "cmu-serif-bold-italic.otf", 700, "italic")].join("\n");
  doc.head.prepend(faces);
  // It grows to fit (sizeFrame), so it never scrolls by itself, and its
  // height never depends on the frame's (no loop between the two). A page
  // reads like a web page: the reading width, not a sheet's margins, in the
  // middle of the space it has (on the left it looked cut off on a wide screen).
  const fit = doc.createElement("style");
  // flow-root: a picture floating beside the text counts too (it isn't cut off).
  fit.textContent = `html, body { overflow: hidden; height: auto !important; min-height: 0 !important; }
    body { display: flow-root; }
    ${reading ? "body { max-width: 46rem !important; margin: 0 auto !important; padding: 4px 2px 12px !important; }" : ""}
    a.hl { margin-left: .35em; color: #a8a29e; text-decoration: none; font-weight: 400; opacity: 0; }
    h1:hover > a.hl, h2:hover > a.hl, h3:hover > a.hl, h4:hover > a.hl, h5:hover > a.hl, h6:hover > a.hl, a.hl:focus { opacity: 1; }
    @media print { a.hl { display: none; } }`;
  doc.head.append(fit);
  return { html: `<!doctype html>${doc.documentElement.outerHTML}`, toc: toc.filter((t) => t.text).slice(0, 40) };
}

/// A link to a document on this server (its node id), from the forms a
/// link to one can take; else null.
function pageLink(href) {
  const m = /^(?:https?:\/\/[^/]+)?(?:\/files\/)?#\/(?:read|page|pages\/\d+|wiki\/\d+)\/(\d+)$/.exec(href)
    || /^(?:https?:\/\/[^/]+)?\/files\/(?:read|page|wiki\/\d+)\/(\d+)$/.exec(href)
    || /^xenify-\w+:\/\/open\?.*\bnode=(\d+)/.exec(href)
    || /^(?:https?:\/\/[^/]+)?\/\w+\/#open=[^,]+,(\d+)$/.exec(href)
    || /^(?:https?:\/\/[^/]+)?\/\w+\/\?open=(\d+)$/.exec(href);
  if (!m) return null;
  // Another server's document isn't one of these pages.
  const full = /^https?:\/\/[^/]+/.exec(href)?.[0] || /server=([^&]+)/.exec(href)?.[1];
  if (full && decodeURIComponent(full) !== location.origin) return null;
  return +m[1];
}

/// Marks the words searched for (each word, at the start of words, as the
/// server's search matches them) so they can be found on the page.
function markWords(doc, find) {
  const words = String(find || "").split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (!words.length) return;
  const re = new RegExp(`(?<![\\p{L}\\p{N}])(${words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "giu");
  const style = doc.createElement("style");
  style.textContent = "mark.found { background: #fde7d9; color: inherit; border-radius: 2px; }";
  doc.head.append(style);
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
  const texts = [];
  while (walker.nextNode()) if (walker.currentNode.data.match(re)) texts.push(walker.currentNode);
  for (const node of texts) {
    const out = doc.createDocumentFragment();
    let at = 0;
    for (const m of node.data.matchAll(re)) {
      out.append(node.data.slice(at, m.index));
      const mark = doc.createElement("mark");
      mark.className = "found";
      mark.textContent = m[0];
      out.append(mark);
      at = m.index + m[0].length;
    }
    out.append(node.data.slice(at));
    node.replaceWith(out);
  }
}

/// Shows file `id` as a page (Write, Calc), or its picture (Draw).
async function read(id) {
  state.view = "read";
  state.reading = { node: null };
  render();
  try {
    const n = await api("GET", `/v1/nodes/${id}`);
    await name(n);
    state.reading.node = n;
    state.sel = n;
    const siblings = n.parent ? await api("GET", `/v1/nodes?space=${n.space}&parent=${n.parent}`) : [];
    for (const s of siblings) await name(s);
    state.reading.siblings = siblings.filter((s) => s.kind === "file").sort((x, y) => nameOf(x).localeCompare(nameOf(y)));
    // The folders above it, for the path shown.
    const path = [];
    for (let up = n.parent; up; ) {
      const f = await api("GET", `/v1/nodes/${up}`).catch(() => null);
      if (!f) break;
      await name(f);
      path.unshift(f);
      up = f.parent;
    }
    state.reading.path = path;
    state.reading.folder = path.length ? nameOf(path.at(-1)) : "";
    // A wiki space's file: its page tree beside it.
    if (wikiSpace(n.space) && !isPage(n)) {
      if (state.wiki?.space !== n.space) state.wiki = { space: n.space, node: null, path: [], tree: {}, open: new Set(), page: {}, drawer: false };
      if (!state.wiki.tree[0]) pagesUnder(n.space, null).then(() => render(), () => {});
    }
    render();
    const as = viewAs(n);
    if (as && n.version > 0) {
      const bytes = await contents(n);
      if (as === "picture") {
        const ext = n.d.name.split(".").pop().toLowerCase();
        state.reading.picture = `data:${PICTURE_TYPE[ext]};base64,${std64(bytes)}`;
      } else state.reading.text = new TextDecoder().decode(new Uint8Array(bytes).slice(0, 2 << 20));
    } else if (appOf(n) === "write" && n.version > 0) {
      Object.assign(state.reading, forFrame((await api("GET", `/v1/nodes/${n.id}/page`)).html, state.lastFind));
    } else if (n.pictured) {
      showPicture(await api("GET", `/v1/nodes/${n.id}/preview`, undefined, { raw: true }), n);
    }
  } catch (e) {
    state.reading.error = e.message;
  }
  render();
}

/// A file's picture in the reader: a Calc book's tables as a page, a Draw
/// canvas as its picture.
function showPicture(bytes, n) {
  const snap = calcSnapshot(bytes);
  if (snap) Object.assign(state.reading, forFrame(calcPage(snap, nameOf(n)), state.find, { reading: false }));
  else state.reading.picture = pictureOf(bytes);
}

// ---- A space's bin ------------------------------------------------------------

/// What was deleted in a space, to put back.
async function bin(space) {
  state.view = "bin";
  state.reading = null;
  state.sel = null;
  state.find = "";
  state.trash = { space, list: null };
  if (wikiSpace(space)) {
    if (state.wiki?.space !== space) state.wiki = { space, node: null, path: [], tree: {}, open: new Set(), page: {}, drawer: false };
    // The tree beside it (again, after something was put back).
    if (!state.wiki.tree[0]) pagesUnder(space, null).then(() => render(), () => {});
  }
  render();
  try {
    const list = await api("GET", `/v1/trash?space=${space}`);
    for (const n of list) await name(n);
    state.trash.list = list;
  } catch (e) {
    state.trash.error = e.message;
  }
  render();
}

/// How long the bin keeps things (Admin › Backups › Deleted files).
function binKeep(list) {
  const n = list?.find((x) => x.gone);
  if (!list?.length) return "";
  if (!n) return "Things in the bin are kept until someone empties it.";
  const days = Math.round((Date.parse(n.gone) - Date.parse(n.trashed)) / 86400000);
  return `Things in the bin are deleted for good ${days} ${days === 1 ? "day" : "days"} after they were deleted.`;
}

function binPage() {
  const t = state.trash;
  const label = state.spaces.find((x) => x.id === t.space)?.label || "Space";
  const manager = spaceRole(t.space) === "manager";
  document.title = `Bin · ${label}`;
  const rows = t.error ? errorBox(t.error) : t.list === null ? '<p class="empty">…</p>' : !t.list.length
    ? '<p class="empty">Nothing deleted here.</p>'
    : `<div class="plist">${t.list.map((n) => `<div class="pitem slim">${isPage(n) ? `<span class="fmark sm app-write" aria-hidden="true">${icon.page}</span>` : mark(n)}<span class="grow"><b>${esc(isPage(n) ? pageName(n) : nameOf(n))}</b><span class="dim">${esc(isPage(n) ? "Page" : n.kind === "folder" ? "Folder" : kindLabel(n))}${n.with ? ` · with ${n.with} ${isPage(n) ? (n.with === 1 ? "page" : "pages") : n.with === 1 ? "thing" : "things"} under it` : ""} · deleted ${esc(ago(n.trashed))}${n.trashed_by && state.people[n.trashed_by] ? ` by ${esc(state.people[n.trashed_by])}` : ""}${n.gone ? ` · gone for good ${esc(new Date(n.gone).toLocaleDateString(undefined, { day: "numeric", month: "short" }))}` : ""}</span></span><button class="btn small" data-act="restore" data-id="${n.id}" ${state.busy ? "disabled" : ""}>Put back</button>${manager ? `<button class="btn small stop" data-act="forever" data-id="${n.id}" ${state.busy ? "disabled" : ""}>Delete forever</button>` : ""}</div>`).join("")}</div>`;
  const wikiSide = wikiSpace(t.space) && state.wiki?.space === t.space;
  const left = wikiSide ? `<div class="wleft">${places()}${spaceNav(state.wiki, "bin")}</div>` : places();
  return `<div class="files">${header()}<div class="fbody ${wikiSide ? `wiki ${state.wiki.drawer ? "drawer-open" : ""}` : ""}">${left}
    <main class="folder" aria-label="Bin">
      <div class="row ftools">${wikiSide ? `<button class="btn small drawer-btn" data-act="drawer" aria-expanded="${state.wiki.drawer}">${state.wiki.drawer ? "Hide pages" : "Pages"}</button>` : ""}<nav class="crumbs" aria-label="Where"><a href="${spaceHome(t.space)}">${esc(label)}</a><span class="dim">›</span><b>Bin</b></nav></div>
      <div class="row"><p class="dim grow">Deleted pages, files and folders of ${esc(label)}. Putting one back brings back what went with it. ${binKeep(t.list)}</p>
        ${manager && t.list?.length ? `<button class="btn small stop" data-act="empty-bin" ${state.busy ? "disabled" : ""}>Empty the bin…</button>` : ""}</div>
      ${rows}
    </main></div></div>`;
}

// ---- A space's pages (a wiki) ----------------------------------------------
//
// A shared space is a wiki: pages (Write documents marked as pages), each
// able to hold pages of its own. Its other files are under its Files tab.

/// The pages under `parent` (the space's top pages when null).
async function pagesUnder(space, parent) {
  const w = state.wiki;
  const key = parent ?? 0;
  if (!w.tree[key]) {
    const list = await api("GET", `/v1/nodes?space=${space}${parent ? `&parent=${parent}` : ""}`);
    for (const n of list) await name(n);
    w.tree[key] = list.filter(isPage).sort(byOrder);
  }
  return w.tree[key];
}

/// Shows page `id` of `space` (the space's overview when null), or what
/// changed in it lately.
async function wiki(space, id, changes = false) {
  state.view = "wiki";
  state.reading = null;
  state.sel = null;
  state.about = null;
  state.find = "";
  const keep = state.wiki?.space === space ? state.wiki : null;
  const w = (state.wiki = { space, node: null, path: [], tree: keep?.tree || {}, open: keep?.open || new Set(), page: { loading: true }, drawer: false, changes });
  render();
  try {
    let n = null;
    // The home page lives at the space's own address.
    if (id && id === spaceDetails(space).home) return void go(`#/wiki/${space}`, true);
    if (id) {
      n = await api("GET", `/v1/nodes/${id}`);
      await name(n);
      // The pages above it.
      for (let up = n.parent; up; ) {
        const f = await api("GET", `/v1/nodes/${up}`);
        await name(f);
        w.path.unshift(f);
        up = f.parent;
      }
    }
    w.node = n;
    await pagesUnder(space, null);
    for (const f of w.path) {
      w.open.add(f.id);
      await pagesUnder(space, f.id);
    }
    const kids = await pagesUnder(space, n?.id ?? null);
    if (n && kids.length) w.open.add(n.id);
    if (state.wiki !== w) return;
    const page = {};
    // The space's own page: its overview (the page chosen as its home).
    let shown = n;
    if (changes) {
      page.recent = await recentIn(space);
      shown = null;
    } else if (!n) {
      const homeId = spaceDetails(space).home;
      const home = homeId ? await api("GET", `/v1/nodes/${homeId}`).catch(() => null) : null;
      if (home && home.space === space && !home.trashed) {
        await name(home);
        if (isPage(home)) {
          page.home = shown = home;
          await pagesUnder(space, home.id);
        }
      }
    }
    // Only what's written on it.
    if (shown && shown.version > 0) Object.assign(page, forFrame((await api("GET", `/v1/nodes/${shown.id}/page`)).html));
    if (state.wiki === w) w.page = page;
    hasPages(w);
  } catch (e) {
    w.page = { error: e.message };
  }
  if (state.wiki === w) render();
}

/// What changed lately in a space: its pages and files, the latest first.
async function recentIn(space) {
  const all = await api("GET", `/v1/everything?space=${space}`);
  const files = all.filter((x) => x.kind === "file").sort((x, y) => (y.changed || "").localeCompare(x.changed || "")).slice(0, 40);
  for (const x of files) await name(x);
  return files;
}

/// Which of the pages in the tree have pages under them (so the others
/// show no arrow).
async function hasPages(w) {
  const shown = (parent) => [...(w.tree[parent ?? 0] || []), ...(w.tree[parent ?? 0] || []).filter((n) => w.open.has(n.id)).flatMap((n) => shown(n.id))];
  for (const n of shown(null).filter((x) => !w.tree[x.id]).slice(0, 60)) {
    await pagesUnder(w.space, n.id).catch(() => {});
    if (state.wiki !== w) return;
  }
  render();
}

/// The space's pages, nested; the page shown is marked.
function treeHtml(w, parent, depth = 0, current = w.node?.id) {
  const home = spaceDetails(w.space).home;
  const list = w.tree[parent ?? 0]?.filter((n) => n.id !== home);
  if (!list) return depth ? "" : '<p class="dim tmore">…</p>';
  if (!list.length && !(depth === 0 && home)) return depth ? "" : '<p class="dim tmore">No pages yet.</p>';
  // The space's home page: first, as Home, at the space's own address.
  const homeRow = depth === 0 && home && w.tree[0]?.some((n) => n.id === home)
    ? (() => {
      const open = w.open.has(home);
      const kids = w.tree[home];
      const on = current == null && w.filesOn !== true && w.filesOn !== "bin";
      const twist = kids && !kids.length ? '<span class="twist" aria-hidden="true"></span>'
        : `<button class="twist" data-act="tree-toggle" data-id="${home}" aria-label="${open ? "Hide" : "Show"} the pages under Home" aria-expanded="${open}">${open ? "▾" : "▸"}</button>`;
      return `<li role="treeitem"><div class="trow">${twist}<a href="#/wiki/${w.space}" data-ctx="${home}" ${on ? 'class="on" aria-current="page"' : ""}><span class="home-ic" aria-hidden="true">${icon.home}</span><span>${esc(pageName(w.tree[0].find((n) => n.id === home)))}</span></a></div>${open ? treeHtml(w, home, depth + 1, current) : ""}</li>`;
    })()
    : "";
  return `<ul role="${depth ? "group" : "tree"}">${homeRow}${list.map((n) => {
    const on = current === n.id;
    const open = w.open.has(n.id);
    const kids = w.tree[n.id];
    const twist = kids && !kids.length
      ? '<span class="twist" aria-hidden="true"></span>'
      : `<button class="twist" data-act="tree-toggle" data-id="${n.id}" aria-label="${open ? "Hide" : "Show"} the pages under ${esc(pageName(n))}" aria-expanded="${open}">${open ? "▾" : "▸"}</button>`;
    return `<li role="treeitem"><div class="trow">${twist}<a href="#/wiki/${w.space}/${n.id}" data-ctx="${n.id}" ${on ? 'class="on" aria-current="page"' : ""}><span>${esc(pageName(n))}</span></a></div>${open ? treeHtml(w, n.id, depth + 1, current) : ""}</li>`;
  }).join("")}</ul>`;
}

/// A space's side: its name (with its menu, for its managers), its pages
/// as a tree, New page, and its files at the bottom.
function spaceNav(w, filesOn) {
  w.filesOn = filesOn;
  const space = state.spaces.find((x) => x.id === w.space);
  const label = space?.label || "Space";
  const editor = space?.role === "editor" || space?.role === "manager";
  const manager = space?.role === "manager";
  return `<nav class="wtree" aria-label="${esc(label)}">
    <div class="tlabel"><a href="#/wiki/${w.space}" ${!filesOn && !w.node ? 'class="on" aria-current="page"' : ""}><b>${esc(label)}</b></a>${manager ? `<span class="anchor"><button class="more" data-act="menu" data-id="space" aria-haspopup="menu" aria-expanded="${state.menu === "space"}" aria-label="${esc(label)}: settings">⋯</button>
      ${state.menu === "space" ? `<div class="menu spacemenu" role="menu"><button role="menuitem" data-act="space-rename">Rename…</button><button role="menuitem" data-act="space-access">Who can use it…</button><hr><button role="menuitem" class="danger" data-act="space-delete">Delete space…</button></div>` : ""}</span>` : ""}</div>
    ${treeHtml(w, null, 0, filesOn ? null : w.node?.id)}
    ${editor ? `<button class="tnew" data-act="page-new" data-id="">${icon.plus}New page</button>` : ""}
    <div class="tfiles"><a href="#/changes/${w.space}" ${filesOn === "changes" ? 'class="on" aria-current="page"' : ""}><span class="dot sm" aria-hidden="true">${icon.recent}</span><span>Recent changes</span></a>
      <a href="#/space/${w.space}" ${filesOn === true ? 'class="on" aria-current="page"' : ""}><span class="fmark is-folder sm" aria-hidden="true">${icon.folderFill}</span><span>Files</span></a>
      ${editor ? `<a href="#/bin/${w.space}" ${filesOn === "bin" ? 'class="on" aria-current="page"' : ""}><span class="dot sm" aria-hidden="true">${icon.bin}</span><span>Bin</span></a>` : ""}</div>
  </nav>`;
}

function wikiPage() {
  const w = state.wiki;
  const pg = w.page || {};
  const n = w.node;
  const space = state.spaces.find((x) => x.id === w.space);
  const label = space?.label || "Space";
  // The home page is the space's own (its crumb is the space's).
  const homeId = spaceDetails(w.space).home;
  const crumbs = w.changes ? [{ hash: `#/wiki/${w.space}`, name: label }, { name: "Recent changes" }] : [{ hash: `#/wiki/${w.space}`, name: label }, ...w.path.filter((f) => f.id !== homeId).map((f) => ({ hash: `#/wiki/${w.space}/${f.id}`, name: pageName(f) })), ...(n ? [{ name: pageName(n) }] : [])];
  const trail = crumbs.map((c, i) => (i === crumbs.length - 1 ? `<b>${esc(c.name)}</b>` : `<a href="${c.hash}">${esc(c.name)}</a><span class="dim">›</span>`)).join("");
  const spaceEditor = space?.role === "editor" || space?.role === "manager";
  const editor = n && (n.role === "editor" || n.role === "manager");
  const home = !n && pg.home;
  const editing = pageEd.node && pageEd.node.id === (n?.id ?? pg.home?.id);
  const tools = editing
    ? `<button class="btn" data-act="edit-discard">Discard my changes</button><button class="btn" data-act="edit-full">Open in Write</button><button class="btn main" data-act="edit-done">Done</button>`
    : n
    ? `${editor ? `<button class="btn" data-act="edit-page" data-id="${n.id}">${icon.edit}Edit</button><button class="btn" data-act="page-new" data-id="${n.id}">${icon.plus}Add page</button>` : ""}
      <button class="btn more" data-act="ctx" data-id="${n.id}" aria-haspopup="menu" aria-label="More for ${esc(pageName(n))}">⋯</button>`
    : `${home && home.role !== "viewer" ? `<button class="btn" data-act="edit-page" data-id="${home.id}">${icon.edit}Edit</button>` : ""}${spaceEditor ? `<button class="btn" data-act="page-new" data-id="">${icon.plus}New page</button>` : ""}`;
  let body;
  if (state.find.trim()) body = foundHtml();
  else if (pg.error) body = errorBox(pg.error);
  else if (pg.loading) body = '<p class="empty">Opening…</p>';
  else if (editing) body = `<div class="editslot" aria-hidden="true"></div>`;
  else body = w.changes ? changesHtml(w, pg.recent || []) : pageMain(w, n, pg);
  document.title = w.changes ? `Recent changes · ${label}` : n ? `${pageName(n)} · ${label}` : label;
  return `<div class="files">${header()}<div class="fbody wiki ${w.drawer ? "drawer-open" : ""}">
    <div class="wleft">${places()}${spaceNav(w, w.changes ? "changes" : false)}</div>
    <main class="reader" aria-label="${esc(n ? pageName(n) : label)}">
      <div class="row ftools"><button class="btn small drawer-btn" data-act="drawer" aria-expanded="${w.drawer}">${w.drawer ? "Hide pages" : "Pages"}</button>
        <nav class="crumbs" aria-label="Where this page is">${trail}</nav><span class="grow"></span>${state.find.trim() || w.changes ? "" : tools}</div>
      ${body}
    </main>${state.sel ? aboutPanel(state.sel) : ""}</div></div>`;
}

// ---- Editing a page where it's read -----------------------------------
// Write runs in a frame laid over the page's place (it isn't moved as the
// page is drawn again, which would start it over).
const pageEd = { node: null, frame: null, wait: null };

function editInline(n) {
  endEditNow();
  pageEd.node = n;
  const f = document.createElement("iframe");
  f.className = "editframe";
  f.title = `Editing ${pageName(n)}`;
  f.src = `/write/?embed=1&open=${n.id}`;
  document.body.append(f);
  pageEd.frame = f;
  render();
}

/// Done: what's typed is saved, then the page is read again.
async function endEdit({ reload = true } = {}) {
  const n = pageEd.node;
  if (!n) return;
  await new Promise((resolve) => {
    pageEd.wait = resolve;
    pageEd.frame?.contentWindow?.postMessage({ t: "done" }, location.origin);
    setTimeout(resolve, 4000);
  });
  endEditNow();
  if (reload && state.view === "wiki") await wiki(state.wiki.space, state.wiki.node?.id ?? null);
  else render();
}

function endEditNow() {
  pageEd.frame?.remove();
  pageEd.frame = null;
  pageEd.node = null;
  pageEd.wait = null;
}

/// Keeps the frame over the page's place.
function placeEditor() {
  const slot = document.querySelector(".editslot");
  const f = pageEd.frame;
  if (!f) return;
  if (!slot) return void (f.style.display = "none");
  const r = slot.getBoundingClientRect();
  Object.assign(f.style, { display: "block", left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
}
window.addEventListener("resize", placeEditor);
window.addEventListener("scroll", placeEditor, true);
window.addEventListener("message", (e) => {
  if (e.origin !== location.origin || e.data?.xenify !== "write" || e.source !== pageEd.frame?.contentWindow) return;
  if (e.data.t === "saved") pageEd.wait?.();
  if (e.data.t === "deleted") {
    const space = state.wiki?.space;
    endEditNow();
    if (state.wiki) state.wiki.tree = {};
    go(`#/wiki/${space}`);
  }
});

/// The space's own page (its overview) or a page: only what's written on
/// it. The name is in the trail and the tab.
function pageMain(w, n, pg) {
  if (!n) {
    if (pg.home) return pg.html ? frameOf(pg.home, pg) : emptyPage(pg.home);
    return spaceRole(w.space) === "manager"
      ? `<div class="pempty"><p>This space has no overview yet: a page people see first when they open it.</p><div class="row"><button class="btn main" data-act="overview-new">${icon.edit}Write an overview</button></div></div>`
      : '<p class="pempty dim">This space has no overview yet.</p>';
  }
  return pg.html ? frameOf(n, pg) : emptyPage(n);
}

/// What changed lately in a space, the latest first.
function changesHtml(w, list) {
  if (!list.length) return '<p class="pempty dim">Nothing has changed here yet.</p>';
  return `<div class="plist">${list.map((x) => `<a class="pitem slim" href="${isPage(x) ? `#/wiki/${w.space}/${x.id}` : x.parent ? `#/space/${w.space}/${x.parent}` : `#/space/${w.space}`}" data-ctx="${x.id}">${isPage(x) ? "" : mark(x)}<span><b>${esc(isPage(x) ? pageName(x) : nameOf(x))}</b><span class="dim">${esc(isPage(x) ? "Page" : kindLabel(x))} · changed ${esc(ago(x.changed))}${byOf(x) ? ` by ${esc(byOf(x))}` : ""}</span></span></a>`).join("")}</div>`;
}

/// A page with nothing on it (this note isn't part of it).
function emptyPage(n) {
  const can = n.role === "editor" || n.role === "manager";
  return `<p class="pempty dim">This page is empty${can ? `. <button class="link" data-act="edit-page" data-id="${n.id}">Edit</button> to write it.` : "."}</p>`;
}

function frameOf(n, pg) {
  return `<iframe class="page" title="${esc(pageName(n))}" sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox" srcdoc="${esc(pg.html)}"></iframe>`;
}

// ---- Opening in the app -------------------------------------------------

// Apps that open a server document from a link (xenify-<app>://, which
// Xenify Hub sets up) and from #open= in the browser.
const LINKED = ["write", "plan"];

/// Starts the desktop app on file `n`. When nothing takes the link, offers
/// the app in this browser.
function openInApp(n) {
  const app = appOf(n) || "write";
  if (!LINKED.includes(app)) {
    state.dialog = { kind: "noapp", node: n, app, unlinked: true };
    return render();
  }
  const url = `xenify-${app}://open?server=${encodeURIComponent(location.origin)}&node=${n.id}`;
  let left = false;
  const gone = () => { left = true; };
  window.addEventListener("blur", gone, { once: true });
  document.addEventListener("visibilitychange", gone, { once: true });
  location.href = url;
  setTimeout(() => {
    window.removeEventListener("blur", gone);
    document.removeEventListener("visibilitychange", gone);
    if (!left) {
      state.dialog = { kind: "noapp", node: n, app };
      render();
    }
  }, 1500);
}

/// Opens a folder (into it), a page (in its space's pages) or a file: in
/// its app, as a page when it has no app but can be read, else it's
/// downloaded.
function openNode(n) {
  if (!n) return;
  if (n.kind === "folder") {
    const p = state.place;
    if (!state.find && state.view === "browse" && p.kind === "space") go(placeHash({ ...p, path: [...p.path, { id: n.id, name: nameOf(n) }] }));
    else go(`#/space/${n.space}/${n.id}`);
    return;
  }
  if (isPage(n) && wikiSpace(n.space)) return void (go(`#/wiki/${n.space}/${n.id}`));
  if (appOf(n)) openApp(n);
  else if (readable(n)) go(`#/read/${n.id}`);
  else acts.download({ dataset: { id: n.id } });
}

/// A file in its app: in this browser when the server has the apps, else
/// on this computer.
function openApp(n) {
  const app = appOf(n);
  noteBack();
  if (app && webApps()) location.href = inBrowser(n, app);
  else openInApp(n);
}

/// Where an app's File › Quit comes back to: here (the page or folder
/// it was opened from).
function noteBack() {
  try {
    sessionStorage.setItem("xenify-back", location.href);
    // The app goes back a step when the history is just one longer then.
    sessionStorage.setItem("xenify-back-len", String(history.length));
  } catch {}
}

function inBrowser(n, app) {
  return `/${app}/?open=${n.id}`;
}

// ---- Sharing --------------------------------------------------------------

async function makeLink(root) {
  const r = await api("POST", `/v1/nodes/${root.id}/public`);
  return linkUrl(r.token);
}

const linkUrl = (token) => `${location.origin}/p/${token}`;

async function shareDialog(n) {
  const folder = n.parent ? await api("GET", `/v1/nodes/${n.parent}`).catch(() => null) : null;
  if (folder) await name(folder);
  const [own, parent] = await Promise.all([
    api("GET", `/v1/nodes/${n.id}/public`).catch(() => null),
    folder ? api("GET", `/v1/nodes/${folder.id}/public`).catch(() => null) : null,
  ]);
  if (!own) throw new Error("Only people who can change it can share it.");
  const d = { kind: "share", node: n, folder: parent ? folder : null, allowed: own.allowed, mode: "off", url: "", notice: "" };
  // Who on this server has it: for whoever may manage it.
  if (n.role === "manager") {
    const [access, roles] = await Promise.all([
      api("GET", `/v1/nodes/${n.id}/access`).catch(() => null),
      api("GET", "/v1/roles").catch(() => null),
    ]);
    if (access) d.access = { own: !!access.own, grants: (access.grants || []).map((g) => ({ ...g })) };
    d.custom = (roles?.roles || []).filter((r) => r.kind === "document");
  }
  const held = own.link ? { on: n, link: own.link, mode: "this" } : parent?.link ? { on: folder, link: parent.link, mode: "folder" } : null;
  if (held) {
    d.mode = held.mode;
    d.url = linkUrl(held.link.token);
  }
  state.dialog = d;
  render();
}

async function setShare(mode) {
  const d = state.dialog;
  const stopAll = async () => {
    await api("DELETE", `/v1/nodes/${d.node.id}/public`).catch(() => {});
    if (d.folder) await api("DELETE", `/v1/nodes/${d.folder.id}/public`).catch(() => {});
  };
  const had = d.url;
  if (mode === "off") {
    await stopAll();
    d.url = "";
    d.notice = had ? "Link stopped: it doesn't work any more." : "";
  } else {
    await stopAll();
    d.url = await makeLink(mode === "folder" ? d.folder : d.node);
    d.notice = had ? "New link made: the old one doesn't work any more." : "Link made: anyone you give it to can read it.";
  }
  d.mode = mode;
  d.confirmNew = false;
  if (state.sel) choose(state.sel);
}

const ROLES = [["viewer", "Can view"], ["commenter", "Can comment"], ["editor", "Can edit"], ["manager", "Can manage"]];

/// Who on this server has the file or folder in the share dialog, and how.
function peopleHtml(d) {
  const a = d.access;
  const me = state.session?.account?.id;
  const nameOfWho = (w) => (w === "everyone" ? `Everyone on ${state.info?.name || "this server"}` : w.user ? state.people[w.user] || "Someone" : `${state.groups[w.group] || "A group"} (group)`);
  // Draw and Calc have no comments: "Can comment" would only be "Can view".
  const noComments = ["draw", "calc"].includes(appOf(d.node));
  const roleSel = (g, i) => `<select class="inp" data-change="share-role" data-i="${i}" aria-label="What ${esc(nameOfWho(g.who))} can do">
    ${ROLES.filter(([v]) => v !== "commenter" || !noComments || g.role === v).map(([v, l]) => `<option value="${v}" ${!g.custom && g.role === v ? "selected" : ""}>${l}</option>`).join("")}
    ${(d.custom || []).map((r) => `<option value="c${r.id}" ${g.custom === r.id ? "selected" : ""}>${esc(r.name)}</option>`).join("")}</select>`;
  const taken = new Set(a.grants.map((g) => JSON.stringify(g.who)));
  const add = `<select class="inp" data-change="share-add" aria-label="Add a person or group"><option value="">Add a person or group…</option>
    ${state.dir.groups.filter((g) => !taken.has(JSON.stringify({ group: g.id }))).map((g) => `<option value="g${g.id}">${esc(g.name)} (group)</option>`).join("")}
    ${state.dir.people.filter((p) => p.id !== me && !taken.has(JSON.stringify({ user: p.id }))).map((p) => `<option value="u${p.id}">${esc(p.display_name ? (p.email ? `${p.display_name} (${p.email})` : p.display_name) : p.email || p.username)}</option>`).join("")}</select>`;
  const rows = a.grants.map((g, i) => `<div class="grant"><span class="gname">${esc(nameOfWho(g.who))}</span>${g.who?.user === me ? '<span class="dim">You</span>' : `${roleSel(g, i)}<button class="btn small" data-act="share-remove" data-i="${i}" aria-label="Remove ${esc(nameOfWho(g.who))}">Remove</button>`}</div>`).join("");
  return `<div class="stack tight">
    <label class="check"><input type="radio" name="share-own" data-change="share-own" value="same" ${a.own ? "" : "checked"}>The same people as the ${d.node.parent ? "folder it's in" : "space"}</label>
    <label class="check"><input type="radio" name="share-own" data-change="share-own" value="own" ${a.own ? "checked" : ""}>Only these people</label>
    ${a.own ? `${add}${rows || '<p class="dim">Nobody but you yet.</p>'}` : ""}
    ${d.changed ? `<div class="row end"><button class="btn main small" data-act="share-save" ${state.busy ? "disabled" : ""}>Save who has it</button></div>` : ""}</div>`;
}

/// A new space, or who can use one: everyone on the server (the usual), or
/// only the people and groups chosen. Its managers always stay.
function spaceDialog(d) {
  const nameOfWho = (w) => (w.user ? state.people[w.user] || "Someone" : `${state.groups[w.group] || "A group"} (group)`);
  const me = state.session?.account?.id;
  const taken = new Set(d.list.map((g) => JSON.stringify(g.who)));
  const add = `<select class="inp" data-change="space-add" aria-label="Add a person or group"><option value="">Add a person or group…</option>
    ${state.dir.groups.filter((g) => !taken.has(JSON.stringify({ group: g.id }))).map((g) => `<option value="g${g.id}">${esc(g.name)} (group)</option>`).join("")}
    ${state.dir.people.filter((p) => p.id !== me && !taken.has(JSON.stringify({ user: p.id }))).map((p) => `<option value="u${p.id}">${esc(p.display_name ? (p.email ? `${p.display_name} (${p.email})` : p.display_name) : p.email || p.username)}</option>`).join("")}</select>`;
  const roleSel = (g, i) => `<select class="inp" data-change="space-role" data-i="${i}" aria-label="What ${esc(nameOfWho(g.who))} can do">${[["viewer", "Can view"], ["editor", "Can edit"], ["manager", "Can manage"]].map(([v, l]) => `<option value="${v}" ${g.role === v ? "selected" : ""}>${l}</option>`).join("")}</select>`;
  const rows = d.list.map((g, i) => `<div class="grant"><span class="gname">${esc(nameOfWho(g.who))}</span>${g.who.user === me ? '<span class="dim">You · can manage</span>' : `${roleSel(g, i)}<button class="btn small" type="button" data-act="space-remove" data-i="${i}" aria-label="Remove ${esc(nameOfWho(g.who))}">Remove</button>`}</div>`).join("");
  return `<form class="stack" data-act="space-save"><h2>${d.id ? `Who can use ${esc(d.label)}` : "New space"}</h2>
    ${d.id ? "" : '<p class="dim">A space is a wiki: pages, with pages under them, and its own files.</p>'}${errorBox(d.error)}
    ${d.id ? "" : `<label class="field"><span>Name</span><input class="inp" data-bind="spaceName" value="${esc(state.form.spaceName || "")}" autofocus></label>`}
    <fieldset class="stack tight"><legend>Who can use it</legend>
      <label class="check"><input type="radio" name="space-mode" data-change="space-mode" value="everyone" ${d.mode === "everyone" ? "checked" : ""}>Everyone on this server can read and change it</label>
      <label class="check"><input type="radio" name="space-mode" data-change="space-mode" value="read" ${d.mode === "read" ? "checked" : ""}>Everyone can read it; the people I choose can change it</label>
      <label class="check"><input type="radio" name="space-mode" data-change="space-mode" value="chosen" ${d.mode === "chosen" ? "checked" : ""}>Only people I choose</label>
      ${d.mode !== "everyone" ? `${add}${rows}` : ""}
    </fieldset>
    <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>${d.id ? "Save" : "Make the space"}</button></div></form>`;
}

/// How to use this server from the desktop apps: Xenify Hub.
function hubDialog() {
  return `<div class="stack"><div class="row"><h2 class="grow">Get the desktop apps</h2><button class="more" data-act="close" aria-label="Close">✕</button></div>
    <p>Xenify Hub installs Write, Draw, Calc, Plan and Chat on your computer, keeps them up to date, and signs them in to this server.</p>
    <ol class="steps-list">
      <li><a href="https://xenify.net/download/" target="_blank" rel="noopener">Download Xenify Hub</a> and install it.</li>
      <li>Open Hub and click <b>Not managed by a server</b> at the top (or the Settings button).</li>
      <li>Under <b>Xenify server</b>, type this address:
        <div class="linkrow"><input class="inp" readonly aria-label="This server's address" value="${esc(location.origin)}" id="hubaddr"><button class="btn" data-act="copy-addr">Copy</button></div></li>
      <li>Type your email address, then your password.</li>
    </ol>
    <p class="dim">That's all: every app on that computer then opens and saves files here.</p>
    <div class="row end"><button class="btn main" data-act="close">Done</button></div></div>`;
}

/// Changes made with lists and choices (not clicks).
const changes = {
  "move-to"(el) {
    state.dialog.to = el.value ? +el.value : null;
    render();
  },
  "space-mode"(el) {
    state.dialog.mode = el.value;
    render();
  },
  "space-add"(el) {
    const v = el.value;
    if (!v) return;
    state.dialog.list.push({ who: v[0] === "g" ? { group: +v.slice(1) } : { user: +v.slice(1) }, role: "editor" });
    render();
  },
  "space-role"(el) {
    state.dialog.list[+el.dataset.i].role = el.value;
    render();
  },
  "share-own"(el) {
    const d = state.dialog;
    d.access.own = el.value === "own";
    d.changed = true;
    render();
  },
  "share-add"(el) {
    const d = state.dialog;
    const v = el.value;
    if (!v) return;
    const who = v[0] === "g" ? { group: +v.slice(1) } : { user: +v.slice(1) };
    d.access.grants.push({ who, role: appOf(d.node) === "draw" || appOf(d.node) === "calc" ? "viewer" : "editor" });
    d.changed = true;
    render();
  },
  "share-role"(el) {
    const d = state.dialog;
    const g = d.access.grants[+el.dataset.i];
    const c = (d.custom || []).find((r) => `c${r.id}` === el.value);
    if (c) Object.assign(g, { role: c.base, custom: c.id });
    else {
      g.role = el.value;
      delete g.custom;
    }
    d.changed = true;
    render();
  },
};

// ---- Public links -------------------------------------------------------

async function openPublic() {
  const token = location.pathname.split("/")[2] || "";
  state.view = "read";
  render();
  try {
    const p = await api("GET", `/v1/p/${encodeURIComponent(token)}`);
    for (const n of p.items) n.d = details(n.meta);
    state.pub = { token, items: p.items, root: p.root, server: p.server };
    const root = p.items.find((n) => n.id === p.root);
    document.title = root ? nameOf(root) : "Shared with you";
    const first = root?.kind === "file" ? root : pubFiles()[0];
    if (first) await readPublic(first.id);
    else render();
  } catch (e) {
    state.reading = { error: e.code === "links_off" ? "Public links are turned off on this server." : e.code === "gone" ? "This link doesn't work any more. Ask whoever shared it for a new one." : e.message };
    render();
  }
}

function pubFiles() {
  return (state.pub?.items || []).filter((n) => n.kind === "file").sort((x, y) => nameOf(x).localeCompare(nameOf(y)));
}

function pubContents(n) {
  return api("GET", `/v1/p/${state.pub.token}/content/${n.id}?file=true`, undefined, { raw: true });
}

async function readPublic(id) {
  const n = state.pub.items.find((x) => x.id === id);
  state.reading = { node: n };
  render();
  try {
    if (appOf(n) === "write" && n.version > 0) {
      Object.assign(state.reading, forFrame((await api("GET", `/v1/p/${state.pub.token}/page/${n.id}`)).html));
    } else if (n.pictured) {
      showPicture(await api("GET", `/v1/p/${state.pub.token}/preview/${n.id}`, undefined, { raw: true }), n);
    }
  } catch (e) {
    state.reading.error = e.message;
  }
  render();
}

// ---- Drawing --------------------------------------------------------------

function render() {
  const root = document.getElementById("app");
  const focus = document.activeElement?.dataset?.bind;
  // Signed out, the tab says only "Sign in" (see signinPage).
  if (state.mode !== "public" && (state.view === "signin" || state.view === "browse")) document.title = state.view === "signin" ? "Sign in" : browseTitle();
  root.innerHTML = state.view === "loading" ? "" : state.view === "signin" ? signinPage() : state.mode === "public" ? publicPage() : state.view === "read" ? readPage() : state.view === "wiki" ? wikiPage() : state.view === "bin" ? binPage() : state.view === "search" ? searchPage() : browsePage();
  root.insertAdjacentHTML("beforeend", ctxMenu() + dialog() + (state.toast ? `<div class="toast" role="status">${esc(state.toast)}${state.undo ? ' <button class="link undo" data-act="undo">Undo</button>' : ""}</div>` : ""));
  if (focus) {
    const el = root.querySelector(`[data-bind="${focus}"]`);
    if (el) {
      el.focus();
      if (el.setSelectionRange && typeof el.value === "string") el.setSelectionRange(el.value.length, el.value.length);
    }
  }
  if (state.mode === "files") root.querySelectorAll('a[href^="#/"]').forEach((a) => a.setAttribute("href", placePath(a.getAttribute("href"))));
  focusOpened();
  placeEditor();
  placeMenu();
  sizeFrame();
}

// A dialog that just opened: its first field (or button) has the keys, so
// typing goes straight in.
let shownDialog = null;
function focusOpened() {
  const opened = state.dialog && state.dialog !== shownDialog;
  shownDialog = state.dialog;
  if (!opened) return;
  const box = document.querySelector(".dialog");
  const el = box?.querySelector("[autofocus], input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([readonly]):not([disabled]), textarea") || box?.querySelector(".btn.main, button");
  el?.focus();
}

/// Grows the page's frame to the page's height (it can't run scripts of
/// its own; its document is readable from here). The height is the
/// content's (its body, whose height doesn't follow the frame's), set on
/// the next frame and only when it changed, so setting it never sets off
/// the watcher again. The watcher stops when the frame goes.
let frameWatch = null;
// The page last shown and its height: drawn again (the same page), its new
// frame starts that tall and where it was, not at the top for a moment.
const lastPage = { doc: "", height: 0 };
function sizeFrame() {
  const f = document.querySelector("iframe.page");
  if (frameWatch && frameWatch.frame !== f) {
    frameWatch.observer?.disconnect();
    cancelAnimationFrame(frameWatch.pending);
    frameWatch = null;
  }
  if (!f || frameWatch) return;
  const watch = (frameWatch = { frame: f, observer: null, pending: 0, height: 0 });
  const doc = f.getAttribute("srcdoc") || "";
  if (doc === lastPage.doc && lastPage.height) {
    f.style.height = `${lastPage.height}px`;
    watch.height = lastPage.height;
    f.shown = true;
  }
  lastPage.doc = doc;
  const measure = () => {
    watch.pending = 0;
    try {
      const doc = f.contentDocument;
      if (!doc?.body || !f.isConnected) return;
      const cs = doc.defaultView.getComputedStyle(doc.body);
      const h = Math.ceil(doc.body.getBoundingClientRect().height + parseFloat(cs.marginTop) + parseFloat(cs.marginBottom));
      if (Math.abs(h - watch.height) > 1) {
        watch.height = h;
        lastPage.height = h;
        f.style.height = `${Math.max(h, 40)}px`;
      }
    } catch {}
  };
  const later = () => {
    if (!watch.pending) watch.pending = requestAnimationFrame(measure);
  };
  const scrollTo = (el) => window.scrollTo({ top: f.getBoundingClientRect().top + window.scrollY + el.getBoundingClientRect().top - 90 });
  // Opened from a search: to the first word found; opened with #a-heading: to that heading.
  const first = () => {
    const m = f.contentDocument?.querySelector("mark.found");
    const id = location.hash.startsWith("#/") ? "" : decodeURIComponent(location.hash.slice(1));
    const h = id ? f.contentDocument?.getElementById(id) : null;
    if (!f.shown && (h || m)) scrollTo(h?.parentElement || m);
    f.shown = true;
  };
  // A link to another page: shown here, not inside the frame.
  const links = () => {
    const d = f.contentDocument;
    if (!d || d.wired) return;
    d.wired = true;
    d.addEventListener("click", (e) => {
      // Within the page (contents entries, a heading's #): the page scrolls; a heading's # also copies its link.
      const inside = e.target.closest?.('a[href^="#"]:not([data-page])');
      if (inside) {
        e.preventDefault();
        const id = decodeURIComponent(inside.getAttribute("href").slice(1));
        const el = f.contentDocument.getElementById(id);
        if (el) scrollTo(inside.classList.contains("hl") ? el.parentElement : el);
        if (inside.classList.contains("hl") && state.mode !== "public") {
          history.replaceState(null, "", `${location.pathname}${location.search}#${id}`);
          navigator.clipboard?.writeText(location.href).then(() => toast("Link to this heading copied"), () => {});
        }
        return;
      }
      const a = e.target.closest?.("a[data-page]");
      if (!a) return;
      e.preventDefault();
      go(`#/page/${a.dataset.page}`);
    });
  };
  const ready = () => {
    const body = f.contentDocument?.body;
    if (body && !watch.observer && frameWatch === watch) {
      watch.observer = new ResizeObserver(later);
      watch.observer.observe(body);
    }
    measure();
  };
  f.addEventListener("load", () => { ready(); first(); links(); });
  // Already loaded (it can be, by the time this runs): the same, now.
  if (f.contentDocument?.readyState === "complete" && f.contentDocument.body?.childElementCount) { ready(); first(); links(); }
}

const errorBox = (msg = state.error) => (msg ? `<div class="error" role="alert">${esc(msg)}</div>` : "");

/// The bar on every signed-in page: Files, the apps in the browser, the
/// admin pages (for whoever may open them) and you.
function header({ searchable = true } = {}) {
  const me = state.session?.account;
  const who = me ? esc(me.display_name || me.email || me.username) : "";
  // Where to search: everywhere, or the space you're looking at (and the
  // one picked before, while its results are shown) — never a list of
  // every space, which gets too long.
  const here = currentSpace();
  const shown = state.spaces.filter((x) => x.id === here || x.id === state.findScope);
  const scope = shown.length
    ? `<select class="scope" data-bind="findScope" aria-label="Where to search"><option value="">Everywhere</option>${shown.map((x) => `<option value="${x.id}" ${state.findScope === x.id ? "selected" : ""}>Only in ${esc(x.label)}</option>`).join("")}</select>`
    : "";
  return `<header class="fhead">
    <a class="brand" href="#/recent"><span class="mark" aria-hidden="true">X</span><span class="sname">${esc(state.info?.name || "Xenify server")}</span></a>
    ${searchable ? `<div class="searchbox"><label class="search">${icon.search}<input aria-label="Search everything" placeholder="Search everything" data-bind="find" value="${esc(state.find)}"></label>${scope}</div>` : '<span style="flex:1"></span>'}
    ${me ? topNew() : ""}
    ${me && webApps() ? `<a class="chatbtn" href="/chat/" data-act="to-chat" title="Chat" aria-label="Chat">${icon.chat}</a>` : ""}
    ${me ? `<span class="anchor mebox"><button class="me" style="background:#0F766E;color:#fff" data-act="menu" data-id="me" aria-haspopup="menu" aria-expanded="${state.menu === "me"}" aria-label="Signed in as ${who}">${esc(initials(me.display_name || me.email || me.username))}</button>
      ${state.menu === "me" ? `<div class="menu" role="menu"><p class="menunote"><b>${who}</b><br><span class="dim">${esc(me.email || me.username)}</span></p>
        ${state.powers.length ? '<a role="menuitem" href="/admin/">Admin</a>' : ""}
        <button role="menuitem" data-act="hub">Get the desktop apps</button><hr>
        <button role="menuitem" data-act="signout">Sign out</button></div>` : ""}</span>` : ""}
  </header>`;
}

/// The space being looked at (a space's pages or files), for "Only in …".
function currentSpace() {
  if (state.view === "wiki") return state.wiki?.space ?? null;
  if (state.view === "browse" && state.place?.kind === "space") return state.place.space;
  return null;
}

/// Where New puts things: the folder shown (when you may add there), the
/// space's files (on its pages), else My files.
function newTarget() {
  const p = state.place;
  const mine = state.spaces.find((x) => x.kind === "personal");
  // Only where you may add: otherwise My files, saying why.
  const shownSpace = state.view === "wiki" || state.view === "bin" ? state.wiki?.space : state.view === "browse" && p?.kind === "space" ? p.space : null;
  const role = shownSpace != null ? spaceRole(shownSpace) : null;
  const label = state.spaces.find((x) => x.id === shownSpace)?.label || "this space";
  if (state.view === "wiki" && (role === "editor" || role === "manager")) {
    return { space: state.wiki.space, parent: null, where: `the files of ${label}` };
  }
  if (state.view === "browse" && p?.kind === "space" && canAddHere()) {
    return { space: p.space, parent: p.path.at(-1)?.id ?? null, where: p.path.at(-1)?.name || label };
  }
  const why = shownSpace != null && shownSpace !== mine?.id ? `You can only read ${label}, so it goes in My files` : "";
  return { space: mine?.id, parent: null, where: "My files", why };
}

/// Whether the folder shown takes new things from you.
function canAddHere() {
  const p = state.place;
  if (state.view !== "browse" || p?.kind !== "space") return false;
  if (p.path.length) return true;
  const role = state.spaces.find((x) => x.id === p.space)?.role;
  return role === "editor" || role === "manager";
}

/// New, at the top: a page (on a space's pages), a folder or upload (in a
/// folder you may add to), and a document, spreadsheet, drawing or plan
/// (made where newTarget() says, then opened in its app).
/// New document or page: what it starts from (blank, or a template: the
/// server's Templates space, and documents of the space marked "Use as a
/// template"). Loaded when the dialog opens.
function templateChoice(d) {
  if (d.templates == null) {
    d.templates = [];
    const space = d.into?.space ?? d.target?.space ?? state.place?.space;
    const tsp = state.spaces.find((x) => x.kind === "templates");
    Promise.all([
      tsp ? api("GET", `/v1/everything?space=${tsp.id}`).catch(() => []) : [],
      space != null && space !== tsp?.id ? api("GET", `/v1/everything?space=${space}`).catch(() => []) : [],
    ]).then(async ([shared, here]) => {
      for (const x of [...shared, ...here]) await name(x);
      const write = (x) => x.kind === "file" && (x.d?.app === "write" || /\.(xwrite|igdoc)$/i.test(x.d?.name || ""));
      d.templates = [...here.filter((x) => write(x) && x.d?.template), ...shared.filter(write)];
      if (state.dialog === d) render();
    });
  }
  if (!d.templates.length) return "";
  return `<label class="field"><span>Start from</span><select class="inp" data-bind="newTemplate"><option value="">A blank document</option>${d.templates.map((t) => `<option value="${t.id}" ${String(state.form.newTemplate || "") === String(t.id) ? "selected" : ""}>${esc(isPage(t) ? pageName(t) : nameOf(t).replace(/\.(xwrite|igdoc)$/i, ""))}</option>`).join("")}</select></label>`;
}

function topNew() {
  const open = state.menu === "new";
  const t = newTarget();
  const order = ["write", "calc", "draw", "plan"].map((a) => NEW_KINDS.find((k) => k.app === a));
  const w = state.view === "wiki" ? state.wiki : null;
  const space = w && state.spaces.find((x) => x.id === w.space);
  const pageOk = w && (space?.role === "editor" || space?.role === "manager");
  const here = canAddHere();
  return `<span class="anchor"><button class="btn main topnew" data-act="menu" data-id="new" aria-haspopup="menu" aria-expanded="${open}">${icon.plus}New</button>
    ${open ? `<div class="menu newmenu" role="menu">
      ${pageOk ? `${w.node ? `<button role="menuitem" data-act="page-new" data-id="${w.node.id}"><span class="fmark sm app-write" aria-hidden="true">${icon.page}</span>Page under “${esc(pageName(w.node))}”</button>` : ""}
        <button role="menuitem" data-act="page-new" data-id=""><span class="fmark sm app-write" aria-hidden="true">${icon.page}</span>Page at the top of ${esc(space.label)}</button><hr>` : ""}
      ${here ? `<button role="menuitem" data-act="new-folder"><span class="fmark is-folder sm" aria-hidden="true">${icon.folderFill}</span>Folder</button><hr>` : ""}
      <p class="menunote dim">${t.why ? `${esc(t.why)}.` : `In ${esc(t.where)}`}</p>
      ${order.map((k) => `<button role="menuitem" data-act="new-file" data-id="${k.app}"><span class="fmark sm app-${k.app}" aria-hidden="true">${k.letter}</span>${k.what}<span class="dim">${APPS[k.app]}</span></button>`).join("")}
      ${here ? `<hr><button role="menuitem" data-act="upload"><span class="dot" aria-hidden="true">${icon.up}</span>Upload files…</button>` : ""}</div>` : ""}</span>`;
}

/// The second step after the password: a code, or setting one up when the
/// server requires it (an authenticator app).
function stepPage() {
  const m = state.step;
  const head = `<div class="solo plain"><main>`;
  const remember = '<label class="check"><input type="checkbox" data-act="step-remember" checked>Remember this computer for 30 days</label>';
  if (m.codes) {
    return `${head}<div class="sheet narrow stack"><div><h1>Keep these backup codes</h1><p class="dim">Each works once, when you can't get a code (a lost phone, say). They won't be shown again.</p></div>
      <div class="codes">${m.codes.map(esc).join("<br>")}</div>
      <div class="row end"><button class="btn main" data-act="step-kept">I've kept them</button></div></div></main></div>`;
  }
  if (m.setup) {
    return `${head}<form class="sheet narrow" data-act="step-finish"><div><h1>Set up two-step sign-in</h1><p class="dim">This server asks for a code after your password. Scan this with your authenticator app, or type the key in, then give the code it shows.</p></div>
      ${errorBox()}${m.app ? `<div class="mfa-qr">${m.app.qr_svg}</div><p class="key">${esc(m.app.secret.replace(/(.{4})/g, "$1 ").trim())}</p>` : ""}
      <label class="field"><span>Code from the app</span><input class="inp" data-bind="code" autocomplete="one-time-code" placeholder="123456"></label>${remember}
      <div class="row end"><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>Turn it on</button></div></form></main></div>`;
  }
  const what = { app: "Code from your authenticator app", email: "Code from the email we sent", backup: "Backup code" }[m.method];
  const others = m.methods.filter((x) => x !== m.method).map((x) => `<button class="link" type="button" data-act="step-method" data-id="${x}">${{ app: "Use the authenticator app", email: "Email me a code", backup: "Use a backup code" }[x]}</button>`).join("");
  return `${head}<form class="sheet narrow" data-act="step-code"><div><h1>Two-step sign-in</h1><p class="dim">Your password was right. Now the second step.</p></div>
    ${errorBox()}<label class="field"><span>${what}</span><input class="inp" data-bind="code" autocomplete="one-time-code" placeholder="${m.method === "backup" ? "xxxx-xxxx" : "123456"}"></label>${remember}
    <div class="row">${others}<span class="grow"></span><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>Sign in</button></div></form></main></div>`;
}

// The email address first; then the password, or (an account without a
// password yet, or a forgotten one) a code, emailed or from an admin, and
// a new password.
const mails = () => !!state.info?.features?.includes("mail");
function signinFields() {
  const stage = state.form.stage || "user";
  const btn = (t) => `<button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>${t}</button>`;
  if (stage === "user") {
    return `<label class="field"><span>Email</span><input class="inp" inputmode="email" data-bind="username" autocomplete="username" value="${esc(state.form.username || "")}"></label>
      <div class="row">${btn("Next")}</div>`;
  }
  const who = `<p class="dim">${esc(state.form.username)} · <button class="link" type="button" data-act="signin-back">Change</button></p>`;
  if (stage === "password") {
    return `${who}<label class="field"><span>Password</span><input class="inp" type="password" data-bind="password" autocomplete="current-password"></label>
      <div class="row">${btn("Sign in")}<span class="grow"></span><button class="link" type="button" data-act="signin-forgot">Forgot password?</button></div>`;
  }
  if (stage === "code") {
    return `${who}<p>${esc(state.form.note || "")}</p>
      <label class="field"><span>Code</span><input class="inp" data-bind="code" autocomplete="one-time-code" spellcheck="false" value="${esc(state.form.code || "")}"></label>
      <div class="row">${btn("Next")}<span class="grow"></span>${state.form.emailed ? '<button class="link" type="button" data-act="signin-email">Send a new code</button>' : ""}</div>`;
  }
  return `${who}<p>${state.form.kind === "reset" ? "Pick a new password." : `Welcome to ${esc(state.info?.name || "this server")}. Pick your password.`}</p>
      <label class="field"><span>Password</span><input class="inp" type="password" data-bind="password" autocomplete="new-password"></label>
      <label class="field"><span>Password again</span><input class="inp" type="password" data-bind="password2" autocomplete="new-password"></label>
      <div class="row">${btn("Set password and sign in")}</div>`;
}

/// Has the server email the person signing in a code, and asks for it.
async function emailCode(note) {
  const f = state.form;
  await api("POST", "/v1/forgot", { username: (f.username || "").trim().toLowerCase() });
  Object.assign(f, { stage: "code", note, code: "", emailed: true });
}

function signinPage() {
  if (state.step) return stepPage();
  // A plain sign-in page: nothing about what the server is or runs.
  return `<div class="solo plain"><main>
    <form class="sheet narrow" data-act="signin">
      <div><h1>Sign in</h1></div>
      ${errorBox()}
      ${signinFields()}
    </form></main></div>`;
}

function places() {
  const p = state.place;
  const on = (k, s) => ((state.view === "wiki" ? k === "space" && state.wiki?.space === s : p.kind === k && (s === undefined || p.space === s) && state.view === "browse" && !state.find) ? 'class="on" aria-current="page"' : "");
  const spaces = state.spaces.filter((s) => s.kind === "shared" || s.kind === "templates");
  const mine = state.spaces.find((s) => s.kind === "personal");
  return `<nav class="places" aria-label="Places">
    <a href="#/recent" ${on("recent")}><span class="dot">${icon.recent}</span>Recent</a>
    ${mine ? `<a href="#/space/${mine.id}" ${on("space", mine.id)}><span class="dot">M</span>My files</a>` : ""}
    <a href="#/shared" ${on("shared")}><span class="dot">${icon.shared}</span>Shared with me</a>
    ${spaces.length || canMakeSpaces() ? `<div class="label">Spaces</div>${spaces.map((s) => `<a href="${spaceHome(s.id)}" ${on("space", s.id)}><span class="dot" style="background:var(--ember-tint);color:var(--ember-dark)">${esc(initials(s.label)[0])}</span>${esc(s.label)}</a>`).join("")}` : ""}
    ${canMakeSpaces() ? `<button class="tnew" data-act="space-new">${icon.plus}New space</button>` : ""}
  </nav>`;
}

const canMakeSpaces = () => !!state.session?.account?.admin || state.perms.includes("files.spaces");

/// The spaces you can open (again, after making, renaming or removing one).
async function loadSpaces() {
  const spaces = await api("GET", "/v1/spaces");
  state.spaces = [];
  for (const s of spaces.filter((s) => s.kind !== "chat")) {
    s.label = s.kind === "personal" ? "My files" : s.kind === "templates" ? "Templates" : details(s.name).name || "Untitled space";
    state.spaces.push(s);
  }
}

const byOf = (n) => (n.changed_by && state.people[n.changed_by]) || "";

/// Something found: a click opens it (a document found by its words: read
/// here, the words marked).
function hitHtml(n) {
  const where = [n.spaceLabel || state.spaces.find((s) => s.id === n.space)?.label || "", ...(n.path || [])].filter(Boolean).join(" › ");
  const act = n.kind === "file" && !isPage(n) && readable(n) && n.snippet ? "read-hit" : "open";
  return `<div class="hitrow" data-ctx="${n.id}"><button class="hit" data-act="${act}" data-id="${n.id}">${mark(n)}<span><b>${esc(isPage(n) ? pageName(n) : nameOf(n))}</b><br><span class="dim">${esc([kindLabel(n), where].filter(Boolean).join(" · "))}</span>${snippet(n)}</span></button>${moreBtn(n)}</div>`;
}

/// The ⋯ button with a file's, folder's or page's menu.
const moreBtn = (n) => `<button class="more" data-act="ctx" data-id="${n.id}" aria-haspopup="menu" aria-label="More for ${esc(nameOf(n))}">⋯</button>`;

function tile(n) {
  const pic = state.thumbs[n.id];
  const sub = n.kind === "folder" ? "Folder" : `${kindLabel(n)} · ${ago(n.changed)}`;
  return `<div class="tile is-${n.kind} ${state.sel?.id === n.id ? "on" : ""}" data-ctx="${n.id}">
    <button class="topen" data-act="open" data-id="${n.id}">
      <span class="thumb">${pic ? `<img src="${pic}" alt="">${mark(n)}` : mark(n, true)}</span>
      <span class="tname"><b>${esc(nameOf(n))}</b><span class="dim">${esc(sub)}</span></span>
    </button>${moreBtn(n)}</div>`;
}

/// The folder as a list, like a file explorer: sortable by each column.
function listView(items) {
  const s = state.sort;
  const head = (key, label, cls = "") => `<th scope="col" class="${cls}" ${s.key === key ? `aria-sort="${s.desc ? "descending" : "ascending"}"` : ""}><button class="sorter" data-act="sort" data-id="${key}">${label}${s.key === key ? `<span aria-hidden="true">${s.desc ? " ↓" : " ↑"}</span>` : ""}</button></th>`;
  const row = (n) => {
    const kind = n.kind === "folder" ? "Folder" : kindLabel(n);
    const when = n.kind === "folder" ? "" : ago(n.changed);
    return `<tr class="${state.sel?.id === n.id ? "on" : ""}" data-ctx="${n.id}">
      <td class="nm"><button class="topen" data-act="open" data-id="${n.id}">${mark(n)}<span class="nmt"><b>${esc(nameOf(n))}</b><span class="dim sub">${esc([kind, when].filter(Boolean).join(" · "))}</span></span></button></td>
      <td class="c-kind">${esc(kind)}</td><td class="c-when">${esc(when)}</td><td class="c-by">${esc(n.kind === "folder" ? "" : byOf(n))}</td><td class="c-more">${moreBtn(n)}</td></tr>`;
  };
  return `<table class="flist"><thead><tr>${head("name", "Name")}${head("kind", "Kind", "c-kind")}${head("changed", "Changed", "c-when")}${head("by", "By", "c-by")}<th class="c-more"><span class="sr">More</span></th></tr></thead>
    <tbody>${items.map(row).join("")}</tbody></table>`;
}

/// What the folder shows: narrowed by the filter, folders first, in the
/// order picked.
function shown() {
  const f = state.filter.trim().toLowerCase();
  const items = f ? state.items.filter((n) => nameOf(n).toLowerCase().includes(f)) : state.items.slice();
  const { key, desc } = state.sort;
  const val = { name: nameOf, kind: (n) => (n.kind === "folder" ? "" : kindLabel(n)), changed: (n) => n.changed || "", by: byOf }[key];
  const order = (x, y) => {
    if (x.kind !== y.kind) return x.kind === "folder" ? -1 : 1;
    if (!val) return 0;
    const a = val(x), b = val(y);
    const c = key === "changed" ? (a < b ? -1 : a > b ? 1 : 0) : String(a).localeCompare(String(b), undefined, { numeric: true, sensitivity: "base" });
    return desc ? -c : c;
  };
  return items.sort(order);
}

// List or icons: chosen here (kept in this browser), else a list once a
// folder has many things in it.
const VIEW_KEY = "xenify-files-view";
function viewChoice() {
  try {
    const v = localStorage.getItem(VIEW_KEY);
    if (v === "list" || v === "icons") return v;
  } catch {}
  return "";
}
const viewOf = () => viewChoice() || (state.items.length > 24 ? "list" : "icons");

/// The tab's title in Files: the folder, then where it is.
function browseTitle() {
  const p = state.place;
  if (!p) return "Files";
  if (p.kind !== "space") return p.kind === "shared" ? "Shared with me" : "Recent";
  const label = state.spaces.find((x) => x.id === p.space)?.label || "Space";
  const at = p.path.at(-1)?.name;
  return wikiSpace(p.space) ? `${at ? `${at} · ` : ""}Files · ${label}` : at ? `${at} · ${label}` : label;
}

function browsePage() {
  const p = state.place;
  let title;
  if (p.kind === "space") {
    const s = state.spaces.find((x) => x.id === p.space);
    const top = wikiSpace(p.space)
      ? [{ hash: `#/wiki/${p.space}`, name: s?.label || "Space" }, { hash: `#/space/${p.space}`, name: "Files" }]
      : [{ hash: `#/space/${p.space}`, name: s?.label || "Space" }];
    const crumbs = [...top, ...p.path.map((f, i) => ({ hash: placeHash({ ...p, path: p.path.slice(0, i + 1) }), name: f.name }))];
    title = crumbs.map((c, i) => (i === crumbs.length - 1 ? `<b>${esc(c.name)}</b>` : `<a href="${c.hash}">${esc(c.name)}</a><span class="dim">›</span>`)).join("");
  } else title = `<b>${p.kind === "recent" ? "Recent" : "Shared with me"}</b>`;
  let body;
  if (state.find.trim()) {
    title = "";
    body = foundHtml();
  } else {
    const items = shown();
    body = !state.items.length
      ? `<p class="empty">${p.kind === "recent" ? "Files you can open show up here once they're changed." : p.kind === "shared" ? "Nothing has been shared with you on its own yet." : "Nothing here yet."}</p>`
      : !items.length
        ? `<p class="empty">Nothing here has “${esc(state.filter.trim())}” in its name.</p>`
        : viewOf() === "list" ? listView(items) : `<div class="tiles">${items.map(tile).join("")}</div>`;
  }
  const view = viewOf();
  // The bin of My files or Templates (a wiki's is in its tree).
  const binLink = p.kind === "space" && !wikiSpace(p.space) && !p.path.length && canAddHere()
    ? `<a class="btn small" href="#/bin/${p.space}">${icon.bin}Bin</a>` : "";
  const tools = binLink + (!state.find && state.items.length
    ? `<label class="ffilter">${icon.search}<input aria-label="Filter this folder" placeholder="Filter this folder" data-bind="filter" value="${esc(state.filter)}"></label>
      <div class="seg viewseg" role="radiogroup" aria-label="Show as">${[["list", "List", icon.list], ["icons", "Icons", icon.grid]].map(([v, label, ic]) => `<button role="radio" aria-checked="${view === v}" class="${view === v ? "on" : ""}" data-act="view" data-id="${v}" title="${label}">${ic}<span>${label}</span></button>`).join("")}</div>`
    : "");
  // A space's files sit beside its pages, like one of them.
  const wikiFiles = p.kind === "space" && wikiSpace(p.space);
  if (wikiFiles && state.wiki?.space !== p.space) {
    state.wiki = { space: p.space, node: null, path: [], tree: {}, open: new Set(), page: {}, drawer: false };
    const w = state.wiki;
    pagesUnder(p.space, null).then(() => state.wiki === w && render(), () => {});
  }
  const left = wikiFiles ? `<div class="wleft">${places()}${spaceNav(state.wiki, true)}</div>` : places();
  return `<div class="files">${header()}<div class="fbody ${wikiFiles ? `wiki ${state.wiki.drawer ? "drawer-open" : ""}` : ""}">${left}
    <main class="folder" aria-label="Files">
      <div class="row ftools">${wikiFiles ? `<button class="btn small drawer-btn" data-act="drawer" aria-expanded="${state.wiki.drawer}">${state.wiki.drawer ? "Hide pages" : "Pages"}</button>` : ""}${title ? `<nav class="crumbs" aria-label="Folder path">${title}</nav>` : ""}<span class="grow"></span>
        ${tools}</div>
      ${errorBox()}${body}
    </main>${state.sel ? aboutPanel(state.sel) : ""}</div></div>`;
}

/// What a search found (everywhere, or in the space shown).
function foundHtml() {
  const hits = state.everything || [];
  const only = state.findScope ? state.spaces.find((x) => x.id === state.findScope)?.label : "";
  const head = `<h1 class="ptitle">Found for “${esc(state.find.trim())}”${only ? ` in ${esc(only)}` : ""}</h1>`;
  return head + (state.everything === null ? '<p class="empty">Looking…</p>' : (hits.length
    ? `<div class="hits">${hits.map(hitHtml).join("")}</div>`
    : `<p class="empty">Nothing${only ? ` in ${esc(only)}` : ""} has that in its name or words.</p>`) + aiFindHtml());
}


function aboutPanel(n) {
  const a = state.about;
  const app = appOf(n);
  const editor = n.role === "editor" || n.role === "manager";
  const changed = n.kind === "file" ? `Changed ${ago(n.changed)}${n.changed_by && state.people[n.changed_by] ? ` by ${state.people[n.changed_by]}` : ""}${a?.versions ? ` · ${a.versions} version${a.versions === 1 ? "" : "s"}` : ""}` : "Folder";
  const acts = n.kind === "folder"
    ? `<button class="btn main wide" data-act="open" data-id="${n.id}">Open folder</button>`
    : app
      ? `<button class="btn main wide" data-act="open" data-id="${n.id}">Open</button>`
      : `<button class="btn wide" data-act="download" data-id="${n.id}">${icon.down}Download</button>`;
  // Other ways to open it, small: as a page here, or in the app on this computer.
  const also = n.kind === "file" && app
    ? [readable(n) ? `<a href="#/read/${n.id}">Read here</a>` : "", webApps() ? `<button class="link" data-act="edit" data-id="${n.id}">Open in the ${esc(APPS[app] || "")} app on this computer</button>` : ""].filter(Boolean).join(" · ")
    : "";
  const pub = a?.pub?.link ? '<div class="live"><i></i>Public link on · read only</div>' : "";
  return `<aside class="about" aria-label="Details of ${esc(nameOf(n))}">
    <div class="row"><h3 class="grow">Details</h3><button class="more" data-act="details-close" aria-label="Close the details">✕</button></div>
    <div class="atitle">${mark(n, true)}<div><span class="kind ${n.kind === "folder" ? "is-folder" : app || ""}">${esc(n.kind === "folder" ? "Folder" : kindLabel(n))}</span>
      <h2>${esc(nameOf(n))}</h2><div class="dim">${esc(changed)}</div></div></div>
    <div class="acts">${acts}</div>
    ${app ? `<p class="dim also">Opens in ${esc(APPS[app] || "its app")}${webApps() ? " in this browser" : " on this computer"}. ${also}</p>` : ""}
    <section><h3>Who can see it</h3><div style="font-size:14px;color:var(--ink-2)">${a ? esc(a.who || "—") : "…"}</div>${pub}
      ${editor ? `<button class="btn" style="align-self:flex-start" data-act="share" data-id="${n.id}">Share…</button>` : ""}</section>
  </aside>`;
}

function readPage() {
  const r = state.reading || {};
  const n = r.node;
  const app = n && appOf(n);
  const editor = n && (n.role === "editor" || n.role === "manager");
  // Beside it: a wiki space's page tree (its files are under Files there),
  // else the other files in its folder.
  const wikiSide = n && wikiSpace(n.space) && !isPage(n) && state.wiki?.space === n.space;
  const siblings = wikiSide
    ? `<div class="wleft">${places()}${spaceNav(state.wiki, true)}</div>`
    : r.siblings?.length > 1
      ? `<nav class="places" aria-label="Pages"><div class="label" style="padding-top:4px">${esc(r.folder || "In this folder")}</div>${r.siblings.map((s) => `<a href="#/read/${s.id}" ${s.id === n.id ? 'class="on" aria-current="page"' : ""}>${esc(nameOf(s))}</a>`).join("")}</nav>`
      : places();
  if (n) {
    const label = state.spaces.find((x) => x.id === n.space)?.label || "";
    document.title = label ? `${nameOf(n)} · ${label}` : nameOf(n);
  }
  // Where it is: the space (› Files, in a wiki space) › its folders.
  const crumbs = n ? (() => {
    const label = state.spaces.find((x) => x.id === n.space)?.label || "Space";
    const top = wikiSpace(n.space) && !isPage(n)
      ? [{ hash: `#/wiki/${n.space}`, name: label }, { hash: `#/space/${n.space}`, name: "Files" }]
      : [{ hash: spaceHome(n.space), name: label }];
    const all = [...top, ...(r.path || []).map((f) => ({ hash: isPage(f) ? `#/wiki/${n.space}/${f.id}` : `#/space/${n.space}/${f.id}`, name: isPage(f) ? pageName(f) : nameOf(f) })), { name: nameOf(n) }];
    return `<nav class="crumbs" aria-label="Where it is">${all.map((c, i) => (i === all.length - 1 ? `<b>${esc(c.name)}</b>` : `<a href="${c.hash}">${esc(c.name)}</a><span class="dim">›</span>`)).join("")}</nav>`;
  })() : "";
  return `<div class="files">${header()}<div class="fbody ${wikiSide ? `wiki ${state.wiki.drawer ? "drawer-open" : ""}` : ""}">${siblings}
    <main class="reader" aria-label="${esc(n ? nameOf(n) : "Page")}">
      ${n ? `<div class="rbar">${wikiSide ? `<button class="btn small drawer-btn" data-act="drawer" aria-expanded="${state.wiki.drawer}">${state.wiki.drawer ? "Hide pages" : "Pages"}</button>` : ""}${crumbs}<span class="grow"></span>
        ${editor ? `<button class="btn" data-act="share" data-id="${n.id}">Share…</button>` : ""}
        ${app ? `<button class="btn main" data-act="edit-page" data-id="${n.id}">${icon.edit}${editor ? "Edit" : "Open"} in ${APPS[app]}</button>` : `<button class="btn main" data-act="download" data-id="${n.id}">${icon.down}Download</button>`}</div>` : ""}
      ${pageBody(r)}
    </main></div></div>`;
}

function placeHashOf(n) {
  if (isPage(n) && wikiSpace(n.space)) return `#/wiki/${n.space}/${n.id}`;
  return n.parent ? `#/space/${n.space}/${n.parent}` : `#/space/${n.space}`;
}

function pageBody(r) {
  if (r.error) return errorBox(r.error);
  if (r.html) return `<iframe class="page" title="The page" sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox" srcdoc="${esc(r.html)}"></iframe>`;
  if (r.picture) return `<img class="picture" src="${r.picture}" alt="${viewAs(r.node) === "picture" ? esc(nameOf(r.node)) : `A picture of ${esc(nameOf(r.node))}`}">`;
  if (r.text !== undefined) return `<pre class="textfile">${esc(r.text)}</pre>`;
  if (r.node && r.node.version === 0) return '<p class="empty">Nothing has been saved in it yet.</p>';
  if (r.node && appOf(r.node) !== "write") return `<p class="empty">This kind of file can't be shown here.</p>${state.mode === "public" ? `<div class="row" style="justify-content:center"><button class="btn" data-act="download" data-id="${r.node.id}">${icon.down}Download</button></div>` : ""}`;
  return '<p class="empty">Opening…</p>';
}

function publicPage() {
  const p = state.pub;
  const r = state.reading || {};
  const files = pubFiles();
  const root = p?.items.find((n) => n.id === p.root);
  const tree = root?.kind === "folder" && files.length
    ? `<nav class="places" aria-label="Pages"><div class="label" style="padding-top:4px">${esc(nameOf(root))}</div>${files.map((n) => `<a href="#${esc(p.key || "")}" data-act="pub-read" data-id="${n.id}" ${r.node?.id === n.id ? 'class="on" aria-current="page"' : ""}>${esc(nameOf(n))}</a>`).join("")}</nav>`
    : "";
  return `<div class="files"><header class="fhead"><div class="brand"><span class="mark" aria-hidden="true">X</span>${esc(root ? nameOf(root) : "Shared with you")}</div><span class="grow"></span>
      <span class="chip">${icon.read.replace("<svg", '<svg width="14" height="14"')} Read only</span></header>
    <div class="fbody">${tree}
      <main class="reader" aria-label="${esc(r.node ? nameOf(r.node) : "Page")}">
        ${r.node ? `<p class="dim">${esc(nameOf(r.node))} · updated ${esc(new Date(r.node.changed).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" }))}</p>` : ""}
        ${pageBody(r)}
        ${p?.server ? `<p class="dim" style="margin-top:auto">Shared from ${esc(p.server)}.</p>` : ""}
      </main></div></div>`;
}

function dialog() {
  const d = state.dialog;
  if (!d) return "";
  let inner;
  if (d.kind === "folder") {
    inner = `<form class="stack" data-act="make-folder"><h2>New folder</h2>${errorBox(d.error)}
      <label class="field"><span>Name</span><input class="inp" data-bind="folderName" value="${esc(state.form.folderName || "")}" autofocus></label>
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit">Make folder</button></div></form>`;
  } else if (d.kind === "file") {
    const k = NEW_KINDS.find((x) => x.app === d.app);
    const where = d.target?.where || state.place?.path?.at(-1)?.name || state.spaces.find((x) => x.id === state.place?.space)?.label || "this folder";
    const about = d.into
      ? `${d.into.parent ? `A page under “${esc(d.into.where)}”` : `A page at the top of ${esc(d.into.where)}`}, in the space’s pages. It opens in Write for you to write it.`
      : `A ${esc(k.what.toLowerCase())} in ${esc(where)} (a file, not a page). It opens in ${APPS[k.app]}.`;
    inner = `<form class="stack" data-act="make-file"><div><h2>New ${d.into ? "page" : esc(k.what.toLowerCase())}</h2><p class="dim">${about}</p></div>${errorBox(d.error)}
      <label class="field"><span>Name</span><input class="inp" data-bind="fileName" value="${esc(state.form.fileName || "")}" autofocus></label>
      ${d.app === "write" ? templateChoice(d) : ""}
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>Make it</button></div></form>`;
  } else if (d.kind === "space") {
    inner = spaceDialog(d);
  } else if (d.kind === "space-rename") {
    inner = `<form class="stack" data-act="space-rename-save"><h2>Rename the space</h2>${errorBox(d.error)}
      <label class="field"><span>Name</span><input class="inp" data-bind="spaceName" value="${esc(state.form.spaceName || "")}" autofocus></label>
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>Rename</button></div></form>`;
  } else if (d.kind === "space-delete") {
    inner = `<div class="stack"><h2>Delete “${esc(d.label)}”?</h2>${errorBox(d.error)}<p>Its pages and files go with it, for everyone. This can't be undone.</p>
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main danger" data-act="space-delete-yes" ${state.busy ? "disabled" : ""}>Delete space</button></div></div>`;
  } else if (d.kind === "forever") {
    inner = `<div class="stack"><h2>${d.all ? "Empty the bin?" : "Delete forever?"}</h2>${errorBox(d.error)}
      <p>${d.all ? `Everything in the bin of ${esc(d.label)} is deleted for good` : `“${esc(d.name)}”${d.with ? ` and what went with it` : ""} is deleted for good`}, with its versions. This can't be undone.</p>
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main danger" data-act="forever-yes" ${state.busy ? "disabled" : ""}>${d.all ? "Empty the bin" : "Delete forever"}</button></div></div>`;
  } else if (d.kind === "hub") {
    inner = hubDialog();
  } else if (d.kind === "rename") {
    inner = `<form class="stack" data-act="rename-save"><h2>Rename ${d.node.kind === "folder" ? "folder" : isPage(d.node) ? "page" : esc(kindLabel(d.node).toLowerCase())}</h2>${errorBox(d.error)}
      <label class="field"><span>Name</span><input class="inp" data-bind="newName" value="${esc(state.form.newName || "")}" autofocus></label>
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" type="submit" ${state.busy ? "disabled" : ""}>Rename</button></div></form>`;
  } else if (d.kind === "move") {
    const what = isPage(d.node) ? "page" : d.node.kind === "folder" ? "folder" : "file";
    const top = state.spaces.find((x) => x.id === d.node.space)?.label || "the space";
    const label = (x) => (isPage(x) ? pageName(x) : nameOf(x));
    const now = d.all ? [top, ...pathOf(d.all, d.node.parent).map(label)].join(" › ") : "…";
    const opt = (id, text, depth) => `<label class="mopt" style="padding-left:${10 + depth * 18}px"><input type="radio" name="move-to" data-change="move-to" value="${id ?? ""}" ${d.to === id ? "checked" : ""}>${id === null ? "" : isPage(d.node) ? "" : `<span class="fmark is-folder sm" aria-hidden="true">${icon.folderFill}</span>`}<span>${esc(text)}</span>${id === (d.node.parent ?? null) ? '<span class="dim">(where it is now)</span>' : ""}</label>`;
    const rows = (parent, depth) => (d.kids?.get(parent) || []).map((x) => opt(x.id, label(x), depth) + rows(x.id, depth + 1)).join("");
    const list = d.all ? `<div class="mlist mtree" role="radiogroup" aria-label="Where to">${opt(null, isPage(d.node) ? `At the top of ${top}` : top, 0)}${rows(null, 1)}</div>` : '<p class="dim">…</p>';
    inner = `<div class="stack"><h2>Move “${esc(label(d.node))}”</h2><p class="dim">Now in: <b>${esc(now)}</b>. ${isPage(d.node) ? "Choose the page it goes under, or the top of the space." : "Choose the folder it goes into."}</p>${errorBox(d.error)}
      ${list}
      <div class="row end"><button class="btn" type="button" data-act="close">Cancel</button><button class="btn main" data-act="move-here" ${state.busy || d.to === undefined || d.to === (d.node.parent ?? null) ? "disabled" : ""}>Move the ${what}</button></div></div>`;
  } else if (d.kind === "delete") {
    const n = d.node;
    const u = d.under;
    const pages = u?.filter(isPage) || [];
    const what = isPage(n)
      ? `the page “${esc(pageName(n))}”${pages.length ? ` and the ${pages.length === 1 ? "page" : `${pages.length} pages`} under it` : ""}`
      : n.kind === "folder" ? `the folder “${esc(nameOf(n))}”${u?.length ? ` and the ${u.length === 1 ? "thing" : `${u.length} things`} in it` : ""}` : `“${esc(nameOf(n))}”`;
    const list = u?.length ? `<ul class="gone">${u.slice(0, 8).map((x) => `<li>${esc(isPage(x) ? pageName(x) : nameOf(x))}</li>`).join("")}${u.length > 8 ? `<li class="dim">and ${u.length - 8} more</li>` : ""}</ul>` : "";
    const bin = n.kind === "file" || n.kind === "folder" ? " It goes to the space’s bin, where it can be put back." : "";
    inner = `<div class="stack"><h2>Delete ${isPage(n) ? "page" : n.kind === "folder" ? "folder" : "file"}?</h2>${errorBox(d.error)}<p>This deletes ${what}, for everyone who has it.${bin}</p>${(isPage(n) || n.kind === "folder") && u === null ? '<p class="dim">…</p>' : list}
      <div class="row end"><button class="btn" data-act="close">Cancel</button><button class="btn main danger" data-act="delete-yes" ${state.busy ? "disabled" : ""}>Delete</button></div></div>`;
  } else if (d.kind === "versions") {
    const write = appOf(d.node) === "write";
    const when = (v) => esc(new Date(v.created).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }));
    const by = (a) => (a && state.people[a] ? esc(state.people[a]) : "");
    const vname = (n) => (d.list?.[0]?.n === n ? "Now" : `Version ${n}`);
    if (d.view) {
      // One version opened, two compared, or who wrote each paragraph.
      const v = d.view;
      const body = v.html == null && v.paragraphs == null ? '<p class="dim">…</p>'
        : v.paragraphs ? `<div class="blame">${v.paragraphs.map((p) => `<div class="brow"><span class="bwho" title="Unchanged since version ${p.n}">${by(p.author) || "Someone"} · ${vname(p.n)}<br><span class="dim">${esc(new Date(p.created).toLocaleDateString(undefined, { dateStyle: "medium" }))}</span></span><span class="btext">${esc(p.text)}</span></div>`).join("") || '<p class="dim">It has no words yet.</p>'}</div>`
        : `<iframe class="vpage" title="${esc(v.title)}" sandbox="allow-same-origin" srcdoc="${esc(v.html)}"></iframe>`;
      inner = `<div class="stack"><div class="row"><button class="btn small" data-act="version-back">‹ All versions</button><h2 class="grow">${esc(v.title)}</h2><button class="more" data-act="close" aria-label="Close">✕</button></div>${v.note ? `<p class="dim">${v.note}</p>` : ""}${errorBox(d.error)}${body}</div>`;
    } else {
      const rows = d.list === null ? '<p class="dim">…</p>' : d.list.length
        ? `<div class="vlist">${d.list.map((v, i) => `<div class="vrow"><span><b>${vname(v.n)}</b><span class="dim">${when(v)}${by(v.author) ? ` · ${by(v.author)}` : ""}${v.label ? ` · ${esc(v.label)}` : ""}</span></span>${write
          ? `<button class="btn small" data-act="version-open" data-id="${v.n}">Open</button>${i < d.list.length - 1 ? `<button class="btn small" data-act="version-compare" data-id="${v.n}" data-from="${d.list[i + 1].n}" title="What changed since the version before">Compare</button>` : ""}<button class="btn small" data-act="version-blame" data-id="${v.n}" title="Each paragraph with who wrote it and when">Who wrote what</button><button class="more" data-act="version-get" data-id="${v.n}" title="Download this version" aria-label="Download version ${v.n}">${icon.down}</button>`
          : `<button class="btn small" data-act="version-get" data-id="${v.n}">${icon.down}Download</button>`}</div>`).join("")}</div>`
        : '<p class="dim">Nothing has been saved in it yet.</p>';
      inner = `<div class="stack"><div class="row"><h2 class="grow">Versions of “${esc(nameOf(d.node))}”</h2><button class="more" data-act="close" aria-label="Close">✕</button></div>${errorBox(d.error)}${rows}</div>`;
    }
  } else if (d.kind === "noapp") {
    const app = APPS[d.app] || "The app";
    inner = d.unlinked
      ? `<h2>Open it from ${esc(app)}</h2><p>${esc(app)} can't be started from this page yet. In ${esc(app)}, choose <b>File › Open from ${esc(state.info?.name || "the server")}</b> and pick “${esc(nameOf(d.node))}”.</p>
        <div class="row end"><button class="btn main" data-act="close">OK</button></div>`
      : `<h2>${esc(app)} didn't open</h2><p>It may not be installed on this computer. Xenify Hub installs it and keeps it up to date.</p>
        <div class="row end"><a class="btn" href="https://xenify.net/download/" target="_blank" rel="noopener">Get Xenify Hub</a>
        ${webApps() ? `<a class="btn main" href="${esc(inBrowser(d.node, d.app))}">Open in this browser</a>` : '<button class="btn main" data-act="close">OK</button>'}</div>`;
  } else if (d.kind === "share") {
    const n = d.node;
    const opts = [["off", "Off"], ["this", n.kind === "folder" ? "This folder" : "This page"], ...(d.folder ? [["folder", `All of “${nameOf(d.folder)}”`]] : [])];
    const link = d.allowed
      ? `<div class="seg wide" role="radiogroup" aria-label="Public link">${opts.map(([m, label]) => `<button role="radio" aria-checked="${d.mode === m}" class="${d.mode === m ? "on" : ""}" data-act="share-mode" data-id="${m}" ${state.busy ? "disabled" : ""}>${esc(label)}</button>`).join("")}</div>
      ${d.notice ? `<p class="live"><i></i>${esc(d.notice)}</p>` : ""}
      ${d.url ? `<div class="linkrow"><input class="inp" readonly aria-label="Public link" value="${esc(d.url)}" id="publink"><button class="btn main" data-act="copy">Copy</button></div>
        ${d.confirmNew
          ? `<div class="note stack"><span>A new link replaces this one: <b>the old link stops working at once</b>, for everyone you gave it to.</span>
            <div class="row end"><button class="btn small" data-act="share-new-no">Keep this link</button><button class="btn small main" data-act="share-mode" data-id="${d.mode}">Make a new link</button></div></div>`
          : `<div class="row"><span class="dim grow">Only one link works at a time.</span><button class="btn small" data-act="share-new">New link…</button><button class="btn small stop" data-act="share-mode" data-id="off">Stop link</button></div>`}` : ""}`
      : `<p class="note">Public links are turned off on ${esc(state.info?.name || "this server")}. Ask an admin if you need one.</p>`;
    inner = `<div class="row"><h2 class="grow">Share “${esc(nameOf(n))}”</h2><button class="more" data-act="close" aria-label="Close">✕</button></div>
      ${errorBox(d.error)}
      <section class="sharepart"><h3>People on this server</h3><p class="dim">They need no link: it shows under <b>Shared with me</b> for them, and they open it with their own sign-in.</p>
        ${d.access ? peopleHtml(d) : '<p class="note">Only people who can manage it choose who has it.</p>'}</section>
      <section class="sharepart"><h3>Anyone with the link</h3><p class="dim">For people without an account: they can read it in a browser, not change it.</p>${link}</section>
      <div class="row end"><button class="btn main" data-act="close">Done</button></div>`;
  }
  const wide = d.kind === "versions" && (d.view || appOf(d.node) === "write") ? " wide" : "";
  return `<div class="backdrop" data-act="backdrop"><div class="dialog${wide}" role="dialog" aria-modal="true">${inner}</div></div>`;
}

/// The open menu of a file, folder or page: what can be done with it.
function ctxMenu() {
  const c = state.ctx;
  if (!c) return "";
  const n = c.node;
  const editor = n.role === "editor" || n.role === "manager";
  const page = isPage(n);
  const item = (act, label, cls = "") => `<button role="menuitem" class="${cls}" data-act="${act}" data-id="${n.id}">${label}</button>`;
  const items = [];
  if (page) {
    if (state.wiki?.node?.id !== n.id) items.push(item("open", "Open page"));
    if (editor) items.push(item("edit-page", "Edit"), item("page-new", "Add page under it"));
    items.push(item("copy-link", "Copy link"));
    if (editor && siblingsOf(n)?.length > 1) items.push(item("order-up", "Move up"), item("order-down", "Move down"));
    if (spaceRole(n.space) === "manager") items.push(spaceDetails(n.space).home === n.id ? item("unhome", "Stop being the space’s home") : item("make-home", "Make it the space’s home"));
  } else items.push(item("open", "Open"), item("copy-link", "Copy link"));
  if (n.kind === "file" && !page && readable(n)) items.push(item("read", "Read here"));
  if (editor) items.push(item("rename", "Rename…"), item("move", "Move to…"));
  if (n.kind === "file") items.push(item("download", "Download"));
  if (editor) items.push(item("share", "Share…"));
  if (n.kind === "file") items.push(item("versions", "Version history"));
  if (editor && !page && appOf(n) === "write" && wikiSpace(n.space)) items.push(item("make-page", "Make it a page"));
  // Any Write document can be what new documents and pages start from (New › Document, Add page).
  if (editor && appOf(n) === "write" && n.kind === "file") items.push(item("template-toggle", n.d?.template ? "Stop using as a template" : "Use as a template"));
  if (n.kind === "file") items.push(item("details", "Details"));
  if (editor) items.push("<hr>", item("delete", "Delete…", "danger"));
  return `<div class="menu ctxmenu" role="menu" aria-label="${esc(nameOf(n))}" style="left:${c.x}px;top:${c.y}px">${items.join("")}</div>`;
}

/// Keeps the open menu on the screen, and gives it the keys.
function placeMenu() {
  const m = document.querySelector(".ctxmenu");
  if (!m) return;
  const r = m.getBoundingClientRect();
  const left = Math.max(8, Math.min(state.ctx.x, innerWidth - r.width - 8));
  const top = state.ctx.y + r.height > innerHeight - 8 ? Math.max(8, state.ctx.y - r.height - (state.ctx.h || 0)) : state.ctx.y;
  m.style.left = `${left}px`;
  m.style.top = `${top}px`;
  if (!m.contains(document.activeElement)) m.querySelector("button")?.focus({ preventScroll: true });
}

/// Opens the menu of `id` at a point on the screen (or below an element).
async function openCtx(id, x, y, h = 0) {
  let n = find(id);
  if (!n) {
    n = await api("GET", `/v1/nodes/${id}`).catch(() => null);
    if (n) await name(n);
  }
  if (!n) return;
  state.menu = null;
  state.ctx = { node: n, x, y, h, fromMore: !!document.activeElement?.matches?.('[data-act="ctx"]') };
  render();
}

/// Closes the menu; the keys go back to what it was for (the page is
/// drawn again meanwhile, so found anew).
function closeCtx(focusBack = true) {
  const c = state.ctx;
  state.ctx = null;
  render();
  if (!focusBack || !c) return;
  const id = c.node.id;
  const el = (c.fromMore && document.querySelector(`[data-act="ctx"][data-id="${id}"]`))
    || document.querySelector(`[data-ctx="${id}"] .topen, [data-ctx="${id}"] .hit, a[data-ctx="${id}"]`);
  el?.focus({ preventScroll: true });
}

/// Everything shown again from the server (after a change).
function refresh() {
  state.everything = null;
  if (state.view === "wiki") {
    const w = state.wiki;
    w.tree = {};
    return wiki(w.space, w.node?.id ?? null);
  }
  if (state.view === "read" && state.reading?.node) return read(state.reading.node.id);
  if (state.view === "bin") return bin(state.trash.space);
  if (state.view === "search") return found(state.find, state.findScope);
  return show(state.place);
}

/// A move dialog's choices: every folder (or, for a page, every page) of
/// the space, as a tree, but not the thing moved nor anything under it.
async function moveList(d) {
  const all = await api("GET", `/v1/everything?space=${d.node.space}`).catch(() => []);
  for (const x of all) await name(x);
  if (state.dialog !== d) return;
  const below = new Set([d.node.id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const x of all) if (x.parent != null && below.has(x.parent) && !below.has(x.id)) { below.add(x.id); grew = true; }
  }
  const fits = (x) => !below.has(x.id) && (isPage(d.node) ? isPage(x) : x.kind === "folder");
  d.kids = new Map();
  for (const x of all.filter(fits)) {
    const key = x.parent ?? null;
    if (!d.kids.has(key)) d.kids.set(key, []);
    d.kids.get(key).push(x);
  }
  for (const l of d.kids.values()) l.sort(isPage(d.node) ? byOrder : (a, b) => nameOf(a).localeCompare(nameOf(b), undefined, { numeric: true }));
  d.all = all;
  render();
}

/// The folders or pages above `parent` (from the top), in `all`.
function pathOf(all, parent) {
  const out = [];
  for (let up = parent; up; ) {
    const f = all.find((x) => x.id === up);
    if (!f) break;
    out.unshift(f);
    up = f.parent;
  }
  return out;
}

// ---- Clicks and typing --------------------------------------------------

function busy(f) {
  state.busy = true;
  state.error = "";
  render();
  return f()
    .catch((e) => {
      if (state.dialog) state.dialog.error = e.message;
      else state.error = e.message;
    })
    .finally(() => {
      state.busy = false;
      render();
    });
}

/// A short note at the bottom; `undo` adds an Undo button (and the note
/// stays longer, so there's time to press it).
function toast(t, undo = null) {
  state.toast = t;
  state.undo = undo;
  render();
  setTimeout(() => {
    if (state.toast === t) {
      state.toast = "";
      state.undo = null;
      render();
    }
  }, undo ? 9000 : 2500);
}

const find = (id) =>
  state.items.find((n) => n.id === id)
  || state.everything?.find((n) => n.id === id)
  || (state.reading?.node?.id === id ? state.reading.node : null)
  || (state.sel?.id === id ? state.sel : null)
  || (state.ctx?.node?.id === id ? state.ctx.node : null)
  || (state.wiki && [state.wiki.node, ...state.wiki.path, ...Object.values(state.wiki.tree).flat(), ...(state.wiki.hits || [])].find((n) => n?.id === id))
  || null;

const acts = {
  "ai-search": () => aiSearch(),
  signin() {
    return busy(async () => {
      const f = state.form;
      const username = (f.username || "").trim().toLowerCase();
      if (!f.stage || f.stage === "user") {
        if (!username) throw new Error("Type your email address.");
        const step = await api("POST", "/v1/login/start", { username });
        if (step?.next !== "pick_password") return void (f.stage = "password");
        if (step.can_email) return emailCode("We’ve emailed you a code. Type it here, then pick your password.");
        Object.assign(f, { stage: "code", note: "This account doesn’t have a password yet. Type the code your admin gave you, then pick one." });
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
        if ((f.password || "").length < 8) throw new Error("Use a password of at least 8 characters.");
        if (f.password !== f.password2) throw new Error("The two passwords aren't the same.");
        s = await api("POST", "/v1/link/use", { link: f.code.trim(), username, password: f.password, device: "Files in a browser" });
      } else s = await api("POST", "/v1/login", { username, password: state.form.password || "", device: "Files in a browser", remember: remembered() });
      if (s.ticket) {
        state.step = { ticket: s.ticket, methods: s.methods, method: s.methods[0], setup: s.setup, remember: true };
        if (s.setup) state.step.app = await api("POST", "/v1/mfa/app/start", { ticket: s.ticket });
        state.form = {};
        return;
      }
      await signedIn(s);
    });
  },
  "signin-back"() {
    state.form = { username: state.form.username };
    state.error = "";
    render();
  },
  "signin-email"() {
    return busy(() => emailCode("A new code is on its way to your email."));
  },
  "signin-forgot"() {
    if (!mails()) {
      state.error = "Ask an admin to reset your password: you then pick a new one when you sign in.";
      return render();
    }
    return busy(() => emailCode("If this account has an email address, a code is on its way to it. Type it here, then pick a new password."));
  },
  "step-remember"(el) {
    state.step.remember = el.checked;
  },
  "step-method"(el) {
    const m = state.step;
    m.method = el.dataset.id;
    if (m.method === "email") return busy(() => api("POST", "/v1/login/email", { ticket: m.ticket }).then(() => toast("Code sent: look in your email")));
    render();
  },
  "step-code"() {
    return busy(async () => {
      const m = state.step;
      const s = await api("POST", "/v1/login/code", { ticket: m.ticket, method: m.method, code: state.form.code || "", remember: m.remember, device: "Files in a browser" });
      await signedIn(s);
    });
  },
  "step-finish"() {
    return busy(async () => {
      const m = state.step;
      const r = await api("POST", "/v1/mfa/app/finish", { ticket: m.ticket, code: state.form.code || "", remember: m.remember });
      m.codes = r.backup_codes;
      m.session = r.session;
    });
  },
  "step-kept"() {
    return busy(() => signedIn(state.step.session));
  },
  signout() {
    signOut();
  },
  into(el) {
    const id = +el.dataset.id;
    const n = find(id);
    if (state.find) {
      // A folder found by searching: open it in its space.
      go(`#/space/${el.dataset.space || n.space}/${id}`);
      return;
    }
    const p = state.place;
    if (p.kind === "space") go(placeHash({ ...p, path: [...p.path, { id, name: nameOf(n) }] }));
    else go(`#/space/${n.space}/${id}`);
  },
  // A click opens: a folder (into it), a page, or a file in its app.
  open(el) {
    if (suppressClick) return;
    const n = find(+el.dataset.id);
    if (state.ctx) closeCtx(false);
    openNode(n);
  },
  ctx(el) {
    const r = el.getBoundingClientRect();
    if (state.ctx?.node?.id === +el.dataset.id) return closeCtx();
    openCtx(+el.dataset.id, r.left, r.bottom + 4, r.height + 8);
  },
  read(el) {
    closeCtx(false);
    go(`#/read/${el.dataset.id}`);
  },
  // Chat opens here; its Quit comes back.
  "to-chat"() {
    noteBack();
  },
  "overview-new"() {
    const space = state.wiki.space;
    return busy(async () => {
      const n = await makeOverview(space);
      await wiki(space, null);
      openApp(n);
    });
  },
  "edit-page"(el) {
    const n = find(+el.dataset.id);
    if (state.ctx) closeCtx(false);
    // A page on a wide screen: Write right where it's read.
    if (state.view === "wiki" && isPage(n) && webApps() && !matchMedia("(max-width: 760px)").matches) return void editInline(n);
    openApp(n);
  },
  "edit-done"() {
    return endEdit();
  },
  "edit-discard"() {
    pageEd.frame?.contentWindow?.postMessage({ t: "discard" }, location.origin);
  },
  "edit-full"() {
    const n = pageEd.node;
    return endEdit({ reload: false }).then(() => openApp(n));
  },
  details(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    choose(n);
  },
  "details-close"() {
    state.sel = null;
    state.about = null;
    render();
  },
  rename(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    state.form.newName = isPage(n) ? pageName(n) : nameOf(n);
    state.dialog = { kind: "rename", node: n };
    render();
  },
  "rename-save"() {
    return busy(async () => {
      const d = state.dialog;
      const nm = (state.form.newName || "").trim();
      if (!nm) throw new Error("Give it a name.");
      if (isPage(d.node)) await noTwin(d.node.space, d.node.parent, nm, d.node.id);
      await api("POST", `/v1/nodes/${d.node.id}`, { meta: JSON.stringify({ ...d.node.d, name: nm }) });
      state.dialog = null;
      await refresh();
    });
  },
  move(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    const d = { kind: "move", node: n, all: null, kids: null, to: undefined };
    state.dialog = d;
    render();
    moveList(d);
  },
  "move-here"() {
    return busy(async () => {
      const d = state.dialog;
      await api("POST", `/v1/nodes/${d.node.id}`, { move_to: { parent: d.to } });
      state.dialog = null;
      if (state.wiki) state.wiki.tree = {};
      toast("Moved");
      await refresh();
    });
  },
  delete(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    const d = { kind: "delete", node: n, under: null };
    state.dialog = d;
    render();
    // What goes with it: the pages under a page, what's in a folder.
    if (isPage(n) || n.kind === "folder") {
      api("GET", `/v1/everything?space=${n.space}`).then(async (all) => {
        const below = new Set([n.id]);
        let grew = true;
        while (grew) {
          grew = false;
          for (const x of all) if (x.parent != null && below.has(x.parent) && !below.has(x.id)) { below.add(x.id); grew = true; }
        }
        const under = all.filter((x) => x.id !== n.id && below.has(x.id));
        for (const x of under) await name(x);
        if (state.dialog === d) {
          d.under = under;
          render();
        }
      }, () => {});
    }
  },
  "delete-yes"() {
    return busy(async () => {
      const n = state.dialog.node;
      await api("DELETE", `/v1/nodes/${n.id}`);
      state.dialog = null;
      if (state.sel?.id === n.id) state.sel = null;
      toast(`Deleted “${isPage(n) ? pageName(n) : nameOf(n)}”.`, { restore: n });
      // The page shown went: to the page above it.
      if (state.view === "wiki" && state.wiki.node?.id === n.id) {
        state.wiki.tree = {};
        return void (go(`#/wiki/${n.space}${n.parent ? `/${n.parent}` : ""}`));
      }
      if (state.view === "read" && state.reading?.node?.id === n.id) return void (go(placeHashOf({ ...n, d: {} })));
      await refresh();
    });
  },
  versions(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    const d = { kind: "versions", node: n, list: null };
    state.dialog = d;
    render();
    api("GET", `/v1/nodes/${n.id}/versions`).then((l) => {
      if (state.dialog !== d) return;
      d.list = l;
      render();
    }, (e) => {
      d.error = e.message;
      d.list = [];
      render();
    });
  },
  "version-open"(el) {
    const d = state.dialog, n = +el.dataset.id;
    const name = nameOf(d.node);
    d.view = { title: d.list?.[0]?.n === n ? `${name}, now` : `${name}, version ${n}` };
    render();
    api("GET", `/v1/nodes/${d.node.id}/page?version=${n}`).then((p) => { if (d.view) { d.view.html = forFrame(p.html, "", { reading: false }).html; render(); } }, (e) => { d.error = e.message; render(); });
  },
  "version-compare"(el) {
    const d = state.dialog, n = +el.dataset.id, from = +el.dataset.from;
    d.view = { title: `What changed in ${d.list?.[0]?.n === n ? "the latest version" : `version ${n}`}` };
    render();
    api("GET", `/v1/nodes/${d.node.id}/compare?from=${from}&to=${n}`).then((p) => {
      if (!d.view) return;
      d.view.html = forFrame(p.html, "", { reading: false }).html;
      d.view.note = `Since version ${from}: ${p.added} word${p.added === 1 ? "" : "s"} added (underlined), ${p.removed} taken out (struck through).`;
      render();
    }, (e) => { d.error = e.message; render(); });
  },
  "version-blame"(el) {
    const d = state.dialog, n = +el.dataset.id;
    d.view = { title: `Who wrote what${d.list?.[0]?.n === n ? "" : ` (version ${n})`}`, note: "Each paragraph with who saved it as it is, and since which version." };
    render();
    api("GET", `/v1/nodes/${d.node.id}/blame?version=${n}`).then((p) => { if (d.view) { d.view.paragraphs = p.paragraphs; render(); } }, (e) => { d.error = e.message; render(); });
  },
  "version-back"() {
    const d = state.dialog;
    d.view = null;
    d.error = "";
    render();
  },
  "version-get"(el) {
    const d = state.dialog;
    return busy(async () => {
      const bytes = await api("GET", `/v1/nodes/${d.node.id}/content?version=${+el.dataset.id}&file=true`, undefined, { raw: true });
      const nm = nameOf(d.node);
      const dot = nm.lastIndexOf(".");
      save(bytes, d.list[0]?.n === +el.dataset.id ? nm : dot > 0 ? `${nm.slice(0, dot)} (version ${el.dataset.id})${nm.slice(dot)}` : `${nm} (version ${el.dataset.id})`);
    });
  },
  undo() {
    const u = state.undo;
    state.toast = "";
    state.undo = null;
    if (!u?.restore) return render();
    return busy(async () => {
      await api("POST", `/v1/nodes/${u.restore.id}/restore`);
      if (state.wiki) state.wiki.tree = {};
      toast("Put back.");
      await refresh();
    });
  },
  forever(el) {
    const n = state.trash.list.find((x) => x.id === +el.dataset.id);
    state.dialog = { kind: "forever", id: n.id, name: isPage(n) ? pageName(n) : nameOf(n), with: n.with };
    render();
  },
  "empty-bin"() {
    const t = state.trash;
    state.dialog = { kind: "forever", all: true, space: t.space, label: state.spaces.find((x) => x.id === t.space)?.label || "this space" };
    render();
  },
  "forever-yes"() {
    const d = state.dialog;
    return busy(async () => {
      if (d.all) await api("DELETE", `/v1/trash?space=${d.space}`);
      else await api("DELETE", `/v1/nodes/${d.id}/forever`);
      state.dialog = null;
      toast(d.all ? "The bin is empty." : "Deleted for good.");
      await bin(state.trash.space);
    });
  },
  restore(el) {
    return busy(async () => {
      await api("POST", `/v1/nodes/${+el.dataset.id}/restore`);
      if (state.wiki) state.wiki.tree = {};
      toast("Put back.");
      await refresh();
    });
  },
  "copy-link"(el) {
    const n = find(+el.dataset.id);
    if (state.ctx) closeCtx(false);
    const url = linkTo(n);
    const done = () => toast("Link copied: people on this server can open it.");
    if (navigator.clipboard && isSecureContext) navigator.clipboard.writeText(url).then(done, () => prompt("The link:", url));
    else prompt("The link:", url);
  },
  "make-home"(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    return setHome(n.space, n.id);
  },
  unhome(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    return setHome(n.space, null);
  },
  "order-up"(el) {
    return reorder(+el.dataset.id, -1);
  },
  "order-down"(el) {
    return reorder(+el.dataset.id, 1);
  },
  "make-page"(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    return busy(async () => {
      // A page of the space, at its top.
      await api("POST", `/v1/nodes/${n.id}`, { meta: JSON.stringify({ ...n.d, page: true }), move_to: { parent: null } });
      if (state.wiki?.space === n.space) state.wiki.tree = {};
      go(`#/wiki/${n.space}/${n.id}`);
    });
  },
  view(el) {
    try {
      localStorage.setItem(VIEW_KEY, el.dataset.id);
    } catch {}
    render();
  },
  sort(el) {
    const k = el.dataset.id;
    state.sort = { key: k, desc: state.sort.key === k ? !state.sort.desc : k === "changed" };
    render();
  },
  // A file found by searching: straight to the page, the words marked.
  "read-hit"(el) {
    go(`#/read/${el.dataset.id}`);
  },
  edit(el) {
    openInApp(find(+el.dataset.id));
  },
  download(el) {
    const id = +el.dataset.id;
    const picked = find(id);
    if (state.ctx) closeCtx(false);
    return busy(async () => {
      if (state.mode === "public") {
        const n = state.pub.items.find((x) => x.id === id);
        save(await pubContents(n), fileNameOf(n));
      } else save(await contents(picked), fileNameOf(picked));
    });
  },
  share(el) {
    const n = find(+el.dataset.id);
    if (state.ctx) closeCtx(false);
    return busy(() => shareDialog(n));
  },
  "share-mode"(el) {
    return busy(() => setShare(el.dataset.id));
  },
  "tree-toggle"(el) {
    const w = state.wiki;
    const id = +el.dataset.id;
    if (w.open.has(id)) w.open.delete(id);
    else {
      w.open.add(id);
      pagesUnder(w.space, id).then(() => hasPages(w), (e) => toast(e.message));
    }
    render();
  },
  drawer() {
    state.wiki.drawer = !state.wiki.drawer;
    render();
  },
  // A new page: at the top of the space, or under a page.
  "page-new"(el) {
    state.menu = null;
    const w = state.wiki;
    const parent = el.dataset.id ? +el.dataset.id : null;
    const above = parent ? find(parent) : null;
    if (state.ctx) closeCtx(false);
    state.form.fileName = "";
    state.form.newTemplate = "";
    state.dialog = { kind: "file", app: "write", into: { space: w?.space ?? above?.space, parent, where: above ? pageName(above) : state.spaces.find((x) => x.id === w?.space)?.label || "this space" } };
    render();
  },
  "share-new"() {
    state.dialog.confirmNew = true;
    render();
  },
  "share-new-no"() {
    state.dialog.confirmNew = false;
    render();
  },
  "share-remove"(el) {
    const d = state.dialog;
    d.access.grants.splice(+el.dataset.i, 1);
    d.changed = true;
    render();
  },
  "share-save"() {
    return busy(async () => {
      const d = state.dialog;
      const me = state.session.account.id;
      // Keep yourself on your own list.
      if (d.access.own && !d.access.grants.some((g) => g.who?.user === me)) d.access.grants.unshift({ who: { user: me }, role: "manager" });
      await api("POST", `/v1/nodes/${d.node.id}/access`, d.access);
      d.changed = false;
      if (state.sel) choose(state.sel);
      toast("Sharing saved");
    });
  },
  copy() {
    const inp = document.getElementById("publink");
    const done = () => toast("Link copied");
    if (navigator.clipboard && isSecureContext) navigator.clipboard.writeText(inp.value).then(done, () => {});
    else {
      inp.select();
      document.execCommand("copy");
      done();
    }
  },
  menu(el) {
    state.ctx = null;
    state.menu = state.menu === el.dataset.id ? null : el.dataset.id;
    render();
  },
  "new-file"(el) {
    state.menu = null;
    const target = newTarget();
    // Drawing, spreadsheet, plan: straight into the app (Draw asks the size
    // there); it's named when it's first saved, in the folder shown here.
    if (["draw", "calc", "plan"].includes(el.dataset.id) && webApps() && target.space != null) {
      if (target.why) toast(target.why);
      noteBack();
      location.href = `/${el.dataset.id}/?new=1&space=${target.space}${target.parent != null ? `&parent=${target.parent}` : ""}`;
      return;
    }
    state.form.fileName = "";
    state.form.newTemplate = "";
    state.dialog = { kind: "file", app: el.dataset.id, target };
    render();
  },
  "space-new"() {
    state.menu = null;
    state.form.spaceName = "";
    const me = state.session.account.id;
    state.dialog = { kind: "space", id: null, mode: "everyone", list: [{ who: { user: me }, role: "manager" }] };
    render();
  },
  "space-access"() {
    state.menu = null;
    const id = state.wiki.space;
    const label = state.spaces.find((x) => x.id === id)?.label || "this space";
    return busy(async () => {
      const a = await api("GET", `/v1/spaces/${id}/access`);
      const everyone = a.grants.find((g) => g.who === "everyone");
      const list = a.grants.filter((g) => g.who !== "everyone").map((g) => ({ who: g.who, role: g.role }));
      const mode = !everyone ? "chosen" : everyone.role === "viewer" ? "read" : "everyone";
      state.dialog = { kind: "space", id, label, mode, list };
    });
  },
  "space-remove"(el) {
    state.dialog.list.splice(+el.dataset.i, 1);
    render();
  },
  "space-save"() {
    const d = state.dialog;
    return busy(async () => {
      // Everyone: everyone can change it, and its managers stay. Read:
      // everyone can read it, and those chosen do what they're given.
      const grants = d.mode === "everyone"
        ? [{ who: "everyone", role: "editor" }, ...d.list.filter((g) => g.role === "manager")]
        : d.mode === "read" ? [{ who: "everyone", role: "viewer" }, ...d.list] : d.list;
      if (d.id) {
        await api("POST", `/v1/spaces/${d.id}/access`, { grants });
        state.dialog = null;
        toast("Saved who can use it.");
        return;
      }
      const nm = (state.form.spaceName || "").trim();
      if (!nm) throw new Error("Give the space a name.");
      const made = await api("POST", "/v1/spaces", { name: JSON.stringify({ name: nm }), grants });
      state.dialog = null;
      await loadSpaces();
      // Its overview, ready to write.
      await makeOverview(made.id).catch(() => {});
      go(`#/wiki/${made.id}`);
    });
  },
  "space-rename"() {
    state.menu = null;
    const id = state.wiki.space;
    state.form.spaceName = state.spaces.find((x) => x.id === id)?.label || "";
    state.dialog = { kind: "space-rename", id };
    render();
  },
  "space-rename-save"() {
    const d = state.dialog;
    return busy(async () => {
      const nm = (state.form.spaceName || "").trim();
      if (!nm) throw new Error("Give the space a name.");
      await api("POST", `/v1/spaces/${d.id}`, { name: JSON.stringify({ ...spaceDetails(d.id), name: nm }) });
      state.dialog = null;
      await loadSpaces();
    });
  },
  "space-delete"() {
    state.menu = null;
    const id = state.wiki.space;
    state.dialog = { kind: "space-delete", id, label: state.spaces.find((x) => x.id === id)?.label || "this space" };
    render();
  },
  "space-delete-yes"() {
    const d = state.dialog;
    return busy(async () => {
      await api("DELETE", `/v1/spaces/${d.id}`);
      state.dialog = null;
      state.wiki = null;
      await loadSpaces();
      const mine = state.spaces.find((x) => x.kind === "personal");
      go(mine ? `#/space/${mine.id}` : "#/recent");
    });
  },
  hub() {
    state.menu = null;
    state.dialog = { kind: "hub" };
    render();
  },
  "copy-addr"() {
    navigator.clipboard?.writeText(location.origin).then(() => toast("Copied."), () => document.getElementById("hubaddr")?.select());
  },
  "make-file"() {
    return busy(async () => {
      const nm = (state.form.fileName || "").trim();
      if (!nm) throw new Error("Give it a name.");
      const k = NEW_KINDS.find((x) => x.app === state.dialog.app);
      const p = state.place;
      const into = state.dialog.into; // a page: { space, parent page }
      const d = { name: nm, app: k.app, ext: k.ext, ...(into ? { page: true } : {}) };
      // Nothing in it yet: its app starts it blank the first time it's opened.
      const t = state.dialog.target;
      const space = into?.space ?? t?.space ?? p.space;
      const parent = into ? into.parent : t ? t.parent : p.path.at(-1)?.id ?? null;
      if (into) await noTwin(space, parent, nm);
      const made = await api("POST", "/v1/nodes", { space, parent, kind: "file", meta: JSON.stringify(d) });
      // From a template: its first version made here, so it opens (and saves) on the server like any other.
      const from = state.form.newTemplate ? +state.form.newTemplate : null;
      if (from && k.app === "write") {
        const bytes = await api("GET", `/v1/nodes/${from}/as-new?origin=${encodeURIComponent(location.origin)}`, undefined, { raw: true });
        await api("POST", `/v1/nodes/${made.id}/content?base=0`, undefined, { bytes });
      }
      state.dialog = null;
      state.everything = null;
      const n = { id: made.id, kind: "file", space, role: "manager", d };
      if (into) {
        // Its page (shown on coming back), then straight into Write.
        if (state.wiki?.space === into.space) state.wiki.tree = {};
        history.pushState(null, "", placePath(`#/wiki/${into.space}/${made.id}`));
        return openApp(n);
      }
      // Straight into its app; back here, it's in the folder.
      if (state.view === "browse" && p.kind === "space" && p.space === space) await show(p);
      openApp(n);
    });
  },
  "template-toggle"(el) {
    const n = find(+el.dataset.id);
    closeCtx(false);
    return busy(async () => {
      const on = !n.d?.template;
      const meta = { ...n.d };
      if (on) meta.template = true;
      else delete meta.template;
      await api("POST", `/v1/nodes/${n.id}`, { meta: JSON.stringify(meta) });
      n.d = meta;
      toast(on ? `New documents and pages in this space can start from “${isPage(n) ? pageName(n) : nameOf(n)}” (Start from, when making one).` : "No longer offered as a template.");
      await refresh();
    });
  },
  "new-folder"() {
    state.menu = null;
    state.form.folderName = "";
    state.dialog = { kind: "folder" };
    render();
  },
  "make-folder"() {
    return busy(async () => {
      const nm = (state.form.folderName || "").trim();
      if (!nm) throw new Error("Give the folder a name.");
      const p = state.place;
      await api("POST", "/v1/nodes", { space: p.space, parent: p.path.at(-1)?.id ?? null, kind: "folder", meta: JSON.stringify({ name: nm }) });
      state.dialog = null;
      state.everything = null;
      await show(p);
    });
  },
  upload() {
    state.menu = null;
    render();
    const inp = document.createElement("input");
    inp.type = "file";
    inp.multiple = true;
    inp.hidden = true;
    inp.id = "pick-files";
    inp.onchange = () => {
      inp.remove();
      busy(() => upload([...inp.files]));
    };
    document.body.append(inp);
    inp.click();
  },
  jump(el, e) {
    e.preventDefault();
    const f = document.querySelector("iframe.page");
    const target = f?.contentDocument?.getElementById(el.dataset.id);
    if (target) window.scrollTo({ top: f.getBoundingClientRect().top + window.scrollY + target.offsetTop - 16, behavior: "smooth" });
  },
  "pub-read"(el, e) {
    e.preventDefault();
    readPublic(+el.dataset.id);
  },
  close() {
    state.dialog = null;
    render();
  },
  backdrop(el, e) {
    if (e.target === el) acts.close();
  },
};

/// Refuses a page name that a page beside it has already (two pages called
/// the same can't be told apart in the tree).
async function noTwin(space, parent, nm, self = null) {
  const list = await api("GET", `/v1/nodes?space=${space}${parent ? `&parent=${parent}` : ""}`);
  for (const x of list) await name(x);
  const same = list.find((x) => isPage(x) && x.id !== self && pageName(x).trim().toLowerCase() === nm.trim().toLowerCase());
  if (same) throw new Error(`There’s a page called “${pageName(same)}” here already: give this one another name.`);
}

/// Makes a space's overview: a page at its top, shown on the space's own
/// page (its home).
async function makeOverview(space) {
  const d = { name: "Overview", app: "write", ext: "xwrite", page: true, order: -1 };
  const made = await api("POST", "/v1/nodes", { space, parent: null, kind: "file", meta: JSON.stringify(d) });
  await api("POST", `/v1/spaces/${space}`, { name: JSON.stringify({ ...spaceDetails(space), home: made.id }) });
  await loadSpaces();
  if (state.wiki?.space === space) state.wiki.tree = {};
  return { id: made.id, kind: "file", space, role: "manager", d };
}

/// The page shown on a space's own page (null: none, the space's pages).
function setHome(space, id) {
  return busy(async () => {
    const s = state.spaces.find((x) => x.id === space);
    const dd = { ...details(s.name) };
    if (id) dd.home = id;
    else delete dd.home;
    await api("POST", `/v1/spaces/${space}`, { name: JSON.stringify(dd) });
    await loadSpaces();
    toast(id ? "That page is the space’s home now." : "The space shows its pages again.");
    if (state.wiki) state.wiki.tree = {};
    await refresh();
  });
}

/// The pages beside page `n` (with it), in their order.
function siblingsOf(n) {
  return state.wiki?.space === n.space ? state.wiki.tree[n.parent ?? 0] : null;
}

/// Moves page `id` one place up (-1) or down (1) among the pages beside it.
function reorder(id, by) {
  const n = find(id);
  closeCtx(false);
  const list = [...(siblingsOf(n) || [])];
  const at = list.findIndex((x) => x.id === id);
  const to = at + by;
  if (at < 0 || to < 0 || to >= list.length) return toast(by < 0 ? "It’s first already." : "It’s last already.");
  [list[at], list[to]] = [list[to], list[at]];
  return busy(async () => {
    // Each page beside it keeps its place from now on.
    for (const [i, x] of list.entries()) {
      if (x.d?.order === i) continue;
      await api("POST", `/v1/nodes/${x.id}`, { meta: JSON.stringify({ ...x.d, order: i }) });
      x.d = { ...x.d, order: i };
    }
    state.wiki.tree[n.parent ?? 0] = list;
    render();
  });
}

/// Puts files from this computer in the folder.
async function upload(files) {
  const p = state.place;
  for (const f of files) {
    if (f.size > MAX_UPLOAD) throw new Error(`${f.name} is too big to send from a browser (200 MB at most): use a Xenify app.`);
    const ext = f.name.includes(".") ? f.name.split(".").pop().toLowerCase() : "";
    const made = await api("POST", "/v1/nodes", { space: p.space, parent: p.path.at(-1)?.id ?? null, kind: "file", meta: JSON.stringify({ name: f.name, ext, app: EXT_APP[ext] }) });
    const bytes = new Uint8Array(await f.arrayBuffer());
    await api("POST", `/v1/nodes/${made.id}/content`, undefined, { bytes });
  }
  state.everything = null;
  toast(files.length === 1 ? "Uploaded" : `${files.length} files uploaded`);
  await show(p);
}

document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-act]");
  // A click anywhere else closes an open menu.
  if (state.menu && !e.target.closest(".menu") && el?.dataset.act !== "menu") {
    state.menu = null;
    render();
  }
  if (state.ctx && !e.target.closest(".ctxmenu") && el?.dataset.act !== "ctx") closeCtx(false);
  if (!el || el.tagName === "FORM") return;
  const f = acts[el.dataset.act];
  if (f) f(el, e);
});

document.addEventListener("change", (e) => {
  const el = e.target.closest("[data-change]");
  const f = el && changes[el.dataset.change];
  if (f) f(el, e);
});

document.addEventListener("submit", (e) => {
  e.preventDefault();
  const f = acts[e.target.dataset.act];
  if (f) f(e.target, e);
});

let finding = null;
document.addEventListener("input", (e) => {
  const k = e.target.dataset?.bind;
  if (!k) return;
  if (k === "find") {
    clearTimeout(finding);
    state.find = e.target.value;
    finding = setTimeout(() => search(e.target.value), 200);
  } else if (k === "filter") {
    state.filter = e.target.value;
    render();
  } else if (k === "findScope") {
    state.findScope = e.target.value ? +e.target.value : null;
    if (state.find.trim()) search(state.find);
    else render();
  } else if (k === "find-noop") {
  } else state.form[k] = e.target.value;
});

// Right-click: the menu of what's under the pointer.
document.addEventListener("contextmenu", (e) => {
  const el = e.target.closest("[data-ctx]");
  if (!el || state.mode !== "files") return;
  e.preventDefault();
  openCtx(+el.dataset.ctx, e.clientX, e.clientY);
});

// A long press on a touch screen: the same menu (and no open after it).
let pressing = null;
let suppressClick = false;
document.addEventListener("touchstart", (e) => {
  const el = e.target.closest("[data-ctx]");
  if (!el || e.touches.length !== 1) return;
  const t = e.touches[0];
  clearTimeout(pressing);
  pressing = setTimeout(() => {
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 700);
    openCtx(+el.dataset.ctx, t.clientX, t.clientY);
  }, 550);
}, { passive: true });
for (const ev of ["touchend", "touchmove", "touchcancel"]) document.addEventListener(ev, () => clearTimeout(pressing), { passive: true });

/// The folder's keys, like a file explorer: arrows move between things,
/// Enter opens, Backspace goes up a folder, typing a name's first letters
/// jumps to it, and the menu key (or Shift+F10) opens the menu.
let typed = "";
let typedAt = 0;
function folderKeys(e) {
  if (state.dialog || state.menu || e.ctrlKey || e.metaKey || e.altKey) return false;
  const on = document.activeElement;
  // The menu of the thing with the keys.
  if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
    const el = on?.closest?.("[data-ctx]");
    if (!el) return false;
    const r = on.getBoundingClientRect();
    openCtx(+el.dataset.ctx, r.left + 12, r.bottom + 4, r.height + 8);
    return true;
  }
  if (state.ctx) return menuKeys(e);
  if (state.view !== "browse" || state.find || e.shiftKey) return false;
  if (/INPUT|TEXTAREA|SELECT/.test(on?.tagName || "")) return false;
  const opens = [...document.querySelectorAll(".folder .topen")];
  const at = opens.indexOf(on);
  const go = (i) => opens[Math.max(0, Math.min(opens.length - 1, i))]?.focus();
  // Icons: up and down a row at a time.
  let across = 1;
  if (viewOf() === "icons") {
    const tiles = [...document.querySelectorAll(".tiles .tile")];
    across = Math.max(1, tiles.filter((t) => t.offsetTop === tiles[0]?.offsetTop).length);
  }
  const p = state.place;
  switch (e.key) {
    case "ArrowDown": go(at < 0 ? 0 : at + across); return true;
    case "ArrowUp": go(at < 0 ? 0 : at - across); return true;
    case "ArrowRight": if (across === 1) return false; go(at + 1); return true;
    case "ArrowLeft": if (across === 1) return false; go(at - 1); return true;
    case "Home": go(0); return true;
    case "End": go(opens.length - 1); return true;
    case "Backspace":
      if (p.kind !== "space" || !p.path.length) return false;
      go(placeHash({ ...p, path: p.path.slice(0, -1) }));
      return true;
  }
  if (e.key.length !== 1 || e.key === "/" || e.key === " ") return false;
  typed = (Date.now() - typedAt > 800 ? "" : typed) + e.key.toLowerCase();
  typedAt = Date.now();
  const items = shown();
  const hit = items.findIndex((n) => nameOf(n).toLowerCase().startsWith(typed));
  if (hit >= 0) go(hit);
  return true;
}

/// Up and down the open menu; Escape closes it.
function menuKeys(e) {
  const items = [...document.querySelectorAll(".ctxmenu button")];
  const at = items.indexOf(document.activeElement);
  if (e.key === "ArrowDown") items[(at + 1) % items.length]?.focus();
  else if (e.key === "ArrowUp") items[(at - 1 + items.length) % items.length]?.focus();
  else if (e.key === "Home") items[0]?.focus();
  else if (e.key === "End") items.at(-1)?.focus();
  else if (e.key === "Escape" || e.key === "Tab") closeCtx();
  else return false;
  return true;
}

document.addEventListener("keydown", (e) => {
  if (folderKeys(e)) {
    e.preventDefault();
    return;
  }
  if (e.key === "Escape" && state.dialog) acts.close();
  // Enter in a dialog is its main button (or its only one); a form sends itself, and a box that uses Enter keeps it.
  else if (e.key === "Enter" && state.dialog && !e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing && !e.defaultPrevented
    && !e.target.closest?.("form, textarea, select, button, a[href], [contenteditable]:not([contenteditable='false'])")) {
    const btns = [...document.querySelectorAll(".backdrop .dialog > .row.end .btn, .backdrop .dialog > .stack > .row.end .btn")];
    const main = btns.find((b) => b.classList.contains("main")) || (btns.length === 1 && !/^cancel$/i.test(btns[0].textContent.trim()) ? btns[0] : null);
    if (main && !main.disabled) {
      e.preventDefault();
      main.click();
    }
  } else if (e.key === "Escape" && state.menu) {
    state.menu = null;
    render();
  } else if (e.key === "Escape" && state.sel && state.view === "browse") acts["details-close"]();
  if (e.key === "/" && !/INPUT|TEXTAREA/.test(document.activeElement?.tagName || "")) {
    const s = document.querySelector('[data-bind="find"]');
    if (s) {
      e.preventDefault();
      s.focus();
    }
  }
});

window.addEventListener("hashchange", () => {
  if (state.mode !== "files" || !state.session) return;
  cleanAddress();
  route();
});
window.addEventListener("popstate", () => {
  if (state.mode === "files" && state.session) route();
});
// Links to places here change the page without loading it again.
document.addEventListener("click", (e) => {
  const a = e.target.closest?.("a[href]");
  if (!a || state.mode !== "files" || e.defaultPrevented || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey || a.target) return;
  const href = a.getAttribute("href");
  if (!href.startsWith("/files/") && href !== "/files") return;
  e.preventDefault();
  history.pushState(null, "", href);
  route();
});

// Back from an app (the browser kept this page as it was): what's here now.
window.addEventListener("pageshow", (e) => {
  if (!e.persisted || state.mode !== "files" || !state.session) return;
  if (state.wiki) state.wiki.tree = {};
  route();
});

start();
