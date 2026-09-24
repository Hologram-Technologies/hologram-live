// Reading and writing a bucket from a page, with no dependencies at all.
//
// The hub does not load code from a CDN, so everything the page needs is here: CIDv1 in
// base32, just enough dag-pb and UnixFS to read and write a file DAG, and the walk over the
// OCI index tree. Object bytes are hashed by `crypto.subtle`, which is native and fast; the
// index nodes and keys use the shared synchronous sha-256 so the tree is one implementation.
import { sha256, sha256hex } from './sha256.mjs?v=3'

export const INDEX_MT = 'application/vnd.oci.image.index.v1+json'
export const MANIFEST_MT = 'application/vnd.oci.image.manifest.v1+json'
export const ACCEPT = `${INDEX_MT}, ${MANIFEST_MT}`
export const CHUNK = 1 << 20
import { sealBlock, openBlock, sealName, openName, keyCheck } from './crypt.mjs?v=3'
export const DEFAULT_QUOTA = 50e9

// public, unlisted, or private (= sealed). A head written before the distinction said
// "private" and meant unlisted; it reads as such.
export function visibilityOf (a = {}) {
  if (a['foundation.uor.bucket.encryption']) return 'private'
  const v = a['foundation.uor.bucket.visibility'] || 'public'
  return v === 'private' ? 'unlisted' : v
}
const RAW = 0x55
const DAG_PB = 0x70

// ---------------------------------------------------------------- CIDs
const B32 = 'abcdefghijklmnopqrstuvwxyz234567'
function base32 (bytes) {
  let bits = 0; let value = 0; let out = ''
  for (const byte of bytes) {
    value = (value << 8) | byte; bits += 8
    while (bits >= 5) { out += B32[(value >>> (bits - 5)) & 31]; bits -= 5 }
  }
  if (bits > 0) out += B32[(value << (5 - bits)) & 31]
  return out
}

/** A CIDv1 string for a codec and a 32-byte sha-256 digest. */
export function cidString (codec, digest) {
  const bytes = new Uint8Array(4 + digest.length)
  bytes[0] = 0x01; bytes[1] = codec; bytes[2] = 0x12; bytes[3] = 0x20
  bytes.set(digest, 4)
  return 'b' + base32(bytes)
}

const hexToBytes = hex => Uint8Array.from(hex.match(/../g).map(h => parseInt(h, 16)))
export const digestToCid = (digest, codec) => cidString(codec, hexToBytes(digest.split(':')[1]))

/** The two spellings of one address. A CID string back to `sha256:<hex>`. */
export function cidToDigest (cid) {
  let bits = 0; let value = 0
  const bytes = []
  for (const ch of cid.slice(1)) {
    value = (value << 5) | B32.indexOf(ch); bits += 5
    if (bits >= 8) { bytes.push((value >>> (bits - 8)) & 255); bits -= 8 }
  }
  const digest = bytes.slice(4)
  return 'sha256:' + digest.map(b => b.toString(16).padStart(2, '0')).join('')
}
export const codecOf = cid => (cid.startsWith('bafk') ? RAW : DAG_PB)

// ---------------------------------------------------------------- dag-pb, the little of it we need
function varint (n) {
  const out = []
  while (n >= 0x80) { out.push((n & 0x7f) | 0x80); n = Math.floor(n / 128) }
  out.push(n)
  return out
}
function readVarint (bytes, i) {
  let value = 0; let shift = 0
  for (;;) {
    const byte = bytes[i++]
    value += (byte & 0x7f) * Math.pow(2, shift)
    if ((byte & 0x80) === 0) break
    shift += 7
  }
  return [value, i]
}

/** The links of a dag-pb node, in order. Enough to walk a UnixFS file. */
export function linksOf (bytes) {
  const links = []
  let i = 0
  while (i < bytes.length) {
    let tag; [tag, i] = readVarint(bytes, i)
    const field = tag >> 3
    let len; [len, i] = readVarint(bytes, i)
    if (field === 2) {
      const link = bytes.subarray(i, i + len)
      let j = 0
      let hash = null
      while (j < link.length) {
        let ltag; [ltag, j] = readVarint(link, j)
        let llen; [llen, j] = readVarint(link, j)
        if ((ltag >> 3) === 1) hash = link.subarray(j, j + llen)
        j += llen
      }
      if (hash) links.push(cidString(hash[1], hash.subarray(4)))
    }
    i += len
  }
  return links
}

function field (num, wire, payload) {
  return [...varint((num << 3) | wire), ...(wire === 2 ? varint(payload.length) : []), ...payload]
}

