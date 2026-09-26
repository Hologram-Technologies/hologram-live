// Verified bytes: the one check the clients do not make. huggingface_hub checks a length, transformers.js checks nothing,
// llama.cpp skips its etag for Hugging Face (MODEL-CLIENTS-COVERAGE-RESEARCH.md). So when hub-resolve runs on the
// user's own machine (HUB_VERIFY=1), it does not redirect a client to a holder: it fetches the file itself and passes
// it on as it lands, hashing it, and holds back the last HOLD bytes until the whole equals the sha256 the index names.
// On a mismatch the connection is cut before those bytes, so the client can never finish a wrong file (every client
// checks the length it was promised); the holder that lied is skipped on the client's retry.
//
// Streaming matters: huggingface_hub gives up after 10 s without a byte, which a model file takes to fetch whole.
// The cache is content-addressed (<cache>/sha256/<hex>): a file is fetched once for every model and name that share
// it, and a cached copy is re-hashed the first time this process serves it (a disk can rot, a user can edit).
import { createHash, randomBytes } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, rename, rm, stat } from "node:fs/promises";
import { join } from "node:path";
import { pipeline } from "node:stream/promises";

const HOLD = 64 * 1024;
const inflight = new Map();     // hex -> Promise<path>: one fetch per file, however many clients ask
const checked = new Set();      // hex re-hashed by this process
const liars = new Map();        // hex -> Set of holder kinds that sent other bytes

async function hashFile(path) {
  const h = createHash("sha256");
  await pipeline(createReadStream(path), async function* (src) { for await (const c of src) h.update(c); });
  return h.digest("hex");
}

async function cached(cache, hex, size) {
  const path = join(cache, "sha256", hex);
  const have = await stat(path).catch(() => null);
  if (!have || have.size !== size) return null;
  if (checked.has(hex) || (await hashFile(path)) === hex) { checked.add(hex); return path; }
  await rm(path, { force: true });
  return null;
}

// Fetch one holder's bytes into the cache, hashing them; `out(chunk)` sees every byte but the last HOLD, `tail` is
// given the held bytes only when they are right. Returns the cached path, or null with the reason in `tried`.
async function fetchOne(cache, hex, size, { url, kind }, tried, out, log) {
  const dir = join(cache, "sha256"), tmp = join(dir, `.${hex}.${randomBytes(4).toString("hex")}`);
  await mkdir(dir, { recursive: true });
  const file = createWriteStream(tmp);
  try {
    const r = await fetch(url, { redirect: "follow", signal: AbortSignal.timeout(60 * 60_000), headers: { "user-agent": "hologram-verify" } });
    if (!r.ok || !r.body) { tried.push(`${kind}: HTTP ${r.status}`); file.destroy(); await rm(tmp, { force: true }); return null; }
    const h = createHash("sha256");
    let n = 0, held = Buffer.alloc(0);
    for await (const chunk of r.body) {
      const c = Buffer.from(chunk);
      h.update(c); n += c.length;
      if (n > size) throw new Error(`sent more than the ${size} bytes the index names`);
      if (!file.write(c)) await new Promise((ok) => file.once("drain", ok));
      held = Buffer.concat([held, c]);
      if (held.length > HOLD) { await out(held.subarray(0, held.length - HOLD)); held = held.subarray(held.length - HOLD); }
    }
    await new Promise((ok, no) => file.end((e) => (e ? no(e) : ok())));
    const got = h.digest("hex");
    if (got !== hex || n !== size) {
      tried.push(`${kind}: sent ${n} bytes hashing to ${got.slice(0, 12)}…, refused`);
      if (!liars.has(hex)) liars.set(hex, new Set());
      liars.get(hex).add(kind);
      log({ refused: kind, want: hex, got, bytes: n });
      await rm(tmp, { force: true });
      return null;
    }
    const path = join(dir, hex);
    await rename(tmp, path);
    checked.add(hex);
    log({ verified: hex, from: kind, bytes: n });
    return { path, held };
  } catch (e) {
    tried.push(`${kind}: ${e.message}`);
    file.destroy();
    await rm(tmp, { force: true }).catch(() => {});
    return e.cut ? { cut: true } : null;
  }
}

