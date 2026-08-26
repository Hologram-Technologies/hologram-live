# Locked dependency Python Component `.holo`

This project proves that `profile: "wasi-component"` can bundle a dependency
from `uv.lock` without reading the developer's virtual environment. The
compiler selects the lock's SHA-256-pinned `six` platform-independent wheel,
installs it into a private build path, and emits an import-free
`hologram:guest/component@1` Wasm layer.

Install `uv`, then compile and run with an optimized Hologram CLI:

```console
cargo build --release
./target/release/hologram compile hologram.json --output python-component-dependency.holo
HOLOGRAM_STATE_DIR=.state ./target/release/hologram run \
  python-component-dependency.holo --input-text Ada --output-format json
```

The portable profile accepts registry packages only when the lock contains an
HTTPS, SHA-256-pinned Python 3 `*-none-any.whl`. Native and source-only
dependencies fail before componentization with guidance to the explicit
`rootfs` profile.
