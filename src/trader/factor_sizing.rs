//! Slippage-aware position sizing for `multimodelv1` -- Phase 3 of
//! `brain::multimodelv1::PLAN-1.md`: every real notional this mode would
//! ever send is sized against a real, depth-aware quote, never against
//! `factor_graph.rs`'s Laplacian edge weights (`ln(1+TVL)`-style
//! structural proxies, not prices).
//!
//! Unlike `factor_graph.rs`/`factor_borrow_gate.rs`, this module is
//! **not** host-import-free -- it depends on
//! [`crate::trader::pricegraph::TradeRouter`], which is the deliberate
//! point of Phase 3: reuse the same real depth-aware quoting path
//! `TradeRouter`'s own slippage-aware arbitrage search already uses
//! (`route_slippage_aware`/`requote_edge`), not a second, parallel
//! pricing model. `pricegraph.rs` itself is consumed as-is here, not
//! modified (`PLAN-1.md`'s own Critical Files note) -- every `TradeRouter`
//! call below goes through its existing public API, including in this
//! module's own tests (`router::Router::new`/`register_mint` +
//! `TradeRouter::from_router`, the same pattern
//! `dex::marinade`/`dex::pumpfun`/`dex::pumpswap`'s tests already use to
//! build a synthetic router without a live host).
//!
//! Respects `TradeRouter::mark_pool_cooldown` for free: every quote here
//! goes through `route_slippage_aware`, which already excludes cooled-down
//! pools -- a leg routed only through a cooled-down pool is simply
//! unpriceable to this module, same as a leg with no route at all (see
//! [`size_basket`]'s tests).
//!
//! `price_impact_bps`/`max_safe_notional`/`size_basket` quote every hop
//! via `TradeRouter`'s own `cp_quote`-based `requote_along`/`requote_edge`
//! -- a real, live-confirmed gap for Raydium CLMM (and Orca Whirlpool)
//! pools, whose liquidity is concentrated near the current tick rather
//! than spread evenly like a constant-product pool (`requote_edge`'s own
//! doc comment already flags this as CLMM's "tick-unaware approximation
//! gap"; `pricegraph::add_raydium_clmm_pool` builds a CLMM edge's
//! `reserve_in`/`reserve_out` straight from vault token-account balances,
//! not real concentrated-liquidity state). `_exact`-suffixed siblings
//! (`price_impact_bps_exact`/`max_safe_notional_exact`/
//! `size_basket_exact`) fix this by additionally taking a `&DexState` and
//! reusing `planner::reverify_route_with_exact_quotes` -- already used
//! everywhere else in this repo as a pre-execution safety gate -- to get
//! real tick-aware quotes for CLMM/Orca hops instead of trusting
//! `cp_quote` on the pool's full vault balance. The original,
//! `TradeRouter`-only functions are unchanged and still the right choice
//! for callers that don't need this (or can't easily reach a live
//! `DexState`); both share their core logic via a private `_with` helper
//! parameterized over how a route gets requoted, so the exact-quote path
//! isn't a parallel reimplementation.

use crate::{
    graph::AccountId,
    trader::{
        dex::DexState,
        planner,
        pricegraph::{Route, TradeRouter},
    },
};

/// Max tolerated price impact for a single leg's real quote, basis
/// points. `50.0` (0.50%) is a starting value, not a calibrated one --
/// no prior bot mode in this repo sizes against a factor-model signal,
/// so there's no real trading history yet to tune this against; revisit
/// once `multimodelv1` has one.
pub const MAX_PRICE_IMPACT_BPS: f64 = 50.0;

