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
curl -s "https://hub.uor.foundation/api/models?search=kokoro&limit=2"
```

```json
[
  {
    "modelId": "hexgrad/Kokoro-82M",
    "pipeline_tag": "text-to-speech",
    "downloads": 11804278,
    "hologram": {
      "manifest": "blake3:2d184ec4d22e1ccab14342f3020c2838788e42186877669ccb1c7d56749c7a4a",
      "files": 72,
      "weight_bytes": 361048692,
      "sources": ["Hugging Face", "ModelScope", "P2P", "IPFS"]
    }
  },
  …
]
```

Truncated to the fields that matter: each row is Hugging Face's list shape plus a `hologram` block that says how many files the model has, how big its weights are, and where the bytes live today.

## 2. List its files

```bash
curl -s https://hub.uor.foundation/api/models/hexgrad/Kokoro-82M/tree/main
```

```json
[
  { "type": "file", "oid": "9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658", "size": 534, "path": "EVAL.md" },
  { "type": "file", "oid": "91dcabced89db6f109b8786642f50402d3ee87450e8189589b6f85520e7f4d78", "size": 6348, "path": "README.md" },
  …
]
```

`oid` is the SHA-256 the file's bytes must have. Keep it: it is the only hash you will trust.

## 3. Fetch the file

```bash
curl -sL https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/main/EVAL.md -o EVAL.md
```

Without `-L` you see what the hub actually does: a `302` to a source that was up a moment ago. The weights never pass through the hub.

```
HTTP/1.1 302 Found
Location: https://huggingface.co/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md
X-Hub-Source: huggingface.co
Etag: "9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658"
```

## 4. Prove it

```bash
echo "9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658  EVAL.md" | sha256sum -c
```

```
EVAL.md: OK
```

The hash on the left came from step 2, the index. The bytes came from step 3, a source. They agree, so the file is what the index says it is, and the source never had a chance to say otherwise. That is the whole product. The [Verification](verification) page has the rule in full.

## The same thing with one variable

Every tool built on `huggingface_hub` reads from the hub once you set one environment variable. Same commands, same cache.

```bash
export HF_ENDPOINT=https://hub.uor.foundation
hf download hexgrad/Kokoro-82M EVAL.md
```

`transformers`, `diffusers`, `sentence-transformers`, `vLLM` and `SGLang` all go through that variable; `llama.cpp` reads `MODEL_ENDPOINT` instead. The [Hugging Face tools](huggingface) page lists each one.

## Next

- Pull a whole model as a verified OCI artifact: [OCI artifacts](oci)
- Run a GGUF straight into Ollama: [Ollama](ollama)
- Choose which source serves you: [Sources and failover](sources)
