// What each App's source says about it — likes, who published it, when it last changed — read from the Hugging
// Face Space it was sealed from and written into the catalog (public/spaces/spaces.json).
//
//   node scripts/spaces.meta.mjs
//
// The catalog is not part of any App's seal (each App's κ covers its own files only), so this can run every night
// without moving an address. A source that does not answer keeps the numbers it had, with the day they were read,
// so a stale figure is visible as stale rather than silently wrong. Nothing here runs in a reader's browser: the
// page shows what this wrote and fetches nothing from Hugging Face itself.

import { readFile, writeFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const CATALOG = join(dirname(fileURLToPath(import.meta.url)), "..", "public", "spaces", "spaces.json");
const UA = "hologram-apps-meta/1.0 (+https://gethologram.ai/spaces/)";

export async function refreshSourceMeta(file = CATALOG, { now = new Date() } = {}) {
  const catalog = JSON.parse(await readFile(file, "utf8"));
  let read = 0, kept = 0;
  for (const s of catalog.spaces) {
    const m = String(s.source || "").match(/^https:\/\/huggingface\.co\/spaces\/([^/]+\/[^/?#]+)/);
    if (!m) continue;
    try {
      const res = await fetch(`https://huggingface.co/api/spaces/${m[1]}`, { headers: { accept: "application/json", "user-agent": UA } });
      if (!res.ok) throw new Error(`answered ${res.status}`);
      const j = await res.json();
      s.sourceMeta = {
        likes: Number.isFinite(j.likes) ? j.likes : null,
        author: j.author || m[1].split("/")[0],
        updated: j.lastModified || null,
        read: now.toISOString().slice(0, 10),
        api: `https://huggingface.co/api/spaces/${m[1]}`,
      };
      read++;
    } catch (e) {
      kept++;
      if (s.sourceMeta) s.sourceMeta.error = String(e.message || e);
    }
  }
  await writeFile(file, JSON.stringify(catalog, null, 2) + "\n");
  return { read, kept, apps: catalog.spaces.length };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  console.log("apps source meta", JSON.stringify(await refreshSourceMeta()));
}
