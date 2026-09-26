// One shape for every weight container: a file is an ordered run of segments covering every byte.
//   l  literal: headers, padding, zip records, pickle bytes. Small; held by the hub as its own κ object.
//   t  tensor payload: exactly the canonical bytes of one tensor (every name that views it sees all of it).
//   s  storage: raw bytes of a torch storage that tensors view partially or with strides; tensors carry `sub`.
// plan = { container, segments: [{ kind, off, len }], tensors: [{ name, dtype, shape, len, seg, sub? }] }
// A tensor's canonical bytes are contiguous, little-endian, unframed; `sub` = { o, stride, itemsize } locates
// them inside an `s` segment (stride present only when a gather is needed).
import { range } from "./src.mjs";
import { unpickle, flatten, contiguous } from "./pickle.mjs";

const GGML = { 0: ["F32", 1, 4], 1: ["F16", 1, 2], 2: ["Q4_0", 32, 18], 3: ["Q4_1", 32, 20], 6: ["Q5_0", 32, 22], 7: ["Q5_1", 32, 24],
  8: ["Q8_0", 32, 34], 9: ["Q8_1", 32, 36], 10: ["Q2_K", 256, 84], 11: ["Q3_K", 256, 110], 12: ["Q4_K", 256, 144], 13: ["Q5_K", 256, 176],
  14: ["Q6_K", 256, 210], 15: ["Q8_K", 256, 292], 16: ["IQ2_XXS", 256, 66], 17: ["IQ2_XS", 256, 74], 18: ["IQ3_XXS", 256, 98],
  19: ["IQ1_S", 256, 50], 20: ["IQ4_NL", 32, 18], 21: ["IQ3_S", 256, 110], 22: ["IQ2_S", 256, 82], 23: ["IQ4_XS", 256, 136],
  24: ["I8", 1, 1], 25: ["I16", 1, 2], 26: ["I32", 1, 4], 27: ["I64", 1, 8], 28: ["F64", 1, 8], 29: ["IQ1_M", 256, 56],
  30: ["BF16", 1, 2], 34: ["TQ1_0", 256, 54], 35: ["TQ2_0", 256, 66], 39: ["MXFP4", 32, 17] };
const ST_ITEM = { F64: 8, F32: 4, F16: 2, BF16: 2, I64: 8, I32: 4, I16: 2, I8: 1, U8: 1, BOOL: 1, F8_E4M3: 1, F8_E5M2: 1, U16: 2, U32: 4, U64: 8, F8_E8M0: 1, F4: 0.5 };

// Fill the gaps between payload segments with literals so segments cover [0, size).
function cover(payload, size) {
  payload.sort((a, b) => a.off - b.off);
  const segs = [];
  let pos = 0;
  for (const p of payload) {
    if (p.off < pos) throw new Error(`overlapping payload at ${p.off}`);
    if (p.off > pos) segs.push({ kind: "l", off: pos, len: p.off - pos });
    segs.push(p); pos = p.off + p.len;
  }
  if (pos < size) segs.push({ kind: "l", off: pos, len: size - pos });
  if (pos > size) throw new Error("payload beyond end of file");
  return segs;
}

function finish(container, payload, tensors, size) {
  const segs = cover(payload.filter((p) => p.len > 0), size);
  const index = new Map(segs.map((s, i) => [s, i]));
  for (const t of tensors) t.seg = t._seg && t._seg.len > 0 ? index.get(t._seg) : -1, delete t._seg;
  return { container, segments: segs, tensors };
}

// ---------------------------------------------------------------- safetensors
async function safetensors(url, size) {
  let head = await range(url, 0, Math.min(size, 1 << 20) - 1);
  const n = Number(head.readBigUInt64LE(0));
  if (8 + n > head.length) head = Buffer.concat([head, await range(url, head.length, 8 + n - 1)]);
  const hdr = JSON.parse(head.toString("utf8", 8, 8 + n));
  const base = 8 + n, payload = [], tensors = [];
  for (const [name, v] of Object.entries(hdr)) {
    if (name === "__metadata__") continue;
    const [a, b] = v.data_offsets;
    const seg = { kind: "t", off: base + a, len: b - a };
    payload.push(seg);
    tensors.push({ name, dtype: v.dtype, shape: v.shape, len: b - a, _seg: seg });
  }
  return finish("safetensors", payload, tensors, size);
}

