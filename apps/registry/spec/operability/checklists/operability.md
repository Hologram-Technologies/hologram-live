# Operability Checklist: Hologram Server v1

**Purpose**: test whether the requirements for installing, configuring, securing, observing, upgrading and supporting the product are complete, clear and measurable. It tests the writing, not the build.
**Created**: 2026-09-21
**Feature**: [spec.md](../spec.md), [operations.md](../operations.md), [form-factor.md](../form-factor.md)

`[x]` = the writing satisfies the criterion after the fixes in Notes. `[ ]` = open, with the reason.

## Requirement Completeness

- [x] CHK001 Does every gap in `study-hologram-server.md` (G1 to G14) map to a requirement? G5 to `001`; the other 13 to FR-S01 to FR-S18. [Traceability]
- [x] CHK002 Does every section of `operations.md` trace to a requirement? **Found**: the generated settings reference and validate command (§1) and backup and restore (§6) had none. **Fixed**: FR-S24, FR-S25. [Gap]
- [x] CHK003 Are the Server's own setting names defined for what the Registry maps onto it (TLS, debug listener, metrics, health, administration socket, log fields)? **Found**: the mapping was stated, the Server keys were not. **Fixed**: table in `operations.md` §1. [Gap]
- [x] CHK004 Are backup and restore specified for a running server, not only a stopped one? [Coverage, operations §6]
- [x] CHK005 Is downgrade specified, not only upgrade? [Coverage, operations §7; spec §FR-S19]
- [x] CHK006 Is every failure mode paired with what the operator sees, what the client sees, and recovery? Nine rows. [Completeness, operations §10]
- [ ] CHK007 Is there a recovery path when the **Kappa** index is damaged? **Open**: `links.redb` can be rebuilt from tags and manifests; the Kappa index cannot in v1.0 ("restore from backup"). Stated honestly; whether that is acceptable is the maintainer's and the engineers' call. [Gap, operations §10]

## Requirement Clarity

- [x] CHK008 Are "robust" and "professional" replaced by checks? [Measurability, spec §Words]
- [x] CHK009 Is what a patch, minor and major release may change written as rules? [Clarity, operations §7]
- [x] CHK010 Are exit codes listed and tied to the code that sets them? [Clarity, operations §10; `src/main.rs:38-44`]
- [x] CHK011 Are metric label values bounded (no repository names as labels)? [Clarity, operations §3]
- [x] CHK012 Is the disclosure policy specific: address, response time, supported versions with dates? [Clarity, spec §FR-S15; operations §8]

## Security

- [x] CHK013 Is what is reachable with no configuration stated per artefact (binary, registry image, server image)? [Completeness, operations §8]
- [x] CHK014 Is the anonymous default justified and made visible to the operator? The start-up warning; gate G item 10. [Clarity]
- [x] CHK015 Is the administration surface separated from every TCP listener in container modes? FR-S06. [Completeness]
- [x] CHK016 Is there a requirement that secrets never reach logs, traces, metrics, dry-run output or the API document, with a test? [Coverage, operations §1; gate G item 9]
- [ ] CHK017 Is running as root in the registry image, to match the reference, a decision the maintainer has seen? **Open**: parity row 53 and `form-factor.md` §1 state it and why; the chart runs non-root. [Assumption]

## Consistency

- [x] CHK018 Does the disk-pressure setting live outside the reference's namespace, as `boundary.md` requires? `[oci] min_free_mb`. [Consistency]
- [x] CHK019 Does tracing default to "nothing leaves the process", and is the difference from the reference listed? D-007. [Consistency, operations §5]
- [x] CHK020 Do the chart's probes use endpoints that the default configuration turns on? `/debug/health` on 5001, on by default in the image's file. [Consistency, form-factor §3]

## Acceptance Criteria Quality

- [x] CHK021 Can each gate G item fail? Ten items, each with a threshold or an exact expected output. [Measurability, operations §12]
- [ ] CHK022 Is SC-S01 (a stranger, 5 minutes) reproducible? **Open**: it needs three real people before release. It is the one criterion no script can stand in for, and is kept on purpose. [Measurability]

## Notes

- Two defects found and fixed: CHK002, CHK003.
- Three items open: CHK007 and CHK017 want a decision; CHK022 is a deliberate human test.
