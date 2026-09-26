// Proof that a model comes back from IPFS alone: rebuild every weight file of each model from its layout, with the
// Hugging Face origin pointed at a dead address, every tensor read by CID from an IPFS gateway, and require the
// sha256 Hugging Face published. Streams to a hash; stores nothing.
//
//   TENSOR_STATE=<state> IPFS_GATEWAY=http://127.0.0.1:8181 node verify-ipfs.mjs --list top10.txt [--max-mb 2000]
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { assemble } from "../deploy/tensor-assemble.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATE = process.env.TENSOR_STATE || join(HERE, "state");
const GW = (process.env.IPFS_GATEWAY || "http://127.0.0.1:8181").replace(/\/$/, "");
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const maxBytes = Number(arg("--max-mb", "100000")) * 1e6;
const repos = readFileSync(arg("--list"), "utf8").split(/\r?\n/).map((l) => l.replace(/#.*/, "").trim()).filter(Boolean);
const models = JSON.parse(readFileSync(join(STATE, "models.json"), "utf8"));
const pins = JSON.parse(readFileSync(join(STATE, "pins.json"), "utf8"));
const literal = (d) => { try { return readFileSync(join(STATE, "sha256", d.slice(7))); } catch { return null; } };
const alternatives = (k) => (pins[k] ? [{ url: `${GW}/ipfs/${pins[k].cid}`, off: 0, len: pins[k].len }] : []);

let ok = 0, bad = 0, bytes = 0; const t0 = Date.now();
for (const repo of repos) {
  const m = models[repo]; if (!m) { console.log(`${repo}: not indexed`); continue; }
  for (const f of m.files) {
    if (f.size > maxBytes) { console.log(`${repo}/${f.path}: ${(f.size / 1e9).toFixed(2)} GB, over --max-mb, skipped`); continue; }
    const layout = JSON.parse(literal(m.blobs[f.digest].layout).toString("utf8"));
    const from = new Map();
    const ctx = { origin: "http://127.0.0.1:9", repo, rev: m.rev, literal, alternatives,
      report: (e) => { if (e.ok) { const s = e.from.startsWith("http") ? "ipfs" : "other"; from.set(s, (from.get(s) || 0) + e.len); } } };
    const h = createHash("sha256"); let n = 0; const s0 = Date.now();
    try { for await (const c of assemble(layout, ctx)) { h.update(c); n += c.length; } }
    catch (e) { bad++; console.log(`${repo}/${f.path}: FAILED ${e.message}`); continue; }
    const got = `sha256:${h.digest("hex")}`, pass = got === f.digest;
    pass ? ok++ : bad++; bytes += n;
    console.log(`${repo}/${f.path}: ${pass ? "equal" : "DIFFERENT"} ${(n / 1e6).toFixed(1)} MB in ${((Date.now() - s0) / 1000).toFixed(1)} s, tensors from ${[...from].map(([k, v]) => `${k} ${(v / 1e6).toFixed(0)} MB`).join(", ") || "literals only"}`);
  }
}
console.log(`\n${ok} files equal to Hugging Face's sha256, ${bad} not; ${(bytes / 1e9).toFixed(2)} GB rebuilt from IPFS in ${((Date.now() - t0) / 1000).toFixed(0)} s, Hugging Face unreachable throughout`);
process.exit(bad ? 1 : 0);
