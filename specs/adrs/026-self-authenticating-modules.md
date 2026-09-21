# ADR 026: A module may authenticate itself, and a module may be opt-in

- Status: proposed
- Date: 2026-09-21

## Context

The server wraps every module's routes in one bearer layer. A request without the configured token is answered
`401` in the daemon's envelope (`LIVE_AUTHENTICATION_FAILED`) before any module sees it.

The registry API cannot live under that layer. Its clients begin with an unauthenticated `GET /v2/` and read the
answer: `200 {}` means no login is needed, `401` with `WWW-Authenticate: Basic realm=…` means log in. Its errors have
their own shape (`{"errors":[{"code":…}]}`) and every response carries `Docker-Distribution-API-Version`. A Docker or
containerd client that meets the daemon's `401` cannot log in, because the challenge it needs is absent.

Separately, every module in the catalogue was enabled by default, because the default configuration is derived from
the catalogue. The registry module opens a volume and owns a lock on it; it must exist in the binary without running
in a stock install.

## Decision

Two small seams in the module host, and nothing registry-specific in it.

1. `LiveModule::authenticates_itself(&self) -> bool`, default `false`. `ModuleRegistry::routers()` returns the
   enabled modules' routes in two routers: `protected`, which the server wraps in the bearer layer as before, and
   `open`, which it merges beside that layer. `ModuleRegistry::router()` still returns both merged, so existing
   callers compile unchanged.
2. `builtin_modules!` takes two lists. `default` is what it was. `opt_in` modules are in `builtins()` and
   `builtin_ids()` and absent from `default_builtin_ids()`, so they resolve when the configuration names them and
   are off otherwise. An entry may carry a `#[cfg(feature = …)]`.

A module that authenticates itself owns what `authenticate` did for it: the request id (`server::next_request_id`,
the same counter) and the `live.server.request` span. The registry module does both in one layer of its own, which
also stamps the version header.

The module router has no `.fallback(…)`. tonic's router carries one, and two fallbacks collide on merge. The module
registers `/v2`, `/v2/` and `/v2/{*rest}`, which leaves nothing under `/v2/` for a fallback to catch.

## Consequences

- With the module off, `/v2/` is the daemon's `404 LIVE_NOT_FOUND`, as before. A stock build and a stock
  configuration are unchanged: the test `default_config_enables_the_builtin_module_catalogue` passes unmodified.
- With the registry module on and `auth.required = true`, `GET /v2/` answers without a token while `/api/v1/modules`
  still answers `401`. The registry's pull request holds both through the real binary (`tests/oci_http.rs`).
- Until the registry's own login lands (P6), an enabled registry is open to whoever can reach the port. The server
  already refuses a non-loopback listener without `auth.required`; the registry image sets its own rule in P5.
- A plugin id is now checked against every builtin id, opt-in ones included.
- Both seams are generic and are offered to the parent as their own pull request, before the registry code.

## Alternatives considered

**Teach `authenticate` about `/v2/`.** Rejected: the server would know a module's paths and protocol, and the next
protocol with its own challenge would add another branch.

**A second listener for the registry.** Rejected for v1: the reference is one port, and the product promise is that
the port and the settings do not change. One process with two ports is also two things to secure.

**Make the registry a plugin process.** Rejected: blobs would cross a process boundary per request, and the store
lock, shutdown and tracing would each need a second implementation.
