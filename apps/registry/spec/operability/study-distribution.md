# Study: CNCF Distribution, the server behind Docker Registry

2026-09-21.

**There is no product called "Docker Server".** The server that powers Docker Registry is **CNCF Distribution**: repository `github.com/distribution/distribution`, binary `registry`, image `registry:3` on Docker Hub (published as `distribution/distribution`). Latest release **v3.1.1**, 2026-05-01 [ran: `gh api repos/distribution/distribution/releases/latest`]. Licence **Apache-2.0** [read: `dist:LICENSE`, README badge]. CNCF maturity **Sandbox**, accepted 2021-01-26 [docs: `cncf.io/projects/distribution`]. It is called Distribution below.

Source read: a clone at `5b354e6` (2026-08-31), paths prefixed `dist:`. **Not run**: Docker was not reachable where this study was written [ran]. So image facts come from the Dockerfile at `main`, not from the published image. `001` P2 T1 pins the published image by digest and checks each.

## 1. What it is for, and who runs it

"The goal of this project is to provide a simple, secure, and scalable base for building a large scale registry solution or running a simple private registry" [read: `dist:README.md:17-19`]. Two audiences follow from that sentence:

| Audience | Uses | Evidence |
|---|---|---|
| Platform builders | The Go libraries, or the binary behind their own front end | "core library for … Docker Hub, GitHub Container Registry, GitLab Container Registry, DigitalOcean …, Harbor" [read: `dist:README.md:19-22`] |
| Operators of a private registry | `docker run -d -p 5000:5000 registry:3` and a compose file | the deploying guide [read: `dist:docs/content/about/deploying.md`] |

**Hologram Registry v1 competes only for the second audience.** The first needs a Go library; we are not one.

## 2. Form factor

| Item | Distribution | Source |
|---|---|---|
| Binary | one static Go binary, `registry`, `CGO_ENABLED=0` | `dist:Dockerfile` |
| Commands | `registry serve <config>`, `registry garbage-collect <config> [--dry-run] [--delete-untagged] [--quiet]`, `registry --version` | `dist:registry/root.go:27,48` |
| Image base | `alpine` 3.23 with `ca-certificates`. **It has a shell.** | `dist:Dockerfile` |
| User | root (no `USER` line) | same |
| Entry point, command | `["registry"]`, `["serve", "/etc/distribution/config.yml"]` | same |
| Port, volume | `EXPOSE 5000`, `VOLUME ["/var/lib/registry"]` | same |
| Environment baked in | `OTEL_TRACES_EXPORTER=none` | same |
| Default config in the image | `cmd/registry/config-dev.yml`: log level **debug**; `storage.delete.enabled: **true**`; blob descriptor cache in memory; upload purging **off**; `http.addr :5000`; **`http.debug.addr :5001` with Prometheus on at `/metrics`**; `X-Content-Type-Options: nosniff`; storage health check every 10 s | `dist:cmd/registry/config-dev.yml` |
| Architectures | **7**: `linux/amd64`, `arm/v6`, `arm/v7`, `arm64`, `ppc64le`, `s390x`, `riscv64` | `dist:docker-bake.hcl:58-65` |
| Release artefacts | `registry_<ver>_<os>_<arch>.tar.gz` holding binary, README, LICENSE; a `.sha256` beside each | `dist:Dockerfile`, stage `releaser` |
| Helm chart | none first-party. No mention of Helm in the docs or README [ran: grep]. The common chart is community-run (`twuni/docker-registry`) [memory] | |
| Docs | one Hugo site, `distribution.github.io/distribution`: about (architecture, compatibility, configuration, deploying, garbage collection, insecure, notifications), storage drivers, recipes (Apache, nginx, mirror, systemd, macOS), spec | `dist:docs/content/` |
| Governance | `GOVERNANCE.md`, `MAINTAINERS`, `ADOPTERS.md`, `CODE-OF-CONDUCT.md`, `CONTRIBUTING.md`, OpenSSF Scorecard and FOSSA badges, a conformance badge | `dist:` root |
| Security policy | private list `cncf-distribution-security@lists.cncf.io`. The supported-versions table still says 3.0 "has not yet been released" while 3.1.1 is out: **the policy page is stale** | `dist:SECURITY.md:7-17` |
| Release cadence | 3.0.0 → 3.1.1 across the files in `dist:releases/`; no written support window | `dist:releases/` |
| Roadmap | the file has a title and nothing else | `dist:ROADMAP.md` |

## 3. Every feature, from the configuration reference

The configuration file **is** the feature list. Every top-level key [read: `dist:docs/content/about/configuration.md:84-342`]:

| Area | Features |
|---|---|
| API | OCI Distribution (`/v2/`): pull, push (monolithic, chunked, mount), tags, catalogue, delete, referrers. The project runs the OCI conformance workflow itself |
| `storage` | drivers: `filesystem`, `s3`, `gcs`, `azure`, `inmemory`. Plus `delete.enabled`, `redirect.disable`, `cache.blobdescriptor` (`inmemory` or `redis`), `maintenance.uploadpurging`, `maintenance.readonly`, `tag.concurrencylimit` |
| `middleware` | `cloudfront` and `redirect` storage middleware; hooks for registry and repository middleware |
| `auth` | `htpasswd`, `token` (JWT from an external service: `realm`, `service`, `issuer`, `rootcertbundle`, `jwks`, `signingalgorithms`, `autoredirect`), `silly` (tests) |
| `http` | `addr`, `net`, `prefix`, `host`, `secret`, `relativeurls`, `draintimeout`, `headers`, `http2.disabled`, `h2c.enabled` |
| `http.tls` | `certificate`, `key`, `clientcas`, `clientauth`, `minimumtls`, `ciphersuites`, `letsencrypt` (`cachefile`, `email`, `hosts`, `directoryurl`) |
| `http.debug` | a second listener: `/debug/health`, `/debug/health/down` and `/up` (manual drain), `/debug/vars`, pprof; `prometheus.enabled`, `prometheus.path` [read: `dist:health/doc.go`] |
| `health` | `storagedriver`, `file`, `http`, `tcp` checks with interval and threshold |
| `notifications` | webhook `endpoints` with `headers`, `timeout`, `threshold`, `backoff`, `ignore.mediatypes`, `ignore.actions`; `events.includereferences` |
| `proxy` | pull-through cache of one upstream: `remoteurl`, `username`, `password`, `exec` (credential helper), `ttl` |
| `redis` | shared blob descriptor cache |
| `validation` | `manifests.urls.allow` and `deny`, `manifests.indexes.platforms` |
| `catalog`, `tags` | `maxentries`, `maxtags` paging caps |
| `log` | `level`, `formatter` (`text`, `json`, `logstash`), `fields`, `accesslog.disabled`, mail `hooks` |
| Tracing | OpenTelemetry, configured only by the standard `OTEL_*` variables; exports to `https://localhost:4318/v1/traces` unless told otherwise [read: `configuration.md:59-63`] |
| Overrides | any key by `REGISTRY_<PATH>`; list items by index, `REGISTRY_HTTP_TLS_LETSENCRYPT_HOSTS_0` [read: `configuration.md:40-52`] |

About 120 leaf keys in all.

## 4. How it is operated

| Task | How | Note |
|---|---|---|
| Install | `docker run`, compose, or the tarball under systemd (recipe) | no package, no installer |
| Configure | one YAML file, mounted over the default, or environment | the docs warn against overriding whole sections by environment |
| Secure | TLS files or Let's Encrypt; htpasswd or a token service; commonly behind nginx, Apache or an ingress (recipes) | anonymous by default |
| Observe | logs to stdout; Prometheus on the debug port; OpenTelemetry traces; `/debug/health` | metrics namespace `registry_*` [memory] |
| Reclaim space | stop writes (read-only mode or stop the container), run `garbage-collect`, start again | documented as unsafe beside writes; nothing enforces it |
| Back up | copy the volume, or rely on the object store | no tool, no guide |
| Upgrade | replace the image. 2.x → 3.x changed the config path (`/etc/docker/registry` → `/etc/distribution`), removed drivers (`oss`, `swift`) and middleware [memory] | no upgrade guide in `docs/content/about` |
| Scale out | many replicas over one object store, Redis for the cache, a shared `http.secret` | filesystem driver means one node |

## 5. Top operator complaints

Open issues, sorted by thumbs up [ran: `gh api search/issues`, 2026-09-21]. Link form: `github.com/distribution/distribution/issues/<n>`.

| # | Votes | Complaint | What it tells us |
|---|---|---|---|
| 1431 | 152 | pull-through cache cannot serve private upstream images well | the proxy is the most wanted and least finished feature |
| 1201 | 83 | tags cannot hold semver build metadata (`+`) | a spec limit; not ours to fix |
| 206 | 44 | no search | out of scope for a registry; a catalogue UI's job |
| 2747 | 39 | no API to delete a repository or a tag | OCI 1.1 allows tag delete; cheap to offer |
| 2367 | 38 | proxy: `unexpected EOF` when a cached blob expires mid-pull | a correctness bug in the scheduler |
| 2017 | 37 | copy between registries without pull and push | tools (`skopeo`, `crane`) answer it |
| 1844 | 32 | garbage-collect did not remove untagged manifests | fixed by `--delete-untagged`; still open |
| 2225 | 27 | push reports "blob upload unknown" although data arrived | upload session state across replicas and restarts |
| 3725 | 25 | only one upstream per proxy instance | |
| 1978, 4533 | 17, 11 | token scopes and JWT validation trouble | token auth is where integrations break |
| 1636 | 12 | S3: "blob unknown" after push | eventual consistency in the driver |
| 2314, 1515 | 11, 11 | cannot delete repositories; garbage collection should be an API | **garbage collection needs downtime** |
| 2386 | 10 | a front proxy's HTML 405 reaches the client as a JSON parse error | operators need it to work without a front proxy |
| 1736 | 9 | an environment override for upload purging crashes the start | the environment mapping is fragile |

**Where equivalence is cheap to beat** (each becomes a row in the "better" table of `parity-matrix.md`):

1. Upload sessions that survive a restart (2225).
2. Bytes verified as they are written, and a verify command (1636's class of doubt).
3. Garbage collection that cannot break an image, and refuses to run where it is unsafe (1844, 2094, 1515).
4. Settings that fail loudly and by name (1736).
5. TLS and login that work with no front proxy (2386).
6. Tag delete by API (2747).
7. An OpenAPI document; Distribution has none.
8. A current security policy and a written support window (section 2).

**Where it is expensive**: the proxy (1431, 2367, 3725, 4281) and token authentication (1978, 4533). Both are wanted, both are where Distribution itself is weakest, and both are large. They are the v1.1 headline, not a v1 line item.

## 6. Limits of this study

Not run: the image, so the default file, entry point and ports are from `main`, not from `registry:3` as published. Not read: the token authentication and proxy packages line by line; costs for them in `parity-matrix.md` are estimates. Metric names and the 2.x → 3.x changes are from memory.
