// Package one verified model snapshot as a hologram-live library manifest.
//
//   node pack-model.mjs <files dir> <out dir> <index.json from hologram-api>
//
// Every file must match the SHA-256 the index records (checked here, fail closed). Files are cut into fixed 64 MiB
// chunks; each distinct chunk is one tensor layer addressed by BLAKE3, so the registry never holds a large upload in
// memory and chunks shared across models or revisions are stored once.
//
// Layer 0 (entry "manifest") is model.json:
//   {format, id, source, revision, chunk_bytes, files: [{path, sha256, blake3, size, chunks: ["blake3:<hex>", ...]}]}
// A client pulls the archive, concatenates each file's chunks in order, and checks both whole-file addresses.

import { createBLAKE3, createSHA256 } from "hash-wasm";
import { createReadStream } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const CHUNK = 64 * 1024 * 1024;
const [filesDir, outDir, indexPath] = process.argv.slice(2);
const index = JSON.parse(await readFile(indexPath, "utf8"));
const store = join(outDir, "store", "blobs", "blake3");
await mkdir(store, { recursive: true });

const b3hex = async (bytes) => { const h = await createBLAKE3(); h.update(bytes); return h.digest("hex"); };

const files = [], layers = [], seen = new Set();
for (const f of [...index.files].sort((a, b) => a.path.localeCompare(b.path))) {
  const whole3 = await createBLAKE3(), whole256 = await createSHA256();
  const chunks = [];
  let buffer = Buffer.alloc(0);
  const flush = async (bytes) => {
    const hex = await b3hex(bytes);
    chunks.push(`blake3:${hex}`);
    if (seen.has(hex)) return;
    seen.add(hex);
    await writeFile(join(store, hex), bytes);
    layers.push({ kind: "tensor", path: `store/blobs/blake3/${hex}`, entry: `b3_${hex}` });
  };
  for await (const piece of createReadStream(join(filesDir, f.path), { highWaterMark: 8 << 20 })) {
    whole3.update(piece);
    whole256.update(piece);
    buffer = Buffer.concat([buffer, piece]);
    while (buffer.length >= CHUNK) {
      await flush(buffer.subarray(0, CHUNK));
      buffer = buffer.subarray(CHUNK);
    }
  }
  if (buffer.length || !chunks.length) await flush(buffer);
  const sha256 = `sha256:${whole256.digest("hex")}`;
  if (sha256 !== f.address) {
    console.error(`MISMATCH ${f.path}: got ${sha256}, index says ${f.address}`);
    process.exit(3);
  }
  files.push({ path: f.path, sha256, blake3: `blake3:${whole3.digest("hex")}`, size: f.size, chunks });
}

const manifest = { format: "hologram.model-hub.model/v1", id: index.name, source: "huggingface.co", revision: index.revision, index_manifest: index.manifest, chunk_bytes: CHUNK, files };
const manifestBytes = Buffer.from(`${JSON.stringify(manifest)}\n`);
const manifestHex = await b3hex(manifestBytes);
await writeFile(join(outDir, "model.json"), manifestBytes);
await writeFile(join(store, manifestHex), manifestBytes);

await writeFile(join(outDir, "hologram.json"), `${JSON.stringify({ schema_version: 4, library: true, layers: [{ kind: "tensor", path: "model.json", entry: "manifest" }, ...layers] }, null, 2)}\n`);
console.log(JSON.stringify({ id: index.name, revision: index.revision, files: files.length, layers: layers.length + 1, bytes: files.reduce((s, x) => s + (x.size || 0), 0) }));

