# [WAVE] Add Stellar testnet deployment script

**Labels:** `Stellar Wave`, `devex`, `good first issue`
**Suggested Points:** 100

## Summary

Create a shell script (or Makefile targets) that automates deploying the vault contract to Stellar testnet using the Stellar CLI. New contributors should be able to go from zero to a running vault in one command.

## Acceptance Criteria

- [ ] Script `scripts/deploy_testnet.sh` that:
  - Builds the WASM contract
  - Creates a funded testnet keypair (or accepts one via env var `SECRET_KEY`)
  - Deploys the contract using `stellar contract deploy`
  - Calls `initialize` with the deployer as admin and a testnet asset as the token
  - Prints the contract ID on success
- [ ] `Makefile` with targets: `make build`, `make test`, `make deploy-testnet`
- [ ] README section updated: "Deploying to Testnet" with step-by-step instructions
- [ ] Script handles missing `stellar` CLI gracefully (prints install instructions)

## Notes

- Use the Stellar testnet (`https://soroban-testnet.stellar.org`) and Friendbot for funding.
- Do not hard-code any private keys — use env vars.
- Test the script end-to-end before submitting the PR.
