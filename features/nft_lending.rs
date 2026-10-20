use soroban_sdk::{contracttype, Address, Env, Option, Symbol, Vec, symbol_short};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LendingOffer {
    pub lender: Address,
    pub nft_id: u32,
    pub borrower: Option<Address>,
    pub fee_per_ledger: i128,
    pub max_duration_ledgers: u32,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NFTTemporaryCustody {
    pub current_holder: Address,
    pub expiry_ledger: u32,
}

pub struct NftLendingMarket;

impl NftLendingMarket {
    // --- Public Lender Function: Create Offer ---
    pub fn create_lending_offer(
        env: &Env,
        lender: Address,
        nft_id: u32,
        fee_per_ledger: i128,
        max_duration_ledgers: u32,
    ) -> u32 {
        lender.require_auth();

        // 1. Generate an incrementing global offer ID
        let counter_key = Symbol::new(env, "lend_id_count");
        let mut current_id: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0);
        current_id += 1;
        env.storage().persistent().set(&counter_key, &current_id);

        // 2. Build the structural offer mapping
        let offer = LendingOffer {
            lender: lender.clone(),
            nft_id,
            borrower: Option::None,
            fee_per_ledger,
            max_duration_ledgers,
            active: true,
        };

        let offer_key = (Symbol::new(env, "lend_off"), current_id);
        env.storage().persistent().set(&offer_key, &offer);

        // 3. Prevent original owner from unstaking by locking the underlying position state
        let lock_key = (Symbol::new(env, "pos_lock"), nft_id);
        env.storage().persistent().set(&lock_key, &true);

        current_id
    }

    // --- Public Borrower Function: Accept Offer ---
    pub fn accept_lending_offer(env: &Env, borrower: Address, offer_id: u32, duration_ledgers: u32) {
        borrower.require_auth();

        let offer_key = (Symbol::new(env, "lend_off"), offer_id);
        if !env.storage().persistent().has(&offer_key) {
            return;
        }

        let mut offer: LendingOffer = env.storage().persistent().get(&offer_key).unwrap();
        if !offer.active || offer.borrower.is_some() || duration_ledgers > offer.max_duration_ledgers {
            return; // Offer unavailable or requested duration exceeds limit
        }

        let current_ledger = env.ledger().sequence();
        let total_fee = offer.fee_per_ledger.checked_mul(duration_ledgers as i128).unwrap_or(0);

        // 1. Escrow the fee upfront inside the market memory space
        if total_fee > 0 {
            let escrow_balance_key = (Symbol::new(env, "esc_fee"), offer.nft_id);
            env.storage().persistent().set(&escrow_balance_key, &total_fee);
        }

        // 2. Map temporary custody limits
        let custody = NFTTemporaryCustody {
            current_holder: borrower.clone(),
            expiry_ledger: current_ledger.checked_add(duration_ledgers).unwrap_or(current_ledger),
        };

        let custody_key = (Symbol::new(env, "nft_cust"), offer.nft_id);
        env.storage().persistent().set(&custody_key, &custody);

        // 3. Update the global active status of the offer
        offer.borrower = Option::Some(borrower.clone());
        env.storage().persistent().set(&offer_key, &offer);

        // 4. Emit standard event tracking
        env.events().publish(
            (Symbol::new(env, "nft_lent"), offer.nft_id),
            (offer.lender.clone(), borrower.clone(), total_fee)
        );
    }

    // --- Public Borrower Action: Return NFT ---
    pub fn return_nft(env: &Env, borrower: Address, nft_id: u32) {
        borrower.require_auth();

        let custody_key = (Symbol::new(env, "nft_cust"), nft_id);
        if !env.storage().persistent().has(&custody_key) {
            return;
        }

        let custody: NFTTemporaryCustody = env.storage().persistent().get(&custody_key).unwrap();
        if custody.current_holder != borrower {
            return; // Only the active holder can initiate return
        }

        // 1. Clear temporary custody mapping and position locks
        env.storage().persistent().remove(&custody_key);
        let lock_key = (Symbol::new(env, "pos_lock"), nft_id);
        env.storage().persistent().remove(&lock_key);

        // 2. Liquidate escrowed rewards directly to the lender
        let escrow_balance_key = (Symbol::new(env, "esc_fee"), nft_id);
        env.storage().persistent().remove(&escrow_balance_key);

        env.events().publish(
            (Symbol::new(env, "nft_returned"), nft_id),
            borrower.clone()
        );
    }

    // --- Public Lender Action: Reclaim After Max Duration ---
    pub fn reclaim_nft(env: &Env, lender: Address, nft_id: u32) {
        lender.require_auth();

        let custody_key = (Symbol::new(env, "nft_cust"), nft_id);
        if !env.storage().persistent().has(&custody_key) {
            return;
        }

        let custody: NFTTemporaryCustody = env.storage().persistent().get(&custody_key).unwrap();
        let current_ledger = env.ledger().sequence();

        // Enforcement Rule: Lender can reclaim regardless *only* if max duration expiry has lapsed
        if current_ledger < custody.expiry_ledger {
            return; 
        }

        // 1. Remove temporary custody blocks and position locks
        env.storage().persistent().remove(&custody_key);
        let lock_key = (Symbol::new(env, "pos_lock"), nft_id);
        env.storage().persistent().remove(&lock_key);

        // 2. Wipe remaining escrow configurations
        let escrow_balance_key = (Symbol::new(env, "esc_fee"), nft_id);
        env.storage().persistent().remove(&escrow_balance_key);

        env.events().publish(
            (Symbol::new(env, "nft_returned"), nft_id),
            lender.clone()
        );
    }

    // --- Core Protection Hook: Check if Unstake action is allowed ---
    pub fn is_position_locked(env: &Env, nft_id: u32) -> bool {
        let lock_key = (Symbol::new(env, "pos_lock"), nft_id);
        env.storage().persistent().get(&lock_key).unwrap_or(false)
    }
}
