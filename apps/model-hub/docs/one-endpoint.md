# One endpoint for models: research, decision, contract

2026-09-18. Evidence is dated and sourced; "measured" means run or read by us that day; UNVERIFIED means not checked.
The short version is the first section. Everything after it is the evidence and the contract.

## 1. Summary

**How the world reaches model files.** One read dialect carries almost everything: Hugging Face's Hub API
(`/api/models/{id}`, `/refs`, `/tree`, `/{id}/resolve/{rev}/{path}`), reached through one environment variable,
`HF_ENDPOINT`. `huggingface_hub` had 271 M downloads in 30 days; transformers, diffusers and sentence-transformers all
go through it; vLLM, SGLang, KServe `hf://` and llama.cpp `-hf` read the same variable; every bring-your-own-container
compute platform (Akash SDL, RunPod, Modal, Vast, io.net, Lambda, Nosana) starts a container that calls it. Every mirror
that works (hf-mirror.com, Artifactory, Nexus, Cloudsmith, Olah) is path-compatible behind that variable; the one
alternative with its own API (ModelScope) needs per-tool opt-in flags. About two thirds of `HF_ENDPOINT` uses on GitHub
name a single volunteer mirror, so the habit of pointing that variable somewhere else already exists.

**Is it the OpenAI API?** No: that is the other plane. OpenAI's `/v1` is the de facto standard for *using* a model
(vLLM, llama.cpp, Ollama, Together, Groq, OpenRouter, AkashML, Docker Model Runner all implement it). Its `/v1/models`
is four fields and every implementation lists models that are loaded or hosted, never files. Nobody offers an
"OpenRouter for model files": no product resolves one model id to ranked sources with fallback and an independent
expected hash. That gap is the hub.

**Decision.** One base URL, many dialects, one truth. `https://hub.uor.foundation` is the only thing anyone configures;
each ecosystem points the setting it already has at it and speaks its own language. Every dialect is a stateless view
over the same addressed index, answers metadata, and sends weight requests to a live source as a redirect. The hub
carries names, addresses and directions, never weights. Inference stays out of scope: the hub has no GPUs, and the
engines it feeds already speak OpenAI.

**Shipped and measured today**

| Dialect | The user's one line | Reach | State |
|---|---|---|---|
| Hugging Face Hub API | `export HF_ENDPOINT=https://hub.uor.foundation` | `hf`, `huggingface_hub`, transformers, diffusers, sentence-transformers, vLLM, SGLang, KServe, llama.cpp `-hf`, every BYO-container compute platform | Live. Matrix passes on `huggingface_hub` 1.32.0 and 0.36.2; `HfApi.list_models`, `model_info`, `list_repo_files` work |
| Ollama registry | `ollama pull hub.uor.foundation/<org>/<name>:<quant>` | Ollama and everything built on it | Live. Ollama 0.34.2 pulled 969 MB in 67 s, verified the SHA-256 itself, and ran the model. **Failover measured:** with huggingface.co, hf.co and the CDN blackholed in the client, the same pull succeeded in 83 s from ModelScope, manifest and template from the hub's cache |
| OCI model artifacts (CNCF ModelPack) | `oras pull hub.uor.foundation/<org>/<name>:latest` (lowercase name; a GGUF quantisation also works as the tag) | oras, modctl, KitOps, containerd, Docker Model Runner | Live. Every file is a raw layer whose digest is its SHA-256, every blob a redirect. Measured with oras 1.2.2: a whole model, 72 files, 347 MB in 17 s, `sha256sum -c` passes on all of them. Docker Model Runner and modctl not run: UNVERIFIED |
| MCP | add the server URL `https://hub.uor.foundation/mcp` | Claude, ChatGPT developer mode, Cursor, VS Code, Gemini CLI | Live. Stateless Streamable HTTP, anonymous, three tools (`search_models`, `get_model`, `resolve_file`); measured with the official MCP inspector client: list, calls, and a tool error |
| Hologram objects | `GET /api/v1/objects/{address}` | Agents and mirrors that want the raw, content-addressed truth | Live (Hologram Server) |
| OCI registry | `hologram pull hub.uor.foundation/model-hub/index:<date>` | The daily index | Live (kappa-registry) |

