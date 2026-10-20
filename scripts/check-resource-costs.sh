#!/usr/bin/env bash
set -euo pipefail

# check-resource-costs.sh
# Verifies that COSTS.md is present and all canonical public functions are documented.

echo "=== Verifying Resource Cost Documentation (Issue #634) ==="

COSTS_FILE="COSTS.md"

if [[ ! -f "${COSTS_FILE}" ]]; then
    echo "Error: ${COSTS_FILE} not found!"
    exit 1
fi

REQUIRED_FUNCS=(
    "initialize"
    "transfer_admin"
    "stake"
    "deposit"
    "withdraw"
    "unstake"
    "claim"
    "staked_amount"
    "pending_reward"
    "total_staked"
    "is_paused"
    "vault_state"
    "set_reward_rate_bps"
    "set_emergency_admin"
    "get_version"
)

MISSING=0
for func in "${REQUIRED_FUNCS[@]}"; do
    if ! grep -q "\`${func}\`" "${COSTS_FILE}"; then
        echo "Missing documentation for public function: ${func}"
        MISSING=$((MISSING + 1))
    fi
done

if [[ ${MISSING} -gt 0 ]]; then
    echo "Error: ${MISSING} required functions are missing from ${COSTS_FILE}."
    exit 1
fi

echo "SUCCESS: All required public functions are documented in ${COSTS_FILE}."
exit 0
