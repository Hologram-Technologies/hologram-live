set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Format the rust code
fmt:
    cargo fmt --all --check

# Check code
check:
    cargo check --all-targets --locked

# Run workspace tests
test:
    cargo test --all-targets --locked

# Run clippy
clippy:
    cargo clippy --all-targets --locked -- -D warnings

# Build project
build:
    cargo build --release --locked

# Verify code
verify: fmt check test clippy build
    ./scripts/smoke.sh ./target/release/hologram

# Run project
run *args:
    cargo run --locked --bin hologram -- {{args}}

# Recompile and restart the foreground daemon when Rust/server inputs change.
dev:
    @command -v cargo-watch >/dev/null || { echo "error: just dev requires cargo-watch (cargo install cargo-watch --locked)" >&2; exit 1; }
    cargo watch --clear --watch src --watch proto --watch build.rs --watch Cargo.toml --watch Cargo.lock --exec 'run --locked --bin hologram -- serve'

# Build the docs
docs:
    cd apps/docs && npm install && npm run build

# Serve the Astro docs with hot reload.
docs-dev:
    cd apps/docs && npm install && npm run dev -- --host 127.0.0.1 --port 5432
