# Feature Specification: Hologram Registry v1, a drop-in container registry

**Feature Branch**: `001-hologram-registry-v1`

**Created**: 2026-09-21

**Status**: Clarified, planned (see plan.md)

**Input**: the product brief: a drop-in replacement for the Docker Registry image (`registry:3`), built on the Kappa store. Take the compose file from the registry's documentation and change only the image name; clients, commands, ports, settings, TLS and the password file keep working.

Terms: **operator** = person who runs the registry. **Reference registry** = the Docker Registry image `registry:3`, pinned to one version. **Client** = any tool that pushes or pulls images.

## Clarifications

### Session 2026-09-21

- Q: Must v1 open an existing reference data volume in place? → A: No. Copy plus a one command import only, tested in both directions.
- Q: Who owns fixes to the Kappa store? → A: Upstream requests plus carried patches from day one; agree merge rights or a maintained fork with the UOR Foundation now; v1 never waits on a merge.
- Q: What is v1 for, and who is the first user? → A: Foundation only. Eight gates green and hub.uor.foundation running on it. No outside adoption target.

### Session 2026-09-21, planning brief (settled by the maintainer, not reopened)

- The registry API is written inside Hologram Live as one new module on the Kappa store crates, pinned by revision.
- There is no cut list. If the schedule slips, the date moves.
- Two engineers.
- Garbage collection refuses to run beside a live server.
- Form factor: container image first (64-bit Intel and ARM), standalone binaries second, Helm chart third. No hosted service.

### Added by the plan audit, 2026-09-21 (each needs the maintainer's yes; the plan proceeds on the recommendation)

- FR-021 and FR-022 below. Reasons are in plan.md, "Audit of draft 1", findings 4 and 9.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Swap the image name, nothing else changes (Priority: P1)

An operator runs the reference registry today from the compose file in its documentation. They change one line, the image name, and start it again. Every client their team uses keeps working with the same commands, the same port, and the same addresses. Nobody on the team is told, and nobody notices.

**Why this priority**: This is the definition of the product. If this fails, nothing else matters. Alone it is a usable registry for a team that runs without login on a private network.

**Independent Test**: Start the product from the documented compose file with only the image name changed, then run one scripted session of pushes, pulls, tag lists and deletes against it and against the reference registry. Statuses, headers, error codes and end state match, or the difference is listed in the published differences file.

**Acceptance Scenarios**:

1. **Given** the documented compose file with only the image name changed, **When** the operator starts it, **Then** the registry answers on port 5000 and the base address returns success, as the reference does.
2. **Given** a running instance, **When** each client in the supported client list pushes an image and pulls it back, **Then** the pulled content has the same digest as the pushed content.
3. **Given** the public conformance suite for the registry protocol, version 1.1, with every category on, **When** it runs against the product, **Then** every test passes.
4. **Given** a layer larger than available memory, **When** a client pushes it and the connection drops midway, **Then** the client resumes the upload and the push completes.
5. **Given** a repository named `team/tags/app`, **When** a client pushes to it, **Then** the image is stored under exactly that name.
6. **Given** a published release, **When** an operator pulls the image on a 64-bit Intel or ARM machine, **Then** it runs, and its checksum matches the published one.

---

### User Story 2 - Secure and maintain it the way I already do (Priority: P2)

The operator protects the registry with the password file and the certificate they already have, using the same setting names. Delete stays off until they turn it on. They reclaim space with the same garbage collection command, and can preview it first. A setting the product does not support stops it at start and names the setting.

**Why this priority**: Most real deployments use login and TLS. Without this the product is a demo. It depends on Story 1 but can be tested on its own.

**Independent Test**: Reuse an existing password file, certificate and settings from a reference deployment; confirm login, refusal, delete behaviour and garbage collection match the reference in the scripted session.

**Acceptance Scenarios**:

1. **Given** an existing password file, **When** a user runs `docker login` with valid credentials, **Then** login succeeds; with wrong credentials it is refused with the same challenge the reference sends.
2. **Given** a certificate and key set through the reference's setting names, **When** the product starts, **Then** it serves over TLS on the configured port.
3. **Given** default settings, **When** a client asks to delete an image, **Then** the request is refused exactly as the reference refuses it.
4. **Given** two repositories that share a layer, **When** the image is deleted in one and garbage collection runs, **Then** the other repository still pulls successfully.
5. **Given** garbage collection with the dry run option, **When** it runs, **Then** it lists what it would remove and removes nothing.
6. **Given** a setting the product does not support (for example cloud storage, token login, mirror mode), **When** the product starts, **Then** it stops and names the unsupported setting.
7. **Given** an upload in progress, **When** the product restarts, **Then** the upload can continue.

