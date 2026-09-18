// Static build: web/data + web/src + vendored brand kit → web/dist.
//
//   node build.mjs                 base / (set BASE=/path/ when served under a path;
//                                  Git Bash: prefix MSYS_NO_PATHCONV=1)

import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as R from "./src/render.mjs";
import * as B from "./src/braille.mjs";
import { overview, metaDescription } from "./src/overview.mjs";

const SITE = dirname(fileURLToPath(import.meta.url));
const DIST = join(SITE, "dist");
const KIT = join(SITE, "vendor", "hologram-brand-kit");
const base = process.env.BASE || "/";
const REPO = "https://github.com/Hologram-Technologies/hologram-live/tree/main/apps/model-hub";
const INDEX = "https://github.com/humuhumu33/hologram-api";

const data = JSON.parse(await readFile(join(SITE, "data", "models.json"), "utf8"));
const models = R.prepare(data.models, data.snapshot);
const archivePath = join(SITE, "data", "archive.json");
const archive = existsSync(archivePath) ? JSON.parse(await readFile(archivePath, "utf8")) : null;

// The header pill. With an archive it opens every captured day; the Wayback idea, one control.
function indexPill() {
  const latest = `Index ${R.day(data.snapshot)}`;
  if (!archive) return `<a class="status" href="${INDEX}" title="Addresses refresh daily">${latest}</a>`;
  const days = [...archive.days].sort((a, b) => b.date.localeCompare(a.date));
  const months = new Map();
  for (const d of days) {
    const key = d.date.slice(0, 7);
    if (!months.has(key)) months.set(key, []);
    months.get(key).push(d);
  }
  const monthName = (key) => new Date(`${key}-01T00:00:00Z`).toLocaleDateString("en-US", { month: "long", year: "numeric", timeZone: "UTC" });
  const row = (d) => `<button type="button" role="menuitemradio" data-at="${d.date}" aria-checked="false">${R.icon.calendar}<span class="label">${R.day(d.date)}<span class="sub">${d.models} models, ${d.addressed} verified</span></span>${R.icon.check.replace('class="i"', 'class="i tick"')}</button>`;
  const groups = [...months].map(([key, list], i) => `<div class="archive-month"${i >= 3 ? " data-older" : ""}><h3>${monthName(key)}</h3>${list.map(row).join("")}</div>`);
  const older = groups.length > 3 ? `<details class="archive-older"><summary>Older</summary>${groups.slice(3).join("")}</details>` : "";
  return `<div class="archive" id="archive">
      <button type="button" class="status" id="archive-button" aria-haspopup="menu" aria-expanded="false" aria-controls="archive-menu" title="Every day's index is stored on IPFS. Open any day."><span id="archive-label">${latest}</span>${R.icon.chevron}</button>
      <div class="menu" id="archive-menu" role="menu" aria-label="Index history" hidden>
        <button type="button" role="menuitemradio" data-at="latest" aria-checked="true">${R.icon.check.replace('class="i"', 'class="i lead"')}<span class="label">Latest<span class="sub">${R.day(data.snapshot)}, ${models.length} models</span></span>${R.icon.check.replace('class="i"', 'class="i tick"')}</button>
        <div class="archive-days">${groups.slice(0, 3).join("")}${older}</div>
        <p class="menu-note">Every day is stored on IPFS and checked in your browser before it is shown. The hub keeps only the current day; older days come from IPFS and can take a minute to open the first time.</p>
        <div class="archive-foot"><button type="button" class="copy" id="archive-cid" data-copy="" title="Copy this day's IPFS address">CID${R.icon.copy}</button><button type="button" class="copy" id="archive-pull" data-copy="" title="Copy the hologram pull command for the current index">hologram pull${R.icon.copy}</button></div>
      </div>
    </div>
    <script type="application/json" id="archive-days">${JSON.stringify({ gateway: archive.gateway, mirror: archive.mirror || null, registry: archive.registry || null, latest: data.snapshot, days: days.map(({ date, cid, index, models: n }) => ({ date, cid, index, models: n })) })}</script>`;
}

