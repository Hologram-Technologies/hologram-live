---
title: Errors
description: Every status the hub answers, the shape it comes in, and what to do about it.
group: Reference
order: 12
---

An error you can act on names the fix in its body. The hub answers in the native shape of the dialect you called, so the client you already have parses it.

## Shapes

| Routes | Shape |
| --- | --- |
| Hugging Face dialect, `/v2/…`, `/mcp`, `/api/account/*` | `{ "error": "<one sentence>" }` |
| `/api/v1/objects/…` and the other Hologram Server routes | `{ "code": "<CONSTANT>", "message": "<text>" }` |

## Status codes

| Status | Where | Meaning | Do |
| --- | --- | --- | --- |
| `302` | `/resolve/` | Not an error. The bytes are at `Location`, served by `X-Hub-Source`. | Follow it; then check the hash from the index |
| `307` | `/v2/…/blobs/` | Not an error. A weight layer at a live source. | Your OCI client follows and verifies |
| `404` | `/api/models/<owner>/<name>` | Not in the index. The request was recorded for the next index run. | Use huggingface.co directly for now |
| `404` | `…/revision/<rev>`, `…/tree/<rev>` | Not the indexed revision. The body names the one the hub has. | Ask for `main` or that commit |
| `404` | `…/resolve/<rev>/<path>` | The file is not in that revision. | Check `tree/main` for the path |
| `404` | `/v2/<owner>/<name>/…` | No such model, or no GGUF for a GGUF manifest request. | Check `tags/list`; use lowercase |
| `404` | `/api/v1/objects/<address>` | No object at that address, `LIVE_NOT_FOUND`. | Check the address; the catalog is the map |
| `401` | `/api/account/*` | No valid sign-in token. | Sign in on the site; only its own origin sends the token |
| `401` | `GET /api/v1/objects`, `POST /api/v1/objects` | Listing, searching and publishing need a publisher token. | Read known addresses instead; ask for a token to publish |
| `405` | `GET /mcp` | MCP is `POST` only. | `POST` a JSON-RPC message |
| `409` | `POST /api/account/request` | Already requested by this account. | Nothing; it is queued |
| `429` | `/api/account/*` | Too many writes from one account. | Wait; the reads are never limited |
| `503` | `/api/account/*` | Sign-in is not configured on this deployment. | `GET /api/account/health` says whether it is |

## Examples

Not in the index:

```bash
curl -s https://hub.uor.foundation/api/models/nope/nothing
```

```json
{ "error": "nope/nothing is not in the Hologram index yet. The request was recorded for the next index run; use huggingface.co directly meanwhile." }
```

Wrong revision:

```json
{ "error": "The hub has hexgrad/Kokoro-82M at f3ff3571791e39611d31c381e3a41a3af07b4987 only." }
```

Missing file:

```json
{ "error": "no-such-file is not in hexgrad/Kokoro-82M at f3ff3571791e39611d31c381e3a41a3af07b4987." }
```

Unknown object:

```json
{ "code": "LIVE_NOT_FOUND", "message": "object blake3:0000…0000 not found" }
```

## Things that are not errors but look like one

- A `/via/<source>` name the hub does not know is not refused: the redirect falls back to the default order. `X-Hub-Source` tells you what was chosen. See [Sources and failover](sources).
- A hash mismatch is never reported by the hub, because the hub never sees the bytes. It is reported by you, `sha256sum -c`, Ollama, or your OCI client. See [Verification](verification).
- `Authorization` sent to any read route is dropped at the edge, not rejected. The request proceeds anonymously.
