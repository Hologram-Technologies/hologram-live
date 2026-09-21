//! `links.redb`: what the registry knows that the Kappa store does not.
//!
//! The Kappa blob store is global; the registry protocol is scoped. A blob or
//! manifest exists *in a repository* only if that repository holds a link to
//! it. Links, the repository list, referrers and hash aliases live here, in a
//! database of our own, so one transaction can cover several rows and garbage
//! collection can read its mark set in one pass (ADR 027). The Kappa store's
//! metadata calls cannot do this: each is its own transaction, keyed by one
//! namespace, and none can list repositories.
//!
//! Composite keys are one string, `"<repo>\0<digest>"`. A NUL cannot appear in
//! a repository name and sorts before every legal byte, so the range
//! `"<repo>\0".."<repo>\u{1}"` is exactly one repository: `team/app` does not
//! see `team/app2` or `team/app/sub`.

use super::{now_ms, Digest, OciStore, OciStoreError, RepoName, UploadId};
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::ops::Bound;

/// Key `repo\0digest`; value: kind byte, then the media type (manifests only).
const LINKS: TableDefinition<&str, &[u8]> = TableDefinition::new("links");
/// Key `repo`; value: creation time in milliseconds.
const REPOS: TableDefinition<&str, u64> = TableDefinition::new("repos");
/// Key `repo\0subject\0referrer`; value: a JSON `ReferrerDescriptor`.
const REFERRERS: TableDefinition<&str, &[u8]> = TableDefinition::new("referrers");
/// Both directions of a sha256 and blake3 pair for the same bytes.
const ALIASES: TableDefinition<&str, &str> = TableDefinition::new("aliases");
/// Upload session records, so a session survives a restart.
const UPLOADS: TableDefinition<&str, &[u8]> = TableDefinition::new("uploads");
/// `"layout"` → the layout version, cross-checked with the marker file.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

/// What a link points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A layer or a config.
    Blob,
    Manifest,
    Index,
}

impl LinkKind {
    const fn byte(self) -> u8 {
        match self {
            Self::Blob => 1,
            Self::Manifest => 2,
            Self::Index => 3,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Blob),
            2 => Some(Self::Manifest),
            3 => Some(Self::Index),
            _ => None,
        }
    }
}

/// One row of `links`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// The `Content-Type` a manifest was pushed with. `None` for blobs.
    pub media_type: Option<String>,
}

impl Link {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![self.kind.byte()];
        if let Some(media_type) = &self.media_type {
            bytes.extend_from_slice(media_type.as_bytes());
        }
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, OciStoreError> {
        let (kind, rest) = bytes
            .split_first()
            .ok_or_else(|| OciStoreError::Io("links.redb: empty link row".to_owned()))?;
        let kind = LinkKind::from_byte(*kind)
            .ok_or_else(|| OciStoreError::Io(format!("links.redb: unknown link kind {kind}")))?;
        let media_type = if rest.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(rest).into_owned())
        };
        Ok(Self { kind, media_type })
    }
}

/// What the referrers route returns for one manifest that names a subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferrerDescriptor {
    pub digest: String,
    pub media_type: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Map<String, serde_json::Value>>,
}

/// One row of `uploads`: what is needed to find a session again after a
/// restart. The running blake3 state cannot be saved, so it is not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UploadRow {
    pub repo: String,
    pub kappa_id: String,
    pub created_ms: u64,
    pub touched_ms: u64,
}

fn io(error: impl std::fmt::Display) -> OciStoreError {
    OciStoreError::Io(format!("links.redb: {error}"))
}

fn link_key(repo: &RepoName, digest: &Digest) -> String {
    format!("{}\0{}", repo.as_str(), digest.as_str())
}

fn referrer_key(repo: &RepoName, subject: &Digest, referrer: &Digest) -> String {
    format!(
        "{}\0{}\0{}",
        repo.as_str(),
        subject.as_str(),
        referrer.as_str()
    )
}

