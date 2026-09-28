//! Real hedge-basket construction and stop-loss logic for trade type 1
//! ("directional factor-neutral") of `brain::multimodelv1::PLAN-1.md` --
//! `INSTRUCTIONS.md` §3A's "long a token, short a basket of the factor
//! portfolio proportional to A's factor loadings (β_k)", made concrete
//! and executable. Pure, no host-import dependency, same testability
//! discipline as `factor_residual.rs`.
//!
//! **Entry itself is human-specified, not computed here** -- no
//! backtested basis exists anywhere in this codebase for automatically
//! deciding "is token A a good long," and `factor_residual.rs`'s own
//! z-score machinery is built the opposite way (bets on reversion, not
//! continuation). This module only builds the real hedge basket for a
//! human-given target and provides the automated stop-loss/risk-exit
//! logic -- see `PLAN-1.md`'s directional-neutral design notes for the
//! full reasoning.
//!
//! **A true per-factor "factor portfolio" would short every curated
//! token weighted by its own loading on that factor** -- real, but
//! impractical to execute (100+ curated mints, most legs reduced to
//! dust, fee-dominated). This module uses a standard, defensible
//! factor-mimicking-portfolio truncation instead: for each of the
//! leading `k` factors, proxy that factor with the single real candidate
//! carrying the largest `|loading|` on it -- the "purest" available
//! representative -- keeping the basket to at most `k` legs.

use crate::graph::AccountId;

/// One curated symbol eligible to serve as a hedge-basket proxy --
/// callers build this from `curated_symbols()`'s real, build-time
/// -generated list plus that symbol's index into `factor_graph::
/// StructuralFactors::eigenvectors` (the same stable per-cycle token
/// ordering `factor_residual::compute_residuals` already relies on).
/// Must already exclude the trade's own long target and USDC (the
/// caller's settlement currency, not a real hedge instrument) before
/// being passed to [`build_directional_basket`].
#[derive(Debug, Clone, Copy)]
pub struct CandidateToken {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub token_index: usize,
}

/// One real short leg of a directional-neutral hedge basket --
/// `weight_fraction` is this leg's share (0.0-1.0, every leg in a
/// basket sums to 1.0) of the trade's total short notional, not a raw
/// amount; the caller (`state.rs`) converts it to a real raw-unit
/// notional against its own intended total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalBasketLeg {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub weight_fraction: f64,
}

/// Minimum total (sum of `|loading|` across all `k` factors) a target
/// token needs before a real hedge basket is buildable at all -- below
/// this, treat the token as structurally isolated in the real liquidity
/// graph (its factor loadings are numerical noise, not a real signal to
/// size a basket against).
///
/// Loosened from `0.05` on 2026-09-07 after live data showed the old bar
/// was almost never cleared: across 230 real directional resync cycles
/// targeting ETH (`live16`/`live17`), mean total loading was `0.0126` and
/// only 4 cycles (1.7%) ever reached `0.05`. The real distribution was
/// 47% exactly `0.0` (genuinely stale/missing factor data that cycle, not
/// a real signal either bar would rescue), 30% in `(0.0, 0.02)`, and 20%
/// in `(0.02, 0.05)` -- `0.02` keeps the "exactly zero" bucket excluded
/// (still refuses on truly missing data) while admitting the next-lowest
/// real bucket, an experiment to see whether that band produces real,
/// tradeable baskets or just fails one step later (borrow APY/router).
pub const MIN_TOTAL_FACTOR_LOADING: f64 = 0.02;

