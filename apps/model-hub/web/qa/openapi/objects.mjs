// Does the content-addressed plane actually keep its promise?
//
// Every safety property of this hub rests on one claim: an object's name is the BLAKE3 of its bytes, so anyone can
// check what they were given without trusting the host that gave it to them. The stress run could not test that —
// Node has no BLAKE3 — so the single most important property went unverified. This verifies it.
//
// It also walks the archive: /archive.json chains one index object per day, and a chain nobody has followed is a
// chain nobody should rely on.
//
//   node qa/openapi/objects.mjs [--n 12] [--out .../objects.json]
import { writeFile, mkdir } from "node:fs/promises";
import { createBLAKE3 } from "hash-wasm";

const arg = (n, d) => { const i = process.argv.indexOf(`--${n}`); return i > -1 ? process.argv[i + 1] : d; };
const BASE = arg("base", "https://gethologram.ai").replace(/\/$/, "");
const N = Number(arg("n", 12));
const OUT = arg("out", null);

const pass = [], fail = [], results = [];
const ok = (n, d = "") => { pass.push(n); console.log(`ok   ${n.padEnd(52)} ${d}`); };
const bad = (n, d) => { fail.push(`${n} — ${d}`); console.log(`FAIL ${n.padEnd(52)} ${d}`); };

const get = (p) => fetch(p.startsWith("http") ? p : BASE + p, { signal: AbortSignal.timeout(60_000) });

// The address is the hash of exactly the bytes served, so hash the bytes and not a re-serialisation of them.
async function addressOf(bytes) {
  const h = await createBLAKE3();
  h.update(new Uint8Array(bytes));
  return `blake3:${h.digest("hex")}`;
}

async function checkObject(label, address) {
  const r = await get(`/api/v1/objects/${address}`);
  if (!r.ok) { bad(`${label} fetched`, `${r.status}`); return null; }
  const bytes = await r.arrayBuffer();
  const got = await addressOf(bytes);
  const match = got === address;
  results.push({ label, address, bytes: bytes.byteLength, computed: got, match });
  if (match) ok(`${label} hashes to its own address`, `${bytes.byteLength} B · ${address.slice(0, 20)}…`);
  else bad(`${label} hashes to its own address`, `served bytes hash to ${got.slice(0, 26)}…, asked for ${address.slice(0, 26)}…`);
  try { return JSON.parse(Buffer.from(bytes).toString("utf8")); } catch { return null; }
}

async function main() {
  console.log(`# verifying the content-addressed plane against ${BASE}\n`);
  const descriptor = await (await get("/.well-known/model-hub.json")).json();

  // 1. the catalog the descriptor points at
  const catalog = await checkObject("the catalog", descriptor.catalog);
  if (!catalog) { console.log("\ncannot continue without the catalog"); process.exit(1); }

  // 2. a spread of model objects and their source records
  const ids = Object.keys(catalog.objects).slice(0, N);
  let sourcesChecked = 0;
  for (const id of ids) {
    const entry = catalog.objects[id];
    const model = await checkObject(`model ${id}`, entry.model);
    for (const src of (entry.sources || []).slice(0, 1)) { await checkObject(`  source of ${id}`, src); sourcesChecked++; }
    if (model && !Array.isArray(model.files)) bad(`model ${id} has files`, "no files array");
  }
  console.log(`\n# ${ids.length} model objects, ${sourcesChecked} source records\n`);

  // 3. a tampered address must not verify — otherwise the check above proves nothing
  const flipped = descriptor.catalog.slice(0, -1) + (descriptor.catalog.endsWith("a") ? "b" : "a");
  const r = await get(`/api/v1/objects/${flipped}`);
  if (r.status === 404) ok("a one-character change in the address is not found", "404");
  else bad("a one-character change in the address is not found", `${r.status} — an address that is not the hash answered`);

  // 4. the archive chain: every day should name the day before it, and each day's index should be fetchable
  const archive = await (await get("/archive.json")).json();
  const days = archive.days || [];
  ok("the archive lists days", `${days.length} days, ${days[0]?.date} → ${days[days.length - 1]?.date}`);
  let chainOk = true, linked = 0;
  for (let i = 1; i < days.length; i++) {
    // The chain links by the previous day's pinned CID, which is how a reader walks backwards without this host.
    if (days[i].prev && days[i].prev === days[i - 1].cid) linked++;
    else if (days[i].prev) { chainOk = false; bad(`archive ${days[i].date} links to the day before`, `prev ${String(days[i].prev).slice(0, 18)}… but ${days[i - 1].date} is ${String(days[i - 1].cid).slice(0, 18)}…`); }
  }
  if (chainOk) ok("every day links to the one before it", `${linked} of ${Math.max(0, days.length - 1)} links verified`);

  // 5. past days must still be retrievable by address, not just listed
  const sample = days.slice(-4, -1);
  for (const day of sample) {
    if (!day.catalog) { console.log(`··   ${day.date} lists no catalog address`); continue; }
    await checkObject(`the catalog of ${day.date}`, day.catalog);
  }

  console.log(`\n${pass.length} passed, ${fail.length} failed`);
  if (fail.length) for (const f of fail) console.log(`  ${f}`);

  if (OUT) {
    await mkdir(OUT.replace(/[^/\\]+$/, ""), { recursive: true }).catch(() => {});
    await writeFile(OUT, JSON.stringify({ base: BASE, at: new Date().toISOString(), summary: { pass: pass.length, fail: fail.length }, objects: results, archiveDays: days.length }, null, 1) + "\n");
    console.log(`\nwrote ${OUT}`);
  }
  process.exit(fail.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
