#!/usr/bin/env node
// hub-resolve: the Hugging Face dialect of the Model Hub, so HF_ENDPOINT=https://gethologram.ai works with every
// tool that downloads models. It answers metadata from the hub's published file lists and sends every file request to
// a source that is alive, as a redirect: no weight byte passes through this process. One file, no dependencies.
//
// Recorded from huggingface_hub 1.32 (web/qa/hf-dialect/recorder.mjs), the whole dialect a download needs:
//   GET  /api/models?search=&author=&pipeline_tag=&library=&filter=&sort=&limit=   list and search (HfApi.list_models)
//   GET  [/via/<source>]/api/models/<org>/<name>[/revision/<rev>]          model info: sha, siblings
//   GET  [/via/<source>]/api/models/<org>/<name>/tree/<rev>[/<dir>]        file listing (?recursive=; any ?cursor= page is [])
//   GET  [/via/<source>]/api/models/<org>/<name>/treesize/<rev>[/<dir>]    total bytes under a folder (hfd.sh)
//   HEAD [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 + X-Repo-Commit, X-Linked-ETag, X-Linked-Size
//   GET  [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 to the chosen source (Range is re-sent there)
//   GET  …/resolve/<rev>/SHA256SUMS                                        generated: `sha256sum -c` checks a download
//   GET  …/api/models/<org>/<name>/xet-read-token/<rev>                    307 to huggingface.co (its token, not ours)
//   GET  /api/models/<org>/<name>/refs                                     branches: main at the indexed revision (llama.cpp -hf)
//   GET|HEAD /v2/<org>/<name>/manifests/<quant> and /blobs/<digest>        Ollama's registry dialect (see below)
//   (same routes, Accept: application/vnd.oci.image.manifest.v1+json)      OCI model artifacts, CNCF ModelPack: oras, modctl, Docker Model Runner
//   GET  /api/v1/models/<org>/<name>/repo/files, /repo?FilePath=, /revisions   ModelScope's dialect (MODELSCOPE_ENDPOINT)
//   POST /<org>/<name>.git/info/lfs/objects/batch                         Git LFS batch API, download only (lfs.url)
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
import { homedir } from "node:os";
import { kappaMirror } from "./kappa-mirror.mjs";

// HUB_VERIFY=1: run on the user's own machine as a verifying edge. Same dialects, same index; instead of redirecting
// a client to a holder, fetch the file, check it against the index's sha256 and only then hand it over
// (verified-bytes.mjs). Point any client here: HF_ENDPOINT=http://127.0.0.1:8090, ollama pull 127.0.0.1:8090/<repo>:<quant>.
const VERIFY = process.env.HUB_VERIFY === "1";
// Loaded only in that mode: the hosted endpoint redirects and never needs it (its installer swaps this one file).
const { streamVerified } = VERIFY ? await import("./verified-bytes.mjs") : {};
const CACHE = process.env.HUB_CACHE || join(homedir(), ".cache", "hologram");
const DATA = process.env.HUB_DATA || "/data";          // the site's published data: files/<org>/<name>.json, models.json
const STATE = process.env.HUB_STATE || (VERIFY ? join(CACHE, "state") : "/state"); // requested.txt, override.json
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
  if (!path) return (await indexed(id)) || (await fromHub(id));
  const doc = JSON.parse(await readFile(path, "utf8"));
  doc.id = path.slice(join(DATA, "files").length + 1, -5).replace(/\\/g, "/");
  return doc;
}

// ---- the hub's own catalog, as the last word on what this hub has
//
// `objects` in the published catalog is cumulative: a model addressed on any past day keeps its model object for
// ever. `models` is only today's trending rows. So a model that drops off the list keeps a published object full
// of file hashes and loses every way of being found -- and the dialects, asking a third-party address index that
// has moved on, answered 404 for something this hub was still holding. A third of the hub was in that state.
//
// The catalog is fetched by address, so it is immutable and cacheable; only the descriptor that names today's
// catalog is mutable, and that is one small file.
let hub = { at: 0, objects: new Map(), extra: [] };
async function hubCatalog() {
  if (Date.now() - hub.at < 600_000) return hub;
  hub.at = Date.now();
  try {
    const d = await (await fetch(`${HUB}/.well-known/model-hub.json`, { signal: AbortSignal.timeout(8000) })).json();
    const cat = await (await fetch(`${HUB}/api/v1/objects/${d.catalog}`, { signal: AbortSignal.timeout(20_000) })).json();

    hub = {
      at: Date.now(),
      objects: new Map(Object.entries(cat.objects || {}).map(([id, e]) => [id.toLowerCase(), { id, ...e }])),
      // Rows for the models the browse list has forgotten. The facts a browse row carries -- task, parameters,
      // downloads -- were never published with the object, so these carry only what is certain: the name. They are
      // flagged, so a client can tell a thin row from a full one rather than inferring it from empty fields.
      // A thin row for every object. rows() drops the ones that already have a full row; doing that here instead
      // would miss a model that appears in today's catalog without being addressed, which falls through both.
      extra: Object.keys(cat.objects || {}).map((id) => ({
        _id: "", id, modelId: id, author: id.split("/")[0], sha: "", private: false, gated: false, disabled: false,
        likes: 0, downloads: 0, trendingScore: 0, tags: [],
        hologram: { manifest: cat.objects[id].model, listed: false },
      })),
    };
  } catch { /* the hub did not answer itself: keep whatever was cached */ }
  return hub;
}