const WALLPAPERS = [
  { key: "alps", name: "Alpine Dawn", by: "Unsplash", url: "https://unsplash.com/?utm_source=Hologram&utm_medium=referral" },
  { key: "galaxy", name: "Galaxy", by: "Tiago Ferreira", url: "https://unsplash.com/@tiago_f_ferreira?utm_source=Hologram&utm_medium=referral" },
  { key: "aurora", name: "Aurora", by: "Lightscape", url: "https://unsplash.com/@lightscape?utm_source=Hologram&utm_medium=referral" },
];
const THEMES = [["dark", "Dark", "moon"], ["light", "Light", "sun"], ["immersive", "Immersive", "image"]];

// Runs before first paint: Dark for first visits, the saved choice after that. No flash.
const prepaint = `(function(){var s={};try{s=JSON.parse(localStorage.getItem("hologram-models-hub.theme"))||{}}catch(e){}
var m=["dark","light","immersive"].indexOf(s.mode)>=0?s.mode:"dark",w=${JSON.stringify(WALLPAPERS.map((w) => w.key))}.indexOf(s.wallpaper)>=0?s.wallpaper:"alps",r=document.documentElement;
r.setAttribute("data-theme",m);r.setAttribute("data-wallpaper",w);r.classList.toggle("dark",m!=="light");
if(m==="immersive"){var l=document.createElement("link");l.rel="preload";l.as="image";l.href="${base}wallpapers/"+w+".jpg";document.head.appendChild(l)}})();`;

const themeSwitch = `<div class="appearance">
      <button type="button" id="theme-button" aria-haspopup="menu" aria-expanded="false" aria-controls="theme-menu" aria-label="Theme" title="Theme">${THEMES.map(([k, , ic]) => R.icon[ic].replace('class="i"', `class="i" data-for="${k}"`)).join("")}</button>
      <div class="menu" id="theme-menu" role="menu" aria-label="Theme" hidden>
        ${THEMES.map(([k, label, ic]) => `<button type="button" role="menuitemradio" data-theme-mode="${k}" aria-checked="false">${R.icon[ic]}<span class="label">${label}</span>${R.icon.check.replace('class="i"', 'class="i tick"')}</button>`).join("")}
        <div class="walls" id="walls">
          <h3>Wallpaper</h3>
          <div class="wall-row" role="group" aria-label="Wallpaper">${WALLPAPERS.map((w) => `<button type="button" class="wall" role="menuitemradio" data-wallpaper="${w.key}" aria-checked="false" aria-label="${w.name}" title="${w.name}"><img src="${base}wallpapers/${w.key}-thumb.jpg" alt="" width="320" height="198" decoding="async"></button>`).join("")}</div>
          <p class="credit" id="wall-credit"></p>
        </div>
      </div>
    </div>`;

const STYLES = ["kit/hologram-warm.css", "kit/hologram-gap-tokens.css", "tokens.css", "styles.css"];

