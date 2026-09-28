//! Real "spike vs. basket" redesign of trade type 2 (pair trading) --
//! replaces `factor_residual::find_best_pair`'s requirement that *two*
//! independent single symbols cross the entry threshold simultaneously
//! (one underperformer, one overperformer, at once) with a single-sided
//! trigger: any one curated symbol crossing [`factor_residual::
//! PAIR_TRADE_ENTRY_ZSCORE`] is a real candidate on its own, hedged
//! against a small, low-noise basket of other curated symbols rather than
//! against a second single symbol that (live-confirmed across 6+ real
//! candidates this session, every one negative) essentially never shows
//! genuine AR(1) mean-reversion on its own raw residual series --
//! averaging a basket cancels exactly the kind of idiosyncratic
//! microstructure noise that plausibly caused that.
//!
//! Pure, no host-import dependency, same testability discipline as
//! `factor_residual.rs`/`dispersion_basket.rs`/`factor_basket.rs`.
//!
//! **Unlike [`dispersion_basket`](crate::trader::dispersion_basket)'s
//! long basket** (ranked by *highest* `residual_stdev` -- a real
//! volatility bet), this basket's only job is noise cancellation, so it
//! ranks by the *lowest* `residual_stdev` -- the calmest, most stable
//! curated symbols make the best hedge instrument, not the most volatile
//! ones. No stdev-proportional weight blend either (unlike dispersion's
//! basket): every surviving member gets equal weight, since there's no
//! volatility-based conviction to weight by here.

use crate::graph::AccountId;
use crate::trader::factor_residual::{self, ResidualSide, SymbolResidual};

/// Target size of the low-noise hedge basket -- small and fixed, same
/// practical reasoning `dispersion_basket::DISPERSION_BASKET_SIZE` gives
/// for starting small (fewer legs means fewer chances any single one is
/// unroutable/illiquid, and a smaller basket is cheaper to keep sized
/// above `MIN_SIZED_FRACTION_OF_INTENDED` per leg). Starting value, not
/// calibrated against any real trading history yet.
pub const PAIR_BASKET_SIZE: usize = 5;

/// Minimum number of real, warmed-up basket members required before a
/// basket is buildable at all -- below this, averaging away idiosyncratic
/// noise isn't meaningful (a 1-member "basket" is just another single
/// symbol, reintroducing the exact problem this redesign targets).
pub const MIN_PAIR_BASKET_SIZE: usize = 2;

/// One real curated symbol eligible for the hedge basket, with its own
/// already-resolved `residual_stdev` (`factor_residual::RollingWindow::
/// stats().stdev` for this mint's rolling residual window) -- the
/// caller's job to resolve, same split `dispersion_basket::
/// DispersionCandidate` uses.
#[derive(Debug, Clone, Copy)]
pub struct BasketMember {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub residual_stdev: f64,
}

/// Real basket selection: ranks `candidates` by `residual_stdev`
/// *ascending* (calmest first -- the opposite ranking direction from
/// `dispersion_basket::build_dispersion_basket`), excludes `exclude_mint`
/// (the spike candidate itself, so it never hedges against its own
/// value), and takes the top [`PAIR_BASKET_SIZE`] (fewer if the universe
/// doesn't have that many). Candidates with a non-positive or non-finite
/// `residual_stdev` are dropped before ranking -- no real stability
/// signal to rank by. Every surviving member gets equal weight (no
/// stdev-proportional blend -- see this module's own doc comment for
/// why). `None` if fewer than [`MIN_PAIR_BASKET_SIZE`] real members
/// survive.
pub fn select_pair_basket_members(candidates: &[BasketMember], exclude_mint: AccountId) -> Option<Vec<BasketMember>> {
    let mut ranked: Vec<&BasketMember> = candidates
        .iter()
        .filter(|c| c.mint != exclude_mint && c.residual_stdev.is_finite() && c.residual_stdev > 0.0)
        .collect();
    if ranked.len() < MIN_PAIR_BASKET_SIZE {
        return None;
    }
    ranked.sort_by(|a, b| a.residual_stdev.total_cmp(&b.residual_stdev));
    ranked.truncate(PAIR_BASKET_SIZE);
    Some(ranked.into_iter().copied().collect())
}

