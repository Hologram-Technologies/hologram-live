// Static build: web/data + web/src + vendored brand kit → web/dist.
//
//   node build.mjs                 base / (set BASE=/path/ when served under a path;
//                                  Git Bash: prefix MSYS_NO_PATHCONV=1)

import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { checkSealed } from "./qa/registry-sealed.mjs";
import { bucketsOf, checkBucketLinks } from "./qa/buckets-links.mjs";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as R from "./src/render.mjs";
import * as B from "./src/braille.mjs";
import { overview, metaDescription } from "./src/overview.mjs";
import { ORIGIN } from "./src/origin.mjs";
import { landing } from "./src/landing.mjs";
import * as D from "./src/docs.mjs";

const SITE = dirname(fileURLToPath(import.meta.url));
const DIST = join(SITE, "dist");
const KIT = join(SITE, "vendor", "hologram-brand-kit");
const base = process.env.BASE || "/";
// Which treatment the highlighted word gets. One build carries them all; this picks the one that ships.
const HIGHLIGHT = process.env.HIGHLIGHT || "seal";
// The root is the landing page, so the browse table moves under the prefix its model pages already use.
const BROWSE = `${base}models/`;
// The repository the hero's star badge counts and links to. scripts/data.mjs bakes the count into
// models.json; a build whose fetch failed, or an offline build off a copied data/, ships the badge without a
// number and the page fills it in from the public API on load.
const STAR_REPO = process.env.MODEL_HUB_REPO || "Hologram-Technologies/hologram-live";
const INDEX = "https://github.com/humuhumu33/hologram-api";
// The one base URL every dialect answers on, whatever prefix this build is served under.
const ENDPOINT = ORIGIN;

const data = JSON.parse(await readFile(join(SITE, "data", "models.json"), "utf8"));
const models = R.prepare(data.models, data.snapshot);
const archivePath = join(SITE, "data", "archive.json");
const archive = existsSync(archivePath) ? JSON.parse(await readFile(archivePath, "utf8")) : null;
const starRepo = { name: STAR_REPO, url: `https://github.com/${STAR_REPO}`, stars: data.repo?.name === STAR_REPO ? data.repo.stars : null };

