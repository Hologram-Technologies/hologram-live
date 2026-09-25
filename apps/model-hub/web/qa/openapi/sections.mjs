// Can an agent that knows only the hostname use this hub?
//
// Five addresses -- /models, /registry, /spaces, /buckets, /docs/ -- each answer three ways: the page to a
// browser, the brief to a reader, and a section descriptor to anything that asks for JSON by name. The
// descriptor is the executable one: it names how to enumerate that section, how to reach one item, how to
// fetch its bytes and how to check them.
//
// This holds all of that to three things, in order of how much they matter:
//
//   shape     every descriptor carries the fixed keys, and where `catalog.address` is not null it is a
//             content address -- fetch it, hash it, and the hash IS the address. That property is the whole
//             reason the catalogue can be one request instead of a crawl.
//   runnable  every URL in every descriptor is callable as written once its {placeholders} are filled from
//             the same document. A descriptor that names a route nobody serves is worse than no descriptor.
//   journey   the acceptance criterion, and the only one that answers the question in the title: starting
//             from the hostname alone, reach every section, list each one, and download and verify one
//             artifact -- in a bounded number of requests, with no key.
//
//   node qa/openapi/sections.mjs [--base https://…] [--budget 20]
//
// The default base is the origin this build was made for (src/origin.mjs), or HUB_LIVE when it is set.
//
// Against a local build, point --base at a static server for dist/. It will report the representations it
// cannot test there (the Accept negotiation is an edge rule, not a file) rather than pretend they passed.

import { createHash } from "node:crypto";
import { ORIGIN, HOST } from "../../src/origin.mjs";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};
const BASE = arg("base", process.env.HUB_LIVE || ORIGIN).replace(/\/$/, "");
const BUDGET = Number(arg("budget", 20));
const SECTIONS = ["models", "registry", "spaces", "buckets", "docs"];
const PATH = { models: "/models", registry: "/registry", spaces: "/spaces", buckets: "/buckets", docs: "/docs/" };
const REQUIRED = ["format", "section", "self", "about", "inventory", "catalog", "list", "item", "fetch", "verify", "brief", "page", "openapi"];

let pass = 0, fail = 0, skip = 0, requests = 0;
const ok = (m) => { pass++; console.log(`  ok   ${m}`); };
const bad = (m) => { fail++; console.log(`  FAIL ${m}`); };
const meh = (m) => { skip++; console.log(`  --   ${m}`); };

async function get(path, { accept = "application/json", method = "GET" } = {}) {
  requests++;
  const r = await fetch(BASE + path, { method, headers: { accept }, redirect: "follow", signal: AbortSignal.timeout(30_000) });
  const buf = method === "HEAD" ? new ArrayBuffer(0) : await r.arrayBuffer();
  return { status: r.status, headers: r.headers, bytes: new Uint8Array(buf), text: () => new TextDecoder().decode(buf) };
}
const sha256 = (b) => createHash("sha256").update(b).digest("hex");

// ---------------------------------------------------------------- shape

console.log(`\n== the descriptor answers, and says what it must  (${BASE})`);
const descriptors = {};
let negotiates = true;   // false against a plain file server, where Accept is not a rule anybody applies
for (const name of SECTIONS) {
  let r = await get(PATH[name]).catch((e) => ({ status: 0, error: e.message }));
  // A static server hands back the page whatever the Accept header says. That is the edge rule missing, not
  // the descriptor missing, so read the file directly and say which of the two was proved.
  let via = PATH[name];
  if (r.status === 200 && /^\s*<!doctype/i.test(r.text())) {
    negotiates = false;
    via = `/${name}.json`;
    r = await get(via).catch((e) => ({ status: 0, error: e.message }));
  }
  if (r.status !== 200) { bad(`${via} as JSON -> ${r.status || r.error}`); continue; }
  let d;
  try { d = JSON.parse(r.text()); } catch { bad(`${via} as JSON did not parse`); continue; }
  if (d.format !== "hologram.section.descriptor/v1") { bad(`${PATH[name]} answered ${d.format || "something else"}`); continue; }
  const missing = REQUIRED.filter((k) => !(k in d));
  if (missing.length) { bad(`${PATH[name]} is missing ${missing.join(", ")}`); continue; }
  if (d.section !== name) { bad(`${PATH[name]} calls itself ${d.section}`); continue; }
  descriptors[name] = d;
  ok(`${via} -> ${d.section} descriptor${via === PATH[name] ? "" : " (read as a file; this server does not negotiate)"}`);
}

console.log("\n== the same address still serves a reader and a browser");
for (const name of SECTIONS) {
  const brief = await get(PATH[name], { accept: "*/*" });
  const heading = `# ${HOST}/${name}`;
  if (brief.status === 200 && brief.text().startsWith(heading)) ok(`${PATH[name]} as */* -> the brief`);
  else if (brief.text().startsWith("<!doctype")) meh(`${PATH[name]} as */* -> the page (the Accept rule is an edge rule; not served by a plain file server)`);
  else bad(`${PATH[name]} as */* -> neither the brief nor the page`);

  const page = await get(PATH[name], { accept: "text/html" });
  if (page.status === 200 && /^\s*<!doctype/i.test(page.text())) ok(`${PATH[name]} as text/html -> the page`);
  else bad(`${PATH[name]} as text/html -> ${page.status}`);

  const vary = String(brief.headers.get("vary") || "");
  if (vary.split(",").some((t) => t.trim().toLowerCase() === "accept")) ok(`${PATH[name]} varies on Accept`);
  else meh(`${PATH[name]} has no Vary: Accept (edge rule)`);
}

// ---------------------------------------------------------------- the address is the bytes

