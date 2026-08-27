//! Canonical Docker archive encoding for Python rootfs layers.

use crate::error::{LiveError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use tar::{Archive, Builder, EntryType, Header};

const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct NormalizedDockerArchive {
    pub(crate) bytes: Vec<u8>,
    pub(crate) image_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DockerManifest {
    #[serde(rename = "Config")]
    config: String,
    #[serde(rename = "RepoTags")]
    repo_tags: Vec<String>,
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

pub(crate) fn normalize(reader: impl Read, expected_tag: &str) -> Result<NormalizedDockerArchive> {
    let files = read_regular_files(reader)?;
    let manifest_bytes = files
        .get(Path::new("manifest.json"))
        .ok_or_else(|| LiveError::Protocol("Docker archive is missing manifest.json".to_owned()))?;
    let mut manifests: Vec<DockerManifest> = serde_json::from_slice(manifest_bytes)
        .map_err(|error| LiveError::Protocol(format!("decode Docker archive manifest: {error}")))?;
    if manifests.len() != 1 {
        return Err(LiveError::Protocol(format!(
            "Docker archive contains {} images; expected exactly one",
            manifests.len()
        )));
    }
    let manifest = manifests
        .first_mut()
        .expect("length was checked before accessing the manifest");
    manifest.repo_tags.sort();
    manifest.repo_tags.dedup();
    if !manifest.repo_tags.iter().any(|tag| tag == expected_tag) {
        return Err(LiveError::Protocol(format!(
            "Docker archive does not contain expected tag {expected_tag}"
        )));
    }
    let config = required_member(&files, &manifest.config, "image config")?;
    let image_id = format!("sha256:{}", crate::util::hex(&Sha256::digest(config)));
    let config_path = blob_path(config);
    let mut normalized_files = BTreeMap::from([(config_path.clone(), config.clone())]);
    manifest.config = config_path.to_string_lossy().into_owned();
    let mut normalized_layers = Vec::with_capacity(manifest.layers.len());
    for layer in &manifest.layers {
        let contents = required_member(&files, layer, "layer")?;
        let path = blob_path(contents);
        normalized_files.insert(path.clone(), contents.clone());
        normalized_layers.push(path.to_string_lossy().into_owned());
    }
    manifest.layers = normalized_layers;
    normalized_files.insert(
        PathBuf::from("manifest.json"),
        serde_json::to_vec(&manifests)?,
    );

    let mut output = Vec::new();
    {
        let mut archive = Builder::new(&mut output);
        for (path, contents) in normalized_files {
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_size(u64::try_from(contents.len()).map_err(|_| {
                LiveError::Protocol(format!(
                    "Docker archive member {} is too large",
                    path.display()
                ))
            })?);
            header.set_cksum();
            archive
                .append_data(&mut header, &path, contents.as_slice())
                .map_err(|error| {
                    LiveError::Io(format!(
                        "write normalized Docker archive member {}: {error}",
                        path.display()
                    ))
                })?;
        }
        archive
            .finish()
            .map_err(|error| LiveError::Io(format!("finish normalized Docker archive: {error}")))?;
    }
    Ok(NormalizedDockerArchive {
        bytes: output,
        image_id,
    })
}

fn read_regular_files(reader: impl Read) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut archive = Archive::new(reader);
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    let entries = archive
        .entries()
        .map_err(|error| LiveError::Protocol(format!("read Docker archive: {error}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|error| LiveError::Protocol(format!("read Docker archive entry: {error}")))?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(LiveError::Protocol(
                "Docker archive contains a non-file member".to_owned(),
            ));
        }
        let path = entry
            .path()
            .map_err(|error| LiveError::Protocol(format!("decode Docker archive path: {error}")))?
            .into_owned();
        validate_path(&path)?;
        let size = entry.size();
        total = total.checked_add(size).ok_or_else(|| {
            LiveError::Protocol("Docker archive expanded size overflowed".to_owned())
        })?;
        if total > MAX_ARCHIVE_BYTES {
            return Err(LiveError::Protocol(format!(
                "Docker archive expands beyond {MAX_ARCHIVE_BYTES} bytes"
            )));
        }
        let capacity = usize::try_from(size)
            .map_err(|_| LiveError::Protocol("Docker archive member is too large".to_owned()))?;
        let mut contents = Vec::with_capacity(capacity);
        entry
            .read_to_end(&mut contents)
            .map_err(|error| LiveError::Protocol(format!("read Docker archive member: {error}")))?;
        if files.insert(path.clone(), contents).is_some() {
            return Err(LiveError::Protocol(format!(
                "Docker archive contains duplicate member {}",
                path.display()
            )));
        }
    }
    Ok(files)
}

fn validate_member_path(path: &str) -> Result<()> {
    validate_path(Path::new(path))
}

fn validate_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LiveError::Protocol(format!(
            "Docker archive contains unsafe member path {}",
            path.display()
        )));
    }
    Ok(())
}