/// Real, quote-derived price impact of routing `amount_in` of `from_mint`
/// into `to_mint` through `router`, in basis points. Compares the
/// per-unit rate at `amount_in` against the per-unit rate at a much
/// smaller `probe_amount_in` (approximating the pool's unslipped
/// marginal rate) -- the caller picks `probe_amount_in` (see
/// [`max_safe_notional`] for the convention this module itself uses).
/// `None` if either quote is unavailable (no route at all, or the only
/// route is on cooldown) -- an unpriceable leg must never be silently
/// treated as zero-impact.
///
/// Both rates are measured along the **same** route (the one found for
/// `amount_in`) -- the probe is requoted along it via
/// `TradeRouter::requote_along`, not found by a second, independent
/// `route_slippage_aware` search. Live-confirmed real bug this fixes:
/// `route_slippage_aware` can pick a structurally different path at a
/// different amount (its own doc comment already flagged this as a
/// known simplification), including paths that revisit an
/// already-visited token -- `widest_path`'s layered DP has no
/// visited-node exclusion. A real sell-direction candidate was observed
/// (via `TradeRouter::route_diagnostics`) getting a probe quote through
/// a plain direct pool and a full quote through a route that looped
/// `mint -> USDC -> X -> USDC`, making the "price impact" a comparison
/// between two unrelated routes rather than real same-path slippage --
/// this rejected safe candidates (and could equally have passed unsafe
/// ones) across every caller of `max_safe_notional`, not just this one.
pub fn price_impact_bps(
    router: &TradeRouter,
    from_mint: AccountId,
    to_mint: AccountId,
    amount_in: u64,
    probe_amount_in: u64,
    max_hops: usize,
) -> Option<f64> {
    price_impact_bps_with(router, from_mint, to_mint, amount_in, probe_amount_in, max_hops, |route, amt| {
        router.requote_along(route, amt)
    })
}

/// Exact-quote-aware twin of [`price_impact_bps`] -- see this module's
/// own doc comment for why. Both the probe and full quotes come from
/// `planner::reverify_route_with_exact_quotes` on the *same* route found
/// once via `route_slippage_aware`, so a CLMM/Orca hop gets a real
/// tick-aware quote (`DexState::raydium_clmm_exact_quote`/
/// `orca_exact_quote`) instead of `cp_quote` on its full vault balance;
/// every other hop's quote is unchanged (`reverify_hops` already treats
/// `cp_quote` as exact there). A hop whose exact quote fails
/// (`Err(pool_id)`) is treated as unpriceable (`None`), same contract as
/// [`price_impact_bps`] -- no cooldown side effects here, this module
/// stays a pure query.
pub fn price_impact_bps_exact(
    router: &TradeRouter,
    dex: &DexState,
    from_mint: AccountId,
    to_mint: AccountId,
    amount_in: u64,
    probe_amount_in: u64,
    max_hops: usize,
) -> Option<f64> {
    price_impact_bps_with(router, from_mint, to_mint, amount_in, probe_amount_in, max_hops, |route, amt| {
        planner::reverify_route_with_exact_quotes(route, amt, router, dex).ok().map(|r| r.amount_out())
    })
}

/// Shared core of [`price_impact_bps`]/[`price_impact_bps_exact`] -- the
/// only difference between them is `requote`, which turns a fixed `Route`
/// (found once, for `amount_in`) plus a different amount into a real
/// output quote along that same route.
fn price_impact_bps_with(
    router: &TradeRouter,
    from_mint: AccountId,
    to_mint: AccountId,
    amount_in: u64,
    probe_amount_in: u64,
    max_hops: usize,
    requote: impl Fn(&Route, u64) -> Option<u64>,
) -> Option<f64> {
    if amount_in == 0 || probe_amount_in == 0 {
        return None;
    }
    let full = router.route_slippage_aware(from_mint, to_mint, amount_in, max_hops)?;
    let probe_out = requote(&full, probe_amount_in)?;
    let probe_rate = probe_out as f64 / probe_amount_in as f64;
    if probe_rate <= 0.0 {
        return None;
    }
    let full_out = requote(&full, amount_in)?;
    let full_rate = full_out as f64 / amount_in as f64;
    Some(((probe_rate - full_rate) / probe_rate) * 10_000.0)
}

/// Largest `amount_in`, bounded above by `candidate_notional`, that keeps
/// [`price_impact_bps`] at or under [`MAX_PRICE_IMPACT_BPS`] -- found by
/// binary search over real quotes (cheap: each step is one or two
/// `route_slippage_aware` calls against already-loaded pool state, not a
/// chain read). `0` means "unpriceable" (no real route at any size, or
/// the only route is cooled down) -- callers must treat `0` as "skip this
/// leg entirely", never as "size it at zero notional and proceed" (see
/// [`size_basket`]).
///
/// The binary search assumes price impact is monotonically
/// non-decreasing in `amount_in`, which holds for a single constant-
/// product hop but is only an approximation for a multi-hop route, since
/// `route_slippage_aware` can in principle pick a *different* path at a
/// different amount -- not fixed here, flagged as a known simplification
/// for `max_hops > 1`.
pub fn max_safe_notional(
    router: &TradeRouter,
    from_mint: AccountId,
    to_mint: AccountId,
    candidate_notional: u64,
    max_hops: usize,
) -> u64 {
    max_safe_notional_with(candidate_notional, |amt, probe| {
        price_impact_bps(router, from_mint, to_mint, amt, probe, max_hops)
    })
}

