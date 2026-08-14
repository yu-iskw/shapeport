# ShapePort Quality-Assurance Architecture

This document is the canonical reference for the quality-assurance system.
Tool configuration lives in the repo root (`clippy.toml`, `deny.toml`, `.trunk/trunk.yaml`).
Make targets are defined in `Makefile`; scripts live in `dev/`.

---

## Architecture

```
Source
  |
  +--> rustfmt
  |
  +--> rustc / Clippy
  |      |
  |      +--> correctness
  |      +--> cognitive complexity
  |      +--> function size
  |
  +--> cargo-shear
  |      |
  |      +--> unused dependencies
  |      +--> unlinked source
  |
  +--> cargo-hack
  |      |
  |      +--> feature configurations
  |
  +--> cargo-deny
  |      |
  |      +--> dependency policy
  |
  +--> Debtmap
  |      |
  |      +--> cyclomatic complexity
  |      +--> hotspots
  |
  +--> CodeQL
  |
  +--> Miri
```

---

## Layers

The tools form complementary layers, not redundant ones. Each layer catches a distinct class of problem.

### FAST / LOCAL / EVERY PR

Run on every pull request and locally before committing:

| Tool | What it catches |
|---|---|
| `rustfmt` | Formatting divergence |
| `cargo check` | Type errors, missing traits |
| `cargo clippy` | Correctness, style, cognitive complexity, function size |
| `cargo-shear` | Unused `Cargo.toml` dependencies; unlinked source files |
| `cargo-deny` | License violations, advisory vulnerabilities, banned sources |
| `cargo-hack --each-feature` | Feature-gated compilation failures |
| `cargo test` | Unit and integration regressions |

**Entry point:** `make lint && make test`

Trunk is the hermetic non-Rust linter runner for markdown, YAML, shell, OSV-scanner, Trivy, and Semgrep.
`cargo-deny` is the Rust license/advisory/source policy layer.
They operate on different file sets and are not duplicates.

### MAINTAINABILITY

| Tool | What it catches |
|---|---|
| Debtmap | Cyclomatic complexity hotspots across the codebase |

The complexity workflow runs on pull requests, but most Debtmap metrics are informational.
The fast authoritative gate for per-function complexity is Clippy's `cognitive_complexity` lint with threshold 10.
Debtmap fails CI only for conservative gates: cyclomatic complexity above 20 (unless justified) and the density threshold in `.debtmap.toml`.

**Entry point:** `make analyze-complexity`

### SECURITY / DATA FLOW

| Tool | What it catches |
|---|---|
| CodeQL | Taint tracking, injection paths, unsafe data-flow patterns |

CodeQL runs via GitHub code scanning.
Private repositories require the `CODEQL_ENABLED=true` secret/environment variable to be set in the Actions environment.

**Entry point:** `make codeql` (local); GitHub Actions for CI.

### DEEP / NIGHTLY

| Tool | What it catches |
|---|---|
| Miri | Undefined behaviour in unsafe code under a Rust interpreter |
| cargo-udeps | Unused dependencies (nightly; second opinion after cargo-shear) |

These tools require a nightly Rust toolchain.
`setup.sh` does **not** silently install nightly; install it explicitly when needed.

**Entry point:** `make deep-analysis`

### CONDITIONAL

These tools are available but not mandatory on every local edit:

| Tool | Condition |
|---|---|
| `cargo-semver-checks` | `shapeport-core` and `shapeport-mcp` are public libraries. Run on `main`/release branches or when a crates.io or git baseline exists. Not required for every local change. |
| `cargo-llvm-cov` | Coverage reporting. Run when evaluating test coverage gaps. |
| Kani | Not enabled. Considered for future formal verification of unsafe sections. |
| Dylint | Not enabled. |

**Entry point:** `make semver-checks`, `make coverage`

---

## Commands

All commands use the `Makefile` targets. Do not invent one-off shell equivalents.

```bash
make setup              # fetch Cargo dependencies; install cargo tools via cargo-binstall
make format             # rustfmt + Trunk repo formatters
make lint               # Trunk (if present), cargo check, Clippy, cargo-shear, cargo-deny
                        # does NOT run Miri or cargo-udeps
make check-features     # cargo hack --each-feature across the workspace
make test               # cargo test --workspace --all-features
make coverage           # cargo-llvm-cov report
make analyze-complexity # Debtmap cyclomatic complexity analysis
make deep-analysis      # Miri + cargo-udeps (requires nightly)
make build              # release binaries and libraries
make semver-checks      # cargo-semver-checks against the published baseline (optional)
make codeql             # local CodeQL analysis
make clean              # remove build artifacts
```

---

## Complexity Thresholds

### Clippy (`clippy.toml`) — stable, mandatory gate