fn required_member<'a>(
    files: &'a BTreeMap<PathBuf, Vec<u8>>,
    path: &str,
    kind: &str,
) -> Result<&'a Vec<u8>> {
    validate_member_path(path)?;
    files
        .get(Path::new(path))
        .ok_or_else(|| LiveError::Protocol(format!("Docker archive is missing {kind} {path}")))
}

fn blob_path(contents: &[u8]) -> PathBuf {
    PathBuf::from(format!(
        "blobs/sha256/{}",
        crate::util::hex(&Sha256::digest(contents))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const CONFIG: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json";
    const LAYER: &str =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/layer.tar";
    const TAG: &str = "hologram-python-test:local";

    fn archive(order: &[&str], mtime: u64, uid: u64) -> Vec<u8> {
        let manifest =
            format!(r#"[{{"Layers":["{LAYER}"],"RepoTags":["{TAG}"],"Config":"{CONFIG}"}}]"#);
        let members = BTreeMap::from([
            ("manifest.json", manifest.as_bytes()),
            (CONFIG, b"{\"architecture\":\"arm64\"}".as_slice()),
            (LAYER, b"layer bytes".as_slice()),
        ]);
        let mut bytes = Vec::new();
        {
            let mut builder = Builder::new(&mut bytes);
            for path in order {
                let contents = members[path];
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Regular);
                header.set_mode(0o600);
                header.set_uid(uid);
                header.set_gid(uid);
                header.set_mtime(mtime);
                header.set_size(contents.len() as u64);
                header.set_cksum();
                builder
                    .append_data(&mut header, path, contents)
                    .expect("append fixture");
            }
            builder.finish().expect("finish fixture");
        }
        bytes
    }

    #[test]
    fn normalization_removes_order_and_header_variation() {
        let first = normalize(
            archive(&[LAYER, "manifest.json", CONFIG], 123, 501).as_slice(),
            TAG,
        )
        .expect("normalize first");
        let second = normalize(
            archive(&[CONFIG, LAYER, "manifest.json"], 999, 1000).as_slice(),
            TAG,
        )
        .expect("normalize second");
        assert_eq!(first.bytes, second.bytes);
        assert_eq!(first.image_id, second.image_id);
        assert!(first.image_id.starts_with("sha256:"));
    }

    #[test]
    fn normalization_rejects_an_unexpected_tag() {
        let error = normalize(
            archive(&["manifest.json", CONFIG, LAYER], 0, 0).as_slice(),
            "another:tag",
        )
        .expect_err("tag mismatch");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
    }

    #[test]
    fn normalization_rejects_duplicate_members() {
        let bytes = archive(&["manifest.json", "manifest.json", CONFIG, LAYER], 0, 0);
        let error = normalize(bytes.as_slice(), TAG).expect_err("duplicate member");
        assert!(error.to_string().contains("duplicate member"));
    }

    #[test]
    fn canonical_member_set_is_stable() {
        let normalized = normalize(
            archive(&["manifest.json", CONFIG, LAYER], 50, 50).as_slice(),
            TAG,
        )
        .expect("normalize");
        let mut paths = BTreeSet::new();
        for entry in Archive::new(normalized.bytes.as_slice())
            .entries()
            .expect("entries")
        {
            paths.insert(entry.expect("entry").path().expect("path").into_owned());
        }
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(Path::new("manifest.json")));
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.starts_with("blobs/sha256"))
                .count(),
            2
        );
    }
}
