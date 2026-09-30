// The Registry's record of every κ object a model reaches: what it is, how big, its piece list, what points to it,
// and who holds it. One walk from the model's index κ, so a record exists for exactly the objects the model's one
// address checks, and nothing else. Records are the only source of digests for the dialects (tensor-mirror.mjs,
// the OCI, HF and Ollama answers): a digest a dialect emits that has no record is a bug the gate catches
// (tensors/check-dialects.mjs).
//
//   record = { kappa, type, size?, media?, pieces?: { in: <pieces object κ>, n }, refs: [κ…], holders: [holder…] }
//   holder = { kind: "hub", path }                  the hub's own store (small objects it holds)
//          | { kind: "hf", url, at }               bytes [at, at+size) of a file on Hugging Face at a pinned commit
//          | { kind: "url", url }                  the object alone at a URL (a pinned IPFS object, say)
//          | { kind: "ipfs", cid }                 the object's raw CID (objects of at most 1 MiB: the CID is the κ)
//          | { kind: "ipfs-pieces" }               each 1 MiB piece by its own raw CID (pieced objects)
//          | { kind: "layout", layout }            rebuilt from a layout, every segment checked (files)
// A holder is a hint: a wrong one costs one fetch, never a wrong byte, because every read is checked against κ.
import { cidOf, PIECE } from "./kappa-get.mjs";

const HF = "https://huggingface.co";
const enc = (p) => p.split("/").map(encodeURIComponent).join("/");

// json(κ) -> parsed held object or null (async); alts(κ) -> [{repo, rev, path, off, len} | {url, off, len}]
export async function recordsOf(repo, m, { json, alts = () => [] }) {
  const recs = new Map(), lc = repo.toLowerCase(), hub = (k) => ({ kind: "hub", path: `/v2/models/${lc}/blobs/${k}` });
  const put = (kappa, fields, ref) => {
    let r = recs.get(kappa);
    if (!r) recs.set(kappa, r = { kappa, refs: [], holders: [], ...fields });
    else Object.assign(r, Object.fromEntries(Object.entries(fields).filter(([, v]) => v !== undefined)));
    if (ref && !r.refs.includes(ref)) r.refs.push(ref);
    return r;
  };
  const hold = (r, h) => { if (!r.holders.some((x) => JSON.stringify(x) === JSON.stringify(h))) r.holders.push(h); };
  const small = (r) => { if (r.size !== undefined && r.size <= PIECE) hold(r, { kind: "ipfs", cid: cidOf(r.kappa.slice(7)) }); };

  const index = await json(m.index);
  const ri = put(m.index, { type: "oci-index", media: index?.mediaType }); hold(ri, hub(m.index));
  const pieceSets = new Map();
  // original first: a file's bytes can appear under other names in other formats (model.safetensors is also the
  // sharded render's model-00001-of-00001.safetensors); every name is kept, per format, and `path` is the original's
  const formats = Object.entries(m.manifests).sort(([a], [b]) => (a === "original" ? -1 : b === "original" ? 1 : 0));
  for (const [format, md] of formats) {
    const man = await json(md);
    const rm = put(md, { type: "oci-manifest", media: man?.mediaType }, m.index); hold(rm, hub(md));
    if (!man) continue;
    const tk = man.config.digest, table = await json(tk);
    const rt = put(tk, { type: "tensor-table", media: man.config.mediaType, size: man.config.size }, md); hold(rt, hub(tk)); small(rt);
    if (table?.pieces && !pieceSets.has(table.pieces)) {
      const P = await json(table.pieces);
      pieceSets.set(table.pieces, P);
      const rp = put(table.pieces, { type: "pieces", media: "application/vnd.hologram.pieces.v1+json" }, tk); hold(rp, hub(table.pieces));
    }
    for (const l of man.layers) {
      const path = l.annotations?.["org.opencontainers.image.title"], blob = m.blobs[l.digest] || {};
      const rf = put(l.digest, { type: "file", size: l.size }, md);
      if (!rf.path) rf.path = path;
      rf.names ||= [];
      if (!rf.names.some((n) => n.format === format && n.path === path)) rf.names.push({ format, path });
      if (blob.held) hold(rf, hub(l.digest));
      if (blob.upstream) hold(rf, { kind: "hf", url: `${HF}/${repo}/resolve/${m.rev}/${enc(blob.path || path)}`, at: 0 });
      small(rf);
      const lk = l.annotations?.["org.hologram.layout"];
      if (!lk) continue;
      hold(rf, { kind: "layout", layout: lk });
      const lay = await json(lk);
      const rl = put(lk, { type: "layout", media: "application/vnd.hologram.layout.v1+json" }, l.digest); hold(rl, hub(lk));
      for (const [kind, k, len, where] of lay?.segments || []) {
        if (kind === "l") { const r = put(k, { type: "literal", size: len }, lk); hold(r, hub(k)); small(r); continue; }
        const r = put(k, { type: kind === "s" ? "storage" : "tensor", size: len }, lk);
        if (typeof where === "number") hold(r, { kind: "hf", url: `${HF}/${repo}/resolve/${m.rev}/${enc(lay.file)}`, at: where });
        for (const a of alts(k)) hold(r, a.url ? { kind: "url", url: a.url } : { kind: "hf", url: `${HF}/${a.repo}/resolve/${a.rev}/${enc(a.path)}`, at: a.off });
        small(r);
      }
    }
  }
  // the model κ: its held bytes name every tensor κ (canonicalBytes in tensors/lib/model.mjs)
  if (m.canonical) { const r = put(m.canonical, { type: "model", media: "text/plain; charset=utf-8" }, m.index); hold(r, hub(m.canonical)); }
  if (m.provenance) { const r = put(m.provenance, { type: "provenance", media: "application/vnd.hologram.provenance.v1+json" }, m.index); hold(r, hub(m.provenance)); }
  // piece lists: every pieced payload names the pieces object that holds its hashes, and is held piece by piece on IPFS
  for (const [pk, P] of pieceSets) for (const [k, list] of Object.entries(P?.of || {})) {
    const r = recs.get(k);
    if (!r) continue;
    r.pieces = { in: pk, n: list.length };
    hold(r, { kind: "ipfs-pieces" });
  }
  return recs;
}

// Every sha256 digest in a text (a dialect's answer body or headers).
export const digestsIn = (text) => [...new Set(String(text).match(/sha256:[0-9a-f]{64}/g) || [])];
