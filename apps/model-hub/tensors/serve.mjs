// Local rehearsal of the hub's tensor routes, and optionally the built site on the same origin (as it is live).
//   TENSOR_STATE=<state dir> [DIST=../web/dist] [PORT=8600] node tensors/serve.mjs
import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, extname, normalize } from "node:path";
import { tensorMirror, tensorMirrorRoot, modelPageRedirect } from "../deploy/tensor-mirror.mjs";

const DIST = process.env.DIST;
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json",
  ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".woff2": "font/woff2", ".md": "text/markdown; charset=utf-8", ".txt": "text/plain" };

async function serveStatic(res, path) {
  let p = normalize(join(DIST, decodeURIComponent(path)));
  if (!p.startsWith(normalize(DIST))) return false;
  try {
    if ((await stat(p)).isDirectory()) p = join(p, "index.html");
    const b = await readFile(p);
    res.writeHead(200, { "content-type": TYPES[extname(p)] || "application/octet-stream" }); res.end(b); return true;
  } catch { return false; }
}

http.createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  const t0 = Date.now();
  res.on("close", () => { if (path.startsWith("/v2/")) console.log(req.method, req.url, req.headers.range || "", res.statusCode, res.getHeader("x-hub-source") || "", `${Date.now() - t0} ms`); });
  if (path === "/v2/" || path === "/v2") { res.writeHead(200, { "docker-distribution-api-version": "registry/2.0", "content-type": "application/json" }); return res.end("{}"); }
  if (await tensorMirror(req, res, path)) return;
  if (await tensorMirrorRoot(req, res, path)) return;
  if (DIST && await serveStatic(res, path)) return;
  if (await modelPageRedirect(req, res, path)) return;             // the plain address, opened in a browser
  res.writeHead(404); res.end();
}).listen(+(process.env.PORT || 8600), "127.0.0.1", () => console.log(`tensor mirror${DIST ? " + site" : ""} on :${process.env.PORT || 8600}`));
