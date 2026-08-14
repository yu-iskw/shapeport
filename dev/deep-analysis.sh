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

# Nightly deep analysis: Miri, then cargo-udeps.
# Nightly is NOT installed automatically unless INSTALL_NIGHTLY=1 is set.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"

bash "${SCRIPT_DIR}/miri.sh"
bash "${SCRIPT_DIR}/udeps.sh"

echo ""
echo "==> deep-analysis: all nightly checks passed."
