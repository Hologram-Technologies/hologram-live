// The gate that keeps public/openapi.json honest, in both directions.
//
//   forward  every operation the document describes is called against the live hub, and its status, media type and
//            response body are checked against what the document promises.
//   reverse  every route the live hub answers is matched against a path in the document. A route that answers and is
//            not described fails the gate: an endpoint with undescribed surface is an endpoint an agent must guess at.
//
//   node conformance.mjs [--base https://gethologram.ai] [--spec ../../public/openapi.json]
//
// Exit code 0 only when both directions pass. Read-only: no token is sent and no weight byte is fetched.
import { readFile } from "node:fs/promises";
import Ajv from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

const arg = (name, fallback) => { const i = process.argv.indexOf(`--${name}`); return i > -1 ? process.argv[i + 1] : fallback; };
const BASE = arg("base", "https://gethologram.ai").replace(/\/$/, "");
const SPEC = arg("spec", new URL("../../public/openapi.json", import.meta.url));
const EVIDENCE = new URL("./evidence.json", import.meta.url);

// Routes that answer nothing today, with the reason each is expected to. Anything else that answers must be described.
const KNOWN_ABSENT = {
  "/api/v1": "reserved: the object API lives under /api/v1/objects and the bare prefix is deliberately closed",
  "/hologram.live.v1.HologramLive/Handshake": "gRPC is closed at the edge; system.shutdown travels on it",
  "/wallpapers/": "the site links it but nothing serves it (open defect)",
};

const pass = [], fail = [];
const ok = (what, detail = "") => pass.push(`${what}${detail ? ` — ${detail}` : ""}`);
const bad = (what, detail) => fail.push(`${what} — ${detail}`);

// ---- schema validation -------------------------------------------------------------------------

function validator(spec) {
  const ajv = new Ajv({ strict: false, allErrors: true, validateFormats: true });
  addFormats(ajv);
  ajv.addSchema({ $id: "spec", components: spec.components }, "spec");
  return (schema, value) => {
    // $ref inside the document points at #/components/schemas/...; rebase them onto the registered document.
    const rebased = JSON.parse(JSON.stringify(schema).replaceAll('"#/components/schemas/', '"spec#/components/schemas/'));
    const check = ajv.compile(rebased);
    return check(value) ? null : ajv.errorsText(check.errors, { separator: "; " }).slice(0, 300);
  };
}

// ---- forward: the document is called ----------------------------------------------------------

async function forward(spec) {
  const validate = validator(spec);
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(item)) {
      const p = op["x-hologram-probe"];
      if (!p) continue;
      const want = p.status ?? 200;
      const init = { method: (p.method || method).toUpperCase(), headers: p.headers || {}, redirect: "manual", signal: AbortSignal.timeout(25_000) };
      if (p.body !== undefined) init.body = typeof p.body === "string" ? p.body : JSON.stringify(p.body);
      let res;
      try {
        res = await fetch(`${BASE}${p.path}`, init);
      } catch (e) {
        bad(`${op.operationId}`, `${p.path} did not answer: ${String(e.message || e)}`);
        continue;
      }
      if (res.status !== want) { bad(op.operationId, `${p.path} answered ${res.status}, the document promises ${want}`); continue; }

      const declared = op.responses?.[String(want)]?.content || {};
      const type = (res.headers.get("content-type") || "").split(";")[0].trim();
      if (p.contentType && type !== p.contentType) { bad(op.operationId, `${p.path} answered ${type || "no media type"}, expected ${p.contentType}`); continue; }
      if (type && Object.keys(declared).length && !declared[type] && !declared[`${type}; charset=utf-8`]) {
        bad(op.operationId, `${p.path} answered ${type}, which the document does not list for ${want}`);
        continue;
      }
      const schema = declared[type]?.schema;
      if (init.method !== "HEAD" && schema && type === "application/json") {
        const value = await res.json().catch(() => undefined);
        if (value === undefined) { bad(op.operationId, "the body is not JSON"); continue; }
        const problem = validate(schema, value);
        if (problem) { bad(op.operationId, `the live body does not match the documented schema: ${problem}`); continue; }
      }
      ok(op.operationId, `${p.path} → ${res.status}`);
    }
  }
}

// ---- reverse: the hub is swept ----------------------------------------------------------------

// A path template becomes a matcher. A parameter marked multi-segment may swallow slashes; every other one may not.
function matchers(spec) {
  return Object.entries(spec.paths).map(([template, item]) => {
    const multi = new Set();
    for (const op of Object.values(item)) for (const param of op.parameters || []) if (param["x-hologram-multi-segment"]) multi.add(param.name);
    const pattern = template.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/\\\{([^}]+)\\\}/g, (_, name) => (multi.has(name) ? "(.+)" : "([^/]+)"));
    return { template, re: new RegExp(`^${pattern}$`) };
  });
}

async function reverse(spec, evidence) {
  const all = matchers(spec);
  for (const record of evidence.records) {
    const path = record.path.split("?")[0];
    if (record.status === null) { bad("sweep", `${path} did not answer at all`); continue; }
    const described = all.find((m) => m.re.test(path));
    const absent = KNOWN_ABSENT[path];

    if (record.status < 400) {
      if (!described) bad("sweep", `${path} answers ${record.status} but no path in the document describes it`);
      else if (absent) bad("sweep", `${path} is recorded as absent (${absent}) but answers ${record.status}`);
      else ok("sweep", `${path} → ${record.status}, described by ${described.template}`);
      continue;
    }
    // A 404 on a described path is only acceptable where the document itself says the path may not be deployed yet.
    if (described && record.status === 404 && !absent) {
      const item = spec.paths[described.template];
      const declares404 = Object.values(item).some((op) => op.responses?.["404"]);
      if (declares404) ok("sweep", `${path} → 404, and ${described.template} documents a 404`);
      else bad("sweep", `${path} answers 404 but ${described.template} does not document one`);
      continue;
    }
    if (absent) ok("sweep", `${path} → ${record.status}, expected absent: ${absent}`);
    else ok("sweep", `${path} → ${record.status}, a refusal the probe asked for`);
  }
}

// ---- run --------------------------------------------------------------------------------------

async function main() {
  const spec = JSON.parse(await readFile(SPEC, "utf8"));
  const evidence = JSON.parse(await readFile(EVIDENCE, "utf8"));
  const age = (Date.now() - Date.parse(evidence.probed_at)) / 3_600_000;
  if (age > 24) bad("evidence", `web/qa/openapi/evidence.json is ${Math.round(age)} h old: run probe.mjs first`);

  await forward(spec);
  await reverse(spec, evidence);

  for (const line of pass) console.log(`  ok   ${line}`);
  for (const line of fail) console.log(`  FAIL ${line}`);
  console.log(`\n${pass.length} passed, ${fail.length} failed, against ${BASE}`);
  process.exit(fail.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
