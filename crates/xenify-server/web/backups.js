// Admin › Backups: where the daily backup goes (a folder, S3-compatible
// storage or an SFTP server), its passphrase, when it runs and how long
// backups are kept; how long bins keep deleted things; how the last backups
// went, Back up now, and how to restore.
//
// app.js hands over its helpers with backupsInit(); its render() calls
// backupsPage() and its click wiring runs backupsActs.

let $ = null; // { state, api, esc, shell, errorBox, render, fail, toast }

export function backupsInit(env) {
  $ = env;
  $.state.bk = { page: null, form: null, check: null, newPass: "", poll: null };
}

export async function loadBackups() {
  const bk = $.state.bk;
  bk.page = await $.api("GET", "/v1/admin/backups");
  if (!bk.form) bk.form = { ...bk.page.settings, passphrase: "", s3_secret: "", sftp_password: "" };
  follow();
}

/// While a backup runs, the page looks again every few seconds.
function follow() {
  const bk = $.state.bk;
  if (bk.page?.running && !bk.poll) {
    bk.poll = setInterval(async () => {
      if ($.state.view !== "backups") return void stopFollowing();
      try {
        bk.page = await $.api("GET", "/v1/admin/backups");
      } catch {}
      if (!bk.page.running) stopFollowing();
      $.render();
    }, 3000);
  }
}
function stopFollowing() {
  clearInterval($.state.bk.poll);
  $.state.bk.poll = null;
}

const DAYS = [[7, "7 days"], [14, "14 days"], [30, "30 days"], [60, "60 days"], [90, "90 days"], [365, "A year"], [0, "Until someone empties it"]];

function bytes(n) {
  if (!n) return "0 MB";
  if (n < 1e9) return `${Math.max(0.1, n / 1e6).toFixed(n < 1e7 ? 1 : 0)} MB`;
  return `${(n / 1e9).toFixed(n < 1e10 ? 1 : 0)} GB`;
}

