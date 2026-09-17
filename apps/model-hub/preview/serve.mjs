// Preview the Model Hub View (apps/model-hub/ui, from `TARGET=holo node web/build.mjs`) against a simulated host.
// Not packed into the .holo.
//
//   node apps/model-hub/preview/serve.mjs   ->  http://127.0.0.1:8141/index.html
//
// Requests follow Desktop's portable View rules (apps/desktop/src-tauri/src/view_surface.rs): exact file paths
// only, so a query string or a directory path fails here the way it would in the application.

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const ui = join(here, "..", "ui");
const port = Number(process.env.PORT ?? 8141);
const types = {
  ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".mjs": "text/javascript",
  ".json": "application/json", ".woff2": "font/woff2", ".svg": "image/svg+xml", ".png": "image/png",
  ".jpg": "image/jpeg", ".webp": "image/webp",
};
const valid = (path) => path && !path.endsWith("/") && /^[A-Za-z0-9._/-]+$/.test(path) && !path.split("/").some((p) => !p || p === "." || p === "..");

createServer(async (request, response) => {
  const url = new URL(request.url, "http://x");
  let path = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
  if (url.search) return response.writeHead(400).end("invalid portable View URL (query string)");
  if (path === "_hologram/fake-host.js") path = join("..", "preview", "fake-host.js");
  else if (!valid(path)) return response.writeHead(400).end(`invalid portable View asset path ${JSON.stringify(path)}`);
  try {
    let body = await readFile(join(ui, path));
    if (path.endsWith(".html")) {
      body = body.toString().replace("<head>", '<head>\n<script src="/_hologram/fake-host.js"></script>');
    }
    response.writeHead(200, { "content-type": types[extname(path)] ?? "application/octet-stream" }).end(body);
  } catch {
    response.writeHead(404).end("not found");
  }
}).listen(port, "127.0.0.1", () => console.log(`Model Hub preview: http://127.0.0.1:${port}/index.html`));