**Next:** one source of truth under all dialects (section 6, step 5); Docker Model Runner and modctl runs; the MCP
registry entry; more IPFS pins (a first batch of 75 permissively licensed models, 199 GB, started 2026-09-18).

## 2. Evidence

### 2.1 Entry points to Hugging Face, ranked (numbers dated 2026-09-18 unless noted)
| # | Entry point | Evidence |
|---|---|---|
| 1 | `huggingface_hub` (every Python path) | 271.1 M downloads / 30 days, 4.46 B all time (pepy.tech); `hf_hub_download` in ~176 k GitHub files, `snapshot_download` in ~134 k |
| 2 | `from_pretrained` | transformers 112.7 M / 30 days, sentence-transformers 26.1 M, diffusers 5.7 M; ~2.81 M GitHub files; 422,404 dependent repositories |
| 3 | Local apps (`ollama run hf.co/…`, `llama -hf`, LM Studio, Docker `hf.co/…`) | GGUF downloads per month: Qwen 39.6 M, Gemma 20.8 M, Llama 7.5 M; GGUF declarations +464 % in 7 months (HF, Aug 2026) |
| 4 | JavaScript | transformers.js 2.65 M / week, `@huggingface/hub` 472 k, `@huggingface/inference` 374 k (npm) |
| 5 | `hf` CLI, including agents | Agents are tagged since April 2026: Claude Code 48.6 M requests, Codex 36.4 M (2026-06-04). In HF's own benchmark the CLI reached 94 % task success against 84 % for curl or the SDK, at up to 6× fewer tokens |
| 6–10 | Gradio clients, Inference Providers router, git + LFS/Xet, MCP server, Inference Endpoints | Small or unpublished; the MCP server URL appears in ~2.1 k GitHub files |

Scale: 3,075,893 public models; 1.5 % of repositories receive 99.2 % of downloads; 41 % of downloads come from China.

**What makes it easy (tested):** the URL is the identity (confirmed); anonymous reads (confirmed, rate-limited per IP);
one function call (confirmed); a cache layout everyone shares, now read by llama.cpp too (confirmed); revisions as git
refs (confirmed, and `X-Repo-Commit` is mandatory, which broke Artifactory once); one variable for a mirror
(confirmed); permissive CORS (measured). Surprise: Hugging Face's OpenAPI document (295 paths) omits `/api/models` and
`/api/models/{id}`, the two most-called routes, and the docs never mention `HF_ENDPOINT`. The compatibility surface is
documented only in source code, which is why we record clients instead of reading docs.

### 2.2 The OpenAI plane and OpenRouter
- `/v1/models` is `{object: "list", data: [{id, object: "model", created, owned_by}]}`; everyone adds their own fields
  (Together: licence, context, pricing; vLLM: `max_model_len`; HF router: `providers[]` with latency and throughput;
  OpenRouter drops `object` and `owned_by`). The one real standardisation effort is Open Responses (Apache-2.0 spec and
  compliance tests), which covers the response loop, not a catalog.
- OpenRouter lessons that transfer to files: (1) one identity string is the whole request, policy in a suffix
  (`:nitro`, `:floor`); (2) a mutable alias over an immutable canonical name, our name over an address; (3) the catalog
  is public, filterable JSON; (4) two levels, a model row and its endpoints with live uptime and latency, our sources
  and their health; (5) the caller declares policy (`order`, `only`, `allow_fallbacks`), the hub executes it, our
  `/via/<source>`; (6) always say who served, our `X-Hub-Source`; (7) one error envelope with a closed vocabulary.

