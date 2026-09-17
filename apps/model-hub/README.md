# Model Hub

Hologram Models Hub as a `.holo` application: the catalog of trending open models with the content address of every
file, plus self-verifying storage on this node (pull, verify, remove).

**Status: blocked on ADR 023** ([#75](https://github.com/Hologram-Technologies/hologram-live/pull/75),
issue [#76](https://github.com/Hologram-Technologies/hologram-live/issues/76)). `hologram compile` rejects the archive
today with `unsupported Wasm guest contract "hologram:guest/component-artifacts@1"`, because the contract, the
`artifact_scopes` capability and the host interface do not exist yet. Everything else is built and tested.

## One generator, two targets

`web/` is the single source of the interface. The same generator also builds a static site, published with the
documentation at [hologram-technologies.github.io/hologram-live/model-hub/](https://hologram-technologies.github.io/hologram-live/model-hub/)
by `.github/workflows/release-docs.yml` on pushes to `apps/model-hub` and daily, with a fresh catalog snapshot. The
site verifies bytes in the browser against Hugging Face and ModelScope; pulling onto a node stays in the `.holo`.

| Target | Command | Output |
|---|---|---|
| View for the `.holo` | `just model-hub` | `ui/` (packed by `hologram.json`) |
| Static site | `just model-hub-site` | `web/dist/` |

The targets differ only where the View sandbox requires it (ADR 018, `view_surface.rs`):

| Site | View |
|---|---|
| Clean URLs, state in `?query` | Exact `…/index.html` paths, state in `#hash` (queries are rejected) |
| External links open | Links cannot leave the View: clicking copies the address |
| Remote images in model cards | Alt text |
| Verify in the browser against Hugging Face and ModelScope; per-source downloads | **Pull to this node**, **Verify on this node**, **Remove** through the host (ADR 023) |
| Catalog fetched from the site | Catalog snapshot packed into the archive; the header shows its date |

A catalog model maps to the registry reference `huggingface/<owner>/<repo>:<revision>` (lowercase, ADR 022 grammar).
Publishers push under that name; until they do, Pull reports that the model is not in the registry.

## Brand kit

Every color, size, space, radius and font comes from
[Hologram-Technologies/hologram-brand-kit](https://github.com/Hologram-Technologies/hologram-brand-kit), vendored by
`web/scripts/vendor-kit.mjs` at `develop` `bce3bdf59` (the same revision Desktop uses). Three gap tokens
(`--success`, `--success-foreground`, `--muted-foreground-subtle`) come from the kit's open PR #1.
`web/scripts/lint-tokens.mjs` fails on any literal color, size, radius, font or inline style in the product code.

## Layout

| Path | What |
|---|---|
| `src/hub.rs`, `src/lib.rs` | The primary: validates View requests, forwards them to `hologram:host/artifacts@1.0.0`, reports host answers unchanged, reduces host failures to closed codes |
| `tests/hub.rs` | Primary behaviour against a fake host |
| `wit/` | **Proposed** interface from ADR 023; moves to `specs/wit/artifacts` once accepted |
| `capabilities.json` | **Proposed** schema 3 request; `@default` is the host's configured registry |
| `web/` | Generator, styles, client code, vendored brand kit, QA scripts (`web/qa`) |
| `preview/` | View preview against a simulated host that enforces the View's path rules; not packed |

The primary decides nothing about trust. Pulls re-hash every layer on write, verification re-hashes every layer, and
admission, consent and audit happen in the host. The View shows only what the host reported.

## Findings for ADR 023 (posted on #75)

1. `list` is a WIT keyword; `wit/` uses `%list`.
2. Host failures should be stable codes (`denied`, `not_found`, `invalid_reference`, `busy`, `conflict`, `failed`).
3. An application cannot name the user's registry; `@default` resolves at consent time.
