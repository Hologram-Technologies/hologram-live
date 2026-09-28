// The verifying edge (hub-resolve with HUB_VERIFY=1) against a holder that lies. Offline: two local holders serve the
// same synthetic model, one of them with a byte flipped; the index names the true sha256.
//
//   node deploy/qa/verify-edge.mjs
//
// What must hold:
//   a small file (under the 64 KiB held back) from a lying first holder: refused inside the one request, the client
//     gets the honest holder's bytes and never sees the wrong ones
//   a large file: the lying holder's stream is cut before its last bytes, so the client cannot finish a wrong file;
//     its retry is served by the honest holder, byte-exact
//   then from the cache: Range answers 206 with the right slice; HEAD carries the index's sha256
//   Ollama's blob route: 307 to the other loopback name, which serves the verified bytes
import http from "node:http";
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const EDGE = 8395, HOLDERS = 8396;
const sha = (b) => createHash("sha256").update(b).digest("hex");
const small = Buffer.from(JSON.stringify({ model_type: "llama", hidden_size: 64 }));
const large = randomBytes(3 * 1048576 + 17);
const gguf = Buffer.concat([Buffer.from("GGUF"), randomBytes(512 * 1024)]);
const files = { "config.json": small, "model.safetensors": large, "tiny-Q8_0.gguf": gguf };
const hits = { liar: 0, honest: 0 };

// /liar/<path> flips one byte in the middle; /honest/<path> serves the truth.
const holders = http.createServer((req, res) => {
  const [, who, ...rest] = decodeURIComponent(new URL(req.url, "http://x").pathname).split("/");
  const body = files[rest.join("/")];
  if (!body || !(who in hits)) { res.writeHead(404); return res.end(); }
  hits[who]++;
  const out = Buffer.from(body);
  if (who === "liar") out[out.length >> 1] ^= 1;
  res.writeHead(200, { "content-length": out.length });
  res.end(out);
}).listen(HOLDERS);

const pass = [], fail = [];
const ok = (n, d = "") => { pass.push(n); console.log(`ok   ${n.padEnd(58)} ${d}`); };
const bad = (n, d) => { fail.push(`${n} — ${d}`); console.log(`FAIL ${n.padEnd(58)} ${d}`); };
const get = (path, init = {}, host = "127.0.0.1") => fetch(`http://${host}:${EDGE}${path}`, { redirect: "manual", signal: AbortSignal.timeout(30_000), ...init });
const body = async (r) => { try { return Buffer.from(await r.arrayBuffer()); } catch { return null; } };

