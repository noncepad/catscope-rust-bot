//! Borrow-cost gating for `multimodelv1`'s short legs -- Phase 2 of
//! `brain::multimodelv1::PLAN-1.md`: every short this bot mode's
//! factor-neutral hedges, pair trades, and dispersion index leg would
//! ever open must clear its real borrow cost before opening, and must
//! keep clearing it every cycle it stays open, or get force-closed.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `derivative_router.rs`/`credit.rs`/`pricegraph.rs`/`factor_graph.rs` --
//! callers resolve real numbers (an expected reversion edge from the
//! caller's own residual model, an OU half-life estimate, real lending
//! reserves via `credit::CreditReserve::from_kamino`/`from_solend`/
//! `from_marginfi`) and hand them to this module.
//!
//! This is a **new, standalone module**, not a call into
//! `derivative_router::decide_basis_trade` -- same "duplicate the shape,
//! don't share the function" reasoning `derivative_router`'s own doc
//! comment already gives for not calling into `perpfundingv1`: the
//! economics here are genuinely different (an annualized reversion edge
//! against a holding-period estimate, not a funding rate against a
//! deposit/borrow choice), even though the *gate shape* -- real cost
//! read live, checked before opening, re-checked every cycle after -- is
//! deliberately the same discipline `PLAN-1.md`'s critique of the
//! original `INSTRUCTIONS.md` draft asked for.

use crate::trader::credit::CreditReserve;

/// Multiplier applied to a raw OU half-life estimate before treating it
/// as the expected holding period [`decide_short_leg`] annualizes
/// against. Half-life estimates fit from short, noisy real Solana price
/// history are not trustworthy point estimates -- padding the assumed
/// holding period is the conservative direction (a *longer* assumed
/// holding period lowers the annualized edge the gate computes, making
/// the gate harder to pass, never easier). `2.0` is a starting value,
/// not a calibrated one -- there's no real trading history yet to tune
/// it against; see [`expected_holding_period_years`]'s tests for what
/// this actually does to the gate's boundary.
pub const HALF_LIFE_SAFETY_MULTIPLIER: f64 = 2.0;

/// Applies [`HALF_LIFE_SAFETY_MULTIPLIER`] to a raw half-life estimate. A
/// non-positive or otherwise degenerate `raw_half_life_years` (no real
/// estimate available) clamps to `0.0`, which makes
/// [`decide_short_leg`] refuse to gate anything open -- "no holding-period
/// estimate" must never be silently treated as "instant reversion."
pub fn expected_holding_period_years(raw_half_life_years: f64) -> f64 {
    raw_half_life_years.max(0.0) * HALF_LIFE_SAFETY_MULTIPLIER
}

/// The real gate: does a short leg's annualized expected edge clear its
/// real borrow cost? `expected_reversion_edge_pct` and `borrow_apy_pct`
/// are both **percent** units (matching `derivative_router`'s own
/// convention -- `CreditReserve::borrow_apy` is a 0.0-1.0 fraction,
/// multiply by 100 before calling this; [`gate_short_legs`] does this for
/// you). `expected_holding_period_years` should already have
/// [`expected_holding_period_years`]'s safety multiplier applied.
///
/// Used both to gate a new open (`PLAN-1.md` Phase 2's "before opening"
/// bullet) and to re-check an already-open leg every cycle (the "while
/// open" bullet) -- same function either way, same as
/// `derivative_router::decide_basis_trade` being reused for both
/// `find_best_funding_opportunities`'s open pass and `run_basis_cycle`'s
/// close pass in `leveragedloopv1`. A caller re-checking an open
/// position force-closes it the moment this flips to `false` (a real
/// borrow-rate spike, not just a reverted residual, is enough on its
/// own to end the trade) -- this function has no memory of what it
/// returned last cycle, that state lives in the caller.
///
/// `expected_holding_period_years <= 0.0` (no real estimate) always
/// refuses -- an edge can't be annualized against an unknown holding
/// period, and treating "unknown" as "zero" would make every nonzero
/// edge look infinitely good.
pub fn decide_short_leg(
    expected_reversion_edge_pct: f64,
    expected_holding_period_years: f64,
    borrow_apy_pct: f64,
) -> bool {
    if expected_holding_period_years <= 0.0 {
        return false;
    }
    let annualized_edge_pct = expected_reversion_edge_pct / expected_holding_period_years;
    annualized_edge_pct > borrow_apy_pct
}

