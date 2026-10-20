#!/usr/bin/env bash
# Seeds N demo/QA accounts on a test network: creates and friendbot-funds
# each one, mints each a test token via a Stellar Asset Contract, and
# deposits that amount into the deployed vault — useful for demos and
# manual QA without hand-running the same five CLI calls per account.
#
# Usage:
#   scripts/seed-testnet.sh --accounts N --amount X [options]
#
# See --help for the full option list and scripts/README usage notes in
# the main README's "Seeding Demo Accounts" section.
set -euo pipefail

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------

if ! command -v stellar >/dev/null 2>&1; then
    echo "Error: stellar CLI is not installed or not on PATH." >&2
    echo "Please install it by running:" >&2
    echo "  cargo install --locked stellar-cli" >&2
    exit 1
fi

if [ -f .env ]; then
    set -a
    # shellcheck disable=SC1091 # .env is local and optional, not in the repo
    . ./.env
    set +a
fi

# ---------------------------------------------------------------------------
# Defaults and argument parsing
# ---------------------------------------------------------------------------

ACCOUNTS=""
AMOUNT=""
NETWORK="${NETWORK:-testnet}"
CONTRACT_ID="${CONTRACT_ID:-}"
TOKEN_ID="${TOKEN_ID:-}"
TOKEN_ISSUER="${TOKEN_ISSUER:-}"
ASSET_CODE="${ASSET_CODE:-SEEDTOK}"
PREFIX="${SEED_PREFIX:-seed}"

show_help() {
    cat <<'EOF'
Usage:
  scripts/seed-testnet.sh --accounts N --amount X [options]

Required:
  --accounts N          Number of demo accounts to create
  --amount X            Amount each account is minted and deposits into the
                         vault, in raw base units (7 decimals, same
                         convention as scripts/stake.sh)

Target (required, via flag or environment variable):
  --contract-id ID       Deployed vault contract id (or $CONTRACT_ID)

Test token (optional — a fresh one is deployed and minted from if omitted):
  --token-id ID          Reuse an already-deployed SAC test token (or
                         $TOKEN_ID). Requires --token-issuer, since only the
                         issuer can mint it.
  --token-issuer NAME    Local identity that issues/mints the test token
                         (or $TOKEN_ISSUER). Generated and funded
                         automatically when a fresh token is deployed.
  --asset-code CODE      Classic asset code for a freshly-deployed test
                         token (or $ASSET_CODE, default SEEDTOK)

Other:
  --prefix NAME          Local identity name prefix for seeded accounts
                         (or $SEED_PREFIX, default "seed"; accounts are
                         named "<prefix>-1", "<prefix>-2", ...). Re-running
                         with the same prefix overwrites those local
                         identities with brand new keys.
  --network NAME         Stellar network (or $NETWORK, default testnet).
                         Only testnet/futurenet/local/standalone are
                         accepted — this script refuses to run against
                         mainnet/public, with no override.
  -h, --help             Show this help

Environment:
  CONTRACT_ID, TOKEN_ID, TOKEN_ISSUER, ASSET_CODE, SEED_PREFIX, NETWORK
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --accounts) ACCOUNTS=${2:-}; shift 2 ;;
        --amount) AMOUNT=${2:-}; shift 2 ;;
        --network) NETWORK=${2:-}; shift 2 ;;
        --contract-id) CONTRACT_ID=${2:-}; shift 2 ;;
        --token-id) TOKEN_ID=${2:-}; shift 2 ;;
        --token-issuer) TOKEN_ISSUER=${2:-}; shift 2 ;;
        --asset-code) ASSET_CODE=${2:-}; shift 2 ;;
        --prefix) PREFIX=${2:-}; shift 2 ;;
        -h|--help) show_help; exit 0 ;;
        *)
            echo "Unknown argument: $1" >&2
            show_help
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Hard mainnet refusal — unconditional, no override flag or env var.
#
# Primary gate: an allowlist of known test networks (safer than a mainnet
# denylist, which only catches names someone thought to block). Secondary,
# defense-in-depth gate: reject the network's own passphrase if it resolves
# to the real "Public Global Stellar Network" passphrase, in case a custom
# network alias under an allowed-sounding name points at production RPC.
# ---------------------------------------------------------------------------

network_lc=$(printf '%s' "$NETWORK" | tr '[:upper:]' '[:lower:]')
case "$network_lc" in
    testnet|futurenet|local|standalone) ;;
    *)
        echo "Refusing to run: network '$NETWORK' is not an allowed test network" >&2
        echo "(allowed: testnet, futurenet, local, standalone). This script never" >&2
        echo "runs against mainnet/public, by design, and has no override for it." >&2
        exit 1
        ;;
esac

passphrase=$(stellar network ls --long 2>/dev/null | awk -v n="$network_lc" '
    BEGIN { IGNORECASE = 1 }
    tolower($0) == "name: " n { found = 1; next }
    /^Name:/ { found = 0 }
    found && /^Network passphrase:/ { print; exit }
' || true)
case "$passphrase" in
    *"Public Global Stellar Network"*)
        echo "Refusing to run: network '$NETWORK' resolves to the mainnet passphrase." >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Validate required inputs
# ---------------------------------------------------------------------------

case "$ACCOUNTS" in
    ''|*[!0-9]*)
        echo "Error: --accounts must be a positive integer." >&2
        exit 1
        ;;
esac
if [ "$ACCOUNTS" -lt 1 ]; then
    echo "Error: --accounts must be at least 1." >&2
    exit 1
fi

case "$AMOUNT" in
    ''|*[!0-9]*)
        echo "Error: --amount must be a positive integer (raw base units)." >&2
        exit 1
        ;;