// Build the internal document from the hub's own published model object: the revision and every file hash, which
// is all a redirect needs. IPFS is offered when the pin matches the same revision, exactly as elsewhere.
async function fromHub(id) {
  const { objects } = await hubCatalog();
  const entry = objects.get(id.toLowerCase());
  if (!entry) return null;
  const hit = remote.get(`hub:${id.toLowerCase()}`);
  if (hit && Date.now() - hit.at < 600_000) return hit.doc;
  let doc = null;
  try {
    const obj = await (await fetch(`${HUB}/api/v1/objects/${entry.model}`, { signal: AbortSignal.timeout(15_000) })).json();
    if (obj && Array.isArray(obj.files)) {
      if (Date.now() - pins.at > 600_000) pins = { at: Date.now(), doc: await fetch(PINS, { signal: AbortSignal.timeout(8000) }).then((p) => p.json()).catch(() => pins.doc) };
      const pin = Object.entries(pins.doc.models || {}).find(([k, v]) => k.toLowerCase() === id.toLowerCase() && v.revision === obj.revision)?.[1];
      const sources = [{ kind: "huggingface.co", resolve: null, missing: [] }];
      if (pin) sources.push({ kind: "ipfs", resolve: `${pins.doc.gateway}${pin.root}/`, missing: [] });
      doc = {
        id: entry.id, revision: obj.revision, manifest: obj.index_manifest || entry.model, sources,
        files: obj.files.map((f) => [f.path, f.size, f.sha256.startsWith("sha256:") ? f.sha256 : `sha256:${f.sha256}`, f.weights ? 1 : 0,
          `https://huggingface.co/${entry.id}/resolve/${obj.revision}/${f.path.split("/").map(encodeURIComponent).join("/")}`]),
      };
    }
  } catch { /* the object did not answer: unknown for now */ }
  if (remote.size > 500) remote.clear();
  remote.set(`hub:${id.toLowerCase()}`, { at: Date.now(), doc });
  return doc;
}

// The site publishes file lists for the models it shows (the trending ones). Every other model of the address index,
// the pinned ones included, is read from the index itself and kept for ten minutes.
const API = process.env.HOLOGRAM_API || "https://humuhumu33.github.io/hologram-api";
const HUB = (process.env.HUB || "https://gethologram.ai").replace(/\/$/, "");  // the hub asking itself: its own object plane is the backstop under every dialect
const HUB_HOST = new URL(HUB).host;
const PINS = process.env.HUB_PINS || `${HUB}/pins.json`;
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
      doc = { id: index.name || id, revision: index.revision, manifest: index.manifest, sources, files: index.files.map((f) => [f.path, f.size, f.address, f.weights ? 1 : 0, f.url, f.hub_etag?.startsWith("gitsha1:") ? f.hub_etag.slice(8) : undefined]) };
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

// Choosing a source. Without a pin the hub picks the first healthy one in ORDER; with a pin it either honours it
// or refuses.
//
// It used to fall back. A caller who asked for IPFS and got Hugging Face was told only by a response header, which
// meant the one obvious use of a pin -- fetch the same file through two sources and compare -- quietly degenerated
// into comparing one host with itself. A pin that silently does the opposite of what was asked is worse than an
// error, so an unavailable or unknown source is now a refusal with a sentence naming what would have worked.
function choose(doc, file, via) {
  const have = doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind) && !(s.missing || []).includes(file[0]));
  have.sort((a, b) => ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind));
  if (via) {
    const offer = have.length ? have.map((s) => s.kind).join(", ") : "no source in this hub's index";
    if (!ORDER.includes(via)) return { denied: { code: "UnknownSource", message: `${via} is not a source this hub knows. It has ${ORDER.join(", ")}; for this file: ${offer}.` } };
    const pinned = have.find((s) => s.kind === via);
    if (!pinned) return { denied: { code: "SourceHasNotGotIt", message: `${via} does not hold ${file[0]} of ${doc.id}. This file is on: ${offer}. Drop the /via/ prefix to let the hub choose.` } };
    return { source: pinned, reason: "asked for" };
  }
  const alive = have.find((s) => health[s.kind].ok);
  if (alive) return { source: alive, reason: alive === have[0] ? "first choice" : `${have[0].kind} is down` };
  return { source: have[0], reason: "no source passed the last probe" };
}

// ---- answers
const json = (res, status, body, headers = {}) => { const text = JSON.stringify(body); res.writeHead(status, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(text), "cache-control": "public, max-age=60", ...headers }); res.end(res.req.method === "HEAD" ? undefined : text); };
const refuse = (res, status, code, message) => json(res, status, { error: message }, { "x-error-code": code, "x-error-message": message });
const hex = (address) => address.split(":")[1];
const isLfs = (f) => Boolean(f[3]);
// An LFS file sits in git as a pointer whose text is fixed by the file's sha256 and size, so Hugging Face's `oid` for it
// (git's blob sha1 of the pointer) and `pointerSize` follow from what the index holds, exactly, with no bytes.
function lfsPointer(f) {
  const text = `version https://git-lfs.github.com/spec/v1
oid sha256:${hex(f[2])}
size ${f[1]}
`;
  return { oid: createHash("sha1").update(`blob ${Buffer.byteLength(text)} ${text}`).digest("hex"), pointerSize: Buffer.byteLength(text) };
}

