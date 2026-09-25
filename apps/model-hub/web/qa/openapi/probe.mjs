// Probe every public route of the hub endpoint and record what it actually answers.
// The recording is the evidence behind every example in deploy/openapi.json: no example is invented.
//   node probe.mjs [--base https://gethologram.ai] [--out evidence.json]
// Read-only. It never sends a token and never asks for weight bytes: file routes are probed with HEAD.
import { writeFile } from "node:fs/promises";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 ? process.argv[i + 1] : fallback;
};
const BASE = arg("base", "https://gethologram.ai").replace(/\/$/, "");
const OUT = arg("out", new URL("./evidence.json", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"));

// A model that is in the index today is discovered at run time, so the probe keeps working as the index changes.
let MODEL = "Qwen/Qwen-Image-2.1";
let REVISION = "main";
let FILE = "README.md";
let CATALOG = null;

const KEEP = [
  "content-type", "cache-control", "etag", "location", "x-repo-commit", "x-linked-etag", "x-linked-size",
  "x-hub-source", "x-total-count", "x-error-code", "x-error-message", "accept-ranges", "docker-content-digest",
  "access-control-allow-origin", "access-control-expose-headers", "content-encoding",
];

async function hit(id, path, { method = "GET", headers = {}, body = null, redirect = "manual" } = {}) {
  const url = path.startsWith("http") ? path : `${BASE}${path}`;
  const started = Date.now();
  let res, text = "", error = null;
  try {
    res = await fetch(url, { method, headers, body, redirect, signal: AbortSignal.timeout(25_000) });
    if (method !== "HEAD") text = await res.text();
  } catch (e) {
    error = String(e.message || e);
  }
  const record = {
    id, method, path, ms: Date.now() - started,
    status: res ? res.status : null,
    headers: res ? Object.fromEntries(KEEP.filter((h) => res.headers.has(h)).map((h) => [h, res.headers.get(h)])) : {},
    body: text ? shrink(safe(text)) : undefined,
    error,
  };
  console.log(`${String(record.status ?? "ERR").padEnd(4)} ${method.padEnd(4)} ${path.slice(0, 78)}`);
  return record;
}

// The whole body is parsed, then shrunk: an example keeps the shape of the answer, never its length.
const safe = (text) => { try { return JSON.parse(text); } catch { return text.slice(0, 600); } };
const shrink = (value, depth = 0) => {
  if (typeof value === "string") return value.length > 400 ? `${value.slice(0, 400)}…` : value;
  if (Array.isArray(value)) return value.slice(0, 3).map((v) => shrink(v, depth + 1));
  if (value && typeof value === "object" && depth < 6) return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, shrink(v, depth + 1)]));
  return value;
};

async function discover() {
  const r = await fetch(`${BASE}/api/models?limit=1&sort=downloads`, { signal: AbortSignal.timeout(20_000) });
  const rows = await r.json();
  if (Array.isArray(rows) && rows[0]?.id) { MODEL = rows[0].id; REVISION = rows[0].sha; }
  const d = await fetch(`${BASE}/.well-known/model-hub.json`, { signal: AbortSignal.timeout(20_000) });
  CATALOG = (await d.json()).catalog;
  const t = await fetch(`${BASE}/api/models/${MODEL}/tree/main`, { signal: AbortSignal.timeout(20_000) });
  const tree = await t.json();
  const small = Array.isArray(tree) ? tree.filter((f) => f.size > 0 && f.size < 200_000).sort((a, b) => a.size - b.size)[0] : null;
  if (small) FILE = small.path;
  console.log(`# model ${MODEL} @ ${REVISION}, file ${FILE}, catalog ${CATALOG}\n`);
}

const CORS = { origin: "https://example.com" };

