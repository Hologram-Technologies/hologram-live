// ZipWriter.sizeOf must equal the bytes ZipWriter writes, or a download that declares Content-Length breaks at the
// end. Run: node qa/zip-size.test.mjs  (the 4 GiB case streams zeros through CRC32; about half a minute).
import { ZipWriter } from "../src/zip.mjs";

const zeros = new Uint8Array(4 * 1024 * 1024);
function stream(size) {
  let left = size;
  return new ReadableStream({
    pull(controller) {
      if (left <= 0) { controller.close(); return; }
      const n = Math.min(left, zeros.length);
      controller.enqueue(n === zeros.length ? zeros : zeros.subarray(0, n));
      left -= n;
    },
  });
}

const GiB = 1024 ** 3;
const cases = {
  "empty file": [{ name: "empty", size: 0 }],
  "one byte": [{ name: "a", size: 1 }],
  "utf-8 names": [{ name: "模型/重み.bin", size: 10 }, { name: "émoji-🙂/x", size: 3 }],
  "many small files": Array.from({ length: 300 }, (_, i) => ({ name: `dir/${i}.json`, size: i })),
  "65,536 files (zip64 counts)": Array.from({ length: 65536 }, (_, i) => ({ name: `f/${i}`, size: 0 })),
  "file above 4 GiB, then a file at a 64-bit offset": [{ name: "big.safetensors", size: 4 * GiB + 5 }, { name: "after.json", size: 7 }],
};

let failed = 0;
for (const [label, entries] of Object.entries(cases)) {
  let written = 0;
  const zip = new ZipWriter((bytes) => { written += bytes.length; });
  for (const e of entries) await zip.add(e.name, e.size, stream(e.size));
  await zip.finish();
  const predicted = ZipWriter.sizeOf(entries);
  const ok = predicted === written;
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${label}: predicted ${predicted}, written ${written}`);
}
process.exit(failed ? 1 : 0);