// Hugging Face's tree listing of the files under `dir`: the direct children (files and folders), or with `recursive`
// every file and every folder beneath. A folder's oid is sha1 over its files' paths and addresses: stable and
// distinct, but not git's tree sha1 (the hub holds file sha256s, not git objects). File oids are Hugging Face's exactly.
// Hugging Face's `oid` for a file kept in git (not LFS) is git's blob sha1, and `hf cache verify` checks it that way
// (measured with huggingface_hub 2.0: our sha256 there failed every small file). The index holds sha256 only, so the
// sha1 is computed once per file from bytes that first matched the index's sha256, and kept by that sha256 for ever
// (content in, content out: it never goes stale). The address index already carries the sha1 it checked the bytes
// against (`hub_etag`), so this only runs for models known from elsewhere. Until a sha1 is known the oid is the sha256.
const gitOid = new Map();
let gitOidsRead = false;
async function gitOids(doc, files) {
  const store = join(STATE, "git-oids.jsonl");
  if (!gitOidsRead) {
    gitOidsRead = true;
    try { for (const l of (await readFile(store, "utf8")).split("\n")) if (l) { const [s, g] = JSON.parse(l); gitOid.set(s, g); } } catch { /* first run */ }
  }
  const todo = files.filter((f) => !isLfs(f) && !f[5] && f[1] <= 64 * 1048576 && !gitOid.has(f[2]));
  const one = async (f) => {
    for (const kind of ORDER) {
      const source = doc.sources?.find((s) => s.kind === kind && !s.p2p && !(s.missing || []).includes(f[0]));
      if (!source || !health[kind].ok || (kind === "huggingface.co" && !f[4])) continue;
      try {
        const r = await fetch(urlFor(doc, source, f), { signal: AbortSignal.timeout(10_000), headers: { "user-agent": "hub-resolve git-oid" } });
        if (!r.ok) continue;
        const bytes = Buffer.from(await r.arrayBuffer());
        if (sha(bytes) !== f[2]) continue;                                // a source never vouches for itself
        const g = createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
        gitOid.set(f[2], g);
        await mkdir(STATE, { recursive: true }).catch(() => {});
        await appendFile(store, JSON.stringify([f[2], g]) + "\n").catch(() => {});
        return;
      } catch { /* next source */ }
    }
  };
  const work = (async () => { for (let i = 0; i < todo.length; i += 8) await Promise.all(todo.slice(i, i + 8).map(one)); })();
  await Promise.race([work, new Promise((r) => setTimeout(r, 12_000))]);  // a slow source costs one answer, not the listing
}

