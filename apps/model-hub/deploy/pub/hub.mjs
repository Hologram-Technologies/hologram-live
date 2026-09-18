#!/usr/bin/env node
// Model Hub v1 over the Hologram Server object API. No hub backend: every call below is an existing endpoint.
//   POST /api/v1/objects            publish (kind + filename headers, body = bytes, id = blake3 of the bytes)
//   GET  /api/v1/objects/search     discover (kind, filename_contains, created_after_millis, cursor)
//   GET  /api/v1/objects/{id}       fetch by address
// Three object kinds carry the hub:
//   model-hub.model    filename <owner>/<name>@<revision>   the file list with sha256 per file, and prev = the version before
//   model-hub.source   filename <owner>/<name>@<revision>   where bytes can be fetched (ipfs, http mirror, a Hologram Server)
//   model-hub.catalog  filename catalog@<date>              the browse catalog for a day
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { blake3 } from "hash-wasm";

const env = existsSync(new URL(".spike.env", import.meta.url))
	? Object.fromEntries(readFileSync(new URL(".spike.env", import.meta.url), "utf8").trim().split("\n").map((l) => l.split("=")))
	: {};
const HUB = process.env.HUB || env.HUB || "http://127.0.0.1:8088";
const TOKEN = process.env.HUB_PUBLISH_TOKEN || env.HUB_PUBLISH_TOKEN || "";
const READ_TOKEN = process.env.HUB_READ_TOKEN || ""; // set when talking to the bare server inside its network; sent to HUB only
const STATE = process.env.HUB_STATE || ""; // canonical descriptor file on the host; without it the descriptor is read over HTTP
const HF = "https://huggingface.co";
const API = "https://humuhumu33.github.io/hologram-api";
const CHUNK = 16 * 1024 * 1024; // half the server's 32 MiB body limit, and small enough for the registry's 3x upload buffering

const MEDIA = {
	"model-hub.model": "application/vnd.hologram.model-hub.model.v1+json",
	"model-hub.source": "application/vnd.hologram.model-hub.source.v1+json",
	"model-hub.catalog": "application/vnd.hologram.model-hub.catalog.v1+json",
	"model-hub.chunk": "application/octet-stream",
};

// ---- the three calls ----
export async function put(kind, filename, bytes, { base = HUB, token = TOKEN } = {}) {
	const r = await fetch(`${base}/api/v1/objects`, {
		method: "POST",
		headers: { authorization: `Bearer ${token}`, "content-type": MEDIA[kind], "x-hologram-kind": kind, "x-hologram-filename": filename },
		body: bytes,
	});
	if (!r.ok) throw new Error(`put ${filename}: ${r.status} ${await r.text()}`);
	const meta = await r.json();
	const local = `blake3:${await blake3(bytes)}`;
	if (meta.id !== local) throw new Error(`put ${filename}: server named it ${meta.id}, the bytes are ${local}`);
	return meta;
}

// Fetch by address and refuse anything that does not hash to the address asked for.
export async function get(id, { base = HUB } = {}) {
	const r = await fetch(`${base}/api/v1/objects/${id}`, base === HUB && READ_TOKEN ? { headers: { authorization: `Bearer ${READ_TOKEN}` } } : {});
	if (!r.ok) throw new Error(`get ${id}: ${r.status}`);
	const bytes = new Uint8Array(await r.arrayBuffer());
	const got = `blake3:${await blake3(bytes)}`;
	if (got !== id) throw new Error(`REFUSED ${id}: ${new URL(base).host} served bytes that hash to ${got}`);
	return bytes;
}

// Search walks every stored object on the registry provider, so the hub keeps it for token holders. Nothing below needs it.
export async function search(params, { base = HUB, token = TOKEN } = {}) {
	const out = [];
	let cursor;
	do {
		const q = new URLSearchParams({ limit: "1000", ...params, ...(cursor ? { cursor } : {}) });
		const r = await fetch(`${base}/api/v1/objects/search?${q}`, { headers: { authorization: `Bearer ${token}` } });
		if (!r.ok) throw new Error(`search: ${r.status} ${await r.text()}`);
		const page = await r.json();
		if (page.truncated) throw new Error("search: the server stopped scanning before the end; refusing a partial answer");
		out.push(...page.objects);
		cursor = page.next_cursor;
	} while (cursor);
	return out;
}

