//! The strategy for `arbv1` — this is the ONE file to edit to change what
//! this bot actually trades. Everything else in `arbv1` is event plumbing
//! (validator event handling, wallet bookkeeping, router glue, WASM host
//! imports) that a strategy change should never need to touch, and that
//! isn't safe for a coding agent to touch: `configuration.rs`'s own
//! `Configuration` struct in particular is a `#[repr(C, align(8))]` wire
//! format shared byte-for-byte with the Go-side brain
//! (`optimizer/brain/arbv1`, a *different* repository) via
//! `MessageAction::AdjustConfiguration`'s raw `copy_nonoverlapping` — adding
//! a field there without a matching Go-side change would silently corrupt
//! that message, not just fail to compile.
//!
//! `accept` (below) is `state.rs`'s `detect_and_log_opportunity`'s own last
//! decision point: it runs after `crate::trader::planner::find_opportunity`
//! has already found a mathematically profitable cycle (Bellman-Ford
//! negative-cycle detection over the live price graph — the real
//! structural gate, unaffected by anything in this file) and, for Orca CLMM
//! hops, after that route has survived exact-quote re-verification. This
//! file's own constants are a *second*, independent, arbv1-owned filter on
//! top of that — `crate::trader::planner`'s own `MIN_PROFIT_BPS`/
//! `REAL_SIZE_MAX_HOPS` constants are the shared baseline every brain mode
//! gets; the ones below only ever apply to `arbv1`, and are meant to be
//! edited freely.
//!
//! Defaults below (`None`) are a strict no-op (`accept` always returns
//! `true`) — this file changes no trading behavior on its own; it only
//! exists to give a coding agent one small, self-contained place to add
//! real filtering logic when asked to implement a specific strategy,
//! without needing to understand or touch the router/wallet/event-handling
//! code around it.
//!
//! `Option`, not a plain `0`/`usize::MAX` sentinel: a bare `>= 0` (u64's own
//! minimum) or `<= usize::MAX` comparison is always trivially true, and
//! this project denies `clippy::absurd_extreme_comparisons` (a real,
//! live-confirmed lint failure caught writing this file the first way) —
//! `None` says "no filter" without ever comparing against a type's own
//! extreme value at all.

use crate::trader::planner::ArbitrageOpportunity;

/// Minimum profit, in basis points of `amount_in`, for an opportunity to
/// be worth executing — in *addition* to `trader::planner::MIN_PROFIT_BPS`
/// (the shared structural floor every brain mode already gets), not a
/// replacement for it. `None` means no additional floor. Set `Some(n)` to
/// require a bigger margin than the shared baseline before `arbv1`
/// specifically will execute — e.g. to leave more room for priority-fee
/// cost or estimation error than the shared default assumes.
pub(crate) const MIN_PROFIT_BPS: Option<u64> = None;

/// Maximum number of hops (DEX swaps) a cycle may take — in *addition* to
/// `trader::planner::REAL_SIZE_MAX_HOPS` (the shared search-depth cap
/// already applied before a cycle ever reaches this function). `None`
/// means no additional cap. Set `Some(n)` to reject longer, more
/// slippage/failure-exposed routes even when the shared search still
/// considers them.
pub(crate) const MAX_HOPS: Option<usize> = None;

/// Decides whether a router-found, exact-quote-verified arbitrage
/// opportunity should actually be executed. Called once per opportunity,
/// from `StateHelper::detect_and_log_opportunity` (`state.rs`) — returning
/// `false` rejects it the same way a failed exact-quote re-verification
/// already does (logged, discarded, nothing sent).
///
/// Add more filters here as needed for a real strategy — e.g. a minimum
/// `opp.wallet_balance`, a specific `opp.cycle.start_token()` allow-list,
/// or gating on anything else `ArbitrageOpportunity`/`ArbitrageCycle`
/// (`crate::trader::pricegraph`) already exposes. Keep it a pure function
/// of `opp` if at all possible — that's what keeps this file easy to
/// reason about and easy to unit test in isolation from the rest of
/// `arbv1`.
pub(crate) fn accept(opp: &ArbitrageOpportunity) -> bool {
    if let Some(min_profit_bps) = MIN_PROFIT_BPS {
        if opp.cycle.profit_bps() < min_profit_bps {
            return false;
        }
    }
    if let Some(max_hops) = MAX_HOPS {
        if opp.cycle.hops.len() > max_hops {
            return false;
        }
    }
    true
}
