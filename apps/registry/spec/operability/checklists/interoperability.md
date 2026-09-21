# Ecosystem Interoperability Checklist: Hologram Server v1

**Purpose**: test whether the interoperability requirements (Docker, Kubernetes, CI, CNCF, Artifact Hub, OpenAPI) are complete, clear and measurable. It tests the writing, not the build.
**Created**: 2026-09-21
**Feature**: [spec.md](../spec.md), [interop-matrix.md](../interop-matrix.md), [openapi-policy.md](../openapi-policy.md)

`[x]` = the writing satisfies the criterion after the fixes in Notes. `[ ]` = open, with the reason.

## Requirement Completeness

- [x] CHK001 Is every claim of "works with X" attached to a named script and a cadence? 50 rows, each with both. [Completeness, interop §1–3]
- [x] CHK002 Are both directions of Artifact Hub specified: hosting others' artifacts and being listed ourselves? [Completeness, interop §Artifact Hub]
- [x] CHK003 Are the exact media types and the special tag Artifact Hub relies on written down? Confirmed from its documents today: tag `artifacthub.io`, config `application/vnd.cncf.artifacthub.config.v1+yaml`, layer `application/vnd.cncf.artifacthub.repository-metadata.layer.v1.yaml`. [Clarity]
- [x] CHK004 Is the limit of Artifact Hub interoperability stated (it must be able to reach the registry)? [Edge case, interop §Artifact Hub]
- [x] CHK005 Are Kubernetes pull paths all covered: kubelet through containerd and CRI-O, mirrors, private CA, pull secrets, local-cluster recipes? Journeys J3 to J5. [Coverage]
- [x] CHK006 Is the single-writer limit and its consequence for Deployments, StatefulSets and rolling updates specified? [Completeness, form-factor §3; spec §US2 scenarios 2 and 5]
- [ ] CHK007 Is each non-Helm, non-image Artifact Hub kind that can live in OCI listed individually with a test? **Open**: the table groups them ("other OCI-capable kinds") because all use the same three registry calls. Listing 29 kinds one by one would add rows, not proof. Reviewer's call. [Completeness]

## Requirement Clarity

- [x] CHK008 Is "seamless" replaced by journeys with time limits? J1 to J6. [Measurability]
- [x] CHK009 Is "fully interoperable with the CNCF ecosystem" bounded to a list, with the rule that an unlisted project is not claimed? [Ambiguity → interop §2 pushback]
- [x] CHK010 Are non-CNCF projects in the CNCF table marked as such? OCI, Sigstore, SLSA. Levels fetched from `cncf.io/projects` today; ORAS and zot are Sandbox from memory, since that page lists only Graduated and Incubating. [Clarity]
- [x] CHK011 Are supported Kubernetes versions stated as a rule, not a number that rots? **Found**: "1.30 to current". **Fixed**: the three newest minor versions on release day. [Clarity]
- [ ] CHK012 Are client version floors verified? **Open**: floors are from memory; `001` P2 T4 confirms each against its release page. [Assumption]

## Consistency

- [x] CHK013 Does SC-S04 count the same rows as the matrix? **Found**: it said "21 of 22 green for three nights", but four project rows are per release and cannot be nightly. **Fixed**: SC-S04 now counts per-commit, nightly and per-release rows separately. [Conflict]
- [x] CHK014 Do gate names agree across files (A to E in `001`; F, G, H here)? [Consistency]

## Scenario Coverage

- [x] CHK015 Is there a rule for a flaky or broken third-party tool, so a red row cannot be skipped silently? **Found**: none. **Fixed**: the quarantine rule in `interop-matrix.md`. [Gap]
- [x] CHK016 Are CI use cases beyond GitHub covered? GitLab service and buildx registry cache. [Coverage]
- [x] CHK017 Are signing tools covered in both discovery modes (referrers and tag fallback)? cosign both; notation through referrers. [Coverage]

## OpenAPI

- [x] CHK018 Is the OpenAPI version fixed and verified against what the generator emits? 3.1.0, read in `utoipa` 5.5.0's source. [Clarity, openapi-policy §start]
- [x] CHK019 Is completeness defined in both directions, and per running configuration? O3. [Completeness]
- [x] CHK020 Does the policy say what OpenAPI cannot express for `/v2/` (slashes in `{name}`, opaque upload URLs, streams)? [Edge case, openapi-policy §cannot say]
- [x] CHK021 Are breaking changes defined and tied to the versioning policy? O5, O7; `operations.md` §7. [Consistency]
- [ ] CHK022 Is it confirmed that OpenAPI Generator's Python and Go clients can be made to leave `/` unencoded in a path parameter? **Open**: O9 tests it with a two-segment name; if a generator cannot, the documented workaround (a custom path template) is part of that task. [Assumption]

## Notes

- Three defects found and fixed: CHK011, CHK013, CHK015.
- Three items open: CHK007 (a judgment call), CHK012 and CHK022 (assumptions a named task settles).
