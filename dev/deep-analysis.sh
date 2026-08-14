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

# Deep analysis using nightly Rust tooling (Miri + cargo-udeps).
#
# Nightly is NOT installed automatically unless INSTALL_NIGHTLY=1 is set.
# This is intentional: nightly is an environment-level commitment and should
# not be silently added on developer machines that have opted out.

SCRIPT_FILE="$(readlink -f "$0")"
SCRIPT_DIR="$(dirname "${SCRIPT_FILE}")"
MODULE_DIR="$(dirname "${SCRIPT_DIR}")"

cd "${MODULE_DIR}"

# ---------------------------------------------------------------------------
# Nightly toolchain availability
# ---------------------------------------------------------------------------
NIGHTLY_AVAILABLE=false
if rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
  NIGHTLY_AVAILABLE=true
fi

if [[ "${NIGHTLY_AVAILABLE}" != "true" ]]; then
  if [[ "${INSTALL_NIGHTLY:-}" == "1" ]]; then
    echo "==> INSTALL_NIGHTLY=1 set; installing nightly toolchain with miri + rust-src ..."
    rustup toolchain install nightly --component miri rust-src
    NIGHTLY_AVAILABLE=true
  else
    echo ""
    echo "ERROR: nightly Rust toolchain is not installed and INSTALL_NIGHTLY is not set."
    echo ""
    echo "  This is an environment limitation for local machines that have opted out of nightly."
    echo "  To install nightly manually, run:"
    echo ""
    echo "    rustup toolchain install nightly --component miri rust-src"
    echo ""
    echo "  To allow this script to install nightly automatically (e.g. in CI), set:"
    echo ""
    echo "    INSTALL_NIGHTLY=1 make deep-analysis"
    echo ""
    exit 1
  fi
fi

# ---------------------------------------------------------------------------
# Ensure miri component is present (may be missing even if nightly is there)
# ---------------------------------------------------------------------------
if ! rustup component list --toolchain nightly 2>/dev/null | grep -q 'miri.*installed'; then
  echo "==> Adding miri and rust-src components to nightly ..."
  rustup toolchain install nightly --component miri rust-src
fi

# ---------------------------------------------------------------------------
# Helper: install a cargo tool idempotently (binstall preferred)
# ---------------------------------------------------------------------------
install_cargo_tool() {
  local binary="$1"
  local crate="$2"
  if command -v "${binary}" &>/dev/null; then
    echo "  [skip]    ${crate} already installed"
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

# ---------------------------------------------------------------------------
# Install cargo-udeps (only when running this script)
# ---------------------------------------------------------------------------
echo "==> Ensuring cargo-udeps is available ..."
install_cargo_tool cargo-udeps cargo-udeps

# ---------------------------------------------------------------------------
# Miri: undefined-behaviour / memory safety testing
# ---------------------------------------------------------------------------
if [[ "${SKIP_MIRI:-}" == "1" ]]; then
  echo "==> SKIP_MIRI=1 set; skipping Miri."
else
  echo ""
  echo "==> Running cargo +nightly miri test --workspace ..."
  echo "    (MIRIFLAGS: ${MIRIFLAGS:--Zmiri-disable-isolation})"
  echo "    Miri is dynamic: it only checks executed paths."
  export MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation}"
  if ! cargo +nightly miri test --workspace; then
    echo ""
    echo "ERROR: Miri reported test failures. Check the output above for details."
    echo "  Common causes: undefined behaviour, invalid memory access, data races,"
    echo "  or tests that need OS features Miri does not emulate."
    echo "  MIRIFLAGS defaults to -Zmiri-disable-isolation so filesystem tests can run."
    echo "  See https://github.com/rust-lang/miri#miriflags"
    exit 1
  fi
fi

# ---------------------------------------------------------------------------
# cargo-udeps: detect unused dependencies
# ---------------------------------------------------------------------------
if [[ "${SKIP_UDEPS:-}" == "1" ]]; then
  echo "==> SKIP_UDEPS=1 set; skipping cargo-udeps."
else
  echo ""
  echo "==> Running cargo +nightly udeps --workspace ..."
  if ! cargo +nightly udeps --workspace --all-targets --all-features; then
    echo ""
    echo "ERROR: cargo-udeps found unused dependencies."
    echo "  Remove them from Cargo.toml after confirming cargo-shear agrees."
    exit 1
  fi
fi

echo ""
echo "==> deep-analysis: all nightly checks passed."
