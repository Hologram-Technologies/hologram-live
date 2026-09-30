// Vendored from HOLOGRAM/spikes/ipfs-pin/recipe.mjs (the hub's one IPFS import recipe); every line below this one is identical to it.
// The hub's one import recipe = IPIP-499 profile "unixfs-v1-2025":
//   CIDv1, sha2-256, raw leaves, fixed 1 MiB chunks, balanced file DAG, at most 1024 links per node,
//   plain (unsharded) dag-pb directories, no mode/mtime.
// Streaming and constant-memory: holds one chunk plus at most 1024 links per DAG level.
// Runs unchanged in browsers, workers, Node, Deno. Depends only on @ipld/dag-pb, ipfs-unixfs, multiformats.
import { encode, prepare } from '@ipld/dag-pb'
import { UnixFS } from 'ipfs-unixfs'
import { CID } from 'multiformats/cid'
import * as raw from 'multiformats/codecs/raw'
import * as Digest from 'multiformats/hashes/digest'

export const RECIPE = 'unixfs-v1-2025'
export const CHUNK = 1 << 20
export const WIDTH = 1024
const DAG_PB = 0x70
const SHA256 = 0x12

async function sha256 (bytes) {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
}
async function cidOf (codec, bytes) {
  return CID.createV1(codec, Digest.create(SHA256, await sha256(bytes)))
}

// A small file (at most one chunk) needs no bytes at all: its CID is the raw CID of its whole-file SHA-256.
export function cidFromSha256 (hex) {
  const d = Uint8Array.from(hex.match(/../g).map(h => parseInt(h, 16)))
  return CID.createV1(raw.code, Digest.create(SHA256, d))
}

// fileImporter(onBlock): feed bytes with write(), end with close() → { cid, size, dagSize, blocks }.
// onBlock({ cid, bytes }) receives every block in CAR order (leaves first, parents after) and may be async.
export function fileImporter (onBlock = () => {}) {
  let buf = new Uint8Array(CHUNK); let fill = 0; let size = 0; let blocks = 0
  const pending = [[]]   // pending[L] = entries { cid, fileSize, dagSize } not yet grouped at level L
  const total = [0]      // total[L]   = entries ever produced at level L

  async function push (level, entry) {
    if (!pending[level]) { pending[level] = []; total[level] = 0 }
    pending[level].push(entry); total[level]++
    if (pending[level].length === WIDTH) await flush(level)
  }
  async function flush (level) {
    const kids = pending[level]; pending[level] = []
    const f = new UnixFS({ type: 'file' })
    for (const k of kids) f.addBlockSize(BigInt(k.fileSize))
    const bytes = encode(prepare({ Data: f.marshal(), Links: kids.map(k => ({ Name: '', Tsize: k.dagSize, Hash: k.cid })) }))
    const cid = await cidOf(DAG_PB, bytes)
    blocks++; await onBlock({ cid, bytes })
    await push(level + 1, {
      cid,
      fileSize: kids.reduce((a, k) => a + k.fileSize, 0),
      dagSize: bytes.length + kids.reduce((a, k) => a + k.dagSize, 0)
    })
  }
  async function leaf (bytes) {
    const cid = await cidOf(raw.code, bytes)
    blocks++; await onBlock({ cid, bytes })
    await push(0, { cid, fileSize: bytes.length, dagSize: bytes.length })
  }

  return {
    async write (chunk) {
      let off = 0
      while (off < chunk.length) {
        const n = Math.min(CHUNK - fill, chunk.length - off)
        buf.set(chunk.subarray(off, off + n), fill); fill += n; off += n; size += n
        if (fill === CHUNK) { await leaf(buf); buf = new Uint8Array(CHUNK); fill = 0 }
      }
    },
    async close () {
      if (fill > 0 || size === 0) await leaf(buf.subarray(0, fill))
      for (let level = 0; ; level++) {
        if (total[level] === 1 && pending[level].length === 1) {
          const root = pending[level][0]
          return { cid: root.cid, size, dagSize: root.dagSize, blocks }
        }
        if (pending[level].length > 0) await flush(level)
      }
    }
  }
}

// Directory root from (path, cid, dagSize) triples alone: no file bytes needed.
// Returns the root and every directory block (they are tiny and belong in any CAR of the model).
export async function directoryRoot (files) {
  const tree = {}
  for (const f of files) {
    const parts = f.path.split('/'); let node = tree
    for (const p of parts.slice(0, -1)) node = (node[p] ??= {})
    node[parts.at(-1)] = { cid: typeof f.cid === 'string' ? CID.parse(f.cid) : f.cid, dagSize: f.dagSize, file: true }
  }
  const out = []
  async function build (node) {
    const links = []
    for (const [name, child] of Object.entries(node)) {
      const c = child.file ? child : await build(child)
      links.push({ Name: name, Tsize: c.dagSize, Hash: c.cid })
    }
    const bytes = encode(prepare({ Data: new UnixFS({ type: 'directory' }).marshal(), Links: links }))
    const cid = await cidOf(DAG_PB, bytes)
    out.push({ cid, bytes })
    return { cid, dagSize: bytes.length + links.reduce((a, l) => a + l.Tsize, 0) }
  }
  const root = await build(tree)
  return { cid: root.cid, dagSize: root.dagSize, blocks: out }
}
