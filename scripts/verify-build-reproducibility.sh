#!/usr/bin/env bash
set -euo pipefail

# verify-build-reproducibility.sh
# Verifies that the Soroban Wasm contract build is reproducible
# (identical source code produces byte-for-byte identical Wasm artifacts).

echo "=== Verifying Build Reproducibility (Issue #632) ==="

TMP_DIR="$(mktemp -d /tmp/stellar-vault-build-check-XXXXXX)"
# shellcheck disable=SC2329 # invoked indirectly via `trap cleanup EXIT` below
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

BUILD_A="${TMP_DIR}/build-a"
BUILD_B="${TMP_DIR}/build-b"
mkdir -p "${BUILD_A}" "${BUILD_B}"

export RUSTFLAGS="-C codegen-units=1 --remap-path-prefix=${PWD}=."

echo "Building Run A..."
CARGO_TARGET_DIR="${BUILD_A}" cargo build --target wasm32-unknown-unknown --release

echo "Building Run B..."
CARGO_TARGET_DIR="${BUILD_B}" cargo build --target wasm32-unknown-unknown --release

WASM_NAME="stellar_defi_vault.wasm"
WASM_A="${BUILD_A}/wasm32-unknown-unknown/release/${WASM_NAME}"
WASM_B="${BUILD_B}/wasm32-unknown-unknown/release/${WASM_NAME}"

if [[ ! -f "${WASM_A}" || ! -f "${WASM_B}" ]]; then
    echo "Error: Output Wasm artifact not found."
    exit 1
fi

HASH_A=$(sha256sum "${WASM_A}" | awk '{print $1}')
HASH_B=$(sha256sum "${WASM_B}" | awk '{print $1}')

echo "Run A SHA256: ${HASH_A}"
echo "Run B SHA256: ${HASH_B}"

if cmp -s "${WASM_A}" "${WASM_B}"; then
    echo "SUCCESS: Wasm builds are byte-for-byte identical."
    echo "Verified reproducible SHA256: ${HASH_A}"
    exit 0
else
    echo "ERROR: Builds differed! Non-deterministic build detected."
    diff -u <(xxd "${WASM_A}") <(xxd "${WASM_B}") | head -n 30 || true
    exit 1
fi