// ---------------------------------------------------------------- GGUF
async function gguf(url, size) {
  for (let want = 1 << 20; ; want *= 4) {
    const buf = await range(url, 0, Math.min(size, want) - 1);
    try { return ggufParse(buf, size); }
    catch (e) { if (!(e instanceof RangeError) || want >= size || want > 1 << 28) throw e; }
  }
}
function ggufParse(buf, size) {
  let i = 0;
  const need = (n) => { if (i + n > buf.length) throw new RangeError("gguf header truncated"); };
  const u32 = () => { need(4); const v = buf.readUInt32LE(i); i += 4; return v; };
  const u64 = () => { need(8); const v = Number(buf.readBigUInt64LE(i)); i += 8; return v; };
  const str = () => { const n = u64(); need(n); const s = buf.toString("utf8", i, i + n); i += n; return s; };
  const SIZES = { 0: 1, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 4, 7: 1, 10: 8, 11: 8, 12: 8 };
  const value = (t) => {
    if (t === 8) return str();
    if (t === 9) { const et = u32(), n = u64(); if (et === 8) { for (let k = 0; k < n; k++) str(); return null; } const w = SIZES[et]; need(w * n); i += w * n; return null; }
    const w = SIZES[t]; if (!w) throw new Error(`gguf value type ${t}`); need(w);
    const v = w === 4 ? buf.readUInt32LE(i) : w === 8 ? Number(buf.readBigUInt64LE(i)) : buf[i]; i += w; return v;
  };
  if (buf.toString("latin1", 0, 4) !== "GGUF") throw new Error("not gguf");
  i = 4; const version = u32(); if (version < 2) throw new Error(`gguf v${version}`);
  const nt = u64(), nkv = u64();
  let align = 32;
  for (let k = 0; k < nkv; k++) { const key = str(), t = u32(), v = value(t); if (key === "general.alignment") align = v; }
  const infos = [];
  for (let k = 0; k < nt; k++) {
    const name = str(), nd = u32(), dims = []; for (let d = 0; d < nd; d++) dims.push(u64());
    const type = u32(), off = u64(); infos.push({ name, dims, type, off });
  }
  const dataStart = Math.ceil(i / align) * align;
  const payload = [], tensors = [];
  for (const t of infos) {
    const g = GGML[t.type];
    if (!g) throw new Error(`gguf tensor ${t.name}: unknown ggml type ${t.type}`);
    const [dtype, blk, tsz] = g, n = t.dims.reduce((a, b) => a * b, 1);
    const len = (n / blk) * tsz;
    if (!Number.isInteger(len)) throw new Error(`gguf tensor ${t.name}: ${n} elements not a multiple of ${blk}`);
    const seg = { kind: "t", off: dataStart + t.off, len };
    payload.push(seg); tensors.push({ name: t.name, dtype, shape: t.dims, len, _seg: seg });
  }
  return finish("gguf", payload, tensors, size);
}

// ---------------------------------------------------------------- torch (zip and legacy)
async function zipEntries(url, size) {
  const tailLen = Math.min(size, 65536 + 22), base = size - tailLen;
  const tail = await range(url, base, size - 1);
  const e = tail.lastIndexOf(Buffer.from("PK\x05\x06", "latin1"));
  if (e < 0) throw new Error("no zip end record");
  let n = tail.readUInt16LE(e + 10), cdSize = tail.readUInt32LE(e + 12), cdOff = tail.readUInt32LE(e + 16);
  if (n === 0xffff || cdSize === 0xffffffff || cdOff === 0xffffffff) {
    const l = tail.lastIndexOf(Buffer.from("PK\x06\x07", "latin1"), e);
    const e64 = Number(tail.readBigUInt64LE(l + 8));
    const rec = e64 >= base ? tail.subarray(e64 - base, e64 - base + 56) : await range(url, e64, e64 + 55);
    n = Number(rec.readBigUInt64LE(32)); cdSize = Number(rec.readBigUInt64LE(40)); cdOff = Number(rec.readBigUInt64LE(48));
  }
  const cd = cdOff >= base ? tail.subarray(cdOff - base, cdOff - base + cdSize) : await range(url, cdOff, cdOff + cdSize - 1);
  const out = [];
  for (let p = 0, k = 0; k < n; k++) {
    if (cd.readUInt32LE(p) !== 0x02014b50) throw new Error("bad central directory");
    const method = cd.readUInt16LE(p + 10); let csize = cd.readUInt32LE(p + 20), usize = cd.readUInt32LE(p + 24);
    const nl = cd.readUInt16LE(p + 28), xl = cd.readUInt16LE(p + 30), cl = cd.readUInt16LE(p + 32);
    let lho = cd.readUInt32LE(p + 42);
    const name = cd.toString("utf8", p + 46, p + 46 + nl);
    for (let x = p + 46 + nl; x < p + 46 + nl + xl;) {           // zip64 extra field
      const id = cd.readUInt16LE(x), sz = cd.readUInt16LE(x + 2); let y = x + 4;
      if (id === 1) { if (usize === 0xffffffff) { usize = Number(cd.readBigUInt64LE(y)); y += 8; } if (csize === 0xffffffff) { csize = Number(cd.readBigUInt64LE(y)); y += 8; } if (lho === 0xffffffff) { lho = Number(cd.readBigUInt64LE(y)); } }
      x += 4 + sz;
    }
    out.push({ name, method, csize, usize, lho });
    p += 46 + nl + xl + cl;
  }
  return out;
}
async function dataStart(url, entry) {
  const h = await range(url, entry.lho, entry.lho + 29);
  if (h.readUInt32LE(0) !== 0x04034b50) throw new Error(`bad local header for ${entry.name}`);
  return entry.lho + 30 + h.readUInt16LE(26) + h.readUInt16LE(28);
}
async function pool(items, n, fn) { const out = new Array(items.length); let k = 0; await Promise.all(Array.from({ length: n }, async () => { while (k < items.length) { const j = k++; out[j] = await fn(items[j]); } })); return out; }

