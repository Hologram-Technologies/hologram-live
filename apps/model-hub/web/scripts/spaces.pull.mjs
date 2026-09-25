// Pull a Space artifact from a registry into a folder, verifying every byte: the manifest against the digest
// it was asked for (or the tag's Docker-Content-Digest), the config and every layer against the manifest.
// The folder that results is the Space exactly as sealed — the same bytes a page serves, IPFS holds and the
// OS mounts. No token: reads are anonymous.
//
//   node scripts/spaces.pull.mjs <registry> <id> <out-dir> [tag-or-digest=latest]
//   node scripts/spaces.pull.mjs http://127.0.0.1:5055 kokoro-tts /tmp/pull
import { mkdirSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";

const [REG, ID, OUT, REF = "latest"] = process.argv.slice(2);
if (!REG || !ID || !OUT) { console.error("usage: node scripts/spaces.pull.mjs <registry> <id> <out-dir> [ref]"); process.exit(1); }
const ACCEPT = "application/vnd.oci.image.manifest.v1+json";
const sha = (b) => "sha256:" + createHash("sha256").update(b).digest("hex");
const repo = `spaces/${ID}`;
const t0 = Date.now();

const mr = await fetch(`${REG.replace(/\/+$/, "")}/v2/${repo}/manifests/${REF}`, { headers: { Accept: ACCEPT } });
if (!mr.ok) throw new Error(`manifest ${REF}: HTTP ${mr.status}`);
const manifestBytes = Buffer.from(await mr.arrayBuffer());
const kappa = sha(manifestBytes);
const claimed = mr.headers.get("docker-content-digest");
if (REF.startsWith("sha256:") && kappa !== REF) throw new Error(`manifest re-derives to ${kappa}, asked for ${REF}`);
if (claimed && claimed !== kappa) throw new Error(`registry says ${claimed}, bytes re-derive to ${kappa}`);
const manifest = JSON.parse(manifestBytes.toString("utf8"));
if (manifest.artifactType !== "application/vnd.hologram.space.v1+json") throw new Error("not a Space artifact: " + manifest.artifactType);

const dir = join(OUT, ID); mkdirSync(dir, { recursive: true });
writeFileSync(join(dir, "manifest.json"), manifestBytes);
let bytes = 0;
async function blob(d, path) {
  const r = await fetch(`${REG.replace(/\/+$/, "")}/v2/${repo}/blobs/${d.digest}`);
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  const b = Buffer.from(await r.arrayBuffer());
  if (sha(b) !== d.digest) throw new Error(`${path}: bytes re-derive to ${sha(b)}, manifest says ${d.digest} — refused`);
  if (d.size != null && b.length !== d.size) throw new Error(`${path}: ${b.length} bytes, manifest says ${d.size}`);
  mkdirSync(dirname(join(dir, path)), { recursive: true });
  writeFileSync(join(dir, path), b); bytes += b.length;
  return b;
}
await blob(manifest.config, "holospace.json");
for (const l of manifest.layers) await blob(l, l.annotations["org.opencontainers.image.title"]);
console.log(JSON.stringify({ id: ID, kappa, ipfs: manifest.annotations["foundation.uor.space.ipfs"] || null, layers: manifest.layers.length, bytes, ms: Date.now() - t0, dir }));
