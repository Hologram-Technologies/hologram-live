// Can an agent framework reach this endpoint knowing only the host name?
//
// This is the claim the document exists to support, so it is tested the way a framework does it, with nothing
// hand-written about the hub:
//
//   0. arrive     GET / the way curl does (Accept: */*) and check the answer is usable prose, not markup
//   1. discover   GET / with Accept: application/json, follow the descriptor to the OpenAPI document
//   2. bind       turn every operation into a tool definition, in the three shapes frameworks actually consume
//                 (OpenAI function calling, Anthropic tool use, MCP), and refuse any operation that cannot be bound
//   3. act        run find -> describe -> list files -> resolve -> download -> verify, choosing every URL from the
//                 tool definitions alone
//   4. mcp        bind the same endpoint through /mcp and run the same chain through its tools
//
// The last step downloads one small file and checks its SHA-256 against the hash the index gave, which is the whole
// point of the hub: the agent proves the bytes without trusting the host that served them.
//
//   node bind.mjs [--base https://gethologram.ai]
//   node bind.mjs --spec ../../public/openapi.json    bind the document as built here, before it is deployed
//
// With --spec, the discovery checks still run against the live endpoint but a path it does not serve yet is reported
// as pending rather than failed, so this can be run before the deploy and again after it, unchanged.
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

const arg = (name, fallback) => { const i = process.argv.indexOf(`--${name}`); return i > -1 ? process.argv[i + 1] : fallback; };
const BASE = arg("base", "https://gethologram.ai").replace(/\/$/, "");
const LOCAL_SPEC = arg("spec", null);
const MAX_FILE = 2_000_000;

const pass = [], fail = [], pending = [];
const ok = (line) => pass.push(line);
const bad = (line) => fail.push(line);
// Before the deploy the live endpoint cannot serve what this document describes; say so instead of pretending.
const soon = (line) => (LOCAL_SPEC ? pending.push(line) : fail.push(line));
const get = async (url, init) => fetch(url, { signal: AbortSignal.timeout(30_000), ...init });

// ---- 1. discover ------------------------------------------------------------------------------

async function discover() {
  // What the hero tells an agent to run. curl, node fetch and python requests all send */*, so this is the
  // request an arriving agent actually makes, and it has to land on something it can act on.
  const arriving = await get(`${BASE}/`, { headers: { accept: "*/*" } });
  const brief = await arriving.text();
  const type = (arriving.headers.get("content-type") || "").split(";")[0];
  if (type === "text/html") {
    soon(`curl ${BASE} still answers ${Math.round(brief.length / 1024)} KB of HTML, which an agent cannot use`);
  } else if (!/resolve\/main/.test(brief) || !/SHA-256|sha256/.test(brief)) {
    bad(`curl ${BASE} answered ${type} but it does not show how to fetch a file or how to check it`);
  } else {
    ok(`curl ${BASE} answers ${Math.round(brief.length / 1024)} KB of ${type}: the calls and the rule, no markup`);
    if (!/canary/.test(brief)) soon("the brief carries no canary, so a summarising fetcher cannot be detected");
  }

  const root = await get(`${BASE}/`, { headers: { accept: "application/json" } });
  const jsonType = (root.headers.get("content-type") || "").split(";")[0];
  if (jsonType !== "application/json") throw new Error(`the root did not answer JSON to an Accept: application/json request, it answered ${jsonType}`);
  const descriptor = await root.json();
  ok(`discovered ${descriptor.name || "the hub"} from the bare host name`);

  const specUrl = descriptor.openapi ? new URL(descriptor.openapi, BASE).href : `${BASE}/openapi.json`;
  if (!descriptor.openapi) bad("the descriptor does not name its OpenAPI document; a framework has to guess the path");
  else ok(`the descriptor names its OpenAPI document: ${descriptor.openapi}`);

  const live = await (await get(specUrl)).json();
  const spec = LOCAL_SPEC ? JSON.parse(await readFile(LOCAL_SPEC, "utf8")) : live;
  if (LOCAL_SPEC) ok(`binding the document built at ${LOCAL_SPEC}; the live one at ${specUrl} has ${Object.keys(live.paths).length} paths`);
  if (spec.openapi?.startsWith("3.")) ok(`the document is OpenAPI ${spec.openapi} with ${Object.keys(spec.paths).length} paths`);
  else bad(`${specUrl} is not an OpenAPI document`);
  if (LOCAL_SPEC && Object.keys(live.paths).length < Object.keys(spec.paths).length) soon(`the live /openapi.json still describes only ${Object.keys(live.paths).length} paths`);

  const wellKnown = await get(`${BASE}/.well-known/openapi.json`);
  if (wellKnown.ok) ok("the document is also at /.well-known/openapi.json, where a framework looks first");
  else soon(`/.well-known/openapi.json answered ${wellKnown.status}`);

  const card = await get(`${BASE}/.well-known/agent-card.json`);
  if (card.ok) { const c = await card.json(); ok(`an agent card is served, with ${c.skills?.length ?? 0} skills`); }
  else soon(`/.well-known/agent-card.json answered ${card.status}`);

  return { descriptor, spec, specUrl };
}

