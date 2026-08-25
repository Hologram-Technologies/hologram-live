set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Format the rust code
fmt:
    cargo fmt --all --check

# Check code
check:
    cargo check --workspace --all-targets --locked

# Run workspace tests
test:
    cargo test --workspace --all-targets --locked

# Run public-boundary Gherkin scenarios.
bdd:
    cargo test --package hologram-live --features bdd --test bdd --locked

# Compile and execute the locked NumPy + pandas example in a .holo archive.
python-holo-demo:
    ./scripts/check-python-holo-demo.sh

# Compile and execute the small standard-library Python example.
python-hello-demo:
    cargo build --release --locked --package hologram-live --bin hologram
    ./target/release/hologram --json compile examples/python-hello/hologram.json --check >/dev/null
    ./target/release/hologram --json run examples/python-hello --input-text Ada --output-format json

# Compile, verify, and retain the NumPy + pandas .holo artifact.
python-holo-package output="target/numpy-pandas.holo":
    ./scripts/check-python-holo-demo.sh --output "{{output}}"

# Keep production source files small enough to review and refactor.
file-size:
    ./scripts/check-file-size.sh

# Keep the standalone server dependency graph free of desktop code.
product-boundary:
    ./scripts/check-product-boundaries.sh

# Run clippy
clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Build the standalone server binary.
server-build:
    cargo build --release --locked --package hologram-live --bin hologram

# Build the Tauri desktop application and its bundled server sidecar.
desktop-build:
    cd apps/desktop && npm ci && npm run build

# Backwards-compatible default release build for the server.
build: server-build

# Verify code
verify: fmt file-size product-boundary check test clippy bdd build
    ./scripts/smoke.sh ./target/release/hologram

# Run project
run *args:
    cargo run --locked --package hologram-live --bin hologram -- {{args}}

# Recompile and restart the foreground daemon when Rust/server inputs change.
dev:
    @command -v cargo-watch >/dev/null || { echo "error: just dev requires cargo-watch (cargo install cargo-watch --locked)" >&2; exit 1; }
    cargo watch --clear --watch src --watch proto --watch build.rs --watch Cargo.toml --watch Cargo.lock --exec 'run --locked --package hologram-live --bin hologram -- serve'

# Build the docs
docs:
    cd apps/docs && npm ci && npm run build

# Validate, tag, and push the current documentation version to GitHub Pages.
docs-release version="":
    ./scripts/release-docs.sh "{{version}}"

# Serve the Astro docs with hot reload.
docs-dev:
    cd apps/docs && npm install && npm run dev -- --host 127.0.0.1 --port 54321

# Work with the Tauri desktop app. Defaults to `dev`; `just tauri build` creates a bundle.
tauri action="dev":
    cd apps/desktop && npm install && npm run "{{action}}"
