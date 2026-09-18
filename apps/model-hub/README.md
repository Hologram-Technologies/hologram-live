# Model Hub

The catalog of trending open models with the content address of every file, published at
[hologram-technologies.github.io/hologram-live/model-hub/](https://hologram-technologies.github.io/hologram-live/model-hub/).

Every model page lists its files by address and offers **Verify**: the browser fetches small files from Hugging Face
and ModelScope and hashes them against the address index. No server of ours is involved. Pulling models onto a
hologram-live node comes with the Model Hub `.holo` application (issue
[#76](https://github.com/Hologram-Technologies/hologram-live/issues/76), ADR 023 in
[#75](https://github.com/Hologram-Technologies/hologram-live/pull/75)).

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
| `gateway`, `mirror` | Where the browser reads a day: `<gateway><cid>/<path>` on IPFS, `<mirror><date>/<path>` on the hub. The mirror only makes reads fast; it is never trusted |
| `days[].date`, `cid` | The day and the root CID of its directory (`index.json`, `models.json`, one JSON per model) |
| `days[].index` | BLAKE3 of that day's `index.json`, which names every other file by address |
| `days[].reference` | The same day on the registry: `hologram pull hub.uor.foundation/model-hub/index:<date>` |
| `days[].prev`, `prev_ledger` | The previous day's CID and the previous ledger's CID: a hash chain, so history cannot be rewritten silently |
| `days[].models`, `addressed`, `files`, `bytes`, `source`, `archived` | What the day held and when it was captured |

The browser trusts none of the sources: it reads `index.json`, hashes it against the ledger's `index`, then hashes
every file against the address the index records, before anything is shown. Bytes that fail are refused and the
next source is tried (measured: a corrupted mirror file was rejected and the gateway's copy used). Verified bytes are
kept in the Cache API under their content address, so a revisited day is instant and works offline. **Verify** and
downloads stay with the latest index because they check live mirrors. `at/<date>.json` stubs give agents the CID,
index address, registry reference and both read locations for a day. What is immutable: the captures and the chain.
What is one operator: the daily writer (a VPS cron) and the single pinning provider.

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
