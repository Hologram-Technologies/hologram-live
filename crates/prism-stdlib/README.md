# prism-stdlib

`prism-stdlib` is the generated portable runtime for Prism applications and
production-system models. Version 0.2.0 preserves the complete 0.1.x Holo/1,
checked-integer, UTF-8, byte, and application foundation while adding the
formally checked `Production.System.SystemManifest` validator. The manifest is
derived from and byte-for-byte compared with the closed system graph; callers
cannot supply Boolean claims of validity.

`Production.ControlCoverage` additionally validates mandatory obligations and
permitted local/inherited contributions, including consumer residual work,
exact identity bindings, closure, uniqueness, and ordered dependencies. This
is Prism-owned structural composition, not OSCAL conformance or evidence
authentication: the surrounding boundary must authenticate the policy and
evidence identities. Its 54 modeled cases have finite expected-result proofs
and generated runtime execution; they do not claim universal soundness.

The Rust in this crate is generated from `.lex.tex` by LexLean, elaborated by
Lean 4, exported by the named `lean4-prod` LCNF boundary, and rendered by
`prod-codegen`. No handwritten Lean or alternative application semantics are
included. `generation-manifest.json` binds the generated source to its exact
LCNF input.

Both `std` and `no_std + alloc` consumers are supported:

```toml
[dependencies]
prism-stdlib = "=0.2.0"
```

The authoritative model sources, reproducible generation command, proofs,
licenses, release SBOM, and verification instructions are maintained at
<https://github.com/UOR-Foundation/PrismPM>.
