// Spaces — the page. It reads its own catalog (spaces.json, sealed roots per Space), asks the registry on this
// origin whether each Space is published there, and opens a Space in a sandboxed frame on this page. The
// Space's runtime narrates itself on a BroadcastChannel (index read · bytes verified · served from the
// store · refused), and the bar shows exactly that — no more words than the runtime has facts.
const $ = (id) => document.getElementById(id);
const fmtMB = (b) => (b >= 1e9 ? (b / 1e9).toFixed(2) + " GB" : Math.round(b / 1e6) + " MB");
const short = (k) => "κ:" + String(k).split(":").pop().slice(0, 8);
const ACCEPT = "application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json";

let catalog = [], caps = { webgpu: false, opfs: false }, chan = null, current = null;

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

// The Models page's faceted mesh, seeded by the entry's own address: same bytes, same surface.
// Lit facets mark what this registry can check itself; everything else shows the bare wireframe.
const ART_W = 260, ART_H = 120;
function art(seed, lit) {
  let h = 2166136261;
  for (const c of String(seed)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  const r = () => { h ^= h << 13; h ^= h >>> 17; h ^= h << 5; return ((h >>> 0) % 100000) / 100000; };
  const f = (n) => n.toFixed(1);
  const cols = 9, rows = 4, gx = ART_W / (cols - 1), gy = ART_H / (rows - 1), p = [];
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    p.push([x * gx + (r() - 0.5) * gx * 0.5, y * gy + (y && y < rows - 1 ? (r() - 0.5) * gy * 0.5 : 0)]);
  }
  let edges = "", faces = "";
  for (let y = 0; y < rows - 1; y++) for (let x = 0; x < cols - 1; x++) {
    const a = p[y * cols + x], b = p[y * cols + x + 1], c = p[(y + 1) * cols + x], d = p[(y + 1) * cols + x + 1];
    for (const t of [[a, b, d], [a, d, c]]) {
      const path = `M${t.map((q) => `${f(q[0])} ${f(q[1])}`).join("L")}Z`;
      edges += path;
      const v = r();
      if (lit && v < 0.35) faces += `<path d="${path}" opacity="${f(0.02 + v * 0.12)}"/>`;
    }
  }
  return `<svg class="art${lit ? " lit" : ""}" viewBox="0 0 ${ART_W} ${ART_H}" preserveAspectRatio="xMaxYMid slice" aria-hidden="true"><g class="facets">${faces}</g><path class="edges" d="${edges}"/></svg>`;
}

function card(s) {
  const need = missing(s);
  const el = document.createElement("button");
  el.type = "button"; el.className = "card"; el.dataset.id = s.id;
  el.innerHTML = `<span class="logo"><svg viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">${s.iconSvg || ""}</svg></span>
    <span class="tags"><span class="tag">${s.task}</span><span class="tag ${need.length ? "no" : "here"}">${need.length ? "needs " + need.join(" + ") : "runs here"}</span><span class="tag">${fmtMB(s.modelBytes)} model</span></span>
    <h3>${s.name}</h3><p>${s.tagline}</p>
    <span class="foot"><span title="${s.root}">${short(s.root)}</span><span>${s.files} files · ${fmtMB(s.bytes)}</span><span id="pub-${s.id}"></span></span>${art(s.root, !need.length)}`;
  el.addEventListener("click", () => open(s.id));
  return el;
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
  $("host").textContent = location.host + "/v2/spaces/";
  const [cat, c] = await Promise.all([fetch("./spaces.json").then((r) => r.json()), detect()]);
  catalog = cat.spaces; caps = c;
  await icons();
  $("count").textContent = String(catalog.length);
  const grid = $("grid"); grid.innerHTML = ""; for (const s of catalog) grid.appendChild(card(s));
  $("s-close").addEventListener("click", close);
  // registry presence, per Space
  let any = 0;
  await Promise.all(catalog.map(async (s) => { const d = await published(s.id); if (d) { any++; const el = $(`pub-${s.id}`); if (el) { el.textContent = "on the registry"; el.title = d; } } }));
  $("dot").className = "dot" + (any ? " ok" : "");
  $("dot").title = any ? `${any} of ${catalog.length} published` : "not published to the registry yet";
  const want = new URLSearchParams(location.search).get("open");
  if (want && catalog.some((s) => s.id === want)) open(want);
}
main();