### 2.3 Artifact protocols: can a registry carry zero bytes?
Yes, for clients that accept a redirect on the blob `GET` and verify the digest afterwards, provided each weight layer
is one raw file, because then the layer digest is the file's SHA-256, which is what the index holds.
| Client | Follows a blob redirect | Verifies the digest | Note |
|---|---|---|---|
| Ollama | Yes, exactly `307` off-host, then 16 parallel `Range` parts | Yes ("verifying sha256 digest") | `HEAD` must answer `200` directly: a commit of 2026-09-17 blocks cross-host redirects on manifest and `HEAD`. Measured end to end |
| containerd, Docker Model Runner | Yes (read in source) | Yes | DMR's `hf.co/…` path is hardcoded to huggingface.co; as a plain OCI registry it can pull from us |
| oras, crane, skopeo, modctl | Yes (**measured** against the hub) | Yes (measured: every blob hashes to its digest) | |
Formats with raw-file layers: CNCF ModelPack `weight.v1.raw`, Docker `vnd.docker.ai.gguf.v3` / `.safetensors`,
Ollama `vnd.ollama.image.model`. Tar-layered formats (KitOps ModelKit) cannot be served this way.
Not reachable: `docker model pull hf.co/…`, LM Studio, Jan (endpoints hardcoded).

### 2.4 Agents
- Given a bare URL, a coding agent reads the page and often `/llms.txt`; it does not probe `.well-known` or OpenAPI
  unless told to. MCP is used once a human or the registry adds the server. Spec 2026-07-28 is stateless; an anonymous
  read-only server needs no auth endpoints. Hugging Face's own MCP server has four tools and none downloads or verifies.
- **Measured on our hub:** following `llms.txt`, an agent spends 4 calls and 375 KB (about 94,000 tokens) to resolve
  one model, because the whole catalog object (364 KB, served uncompressed) must be read to find one entry. The same
  question through the Hugging Face dialect costs 655 bytes; a search returns three rows in 2 KB. Agents already run
  the `hf` CLI better than any SDK, so `HF_ENDPOINT` is also the agent interface. `llms.txt` should lead with it.

### 2.5 Compute providers
Bring-your-own-container platforms take weights from a Hugging Face repository id at container start (Akash SDL,
RunPod `MODEL_NAME`, Modal, Vast, io.net, Lambda, Nosana `resources`): the user owns the environment, so `HF_ENDPOINT`
is a drop-in (inferred from their templates; none run by us: UNVERIFIED). Managed platforms (Together, Fireworks,
Baseten, AkashML) accept only named sources; a hub gets in there only as a named integration.

## 3. Compatibility matrix
| Tool | The one setting | Works against the hub |
|---|---|---|
| `hf` CLI, `huggingface_hub` 1.32 / 0.36 | `HF_ENDPOINT` | **Measured**: whole-model download, 30 of 30 files match; cache reuse; typed errors; and the same with Hugging Face blackholed (ModelScope + IPFS, then IPFS alone) |
| transformers (`AutoConfig`, `AutoTokenizer`) | `HF_ENDPOINT` | **Measured** |
| `HfApi.list_models`, `model_info`, `list_repo_files` | `HF_ENDPOINT` | **Measured** |
| plain `curl -L`, `Range` | the URL | **Measured** |
| A browser on another origin (what transformers.js and `huggingface.js` do) | the URL | **Measured** from `humuhumu33.github.io`: model info, a file through the redirect with its SHA-256 matching, the same through `/via/ipfs`, and `Range` reads on 327 MB weights from both sources. Every read route sends `Access-Control-Allow-Origin: *` (it did not before 2026-09-18: only search did). One ranged read of a small JSON file came back empty once, unexplained |
| Ollama 0.34.2 | the name: `hub.uor.foundation/<org>/<name>:<quant>` | **Measured**: pull, verify, run; pull again with Hugging Face blackholed |
| llama.cpp `-hf` (build 11028) | `MODEL_ENDPOINT=https://hub.uor.foundation/` | **Measured**: `-hf bartowski/MiniCPM5-2B-GGUF:IQ2_M` fetched the GGUF through the hub into the shared Hugging Face cache layout and loaded it; generated text not captured in the non-interactive container |
| vLLM, SGLang, KServe `hf://`, TGI | `HF_ENDPOINT` | Use `huggingface_hub`; not run (need a GPU or a large image): UNVERIFIED |
| transformers with torch, diffusers, sentence-transformers full load | `HF_ENDPOINT` | Same client path as measured; full load not run: UNVERIFIED |
| oras 1.2.2 | OCI reference, lowercase: `hub.uor.foundation/hexgrad/kokoro-82m:latest` | **Measured**: whole model pulled, every file matches |
| modctl 0.2.2 (the ModelPack reference tool) | OCI reference | **Measured**: pull and extract, 72 files, `sha256sum -c` passes on all |
| skopeo, crane | OCI reference | **Measured**: skopeo copied the whole artifact (73 blobs, every one hashing to its name); crane fetched the manifest and a blob through the redirect, digest matching |
| Docker Model Runner, KitOps, containerd | OCI reference | Same routes; not run: UNVERIFIED (Model Runner needs a VM or Docker Desktop, not the shared host) |
| MCP clients (Claude, ChatGPT developer mode, Cursor, VS Code, Gemini CLI) | MCP server URL | **Measured** with the official inspector client; not yet added inside each product: UNVERIFIED |
| LM Studio, Jan, `docker model pull hf.co/…` | none | Not reachable |