function listing(files, dir, recursive) {
  const out = [], dirs = new Map();
  for (const f of files) {
    const parts = f[0].slice(dir.length).split("/");
    for (let i = 1; i < parts.length && (recursive || i === 1); i++) {
      const d = dir + parts.slice(0, i).join("/");
      if (!dirs.has(d)) dirs.set(d, createHash("sha1"));
      dirs.get(d).update(`${f[0]} ${f[2]}\n`);
    }
    if (!recursive && parts.length > 1) continue;
    if (isLfs(f)) { const p = lfsPointer(f); out.push({ type: "file", oid: p.oid, size: f[1], path: f[0], lfs: { oid: hex(f[2]), size: f[1], pointerSize: p.pointerSize } }); }
    else out.push({ type: "file", oid: f[5] || gitOid.get(f[2]) || hex(f[2]), size: f[1], path: f[0] });
  }
  for (const [d, h] of dirs) out.push({ type: "directory", oid: h.digest("hex"), size: 0, path: d });
  return out.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

// A model asked for but not indexed: the next index run picks it up.
const record = (id) => { if (/^[\w.-]+\/[\w.-]+$/.test(id)) appendFile(join(STATE, "requested.txt"), `${new Date().toISOString().slice(0, 10)} ${id}\n`).catch(() => {}); };
async function missing(res, id) {
  let gated = false;
  try { gated = JSON.parse(await readFile(join(DATA, "models.json"), "utf8")).models.some((m) => m.id.toLowerCase() === id.toLowerCase() && m.state === "skipped"); } catch { /* no catalog */ }
  if (gated) return refuse(res, 403, "GatedRepo", `${id} is gated on Hugging Face. The hub serves public models only; use huggingface.co directly for this one.`);
  record(id);
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
  const { extra } = await hubCatalog();
  if (!extra.length) return catalog.rows;
  // Today's rows win where both exist: they carry the facets, and a thin row would otherwise mask a full one.
  const listed = new Set(catalog.rows.map((r) => r.id));
  return catalog.rows.concat(extra.filter((r) => !listed.has(r.id)));
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
  // The document states `minimum: 1, maximum: 500` and an enum of sort keys. Silently defaulting a bad value means
  // a client coding against those constraints never sees the error it is handling, and ships the bug anyway.
  const sortParam = q.get("sort");
  if (sortParam !== null && !SORTS[sortParam]) return refuse(res, 400, "BadParameter", `sort must be one of ${Object.keys(SORTS).join(", ")}.`);
  const limitParam = q.get("limit");
  if (limitParam !== null && !/^\d+$/.test(limitParam)) return refuse(res, 400, "BadParameter", "limit must be a whole number between 1 and 500.");
  const limitValue = limitParam === null ? 50 : Number(limitParam);
  if (limitParam !== null && (limitValue < 1 || limitValue > 500)) return refuse(res, 400, "BadParameter", "limit must be between 1 and 500.");
  const key = SORTS[sortParam || "trendingScore"], up = q.get("direction") === "1";
  out = [...out].sort((a, b) => (a[key] > b[key] ? 1 : a[key] < b[key] ? -1 : 0) * (up ? 1 : -1));
  const limit = limitValue;
  return json(res, 200, out.slice(0, limit), { "access-control-allow-origin": "*", "x-total-count": String(out.length) });
}

// ---- Ollama's registry dialect: `ollama pull gethologram.ai/<org>/<name>:<quant>`
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

// ---- OCI model artifacts (CNCF ModelPack) on the same /v2 routes, for clients that accept OCI manifests:
// `oras pull gethologram.ai/<org>/<name>:latest`, modctl, KitOps, Docker Model Runner, containerd.
// Every file is one raw layer, so a layer's digest is the file's SHA-256 from the index and its blob is a redirect to
// a live source; the client verifies the digest. Only the manifest and the config (a few KB of JSON) are ours.
// Tags: latest or main (or a prefix of the indexed revision) = every file; a GGUF quantisation = that file and the docs.
const OCI_MANIFEST = "application/vnd.oci.image.manifest.v1+json";
const WEIGHTS = /\.(safetensors|gguf|bin|pt|pth|ckpt|onnx|h5|msgpack|ot|mlmodel|npz|tflite|pb)$/i;
const layerType = (f) => `application/vnd.cncf.model.${f[3] || WEIGHTS.test(f[0]) ? "weight" : /(^|\/)(readme|license|notice|changelog)[^/]*$|\.(md|txt|pdf)$/i.test(f[0]) ? "doc" : /\.(py|sh|ipynb|js|ts|c|cpp|h|rs)$/i.test(f[0]) ? "code" : "weight.config"}.v1.raw`;
function ociFiles(doc, tag) {
  if (["latest", "main"].includes(tag.toLowerCase()) || (tag.length >= 7 && doc.revision.startsWith(tag))) return doc.files;
  const { file } = pickGguf(doc, tag);
  return file ? doc.files.filter((f) => f === file || /^(readme|license)[^/]*$/i.test(f[0])) : null;
}
async function ociManifest(doc, files) {
  const row = (await rows()).find((m) => m.id === doc.id);
  const license = (row?.tags.find((t) => t.startsWith("license:")) || "").slice(8);
  const params = row?.hologram.parameters;
  const config = Buffer.from(JSON.stringify({
    descriptor: { name: doc.id, version: doc.revision, revision: doc.revision, vendor: doc.id.split("/")[0], sourceURL: `https://huggingface.co/${doc.id}`, docURL: `${HUB}/models/${doc.id}/`, ...(license ? { licenses: [license] } : {}) },
    config: { format: files.some((f) => /\.gguf$/i.test(f[0])) ? "gguf" : files.some((f) => /\.safetensors$/i.test(f[0])) ? "safetensors" : "other", ...(params ? { paramSize: params >= 1e9 ? `${Math.round(params / 1e8) / 10}B` : `${Math.round(params / 1e6)}M` } : {}) },
    modelfs: { type: "layers", diffIds: files.map((f) => f[2]) },
  }));
  const configDigest = await keep(config);
  const bytes = Buffer.from(JSON.stringify({
    schemaVersion: 2, mediaType: OCI_MANIFEST, artifactType: "application/vnd.cncf.model.manifest.v1+json",
    config: { mediaType: "application/vnd.cncf.model.config.v1+json", digest: configDigest, size: config.length },
    layers: files.map((f) => ({ mediaType: layerType(f), digest: f[2], size: f[1], annotations: { "org.opencontainers.image.title": f[0], "org.cncf.model.filepath": f[0], "org.cncf.model.file.metadata+json": JSON.stringify({ name: f[0], mode: 420, uid: 0, gid: 0, size: f[1], mtime: "1970-01-01T00:00:00Z", typeflag: 48 }) } })),
    annotations: { "org.opencontainers.image.source": `https://huggingface.co/${doc.id}`, "org.opencontainers.image.revision": doc.revision, "org.opencontainers.image.url": `${HUB}/models/${doc.id}/` },
  }));
  await keep(bytes); // also fetchable by its digest, which is how OCI clients ask the second time
  return bytes;
}
// ---- names that are not models belong to the registry. Caddy sends every GET/HEAD under /v2/<org>/<name>/ here
// because it cannot know which names are models; this can. An image pushed as `you/app` is answered by the
// registry, byte for byte, through this process: status, headers and body are relayed, nothing is rewritten.
const REGISTRY = process.env.HUB_REGISTRY || "http://hub-kappa:5000";
async function registry(req, res, path) {
  let upstream;
  try {
    const headers = {};
    for (const name of ["accept", "range", "if-none-match", "user-agent"]) if (req.headers[name]) headers[name] = req.headers[name];
    upstream = await fetch(`${REGISTRY}${path}${req.url.includes("?") ? req.url.slice(req.url.indexOf("?")) : ""}`, { method: req.method, headers, redirect: "manual", signal: AbortSignal.timeout(120_000) });
  } catch (error) {
    return ociError(res, 502, "UNAVAILABLE", `the registry did not answer: ${error.message}`);
  }
  const out = {};
  for (const [name, value] of upstream.headers) if (!/^(transfer-encoding|connection|content-encoding)$/i.test(name)) out[name] = value;
  res.writeHead(upstream.status, out);
  if (req.method === "HEAD" || !upstream.body) return res.end();
  const { Readable } = await import("node:stream");
  Readable.fromWeb(upstream.body).pipe(res);
}

async function ollama(req, res, id, kind, ref) {
  const doc = await model(id);
  if (!doc) return registry(req, res, `/v2/${id}/${kind}/${ref}`);
  if (kind === "tags") {
    const quants = [...new Set(doc.files.filter((f) => /\.gguf$/i.test(f[0]) && !/mmproj|imatrix|-\d{5}-of-/i.test(f[0])).map((f) => quantOf(f[0])).filter(Boolean))];
    return json(res, 200, { name: doc.id, tags: ["latest", ...quants] });
  }
  if (kind === "manifests" && /^sha256:[0-9a-f]{64}$/.test(ref)) { // a manifest asked again by its digest
    const kept = await smallBlob(doc.id, ref);
    if (!kept) return ociError(res, 404, "MANIFEST_UNKNOWN", `${ref} is not a manifest the hub has generated; ask by tag first.`);
    const type = JSON.parse(kept.toString("utf8")).mediaType || OCI_MANIFEST;
    res.writeHead(200, { "content-type": type, "content-length": kept.length, "docker-content-digest": ref, "cache-control": "no-store" });
    return res.end(req.method === "HEAD" ? undefined : kept);
  }
  if (kind === "manifests" && String(req.headers.accept || "").includes(OCI_MANIFEST)) {
    const files = ociFiles(doc, ref);
    if (!files) return ociError(res, 404, "MANIFEST_UNKNOWN", `${doc.id} has no tag ${ref}. Use latest, or one of its GGUF quantisations.`);
    const bytes = await ociManifest(doc, files);
    res.writeHead(200, { "content-type": OCI_MANIFEST, "content-length": bytes.length, "docker-content-digest": sha(bytes), "x-repo-commit": doc.revision, "cache-control": "no-store" });
    return res.end(req.method === "HEAD" ? undefined : bytes);
  }
  if (kind === "manifests") {
    const { file, sharded, quants } = pickGguf(doc, ref);
    if (!file) return ociError(res, 404, "MANIFEST_UNKNOWN", sharded ? `${ref} of ${doc.id} is split across several files; Ollama needs a single GGUF file.` : quants.length ? `${doc.id} has no ${ref}. It has: ${[...new Set(quants)].join(", ")}.` : `${doc.id} has no GGUF file; Ollama pulls GGUF models.`);
    let bytes = await manifestFor(doc, ref, file);
    // llama.cpp before b8498 (distro builds, older embedders) asks this route with its own user agent and reads the
    // file name from Hugging Face's extension to the manifest, `ggufFile` (captured from huggingface.co 2026-09-26).
    if (/^llama-cpp/i.test(req.headers["user-agent"] || "")) {
      const m = JSON.parse(bytes.toString("utf8"));
      const p = lfsPointer(file);
      m.ggufFile = { rfilename: file[0], blobId: p.oid, size: file[1], lfs: { sha256: hex(file[2]), size: file[1], pointerSize: p.pointerSize } };
      bytes = Buffer.from(JSON.stringify(m));
      res.writeHead(200, { "content-type": "application/json; charset=utf-8", "content-length": bytes.length, "x-repo-commit": doc.revision, "cache-control": "no-store" });
      return res.end(req.method === "HEAD" ? undefined : bytes);
    }
    res.writeHead(200, { "content-type": MANIFEST_TYPE, "content-length": bytes.length, "docker-content-digest": sha(bytes), "x-repo-commit": doc.revision, "cache-control": "no-store" });
    return res.end(req.method === "HEAD" ? undefined : bytes);
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(ref)) return ociError(res, 400, "DIGEST_INVALID", "A blob is named sha256:<64 hex digits>.");
  const entry = doc.files.find((f) => f[2] === ref);
  if (entry) { // the weights: never through us
    if (req.method === "HEAD") { res.writeHead(200, { "content-length": entry[1], "docker-content-digest": ref, "accept-ranges": "bytes", "content-type": "application/octet-stream" }); return res.end(); }
    if (VERIFY) {
      // Ollama follows a same-host redirect itself and wants its last answer to be a 307 to another host: send it to
      // the other loopback name of this same process, which serves the verified file there.
      blobs.set(hex(ref), { doc, entry });
      const host = String(req.headers.host || `127.0.0.1:${PORT}`), other = host.startsWith("localhost") ? host.replace("localhost", "127.0.0.1") : host.replace(/^[^:]+/, "localhost");
      res.writeHead(307, { location: `http://${other}/_blob/${ref}`, "docker-content-digest": ref, "x-hub-source": "verified", "cache-control": "no-store", "content-length": "0" });
      return res.end();
    }
    const { source, reason } = choose(doc, entry, null);
    console.log(JSON.stringify({ t: new Date().toISOString(), dialect: /^ollama/i.test(req.headers["user-agent"] || "") ? "ollama" : "oci", model: doc.id, file: entry[0], source: source.kind, reason }));
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
    // Truncating at 200 is fine; truncating alphabetically is not, because the tail of a model directory is where
    // the tokenizer and config files live and those are the ones a caller cannot proceed without. Keep the small
    // text files that make a model usable, then fill the rest with the largest weights.
    const ESSENTIAL = /(^|\/)(config|tokenizer|tokenizer_config|special_tokens_map|generation_config|preprocessor_config|vocab|merges|added_tokens|chat_template)[^/]*$|\.(json|txt|model|py|md)$/i;
    const all = doc.files.map((f) => ({ path: f[0], size: f[1], sha256: hex(f[2]) }));
    const essential = all.filter((f) => ESSENTIAL.test(f.path));
    const rest = all.filter((f) => !ESSENTIAL.test(f.path)).sort((a, b) => b.size - a.size);
    const files = all.length > 200 ? [...essential, ...rest].slice(0, 200).sort((a, b) => a.path.localeCompare(b.path)) : all;
    return toolResult({ id: doc.id, revision: doc.revision, purl: `pkg:huggingface/${doc.id}@${doc.revision}`, files_total: all.length, bytes_total: all.reduce((s, f) => s + f.size, 0), files, files_truncated: all.length > files.length, files_truncated_note: all.length > files.length ? `showing ${files.length} of ${all.length}: every config and tokenizer file, then the largest weights. The full list is at ${HUB}/api/models/${doc.id}/tree/main.` : undefined, gguf_quants: [...new Set(ggufs.map((f) => quantOf(f[0])).filter(Boolean))], sources: sourcesOf(doc), sha256sums: `${HUB}/${doc.id}/resolve/main/SHA256SUMS`, download_all: [`export HF_ENDPOINT=${HUB}`, `hf download ${doc.id}`] });
  }
  if (name === "resolve_file") {
    const entry = a.path ? doc.files.find((f) => f[0] === a.path) : a.quant ? pickGguf(doc, String(a.quant)).file : null;
    if (!entry) return toolError(a.path ? `${a.path} is not a file of ${doc.id} at ${doc.revision}. get_model lists the files.` : a.quant ? `${doc.id} has no single-file GGUF for ${a.quant}. It has: ${[...new Set(ggufs.map((f) => quantOf(f[0])).filter(Boolean))].join(", ") || "no GGUF files"}.` : "Give a path or a quant.");
    const have = doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind) && !(s.missing || []).includes(entry[0]));
    const { source } = choose(doc, entry, null); // no pin here: this tool reports every source and lets the caller pick
    const url = `${HUB}/${doc.id}/resolve/${doc.revision}/${encodePath(entry[0])}`, name0 = entry[0].split("/").pop(), isGguf = /\.gguf$/i.test(entry[0]);
    return toolResult({ id: doc.id, revision: doc.revision, path: entry[0], size: entry[1], sha256: hex(entry[2]),
      url, url_note: "Redirects to a source that is up right now; supports Range. No credentials needed.", served_by_now: source.kind,
      sources: have.map((s) => ({ kind: s.kind, healthy: health[s.kind].ok, url: urlFor(doc, s, entry) })),
      download: `curl -L -o ${JSON.stringify(name0)} ${JSON.stringify(url)}`, verify: `echo "${hex(entry[2])}  ${name0}" | sha256sum -c`,
      handoff: { hf: [`export HF_ENDPOINT=${HUB}`, `hf download ${doc.id} ${entry[0]}`], ...(isGguf && quantOf(entry[0]) ? { ollama: `ollama pull ${HUB_HOST}/${doc.id}:${quantOf(entry[0])}`, llama_cpp: `MODEL_ENDPOINT=${HUB}/ llama-server -hf ${doc.id}:${quantOf(entry[0])}` } : {}) } });
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

// ---- the verifying edge (HUB_VERIFY=1)
const blobs = new Map();   // sha256 hex -> { doc, entry } of a blob a manifest named, for the /_blob/ hop
function holders(doc, entry, via) {
  const have = doc.sources.filter((s) => !s.p2p && !s.pull && ORDER.includes(s.kind) && !(s.missing || []).includes(entry[0]) && (!via || s.kind === via));
  have.sort((a, b) => (health[b.kind].ok - health[a.kind].ok) || ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind));
  return have.map((s) => ({ kind: s.kind, url: urlFor(doc, s, entry) })).filter((h) => h.url);
}
async function serveVerified(req, res, doc, entry, via, headers) {
  const all = { ...headers, "x-hub-source": "verified", "cache-control": "no-store" };
  const log = (e) => console.log(JSON.stringify({ t: new Date().toISOString(), model: doc.id, file: entry[0], ...e }));
  try { return await streamVerified(req, res, CACHE, hex(entry[2]), entry[1], holders(doc, entry, via), all, log); }
  catch (e) { if (!res.headersSent) return refuse(res, 502, "NoVerifiedSource", `${entry[0]} of ${doc.id}: ${e.message}`); res.destroy(); }
}

