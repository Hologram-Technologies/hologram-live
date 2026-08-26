# Component v1 echo fixture

`echo.wat` is the checked-in Component Model artifact used by unit and BDD
tests. Its Rust source binds directly to the repository's canonical
`specs/wit/hologram-application-v1.wit` world. Normal input is echoed; the
literal input `guest-error` returns the WIT `failed` error for negative tests.

The source crate is intentionally separate from the product workspace so
ordinary builds do not require a Wasm target or regenerate the golden artifact.
Regeneration requires `wasm32-unknown-unknown`, `wit-component`, and a
WebAssembly printer; the committed WAT keeps normal tests toolchain-independent.