## 4. The decision (ADR)
**Considered.** (A) A new unified REST API of our own with SDKs. Rejected: every mirror that invented an API needs
per-tool flags; nobody installs a client. (B) An OpenAI-shaped API as the single surface. Rejected: `/v1` is the
inference plane, its model list has no place for files, revisions or hashes, and listing models one cannot chat with
misleads the UIs that read it. (C) **One base URL, many dialects, one truth.** Chosen.

**Rules.**
1. One URL to configure, nothing to install. Use the standard the caller already speaks; never invent a client.
2. The same identity everywhere: `org/name`, pinned to the indexed revision by default; an address is the canonical
   name under it. For SBOMs the purl is `pkg:huggingface/<org>/<name>@<revision>`.
3. Zero weight bytes: metadata is answered, weights are a redirect to a source that passed the last verified probe.
4. Every answer is checkable without a tool of ours: `…/resolve/main/SHA256SUMS` with `sha256sum -c`; Ollama and OCI
   clients verify digests themselves. Expected hashes come from the index only, never from a byte source.
5. The caller may state policy (`/via/<source>`); the hub says who served (`X-Hub-Source`).
6. Errors in each dialect's native shape, with one sentence that names the fix.
7. Credentials that clients send to whatever endpoint they are given are removed at the edge and never logged; gated
   content is refused, not proxied.
8. Dialects are thin stateless adapters (`deploy/hub-resolve.mjs`, one file, no dependencies, 96 MB) until one source
   of truth exists; they move into hologram-live only if upstream wants them. The Hologram Server on the hub exposes
   six paths (objects: get, list, search, put; capabilities; modules): it is the right substrate for the truth and the
   wrong place for someone else's URL shapes (unmatched paths answer `200 application/grpc`, search needs a token,
   objects are served uncompressed, 32 MB ceiling).
9. **Inference: out of scope**, with a kill criterion for revisiting: a provider or engine asks us for a directory of
   where an exact addressed revision is served. Until then the engines we feed are the OpenAI endpoints.

