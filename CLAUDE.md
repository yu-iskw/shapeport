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
make setup              # fetch Cargo dependencies; install cargo tools
make format             # rustfmt + Trunk repo formatters
make lint               # Trunk, cargo check, Clippy, cargo-shear, cargo-deny
make check-features     # cargo hack --each-feature across the workspace
make test               # cargo test --workspace --all-features
make coverage           # cargo-llvm-cov report
make analyze-complexity # Debtmap cyclomatic complexity analysis
make deep-analysis      # Miri + cargo-udeps (requires nightly toolchain)
make build              # release binaries and libraries
make semver-checks      # cargo-semver-checks (run on main/release or with a baseline)
make codeql             # local CodeQL analysis
make clean              # remove build artifacts
```

Run the appropriate tier before completing work:

- **Ordinary changes:** `make lint && make test`
- **Significant changes** (new modules, refactors, dependency additions): also `make check-features && make analyze-complexity`
- **Unsafe or high-risk changes:** also `make deep-analysis`

See `docs/quality.md` for the full quality-assurance architecture.

## Rust Guardrails

- Prefer shared versions in `[workspace.dependencies]` over duplicating dependency versions in member crates.
- Each crate must opt into workspace lints with:

```toml
[lints]
workspace = true
```

- Keep `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- Treat Clippy `pedantic`, `cargo`, and `cognitive_complexity` findings as mandatory fixes.
- Refactor functions before they become hard to read; the cognitive complexity threshold is `10` (`clippy.toml`).
- `too-many-arguments-threshold` is `6` — use option structs (e.g. `PlanCmd`, `TransformCmd`) for functions with more parameters.
- `too-many-lines-threshold` is `100`; `type-complexity-threshold` is `250`.
- `unsafe_code = forbid`. If unsafe becomes necessary: isolate it, add `// SAFETY:` comments, run Miri.

### Lint suppressions

- Do not suppress lint failures merely to make CI green.
- Refactor complexity violations first; split functions, introduce named types.
- When suppression is genuinely necessary, use a scoped `#[allow(...)]` with a comment explaining why the complexity is inherent.
- `unwrap_used`, `expect_used`, `panic`, and `indexing_slicing` are **not** globally denied. Tests may use `unwrap`/`expect`. Production code uses `Result`.

### Dependencies

- Do not add a dependency without running `make lint` to verify cargo-shear and cargo-deny still pass.
- Do not assume `--all-features` proves individual features compile in isolation; use `make check-features`.

### Dead-code and unused APIs

- Public APIs in `shapeport-core` and `shapeport-mcp` cannot always be proven dead because external callers are invisible. Use `cargo-udeps` (nightly, via `make deep-analysis`) as a second opinion.

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
