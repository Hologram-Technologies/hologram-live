//! Validated names the registry accepts from a client.
//!
//! Every value that reaches the store or the file system passes through one of
//! these constructors first, so a path such as `../x` or a digest with a slash
//! in it cannot become a file name. The grammars are the reference registry's
//! (`contracts/registry-api.md`), written by hand: no regex crate is a direct
//! dependency of the server.

use super::OciStoreError;
use sha2::Digest as _;
use std::fmt;

/// The longest repository name the reference accepts.
const MAX_REPOSITORY_NAME: usize = 255;
/// The longest tag the reference accepts.
const MAX_TAG: usize = 128;

/// A hash algorithm a digest may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Algorithm {
    Sha256,
    Sha512,
    /// Hologram's own addresses. Accepted under `/v2/` so the hub's objects,
    /// which are stored by blake3, keep working (FR-022, ADR-031).
    Blake3,
}

impl Algorithm {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            "blake3" => Some(Self::Blake3),
            _ => None,
        }
    }

    const fn hex_len(self) -> usize {
        match self {
            Self::Sha256 | Self::Blake3 => 64,
            Self::Sha512 => 128,
        }
    }
}

/// `algorithm:hex`, lowercase hex of exactly the algorithm's length.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    /// # Errors
    ///
    /// `Invalid` when the algorithm is unknown, or the hex is not lowercase,
    /// not hex, or not the algorithm's length.
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        let invalid = || OciStoreError::Invalid {
            what: "digest",
            value: value.to_owned(),
        };
        let (name, hex) = value.split_once(':').ok_or_else(invalid)?;
        let algorithm = Algorithm::parse(name).ok_or_else(invalid)?;
        let lowercase_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        if hex.len() != algorithm.hex_len() || !hex.bytes().all(lowercase_hex) {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    /// The sha256 digest of `bytes`. For manifests, which are small.
    #[must_use]
    pub fn sha256_of(bytes: &[u8]) -> Self {
        Self(format!(
            "sha256:{}",
            crate::util::hex(&sha2::Sha256::digest(bytes))
        ))
    }

    #[must_use]
    pub fn algorithm(&self) -> Algorithm {
        let name = self.0.split(':').next().unwrap_or_default();
        // Constructed only by `parse` and `sha256_of`, so the name is known.
        Algorithm::parse(name).unwrap_or(Algorithm::Sha256)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository name: path components joined by `/`, 255 characters at most.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoName(String);

impl RepoName {
    /// # Errors
    ///
    /// `Invalid` when the name is empty, too long, or a component is outside
    /// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`.
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        let invalid = || OciStoreError::Invalid {
            what: "repository name",
            value: value.to_owned(),
        };
        if value.is_empty() || value.len() > MAX_REPOSITORY_NAME {
            return Err(invalid());
        }
        if !value
            .split('/')
            .all(|part| component_is_valid(part.as_bytes()))
        {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`
fn component_is_valid(bytes: &[u8]) -> bool {
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if !alnum(*first) || !alnum(*last) {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        if alnum(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && !alnum(bytes[index]) {
            index += 1;
        }
        let separator = &bytes[start..index];
        let legal = separator == b"."
            || separator == b"_"
            || separator == b"__"
            || separator.iter().all(|b| *b == b'-');
        if !legal {
            return false;
        }
    }
    true
}

/// A tag: `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tag(String);

impl Tag {
    /// # Errors
    ///
    /// `Invalid` when the tag is empty, longer than 128 characters, starts
    /// with `.` or `-`, or holds a character outside the grammar.
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        let invalid = || OciStoreError::Invalid {
            what: "tag",
            value: value.to_owned(),
        };
        let bytes = value.as_bytes();
        let first = *bytes.first().ok_or_else(invalid)?;
        if bytes.len() > MAX_TAG || !(first.is_ascii_alphanumeric() || first == b'_') {
            return Err(invalid());
        }
        let legal = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        if !bytes.iter().all(legal) {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a manifest route names: a tag or a digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reference {
    Tag(Tag),
    Digest(Digest),
}

impl Reference {
    /// A string with a `:` is a digest or an error, never a tag: a malformed
    /// digest must not quietly become a tag named `sha256:short`.
    ///
    /// # Errors
    ///
    /// `Invalid` from the digest or the tag grammar.
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        if value.contains(':') {
            Digest::parse(value).map(Self::Digest)
        } else {
            Tag::parse(value).map(Self::Tag)
        }
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => tag.fmt(f),
            Self::Digest(digest) => digest.fmt(f),
        }
    }
}

/// An upload session id in the canonical hyphenated UUID form. It becomes a
/// file name under the staging directory, so nothing else is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UploadId(String);

impl UploadId {
    /// # Errors
    ///
    /// `Invalid` unless the value is 36 characters, hyphens at 8, 13, 18 and
    /// 23, lowercase hex elsewhere.
    pub fn parse(value: &str) -> Result<Self, OciStoreError> {
        let invalid = || OciStoreError::Invalid {
            what: "upload id",
            value: value.to_owned(),
        };
        let bytes = value.as_bytes();
        if bytes.len() != 36 {
            return Err(invalid());
        }
        let well_formed = bytes.iter().enumerate().all(|(index, b)| match index {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(b),
        });
        if !well_formed {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UploadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_names_follow_the_reference_grammar() {
        for good in [
            "a",
            "library/alpine",
            "team/tags/app",
            "a/blobs/b",
            "a__b",
            "a---b",
            "a.b_c-d/e0",
            "0/1",
        ] {
            assert!(RepoName::parse(good).is_ok(), "{good} must be accepted");
        }
        let too_long = "a/".repeat(128); // 256 characters
        for bad in [
            "",
            "Foo",
            "a//b",
            "/a",
            "a/",
            "a___b",
            "a..b",
            "-a",
            "a-",
            "_catalog",
            "a b",
            too_long.as_str(),
        ] {
            assert!(RepoName::parse(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(
            RepoName::parse(&"a".repeat(255)).is_ok(),
            "255 is the longest legal name"
        );
    }

    #[test]
    fn tags_follow_the_reference_grammar() {
        for good in [
            "latest",
            "v1.0.0",
            "_x",
            "A-b.c_d",
            "blake3_cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959",
        ] {
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
        assert!(
            Digest::parse(&format!("blake3:{hex}")).is_ok(),
            "FR-022: the hub's objects are blake3"
        );
        assert!(Digest::parse(&format!("sha512:{}", hex.repeat(2))).is_ok());
        for bad in [
            format!("sha256:{}", hex.to_uppercase()),
            format!("sha256:{}", &hex[1..]),
            format!("md5:{hex}"),
            format!("sha256-{hex}"),
            "sha256:".to_owned(),
            format!("sha256:{hex}/../x"),
        ] {
            assert!(Digest::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_reference_with_a_colon_is_never_a_tag() {
        assert!(matches!(Reference::parse("latest"), Ok(Reference::Tag(_))));
        assert!(matches!(
            Reference::parse(&format!("sha256:{}", "0".repeat(64))),
            Ok(Reference::Digest(_))
        ));
        assert!(
            Reference::parse("sha256:short").is_err(),
            "a malformed digest is an error, not a tag named sha256:short"
        );
    }

    #[test]
    fn an_upload_id_cannot_be_a_path() {
        assert!(UploadId::parse("6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41").is_ok());
        for bad in ["..", "../x", "a/b", "", "6f1c2a9e3b7d4c559f0a2d8e7b6a5c41"] {
            assert!(UploadId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_sha256_of_bytes_is_a_digest_that_parses() {
        let digest = Digest::sha256_of(b"hello");
        assert_eq!(digest.algorithm(), Algorithm::Sha256);
        assert_eq!(Digest::parse(digest.as_str()).expect("parses"), digest);
        assert_eq!(
            digest.as_str(),
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