---

### User Story 3 - Move my data in, and back out (Priority: P3)

The operator has years of images in a reference registry. They move everything into the product with one import command or a standard copy tool, and can move everything back the same way. Every image keeps its digest in both directions, so signatures and pinned deployments stay valid.

**Why this priority**: Without a tested way in, only empty registries can adopt. Without a tested way out, a careful operator will not try it.

**Independent Test**: Copy a fixed set of images reference → product → reference and compare digests at each hop.

**Acceptance Scenarios**:

1. **Given** a data directory from the reference registry, **When** the operator runs the import command, **Then** every repository, tag and image is available with identical digests.
2. **Given** a populated product instance, **When** the operator copies everything to a reference registry with a standard copy tool, **Then** every digest matches.
3. **Given** an import that is interrupted, **When** it is run again, **Then** it completes without duplicates or damage.
4. **Given** an existing reference data volume mounted directly, **When** the product starts, **Then** it refuses to start, says the volume is in the reference's layout, and names the import command. Opening a reference volume in place is not supported; copy and import are the only ways in and out.

---

### User Story 4 - Prove that what I hold is intact (Priority: P4)

The operator runs one command that re-hashes everything the registry holds and reports any object whose bytes no longer match its address.

**Why this priority**: It is the only thing in v1 the reference does not do. It is small, and it is the first visible sign of what the store underneath makes possible.

**Independent Test**: Flip one byte in a stored object, run the command, confirm it names that object and exits with failure.

**Acceptance Scenarios**:

1. **Given** an intact store, **When** verify runs, **Then** it reports the count checked and exits with success.
2. **Given** one damaged object, **When** verify runs, **Then** it names the object and the repositories that use it, and exits with failure.

---

### Edge Cases

- A push larger than the default upload limit of the underlying store (256 MiB today): must succeed without a setting change.
- Two clients push the same layer at the same time: both succeed, one copy is kept.
- A client sends a malformed image description: refused with the reference's error code, nothing stored.
- A client asks to reuse a layer from a repository it names: only that repository's layers are eligible.
- Disk fills during a push: the push fails with a clear error, and existing images stay pullable.
- Garbage collection is started while the server is running: it refuses, names the reason, and changes nothing. This is a listed difference from the reference, which allows it and can lose data.
- A tag list longer than one page: paging links behave as the reference's do.
- The password file is missing or unreadable at start: behaves as the reference does (to be confirmed by the scripted comparison, since the reference was not run during scoping).
- All non-registry routes of Hologram Server (objects, files, apps, models, chat): answer "not found" in registry mode.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The product MUST start from the reference registry's documented compose file with only the image name changed.
- **FR-002**: The product MUST serve every route of the registry protocol version 1.1, including referrers, the catalogue listing, and paging.
- **FR-003**: The product MUST return the same status codes, headers and all 18 registry error codes as the reference for the same requests, in the same structured format.
- **FR-004**: The product MUST accept repository names and tags by the reference's grammar and refuse names outside it.
- **FR-005**: The product MUST validate image descriptions before storing them.
- **FR-006**: The product MUST accept uploads of any size without holding a whole layer in memory, and MUST let an interrupted upload resume, including across a restart.
- **FR-007**: The product MUST support login from a password file with the reference's challenge, and anonymous access when none is set.
- **FR-008**: The product MUST serve TLS from a certificate and key supplied through the reference's setting names.
- **FR-009**: The product MUST keep delete off by default and scope a delete to the repository it was asked in.
- **FR-010**: The product MUST provide the reference's garbage collection command, with dry run and the delete-untagged option, and MUST never remove a layer that any image in any repository still uses.
- **FR-011**: The product MUST read the reference's environment settings and a documented subset of its settings file, and MUST refuse to start on any unsupported key, naming the key.
- **FR-012**: The product MUST provide a one command import from a reference data directory, safe to re-run.
- **FR-013**: The product MUST publish, for every difference from the reference that is kept, one entry in a differences file shipped with each release.
- **FR-020**: The product MUST hash every object as it is written and refuse bytes that do not match the address the client gave.
- **FR-014**: The product MUST provide a verify command that re-hashes everything held and reports damaged objects by name.
- **FR-015**: In registry mode, the product MUST expose only system routes (health, status, API reference) and the registry routes.
- **FR-016**: Each release MUST include an image for 64-bit Intel and ARM, standalone binaries, and checksums.
- **FR-017**: A release MUST be blocked unless all eight gates pass: (A) protocol conformance, (B) scripted comparison with the reference, (C) the client list, (D) round trip copies with peer registries, (E) peer registries pulling from the product, (F) form factor, (G) operations, (H) OpenAPI, the last three defined in the Server specification `002-hologram-server-v1`.
- **FR-018**: The client list for gate C MUST be: docker (login, push, pull, buildx), containerd, a Kubernetes pull, oras, crane, skopeo, helm, cosign, and `ollama push` and `pull`.
- **FR-021**: In registry mode, nothing but the registry routes, health, status and the API reference MUST be reachable from the network. Operator commands (verify, shutdown) MUST be reachable only from inside the container or host. *(added by the plan audit)*
- **FR-022**: Objects that hub.uor.foundation already holds, addressed by blake3, MUST stay readable and writable through the registry routes after the switch. Accepting a blake3 address is a listed difference from the reference. *(added by the plan audit)*
- **FR-019**: Fixes to the Kappa store MUST be opened as upstream requests and carried as pinned patches from day one, so that no release waits on an upstream merge. Merge rights or a maintained fork MUST be agreed with the UOR Foundation before the first release. No release may depend on a branch of a personal fork.