// The path of a local copy whose sha256 is `hex`, from the first of `holders` that sends exactly those bytes.
export function verified(cache, hex, size, holders, log = () => {}) {
  if (!inflight.has(hex)) inflight.set(hex, (async () => {
    const hit = await cached(cache, hex, size);
    if (hit) return hit;
    const tried = [];
    for (const h of holders.filter((x) => !liars.get(hex)?.has(x.kind))) {
      const got = await fetchOne(cache, hex, size, h, tried, async () => {}, log);
      if (got?.path) return got.path;
    }
    throw new Error(`no holder sent bytes matching sha256:${hex} (${tried.join("; ") || "no holder"})`);
  })().finally(() => inflight.delete(hex)));
  return inflight.get(hex);
}

// Answer a GET for the whole file while it is being fetched: every byte passes through as it lands, the last HOLD
// only once the whole is right. A Range, a HEAD or a file already cached or being fetched goes through the cache.
export async function streamVerified(req, res, cache, hex, size, holders, headers = {}, log = () => {}) {
  const hit = inflight.has(hex) ? null : await cached(cache, hex, size);
  if (hit || req.method === "HEAD" || req.headers.range || inflight.has(hex)) {
    const path = hit || (req.method === "HEAD" ? null : await verified(cache, hex, size, holders, log));
    if (!path) { res.writeHead(200, { "content-length": size, "accept-ranges": "bytes", "content-type": "application/octet-stream", ...headers }); return res.end(); }
    return sendFile(req, res, path, size, headers);
  }
  let settle;
  inflight.set(hex, new Promise((ok, no) => { settle = { ok, no }; }).finally(() => inflight.delete(hex)));
  inflight.get(hex).catch(() => {});
  const tried = [];
  let started = false;
  try {
    for (const h of holders.filter((x) => !liars.get(hex)?.has(x.kind))) {
      const out = async (bytes) => {
        if (!started) { started = true; res.writeHead(200, { "content-type": "application/octet-stream", "content-length": size, "accept-ranges": "bytes", ...headers }); }
        if (res.destroyed) { const e = new Error("the client went away"); e.cut = true; throw e; }
        if (!res.write(bytes)) await new Promise((ok) => res.once("drain", ok));
      };
      const got = await fetchOne(cache, hex, size, h, tried, out, log);
      if (got?.path) {
        if (!started) { res.writeHead(200, { "content-type": "application/octet-stream", "content-length": size, "accept-ranges": "bytes", ...headers }); started = true; }
        res.end(got.held);
        settle.ok(got.path);
        return;
      }
      if (started) { res.destroy(); settle.no(new Error(tried.join("; "))); return; }   // bytes went out: cut, never finish
    }
    const message = `no holder sent bytes matching sha256:${hex} (${tried.join("; ") || "no holder"})`;
    settle.no(new Error(message));
    const body = JSON.stringify({ error: message });
    res.writeHead(502, { "content-type": "application/json", "content-length": Buffer.byteLength(body), "x-error-code": "NoVerifiedSource", "x-error-message": message.slice(0, 300) });
    res.end(body);
  } catch (e) {
    settle.no(e);
    res.destroy();
  }
}

// Serve a verified local file, with Range (206) as every resuming client expects.
export async function sendFile(req, res, path, size, headers = {}) {
  const m = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || "");
  let from = 0, to = size - 1, status = 200;
  if (m && (m[1] || m[2])) {
    from = m[1] ? Number(m[1]) : Math.max(0, size - Number(m[2]));
    to = m[1] && m[2] ? Math.min(Number(m[2]), size - 1) : size - 1;
    if (from > to || from >= size) { res.writeHead(416, { "content-range": `bytes */${size}` }); return res.end(); }
    status = 206;
  }
  res.writeHead(status, { "content-type": "application/octet-stream", "content-length": to - from + 1, "accept-ranges": "bytes", ...(status === 206 ? { "content-range": `bytes ${from}-${to}/${size}` } : {}), ...headers });
  if (req.method === "HEAD" || size === 0) return res.end();
  await pipeline(createReadStream(path, { start: from, end: to }), res).catch(() => {});
}