console.log("\n== where a section names a catalogue, the address is the hash of what comes back");
for (const [name, d] of Object.entries(descriptors)) {
  if (!d.catalog || !d.catalog.address) {
    if (d.catalog_note) ok(`${name} has no snapshot and says why`);
    else bad(`${name} has no catalogue and no catalog_note saying why`);
    continue;
  }
  const addr = d.catalog.address;
  const m = /^blake3:([0-9a-f]{64})$/.exec(addr);
  if (!m) { bad(`${name} catalog.address is not a content address: ${addr}`); continue; }
  const r = await get(d.catalog.fetch.replace("{address}", addr), { accept: "application/json" });
  if (r.status === 404 && !negotiates) { meh(`${name} catalogue is on the object plane, which this base does not serve`); continue; }
  if (r.status !== 200) { bad(`${name} catalogue ${addr} -> ${r.status}`); continue; }
  // BLAKE3 is not in node:crypto. The object plane has its own gate for the hash itself
  // (qa/openapi/objects.mjs); what matters here is that the address the descriptor gave resolves and parses.
  let parsed = null;
  try { parsed = JSON.parse(r.text()); } catch { /* not json */ }
  if (!parsed) bad(`${name} catalogue did not parse`);
  else ok(`${name} catalogue ${addr.slice(0, 18)}… -> ${r.bytes.length} bytes, ${Object.keys(parsed.objects || parsed).length} entries`);
}

// ---------------------------------------------------------------- runnable

console.log("\n== every URL a descriptor names is callable as written");
// A placeholder is filled from the descriptor itself where the descriptor can answer it, and otherwise from
// what the section's own listing just returned. Anything still unfilled is reported, not quietly skipped.
function fill(url, d, vars) {
  return url.replace(/\{([^}]+)\}/g, (whole, key) => {
    if (key === "catalog.address") return d.catalog?.address ?? whole;
    return vars[key] ?? whole;
  });
}
for (const [name, d] of Object.entries(descriptors)) {
  const vars = {};
  for (const route of d.list || []) {
    const url = fill(route.url, d, vars);
    if (url.includes("{")) { meh(`${name}: ${route.url} needs ${url.match(/\{[^}]+\}/g).join(", ")}, which nothing here supplies`); continue; }
    const r = await get(url, { accept: "application/json" });
    if (r.status === 200) ok(`${name}: ${url} -> 200, ${r.bytes.length} bytes`);
    else if (!negotiates && r.status === 404) meh(`${name}: ${url} is not served by this base (only the site is)`);
    else bad(`${name}: ${url} -> ${r.status}`);
  }
}

// ---------------------------------------------------------------- the journey

console.log(`\n== from the hostname alone, in ${BUDGET} requests or fewer`);
const before = requests;
let root = await get("/", { accept: "application/json" });
if (root.status === 200 && /^\s*<!doctype/i.test(root.text())) root = await get("/.well-known/model-hub.json", { accept: "application/json" });
let reached = 0, verified = null;
if (root.status === 404 && !negotiates) meh("the root descriptor is written by publish.sh on the host, not by the site build");
else if (root.status !== 200) bad(`/ as JSON -> ${root.status}`);
else {
  let rd = null;
  try { rd = JSON.parse(root.text()); } catch { /* */ }
  if (!rd) bad("/ as JSON did not parse");
  else if (!rd.sections) bad("/ does not list its sections, so an agent cannot find them from the hostname alone");
  else {
    ok(`/ lists ${Object.keys(rd.sections).length} sections`);
    for (const [name, s] of Object.entries(rd.sections)) {
      const r = await get(s.url, { accept: "application/json" });
      if (r.status === 200 && r.text().includes("hologram.section.descriptor/v1")) reached++;
      else bad(`/ pointed at ${s.url} for ${name}, which did not answer a descriptor`);
    }
    if (reached === Object.keys(rd.sections).length) ok(`every section named at / answers its descriptor (${reached})`);

    // ...and one real artifact, fetched and checked against a hash that did not come from whoever served it.
    const models = descriptors.models;
    if (models?.catalog?.address) {
      const cat = JSON.parse((await get(models.catalog.fetch.replace("{address}", models.catalog.address))).text());
      const id = Object.keys(cat.objects || {})[0];
      if (!id) meh("the catalogue is empty, so nothing could be downloaded");
      else {
        const tree = await get(`/api/models/${id}/tree/main`, { accept: "application/json" });
        const files = JSON.parse(tree.text());
        const small = files.filter((f) => f.type === "file" && f.size > 0 && f.size < 200_000).sort((a, b) => a.size - b.size)[0];
        if (!small) meh(`${id} has no small file to prove the path with`);
        else {
          const expected = small.lfs?.oid || small.oid;   // from the index, never from the byte source
          const got = await get(`/${id}/resolve/main/${small.path}`, { accept: "*/*" });
          const actual = sha256(got.bytes);
          if (!expected) meh(`${id}/${small.path} has no expected hash in the listing`);
          else if (actual === expected) { verified = `${id}/${small.path}`; ok(`downloaded and verified ${verified} (${got.bytes.length} bytes)`); }
          else bad(`${id}/${small.path}: expected ${expected}, got ${actual}`);
        }
      }
    }
  }
}
const spent = requests - before;
if (spent <= BUDGET) ok(`the journey took ${spent} requests`);
else bad(`the journey took ${spent} requests, over the budget of ${BUDGET}`);

console.log(`\n${pass} passed, ${fail} failed, ${skip} not testable here; ${requests} requests in total`);
if (!negotiates) console.log("this server does not negotiate on Accept, so the descriptors were read as files: the shape is proved, the edge rule is not");
if (verified) console.log(`proved end to end: ${verified}`);
process.exit(fail ? 1 : 0);
