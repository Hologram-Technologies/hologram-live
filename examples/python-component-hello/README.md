# Dependency-free Python Component `.holo`

This project uses `profile: "wasi-component"` to bundle CPython and the app
into an import-free `hologram:guest/component@1` Wasm layer. It does not use
Docker and the Hologram runtime does not link WASI.

Install `uv`, then compile and run with an optimized Hologram CLI:

```console
cargo build --release
./target/release/hologram compile hologram.json --output python-component-hello.holo
HOLOGRAM_STATE_DIR=.state ./target/release/hologram run \
  python-component-hello.holo --input-text Ada --output-format json
```

The first compile lets `uvx` download Hologram's pinned deterministic
`componentize-py 0.25.0` wheel from the immutable
`componentizer-v0.25.0-hologram.2` release. Later compiles reuse uv's tool
cache. This example is dependency-free; the companion
`../python-component-dependency/` shows the profile's locked pure-Python wheel
support. It stubs CPython's WASI imports, so Python randomness is deterministic
within one built component and must not protect secrets.
