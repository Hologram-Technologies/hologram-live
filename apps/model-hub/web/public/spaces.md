# gethologram.ai/spaces

Apps that run entirely in the visitor's browser, each sealed under one address. Nothing runs on a server:
the page is a folder of static files, the model comes from the host the App names, and the work happens on
the visitor's own GPU.

## What is here

    GET /spaces/spaces.json
        The catalog: every App, and the SHA-256 root its whole folder seals under.

    GET /v2/spaces/{id}/manifests/latest
        A published App as an OCI artifact — config `holospace.json`, one layer per file, artifact type
        `application/vnd.hologram.space.v1+json`, the sealed root in an annotation. Reads need no token.
        An App that is listed but not yet published answers 404 here, and that is not an error.

## What sealed means

Every file of an App is named in `holospace.lock.json` with its SHA-256, and the lock's `root` is the
SHA-256 over that map. Change one byte anywhere and the root changes. Before an App's code runs, its model
files are held until their bytes re-derive to the digest the model index gave; a byte that does not match is
refused, so the app sees a failed load rather than a wrong file.

Each App runs in a sandboxed frame with its own storage and a policy allowing exactly its own files, the
brand kit, and the one model host it declares. No CDN script, no third party, no camera, no microphone.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://gethologram.ai/agent.md        the whole hub in one screen
    https://gethologram.ai/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://gethologram.ai/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://gethologram.ai/spaces again and read it verbatim. -->
