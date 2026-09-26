// Index one model revision end to end: plan every weight file, hash it once, refuse any file whose sha256
// differs from Hugging Face's LFS oid, and seal the objects that make the model a tiny κ manifest:
//   layout      one per weight file: the segments (literal / tensor / storage) that rebuild it byte for byte
//   tensors     the manifest's config: every tensor's name, dtype, shape, κ, and the canonical model κ
//   manifest    one per format (original, safetensors, safetensors-sharded), layers = files by digest
//   index       an OCI image index over the formats: the model's address
//   provenance  per distinct κ: narrowest-dtype digest, sign sample, row-block digests (sample.mjs); not in the table
import { createHash } from "node:crypto";
import { info, tree, cdnUrl, whole } from "./src.mjs";
import { plan as planFile, isWeightPath } from "./containers.mjs";
import { hashFile, emissionOrder } from "./hashpass.mjs";
import { planFormat, RenderHasher, RENDERABLE } from "./render.mjs";
import { TensorSample, SAMPLED } from "./sample.mjs";

export const T = {
  manifest: "application/vnd.oci.image.manifest.v1+json",
  index: "application/vnd.oci.image.index.v1+json",
  model: "application/vnd.hologram.model.v1",
  tensors: "application/vnd.hologram.tensors.v1+json",
  layout: "application/vnd.hologram.layout.v1+json",
  provenance: "application/vnd.hologram.provenance.v1+json",
  file: "application/octet-stream",
};
export const FORMATS = ["safetensors", "safetensors-sharded"];
const SMALL = 16 << 20;
// Weights of other frameworks travel with the original format only; renders carry configs and tokenizers.
const FOREIGN = /\.(safetensors|gguf|bin|pt|pth|ckpt|h5|msgpack|onnx|onnx_data|ot|tflite|mlmodel|pb|npz|keras)$|^(onnx|openvino|coreml|tf|flax)\//i;

export function canonicalKappa(tensors) {
  const lines = [...new Set(tensors.map((t) => `${t.dtype}|${JSON.stringify(t.shape)}|${t.kappa}`))].sort();
  return `sha256:${createHash("sha256").update(lines.join("\n")).digest("hex")}`;
}

