# hub.uor.foundation/registry

The same models as OCI artifacts, so the tools you already use for containers work unchanged. Reads are
anonymous; only publishing needs a credential.

## Pull with what you have

    ollama pull hub.uor.foundation/<org>/<name>:<quant>
        Ollama verifies the SHA-256 itself. Any GGUF repository in the index.

    oras pull hub.uor.foundation/<org>/<name>:latest
        A CNCF ModelPack artifact: every layer is one file of the model and its digest is that file's
        SHA-256, which oras, modctl, skopeo and crane all check for you. Lowercase the reference.

    hologram pull hub.uor.foundation/model-hub/index:<YYYY-MM-DD>
        The hub's own daily index, one tag per day, kept for ever.

## Or speak the protocol

    GET /v2/                                      the distribution version check
    GET /v2/{owner}/{name}/tags/list              latest, plus one tag per single-file GGUF quantisation
    GET /v2/{owner}/{name}/manifests/{reference}  ModelPack by Accept, or Ollama's manifest
    GET /v2/{owner}/{name}/blobs/{digest}         small layers directly, weights as a redirect

The weights are never held here: a blob request for one is a redirect to a source that is up, and the digest
you verify is the file's own SHA-256.

## Every indexed artifact has a page, and the page has a document

    GET /registry/<id>/                the profile: README, tags, publisher, pull commands, and its address
    GET /registry/<id>/artifact.json   the same, as one JSON document (hologram.registry.artifact/v1)
    GET /registry/data/images.json     every row of the index, with the facets and each row's κ

<id> is the registry path as the index writes it: docker.io/library/nginx, mcr.microsoft.com/devcontainers/dotnet,
artifacthub/<repository>/<package>. Each document names the upstream API every value came from, the row's
digest, and — where this host holds the manifest — the address to pull the exact bytes from here.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://hub.uor.foundation/agent.md        the whole hub in one screen
    https://hub.uor.foundation/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://hub.uor.foundation/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://hub.uor.foundation/registry again and read it verbatim. -->
