// Local preview for the model page's registry section: serves dist/ and forwards /v2/* to a live registry, so the
// page reads real artifacts from the same origin, as it does in production.
//
//   node qa/preview-oci.mjs [port] [registry origin]      default 8911, https://gethologram.ai
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const DIST = join(dirname(fileURLToPath(import.meta.url)), "..", "dist");
const PORT = Number(process.argv[2] || 8911);
const UPSTREAM = process.argv[3] || "https://gethologram.ai";
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".woff2": "font/woff2", ".txt": "text/plain" };

createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  if (url.pathname.startsWith("/v2/")) {
    const r = await fetch(UPSTREAM + url.pathname + url.search, { headers: { accept: req.headers.accept || "*/*" }, redirect: "manual" });
    const headers = {};
    for (const h of ["content-type", "docker-content-digest", "location", "content-length"]) if (r.headers.get(h)) headers[h] = r.headers.get(h);
    res.writeHead(r.status, headers);
    return res.end(Buffer.from(await r.arrayBuffer()));
  }
  let path = normalize(join(DIST, decodeURIComponent(url.pathname)));
  if (!path.startsWith(DIST)) return res.writeHead(403).end();
  try {
    if ((await stat(path)).isDirectory()) path = join(path, "index.html");
    res.writeHead(200, { "content-type": TYPES[extname(path)] || "application/octet-stream" });
    res.end(await readFile(path));
  } catch {
    res.writeHead(404, { "content-type": "text/html; charset=utf-8" }).end(await readFile(join(DIST, "404.html")).catch(() => "not found"));
  }
}).listen(PORT, () => console.log(`preview on http://localhost:${PORT}, /v2/ -> ${UPSTREAM}`));
