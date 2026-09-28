//! Funding-rate arbitrage search -- the Phoenix-perp-funding-vs-lending-
//! rate basis trade, generalized into a reusable, ranked, multi-symbol
//! search instead of one bot-mode's own private per-symbol decision.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `pricegraph.rs`/`spfa.rs`/`credit.rs`/`timegraph.rs` -- callers are
//! responsible for pulling real rates (a Phoenix funding rate via
//! `perp_router::PerpRouter::pending_rate`, real lending reserves via
//! `credit::CreditReserve::from_kamino`/`from_solend`/`from_marginfi`,
//! all of which need the live WASM runtime) and handing the resolved
//! numbers to this module -- it does no host-import work itself and
//! doesn't even depend on `perp_router`'s own types, only on plain `f64`
//! rates a caller already resolved.
//!
//! This is a standalone module: it deliberately duplicates, rather than
//! calls into, `brain::perpfundingv1::state`'s existing private
//! `decide_basis_trade`/`log_basis_cycles` -- see this module's own doc
//! comment history (2026-08-29) for why: `perpfundingv1` is presumed
//! live, real-money code, and the user explicitly asked for this module
//! to be purely additive, not a refactor of it. The economics/thresholds
//! are copied verbatim (including the original's test cases below) so
//! behavior is provably identical, not just similarly-shaped -- see
//! [`decide_basis_trade`]'s doc comment for the real economics.
//!
//! Reuses, rather than reimplements, the lending-side data model:
//! [`crate::trader::credit::CreditReserve`] (built for exactly this,
//! its own doc comment already invited composing it with a search like
//! this one, but nothing had wired it in yet).

use crate::trader::credit::CreditReserve;

/// Which side of the basis trade is profitable for a symbol right now.
/// See [`decide_basis_trade`]'s doc comment for the real economics of
/// each. Identical shape to `brain::perpfundingv1::state`'s private
/// `BasisDirection` -- not the same type (this module doesn't depend on
/// that one, see the module doc comment), but the same meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisDirection {
    /// Funding positive (longs pay shorts on Phoenix): short the perp,
    /// hedge with a lending-protocol deposit of the underlying (no
    /// borrowing).
    DepositHedge,
    /// Funding negative (shorts pay longs): long the perp, hedge with a
    /// lending-protocol borrow of the underlying, sold for USDC
    /// (synthetic short).
    BorrowHedge,
}

/// Pure (no host-import dependency, natively testable): decides which
/// side of the Phoenix-perp-funding-vs-lending-rate basis trade is
/// profitable for a symbol, given this epoch's Phoenix funding rate and
/// the cheapest real borrow APY available across every lending protocol
/// with a reserve for this symbol (both **percent** units --
/// `CreditReserve::borrow_apy` is a 0.0-1.0 fraction, multiply by 100
/// before calling this -- [`find_best_funding_opportunities`] does this
/// for you).
///
/// - `phoenix_funding_pct > 0.0` (longs pay shorts): short the perp,
///   deposit the underlying on whichever protocol pays the best supply
///   APY to stay delta-neutral -- this *adds* the deposit's yield on top
///   of the captured funding, so it's worth doing whenever funding is
///   positive at all, no threshold against any lending rate needed
///   (depositing never costs anything, only ever earns).
/// - `phoenix_funding_pct < 0.0` (shorts pay longs): long the perp,
///   borrow the underlying and sell it for a synthetic short hedge --
///   only profitable if the funding collected exceeds the real interest
///   paid to borrow, i.e. `-phoenix_funding_pct > borrow_apy_pct`.
/// - Otherwise (funding is exactly zero, or negative but not enough to
///   clear the borrow cost): `None`, no capturable edge.
pub fn decide_basis_trade(phoenix_funding_pct: f64, borrow_apy_pct: f64) -> Option<BasisDirection> {
    if phoenix_funding_pct > 0.0 {
        Some(BasisDirection::DepositHedge)
    } else if phoenix_funding_pct < 0.0 && -phoenix_funding_pct > borrow_apy_pct {
        Some(BasisDirection::BorrowHedge)
    } else {
        None
    }
}

