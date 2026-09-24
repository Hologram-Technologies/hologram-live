// holo-spaces-hf-fetch.mjs — VERIFIED model-file fetch over the Hugging Face dialect (Spaces slice 0).
//
// A Space names its model by id (`onnx-community/Kokoro-82M-v1.0-ONNX`); the runtime (transformers.js,
// kokoro-js, raw fetch) pulls files by URL from whatever HF-dialect host is configured — huggingface.co
// today, hub.uor.foundation once the model is indexed there (both speak `/api/models/{id}/tree/{rev}`
// and `/{id}/resolve/{rev}/{path}`, both send CORS). The HOST is a location; it is never the identity.
// The identity of every file is the digest the TREE names for it, and this module makes that digest
// the gate (Law L5): a byte that does not re-derive to its tree digest never reaches the model.
//
//   • LFS files carry `lfs.oid` = sha-256 of the bytes — the κ on the open-web axis (= CIDv1 sha2-256,
//     = the registry's blob digest). hub.uor.foundation normalises every file's `oid` to sha-256.
//   • Non-LFS files on huggingface.co carry the GIT BLOB sha-1 (`sha1("blob <len>\0" + bytes)`), so a
//     small config/tokenizer is verified on that axis — still a digest of the bytes named by the index,
//     never by the host that served them.
//
// Pure + isomorphic: `expectedFor` / `verifyBytes` / `digestOf` are plain functions (Node witness); the
// browser pieces (`verifiedFetch`, `opfsModelCache`) take their `fetch`/store injected (Law L4).
//
//   const idx  = await loadTree(fetch, host, modelId, "main");   // { modelId, rev, files: {path → {axis, hex, size}} }
//   const vf   = verifiedFetch(fetch, { trees: [idx], cache });    // drop-in for the runtime's global fetch
//   env.useCustomCache = true; env.customCache = opfsModelCache(store, { trees: [idx] });

export const VERSION = "holo-spaces-hf-fetch/0.1.0";

// ── digests (WebCrypto in the browser/worker, node:crypto in Node — the same bytes → the same hex) ──
const hex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
async function digestWith(alg, u8) {
  if (globalThis.crypto && globalThis.crypto.subtle) return hex(await crypto.subtle.digest(alg, u8));
  const { createHash } = await import("node:crypto");
  return createHash(alg === "SHA-1" ? "sha1" : "sha256").update(u8).digest("hex");
}
export async function sha256Of(bytes) { return digestWith("SHA-256", bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)); }
// git's blob object id: sha-1 over the header "blob <size>\0" followed by the bytes.
export async function gitBlobSha1Of(bytes) {
  const u = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const head = new TextEncoder().encode(`blob ${u.byteLength}\0`);
  const all = new Uint8Array(head.byteLength + u.byteLength); all.set(head, 0); all.set(u, head.byteLength);
  return digestWith("SHA-1", all);
}
export async function digestOf(bytes, axis) { return axis === "sha256" ? sha256Of(bytes) : axis === "gitsha1" ? gitBlobSha1Of(bytes) : null; }

// ── the tree → expected digests. One index per model; the axis is decided per file by what the tree
//    names (sha-256 when it is 64 hex — LFS on HF, everything on the hub — else the git sha-1). ──
export function indexFromTree(modelId, rev, entries) {
  const files = {};
  for (const e of entries || []) {
    if (!e || e.type !== "file" || !e.path) continue;
    const lfs = e.lfs && /^[0-9a-f]{64}$/i.test(String(e.lfs.oid || "")) ? String(e.lfs.oid).toLowerCase() : null;
    const oid = String(e.oid || "").toLowerCase();
    if (lfs) files[e.path] = { axis: "sha256", hex: lfs, size: e.size ?? e.lfs.size ?? null };
    else if (/^[0-9a-f]{64}$/.test(oid)) files[e.path] = { axis: "sha256", hex: oid, size: e.size ?? null };
    else if (/^[0-9a-f]{40}$/.test(oid)) files[e.path] = { axis: "gitsha1", hex: oid, size: e.size ?? null };
  }
  return { modelId, rev: rev || "main", files };
}
export async function loadTree(fetchImpl, host, modelId, rev = "main") {
  const base = String(host).replace(/\/+$/, "");
  const r = await fetchImpl(`${base}/api/models/${modelId}/tree/${encodeURIComponent(rev)}?recursive=true`, { cache: "no-store" });
  if (!r.ok) throw new Error(`tree ${modelId}@${rev}: HTTP ${r.status}`);
  return indexFromTree(modelId, rev, await r.json());
}