esac
if [ "$AMOUNT" -lt 1 ]; then
    echo "Error: --amount must be at least 1." >&2
    exit 1
fi

if [ -z "$CONTRACT_ID" ]; then
    echo "Error: --contract-id (or \$CONTRACT_ID) is required." >&2
    exit 1
fi

echo "Network:     $NETWORK"
echo "Vault:       $CONTRACT_ID"
echo "Accounts:    $ACCOUNTS"
echo "Amount each: $AMOUNT raw units"
echo

# ---------------------------------------------------------------------------
# Test token: reuse one, or deploy and mint from a fresh one.
# ---------------------------------------------------------------------------

if [ -z "$TOKEN_ID" ]; then
    if [ -z "$TOKEN_ISSUER" ]; then
        TOKEN_ISSUER="${PREFIX}-issuer"
        echo "No --token-issuer given; generating and funding '$TOKEN_ISSUER' as the test asset's issuer..."
        stellar keys generate --network "$NETWORK" --overwrite "$TOKEN_ISSUER" >/dev/null
        stellar keys fund "$TOKEN_ISSUER" --network "$NETWORK" >/dev/null
    fi
    ISSUER_ADDRESS=$(stellar keys address "$TOKEN_ISSUER")
    echo "Deploying test asset $ASSET_CODE:$ISSUER_ADDRESS..."
    TOKEN_ID=$(stellar contract asset deploy --asset "$ASSET_CODE:$ISSUER_ADDRESS" --source "$TOKEN_ISSUER" --network "$NETWORK" | tail -n 1)
    echo "Deployed test token: $TOKEN_ID"
else
    if [ -z "$TOKEN_ISSUER" ]; then
        echo "Error: --token-issuer (or \$TOKEN_ISSUER) is required when --token-id is already set," >&2
        echo "since only the issuer can mint $ASSET_CODE to new accounts." >&2
        exit 1
    fi
    ISSUER_ADDRESS=$(stellar keys address "$TOKEN_ISSUER")
    echo "Reusing test token: $TOKEN_ID (issuer: $TOKEN_ISSUER / $ISSUER_ADDRESS)"
fi
echo

# ---------------------------------------------------------------------------
# Seed each account: create, friendbot-fund, trust the test asset, mint it,
# and deposit into the vault. Each account is independent — one failing
# doesn't stop the rest; it's recorded in the summary table instead.
# ---------------------------------------------------------------------------

names=()
addresses=()
minted=()
shares=()
statuses=()

i=1
while [ "$i" -le "$ACCOUNTS" ]; do
    name="${PREFIX}-${i}"
    echo "=== Account $i/$ACCOUNTS: $name ==="

    status="ok"
    address="-"
    minted_amount="-"
    shares_minted="-"

    if stellar keys generate --network "$NETWORK" --overwrite "$name" >/dev/null 2>&1 && \
            address=$(stellar keys address "$name") && \
            stellar keys fund "$name" --network "$NETWORK" >/dev/null 2>&1; then
        echo "  Created and funded $address (friendbot)"
    else
        echo "  FAILED: could not create/fund $name" >&2
        status="funding_failed"
        address="-"
    fi

    if [ "$status" = "ok" ]; then
        if stellar tx new change-trust --source "$name" --network "$NETWORK" \
                --line "$ASSET_CODE:$ISSUER_ADDRESS" >/dev/null 2>&1 && \
                stellar contract invoke --id "$TOKEN_ID" --source "$TOKEN_ISSUER" --network "$NETWORK" \
                    --send=yes -- mint --to "$address" --amount "$AMOUNT" >/dev/null 2>&1; then
            minted_amount="$AMOUNT"
            echo "  Minted $AMOUNT $ASSET_CODE"
        else
            echo "  FAILED: could not mint $ASSET_CODE for $name" >&2
            status="mint_failed"
        fi
    fi

    if [ "$status" = "ok" ]; then
        if stake_out=$(stellar contract invoke --id "$CONTRACT_ID" --source "$name" --network "$NETWORK" \
                -- stake --staker "$address" --amount "$AMOUNT" 2>&1); then
            shares_minted=$(printf '%s\n' "$stake_out" | tail -n 1 | tr -d '"')
            echo "  Deposited $AMOUNT into the vault (minted $shares_minted shares)"
        else
            echo "  FAILED: could not deposit for $name" >&2
            echo "$stake_out" | sed 's/^/    /' >&2
            status="deposit_failed"
        fi
    fi

    names+=("$name")
    addresses+=("$address")
    minted+=("$minted_amount")
    shares+=("$shares_minted")
    statuses+=("$status")

    i=$((i + 1))
done

# ---------------------------------------------------------------------------
# Summary table
# ---------------------------------------------------------------------------

echo
echo "Summary"
printf '%-12s  %-56s  %-14s  %-14s  %s\n' "Name" "Address" "Minted" "Shares" "Status"
printf '%-12s  %-56s  %-14s  %-14s  %s\n' "------------" "--------------------------------------------------------" "--------------" "--------------" "------"
idx=0
ok_count=0
while [ "$idx" -lt "${#names[@]}" ]; do
    printf '%-12s  %-56s  %-14s  %-14s  %s\n' \
        "${names[$idx]}" "${addresses[$idx]}" "${minted[$idx]}" "${shares[$idx]}" "${statuses[$idx]}"
    if [ "${statuses[$idx]}" = "ok" ]; then
        ok_count=$((ok_count + 1))
    fi
    idx=$((idx + 1))
done
echo
echo "$ok_count/$ACCOUNTS accounts fully seeded (created, funded, minted, deposited)."
if [ "$ok_count" -lt "$ACCOUNTS" ]; then
    echo "See the FAILED lines above for details on the rest." >&2
    exit 1
fi
