# Contract: drop-in configuration

Every setting the reference registry documents, and what Hologram Registry v1 does with it. One table in code (`src/registry_compat/table.rs`) is the single source; the test `registry_compat::tests::walks_every_documented_key` reads the key list extracted from the pinned registry's `configuration.md` (checked into `tests/fixtures/registry-config-keys.txt` by P5 Task 1 step 1) and fails on any key the table does not classify.

The key list below is **[memory]**. The fixture file, made from the real document, is the authority. A key in the fixture that is missing here fails the test, which is the point.

Classes:
- **S** supported: changes our behaviour the same way.
- **I** accepted and ignored: cannot change what a client sees. Logged once at start at `info`.
- **R** refused: the server does not start; the message names the key and the reason.

Environment form: `REGISTRY_` + path, upper case, `_` between levels (`storage.delete.enabled` → `REGISTRY_STORAGE_DELETE_ENABLED`). Environment overrides the file. An environment variable that starts with `REGISTRY_` and maps to no documented key is **R** (the reference ignores it; we refuse because a typo in a security setting must not pass silently: listed in `apps/registry/DIFFERENCES.md`). **B:`env-unknown-key`** records what the reference does.

| Key | Class | Maps to / reason |
|---|---|---|
| `version` | S | must be `0.1`; anything else R |
| `log.level` | S | `tracing.level` (`error`, `warn`, `info`, `debug`) |
| `log.formatter` | S | `text` and `json` map to the existing formats; `logstash` R |
| `log.fields` | I | |
| `log.accesslog.disabled` | S | turns off the per-request `info` line |
| `log.hooks` | R | mail hooks are not built |
| `loglevel` (deprecated) | S | as `log.level` |
| `storage.filesystem.rootdirectory` | S | `paths.data_dir`; default `/var/lib/registry` |
| `storage.filesystem.maxthreads` | I | |
| `storage.inmemory` | R | no such driver |
| `storage.s3`, `storage.gcs`, `storage.azure` (any subkey) | R | "stores on the local filesystem only" |
| `storage.delete.enabled` | S | route 7 and 10 on or off; default off |
| `storage.redirect.disable` | I | there is nothing to redirect to |
| `storage.cache.blobdescriptor` | I | `inmemory` is in the reference's default file; `redis` R |
| `storage.cache.blobdescriptorsize` | I | |
| `storage.maintenance.uploadpurging.enabled` / `.age` / `.interval` / `.dryrun` | S | drives upload session expiry (`data-model.md`); defaults 168h, 24h |
| `storage.maintenance.readonly.enabled` | S | every write route answers 405 `UNSUPPORTED`: **B:`readonly-mode`** |
| `storage.tag.concurrencylimit` | I | |
| `auth.htpasswd.realm`, `auth.htpasswd.path` | S | P6 |
| `auth.token` (any subkey) | R | token login is out of scope |
| `auth.silly` | R | test-only in the reference |
| `middleware` (any) | R | |
| `http.addr` | S | `server.listen`; `:5000` means `0.0.0.0:5000` |
| `http.net` | S | `tcp` only; `unix` R |
| `http.prefix` | R | changes every URL; not built |
| `http.host` | S | absolute `Location` headers use it |
| `http.relativeurls` | S | `Location` form |
| `http.secret` | I | upload state is server side here; listed in `apps/registry/DIFFERENCES.md` |
| `http.draintimeout` | S | `server.graceful_shutdown_secs` |
| `http.tls.certificate`, `http.tls.key` | S | P6 |
| `http.tls.minimumtls` | S | `tls1.2` (default) or `tls1.3`; `tls1.0`, `tls1.1` R |
| `http.tls.ciphersuites` | R | rustls chooses; refusing beats pretending |
| `http.tls.clientcas`, `http.tls.clientauth` | R | mutual TLS is not built |
| `http.tls.letsencrypt` (any) | R | out of scope |
| `http.debug.addr`, `http.debug.prometheus` | R | metrics port is out of scope. Refused, not ignored: an operator who set it expects to scrape it |
| `http.headers` | S | added to every `/v2/` response |
| `http.http2.disabled` | S | |
| `http.h2c.enabled` | I | h2c already works on the plain listener (gRPC needs it) |
| `notifications` (any) | R | webhooks are out of scope |
| `redis` (any) | R | |
| `health.storagedriver` (any) | I | in the reference's default file |
| `health.file`, `health.http`, `health.tcp` | R | they change what `/debug/health` reports, which we do not serve |
| `proxy` (any) | R | pull-through mirror is out of scope |
| `validation.disabled`, `validation.manifests.urls`, `validation.manifests.indexes` | R | changes what a manifest `PUT` accepts |
| `catalog.maxentries` | S | cap for `n` on routes 2 and 3; default 1000 |
| `policy`, `compatibility`, `reporting` (v2 leftovers) | R | removed in v3; an operator carrying them has a v2 file |

Error text, fixed form, asserted by the walk test:

```
unsupported registry setting: storage.s3.bucket (Hologram Registry v1 stores on the local filesystem only). See apps/registry/DIFFERENCES.md.
```

Exit code 2, as every `LiveError::Config` (`src/main.rs:39`).

## Command names

The image ships one binary under two names: `/usr/local/bin/hologram` and `/bin/registry` (a hard link made in the Dockerfile). When `argv[0]` ends in `registry` (or `registry.exe`), before clap runs, `main` rewrites the arguments:

| Typed | Runs as |
|---|---|
| `registry serve <config.yml>` | `hologram serve --registry-config <config.yml>` |
| `registry garbage-collect [--dry-run] [--delete-untagged] [--quiet] <config.yml>` | `hologram oci garbage-collect … --registry-config <config.yml>` |
| `registry --version`, `registry -v` | prints `registry github.com/distribution/distribution/v3 <our version> (hologram)`: **B:`version-string`** is informational only |
| `registry` anything else | exit 2, names the unsupported command |

`hologram registry …` (R16) stays what it is today: the provider subcommand. The new operator commands live under `hologram oci …` to avoid the clash: `hologram oci verify`, `hologram oci import <dir>`, `hologram oci adopt`, `hologram oci garbage-collect`.

Image metadata, equal to the reference's (D1): `EXPOSE 5000`, `VOLUME ["/var/lib/registry"]`, `ENTRYPOINT ["registry"]`, `CMD ["serve", "/etc/distribution/config.yml"]`, and a default `/etc/distribution/config.yml` equal to D2. A compose file that overrides the config path at `/etc/docker/registry/config.yml` (the v2 location) still works because the path is an argument.