const json = (o) => new TextEncoder().encode(JSON.stringify(o));
const parse = (b) => JSON.parse(new TextDecoder().decode(b));
const split = (filename) => { const at = filename.lastIndexOf("@"); return [filename.slice(0, at), filename.slice(at + 1)]; };

// ---- the head: one small mutable file names today's catalog; everything under it is fetched by address ----
export async function head({ base = HUB } = {}) {
	const d = base === HUB && STATE
		? (existsSync(STATE) ? JSON.parse(readFileSync(STATE, "utf8")) : {})
		: await (await fetch(`${base}/`, { headers: { accept: "application/json" } })).json();
	if (!d.catalog) return { descriptor: d, catalog: null };
	return { descriptor: d, id: d.catalog, catalog: parse(await get(d.catalog, { base })) };
}

// ---- versions: every revision is its own object; prev links them. Walk from the head; no search needed. ----
export async function versions(id, opts, from) {
	let cur = from || (await head(opts)).catalog?.objects[id]?.model;
	const chain = [];
	while (cur) { const doc = parse(await get(cur, opts)); chain.push({ meta: { id: cur }, doc }); cur = doc.prev; }
	return chain; // newest first
}

function manifest({ id, revision, license, files, prev, indexManifest }) {
	return {
		format: "hologram.model-hub.model/v1",
		id, origin: "huggingface.co", revision, prev: prev || null, license: license || null,
		index_manifest: indexManifest || null,
		files: files.map(([path, size, address, weights]) => ({ path, size, sha256: address.replace(/^sha256:/, ""), weights: !!weights })),
	};
}

async function publishModel(m, objects, opts) {
	const before = objects[m.id]?.model || null;
	// A revision already in the chain is already published. Walking costs one fetch per earlier version.
	for (let c = before; c; ) { const d = parse(await get(c, opts)); if (d.revision === m.revision) return { skipped: true }; c = d.prev; }
	const meta = await put("model-hub.model", `${m.id}@${m.revision}`, json(manifest({ ...m, prev: before })), opts);
	objects[m.id] = { model: meta.id, sources: [] }; // sources name one revision, so a new revision starts with none
	return { object: meta.id, prev: before };
}

// The catalog is the root: browse rows for today, plus the address of every model and source the hub has ever published.
async function publishCatalog(rows, snapshot, objects, before, siteDir) {
	const same = (c) => c && JSON.stringify([c.snapshot, c.models, c.objects]) === JSON.stringify([snapshot, rows, objects]);
	let meta;
	if (same(before?.catalog)) meta = { id: before.id, size: 0, unchanged: true }; // nothing new: no new link in the chain
	else meta = await put("model-hub.catalog", `catalog@${snapshot}`, json({ format: "hologram.model-hub.catalog/v1", snapshot, prev: before?.id || null, models: rows, objects }));
	writeDescriptor(meta.id, snapshot, siteDir);
	return meta;
}

// The descriptor is the one mutable file. STATE is its home on the host; the copy under the site is what the world reads.
function writeDescriptor(catalogId, snapshot, siteDir) {
	const d = JSON.parse(readFileSync(new URL("descriptor.template.json", import.meta.url), "utf8"));
	d.catalog = catalogId; d.snapshot = snapshot;
	for (const path of [STATE, siteDir && join(siteDir, ".well-known", "model-hub.json")].filter(Boolean)) {
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, `${JSON.stringify(d, null, 1)}\n`);
	}
}

