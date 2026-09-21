# Hologram Registry v1, P0 verdict

Date: 2026-09-21. Branch: `registry/p0-spike` @ `a3b5e4c` on `humuhumu33/hologram-registry`.
CI runs: [35593011870](https://github.com/humuhumu33/hologram-registry/actions/runs/35593011870) (first: Linux and macOS green, Windows red),
[35594124604](https://github.com/humuhumu33/hologram-registry/actions/runs/35594124604) (with the Windows workaround: all four jobs green).

Marks: **[CI]** read from a CI log. **[local]** run on Ilya's Windows 11 machine. **[read]** read in the Kappa source, not run. **[estimate]**.

## Verdict: GO, on one condition

The Kappa store crates live inside the `hologram` binary on Linux, macOS and Windows, stream 2 GiB in 8.8 MB of
memory, and behave as the plan assumed in every test. **Condition:** Kappa's encryption crate pulls `aws-lc` into
the registry build. This repository keeps `aws-lc` out on purpose, and it broke the clean Windows build. One small
upstream patch removes it. Registry code does not merge into the parent until that patch is carried.

**Decision 1: GO, recorded by Ilya on 2026-09-21** ("proceed", in reply to this verdict). P1 may start on fork branches. The condition above still gates every merge into the parent.

## 1. How the crates are pinned

Route that worked: **the plain upstream git dependency, pinned by `rev`.** None of the plan's fallbacks was needed.

The plan expected this to fail with a manifest parse error, because upstream's workspace root declares `[[test]]`
in a virtual manifest. It did not fail: cargo 1.97.1 does not read that section when the workspace is a git
dependency. It only breaks a build of the upstream workspace itself. **[local, CI]**

Pin revision: `2af86560a177fc9651b6c0e92e7974140ed77dd5` (upstream `main`). Carried commits: 0. No fork exists yet.
`dcbor` locked at `2e5b901e8c9946794c5491cf92eeec2540b84261`; `rekindle-aead` locked at
`3fb5b80f2d5d3d5b5a59dded1a56b477cb9f23ca`. Both are named upstream by **branch** on personal forks; `Cargo.lock`
fixes the commit, so `--locked` builds reproduce while the commits stay fetchable. Mirrored: no. A deleted branch
would break the fetch even with the lock, so P1 still needs a fork that names them by `rev` on mirrors we control.

## 2. Native code added

New `-sys` crates in the registry graph: `lzma-sys`, `bzip2-sys`, **`aws-lc-sys`** (Linux also shows `ittapi-sys`
and `linux-raw-sys`, which are already in the default graph). **[CI]**
Need a system package on any system: **yes, Windows.** `aws-lc-sys` needs NASM; a clean `windows-2022` runner has
none and the first run failed there. With `AWS_LC_SYS_PREBUILT_NASM=1` it builds. **[CI]**
Forbidden crates in the graph: **`aws-lc-rs`, `aws-lc-sys`**, through `rekindle-aead` (blob encryption at rest,
which the registry does not use). `topcoat`, `veilid`, `openssl`: none. **[local]**
New crates in the normal graph: 94 (318 → 412). The default graph is unchanged; the extended product-boundary gate
passes. **[local]**

## 3. Build time and binary size

| | default | with `--features oci` | delta |
|---|---|---|---|
| Linux release binary size **[CI]** | 36,502,072 bytes | 36,501,208 bytes | none: nothing calls the crates yet, the linker drops them. Measure again at the end of P1 |
| Linux debug build, cold **[CI]** | not measured | 3 min 21 s | |
| macOS debug build, cold **[CI]** | not measured | 3 min 38 s | |
| Windows debug build, cold **[CI]** | not measured | 6 min 26 s | |
| Windows `cargo check`, warm registry **[local]** | | 2 min 08 s | |

A clean release build before and after was not timed; the CI step built both but was not instrumented. **Gap.**

## 4. Memory while streaming 2 GiB in 4 MiB frames

Peak memory: **8.8 MB** at 2 GiB (Linux, `/usr/bin/time -v`: 8,804 kB). **[CI]** Not measured at 1 GiB; at 8.8 MB
for 512 frames of 4 MiB the answer to "does it grow with size" is no. SC-007 allows 200 MB at 20 GB.

| System | Stream 2 GiB | `upload_complete` |
|---|---|---|
| Linux **[CI]** | 7.1 s (about 290 MB/s, including generating and hashing the bytes) | 2.0 s |
| macOS **[CI]** | 13.4 s | 2.2 s |
| Windows **[CI]** | 13.8 s | 1.9 s |

`upload_complete` re-reads the staged file once to hash it, as the plan said: about 1 GB/s on Linux.
The per-part cost the plan feared (a file open, MD5 and two CRCs per call) does not hurt at 4 MiB frames. Patch
0005 (skip S3 part digests) is **not needed**.

## 5. What survives a restart

Finished blobs: yes. Tags: yes. Half-done upload: `VERDICT q6 half-done upload after restart: None` on all three
systems. **[CI, local]** Sessions are an in-memory map and staging is wiped at open, as predicted.
Second opener of the same store: **fails, in under 2 s, on all three systems.** Garbage collection can rely on it;
P8 needs no lock file of its own.

## 6. Result per system

| System | Builds | q1–q7 | q8 (2 GiB) | Notes |
|---|---|---|---|---|
| Linux x86_64 gnu **[CI]** | yes | 7 of 7 | pass | pin gate passes in spike mode |
| macOS aarch64 **[CI]** | yes | 7 of 7 | pass | |
| Windows x86_64 MSVC **[CI + local]** | yes, **only with the NASM workaround** on a clean runner | 7 of 7 (CI and local) | pass | no path, `:`-in-file-name, rename or lock problem appeared |
| Linux x86_64 musl **[CI, informational]** | yes | not run | not run | feeds decision 6: a static image is possible |

Systems v1 commits to: **Linux x86_64 and aarch64, macOS aarch64, Windows x86_64.** Windows is unconditional once
`aws-lc` leaves the graph.

## 7. Upstream fixes v1 needs

| # | Fix | Why | Size **[estimate]** | Blocks |
|---|---|---|---|---|
| A | `rekindle-aead` behind a feature in `kappa-core` and `kappa-store-redb` | removes `aws-lc`; used in two files only (`kappa-core/src/crypto/aead.rs`, `kappa-store-redb/src/encrypted.rs`) **[read]** | 40 to 60 lines | **any merge to the parent**; Windows builds |
| B | durable upload sessions: persist the session record, stop wiping `staging/` at open, expire by last activity | FR-006 | 80 to 120 lines (`kappa-store-redb/src/lib.rs` near 200 and 717) **[read]** | P1 T6 |
| C | sync blob data before the rename in `upload_complete` (`lib.rs:964`); `blob.rs` already does it behind its `fsync` flag **[read]** | power loss | under 10 lines | a release |
| D | pin `dcbor` and `rekindle-aead` by `rev`, on mirrors | a deleted branch must not break the build | 2 lines, plus two mirror forks | a release |
| E | LICENSE file | the crates declare `MIT OR Apache-2.0` and ship no file | 0 | a release |
| not needed | remove `[[test]]` from the virtual manifest | does not affect a dependent | | nothing |
| not needed | skip S3 part digests | section 4 | | nothing |
| nice to have | `zstd`, `xz2`, `bzip2` behind a feature | drops `lzma-sys`, `bzip2-sys` | 30 lines | nothing |

Required: **4 patches (A to D), about 130 to 190 lines.** Go threshold: under 5 required patches and 300 lines. Met.

## 8. What differed from `research.md`

- K15: the unparseable workspace root does **not** stop a git dependency. Fallbacks (a) to (c) were not needed.
- K14 was incomplete: besides the bundled C codecs, `rekindle-aead` pulls `aws-lc-rs` and `aws-lc-sys`.
- K3 (per-part overhead): real in the code, harmless at 4 MiB frames (section 4).
- Every signature the test used (K1, K8, K10, K12) matched: `tests/oci_spike.rs` from the plan compiled unchanged.
- `ci.yml` runs on every push, so the existing `rust` job ran on the spike branch too and stayed green: the default
  build and its gates are unaffected by the optional dependency.

## 9. Changes to the plan

1. **P0 T1 steps 3 and 4 are void** (no fork to compile). The fork moves to **P1 T6**, where the first real patch
   (B) needs it. P1 T8 opens A to D upstream.
2. **New entry condition for the first registry pull request to the parent: patch A carried**, so `aws-lc` is out
   of the `oci` graph and `check-kappa-pin.sh` passes in strict mode. Until then the pin gate runs with
   `KAPPA_PIN_ALLOW_AWS_LC=1` on spike branches only.
3. **Remove** carried patch `0001` and `0005` from the plan's lists. **Remove** the "own lock file" risk row.
4. ADR-025 (draft written, `specs/adrs/025-kappa-crates-in-the-graph.md` on `registry/app-scaffold`) already records
   the `aws-lc` rule. It has **not** been opened on the parent; that is Ilya's call (decision 8).
5. Days: P0 took one working session, not four engineer days. No other estimate changes on this evidence.

## Not verified

- Peak memory on macOS and Windows (measured on Linux only), and at 1 GiB.
- Clean release build time before and after.
- The size estimates in section 7: they come from reading the upstream code, not from writing the patches.
- aarch64 Linux: not in the spike matrix.
