# ShapePort — Codex Project Guide

## Purpose

This repository includes Codex as a lightweight, repo-local collaborator for ShapePort development. Use this file as the Codex-facing source of truth for project conventions, then follow the existing project scripts and checks instead of inventing parallel automation.

## Project Shape

- Root `Cargo.toml` defines the Cargo workspace, shared dependency versions, and workspace lint policy.
- `crates/shapeport-core` is the reusable library: schema model, Transformation Plan IR, planner, document VM, format adapters (JSON, JSONL, YAML, CSV, TSV, Parquet, Arrow IPC), SQL query engine, and application services.
- `crates/shapeport-mcp` is the MCP 2026-07-28 server library built on `rmcp` 3.x, supporting stdio and stateless Streamable HTTP transports.
- `crates/shapeport-cli` is the `shapeport` binary — CLI wrapping core app services and the MCP server.
- `dev/` contains the project scripts for setup, lint, format, test, build, clean, and local CodeQL analysis.
- `.trunk/trunk.yaml` defines repository-wide linting for Rust and non-Rust files.
- `docs/adr/` contains Architecture Decision Records.
- `fixtures/` contains test data used by integration tests.

## Required Verification

Use the project entrypoints that already exist:

```bash
make lint
make test
make build
```

Before finishing substantial code changes, run at least `make lint && make test`. Use `make build` when changes affect crate wiring, binary behavior, or release artifacts.

## Rust Guardrails

- Prefer shared versions in `[workspace.dependencies]` over duplicating versions in member crates.
- Keep crate lint opt-in enabled with:

```toml
[lints]
workspace = true
```

- Keep `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- Treat workspace Clippy `all`, `cargo`, and `pedantic` findings as mandatory fixes.
- The workspace forbids `unsafe` code and denies warnings in `[workspace.lints.rust]`.
- Refactor code before it becomes hard to read; the Clippy cognitive complexity threshold is `10`.
- `too-many-arguments-threshold` is `6` in `clippy.toml`; use option structs for functions that exceed this.

## Editing Expectations

- Update the root `Cargo.toml` first when adding shared dependencies or changing workspace-wide lint policy.
- Do not duplicate dependency versions inside member crates when the dependency can live in `[workspace.dependencies]`.
- Keep `Cargo.lock` committed because this workspace includes an executable crate.
- If you add a new member crate, update workspace membership and ensure the crate enables workspace lints.
- Reuse `make` targets and `dev/` scripts instead of adding one-off verification commands to documentation.

## Security Model

- `serve_http` on a non-loopback address requires `SHAPEPORT_MCP_TOKEN` to be set.
- Binding to loopback (`127.0.0.1`) does not require a token.
- Set `SHAPEPORT_MCP_ORIGIN_ALLOWLIST` (comma-separated) to restrict allowed `Origin` headers.
- All non-loopback requests must carry `Authorization: Bearer <token>`.

## Claude Coexistence

- Existing files under `.claude/` are Claude Code specific.
- Do not assume Claude hooks, settings, plugins, or agent definitions apply to Codex.
- Keep Codex guidance in this file and keep Claude-specific operating details in `CLAUDE.md` and `.claude/`.
- Shared skill discovery for non-Claude agents lives under `.agents/skills`, which mirrors top-level directories from `.claude/skills` with symlinks.
- Treat `.claude/skills` as the canonical source of truth and edit skills there rather than under `.agents/skills`.
