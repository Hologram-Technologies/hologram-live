---
title: Names, revisions and addresses
description: What a model id means on the hub, why main does not move, and how an address pins a version forever.
group: Concepts
order: 4
---

`main` on Hugging Face is a moving branch. A model you tested last week can be a different set of bytes today under the same name. The hub gives every name a fixed meaning and every version a permanent address.

## Names

A model is `<owner>/<name>`, exactly as on Hugging Face: `hexgrad/Kokoro-82M`. The hub is case-sensitive on the Hugging Face routes and lowercases the name on the OCI routes, because OCI repository names must be lowercase.

| Dialect | The same model |
| --- | --- |
| Hugging Face | `hexgrad/Kokoro-82M` |
| Ollama | `hub.uor.foundation/hexgrad/Kokoro-82M:<quant>` |
| OCI | `hub.uor.foundation/hexgrad/kokoro-82m:latest` |
| SBOM (purl) | `pkg:huggingface/hexgrad/Kokoro-82M@f3ff3571791e39611d31c381e3a41a3af07b4987` |

## Revisions

The hub indexes one revision per model: the commit that was current when the index last ran. `main` on the hub means that revision, and nothing else.

```bash
curl -s https://hub.uor.foundation/api/models/hexgrad/Kokoro-82M/refs
```

```json
{
  "branches": [{ "name": "main", "ref": "refs/heads/main", "targetCommit": "f3ff3571791e39611d31c381e3a41a3af07b4987" }],
  "tags": [],
  "converts": []
}
```

Asking for any other revision is refused with the one the hub has:

```bash
curl -s https://hub.uor.foundation/api/models/hexgrad/Kokoro-82M/revision/deadbeef
```

```json
{ "error": "The hub has hexgrad/Kokoro-82M at f3ff3571791e39611d31c381e3a41a3af07b4987 only." }
```

A `/resolve/` redirect always carries the full commit, never `main`, so the URL you are sent to is itself pinned:

```
Location: https://huggingface.co/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md
```

## Addresses

Under the names sits a content-addressed store. Every object in it is named by the BLAKE3 hash of its bytes, written `blake3:<64 hex>`. Two things follow:

- An address never changes meaning. A response under `/api/v1/objects/<address>` can be cached forever.
- A version is an address. To pin a model, keep its model address. To see history, follow `prev`.

The catalog's address changes daily; the descriptor at the root points at today's:

```bash
curl -s https://hub.uor.foundation/.well-known/model-hub.json
```

```json
{
  "format": "hologram.model-hub.descriptor/v1",
  "catalog": "blake3:423d29497813ce4cb5a21cf5c50bf8fe6efa1a1f19dc203e0dec8e5bb21a516f",
  "snapshot": "2026-09-23",
  "fetch": "/api/v1/objects/{id}",
  …
}
```

Inside the catalog, each name maps to a model address and its sources:

```json
"hexgrad/Kokoro-82M": {
  "model": "blake3:3cc11e52049117dfc397240fddc7a4d3aa392ded7f623986e4e5757371363d9e",
  "sources": ["blake3:1163c7df…", "blake3:6f5a0835…"]
}
```

And the model object holds the revision, the licence, every file with its SHA-256, and `prev`, the address of the revision before it, or `null` for the first one indexed:

```bash
curl -s https://hub.uor.foundation/api/v1/objects/blake3:3cc11e52049117dfc397240fddc7a4d3aa392ded7f623986e4e5757371363d9e
```

```json
{
  "format": "hologram.model-hub.model/v1",
  "id": "hexgrad/Kokoro-82M",
  "revision": "f3ff3571791e39611d31c381e3a41a3af07b4987",
  "license": "apache-2.0",
  "prev": null,
  "files": [
    { "path": "EVAL.md", "size": 534, "sha256": "9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658", "weights": false },
    …
  ]
}
```

The `manifest` in a search row is this address, so a search result already tells you which version you will get.

## Yesterday's hub

The catalog has `prev` too. Follow it and you are reading the index as it stood the day before, with every address still valid. `/archive.json` lists every day the hub has indexed, with the IPFS CID of each day's index.

```bash
curl -s https://hub.uor.foundation/archive.json | head -c 300
```

Next: where the bytes actually come from, in [Sources and failover](sources).
