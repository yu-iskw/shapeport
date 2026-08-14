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

# SemVer compatibility check for library crates.
#
# shapeport-core and shapeport-mcp are library crates. cargo-semver-checks
# requires a published baseline on crates.io. If neither crate has been
# published yet, this script exits 0 with a clear message.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

if ! command -v cargo-semver-checks &>/dev/null; then
	echo "ERROR: cargo-semver-checks is not installed."
	echo "  Run 'make setup' to install optional tools, or:"
	echo "  cargo install cargo-semver-checks --locked"
	exit 1
fi

# ---------------------------------------------------------------------------
# Detect whether library crates have been published to crates.io
# ---------------------------------------------------------------------------
LIBRARY_CRATES=(shapeport-core shapeport-mcp)
PUBLISHED=()

check_published() {
	local crate="$1"
	# curl to the crates.io API; treat HTTP 404 as "not published"
	local status
	status="$(curl -s -o /dev/null -w '%{http_code}' \
		"https://crates.io/api/v1/crates/${crate}" \
		-H 'User-Agent: shapeport-semver-checks/0.1 (github.com/yu-iskw/shapeport)' \
		--max-time 10 2>/dev/null || echo '000')"
	[[ ${status} == "200" ]]
}

echo "==> Checking crates.io publication status ..."
for crate in "${LIBRARY_CRATES[@]}"; do
	# Predicate: non-zero means unpublished, not a script failure.
	# shellcheck disable=SC2310
	if check_published "${crate}"; then
		echo "  [published]   ${crate}"
		PUBLISHED+=("${crate}")
	else
		echo "  [unpublished] ${crate}  (no baseline available; skipping semver check)"
	fi
done

if [[ ${#PUBLISHED[@]} -eq 0 ]]; then
	echo ""
	echo "==> semver-checks: no published crates found on crates.io."
	echo "    Skipping semver compatibility check (no baseline)."
	echo "    Once crates are published, this check will enforce semver compatibility."
	exit 0
fi

# ---------------------------------------------------------------------------
# Run semver checks for published crates
# ---------------------------------------------------------------------------
echo ""
pkg_args=()
for crate in "${PUBLISHED[@]}"; do
	pkg_args+=(-p "${crate}")
done
echo "==> Running cargo semver-checks check-release ${pkg_args[*]} ..."
cargo semver-checks check-release "${pkg_args[@]}"

echo ""
echo "==> semver-checks: all semver compatibility checks passed."
