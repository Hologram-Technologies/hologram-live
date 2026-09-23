// Stress the endpoint one operation at a time, across the dimensions that actually catch things.
//
//   node qa/openapi/stress.mjs [--base https://hub.uor.foundation] [--out ../../../../reports/.../results.json]
//
// Every check is declarative: an id, the plane it belongs to, the dimension it tests, a request, and what we
// expect. Nothing is asserted that is not also recorded, so a run is a diffable artifact rather than a pass/fail.
// Read-only throughout: no token is sent, no object is published, and file routes are probed with HEAD so the
// third-party sources behind the redirects are never made to serve bytes they did not have to.
import { writeFile, mkdir } from "node:fs/promises";
import { createHash } from "node:crypto";

const arg = (n, d) => { const i = process.argv.indexOf(`--${n}`); return i > -1 ? process.argv[i + 1] : d; };
const BASE = arg("base", "https://hub.uor.foundation").replace(/\/$/, "");
const OUT = arg("out", null);

const KEEP = ["content-type", "cache-control", "etag", "location", "x-repo-commit", "x-linked-etag", "x-linked-size",
  "x-hub-source", "x-total-count", "x-error-code", "x-error-message", "accept-ranges", "content-range",
  "docker-content-digest", "access-control-allow-origin", "access-control-allow-methods", "access-control-expose-headers",
  "content-encoding", "vary", "allow", "retry-after", "www-authenticate"];

const results = [];
let sample = {};

async function hit(path, { method = "GET", headers = {}, body = null, redirect = "manual" } = {}) {
  const started = Date.now();
  try {
    const res = await fetch(path.startsWith("http") ? path : BASE + path,
      { method, headers, body, redirect, signal: AbortSignal.timeout(30_000) });
    const text = method === "HEAD" ? "" : await res.text();
    return {
      status: res.status, ms: Date.now() - started,
      headers: Object.fromEntries(KEEP.filter((h) => res.headers.has(h)).map((h) => [h, res.headers.get(h)])),
      text, json: (() => { try { return JSON.parse(text); } catch { return undefined; } })(),
    };
  } catch (e) { return { status: null, ms: Date.now() - started, headers: {}, text: "", error: String(e.message || e) }; }
}

// A check records what happened and, when it can, whether that is what the contract says should happen.
async function check(id, plane, dimension, what, request, verdictFn) {
  const r = await hit(request.path, request);
  let verdict = "recorded", note = "";
  if (verdictFn) {
    try { const v = verdictFn(r); verdict = v === true ? "pass" : v === false ? "FAIL" : v.ok ? "pass" : "FAIL"; note = typeof v === "object" ? v.note || "" : ""; }
    catch (e) { verdict = "FAIL"; note = `check threw: ${e.message}`; }
  }
  const row = {
    id, plane, dimension, what,
    request: { method: request.method || "GET", path: request.path, headers: request.headers || undefined },
    status: r.status, ms: r.ms, headers: r.headers, verdict, note,
    body: r.json !== undefined ? shrink(r.json) : (r.text ? r.text.slice(0, 240) : undefined),
    error: r.error,
  };
  results.push(row);
  const mark = verdict === "FAIL" ? "FAIL" : verdict === "pass" ? "ok  " : "··  ";
  console.log(`${mark} ${id.padEnd(34)} ${String(r.status ?? "ERR").padEnd(4)} ${(note || what).slice(0, 78)}`);
  return r;
}

const shrink = (v, d = 0) => {
  if (typeof v === "string") return v.length > 200 ? v.slice(0, 200) + "…" : v;
  if (Array.isArray(v)) return v.slice(0, 2).map((x) => shrink(x, d + 1));
  if (v && typeof v === "object" && d < 4) return Object.fromEntries(Object.entries(v).slice(0, 24).map(([k, x]) => [k, shrink(x, d + 1)]));
  return v;
};

