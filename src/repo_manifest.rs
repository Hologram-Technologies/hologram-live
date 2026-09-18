//! The canonical model repository manifest (`hologram.repo-manifest/v1`).
//!
//! One manifest describes one immutable revision of a model repository: every
//! file with its size, whole-file addresses, and the ordered, independently
//! addressed chunks that reassemble it. The revision identity is the BLAKE3
//! kappa of the manifest's canonical encoding, so the manifest never contains
//! its own address. See ADR 024.
//!
//! The module is pure: it performs no I/O beyond the `Read`/`Write` values and
//! fetch closure a caller passes in, and it holds no configuration.

use crate::error::{LiveError, Result};
use crate::util::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};

/// The only accepted value of [`RepoManifest::format`]. No alias is accepted.
pub const REPO_MANIFEST_FORMAT: &str = "hologram.repo-manifest/v1";

/// Default chunk size. The Kappa Registry buffers an upload at roughly three
/// times its size; a 327 MB blob was OOM-killed under a 1 GB memory limit, so
/// chunks stay well below that.
pub const DEFAULT_CHUNK_BYTES: u64 = 64 * 1024 * 1024;

/// Largest chunk a chunker may produce or a manifest may declare.
pub const MAX_CHUNK_BYTES: u64 = 256 * 1024 * 1024;

const BLAKE3_PREFIX: &str = "blake3:";
const SHA256_PREFIX: &str = "sha256:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoManifest {
    pub format: String,
    /// `namespace/repo`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ManifestSource>,
    pub files: Vec<ManifestFile>,
}

/// Where an imported manifest came from, for example
/// `{kind: "huggingface", repo: "org/model", revision: "<commit>"}`.
/// Provenance only: it confers no trust and is part of the revision identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSource {
    pub kind: String,
    pub repo: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// `sha256:<hex>` of the whole file, when known. Hugging Face reports it
    /// for LFS files, so an import can be checked against upstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// `blake3:<hex>` of the whole file.
    pub blake3: String,
    pub chunks: Vec<ManifestChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestChunk {
    pub kappa: String,
    pub offset: u64,
    pub size: u64,
}

impl RepoManifest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: REPO_MANIFEST_FORMAT.to_owned(),
            name: name.into(),
            source: None,
            files: Vec::new(),
        }
    }

    /// Canonical bytes: compact JSON, declared field order, files sorted by
    /// path (UTF-8 byte order). Refuses an invalid manifest, so no revision
    /// is ever derived from one.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut ordered = self.clone();
        ordered
            .files
            .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
        serde_json::to_vec(&ordered)
            .map_err(|error| LiveError::Protocol(format!("encode repo manifest: {error}")))
    }

    /// Decodes and validates, refusing any bytes that are not already the
    /// canonical encoding, so a revision is byte-stable across nodes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| LiveError::Protocol(format!("decode repo manifest: {error}")))?;
        if manifest.encode()? != bytes {
            return Err(LiveError::Protocol(
                "repo manifest is not canonically encoded".to_owned(),
            ));
        }
        Ok(manifest)
    }

    /// The revision identity: `blake3:<hex>` of the canonical encoding.
    pub fn revision(&self) -> Result<String> {
        Ok(kappa_of(&self.encode()?))
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != REPO_MANIFEST_FORMAT {
            return Err(protocol(format!(
                "unsupported repo manifest format {:?}; expected {REPO_MANIFEST_FORMAT:?}",
                self.format
            )));
        }
        validate_repo_name(&self.name)?;
        if let Some(source) = &self.source {
            for (field, value) in [
                ("kind", &source.kind),
                ("repo", &source.repo),
                ("revision", &source.revision),
            ] {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
                    return Err(protocol(format!(
                        "manifest source {field} {value:?} must be non-empty printable ASCII"
                    )));
                }
            }
        }
        let mut seen = BTreeSet::new();
        for file in &self.files {
            validate_file(file)?;
            if !seen.insert(file.path.as_str()) {
                return Err(protocol(format!(
                    "duplicate file path {:?} in repo manifest",
                    file.path
                )));
            }
        }
        Ok(())
    }

    pub fn file(&self, path: &str) -> Option<&ManifestFile> {
        self.files.iter().find(|file| file.path == path)
    }
}