// ---- Git LFS: `git -c lfs.url=https://gethologram.ai/<org>/<name>.git/info/lfs lfs pull` (download only)
//
// The batch API answers where each object can be fetched; git-lfs then checks every object against the oid its git
// pointer names (sha256) and refuses one that differs. So this route verifies without any help from us; with
// HUB_VERIFY=1 the href is this process's own verifying /_blob/ route as well.
async function lfsBatch(req, res, id) {
  const reply = (status, body) => { const text = JSON.stringify(body); res.writeHead(status, { "content-type": "application/vnd.git-lfs+json", "content-length": Buffer.byteLength(text) }); res.end(text); };
  let ask;
  try { const chunks = []; for await (const c of req) { chunks.push(c); if (chunks.reduce((n, x) => n + x.length, 0) > 4 << 20) throw new Error("too large"); } ask = JSON.parse(Buffer.concat(chunks).toString("utf8")); }
  catch { return reply(400, { message: "The batch request is not JSON." }); }
  if (ask.operation !== "download") return reply(403, { message: "This endpoint is read-only: download only." });
  const doc = await model(id);
  if (!doc) return reply(404, { message: `${id} is not in the Hologram index yet.` });
  const byOid = new Map(doc.files.map((f) => [hex(f[2]), f]));
  const host = String(req.headers["x-forwarded-host"] || req.headers.host || `127.0.0.1:${PORT}`);
  const objects = (ask.objects || []).slice(0, 1000).map(({ oid, size }) => {
    const entry = byOid.get(String(oid));
    if (!entry) return { oid, size, error: { code: 404, message: `${oid} is not part of ${doc.id} at ${doc.revision}.` } };
    if (VERIFY) { blobs.set(entry[2].slice(7), { doc, entry }); return { oid, size: entry[1], authenticated: true, actions: { download: { href: `http://${host}/_blob/${entry[2]}`, expires_in: 3600 } } }; }
    const { source } = choose(doc, entry, null);
    return { oid, size: entry[1], authenticated: true, actions: { download: { href: urlFor(doc, source, entry), expires_in: 3600 } } };
  });
  return reply(200, { transfer: "basic", objects, hash_algo: "sha256" });
}

