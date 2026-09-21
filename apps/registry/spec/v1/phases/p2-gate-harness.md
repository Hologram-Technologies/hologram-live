# P2. Gate harness, built against the reference alone

> Step level. Entry: P0 done (needs no product code, and does not need a go). Engineer B. 5 days (days 3 to 7).

**Goal:** the tools that will judge the product exist, and are proven correct, before the product does. Exit: the reference compared with itself shows zero differences over 12 scenarios; every client script is green against the reference; the first golden transcripts are in the repository for engineer A to build against.

**Why first:** the reference was never run during scoping (`research.md`, section "Registry reference": every row is from memory). Each **B:`name`** cell in `contracts/registry-api.md` is a question. This phase answers them in week 1.

**Architecture:** everything lives in a new top-level directory, `apps/registry/gates/`, outside the cargo workspace, so `just verify` and upstream merges never see it.

```
apps/registry/gates/
  reference.env                 REGISTRY_REF=registry:3@sha256:<digest>   (one line; the pin)
  differential/                 a small Rust crate, its own Cargo.lock
    src/main.rs  scenario.rs  play.rs  normalise.rs  compare.rs  allowlist.rs
    scenarios/*.toml
    golden/*.json               transcripts recorded from the reference
  clients/lib.sh  *.sh          gate C, one script per client
  corpus/build.sh  corpus.lock  gate D
  compose/                      the reference's documented compose files, verbatim, with their source URL
apps/registry/DIFFERENCES.md                  repository root; shipped with each release (FR-013)
.github/workflows/gates.yml
```

One line is added to the root `Cargo.toml` so cargo does not claim the crate: under `[workspace]`, `exclude = ["apps/registry/gates/differential"]`. That is the only shared file this phase touches.

**Tech:** Rust 1.97.1; `reqwest` (blocking, rustls, no default features), `serde`, `serde_json`, `toml`, `sha2`, `clap`. Docker on the runner. No YAML, no async.

---

## Task 1: The runner: scenario format, play, normalise, compare

**Requirements:** FR-013, FR-003, SC-003. **Files:** create `apps/registry/gates/differential/Cargo.toml`, `src/{main,scenario,play,normalise,compare}.rs`, `apps/registry/gates/reference.env`; modify root `Cargo.toml` (one `exclude` line).

**Interfaces (produces):**
- CLI: `differential run --left <url> --right <url> [--scenario <name>] [--allowlist apps/registry/DIFFERENCES.md]`, `differential record --target <url> --out golden/`, `differential check-golden --target <url>`
- `Scenario::load(path: &Path) -> Result<Scenario, String>`
- `play(scenario: &Scenario, base: &Url, seed: u64) -> Transcript`
- `normalise(transcript: &mut Transcript, base: &Url)`
- `compare(left: &Transcript, right: &Transcript) -> Vec<Difference>` where `Difference { scenario, step, field, left, right }` and `field` is `status`, `header:<name>`, `body`, `body.errors[0].code`, `state:<what>`

**Scenario format** (TOML, because `toml` is already a dependency of the main tree and B knows it):

```toml
name = "push-chunked"
summary = "Start, two PATCH chunks with Content-Range, finish with PUT, then HEAD the blob."
needs = []                       # any of: "delete", "auth", "readonly"  → selects the config variant both sides start with
repo = "gate-b/chunked"

[[step]]
id = "start"
method = "POST"
path = "/v2/{repo}/blobs/uploads/"
capture = { upload = "header:location" }

[[step]]
id = "chunk-1"
method = "PATCH"
url = "{upload}"                 # a captured Location is followed verbatim, relative or absolute
headers = { "content-type" = "application/octet-stream", "content-range" = "0-1048575" }
body = "bytes:seed=1:len=1048576"
capture = { upload = "header:location" }

[[step]]
id = "finish"
method = "PUT"
url = "{upload}"
query = { digest = "{sha256:seed=1:len=1048576}" }

[[step]]
id = "head"
method = "HEAD"
path = "/v2/{repo}/blobs/{sha256:seed=1:len=1048576}"

[state]                          # dumped from both sides after the last step and compared
catalog = true
tags = ["{repo}"]
```

