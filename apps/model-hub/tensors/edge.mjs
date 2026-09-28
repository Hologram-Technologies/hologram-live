#!/usr/bin/env node
// hologram edge: a verifying gateway on your own machine. Point any client at it and every byte it hands over has
// been checked against its κ first, whichever holder sent it.
//
//   node edge.mjs [--hub https://gethologram.ai] [--port 8095] [--ipfs http://127.0.0.1:8080]
//   HF_ENDPOINT=http://127.0.0.1:8095 hf download HuggingFaceTB/SmolLM2-135M-Instruct       (huggingface_hub, transformers,
//                                                                                           vLLM, mlx-lm, llama.cpp -hf …)
//   crane pull --insecure 127.0.0.1:8095/huggingfacetb/smollm2-135m-instruct:safetensors out.tar   (OCI: oras, crane, KitOps)
//
// What it trusts: the hub's answer to "which index κ does this name point to" (the signed names log replaces even
// that). Everything below the index is checked against a κ named by the object above it:
//   manifests and layouts           hashed against the digest that names them
//   weight files with a layout      rebuilt through get(κ) (deploy/kappa-get.mjs): tensors piece by piece from any
//                                   holder (their own file on Hugging Face, other repos, IPFS by raw CID), each piece
//                                   checked before release, liars and slow holders demoted
//   every other file                streamed from any holder while hashed; the last 64 KiB are held back until the
//                                   whole equals the manifest's digest, and a mismatch cuts the connection, so a
//                                   client can never finish a wrong file
import http from "node:http";
import { createHash } from "node:crypto";
import { assemble } from "../deploy/tensor-assemble.mjs";

const args = process.argv.slice(2), opt = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const HUB = opt("--hub", process.env.HOLOGRAM_HUB || "https://gethologram.ai").replace(/\/$/, "");
const PORT = Number(opt("--port", process.env.PORT || 8095));
const IPFS = [opt("--ipfs", process.env.HOLOGRAM_IPFS || "")].filter(Boolean);
const ORIGIN = process.env.HF_ORIGIN || "https://huggingface.co";
const HOLD = 64 << 10;
const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
const log = (o) => console.log(JSON.stringify({ t: new Date().toISOString().slice(11, 19), ...o }));

// ---- objects from the hub, each checked against the κ that names it
const objs = new Map();
async function obj(repo, d) {
  if (objs.has(d)) return objs.get(d);
  const r = await fetch(`${HUB}/v2/models/${repo.toLowerCase()}/blobs/${d}`);
  if (!r.ok) throw new Error(`${d}: hub ${r.status}`);
  const b = Buffer.from(await r.arrayBuffer());
  if (sha(b) !== d) throw new Error(`${d}: the hub's bytes do not hash to it; refused`);
  if (b.length < 8 << 20) objs.set(d, b);
  return b;
}
const json = async (repo, d) => JSON.parse((await obj(repo, d)).toString("utf8"));

// ---- a model: the one trusted step (name -> index κ), then everything by hash
const models = new Map();
async function model(id) {
  const hit = models.get(id.toLowerCase());
  if (hit && Date.now() - hit.at < 600_000) return hit;
  const r = await fetch(`${HUB}/v2/models/${id.toLowerCase()}/manifests/index`, { headers: { accept: "application/vnd.oci.image.index.v1+json" } });
  if (!r.ok) return null;
  const bytes = Buffer.from(await r.arrayBuffer()), index = sha(bytes);
  if (r.headers.get("docker-content-digest") && r.headers.get("docker-content-digest") !== index) throw new Error(`${id}: index bytes do not match their digest`);
  objs.set(index, bytes);
  const idx = JSON.parse(bytes.toString("utf8"));
  const pick = (f) => idx.manifests.find((m) => m.annotations?.["org.hologram.format"] === f)?.digest;
  const mk = pick("original") || idx.manifests[0].digest;
  const man = await json(id, mk);
  const repo = man.annotations["org.hologram.repo"], rev = man.annotations["org.hologram.revision"];
  const table = await json(id, man.config.digest);
  const pieces = table.pieces ? await json(id, table.pieces) : null;
  const files = man.layers.map((l) => ({ path: l.annotations["org.opencontainers.image.title"], digest: l.digest, size: l.size, layout: l.annotations["org.hologram.layout"] }));
  const m = { at: Date.now(), id: repo, rev, index, manifests: Object.fromEntries(idx.manifests.map((x) => [x.annotations?.["org.hologram.format"], x.digest])), files, pieces };
  models.set(id.toLowerCase(), m);
  log({ model: repo, index: index.slice(0, 19), files: files.length, pieced: Object.keys(pieces?.of || {}).length });
  return m;
}

