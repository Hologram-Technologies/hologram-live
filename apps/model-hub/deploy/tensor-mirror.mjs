// The tensor mirror: every indexed open model as a tiny OCI artifact whose files are rebuilt from κ-addressed
// tensors on demand. Same rule as kappa-mirror.mjs (hold the small objects, redirect the large ones), plus one
// new case: when no whole-file source should be used, assemble the file from its layout, verifying every tensor.
//
//   GET|HEAD /v2/<owner>/<name>/…                        the plain name (tensorMirrorRoot): what the page shows
//   GET|HEAD /v2/models/<owner>/<name>/manifests/<ref>   ref = latest (safetensors) | safetensors | sharded |
//                                                        original | index | <revision> (the OCI index) | sha256:…
//   GET|HEAD /v2/models/<owner>/<name>/blobs/<sha256:…>  held objects from here; weight files 307 to Hugging Face,
//                                                        or assembled (?source=tensors, or Hugging Face down,
//                                                        or a render that exists nowhere as a file); Range honoured
//   GET      /v2/models/<owner>/<name>/tags/list
//   GET      /v2/models/<owner>/<name>/tensors            the tensor table (also the manifest's config blob)
//   GET      /v2/models/<owner>/<name>/provenance         per-κ provenance sample: narrow dtype, sign, row blocks
//
// State ($HUB_STATE/tensors, built by tensors/pipeline.mjs and synced like the κ mirror's):
//   models.json   { "<owner>/<name>": { rev, index, manifests: {format: digest}, table, blobs: {digest: {path,size,layout?,held?,upstream?}}, gated } }
//   sources/*.json  [κ, repo, rev, path, off, len] rows: where each payload can be read
//   sha256/<hex>  every held object, hashed on read
import { readFile, stat, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { assemble } from "./tensor-assemble.mjs";

const ROOT = process.env.TENSOR_STATE || join(process.env.HUB_STATE || "/state", "tensors");
const HF = process.env.HF_ORIGIN || "https://huggingface.co";
const INDEX = "application/vnd.oci.image.index.v1+json", MANIFEST = "application/vnd.oci.image.manifest.v1+json";

let state = { at: 0, mtime: 0, models: {}, alts: new Map(), lower: new Map() };
async function load() {
  if (Date.now() - state.at < 30_000) return state;
  state.at = Date.now();
  try {
    const s = await stat(join(ROOT, "models.json"));
    if (s.mtimeMs !== state.mtime) {
      const models = JSON.parse(await readFile(join(ROOT, "models.json"), "utf8"));
      const alts = new Map();
      for (const f of await readdir(join(ROOT, "sources")).catch(() => [])) {
        for (const [k, repo, rev, path, off, len] of JSON.parse(await readFile(join(ROOT, "sources", f), "utf8"))) {
          if (!models[repo]?.gated) continue;                       // only gated models serve as sources
          if (!alts.has(k)) alts.set(k, []);
          alts.get(k).push({ repo, rev, path, off, len });
        }
      }
      const gw = process.env.IPFS_GATEWAY;                          // pinned payloads, each its own IPFS object
      if (gw) {
        const pins = JSON.parse(await readFile(join(ROOT, "pins.json"), "utf8").catch(() => "{}"));
        for (const [k, p] of Object.entries(pins)) (alts.get(k) || alts.set(k, []).get(k)).push({ url: `${gw.replace(/\/$/, "")}/ipfs/${p.cid}`, off: 0, len: p.len });
      }
      const lower = new Map(Object.keys(models).map((r) => [r.toLowerCase(), r]));   // OCI names are lowercase
      // relations from the latest sealed day root: same weights, and tensors shared with other models
      let root = null;
      try { const latest = JSON.parse(await readFile(join(ROOT, "latest.json"), "utf8")); root = JSON.parse((await held(latest.digest))?.toString("utf8") || "null"); root.digest = latest.digest;
        const car = JSON.parse(await readFile(join(ROOT, "car", "ipfs.json"), "utf8").catch(() => "null"));
        root.car = car && car.sealed === latest.digest ? car.root : null;   // the day's CAR exists for this very root
      } catch { root = null; }
      state = { at: Date.now(), mtime: s.mtimeMs, models, alts, lower, root };
    }
  } catch { /* nothing published yet */ }
  return state;
}

const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
async function held(d) {
  if (!/^sha256:[0-9a-f]{64}$/.test(d)) return null;
  try { const b = await readFile(join(ROOT, "sha256", d.slice(7))); return sha(b) === d ? b : null; } catch { return null; }
}
function ociError(res, status, code, message) {
  const t = JSON.stringify({ errors: [{ code, message }] });
  res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "docker-distribution-api-version": "registry/2.0" });
  res.end(res.req.method === "HEAD" ? undefined : t);
}
function send(res, bytes, type, digest) {
  res.writeHead(200, { "content-type": type, "content-length": bytes.length, "docker-content-digest": digest, etag: `"${digest}"`,
    "docker-distribution-api-version": "registry/2.0", "cache-control": "public, max-age=31536000, immutable" });
  res.end(res.req.method === "HEAD" ? undefined : bytes);
}