/// Real factor-mimicking-portfolio construction. `eigenvectors`/
/// `target_token_index` follow `factor_graph::StructuralFactors`'s own
/// `[token][factor]` indexing and ascending-eigenvalue convention
/// exactly (the same one `factor_residual::compute_residuals` already
/// relies on) -- `k` should be the caller's own `RESIDUAL_FACTOR_COUNT`,
/// not a new number, for consistency with the residual/z-score machinery
/// computed over the same factors. `candidates` must already exclude the
/// target token and USDC.
///
/// Returns `None` if the target's own total loading across all `k`
/// factors is below [`MIN_TOTAL_FACTOR_LOADING`] (no real hedge basket
/// exists for a structurally isolated token), or if `candidates` is
/// empty, or if every factor's best real candidate loading is itself
/// ~0 (nothing real to hedge with at all). A single factor whose best
/// real candidate loading is ~0 is skipped individually (not forced into
/// the basket) rather than failing the whole basket, and the surviving
/// legs' weights are renormalized to sum to 1.0. Two factors sharing the
/// same best proxy dedupe into a single leg (summed weight) -- never two
/// borrow instructions against the same reserve.
pub fn build_directional_basket(
    eigenvectors: &[Vec<f64>],
    target_token_index: usize,
    k: usize,
    candidates: &[CandidateToken],
) -> Option<Vec<DirectionalBasketLeg>> {
    if candidates.is_empty() || target_token_index >= eigenvectors.len() {
        return None;
    }
    let target_row = &eigenvectors[target_token_index];
    let n_factors = target_row.len().min(k);
    if n_factors == 0 {
        return None;
    }
    let target_loadings: Vec<f64> = target_row[0..n_factors].to_vec();
    let total_loading: f64 = target_loadings.iter().map(|b| b.abs()).sum();
    if total_loading < MIN_TOTAL_FACTOR_LOADING {
        return None;
    }

    // Per-factor proxy selection: the real candidate with the largest
    // |loading| on that factor. A factor whose best real candidate has
    // ~0 loading is skipped -- nothing real to proxy it with.
    const MIN_PROXY_LOADING: f64 = 1e-9;
    let mut chosen: Vec<(usize, CandidateToken)> = Vec::with_capacity(n_factors);
    for f in 0..n_factors {
        let best = candidates
            .iter()
            .filter(|c| c.token_index < eigenvectors.len())
            .max_by(|a, b| eigenvectors[a.token_index][f].abs().total_cmp(&eigenvectors[b.token_index][f].abs()));
        if let Some(best) = best {
            if eigenvectors[best.token_index][f].abs() >= MIN_PROXY_LOADING {
                chosen.push((f, *best));
            }
        }
    }
    if chosen.is_empty() {
        return None;
    }

    // Weight each surviving factor by the TARGET's own loading (not the
    // proxy's), renormalized across only the factors that survived the
    // skip above.
    let survived_total: f64 = chosen.iter().map(|(f, _)| target_loadings[*f].abs()).sum();
    if survived_total <= 0.0 {
        return None;
    }

    let mut legs: Vec<DirectionalBasketLeg> = Vec::new();
    for (f, proxy) in &chosen {
        let weight = target_loadings[*f].abs() / survived_total;
        if let Some(existing) = legs.iter_mut().find(|l| l.mint == proxy.mint) {
            existing.weight_fraction += weight;
        } else {
            legs.push(DirectionalBasketLeg { symbol: proxy.symbol, mint: proxy.mint, weight_fraction: weight });
        }
    }
    Some(legs)
}

/// Same real "how many σ of idiosyncratic residual movement is a tail
/// event" threshold `factor_residual::PAIR_TRADE_STOP_ZSCORE` already
/// answers against the same real residual distribution
/// (`factor_residual::compute_residuals`, same `RESIDUAL_FACTOR_COUNT`
/// factors) -- reused verbatim rather than a second, uncalibrated number
/// for what's really the same underlying question.
pub const DIRECTIONAL_STOP_ZSCORE: f64 = crate::trader::factor_residual::PAIR_TRADE_STOP_ZSCORE;

