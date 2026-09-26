// Rebuild any file from its layout: literals from the hub, tensors from wherever they live, each segment
// checked against its κ before a byte of it is released. Used by the hub (tensor-mirror.mjs) and by clients.
// No dependencies beyond node:crypto and fetch, so the same file runs on the VPS, in a CLI, and (with a
// Web Crypto shim) in the OS service worker.
//
// layout.segments entries:
//   ["l", κ, len]                           literal, held by the hub
//   ["t", κ, len, off]                      original layout: tensor payload at `off` in this same file
//   ["s", κ, len, off]                      original layout: raw storage at `off` in this same file
//   ["t", κ, len, { f, o }]                 render: tensor payload at offset o of file f of the model
//   ["v", κ, len, { f, o, n, s, sub }]      render: tensor viewed inside storage s (n bytes at o of file f)
// Alternative holders of a κ come from ctx.alternatives(κ): { repo, rev, path, off, len } (a range in another
// file on the origin) or { url, off: 0, len } (the payload as its own object, e.g. an IPFS gateway path by CID).
import { createHash } from "node:crypto";

// segments up to this size are verified before release; larger ones stream and abort on mismatch. Hosts with little
// memory lower it (TENSOR_MAX_BUFFER_MB) and the read window (TENSOR_WINDOW).
const MAX_BUFFER = Number(globalThis.process?.env?.TENSOR_MAX_BUFFER_MB || 256) * 2 ** 20;
const UA = { "user-agent": "hologram-tensor-assemble/0.1" };
const cdn = new Map();
// A host that refused a connection is skipped for a minute, so a dead origin costs one failure, not one per read.
const down = new Map();
const hostOf = (u) => { try { return new URL(u).host; } catch { return ""; } };
const isDown = (u) => (down.get(hostOf(u)) || 0) > Date.now();
const markDown = (u) => down.set(hostOf(u), Date.now() + 60_000);

async function cdnUrl(origin, repo, rev, path, fresh) {
  const key = `${repo}@${rev}/${path}`, c = cdn.get(key);
  if (c && !fresh && Date.now() - c.at < 30 * 60_000) return c.url;
  let url = `${origin}/${repo}/resolve/${rev}/${path.split("/").map(encodeURIComponent).join("/")}`;
  for (let hop = 0; hop < 4; hop++) {
    const r = await fetch(url, { redirect: "manual", headers: UA, signal: AbortSignal.timeout(15_000) });
    const loc = r.headers.get("location"); r.body?.cancel();
    if (r.status >= 300 && r.status < 400 && loc) { const abs = new URL(loc, url).href; if (new URL(abs).origin === new URL(origin).origin) { url = abs; continue; } cdn.set(key, { url: abs, at: Date.now() }); return abs; }
    if (r.ok) { cdn.set(key, { url, at: Date.now() }); return url; }
    throw new Error(`resolve ${key}: ${r.status}`);
  }
  throw new Error(`resolve ${key}: redirect loop`);
}

// Read bytes [a, a+len) of `path` in repo@rev as one buffer or a stream of chunks.
// `direct` is a URL that serves the bytes itself (e.g. an IPFS gateway path to the payload's own CID).
async function* readRange(origin, repo, rev, path, a, len, direct) {
  let pos = a; const end = a + len - 1;
  for (let k = 0; k < (direct ? 2 : 5) && pos <= end; k++) {
    if (isDown(direct || origin)) break;
    try {
      const url = direct || await cdnUrl(origin, repo, rev, path, k > 0);
      const r = await fetch(url, { headers: { ...UA, range: `bytes=${pos}-${end}` }, signal: AbortSignal.timeout(600_000) });
      if (r.status !== 206 && !(r.status === 200 && pos === 0)) { r.body?.cancel(); throw new Error(`status ${r.status}`); }
      for await (const c of r.body) { const b = Buffer.from(c); const take = Math.min(b.length, end - pos + 1); if (take > 0) { yield b.subarray(0, take); pos += take; } if (pos > end) break; }
    } catch (e) {
      const code = e?.cause?.code || e?.code;
      if (["ECONNREFUSED", "ENOTFOUND", "EHOSTUNREACH"].includes(code)) { markDown(direct || origin); break; }
      await new Promise((r) => setTimeout(r, 2 ** k * 500));
    }
  }
  if (pos <= end) throw new Error(`could not read ${direct || path} ${a}+${len}`);
}
async function readAll(it) { const parts = []; for await (const c of it) parts.push(c); return Buffer.concat(parts); }
const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
const label = (src, off) => (src.url ? src.url : `${src.repo}/${src.path}@${off ?? src.off}`);

