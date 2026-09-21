# Data model: what is on disk, and why a crash is safe

Facts cited as R, H, K are in `research.md`.

## Who owns what

| Data | Owner | Why |
|---|---|---|
| Blob bytes, addressed by hash | Kappa store (`kappa-store-redb`) | Settled: Kappa is the store |
| Tags, per repository | Kappa store (`tag_set`, `tag_get`, `tag_list`; a repository is a Kappa namespace, K12) | The hub's existing tags already live there (H5) |
| Repository links, referrers, hash aliases, upload session records, layout version | **Hologram Registry**, in its own redb file | The store has no repository scoping (brief), its metadata calls are one transaction each (K13), and its garbage collection must never be called. One file of ours gives multi-row transactions and a mark set that garbage collection reads in one pass. Recorded as ADR-003 |

`redb` is already in the graph through `kappa-store-redb` (K14); using it directly adds a row to `DEPENDENCIES.md`, not a crate.

## Directory layout, version 1

`<root>` is `storage.filesystem.rootdirectory`, default `/var/lib/registry`.

```
<root>/HOLOGRAM_REGISTRY_LAYOUT      one line: "1"
<root>/kappa/blobs/…                 Kappa blob_root (paths from kappa_core::kappa::blob_path_for)
<root>/kappa/staging/<upload id>     Kappa staging = blob_root.parent()/staging (K5)
<root>/kappa/kappa.redb              Kappa database
<root>/oci/links.redb                ours
<root>/live/{state,cache,config}     Hologram Live's own directories in registry mode (R17, R18)
```

Start-up rules, in order:
1. `<root>/docker/registry/v2` exists and the marker does not: refuse. Message: `this volume is in the Docker Registry layout. Hologram Registry cannot open it in place. Run: hologram oci import <root> --into <new dir>`. (Spec, Story 3 scenario 4.)
2. Marker missing, directory empty or absent: create the layout, write the marker last.
3. Marker says `1`: open.
4. Marker says anything else: refuse, name the version found and the versions this binary reads.

Staging, blobs and both databases sit on one filesystem, so the rename in `upload_complete` (K4) stays atomic. A volume that splits them is refused at start: `std::fs::metadata` device ids are compared on Unix; on Windows a test rename in `kappa/` decides.

## `links.redb` tables

| Table | Key | Value | Used by |
|---|---|---|---|
| `links` | `(repo: &str, digest: &str)` | `kind: u8` (1 layer or config, 2 manifest, 3 index) + `media_type` for manifests | every scoped read; mount; `MANIFEST_BLOB_UNKNOWN`; garbage collection mark |
| `repos` | `repo: &str` | `created_ms: u64` | `_catalog`; `NAME_UNKNOWN`. A row exists from the first upload `POST`, as in the reference: **B:`catalog-after-failed-push`** |
| `referrers` | `(repo, subject_digest, referrer_digest)` | `artifact_type`, `size`, `annotations` (JSON) | route R; mark rule |
| `aliases` | `digest_a: &str` | `digest_b: &str`, both directions stored | ADR-006: blake3 beside sha256 without hard links |
| `uploads` | `uuid: &str` | `repo`, `kappa_upload_id`, `created_ms`, `touched_ms`, `blake3_state: Vec<u8>` (may be empty) | sessions across restart |
| `meta` | `"layout"` | `1` | cross-check with the marker file |

Keys are strings because redb orders `&str` byte-wise, which is the lexical order routes 2 and 3 must page in.

## blake3 beside sha256 (ADR-006)

The store does not make a blake3 address for a sha256 upload (K6), and it throws hard link errors away (K7). So the registry does not depend on hard links at all.

- While a blob streams in, the adapter feeds each frame to a `blake3::Hasher` as well. Cost: one more pass in memory over bytes already there, no second read.
- At finish, `aliases` gets both directions: `sha256:… → blake3:…` and back.
- To open a digest: try the store by that digest; on not found, look up `aliases` and try the other one. That covers blake3-primary blobs already in the hub's store (H5) and sha256-primary blobs from Docker clients, with or without hard links.
- A restart mid-upload loses the running blake3 state (the hasher cannot be serialised by the `blake3` crate). Then the alias is computed after finish by one read of the blob, in a background task, and the upload still succeeds. `blake3_state` stays empty in v1; the column exists so a later version can fill it.
- `/v2/` responses name the digest the client asked by. `Docker-Content-Digest` on a blob fetched by blake3 is that blake3 digest. A sha256 client never sees blake3.

## Upload sessions across a restart (FR-006)

Today a restart loses every session and deletes every staging file (K5). Carried patch `0003-durable-upload-sessions`, opened upstream the same day:

1. `PersistentStoreConfig` gains `preserve_staging: bool` (default `false`, so upstream behaviour is unchanged). When true, open does not delete staging files.
2. New trait method with a default that errors: `upload_resume(&self, upload_id: &str, namespace: &NamespaceRef, max_size: u64) -> Result<u64, StoreError>`. It rebuilds the in-memory session from the staging file's length and returns that length. S3 part digests are not rebuilt; the registry never reads them.

