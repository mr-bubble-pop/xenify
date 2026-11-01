// Admin › Email: where people's email in Chat comes from.
//
// - The company's mailboxes, connected once here: Microsoft 365 (an app
//   registration the company's admin consents to), Google Workspace (a
//   service account with domain-wide delegation) or the company's own mail
//   server (one master login). Everyone at the company's domain then has
//   their mailbox in Chat, with nothing to set up themselves.
// - Whether people may also connect their own Microsoft or Google mail (on
//   the provider's sign-in page) or add an account with a password.
//
// app.js hands over its helpers with emailInit(); its render() calls
// emailPage() and its click wiring runs emailActs.

let $ = null; // { state, api, esc, shell, errorBox, render, fail, toast }

export function emailInit(env) {
  $ = env;
  $.state.em = { page: null, form: null, groups: [], test: "", tested: null };
}

const BLANK_SERVER = { host: "", port: 993, security: "tls" };

export async function loadEmail() {
  const em = $.state.em;
  const [page, groups] = await Promise.all([$.api("GET", "/v1/admin/usermail"), $.api("GET", "/v1/admin/groups").catch(() => [])]);
  em.page = page;
  em.groups = groups;
  if (!em.form) {
    em.form = { ...page, imap: page.imap || { ...BLANK_SERVER }, smtp: page.smtp || { host: "", port: 587, security: "starttls" } };
  }
  em.test ||= $.state.session?.account?.email || "";
}

function input(label, bind, value, opts = {}) {
  const { esc } = $;
  const tag = opts.area
    ? `<textarea class="inp" rows="4" data-bind="em.form.${bind}" placeholder="${esc(opts.placeholder || "")}" spellcheck="false">${esc(value ?? "")}</textarea>`
    : `<input class="inp" ${opts.type ? `type="${opts.type}"` : ""} data-bind="em.form.${bind}" value="${esc(value ?? "")}" placeholder="${esc(opts.placeholder || "")}" ${opts.auto ? `autocomplete="${opts.auto}"` : 'autocomplete="off"'} spellcheck="false">`;
  return `<label class="field ${opts.grow ? "grow" : ""}"><span>${esc(label)}</span>${tag}${opts.hint ? `<small class="dim">${opts.hint}</small>` : ""}</label>`;
}

const kept = (has) => (has ? "Kept (type to change)" : "");

function serverRow(which, s) {
  const { esc } = $;
  const secs = [["tls", "TLS"], ["starttls", "STARTTLS"], ["none", "None (this computer only)"]];
  return `<div class="row wrap">${input(which === "imap" ? "Incoming mail server (IMAP)" : "Outgoing mail server (SMTP)", `${which}.host`, s.host, { grow: true, placeholder: which === "imap" ? "mail.example.com" : "mail.example.com" })}
    ${input("Port", `${which}.port`, s.port, { type: "number" })}
    <label class="field"><span>Connection</span><select class="inp" data-bind="em.form.${which}.security">${secs.map(([v, n]) => `<option value="${v}" ${s.security === v ? "selected" : ""}>${esc(n)}</option>`).join("")}</select></label></div>`;
}

