#!/usr/bin/env bash
set -euo pipefail

# 1. Check for stellar CLI
if ! command -v stellar >/dev/null 2>&1; then
    echo "Error: stellar CLI is not installed or not on PATH."
    echo "Please install it by running:"
    echo "  cargo install --locked stellar-cli"
    exit 1
fi

# 2. Get configuration
# Use IDENTITY from env, default to "deployer"
IDENTITY="${IDENTITY:-deployer}"
NETWORK="${NETWORK:-testnet}"

echo "Using identity: $IDENTITY"
echo "Using network: $NETWORK"

# Ensure the identity exists, if not generate it
if ! stellar keys address "$IDENTITY" >/dev/null 2>&1; then
    echo "Identity '$IDENTITY' not found. Generating a new one..."
    stellar keys generate --network "$NETWORK" "$IDENTITY"
fi

IDENTITY_ADDRESS=$(stellar keys address "$IDENTITY")
echo "Identity address: $IDENTITY_ADDRESS"

# Fund the identity if on testnet and balance is low/zero
if [ "$NETWORK" = "testnet" ]; then
    echo "Funding identity on testnet using Friendbot..."
    curl -s "https://friendbot.stellar.org?addr=$IDENTITY_ADDRESS" > /dev/null || true
fi

# 3. Build contract
echo "Building contract WASM..."
stellar contract build --optimize

WASM_PATH="target/wasm32v1-none/release/stellar_defi_vault.wasm"

if [ ! -f "$WASM_PATH" ]; then
    echo "Error: WASM file not found at $WASM_PATH"
    exit 1
fi

# 4. Get token address
# Use TOKEN_ID from env, or get native XLM wrapper if not specified
if [ -z "${TOKEN_ID:-}" ]; then
    echo "No TOKEN_ID specified. Fetching native XLM wrapper contract ID..."
    TOKEN_ID=$(stellar contract id asset \
        --asset native \
        --network "$NETWORK")
fi
echo "Using token address: $TOKEN_ID"

# 5. Deploy contract with constructor arguments (atomic deploy & init, preventing front-running)
echo "Deploying and initializing contract atomically to $NETWORK..."
CONTRACT_ID=$(stellar contract deploy \
    --wasm "$WASM_PATH" \
    --source "$IDENTITY" \
    --network "$NETWORK" \
    -- \
    --admin "$IDENTITY_ADDRESS" \
    --token "$TOKEN_ID")

echo "Contract deployed and initialized atomically! ID: $CONTRACT_ID"
echo "----------------------------------------"
echo "Vault Contract ID: $CONTRACT_ID"
echo "Token Contract ID: $TOKEN_ID"
echo "Admin Address:     $IDENTITY_ADDRESS"
echo "----------------------------------------"
echo "Save these to your .env file:"
echo "CONTRACT_ID=$CONTRACT_ID"
echo "TOKEN_ID=$TOKEN_ID"
echo "IDENTITY=$IDENTITY"
echo "NETWORK=$NETWORK"
