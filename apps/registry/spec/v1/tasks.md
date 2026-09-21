# Tasks: Hologram Registry v1

**Revision 3:** built in `Hologram-Technologies/hologram-live`; product files under `apps/registry/`; feature `oci` off by default; every pull request targets the parent's `main` (see `plan.md` section 0).

**Input**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `phases/`.
**Format**: `- [ ] T### [P?] [Story] P<phase>.T<n> (owner, days) Description → main file. Closes/serves: requirement ids.`
`[P]` = can run in parallel with its neighbours (different files, no dependency). Stories: US1 swap the image name, US2 secure and maintain, US3 move data in and out, US4 prove integrity. `[F]` = foundation, serves every story.
Each task's steps, test code and traps are in its phase file. Owner A = protocol track, B = gates, configuration, operations.

## Phase 0: Spike (days 1 to 2): go or no-go — DONE 2026-09-21, verdict GO on one condition (`p0-verdict.md`)

- [x] T000 [F] P0.T0 (B, 0.25) ADR-025 as a draft pull request to the parent; `apps/registry/README.md` scaffold → `specs/adrs/025-kappa-crates-in-the-graph.md`, `apps/registry/README.md`. Serves: decision 8
- [x] T001 [F] P0.T1 (A, 1) Pin `kappa-core` and `kappa-store-redb` behind feature `oci`; fork with carried commits; pin gate; product boundary rule for the default graph → `Cargo.toml`, `third_party/kappa/`, `scripts/check-kappa-pin.sh`, `scripts/check-product-boundaries.sh`. Serves: FR-019, the open decision
- [x] T002 [P] [F] P0.T2 (B, 1.25) Store round trip, restart and lock tests → `tests/oci_spike.rs`. Serves: FR-006, FR-020 (feasibility)
- [x] T003 [F] P0.T3 (A, 1) Three-system CI job → `.github/workflows/ci.yml`. Serves: FR-016
- [x] T004 [F] P0.T4 (B, 0.75) Verdict, sent to the maintainer → `docs/superpowers/specs/<date>-registry-p0-verdict.md`. Serves: decision 1

**Checkpoint: go recorded by the maintainer 2026-09-21.** ADR 025 is draft pull request #90 on the parent. The plain upstream dependency built, so no Kappa fork exists yet: it is created in T010. **New gate on every merge into the parent: carried patch A (optional `rekindle-aead`) so `aws-lc` leaves the `oci` graph and `check-kappa-pin.sh` passes in strict mode.**

## Phase 1: Store adapter (A, days 3 to 8) — CODE COMPLETE 2026-09-21: T005–T012 done on `registry/p1-store-adapter` (ed3f1f3); upstream PRs UOR-Foundation/kappa-registry #13 #14 #15 + issue #16; fork moved to Hologram-Technologies/kappa-registry; pin gate strict. Phase exit waits on the final three-system CI run; carried patch A (optional encryption, removes aws-lc) done early, fork `humuhumu33/kappa-registry` @ c7b2ee7 (6 carried commits: optional encryption, resumable uploads, sync before rename)

- [x] T005 [F] P1.T1 (1) Feature `oci` off by default + `oci-check` recipe, layout version 1, open, refusal of a reference volume → `src/oci_store/mod.rs`, `layout.rs`. Serves: FR-012; ADR-029
- [x] T006 [P] [F] P1.T2 (0.5) `Digest`, `RepoName`, `Tag`, `Reference`, `UploadId` → `src/oci_store/types.rs`. Serves: FR-004, FR-022; ADR-031
- [x] T007 [F] P1.T3 (1) `links.redb`: links, repos, referrers, aliases → `src/oci_store/links.rs`. Serves: FR-009; ADR-027
- [x] T008 [F] P1.T4 (0.75) Scoped blob reads, alias resolution, `AsyncBlob` → `src/oci_store/blobs.rs`. Serves: FR-002, FR-022; ADR-030
- [x] T009 [F] P1.T5 (1) Streaming uploads, hash on write, blake3 in the pass, `Framer`, streaming gate script → `src/oci_store/uploads.rs`, `scripts/check-oci-streaming.sh`. Closes: FR-020. Serves: FR-006
- [x] T010 [F] P1.T6 (1.25) Create the Kappa fork (deferred from P0); carried patch A optional `rekindle-aead` (removes `aws-lc`, 40 to 60 lines); sessions survive a kill: carried patch B → Kappa fork, `uploads.rs`, `tests/support/oci_child.rs`. Serves: FR-006, FR-019
- [x] T011 [F] P1.T7 (0.5) Manifests, tags, referrers in the store → `src/oci_store/manifests.rs`. Serves: FR-005, FR-009
- [x] T012 [F] P1.T8 (0.5) Upstream pull requests A to D (A optional aead, B durable sessions, C fsync before rename, D rev pins on mirrors) and the LICENSE issue, fork moved to the organisation, ADR-025/029/031, `registry-os.yml`. Closes: FR-019

