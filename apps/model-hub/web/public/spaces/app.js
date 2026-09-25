import { art } from "../card-art.mjs";
// Apps — the page. It reads its own catalog (spaces.json, sealed roots per App), asks the registry on this
// origin whether each App is published there, and opens an App in a sandboxed frame on this page. The
// App's runtime narrates itself on a BroadcastChannel (index read · bytes verified · served from the
// store · refused), and the bar shows exactly that — no more words than the runtime has facts.
//
// Same shape as the Models and Registry pages: the lead row, facets on the left, search and sort on top, a
// card grid. The rail is the shared one (lib/rail.mjs); the facets are read off each App's own record and
// off what this browser and the registry say about it, and their counts narrow as you choose.
import { createRail, ICONS as I } from "../lib/rail.mjs";
import { createSocial, toast } from "./social.js";

const $ = (id) => document.getElementById(id);
const fmtMB = (b) => (b >= 1e9 ? (b / 1e9).toFixed(2) + " GB" : Math.round(b / 1e6) + " MB");
const short = (k) => "κ:" + String(k).split(":").pop().slice(0, 8);
const ACCEPT = "application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json";
const NEED = { webgpu: "WebGPU", opfs: "Private file storage" };
const SIZES = ["Under 100 MB", "100 MB to 1 GB", "1 to 10 GB", "Over 10 GB"];
const sizeOf = (b) => (b < 1e8 ? SIZES[0] : b < 1e9 ? SIZES[1] : b < 1e10 ? SIZES[2] : SIZES[3]);
const RUNS = "Runs in this browser", ON_REGISTRY = "On the registry";

let catalog = [], caps = { webgpu: false, opfs: false }, chan = null, current = null;
const onRegistry = {}; // id → digest, once the registry has answered
const registryDiffers = {}; // id → digest the registry holds under this name when it is NOT this App's κ
const picked = { task: new Set(), marks: new Set(), model: new Set(), publisher: new Set(), needs: new Set(), size: new Set(), origin: new Set() };

async function detect() {
  let webgpu = false;
  try { webgpu = !!(navigator.gpu && (await navigator.gpu.requestAdapter())); } catch {}
  const opfs = !!(navigator.storage && typeof navigator.storage.getDirectory === "function");
  return { webgpu, opfs };
}
const missing = (s) => (s.requires || []).filter((r) => !caps[r]);

// The registry on this origin: GET/HEAD need no token. An App published as an OCI artifact under
// spaces/<id> answers its manifest; until then it is on the page only, and the card says so by saying nothing.
async function published(id) {
  try { const r = await fetch(`/v2/spaces/${id}/manifests/latest`, { headers: { Accept: ACCEPT } }); return r.ok ? (r.headers.get("docker-content-digest") || "yes") : null; }
  catch { return null; }
}

// ---- facets: every value an App carries under a key
function values(s, key) {
  switch (key) {
    case "task": return [s.task];
    case "marks": return [...(missing(s).length ? [] : [RUNS]), ...(onRegistry[s.id] ? [ON_REGISTRY] : [])];
    case "model": return s.models || [];
    case "publisher": return (s.models || []).map((m) => m.split("/")[0]);
    case "needs": return (s.requires || []).map((r) => NEED[r] || r);
    case "size": return [sizeOf(s.modelBytes)];
    case "origin": return [s.source.split("/spaces/")[1]?.split("/")[0] || new URL(s.source).host];
  }
  return [];
}
function hit(s) {
  const q = $("q").value.trim().toLowerCase();
  if (!q) return true;
  return [s.name, s.id, s.tagline, s.task, s.root, ...(s.models || [])].some((v) => String(v).toLowerCase().includes(q));
}
// `skip` leaves one facet's own choice out, so a chip's count says what choosing it would show.
const matches = (s, skip) => hit(s) && Object.keys(picked).every((k) => k === skip || !picked[k].size || values(s, k).some((v) => picked[k].has(v)));
function counts(key) {
  const c = {};
  for (const v of picked[key]) c[v] = 0;
  for (const s of catalog) if (matches(s, key)) for (const v of values(s, key)) c[v] = (c[v] || 0) + 1;
  return c;
}
const SORT = {
  featured: () => 0,
  name: (a, b) => a.name.localeCompare(b.name),
  small: (a, b) => a.modelBytes - b.modelBytes,
  large: (a, b) => b.modelBytes - a.modelBytes,
};

