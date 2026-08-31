//! Canonical payloads for portable Hologram View layers.

use crate::error::{LiveError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const MAGIC: &[u8; 8] = b"HOLOVIEW";
pub const VIEW_BUNDLE_VERSION: u16 = 1;
pub const PORTABLE_SURFACE: &str = "portable";
pub const PORTABLE_ENTRY: &str = "index.html";
const MAX_FILES: usize = 4_096;
const MAX_PATH_BYTES: usize = 1_024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewBundle {
    pub version: u16,
    pub entry: String,
    pub files: Vec<ViewFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn validate_surface(surface: &str) -> Result<()> {
    if surface == PORTABLE_SURFACE {
        Ok(())
    } else {
        Err(LiveError::Config(format!(
            "unsupported View surface {surface:?}; expected {PORTABLE_SURFACE:?}"
        )))
    }
}

pub fn compile_directory(directory: &Path) -> Result<Vec<u8>> {
    let metadata =
        std::fs::symlink_metadata(directory).map_err(|error| LiveError::io(directory, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LiveError::Config(format!(
            "View layer path {} must be a directory, not a file or symlink",
            directory.display()
        )));
    }

    let mut files = BTreeMap::new();
    let mut folded_paths = BTreeSet::new();
    let mut total_bytes = 0;
    collect_files(
        directory,
        directory,
        &mut files,
        &mut folded_paths,
        &mut total_bytes,
    )?;
    if !files.contains_key(PORTABLE_ENTRY) {
        return Err(LiveError::Config(format!(
            "View bundle {} must contain {PORTABLE_ENTRY}",
            directory.display()
        )));
    }

    let bundle = ViewBundle {
        version: VIEW_BUNDLE_VERSION,
        entry: PORTABLE_ENTRY.to_owned(),
        files: files
            .into_iter()
            .map(|(path, bytes)| ViewFile { path, bytes })
            .collect(),
    };
    encode(&bundle)
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    folded_paths: &mut BTreeSet<String>,
    total_bytes: &mut u64,
) -> Result<()> {
    let entries = std::fs::read_dir(directory).map_err(|error| LiveError::io(directory, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| LiveError::io(directory, error))?;
        let path = entry.path();
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|error| LiveError::io(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(LiveError::Config(format!(
                "View bundles do not permit symlinks: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_files(root, &path, files, folded_paths, total_bytes)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(LiveError::Config(format!(
                "View bundles contain only regular files: {}",
                path.display()
            )));
        }
        if files.len() >= MAX_FILES {
            return Err(LiveError::Config(format!(
                "View bundle exceeds the {MAX_FILES}-file limit"
            )));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(LiveError::Config(format!(
                "View asset {} exceeds the {MAX_FILE_BYTES}-byte limit",
                path.display()
            )));
        }
        *total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| LiveError::Config("View bundle byte length overflowed".to_owned()))?;
        if *total_bytes > MAX_BUNDLE_BYTES {
            return Err(LiveError::Config(format!(
                "View bundle exceeds the {MAX_BUNDLE_BYTES}-byte limit"
            )));
        }

        let relative = path.strip_prefix(root).map_err(|error| {
            LiveError::Config(format!(
                "derive View asset path {}: {error}",
                path.display()
            ))
        })?;
        let logical = portable_path(relative)?;
        let folded = logical.to_ascii_lowercase();
        if !folded_paths.insert(folded) {
            return Err(LiveError::Config(format!(
                "View bundle contains case-insensitive path collision at {logical:?}"
            )));
        }
        let bytes = std::fs::read(&path).map_err(|error| LiveError::io(&path, error))?;
        files.insert(logical, bytes);
    }
    Ok(())
}

fn portable_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(LiveError::Config(format!(
                "View asset path {} is not relative and normalized",
                path.display()
            )));
        };
        let part = part.to_str().ok_or_else(|| {
            LiveError::Config(format!(
                "View asset path {} is not valid UTF-8",
                path.display()
            ))
        })?;
        validate_component(part, path)?;
        parts.push(part);
    }
    let logical = parts.join("/");
    if logical.is_empty() || logical.len() > MAX_PATH_BYTES {
        return Err(LiveError::Config(format!(
            "View asset path {} must contain 1..={MAX_PATH_BYTES} bytes",
            path.display()
        )));
    }
    Ok(logical)
}

