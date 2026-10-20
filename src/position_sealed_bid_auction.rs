//! Sealed-bid position auction (issue #403).
//!
//! Distinct from issue #211's peer-to-peer AMM swap (agreed ratio, two
//! willing parties). Here a staker lists their whole position for sale;
//! bidders commit to a hidden bid during a commit phase, reveal it during a
//! reveal phase, and the highest revealed bid wins the position. Modeled on
//! `commitment.rs`'s commitâ€“reveal hashing (`SHA256(amount_bytes || salt)`).
//!
//! # Fund custody
//!
//! A bidder "locks funds" at commit time even though the bid amount is still
//! hidden by pre-funding the auction's `min_bid` as escrow when committing,
//! then topping up to their revealed amount (if higher) at reveal time. An
//! unrevealed bid's locked `min_bid` is forfeited to the slash treasury as
//! the anti-gaming measure the issue's notes call for â€” silently reneging on
//! a commitment costs the same as never having bid, plus the forfeited
//! deposit.
//!
//! # Position custody
//!
//! This contract's positions live as a plain `ShareBalance(Address) -> i128`
//! entry (see `balance.rs`); there is no separate NFT/receipt object to hand
//! off. Listing an auction moves the seller's shares out of their own
//! balance into the listing itself (`AuctionListing::escrowed_shares`) so
//! they can't be staked/unstaked away mid-auction; `total_shares`/
//! `total_deposited` are left untouched throughout, since the underlying
//! claim never leaves the pool, only its owner-of-record changes at
//! settlement. `AuctionListing` therefore carries one field beyond the
//! issue's literal struct â€” `escrowed_shares` â€” since without it there is
//! nothing to actually transfer to the winner.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs` and `commitment.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ MAX_AUCTION_BIDS};

/// Instance-storage key for the next auction id counter.
const NEXT_ID_KEY: Symbol = symbol_short!("psa_next");
/// Persistent-storage key prefix for an auction listing. Keyed by
/// `(LISTING_KEY, auction_id)`.
const LISTING_KEY: Symbol = symbol_short!("psa_list");
/// Persistent-storage key prefix for a seller's currently active auction id.
/// Keyed by `(SELLER_KEY, seller)`.
const SELLER_KEY: Symbol = symbol_short!("psa_slr");
/// Persistent-storage key prefix for one bidder's bid on an auction. Keyed by
/// `(BID_KEY, auction_id, bidder)`.
const BID_KEY: Symbol = symbol_short!("psa_bid");
/// Persistent-storage key prefix for an auction's bidder list. Keyed by
/// `(BIDDERS_KEY, auction_id)`.
const BIDDERS_KEY: Symbol = symbol_short!("psa_bdrs");

/// A sealed-bid auction listing for a staking position.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AuctionListing {
    pub seller: Address,
    pub min_bid: i128,
    pub commit_deadline: u32,
    pub reveal_deadline: u32,
    pub highest_bid: i128,
    pub winner: Option<Address>,
    pub settled: bool,
    /// Shares moved out of the seller's balance at listing time; see the
    /// module-level "Position custody" note.
    pub escrowed_shares: i128,
}

/// One bidder's commitâ€“reveal state for an auction.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AuctionBidRecord {
    pub hash: Bytes,
    pub locked_amount: i128,
    pub revealed: bool,
    pub amount: i128,
}

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn next_auction_id(env: &Env) -> u32 {
    let id: u32 = env.storage().instance().get(&NEXT_ID_KEY).unwrap_or(0);
    env.storage().instance().set(&NEXT_ID_KEY, &(id + 1));
    id
}

fn get_listing(env: &Env, auction_id: u32) -> Option<AuctionListing> {
    env.storage()
        .persistent()
        .get(&(LISTING_KEY, auction_id))
}

fn set_listing(env: &Env, auction_id: u32, listing: &AuctionListing) {
    env.storage()
        .persistent()
        .set(&(LISTING_KEY, auction_id), listing);
}

fn get_seller_active(env: &Env, seller: &Address) -> Option<u32> {
    env.storage().persistent().get(&(SELLER_KEY, seller.clone()))
}

