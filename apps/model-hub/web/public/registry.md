# gethologram.ai/registry

The same models as OCI artifacts, so the tools you already use for containers work unchanged. Reads are
anonymous; only publishing needs a credential.

## What this registry holds, and what it only describes

The page lists every repository the hub knows about, most of them held by Docker Hub, Artifact Hub or
Microsoft Artifact Registry and pulled from there. Only the ones `/v2/_catalog` returns are served from
here. Ask it rather than the page count:

    GET /v2/_catalog?n=1000
        every repository this registry serves the bytes of

## Pull with what you have

    ollama pull gethologram.ai/<org>/<name>:<quant>
        Ollama verifies the SHA-256 itself. Any GGUF repository in the index.

    oras pull gethologram.ai/<org>/<name>:latest
        A CNCF ModelPack artifact: every layer is one file of the model and its digest is that file's
        SHA-256, which oras, modctl, skopeo and crane all check for you. Lowercase the reference.

    hologram pull gethologram.ai/model-hub/index:<YYYY-MM-DD>
        The hub's own daily index, one tag per day, kept for ever.

## Or speak the protocol

    GET /v2/                                      the distribution version check
    GET /v2/{owner}/{name}/tags/list              latest, plus one tag per single-file GGUF quantisation
    GET /v2/{owner}/{name}/manifests/{reference}  ModelPack by Accept, or Ollama's manifest
    GET /v2/{owner}/{name}/blobs/{digest}         small layers directly, weights as a redirect

The weights are never held here: a blob request for one is a redirect to a source that is up, and the digest
you verify is the file's own SHA-256.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://gethologram.ai/agent.md        the whole hub in one screen
    https://gethologram.ai/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://gethologram.ai/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://gethologram.ai/registry again and read it verbatim. -->
