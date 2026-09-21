# Impact on `001-hologram-registry-v1`

2026-09-21. What this definition changes in the Registry spec and plan. **The `001` files were not edited.** Each line below is an edit for whoever accepts this spec. Paths are inside `../v1/`.

## 1. Defects in `001` found by reading the reference's source and guide

These are wrong today regardless of whether `002` is accepted.

| # | Where | Defect | Edit |
|---|---|---|---|
| I1 | `research.md` D2; `contracts/config-table.md`; P5 T4 | The reference image's default file was recalled, not read. The real one [read: `dist:cmd/registry/config-dev.yml`] sets log level `debug`, `storage.delete.enabled: true`, upload purging **off**, `storage.tag.concurrencylimit: 8`, and **`http.debug.addr: :5001` with Prometheus on** | Correct D2 from the source. P5 T4's `config.yml` is built from the pinned image, as P2 T1 step 1 already extracts it |
| I2 | `contracts/config-table.md`: `http.debug.*` classed **Refused** | Our image's default file equals the reference's, so **our image would refuse its own default configuration**, and so would every operator who never wrote a config | `http.debug.addr`, `http.debug.prometheus.enabled`, `.path` → **Supported** (FR-R24, FR-S09) |
| I3 | `plan.md` ADR-028, P5 T3, P6 T3: administration listener on `127.0.0.1:5001` | Collides with the reference's default debug port | Administration moves to a Unix socket (FR-S06). Port 5001 is left alone. P6 T3 shrinks: a socket needs no TLS story |
| I4 | `contracts/config-table.md`: "an environment variable that starts with `REGISTRY_` and maps to no documented key is refused" | The guide's own htpasswd example sets `REGISTRY_AUTH=htpasswd` [read: `dist:docs/content/about/deploying.md:448`, `:508`]. `auth` is a section, not a leaf; the rule as written refuses the guide's compose file, which is gate C's `compose-swap.sh` | Add the type selectors `REGISTRY_AUTH` and `REGISTRY_STORAGE` to the table (FR-R30). Accept `htpasswd` and `filesystem`; refuse other values by name |
| I5 | nowhere | The guide's load balancing section requires `X-Forwarded-Proto` handling [read: `deploying.md:343-390`]. `001` has no requirement. The hub runs behind Caddy | New requirement (FR-R29), scenario `forwarded-headers` in P2 T2, handling in P3 `respond.rs`. +0.5 day |
| I6 | `spec.md` FR-009 and the brief: "delete off by default" | True of the binary. The reference **image** turns delete on | Reword: "delete follows `storage.delete.enabled`; the binary's default is off; the image's default file, like the reference's, turns it on". No behaviour change |
| I7 | `contracts/config-table.md`: `tags.maxtags` missing | New key in the reference's list [read: `configuration.md`, options list] | Add as Supported (FR-R23). +0.25 day. The walk test would have caught it once the fixture was extracted |
| I8 | `research.md` D1 to D6 marked "[memory]" | D1 (image metadata) is now confirmed from the Dockerfile; D2 corrected (I1); architectures are 7, not "multi" | Update marks to [read], keep "published image not run" |

## 2. Requirements that move to the Server

Same code, same days. They are rewritten so another module can use them.

| `001` | Becomes | Note |
|---|---|---|
| FR-008 (TLS), listener half | **FR-S05** | the setting names (`http.tls.*`) stay in `001`; adds certificate reload |
| FR-021 (nothing else reachable; admin on loopback) | **FR-S06** | socket instead of port |
| FR-015 (registry mode exposes only system and registry) | stays, but "registry mode paths under one root, no `$HOME`" becomes **FR-S07** | |
| The `authenticates_itself` hook (P3 T1) | **FR-S08** | |
| FR-016 (image, binaries, checksums) | stays for the registry image; binaries, signatures, SBOM, provenance become **FR-S02, FR-S03, FR-S13** | |
| ADR-026, ADR-028 | Server ADRs | titles lose the word "registry" |

## 3. Settings that change class

| Key | `001` | Now | Why |
|---|---|---|---|
| `http.debug.addr`, `http.debug.prometheus.*` | Refused | **Supported** | I2 |
| `health.storagedriver.*` | Ignored | **Supported** | feeds `/debug/health`; FR-S09 |
| `log.fields` | Ignored | **Supported** | static log attributes; FR-R25 |
| `tags.maxtags` | absent | **Supported** | I7 |
| `REGISTRY_AUTH`, `REGISTRY_STORAGE` | would be refused | **Supported** as selectors | I4 |
| `auth.token`, `proxy`, `http.tls.clientcas`, `clientauth`, `http.prefix`, `http.net`, `validation.*`, `health.file`, `.http`, `.tcp` | Refused, "out of scope" | Refused, reason text: **"arrives in v1.1"** | named release |
| `storage.s3`, `storage.redirect`, `notifications` | Refused, "out of scope" | Refused, reason text: **"arrives in v1.2"** | named release |
| everything else Refused | unchanged | unchanged, with the "instead" text from `002` Out of Scope | |

