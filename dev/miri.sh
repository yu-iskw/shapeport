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

require_nightly --component miri --component rust-src

echo ""
echo "==> Running cargo +nightly miri test --workspace --lib ..."
echo "    (MIRIFLAGS: ${MIRIFLAGS:--Zmiri-disable-isolation})"
echo "    Miri is dynamic: it only checks executed paths."
echo "    Default scope is --lib: OS-heavy integration tests (sockets, subprocesses)"
echo "    are not part of the routine Miri gate."
export MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation}"
if ! cargo +nightly miri test --workspace --lib; then
	echo ""
	echo "ERROR: Miri reported test failures. Check the output above for details."
	echo "  Common causes: undefined behaviour, invalid memory access, data races,"
	echo "  or tests that need OS features Miri does not emulate."
	echo "  MIRIFLAGS defaults to -Zmiri-disable-isolation so filesystem tests can run."
	echo "  See https://github.com/rust-lang/miri#miriflags"
	exit 1
fi

echo ""
echo "==> miri: passed."
