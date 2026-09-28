// The tensor index is only served when it is whole. Runs inside build.mjs (gates travel with the site source).
//
//   node qa/tensor-index.mjs [state dir]      default $TENSOR_STATE, else $HUB_STATE/tensors
//
// For every model marked gated in models.json: the index, every manifest and its config are held and hash to
// their names; every weight layer's layout covers exactly the file's size; every literal a layout needs is held;
// gates.json records a passing rebuild for this same revision. A state directory that does not exist yet is
// reported and skipped (the feature is not deployed there); a state that exists and fails stops the build.
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;

export function checkTensorIndex(dir = process.env.TENSOR_STATE || (process.env.HUB_STATE && join(process.env.HUB_STATE, "tensors"))) {
  if (!dir || !existsSync(join(dir, "models.json"))) return `tensor index: no state at ${dir || "(unset)"}, skipped`;
  const get = (d) => { try { const b = readFileSync(join(dir, "sha256", d.slice(7))); return sha(b) === d ? b : null; } catch { return null; } };
  const models = JSON.parse(readFileSync(join(dir, "models.json"), "utf8"));
  const gates = existsSync(join(dir, "gates.json")) ? JSON.parse(readFileSync(join(dir, "gates.json"), "utf8")) : {};
  const fail = []; let n = 0, layouts = 0;
  for (const [repo, m] of Object.entries(models)) {
    if (!m.gated) continue;
    n++;
    if (gates[repo]?.rev !== m.rev || !gates[repo]?.pass) fail.push(`${repo}: marked gated without a passing rebuild for ${m.rev.slice(0, 12)}`);
    if (!get(m.index)) fail.push(`${repo}: index ${m.index} not held`);
    for (const [fmt, d] of Object.entries(m.manifests)) {
      const b = get(d); if (!b) { fail.push(`${repo}: ${fmt} manifest not held`); continue; }
      const man = JSON.parse(b);
      if (!get(man.config.digest)) fail.push(`${repo}: ${fmt} tensor table not held`);
      for (const l of man.layers) {
        const lay = l.annotations?.["org.hologram.layout"];
        if (!lay) { if (m.blobs[l.digest]?.held && !get(l.digest)) fail.push(`${repo}: held file ${l.annotations?.["org.opencontainers.image.title"]} missing`); continue; }
        const lb = get(lay); if (!lb) { fail.push(`${repo}: layout ${lay} not held`); continue; }
        layouts++;
        const L = JSON.parse(lb), total = L.segments.reduce((a, s) => a + s[2], 0);
        if (total !== l.size || L.size !== l.size || L.digest !== l.digest) fail.push(`${repo}: layout of ${L.file} covers ${total} of ${l.size} bytes`);
        for (const s of L.segments) if (s[0] === "l" && !get(s[1])) fail.push(`${repo}: literal ${s[1].slice(0, 19)} of ${L.file} not held`);
      }
    }
  }
  if (fail.length) throw new Error("tensor index gate: " + fail.slice(0, 12).join("; ") + (fail.length > 12 ? ` (+${fail.length - 12})` : ""));
  return `tensor index: ${n} models whole, ${layouts} layouts cover their files, every literal held`;
}

if (process.argv[1]?.endsWith("tensor-index.mjs")) console.log(checkTensorIndex(process.argv[2]));
