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
const LIVE = "https://gethologram.ai";

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

  // A GGUF quantisation repo as the address index lists one (bartowski's Q8_0, proven byte-exact from IPFS objects).
  const gguf = { ...base, files: [
    ["README.md", 9807, "sha256:09b1f05942d11f5c4f1a5b3a0e4fc5a6bd6e0a47f0c2f7f5ad2a4c7b9e1d3f01", 0, "https://example.invalid/README.md"],
    ["SmolLM2-135M-Instruct-Q8_0.gguf", 144811392, "sha256:5a1395716f7913741cc7b61ebe0d2f7ee2c1ca3ad83dd6d3f2a0a5b0f9d7c2e1", 1,
      "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/main/SmolLM2-135M-Instruct-Q8_0.gguf"]] };
  await writeFile(join(FIX, "files", "fixture", "gguf-repo.json"), JSON.stringify(gguf));
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
    env: { ...process.env, HUB_DATA: FIX, HUB_STATE: join(FIX, "state"), PORT: String(PORT), HUB: LIVE },
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

    // ---- the hub's own object plane as the backstop under every dialect
    //
    // These models are in neither the fixtures nor the third-party address index; the only place their file
    // hashes exist is the model object this hub published on an earlier day. Before the backstop they 404'd from
    // every dialect while being served perfectly well by address.
    for (const orphan of ["thesysdev/OUI-1", "Comfy-Org/marigold-v2-0"]) {
      const info = await get(`/api/models/${orphan}`);
      if (info.status !== 200) { bad(`the hub serves ${orphan}`, `getModel ${info.status}`); continue; }
      const doc = await info.json();
      const tree = await (await get(`/api/models/${orphan}/tree/main`)).json();
      const hashed = Array.isArray(tree) && tree.length && tree.every((f) => /^[0-9a-f]{64}$/.test(f.oid));
      if (hashed) ok(`the hub serves ${orphan} from its own objects`, `${tree.length} files, revision ${String(doc.sha).slice(0, 8)}`);
      else bad(`the hub serves ${orphan} from its own objects`, `tree ${JSON.stringify(tree).slice(0, 60)}`);

      const big = tree[0], want256 = big.oid;
      const red = await get(`/${orphan}/resolve/main/${big.path}`, { method: "HEAD" });
      if (red.status === 302 && (red.headers.get("etag") || "").includes(want256)) ok(`  and redirects with the index hash`, `→ ${red.headers.get("x-hub-source")}`);
      else bad(`  and redirects with the index hash`, `status ${red.status}, etag ${red.headers.get("etag")}`);

      const oci = await get(`/v2/${orphan.toLowerCase()}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } });
      if (oci.status === 200) ok(`  and has an OCI manifest`, `${(await oci.json()).layers.length} layers`);
      else bad(`  and has an OCI manifest`, `status ${oci.status}`);
    }

    // ---- and search can find them
    const listed = await (await get("/api/models?limit=500")).json();
    const total = (await get("/api/models?limit=1")).then ? null : null;
    const head = await get("/api/models?limit=1");
    const count = Number(head.headers.get("x-total-count") || 0);
    if (count > 500) ok("search covers more than the browse list", `${count} rows`);
    else bad("search covers more than the browse list", `${count} rows — the backstop rows are not in the list`);
    const found = await (await get("/api/models?search=OUI-1&limit=20")).json();
    if (found.some((m) => m.id === "thesysdev/OUI-1")) ok("search finds a model the browse list forgot", "thesysdev/OUI-1");
    else bad("search finds a model the browse list forgot", `${found.length} rows, none matching`);
    const thin = found.find((m) => m.id === "thesysdev/OUI-1");
    if (thin && thin.hologram && thin.hologram.listed === false) ok("a thin row says it is thin", "hologram.listed false");
    else if (thin) bad("a thin row says it is thin", `hologram ${JSON.stringify(thin.hologram)}`);

    // ---- the trust root: model info names the public canonical manifest every digest comes from
    const tr = (await (await get(`/api/models/${M}`)).json()).hologram;
    try {
      const man = await (await fetch(tr.manifest_url, { signal: AbortSignal.timeout(20_000) })).json();
      const same = man.files.length === doc.files.length && man.files.every((f) => doc.files.some((d) => d[0] === f.path && d[1] === f.size && d[2] === f.address));
      if (same && man.revision === doc.revision) ok("model info names the public manifest; its files are the tree's", tr.manifest.slice(0, 20));
      else bad("model info names the public manifest; its files are the tree's", `${man.files.length} files, revision ${man.revision}`);
    } catch (e) { bad("model info names the public manifest", `${tr?.manifest_url}: ${e.message}`); }

    // ---- ModelScope's dialect (MODELSCOPE_ENDPOINT): the SDK hashes every file against Sha256, on every cache hit too
    const msf = await (await get(`/api/v1/models/${M}/repo/files?Revision=master&Recursive=True`)).json();
    const blobsMs = (msf.Data?.Files || []).filter((f) => f.Type === "blob");
    const sameMs = blobsMs.length === doc.files.length && blobsMs.every((f) => doc.files.some((d) => d[0] === f.Path && d[1] === f.Size && d[2] === `sha256:${f.Sha256}`));
    if (msf.Code === 200 && sameMs) ok("ModelScope file list: every Sha256 is the index's", `${blobsMs.length} files`);
    else bad("ModelScope file list: every Sha256 is the index's", JSON.stringify(msf).slice(0, 100));
    const msget = await get(`/api/v1/models/${M}/repo?Revision=master&FilePath=config.json`);
    if (msget.status === 302 && msget.headers.get("location")) ok("ModelScope file download redirects to a holder", msget.headers.get("x-hub-source"));
    else bad("ModelScope file download redirects to a holder", String(msget.status));
    const acc = await (await get("/api/v1/repos/internalAccelerationInfo")).json();
    const revs = await (await get(`/api/v1/models/${M}/revisions`)).json();
    if (acc.Code === 200 && revs.Data?.RevisionMap?.Branches?.[0]?.Revision === "master") ok("ModelScope side calls answered", "acceleration info, revisions");
    else bad("ModelScope side calls answered", JSON.stringify([acc, revs]).slice(0, 100));
    const msno = await get("/api/v1/models/nobody/not-a-model/repo/files");
    const msnoj = await msno.json();
    if (msno.status === 404 && msnoj.Success === false) ok("ModelScope: unknown model is its 404", msnoj.Message.slice(0, 40));
    else bad("ModelScope: unknown model is its 404", String(msno.status));

    // ---- llama.cpp before b8498 reads the GGUF file name from Hugging Face's `ggufFile` manifest extension
    const old = await get("/v2/FIXTURE/GGUF-REPO/manifests/q8_0", { headers: { "user-agent": "llama-cpp/b8400-cf23ee244", accept: "application/json" } });
    const om = old.status === 200 ? await old.json() : {};
    if (om.ggufFile?.rfilename === "SmolLM2-135M-Instruct-Q8_0.gguf" && om.ggufFile.lfs?.sha256?.startsWith("5a1395716f79") && om.layers?.length)
      ok("old llama.cpp gets ggufFile (any case)", om.ggufFile.rfilename);
    else bad("old llama.cpp gets ggufFile (any case)", `status ${old.status} ${JSON.stringify(om).slice(0, 80)}`);
    const oll = await get("/v2/fixture/gguf-repo/manifests/Q8_0", { headers: { "user-agent": "ollama/0.12.0" } });
    const olm = oll.status === 200 ? await oll.json() : {};
    if (!olm.ggufFile && olm.layers?.some((l) => l.digest.startsWith("sha256:5a1395716f79"))) ok("Ollama's manifest stays Ollama's", "no ggufFile, model layer present");
    else bad("Ollama's manifest stays Ollama's", `status ${oll.status}`);

    const small = await tool("get_model", { id: M });
    if (!small.files_truncated && small.files.length === small.files_total) ok("a small model is not truncated", `${small.files_total} files`);
    else bad("a small model is not truncated", `${small.files.length}/${small.files_total}`);

    // ---- the Hugging Face listing, as the clients that page and walk it expect (MODEL-CLIENTS-GAP-CLOSURE G1, G2)
    const all = await (await get(`/api/models/${M}/tree/main?recursive=true&expand=false`)).json();
    const files = all.filter((e) => e.type === "file"), folders = all.filter((e) => e.type === "directory");
    const nested = files.find((e) => e.path.includes("/"));
    if (files.length === doc.files.length) ok("recursive tree lists every file", `${files.length} files, ${folders.length} folders`);
    else bad("recursive tree lists every file", `${files.length} of ${doc.files.length}`);
    if (!nested || folders.some((d) => nested.path.startsWith(`${d.path}/`))) ok("recursive tree lists the folders too", nested ? nested.path.split("/")[0] : "(no folders)");
    else bad("recursive tree lists the folders too", `no folder entry for ${nested.path}`);
    const top = await (await get(`/api/models/${M}/tree/main?recursive=False`)).json();
    if (top.every((e) => !e.path.includes("/")) && top.some((e) => e.type === "directory")) ok("recursive=False lists direct children and folders", `${top.length} entries`);
    else bad("recursive=False lists direct children and folders", `${top.length} entries — HfFileSystem.ls would misread it`);
    const plainTree = await (await get(`/api/models/${M}/tree/main`)).json();
    if (plainTree.length === doc.files.length && plainTree.every((e) => e.type === "file")) ok("no ?recursive: every file, files only (documented)", `${plainTree.length} files`);
    else bad("no ?recursive: every file, files only (documented)", `${plainTree.length} entries, types ${[...new Set(plainTree.map((e) => e.type))]}`);
    if (nested) {
      const d = nested.path.split("/")[0], sub = await (await get(`/api/models/${M}/tree/main/${d}?recursive=false`)).json();
      if (sub.length && sub.every((e) => e.path.startsWith(`${d}/`))) ok("tree of a folder lists that folder", `${d}: ${sub.length}`);
      else bad("tree of a folder lists that folder", JSON.stringify(sub).slice(0, 80));
    }
    const page2 = await get(`/api/models/${M}/tree/main?recursive=true&cursor=abc`);
    const p2 = await page2.json();
    if (page2.status === 200 && Array.isArray(p2) && !p2.length) ok("any next page is empty", "text-generation-webui stops");
    else bad("any next page is empty", `${page2.status}, ${p2.length} entries — a cursor loop never ends`);
    const nowhere = await get(`/api/models/${M}/tree/main/no-such-folder`);
    if (nowhere.status === 404 && nowhere.headers.get("x-error-code") === "EntryNotFound") ok("a missing folder is EntryNotFound", "404");
    else bad("a missing folder is EntryNotFound", `status ${nowhere.status}, ${nowhere.headers.get("x-error-code")}`);
    const size = await (await get(`/api/models/${M}/treesize/main`)).json();
    const want = doc.files.reduce((s, f) => s + f[1], 0);
    if (size.size === want) ok("treesize is the sum of the file sizes", String(want));
    else bad("treesize is the sum of the file sizes", `${size.size} vs ${want}`);
    const meta = await (await get(`/api/models/${M}?blobs=true`)).json();
    const lfs = meta.siblings.filter((s) => s.lfs);
    if (meta.siblings.every((s) => Number.isInteger(s.size)) && lfs.length === meta.siblings.length && lfs.every((s) => /^[0-9a-f]{64}$/.test(s.lfs.sha256)))
      ok("?blobs=true siblings carry size and lfs.sha256", `${lfs.length} LFS of ${meta.siblings.length}`);
    else bad("?blobs=true siblings carry size and lfs.sha256", JSON.stringify(meta.siblings[0]));
    const w = lfs[0] && doc.files.find((f) => f[0] === lfs[0].rfilename);
    if (w && w[2] === `sha256:${lfs[0].lfs.sha256}`) ok("  the sha256 is the index's", w[0]);
    else bad("  the sha256 is the index's", lfs[0]?.rfilename);
    // Every file: oid is the index's sha256 (the documented contract Spaces and agents check against), and `lfs`
    // carries it too, which is what makes `hf cache verify` check small files by sha256 instead of git's sha1.
    const wrong = files.filter((e) => { const d = doc.files.find((f) => f[0] === e.path); return !d || e.oid !== d[2].slice(7) || e.lfs?.oid !== e.oid; });
    if (!wrong.length) ok("every file: oid and lfs.oid are the index's sha256", `${files.length} files`);
    else bad("every file: oid and lfs.oid are the index's sha256", JSON.stringify(wrong[0]));
    const plainInfo = await (await get(`/api/models/${M}`)).json();
    if (plainInfo.siblings.every((s) => Object.keys(s).length === 1)) ok("without blobs, siblings stay names only", "as Hugging Face");
    else bad("without blobs, siblings stay names only", JSON.stringify(plainInfo.siblings[0]));

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