export function gather(buf, sub, shape, len) {
  const { o, stride, itemsize } = sub;
  if (!stride) return buf.subarray(o, o + len);
  const out = Buffer.allocUnsafe(len), nd = shape.length, idx = new Array(nd).fill(0);
  for (let k = 0, n = len / itemsize; k < n; k++) {
    let off = o; for (let d = 0; d < nd; d++) off += idx[d] * stride[d] * itemsize;
    buf.copy(out, k * itemsize, off, off + itemsize);
    for (let d = nd - 1; d >= 0; d--) { if (++idx[d] < shape[d]) break; idx[d] = 0; }
  }
  return out;
}

// Candidate places to read one payload segment, best first: its own location, then every other known holder.
function candidates(ctx, layout, seg) {
  const [kind, kappa, len, where] = seg;
  const out = [];
  if (typeof where === "number") out.push({ repo: ctx.repo, rev: ctx.rev, path: layout.file, off: where });
  else if (where && kind === "t") out.push({ repo: ctx.repo, rev: ctx.rev, path: where.f, off: where.o });
  for (const alt of ctx.alternatives?.(kappa) || []) if (alt.len === len && !out.some((x) => x.url === alt.url && x.repo === alt.repo && x.path === alt.path && x.off === alt.off)) out.push(alt);
  if (ctx.prefer === "alternatives") out.push(out.shift());       // demo / failover: try other holders first
  return out;
}

