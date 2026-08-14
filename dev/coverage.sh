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

# Generate an LCOV coverage report using cargo-llvm-cov.
#
# NOTE: coverage.lcov is gitignored and is intended as input for Debtmap /
# gap analysis (make analyze-complexity), NOT as a vanity percentage metric.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

if ! command -v cargo-llvm-cov &>/dev/null; then
  echo "ERROR: cargo-llvm-cov is not installed."
  echo "  Run 'make setup' to install optional tools, or:"
  echo "  cargo install cargo-llvm-cov --locked"
  exit 1
fi

OUTPUT="coverage.lcov"

echo "==> Generating LCOV coverage report -> ${OUTPUT} ..."
cargo llvm-cov --workspace --all-features --lcov --output-path "${OUTPUT}"

echo ""
echo "==> coverage: report written to ${OUTPUT}"
echo "    This file is gitignored and is for Debtmap / gap analysis only."
echo "    Pass it to 'make analyze-complexity' for hotspot-aware debt scoring."
