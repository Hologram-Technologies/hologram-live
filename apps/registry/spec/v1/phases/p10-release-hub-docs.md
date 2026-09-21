# P10. Release, hub rehearsal, docs, Helm

> Task level. **Steps to be written before the phase starts** (day 22).
> Entry: P9. Both engineers. 5 days (days 23 to 27). B: T1, T2, T4, T5. A: T3. Both: T6.

**Goal:** a published release with five green gates; a rehearsed, reversible hub swap handed to the maintainer. Exit: the release job refuses a tag whose gates are not green; the rehearsal script passes, rollback included.

**This phase never writes to the hub's host.** the maintainer runs the deploy.

---

## Task 1: Release workflow (B, 1.5 days)

**Requirements:** FR-016, FR-017. **Files:** modify `.github/workflows/release-server.yml` (R29: five targets, never run); create `apps/registry/gates/release/check-artifacts.sh`.

- Trigger on tags `server-v*`: one version, one tag, publishing binaries, both images and the chart (plan section 0, rule 6).
- First job: `apps/registry/gates/release/check-gates.sh ${{ github.sha }}` (P9 T6). Red gate, no release.
- Second job: `./scripts/check-kappa-pin.sh` without allowance, and an extra check that the README records the UOR Foundation agreement (FR-019: merge rights or a maintained fork, agreed before the first release). A missing line fails the release, not the build.
- Binaries: the existing five targets. A system the P0 verdict dropped is removed here and named in the release notes.
- Image: `docker buildx` on native runners (`ubuntu-24.04`, `ubuntu-24.04-arm`), one manifest list `ghcr.io/hologram-technologies/registry:<version>` and `:1`; image name until it moves to the organisation: `ghcr.io/humuhumu33/hologram-registry`.
- Step 2 of the step-level plan tries musl once: `cargo build --target x86_64-unknown-linux-musl`. Adopted only if it links with no patch (decision 6). Otherwise gnu and distroless `cc`, as built since P5.
- Artefacts: archives, `SHA256SUMS`, the image digest, `apps/registry/DIFFERENCES.md`, `third_party/kappa/README.md`.

**Done when:** `check-artifacts.sh <tag>` passes: every expected file exists on the release; each checksum verifies; `docker buildx imagetools inspect` shows `linux/amd64` and `linux/arm64`; the image's entrypoint, command, port and volume equal the reference's.

## Task 2: Dry run, including a refusal (B, 0.5 day)

**Requirements:** FR-017, SC-010.

1. On a scratch branch with a deliberately failing gate B scenario, push tag `server-v0.0.1-red`. **The release must not publish.** Delete the tag and branch.
2. On main, push `server-v1.0.0-rc.1`. It must publish. Run gate C once against the published image by digest.

**Done when:** both outcomes observed and linked in the release checklist. This is the test of FR-017 that can fail.

**Trap:** tags fire workflows. Push only the `server-v*` tags this task names; never a `desktop-v*`, `componentizer-v*` or docs tag.

## Task 3: Hub rehearsal and the swap script (A, 1.5 days)

**Requirements:** SC-010, FR-022. **Files:** create `apps/model-hub/deploy/swap-registry.sh`, `apps/model-hub/deploy/rehearse-registry.sh`; modify `apps/model-hub/deploy/docker-compose.yml` on a branch (service `kappa` → the v1 image), `README.md` there.

