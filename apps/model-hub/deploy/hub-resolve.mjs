#!/usr/bin/env node
// hub-resolve: the Hugging Face dialect of the Model Hub, so HF_ENDPOINT=https://hub.uor.foundation works with every
// tool that downloads models. It answers metadata from the hub's published file lists and sends every file request to
// a source that is alive, as a redirect: no weight byte passes through this process. One file, no dependencies.
//
// Recorded from huggingface_hub 1.32 (web/qa/hf-dialect/recorder.mjs), the whole dialect a download needs:
//   GET  [/via/<source>]/api/models/<org>/<name>[/revision/<rev>]          model info: sha, siblings
//   GET  [/via/<source>]/api/models/<org>/<name>/tree/<rev>[/<dir>]        file listing
//   HEAD [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 + X-Repo-Commit, X-Linked-ETag, X-Linked-Size
//   GET  [/via/<source>]/<org>/<name>/resolve/<rev>/<path>                 302 to the chosen source (Range is re-sent there)
//   GET  …/resolve/<rev>/SHA256SUMS                                        generated: `sha256sum -c` checks a download
//   GET  …/api/models/<org>/<name>/xet-read-token/<rev>                    307 to huggingface.co (its token, not ours)
// Older clients read those three headers from our 302 and GET the Location; huggingface_hub 1.32 follows the redirect
// on HEAD too, so while Hugging Face is the source it meets Hugging Face's own Xet headers and downloads through Xet
// (hence the token route). We never send an X-Xet-* header ourselves: from ModelScope and IPFS the client uses plain
// HTTP. Authorization is stripped by the front Caddy and never read here. `/via/ipfs`, `/via/modelscope.cn`, `/via/huggingface.co` pin the first choice of source.
//
// The source for a file: one that has the file and passed the last health probe, in the order Hugging Face (its CDN
// is the fastest), ModelScope, IPFS. Probes run in the background; a request never waits for one.
import http from "node:http";
import { createHash } from "node:crypto";
import { appendFile, readFile } from "node:fs/promises";
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
const revisionOk = (doc, rev) => rev === "main" || (rev.length >= 7 && doc.revision.startsWith(rev));

http.createServer(async (req, res) => {
  try {
    if (req.method !== "GET" && req.method !== "HEAD") return refuse(res, 405, "ReadOnly", "The hub endpoint is read-only.");
    const url = new URL(req.url, "http://hub");
    let path = decodeURIComponent(url.pathname), via = url.searchParams.get("source");
    const prefix = path.match(/^\/via\/([a-z.]+)(\/.*)$/);
    if (prefix) { via = prefix[1] === "modelscope" ? "modelscope.cn" : prefix[1] === "huggingface" ? "huggingface.co" : prefix[1]; path = prefix[2]; }

    if (path === "/api/hub/health") return json(res, 200, { sources: health, order: ORDER }, { "cache-control": "no-store", "access-control-allow-origin": "*" });

    // Measured with huggingface_hub 1.32: the client follows our redirect on HEAD, meets Hugging Face's Xet headers
    // there, and then asks *this* endpoint for the Xet read token. The token is Hugging Face's to give: send the
    // client there. (Other sources carry no Xet headers, so this only happens while Hugging Face is the source.)
    const xet = path.match(/^\/api\/models\/[^/]+\/[^/]+\/xet-read-token\/[^/]+$/);
    if (xet) { res.writeHead(307, { location: `https://huggingface.co${path}`, "cache-control": "no-store", "content-length": "0" }); return res.end(); }

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