// ── URL → (index, path). Matches `/{owner}/{name}/resolve/{rev}/{path}` on ANY host, and the
//    redirect targets HF uses (`/api/resolve-cache/models/{id}/{commit}/{path}`, cdn-lfs hosts carry the
//    path in the query). A URL that names no indexed file is NOT ours — passed through untouched. ──
export function expectedFor(url, trees) {
  let u; try { u = new URL(url); } catch { return null; }
  const p = decodeURIComponent(u.pathname);
  for (const idx of trees || []) {
    const id = idx.modelId;
    let m = p.match(new RegExp(`^/${id.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")}/resolve/[^/]+/(.+)$`));
    if (!m) m = p.match(new RegExp(`^/api/resolve-cache/models/${id.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")}/[0-9a-f]{40}/(.+)$`));
    if (!m) { const q = u.search.match(/[?&]response-content-disposition=[^&]*filename%3D%22?([^%&"]+)/i); if (q) m = [null, q[1]]; }
    if (!m) continue;
    const path = m[1].replace(/^\/+/, "");
    const f = idx.files[path];
    if (f) return { modelId: id, path, ...f };
  }
  return null;
}
export async function verifyBytes(bytes, expected) {
  if (!expected) return { ok: false, reason: "no-expectation" };
  const got = await digestOf(bytes, expected.axis);
  return got === expected.hex ? { ok: true, axis: expected.axis, hex: got } : { ok: false, reason: "digest-mismatch", axis: expected.axis, want: expected.hex, got };
}

// ── verifiedFetch — the drop-in. For a URL that resolves to an indexed file: serve from the κ-store if
//    held (by its digest — location-free, Law L1), else fetch, buffer, re-derive, REFUSE a mismatch
//    (throws; the runtime sees a failed load, never wrong bytes), store, and answer. Range requests are
//    honoured out of the verified whole (the runtime may read weights in slices); everything else passes
//    through to the real fetch untouched. `onEvent` narrates {kind, path, axis, bytes, ms, source}. ──
export function verifiedFetch(rawFetch, { trees = [], cache = null, onEvent = null } = {}) {
  const say = (e) => { try { onEvent && onEvent(e); } catch {} };
  const key = (x) => `${x.axis}:${x.hex}`;
  return async function fetchVerified(input, init) {
    const url = typeof input === "string" ? input : (input && input.url) || String(input);
    const exp = expectedFor(url, trees);
    if (!exp) return rawFetch(input, init);
    const t0 = Date.now();
    let bytes = null, source = "cache";
    if (cache) { try { bytes = await cache.get(key(exp)); } catch { bytes = null; } }
    if (!bytes) {
      source = "network";
      const r = await rawFetch(url, { ...(init || {}), headers: stripRange(init && init.headers) });
      if (!r.ok) { say({ kind: "http-error", path: exp.path, status: r.status }); return r; }
      const u8 = new Uint8Array(await r.arrayBuffer());
      const v = await verifyBytes(u8, exp);
      if (!v.ok) { say({ kind: "refused", path: exp.path, ...v }); throw new Error(`refused ${exp.path}: ${v.reason} (${exp.axis} want ${exp.hex.slice(0, 12)}… got ${String(v.got || "").slice(0, 12)}…)`); }
      bytes = u8;
      if (cache) { try { await cache.put(key(exp), u8); } catch (e) { say({ kind: "cache-put-failed", path: exp.path, error: String(e && e.message || e) }); } }
    }
    say({ kind: "served", path: exp.path, axis: exp.axis, bytes: bytes.byteLength, ms: Date.now() - t0, source });
    return sliceResponse(bytes, init && init.headers, exp.path);
  };
}
function stripRange(h) { if (!h) return h; const o = new Headers(h); o.delete("range"); return o; }
function sliceResponse(bytes, headers, path) {
  const range = headers ? new Headers(headers).get("range") : null;
  const type = /\.json$/i.test(path) ? "application/json" : "application/octet-stream";
  const m = range && range.match(/^bytes=(\d*)-(\d*)$/);
  if (m) {
    const total = bytes.byteLength;
    const start = m[1] === "" ? Math.max(0, total - Number(m[2])) : Number(m[1]);
    const end = m[1] !== "" && m[2] !== "" ? Math.min(total - 1, Number(m[2])) : total - 1;
    return new Response(bytes.subarray(start, end + 1), { status: 206, headers: { "content-type": type, "content-range": `bytes ${start}-${end}/${total}`, "content-length": String(end - start + 1), "accept-ranges": "bytes" } });
  }
  return new Response(bytes, { status: 200, headers: { "content-type": type, "content-length": String(bytes.byteLength), "accept-ranges": "bytes" } });
}

// ── opfsModelCache — transformers.js `env.customCache` over a κ-store: `match(url)` answers from the
//    store BY DIGEST (never by URL, Law L1), `put(url, response)` stores only bytes that re-derive.
//    The store contract is the OS's (`getByKey(axis, hex)` / `putVerified(axis, hex, bytes)` — see
//    holo-opfs-kappastore.mjs); a Map-backed store works for witnesses. ──
export function kappaCache(store) {
  const get = async (k) => { const [axis, h] = k.split(":"); return store.getByKey ? store.getByKey(axis, h) : (store.get ? store.get(k) : null); };
  const put = async (k, u8) => { const [axis, h] = k.split(":"); return store.putVerified ? store.putVerified(axis, h, u8) : store.set(k, u8); };
  return { get, put };
}
export function opfsModelCache(store, { trees = [] } = {}) {
  const c = kappaCache(store);
  return {
    async match(url) { const exp = expectedFor(url, trees); if (!exp) return undefined; const b = await c.get(`${exp.axis}:${exp.hex}`); return b ? sliceResponse(b, null, exp.path) : undefined; },
    async put(url, response) { const exp = expectedFor(url, trees); if (!exp) return; const u8 = new Uint8Array(await response.clone().arrayBuffer()); const v = await verifyBytes(u8, exp); if (v.ok) await c.put(`${exp.axis}:${exp.hex}`, u8); },
  };
}

// ── installVerifiedFetch — patch this context's global `fetch` SYNCHRONOUSLY (so a bundle that runs
//    right after, in a page or a worker, is already covered) with a wrapper that waits for the model
//    trees to load and then delegates to `verifiedFetch`. Until the trees arrive, URLs are held, not
//    passed through: no model byte can slip by unverified during the window. Returns the ready promise. ──
//    `pinned` — digests sealed at publish time ({ modelId: { path: { axis, hex, size } } }): with it, no live
//    index read is needed and the expectation cannot drift; `hosts` (first that answers) is the fallback
//    when a model is not pinned. `altHosts` — other HF-dialect hosts to try the same path on when the
//    app's own URL fails or its bytes are refused (each retry is verified against the same digest).
export function indexFromPinned(modelId, files) { return { modelId, rev: "main", files: { ...files } }; }
export function installVerifiedFetch({ host, hosts, models, pinned = null, altHosts = [], store = null, onEvent = null, g = globalThis } = {}) {
  const raw = g.fetch.bind(g);
  const ids = Array.isArray(models) ? models : [models];
  const HOSTS = hosts || (host ? [host] : []);
  const say = (e) => { try { onEvent && onEvent(e); } catch {} };
  const treeOf = async (id) => {
    if (pinned && pinned[id]) return indexFromPinned(id, pinned[id]);
    let last = null;
    for (const h of HOSTS) { try { return await loadTree(raw, h, id, "main"); } catch (e) { last = e; say({ kind: "index-miss", host: h, model: id }); } }
    throw last || new Error("no host for " + id);
  };
  const ready = Promise.all(ids.map(treeOf)).then((trees) => {
    const vf = verifiedFetch(raw, { trees, cache: store ? kappaCache(store) : null, onEvent });
    if (!altHosts.length) return vf;
    return async (input, init) => {
      const url = typeof input === "string" ? input : (input && input.url) || String(input);
      const exp = expectedFor(url, trees);
      try { const r = await vf(input, init); if (r.ok || !exp) return r; } catch (e) { if (!exp) throw e; say({ kind: "retry", path: exp.path, error: String(e && e.message || e) }); }
      let last = null;
      for (const h of altHosts) {
        const alt = `${String(h).replace(/\/+$/, "")}/${exp.modelId}/resolve/main/${exp.path}`;
        try { const r = await vf(alt, init); if (r.ok) { say({ kind: "served-alt", path: exp.path, host: h }); return r; } } catch (e) { last = e; say({ kind: "retry", path: exp.path, host: h, error: String(e && e.message || e) }); }
      }
      throw last || new Error(`no source served ${exp.path}`);
    };
  });
  g.fetch = async (input, init) => (await ready)(input, init);
  return ready;
}

export default { VERSION, sha256Of, gitBlobSha1Of, digestOf, indexFromTree, loadTree, expectedFor, verifyBytes, verifiedFetch, kappaCache, opfsModelCache, installVerifiedFetch };
