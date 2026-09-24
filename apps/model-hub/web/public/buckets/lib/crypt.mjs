// A private bucket is an encrypted bucket. Nothing else holds on a public network: a registry
// that refuses anonymous reads is one registry, and the whole point of a block is that any
// machine can be its home. So "private" means the bytes and the names leave this machine
// sealed, and the key never does.
//
//   block on the wire = nonce(12) ‖ AES-256-GCM(key, nonce, plaintext, aad) ‖ tag(16)
//   nonce            = HMAC-SHA256(key, plaintext)[:12]      convergent: same bytes, same block
//   address          = sha256(block on the wire)              still self-verifying, still dedups
//   name on the wire = "e." + base64url(nonce ‖ AES-GCM(key, nonce, name, aad))
//   nonce            = HMAC-SHA256(key, "name/" + name)[:12]  deterministic: a rename is a lookup
//
// A deterministic nonce is safe here because it is a function of the key and the exact
// plaintext: the same (key, plaintext) pair yields the same block, which is what dedup wants,
// and a different plaintext yields a different nonce. What it leaks is equality — two people
// with the key can see two objects share a block — and that is the leak dedup already has.
//
// Same code in Node and the browser: WebCrypto only.
const subtle = globalThis.crypto.subtle
const te = new TextEncoder()
const td = new TextDecoder()

export const ENC = 'aes-256-gcm/v1'
const AAD_BLOCK = te.encode('uor-bucket-block/v1')
const AAD_NAME = te.encode('uor-bucket-name/v1')

export function newKey () {
  const k = new Uint8Array(32)
  globalThis.crypto.getRandomValues(k)
  return k
}

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_'
export function b64url (bytes) {
  let out = ''
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0)
    out += B64[n >> 18] + B64[(n >> 12) & 63] + (i + 1 < bytes.length ? B64[(n >> 6) & 63] : '') + (i + 2 < bytes.length ? B64[n & 63] : '')
  }
  return out
}
export function unb64url (text) {
  const out = []
  let bits = 0; let value = 0
  for (const ch of text) {
    const v = B64.indexOf(ch)
    if (v < 0) throw new Error('not base64url')
    value = (value << 6) | v; bits += 6
    if (bits >= 8) { bits -= 8; out.push((value >> bits) & 255) }
  }
  return new Uint8Array(out)
}

export const keyToText = key => b64url(key)
export function keyFromText (text) {
  const k = unb64url(String(text).trim())
  if (k.length !== 32) throw new Error('a bucket key is 32 bytes')
  return k
}

/** A short fingerprint of the key, written on the head, so a wrong key is told apart from a
    corrupt block before anything is decrypted. It reveals nothing about the key. */
export async function keyCheck (key) {
  const d = new Uint8Array(await subtle.digest('SHA-256', concat(te.encode('uor-bucket-key/v1'), key)))
  return hex(d).slice(0, 16)
}

const aesKeys = new WeakMap()
const macKeys = new WeakMap()
async function aes (key) {
  let k = aesKeys.get(key)
  if (!k) { k = await subtle.importKey('raw', key, 'AES-GCM', false, ['encrypt', 'decrypt']); aesKeys.set(key, k) }
  return k
}
async function mac (key) {
  let k = macKeys.get(key)
  if (!k) { k = await subtle.importKey('raw', key, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']); macKeys.set(key, k) }
  return k
}
async function nonceFor (key, bytes) {
  return new Uint8Array(await subtle.sign('HMAC', await mac(key), bytes)).slice(0, 12)
}

export async function sealBlock (key, plain) {
  const nonce = await nonceFor(key, plain)
  const ct = new Uint8Array(await subtle.encrypt({ name: 'AES-GCM', iv: nonce, additionalData: AAD_BLOCK }, await aes(key), plain))
  return concat(nonce, ct)
}
export async function openBlock (key, sealed) {
  if (sealed.length < 28) throw refused('a sealed block is at least 28 bytes')
  try {
    return new Uint8Array(await subtle.decrypt({ name: 'AES-GCM', iv: sealed.subarray(0, 12), additionalData: AAD_BLOCK }, await aes(key), sealed.subarray(12)))
  } catch { throw refused('this block does not open with this key') }
}

export async function sealName (key, name) {
  const plain = te.encode(name)
  const nonce = await nonceFor(key, concat(te.encode('name/'), plain))
  const ct = new Uint8Array(await subtle.encrypt({ name: 'AES-GCM', iv: nonce, additionalData: AAD_NAME }, await aes(key), plain))
  return 'e.' + b64url(concat(nonce, ct))
}
export const isSealedName = s => typeof s === 'string' && s.startsWith('e.')
export async function openName (key, sealed) {
  if (!isSealedName(sealed)) return sealed
  const bytes = unb64url(sealed.slice(2))
  try {
    return td.decode(await subtle.decrypt({ name: 'AES-GCM', iv: bytes.subarray(0, 12), additionalData: AAD_NAME }, await aes(key), bytes.subarray(12)))
  } catch { throw refused('this name does not open with this key') }
}

/** The stored size of a plaintext of n bytes cut at `chunk`: each block grows by 28. */
export function sealedSize (n, chunk) {
  const blocks = n === 0 ? 1 : Math.ceil(n / chunk)
  return n + 28 * blocks
}

function refused (why) { const e = new Error('refused: ' + why); e.refused = true; return e }
function concat (a, b) { const out = new Uint8Array(a.length + b.length); out.set(a, 0); out.set(b, a.length); return out }
const hex = bytes => [...bytes].map(b => b.toString(16).padStart(2, '0')).join('')
