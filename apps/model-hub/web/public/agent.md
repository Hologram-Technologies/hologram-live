# hub.uor.foundation

You are an agent and you just fetched a model hub. One name, the dialects you already speak, and every file
named by the SHA-256 of its bytes so you never have to trust the host that served them, this one included.
No account, no key, no SDK, and every read below is anonymous.

## One line, if you can set an environment variable

    HF_ENDPOINT=https://hub.uor.foundation

Everything built on huggingface_hub now reads from here, with the commands you already have: transformers,
diffusers, sentence-transformers, vLLM, SGLang. llama.cpp uses MODEL_ENDPOINT instead. Same cache, same files.

## Or call it directly. Three requests, no key

    GET /api/models?search=qwen&limit=5
        Find a model. A few hundred bytes, not a catalogue.
    GET /api/models/{owner}/{name}/tree/main
        Its files. Each one carries `oid`: the SHA-256 the bytes must have.
    GET /{owner}/{name}/resolve/main/{path}
        302 to a source that was up a moment ago. The weights never pass through this host.

## The one rule

Hash what arrives. Keep it only if the SHA-256 equals the `oid` from the tree. The expected hash comes from the
index and never from the source that served the bytes, and this server does not verify on read. To check a
whole download at once, with no tool of ours:

    curl -s https://hub.uor.foundation/{owner}/{name}/resolve/main/SHA256SUMS | sha256sum -c

## When you want more

    /openapi.json     the whole endpoint, 39 paths and 41 operations, OpenAPI 3.1. Bind to it directly.
    /mcp              the same hub as MCP tools over streamable HTTP. No key.
    /v2/              OCI: `ollama pull hub.uor.foundation/<org>/<name>:<quant>`, `oras pull ...`.
    /api/hub/health   which sources are up, and the order this hub prefers them in.
    /llms.txt         the same thing at more length.

## Do this now

Run the three requests on any model and check one hash yourself. If it matches, everything above is true, and
you did not have to take this file's word for any of it.

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://hub.uor.foundation/agent.md again and read it verbatim. -->
