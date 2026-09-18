#!/usr/bin/env node
// hub-resolve: the Hugging Face dialect of the Model Hub, so HF_ENDPOINT=https://hub.uor.foundation works with every
// tool that downloads models. It answers metadata from the hub's published file lists and sends every file request to
// a source that is alive, as a redirect: no weight byte passes through this process. One file, no dependencies.
//
// Recorded from huggingface_hub 1.32 (web/qa/hf-dialect/recorder.mjs), the whole dialect a download needs:
//   GET  /api/models?search=&author=&pipeline_tag=&library=&filter=&sort=&limit=   list and search (HfApi.list_models)
//   GET  [/via/<source>]/api/models/<org>/<name>[/revision/<rev>]          model info: sha, siblings
//   GET  [/via/<source>]/api/models/<org>/<name>/tree/<rev>[/<dir>]        file listing
//   HEAD [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 + X-Repo-Commit, X-Linked-ETag, X-Linked-Size
//   GET  [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 to the chosen source (Range is re-sent there)
//   GET  …/resolve/<rev>/SHA256SUMS                                        generated: `sha256sum -c` checks a download
//   GET  …/api/models/<org>/<name>/xet-read-token/<rev>                    307 to huggingface.co (its token, not ours)
//   GET  /api/models/<org>/<name>/refs                                     branches: main at the indexed revision (llama.cpp -hf)
//   GET|HEAD /v2/<org>/<name>/manifests/<quant> and /blobs/<digest>        Ollama's registry dialect (see below)
//   POST /mcp                                                              MCP for agents: search_models, get_model, resolve_file
// Older clients read those three headers from our 302 and GET the Location; huggingface_hub 1.32 follows the redirect
// on HEAD too, so while Hugging Face is the source it meets Hugging Face's own Xet headers and downloads through Xet
// (hence the token route). We never send an X-Xet-* header ourselves: from ModelScope and IPFS the client uses plain
// HTTP. Authorization is stripped by the front Caddy and never read here. `/via/ipfs`, `/via/modelscope.cn`, `/via/huggingface.co` pin the first choice of source.
//
// The source for a file: one that has the file and passed the last health probe, in the order Hugging Face (its CDN
// is the fastest), ModelScope, IPFS. Probes run in the background; a request never waits for one.
import http from "node:http";
import { createHash } from "node:crypto";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

const DATA = process.env.HUB_DATA || "/data";          // the site's published data: files/<org>/<name>.json, models.json
const STATE = process.env.HUB_STATE || "/state";        // requested.txt (models asked for but not indexed), override.json
const PORT = Number(process.env.PORT || 8090);
const ORDER = ["huggingface.co", "modelscope.cn", "ipfs"];
const PROBE = { model: "sentence-transformers/all-MiniLM-L6-v2", file: "config.json" };

// ---- the index on disk (re-read when the daily build swaps it)
let byLower = new Map(), loadedAt = 0;
function refresh() {
  if (Date.now() - loadedAt < 60_000) return;
  loadedAt = Date.now();
  const map = new Map(), root = join(DATA, "files");
  if (!existsSync(root)) return;
  for (const org of readdirSync(root)) for (const f of readdirSync(join(root, org))) if (f.endsWith(".json")) map.set(`${org}/${f.slice(0, -5)}`.toLowerCase(), join(root, org, f));
  byLower = map;
}
async function model(id) {
  refresh();
  const path = byLower.get(id.toLowerCase());
  if (!path) return indexed(id);
  const doc = JSON.parse(await readFile(path, "utf8"));
  doc.id = path.slice(join(DATA, "files").length + 1, -5).replace(/\\/g, "/");
  return doc;
}