const rail = createRail({
  el: $("rail"),
  tabs: [
    ["Main", I.grid, ["task", "marks"]],
    ["Model", I.layers, ["model", "publisher"]],
    ["Needs", I.chip, ["needs"]],
    ["Size", I.box, ["size"]],
    ["Source", I.globe, ["origin"]],
  ],
  labels: { task: "Task", marks: "Marks", model: "Model", publisher: "Publisher", needs: "Needs", size: "Model size", origin: "Source" },
  icons: { task: I.tag, marks: I.seal, model: I.layers, publisher: I.seal, needs: I.chip, size: I.box, origin: I.globe },
  counts,
  picked,
  onChange: () => refresh(),
  order: { size: SIZES, marks: [RUNS, ON_REGISTRY] },
  lead: { marks: { value: ON_REGISTRY, className: "ours", title: "Published to this registry as a sealed artifact: its root is a digest you can check here" } },
});

function card(s) {
  const need = missing(s);
  const el = document.createElement("button");
  el.type = "button"; el.className = "card"; el.dataset.id = s.id;
  const foot = [`<span title="${s.root}">${short(s.root)}</span>`, `<span>${s.files} files · ${fmtMB(s.bytes)}</span>`];
  if (onRegistry[s.id]) foot.push(`<span title="${onRegistry[s.id]}">on the registry</span>`);
  else if (registryDiffers[s.id]) foot.push(`<span title="${registryDiffers[s.id]}" style="color:var(--bad)">registry differs</span>`);
  el.innerHTML = `<span class="logo"><svg viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">${s.iconSvg || ""}</svg></span>
    <span class="tags"><span class="tag">${s.task}</span><span class="tag ${need.length ? "no" : "here"}">${need.length ? "needs " + need.join(" + ") : "runs here"}</span><span class="tag">${fmtMB(s.modelBytes)} model</span></span>
    <h3>${s.name}</h3><p>${s.tagline}</p>
    <span class="foot">${foot.join("")}</span>${art(s.root)}`;
  el.addEventListener("click", () => open(s.id));
  return el;
}

function render() {
  const rows = catalog.filter((s) => matches(s)).sort(SORT[$("sort").value] || SORT.featured);
  const grid = $("grid"); grid.innerHTML = ""; for (const s of rows) grid.appendChild(card(s));
  $("count").textContent = String(rows.length);
  $("empty").hidden = rows.length > 0;
}

async function icons() {
  // each App ships its icon; fetch once so the cards and the frame agree
  await Promise.all(catalog.map(async (s) => { try { const t = await (await fetch(`./${s.id}/icon.svg`)).text(); s.iconSvg = t.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, ""); } catch {} }));
}

// The runtime's own account of itself, shown at the head of the description where a video shows its views.
let lastStatus = ["", "opening…"];
function status(kind, text) {
  lastStatus = [kind, text];
  const t = $("s-text"), d = $("s-dot");
  if (t) t.textContent = text;
  if (d) d.className = "dot" + (kind === "ok" ? " ok" : kind === "bad" ? " bad" : "");
}

