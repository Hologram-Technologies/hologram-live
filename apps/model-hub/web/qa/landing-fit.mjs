// Proves the landing page holds one screen: no scroll, nothing clipped, contrast intact.
//
//   node apps/model-hub/web/qa/landing-fit.mjs            (build dist first)
//   CHROME=/path/to/chrome node .../landing-fit.mjs
//
// Serves web/dist itself and drives a headless Chromium over the DevTools protocol with Node's own
// WebSocket. No dependency, and no browser needed for the rest of the build: with none installed it
// prints why and exits 0, so it never blocks a machine that cannot run one.

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(SITE, "dist");

// Every shape the hero has to survive, including a deliberately short laptop and a small phone.
const VIEWPORTS = [
  [320, 568, "small phone"],
  [390, 844, "phone"],
  [768, 1024, "tablet"],
  [1440, 700, "short laptop"],
  [1600, 808, "laptop at 125%"],
  [2000, 1010, "wide desktop"],
  [1440, 900, "laptop"],
  [1920, 1080, "desktop"],
  [2560, 1440, "ultrawide"],
];
const THEMES = ["dark", "light", "immersive"];
// --shoot <dir> writes one PNG per theme at these shapes, the evidence a review asks for.
const SHOOT = [[390, 844], [1440, 900]];

const TYPES = { html: "text/html", js: "text/javascript", mjs: "text/javascript", css: "text/css", json: "application/json", svg: "image/svg+xml", woff2: "font/woff2", jpg: "image/jpeg", png: "image/png" };

function findBrowser() {
  const candidates = [
    process.env.CHROME,
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ];
  return candidates.find((p) => p && existsSync(p));
}

function serve(root) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://localhost");
    let path = normalize(join(root, decodeURIComponent(url.pathname)));
    if (!path.startsWith(root)) return res.writeHead(403).end();
    if (path.endsWith("/") || path.endsWith("\\") || (existsSync(path) && !/\.[a-z0-9]+$/i.test(path))) path = join(path, "index.html");
    try {
      const body = await readFile(path);
      res.writeHead(200, { "content-type": TYPES[path.split(".").pop().toLowerCase()] || "application/octet-stream" }).end(body);
    } catch { res.writeHead(404).end("not found"); }
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok({ server, port: server.address().port })));
}

// ---- the smallest usable DevTools client
class CDP {
  constructor(ws) { this.ws = ws; this.id = 0; this.waiting = new Map(); ws.onmessage = (e) => { const m = JSON.parse(e.data); const w = this.waiting.get(m.id); if (w) { this.waiting.delete(m.id); m.error ? w.bad(new Error(m.error.message)) : w.ok(m.result); } }; }
  static async open(url) { const ws = new WebSocket(url); await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = () => bad(new Error(`cannot reach ${url}`)); }); return new CDP(ws); }
  send(method, params = {}) { const id = ++this.id; this.ws.send(JSON.stringify({ id, method, params })); return new Promise((ok, bad) => this.waiting.set(id, { ok, bad })); }
  async evaluate(expression) { const r = await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || "evaluate failed"); return r.result.value; }
  close() { this.ws.close(); }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Runs in the page. Nothing may scroll, and every part of the hero must sit inside the viewport.
const FIT = `(() => {
  const d = document.documentElement, T = 1;
  // The strip counts too when it is on: a hero that pushes it past the bottom edge is a clipped page.
  const must = [".top", ".land-badge", ".land-title", ".land-sub", ".land-actions .button", ".land-second", ".land-actions", ".land-marquee"];
  const outside = [];
  for (const sel of must) {
    const el = document.querySelector(sel);
    if (!el) { outside.push(sel + " missing"); continue; }
    if (getComputedStyle(el).display === "none") continue;
    const r = el.getBoundingClientRect();
    if (r.top < -T || r.left < -T || r.bottom > innerHeight + T || r.right > innerWidth + T)
      outside.push(sel + " " + [r.left, r.top, r.right, r.bottom].map(Math.round).join(","));
  }
  return {
    overflowY: d.scrollHeight - innerHeight,
    overflowX: d.scrollWidth - innerWidth,
    outside,
    theme: d.dataset.theme,
    display: parseFloat(getComputedStyle(document.querySelector(".land-title")).fontSize),
    accents: [...document.querySelectorAll(".land-actions .i, .land .i")].length,
  };
})()`;

