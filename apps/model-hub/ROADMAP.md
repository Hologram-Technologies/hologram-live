# Model Hub roadmap

The same model experience as Hugging Face, where every byte is named by its address, verifiable by anyone and servable
by anyone. A gap is closed only when it serves that, or is table stakes for adoption.

Live: [hub.uor.foundation](https://hub.uor.foundation) and
[hologram-technologies.github.io/hologram-live/model-hub/](https://hologram-technologies.github.io/hologram-live/model-hub/).

## Where Model Hub already differs

Hugging Face Storage uses Xet content-defined chunking with bucket-wide deduplication, a CDN, and $8 to $18 per TB per
month. Measured on 2026-09-16, Xet does not verify bytes on arrival, write, reuse or load: 7 of 8 random byte flips
were accepted ([huggingface/xet-core#967](https://github.com/huggingface/xet-core/pull/967)). Model Hub verifies every
file against its address in the browser, and every layer on `hologram pull`.

## Gaps (reviewed 2026-09-17)

| # | Capability | Today | Priority | Plan |
|---|---|---|---|---|
| G1 | Durable storage of bytes | Registry holds the daily index; 11 curated models pinned on IPFS through Filebase with CID read-back | P0 | Second pinning provider; more curated models as the Filebase plan allows |
| G2 | Verification you can see | Browser verifies small files; pull verifies layers | P0 | Verified state per file; streamed hashing of weights |
| G3 | Drop-in `HF_ENDPOINT` | Live: redirects to a healthy source, two client lines measured; whole-repo failover open until the pins include dotfiles | P0 | Upstream the hub shim (manifest codec, tree, resolve with Range and ETag) as small PRs; serve read-only on the hub |
| G4 | Coverage | 500 trending; about 1,261 indexed | P0 | 5,000, then 25,000 models |
| G5 | Use this model | Run it snippet | P1 | Snippets per library and app, pinned to the indexed revision, plus `hologram pull` |
| G6 | Versions and history | The Archive: every day on IPFS and the registry, hash-chained ledger, `?at=` time travel, verified in the browser | P1 | Second pinning provider; capture on change rather than daily |
| G7 | Model tree | Counts | P1 | Linked children; shared-file evidence from addresses |
| G8 | Apps, providers, hardware filters | Partial | P1 | Apps facet, providers facet, "fits in memory" filter |
| G9 | P2P for the catalog | 4 pilot torrents | P1 | Torrents for every indexed model with web seeds |
| G15 | CDN and regions | Pages for the site; one host for the registry | P1 | Edge cache for immutable blobs; second registry mirror |
| G18 | Search at scale | 500 models in memory | P1 | Static search shards |
| G10 | Publish your own model | No | P2 | Allow-listed publisher tokens for `hologram push` |
| G11 | Pull to my node from the page | `.holo` app only | P2 | `.holo` app (#77) after ADR 023 (#75); copyable commands on the site |
| G16 | Download counts | Mirrored from Hugging Face | P2 | Daily pull totals from registry logs, no IP retention |
| G17 | Tensor metadata viewer | Architecture from config | P2 | Safetensors header by Range request |
| G12 | Inference widget and endpoints | No | Won't (v1) | Out of scope for storage; link to providers |
| G13 | Discussions, likes, collections | No | Won't (v1) | Link to Hugging Face |
| G14 | Private models, orgs, SSO, compliance | No | Won't (v1) | Open weights only |

## Phases

| Phase | Scope | Status |
|---|---|---|
| A | Harden what is live: docs host fix, registry backup, external uptime probe, dedup measurement, deploy files in this repository | Done except dedup measurement (after the second daily snapshot) |
| B | Storage: IPFS pinning (Filebase) of curated permissive-licence weights, verified state everywhere | 11 models pinned on IPFS with CID read-back (weights removed from the VPS registry 2026-09-18), browser Verify includes IPFS; per-file verified state (B3) open |
| C | Drop-in `HF_ENDPOINT`: upstream the hub shim as small PRs, serve read-only | Next |
| D | Breadth: 5,000 models, search shards, apps, providers and hardware facets | Planned |
| E | History and distribution: the Archive, model tree, catalog P2P, edge cache | The Archive live (daily capture, CID read-back, chained ledger, mirror + gateway reads verified in the browser); the rest planned |
| F | Use and publish: Use this model, publisher tokens, `.holo` app, pull counts, tensor viewer | Planned |

## Decisions

| Date | Decision |
|---|---|
| 2026-09-17 | Host at `hub.uor.foundation` on the existing VPS; registry reads public, writes token-gated |
| 2026-09-17 | Pinning provider: Filebase |
| 2026-09-17 | Weights on the hub: up to 15 GB, small permissive-licence models |
| 2026-09-18 | Reversed: no weights on the VPS. IPFS through Filebase is the only copy of weights the hub offers; the registry holds the daily index only |
| 2026-09-18 | The index is backed up on IPFS only (no S3 copy); the hub's mirror serves the current index only, every past day is read from IPFS |
| 2026-09-18 | Reconciled with the hub server work: the registry keeps every day's index (not current-day-only as first built). Its store also holds the objects `publish.sh` writes through the Hologram Server, so it must never be rebuilt. Growth to be measured over a week (one run re-uploaded 1,297 of 1,300 layers; the server work measured 96.7 % reuse) |
| 2026-09-17 | Upstream the local hub shim as small reviewed PRs |
| 2026-09-17 | External uptime monitoring: GitHub Actions probe |
