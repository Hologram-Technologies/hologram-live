---
title: OCI artifacts
description: Pull a whole model as a CNCF ModelPack artifact with oras, modctl, skopeo or crane, every file verified by its digest.
group: Connect
order: 8
---

Your registry tooling already knows how to fetch a manifest, follow a blob redirect and verify every digest. The hub serves each model as an OCI artifact whose layers are the model's files, so that tooling works unchanged and verifies the weights for you.

```bash
oras pull gethologram.ai/hexgrad/kokoro-82m:latest
```

That is the whole model: 72 files, 347 MB, every one hashing to its digest. `modctl pull`, `skopeo copy` and `crane pull` do the same.

## The reference

```
gethologram.ai/<owner>/<name>:<tag>
```

Lowercase owner and name. `latest` is the indexed revision; a single-file GGUF quantisation is also a valid tag.

```bash
curl -s https://gethologram.ai/v2/hexgrad/kokoro-82m/tags/list
```

```json
{ "name": "hexgrad/Kokoro-82M", "tags": ["latest"] }
```

## The manifest

Ask with the OCI manifest media type and the hub answers a CNCF ModelPack artifact:

```bash
curl -s -H 'Accept: application/vnd.oci.image.manifest.v1+json' \
  https://gethologram.ai/v2/hexgrad/kokoro-82m/manifests/latest
```

```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "artifactType": "application/vnd.cncf.model.manifest.v1+json",
  "config": { "mediaType": "application/vnd.cncf.model.config.v1+json", "digest": "sha256:4aa00e2b…", "size": 5713 },
  "layers": [
    {
      "mediaType": "application/vnd.cncf.model.weight.config.v1.raw",
      "digest": "sha256:7fe8ef14ea6c71598ec59ee78e651962eb6eb769599c5f98f9a45405428efd7f",
      "size": 1913,
      "annotations": { "org.opencontainers.image.title": ".gitattributes", "org.cncf.model.filepath": ".gitattributes" }
    },
    …
  ]
}
```

Each layer is one raw file. Its digest is therefore the file's SHA-256, the same value `tree/main` reports as `oid`, so the artifact and the Hugging Face dialect can never disagree about a byte.

The same route with the Docker manifest media type, or with no `Accept` header at all, answers the GGUF manifest Ollama expects; a model with no GGUF file answers `404` to that request, so send the OCI type for a ModelPack pull. See [Ollama](ollama).

## Blobs

Small layers (config, template, parameters) are served directly. Weight layers are a `307` to a source that is up right now; your client follows it and verifies the digest, as every OCI client already does.

```bash
curl -sI https://gethologram.ai/v2/hexgrad/kokoro-82m/blobs/sha256:7fe8ef14ea6c71598ec59ee78e651962eb6eb769599c5f98f9a45405428efd7f
```

## Clients

| Client | Measured | Note |
| --- | --- | --- |
| oras 1.2.2 | 72 files, 347 MB, 17 s, `sha256sum -c` passes on all | |
| modctl 0.2.2 | pull and extract, all files match | the ModelPack reference tool |
| skopeo | copied the whole artifact, 73 blobs, every one hashing to its name | |
| crane | manifest and a blob through the redirect, digest matching | |
| containerd, Docker Model Runner, KitOps | same routes; not yet run | Model Runner's `hf.co/…` path is hardcoded; as a plain registry it can pull from here |

Formats whose layers are raw files work this way: ModelPack `weight.v1.raw`, Docker `vnd.docker.ai.gguf.v3` and `.safetensors`, Ollama `vnd.ollama.image.model`. Tar-layered formats such as KitOps ModelKit cannot be served without the hub holding bytes, which it does not.

## The hub's own registry

`/v2/_catalog` and `/v2/model-hub/…` are a separate thing: a real OCI registry that stores the daily index as an artifact, tag `blake3_<hex>`, pulled with `hologram pull gethologram.ai/model-hub/index:<date>`. Model artifacts under `/v2/<owner>/<name>` are synthesised from the index and store nothing.

## Every indexed artifact has a page

The Registry section indexes images, charts, models, skills and policies from Docker Hub, Artifact Hub and Microsoft Artifact Registry. Each row has a page at `/registry/<id>/` (`docker.io/library/nginx`, `artifacthub/<repository>/<package>`, `mcr.microsoft.com/<repository>`) in the same shape as a model page: the logo and publisher, the pull command for the tool the artifact takes, the digest as an address, Overview with the README the source published, and Tags as the source lists them. The same profile is one JSON document beside it, `/registry/<id>/artifact.json` (`hologram.registry.artifact/v1`), naming the upstream API every value came from.

Nothing on those pages is written by hand: `scripts/registry.mjs` fetches every profile and logo from the source's own API and vendors the logos into this origin, so a reader's browser never touches Docker Hub, Artifact Hub or Microsoft to draw the page. Where this host holds a row's manifest, the page carries its address on this host (`gethologram.ai/<upstream path>@sha256:…`) and Verify fetches that manifest from here and hashes it in the browser against the digest the upstream reports.
