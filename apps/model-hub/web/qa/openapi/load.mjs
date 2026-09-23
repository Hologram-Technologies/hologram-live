// Latency and error curves for the metadata plane.
//
// The hub is a redirector: a load test aimed through it becomes a load test aimed at Hugging Face, ModelScope and
// an IPFS gateway, who did not agree to it. So this measures metadata only, and probes the resolve route with HEAD
// so the hub's routing decision is timed without any source being asked to serve bytes.
//
//   node qa/openapi/load.mjs [--secs 15] [--levels 1,4,16,64] [--out .../load.json]
//
// It stops early if the error rate at any level exceeds 2 %, on the assumption that something upstream is being
// hurt and the number is not worth having.
import { writeFile, mkdir } from "node:fs/promises";

const arg = (n, d) => { const i = process.argv.indexOf(`--${n}`); return i > -1 ? process.argv[i + 1] : d; };
const BASE = arg("base", "https://hub.uor.foundation").replace(/\/$/, "");
const SECS = Number(arg("secs", 15));
const LEVELS = String(arg("levels", "1,4,16,64")).split(",").map(Number);
const OUT = arg("out", null);

const pct = (xs, p) => xs.length ? xs.slice().sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(xs.length * p))] : 0;

async function level(route, concurrency) {
  const until = Date.now() + SECS * 1000;
  const lat = []; let n = 0, errors = 0, statuses = {};
  const worker = async () => {
    while (Date.now() < until) {
      const t = Date.now();
      try {
        const r = await fetch(BASE + route.path, { method: route.method || "GET", headers: route.headers || {}, body: route.body, redirect: "manual", signal: AbortSignal.timeout(20_000) });
        if (route.method !== "HEAD") await r.arrayBuffer();
        lat.push(Date.now() - t); n++;
        statuses[r.status] = (statuses[r.status] || 0) + 1;
        if (r.status >= 500 || r.status === 429) errors++;
      } catch { errors++; n++; lat.push(Date.now() - t); }
    }
  };
  await Promise.all(Array.from({ length: concurrency }, worker));
  return { concurrency, requests: n, rps: +(n / SECS).toFixed(1), p50: pct(lat, 0.5), p95: pct(lat, 0.95), p99: pct(lat, 0.99), max: Math.max(...lat), errors, errorRate: +(errors / Math.max(1, n) * 100).toFixed(2), statuses };
}

async function main() {
  const descriptor = await (await fetch(BASE + "/.well-known/model-hub.json")).json();
  const rows = await (await fetch(BASE + "/api/models?limit=1&sort=downloads")).json();
  const M = rows[0].id;

  const routes = [
    { id: "search", what: "listModels, the hot browse path", path: "/api/models?search=qwen&limit=20" },
    { id: "tree", what: "listModelFiles, the hashes an agent needs", path: `/api/models/${M}/tree/main` },
    { id: "resolve.head", what: "the routing decision, no bytes pulled", path: `/${M}/resolve/main/config.json`, method: "HEAD" },
    { id: "object.head", what: "the content-addressed floor", path: `/api/v1/objects/${descriptor.catalog}`, method: "HEAD" },
    { id: "brief", what: "what every arriving agent fetches", path: "/", headers: { accept: "*/*" } },
    { id: "mcp", what: "a real MCP tool call", path: "/mcp", method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "search_models", arguments: { query: "qwen", limit: 5 } } }) },
  ];

  console.log(`# ${SECS}s per cell, concurrency ${LEVELS.join("/")}, metadata only, HEAD on resolve\n`);
  console.log("route          conc   rps    p50    p95    p99    max   err%");
  const out = [];
  for (const route of routes) {
    // One warm request first: the metadata cache has a 60 s TTL and a cold first call is not the steady state.
    const cold = Date.now(); await fetch(BASE + route.path, { method: route.method || "GET", headers: route.headers || {}, body: route.body, redirect: "manual" }).then((r) => r.arrayBuffer()).catch(() => {});
    const coldMs = Date.now() - cold;
    for (const c of LEVELS) {
      const r = await level(route, c);
      out.push({ route: route.id, what: route.what, coldMs, ...r });
      console.log(`${route.id.padEnd(14)} ${String(c).padStart(4)} ${String(r.rps).padStart(6)} ${String(r.p50).padStart(6)} ${String(r.p95).padStart(6)} ${String(r.p99).padStart(6)} ${String(r.max).padStart(6)} ${String(r.errorRate).padStart(6)}`);
      if (r.errorRate > 2) { console.log(`  stopping ${route.id}: error rate ${r.errorRate}% — not worth pushing harder`); break; }
    }
  }

  if (OUT) {
    await mkdir(OUT.replace(/[^/\\]+$/, ""), { recursive: true }).catch(() => {});
    await writeFile(OUT, JSON.stringify({ base: BASE, at: new Date().toISOString(), secondsPerCell: SECS, levels: LEVELS, model: M, results: out }, null, 1) + "\n");
    console.log(`\nwrote ${OUT}`);
  }
}

main().catch((e) => { console.error(e); process.exit(1); });