const page = ({ title, description, body, search = false, model = "" }) => `<!doctype html>
<html lang="en" class="dark" data-theme="dark" data-wallpaper="alps" data-base="${base}"${model ? ` data-model="${R.esc(model)}"` : ""}>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${R.esc(title)}</title>
<meta name="description" content="${R.esc(description)}">
<meta property="og:title" content="${R.esc(title)}">
<meta property="og:description" content="${R.esc(description)}">
<meta name="color-scheme" content="dark light">
<script>${prepaint}</script>
<script type="application/json" id="wallpapers">${JSON.stringify(WALLPAPERS)}</script>
<link rel="icon" href="${base}logos/Hologram_Logomark_White.svg" type="image/svg+xml">
<link rel="preload" href="${base}fonts/Geist-Regular.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="${base}fonts/GeistMono-Regular.woff2" as="font" type="font/woff2" crossorigin>
${STYLES.map((s) => `<link rel="stylesheet" href="${base}${s}">`).join("\n")}
<script type="module" src="${base}app.js"></script>
</head>
<body>
<div class="veil" aria-hidden="true"></div>
<div class="shell">
<header class="top">
  <a class="brand" href="${base}" aria-label="Hologram Models Hub"><img class="mark on-dark" src="${base}logos/Hologram_Logomark_White.svg" alt="" width="32" height="32"><img class="word on-dark" src="${base}logos/Hologram_Wordmark_White.svg" alt="Hologram" width="172" height="16"><img class="mark on-light" src="${base}logos/Hologram_Logomark_Black.svg" alt="" width="32" height="32"><img class="word on-light" src="${base}logos/Hologram_Wordmark_Black.svg" alt="Hologram" width="172" height="16"><span class="hub">Models Hub</span></a>
  <div class="top-end">
    ${search ? `<form class="field compact top-search" action="${base}" role="search">${R.icon.search}<input type="search" name="q" placeholder="Search models" aria-label="Search models" autocomplete="off"></form>` : ""}
    ${indexPill()}
    <a class="github" href="${REPO}" aria-label="GitHub" title="GitHub">${R.icon.github}</a>
    ${themeSwitch}
  </div>
</header>
${archive ? `<div class="archive-banner" id="archive-banner" role="status" hidden>${R.icon.calendar}<span>Viewing the index of <b id="archive-banner-date"></b>. Every file shown was checked against its address.</span><button type="button" class="link" data-at="latest">Back to latest</button></div>` : ""}
${body}
</div>
</body>
</html>
`;

// ---- browse
const initial = R.parseState("");
const r = R.query(models, initial);
const sortMenu = R.SORTS.map(([k, label]) => `<li role="option" data-sort="${k}" aria-selected="${k === initial.sort}">${label}${R.icon.check}</li>`).join("");
const browse = page({
  title: R.title(initial),
  description: `The ${models.length} trending models on Hugging Face, every file named by its bytes.`,
  body: `<main class="browse" id="browse">
  <aside class="panel filters" aria-label="Filters">
    <button type="button" class="control square close-filters" id="close-filters" aria-label="Close filters">${R.icon.close}</button>
    <div id="filters-body">${R.filters(r, initial)}</div>
    <div class="sheet-footer"><button type="button" class="button primary" id="sheet-done">Show <span id="sheet-count">${r.results.length}</span> models</button></div>
  </aside>
  <section class="panel" id="results" aria-label="Models">
    <div class="results-head">
    <div class="head"><h1>Models</h1><span class="pill" id="total">${r.results.length}</span></div>
    <div class="bar">
      <label class="field search">${R.icon.search}<input id="q" type="search" placeholder="Search models" autocomplete="off" spellcheck="false" aria-label="Search models"></label>
      <button type="button" class="open-filters" id="open-filters">${R.icon.sliders}Filters</button>
      <button type="button" class="switch" id="verified-only" role="switch" aria-checked="false"><span class="track" aria-hidden="true"><span class="thumb"></span></span>Verified only</button>
      <div class="sort">
        <button type="button" id="sort" aria-haspopup="listbox" aria-expanded="false"><span id="sort-label">Trending</span>${R.icon.chevron}</button>
        <ul role="listbox" id="sort-list" aria-label="Sort" hidden>${sortMenu}</ul>
      </div>
    </div>
    </div>
    <div class="grid" id="grid">${R.grid(r, { base })}</div>
    <nav class="pager" id="pager" aria-label="Pages">${R.pager(r, initial)}</nav>
  </section>
</main>`,
});