async function main() {
  await discover();
  const cases = [
    // discovery
    ["root.html", "/", {}],
    ["root.descriptor", "/", { headers: { accept: "application/json" } }],
    // What curl, node fetch and python requests all send. This is the one an arriving agent actually makes.
    ["root.brief", "/", { headers: { accept: "*/*" } }],
    ["brief", "/agent.md", { headers: CORS }],
    // Each section answers the way the root does; the page below a section must not be swallowed by that.
    ["section.models", "/models", { headers: { accept: "*/*" } }],
    ["section.models.slash", "/models/", { headers: { accept: "*/*" } }],
    ["section.models.page", "/models/", { headers: { accept: "text/html" } }],
    ["section.registry", "/registry", { headers: { accept: "*/*" } }],
    ["section.spaces", "/spaces", { headers: { accept: "*/*" } }],
    ["section.buckets", "/buckets", { headers: { accept: "*/*" } }],
    // Docs is the one section with no slashless address of its own: /docs is the server's API reference.
    ["section.docs", "/docs/", { headers: { accept: "*/*" } }],
    ["section.docs.page", "/docs/", { headers: { accept: "text/html" } }],
    // The descriptor: the same five addresses, asked for by name.
    ["section.models.json", "/models", { headers: { accept: "application/json" } }],
    ["section.registry.json", "/registry", { headers: { accept: "application/json" } }],
    ["section.spaces.json", "/spaces", { headers: { accept: "application/json" } }],
    ["section.buckets.json", "/buckets", { headers: { accept: "application/json" } }],
    ["section.docs.json", "/docs/", { headers: { accept: "application/json" } }],
    // The descriptor: the same five addresses, asked for by name.
    ["section.models.json", "/models", { headers: { accept: "application/json" } }],
    ["section.registry.json", "/registry", { headers: { accept: "application/json" } }],
    ["section.spaces.json", "/spaces", { headers: { accept: "application/json" } }],
    ["section.buckets.json", "/buckets", { headers: { accept: "application/json" } }],
    ["section.docs.json", "/docs/", { headers: { accept: "application/json" } }],
    // The descriptor: the same five addresses, asked for by name.
    ["section.models.json", "/models", { headers: { accept: "application/json" } }],
    ["section.registry.json", "/registry", { headers: { accept: "application/json" } }],
    ["section.spaces.json", "/spaces", { headers: { accept: "application/json" } }],
    ["section.buckets.json", "/buckets", { headers: { accept: "application/json" } }],
    ["section.docs.json", "/docs/", { headers: { accept: "application/json" } }],
    ["descriptor", "/.well-known/model-hub.json", { headers: CORS }],
    ["llms", "/llms.txt", { headers: CORS }],
    ["openapi", "/openapi.json", { headers: CORS }],
    ["wellknown.openapi", "/.well-known/openapi.json", {}],
    ["agentcard", "/.well-known/agent-card.json", {}],
    ["robots", "/robots.txt", {}],
    ["docs", "/docs", {}],
    ["docs.index", "/docs/", {}],
    ["docs.page", "/docs/quickstart/", {}],
    ["docs.twin", "/docs/quickstart.md", { headers: CORS }],
    ["docs.missing", "/docs/not-a-page/", {}],
    ["archive", "/archive.json", { headers: CORS }],
    ["pins", "/pins.json", {}],
    // health
    ["healthz", "/healthz", { headers: CORS }],
    ["hub.health", "/api/hub/health", { headers: CORS }],
    // models, Hugging Face dialect
    ["models.list", "/api/models?limit=2&sort=downloads", { headers: CORS }],
    ["models.search", "/api/models?search=qwen&pipeline_tag=text-generation&filter=gguf&limit=2", {}],
    ["models.info", `/api/models/${MODEL}`, { headers: CORS }],
    ["models.info.revision", `/api/models/${MODEL}/revision/${REVISION}`, {}],
    ["models.refs", `/api/models/${MODEL}/refs`, {}],
    ["models.tree", `/api/models/${MODEL}/tree/main`, {}],
    ["models.missing", "/api/models/nobody/not-a-real-model-xyz", {}],
    ["models.revision.wrong", `/api/models/${MODEL}/tree/0000000`, {}],
    ["models.readonly", `/api/models/${MODEL}`, { method: "POST" }],
    // files
    ["resolve", `/${MODEL}/resolve/main/${FILE}`, { method: "HEAD", headers: CORS }],
    ["resolve.sha256sums", `/${MODEL}/resolve/main/SHA256SUMS`, { headers: CORS }],
    ["resolve.entry.missing", `/${MODEL}/resolve/main/not-a-file.bin`, {}],
    ["via.ipfs", `/via/ipfs/${MODEL}/resolve/main/${FILE}`, { method: "HEAD" }],
    ["via.huggingface", `/via/huggingface/${MODEL}/resolve/main/${FILE}`, { method: "HEAD" }],
    ["via.list", "/via/ipfs/api/models?limit=1", {}],
    ["xet", `/api/models/${MODEL}/xet-read-token/main`, { method: "HEAD" }],
    // objects
    ["objects.get", `/api/v1/objects/${CATALOG}`, { method: "HEAD", headers: CORS }],
    ["objects.list.anon", "/api/v1/objects", {}],
    ["objects.search.anon", "/api/v1/objects/search?q=qwen", {}],
    ["objects.missing", "/api/v1/objects/blake3:" + "0".repeat(64), {}],
    ["capabilities", "/api/v1/capabilities", { headers: CORS }],
    ["modules", "/api/v1/modules", {}],
    ["api.v1.bare", "/api/v1", {}],
    // OCI registry
    ["oci.base", "/v2/", {}],
    ["oci.catalog", "/v2/_catalog", {}],
    ["oci.manifest.docker", `/v2/${MODEL.toLowerCase()}/manifests/latest`, { headers: { accept: "application/vnd.docker.distribution.manifest.v2+json" } }],
    ["oci.manifest.modelpack", `/v2/${MODEL.toLowerCase()}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" } }],
    ["oci.tags", `/v2/${MODEL.toLowerCase()}/tags/list`, {}],
    ["oci.index.tags", "/v2/model-hub/index/tags/list", {}],
    // account, the only surface that needs a person signed in
    ["account.health", "/api/account/health", {}],
    ["account.me.anon", "/api/account/me", {}],
    ["account.me.patch.anon", "/api/account/me", { method: "PATCH" }],
    ["account.saved.anon", "/api/account/saved", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: "a/b" }) }],
    ["account.saved.delete.anon", "/api/account/saved/a%2Fb", { method: "DELETE" }],
    ["account.request.anon", "/api/account/request", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: "a/b" }) }],
    ["account.publisher.anon", "/api/account/publisher-request", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ note: "probe" }) }],
    ["account.unknown.anon", "/api/account/nope", {}],
    // MCP
    ["mcp.get", "/mcp", {}],
    ["mcp.initialize", "/mcp", { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "hub-probe", version: "1" } } }) }],
    ["mcp.tools.list", "/mcp", { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }) }],
    ["mcp.tools.call", "/mcp", { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "search_models", arguments: { query: "qwen", limit: 2 } } }) }],
    // preflight
    ["preflight.models", "/api/models", { method: "OPTIONS", headers: { origin: "https://example.com", "access-control-request-method": "GET" } }],
    ["preflight.objects", "/api/v1/objects", { method: "OPTIONS", headers: { origin: "https://example.com", "access-control-request-method": "POST" } }],
    // routes that must stay closed
    ["closed.grpc", "/hologram.live.v1.HologramLive/Handshake", { method: "POST" }],
    ["nav.wallpapers", "/wallpapers/", {}],
  ];

  const records = [];
  for (const [id, path, opts] of cases) records.push(await hit(id, path, opts));

  const evidence = {
    probed_at: new Date().toISOString(),
    base: BASE,
    sample: { model: MODEL, revision: REVISION, file: FILE, catalog: CATALOG },
    records,
  };
  await writeFile(OUT, JSON.stringify(evidence, null, 1) + "\n");
  const bad = records.filter((r) => r.error);
  console.log(`\n# ${records.length} routes recorded to ${OUT}${bad.length ? `, ${bad.length} failed to connect` : ""}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
