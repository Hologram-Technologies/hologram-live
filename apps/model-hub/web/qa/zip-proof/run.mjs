// Transport proof: one engine per run. Serves ./site on localhost (a secure context, so service workers work), clicks
// the proof link, lets the engine's download manager save the streamed zip, and reports size, time and browser memory.
//
//   node run.mjs <chromium|firefox|webkit> <tiny|6g>      (inside mcr.microsoft.com/playwright, see README in the PR)
import { chromium, firefox, webkit } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { createHash } from "node:crypto";

const [engineName = "chromium", spec = "tiny", trigger = "go"] = process.argv.slice(2); // trigger: go (link) or frame
const ROOT = path.resolve("site"), OUT = path.resolve("out");
fs.mkdirSync(OUT, { recursive: true });
const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript" };
// Two local "model files" for the logic tests: ok.bin is served whole; cut.bin drops the connection half way on its
// first request and honours Range afterwards, so the worker has to resume. test/bad names a wrong address for ok.bin.
const blob = (seed, size) => Buffer.from(Uint8Array.from({ length: size }, (_, i) => (i * seed + (i >> 7)) & 0xff));
const blobs = { "ok.bin": blob(13, 3 * 1024 * 1024 + 11), "cut.bin": blob(29, 5 * 1024 * 1024 + 3) };
const address = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
const testModel = (bad) => ({ revision: "0123456789abcdef0123456789abcdef01234567", manifest: "blake3:" + "0".repeat(64),
  sources: [{ kind: "huggingface.co", name: "Rig", page: "", resolve: null, missing: [], p2p: false }],
  files: Object.entries(blobs).map(([name, b]) => [name, b.length, bad && name === "ok.bin" ? address(b).replace(/.$/, (c) => (c === "0" ? "1" : "0")) : address(b), 0, `http://localhost:8099/blob/${name}`]) });
let cuts = 0;
const server = http.createServer((req, res) => {
  const test = req.url.match(/^\/data\/files\/test\/(good|bad)\.json/);
  if (test) { res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify(testModel(test[1] === "bad"))); return; }
  const wanted = req.url.match(/^\/blob\/([a-z.]+)/);
  if (wanted && blobs[wanted[1]]) {
    const bytes = blobs[wanted[1]], range = /bytes=(\d+)-/.exec(req.headers.range || "");
    const from = range ? Number(range[1]) : 0;
    res.writeHead(range ? 206 : 200, { "content-type": "application/octet-stream", "content-length": String(bytes.length - from), "accept-ranges": "bytes", ...(range ? { "content-range": `bytes ${from}-${bytes.length - 1}/${bytes.length}` } : {}) });
    if (wanted[1] === "cut.bin" && !range) { cuts++; res.write(bytes.subarray(0, bytes.length >> 1), () => res.destroy()); return; }
    res.end(bytes.subarray(from));
    return;
  }
  // Control: the same volume as an ordinary server download (no service worker), paced to 60 MB/s.
  if (req.url.startsWith("/plain/")) {
    const size = 5905700575, chunk = Buffer.alloc(1024 * 1024, 7), started = Date.now();
    res.writeHead(200, { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="plain.bin"', "content-length": String(size) });
    let sent = 0;
    const pump = () => {
      while (sent < size) {
        const due = (sent / (60 * 1024 * 1024)) * 1000 - (Date.now() - started);
        if (due > 0) { setTimeout(pump, due); return; }
        const n = Math.min(chunk.length, size - sent);
        sent += n;
        if (!res.write(n === chunk.length ? chunk : chunk.subarray(0, n))) { res.once("drain", pump); return; }
      }
      res.end();
    };
    pump();
    return;
  }
  let file = path.join(ROOT, decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (file.endsWith(path.sep) || file === ROOT) file = path.join(file, "index.html");
  if (!file.startsWith(ROOT) || !fs.existsSync(file)) { res.writeHead(404); res.end("not served here: the zip is assembled in the browser"); return; }
  res.writeHead(200, { "content-type": types[path.extname(file)] || "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
}).listen(8099);

// Peak resident memory of every browser process, sampled every 2 s.
let peak = 0;
const measure = () => {
  try {
    const rows = execSync("ps -eo rss=,args=", { encoding: "utf8" }).split("\n").filter((l) => /chrom|firefox|webkit|MiniBrowser|WPE|playwright\/.*(chrome|firefox|pw_run)/i.test(l) && !/node /.test(l));
    const now = rows.reduce((sum, l) => sum + Number(l.trim().split(/\s+/)[0] || 0), 0);
    peak = Math.max(peak, now);
    return now;
  } catch { return 0; /* ps missing */ }
};
const sample = setInterval(measure, 2000);

const engine = { chromium, firefox, webkit }[engineName];
const result = { engine: engineName, spec };
const browser = await engine.launch({ downloadsPath: OUT });
try {
  result.version = browser.version();
  const context = await browser.newContext({ acceptDownloads: true });
  const page = await context.newPage();
  page.on("console", (m) => { if (m.type() === "error") console.error("console:", m.text()); });
  await page.goto("http://localhost:8099/");
  await page.waitForFunction(() => window.ready === true, null, { timeout: 60000 });
  result.declared = spec === "plain" ? 5905700575 : await page.evaluate((s) => window.expected[s.replace("net", "")], spec);
  result.connectionCuts = () => cuts;
  result.baselineBrowserMB = Math.round(measure() / 1024);
  const started = Date.now();
  const [download] = await Promise.all([page.waitForEvent("download", { timeout: 60000 }), page.click(`#${trigger}-${spec}`)]);
  result.filename = download.suggestedFilename();
  // Resolves when the download has finished. A stream the worker errored must end the download; if the engine leaves
  // it hanging instead, say so after 40 s and list what it left on disk.
  const file = await Promise.race([download.path().catch((e) => { result.error = String(e.message).split(String.fromCharCode(10))[0]; return null; }), new Promise((r) => setTimeout(() => r(undefined), spec === "bad" ? 40000 : 3600000))]);
  if (file === undefined) result.hung = true;
  result.failure = file === undefined ? "still in progress" : await download.failure();
  result.left = fs.readdirSync(OUT).map((f) => `${f}:${fs.statSync(path.join(OUT, f)).size}`);
  result.seconds = Math.round((Date.now() - started) / 1000);
  result.saved = file && fs.existsSync(file) ? fs.statSync(file).size : 0;
  result.sizeMatches = result.saved === result.declared;
  result.progress = await page.evaluate(() => window.progress).catch(() => null);
  result.connectionCuts = cuts;
  if (file) fs.renameSync(file, path.join(OUT, `${engineName}-${spec}.zip`));
  result.trigger = trigger;
} catch (error) {
  result.error = String(error.message || error).split("\n")[0];
} finally {
  clearInterval(sample);
  result.peakBrowserMB = Math.round(peak / 1024);
  await browser.close().catch(() => {});
  server.close();
}
console.log(JSON.stringify(result));
