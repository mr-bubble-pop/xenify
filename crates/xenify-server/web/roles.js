// Admin › Roles: what people may do on this server, and roles for sharing
// documents. Server roles hold permissions per app (the built-in Everyone
// role applies to all signed-in people; others are given to people or
// groups); document roles are picked when sharing a file or space, and for
// Plan say exactly which changes are allowed (the server checks each one).
//
// app.js hands over its helpers with rolesInit(); its render() calls
// rolesPage() and its click wiring runs rolesActs.

let $ = null; // { state, api, esc, shell, errorBox, render, busy, fail, toast }

export function rolesInit(env) {
  $ = env;
  $.state.roles = { list: null, catalogue: null, sel: null, people: [], groups: [], add: "" };
}

export async function loadRoles() {
  const r = $.state.roles;
  const [all, people, groups] = await Promise.all([$.api("GET", "/v1/roles"), $.api("GET", "/v1/admin/people"), $.api("GET", "/v1/admin/groups")]);
  r.list = all.roles;
  r.catalogue = all.catalogue;
  r.people = people;
  r.groups = groups;
  if (!r.list.some((x) => x.id === r.sel)) r.sel = (r.list.find((x) => x.everyone) || r.list[0])?.id ?? null;
}

const BASES = [["viewer", "Can view"], ["commenter", "Can comment"], ["editor", "Can edit"]];

export function rolesPage() {
  const { state, shell, errorBox, esc } = $;
  const r = state.roles;
  if (!r.list) return shell('<div class="head"><h1>Roles</h1></div><p class="dim">Loading…</p>');
  const sel = r.list.find((x) => x.id === r.sel);
  const item = (x) => `<button class="space ${x.id === r.sel ? "on" : ""}" data-act="role-pick" data-id="${x.id}"><span class="ico" aria-hidden="true">${esc((x.name[0] || "?").toUpperCase())}</span><span><b>${esc(x.name)}</b><span>${x.kind === "document" ? "For sharing documents" : x.everyone ? "Everyone signed in" : x.admin ? `${x.members?.length || 0} ${x.members?.length === 1 ? "person" : "people"}, everything` : `${x.members?.length || 0} ${x.members?.length === 1 ? "person or group" : "people and groups"}`}</span></span></button>`;
  const server = r.list.filter((x) => x.kind === "server");
  const docs = r.list.filter((x) => x.kind === "document");
  const list = `<section class="list" aria-label="Roles"><div class="head"><h1>Roles</h1></div>${errorBox()}
    <h3 class="dim">On this server</h3>${server.map(item).join("")}<button class="btn small" data-act="role-new" data-id="server">New server role</button>
    <h3 class="dim">For sharing documents</h3>${docs.map(item).join("") || '<p class="dim">None yet: sharing offers Can view, Can comment, Can edit and Can manage.</p>'}<button class="btn small" data-act="role-new" data-id="document">New document role</button></section>`;
  return shell(`<div class="split">${list}<section class="detail" aria-label="Role">${sel ? detail(sel) : ""}</section></div>`);
}

