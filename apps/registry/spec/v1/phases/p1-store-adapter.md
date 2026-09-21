# P1. Store adapter

> Step level. Entry: a recorded P0 go. Engineer A. 6 days (days 3 to 8).

**Goal:** everything the registry needs from storage, behind one type, `OciStore`. No Kappa type leaves `src/oci_store/`. After this phase a test can push 2 GiB, kill the process, resume, finish, and read the blob back from the right repository only.

**Architecture:** `OciStore` owns an `Arc<PersistentStore>` (blobs, tags) and a `redb::Database` (`links.redb`: links, repos, referrers, aliases, uploads). All methods are synchronous; callers wrap them in `spawn_blocking` (P3, P4). The one async piece is `AsyncBlob`, a reader adapter for response bodies.

**Read first:** `data-model.md` (tables, layout, crash rules), `research.md` K1 to K15.

**Files in this phase** (each well under the 1500-line gate; tests at the bottom of each file do not count, R27):

| File | Holds | Expected lines |
|---|---|---|
| `src/oci_store/mod.rs` | `OciStore`, `open`, `OciStoreError`, re-exports | 250 |
| `src/oci_store/types.rs` | `Digest`, `RepoName`, `Tag`, `Reference`, `UploadId` | 300 |
| `src/oci_store/layout.rs` | directory layout, marker, reference-volume refusal | 200 |
| `src/oci_store/links.rs` | `links.redb` tables and transactions | 450 |
| `src/oci_store/blobs.rs` | `blob_stat`, `blob_open`, `AsyncBlob`, alias resolution | 300 |
| `src/oci_store/uploads.rs` | sessions, framing, finish, resume | 500 |
| `src/oci_store/manifests.rs` | manifest put/get/delete, tags, referrers | 450 |
| `tests/oci_store.rs` | cross-system integration tests | exempt |

**Error type, used by every task:**

```rust
/// Why a store operation did not happen. P3 maps each variant to one registry
/// error code; the mapping lives in `modules/oci/error.rs`, not here, so this
/// layer knows nothing about HTTP.
#[derive(Debug)]
pub enum OciStoreError {
    /// The digest, name, tag or upload id is not in the grammar.      -> DIGEST_INVALID / NAME_INVALID / TAG_INVALID / BLOB_UPLOAD_UNKNOWN
    Invalid { what: &'static str, value: String },
    /// Not linked in this repository (it may exist in another).       -> BLOB_UNKNOWN / MANIFEST_UNKNOWN
    NotInRepository { repo: String, digest: String },
    /// The repository has never been written to.                      -> NAME_UNKNOWN
    UnknownRepository(String),
    /// No such upload session.                                        -> BLOB_UPLOAD_UNKNOWN
    UnknownUpload(String),
    /// The bytes do not hash to the digest the client gave.           -> DIGEST_INVALID
    DigestMismatch { claimed: String },
    /// A chunk does not start where the last one ended.               -> RANGE_INVALID (416)
    OffsetMismatch { expected: u64, got: u64 },
    /// A manifest names content this repository does not hold.        -> MANIFEST_BLOB_UNKNOWN
    MissingReferences(Vec<String>),
    /// The volume is in another layout, or a newer one.               -> startup error, exit 2
    Layout(String),
    /// Another process holds the store.                               -> "the registry is running"
    Locked,
    /// Anything the disk or the store refused.                        -> UNKNOWN (500)
    Io(String),
}
```

---

## Task 1: Feature `oci` (off by default), layout, open

**Requirements:** FR-012 (refusal of a reference volume), ADR-029. **Files:** create `src/oci_store/mod.rs`, `src/oci_store/layout.rs`; modify `Cargo.toml`, `src/lib.rs`, `DEPENDENCIES.md`, `.github/workflows/ci.yml` (remove `registry-spike`); test `tests/oci_store.rs`.

**Interfaces (produces):**
- `OciStore::open(root: &Path, options: OpenOptions) -> Result<OciStore, OciStoreError>` where `OpenOptions { create: bool, upload_max_age: Duration }`
- `layout::Layout::resolve(root: &Path) -> Layout` with `blob_root()`, `kappa_db()`, `links_db()`, `marker()`
- `layout::check(root: &Path, create: bool) -> Result<(), OciStoreError>`

- [ ] **Step 1: Bring the pin from the spike.** From `registry/p0-spike` take only `third_party/kappa/`, `scripts/check-kappa-pin.sh` and the `Cargo.toml` lines. Keep the feature **off by default** (plan section 0, rule 1: a stock build of Hologram Live must not change), and make `redb` direct:

```toml
[dependencies]
kappa-core = { git = "https://github.com/<org>/kappa-registry", rev = "<pin rev>", optional = true }
kappa-store-redb = { git = "https://github.com/<org>/kappa-registry", rev = "<pin rev>", optional = true }
redb = { version = "4", optional = true }

[features]
default = []
oci = ["dep:kappa-core", "dep:kappa-store-redb", "dep:redb"]
bdd = []
```

Add to `justfile` `verify` prerequisites: `kappa-pin` (recipe: `./scripts/check-kappa-pin.sh`) and `oci-check` (recipe: `cargo check --locked --features oci`). The second keeps the registry build compiling on every commit to the parent, while the default build stays as it is. Every `cargo test` and `cargo build` command in this file and the later phase files runs with `--features oci`.

- [ ] **Step 2: Write the failing tests.** Create `tests/oci_store.rs`:

```rust
#![cfg(feature = "oci")]
//! Integration tests for the store adapter. Run on Linux, macOS and Windows.

use hologram_live::oci_store::{OciStore, OciStoreError, OpenOptions};
use std::time::Duration;

fn options() -> OpenOptions {
    OpenOptions { create: true, upload_max_age: Duration::from_secs(7 * 24 * 3600) }
}

#[test]
fn a_new_directory_gets_layout_version_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    drop(OciStore::open(dir.path(), options()).expect("open"));
    let marker = std::fs::read_to_string(dir.path().join("HOLOGRAM_REGISTRY_LAYOUT")).expect("marker");
    assert_eq!(marker.trim(), "1");
    assert!(dir.path().join("oci/links.redb").exists());
    assert!(dir.path().join("kappa/kappa.redb").exists());
}

#[test]
fn a_docker_registry_volume_is_refused_and_names_the_import_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("docker/registry/v2/repositories")).expect("mkdir");
    let error = OciStore::open(dir.path(), options()).expect_err("must refuse");
    let OciStoreError::Layout(message) = error else { panic!("wrong variant: {error:?}") };
    assert!(message.contains("hologram oci import"), "{message}");
    assert!(!dir.path().join("oci").exists(), "a refused volume is not touched");
}

#[test]
fn a_newer_layout_is_refused_by_number() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("HOLOGRAM_REGISTRY_LAYOUT"), "2\n").expect("write");
    let error = OciStore::open(dir.path(), options()).expect_err("must refuse");
    assert!(matches!(error, OciStoreError::Layout(m) if m.contains('2') && m.contains('1')));
}

#[test]
fn a_second_opener_gets_locked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _first = OciStore::open(dir.path(), options()).expect("open");
    assert!(matches!(OciStore::open(dir.path(), options()), Err(OciStoreError::Locked)));
}
```

- [ ] **Step 3: Run.** `cargo test --locked --features oci --test oci_store` → FAIL: module `oci_store` does not exist.

- [ ] **Step 4: Write `layout.rs` and `mod.rs`.** The hard parts:

```rust
// layout.rs
pub(crate) fn check(root: &Path, create: bool) -> Result<(), OciStoreError> {
    let layout = Layout::resolve(root);
    match std::fs::read_to_string(layout.marker()) {
        Ok(text) => match text.trim() {
            "1" => Ok(()),
            other => Err(OciStoreError::Layout(format!(
                "this volume has layout version {other}; this binary reads version 1"
            ))),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if root.join("docker/registry/v2").is_dir() {
                return Err(OciStoreError::Layout(format!(
                    "this volume is in the Docker Registry layout. Hologram Registry cannot open \
                     it in place. Run: hologram oci import {} --into <new directory>",
                    root.display()
                )));
            }
            if !create {
                return Err(OciStoreError::Layout(format!("{} is not a registry volume", root.display())));
            }
            Ok(()) // `open` creates directories and both databases, then writes the marker LAST.
        }
        Err(error) => Err(OciStoreError::Io(format!("read {}: {error}", layout.marker().display()))),
    }
}
```

```rust
// mod.rs
pub struct OciStore {
    kappa: Arc<PersistentStore>,
    links: redb::Database,
    layout: Layout,
    sessions: std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<SessionState>>>>, // Task 5
    upload_max_age: Duration,
}

impl OciStore {
    pub fn open(root: &Path, options: OpenOptions) -> Result<Self, OciStoreError> {
        layout::check(root, options.create)?;
        let layout = Layout::resolve(root);
        layout.create_directories()?;
        layout.require_one_filesystem()?;               // data-model.md: rename must stay atomic
        // Ours first: it is the cheaper lock, and its error is unambiguous.
        let links = redb::Database::create(layout.links_db()).map_err(locked_or_io)?;
        let mut config = PersistentStoreConfig::new(layout.blob_root(), layout.kappa_db());
        config.upload_timeout_secs = None;              // expiry is ours (Task 6)
        let kappa = PersistentStore::new(config, Arc::new(WallClock)).map_err(locked_or_io)?;
        links::create_tables(&links)?;
        layout.write_marker_if_missing()?;
        // Task 6 adds: store.resume_uploads()?
        Ok(Self { kappa: Arc::new(kappa), links, layout, sessions: Mutex::default(), upload_max_age: options.upload_max_age })
    }
}

fn locked_or_io<E: std::fmt::Display>(error: E) -> OciStoreError {
    let text = error.to_string();
    // redb 4: DatabaseError::DatabaseAlreadyOpen. Kappa wraps it in StoreError::Io(String-ish).
    // P0 verdict section 5 records the exact text on each system; match on that.
    if text.contains("already open") || text.contains("locked") { OciStoreError::Locked } else { OciStoreError::Io(text) }
}
```

- [ ] **Step 5: Run.** `cargo test --locked --features oci --test oci_store` → PASS (4 tests).
- [ ] **Step 6: Gate.** Add the `DEPENDENCIES.md` rows (`kappa-core`, `kappa-store-redb`, `redb`), replace its last paragraph with a pointer to ADR-025, fix "pure Rust" (R24). `just verify`. Commit: `feat(oci): store adapter skeleton, layout version 1`.

**Done when:** the four tests pass and `just verify` is green with and without default features.

**Traps:**
- Write the marker last. A crash between creating the databases and writing the marker leaves a directory that `check` sees as "new and empty enough": `create_directories` and both `create` calls must be idempotent.
- String matching on an error is ugly. If P0 found that Kappa exposes the redb error as a typed variant, match on the type and delete `locked_or_io`'s text match.
- `AppState::build` must not open the store yet. That wiring is P3 T1, where the module exists to ask for it.

---

## Task 2: Validated types

**Requirements:** FR-004, FR-022 (blake3 accepted), ADR-031. **Files:** create `src/oci_store/types.rs`; test in the same file.