// Is Hugging Face serving right now? Probed at most once a minute.
let hf = { at: 0, ok: true };
async function hfUp() {
  if (Date.now() - hf.at < 60_000) return hf.ok;
  hf.at = Date.now();
  try { const r = await fetch(`${HF}/api/models/gpt2`, { method: "HEAD", signal: AbortSignal.timeout(5000) }); hf.ok = r.status < 500; }
  catch { hf.ok = false; }
  return hf.ok;
}

// Tags are formats. The bare name (and "latest", "main", the revision) is the OCI index; ":safetensors",
// ":sharded", ":original" name one format; "<main|rev>-<format>" is kept for older pulls.
const FORMAT_TAG = { safetensors: "safetensors", sharded: "safetensors-sharded", "safetensors-sharded": "safetensors-sharded", original: "original" };
function resolveRef(m, ref) {
  if (ref.startsWith("sha256:")) return ref;
  if (ref === m.rev || ref === "index") return m.index;                       // every format, as an OCI index
  if (ref === "main" || ref === "latest") return m.manifests.safetensors || m.manifests.original;   // the bare name: the default format, so a plain pull just works
  if (FORMAT_TAG[ref] && m.manifests[FORMAT_TAG[ref]]) return m.manifests[FORMAT_TAG[ref]];
  for (const f of Object.keys(m.manifests)) for (const head of ["main", m.rev]) if (ref === `${head}-${f}`) return m.manifests[f];
  return null;
}

// The raw-leaf CIDv1 of a sha256 digest: for an object of at most 1 MiB this is its IPFS address (unixfs-v1-2025).
function rawCid(digest) {
  const bytes = Buffer.concat([Buffer.from([0x01, 0x55, 0x12, 0x20]), Buffer.from(digest.slice(7), "hex")]);
  const A = "abcdefghijklmnopqrstuvwxyz234567"; let bits = 0, v = 0, out = "b";
  for (const b of bytes) { v = (v << 8) | b; bits += 8; while (bits >= 5) { out += A[(v >>> (bits - 5)) & 31]; bits -= 5; } }
  if (bits) out += A[(v << (5 - bits)) & 31];
  return out;
}

