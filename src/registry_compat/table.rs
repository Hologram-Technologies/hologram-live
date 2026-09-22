//! Every setting the reference registry documents, and what Hologram Registry
//! does with it.
//!
//! Source: `docs/content/about/configuration.md` of Distribution v3.1.1, the
//! version inside the pinned `registry:3` image; the list is kept in
//! `tests/fixtures/registry-config-keys.txt` and the walk test fails if a
//! documented key has no entry here. Classes follow
//! `apps/registry/spec/operability/parity-matrix.md`.

/// What a setting does here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Read and applied.
    Supported,
    /// Accepted and has no effect, because it tunes something this registry
    /// does not have. The reason says why.
    Ignored(&'static str),
    /// Supported in this release but not built yet, and harmless to leave
    /// unapplied (observability). Accepted, and the start logs a warning, so
    /// the reference's own default file still loads.
    Pending(&'static str),
    /// Stops the start, naming the key and the reason. Also every v1 setting
    /// whose absence would change security or meaning, until it is built.
    Refused(&'static str),
}

#[derive(Debug)]
pub struct Entry {
    /// A key path, or a section: a section covers every key under it.
    pub key: &'static str,
    pub class: Class,
}

const V11: &str = "arrives in v1.1";
const V12: &str = "arrives in v1.2";
pub const NOT_BUILT: &str =
    "is supported in v1.0 and not built yet; the start refuses it rather than ignore it";

macro_rules! table {
    ($( $key:literal => $class:expr ),+ $(,)?) => {
        pub const TABLE: &[Entry] = &[$( Entry { key: $key, class: $class } ),+];
    };
}

use Class::{Ignored, Pending, Refused, Supported};

table! {
    "version" => Supported,

    "log.level" => Supported,
    "log.formatter" => Supported,
    "log.fields" => Pending("static log attributes arrive with the operations work (FR-R25)"),
    "log.accesslog.disabled" => Pending("an access log arrives with the operations work"),
    "log.hooks" => Refused("is not supported: alert from your log pipeline instead"),
    "loglevel" => Supported,

    "storage.filesystem.rootdirectory" => Supported,
    "storage.filesystem.maxthreads" => Ignored("tunes the reference's file driver"),
    "storage.azure" => Refused("is not supported in 1.x: use a managed disk"),
    "storage.gcs" => Refused("is not supported in 1.x: use GCS's S3-compatible endpoint once S3 arrives (v1.2), or a disk"),
    "storage.s3" => Refused(V12),
    "storage.inmemory" => Refused("is for tests only: use a tmpfs volume"),
    "storage.maintenance.uploadpurging" => Refused(NOT_BUILT),
    "storage.maintenance.readonly" => Refused(NOT_BUILT),
    "storage.delete.enabled" => Supported,
    "storage.cache.blobdescriptor" => Ignored("the index is local; there is nothing to cache (the value redis is refused)"),
    "storage.cache.blobdescriptorsize" => Ignored("the index is local; there is nothing to cache"),
    "storage.redirect" => Refused(V12),
    "storage.tag.concurrencylimit" => Ignored("tunes the reference's tag lookup"),

    "auth.htpasswd" => Supported,
    "auth.token" => Refused(V11),
    "auth.silly" => Refused("is for tests only"),

    "middleware" => Refused("is not supported: Go plug-in points; put a CDN or proxy in front"),

    "tags.maxtags" => Refused(NOT_BUILT),

    "http.addr" => Supported,
    "http.net" => Refused(V11),
    "http.host" => Refused(NOT_BUILT),
    "http.prefix" => Refused(V11),
    "http.secret" => Ignored("upload state is kept by the server, not in the client's URL"),
    "http.relativeurls" => Refused(NOT_BUILT),
    "http.draintimeout" => Supported,
    "http.tls.certificate" => Supported,
    "http.tls.key" => Supported,
    "http.tls.minimumtls" => Supported,
    "http.tls.clientcas" => Refused(V11),
    "http.tls.clientauth" => Refused(V11),
    "http.tls.ciphersuites" => Refused("is not supported: only safe suites are offered"),
    "http.tls.letsencrypt" => Refused("is not supported in 1.x: use cert-manager, or certbot writing the two files http.tls reads"),
    "http.debug.addr" => Supported,
    "http.debug.prometheus" => Supported,
    "http.debug.tls" => Refused(V11),
    "http.headers" => Supported,
    "http.http2.disabled" => Refused(NOT_BUILT),
    "http.h2c.enabled" => Ignored("h2c is always on; the server's gRPC needs it"),

    "notifications" => Refused(V12),
    "redis" => Refused("is not supported: it shares a cache between replicas, and v1 is one writer"),

    "health.storagedriver" => Supported,
    "health.file" => Refused(V11),
    "health.http" => Refused(V11),
    "health.tcp" => Refused(V11),

    "proxy" => Refused(V11),
    "validation" => Refused(V11),
}

/// The entry for `key`: the longest table key that is `key` itself or a
/// section containing it.
pub fn classify(key: &str) -> Option<&'static Entry> {
    TABLE
        .iter()
        .filter(|entry| {
            key == entry.key
                || key
                    .strip_prefix(entry.key)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
        .max_by_key(|entry| entry.key.len())
}
