// The day's tensor index on IPFS: one CAR holding the sealed root and every object it reaches (model indexes,
// manifests, tensor tables, layouts, literals, small files), as a UnixFS directory under the hub's fixed recipe.
//
//   sha256/<hex>   every object, named by its OCI digest; for objects <= 1 MiB the raw CID is that same hash
//   root.json      the sealed day root (the κ named by latest.json)
//   README.md      how to verify
//
// Tensor payloads are not in the CAR: their first source is the byte range inside the file Hugging Face already
// hosts, recorded in every layout. Pinning payload bytes is a separate, budgeted choice.
//   TENSOR_STATE=<dir> node car.mjs   ->  <state>/car/tensor-index-<day>.car + ipfs.json, re-verified block by block
import { readFileSync, writeFileSync, createWriteStream, createReadStream, statSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { CarWriter } from "@ipld/car";
import { CarBlockIterator } from "@ipld/car/iterator";
import { sha256 } from "multiformats/hashes/sha2";

const HERE = dirname(fileURLToPath(import.meta.url));
const RECIPE_PATH = process.env.IPFS_RECIPE || join(HERE, "lib", "ipfs-recipe.mjs");   // the hub's unixfs-v1-2025 recipe, vendored
const { fileImporter, directoryRoot, cidFromSha256, CHUNK, RECIPE } = await import(pathToFileURL(RECIPE_PATH).href);
const STATE = process.env.TENSOR_STATE || join(HERE, "state");
const obj = (d) => join(STATE, "sha256", d.slice(7));
const json = (d) => JSON.parse(readFileSync(obj(d), "utf8"));

export async function buildCar() {
  const latest = JSON.parse(readFileSync(join(STATE, "latest.json"), "utf8"));
  const root = json(latest.digest);
  // walk: root -> model index -> manifests -> config + held layers + layouts -> literals
  const need = new Set([latest.digest]);
  const models = JSON.parse(readFileSync(join(STATE, "models.json"), "utf8"));
  for (const [repo, r] of Object.entries(root.models)) {
    const m = models[repo];
    need.add(r.index);
    if (r.provenance) need.add(r.provenance);        // per-κ provenance sample (sample.mjs)
    for (const md of Object.values(m.manifests)) {
      need.add(md);
      const man = json(md);
      need.add(man.config.digest);
      for (const l of man.layers) {
        if (m.blobs[l.digest]?.held) need.add(l.digest);
        const lay = l.annotations?.["org.hologram.layout"];
        if (lay) { need.add(lay); for (const s of json(lay).segments) if (s[0] === "l") need.add(s[1]); }
      }
    }
  }
  const readme = `# Hologram tensor index on IPFS (${root.day})

root.json is the sealed day root; its sha256 is ${latest.digest}.
Every file under sha256/ is an object named by the SHA-256 of its bytes: model indexes and manifests (OCI 1.1),
tensor tables (the manifests' config), layouts (how each file is rebuilt from literals and tensor κs), literals,
and small files. For a file of at most 1 MiB its CID is the raw-leaf CID of that same hash (${RECIPE}).
Verify any file: sha256sum sha256/<hex> must print <hex>.
Tensor payloads are addressed, not included: each layout names every tensor by κ and records where Hugging Face
already serves those bytes; any holder of a κ can serve it, and the reader checks it.
`;
  const entries = [...need].map((d) => ({ path: `sha256/${d.slice(7)}`, file: obj(d), hex: d.slice(7) }));
  const extra = [{ path: "root.json", bytes: readFileSync(obj(latest.digest)) }, { path: "README.md", bytes: Buffer.from(readme) }];
  const files = [];
  for (const e of entries) {
    const size = statSync(e.file).size;
    if (size <= CHUNK) { files.push({ ...e, size, cid: cidFromSha256(e.hex), dagSize: size, small: true }); continue; }
    const imp = fileImporter(); await imp.write(readFileSync(e.file)); const r = await imp.close();
    files.push({ ...e, size, cid: r.cid, dagSize: r.dagSize, small: false });
  }
  for (const x of extra) {
    const imp = fileImporter(); await imp.write(x.bytes); const r = await imp.close();
    files.push({ path: x.path, bytes: x.bytes, size: x.bytes.length, cid: r.cid, dagSize: r.dagSize, small: false });
  }
  const dir = await directoryRoot(files.map((f) => ({ path: f.path, cid: f.cid, dagSize: f.dagSize })));
  mkdirSync(join(STATE, "car"), { recursive: true });
  const carPath = join(STATE, "car", `tensor-index-${root.day}.car`);
  const { writer, out } = CarWriter.create([dir.cid]);
  const sink = createWriteStream(carPath), done = new Promise((ok) => sink.on("finish", ok));
  (async () => { for await (const c of out) sink.write(c); sink.end(); })();
  let blocks = 0;
  for (const f of files) {
    const bytes = f.bytes || readFileSync(f.file);
    if (f.small) { await writer.put({ cid: f.cid, bytes }); blocks++; continue; }
    const imp = fileImporter(async (b) => { await writer.put(b); blocks++; }); await imp.write(bytes); await imp.close();
  }
  for (const b of dir.blocks) { await writer.put(b); blocks++; }
  await writer.close(); await done;

  // verify: every block's bytes hash to its CID's multihash (sha2-256)
  let checked = 0, bad = 0;
  for await (const { cid, bytes } of await CarBlockIterator.fromIterable(createReadStream(carPath))) {
    const mh = await sha256.digest(bytes);
    if (Buffer.compare(Buffer.from(mh.digest), Buffer.from(cid.multihash.digest)) !== 0) bad++;
    checked++;
  }
  const manifest = { recipe: RECIPE, day: root.day, root: dir.cid.toString(), sealed: latest.digest, objects: need.size, blocks, checked, bad, carBytes: statSync(carPath).size, car: carPath };
  writeFileSync(join(STATE, "car", "ipfs.json"), JSON.stringify(manifest, null, 1));
  if (bad) throw new Error(`CAR verification failed: ${bad} of ${checked} blocks`);
  return manifest;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) console.log(JSON.stringify(await buildCar(), null, 1));
