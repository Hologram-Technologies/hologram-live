// One sequential pass over a file: every byte is read once and yields
//   the file's sha256 (must equal the HF LFS oid, or the file is refused),
//   a digest per segment (literals are kept, payloads are not),
//   a κ per tensor (payload segments directly; views and strided tensors by slicing or gathering their storage),
//   and, when `emit` is given, each tensor's canonical bytes in emissionOrder(plan), so format renders are
//   hashed in the same pass. Only storages and multiply-named payloads are buffered.
import { createHash } from "node:crypto";
import { stream } from "./src.mjs";

export const EMPTY = `sha256:${createHash("sha256").digest("hex")}`;
const CAP = 2 ** 31;

// The order in which a tensor's canonical bytes become available in a sequential read: segment order;
// inside one segment, by sub-offset then name. Zero-length tensors come first. Deterministic from the plan.
export function emissionOrder(plan) {
  const byseg = plan.segments.map(() => []);
  const empty = [];
  plan.tensors.forEach((t, i) => (t.seg >= 0 ? byseg[t.seg].push(i) : empty.push(i)));
  const cmp = (a, b) => ((plan.tensors[a].sub?.o || 0) - (plan.tensors[b].sub?.o || 0)) || (plan.tensors[a].name < plan.tensors[b].name ? -1 : 1);
  return [...empty.sort(cmp), ...byseg.flatMap((l) => l.sort(cmp))];
}

// Canonical bytes of a tensor that views a storage buffer: a slice when contiguous, a gather when strided.
export function canonical(buf, t) {
  const { o, stride, itemsize } = t.sub;
  if (!stride) return buf.subarray(o, o + t.len);
  const out = Buffer.allocUnsafe(t.len), shape = t.shape, nd = shape.length, idx = new Array(nd).fill(0);
  for (let k = 0, n = t.len / itemsize; k < n; k++) {
    let off = o;
    for (let d = 0; d < nd; d++) off += idx[d] * stride[d] * itemsize;
    buf.copy(out, k * itemsize, off, off + itemsize);
    for (let d = nd - 1; d >= 0; d--) { if (++idx[d] < shape[d]) break; idx[d] = 0; }
  }
  return out;
}

const kappaOf = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;

export async function hashFile({ getUrl, size, plan, emit }) {
  const segs = plan.segments, tensors = plan.tensors;
  const byseg = segs.map(() => []);
  tensors.forEach((t, i) => { if (t.seg >= 0) byseg[t.seg].push(i); });
  const order = emissionOrder(plan);
  const out = { segments: [], literals: new Map(), kappa: tensors.map((t) => (t.seg < 0 ? EMPTY : null)) };
  if (emit) for (const i of order) if (tensors[i].seg < 0) emit(i, Buffer.alloc(0));
  const whole = createHash("sha256");
  // Buffer literals, storages, and payloads read by more than one name; stream the rest.
  const buffered = (k) => segs[k].kind !== "t" || (emit && byseg[k].length > 1);
  let k = 0, pos = 0, h, parts;
  const begin = () => {
    h = createHash("sha256"); parts = [];
    if (buffered(k) && segs[k].len > CAP) throw new Error(`segment of ${segs[k].len} bytes is too large to buffer`);
  };
  const end = () => {
    const s = segs[k], digest = `sha256:${h.digest("hex")}`;
    out.segments.push({ kind: s.kind, digest, len: s.len, off: s.off });
    const ids = [...byseg[k]].sort((a, b) => order.indexOf(a) - order.indexOf(b));
    if (s.kind === "l") out.literals.set(digest, Buffer.concat(parts));
    else if (s.kind === "t") {
      for (const i of ids) out.kappa[i] = digest;
      if (emit && buffered(k)) { const b = Buffer.concat(parts); for (const i of ids) emit(i, b); }
    } else {
      const b = Buffer.concat(parts);
      for (const i of ids) { const c = canonical(b, tensors[i]); out.kappa[i] = kappaOf(c); if (emit) emit(i, c); }
    }
  };
  if (segs.length) begin();
  for await (const chunk of stream(getUrl, 0, size - 1)) {
    whole.update(chunk);
    let c = chunk;
    while (c.length) {
      const s = segs[k], take = Math.min(c.length, s.off + s.len - pos), piece = c.subarray(0, take);
      h.update(piece);
      if (buffered(k)) parts.push(piece);
      else if (emit && byseg[k].length === 1) emit(byseg[k][0], piece);
      pos += take; c = c.subarray(take);
      if (pos === s.off + s.len) { end(); if (++k < segs.length) begin(); }
    }
  }
  if (k !== segs.length || pos !== size) throw new Error(`short read: ${pos} of ${size}`);
  return { digest: `sha256:${whole.digest("hex")}`, ...out };
}
