# Hologram deterministic componentize-py distribution

Hologram applies an exact six-patch set to upstream `componentize-py` revision
`c0949b19d464f5d70bc1051195a3ae0e6a012df9` (`v0.25.0`):

- [`deterministic-build-randomness.patch`](deterministic-build-randomness.patch)
  controls private pre-initialization entropy, clocks, timestamps, allocation,
  preopens, and ambient package discovery;
- [`deterministic-generation-order.patch`](deterministic-generation-order.patch)
  gives generated sections stable map/set iteration order;
- [`wasmtime-wasi-deterministic-metadata.patch`](wasmtime-wasi-deterministic-metadata.patch)
  adds the build-only filesystem metadata identity policy;
- [`wasmtime-wasi-deterministic-readdir.patch`](wasmtime-wasi-deterministic-readdir.patch)
  gives guest directory streams stable lexical order; and
- [`deterministic-metadata-wiring.patch`](deterministic-metadata-wiring.patch)
  wires that policy to a vendored, feature-gated Wasmtime WASI dependency; and
- [`deterministic-metadata-preregistration.patch`](deterministic-metadata-preregistration.patch)
  assigns guest metadata identities with a lexical pre-execution walk instead
  of runtime observation order.

The preparation script downloads `wasmtime-wasi 46.0.1`, requires SHA-256
`e9f65ef30a2c5478873cdb619085a7a649d3ce41cc3eaf298a7ce3dee96a8e11`,
and fails if the componentize-py checkout is dirty or at another revision. The
patch set changes only the private WASI context used while pre-initializing a
Python Component:

- secure and insecure random byte streams use separate, fixed domain strings;
- `wasi:random/insecure-seed` returns zero;
- wall and monotonic clocks return fixed epoch-zero values with one-nanosecond
  resolution; and
- every private/preopened scratch tree is traversed lexically and its access and
  modification timestamps are fixed at the Unix epoch immediately before
  pre-initialization; and
- every preopened input is read-only during pre-initialization, preventing
  CPython bytecode-cache writes from reintroducing host-time mtimes; and
- guest directory entries are returned in lexical order instead of host
  filesystem enumeration order; and
- every preopened filesystem identity is assigned before guest execution by a
  lexical tree walk, so runtime metadata call order cannot change the snapshot;
  and
- CPython receives `PYTHONHASHSEED=0`; and
- CPython's debug allocator fills allocated/freed blocks deterministically.

These values are build inputs, not runtime entropy. Hologram invokes the tool
with `--stub-wasi`, so its generated components already contain no runtime WASI
random import and upstream warns that build-time PRNG state is baked into the
component. The patch makes that baked state repeatable without rewriting a
finished Wasm snapshot.

Timestamp normalization includes the supplied `--python-path` trees. Hologram
passes only compiler-owned staging directories here; the patched wheel is an
internal build tool and must not be pointed directly at a developer's source
tree.

The distribution also removes componentize-py's implicit virtualenv, pipenv,
and host-Python site-packages discovery. Hologram already passes the complete
source and locked dependency closure through explicit `--python-path` flags;
adding the uvx tool environment would violate that isolation boundary and make
its metadata part of the snapshot.

The Wasmtime patch replaces host filesystem metadata hashes only in
the private preinitializer. It maps each distinct host `(device, inode)` pair
to a distinct, context-local guest identity in deterministic observation order;
the mapping is absent from Hologram's runtime and from the emitted import-free
component. An exact locked arm64 macOS wheel build took 20m31s and produced
SHA-256
`06b3896b922e77bd6257b2b773348f62b37327fa9ea043b61054f70620904f5b`.
Two independent local compiles with separate uvx caches produced byte-identical
19,554,774-byte `.holo` archives with complete-file SHA-256
`04d9b1b62ef98336d02ddcd76e13981aa43f636c2c984a616fc7ba6af9907048`,
and the archive ran successfully. This is local evidence only: the full
five-host clean-build gate must still pass before Hologram reports the output
as reproducible.

The `componentizer-release.yml` workflow checks that this exact patch set still
applies, builds the shared CPython WASI inputs once, builds one native wheel for
each standalone-server release host, and publishes the wheels, `SHA256SUMS`,
and `PATCHSET.sha256` under a `componentizer-v*` release tag. The first
distribution is
[`componentizer-v0.25.0-hologram.1`](https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.1).
The compiler pins each exact release URL/checksum plus the patch-manifest
identity and never selects these assets by a mutable release name.
