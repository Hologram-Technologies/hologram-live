// Proves the claim the header makes: every page of this site carries the same top-level menu, and the
// section you are in is marked, in the accent, legibly. And the claim the catalogue pages make below it:
// Models, Registry and Spaces each open with the same lead row — name, count, address — at the same height,
// in the same face, so crossing between them moves nothing.
//
//   node apps/model-hub/web/qa/menu.mjs            (build dist first)
//   CHROME=/path/to/chrome node .../menu.mjs
//
// The menu is built once, by topNav() in build.mjs, but it reaches the Registry page by a different road:
// that page ships as its own finished file and the build puts the header into it. Two roads is exactly the
// arrangement that drifts, so this walks every page shape the site has and compares what it finds.
//
// Serves web/dist itself and drives a headless Chromium over the DevTools protocol with Node's own
// WebSocket, the same way qa/landing-fit.mjs does. With no browser installed it says so and exits 0.

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { readFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(SITE, "dist");

// What the menu is. Written out here rather than imported, so a change to build.mjs has to be made twice
// on purpose instead of once by accident.
const SECTIONS = ["Models", "Registry", "Spaces", "Buckets", "Docs"];

// One page of every shape the site builds, and the section each belongs to. "" means no section is current:
// the landing is the front door, it is not inside any section. The last field says what that page opens with:
// `true` for a catalogue page, which opens with the shared lead row (name · count · address) above its
// columns, and "own" for a page that opens with a row of its own on purpose. Buckets is the one "own": its
// row carries the controls that make and open a bucket, which no other section has. A page marked neither
// must not open with a lead row at all, which is what catches one arriving by accident.
const PAGES = [
  ["landing", "/", ""],
  ["browse", "/models/", "Models", true, true],
  ["model", null, "Models"],           // filled in from dist below: whichever model page is first
  ["registry", "/registry/", "Registry", true, true],
  ["spaces", "/spaces/", "Spaces", true, true],
  ["buckets", "/buckets/", "Buckets", "own", true],
  ["docs", "/docs/", "Docs", false, true],
  ["docs page", "/docs/quickstart/", "Docs"],
  ["not found", "/404.html", ""],
];
// What every catalogue page's lead row is made of, in order, and nothing else. A page that opens with more or
// parts in another order, moves the reader's eye when they cross into it.
const LEAD = "name count address";
// Wide enough for the menu, and narrow enough to have hidden it. Both are checked.
const WIDTHS = [2560, 1600, 1440, 1280, 1101, 1100, 1024, 900, 861, 860, 768, 390, 320];
const THEMES = ["dark", "light"];
// AA for a 14px label. Immersive puts the row on a photograph nobody can rule on, so it is not held here.
const AA = 4.5;

const TYPES = { html: "text/html", js: "text/javascript", mjs: "text/javascript", css: "text/css", json: "application/json", svg: "image/svg+xml", woff2: "font/woff2", jpg: "image/jpeg", png: "image/png", txt: "text/plain", md: "text/markdown" };

function findBrowser() {
  return [
    process.env.CHROME,
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ].find((p) => p && existsSync(p));
}

// `prefix` is the BASE the build was made for: dist always sits at the root of itself, but a prefixed build
// writes that prefix into every link, so the server has to answer under it for the pages to work at all.
function serve(root, prefix = "") {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://localhost");
    const name = prefix && url.pathname.startsWith(prefix) ? url.pathname.slice(prefix.length) : url.pathname;
    let path = normalize(join(root, decodeURIComponent(name)));
    if (!path.startsWith(root)) return res.writeHead(403).end();
    if (path.endsWith("/") || path.endsWith("\\") || (existsSync(path) && !/\.[a-z0-9]+$/i.test(path))) path = join(path, "index.html");
    let body = null;
    try { body = await readFile(path); } catch { return res.writeHead(404).end("not found"); }
    res.writeHead(200, { "content-type": TYPES[path.split(".").pop().toLowerCase()] || "application/octet-stream" }).end(body);
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok({ server, port: server.address().port })));
}

class CDP {
  constructor(ws) { this.ws = ws; this.id = 0; this.waiting = new Map(); ws.onmessage = (e) => { const m = JSON.parse(e.data); const w = this.waiting.get(m.id); if (w) { this.waiting.delete(m.id); m.error ? w.bad(new Error(m.error.message)) : w.ok(m.result); } }; }
  static async open(url) { const ws = new WebSocket(url); await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = () => bad(new Error(`cannot reach ${url}`)); }); return new CDP(ws); }
  send(method, params = {}) { const id = ++this.id; this.ws.send(JSON.stringify({ id, method, params })); return new Promise((ok, bad) => this.waiting.set(id, { ok, bad })); }
  async evaluate(expression) { const r = await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || "evaluate failed"); return r.result.value; }
  close() { this.ws.close(); }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Runs in the page. Reads the row as a reader meets it: the labels in order, which one is marked, what
