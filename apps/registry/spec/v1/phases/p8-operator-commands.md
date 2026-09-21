# P8. Operator commands: garbage-collect, import, verify, adopt

> Task level. **Steps to be written before the phase starts** (day 15).
> Entry: P1 (import, verify); P7 (garbage-collect, adopt: they need the final link and referrer shapes). 5.5 days. B: T1, T2, T3 (days 16 to 19) and T5. A: T4 (day 21).

**Goal:** the four commands under `hologram oci`, and the hub's own provider moved onto standard uploads. Exit: the property test passes 256 cases; import twice adds nothing; 100 of 100 flipped bytes are named.

**Files:** `src/oci_store/gc.rs` (400), `import.rs` (450), `verify.rs` (300), `adopt.rs` (300); `src/cli/oci.rs` (+300); `src/modules/oci/admin.rs` (200, the verify operations).

---

## Task 1: `garbage-collect` (ADR-032)

**Requirements:** FR-010, SC-008. **Owner:** B.

**Interfaces:**
- `pub fn collect(root: &Path, options: GcOptions) -> Result<GcReport, OciStoreError>`; `GcOptions { dry_run, delete_untagged, quiet }`; `GcReport { marked, swept_blobs, swept_bytes, removed_links }`
- Opens with `OciStore::open(root, OpenOptions { create: false, .. })`. `OciStoreError::Locked` → message `the registry is running on this volume; stop it first (see apps/registry/DIFFERENCES.md, D-004)`, exit 1, nothing changed
- The mark rule and the sweep: `data-model.md`, "Garbage collection". A manifest that does not parse stops the run before any sweep
- CLI: `hologram oci garbage-collect [--dry-run] [--delete-untagged] [--quiet] --registry-config <file>`; reached from `registry garbage-collect …` by the `argv[0]` rewrite (P5 T2)
- Output lines under `--dry-run`: the reference's wording, from scenario `gc-dry-run-output` (run the reference's own command in its container, capture stdout)

**Done when:** `tests/oci_gc_property.rs` passes 256 cases per run (fixed seed list plus one random seed printed on failure): random sequences over three repositories sharing layers of push, tag, untag, delete manifest, delete blob, attach referrer, collect, collect `--delete-untagged`; after every collect, every tag pulls completely and every referrer of a tagged manifest is listed and pulls. Plus `tests/oci_concurrency.rs::refuses_beside_live_server` (enabled now). Plus `--dry-run` leaves a byte-identical directory tree (hash the tree before and after).

**Traps:**
- Never call Kappa's own collection. It treats one namespace as the whole truth and deletes the rest (brief, scope document).
- An index inside an index: recurse, with a depth cap of 8. Over the cap is an error and nothing is swept.
- Aliases: marking `sha256:X` marks its `blake3:` twin. Sweeping one address of a hard-linked pair while the other is marked would be harmless on Unix and wrong in the tables; mark both.
- Operators do run `docker exec registry registry garbage-collect` on a live reference. Ours refuses. The docs must show the three-command recipe (ADR-032) where they will look for it.

## Task 2: `import`

**Requirements:** FR-012. **Owner:** B.

**Interfaces:** `pub fn import(source: &Path, into: &Path, progress: &mut dyn FnMut(ImportEvent)) -> Result<ImportReport, OciStoreError>`. Reads the reference's filesystem layout **[memory; the gate D corpus volume is the fixture that proves it]**:

```
<source>/docker/registry/v2/blobs/sha256/<2 hex>/<64 hex>/data
<source>/docker/registry/v2/repositories/<name>/_layers/sha256/<hex>/link
<source>/docker/registry/v2/repositories/<name>/_manifests/revisions/sha256/<hex>/link
<source>/docker/registry/v2/repositories/<name>/_manifests/tags/<tag>/current/link
```

Order: for each repository, every `_layers` link → stream the blob file through `upload_begin`/`upload_append`/`upload_finish` (so every byte is hashed, FR-020), skipping digests already linked; then manifests by revision (children before indexes: two passes); then tags. The source is opened read-only and never written.

**Done when:** `tests/oci_import.rs`: a reference volume holding the gate D corpus imports; every tag pulls with its original digest; `second_run_adds_nothing` (report shows zero new objects, zero bytes written); `interrupted_run_finishes` (kill the child at 50%, run again, same end state); a source blob whose bytes do not match its path is reported by name and skipped, exit code 1, everything else imported.

