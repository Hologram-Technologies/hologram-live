// The κ mirror: every OCI image the Registry page indexes, pullable from this origin by its upstream digest.
//
//   GET|HEAD /v2/<host>/<path>/manifests/<tag|sha256:…>   the upstream manifest, byte for byte (so its digest,
//                                                         and every signature over it, is the upstream's own)
//   GET|HEAD /v2/<host>/<path>/blobs/<sha256:…>            configs from here; layers 307 to the upstream's own
//                                                         blob location (its signed CDN URL where it has one)
//   GET      /v2/<host>/<path>/tags/list
//   GET      /v2/<host>/<path>/referrers/<sha256:…>        provenance: where and when this κ was resolved
//
// <host> is the upstream registry (docker.io, mcr.microsoft.com, ghcr.io, …), so the name maps back to its
// source with no table. Nothing here decides trust: a client asks for a digest and hashes what arrives. The
// digest itself equals what the upstream registry reports, and is committed to by the dated sealed index.
//
// State (built by registry-kappa/pack.mjs, synced to $HUB_STATE/kappa/):
//   mirror.json          { repos: { "<host>/<path>": { tags: {tag: digest}, manifests: [digest], layers: {digest: [size, mediaType]}, referrers: [digest] } } }
//   sha256/<hex>         every manifest, config and referrer this mirror serves; hashed on load
import { readFile, stat } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";

const ROOT = join(process.env.HUB_STATE || "/state", "kappa");
const HOSTS = /^(docker\.io|mcr\.microsoft\.com|ghcr\.io|quay\.io|registry\.k8s\.io|gcr\.io|[a-z0-9-]+\.gcr\.io|public\.ecr\.aws|registry\.werf\.io|registry\.gitlab\.com|[a-z0-9.-]+\.azurecr\.io)\//;
const API_HOST = (h) => (h === "docker.io" ? "registry-1.docker.io" : h);

let mirror = { at: 0, mtime: 0, repos: {} };
async function load() {
  if (Date.now() - mirror.at < 30_000) return mirror.repos;
  mirror.at = Date.now();
  try {
    const s = await stat(join(ROOT, "mirror.json"));
    if (s.mtimeMs !== mirror.mtime) {
      mirror.repos = JSON.parse(await readFile(join(ROOT, "mirror.json"), "utf8")).repos;
      mirror.mtime = s.mtimeMs;
    }
  } catch { /* no mirror published yet */ }
  return mirror.repos;
}

const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;
async function held(digest) {
  if (!/^sha256:[0-9a-f]{64}$/.test(digest)) return null;
  try {
    const b = await readFile(join(ROOT, "sha256", digest.slice(7)));
    return sha(b) === digest ? b : null; // a corrupted file is treated as absent, never served
  } catch { return null; }
}

const ociError = (res, status, code, message) => {
  const t = JSON.stringify({ errors: [{ code, message }] });
  res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "cache-control": "no-store", "docker-distribution-api-version": "registry/2.0" });
  res.end(res.req.method === "HEAD" ? undefined : t);
};

function send(res, bytes, type, digest, immutable) {
  res.writeHead(200, {
    "content-type": type, "content-length": bytes.length, "docker-content-digest": digest, etag: `"${digest}"`,
    "docker-distribution-api-version": "registry/2.0",
    "cache-control": immutable ? "public, max-age=31536000, immutable" : "public, max-age=300",
  });
  res.end(res.req.method === "HEAD" ? undefined : bytes);
}

// Upstream pull tokens, held here so a client never talks to the upstream's auth server.
const tokens = new Map();
async function upstreamFetch(host, repo, path, method) {
  const url = `https://${API_HOST(host)}/v2/${repo}/${path}`;
  const key = `${host}/${repo}`;
  for (let attempt = 0; attempt < 2; attempt++) {
    const t = tokens.get(key);
    const r = await fetch(url, { method, redirect: "manual", headers: { "user-agent": "hologram-kappa-mirror/1", ...(t && Date.now() < t.until ? { authorization: `Bearer ${t.token}` } : {}) }, signal: AbortSignal.timeout(15_000) });
    if (r.status !== 401 || attempt) return r;
    const m = /Bearer realm="([^"]+)"(?:,service="([^"]+)")?/i.exec(r.headers.get("www-authenticate") || "");
    if (!m) return r;
    const u = new URL(m[1]);
    if (m[2]) u.searchParams.set("service", m[2]);
    u.searchParams.set("scope", `repository:${repo}:pull`);
    const tr = await fetch(u, { signal: AbortSignal.timeout(10_000) });
    if (!tr.ok) return r;
    const j = await tr.json();
    tokens.set(key, { token: j.token || j.access_token, until: Date.now() + Math.max(60, (j.expires_in || 300) - 30) * 1000 });
  }
}