// Storage segments + tensor records from a name->TensorRec map and key -> [start, nbytes].
function torchPlan(container, recs, storages, size) {
  const byKey = new Map();
  for (const [name, t] of recs) { if (!byKey.has(t.storage.key)) byKey.set(t.storage.key, []); byKey.get(t.storage.key).push([name, t]); }
  const payload = [], tensors = [];
  for (const [key, list] of byKey) {
    const st = storages.get(key);
    if (!st) throw new Error(`storage ${key} not found`);
    const [start, nbytes] = st, item = list[0][1].storage.itemsize;
    const full = list.every(([, t]) => t.offset === 0 && contiguous(t.size, t.stride) && t.size.reduce((a, b) => a * b, 1) * item === nbytes);
    const seg = { kind: full ? "t" : "s", off: start, len: nbytes };
    payload.push(seg);
    for (const [name, t] of list) {
      const numel = t.size.reduce((a, b) => a * b, 1), len = numel * item;
      const rec = { name, dtype: t.storage.dtype, shape: t.size, len, _seg: seg };
      if (!full) rec.sub = contiguous(t.size, t.stride) ? { o: t.offset * item, itemsize: item } : { o: t.offset * item, stride: t.stride, itemsize: item };
      tensors.push(rec);
    }
  }
  return finish(container, payload, tensors, size);
}

async function torchZip(url, size) {
  const entries = await zipEntries(url, size);
  const pkl = entries.find((e) => /(^|\/)data\.pkl$/.test(e.name));
  if (!pkl) throw new Error("zip without data.pkl (not a torch checkpoint)");
  if (pkl.method !== 0) throw new Error("compressed data.pkl");
  const prefix = pkl.name.slice(0, -"data.pkl".length);
  const ps = await dataStart(url, pkl);
  const { value } = unpickle(await range(url, ps, ps + pkl.csize - 1));
  const recs = flatten(value);
  const keys = new Set([...recs.values()].map((t) => t.storage.key));
  const want = entries.filter((e) => e.name.startsWith(`${prefix}data/`) && keys.has(e.name.slice(prefix.length + 5)));
  if (want.some((e) => e.method !== 0)) throw new Error("compressed storage entries");
  const starts = await pool(want, 8, (e) => dataStart(url, e));
  const storages = new Map(want.map((e, k) => [e.name.slice(prefix.length + 5), [starts[k], e.usize]]));
  return torchPlan("torch-zip", recs, storages, size);
}

async function torchLegacy(url, size) {
  for (let want = 4 << 20; ; want *= 4) {
    const buf = await range(url, 0, Math.min(size, want) - 1);
    let recs, keys, p;
    try {
      p = 0; const next = () => { const r = unpickle(buf, p); p = r.end; return r.value; };
      if (next() !== 0x1950a86a20f9469cfc6cn) throw new Error("not a torch legacy file");
      next(); const sys = next();
      if (sys instanceof Map && sys.get("little_endian") === false) throw new Error("big-endian legacy file");
      recs = flatten(next()); keys = next();
    } catch (e) { if (!(e instanceof RangeError) || want >= size) throw e; continue; }
    const item = new Map([...recs.values()].map((t) => [t.storage.key, t.storage.itemsize]));
    const storages = new Map();
    for (const k of keys) {                         // each storage: 8-byte element count, then its bytes
      const key = String(k);
      if (!item.has(key)) throw new Error(`legacy storage ${key} has no tensor`);
      const pre = p + 8 <= buf.length ? buf.subarray(p, p + 8) : await range(url, p, p + 7);
      const nbytes = Number(pre.readBigInt64LE(0)) * item.get(key);
      storages.set(key, [p + 8, nbytes]); p += 8 + nbytes;
    }
    if (p !== size) throw new Error(`legacy storages end at ${p}, file is ${size}`);
    return torchPlan("torch-legacy", recs, storages, size);
  }
}

// Which container is this file? `path` hints, the first bytes decide.
export async function plan(url, size, path) {
  if (/\.safetensors$/i.test(path)) return safetensors(url, size);
  const magic = await range(url, 0, Math.min(size, 16) - 1);
  if (magic.toString("latin1", 0, 4) === "GGUF") return gguf(url, size);
  if (magic.readUInt32LE(0) === 0x04034b50) return torchZip(url, size);
  if (magic[0] === 0x80 && magic[1] === 0x02 && magic[2] === 0x8a) return torchLegacy(url, size);
  return null;
}

export const isWeightPath = (p) => /\.(safetensors|gguf|bin|pt|pth|ckpt)$/i.test(p) && !/^openvino\//i.test(p) && !/(^|\/)(training_args|optimizer|scheduler|rng_state)[^/]*$/i.test(p);
export { ST_ITEM };