/// The half-open key range that holds exactly `prefix`'s rows.
fn prefix_range(prefix: &str) -> (String, String) {
    (format!("{prefix}\0"), format!("{prefix}\u{1}"))
}

pub(crate) fn create_tables(database: &redb::Database) -> Result<(), OciStoreError> {
    let txn = database.begin_write().map_err(io)?;
    {
        txn.open_table(LINKS).map_err(io)?;
        txn.open_table(REPOS).map_err(io)?;
        txn.open_table(REFERRERS).map_err(io)?;
        txn.open_table(ALIASES).map_err(io)?;
        txn.open_table(UPLOADS).map_err(io)?;
        let mut meta = txn.open_table(META).map_err(io)?;
        if meta.get("layout").map_err(io)?.is_none() {
            meta.insert("layout", 1).map_err(io)?;
        }
    }
    txn.commit().map_err(io)
}

impl OciStore {
    /// Link `digest` into `repo`, creating the repository row if it is new.
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn link_add(
        &self,
        repo: &RepoName,
        digest: &Digest,
        kind: LinkKind,
        media_type: Option<&str>,
    ) -> Result<(), OciStoreError> {
        self.links_add_bulk(std::iter::once((
            repo,
            digest,
            Link {
                kind,
                media_type: media_type.map(str::to_owned),
            },
        )))
    }