// ---- model pages
// Sources a file can be downloaded from, as table columns. The hub registry holds the daily index only (decision
// 2026-09-18), so it is not a weights source here; data.mjs still records it if a model ever appears there.
const SOURCE_COLUMNS = [["huggingface.co", "Hugging Face"], ["modelscope.cn", "ModelScope"], ["ipfs", "IPFS"], ["bittorrent", "P2P"]];
// The manifest address drawn as braille: 32 bytes, 32 cells, two rows of 16. Lossless: the dots are the bits.
function signature(manifest) {
  const bytes = B.hexToBytes(manifest.split(":")[1]);
  return `<div class="signature" title="${R.esc(manifest)}">
    <span class="label">Address</span>
    <span class="bx glyph" id="glyph" aria-hidden="true"><span>${B.cells(bytes.slice(0, 16))}</span><span>${B.cells(bytes.slice(16))}</span></span>
  </div>`;
}

// Where the identical bytes live. One line per source; Verify checks every one of them.
function sourceList(sources) {
  return `<div class="sources">
    <span class="label">${sources.length > 1 ? "Identical bytes on" : "Available from"}</span>
    <ul>${sources.map((s) => `<li data-source="${R.esc(s.kind)}"${s.p2p ? ' title="Peer to peer via BitTorrent. Your torrent client checks every piece; Hugging Face seeds it, so it completes with zero peers."' : s.pull ? ` title="Stored on Hologram. hologram pull ${R.esc(s.pull)} verifies every chunk as it arrives."` : ""}><span class="state">${s.p2p ? R.icon.nodes : R.icon.seal}${B.loader("orbit")}${R.icon.check}${R.icon.close}</span><a href="${R.esc(s.page)}"${s.p2p ? " download" : ' target="_blank" rel="noopener"'}>${R.esc(s.name)}${s.p2p ? R.icon.down : R.icon.external}</a></li>`).join("")}</ul>
  </div>`;
}

// The file every source is asked for during Verify: small (ModelScope omits CORS on mid-size non-CDN files),
// not weights, present on all of them.
function probe(files) {
  const srcs = files.sources || [];
  return files.files
    .filter(([path, size, , weights]) => !weights && size && size < 256e3 && srcs.every((s) => !s.missing.includes(path)))
    .sort((a, b) => a[1] - b[1]).pop()?.[0];
}

