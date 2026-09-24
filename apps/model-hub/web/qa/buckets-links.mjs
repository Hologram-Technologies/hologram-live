// A model card links its buckets; the Buckets page links back. Both from one field.
//
// Hugging Face's model cards carry `buckets:` in the YAML front matter: a list of the buckets a
// model's checkpoints, data or artifacts live in. The same field, read the same way, so a card
// written for their hub says the same thing here. Accepted spellings, all meaning owner/name:
//
//   buckets:
//     - owner/name
//     - hf://buckets/owner/name
//     - uor://buckets/owner/name
//
// bucketsOf(readme) is the parser; checkBucketLinks() is the gate the build runs, and it also
// runs the parser over a fixture so the parser cannot rot while no live card carries the field.
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";

const ID = /^[a-z0-9][a-z0-9._-]*\/[a-z0-9][a-z0-9._-]*$/i;

export function bucketsOf(readme) {
  if (!readme || !readme.startsWith("---")) return [];
  const end = readme.indexOf("\n---", 3);
  if (end < 0) return [];
  const front = readme.slice(3, end).split(/\r?\n/);
  const out = [];
  let inList = false;
  for (const line of front) {
    if (/^buckets:\s*$/.test(line)) { inList = true; continue; }
    if (/^buckets:\s*\[/.test(line)) {           // inline: buckets: [a/b, c/d]
      for (const item of line.replace(/^buckets:\s*\[|\]\s*$/g, "").split(",")) push(out, item);
      continue;
    }
    if (inList) {
      const m = /^\s+-\s*(.+?)\s*$/.exec(line);
      if (m) { push(out, m[1]); continue; }
      inList = false;
    }
  }
  return [...new Set(out)];
}

function push(out, raw) {
  let v = String(raw).trim().replace(/^["']|["']$/g, "");
  v = v.replace(/^(?:hf|uor):\/\/buckets\//, "").replace(/\/+$/, "");
  if (ID.test(v)) out.push(v);
}

const FIXTURE = `---
license: apache-2.0
buckets:
  - ilya/run-42
  - hf://buckets/team/checkpoints
  - "uor://buckets/team/data/"
  - not a bucket
tags:
  - demo
---
# A model
`;

export async function checkBucketLinks(dist, modelCount) {
  const got = bucketsOf(FIXTURE);
  const want = ["ilya/run-42", "team/checkpoints", "team/data"];
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`buckets: parser drifted: ${JSON.stringify(got)}`);
  if (bucketsOf("# no front matter\nbuckets:\n  - x/y\n").length) throw new Error("buckets: read outside the front matter");
  const at = join(dist, "buckets", "links.json");
  if (!existsSync(at)) throw new Error("buckets/links.json was not written");
  const links = JSON.parse(await readFile(at, "utf8"));
  for (const [bucket, ids] of Object.entries(links)) {
    if (!ID.test(bucket)) throw new Error(`links.json carries a bad bucket id: ${bucket}`);
    for (const id of ids) if (!existsSync(join(dist, "models", id, "index.html"))) throw new Error(`links.json points at a model page that does not exist: ${id}`);
  }
  const n = Object.keys(links).length;
  return `bucket links ok: parser fixture 3/3, ${n} bucket${n === 1 ? "" : "s"} linked from ${modelCount} model cards`;
}
