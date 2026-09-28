//! Real basket construction and entry/exit signal logic for trade type 3
//! ("dispersion") of `brain::multimodelv1::PLAN-1.md` -- `INSTRUCTIONS.md`
//! §3C's "long a basket of high-idiosyncratic-volatility spot tokens...
//! shorting the market index," made concrete and executable. Pure, no
//! host-import dependency, same testability discipline as
//! `factor_basket.rs`/`factor_residual.rs`.
//!
//! **Unlike trade type 1 (directional), entry here is automated, not
//! human-specified** -- the underlying signal (aggregate idiosyncratic
//! volatility elevated vs. its own recent history) is a real, computable
//! quantity this codebase already tracks per symbol
//! (`factor_residual::RollingWindow::stats().stdev`), not an invented
//! stock-picking call. The caller (`state.rs`) is responsible for
//! resolving each candidate's real `residual_stdev` and for tracking the
//! aggregate series' own rolling z-score (a second, separate
//! `factor_residual::RollingWindow` over the cross-sectional mean stdev,
//! reusing that same pure type for a new purpose) -- this module only
//! ranks/weights the basket and evaluates the entry/exit thresholds
//! against an already-resolved z-score.
//!
//! Unlike directional trading's per-factor hedge basket (short several
//! proxies, one per factor), dispersion's short side is a single
//! instrument (the market-index perp) sized once as the long basket's
//! aggregate dollar-beta on the leading factor -- see
//! [`aggregate_market_factor_exposure`].

use crate::graph::AccountId;

/// One curated symbol eligible for the long-basket, with its own
/// real, already-resolved idiosyncratic-volatility signal --
/// `residual_stdev` is `factor_residual::RollingWindow::stats().stdev`
/// for this mint's rolling residual window (0.0 or absent candidates
/// should simply not be included by the caller; `MIN_SAMPLES_FOR_ZSCORE`
/// warm-up is the caller's own concern, same split every other pure
/// module in this crate uses).
#[derive(Debug, Clone, Copy)]
pub struct DispersionCandidate {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub token_index: usize,
    pub residual_stdev: f64,
}

/// One real long leg of a dispersion basket -- `weight_fraction` is this
/// leg's share (0.0-1.0, every leg in a basket sums to 1.0) of the
/// trade's total long notional, not a raw amount; the caller converts it
/// to a real USD notional against its own intended total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispersionBasketLeg {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub weight_fraction: f64,
}

/// Target basket size -- top this many curated candidates by real
/// idiosyncratic volatility. Starting value, not calibrated against any
/// real trading history yet.
// TEMPORARY (2026-09-02, Phase 7 live verification): lowered from the
// real starting value of 8 to 4 at the user's explicit request, to make
// a real open more likely to succeed -- live-confirmed this session that
// `factor_sizing::size_basket` zeroes the *entire* basket if even one of
// the `DISPERSION_BASKET_SIZE` candidate legs has no real spot route
// from USDC at all (`max_safe_notional` returns 0), and every real
// entry-threshold crossing so far (8 of them, up to z=5.13) has been
// refused this way. Fewer legs means fewer chances any single one is
// unroutable. Not a recalibration -- revert to 8 once a real open (and
// ideally a real close) has been observed, or once the specific
// unroutable mint(s) are identified (`size_dispersion_long_legs`'s own
// culprit-finder diagnostic) and excluded some other way.
pub const DISPERSION_BASKET_SIZE: usize = 4;