Body generators: `bytes:seed=N:len=M` (deterministic, from a blake3 XOF, never held whole: streamed), `json:<inline>`, `file:<path under scenarios/>`, `manifest:oci:layers=[…]` (builds a valid manifest from generated blobs and yields its digest as `{manifest}`).

- [ ] **Step 1: Pin the reference.**

```bash
docker pull registry:3
docker inspect --format '{{index .RepoDigests 0}}' registry:3     # registry@sha256:…
printf 'REGISTRY_REF=%s\n' "registry:3@sha256:<digest>" > apps/registry/gates/reference.env
docker inspect --format '{{json .Config.Entrypoint}} {{json .Config.Cmd}} {{json .Config.ExposedPorts}} {{json .Config.Volumes}}' "registry:3@sha256:<digest>"
docker run --rm --entrypoint cat "registry:3@sha256:<digest>" /etc/distribution/config.yml
```

Expected: entrypoint `["registry"]`, command `["serve","/etc/distribution/config.yml"]`, port 5000, volume `/var/lib/registry`. Paste the four outputs into `apps/registry/gates/compose/REFERENCE-IMAGE.md`. **If any differs from `research.md` D1 or D2, correct `research.md` and `contracts/config-table.md` in the same commit.** This closes CHK001.

- [ ] **Step 2: Write the failing tests for normalisation** (`apps/registry/gates/differential/src/normalise.rs`). This is the part that decides whether the gate is trustworthy, so it is tested first and hardest.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> (String, String) { (name.to_owned(), value.to_owned()) }

    #[test]
    fn upload_ids_and_state_tokens_vanish_but_the_shape_stays() {
        let mut step = Step::response(202, vec![
            header("location", "http://127.0.0.1:5000/v2/gate-b/x/blobs/uploads/6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41?_state=abcDEF123"),
            header("docker-upload-uuid", "6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41"),
            header("range", "0-0"),
        ], b"");
        normalise_step(&mut step, "http://127.0.0.1:5000");
        assert_eq!(step.header("location"), Some("<base>/v2/gate-b/x/blobs/uploads/<uuid>"));
        assert_eq!(step.header("docker-upload-uuid"), Some("<uuid>"));
        assert_eq!(step.header("range"), Some("0-0"), "meaningful values are untouched");
    }

    #[test]
    fn a_relative_and_an_absolute_location_are_different_on_purpose() {
        let mut absolute = Step::response(202, vec![header("location", "http://h:5000/v2/r/blobs/uploads/6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41")], b"");
        let mut relative = Step::response(202, vec![header("location", "/v2/r/blobs/uploads/6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41")], b"");
        normalise_step(&mut absolute, "http://h:5000");
        normalise_step(&mut relative, "http://h:5000");
        assert_ne!(absolute.header("location"), relative.header("location"), "B:location-form must be able to fail");
    }

    #[test]
    fn only_compared_headers_survive() {
        let mut step = Step::response(200, vec![header("date", "Mon, 21 Sep 2026 10:00:00 GMT"), header("x-content-type-options", "nosniff"),
            header("content-length", "2"), header("server", "whatever")], b"{}");
        normalise_step(&mut step, "http://h:5000");
        assert_eq!(step.header("date"), None);
        assert_eq!(step.header("server"), None);
        assert_eq!(step.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(step.header("content-length"), Some("2"));
    }

    #[test]
    fn json_bodies_compare_by_value_not_by_key_order_or_spacing() {
        let mut a = Step::response(404, vec![header("content-type", "application/json")], br#"{"errors":[{"code":"BLOB_UNKNOWN","message":"blob unknown to registry","detail":{"digest":"sha256:00"}}]}"#);
        let mut b = Step::response(404, vec![header("content-type", "application/json; charset=utf-8")], b"{ \"errors\": [ { \"detail\": {\"digest\":\"sha256:00\"}, \"message\":\"blob unknown to registry\", \"code\":\"BLOB_UNKNOWN\" } ] }\n");
        normalise_step(&mut a, "x"); normalise_step(&mut b, "x");
        assert_eq!(a.body, b.body);
        assert_ne!(a.header("content-type"), b.header("content-type"), "the charset difference is real and is reported");
    }

    #[test]
    fn content_length_is_dropped_only_when_the_body_held_a_normalised_value() {
        // A body that contained a UUID changes length after normalisation; comparing Content-Length would be noise.
        // A blob body's Content-Length is signal and stays.
    }
}
```

Compared headers (lowercase): `content-type`, `content-length`, `content-range`, `accept-ranges`, `range`, `location`, `link`, `etag`, `www-authenticate`, `allow`, `docker-content-digest`, `docker-distribution-api-version`, `docker-upload-uuid`, `oci-subject`, `oci-filters-applied`, `x-content-type-options`, `cache-control`. Every other header is dropped. This list is the answer to CHK008 ("which headers count"); header order and timing do not count.

- [ ] **Step 3: Run.** `cd apps/registry/gates/differential && cargo test` → FAIL (nothing exists). 
- [ ] **Step 4: Implement** `scenario.rs` (serde structs for the format above, `{name}` substitution, generators), `play.rs` (one `reqwest::blocking::Client`, redirects **off**, `http1_only`, 60 s timeout; a streamed body for `bytes:`; records method, URL template, status, headers, body or, over 64 KiB, its sha256 and length), `normalise.rs`, `compare.rs`.

The one subtle piece, in `play.rs`: a captured `Location` may be relative or absolute and may carry opaque query state. Follow it verbatim, then **append** the scenario's `query`, never replace:

```rust
fn resolve(base: &Url, captured: &str, extra: &BTreeMap<String, String>) -> Url {
    let mut url = base.join(captured).expect("a Location header is a URL or a path");
    url.query_pairs_mut().extend_pairs(extra.iter());      // keeps `_state=…` from the reference
    url
}
```

- [ ] **Step 5: `run` starts both sides itself.** `differential run --left reference --right reference` means: for each scenario, start two fresh containers of `REGISTRY_REF` on random loopback ports with a `tmpfs` volume and the config variant the scenario `needs`, wait for `GET /v2/` to answer, play, dump state, stop. A fresh store per scenario keeps scenarios independent. `--right <url>` or `--right image:<ref>` swaps the product in later (P9).

Config variants, as environment on the container, equal on both sides: `default` (none); `delete` (`REGISTRY_STORAGE_DELETE_ENABLED=true`); `auth` (`REGISTRY_AUTH_HTPASSWD_REALM`, `…_PATH`, with a generated bcrypt file mounted); `readonly` (`REGISTRY_STORAGE_MAINTENANCE_READONLY_ENABLED=true`).

- [ ] **Step 6: Run** → tests PASS. Commit: `test(gates): differential runner for gate B`.

**Done when:** the five normalisation tests pass and `differential run --left reference --right reference --scenario <any>` exits 0.

**Traps:**
- Redirects must be off. A registry that answers 307 and one that answers 200 are different; a client that follows redirects hides it.
- Header names are case-insensitive; values are not. Lowercase names only.
- The reference returns `Location` with `_state`. The product has none. That is normalised away, and recorded once in `apps/registry/DIFFERENCES.md` as a note, not as a behavioural difference (`contracts/registry-api.md`).
- Do not compare `Content-Length` on `HEAD` of a manifest blindly: both must equal the manifest size. It stays compared; it is signal.

---

## Task 2: Twelve scenarios and their golden transcripts

**Requirements:** FR-003; answers the **B:** questions. **Files:** create `apps/registry/gates/differential/scenarios/*.toml`, `apps/registry/gates/differential/golden/*.json`.

The first twelve, in the order engineer A will need them:

| # | Scenario | Pins | First consumer |
|---|---|---|---|
| 1 | `base` | `GET /v2/`, `GET /v2` (**B:`v2-no-slash`**), `HEAD /v2/`, `OPTIONS /v2/` (**B:`options`**) | P3 T1 |
| 2 | `unknown-route` | paths under `/v2/` that match nothing; reference with a slash (**B:`unknown-route`**); wrong method and `Allow` (**B:`wrong-method`**) | P3 T2 |
| 3 | `names` | 20 names from the P1 T2 table, each as `GET …/tags/list`: status and code per name; 255 and 256 characters (**`name-length`**) | P3 T2 |
| 4 | `errors-read` | `BLOB_UNKNOWN`, `MANIFEST_UNKNOWN`, `NAME_UNKNOWN`, `DIGEST_INVALID`, `TAG_INVALID`: exact message and `detail` (**B:`errors-<code>`**) | P3 T3 |
| 5 | `blob-read` | push one 3 MiB blob, then `GET`, `HEAD`, `Range: bytes=0-0`, `bytes=1048576-`, `bytes=-5`, two ranges, past the end (**B:`blob-range-forms`**), `If-None-Match` | P3 T4 |
| 6 | `manifest-read` | push an image; `GET` and `HEAD` by tag and digest; `Accept` that excludes the stored type (**B:`manifest-accept`**); no `Accept` | P3 T5 |
| 7 | `push-monolithic` | `POST ?digest=` with body; `POST` then `PUT` with body; `Location` form (**B:`location-form`**) | P4 T2 |
| 8 | `push-chunked` | as the example above; plus `PATCH` without `Content-Range`; a gap; an overlap; status `GET` between chunks | P4 T1 |
| 9 | `digest-mismatch` | finish with a wrong digest; then: is the session still there? is the blob there? (**B:`digest-mismatch`**) | P4 T1 |
| 10 | `mount` | hit, miss, no `from` (**B:`mount-no-from`**), `from` a repository that does not exist | P4 T2 |
| 11 | `manifest-put-invalid` | missing layer (`MANIFEST_BLOB_UNKNOWN` and its `detail`), bad JSON, schema 1, no `Content-Type` (**B:`manifest-no-content-type`**), tag and body digest disagree, 4 MiB + 1 (**B:`manifest-too-large`**), index with a missing child (**B:`index-missing-child`**) | P4 T3 |
| 12 | `digest-forms` | `sha512:` (**B:`digest-sha512`**), `blake3:` (expected refused: the FR-022 difference), upper case hex | P1 T2, P3 |

Later scenarios are written in the phase that needs them and listed there: `tags-paging`, `catalog-paging`, `catalog-after-failed-push`, `referrers-present`, `delete-*`, `delete-manifest-tags`, `readonly-mode` (P7); `auth-*`, `auth-missing-htpasswd` (P6); `env-unknown-key`, `version-string` (P5); `gc-dry-run-output` (P8).

- [ ] **Step 1:** Write scenario 1. Run `differential run --left reference --right reference --scenario base`. Expected: exit 0. If not, the normaliser is wrong: fix Task 1, not the scenario.
- [ ] **Step 2:** `differential record --target reference --scenario base --out golden/`. Open `golden/base.json` and **read it**. Each surprise against `contracts/registry-api.md` is corrected in the contract file now, in this commit, with the scenario name as the source.
- [ ] **Step 3:** Repeat for scenarios 2 to 12. One commit per scenario.
- [ ] **Step 4:** For every **B:** cell in `contracts/registry-api.md` and `contracts/config-table.md` covered by these twelve, replace the question with the recorded answer and keep the scenario name beside it.
- [ ] **Step 5:** `differential check-golden --target reference` → exit 0 (the transcripts equal a fresh run). This is what CI runs to detect a drifting reference.

**Done when:** 12 scenarios; reference-vs-reference is zero differences on all; 12 golden files committed; no **B:** cell that these scenarios cover is still a question.

**Traps:**
- A scenario that fails **against the reference alone** is a wrong scenario. `expect_status` on a step exists only to catch that; it is never the product's test.
- Generated bytes must be the same on every run and machine: seed the XOF from `seed` only. No clocks, no randomness.
- Keep blobs small (at most 3 MiB). Large bodies belong to gate D and performance. Twelve scenarios should run in under 90 seconds against two local containers.

---

## Task 3: `apps/registry/DIFFERENCES.md` is the allowlist, and stale lines fail

**Requirements:** FR-013, SC-003. **Files:** create `apps/registry/DIFFERENCES.md` (repository root), `apps/registry/gates/differential/src/allowlist.rs`.

**Format.** Prose for operators on top. The machine part is one table between two markers:

```markdown
<!-- gate-b:begin -->
| id | scenario | step | field | reference | product | reason |
|---|---|---|---|---|---|---|
| D-001 | digest-forms | blake3-head | status | 400 | 404 | Hologram Registry accepts blake3 digests (FR-022). |
<!-- gate-b:end -->
```

A row matches a `Difference` when `scenario`, `step` and `field` are equal and `reference` and `product` equal the observed values. `*` is allowed in `step` only.

- [ ] **Step 1: Failing tests.**

```rust
#[test]
fn an_unlisted_difference_fails() {
    let verdict = judge(&[diff("base", "get", "status", "200", "204")], &allowlist(""));
    assert_eq!(verdict.unlisted.len(), 1);
    assert!(!verdict.passed());
}

#[test]
fn a_listed_difference_passes() {
    let list = allowlist("| D-001 | base | get | status | 200 | 204 | because |");
    assert!(judge(&[diff("base", "get", "status", "200", "204")], &list).passed());
}

#[test]
fn a_listed_difference_that_no_longer_happens_fails_as_stale() {
    let list = allowlist("| D-001 | base | get | status | 200 | 204 | because |");
    let verdict = judge(&[], &list);
    assert_eq!(verdict.stale, ["D-001"]);
    assert!(!verdict.passed(), "the file must never claim a difference that is gone");
}

#[test]
fn a_listed_difference_whose_values_changed_is_both_unlisted_and_stale() {
    let list = allowlist("| D-001 | base | get | status | 200 | 204 | because |");
    let verdict = judge(&[diff("base", "get", "status", "200", "500")], &list);
    assert_eq!((verdict.unlisted.len(), verdict.stale.len()), (1, 1));
}

#[test]
fn a_row_without_a_reason_is_a_parse_error() { /* empty last cell → Err */ }

