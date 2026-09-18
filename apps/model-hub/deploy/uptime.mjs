#!/usr/bin/env node
// Outside probe for hub.uor.foundation, run every 15 minutes by .github/workflows/model-hub-uptime.yml.
//   node uptime.mjs [base url]      needs hash-wasm (BLAKE3); prints one line per check
// It checks what "container running" cannot: that the pointer names a catalog, that the catalog and a model hash to
// their addresses, that the daily publish is fresh, and that the doors that must stay closed are closed.
// Writes failures=<space separated> to $GITHUB_OUTPUT when set. Always exits 0: the workflow decides what a failure means.
import { appendFileSync } from "node:fs";
import { blake3 } from "hash-wasm";

const BASE = (process.argv[2] || "https://hub.uor.foundation").replace(/\/$/, "");
const EXPECT_SEARCH = Number(process.env.EXPECT_SEARCH || 401); // 200 once search is open to the public
const MAX_AGE_HOURS = Number(process.env.MAX_AGE_HOURS || 36); // snapshot day D is published about 10:00 UTC; D+1 noon is late
const failures = [];
const note = (name, ok, detail) => { console.log(`${ok ? "ok  " : "FAIL"} ${name} ${detail ?? ""}`); if (!ok) failures.push(`${name}=${String(detail).replace(/\s+/g, "_")}`); };

async function http(path, init = {}, tries = 3) {
	for (let i = 1; ; i++) {
		try { return await fetch(BASE + path, { ...init, signal: AbortSignal.timeout(20_000) }); }
		catch (e) { if (i === tries) return { status: 0, ok: false, headers: new Headers(), text: async () => "", json: async () => ({}), arrayBuffer: async () => new ArrayBuffer(0) }; await new Promise((r) => setTimeout(r, 5_000)); }
	}
}
const status = async (name, path, want, init) => { const r = await http(path, init); note(name, r.status === want, r.status); return r; };
async function object(name, id) {
	const r = await http(`/api/v1/objects/${id}`);
	const bytes = new Uint8Array(await r.arrayBuffer());
	const got = `blake3:${await blake3(bytes)}`;
	note(name, r.status === 200 && got === id, r.status === 200 ? (got === id ? `${bytes.length}B` : `hashes-to-${got.slice(0, 23)}`) : r.status);
	return got === id ? JSON.parse(new TextDecoder().decode(bytes)) : null;
}

// What was probed before: the site, a model page, the registry.
await status("site", "/", 200);
await status("model-page", "/models/hexgrad/Kokoro-82M/", 200);
await status("registry", "/v2/", 200);
const tags = await http("/v2/model-hub/index/tags/list");
note("index-tags", tags.status === 200 && /"tags":\[\s*"/.test(await tags.text()), tags.status);

// The server, and the hub it carries.
const health = await http("/healthz");
const h = await health.json().catch(() => ({}));
note("server", health.status === 200 && h.modules_ready === 2, `${health.status}/modules=${h.modules_ready}`);
await status("llms", "/llms.txt", 200);
const root = await http("/", { headers: { accept: "application/json" } });
const d = await root.json().catch(() => ({}));
const named = /^blake3:[0-9a-f]{64}$/.test(d.catalog || "");
note("descriptor", named, named ? d.snapshot : "no-catalog");
if (named) {
	const catalog = await object("catalog", d.catalog);
	if (catalog) {
		const age = (Date.now() - Date.parse(`${catalog.snapshot}T00:00:00Z`)) / 36e5;
		note("fresh", age <= MAX_AGE_HOURS, `${catalog.snapshot}/${Math.round(age)}h`);
		const first = Object.values(catalog.objects || {})[0];
		if (first) await object("model", first.model); else note("model", false, "catalog-names-none");
	}
}

// Doors that must stay closed. A bare Hologram Server answers 200 application/grpc to any unknown path.
await status("unknown-path", "/api/v1/nope", 404);
await status("grpc", "/hologram.live.v1.HologramLive/Call", 404, { method: "POST" });
await status("anonymous-publish", "/api/v1/objects", 401, { method: "POST", body: "x" });
await status("anonymous-list", "/api/v1/objects", 401);
await status("search", "/api/v1/objects/search?kind=model-hub.catalog", EXPECT_SEARCH);

console.log(failures.length ? `failures: ${failures.join(" ")}` : "all checks passed");
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `failures=${failures.join(" ")}\n`);