// The site publishes file lists for the models it shows (the trending ones). Every other model of the address index,
// the pinned ones included, is read from the index itself and kept for ten minutes.
const API = process.env.HOLOGRAM_API || "https://humuhumu33.github.io/hologram-api";
const PINS = process.env.HUB_PINS || "https://hub.uor.foundation/pins.json";
const remote = new Map();
let pins = { at: 0, doc: { models: {} } };
async function indexed(id) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(id)) return null;
  const hit = remote.get(id.toLowerCase());
  if (hit && Date.now() - hit.at < 600_000) return hit.doc;
  let doc = null;
  try {
    const r = await fetch(`${API}/v1/huggingface.co/${id}/latest.json`, { signal: AbortSignal.timeout(8000) });
    if (r.ok) {
      const index = await r.json();
      if (Date.now() - pins.at > 600_000) pins = { at: Date.now(), doc: await fetch(PINS, { signal: AbortSignal.timeout(8000) }).then((p) => p.json()).catch(() => pins.doc) };
      const pin = Object.entries(pins.doc.models || {}).find(([k, v]) => k.toLowerCase() === id.toLowerCase() && v.revision === index.revision)?.[1];
      const sources = [{ kind: "huggingface.co", resolve: null, missing: [] }];
      if (pin) sources.push({ kind: "ipfs", resolve: `${pins.doc.gateway}${pin.root}/`, missing: pin.hidden ? [] : index.files.map((f) => f.path).filter((p) => p.split("/").some((part) => part.startsWith("."))) });
      doc = { id: index.name || id, revision: index.revision, manifest: index.manifest, sources, files: index.files.map((f) => [f.path, f.size, f.address, f.weights ? 1 : 0, f.url]) };
    }
  } catch { /* the index did not answer: treat as unknown for now */ }
  if (remote.size > 500) remote.clear();
  remote.set(id.toLowerCase(), { at: Date.now(), doc });
  return doc;
}

// ---- health: one small verified fetch per source, in the background
const health = Object.fromEntries(ORDER.map((k) => [k, { ok: true, checked: null, reason: "not probed yet" }]));
const encodePath = (p) => p.split("/").map(encodeURIComponent).join("/");
const urlFor = (doc, source, [path, , , , hfUrl]) => (source.kind === "huggingface.co" ? hfUrl : source.resolve + encodePath(path));
async function probe() {
  const doc = await model(PROBE.model).catch(() => null);
  let forced = [];
  try { forced = JSON.parse(await readFile(join(STATE, "override.json"), "utf8")).down || []; } catch { /* no drill running */ }
  for (const kind of ORDER) {
    const source = doc?.sources.find((s) => s.kind === kind), file = doc?.files.find((f) => f[0] === PROBE.file);
    const was = health[kind].ok;
    if (forced.includes(kind)) health[kind] = { ok: false, checked: new Date().toISOString(), reason: "marked down for a drill" };
    else if (!source || !file) health[kind] = { ok: true, checked: null, reason: "no probe file on this source" };
    else {
      try {
        const r = await fetch(urlFor(doc, source, file), { signal: AbortSignal.timeout(kind === "ipfs" ? 20_000 : 8_000), headers: { "user-agent": "hub-resolve probe" } });
        if (!r.ok) throw new Error(`answered ${r.status}`);
        const got = `sha256:${createHash("sha256").update(Buffer.from(await r.arrayBuffer())).digest("hex")}`;
        if (got !== file[2]) throw new Error("served different bytes");
        health[kind] = { ok: true, checked: new Date().toISOString(), reason: "verified" };
      } catch (e) { health[kind] = { ok: false, checked: new Date().toISOString(), reason: String(e.message || e).slice(0, 80) }; }
    }
    if (was !== health[kind].ok) console.log(JSON.stringify({ t: new Date().toISOString(), health: kind, ok: health[kind].ok, reason: health[kind].reason }));
  }
}
probe(); setInterval(probe, 30_000).unref();

function choose(doc, file, via) {
  const have = doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind) && !(s.missing || []).includes(file[0]));
  have.sort((a, b) => ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind));
  const pinned = via && have.find((s) => s.kind === via);
  if (pinned) return { source: pinned, reason: "asked for" };
  const alive = have.find((s) => health[s.kind].ok);
  if (alive) return { source: alive, reason: alive === have[0] ? "first choice" : `${have[0].kind} is down` };
  return { source: have[0], reason: "no source passed the last probe" };
}

// ---- answers
const json = (res, status, body, headers = {}) => { const text = JSON.stringify(body); res.writeHead(status, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(text), "cache-control": "public, max-age=60", ...headers }); res.end(res.req.method === "HEAD" ? undefined : text); };
const refuse = (res, status, code, message) => json(res, status, { error: message }, { "x-error-code": code, "x-error-message": message });
const hex = (address) => address.split(":")[1];
const isLfs = (f) => Boolean(f[3]);

