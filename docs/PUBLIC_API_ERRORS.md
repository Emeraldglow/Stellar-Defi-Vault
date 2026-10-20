# Public API error behavior

Public validation failures return typed contract errors. The #608 pass removes
direct panics from waitlist admission, treasury-split configuration, reward
swap route lookup, admin-timelock execution, auto-compounding, and position-NFT
tokenization/redemption.

The remaining `unwrap` calls in public implementation code are bounded vector
accesses where the index is derived from that same vector's length, or internal
state invariants after a preceding presence check. Soroban SDK authorization
and cross-contract invocation failures can still abort the transaction; those
are host-level failures rather than recoverable input validation performed by
this contract.