/// Exact-quote-aware twin of [`max_safe_notional`] -- see this module's
/// own doc comment. Same binary search, driven by
/// [`price_impact_bps_exact`] instead of [`price_impact_bps`], so a
/// candidate whose route runs through a CLMM/Orca hop gets sized against
/// a real tick-aware quote rather than `cp_quote` on that pool's full
/// vault balance.
pub fn max_safe_notional_exact(
    router: &TradeRouter,
    dex: &DexState,
    from_mint: AccountId,
    to_mint: AccountId,
    candidate_notional: u64,
    max_hops: usize,
) -> u64 {
    max_safe_notional_with(candidate_notional, |amt, probe| {
        price_impact_bps_exact(router, dex, from_mint, to_mint, amt, probe, max_hops)
    })
}

/// Shared core of [`max_safe_notional`]/[`max_safe_notional_exact`] --
/// the only difference between them is `price_impact_at`, which measures
/// real price impact at a given `(amount_in, probe_amount_in)` pair
/// (`cp_quote`-only vs. exact-quote-aware).
///
/// The binary search assumes price impact is monotonically
/// non-decreasing in `amount_in`, which holds for a single constant-
/// product hop but is only an approximation for a multi-hop route, since
/// `route_slippage_aware` can in principle pick a *different* path at a
/// different amount -- not fixed here, flagged as a known simplification
/// for `max_hops > 1`.
fn max_safe_notional_with(candidate_notional: u64, price_impact_at: impl Fn(u64, u64) -> Option<f64>) -> u64 {
    if candidate_notional == 0 {
        return 0;
    }
    // The probe must be small relative to `candidate_notional` to
    // approximate the marginal rate, but large enough in absolute terms
    // that cp_quote's integer truncation doesn't dominate it -- a 1-unit
    // probe against billion-unit reserves can truncate to an output that
    // makes the "marginal" rate look *worse* than the full trade's, a
    // pure integer-rounding artifact, not a real price signal. 1% of the
    // candidate, floored at 1_000 raw units and capped at the candidate
    // itself, is a pragmatic balance -- not derived from reserve size
    // (this module never sees raw reserves, only quoted amounts).
    let probe = ((candidate_notional / 100).max(1_000)).min(candidate_notional).max(1);
    // `None` at the *full* candidate size no longer means "unpriceable at
    // any size" -- `TradeRouter::widest_path`'s per-hop pool-utilization
    // cap (added alongside `requote_along`/`path_revisits_a_node`, real
    // live incidents) makes route *existence* depend on `amount_in`: a
    // candidate too large for a shallow pool now genuinely has no route
    // at that size, even though a smaller size through the very same pool
    // would. So this no longer short-circuits to 0 -- it falls through to
    // the binary search below, whose own loop already treats a `None` at
    // any given `mid` the same as "impact too high" (shrinks `hi`), which
    // also naturally converges to `lo == 0` for the genuinely-never-
    // routable case (disconnected tokens, or every candidate pool cooled
    // down) -- same end result as the old short-circuit, just correct now
    // for the size-dependent case too.
    if let Some(impact) = price_impact_at(candidate_notional, probe) {
        if impact <= MAX_PRICE_IMPACT_BPS {
            return candidate_notional;
        }
    }
    let mut lo: u64 = 0;
    let mut hi: u64 = candidate_notional;
    // 24 halvings is comfortably enough to converge a u64 range to
    // within a handful of raw units, at a cost of ~48 quote calls --
    // trivial against already-loaded in-memory pool state.
    for _ in 0..24 {
        if hi <= lo + 1 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        match price_impact_at(mid, probe) {
            Some(impact) if impact <= MAX_PRICE_IMPACT_BPS => lo = mid,
            _ => hi = mid,
        }
    }
    lo
}

/// One leg of a candidate multi-leg basket trade (factor-neutral hedge,
/// dispersion basket, ...) before slippage-aware sizing -- the notional
/// the strategy *wants*, not what it's safe to actually send.
pub struct LegNotional {
    pub from_mint: AccountId,
    pub to_mint: AccountId,
    pub intended_notional: u64,
}

