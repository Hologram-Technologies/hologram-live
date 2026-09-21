# Hologram Registry: specification and plan

The registry's requirements, design and plan, kept beside the code they describe. The code is the authority where
they disagree; the gates (`../gates/`) are the proof.

| Folder | Holds |
|---|---|
| [`v1/`](v1/) | The v1 specification ([`spec.md`](v1/spec.md): FR-001 to FR-022, SC-001 to SC-010), the phased plan ([`plan.md`](v1/plan.md), [`phases/`](v1/phases/), [`tasks.md`](v1/tasks.md)), the API and configuration contracts ([`contracts/`](v1/contracts/)), the on-disk model ([`data-model.md`](v1/data-model.md)) and the research they rest on |
| [`operability/`](operability/) | What any server owes its operator, read as the registry's requirements: form factor, operations, OpenAPI policy, parity with CNCF Distribution tier by tier ([`parity-matrix.md`](operability/parity-matrix.md)), and the ecosystem test matrix ([`interop-matrix.md`](operability/interop-matrix.md)): everything that works with Docker Registry must work with this |

Decisions live in `specs/adrs/` (025 to 032). The spike that preceded the plan is
`docs/superpowers/specs/2026-09-21-registry-p0-verdict.md`. Kept differences from the reference are in
[`../DIFFERENCES.md`](../DIFFERENCES.md), and endpoint reuse in [`../ENDPOINTS.md`](../ENDPOINTS.md).

Where these documents say "recalled" or "[memory]", the claim was written before it was measured. The reference
comparison (gate B) has since corrected several of them; `../DIFFERENCES.md` and the golden transcripts are what
was measured.