// ---- 2. bind ----------------------------------------------------------------------------------

// The conversion every framework performs. It is deliberately dumb: anything it cannot do here, a framework cannot
// do either, and that is a defect in the document rather than in the framework.
function bind(spec) {
  const tools = [];
  const unbindable = [];
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(item)) {
      if (!["get", "post", "put", "patch", "delete"].includes(method)) continue;
      if (!op.operationId) { unbindable.push(`${method.toUpperCase()} ${path}: no operationId, so a tool has no name`); continue; }
      if (!op.description && !op.summary) { unbindable.push(`${op.operationId}: no description, so a model cannot tell when to call it`); continue; }
      const properties = {}, required = [];
      for (const p of op.parameters || []) {
        if (!p.schema) { unbindable.push(`${op.operationId}: parameter ${p.name} has no schema`); continue; }
        properties[p.name] = { ...p.schema, description: p.description || p.name };
        if (p.required) required.push(p.name);
      }
      if (op.requestBody) {
        const schema = op.requestBody.content?.["application/json"]?.schema;
        properties.body = schema ? { ...schema, description: op.requestBody.description || "request body" } : { type: "string", description: "request body" };
        if (op.requestBody.required) required.push("body");
      }
      tools.push({
        name: op.operationId,
        description: [op.summary, op.description].filter(Boolean).join(". ").slice(0, 1024),
        method, path,
        parameters: { type: "object", properties, required },
        security: op.security ?? spec.security ?? [],
      });
    }
  }
  return { tools, unbindable };
}

const asOpenAi = (t) => ({ type: "function", function: { name: t.name, description: t.description, parameters: t.parameters } });
const asAnthropic = (t) => ({ name: t.name, description: t.description, input_schema: t.parameters });
const asMcp = (t) => ({ name: t.name, description: t.description, inputSchema: t.parameters });

// ---- 3. act, using only the bound tools -------------------------------------------------------

// Call a bound tool the way a framework would: fill the path template, put the rest in the query string.
async function call(tool, args = {}, init = {}) {
  let path = tool.path;
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(args)) {
    if (value === undefined) continue;
    if (path.includes(`{${key}}`)) path = path.replace(`{${key}}`, String(value).split("/").map(encodeURIComponent).join("/"));
    else query.set(key, String(value));
  }
  if (path.includes("{")) throw new Error(`${tool.name}: no value for ${path.match(/\{[^}]+\}/g).join(", ")}`);
  return get(`${BASE}${path}${query.size ? `?${query}` : ""}`, { method: tool.method.toUpperCase(), ...init });
}

async function act(tools) {
  const tool = (name) => { const t = tools.find((x) => x.name === name); if (!t) throw new Error(`the document has no ${name} operation`); return t; };

  const rows = await (await call(tool("listModels"), { limit: 20, sort: "downloads" })).json();
  if (!rows.length) throw new Error("listModels returned nothing");
  ok(`listModels found ${rows.length} models, the first being ${rows[0].id}`);

  // Pick a model with a small file, the way an agent testing a pipeline would.
  let chosen = null;
  for (const row of rows) {
    const [owner, name] = row.id.split("/");
    const files = await (await call(tool("listModelFiles"), { owner, name, revision: "main" })).json();
    const small = files.filter((f) => f.size > 0 && f.size < MAX_FILE).sort((a, b) => a.size - b.size)[0];
    if (small) { chosen = { owner, name, id: row.id, file: small }; break; }
  }
  if (!chosen) throw new Error("no model in the first page has a file small enough to check");

  const info = await (await call(tool("getModel"), { owner: chosen.owner, name: chosen.name })).json();
  ok(`getModel pinned ${info.id} at ${info.sha.slice(0, 12)} with ${info.siblings.length} files`);
  ok(`listModelFiles gave ${chosen.file.path} its size and its expected SHA-256`);

  const redirect = await call(tool("resolveFile"), { owner: chosen.owner, name: chosen.name, revision: "main", path: chosen.file.path }, { redirect: "manual" });
  if (redirect.status !== 302) throw new Error(`resolveFile answered ${redirect.status}, not a redirect`);
  const location = redirect.headers.get("location");
  const source = redirect.headers.get("x-hub-source");
  const advertised = (redirect.headers.get("etag") || "").replaceAll('"', "");
  ok(`resolveFile chose ${source} and pointed at ${new URL(location).host}`);

  // The hash the agent checks against comes from the index, never from the source. Prove they agree before trusting.
  if (advertised !== chosen.file.oid) bad(`the redirect advertises ${advertised.slice(0, 12)} but the index says ${chosen.file.oid.slice(0, 12)}`);
  else ok("the redirect's ETag equals the hash the index gave, so nothing was substituted in between");

  const bytes = Buffer.from(await (await get(location)).arrayBuffer());
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== chosen.file.oid) bad(`the downloaded ${chosen.file.path} hashes to ${digest.slice(0, 12)}, the index says ${chosen.file.oid.slice(0, 12)}`);
  else ok(`downloaded ${chosen.file.path} (${bytes.length} bytes) from ${source} and its SHA-256 matches the index`);

  const sums = await (await call(tool("resolveFile"), { owner: chosen.owner, name: chosen.name, revision: "main", path: "SHA256SUMS" })).json().catch(() => null);
  const sumsText = sums === null ? await (await call(tool("resolveFile"), { owner: chosen.owner, name: chosen.name, revision: "main", path: "SHA256SUMS" })).text() : "";
  if (sumsText.includes(chosen.file.oid)) ok("the synthesised SHA256SUMS carries the same hash, so a whole download checks with sha256sum -c");
  else bad("SHA256SUMS does not contain the hash the tree gave for this file");

  return chosen;
}

