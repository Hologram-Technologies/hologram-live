# ADR 021: Component fetch is host-mediated and bounded

- Status: accepted and implemented
- Date: 2026-09-01

## Context

ADR 020 introduced canonical HTTPS origin/path capability scopes without
linking a network interface. A guest-facing fetch operation now needs a fixed
contract that preserves that fail-closed boundary and cannot become ambient
WASI networking or an SSRF primitive.

## Decision

`hologram:guest/component-network-fetch@1` is a distinct Component profile. Its
world imports exactly `hologram:host/network-fetch@1.0.0`. The single `fetch`
function accepts one canonical endpoint string and returns only an HTTP status
and body. It performs HTTPS GET only. Guests cannot supply methods, headers,
bodies, credentials, cookies, proxy configuration, client identities, DNS
answers, or redirect policy. Query strings and fragments remain outside the v1
canonical endpoint grammar. Announce and listen authority remain separate and
unimplemented.

The provider refuses to construct its linker unless the admitted request
retains at least one `network_fetch_endpoints` scope. Every call reparses the
target with upstream `NetworkEndpointScope` and checks exact origin plus
path-segment containment before entering the transport and again at that
boundary. Public errors and traces never include the target or admitted scope.

The production transport resolves DNS afresh for each operation, removes
loopback, private, link-local, carrier-grade NAT, benchmarking, documentation,
multicast, unspecified, reserved, IPv4-mapped-private, and IPv6 unique-local
destinations, and refuses the operation if none remain. A fresh HTTPS-only
client pins connection attempts to the checked addresses. Environment proxies,
referers, cookies, credentials, connection reuse, and automatic redirects are
disabled. A 3xx response is returned without following `Location`.

Host ceilings are 2 KiB for the canonical target, 1 MiB for the response body,
1.5 seconds for connect/send/read, and eight concurrent mediated fetches across
the process. These are host limits, not guest-controlled budgets. Component
execution retains its independent 2-second invocation deadline and 1 MiB
input/output limits.

## Consequences

- Endpoint capability alone still grants no raw socket, DNS, WASI HTTP, or
  announce/listen interface.
- Direct and resident runtimes register the same fixed profile and share the
  same process-wide limiter and address policy.
- Redirect-based scope changes and DNS rebinding cannot bypass admission.
- V1 cannot express query-bearing APIs, POST requests, streaming bodies, or
  response headers; each requires a later versioned decision.

## Verification

Tests cover the upstream selector, exact WIT import, pre-link authority denial,
canonical path containment, redacted failures, forbidden address families,
host response and concurrency ceilings, a generated Component's successful
mediated call through a deterministic transport, and identical direct/resident
rejection of a private DNS result. The normal workspace, Clippy, BDD, release,
smoke, documentation, and upstream `no_std` gates remain required.
