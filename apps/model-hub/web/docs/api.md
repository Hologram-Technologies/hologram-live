---
title: API reference
description: Every route the hub answers, grouped by what it is for, generated from the OpenAPI document.
group: Reference
order: 11
---

One base URL, no key for any read. Everything below is generated at build time from [`/openapi.json`](/openapi.json), the OpenAPI 3.1 document that describes the whole endpoint, so a route cannot appear here without being described there. The interactive rendering of the same document is at [`/docs`](/docs).

```
https://hub.uor.foundation
```

| Placeholder | Meaning | Example |
| --- | --- | --- |
| `{owner}/{name}` | a model id, as on Hugging Face | `hexgrad/Kokoro-82M` |
| `{revision}` | `main`, or the commit the hub indexed | `f3ff3571791e39611d31c381e3a41a3af07b4987` |
| `{path}` | a file inside the model; may contain slashes | `voices/af_heart.pt` |
| `{source}` | `huggingface`, `modelscope` or `ipfs` | `ipfs` |
| `{id}` | an object address | `blake3:3cc11e52…` |

<!--openapi:reference-->
