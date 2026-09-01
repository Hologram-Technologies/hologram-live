# Component network-fetch fixture

The checked-in `network-fetch.wasm` is generated from this crate for runtime
contract tests. Input is one canonical HTTPS endpoint. The guest performs one
mediated GET and returns the big-endian status code followed by the response
body.

The fixture is compiled for `wasm32-unknown-unknown` and encoded with
`wit-component` 0.247.0. Building it as `wasm32-wasip2` would add WASI imports
that this fixed-import profile intentionally refuses to link.