**Interfaces (produces):**
- `Digest::parse(&str) -> Result<Digest, OciStoreError>`; `Digest::algorithm() -> Algorithm` (`Sha256`, `Sha512`, `Blake3`); `as_str()`; `Digest::sha256_of(&[u8]) -> Digest`
- `RepoName::parse`, `Tag::parse`, `UploadId::parse`, each `-> Result<Self, OciStoreError>` with `as_str()`
- `Reference::parse(&str) -> Result<Reference, OciStoreError>`: `Reference::Tag(Tag) | Reference::Digest(Digest)`. A string with `:` is a digest or an error, never a tag

- [ ] **Step 1: Write the failing tests.**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_names_follow_the_reference_grammar() {
        for good in ["a", "library/alpine", "team/tags/app", "a/blobs/b", "a__b", "a---b", "a.b_c-d/e0", "0/1"] {
            assert!(RepoName::parse(good).is_ok(), "{good} must be accepted");
        }
        let too_long = "a/".repeat(128); // 256 characters
        for bad in ["", "Foo", "a//b", "/a", "a/", "a___b", "a..b", "-a", "a-", "_catalog", "a b", too_long.as_str()] {
            assert!(RepoName::parse(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(RepoName::parse(&"a".repeat(255)).is_ok(), "255 is the longest legal name");
    }

    #[test]
    fn tags_follow_the_reference_grammar() {
        for good in ["latest", "v1.0.0", "_x", "A-b.c_d", "blake3_cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959"] {
            assert!(Tag::parse(good).is_ok(), "{good}");
        }
        let long = "a".repeat(129);
        for bad in ["", ".x", "-x", "a:b", "a/b", long.as_str()] {
            assert!(Tag::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn digests_are_lowercase_hex_of_the_right_length() {
        let hex = "cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959";
        assert!(Digest::parse(&format!("sha256:{hex}")).is_ok());
        assert!(Digest::parse(&format!("blake3:{hex}")).is_ok(), "FR-022: the hub's objects are blake3");
        assert!(Digest::parse(&format!("sha512:{}", hex.repeat(2))).is_ok());
        for bad in [format!("sha256:{}", hex.to_uppercase()), format!("sha256:{}", &hex[1..]), format!("md5:{hex}"),
                    format!("sha256-{hex}"), "sha256:".to_owned(), format!("sha256:{hex}/../x")] {
            assert!(Digest::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_reference_with_a_colon_is_never_a_tag() {
        assert!(matches!(Reference::parse("latest"), Ok(Reference::Tag(_))));
        assert!(matches!(Reference::parse(&format!("sha256:{}", "0".repeat(64))), Ok(Reference::Digest(_))));
        assert!(Reference::parse("sha256:short").is_err(), "a malformed digest is an error, not a tag named sha256:short");
    }

    #[test]
    fn an_upload_id_cannot_be_a_path() {
        assert!(UploadId::parse("6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41").is_ok());
        for bad in ["..", "../x", "a/b", "", "6f1c2a9e3b7d4c559f0a2d8e7b6a5c41"] {
            assert!(UploadId::parse(bad).is_err(), "{bad:?}");
        }
    }
}
```

- [ ] **Step 2: Run.** `cargo test --locked --features oci oci_store::types` → FAIL.
- [ ] **Step 3: Implement.** No regex crate (none is direct). The name grammar by hand:

```rust
impl RepoName {
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        let invalid = || OciStoreError::Invalid { what: "repository name", value: value.to_owned() };
        if value.is_empty() || value.len() > 255 { return Err(invalid()); }
        for component in value.split('/') {
            if !component_is_valid(component.as_bytes()) { return Err(invalid()); }
        }
        Ok(Self(value.to_owned()))
    }
}

/// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`
fn component_is_valid(bytes: &[u8]) -> bool {
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else { return false };
    if !alnum(*first) || !alnum(*last) { return false; }
    let mut index = 0;
    while index < bytes.len() {
        if alnum(bytes[index]) { index += 1; continue; }
        let start = index;
        while index < bytes.len() && !alnum(bytes[index]) { index += 1; }
        let separator = &bytes[start..index];
        let legal = separator == b"." || separator == b"_" || separator == b"__" || separator.iter().all(|b| *b == b'-');
        if !legal { return false; }
    }
    true
}
```

- [ ] **Step 4: Run** → PASS. **Step 5:** `just verify`; commit `feat(oci): validated name, tag, digest and upload id types`.

**Done when:** the five tests pass.

**Traps:**
- `sha512` support in the reference is unconfirmed (**B:`digest-sha512`**). The type accepts it; whether the store can address it is checked in Task 4. If the store cannot, `Digest::parse` refuses it and the contract file is corrected.
- The 255 limit applies to the whole name in the reference's source. The OCI text says less. Gate B scenario `name-length` decides; change one constant.

---

## Task 3: `links.redb`

**Requirements:** FR-009, ADR-027. **Files:** create `src/oci_store/links.rs`; tests in file.

**Interfaces (produces)**, all on `OciStore`, all one redb transaction each:
- `link_add(&self, repo: &RepoName, digest: &Digest, kind: LinkKind, media_type: Option<&str>) -> Result<(), OciStoreError>`; creates the `repos` row
- `link_get(&self, repo, digest) -> Result<Option<Link>, OciStoreError>`
- `link_remove(&self, repo, digest) -> Result<bool, OciStoreError>`; also removes `referrers` rows where it is the referrer
- `links_page(&self, repo, after: Option<&str>, limit: usize) -> Result<Vec<(Digest, Link)>, OciStoreError>`
- `repo_touch(&self, repo)`, `repo_exists(&self, repo) -> Result<bool, _>`, `repos_page(&self, after: Option<&str>, limit: usize) -> Result<Vec<RepoName>, _>`
- `referrer_add(&self, repo, subject: &Digest, referrer: &Digest, descriptor: &ReferrerDescriptor)`, `referrers_of(&self, repo, subject) -> Result<Vec<ReferrerDescriptor>, _>`
- `alias_put(&self, a: &Digest, b: &Digest)`, `alias_of(&self, digest: &Digest) -> Result<Option<Digest>, _>`
- `manifest_commit(&self, repo, digest, link: Link, referrer: Option<(Digest, ReferrerDescriptor)>) -> Result<(), _>`: the single transaction of `data-model.md` "Manifest PUT" step 2

Composite keys are one string, `"<repo>\u{0}<digest>"`. `\0` cannot appear in a repository name, and it sorts before every legal byte, so a prefix range `"<repo>\0".."<repo>\u{1}"` is exactly one repository.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn a_blob_linked_in_one_repository_is_absent_from_another() {
    let (store, _dir) = test_store();
    let (a, b, digest) = (repo("a/x"), repo("b/y"), sha("1"));
    store.link_add(&a, &digest, LinkKind::Blob, None).expect("link");
    assert!(store.link_get(&a, &digest).expect("get").is_some());
    assert!(store.link_get(&b, &digest).expect("get").is_none());
    store.link_add(&b, &digest, LinkKind::Blob, None).expect("mount");
    assert!(store.link_remove(&a, &digest).expect("remove"));
    assert!(store.link_get(&b, &digest).expect("get").is_some(), "b/y is untouched");
}

#[test]
fn a_repository_whose_name_is_a_prefix_of_another_does_not_see_its_links() {
    let (store, _dir) = test_store();
    store.link_add(&repo("team/app"), &sha("1"), LinkKind::Blob, None).expect("link");
    store.link_add(&repo("team/app2"), &sha("2"), LinkKind::Blob, None).expect("link");
    store.link_add(&repo("team/app/sub"), &sha("3"), LinkKind::Blob, None).expect("link");
    let seen: Vec<_> = store.links_page(&repo("team/app"), None, 10).expect("page");
    assert_eq!(seen.len(), 1);
}

#[test]
fn repositories_page_in_lexical_order() {
    let (store, _dir) = test_store();
    for name in ["b", "a/z", "a", "c"] { store.repo_touch(&repo(name)).expect("touch"); }
    let first = store.repos_page(None, 2).expect("page");
    assert_eq!(names(&first), ["a", "a/z"]);
    let second = store.repos_page(Some("a/z"), 2).expect("page");
    assert_eq!(names(&second), ["b", "c"]);
}

#[test]
fn aliases_resolve_both_ways() { /* alias_put(sha, blake); alias_of(sha)==blake; alias_of(blake)==sha */ }

#[test]
fn removing_a_referrer_manifest_removes_its_referrer_row() { /* referrer_add; link_remove(referrer); referrers_of(subject).is_empty() */ }

#[test]
fn one_hundred_thousand_links_page_in_under_50_ms() {
    // Sizes the choice in ADR-027. 100k link_add in ONE write transaction (a bulk helper
    // used by import and adopt), then time links_page(limit 1000) and link_get.
    // Fails over 50 ms per page or 1 ms per get on the CI runner.
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** Table definitions:

```rust
const LINKS: TableDefinition<&str, &[u8]> = TableDefinition::new("links");        // value: kind byte + media type
const REPOS: TableDefinition<&str, u64> = TableDefinition::new("repos");
const REFERRERS: TableDefinition<&str, &[u8]> = TableDefinition::new("referrers"); // key repo\0subject\0referrer, value JSON
const ALIASES: TableDefinition<&str, &str> = TableDefinition::new("aliases");
const UPLOADS: TableDefinition<&str, &[u8]> = TableDefinition::new("uploads");     // Task 5
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
```

Every write method: `let txn = self.links.begin_write()?; { open tables; mutate } txn.commit()?;`. redb commits are durable by default; do not lower durability.

- [ ] **Step 4: Run** → PASS. **Step 5:** write `specs/adrs/027-registry-links-owned-here.md` (decision, the measurement from the 100k test, the rejected option `meta_set`/`meta_query` and why: K13, and it cannot list repositories). `just verify`; commit.

**Done when:** the six tests pass, including the timing test on the CI runner.

**Traps:**
- redb allows one write transaction at a time. A slow writer blocks every other write. Keep blob I/O out of transactions: link rows are written after bytes are safe, never around them.
- `links_page` must use a range, not a scan with a filter. The prefix test exists because `starts_with("team/app")` is the obvious wrong implementation.

---

## Task 4: Blob reads, with alias resolution

**Requirements:** FR-002 (read), FR-022, ADR-030. **Files:** create `src/oci_store/blobs.rs`; tests in `tests/oci_store.rs`.

**Interfaces (produces):**
- `OciStore::blob_stat(&self, repo: &RepoName, digest: &Digest) -> Result<BlobStat, OciStoreError>` where `BlobStat { size: u64, stored_as: Digest }`. `NotInRepository` when there is no link, even if the bytes exist
- `OciStore::blob_open(&self, repo, digest) -> Result<(BlobStat, Box<dyn BlobRead>), OciStoreError>`; `trait BlobRead: Read + Seek + Send` (our own trait, blanket-implemented for the store's reader, so the Kappa trait does not leak)
- `AsyncBlob::new(reader: Box<dyn BlobRead>, start: u64, len: u64) -> AsyncBlob`, `impl tokio::io::AsyncRead`

- [ ] **Step 1: Failing tests** (in `tests/oci_store.rs`; `push_blob` is a helper that uses Task 5's API, so write these tests now and let them fail to compile until Task 5; that is the plan's order, A works on T4 and T5 together):

```rust
#[test]
fn a_blob_is_readable_only_through_a_repository_that_links_it() { /* push to a/x; blob_stat(b/y) is NotInRepository; blob_stat(a/x).size == n */ }

#[test]
fn a_blob_pushed_by_sha256_opens_by_its_blake3_alias_and_back() { /* push by sha256; compute blake3 of the same bytes; blob_open(repo, blake3) returns the same bytes and stored_as == sha256 */ }

#[tokio::test]
async fn a_range_is_served_without_reading_the_rest() {
    // 64 MiB blob. AsyncBlob::new(reader, 10 MiB, 1 MiB). read_to_end gives exactly 1 MiB equal to the source slice.
    // The reader is wrapped in a counting Read that asserts total bytes read <= 2 MiB.
}
```

- [ ] **Step 2: Implement.** The hard part is `AsyncBlob`: a synchronous `Read + Seek` driven from an async body without blocking the runtime and without `unsafe`.

```rust
/// Reads a synchronous blob from async code. Each poll hands the reader to the
/// blocking pool for one buffer, then takes it back. One read is in flight at a time.
pub struct AsyncBlob {
    state: State,
    remaining: u64,
}
enum State {
    Idle(Option<Box<dyn BlobRead>>),
    Reading(tokio::task::JoinHandle<(Box<dyn BlobRead>, std::io::Result<Vec<u8>>)>),
    Buffered { reader: Box<dyn BlobRead>, chunk: Vec<u8>, at: usize },
}
const CHUNK: usize = 1024 * 1024;

impl tokio::io::AsyncRead for AsyncBlob {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, out: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        loop {
            match &mut self.state {
                State::Buffered { chunk, at, .. } if *at < chunk.len() => {
                    let n = out.remaining().min(chunk.len() - *at);
                    out.put_slice(&chunk[*at..*at + n]);
                    *at += n;
                    return Poll::Ready(Ok(()));
                }
                State::Buffered { .. } => { /* move reader back to Idle */ }
                State::Idle(reader) => {
                    if self.remaining == 0 { return Poll::Ready(Ok(())); }
                    let mut reader = reader.take().expect("reader present while idle");
                    let want = usize::try_from(self.remaining.min(CHUNK as u64)).expect("fits");
                    self.state = State::Reading(tokio::task::spawn_blocking(move || {
                        let mut chunk = vec![0_u8; want];
                        let result = reader.read(&mut chunk).map(|n| { chunk.truncate(n); chunk });
                        (reader, result)
                    }));
                }
                State::Reading(handle) => match Pin::new(handle).poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(join)) => return Poll::Ready(Err(std::io::Error::other(join))),
                    Poll::Ready(Ok((reader, Err(error)))) => { self.state = State::Idle(Some(reader)); return Poll::Ready(Err(error)); }
                    Poll::Ready(Ok((reader, Ok(chunk)))) => {
                        if chunk.is_empty() { self.remaining = 0; self.state = State::Idle(Some(reader)); return Poll::Ready(Ok(())); }
                        self.remaining -= chunk.len() as u64;
                        self.state = State::Buffered { reader, chunk, at: 0 };
                    }
                },
            }
        }
    }
}
```

`AsyncBlob::new` seeks once, inside the first `spawn_blocking`. `Box<dyn BlobRead>` is `Unpin`, so no `unsafe` and no pin projection crate.

Resolution order in `blob_open`: link check first (`NotInRepository` beats everything), then `kappa.blob_open(digest)`; on `StoreError::NotFound`, `alias_of(digest)` and retry once.

- [ ] **Step 3: Run** (after Task 5) → PASS. **Step 4:** `just verify`; commit.

**Done when:** the three tests pass on three systems.

**Traps:**
- On Windows an open file cannot be renamed over or deleted. Garbage collection is offline, so no reader is open when it deletes. Keep it that way.
- Do not use `blob_get` or `blob_get_range`: they allocate the whole result (K9). The streaming gate script fails the build if they appear here.
- A blake3-primary blob in an adopted store may have no sha256 file if the hard link failed (K7). `alias_of` covers it. The adopt task (P8 T4) writes the alias.

---

## Task 5: Uploads: streaming in, hash on write, blake3 in the same pass

**Requirements:** FR-006, FR-020, ADR-030. **Files:** create `src/oci_store/uploads.rs`; tests in file and in `tests/oci_store.rs`.

**Interfaces (produces):**
- `upload_begin(&self, repo: &RepoName) -> Result<UploadId, OciStoreError>`
- `upload_append(&self, id: &UploadId, offset: u64, frame: &[u8]) -> Result<u64, OciStoreError>`: returns the new total. `OffsetMismatch` when `offset` is not the current total
- `upload_status(&self, id: &UploadId) -> Result<UploadStatus, OciStoreError>` where `UploadStatus { repo: RepoName, received: u64 }`
- `upload_finish(&self, id: &UploadId, claimed: &Digest) -> Result<Digest, OciStoreError>`: verifies, stores, links in `repo`, writes the alias, deletes the session
- `upload_cancel(&self, id: &UploadId) -> Result<(), OciStoreError>`
- `pub const FRAME: usize = 4 * 1024 * 1024;` and `Framer`, which turns arbitrary input slices into exact `FRAME` slices:

```rust
pub struct Framer { buffer: bytes::BytesMut }
impl Framer {
    /// Feed bytes as they arrive; get back zero or more full frames.
    pub fn push(&mut self, input: &[u8]) -> impl Iterator<Item = bytes::Bytes> + '_;
    /// The short last frame, if any.
    pub fn finish(self) -> Option<bytes::Bytes>;
}
```

(`bytes` becomes a direct dependency; it is locked already.)

- [ ] **Step 1: Failing tests.**

```rust
#[test]
fn a_wrong_digest_leaves_nothing() {
    let (store, _dir) = test_store();
    let id = store.upload_begin(&repo("a/x")).expect("begin");
    store.upload_append(&id, 0, b"hello").expect("append");
    let wrong = sha("0");
    assert!(matches!(store.upload_finish(&id, &wrong), Err(OciStoreError::DigestMismatch { .. })));
    assert!(matches!(store.blob_stat(&repo("a/x"), &wrong), Err(OciStoreError::NotInRepository { .. })));
    let real = Digest::sha256_of(b"hello");
    assert!(matches!(store.blob_stat(&repo("a/x"), &real), Err(OciStoreError::NotInRepository { .. })), "FR-020: nothing reachable is left");
    assert!(matches!(store.upload_status(&id), Err(OciStoreError::UnknownUpload(_))), "the session is gone, as in the reference: B:digest-mismatch");
}

#[test]
fn a_gap_or_an_overlap_is_refused_with_the_expected_offset() {
    let (store, _dir) = test_store();
    let id = store.upload_begin(&repo("a/x")).expect("begin");
    store.upload_append(&id, 0, b"abc").expect("append");
    assert!(matches!(store.upload_append(&id, 5, b"x"), Err(OciStoreError::OffsetMismatch { expected: 3, got: 5 })));
    assert!(matches!(store.upload_append(&id, 1, b"x"), Err(OciStoreError::OffsetMismatch { expected: 3, got: 1 })));
}

#[test]
fn a_zero_length_blob_is_a_blob() { /* begin, finish with sha256 of b"" ; blob_stat.size == 0 */ }

#[test]
fn the_same_blob_pushed_twice_at_once_ends_as_one_file_and_two_links() {
    // two threads, two sessions, same 8 MiB content, repos a/x and b/y; both finish Ok;
    // both blob_stat Ok; the blob directory holds exactly one file of that size.
}

#[test]
fn the_framer_emits_exact_frames_whatever_the_input_sizes() {
    let mut framer = Framer::default();
    let mut frames = Vec::new();
    for size in [1, 16 * 1024, FRAME - 1, FRAME, FRAME + 1, 3] {
        frames.extend(framer.push(&vec![7_u8; size]).map(|f| f.len()));
    }
    let tail = framer.finish().map(|f| f.len());
    assert!(frames.iter().all(|len| *len == FRAME));
    let total: usize = frames.iter().sum::<usize>() + tail.unwrap_or(0);
    assert_eq!(total, 1 + 16 * 1024 + (FRAME - 1) + FRAME + (FRAME + 1) + 3);
}

#[test]
#[ignore = "2 GiB; run by the oci-store CI job"]
fn two_gib_stay_under_200_mb() {
    // Stream 2 GiB through upload_append in FRAME slices while a sampler thread reads this
    // process's RSS every 100 ms (Linux: /proc/self/statm; macOS and Windows: sysinfo is NOT a
    // dependency, so the test is Linux-only and cfg-gated). Fail over 200 MB.
}
```

- [ ] **Step 2: Implement.** Session state and the finish sequence:

```rust
struct SessionState {
    repo: RepoName,
    kappa_id: String,
    received: u64,
    blake3: Option<blake3::Hasher>,   // None after a restart: alias is computed later
    touched_ms: u64,
}

pub fn upload_finish(&self, id: &UploadId, claimed: &Digest) -> Result<Digest, OciStoreError> {
    let session = self.take_session(id)?;                       // removes it from the map: a second finish is UnknownUpload
    let state = session.lock().expect("session lock");
    // 1. The store re-reads the staging file, hashes it, and refuses a mismatch (K4). Nothing is visible before this returns Ok.
    let result = self.kappa.upload_complete(&state.kappa_id, Some(claimed.as_str()));
    let stored = match result {
        Ok(result) => Digest::parse(&result.kappa)?,
        Err(StoreError::Rejected(_)) => { self.delete_upload_row(id)?; return Err(OciStoreError::DigestMismatch { claimed: claimed.as_str().to_owned() }); }
        Err(other) => return Err(OciStoreError::Io(other.to_string())),
    };
    // 2. Link, alias and drop the session row in ONE links.redb transaction (data-model.md, crash table row 4 to 5).
    let alias = state.blake3.as_ref().map(|hasher| Digest::from_blake3(hasher.finalize()));
    self.commit_finished_upload(id, &state.repo, &stored, alias.as_ref())?;
    // 3. No hasher (restart happened): compute the alias off the request path.
    if alias.is_none() { self.queue_alias(stored.clone()); }
    Ok(stored)
}
```

`upload_append` holds the per-session mutex for the whole call, so two `PATCH`es on one session are ordered and the loser sees `OffsetMismatch` (concurrency table). It updates `touched_ms` in memory and writes the `uploads` row at most once per 5 s, so a 20 GB push is about 100 row writes, not 5,120.

Public upload id: use the Kappa id if `UploadId::parse` accepts it (expected: `kappa-store-redb` depends on `uuid`). If P0 showed otherwise, make `uuid` a direct dependency (it is locked) and keep a `uuid → kappa_id` map in the row.

- [ ] **Step 3: Run** → PASS, including Task 4's tests. **Step 4:** add `scripts/check-oci-streaming.sh` to `just verify`:

```bash
#!/usr/bin/env bash
set -euo pipefail
# No layer in memory, and no Kappa type outside the adapter.
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd); fail=0
if grep -rnE 'blob_get\(|blob_get_range\(|blob_get_verified\(|to_bytes\(' "${root}/src/oci_store" "${root}/src/modules/oci" 2>/dev/null | grep -v '/manifests.rs:'; then
  echo 'a whole-buffer call outside manifests.rs' >&2; fail=1; fi
if grep -rnE 'kappa_core|kappa_store_redb' "${root}/src" | grep -v '^'"${root}"'/src/oci_store/'; then
  echo 'a Kappa type outside src/oci_store/' >&2; fail=1; fi
exit "${fail}"
```

- [ ] **Step 5:** write `specs/adrs/030-blake3-beside-sha256.md`. `just verify`; commit.

**Done when:** six tests pass; `two_gib_stay_under_200_mb` passes in the CI job; the streaming gate passes.

**Traps:**
- hyper hands the body over in pieces of about 16 KiB. Passing them straight to the store costs a file open and three checksums each (K3). Always go through `Framer`.
- `upload_complete` takes as long as one full read of the blob. It runs in `spawn_blocking` from P4, with no HTTP timeout on that route.
- Hashing blake3 costs CPU per byte. If P9 T5 shows push speed outside 20% of the reference, make the hasher optional by config and compute aliases lazily; the design already handles a missing hasher.

---

## Task 6: Sessions that survive a restart (carried patch 0003)

**Requirements:** FR-006, FR-019. **Files:** the Kappa fork (`crates/kappa-core/src/store/mod.rs`, `crates/kappa-store-redb/src/lib.rs`); `third_party/kappa/patches/0003-durable-upload-sessions.patch`; `src/oci_store/uploads.rs`; `tests/oci_store.rs`; `tests/support/oci_child.rs` (a tiny binary target for the kill test).

**Interfaces:** consumes (from the patch) `PersistentStoreConfig::preserve_staging: bool` and `KappaStore::upload_resume(&self, upload_id: &str, namespace: &NamespaceRef, max_size: u64) -> Result<u64, StoreError>`. Produces `OciStore::resume_uploads(&self) -> Result<ResumeReport, OciStoreError>` (`resumed`, `dropped_rows`, `dropped_files`) and `OciStore::purge_expired_uploads(&self, now_ms: u64) -> Result<usize, _>`.

- [ ] **Step 1: The failing test, a real kill.**

```rust
#[test]
fn an_upload_resumes_after_the_process_is_killed() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The child opens the store, begins an upload, appends 3 frames of known bytes,
    // prints "<upload id>\n" and then sleeps forever.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_oci_child"))
        .arg(dir.path()).stdout(std::process::Stdio::piped()).spawn().expect("spawn");
    let id = read_line(child.stdout.as_mut().expect("stdout"));
    child.kill().expect("kill");            // SIGKILL on Unix, TerminateProcess on Windows
    child.wait().expect("wait");

    let store = OciStore::open(dir.path(), options()).expect("reopen");
    let id = UploadId::parse(id.trim()).expect("id");
    let status = store.upload_status(&id).expect("the session survived");
    assert_eq!(status.received, 3 * FRAME as u64);
    store.upload_append(&id, status.received, &known_frame(3)).expect("append the rest");
    let digest = sha_of_known_frames(4);
    assert_eq!(store.upload_finish(&id, &digest).expect("finish"), digest);
}

#[test]
fn a_staging_file_with_no_row_and_a_row_with_no_file_are_both_cleaned() { /* plant each; open; ResumeReport counts 1 and 1; neither remains */ }

#[test]
fn an_upload_untouched_for_longer_than_the_purge_age_is_aborted() { /* upload_max_age = 1 s; purge_expired_uploads(now + 2 s) == 1; status is UnknownUpload */ }
```

- [ ] **Step 2: Write the patch in the fork.** On branch `hologram-registry-pin`:
  1. `PersistentStoreConfig` gains `pub preserve_staging: bool`, `false` in `new`. In `PersistentStore::new`, the staging cleanup loop (lib.rs:204-215) runs only when it is `false`.
  2. `KappaStore` gains `fn upload_resume(&self, upload_id: &str, namespace: &NamespaceRef, max_size: u64) -> Result<u64, StoreError> { let _ = (upload_id, namespace, max_size); Err(StoreError::Rejected("upload_resume is not supported by this store".into())) }`.
  3. `PersistentStore` implements it: validate the id has no path separator; `staging_root.join(id)` must exist; its length becomes `offset`; insert a `DiskUploadSession` with fresh MD5 and CRC state and an empty `part_digests`. Return the length.
  4. A unit test beside the existing upload tests: begin, put 2 parts, drop the store, reopen with `preserve_staging`, `upload_resume` returns the byte count, `upload_put_part` at that offset works, `upload_complete` verifies.
  5. `git format-patch -1 -o …/third_party/kappa/patches/`, push, move the `rev` in `Cargo.toml`, `cargo update -p kappa-core -p kappa-store-redb`, update the README table.
- [ ] **Step 3: Use it.** In `OciStore::open`: set `config.preserve_staging = true`; after both databases are open call `resume_uploads`. In `resume_uploads`: for each `uploads` row, `namespace_resolve_or_create(repo)` then `upload_resume`; on error delete the row. Then list `kappa/staging/`; delete files with no row.
- [ ] **Step 4: Run** → PASS on three systems. **Step 5:** open the upstream pull request the same day; put its link in the README; `just verify`; commit.

**Done when:** the kill test passes on Linux, macOS and Windows.

**Traps:**
- After a kill the staging file may hold a torn last frame. That is fine: its length is the truth, the client continues from it, and finish verifies the whole (`data-model.md`). Do not try to truncate to a frame boundary; the client did not send frames.
- `uploads` rows are written at most every 5 s (Task 5). After a kill the row's `touched_ms` may be stale, never the offset: the offset always comes from the file.
- `CARGO_BIN_EXE_oci_child` needs a `[[bin]]` entry. Put it behind `required-features = ["oci"]` and name it so it is not shipped: `test = false`, `doc = false`, and exclude it from the release workflow's copy step (it only copies `hologram`).

---

## Task 7: Manifests, tags, referrers

**Requirements:** FR-005 (storage half), FR-009, FR-020. **Files:** create `src/oci_store/manifests.rs`; tests in file.

**Interfaces (produces):**
- `manifest_put(&self, repo: &RepoName, reference: &Reference, media_type: &str, bytes: &[u8], plan: &ManifestPlan) -> Result<Digest, OciStoreError>` where `ManifestPlan { kind: LinkKind, must_exist: Vec<Digest>, subject: Option<(Digest, ReferrerDescriptor)> }` is computed by P4's validator. This layer checks `must_exist` against `links` and returns `MissingReferences`
- `manifest_get(&self, repo, reference) -> Result<StoredManifest, OciStoreError>` where `StoredManifest { digest: Digest, media_type: String, bytes: Vec<u8> }`
- `manifest_delete(&self, repo, digest: &Digest) -> Result<(), OciStoreError>`; `tag_delete(&self, repo, tag: &Tag)`
- `tags_page(&self, repo, after: Option<&str>, limit: usize) -> Result<Vec<Tag>, OciStoreError>`

- [ ] **Step 1: Failing tests:** put by tag then get by tag and by digest give the same bytes and media type; put by digest with a body that hashes differently is `DigestMismatch`; a manifest whose layer is linked only in another repository is `MissingReferences([that digest])`; moving a tag keeps the old digest reachable by digest; `tags_page` is lexical and resumes after `after`; deleting a manifest removes every tag that points at it (**B:`delete-manifest-tags`** confirms the reference does this); a get by tag during a concurrent tag move returns one whole manifest, old or new (spawn a mover thread, 1,000 reads, each body must hash to its reported digest).
- [ ] **Step 2: Implement** in the order of `data-model.md` "Manifest PUT": `ingest_verified(digest, bytes)` (whole buffer, the one allowed place, K9), then `manifest_commit` (Task 3), then `tag_set`. `tags_page` uses `tag_list` and sorts and slices in memory: the store has no ordered range over tags (K12). Note the cost in the doc comment: a repository with 100,000 tags pays one full list per page. P7 T1 measures it; if over 50 ms, mirror tag names into a `tags` table in `links.redb` within `manifest_commit`.
- [ ] **Step 3: Run** → PASS. **Step 4:** `just verify`; commit.

**Done when:** the seven tests pass.

**Traps:**
- `tag_get` by name and the read of the manifest are two calls. Resolve the tag to a digest once, then serve that digest; never re-resolve inside one request.
- The hub's manifests have a `subject` that points at a **blob** (H4). `subject` is never validated to be a manifest, here or in P4.

---

## Task 8: Upstream housekeeping, ADRs, three-system CI

**Requirements:** FR-019. **Files:** `third_party/kappa/README.md`; `specs/adrs/025-kappa-crates-in-the-dependency-graph.md`, `029-registry-on-disk-layout.md`, `031-blake3-digests-on-v2.md`; create `.github/workflows/registry-os.yml`; modify `.github/workflows/ci.yml` (remove `KAPPA_PIN_ALLOW_UNOPENED`).

- [ ] **Step 1:** Open upstream pull requests for 0001, 0002, 0003, and `0004-fsync-before-rename` (about 10 lines: `sync_all` on the staging file before `rename` in both branches of `upload_complete`, lib.rs:886 and :964; and in `ingest_*`). Open an issue asking for a LICENSE file. Put each link in the README table.
- [ ] **Step 2:** Move the fork from the engineer's account to the organisation the maintainer named (decision 3). Update `Cargo.toml`, README, `cargo update -p kappa-core -p kappa-store-redb`.
- [ ] **Step 3:** Write to the UOR Foundation asking for merge rights or agreement on the maintained fork. Record the date sent in the README. The answer is required before `server-v1.0.0` (FR-019), not before code.
- [ ] **Step 4:** `registry-os.yml`:

```yaml
name: registry-os
on:
  push: { branches: [main, 'registry/**'] }
  pull_request:
jobs:
  oci-store:
    strategy:
      fail-fast: false
      matrix:
        os: [macos-14, windows-2022]      # Linux runs in ci.yml
    runs-on: ${{ matrix.os }}
    timeout-minutes: 45
    env: { CARGO_INCREMENTAL: "0", CARGO_NET_GIT_FETCH_WITH_CLI: "true" }
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.97.1
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --locked --features oci --test oci_store
      - run: cargo test --locked --features oci --lib oci_store
```

In `ci.yml` add one step after the existing tests: `cargo test --features oci --release --locked --test oci_store -- --ignored two_gib_stay_under_200_mb`.

- [ ] **Step 5:** `./scripts/check-kappa-pin.sh` without the allow variable → PASS. `just verify`; commit.

**Done when:** `check-kappa-pin.sh` passes with no allowance, and `registry-os` is green on the systems the verdict commits to. From this commit both block merges.

**Traps:**
- The matrix lists only the systems the P0 verdict committed to. A system the verdict dropped is removed here, with a line in `apps/registry/DIFFERENCES.md`.
- Do not wait for upstream. The pull requests are opened; the patches are carried; work goes on.

---

## Phase exit

`cargo test --locked --features oci --test oci_store` green on three systems, including `an_upload_resumes_after_the_process_is_killed`; the 2 GiB memory test under 200 MB on Linux; `check-kappa-pin.sh` and `check-oci-streaming.sh` in `just verify`. Closes FR-019, FR-020. Unblocks P3 and P8.