// The index control: with an archive it opens every captured day; the Wayback idea, one control.
//
// It is no longer in the lead row — that row carries the section's own address now, as the Registry and
// Spaces rows do. The control stays in the page, hidden: a link to ?at=<day> must still open that day, and
// the banner above the results is what says which day you are reading. Hidden, not removed.
function indexPill() {
  const latest = `Index ${R.day(data.snapshot)}`;
  if (!archive) return `<a class="status endpoint" href="${INDEX}" title="Addresses refresh daily">${latest}</a>`;
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
      <button type="button" class="status endpoint" id="archive-button" aria-haspopup="menu" aria-expanded="false" aria-controls="archive-menu" title="Every day's index is stored on IPFS. Open any day."><span id="archive-label">${latest}</span>${R.icon.chevron}</button>
      <div class="menu" id="archive-menu" role="menu" aria-label="Index history" hidden>
        <button type="button" role="menuitemradio" data-at="latest" aria-checked="true">${R.icon.check.replace('class="i"', 'class="i latest"')}<span class="label">Latest<span class="sub">${R.day(data.snapshot)}, ${models.length} models</span></span>${R.icon.check.replace('class="i"', 'class="i tick"')}</button>
        <div class="archive-days">${groups.slice(0, 3).join("")}${older}</div>
        <p class="menu-note">Each day's index is saved on IPFS and verified in your browser. Today opens at once; older days can take a minute the first time.</p>
        <div class="archive-foot"><button type="button" class="copy" id="archive-cid" data-copy="" title="Copy this day's IPFS address">CID${R.icon.copy}</button><button type="button" class="copy" id="archive-pull" data-copy="" title="Copy the hologram pull command for the current index">hologram pull${R.icon.copy}</button></div>
      </div>
    </div>
    <script type="application/json" id="archive-days">${JSON.stringify({ gateway: archive.gateway, mirror: archive.mirror || null, registry: archive.registry || null, latest: data.snapshot, days: days.map(({ date, cid, index, models: n }) => ({ date, cid, index, models: n })) })}</script>`;
}

const WALLPAPERS = [
  { key: "alps", name: "Alpine Dawn" },
  { key: "galaxy", name: "Galaxy" },
  { key: "aurora", name: "Aurora" },
];
const THEMES = [["dark", "Dark", "moon"], ["light", "Light", "sun"], ["immersive", "Immersive", "image"]];

// ---- sign-in
//
// Two public ids and the name of the vendored bundle. With no app id the control is not built at all, so a build
// without sign-in configured produces exactly the site that was there before it existed.
const privyStamp = existsSync(join(SITE, "vendor", "privy", "VENDORED.json"))
  ? JSON.parse(await readFile(join(SITE, "vendor", "privy", "VENDORED.json"), "utf8"))
  : null;
const privy = process.env.PRIVY_APP_ID && privyStamp
  ? { appId: process.env.PRIVY_APP_ID, clientId: process.env.PRIVY_CLIENT_ID || "", bundle: `vendor/privy/${privyStamp.file}` }
  : null;

// Signed out, this is one button and nothing else. The menu it becomes is built by the browser once somebody is
// actually signed in, so nobody downloads an account they do not have.
const accountControl = () => privy ? `<div class="account" id="account">
      <button type="button" class="sign-in" id="sign-in-button" title="Sign in">${R.icon.user}<span class="label">Sign in</span></button>
      <button type="button" class="account-mark" id="account-button" aria-haspopup="menu" aria-expanded="false" aria-controls="account-menu" aria-label="Your account"><span id="account-initial" aria-hidden="true"></span></button>
      <div class="menu" id="account-menu" role="menu" aria-label="Your account" hidden></div>
    </div>` : "";

// ---- the top-level menu
//
// One list, one renderer, one place to add a section. Every page this build writes carries it, and the
// Registry page — which ships as its own finished file from public/ — has the same markup put into it at
// the end of this build, so no page of the site can be left holding a different menu.
const SECTIONS = [
  ["models", "Models", "grid", BROWSE],
  ["registry", "Registry", "box", `${base}registry/`],
  ["spaces", "Spaces", "cpu", `${base}spaces/`],
  ["buckets", "Buckets", "bucket", `${base}buckets/`],
  ["docs", "Docs", "file", `${base}docs/`],
];
// `current` is the section the page belongs to. It marks that one link aria-current, which the stylesheet
// draws in the brand colour: where you are, said once in the row and once to a screen reader.
const topNav = (current = "") => `<nav class="top-nav" aria-label="Sections">${SECTIONS
  .map(([key, label, mark, href]) => `<a href="${href}"${key === current ? ' aria-current="page"' : ""}>${label}${R.icon[mark]}</a>`)
  .join("")}</nav>`;

// Runs before first paint: Dark for first visits, the saved choice after that. No flash.
const prepaint = `(function(){var s={};try{s=JSON.parse(localStorage.getItem("hologram-models-hub.theme"))||{}}catch(e){}
var m=["dark","light","immersive"].indexOf(s.mode)>=0?s.mode:"dark",w=${JSON.stringify(WALLPAPERS.map((w) => w.key))}.indexOf(s.wallpaper)>=0?s.wallpaper:"alps",r=document.documentElement;
r.setAttribute("data-theme",m);r.setAttribute("data-wallpaper",w);r.classList.toggle("dark",m!=="light");
if(m==="immersive"){var l=document.createElement("link");l.rel="preload";l.as="image";l.href="${base}wallpapers/"+w+".jpg";document.head.appendChild(l)}
// Someone signed in here before: say so now, not after auth.js has loaded and Privy has answered. Otherwise
// every page they open shows "Sign in" for a moment first.
var a=null;try{a=JSON.parse(localStorage.getItem("hologram-models-hub.account"))}catch(e){}
if(a){r.setAttribute("data-account","in");if(a.i)r.style.setProperty("--hh-account-initial",JSON.stringify(a.i))}})();`;

const themeSwitch = `<div class="appearance">
      <button type="button" id="theme-button" aria-haspopup="menu" aria-expanded="false" aria-controls="theme-menu" aria-label="Theme" title="Theme">${THEMES.map(([k, , ic]) => R.icon[ic].replace('class="i"', `class="i" data-for="${k}"`)).join("")}</button>
      <div class="menu" id="theme-menu" role="menu" aria-label="Theme" hidden>
        ${THEMES.map(([k, label, ic]) => `<button type="button" role="menuitemradio" data-theme-mode="${k}" aria-checked="false">${R.icon[ic]}<span class="label">${label}</span>${R.icon.check.replace('class="i"', 'class="i tick"')}</button>`).join("")}
        <div class="walls" id="walls">
          <h3>Wallpaper</h3>
          <div class="wall-row" role="group" aria-label="Wallpaper">${WALLPAPERS.map((w) => `<button type="button" class="wall" role="menuitemradio" data-wallpaper="${w.key}" aria-checked="false" aria-label="${w.name}" title="${w.name}"><img src="${base}wallpapers/${w.key}-thumb.jpg" alt="" width="320" height="198" decoding="async"></button>`).join("")}</div>
        </div>
      </div>
    </div>`;

const STYLES = ["kit/hologram-warm.css", "kit/hologram-gap-tokens.css", "tokens.css", "chrome.css", "styles.css"];

// The header, one definition for the whole site, and the same row on every page: the brand lockup is the
// wordmark alone, because the menu beside it is what says where you are, and nothing here changes between the
// landing and the pages behind it. The repository is reached from the hero's star badge, not from this row.
//
// Everything after the brand lives in one group, .top-menu. Wide, the group is laid out as the row it always
// was (display: contents). Narrow, the same group becomes a sheet under the header, and one control stands in
// for it at the end of the row: the menu button. Same markup, same ids, same controls; only the layout folds.
const header = ({ section = "", search = false } = {}) => `<header class="top">
  <a class="brand" href="${base}" aria-label="Hologram Models Hub"><img class="mark on-dark" src="${base}logos/Hologram_Logomark_White.svg" alt="" width="32" height="32"><img class="word on-dark" src="${base}logos/Hologram_Wordmark_White.svg" alt="Hologram" width="172" height="16"><img class="mark on-light" src="${base}logos/Hologram_Logomark_Black.svg" alt="" width="32" height="32"><img class="word on-light" src="${base}logos/Hologram_Wordmark_Black.svg" alt="Hologram" width="172" height="16"></a>
  <div class="top-end">
    <div class="top-menu" id="top-menu">
      ${topNav(section)}
      ${search ? `<form class="field compact top-search" action="${BROWSE}" role="search">${R.icon.search}<input type="search" name="q" placeholder="Search models" aria-label="Search models" autocomplete="off"></form>` : ""}
      <div class="top-tools">
        ${themeSwitch}
        ${accountControl()}
      </div>
    </div>
    <button type="button" class="menu-button" id="menu-button" aria-expanded="false" aria-controls="top-menu" aria-label="Menu" title="Menu">${R.icon.menu}${R.icon.close}</button>
  </div>
</header>`;

// Everything the header needs in <head>, for a page that is not written by page(): the no-flash theme
// script, the data the theme menu reads, the kit and chrome stylesheets, and the module that wires the two
// controls up. The Registry page's own stylesheet loads after these and keeps its own component rules.
const chromeHead = [
  `<script>${prepaint}</script>`,
  `<script type="application/json" id="wallpapers">${JSON.stringify(WALLPAPERS)}</script>`,
  ...(privy ? [`<script type="application/json" id="privy">${JSON.stringify(privy)}</script>`] : []),
  `<link rel="preload" href="${base}fonts/Geist-Regular.woff2" as="font" type="font/woff2" crossorigin>`,
  `<link rel="preload" href="${base}fonts/GeistMono-Regular.woff2" as="font" type="font/woff2" crossorigin>`,
  ...["kit/hologram-warm.css", "kit/hologram-gap-tokens.css", "tokens.css", "chrome.css"].map((f) => `<link rel="stylesheet" href="${base}${f}">`),
  `<script type="module">import { mountChrome } from "${base}chrome.js"; mountChrome();</script>`,
].join("\n");

const page = ({ title, description, body, search = false, model = "", home = false, section = "", styles = [] }) => `<!doctype html>
<html lang="en" class="dark" data-theme="dark" data-wallpaper="alps" data-base="${base}"${home ? ` data-page="landing" data-highlight="${HIGHLIGHT}"` : ""}${model ? ` data-model="${R.esc(model)}"` : ""}>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${R.esc(title)}</title>
<meta name="description" content="${R.esc(description)}">
<meta property="og:title" content="${R.esc(title)}">
<meta property="og:description" content="${R.esc(description)}">
<meta name="color-scheme" content="dark light">
<script>${prepaint}</script>
${home ? `<script>if(location.search)location.replace(${JSON.stringify(BROWSE)}+location.search);</script>\n` : ""}<script type="application/json" id="wallpapers">${JSON.stringify(WALLPAPERS)}</script>
${privy ? `<script type="application/json" id="privy">${JSON.stringify(privy)}</script>` : ""}
<link rel="service-desc" type="application/openapi+json" href="${base}openapi.json">
<link rel="service-doc" type="text/markdown" href="${base}agent.md">
<link rel="llms-txt" href="${base}llms.txt">
<link rel="alternate" type="application/json" href="${base}.well-known/model-hub.json">
<script type="application/ld+json">${JSON.stringify({
  "@context": "https://schema.org", "@type": "WebAPI", name: "Hologram Model Hub", url: ENDPOINT,
  description: "One endpoint for open models: find one, fetch it from a source that is up, and prove the bytes.",
  documentation: `${ENDPOINT}/agent.md`, provider: { "@type": "Organization", name: "UOR Foundation", url: "https://uor.foundation" },
})}</script>
<link rel="icon" href="${base}logos/Hologram_Logomark_White.svg" type="image/svg+xml">
<link rel="preload" href="${base}fonts/Geist-Regular.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="${base}fonts/GeistMono-Regular.woff2" as="font" type="font/woff2" crossorigin>
${[...STYLES, ...styles].map((s) => `<link rel="stylesheet" href="${base}${s}">`).join("\n")}
<script type="module" src="${base}app.js"></script>
</head>
<body>
${home ? "" : `<div class="veil" aria-hidden="true"></div>\n`}<div class="shell">
${header({ section, search })}
${archive ? `<div class="archive-banner" id="archive-banner" role="status" hidden>${R.icon.calendar}<span>Viewing the index of <b id="archive-banner-date"></b>. Every file shown was checked against its address.</span><button type="button" class="link" data-at="latest">Back to latest</button></div>` : ""}
${body}
</div>
</body>
</html>
`;

// ---- landing: the front door. One hero, one screen, no scroll.
const home = page({
  title: "Hologram Models Hub",
  description: "Discover, use and share self-verifying models, skills and artifacts.",
  home: true,
  body: landing({ base, models, endpoint: ENDPOINT, repo: starRepo }),
});

// ---- browse
const initial = R.parseState("");
const r = R.query(models, initial);
const sortMenu = R.SORTS.map(([k, label]) => `<li role="option" data-sort="${k}" aria-selected="${k === initial.sort}">${label}${R.icon.check}</li>`).join("");
const browse = page({
  section: "models",
  title: R.title(initial),
  description: `The ${models.length} trending models on Hugging Face, every file named by its bytes.`,
  // The lead row is the same row every catalogue page opens with (.lead, chrome.css): the section's name, how
  // many of its things are listed, and the address they are read from. The address is the one an agent would
  // call, not the name of the page — /models answers this page to a browser and its brief to everything else,
  // so the row names the route that returns the models themselves. app.js fills it in and asks it whether it
  // is up, which is what the dot says.
  body: `<div class="lead"><h1>Models</h1><span class="pill" id="total">${r.results.length}</span><span class="endpoint"><span class="dot" id="dot"></span><span id="host">…</span></span></div>
${archive ? `<div class="archive-hidden" hidden>${indexPill()}</div>` : ""}
<main class="browse" id="browse">
  <aside class="panel filters" aria-label="Filters">
    <button type="button" class="control square close-filters" id="close-filters" aria-label="Close filters">${R.icon.close}</button>
    <div id="filters-body">${R.filters(r, initial)}</div>
    <div class="sheet-footer"><button type="button" class="button primary" id="sheet-done">Show <span id="sheet-count">${r.results.length}</span> models</button></div>
  </aside>
  <section id="results" aria-label="Models">
    <div class="results-head">
    <div class="bar">
      <label class="field search">${R.icon.search}<input id="q" type="search" placeholder="Search models" autocomplete="off" spellcheck="false" aria-label="Search models"></label>
      <button type="button" class="open-filters" id="open-filters">${R.icon.sliders}Filters</button>
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
    // A model card may name the buckets its checkpoints and data live in (`buckets:` in the
    // card's YAML, HF's own field). Each becomes a link, and the bucket page links back.
    bucketsOf(readme).length
      ? fact("Buckets", bucketsOf(readme).map((b) => `<a href="${base}buckets/#/${R.esc(b)}">${R.esc(b)}</a>`).join(", "))
      : "",
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
      return row(kind, name, `${have.length === n ? n : `${have.length} of ${n}`} files, ${R.bytes(size)}`, "Download zip", "button", `type="button" data-zip="${name}" data-kind="${R.esc(kind)}" title="Every file ${name} has, as one zip, each checked against its address"`);
    };
    downloadMenu = `<div class="download-all">
        <button type="button" class="button success" id="dl-all" aria-haspopup="menu" aria-expanded="false" aria-controls="dl-menu" title="Choose a source; the model arrives as one zip, every file checked against its address">${R.icon.down}<span>Download</span></button>
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
    section: "models",
    model: m.id,
    title: `${m.name} · Hologram Models Hub`,
    description: metaDescription(ov) || `${m.id}: every file of this model with the address that proves its bytes.`,
    search: true,
    body: `<div class="head back-row"><a class="back" href="${BROWSE}">${R.icon.left}Models</a>${indexPill()}</div>
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
await mkdir(join(DIST, "models"), { recursive: true });
await writeFile(join(DIST, "index.html"), home);
await writeFile(join(DIST, "models", "index.html"), browse);
await writeFile(join(DIST, "404.html"), page({
  title: "Not found · Hologram Models Hub",
  description: "Page not found.",
  search: true,
  body: `<section class="panel browse"><div class="empty"><p>This page does not exist.</p><a class="link" href="${BROWSE}">All models</a></div></section>`,
}));