/// One symbol's real, resolved inputs for [`find_best_funding_opportunities`]
/// -- the caller's job to build (see the module doc comment): a real
/// Phoenix funding rate (e.g. via `PerpRouter::pending_rate`) and every
/// real lending reserve tracked for this symbol's underlying mint across
/// whichever protocols the caller has (Kamino/Solend/marginfi/...).
pub struct SymbolFundingInput<'a> {
    pub symbol: &'static str,
    /// Phoenix's current annualized funding rate for this symbol,
    /// percent units (e.g. `5.0` for +5%/yr) -- already resolved by the
    /// caller, not re-derived here.
    pub phoenix_funding_pct: f64,
    /// Every real lending reserve for this symbol's mint the caller
    /// tracks. Empty means "no lending data yet" -- the symbol is
    /// skipped, not treated as zero-cost.
    pub reserves: &'a [CreditReserve],
}

/// One symbol's real opportunity, if any -- the data
/// `brain::perpfundingv1::state`'s private `log_basis_cycles` computes
/// inline every epoch but never returns/exposes outside that one
/// bot-mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FundingOpportunity {
    pub symbol: &'static str,
    pub direction: BasisDirection,
    pub phoenix_funding_pct: f64,
    /// The cheapest real borrow APY found across the symbol's reserves,
    /// percent units -- the one [`decide_basis_trade`] actually used.
    pub borrow_apy_pct: f64,
    /// Net edge this opportunity clears by, for ranking -- generalizes
    /// `decide_basis_trade`'s boolean into a comparable number: the
    /// funding captured for `DepositHedge` (depositing never costs
    /// anything, per `decide_basis_trade`'s own doc comment), or funding
    /// captured minus interest paid for `BorrowHedge`.
    pub net_edge_pct: f64,
}

