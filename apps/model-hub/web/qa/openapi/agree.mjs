// Do the dialects agree about the same model?
//
// The architecture promises "one base URL, many dialects, one truth", and the ADR's own sequencing records that
// step 5 — one source of truth under all dialects — is not done: the dialects read the site's published file
// lists while the agent API reads content-addressed objects. Nobody had measured what that costs.
//
// For each model this asks four planes the same question and diffs the answers:
//   HF      GET /api/models/{id}/tree/main            path -> oid (SHA-256), plus the pinned revision
//   OCI     GET /v2/{id}/manifests/latest (ModelPack) layer digest per org.cncf.model.filepath
//   MCP     POST /mcp tools/call get_model            files with sha256
//   OBJECT  catalog -> objects[id].model -> that object's files
//
// A disagreement about a hash is the highest-severity finding this endpoint can produce: every safety property
// rests on the planes agreeing about what the bytes should be.
//
//   node qa/openapi/agree.mjs [--n 20] [--out .../agreement.json]
import { writeFile, mkdir } from "node:fs/promises";

const arg = (n, d) => { const i = process.argv.indexOf(`--${n}`); return i > -1 ? process.argv[i + 1] : d; };
const BASE = arg("base", "https://gethologram.ai").replace(/\/$/, "");
const N = Number(arg("n", 20));
const OUT = arg("out", null);

const get = async (p, init) => fetch(p.startsWith("http") ? p : BASE + p, { signal: AbortSignal.timeout(40_000), ...init });
const json = async (p, init) => { try { const r = await get(p, init); return r.ok ? await r.json() : null; } catch { return null; } };

async function hf(id) {
  const tree = await json(`/api/models/${id}/tree/main`);
  const info = await json(`/api/models/${id}`);
  if (!Array.isArray(tree)) return null;
  return { revision: info?.sha, files: new Map(tree.map((f) => [f.path, f.oid])) };
}