## 4. `DIFFERENCES.md`: rows added

| id | What | Why |
|---|---|---|
| D-006 | image default `log.level` is `info`, the reference's is `debug` | a development default; one line to change back. **The maintainer may prefer exact equality**: then delete this row and the change |
| D-007 | traces are not exported unless configured; the reference exports to `localhost:4318` unless told not to | "nothing leaves the process unless asked" is the Server's rule [read: `src/config.rs:199`]. The image sets `OTEL_TRACES_EXPORTER=none` like the reference, so behaviour in the image is equal |
| D-008 | `/debug/vars` and pprof answer 404 | Go runtime internals |
| note | uploads are refused below `[oci] min_free_mb` of free disk | not observable in gate B; stated for operators |

D-005's text changes from "out of scope" to the release that brings each feature.

## 5. Spec edits

- Add FR-023 to FR-030 to `001` (this file's FR-R23 to FR-R30), or cite them from `002`; one place only.
- Out-of-scope list: remove "metrics port". Rewrite "token login, pull-through mirror, webhooks, cloud object storage" as "v1.1" and "v1.2" per `002`.
- SC-010: gates F, G and H join the release condition.
- Assumptions: 80 engineer days, about 41 working days.

## 6. Plan edits, by phase

Days are engineer days **[assumption]**.

| Phase | Was | Change | Now |
|---|---|---|---|
| P0 | 4 | none | 4 |
| P1 | 6 | none | 6 |
| P2 | 5 | scenarios `forwarded-headers`, `http-on-tls-port`; read the default file from the pinned image into `contracts/` (already step 1) | 5 |
| P3 | 4 | forwarded headers in `respond.rs` (+0.5); the `/v2/` route table also emits the OpenAPI paths (rule O4, +2); route completeness test (+0.75) | 7.25 |
| P4 | 4.5 | none | 4.5 |
| P5 | 4.5 | selectors (+0.25); default file from the real source; administration socket replaces the loopback port (0); Server image (+0.5) | 5.25 |
| P6 | 3.5 | certificate reload (+0.5); T3 shrinks (−0.25); **new T5: debug listener, `/debug/health`, `/metrics`, storage probe (+3.5)**; **new T6: `OTEL_*` (+1)** | 8.25 |
| P7 | 3 | `tags.maxtags` (+0.25) | 3.25 |
| P8 | 5.5 | `adopt --rebuild-links` is the same walk as adopt (0) | 5.5 |
| P9 | 6 | gate G (in P6's tests, wired here); gate H linters, generated clients, Schemathesis (+2.75); interoperability scripts E2 to E5 (+6.5) | 15.25 |
| P10 | 5 | one tag family; signatures, SBOM, provenance (+2); licence files and gate (+1); tarball contents (+0.5); chart to Artifact Hub grade (+1.5); operations docs and generated references (+3); upgrade and backup tests (+1); security, support, changelog, templates, first screen (+1.5); gate F | 15.5 |
| **Total** | **51** | **+28.75** | **79.75, called 80** |

Calendar: 27 working days become **about 41**. The additions are mostly independent of the protocol track, so at plan time they should be dealt between the two engineers to keep both near 40 days; engineer A's track (P3, P4, P7) is still the critical path and grows by about 3 days itself. There is no cut list: the date moves by about 14 working days.

## 7. Decisions in `001` that this touches

| `001` decision | Effect |
|---|---|
| 4: registry mode hides the admin API | still yes; the mechanism is a socket, not a loopback port |
| 5: unknown `REGISTRY_*` stops the start | still yes, **with the selectors of I4 known**. Without I4 the rule breaks the guide's own example |
| 6: image base | unchanged |
| plan section 0, rule 6: tags `registry-v*` | **decided 2026-09-21: replaced by `server-v<version>`**, one tag for binaries, both images and the chart. Edit rule 6, P10 T1, T2, T6 and section 10's tag row |
| 8: the other maintainer's agreement | three more items to agree: a second listener, an administration socket, a licence gate |

## 8. What does not change

The architecture. The store adapter, `links.redb`, the crash tables, the garbage collection rule. Gates A to E. The P0 spike, which is still the next action and is unaffected by anything here.
