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
`python:3.12-slim` is mutable and Docker's exported archive was not normalized.
ADR 015 subsequently closed the
mutable-tag build race by binding real builds to a registry manifest digest.
Reporting observations is still useful, but those observations must not be
mistaken for either canonical application identity or a reproducibility claim.

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
- `reproducible: false` with the unresolved mutable-base and/or remaining
  clean-supported-host proof blocker.

`compile --check` remains read-only and does not require or contact Docker. A
completed compile adds the registry-resolved reference that ADR 015 passes to
Docker, observed Docker client/server versions, the locally observed identity
of that resolved base when Docker exposes it, and output evidence containing
the exact rootfs layer κ, envelope byte length, final image ID, and
uncompressed image-archive byte length.

The registry-resolved reference and observed local image ID are distinct:
`resolved_reference` is the immutable registry manifest identity actually used
by `FROM`, while `observed_image_id` is optional local-engine evidence. Neither
rewrites the mutable source manifest. Likewise, the output image ID and layer κ
identify what was emitted; they do not prove that a clean build will emit the
same bytes.

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
- Planned checks expose an unresolved mutable base without contacting a
  registry; completed builds expose its digest binding. Docker export
  normalization is reported explicitly, while clean-build equality remains a
  machine-readable blocker under ADR 017.
- Provenance stays additive and may evolve independently of the archive codec.
- The report is not yet a signed attestation or an SBOM.

## Follow-up

- Keep ADR 015's registry resolver compatible with authenticated registries and
  future OCI-native builders.
- Complete ADR 017's uncached cross-host proof and close any remaining
  build-content differences before setting `reproducible: true`.
- Add dependency inventory and SBOM material for the installed rootfs closure.
- Define retention and signing when this evidence graduates to an attestation.
