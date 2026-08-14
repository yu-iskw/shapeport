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

# Maintainability / complexity analysis using Debtmap.
#
# Clippy cognitive complexity (threshold 10) is the fast mandatory gate.
# This script is the richer maintainability layer:
#   - JSON artifact for CI
#   - human-readable hotspot listing from that artifact
#   - debtmap validate (density-based)
#   - function-level cyclomatic > 20 hard fail

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

COMPLEXITY_THRESHOLD=20
OUTPUT_DIR="${MODULE_DIR}/target"
OUTPUT_JSON="${OUTPUT_DIR}/debtmap.json"
COVERAGE_FILE="${MODULE_DIR}/coverage.lcov"

if ! command -v debtmap &>/dev/null; then
	echo "ERROR: debtmap is not installed. Run 'make setup' to install required tools."
	exit 1
fi

if ! command -v python3 &>/dev/null; then
	echo "ERROR: python3 is required to evaluate Debtmap JSON against the cyclomatic gate."
	exit 1
fi

echo "==> Running cyclomatic gate unit tests ..."
python3 -m unittest discover -s "${SCRIPT_DIR}" -p 'test_cyclomatic_gate.py'

mkdir -p "${OUTPUT_DIR}"

COVERAGE_ARGS=()
if [[ -f ${COVERAGE_FILE} ]]; then
	echo "==> Passing coverage data: --coverage-file ${COVERAGE_FILE}"
	COVERAGE_ARGS+=(--coverage-file "${COVERAGE_FILE}")
elif [[ -n ${GITHUB_ACTIONS-} ]]; then
	echo "ERROR: coverage.lcov is required in CI because max_debt_density is sized for coverage-aware scoring."
	echo "  The complexity workflow must generate LCOV before this step; do not skip coverage to make the density gate pass."
	exit 1
else
	echo "    (no coverage.lcov found; run 'make coverage' first to enable coverage-aware analysis)"
fi

echo "==> Writing Debtmap JSON to ${OUTPUT_JSON}"
debtmap analyze . --languages rust --format json --output "${OUTPUT_JSON}" "${COVERAGE_ARGS[@]}"

echo ""
echo "==> Running: debtmap validate ."
debtmap validate . "${COVERAGE_ARGS[@]}"

echo ""
echo "==> Applying cyclomatic complexity hard gate (threshold: ${COMPLEXITY_THRESHOLD}) ..."
python3 "${SCRIPT_DIR}/cyclomatic_gate.py" "${OUTPUT_JSON}" "${COMPLEXITY_THRESHOLD}"

echo ""
echo "==> analyze-complexity: complete."
