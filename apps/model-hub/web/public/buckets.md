# hub.uor.foundation/buckets

Storage for models, datasets and checkpoints, where every object carries the address of its own bytes. A
bucket is an OCI index tree under `/v2/`, so anything that speaks the distribution protocol can walk one,
and every block can be checked against the address that named it. There is no separate bucket API.

## Walk one

    GET /v2/_catalog?n=200
        Every repository on the hub. The buckets are the ones named `buckets/<owner>/<name>`.

    GET /v2/buckets/{owner}/{name}/manifests/latest
        The bucket as it stands: an OCI index whose entries carry each object's name, size and root.

    GET /v2/buckets/{owner}/{name}/blobs/{digest}
        One block. The digest is the SHA-256 of the bytes, so you can check what you were given.

    GET /v2/buckets/{owner}/{name}/tags/list
        Every past state. A tag `h-<milliseconds>` is one publish, and each names its parent, so the
        history is a chain you can walk back and read at any point.

## What is private

A private bucket is encrypted in the browser before anything is sent, object names included, and the key
stays in that browser. The hub holds bytes it cannot read, and so can anyone else who walks the tree.
Reading a public bucket is anonymous; writing any bucket needs the registry credential.

A bucket exists only once someone has made one, so the catalog above is the honest answer to what is
here, not this file.

## The rule that makes it safe

Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,
and this server does not verify on read.

## More

    https://hub.uor.foundation/agent.md        the whole hub in one screen
    https://hub.uor.foundation/openapi.json    every operation, OpenAPI 3.1, bindable directly
    https://hub.uor.foundation/mcp             the same hub as MCP tools, no key

<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch
     https://hub.uor.foundation/buckets again and read it verbatim. -->