## Phase 2: Gate harness against the reference alone (B, days 3 to 7), parallel with Phase 1

- [ ] T013 [P] [US1] P2.T1 (2) Differential runner: format, play, normalise, compare; reference pinned by digest → `apps/registry/gates/differential/`. Serves: FR-003, FR-013
- [ ] T014 [US1] P2.T2 (1) Twelve scenarios and golden transcripts; contract files corrected from them → `apps/registry/gates/differential/scenarios/`, `golden/`. Serves: FR-003
- [ ] T015 [US1] P2.T3 (0.5) `apps/registry/DIFFERENCES.md` as allowlist; stale rows fail → `apps/registry/DIFFERENCES.md`, `allowlist.rs`. Closes: FR-013 (mechanism)
- [ ] T016 [P] [US1] P2.T4 (1) Gate C client scripts, green against the reference → `apps/registry/gates/clients/*.sh`. Serves: FR-018
- [ ] T017 [P] [US3] P2.T5 (0.25) Gate D corpus and round trip script → `apps/registry/gates/corpus/`. Serves: SC-005
- [ ] T018 [US1] P2.T6 (0.25) `gates.yml`: `b-self` blocking, `c-reference` → `.github/workflows/gates.yml`. Serves: FR-017

**Checkpoint (day 7): reference vs reference = zero differences. Golden transcripts 1 to 6 handed to A.**

## Phase 3: Module and read path (A, days 9 to 12) — DONE 2026-09-21 on `registry/p3-module` (6a128eb, a40edac): `ci` and `registry-os` green on three systems. Built without golden transcripts (P2 not started): messages, details, multi-range, OPTIONS, 405 bodies and `/v2` without the slash are recalled and wait for gate B to correct them

- [x] T019 [US1] P3.T1 (1) `authenticates_itself`, two-list module macro, `OciRegistryModule` skeleton, store wired into `AppState` → `src/module.rs`, `src/modules/mod.rs`, `src/server.rs`, `src/app.rs`, `src/modules/oci/mod.rs`. Serves: FR-015, FR-021; ADR-026
- [x] T020 [P] [US1] P3.T2 (0.5) Path parser from the right → `src/modules/oci/path.rs`. Closes: FR-004
- [x] T021 [P] [US1] P3.T3 (0.5) Error envelope, 18 codes, store error mapping → `src/modules/oci/error.rs`, `respond.rs`. Serves: FR-003
- [x] T022 [US1] P3.T4 (1) Blob `GET`, `HEAD`, `Range`, streamed → `src/modules/oci/blobs.rs`. Serves: FR-002
- [x] T023 [US1] P3.T5 (0.5) Manifest `GET`, `HEAD` → `src/modules/oci/manifests.rs`. Serves: FR-002
- [x] T024 [US1] P3.T6 (0.5) Gate A job, pull category → `gates.yml`. Serves: SC-002

## Phase 4: Write path (A, days 13 to 17) — T026, T027 DONE and T025 PART DONE 2026-09-21 (0a90892): uploads and manifest `PUT` built, body limit test passes through the binary; the 2 GiB router memory test (`tests/oci_memory.rs`) is NOT written. T028 concurrency and T029 crash tests NOT started. Conformance pull + push: 49 of 49

