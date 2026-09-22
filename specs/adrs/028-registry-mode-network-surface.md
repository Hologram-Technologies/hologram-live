# ADR 028: Registry mode serves /v2/ in public and administration on a socket

- Status: accepted
- Date: 2026-09-22
- Requirements: FR-015, FR-021 (`apps/registry/spec/v1`), superseded in part by FR-S06 and FR-S07
  (`apps/registry/spec/operability`)

## Context

The reference registry is a public anonymous service by default: `docker run -p 5000:5000 registry:3` answers
anyone. A drop-in replacement has to do the same, or the deployment guide's first compose file fails.

This server, by default, puts everything on one listener: the module API under `/api/v1/`, the public pages, and
gRPC, which carries `shutdown`. `validate()` therefore refuses a non-loopback listener without `auth.required`,
and refuses the registry module on a non-loopback listener at all, because `/v2/` has no login yet. Both rules
are right for that listener. Neither can be met by a public registry.

The first plan put administration on `127.0.0.1:5001`. The reference's own default configuration uses `:5001`
for its debug and metrics listener, so that plan would have collided with the file our image ships.

## Decision

`serve --registry-config <file>` (and so `registry serve <file>` and the image's entry point) sets
`AppConfig::registry_mode`. It is never read from a file. In registry mode:

| Listener | Carries |
|---|---|
| `http.addr`, default `:5000` | `/v2`, `/v2/…`, `/`, `/healthz`, `/openapi.json`, `/docs`, `/docs/scalar.js`. Nothing else: any other path is a 404, and gRPC is answered `UNIMPLEMENTED`. `/` names only these, and `/openapi.json` describes only `/v2/` and `/healthz` |
| `<root>/live/state/admin.sock`, a Unix socket, mode 0600, in a directory of mode 0700 | the module API and gRPC, `shutdown` included |

Only the system and registry modules run, plugins are off, and the inference engine is `echo`. `validate()`
checks these instead of the loopback rules, which stay exactly as they were in every other mode.

No token guards the socket: its file mode does, as FR-S06 says. `docker exec` reaches it; the network cannot.
A socket left by a stopped server is replaced (the process lock proves no server owns it); anything else at the
path stops the start. A path too long for a Unix socket stops the start and names the path. The socket is
bound before the public port, so a public port that accepts means administration is up, and it is removed when the
server stops or fails to start. Both listeners drain on the one shutdown signal, and either failing ends the
process with its error.

## Consequences

- A public anonymous registry is possible in registry mode and only there. It exposes what the reference
  exposes, plus the server's own public pages.
- `hologram` commands that dial the TCP endpoint (`status`, `stop`) do not reach a registry-mode server. The
  in-container operator commands (`hologram oci …`, plan P8) dial the socket.
- On Windows the named pipe is not built yet: registry mode there serves no administration at all and logs a
  warning saying so. The image is Linux; Windows registry mode is for development.
- Port 5001 stays free for `http.debug.addr` (plan P6 T5).