**Traps:**
- Repository names contain slashes, so `repositories/` is a tree: a directory is a repository if it has `_manifests`.
- A layer link can point at a blob that garbage collection already removed from the source. Report it; do not stop.
- Large volumes: one `links.redb` transaction per repository, not per blob and not one for everything.

## Task 3: `verify`

**Requirements:** FR-014, SC-009. **Owner:** B.

gRPC has one unary method (R11), so verify is a job with two operations on the existing `Call`, declared in the module descriptor:
- `oci.verify.start` `{ throttle_mb_per_s: Option<u32> }` → `{ job: String }` (one job at a time; a second start returns the running job)
- `oci.verify.status` `{ job }` → `{ state: "running" | "done", checked, total, bytes, damaged: [{ digest, size, repositories: [{ name, tags }] }] }`

The job runs on the blocking pool: for each blob in `blob_list()`, open, hash in 1 MiB reads with the algorithm of its address, compare; for damaged blobs, find the repositories and tags that reach it by walking `links` and manifests. CLI `hologram oci verify [--json] [--throttle <MB/s>]` starts, polls once per second, prints progress to stderr, prints the report, exits 1 if anything is damaged. With no server running it opens the store directly and runs the same function.

**Consequence of damage, defined (closes CHK021):** verify reports and changes nothing. It never deletes, never quarantines, and the server keeps serving. The exit code and `--json` are for the operator's own automation. Repair is: re-push the named tags, or restore the named files.

**Done when:** `tests/oci_verify.rs::names_every_flipped_byte`: 1,000 blobs, flip one byte in each of 100 chosen by seed; the report names exactly those 100, with their repositories and tags; exit code 1. `::a_clean_store_exits_zero`. `::runs_beside_a_live_server_and_beside_pushes` (closes CHK020: a blob that appears mid-run is either checked or not counted; never reported damaged).

**Trap:** an upload's staging file is not a blob; do not list it. A blob renamed into place mid-run is whole by then (rename is atomic).

## Task 4: `adopt`

**Requirements:** FR-022, SC-010. **Owner:** A (day 21).

**Interfaces:** `pub fn adopt(blob_root: &Path, kappa_db: &Path, into: &Path) -> Result<AdoptReport, OciStoreError>`; design in `data-model.md`, "Adopting the hub's store". Refuses if the store is locked. Writes only under `<into>/oci/` and the marker; the Kappa paths are bind-mounted or moved by the operator, never modified.

**Done when:** `tests/oci_adopt.rs`: build a store the way the hub's provider does (through `src/registry/kappa_client.rs` against a pinned `kappa-server` from `just kappa-registry`, R30), stop it, adopt, start Hologram Registry on it, and run the existing provider conformance suite (`tests/registry_conformance.rs`) against **our** `/v2/`: green. A second adopt run changes nothing. Deleting `<into>/oci/` and the marker leaves a store `kappa-server` still opens.

**Traps:**
- The hub's blobs are blake3-primary; their sha256 twin exists only if the hard link worked (K7). Write the alias from the file that exists; hash once if there is none.
- The hub's manifests point `subject` at a blob (H4). Link it as a blob.
- Where `kappa-server` keeps its blob root and database under `KAPPA_STORE_ROOT` is not yet read from its source. Step 1 of the step-level plan reads `kappa-server`'s config code and records the two paths.

## Task 5: The provider moves to standard uploads

**Requirements:** FR-022. **Owner:** B (0.5 day).

`src/registry/kappa_client.rs` writes blobs with a direct `PUT /v2/<repo>/blobs/<digest>` (H3), which is not a registry route. Change `put_blob` (both the configured-namespace and the repository-scoped one) to `POST /v2/<repo>/blobs/uploads/?digest=<blake3:…>` with the body (route 11b). Reads stay as they are: `GET` and `HEAD` by `blake3:` digest work on our `/v2/` through the alias table (P1 T4).

**Done when:** `just kappa-registry` (the suite against a real `kappa-server`) still passes, **and** the same suite passes against Hologram Registry. If `kappa-server` does not accept a monolithic `POST` with a blake3 digest, keep both flows behind `registry.upload_flow = "standard" | "direct"` (default `standard`), and the hub's config says `direct` until the swap.

**Trap:** this change ships to the hub **before** the cutover (`plan.md` section 11). It must be safe on `kappa-server` first.

---

## Phase exit

Property test 256 of 256. Import idempotent. Verify 100 of 100. Adopt proven by the provider suite against our `/v2/`. Closes FR-010, FR-012, FR-014, FR-022, SC-008, SC-009.
