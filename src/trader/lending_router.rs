//! `LendingRouter` -- best-venue selection across lending protocols.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `pricegraph.rs`/`derivative_router.rs`/`credit.rs` -- callers pull
//! real [`CreditReserve`]s from live `DexState` (via each protocol's own
//! `reserve_by_mint`, wrapped through `CreditReserve::from_*`) and hand
//! the resolved candidates to this module; it does no host-import work
//! itself. Not a graph-search engine, for the same reason as
//! [`crate::trader::perp_router::PerpRouter`]: lending positions don't
//! chain/compose across protocols the way multi-hop spot swaps do. What
//! *does* generalize is the other thing both `TradeRouter` and
//! `PerpRouter` do -- picking the best among multiple venues for the
//! same instrument -- which is all this module is.
//!
//! This centralizes a comparison that used to be hand-duplicated, with
//! only Solend/Kamino ever wired in, across six different brain modules
//! (`testperpv1`, `perpfundingv1`, `multimodelv1`, `testlatencylitev1`,
//! `testperplatencyv1`, `testperplatencyv1lite` -- each had its own copy
//! of `best_borrow_apy`/`best_supply_apy`). Building on `CreditReserve`
//! (already unified across Solend/Kamino/Marginfi/Drift/Jet for
//! `derivative_router.rs`'s basis-trade search) means this comparison
//! now covers every protocol `CreditReserve` covers, not just the two a
//! given brain module happened to wire in by hand.

use crate::trader::credit::CreditReserve;

/// A lending protocol a [`CreditReserve`] can come from -- mirrors
/// `CreditReserve::from_*`'s constructors exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LendingProtocol {
    Solend,
    Kamino,
    Marginfi,
    /// See [`CreditReserve::from_drift`]'s doc comment -- `price_usd` is
    /// an oracle snapshot, not a live read, and `supply_apy` is always
    /// unpriced (`0.0`).
    Drift,
    /// See [`CreditReserve::from_jet`]'s doc comment -- `borrow_apy`/
    /// `supply_apy`/`available_liquidity_usd` are always unpriced
    /// (`0.0`), so Jet never wins a comparison against a priced
    /// competitor; it can still be returned if it's the only candidate.
    Jet,
}

/// Stateless (for now) -- a real struct rather than free functions, so
/// this can grow retained state later (e.g. rate history, same shape
/// [`crate::trader::perp_router::PerpRouter`] needed for funding rates)
/// without changing the call-site API. Mirrors
/// `pricegraph::TradeRouter`/`perp_router::PerpRouter`'s shape.
#[derive(Debug, Default, Clone, Copy)]
pub struct LendingRouter;

impl LendingRouter {
    pub fn new() -> Self {
        Self
    }

    /// Cheapest real borrow APY among `candidates` (fraction per year,
    /// e.g. `0.042` for 4.2%). `None` if `candidates` is empty.
    ///
    /// **Caveat inherited from `CreditReserve`**: a `0.0` `borrow_apy`
    /// (Drift/Jet's unpriced convention, see their own `from_*` doc
    /// comments) always wins this comparison, which is wrong if it's
    /// actually unpriced rather than genuinely free. Callers mixing in
    /// Drift/Jet candidates alongside a fully-priced protocol should
    /// filter unpriced entries first (e.g. `available_liquidity_usd >
    /// 0.0`) -- callers sticking to Solend/Kamino/Marginfi don't need to,
    /// since all three are always fully priced when tracked at all.
    pub fn best_borrow_apy(
        &self,
        candidates: &[(LendingProtocol, CreditReserve)],
    ) -> Option<(LendingProtocol, CreditReserve)> {
        candidates.iter().copied().min_by(|a, b| a.1.borrow_apy.total_cmp(&b.1.borrow_apy))
    }

    /// Highest real supply APY among `candidates` (fraction per year).
    /// `None` if `candidates` is empty. Same unpriced caveat as
    /// [`Self::best_borrow_apy`], in the opposite direction: a `0.0`
    /// entry always *loses* here instead of always winning.
    pub fn best_supply_apy(
        &self,
        candidates: &[(LendingProtocol, CreditReserve)],
    ) -> Option<(LendingProtocol, CreditReserve)> {
        candidates.iter().copied().max_by(|a, b| a.1.supply_apy.total_cmp(&b.1.supply_apy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve(borrow_apy: f64, supply_apy: f64) -> CreditReserve {
        CreditReserve {
            reserve_id: 1,
            mint: 100,
            max_ltv_pct: 0.7,
            borrow_apy,
            supply_apy,
            available_liquidity_usd: 1_000_000.0,
        }
    }

    #[test]
    fn best_borrow_apy_picks_the_cheapest() {
        let router = LendingRouter::new();
        let candidates = [
            (LendingProtocol::Solend, reserve(0.05, 0.0)),
            (LendingProtocol::Kamino, reserve(0.03, 0.0)),
            (LendingProtocol::Marginfi, reserve(0.04, 0.0)),
        ];
        let (protocol, r) = router.best_borrow_apy(&candidates).unwrap();
        assert_eq!(protocol, LendingProtocol::Kamino);
        assert_eq!(r.borrow_apy, 0.03);
    }

    #[test]
    fn best_supply_apy_picks_the_highest() {
        let router = LendingRouter::new();
        let candidates = [
            (LendingProtocol::Solend, reserve(0.0, 0.02)),
            (LendingProtocol::Kamino, reserve(0.0, 0.045)),
            (LendingProtocol::Marginfi, reserve(0.0, 0.031)),
        ];
        let (protocol, r) = router.best_supply_apy(&candidates).unwrap();
        assert_eq!(protocol, LendingProtocol::Kamino);
        assert_eq!(r.supply_apy, 0.045);
    }

    #[test]
    fn empty_candidates_returns_none() {
        let router = LendingRouter::new();
        assert!(router.best_borrow_apy(&[]).is_none());
        assert!(router.best_supply_apy(&[]).is_none());
    }

    #[test]
    fn single_candidate_wins_by_default() {
        let router = LendingRouter::new();
        let candidates = [(LendingProtocol::Jet, reserve(0.0, 0.0))];
        let (protocol, _) = router.best_borrow_apy(&candidates).unwrap();
        assert_eq!(protocol, LendingProtocol::Jet);
    }
}