async function main() {
  const root = await mkdtemp(join(tmpdir(), "verify-edge-"));
  const data = join(root, "data"), cache = join(root, "cache");
  await mkdir(join(data, "fixture"), { recursive: true }).then(() => mkdir(join(data, "files", "fixture"), { recursive: true }));
  const H = `http://127.0.0.1:${HOLDERS}`;
  const doc = {
    revision: "0".repeat(40), manifest: `blake3:${"1".repeat(64)}`,
    sources: [
      { kind: "huggingface.co", resolve: null, missing: [] },                       // first choice: the liar
      { kind: "modelscope.cn", resolve: `${H}/honest/`, missing: [] },               // second: the honest holder
    ],
    files: Object.entries(files).map(([p, b]) => [p, b.length, `sha256:${sha(b)}`, p === "config.json" ? 0 : 1, `${H}/liar/${p}`]),
  };
  await writeFile(join(data, "files", "fixture", "tiny.json"), JSON.stringify(doc));

  const edge = spawn(process.execPath, [join(HERE, "..", "hub-resolve.mjs")], {
    env: { ...process.env, HUB_VERIFY: "1", HUB_CACHE: cache, HUB_DATA: data, PORT: String(EDGE), HUB: "http://127.0.0.1:9" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const log = [];
  edge.stdout.on("data", (d) => log.push(String(d)));
  await new Promise((r) => { edge.stdout.on("data", (d) => { if (String(d).includes("listening")) r(); }); setTimeout(r, 3000); });

  try {
    // small file: the liar's bytes never leave the edge
    const a = await get("/fixture/tiny/resolve/main/config.json");
    const ab = await body(a);
    if (a.status === 200 && ab?.equals(small)) ok("small file: liar refused inside one request", `liar ${hits.liar}, honest ${hits.honest}`);
    else bad("small file: liar refused inside one request", `status ${a.status}, ${ab?.length} bytes, equal ${ab?.equals(small)}`);

    // large file: first answer is cut short, never complete; the retry is exact
    hits.liar = hits.honest = 0;
    const b1 = await get("/fixture/tiny/resolve/main/model.safetensors");
    const bb1 = await body(b1);
    if (!bb1 || bb1.length < large.length) ok("large file: the liar's stream is cut before the end", `${bb1 ? bb1.length : "no"} of ${large.length} bytes, then reset`);
    else bad("large file: the liar's stream is cut before the end", `the client received all ${bb1.length} bytes (equal ${bb1.equals(large)})`);
    const b2 = await get("/fixture/tiny/resolve/main/model.safetensors");
    const bb2 = await body(b2);
    if (b2.status === 200 && bb2?.equals(large)) ok("large file: the retry is served by the honest holder", `sha256 ${sha(bb2).slice(0, 12)}, liar asked ${hits.liar}x`);
    else bad("large file: the retry is served by the honest holder", `status ${b2.status}, equal ${bb2?.equals(large)}`);

    // from the cache
    const r = await get("/fixture/tiny/resolve/main/model.safetensors", { headers: { range: "bytes=1000-1999" } });
    const rb = await body(r);
    if (r.status === 206 && rb?.equals(large.subarray(1000, 2000))) ok("Range from the verified cache", r.headers.get("content-range"));
    else bad("Range from the verified cache", `status ${r.status}`);
    const h = await get("/fixture/tiny/resolve/main/model.safetensors", { method: "HEAD" });
    if (h.status === 200 && h.headers.get("x-linked-etag") === `"${sha(large)}"` && h.headers.get("x-hub-source") === "verified") ok("HEAD names the index's sha256", "x-hub-source: verified");
    else bad("HEAD names the index's sha256", `${h.status} ${h.headers.get("x-linked-etag")}`);

    // Ollama: manifest, then a 307 to the other loopback name, which serves verified bytes
    const m = await get("/v2/fixture/tiny/manifests/Q8_0", { headers: { "user-agent": "ollama/0.12.0" } });
    const mj = m.status === 200 ? await m.json() : null;
    const layer = mj?.layers?.find((l) => l.digest === `sha256:${sha(gguf)}`);
    if (layer) {
      const hop = await get(`/v2/fixture/tiny/blobs/${layer.digest}`);
      const loc = hop.headers.get("location") || "";
      if (hop.status === 307 && loc.startsWith(`http://localhost:${EDGE}/_blob/`)) ok("Ollama blob: 307 to the other loopback name", loc.replace(/sha256:(.{12}).*/, "sha256:$1…"));
      else bad("Ollama blob: 307 to the other loopback name", `${hop.status} ${loc}`);
      const g = await fetch(loc, { signal: AbortSignal.timeout(30_000) });
      const gb = await body(g);
      if (!gb?.equals(gguf)) { const g2 = await fetch(loc, { signal: AbortSignal.timeout(30_000) }); const gb2 = await body(g2); if (gb2?.equals(gguf)) ok("Ollama blob: verified bytes (after the liar was cut)", `${gb2.length} bytes`); else bad("Ollama blob: verified bytes", `${g2.status}`); }
      else ok("Ollama blob: verified bytes", `${gb.length} bytes`);
    } else bad("Ollama manifest names the GGUF", `status ${m.status}`);

    // Git LFS batch: download hrefs to the verifying /_blob/ route; uploads refused; unknown oids answered per object
    const lfs = (b) => get("/fixture/tiny.git/info/lfs/objects/batch", { method: "POST", headers: { "content-type": "application/vnd.git-lfs+json" }, body: JSON.stringify(b) });
    const bt = await lfs({ operation: "download", transfers: ["basic"], objects: [{ oid: sha(large), size: large.length }, { oid: "0".repeat(64), size: 1 }] });
    const bj = bt.status === 200 ? await bt.json() : null;
    const href = bj?.objects?.[0]?.actions?.download?.href;
    const got = href && await body(await fetch(href, { signal: AbortSignal.timeout(30_000) }));
    if (got?.equals(large) && bj.objects[1].error?.code === 404) ok("Git LFS batch: verified href, unknown oid 404", href.replace(/sha256:(.{12}).*/, "sha256:$1…"));
    else bad("Git LFS batch: verified href, unknown oid 404", `${bt.status} ${JSON.stringify(bj).slice(0, 100)}`);
    const up = await lfs({ operation: "upload", objects: [{ oid: sha(large), size: large.length }] });
    if (up.status === 403) ok("Git LFS batch: upload refused", "403");
    else bad("Git LFS batch: upload refused", String(up.status));

    const refused = log.join("").split("\n").filter((l) => l.includes('"refused"')).length;
    if (refused >= 3) ok("every lie was logged", `${refused} refusals`);
    else bad("every lie was logged", `${refused} refusals`);
  } finally {
    edge.kill(); holders.close();
  }
  console.log(`\n${pass.length} passed, ${fail.length} failed`);
  for (const f of fail) console.log(`  ${f}`);
  process.exit(fail.length ? 1 : 0);
}
main().catch((e) => { console.error(e); process.exit(1); });
