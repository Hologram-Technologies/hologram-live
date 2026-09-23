---
title: Overview
description: What Hologram Hub is, what it replaces, and the one command that proves it.
group: Start
order: 1
---

You need a model file. Today you get it from whichever host is up, and you trust that host about which bytes you received. Hologram Hub removes the second half of that sentence.

The hub is one base URL, `https://hub.uor.foundation`, that answers in the dialects your tools already speak: the Hugging Face Hub API, the Ollama registry, OCI distribution, MCP, and a raw content-addressed object store. Every dialect is a view over one index in which every file is named by the SHA-256 of its bytes and every object by the BLAKE3 of its bytes. The hub carries names, addresses and directions. It never carries weights: a request for bytes is a redirect to a source that was up a moment ago, and the hash you check against comes from the index, never from that source.

No account, no key, no SDK. Every read is anonymous.

| You want to | Read |
| --- | --- |
| Get a verified file in five minutes | [Quickstart](quickstart) |
| Keep using `hf`, `transformers`, `vLLM`, `llama.cpp` | [Hugging Face tools](huggingface) |
| Pull with Ollama or an OCI client | [Ollama](ollama), [OCI artifacts](oci) |
| Give a coding agent the hub as tools | [MCP](mcp) |
| Read the raw index or mirror the whole hub | [Objects](objects) |
| Look up one route | [API reference](api) |

## What the hub replaces

| Before | With the hub |
| --- | --- |
| One host per ecosystem, each its own single point of failure | One host name, and each file served from whichever of Hugging Face, ModelScope or IPFS is up right now |
| The server that hands you bytes also tells you their hash | The expected hash comes from the index; the byte source is never asked |
| `main` moves under you | `main` is the revision the hub indexed, and the address of that revision never changes |
| A mirror with its own API and its own client | The dialects you already have, pointed at one URL |

## The proof

Run this on any machine with `curl`. It fetches one file of one model and checks it against the hash the index holds.

```bash
cd "$(mktemp -d)"
curl -sL https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/main/SHA256SUMS -o SHA256SUMS
curl -sL https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/main/EVAL.md -o EVAL.md
grep EVAL.md SHA256SUMS | sha256sum -c
```

```
EVAL.md: OK
```

If that line prints, everything on this page is true, and you did not take the page's word for any of it. Continue with the [Quickstart](quickstart).