function when(iso) {
  if (!iso) return "";
  const d = new Date(/^\d+$/.test(iso) ? Number(iso) * 1000 : iso);
  return d.toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

/// How long a deleted file can still come back from backups.
function reach(f) {
  if (f.keep_monthly > 0) return f.keep_monthly === 1 ? "about a month" : `about ${f.keep_monthly} months`;
  if (f.keep_weekly > 0) return f.keep_weekly === 1 ? "about a week" : `about ${f.keep_weekly} weeks`;
  return f.keep_daily === 1 ? "a day" : `${f.keep_daily} days`;
}

function input(label, bind, value, opts = {}) {
  const { esc } = $;
  return `<label class="field ${opts.grow ? "grow" : ""}"><span>${esc(label)}</span><input class="inp" ${opts.type ? `type="${opts.type}"` : ""} data-bind="bk.form.${bind}" value="${esc(value ?? "")}" placeholder="${esc(opts.placeholder || "")}" ${opts.auto ? `autocomplete="${opts.auto}"` : 'autocomplete="off"'} ${opts.locked ? "disabled" : ""} spellcheck="false"></label>`;
}

/// Kept secrets only go to the place they were given for.
function kept(set, same) {
  if (!set) return "";
  return same ? "Kept (type to change)" : "Type the password again for the new address";
}

function placeCard() {
  const { state, esc } = $;
  const bk = state.bk;
  const f = bk.form;
  const p = bk.page;
  const s = p.settings;
  // People who may only manage backups don't move them.
  const locked = !p.can_change_place;
  const kinds = [["folder", "A folder on this computer"], ["s3", "S3-compatible storage"], ["sftp", "An SFTP server"]];
  const radios = kinds.map(([k, n]) => `<label class="check"><input type="radio" name="bk-kind" data-act="bk-kind" value="${k}" ${f.kind === k ? "checked" : ""} ${locked ? "disabled" : ""}> ${n}</label>`).join("");
  const sameS3 = f.kind === s.kind && f.s3_endpoint.trim() === s.s3_endpoint.trim() && f.s3_bucket.trim() === s.s3_bucket.trim();
  const sameSftp = f.kind === s.kind && f.sftp_address.trim() === s.sftp_address.trim();
  let fields = "";
  if (f.kind === "folder") {
    fields = `${input("Folder", "folder", f.folder, { grow: true, placeholder: "/mnt/backup/xenify", locked })}
      <p class="dim">A second disk, a USB disk, or a network share this computer has mounted. A folder on the same disk as the server doesn’t help if that disk fails.</p>`;
  } else if (f.kind === "s3") {
    fields = `<div class="row wrap">${input("Storage address", "s3_endpoint", f.s3_endpoint, { grow: true, placeholder: "https://s3.eu-central-003.backblazeb2.com", locked })}
      ${input("Bucket", "s3_bucket", f.s3_bucket, { placeholder: "my-bucket or my-bucket/xenify", locked })}</div>
      <div class="row wrap">${input("Access key", "s3_key", f.s3_key, { grow: true })}
      ${input("Secret key", "s3_secret", f.s3_secret, { type: "password", grow: true, placeholder: kept(s.s3_secret_set, sameS3), auto: "new-password" })}</div>
      <p class="dim">Amazon S3, Backblaze B2, Cloudflare R2, Wasabi, Hetzner, Scaleway, or MinIO on your own NAS. Give the key access to this bucket only.</p>`;
  } else {
    const key = f.sftp_auth !== "password";
    fields = `<div class="row wrap">${input("Address", "sftp_address", f.sftp_address, { grow: true, placeholder: "backup@nas.local or u123@u123.your-storagebox.de:23", locked })}
      ${input("Folder there", "sftp_folder", f.sftp_folder, { placeholder: "xenify-backups", locked })}</div>
      <div class="row wrap"><label class="check"><input type="radio" name="bk-auth" data-act="bk-auth" value="key" ${key ? "checked" : ""}> Sign in with this server’s key</label>
      <label class="check"><input type="radio" name="bk-auth" data-act="bk-auth" value="password" ${key ? "" : "checked"}> With a password</label></div>
      ${key
        ? `<p class="dim">Add this line to <code>~/.ssh/authorized_keys</code> of that user on the SFTP server:</p>
           <div class="row"><code class="grow bk-key">${esc(p.public_key)}</code><button class="btn small" data-act="bk-copy-key">Copy</button></div>`
        : input("Password", "sftp_password", f.sftp_password, { type: "password", placeholder: kept(s.sftp_password_set, sameSftp), auto: "new-password" })}
      ${p.settings.sftp_host_key && f.sftp_address === p.settings.sftp_address ? `<p class="dim">Trusted server key: <code>${esc(p.settings.sftp_host_key)}</code></p>` : '<p class="dim">Check trusts the SFTP server’s key the first time; after that a different key stops backups.</p>'}`;
  }
  const c = bk.check;
  const checked = !c ? "" : c.ok
    ? `<p class="bk-ok">${c.there ? (c.opens ? "Works. Backups are there already, and the passphrase opens them." : "Works, but the backups there need a different passphrase.") : "Works. Nothing is there yet: the first backup starts it."}${c.host_key ? ` Server key: <code>${esc(c.host_key)}</code>` : ""}</p>`
    : `<p class="bk-err">${esc(c.error)}</p>`;
  return `<section class="card"><h2>Where backups go</h2>
    ${locked ? '<p class="dim">Only an admin can change where backups go.</p>' : ""}
    <div class="row wrap">${radios}</div>
    ${fields}
    <div class="row"><button class="btn" data-act="bk-check" ${state.busy ? "disabled" : ""}>${c?.key_changed ? "Check and accept the new key" : "Check"}</button><span class="grow"></span></div>
    ${checked}</section>`;
}

function passCard() {
  const { state, esc } = $;
  const bk = state.bk;
  const f = bk.form;
  const p = bk.page;
  const atRest = p.secure && p.data && p.secure !== p.data;
  const has = (p.status?.snapshots || []).length > 0;
  let body;
  if (!f.encrypt) {
    body = `<p class="${atRest ? "bk-err" : "dim"}">${atRest
      ? "This server encrypts its files at rest, but backups would be readable by anyone who can open them. Turn encryption on."
      : "Anyone who can open the backup place can read everything in it: files, chats and the database."}</p>`;
  } else if (bk.newPass) {
    body = `<p>The new passphrase:</p><div class="row"><code class="grow bk-key big">${esc(bk.newPass)}</code><button class="btn small" data-act="bk-copy-pass">Copy</button></div>
      <p class="bk-err">Write it down and keep it somewhere other than this server. Without it the backups can’t be read, and nobody can get it back for you.</p>`;
  } else if (p.settings.passphrase_set && has) {
    body = `<p class="dim">The passphrase is set. It can’t be changed for backups that exist; to start over with a new one, pick a new place.</p>`;
  } else {
    body = `${p.settings.passphrase_set ? '<p class="dim">A passphrase is set.</p>' : ""}
      <div class="row wrap">${input(p.settings.passphrase_set ? "New passphrase" : "Passphrase", "passphrase", f.passphrase, { type: "password", grow: true, placeholder: "At least 8 characters", auto: "new-password" })}
      <button class="btn" data-act="bk-make-pass">Make one</button></div>
      <p class="dim">Keep it somewhere other than this server. Without it the backups can’t be read, and nobody can get it back for you.</p>`;
  }
  return `<section class="card"><h2>Encryption</h2>
    <label class="check"><input type="checkbox" data-act="bk-encrypt" ${f.encrypt ? "checked" : ""} ${(has && p.settings.passphrase_set) || (f.encrypt && p.settings.encrypt && !p.can_change_place) ? "disabled" : ""}> Encrypt backups with a passphrase</label>
    ${f.encrypt && p.settings.encrypt && !p.can_change_place ? '<p class="dim">Only an admin can turn encryption off.</p>' : ""}
    ${body}</section>`;
}

function whenCard() {
  const { state, esc } = $;
  const f = state.bk.form;
  const hours = Array.from({ length: 24 }, (_, h) => `<option value="${h}" ${f.hour === h ? "selected" : ""}>${String(h).padStart(2, "0")}:00</option>`).join("");
  const num = (label, bind, v) => `<label class="field"><span>${esc(label)}</span><input class="inp small" type="number" min="0" max="999" data-act="bk-num" data-k="${bind}" value="${esc(String(v))}"></label>`;
  return `<section class="card"><h2>When, and how long they’re kept</h2>
    <div class="row wrap"><label class="field"><span>Every day at</span><select class="inp" data-act="bk-hour">${hours}</select></label>
      <label class="field"><span>Upload speed limit</span><span class="row"><input class="inp small" type="number" min="0" step="1" data-act="bk-num" data-k="limit_mbit" value="${esc(String(f.limit_mbit || 0))}"> Mbit/s</span></label></div>
    <p class="dim">Upload speed 0 means no limit. A limit keeps a large first backup from filling a home connection.</p>
    <div class="row wrap">${num("Daily backups", "keep_daily", f.keep_daily)}${num("Weekly", "keep_weekly", f.keep_weekly)}${num("Monthly", "keep_monthly", f.keep_monthly)}</div>
    <p class="dim">Deleted files can be restored from backups for up to ${reach(f)} after they leave the bin. Each backup is complete, but only what changed is stored again.</p></section>`;
}

function binCard() {
  const { state } = $;
  const d = state.bk.page.bin_days;
  const opts = DAYS.map(([v, n]) => `<option value="${v}" ${v === d ? "selected" : ""}>${n}</option>`).join("");
  return `<section class="card"><h2>Deleted files</h2>
    <div class="row wrap"><label class="field"><span>Bins keep deleted things for</span><select class="inp" data-act="bk-bin">${opts}</select></label></div>
    <p class="dim">After that they’re deleted for good, and their space on this server is freed a day later. Anyone who may change a space can put things back from its bin until then.</p></section>`;
}

function statusCard() {
  const { state, esc } = $;
  const p = state.bk.page;
  const st = p.status || {};
  const snaps = st.snapshots || [];
  let line;
  if (p.running) line = `<p><span class="bk-spin" aria-hidden="true"></span> ${esc(p.running)}…</p>`;
  else if (st.error) line = `<p class="bk-err">The last backup (${esc(when(st.last_try))}) didn’t work: ${esc(st.error)}</p>`;
  else if (st.last_ok) line = `<p class="bk-ok">Last backup ${esc(when(st.last_ok))}: ${esc(bytes(st.size))} looked at, ${esc(bytes(st.added))} new, in ${st.took < 90 ? `${st.took} s` : `${Math.round(st.took / 60)} min`}.</p>`;
  else line = '<p class="dim">No backups yet.</p>';
  const list = snaps.length
    ? `<h3 class="bk-h">${snaps.length === 1 ? "1 backup kept" : `${snaps.length} backups kept`}</h3><div class="bk-snaps">${snaps.slice(0, 40).map(([id, t, size]) => `<div class="bk-snap"><b>${esc(when(t))}</b><span class="dim">${esc(bytes(size))}</span><code class="dim">${esc(id.slice(0, 8))}</code></div>`).join("")}</div>`
    : "";
  const off = !p.settings.on;
  return `<section class="card"><h2>Latest</h2>
    ${off ? '<p class="dim">Daily backups are off.</p>' : ""}
    ${line}
    <div class="row"><button class="btn main" data-act="bk-run" ${p.running || state.busy || !p.settings.kind || (!p.settings.folder && !p.settings.s3_bucket && !p.settings.sftp_address) ? "disabled" : ""}>Back up now</button></div>
    ${list}
    <details class="bk-restore"><summary>Restoring</summary>
      <p>Put back a whole server, on this computer or a new one, with the server stopped:</p>
      <pre>xenify-server backup list
xenify-server backup restore latest</pre>
      <p class="dim">On a new computer, install Xenify Server first and tell it where the backups are: <code>--from ${esc(p.place || "PLACE")}</code>. It asks for the passphrase${p.settings.kind === "s3" ? " (and reads the keys from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY)" : p.settings.kind === "sftp" ? " (and signs in with --ssh-key or a password)" : ""}.</p>
      <p class="dim">Backups are in restic’s format, so <code>restic</code> can list and restore them too, without Xenify.</p>
    </details></section>`;
}

export function backupsPage() {
  const { state, shell, errorBox } = $;
  const bk = state.bk;
  if (!bk.page || !bk.form) return shell('<div class="head"><h1>Backups</h1></div><p class="dim">Loading…</p>');
  const f = bk.form;
  return shell(`<div class="head"><h1>Backups</h1></div>
    <p class="dim">Every day, the database and everyone’s files, as one backup. Each one can be restored by itself.</p>${errorBox()}
    ${statusCard()}
    <section class="card"><label class="check bk-big"><input type="checkbox" data-act="bk-on" ${f.on ? "checked" : ""}> Back up every day</label></section>
    ${placeCard()}
    ${passCard()}
    ${whenCard()}
    <div class="row"><button class="btn main" data-act="bk-save" ${state.busy ? "disabled" : ""}>Save</button><span class="grow"></span></div>
    ${binCard()}`);
}

/// A passphrase of six words from a short list (about 60 bits), easy to
/// write down.
function makePassphrase() {
  const words = "amber apple arrow badge basil beach birch blaze bloom brook cabin camel cedar chalk charm cider cliff cloud clover coral crane creek crisp delta dune eagle ember fable fern field flint frost gable glade grape gravel hazel heron honey iris ivory jade kayak kettle lemon lilac linen lotus maple marsh meadow mint mossy nectar noble oasis olive opal orbit otter pearl pepper pine plum polar poppy quartz quill raven reed ridge river robin saffron sage satin shore silver slate solar spruce stone storm summit swift thistle tiger topaz tulip tundra velvet violet walnut willow winter yarrow zephyr".split(" ");
  const pick = new Uint32Array(6);
  crypto.getRandomValues(pick);
  return Array.from(pick, (n) => words[n % words.length]).join("-");
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    $.toast("Copied");
  } catch {
    $.fail("Couldn’t copy: select the text and copy it yourself.");
  }
}

