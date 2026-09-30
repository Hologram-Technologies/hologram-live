// Provenance of one model against a base, from two published tensor tables (application/vnd.hologram.tensors.v2).
// The same rules as spikes/kappa-provenance/provenance.py; the page recomputes a registry's lineage claim with it
// instead of trusting the claim.
//
// bytesShared  exact. Share of the model's tensor bytes whose canonical tensor κ (canonical dtype + shape +
//              values; name, file, sharding and exact dtype widening ignored) is in the base's multiset, each base
//              tensor matched at most once; a tensor that does not match whole is credited with its 1024-row
//              blocks whose κ the base holds (vocabulary resizes).
// lineage      deterministic sample. Over matrices with a same-name, same-shape base tensor: the share of sampled
//              sign bits that agree, weighted by tensor size. Independent weights agree about half the time.
// coverage     share of the model's matrix bytes that lineage stands on.

const rows = (t) => Object.values(t.files).flatMap((f) => f.tensors);
const norm = (name) => name.replace(/^(model\.|transformer\.)/, "");
const count = (keys) => { const m = new Map(); for (const k of keys) m.set(k, (m.get(k) || 0) + 1); return m; };
const take = (m, k) => { const n = m.get(k) || 0; if (!n) return false; m.set(k, n - 1); return true; };
const POP = Uint8Array.from({ length: 256 }, (_, i) => { let c = 0; for (let x = i; x; x >>= 1) c += x & 1; return c; });

function agreement(aHex, bHex, n) {
  let diff = 0;
  for (let i = 0; i < aHex.length; i += 2) diff += POP[parseInt(aHex.slice(i, i + 2), 16) ^ parseInt(bHex.slice(i, i + 2), 16)];
  return (n - diff) / n; // the packed tail is zero-padded on both sides, so it never differs
}

export function score(child, base) {
  const C = rows(child), B = rows(base);
  const pool = count(B.map((r) => r.kappa));
  const total = C.reduce((s, r) => s + r.length, 0);
  let whole = 0, sharedTensors = 0;
  const rest = [], matched = [];
  for (const r of C) {
    if (take(pool, r.kappa)) { whole += r.length; sharedTensors++; matched.push(r.kappa); } else rest.push(r);
  }
  const consumed = count(matched), blocks = new Map();
  for (const r of B) {
    if (take(consumed, r.kappa)) continue;
    for (const [k] of r.blocks || []) blocks.set(k, (blocks.get(k) || 0) + 1);
  }
  let partial = 0;
  for (const r of rest) for (const [k, n] of r.blocks || []) if (take(blocks, k)) partial += n;

  const byName = new Map();
  for (const r of B) { const key = `${norm(r.name)}|${r.shape.join(",")}`; if (!byName.has(key)) byName.set(key, r); }
  const mats = C.filter((r) => r.shape.length >= 2 && r.sign);
  const matBytes = mats.reduce((s, r) => s + r.length, 0);
  let agree = 0, weight = 0, covered = 0;
  for (const r of mats) {
    const b = byName.get(`${norm(r.name)}|${r.shape.join(",")}`);
    if (!b || !b.sign || b.sign_n !== r.sign_n) continue;
    const w = r.shape.reduce((p, d) => p * d, 1);
    agree += agreement(r.sign, b.sign, r.sign_n) * w; weight += w; covered += r.length;
  }
  const pct = (x, d) => Math.round(x * 100 * 10 ** d) / 10 ** d;
  return {
    bytesShared: total ? pct((whole + partial) / total, 4) : 0,
    wholeTensor: total ? pct(whole / total, 4) : 0,
    sharedTensors, tensors: C.length,
    lineage: weight ? pct(agree / weight, 2) : null,
    coverage: matBytes ? pct(covered / matBytes, 2) : 0,
  };
}

// Lineage on a 0-100 scale: 0 = independent training (half the signs agree), 100 = the same weights.
export function kinship(s) {
  return s.lineage === null ? null : Math.max(0, Math.round((s.lineage - 50) * 2 * 10) / 10);
}

// What the two numbers say together, with the thresholds they were read against (ten measured pairs, 2026-09-25:
// independent same-shape training 49.9 %, every fine-tune, merge or abliteration 95.6 % and above).
export function verdict(s) {
  if (s.bytesShared >= 99.99) return "Same weights";
  if (s.bytesShared >= 50) return "Edited copy";
  if (s.lineage === null) return "No comparable tensors";
  if (s.lineage >= 90) return "Fine-tuned or merged";
  if (s.lineage <= 60) return "Unrelated";
  return "Inconclusive";
}
