// A browser test of the hub worker's κ check (src/zip-sw.js), with honest and lying answers on one origin.
//   node qa/kappa-sw/serve.mjs [port]      then open http://localhost:<port>/  (results on the page and in window.__results)
//
// The worker is served exactly as build.mjs builds it (zip.mjs + zip-sw.js), with the vendored hash-wasm.
import http from "node:http";
import { createHash, randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const PORT = Number(process.argv[2] || 18990);
const sha = (b) => createHash("sha256").update(b).digest("hex");
const small = Buffer.from(JSON.stringify({ model_type: "llama", note: "a small file" }));
const big = randomBytes(24 * 1024 * 1024 + 123);
const lie = (b) => { const x = Buffer.from(b); x[x.length >> 1] ^= 1; return x; };
const files = { "config.json": small, "model.safetensors": big, "liar.json": small, "liar.safetensors": big, "moved.safetensors": big };
const served = { "liar.json": lie(small), "liar.safetensors": lie(big) };        // what the holder sends instead
const worker = `${readFileSync(join(WEB, "src", "zip.mjs"), "utf8").replace(/^export /gm, "")}\n${readFileSync(join(WEB, "src", "zip-sw.js"), "utf8")}`;

const page = `<!doctype html><html data-base="/"><meta charset="utf-8"><title>κ worker test</title><pre id="out">running…</pre><script type="module">
const out = document.getElementById("out"), results = [];
await navigator.serviceWorker.register("/zip-sw.js");
await navigator.serviceWorker.ready;
if (!navigator.serviceWorker.controller) await new Promise((r) => navigator.serviceWorker.addEventListener("controllerchange", r, { once: true }));
async function sha(buf) { return [...new Uint8Array(await crypto.subtle.digest("SHA-256", buf))].map((b) => b.toString(16).padStart(2, "0")).join(""); }
async function t(name, path, init, expect) {
  let r = { name };
  try {
    const res = await fetch(path, init);
    r.status = res.status; r.verified = res.headers.get("x-hologram-verified");
    const b = await res.arrayBuffer(); r.bytes = b.byteLength; r.sha = (await sha(b)).slice(0, 12);
    r.completed = true;
  } catch (e) { r.completed = false; r.error = String(e.message || e).slice(0, 80); }
  r.pass = expect(r); results.push(r); out.textContent = results.map((x) => (x.pass ? "ok   " : "FAIL ") + JSON.stringify(x)).join("\\n");
}
const K = ${JSON.stringify({ small: sha(small), big: sha(big) })};
await t("named blob, honest", "/v2/models/o/m/blobs/sha256:" + K.small, {}, (r) => r.completed && r.verified === "sha256:" + K.small);
await t("named blob, lying holder", "/v2/models/o/m/blobs/sha256:" + K.big + "?liar=1", {}, (r) => !r.completed || r.status === 502);
await t("resolve small, honest", "/o/m/resolve/main/config.json", {}, (r) => r.completed && r.sha === K.small.slice(0, 12) && !!r.verified);
await t("resolve small, lying holder", "/o/m/resolve/main/liar.json", {}, (r) => !r.completed || r.status === 502);
await t("resolve 24 MB, honest", "/o/m/resolve/main/model.safetensors", {}, (r) => r.completed && r.sha === K.big.slice(0, 12) && !!r.verified);
await t("resolve 24 MB, lying holder", "/o/m/resolve/main/liar.safetensors", {}, (r) => !r.completed);
await t("resolve via a redirect, honest", "/o/m/resolve/main/moved.safetensors", {}, (r) => r.completed && r.sha === K.big.slice(0, 12) && !!r.verified);
await t("a Range request passes through", "/o/m/resolve/main/model.safetensors", { headers: { range: "bytes=0-99" } }, (r) => r.completed && r.bytes === 100 && !r.verified);
await t("a page is not intercepted", "/", {}, (r) => r.completed && !r.verified);
window.__results = results;
out.textContent += "\\n" + (results.every((x) => x.pass) ? "ALL PASS" : "FAILURES");
</script></html>`;

http.createServer((req, res) => {
  const u = new URL(req.url, "http://x"), p = u.pathname;
  if (p === "/") { res.writeHead(200, { "content-type": "text/html; charset=utf-8" }); return res.end(page); }
  if (p === "/zip-sw.js") { res.writeHead(200, { "content-type": "text/javascript", "cache-control": "no-store" }); return res.end(worker); }
  if (p === "/vendor/hash-wasm/sha256.umd.min.js") { res.writeHead(200, { "content-type": "text/javascript" }); return res.end(readFileSync(join(WEB, "vendor", "hash-wasm", "sha256.umd.min.js"))); }
  if (p === "/api/models/o/m/tree/main") { const t = JSON.stringify(Object.entries(files).map(([path, b]) => ({ type: "file", path, size: b.length, oid: sha(b) }))); res.writeHead(200, { "content-type": "application/json" }); return res.end(t); }
  let m = /^\/v2\/models\/o\/m\/blobs\/sha256:([0-9a-f]{64})$/.exec(p);
  if (m) { const b = [small, big].find((x) => sha(x) === m[1]); if (!b) { res.writeHead(404); return res.end(); } const body = u.searchParams.get("liar") ? lie(b) : b; res.writeHead(200, { "content-length": body.length }); return res.end(body); }
  m = /^\/o\/m\/resolve\/main\/(.+)$/.exec(p);
  if (m && files[m[1]]) {
    if (m[1] === "moved.safetensors") { res.writeHead(302, { location: "/cdn/moved.bin" }); return res.end(); }
    const body = served[m[1]] || files[m[1]];
    const r = /^bytes=(\d+)-(\d+)$/.exec(req.headers.range || "");
    if (r) { const part = body.subarray(+r[1], +r[2] + 1); res.writeHead(206, { "content-length": part.length, "content-range": `bytes ${r[1]}-${r[2]}/${body.length}` }); return res.end(part); }
    res.writeHead(200, { "content-length": body.length }); return res.end(body);
  }
  if (p === "/cdn/moved.bin") { res.writeHead(200, { "content-length": big.length }); return res.end(big); }
  res.writeHead(404); res.end();
}).listen(PORT, () => console.log(`κ worker test on http://localhost:${PORT}/`));