function listen(id) {
  if (chan) { try { chan.close(); } catch {} chan = null; }
  try { chan = new BroadcastChannel("holo-spaces:" + id); } catch { return; }
  let verified = 0, ms = 0;
  chan.onmessage = ({ data: e }) => {
    if (!e || current !== id) return;
    if (e.kind === "tree") status("", `index read · ${e.files} files`);
    else if (e.kind === "served" && e.source === "network") { verified += e.bytes; ms += e.ms; status("ok", `verified ${fmtMB(verified)} · ${(ms / 1000).toFixed(1)} s`); }
    else if (e.kind === "served" && e.source === "cache") status("ok", `from your store · ${fmtMB(e.bytes)} · ${e.ms} ms`);
    else if (e.kind === "refused") status("bad", `refused ${e.path}: bytes did not match the index`);
    else if (e.kind === "index-failed") status("bad", "model index unreachable");
    else if (e.kind === "armed") status("", "verified fetch armed");
    else if (e.kind === "error") status("bad", String(e.error).slice(0, 90));
  };
}

// ---- watch: one App open, in the shape of a video page
//
// The frame, its title and the row of actions are one block exactly as tall as the screen below the header, so
// all three are on screen the moment it opens; the description and the comments follow beneath. Every other App
// lines the right under a search and a filter that are the catalogue's own: one query, one set of chosen facets,
// so leaving the App shows the catalogue filtered exactly as the list was. Nothing is drawn over the frame.
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const iconSvg = (s, cls = "") => `<svg class="${cls}" viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${s.iconSvg || ""}</svg>`;
const publisherOf = (s) => (s.models || [])[0]?.split("/")[0] || new URL(s.source).host;
const social = createSocial();
let comments = null, descOpen = false;

// Saved is this reader's own shortlist, kept in this browser only; it is a convenience, not a record.
const SAVED = "hologram-apps.saved";
const savedSet = () => { try { return new Set(JSON.parse(localStorage.getItem(SAVED)) || []); } catch { return new Set(); } };
const saveSet = (set) => { try { localStorage.setItem(SAVED, JSON.stringify([...set])); } catch {} };

function describe(s) {
  const need = missing(s);
  const models = (s.models || []).map((m) => `<a href="/models/${esc(m)}/">${esc(m)}</a>`).join(", ");
  const rows = [
    ["Task", esc(s.task)],
    ["Model", models],
    ["Model size", fmtMB(s.modelBytes)],
    ["App", `${s.files} files · ${fmtMB(s.bytes)}`],
    ["Root", `<span title="${esc(s.root)}">${esc(s.root)}</span>`],
    s.ipfs ? ["IPFS", esc(s.ipfs)] : null,
    s.ort ? ["Runtime", esc(s.ort)] : null,
    ["Needs", (s.requires || []).map((r) => NEED[r] || r).join(" + ") || "nothing beyond a browser"],
    ["Registry", onRegistry[s.id] ? `on the registry as <span title="${esc(onRegistry[s.id])}">${esc(short(onRegistry[s.id]))}</span>` : registryDiffers[s.id] ? `<span style="color:var(--bad)">the registry holds a different digest under this name</span>` : "not published yet"],
    ["Source", `<a href="${esc(s.source)}" target="_blank" rel="noopener">${esc(s.source.replace(/^https?:\/\//, ""))}</a>`],
  ].filter(Boolean);
  const [kind, text] = lastStatus;
  const box = $("s-desc");
  box.classList.toggle("open", descOpen);
  box.setAttribute("aria-expanded", String(descOpen));
  box.innerHTML = `<div class="desc-head"><span class="status"><span class="dot${kind === "ok" ? " ok" : kind === "bad" ? " bad" : ""}" id="s-dot"></span><span id="s-text">${esc(text)}</span></span><span>${fmtMB(s.modelBytes)} model</span><span>${s.files} files</span><span>${esc(s.task)}</span></div>
    <p class="desc-lede">${esc(s.tagline)}${need.length ? ` <span style="color:var(--dim)">Needs ${need.map((n) => NEED[n] || n).join(" and ")}, which this browser does not offer.</span>` : ""}${descOpen ? "" : ' <button type="button" class="desc-more">…more</button>'}</p>
    <div class="desc-full"><dl>${rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("")}</dl><button type="button" class="desc-more less">Show less</button></div>`;
}

