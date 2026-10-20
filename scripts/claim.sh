#!/usr/bin/env bash
set -euo pipefail

# 1. Check for stellar CLI
if ! command -v stellar >/dev/null 2>&1; then
    echo "Error: stellar CLI is not installed or not on PATH."
    echo "Please install it by running:"
    echo "  cargo install --locked stellar-cli"
    exit 1
fi

# Load env file if it exists
if [ -f .env ]; then
    # Load environment variables. `export $(grep … | xargs)` word-split
    # every value, breaking any value with spaces or quotes; sourcing with
    # allexport keeps the shell's own quoting rules (#669).
    set -a
    # shellcheck disable=SC1091 # .env is local and optional, not in the repo
    . ./.env
    set +a
fi

CONTRACT_ID="${CONTRACT_ID:-}"
IDENTITY="${IDENTITY:-}"
NETWORK="${NETWORK:-testnet}"

if [ -z "$CONTRACT_ID" ]; then
    echo "Error: CONTRACT_ID is not set in environment or .env file."
    exit 1
fi

if [ -z "$IDENTITY" ]; then
    echo "Error: IDENTITY is not set in environment or .env file."
    exit 1
fi

IDENTITY_ADDRESS=$(stellar keys address "$IDENTITY")

echo "Claiming rewards for $IDENTITY ($IDENTITY_ADDRESS) in vault $CONTRACT_ID..."

stellar contract invoke \
    --id "$CONTRACT_ID" \
    --source "$IDENTITY" \
    --network "$NETWORK" \
    -- \
    claim \
    --staker "$IDENTITY_ADDRESS"