function companyCard() {
  const { state, esc } = $;
  const em = state.em;
  const f = em.form;
  const p = em.page;
  const kinds = [
    ["", "Not connected", "People add their own email in Chat (below)."],
    ["microsoft", "Microsoft 365", "Outlook and Exchange Online."],
    ["google", "Google Workspace", "Gmail for your company's domain."],
    ["imap", "Your own mail server", "Any IMAP and SMTP server that has a master login (Dovecot, for one)."],
  ];
  const radios = kinds.map(([k, n, why]) => `<label class="choice ${f.org === k ? "on" : ""}"><input type="radio" name="em-org" data-act="em-org" value="${k}" ${f.org === k ? "checked" : ""}><span><b>${esc(n)}</b><span class="why">${esc(why)}</span></span></label>`).join("");
  let body = "";
  if (f.org === "microsoft") {
    body = `<ol class="dim steps-list">
        <li>In the <b>Microsoft Entra admin center</b> (entra.microsoft.com) › App registrations › <b>New registration</b>. Name it "Xenify". Who can use it: <b>this organisation only</b>.</li>
        <li>In the new app › <b>API permissions</b> › Add › Microsoft Graph › <b>Application permissions</b>: <code>Mail.ReadWrite</code> and <code>Mail.Send</code>. Then <b>Grant admin consent</b>.</li>
        <li><b>Certificates &amp; secrets</b> › New client secret. Copy its <b>value</b>.</li>
        <li>Copy the <b>Directory (tenant) ID</b> and <b>Application (client) ID</b> from the app's Overview, and paste all three here.</li>
        <li>To limit it to some mailboxes, Microsoft's admins can add an application access policy in Exchange; otherwise the server can reach every mailbox in the company (it only opens those of people on this server).</li>
      </ol>
      <div class="row wrap">${input("Directory (tenant) ID", "ms_tenant", f.ms_tenant, { grow: true })}${input("Application (client) ID", "ms_client_id", f.ms_client_id, { grow: true })}</div>
      ${input("Client secret", "ms_secret", f.ms_secret, { type: "password", placeholder: kept(p.has_ms_secret), auto: "new-password" })}`;
  } else if (f.org === "google") {
    body = `<ol class="dim steps-list">
        <li>In the <b>Google Cloud console</b> (console.cloud.google.com): make a project, turn on the <b>Gmail API</b>, then IAM › Service accounts › <b>Create service account</b> › Keys › Add key › <b>JSON</b>. A key file downloads.</li>
        <li>In the <b>Google Admin console</b> (admin.google.com) › Security › Access and data control › API controls › <b>Domain-wide delegation</b> › Add new: the service account's client ID, with the scope <code>https://mail.google.com/</code>.</li>
        <li>Paste the whole key file here.</li>
      </ol>
      ${input("Service account key (the JSON file)", "google_key", f.google_key, { area: true, placeholder: p.has_google_key ? "Kept (paste a new key to change it)" : '{ "type": "service_account", … }' })}`;
  } else if (f.org === "imap") {
    body = `<p class="dim">The server signs in for each person with one master login, written as you set below (Dovecot's is <code>person@example.com*master</code>), for reading and for sending. Ask whoever runs the mail server for one.</p>
      ${serverRow("imap", f.imap)}${serverRow("smtp", f.smtp)}
      <div class="row wrap">${input("Master user name", "master_user", f.master_user, { grow: true })}${input("Master password", "master_password", f.master_password, { type: "password", grow: true, placeholder: kept(p.has_master_password), auto: "new-password" })}</div>
      <details class="dim"><summary>How the sign-in is written</summary>${input("Sign-in name", "login_as", f.login_as, { placeholder: "{email}*{master}", hint: "{email} is the person's address, {user} the part before the @, {master} the master user name." })}</details>`;
  }
  const scope = f.org
    ? `<div class="row wrap">${input("Your company's email domain", "domains", f.domains, { grow: true, placeholder: "example.com", hint: "Several? Separate them with commas. People on this server whose email address is at it get their mailbox in Chat." })}
       <label class="field"><span>For</span><select class="inp" data-bind="em.form.group">${[[0, "Everyone at the domain"], ...state.em.groups.map((g) => [g.id, `Only the group ${g.name}`])].map(([v, n]) => `<option value="${v}" ${Number(f.group) === Number(v) ? "selected" : ""}>${esc(n)}</option>`).join("")}</select></label></div>`
    : "";
  const t = em.tested;
  const test = f.org
    ? `<div class="row wrap">${input("Try it with", "test", em.test, { placeholder: "someone@example.com" }).replace('data-bind="em.form.test"', 'data-bind="em.test"')}<button class="btn" data-act="em-test" ${state.busy ? "disabled" : ""}>Test</button><span class="grow"></span></div>
       ${!t ? "" : t.ok ? `<p class="bk-ok">Works: the server can open ${esc(t.email)}'s mailbox. Save, and everyone covered has their email in Chat.</p>` : `<p class="bk-err">${esc(t.error)}</p>`}
       <p class="dim">Test saves this page first.</p>`
    : "";
  return `<section class="card"><h2>Your company's mailboxes</h2>
    <p class="dim">Connected once here, everyone's email is in Chat — in the browser and the desktop app — without anyone typing their email password into Xenify.</p>
    <div class="radios">${radios}</div>
    ${body}${scope}${test}</section>`;
}

function ownCard() {
  const { state, esc } = $;
  const f = state.em.form;
  const p = state.em.page;
  const redirect = `<p class="dim">Sign-in address to add to the app (redirect URI): <code>${esc(p.redirect)}</code></p>`;
  return `<section class="card"><h2>People's own email</h2>
    <p class="dim">For mail that isn't the company's: someone's own Microsoft or Google account, or another provider.</p>
    <label class="check"><input type="checkbox" data-act="em-check" data-k="personal_ms" ${f.personal_ms ? "checked" : ""}> People can connect their own Microsoft mail (Outlook.com, Microsoft 365)</label>
    ${f.personal_ms ? `<div class="indent"><p class="dim">An app registration in the Entra admin center with <b>delegated</b> permissions <code>IMAP.AccessAsUser.All</code>, <code>SMTP.Send</code>, <code>offline_access</code>, <code>openid</code> and <code>email</code>, and a <b>Web</b> redirect URI below. For accounts outside your company, its "Supported account types" must include personal Microsoft accounts.${f.org === "microsoft" ? " Leave these empty to use the company's app (for your company's people only)." : ""}</p>
      ${redirect}
      <div class="row wrap">${input("Application (client) ID", "pm_client_id", f.pm_client_id, { grow: true })}${input("Client secret", "pm_secret", f.pm_secret, { type: "password", grow: true, placeholder: kept(p.has_pm_secret), auto: "new-password" })}</div></div>` : ""}
    <label class="check"><input type="checkbox" data-act="em-check" data-k="personal_google" ${f.personal_google ? "checked" : ""}> People can connect their own Google mail (Gmail)</label>
    ${f.personal_google ? `<div class="indent"><p class="dim">An OAuth client in the Google Cloud console (APIs &amp; Services › Credentials › Create credentials › OAuth client ID › <b>Web application</b>) with the redirect URI below; its consent screen needs the scope <code>https://mail.google.com/</code>. While Google hasn't reviewed the app, add the people who'll use it as test users.</p>
      ${redirect}
      <div class="row wrap">${input("Client ID", "pg_client_id", f.pg_client_id, { grow: true })}${input("Client secret", "pg_secret", f.pg_secret, { type: "password", grow: true, placeholder: kept(p.has_pg_secret), auto: "new-password" })}</div></div>` : ""}
    <label class="check"><input type="checkbox" data-act="em-check" data-k="passwords" ${!f.no_passwords ? "checked" : ""}> People can add an email account with its password</label>
    <p class="dim">For providers with nothing better (an app password from Gmail or iCloud, a hosting company's mail). The password is kept encrypted on this server.</p>
    <label class="check"><input type="checkbox" data-act="em-check" data-k="private_hosts" ${f.private_hosts ? "checked" : ""}> These accounts may use mail servers on this server's own network</label>
    <p class="dim">Off, people can only add mail servers on the internet, so nobody can use this server to reach what's behind it. Turn it on for a mail server in your office.</p>
  </section>`;
}

export function emailPage() {
  const { state, shell, errorBox } = $;
  const em = state.em;
  if (!em.page || !em.form) return shell('<div class="head"><h1>Email</h1></div><p class="dim">Loading…</p>');
  return shell(`<div class="head"><h1>Email</h1></div>
    <p class="dim">People read and send their email in Chat, next to their messages. The server keeps it, so it's the same in the browser and on every computer. (The server's own emails — invitations and sign-in codes — are under Network › Outgoing mail.)</p>${errorBox()}
    ${companyCard()}
    ${ownCard()}
    <div class="row"><button class="btn main" data-act="em-save" ${state.busy ? "disabled" : ""}>Save</button><span class="grow"></span></div>`);
}

function out() {
  const f = { ...$.state.em.form };
  f.group = Number(f.group) || 0;
  for (const s of ["imap", "smtp"]) if (f[s]) f[s] = { ...f[s], port: Number(f[s].port) || 0 };
  for (const k of Object.keys(f)) if (k.startsWith("has_") || k === "redirect") delete f[k];
  return f;
}

export const emailActs = {
  "em-org"(el) {
    $.state.em.form.org = el.value;
    $.state.em.tested = null;
    $.render();
  },
  "em-check"(el) {
    const f = $.state.em.form;
    if (el.dataset.k === "passwords") f.no_passwords = !el.checked;
    else f[el.dataset.k] = el.checked;
    $.render();
  },
  async "em-save"() {
    const { state, api, render, fail, toast } = $;
    try {
      await api("POST", "/v1/admin/usermail", out());
      state.em.form = null;
      await loadEmail();
      state.error = "";
      render();
      toast("Saved");
    } catch (e) {
      fail(e.message);
    }
  },
  async "em-test"() {
    const { state, api, render } = $;
    const email = (state.em.test || "").trim();
    state.busy = true;
    state.em.tested = null;
    render();
    try {
      await api("POST", "/v1/admin/usermail", out());
      state.em.form = null;
      await loadEmail();
      await api("POST", "/v1/admin/usermail/test", { email });
      state.em.tested = { ok: true, email };
    } catch (e) {
      state.em.tested = { ok: false, error: e.message };
    }
    state.busy = false;
    render();
  },
};