/** A UnixFS file node over leaves of the given sizes, and the dag-pb node that carries it. */
export function fileNode (leaves, totalSize) {
  // UnixFS: Type=2 (File), filesize, blocksizes…
  const unixfs = [...field(1, 0, varint(2)), ...field(3, 0, varint(totalSize))]
  for (const l of leaves) unixfs.push(...field(4, 0, varint(l.size)))
  const parts = []
  for (const l of leaves) {
    const cidBytes = [0x01, RAW, 0x12, 0x20, ...l.digest]
    const link = [...field(1, 2, cidBytes), ...field(2, 2, []), ...field(3, 0, varint(l.size))]
    parts.push(...field(2, 2, link))
  }
  parts.push(...field(1, 2, unixfs))
  return new Uint8Array(parts)
}

// ---------------------------------------------------------------- the registry
export function registry (base = '') {
  const at = path => `${base}/v2/${path}`
  return {
    async manifest (repo, ref) {
      const r = await fetch(at(`${repo}/manifests/${ref}`), { headers: { accept: ACCEPT } })
      if (r.status === 404) return null
      if (!r.ok) throw new Error(`${repo}:${ref} → ${r.status}`)
      const text = await r.text()
      return { json: JSON.parse(text), digest: r.headers.get('docker-content-digest') || 'sha256:' + sha256hex(new TextEncoder().encode(text)) }
    },
    async blob (repo, digest) {
      const r = await fetch(at(`${repo}/blobs/${digest}`))
      if (!r.ok) throw new Error(`blob ${digest} → ${r.status}`)
      return new Uint8Array(await r.arrayBuffer())
    },
    async catalogue () {
      const r = await fetch(at('_catalog?n=200'))
      return r.ok ? (await r.json()).repositories || [] : []
    },
    async tags (repo) {
      const r = await fetch(at(`${repo}/tags/list?n=10000`))
      return r.ok ? (await r.json()).tags || [] : []
    },
    async putBlob (repo, bytes, token) {
      const digest = 'sha256:' + await hashBytes(bytes)
      const head = await fetch(at(`${repo}/blobs/${digest}`), { method: 'HEAD', headers: auth(token) })
      if (head.status === 200) return { digest, uploaded: false }
      const r = await fetch(at(`${repo}/blobs/uploads/?digest=${digest}`), {
        method: 'POST', headers: { 'content-type': 'application/octet-stream', ...auth(token) }, body: bytes
      })
      if (r.status !== 201) throw new Error(`upload → ${r.status}`)
      return { digest, uploaded: true }
    },
    async deleteManifest (repo, reference, token) {
      const r = await fetch(at(`${repo}/manifests/${reference}`), { method: 'DELETE', headers: auth(token) })
      if (r.status !== 202 && r.status !== 204) throw new Error(`delete → ${r.status}`)
      return true
    },
    async putManifest (repo, ref, bytes, mediaType, token) {
      const r = await fetch(at(`${repo}/manifests/${ref}`), {
        method: 'PUT', headers: { 'content-type': mediaType, ...auth(token) }, body: bytes
      })
      if (r.status !== 201) throw new Error(`manifest → ${r.status}`)
      return 'sha256:' + await hashBytes(bytes)
    }
  }
}
const auth = token => (token ? { authorization: token.startsWith('Basic ') || token.startsWith('Bearer ') ? token : `Bearer ${token}` } : {})

export async function hashBytes (bytes) {
  const out = await crypto.subtle.digest('SHA-256', bytes)
  return [...new Uint8Array(out)].map(b => b.toString(16).padStart(2, '0')).join('')
}

// ---------------------------------------------------------------- reading a bucket
export async function readBucket (reg, repo, { key = null } = {}) {
  const head = await reg.manifest(repo, 'latest')
  if (!head) return null
  const a = head.json.annotations || {}
  const encrypted = !!a['foundation.uor.bucket.encryption']
  if (encrypted && key && a['foundation.uor.bucket.keycheck'] && (await keyCheck(key)) !== a['foundation.uor.bucket.keycheck']) {
    const e = new Error('this key does not fit this bucket'); e.wrongKey = true; throw e
  }
  const entries = new Map()
  async function walk (node) {
    const level = Number(node.annotations?.['foundation.uor.bucket.level'] ?? '0')
    for (const child of node.manifests || []) {
      if (level === 0) {
        const c = child.annotations || {}
        const wire = c['foundation.uor.bucket.key']
        // A private bucket's names are sealed on the wire; with the key they read as written.
        entries.set(encrypted && key ? await openName(key, wire) : wire, {
          manifest: child.digest,
          root: c['foundation.uor.object.root'],
          size: Number(c['foundation.uor.bucket.size'] || 0),
          mtime: Number(c['foundation.uor.bucket.mtime'] || 0),
          ...(encrypted ? { wire } : {})
        })
      } else {
        walkNext.push(child.digest)
      }
    }
  }
  const walkNext = []
  await walk(head.json)
  while (walkNext.length) {
    const digest = walkNext.shift()
    const m = await reg.manifest(repo, digest)
    await walk(m.json)
  }
  return {
    repo,
    head: head.digest,
    annotations: a,
    visibility: visibilityOf(a),
    encrypted,
    key: encrypted ? key : null,
    locked: encrypted && !key,
    bytes: Number(a['foundation.uor.bucket.bytes'] || 0),
    quota: Number(a['foundation.uor.bucket.quota.bytes'] || DEFAULT_QUOTA),
    entries
  }
}

