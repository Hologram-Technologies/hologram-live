# ADR 019: Explicit application sessions and user-owned View lifetimes

- Status: accepted; implemented and verified
- Date: 2026-08-31

## Context

Direct `.holo` execution has one-shot semantics: prepare and start every layer,
invoke the root primary once, then stop every layer in reverse order. That is
the right contract for the CLI and for noninteractive Desktop applications,
but it closes a portable View as soon as the primary invocation returns. An
interactive application needs its surface and prepared primary to remain
available across multiple user messages.

Changing `run` to stay resident would make existing callers leak resources and
would blur completion of one invocation with lifetime of an application. A
View is non-exit-bearing and cannot decide application completion by itself.
The lifetime therefore needs an explicit owner and explicit close operation.

## Decision

### Runtime contract

`HoloExecutor` exposes a separate `start_session` family alongside one-shot
`execute`. Starting a session performs the same validation, planning, provider
selection, capability admission, audit, preparation, and transactional start,
but does not invoke the primary and does not stop providers.

An application session exposes immutable archive and application identities,
its lifecycle state, repeated primary invocation, and idempotent `stop`.
Invocation results keep the existing `HoloRunResult` schema. Callers must stop
the session; dropping a running handle is not defined as a successful stop.
One-shot execution is implemented as start, invoke once, and stop, including
rollback diagnostics when both invocation and stop fail.

Every application in a composed plan owns one invocation gate. The root gate
serializes direct session invocations with root View intents. Child View
intents use their child's gate. Stop acquires all gates in deterministic
application order before entering `stopping`, so no provider is detached while
an admitted invocation is in flight. New invocations observe the stopped state
or an unavailable attachment and fail closed.

### Desktop ownership

Hologram Desktop owns application sessions started from its Applications
screen. A portable View uses **Open application**, not the one-shot Run form.
It opens as its own native application window outside the dashboard and stays
open until the user chooses **Stop application**, closes that window, or quits
Hologram Desktop. The dashboard is a launcher and session manager; it is not a
container for guest View content.

Desktop keeps an in-memory registry keyed by opaque session id, archive κ, and
the window labels attached for the application. Starting an already-open
archive is idempotent. Stop removes the registry entry, performs reverse
provider shutdown, and restores the entry if shutdown fails so the user can
retry. Closing a dynamic View window is allowed to proceed and asynchronously
stops its owning session. Only closing the main dashboard window is converted
to hide. Quit attempts to stop every session before exiting.

The Desktop publishes session lifecycle events so the Applications inspector
can switch between Open and Stop state. Sessions are process-local and are not
restored across Desktop restarts. Non-View applications retain the existing
one-shot Run form.

The portable View/provider boundary remains host-neutral. Tauri is the first
host, not part of the `.holo` format. A future browser or server host may
publish its own admitted portable surface and own sessions under the same
runtime contract; headless CLI/server execution continues to reject View
applications when no surface is configured.

## Consequences

- One-shot CLI behavior and invocation completion remain unchanged.
- Interactive Views can issue multiple bounded messages without recompiling or
  restarting their primary layer.
- A View window has a visible user-owned lifetime separate from one invocation.
- Desktop shutdown and window-close paths become lifecycle operations and may
  report provider cleanup failures.
- Multiple simultaneous sessions for the same canonical application are
  intentionally not supported in this slice, even if physically different
  archives carry it, because View attachment identities address the same
  application/layer surface.
- Durable/background sessions, restoration, detached server-hosted surfaces,
  and multi-window View manifests remain follow-up work.

## Verification

Core tests compile the checked-in Wasm + View example, start an explicit
session, observe one attachment, invoke its primary directly and through the
bounded View intent, prove the View remains attached, then stop twice and
observe exactly one detach. The pre-existing one-shot executor test continues
to prove start/invoke/stop behavior.

Desktop tests and builds verify session command serialization, independent
dynamic-window close behavior, reverse shutdown on explicit stop and quit, and
frontend state changes between Open and Stop. The full repository and packaged
Desktop gates remain required before this decision is marked implemented.

## Follow-up

- Publish a browser/server portable-surface adapter only with an authenticated
  session owner and an origin-isolated asset/intent transport.
- Define durable restoration separately; do not infer it from catalog or watch
  state.
- Define multi-window attachment identity and focus/show behavior before
  allowing more than one session for an application archive.
