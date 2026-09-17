# Model Hub

The catalog of trending open models with the content address of every file, published at
[hologram-technologies.github.io/hologram-live/model-hub/](https://hologram-technologies.github.io/hologram-live/model-hub/).

Every model page lists its files by address and offers **Verify**: the browser fetches small files from Hugging Face
and ModelScope and hashes them against the address index. No server of ours is involved. Pulling models onto a
hologram-live node comes with the Model Hub `.holo` application (issue
[#76](https://github.com/Hologram-Technologies/hologram-live/issues/76), ADR 023 in
[#75](https://github.com/Hologram-Technologies/hologram-live/pull/75)).

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
