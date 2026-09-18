# ADR 024: One canonical manifest for a model repository revision

- Status: proposed
- Date: 2026-09-17

## Context

Two consumers need to describe a model repository as addressed bytes.

A Hugging Face compatible read surface (so that `HF_ENDPOINT` can point at a
Hologram node) must answer a tree listing with sizes and LFS SHA-256 values,
and serve `resolve` requests with `Range` and a stable `ETag`. It needs, per
file, the size, the whole-file digests, and where each byte range lives.

Model Hub publishing (the deploy scripts proposed in #79) writes an interim document,
`hologram.model-hub.model/v1`, from a Node script: files cut into 64 MiB BLAKE3
chunks, with whole-file SHA-256 and BLAKE3. It is not specified, not validated
in Rust, has no canonical encoding, and records chunk kappas without offsets or
sizes.

Measured on the hub deployment: the Kappa Registry buffers an upload at roughly
three times its size, and a 327 MB blob was OOM-killed under a 1 GB memory
limit. Chunk size is therefore an operational constraint, not a free choice.

## Decision

`src/repo_manifest.rs` defines `hologram.repo-manifest/v1`, a pure module with
no I/O of its own and no new dependency (it uses `blake3`, `sha2`, `serde_json`
and `util::hex`, all already present).

**Shape.** `{format, name, source?, files}`. `name` is `namespace/repo` over
`[A-Za-z0-9._-]`. `source` is optional provenance `{kind, repo, revision}`,
for example a Hugging Face import. Each file is
`{path, size, media_type?, sha256?, blake3, chunks}` and each chunk is
`{kappa, offset, size}`. Addresses are `blake3:` or `sha256:` followed by 64
lowercase hex characters. `sha256` is optional because not every publisher has
it; `blake3` is required.

**Format string.** Exactly `hologram.repo-manifest/v1`. No alias is accepted,
including the unpublished prototype string `kappahub.repo-manifest/v1`, which
never shipped in this repository.

**Canonical encoding and revision.** The encoding is compact JSON in declared
field order, absent optionals omitted, files sorted by path in UTF-8 byte
order. The revision is `blake3:` of those bytes. The manifest never contains
its own address. `decode` refuses bytes that are not already canonical, and
unknown fields, so one revision has exactly one byte form. `source` is part of
the encoding: the same bytes imported from two upstream revisions are two
manifest revisions.

**Chunking.** `describe_file` streams a reader into fixed-size chunks of a
caller-chosen size, from 1 byte to `MAX_CHUNK_BYTES` (256 MiB), with
`DEFAULT_CHUNK_BYTES` = 64 MiB. It computes whole-file BLAKE3 and SHA-256 in
the same pass and holds one chunk in memory. An empty file has no chunks.
Fixed boundaries (not content-defined) keep the result identical to the
interim publisher for non-empty files.

**Validation.** Paths are relative and `/`-separated with no empty, `.` or `..`
segments, no backslash and no control characters; they are compared as exact
UTF-8 bytes with no Unicode normalization. Paths are unique. Chunks are 1 byte
to 256 MiB, contiguous from offset 0, and cover the file size exactly. Encoding
validates first, so no revision is derived from an invalid manifest.

**Reassembly is fail-closed.** `reassemble_file_to` checks each fetched chunk's
size and kappa before writing any of its bytes, then the whole file against
`blake3` and, when present, `sha256`. On error a streaming sink may hold
verified chunks of an unverified file and the caller must discard it;
`reassemble_file` returns bytes only when every check passed.

## Alternatives considered

**Keep the interim `hologram.model-hub.model/v1`.** Rejected: bare chunk lists
cannot answer a `Range` request without re-deriving offsets, and a document
with no canonical encoding cannot name a revision.

**Content-defined chunking.** Rejected for v1: it improves deduplication across
edited files, but model weights are replaced rather than edited, and fixed
boundaries are simpler to verify and to serve by range. A future version can
change the chunker without changing the chunk record.

**Single chunk for files under the cap** (the prototype's rule). Rejected:
anything above roughly 300 MB would reach the registry as one upload and hit
the measured memory failure.

**Unicode normalization or ASCII-only paths.** Not adopted. Normalization needs
a new dependency; ASCII-only would refuse valid upstream repositories. Exact
byte comparison is deterministic; the cost is that NFC and NFD spellings of one
name are distinct paths.

**Keeping `source` out of the revision.** Rejected: provenance that does not
change the identity could be swapped without detection.

## Consequences

- The HF-compatible read surface and the hub publisher can share one Rust type,
  one validator and one reassembler.
- The interim publisher must move to this format; its chunk kappas carry over
  for non-empty files, and it currently emits one empty chunk for an empty file.
- The manifest is a library contract, not a user-reachable capability, so
  `ACTUAL_CAPABILITIES.md` is unchanged until a command or route exposes it.
- Revision identity depends on `serde_json`'s compact output; the golden-bytes
  test fails if that output ever changes.

## Verification

`cargo test repo_manifest` covers: independent BLAKE3 and SHA-256 vectors,
golden canonical bytes and revision, order independence, canonical-only decode,
chunk boundaries on an exact multiple, with a remainder and for an empty file,
short reads, every validation rejection, and reassembly. Planted defects, each
required to fail: a flipped chunk byte, reordered chunks, a wrong whole-file
BLAKE3, a wrong SHA-256. Removing the SHA-256, whole-file BLAKE3, chunk-kappa
or offset check each makes a test fail.

`features/suites/s4_repositories/repo_manifest.feature` records the behaviour as
scenarios. The BDD runner drives the `hologram` binary and no command exposes
this module yet, so the suite is tagged `@status:specified` and becomes
`@status:enforced` with the first user-reachable surface.