/// Minimum number of real, warmed-up candidates required before a basket
/// is buildable at all -- below this, the curated universe doesn't have
/// enough real data yet (or is too thin) for a meaningfully diversified
/// dispersion basket, so refuse rather than open a too-small one.
// TEMPORARY (2026-09-02, Phase 7 live verification): lowered from 4 to 2
// at the user's explicit request. Real, live-confirmed this session (after
// the `factor_sizing::*_exact` CLMM fix): the aggregate z-score clears the
// real entry threshold almost every cycle, but only 1-2 of the ~104
// warmed-up curated mints ever clear *both* the buy- and sell-direction
// liquidity floors at once -- never reaching 4, across a real 35+ minute
// run. Dispersion ranks candidates by highest idiosyncratic volatility,
// and those tend to be the thinnest names, so a small surviving pool is
// plausible, not obviously a bug. Not a recalibration -- revert to 4 (or
// higher) once a real open (and ideally a real close) has been observed,
// or once the real candidate pool is reliably wider.
pub const MIN_DISPERSION_BASKET_SIZE: usize = 2;

/// Minimum share of the basket's total notional any surviving leg is
/// allowed to receive, once [`build_dispersion_basket`] blends pure
/// stdev-proportional weighting toward equal weighting (see that
/// function's doc comment for the exact formula and why).
// TEMPORARY (2026-09-04, Phase 7 live verification): added after a real,
// live-confirmed incident -- with pure stdev-proportional weighting (no
// floor) and DISPERSION_BASKET_SIZE=4, one dominant-volatility candidate
// (e.g. "jupS..x93v") repeatedly claimed ~90% of a real $40 basket,
// leaving the other 3 legs at $1-1.5 each. At that size, real per-hop
// integer-rounding/price-impact costs on a multi-hop route made those
// legs fail `size_dispersion_long_legs`'s MIN_SIZED_FRACTION_OF_INTENDED
// floor even though the underlying mint had real, deep liquidity
// (confirmed live: $719k for one such culprit via Jupiter's price API) --
// not a routing bug, just economically nonviable at that dollar size.
// Because `factor_sizing::size_basket_with` scales the *whole* basket
// uniformly to match its worst leg ("never partial"), that one dust-tier
// leg blocked every real open across 10+ consecutive real cycles, even
// though the dominant leg would have sized fine alone. 0.15 (15%) with
// DISPERSION_BASKET_SIZE=4 guarantees at least $6 of a $40 basket per
// leg -- comfortably above the sub-$2 range where this broke. Revert
// (or recalibrate against a real basket size) once a real open has been
// observed, same as the two adjacent TEMPORARY overrides in this file.
pub const MIN_LEG_WEIGHT_FRACTION: f64 = 0.15;

/// Real basket selection: sorts `candidates` by `residual_stdev`
/// descending, takes the top [`DISPERSION_BASKET_SIZE`] (fewer if the
/// universe doesn't have that many), weights each surviving leg as a
/// blend of equal weighting (a guaranteed [`MIN_LEG_WEIGHT_FRACTION`]
/// floor) and pure stdev-proportional weighting (the remainder,
/// distributed by each leg's own share of the total stdev) --
/// `floor + (1 - floor * n) * (stdev_i / total_stdev)`, which sums to
/// exactly 1.0 across `n` legs and never lets any leg fall below the
/// floor, unlike a naive clamp-then-renormalize (which can still leave a
/// floored leg below the nominal floor once a dominant leg's excess
/// weight is folded back in -- see [`MIN_LEG_WEIGHT_FRACTION`]'s doc
/// comment for the real incident this fixes).
/// Candidates with a non-positive or non-finite `residual_stdev` are
/// dropped before ranking -- no real volatility signal to weight by.
///
/// Returns `None` if fewer than [`MIN_DISPERSION_BASKET_SIZE`] real
/// candidates survive.
pub fn build_dispersion_basket(candidates: &[DispersionCandidate]) -> Option<Vec<DispersionBasketLeg>> {
    let mut ranked: Vec<&DispersionCandidate> =
        candidates.iter().filter(|c| c.residual_stdev.is_finite() && c.residual_stdev > 0.0).collect();
    if ranked.len() < MIN_DISPERSION_BASKET_SIZE {
        return None;
    }
    ranked.sort_by(|a, b| b.residual_stdev.total_cmp(&a.residual_stdev));
    ranked.truncate(DISPERSION_BASKET_SIZE);

    let stdevs: Vec<f64> = ranked.iter().map(|c| c.residual_stdev).collect();
    let weights = weight_fractions_for(&stdevs)?;
    Some(
        ranked
            .into_iter()
            .zip(weights)
            .map(|(c, weight_fraction)| DispersionBasketLeg { symbol: c.symbol, mint: c.mint, weight_fraction })
            .collect(),
    )
}

