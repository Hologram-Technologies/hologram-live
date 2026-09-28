#!/usr/bin/env node
// hologram pull: fetch an open model in the format you want, rebuilt from κ-addressed tensors on this machine.
//
//   node pull.mjs <owner/name> [--format original|safetensors|safetensors-sharded] [--out DIR]
//                 [--hub https://gethologram.ai] [--rev main] [--verify-only] [--prefer-alternatives]
//
// The hub supplies only small objects (manifest, layouts, literals, configs); every tensor is read straight
// from wherever it lives (its own file on Hugging Face, or any other repo or format holding the same κ),
// verified against its κ, and every file is checked against the digest in the manifest before it is kept.
import { createHash } from "node:crypto";
import { mkdirSync, createWriteStream, renameSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { assemble } from "../deploy/tensor-assemble.mjs";

const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const FLAGS = new Set(["--verify-only", "--prefer-alternatives"]);
const repo = args.find((a, i) => !a.startsWith("--") && !(i > 0 && args[i - 1].startsWith("--") && !FLAGS.has(args[i - 1])));
if (!repo) { console.log("usage: node pull.mjs <owner/name> [--format safetensors] [--out DIR] [--hub URL] [--verify-only]"); process.exit(1); }
const hub = opt("--hub", process.env.HOLOGRAM_HUB || "https://gethologram.ai").replace(/\/$/, "");
const format = opt("--format", "original"), rev = opt("--rev", "main"), out = opt("--out", repo.split("/")[1]);
const verifyOnly = args.includes("--verify-only"), prefer = args.includes("--prefer-alternatives") ? "alternatives" : undefined;
const name = repo.toLowerCase();
const base = `${hub}/v2/models/${name}`;
const get = async (path, accept) => { const r = await fetch(`${base}/${path}`, { headers: accept ? { accept } : {} }); if (!r.ok) throw new Error(`${path}: ${r.status} ${await r.text()}`); return Buffer.from(await r.arrayBuffer()); };
const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
const blob = async (d) => { const b = await get(`blobs/${d}`); if (sha(b) !== d) throw new Error(`${d} failed verification`); return b; };

const t0 = Date.now();
const man = JSON.parse(await get(`manifests/${rev}-${format}`, "application/vnd.oci.image.manifest.v1+json"));
const annot = man.annotations || {};
console.log(`${annot["org.hologram.repo"]}@${annot["org.hologram.revision"]?.slice(0, 12)} as ${format}: ${man.layers.length} files, canonical ${annot["org.hologram.canonical"]?.slice(0, 19)}`);
let bytes = 0; const sources = new Map();
for (const l of man.layers) {
  const path = l.annotations["org.opencontainers.image.title"], lay = l.annotations["org.hologram.layout"];
  const dest = join(out, path);
  let h = createHash("sha256"), n = 0, sink = null;
  if (!verifyOnly) { mkdirSync(dirname(dest), { recursive: true }); sink = createWriteStream(dest + ".part"); }
  const write = async (c) => { h.update(c); n += c.length; if (sink && !sink.write(c)) await new Promise((ok) => sink.once("drain", ok)); };
  if (lay) {
    const layout = JSON.parse(await blob(lay));
    const alts = JSON.parse(await get(`alternatives/${lay}`).catch(() => Buffer.from("{}")));
    const ctx = { repo: annot["org.hologram.repo"], rev: annot["org.hologram.revision"], literal: blob, alternatives: (k) => alts[k] || [], prefer,
      report: (e) => { if (e.ok) { const k = e.from.replace(/@\d+$/, ""); sources.set(k, (sources.get(k) || 0) + e.len); } } };
    for await (const c of assemble(layout, ctx)) await write(c);
  } else {
    const r = await fetch(`${base}/blobs/${l.digest}`);                   // small files from the hub; others via its redirect
    if (!r.ok) throw new Error(`${path}: ${r.status}`);
    for await (const c of r.body) await write(Buffer.from(c));
  }
  if (sink) await new Promise((ok) => sink.end(ok));
  const digest = `sha256:${h.digest("hex")}`;
  if (digest !== l.digest || n !== l.size) { if (sink) rmSync(dest + ".part"); throw new Error(`${path}: got ${digest} (${n} bytes), manifest says ${l.digest}`); }
  if (sink) renameSync(dest + ".part", dest);
  bytes += n;
  console.log(`  ${path.padEnd(48)} ${(n / 1e6).toFixed(1).padStart(8)} MB  ${lay ? "assembled" : "fetched"}  ok`);
}
const s = (Date.now() - t0) / 1000;
console.log(`${(bytes / 1e6).toFixed(1)} MB in ${s.toFixed(1)} s (${(bytes / 1e6 / s).toFixed(1)} MB/s), every file equal to its manifest digest`);
for (const [k, v] of sources) console.log(`  tensors from ${k}: ${(v / 1e6).toFixed(1)} MB`);
