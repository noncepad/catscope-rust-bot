//! Real discrete-time self-exciting ("Hawkes") momentum trade on a shared
//! eigenfactor -- see `docs/HAWKES_FACTOR_TRADE_PLAN.md` for the full
//! design this implements. Trade type 5, separate from pair/directional/
//! dispersion/arbitrage.
//!
//! **Every other trade type in this codebase bets on reversion** -- pair
//! trades a spike back toward its basket, directional/dispersion bet a
//! factor-loading position back toward zero. This one bets the opposite:
//! that a cluster of jumps on names sharing an eigenvector predicts *more*
//! jumps in the same direction, for a short window, before it decays. It
//! targets illiquid tokens specifically -- a single illiquid token's jump
//! is noise, but jumps pooled across many names sharing a factor
//! (loading-weighted) are a higher-SNR signal, the same reasoning that
//! already justifies basket sizing over single-leg sizing elsewhere in
//! this codebase (`pair_basket`, `dispersion_basket`, `factor_basket`).
//!
//! **Discrete-time, not textbook continuous Hawkes.** Textbook Hawkes
//! needs a real point-process event log (exact jump timestamps) and MLE
//! fitting. This bot's factor resync cadence is already effectively
//! fixed-interval (live-observed ~30s between cycles), so this models a
//! discrete-time self-exciting intensity instead -- a Poisson-
//! autoregression / INGARCH-style recursion, the standard discretization
//! of Hawkes when observations arrive on a regular grid:
//! `λ_t = μ + α·λ_{t-1} + β·event_magnitude_{t-1}`. Updates in O(1) per
//! cycle -- no event-history buffer needed, which matters because the
//! whole motivation here is illiquid names with sparse real data.
//!
//! Pure, no host-import dependency, same testability discipline as
//! `factor_residual.rs`/`pair_basket.rs`/`dispersion_basket.rs`/
//! `factor_basket.rs`.

use crate::graph::AccountId;

/// State for one factor's discrete-time self-exciting jump intensity.
/// Fields are public so the caller (`state.rs`) can persist this verbatim
/// across resync cycles as part of `State`, same shape as
/// `factor_residual::RollingWindow`'s own public fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactorIntensityState {
    /// Real baseline/drift term of the recursion -- *not* itself the
    /// long-run steady-state intensity with no recent excitation (that's
    /// `mu / (1.0 - alpha)`, standard for this kind of AR(1)-shaped
    /// recursion); `mu` is the constant term the recursion mean-reverts
    /// around, one `alpha` factor removed from the actual asymptote.
    pub mu: f64,
    /// Persistence coefficient -- how much of last cycle's elevated
    /// intensity carries forward. Must be in `[0.0, 1.0)` for the
    /// recursion to be stationary (mean-reverting back to `mu / (1.0 -
    /// alpha)`); this module does not enforce that on construction (the
    /// caller owns calibration), only [`update_intensity`]'s own behavior
    /// depends on it being sane.
    pub alpha: f64,
    /// Excitation coefficient -- how strongly an observed jump this cycle
    /// raises next cycle's intensity.
    pub beta: f64,
    /// Real current recursed intensity -- `mu` before any real jump data
    /// has been observed.
    pub lambda: f64,
}

/// Starting `alpha` -- a short half-life (~1 cycle) so the self-exciting
/// effect is conservative by default: it fades fast rather than keeping a
/// position open indefinitely on stale excitation. Not calibrated against
/// any real Hawkes MLE fit yet -- same "start conservative, tune from
/// live-observed data" discipline `factor_residual::PAIR_TRADE_ENTRY_ZSCORE`
/// and `factor_basket::MIN_TOTAL_FACTOR_LOADING` both started from. See
/// `docs/HAWKES_FACTOR_TRADE_PLAN.md`'s open questions.
pub const DEFAULT_ALPHA: f64 = 0.5;

/// Starting `beta` -- how strongly one cycle's real jump magnitude
/// (already loading-weighted, see [`factor_jump_magnitude`]) excites next
/// cycle's intensity. Same "starting value, not calibrated yet" caveat as
/// [`DEFAULT_ALPHA`].
pub const DEFAULT_BETA: f64 = 0.5;

