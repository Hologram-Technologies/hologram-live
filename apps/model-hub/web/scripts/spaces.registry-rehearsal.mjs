// A minimal OCI distribution server for rehearsing the Spaces publish without the hub's write token.
// Blobs by digest, monolithic and chunked uploads, manifests by tag or digest, Docker-Content-Digest on
// every manifest response, `_catalog` and `tags/list`. Every uploaded blob is re-derived before it is kept
// (a wrong digest is 400), exactly as the hub's registry does. Nothing persists beyond the process.
//
//   node scripts/spaces.registry-rehearsal.mjs [--port 5055]
import http from "node:http";
import { createHash } from "node:crypto";

const PORT = Number(process.argv[process.argv.indexOf("--port") + 1] || 5055);
const blobs = new Map();       // "sha256:hex" → Buffer
const manifests = new Map();   // repo → Map(ref → { bytes, type, digest })
const uploads = new Map();     // id → { repo, chunks: [] }
const sha = (b) => "sha256:" + createHash("sha256").update(b).digest("hex");
const read = (req) => new Promise((res) => { const c = []; req.on("data", (d) => c.push(d)); req.on("end", () => res(Buffer.concat(c))); });
const json = (res, code, obj, extra = {}) => { const b = Buffer.from(JSON.stringify(obj)); res.writeHead(code, { "content-type": "application/json", "content-length": b.length, ...extra }); res.end(b); };

http.createServer(async (req, res) => {
  const u = new URL(req.url, "http://x"); const p = u.pathname;
  if (p === "/v2/" || p === "/v2") return json(res, 200, {}, { "docker-distribution-api-version": "registry/2.0" });
  if (p === "/v2/_catalog") return json(res, 200, { repositories: [...manifests.keys()].sort() });
  let m;
  if ((m = p.match(/^\/v2\/(.+)\/tags\/list$/))) return json(res, manifests.has(m[1]) ? 200 : 404, { name: m[1], tags: [...(manifests.get(m[1]) || new Map()).keys()].filter((t) => !t.startsWith("sha256:")).sort() });
  if ((m = p.match(/^\/v2\/(.+)\/blobs\/uploads\/$/)) && req.method === "POST") {
    const id = createHash("sha256").update(String(Math.random())).digest("hex").slice(0, 32); uploads.set(id, { repo: m[1], chunks: [] });
    res.writeHead(202, { location: `/v2/${m[1]}/blobs/uploads/${id}`, "docker-upload-uuid": id, range: "0-0" }); return res.end();
  }
  if ((m = p.match(/^\/v2\/(.+)\/blobs\/uploads\/([a-f0-9]+)$/))) {
    const up = uploads.get(m[2]); if (!up) { res.writeHead(404); return res.end(); }
    const body = await read(req); if (body.length) up.chunks.push(body);
    if (req.method === "PATCH") { res.writeHead(202, { location: `/v2/${m[1]}/blobs/uploads/${m[2]}`, range: `0-${up.chunks.reduce((a, c) => a + c.length, 0) - 1}` }); return res.end(); }
    if (req.method === "PUT") {
      const want = u.searchParams.get("digest"); const all = Buffer.concat(up.chunks); uploads.delete(m[2]);
      if (!want || sha(all) !== want) return json(res, 400, { errors: [{ code: "DIGEST_INVALID", message: `got ${sha(all)} want ${want}` }] });
      blobs.set(want, all); res.writeHead(201, { location: `/v2/${m[1]}/blobs/${want}`, "docker-content-digest": want }); return res.end();
    }
  }
  if ((m = p.match(/^\/v2\/(.+)\/blobs\/(sha256:[a-f0-9]{64})$/))) {
    const b = blobs.get(m[2]); if (!b) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { "content-type": "application/octet-stream", "content-length": b.length, "docker-content-digest": m[2] }); return res.end(req.method === "HEAD" ? undefined : b);
  }
  if ((m = p.match(/^\/v2\/(.+)\/manifests\/([^/]+)$/))) {
    const repo = m[1], ref = m[2];
    if (req.method === "PUT") {
      const bytes = await read(req); const digest = sha(bytes); const type = req.headers["content-type"] || "application/vnd.oci.image.manifest.v1+json";
      let doc; try { doc = JSON.parse(bytes.toString("utf8")); } catch { return json(res, 400, { errors: [{ code: "MANIFEST_INVALID" }] }); }
      for (const l of [doc.config, ...(doc.layers || [])]) if (l && !blobs.has(l.digest)) return json(res, 400, { errors: [{ code: "MANIFEST_BLOB_UNKNOWN", message: l.digest }] });
      if (!manifests.has(repo)) manifests.set(repo, new Map());
      manifests.get(repo).set(ref, { bytes, type, digest }); manifests.get(repo).set(digest, { bytes, type, digest });
      res.writeHead(201, { location: `/v2/${repo}/manifests/${digest}`, "docker-content-digest": digest }); return res.end();
    }
    const e = (manifests.get(repo) || new Map()).get(ref); if (!e) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { "content-type": e.type, "content-length": e.bytes.length, "docker-content-digest": e.digest }); return res.end(req.method === "HEAD" ? undefined : e.bytes);
  }
  res.writeHead(404); res.end();
}).listen(PORT, "127.0.0.1", () => console.log(`rehearsal registry http://127.0.0.1:${PORT}/v2/`));