// ---- commands ----
const cmd = {
	// publish <data dir> [--site dir] [--limit n] [--revise id=file.json]
	// One model object per addressed model, one source object per IPFS pin, then the catalog that names them all.
	async publish([dir, ...flags]) {
		const flag = (n) => (flags.includes(n) ? flags[flags.indexOf(n) + 1] : undefined);
		const t0 = Date.now();
		const today = JSON.parse(readFileSync(join(dir, "models.json"), "utf8"));
		const before = await head();
		const objects = structuredClone(before.catalog?.objects || {}); // cumulative: nothing published is ever dropped
		let made = 0, skipped = 0;
		const rows = today.models.filter((m) => m.state === "addressed").slice(0, Number(flag("--limit")) || Infinity);
		let missing = 0;
		for (const row of rows) {
			// A row can say "addressed" while its file list was not written that day. No file list, no model object: skip, count, go on.
			if (!existsSync(join(dir, "files", `${row.id}.json`))) { missing++; continue; }
			const f = JSON.parse(readFileSync(join(dir, "files", `${row.id}.json`), "utf8"));
			const r = await publishModel({ id: row.id, revision: f.revision, license: row.license, files: f.files, indexManifest: f.manifest }, objects);
			r.skipped ? skipped++ : made++;
		}
		// Pinned models that fell off today's trending list still belong in the hub.
		const pins = flag("--pins") ? JSON.parse(readFileSync(flag("--pins"), "utf8")) : await (await fetch("https://hub.uor.foundation/pins.json")).json();
		for (const [id, pin] of Object.entries(pins.models)) {
			if (!objects[id]) {
				const doc = await (await fetch(`${API}/v1/huggingface.co/${id}/latest.json`)).json();
				if (doc.revision !== pin.revision) continue;
				await publishModel({ id, revision: doc.revision, files: doc.files.map((x) => [x.path, x.size, x.address, 0]), indexManifest: doc.manifest }, objects);
				made++;
			}
			const target = (await versions(id, undefined, objects[id].model)).find((v) => v.doc.revision === pin.revision);
			if (!target || target.meta.id !== objects[id].model) continue;
			const s = await put("model-hub.source", `${id}@${pin.revision}`, json({
				format: "hologram.model-hub.source/v1", id, revision: pin.revision, model: target.meta.id,
				kind: "ipfs", root: pin.root, resolve: `${pins.gateway}${pin.root}/`,
			}));
			if (!objects[id].sources.includes(s.id)) objects[id].sources.push(s.id);
		}
		const cat = await publishCatalog(today.models, today.snapshot, objects, before, flag("--site"));
		console.log(JSON.stringify({ models_published: made, models_unchanged: skipped, rows_without_file_list: missing, models_in_hub: Object.keys(objects).length, catalog: cat.id, catalog_bytes: cat.size, catalog_unchanged: !!cat.unchanged, prev_catalog: before.id || null, seconds: (Date.now() - t0) / 1000 }));
	},

	// revise <id> <revision> --site dir: publish another revision of a model from Hugging Face (LFS sha256 from the API, small files hashed here).
	async revise([id, revision, ...flags]) {
		const flag = (n) => (flags.includes(n) ? flags[flags.indexOf(n) + 1] : undefined);
		const before = await head();
		const objects = structuredClone(before.catalog.objects);
		const info = await (await fetch(`${HF}/api/models/${id}/revision/${revision}?blobs=true`)).json();
		const files = [];
		for (const s of info.siblings) {
			let sha = s.lfs?.sha256;
			if (!sha) sha = createHash("sha256").update(new Uint8Array(await (await fetch(`${HF}/${id}/resolve/${info.sha}/${s.rfilename}`)).arrayBuffer())).digest("hex");
			files.push([s.rfilename, s.lfs?.size ?? s.size, `sha256:${sha}`, s.lfs ? 1 : 0]);
		}
		const r = await publishModel({ id, revision: info.sha, license: info.cardData?.license, files }, objects);
		const cat = await publishCatalog(before.catalog.models, before.catalog.snapshot, objects, before, flag("--site"));
		console.log(JSON.stringify({ id, revision: info.sha, ...r, catalog: cat.id }));
	},

	// discover <text> [--search]: default reads the catalog (two fetches by address). --search asks the server instead.
	async discover([text = "", mode]) {
		const t0 = Date.now();
		let ids;
		if (mode === "--search") ids = [...new Set((await search({ kind: "model-hub.model", filename_contains: text })).map((m) => split(m.filename)[0]))];
		else ids = Object.keys((await head()).catalog.objects).filter((id) => id.toLowerCase().includes(text.toLowerCase()));
		console.log(JSON.stringify({ query: text, via: mode === "--search" ? "server search" : "catalog", models: ids.length, ms: Date.now() - t0 }));
		for (const id of ids.slice(0, 12)) console.log(`  ${id}`);
	},

	// versions <id>: the verified chain, newest first.
	async versions([id]) {
		for (const v of await versions(id)) console.log(`${v.doc.revision}  ${v.meta.id}  prev=${v.doc.prev || "none"}  files=${v.doc.files.length}`);
	},

	// download <id> [--rev r] [--prefer ipfs|hologram|origin] [--out dir] [--only substring]
	// Name -> address -> sources -> bytes -> sha256 against the manifest. A mismatch is refused and the next source is tried.
	async download([id, ...flags]) {
		const flag = (n) => (flags.includes(n) ? flags[flags.indexOf(n) + 1] : undefined);
		const chain = await versions(id);
		if (!chain.length) throw new Error(`${id}: not in the hub`);
		const v = flag("--rev") ? chain.find((c) => c.doc.revision.startsWith(flag("--rev"))) : chain[0];
		const out = flag("--out") || join("out", id.replace("/", "__") + "@" + v.doc.revision.slice(0, 10));
		const sources = [];
		for (const sid of (await head()).catalog.objects[id].sources) { const s = parse(await get(sid)); if (s.model === v.meta.id) sources.push(s); }
		sources.push({ kind: "origin", resolve: `${HF}/${id}/resolve/${v.doc.revision}/` });
		const prefer = flag("--prefer");
		if (prefer) sources.sort((a, b) => (b.kind === prefer) - (a.kind === prefer));
		console.log(`model ${v.meta.id}\nrevision ${v.doc.revision}\nsources ${sources.map((s) => s.kind).join(", ")}`);
		const t0 = Date.now();
		let bytes = 0, kept = 0, refused = 0;
		const only = flag("--only");
		for (const f of v.doc.files.filter((x) => !only || x.path.includes(only))) {
			const dest = join(out, f.path);
			if (existsSync(dest) && statSync(dest).size === f.size && (await sha256File(dest)) === f.sha256) { kept++; continue; }
			mkdirSync(dirname(dest), { recursive: true });
			let ok = false;
			for (const s of sources) {
				try {
					const got = await fetchFile(s, f, dest);
					if (got !== f.sha256) { refused++; console.log(`  REFUSED ${f.path} from ${s.kind}: bytes hash to ${got.slice(0, 16)}…, the manifest says ${f.sha256.slice(0, 16)}…`); continue; }
					ok = true; bytes += f.size; console.log(`  ok ${f.path} ${f.size} B from ${s.kind}`); break;
				} catch (e) { console.log(`  ${s.kind} failed for ${f.path}: ${e.message}`); }
			}
			if (!ok) throw new Error(`${f.path}: no source served the right bytes. Nothing was kept for this file.`);
		}
		const s = (Date.now() - t0) / 1000;
		console.log(JSON.stringify({ out, files: v.doc.files.length, already_verified: kept, bytes, seconds: s, MBps: +(bytes / 1e6 / s).toFixed(1), refused }));
	},

	// host <id> <local dir> --peer <url> --peer-token <t>: put the files on ANY Hologram Server as chunks, announce it on the hub.
	async host([id, dir, ...flags]) {
		const flag = (n) => flags[flags.indexOf(n) + 1];
		const peer = { base: flag("--peer"), token: flag("--peer-token") };
		const v = (await versions(id))[0];
		const files = {};
		let total = 0;
		for (const f of v.doc.files) {
			const path = join(dir, f.path);
			if ((await sha256File(path)) !== f.sha256) throw new Error(`${f.path}: local bytes do not match the manifest; refusing to host them`);
			const buf = readFileSync(path);
			files[f.path] = [];
			for (let o = 0; o < buf.length || o === 0; o += CHUNK) {
				const meta = await put("model-hub.chunk", `${id}@${v.doc.revision.slice(0, 10)}/${f.path}#${o / CHUNK}`, buf.subarray(o, o + CHUNK), peer);
				files[f.path].push(meta.id);
				total += meta.size;
				if (buf.length === 0) break;
			}
		}
		const rec = await put("model-hub.source", `${id}@${v.doc.revision}`, json({
			format: "hologram.model-hub.source/v1", id, revision: v.doc.revision, model: v.meta.id,
			kind: "hologram", resolve: `${peer.base}/api/v1/objects/`, files,
		}));
		const before = await head();
		const objects = structuredClone(before.catalog.objects);
		if (!objects[id].sources.includes(rec.id)) objects[id].sources.push(rec.id);
		await publishCatalog(before.catalog.models, before.catalog.snapshot, objects, before, flag("--site"));
		console.log(JSON.stringify({ hosted: id, on: peer.base, bytes: total, chunks: Object.values(files).flat().length, source_record: rec.id }));
	},

	// source <id> <kind> <resolve url> --site dir: announce another place the head revision's bytes can be fetched.
	async source([id, kind, resolve, ...flags]) {
		const flag = (n) => (flags.includes(n) ? flags[flags.indexOf(n) + 1] : undefined);
		const before = await head();
		const objects = structuredClone(before.catalog.objects);
		const v = (await versions(id, undefined, objects[id].model))[0];
		const rec = await put("model-hub.source", `${id}@${v.doc.revision}`, json({ format: "hologram.model-hub.source/v1", id, revision: v.doc.revision, model: v.meta.id, kind, resolve }));
		objects[id].sources.push(rec.id);
		await publishCatalog(before.catalog.models, before.catalog.snapshot, objects, before, flag("--site"));
		console.log(JSON.stringify({ id, source_record: rec.id, kind, resolve }));
	},

	// mirror <from> <to> --to-token t: copy a whole hub with anonymous reads only. Start at its catalog, follow the addresses.
	// A second hub needs no coordination with the first: same bytes, same addresses.
	async mirror([from, to, ...flags]) {
		const flag = (n) => (flags.includes(n) ? flags[flags.indexOf(n) + 1] : undefined);
		const dest = { base: to, token: flag("--to-token") };
		const t0 = Date.now();
		const seen = new Set();
		let bytes = 0;
		const copy = async (kind, id, name) => {
			if (!id || seen.has(id)) return null;
			seen.add(id);
			const b = await get(id, { base: from });
			const doc = parse(b);
			const r = await put(kind, name(doc), b, dest);
			if (r.id !== id) throw new Error("address changed in transit");
			bytes += b.length;
			return doc;
		};
		const root = (await head({ base: from })).id;
		for (let c = root; c; ) {
			const cat = await copy("model-hub.catalog", c, (d) => `catalog@${d.snapshot}`);
			if (!cat) break;
			for (const o of Object.values(cat.objects)) {
				for (let m = o.model; m; ) { const d = await copy("model-hub.model", m, (x) => `${x.id}@${x.revision}`); m = d?.prev; }
				for (const s of o.sources) await copy("model-hub.source", s, (x) => `${x.id}@${x.revision}`);
			}
			c = cat.prev;
		}
		console.log(JSON.stringify({ mirrored: seen.size, bytes, root, seconds: (Date.now() - t0) / 1000 }));
	},
};