- [ ] T025 [US1] P4.T1 (1.5) Chunked uploads, streamed in exact 4 MiB frames; memory test; body limit test → `src/modules/oci/uploads.rs`, `body.rs`, `tests/oci_memory.rs`. Closes: FR-006. Serves: SC-007
- [x] T026 [US1] P4.T2 (0.5) Monolithic upload, mount with source check → `uploads.rs`. Serves: FR-002, FR-009
- [x] T027 [US1] P4.T3 (1) Manifest `PUT` with validation → `manifests.rs`, `media.rs`. Closes: FR-005. Serves: FR-022
- [ ] T028 [P] [US1] P4.T4 (0.75) Concurrency tests, six rules → `tests/oci_concurrency.rs`. Serves: FR-006, FR-009
- [ ] T029 [P] [US1] P4.T5 (0.75) Crash tests, seven kill points → `tests/oci_crash.rs`. Serves: FR-006

**Checkpoint (day 17): `docker push` and `pull` work. US1 is usable without login.**

## Phase 5: Drop-in configuration, registry mode, image (B, days 8 to 12), parallel with Phases 1 and 3

- [ ] T030 [US2] P5.T1 (1.5) Key table, `REGISTRY_*`, `config.yml`, walk test → `src/registry_compat/`. Closes: FR-011. Serves: FR-008
- [ ] T031 [P] [US1] P5.T2 (0.5) `argv[0]` rewrite, `hologram oci` group → `src/main.rs`, `src/cli/oci.rs`. Serves: FR-001, FR-010
- [ ] T032 [US1] P5.T3 (1) Registry mode: modules, paths, token, gRPC on loopback (needs T019) → `src/config.rs`, `src/cli/serve.rs`, `src/server.rs`. Closes: FR-015. Serves: FR-021; ADR-028
- [ ] T033 [US1] P5.T4 (1) Dockerfile, default config, `image` job → `apps/registry/Dockerfile`, `apps/registry/config.yml`. Serves: FR-016, FR-001
- [ ] T034 [US1] P5.T5 (0.5) Compose swap test, basic file → `apps/registry/gates/clients/compose-swap.sh`, `apps/registry/gates/compose/`. Serves: FR-001, SC-001

## Phase 6: Login and TLS (B, days 12 to 16)

- [ ] T035 [US2] P6.T1 (1.25) htpasswd, challenge, cache, reload → `src/modules/oci/auth.rs`. Closes: FR-007
- [ ] T036 [US2] P6.T2 (1.25) TLS accept loop on `hyper-util` → `src/tls.rs`, `src/server.rs`. Closes: FR-008
- [ ] T037 [US2] P6.T3 (0.5) Admin listener stays plain on loopback under TLS → `src/server.rs`, `src/cli/oci.rs`. Closes: FR-021
- [ ] T038 [US2] P6.T4 (0.5) TLS and htpasswd compose files turned on → `apps/registry/gates/compose/`. Closes: FR-001, SC-001

## Phase 7: Discovery and management (A, days 17 to 20) — T039 to T042 and T044 DONE 2026-09-21 (6d23904, bb576a2): gate A, every category, 74 passed, 0 failed, 5 skipped, and blocking. T043 (read-only mode, upload purging) NOT started. Files differ from the plan: tags, catalogue and referrers share `listing.rs`. Delete switch is read from `REGISTRY_STORAGE_DELETE_ENABLED` until P5's key table lands

- [x] T039 [P] [US1] P7.T1 (0.5) Tags list with paging → `src/modules/oci/tags.rs`. Serves: FR-002
- [x] T040 [P] [US1] P7.T2 (0.25) Catalogue → `catalog.rs`. Serves: FR-002
- [x] T041 [P] [US1] P7.T3 (0.75) Referrers → `referrers.rs`. Closes: FR-002
- [x] T042 [US2] P7.T4 (0.75) Delete, off by default, scoped → `delete.rs`. Closes: FR-009
- [ ] T043 [US2] P7.T5 (0.5) Read-only mode, upload purging. Serves: FR-011
- [x] T044 [US1] P7.T6 (0.25) Gate A, all categories, blocking → `gates.yml`. Closes: SC-002

