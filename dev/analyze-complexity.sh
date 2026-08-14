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
#   - human-readable hotspot listing
#   - debtmap validate (density-based)
#   - function-level cyclomatic > 20 hard fail unless justified

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

COMPLEXITY_THRESHOLD=20
JUSTIFICATION_FILE="${MODULE_DIR}/complexity-justifications.txt"
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

mkdir -p "${OUTPUT_DIR}"

COVERAGE_ARGS=()
if [[ -f "${COVERAGE_FILE}" ]]; then
	echo "==> Passing coverage data: --coverage-file ${COVERAGE_FILE}"
	COVERAGE_ARGS+=(--coverage-file "${COVERAGE_FILE}")
else
	echo "    (no coverage.lcov found; run 'make coverage' first to enable coverage-aware analysis)"
fi

# Full JSON for gating. Do not pass --top here: that truncates the artifact.
# --threshold-complexity only filters reporting; it does not fail the process.
echo "==> Writing Debtmap JSON to ${OUTPUT_JSON}"
debtmap analyze . --languages rust --format json --output "${OUTPUT_JSON}" "${COVERAGE_ARGS[@]}"

echo ""
echo "==> Highest-complexity functions (top 20):"
debtmap analyze . --languages rust --top 20 "${COVERAGE_ARGS[@]}" || true

echo ""
echo "==> Running: debtmap validate ."
debtmap validate . "${COVERAGE_ARGS[@]}"

echo ""
echo "==> Applying cyclomatic complexity hard gate (threshold: ${COMPLEXITY_THRESHOLD}) ..."

if [[ ! -f "${OUTPUT_JSON}" ]]; then
	echo "ERROR: ${OUTPUT_JSON} was not produced; cannot enforce the cyclomatic gate."
	exit 1
fi

set +e
GATE_OUTPUT="$(python3 - "${OUTPUT_JSON}" "${COMPLEXITY_THRESHOLD}" <<'PYEOF'
import json
import sys

path, threshold = sys.argv[1], int(sys.argv[2])
try:
    data = json.load(open(path, encoding="utf-8"))
except Exception as exc:
    print(f"ERROR: failed to parse {path}: {exc}", file=sys.stderr)
    sys.exit(2)

CYCLO_KEYS = (
    "cyclomatic",
    "cyclomatic_complexity",
    "cyclo",
    "cyclomatic_complexity_value",
)

def iter_items(node):
    if isinstance(node, list):
        for item in node:
            yield from iter_items(item)
    elif isinstance(node, dict):
        yield node
        for value in node.values():
            yield from iter_items(value)

seen = set()
violations = []
for item in iter_items(data):
    cyclo = None
    metrics = item.get("metrics") if isinstance(item.get("metrics"), dict) else {}
    for key in CYCLO_KEYS:
        for source in (item, metrics):
            val = source.get(key) if isinstance(source, dict) else None
            if isinstance(val, (int, float)):
                cyclo = val
                break
        if cyclo is not None:
            break
    if cyclo is None or cyclo <= threshold:
        continue
    name = (
        item.get("canonical_name")
        or item.get("name")
        or item.get("function")
        or item.get("function_name")
        or item.get("id")
        or "unknown"
    )
    file = item.get("file") or item.get("path") or item.get("filename") or ""
    line = item.get("line") or item.get("start_line") or item.get("line_start") or ""
    identity = (str(file), str(name), str(line), float(cyclo))
    if identity in seen:
        continue
    seen.add(identity)
    location = f"{file}:{line}" if file else ""
    violations.append((cyclo, name, location))

violations.sort(key=lambda row: row[0], reverse=True)
for cyclo, name, location in violations:
    suffix = f"  {location}" if location else ""
    print(f"  VIOLATION: {name}  cyclomatic={cyclo} > {threshold}{suffix}")
print(f"COUNT={len(violations)}")
sys.exit(0)
PYEOF
)"
GATE_STATUS=$?
set -e

if [[ "${GATE_STATUS}" -ne 0 ]]; then
	echo "${GATE_OUTPUT}"
	echo "ERROR: failed to evaluate Debtmap JSON for the cyclomatic gate."
	exit 1
fi

echo "${GATE_OUTPUT}"
VIOLATING_COUNT="$(printf '%s\n' "${GATE_OUTPUT}" | sed -n 's/^COUNT=//p' | tail -n 1)"
VIOLATING_COUNT="${VIOLATING_COUNT:-0}"

if [[ "${VIOLATING_COUNT}" -gt 0 ]]; then
	if [[ -f "${JUSTIFICATION_FILE}" ]]; then
		echo "    WARNING: ${VIOLATING_COUNT} function(s) exceed cyclomatic complexity ${COMPLEXITY_THRESHOLD}."
		echo "    Justification file found (${JUSTIFICATION_FILE}); gate suppressed."
		echo "    Document why each hotspot's complexity is inherent before relying on this escape hatch."
	else
		echo ""
		echo "ERROR: ${VIOLATING_COUNT} function(s) exceed cyclomatic complexity ${COMPLEXITY_THRESHOLD}."
		echo "  Refactor the flagged functions. Do not suppress this gate to make CI green."
		echo "  If complexity is inherent, document it in ${JUSTIFICATION_FILE}."
		exit 1
	fi
else
	echo "    No functions exceeded cyclomatic complexity ${COMPLEXITY_THRESHOLD}."
fi

echo ""
echo "==> analyze-complexity: complete."