fn set_seller_active(env: &Env, seller: &Address, auction_id: u32) {
    env.storage()
        .persistent()
        .set(&(SELLER_KEY, seller.clone()), &auction_id);
}

fn clear_seller_active(env: &Env, seller: &Address) {
    env.storage()
        .persistent()
        .remove(&(SELLER_KEY, seller.clone()));
}

fn get_bid(env: &Env, auction_id: u32, bidder: &Address) -> Option<AuctionBidRecord> {
    env.storage()
        .persistent()
        .get(&(BID_KEY, auction_id, bidder.clone()))
}

fn set_bid(env: &Env, auction_id: u32, bidder: &Address, bid: &AuctionBidRecord) {
    env.storage()
        .persistent()
        .set(&(BID_KEY, auction_id, bidder.clone()), bid);
}

fn get_bidders(env: &Env, auction_id: u32) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&(BIDDERS_KEY, auction_id))
        .unwrap_or(Vec::new(env))
}

fn set_bidders(env: &Env, auction_id: u32, bidders: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&(BIDDERS_KEY, auction_id), bidders);
}

/// The preimage a bid commitment hashes: the amount's big-endian bytes, then
/// salt. Mirrors `commitment.rs::preimage`.
fn preimage(env: &Env, amount: i128, salt: &Bytes) -> Bytes {
    let mut buffer = Bytes::new(env);
    for byte in amount.to_be_bytes().iter() {
        buffer.push_back(*byte);
    }
    buffer.append(salt);
    buffer
}

/// Compute the commit hash for an `(amount, salt)` pair. Exposed (like
/// `commitment.rs::compute_hash`) so a caller â€” or a test â€” can build the
/// commitment with exactly the encoding `reveal_bid` verifies against.
pub(crate) fn compute_bid_hash(env: &Env, amount: i128, salt: &Bytes) -> Bytes {
    env.crypto().sha256(&preimage(env, amount, salt)).into()
}

