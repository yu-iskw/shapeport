# ShapePort - Claude Code Memory

## Project Overview

ShapePort is a schema-driven data transformation runtime and MCP server written in Rust.

Codex-specific project guidance lives in `AGENTS.md`. Keep Claude-only workflow details here and under `.claude/`.

- **Build System**: Cargo workspace
- **Linting/Formatting**: Clippy, rustfmt, and Trunk
- **Testing**: `cargo test --workspace --all-features`
- **Security**: GitHub CodeQL and Trunk security linters

## Crate Layout

- `crates/shapeport-core` — schema model, Transformation Plan IR, planner, document VM, format adapters, query engine, application services (library)
- `crates/shapeport-mcp` — MCP 2026-07-28 server built on `rmcp` 3.x, stdio and Streamable HTTP (library)
- `crates/shapeport-cli` — `shapeport` binary, CLI wrapper around core and MCP (binary)

## Quick Commands

```bash
make setup   # fetch Cargo dependencies
make lint    # run Trunk plus strict workspace clippy
make format  # format Rust and repo files
make test    # run workspace tests
make codeql  # run local CodeQL analysis
make build   # build release binaries and libraries
make clean   # remove build artifacts
```

## Rust Guardrails

- Prefer shared versions in `[workspace.dependencies]` over duplicating dependency versions in member crates.
- Each crate must opt into workspace lints with:

```toml
[lints]
workspace = true
```

- Keep `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- Treat Clippy `pedantic`, `cargo`, and `cognitive_complexity` findings as mandatory fixes.
- Refactor functions before they become hard to read; the cognitive complexity threshold is `10`.
- `too-many-arguments-threshold` is `6` — use option structs (e.g. `PlanCmd`, `TransformCmd`) for functions with more parameters.
- Avoid `unsafe` unless there is a documented need and explicit review.

## Testing

- Add crate-local unit tests near the code they cover.
- Add integration tests under `crates/<crate-name>/tests/` when testing public behavior across modules.
- Run `make lint && make test` before committing.
- Use `cargo run -p shapeport-cli` to verify the CLI binary path stays healthy.

## Architecture

- Root `Cargo.toml` defines the workspace and shared dependency versions.
- `crates/shapeport-core` is the reusable library crate.
- `crates/shapeport-mcp` is the MCP server library.
- `crates/shapeport-cli` is the `shapeport` binary.
- `dev/` holds helper scripts for local setup, lint, build, test, and CodeQL flows.
- `.claude/skills/initialize-project/SKILL.md` owns bootstrap-time renaming.
- `docs/adr/` contains Architecture Decision Records.
- `fixtures/` contains test data for integration tests.

## Common Gotchas

- Do not duplicate dependency versions inside member crates when the dependency can live in `[workspace.dependencies]`.
- Keep `Cargo.lock` committed because this workspace includes an executable crate.
- Trunk manages non-Rust repo linters hermetically; do not replace it with ad hoc local installs.
- If a new member crate is added, update workspace membership and ensure it enables workspace lints.
- The MCP server on non-loopback addresses requires `SHAPEPORT_MCP_TOKEN`; loopback is allowed without a token.

## Git Workflow

- Create feature branches from `main`.
- Use conventional commit messages such as `feat(cli): add init command`.
- Run `make lint && make test` before commits.
- Record release notes with the `manage-changelog` skill when that workflow is in use.

## Available Skills

- `initialize-project`: rename the template and its workspace members
- `manage-adr`: maintain architecture decisions in `docs/adr`
- `manage-changelog`: maintain changelog fragments when enabled
- `.claude/skills` remains the canonical skill source even when other agents consume the mirrored tree under `.agents/skills`

## Self-Improvement

- Add or refine Claude rules here when recurring Rust-specific mistakes appear.
- Prefer reusable skills under `.claude/skills/` for workflows that should survive across projects.
