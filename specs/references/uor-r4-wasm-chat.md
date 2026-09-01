# Reference project: UOR-R4 WASM Chat

## Reference status

- Purpose: product and interaction reference for Hologram Desktop Chat and
  future chat-oriented portable Views
- Upstream: [Casey-allard/uor-r4-wasm-chat](https://github.com/Casey-allard/uor-r4-wasm-chat)
- Reviewed revision: [`e7f4b698d1b6724ec6f640e2368de05b48c42832`](https://github.com/Casey-allard/uor-r4-wasm-chat/tree/e7f4b698d1b6724ec6f640e2368de05b48c42832)
- Reviewed: 2026-09-01
- License: [MIT](https://github.com/Casey-allard/uor-r4-wasm-chat/blob/e7f4b698d1b6724ec6f640e2368de05b48c42832/LICENSE)
- Visual reference: [v3.1 studio screenshot](https://github.com/Casey-allard/uor-r4-wasm-chat/blob/e7f4b698d1b6724ec6f640e2368de05b48c42832/assets/images/sovereign_studio_v31.png)
- Live demonstration: [UOR-R4 Sovereign Studio](https://casey-allard.github.io/uor-r4-wasm-chat/)
- Hologram dependency: none

This is prior art, not a normative Hologram contract or an endorsed source of
runtime, security, inference, or mathematical claims. No upstream code or
assets are vendored. Revisit the pinned revision before relying on behavior
that is not captured here.

## Why it is useful

The project provides a working, high-density chat and development window rather
than a static mockup. Its visible shell combines a searchable conversation rail,
a bounded central transcript, a persistent composer, workspace modes, and a
collapsible telemetry rail. Its source also demonstrates explicit loading,
streaming, completion, error, stop, cache, and storage states around an
in-browser model worker.

Hologram already has the stronger foundation for durable chat: conversations
are service-backed, independently addressable, and resumable instead of being
browser-local state. The reference is most valuable for presentation and
interaction details that can sit on top of that boundary.

## Patterns to retain or evaluate

| Area | Lesson from the reference | Hologram disposition |
| --- | --- | --- |
| Information architecture | Keep conversation search and creation in a narrow left rail, a readable-width transcript in the center, and the composer anchored near the bottom. | Retain Hologram's existing durable thread/search layout and use this as a density and responsive-layout comparison. |
| Request lifecycle | Show distinct loading or compilation, first-token, streaming, stopped, completed, and failed states; correlate worker events with a request id. | Define these as typed service/provider events before adding token streaming and an explicit Stop action. |
| Composer | Auto-grow to a bound, use Enter to send and Shift+Enter for a newline, display removable attachment chips, and keep the selected model close to the prompt. | Preserve the keyboard behavior; model selection must resolve a cataloged/admitted provider, and attachments should be bounded κ-addressed objects rather than ambient file access. |
| Responses | Render readable Markdown, math, and labeled code blocks with focused copy or editor actions. | Add only with bounded input, sanitization, a restrictive CSP, safe links, accessible controls, and tests for hostile model output. Plain text remains the safe fallback. |
| Progress and readiness | Surface model download/cache size, compilation progress, active model, and storage pressure where the user makes a selection. | Source every status from typed Live model/provider APIs. Do not infer readiness from decorative client state. |
| Diagnostics | A collapsible right rail can expose generation rate and provider telemetry without shrinking the default reading surface. | Keep diagnostics optional and evidence-backed. Do not present synthetic geometric or model telemetry as runtime truth. |
| Empty states | Starter prompts explain the available workflows better than an empty transcript. | Tailor starters to capabilities actually reported by the selected Hologram model/application. |
| Code workflow | Per-code-block Copy, Edit, Diff, and Preview actions make generated code actionable. | Treat editor/preview integration as a separate trusted Desktop feature or a capability-scoped `.holo` application, never as ambient authority granted to model output. |

## Boundaries not to copy

- Do not collapse the trusted Hologram Desktop dashboard and an untrusted guest
  View into one DOM or authority domain. [ADR 018](../adrs/018-portable-view-bundle-and-surface.md)
  and [ADR 019](../adrs/019-explicit-application-sessions.md) continue to
  require opaque origins, separate application windows, bounded intents, and
  explicit session ownership.
- Do not expose GitHub personal access tokens, arbitrary filesystem handles,
  arbitrary network fetches, or remote CDN imports to a chat View. Hologram
  must mediate those operations through explicit capabilities and trusted host
  adapters.
- Do not inject model-produced Markdown or HTML with `innerHTML` unless it has
  passed a deliberately selected and tested sanitizer. Escaping code spans is
  not a complete HTML security boundary.
- Do not couple the Desktop UI directly to one browser worker or inference
  library. Model residency, streaming, cancellation, usage, and failures belong
  to the existing host-neutral inference boundary.
- Do not make the telemetry sidecar mandatory. On smaller windows the
  conversation, response, and composer remain the primary experience.

## Follow-up questions

- What is the minimal typed event envelope for queued, preparing, streaming,
  completed, stopped, and failed chat turns?
- Should a stopped partial answer be durable, and if so, how is its terminal
  status represented in conversation history?
- Which response subset ships first: CommonMark, fenced code, copy actions,
  math, or a smaller safe renderer?
- How do file-store objects become explicit per-turn attachments with size,
  media-type, and model-context limits?
- Which provider metrics are stable and meaningful enough for an optional
  diagnostics rail?
