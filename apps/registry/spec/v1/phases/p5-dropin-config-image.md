# P5. Drop-in configuration, registry mode, image

> Task level. **Steps to be written before the phase starts** (day 7).
> Entry: P0 done. Task 3 needs P3 T1 (lands day 9). Engineer B. 4.5 days (days 8 to 12).

**Goal:** the compose file from the registry's deployment guide runs with only the `image:` line changed. Exit: `apps/registry/gates/clients/compose-swap.sh` green; the key walk test green.

**Files:**

| File | Holds | Lines |
|---|---|---|
| `src/registry_compat/mod.rs` | `load(path, env) -> Result<RegistrySettings, LiveError>`; `apply(settings, &mut AppConfig)` | 250 |
| `src/registry_compat/table.rs` | the key table: one `const` slice of `(key, Class, reason)` | 300 |
| `src/registry_compat/env.rs` | `REGISTRY_*` → key path | 150 |
| `src/registry_compat/yaml.rs` | YAML → flat `BTreeMap<String, Value>` | 150 |
| `src/cli/oci.rs` | `hologram oci …` command group | 200 |
| `src/main.rs` | `argv[0]` rewrite | +15 |
| `apps/registry/Dockerfile`, `apps/registry/Dockerfile.dockerignore`, `apps/registry/config.yml` | the image | |

---

## Task 1: The key table, `REGISTRY_*`, `config.yml`

**Requirements:** FR-011, FR-008 (names). **Files:** create `src/registry_compat/*`, `tests/fixtures/registry-config-keys.txt`; modify `src/lib.rs`, `Cargo.toml` (one YAML parser), `DEPENDENCIES.md`.

**Interfaces:**
- `pub enum Class { Supported, Ignored, Refused(&'static str) }`
- `pub fn classify(key: &str) -> Option<&'static Entry>`: longest-prefix match, so `storage.s3` covers `storage.s3.bucket`
- `pub fn load(config: Option<&Path>, env: impl Iterator<Item = (String, String)>) -> Result<RegistrySettings, LiveError>`: file first, environment over it (D3); any key classed `Refused`, or not in the table at all, is `LiveError::Config` with the fixed text in `contracts/config-table.md` (exit code 2, `src/main.rs:39`)
- `pub fn apply(settings: &RegistrySettings, config: &mut AppConfig)`: `http.addr` → `server.listen` (`:5000` becomes `0.0.0.0:5000`), `storage.filesystem.rootdirectory` → registry root, and so on per the table

Step 1 of the step-level plan: extract every key from the pinned reference's `configuration.md` into `tests/fixtures/registry-config-keys.txt` (a script, `apps/registry/gates/compose/extract-config-keys.sh`, kept so the fixture can be regenerated when the pin moves). Step 2: choose the YAML parser by three checks: a release in the last 6 months; `cargo tree` shows no `-sys` crate; it parses the reference's default file (D2) and a file with anchors. Record the choice and the two rejected candidates in `DEPENDENCIES.md`.

**Done when:** `registry_compat::tests::walks_every_documented_key` passes: for every line of the fixture, `classify` returns an entry; for every `Refused` entry, `load` of a file that sets only that key fails and the message contains the key and the reason; for every `Ignored` entry, `load` succeeds. Plus: the reference's default `config.yml` loads; `REGISTRY_STORAGE_DELETE_ENABLED=true` overrides a file that says `false`; `REGISTRY_STORAGE_S3_BUCKET=x` stops the start naming `storage.s3.bucket`; gate B scenario `env-unknown-key` is recorded and the difference (we refuse, the reference ignores) is row D-002 in `apps/registry/DIFFERENCES.md`.

**Traps:**
- The environment form is ambiguous: `REGISTRY_HTTP_TLS_KEY` is `http.tls.key`, but a key with an underscore in a name cannot be told from a level break. Resolve by matching against the table's known keys, longest first; an unmatched variable is refused.
- `REGISTRY_HTTP_SECRET` is set in many real compose files. It is `Ignored`, not refused.
- Do not let serde's `deny_unknown_fields` do the refusing: its message names a field, not a full key path, and gives no reason.

## Task 2: The `registry` command name and `hologram oci`

**Requirements:** FR-001, FR-010 (command shape). **Files:** modify `src/main.rs`, `src/cli/mod.rs`; create `src/cli/oci.rs`.

**Interfaces:** before `Cli::parse()` (R20):

```rust
fn rewrite_registry_argv(args: Vec<OsString>) -> Result<Vec<OsString>, String>;
// ["registry","serve","/etc/distribution/config.yml"] → ["hologram","serve","--registry-config","/etc/distribution/config.yml"]
// ["registry","garbage-collect","--dry-run","cfg.yml"] → ["hologram","oci","garbage-collect","--dry-run","--registry-config","cfg.yml"]
// ["registry","--version"] → prints and exits 0;   anything else → Err naming the command, exit 2
```

Applied only when the file name of `argv[0]` is `registry` or `registry.exe`. `hologram serve` gains `--registry-config <path>` (clap, `env = "HOLOGRAM_REGISTRY_CONFIG"`), which implies registry mode even when no file is given (`--registry-config ""` is not allowed; use `--registry-mode` for mode without a file). New `Command::Oci(oci::OciArgs)` with subcommands `garbage-collect`, `verify`, `import`, `adopt`: declared here, implemented in P8 (until then each returns `LiveError::Capability("not built yet")`).

