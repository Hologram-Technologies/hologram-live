# P0. Spike: the Kappa store inside this binary

> Step level. Executable as written by someone who has never opened the Kappa repository.

**Goal:** answer one question with evidence: can `kappa-core` and `kappa-store-redb` live inside the `hologram` binary on Linux, macOS and Windows? Go or no-go.

**Days:** 2 calendar days, both engineers (4 engineer days). **Branch:** `registry/p0-spike`. Nothing here ships; the branch is kept as the record.

**What Kappa is, in four lines.** `github.com/UOR-Foundation/kappa-registry` is a Rust workspace of 15 crates. We want two. `kappa-core` defines the `KappaStore` trait (`crates/kappa-core/src/store/mod.rs`). `kappa-store-redb` implements it as `PersistentStore`: blob files on disk, an index in one redb file. Pinned revision: `2af86560a177fc9651b6c0e92e7974140ed77dd5`.

**Known before you start** (`research.md`): the workspace root does not parse as published (K15); the two crates pull `dcbor` and `rekindle-aead` from branches of personal forks, and `zstd`, `xz2`, `bzip2` as bundled C (K14); sessions die on restart (K5); blob data is not synced before rename (K4); a sha256 upload gets no blake3 address (K6).

**Local builds:** never run a build against a real configuration (`~/.config/hologram/live.toml`); use an isolated `HOLOGRAM_CONFIG`.

---

## Task 1: Pin the crates and make them build (engineer A, day 1)

**Requirements:** the open decision; FR-019.

**Files:**
- Modify: `Cargo.toml`
- Create: `third_party/kappa/README.md`
- Create: `third_party/kappa/patches/0001-remove-test-section-from-virtual-manifest.patch`, `0002-pin-fork-branch-dependencies-by-rev.patch`
- Create: `scripts/check-kappa-pin.sh`

- [ ] **Step 1: Branch.**

```bash
cd hologram-registry
git switch main && git pull --ff-only origin main
git switch -c registry/p0-spike
```

- [ ] **Step 2: Prove the plain dependency fails, and keep the error (10 minutes).** This is the try that draft 1 put first. It is expected to fail; the error text goes in the verdict.

Add to `Cargo.toml`, under `[dependencies]`:

```toml
kappa-core = { git = "https://github.com/UOR-Foundation/kappa-registry", rev = "2af86560a177fc9651b6c0e92e7974140ed77dd5", optional = true }
kappa-store-redb = { git = "https://github.com/UOR-Foundation/kappa-registry", rev = "2af86560a177fc9651b6c0e92e7974140ed77dd5", optional = true }
```

and under `[features]`:

```toml
oci = ["dep:kappa-core", "dep:kappa-store-redb"]
```

Run: `cargo check --features oci 2>&1 | tee /tmp/p0-plain-dep.txt` (no `--locked`: the lock file must change).
Expected: FAIL while loading the git dependency, with a manifest parse error naming `[[test]]` in a virtual manifest. If it **passes**, upstream fixed it: skip steps 3 and 4, keep these lines, go to step 5.

- [ ] **Step 3: First real try: a fork that carries the patches as commits.**

```bash
gh repo fork UOR-Foundation/kappa-registry --clone=false        # spike: your own account
timeout 300 git -c http.version=HTTP/1.1 clone https://github.com/<you>/kappa-registry ../kappa-registry
cd ../kappa-registry
git switch -c hologram-registry-pin 2af86560a177fc9651b6c0e92e7974140ed77dd5
```

Commit 1. In the root `Cargo.toml`, delete these three lines (they sit just above `[profile.release]`):

```toml
[[test]]
name = "bdd"
harness = false
```

```bash
git commit -am "fix: remove [[test]] from the virtual workspace manifest"
```

Commit 2. Replace the two branch pins in `[workspace.dependencies]` with `rev` pins. Find the commits the branches point at today:

```bash
timeout 60 git ls-remote https://github.com/usrbinkat/bc-dcbor-rust refs/heads/feat/dcbor-derive
timeout 60 git ls-remote https://github.com/ScopeCreep-zip/rekindle refs/heads/feat/cli-tui-restructure-rewrite
```

Edit so the two lines read (keep every other key as it is):

```toml
dcbor = { git = "https://github.com/usrbinkat/bc-dcbor-rust", rev = "<40 hex from ls-remote>", features = ["derive", "multithreaded"] }
rekindle-aead = { git = "https://github.com/ScopeCreep-zip/rekindle", rev = "<40 hex from ls-remote>", default-features = false }
```

