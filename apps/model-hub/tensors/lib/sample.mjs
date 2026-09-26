// Per-tensor provenance sample, computed from the canonical bytes hashpass.mjs already emits: no second read.
//
//   narrowDtype the narrowest of BF16, F16, F32 that holds every value exactly (checked in that order; F64 and
//               non-float dtypes are kept as stored). An f32 file of bf16 values gets the bf16 tensor's canonical.
//   narrow      sha256 of the payload in narrowDtype: little-endian, contiguous, unframed. Equals the tensor κ whenever
//               the tensor is already stored in its narrowest type (the common case). An equivalence key only; it
//               never replaces κ. (Not `canonical`: that is the canonical MODEL κ in the tensor table.)
//   blocks      for float tensors with 2+ dimensions: [sha256, bytes] of the canonical payload per 1024 rows of the
//               first axis, position not included (a vocabulary resize keeps every earlier block).
//   sign        the sign bits of the values at element indices floor(k·n/4096), k < 4096 (all of them when
//               n <= 4096), packed MSB-first, hex; `sign_n` bits.
//
// Same rules as HOLOGRAM/spikes/kappa-provenance/kappa_index/tensorhash.py (parity-tested); sign agreement between two
// tensors of the same name and shape measures lineage: 49.9 % for independent training, 95.6-100 % for fine-tunes,
// merges and edits (ten measured pairs).
import { createHash } from "node:crypto";

export const SAMPLE = 4096;
export const ROWS = 1024;
const WIDTH = { BF16: 2, F16: 2, F32: 4 };
const CANDIDATES = { BF16: ["BF16"], F16: ["BF16", "F16"], F32: ["BF16", "F16", "F32"] };

// f32 bits -> f16 bits when the value is exactly representable, else -1 (same result as numpy's round trip).
function f32toF16Exact(u) {
  const s = (u >>> 16) & 0x8000, e = (u >>> 23) & 0xff, m = u & 0x7fffff;
  if (e === 0xff) return m === 0 ? s | 0x7c00 : (m & 0x1fff) === 0 && m >>> 13 ? s | 0x7c00 | (m >>> 13) : -1;
  if (e === 0) return m === 0 ? s : -1;
  const E = e - 127;
  if (E >= -14 && E <= 15) return (m & 0x1fff) === 0 ? s | ((E + 15) << 10) | (m >>> 13) : -1;
  if (E >= -24 && E < -14) {
    const full = 0x800000 | m, shift = 13 + (-14 - E);
    return (full & ((1 << shift) - 1)) === 0 ? s | (full >>> shift) : -1;
  }
  return -1;
}

// f16 bits -> f32 bits (exact, always).
function f16toF32(h) {
  const s = (h & 0x8000) << 16, e = (h >>> 10) & 0x1f, m = h & 0x3ff;
  if (e === 0x1f) return (s | 0x7f800000 | (m << 13)) >>> 0;
  if (e === 0) {
    if (m === 0) return s >>> 0;
    let E = -14, mm = m;
    while (!(mm & 0x400)) { mm <<= 1; E--; }
    return (s | ((E + 127) << 23) | ((mm & 0x3ff) << 13)) >>> 0;
  }
  return (s | ((e - 15 + 127) << 23) | (m << 13)) >>> 0;
}

class Candidate {
  constructor(cdtype, shape) {
    this.cdtype = cdtype; this.exact = true; this.h = createHash("sha256");
    this.rowed = shape.length >= 2;
    if (this.rowed) {
      this.blockBytes = WIDTH[cdtype] * shape.slice(1).reduce((a, b) => a * b, 1) * ROWS;
      this.blocks = []; this.cur = createHash("sha256"); this.curN = 0;
    }
  }
  feed(buf) {
    this.h.update(buf);
    if (!this.rowed) return;
    let o = 0;
    while (o < buf.length) {
      const take = Math.min(this.blockBytes - this.curN, buf.length - o);
      this.cur.update(buf.subarray(o, o + take)); this.curN += take; o += take;
      if (this.curN === this.blockBytes) { this.blocks.push([`sha256:${this.cur.digest("hex")}`, this.curN]); this.cur = createHash("sha256"); this.curN = 0; }
    }
  }
  finish() {
    if (this.rowed && this.curN) this.blocks.push([`sha256:${this.cur.digest("hex")}`, this.curN]);
    return { narrowDtype: this.cdtype, narrow: `sha256:${this.h.digest("hex")}`, ...(this.rowed ? { blocks: this.blocks } : {}) };
  }
}

export const SAMPLED = (dtype) => dtype in WIDTH;

export class TensorSample {
  constructor(dtype, shape) {
    this.dtype = dtype; this.shape = shape;
    this.n = shape.reduce((a, b) => a * b, 1);
    this.float = dtype in WIDTH;
    this.pending = Buffer.alloc(0); this.seen = 0; this.at = 0;
    if (!this.float) { this.plain = createHash("sha256"); return; }
    this.width = WIDTH[dtype];
    this.cands = CANDIDATES[dtype].map((c) => new Candidate(c, shape));
    this.count = this.n <= SAMPLE ? this.n : SAMPLE;
    this.bits = new Uint8Array(this.count);
  }
  idx(k) { return this.n <= SAMPLE ? k : Math.floor((k * this.n) / SAMPLE); }

  update(buf) {
    if (!this.float) { this.plain.update(buf); return; }
    if (this.pending.length) { buf = Buffer.concat([this.pending, buf]); this.pending = Buffer.alloc(0); }
    const cut = buf.length - (buf.length % this.width);
    if (cut < buf.length) { this.pending = Buffer.from(buf.subarray(cut)); buf = buf.subarray(0, cut); }
    if (!buf.length) return;
    const count = buf.length / this.width, lo = this.seen, hi = lo + count;
    const bitsOf = (i) => (this.width === 2 ? buf.readUInt16LE((i - lo) * 2) >>> 15 : buf.readUInt32LE((i - lo) * 4) >>> 31);
    while (this.at < this.count && this.idx(this.at) < hi) { this.bits[this.at] = bitsOf(this.idx(this.at)); this.at++; }
    this.seen = hi;
    for (const c of this.cands) {
      if (!c.exact) continue;
      const out = this.convert(buf, count, c.cdtype);
      if (!out) { c.exact = false; continue; }
      c.feed(out);
    }
  }

  convert(buf, count, cdtype) {
    const src = this.dtype;
    if (src === cdtype) return buf;
    const out = Buffer.allocUnsafe(count * WIDTH[cdtype]);
    for (let i = 0; i < count; i++) {
      const u = src === "F32" ? buf.readUInt32LE(i * 4) : f16toF32(buf.readUInt16LE(i * 2));
      if (cdtype === "BF16") {
        if (u & 0xffff) return null;
        out.writeUInt16LE(u >>> 16, i * 2);
      } else if (cdtype === "F16") {
        const h = f32toF16Exact(u);
        if (h < 0) return null;
        out.writeUInt16LE(h, i * 2);
      } else return null;
    }
    return out;
  }

  result() {
    if (!this.float) return { narrowDtype: this.dtype, narrow: `sha256:${this.plain.digest("hex")}` };
    if (this.pending.length || this.seen !== this.n) throw new Error("tensor bytes do not match its shape");
    const c = this.cands.find((x) => x.exact);
    const bytes = Buffer.alloc(Math.ceil(this.count / 8));
    for (let k = 0; k < this.count; k++) if (this.bits[k]) bytes[k >> 3] |= 0x80 >> (k & 7);
    return { ...c.finish(), sign: bytes.toString("hex"), sign_n: this.count };
  }
}
