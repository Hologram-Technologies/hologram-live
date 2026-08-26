# ADR 012: Python Components admit only locked portable wheels

- Status: accepted and implemented
- Date: 2026-08-26

## Context

The import-free `hologram:guest/component@1` provider can run CPython packaged
by `componentize-py --stub-wasi`, but the first compiler slice accepted only an
editable project with no third-party packages. Pure-Python libraries do not
need WASI or native linking and can be embedded in the same component. Reading
the developer virtual environment, resolving mutable package versions during
componentization, or accepting host-native wheels would make the archive
ambient and non-portable.

`uv.lock` records the resolved package closure and artifact hashes. It may also
record source distributions, Git/path dependencies, and platform-specific
wheels. Those forms either execute build code or carry a host ABI, so accepting
all lock records would overstate what the import-free Component profile can
run.

## Decision

Source profile `wasi-component` supports `uv.lock` format version 1, revisions
1 through 3. The editable root project remains source-staged through the
existing `pyproject.toml`, declared lock file, and `src/` boundary. The compiler
traverses that project's `dependencies` graph, excluding unreferenced
development and optional records. Every reached external package must:

1. declare a registry source;
2. include an HTTPS wheel URL;
3. include a SHA-256 artifact hash;
4. use a Python 3 tag and the platform-independent `none-any` ABI/platform
   tags.

If a package records multiple qualifying wheels, the compiler prefers the
exact `py3` tag over a combined tag and then chooses lexical URL order. Package
names are validated before writing a private requirements file. Git/path
dependencies, source-only packages, native wheel tags, unsupported lock
revisions, missing hashes, and unsafe names fail during `compile --check`,
before network access or archive emission.

During `compile`, Hologram writes exact direct wheel references with their
SHA-256 fragments and invokes:

```text
uv --no-config pip install --no-index --no-deps --require-hashes \
  --only-binary :all: --python-version 3.14 --link-mode copy --target ...
```

No version is re-resolved and no source distribution is built. Copy mode keeps
the private target independent from uv cache links. The compiler recursively
rejects symlinks and native library suffixes in the installed tree, then adds
that directory as an explicit `componentize-py --python-path` after the private
adapter and application source.

Both subprocesses withhold `VIRTUAL_ENV`, `PYTHONPATH`, `PYTHONHOME`, Python
user-site lookup, pip/uv index overrides, and discovered configuration. Build
time may fetch only the exact lock URLs; runtime remains the unchanged
import-free Component v1 world with no network, filesystem, environment, or
other WASI host interface.

## Diagnostics

- Malformed lock structure, unsupported lock revisions, and unsafe package
  names return `LIVE_CONFIG_INVALID`.
- A package without an admissible portable wheel, a non-registry source, an
  installed symlink, or an installed native payload returns
  `LIVE_CAPABILITY_MISSING` and recommends the explicit `rootfs` profile.
- Hash/download/installer failure prevents componentization and archive
  emission; it never falls back to an index or another artifact.

## Consequences

- Dependency-free projects remain compatible.
- Locked pure-Python dependencies can execute directly and resident without
  widening the runtime contract or adding a `.holo` layer kind.
- Packages such as NumPy and pandas remain on the explicit rootfs path because
  their native wheels are not valid Component inputs.
- The compiler conservatively requires every reached runtime dependency to be
  portable; marker-aware pruning within that graph is deferred until it can be
  performed for a defined Component Python target rather than the build host.
- Artifact URLs and hashes are compiler inputs but are not yet emitted as a
  versioned provenance report or added to canonical application identity.
- Byte-for-byte reproducibility is not claimed. Pinned
  `componentize-py 0.25.0 --stub-wasi` bakes a build-time PRNG seed and exposes
  no deterministic seed control; clean builds currently produce different
  component and application κ values.

## Follow-up

- Define a versioned, non-canonical build-provenance report containing the
  Python runtime, component toolchain, target ABI, artifact URLs, and hashes.
- Obtain an upstream deterministic seed/source-epoch control or replace the
  nondeterministic componentization step before checking reproducible-output
  acceptance criteria.
- Add marker-aware dependency pruning only with an explicit, stable Component
  Python target model.
- Add real WASI imports only through a new guest-contract identifier and
  capability admission as required by ADR 011.