```bash
git commit -am "build: pin fork-branch dependencies by rev"
git format-patch -2 -o ../hologram-registry/third_party/kappa/patches/
timeout 120 git -c http.version=HTTP/1.1 push -u origin hologram-registry-pin
git rev-parse HEAD            # this is <pin rev>
```

Fork `bc-dcbor-rust` and `rekindle` too (`gh repo fork … --clone=false`). Nothing points at those forks yet; they exist so a deleted upstream branch cannot take the `rev` with it. Record their URLs in the README.

Back in `hologram-registry`, change the two lines from step 2:

```toml
kappa-core = { git = "https://github.com/<you>/kappa-registry", rev = "<pin rev>", optional = true }
kappa-store-redb = { git = "https://github.com/<you>/kappa-registry", rev = "<pin rev>", optional = true }
```

Run: `cargo check --features oci`
Expected: PASS. Note the wall time of this first build.

- [ ] **Step 4: Fallbacks, in order. Stop at the first that works.**

  - **(b) Path dependency on a checkout.** If cargo cannot fetch the git dependency (network, submodules): clone the fork at `<pin rev>` to `third_party/kappa/src/` (git-ignored), and use `kappa-core = { path = "third_party/kappa/src/crates/kappa-core", optional = true }`. Add `exclude = ["third_party/kappa/src"]` under `[workspace]` so the crates keep their own workspace root. Cost: CI needs a clone step. Not shippable; good enough to finish the spike.
  - **(c) Vendored copy of the two crates.** Copy `crates/kappa-core` and `crates/kappa-store-redb` into `third_party/kappa/`, replace every `workspace = true` with the concrete value from the fork's root manifest (about 40 keys), add both as path dependencies. Cost: every upstream change is a manual diff. Use only if (a) and (b) both fail for a reason that will not go away.
  - **(d) No-go.** If the crates cannot be compiled on Linux by any of these, stop. Write the verdict as no-go. The re-plan is `kappa-server` as a second process behind Hologram Registry; it is a new plan, not a patch to this one.

- [ ] **Step 5: What came in.**

```bash
cargo tree --features oci -e normal --prefix none | sort -u > /tmp/p0-tree-after.txt
git stash -q && cargo tree -e normal --prefix none | sort -u > /tmp/p0-tree-before.txt; git stash pop -q
comm -13 /tmp/p0-tree-before.txt /tmp/p0-tree-after.txt > /tmp/p0-new-crates.txt
wc -l /tmp/p0-new-crates.txt
grep -i -E "topcoat|veilid|openssl|aws-lc" /tmp/p0-tree-after.txt        # must print nothing
grep -E -- "-sys " /tmp/p0-new-crates.txt                                 # expect lzma-sys, bzip2-sys
```

Expected: the forbidden grep prints nothing. New `-sys` crates are `lzma-sys` and `bzip2-sys` only (`zstd-sys` is already locked, R24). Anything else is a finding for the verdict.

- [ ] **Step 6: Size and time.**

```bash
git stash -q
/usr/bin/time -v cargo build --release --locked 2> /tmp/p0-build-before.txt; ls -l target/release/hologram > /tmp/p0-size-before.txt
git stash pop -q
/usr/bin/time -v cargo build --release --features oci 2> /tmp/p0-build-after.txt; ls -l target/release/hologram > /tmp/p0-size-after.txt
```

On macOS use `/usr/bin/time -l`. On Windows use `Measure-Command { cargo build --release --features oci }`. Record clean-build wall time and binary size, before and after. Nothing references the crates yet, so the linker drops most of them: the size number that matters is taken again after Task 2.

- [ ] **Step 7: Write `third_party/kappa/README.md`.**