async function fetchFile(source, f, dest) {
	const hash = createHash("sha256");
	const w = createWriteStream(dest);
	const write = (chunk) => new Promise((res, rej) => { hash.update(chunk); w.write(chunk, (e) => (e ? rej(e) : res())); });
	if (source.kind === "hologram") {
		const ids = source.files?.[f.path];
		if (!ids) throw new Error("not hosted there");
		for (const cid of ids) await write(await get(cid, { base: source.resolve.replace(/\/api\/v1\/objects\/$/, "") }));
	} else {
		const url = source.resolve + f.path.split("/").map(encodeURIComponent).join("/");
		const r = await fetch(url, { redirect: "follow", signal: AbortSignal.timeout(source.kind === "ipfs" ? 90_000 : 600_000) });
		if (!r.ok) throw new Error(`${r.status}`);
		for await (const chunk of r.body) await write(chunk);
	}
	await new Promise((res) => w.end(res));
	return hash.digest("hex");
}

function sha256File(path) {
	return new Promise((res, rej) => {
		if (!existsSync(path)) return res(null);
		const h = createHash("sha256");
		createReadStream(path).on("data", (d) => h.update(d)).on("end", () => res(h.digest("hex"))).on("error", rej);
	});
}

if (import.meta.url === `file:///${process.argv[1].replace(/\\/g, "/")}` || process.argv[1].endsWith("hub.mjs")) {
	const [name, ...args] = process.argv.slice(2);
	if (!cmd[name]) { console.error(`usage: hub.mjs ${Object.keys(cmd).join("|")} …`); process.exit(2); }
	cmd[name](args).catch((e) => { console.error(String(e.message || e)); process.exit(1); });
}
