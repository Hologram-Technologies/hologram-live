// Audit: is every hash in the tensor index a κ (sha256 of the named bytes), and does every reference verify?
// Read-only. Walks: every held object; the sealed day root and everything it reaches; tensor tables; layouts;
// the CAR (block hashes, and κ = raw CID for objects <= 1 MiB).
import { readFileSync, readdirSync, statSync, createReadStream } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";
import { CarBlockIterator } from "@ipld/car/iterator";
import { sha256 as mh } from "multiformats/hashes/sha2";
import { cidFromSha256 } from "./lib/ipfs-recipe.mjs";

// Usage: TENSOR_STATE=<state> node audit.mjs   (exit 1 if anything fails)
const STATE = process.env.TENSOR_STATE || join(dirname(fileURLToPath(import.meta.url)), "state");

const K = /^sha256:[0-9a-f]{64}$/;
const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
const get = (d) => { try { return readFileSync(join(STATE, "sha256", d.slice(7))); } catch { return null; } };
const out = { held: 0, heldBad: [], reached: new Set(), refs: 0, refsBad: [], notKappa: [], tensors: 0, tensorsBad: 0, segments: 0, layoutsBad: [], models: 0 };

// 1. every held object: its name is the sha256 of its bytes
for (const hex of readdirSync(join(STATE, "sha256"))) {
  if (hex.endsWith(".tmp")) continue;
  out.held++;
  const b = readFileSync(join(STATE, "sha256", hex));
  if (sha(b) !== `sha256:${hex}`) out.heldBad.push(hex);
}

// 2. from the sealed root down
const ref = (d, where, mustHold = true) => {
  out.refs++;
  if (!K.test(d)) { out.notKappa.push(`${where}: ${d}`); return null; }
  out.reached.add(d);
  const b = get(d);
  if (mustHold && (!b || sha(b) !== d)) out.refsBad.push(`${where}: ${d}`);
  return b;
};
const latest = JSON.parse(readFileSync(join(STATE, "latest.json"), "utf8"));
const root = JSON.parse(ref(latest.digest, "latest.json -> day root"));
const models = JSON.parse(readFileSync(join(STATE, "models.json"), "utf8"));

// The names log: its chain replays to the signed head, and the signature verifies.
if (root.names?.log) {
  const { verifyLog } = await import("./lib/names.mjs");
  const [entries, problems] = verifyLog(ref(root.names.log, "names log").toString(), JSON.parse(ref(root.names.head, "names head")));
  out.names = { entries: entries.length, problems };
}

for (const [repo, r] of Object.entries(root.models)) {
  out.models++;
  const idx = JSON.parse(ref(r.index, `${repo} index`));
  if (r.canonical && !K.test(r.canonical)) out.notKappa.push(`${repo} canonical: ${r.canonical}`);
  for (const m of idx.manifests) {
    const man = JSON.parse(ref(m.digest, `${repo} manifest ${m.annotations?.["org.hologram.format"]}`));
    const table = JSON.parse(ref(man.config.digest, `${repo} tensor table`));
    for (const row of table.tensors) { out.tensors++; if (!K.test(row[4])) out.tensorsBad++; }
    for (const l of man.layers) {
      const blob = models[repo].blobs[l.digest];
      if (!K.test(l.digest)) out.notKappa.push(`${repo} layer ${l.digest}`);
      if (blob?.held) ref(l.digest, `${repo} held file ${blob.path}`);
      const lay = l.annotations?.["org.hologram.layout"];
      if (!lay) continue;
      const L = JSON.parse(ref(lay, `${repo} layout ${l.annotations["org.opencontainers.image.title"]}`));
      let total = 0;
      for (const s of L.segments) {
        out.segments++; total += s[2];
        if (!K.test(s[1])) out.notKappa.push(`${repo} ${L.file} segment ${s[1]}`);
        if (s[0] === "l") ref(s[1], `${repo} ${L.file} literal`);
        if (s[0] === "v" && !K.test(s[3].s)) out.notKappa.push(`${repo} ${L.file} storage ${s[3].s}`);
      }
      if (total !== L.size || L.digest !== l.digest || L.size !== l.size) out.layoutsBad.push(`${repo} ${L.file}`);
    }
  }
}

// 3. the CAR: every block's bytes hash to its CID; every object <= 1 MiB has CID = raw CID of its κ
const car = JSON.parse(readFileSync(join(STATE, "car", "ipfs.json"), "utf8"));
let blocks = 0, blockBad = 0;
const cids = new Set();
for await (const { cid, bytes } of await CarBlockIterator.fromIterable(createReadStream(car.car))) {
  blocks++; cids.add(cid.toString());
  const d = await mh.digest(bytes);
  if (Buffer.compare(Buffer.from(d.digest), Buffer.from(cid.multihash.digest)) !== 0) blockBad++;
}
let rawEqual = 0, rawMissing = 0, larger = 0;
// only objects the sealed root reaches must be in its CAR; newer objects belong to the next day's CAR
for (const d of out.reached) {
  const hex = d.slice(7);
  if (statSync(join(STATE, "sha256", hex)).size > 1 << 20) { larger++; continue; }
  if (cids.has(cidFromSha256(hex).toString())) rawEqual++; else rawMissing++;
}

const report = {
  heldObjects: out.held, heldNameEqualsHash: out.held - out.heldBad.length, heldBad: out.heldBad.slice(0, 5),
  sealedRoot: latest.digest, modelsInRoot: out.models,
  names: out.names || null, referencesWalked: out.refs, referencesThatFailed: out.refsBad.slice(0, 5), distinctObjectsReached: out.reached.size,
  identifiersThatAreNotKappa: out.notKappa.slice(0, 5), notKappaCount: out.notKappa.length,
  tensorRows: out.tensors, tensorRowsWithoutKappa: out.tensorsBad, layoutSegments: out.segments, layoutsThatDoNotCoverTheirFile: out.layoutsBad,
  car: { blocks, blocksWhoseBytesDoNotMatchTheirCid: blockBad, root: car.root, sealsThisRoot: car.sealed === latest.digest },
  reachedObjectsUpTo1MiB: { cidEqualsKappa: rawEqual, notInCar: rawMissing }, reachedObjectsOver1MiB: larger, heldSinceTheSeal: out.held - out.reached.size,
};
console.log(JSON.stringify(report, null, 1));
const failed = out.heldBad.length || out.refsBad.length || out.notKappa.length || out.tensorsBad || out.layoutsBad.length || blockBad || rawMissing;
process.exit(failed ? 1 : 0);