export async function indexModel(repo, { store, rev: pin, formats = FORMATS, log = () => {}, headersOnly = false } = {}) {
  const meta = await info(repo);
  if (meta.gated || meta.private) throw Object.assign(new Error("gated or private"), { skip: true });
  const rev = pin || meta.sha;
  const files = await tree(repo, rev);
  const t0 = Date.now();
  const getUrl = (path) => (fresh) => cdnUrl(repo, rev, path, fresh);

  // 1. plan
  const planned = [], small = [], other = [];
  for (const f of files) {
    if (f.oid && isWeightPath(f.path)) {
      const p = await planFile(await getUrl(f.path)(false), f.size, f.path).catch((e) => { log(`  ${f.path}: not indexed (${e.message})`); return null; });
      if (p) { planned.push({ ...f, plan: p }); continue; }
    }
    if (!f.oid || f.size <= SMALL) small.push(f); else other.push(f);
  }
  planned.sort((a, b) => (a.path < b.path ? -1 : 1));
  const nT = planned.reduce((a, f) => a + f.plan.tensors.length, 0);
  log(`  planned ${planned.length} weight files, ${nT} tensors; ${small.length} small files; ${other.length} other LFS files`);

  // 2. which files a render is made from: safetensors when present, else torch; never GGUF
  const kinds = new Set(planned.map((f) => f.plan.container));
  const src = kinds.has("safetensors") ? planned.filter((f) => f.plan.container === "safetensors")
    : planned.filter((f) => f.plan.container.startsWith("torch"));
  const list = [];
  for (const f of src) for (const i of emissionOrder(f.plan)) { const t = f.plan.tensors[i]; list.push({ ...t, key: `${f.path}#${i}` }); }
  const renderable = list.length && list.every((t) => RENDERABLE.has(t.dtype)) && new Set(list.map((t) => t.name)).size === list.length;
  const fmts = renderable ? Object.fromEntries(formats.map((k) => [k, planFormat(k, list)])) : {};
  if (headersOnly) return { repo, rev, planned, provisional: true };

  // 3. one pass per file: render sources first (in order), the rest after
  const rh = renderable ? new RenderHasher(fmts) : null;
  const order = [...src, ...planned.filter((f) => !src.includes(f))];
  let bytes = 0;
  // Every float tensor's canonical bytes also feed a provenance sample (sample.mjs) in the same read.
  const samples = new Map();
  for (const f of order) {
    const feeds = rh && src.includes(f);
    const sampled = f.plan.tensors.some((t) => SAMPLED(t.dtype));
    const emit = (i, c) => {
      if (feeds) rh.feed(`${f.path}#${i}`, c);
      const t = f.plan.tensors[i];
      if (!SAMPLED(t.dtype)) return;
      const key = `${f.path}#${i}`;
      if (!samples.has(key)) samples.set(key, new TensorSample(t.dtype, t.shape));
      samples.get(key).update(c);
    };
    const r = await hashFile({ getUrl: getUrl(f.path), size: f.size, plan: f.plan, emit: feeds || sampled ? emit : null });
    if (r.digest !== `sha256:${f.oid}`) throw new Error(`${f.path}: sha256 ${r.digest} differs from Hugging Face's ${f.oid}; refused`);
    for (const [d, b] of r.literals) store.put(b);
    f.result = r; bytes += f.size;
    log(`  hashed ${f.path} ${(f.size / 1e6).toFixed(1)} MB (${((Date.now() - t0) / 1000).toFixed(0)} s)`);
  }
  const rendered = rh ? rh.finish() : {};

  // 4. small files: held, addressed by their own sha256
  for (const f of small) {
    const b = await whole(repo, rev, f.path);
    if (b.length !== f.size) throw new Error(`${f.path}: ${b.length} bytes, tree says ${f.size}`);
    const put = store.put(b);
    if (f.oid && put.digest !== `sha256:${f.oid}`) throw new Error(`${f.path}: sha256 differs from its LFS oid`);
    f.digest = put.digest;
  }

  // 5. seal: layouts, tensor table, manifests, index
  const fileIdx = new Map(planned.map((f, k) => [f.path, k]));
  const tensors = [], rows = [], kappaOf = new Map();
  for (const f of planned) {
    f.plan.tensors.forEach((t, i) => {
      const kappa = f.result.kappa[i];
      kappaOf.set(`${f.path}#${i}`, kappa);
      tensors.push({ ...t, kappa, file: f.path });
      const row = [fileIdx.get(f.path), t.name, t.dtype, t.shape, kappa, t.len];
      if (t.sub) row.push(t.sub);
      rows.push(row);
    });
    const layout = { v: 1, file: f.path, digest: `sha256:${f.oid}`, size: f.size, container: f.plan.container,
      segments: f.result.segments.map((s) => [s.kind, s.digest, s.len, s.off]) };
    f.layout = store.putJson(layout);
  }
  // Provenance: one row per distinct κ (the same κ has the same sample): [κ, narrowDtype, narrow, signB64, blocks].
  const provRows = new Map();
  for (const [key, s] of samples) {
    const kappa = kappaOf.get(key);
    if (provRows.has(kappa)) continue;
    const r = s.result();
    provRows.set(kappa, [kappa, r.narrowDtype, r.narrow, Buffer.from(r.sign, "hex").toString("base64"), r.blocks || []]);
  }
  const provenance = provRows.size ? store.putJson({ v: 1, repo, revision: rev,
    method: "hologram.provenance/v1: narrowest exact dtype of BF16, F16, F32; sign bits at floor(k*n/4096), k<4096, MSB-first; sha256 per 1024 rows of the narrow payload",
    rows: [...provRows.values()] }) : null;
  const canonSet = src.length ? tensors.filter((t) => src.some((f) => f.path === t.file)) : tensors;
  const canonical = canonSet.length ? canonicalKappa(canonSet) : null;
  const table = store.putJson({ v: 1, repo, revision: rev, canonical, files: planned.map((f) => f.path), tensors: rows });

  const annot = (format) => ({ "org.hologram.format": format, "org.hologram.repo": repo, "org.hologram.revision": rev,
    ...(canonical ? { "org.hologram.canonical": canonical } : {}), "org.opencontainers.image.source": `https://huggingface.co/${repo}/tree/${rev}` });
  const layer = (path, digest, size, extra = {}) => ({ mediaType: T.file, digest, size, annotations: { "org.opencontainers.image.title": path, ...extra } });
  const blobs = {};
  const smallLayers = (keep) => small.filter((f) => keep(f.path)).map((f) => { blobs[f.digest] = { path: f.path, size: f.size, held: true }; return layer(f.path, f.digest, f.size); });

  // original: every file of the repo, weights by their HF digest with a layout for assembly
  const origLayers = [
    ...planned.map((f) => { blobs[`sha256:${f.oid}`] = { path: f.path, size: f.size, layout: f.layout.digest, upstream: true }; return layer(f.path, `sha256:${f.oid}`, f.size, { "org.hologram.layout": f.layout.digest }); }),
    ...other.map((f) => { blobs[`sha256:${f.oid}`] = { path: f.path, size: f.size, upstream: true }; return layer(f.path, `sha256:${f.oid}`, f.size); }),
    ...smallLayers(() => true),
  ].sort((a, b) => (a.annotations["org.opencontainers.image.title"] < b.annotations["org.opencontainers.image.title"] ? -1 : 1));
  const manifests = {};
  const seal = (format, layers) => {
    const m = { schemaVersion: 2, mediaType: T.manifest, artifactType: T.model, config: { mediaType: T.tensors, digest: table.digest, size: table.size }, layers, annotations: annot(format) };
    manifests[format] = { ...store.putJson(m), format };
  };
  seal("original", origLayers);

  // renders: each output is a layout of [header literal, tensor segments with their source location]
  const srcSeg = new Map(); // key -> source reference in an original file
  for (const f of src) f.plan.tensors.forEach((t, i) => {
    const seg = f.result.segments[t.seg];
    srcSeg.set(`${f.path}#${i}`, t.seg < 0 ? null : t.sub ? { f: f.path, o: seg.off, n: seg.len, s: seg.digest, sub: t.sub, shape: t.shape } : { f: f.path, o: seg.off });
  });
  for (const [format, p] of Object.entries(fmts)) {
    const out = rendered[format], layers = [];
    p.outputs.forEach((o, k) => {
      const h = store.put(o.header);
      const segments = [["l", h.digest, o.header.length]];
      for (const t of o.tensors) {
        if (!t.len) continue;
        const s = srcSeg.get(t.key), kappa = kappaOf.get(t.key);
        segments.push([s.sub ? "v" : "t", kappa, t.len, s]);
      }
      const lay = store.putJson({ v: 1, file: o.path, digest: out[k].digest, size: out[k].size, container: "safetensors", render: format, segments });
      if (!blobs[out[k].digest]) blobs[out[k].digest] = { path: o.path, size: out[k].size, layout: lay.digest }; // a render equal to an original file keeps the original's entry
      else blobs[out[k].digest].sameAs = [...(blobs[out[k].digest].sameAs || []), `${format}:${o.path}`];
      layers.push(layer(o.path, out[k].digest, out[k].size, { "org.hologram.layout": lay.digest }));
    });
    for (const x of p.extras) { const put = store.put(x.bytes); blobs[put.digest] ||= { path: x.path, size: put.size, held: true }; layers.push(layer(x.path, put.digest, put.size)); }
    layers.push(...smallLayers((path) => !FOREIGN.test(path)));
    seal(format, layers);
  }
  const index = store.putJson({ schemaVersion: 2, mediaType: T.index, artifactType: T.model,
    manifests: Object.values(manifests).map((m) => ({ mediaType: T.manifest, digest: m.digest, size: m.size, artifactType: T.model, annotations: { "org.hologram.format": m.format } })),
    annotations: annot("index") });

  // sources: where each tensor κ can be read (this model's own files); the hub merges these across models
  const sources = [];
  for (const f of planned) f.result.segments.forEach((s) => { if (s.kind !== "l") sources.push([s.digest, repo, rev, f.path, s.off, s.len]); });

  return {
    repo, rev, index: index.digest, table: table.digest, canonical, provenance: provenance?.digest || null, manifests: Object.fromEntries(Object.entries(manifests).map(([k, v]) => [k, v.digest])),
    blobs, sources, tensors: nT, weightBytes: bytes, seconds: Math.round((Date.now() - t0) / 1000),
    files: planned.map((f) => ({ path: f.path, digest: `sha256:${f.oid}`, size: f.size, container: f.plan.container, layout: f.layout.digest })),
    renders: rendered, license: meta.cardData?.license || meta.tags?.find((t) => t.startsWith("license:"))?.slice(8) || null,
  };
}