// ---- bytes of one file, verified
async function alternatives(m, lay) {
  const r = await fetch(`${HUB}/v2/models/${m.id.toLowerCase()}/alternatives/${lay}`);
  return r.ok ? await r.json() : {};
}
async function* fileBytes(m, f, start, end, report) {
  if (f.layout) {
    const layout = await json(m.id, f.layout), alts = await alternatives(m, f.layout);
    const ctx = { origin: ORIGIN, repo: m.id, rev: m.rev, literal: (d) => obj(m.id, d).catch(() => null), alternatives: (k) => alts[k] || [], pieces: (k) => m.pieces?.of?.[k], ipfs: IPFS, report };
    yield* assemble(layout, ctx, { start, end });
    return;
  }
  // no layout: the hub (or where it redirects) sends the whole file; hash it, hold back the tail, cut on mismatch
  const r = await fetch(`${HUB}/v2/models/${m.id.toLowerCase()}/blobs/${f.digest}`);
  if (!r.ok) throw new Error(`${f.path}: ${r.status}`);
  const h = createHash("sha256"); let held = Buffer.alloc(0), at = 0;
  const out = function* (b) { const lo = Math.max(at, start), hi = Math.min(at + b.length - 1, end); if (hi >= lo) yield b.subarray(lo - at, hi - at + 1); at += b.length; };
  for await (const c of r.body) {
    const b = Buffer.from(c); h.update(b); held = Buffer.concat([held, b]);
    if (held.length > HOLD) { yield* out(held.subarray(0, held.length - HOLD)); held = held.subarray(held.length - HOLD); }
  }
  if (`sha256:${h.digest("hex")}` !== f.digest) throw Object.assign(new Error(`${f.path}: bytes do not hash to ${f.digest}; cut`), { cut: true });
  yield* out(held);
}

async function send(req, res, m, f, extra = {}) {
  const rg = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || "");
  let start = 0, end = f.size - 1;
  if (rg) { start = rg[1] ? +rg[1] : f.size - +rg[2]; end = rg[1] && rg[2] ? Math.min(+rg[2], f.size - 1) : f.size - 1; }
  const head = { "content-type": "application/octet-stream", "accept-ranges": "bytes", "x-repo-commit": m.rev, etag: `"${f.digest.slice(7)}"`, "x-linked-etag": `"${f.digest.slice(7)}"`, "x-linked-size": String(f.size), "x-hologram-verified": "per-piece", ...extra };
  if (req.method === "HEAD") { res.writeHead(200, { ...head, "content-length": f.size }); return res.end(); }
  if (start > end || start >= f.size) { res.writeHead(416, { "content-range": `bytes */${f.size}` }); return res.end(); }
  const stats = { ok: 0, lies: 0, from: {} };
  const report = (e) => { if (e.ok) { stats.ok++; stats.from[e.from] = (stats.from[e.from] || 0) + e.len; } else stats.lies++; };
  let started = false;
  try {
    for await (const c of fileBytes(m, f, start, end, report)) {
      if (!started) { res.writeHead(rg ? 206 : 200, { ...head, "content-length": end - start + 1, ...(rg ? { "content-range": `bytes ${start}-${end}/${f.size}` } : {}) }); started = true; }
      if (!res.write(c)) await new Promise((ok) => res.once("drain", ok));
    }
    if (!started) res.writeHead(rg ? 206 : 200, { ...head, "content-length": 0 });
    res.end();
    log({ file: `${m.id}/${f.path}`, bytes: end - start + 1, pieces: stats.ok, refused: stats.lies, from: Object.fromEntries(Object.entries(stats.from).map(([k, v]) => [k, Math.round(v / 1e5) / 10 + " MB"])) });
  } catch (e) {
    log({ file: `${m.id}/${f.path}`, error: e.message });
    if (started) return res.destroy();                                     // everything already sent was verified
    const t = JSON.stringify({ error: e.message }); res.writeHead(502, { "content-type": "application/json", "content-length": Buffer.byteLength(t) }); res.end(t);
  }
}

