// Turn one day's Model Hub index (web/data) into a hologram-live library manifest plus a staged object store.
//
//   node snapshot.mjs <data dir> <out dir> <date> <source commit>
//
// Output in <out dir>:
//   index.json       {format, date, snapshot, source, files: [[path, "blake3:<hex>", size], ...]}, sorted by path
//   hologram.json    library .holo manifest: layer 0 = index.json, then one tensor layer per distinct file content
//   store/blobs/blake3/<hex>   every layer's bytes, so `hologram push` finds each payload locally
//
// One layer per distinct content means files unchanged since yesterday are layers the registry already holds.

import { createBLAKE3 } from "hash-wasm";
import { copyFile, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative, sep } from "node:path";

const [dataDir, outDir, date, source] = process.argv.slice(2);
if (!dataDir || !outDir || !/^\d{4}-\d{2}-\d{2}$/.test(date || "")) {
  console.error("usage: node snapshot.mjs <data dir> <out dir> <YYYY-MM-DD> <source commit>");
  process.exit(2);
}

async function walk(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(path)));
    else if (entry.isFile()) out.push(path);
  }
  return out;
}

const blake3 = async (bytes) => {
  const h = await createBLAKE3();
  h.update(bytes);
  return h.digest("hex");
};

const store = join(outDir, "store", "blobs", "blake3");
const layersDir = join(outDir, "layers");
await mkdir(store, { recursive: true });
await mkdir(layersDir, { recursive: true });

const files = [];
const distinct = new Map(); // hex -> absolute source path
for (const abs of (await walk(dataDir)).sort()) {
  const path = relative(dataDir, abs).split(sep).join("/");
  if (path === "pending.txt") continue;
  const bytes = await readFile(abs);
  const hex = await blake3(bytes);
  files.push([path, `blake3:${hex}`, bytes.length]);
  if (!distinct.has(hex)) distinct.set(hex, abs);
}

const models = JSON.parse(await readFile(join(dataDir, "models.json"), "utf8"));
const index = { format: "hologram.model-hub.index/v1", date, snapshot: models.snapshot, source, models: models.models.length, files };
const indexBytes = Buffer.from(`${JSON.stringify(index)}\n`);
const indexHex = await blake3(indexBytes);
await writeFile(join(outDir, "index.json"), indexBytes);

// Layer sources live under layers/<hex> so the manifest never depends on directory structure or file names.
const layers = [{ kind: "tensor", path: "index.json", entry: "index" }]; // entries must be unique per archive
await writeFile(join(store, indexHex), indexBytes);
for (const [hex, abs] of [...distinct].sort(([a], [b]) => a.localeCompare(b))) {
  if (hex === indexHex) continue;
  await copyFile(abs, join(layersDir, hex));
  await copyFile(abs, join(store, hex));
  layers.push({ kind: "tensor", path: `layers/${hex}`, entry: `b3_${hex}` });
}

await writeFile(join(outDir, "hologram.json"), `${JSON.stringify({ schema_version: 4, library: true, layers }, null, 2)}\n`);
const bytes = files.reduce((sum, f) => sum + f[2], 0);
console.log(JSON.stringify({ date, files: files.length, distinct: layers.length, bytes, index: `blake3:${indexHex}` }));

