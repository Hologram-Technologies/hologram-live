# ADR 029: The registry's on-disk layout, version 1, and the volumes it refuses

- Status: proposed
- Date: 2026-09-21

## Context

Hologram Registry is a drop-in replacement for the Docker Registry image for clients and configuration, not for data
on disk: the store underneath is Kappa's. An operator who points it at an existing `/var/lib/registry` volume must
get a clear refusal and the command that migrates, never a silent empty registry beside their data.

## Decision

`<root>` is `storage.filesystem.rootdirectory`, default `/var/lib/registry`.

```
<root>/HOLOGRAM_REGISTRY_LAYOUT   one line: "1"
<root>/kappa/blobs/…              the Kappa store's blobs
<root>/kappa/staging/…            the Kappa store's staging
<root>/kappa/kappa.redb           the Kappa store's database
<root>/oci/links.redb             ours (ADR 027)
```

Start-up rules, in order (`src/oci_store/layout.rs`):

1. `<root>/docker/registry/v2` exists and the marker does not: refuse, and name `hologram oci import`. The volume is
   not touched.
2. Marker missing: create the layout. The marker is written **last**, so a crash while creating leaves a directory
   the next start treats as new; every creation step is idempotent.
3. Marker says `1`: open.
4. Marker says anything else: refuse, naming the version found and the version this binary reads.

Staging, blobs and both databases must sit on one file system, so the rename that publishes a finished upload stays
atomic. A test rename between the two directories decides at start, the same way on every system.

A second process opening the same volume gets `Locked`: `links.redb` is opened first and redb refuses a second
opener with a typed error. Garbage collection relies on this to refuse beside a live server.

## Consequences

- An upgrade that changes the layout bumps the marker and ships a migration; a newer volume is never opened by an
  older binary.
- There is no in-place compatibility with a Docker Registry volume. Migration is a copy (`skopeo`, `crane`) or
  `hologram oci import`.