**Done when:** a table test over `rewrite_registry_argv` with the rows in `contracts/config-table.md`; `hologram registry …` (the existing provider subcommand, R16) behaves exactly as before (its existing tests pass untouched).

**Trap:** clap sees `registry` as a subcommand of `hologram` today. The rewrite happens on `argv[0]`, never on `argv[1]`. `hologram registry serve` must keep meaning what it means now.

## Task 3: Registry mode and its network surface (ADR-028)

**Requirements:** FR-015, FR-021. **Files:** modify `src/config.rs`, `src/cli/serve.rs`, `src/server.rs`; create `specs/adrs/028-registry-mode-network-surface.md`, `features/suites/s5_registry/registry_mode.feature`, `apps/registry/gates/clients/surface.sh`.

In registry mode, `serve` builds its `AppConfig` like this, before `validate()`:

| Setting | Value | Why |
|---|---|---|
| `modules.enabled` | `dev.hologram.live.system`, `dev.hologram.live.oci` | FR-015 |
| `server.listen` | from `http.addr`, default `0.0.0.0:5000` | D1 |
| registry root | `storage.filesystem.rootdirectory`, default `/var/lib/registry` | D1 |
| `paths.data_dir`, `state_dir`, `cache_dir`, `config_dir` | `<root>/live/…` | every path defaults under `$HOME` (R18); a container may have none |
| `auth.required` | `true`, with a token generated at start (32 random bytes, hex), written to `<root>/live/state/admin-token`, mode 0600 | `validate()` refuses a public listen without it (R9); the system API must not be open on port 5000 |
| gRPC | **not** merged into the public router; served on `127.0.0.1:5001`, plain, always | gRPC shares the port today (R12); `shutdown` on a public anonymous port is not acceptable |
| `plugins.enabled`, inference engine | off; engine `echo` | `AppState::build` opens them in every mode (R17); keep them inert |

`/`, `/healthz`, `/docs`, `/openapi.json` stay public (the reference answers 200 on `/`). The in-container CLI (`hologram oci verify`) reads the token file and dials `127.0.0.1:5001`.

**Done when:** the BDD feature passes: every route group marked Off in the brief's endpoint map answers 404; `GET /api/v1/modules` without the token answers 401; a gRPC call to the public port is refused; a gRPC call to `127.0.0.1:5001` with the token file's content succeeds. `apps/registry/gates/clients/surface.sh` does the same from outside a running container and also asserts port 5001 is not published.

**Traps:**
- `validate()`'s loopback rule (R9) stays as it is for every other mode. Registry mode satisfies it; it does not weaken it.
- Two listeners, one shutdown signal: both must drain on `wait_shutdown` (R13's graceful path), and the process must exit non-zero if either fails to bind.
- The token file lives on the data volume. It is regenerated at each start; nothing should cache it.

## Task 4: The image

**Requirements:** FR-016, FR-001. **Files:** create `apps/registry/Dockerfile`, `apps/registry/Dockerfile.dockerignore`, `apps/registry/config.yml`; modify `.github/workflows/gates.yml` (job `image`).

```dockerfile
# syntax=docker/dockerfile:1
FROM rust:1.97.1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked --package hologram-live --bin hologram \
 && install -D target/release/hologram /out/usr/local/bin/hologram \
 && install -d /out/bin /out/var/lib/registry /out/etc/distribution \
 && ln /out/usr/local/bin/hologram /out/bin/registry \
 && cp apps/registry/config.yml /out/etc/distribution/config.yml

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build --chown=nonroot:nonroot /out/ /
EXPOSE 5000
VOLUME ["/var/lib/registry"]
ENTRYPOINT ["registry"]
CMD ["serve", "/etc/distribution/config.yml"]
```

`apps/registry/config.yml` equals the reference's default file (recorded in `apps/registry/gates/compose/REFERENCE-IMAGE.md` by P2 T1).

**Done when:** `docker run --rm -p 5000:5000 <image>` answers `GET /v2/` with 200; `docker inspect` shows the same entrypoint, command, port and volume as the reference (a test compares the two JSON outputs); `docker exec <c> hologram oci verify` is found on the path.

**Traps:**
- The reference runs as root; `nonroot` cannot write a volume that an old deployment created as root. Decide from `compose-swap.sh`: if the documented compose file with a named volume fails, run as root like the reference and list nothing. Drop-in beats hardening here.
- `ln` (hard link) inside one layer keeps one copy of the binary. A symlink would make `argv[0]` resolve differently under some runtimes.
- distroless has no shell: `docker exec registry sh` does not work. The reference image has one. Note it in the docs, not in `apps/registry/DIFFERENCES.md` (it is not registry behaviour).

## Task 5: The compose swap test

**Requirements:** FR-001, SC-001. **Files:** `apps/registry/gates/clients/compose-swap.sh` (written in P2 T4) now runs against the image; `apps/registry/gates/compose/*.yml`.

Three files, verbatim from the deployment guide, each with its source URL in a comment: basic; with `REGISTRY_HTTP_TLS_*` and a mounted certificate (runs from P6); with `REGISTRY_AUTH_HTPASSWD_*` (runs from P6).

**Done when:** the basic file passes on day 12; the script measures the time from `sed` to a successful push and fails over 300 s with the image already pulled (closes CHK014: pull time is excluded).

---

## Phase exit

`compose-swap.sh` (basic) green and in `gates.yml`. Key walk test green. `image` job blocking from day 12. Closes FR-011, FR-015, FR-021 (extended in P6 T3), and FR-001 and SC-001 once P6 turns on the other two compose files.