fn validate_component(component: &str, path: &Path) -> Result<()> {
    let valid = !component.is_empty()
        && component != "."
        && component != ".."
        && !component.ends_with('.')
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    let stem = component
        .split_once('.')
        .map_or(component, |(stem, _)| stem)
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if valid && !reserved {
        Ok(())
    } else {
        Err(LiveError::Config(format!(
            "View asset path {} contains non-portable component {component:?}",
            path.display()
        )))
    }
}

pub fn encode(bundle: &ViewBundle) -> Result<Vec<u8>> {
    validate_bundle(bundle, false)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&bundle.version.to_be_bytes());
    write_len(&mut bytes, bundle.entry.len(), "View entry path")?;
    bytes.extend_from_slice(bundle.entry.as_bytes());
    write_len(&mut bytes, bundle.files.len(), "View file count")?;
    for file in &bundle.files {
        write_len(&mut bytes, file.path.len(), "View asset path")?;
        bytes.extend_from_slice(file.path.as_bytes());
        let length = u64::try_from(file.bytes.len())
            .map_err(|_| LiveError::Config("View asset is too large".to_owned()))?;
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(&file.bytes);
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<ViewBundle> {
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(invalid("invalid View bundle magic"));
    }
    let version = reader.u16()?;
    let entry = reader.string()?;
    let file_count =
        usize::try_from(reader.u32()?).map_err(|_| invalid("View file count is too large"))?;
    if file_count > MAX_FILES {
        return Err(invalid("View file count exceeds the format limit"));
    }
    let mut files = Vec::with_capacity(file_count);
    let mut total_bytes = 0_u64;
    for _ in 0..file_count {
        let path = reader.string()?;
        let length = usize::try_from(reader.u64()?)
            .map_err(|_| invalid("View asset length is too large"))?;
        let length_u64 =
            u64::try_from(length).map_err(|_| invalid("View asset length is too large"))?;
        if length_u64 > MAX_FILE_BYTES {
            return Err(invalid("View asset exceeds the format limit"));
        }
        total_bytes = total_bytes
            .checked_add(length_u64)
            .filter(|total| *total <= MAX_BUNDLE_BYTES)
            .ok_or_else(|| invalid("View bundle exceeds the format limit"))?;
        files.push(ViewFile {
            path,
            bytes: reader.take(length)?.to_vec(),
        });
    }
    if !reader.remaining().is_empty() {
        return Err(invalid("View bundle has trailing bytes"));
    }
    let bundle = ViewBundle {
        version,
        entry,
        files,
    };
    validate_bundle(&bundle, true)?;
    Ok(bundle)
}

fn validate_bundle(bundle: &ViewBundle, archive: bool) -> Result<()> {
    let error = |message: &str| {
        if archive {
            invalid(message)
        } else {
            LiveError::Config(message.to_owned())
        }
    };
    if bundle.version != VIEW_BUNDLE_VERSION {
        return Err(error("unsupported View bundle version"));
    }
    if bundle.entry != PORTABLE_ENTRY {
        return Err(error("View bundle entry must be index.html"));
    }
    if bundle.files.is_empty() || bundle.files.len() > MAX_FILES {
        return Err(error("View bundle file count is outside the format limit"));
    }
    let mut previous: Option<&str> = None;
    let mut folded_paths = BTreeSet::new();
    let mut has_entry = false;
    let mut total = 0_u64;
    for file in &bundle.files {
        portable_path(Path::new(&file.path)).map_err(|_| error("invalid View asset path"))?;
        if previous.is_some_and(|value| value >= file.path.as_str()) {
            return Err(error("View assets must be strictly lexically ordered"));
        }
        if !folded_paths.insert(file.path.to_ascii_lowercase()) {
            return Err(error(
                "View assets contain a case-insensitive path collision",
            ));
        }
        previous = Some(&file.path);
        has_entry |= file.path == bundle.entry;
        let length =
            u64::try_from(file.bytes.len()).map_err(|_| error("View asset is too large"))?;
        if length > MAX_FILE_BYTES {
            return Err(error("View asset exceeds the format limit"));
        }
        total = total
            .checked_add(length)
            .ok_or_else(|| error("View bundle byte length overflowed"))?;
    }
    if total > MAX_BUNDLE_BYTES {
        return Err(error("View bundle exceeds the format limit"));
    }
    if !has_entry {
        return Err(error("View bundle is missing index.html"));
    }
    Ok(())
}