```markdown
# Kappa store pin

| | |
|---|---|
| Upstream | https://github.com/UOR-Foundation/kappa-registry |
| Upstream revision the pin is based on | 2af86560a177fc9651b6c0e92e7974140ed77dd5 |
| Fork | https://github.com/<you>/kappa-registry, branch `hologram-registry-pin` |
| Pin revision (must equal Cargo.lock) | <pin rev> |
| Crates used | kappa-core, kappa-store-redb. Nothing else from the workspace |

## Carried patches

| File | What | Upstream | State |
|---|---|---|---|
| patches/0001-remove-test-section-from-virtual-manifest.patch | cargo cannot parse the workspace root | <pull request URL, filled in P1 T8> | not opened |
| patches/0002-pin-fork-branch-dependencies-by-rev.patch | a deleted branch must not break our build | <URL> | not opened |

## Fork-branch dependencies

| Crate | Upstream | Pinned rev | Our mirror |
|---|---|---|---|
| dcbor | https://github.com/usrbinkat/bc-dcbor-rust (branch feat/dcbor-derive) | <rev> | https://github.com/<you>/bc-dcbor-rust |
| rekindle-aead | https://github.com/ScopeCreep-zip/rekindle (branch feat/cli-tui-restructure-rewrite) | <rev> | https://github.com/<you>/rekindle |

To rebuild the pin from upstream alone: check out the upstream revision, `git am patches/*.patch`.
```

- [ ] **Step 8: Write the pin gate, `scripts/check-kappa-pin.sh`.**

```bash
#!/usr/bin/env bash
set -euo pipefail
# The Kappa pin is auditable: the locked revision is the documented one, every
# carried patch names where it was offered upstream, and nothing forbidden or
# branch-floating is in the server's dependency graph.
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
readme="${root}/third_party/kappa/README.md"
fail=0

documented=$(grep -E '^\| Pin revision' "${readme}" | grep -oE '[0-9a-f]{40}' | head -1 || true)
locked=$(grep -A2 '^name = "kappa-core"$' "${root}/Cargo.lock" | grep -oE '#[0-9a-f]{40}' | tr -d '#' | head -1 || true)
if [[ -z "${documented}" || "${documented}" != "${locked}" ]]; then
  printf 'kappa pin: README says %s, Cargo.lock says %s\n' "${documented:-none}" "${locked:-none}" >&2
  fail=1
fi

for patch in "${root}"/third_party/kappa/patches/*.patch; do
  name=$(basename "${patch}")
  if ! grep -F "${name}" "${readme}" | grep -qE 'https://github\.com/[^ |]+/(pull|issues)/[0-9]+'; then
    if [[ "${KAPPA_PIN_ALLOW_UNOPENED:-0}" != "1" ]]; then
      printf 'kappa pin: %s has no upstream link in the README\n' "${name}" >&2
      fail=1
    fi
  fi
done

tree=$(RUSTC_WRAPPER= cargo tree --manifest-path "${root}/Cargo.toml" --package hologram-live --features oci --edges normal --prefix none)
if grep -qiE '^(topcoat|veilid|openssl-sys|aws-lc)' <<<"${tree}"; then
  printf 'kappa pin: a forbidden crate is in the graph\n' >&2
  fail=1
fi
if grep -E '^source = "git\+' "${root}/Cargo.lock" | grep -vqE '#[0-9a-f]{40}"$'; then
  printf 'kappa pin: a git dependency is not locked to a revision\n' >&2
  fail=1
fi

(( fail == 0 )) && printf 'kappa pin gate passed (%s)\n' "${locked}"
exit "${fail}"
```

Run: `KAPPA_PIN_ALLOW_UNOPENED=1 ./scripts/check-kappa-pin.sh`
Expected: PASS. The environment variable exists for the spike only; P1 T8 opens the pull requests and removes it from CI.

- [ ] **Step 9: The existing gates still pass.**

```bash
./scripts/check-product-boundaries.sh
./scripts/check-file-size.sh
cargo check --workspace --all-targets --locked          # default features: nothing changed
git add -A && git commit -m "spike(oci): pin kappa-core and kappa-store-redb behind feature oci"
```

**Done when:** `cargo check --features oci --locked` passes on Linux and `check-kappa-pin.sh` passes.

**Traps:**
- `gh` and `git` over HTTPS stall on the maintainer's machine. A push can land while the reply hangs: run `git ls-remote origin hologram-registry-pin` before retrying.
- `cargo` fetches git dependencies with its own git client. If it stalls, set `CARGO_NET_GIT_FETCH_WITH_CLI=true`.
- The fork has about 60 stale branches . Work only on `registry/*`.
- Do not add the crates without `optional = true`. Default builds must stay byte-for-byte what they were until P1.

---

## Task 2: Prove the store round trip in a test (engineer B, day 1 to day 2 morning)

**Requirements:** FR-006, FR-020 (feasibility); sizes P1 T5, T6.

**Files:**
- Create: `tests/oci_spike.rs`
- Modify: `Cargo.toml` (one `[[test]]` entry)

Add to `Cargo.toml`:

```toml
[[test]]
name = "oci_spike"
required-features = ["oci"]
```

- [ ] **Step 1: Write the failing test.** Create `tests/oci_spike.rs`:

```rust
//! P0 spike: the Kappa store, driven the way the registry adapter will drive it.
//! Not product code. Every test answers one question in the P0 verdict.

use kappa_core::clock::Clock;
use kappa_core::KappaStore;
use kappa_store_redb::{PersistentStore, PersistentStoreConfig};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MIB: usize = 1024 * 1024;
const FRAME: usize = 4 * MIB;

struct WallClock;

impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after 1970");
        u64::try_from(elapsed.as_millis()).expect("milliseconds fit in u64")
    }
}

fn open(root: &Path) -> PersistentStore {
    let config = PersistentStoreConfig::new(root.join("kappa/blobs"), root.join("kappa/kappa.redb"));
    PersistentStore::new(config, Arc::new(WallClock)).expect("open store")
}

/// Deterministic bytes that do not compress, produced without holding them all.
fn frame(index: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME);
    let mut block = blake3::Hasher::new();
    block.update(&index.to_le_bytes());
    let mut reader = block.finalize_xof();
    out.resize(FRAME, 0);
    reader.fill(&mut out);
    out
}

fn stream_in(store: &PersistentStore, repo: &str, frames: u64) -> (String, String) {
    let namespace = store
        .namespace_resolve_or_create(repo, "spike", Some("oci"))
        .expect("namespace");
    let id = store.upload_begin(&namespace, 0).expect("begin");
    let mut sha = Sha256::new();
    let mut offset = 0_u64;
    for index in 0..frames {
        let bytes = frame(index);
        sha.update(&bytes);
        offset = store.upload_put_part(&id, offset, &bytes).expect("part");
    }
    let digest = format!("sha256:{:x}", sha.finalize());
    (id, digest)
}

#[test]
fn q1_a_64_mib_stream_lands_under_its_sha256() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let (id, digest) = stream_in(&store, "library/alpine", 16);

    let result = store.upload_complete(&id, Some(&digest)).expect("complete");

    assert_eq!(result.kappa, digest, "the store's address is the client's sha256");
    assert!(store.blob_exists(&digest).expect("exists"));
    assert_eq!(store.blob_size(&digest).expect("size"), (16 * FRAME) as u64);
}

#[test]
fn q2_a_range_read_seeks_without_reading_the_whole_blob() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let (id, digest) = stream_in(&store, "library/alpine", 16);
    store.upload_complete(&id, Some(&digest)).expect("complete");

    let mut reader = store.blob_open(&digest).expect("open");
    reader.seek(SeekFrom::Start(10 * MIB as u64)).expect("seek");
    let mut got = vec![0_u8; MIB];
    reader.read_exact(&mut got).expect("read");

    // 10 MiB into the stream is 2 MiB into frame 2.
    let expected = &frame(2)[2 * MIB..3 * MIB];
    assert_eq!(got, expected);
}

#[test]
fn q3_a_wrong_digest_is_refused_and_leaves_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let (id, real) = stream_in(&store, "library/alpine", 1);
    let wrong = format!("sha256:{}", "0".repeat(64));

    let outcome = store.upload_complete(&id, Some(&wrong));

    assert!(outcome.is_err(), "hash on write: a lie is refused");
    assert!(!store.blob_exists(&wrong).expect("exists"));
    assert!(!store.blob_exists(&real).expect("exists"), "nothing reachable is left behind");
}

#[test]
fn q4_tags_are_scoped_to_a_repository() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let (id, digest) = stream_in(&store, "team/tags/app", 1);
    store.upload_complete(&id, Some(&digest)).expect("complete");
    let here = store.namespace_resolve_or_create("team/tags/app", "spike", Some("oci")).expect("ns");
    let there = store.namespace_resolve_or_create("other/app", "spike", Some("oci")).expect("ns");

    store.tag_set(&here, "latest", &digest).expect("tag");

    assert_eq!(store.tag_get(&here, "latest").expect("get").kappa, digest);
    assert!(store.tag_get(&there, "latest").is_err(), "a tag does not leak across repositories");
    let names: Vec<String> = store.tag_list(&here).expect("list").into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["latest"]);
}

#[test]
fn q5_the_store_does_not_make_a_blake3_address_for_a_sha256_upload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let mut blake = blake3::Hasher::new();
    blake.update(&frame(0));
    let blake_digest = format!("blake3:{}", blake.finalize().to_hex());
    let (id, digest) = stream_in(&store, "library/alpine", 1);
    let result = store.upload_complete(&id, Some(&digest)).expect("complete");

    // Records the fact ADR-030 stands on. If this starts failing, upstream
    // changed the default axes and the alias table can be simplified.
    assert!(result.additional_kappas.iter().all(|k| !k.starts_with("blake3:")));
    assert!(!store.blob_exists(&blake_digest).expect("exists"));
}

#[test]
fn q6_what_survives_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (finished, half_done) = {
        let store = open(dir.path());
        let (id, digest) = stream_in(&store, "library/alpine", 2);
        store.upload_complete(&id, Some(&digest)).expect("complete");
        let namespace = store.namespace_resolve_or_create("library/alpine", "spike", Some("oci")).expect("ns");
        store.tag_set(&namespace, "v1", &digest).expect("tag");
        let (open_id, _) = stream_in(&store, "library/alpine", 1);
        (digest, open_id)
    }; // store dropped: the redb lock is released

    let store = open(dir.path());
    let namespace = store.namespace_resolve_or_create("library/alpine", "spike", Some("oci")).expect("ns");

    assert!(store.blob_exists(&finished).expect("exists"), "finished blobs survive");
    assert_eq!(store.tag_get(&namespace, "v1").expect("tag").kappa, finished, "tags survive");
    // Today this is None: sessions are an in-memory map and staging is wiped
    // at open (research K5). The verdict records the observed value; patch
    // 0003 in P1 T6 turns it into Some(4 MiB).
    println!("VERDICT q6 half-done upload after restart: {:?}", store.upload_bytes_received(&half_done));
}

#[test]
fn q7_a_second_opener_fails_fast() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _first = open(dir.path());
    let started = std::time::Instant::now();
    let config = PersistentStoreConfig::new(dir.path().join("kappa/blobs"), dir.path().join("kappa/kappa.redb"));

    let second = PersistentStore::new(config, Arc::new(WallClock));

    assert!(second.is_err(), "garbage collection relies on this to refuse beside a live server");
    assert!(started.elapsed().as_secs() < 2, "it must fail, not wait");
}

/// Run alone, in release, under a memory meter. See Step 5.
#[test]
#[ignore = "2 GiB; run by hand and by the CI job"]
fn q8_two_gib_streams_with_flat_memory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let started = std::time::Instant::now();
    let (id, digest) = stream_in(&store, "big/layer", 512);
    let streamed = started.elapsed();
    store.upload_complete(&id, Some(&digest)).expect("complete");
    println!("VERDICT q8 stream {streamed:?}, complete {:?}", started.elapsed() - streamed);
}
```

- [ ] **Step 2: Run it and see it fail.**

Run: `cargo test --features oci --test oci_spike`
Expected before Task 1 is merged into your branch: FAIL, unresolved crate `kappa_core`. After: compile errors are the first findings. Each one is a signature that differs from `research.md` K1, K10 or K12; fix the test to match the code and note the difference for the verdict.

- [ ] **Step 3: Make q1 to q4 pass.** No product code exists; "make it pass" means the store behaves as the plan assumes. A failure here is a finding, not a bug to patch around:

| Fails | Means | Goes to |
|---|---|---|
| q1 | the staged upload path does not address by the claimed sha256 | verdict: blocks P1 T5; read `upload_complete` again |
| q2 | `blob_open` is not a real file for staged uploads (for example it is encrypted or compressed at rest) | verdict: P3 T4 cannot seek; needs `blob_get_range` in frames |
| q3, second assert | a refused upload leaves the bytes reachable | verdict: required upstream fix |
| q4 | namespaces do not isolate tags | verdict: tags move into `links.redb`; +1 day in P1 |

- [ ] **Step 4: Record q5, q6, q7.** q5 and q7 must pass. q6 prints a line; copy it into the verdict. If q7 fails because the second open **succeeds**, P8 T1 needs its own lock file (risk table); if it **hangs**, same, and say so loudly.

- [ ] **Step 5: Peak memory at 2 GiB.**

```bash
cargo test --release --features oci --test oci_spike --no-run
bin=$(ls -t target/release/deps/oci_spike-* | grep -v '\.d$' | head -1)
/usr/bin/time -v "$bin" --ignored --nocapture q8 2>&1 | grep -E "VERDICT|Maximum resident"
```

macOS: `/usr/bin/time -l`. Windows PowerShell:

```powershell
$p = Start-Process -FilePath $bin -ArgumentList '--ignored','--nocapture','q8' -PassThru -NoNewWindow
while (-not $p.HasExited) { $peak = [Math]::Max($peak, $p.PeakWorkingSet64); Start-Sleep -Milliseconds 200 }
"peak MB: {0:N0}" -f ($peak / 1MB)
```

Expected: under 100 MB, and the same within 10 MB at 1 GiB (`512` → `256`). What the product needs is that memory does not grow with size (SC-007). Also record the two timings: `complete` re-reads the whole staging file to hash it (K4), so it should be about the time of one sequential read of 2 GiB.

- [ ] **Step 6: Size the restart patch.** Read `crates/kappa-store-redb/src/lib.rs` lines 195 to 225 (staging wipe) and 715 to 745 (`upload_begin`). Write down, for the verdict, the smallest change that lets a session resume (`data-model.md`, "Upload sessions across a restart" has the design) and its size in lines. Do not write the patch; that is P1 T6.

- [ ] **Step 7: Commit.**

```bash
cargo fmt --all && cargo clippy --features oci --test oci_spike --locked -- -D warnings
git add -A && git commit -m "spike(oci): store round trip, restart and lock behaviour"
```

**Done when:** q1 to q5 and q7 pass on Linux; q6 and q8 lines are captured.

**Traps:**
- `PersistentStore::new` needs a `Clock`; the only one upstream ships is `NtpLamportClock` (K10). The test brings its own so it does not depend on that type's path.
- `upload_begin` takes `max_size`; `0` means unlimited (K1).
- Tests share nothing, but `just test` runs with one thread (R26). q8 is `#[ignore]` so it never runs there.
- Clippy pedantic applies to tests. The casts above use `try_from`; keep it that way.

---

## Task 3: Three systems (engineer A, day 2)

**Requirements:** the open decision; FR-016 (which binaries can exist).

**Files:**
- Modify: `.github/workflows/ci.yml` (one job, removed again in P1 T1)

- [ ] **Step 1: Add the job.** Append under `jobs:`:

```yaml
  registry-spike:
    if: startsWith(github.ref, 'refs/heads/registry/p0-')
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-24.04, macos-14, windows-2022]
    runs-on: ${{ matrix.os }}
    timeout-minutes: 60
    env:
      CARGO_INCREMENTAL: "0"
      CARGO_NET_GIT_FETCH_WITH_CLI: "true"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.97.1
      - uses: Swatinem/rust-cache@v2
        with:
          key: registry-spike-${{ matrix.os }}
      - name: Build with the Kappa crates
        run: cargo build --locked --features oci
      - name: Store round trip
        run: cargo test --locked --features oci --test oci_spike -- --nocapture
      - name: 2 GiB stream (timing only; memory is measured by hand)
        run: cargo test --release --locked --features oci --test oci_spike -- --ignored --nocapture q8
      - name: New native code
        shell: bash
        run: cargo tree --features oci -e normal --prefix none | sort -u | grep -E -- "-sys " || true
      - name: musl builds (Linux only, informational)
        if: runner.os == 'Linux'
        continue-on-error: true
        run: |
          rustup target add x86_64-unknown-linux-musl
          sudo apt-get update && sudo apt-get install -y musl-tools
          cargo build --locked --features oci --target x86_64-unknown-linux-musl
```

- [ ] **Step 2: Push and read three results.**

```bash
timeout 120 git -c http.version=HTTP/1.1 push -u origin registry/p0-spike
gh run watch --exit-status || gh run view --log-failed
```

- [ ] **Step 3: Classify every failure.** One row per failure in the verdict:

| Class | Example | Whose | 
|---|---|---|
| Native build | `lzma-sys` needs a C compiler flag on MSVC | upstream: ask for codecs behind a feature; carry the patch |
| Path handling | `\` in a blob path, a `:` from `sha256:` in a file name on Windows | upstream; blocks Windows |
| Hard links | alt address missing on a volume without them (K7) | not a blocker: ADR-030 does not use them |
| File locking | q7 behaves differently per system | ours: own lock file in P8 |
| Rename over an open file | Windows refuses; Unix allows | ours: P1 T5 must close the reader first |

Fix what is ours if it takes under an hour. Write down the rest with a size.

**Done when:** the job has run on all three systems and each failure has a class, an owner and a size.

**Traps:**
- `windows-2022` has MSVC and CMake; `lzma-sys` and `bzip2-sys` build from bundled C with `cc`. If one looks for a system library instead, set its `static` feature through a `[patch]`-free route: a feature request upstream.
- A blob path that contains `:` is illegal on Windows. Check `kappa_core::kappa::blob_path_for` output in the q1 failure before anything else.
- The musl step is allowed to fail. Its result feeds decision 6 (image base), not go or no-go.

---

## Task 4: The verdict (engineer B, day 2 afternoon; both sign)

**Files:**
- Create: `docs/superpowers/specs/<date>-registry-p0-verdict.md`

- [ ] **Step 1: Fill the template. One page. Every blank is a number, a yes or no, or a link.**

```markdown
# Hologram Registry v1, P0 verdict

Date: …  Branch: registry/p0-spike @ …  CI run: <link>

## Verdict: GO / NO-GO
One sentence why.

## 1. How the crates are pinned
Route that worked: (a) fork + rev / (b) path checkout / (c) vendored copy.
Error text of the plain upstream dependency: …
Pin revision: …  Based on upstream: 2af8656…  Carried commits: N.
dcbor pinned at …; rekindle-aead pinned at …; both mirrored: yes / no.

## 2. Native code added
New -sys crates: …   Need a system package on any system: yes / no (which).
Forbidden crates in the graph (topcoat, veilid, openssl, aws-lc): none / ….
New crates in the normal graph: N.

## 3. Build time and binary size (release, clean)
| | before | after | delta |
| Linux build time | | | |
| Linux binary size | | | |

## 4. Memory while streaming 2 GiB in 4 MiB frames
Peak RSS: … MB at 2 GiB; … MB at 1 GiB. Grows with size: yes / no.
Stream time: … s. upload_complete time: … s (expected: one sequential read).

## 5. What survives a restart
Finished blobs: yes / no. Tags: yes / no. Half-done upload: <the q6 line>.
Second opener: fails in … ms / succeeds / hangs.

## 6. Result per system
| System | Builds | q1–q7 | q8 | Notes |
| Linux x86_64 gnu | | | | |
| macOS aarch64 | | | | |
| Windows x86_64 MSVC | | | | |
| Linux x86_64 musl (informational) | | | | |
Systems v1 commits to: …

## 7. Upstream fixes v1 needs
| # | Fix | Why | Size (lines) | Blocks |
| 0001 | remove [[test]] from the virtual manifest | cargo cannot parse it | 3 | everything |
| 0002 | pin fork-branch dependencies by rev | supply | 2 | release |
| 0003 | durable upload sessions | FR-006 | … | P1 T6 |
| 0004 | fsync blob data before rename | power loss | … | release |
| 0005 | skip S3 part digests when not asked | speed at 20 GB | … | only if section 4 is slow |
| … | LICENSE file | crates declare MIT OR Apache-2.0, ship no file | 0 | release |
Total: N patches, M lines. Go threshold: under 5 required patches and 300 lines.

## 8. What differed from research.md
Signatures, behaviours, surprises. Each with the file and line.

## 9. Changes to the plan
Phase order, days, or ADRs this verdict changes. "None" is a valid answer.
```

- [ ] **Step 2: Decide.** Go if: Linux green; at least one of macOS or Windows green; q1, q3, q7 hold; required upstream fixes are under 5 patches and 300 lines. Otherwise no-go, and the re-plan starts from Task 1 step 4 (d).

- [ ] **Step 3: Send it to the maintainer. Do not start P1 without a recorded go.** Decision 1 in `plan.md` is his.

**Done when:** the verdict file exists with no blank, **and** the `registry-spike` job is green on every system section 6 commits to, **and** `check-kappa-pin.sh` passes. The last two can fail.

**Traps:**
- A go with Windows red is legitimate. Then FR-016's Windows binary is a listed limitation in `apps/registry/DIFFERENCES.md` and the release notes, and section 9 says so. It is not a cut: the requirement names no systems.
- Do not tidy the spike into product code. P1 T1 starts from main and takes only `Cargo.toml`, `third_party/kappa/` and `scripts/check-kappa-pin.sh` from this branch.