Rehearsal, all local (`rehearse-registry.sh`):
1. Build a mirror of `/root/hub` from the repository's deploy files plus a store built the hub's way: start pinned `kappa-server` (R30), publish 20 objects and one `model-hub/index:<date>` through the `hologram` service and `publish.sh`.
2. Ship the provider change first (P8 T5) against `kappa-server`; re-run the object checks. This is the state the hub is in before the swap.
3. Run `swap-registry.sh`: stop `kappa`; snapshot `./store` by hard-link copy (`cp -al`); `hologram oci adopt`; start the v1 image with the same volume and port 5000, anonymous, behind the same Caddy rules (H1: writes gated at Caddy stays as is); wait for `/v2/`.
4. Checks: `health.sh`; `hologram pull hub.uor.foundation/model-hub/index:<date>` (against the mirror's name); read one object through the `kappa` provider; publish one new object; `docker pull` of nothing is needed (the hub holds no images) but `crane catalog` must list the hub's repositories.
5. Force a failure (break the image name) and prove rollback: stop v1, remove `<root>/oci/` and the marker, start `kappa-server`, re-run the checks.

The compose change keeps the limits the hub has today: `mem_limit: 1g`, `cpus: 1.5`. The 1.1 GiB blob cap (`KAPPA_MAX_BLOB_SIZE`) has no v1 equivalent in the reference's settings; keep the cap at Caddy (`request_body max_size`) and say so in the README. Disk pressure refusal (`KAPPA_DISK_PRESSURE_THRESHOLD_MB`) has no equivalent either: note it as a gap for the maintainer, with the option of a Hologram-only setting `oci.min_free_mb` outside the `REGISTRY_*` namespace.

**Done when:** `rehearse-registry.sh` exits 0 end to end, including the forced failure and rollback, on a clean machine with Docker. The script never contacts the hub's host (asserted: it refuses to run if `HUB_HOST` is set).

**Traps:**
- The hub's `kappa` service mounts the binary into `debian:bookworm-slim`. The v1 service uses the published image by digest instead; no binary is copied to the hub's host.
- `cp -al` needs the snapshot on the same filesystem. It costs no space and makes rollback exact.
- The rehearsal pulls the published image; it does not build one.

## Task 4: Docs (B, 1 day)

**Requirements:** FR-013, FR-001. **Files:** `apps/registry/docs/*.md` (linked from `apps/docs`; the docs site is separate from the server graph): decided in step 1 by where the brief's three commands should live.

- Image README, led by the three commands in the brief.
- "Coming from Docker Registry": supported, ignored and refused settings (generated from `src/registry_compat/table.rs` by `hologram oci settings --markdown`, so it cannot drift); `apps/registry/DIFFERENCES.md`; migration in and out with `skopeo`, `crane` and `hologram oci import`; the garbage collection recipe (ADR-032); no shell in the image.
- "If something goes wrong": `plan.md` section 11, for operators.
- ADRs 025 to 032 final.

**Done when:** `scripts/check-docs-settings.sh` passes: every key in the table appears in the page with the same class.

## Task 5: Helm chart (B, 0.5 day)

**Requirements:** form factor, third. **Files:** `apps/registry/chart/`.

Values mirror the widely used `docker-registry` chart's names where they exist (`persistence`, `secrets.htpasswd`, `tlsSecretName`, `configData`), so a values file moves over. One StatefulSet replica (the store is single-writer: redb lock, K11), one PVC, one Service on 5000.

**Done when:** `helm lint` clean; `kubernetes.sh` variant installs the chart into kind, pushes and pulls.

**Trap:** `replicas: 2` must be refused by the chart's schema (`values.schema.json`, `maximum: 1`). Two pods on one volume means the second cannot open the store.

## Task 6: `server-v1.0.0` (both, day 27)

**Requirements:** SC-010.

Release checklist, each line a link: gates green on the commit; three green nights; cloud peers run by hand (AWS, Google, Azure) with digests equal; UOR Foundation agreement recorded; verdict systems match released binaries; `apps/registry/DIFFERENCES.md` reviewed row by row by both engineers; rehearsal green on the release image by digest.

Then tag. Then hand the maintainer: `swap-registry.sh`, the rehearsal log, the rollback steps on one page.

**Done when:** the release is published with five green gates. SC-010's second half, the hub serving from it, is done when the maintainer has run the swap and `health.sh` plus the three checks pass on the live hub.

---

## Phase exit

A published `server-v1.0.0`; a red tag proven unpublishable; rehearsal and rollback green. Closes FR-016, SC-010 (pending the maintainer's swap).