/// Maximum real borrow APY (percent units) a directional-neutral hedge
/// basket's short leg (`brain::multimodelv1`'s trade type 1) may cost --
/// a simple absolute affordability cap, deliberately *not* an
/// edge/holding-period ratio like [`decide_short_leg`]. That trade type's
/// entry is a human directional bet (see `PLAN-1.md`'s directional
/// -neutral design notes), not a residual-model estimate with a real
/// expected-reversion-edge number -- synthesizing a fake edge just to
/// reuse [`decide_short_leg`]'s formula would be dishonest arithmetic,
/// not a real gate. Starting value, no live trading history to calibrate
/// against yet.
pub const MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT: f64 = 20.0;

/// The directional-neutral hedge basket's own borrow gate -- see
/// [`MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT`]'s doc comment for why this is
/// a plain affordability cap rather than [`decide_short_leg`]'s
/// edge-annualized-against-holding-period shape. Used identically both
/// to gate a new leg before opening and to re-check every already-open
/// leg every cycle (same "no memory of last cycle's result" discipline
/// as [`decide_short_leg`]) -- a caller re-checking an open basket
/// force-closes the whole basket the moment any single leg's rate spikes
/// past this cap.
///
/// Non-strict at the boundary (`<=`, changed 2026-09-07): live-observed a
/// real covered proxy sitting at *exactly* [`MAX_DIRECTIONAL_SHORT_BORROW_
/// APY_PCT`] (20.00%) refused solely because the old strict `<` treated
/// the boundary itself as "too expensive" -- this is an affordability
/// cap, not a tie-breaker that needs a strict edge, so a rate exactly
/// *at* the cap is affordable.
pub fn decide_directional_short_leg(borrow_apy_pct: f64) -> bool {
    borrow_apy_pct <= MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT
}

/// Real, higher cap [`decide_directional_short_leg_with_carry`] applies
/// instead of [`MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT`] when the basket's
/// own real net carry is positive (see
/// [`directional_basket_is_net_profitable`]) -- user-directed
/// (2026-09-06): tolerating a costlier individual short leg is justified
/// once the basket's *own* real, already-known yield/cost spread (the
/// long leg's real Kamino/Solend supply APY against the short basket's
/// real blended borrow APY) is net positive, independent of the human's
/// directional price bet -- this is real, already-observed interest-rate
/// data, not a fabricated edge, so it doesn't run into
/// [`MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT`]'s own "don't synthesize a
/// fake edge" objection. Starting value, no live trading history to
/// calibrate against yet, same as the flat cap it supplements.
pub const MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_PROFITABLE: f64 = 30.0;

/// `true` iff the hedge basket's real net carry -- `long_supply_apy_pct`
/// (the target's own real Kamino/Solend supply APY while deposited) minus
/// `blended_short_borrow_apy_pct` (the short basket's real notional- or
/// weight-weighted average borrow APY across every leg) -- is positive.
/// Both arguments are real, already-known percent-unit numbers (real
/// reserve data, not a forecast), so this is never a fabricated
/// profitability signal -- just whether the position is net profitable to
/// simply hold on its own real yield/cost spread, price direction aside.
/// Strict inequality: an exactly-break-even carry doesn't count as
/// "profitable," same boundary discipline as this module's other gates.
pub fn directional_basket_is_net_profitable(long_supply_apy_pct: f64, blended_short_borrow_apy_pct: f64) -> bool {
    long_supply_apy_pct > blended_short_borrow_apy_pct
}