impl FactorIntensityState {
    /// Real starting state before any jump-event data exists for this
    /// factor this run -- `lambda` starts at `mu` itself as a conservative
    /// cold-start value (not yet the recursion's true unexcited steady
    /// state of `mu / (1.0 - alpha)`; it converges there after enough
    /// zero-event cycles, same "start conservative" reasoning as
    /// [`DEFAULT_ALPHA`]/[`DEFAULT_BETA`]), `alpha`/`beta` at their
    /// documented starting placeholders.
    pub fn starting(mu: f64) -> Self {
        Self { mu, alpha: DEFAULT_ALPHA, beta: DEFAULT_BETA, lambda: mu }
    }
}

/// Real discrete-time self-exciting intensity update -- one Hawkes-style
/// recursion step per resync cycle:
/// `λ_t = μ + α·λ_{t-1} + β·event_magnitude_{t-1}`
/// (see this module's own doc comment for why this discretization, not
/// continuous-time Hawkes). `event_magnitude` is this cycle's real jump
/// magnitude for the factor (see [`factor_jump_magnitude`]) -- always
/// `>= 0.0`, the caller's job to guarantee (debug-asserted here, not
/// silently clamped, since a negative magnitude would mean a real bug in
/// the caller's projection, not a legitimate input). Returns a new
/// `FactorIntensityState` with the same `mu`/`alpha`/`beta` (calibration
/// parameters don't change here) and the newly recursed `lambda`.
pub fn update_intensity(state: FactorIntensityState, event_magnitude: f64) -> FactorIntensityState {
    debug_assert!(event_magnitude >= 0.0, "event_magnitude must be non-negative, got {event_magnitude}");
    let lambda = state.mu + state.alpha * state.lambda + state.beta * event_magnitude;
    FactorIntensityState { lambda, ..state }
}

/// One real curated symbol's factor loading plus this cycle's raw z-score
/// delta -- caller's job to compute `delta_zscore` as `current_zscore -
/// previous_zscore` (see `docs/HAWKES_FACTOR_TRADE_PLAN.md`), same split
/// `pair_basket::BasketMember` uses (caller resolves the real numbers,
/// this module only reduces over them).
#[derive(Debug, Clone, Copy)]
pub struct SymbolJump {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub loading: f64,
    pub delta_zscore: f64,
}

/// Real per-cycle factor-level jump magnitude -- `Σ |loading_i ·
/// delta_zscore_i|` across every real symbol this cycle. Loading-weighted
/// pooling is what rescues signal on illiquid single names (see this
/// module's own doc comment). `0.0` (not `None`) when `jumps` is empty --
/// a quiet cycle is a real, valid zero-magnitude observation for the
/// intensity recursion, not a missing-data case.
pub fn factor_jump_magnitude(jumps: &[SymbolJump]) -> f64 {
    jumps.iter().map(|j| (j.loading * j.delta_zscore).abs()).sum()
}

/// How many σ above baseline `mu` the recursed intensity must clear before
/// a real momentum position opens -- same z-scored-gate-on-a-rolling-stat
/// shape as `factor_residual::PAIR_TRADE_ENTRY_ZSCORE`/`factor_basket::
/// MIN_TOTAL_FACTOR_LOADING`, not a hardcoded absolute intensity value
/// (which would have no comparable meaning across factors with different
/// baseline jumpiness). Starting value, not calibrated against any real
/// trading history yet -- see `docs/HAWKES_FACTOR_TRADE_PLAN.md`'s open
/// questions.
pub const HAWKES_ENTRY_SIGMA: f64 = 2.5;

/// How many σ above baseline `mu` the intensity must decay back below
/// before a real open position is closed on decay (see
/// [`should_close_hawkes_on_decay`]). Deliberately *tighter* than
/// [`HAWKES_ENTRY_SIGMA`] so a position doesn't flap open/closed right at
/// the entry boundary.
pub const HAWKES_EXIT_SIGMA: f64 = 1.0;

