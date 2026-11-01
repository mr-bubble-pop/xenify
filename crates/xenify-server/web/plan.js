// Admin › Holidays: the region whose public holidays everyone gets (people
// can pick their own in Plan), and company holidays. Plan schedules around
// them; Chat's calendar shows them and Find a time avoids them. The page
// also says what the server keeps of Plan's bookings.
//
// app.js hands over its helpers with planInit(); its render() calls
// planPage() and its click wiring runs planActs.

let $ = null; // { state, api, esc, shell, errorBox, render, busy, fail, toast }

export function planInit(env) {
  $ = env;
  $.state.hol = { set: null, countries: [], days: [], year: new Date().getFullYear(), add: { from: "", to: "", name: "" } };
}

export async function loadPlan() {
  const hol = $.state.hol;
  [hol.set, hol.countries] = await Promise.all([$.api("GET", "/v1/plan/holidays"), $.api("GET", "/v1/plan/countries")]);
  await loadDays();
}

async function loadDays() {
  const hol = $.state.hol;
  const [a, b] = await Promise.all([$.api("GET", `/v1/plan/holidays/${hol.year}`), $.api("GET", `/v1/plan/holidays/${hol.year + 1}`)]);
  const today = new Date().toISOString().slice(0, 10);
  hol.days = [...a, ...b].filter((d) => d.to >= today && !d.company).slice(0, 10);
}

const W = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const M = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
function day(s) {
  const d = new Date(`${s}T00:00:00Z`);
  return `${W[d.getUTCDay()]} ${d.getUTCDate()} ${M[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}
function span(f, t) {
  return f === t ? day(f) : `${day(f)} – ${day(t)}`;
}

export function planPage() {
  const { state, shell, errorBox, esc } = $;
  const hol = state.hol;
  if (!hol.set) return shell('<div class="head"><h1>Holidays</h1></div><p class="dim">Loading…</p>');
  const c = hol.countries.find((x) => x.code === hol.set.country);
  const countryOpts = `<option value="">None</option>${hol.countries.map((x) => `<option value="${esc(x.code)}" ${x.code === hol.set.country ? "selected" : ""}>${esc(x.name)}</option>`).join("")}`;
  const regionOpts = `<option value="">${c?.regions.length ? "All of it" : "—"}</option>${(c?.regions || []).map(([k, n]) => `<option value="${esc(k)}" ${k === hol.set.region ? "selected" : ""}>${esc(n)}</option>`).join("")}`;
  const company = (hol.set.company_days || []).map((d, i) => `<div class="hol-row"><b>${esc(span(d[0], d[1]))}</b><span class="grow">${esc(d[2])}</span><button class="btn small" data-act="hol-unday" data-id="${i}">Remove</button></div>`).join("")
    || '<p class="dim">None yet.</p>';
  const add = hol.add;
  return shell(`<div class="head"><h1>Holidays</h1></div><p class="dim">Days off for everyone. Plan schedules around them; Chat’s calendar shows them and Find a time avoids them.</p>${errorBox()}
    <section class="card"><h2>Region for everyone</h2>
      <div class="row wrap"><label class="field"><span>Country</span><select class="inp" data-act="hol-country">${countryOpts}</select></label>
      <label class="field"><span>Region</span><select class="inp" data-act="hol-region" ${c?.regions.length ? "" : "disabled"}>${regionOpts}</select></label></div>
      <p class="dim">People can pick their own region in Plan (People, or Settings › My region and time off).</p></section>
    <section class="card"><h2>Company holidays</h2><p class="dim">Days the whole company is off, besides public holidays: a Christmas shutdown, a company anniversary.</p>${company}
      <div class="row wrap"><label class="field"><span>First day</span><input class="inp" type="date" data-bind="hol.add.from" value="${esc(add.from)}"></label>
      <label class="field"><span>Last day</span><input class="inp" type="date" data-bind="hol.add.to" value="${esc(add.to)}"></label>
      <label class="field grow"><span>Name</span><input class="inp" data-bind="hol.add.name" value="${esc(add.name)}" placeholder="Christmas shutdown"></label>
      <button class="btn main" data-act="hol-day">Add</button></div></section>
    <section class="card"><h2>Coming up${c ? ` · ${esc(hol.set.region ? (c.regions.find((r) => r[0] === hol.set.region)?.[1] || c.name) : c.name)}` : ""}</h2>
      ${hol.days.length ? hol.days.map((d) => `<div class="hol-row"><b>${esc(day(d.from))}</b><span class="grow">${esc(d.name)}</span>${d.moved_from ? `<span class="dim">moved from ${esc(day(d.moved_from))}</span>` : ""}</div>`).join("") : '<p class="dim">Pick a country to see its holidays.</p>'}
      <p class="dim">Built in for every year; no internet needed.</p></section>
    <section class="card"><h2>Plan bookings on this server</h2>
      <p>So Plan can add up how busy people are across all plans, this server keeps who is booked when and how much, for example “Sam, 7–11 Dec, 100%”.</p>
      <p class="dim">Task names in bookings are shown only to people who may open that plan, or whose own task it is.</p></section>`);
}

async function save(next) {
  const { state, api, render, fail, toast } = $;
  try {
    state.hol.set = await api("PUT", "/v1/admin/plan/holidays", next);
    state.error = "";
    await loadDays();
    render();
    toast("Saved");
  } catch (e) {
    fail(e.message);
  }
}

export const planActs = {
  async "hol-country"(el) {
    const s = $.state.hol.set;
    await save({ country: el.value, region: "", company_days: s.company_days || [] });
  },
  async "hol-region"(el) {
    const s = $.state.hol.set;
    await save({ country: s.country, region: el.value, company_days: s.company_days || [] });
  },
  async "hol-day"() {
    const hol = $.state.hol;
    const a = hol.add;
    if (!a.from || !a.name.trim()) { $.fail("A company holiday needs a first day and a name."); return; }
    const days = [...(hol.set.company_days || []), [a.from, a.to && a.to >= a.from ? a.to : a.from, a.name.trim()]].sort((x, y) => (x[0] < y[0] ? -1 : 1));
    hol.add = { from: "", to: "", name: "" };
    await save({ country: hol.set.country, region: hol.set.region, company_days: days });
  },
  async "hol-unday"(el) {
    const hol = $.state.hol;
    const days = (hol.set.company_days || []).filter((_, i) => i !== Number(el.dataset.id));
    await save({ country: hol.set.country, region: hol.set.region, company_days: days });
  },
};
