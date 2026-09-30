// Content-addressed store of the small objects the hub holds: literals, layouts, tensor tables, manifests,
// indexes, small files. Named by sha256; verified on every read, so a corrupted file is absent, never served.
import { mkdirSync, existsSync, readFileSync, writeFileSync, renameSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

export const sha = (b) => `sha256:${createHash("sha256").update(b).digest("hex")}`;

export class Store {
  constructor(root) { this.root = root; this.dir = join(root, "sha256"); mkdirSync(this.dir, { recursive: true }); }
  path(d) { return join(this.dir, d.slice(7)); }
  has(d) { return existsSync(this.path(d)); }
  put(buf) {
    const d = sha(buf), p = this.path(d);
    if (!existsSync(p)) { writeFileSync(p + ".tmp", buf); renameSync(p + ".tmp", p); }
    return { digest: d, size: buf.length };
  }
  putJson(obj) { return this.put(Buffer.from(JSON.stringify(obj))); }
  get(d) {
    try { const b = readFileSync(this.path(d)); return sha(b) === d ? b : null; } catch { return null; }
  }
  json(d) { const b = this.get(d); return b ? JSON.parse(b.toString("utf8")) : null; }
}