// ---- 4. the same endpoint through MCP ---------------------------------------------------------

async function viaMcp(descriptor, spec) {
  const url = spec.paths["/mcp"] ? `${BASE}/mcp` : null;
  if (!url) { bad("the document does not describe an MCP server"); return; }
  const rpc = async (method, params, id = 1) => {
    const r = await get(url, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id, method, params }) });
    const text = await r.text();
    return JSON.parse(text.replace(/^data: /gm, "").trim().split("\n").pop());
  };
  const init = await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "hub-bind", version: "1" } });
  ok(`MCP initialised: ${init.result.serverInfo.name} speaking ${init.result.protocolVersion}`);
  const list = await rpc("tools/list", {}, 2);
  ok(`MCP offers ${list.result.tools.length} tools: ${list.result.tools.map((t) => t.name).join(", ")}`);

  const found = await rpc("tools/call", { name: "search_models", arguments: { query: "qwen", limit: 3 } }, 3);
  if (found.result?.isError) { bad(`MCP search_models failed: ${found.result.content?.[0]?.text}`); return; }
  const models = JSON.parse(found.result.content[0].text);
  const id = models.models[0].id;
  ok(`MCP search_models found ${models.total} models, the first being ${id}`);

  const one = await rpc("tools/call", { name: "get_model", arguments: { id } }, 4);
  if (one.result?.isError) { bad(`MCP get_model failed: ${one.result.content?.[0]?.text}`); return; }
  const model = JSON.parse(one.result.content[0].text);
  const small = (model.files || []).filter((f) => f.size > 0 && f.size < MAX_FILE).sort((a, b) => a.size - b.size)[0];
  if (!small) { ok(`MCP get_model described ${id}; no file small enough to download in this check`); return; }

  const where = await rpc("tools/call", { name: "resolve_file", arguments: { id, path: small.path } }, 5);
  if (where.result?.isError) { bad(`MCP resolve_file failed: ${where.result.content?.[0]?.text}`); return; }
  const answer = JSON.parse(where.result.content[0].text);
  const bytes = Buffer.from(await (await get(answer.url)).arrayBuffer());
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest === (answer.sha256 || small.sha256)) ok(`MCP resolve_file gave a URL and a hash; the bytes match (${small.path}, ${bytes.length} bytes)`);
  else bad(`MCP resolve_file: ${small.path} hashed to ${digest.slice(0, 12)}, expected ${(answer.sha256 || small.sha256 || "").slice(0, 12)}`);
}

// ---- run --------------------------------------------------------------------------------------

async function main() {
  console.log(`# binding to ${BASE} knowing nothing but the host name\n`);
  const { spec, descriptor } = await discover();

  const { tools, unbindable } = bind(spec);
  for (const problem of unbindable) bad(`cannot be bound: ${problem}`);
  ok(`bound ${tools.length} operations into tool definitions`);
  const shapes = { "OpenAI function calling": asOpenAi, "Anthropic tool use": asAnthropic, MCP: asMcp };
  for (const [name, shape] of Object.entries(shapes)) {
    const built = tools.map(shape);
    const broken = built.filter((t) => !(t.name || t.function?.name) || !(t.description || t.function?.description));
    if (broken.length) bad(`${name}: ${broken.length} tools came out without a name or a description`);
    else ok(`${name}: ${built.length} tool definitions, every one named and described`);
  }

  await act(tools);
  await viaMcp(descriptor, spec);

  console.log(pass.map((l) => `  ok      ${l}`).join("\n"));
  if (pending.length) console.log(pending.map((l) => `  pending ${l}`).join("\n"));
  if (fail.length) console.log(fail.map((l) => `  FAIL    ${l}`).join("\n"));
  console.log(`\n${pass.length} passed, ${fail.length} failed${pending.length ? `, ${pending.length} waiting on the deploy` : ""}`);
  process.exit(fail.length ? 1 : 0);
}

main().catch((e) => { console.error(`FAIL ${e.message}`); process.exit(1); });
