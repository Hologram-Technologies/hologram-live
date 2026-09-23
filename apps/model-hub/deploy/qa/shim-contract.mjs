// Run hub-resolve.mjs locally against real published data and assert the contracts it is supposed to keep.
//
// The shim is the busiest thing on the endpoint and it has no tests. These are the ones that matter for the
// changes of 2026-09-23: a source pin is a constraint rather than a hint, the documented query constraints are
// enforced, and the MCP file list truncates without dropping the files a caller cannot proceed without.
//
//   node deploy/qa/shim-contract.mjs            fetches fixtures from the live site on first run, then offline
//   FIXTURES=/path node deploy/qa/shim-contract.mjs
//
// Nothing here touches the live host: the shim runs on a loopback port against a directory of fixtures.
import { spawn } from "node:child_process";
import { mkdir, writeFile, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SHIM = join(HERE, "..", "hub-resolve.mjs");
const FIX = process.env.FIXTURES || join(HERE, "fixtures");
const PORT = 8390;
const LIVE = "https://hub.uor.foundation";

// A spread of real models: one held by several sources, one held by Hugging Face alone, one with enough files to
// force the MCP truncation.
const MODELS = ["sentence-transformers/all-MiniLM-L6-v2", "Qwen/Qwen3-0.6B", "internlm/Intern-S2-397B"];

async function fixtures() {
  await mkdir(join(FIX, "files"), { recursive: true });
  await mkdir(join(FIX, "state"), { recursive: true });
  for (const id of MODELS) {
    const [org, name] = id.split("/");
    const out = join(FIX, "files", org, `${name}.json`);
    if (existsSync(out)) continue;
    await mkdir(join(FIX, "files", org), { recursive: true });
    const r = await fetch(`${LIVE}/data/files/${id}.json`);
    if (!r.ok) { console.log(`  (no published file doc for ${id}: ${r.status})`); continue; }
    await writeFile(out, await r.text());
    console.log(`  fetched ${id}`);
  }
  const models = join(FIX, "models.json");
  if (!existsSync(models)) { const r = await fetch(`${LIVE}/data/models.json`); await writeFile(models, await r.text()); console.log("  fetched models.json"); }

  // Two synthetic models, because the published data happens not to contain either shape: every real file doc
  // lists all three sources, and none has more than 200 files. Both are derived from a real one so the schema
  // cannot drift away from what the shim actually reads.
  const base = JSON.parse(await readFile(join(FIX, "files", ...MODELS[0].split("/")) + ".json", "utf8"));
  await mkdir(join(FIX, "files", "fixture"), { recursive: true });

  const hfOnly = { ...base, sources: base.sources.filter((x) => x.kind === "huggingface.co") };
  await writeFile(join(FIX, "files", "fixture", "hf-only.json"), JSON.stringify(hfOnly));

  const many = { ...base, files: [] };
  // 240 weight shards, then the small files a caller cannot proceed without, last in alphabetical order.
  for (let i = 0; i < 240; i++) many.files.push([`weights/shard-${String(i).padStart(3, "0")}.safetensors`, 1_000_000 + i, `sha256:${String(i).padStart(64, "0")}`, 1, `https://example.invalid/shard${i}`]);
  for (const [n, size] of [["config.json", 612], ["tokenizer.json", 1_200_000], ["tokenizer_config.json", 1400], ["vocab.json", 800_000], ["zzz-last.json", 10]])
    many.files.push([n, size, `sha256:${n.padEnd(64, "f").slice(0, 64).replace(/[^0-9a-f]/g, "a")}`, 0, `https://example.invalid/${n}`]);
  await writeFile(join(FIX, "files", "fixture", "many-files.json"), JSON.stringify(many));
  console.log(`  synthesised fixture/hf-only (1 source) and fixture/many-files (${many.files.length} files)`);
}

const pass = [], fail = [];
const ok = (n, d = "") => { pass.push(n); console.log(`ok   ${n.padEnd(46)} ${d}`); };
const bad = (n, d) => { fail.push(`${n} — ${d}`); console.log(`FAIL ${n.padEnd(46)} ${d}`); };

const get = (path, init) => fetch(`http://127.0.0.1:${PORT}${path}`, { redirect: "manual", signal: AbortSignal.timeout(15_000), ...init });
const rpc = (name, args) => get("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }) });
const tool = async (name, args) => { const r = await rpc(name, args); const j = await r.json(); try { return JSON.parse(j.result.content[0].text); } catch { return { isError: true, text: j.result?.content?.[0]?.text }; } };

async function main() {
  console.log("# fixtures");
  await fixtures();

  const child = spawn(process.execPath, [SHIM], {
    env: { ...process.env, HUB_DATA: FIX, HUB_STATE: join(FIX, "state"), PORT: String(PORT), HUB: `http://127.0.0.1:${PORT}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stderr.on("data", (d) => process.stderr.write(`  shim: ${d}`));
  await new Promise((r) => { child.stdout.on("data", (d) => { if (String(d).includes("listening")) r(); }); setTimeout(r, 3000); });
  console.log(`\n# shim on :${PORT}, data ${FIX}\n`);

  try {
    const M = MODELS[0];
    const doc = JSON.parse(await readFile(join(FIX, "files", ...M.split("/")) + ".json", "utf8"));
    const kinds = doc.sources.filter((s) => !s.p2p && !s.pull).map((s) => s.kind);
    console.log(`# ${M} is held by: ${kinds.join(", ")}\n`);

    // ---- a pin is a constraint
    for (const kind of kinds) {
      const token = kind === "huggingface.co" ? "huggingface" : kind === "modelscope.cn" ? "modelscope" : kind;
      const r = await get(`/via/${token}/${M}/resolve/main/config.json`, { method: "HEAD" });
      const served = r.headers.get("x-hub-source");
      if (r.status === 302 && served === kind) ok(`pin honoured: /via/${token}`, `→ ${served}`);
      else bad(`pin honoured: /via/${token}`, `status ${r.status}, served ${served}`);
    }
    // The case the change exists for: a model only Hugging Face holds, asked for through another source.
    for (const token of ["modelscope", "ipfs"]) {
      const r = await get(`/via/${token}/fixture/hf-only/resolve/main/config.json`, { method: "HEAD" });
      if (r.status === 404 && r.headers.get("x-error-code") === "SourceHasNotGotIt") ok(`pin refused when the source lacks it: /via/${token}`, "404 SourceHasNotGotIt");
      else bad(`pin refused when the source lacks it: /via/${token}`, `status ${r.status}, served ${r.headers.get("x-hub-source")} — the silent fallback is back`);
    }
    const stillWorks = await get("/via/huggingface/fixture/hf-only/resolve/main/config.json", { method: "HEAD" });
    if (stillWorks.status === 302) ok("the source it does have still resolves", `→ ${stillWorks.headers.get("x-hub-source")}`);
    else bad("the source it does have still resolves", `status ${stillWorks.status}`);
    const noPin = await get("/fixture/hf-only/resolve/main/config.json", { method: "HEAD" });
    if (noPin.status === 302) ok("no pin on a single-source model still resolves", `→ ${noPin.headers.get("x-hub-source")}`);
    else bad("no pin on a single-source model still resolves", `status ${noPin.status}`);

    const bogus = await get(`/via/bogus/${M}/resolve/main/config.json`, { method: "HEAD" });
    if (bogus.status === 404 && bogus.headers.get("x-error-code") === "UnknownSource") ok("unknown source refused", "404 UnknownSource");
    else bad("unknown source refused", `status ${bogus.status}, served ${bogus.headers.get("x-hub-source")}`);

    const plain = await get(`/${M}/resolve/main/config.json`, { method: "HEAD" });
    if (plain.status === 302) ok("no pin still resolves", `→ ${plain.headers.get("x-hub-source")}`);
    else bad("no pin still resolves", `status ${plain.status}`);

    // ---- documented query constraints
    for (const [q, why] of [["limit=0", "below the documented minimum"], ["limit=501", "above the documented maximum"],
      ["limit=-5", "negative"], ["limit=abc", "not a number"], ["sort=bogus", "outside the enum"]]) {
      const r = await get(`/api/models?${q}`);
      const code = r.headers.get("x-error-code");
      if (r.status === 400 && code === "BadParameter") ok(`rejects ${q}`, why);
      else bad(`rejects ${q}`, `status ${r.status} (${why}) — silently defaulted`);
    }
    for (const q of ["limit=1", "limit=500", "sort=downloads", "sort=likes&direction=1", ""]) {
      const r = await get(`/api/models${q ? "?" + q : ""}`);
      if (r.status === 200) ok(`accepts ${q || "(no parameters)"}`, `${(await r.json()).length} rows`);
      else bad(`accepts ${q || "(no parameters)"}`, `status ${r.status}`);
    }

    // ---- MCP truncation keeps what a caller cannot proceed without
    const got = await tool("get_model", { id: "fixture/many-files" });
    if (!got.files_truncated) bad("a 245-file model truncates", `files_truncated ${got.files_truncated}, ${got.files.length} files`);
    else {
      const paths = got.files.map((f) => f.path);
      const missing = ["config.json", "tokenizer.json", "tokenizer_config.json", "vocab.json"].filter((n) => !paths.includes(n));
      if (!missing.length) ok("truncation keeps config and tokenizers", `${got.files.length} of ${got.files_total}, none of the four dropped`);
      else bad("truncation keeps config and tokenizers", `dropped ${missing.join(", ")} — alphabetical truncation is back`);
      if (got.files_total === 245) ok("files_total counts the whole model", "245");
      else bad("files_total counts the whole model", String(got.files_total));
      if (got.files_truncated_note && /tree\/main/.test(got.files_truncated_note)) ok("truncation says where the full list is", "points at the tree route");
      else bad("truncation says where the full list is", got.files_truncated_note || "no note");
    }

    const small = await tool("get_model", { id: M });
    if (!small.files_truncated && small.files.length === small.files_total) ok("a small model is not truncated", `${small.files_total} files`);
    else bad("a small model is not truncated", `${small.files.length}/${small.files_total}`);

    console.log(`\n${pass.length} passed, ${fail.length} failed`);
    if (fail.length) for (const f of fail) console.log(`  ${f}`);
  } finally {
    child.kill();
  }
  process.exit(fail.length ? 1 : 0);
}

function ORDERS() { return ["huggingface.co", "modelscope.cn", "ipfs"]; }
import { readFileSync } from "node:fs";
function require$(p) { return readFileSync(p, "utf8"); }

main().catch((e) => { console.error(e); process.exit(1); });