// Returns true when the request was a mirror route (answered), false to let the caller continue.
export async function kappaMirror(req, res, path) {
  const m = path.match(/^\/v2\/(.+)\/(manifests|blobs|tags|referrers)\/([^/]+)$/);
  if (!m || !HOSTS.test(m[1] + "/")) return false;
  const [, name, kind, ref] = m;
  const repos = await load();
  const r = repos[name];
  if (!r) { ociError(res, 404, "NAME_UNKNOWN", `${name} is not in the κ mirror; see https://hub.uor.foundation/registry/`); return true; }

  if (kind === "tags") {
    const t = JSON.stringify({ name, tags: Object.keys(r.tags).sort() });
    res.writeHead(200, { "content-type": "application/json", "content-length": Buffer.byteLength(t), "cache-control": "public, max-age=300" });
    res.end(req.method === "HEAD" ? undefined : t);
    return true;
  }

  if (kind === "referrers") {
    const out = [];
    for (const d of r.referrers || []) {
      const b = await held(d);
      if (!b) continue;
      const mm = JSON.parse(b.toString("utf8"));
      if (mm.subject?.digest !== ref) continue;
      out.push({ mediaType: mm.mediaType, digest: d, size: b.length, artifactType: mm.artifactType || mm.config?.mediaType, ...(mm.annotations ? { annotations: mm.annotations } : {}) });
    }
    const t = JSON.stringify({ schemaVersion: 2, mediaType: "application/vnd.oci.image.index.v1+json", manifests: out });
    res.writeHead(200, { "content-type": "application/vnd.oci.image.index.v1+json", "content-length": Buffer.byteLength(t), "cache-control": "public, max-age=300" });
    res.end(req.method === "HEAD" ? undefined : t);
    return true;
  }

  if (kind === "manifests") {
    const digest = ref.startsWith("sha256:") ? ref : r.tags[ref];
    if (!digest || !(r.manifests.includes(digest) || (r.referrers || []).includes(digest))) { ociError(res, 404, "MANIFEST_UNKNOWN", `${name}:${ref} is not mirrored`); return true; }
    const b = await held(digest);
    if (!b) { ociError(res, 404, "MANIFEST_UNKNOWN", `${digest} is indexed but not held`); return true; }
    const mm = JSON.parse(b.toString("utf8"));
    const type = mm.mediaType || (mm.manifests ? "application/vnd.oci.image.index.v1+json" : "application/vnd.oci.image.manifest.v1+json");
    send(res, b, type, digest, ref.startsWith("sha256:"));
    return true;
  }

  // Blobs: held (configs, small) from here; known layers by redirect; anything else is unknown.
  const b = await held(ref);
  if (b) { send(res, b, "application/octet-stream", ref, true); return true; }
  const layer = r.layers[ref];
  if (!layer) { ociError(res, 404, "BLOB_UNKNOWN", `${ref} is not a layer of ${name}`); return true; }
  const [size] = layer;
  const [host, ...rest] = name.split("/");
  if (req.method === "HEAD") {
    // Answered directly: a cross-host redirect on HEAD is refused by some clients.
    res.writeHead(200, { "content-length": size, "docker-content-digest": ref, "content-type": "application/octet-stream", "cache-control": "public, max-age=31536000, immutable" });
    res.end();
    return true;
  }
  let up;
  try { up = await upstreamFetch(host, rest.join("/"), `blobs/${ref}`, "GET"); } catch (e) { ociError(res, 502, "UNAVAILABLE", `upstream ${host}: ${e.message}`); return true; }
  const loc = up.headers.get("location");
  if (up.status >= 300 && up.status < 400 && loc) {
    up.body?.cancel();
    // The upstream's own signed CDN URL: no credential in it that the client could not have got itself.
    res.writeHead(307, { location: new URL(loc, `https://${API_HOST(host)}`).href, "docker-content-digest": ref, "x-hub-source": host, "cache-control": "no-store", "content-length": "0" });
    res.end();
    return true;
  }
  if (up.ok) {
    // The upstream serves the bytes itself and needs our token, so they pass through. The client still hashes them.
    res.writeHead(200, { "content-type": "application/octet-stream", "content-length": up.headers.get("content-length") || size, "docker-content-digest": ref, "x-hub-source": `${host} (relayed)`, "cache-control": "no-store" });
    for await (const chunk of up.body) res.write(chunk);
    res.end();
    return true;
  }
  up.body?.cancel();
  ociError(res, 502, "UNAVAILABLE", `upstream ${host} answered ${up.status} for ${ref}`);
  return true;
}
