// Formats are just more layouts over the same tensors. A render is planned from tensor metadata alone
// (header bytes + tensor order), and its digest is hashed during the one pass over the source files,
// because the render's tensor order is the pass's emission order.
import { createHash } from "node:crypto";

export const RENDERABLE = new Set(["F64", "F32", "F16", "BF16", "I64", "I32", "I16", "I8", "U8", "BOOL", "F8_E4M3", "F8_E5M2", "U16", "U32", "U64"]);

export function safetensorsHeader(list) {
  const hdr = { __metadata__: { format: "pt" } };
  let off = 0;
  for (const t of list) { hdr[t.name] = { dtype: t.dtype, shape: t.shape, data_offsets: [off, off + t.len] }; off += t.len; }
  let json = Buffer.from(JSON.stringify(hdr));
  const pad = (8 - (json.length % 8)) % 8;
  if (pad) json = Buffer.concat([json, Buffer.alloc(pad, 0x20)]);
  const n = Buffer.alloc(8); n.writeBigUInt64LE(BigInt(json.length));
  return { header: Buffer.concat([n, json]), payload: off };
}

// list: tensors in emission order, each { name, dtype, shape, len, key } where key identifies the tensor.
// Returns the outputs of a format: [{ path, header, tensors: [key...], size }], plus extra small files.
export function planFormat(format, list, { shardBytes = 1 << 30 } = {}) {
  if (format === "safetensors") {
    const { header, payload } = safetensorsHeader(list);
    return { outputs: [{ path: "model.safetensors", header, tensors: list, size: header.length + payload }], extras: [] };
  }
  if (format === "safetensors-sharded") {
    const shards = [[]]; let acc = 0;
    for (const t of list) { if (acc + t.len > shardBytes && shards[shards.length - 1].length) { shards.push([]); acc = 0; } shards[shards.length - 1].push(t); acc += t.len; }
    const N = String(shards.length).padStart(5, "0");
    const outputs = shards.map((s, k) => {
      const { header, payload } = safetensorsHeader(s);
      return { path: `model-${String(k + 1).padStart(5, "0")}-of-${N}.safetensors`, header, tensors: s, size: header.length + payload };
    });
    const map = {}; for (const o of outputs) for (const t of o.tensors) map[t.name] = o.path;
    const weight_map = Object.fromEntries(Object.keys(map).sort().map((k) => [k, map[k]]));
    const index = Buffer.from(JSON.stringify({ metadata: { total_size: outputs.reduce((a, o) => a + o.size - o.header.length, 0) }, weight_map }, null, 2) + "\n");
    return { outputs, extras: [{ path: "model.safetensors.index.json", bytes: index }] };
  }
  throw new Error(`unknown format ${format}`);
}

// Feeds emitted tensor bytes, in order, to one hasher per output of every planned format.
export class RenderHasher {
  constructor(formats) {
    this.lanes = [];
    for (const [format, p] of Object.entries(formats)) for (const o of p.outputs) {
      const h = createHash("sha256").update(o.header);
      this.lanes.push({ format, o, h, i: 0, got: 0, n: o.header.length });
    }
    this.pos = new Map(); // key -> lanes expecting it next
  }
  feed(key, chunk) {
    for (const L of this.lanes) {
      const t = L.o.tensors[L.i];
      if (!t || t.key !== key) continue;
      L.h.update(chunk); L.got += chunk.length; L.n += chunk.length;
      if (L.got === t.len) { L.i++; L.got = 0; }
      else if (L.got > t.len) throw new Error(`render ${L.o.path}: ${t.name} overran`);
    }
  }
  finish() {
    const out = {};
    for (const L of this.lanes) {
      if (L.i !== L.o.tensors.length) throw new Error(`render ${L.o.path}: fed ${L.i} of ${L.o.tensors.length} tensors (order mismatch)`);
      if (L.n !== L.o.size) throw new Error(`render ${L.o.path}: ${L.n} bytes, planned ${L.o.size}`);
      (out[L.format] ||= []).push({ path: L.o.path, digest: `sha256:${L.h.digest("hex")}`, size: L.o.size });
    }
    return out;
  }
}