/// The real, reusable "search" -- generalizes `log_basis_cycles`'
/// private open-pass loop (`brain::perpfundingv1::state`) into a pure
/// query: given real funding data and real lending reserves for a set
/// of symbols, return every symbol with a real opportunity right now,
/// ranked by `net_edge_pct` descending (best first). Does not open or
/// close anything -- callers (a bot-mode's own `state.rs`) decide what,
/// if anything, to do with the result, same separation
/// `timegraph::dag_best_lst_path` draws between "decide" and "act."
///
/// A symbol with no reserves, or whose best real rate doesn't clear
/// [`decide_basis_trade`]'s threshold, is simply absent from the
/// result -- not included as a `None`/zero entry.
pub fn find_best_funding_opportunities(symbols: &[SymbolFundingInput]) -> Vec<FundingOpportunity> {
    let mut out = Vec::new();
    for input in symbols {
        let Some(cheapest) = input
            .reserves
            .iter()
            .min_by(|a, b| a.borrow_apy.total_cmp(&b.borrow_apy))
        else {
            continue;
        };
        let borrow_apy_pct = cheapest.borrow_apy * 100.0;
        let Some(direction) = decide_basis_trade(input.phoenix_funding_pct, borrow_apy_pct) else {
            continue;
        };
        let net_edge_pct = match direction {
            BasisDirection::DepositHedge => input.phoenix_funding_pct,
            BasisDirection::BorrowHedge => -input.phoenix_funding_pct - borrow_apy_pct,
        };
        out.push(FundingOpportunity {
            symbol: input.symbol,
            direction,
            phoenix_funding_pct: input.phoenix_funding_pct,
            borrow_apy_pct,
            net_edge_pct,
        });
    }
    out.sort_by(|a, b| b.net_edge_pct.total_cmp(&a.net_edge_pct));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported verbatim from brain::perpfundingv1::state's original
    // decide_basis_trade tests -- same inputs, same expected outputs,
    // proof this copy's economics are identical, not just similarly
    // shaped.

    #[test]
    fn decide_basis_trade_deposit_hedge_on_positive_funding() {
        assert_eq!(decide_basis_trade(5.0, 20.0), Some(BasisDirection::DepositHedge));
        assert_eq!(decide_basis_trade(0.01, 0.0), Some(BasisDirection::DepositHedge));
    }

    #[test]
    fn decide_basis_trade_borrow_hedge_only_when_funding_exceeds_borrow_cost() {
        assert_eq!(decide_basis_trade(-20.0, 5.0), Some(BasisDirection::BorrowHedge));
        assert_eq!(decide_basis_trade(-3.0, 5.0), None);
    }

    #[test]
    fn decide_basis_trade_none_for_zero_funding() {
        assert_eq!(decide_basis_trade(0.0, 5.0), None);
    }

    #[test]
    fn decide_basis_trade_none_at_exact_borrow_cost_boundary() {
        assert_eq!(decide_basis_trade(-5.0, 5.0), None);
    }

    fn reserve(borrow_apy: f64) -> CreditReserve {
        CreditReserve {
            reserve_id: 1,
            mint: 100,
            max_ltv_pct: 0.7,
            borrow_apy,
            available_liquidity_usd: 1_000_000.0,
        }
    }

    #[test]
    fn find_best_funding_opportunities_picks_cheapest_reserve_per_symbol() {
        // Two lending protocols for the same symbol -- Kamino at 10%,
        // Solend cheaper at 4% -- BorrowHedge should use the cheaper one.
        let reserves = [reserve(0.10), reserve(0.04)];
        let inputs = [SymbolFundingInput {
            symbol: "SOL",
            phoenix_funding_pct: -20.0,
            reserves: &reserves,
        }];
        let out = find_best_funding_opportunities(&inputs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbol, "SOL");
        assert_eq!(out[0].direction, BasisDirection::BorrowHedge);
        assert!((out[0].borrow_apy_pct - 4.0).abs() < 1e-9);
        assert!((out[0].net_edge_pct - 16.0).abs() < 1e-9); // 20 - 4
    }

    #[test]
    fn find_best_funding_opportunities_skips_symbols_with_no_edge_or_no_reserves() {
        let reserves = [reserve(0.05)];
        let no_reserves: [CreditReserve; 0] = [];
        let inputs = [
            // Negative funding, doesn't clear the borrow cost -- no edge.
            SymbolFundingInput {
                symbol: "BTC",
                phoenix_funding_pct: -3.0,
                reserves: &reserves,
            },
            // No lending data at all yet -- skipped, not zero-cost.
            SymbolFundingInput {
                symbol: "ETH",
                phoenix_funding_pct: 5.0,
                reserves: &no_reserves,
            },
        ];
        assert!(find_best_funding_opportunities(&inputs).is_empty());
    }

    #[test]
    fn find_best_funding_opportunities_ranks_by_net_edge_descending() {
        let reserves = [reserve(0.05)];
        let inputs = [
            SymbolFundingInput {
                symbol: "SOL",
                phoenix_funding_pct: 3.0, // net edge 3.0
                reserves: &reserves,
            },
            SymbolFundingInput {
                symbol: "BTC",
                phoenix_funding_pct: -15.0, // net edge 15.0 - 5.0 = 10.0
                reserves: &reserves,
            },
            SymbolFundingInput {
                symbol: "ETH",
                phoenix_funding_pct: 9.0, // net edge 9.0
                reserves: &reserves,
            },
        ];
        let out = find_best_funding_opportunities(&inputs);
        let symbols: Vec<&str> = out.iter().map(|o| o.symbol).collect();
        assert_eq!(symbols, vec!["BTC", "ETH", "SOL"]);
    }
}
