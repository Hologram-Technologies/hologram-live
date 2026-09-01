# Component store-read fixture

The checked-in `store-read.wasm` component imports only
`hologram:host/store@1.0.0`, interprets its invocation input as a UTF-8 object
κ, and returns the bytes read through that interface. Its Rust source binds to
the canonical WIT profile under `specs/wit/store-read`.
