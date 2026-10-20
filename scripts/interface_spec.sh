#!/usr/bin/env bash
# Regenerates the contract's public interface spec from a real build and
# either checks it against the committed snapshot (default) or overwrites
# the snapshot with it (--update). See docs/INTERFACE_SPEC.md.
set -euo pipefail

if ! command -v stellar >/dev/null 2>&1; then
    echo "Error: stellar CLI is not installed or not on PATH." >&2
    echo "Please install it by running:" >&2
    echo "  cargo install --locked stellar-cli" >&2
    exit 1
fi

MODE="check"
if [ "${1:-}" = "--update" ]; then
    MODE="update"
elif [ -n "${1:-}" ]; then
    echo "Usage: scripts/interface_spec.sh [--update]" >&2
    exit 1
fi

# Same build flags as CI's "WASM Build" job (.github/workflows/ci.yml):
# `--features vault-wasm` is what actually ships, so the spec this checks
# against is the spec of the real deployable artifact, not an incidental
# dev build.
echo "Building the release WASM (--features vault-wasm)..." >&2
cargo build --target wasm32-unknown-unknown --release --features vault-wasm

WASM_PATH="target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm"
if [ ! -f "$WASM_PATH" ]; then
    echo "Error: WASM file not found at $WASM_PATH" >&2
    exit 1
fi

SPEC_DIR="interface-spec"
SPEC_FILE="$SPEC_DIR/VaultContract.json"
mkdir -p "$SPEC_DIR"

GENERATED=$(stellar contract info interface --wasm "$WASM_PATH" --output json-formatted)

if [ "$MODE" = "update" ]; then
    printf '%s\n' "$GENERATED" > "$SPEC_FILE"
    echo "Wrote $SPEC_FILE from the current build." >&2
    echo >&2
    echo "Next steps:" >&2
    echo "  1. Review the diff — does it match what you intended to change?" >&2
    echo "  2. Add a CHANGELOG.md entry describing the change and why it's" >&2
    echo "     intentional (see docs/INTERFACE_SPEC.md)." >&2
    echo "  3. Commit $SPEC_FILE alongside the code change and the changelog" >&2
    echo "     entry in the same pull request." >&2
    exit 0
fi

if [ ! -f "$SPEC_FILE" ]; then
    echo "Error: $SPEC_FILE does not exist yet." >&2
    echo "Run 'scripts/interface_spec.sh --update' once to seed it, review the" >&2
    echo "result, and commit it — see docs/INTERFACE_SPEC.md." >&2
    exit 1
fi

if ! diff -u "$SPEC_FILE" <(printf '%s\n' "$GENERATED") > /tmp/interface-spec.diff 2>&1; then
    echo "The contract's public interface differs from the committed snapshot" >&2
    echo "at $SPEC_FILE:" >&2
    echo >&2
    cat /tmp/interface-spec.diff >&2
    echo >&2
    echo "If this change is intentional: run 'scripts/interface_spec.sh --update'," >&2
    echo "commit the updated $SPEC_FILE, and add a CHANGELOG.md entry describing" >&2
    echo "the change and why it's intentional (see docs/INTERFACE_SPEC.md)." >&2
    echo >&2
    echo "If it isn't intentional: this is exactly the accidental public-API change" >&2
    echo "this check exists to catch before it ships — fix the code instead." >&2
    exit 1
fi

echo "Interface spec matches $SPEC_FILE." >&2
