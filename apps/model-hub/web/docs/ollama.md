---
title: Ollama
description: Pull any GGUF in the index straight into Ollama, verified by Ollama itself.
group: Connect
order: 7
---

Ollama can pull from any registry that speaks its dialect. The hub does, so every GGUF quantisation in the index is one `ollama pull` away, and Ollama checks the SHA-256 of every layer before the model is usable.

```bash
ollama pull gethologram.ai/ornith-ai/Ornith-1.5-9B-GGUF:Q4_K_M
ollama run  gethologram.ai/ornith-ai/Ornith-1.5-9B-GGUF:Q4_K_M
```

## The name

```
gethologram.ai/<owner>/<name>:<quant>
```

`<quant>` is the quantisation tag of a single-file GGUF in the repository: `Q4_K_M`, `IQ2_M`, `Q8_0` and so on. `latest` is also a tag. The tags a model has:

```bash
curl -s https://gethologram.ai/v2/ornith-ai/ornith-1.5-9b-gguf/tags/list
```

```json
{ "name": "ornith-ai/Ornith-1.5-9B-GGUF", "tags": ["latest", "BF16", "Q4_K_M", "Q5_K_M", "Q6_K", "Q8_0"] }
```

Owner and name are lowercase on this route; OCI references are case-sensitive and lowercase-only. A model with no GGUF file has no Ollama tags and refuses the pull with a sentence saying so.

## What happens

1. Ollama asks for the manifest with the Docker manifest media type. The hub answers a GGUF model manifest whose layers are the model file, its chat template and its parameters.
2. Small layers come from the hub directly.
3. The weight layer is a `307` to a source that is up right now. Ollama follows it and fetches in parallel `Range` parts. `HEAD` on the blob answers `200` with the size directly, because newer Ollama refuses a cross-host redirect on `HEAD`.
4. Ollama prints `verifying sha256 digest`. The digest it checks is the file's SHA-256 from the index.

The same three requests, by hand:

```bash
curl -s https://gethologram.ai/v2/ornith-ai/ornith-1.5-9b-gguf/manifests/Q4_K_M
```

```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": { "digest": "sha256:84736973…", "mediaType": "application/vnd.docker.container.image.v1+json", "size": 627 },
  "layers": [
    { "digest": "sha256:70c112196e0b7023803c9762752e46d29e612a92c83f995bc3ba1ceb07e8fab6", "mediaType": "application/vnd.ollama.image.model", "size": 5780090816 },
    { "digest": "sha256:940b926e…", "mediaType": "application/vnd.ollama.image.template", "size": 201 },
    …
  ]
}
```

```bash
curl -sI https://gethologram.ai/v2/ornith-ai/ornith-1.5-9b-gguf/blobs/sha256:70c112196e0b7023803c9762752e46d29e612a92c83f995bc3ba1ceb07e8fab6
```

```
HTTP/1.1 200 OK
Content-Length: 5780090816
Docker-Content-Digest: sha256:70c112196e0b7023803c9762752e46d29e612a92c83f995bc3ba1ceb07e8fab6
```

A `GET` on that blob answers `307` to `huggingface.co/ornith-ai/Ornith-1.5-9B-GGUF/resolve/abdd624b…/Ornith-1.5-9B-Q4_K_M.gguf`, with `X-Hub-Source: huggingface.co`.

Ollama 0.34.2 pulled a 969 MB model this way in 67 s. With huggingface.co unreachable from the client, the same pull completed in 83 s from ModelScope, with the manifest and template served from the hub. That failover is the point of the hub, and it costs you nothing to configure.

## Other engines that read the same routes

`ollama run hf.co/…` style names in LM Studio and Jan are hardcoded to huggingface.co and cannot be pointed here. Anything that pulls from a named OCI registry can: see [OCI artifacts](oci).