async function missing(res, id) {
  let gated = false;
  try { gated = JSON.parse(await readFile(join(DATA, "models.json"), "utf8")).models.some((m) => m.id.toLowerCase() === id.toLowerCase() && m.state === "skipped"); } catch { /* no catalog */ }
  if (gated) return refuse(res, 403, "GatedRepo", `${id} is gated on Hugging Face. The hub serves public models only; use huggingface.co directly for this one.`);
  if (/^[\w.-]+\/[\w.-]+$/.test(id)) appendFile(join(STATE, "requested.txt"), `${new Date().toISOString().slice(0, 10)} ${id}\n`).catch(() => {});
  return refuse(res, 404, "RepoNotFound", `${id} is not in the Hologram index yet. The request was recorded for the next index run; use huggingface.co directly meanwhile.`);
}
// The catalog the site shows, in the shape Hugging Face's list route answers. A search costs an agent a few hundred
// bytes instead of the whole catalog. Only models whose files are addressed are listed.
let catalog = { at: 0, rows: [] };
async function rows() {
  if (Date.now() - catalog.at > 60_000) {
    try {
      const doc = JSON.parse(await readFile(join(DATA, "models.json"), "utf8"));
      catalog = { at: Date.now(), rows: doc.models.filter((m) => m.state === "addressed").map((m) => ({
        _id: hex(m.manifest).slice(0, 24), id: m.id, modelId: m.id, author: m.org, sha: m.revision, private: false, gated: false, disabled: false,
        likes: m.likes || 0, downloads: m.downloads || 0, trendingScore: Math.max(0, 501 - (m.rank || 501)), createdAt: m.created ? `${m.created}T00:00:00.000Z` : undefined,
        pipeline_tag: m.task || undefined, library_name: m.library || undefined,
        tags: [m.task, m.library, m.format && m.format.toLowerCase(), m.arch, m.license && `license:${m.license}`, ...(m.languages || [])].filter(Boolean),
        hologram: { manifest: m.manifest, files: m.files, weight_bytes: m.weightBytes, parameters: m.params || undefined, context: m.context || undefined, sources: m.sources },
      })) };
    } catch { catalog.at = Date.now(); }
  }
  return catalog.rows;
}
const SORTS = { downloads: "downloads", likes: "likes", trendingScore: "trendingScore", trending_score: "trendingScore", createdAt: "createdAt", created_at: "createdAt" };
async function list(res, q) {
  const has = (v, needle) => String(v || "").toLowerCase().includes(needle.toLowerCase());
  let out = await rows();
  const search = q.get("search"), author = q.get("author"), task = q.get("pipeline_tag"), library = q.get("library");
  if (search) out = out.filter((m) => has(m.id, search));
  if (author) out = out.filter((m) => m.author.toLowerCase() === author.toLowerCase());
  if (task) out = out.filter((m) => m.pipeline_tag === task);
  if (library) out = out.filter((m) => m.library_name === library);
  for (const tag of q.getAll("filter").flatMap((f) => f.split(","))) out = out.filter((m) => m.tags.some((t) => t.toLowerCase() === tag.toLowerCase()));
  const key = SORTS[q.get("sort") || "trendingScore"] || "trendingScore", up = q.get("direction") === "1";
  out = [...out].sort((a, b) => (a[key] > b[key] ? 1 : a[key] < b[key] ? -1 : 0) * (up ? 1 : -1));
  const limit = Math.min(500, Math.max(1, Number(q.get("limit")) || 50));
  return json(res, 200, out.slice(0, limit), { "access-control-allow-origin": "*", "x-total-count": String(out.length) });
}

