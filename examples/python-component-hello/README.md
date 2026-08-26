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

The first compile lets `uvx` download the pinned `componentize-py 0.25.0`
tool. Later compiles reuse uv's tool cache. The current profile intentionally
accepts dependency-free locks only. It stubs CPython's WASI imports, so Python
randomness is deterministic and must not be used for security-sensitive work.