| Setting | Value |
|---|---|
| `cognitive-complexity-threshold` | `10` |
| `too-many-arguments-threshold` | `6` |
| `too-many-lines-threshold` | `100` |
| `type-complexity-threshold` | `250` |

Clippy is the fast, authoritative per-function gate. Violations block CI.

### Debtmap — cyclomatic complexity interpretation

| Range | Signal |
|---|---|
| 1–5 | Simple — no action needed |
| 6–10 | Moderate — acceptable with clear logic |
| 11–15 | Investigate — consider splitting |
| 16–20 | Strong refactoring signal |
| > 20 | CI failure unless the complexity is explicitly justified and documented |

Prefer function-level thresholds. Do not suppress a complexity violation without documenting why the complexity is inherent to the algorithm.

---

## Stable vs Nightly Toolchain

| Category | Stable | Nightly |
|---|---|---|
| Formatting | rustfmt | — |
| Type checking | cargo check | — |
| Linting | cargo clippy | — |
| Unused deps (fast) | cargo-shear | — |
| Dependency policy | cargo-deny | — |
| Feature gating | cargo-hack | — |
| Testing | cargo test | — |
| Coverage | cargo-llvm-cov | — |
| Complexity | Debtmap | — |
| Semver | cargo-semver-checks | — |
| UB detection | — | Miri |
| Unused deps (deep) | — | cargo-udeps |

`setup.sh` installs stable tools only. Nightly tools require an explicit `rustup toolchain install nightly` step.

---

## Lint Suppressions

- **Do not suppress to make CI green.** A red CI gate is a signal, not an obstacle.
- Refactor complexity violations first; split functions, introduce types.
- When suppression is genuinely necessary, use a scoped `#[allow(...)]` with a comment explaining why the complexity or lint is inherent.
- **Production vs tests:** tests may use `unwrap`/`expect` and larger functions where clarity benefits from directness. Production code uses `Result`. The lints `unwrap_used`, `expect_used`, `panic`, and `indexing_slicing` are evaluated in context and are **not** globally denied.

---

## Dead-Code Limitations

`rustc` `dead_code` and `unreachable_pub` together with `cargo-shear` detect unused private items and unused `Cargo.toml` entries.

Public APIs in reusable library crates (`shapeport-core`, `shapeport-mcp`) **cannot always be proven dead** because external consumers may call them. The compiler cannot see those call sites. Use `cargo-udeps` (nightly) as a second opinion in these cases.

---

## Feature Checking

Do not assume `--all-features` proves that each feature compiles independently.
When features interact or provide alternative implementations, some combinations may fail even though the union succeeds.

`cargo hack --each-feature` compiles the workspace once per feature in isolation.
This repo currently has no optional crate features, but `--each-feature` still runs as a regression gate to catch unintentional feature dependencies introduced in future work.

**Entry point:** `make check-features`

---

## Dependency and License Policy

Controlled by `deny.toml`. Key rules:

| Rule | Policy |
|---|---|
| License allowlist | Defined in `deny.toml`; add new licenses there with a rationale comment |
| Advisory ignores | Must include a reason and a tracking issue reference |
| Sources | `crates.io` only by default; git dependencies are denied unless explicitly allowed |
| Wildcards | Denied — all version constraints must be explicit |
| Duplicate versions | Warn; resolve when practical |

To change a policy: edit `deny.toml`, add a comment explaining the reason, and run `make lint` to verify.

Do not add a new dependency without first checking that `cargo-shear` still reports no unused deps and that `cargo-deny` still passes.

---

## Tool Upgrades

- **Cargo dependencies:** Dependabot runs weekly on the `cargo` ecosystem.
- **GitHub Actions:** Dependabot runs weekly on `github-actions`; action pins use commit SHAs with a version comment.
- **Cargo tools** (cargo-shear, cargo-deny, cargo-hack, Debtmap, etc.): installed by `setup.sh` via `cargo-binstall` or `cargo install --locked`. Upgrade by re-running `make setup` after bumping the tool version in `setup.sh`, or via Dependabot when the tool is declared in a lockable form.

---

## Unsafe Policy

`unsafe_code = forbid` is set in `[workspace.lints.rust]`.

If `unsafe` becomes necessary:

1. Isolate it in a clearly named module.
2. Add a `// SAFETY:` comment on every `unsafe` block explaining the invariants upheld.
3. Enable `undocumented_unsafe_blocks` to enforce that policy.
4. Run Miri (`make deep-analysis`) to catch UB under the interpreter.
5. Consider Kani for formal verification of the unsafe section before merging.

---

## See Also

- `clippy.toml` — Clippy threshold configuration
- `deny.toml` — dependency/license policy
- `.trunk/trunk.yaml` — non-Rust linter configuration
- `docs/adr/` — Architecture Decision Records
- `Makefile` — authoritative command definitions
- `dev/` — implementation scripts for each Make target