// ---- Ollama's registry dialect: `ollama pull hub.uor.foundation/<org>/<name>:<quant>`
// From Ollama's source (server/images.go, download.go): it GETs a Docker v2 manifest, HEADs each blob for its size
// (we answer 200 directly: a cross-host redirect on HEAD is refused by newer Ollama), GETs each blob expecting 307 to
// another host or 200 with a Location header, downloads the target in parallel Range parts, and verifies every
// blob's SHA-256 itself. The model layer's digest is the GGUF file's SHA-256, which is exactly what the index holds,
// so the weights are one redirect to a live source and the client proves the bytes.
// The small layers (config, chat template, params) are metadata. Hugging Face already derives them for every GGUF
// repository; we take its manifest only when its model digest is a file of OUR index at the pinned revision, keep the
// small blobs (each checked against its digest), and fall back to a minimal manifest when Hugging Face is away.
const MANIFEST_TYPE = "application/vnd.docker.distribution.manifest.v2+json";
const small = new Map();      // digest → Buffer: config, template, params. Never weights (1 MiB ceiling).
const manifests = new Map();  // "<id>:<tag>" → { at, bytes }
const sha = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const ociError = (res, status, code, message) => json(res, status, { errors: [{ code, message }] }, { "cache-control": "no-store" });
const quantOf = (p) => (p.match(/(?:^|[-._])((?:I?Q\d\w*|BF16|F16|F32|MXFP4\w*))\.gguf$/i) || [])[1] || "";
const cacheFile = (digest) => join(STATE, "ollama", digest.replace(":", "-"));
async function keep(bytes) {
  const digest = sha(bytes);
  small.set(digest, bytes);
  await mkdir(join(STATE, "ollama"), { recursive: true }).catch(() => {});
  await writeFile(cacheFile(digest), bytes).catch(() => {});
  return digest;
}
async function smallBlob(id, digest) {
  if (small.has(digest)) return small.get(digest);
  try { const bytes = await readFile(cacheFile(digest)); if (sha(bytes) === digest) { small.set(digest, bytes); return bytes; } } catch { /* not kept yet */ }
  try {
    const r = await fetch(`https://hf.co/v2/${id}/blobs/${digest}`, { signal: AbortSignal.timeout(8000), headers: { "user-agent": "ollama/0.0 (hub-resolve)" } });
    if (!r.ok || Number(r.headers.get("content-length") || 0) > 1048576) return null;
    const bytes = Buffer.from(await r.arrayBuffer());
    if (bytes.length > 1048576 || sha(bytes) !== digest) return null; // a source never vouches for itself
    await keep(bytes);
    return bytes;
  } catch { return null; }
}
function pickGguf(doc, tag) {
  const ggufs = doc.files.filter((f) => /\.gguf$/i.test(f[0]) && !/mmproj|imatrix/i.test(f[0]));
  const whole = ggufs.filter((f) => !/-\d{5}-of-\d{5}/.test(f[0]));
  const t = tag.toLowerCase();
  const match = (list) => list.find((f) => [f[0].toLowerCase(), f[0].split("/").pop().toLowerCase()].some((n) => n === t || n === `${t}.gguf`)) || list.find((f) => quantOf(f[0]).toLowerCase() === t);
  if (t === "latest") return { file: whole.find((f) => /q4_k_m/i.test(f[0])) || whole[0], sharded: !whole.length && ggufs.length > 0, quants: whole.map((f) => quantOf(f[0])).filter(Boolean) };
  return { file: match(whole), sharded: !match(whole) && Boolean(match(ggufs)), quants: whole.map((f) => quantOf(f[0])).filter(Boolean) };
}
async function manifestFor(doc, tag, file) {
  const key = `${doc.id}:${tag}`, digest = `sha256:${hex(file[2])}`, hit = manifests.get(key);
  if (hit && Date.now() - hit.at < 600_000) return hit.bytes;
  const saved = join(STATE, "ollama", `manifest-${doc.id.replace("/", "--")}-${hex(file[2]).slice(0, 16)}.json`);
  let bytes = null;
  if (health["huggingface.co"].ok) {
    try {
      const r = await fetch(`https://hf.co/v2/${doc.id}/manifests/${encodeURIComponent(tag)}`, { signal: AbortSignal.timeout(8000), headers: { accept: MANIFEST_TYPE, "user-agent": "ollama/0.0 (hub-resolve)" } });
      if (r.ok) {
        const theirs = Buffer.from(await r.arrayBuffer()), m = JSON.parse(theirs.toString("utf8"));
        const model = (m.layers || []).find((l) => l.mediaType === "application/vnd.ollama.image.model");
        // Only if Hugging Face names the very bytes our index names, at our pinned revision.
        if (model?.digest === digest && model.size === file[1]) { bytes = theirs; await mkdir(join(STATE, "ollama"), { recursive: true }).catch(() => {}); await writeFile(saved, theirs).catch(() => {}); }
      }
    } catch { /* fall through */ }
  }
  if (!bytes) bytes = await readFile(saved).catch(() => null);
  if (!bytes) { // minimal: the weights alone. Ollama reads the chat template from the GGUF metadata when it can.
    const config = Buffer.from(JSON.stringify({ model_format: "gguf", model_family: "unknown", model_families: [], model_type: "", file_type: quantOf(file[0]) || "unknown", architecture: "amd64", os: "linux", rootfs: { type: "layers", diff_ids: [digest] } }));
    const configDigest = await keep(config);
    bytes = Buffer.from(JSON.stringify({ schemaVersion: 2, mediaType: MANIFEST_TYPE, config: { digest: configDigest, mediaType: "application/vnd.docker.container.image.v1+json", size: config.length }, layers: [{ digest, mediaType: "application/vnd.ollama.image.model", size: file[1] }] }));
  }
  manifests.set(key, { at: Date.now(), bytes });
  return bytes;
}
async function ollama(req, res, id, kind, ref) {
  const doc = await model(id);
  if (!doc) return ociError(res, 404, "NAME_UNKNOWN", `${id} is not in the Hologram index. Try: ollama pull hf.co/${id}`);
  if (kind === "manifests") {
    const { file, sharded, quants } = pickGguf(doc, ref);
    if (!file) return ociError(res, 404, "MANIFEST_UNKNOWN", sharded ? `${ref} of ${doc.id} is split across several files; Ollama needs a single GGUF file.` : quants.length ? `${doc.id} has no ${ref}. It has: ${[...new Set(quants)].join(", ")}.` : `${doc.id} has no GGUF file; Ollama pulls GGUF models.`);
    const bytes = await manifestFor(doc, ref, file);
    res.writeHead(200, { "content-type": MANIFEST_TYPE, "content-length": bytes.length, "docker-content-digest": sha(bytes), "x-repo-commit": doc.revision, "cache-control": "no-store" });
    return res.end(req.method === "HEAD" ? undefined : bytes);
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(ref)) return ociError(res, 400, "DIGEST_INVALID", "A blob is named sha256:<64 hex digits>.");
  const entry = doc.files.find((f) => f[2] === ref);
  if (entry) { // the weights: never through us
    if (req.method === "HEAD") { res.writeHead(200, { "content-length": entry[1], "docker-content-digest": ref, "accept-ranges": "bytes", "content-type": "application/octet-stream" }); return res.end(); }
    const { source, reason } = choose(doc, entry, null);
    console.log(JSON.stringify({ t: new Date().toISOString(), dialect: "ollama", model: doc.id, file: entry[0], source: source.kind, reason }));
    res.writeHead(307, { location: urlFor(doc, source, entry), "docker-content-digest": ref, "x-hub-source": source.kind, "cache-control": "no-store", "content-length": "0" });
    return res.end();
  }
  const bytes = await smallBlob(doc.id, ref);
  if (!bytes) return ociError(res, 404, "BLOB_UNKNOWN", `${ref} is not part of ${doc.id}.`);
  // Ollama asks a 200 for its Location and then reads that URL in Range parts: answer both from here.
  const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || "");
  const from = range ? Number(range[1]) : 0, to = range && range[2] ? Math.min(Number(range[2]), bytes.length - 1) : bytes.length - 1;
  const part = bytes.subarray(from, to + 1);
  res.writeHead(range ? 206 : 200, { "content-type": "application/octet-stream", "content-length": part.length, "docker-content-digest": ref, "accept-ranges": "bytes", location: `https://${req.headers["x-forwarded-host"] || req.headers.host}${req.url}`, ...(range ? { "content-range": `bytes ${from}-${to}/${bytes.length}` } : {}) });
  return res.end(req.method === "HEAD" ? undefined : part);
}

