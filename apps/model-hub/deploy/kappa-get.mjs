// get(κ): any byte range of any κ object, from any holder, every byte checked before it is released.
//
// An object over one piece (1 MiB) carries a piece list: the sha256 of each 1 MiB of it, aligned to its start
// (tensor.pieces in the model's pieces object). A piece is checked alone, so
//   a Range is served by fetching only the pieces it covers, checked, never the whole object;
//   nothing is released before it checks: no hold-back, no abort after release;
//   a holder that lies is blamed for one piece of one κ, its other pieces are kept, and that piece alone goes to
//     the next holder;
//   a piece's sha256 is also its IPFS raw CID (sha2-256, raw codec, <= 1 MiB), so any IPFS node or gateway is a
//     holder with no second index.
// Holders are hints: a wrong one costs one fetch, never a wrong byte.
//
// No dependencies beyond node:crypto and fetch, like tensor-assemble.mjs: the hub, the CLI and (with a Web Crypto
// shim) the OS service worker run the same file.
import { createHash } from "node:crypto";

export const PIECE = 1 << 20;
const RUN = 8 << 20;          // neighbouring pieces read in one Range request from a byte-range holder
const AHEAD = 6;              // runs in flight at once

const hex = (b) => createHash("sha256").update(b).digest("hex");

// Every holder's record in this process: pieces it sent right, pieces it sent wrong. An honest holder never sends
// a wrong byte, so one lie sends a holder to the back of the line for this process: a liar costs a few pieces, not a
// fetch per piece, even one that lies about one piece in a hundred to keep its average up.
export const standing = new Map();
export const note = (holder, ok) => { const s = standing.get(holder) || { ok: 0, lies: 0 }; if (ok) s.ok++; else s.lies++; standing.set(holder, s); };
export const demoted = (holder) => (standing.get(holder)?.lies || 0) > 0;
// A holder must also be fast enough: one that has not delivered a read by this deadline (10 s plus 1 s per MB) is
// passed over like one that lied, and gets the same strike. A slow holder is how a lie is made to cost time.
export const deadline = (len) => 10_000 + Math.ceil(len / 1e3);
export function within(p, ms, what) {
  let t; const late = new Promise((_, no) => { t = setTimeout(() => no(Object.assign(new Error(`${what}: no answer in ${ms} ms`), { slow: true })), ms); });
  return Promise.race([p, late]).finally(() => clearTimeout(t));
}
export const rank = (holders) => [...holders.filter((h) => !demoted(h.kind)), ...holders.filter((h) => demoted(h.kind))];

// Piece hashes of a byte stream, computed as it passes: update() any chunking, finish() -> [hex, ...].
export class PieceHasher {
  constructor() { this.h = createHash("sha256"); this.n = 0; this.out = []; }
  update(buf) {
    let o = 0;
    while (o < buf.length) {
      const take = Math.min(PIECE - this.n, buf.length - o);
      this.h.update(buf.subarray(o, o + take)); this.n += take; o += take;
      if (this.n === PIECE) { this.out.push(this.h.digest("hex")); this.h = createHash("sha256"); this.n = 0; }
    }
  }
  finish() { if (this.n) this.out.push(this.h.digest("hex")); this.n = 0; return this.out; }
}

// CIDv1, raw codec, sha2-256, base32: the IPFS name of a piece, straight from its sha256.
export function cidOf(h) {
  const bytes = [0x01, 0x55, 0x12, 0x20, ...Buffer.from(h, "hex")], A = "abcdefghijklmnopqrstuvwxyz234567";
  let bits = 0, val = 0, out = "b";
  for (const x of bytes) { val = (val << 8) | x; bits += 8; while (bits >= 5) { out += A[(val >>> (bits - 5)) & 31]; bits -= 5; } }
  if (bits) out += A[(val << (5 - bits)) & 31];
  return out;
}