/** Every state a bucket has been in, newest first: one per publish, each a full root. */
export async function readHistory (reg, repo) {
  const tags = (await reg.tags(repo)).filter(t => /^h-\d+$/.test(t)).sort((x, y) => Number(y.slice(2)) - Number(x.slice(2)))
  const out = []
  for (const tag of tags) {
    const m = await reg.manifest(repo, tag).catch(() => null)
    if (!m) continue
    const a = m.json.annotations || {}
    out.push({ tag, state: m.digest, json: m.json, published: a['foundation.uor.bucket.published'] || new Date(Number(tag.slice(2))).toISOString(),
      objects: Number(a['foundation.uor.bucket.objects'] || 0), size: Number(a['foundation.uor.bucket.bytes'] || 0), parent: a['foundation.uor.bucket.parent'] || null })
  }
  return out
}

/**
 * Read one object, checking every block against its address before it counts.
 * onProgress({ done, total, blocks }) is called as blocks arrive.
 * Throws with `refused` set when a block's bytes are not what its address names.
 */
export async function readObject (reg, repo, rootCid, onProgress = () => {}, key = null) {
  const parts = []
  let done = 0
  let blocks = 0
  async function walk (cid) {
    let bytes = await reg.blob(repo, cidToDigest(cid))
    const got = await hashBytes(bytes)
    if (got !== cidToDigest(cid).slice(7)) {
      const error = new Error('these bytes are not what this address names')
      error.refused = cid
      throw error
    }
    blocks++
    if (codecOf(cid) === RAW) {
      if (key) bytes = await openBlock(key, bytes)     // checked first, opened second
      parts.push(bytes)
      done += bytes.length
      onProgress({ done, blocks })
      return
    }
    for (const child of linksOf(bytes)) await walk(child)
  }
  await walk(rootCid)
  const out = new Uint8Array(done)
  let at = 0
  for (const p of parts) { out.set(p, at); at += p.length }
  return out
}

// ---------------------------------------------------------------- writing an object
/** Chunk a file the way the recipe says, and push every block. Returns the object entry. */
const emptyPushed = new Set()
/** The OCI empty descriptor must exist as a blob before a manifest may name it, or the
    registry refuses the manifest with MANIFEST_BLOB_UNKNOWN. */
export async function ensureEmptyConfig (reg, repo, token) {
  if (emptyPushed.has(repo)) return
  await reg.putBlob(repo, new TextEncoder().encode('{}'), token)
  emptyPushed.add(repo)
}

export async function writeObject (reg, repo, key, file, token, onProgress = () => {}, bkey = null) {
  await ensureEmptyConfig(reg, repo, token)
  const leaves = []
  let sent = 0
  for (let offset = 0; offset < file.size || (file.size === 0 && offset === 0); offset += CHUNK) {
    let slice = new Uint8Array(await file.slice(offset, Math.min(offset + CHUNK, file.size)).arrayBuffer())
    if (bkey) slice = await sealBlock(bkey, slice)      // a private bucket: the block leaves sealed
    const hex = await hashBytes(slice)
    await reg.putBlob(repo, slice, token)
    leaves.push({ digest: hexToBytes(hex), size: slice.length })
    sent += slice.length
    onProgress({ done: sent, total: file.size })
    if (file.size === 0) break
  }
  let root
  const blocks = leaves.map(l => ({ digest: 'sha256:' + [...l.digest].map(b => b.toString(16).padStart(2, '0')).join(''), size: l.size }))
  if (leaves.length === 1) {
    root = cidString(RAW, leaves[0].digest)
  } else {
    const node = fileNode(leaves, leaves.reduce((n, l) => n + l.size, 0))
    const { digest } = await reg.putBlob(repo, node, token)
    blocks.push({ digest, size: node.length })
    root = digestToCid(digest, DAG_PB)
  }
  // The object's size is the plaintext size; its name on the wire is sealed when the bucket is.
  return { key, wire: bkey ? await sealName(bkey, key) : key, root, size: file.size, mtime: Date.now(), blocks }
}
