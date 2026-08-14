#!/usr/bin/env bash
set -Eeuo pipefail

# Copyright 2025 yu-iskw
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Constants
SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

if ! command -v cargo &>/dev/null; then
  echo "Error: cargo is not installed."
  echo "Install Rust from https://rustup.rs/ and rerun make setup."
  exit 1
fi

# ---------------------------------------------------------------------------
# Helper: install a cargo tool idempotently
# Prefer cargo-binstall when available (faster); fall back to cargo install.
# Usage: install_cargo_tool <binary-name> <crate-name>
# ---------------------------------------------------------------------------
install_cargo_tool() {
  local binary="$1"
  local crate="$2"
  if command -v "${binary}" &>/dev/null; then
    echo "  [skip]    ${crate} (${binary} already on PATH)"
    return 0
  fi
  if command -v cargo-binstall &>/dev/null; then
    echo "  [install] ${crate} via cargo-binstall"
    cargo binstall -y "${crate}"
  else
    echo "  [install] ${crate} via cargo install --locked"
    cargo install "${crate}" --locked
  fi
}

# ---------------------------------------------------------------------------
# Step 1: fetch workspace dependencies
# ---------------------------------------------------------------------------
echo "==> Fetching workspace dependencies..."
cargo fetch

# ---------------------------------------------------------------------------
# Step 2: required tools
# ---------------------------------------------------------------------------
echo ""
echo "==> Installing required tools..."
install_cargo_tool cargo-hack       cargo-hack
install_cargo_tool cargo-shear      cargo-shear
install_cargo_tool cargo-deny       cargo-deny
install_cargo_tool debtmap          debtmap

# ---------------------------------------------------------------------------
# Step 3: optional tools
# ---------------------------------------------------------------------------
echo ""
echo "==> Installing optional tools (skipped gracefully if unavailable)..."
install_cargo_tool cargo-llvm-cov   cargo-llvm-cov   || echo "  [warn]    cargo-llvm-cov install failed; coverage target will not work"
install_cargo_tool cargo-semver-checks cargo-semver-checks || echo "  [warn]    cargo-semver-checks install failed; semver-checks target will not work"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "==> Setup complete."
echo ""
echo "  Required tools (needed by make lint, make check-features, make analyze-complexity):"
for t in cargo-hack cargo-shear cargo-deny debtmap; do
  if command -v "${t}" &>/dev/null; then
    echo "    [ok] ${t}"
  else
    echo "    [MISSING] ${t}  <-- run make setup again or install manually"
  fi
done
echo ""
echo "  Optional tools:"
for t in cargo-llvm-cov cargo-semver-checks; do
  if command -v "${t}" &>/dev/null; then
    echo "    [ok] ${t}"
  else
    echo "    [missing] ${t}  (non-blocking)"
  fi
done
echo ""
echo "  NOTE: nightly Rust (miri, cargo-udeps) is NOT installed by default."
echo "        'make deep-analysis' requires nightly. Set INSTALL_NIGHTLY=1 or"
echo "        run: rustup toolchain install nightly --component miri rust-src"
