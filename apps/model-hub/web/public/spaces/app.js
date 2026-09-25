import { art } from "../card-art.mjs";
// Spaces — the page. It reads its own catalog (spaces.json, sealed roots per Space), asks the registry on this
// origin whether each Space is published there, and opens a Space in a sandboxed frame on this page. The
// Space's runtime narrates itself on a BroadcastChannel (index read · bytes verified · served from the
// store · refused), and the bar shows exactly that — no more words than the runtime has facts.
//
// Same shape as the Models and Registry pages: the lead row, facets on the left, search and sort on top, a
// card grid. The rail is the shared one (lib/rail.mjs); the facets are read off each Space's own record and
// off what this browser and the registry say about it, and their counts narrow as you choose.
import { createRail, ICONS as I } from "../lib/rail.mjs";

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
const picked = { task: new Set(), marks: new Set(), model: new Set(), publisher: new Set(), needs: new Set(), size: new Set(), origin: new Set() };

async function detect() {
  let webgpu = false;
  try { webgpu = !!(navigator.gpu && (await navigator.gpu.requestAdapter())); } catch {}
  const opfs = !!(navigator.storage && typeof navigator.storage.getDirectory === "function");
  return { webgpu, opfs };
}
const missing = (s) => (s.requires || []).filter((r) => !caps[r]);

// The registry on this origin: GET/HEAD need no token. A Space published as an OCI artifact under
// spaces/<id> answers its manifest; until then it is on the page only, and the card says so by saying nothing.
async function published(id) {
  try { const r = await fetch(`/v2/spaces/${id}/manifests/latest`, { headers: { Accept: ACCEPT } }); return r.ok ? (r.headers.get("docker-content-digest") || "yes") : null; }
  catch { return null; }
}

// ---- facets: every value a Space carries under a key
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
  onChange: render,
  order: { size: SIZES, marks: [RUNS, ON_REGISTRY] },
  lead: { marks: { value: ON_REGISTRY, className: "ours", title: "Published to this registry as a sealed artifact: its root is a digest you can check here" } },
});

function card(s) {
  const need = missing(s);
  const el = document.createElement("button");
  el.type = "button"; el.className = "card"; el.dataset.id = s.id;
  const foot = [`<span title="${s.root}">${short(s.root)}</span>`, `<span>${s.files} files · ${fmtMB(s.bytes)}</span>`];
  if (onRegistry[s.id]) foot.push(`<span title="${onRegistry[s.id]}">on the registry</span>`);
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
  // each Space ships its icon; fetch once so the cards and the frame agree
  await Promise.all(catalog.map(async (s) => { try { const t = await (await fetch(`./${s.id}/icon.svg`)).text(); s.iconSvg = t.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, ""); } catch {} }));
}

function status(kind, text) { $("s-text").textContent = text; $("s-dot").className = "dot" + (kind === "ok" ? " ok" : kind === "bad" ? " bad" : ""); }

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

function open(id) {
  const s = catalog.find((x) => x.id === id); if (!s) return;
  current = id;
  const stage = $("stage"), body = $("s-body");
  $("s-name").textContent = s.name; $("s-root").textContent = short(s.root); $("s-root").title = s.root;
  $("s-open").href = `./${s.id}/index.html`;
  body.innerHTML = "";
  const need = missing(s);
  if (need.length) {
    body.innerHTML = `<p class="need">This Space needs <b>${need.map((n) => ({ webgpu: "WebGPU", opfs: "private file storage (OPFS)" })[n] || n).join("</b> and <b>")}</b>, which this browser does not offer. It runs on current Chrome, Edge and Safari on a device with a GPU.</p>`;
    status("bad", "cannot run here");
  } else {
    listen(id);
    const f = document.createElement("iframe");
    f.title = s.name;
    f.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-downloads allow-modals");
    f.setAttribute("allow", "");
    f.referrerPolicy = "no-referrer";
    f.src = `./${s.id}/index.html`;
    body.appendChild(f);
    status("", "opening…");
  }
  stage.classList.add("on");
  const u = new URL(location.href); u.searchParams.set("open", id); history.replaceState(null, "", u);
  stage.scrollIntoView({ block: "start", behavior: "smooth" });
}
function close() {
  current = null; $("stage").classList.remove("on"); $("s-body").innerHTML = "";
  if (chan) { try { chan.close(); } catch {} chan = null; }
  const u = new URL(location.href); u.searchParams.delete("open"); history.replaceState(null, "", u);
}


async function main() {
  const [cat, c] = await Promise.all([fetch("./spaces.json").then((r) => r.json()), detect()]);
  catalog = cat.spaces; caps = c;
  await icons();
  rail.render();
  render();
  $("q").addEventListener("input", () => { rail.render(); render(); });
  $("sort").addEventListener("change", render);
  $("s-close").addEventListener("click", close);
  const want = new URLSearchParams(location.search).get("open");
  if (want && catalog.some((s) => s.id === want)) open(want);
  // registry presence, per Space: a mark on the card and a chip in the rail
  await Promise.all(catalog.map(async (s) => { const d = await published(s.id); if (d) onRegistry[s.id] = d; }));
  rail.render();
  render();
}
main();