const revisionOk = (doc, rev) => rev === "main" || (rev.length >= 7 && doc.revision.startsWith(rev));

http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://hub");
    if (url.pathname === "/mcp") return mcp(req, res);
    // Browsers on other sites (transformers.js, huggingface.js) read this endpoint too: everything is public and
    // read-only, so every answer may be read cross-origin, the redirect included, and a Range preflight is allowed.
    res.setHeader("access-control-allow-origin", "*");
    res.setHeader("access-control-expose-headers", "etag, x-repo-commit, x-linked-etag, x-linked-size, x-hub-source, x-total-count, x-error-code, x-error-message, accept-ranges, content-range, docker-content-digest, location");
    if (req.method === "OPTIONS") { res.writeHead(204, { "access-control-allow-methods": "GET, HEAD, OPTIONS", "access-control-allow-headers": "range, accept, content-type, if-none-match, user-agent", "access-control-max-age": "86400" }); return res.end(); }
    const lfs = url.pathname.match(/^\/([^/]+\/[^/]+?)(?:\.git)?\/info\/lfs\/objects\/batch$/);
    if (lfs && req.method === "POST") return lfsBatch(req, res, decodeURIComponent(lfs[1]));
    if (req.method !== "GET" && req.method !== "HEAD") return refuse(res, 405, "ReadOnly", "The hub endpoint is read-only.");
    let path = decodeURIComponent(url.pathname), via = url.searchParams.get("source");
    const prefix = path.match(/^\/via\/([a-z.]+)(\/.*)$/);
    if (prefix) { via = prefix[1] === "modelscope" ? "modelscope.cn" : prefix[1] === "huggingface" ? "huggingface.co" : prefix[1]; path = prefix[2]; }

    // The κ mirror: /v2/<upstream host>/<path>/… for every image the Registry page indexes (kappa-mirror.mjs).
    const blob = VERIFY && path.match(/^\/_blob\/sha256:([0-9a-f]{64})$/);
    if (blob) {
      const known = blobs.get(blob[1]);
      if (!known) return refuse(res, 404, "BlobUnknown", "Ask for the blob through its model's manifest first.");
      return serveVerified(req, res, known.doc, known.entry, null, { "docker-content-digest": `sha256:${blob[1]}` });
    }
    if (path.startsWith("/v2/") && await kappaMirror(req, res, path)) return;
    const oci = path.match(/^\/v2\/([^/]+\/[^/]+)\/(manifests|blobs|tags)\/(.+)$/);
    if (oci) return ollama(req, res, oci[1], oci[2], oci[3]);
    if (path === "/api/models" || path === "/api/models/") return list(res, url.searchParams);
    if (path === "/api/hub/health") return json(res, 200, { sources: health, order: ORDER }, { "cache-control": "no-store", "access-control-allow-origin": "*" });

    // ModelScope's dialect: `MODELSCOPE_ENDPOINT=https://gethologram.ai` (modelscope 1.40 SDK, recorded 2026-09-26).
    // Its SDK hashes every file against the listing's Sha256 on download and on every cache hit: the strongest client.
    const ms = path.match(/^\/api\/v1\/models\/([^/]+\/[^/]+?)(\/repo\/files|\/repo|\/revisions)?$/);
    if (ms || path === "/api/v1/repos/internalAccelerationInfo") {
      const ok = (Data) => json(res, 200, { Code: 200, Data, Message: "success", Success: true });
      const no = (status, Message) => json(res, status, { Code: status, Data: null, Message, Success: false });
      if (!ms) return ok({});
      const doc = await model(ms[1]);
      if (!doc) { record(ms[1]); return no(404, `${ms[1]} is not in the Hologram index yet; the request was recorded for the next index run.`); }
      const rev = url.searchParams.get("Revision") || "master";
      if (rev !== "master" && !revisionOk(doc, rev)) return no(404, `The hub has ${doc.id} at ${doc.revision} (master) only.`);
      if (ms[2] === "/revisions") return ok({ RevisionMap: { Branches: [{ Revision: "master", CreatedAt: 0 }], Tags: [] } });
      if (!ms[2]) return ok({ Name: doc.id.split("/")[1], Path: doc.id.split("/")[0], Revision: doc.revision, ModelId: doc.id });
      if (ms[2] === "/repo/files") {
        const dirs = [...new Set(doc.files.flatMap((f) => f[0].split("/").slice(0, -1).map((_, i, a) => a.slice(0, i + 1).join("/"))))];
        return ok({ Files: [
          ...dirs.map((d) => ({ Name: d.split("/").pop(), Path: d, Type: "tree", Size: 0, Sha256: "", IsLFS: false, Revision: doc.revision })),
          ...doc.files.map((f) => ({ Name: f[0].split("/").pop(), Path: f[0], Type: "blob", Size: f[1], Sha256: hex(f[2]), IsLFS: isLfs(f), Revision: doc.revision })),
        ] });
      }
      const entry = doc.files.find((f) => f[0] === url.searchParams.get("FilePath"));
      if (!entry) return no(404, `${url.searchParams.get("FilePath")} is not in ${doc.id}.`);
      if (VERIFY) return serveVerified(req, res, doc, entry, via, { etag: `"${hex(entry[2])}"` });
      const { source } = choose(doc, entry, via);
      res.writeHead(302, { location: urlFor(doc, source, entry), etag: `"${hex(entry[2])}"`, "x-hub-source": source.kind, "cache-control": "no-store", "content-length": "0" });
      return res.end();
    }

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
    const info = path.match(/^\/api\/models\/([^/]+\/[^/]+?)(?:\/revision\/(.+))?$/), tree = path.match(/^\/api\/models\/([^/]+\/[^/]+)\/(tree|treesize)\/([^/]+)(?:\/(.*))?$/);
    if (tree) {
      const doc = await model(tree[1]);
      if (!doc) return missing(res, tree[1]);
      if (!revisionOk(doc, tree[3])) return refuse(res, 404, "RevisionNotFound", `The hub has ${doc.id} at ${doc.revision} only.`);
      const at = (tree[4] || "").replace(/\/$/, ""), dir = at ? `${at}/` : "";
      const under = doc.files.filter((f) => f[0].startsWith(dir));
      // A path that names nothing is Hugging Face's EntryNotFound, not an empty listing: clients tell the two apart.
      if (at && !under.length) return refuse(res, 404, "EntryNotFound", `${at} is not a folder in ${doc.id} at ${doc.revision}.`);
      if (tree[2] === "treesize") return json(res, 200, { path: at, size: under.reduce((s, f) => s + f[1], 0) });
      // One page holds the whole listing, so any next page is empty (text-generation-webui pages until it gets []).
      if (url.searchParams.has("cursor")) return json(res, 200, []);
      await gitOids(doc, under);
      return json(res, 200, listing(under, dir, ["true", "1"].includes(url.searchParams.get("recursive") || "")));
    }
    if (info) {
      const doc = await model(info[1]);
      if (!doc) return missing(res, info[1]);
      if (info[2] && !revisionOk(doc, info[2])) return refuse(res, 404, "RevisionNotFound", `The hub has ${doc.id} at ${doc.revision} only.`);
      // ?blobs=true (huggingface_hub's files_metadata=True): size and the LFS sha256 per file, as Hugging Face sends.
      const blobs = ["true", "1"].includes(url.searchParams.get("blobs") || "");
      const siblings = doc.files.map((f) => blobs ? { rfilename: f[0], size: f[1], ...(isLfs(f) ? { blobId: lfsPointer(f).oid, lfs: { sha256: hex(f[2]), size: f[1], pointerSize: lfsPointer(f).pointerSize } } : { ...(f[5] || gitOid.get(f[2]) ? { blobId: f[5] || gitOid.get(f[2]) } : {}) }) } : { rfilename: f[0] });
      return json(res, 200, { _id: hex(doc.manifest).slice(0, 24), id: doc.id, modelId: doc.id, sha: doc.revision, private: false, gated: false, disabled: false, tags: [], downloads: 0, likes: 0, siblings, ...(blobs ? { usedStorage: doc.files.reduce((s, f) => s + f[1], 0) } : {}) });
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
      const { source, reason, denied } = choose(doc, entry, via);
      if (denied) return refuse(res, 404, denied.code, denied.message);
      if (VERIFY) return serveVerified(req, res, doc, entry, via, { "x-repo-commit": doc.revision, "x-linked-etag": `"${hex(entry[2])}"`, "x-linked-size": String(entry[1]), etag: `"${hex(entry[2])}"` });
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
