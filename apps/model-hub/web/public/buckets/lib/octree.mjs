// The bucket index as an OCI index tree.
//
// Phase 1 built this index as dag-cbor nodes stored as blobs, which worked and then met
// `hologram oci garbage-collect`: the collector marks a manifest's config, layers and an
// index's children, and does not walk a dag-cbor tree, so every object block in every bucket
// was eligible for deletion. Measured, not guessed — `out-gc-before.txt`.
//
// The fix is not to teach the collector about buckets. It is to build the index out of the
// two things the collector already walks:
//
//   bucket root          = an OCI index
//     → internal nodes   = OCI indexes, ~32 children each
//       → leaf nodes     = OCI indexes, ~32 objects each
//         → one object   = an OCI manifest whose layers are ALL of that object's blocks
//
// Everything is then reachable by the existing rules, in every OCI registry, with no change
// to the registry at all. Whether `oras copy` and `crane copy` will also move a whole bucket
// between registries is a reasonable expectation and UNTESTED: neither tool is installed here.
//
// The boundary rule is unchanged from the dag-cbor version: a key ends a node at level L
// when sha-256 of the key begins with at least 5 × (L + 1) zero bits.
import { sha256, sha256hex } from './sha256.mjs?v=9'

export const BOUNDARY_BITS = 5
export const NODE_TYPE = 'application/vnd.uor.bucket.node.v1'
export const OBJECT_TYPE = 'application/vnd.uor.bucket.object.v1'
export const INDEX_MT = 'application/vnd.oci.image.index.v1+json'
export const MANIFEST_MT = 'application/vnd.oci.image.manifest.v1+json'
export const BLOCK_MT = 'application/vnd.ipld.raw'
export const EMPTY_CONFIG = {
  mediaType: 'application/vnd.oci.empty.v1+json',
  digest: 'sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a',
  size: 2
}

const keyHash = k => sha256(new TextEncoder().encode(k))

function leadingZeroBits (key) {
  const h = keyHash(key)
  let bits = 0
  for (const byte of h) {
    if (byte === 0) { bits += 8; continue }
    bits += Math.clz32(byte) - 24
    break
  }
  return bits
}
const isBoundary = (key, level) => leadingZeroBits(key) >= BOUNDARY_BITS * (level + 1)

export function jsonBlock (value) {
  const bytes = new TextEncoder().encode(JSON.stringify(value))
  return { bytes, digest: 'sha256:' + sha256hex(bytes), size: bytes.length }
}

/**
 * One object's manifest: every block of the file, so the collector marks them all.
 * blocks: [{ cid, size }] in DAG order, root last. root: the file's CID. key: its path.
 */
export function objectManifest ({ key, root, size, mtime, blocks }) {
  return jsonBlock({
    schemaVersion: 2,
    mediaType: MANIFEST_MT,
    artifactType: OBJECT_TYPE,
    config: EMPTY_CONFIG,
    layers: blocks.map(b => ({ mediaType: BLOCK_MT, digest: b.digest, size: b.size })),
    annotations: {
      'org.opencontainers.image.title': key,
      'foundation.uor.object.root': root.toString(),
      'foundation.uor.object.size': String(size),
      'foundation.uor.object.mtime': String(mtime || 0)
    }
  })
}

/**
 * Build the index tree over the sorted entry list.
 * entries: [[key, { manifest, manifestSize, root, size, mtime }], …]
 * Returns { root: digest, nodes: [{digest, size, bytes}], levels }.
 */
export function build (entries) {
  const nodes = []
  const sorted = [...entries].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))

  const descriptorFor = ([key, v]) => ({
    mediaType: MANIFEST_MT,
    digest: v.manifest,
    size: v.manifestSize,
    annotations: {
      'foundation.uor.bucket.key': key,
      'foundation.uor.bucket.size': String(v.size),
      'foundation.uor.bucket.mtime': String(v.mtime || 0),
      'foundation.uor.object.root': String(v.root)
    }
  })

  const makeNode = (children, level, last) => {
    const b = jsonBlock({
      schemaVersion: 2,
      mediaType: INDEX_MT,
      artifactType: NODE_TYPE,
      manifests: children,
      annotations: { 'foundation.uor.bucket.last': last, 'foundation.uor.bucket.level': String(level) }
    })
    nodes.push(b)
    return { digest: b.digest, size: b.size, last }
  }

  // Level 0
  let groups = []
  let run = []
  for (const e of sorted) {
    run.push(e)
    if (isBoundary(e[0], 0)) { groups.push(run); run = [] }
  }
  if (run.length || groups.length === 0) groups.push(run)
  let level = groups.map(g => makeNode(g.map(descriptorFor), 0, g.length ? g.at(-1)[0] : ''))

  let levels = 1
  while (level.length > 1) {
    const packs = []
    let g = []
    for (const n of level) {
      g.push(n)
      if (isBoundary(n.last, levels)) { packs.push(g); g = [] }
    }
    if (g.length) packs.push(g)
    if (packs.length === level.length) { packs.length = 0; packs.push(level) }
    level = packs.map(pack => makeNode(
      pack.map(n => ({
        mediaType: INDEX_MT,
        digest: n.digest,
        size: n.size,
        annotations: { 'foundation.uor.bucket.last': n.last }
      })),
      levels,
      pack.at(-1).last
    ))
    levels++
  }

  return { root: level[0].digest, rootSize: level[0].size, nodes, levels }
}

/** Walk the tree in key order. `load(digest) -> parsed JSON node`. */
export async function * walk (rootOrNode, load) {
  const node = typeof rootOrNode === 'string' ? await load(rootOrNode) : rootOrNode
  const level = Number(node.annotations?.['foundation.uor.bucket.level'] ?? '0')
  for (const child of node.manifests) {
    if (level === 0) {
      const a = child.annotations || {}
      yield [a['foundation.uor.bucket.key'], {
        manifest: child.digest,
        manifestSize: child.size,
        size: Number(a['foundation.uor.bucket.size'] || 0),
        mtime: Number(a['foundation.uor.bucket.mtime'] || 0),
        root: a['foundation.uor.object.root']
      }]
    } else {
      yield * walk(child.digest, load)
    }
  }
}

export async function toMap (rootDigest, load) {
  const m = new Map()
  for await (const [k, v] of walk(rootDigest, load)) m.set(k, v)
  return m
}
