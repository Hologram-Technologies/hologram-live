// sample.mjs against vectors from the Python reference (tensorhash.py): every expected value, under several chunkings.
//   node --test apps/model-hub/tensors/test/sample.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { TensorSample } from "../lib/sample.mjs";

const { cases } = JSON.parse(readFileSync(new URL("./sample-vectors.json", import.meta.url), "utf8"));

for (const c of cases) {
  test(`${c.note} ${c.dtype}${JSON.stringify(c.shape)}`, () => {
    const buf = Buffer.from(c.data, "base64");
    for (const step of [1, 3, 7, 64, 1000, buf.length || 1]) {
      const s = new TensorSample(c.dtype, c.shape);
      for (let i = 0; i < buf.length; i += step) s.update(buf.subarray(i, i + step));
      const got = s.result();
      for (const k of Object.keys(c.expected)) assert.deepEqual(got[k], c.expected[k], `${k} with chunks of ${step}`);
    }
  });
}
