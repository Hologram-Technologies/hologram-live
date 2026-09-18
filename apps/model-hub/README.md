# Model Hub

The catalog of trending open models with the content address of every file, published at
[hologram-technologies.github.io/hologram-live/model-hub/](https://hologram-technologies.github.io/hologram-live/model-hub/).

Every model page lists its files by address and offers **Verify**: the browser fetches small files from Hugging Face
and ModelScope and hashes them against the address index. No server of ours is involved. Pulling models onto a
hologram-live node comes with the Model Hub `.holo` application (issue
[#76](https://github.com/Hologram-Technologies/hologram-live/issues/76), ADR 023 in
[#75](https://github.com/Hologram-Technologies/hologram-live/pull/75)).

## Use it with your tools: `HF_ENDPOINT`

```bash
export HF_ENDPOINT=https://hub.uor.foundation
hf download sentence-transformers/all-MiniLM-L6-v2
```

Everything that downloads from Hugging Face honours that variable (`hf`, `huggingface_hub`, transformers, diffusers,
sentence-transformers, llama.cpp `-hf`), so the hub needs no client of its own. `deploy/hub-resolve.mjs` answers in
Hugging Face's dialect from the address index and sends every file request, as a redirect, to a source that passed the
last health probe: Hugging Face first (its CDN is the fastest), then ModelScope, then IPFS. No weight byte passes
through the hub. `main` is the indexed revision, so a build gets the same bytes tomorrow. `/via/ipfs`,
`/via/modelscope` or `/via/huggingface` in front of the path pins the first choice of source
(`HF_ENDPOINT=https://hub.uor.foundation/via/ipfs`). `…/resolve/main/SHA256SUMS` is generated for every model, so a
download is checked with plain `sha256sum -c`; the clients themselves verify nothing.

The dialect was recorded from the clients, not guessed (`web/qa/hf-dialect/recorder.mjs`): model info, the tree listing,
`HEAD` and `GET` on `resolve`, and the Xet read-token route, which is a redirect to Hugging Face because
`huggingface_hub` 1.32 follows our redirect on `HEAD`, meets Hugging Face's Xet headers there and asks this endpoint
for the token. A user's Hugging Face token reaches whatever `HF_ENDPOINT` names: the front Caddy removes
`Authorization` and `Cookie` before the service sees a request, and gated models are refused, never proxied. A model
outside the index answers 404 with a sentence and is recorded in `state/resolve/requested.txt`.

Measured 2026-09-18 with `web/qa/hf-dialect/matrix.sh` in `python:3.12-slim` against the public URL, on
`huggingface_hub` 1.32.0 and 0.36.2: `hf download` and `snapshot_download` of a whole model (30 of 30 files matching
`SHA256SUMS`), cache reuse, transformers `AutoConfig` and `AutoTokenizer`, a plain `GET` and a `Range` request,
and the clients' own typed errors for an unknown model and revision. **Failover, measured the same day** with huggingface.co, hf.co and
Hugging Face's CDN hosts blackholed in the client container and Hugging Face marked down on the hub: `hf download` and
`snapshot_download` of the whole model, 30 of 30 files matching, transformers loading, in 110 s from ModelScope and
IPFS; and with ModelScope down as well, the same from IPFS alone in 126 s (135 reroutes logged). The first attempt
had failed at `.gitattributes`: the IPFS archives had been packed without dotfiles (`ipfs-car` skips hidden paths).
`pin-model.sh` now packs them and all eleven models were pinned again. Test whole repositories, not single files.

### Ollama, and the design behind both

```bash
ollama pull hub.uor.foundation/bartowski/MiniCPM5-2B-GGUF:Q4_K_M
```

The same service speaks Ollama's registry dialect for every GGUF repository in the index: the manifest's model layer
is the GGUF file's SHA-256 from the index, the blob request is a `307` to a live source, and Ollama verifies the
digest itself. The chat template and parameters come from Hugging Face's own manifest, accepted only when it names
the very bytes our index names, and kept so the pull still works when Hugging Face is away (measured: Ollama 0.34.2
pulled, verified and ran a model; with Hugging Face blackholed the same pull completed from ModelScope).
`GET /api/models?search=…` lists and searches in Hugging Face's shape (`HfApi.list_models` works), which is also the
cheap path for agents. Why these dialects, what the research found, the path map and what comes next:
[docs/one-endpoint.md](docs/one-endpoint.md).

## Download

**Download** saves a whole model as one `.zip`, any size, in any current browser. A small service worker
(`web/src/zip-sw.js`, which answers only `zip/…` and never a page) assembles the archive while it streams, so the
browser's own download manager saves it with a real progress bar and nothing is held in memory: the zip is
uncompressed, so its exact size is declared before the first byte. Every file is hashed as it passes and compared with
its address from the index; a wrong byte fails the download and nothing is kept. Each file comes from the first source
that answers (Hugging Face, ModelScope, IPFS), a dropped connection resumes from the byte it reached, from the same
source or the next, and the menu beside the button restricts the zip to one source. Inside every zip: `SHA256SUMS`
(`sha256sum -c SHA256SUMS` checks it offline, forever) and `HOLOGRAM.json` (model, revision, index manifest, which
source delivered each file). `zip/<org>/<name>/auto.zip` is the link; `data/files/<org>/<name>.json` is the file list
it reads. Without a service worker the page falls back to the file picker stream (Chrome, Edge) or an in-memory zip
up to 1.5 GB.

Measured 2026-09-18 (`web/qa/zip-proof`, Docker on the hub host): a generated 5.9 GB zip saved with the declared size
exactly, valid, in Chromium 131 (140 s, browser memory 377 to 471 MB), WebKit 18.2 (38 s, 383 to 419 MB) and stock
Firefox 156 (169 s at 60 MB/s, container under 1 GB). A connection cut mid-file resumed and verified; a planted wrong
address failed the download in all three with nothing left on disk. Playwright's own Firefox build cannot judge
downloads: an ordinary 6 GB server download exhausts its memory too. `node qa/zip-size.test.mjs` holds the declared
size and the written bytes together (empty file, UTF-8 names, 65,536 entries, a file above 4 GiB).

## The Archive

The header pill `Index <day>` opens every day the catalog was captured, the way the Wayback Machine opens a page's
past. Choosing a day switches browse, filters, search and every model page to that day's index; the pill turns
amber, a banner names the day, and **Back to latest** returns. `?at=YYYY-MM-DD` on any page opens the nearest
captured day on or before that date, so a link to a day is a link to exactly what it showed.

Each day is captured once by `deploy/archive.sh` after the daily registry push: the day's files are packed into a
CAR (IPFS archive) whose root CID is computed locally, pinned through Filebase, and accepted only if the CID Filebase
reports is the same. The day is appended to the ledger `archive.json` (`hologram.model-hub.archive/v1`), served at
[hub.uor.foundation/archive.json](https://hub.uor.foundation/archive.json) and pinned itself:

| Field | Meaning |
|---|---|
| `gateway`, `mirror`, `registry` | Where a day is read: `<gateway><cid>/<path>` on IPFS and `hologram pull <registry>:<date>` on the hub registry for every day; `<mirror><date>/<path>` on the hub for the current day only. The mirror only makes reads fast; it is never trusted |
| `days[].date`, `cid` | The day and the root CID of its directory (`index.json`, `models.json`, one JSON per model) |
| `days[].index` | BLAKE3 of that day's `index.json`, which names every other file by address |
| `days[].prev`, `prev_ledger` | The previous day's CID and the previous ledger's CID: a hash chain, so history cannot be rewritten silently |
| `days[].models`, `addressed`, `files`, `bytes`, `source`, `archived` | What the day held and when it was captured |

The browser trusts none of the sources: it reads `index.json`, hashes it against the ledger's `index`, then hashes
every file against the address the index records, before anything is shown. Bytes that fail are refused and the
next source is tried (measured: a corrupted mirror file was rejected and the gateway's copy used). Verified bytes are
kept in the Cache API under their content address, so a revisited day is instant and works offline. **Verify** and
downloads stay with the latest index because they check live mirrors. `at/<date>.json` stubs give agents the CID,
index address and read locations for a day. The hub's mirror holds today only; the registry keeps every day's tag (its store also
holds the hub's published objects, so it is never rebuilt); a past day is read in the browser from IPFS, and its first visit can take a minute while the gateway fetches
it (measured 25 to 55 s per file cold). What is immutable: the captures and the chain. What is one operator: the
daily writer (a VPS cron) and the single pinning provider.

## Build

```bash
just model-hub-site   # catalog snapshot (if missing), token lint, static build → apps/model-hub/web/dist
just model-hub-data   # refresh the catalog snapshot
```

`BASE=/path/` builds for a site served under a path. GitHub Pages publishes it beside the documentation from
`.github/workflows/release-docs.yml`: on pushes to `apps/model-hub`, daily with a fresh snapshot, and on manual runs.
The documentation itself is always built from its latest `docs-v*` tag.

The catalog comes from Hugging Face's trending list joined with the model address index at
`humuhumu33.github.io/hologram-api` (`HOLOGRAM_API` overrides it; `HF_TOKEN` avoids rate limits).

## Brand kit

Every color, size, space, radius and font comes from
[Hologram-Technologies/hologram-brand-kit](https://github.com/Hologram-Technologies/hologram-brand-kit), vendored by
`web/scripts/vendor-kit.mjs` at `develop` `bce3bdf59` (the revision Desktop uses). Three gap tokens (`--success`,
`--success-foreground`, `--muted-foreground-subtle`) come from the kit's open PR #1. `web/scripts/lint-tokens.mjs`
fails on any literal color, size, radius, font or inline style in the product code.

## Layout

| Path | What |
|---|---|
| `web/build.mjs` | Static generator: browse page and one page per model |
| `web/src/` | Rendering, client code, styles, generated brand tokens |
| `web/scripts/` | `data.mjs` (catalog snapshot), `vendor-kit.mjs`, `lint-tokens.mjs` |
| `web/vendor/hologram-brand-kit/` | Vendored kit CSS, tokens, fonts and logos |
| `web/public/wallpapers/` | Immersive theme wallpapers |
| `web/qa/` | Layout, contrast and animation audits used during design review |