const bucketLinks = {};   // "owner/name" -> [model id, …], written for the Buckets page
for (const m of models) {
  const filesPath = join(SITE, "data", "files", m.org, `${m.name}.json`);
  const files = existsSync(filesPath) ? JSON.parse(await readFile(filesPath, "utf8")) : null;
  // The same file list, published: the download worker reads it, and so can any agent.
  if (files) { await mkdir(join(DIST, "data", "files", m.org), { recursive: true }); await cp(filesPath, join(DIST, "data", "files", m.org, `${m.name}.json`)); }
  const ovPath = join(SITE, "data", "overview", m.org, `${m.name}.json`), mdPath = join(SITE, "data", "overview", m.org, `${m.name}.md`);
  const ov = existsSync(ovPath) ? JSON.parse(await readFile(ovPath, "utf8")) : null;
  const readme = existsSync(mdPath) ? await readFile(mdPath, "utf8") : null;
  const dir = join(DIST, "models", m.org, m.name);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, "index.html"), modelPage(m, files, ov, readme));
  for (const b of bucketsOf(readme)) (bucketLinks[b] ??= []).push(m.id);
}

// `task` (Hugging Face's pipeline tag) stays in the published catalog: the endpoint's list route filters on it.
const slim = models.map(({ stateLabel, recency, isNew, ...m }) => m);
await writeFile(join(DIST, "data", "models.json"), JSON.stringify({ snapshot: data.snapshot, models: slim }));
for (const f of ["app.js", "chrome.js", "render.mjs", "braille.mjs", "zip.mjs", "chrome.css", "styles.css", "tokens.css", "docs.css"]) await cp(join(SITE, "src", f), join(DIST, f));

