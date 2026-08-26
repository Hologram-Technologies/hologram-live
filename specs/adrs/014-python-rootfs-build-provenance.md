# ADR 014: Python rootfs build provenance is observational and non-canonical

- Status: accepted and implemented
- Date: 2026-08-26

## Context

The Python rootfs compiler accepts a project, lock file, target architecture,
and OCI base reference, then asks Docker to create and export an image. Before
this decision, `compile --check` validated those inputs but returned no build
evidence. A completed compile also gave operators no machine-readable link
between the requested recipe, observed Docker environment, emitted rootfs
layer, and final image identity.

That gap is especially confusing because the default base
`python:3.12-slim` is mutable and Docker's exported OCI archive is not
normalized for byte-for-byte reproducibility. Reporting observations is still
useful, but those observations must not be mistaken for either canonical
application identity or a reproducibility claim.

## Decision

Extend the version-1, `canonical: false` `build_provenance` report introduced
by ADR 013 to every source-compiled Python layer, including `rootfs`.

A rootfs entry from `compile --check` records:

- profile `rootfs`, normalized Linux target platform, and build-host OS/arch;
- the Hologram compiler and pinned uv version;
- the requested OCI base reference and whether it is already an exact
  lowercase `sha256` digest reference;
- normalized logical paths and SHA-256 values for `pyproject.toml`, `uv.lock`,
  and the versioned source-tree digest;
- Docker as the requested builder, without claiming an observed version; and
- `reproducible: false` with the unresolved mutable-base and/or unnormalized
  OCI-output blocker.

`compile --check` remains read-only and does not require or contact Docker. A
completed compile adds the observed Docker client/server versions, the locally
observed identity of the requested base when Docker exposes it, and output
evidence containing the exact rootfs layer κ, envelope byte length, final image
ID, and uncompressed image-archive byte length.

The observed base image ID is evidence about the local build. It is not treated
as a registry-resolved digest and does not rewrite a mutable source manifest.
Likewise, the output image ID and layer κ identify what was emitted; they do not
prove that a clean build will emit the same bytes.

As in ADR 013, the report is CLI result data. It remains outside the `.holo`
archive, source metadata, application directory, content blobs, and canonical
`AppManifest`, and therefore cannot affect application or archive identity.
Operators can retain it explicitly:

```console
hologram --json compile hologram.json --output application.holo \
  | jq '.build_provenance' > application.provenance.json
```

## Consequences

- The NumPy/pandas project can be audited with `jq` during both validation and
  a real build.
- Planned provenance is available on machines without Docker and cannot be
  confused with observed build-run evidence because version/output fields are
  absent until compilation.
- Mutable bases and Docker export nondeterminism remain visible, machine-
  readable blockers instead of implicit limitations.
- Provenance stays additive and may evolve independently of the archive codec.
- The report is not yet a signed attestation or an SBOM.

## Follow-up

- Resolve tag references through a registry and record the manifest digest
  used by the build without mutating the source recipe silently.
- Define and implement a normalized, byte-reproducible OCI/rootfs construction
  path, then prove equal layer κ values across clean supported hosts.
- Add dependency inventory and SBOM material for the installed rootfs closure.
- Define retention and signing when this evidence graduates to an attestation.
