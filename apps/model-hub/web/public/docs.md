# gethologram.ai/docs

The written documentation, in reading order. Every page has a Markdown twin at the address below: the same
words, with no navigation, no markup and no scripts.

## Start

    GET /docs/index.md
        Overview. What Hologram Hub is, what it replaces, and the one command that proves it.
    GET /docs/quickstart.md
        Quickstart. Find a model, download one file, and prove the bytes, in three requests and no key.

## Concepts

    GET /docs/verification.md
        Verification. The one rule that makes a download from anywhere safe, and the three ways to apply it.
    GET /docs/addresses.md
        Names, revisions and addresses. What a model id means on the hub, why main does not move, and how an address pins a version forever.
    GET /docs/sources.md
        Sources and failover. Where the bytes come from, how the hub picks a source, and how you pin one.
    GET /docs/spaces.md
        Apps. Apps that run entirely in your browser, each in its own sealed frame, with every model byte verified before it is used.

## Connect

    GET /docs/huggingface.md
        Hugging Face tools. Point the tools you already have at the hub with one environment variable.
    GET /docs/ollama.md
        Ollama. Pull any GGUF in the index straight into Ollama, verified by Ollama itself.
    GET /docs/oci.md
        OCI artifacts. Pull a whole model as a CNCF ModelPack artifact with oras, modctl, skopeo or crane, every file verified by its digest.
    GET /docs/mcp.md
        MCP. Give a coding agent the hub as three tools, over one URL, with no key.
    GET /docs/objects.md
        Objects. The raw, content-addressed index under every dialect, and how to read, mirror or publish to it.

## Reference

    GET /docs/api.md
        API reference. Every route the hub answers, grouped by what it is for, generated from the OpenAPI document.
    GET /docs/errors.md
        Errors. Every status the hub answers, the shape it comes in, and what to do about it.
    GET /docs/limits.md
        Limits and scope. What the hub does not do, how big things can be, and where the edges are.

If you would rather have it in one request, `/llms.txt` is this same list with the endpoints appended.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://gethologram.ai/agent.md        the whole hub in one screen
    https://gethologram.ai/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://gethologram.ai/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://gethologram.ai/docs again and read it verbatim. -->