// Yields the file's bytes. Each segment is verified; ctx.report(event) receives per-segment outcomes.
// ctx: { origin, repo, rev, literal(κ) -> Buffer, alternatives(κ) -> [{repo,rev,path,off,len}], report?, prefer? }
// Speed: adjacent payloads that live contiguously in one source file are read as one Range request (up to
// GROUP bytes), and up to WINDOW reads are in flight at once; bytes are still released in order, each segment
// only after its κ checks. A segment that fails from its first source is retried from every other holder.
const GROUP = Math.min(64 << 20, MAX_BUFFER), GAP = 1 << 20, WINDOW = Number(globalThis.process?.env?.TENSOR_WINDOW || 4);
export async function* assemble(layout, ctx, { start = 0, end = layout.size - 1 } = {}) {
  const origin = ctx.origin || "https://huggingface.co";
  // 1. units in file order: literal | view | big (streamed) | group (one read, many segments)
  const units = []; let pos = 0, cur = null;
  for (const seg of layout.segments) {
    const [kind, kappa, len, where] = seg, a = pos; pos += len;
    if (a + len - 1 < start || a > end || len === 0) { cur = null; continue; }
    const item = { seg, a, len };
    if (kind === "l") { units.push({ type: "l", items: [item] }); cur = null; continue; }
    if (kind === "v") { units.push({ type: "v", items: [item] }); cur = null; continue; }
    if (len > MAX_BUFFER) { units.push({ type: "big", items: [item] }); cur = null; continue; }
    const src = candidates(ctx, layout, seg)[0];
    item.src = src;
    // join the current read when this payload follows it in the same source file (small gaps are read and dropped)
    if (cur && src && cur.src && !src.url && !cur.src.url && cur.src.repo === src.repo && cur.src.path === src.path && src.off >= cur.end && src.off - cur.end <= GAP && src.off + len - cur.start <= GROUP) {
      item.rel = src.off - cur.start; cur.items.push(item); cur.end = src.off + len;
    } else { cur = { type: "g", src, start: src ? src.off : 0, end: src ? src.off + len : 0, items: [item] }; item.rel = 0; units.push(cur); }
  }
  const cut = (it, buf) => buf.subarray(Math.max(0, start - it.a), Math.min(it.len, end - it.a + 1));
  // One segment from any holder but `skip`, verified.
  const fallback = async (it, skip) => {
    const [, kappa, len] = it.seg; const tried = [];
    for (const src of candidates(ctx, layout, it.seg)) {
      if (skip && !src.url && src.repo === skip.repo && src.path === skip.path) continue;
      const from = label(src); tried.push(from);
      try {
        const buf = await readAll(readRange(origin, src.repo, src.rev, src.path, src.off, len, src.url));
        if (sha(buf) === kappa) { ctx.report?.({ kappa, len, from, ok: true }); return buf; }
        ctx.report?.({ kappa, len, from, ok: false });
      } catch (e) { ctx.report?.({ kappa, len, from, ok: false, error: e.message }); }
    }
    throw new Error(`no source verified ${kappa} (tried ${tried.join(", ") || "none"})`);
  };
  // 2. resolve a unit to verified buffers (groups and literals); views and bigs are handled inline
  const fetchUnit = async (u) => {
    if (u.type === "l") { const [, kappa] = u.items[0].seg; const buf = await ctx.literal(kappa); if (!buf || sha(buf) !== kappa) throw new Error(`literal ${kappa} missing or corrupt`); return [buf]; }
    if (u.type === "v") {
      const [, kappa, len, where] = u.items[0].seg;
      const st = await readAll(readRange(origin, ctx.repo, ctx.rev, where.f, where.o, where.n));
      if (sha(st) !== where.s) throw new Error(`storage ${where.s} failed verification`);
      const bytes = gather(st, where.sub, where.shape, len);
      if (sha(bytes) !== kappa) throw new Error(`tensor ${kappa} failed verification after gather`);
      ctx.report?.({ kappa, len, from: `${ctx.repo}/${where.f}`, ok: true }); return [bytes];
    }
    if (u.type === "g") {
      let whole = null;
      if (u.src) { try { whole = await readAll(readRange(origin, u.src.repo, u.src.rev, u.src.path, u.start, u.end - u.start, u.src.url)); } catch { whole = null; } }
      const out = [];
      for (const it of u.items) {
        const [, kappa, len] = it.seg;
        const piece = whole ? whole.subarray(it.rel, it.rel + len) : null;
        if (piece && sha(piece) === kappa) { ctx.report?.({ kappa, len, from: label(u.src, it.src.off), ok: true }); out.push(piece); }
        else { if (piece) ctx.report?.({ kappa, len, from: label(u.src, it.src.off), ok: false }); out.push(await fallback(it, u.src)); }
      }
      return out;
    }
    return null;
  };
  // 3. in order, with a window of reads in flight
  const pending = [];
  let next = 0;
  const fill = () => { while (pending.length < WINDOW && next < units.length) { const u = units[next++]; pending.push({ u, p: u.type === "big" ? null : fetchUnit(u) }); } };
  fill();
  while (pending.length) {
    const { u, p } = pending.shift();
    if (u.type !== "big") {
      const bufs = await p; fill();
      for (let k = 0; k < u.items.length; k++) yield cut(u.items[k], bufs[k]);
      continue;
    }
    const it = u.items[0], [, kappa, len] = it.seg; let done = false; const tried = [];
    for (const src of candidates(ctx, layout, it.seg)) {                 // large: stream, verify at the end, abort on mismatch
      const from = label(src); tried.push(from);
      try {
        const h = createHash("sha256"); let at = it.a;
        for await (const c of readRange(origin, src.repo, src.rev, src.path, src.off, len, src.url)) {
          h.update(c);
          const lo = Math.max(at, start), hi = Math.min(at + c.length - 1, end);
          if (hi >= lo) yield c.subarray(lo - at, hi - at + 1);
          at += c.length;
        }
        if (`sha256:${h.digest("hex")}` !== kappa) throw Object.assign(new Error(`tensor ${kappa} failed verification after release; aborting`), { fatal: true });
        ctx.report?.({ kappa, len, from, ok: true }); done = true; break;
      } catch (e) { if (e.fatal) throw e; ctx.report?.({ kappa, len, from, ok: false, error: e.message }); }
    }
    if (!done) throw new Error(`no source verified ${kappa} (tried ${tried.join(", ")})`);
    fill();
  }
}