/// Real per-leg borrow gate for trade type 1's hedge basket, generalizing
/// [`decide_directional_short_leg`] with a carry-dependent cap: `borrow_
/// apy_pct` must clear [`MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_
/// PROFITABLE`] if `basket_is_net_profitable` (from
/// [`directional_basket_is_net_profitable`]), or the original, lower
/// [`MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT`] otherwise. Same "no memory of
/// last cycle's result" discipline as [`decide_directional_short_leg`] --
/// used both to gate a new leg before opening and to re-check every
/// already-open leg every cycle.
pub fn decide_directional_short_leg_with_carry(borrow_apy_pct: f64, basket_is_net_profitable: bool) -> bool {
    let cap =
        if basket_is_net_profitable { MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_PROFITABLE } else { MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT };
    borrow_apy_pct <= cap
}

/// One short leg's real, resolved inputs for [`gate_short_legs`] -- the
/// caller's job to build (see the module doc comment).
pub struct ShortLegInput<'a> {
    pub token: &'static str,
    /// Expected total (not annualized) reversion edge, percent units --
    /// already resolved by the caller's own residual/OU model, not
    /// re-derived here.
    pub expected_reversion_edge_pct: f64,
    /// Raw OU half-life estimate for this token's residual, in years --
    /// [`HALF_LIFE_SAFETY_MULTIPLIER`] is applied inside
    /// [`gate_short_legs`], callers should pass the raw estimate, not a
    /// pre-padded one.
    pub raw_half_life_years: f64,
    /// Every real lending reserve for this leg's underlying mint the
    /// caller tracks. Empty means "no lending data yet" -- the leg is
    /// skipped, not treated as free to borrow.
    pub reserves: &'a [CreditReserve],
}

/// One short leg cleared to open (or stay open) right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GatedShortLeg {
    pub token: &'static str,
    /// The cheapest real borrow APY found across the leg's reserves,
    /// percent units -- the one [`decide_short_leg`] actually used.
    pub borrow_apy_pct: f64,
    /// The padded holding-period estimate actually used (post-
    /// [`HALF_LIFE_SAFETY_MULTIPLIER`]), for the caller's own logging/
    /// diagnostics -- not re-derivable from `annualized_edge_pct` alone.
    pub holding_period_years_used: f64,
    /// `expected_reversion_edge_pct / holding_period_years_used` -- the
    /// number [`decide_short_leg`] actually compared against
    /// `borrow_apy_pct`, exposed for ranking.
    pub annualized_edge_pct: f64,
}

