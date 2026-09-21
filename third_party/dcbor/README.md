# dcbor, vendored

Deterministic CBOR, needed by `kappa-core` (see `../kappa/README.md`). Copied from
https://github.com/usrbinkat/bc-dcbor-rust at commit `2e5b901e8c9946794c5491cf92eeec2540b84261`, branch
`feat/dcbor-derive`, a fork of https://github.com/BlockchainCommons/bc-dcbor-rust that adds `dcbor-derive`.

Licence: BSD-2-Clause-Patent, Copyright 2023 Blockchain Commons, LLC. The full text is in `dcbor/LICENSE.md` and
`dcbor-derive/LICENSE.md`.

Copied: `src/`, `README.md` and `LICENSE.md` of each crate. The manifests were rewritten to stand alone (no
workspace, `dcbor-derive` by path, no dev-dependencies because the tests are not copied). The sources are byte for
byte those of the commit. `../kappa/VENDORED.sha256` records every file here, and `scripts/check-kappa-pin.sh`
fails if one changes.
