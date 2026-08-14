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

# Verify that the workspace compiles cleanly under every individual feature flag
# using cargo-hack. This catches feature-gated code paths that would otherwise
# be hidden when --all-features is used.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

if ! command -v cargo-hack &>/dev/null; then
	echo "ERROR: cargo-hack is not installed. Run 'make setup' to install required tools."
	exit 1
fi

echo "==> Running cargo hack clippy (each-feature) ..."
cargo hack clippy --workspace --each-feature --all-targets -- -D warnings

echo ""
echo "==> check-features: all per-feature clippy checks passed."
