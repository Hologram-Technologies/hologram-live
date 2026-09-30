// get(κ) against honest, lying, partial and absent holders, offline.
//   node --test apps/model-hub/tensors/test/kappa-get.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { get, PieceHasher, PIECE, cidOf } from "../../deploy/kappa-get.mjs";

const sha = (b) => createHash("sha256").update(b).digest("hex");
const obj = randomBytes(5 * PIECE + 12345);
const kappa = `sha256:${sha(obj)}`;
const pieces = (() => { const h = new PieceHasher(); for (let o = 0; o < obj.length; o += 77777) h.update(obj.subarray(o, o + 77777)); return h.finish(); })();
const collect = async (it) => { const parts = []; for await (const c of it) parts.push(c); return Buffer.concat(parts); };
const honest = { kind: "honest", range: async (off, len) => obj.subarray(off, off + len) };
const liar = (every = 1) => { let n = 0; return { kind: "liar", range: async (off, len) => { const b = Buffer.from(obj.subarray(off, off + len)); if (n++ % every === 0) b[b.length >> 1] ^= 1; return b; } }; };
const byPiece = { kind: "ipfs", piece: async (h) => { const p = pieces.indexOf(h); return p < 0 ? null : obj.subarray(p * PIECE, (p + 1) * PIECE); } };

test("piece hashes are independent of chunking, and a piece's CID is its raw sha2-256", () => {
  const h = new PieceHasher(); h.update(obj);
  assert.deepEqual(h.finish(), pieces);
  assert.equal(pieces.length, 6);
  assert.match(cidOf(pieces[0]), /^bafkrei[a-z2-7]{52}$/);
});

test("whole object from an honest holder", async () => {
  assert.ok((await collect(get(kappa, { size: obj.length, pieces, holders: [honest] }))).equals(obj));
});

test("a range fetches only the pieces it covers", async () => {
  const seen = new Set();
  const out = await collect(get(kappa, { size: obj.length, pieces, from: PIECE + 5, to: 2 * PIECE + 9, holders: [honest], report: (e) => e.ok && seen.add(e.piece) }));
  assert.ok(out.equals(obj.subarray(PIECE + 5, 2 * PIECE + 10)));
  assert.deepEqual([...seen].sort(), [1, 2]);
});

test("a liar first: every lie is blamed per piece, its honest pieces kept, the object exact", async () => {
  const lies = [], kept = [];
  const out = await collect(get(kappa, { size: obj.length, pieces, run: 3 * PIECE, holders: [liar(), byPiece, honest], report: (e) => (e.ok ? e.holder === "liar" && kept.push(e.piece) : lies.push(e)) }));
  assert.ok(out.equals(obj));
  assert.equal(lies.length, 2);                                 // one flipped byte per range answer, two runs
  assert.ok(lies.every((e) => e.holder === "liar"));
  assert.equal(kept.length, 4);                                 // the rest of what the liar sent was right, and kept
});

test("only a liar: nothing is released", async () => {
  const out = [];
  await assert.rejects(async () => { for await (const c of get(kappa, { size: obj.length, pieces, run: PIECE, holders: [liar()] })) out.push(c); }, /no holder sent piece 0/);
  assert.equal(out.length, 0);
});

test("a holder that is down is skipped", async () => {
  const down = { kind: "down", range: async () => { throw new Error("ECONNREFUSED"); } };
  assert.ok((await collect(get(kappa, { size: obj.length, pieces, holders: [down, honest] }))).equals(obj));
});

test("a wrong piece list is refused before any fetch", async () => {
  await assert.rejects(() => collect(get(kappa, { size: obj.length, pieces: pieces.slice(1), holders: [honest] })), /no piece list/);
});