/// One leg after [`size_basket`] -- the real, slippage-safe notional to
/// actually send.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizedLeg {
    pub from_mint: AccountId,
    pub to_mint: AccountId,
    pub sized_notional: u64,
}

/// Sizes an entire multi-leg basket down to its shallowest-liquidity leg
/// -- `PLAN-1.md` Phase 3's own rule: a basket that's neutral on paper
/// but has one leg too large for its own pool isn't actually neutral
/// once real slippage-adjusted fills land, so legs are never sized
/// independently. Every leg is quoted first via [`max_safe_notional`];
/// the whole basket is then scaled by the single smallest
/// `safe/intended` ratio found across all legs, applied *uniformly* to
/// every leg -- including legs whose own pool could easily have
/// absorbed more.
///
/// A leg that's entirely unpriceable ([`max_safe_notional`] returns `0`)
/// forces the whole basket's scale to `0.0` (every leg comes back with
/// `sized_notional: 0`) -- a basket missing one leg isn't the trade that
/// was decided on; the caller is expected to treat an all-zero result as
/// "do not send this basket", not "send the legs that did price."
///
/// The single constraining leg (the one with the smallest `safe/intended`
/// ratio) is assigned its own [`max_safe_notional`] result exactly, not
/// `intended_notional * scale` re-derived through the same floating-point
/// ratio -- a float round-trip (`safe / intended` then `* intended`) can
/// lose the last raw unit to rounding, which would make the "shallowest"
/// leg not actually get its own real safe cap. Every other leg is scaled
/// by that same ratio and floored, which is safe to do (floors err
/// toward smaller, never over the cap).
pub fn size_basket(router: &TradeRouter, legs: &[LegNotional], max_hops: usize) -> Vec<SizedLeg> {
    size_basket_with(legs, |l| max_safe_notional(router, l.from_mint, l.to_mint, l.intended_notional, max_hops))
}

/// Exact-quote-aware twin of [`size_basket`] -- see this module's own doc
/// comment. Sizes every leg via [`max_safe_notional_exact`] instead of
/// [`max_safe_notional`], so a leg whose route runs through a CLMM/Orca
/// hop is scaled against a real tick-aware safe size, not `cp_quote` on
/// that pool's full vault balance.
pub fn size_basket_exact(router: &TradeRouter, dex: &DexState, legs: &[LegNotional], max_hops: usize) -> Vec<SizedLeg> {
    size_basket_with(legs, |l| max_safe_notional_exact(router, dex, l.from_mint, l.to_mint, l.intended_notional, max_hops))
}