### Key Entities

- **Repository**: a named collection of images, for example `team/app`. Owns its own links to layers, so delete and reuse are scoped to it.
- **Image description (manifest)**: the list of layers that make up one image. Addressed by digest.
- **Layer (blob)**: a block of bytes addressed by its hash. Stored once, shared by every image that uses it.
- **Tag**: a readable name that points to one image description inside a repository.
- **Upload session**: a push in progress. Survives a restart.
- **Differences file**: the published list of every known behavioural difference from the reference.
- **Equivalence gate**: an automated check that blocks a release when it fails.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator switches from the reference by changing one line, in under 5 minutes, with zero changes to any client command or setting.
- **SC-002**: 100% of the public conformance suite, version 1.1, every category, passes on every change.
- **SC-003**: The scripted comparison shows zero unlisted differences from the reference.
- **SC-004**: Every client on the client list completes login, push and pull.
- **SC-005**: A fixed image set copied peer → product → peer keeps 100% identical digests, for every peer on the list.
- **SC-006**: A 5 GB push and pull each finish within 20% of the reference's time on the same machine.
- **SC-007**: Memory stays under 200 MB while a 20 GB layer is pushed.
- **SC-008**: Zero images become unpullable after any tested sequence of delete and garbage collection across repositories.
- **SC-009**: Verify detects 100% of single byte corruptions in the test set.
- **SC-010**: v1 is a foundation, not an adoption push. It succeeds when all eight gates are green on a published release and hub.uor.foundation serves its registry from that release. No outside adoption target is set for v1; the reason to switch is the next release's job.

## Assumptions

- The reference is `registry:3`, pinned to one exact version for all comparisons. [assumption: version to be fixed at plan time]
- The product is built on the Kappa store, by the maintainer's direction of 2026-09-20. Consequence: data on disk is not in the reference's layout. [read]
- A two day trial decides whether the Kappa store can be used inside Hologram Server on Linux, macOS and Windows. If it fails, scope is revisited before any other work. [assumption]
- The Kappa store's own claim of full conformance is unverified and is not relied on. [read, not run]
- The reference registry was not run during scoping; its behaviour here comes from its documentation. Gate B is what settles each such point. [docs]
- Two engineers, 51 engineer days, 27 working days on the calendar. The estimate is unverified. [assumption]
- There is no cut list. An earlier cut order contradicted MUST requirements and was removed. If the schedule slips, the date moves.
- Form factor: container image first, standalone binaries second, Helm chart third. No hosted service.
- Out of scope for v1: everything Hugging Face; running models; chat APIs; desktop app; `.holo` apps; plugins; public gRPC; cloud object storage; token login; pull-through mirror; webhooks; automatic certificates; metrics port; Kappa's other surfaces (S3, Git, Nix, AT Protocol, identity, federation); merging the object API and the registry into one store.
- Users have a working reference deployment or know how to run one; no new onboarding is designed.

## Done check

- **Asked**: a drop-in replacement, defined by a test. Stories 1 to 3 state the test; FR-017 makes it a release gate.
- **Skeptic**: "why not keep running the reference?" For v1 alone there is no strong answer. SC-010 says so: v1 is a foundation, measured by the gates and by the hub running on it.
- **Not verified**: whether the store builds on all three systems; whether it can stream uploads; the reference's behaviour on a missing password file, referrers, and re-hash on read; the day estimate.
