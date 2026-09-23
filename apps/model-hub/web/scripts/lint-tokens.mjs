// Fails when product code bypasses the Hologram brand kit.
//
//   node apps/model-hub/web/scripts/lint-tokens.mjs

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (p) => readFile(join(SITE, p), "utf8");

const styles = await read("src/styles.css");
const defined = new Set();
for (const src of [await read("vendor/hologram-brand-kit/hologram-warm.css"), await read("vendor/hologram-brand-kit/hologram-gap-tokens.css"), await read("src/tokens.css"), styles]) {
  for (const [, name] of src.matchAll(/(--[a-z0-9-]+)\s*:/g)) defined.add(name);
}

const problems = [];
const lines = styles.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, " ")).split("\n");
lines.forEach((line, i) => {
  const at = `styles.css:${i + 1}`;
  if (/#[0-9a-f]{3,8}\b/i.test(line)) problems.push(`${at} hex color`);
  if (/\b(rgba?|hsla?|oklch)\(/i.test(line)) problems.push(`${at} literal color function`);
  if (!line.trim().startsWith("@media")) {
    for (const [px] of line.matchAll(/\b\d+(\.\d+)?px\b/g)) if (px !== "1px" && px !== "2px") problems.push(`${at} literal ${px}`);
  }
  if (/font-family:(?!\s*(var\(|inherit))/.test(line)) problems.push(`${at} font family not from the kit`);
  if (/font-size:(?!\s*(var\(|inherit))/.test(line)) problems.push(`${at} font size not from the kit`);
  if (/border-radius:(?!\s*(var\(|50%|0\b))/.test(line)) problems.push(`${at} radius not from the kit`);
  for (const [, name] of line.matchAll(/var\((--[a-z0-9-]+)/g)) if (!defined.has(name)) problems.push(`${at} undefined ${name}`);
});

for (const file of ["build.mjs", "src/render.mjs", "src/app.js", "src/auth.js", "src/overview.mjs", "src/braille.mjs", "src/landing.mjs", "src/hero-badge.mjs"]) {
  const src = await read(file);
  if (/style="/.test(src)) problems.push(`${file} inline style attribute`);
  if (/#[0-9a-f]{6}\b/i.test(src.replace(/\/\/.*$/gm, ""))) problems.push(`${file} hex color`);
}

const brand = (styles.match(/var\(--hh-accent\)/g) || []).length;
if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} token violations`);
  process.exit(1);
}
console.log(`tokens ok: ${defined.size} defined variables, no literal colors, sizes or radii; accent used in ${brand} rule(s)`);
