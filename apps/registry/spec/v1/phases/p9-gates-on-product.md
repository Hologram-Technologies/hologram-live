# P9. The gates, pointed at the product

> Task level. **Steps to be written before the phase starts** (day 19).
> Entry: P7 (gate A green), P5 (image), P6 (login, TLS). Both engineers. 6 days (days 19 to 25). A: T1, T5, T6. B: T2, T3, T4.

**Goal:** the five gates this plan owns (A to E) judge the image on every commit or every night, and block what they should. Gates F, G and H belong to the Server specification and block the same tag (plan section 0, rule 8). Exit: `gates.yml` blocks merges on A, B, C and the per-commit part of D; nightly D, E and performance green three nights running.

The tools exist since P2. This phase is the burn-down and the wiring.

---

## Task 1: Gate B burn-down (A, 2 days)

**Requirements:** FR-003, FR-013, SC-003.

Gate B has run against the product since day 12, red allowed. Now: run all scenarios (12 from P2, about 25 added in P5 to P8); sort every `Difference` into **fix** (the default) or **list** (a row in `apps/registry/DIFFERENCES.md` with a reason an operator accepts). Expected rows, known today:

| id | What | Why kept |
|---|---|---|
| D-001 | `blake3:` digests accepted | FR-022 |
| D-002 | an unknown `REGISTRY_*` variable stops the start | a typo in a security setting must not pass (decision 5) |
| D-003 | referrers route served (if the reference does not) | OCI 1.1 conformance needs it |
| D-004 | `garbage-collect` refuses beside a live server | ADR-032 |
| D-005 | settings refused that the reference accepts (`storage.s3`, `proxy`, `auth.token`, …): one row, `step = *` | out of scope, refused loudly (FR-011) |

Anything else is fixed unless both engineers agree it cannot break a client on the gate C list.

**Done when:** `differential run --left reference --right image:<sha> --allowlist apps/registry/DIFFERENCES.md` exits 0 in CI; zero unlisted, zero stale; `continue-on-error` removed from `gate-b` on day 24.

**Trap:** do not list a difference to get green. Each row is read by an operator deciding whether to trust the swap.

## Task 2: Gate C on the image (B, 1 day)

**Requirements:** FR-018, SC-004. In `gates.yml`, `c-reference` stays (it proves the scripts); a new job `gate-c` runs the same scripts with the image built by the `image` job. Add `kubernetes.sh`, `ollama.sh`, `surface.sh`, and the service-container job:

```yaml
  gate-c-service:
    runs-on: ubuntu-24.04
    services:
      registry: { image: "ghcr.io/${{ github.repository }}:sha-${{ github.sha }}", ports: ["5000:5000"] }
    steps:
      - run: docker pull -q alpine:3.20 && docker tag alpine:3.20 localhost:5000/s/alpine && docker push -q localhost:5000/s/alpine
```

**Done when:** every client on the FR-018 list completes login, push and pull against the image; job under 12 minutes, or split in two (risk table).

## Task 3: Gate D (B, 1 day)

**Requirements:** SC-005. Per commit: `roundtrip.sh reference <image> reference` and `roundtrip.sh zot <image> zot`, corpus with the 512 MiB layer. Nightly (`gates-nightly.yml`, 02:00 UTC): the same with the 5 GB layer, plus Harbor (its own compose, started in the job), GitHub Packages (`GITHUB_TOKEN`), Docker Hub (`DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN`). Before a release, by hand, recorded in the release checklist: AWS, Google, Azure.

**Done when:** every digest in `corpus.lock` is found at every hop, for every peer on that schedule.

**Trap:** Docker Hub rate limits and refuses some artifact types. A peer that refuses an artifact the reference also fails to copy there is not our failure: run `reference → peer` first and skip what the reference cannot do, by name, in the job log.

## Task 4: Gate E (B, 1 day)

**Requirements:** FR-017. Nightly: a Harbor replication rule (pull-based, our image as the source endpoint of type `docker-registry`) and a Zot `sync` extension config, each pulling the corpus **from** us; then every digest in `corpus.lock` must be in the peer.

**Done when:** both peers hold the whole corpus after one sync run.

**Trap:** Harbor lists repositories through `_catalog` and tags through `tags/list` with paging. This gate is the real test of P7 T1 and T2.

## Task 5: Performance (A, 0.5 day)

**Requirements:** SC-006, SC-007. Nightly, same GitHub-hosted runner class for both sides, named in the job (closes CHK013): `ubuntu-24.04`, 4 vCPU, the runner's local SSD, loopback network. Reference first, then the image, three runs each, median:
- 5 GB push and pull with `crane`: ours within 20% of the reference;
- 20 GB single-layer push: container RSS sampled every second from `docker stats`, fail over 200 MB;
- 16 parallel pulls of the 512 MiB layer: all digests right, no error.

**Done when:** three consecutive nights green. If push is slow, profile in this order: blake3 in the pass (make it lazy, P1 T5 trap), per-part digests in the store (patch 0005, K3), the second read in `upload_complete` (K4; cannot be removed without an upstream change to hash while staging).

## Task 6: Blocking switch and the release condition (A, 0.5 day)

**Requirements:** FR-017. Remove the last `continue-on-error` lines on their dates (`plan.md` section 6). Add `apps/registry/gates/release/check-gates.sh <sha>`: uses `gh api` to assert `gates.yml` concluded `success` on that commit and the latest `gates-nightly.yml` run is `success` and younger than 36 hours. The release workflow calls it first (P10 T1).

**Done when:** `check-gates.sh` exits non-zero for a commit with a red gate (tested against a known red commit on a scratch branch) and zero for a green one. Measured wall times written to `apps/registry/gates/README.md`, replacing the assumptions in `plan.md` section 6.

---

## Phase exit

Per commit: A, B, C, D-commit blocking. Nightly: D, E, performance green three nights. Closes FR-003, FR-017, FR-018, SC-003 to SC-007.