/// `blake3:<lowercase hex>` of `bytes`.
pub fn kappa_of(bytes: &[u8]) -> String {
    format!("{BLAKE3_PREFIX}{}", hex(blake3::hash(bytes).as_bytes()))
}

/// Streams `reader` into a manifest file entry: fixed-size chunks of
/// `chunk_bytes` (the last may be shorter), whole-file BLAKE3 and SHA-256.
/// An empty input has no chunks. Holds at most one chunk in memory.
pub fn describe_file<R: Read>(path: &str, mut reader: R, chunk_bytes: u64) -> Result<ManifestFile> {
    validate_file_path(path)?;
    if chunk_bytes == 0 || chunk_bytes > MAX_CHUNK_BYTES {
        return Err(protocol(format!(
            "chunk size {chunk_bytes} must be between 1 and {MAX_CHUNK_BYTES} bytes"
        )));
    }
    let capacity = usize::try_from(chunk_bytes)
        .map_err(|_| protocol("chunk size exceeds addressable memory".to_owned()))?;
    let mut buffer = vec![0u8; capacity];
    let mut whole_blake3 = blake3::Hasher::new();
    let mut whole_sha256 = Sha256::new();
    let mut chunks = Vec::new();
    let mut offset = 0u64;
    loop {
        let filled = read_full(&mut reader, &mut buffer)
            .map_err(|error| LiveError::Io(format!("read {path:?}: {error}")))?;
        if filled == 0 {
            break;
        }
        let bytes = &buffer[..filled];
        whole_blake3.update(bytes);
        whole_sha256.update(bytes);
        let size = byte_len(bytes);
        chunks.push(ManifestChunk {
            kappa: kappa_of(bytes),
            offset,
            size,
        });
        offset += size;
        if filled < capacity {
            break;
        }
    }
    Ok(ManifestFile {
        path: path.to_owned(),
        size: offset,
        media_type: None,
        sha256: Some(format!("{SHA256_PREFIX}{}", hex(&whole_sha256.finalize()))),
        blake3: format!("{BLAKE3_PREFIX}{}", hex(whole_blake3.finalize().as_bytes())),
        chunks,
    })
}

/// Writes a file's chunks to `sink` in order. Each chunk is checked against
/// its size and kappa before any of its bytes are written; the whole file is
/// checked against `blake3` and, when present, `sha256` at the end.
///
/// On `Err` the sink may hold verified chunks of a file whose whole-file check
/// did not pass: the caller must discard it. Use [`reassemble_file`] when the
/// file fits in memory and nothing unverified may escape.
pub fn reassemble_file_to<F, W>(file: &ManifestFile, mut fetch: F, sink: &mut W) -> Result<u64>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>>,
    W: Write,
{
    validate_file(file)?;
    let mut whole_blake3 = blake3::Hasher::new();
    let mut whole_sha256 = Sha256::new();
    let mut written = 0u64;
    for chunk in &file.chunks {
        let bytes = fetch(&chunk.kappa)?.ok_or_else(|| {
            LiveError::NotFound(format!(
                "chunk {} of {:?} is not available",
                chunk.kappa, file.path
            ))
        })?;
        if byte_len(&bytes) != chunk.size {
            return Err(LiveError::InvalidHolo(format!(
                "chunk {} of {:?} is {} bytes but the manifest declares {}",
                chunk.kappa,
                file.path,
                bytes.len(),
                chunk.size
            )));
        }
        if kappa_of(&bytes) != chunk.kappa {
            return Err(LiveError::InvalidHolo(format!(
                "chunk at offset {} of {:?} does not match its kappa {}",
                chunk.offset, file.path, chunk.kappa
            )));
        }
        whole_blake3.update(&bytes);
        whole_sha256.update(&bytes);
        sink.write_all(&bytes)
            .map_err(|error| LiveError::Io(format!("write {:?}: {error}", file.path)))?;
        written += chunk.size;
    }
    let blake3 = format!("{BLAKE3_PREFIX}{}", hex(whole_blake3.finalize().as_bytes()));
    if blake3 != file.blake3 {
        return Err(LiveError::InvalidHolo(format!(
            "reassembled {:?} is {blake3} but the manifest declares {}",
            file.path, file.blake3
        )));
    }
    if let Some(expected) = &file.sha256 {
        let sha256 = format!("{SHA256_PREFIX}{}", hex(&whole_sha256.finalize()));
        if &sha256 != expected {
            return Err(LiveError::InvalidHolo(format!(
                "reassembled {:?} is {sha256} but the manifest declares {expected}",
                file.path
            )));
        }
    }
    Ok(written)
}