// ---- MCP: the same hub for agents, at /mcp (Streamable HTTP, stateless, anonymous, read-only)
// Three tools, because an agent needs three answers: which model, which file, and where to get it with what hash.
// Weights never travel through a tool result: resolve_file returns URLs, the expected SHA-256 (from the index, never
// from a source) and the exact commands that hand the file to an engine.
const MCP_VERSIONS = ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];
const HUB = "https://hub.uor.foundation";
const TOOLS = [
  { name: "search_models", title: "Search models",
    description: "Find open models in the hub's index. Every result has all of its files addressed by SHA-256. Returns id, task, library, licence, parameters, weight size, downloads and where the bytes live.",
    inputSchema: { type: "object", additionalProperties: false, properties: {
      query: { type: "string", description: "Part of the model id, e.g. 'qwen' or 'whisper'." },
      task: { type: "string", description: "Hugging Face pipeline tag, e.g. text-generation, text-to-speech, feature-extraction." },
      license: { type: "string", description: "SPDX-style licence id, e.g. apache-2.0, mit." },
      format: { type: "string", description: "gguf, safetensors, mlx, onnx …" },
      max_weights_gb: { type: "number", description: "Upper bound on the total size of the weights." },
      sort: { type: "string", enum: ["trending", "downloads", "likes", "newest"], default: "trending" },
      limit: { type: "integer", minimum: 1, maximum: 50, default: 10 } } } },
  { name: "get_model", title: "Get a model",
    description: "The pinned revision of one model, its files with size and SHA-256, its GGUF quantisations if any, and its sources with their current health.",
    inputSchema: { type: "object", additionalProperties: false, required: ["id"], properties: { id: { type: "string", description: "org/name, as on Hugging Face." } } } },
  { name: "resolve_file", title: "Resolve a file",
    description: "Where to download one file of a model right now, the SHA-256 it must have, how to check it, and the commands that hand it to an engine (hf, Ollama, llama.cpp). Give either a path or, for GGUF models, a quantisation such as Q4_K_M.",
    inputSchema: { type: "object", additionalProperties: false, required: ["id"], properties: { id: { type: "string" }, path: { type: "string" }, quant: { type: "string" } } } },
];
const toolError = (text) => ({ content: [{ type: "text", text }], isError: true });
const toolResult = (data) => ({ content: [{ type: "text", text: JSON.stringify(data) }], structuredContent: data });
const sourcesOf = (doc) => doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind)).map((s) => ({ kind: s.kind, healthy: health[s.kind].ok, missing_files: (s.missing || []).length }));
async function callTool(name, a = {}) {
  if (name === "search_models") {
    let out = await rows();
    const has = (v, n) => String(v || "").toLowerCase().includes(String(n).toLowerCase());
    if (a.query) out = out.filter((m) => has(m.id, a.query));
    if (a.task) out = out.filter((m) => m.pipeline_tag === a.task);
    if (a.license) out = out.filter((m) => m.tags.includes(`license:${String(a.license).toLowerCase()}`));
    if (a.format) out = out.filter((m) => m.tags.includes(String(a.format).toLowerCase()));
    if (a.max_weights_gb) out = out.filter((m) => (m.hologram.weight_bytes || 0) <= a.max_weights_gb * 1e9);
    const key = { trending: "trendingScore", downloads: "downloads", likes: "likes", newest: "createdAt" }[a.sort || "trending"] || "trendingScore";
    out = [...out].sort((x, y) => (x[key] < y[key] ? 1 : x[key] > y[key] ? -1 : 0));
    const limit = Math.min(50, Math.max(1, a.limit || 10));
    return toolResult({ total: out.length, models: out.slice(0, limit).map((m) => ({ id: m.id, task: m.pipeline_tag || null, library: m.library_name || null, license: (m.tags.find((t) => t.startsWith("license:")) || "").slice(8) || null, parameters: m.hologram.parameters || null, weights_gb: Math.round((m.hologram.weight_bytes || 0) / 1e7) / 100, downloads: m.downloads, sources: m.hologram.sources, page: `${HUB}/models/${m.id}/` })) });
  }
  const doc = a.id ? await model(String(a.id)) : null;
  if (!doc) return toolError(`${a.id || "(no id)"} is not in the hub's index. search_models lists what is; Hugging Face has the rest.`);
  const ggufs = doc.files.filter((f) => /\.gguf$/i.test(f[0]) && !/mmproj|imatrix/i.test(f[0]));
  if (name === "get_model") {
    const files = doc.files.map((f) => ({ path: f[0], size: f[1], sha256: hex(f[2]) }));
    return toolResult({ id: doc.id, revision: doc.revision, purl: `pkg:huggingface/${doc.id}@${doc.revision}`, files_total: files.length, bytes_total: files.reduce((s, f) => s + f.size, 0), files: files.slice(0, 200), files_truncated: files.length > 200, gguf_quants: [...new Set(ggufs.map((f) => quantOf(f[0])).filter(Boolean))], sources: sourcesOf(doc), sha256sums: `${HUB}/${doc.id}/resolve/main/SHA256SUMS`, download_all: [`export HF_ENDPOINT=${HUB}`, `hf download ${doc.id}`] });
  }
  if (name === "resolve_file") {
    const entry = a.path ? doc.files.find((f) => f[0] === a.path) : a.quant ? pickGguf(doc, String(a.quant)).file : null;
    if (!entry) return toolError(a.path ? `${a.path} is not a file of ${doc.id} at ${doc.revision}. get_model lists the files.` : a.quant ? `${doc.id} has no single-file GGUF for ${a.quant}. It has: ${[...new Set(ggufs.map((f) => quantOf(f[0])).filter(Boolean))].join(", ") || "no GGUF files"}.` : "Give a path or a quant.");
    const have = doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind) && !(s.missing || []).includes(entry[0]));
    const { source } = choose(doc, entry, null);
    const url = `${HUB}/${doc.id}/resolve/${doc.revision}/${encodePath(entry[0])}`, name0 = entry[0].split("/").pop(), isGguf = /\.gguf$/i.test(entry[0]);
    return toolResult({ id: doc.id, revision: doc.revision, path: entry[0], size: entry[1], sha256: hex(entry[2]),
      url, url_note: "Redirects to a source that is up right now; supports Range. No credentials needed.", served_by_now: source.kind,
      sources: have.map((s) => ({ kind: s.kind, healthy: health[s.kind].ok, url: urlFor(doc, s, entry) })),
      download: `curl -L -o ${JSON.stringify(name0)} ${JSON.stringify(url)}`, verify: `echo "${hex(entry[2])}  ${name0}" | sha256sum -c`,
      handoff: { hf: [`export HF_ENDPOINT=${HUB}`, `hf download ${doc.id} ${entry[0]}`], ...(isGguf && quantOf(entry[0]) ? { ollama: `ollama pull hub.uor.foundation/${doc.id}:${quantOf(entry[0])}`, llama_cpp: `MODEL_ENDPOINT=${HUB}/ llama-server -hf ${doc.id}:${quantOf(entry[0])}` } : {}) } });
  }
  return toolError(`Unknown tool ${name}.`);
}
async function mcp(req, res) {
  const cors = { "access-control-allow-origin": "*", "access-control-allow-headers": "content-type, accept, mcp-protocol-version, mcp-session-id, mcp-method, mcp-name, last-event-id", "access-control-allow-methods": "POST, OPTIONS", "access-control-expose-headers": "mcp-protocol-version" };
  if (req.method === "OPTIONS") { res.writeHead(204, cors); return res.end(); }
  if (req.method !== "POST") { res.writeHead(405, { allow: "POST, OPTIONS", ...cors }); return res.end(); } // no server-initiated stream
  let raw = "";
  for await (const chunk of req) { raw += chunk; if (raw.length > 65536) { res.writeHead(413, cors); return res.end(); } }
  let body;
  try { body = JSON.parse(raw); } catch { return json(res, 400, { jsonrpc: "2.0", id: null, error: { code: -32700, message: "Parse error" } }, cors); }
  const version = MCP_VERSIONS.includes(req.headers["mcp-protocol-version"]) ? req.headers["mcp-protocol-version"] : null;
  const info = { name: "hologram-model-hub", title: "Hologram Model Hub", version: "1.0.0" };
  const instructions = "Open models, every file named by its SHA-256. search_models to find one, get_model for its files, resolve_file for a download URL with the hash it must have. Weights are fetched by your shell or engine, never through a tool result.";
  const one = async (m) => {
    if (!m || m.jsonrpc !== "2.0" || typeof m.method !== "string") return { jsonrpc: "2.0", id: m?.id ?? null, error: { code: -32600, message: "Invalid request" } };
    if (m.id === undefined) return null; // a notification: nothing to answer
    const ok = (result) => ({ jsonrpc: "2.0", id: m.id, result });
    if (m.method === "initialize") return ok({ protocolVersion: MCP_VERSIONS.includes(m.params?.protocolVersion) ? m.params.protocolVersion : "2025-06-18", capabilities: { tools: { listChanged: false } }, serverInfo: info, instructions });
    if (m.method === "server/discover") return ok({ protocolVersions: MCP_VERSIONS, capabilities: { tools: { listChanged: false } }, serverInfo: info, instructions });
    if (m.method === "ping") return ok({});
    if (m.method === "tools/list") return ok({ tools: TOOLS });
    if (m.method === "tools/call") { try { return ok(await callTool(m.params?.name, m.params?.arguments)); } catch (e) { return ok(toolError(`The hub failed on this call: ${String(e.message || e).slice(0, 120)}`)); } }
    if (m.method === "resources/list") return ok({ resources: [] });
    if (m.method === "prompts/list") return ok({ prompts: [] });
    return { jsonrpc: "2.0", id: m.id, error: { code: -32601, message: `Method not found: ${m.method}` } };
  };
  const answers = (await Promise.all((Array.isArray(body) ? body : [body]).map(one))).filter(Boolean);
  if (!answers.length) { res.writeHead(202, cors); return res.end(); }
  return json(res, 200, Array.isArray(body) ? answers : answers[0], { ...cors, "cache-control": "no-store", ...(version ? { "mcp-protocol-version": version } : {}) });
}