// What the Models and Registry pages show for one model: its address, formats and relations. Small, derived.
function summary(repo, m, root) {
  const lc = repo.toLowerCase();
  const edges = (root?.edges || []).filter((e) => e.a === repo || e.b === repo)
    .map((e) => ({ repo: e.a === repo ? e.b : e.a, bytes: e.bytes, tensors: e.tensors, sameWeights: e.sameWeights, licenceDiffers: e.licenceDiffers }));
  return {
    repo, revision: m.rev, license: m.license, index: m.index, canonical: m.canonical, tensors: m.tensors, weightBytes: m.weightBytes,
    reference: lc,                                            // pulled as <host>/<org>/<name>, the address shown
    formats: Object.fromEntries(Object.entries(m.manifests).map(([f, d]) => [f, { digest: d, tag: f === "safetensors-sharded" ? "sharded" : f }])),
    ipfs: rawCid(m.index),                                    // the manifest's IPFS address: the same sha256, as a raw CID
    pinned: Boolean(root?.car),                               // published in the day's CAR
    sameWeights: (root?.sameWeights || []).find((g) => g.includes(repo))?.filter((r) => r !== repo) || [],
    shares: edges.filter((e) => !e.sameWeights).sort((a, b) => b.bytes - a.bytes),
    root: root?.digest || null, day: root?.day || null,
  };
}