function modelPage(m, files, ov, readme) {
  const fact = (label, value) => (value ? `<div><dt>${label}</dt><dd>${value}</dd></div>` : "");
  const copy = (text, shown) => `<button type="button" class="copy" data-copy="${R.esc(text)}" aria-label="Copy ${R.esc(text)}">${R.esc(shown)}${R.icon.copy}</button>`;
  // Identity and trust only; everything descriptive lives in the Overview tab.
  const facts = [
    fact("Status", `<span class="${m.state === "addressed" ? "ok" : m.state === "skipped" ? "bad" : "dim"}">${R.STATE_LABEL[m.state]}</span>`),
    fact("Trending", `#${m.rank}`),
    fact("Downloads, 30 days", R.count(m.downloads)),
    m.weightBytes ? fact("Weights", R.bytes(m.weightBytes)) : "",
    files?.sources?.length ? fact("Sources", String(files.sources.length)) : "",
    m.revision ? fact("Revision", copy(m.revision, m.revision.slice(0, 12))) : "",
    m.manifest ? fact("Manifest", copy(m.manifest, R.shortAddress(m.manifest))) : "",
  ].join("");

  let filesPanel, downloadMenu = "";
  if (files) {
    const srcs = files.sources || [];
    const byKind = Object.fromEntries(srcs.map((s) => [s.kind, s]));
    const encodePath = (path) => path.split("/").map(encodeURIComponent).join("/");
    // One column per source. Green: this file is available there (a download, checked against its address).
    // Red: not available there.
    const cell = ([kind, name], path, size, address, hfUrl) => {
      const s = byKind[kind];
      if (s?.pull) {
        const command = `hologram pull ${s.pull}`;
        return `<td class="dl"><button type="button" class="dl-yes" data-copy="${R.esc(command)}" title="Stored on Hologram. Copy the pull command: every chunk is verified as it arrives" aria-label="Copy hologram pull command for ${R.esc(path)}">${R.icon.down}</button></td>`;
      }
      if (!s || s.missing.includes(path)) {
        return `<td class="dl"><span class="dl-no" role="img" aria-label="Not available on ${name}" title="Not available on ${name}">${R.icon.close}</span></td>`;
      }
      if (s.p2p) {
        return `<td class="dl"><a class="dl-yes" href="${R.esc(s.page)}" title="Peer to peer: a BitTorrent file with every file. Pick ${R.esc(path)} in your torrent client" aria-label="Peer to peer torrent for ${R.esc(path)}">${R.icon.down}</a></td>`;
      }
      const href = kind === "huggingface.co" ? hfUrl : s.resolve + encodePath(path);
      return `<td class="dl"><a class="dl-yes" href="${R.esc(href)}" data-download data-source="${name}" title="Download ${R.esc(path)} from ${name}, checked against its address" aria-label="Download ${R.esc(path)} from ${name}">${R.icon.down}</a></td>`;
    };
    const total = files.files.reduce((sum, f) => sum + (f[1] || 0), 0);
    const rows = files.files.map(([path, size, address, , hfUrl]) => `<tr data-path="${R.esc(path)}" data-size="${size ?? 0}" data-address="${R.esc(address)}"><td class="path" title="${R.esc(path)}">${R.esc(path)}</td><td class="size">${R.bytes(size)}</td><td class="addr">${copy(address, R.shortAddress(address))}</td>${SOURCE_COLUMNS.map((c) => cell(c, path, size, address, hfUrl)).join("")}</tr>`).join("\n");
    // One button above each source column: every file this source has, as one zip, each file checked against its
    // address as it streams. P2P is the torrent; Hologram copies the pull command.
    const head = ([kind, name]) => {
      const s = byKind[kind];
      const count = s ? files.files.filter(([path]) => !s.missing.includes(path)).length : 0;
      const label = `<span class="name">${name}</span>`;
      if (!s) return `<th class="dl" data-source="${R.esc(kind)}" data-state="off"><span class="dl-all off" aria-hidden="true">${R.icon.down}</span>${label}</th>`;
      if (s.pull) return `<th class="dl" data-source="${R.esc(kind)}"><button type="button" class="dl-yes dl-all" data-copy="hologram pull ${R.esc(s.pull)}" title="Copy the hologram pull command: every file, verified as it arrives" aria-label="Copy hologram pull command">${R.icon.down}</button>${label}</th>`;
      if (s.p2p) return `<th class="dl" data-source="${R.esc(kind)}"><a class="dl-yes dl-all" href="${R.esc(s.page)}" title="Torrent with every file. Your client checks every piece" aria-label="Download torrent">${R.icon.down}</a>${label}</th>`;
      return `<th class="dl" data-source="${R.esc(kind)}"><button type="button" class="dl-yes dl-all" data-zip="${name}" title="Download ${count} of ${files.files.length} files from ${name} as one zip, each checked against its address" aria-label="Download all files from ${name} as one zip">${R.icon.down}</button>${label}</th>`;
    };
    // The Download menu: every source in column order, always; its availability (probed when the menu opens, see
    // app.js); one action. A source the model is not on keeps its row, disabled, so absence is visible, not silent.
    const n = files.files.length;
    const row = (kind, name, facts, act, tag, attrs) => `<${tag} role="menuitem" class="src" data-source="${R.esc(kind)}" ${attrs}><span class="state" aria-hidden="true">${B.loader("orbit")}</span><span class="label">${name}<span class="sub">${facts}</span></span><span class="act">${act}</span></${tag}>`;
    const item = ([kind, name]) => {
      const s = byKind[kind];
      if (!s) {
        const why = kind === "ipfs" ? "not pinned yet" : kind === "bittorrent" ? "no torrent yet" : "not on this source";
        return row(kind, name, why, kind === "bittorrent" ? "Get torrent" : "Download zip", "button", `type="button" disabled data-state="off" title="This model is not published on ${name}"`);
      }
      if (s.p2p) return row(kind, name, `torrent, ${n} files`, "Get torrent", "a", `href="${R.esc(s.page)}" download title="A BitTorrent file with every file, seeded by Hugging Face. Your client checks every piece"`);
      const have = files.files.filter(([path]) => !s.missing.includes(path));
      const size = have.reduce((sum, f) => sum + (f[1] || 0), 0);
      return row(kind, name, `${have.length === n ? n : `${have.length} of ${n}`} files, ${R.bytes(size)}`, "Download zip", "button", `type="button" data-zip="${name}" title="Every file ${name} has, as one zip, each checked against its address"`);
    };
    downloadMenu = `<div class="download-all">
        <button type="button" class="button" id="dl-all" aria-haspopup="menu" aria-expanded="false" aria-controls="dl-menu">${R.icon.down}<span>Download</span></button>
        <div class="menu" id="dl-menu" role="menu" aria-label="Download" hidden>
          <p class="menu-note">Choose where to download from. Every file is checked against its address as it arrives.</p>
          ${SOURCE_COLUMNS.map(item).join("")}
        </div>
      </div>`;
    filesPanel = `<div class="section-head files-head" data-name="${R.esc(m.name)}" data-repo="${R.esc(m.id)}" data-revision="${R.esc(files.revision)}">
      <p class="note">${files.files.length} files, ${R.bytes(total)}. Every download is checked against its address.</p>
    </div>
    <p class="progress" id="dl-progress" role="status" hidden></p>
    <div class="scroll"><table id="files">
      <thead><tr><th><button type="button" data-col="path" aria-sort="ascending">Path${R.icon.chevron}</button></th><th class="size"><button type="button" data-col="size">Size${R.icon.chevron}</button></th><th>Address</th>${SOURCE_COLUMNS.map(head).join("")}</tr></thead>
      <tbody>${rows}</tbody>
    </table></div>`;
  } else {
    const note = m.state === "skipped"
      ? "This model is gated on Hugging Face. Addresses are recorded for public models only."
      : "This model is queued. Every file receives its address on the next daily index.";
    filesPanel = `<p class="note">${note}</p>`;
  }

  return page({
    model: m.id,
    title: `${m.name} · Hologram Models Hub`,
    description: metaDescription(ov) || `${m.id}: every file of this model with the address that proves its bytes.`,
    search: true,
    body: `<a class="back" href="${base}">${R.icon.left}Models</a>
<section class="panel">
  <div class="hero">
    ${R.avatar(m, base)}
    <div class="who">
      <p class="org">${R.esc(m.org)}</p>
      <h1>${R.esc(m.name)}</h1>
      <div class="tags">${R.tags(m, { full: true })}</div>
    </div>
    <div class="actions">
      ${m.manifest && files
        ? `<button type="button" class="button primary" data-verify="${R.esc(m.id)}" data-manifest="${R.esc(m.manifest)}" data-probe="${R.esc(probe(files) || "")}">${R.icon.check}${B.loader("orbit")}<span>Verify</span></button>${downloadMenu}`
        : `<a class="button" href="https://huggingface.co/${R.esc(m.id)}" target="_blank" rel="noopener">Hugging Face${R.icon.external}</a>`}
    </div>
  </div>
  ${m.manifest && files ? `<div class="provenance">${signature(m.manifest)}${sourceList(files.sources || [])}</div><script type="application/json" id="sources">${JSON.stringify((files.sources || []).map(({ kind, name, resolve, p2p, pull, page }) => ({ kind, name, resolve, p2p, pull, page: p2p ? page : undefined })))}</script>` : ""}
  <p class="verdict" id="verdict" role="status" hidden></p>
  <p class="verdict" id="dl-status" role="status" hidden></p>
</section>
<main class="detail">
  <section class="panel"><dl class="facts">${facts}</dl></section>
  <section class="panel">
    <div class="panel-tabs" role="tablist" aria-label="Model">
      <button type="button" class="tab" role="tab" id="tab-overview" aria-controls="pane-overview" aria-selected="true">Overview</button>
      <button type="button" class="tab" role="tab" id="tab-files" aria-controls="pane-files" aria-selected="false" tabindex="-1">Files${files ? `<span class="pill">${files.files.length}</span>` : ""}</button>
    </div>
    <div class="pane" id="pane-overview" role="tabpanel" aria-labelledby="tab-overview">${overview(m, ov, readme)}</div>
    <div class="pane" id="pane-files" role="tabpanel" aria-labelledby="tab-files" hidden>${filesPanel}</div>
  </section>
</main>`,
  });
}

