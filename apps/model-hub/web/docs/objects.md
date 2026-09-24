---
title: Objects
description: The raw, content-addressed index under every dialect, and how to read, mirror or publish to it.
group: Connect
order: 10
---

Every dialect above is a view. Underneath is one store of objects, each named by the BLAKE3 hash of its bytes, on a Hologram Server. Read it when you want the truth with nothing in between: an agent that verifies everything, a mirror, or a publisher.

Objects are slower to read than the Hugging Face dialect, because finding one model means reading today's catalog first, and the catalog is about 380 KB. Use the dialect for search; use objects for proof, history and mirroring.

## Read

Two requests find a model; two more fetch and verify a file.

```bash
# 1. the descriptor names today's catalog
curl -s https://hub.uor.foundation/.well-known/model-hub.json | grep catalog

# 2. the catalog maps names to addresses
curl -s https://hub.uor.foundation/api/v1/objects/blake3:423d29497813ce4cb5a21cf5c50bf8fe6efa1a1f19dc203e0dec8e5bb21a516f -o catalog.json

# 3. the model object: revision, licence, files with SHA-256, prev
curl -s https://hub.uor.foundation/api/v1/objects/blake3:3cc11e52049117dfc397240fddc7a4d3aa392ded7f623986e4e5757371363d9e -o model.json

# 4. a source object: where one revision's bytes can be fetched
curl -s https://hub.uor.foundation/api/v1/objects/blake3:1163c7df258abaa3b8494261198e064bbfc9bef62c1b5fb5a7ef72d24017c6d3
```

```json
{
  "format": "hologram.model-hub.source/v1",
  "id": "hexgrad/Kokoro-82M",
  "revision": "f3ff3571791e39611d31c381e3a41a3af07b4987",
  "model": "blake3:3cc11e52049117dfc397240fddc7a4d3aa392ded7f623986e4e5757371363d9e",
  "kind": "ipfs",
  "root": "bafybeigtzo7l5ypt3l5oyedcn4yjzsasrap26ze6c64lzltf27rxq2irei",
  "resolve": "https://ipfs.filebase.io/ipfs/bafybeigtzo7l5ypt3l5oyedcn4yjzsasrap26ze6c64lzltf27rxq2irei/"
}
```

`resolve + path` is a URL for any file of that revision. The origin is always also valid: `https://huggingface.co/<id>/resolve/<revision>/<path>`.

| Kind | `resolve + path` | Extra |
| --- | --- | --- |
| `ipfs` | a gateway URL under the revision's CID | |
| `http` | a plain URL | |
| `hologram` | a Hologram Server | `chunks` per file: fetch each by address and join them |

## The three object kinds

| Kind | Content type | Holds |
| --- | --- | --- |
| catalog | `application/vnd.hologram.model-hub.catalog.v1+json` | `snapshot`, `prev`, `models` (browse facts) and `objects` (name → `{ model, sources }`) |
| model | `application/vnd.hologram.model-hub.model.v1+json` | `id`, `revision`, `license`, `prev`, `files[{ path, size, sha256, weights }]` |
| source | `application/vnd.hologram.model-hub.source/v1` | `kind`, `root`, `resolve`, and the model address it belongs to |

## The rules

- Check every object you fetch: the BLAKE3 of the bytes must equal the address you asked for. The server does not check on read. See [Verification](verification).
- Addresses never change meaning. Cache `/api/v1/objects/<address>` forever.
- The expected hash of a file comes from the model object only, never from a source.
- Follow `prev` on a model for its history, and on the catalog for yesterday's hub.

## Mirror

Start at the catalog, follow every address, `POST` each object to your own Hologram Server. Same bytes, same addresses, so anyone can check your mirror against the original.

```bash
# every address the catalog names
node -e 'const c=require("./catalog.json");for(const o of Object.values(c.objects)){console.log(o.model);o.sources.forEach(s=>console.log(s))}'
```

Weights are not objects. A mirror that wants to hold bytes as well runs a Hologram Server, posts each file as chunks of 16 MiB, and publishes a source record of kind `hologram` on the hub.

## Publish

Writes need a publisher token. Reads of a known address never do.

```bash
curl -X POST https://hub.uor.foundation/api/v1/objects \
  -H "Authorization: Bearer $HOLOGRAM_PUBLISHER_TOKEN" \
  -H 'x-hologram-kind: model-hub.source' \
  -H 'x-hologram-filename: source.json' \
  --data-binary @source.json
```

The response `id` is the address. Up to 8 MiB per request at the edge and 32 MiB at the server; larger payloads are published as chunks joined by a source record. There is no anonymous listing, on purpose: the catalog is the public index and it is one object away.

`/api/v1/capabilities` reports what the server can do and its limits; `/api/v1/modules` lists what is loaded.
