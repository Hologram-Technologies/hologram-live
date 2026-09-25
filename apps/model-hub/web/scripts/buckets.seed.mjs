// Seed public buckets on a registry from local files, with the page's own library, so what lands is exactly
// what the Buckets page and the CLI would have written: every file cut into 1 MiB blocks addressed by their
// sha-256, an object manifest per file, an index tree, and a head tagged `latest` and by its moment.
//
//   node scripts/buckets.seed.mjs --registry https://hub.uor.foundation --token "user:password" [plan.json]
//
// The plan (default scripts/buckets.seed.json) names each bucket, what to say about it, and which local
// files or folders go in. Paths are relative to web/. A bucket that already exists is republished with the
// same files: unchanged blocks are HEADed and not re-sent, so running it twice is cheap and harmless.
// The token is the registry's write credential (`user:password` for the Caddy basic auth, or a bearer);
// it goes in the Authorization header and nowhere else.

import { readFile, readdir, stat } from "node:fs/promises";
import { join, relative, basename, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { registry, writeObject, INDEX_MT, MANIFEST_MT, DEFAULT_QUOTA } from "../public/buckets/lib/buckets-lib.mjs";
import { build, objectManifest } from "../public/buckets/lib/octree.mjs";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const opt = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : fallback; };
const base = (opt("--registry", process.env.HUB_REGISTRY || "https://hub.uor.foundation")).replace(/\/$/, "");
const raw = opt("--token", process.env.HUB_REGISTRY_TOKEN || "");
if (!raw) { console.error("no credential: --token user:password (or HUB_REGISTRY_TOKEN)"); process.exit(2); }
const token = /^(Basic|Bearer) /.test(raw) ? raw : raw.includes(":") ? "Basic " + Buffer.from(raw).toString("base64") : raw;
const planPath = args.find((a) => a.endsWith(".json")) || join(WEB, "scripts", "buckets.seed.json");
const plan = JSON.parse(await readFile(planPath, "utf8"));

const reg = registry(base);
const human = (n) => (n >= 1e9 ? (n / 1e9).toFixed(2) + " GB" : n >= 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.round(n / 1e3) + " KB");

async function walk(dir, under = "") {
  const out = [];
  for (const d of await readdir(dir, { withFileTypes: true })) {
    const p = join(dir, d.name);
    if (d.isDirectory()) out.push(...(await walk(p, under + d.name + "/")));
    else out.push({ key: under + d.name, path: p });
  }
  return out;
}

// Every file the plan names, keyed the way it will be listed in the bucket.
async function filesOf(entry) {
  const files = [];
  const only = entry.only ? new RegExp(entry.only) : null;
  for (const src of entry.sources) {
    const abs = join(WEB, src);
    const s = await stat(abs);
    if (s.isDirectory()) {
      const prefix = entry.prefixed ? basename(abs) + "/" : "";
      for (const f of await walk(abs)) files.push({ key: prefix + f.key, path: f.path });
    } else files.push({ key: basename(abs), path: abs });
  }
  return files.filter((f) => !only || only.test(f.key)).sort((a, b) => a.key.localeCompare(b.key));
}

// The page's publishRoot, for a public bucket: the tree's inner nodes, then the head with the annotations
// the page reads, tagged latest and by its moment so every state stays listable.
async function publish(repo, entries, description) {
  const prior = await reg.manifest(repo, "latest").catch(() => null);
  const pa = (prior && prior.json.annotations) || {};
  const now = new Date().toISOString();
  const list = entries.map(([key, v]) => [key, { manifest: v.manifest, manifestSize: v.manifestSize || 0, root: v.root, size: v.size, mtime: v.mtime }]);
  const bytes = list.reduce((s, [, v]) => s + v.size, 0);
  const tree = build(list);
  for (const node of tree.nodes) if (node.digest !== tree.root) await reg.putManifest(repo, node.digest, node.bytes, INDEX_MT, token);
  const root = JSON.parse(new TextDecoder().decode(tree.nodes.find((n) => n.digest === tree.root).bytes));
  root.annotations = {
    ...root.annotations,
    "foundation.uor.bucket.type": "application/vnd.uor.bucket.v1",
    "foundation.uor.bucket.objects": String(list.length),
    "foundation.uor.bucket.bytes": String(bytes),
    "foundation.uor.bucket.levels": String(tree.levels),
    "foundation.uor.bucket.visibility": "public",
    "foundation.uor.bucket.created": pa["foundation.uor.bucket.created"] || now,
    "foundation.uor.bucket.quota.bytes": String(Number(pa["foundation.uor.bucket.quota.bytes"] || DEFAULT_QUOTA)),
    ...(description ? { "foundation.uor.bucket.description": description } : {}),
    ...(prior ? { "foundation.uor.bucket.parent": prior.digest } : {}),
    "foundation.uor.bucket.published": now,
  };
  const head = new TextEncoder().encode(JSON.stringify(root));
  const digest = await reg.putManifest(repo, "latest", head, INDEX_MT, token);
  await reg.putManifest(repo, "h-" + Date.now(), head, INDEX_MT, token);
  return { digest, bytes, objects: list.length };
}

for (const entry of plan) {
  const files = await filesOf(entry);
  console.log(`${entry.repo}: ${files.length} files`);
  const entries = new Map();
  for (const f of files) {
    const bytes = await readFile(f.path);
    const m = await stat(f.path);
    const file = new File([bytes], basename(f.key), { lastModified: m.mtimeMs });
    const object = await writeObject(reg, entry.repo, f.key, file, token, () => {});
    const om = objectManifest({ key: object.wire, root: object.root, size: object.size, mtime: object.mtime, blocks: object.blocks });
    await reg.putManifest(entry.repo, om.digest, om.bytes, MANIFEST_MT, token);
    entries.set(f.key, { manifest: om.digest, root: object.root, size: object.size, mtime: object.mtime, manifestSize: om.size });
    process.stdout.write(`  ${f.key} ${human(object.size)}\n`);
  }
  const r = await publish(entry.repo, [...entries], entry.description);
  console.log(`  → ${r.objects} objects, ${human(r.bytes)}, head ${r.digest}\n`);
}
console.log(`seeded ${plan.length} buckets on ${base}`);