fn token_address(env: &Env) -> Result<Address, VaultError> {
    env.storage()
        .instance()
        .get(&crate::storage::DataKey::Token)
        .ok_or(VaultError::NotInitialized)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// List the caller's entire staking position for sale via sealed-bid
    /// auction. Only one active (unsettled) auction per seller at a time.
    ///
    /// `commit_duration` / `reveal_duration` are both measured in ledgers
    /// from now, back to back: the commit phase runs until
    /// `now + commit_duration`, the reveal phase from there until
    /// `now + commit_duration + reveal_duration`.
    pub fn list_position_for_auction(
        env: Env,
        user: Address,
        min_bid: i128,
        commit_duration: u32,
        reveal_duration: u32,
    ) -> Result<u32, VaultError> {
        user.require_auth();

        if min_bid <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if commit_duration == 0 || reveal_duration == 0 {
            return Err(VaultError::ZeroAmount);
        }
        if let Some(existing_id) = get_seller_active(&env, &user) {
            let existing = get_listing(&env, existing_id);
            if matches!(existing, Some(l) if !l.settled) {
                return Err(VaultError::SellerAuctionAlreadyActive);
            }
        }

        let shares = balance::get_shares(&env, &user);
        if shares == 0 {
            return Err(VaultError::PositionNotFound);
        }

        // Move the position out of the seller's own balance for the
        // duration of the auction â€” see the module-level "Position custody"
        // note. `total_shares`/`total_deposited` are untouched.
        balance::set_shares(&env, &user, 0);

        let now = env.ledger().sequence();
        let commit_deadline = now.saturating_add(commit_duration);
        let reveal_deadline = commit_deadline.saturating_add(reveal_duration);

        let auction_id = next_auction_id(&env);
        let listing = AuctionListing {
            seller: user.clone(),
            min_bid,
            commit_deadline,
            reveal_deadline,
            highest_bid: 0,
            winner: None,
            settled: false,
            escrowed_shares: shares,
        };
        set_listing(&env, auction_id, &listing);
        set_seller_active(&env, &user, auction_id);

        env.events().publish(
            (symbol_short!("psa_list"), user),
            (auction_id, min_bid, commit_deadline, reveal_deadline),
        );
        Ok(auction_id)
    }

    /// Commit to a sealed bid on `auction_id`, locking `min_bid` as escrow.
    /// Callable again before the commit deadline to replace a prior,
    /// unrevealed hash without locking additional funds.
    pub fn commit_bid(
        env: Env,
        bidder: Address,
        auction_id: u32,
        bid_hash: Bytes,
    ) -> Result<(), VaultError> {
        bidder.require_auth();

        let listing = get_listing(&env, auction_id).ok_or(VaultError::AuctionNotFound)?;
        if listing.settled {
            return Err(VaultError::AuctionAlreadySettled);
        }
        if env.ledger().sequence() > listing.commit_deadline {
            return Err(VaultError::AuctionPhaseClosed);
        }
        if bid_hash.len() != 32 {
            return Err(VaultError::InvalidAddress);
        }

        match get_bid(&env, auction_id, &bidder) {
            Some(mut existing) if !existing.revealed => {
                existing.hash = bid_hash;
                set_bid(&env, auction_id, &bidder, &existing);
            }
            Some(_) => {
                // Already revealed; nothing further to commit.
                return Err(VaultError::AuctionPhaseClosed);
            }
            None => {
                let token = token_address(&env)?;
                soroban_sdk::token::Client::new(&env, &token).transfer(
                    &bidder,
                    &env.current_contract_address(),
                    &listing.min_bid,
                );

                let mut bidders = get_bidders(&env, auction_id);
                if bidders.len() >= MAX_AUCTION_BIDS {
                    return Err(VaultError::BatchTooLarge);
                }
                bidders.push_back(bidder.clone());
                set_bidders(&env, auction_id, &bidders);

                set_bid(
                    &env,
                    auction_id,
                    &bidder,
                    &AuctionBidRecord {
                        hash: bid_hash,
                        locked_amount: listing.min_bid,
                        revealed: false,
                        amount: 0,
                    },
                );
            }
        }

        env.events()
            .publish((symbol_short!("psa_cmt"), bidder), auction_id);
        Ok(())
    }

    /// Reveal a previously committed bid. Reverts if the hash doesn't match
    /// or the reveal window isn't open. A reveal below `min_bid` is ignored
    /// (left unrevealed) rather than reverting, per the issue's notes â€” its
    /// locked deposit is still forfeited at `refund_losing_bids` time.
    pub fn reveal_bid(
        env: Env,
        bidder: Address,
        auction_id: u32,
        amount: i128,
        salt: Bytes,
    ) -> Result<(), VaultError> {
        bidder.require_auth();

        let mut listing = get_listing(&env, auction_id).ok_or(VaultError::AuctionNotFound)?;
        if listing.settled {
            return Err(VaultError::AuctionAlreadySettled);
        }
        let now = env.ledger().sequence();
        if now <= listing.commit_deadline || now > listing.reveal_deadline {
            return Err(VaultError::AuctionPhaseClosed);
        }

        let mut bid = get_bid(&env, auction_id, &bidder).ok_or(VaultError::BidNotFound)?;
        if bid.revealed {
            return Ok(());
        }

        let expected = compute_bid_hash(&env, amount, &salt);
        if expected != bid.hash {
            return Err(VaultError::InvalidAddress);
        }

        if amount < listing.min_bid {
            // Below the minimum: reveal is ignored, bid stays unrevealed and
            // its locked deposit forfeits at settlement.
            return Ok(());
        }

        if amount > bid.locked_amount {
            let additional = amount - bid.locked_amount;
            let token = token_address(&env)?;
            soroban_sdk::token::Client::new(&env, &token).transfer(
                &bidder,
                &env.current_contract_address(),
                &additional,
            );
        }
        bid.locked_amount = amount;
        bid.revealed = true;
        bid.amount = amount;
        set_bid(&env, auction_id, &bidder, &bid);

        if amount > listing.highest_bid {
            listing.highest_bid = amount;
            listing.winner = Some(bidder.clone());
            set_listing(&env, auction_id, &listing);
        }

        env.events()
            .publish((symbol_short!("psa_rvl"), bidder), (auction_id, amount));
        Ok(())
    }

    /// Settle `auction_id` once the reveal deadline has passed: transfers
    /// the position to the highest revealed bidder and the winning bid to
    /// the seller, or (with no valid reveals) returns the position to the
    /// seller unchanged. Callable by anyone.
    pub fn settle_auction(env: Env, auction_id: u32) -> Result<(), VaultError> {
        let mut listing = get_listing(&env, auction_id).ok_or(VaultError::AuctionNotFound)?;
        if listing.settled {
            return Err(VaultError::AuctionAlreadySettled);
        }
        if env.ledger().sequence() <= listing.reveal_deadline {
            return Err(VaultError::AuctionPhaseClosed);
        }

        listing.settled = true;

        match &listing.winner {
            Some(winner) => {
                let winner_shares = balance::get_shares(&env, winner);
                balance::set_shares(&env, winner, winner_shares + listing.escrowed_shares);

                let token = token_address(&env)?;
                soroban_sdk::token::Client::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &listing.seller,
                    &listing.highest_bid,
                );
            }
            None => {
                let seller_shares = balance::get_shares(&env, &listing.seller);
                balance::set_shares(
                    &env,
                    &listing.seller,
                    seller_shares + listing.escrowed_shares,
                );
            }
        }

        clear_seller_active(&env, &listing.seller);
        set_listing(&env, auction_id, &listing);

        env.events().publish(
            (symbol_short!("psa_stl"), listing.seller.clone()),
            (
                auction_id,
                listing.winner.clone(),
                listing.highest_bid,
                env.ledger().sequence(),
            ),
        );
        Ok(())
    }

    /// Refund every non-winning revealed bidder's locked deposit, and
    /// forfeit every non-revealed bidder's locked deposit to the slash
    /// treasury. Callable once `auction_id` is settled; safe to call more
    /// than once (already-refunded bids are skipped). Returns the number of
    /// bidders processed.
    pub fn refund_losing_bids(env: Env, auction_id: u32) -> Result<u32, VaultError> {
        let listing = get_listing(&env, auction_id).ok_or(VaultError::AuctionNotFound)?;
        if !listing.settled {
            return Err(VaultError::AuctionPhaseClosed);
        }

        let token = token_address(&env)?;
        let token_client = soroban_sdk::token::Client::new(&env, &token);
        let treasury = balance::get_slash_treasury(&env);

        let bidders = get_bidders(&env, auction_id);
        let mut processed: u32 = 0;

        for bidder in bidders.iter() {
            if matches!(&listing.winner, Some(w) if w == &bidder) {
                continue;
            }
            let Some(mut bid) = get_bid(&env, auction_id, &bidder) else {
                continue;
            };
            if bid.locked_amount <= 0 {
                continue;
            }

            if bid.revealed {
                token_client.transfer(
                    &env.current_contract_address(),
                    &bidder,
                    &bid.locked_amount,
                );
                env.events().publish(
                    (symbol_short!("psa_rfd"), bidder.clone()),
                    (auction_id, bid.locked_amount),
                );
            } else if let Some(treasury_addr) = &treasury {
                token_client.transfer(
                    &env.current_contract_address(),
                    treasury_addr,
                    &bid.locked_amount,
                );
                env.events().publish(
                    (symbol_short!("psa_frf"), bidder.clone()),
                    (auction_id, bid.locked_amount),
                );
            }

            bid.locked_amount = 0;
            set_bid(&env, auction_id, &bidder, &bid);
            processed += 1;
        }

        Ok(processed)
    }

    /// Read-only query: an auction listing by id.
    pub fn get_auction(env: Env, auction_id: u32) -> Option<AuctionListing> {
        get_listing(&env, auction_id)
    }

    /// Read-only query: a bidder's bid record on an auction.
    pub fn get_auction_bid(
        env: Env,
        auction_id: u32,
        bidder: Address,
    ) -> Option<AuctionBidRecord> {
        get_bid(&env, auction_id, &bidder)
    }

    /// Read-only query: `seller`'s currently active (unsettled) auction id,
    /// if any.
    pub fn get_seller_active_auction(env: Env, seller: Address) -> Option<u32> {
        match get_seller_active(&env, &seller) {
            Some(id) => match get_listing(&env, id) {
                Some(l) if !l.settled => Some(id),
                _ => None,
            },
            None => None,
        }
    }
}