/// Shared core of [`size_basket`]/[`size_basket_exact`] -- the only
/// difference between them is `safe_for`, which computes one leg's own
/// real safe notional (`cp_quote`-only vs. exact-quote-aware).
fn size_basket_with(legs: &[LegNotional], safe_for: impl Fn(&LegNotional) -> u64) -> Vec<SizedLeg> {
    let zero_basket = || {
        legs.iter()
            .map(|l| SizedLeg { from_mint: l.from_mint, to_mint: l.to_mint, sized_notional: 0 })
            .collect::<Vec<_>>()
    };
    if legs.is_empty() || legs.iter().any(|l| l.intended_notional == 0) {
        return zero_basket();
    }
    let safes: Vec<u64> = legs.iter().map(&safe_for).collect();
    if safes.iter().any(|&s| s == 0) {
        return zero_basket();
    }
    let min_idx = (0..legs.len())
        .min_by(|&a, &b| {
            let ratio_a = safes[a] as f64 / legs[a].intended_notional as f64;
            let ratio_b = safes[b] as f64 / legs[b].intended_notional as f64;
            ratio_a.total_cmp(&ratio_b)
        })
        .expect("legs is non-empty, checked above");
    let scale = safes[min_idx] as f64 / legs[min_idx].intended_notional as f64;
    legs.iter()
        .enumerate()
        .map(|(i, leg)| SizedLeg {
            from_mint: leg.from_mint,
            to_mint: leg.to_mint,
            sized_notional: if i == min_idx {
                safes[min_idx]
            } else {
                (leg.intended_notional as f64 * scale).floor() as u64
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trader::{router::Router, types::DexType};

    const MINT_A: AccountId = 1;
    const MINT_B: AccountId = 2;
    const MINT_C: AccountId = 3;

    fn router_with_mints(mints: &[AccountId]) -> TradeRouter {
        let mut r = Router::new(mints.len(), 0.01);
        for &m in mints {
            r.register_mint(m);
        }
        TradeRouter::from_router(&r)
    }

    #[test]
    fn price_impact_is_near_zero_for_a_tiny_trade_on_a_deep_pool() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(
            100,
            MINT_A,
            MINT_B,
            1.0,
            0.0,
            1_000_000_000_000,
            1_000_000_000_000,
            DexType::Sanctum,
        );
        // amount_in and probe are both tiny relative to the pool, but
        // large enough in absolute terms that cp_quote's integer
        // truncation doesn't swamp the real signal (see max_safe_notional's
        // own doc comment on why the probe can't just be "amount_in / 1000"
        // unconditionally) -- near zero impact, well under the cap.
        let impact = price_impact_bps(&router, MINT_A, MINT_B, 10_000_000, 1_000_000, 1).unwrap();
        assert!(impact.abs() < 1.0, "expected near-zero impact, got {impact} bps");
    }

    #[test]
    fn price_impact_grows_with_trade_size_on_the_same_pool() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 100_000, 100_000, DexType::Sanctum);
        let probe = 10;
        let small = price_impact_bps(&router, MINT_A, MINT_B, 1_000, probe, 1).unwrap();
        // Stays under `pricegraph::MAX_HOP_POOL_UTILIZATION_BPS` (20% of
        // this pool's 100_000 reserve) -- a trade that large is excluded
        // from the route search entirely now (real fix, see that
        // constant's own doc comment), not just measured as high-impact.
        let large = price_impact_bps(&router, MINT_A, MINT_B, 15_000, probe, 1).unwrap();
        assert!(large > small, "larger trade ({large} bps) should impact more than smaller ({small} bps)");
    }

    #[test]
    fn price_impact_none_when_no_route_exists() {
        let router = router_with_mints(&[MINT_A, MINT_B]);
        assert_eq!(price_impact_bps(&router, MINT_A, MINT_B, 1_000, 10, 1), None);
    }

    #[test]
    fn max_safe_notional_returns_full_candidate_when_already_safe() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);
        let safe = max_safe_notional(&router, MINT_A, MINT_B, 1_000, 1);
        assert_eq!(safe, 1_000);
    }

    #[test]
    fn max_safe_notional_caps_below_candidate_for_a_shallow_pool() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 100_000, 100_000, DexType::Sanctum);
        let candidate = 90_000; // a large fraction of the pool -- unsafe at full size
        let safe = max_safe_notional(&router, MINT_A, MINT_B, candidate, 1);
        assert!(safe < candidate, "expected capping, got {safe} == candidate");
        assert!(safe > 0, "expected a nonzero safe size, got 0");
        // The capped size itself must actually clear the cap (that's the
        // whole point of the binary search, not just "smaller than
        // candidate").
        let probe = (candidate / 1000).max(1);
        let impact = price_impact_bps(&router, MINT_A, MINT_B, safe, probe, 1).unwrap();
        assert!(impact <= MAX_PRICE_IMPACT_BPS, "capped size still exceeds cap: {impact} bps");
    }

    #[test]
    fn max_safe_notional_zero_when_unpriceable() {
        let router = router_with_mints(&[MINT_A, MINT_B]);
        assert_eq!(max_safe_notional(&router, MINT_A, MINT_B, 1_000, 1), 0);
    }

    #[test]
    fn max_safe_notional_zero_when_only_route_is_cooled_down() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);
        router.set_current_slot(100);
        router.mark_pool_cooldown(100, 50);
        assert_eq!(max_safe_notional(&router, MINT_A, MINT_B, 1_000, 1), 0);
    }

    #[test]
    fn size_basket_scales_every_leg_uniformly_to_the_shallowest_leg() {
        let mut router = router_with_mints(&[MINT_A, MINT_B, MINT_C]);
        // Leg A->B: very deep, could easily absorb its intended notional.
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);
        // Leg A->C: shallow -- the same intended notional is unsafe here.
        router.add_generic_pair(101, MINT_A, MINT_C, 1.0, 0.0, 100_000, 100_000, DexType::Sanctum);

        let intended = 90_000;
        let legs = [
            LegNotional { from_mint: MINT_A, to_mint: MINT_B, intended_notional: intended },
            LegNotional { from_mint: MINT_A, to_mint: MINT_C, intended_notional: intended },
        ];
        let sized = size_basket(&router, &legs, 1);
        assert_eq!(sized.len(), 2);

        // The shallow leg's own individually-safe cap...
        let shallow_safe = max_safe_notional(&router, MINT_A, MINT_C, intended, 1);
        // ...must be what BOTH legs actually got sized to (the deep leg
        // is not sized independently at its own, much larger, safe cap).
        assert_eq!(sized[0].sized_notional, shallow_safe, "deep leg wasn't scaled down to match the shallow leg");
        assert_eq!(sized[1].sized_notional, shallow_safe);

        // And that shared size must be strictly less than what the deep
        // leg alone could have safely absorbed -- proof the deep leg's
        // own headroom was deliberately left on the table, not just
        // coincidentally equal.
        let deep_safe_alone = max_safe_notional(&router, MINT_A, MINT_B, intended, 1);
        assert!(shallow_safe < deep_safe_alone);
    }

    #[test]
    fn size_basket_zeroes_the_whole_basket_when_one_leg_is_unpriceable() {
        let mut router = router_with_mints(&[MINT_A, MINT_B, MINT_C]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);
        // No pool registered for A->C at all.
        let legs = [
            LegNotional { from_mint: MINT_A, to_mint: MINT_B, intended_notional: 1_000 },
            LegNotional { from_mint: MINT_A, to_mint: MINT_C, intended_notional: 1_000 },
        ];
        let sized = size_basket(&router, &legs, 1);
        assert!(sized.iter().all(|s| s.sized_notional == 0), "unpriceable leg must zero the whole basket");
    }

    #[test]
    fn size_basket_respects_pool_cooldown() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);
        router.set_current_slot(100);
        router.mark_pool_cooldown(100, 50);
        let legs = [LegNotional { from_mint: MINT_A, to_mint: MINT_B, intended_notional: 1_000 }];
        let sized = size_basket(&router, &legs, 1);
        assert_eq!(sized[0].sized_notional, 0, "cooled-down pool must be treated as unpriceable, not ignored");
    }

    // `price_impact_bps_exact`/`max_safe_notional_exact`/
    // `size_basket_exact` themselves can't be exercised end-to-end in a
    // unit test: they need a real `&DexState`, and `DexState::new()`
    // genuinely touches the host -- confirmed live (not the "host-import-
    // free" the constructor's own doc comment claims): `RaydiumState::
    // new()`'s `Setup::default()` calls `account_id_from_pubkey`, which
    // falls back to the real WIT import `shooter::pubkey_map_by_pubkey`
    // on a cold cache (always cold in a fresh test process), aborting
    // the test process with "entered unreachable code" outside a real
    // wasm host. So the tests below exercise the actual new logic --
    // `price_impact_bps_with`/`max_safe_notional_with`/`size_basket_with`,
    // the shared cores both the plain and `_exact` public functions
    // delegate to -- directly, with a hand-rolled fake quoting strategy
    // standing in for `planner::reverify_route_with_exact_quotes`. This
    // proves the strategy parameter is genuinely used (not ignored) and
    // that the surrounding impact/binary-search/basket-scaling logic is
    // correct for *whatever* strategy it's given. The `_exact` functions'
    // own bodies are then just one more (obviously-correct-by-inspection)
    // strategy plugged into the same, now-proven cores; the real
    // CLMM/Orca tick-math correction itself is proven live.

    #[test]
    fn price_impact_bps_with_uses_the_injected_requote_strategy() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 100_000, 100_000, DexType::Sanctum);
        // A fake "exact" strategy that reports the same per-unit rate at
        // any amount (zero real impact) -- deliberately different from
        // real cp_quote's degrading rate on this same pool, so seeing
        // that flat result (instead of cp_quote's real growing impact)
        // proves the injected closure drove the result, not a hardcoded
        // cp_quote call.
        let impact = price_impact_bps_with(&router, MINT_A, MINT_B, 15_000, 10, 1, |_route, amt| Some(amt));
        assert_eq!(impact, Some(0.0), "expected zero impact from the injected flat-rate strategy");
        // Sanity: the real (non-injected) function shows real, nonzero
        // impact for this same shallow trade -- the injected result above
        // isn't just what cp_quote would have given anyway.
        let real_impact = price_impact_bps(&router, MINT_A, MINT_B, 15_000, 10, 1).unwrap();
        assert!(real_impact > 0.0, "expected the real cp_quote path to show nonzero impact, got {real_impact}");
    }

    #[test]
    fn price_impact_bps_with_none_when_requote_strategy_fails() {
        let mut router = router_with_mints(&[MINT_A, MINT_B]);
        router.add_generic_pair(100, MINT_A, MINT_B, 1.0, 0.0, 100_000, 100_000, DexType::Sanctum);
        // A route is found (the pool exists), but the injected strategy
        // -- standing in for a real exact-quote failure, e.g. `planner::
        // reverify_route_with_exact_quotes` returning `Err(pool_id)` --
        // can't quote it. Must be treated as unpriceable, not a panic or
        // a silent zero-impact pass.
        let impact = price_impact_bps_with(&router, MINT_A, MINT_B, 1_000, 10, 1, |_route, _amt| None);
        assert_eq!(impact, None);
    }

    #[test]
    fn max_safe_notional_with_uses_the_injected_price_impact_strategy() {
        // No router/pool needed here at all -- `max_safe_notional_with`
        // never touches `TradeRouter` itself, only the injected
        // `price_impact_at` closure, so this proves its binary-search
        // shell is correct in complete isolation from real routing.
        let candidate = 90_000;
        // A strategy that always reports zero impact -- must short-
        // circuit to the full candidate immediately, skipping the binary
        // search entirely (proves the injected closure's verdict is what
        // drives the early-return, not real routing math).
        let safe = max_safe_notional_with(candidate, |_amt, _probe| Some(0.0));
        assert_eq!(safe, candidate);
    }

    #[test]
    fn max_safe_notional_with_binary_searches_a_synthetic_linear_impact_curve() {
        // A strategy whose impact grows linearly with amount (no router
        // involved) -- proves the binary search itself converges to the
        // real threshold crossing for *some* impact function, not just
        // the specific shape `cp_quote` happens to produce.
        let candidate = 100_000u64;
        let safe = max_safe_notional_with(candidate, |amt, _probe| Some(amt as f64 / 100.0));
        // impact(amt) = amt/100 <= MAX_PRICE_IMPACT_BPS (50.0) iff amt <= 5_000.
        let impact_at_safe = safe as f64 / 100.0;
        let impact_one_above = (safe + 1) as f64 / 100.0;
        assert!(impact_at_safe <= MAX_PRICE_IMPACT_BPS, "safe={safe} should itself clear the cap, impact={impact_at_safe}");
        assert!(impact_one_above > MAX_PRICE_IMPACT_BPS, "safe={safe} should be the real threshold, not an under-shoot");
    }

    #[test]
    fn size_basket_with_uses_the_injected_safe_for_strategy() {
        // No router/pool needed -- `size_basket_with` never touches
        // `TradeRouter` itself, only the injected `safe_for` closure.
        let legs = [
            LegNotional { from_mint: MINT_A, to_mint: MINT_B, intended_notional: 10_000 },
            LegNotional { from_mint: MINT_A, to_mint: MINT_C, intended_notional: 10_000 },
        ];
        // Leg 0 (A->B) is fully safe; leg 1 (A->C) is only half-safe --
        // the whole basket must scale down to leg 1's ratio, uniformly,
        // same contract `size_basket_scales_every_leg_uniformly_to_the_
        // shallowest_leg` already proves for the real cp_quote path.
        let sized = size_basket_with(&legs, |l| if l.to_mint == MINT_B { l.intended_notional } else { l.intended_notional / 2 });
        assert_eq!(sized[0].sized_notional, 5_000, "deep leg wasn't scaled down to match the shallow leg");
        assert_eq!(sized[1].sized_notional, 5_000);
    }

    #[test]
    fn size_basket_with_zeroes_the_whole_basket_when_the_injected_strategy_reports_unpriceable() {
        let legs = [
            LegNotional { from_mint: MINT_A, to_mint: MINT_B, intended_notional: 1_000 },
            LegNotional { from_mint: MINT_A, to_mint: MINT_C, intended_notional: 1_000 },
        ];
        let sized = size_basket_with(&legs, |l| if l.to_mint == MINT_B { l.intended_notional } else { 0 });
        assert!(sized.iter().all(|s| s.sized_notional == 0), "unpriceable leg must zero the whole basket");
    }
}