/// The real, reusable search -- generalizes the borrow-cost gate across
/// every candidate short leg, same shape as
/// `derivative_router::find_best_funding_opportunities`: picks each
/// leg's cheapest real reserve, applies [`decide_short_leg`], and returns
/// only the legs that clear it, ranked by `annualized_edge_pct`
/// descending (best first). A leg with no reserves, or whose edge
/// doesn't clear the gate, is simply absent from the result.
pub fn gate_short_legs(inputs: &[ShortLegInput]) -> Vec<GatedShortLeg> {
    let mut out = Vec::new();
    for input in inputs {
        let Some(cheapest) = input.reserves.iter().min_by(|a, b| a.borrow_apy.total_cmp(&b.borrow_apy)) else {
            continue;
        };
        let borrow_apy_pct = cheapest.borrow_apy * 100.0;
        let holding_period_years_used = expected_holding_period_years(input.raw_half_life_years);
        if !decide_short_leg(input.expected_reversion_edge_pct, holding_period_years_used, borrow_apy_pct) {
            continue;
        }
        let annualized_edge_pct = input.expected_reversion_edge_pct / holding_period_years_used;
        out.push(GatedShortLeg {
            token: input.token,
            borrow_apy_pct,
            holding_period_years_used,
            annualized_edge_pct,
        });
    }
    out.sort_by(|a, b| b.annualized_edge_pct.total_cmp(&a.annualized_edge_pct));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table-driven, same style as derivative_router's decide_basis_trade
    // tests -- boundary behavior is the actual spec here.

    #[test]
    fn decide_short_leg_clears_when_annualized_edge_exceeds_borrow_cost() {
        // 20% edge over a 0.5yr holding period = 40%/yr annualized,
        // comfortably above a 10% borrow cost.
        assert!(decide_short_leg(20.0, 0.5, 10.0));
    }

    #[test]
    fn decide_short_leg_refuses_at_exact_boundary() {
        // 10%/yr annualized exactly equals a 10% borrow cost -- strict
        // inequality, boundary itself does not clear.
        assert!(!decide_short_leg(5.0, 0.5, 10.0));
    }

    #[test]
    fn decide_short_leg_refuses_just_below_boundary() {
        assert!(!decide_short_leg(4.9, 0.5, 10.0));
    }

    #[test]
    fn decide_short_leg_clears_just_above_boundary() {
        assert!(decide_short_leg(5.1, 0.5, 10.0));
    }

    #[test]
    fn decide_short_leg_refuses_on_zero_or_negative_holding_period() {
        assert!(!decide_short_leg(100.0, 0.0, 1.0));
        assert!(!decide_short_leg(100.0, -1.0, 1.0));
    }

    #[test]
    fn expected_holding_period_applies_safety_multiplier() {
        assert!((expected_holding_period_years(0.25) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn expected_holding_period_clamps_negative_to_zero() {
        assert_eq!(expected_holding_period_years(-1.0), 0.0);
    }

    #[test]
    fn safety_multiplier_makes_the_gate_strictly_harder_to_pass() {
        // Same raw inputs, only the safety multiplier differs (simulated
        // by comparing the padded vs. unpadded holding period directly)
        // -- confirms "conservative" isn't just a claim in the doc
        // comment, it's an enforced direction: a longer assumed holding
        // period must never help a leg clear the gate.
        let raw_half_life = 0.5;
        let padded = expected_holding_period_years(raw_half_life); // 1.0yr
        assert!(decide_short_leg(20.0, raw_half_life, 30.0)); // 40%/yr > 30%, clears unpadded
        assert!(!decide_short_leg(20.0, padded, 30.0)); // 20%/yr < 30%, refused once padded
    }

    // --- decide_directional_short_leg -----------------------------------

    #[test]
    fn decide_directional_short_leg_clears_below_cap() {
        assert!(decide_directional_short_leg(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT - 0.1));
    }

    #[test]
    fn decide_directional_short_leg_clears_at_exact_boundary() {
        // Non-strict inequality (changed 2026-09-07): a rate exactly at
        // the cap is affordable, not refused -- see this fn's doc comment.
        assert!(decide_directional_short_leg(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT));
    }

    #[test]
    fn decide_directional_short_leg_refuses_above_cap() {
        assert!(!decide_directional_short_leg(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT + 0.1));
    }

    // --- directional_basket_is_net_profitable / decide_directional_short_leg_with_carry ---

    #[test]
    fn net_profitable_true_when_long_yield_exceeds_short_cost() {
        assert!(directional_basket_is_net_profitable(25.0, 20.0));
    }

    #[test]
    fn net_profitable_false_at_exact_break_even() {
        // Strict inequality -- an exactly break-even carry doesn't count
        // as "profitable," matching this module's other gate boundaries.
        assert!(!directional_basket_is_net_profitable(20.0, 20.0));
    }

    #[test]
    fn net_profitable_false_when_short_cost_exceeds_long_yield() {
        assert!(!directional_basket_is_net_profitable(15.0, 20.0));
    }

    #[test]
    fn decide_directional_short_leg_with_carry_uses_flat_cap_when_not_profitable() {
        assert!(decide_directional_short_leg_with_carry(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT - 0.1, false));
        assert!(decide_directional_short_leg_with_carry(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT, false));
        // Would clear the higher, profitable-basket cap, but the basket
        // isn't profitable this call -- must still refuse.
        assert!(!decide_directional_short_leg_with_carry(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT + 5.0, false));
    }

    #[test]
    fn decide_directional_short_leg_with_carry_uses_higher_cap_when_profitable() {
        // Between the two caps -- refused flat, clears once the basket's
        // real net carry is positive.
        let between_caps = (MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT + MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_PROFITABLE) / 2.0;
        assert!(!decide_directional_short_leg_with_carry(between_caps, false));
        assert!(decide_directional_short_leg_with_carry(between_caps, true));
    }

    #[test]
    fn decide_directional_short_leg_with_carry_clears_at_exact_higher_boundary() {
        assert!(decide_directional_short_leg_with_carry(MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_PROFITABLE, true));
    }

    fn reserve(borrow_apy: f64) -> CreditReserve {
        CreditReserve {
            reserve_id: 1,
            mint: 100,
            max_ltv_pct: 0.7,
            borrow_apy,
            supply_apy: 0.0,
            available_liquidity_usd: 1_000_000.0,
        }
    }

    #[test]
    fn gate_short_legs_picks_cheapest_reserve_per_leg() {
        // Two lending protocols for the same mint -- Kamino at 15%,
        // Solend cheaper at 5% -- the gate should use the cheaper one.
        let reserves = [reserve(0.15), reserve(0.05)];
        let inputs = [ShortLegInput {
            token: "SOL",
            expected_reversion_edge_pct: 20.0,
            raw_half_life_years: 0.5, // padded to 1.0yr
            reserves: &reserves,
        }];
        let out = gate_short_legs(&inputs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].token, "SOL");
        assert!((out[0].borrow_apy_pct - 5.0).abs() < 1e-9);
        assert!((out[0].holding_period_years_used - 1.0).abs() < 1e-9);
        assert!((out[0].annualized_edge_pct - 20.0).abs() < 1e-9); // 20.0 / 1.0
    }

    #[test]
    fn gate_short_legs_skips_legs_with_no_edge_or_no_reserves() {
        let reserves = [reserve(0.30)];
        let no_reserves: [CreditReserve; 0] = [];
        let inputs = [
            // Edge doesn't clear the padded-holding-period borrow cost.
            ShortLegInput {
                token: "BTC",
                expected_reversion_edge_pct: 5.0,
                raw_half_life_years: 0.5, // padded to 1.0yr -> 5%/yr < 30%
                reserves: &reserves,
            },
            // No lending data at all yet -- skipped, not zero-cost.
            ShortLegInput {
                token: "ETH",
                expected_reversion_edge_pct: 50.0,
                raw_half_life_years: 0.1,
                reserves: &no_reserves,
            },
        ];
        assert!(gate_short_legs(&inputs).is_empty());
    }

    #[test]
    fn gate_short_legs_ranks_by_annualized_edge_descending() {
        let reserves = [reserve(0.05)];
        let inputs = [
            ShortLegInput {
                token: "SOL",
                expected_reversion_edge_pct: 10.0,
                raw_half_life_years: 0.5, // -> 10%/yr annualized
                reserves: &reserves,
            },
            ShortLegInput {
                token: "BTC",
                expected_reversion_edge_pct: 40.0,
                raw_half_life_years: 0.5, // -> 40%/yr annualized
                reserves: &reserves,
            },
            ShortLegInput {
                token: "ETH",
                expected_reversion_edge_pct: 20.0,
                raw_half_life_years: 0.5, // -> 20%/yr annualized
                reserves: &reserves,
            },
        ];
        let out = gate_short_legs(&inputs);
        let tokens: Vec<&str> = out.iter().map(|o| o.token).collect();
        assert_eq!(tokens, vec!["BTC", "ETH", "SOL"]);
    }

    #[test]
    fn gate_short_legs_reflects_a_mid_cycle_borrow_spike_forcing_a_close() {
        // Simulates PLAN-1.md's "while open" re-check: same leg, same
        // edge, checked twice -- once with the entry borrow rate, once
        // after a real spike. The caller (a future Phase 5 state
        // machine) is expected to force-close the moment this flips.
        let cheap = [reserve(0.05)];
        let spiked = [reserve(0.50)];
        let entry_input = [ShortLegInput {
            token: "SOL",
            expected_reversion_edge_pct: 20.0,
            raw_half_life_years: 0.5,
            reserves: &cheap,
        }];
        let recheck_input = [ShortLegInput {
            token: "SOL",
            expected_reversion_edge_pct: 20.0,
            raw_half_life_years: 0.5,
            reserves: &spiked,
        }];
        assert_eq!(gate_short_legs(&entry_input).len(), 1);
        assert!(gate_short_legs(&recheck_input).is_empty());
    }
}
