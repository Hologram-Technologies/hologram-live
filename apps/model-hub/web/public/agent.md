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
        Find a model. A few hundred bytes, not a catalogue. This lists everything the hub holds, including
        models that have fallen off the trending list; those come back as thin rows carrying the name and
        `hologram.listed: false`, because their browse facts were never published with the bytes.
    GET /api/models/{owner}/{name}/tree/main
        Its files. Each one carries `oid`: the SHA-256 the bytes must have.
        Read that field carefully if you already know this dialect. Hugging Face puts a 40-character git
        blob SHA-1 in `oid` and hides the SHA-256 inside `lfs` on large files only. Here `oid` is always
        the SHA-256 of the file, 64 characters, for every file large or small. The length tells you which
        one you are holding.
    GET /{owner}/{name}/resolve/main/{path}
        302 to a source that was up a moment ago. The weights never pass through this host.

## The one rule

Hash what arrives. Keep it only if the SHA-256 equals the `oid` from the tree. The expected hash comes from the
index and never from the source that served the bytes, and this server does not verify on read. To check a
whole download at once, with no tool of ours:

    curl -s https://hub.uor.foundation/{owner}/{name}/resolve/main/SHA256SUMS | sha256sum -c

What that proves exactly: the bytes you received are the bytes this index names, so no mirror, CDN or network
in between altered them without you noticing. It is not independent ground truth, because the index and the
checksum file come from the same host. If you want a check that does not rest on trusting this host, fetch the
same file again through a different source and compare the two for yourself:

    curl -sI https://hub.uor.foundation/via/modelscope/{owner}/{name}/resolve/main/config.json | grep -i x-hub-source
    curl -sL https://hub.uor.foundation/via/modelscope/{owner}/{name}/resolve/main/config.json | sha256sum

`/via/huggingface`, `/via/modelscope` and `/via/ipfs` each pin one source and refuse rather than falling back,
so two of them agreeing is two unrelated hosts agreeing and not one host repeating itself. A source that does
not hold the file answers 404 and names the ones that do.

## When you want more

    /openapi.json     the whole endpoint, 48 paths and 50 operations, OpenAPI 3.1. Bind to it directly.
    /mcp              the same hub as MCP tools over streamable HTTP. No key.
    /v2/              OCI: `ollama pull hub.uor.foundation/<org>/<name>:<quant>`, `oras pull ...`.
    /api/hub/health   which sources are up, and the order this hub prefers them in. Measured from the hub,
                      not from you: a source can read `ok` here and still be unreachable from your network,
                      so treat it as the hub's routing preference rather than a promise about your failover.
    /spaces/          apps that run entirely in the browser, each in its own sealed frame; the catalog with every
                      Space's root digest and models is /spaces/spaces.json, and a published Space is the OCI
                      artifact at /v2/spaces/<id>.
    /docs/            the documentation: quickstart, the concepts, one page per dialect, the reference. Each page also at /docs/<page>.md.
    /llms.txt         the same thing at more length, and the index of every docs page.

## Do this now

Run the three requests on any model and check one hash yourself. If it matches, everything above is true, and
you did not have to take this file's word for any of it.

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://hub.uor.foundation/agent.md again and read it verbatim. -->