function detail(x) {
  const { esc } = $;
  const r = $.state.roles;
  // What everyone has already can't be taken away by another role.
  const all = x.kind === "server" && !x.everyone ? r.list.find((y) => y.everyone)?.perms || [] : [];
  const tick = (p) => all.includes(p.id)
    ? `<label class="check"><input type="checkbox" checked disabled>${esc(p.name)}<span class="dim"> · everyone has this</span></label>`
    : `<label class="check"><input type="checkbox" data-act="role-perm" data-id="${esc(p.id)}" ${x.perms.includes(p.id) ? "checked" : ""}>${esc(p.name)}</label>`;
  const head = `<div class="head"><h2>${esc(x.name)}</h2>${x.everyone || x.admin ? "" : '<button class="btn small" data-act="role-rename">Rename</button><button class="btn small danger" data-act="role-delete">Remove</button>'}</div>`;
  if (x.kind === "document") {
    const base = `<label class="field"><span>Works like</span><select class="inp" data-act="role-base">${BASES.map(([k, n]) => `<option value="${k}" ${x.base === k ? "selected" : ""}>${n}</option>`).join("")}</select></label>
      <p class="dim">In Write, Draw and Calc this role is the same as “${esc(BASES.find((b) => b[0] === x.base)?.[1] || "")}”. In Plan it allows exactly the changes ticked below.</p>`;
    return `${head}<p class="dim">People pick this role when they share a plan, a document or a space.</p>${base}
      <section class="card role-card"><h3>In Plan</h3>${r.catalogue.plan.map(tick).join("")}</section>`;
  }
  const apps = [...new Set(r.catalogue.server.map((p) => p.app))];
  const perms = apps.map((a) => `<section class="card role-card"><h3>${esc(a)}</h3>${r.catalogue.server.filter((p) => p.app === a).map(tick).join("")}</section>`).join("");
  const nameOf = (m) => (m.user ? (r.people.find((p) => p.id === m.user)?.display_name || r.people.find((p) => p.id === m.user)?.username || "?") : `${r.groups.find((g) => g.id === m.group)?.name || "?"} (group)`);
  // Admin: everything, for people (not groups); someone always has it.
  if (x.admin) {
    const mine = !!$.state.session?.account?.admin;
    const only = (x.members || []).length === 1;
    return `${head}<p class="dim">Admins may do everything here: every permission, every part of the admin pages, and making other admins. Only admins give it out.</p>
      <section class="card role-card"><h3>Who has it</h3>
      ${(x.members || []).map((m, i) => `<div class="role-member"><span class="who"><b>${esc(nameOf(m))}</b></span>${mine && !only ? `<button class="more" aria-label="Take it from ${esc(nameOf(m))}" data-act="role-unmember" data-id="${i}">×</button>` : ""}</div>`).join("")}
      ${only ? '<p class="dim">Someone must always be an admin: give it to someone else first to take it from them.</p>' : ""}
      ${mine ? `<div class="row"><select class="inp grow" aria-label="Give it to a person" data-bind="roles.add"><option value="">Give it to a person…</option>
        ${r.people.filter((p) => !p.disabled && !p.admin).map((p) => `<option value="u${p.id}">${esc(p.display_name || p.username)}</option>`).join("")}</select>
      <button class="btn" data-act="role-member">Add</button></div>` : ""}</section>`;
  }
  const members = x.everyone
    ? '<p class="dim">Everyone signed in has this role. Admins may always do everything.</p>'
    : `<section class="card role-card"><h3>Who has it</h3>
      ${(x.members || []).map((m, i) => `<div class="role-member"><span class="who"><b>${esc(nameOf(m))}</b></span><button class="more" aria-label="Take it from ${esc(nameOf(m))}" data-act="role-unmember" data-id="${i}">×</button></div>`).join("") || '<p class="dim">Nobody yet.</p>'}
      <div class="row"><select class="inp grow" aria-label="Give it to a person or group" data-bind="roles.add"><option value="">Give it to a person or group…</option>
        ${r.groups.map((g) => `<option value="g${g.id}">${esc(g.name)} (group)</option>`).join("")}
        ${r.people.filter((p) => !p.disabled).map((p) => `<option value="u${p.id}">${esc(p.display_name || p.username)}</option>`).join("")}</select>
      <button class="btn" data-act="role-member">Add</button></div></section>`;
  // Advanced: parts of running the server, for someone who isn't an admin.
  const admin = !!$.state.session?.account?.admin;
  const powerTick = (p) => `<label class="check"><input type="checkbox" data-act="role-perm" data-id="${esc(p.id)}" ${x.perms.includes(p.id) ? "checked" : ""} ${admin ? "" : "disabled"}>${esc(p.name)}</label>`;
  const held = (r.catalogue.admin || []).filter((p) => x.perms.includes(p.id)).length;
  const advanced = x.everyone || !r.catalogue.admin?.length ? "" : `<details class="card role-card advanced" ${held ? "open" : ""}><summary><h3>Advanced: running the server${held ? ` <span class="dim">· ${held}</span>` : ""}</h3></summary>
      <div class="advanced-body"><p class="dim">Lets the people with this role look after parts of the admin pages without being an admin. Admins may always do all of it. ${admin ? "Only admins can give these out." : "Only admins can change these, or who has a role that holds them."}</p>
      ${r.catalogue.admin.map(powerTick).join("")}</div></details>`;
  return `${head}<p class="dim">${x.everyone ? "What everyone may do. Turn something off here and give it back to some people with a role of its own." : "Adds to what Everyone may do, for the people and groups below."}</p>${members}${perms}${advanced}`;
}

async function change(body) {
  const { state, api, render, fail } = $;
  const r = state.roles;
  try {
    await api("POST", `/v1/admin/roles/${r.sel}`, body);
    await loadRoles();
    state.error = "";
    render();
  } catch (e) {
    fail(e.message);
  }
}

/// The name dialog (app.js draws kind "role-name").
function named(title, placeholder, name, yes, then) {
  $.state.dialog = { kind: "role-name", title, placeholder, name, yes, then: async () => {
    const n = ($.state.dialog.name || "").trim();
    if (!n) throw new Error("Give the role a name.");
    await then(n);
    $.state.dialog = null;
    await loadRoles();
  } };
  $.render();
}

const current = () => $.state.roles.list.find((x) => x.id === $.state.roles.sel);

export const rolesActs = {
  "role-pick"(el) {
    $.state.roles.sel = Number(el.dataset.id);
    $.render();
  },
  async "role-perm"(el) {
    const x = current();
    const perms = el.checked ? [...x.perms, el.dataset.id] : x.perms.filter((p) => p !== el.dataset.id);
    await change({ perms });
  },
  async "role-base"(el) {
    await change({ base: el.value });
  },
  async "role-member"() {
    const r = $.state.roles;
    const v = r.add;
    if (!v) return;
    const m = v[0] === "g" ? { group: Number(v.slice(1)) } : { user: Number(v.slice(1)) };
    r.add = "";
    await change({ members: [...(current().members || []), m] });
  },
  async "role-unmember"(el) {
    await change({ members: (current().members || []).filter((_, i) => i !== Number(el.dataset.id)) });
  },
  "role-new"(el) {
    const kind = el.dataset.id;
    named(kind === "document" ? "New document role" : "New server role", kind === "document" ? "Reporter" : "Team leads", "", "Make role", async (name) => {
      const made = await $.api("POST", "/v1/admin/roles", { name, kind, perms: kind === "document" ? ["plan.comment", "plan.create"] : [], base: kind === "document" ? "commenter" : undefined });
      $.state.roles.sel = made.id;
    });
  },
  "role-rename"() {
    named("Rename the role", "", current().name, "Save", (name) => $.api("POST", `/v1/admin/roles/${current().id}`, { name }));
  },
  "role-delete"() {
    const x = current();
    $.state.dialog = { kind: "confirm", title: `Remove the role ${x.name}?`, text: x.kind === "document" ? "Files and spaces shared with it keep what it was based on." : "Its people keep what Everyone may do.", yes: "Remove role", then: async () => {
      await $.api("DELETE", `/v1/admin/roles/${x.id}`);
      $.state.dialog = null;
      await loadRoles();
    } };
    $.render();
  },
};
