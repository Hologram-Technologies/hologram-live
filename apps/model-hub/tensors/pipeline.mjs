#!/usr/bin/env node
// The tensor index pipeline. Every stage is idempotent and resumable; one runner at a time (lock file).
//
//   node pipeline.mjs discover                 queue every model the hub indexes, by downloads (skips done revisions)
//   node pipeline.mjs run [repo...]            index the named repos, or the queue head; --budget-gb N, --list file
//   node pipeline.mjs gate [repo...]           rebuild files from tensors and require Hugging Face's exact sha256
//   node pipeline.mjs seal                     publish the day's index root over every gated model
//   node pipeline.mjs car                      the day's root and objects as one verified CAR (for IPFS)
//   node pipeline.mjs pin <repo...|--list f>   each tensor payload as its own IPFS object, once; IPFS_API = a Kubo RPC
//                                              (Filebase: IPFS_API=https://rpc.filebase.io IPFS_API_TOKEN=<key>); --budget-gb N
//   node audit.mjs                             prove every hash is a κ: held objects, the sealed root down, the CAR
//   node pipeline.mjs status                   one screen: models, tensors, bytes, queue, failures, latest root
//   node pipeline.mjs nightly [--budget-gb N]  discover -> run -> gate -> seal -> car
//
// State lives in $TENSOR_STATE (default ./state): sha256/ (held objects), models.json, queue.json, gates.json,
// failures.json, sources/<repo>.json, roots/<day>.json. Nothing is written to any live host.
import { existsSync, readFileSync, writeFileSync, mkdirSync, rmSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { Store } from "./lib/store.mjs";
import { indexModel, T } from "./lib/model.mjs";
import { assemble } from "../deploy/tensor-assemble.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATE = process.env.TENSOR_STATE || join(HERE, "state");
mkdirSync(join(STATE, "sources"), { recursive: true }); mkdirSync(join(STATE, "roots"), { recursive: true });
const store = new Store(STATE);
const read = (f, d) => { try { return JSON.parse(readFileSync(join(STATE, f), "utf8")); } catch { return d; } };
const write = (f, v) => writeFileSync(join(STATE, f), JSON.stringify(v, null, 1));
const day = () => new Date().toISOString().slice(0, 10);
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const safe = (repo) => repo.replace("/", "__");

// ---------------------------------------------------------------- lock
const LOCK = join(STATE, "lock");
function lock() {
  if (existsSync(LOCK)) {
    const { pid, at } = JSON.parse(readFileSync(LOCK, "utf8"));
    let alive = false; try { process.kill(pid, 0); alive = true; } catch {}
    if (alive && Date.now() - at < 24 * 3600e3) { console.error(`another runner (pid ${pid}) holds ${LOCK}`); process.exit(2); }
  }
  writeFileSync(LOCK, JSON.stringify({ pid: process.pid, at: Date.now() }));
  process.on("exit", () => { try { if (JSON.parse(readFileSync(LOCK, "utf8")).pid === process.pid) rmSync(LOCK); } catch {} });
}

// ---------------------------------------------------------------- discover
async function discover() {
  const hub = process.env.HUB_ORIGIN || "https://gethologram.ai";
  const seen = new Map();
  for (const sort of ["downloads", "trendingScore", "likes"]) {
    const r = await fetch(`${hub}/api/models?sort=${sort}&limit=500`, { signal: AbortSignal.timeout(30_000) });
    if (!r.ok) { log(`hub /api/models?sort=${sort}: ${r.status}`); continue; }
    for (const m of await r.json()) {
      const prev = seen.get(m.id);
      if (!prev) seen.set(m.id, { repo: m.id, rev: m.sha, downloads: m.downloads || 0, bytes: m.hologram?.weight_bytes || 0, gated: !!m.gated });
    }
  }
  const models = read("models.json", {}), failures = read("failures.json", {});
  const queue = [...seen.values()].filter((m) => !m.gated && models[m.repo]?.rev !== m.rev && !(failures[m.repo]?.permanent))
    .sort((a, b) => b.downloads - a.downloads);
  write("queue.json", { at: new Date().toISOString(), items: queue });
  log(`queue: ${queue.length} models (of ${seen.size} on the hub; ${Object.keys(models).length} already indexed)`);
}

// ---------------------------------------------------------------- run
async function run(repos) {
  const models = read("models.json", {}), failures = read("failures.json", {});
  const budget = Number(arg("--budget-gb", "Infinity")) * 1e9;
  let spent = 0;
  if (!repos.length) {
    const list = arg("--list"); repos = list ? readFileSync(list, "utf8").split(/\r?\n/).map((l) => l.replace(/#.*/, "").trim()).filter(Boolean) : read("queue.json", { items: [] }).items.map((m) => m.repo);
  }
  for (const repo of repos) {
    if (spent >= budget) { log(`budget reached (${(spent / 1e9).toFixed(1)} GB)`); break; }
    log(`index ${repo}`);
    try {
      // Same revision already indexed and gated: nothing to hash (one metadata call, no bytes).
      if (models[repo]?.gated && !process.argv.includes("--force")) {
        const { info } = await import("./lib/src.mjs");
        if ((await info(repo)).sha === models[repo].rev) { log(`  already indexed at ${models[repo].rev.slice(0, 12)}`); continue; }
      }
      const r = await indexModel(repo, { store, log });
      if (models[repo]?.rev === r.rev && models[repo]?.index === r.index) { log(`  unchanged`); continue; }
      writeFileSync(join(STATE, "sources", `${safe(repo)}.json`), JSON.stringify(r.sources));
      const { sources, ...rest } = r;
      if (models[repo] && models[repo].rev !== r.rev) (rest.history ||= models[repo].history || []).push({ rev: models[repo].rev, index: models[repo].index, until: new Date().toISOString() });
      models[repo] = { ...rest, indexedAt: new Date().toISOString(), gated: false };
      delete failures[repo];
      spent += r.weightBytes;
      log(`  ${r.tensors} tensors, ${(r.weightBytes / 1e9).toFixed(2)} GB in ${r.seconds} s; index ${r.index.slice(0, 19)}; canonical ${r.canonical?.slice(0, 19)}`);
    } catch (e) {
      failures[repo] = { error: e.message, at: new Date().toISOString(), permanent: !!e.skip };
      log(`  FAILED ${e.message}`);
    }
    write("models.json", models); write("failures.json", failures);
  }
}

// ---------------------------------------------------------------- gate
// Rebuild from tensors, trying every OTHER holder of each κ first (other repos, other formats), and require the
// published sha256. Original: the smallest weight file. Renders: the single-file safetensors when <= 400 MB.
export function alternativesIndex() {
  const idx = new Map();
  for (const f of readdirSync(join(STATE, "sources"))) for (const [k, repo, rev, path, off, len] of JSON.parse(readFileSync(join(STATE, "sources", f), "utf8"))) {
    if (!idx.has(k)) idx.set(k, []);
    idx.get(k).push({ repo, rev, path, off, len });
  }
  const gw = process.env.IPFS_GATEWAY;                              // pinned payloads: each tensor its own IPFS object
  if (gw) for (const [k, p] of Object.entries(read("pins.json", {}))) (idx.get(k) || idx.set(k, []).get(k)).push({ url: `${gw.replace(/\/$/, "")}/ipfs/${p.cid}`, off: 0, len: p.len });
  return idx;
}

// ---------------------------------------------------------------- pin
// Put a model's tensor payloads on IPFS, one UnixFS object per tensor (cut on tensor boundaries, so every repack,
// reshard and format that holds the same tensor shares the same CID). Bytes stream from the model's own file on
// Hugging Face straight into the node's add API; nothing is staged. For a payload of at most 1 MiB the returned
// CID must be the raw CID of its κ; larger ones get the chunked DAG and are verified by reading them back.
async function pin(repos) {
  const api = process.env.IPFS_API; if (!api) { console.error("set IPFS_API (a Kubo RPC endpoint you control)"); process.exit(1); }
  const { stream, cdnUrl } = await import("./lib/src.mjs");
  // raw-leaf CIDv1 of a sha256 (what a payload of at most 1 MiB must pin as); no dependencies, so pin runs anywhere
  const cidFromSha256 = (hex) => { const bytes = Buffer.concat([Buffer.from([1, 0x55, 0x12, 0x20]), Buffer.from(hex, "hex")]); const A = "abcdefghijklmnopqrstuvwxyz234567"; let bits = 0, v = 0, o = "b"; for (const x of bytes) { v = (v << 8) | x; bits += 8; while (bits >= 5) { o += A[(v >>> (bits - 5)) & 31]; bits -= 5; } } if (bits) o += A[(v << (5 - bits)) & 31]; return o; };
  const pins = read("pins.json", {}), models = read("models.json", {});
  const list = arg("--list"); if (!repos.length && list) repos = readFileSync(list, "utf8").split(/\r?\n/).map((l) => l.replace(/#.*/, "").trim()).filter(Boolean);
  const budget = Number(arg("--budget-gb", "Infinity")) * 1e9; let spentAll = 0;
  // A payload is correct when its bytes hash to its κ, whatever the model's gate says; --ungated pins indexed models
  // whose layouts have not been rebuilt yet. The budget is checked per payload, so a 50 GB model fits a small disk.
  const ungated = process.argv.includes("--ungated"), N = Number(process.env.PIN_CONCURRENCY || 1);
  const bufMax = Number(process.env.PIN_BUFFER_MB || 64) * 2 ** 20;
  const headers = process.env.IPFS_API_TOKEN ? { authorization: `Bearer ${process.env.IPFS_API_TOKEN}` } : {};
  const { spawn } = await import("node:child_process");
  const pinOne = async ([k, r, rev, path, off, len]) => {
    const h = createHash("sha256"), src = stream((fresh) => cdnUrl(r, rev, path, fresh), off, off + len - 1);
    let cid;
    if (process.env.IPFS_ADD_CMD && len > bufMax) {                   // big payload: stream into the node, never whole
      const child = spawn("sh", ["-c", process.env.IPFS_ADD_CMD], { stdio: ["pipe", "pipe", "inherit"] });
      let outText = ""; child.stdout.on("data", (d) => (outText += d));
      const exited = new Promise((ok) => child.on("close", ok));
      for await (const c of src) { h.update(c); if (!child.stdin.write(c)) await new Promise((ok) => child.stdin.once("drain", ok)); }
      child.stdin.end();
      const code = await exited;
      if (code !== 0) throw new Error(`ipfs add exited ${code}`);
      cid = outText.trim().split(/\s+/).pop();
    } else {                                                          // small payload: one buffered RPC call
      if (len > bufMax) throw new Error(`${r}/${path}@${off}: ${(len / 2 ** 20).toFixed(0)} MB payload needs IPFS_ADD_CMD (streaming) rather than the buffered RPC path`);
      const parts = []; for await (const c of src) { h.update(c); parts.push(c); }
      const fd = new FormData(); fd.append("file", new Blob(parts), k.slice(7));
      const res = await fetch(`${api.replace(/\/$/, "")}/api/v0/add?cid-version=1&raw-leaves=true&chunker=size-1048576&pin=true&quieter=true`, { method: "POST", body: fd, headers });
      if (!res.ok) throw new Error(`ipfs add: ${res.status} ${await res.text()}`);
      cid = JSON.parse((await res.text()).trim().split("\n").pop()).Hash;
    }
    if (`sha256:${h.digest("hex")}` !== k) {                          // the bytes were not the κ: take the pin back and stop
      await fetch(`${api.replace(/\/$/, "")}/api/v0/pin/rm?arg=${cid}`, { method: "POST", headers }).catch(() => {});
      throw new Error(`${r}/${path}@${off}: bytes do not match ${k}; pin removed`);
    }
    if (len <= 1 << 20 && cid !== cidFromSha256(k.slice(7)).toString()) throw new Error(`${k}: CID ${cid} is not its raw CID`);
    return cid;
  };
  for (const repo of repos) {
    if (!models[repo]) { log(`pin ${repo}: not indexed, skipped`); continue; }
    if (!models[repo].gated && !ungated) { log(`pin ${repo}: not gated yet, skipped (--ungated pins it anyway)`); continue; }
    if (spentAll >= budget) { log(`pin budget reached (${(spentAll / 1e9).toFixed(1)} GB)`); break; }
    const rows = JSON.parse(readFileSync(join(STATE, "sources", `${safe(repo)}.json`), "utf8"));
    const todo = []; const queued = new Set(); let reused = 0;
    for (const row of rows) { if (pins[row[0]] || queued.has(row[0])) { reused++; continue; } queued.add(row[0]); todo.push(row); }
    let added = 0, bytes = 0, next = 0, stop = false;
    const worker = async () => {
      while (!stop && next < todo.length) {
        if (spentAll >= budget) { stop = true; break; }
        const row = todo[next++]; spentAll += row[5];
        const cid = await pinOne(row);
        pins[row[0]] = { cid, len: row[5], at: new Date().toISOString() }; added++; bytes += row[5];
        if (added % 200 === 0) write("pins.json", pins);              // resumable mid-model
      }
    };
    try { await Promise.all(Array.from({ length: Math.max(1, N) }, worker)); }
    finally { write("pins.json", pins); }
    log(`pinned ${repo}: ${added} payloads (${(bytes / 1e6).toFixed(1)} MB) added, ${reused} already pinned by another model or format${stop ? ", budget reached" : ""}`);
    if (stop) break;
  }
}
async function rebuild(m, blob, idx, prefer) {
  const layout = store.json(m.blobs[blob].layout);
  const h = createHash("sha256"); let n = 0; const from = new Map(); let bad = 0;
  const ctx = { repo: m.repo, rev: m.rev, literal: (d) => store.get(d), alternatives: (k) => idx.get(k) || [], prefer,
    report: (e) => { if (e.ok) { const r = e.from.replace(/@\d+$/, ""); from.set(r, (from.get(r) || 0) + e.len); } else bad++; } };
  for await (const c of assemble(layout, ctx)) { h.update(c); n += c.length; }
  const digest = `sha256:${h.digest("hex")}`;
  return { file: layout.file, size: n, ok: digest === blob, digest, from: Object.fromEntries(from), rejected: bad };
}
async function gate(repos) {
  const models = read("models.json", {}), gates = read("gates.json", {}), idx = alternativesIndex();
  for (const repo of repos.length ? repos : Object.keys(models).filter((r) => !models[r].gated)) {
    const m = models[repo]; if (!m) continue;
    const checks = [];
    try {
      const weights = m.files.filter((f) => m.blobs[f.digest]?.layout).sort((a, b) => a.size - b.size);
      if (weights[0]) checks.push({ kind: "original", ...(await rebuild(m, weights[0].digest, idx, "alternatives")) });
      const single = m.renders?.safetensors?.[0];
      if (single && single.size <= 400e6) checks.push({ kind: "safetensors", ...(await rebuild(m, single.digest, idx, "alternatives")) });
      const pass = checks.length > 0 && checks.every((c) => c.ok);
      gates[repo] = { rev: m.rev, pass, checks, at: new Date().toISOString() };
      m.gated = pass;
      log(`gate ${repo}: ${pass ? "PASS" : "FAIL"} ${checks.map((c) => `${c.kind} ${c.file} ${(c.size / 1e6).toFixed(1)} MB from ${Object.entries(c.from).map(([k, v]) => `${k} ${(v / 1e6).toFixed(1)} MB`).join(" + ")}${c.rejected ? ` (${c.rejected} rejected)` : ""}`).join("; ")}`);
    } catch (e) { gates[repo] = { rev: m.rev, pass: false, error: e.message, at: new Date().toISOString() }; m.gated = false; log(`gate ${repo}: FAIL ${e.message}`); }
    write("gates.json", gates); write("models.json", models);
  }
}

// ---------------------------------------------------------------- seal
// The day's root: every gated model's index digest, plus the derived relations. Stored as a κ object;
// roots/<day>.json and latest.json name it. Tables and manifests are shared across days by digest.
function seal() {
  const models = read("models.json", {}), idx = alternativesIndex();
  const pub = Object.fromEntries(Object.entries(models).filter(([, m]) => m.gated).map(([r, m]) => [r, { rev: m.rev, index: m.index, canonical: m.canonical, tensors: m.tensors, weightBytes: m.weightBytes, license: m.license }]));
  // relations: same weights (canonical κ equal), and tensors shared across repos (bytes)
  const byCanon = new Map(); for (const [r, m] of Object.entries(pub)) if (m.canonical) (byCanon.get(m.canonical) || byCanon.set(m.canonical, []).get(m.canonical)).push(r);
  const sameWeights = [...byCanon.values()].filter((l) => l.length > 1);
  const shared = new Map();
  for (const [k, list] of idx) {
    const repos = [...new Set(list.map((x) => x.repo))].filter((r) => pub[r]).sort();
    if (repos.length < 2) continue;
    for (let i = 0; i < repos.length; i++) for (let j = i + 1; j < repos.length; j++) { const key = `${repos[i]} ${repos[j]}`; const e = shared.get(key) || { tensors: 0, bytes: 0 }; e.tensors++; e.bytes += list[0].len; shared.set(key, e); }
  }
  const edges = [...shared].map(([k, v]) => { const [a, b] = k.split(" "); const lic = pub[a].license !== pub[b].license; return { a, b, ...v, sameWeights: pub[a].canonical === pub[b].canonical, licenceDiffers: lic }; }).sort((x, y) => y.bytes - x.bytes);
  const distinct = idx.size, instances = [...idx.values()].reduce((a, l) => a + l.length, 0);
  const root = { v: 1, day: day(), mediaType: "application/vnd.hologram.tensor-index.v1+json", models: pub, sameWeights, edges,
    counts: { models: Object.keys(pub).length, distinctPayloads: distinct, payloadInstances: instances } };
  const put = store.putJson(root);
  write(`roots/${day()}.json`, { digest: put.digest, size: put.size });
  write("latest.json", { day: day(), digest: put.digest, size: put.size });
  log(`sealed ${day()}: ${Object.keys(pub).length} models, ${distinct} distinct payload κs over ${instances} placements, ${edges.length} sharing edges; root ${put.digest}`);
  return put;
}

// ---------------------------------------------------------------- car
async function car() {
  process.env.TENSOR_STATE = STATE;
  const { buildCar } = await import("./car.mjs");
  const m = await buildCar();
  log(`car ${m.day}: root ${m.root}, ${m.objects} objects, ${m.blocks} blocks, ${(m.carBytes / 1e6).toFixed(1)} MB, ${m.checked} blocks re-verified`);
}

// ---------------------------------------------------------------- status
function status() {
  const models = read("models.json", {}), q = read("queue.json", { items: [] }), f = read("failures.json", {}), latest = read("latest.json", null);
  const ms = Object.values(models);
  const bytes = ms.reduce((a, m) => a + (m.weightBytes || 0), 0), tensors = ms.reduce((a, m) => a + (m.tensors || 0), 0);
  let held = 0, objs = 0; for (const x of readdirSync(join(STATE, "sha256"))) { objs++; held += statSync(join(STATE, "sha256", x)).size; }
  console.log(`models indexed   ${ms.length} (${ms.filter((m) => m.gated).length} gated)
tensors          ${tensors.toLocaleString()}
bytes hashed     ${(bytes / 1e9).toFixed(2)} GB
held objects     ${objs.toLocaleString()} (${(held / 1e6).toFixed(1)} MB, ${(100 * held / Math.max(1, bytes)).toFixed(3)}% of the weights)
queue            ${q.items.length}${q.at ? ` (built ${q.at.slice(0, 16)})` : ""}
failures         ${Object.keys(f).length}${Object.entries(f).slice(0, 5).map(([r, e]) => `\n  ${r}: ${e.error}`).join("")}
latest root      ${latest ? `${latest.day} ${latest.digest}` : "none"}`);
}

const [cmd, ...rest] = process.argv.slice(2);
const repos = rest.filter((x) => !x.startsWith("--") && !/^\d+(\.\d+)?$/.test(x) && !x.endsWith(".txt"));
if (["run", "gate", "seal", "nightly", "discover", "car", "pin"].includes(cmd)) lock();
if (cmd === "discover") await discover();
else if (cmd === "run") await run(repos);
else if (cmd === "gate") await gate(repos);
else if (cmd === "seal") seal();
else if (cmd === "car") await car();
else if (cmd === "pin") await pin(repos);
else if (cmd === "status") status();
else if (cmd === "nightly") {
  await discover(); await run([]); await gate([]); seal();
  try { await car(); } catch (e) { log(`car skipped: ${e.message}`); }   // the CAR is published separately; never blocks promotion
}
else console.log(readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 12).join("\n"));
