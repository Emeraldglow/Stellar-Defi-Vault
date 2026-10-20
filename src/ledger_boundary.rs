//! Shared inclusive ledger-boundary semantics.
//!
//! A deadline at ledger `N` is reached at `N`: `N - 1` is before the
//! boundary, and `N` plus every later ledger is at or after it.

pub const fn is_before(now: u32, boundary: u32) -> bool {
    now < boundary
}

pub const fn is_reached(now: u32, boundary: u32) -> bool {
    now >= boundary
}

pub const fn duration_elapsed(now: u32, started_at: u32, duration: u32) -> bool {
    now.saturating_sub(started_at) >= duration
}

#[cfg(test)]
mod tests {
    use super::{duration_elapsed, is_before, is_reached};

    #[test]
    fn absolute_boundary_is_inclusive() {
        let boundary = 1_000;
        assert!(is_before(boundary - 1, boundary));
        assert!(!is_reached(boundary - 1, boundary));
        assert!(!is_before(boundary, boundary));
        assert!(is_reached(boundary, boundary));
        assert!(!is_before(boundary + 1, boundary));
        assert!(is_reached(boundary + 1, boundary));
    }

    #[test]
    fn duration_boundary_is_inclusive() {
        let start = 900;
        let duration = 100;
        assert!(!duration_elapsed(999, start, duration));
        assert!(duration_elapsed(1_000, start, duration));
        assert!(duration_elapsed(1_001, start, duration));
    }
}
