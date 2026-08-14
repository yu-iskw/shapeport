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

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

require_nightly
install_cargo_tool cargo-udeps cargo-udeps

echo ""
echo "==> Running cargo +nightly udeps --workspace ..."
if ! cargo +nightly udeps --workspace --all-targets --all-features; then
	echo ""
	echo "ERROR: cargo-udeps found unused dependencies."
	echo "  Remove them from Cargo.toml after confirming cargo-shear agrees."
	exit 1
fi

echo ""
echo "==> udeps: passed."
