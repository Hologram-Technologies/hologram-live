# ADR 027: Repository links, referrers and aliases live in a database of our own

- Status: proposed
- Date: 2026-09-21

## Context

The Kappa blob store is global: a blob exists once, whoever pushed it. The registry protocol is scoped: a blob or
manifest exists in a repository only if it was pushed or mounted there, `_catalog` lists repositories, and a delete
in one repository must not affect another. The store has no such notion, and its own garbage collection treats one
namespace's references as the whole truth and deletes the rest.

## Decision

`src/oci_store/links.rs` owns one redb file, `<root>/oci/links.redb`, with six tables: `links`, `repos`,
`referrers`, `aliases`, `uploads`, `meta`. Keys are strings, because redb orders `&str` byte-wise, which is the
lexical order the tag and catalogue routes must page in. A composite key is `"<repo>\0<digest>"`: a NUL cannot appear
in a repository name and sorts before every legal byte, so the range `"<repo>\0".."<repo>\u{1}"` is exactly one
repository.

Garbage collection is ours and reads its mark set from this file. The Kappa store's sweep is never called.

## Measurement

One repository with 100,000 links, written in one transaction (debug build, Windows 11, NVMe): a page of 1,000 links
in 1.8 ms; a single `link_get` in 9.6 µs. The budget was 50 ms and 1 ms.

## Alternatives considered

**The Kappa store's `meta_set` and `meta_query`.** Rejected. Each call is its own transaction, so a manifest `PUT`
(link, referrer row, repository row) could not be atomic; `meta_query` returns addresses keyed by one namespace, so it
cannot list repositories; and it would tie the registry's correctness to a table whose semantics upstream has not
fixed.

**Edges in the Kappa store.** Rejected for the same atomicity reason, and because edges feed the sweep that must never
run.

## Consequences

- One more file to back up with the volume. `hologram oci adopt` can rebuild it from the store's tags and manifests.
- redb allows one write transaction at a time. Blob I/O therefore stays outside transactions: link rows are written
  after bytes are safe, never around them.