// ---- the documentation
//
// web/docs/*.md, one file per page, becomes /docs/<slug>/ for a reader, /docs/<slug>.md for an agent, and one line
// each in /llms.txt, the index every agent fetches first. The API reference page is generated from the same
// OpenAPI document the endpoint is held to, so it cannot name a route that is not described. The build refuses a
// page that links to nothing or has nothing to run (D.check, after public/ is copied in).
const spec = JSON.parse(await readFile(join(SITE, "public", "openapi.json"), "utf8"));
const docPages = D.render(await D.load(join(SITE, "docs")), { base, spec });
for (const p of docPages) {
  await mkdir(dirname(join(DIST, p.path)), { recursive: true });
  await writeFile(join(DIST, p.path), page({
    title: p.slug === "index" ? "Docs · Hologram Models Hub" : `${p.title} · Docs · Hologram Models Hub`,
    description: p.description,
    section: "docs",
    styles: ["docs.css"],
    body: `<main class="docs">${D.sidebar(docPages, p.slug, base)}${D.article(p, docPages, base)}</main>`,
  }));
  await writeFile(join(DIST, "docs", `${p.slug}.md`), D.twin(p, { endpoint: ENDPOINT }));
}
await writeFile(join(DIST, "llms.txt"), D.llms(docPages, { endpoint: ENDPOINT, spec, snapshot: data.snapshot, models: models.length }));
if (privy) {
  await cp(join(SITE, "src", "auth.js"), join(DIST, "auth.js"));
  await mkdir(join(DIST, "vendor", "privy"), { recursive: true });
  await cp(join(SITE, "vendor", "privy", privyStamp.file), join(DIST, "vendor", "privy", privyStamp.file));
  // Where a provider sends people back. It finishes the sign-in and returns them to the page they left, so the
  // round trip reads as one step rather than as a visit to somewhere else.
  await mkdir(join(DIST, "auth"), { recursive: true });
  await writeFile(join(DIST, "auth", "index.html"), page({
    title: "Signing you in · Hologram Models Hub",
    description: "Finishing sign-in.",
    body: `<section class="panel browse"><div class="empty"><p id="auth-landing">Signing you in…</p><a class="link" href="${base}">All models</a></div></section>`,
  }));
}
await mkdir(join(DIST, "kit"), { recursive: true });
for (const f of ["hologram-warm.css", "hologram-gap-tokens.css"]) await cp(join(KIT, f), join(DIST, "kit", f));
await cp(join(KIT, "fonts"), join(DIST, "fonts"), { recursive: true });
await cp(join(KIT, "logos"), join(DIST, "logos"), { recursive: true });
if (existsSync(join(SITE, "public"))) await cp(join(SITE, "public"), DIST, { recursive: true });
// The download worker is a classic script (module workers are not everywhere yet): ZipWriter first, then the worker.
await writeFile(join(DIST, "zip-sw.js"), `${(await readFile(join(SITE, "src", "zip.mjs"), "utf8")).replace(/^export /gm, "")}\n${await readFile(join(SITE, "src", "zip-sw.js"), "utf8")}`);
await mkdir(join(DIST, "vendor", "hash-wasm"), { recursive: true });
await cp(join(SITE, "vendor", "hash-wasm", "sha256.umd.min.js"), join(DIST, "vendor", "hash-wasm", "sha256.umd.min.js"));
if (archive) {
  // Machine access: the ledger, and one tiny stub per day so a script resolves a date with one request.
  await cp(archivePath, join(DIST, "archive.json"));
  await mkdir(join(DIST, "at"), { recursive: true });
  for (const d of archive.days) await writeFile(join(DIST, "at", `${d.date}.json`), JSON.stringify({ date: d.date, cid: d.cid, index: d.index, gateway: archive.gateway, mirror: archive.mirror && d === archive.days[archive.days.length - 1] ? `${archive.mirror}${d.date}/` : null, registry: archive.registry && d === archive.days[archive.days.length - 1] ? `${archive.registry}:${d.date}` : null }));
}
await writeFile(join(DIST, ".nojekyll"), "");