// Likes as the App's source counts them (spaces.meta.mjs reads the Hugging Face Space nightly and writes them into
// the catalog; this page fetches nothing from Hugging Face). A browser that cannot run the App is still told so.
const HEART = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M19.5 12.6 12 20l-7.5-7.4A4.6 4.6 0 0 1 12 6.6a4.6 4.6 0 0 1 7.5 6Z"/></svg>';
const compact = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1).replace(/\.0$/, "")}K` : String(n));
function likesLine(s, need) {
  const m = s.sourceMeta;
  const likes = m && Number.isFinite(m.likes) ? `<span class="likes" title="${esc(`${m.likes.toLocaleString("en-US")} likes on Hugging Face, read ${m.read}`)}">${HEART}${compact(m.likes)}</span>` : "";
  const warn = need.length ? `<span class="warn">needs ${need.map((n) => NEED[n] || n).join(" + ")}</span>` : "";
  return likes || warn ? `<span class="meta-line">${likes}${warn}</span>` : "";
}

// ---- the list on the right, and its search and filter
function nextRows() {
  return catalog.filter((s) => s.id !== current && matches(s)).sort(SORT[$("sort").value] || SORT.featured);
}
function upnext() {
  const box = $("nlist"); box.innerHTML = "";
  const rows = nextRows();
  if (!rows.length) {
    const others = catalog.filter((s) => s.id !== current).length;
    box.innerHTML = `<div class="nempty">${others ? "No other Apps match." : "This is the only App so far."}${others && filtering() ? ' <button type="button" class="pill" id="nclear">Clear filters</button>' : ""}</div>`;
    $("nclear")?.addEventListener("click", clearFilters);
    return;
  }
  for (const s of rows) {
    const need = missing(s);
    const el = document.createElement("button");
    el.type = "button"; el.className = "next"; el.dataset.id = s.id;
    el.innerHTML = `<span class="thumb">${art(s.root)}${iconSvg(s, "icon")}<span class="len">${fmtMB(s.modelBytes)}</span></span>
      <span class="text"><b>${esc(s.name)}</b><span>${esc(s.task)} · ${esc(publisherOf(s))}</span>${likesLine(s, need)}</span>`;
    el.addEventListener("click", () => open(s.id));
    box.appendChild(el);
  }
}
const chosen = () => Object.values(picked).reduce((n, set) => n + set.size, 0);
const filtering = () => chosen() > 0 || $("q").value.trim() !== "";
function badge() { const n = chosen(), b = $("nbadge"); b.hidden = !n; b.textContent = String(n); }
function clearFilters() {
  for (const set of Object.values(picked)) set.clear();
  $("q").value = ""; $("nq").value = "";
  refresh();
}
// The rail every catalogue page has, a second view of the same `picked`: a chip chosen here is chosen there.
const nrail = createRail({
  el: $("npanel"),
  tabs: [
    ["Main", I.grid, ["task", "marks"]],
    ["Model", I.layers, ["model", "publisher"]],
    ["Needs", I.chip, ["needs"]],
    ["Size", I.box, ["size"]],
    ["Source", I.globe, ["origin"]],
  ],
  labels: { task: "Task", marks: "Marks", model: "Model", publisher: "Publisher", needs: "Needs", size: "Model size", origin: "Source" },
  icons: { task: I.tag, marks: I.seal, model: I.layers, publisher: I.seal, needs: I.chip, size: I.box, origin: I.globe },
  counts,
  picked,
  onChange: () => refresh(),
  order: { size: SIZES, marks: [RUNS, ON_REGISTRY] },
  lead: { marks: { value: ON_REGISTRY, className: "ours", title: "Published to this registry as a sealed artifact: its root is a digest you can check here" } },
});
function refresh() {
  rail.render(); render();
  if (current) { nrail.render(); upnext(); badge(); }
}

// ---- the block under the frame
async function stats(s) {
  const st = await social.stats(s.id);
  const like = $("s-like"), dislike = $("s-dislike");
  $("s-likes").textContent = st && st.likes ? st.likes.toLocaleString("en-US") : "";
  like.setAttribute("aria-pressed", String(st?.mine?.react === 1));
  dislike.setAttribute("aria-pressed", String(st?.mine?.react === -1));
}
async function rate(value) {
  const s = catalog.find((x) => x.id === current); if (!s) return;
  const was = $("s-like").getAttribute("aria-pressed") === "true" ? 1 : $("s-dislike").getAttribute("aria-pressed") === "true" ? -1 : 0;
  const next = was === value ? 0 : value;
  try {
    const out = await social.react(s.id, next);
    $("s-likes").textContent = out.likes ? out.likes.toLocaleString("en-US") : "";
    $("s-like").setAttribute("aria-pressed", String(out.mine.react === 1));
    $("s-dislike").setAttribute("aria-pressed", String(out.mine.react === -1));
  } catch (e) { if (e.code !== "signin") toast(e.message); }
}
function paintSaved() {
  const on = savedSet().has(current);
  const b = $("s-save");
  b.setAttribute("aria-pressed", String(on));
  b.querySelector("span").textContent = on ? "Saved" : "Save";
}

// The frame block is as tall as what is left of the screen below its own top edge.
function fitStage() {
  const stage = $("stage");
  if (!stage || !current) return;
  const top = stage.getBoundingClientRect().top + window.scrollY;
  document.documentElement.style.setProperty("--stage-top", `${Math.round(top)}px`);
}

function open(id) {
  const s = catalog.find((x) => x.id === id); if (!s) return;
  current = id;
  descOpen = false;
  lastStatus = ["", "opening…"];
  const player = $("player");
  $("s-name").textContent = s.name;
  $("s-mark").innerHTML = iconSvg(s);
  $("s-pub").textContent = publisherOf(s);
  $("s-sub").textContent = `${s.task} · ${short(s.root)}`;
  $("s-sub").title = s.root;
  $("s-open").href = `./${s.id}/index.html`;
  $("s-source").href = s.source;
  player.innerHTML = "";
  const need = missing(s);
  if (need.length) {
    player.innerHTML = `<div class="need"><p>This App needs <b>${need.map((n) => ({ webgpu: "WebGPU", opfs: "private file storage (OPFS)" })[n] || n).join("</b> and <b>")}</b>, which this browser does not offer. It runs on current Chrome, Edge and Safari on a device with a GPU.</p></div>`;
    lastStatus = ["bad", "cannot run here"];
  } else {
    listen(id);
    const f = document.createElement("iframe");
    f.title = s.name;
    f.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-downloads allow-modals");
    f.setAttribute("allow", "");
    f.referrerPolicy = "no-referrer";
    f.src = `./${s.id}/index.html`;
    player.appendChild(f);
  }
  describe(s);
  $("nq").value = $("q").value;
  $("main").classList.add("watching");
  nrail.render(); upnext(); badge(); paintSaved();
  $("s-likes").textContent = "";
  $("s-like").setAttribute("aria-pressed", "false"); $("s-dislike").setAttribute("aria-pressed", "false");
  stats(s);
  comments?.destroy();
  comments = social.mountComments($("comments"), s.id);
  document.title = `${s.name} · Apps · Hologram Models Hub`;
  const u = new URL(location.href); u.searchParams.set("open", id); history.replaceState(null, "", u);
  window.scrollTo({ top: 0, behavior: "smooth" });
  requestAnimationFrame(fitStage);
}
function close() {
  current = null; $("main").classList.remove("watching"); $("player").innerHTML = ""; $("nlist").innerHTML = "";
  comments?.destroy(); comments = null; $("comments").innerHTML = "";
  if (chan) { try { chan.close(); } catch {} chan = null; }
  document.title = "Apps · Hologram Models Hub";
  const u = new URL(location.href); u.searchParams.delete("open"); history.replaceState(null, "", u);
  refresh();
}

function wireWatch() {
  // the description: a click anywhere on it opens it, "Show less" folds it
  $("s-desc").addEventListener("click", (e) => {
    if (e.target.closest("a")) return;
    const s = catalog.find((x) => x.id === current); if (!s) return;
    if (e.target.closest(".less")) descOpen = false; else if (!descOpen) descOpen = true; else return;
    describe(s);
  });
  $("s-desc").addEventListener("keydown", (e) => { if ((e.key === "Enter" || e.key === " ") && !descOpen && e.target === $("s-desc")) { e.preventDefault(); descOpen = true; describe(catalog.find((x) => x.id === current)); } });
  $("s-like").addEventListener("click", () => rate(1));
  $("s-dislike").addEventListener("click", () => rate(-1));
  $("s-share").addEventListener("click", async () => {
    const url = location.href;
    const s = catalog.find((x) => x.id === current);
    if (navigator.share && matchMedia("(pointer: coarse)").matches) { try { await navigator.share({ title: s?.name, url }); return; } catch {} }
    try { await navigator.clipboard.writeText(url); toast("Link copied"); } catch { toast(url); }
  });
  $("s-save").addEventListener("click", () => {
    const set = savedSet();
    if (set.has(current)) { set.delete(current); toast("Removed from saved"); } else { set.add(current); toast("Saved in this browser"); }
    saveSet(set); paintSaved();
  });
  const more = $("s-more"), menu = $("s-more-menu");
  const fold = (on) => { menu.hidden = !on; more.setAttribute("aria-expanded", String(on)); };
  more.addEventListener("click", (e) => { e.stopPropagation(); fold(menu.hidden); });
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest(".more-wrap")) fold(false); });
  $("s-copy").addEventListener("click", async () => {
    const s = catalog.find((x) => x.id === current); if (!s) return;
    try { await navigator.clipboard.writeText(s.root); toast("κ copied"); } catch { toast(s.root); }
    fold(false);
  });
  $("s-close").addEventListener("click", () => { fold(false); close(); });
  // the search on the right is the catalogue's search: one query, typed in either place
  $("nq").addEventListener("input", () => { $("q").value = $("nq").value; refresh(); });
  $("nfilter").addEventListener("click", () => {
    const panel = $("npanel"), on = panel.hidden;
    panel.hidden = !on;
    $("nfilter").setAttribute("aria-expanded", String(on));
    if (on) nrail.render();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !current) return;
    if (!menu.hidden) return fold(false);
    if (!$("npanel").hidden) { $("npanel").hidden = true; $("nfilter").setAttribute("aria-expanded", "false"); return; }
    if (e.target.closest?.("textarea, input")) return;
    close();
  });
  addEventListener("resize", () => requestAnimationFrame(fitStage));
}

async function main() {
  const [cat, c] = await Promise.all([fetch("./spaces.json").then((r) => r.json()), detect()]);
  catalog = cat.spaces; caps = c;
  await icons();
  rail.render();
  render();
  $("q").addEventListener("input", () => { rail.render(); render(); });
  $("sort").addEventListener("change", render);
  wireWatch();
  const want = new URLSearchParams(location.search).get("open");
  if (want && catalog.some((s) => s.id === want)) open(want);
  // registry presence, per App: a mark on the card, a chip in the rail, a count in the lead row
  // the registry's digest must be the card's κ (the manifest digest): the same bytes under the same name,
  // or it is not this App — a differing digest is shown as such, never as "on the registry"
  await Promise.all(catalog.map(async (s) => { const d = await published(s.id); if (d) { if (d === s.root) onRegistry[s.id] = d; else registryDiffers[s.id] = d; } }));
  refresh();
  // an App already open learns what the registry said, too: its record and the rows beside it
  if (current) { const s = catalog.find((x) => x.id === current); if (s) describe(s); }
}
main();