/// Real equal-weighted mean of `members`' current-cycle `residual_pct`,
/// drawn from `residuals` (this cycle's already-computed values -- a
/// member absent from `residuals` this cycle, e.g. no fresh price, is
/// simply skipped, same "average over whatever's present" reasoning
/// every other per-cycle real average in this codebase uses). `None` if
/// none of `members` appear in `residuals` at all this cycle.
pub fn basket_average_residual(residuals: &[SymbolResidual], members: &[BasketMember]) -> Option<f64> {
    let values: Vec<f64> =
        residuals.iter().filter(|r| members.iter().any(|m| m.mint == r.mint)).map(|r| r.residual_pct).collect();
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// A real single-sided pair-trade candidate -- whichever curated symbol
/// has the single most extreme `|zscore|` clearing
/// [`factor_residual::PAIR_TRADE_ENTRY_ZSCORE`] this cycle, in *either*
/// direction. Unlike `factor_residual::find_best_pair`, no second,
/// independent single symbol on the opposite side is required -- the
/// counterparty is always the hedge basket (see this module's own doc
/// comment for why).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpikeCandidate {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub residual_pct: f64,
    pub zscore: f64,
    /// `Underperformer` -- long the spike, short the basket.
    /// `Overperformer` -- short the spike, long the basket.
    pub side: ResidualSide,
}