/// Real stop-loss check for an open directional-neutral position --
/// deliberately one-sided, unlike `factor_residual::should_close_pair`'s
/// two-sided reversion/stop shape: a large *positive* move in the
/// target's own idiosyncratic residual is the long thesis working, not
/// against it. This trade type has no computed take-profit target (entry
/// is a human directional bet, not a mean-reversion estimate with a real
/// convergence target) -- only a human close trigger or this stop-loss
/// ever closes a real position. `true` only when `target_zscore` has
/// moved sharply negative (against the long).
pub fn should_stop_directional(target_zscore: f64) -> bool {
    target_zscore <= -DIRECTIONAL_STOP_ZSCORE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn cand(symbol: &'static str, mint: AccountId, token_index: usize) -> CandidateToken {
        CandidateToken { symbol, mint, token_index }
    }

    // A real, orthonormal-ish 3-factor synthetic matrix, one row per
    // token: [target, proxyA, proxyB, proxyC, weak].
    // target's loadings: factor0=0.6, factor1=0.3, factor2=0.1
    // proxyA is the purest factor-0 proxy (0.9), proxyB the purest
    // factor-1 proxy (0.8), proxyC the purest factor-2 proxy (0.7).
    fn synthetic_eigenvectors() -> Vec<Vec<f64>> {
        vec![
            vec![0.6, 0.3, 0.1],  // 0: target
            vec![0.9, 0.05, 0.0], // 1: proxyA (best on factor 0)
            vec![0.1, 0.8, 0.0],  // 2: proxyB (best on factor 1)
            vec![0.0, 0.0, 0.7],  // 3: proxyC (best on factor 2)
            vec![0.01, 0.0, 0.0], // 4: weak, never chosen
        ]
    }

    #[test]
    fn build_directional_basket_weights_sum_to_one() {
        let eigenvectors = synthetic_eigenvectors();
        let candidates = [cand("A", 1, 1), cand("B", 2, 2), cand("C", 3, 3), cand("W", 4, 4)];
        let basket = build_directional_basket(&eigenvectors, 0, 3, &candidates).expect("should build a basket");
        assert_eq!(basket.len(), 3);
        let total: f64 = basket.iter().map(|l| l.weight_fraction).sum();
        assert!(approx(total, 1.0), "weights should sum to 1.0, got {total}");
    }

    #[test]
    fn build_directional_basket_picks_the_purest_proxy_per_factor() {
        let eigenvectors = synthetic_eigenvectors();
        let candidates = [cand("A", 1, 1), cand("B", 2, 2), cand("C", 3, 3), cand("W", 4, 4)];
        let basket = build_directional_basket(&eigenvectors, 0, 3, &candidates).expect("should build a basket");
        let symbols: Vec<&str> = basket.iter().map(|l| l.symbol).collect();
        assert!(symbols.contains(&"A"));
        assert!(symbols.contains(&"B"));
        assert!(symbols.contains(&"C"));
        assert!(!symbols.contains(&"W"));
    }

    #[test]
    fn build_directional_basket_weights_by_targets_own_loading() {
        // target's loadings are 0.6/0.3/0.1 -- factor 0's leg should get
        // the largest weight, factor 2's the smallest.
        let eigenvectors = synthetic_eigenvectors();
        let candidates = [cand("A", 1, 1), cand("B", 2, 2), cand("C", 3, 3)];
        let basket = build_directional_basket(&eigenvectors, 0, 3, &candidates).expect("should build a basket");
        let w = |sym: &str| basket.iter().find(|l| l.symbol == sym).unwrap().weight_fraction;
        assert!(w("A") > w("B"));
        assert!(w("B") > w("C"));
        assert!(approx(w("A"), 0.6 / 1.0));
        assert!(approx(w("B"), 0.3 / 1.0));
        assert!(approx(w("C"), 0.1 / 1.0));
    }

    #[test]
    fn build_directional_basket_dedupes_shared_proxy() {
        // proxyA is simultaneously the best proxy for factor 0 AND
        // factor 1 -- must collapse into one leg with the summed weight,
        // not two legs against the same mint.
        let eigenvectors = vec![
            vec![0.6, 0.3], // 0: target
            vec![0.9, 0.9], // 1: proxyA, best on both factors
        ];
        let candidates = [cand("A", 1, 1)];
        let basket = build_directional_basket(&eigenvectors, 0, 2, &candidates).expect("should build a basket");
        assert_eq!(basket.len(), 1);
        assert!(approx(basket[0].weight_fraction, 1.0));
    }

    #[test]
    fn build_directional_basket_none_for_structurally_isolated_target() {
        let eigenvectors = vec![
            vec![0.001, 0.001, 0.001], // 0: target, total loading << MIN_TOTAL_FACTOR_LOADING
            vec![0.9, 0.0, 0.0],
        ];
        let candidates = [cand("A", 1, 1)];
        assert!(build_directional_basket(&eigenvectors, 0, 3, &candidates).is_none());
    }

    #[test]
    fn build_directional_basket_none_for_empty_candidates() {
        let eigenvectors = synthetic_eigenvectors();
        assert!(build_directional_basket(&eigenvectors, 0, 3, &[]).is_none());
    }

    #[test]
    fn build_directional_basket_skips_zero_loading_factor_and_renormalizes() {
        // factor 2 has NO real candidate with nonzero loading -- must be
        // skipped, and the surviving two legs' weights renormalized to
        // sum to 1.0 (not left summing to less than 1.0).
        let eigenvectors = vec![
            vec![0.6, 0.3, 0.1], // 0: target
            vec![0.9, 0.0, 0.0], // 1: proxyA, factor 0
            vec![0.0, 0.8, 0.0], // 2: proxyB, factor 1
            vec![0.0, 0.0, 0.0], // 3: proxyC, zero loading on factor 2 -- skipped
        ];
        let candidates = [cand("A", 1, 1), cand("B", 2, 2), cand("C", 3, 3)];
        let basket = build_directional_basket(&eigenvectors, 0, 3, &candidates).expect("should build a basket");
        assert_eq!(basket.len(), 2);
        let total: f64 = basket.iter().map(|l| l.weight_fraction).sum();
        assert!(approx(total, 1.0), "renormalized weights should still sum to 1.0, got {total}");
        let w = |sym: &str| basket.iter().find(|l| l.symbol == sym).unwrap().weight_fraction;
        assert!(approx(w("A"), 0.6 / 0.9));
        assert!(approx(w("B"), 0.3 / 0.9));
    }

    #[test]
    fn build_directional_basket_none_when_target_index_out_of_range() {
        let eigenvectors = synthetic_eigenvectors();
        let candidates = [cand("A", 1, 1)];
        assert!(build_directional_basket(&eigenvectors, 99, 3, &candidates).is_none());
    }

    // --- should_stop_directional ---------------------------------------

    #[test]
    fn should_stop_directional_boundaries() {
        assert!(should_stop_directional(-DIRECTIONAL_STOP_ZSCORE));
        assert!(!should_stop_directional(-DIRECTIONAL_STOP_ZSCORE + 0.01));
    }

    #[test]
    fn should_stop_directional_never_stops_on_a_large_positive_move() {
        // The thesis working (target overperforming) must never trigger
        // a stop -- only the negative side does.
        assert!(!should_stop_directional(10.0));
        assert!(!should_stop_directional(DIRECTIONAL_STOP_ZSCORE));
    }

    #[test]
    fn should_stop_directional_false_within_normal_range() {
        assert!(!should_stop_directional(-1.0));
        assert!(!should_stop_directional(0.0));
    }
}