Ours: at start, for each row in `uploads`, call `upload_resume`. A row whose staging file is gone is deleted. A staging file with no row is deleted. Rows older than `storage.maintenance.uploadpurging.age` by `touched_ms` are aborted by a task that runs every `interval`.

Size of the patch: about 60 lines plus tests **[assumption]**, confirmed or corrected by P0 Task 2 step 6.

## Crash safety

"Kill" means `SIGKILL` or power loss. The second column is what a restart finds.

### Blob upload

| Killed | On disk | Why the restart is safe |
|---|---|---|
| After `POST`, before any `PATCH` | `uploads` row, empty staging file | Session resumes at offset 0. Expires if never touched |
| During a `PATCH` | Staging file holds a prefix, possibly with a torn last frame. `uploads` row | `upload_resume` reports the file's length; the client's next `PATCH` or status `GET` learns the true offset. Bytes are verified at finish, so a torn frame cannot become a blob: the digest will not match and the client restarts the layer |
| In `upload_complete`, before the rename | As above | As above. The client's `PUT` failed; it retries |
| After the rename, before our link | Blob file in place, no link, `uploads` row pointing at a finished Kappa session | Blob is unreachable through `/v2/` (scoped reads need a link). The `uploads` row fails to resume and is deleted. The client's retry re-uploads; the store sees the path exists and keeps one copy. Garbage collection would sweep the orphan |
| After the link | Done | The `uploads` row delete is idempotent |

Blob data is not synced before the rename (K4). After power loss a file can exist with the right name and wrong bytes. Carried patch `0004-fsync-before-rename` (about 10 lines) fixes it; until it lands in our pin, `hologram oci verify` is the net. P0 lists it as a required upstream fix.

### Manifest `PUT`

Writes happen in this order, each durable before the next:

1. manifest bytes into the store (`ingest_verified`, K9; at most 4 MiB);
2. one `links.redb` transaction: `links` row, `referrers` row if `subject`, `repos` row;
3. `tag_set` in Kappa, when the reference is a tag.

| Killed after | State | Safe because |
|---|---|---|
| 1 | Unlinked bytes | Unreachable; swept by garbage collection |
| 2 | Manifest reachable by digest, tag not moved | A legal registry state: it is what a push by digest leaves. The client saw no 201 and retries; every step is idempotent |
| 3 | Done | |

No order of kills leaves a tag pointing at a manifest that is not linked, or a link pointing at bytes that are not stored.

### Delete

Delete removes rows in one `links.redb` transaction and, for a tag, calls `tag_delete`. It never touches blob bytes. Only garbage collection removes bytes, and it cannot run beside the server.

## Garbage collection: the mark rule (FR-010, SC-008)

Offline. Opens both databases, which fails fast if the server holds them (K11); that failure is reported as "the registry is running; stop it first" and exit code 1.

Mark, for **every** repository in `repos`:
1. every digest a tag points at (`tag_list`);
2. without `--delete-untagged`: also every `links` row of kind manifest or index;
3. for each marked manifest: its `config` and every `layers` digest; for each marked index: every child, recursively (depth cap 8; deeper is an error, nothing is swept);
4. every `referrers` row whose subject is marked: mark the referrer, then apply 3 to it. Repeat until no new marks;
5. both sides of every `aliases` pair of a marked digest;
6. the blob behind every live `uploads` row is staging, not a blob, and is never swept.

Sweep: `blob_list()` minus marks, `blob_delete` each; then delete `links`, `referrers`, `aliases` rows that name a swept digest, in one transaction per 1,000 rows. With `--delete-untagged`, `links` rows of unmarked manifests go first.

A manifest that cannot be parsed during mark stops the run before any sweep. Garbage collection never guesses.

`--dry-run` prints one line per blob it would delete. Line format: **B:`gc-dry-run-output`** records the reference's wording; scripts may parse it.

Property test (`tests/oci_gc_property.rs`): random sequences over three repositories sharing layers of `push`, `tag`, `untag`, `delete manifest`, `delete blob`, `attach referrer`, `collect`, `collect --delete-untagged`; after every `collect`, every tag in every repository must pull completely, and every referrer of a tagged manifest must be listed and pull. 256 cases per CI run, fixed seed list plus one random seed printed on failure.

## Adopting the hub's store (P10)

The hub's store has blobs and tags and no `links.redb` (H5). `hologram oci adopt --blob-root <dir> --db <file> --into <root>`:

1. refuse unless the server is stopped (same lock rule);
2. move or bind the two Kappa paths under `<root>/kappa/` (the hub's compose maps them; nothing is copied);
3. for each namespace (`namespace_list`), for each tag: read the manifest, write `repos`, `links` for the manifest, its `config`, `layers`, and its `subject` target (H4: the subject is the object's blob), and a `referrers` row;
4. for each blob linked: if it is blake3-primary and a sha256 hard link exists, write the `aliases` pair; if not, hash once and write it;
5. write the marker. Re-runnable: every write is an upsert.

Rollback is deleting `<root>/oci/` and the marker; the Kappa paths were never modified.