export async function tensorMirror(req, res, path) {
  // The catalogue of models with a tensor index, for the Registry page and agents.
  if (path === "/v2/models/_catalog") {
    const { models, root } = await load();
    const rows = Object.entries(models).filter(([, m]) => m.gated).map(([r, m]) => summary(r, m, root));
    const t = JSON.stringify({ day: root?.day || null, root: root?.digest || null, models: rows });
    res.writeHead(200, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "cache-control": "public, max-age=300", "access-control-allow-origin": "*" });
    res.end(req.method === "HEAD" ? undefined : t);
    return true;
  }
  // /v2/models/…: files redirect to Hugging Face while it serves. /v2/tensors/…: the same model, every file
  // assembled from its tensors (what a client sees when Hugging Face is gone).
  const sm = path.match(/^\/v2\/models\/([^/]+\/[^/]+)\/summary$/);
  if (sm) {
    const { models, lower, root } = await load();
    const repo = models[sm[1]] ? sm[1] : lower.get(sm[1].toLowerCase()), m = repo && models[repo];
    if (!m || !m.gated) { ociError(res, 404, "NAME_UNKNOWN", `${sm[1]} has no tensor index yet`); return true; }
    const t = JSON.stringify(summary(repo, m, root));
    res.writeHead(200, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "cache-control": "public, max-age=300", "access-control-allow-origin": "*" });
    res.end(req.method === "HEAD" ? undefined : t);
    return true;
  }
  const mm = path.match(/^\/v2\/(models|tensors)\/([^/]+\/[^/]+)\/(manifests|blobs|tags|tensors|provenance|alternatives)(?:\/([^/]+))?$/);
  if (!mm) return false;
  const [, space, name, kind, ref] = mm;
  const { models, alts, lower } = await load();
  const repo = models[name] ? name : lower.get(name.toLowerCase());
  const m = repo && models[repo];
  if (!m || !m.gated) { ociError(res, 404, "NAME_UNKNOWN", `${name} has no tensor index yet`); return true; }

  if (kind === "tags") {
    const tags = ["latest", "index", m.rev, ...Object.keys(FORMAT_TAG).filter((t) => t !== "safetensors-sharded" && m.manifests[FORMAT_TAG[t]])];
    const t = JSON.stringify({ name: `models/${repo}`, tags });
    res.writeHead(200, { "content-type": "application/json", "content-length": Buffer.byteLength(t) }); res.end(req.method === "HEAD" ? undefined : t);
    return true;
  }
  if (kind === "provenance") { const b = m.provenance && await held(m.provenance); if (!b) return ociError(res, 404, "BLOB_UNKNOWN", "no provenance sample for this model"), true; send(res, b, "application/vnd.hologram.provenance.v1+json", m.provenance); return true; }
  if (kind === "tensors") { const b = await held(m.table); if (!b) return ociError(res, 404, "BLOB_UNKNOWN", "table not held"), true; send(res, b, "application/vnd.hologram.tensors.v1+json", m.table); return true; }

  if (kind === "manifests") {
    const digest = resolveRef(m, ref || "");
    const known = digest && (digest === m.index || Object.values(m.manifests).includes(digest));
    const b = known && await held(digest);
    if (!b) { ociError(res, 404, "MANIFEST_UNKNOWN", `models/${repo}:${ref} is not indexed`); return true; }
    send(res, b, digest === m.index ? INDEX : MANIFEST, digest);
    return true;
  }

  // other holders of every payload in one layout, for clients that assemble locally
  if (kind === "alternatives") {
    const lay = JSON.parse((await held(ref || ""))?.toString("utf8") || "null");
    if (!lay?.segments) { ociError(res, 404, "BLOB_UNKNOWN", "not a layout"); return true; }
    const out = {}; for (const s of lay.segments) if (s[0] !== "l" && alts.has(s[1])) out[s[1]] = alts.get(s[1]);
    const t = JSON.stringify(out);
    res.writeHead(200, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "cache-control": "public, max-age=300" }); res.end(t);
    return true;
  }

  // blobs
  if (ref === m.table) { const b = await held(ref); send(res, b, "application/vnd.hologram.tensors.v1+json", ref); return true; }
  const blob = m.blobs[ref];
  if (!blob) {                                                     // layouts and literals: any held object, by digest
    const b = await held(ref);
    if (b) { send(res, b, "application/octet-stream", ref); return true; }
    ociError(res, 404, "BLOB_UNKNOWN", `${ref} is not a file of models/${repo}`); return true;
  }
  // Hugging Face clients follow hub-resolve's relative redirect to here and read the file's metadata from this
  // answer: the commit, and the sha256 as the (linked) ETag, exactly as huggingface.co sends them.
  const hfMeta = { "x-repo-commit": m.rev, etag: `"${ref.slice(7)}"`, "x-linked-etag": `"${ref.slice(7)}"`, "x-linked-size": String(blob.size) };
  if (blob.held) { const b = await held(ref); if (b) { if (req.method === "HEAD") { res.writeHead(200, { ...hfMeta, "docker-content-digest": ref, "content-length": b.length, "content-type": "application/octet-stream" }); res.end(); return true; } res.setHeader("x-repo-commit", m.rev); send(res, b, "application/octet-stream", ref); return true; } }
  const q = new URL(req.url, "http://x").searchParams;
  const force = space === "tensors" ? "tensors" : q.get("source") || req.headers["x-hub-source"];
  const base = { ...hfMeta, "docker-content-digest": ref, "docker-distribution-api-version": "registry/2.0", "accept-ranges": "bytes" };
  if (req.method === "HEAD") { res.writeHead(200, { ...base, "content-length": blob.size, "content-type": "application/octet-stream" }); res.end(); return true; }

  if (blob.upstream && force !== "tensors" && await hfUp()) {
    res.writeHead(307, { ...base, location: `${HF}/${repo}/resolve/${m.rev}/${blob.path.split("/").map(encodeURIComponent).join("/")}`, "x-hub-source": "huggingface.co", "cache-control": "no-store", "content-length": "0" });
    res.end(); return true;
  }
  if (!blob.layout) { ociError(res, 503, "UNAVAILABLE", `${blob.path} has no tensor layout and its upstream is down`); return true; }
  const layout = JSON.parse((await held(blob.layout))?.toString("utf8") || "null");
  if (!layout) { ociError(res, 500, "UNKNOWN", `layout ${blob.layout} not held`); return true; }

  let start = 0, end = layout.size - 1, status = 200;
  const r = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || "");
  if (r) { start = r[1] ? +r[1] : layout.size - +r[2]; end = r[1] && r[2] ? Math.min(+r[2], layout.size - 1) : r[1] ? layout.size - 1 : layout.size - 1; status = 206; }
  if (start > end || start >= layout.size) { res.writeHead(416, { "content-range": `bytes */${layout.size}` }); res.end(); return true; }
  res.writeHead(status, { ...base, "content-type": "application/octet-stream", "content-length": end - start + 1, "x-hub-source": "tensors", "cache-control": "no-store",
    ...(status === 206 ? { "content-range": `bytes ${start}-${end}/${layout.size}` } : {}) });
  const ctx = { origin: HF, repo, rev: m.rev, literal: held, alternatives: (k) => alts.get(k) || [], prefer: force === "tensors" ? "alternatives" : undefined };
  try {
    for await (const c of assemble(layout, ctx, { start, end })) if (!res.write(c)) await new Promise((ok) => res.once("drain", ok));
    res.end();
  } catch (e) { res.destroy(e); }                                  // a tensor that fails verification ends the transfer; the client's digest check refuses it
  return true;
}