await rm(DIST, { recursive: true, force: true });
await mkdir(join(DIST, "data"), { recursive: true });
await writeFile(join(DIST, "index.html"), browse);
await writeFile(join(DIST, "404.html"), page({
  title: "Not found · Hologram Models Hub",
  description: "Page not found.",
  search: true,
  body: `<section class="panel browse"><div class="empty"><p>This page does not exist.</p><a class="link" href="${base}">All models</a></div></section>`,
}));

for (const m of models) {
  const filesPath = join(SITE, "data", "files", m.org, `${m.name}.json`);
  const files = existsSync(filesPath) ? JSON.parse(await readFile(filesPath, "utf8")) : null;
  const ovPath = join(SITE, "data", "overview", m.org, `${m.name}.json`), mdPath = join(SITE, "data", "overview", m.org, `${m.name}.md`);
  const ov = existsSync(ovPath) ? JSON.parse(await readFile(ovPath, "utf8")) : null;
  const readme = existsSync(mdPath) ? await readFile(mdPath, "utf8") : null;
  const dir = join(DIST, "models", m.org, m.name);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, "index.html"), modelPage(m, files, ov, readme));
}

const slim = models.map(({ stateLabel, task, recency, isNew, ...m }) => m);
await writeFile(join(DIST, "data", "models.json"), JSON.stringify({ snapshot: data.snapshot, models: slim }));
for (const f of ["app.js", "render.mjs", "braille.mjs", "zip.mjs", "styles.css", "tokens.css"]) await cp(join(SITE, "src", f), join(DIST, f));
await mkdir(join(DIST, "kit"), { recursive: true });
for (const f of ["hologram-warm.css", "hologram-gap-tokens.css"]) await cp(join(KIT, f), join(DIST, "kit", f));
await cp(join(KIT, "fonts"), join(DIST, "fonts"), { recursive: true });
await cp(join(KIT, "logos"), join(DIST, "logos"), { recursive: true });
if (existsSync(join(SITE, "public"))) await cp(join(SITE, "public"), DIST, { recursive: true });
if (archive) {
  // Machine access: the ledger, and one tiny stub per day so a script resolves a date with one request.
  await cp(archivePath, join(DIST, "archive.json"));
  await mkdir(join(DIST, "at"), { recursive: true });
  for (const d of archive.days) await writeFile(join(DIST, "at", `${d.date}.json`), JSON.stringify({ date: d.date, cid: d.cid, index: d.index, gateway: archive.gateway, mirror: archive.mirror && d === archive.days[archive.days.length - 1] ? `${archive.mirror}${d.date}/` : null, registry: archive.registry && d === archive.days[archive.days.length - 1] ? `${archive.registry}:${d.date}` : null }));
}
await writeFile(join(DIST, ".nojekyll"), "");

console.log(`built ${models.length} model pages + browse at base ${base} → ${DIST}`);
