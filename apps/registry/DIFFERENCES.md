# Differences from the reference registry

Hologram Registry is measured against the Docker Registry image, pinned by digest in
[`gates/reference.env`](gates/reference.env). One scripted session is played against both and compared field by
field: status, the headers that carry meaning, and the body. This file lists every difference that is kept on
purpose, and why. A difference that is not listed here fails the gate. So does a line here that no longer happens.

## For operators

**blake3 digests.** Hologram Registry accepts `blake3:` digests beside `sha256:` and `sha512:`, and serves the same
bytes by either name. The reference refuses them. Clients that never send blake3 see no difference.

**Where the reference fails, we answer.** For a repository name longer than 255 characters, and for a malformed digest
in a manifest path, the reference answers `500 UNKNOWN`. We answer the plain 404 and `400 DIGEST_INVALID`. For a
name of exactly 255 characters the reference answers `NAME_INVALID`; we accept it.

**Monolithic upload.** `POST /v2/<name>/blobs/uploads/?digest=<d>` with the whole blob in the body finishes the upload
in one request (`201`), as the OCI specification describes. The reference ignores the digest and opens an ordinary
session (`202`); a client then finishes it as usual. Both answers are allowed by the specification.

**Several ranges in one request.** A blob `GET` with more than one range is answered with the whole blob (`200`). The
reference answers `206 multipart/byteranges`. No registry client asks for several ranges.

**After a digest mismatch.** When the bytes of an upload do not hash to the digest the client gave, the upload is
discarded: the next status request answers `BLOB_UPLOAD_UNKNOWN`. The reference keeps the session open. A client
starts the upload again in both cases.

**Parse errors in a manifest.** The `detail` of `MANIFEST_INVALID` for a body that is not JSON is our parser's words,
not Go's.

**sha512 is kept as pushed.** A blob finished with a `sha512:` digest is stored and reported under that digest. The
reference rewrites it to the blob's `sha256:` digest and serves it only by that. This one is a debt, not a choice: it
is to be fixed before 1.0.0, and then its rows go.

**The image.** Entry point, command, port, volume and `OTEL_TRACES_EXPORTER=none` equal the reference image's, and
its default `/etc/distribution/config.yml` is the reference's key for key, with one change: `log.level` is `info`,
not `debug`, because debug logging is a development setting. Set `REGISTRY_LOG_LEVEL=debug` for the reference's
behaviour. The image has no shell: `docker exec <c> sh` fails, and `docker run <image> <command>` runs only the
registry's own commands (`serve`, `garbage-collect`, `--version`), where the reference's entry point runs anything.
`hologram` is on the path for operator commands. One registry per volume: the server's lock, pid file and
administration socket live under `<rootdirectory>/live/state/`, so a second container on the same volume refuses to
start, where the reference would run beside it. A volume must hold a Unix socket and honour file locks; a local
disk or a Docker named volume does. The debug listener (`http.debug.addr`, `:5001` in the default file)
serves `/debug/health` (with `/down` and `/up`) and `/metrics`; `/debug/vars` and pprof are Go internals and answer
404. `/metrics` holds the `registry_http_*` request metrics and `hologram_build_info`, and ours beside them: bytes
uploaded and served, digest mismatches, uploads open now, and, read once a minute, the registry's objects, the bytes
under the blob root, the repositories, and the volume's free space (`hologram_disk_free_bytes`). When `http.debug.addr` is the registry's own port (the image's default file puts the debug listener on
`:5001`, and the deployment guide moves the registry there with `REGISTRY_HTTP_ADDR=0.0.0.0:5001`), the debug
listener is not started and the start logs why; the reference starts both and one of the two exits the process.
While any health check fails, `/v2/` answers 503 `UNAVAILABLE`, as the reference's
does: that is what `POST /debug/health/down` drains. The storage check writes, reads back and deletes a small file on
the volume; the reference only stats its root, so a volume that has turned read-only shows here and not there.

**Stopping.** `http.draintimeout` is read as the reference reads it: on `SIGTERM` open requests get that long, then
the process stops without them; without it, the stop is immediate. Where the reference, with no drain set, is killed
by the signal (exit 143), this registry stops cleanly and exits 0 either way.