/// The floor/proportional blend itself, decoupled from
/// [`DispersionCandidate`]/ranking so it's directly testable at any `n`,
/// including the degenerate `MIN_LEG_WEIGHT_FRACTION * n >= 1.0` case
/// that [`build_dispersion_basket`]'s own truncation to
/// [`DISPERSION_BASKET_SIZE`] can never actually reach today. `None` if
/// `stdevs` is empty or sums to <= 0.0 (no real signal to weight by).
fn weight_fractions_for(stdevs: &[f64]) -> Option<Vec<f64>> {
    let total_stdev: f64 = stdevs.iter().sum();
    if stdevs.is_empty() || total_stdev <= 0.0 {
        return None;
    }
    let n = stdevs.len() as f64;
    // Falls back to pure equal weighting if the floor alone would already
    // consume (or exceed) the whole basket -- only possible if
    // DISPERSION_BASKET_SIZE ever grows past 1/MIN_LEG_WEIGHT_FRACTION
    // without this floor being revisited together with it. The blend
    // formula's `remainder` would go negative in that case, which would
    // both let a low-stdev leg's weight go negative *and* stop summing
    // to 1.0 -- equal weighting is the correct degenerate case (the
    // floor wants every leg equal anyway, it just can't fit alongside a
    // proportional remainder).
    if MIN_LEG_WEIGHT_FRACTION * n >= 1.0 {
        return Some(vec![1.0 / n; stdevs.len()]);
    }
    let remainder = 1.0 - MIN_LEG_WEIGHT_FRACTION * n;
    Some(stdevs.iter().map(|&s| MIN_LEG_WEIGHT_FRACTION + remainder * (s / total_stdev)).collect())
}

/// The long basket's aggregate dollar-beta on the leading (market/beta)
/// factor -- `eigenvectors`/factor indexing follows `factor_graph::
/// StructuralFactors`'s own `[token][factor]` convention exactly, same
/// as `factor_basket::build_directional_basket`. This is the short-index
/// leg's target USD notional: `Σ (leg_notional_usd_i *
/// eigenvectors[token_index_i][0])`. A leg whose mint doesn't resolve to
/// a real token index (via `token_index_of`) contributes nothing rather
/// than failing the whole calculation -- same fail-soft-per-leg
/// convention `build_directional_basket` uses for a missing proxy.
pub fn aggregate_market_factor_exposure(
    basket: &[(DispersionBasketLeg, f64)],
    eigenvectors: &[Vec<f64>],
    token_index_of: impl Fn(AccountId) -> Option<usize>,
) -> f64 {
    const MARKET_FACTOR_INDEX: usize = 0;
    basket
        .iter()
        .map(|(leg, notional_usd)| {
            let Some(idx) = token_index_of(leg.mint) else { return 0.0 };
            let Some(row) = eigenvectors.get(idx) else { return 0.0 };
            let Some(loading) = row.get(MARKET_FACTOR_INDEX) else { return 0.0 };
            notional_usd * loading
        })
        .sum()
}

