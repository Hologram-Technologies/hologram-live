// Ship pinned tensor payloads from a Kubo node you control to Filebase, where they stay on IPFS without it.
// Each batch is one UnixFS directory whose entries are named by κ (sha256 hex) and link the tensor's own CID, so the
// directory is a self-verifying κ -> CID map. It goes up as a CAR with Filebase's CAR import, and is refused unless
// the CID Filebase reports is our root and tensors read back through Filebase's gateway hash to their κ.
// Payloads not in --keep are then unpinned locally, so a small disk moves any number of models through.
//
//   TENSOR_STATE=state IPFS_API=http://127.0.0.1:5101 FB_ENV=/root/hub/filebase.env STAGE=/root/hub/fb-stage \
//     node filebase.mjs ship [--keep top10.txt] [--batch-gb 4]
//   node filebase.mjs map          publish the whole κ -> CID map (one JSON object) the same way
import { readFileSync, writeFileSync, renameSync, createWriteStream, mkdirSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

const STATE = process.env.TENSOR_STATE || "state";
const API = (process.env.IPFS_API || "http://127.0.0.1:5101").replace(/\/$/, "");
const STAGE = process.env.STAGE || "/root/hub/fb-stage";
const BUCKET = process.env.FB_BUCKET || "hologram-model-hub";
const GW = (process.env.FB_GATEWAY || "https://ipfs.filebase.io").replace(/\/$/, "");
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const log = (s) => console.log(`${new Date().toISOString().slice(11, 19)} ${s}`);
const readJ = (f, d) => { try { return JSON.parse(readFileSync(join(STATE, f), "utf8")); } catch { return d; } };
const writeJ = (f, v) => { const p = join(STATE, f); writeFileSync(p + ".tmp", JSON.stringify(v)); renameSync(p + ".tmp", p); };
const rpc = async (path, q = []) => {
  const u = `${API}/api/v0/${path}?` + q.map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");
  const r = await fetch(u, { method: "POST" });
  if (!r.ok) throw new Error(`${path}: ${r.status} ${(await r.text()).slice(0, 300)}`);
  return r;
};
const hashOf = async (dir) => (await (await rpc("files/stat", [["arg", dir], ["hash", "true"]])).json()).Hash;

// Filebase credentials are read from FB_ENV by the shell inside the rclone call, never by this process or its logs.
const rclone = (...a) => {
  const env = process.env.FB_ENV || "/root/hub/filebase.env";
  const sh = `set -a; . "${env}"; set +a; exec docker run --rm --memory 256m -v "${STAGE}:/a" -e RCLONE_CONFIG_FB_TYPE=s3 -e RCLONE_CONFIG_FB_PROVIDER=Other ` +
    `-e RCLONE_CONFIG_FB_ENDPOINT=https://s3.filebase.io -e RCLONE_CONFIG_FB_REGION=auto -e RCLONE_CONFIG_FB_ACCESS_KEY_ID="$FILEBASE_KEY" ` +
    `-e RCLONE_CONFIG_FB_SECRET_ACCESS_KEY="$FILEBASE_SECRET" rclone/rclone:1.71 "$@"`;
  const r = spawnSync("sh", ["-c", sh, "sh", ...a], { encoding: "utf8", maxBuffer: 64 << 20 });
  if (r.status !== 0) throw new Error(`rclone ${a[0]}: ${(r.stderr || r.stdout).split("\n").filter((l) => !/NOTICE/.test(l)).join(" ").slice(0, 400)}`);
  return r.stdout;
};
const filebaseCid = (key) => (rclone("lsjson", "-M", `fb:${BUCKET}/${key}`).match(/"cid"\s*:\s*"([^"]+)"/) || [])[1];

// Put one MFS directory up as a CAR under key; returns the root once Filebase reports the same CID.
async function upload(dir, key) {
  const root = await hashOf(dir);
  mkdirSync(STAGE, { recursive: true });
  const car = join(STAGE, `${root}.car`);
  await pipeline(Readable.fromWeb((await rpc("dag/export", [["arg", root]])).body), createWriteStream(car));
  const size = statSync(car).size;
  try {
    rclone("copyto", `/a/${root}.car`, `fb:${BUCKET}/${key}`, "--header-upload", "x-amz-meta-import: car", "-q",
      "--s3-chunk-size", "64M", "--s3-upload-concurrency", "2");
  } finally { rmSync(car, { force: true }); }
  const got = filebaseCid(key);
  if (got !== root) throw new Error(`refused: Filebase reports ${got || "no CID"} for ${key}, our root is ${root}`);
  return { root, size };
}

async function readBack(root, hex, len) {
  const r = await fetch(`${GW}/ipfs/${root}/${hex}`, { signal: AbortSignal.timeout(600_000) });
  if (!r.ok) throw new Error(`gateway ${r.status} for ${root}/${hex}`);
  const h = createHash("sha256"); let n = 0;
  for await (const c of r.body) { h.update(c); n += c.length; }
  if (h.digest("hex") !== hex || n !== len) throw new Error(`gateway bytes for ${hex} do not hash to it`);
}

async function ship() {
  const pins = readJ("pins.json", {});
  const budget = Number(arg("--batch-gb", "4")) * 1e9, max = Number(arg("--max-batches", "Infinity"));
  const keep = new Set(); const kl = arg("--keep");
  if (kl) for (const repo of readFileSync(kl, "utf8").split(/\r?\n/).map((l) => l.replace(/#.*/, "").trim()).filter(Boolean)) {
    try { for (const row of JSON.parse(readFileSync(join(STATE, "sources", `${repo.replace("/", "__")}.json`), "utf8"))) keep.add(row[0]); } catch {}
  }
  const pending = Object.entries(pins).filter(([, v]) => !v.fb).sort((a, b) => a[1].len - b[1].len);
  if (!pending.length) { log("nothing to ship"); return 0; }
  const batches = []; let cur = [], sz = 0;
  for (const e of pending) { if (cur.length && sz + e[1].len > budget) { batches.push(cur); cur = []; sz = 0; } cur.push(e); sz += e[1].len; }
  if (cur.length) batches.push(cur);
  let done = 0;
  for (const b of batches.slice(0, max)) {
    const dir = "/fb-batch", t0 = Date.now();
    await rpc("files/rm", [["arg", dir], ["recursive", "true"], ["force", "true"]]).catch(() => {});
    await rpc("files/mkdir", [["arg", dir], ["parents", "true"], ["cid-version", "1"]]);
    for (const [k, v] of b) await rpc("files/cp", [["arg", `/ipfs/${v.cid}`], ["arg", `${dir}/${k.slice(7)}`]]);
    const bytes = b.reduce((a, [, v]) => a + v.len, 0);
    const { root, size } = await upload(dir, `tensors/${await hashOf(dir)}.car`);
    // read back the smallest payload and the largest one of at most 64 MB, through Filebase alone
    const small = b[0], big = [...b].reverse().find(([, v]) => v.len <= 64 << 20) || b[0];
    for (const [k, v] of new Map([small, big])) await readBack(root, k.slice(7), v.len);
    const at = new Date().toISOString();
    for (const [k] of b) { pins[k].fb = root; pins[k].fbAt = at; }
    writeJ("pins.json", pins);
    await rpc("files/rm", [["arg", dir], ["recursive", "true"], ["force", "true"]]);
    const drop = b.filter(([k]) => !keep.has(k)).map(([, v]) => v.cid);
    for (let i = 0; i < drop.length; i += 200) await rpc("pin/rm", drop.slice(i, i + 200).map((c) => ["arg", c])).catch((e) => log(`unpin: ${e.message}`));
    if (drop.length) await (await rpc("repo/gc", [["quiet", "true"]])).text();
    done++;
    log(`shipped ${b.length} tensors, ${(bytes / 1e9).toFixed(2)} GB (CAR ${(size / 1e9).toFixed(2)} GB) as ${root} in ${((Date.now() - t0) / 1000).toFixed(0)} s; read back via ${GW}; ${drop.length} unpinned locally`);
  }
  return done;
}

// The κ -> CID map for every payload on Filebase, as one JSON object, so a client holding only a manifest can find
// the CID of a tensor over 1 MiB (one of at most 1 MiB is its raw CID and needs no map).
async function map() {
  const pins = readJ("pins.json", {});
  const m = {}; for (const [k, v] of Object.entries(pins).sort()) if (v.fb) m[k.slice(7)] = v.cid;
  const body = JSON.stringify(m);
  const fd = new FormData(); fd.append("file", new Blob([body]), "tensor-cids.json");
  const r = await fetch(`${API}/api/v0/add?cid-version=1&raw-leaves=true&chunker=size-1048576&pin=true&wrap-with-directory=true&quieter=true`, { method: "POST", body: fd });
  if (!r.ok) throw new Error(`add: ${r.status}`);
  const dirCid = JSON.parse((await r.text()).trim().split("\n").pop()).Hash;
  await rpc("files/rm", [["arg", "/fb-map"], ["recursive", "true"], ["force", "true"]]).catch(() => {});
  await rpc("files/cp", [["arg", `/ipfs/${dirCid}`], ["arg", "/fb-map"]]);
  const day = new Date().toISOString().slice(0, 10);
  const { root } = await upload("/fb-map", `index/tensor-cids-${day}.car`);
  await rpc("files/rm", [["arg", "/fb-map"], ["recursive", "true"], ["force", "true"]]);
  const sha = createHash("sha256").update(body).digest("hex");
  writeJ("filebase.json", { day, map: `${root}/tensor-cids.json`, sha256: sha, tensors: Object.keys(m).length, gateway: GW });
  log(`map of ${Object.keys(m).length} tensors on Filebase: ${GW}/ipfs/${root}/tensor-cids.json (sha256 ${sha})`);
}

// An already built CAR (the day index: every manifest, config and layout), refused unless Filebase reports its root.
async function put(file, key, root) {
  mkdirSync(STAGE, { recursive: true });
  const name = `${root}.car`; spawnSync("cp", [file, join(STAGE, name)]);
  try { rclone("copyto", `/a/${name}`, `fb:${BUCKET}/${key}`, "--header-upload", "x-amz-meta-import: car", "-q", "--s3-chunk-size", "64M"); }
  finally { rmSync(join(STAGE, name), { force: true }); }
  const got = filebaseCid(key);
  if (got !== root) throw new Error(`refused: Filebase reports ${got || "no CID"} for ${key}, the CAR root is ${root}`);
  log(`${key} on Filebase as ${root}`);
}

const cmd = process.argv[2];
if (cmd === "ship") await ship();
else if (cmd === "map") await map();
else if (cmd === "put") await put(process.argv[3], process.argv[4], process.argv[5]);
else { console.error("usage: node filebase.mjs ship [--keep list] [--batch-gb N] [--max-batches N] | map | put <car> <key> <root>"); process.exit(1); }
