# ADR 005: Subprocess plugin boundary

## Status

Accepted.

## Context

ADR 001 made `hologram-live` a module host with v1 modules statically linked and trusted. Third-party modules cannot meet that bar: they ship as independently built binaries, are not audited with the daemon, and must not be able to take the host down or read its state. The dynamic-module seam therefore needs a process boundary with a narrow, versioned contract.

## Decision

Third-party modules run as **supervised subprocess plugins** speaking the `hologram.live.plugin.v1` gRPC contract (`Describe`/`Invoke`/`Ping`) over a **Unix domain socket** under the daemon state directory.

- **Explicit allowlist only.** `[plugins] enabled = false` by default; `[[plugins.modules]]` entries pin `{ id, path, sha256 }`. There is no directory scanning. The sha256 is re-verified before **every** spawn, so an executable swapped on disk after configuration is never executed. Plugin ids must not collide with builtin module ids.
- **One Kameo supervisor actor per plugin** owns the child handle and the tonic client, restarts the child with capped backoff (three attempts) on transport failure, and health-checks with `Ping` as part of every (re)connect handshake.
- **Scrubbed environment.** The child receives exactly one variable, `HOLOGRAM_PLUGIN_SOCKET`; no shell is involved in the spawn.
- **No host capabilities in v1.** Plugins receive no object store, config, or network mediation — they are pure compute over their JSON input. Plugin HTTP routes are likewise out of scope; plugin operations are reachable through the native `plugin.call` / `plugin.list` operations and are merged into the capability manifest so clients can discover them.
- **Auditing is structural.** `plugin.call` is a mutation flowing through the standard dispatch path, so invocations land in the existing audit log with the plugin id as resource.

## Consequences

- A crashed or hung plugin degrades to typed errors (`LIVE_TRANSPORT_UNAVAILABLE` after bounded restarts) instead of daemon failure; unknown, disabled, or undeclared operations keep `LIVE_CAPABILITY_MISSING` semantics.
- The transport is unix-only; other platforms build an empty plugin registry and degrade to `LIVE_CAPABILITY_MISSING`.
- The process boundary alone is not a sandbox: a malicious allowlisted plugin can still act as the daemon's user. The sha256 allowlist governs *which binary* runs, not what it does.
- UDS path length limits (104 bytes on macOS) bound how deep the state directory can live; socket file names are content-hashed to stay short.

## Hardening path

- Wrap plugin executables in `mvm` microVMs (the same boundary planned for `rootfs` layers) so plugins execute without direct host syscalls.
- Introduce scoped host capabilities (per-plugin grants to read store objects, call inference, etc.) negotiated through `Describe` once the isolation story makes them safe to expose.