const shotFlag = process.argv.indexOf("--shoot");
const shots = shotFlag > 0 ? process.argv[shotFlag + 1] : null;
if (shots) await mkdir(shots, { recursive: true });

const browser = findBrowser();
if (!browser) { console.log("landing-fit: no Chromium found (set CHROME=<path>); skipped"); process.exit(0); }
if (!existsSync(join(DIST, "index.html"))) { console.error("landing-fit: build dist first (npm run build)"); process.exit(1); }

const { server, port } = await serve(DIST);
const home = `http://127.0.0.1:${port}/`;
const proc = spawn(browser, [
  "--headless=new", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  "--disable-gpu", "--hide-scrollbars", "--user-data-dir=" + join(process.env.TEMP || "/tmp", `landing-fit-${process.pid}`), home,
], { stdio: ["ignore", "ignore", "pipe"] });

const devtools = await new Promise((ok, bad) => {
  let buf = "";
  const timer = setTimeout(() => bad(new Error("browser did not report a DevTools endpoint")), 20000);
  proc.stderr.on("data", (d) => { buf += d; const m = buf.match(/ws:\/\/[^\s]+/); if (m) { clearTimeout(timer); ok(m[0]); } });
  proc.on("exit", (c) => { clearTimeout(timer); bad(new Error(`browser exited (${c})`)); });
});

const httpBase = devtools.replace(/^ws:\/\//, "http://").replace(/\/devtools\/.*$/, "");
const targets = await fetch(`${httpBase}/json/list`).then((r) => r.json());
const pageTarget = targets.find((t) => t.type === "page");
const cdp = await CDP.open(pageTarget.webSocketDebuggerUrl);
await cdp.send("Page.enable");
await cdp.send("Runtime.enable");

const contrastSrc = await readFile(join(SITE, "qa", "contrast.js"), "utf8");

async function load(url) {
  await cdp.send("Page.navigate", { url });
  for (let i = 0; i < 100; i++) { if (await cdp.evaluate("document.readyState === 'complete'")) break; await sleep(50); }
  await sleep(120); // fonts settle: the display step depends on the metrics of Archivo
}

const problems = [];
// Immersive puts the type straight on a photograph the viewer picks, undimmed and with nothing drawn behind
// the words. That is a deliberate call, and no static check can rule on an unknown image, so its contrast is
// reported and never fails the run. Dark and Light are held to AA.
const notes = [];
const rows = [];
for (const theme of THEMES) {
  await load(home);
  await cdp.evaluate(`localStorage.setItem("hologram-models-hub.theme", ${JSON.stringify(JSON.stringify({ mode: theme, wallpaper: "alps" }))})`);
  for (const [w, h, label] of VIEWPORTS) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: w < 768 });
    await load(home);
    const fit = await cdp.evaluate(FIT);
    const contrast = await cdp.evaluate(contrastSrc);
    const where = `${theme} ${w}x${h} (${label})`;
    if (shots && SHOOT.some(([sw, sh]) => sw === w && sh === h)) {
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      await writeFile(join(shots, `landing-${theme}-${w}x${h}.png`), Buffer.from(data, "base64"));
    }
    if (fit.theme !== theme) problems.push(`${where}: theme did not apply (${fit.theme})`);
    if (fit.overflowY > 1) problems.push(`${where}: scrolls ${fit.overflowY}px vertically`);
    if (fit.overflowX > 1) problems.push(`${where}: scrolls ${fit.overflowX}px horizontally`);
    for (const o of fit.outside) problems.push(`${where}: outside the viewport ${o}`);
    for (const f of contrast.failures) (theme === "immersive" ? notes : problems).push(`${where}: contrast ${f}`);
    rows.push(`${where.padEnd(34)} display ${String(Math.round(fit.display)).padStart(3)}px  overflow ${fit.overflowY}/${fit.overflowX}  contrast lowest ${contrast.lowest} over ${contrast.checked} texts`);
  }
}

cdp.close();
proc.kill();
server.close();

console.log(rows.join("\n"));
if (notes.length) console.log(`\nImmersive, reported and not enforced (${notes.length}): the hero sits on an undimmed photograph.\n${[...new Set(notes.map((n) => n.replace(/^immersive \S+ \([^)]*\): /, "  ")))].join("\n")}`);
if (problems.length) { console.error(`\n${problems.length} problems\n${problems.join("\n")}`); process.exit(1); }
console.log(`\nlanding fits: ${THEMES.length} themes x ${VIEWPORTS.length} viewports, no scroll, nothing clipped, contrast AA in Dark and Light`);