// colour it is, and what colour is actually behind it once every transparent ancestor is accounted for.
const READ = `(() => {
  const nav = document.querySelector(".top-nav");
  const top = document.querySelector("header.top");
  if (!top) return { header: false };
  if (!nav) return { header: true, nav: false };

  const lum = (c) => { const [r, g, b] = c.map((v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
  // The accent arrives as color-mix(), which a browser reports as color(srgb 0..1) rather than rgb(0..255).
  // Painting it settles every syntax into the same four bytes, so nothing here has to parse CSS colour.
  const ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  const rgba = (css) => {
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = "#000";
    ctx.fillStyle = css;
    ctx.fillRect(0, 0, 1, 1);
    return [...ctx.getImageData(0, 0, 1, 1).data.slice(0, 3), ctx.getImageData(0, 0, 1, 1).data[3] / 255];
  };
  // An opaque backdrop, found by walking up through whatever is see-through.
  const behind = (el) => {
    for (let n = el; n; n = n.parentElement) {
      const bg = rgba(getComputedStyle(n).backgroundColor);
      if (bg[3] >= 0.99) return bg.slice(0, 3);
    }
    return [255, 255, 255];
  };
  const ratio = (fg, bg) => { const a = lum(fg) + 0.05, b = lum(bg) + 0.05; return Math.round((Math.max(a, b) / Math.min(a, b)) * 100) / 100; };

  const links = [...nav.querySelectorAll("a")];
  const current = links.filter((a) => a.hasAttribute("aria-current"));
  const rest = links.find((a) => !a.hasAttribute("aria-current"));
  const shown = getComputedStyle(nav).display !== "none";
  const cur = current[0];
  return {
    header: true, nav: true, shown,
    labels: links.map((a) => a.textContent.trim()),
    hrefs: links.map((a) => new URL(a.href).pathname),
    marks: links.map((a) => a.querySelectorAll("svg").length),
    current: current.map((a) => a.textContent.trim()),
    currentColor: cur ? getComputedStyle(cur).color : null,
    restColor: rest ? getComputedStyle(rest).color : null,
    currentContrast: cur ? ratio(rgba(getComputedStyle(cur).color).slice(0, 3), behind(cur)) : null,
    // The rest of the header cluster, which has to be the same cluster on every page.
    theme: !!document.querySelector("#theme-button"),
    account: !!document.querySelector("#account"),
    headerOverflow: Math.max(0, top.scrollWidth - top.clientWidth),
    pageOverflowX: Math.max(0, document.documentElement.scrollWidth - document.documentElement.clientWidth),
    // The lead row a catalogue page opens with: its name, a count, an address, a line of provenance — in
    // that order, above the columns, on Models, Registry and Spaces alike.
    // The section tag: the same control beside the heading of every section, whatever else that section's
    // opening row carries. What is measured is what a reader compares across pages -- that it is there, that it
    // sits to the right of the title on the same line, and that it is the same object drawn the same way.
    tag: (() => {
      const tag = document.querySelector(".section-tag");
      if (!tag) return null;
      const h1 = tag.closest(".lead, .docs-title, .lead-main")?.querySelector("h1") || document.querySelector("h1");
      const t = tag.getBoundingClientRect(), h = h1?.getBoundingClientRect();
      const cs = getComputedStyle(tag);
      return {
        section: tag.dataset.section,
        text: tag.textContent.trim(),
        rightOfTitle: h ? t.left >= h.right - 1 : null,
        below: h ? t.top >= h.top - 1 : null,
        sameLine: h ? Math.abs((t.top + t.height / 2) - (h.top + h.height / 2)) <= Math.max(6, h.height / 2) : null,
        look: [cs.fontFamily, cs.fontSize, cs.borderRadius, cs.borderWidth, Math.round(t.height)].join(" | "),
        dot: !!tag.querySelector(".dot"),
      };
    })(),
    lead: (() => {
      const lead = document.querySelector(".lead");
      if (!lead) return null;
      const kids = [...lead.children].filter((k) => !k.matches("script")).map((k) => k.matches("h1") ? "name" : k.matches(".pill") ? "count" : k.matches(".endpoint, .archive") ? "address" : k.matches(".prov") ? "provenance" : k.tagName.toLowerCase());
      const top = lead.getBoundingClientRect().top;
      return { name: lead.querySelector("h1")?.textContent.trim() || "", order: kids.join(" "), top: Math.round(top), font: getComputedStyle(lead.querySelector("h1")).fontFamily };
    })(),
  };
})()`;