function settingsOut() {
  const f = { ...$.state.bk.form };
  for (const k of ["hour", "keep_daily", "keep_weekly", "keep_monthly"]) f[k] = Math.max(0, parseInt(f[k], 10) || 0);
  f.keep_daily = Math.max(1, f.keep_daily);
  f.limit_mbit = Math.max(0, Number(f.limit_mbit) || 0);
  return f;
}

export const backupsActs = {
  "bk-kind"(el) {
    $.state.bk.form.kind = el.value;
    $.state.bk.check = null;
    $.render();
  },
  "bk-auth"(el) {
    $.state.bk.form.sftp_auth = el.value;
    $.render();
  },
  "bk-on"(el) {
    $.state.bk.form.on = el.checked;
    $.render();
  },
  "bk-encrypt"(el) {
    $.state.bk.form.encrypt = el.checked;
    $.render();
  },
  "bk-hour"(el) {
    $.state.bk.form.hour = Number(el.value);
  },
  "bk-num"(el) {
    $.state.bk.form[el.dataset.k] = el.value;
    $.render();
  },
  "bk-make-pass"() {
    const p = makePassphrase();
    $.state.bk.newPass = p;
    $.state.bk.form.passphrase = p;
    $.render();
  },
  "bk-copy-pass"() {
    copy($.state.bk.newPass);
  },
  "bk-copy-key"() {
    copy($.state.bk.page.public_key);
  },
  async "bk-check"() {
    const { state, api, render } = $;
    // After "the key changed", Check again takes the new key.
    const trust = !!state.bk.check?.key_changed;
    state.busy = true;
    state.bk.check = null;
    render();
    try {
      state.bk.check = await api("POST", "/v1/admin/backups/check", { settings: settingsOut(), trust });
      if (state.bk.check.ok) state.bk.page = await api("GET", "/v1/admin/backups");
    } catch (e) {
      state.bk.check = { ok: false, error: e.message };
    }
    state.busy = false;
    render();
  },
  async "bk-save"() {
    const { state, api, render, fail, toast } = $;
    const f = settingsOut();
    if (f.on && f.encrypt && !state.bk.page.settings.passphrase_set && (f.passphrase || "").length < 8) {
      return fail("Encrypted backups need a passphrase of at least 8 characters (or Make one).");
    }
    try {
      state.bk.page = await api("PUT", "/v1/admin/backups", { settings: f });
      state.bk.form = { ...state.bk.page.settings, passphrase: "", s3_secret: "", sftp_password: "" };
      state.bk.newPass = "";
      state.error = "";
      render();
      toast("Saved");
    } catch (e) {
      fail(e.message);
    }
  },
  async "bk-bin"(el) {
    const { state, api, render, fail, toast } = $;
    try {
      state.bk.page = await api("PUT", "/v1/admin/backups", { bin_days: Number(el.value) });
      render();
      toast("Saved");
    } catch (e) {
      fail(e.message);
    }
  },
  async "bk-run"() {
    const { state, api, render, fail } = $;
    try {
      await api("POST", "/v1/admin/backups/run");
      state.bk.page.running = "Starting";
      render();
      setTimeout(async () => {
        state.bk.page = await api("GET", "/v1/admin/backups");
        follow();
        render();
      }, 800);
    } catch (e) {
      fail(e.message);
    }
  },
};