// For hub-resolve: the "tensors" source of a model at a revision, if the tensor index holds it. Files the index
// cannot rebuild (no layout, not held) are listed as missing, exactly like the other sources' gaps.
// `files` are hub-resolve rows: [path, size, "sha256:…", weights, url].
export async function tensorSource(id, revision, files) {
  const { models, lower } = await load();
  const repo = models[id] ? id : lower.get(id.toLowerCase());
  const m = repo && models[repo];
  if (!m || !m.gated || m.rev !== revision) return null;
  const can = (addr) => Boolean(m.blobs[addr]?.layout || m.blobs[addr]?.held);
  return { kind: "tensors", resolve: `/v2/tensors/${repo.toLowerCase()}/blobs/`, byDigest: true, missing: files.filter((f) => !can(f[2])).map((f) => f[0]) };
}

// The plain name: <host>/<org>/<name> is the address a person copies, so it must also be what every client pulls.
// Registry paths /v2/<org>/<name>/… are shared with hub-resolve's Ollama dialect, so this answers only what the
// tensor index holds (format tags, the bare name, known digests) and returns false for everything else — a quant
// tag such as :q8_0 falls through to Ollama exactly as before.
export async function tensorMirrorRoot(req, res, path) {
  const mm = path.match(/^\/v2\/([^/]+\/[^/]+)\/(manifests|blobs|tags)\/([^/]+)$/);
  if (!mm) return false;
  const [, name, kind, ref] = mm;
  const { models, lower } = await load();
  const repo = models[name] ? name : lower.get(name.toLowerCase()), m = repo && models[repo];
  if (!m || !m.gated) return false;
  if (kind === "manifests") { const d = resolveRef(m, ref); if (!d || !(d === m.index || Object.values(m.manifests).includes(d))) return false; }
  if (kind === "blobs" && !m.blobs[ref] && ref !== m.table && !(await held(ref))) return false;
  if (kind === "tags" && ref !== "list") return false;
  return tensorMirror(req, res, `/v2/models/${name}/${kind}/${ref}`);
}

// The address is also a link. A browser opening <host>/<org>/<name> is sent to the model's page, with the name's
// real capitalisation; `known(id)` lets the caller add other catalogues (hub-resolve passes its own index).
export async function modelPageRedirect(req, res, path, known = async () => null) {
  const mm = path.match(/^\/([^/]+)\/([^/]+)\/?$/);
  if (!mm || req.method !== "GET" || !/text\/html/.test(req.headers.accept || "")) return false;
  const id = `${decodeURIComponent(mm[1])}/${decodeURIComponent(mm[2])}`;
  const { models, lower } = await load();
  const repo = (models[id] ? id : lower.get(id.toLowerCase())) || (await known(id).catch(() => null));
  const location = repo ? `/models/${repo.split("/").map(encodeURIComponent).join("/")}/` : `/models/?q=${encodeURIComponent(id)}`;
  res.writeHead(302, { location, "cache-control": "no-store", "content-length": "0" });
  res.end();
  return true;
}
