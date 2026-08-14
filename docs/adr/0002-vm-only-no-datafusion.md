# 2. Document VM only — no DataFusion in default build

Date: 2026-08-14

## Status

Accepted

## Context

ShapePort needs to execute Transformation Plans over structured documents (JSON, JSONL, YAML, CSV, Parquet, Arrow IPC). Two implementation strategies were evaluated:

1. **DataFusion** — a full columnar query engine with SQL support, rich optimisation passes, and a large dependency footprint (~30 additional transitive crates, several hundred kilobytes of binary weight).
2. **Document VM** — a pure-Rust tree-walking evaluator that processes `shapeport_core::Value` rows directly, matching the ShapePort Transformation Plan IR operations one-to-one.

The default build of ShapePort targets environments where binary size, compile time, and dependency auditing matter (AI agent runtimes, containers, IDE extensions). DataFusion pulls in `tokio`, `object-store`, and Arrow-derived crates that duplicate what ShapePort already vendors separately.

The 11 plan operations (Project, Rename, Drop, Literal, Cast, Coalesce, Object, Map, Filter, Sort, Explode) are straightforwardly implementable as a tree walk without a columnar representation. The bounded SQL query engine (`shapeport_query`) already uses `sqlparser` directly.

## Decision

The default Cargo feature set does **not** include DataFusion. The document executor in `crates/shapeport-core/src/engine.rs` is a pure-Rust `Value`-level evaluator. A `datafusion` feature flag may be added in a future milestone if full SQL push-down or very large dataset support is required, but it will remain opt-in and behind a feature gate.

## Consequences

- Binary size and compile time remain small.
- No additional auditing surface from DataFusion's transitive closure.
- Plans that require advanced aggregation or window functions not covered by the current 11 operations will need an explicit `datafusion` feature or pre-processing steps.
- The `shapeport_query` path continues to use `sqlparser` for SQL parsing and a row-level evaluator for execution, which is sufficient for the bounded queries in the MCP tool set.
