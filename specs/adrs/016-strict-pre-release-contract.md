# ADR 016: Strict pre-release contract

- Status: accepted
- Date: 2026-08-26
- Supersedes: compatibility portions of ADRs 006–011

## Context

Hologram Live has no deployed users or published artifacts that require format
compatibility. Carrying speculative migrations and decode defaults makes the
current contract ambiguous, hides malformed inputs, and increases the number of
paths that every compiler, runtime, client, and test must maintain.

## Decision

Live supports one complete current contract:

- physical `.holo` version 4 only;
- exactly one verified application-directory extension for every archive with
  an `AppManifest`;
- source-manifest schema version 4 only;
- explicit Wasm `entry` and canonical `contract` tags;
- canonical `CapabilitySet` objects only, including the canonical empty set;
- Python rootfs bundle schema version 3 only, using ADR 017's normalized Docker
  archive;
- configuration schema version 2 only; and
- complete history, resident, and run records, including typed completion and
  authorization evidence.

Readers reject missing fields, older version numbers, empty Wasm contract tags,
zero-byte capability sentinels, and absent application directories. Startup
does not rewrite configuration. RPC decoding does not synthesize unknown state,
completion, or authorization values.

OpenAI- and Ollama-compatible HTTP endpoints remain supported integrations.
Their presence is independent of internal format compatibility.

## Consequences

- There is one code path and one set of fixtures to test before the first public
  release.
- Invalid or incomplete data fails close to its boundary with a typed error.
- When a real compatibility requirement appears, it must be introduced by a
  deliberate versioned migration with fixtures from an actually shipped
  release.
- Git history remains the record of discarded experimental encodings; the
  runtime does not carry them.