const revisionOk = (doc, rev) => rev === "main" || (rev.length >= 7 && doc.revision.startsWith(rev));

http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://hub");
    if (url.pathname === "/mcp") return mcp(req, res);
    if (req.method !== "GET" && req.method !== "HEAD") return refuse(res, 405, "ReadOnly", "The hub endpoint is read-only.");
    let path = decodeURIComponent(url.pathname), via = url.searchParams.get("source");
    const prefix = path.match(/^\/via\/([a-z.]+)(\/.*)$/);
    if (prefix) { via = prefix[1] === "modelscope" ? "modelscope.cn" : prefix[1] === "huggingface" ? "huggingface.co" : prefix[1]; path = prefix[2]; }

    const oci = path.match(/^\/v2\/([^/]+\/[^/]+)\/(manifests|blobs)\/(.+)$/);
    if (oci) return ollama(req, res, oci[1], oci[2], oci[3]);
    if (path === "/api/models" || path === "/api/models/") return list(res, url.searchParams);
    if (path === "/api/hub/health") return json(res, 200, { sources: health, order: ORDER }, { "cache-control": "no-store", "access-control-allow-origin": "*" });

    // Measured with huggingface_hub 1.32: the client follows our redirect on HEAD, meets Hugging Face's Xet headers
    // there, and then asks *this* endpoint for the Xet read token. The token is Hugging Face's to give: send the
    // client there. (Other sources carry no Xet headers, so this only happens while Hugging Face is the source.)
    const xet = path.match(/^\/api\/models\/[^/]+\/[^/]+\/xet-read-token\/[^/]+$/);
    if (xet) { res.writeHead(307, { location: `https://huggingface.co${path}`, "cache-control": "no-store", "content-length": "0" }); return res.end(); }

    const refs = path.match(/^\/api\/models\/([^/]+\/[^/]+)\/refs$/);
    if (refs) {
      const doc = await model(refs[1]);
      if (!doc) return missing(res, refs[1]);
      return json(res, 200, { branches: [{ name: "main", ref: "refs/heads/main", targetCommit: doc.revision }], tags: [], converts: [] });
    }
    const info = path.match(/^\/api\/models\/([^/]+\/[^/]+?)(?:\/revision\/(.+))?$/), tree = path.match(/^\/api\/models\/([^/]+\/[^/]+)\/tree\/([^/]+)(?:\/(.*))?$/);
    if (tree) {
      const doc = await model(tree[1]);
      if (!doc) return missing(res, tree[1]);
      if (!revisionOk(doc, tree[2])) return refuse(res, 404, "RevisionNotFound", `The hub has ${doc.id} at ${doc.revision} only.`);
      const dir = tree[3] ? `${tree[3].replace(/\/$/, "")}/` : "";
      return json(res, 200, doc.files.filter((f) => f[0].startsWith(dir)).map((f) => ({ type: "file", oid: hex(f[2]), size: f[1], path: f[0], ...(isLfs(f) ? { lfs: { oid: hex(f[2]), size: f[1], pointerSize: 0 } } : {}) })));
    }
    if (info) {
      const doc = await model(info[1]);
      if (!doc) return missing(res, info[1]);
      if (info[2] && !revisionOk(doc, info[2])) return refuse(res, 404, "RevisionNotFound", `The hub has ${doc.id} at ${doc.revision} only.`);
      return json(res, 200, { _id: hex(doc.manifest).slice(0, 24), id: doc.id, modelId: doc.id, sha: doc.revision, private: false, gated: false, disabled: false, tags: [], downloads: 0, likes: 0, siblings: doc.files.map((f) => ({ rfilename: f[0] })) });
    }

    const file = path.match(/^\/([^/]+\/[^/]+)\/resolve\/([^/]+)\/(.+)$/);
    if (file && file[1] !== "api/models") {
      const doc = await model(file[1]);
      if (!doc) return missing(res, file[1]);
      if (!revisionOk(doc, file[2])) return refuse(res, 404, "RevisionNotFound", `The hub has ${doc.id} at ${doc.revision} only.`);
      const entry = doc.files.find((f) => f[0] === file[3]);
      if (!entry && file[3] === "SHA256SUMS") {
        const text = doc.files.map((f) => `${hex(f[2])}  ${f[0]}\n`).join("");
        res.writeHead(200, { "content-type": "text/plain; charset=utf-8", "content-length": Buffer.byteLength(text), "x-repo-commit": doc.revision, etag: `"${createHash("sha256").update(text).digest("hex")}"` });
        return res.end(req.method === "HEAD" ? undefined : text);
      }
      if (!entry) return refuse(res, 404, "EntryNotFound", `${file[3]} is not in ${doc.id} at ${doc.revision}.`);
      const { source, reason } = choose(doc, entry, via);
      const location = urlFor(doc, source, entry);
      if (req.method === "GET" || reason !== "first choice") console.log(JSON.stringify({ t: new Date().toISOString(), model: doc.id, file: entry[0], method: req.method, source: source.kind, reason }));
      res.writeHead(302, { location, "x-repo-commit": doc.revision, "x-linked-etag": `"${hex(entry[2])}"`, "x-linked-size": String(entry[1]), etag: `"${hex(entry[2])}"`, "accept-ranges": "bytes", "x-hub-source": source.kind, "cache-control": "no-store", "content-length": "0" });
      return res.end();
    }
    return refuse(res, 404, "NotFound", "Not part of the hub endpoint.");
  } catch (error) {
    console.error(error);
    return refuse(res, 500, "HubError", "The hub endpoint failed on this request.");
  }
}).listen(PORT, () => console.log(JSON.stringify({ t: new Date().toISOString(), listening: PORT, data: DATA })));
