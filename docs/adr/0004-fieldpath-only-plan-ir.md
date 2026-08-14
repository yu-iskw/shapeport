# 4. FieldPath-only plan IR — no JSONPath or JMESPath

Date: 2026-08-14

## Status

Accepted

## Context

The ShapePort Transformation Plan IR needs a way to reference fields within structured documents. Several path expression languages were considered:

1. **JSONPath** (RFC 9535) — expressive, supports recursive descent and filters, but adds a parsing dependency and allows non-deterministic references that complicate plan validation and fingerprinting.
2. **JMESPath** — powerful projection and filter language, but the reference implementation is not idiomatic Rust and the semantics differ significantly from record-oriented access.
3. **FieldPath** — a simple dot-separated sequence of string segments (`a.b.c`), deterministic, zero external dependency, sufficient for record-level field access, and trivially serialisable.

ShapePort plans operate on record-shaped documents (rows) where each operation targets a specific named field or path. Recursive descent and wildcards are not needed for the current 11 operations (Project, Rename, Drop, Literal, Cast, Coalesce, Object, Map, Filter, Sort, Explode). Plan validation and schema assignability checks benefit from the guarantee that every `FieldPath` refers to exactly one field.

Adding JSONPath or JMESPath would introduce:

- An additional parsing dependency.
- Non-deterministic references (wildcards, filters) that break plan fingerprinting.
- Complexity in the type-checker that must resolve dynamic paths against a schema.

## Decision

The plan IR uses `shapeport_core::path::FieldPath` — a `Vec<String>` of segments serialised as a dot-separated string. All plan operations that reference source or target fields use `FieldPath` exclusively. No JSONPath or JMESPath dependency is added.

If future operations require collection traversal (e.g. flattening nested arrays), a new operation variant (`Explode`, already present) handles the common case without a general path language.

## Consequences

- The plan IR is fully deterministic and fingerprint-stable.
- Schema assignability checks are O(depth) per path with no ambiguity.
- Plans serialise to compact JSON without embedded path expression strings.
- Users and AI agents must reference fields by their exact dot-separated path; wildcard selection is not supported in plans.
- Future requirements for deeply nested or dynamic path access will need a new, scoped IR extension rather than a general path language.
