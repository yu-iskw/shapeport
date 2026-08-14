#!/usr/bin/env bash
# Shared helpers for quality-system scripts. Source this file; do not execute it.

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

# Install a cargo binary idempotently.
# Prefer cargo-binstall; fall back to cargo install --locked.
# Usage: install_cargo_tool <binary-name> <crate-name>
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

# Print the rustup command a developer should run when nightly is missing.
_nightly_install_hint() {
	if [[ $# -gt 0 ]]; then
		echo "    rustup toolchain install nightly $*"
	else
		echo "    rustup toolchain install nightly"
	fi
}

# Collect `--component NAME` values into the nameref array given as $1.
_nightly_components_from_args() {
	local -n _components=$1
	shift
	_components=()
	local expecting_component=false
	local arg
	for arg in "$@"; do
		if [[ "${expecting_component}" == "true" ]]; then
			_components+=("${arg}")
			expecting_component=false
		elif [[ "${arg}" == "--component" ]]; then
			expecting_component=true
		else
			echo "ERROR: require_nightly: unsupported argument: ${arg}" >&2
			exit 1
		fi
	done
	if [[ "${expecting_component}" == "true" ]]; then
		echo "ERROR: require_nightly: --component requires a name" >&2
		exit 1
	fi
}

# Add any requested nightly components that are not already installed.
_ensure_nightly_components() {
	local -n _components=$1
	[[ ${#_components[@]} -eq 0 ]] && return 0
	local installed
	installed="$(rustup component list --toolchain nightly 2>/dev/null || true)"
	local component
	for component in "${_components[@]}"; do
		if grep -q "${component}.*installed" <<<"${installed}"; then
			continue
		fi
		echo "==> Adding ${component} to nightly ..."
		rustup component add --toolchain nightly "${component}"
	done
}

# Ensure the nightly toolchain exists. Extra args must be `--component NAME`
# pairs. When nightly is already present, missing components are added without
# re-running `rustup toolchain install` (which would hit the network).
require_nightly() {
	local components=()
	_nightly_components_from_args components "$@"

	if rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
		_ensure_nightly_components components
		return 0
	fi

	if [[ "${INSTALL_NIGHTLY:-}" != "1" ]]; then
		echo ""
		echo "ERROR: nightly Rust toolchain is not installed and INSTALL_NIGHTLY is not set."
		echo ""
		echo "  This is an environment limitation for local machines that have opted out of nightly."
		echo "  To install nightly manually, run:"
		echo ""
		_nightly_install_hint "$@"
		echo ""
		echo "  To allow this script to install nightly automatically (e.g. in CI), set:"
		echo ""
		echo "    INSTALL_NIGHTLY=1 make deep-analysis"
		echo ""
		exit 1
	fi

	echo "==> INSTALL_NIGHTLY=1 set; installing nightly toolchain ..."
	rustup toolchain install nightly "$@"
}
