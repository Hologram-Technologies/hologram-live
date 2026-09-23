---
title: MCP
description: Give a coding agent the hub as three tools, over one URL, with no key.
group: Connect
order: 9
---

A coding agent that can search the hub, list a model's files and get a download URL with the hash it must have can fetch and verify a model without a human in the loop. The hub is an MCP server; add its URL to any client that speaks Streamable HTTP.

```
https://hub.uor.foundation/mcp
```

Stateless, anonymous, `POST` only. No OAuth, no key, nothing to install.

## Connect

| Client | How |
| --- | --- |
| Claude Code | `claude mcp add --transport http hologram-hub https://hub.uor.foundation/mcp` |
| Codex CLI | `codex mcp add hologram-hub --url https://hub.uor.foundation/mcp` |
| Cursor | in `~/.cursor/mcp.json`: `{ "mcpServers": { "hologram-hub": { "url": "https://hub.uor.foundation/mcp" } } }` |
| Any client | server URL `https://hub.uor.foundation/mcp`, transport Streamable HTTP |

Protocol versions `2026-07-28`, `2025-11-25`, `2025-06-18` and `2025-03-26` are accepted.

## The tools

| Tool | Input | Returns |
| --- | --- | --- |
| `search_models` | `query`, `task`, `license`, `format`, `max_weights_gb`, `sort`, `limit` | rows with id, task, library, licence, parameters, weight size, downloads, and where the bytes live |
| `get_model` | `id` | the pinned revision, every file with size and SHA-256, GGUF quantisations, sources with their health |
| `resolve_file` | `id`, `path` | a URL, the SHA-256 it must have, every source's URL and health, and the shell commands that download and verify |

Weight bytes never travel in a tool result. The tool returns the instruction; the agent's shell or engine does the download.

## By hand

The same server, called with `curl`:

```bash
curl -s -X POST https://hub.uor.foundation/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_file","arguments":{"id":"hexgrad/Kokoro-82M","path":"EVAL.md"}}}'
```

The `text` of the result, formatted:

```json
{
  "id": "hexgrad/Kokoro-82M",
  "revision": "f3ff3571791e39611d31c381e3a41a3af07b4987",
  "path": "EVAL.md",
  "size": 534,
  "sha256": "9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658",
  "url": "https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md",
  "served_by_now": "huggingface.co",
  "sources": [
    { "kind": "huggingface.co", "healthy": true, "url": "https://huggingface.co/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md" },
    { "kind": "modelscope.cn",  "healthy": true, "url": "https://modelscope.cn/models/hexgrad/Kokoro-82M/resolve/master/EVAL.md" },
    { "kind": "ipfs",           "healthy": true, "url": "https://ipfs.filebase.io/ipfs/bafybeiauiszvph34uhfnyzxi3uojfcm546hfcyi7cd5ozd272latt5pnnu/EVAL.md" }
  ],
  "download": "curl -L -o \"EVAL.md\" \"https://hub.uor.foundation/hexgrad/Kokoro-82M/resolve/f3ff3571791e39611d31c381e3a41a3af07b4987/EVAL.md\"",
  "verify": "echo \"9b4d7a54809bf22127d19d936d9e749249e1a655e2e0a1df8a6546f77a819658  EVAL.md\" | sha256sum -c",
  "handoff": { "hf": ["export HF_ENDPOINT=https://hub.uor.foundation", "hf download hexgrad/Kokoro-82M EVAL.md"] }
}
```

`download` and `verify` are complete commands. An agent runs the first, then the second, and keeps the file only on `OK`.

## Without MCP

An agent that only has HTTP does not need this server. `/agent.md` is the whole hub on one screen, and `/openapi.json` describes every operation in a form any framework turns into tools. See the [API reference](api).