#[test]
fn stale_rows_are_only_judged_for_scenarios_that_ran() {
    // `--scenario base` must not call rows of other scenarios stale.
}
```

- [ ] **Step 2: Implement; run** → PASS. 
- [ ] **Step 3:** Write the first `apps/registry/DIFFERENCES.md`: a title, two paragraphs for operators (what this file is; how to read a row), the empty table, and a "Notes" section with the one non-behavioural note (upload `Location` carries no `_state`).
- [ ] **Step 4:** Commit.

Who may add a row (CHK009): the engineer whose pull request makes the difference, in that pull request, with a reason a registry operator would accept. A difference that breaks a client on the gate C list is never acceptable; fix the code.

**Done when:** six tests pass; `differential run … --allowlist apps/registry/DIFFERENCES.md` reads the file.

---

## Task 4: Gate C client scripts, green against the reference

**Requirements:** FR-018, SC-004. **Files:** create `apps/registry/gates/clients/lib.sh` and one script per client; `apps/registry/gates/compose/*.yml`.

Each script takes one argument, the image under test, starts it the way the registry's deployment guide does, drives one client, and exits non-zero on any failure. They run against `REGISTRY_REF` now and against the product image from P9.

- [ ] **Step 1: `lib.sh`.**

```bash
#!/usr/bin/env bash
# Shared by every gate C script. Source it; do not run it.
set -euo pipefail
IMAGE_UNDER_TEST="${1:?usage: $0 <image>}"
NAME="gatec-$$-${RANDOM}"
PORT=""

start_registry() {                       # start_registry [docker run args…]
  docker run -d --name "${NAME}" -p 127.0.0.1::5000 "$@" "${IMAGE_UNDER_TEST}" >/dev/null
  PORT=$(docker port "${NAME}" 5000/tcp | head -1 | sed 's/.*://')
  for _ in $(seq 1 50); do
    if curl -fsS "http://127.0.0.1:${PORT}/v2/" >/dev/null 2>&1; then return 0; fi
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/v2/")" == "401" ]]; then return 0; fi
    sleep 0.2
  done
  docker logs "${NAME}" >&2; echo "registry did not answer on ${PORT}" >&2; return 1
}
cleanup() { docker rm -f "${NAME}" >/dev/null 2>&1 || true; }
trap cleanup EXIT

same_digest() {                          # same_digest <ref-a> <ref-b>
  local a b
  a=$(crane digest --insecure "$1"); b=$(crane digest --insecure "$2")
  [[ "${a}" == "${b}" ]] || { echo "digest differs: $1=${a} $2=${b}" >&2; return 1; }
}
```

- [ ] **Step 2: `docker.sh`, in full, as the model for the rest.**

```bash
#!/usr/bin/env bash
# docker: push, pull, and a two-platform buildx push. localhost registries need no insecure-registries entry.
source "$(dirname "$0")/lib.sh"
start_registry
REG="127.0.0.1:${PORT}"

docker pull -q alpine:3.20
docker tag alpine:3.20 "${REG}/gate-c/alpine:3.20"
docker push -q "${REG}/gate-c/alpine:3.20"
docker rmi -f "${REG}/gate-c/alpine:3.20" >/dev/null
docker pull -q "${REG}/gate-c/alpine:3.20"
same_digest "alpine:3.20" "${REG}/gate-c/alpine:3.20" || true    # docker re-tags an index; equality is checked on the manifest below
want=$(docker buildx imagetools inspect alpine:3.20 --format '{{json .Manifest.Digest}}')
got=$(docker buildx imagetools inspect "${REG}/gate-c/alpine:3.20" --format '{{json .Manifest.Digest}}')
[[ "${want}" == "${got}" ]] || { echo "index digest changed: ${want} → ${got}" >&2; exit 1; }

builder="gatec-${RANDOM}"
docker buildx create --name "${builder}" --driver docker-container --driver-opt network=host >/dev/null
trap 'docker buildx rm -f "${builder}" >/dev/null 2>&1 || true; cleanup' EXIT
printf 'FROM alpine:3.20\nRUN echo gate-c > /marker\n' | docker buildx build --builder "${builder}" \
  --platform linux/amd64,linux/arm64 --push -t "${REG}/gate-c/multi:1" -f - .
docker buildx imagetools inspect "${REG}/gate-c/multi:1" | grep -q 'linux/arm64'
echo "gate C docker: ok"
```

- [ ] **Step 3: The rest, same shape.** One line each on what must be asserted:

| Script | Asserts |
|---|---|
| `containerd.sh` | `ctr images pull --plain-http` then `ctr images push`; digests equal |
| `kubernetes.sh` | kind cluster wired by the registry docs' local-registry recipe; a pod from `localhost:5001/gate-c/alpine` reaches `Running` |
| `oras.sh` | `oras push` an artifact with a custom `artifactType`; `oras pull`; `oras discover` lists an attached referrer |
| `crane.sh` | `crane copy` in, `crane digest`, `crane ls`, `crane catalog` |
| `skopeo.sh` | `skopeo copy --all` in and out; `skopeo inspect` digests equal |
| `helm.sh` | `helm package` a one-file chart; `helm push oci://…`; `helm pull`; file hashes equal |
| `cosign.sh` | key pair; `cosign sign`; `cosign verify`; runs twice, with `COSIGN_EXPERIMENTAL_OCI_11=1` (referrers) and without (tag fallback) |
| `ollama.sh` | pinned Ollama version; a model from a 1 KB Modelfile; `ollama push` and `pull`. Over TLS only if plain HTTP needs a flag that changes by version (risk table) |
| `login.sh` | htpasswd variant of the docs' compose file; `docker login` good and bad password; anonymous push refused |
| `tls.sh` | throwaway CA and certificate; registry and a client container on one Docker network; CA installed in the client; `docker push` by DNS name with no `insecure-registries` |
| `compose-swap.sh` | each file in `apps/registry/gates/compose/` is the reference's documented compose file, verbatim; the script changes only the `image:` line with `sed`, runs `docker compose up -d`, times it, pushes and pulls once. Fails over 300 s (SC-001) |
| `service-container.sh` | the image as a GitHub Actions service container (asserted by a job in `gates.yml`, not locally) |
| `surface.sh` | written in P5 (FR-021) |

- [ ] **Step 4:** Run every script with `REGISTRY_REF`. Each must exit 0. A script red against the reference is a wrong script.
- [ ] **Step 5:** Commit, one per script.

**Done when:** 12 scripts exit 0 against the reference on the CI runner.

**Traps:**
- `127.0.0.1:<port>` is treated as insecure-allowed by docker; a hostname is not. `tls.sh` exists to cover the real case.
- Pin every tool version in `apps/registry/gates/clients/VERSIONS` and install from there in CI. A client update must be a reviewed change, not a surprise red.
- `kubernetes.sh` is the slow one (about 4 minutes). It is the first candidate to move to nightly (risk table).

---

## Task 5: The gate D corpus

**Requirements:** SC-005. **Files:** create `apps/registry/gates/corpus/build.sh`, `apps/registry/gates/corpus/corpus.lock`, `apps/registry/gates/corpus/roundtrip.sh`.

Corpus (closes CHK005): a two-platform image; an image with 100 small layers; a Helm chart; a cosign-signed image with its signature; an image with an attached SBOM referrer; an Ollama model; an artifact with an empty config; one 512 MiB layer per commit, one 5 GB layer nightly. `build.sh` builds it into a local reference registry and writes every digest to `corpus.lock`. `roundtrip.sh <peer-a> <under-test> <peer-b>` copies with `skopeo copy --all` and `oras cp -r` (referrers) and fails unless every digest in `corpus.lock` is found at each hop.

- [ ] **Step 1:** `build.sh`; run; commit `corpus.lock`.
- [ ] **Step 2:** `roundtrip.sh reference reference reference` → exit 0.
- [ ] **Step 3:** `roundtrip.sh reference zot reference` (Zot from `ghcr.io/project-zot/zot-linux-amd64`, pinned by digest) → exit 0. This proves the script against a second implementation before ours exists.

**Done when:** both round trips exit 0 and `corpus.lock` is committed.

**Trap:** `skopeo copy` does not copy referrers. `oras cp -r` does. Without it the signature and SBOM rows pass vacuously.

---

## Task 6: `gates.yml`, with the self-test blocking

**Requirements:** FR-017 (mechanism). **Files:** create `.github/workflows/gates.yml`.

- [ ] **Step 1:**

```yaml
name: gates
on:
  push: { branches: [main, 'registry/**'] }
  pull_request:
concurrency: { group: gates-${{ github.ref }}, cancel-in-progress: true }
jobs:
  b-self:                                    # blocking from day 7
    runs-on: ubuntu-24.04
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.97.1
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: apps/registry/gates/differential }
      - run: cargo test --locked --manifest-path apps/registry/gates/differential/Cargo.toml
      - run: cargo run --locked --release --manifest-path apps/registry/gates/differential/Cargo.toml -- run --left reference --right reference
      - run: cargo run --locked --release --manifest-path apps/registry/gates/differential/Cargo.toml -- check-golden --target reference
  c-reference:                               # proves the scripts; becomes gate-c in P9 by changing one variable
    runs-on: ubuntu-24.04
    timeout-minutes: 25
    steps:
      - uses: actions/checkout@v4
      - run: apps/registry/gates/clients/install-tools.sh
      - run: |
          source apps/registry/gates/reference.env
          for script in docker containerd oras crane skopeo helm cosign login tls compose-swap; do
            echo "::group::${script}"; "apps/registry/gates/clients/${script}.sh" "${REGISTRY_REF}"; echo "::endgroup::"
          done
```

- [ ] **Step 2:** Push. Record each job's wall time in `apps/registry/gates/README.md`: this is the measured runtime budget that `plan.md` section 6 assumed.
- [ ] **Step 3:** Mark `b-self` required in the fork's branch protection (the maintainer or a maintainer; the plan does not change repository settings).

**Done when:** `b-self` and `c-reference` are green on `main`; wall times recorded.

**Traps:**
- The two disabled workflows (`model-hub-uptime`, `release-docs`) stay disabled. Do not touch them.
- `kubernetes.sh` and `ollama.sh` are left out of the per-commit loop until P9 measures them.

---

## Phase exit

Reference compared with itself: zero differences over 12 scenarios, in CI, blocking. 12 golden transcripts committed. Every client script green against the reference. Corpus locked; round trip proven through Zot. Meeting on day 7: B walks A through the golden files for scenarios 1 to 6, which are A's fixtures for P3.
