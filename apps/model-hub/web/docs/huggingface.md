---
title: Hugging Face tools
description: Point the tools you already have at the hub with one environment variable.
group: Connect
order: 6
---

Your code already downloads models through `huggingface_hub`. It reads one variable to decide where from. Set it and nothing else changes: same commands, same cache directory, same file layout.

```bash
export HF_ENDPOINT=https://hub.uor.foundation
```

## What that reaches

| Tool | Setting | Verified |
| --- | --- | --- |
| `hf` CLI, `huggingface_hub` (`hf_hub_download`, `snapshot_download`, `HfApi`) | `HF_ENDPOINT` | 1.x and 0.36 |
| `transformers`, `diffusers`, `sentence-transformers` (`from_pretrained`) | `HF_ENDPOINT` | metadata and download path |
| vLLM, SGLang, TGI, KServe `hf://` | `HF_ENDPOINT` | same client path; a full serving run has not been recorded |
| llama.cpp `-hf`, `llama-server -hf` | `MODEL_ENDPOINT=https://hub.uor.foundation/` | build 11028 |
| `transformers.js`, `@huggingface/hub` in a browser | the URL, cross-origin | every read route sends `Access-Control-Allow-Origin: *` |

`docker model pull hf.co/…`, LM Studio and Jan hardcode huggingface.co and cannot be pointed here.

## Download a model

```bash
export HF_ENDPOINT=https://hub.uor.foundation
hf download hexgrad/Kokoro-82M
```

Or from Python:

```python
import os
os.environ["HF_ENDPOINT"] = "https://hub.uor.foundation"

from huggingface_hub import HfApi, hf_hub_download, snapshot_download

api = HfApi()
print([m.id for m in api.list_models(search="kokoro", limit=3)])
# ['hexgrad/Kokoro-82M', 'oddadmix/Kokoro-7M-Distill']

info = api.model_info("hexgrad/Kokoro-82M")
print(info.sha, len(info.siblings))
# f3ff3571791e39611d31c381e3a41a3af07b4987 72

path = hf_hub_download("hexgrad/Kokoro-82M", "EVAL.md")
local = snapshot_download("hexgrad/Kokoro-82M")
```

`main` resolves to the indexed revision, so `snapshot_download` lands in `snapshots/f3ff3571…/`, the same directory a download from huggingface.co of that commit would use. The two caches are interchangeable.

## Load a model

```python
import os
os.environ["HF_ENDPOINT"] = "https://hub.uor.foundation"

from transformers import AutoConfig, AutoTokenizer
config = AutoConfig.from_pretrained("Qwen/Qwen3-0.6B")
tokenizer = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
print(config.model_type, tokenizer("hub")["input_ids"])
# qwen3 [26682]
```

`from_pretrained` for weights takes the same path; it needs a framework such as PyTorch installed, which the hub does not change.

## llama.cpp

```bash
MODEL_ENDPOINT=https://hub.uor.foundation/ llama-server -hf bartowski/MiniCPM5-2B-GGUF:IQ2_M
```

The GGUF lands in the shared Hugging Face cache layout and loads.

## Verify afterwards

`huggingface_hub` checks the size of each file, not its hash. For the check that matters, run this inside the downloaded snapshot directory:

```bash
cd "$(hf download hexgrad/Kokoro-82M)"
curl -s https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/main/SHA256SUMS | sha256sum -c
```

`hf download` prints the snapshot directory it filled, so the first line lands you in it.

## Search from the command line

The list route is Hugging Face's, with the same filters, plus a `hologram` block per row. It answers in a few hundred bytes, not a catalogue.

```bash
curl -s "https://hub.uor.foundation/api/models?search=qwen&pipeline_tag=text-generation&filter=gguf&sort=downloads&limit=5"
```

| Parameter | Meaning |
| --- | --- |
| `search` | case-insensitive substring of the model id |
| `author` | exact `<owner>` |
| `pipeline_tag` | exact task, e.g. `text-generation`, `text-to-speech` |
| `library` | exact library, e.g. `transformers`, `gguf` |
| `filter` | a tag that must be present, e.g. `gguf`, `apache-2.0`; repeat or comma-separate for several |
| `sort` | `trendingScore` (default), `downloads`, `likes`, `createdAt` |
| `direction` | `1` ascending; anything else descending |
| `limit` | 1 to 500, default 50 |

## What is not here

The hub indexes a daily snapshot of trending open models, not all of Hugging Face. A model that is not in the index answers `404` with a sentence saying so, and the request is recorded for the next index run. Until then, `huggingface.co` still works for it: unset `HF_ENDPOINT` for that one call. See [Limits and scope](limits).