**TLS.** `http.tls.certificate` and `.key` are read as the reference reads them: a certificate file may hold the whole
chain, and all of it is sent; the key may be PKCS#8, PKCS#1 or SEC1. HTTP/2 is offered. `minimumtls` takes `tls1.2`
(the default) and `tls1.3`; `tls1.0` and `tls1.1` stop the start by name, because this listener does not speak them.
A key without a certificate stops the start, where the reference quietly serves plain HTTP. Where the reference
reads the certificate once, at start, this listener reads it again when either file changes: new handshakes get the
renewed pair within 5 s, with no restart. A pair that does not load (a half-written renewal, a key that is not the
certificate's) leaves the last good one serving, logs one error, and adds to `hologram_tls_reload_failures_total`;
`hologram_tls_certificate_not_after_seconds` on `/metrics` is when the served certificate expires. `letsencrypt`
and client certificates (`clientcas`) are refused by name.

**Garbage collection.** `registry garbage-collect [--dry-run] [--delete-untagged] [--quiet] <config>` marks and sweeps
as the reference does and prints the reference's lines. Two rules are this registry's: it removes only objects it
stored itself (the store keeps small records of its own beside the blobs, which are never garbage), and a signature
or SBOM whose subject is kept is kept too, so `--delete-untagged` never strips an image of its signatures, where the
reference would delete them as untagged manifests. It runs on a stopped registry: beside a running one it refuses
and exits 5.

## The table the gate reads

Keep its two markers and its seven columns. `*` in step, field, reference or product matches anything there; a row
with `*` in the field covers a whole answer that differs by design. Use it only where every field differs; where one field differs, name
that field, so a change to the status or anything else still fails.

<!-- gate-b:begin -->
| id | scenario | step | field | reference | product | reason |
|---|---|---|---|---|---|---|
| D-001 | 12-digest-forms | head-blake3 | * | * | * | blake3 digests are accepted (FR-022); the reference refuses them |
| D-002 | 12-digest-forms | finish-blake3 | * | * | * | blake3 digests are accepted (FR-022); the reference refuses them |
| D-003 | 03-names | name-13 | * | * | * | a name of 255 characters is accepted; the reference answers NAME_INVALID |
| D-004 | 03-names | name-14 | * | * | * | the reference answers 500 for a name over 255 characters; we answer the plain 404 |
| D-005 | 03-names | name-15 | * | * | * | the reference answers 500 for a name over 255 characters; we answer the plain 404 |
| D-006 | 03-names | name-16 | * | * | * | the reference answers 500 for a name over 255 characters; we answer the plain 404 |
| D-007 | 04-errors-read | digest-invalid-manifest | * | * | * | the reference answers 500 for a malformed digest in a manifest path; we answer 400 DIGEST_INVALID |
| D-008 | 05-blob-read | range-two | * | * | * | several ranges are answered with the whole blob; the reference sends multipart/byteranges |
| D-009 | 07-push-monolithic | post-with-digest | * | * | * | a monolithic upload finishes in one request (201); the reference opens a session (202) |
| D-010 | 07-push-monolithic | head-after-post | * | * | * | follows from D-009: the blob exists after the one request |
| D-011 | 09-digest-mismatch | session-after | * | * | * | an upload whose bytes do not match its digest is discarded; the reference keeps the session |
| D-012a | 11-manifest-put-invalid | bad-json | body | * | * | the parse error in detail is our parser's words, not Go's |
| D-012b | 11-manifest-put-invalid | bad-json | header:content-length | * | * | follows from D-012a: the detail's length |
| D-013a | 12-digest-forms | finish-sha512 | header:docker-content-digest | * | * | debt, to fix before 1.0.0: sha512 is kept as pushed; the reference rewrites it to sha256 |
| D-013b | 12-digest-forms | finish-sha512 | header:location | * | * | debt, as D-013a |
| D-014a | 12-digest-forms | head-after-sha512 | header:docker-content-digest | * | * | debt, as D-013a |
| D-014b | 12-digest-forms | head-after-sha512 | header:etag | * | * | debt, as D-013a |
| D-015 | 12-digest-forms | head-after-sha512-by-sha256 | * | * | * | debt, as D-013a: the blob is not found by its sha256 |
<!-- gate-b:end -->