// Yield bytes [from, to] of object `kappa` (size `size`, piece list `pieces`), each piece checked.
//   holders: [{ kind, range(off, len) -> Promise<Buffer> }]  a holder that serves byte ranges of THIS object
//            [{ kind, piece(hex) -> Promise<Buffer> }]       a holder that serves pieces by their hash (IPFS)
//   report({ kappa, piece, holder, ok, error? })              every outcome, lies included
export async function* get(kappa, { size, pieces, from = 0, to = size - 1, holders, report = () => {}, run = RUN, ahead = AHEAD }) {
  if (!pieces?.length || pieces.length !== Math.ceil(size / PIECE)) throw new Error(`${kappa}: no piece list for ${size} bytes`);
  const first = Math.floor(from / PIECE), last = Math.floor(to / PIECE);
  const per = Math.max(1, Math.floor(run / PIECE));
  const runs = [];
  for (let p = first; p <= last; p += per) runs.push([p, Math.min(last, p + per - 1)]);
  const lenOf = (p) => Math.min(PIECE, size - p * PIECE);
  const ok = (p, b) => b && b.length === lenOf(p) && hex(b) === pieces[p];

  const fetchRun = async ([a, b]) => {
    const got = new Map();
    for (const h of rank(holders)) {
      const want = [];
      for (let p = a; p <= b; p++) if (!got.has(p)) want.push(p);
      if (!want.length) break;
      if (h.piece) {
        await Promise.all(want.map(async (p) => {
          try { const buf = await within(h.piece(pieces[p]), deadline(lenOf(p)), h.kind); if (ok(p, buf)) { got.set(p, buf); note(h.kind, true); report({ kappa, piece: p, holder: h.kind, ok: true }); } else if (buf) { note(h.kind, false); report({ kappa, piece: p, holder: h.kind, ok: false }); } }
          catch (e) { if (e.slow) note(h.kind, false); report({ kappa, piece: p, holder: h.kind, ok: false, error: e.message }); }
        }));
        continue;
      }
      // one range over the still-missing span; each piece in it checked on its own
      const lo = want[0], hi = want[want.length - 1], off = lo * PIECE, len = hi * PIECE + lenOf(hi) - off;
      let buf;
      try { buf = await within(h.range(off, len), deadline(len), h.kind); } catch (e) { if (e.slow) note(h.kind, false); report({ kappa, piece: lo, holder: h.kind, ok: false, error: e.message }); continue; }
      for (const p of want) {
        const part = buf?.subarray(p * PIECE - off, p * PIECE - off + lenOf(p));
        if (ok(p, part)) { got.set(p, Buffer.from(part)); note(h.kind, true); report({ kappa, piece: p, holder: h.kind, ok: true }); }
        else { note(h.kind, false); report({ kappa, piece: p, holder: h.kind, ok: false }); }
      }
    }
    for (let p = a; p <= b; p++) if (!got.has(p)) throw new Error(`${kappa}: no holder sent piece ${p} (sha256 ${pieces[p].slice(0, 12)})`);
    return got;
  };

  const inflight = [];
  let next = 0;
  const pump = () => { while (next < runs.length && inflight.length < ahead) { const r = runs[next]; const pr = fetchRun(runs[next++]); pr.catch(() => {}); inflight.push([r, pr]); } };
  pump();
  while (inflight.length) {
    const [[a, b], pr] = inflight.shift();
    const got = await pr;
    pump();
    for (let p = a; p <= b; p++) {
      const buf = got.get(p), start = p * PIECE;
      yield buf.subarray(Math.max(from, start) - start, Math.min(to, start + buf.length - 1) - start + 1);
    }
  }
}

// Byte-range holders from plain HTTP: `url` serves the object itself (offset 0 = the object's first byte), or a
// larger file in which the object starts at `at` (a tensor inside a Hugging Face file).
export const httpRange = (kind, url, at = 0, headers = {}) => ({
  kind,
  range: async (off, len) => {
    const u = typeof url === "function" ? await url() : url;
    const r = await fetch(u, { headers: { ...headers, range: `bytes=${at + off}-${at + off + len - 1}` }, signal: AbortSignal.timeout(120_000) });
    if (r.status !== 206 && !(r.status === 200 && at + off === 0)) { r.body?.cancel(); throw new Error(`${kind}: HTTP ${r.status}`); }
    const b = Buffer.from(await r.arrayBuffer());
    return r.status === 200 ? b.subarray(0, len) : b;
  },
});
// A piece holder from an IPFS gateway: /ipfs/<raw CID>?format=raw.
export const ipfsPieces = (gateway) => ({
  kind: `ipfs ${new URL(gateway).host}`,
  piece: async (h) => {
    const r = await fetch(`${gateway.replace(/\/$/, "")}/ipfs/${cidOf(h)}?format=raw`, { signal: AbortSignal.timeout(30_000) });
    if (!r.ok) { r.body?.cancel(); return null; }
    return Buffer.from(await r.arrayBuffer());
  },
});
