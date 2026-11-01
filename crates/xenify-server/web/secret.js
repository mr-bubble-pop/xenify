// A one-time secret (see src/secrets.rs): the link's # part holds the
// secret's id and its key (and, with a passphrase, its salt). Opening this
// page reads nothing; "Show secret" fetches it (which deletes it on the
// server) and opens it here.

const app = document.getElementById("app");
const [id, key, salt] = location.hash.slice(1).split(".");
let state = { stage: id && key ? "ready" : "broken", text: "", error: "", busy: false, copied: false, pass: "" };

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const b64 = (bytes) => btoa(String.fromCharCode(...bytes));
const unb64 = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=")), (c) => c.charCodeAt(0));

function render() {
  const head = `<div class="solo"><div class="top"><span class="mark" aria-hidden="true">X</span><b>${esc(document.title = "A secret for you")}</b></div><main>`;
  let body;
  if (state.stage === "broken") {
    body = `<div><h1>This link isn't whole</h1><p class="dim">${id && key ? "Its key doesn't open the secret, and the secret is gone now. Ask whoever sent it to share it again." : "Part of it is missing. Ask whoever sent it to copy the whole link again."}</p></div>`;
  } else if (state.stage === "ready") {
    body = `<div><h1>Someone shared a secret with you</h1><p class="dim">It can be shown once. After that, this link stops working, for you and for anyone else.</p></div>
      ${salt ? `<label class="field" for="pass"><span>Passphrase</span><input class="inp" id="pass" type="password" autocomplete="off" value="${esc(state.pass)}"></label><p class="dim">Whoever sent the link gives you this another way. Five wrong tries delete the secret.</p>` : ""}
      ${state.error ? `<div class="error" role="alert">${esc(state.error)}</div>` : ""}
      <div class="row end"><button class="btn main" data-act="show" ${state.busy ? "disabled" : ""}>${state.busy ? "Opening…" : "Show secret"}</button></div>`;
  } else if (state.stage === "shown") {
    body = `<div><h1>The secret</h1><p class="dim">Keep it somewhere safe now: this page won't show it again.</p></div>
      <pre class="key" style="white-space:pre-wrap;overflow-wrap:anywhere;margin:0">${esc(state.text)}</pre>
      <div class="row end"><button class="btn${state.copied ? "" : " main"}" data-act="copy">${state.copied ? "Copied" : "Copy"}</button></div>`;
  } else {
    body = `<div><h1>This secret is gone</h1><p class="dim">It was already viewed, it expired, or the passphrase was wrong too many times. If you didn't view it yourself, tell whoever sent it: someone else may have.</p></div>`;
  }
  app.innerHTML = `${head}<div class="sheet narrow">${body}</div></main></div>`;
}

// The same as Chat's secretKeys: with a passphrase, the key comes from the
// link's key and the passphrase, and a check the server compares.
async function keys(pass) {
  const linkKey = unb64(key);
  if (!salt) return { key: await crypto.subtle.importKey("raw", linkKey, "AES-GCM", false, ["decrypt"]), check: null };
  const base = await crypto.subtle.importKey("raw", new TextEncoder().encode(pass), "PBKDF2", false, ["deriveBits"]);
  const stretched = await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt: unb64(salt), iterations: 600000 }, base, 256);
  const hk = await crypto.subtle.importKey("raw", linkKey, "HKDF", false, ["deriveKey", "deriveBits"]);
  const how = (info) => ({ name: "HKDF", hash: "SHA-256", salt: stretched, info: new TextEncoder().encode(info) });
  return {
    key: await crypto.subtle.deriveKey(how("xenify secret key"), hk, { name: "AES-GCM", length: 256 }, false, ["decrypt"]),
    check: b64(new Uint8Array(await crypto.subtle.deriveBits(how("xenify secret check"), hk, 256))),
  };
}

async function show() {
  if (salt) state.pass = document.getElementById("pass")?.value || "";
  if (salt && !state.pass) return ((state.error = "Type the passphrase first."), render());
  state.busy = true;
  state.error = "";
  render();
  try {
    const k = await keys(state.pass);
    const r = await fetch(`/v1/secrets/${encodeURIComponent(id)}/open`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ check: k.check }) });
    if (r.status === 404) return ((state.stage = "gone"), (state.busy = false), render());
    const j = await r.json();
    if (j.code === "wrong_passphrase") {
      state.busy = false;
      if (!j.left) return ((state.stage = "gone"), render());
      state.left = j.left;
      state.error = `That passphrase is wrong. ${j.left === 1 ? "One more wrong try deletes it." : `${j.left} tries left.`}`;
      return render();
    }
    if (!r.ok) throw new Error(j.error || "It couldn't be opened.");
    const sealed = unb64(j.sealed);
    const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.slice(0, 12) }, k.key, sealed.slice(12));
    state.text = new TextDecoder().decode(plain);
    state.stage = "shown";
    // The key isn't needed any more: out of the address bar and history.
    history.replaceState(null, "", location.pathname);
  } catch (e) {
    // The server has let it go by now, so a wrong key can't be tried again.
    if (e instanceof DOMException) state.stage = "broken";
    else state.error = e.message;
  }
  state.busy = false;
  render();
}

app.addEventListener("click", (e) => {
  const act = e.target.closest("[data-act]")?.dataset.act;
  if (act === "show") show();
  if (act === "copy") navigator.clipboard.writeText(state.text).then(() => ((state.copied = true), render()));
});
app.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.id === "pass") show();
});
render();
// Another link pasted into this tab.
addEventListener("hashchange", () => location.reload());