fn write_len(output: &mut Vec<u8>, length: usize, description: &str) -> Result<()> {
    let length = u32::try_from(length)
        .map_err(|_| LiveError::Config(format!("{description} is too large")))?;
    output.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("truncated View bundle"))?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| invalid("truncated View bundle version"))?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| invalid("truncated View bundle length"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| invalid("truncated View bundle length"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn string(&mut self) -> Result<String> {
        let length =
            usize::try_from(self.u32()?).map_err(|_| invalid("View string length is too large"))?;
        if length == 0 || length > MAX_PATH_BYTES {
            return Err(invalid("View string length is outside the format limit"));
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| invalid("View path is not valid UTF-8"))
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}

fn invalid(message: &str) -> LiveError {
    LiveError::InvalidHolo(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_order_and_metadata_do_not_change_bundle_bytes() {
        let first = tempfile::tempdir().expect("first");
        let second = tempfile::tempdir().expect("second");
        for (directory, reverse) in [(first.path(), false), (second.path(), true)] {
            std::fs::create_dir_all(directory.join("assets")).expect("assets");
            let files = [
                (
                    "index.html",
                    b"<script src=\"assets/app.js\"></script>".as_slice(),
                ),
                ("assets/app.js", b"console.log('hologram')".as_slice()),
                ("assets/app.css", b"body { color: black; }".as_slice()),
            ];
            let values: Vec<_> = if reverse {
                files.into_iter().rev().collect()
            } else {
                files.into_iter().collect()
            };
            for (path, bytes) in values {
                std::fs::write(directory.join(path), bytes).expect("asset");
            }
        }

        let first_bytes = compile_directory(first.path()).expect("first bundle");
        let second_bytes = compile_directory(second.path()).expect("second bundle");
        assert_eq!(first_bytes, second_bytes);
        let bundle = decode(&first_bytes).expect("decode");
        assert_eq!(bundle.entry, PORTABLE_ENTRY);
        assert_eq!(
            bundle
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["assets/app.css", "assets/app.js", "index.html"]
        );
    }

    #[test]
    fn rejects_missing_entry_and_non_portable_surface() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(directory.path().join("other.html"), "missing entry").expect("asset");
        let error = compile_directory(directory.path()).expect_err("missing index");
        assert!(error.to_string().contains(PORTABLE_ENTRY), "{error}");
        let error = validate_surface("desktop").expect_err("unsupported surface");
        assert!(error.to_string().contains(PORTABLE_SURFACE), "{error}");
    }

    #[test]
    fn decoder_rejects_noncanonical_order_and_trailing_bytes() {
        let bundle = ViewBundle {
            version: VIEW_BUNDLE_VERSION,
            entry: PORTABLE_ENTRY.to_owned(),
            files: vec![
                ViewFile {
                    path: PORTABLE_ENTRY.to_owned(),
                    bytes: b"entry".to_vec(),
                },
                ViewFile {
                    path: "app.js".to_owned(),
                    bytes: b"app".to_vec(),
                },
            ],
        };
        assert!(encode(&bundle).is_err());

        let canonical = ViewBundle {
            version: VIEW_BUNDLE_VERSION,
            entry: PORTABLE_ENTRY.to_owned(),
            files: vec![ViewFile {
                path: PORTABLE_ENTRY.to_owned(),
                bytes: b"entry".to_vec(),
            }],
        };
        let mut bytes = encode(&canonical).expect("encode");
        bytes.push(0);
        assert!(decode(&bytes).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(directory.path().join(PORTABLE_ENTRY), "entry").expect("entry");
        symlink(PORTABLE_ENTRY, directory.path().join("alias.html")).expect("symlink");
        let error = compile_directory(directory.path()).expect_err("symlink");
        assert!(error.to_string().contains("symlink"), "{error}");
    }
}