    /// Many links in one write transaction. Import and adopt use this: redb
    /// syncs on every commit, so one commit per link would be far too slow.
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write. Nothing is written then.
    pub fn links_add_bulk<'a>(
        &self,
        rows: impl IntoIterator<Item = (&'a RepoName, &'a Digest, Link)>,
    ) -> Result<(), OciStoreError> {
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut links = txn.open_table(LINKS).map_err(io)?;
            let mut repos = txn.open_table(REPOS).map_err(io)?;
            for (repo, digest, link) in rows {
                links
                    .insert(link_key(repo, digest).as_str(), link.encode().as_slice())
                    .map_err(io)?;
                if repos.get(repo.as_str()).map_err(io)?.is_none() {
                    repos.insert(repo.as_str(), now_ms()).map_err(io)?;
                }
            }
        }
        txn.commit().map_err(io)
    }

    /// # Errors
    ///
    /// `Io` when the database cannot be read or a row is malformed.
    pub fn link_get(
        &self,
        repo: &RepoName,
        digest: &Digest,
    ) -> Result<Option<Link>, OciStoreError> {
        let txn = self.links.begin_read().map_err(io)?;
        let links = txn.open_table(LINKS).map_err(io)?;
        let row = links.get(link_key(repo, digest).as_str()).map_err(io)?;
        row.map(|guard| Link::decode(guard.value())).transpose()
    }

    /// Remove a link. Also removes every referrers row in which `digest` is
    /// the referrer, so a deleted signature stops being listed. Returns
    /// whether the link existed. Blob bytes are never touched here.
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn link_remove(&self, repo: &RepoName, digest: &Digest) -> Result<bool, OciStoreError> {
        let txn = self.links.begin_write().map_err(io)?;
        let existed;
        {
            let mut links = txn.open_table(LINKS).map_err(io)?;
            existed = links
                .remove(link_key(repo, digest).as_str())
                .map_err(io)?
                .is_some();
            let mut referrers = txn.open_table(REFERRERS).map_err(io)?;
            let (start, end) = prefix_range(repo.as_str());
            let suffix = format!("\0{}", digest.as_str());
            let mut stale = Vec::new();
            for row in referrers
                .range::<&str>(start.as_str()..end.as_str())
                .map_err(io)?
            {
                let (key, _) = row.map_err(io)?;
                if key.value().ends_with(&suffix) {
                    stale.push(key.value().to_owned());
                }
            }
            for key in stale {
                referrers.remove(key.as_str()).map_err(io)?;
            }
        }
        txn.commit().map_err(io)?;
        Ok(existed)
    }

    /// One page of a repository's links, in digest order, after `after`.
    ///
    /// # Errors
    ///
    /// `Io` when the database cannot be read or a row is malformed.
    pub fn links_page(
        &self,
        repo: &RepoName,
        after: Option<&Digest>,
        limit: usize,
    ) -> Result<Vec<(Digest, Link)>, OciStoreError> {
        let (first, end) = prefix_range(repo.as_str());
        let start = match after {
            Some(digest) => Bound::Excluded(link_key(repo, digest)),
            None => Bound::Included(first),
        };
        let txn = self.links.begin_read().map_err(io)?;
        let links = txn.open_table(LINKS).map_err(io)?;
        let bounds = (
            match &start {
                Bound::Included(key) => Bound::Included(key.as_str()),
                Bound::Excluded(key) => Bound::Excluded(key.as_str()),
                Bound::Unbounded => Bound::Unbounded,
            },
            Bound::Excluded(end.as_str()),
        );
        let mut page = Vec::new();
        for row in links.range::<&str>(bounds).map_err(io)?.take(limit) {
            let (key, value) = row.map_err(io)?;
            let digest = key
                .value()
                .rsplit('\0')
                .next()
                .ok_or_else(|| io("link key without a digest"))
                .and_then(Digest::parse)?;
            page.push((digest, Link::decode(value.value())?));
        }
        Ok(page)
    }

    /// Record that a repository exists. The reference lists a repository from
    /// its first upload `POST`, before any blob is linked.
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn repo_touch(&self, repo: &RepoName) -> Result<(), OciStoreError> {
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut repos = txn.open_table(REPOS).map_err(io)?;
            if repos.get(repo.as_str()).map_err(io)?.is_none() {
                repos.insert(repo.as_str(), now_ms()).map_err(io)?;
            }
        }
        txn.commit().map_err(io)
    }

    /// # Errors
    ///
    /// `Io` when the database cannot be read.
    pub fn repo_exists(&self, repo: &RepoName) -> Result<bool, OciStoreError> {
        let txn = self.links.begin_read().map_err(io)?;
        let repos = txn.open_table(REPOS).map_err(io)?;
        Ok(repos.get(repo.as_str()).map_err(io)?.is_some())
    }

    /// One page of repository names in lexical order, after `after`.
    ///
    /// # Errors
    ///
    /// `Io` when the database cannot be read or a stored name is malformed.
    pub fn repos_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RepoName>, OciStoreError> {
        let txn = self.links.begin_read().map_err(io)?;
        let repos = txn.open_table(REPOS).map_err(io)?;
        let start = after.map_or(Bound::Unbounded, Bound::Excluded);
        let mut page = Vec::new();
        for row in repos
            .range::<&str>((start, Bound::Unbounded))
            .map_err(io)?
            .take(limit)
        {
            let (key, _) = row.map_err(io)?;
            page.push(RepoName::parse(key.value())?);
        }
        Ok(page)
    }

    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn referrer_add(
        &self,
        repo: &RepoName,
        subject: &Digest,
        referrer: &Digest,
        descriptor: &ReferrerDescriptor,
    ) -> Result<(), OciStoreError> {
        let value = serde_json::to_vec(descriptor).map_err(io)?;
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut referrers = txn.open_table(REFERRERS).map_err(io)?;
            referrers
                .insert(
                    referrer_key(repo, subject, referrer).as_str(),
                    value.as_slice(),
                )
                .map_err(io)?;
        }
        txn.commit().map_err(io)
    }

    /// Every manifest in `repo` that names `subject`, in digest order.
    ///
    /// # Errors
    ///
    /// `Io` when the database cannot be read or a row is malformed.
    pub fn referrers_of(
        &self,
        repo: &RepoName,
        subject: &Digest,
    ) -> Result<Vec<ReferrerDescriptor>, OciStoreError> {
        let (start, end) = prefix_range(&format!("{}\0{}", repo.as_str(), subject.as_str()));
        let txn = self.links.begin_read().map_err(io)?;
        let referrers = txn.open_table(REFERRERS).map_err(io)?;
        let mut found = Vec::new();
        for row in referrers
            .range::<&str>(start.as_str()..end.as_str())
            .map_err(io)?
        {
            let (_, value) = row.map_err(io)?;
            found.push(serde_json::from_slice(value.value()).map_err(io)?);
        }
        Ok(found)
    }

    /// Record that `a` and `b` name the same bytes. Both directions are kept.
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn alias_put(&self, a: &Digest, b: &Digest) -> Result<(), OciStoreError> {
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut aliases = txn.open_table(ALIASES).map_err(io)?;
            aliases.insert(a.as_str(), b.as_str()).map_err(io)?;
            aliases.insert(b.as_str(), a.as_str()).map_err(io)?;
        }
        txn.commit().map_err(io)
    }

    /// # Errors
    ///
    /// `Io` when the database cannot be read or a stored digest is malformed.
    pub fn alias_of(&self, digest: &Digest) -> Result<Option<Digest>, OciStoreError> {
        let txn = self.links.begin_read().map_err(io)?;
        let aliases = txn.open_table(ALIASES).map_err(io)?;
        let row = aliases.get(digest.as_str()).map_err(io)?;
        row.map(|guard| Digest::parse(guard.value())).transpose()
    }

    /// The single transaction of a manifest `PUT`: the link, the referrers row
    /// when the manifest names a subject, and the repository row. Either all
    /// of it is durable or none of it is (`data-model.md`, "Manifest PUT").
    ///
    /// # Errors
    ///
    /// `Io` when the database refuses the write.
    pub fn manifest_commit(
        &self,
        repo: &RepoName,
        digest: &Digest,
        link: &Link,
        referrer: Option<(&Digest, &ReferrerDescriptor)>,
    ) -> Result<(), OciStoreError> {
        let referrer_row = referrer
            .map(|(subject, descriptor)| {
                serde_json::to_vec(descriptor)
                    .map(|value| (referrer_key(repo, subject, digest), value))
                    .map_err(io)
            })
            .transpose()?;
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut links = txn.open_table(LINKS).map_err(io)?;
            links
                .insert(link_key(repo, digest).as_str(), link.encode().as_slice())
                .map_err(io)?;
            if let Some((key, value)) = &referrer_row {
                let mut referrers = txn.open_table(REFERRERS).map_err(io)?;
                referrers
                    .insert(key.as_str(), value.as_slice())
                    .map_err(io)?;
            }
            let mut repos = txn.open_table(REPOS).map_err(io)?;
            if repos.get(repo.as_str()).map_err(io)?.is_none() {
                repos.insert(repo.as_str(), now_ms()).map_err(io)?;
            }
        }
        txn.commit().map_err(io)
    }

    pub(crate) fn upload_row_put(
        &self,
        id: &UploadId,
        row: &UploadRow,
    ) -> Result<(), OciStoreError> {
        let value = serde_json::to_vec(row).map_err(io)?;
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut uploads = txn.open_table(UPLOADS).map_err(io)?;
            uploads.insert(id.as_str(), value.as_slice()).map_err(io)?;
        }
        txn.commit().map_err(io)
    }

    pub(crate) fn upload_row_delete(&self, id: &UploadId) -> Result<(), OciStoreError> {
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut uploads = txn.open_table(UPLOADS).map_err(io)?;
            uploads.remove(id.as_str()).map_err(io)?;
        }
        txn.commit().map_err(io)
    }

    /// Every recorded session, for resuming at start and for purging.
    pub(crate) fn upload_rows(&self) -> Result<Vec<(UploadId, UploadRow)>, OciStoreError> {
        let txn = self.links.begin_read().map_err(io)?;
        let uploads = txn.open_table(UPLOADS).map_err(io)?;
        let mut rows = Vec::new();
        for row in uploads.iter().map_err(io)? {
            let (key, value) = row.map_err(io)?;
            rows.push((
                UploadId::parse(key.value())?,
                serde_json::from_slice(value.value()).map_err(io)?,
            ));
        }
        Ok(rows)
    }

    /// The end of an upload, in one transaction: the blob is linked into its
    /// repository, its alias is recorded, and the session row goes. A crash
    /// before this leaves an unlinked, unreachable blob that garbage
    /// collection sweeps (`data-model.md`, crash table).
    pub(crate) fn commit_finished_upload(
        &self,
        id: &UploadId,
        repo: &RepoName,
        stored: &Digest,
        alias: Option<&Digest>,
    ) -> Result<(), OciStoreError> {
        let link = Link {
            kind: LinkKind::Blob,
            media_type: None,
        };
        let txn = self.links.begin_write().map_err(io)?;
        {
            let mut links = txn.open_table(LINKS).map_err(io)?;
            links
                .insert(link_key(repo, stored).as_str(), link.encode().as_slice())
                .map_err(io)?;
            let mut repos = txn.open_table(REPOS).map_err(io)?;
            if repos.get(repo.as_str()).map_err(io)?.is_none() {
                repos.insert(repo.as_str(), now_ms()).map_err(io)?;
            }
            if let Some(alias) = alias {
                let mut aliases = txn.open_table(ALIASES).map_err(io)?;
                aliases
                    .insert(stored.as_str(), alias.as_str())
                    .map_err(io)?;
                aliases
                    .insert(alias.as_str(), stored.as_str())
                    .map_err(io)?;
            }
            let mut uploads = txn.open_table(UPLOADS).map_err(io)?;
            uploads.remove(id.as_str()).map_err(io)?;
        }
        txn.commit().map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci_store::OpenOptions;
    use std::time::{Duration, Instant};

    fn test_store() -> (OciStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let options = OpenOptions {
            create: true,
            upload_max_age: Duration::from_hours(1),
        };
        let store = OciStore::open(dir.path(), options).expect("open");
        (store, dir)
    }

    fn repo(name: &str) -> RepoName {
        RepoName::parse(name).expect("repository name")
    }

    /// A sha256 digest whose hex is `seed`, left-padded with zeros.
    fn sha(seed: &str) -> Digest {
        Digest::parse(&format!("sha256:{seed:0>64}")).expect("digest")
    }

    fn descriptor(digest: &Digest) -> ReferrerDescriptor {
        ReferrerDescriptor {
            digest: digest.as_str().to_owned(),
            media_type: "application/vnd.oci.image.manifest.v1+json".to_owned(),
            size: 123,
            artifact_type: Some("application/vnd.example.signature".to_owned()),
            annotations: None,
        }
    }

    #[test]
    fn a_blob_linked_in_one_repository_is_absent_from_another() {
        let (store, _dir) = test_store();
        let (a, b, digest) = (repo("a/x"), repo("b/y"), sha("1"));
        store
            .link_add(&a, &digest, LinkKind::Blob, None)
            .expect("link");
        assert!(store.link_get(&a, &digest).expect("get").is_some());
        assert!(store.link_get(&b, &digest).expect("get").is_none());
        store
            .link_add(&b, &digest, LinkKind::Blob, None)
            .expect("mount");
        assert!(store.link_remove(&a, &digest).expect("remove"));
        assert!(
            store.link_get(&b, &digest).expect("get").is_some(),
            "b/y is untouched"
        );
        assert!(!store.link_remove(&a, &digest).expect("remove again"));
    }

    #[test]
    fn a_repository_whose_name_is_a_prefix_of_another_does_not_see_its_links() {
        let (store, _dir) = test_store();
        for (name, seed) in [("team/app", "1"), ("team/app2", "2"), ("team/app/sub", "3")] {
            store
                .link_add(&repo(name), &sha(seed), LinkKind::Blob, None)
                .expect("link");
        }
        let seen = store.links_page(&repo("team/app"), None, 10).expect("page");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, sha("1"));
    }

    #[test]
    fn links_page_continues_after_a_digest() {
        let (store, _dir) = test_store();
        let name = repo("a");
        for seed in ["1", "2", "3"] {
            store
                .link_add(&name, &sha(seed), LinkKind::Blob, None)
                .expect("link");
        }
        let first = store.links_page(&name, None, 2).expect("page");
        assert_eq!(first.len(), 2);
        let rest = store.links_page(&name, Some(&first[1].0), 2).expect("page");
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].0, sha("3"));
    }

    #[test]
    fn a_manifest_link_keeps_its_media_type() {
        let (store, _dir) = test_store();
        let media_type = "application/vnd.docker.distribution.manifest.v2+json";
        store
            .link_add(&repo("a"), &sha("1"), LinkKind::Manifest, Some(media_type))
            .expect("link");
        let link = store
            .link_get(&repo("a"), &sha("1"))
            .expect("get")
            .expect("row");
        assert_eq!(link.kind, LinkKind::Manifest);
        assert_eq!(link.media_type.as_deref(), Some(media_type));
    }

    #[test]
    fn repositories_page_in_lexical_order() {
        let (store, _dir) = test_store();
        for name in ["b", "a/z", "a", "c"] {
            store.repo_touch(&repo(name)).expect("touch");
        }
        let names = |page: &[RepoName]| -> Vec<String> {
            page.iter().map(|name| name.as_str().to_owned()).collect()
        };
        let first = store.repos_page(None, 2).expect("page");
        assert_eq!(names(&first), ["a", "a/z"]);
        let second = store.repos_page(Some("a/z"), 2).expect("page");
        assert_eq!(names(&second), ["b", "c"]);
        assert!(store.repo_exists(&repo("a/z")).expect("exists"));
        assert!(!store.repo_exists(&repo("zzz")).expect("exists"));
    }

    #[test]
    fn aliases_resolve_both_ways() {
        let (store, _dir) = test_store();
        let sha256 = sha("1");
        let blake3 = Digest::parse(&format!("blake3:{:0>64}", "2")).expect("digest");
        store.alias_put(&sha256, &blake3).expect("alias");
        assert_eq!(
            store.alias_of(&sha256).expect("alias"),
            Some(blake3.clone())
        );
        assert_eq!(store.alias_of(&blake3).expect("alias"), Some(sha256));
        assert_eq!(store.alias_of(&sha("9")).expect("alias"), None);
    }

    #[test]
    fn removing_a_referrer_manifest_removes_its_referrer_row() {
        let (store, _dir) = test_store();
        let (name, subject, signature) = (repo("a"), sha("1"), sha("2"));
        store
            .manifest_commit(
                &name,
                &signature,
                &Link {
                    kind: LinkKind::Manifest,
                    media_type: None,
                },
                Some((&subject, &descriptor(&signature))),
            )
            .expect("commit");
        assert_eq!(store.referrers_of(&name, &subject).expect("list").len(), 1);
        assert!(store
            .referrers_of(&repo("b"), &subject)
            .expect("list")
            .is_empty());
        assert!(store.link_remove(&name, &signature).expect("remove"));
        assert!(store
            .referrers_of(&name, &subject)
            .expect("list")
            .is_empty());
    }

    /// Sizes the choice in ADR 027: one repository with 100,000 links must
    /// still page and look up fast.
    #[test]
    fn one_hundred_thousand_links_page_in_under_50_ms() {
        let (store, _dir) = test_store();
        let name = repo("big/repo");
        let digests: Vec<Digest> = (0..100_000_u32).map(|n| sha(&format!("{n:x}"))).collect();
        let blob = || Link {
            kind: LinkKind::Blob,
            media_type: None,
        };
        store
            .links_add_bulk(digests.iter().map(|digest| (&name, digest, blob())))
            .expect("bulk");

        let started = Instant::now();
        let page = store.links_page(&name, None, 1000).expect("page");
        let paged = started.elapsed();
        assert_eq!(page.len(), 1000);

        let started = Instant::now();
        for digest in digests.iter().step_by(1000) {
            assert!(store.link_get(&name, digest).expect("get").is_some());
        }
        let per_get = started.elapsed() / 100;

        println!("ADR-027 measurement: page of 1000 in {paged:?}, link_get in {per_get:?}");
        assert!(paged < Duration::from_millis(50), "page took {paged:?}");
        assert!(per_get < Duration::from_millis(1), "get took {per_get:?}");
    }
}