## 5. Path map (one host, no collisions)
| Path | Owner | Dialect | Cache | Auth |
|---|---|---|---|---|
| `/`, `/models/…`, `/data/…`, `/zip/…` (worker), `/archive/…`, `/archive.json`, `/pins.json` | site | website; `/` answers three ways by `Accept` (see below) | short | none |
| `/agent.md`, `/openapi.json`, `/.well-known/openapi.json`, `/.well-known/agent-card.json`, `/robots.txt` | site | agent discovery, all generated from the OpenAPI document | 5 min | none |
| `/api/models`, `/api/models/{id}[/revision/{rev}]`, `/refs`, `/tree/{rev}`, `/xet-read-token/{rev}`, `/{org}/{name}/resolve/{rev}/{path}`, `/via/{source}/…`, `/api/hub/health` | hub-resolve | Hugging Face | 60 s metadata, redirects uncached | `Authorization` and `Cookie` stripped |
| `/v2/{org}/{name}/manifests/{tag or digest}`, `/blobs/{digest}`, `/tags/list` (org ≠ `model-hub`) | hub-resolve | Ollama registry, or CNCF ModelPack when the client accepts `application/vnd.oci.image.manifest.v1+json` | uncached | stripped |
| `/v2/`, `/v2/model-hub/…`, `/_*` | kappa-registry | OCI distribution, the daily index | registry | token for writes |
| `/api/v1/objects[/{id}]`, `/api/v1/capabilities`, `/api/v1/modules`, `/docs`, `/healthz` | Hologram Server | Hologram objects | immutable per address | publisher token for writes |
| `/openapi.json`, `/.well-known/openapi.json`, `/.well-known/agent-card.json`, `/robots.txt` | site | one OpenAPI 3.1 document over every dialect, built by `web/scripts/openapi.build.mjs` | 5 min | none |
| `/.well-known/model-hub.json`, `/llms.txt` | site | agent discovery | short | none |
| `/mcp` | hub-resolve | MCP (POST only, stateless) | uncached | none |
| `/api/account`, `/api/account/*` | hub-account | the only surface that needs a person: saved list, attributed model request, recorded interest in publishing. Declared before `@server_other`; answers its own origin only; `Authorization` is NOT stripped here, because it carries the visitor's own Privy token | uncached | Privy access token |
| `/v1/…` | — | reserved and deliberately empty: inference is out of scope | — | — |

## 6. Sequencing, each with its adoption signal and kill criterion
1. **Failover proof: done.** With Hugging Face blackholed in the client, `hf download` and `snapshot_download` deliver
   the whole model (30 of 30 files matching) in 110 s; with ModelScope down too, from IPFS alone in 126 s. Signal to
   keep watching: reroutes per week in the log. It holds for the eleven pinned models and for whatever ModelScope
   mirrors; elsewhere Hugging Face remains a single source.
2. **`llms.txt` leads with the short way: done** (`HF_ENDPOINT`, `ollama pull`, the search route, MCP). Signal: agent
   user-agents on `/api/models`.
3. **OCI model artifacts: done** on the same `/v2/{org}/{name}` routes (ModelPack raw layers), measured with oras.
   Left: run Docker Model Runner and modctl against it. Kill: if no OCI client user-agent appears in 60 days.
4. **MCP server: done** (three tools, stateless, anonymous). Left: publish `deploy/mcp-server.json` to the MCP registry, which needs the `uor.foundation` domain verified with `mcp-publisher` (Ilya). Kill: fewer tool calls than `/api/models` searches from agents after 60 days.
5. **One truth.** Today the dialects read the site's published file lists and the address index, while the agent API
   reads objects. Move the dialects onto the objects (they need a by-name lookup the object API lacks: a small upstream
   PR or a name→address map published daily), then delete the duplicate lists.

## 7. Decisions for Ilya
1. Confirm inference stays out of scope (recommended).
2. Which audience next quarter decides between step 3 and step 4.
3. Wording: "works with `HF_ENDPOINT`", "pull with Ollama": compatibility stated as the setting, never as affiliation.
4. More IPFS pins (failover today covers 11 models beyond Hugging Face and ModelScope) needs a paid Filebase plan.

## 8. One document over every dialect

Section 4 rejected inventing a REST API of our own, and that stands: no route here is new and no client needs
changing. What was missing was a machine-readable description of the endpoint that already exists. `/openapi.json`
was the Hologram Server's own document: six paths, no `servers`, no security schemes, no examples, and no mention of
the Hugging Face dialect, the registry, MCP or the discovery files. About four fifths of the endpoint was reachable
but undescribed, so a framework had to be told about it by a person.

One OpenAPI 3.1 document now covers all of it, 39 paths and 41 operations, built by `web/scripts/openapi.build.mjs`
and shipped with the site. It keeps the server's generated document as the source of truth for the server's own six
paths and patches what that document leaves empty, so the two can never drift; every example in it is copied from a
recording of the live endpoint (`web/qa/openapi/evidence.json`), never written by hand. `/docs` renders it unchanged,
because Scalar loads the spec from the same URL. The agent card and `robots.txt` are generated from the same document,
so a skill cannot say one thing in one place and something else in another.

Three gates keep it honest, all in `web/qa/openapi`:

| Gate | What it proves |
|---|---|
| `redocly lint` | The document is valid and complete. Both relaxed rules are written down in `scripts/redocly.yaml` with their reason |
| `conformance.mjs` | Forward: every operation is called against the live hub and its status, media type and body are checked against the document. Reverse: every route the hub answers is matched back to a path in it, so surface cannot be added without describing it |
| `bind.mjs` | Given only the host name, an agent discovers the document, turns all 41 operations into tool definitions in the OpenAI, Anthropic and MCP shapes, and completes find, describe, resolve, download and SHA-256 verification using nothing else. The same chain is then run through `/mcp` |

Measured 2026-09-23 against the built document and the live hub: 19 assertions pass in `bind.mjs` with a real
download hashing to the value the index gave, and 86 of 86 in `conformance.mjs` once the three files it adds are
deployed. The gates found four defects in the first draft of the document and two on the endpoint itself: `/docs`
answered but was undescribed, and the account service, the one surface a signed-in person can write to, was missing
from the document entirely. Both are now covered.

Left: the two dialect surfaces are described but still read a different source from the agent API (step 5 of the
sequencing above). The document makes that split visible, it does not close it.

## 9. One line

The endpoint had a discovery problem shaped like its hero. The landing page showed `hub.uor.foundation` and copied
`HF_ENDPOINT=https://hub.uor.foundation`, so the useful half was the half nobody could see; and the bare name
answered 77 KB of markup to `curl`, because curl, node's `fetch` and python's `requests` all send `Accept: */*` and
only an explicit `Accept: application/json` was negotiated. An agent handed nothing but the host name got a page it
could not use.

`/` now answers three ways. `text/html` gets the site. `application/json` gets the descriptor, as before. Anything
else gets `agent.md`: the whole hub on one screen, in the imperative — the one environment variable, the three
requests, and the rule that makes the bytes safe — ending in a check the reader runs itself. Known link unfurlers
are excepted by user agent, because a preview of markdown is a worse card than a preview of the page. `agent.md` is
generated from the OpenAPI document, so it cannot name a route that does not exist, and it ends in a canary comment:
a fetcher that summarises drops it, and an agent that cannot see it knows to fetch the file again verbatim.

The hero is now that line and nothing else: `curl hub.uor.foundation`. Measured against the ten API-first products
surveyed on 2026-09-23, `llms.txt` is near universal (9 of 10) and a root `/openapi.json` is common (4 of 10), but
only OpenRouter addresses the agent directly (`/learn.md`, `/agents.md`), only Resend ships a `/.well-known/mcp.json`,
and **none of the ten uses `rel="service-desc"`**, the registered relation (RFC 8631) for an API description. The
head now carries it, beside `service-doc`, `llms-txt` and a schema.org `WebAPI` block, so an agent that never renders
a pixel finds the same line.

`bind.mjs` gates the claim: it makes the request curl makes and fails if the answer is markup or if it does not show
how to fetch a file and how to check it.

## 10. Sources
pepy.tech and pypistats.org (PyPI), api.npmjs.org (npm), GitHub code search (measured counts);
huggingface.co/.well-known/openapi.json; huggingface.co/docs/hub/ollama, /agents-mcp, /rate-limits;
huggingface.co/blog/hf-cli-for-agents, /state-of-open-models-summer-2026, /jeffboudier/jfrog-artifactory-june-2026;
github.com/huggingface/huggingface_hub (constants.py, hf_api.py); github.com/ggml-org/llama.cpp (common.cpp,
hf-cache.cpp); github.com/ollama/ollama (server/images.go, download.go, auth.go; commit dfabde4);
github.com/opencontainers/distribution-spec and image-spec; github.com/modelpack/model-spec;
github.com/docker/model-runner; github.com/openai/openai-openapi; openrouter.ai/docs (models, provider routing,
fallbacks, errors, generation); openresponses.org; modelcontextprotocol.io/specification/2026-07-28;
github.com/package-url/purl-spec (types/huggingface-definition.json); provider docs for Akash, RunPod, Modal, Baseten,
Together, Fireworks, Vast, io.net, Nosana. Recordings: `web/qa/hf-dialect/`.
