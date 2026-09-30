// The names log: chain, signature, replacement alerts, point-in-time answers, tamper and forgery detection.
//   node --test apps/model-hub/tensors/test/names.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { NamesLog, verifyLog, pointedAt } from "../lib/names.mjs";

test("a names log records, alerts, verifies and answers", () => {
  const dir = mkdtempSync(join(tmpdir(), "names-"));
  const log = new NamesLog(dir, "test");
  log.witnessFiles("org/m", "r1", { "model.safetensors": "sha256:aa", "config.json": "sha256:bb" });
  log.indexed("org/m", "r1", "sha256:11", "sha256:22");
  assert.equal(log.witnessFiles("org/m", "r1", { "model.safetensors": "sha256:aa", "config.json": "sha256:bb" }).length, 0, "unchanged: no alert");
  const alerts = log.witnessFiles("org/m", "r1", { "model.safetensors": "sha256:cc" });
  assert.equal(alerts.length, 1); assert.deepEqual(alerts[0].before, { "model.safetensors": "sha256:aa" });
  log.witnessFiles("org/m", "r2", { "model.safetensors": "sha256:dd" });
  const head = log.signHead();
  const text = readFileSync(join(dir, "names.log.jsonl"), "utf8");
  const [entries, problems] = verifyLog(text, head);
  assert.deepEqual(problems, []);
  assert.deepEqual(entries.map((e) => e.kind), ["witnessed", "indexed", "replaced", "moved", "witnessed"]);
  assert.equal(pointedAt(entries, "org/m", { upto: 2 }).files["model.safetensors"], "sha256:aa");
  assert.equal(pointedAt(entries, "org/m", { upto: 2 }).index, "sha256:11");
  assert.equal(pointedAt(entries, "org/m").revision, "r2");
  // one edited byte breaks the chain; a forged head fails its signature
  assert.match(verifyLog(text.replace("sha256:22", "sha256:23"), head)[1].join(";"), /prev link broken/);
  assert.match(verifyLog(text, { ...head, head: "0".repeat(64) })[1].join(";"), /signature does not verify/);
  // a reopened log resumes the chain with the same key
  const again = new NamesLog(dir, "test");
  assert.equal(again.seq, 5); assert.equal(again.pub, log.pub);
});
