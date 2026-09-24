---
title: Sources and failover
description: Where the bytes come from, how the hub picks a source, and how you pin one.
group: Concepts
order: 5
---

When Hugging Face is slow or down, a download that names only Hugging Face waits with it. The hub names the file instead of the host, and sends you to whichever source passed its last probe.

## How a source is chosen

The hub keeps a health record per source and an order of preference. A `/resolve/` request is redirected to the first source in that order that is up and has the file. Every redirect says who was chosen in `X-Hub-Source`.

```bash
curl -s https://hub.uor.foundation/api/hub/health
```

```json
{
  "sources": {
    "huggingface.co": { "ok": true, "checked": "2026-09-23T21:45:36.370Z", "reason": "verified" },
    "modelscope.cn":  { "ok": true, "checked": "2026-09-23T21:45:37.526Z", "reason": "verified" },
    "ipfs":           { "ok": true, "checked": "2026-09-23T21:45:37.596Z", "reason": "verified" }
  },
  "order": ["huggingface.co", "modelscope.cn", "ipfs"]
}
```

| Source | Covers | Notes |
| --- | --- | --- |
| `huggingface.co` | every indexed model | the origin; always also a valid URL for any file |
| `modelscope.cn` | models ModelScope mirrors | served at ModelScope's `master` branch of the same commit |
| `ipfs` | models the hub has pinned | a CID per revision; `/pins.json` lists them |

The `hologram.sources` list in a search row tells you which of these hold a given model before you ask.

## Pin a source

Put `/via/<source>` in front of any `/resolve/` path. The expected hash is unchanged; only the host is.

```bash
curl -sI https://hub.uor.foundation/via/ipfs/hexgrad/Kokoro-82M/resolve/main/EVAL.md
```

```
HTTP/1.1 302 Found
Location: https://ipfs.filebase.io/ipfs/bafybeiauiszvph34uhfnyzxi3uojfcm546hfcyi7cd5ozd272latt5pnnu/EVAL.md
X-Hub-Source: ipfs
```

| Name | Pins |
| --- | --- |
| `huggingface` | huggingface.co |
| `modelscope` | modelscope.cn |
| `ipfs` | the IPFS gateway |

A name outside this table is not refused: the hub falls back to its normal order and `X-Hub-Source` reports what it actually chose. If a pin matters to you, read that header.

`/via/<source>/api/models` runs a search with the source pinned for the rows it returns, so a client that sets `HF_ENDPOINT=https://hub.uor.foundation/via/ipfs` reads only from IPFS.

## What the hub never does

- It never proxies bytes. Every `/resolve/` answer is a `302`; the `Range` requests and the bandwidth are between you and the source.
- It never forwards credentials. `Authorization` and `Cookie` headers sent to the dialect routes are removed at the edge and not logged. Gated or private models are refused, not proxied.
- It never trusts a source about a hash. See [Verification](verification).

## Crawlers

`/robots.txt` allows the site and the discovery documents and disallows every route that hands out bytes, including `/via/`, `/v2/` and `/*/resolve/`. A crawler that followed them would cost the sources traffic and keep nothing.
