# Contributor Guide

A step-by-step walkthrough for first-time Soroban contributors — from environment setup to a passing PR.

---

## 1. Environment Setup

### Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

### WASM target

```bash
rustup target add wasm32-unknown-unknown
```

### Stellar CLI (optional, for local deployment)

```bash
cargo install --locked stellar-cli
```

### Verify everything works

```bash
rustc --version
cargo --version
rustup target list --installed | grep wasm
```

---

## 2. How the Contract Works

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     VaultContract                           │
│                                                             │
│  ┌──────────┐   ┌──────────┐   ┌───────────┐              │
│  │  admin.rs │   │balance.rs│   │storage.rs │              │
│  │           │   │          │   │           │              │
│  │ get_admin │   │ get_shares│  │ DataKey   │              │
│  │ set_admin │   │ set_shares│  │ enum      │              │
│  │ require_  │   │ shares_to│  │           │              │
│  │  admin    │   │  _amount │  │ VaultState│              │
│  └──────────┘   └──────────┘   └───────────┘              │
│                                                             │
│  ┌──────────┐   ┌──────────┐                               │
│  │ events.rs │   │errors.rs │                               │
│  │           │   │          │                               │
│  │ deposit   │   │VaultError│                               │
│  │ withdraw  │   │ enum     │                               │
│  │ paused    │   │          │                               │
│  └──────────┘   └──────────┘                               │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                    vault.rs                          │   │
│  │                                                      │   │
│  │  Public entry points:                                │   │
│  │    initialize, stake, unstake, claim, deposit,       │   │
│  │    withdraw, pause, unpause, add_yield,              │   │
│  │    set_reward_rate_bps, fund_reward_pool,            │   │
│  │    set_boost_schedule, request_unstake,              │   │
│  │    execute_unstake, slash, simulate_*                │   │
│  │                                                      │   │
│  │  Read-only queries:                                  │   │
│  │    shares_of, get_admin, get_version, is_paused,     │   │
│  │    position_of, total_staked, vault_state,           │   │
│  │    pool_stats, user_stats, calc_pending_reward,      │   │
│  │    get_stake_token, total_rewards_paid,              │   │
│  │    simulate_stake, simulate_compound,                │   │
│  │    simulate_boost_impact                             │   │
│  │                                                      │   │
│  │  Internal helpers:                                   │   │
│  │    do_stake, do_unstake, accrue_rewards,             │   │
│  │    reward_between_ledgers, reward_for_ledgers,       │   │
│  │    require_min_stake, record_stake_snapshot          │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Key Concepts

- **Shares**: When a user stakes tokens, they receive shares. First deposit is 1:1; subsequent deposits are proportional to existing pool value.
- **Reward Accrual**: Rewards accrue per-ledger based on `reward_rate_bps` and the user's boost multiplier.
- **Boost Tiers**: A time-based multiplier schedule that increases rewards the longer a user stakes.
- **Cooldown Flow**: When `cooldown_period > 0`, unstaking goes through `request_unstake` → wait → `execute_unstake`.
- **Slash**: Admin can slash a user's principal, sending tokens to a treasury address.

---

## 3. Step-by-Step: Picking Up an Issue

1. **Find an issue**: Browse open issues tagged **Stellar Wave** at [drips.network/wave/stellar](https://www.drips.network/wave/stellar/issues).
2. **Apply**: Use the Drips Wave app to apply. Wait to be **assigned** before starting.
3. **Fork and branch**:
   ```bash
   git clone https://github.com/YOUR_USERNAME/stellar-defi-vault.git
   cd stellar-defi-vault
   git checkout -b feat/<issue-number>-short-description
   ```
4. **Implement**: Make your changes (see worked example below).
5. **Test**:
   ```bash
   cargo test --features testutils
   ```
6. **Lint**:
   ```bash
   cargo fmt
   cargo clippy --features testutils
   ```
7. **Commit and push**:
   ```bash
   git add <files>
   git commit -m "feat: <short description>"
   git push origin feat/<issue-number>-short-description
   ```
8. **Open a PR**: Include `Closes #<issue-number>` in the description.

---

## 4. Worked Example: Adding a New Read-Only Query

**Issue**: Add a function to query the total number of stakers.

### Step 1: Add the storage key (if needed)

Check `src/storage.rs` — `TotalStakers` already exists in the `DataKey` enum, so no change needed here.

### Step 2: Add the balance helper (if needed)

Check `src/balance.rs` — `get_total_stakers` and `set_total_stakers` already exist.

### Step 3: Add the public function

In `src/vault.rs`, add inside the `#[contractimpl] impl VaultContract` block:

```rust
/// Read-only query for the total number of active stakers.
pub fn total_stakers(env: Env) -> Result<u32, VaultError> {
    let _ = admin::get_admin(&env)?;
    Ok(balance::get_total_stakers(&env))
}
```

### Step 4: Add a test

In `src/test.rs`, add:

```rust
#[test]
fn test_total_stakers_query() {
    let f = VaultFixture::new();
    assert_eq!(f.vault.total_stakers(), 0);

    f.vault.stake(&f.alice, &100_000);
    assert_eq!(f.vault.total_stakers(), 1);

    f.vault.stake(&f.bob, &200_000);
    assert_eq!(f.vault.total_stakers(), 2);
}
```

### Step 5: Verify

```bash
cargo test --features testutils
cargo clippy --features testutils
cargo fmt --check
```

---

## 5. Common Mistakes and How to Fix Them

### Clippy warnings

```bash
cargo clippy --features testutils
```

Common fixes:
- Unused variables: prefix with `_` (e.g., `_env`)
- `clone()` on `Copy` types: remove the `.clone()`
- Needless borrows: remove `&` where not needed

### Test failures

Run a specific test:
```bash
cargo test test_name --features testutils
```

Common issues:
- Forgetting `env.mock_all_auths()` — transactions will fail with auth errors
- Not advancing the ledger with `set_ledger()` before asserting time-dependent behavior
- Using `unwrap()` on `Option` that is `None` — check storage defaults

### WASM build issues

```bash
cargo build --target wasm32-unknown-unknown --release
```

Common fixes:
- Ensure `wasm32-unknown-unknown` target is installed: `rustup target add wasm32-unknown-unknown`
- `panic = "abort"` is required in release profile (already set in `Cargo.toml`)
- No `std` library calls allowed — use `no_std` compatible code

---

## 6. Running Specific Tests

```bash
# Run all tests
cargo test --features testutils

# Run a specific test by name
cargo test test_total_rewards_paid_starts_at_zero --features testutils

# Run tests matching a pattern
cargo test stake --features testutils

# Run tests with output displayed
cargo test --features testutils -- --nocapture
```

---

## 7. Code Style

- Run `cargo fmt` before committing.
- Ensure `cargo clippy --features testutils` passes with no warnings.
- All new functionality must include unit tests.
- Public functions must have doc comments (`///`).
- Prefer `checked_add` / `checked_sub` for arithmetic to avoid panics.
- Use existing storage helpers in `balance.rs` — don't write raw storage access.

---

## 8. PR Checklist

- [ ] Tests pass (`cargo test --features testutils`)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --features testutils` passes with no warnings
- [ ] New logic is covered by tests
- [ ] PR description references the issue (`Closes #N`)
- [ ] Doc comments added for all new public functions
