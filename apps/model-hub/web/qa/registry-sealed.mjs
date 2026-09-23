// The Registry page must reach no origin but its own.
//
//   node qa/registry-sealed.mjs [dist]
//
// and build.mjs calls checkSealed() at the end of every build, so the gate travels with the site
// source. The VPS build script is a separate copy that no clone updates; a check that lived only
// there would be a check nobody runs.
//
// Two reasons for the rule, and the second is the sharp one. The hub loads no third-party asset
// anywhere, so this page should not be the exception. And a per-card icon fetched from a CDN would
// tell that CDN which registry entries each visitor scrolled past: a behavioural log of our readers'
// interest, handed to someone else, on a page whose entire claim is that you can check what you were
// given. A hasher loaded from a CDN is worse still, because then the verification is on loan.
//
// Loads are what this checks: script, style, img, font, fetch, import. Links the reader may click are
// left alone, because the whole point of a card is to hand you back to the source it came from.

import { readFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";

export async function checkSealed(dist) {
  const REG = join(dist, "registry");
  const fail = [];

  for (const f of ["index.html", "app.js", "data/images.json", "vendor/blake3.umd.min.js"]) {
    if (!existsSync(join(REG, f))) fail.push(`missing registry/${f}`);
  }
  if (fail.length) throw new Error("registry page incomplete: " + fail.join("; "));

  const html = await readFile(join(REG, "index.html"), "utf8");
  const js = await readFile(join(REG, "app.js"), "utf8");
  const data = JSON.parse(await readFile(join(REG, "data", "images.json"), "utf8"));

  // markup: anything the browser loads without being asked
  for (const m of html.matchAll(/<(script|link|img|source|iframe)\b[^>]*>/gi)) {
    const url = /\b(?:src|href)\s*=\s*["']([^"']+)["']/i.exec(m[0])?.[1] || "";
    if (/^https?:\/\//i.test(url)) fail.push(`index.html loads ${url}`);
  }
  // script: a fetch or a dynamic import at an absolute url
  for (const m of js.matchAll(/\b(?:fetch|import)\s*\(\s*["'`](https?:\/\/[^"'`]+)/gi)) {
    fail.push(`app.js fetches ${m[1]}`);
  }
  // data: a cover is a path under icons/ or a data url, never somebody else's host
  let remote = 0;
  for (const row of data.images || []) {
    for (const key of ["cover", "logo"]) {
      if (/^https?:/i.test(row[key] || "")) remote++;
    }
    if (row.slugs) remote++;                     // slugs existed only to build a CDN icon url
  }
  if (remote) fail.push(`${remote} rows still carry a remote image; run vendor-covers.mjs`);

  const icons = existsSync(join(REG, "icons")) ? (await readdir(join(REG, "icons"))).length : 0;
  if (icons < 200) fail.push(`only ${icons} vendored covers; expected the icons/ set`);

  if (fail.length) throw new Error("registry page is not sealed:\n  " + fail.join("\n  "));
  return `registry page sealed: ${data.images.length} rows, ${icons} vendored covers, no external load`;
}

if (process.argv[1] && process.argv[1].endsWith("registry-sealed.mjs")) {
  try { console.log(await checkSealed(process.argv[2] || "dist")); }
  catch (err) { console.error(err.message); process.exit(1); }
}