**Checkpoint (day 20): gate A fully green.**

## Phase 8: Operator commands (B then A, days 16 to 21)

- [ ] T045 [US2] P8.T1 (B, 1.5) `garbage-collect`, mark rule, property test, refusal beside a live server → `src/oci_store/gc.rs`. Closes: FR-010, SC-008; ADR-032
- [ ] T046 [P] [US3] P8.T2 (B, 1.5) `import` from a reference volume, re-runnable → `src/oci_store/import.rs`. Closes: FR-012
- [ ] T047 [P] [US4] P8.T3 (B, 1) `verify` as a polled job; defined consequence → `src/oci_store/verify.rs`, `src/modules/oci/admin.rs`. Closes: FR-014, SC-009
- [ ] T048 [US3] P8.T4 (A, 1) `adopt` the hub's store; provider suite against our `/v2/` → `src/oci_store/adopt.rs`. Serves: FR-022, SC-010
- [ ] T049 [US3] P8.T5 (B, 0.5) Hub provider moves to standard uploads → `src/registry/kappa_client.rs`. Closes: FR-022

## Phase 9: Gates on the product (both, days 19 to 25)

- [ ] T050 [US1] P9.T1 (A, 2) Gate B burn-down to zero unlisted, zero stale. Closes: FR-003, SC-003
- [ ] T051 [P] [US1] P9.T2 (B, 1) Gate C on the image, all clients. Closes: FR-018, SC-004
- [ ] T052 [P] [US3] P9.T3 (B, 1) Gate D per commit and nightly. Closes: SC-005
- [ ] T053 [P] [US3] P9.T4 (B, 1) Gate E: Harbor replication, Zot sync. Serves: FR-017
- [ ] T054 [P] [US1] P9.T5 (A, 0.5) Performance nightly. Closes: SC-006, SC-007
- [ ] T055 [US1] P9.T6 (A, 0.5) Blocking switch; `check-gates.sh`. Closes: FR-017

## Phase 10: Release, hub rehearsal, docs (both, days 23 to 27)

- [ ] T056 [US1] P10.T1 (B, 1.5) Release workflow: gates first, pin check, binaries, multi-arch image, checksums. Closes: FR-016
- [ ] T057 [US1] P10.T2 (B, 0.5) Dry run: a red tag (`server-v0.0.1-red`) must not publish; `server-v1.0.0-rc.1` must. Serves: FR-017, SC-010
- [ ] T058 [US3] P10.T3 (A, 1.5) Hub rehearsal, swap script, forced failure and rollback. Serves: SC-010, FR-022
- [ ] T059 [P] [US1] P10.T4 (B, 1) Operator docs generated from the key table; ADRs final. Serves: FR-013, FR-001
- [ ] T060 [P] [US1] P10.T5 (B, 0.5) Helm chart, one replica enforced. Serves: form factor
- [ ] T061 [US1] P10.T6 (both, 0) Release checklist, `server-v1.0.0`, hand-over to the maintainer. Closes: SC-010

## Dependencies

- T001 → T002, T003 → T004 → Phase 1. Phase 2 needs only T001 to T004 to be **finished**, not a go.
- Phase 1 → Phase 3 → Phase 4 → Phase 7 → T045, T048 → T050 → T058 → T061. This is the critical path.
- T019 → T032. T030 → T035, T036. T033 → gate jobs against the image (day 12).
- T013 to T015 → T050. T016 → T051. T017 → T052, T053.
- T049 ships to the hub before T058's swap is run.

## Parallel work, by example

Days 3 to 7: A on T005 to T009; B on T013 to T018. No shared file except one `exclude` line in `Cargo.toml` (B, day 3) and the dependency lines (A, day 3): B's line goes first, as its own commit.
Days 13 to 16: A on T025 to T029 (`src/modules/oci/`, `tests/`); B on T035 to T038 (`src/modules/oci/auth.rs`, `src/tls.rs`). One shared file, `src/modules/oci/mod.rs`: A owns it; B's layer is one line, added by pull request review.