// ---- the Registry page gets the same header as everything else
//
// It ships as its own finished file rather than through page(), because it carries its own component set,
// its own covers and its own hasher. What it must not carry is its own idea of the header: a second copy
// of that row is a copy that drifts, and a menu that changes shape when you cross into a section is the
// one thing a top-level menu cannot do. So the file leaves two marks and the build fills them, from the
// very same header() and topNav() every other page is built with.
// The Spaces and Buckets pages ship the same way: their own components, the site's header.
for (const section of ["registry", "spaces", "buckets"]) {
  const path = join(DIST, section, "index.html");
  let html = await readFile(path, "utf8");
  for (const mark of ["<!--chrome:head-->", "<!--chrome:header-->"]) {
    if (!html.includes(mark)) throw new Error(`${section}/index.html lost ${mark}: the shared header has nowhere to go`);
  }
  html = html.replace("<!--chrome:head-->", chromeHead).replace("<!--chrome:header-->", header({ section }));
  await writeFile(path, html);
}

// The Registry page ships from public/. It carries its own covers and its own hasher, and this
// refuses to build a copy that would fetch either from somebody else.
await writeFile(join(DIST, "buckets", "links.json"), JSON.stringify(bucketLinks));
console.log(await checkBucketLinks(DIST, models.length));
console.log(await checkSealed(DIST));
// Every link in the docs lands on something this build ships, and every page has something to run.
console.log(D.check(docPages, { exists: (p) => existsSync(join(DIST, p.replace(/^\//, ""))) }));

console.log(`built ${models.length} model pages + browse + ${docPages.length} docs pages at base ${base} → ${DIST}`);