async function oci(id) {
  const m = await json(`/v2/${id.toLowerCase()}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } });
  if (!m?.layers) return null;
  const files = new Map();
  for (const l of m.layers) {
    const path = l.annotations?.["org.cncf.model.filepath"] || l.annotations?.["org.opencontainers.image.title"];
    if (path) files.set(path, String(l.digest).replace(/^sha256:/, ""));
  }
  return { revision: m.annotations?.["org.opencontainers.image.revision"], files };
}

async function mcp(id) {
  const r = await get("/mcp", {
    method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "get_model", arguments: { id } } }),
  });
  const text = (await r.text()).replace(/^data: /gm, "").trim().split("\n").pop();
  let doc; try { doc = JSON.parse(JSON.parse(text).result.content[0].text); } catch { return null; }
  if (!Array.isArray(doc.files)) return null;
  // get_model caps at 200 files by design and says so; a capped list is not a disagreement about the model.
  return { revision: doc.revision, truncated: Boolean(doc.files_truncated), files: new Map(doc.files.map((f) => [f.path, (f.sha256 || "").replace(/^sha256:/, "")])) };
}

async function object(address) {
  const doc = await json(`/api/v1/objects/${address}`);
  if (!doc?.files) return null;
  return { revision: doc.revision, files: new Map(doc.files.map((f) => [f.path, (f.sha256 || "").replace(/^sha256:/, "")])) };
}

// Compare two planes on the models they both claim to have.
function diff(a, b) {
  if (!a || !b) return null;
  const onlyA = [], onlyB = [], mismatched = [];
  // When one side is a documented truncation, only the files it did return are comparable.
  const truncated = a.truncated || b.truncated;
  for (const [p, h] of a.files) {
    if (!b.files.has(p)) { if (!truncated) onlyA.push(p); }
    else if (b.files.get(p) !== h) mismatched.push({ path: p, a: h, b: b.files.get(p) });
  }
  for (const p of b.files.keys()) if (!a.files.has(p) && !truncated) onlyB.push(p);
  return { truncated, shared: a.files.size - onlyA.length, onlyA: onlyA.length, onlyB: onlyB.length, mismatched, revisionSame: !a.revision || !b.revision || a.revision === b.revision };
}

async function main() {
  const descriptor = await json("/.well-known/model-hub.json");
  const catalog = await json(`/api/v1/objects/${descriptor.catalog}`);
  const rows = await json(`/api/models?limit=500&sort=downloads`);
  const searchable = new Set(rows.map((r) => r.id));
  const addressed = Object.keys(catalog.objects);

  // Span the space deliberately: big and small, searchable and not, GGUF and not.
  const pick = [];
  for (const r of rows) if (pick.length < Math.ceil(N * 0.6)) pick.push(r.id);
  for (const id of addressed) if (!searchable.has(id) && pick.length < N) pick.push(id);
  console.log(`# ${catalog.objects && Object.keys(catalog.objects).length} models addressed, ${rows.length} searchable`);
  console.log(`# comparing ${pick.length}: ${pick.filter((p) => searchable.has(p)).length} searchable, ${pick.filter((p) => !searchable.has(p)).length} addressed-only\n`);

  const out = [];
  for (const id of pick) {
    const addr = catalog.objects[id]?.model;
    const [h, o, m, b] = await Promise.all([hf(id), oci(id), mcp(id), addr ? object(addr) : null]);
    const planes = { hf: !!h, oci: !!o, mcp: !!m, object: !!b };
    const pairs = { "hf~oci": diff(h, o), "hf~mcp": diff(h, m), "hf~object": diff(h, b), "oci~object": diff(o, b) };
    const mism = Object.entries(pairs).filter(([, d]) => d && d.mismatched.length);
    const setdiff = Object.entries(pairs).filter(([, d]) => d && (d.onlyA || d.onlyB));
    const revdiff = Object.entries(pairs).filter(([, d]) => d && !d.revisionSame);
    const verdict = mism.length ? "HASH MISMATCH" : setdiff.length ? "file-set differs" : revdiff.length ? "revision differs" : Object.values(planes).filter(Boolean).length < 2 ? "too few planes" : "agree";
    out.push({ id, searchable: searchable.has(id), planes, files: h?.files.size ?? o?.files.size ?? 0, pairs, verdict });
    const absent = Object.entries(planes).filter(([, v]) => !v).map(([k]) => k);
    console.log(`${verdict === "agree" ? "ok  " : "DIFF"} ${id.padEnd(46)} ${String(h?.files.size ?? "-").padStart(3)} files  planes:${Object.values(planes).filter(Boolean).length}/4${absent.length ? ` (no ${absent.join(",")})` : ""}  ${verdict}`);
    for (const [k, d] of mism) console.log(`      ${k}: ${d.mismatched.length} hash mismatches, first ${d.mismatched[0].path}`);
    for (const [k, d] of setdiff) console.log(`      ${k}: +${d.onlyA} only in first, +${d.onlyB} only in second`);
    for (const [k] of revdiff) console.log(`      ${k}: revision differs`);
  }

  const agree = out.filter((r) => r.verdict === "agree").length;
  const hashes = out.filter((r) => r.verdict === "HASH MISMATCH").length;
  console.log(`\n${out.length} models · ${agree} fully agree · ${hashes} hash mismatches · ${out.length - agree - hashes} other differences`);
  const coverage = { hf: out.filter((r) => r.planes.hf).length, oci: out.filter((r) => r.planes.oci).length, mcp: out.filter((r) => r.planes.mcp).length, object: out.filter((r) => r.planes.object).length };
  console.log(`plane coverage of ${out.length}: ${Object.entries(coverage).map(([k, v]) => `${k} ${v}`).join(" · ")}`);

  if (OUT) {
    await mkdir(OUT.replace(/[^/\\]+$/, ""), { recursive: true }).catch(() => {});
    await writeFile(OUT, JSON.stringify({ base: BASE, at: new Date().toISOString(), addressed: addressed.length, searchable: rows.length, coverage, summary: { agree, hashMismatch: hashes, other: out.length - agree - hashes }, models: out }, null, 1) + "\n");
    console.log(`\nwrote ${OUT}`);
  }
}

main().catch((e) => { console.error(e); process.exit(1); });