/// In-memory reassembly: returns the bytes only if every check passed.
pub fn reassemble_file<F>(file: &ManifestFile, fetch: F) -> Result<Vec<u8>>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>>,
{
    let mut output = Vec::with_capacity(usize::try_from(file.size).unwrap_or(0));
    reassemble_file_to(file, fetch, &mut output)?;
    Ok(output)
}

pub fn validate_repo_name(name: &str) -> Result<()> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return Err(protocol(format!(
            "repo name {name:?} must be \"namespace/repo\" with non-empty parts"
        )));
    }
    for part in parts {
        if !part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(protocol(format!(
                "repo name part {part:?} may only contain [A-Za-z0-9._-]"
            )));
        }
    }
    Ok(())
}

/// Relative, `/`-separated, no empty, `.` or `..` segments, no backslash, no
/// control characters. Paths compare as exact UTF-8 bytes: no Unicode
/// normalization is applied.
pub fn validate_file_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || path.chars().any(char::is_control)
    {
        return Err(protocol(format!("invalid repo file path {path:?}")));
    }
    Ok(())
}

/// `blake3:` followed by 64 lowercase hex characters.
pub fn validate_kappa(kappa: &str) -> Result<()> {
    validate_digest(kappa, BLAKE3_PREFIX)
}

fn validate_digest(value: &str, prefix: &str) -> Result<()> {
    let valid = value.strip_prefix(prefix).is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    });
    if !valid {
        return Err(protocol(format!(
            "{value:?} must be {prefix:?} followed by 64 lowercase hex characters"
        )));
    }
    Ok(())
}

fn validate_file(file: &ManifestFile) -> Result<()> {
    validate_file_path(&file.path)?;
    if let Some(media_type) = &file.media_type {
        if !media_type.contains('/') || !media_type.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(protocol(format!(
                "file {:?} media type {media_type:?} is malformed",
                file.path
            )));
        }
    }
    if let Some(sha256) = &file.sha256 {
        validate_digest(sha256, SHA256_PREFIX)?;
    }
    validate_kappa(&file.blake3)?;
    let mut expected_offset = 0u64;
    for chunk in &file.chunks {
        validate_kappa(&chunk.kappa)?;
        if chunk.size == 0 || chunk.size > MAX_CHUNK_BYTES {
            return Err(protocol(format!(
                "file {:?} declares a {} byte chunk; chunks must be 1 to {MAX_CHUNK_BYTES} bytes",
                file.path, chunk.size
            )));
        }
        if chunk.offset != expected_offset {
            return Err(protocol(format!(
                "file {:?} chunks are not contiguous: offset {} where {expected_offset} was expected",
                file.path, chunk.offset
            )));
        }
        expected_offset = expected_offset
            .checked_add(chunk.size)
            .ok_or_else(|| protocol(format!("file {:?} chunk offsets overflow", file.path)))?;
    }
    if expected_offset != file.size {
        return Err(protocol(format!(
            "file {:?} chunks cover {expected_offset} bytes but the manifest declares {}",
            file.path, file.size
        )));
    }
    Ok(())
}

