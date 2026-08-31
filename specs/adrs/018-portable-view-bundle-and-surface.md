# ADR 018: Canonical portable View bundles and desktop attachment

- Status: accepted; bundle/compiler and host-neutral provider slices implemented
- Date: 2026-08-30

## Context

The source manifest historically allowed a View layer to point at one HTML
file. The compiler stored those bytes directly under the layer's content κ.
That placeholder could not carry scripts, styles, images, or fonts, did not
identify an entry document, and left filesystem ordering and the desktop
attachment boundary undefined.

A View is a non-exit-bearing application surface. It must compose with an
exit-bearing Wasm or rootfs primary without receiving ambient Tauri, shell,
filesystem, localhost, or desktop authority. Headless runtimes must also be
able to explain why an otherwise valid application cannot attach its View.

The application has not shipped, so the directory-bundle contract replaces the
single-file placeholder without a compatibility reader.

## Decision

### Bundle format

A source View `path` names a directory. Version 1 requires `index.html` at its
root and compiles the complete regular-file tree into one canonical payload:

1. The eight-byte ASCII magic is `HOLOVIEW`, followed by a big-endian `u16`
   version (`1`).
2. A big-endian `u32` length and UTF-8 entry path follow. Version 1 permits
   exactly `index.html`.
3. A big-endian `u32` file count follows.
4. Each file is encoded in strict lexical path order as a `u32` path length,
   UTF-8 path, `u64` byte length, and exact file bytes.
5. No timestamp, permission, ownership, host path, directory record, MIME
   guess, compression metadata, or source-manifest path enters the payload.

Logical paths use `/` separators. Every component is nonempty portable ASCII
(`A-Z`, `a-z`, `0-9`, `.`, `_`, or `-`), is neither `.` nor `..`, does not end
in `.`, and is not a Windows device name. Symlinks and special files are
rejected. Case-insensitive path collisions are rejected even on a
case-sensitive host. Empty directories are not semantic content.

Version 1 allows at most 4,096 files, 1,024 bytes per logical path, 64 MiB per
file, and 256 MiB of aggregate file content. Decoders reject unsupported
versions, invalid or unordered paths, duplicate/colliding paths, a missing
entry, over-limit lengths, truncation, and trailing bytes. The complete bundle
is addressed by the existing View layer content κ; no second identity is
introduced.

### Surface and attachment contract

`portable` is the only accepted View surface in this slice. Other values fail
source validation instead of silently selecting a platform WebView.

The portable View provider attaches a validated bundle to a trusted,
host-owned surface handle. The host-neutral `hologram-view-surface` crate owns
only attachment identifiers, immutable asset records, the attach/detach trait,
and a dynamically published portable-surface registry. Platform types stay in
the host adapter. A prepared layer retains the exact resolved surface handle,
so replacing or clearing the registry cannot redirect it midway through its
lifecycle.

The Desktop adapter will own the first concrete handle. It serves assets from
an opaque per-application, per-layer origin; it does not use `file://`, expose a
workstation path, or ask the application to connect to a localhost server.
Asset MIME types are runtime delivery metadata inferred from logical paths and
are not canonical bundle content.

The web content receives no general Tauri invocation object. Application
communication crosses a versioned Hologram intent boundary owned by the View
provider. The first protocol will carry bounded named messages between the
attached View layer and its admitted application; it will not expose arbitrary
commands, shell execution, filesystem paths, or raw host APIs. Network,
storage, clipboard, notifications, and other host effects require explicit
capability admission plus a narrow provider-owned interface. Browser APIs that
cannot be mediated remain disabled.

Attachment is lifecycle work, not application invocation. `prepare` validates
and decodes the bundle without displaying it; `start` attaches only after the
requested surface exists; `stop` detaches idempotently during normal reverse
shutdown or rollback. A View never supplies application completion or exit
status. The root primary remains the only invoked, exit-bearing layer.

A runtime without the requested surface reports an explicit unavailable-
surface provider blocker before any layer starts. It does not pretend the View
attached, drop the layer, or reinterpret it as static server content.

## Consequences

- One View layer can carry a complete static frontend with stable bytes and κ
  across source creation order and filesystem metadata changes.
- Fat and thin packaging remain independent of View compilation and preserve
  the same canonical application κ.
- Existing single-file View manifests must move the file to
  `<view-directory>/index.html` and point `path` at the directory.
- The provider can be lifecycle-tested independently of Tauri. A runtime with
  an empty registry reports that the portable surface is unavailable rather
  than claiming the View provider itself is missing.
- Dynamic module loading, host intents, capability brokers, CSP, navigation,
  popup policy, and external-link handling remain provider implementation work
  under this boundary.

## Verification

Unit tests build equal bundles from trees created in opposite orders, decode
the canonical file order, and reject missing entries, unsupported surfaces,
symlinks, noncanonical ordering, and trailing bytes. Compiler tests verify a
fat View application embeds the decoded bundle and that fat/thin packages
retain equal application manifests.

The provider tests prove `prepare` does not display content, `start` attaches
the decoded immutable assets, and repeated `stop` detaches exactly once. The
enforced CLI BDD View fixture compiles into a self-contained `.holo` archive
and proves direct/headless planning returns an unavailable-surface
`LIVE_CAPABILITY_MISSING` blocker.

## Follow-up

- Register a Tauri-owned portable surface adapter and opaque asset protocol in
  Hologram Desktop.
- Define and implement the bounded intent/message schema named above.
- Add a composed Wasm + View example proving ordered startup, attachment,
  message exchange, reverse shutdown, rollback, and headless failure.
- Add bundle fuzzing and a cross-platform golden fixture to the M8 conformance
  suite.