const browser = findBrowser();
if (!browser) { console.log("menu: no Chromium found (set CHROME=<path>); skipped"); process.exit(0); }
if (!existsSync(join(DIST, "index.html"))) { console.error("menu: build dist first (npm run build)"); process.exit(1); }

// A build served under a prefix (BASE=/repo/model-hub/) writes that prefix into every page; the paths
// here are relative to it, so read it back rather than assuming this build was made for the site root.
const BASE = (/data-base="([^"]*)"/.exec(await readFile(join(DIST, "index.html"), "utf8"))?.[1] || "/").replace(/\/$/, "");

// Any one model page stands for the five hundred: they come out of one template.
const org = (await readdir(join(DIST, "models"), { withFileTypes: true })).find((d) => d.isDirectory());
const name = org && (await readdir(join(DIST, "models", org.name), { withFileTypes: true })).find((d) => d.isDirectory());
if (!name) { console.error("menu: dist has no model pages"); process.exit(1); }
PAGES[2][1] = `/models/${org.name}/${name.name}/`;

const { server, port } = await serve(DIST, BASE);
const origin = `http://127.0.0.1:${port}`;
const proc = spawn(browser, [
  "--headless=new", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  "--disable-gpu", "--hide-scrollbars", "--user-data-dir=" + join(process.env.TEMP || "/tmp", `menu-${process.pid}`), `${origin}${BASE}/`,
], { stdio: ["ignore", "ignore", "pipe"] });

