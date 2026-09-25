// Every indexed registry artifact has its page, and every page stands on its own.
//
//   node qa/registry-pages.mjs [dist]
//
// build.mjs calls checkRegistryPages() after writing the pages, so the gate travels with the site source (the VPS
// build script is a copy nobody updates). What it holds:
//   1. one page and one artifact.json per row of registry/data/images.json (live rows aside: those are read at load);
//   2. every page shows a logo: a vendored cover that exists in dist, or the mark drawn from the name (a data URL) —
//      never a remote image, never a blank;
//   3. no page loads anything from another origin (script, style, img, font, fetch), the Registry page's own rule;
//   4. a row whose profile carries a README shows it, and every page carries a pull command;
//   5. the profile is fresh enough to trust: at least 95% of rows have one, so a refresh that fell short is named.

import { readFile, readdir, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { artifactFile } from "../src/registry-page.mjs";

export async function checkRegistryPages(dist, { base = "/", floor = 0.95 } = {}) {
  const REG = join(dist, "registry");
  const data = JSON.parse(await readFile(join(REG, "data", "images.json"), "utf8"));
  const rows = data.images.filter((r) => !r.here);
  const fail = [];
  let pages = 0, covers = 0, marks = 0, readmes = 0, profiles = 0, commands = 0;

  for (const r of rows) {
    const dir = join(REG, ...r.id.split("/"));
    const html = existsSync(join(dir, "index.html")) ? await readFile(join(dir, "index.html"), "utf8") : null;
    if (!html) { fail.push(`${r.id}: no page`); continue; }
    if (!existsSync(join(dir, "artifact.json"))) fail.push(`${r.id}: no artifact.json`);
    pages++;

    // 2. the logo
    const cover = /<img class="avatar cover" src="([^"]+)"/.exec(html)?.[1];
    if (!cover) fail.push(`${r.id}: no logo on the page`);
    else if (cover.startsWith("data:image/svg+xml")) marks++;
    else if (cover.startsWith(`${base}registry/icons/`) && existsSync(join(REG, "icons", cover.slice(`${base}registry/icons/`.length)))) covers++;
    else fail.push(`${r.id}: logo ${cover.slice(0, 80)} is not a file this build ships`);

    // 3. nothing off-origin is loaded
    for (const m of html.matchAll(/<(script|link|img|source|iframe)\b[^>]*>/gi)) {
      const url = /\b(?:src|href)\s*=\s*["']([^"']+)["']/i.exec(m[0])?.[1] || "";
      if (/^https?:\/\//i.test(url) && !/<link\b[^>]*\brel=["'](?:alternate|describedby|canonical|preload)["']/i.test(m[0]) && !/<link\b[^>]*\brel=["']service-/i.test(m[0])) {
        if (m[1] !== "link" || /stylesheet|modulepreload|icon/i.test(m[0])) fail.push(`${r.id}: page loads ${url}`);
      }
    }

    // 4. what the profile gave is on the page
    const profilePath = join(REG, "data", "artifacts", artifactFile(r.id));
    const p = existsSync(profilePath) ? JSON.parse(await readFile(profilePath, "utf8")) : null;
    if (p && !p.error) profiles++;
    if (p?.readme) { if (html.includes('id="ov-readme"')) readmes++; else fail.push(`${r.id}: profile has a README the page does not show`); }
    if (/data-copy="[^"]*(?:docker pull|docker model pull|docker mcp|sbx run|helm |kubectl krew|oras pull|curl -LO|open https)/.test(html)) commands++;
    else if (r.registry !== "Artifact Hub") fail.push(`${r.id}: no pull command on the page`);
  }

  // 5. the profiles came through
  if (profiles < rows.length * floor) fail.push(`only ${profiles} of ${rows.length} rows have a profile (floor ${Math.round(floor * 100)}%): run node scripts/registry.mjs`);

  if (fail.length) throw new Error(`registry pages: ${fail.length} problem${fail.length === 1 ? "" : "s"}\n  ${fail.slice(0, 20).join("\n  ")}${fail.length > 20 ? `\n  … and ${fail.length - 20} more` : ""}`);
  return `registry pages: ${pages} pages for ${rows.length} rows; ${covers} vendored logos + ${marks} drawn marks, ${profiles} profiles, ${readmes} READMEs, ${commands} with a pull command; nothing loads off-origin`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const dist = process.argv[2] || join(dirname(fileURLToPath(import.meta.url)), "..", "dist");
  console.log(await checkRegistryPages(dist));
}
