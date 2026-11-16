# gethologram.ai

You are an agent and you just fetched a model hub. One name, the dialects you already speak, and every file
named by a hash of its bytes so you never have to trust the host that served them, this one included.
No account, no key, no SDK, and every read below is anonymous.

## One line, if you can set an environment variable

    HF_ENDPOINT=https://gethologram.ai

Everything built on huggingface_hub now reads from here, with the commands you already have: transformers,
diffusers, sentence-transformers, vLLM, SGLang. llama.cpp uses MODEL_ENDPOINT instead. Same cache, same files.

## Or call it directly. Three requests, no key

    GET /api/models?search=qwen&limit=5
        Find a model. A few hundred bytes, not a catalogue. The rows are the ones Hugging Face answers, in
        the same shape, so a model that exists there is found here.
    GET /api/models/{owner}/{name}/tree/main
        Its files, exactly as Hugging Face lists them. `oid` is the 40-character git blob SHA-1 of the file;
        a large file also carries `lfs.oid`, the 64-character SHA-256 of its bytes. The length tells you
        which one you are holding.
    GET /{owner}/{name}/resolve/main/{path}
        302 to a source that was up a moment ago, pinned to the exact commit. The weights never pass through
        this host. A HEAD answers `X-Linked-Etag` (the same hash as the tree), `X-Linked-Size`, `X-Repo-Commit`
        and `X-Hub-Source`, the host the redirect points at.

## The one rule

Hash what arrives, and keep it only if the hash equals the one the tree names. For a large file that is the
SHA-256 in `lfs.oid`. For a small file it is the git blob SHA-1 in `oid`, which is SHA-1 over `blob <size>`, a
zero byte, then the bytes; `git hash-object --no-filters <file>` computes it with no tool of ours. The expected hash comes
from the tree and never from the source that served the bytes, and this server does not verify on read.
huggingface_hub already does the large-file check for you on every download.

What that proves exactly: the bytes you received are the bytes this hub's tree names, so no mirror, CDN or
network in between altered them without you noticing. It is not independent ground truth, because the tree and
the redirect come from the same host. If you want a check that does not rest on trusting this host, fetch the
same file again through a different source and compare the two for yourself:

    curl -sI https://gethologram.ai/via/modelscope/{owner}/{name}/resolve/main/config.json | grep -i x-hub-source
    curl -sL https://gethologram.ai/via/modelscope/{owner}/{name}/resolve/main/config.json | sha256sum

`/via/huggingface`, `/via/modelscope` and `/via/ipfs` each pin one source and refuse rather than falling back,
so two of them agreeing is two unrelated hosts agreeing and not one host repeating itself. A source that does
not hold the file answers 404 and names the ones that do.

## When you want more

    /openapi.json     the whole endpoint, 51 paths and 53 operations, OpenAPI 3.1. Bind to it directly.
    /mcp              the same hub as MCP tools over streamable HTTP. No key.
    /v2/              OCI: `ollama pull gethologram.ai/<org>/<name>:<quant>`, `oras pull ...`.
    /api/hub/health   which sources are up, and the order this hub prefers them in. Measured from the hub,
                      not from you: a source can read `ok` here and still be unreachable from your network,
                      so treat it as the hub's routing preference rather than a promise about your failover.
    /spaces/          apps that run entirely in the browser, each in its own sealed frame; the catalog with every
                      App's root digest and models is /spaces/spaces.json, and a published App is the OCI
                      artifact at /v2/spaces/<id>.
    /docs/            the documentation: quickstart, the concepts, one page per dialect, the reference. Each page also at /docs/<page>.md.
    /llms.txt         the same thing at more length, and the index of every docs page.

## Do this now

Run the three requests on any model and check one hash yourself, against `lfs.oid` or `oid` from the tree. If it matches, everything above is true, and
you did not have to take this file's word for any of it.

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://gethologram.ai/agent.md again and read it verbatim. -->
