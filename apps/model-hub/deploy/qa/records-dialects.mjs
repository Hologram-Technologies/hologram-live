// hub-resolve's HF, ModelScope, Git LFS, OCI and Ollama answers come only from the Registry's records when the tensor
// index has the model, even when the address index says otherwise.
//
//   RECORDS=http://127.0.0.1:18976 MODEL=HuggingFaceTB/SmolLM2-135M-Instruct node deploy/qa/records-dialects.mjs
//
// RECORDS is a tensor mirror serving /v2/models/<repo>/records (tensors/serve.mjs over a state that indexed MODEL).
// The address index given to hub-resolve is deliberately wrong about config.json; the answers must carry the
// record's digest, never the index's. Every 64-hex digest in every answer (bodies and headers) must be a record,
// except the digests of the OCI manifest and config hub-resolve generates itself (a view names itself).
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const RECORDS = process.env.RECORDS, M = process.env.MODEL || "HuggingFaceTB/SmolLM2-135M-Instruct", PORT = 8396;
if (!RECORDS) { console.error("RECORDS is required"); process.exit(2); }
const recs = await (await fetch(`${RECORDS}/v2/models/${M.toLowerCase()}/records`)).json();
const known = new Set(recs.records.map((r) => r.kappa.slice(7)));
// the repository's real file list, independent of the records: the original format's manifest
const orig = await (await fetch(`${RECORDS}/v2/models/${M.toLowerCase()}/manifests/original`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } })).json();
const files = orig.layers.map((l) => ({ path: l.annotations["org.opencontainers.image.title"], size: l.size, kappa: l.digest, held: recs.records.find((r) => r.kappa === l.digest)?.holders.some((h) => h.kind === "hub") }));
const cfg = files.find((f) => f.path === "config.json");
const WRONG = "e".repeat(64);

// an address-index doc for the model that is wrong about config.json
const data = await mkdtemp(join(tmpdir(), "records-dialects-"));
const [org, name] = M.split("/");
await mkdir(join(data, "files", org), { recursive: true });
await writeFile(join(data, "files", org, `${name}.json`), JSON.stringify({ revision: recs.revision, manifest: `blake3:${"1".repeat(64)}`,
  sources: [{ kind: "huggingface.co", resolve: null, missing: [] }],
  files: files.map((f) => [f.path, f.size, f.path === "config.json" ? `sha256:${WRONG}` : f.kappa, f.held ? 0 : 1, `https://huggingface.co/${M}/resolve/${recs.revision}/${f.path}`]) }));

const shim = spawn(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), "..", "hub-resolve.mjs")], {
  env: { ...process.env, HUB_DATA: data, HUB_STATE: join(data, "state"), PORT: String(PORT), HUB: "http://127.0.0.1:9", TENSOR_RECORDS: RECORDS }, stdio: ["ignore", "pipe", "pipe"] });
const logs = []; shim.stdout.on("data", (d) => logs.push(String(d)));
await new Promise((r) => { shim.stdout.on("data", (d) => String(d).includes("listening") && r()); setTimeout(r, 3000); });

const B = `http://127.0.0.1:${PORT}`, views = new Set();
let fails = 0, digests = 0, answers = 0;
const bad = (m) => { fails++; console.log(`FAIL ${m}`); };
const see = async (what, path, init = {}) => {
  const r = await fetch(B + path, { redirect: "manual", ...init });
  const body = init.method === "HEAD" ? "" : await r.text();
  const text = body + "\n" + [...r.headers].map(([k, v]) => `${k}: ${v}`).join("\n");
  answers++;
  for (const d of new Set(text.match(/(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])/g) || [])) {
    digests++;
    if (d === WRONG) bad(`${what}: carries the index's wrong digest for config.json`);
    else if (!known.has(d) && !views.has(d)) bad(`${what}: ${d.slice(0, 16)}… is not a record`);
  }
  return { r, body };
};
try {
  const lc = M.toLowerCase();
  await see("model info", `/api/models/${M}`);
  await see("model info ?blobs", `/api/models/${M}?blobs=true`);
  const tree = JSON.parse((await see("tree", `/api/models/${M}/tree/main`)).body);
  await see("tree recursive", `/api/models/${M}/tree/main?recursive=true`);
  await see("treesize", `/api/models/${M}/treesize/main`);
  await see("refs", `/api/models/${M}/refs`);
  const sums = await fetch(`${B}/${M}/resolve/main/SHA256SUMS`);          // generated: its ETag is its own text's hash (a view)
  views.add((sums.headers.get("etag") || "").replace(/"/g, "")); await sums.text();
  await see("SHA256SUMS", `/${M}/resolve/main/SHA256SUMS`);
  for (const f of files) await see(`HEAD ${f.path}`, `/${M}/resolve/main/${f.path}`, { method: "HEAD" });
  await see("ModelScope files", `/api/v1/models/${M}/repo/files?Revision=master&Recursive=True`);
  await see("Git LFS batch", `/${M}.git/info/lfs/objects/batch`, { method: "POST", headers: { "content-type": "application/vnd.git-lfs+json" }, body: JSON.stringify({ operation: "download", objects: files.map((f) => ({ oid: f.kappa.slice(7), size: f.size })) }) });
  // the OCI manifest hub-resolve generates: its own digest and its config's are views, everything else must be a record
  const oci = await fetch(`${B}/v2/${lc}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } });
  const ociBody = await oci.text();
  views.add(oci.headers.get("docker-content-digest")?.slice(7)); views.add(JSON.parse(ociBody).config?.digest?.slice(7));
  await see("OCI manifest", `/v2/${lc}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } });
  await see("Ollama tags", `/v2/${lc}/tags/list`);
  // the listing is the repository's, file for file: no render-only name, no original name lost
  const want = new Set(files.map((f) => f.path)), got = new Set(tree.map((e) => e.path));
  const lost = [...want].filter((p) => !got.has(p)), extra = [...got].filter((p) => !want.has(p));
  if (!lost.length && !extra.length) console.log(`ok   the listing is the repository's own ${want.size} files`); else bad(`listing differs: lost ${lost.join(",")} extra ${extra.join(",")}`);
  const tcfg = tree.find((e) => e.path === "config.json");
  if (tcfg?.oid === cfg.kappa.slice(7)) console.log(`ok   config.json answers the record's ${tcfg.oid.slice(0, 12)}…, not the index's wrong digest`); else bad(`config.json answers ${tcfg?.oid}`);
  if (logs.join("").includes('"used":"record"')) console.log("ok   the disagreement is logged"); else bad("the disagreement was not logged");
  // and without records, the address index as before
  const plain = spawn(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), "..", "hub-resolve.mjs")], { env: { ...process.env, HUB_DATA: data, HUB_STATE: join(data, "state"), PORT: String(PORT + 1), HUB: "http://127.0.0.1:9", TENSOR_RECORDS: "" }, stdio: ["ignore", "pipe", "pipe"] });
  await new Promise((r) => { plain.stdout.on("data", (d) => String(d).includes("listening") && r()); setTimeout(r, 3000); });
  const t2 = await (await fetch(`http://127.0.0.1:${PORT + 1}/api/models/${M}/tree/main`)).json();
  plain.kill();
  if (t2.find((e) => e.path === "config.json")?.oid === WRONG) console.log("ok   without records, the address index as before"); else bad("without records the answer changed");
} finally { shim.kill(); }
console.log(`${answers} answers, ${digests} digests, ${fails} failures (records: ${known.size}, views: ${views.size})`);
process.exit(fails ? 1 : 0);