/// Entry threshold for the aggregate cross-sectional idiosyncratic-
/// volatility z-score -- how many σ above the series' own recent mean
/// counts as "elevated enough to open." Starting value, not calibrated
/// against any real trading history yet.
// TEMPORARY (2026-09-02, Phase 7 live verification): first lowered from
// the real starting value of 2.0 to 0.6 (see prior revision), then
// dropped further to -1.5 at the user's explicit request ("I would like
// to trigger at least one trade") after real observation showed the
// aggregate z-score sitting well below even 0.6 for an extended stretch
// (recent real readings around -0.75 to -1.3, after `DISPERSION_BASKET_
// SIZE` was also temporarily shrunk to address the real sizing/routing
// blocker -- see that constant's own TEMPORARY note). Below `DISPERSION_
// EXIT_ZSCORE` on purpose at this setting -- a real open is expected to
// satisfy the exit condition again almost immediately, giving a fast
// real open+close round trip purely to verify the execution path, not a
// meaningful trading signal at this value. Not a recalibration -- revert
// to 2.0 once a real open (and ideally a real close) has been observed.
pub const DISPERSION_ENTRY_ZSCORE: f64 = -1.5;

/// Exit threshold -- once the aggregate series reverts back to (or
/// below) this many σ, the dispersion thesis has normalized and the
/// position is closed. Deliberately lower than [`DISPERSION_ENTRY_
/// ZSCORE`] (an asymmetric band, not a single toggle point) so the trade
/// doesn't open and close every cycle right at the entry boundary.
pub const DISPERSION_EXIT_ZSCORE: f64 = 0.5;

/// `true` once the aggregate idiosyncratic-volatility z-score clears the
/// entry threshold -- the only automated open signal this trade type
/// has.
pub fn should_enter_dispersion(aggregate_zscore: f64) -> bool {
    aggregate_zscore >= DISPERSION_ENTRY_ZSCORE
}