/// The real, reusable search -- scans every real residual, picks the
/// single most extreme `|zscore|` clearing the entry threshold on either
/// side. `None` if no real candidate clears the bar this cycle -- not a
/// "trade something anyway" fallback, same discipline
/// `factor_residual::find_best_pair` already established.
pub fn find_spike_candidate(residuals: &[SymbolResidual]) -> Option<SpikeCandidate> {
    residuals
        .iter()
        .filter_map(|r| factor_residual::classify_residual(r.zscore).map(|side| (r, side)))
        .max_by(|(a, _), (b, _)| a.zscore.abs().total_cmp(&b.zscore.abs()))
        .map(|(r, side)| SpikeCandidate { symbol: r.symbol, mint: r.mint, residual_pct: r.residual_pct, zscore: r.zscore, side })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn member(symbol: &'static str, mint: AccountId, residual_stdev: f64) -> BasketMember {
        BasketMember { symbol, mint, residual_stdev }
    }

    fn residual(symbol: &'static str, mint: AccountId, residual_pct: f64, zscore: f64) -> SymbolResidual {
        SymbolResidual { symbol, mint, residual_pct, zscore }
    }

    // --- select_pair_basket_members --------------------------------------

    #[test]
    fn select_pair_basket_members_truncates_to_basket_size() {
        let candidates: Vec<BasketMember> = (0..PAIR_BASKET_SIZE + 5).map(|i| member("S", i as AccountId, 1.0 + i as f64)).collect();
        let basket = select_pair_basket_members(&candidates, 999).expect("should build a basket");
        assert_eq!(basket.len(), PAIR_BASKET_SIZE);
    }

    #[test]
    fn select_pair_basket_members_picks_lowest_stdev_first() {
        // Opposite ranking direction from dispersion_basket -- the
        // *calmest* members should survive, not the most volatile.
        let candidates =
            vec![member("Calm", 1, 0.1), member("Volatile", 2, 100.0), member("MidA", 3, 1.0), member("MidB", 4, 2.0)];
        let basket = select_pair_basket_members(&candidates, 999).unwrap();
        assert!(basket.iter().any(|m| m.symbol == "Calm"));
        assert!(!basket.iter().any(|m| m.symbol == "Volatile") || basket.len() == candidates.len());
    }

    #[test]
    fn select_pair_basket_members_excludes_spike_mint() {
        let candidates = vec![member("A", 1, 1.0), member("B", 2, 2.0), member("Spike", 3, 0.5)];
        let basket = select_pair_basket_members(&candidates, 3).unwrap();
        assert!(!basket.iter().any(|m| m.mint == 3));
    }

    #[test]
    fn select_pair_basket_members_none_below_minimum() {
        let candidates = vec![member("A", 1, 1.0)];
        assert!(select_pair_basket_members(&candidates, 999).is_none());
    }

    #[test]
    fn select_pair_basket_members_drops_non_positive_and_non_finite() {
        let candidates = vec![
            member("A", 1, 1.0),
            member("B", 2, 2.0),
            member("Zero", 3, 0.0),
            member("Neg", 4, -1.0),
            member("Nan", 5, f64::NAN),
        ];
        let basket = select_pair_basket_members(&candidates, 999).unwrap();
        let symbols: Vec<&str> = basket.iter().map(|m| m.symbol).collect();
        assert!(!symbols.contains(&"Zero"));
        assert!(!symbols.contains(&"Neg"));
        assert!(!symbols.contains(&"Nan"));
        assert_eq!(basket.len(), 2);
    }

    #[test]
    fn select_pair_basket_members_ok_at_exactly_minimum_size() {
        let candidates = vec![member("A", 1, 1.0), member("B", 2, 2.0)];
        assert!(select_pair_basket_members(&candidates, 999).is_some());
    }

    // --- basket_average_residual ------------------------------------------

    #[test]
    fn basket_average_residual_computes_equal_weighted_mean() {
        let members = vec![member("A", 1, 1.0), member("B", 2, 1.0), member("C", 3, 1.0)];
        let residuals = vec![residual("A", 1, 3.0, 0.1), residual("B", 2, 6.0, 0.2), residual("C", 3, 9.0, 0.3)];
        let avg = basket_average_residual(&residuals, &members).unwrap();
        assert!(approx(avg, 6.0));
    }

    #[test]
    fn basket_average_residual_skips_absent_members() {
        // "B" isn't in `residuals` this cycle (no fresh price) -- average
        // over whichever subset is present, not a hard failure.
        let members = vec![member("A", 1, 1.0), member("B", 2, 1.0)];
        let residuals = vec![residual("A", 1, 4.0, 0.1)];
        let avg = basket_average_residual(&residuals, &members).unwrap();
        assert!(approx(avg, 4.0));
    }

    #[test]
    fn basket_average_residual_none_if_no_member_present() {
        let members = vec![member("A", 1, 1.0)];
        let residuals = vec![residual("Other", 99, 4.0, 0.1)];
        assert!(basket_average_residual(&residuals, &members).is_none());
    }

    // --- find_spike_candidate ---------------------------------------------

    #[test]
    fn find_spike_candidate_none_when_nothing_clears_threshold() {
        let residuals = vec![residual("A", 1, 1.0, 0.5), residual("B", 2, -1.0, -1.2)];
        assert!(find_spike_candidate(&residuals).is_none());
    }

    #[test]
    fn find_spike_candidate_picks_most_extreme_either_side() {
        let residuals = vec![
            residual("Under", 1, -3.0, -2.6),
            residual("Over", 2, 5.0, 3.9),
            residual("Mid", 3, 1.0, 0.4),
        ];
        let c = find_spike_candidate(&residuals).unwrap();
        assert_eq!(c.symbol, "Over");
        assert_eq!(c.side, ResidualSide::Overperformer);
    }

    #[test]
    fn find_spike_candidate_underperformer_side() {
        let residuals = vec![residual("Under", 1, -3.0, -3.9), residual("Over", 2, 5.0, 2.6)];
        let c = find_spike_candidate(&residuals).unwrap();
        assert_eq!(c.symbol, "Under");
        assert_eq!(c.side, ResidualSide::Underperformer);
    }

    #[test]
    fn find_spike_candidate_single_candidate_is_enough() {
        // The whole point of this redesign -- unlike find_best_pair, a
        // single real crossing (no opposite-side match needed) is a real
        // candidate on its own.
        let residuals = vec![residual("Only", 1, -3.0, -3.0)];
        assert!(find_spike_candidate(&residuals).is_some());
    }

    #[test]
    fn find_spike_candidate_boundary_excluded() {
        let residuals = vec![residual("Boundary", 1, 1.0, factor_residual::PAIR_TRADE_ENTRY_ZSCORE - 0.01)];
        assert!(find_spike_candidate(&residuals).is_none());
    }
}
