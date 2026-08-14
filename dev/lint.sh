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

# FAST lint loop — does NOT run miri, udeps, debtmap, or cargo-hack.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

# ---------------------------------------------------------------------------
# 1. Trunk (non-Rust linters): optional — skip gracefully when not on PATH
# ---------------------------------------------------------------------------
if command -v trunk &>/dev/null; then
	echo "==> Running trunk check -a ..."
	trunk check -a
else
	echo "==> trunk not found on PATH; skipping non-Rust linters (Trunk)."
	echo "    Install Trunk from https://trunk.io to enable full repo linting."
fi

# ---------------------------------------------------------------------------
# 2. Cargo type check
# ---------------------------------------------------------------------------
echo "==> Running cargo check ..."
cargo check --workspace --all-targets --all-features

# ---------------------------------------------------------------------------
# 3. Clippy
# ---------------------------------------------------------------------------
echo "==> Running cargo clippy ..."
cargo clippy --workspace --all-targets --all-features -- -D warnings

# ---------------------------------------------------------------------------
# 4. cargo-shear: detect unused dependencies
# ---------------------------------------------------------------------------
if ! command -v cargo-shear &>/dev/null; then
	echo ""
	echo "ERROR: cargo-shear is not installed. Run 'make setup' to install required tools."
	exit 1
fi
echo "==> Running cargo shear ..."
cargo shear --deny-warnings

# ---------------------------------------------------------------------------
# 5. cargo-deny: license / advisory / duplicate checks
# ---------------------------------------------------------------------------
if ! command -v cargo-deny &>/dev/null; then
	echo ""
	echo "ERROR: cargo-deny is not installed. Run 'make setup' to install required tools."
	exit 1
fi
echo "==> Running cargo deny check ..."
cargo deny check

echo ""
echo "==> lint: all checks passed."