/// Minimum real `sigma_lambda` for it to count as a measurable
/// distribution -- live-observed (2026-09-06/07) a factor with genuinely
/// zero jump activity its entire history still produces a nonzero rolling
/// stdev on the order of `1e-18` (floating-point residue around the true
/// mathematical zero, not a real measured spread), which is `> 0.0` and
/// finite, so it used to slip past the old `sigma_lambda > 0.0` check --
/// any infinitesimal floating-point residue in `lambda` then divided by
/// that near-zero denominator and spuriously cleared
/// [`HAWKES_ENTRY_SIGMA`], with no real signal behind it. `1e-6` is well
/// above any float-noise floor and well below any real, meaningfully-
/// measured jump-magnitude spread this bot has live-observed.
pub const HAWKES_MIN_SIGMA_LAMBDA: f64 = 1e-6;

/// Target size of the momentum basket -- same small/fixed starting
/// reasoning `pair_basket::PAIR_BASKET_SIZE`/`dispersion_basket::
/// DISPERSION_BASKET_SIZE` both give (fewer legs means fewer chances any
/// single one is unroutable/illiquid, cheaper to keep each leg above the
/// caller's own sized-fraction floor). Starting value, not calibrated
/// against any real trading history yet.
pub const HAWKES_BASKET_SIZE: usize = 5;

/// Real entry gate -- `true` only when `lambda` has cleared
/// [`HAWKES_ENTRY_SIGMA`] standard deviations above `mu`, using the
/// caller's own rolling stdev of `lambda` (`sigma_lambda` -- the caller,
/// `state.rs`, is expected to track this the same way
/// `factor_residual::RollingWindow` tracks each symbol's own rolling
/// stats). `false` whenever `sigma_lambda` isn't yet a real, positive,
/// finite number (not enough warm-up data -- same "refuse on an unknown
/// distribution" discipline every other entry gate in this codebase
/// uses, e.g. pair trading's half-life warm-up check).
pub fn should_open_hawkes(lambda: f64, mu: f64, sigma_lambda: f64) -> bool {
    sigma_lambda.is_finite()
        && sigma_lambda > HAWKES_MIN_SIGMA_LAMBDA
        && lambda > mu + HAWKES_ENTRY_SIGMA * sigma_lambda
}

/// Real decay-based exit gate -- `true` once `lambda` has decayed back to
/// within [`HAWKES_EXIT_SIGMA`] standard deviations of `mu` (the self-
/// exciting effect that justified the momentum bet has worn off). Also
/// `true` whenever `sigma_lambda` isn't a real, positive, finite number --
/// favor closing over holding a position open on a distribution we can no
/// longer measure.
pub fn should_close_hawkes_on_decay(lambda: f64, mu: f64, sigma_lambda: f64) -> bool {
    !(sigma_lambda.is_finite() && sigma_lambda > HAWKES_MIN_SIGMA_LAMBDA)
        || lambda <= mu + HAWKES_EXIT_SIGMA * sigma_lambda
}

/// Long or short a single real momentum-basket leg -- same shape as
/// `spfa::PositionSide`, kept as its own type here rather than reused
/// since this module has no dependency on `spfa` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MomentumSide {
    Long,
    Short,
}

/// One real leg of a factor-momentum basket -- `weight_fraction` is this
/// leg's share (0.0-1.0, every leg in a basket sums to 1.0) of the trade's
/// total notional, not a raw amount, same convention
/// `factor_basket::DirectionalBasketLeg` uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactorMomentumLeg {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub weight_fraction: f64,
    pub side: MomentumSide,
}