const J = (res, status, body, h = {}) => { const t = JSON.stringify(body); res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(t), ...h }); res.end(res.req.method === "HEAD" ? undefined : t); };
const nf = (res, code, message) => J(res, 404, { error: message }, { "x-error-code": code, "x-error-message": message });
const revOk = (m, r) => r === "main" || (r.length >= 7 && m.rev.startsWith(r));

http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://x"), path = decodeURIComponent(url.pathname);
    // ---- Hugging Face dialect
    let x = path.match(/^\/api\/models\/([^/]+\/[^/]+)\/(tree|treesize)\/([^/]+)(?:\/(.*))?$/);
    if (x) {
      const m = await model(x[1]); if (!m) return nf(res, "RepoNotFound", `${x[1]} is not indexed`);
      if (!revOk(m, x[3])) return nf(res, "RevisionNotFound", `indexed at ${m.rev} only`);
      if (url.searchParams.has("cursor")) return J(res, 200, []);
      const at = (x[4] || "").replace(/\/$/, ""), dir = at ? `${at}/` : "", under = m.files.filter((f) => f.path.startsWith(dir));
      if (at && !under.length) return nf(res, "EntryNotFound", `${at} is not a folder`);
      if (x[2] === "treesize") return J(res, 200, { path: at, size: under.reduce((s, f) => s + f.size, 0) });
      const rec = url.searchParams.get("recursive"), deep = rec === null || /^(true|1)$/i.test(rec), out = [], dirs = new Set();
      for (const f of under) {
        const parts = f.path.slice(dir.length).split("/");
        if (rec !== null) for (let i = 1; i < parts.length && (deep || i === 1); i++) dirs.add(dir + parts.slice(0, i).join("/"));
        if (deep || parts.length === 1) out.push({ type: "file", oid: f.digest.slice(7), size: f.size, path: f.path, lfs: { oid: f.digest.slice(7), size: f.size, pointerSize: 0 } });
      }
      for (const d of dirs) out.push({ type: "directory", oid: createHash("sha1").update(d).digest("hex"), size: 0, path: d });
      return J(res, 200, out.sort((a, b) => (a.path < b.path ? -1 : 1)));
    }
    x = path.match(/^\/api\/models\/([^/]+\/[^/]+)\/refs$/);
    if (x) { const m = await model(x[1]); if (!m) return nf(res, "RepoNotFound", "not indexed"); return J(res, 200, { branches: [{ name: "main", ref: "refs/heads/main", targetCommit: m.rev }], tags: [], converts: [] }); }
    x = path.match(/^\/api\/models\/([^/]+\/[^/]+?)(?:\/revision\/(.+))?$/);
    if (x) {
      const m = await model(x[1]); if (!m) return nf(res, "RepoNotFound", `${x[1]} is not indexed`);
      if (x[2] && !revOk(m, x[2])) return nf(res, "RevisionNotFound", `indexed at ${m.rev} only`);
      const blobs = /^(true|1)$/i.test(url.searchParams.get("blobs") || "");
      return J(res, 200, { id: m.id, modelId: m.id, sha: m.rev, private: false, gated: false, disabled: false, tags: [], hologram: { index: m.index },
        siblings: m.files.map((f) => (blobs ? { rfilename: f.path, size: f.size, lfs: { sha256: f.digest.slice(7), size: f.size, pointerSize: 0 } } : { rfilename: f.path })) });
    }
    x = path.match(/^\/([^/]+\/[^/]+)\/resolve\/([^/]+)\/(.+)$/);
    if (x) {
      const m = await model(x[1]); if (!m) return nf(res, "RepoNotFound", `${x[1]} is not indexed`);
      if (!revOk(m, x[2])) return nf(res, "RevisionNotFound", `indexed at ${m.rev} only`);
      const f = m.files.find((y) => y.path === x[3]); if (!f) return nf(res, "EntryNotFound", `${x[3]} is not in ${m.id}`);
      return send(req, res, m, f);
    }
    // ---- OCI / Ollama dialect: manifests checked by digest; a blob is a 307 to the other loopback name (Ollama's rule)
    if (path === "/v2/" || path === "/v2") { res.writeHead(200, { "docker-distribution-api-version": "registry/2.0" }); return res.end(); }
    x = path.match(/^\/v2\/([^/]+\/[^/]+)\/(manifests|blobs)\/([^/]+)$/);
    if (x) {
      const m = await model(x[1]); if (!m) return J(res, 404, { errors: [{ code: "NAME_UNKNOWN", message: `${x[1]} is not indexed` }] });
      if (x[2] === "manifests") {
        const tag = x[3], d = tag.startsWith("sha256:") ? tag : tag === "index" || tag === m.rev ? m.index : m.manifests[{ sharded: "safetensors-sharded", latest: "safetensors", main: "safetensors" }[tag] || tag] || m.manifests.original;
        const b = await obj(m.id, d), type = JSON.parse(b.toString("utf8")).mediaType;
        res.writeHead(200, { "content-type": type, "content-length": b.length, "docker-content-digest": d }); return res.end(req.method === "HEAD" ? undefined : b);
      }
      const f = m.files.find((y) => y.digest === x[3]) || await (async () => { for (const md of Object.values(m.manifests)) for (const l of (await json(m.id, md)).layers) if (l.digest === x[3]) return { path: l.annotations["org.opencontainers.image.title"], digest: l.digest, size: l.size, layout: l.annotations["org.hologram.layout"] }; return null; })();
      if (!f) {                                                                // a held object (config, table): checked by its digest
        const b = await obj(m.id, x[3]).catch(() => null); if (!b) return J(res, 404, { errors: [{ code: "BLOB_UNKNOWN", message: x[3] }] });
        res.writeHead(200, { "content-type": "application/octet-stream", "content-length": b.length, "docker-content-digest": x[3] }); return res.end(req.method === "HEAD" ? undefined : b);
      }
      if (/^ollama/i.test(req.headers["user-agent"] || "") && req.method === "GET") {
        const host = String(req.headers.host), other = host.startsWith("localhost") ? host.replace("localhost", "127.0.0.1") : host.replace(/^[^:]+/, "localhost");
        res.writeHead(307, { location: `http://${other}/_blob/${x[1]}/${f.digest}`, "content-length": 0 }); return res.end();
      }
      return send(req, res, m, f, { "docker-content-digest": f.digest });
    }
    x = path.match(/^\/_blob\/([^/]+\/[^/]+)\/(sha256:[0-9a-f]{64})$/);
    if (x) { const m = await model(x[1]); const f = m?.files.find((y) => y.digest === x[2]); if (!f) return nf(res, "BlobUnknown", x[2]); return send(req, res, m, f, { "docker-content-digest": f.digest }); }
    return nf(res, "NotFound", "not part of the edge");
  } catch (e) { log({ url: req.url, error: e.message }); if (!res.headersSent) J(res, 502, { error: e.message }); else res.destroy(); }
}).listen(PORT, "127.0.0.1", () => log({ edge: `http://127.0.0.1:${PORT}`, hub: HUB, ipfs: IPFS }));