/// `true` once the aggregate idiosyncratic-volatility z-score has
/// reverted back down to (or below) the exit threshold.
pub fn should_exit_dispersion(aggregate_zscore: f64) -> bool {
    aggregate_zscore <= DISPERSION_EXIT_ZSCORE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn cand(symbol: &'static str, mint: AccountId, token_index: usize, residual_stdev: f64) -> DispersionCandidate {
        DispersionCandidate { symbol, mint, token_index, residual_stdev }
    }

    fn candidates_of_len(n: usize) -> Vec<DispersionCandidate> {
        (0..n).map(|i| cand("S", i as AccountId, i, 1.0 + i as f64)).collect()
    }

    #[test]
    fn build_dispersion_basket_truncates_to_basket_size() {
        let candidates = candidates_of_len(DISPERSION_BASKET_SIZE + 5);
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        assert_eq!(basket.len(), DISPERSION_BASKET_SIZE);
    }

    #[test]
    fn build_dispersion_basket_picks_highest_stdev_candidates() {
        // Only the top DISPERSION_BASKET_SIZE by stdev should survive --
        // candidate 0 (lowest stdev) must be dropped once more than
        // DISPERSION_BASKET_SIZE candidates are offered.
        let candidates = candidates_of_len(DISPERSION_BASKET_SIZE + 1);
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        let mints: Vec<AccountId> = basket.iter().map(|l| l.mint).collect();
        assert!(!mints.contains(&0), "lowest-stdev candidate should have been dropped");
    }

    #[test]
    fn build_dispersion_basket_weights_sum_to_one() {
        let candidates = candidates_of_len(DISPERSION_BASKET_SIZE);
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        let total: f64 = basket.iter().map(|l| l.weight_fraction).sum();
        assert!(approx(total, 1.0), "weights should sum to 1.0, got {total}");
    }

    #[test]
    fn build_dispersion_basket_weights_blend_floor_and_stdev_proportion() {
        let candidates = vec![cand("A", 1, 1, 1.0), cand("B", 2, 2, 2.0), cand("C", 3, 3, 1.0), cand("D", 4, 4, 4.0)];
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        let w = |sym: &str| basket.iter().find(|l| l.symbol == sym).unwrap().weight_fraction;
        // total stdev = 1+2+1+4 = 8, n = 4, remainder = 1 - 0.15*4 = 0.4
        // weight = 0.15 + 0.4 * (stdev / 8)
        assert!(approx(w("A"), 0.15 + 0.4 * (1.0 / 8.0)));
        assert!(approx(w("B"), 0.15 + 0.4 * (2.0 / 8.0)));
        assert!(approx(w("D"), 0.15 + 0.4 * (4.0 / 8.0)));
    }

    #[test]
    fn build_dispersion_basket_higher_stdev_still_gets_more_weight() {
        // The floor changes the *magnitude* of the spread, not the
        // ranking -- a leg with more real stdev must still end up with
        // strictly more weight than one with less.
        let candidates = vec![cand("Low", 1, 1, 1.0), cand("High", 2, 2, 4.0)];
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        let w = |sym: &str| basket.iter().find(|l| l.symbol == sym).unwrap().weight_fraction;
        assert!(w("High") > w("Low"));
    }

    #[test]
    fn build_dispersion_basket_no_leg_falls_below_the_floor() {
        // One wildly dominant candidate (real, live-confirmed pattern:
        // one high-volatility symbol claiming ~90% under pure
        // proportional weighting) must not starve the others below
        // MIN_LEG_WEIGHT_FRACTION.
        let candidates = vec![cand("Dominant", 1, 1, 1_000_000.0), cand("A", 2, 2, 1.0), cand("B", 3, 3, 1.0), cand("C", 4, 4, 1.0)];
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        for leg in &basket {
            assert!(
                leg.weight_fraction >= MIN_LEG_WEIGHT_FRACTION - 1e-9,
                "{} got weight {}, below the {} floor",
                leg.symbol,
                leg.weight_fraction,
                MIN_LEG_WEIGHT_FRACTION,
            );
        }
    }

    #[test]
    fn weight_fractions_for_degenerate_floor_falls_back_to_equal_weight() {
        // If MIN_LEG_WEIGHT_FRACTION * n >= 1.0 (only reachable if
        // DISPERSION_BASKET_SIZE ever grows past
        // 1/MIN_LEG_WEIGHT_FRACTION without the floor being revisited --
        // build_dispersion_basket's own truncation can't reach this
        // today), every leg must still get an equal, valid (sums to 1.0,
        // all positive) weight rather than a negative one.
        let n = (1.0 / MIN_LEG_WEIGHT_FRACTION).ceil() as usize;
        let stdevs: Vec<f64> = (0..n).map(|i| 1.0 + i as f64).collect();
        let weights = weight_fractions_for(&stdevs).expect("should still produce weights");
        let total: f64 = weights.iter().sum();
        assert!(approx(total, 1.0), "weights should still sum to 1.0, got {total}");
        for &w in &weights {
            assert!(approx(w, 1.0 / n as f64), "expected equal weight {}, got {w}", 1.0 / n as f64);
        }
    }

    #[test]
    fn weight_fractions_for_empty_is_none() {
        assert!(weight_fractions_for(&[]).is_none());
    }

    #[test]
    fn build_dispersion_basket_drops_non_positive_and_non_finite_stdev() {
        let candidates = vec![
            cand("A", 1, 1, 1.0),
            cand("B", 2, 2, 2.0),
            cand("C", 3, 3, 3.0),
            cand("Zero", 4, 4, 0.0),
            cand("Neg", 5, 5, -1.0),
            cand("Nan", 6, 6, f64::NAN),
            cand("D", 7, 7, 4.0),
        ];
        let basket = build_dispersion_basket(&candidates).expect("should build a basket");
        let symbols: Vec<&str> = basket.iter().map(|l| l.symbol).collect();
        assert!(!symbols.contains(&"Zero"));
        assert!(!symbols.contains(&"Neg"));
        assert!(!symbols.contains(&"Nan"));
        assert_eq!(basket.len(), 4);
    }

    #[test]
    fn build_dispersion_basket_none_below_minimum_size() {
        let candidates = candidates_of_len(MIN_DISPERSION_BASKET_SIZE - 1);
        assert!(build_dispersion_basket(&candidates).is_none());
    }

    #[test]
    fn build_dispersion_basket_ok_at_exactly_minimum_size() {
        let candidates = candidates_of_len(MIN_DISPERSION_BASKET_SIZE);
        assert!(build_dispersion_basket(&candidates).is_some());
    }

    #[test]
    fn build_dispersion_basket_none_for_empty_candidates() {
        assert!(build_dispersion_basket(&[]).is_none());
    }

    // --- aggregate_market_factor_exposure -------------------------------

    #[test]
    fn aggregate_market_factor_exposure_sums_dollar_beta() {
        let eigenvectors = vec![
            vec![0.5, 0.1], // token_index 0
            vec![0.2, 0.9], // token_index 1
        ];
        let basket = vec![
            (DispersionBasketLeg { symbol: "A", mint: 10, weight_fraction: 0.5 }, 100.0),
            (DispersionBasketLeg { symbol: "B", mint: 20, weight_fraction: 0.5 }, 200.0),
        ];
        let token_index_of = |mint: AccountId| match mint {
            10 => Some(0),
            20 => Some(1),
            _ => None,
        };
        let exposure = aggregate_market_factor_exposure(&basket, &eigenvectors, token_index_of);
        // 100*0.5 + 200*0.2 = 50 + 40 = 90
        assert!(approx(exposure, 90.0));
    }

    #[test]
    fn aggregate_market_factor_exposure_skips_unresolvable_leg() {
        let eigenvectors = vec![vec![0.5, 0.1]];
        let basket = vec![
            (DispersionBasketLeg { symbol: "A", mint: 10, weight_fraction: 0.5 }, 100.0),
            (DispersionBasketLeg { symbol: "Unknown", mint: 999, weight_fraction: 0.5 }, 500.0),
        ];
        let token_index_of = |mint: AccountId| if mint == 10 { Some(0) } else { None };
        let exposure = aggregate_market_factor_exposure(&basket, &eigenvectors, token_index_of);
        assert!(approx(exposure, 50.0));
    }

    #[test]
    fn aggregate_market_factor_exposure_zero_for_empty_basket() {
        let eigenvectors = vec![vec![0.5, 0.1]];
        let exposure = aggregate_market_factor_exposure(&[], &eigenvectors, |_| None);
        assert!(approx(exposure, 0.0));
    }

    // --- should_enter_dispersion / should_exit_dispersion ---------------

    #[test]
    fn should_enter_dispersion_boundaries() {
        assert!(should_enter_dispersion(DISPERSION_ENTRY_ZSCORE));
        assert!(!should_enter_dispersion(DISPERSION_ENTRY_ZSCORE - 0.01));
    }

    #[test]
    fn should_enter_dispersion_true_well_above_threshold() {
        assert!(should_enter_dispersion(10.0));
    }

    #[test]
    fn should_exit_dispersion_boundaries() {
        assert!(should_exit_dispersion(DISPERSION_EXIT_ZSCORE));
        assert!(!should_exit_dispersion(DISPERSION_EXIT_ZSCORE + 0.01));
    }

    #[test]
    fn should_exit_dispersion_true_at_zero_and_negative() {
        assert!(should_exit_dispersion(0.0));
        assert!(should_exit_dispersion(-5.0));
    }

    #[test]
    fn enter_and_exit_thresholds_leave_a_hold_band() {
        // Between the exit and entry thresholds, neither fires -- a real
        // hold band, not a single toggle point. Only checked when entry
        // is genuinely above exit (the real, calibrated relationship) --
        // `DISPERSION_ENTRY_ZSCORE`'s own TEMPORARY live-verification
        // override can push entry *below* exit on purpose (deliberately
        // collapsing the hold band to force a fast open+close for
        // execution-path verification), which this invariant doesn't
        // apply to; skip rather than fail during that override.
        if DISPERSION_ENTRY_ZSCORE <= DISPERSION_EXIT_ZSCORE {
            return;
        }
        let mid = (DISPERSION_ENTRY_ZSCORE + DISPERSION_EXIT_ZSCORE) / 2.0;
        assert!(!should_enter_dispersion(mid));
        assert!(!should_exit_dispersion(mid));
    }
}
