---
title: Quickstart
description: Find a model, download one file, and prove the bytes, in three requests and no key.
group: Start
order: 2
---

You want a file from a model and you want to know it is the right file. Three requests do it. Nothing to install, nothing to sign up for.

Every command on this page runs as written. `hexgrad/Kokoro-82M` is a real model in the index; swap in any other `<owner>/<name>` the search returns.

## 1. Find a model

```bash
curl -s "https://gethologram.ai/api/models?search=kokoro&limit=2"
```

```json
[
  {
    "id": "hexgrad/Kokoro-82M",
    "pipeline_tag": "text-to-speech",
    "downloads": 12859717,
    "likes": 7433,
    …
  },
  …
]
```

Truncated to the fields that matter. The rows are Hugging Face's, field for field, so a model that exists there is found here.

## 2. List its files

```bash
curl -s https://gethologram.ai/api/models/hexgrad/Kokoro-82M/tree/main
```

```json
[
  { "type": "file", "oid": "484ac8dcd971465258007528fb8bc4a99cf98633", "size": 534, "path": "EVAL.md" },
  { "type": "file", "oid": "15769c415594dcb424579ba3dfae3085578aac4e", "size": 327212226, "path": "kokoro-v1_0.pth",
    "lfs": { "oid": "496dba118d1a58f5f3db2efc88dbdc216e0483fc89fe6e47ee1f2c53f18ad1e4", "size": 327212226, "pointerSize": 134 } },
  …
]
```

This is Hugging Face's tree, unchanged. A large file's `lfs.oid` is the SHA-256 its bytes must have; a small file's `oid` is its git blob SHA-1. Keep them: they are the only hashes you will trust.

## 3. Fetch the file

```bash
curl -sL https://gethologram.ai/hexgrad/Kokoro-82M/resolve/main/EVAL.md -o EVAL.md
```

Without `-L` you see what the hub actually does: a `302` to a source that was up a moment ago. The weights never pass through the hub.

```
HTTP/1.1 302 Found
Location: https://huggingface.co/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md
X-Hub-Source: huggingface.co
Etag: "484ac8dcd971465258007528fb8bc4a99cf98633"
```

## 4. Prove it

```bash
git hash-object --no-filters EVAL.md
```

```
484ac8dcd971465258007528fb8bc4a99cf98633
```

The hash on the left came from step 2, the index. The bytes came from step 3, a source. They agree, so the file is what the index says it is, and the source never had a chance to say otherwise. That is the whole product. The [Verification](verification) page has the rule in full.

## The same thing with one variable

Every tool built on `huggingface_hub` reads from the hub once you set one environment variable. Same commands, same cache.

```bash
export HF_ENDPOINT=https://gethologram.ai
hf download hexgrad/Kokoro-82M EVAL.md
```

`transformers`, `diffusers`, `sentence-transformers`, `vLLM` and `SGLang` all go through that variable; `llama.cpp` reads `MODEL_ENDPOINT` instead. The [Hugging Face tools](huggingface) page lists each one.

## Next

- Pull a whole model as a verified OCI artifact: [OCI artifacts](oci)
- Run a GGUF straight into Ollama: [Ollama](ollama)
- Choose which source serves you: [Sources and failover](sources)
