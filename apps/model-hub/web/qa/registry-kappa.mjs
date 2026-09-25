// Every row on the Registry page has a κ, or a named reason it has none. Runs inside build.mjs, so the
// gate travels with the site source (the VPS build script is a copy nobody updates).
//
//   node qa/registry-kappa.mjs [dist]
//
// A build whose index lost its addresses, or whose rows silently fell out of "addressed" without a reason,
// cannot ship. The threshold is a floor, not a target: 30 rows are gone upstream today and stay listed.
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { HOST } from "../src/origin.mjs";

const REASONS = new Set(["gone", "error", "rate-limited-retry", "licence-refused"]);
// The hub's own name comes from src/origin.mjs, the one file that holds it; a κ is an address on this host.
const KAPPA = new RegExp(`^${HOST.replace(/[.]/g, "\\.")}/[a-z0-9._/-]+@sha256:[0-9a-f]{64}$`);
const TRUST = new Set(["upstream-digest", "upstream-attested", "first-seen"]);

export async function checkKappa(dist, { floor = 0.9 } = {}) {
  const data = JSON.parse(await readFile(join(dist, "registry", "data", "images.json"), "utf8"));
  const fail = [];
  if (!data.kappaIndex?.digest?.startsWith("sha256:")) fail.push("no sealed index digest (kappaIndex)");
  let addressed = 0, reasoned = 0, live = 0;
  for (const r of data.images || []) {
    if (r.here && !r.kappa) { live++; continue; }             // our own registry's rows carry a digest already
    if (r.kappa) {
      addressed++;
      if (!KAPPA.test(r.kappa)) fail.push(`${r.id}: malformed κ ${r.kappa}`);
      if (r.digest !== r.kappa.split("@")[1]) fail.push(`${r.id}: digest differs from its κ`);
      if (!TRUST.has(r.trust)) fail.push(`${r.id}: unknown trust ${r.trust}`);
      continue;
    }
    if (REASONS.has(r.kappaStatus)) reasoned++;
    else fail.push(`${r.id}: no κ and no reason`);
  }
  const total = (data.images || []).length;
  if (addressed + reasoned + live !== total) fail.push(`rows do not add up: ${addressed}+${reasoned}+${live} != ${total}`);
  if (addressed / total < floor) fail.push(`only ${addressed}/${total} rows addressed; floor is ${floor * 100}%`);
  if (fail.length) throw new Error("registry κ gate: " + fail.slice(0, 12).join("; ") + (fail.length > 12 ? ` (+${fail.length - 12})` : ""));
  return `registry κ: ${addressed}/${total} addressed, ${reasoned} with a reason, index ${data.kappaIndex.digest.slice(0, 19)}…`;
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("registry-kappa.mjs")) {
  console.log(await checkKappa(process.argv[2] || "dist"));
}