/// Real momentum-basket construction for a triggered factor -- ranks
/// `jumps` by `|loading|` *descending* (opposite ranking direction from
/// `pair_basket::select_pair_basket_members`'s calmest-first ranking:
/// this basket's thesis is momentum continuation, so it wants the names
/// most responsible for the jump, not the calmest ones -- same "highest,
/// not lowest" ranking direction `dispersion_basket::
/// build_dispersion_basket` already uses for its own volatility bet).
/// Keeps at most `basket_size` legs.
///
/// The factor's overall direction is the *signed* `Σ loading·delta_zscore`
/// across ALL real `jumps` this cycle (not just the chosen legs -- the
/// direction is a property of the whole factor, decided before truncating
/// to the basket). Each leg's `side` is `sign(loading) · sign(factor_
/// direction)` -- go *with* the factor's own signed move, not against it
/// (this is a momentum bet, the opposite of every other trade type in
/// this codebase). Weights are `|loading|`-proportional among the chosen
/// legs, renormalized to sum to 1.0.
///
/// `None` if `jumps` is empty, if the factor's overall signed direction is
/// exactly zero (no real signal to bet with), or if every candidate's own
/// loading is ~0 (nothing real to build a basket from).
pub fn factor_jump_basket(jumps: &[SymbolJump], basket_size: usize) -> Option<Vec<FactorMomentumLeg>> {
    if jumps.is_empty() {
        return None;
    }
    let signed_sum: f64 = jumps.iter().map(|j| j.loading * j.delta_zscore).sum();
    if signed_sum == 0.0 {
        return None;
    }
    let factor_direction = signed_sum.signum();

    let mut ranked: Vec<&SymbolJump> = jumps.iter().filter(|j| j.loading.abs() > 1e-9).collect();
    if ranked.is_empty() {
        return None;
    }
    ranked.sort_by(|a, b| b.loading.abs().total_cmp(&a.loading.abs()));
    ranked.truncate(basket_size);

    let total_abs_loading: f64 = ranked.iter().map(|j| j.loading.abs()).sum();
    if total_abs_loading <= 0.0 {
        return None;
    }

    Some(
        ranked
            .into_iter()
            .map(|j| {
                let side =
                    if j.loading.signum() * factor_direction >= 0.0 { MomentumSide::Long } else { MomentumSide::Short };
                FactorMomentumLeg {
                    symbol: j.symbol,
                    mint: j.mint,
                    weight_fraction: j.loading.abs() / total_abs_loading,
                    side,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn jump(symbol: &'static str, mint: AccountId, loading: f64, delta_zscore: f64) -> SymbolJump {
        SymbolJump { symbol, mint, loading, delta_zscore }
    }

    // --- update_intensity --------------------------------------------------

    #[test]
    fn update_intensity_no_event_decays_toward_mu() {
        let state = FactorIntensityState { mu: 1.0, alpha: 0.5, beta: 0.5, lambda: 5.0 };
        let next = update_intensity(state, 0.0);
        // 1.0 + 0.5*5.0 + 0.5*0.0 = 3.5 -- moving down from 5.0 toward mu.
        assert!(approx(next.lambda, 3.5));
        assert!(next.lambda < state.lambda);
    }

    #[test]
    fn update_intensity_real_jump_raises_next_lambda() {
        let state = FactorIntensityState::starting(1.0);
        let next = update_intensity(state, 4.0);
        // 1.0 + 0.5*1.0 + 0.5*4.0 = 3.5
        assert!(approx(next.lambda, 3.5));
        assert!(next.lambda > state.lambda);
    }

    #[test]
    fn update_intensity_preserves_calibration_params() {
        let state = FactorIntensityState { mu: 2.0, alpha: 0.3, beta: 0.7, lambda: 2.0 };
        let next = update_intensity(state, 1.0);
        assert!(approx(next.mu, 2.0));
        assert!(approx(next.alpha, 0.3));
        assert!(approx(next.beta, 0.7));
    }

    #[test]
    fn update_intensity_repeated_zero_events_converges_to_steady_state() {
        // With no further events, the recursion's true fixed point is
        // mu / (1.0 - alpha), not mu itself -- standard for this AR(1)-
        // shaped recursion (see FactorIntensityState::mu's own doc
        // comment).
        let mut state = FactorIntensityState::starting(1.0);
        state.lambda = 100.0;
        for _ in 0..200 {
            state = update_intensity(state, 0.0);
        }
        let steady_state = state.mu / (1.0 - state.alpha);
        assert!(
            (state.lambda - steady_state).abs() < 1e-6,
            "expected convergence to mu/(1-alpha)={steady_state}, got {}",
            state.lambda
        );
    }

    // --- factor_jump_magnitude ----------------------------------------------

    #[test]
    fn factor_jump_magnitude_empty_is_zero() {
        assert!(approx(factor_jump_magnitude(&[]), 0.0));
    }

    #[test]
    fn factor_jump_magnitude_sums_absolute_loading_weighted_moves() {
        let jumps = vec![jump("A", 1, 0.5, 2.0), jump("B", 2, -0.5, -1.0)];
        // |0.5*2.0| + |-0.5*-1.0| = 1.0 + 0.5 = 1.5
        assert!(approx(factor_jump_magnitude(&jumps), 1.5));
    }

    #[test]
    fn factor_jump_magnitude_never_negative() {
        let jumps = vec![jump("A", 1, -0.9, 3.0)];
        assert!(factor_jump_magnitude(&jumps) >= 0.0);
    }

    // --- should_open_hawkes / should_close_hawkes_on_decay -------------------

    #[test]
    fn should_open_hawkes_true_above_entry_sigma() {
        assert!(should_open_hawkes(10.0, 1.0, 1.0));
    }

    #[test]
    fn should_open_hawkes_false_at_or_below_entry_sigma() {
        let boundary = 1.0 + HAWKES_ENTRY_SIGMA * 1.0;
        assert!(!should_open_hawkes(boundary, 1.0, 1.0));
    }

    #[test]
    fn should_open_hawkes_false_without_real_sigma() {
        assert!(!should_open_hawkes(100.0, 1.0, 0.0));
        assert!(!should_open_hawkes(100.0, 1.0, -1.0));
        assert!(!should_open_hawkes(100.0, 1.0, f64::NAN));
    }

    /// Live-observed 2026-09-06/07: a factor with zero real jump history
    /// its entire run still produced `sigma_lambda` on the order of
    /// `1e-18` (floating-point residue around the true mathematical zero,
    /// not a real measured spread) -- `> 0.0` and finite, so it used to
    /// slip past the old check, and any infinitesimal float residue in
    /// `lambda` then spuriously cleared the entry gate against that
    /// near-zero denominator. `HAWKES_MIN_SIGMA_LAMBDA` must reject this.
    #[test]
    fn should_open_hawkes_false_on_float_noise_sigma() {
        assert!(!should_open_hawkes(1e-15, 0.0, 1.4e-18));
    }

    #[test]
    fn should_open_hawkes_true_with_sigma_just_above_the_noise_floor() {
        let sigma = HAWKES_MIN_SIGMA_LAMBDA * 2.0;
        assert!(should_open_hawkes(1.0 + HAWKES_ENTRY_SIGMA * sigma * 2.0, 1.0, sigma));
    }

    #[test]
    fn should_close_hawkes_on_decay_true_once_back_within_band() {
        assert!(should_close_hawkes_on_decay(1.5, 1.0, 1.0));
    }

    /// Symmetric floor check for the close gate -- a float-noise
    /// `sigma_lambda` must count as "no real distribution to measure" and
    /// favor closing, same as the `None`/non-positive cases already do.
    #[test]
    fn should_close_hawkes_on_decay_true_on_float_noise_sigma() {
        assert!(should_close_hawkes_on_decay(1e-15, 0.0, 1.4e-18));
    }

    #[test]
    fn should_close_hawkes_on_decay_false_while_still_elevated() {
        assert!(!should_close_hawkes_on_decay(10.0, 1.0, 1.0));
    }

    #[test]
    fn should_close_hawkes_on_decay_true_without_real_sigma() {
        assert!(should_close_hawkes_on_decay(10.0, 1.0, 0.0));
        assert!(should_close_hawkes_on_decay(10.0, 1.0, f64::NAN));
    }

    #[test]
    fn exit_sigma_is_tighter_than_entry_sigma() {
        assert!(HAWKES_EXIT_SIGMA < HAWKES_ENTRY_SIGMA);
    }

    // --- factor_jump_basket --------------------------------------------------

    #[test]
    fn factor_jump_basket_none_when_empty() {
        assert!(factor_jump_basket(&[], 5).is_none());
    }

    #[test]
    fn factor_jump_basket_none_when_direction_is_exactly_zero() {
        // +0.5*1.0 and -0.5*1.0 cancel exactly -- no real factor direction.
        let jumps = vec![jump("A", 1, 0.5, 1.0), jump("B", 2, -0.5, 1.0)];
        assert!(factor_jump_basket(&jumps, 5).is_none());
    }

    #[test]
    fn factor_jump_basket_none_when_all_loadings_are_zero() {
        let jumps = vec![jump("A", 1, 0.0, 3.0), jump("B", 2, 0.0, -3.0)];
        assert!(factor_jump_basket(&jumps, 5).is_none());
    }

    #[test]
    fn factor_jump_basket_ranks_highest_loading_first_and_truncates() {
        let jumps = vec![jump("Weak", 1, 0.1, 1.0), jump("Strong", 2, 0.9, 1.0), jump("Mid", 3, 0.5, 1.0)];
        let basket = factor_jump_basket(&jumps, 2).expect("should build a basket");
        assert_eq!(basket.len(), 2);
        assert!(basket.iter().any(|l| l.symbol == "Strong"));
        assert!(basket.iter().any(|l| l.symbol == "Mid"));
        assert!(!basket.iter().any(|l| l.symbol == "Weak"));
    }

    #[test]
    fn factor_jump_basket_weights_sum_to_one() {
        let jumps = vec![jump("A", 1, 0.3, 1.0), jump("B", 2, 0.6, 1.0), jump("C", 3, 0.1, 1.0)];
        let basket = factor_jump_basket(&jumps, 5).unwrap();
        let total: f64 = basket.iter().map(|l| l.weight_fraction).sum();
        assert!(approx(total, 1.0), "weights should sum to 1.0, got {total}");
    }

    #[test]
    fn factor_jump_basket_positive_loading_matches_factor_direction_goes_long() {
        // Factor direction is positive (0.5*2.0 dominates over -0.1*1.0).
        let jumps = vec![jump("Pos", 1, 0.5, 2.0), jump("SmallNeg", 2, -0.1, 1.0)];
        let basket = factor_jump_basket(&jumps, 5).unwrap();
        let pos = basket.iter().find(|l| l.symbol == "Pos").unwrap();
        assert_eq!(pos.side, MomentumSide::Long);
    }

    #[test]
    fn factor_jump_basket_negative_loading_against_factor_direction_goes_short() {
        // Same setup: factor direction positive, but this leg has negative
        // loading, so it should be shorted (moves opposite the factor).
        let jumps = vec![jump("Pos", 1, 0.5, 2.0), jump("Neg", 2, -0.5, 0.1)];
        let basket = factor_jump_basket(&jumps, 5).unwrap();
        let neg = basket.iter().find(|l| l.symbol == "Neg").unwrap();
        assert_eq!(neg.side, MomentumSide::Short);
    }

    #[test]
    fn factor_jump_basket_negative_factor_direction_flips_sides() {
        // Factor direction is negative here (-0.5*2.0 dominates).
        let jumps = vec![jump("A", 1, -0.5, 2.0), jump("B", 2, 0.5, 0.1)];
        let basket = factor_jump_basket(&jumps, 5).unwrap();
        // A has negative loading matching negative factor direction -> long.
        assert_eq!(basket.iter().find(|l| l.symbol == "A").unwrap().side, MomentumSide::Long);
        // B has positive loading against negative factor direction -> short.
        assert_eq!(basket.iter().find(|l| l.symbol == "B").unwrap().side, MomentumSide::Short);
    }

    #[test]
    fn factor_jump_basket_drops_zero_loading_candidates() {
        let jumps = vec![jump("Real", 1, 0.5, 1.0), jump("Zero", 2, 0.0, 5.0)];
        let basket = factor_jump_basket(&jumps, 5).unwrap();
        assert!(!basket.iter().any(|l| l.symbol == "Zero"));
    }
}