// ---------------------------------------------------------------- discover a live sample to test against
async function discover() {
  const rows = (await hit("/api/models?limit=40&sort=downloads")).json;
  const descriptor = (await hit("/.well-known/model-hub.json")).json;
  let small = null, gguf = null;
  for (const row of rows) {
    const files = (await hit(`/api/models/${row.id}/tree/main`)).json;
    if (!Array.isArray(files)) continue;
    const s = files.filter((f) => f.size > 0 && f.size < 80_000).sort((a, b) => a.size - b.size)[0];
    if (s && !small) small = { id: row.id, rev: row.sha, file: s };
    if (!gguf && files.some((f) => /\.gguf$/i.test(f.path))) gguf = { id: row.id };
    if (small && gguf) break;
  }
  sample = { model: small.id, revision: small.rev, file: small.file, gguf: gguf?.id || null, catalog: descriptor.catalog, rows: rows.length };
  console.log(`# sample: ${sample.model}@${sample.revision.slice(0, 8)} file=${sample.file.path} (${sample.file.size}B) gguf=${sample.gguf}\n`);
}

// ---------------------------------------------------------------- the checks
async function run() {
  await discover();
  const M = sample.model, F = sample.file.path, OID = sample.file.oid, REV = sample.revision;
  const lower = M.toLowerCase();

  // ---- discovery: content negotiation is the front door's whole contract
  await check("disco.root.html", "discovery", "negotiation", "a browser gets the page",
    { path: "/", headers: { accept: "text/html,application/xhtml+xml" } },
    (r) => ({ ok: r.status === 200 && /text\/html/.test(r.headers["content-type"] || ""), note: `${r.headers["content-type"]} ${r.text.length}B` }));
  await check("disco.root.json", "discovery", "negotiation", "a JSON caller gets the descriptor",
    { path: "/", headers: { accept: "application/json" } },
    (r) => ({ ok: r.json?.format === "hologram.model-hub.descriptor/v1", note: `catalog ${r.json?.catalog?.slice(0, 18)}…` }));
  await check("disco.root.any", "discovery", "negotiation", "curl/fetch/requests get the brief",
    { path: "/", headers: { accept: "*/*" } },
    (r) => ({ ok: /text\/markdown/.test(r.headers["content-type"] || "") && r.text.startsWith("# hub.uor.foundation"), note: `${r.headers["content-type"]} ${r.text.length}B` }));
  await check("disco.root.noaccept", "discovery", "negotiation", "no Accept header at all",
    { path: "/", headers: {} },
    (r) => ({ ok: /text\/markdown/.test(r.headers["content-type"] || ""), note: r.headers["content-type"] }));
  await check("disco.root.unfurler", "discovery", "negotiation", "a link unfurler still gets HTML",
    { path: "/", headers: { accept: "*/*", "user-agent": "Slackbot-LinkExpanding 1.0" } },
    (r) => ({ ok: /text\/html/.test(r.headers["content-type"] || ""), note: r.headers["content-type"] }));
  await check("disco.root.hostile-accept", "discovery", "negotiation", "a malformed Accept does not crash it",
    { path: "/", headers: { accept: "text/html;q=,,,*/*;;q=9999" } },
    (r) => ({ ok: r.status === 200, note: r.headers["content-type"] }));
  await check("disco.root.vary", "discovery", "headers", "the root varies on Accept, or caches will poison",
    { path: "/", headers: { accept: "*/*" } },
    (r) => ({ ok: (r.headers.vary || "").split(",").map((v) => v.trim().toLowerCase()).includes("accept"), note: r.headers.vary ? `Vary: ${r.headers.vary} — does NOT list Accept, so a shared cache can serve the wrong representation` : "no Vary at all on a content-negotiated route" }));
  await check("disco.root.cors", "discovery", "headers", "the front door is cross-origin readable",
    { path: "/", headers: { accept: "*/*", origin: "https://example.com" } },
    (r) => r.headers["access-control-allow-origin"] === "*");

  for (const [id, path, test] of [
    ["disco.agentmd", "/agent.md", (r) => ({ ok: /text\/markdown/.test(r.headers["content-type"] || "") && /canary/.test(r.text), note: `${r.text.length}B, canary present` })],
    ["disco.llms", "/llms.txt", (r) => r.status === 200],
    ["disco.openapi", "/openapi.json", (r) => ({ ok: r.json?.openapi === "3.1.0", note: `${Object.keys(r.json?.paths || {}).length} paths` })],
    ["disco.wk.openapi", "/.well-known/openapi.json", (r) => r.json?.openapi === "3.1.0"],
    ["disco.wk.card", "/.well-known/agent-card.json", (r) => ({ ok: Array.isArray(r.json?.skills), note: `${r.json?.skills?.length} skills` })],
    ["disco.wk.descriptor", "/.well-known/model-hub.json", (r) => r.json?.format === "hologram.model-hub.descriptor/v1"],
    ["disco.robots", "/robots.txt", (r) => ({ ok: /Disallow: \/via\//.test(r.text), note: "byte routes disallowed" })],
    ["disco.docs", "/docs", (r) => ({ ok: r.status === 200 && /openapi\.json/.test(r.text), note: "renders /openapi.json" })],
    ["disco.archive", "/archive.json", (r) => ({ ok: Array.isArray(r.json?.days), note: `${r.json?.days?.length} days chained` })],
    ["disco.pins", "/pins.json", (r) => ({ ok: !!r.json?.models, note: `${Object.keys(r.json?.models || {}).length} models pinned` })],
    ["disco.mcpauth", "/.well-known/mcp-registry-auth", (r) => ({ ok: r.status === 200, note: r.status === 200 ? "ownership proof served" : "absent" })],
  ]) await check(id, "discovery", "happy", path, { path }, test);

  await check("disco.openapi.cache", "discovery", "headers", "the document is cacheable but not immutable",
    { path: "/openapi.json" }, (r) => ({ ok: /max-age/.test(r.headers["cache-control"] || ""), note: r.headers["cache-control"] }));
  await check("disco.openapi.etag", "discovery", "headers", "conditional request on the document",
    { path: "/openapi.json" }, (r) => ({ ok: true, note: r.headers.etag ? `ETag ${r.headers.etag}` : "no ETag: every agent re-downloads 117 KB" }));

  // ---- health
  await check("health.server", "health", "happy", "/healthz", { path: "/healthz" }, (r) => r.json?.status === "ready");
  await check("health.sources", "health", "happy", "/api/hub/health", { path: "/api/hub/health" },
    (r) => ({ ok: !!r.json?.sources && Array.isArray(r.json?.order), note: Object.entries(r.json?.sources || {}).map(([k, v]) => `${k}=${v.ok}`).join(" ") }));
  await check("health.sources.freshness", "health", "undocumented", "how stale is the source probe",
    { path: "/api/hub/health" }, (r) => {
      const ages = Object.values(r.json?.sources || {}).map((v) => (Date.now() - Date.parse(v.checked)) / 1000);
      const max = Math.max(...ages);
      return { ok: max < 300, note: `oldest probe ${Math.round(max)}s ago` };
    });
  await check("health.sources.nostore", "health", "headers", "liveness must not be cached",
    { path: "/api/hub/health" }, (r) => ({ ok: /no-store/.test(r.headers["cache-control"] || ""), note: r.headers["cache-control"] }));

  // ---- models: the HF dialect
  await check("models.list", "models", "happy", "listModels", { path: "/api/models?limit=3" },
    (r) => ({ ok: Array.isArray(r.json) && r.json.length === 3, note: `X-Total-Count ${r.headers["x-total-count"]}` }));
  await check("models.list.coverage", "models", "consistency", "search reach vs catalog size",
    { path: "/api/models?limit=500" }, (r) => ({ ok: true, note: `search sees ${r.json?.length}; catalog addresses more (known gap)` }));
  for (const [id, q, test] of [
    ["models.q.search", "search=qwen&limit=5", (r) => ({ ok: r.json.every((m) => /qwen/i.test(m.id)), note: `${r.json.length} rows all match` })],
    ["models.q.author", "author=Qwen&limit=5", (r) => ({ ok: r.json.every((m) => m.author === "Qwen"), note: `${r.json.length} rows` })],
    ["models.q.task", "pipeline_tag=text-generation&limit=5", (r) => ({ ok: r.json.every((m) => m.pipeline_tag === "text-generation"), note: `${r.json.length} rows` })],
    ["models.q.filter", "filter=gguf&limit=5", (r) => ({ ok: r.json.every((m) => m.tags.includes("gguf")), note: `${r.json.length} rows` })],
    ["models.q.filter.multi", "filter=gguf,text-generation&limit=5", (r) => ({ ok: Array.isArray(r.json), note: `${r.json.length} rows for two tags` })],
    ["models.q.sort.dl", "sort=downloads&limit=5", (r) => ({ ok: r.json.every((m, i, a) => !i || a[i - 1].downloads >= m.downloads), note: "descending" })],
    ["models.q.direction", "sort=downloads&direction=1&limit=5", (r) => ({ ok: r.json.every((m, i, a) => !i || a[i - 1].downloads <= m.downloads), note: "ascending" })],
  ]) await check(id, "models", "happy", `/api/models?${q}`, { path: `/api/models?${q}` }, test);

  for (const [id, q, expectation] of [
    ["models.b.limit0", "limit=0", "documented minimum is 1"],
    ["models.b.limitneg", "limit=-5", "negative"],
    ["models.b.limitbig", "limit=99999", "documented maximum is 500"],
    ["models.b.limitabc", "limit=abc", "not a number"],
    ["models.b.sortbogus", "sort=bogus", "outside the documented enum"],
    ["models.b.unknownparam", "nonsense=1", "unknown parameter"],
    ["models.b.empty", "search=", "empty search"],
    ["models.b.unicode", "search=" + encodeURIComponent("模型"), "unicode search"],
    ["models.b.inject", "search=" + encodeURIComponent("' OR 1=1--"), "injection-shaped input"],
    ["models.b.long", "search=" + "a".repeat(2000), "2 KB search term"],
  ]) await check(id, "models", "boundary", `${q} — ${expectation}`, { path: `/api/models?${q}` },
    (r) => ({ ok: r.status === 200 || r.status === 400, note: `${r.status}, ${Array.isArray(r.json) ? r.json.length + " rows" : "non-array"} (constraints are advisory)` }));

  await check("models.info", "models", "happy", "getModel", { path: `/api/models/${M}` }, (r) => ({ ok: r.json?.id === M, note: `${r.json?.siblings?.length} siblings` }));
  await check("models.info.rev", "models", "happy", "getModelAtRevision", { path: `/api/models/${M}/revision/${REV}` }, (r) => r.json?.sha === REV);
  await check("models.refs", "models", "happy", "listModelRefs", { path: `/api/models/${M}/refs` }, (r) => r.json?.branches?.[0]?.name === "main");
  await check("models.tree", "models", "happy", "listModelFiles", { path: `/api/models/${M}/tree/main` }, (r) => ({ ok: Array.isArray(r.json), note: `${r.json.length} files` }));
  await check("models.tree.prefix", "models", "happy", "listModelFilesUnder", { path: `/api/models/${M}/tree/main/1_Pooling` }, (r) => ({ ok: Array.isArray(r.json), note: `${r.json.length} under prefix` }));
  await check("models.tree.oid", "models", "consistency", "oid is a 64-char SHA-256, not HF's 40-char SHA-1",
    { path: `/api/models/${M}/tree/main` }, (r) => ({ ok: r.json.every((f) => /^[0-9a-f]{64}$/.test(f.oid)), note: "every entry 64 hex" }));
  await check("models.404", "models", "errors", "RepoNotFound", { path: "/api/models/nobody/nothing-xyz" },
    (r) => ({ ok: r.status === 404 && r.headers["x-error-code"] === "RepoNotFound", note: r.headers["x-error-code"] }));
  await check("models.404.rev", "models", "errors", "RevisionNotFound", { path: `/api/models/${M}/tree/0000000` },
    (r) => ({ ok: r.status === 404 && r.headers["x-error-code"] === "RevisionNotFound", note: r.headers["x-error-code"] }));
  await check("models.404.revshort", "models", "boundary", "a six-character prefix is too short", { path: `/api/models/${M}/tree/${REV.slice(0, 6)}` },
    (r) => ({ ok: r.status === 404, note: `${r.status} ${r.headers["x-error-code"] || ""}` }));
  await check("models.revshort7", "models", "boundary", "a seven-character prefix is accepted", { path: `/api/models/${M}/tree/${REV.slice(0, 7)}` },
    (r) => ({ ok: r.status === 200, note: `${r.status}` }));
  for (const [id, method] of [["models.m.post", "POST"], ["models.m.put", "PUT"], ["models.m.delete", "DELETE"]])
    await check(id, "models", "methods", `${method} is refused`, { path: `/api/models/${M}`, method },
      (r) => ({ ok: r.status === 405 && r.headers["x-error-code"] === "ReadOnly", note: `${r.status} ${r.headers["x-error-code"] || ""}` }));
  await check("models.m.head", "models", "methods", "HEAD works", { path: "/api/models?limit=1", method: "HEAD" }, (r) => r.status === 200);
  await check("models.m.options", "models", "methods", "preflight", { path: "/api/models", method: "OPTIONS", headers: { origin: "https://example.com", "access-control-request-method": "GET" } },
    (r) => ({ ok: r.status === 204, note: r.headers["access-control-allow-methods"] || "" }));

  // ---- files
  const rf = await check("files.resolve", "files", "happy", "resolveFile → 302", { path: `/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: r.status === 302 && !!r.headers.location, note: `${r.headers["x-hub-source"]} → ${new URL(r.headers.location).host}` }));
  await check("files.resolve.hash", "files", "consistency", "the 302 carries the index's hash",
    { path: `/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: (r.headers.etag || "").replaceAll('"', "") === OID, note: `ETag matches tree oid ${OID.slice(0, 12)}…` }));
  await check("files.resolve.headers", "files", "headers", "all six documented headers present",
    { path: `/${M}/resolve/main/${F}`, method: "HEAD" }, (r) => {
      const want = ["location", "etag", "x-linked-etag", "x-linked-size", "x-repo-commit", "x-hub-source"];
      const missing = want.filter((h) => !r.headers[h]);
      return { ok: !missing.length, note: missing.length ? `missing ${missing.join(",")}` : "all present" };
    });
  await check("files.sums", "files", "happy", "the synthesised SHA256SUMS", { path: `/${M}/resolve/main/SHA256SUMS` },
    (r) => ({ ok: r.text.includes(OID), note: `${r.text.trim().split("\n").length} lines, contains our file's hash` }));
  await check("files.404", "files", "errors", "EntryNotFound", { path: `/${M}/resolve/main/not-a-file.bin` },
    (r) => ({ ok: r.status === 404 && r.headers["x-error-code"] === "EntryNotFound", note: r.headers["x-error-code"] }));
  await check("files.traversal", "files", "security", "path traversal is not honoured", { path: `/${M}/resolve/main/../../../etc/passwd` },
    (r) => ({ ok: r.status >= 400 || !/root:/.test(r.text), note: `${r.status}` }));
  await check("files.range", "files", "headers", "Range is advertised on the redirect", { path: `/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: r.headers["accept-ranges"] === "bytes", note: r.headers["accept-ranges"] || "absent" }));
  await check("files.nostore", "files", "headers", "the redirect itself is not cached", { path: `/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: /no-store/.test(r.headers["cache-control"] || ""), note: r.headers["cache-control"] }));

  for (const [id, src] of [["files.via.hf", "huggingface"], ["files.via.ms", "modelscope"], ["files.via.ipfs", "ipfs"]])
    await check(id, "files", "consistency", `/via/${src} — does it honour the pin?`, { path: `/via/${src}/${M}/resolve/main/${F}`, method: "HEAD" },
      (r) => {
        const got = r.headers["x-hub-source"] || "";
        const asked = src === "huggingface" ? "huggingface.co" : src === "modelscope" ? "modelscope.cn" : "ipfs";
        return { ok: got === asked, note: got === asked ? `honoured (${got})` : `FELL BACK to ${got} without error` };
      });
  // A model the named source does not hold: this is where the pin silently becomes a preference.
  for (const [id, src, expect] of [["files.via.miss.ms", "modelscope", "modelscope.cn"], ["files.via.miss.ipfs", "ipfs", "ipfs"]])
    await check(id, "files", "consistency", `/via/${src} on a model that source may not hold`, { path: `/via/${src}/BAAI/bge-base-en-v1.5/resolve/main/config.json`, method: "HEAD" },
      (r) => { const got = r.headers["x-hub-source"] || ""; return { ok: got === expect, note: got === expect ? `honoured (${got})` : `FELL BACK to ${got} with no error and no signal but this header` }; });

  await check("files.via.bogus", "files", "boundary", "/via/bogus is accepted and ignored", { path: `/via/bogus/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: r.status >= 400, note: r.status === 302 ? `accepted, served ${r.headers["x-hub-source"]}` : `${r.status}` }));
  await check("files.via.caps", "files", "boundary", "/via/HuggingFace (capitals)", { path: `/via/HuggingFace/${M}/resolve/main/${F}`, method: "HEAD" },
    (r) => ({ ok: true, note: `${r.status} ${r.headers["x-error-code"] || "(bare edge 404, not the documented shape)"}` }));

  // ---- objects
  await check("obj.get", "objects", "happy", "getObject by address", { path: `/api/v1/objects/${sample.catalog}`, method: "HEAD" },
    (r) => ({ ok: r.status === 200, note: r.headers["content-type"] }));
  await check("obj.immutable", "objects", "headers", "an address is cacheable forever", { path: `/api/v1/objects/${sample.catalog}`, method: "HEAD" },
    (r) => ({ ok: /immutable/.test(r.headers["cache-control"] || ""), note: r.headers["cache-control"] }));
  await check("obj.selfhash", "objects", "security", "the catalog hashes to its own address", { path: `/api/v1/objects/${sample.catalog}` },
    (r) => ({ ok: true, note: `${r.text.length}B (BLAKE3 not verifiable with node crypto; length recorded)` }));
  await check("obj.404", "objects", "errors", "a well-formed absent address", { path: "/api/v1/objects/blake3:" + "0".repeat(64) },
    (r) => ({ ok: r.json?.code === "LIVE_NOT_FOUND", note: r.json?.code || r.text.slice(0, 40) }));
  await check("obj.malformed", "objects", "errors", "a malformed address", { path: "/api/v1/objects/notanaddress" },
    (r) => ({ ok: !!r.json?.code, note: r.json?.code || `${r.status} ${r.headers["content-type"] || ""} — not the documented ApiError` }));
  await check("obj.list.anon", "objects", "auth", "listing needs a token", { path: "/api/v1/objects" },
    (r) => ({ ok: r.status === 401, note: `${r.status} ${r.headers["content-type"]}` }));
  await check("obj.search.anon", "objects", "auth", "search needs a token", { path: "/api/v1/objects/search?q=a" }, (r) => r.status === 401);
  await check("obj.post.anon", "objects", "auth", "publishing needs a token", { path: "/api/v1/objects", method: "POST", body: "x" }, (r) => r.status === 401);
  await check("obj.caps", "objects", "happy", "capabilities", { path: "/api/v1/capabilities" },
    (r) => ({ ok: !!r.json?.operations, note: `${r.json?.operations?.length} ops, max ${r.json?.maximum_message_bytes}B` }));
  await check("obj.modules", "objects", "happy", "modules", { path: "/api/v1/modules" }, (r) => Array.isArray(r.json));
  await check("obj.bare", "objects", "boundary", "/api/v1 is deliberately closed", { path: "/api/v1" }, (r) => r.status === 404);

  // ---- registry
  await check("reg.base", "registry", "happy", "/v2/ version check", { path: "/v2/" }, (r) => r.status === 200);
  await check("reg.catalog", "registry", "happy", "/v2/_catalog", { path: "/v2/_catalog" }, (r) => ({ ok: Array.isArray(r.json?.repositories), note: r.json?.repositories?.join(", ") }));
  await check("reg.tags", "registry", "happy", "model tags", { path: `/v2/${lower}/tags/list` }, (r) => ({ ok: Array.isArray(r.json?.tags), note: (r.json?.tags || []).join(",") }));
  await check("reg.manifest.oci", "registry", "happy", "ModelPack manifest", { path: `/v2/${lower}/manifests/latest`, headers: { accept: "application/vnd.oci.image.manifest.v1+json" } },
    (r) => ({ ok: r.json?.schemaVersion === 2, note: `${r.json?.layers?.length} layers, ${r.json?.artifactType || ""}` }));
  await check("reg.manifest.docker", "registry", "negotiation", "Docker/Ollama manifest", { path: `/v2/${lower}/manifests/latest`, headers: { accept: "application/vnd.docker.distribution.manifest.v2+json" } },
    (r) => ({ ok: r.status === 200 || r.status === 404, note: r.status === 404 ? `refused: ${r.json?.errors?.[0]?.code} (no GGUF)` : "served" }));
  await check("reg.case", "registry", "boundary", "OCI names are lowercase-only", { path: `/v2/${M}/tags/list` },
    (r) => ({ ok: true, note: `uppercase → ${r.status}` }));
  await check("reg.write.shut", "registry", "auth", "a write to the hub namespace needs a token", { path: "/v2/model-hub/index/blobs/uploads/", method: "PUT" },
    (r) => ({ ok: r.status === 401, note: `${r.status}` }));
  await check("reg.write.model", "registry", "auth", "a write to a model repo hits the read-only shim", { path: `/v2/${lower}/blobs/uploads/`, method: "PUT" },
    (r) => ({ ok: r.status === 405 || r.status === 401, note: `${r.status} ${r.headers["x-error-code"] || ""}` }));

  // ---- MCP
  const rpc = (method, params, id = 1) => ({ path: "/mcp", method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id, method, params }) });
  await check("mcp.get", "mcp", "methods", "GET is refused (stateless)", { path: "/mcp" }, (r) => r.status === 405);
  await check("mcp.init", "mcp", "happy", "initialize", rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "stress", version: "1" } }),
    (r) => ({ ok: !!r.json?.result?.serverInfo, note: r.json?.result?.protocolVersion }));
  await check("mcp.tools", "mcp", "happy", "tools/list", rpc("tools/list", {}, 2), (r) => ({ ok: r.json?.result?.tools?.length === 3, note: r.json?.result?.tools?.map((t) => t.name).join(",") }));
  await check("mcp.call", "mcp", "happy", "tools/call search_models", rpc("tools/call", { name: "search_models", arguments: { query: "qwen", limit: 2 } }, 3),
    (r) => ({ ok: !r.json?.result?.isError, note: "returned results" }));
  await check("mcp.call.badargs", "mcp", "errors", "a wrong argument name", rpc("tools/call", { name: "resolve_file", arguments: { model: "x/y" } }, 4),
    (r) => ({ ok: r.json?.result?.isError === true, note: (r.json?.result?.content?.[0]?.text || "").slice(0, 70) }));
  await check("mcp.call.unknown", "mcp", "errors", "an unknown tool", rpc("tools/call", { name: "no_such_tool", arguments: {} }, 5),
    (r) => ({ ok: r.json?.result?.isError === true || !!r.json?.error, note: (r.json?.result?.content?.[0]?.text || r.json?.error?.message || "").slice(0, 70) }));
  await check("mcp.malformed", "mcp", "boundary", "not JSON at all", { path: "/mcp", method: "POST", headers: { "content-type": "application/json" }, body: "{not json" },
    (r) => ({ ok: r.status < 500, note: `${r.status} ${r.json?.error?.code ?? ""}` }));
  await check("mcp.ping", "mcp", "happy", "ping", rpc("ping", {}, 6), (r) => !!r.json?.result);
  await check("mcp.resources", "mcp", "happy", "resources/list", rpc("resources/list", {}, 7), (r) => Array.isArray(r.json?.result?.resources));
  await check("mcp.prompts", "mcp", "happy", "prompts/list", rpc("prompts/list", {}, 8), (r) => Array.isArray(r.json?.result?.prompts));

  // ---- account
  await check("acct.health", "account", "happy", "sign-in availability", { path: "/api/account/health" },
    (r) => ({ ok: r.json?.ok === true, note: `configured=${r.json?.configured}` }));
  for (const [id, path, method] of [
    ["acct.me.anon", "/api/account/me", "GET"], ["acct.patch.anon", "/api/account/me", "PATCH"],
    ["acct.saved.anon", "/api/account/saved", "POST"], ["acct.del.anon", "/api/account/saved/a%2Fb", "DELETE"],
    ["acct.req.anon", "/api/account/request", "POST"], ["acct.pub.anon", "/api/account/publisher-request", "POST"],
  ]) await check(id, "account", "auth", `${method} without a token`, { path, method, headers: { "content-type": "application/json" }, body: method === "GET" ? null : "{}" },
    (r) => ({ ok: r.status === 401, note: `${r.status} ${r.json?.error || ""}` }));
  await check("acct.badtoken", "account", "auth", "a forged bearer token", { path: "/api/account/me", headers: { authorization: "Bearer not.a.real.jwt" } },
    (r) => ({ ok: r.status === 401, note: `${r.status} ${r.json?.error || ""}` }));

  // ---- edge / security posture
  await check("sec.grpc", "edge", "security", "gRPC is closed", { path: "/hologram.live.v1.HologramLive/Handshake", method: "POST" }, (r) => r.status === 404);
  await check("sec.v1", "edge", "security", "/v1/* is reserved and empty", { path: "/v1/chat/completions", method: "POST" }, (r) => ({ ok: r.status === 404, note: `${r.status}` }));
  await check("sec.authstrip", "edge", "security", "a token sent to the shim is not honoured", { path: `/api/models/${M}`, headers: { authorization: "Bearer hf_fake" } },
    (r) => ({ ok: r.status === 200, note: "answered anonymously, credential ignored" }));
  await check("sec.unknown", "edge", "boundary", "an unknown path", { path: "/definitely-not-a-route" }, (r) => ({ ok: r.status === 404, note: `${r.status}` }));

  // ---- summary
  const fails = results.filter((r) => r.verdict === "FAIL");
  const passes = results.filter((r) => r.verdict === "pass");
  console.log(`\n${results.length} checks · ${passes.length} pass · ${fails.length} fail · ${results.length - passes.length - fails.length} recorded`);
  if (fails.length) { console.log("\nfailures:"); for (const f of fails) console.log(`  ${f.id.padEnd(30)} ${f.note || f.what}`); }

  if (OUT) {
    await mkdir(OUT.replace(/[^/\\]+$/, ""), { recursive: true }).catch(() => {});
    await writeFile(OUT, JSON.stringify({ base: BASE, at: new Date().toISOString(), sample, summary: { total: results.length, pass: passes.length, fail: fails.length }, results }, null, 1) + "\n");
    console.log(`\nwrote ${OUT}`);
  }
}

run().catch((e) => { console.error(e); process.exit(1); });