const devtools = await new Promise((ok, bad) => {
  let buf = "";
  const timer = setTimeout(() => bad(new Error("browser did not report a DevTools endpoint")), 20000);
  proc.stderr.on("data", (d) => { buf += d; const m = buf.match(/ws:\/\/[^\s]+/); if (m) { clearTimeout(timer); ok(m[0]); } });
  proc.on("exit", (c) => { clearTimeout(timer); bad(new Error(`browser exited (${c})`)); });
});
const httpBase = devtools.replace(/^ws:\/\//, "http://").replace(/\/devtools\/.*$/, "");
const targets = await fetch(`${httpBase}/json/list`).then((r) => r.json());
const cdp = await CDP.open(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await cdp.send("Page.enable");
await cdp.send("Runtime.enable");

async function load(url) {
  await cdp.send("Page.navigate", { url });
  for (let i = 0; i < 120; i++) { if (await cdp.evaluate("document.readyState === 'complete'")) break; await sleep(50); }
  await sleep(90);
}

const problems = [];
const rows = [];
const leads = new Map(); // theme+width → where the first catalogue page put its lead row
let checked = 0, lowest = Infinity;
// Sign-in is only built when the build was given a Privy app id, so whether the control exists is a property
// of the build, not of the page. What is checked is that every page agrees: the first page read sets the
// expectation and the rest have to match it.
let cluster = null, links = null;
const tags = new Map();    // theme+width → how the first section drew its tag
const tagged = new Set();  // which sections were seen carrying one
for (const theme of THEMES) {
  await load(`${origin}${BASE}/`);
  await cdp.evaluate(`localStorage.setItem("hologram-models-hub.theme", ${JSON.stringify(JSON.stringify({ mode: theme, wallpaper: "alps" }))})`);
  for (const [page, path, section, catalogue, sectionRoot] of PAGES) {
    for (const width of WIDTHS) {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: width < 768 });
      await load(origin + BASE + path);
      const r = await cdp.evaluate(READ);
      const where = `${theme} ${page} ${width}px`;
      checked++;

      if (!r.header) { problems.push(`${where}: no header at all`); continue; }
      if (!r.nav) { problems.push(`${where}: the header has no top-level menu`); continue; }
      if (r.labels.join(" · ") !== SECTIONS.join(" · ")) problems.push(`${where}: the menu reads ${r.labels.join(" · ")}`);
      if (r.marks.some((n) => n !== 1)) problems.push(`${where}: a section lost its mark (${r.marks.join(",")})`);
      cluster ??= { theme: r.theme, account: r.account };
      if (!r.theme) problems.push(`${where}: the header has no theme switch`);
      if (r.account !== cluster.account) problems.push(`${where}: sign-in is ${r.account ? "present" : "missing"} and elsewhere it is not`);
      if (r.headerOverflow > 1) problems.push(`${where}: the header row overflows by ${r.headerOverflow}px`);
      if (r.pageOverflowX > 1) problems.push(`${where}: the page scrolls ${r.pageOverflowX}px sideways`);

      // The section tag. Every section carries one, it sits to the right of that section's title on the same
      // line, and it is drawn the same way everywhere: a reader crossing from Models to Docs sees one control
      // that has not changed, and an agent finds the address in the same place whichever page it landed on.
      if (sectionRoot) {
        if (!r.tag) problems.push(`${where}: the ${section} heading carries no section tag`);
        else {
          tagged.add(section);
          if (r.tag.section !== section.toLowerCase()) problems.push(`${where}: the tag says ${r.tag.section}, the section is ${section}`);
          if (!r.tag.dot) problems.push(`${where}: the tag has no liveness dot`);
          // Beside the title where there is room for it. On a phone the row wraps and the tag drops under the
          // heading, which is the row doing its job rather than the tag losing its place; what still has to
          // hold there is that it follows the title, which the markup guarantees. So the geometry is checked
          // where the row does not wrap.
          if (width >= 768) {
            if (r.tag.rightOfTitle === false) problems.push(`${where}: the tag sits left of the title`);
            if (r.tag.sameLine === false) problems.push(`${where}: the tag is off the title's line`);
          } else if (r.tag.below === false) problems.push(`${where}: the row wrapped and the tag did not follow the title`);
          const key = `${theme} ${width}`;
          if (!tags.has(key)) tags.set(key, { page, look: r.tag.look });
          else if (tags.get(key).look !== r.tag.look) {
            problems.push(`${where}: the tag is drawn ${r.tag.look}, on ${tags.get(key).page} it is ${tags.get(key).look}`);
          }
        }
      } else if (r.tag) problems.push(`${where}: a section tag on a page that is not a section front page`);

      // The lead row: every catalogue page opens with the same one, and it sits at the same height on each, so
      // crossing from Models to Registry to Spaces moves nothing. A page outside the catalogue has none.
      if (catalogue === true) {
        if (!r.lead) problems.push(`${where}: no lead row`);
        else {
          if (r.lead.name !== section) problems.push(`${where}: the lead row says ${r.lead.name || "nothing"}, the section is ${section}`);
          if (r.lead.order !== LEAD) problems.push(`${where}: the lead row reads ${r.lead.order}, expected ${LEAD}`);
          const key = `${theme} ${width}`;
          if (!leads.has(key)) leads.set(key, { page, top: r.lead.top, font: r.lead.font });
          else {
            const first = leads.get(key);
            if (Math.abs(first.top - r.lead.top) > 1) problems.push(`${where}: the lead row sits at ${r.lead.top}px, on ${first.page} it sits at ${first.top}px`);
            if (first.font !== r.lead.font) problems.push(`${where}: the lead row is set in ${r.lead.font}, on ${first.page} in ${first.font}`);
          }
        }
      } else if (r.lead && !catalogue) problems.push(`${where}: a lead row on a page outside the catalogue`);

      if (r.current.length > 1) problems.push(`${where}: ${r.current.length} sections marked current`);
      if ((r.current[0] || "") !== section) problems.push(`${where}: current is ${r.current[0] || "nothing"}, expected ${section || "nothing"}`);
      if (section) {
        if (r.currentColor === r.restColor) problems.push(`${where}: the marked section is the same colour as the rest`);
        if (r.currentContrast < AA) problems.push(`${where}: the marked section reads ${r.currentContrast} < ${AA}`);
        lowest = Math.min(lowest, r.currentContrast);
      }
      rows.push(`${where.padEnd(28)} menu ${r.shown ? "shown " : "folded"} current ${String(section || "—").padEnd(9)}${section ? ` ${r.currentContrast}:1` : ""}`);
      links ??= r.hrefs;
    }
  }
}

// A section that 404s is worse than no section: this menu has shipped with a dead Registry link before.
for (const href of links || []) {
  const res = await fetch(origin + href, { redirect: "manual" });
  if (res.status >= 400) problems.push(`the menu points at ${href}, which answers ${res.status}`);
}

cdp.close();
proc.kill();
server.close();

console.log(rows.join("\n"));
if (problems.length) { console.error(`\n${problems.length} problems\n${problems.join("\n")}`); process.exit(1); }
console.log(`\nthe menu holds: ${SECTIONS.join(" · ")} on ${PAGES.length} page shapes x ${THEMES.length} themes x ${WIDTHS.length} widths (${checked} checks); the section you are in is marked and reads at ${lowest}:1 or better; each of the ${tagged.size} sections carries the same tag beside its title; the ${PAGES.filter((p) => p[3] === true).length} catalogue pages open with the same lead row (${LEAD}) at the same height`);
