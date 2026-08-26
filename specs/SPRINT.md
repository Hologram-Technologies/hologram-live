# Current sprint: M2.1 legacy empty-capability compatibility

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M2 — Capability enforcement and child attenuation](plans/holo-application-runtime.md#m2--capability-enforcement-and-child-attenuation)
- Goal: run archives emitted by the early Live compiler when they use its
  content-addressed zero-byte sentinel for an empty capability request, without
  weakening canonical capability enforcement
- Exit signal: the reported August 25 NumPy/pandas archive runs unchanged,
  synthetic direct execution proves the compatibility path, malformed nonempty
  capability objects still fail closed, all gates pass, and the fix is merged

Completed rootfs provenance work is retained in Git history and ADR 014.
Durable requirements and evidence remain in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md).

## Acceptance boundary

- Accept only the content-address-verified zero-length object used by the
  early Hologram Live compiler.
- Interpret that sentinel as the canonical semantic value “request no
  capabilities”; it can never grant storage, network, channels, or budgets.
- Preserve the legacy object κ and source bytes in plan, authorization, and
  audit identity instead of silently rewriting application identity.
- Keep `decode_canonical` strict. New source compilation, explicit grants, and
  every nonempty archive object must remain canonically encoded.
- Apply the same deny-all interpretation to requested and delegated archive
  objects so one decoder cannot become weaker than the other.
- Do not rewrite or repack a user's old `.holo` file merely to execute it.

## Runtime implementation

- [x] Identify `blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`
  as the address of the old compiler's zero-byte object.
- [x] Add an archive-only decoder that maps zero bytes to the deny-all
  `Capabilities` value.
- [x] Route both root requests and child delegations through the archive
  compatibility decoder after κ verification.
- [x] Keep canonical source/grant decoding and nonempty archive decoding
  unchanged.

## Tests and evidence

- [x] Prove requested and delegated zero-byte objects decode to zero authority.
- [x] Prove an incorrect κ for zero bytes is rejected before compatibility
  decoding.
- [x] Prove a content-addressed, nonempty malformed object remains invalid.
- [x] Prove planning accepts the legacy sentinel while preserving its κ and
  source bytes.
- [x] Prove a synthetic legacy archive executes through the direct Wasm path.
- [x] Run the user's existing `target/numpy-pandas.holo` through the current
  direct rootfs provider without recompiling it.
- [x] Run formatting, workspace tests/checks, Clippy, BDD, release/smoke, and
  documentation gates.

## Documentation and delivery

- [x] Record the compatibility slice and strict security boundary in this
  sprint before claiming verification.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with the
  discovery, acceptance criteria, and next compiler milestone.
- [x] Update user-facing `.holo` and security documentation with the narrow
  legacy rule.
- [ ] Commit, open and merge the PR, remove only this worktree, and return the
  primary checkout to clean synchronized `main`.

## Deferred work

- [ ] `DISC-019a` — Resolve mutable rootfs base tags through a registry and
  bind the selected manifest digest into build execution and evidence.
- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
