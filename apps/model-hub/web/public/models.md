# gethologram.ai/models

Open models: find one, prove it, and fetch it from whichever source is up. No account, no key, no SDK, and
every request below is anonymous.

To read the whole catalogue rather than a page of it, start at the descriptor and fetch the one address it
names: `curl -H 'accept: application/json' https://gethologram.ai/models` gives `catalog.address`, and that object is
every model in one request. `/api/models` is capped at 500 with no cursor, so it cannot enumerate a
catalogue larger than that.

## One line, if you can set an environment variable

    HF_ENDPOINT=https://gethologram.ai

Everything built on huggingface_hub then reads from here with the commands you already have. llama.cpp uses
MODEL_ENDPOINT instead.

## Or four requests

    GET /api/models?search=qwen&filter=gguf&limit=5
        Find one. A few hundred bytes. This lists everything the hub holds, including models that have
        fallen off the trending list; those carry `hologram.listed: false` and only their name is certain.

    GET /api/models/{owner}/{name}/tree/main
        Its files, each with `oid`: the SHA-256 the bytes must have. This is the only place an expected
        hash may come from. Note it is a 64-character SHA-256, where Hugging Face puts a 40-character git
        blob SHA-1 in the same field.

    GET /{owner}/{name}/resolve/main/{path}
        302 to a source that was up a moment ago, carrying the expected hash in `ETag` and the server in
        `X-Hub-Source`. No weight byte passes through this host.

    GET /{owner}/{name}/resolve/main/SHA256SUMS
        The whole model's checksums, synthesised, so `sha256sum -c` checks a download with no tool of ours.

## Choosing the source yourself

    GET /via/{huggingface|modelscope|ipfs}/{owner}/{name}/resolve/main/{path}
        Pins one source. It refuses rather than falling back, so fetching the same file through two of them
        and comparing is two unrelated hosts agreeing and not one host repeating itself.

    GET /api/hub/health
        Which sources are up and the order this hub prefers them. Measured from the hub, not from you.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://gethologram.ai/agent.md        the whole hub in one screen
    https://gethologram.ai/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://gethologram.ai/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://gethologram.ai/models again and read it verbatim. -->