fn read_full<R: Read>(reader: &mut R, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn byte_len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

fn protocol(message: String) -> LiveError {
    LiveError::Protocol(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const ABC_BLAKE3: &str =
        "blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
    const ABC_SHA256: &str =
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const EMPTY_BLAKE3: &str =
        "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    fn describe(path: &str, bytes: &[u8], chunk_bytes: u64) -> ManifestFile {
        describe_file(path, bytes, chunk_bytes).expect("describe")
    }

    fn manifest_with(files: Vec<ManifestFile>) -> RepoManifest {
        let mut manifest = RepoManifest::new("org/repo");
        manifest.files = files;
        manifest
    }

    /// A content-addressed store holding every chunk of `bytes`.
    fn store_for(bytes: &[u8], file: &ManifestFile) -> BTreeMap<String, Vec<u8>> {
        file.chunks
            .iter()
            .map(|chunk| {
                let start = usize::try_from(chunk.offset).expect("offset");
                let end = start + usize::try_from(chunk.size).expect("size");
                (chunk.kappa.clone(), bytes[start..end].to_vec())
            })
            .collect()
    }

    fn fetch_from(
        store: &BTreeMap<String, Vec<u8>>,
    ) -> impl FnMut(&str) -> Result<Option<Vec<u8>>> + '_ {
        move |kappa| Ok(store.get(kappa).cloned())
    }

    fn rejects(manifest: &RepoManifest, why: &str) {
        let error = manifest
            .validate()
            .expect_err(&format!("expected rejection: {why}"));
        assert!(matches!(error, LiveError::Protocol(_)), "{why}: {error:?}");
        assert!(manifest.encode().is_err(), "{why}: no encoding either");
    }

    #[test]
    fn whole_file_addresses_match_independent_vectors() {
        let file = describe("abc.txt", b"abc", DEFAULT_CHUNK_BYTES);
        assert_eq!(file.blake3, ABC_BLAKE3);
        assert_eq!(file.sha256.as_deref(), Some(ABC_SHA256));
        assert_eq!(file.chunks.len(), 1);
        assert_eq!(file.chunks[0].kappa, ABC_BLAKE3, "one chunk is the file");
        assert_eq!(kappa_of(b""), EMPTY_BLAKE3);
    }

    #[test]
    fn canonical_encoding_is_pinned_by_golden_bytes() {
        let mut manifest = manifest_with(vec![
            describe("z/b.bin", b"abc", 2),
            describe("a.json", b"", 2),
        ]);
        manifest.files[1].media_type = Some("application/json".to_owned());
        manifest.files[1].sha256 = None;
        manifest.source = Some(ManifestSource {
            kind: "huggingface".to_owned(),
            repo: "org/repo".to_owned(),
            revision: "0123abcd".to_owned(),
        });

        let encoded = String::from_utf8(manifest.encode().expect("encode")).expect("utf8");
        let golden = concat!(
            r#"{"format":"hologram.repo-manifest/v1","name":"org/repo","#,
            r#""source":{"kind":"huggingface","repo":"org/repo","revision":"0123abcd"},"#,
            r#""files":[{"path":"a.json","size":0,"media_type":"application/json","#,
            r#""blake3":"blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262","chunks":[]},"#,
            r#"{"path":"z/b.bin","size":3,"#,
            r#""sha256":"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","#,
            r#""blake3":"blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85","#,
            r#""chunks":[{"kappa":"GOLDEN_AB","offset":0,"size":2},"#,
            r#"{"kappa":"GOLDEN_C","offset":2,"size":1}]}]}"#,
        )
        .replace("GOLDEN_AB", &kappa_of(b"ab"))
        .replace("GOLDEN_C", &kappa_of(b"c"));
        assert_eq!(encoded, golden);
        assert_eq!(
            manifest.revision().expect("revision"),
            "blake3:b3044e7638fc223661bcd935a845bd4f5f2461bf657a7d606f1b969427088654"
        );

        // In-memory order never reaches the bytes.
        let mut reordered = manifest.clone();
        reordered.files.reverse();
        assert_eq!(reordered.encode().expect("encode"), encoded.as_bytes());
    }

    #[test]
    fn source_is_optional_and_part_of_the_revision() {
        let plain = manifest_with(vec![describe("a.bin", b"abc", 2)]);
        let text = String::from_utf8(plain.encode().expect("encode")).expect("utf8");
        assert!(!text.contains("source"));
        let mut sourced = plain.clone();
        sourced.source = Some(ManifestSource {
            kind: "huggingface".to_owned(),
            repo: "org/repo".to_owned(),
            revision: "main".to_owned(),
        });
        assert_ne!(
            plain.revision().expect("revision"),
            sourced.revision().expect("revision")
        );
    }

    #[test]
    fn decode_round_trips_and_refuses_non_canonical_bytes() {
        let manifest = manifest_with(vec![
            describe("b.bin", b"hello", 2),
            describe("a.bin", b"x", 2),
        ]);
        let encoded = manifest.encode().expect("encode");
        let decoded = RepoManifest::decode(&encoded).expect("decode");
        assert_eq!(decoded.encode().expect("encode"), encoded);

        let mut padded = encoded.clone();
        padded.push(b'\n');
        assert!(RepoManifest::decode(&padded).is_err(), "trailing newline");

        let text = String::from_utf8(encoded).expect("utf8");
        let unsorted = text
            .replacen("\"a.bin\"", "\"TMP\"", 1)
            .replacen("\"b.bin\"", "\"a.bin\"", 1)
            .replacen("\"TMP\"", "\"b.bin\"", 1);
        assert!(
            RepoManifest::decode(unsorted.as_bytes()).is_err(),
            "unsorted files"
        );

        let extra = text.replacen("{\"format\"", "{\"extra\":1,\"format\"", 1);
        assert!(
            RepoManifest::decode(extra.as_bytes()).is_err(),
            "unknown field"
        );

        let legacy = text.replacen(REPO_MANIFEST_FORMAT, "kappahub.repo-manifest/v1", 1);
        assert!(
            RepoManifest::decode(legacy.as_bytes()).is_err(),
            "no legacy alias"
        );
    }

    #[test]
    fn chunk_boundaries_on_an_exact_multiple() {
        let bytes: Vec<u8> = (0..8).collect();
        let file = describe("f", &bytes, 4);
        let spans: Vec<(u64, u64)> = file.chunks.iter().map(|c| (c.offset, c.size)).collect();
        assert_eq!(spans, vec![(0, 4), (4, 4)]);
        assert_eq!(file.size, 8);
        assert_eq!(file.chunks[0].kappa, kappa_of(&bytes[..4]));
        assert_eq!(file.chunks[1].kappa, kappa_of(&bytes[4..]));
    }

    #[test]
    fn chunk_boundaries_with_a_remainder() {
        let bytes: Vec<u8> = (0..10).collect();
        let file = describe("f", &bytes, 4);
        let spans: Vec<(u64, u64)> = file.chunks.iter().map(|c| (c.offset, c.size)).collect();
        assert_eq!(spans, vec![(0, 4), (4, 4), (8, 2)]);
        assert_eq!(file.chunks[2].kappa, kappa_of(&bytes[8..]));
    }

    #[test]
    fn empty_file_has_no_chunks() {
        let file = describe("empty", b"", 4);
        assert_eq!(file.size, 0);
        assert_eq!(file.blake3, EMPTY_BLAKE3);
        assert!(file.chunks.is_empty());
        manifest_with(vec![file])
            .validate()
            .expect("empty file validates");
    }

    #[test]
    fn identical_chunks_share_one_kappa() {
        let file = describe("f", &[7u8; 8], 4);
        assert_eq!(file.chunks[0].kappa, file.chunks[1].kappa);
    }

    #[test]
    fn chunker_reads_through_short_reads() {
        struct Trickle<'a>(&'a [u8]);
        impl Read for Trickle<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let Some((first, rest)) = self.0.split_first() else {
                    return Ok(0);
                };
                buffer[0] = *first;
                self.0 = rest;
                Ok(1)
            }
        }
        let bytes: Vec<u8> = (0..10).collect();
        let trickled = describe_file("f", Trickle(&bytes), 4).expect("describe");
        assert_eq!(trickled, describe("f", &bytes, 4));
    }

    #[test]
    fn chunk_size_parameter_is_bounded() {
        assert!(describe_file("f", &b"x"[..], 0).is_err());
        assert!(describe_file("f", &b"x"[..], MAX_CHUNK_BYTES + 1).is_err());
        assert!(describe_file("../f", &b"x"[..], 4).is_err());
    }

    #[test]
    fn validation_rejects_every_malformed_field() {
        let good = || manifest_with(vec![describe("dir/weights.bin", b"weights", 4)]);
        good().validate().expect("baseline validates");

        let mut m = good();
        m.format = "kappahub.repo-manifest/v1".to_owned();
        rejects(&m, "legacy format string");

        for name in ["justonepart", "org/", "/repo", "a/b/c", "org/Repo!"] {
            let mut m = good();
            m.name = name.to_owned();
            rejects(&m, name);
        }

        for path in [
            "", "/abs", "a\\b", "a//b", "a/", "./a", "a/../b", "..", "a/./b", "a\u{0}b", "a\nb",
        ] {
            let mut m = good();
            m.files[0].path = path.to_owned();
            rejects(&m, &format!("path {path:?}"));
        }

        for field in ["kind", "repo", "revision"] {
            let mut source = ManifestSource {
                kind: "huggingface".to_owned(),
                repo: "org/repo".to_owned(),
                revision: "main".to_owned(),
            };
            let (empty, spaced) = match field {
                "kind" => (&mut source.kind, "hugging face"),
                "repo" => (&mut source.repo, "org /repo"),
                _ => (&mut source.revision, "tab\t"),
            };
            empty.clear();
            let mut m = good();
            m.source = Some(source.clone());
            rejects(&m, &format!("empty source {field}"));
            let mut spaced_source = source;
            match field {
                "kind" => spaced_source.kind = spaced.to_owned(),
                "repo" => spaced_source.repo = spaced.to_owned(),
                _ => spaced_source.revision = spaced.to_owned(),
            }
            let mut m = good();
            m.source = Some(spaced_source);
            rejects(&m, &format!("whitespace in source {field}"));
        }

        let mut m = good();
        m.files[0].media_type = Some("octet".to_owned());
        rejects(&m, "media type without slash");

        for sha256 in [
            "deadbeef".to_owned(),
            "sha256:abc".to_owned(),
            format!("sha256:{}", "AB".repeat(32)),
            format!("blake3:{}", "ab".repeat(32)),
        ] {
            let mut m = good();
            m.files[0].sha256 = Some(sha256.clone());
            rejects(&m, &format!("sha256 {sha256:?}"));
        }

        for kappa in [
            "sha256:deadbeef".to_owned(),
            format!("blake3:{}", "AB".repeat(32)),
            format!("blake3:{}", "ab".repeat(31)),
            format!("blake3:{}g", "a".repeat(63)),
        ] {
            let mut m = good();
            m.files[0].chunks[0].kappa.clone_from(&kappa);
            rejects(&m, &format!("chunk kappa {kappa:?}"));
            let mut m = good();
            m.files[0].blake3.clone_from(&kappa);
            rejects(&m, &format!("file blake3 {kappa:?}"));
        }

        let mut m = good();
        m.files[0].chunks[1].offset += 1;
        rejects(&m, "gap between chunks");

        let mut m = good();
        m.files[0].chunks[1].offset -= 1;
        rejects(&m, "overlapping chunks");

        let mut m = good();
        m.files[0].size += 1;
        rejects(&m, "chunks short of size");

        let mut m = good();
        m.files[0].size -= 1;
        rejects(&m, "chunks beyond size");

        let mut m = good();
        m.files[0].chunks.push(ManifestChunk {
            kappa: EMPTY_BLAKE3.to_owned(),
            offset: 7,
            size: 0,
        });
        rejects(&m, "zero-size chunk");

        let mut m = good();
        m.files[0].size = MAX_CHUNK_BYTES + 1;
        m.files[0].chunks = vec![ManifestChunk {
            kappa: ABC_BLAKE3.to_owned(),
            offset: 0,
            size: MAX_CHUNK_BYTES + 1,
        }];
        rejects(&m, "chunk above the cap");

        let mut m = good();
        m.files[0].size = 0;
        rejects(&m, "empty file with chunks");

        let mut m = good();
        m.files.push(describe("dir/weights.bin", b"other", 4));
        rejects(&m, "duplicate path");
    }

    #[test]
    fn reassembly_succeeds_for_valid_chunks() {
        let bytes: Vec<u8> = (0..=255).cycle().take(1000).collect();
        let file = describe("w.bin", &bytes, 64);
        let store = store_for(&bytes, &file);
        assert_eq!(
            reassemble_file(&file, fetch_from(&store)).expect("reassemble"),
            bytes
        );

        let empty = describe("e.bin", b"", 64);
        assert!(reassemble_file(&empty, |_| Ok(None))
            .expect("empty")
            .is_empty());

        let mut sink = Vec::new();
        let written = reassemble_file_to(&file, fetch_from(&store), &mut sink).expect("stream");
        assert_eq!(written, 1000);
        assert_eq!(sink, bytes);
    }

    #[test]
    fn planted_flipped_chunk_byte_fails_closed() {
        let bytes: Vec<u8> = (0..32).collect();
        let file = describe("w.bin", &bytes, 8);
        let mut store = store_for(&bytes, &file);
        store.get_mut(&file.chunks[2].kappa).expect("chunk")[3] ^= 0x01;
        let error = reassemble_file(&file, fetch_from(&store)).expect_err("flipped byte");
        assert!(matches!(error, LiveError::InvalidHolo(_)), "{error:?}");
    }

    #[test]
    fn planted_reordered_chunks_fail_closed() {
        let bytes: Vec<u8> = (0..32).collect();
        let mut file = describe("w.bin", &bytes, 8);
        let store = store_for(&bytes, &file);
        // Swap two chunks' kappas, keeping offsets: well formed, wrong order.
        file.chunks.swap(0, 1);
        file.chunks[0].offset = 0;
        file.chunks[1].offset = 8;
        manifest_with(vec![file.clone()])
            .validate()
            .expect("still well formed");
        let error = reassemble_file(&file, fetch_from(&store)).expect_err("reordered");
        assert!(matches!(error, LiveError::InvalidHolo(_)), "{error:?}");
    }

    #[test]
    fn planted_wrong_whole_file_blake3_fails_closed() {
        let bytes: Vec<u8> = (0..32).collect();
        let mut file = describe("w.bin", &bytes, 8);
        let store = store_for(&bytes, &file);
        file.blake3 = kappa_of(b"something else");
        let error = reassemble_file(&file, fetch_from(&store)).expect_err("wrong blake3");
        assert!(matches!(error, LiveError::InvalidHolo(_)), "{error:?}");
    }

    #[test]
    fn planted_wrong_sha256_fails_closed() {
        let bytes: Vec<u8> = (0..32).collect();
        let mut file = describe("w.bin", &bytes, 8);
        let store = store_for(&bytes, &file);
        file.sha256 = Some(ABC_SHA256.to_owned());
        let error = reassemble_file(&file, fetch_from(&store)).expect_err("wrong sha256");
        assert!(matches!(error, LiveError::InvalidHolo(_)), "{error:?}");

        // Without a declared sha256 the same bytes reassemble.
        file.sha256 = None;
        assert_eq!(
            reassemble_file(&file, fetch_from(&store)).expect("no sha256"),
            bytes
        );
    }

    #[test]
    fn missing_or_mis_sized_chunks_fail_closed() {
        let bytes: Vec<u8> = (0..32).collect();
        let file = describe("w.bin", &bytes, 8);
        let error = reassemble_file(&file, |_| Ok(None)).expect_err("missing");
        assert!(matches!(error, LiveError::NotFound(_)), "{error:?}");

        let mut long = store_for(&bytes, &file);
        long.get_mut(&file.chunks[1].kappa).expect("chunk").push(0);
        assert!(
            reassemble_file(&file, fetch_from(&long)).is_err(),
            "long chunk"
        );

        let mut malformed = file.clone();
        malformed.chunks.pop();
        let store = store_for(&bytes, &file);
        assert!(
            reassemble_file(&malformed, fetch_from(&store)).is_err(),
            "structurally invalid"
        );
    }

    #[test]
    fn stream_reassembly_writes_nothing_from_a_bad_chunk() {
        let bytes: Vec<u8> = (0..32).collect();
        let file = describe("w.bin", &bytes, 8);
        let mut store = store_for(&bytes, &file);
        store.get_mut(&file.chunks[1].kappa).expect("chunk")[0] ^= 0xff;
        let mut sink = Vec::new();
        assert!(reassemble_file_to(&file, fetch_from(&store), &mut sink).is_err());
        assert_eq!(
            sink,
            bytes[..8],
            "only the verified first chunk reached the sink"
        );
    }
}
