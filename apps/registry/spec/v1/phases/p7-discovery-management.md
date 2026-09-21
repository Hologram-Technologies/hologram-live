# P7. Discovery and management: tags, catalogue, referrers, delete

> Task level. **Steps to be written before the phase starts** (day 16).
> Entry: P4 exit. Engineer A. 3 days (days 17 to 20).

**Goal:** every remaining route. Exit: **gate A, every category, green and blocking.**

**Files:** `src/modules/oci/tags.rs` (200), `catalog.rs` (150), `referrers.rs` (250), `delete.rs` (200), `respond.rs` (+80 for `Link`).

---

## Task 1: Tags list

**Requirements:** FR-002. **Interfaces:** `GET …/tags/list?n=&last=`; `OciStore::tags_page` (P1 T7). Shared paging helper in `respond.rs`:

```rust
pub struct Page { pub n: usize, pub last: Option<String> }
pub fn parse_page(query: &str, max: usize) -> Result<Page, OciError>;      // PAGINATION_NUMBER_INVALID
pub fn link_next(path: &str, page: &Page, last_item: &str) -> HeaderValue; // </v2/…?last=<x>&n=<n>>; rel="next"
```

Edge behaviour comes from gate B scenario `tags-paging`, written in step 1: `n=0`, `n` negative, `n` not a number, `n` over `catalog.maxentries`, `last` that does not exist, `last` equal to the final tag, an empty repository, an unknown repository (`NAME_UNKNOWN`).

**Done when:** `tags-paging` shows zero differences; a repository with 100,000 tags pages in under 50 ms per page on the CI runner. If slower, mirror tag names into a `tags` table in `links.redb` inside `manifest_commit` (P1 T7 notes this), and re-measure.

**Trap:** order is byte-wise lexical, not natural. `v10` sorts before `v2`. The reference does the same; the scenario proves it.

## Task 2: Catalogue

**Requirements:** FR-002. **Interfaces:** `GET /v2/_catalog?n=&last=` over `OciStore::repos_page` (P1 T3); same helper.

**Done when:** scenarios `catalog-paging` and `catalog-after-failed-push` (does a repository appear after an upload that never finished?) show zero differences.

## Task 3: Referrers

**Requirements:** FR-002. **Interfaces:** `GET …/referrers/<digest>?artifactType=` → an OCI index built from `OciStore::referrers_of`; `OCI-Filters-Applied: artifactType` when filtered; an empty index, never 404, when there are none.

Scenario `referrers-present` answers whether the reference serves this route at all. If it does not (it answers 404 and clients fall back to the tag scheme `sha256-<hex>`), then: we still serve it (the conformance suite requires it for 1.1), it is row D-003 in `apps/registry/DIFFERENCES.md`, and `cosign.sh` runs both ways, which P2 T4 already does.

**Done when:** conformance referrers tests green; `oras.sh` `discover` lists the attached artifact; the scenario is recorded.

**Trap:** a referrer pushed **before** its subject exists must appear once the subject is pushed. Rows are keyed by subject digest, so it does, with no extra work; write the test anyway.

## Task 4: Delete

**Requirements:** FR-009. **Interfaces:** manifest delete by digest, blob delete, both 405 `UNSUPPORTED` unless `storage.delete.enabled`. Delete removes link rows and tags in one `links.redb` transaction plus `tag_delete` calls; it never touches bytes (`data-model.md`). Delete by tag: whatever scenario `delete-by-tag` records (the OCI text allows it; the reference has refused it).

Scenarios: `delete-disabled`, `delete-enabled`, `delete-by-tag`, `delete-manifest-tags`, `delete-shared-blob` (two repositories share a layer; delete in one; the other still pulls).

**Done when:** conformance content management category green; the five scenarios show zero unlisted differences; `tests/oci_concurrency.rs::delete_during_pull` passes.

## Task 5: Read-only mode and upload purging

**Requirements:** FR-011 (two supported keys that change behaviour). **Interfaces:** `storage.maintenance.readonly.enabled` → every write route answers as scenario `readonly-mode` records. `storage.maintenance.uploadpurging.*` → a task started from `OciRegistryModule::start` (R1) calls `OciStore::purge_expired_uploads` every `interval`; `dryrun` logs only.

**Done when:** `readonly-mode` shows zero differences; a test with `age = 1s`, `interval = 1s` sees an abandoned session disappear.

## Task 6: Gate A, all categories, blocking

**Requirements:** SC-002. **Files:** `gates.yml`. Set all four `OCI_TEST_*` switches on, remove `continue-on-error`. 

**Done when:** the job is green and required. From this commit a conformance failure blocks every merge.

---

## Phase exit

Gate A fully green, blocking, day 20. Closes FR-002, FR-009, SC-002. Unblocks garbage collection and adopt (they need the final link and referrer shapes).
