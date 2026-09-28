#!/usr/bin/env node
// The dialects emit no digest the Registry has no record of, and every record's κ recomputes from its bytes.
//
//   TENSOR_STATE=<state> node check-dialects.mjs            exit 1 on any failure
//
// Runs the tensor mirror in-process against the state and calls every answer it gives for every indexed model:
// tags, the index and every manifest, the tensor table, the records, every layout's alternatives, and the headers
// of every file blob. Every sha256 found in a body or a header must be a κ with a record (provenance rows are
// excluded: their narrow-dtype digests are equivalence claims, not objects). Then every record the hub holds is
// fetched through the mirror and hashed.
import http from "node:http";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { tensorMirror } from "../deploy/tensor-mirror.mjs";
import { digestsIn } from "../deploy/kappa-records.mjs";

const STATE = process.env.TENSOR_STATE;
if (!STATE) { console.error("TENSOR_STATE is required"); process.exit(2); }
const models = JSON.parse(readFileSync(join(STATE, "models.json"), "utf8"));
const server = http.createServer(async (req, res) => { if (!(await tensorMirror(req, res, new URL(req.url, "http://x").pathname))) { res.writeHead(404); res.end(); } });
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
const base = `http://127.0.0.1:${server.address().port}`;
const call = async (path, init = {}) => { const r = await fetch(base + path, { redirect: "manual", ...init }); const body = init.method === "HEAD" ? "" : await r.text(); return { r, body, text: body + " " + [...r.headers].map(([k, v]) => `${k}: ${v}`).join("\n") }; };

let fails = 0, checked = 0, answers = 0;
const bad = (what) => { fails++; console.log(`FAIL ${what}`); };
for (const [repo, m] of Object.entries(models)) {
  if (!m.gated) continue;
  const lc = repo.toLowerCase(), v2 = `/v2/models/${lc}`;
  const recs = JSON.parse((await call(`${v2}/records`)).body);
  const known = new Set(recs.records.map((r) => r.kappa));
  const see = (where, text) => { answers++; for (const d of digestsIn(text)) { checked++; if (!known.has(d)) bad(`${repo}: ${where} emits ${d.slice(0, 19)}… with no record`); } };
  const tags = await call(`${v2}/tags/list`); see("tags", tags.text);
  for (const t of JSON.parse(tags.body).tags) see(`manifest ${t}`, (await call(`${v2}/manifests/${t}`, { headers: { accept: "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json" } })).text);
  see("tensor table", (await call(`${v2}/tensors`)).text);
  see("records", JSON.stringify(recs));
  for (const r of recs.records) {
    if (r.type === "layout") see(`alternatives ${r.kappa.slice(7, 19)}`, (await call(`${v2}/alternatives/${r.kappa}`)).text);
    if (r.type === "file") see(`HEAD ${r.path}`, (await call(`${v2}/blobs/${r.kappa}`, { method: "HEAD" })).text);
  }
  // every record the hub holds: fetch it through the mirror, hash it
  let held = 0;
  for (const r of recs.records) {
    if (!r.holders.some((h) => h.kind === "hub")) continue;
    const res = await fetch(base + `${v2}/blobs/${r.kappa}`);
    const b = Buffer.from(await res.arrayBuffer());
    if (`sha256:${createHash("sha256").update(b).digest("hex")}` !== r.kappa) bad(`${repo}: held ${r.type} ${r.kappa.slice(0, 19)}… does not hash to its κ (${res.status})`);
    held++;
  }
  const types = {}; for (const r of recs.records) types[r.type] = (types[r.type] || 0) + 1;
  console.log(`${repo}: ${recs.records.length} records ${JSON.stringify(types)}; ${held} held, each re-hashed`);
  if (!recs.records.some((r) => r.type === "model" && r.kappa === m.canonical)) bad(`${repo}: the model κ ${m.canonical} has no record`);
}
server.close();
console.log(`${answers} answers, ${checked} digests checked against the records, ${fails} failures`);
process.exit(fails ? 1 : 0);
