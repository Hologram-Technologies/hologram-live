---
title: Limits and scope
description: What the hub does not do, how big things can be, and where the edges are.
group: Reference
order: 13
---

Knowing what a thing refuses to do is half of trusting it. This page is the other half of the hub.

## Scope

| In | Out |
| --- | --- |
| Finding a model, listing its files, getting a URL and the hash | Serving weights; every byte comes from a source |
| Trending open models, indexed daily | All of Hugging Face; a model not in the index answers `404` and is queued |
| One revision per model: the one indexed | Arbitrary revisions or branches |
| Public models | Gated and private models; they are refused, not proxied |
| Reading, anonymously | Inference; `/v1/` is reserved and deliberately empty |

Inference stays out because the hub has no GPUs, and the engines it feeds already speak the OpenAI API.

## Sizes

| Limit | Value |
| --- | --- |
| Models in the index | about 500 on the site and in search, refreshed daily; 575 in today's catalog object |
| `limit` on `/api/models` | 1 to 500, default 50 |
| Publish, one request at the edge | 8 MiB |
| Publish, one message at the server | 32 MiB; larger payloads go as 16 MiB chunks joined by a source record |
| Catalog object | about 380 KB, uncompressed; read it once a day, not once a query |

The server states its own ceiling; ask it rather than this page when they disagree:

```bash
curl -s https://gethologram.ai/api/v1/capabilities
```

```json
{
  "protocol_version": 1,
  "server_version": "1.0.0",
  "server_id": "blake3:f0f219a02da8703831b6ae0c97c74cbb9dba43b890151d9fb5e036be9412c50e",
  "role": "node",
  "maximum_message_bytes": 33554432,
  …
}
```

Whether sign-in is available on this deployment:

```bash
curl -s https://gethologram.ai/api/account/health
```

```json
{ "ok": true, "configured": true }
```

## Rates

Reads are anonymous and not metered by the hub. The sources the hub redirects to have their own limits: Hugging Face rate-limits anonymous downloads per IP, and the IPFS gateway is slower on a cold read. Writes to `/api/account/*` answer `429` when one account writes too often.

## Caching

| Path | Cache |
| --- | --- |
| `/api/v1/objects/<address>` | forever; an address never changes meaning |
| `/api/models…`, `/tree/…`, `/refs` | 60 s |
| `/resolve/…` redirects | not cached; the source may change between two requests |
| `/openapi.json`, `/agent.md`, `/robots.txt` | 5 min |

## Compatibility

| Works | Does not |
| --- | --- |
| Anything that reads `HF_ENDPOINT` or `MODEL_ENDPOINT` | LM Studio, Jan, `docker model pull hf.co/…`: the host is hardcoded |
| Any OCI client that follows a blob redirect and verifies digests | Formats with tar layers, such as KitOps ModelKit |
| Ollama 0.34 and later | |
| MCP clients on Streamable HTTP | MCP over stdio or SSE |
| A browser on any origin: every read route sends `Access-Control-Allow-Origin: *` | |

## Reporting

A model that should be in the index: `POST /api/account/request` from a signed-in account, or open an issue on the [repository](https://github.com/Hologram-Technologies/hologram-live). A file whose bytes never match the index from any source is an index bug; report it the same way, with the model id, the path and the SHA-256 you observed.
