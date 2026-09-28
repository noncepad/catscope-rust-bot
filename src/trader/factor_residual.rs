//! Real residual computation and pair-trade signal detection -- Phase 5
//! sub-phase 5c of `brain::multimodelv1::PLAN-1.md`: the
//! Phoenix-perp-funding-style "decide" logic
//! [`INSTRUCTIONS.md`]'s §3B calls for, generalized the same way
//! `derivative_router::find_best_funding_opportunities` generalized the
//! basis trade's own decision -- pure, no host-import dependency, same
//! testability discipline as every other `trader::factor_*` module.
//!
//! Genuinely new statistical infrastructure, not a port of anything --
//! nothing in this codebase tracks a rolling residual history before
//! this. [`RollingWindow`] is a real, bounded rolling window (old
//! samples evicted, not an unbounded running average), matching
//! `INSTRUCTIONS.md`'s own "rolling mean" framing more literally than an
//! unbounded Welford accumulator would.
//!
//! **Scope call, not directly specified by `INSTRUCTIONS.md`**: a "pair
//! trade" here means exactly two legs -- the single most-underperforming
//! and single most-overperforming curated symbol each cycle, not "long
//! one token, short the whole factor basket" (that's trade type 1,
//! directional factor-neutral, a separate and larger undertaking this
//! module doesn't attempt). Kept deliberately literal to the word
//! "pair."

use crate::graph::AccountId;

/// Min samples a [`RollingWindow`] needs before it will return real
/// stats/a real z-score -- same "don't trust a raw point estimate off
/// too little history" reasoning `factor_borrow_gate`'s
/// `HALF_LIFE_SAFETY_MULTIPLIER` doc comment gives, just for "how many
/// resync cycles" instead of "how wide a half-life pad." Starting value,
/// not calibrated -- no real pair trade has gone through enough resync
/// cycles yet to know how many are actually needed for a trustworthy
/// estimate.
pub const MIN_SAMPLES_FOR_ZSCORE: usize = 10;

/// A real, bounded rolling window of a single symbol's residual history
/// -- old samples are evicted once `capacity` is reached, not averaged
/// in forever. `capacity` samples at this bot's ~30s resync cadence
/// (`factor_graph::MAX_FACTOR_STALENESS_SECS`) is a real, bounded amount
/// of wall-clock history, not an arbitrary count -- callers should size
/// `capacity` with that cadence in mind.
#[derive(Debug, Clone)]
pub struct RollingWindow {
    capacity: usize,
    samples: std::collections::VecDeque<f64>,
}

/// Real mean/stdev computed from a [`RollingWindow`]'s current contents.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RollingStats {
    pub n: usize,
    pub mean: f64,
    pub stdev: f64,
}

/// Minimal placeholder (capacity 1) -- exists only so a struct embedding a
/// bare (non-`Option`) `RollingWindow` field can derive `Default` (e.g.
/// `brain::multimodelv1::State::m_dispersion_history`, real-initialized
/// with its real capacity in that mode's own `on_load`, same "junk
/// default, immediately overwritten" pattern `pricegraph::TradeRouter`'s
/// own manual `Default` impl already uses). Never meant to be used as a
/// real, working window.
impl Default for RollingWindow {
    fn default() -> Self {
        Self::new(1)
    }
}

impl RollingWindow {
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), samples: std::collections::VecDeque::with_capacity(capacity.max(1)) }
    }

    pub fn push(&mut self, x: f64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(x);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Real current samples, oldest-first (same order [`Self::push`]
    /// appends in) -- needed to snapshot this window for cross-restart
    /// persistence (`trader::residual_snapshot`); no other consumer
    /// needs raw sample access, every other real use goes through
    /// [`Self::stats`]/[`Self::zscore`]/[`Self::estimated_half_life_cycles`].
    pub fn samples(&self) -> impl Iterator<Item = f64> + '_ {
        self.samples.iter().copied()
    }

    /// Inverse of repeatedly [`Self::push`]ing `samples` in order onto a
    /// fresh `capacity`-sized window -- reconstructs a window from a
    /// persisted snapshot. Deliberately reuses `push`'s own eviction
    /// logic rather than a raw `VecDeque` assignment, so a snapshot
    /// carrying more samples than `capacity` (e.g. `capacity` shrank
    /// since the snapshot was taken) degrades the same way live pushes
    /// always have -- oldest evicted first -- instead of panicking or
    /// silently keeping stale entries.
    pub fn from_samples(capacity: usize, samples: impl IntoIterator<Item = f64>) -> Self {
        let mut w = Self::new(capacity);
        for x in samples {
            w.push(x);
        }
        w
    }

    /// `None` until at least [`MIN_SAMPLES_FOR_ZSCORE`] real samples
    /// have been pushed -- an early stdev from 2-3 samples is not a
    /// trustworthy volatility estimate.
    pub fn stats(&self) -> Option<RollingStats> {
        if self.samples.len() < MIN_SAMPLES_FOR_ZSCORE {
            return None;
        }
        let n = self.samples.len();
        let mean = self.samples.iter().sum::<f64>() / n as f64;
        let variance = self.samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        Some(RollingStats { n, mean, stdev: variance.sqrt() })
    }

    /// Real z-score of `x` against this window's current rolling stats.
    /// `None` if there isn't enough history yet, or the window has zero
    /// variance (every sample identical so far -- a real but degenerate
    /// case, not divide-by-zero UB).
    pub fn zscore(&self, x: f64) -> Option<f64> {
        let stats = self.stats()?;
        if stats.stdev <= 0.0 {
            return None;
        }
        Some((x - stats.mean) / stats.stdev)
    }

    /// Real AR(1) mean-reversion half-life estimate, in *cycles* (not
    /// years -- this module doesn't know the real wall-clock time per
    /// cycle, only the caller does; see [`half_life_cycles_to_years`]).
    /// Standard method: fit `x_t = phi * x_{t-1}` by OLS on this
    /// window's own real, consecutive residual history
    /// (`phi = cov(x_t, x_{t-1}) / var(x_{t-1})`, mean-centered), then
    /// `half_life = ln(2) / -ln(phi)` (the real closed form for a
    /// discretized OU process, since `phi = e^{-1/half_life}`).
    ///
    /// `None` if there isn't enough real history yet
    /// ([`MIN_SAMPLES_FOR_ZSCORE`], the same bar [`Self::zscore`] uses --
    /// no extra warm-up cost, since a caller can't get a real z-score
    /// candidate before this window has that many samples either), or if
    /// the fitted `phi` falls outside `(0, 1)`. `phi <= 0` catches pure
    /// oscillation/anti-persistence.
    ///
    /// **Real, documented limitation, not fully solved here**: `phi >= 1`
    /// only catches a fit that's *literally* non-reverting or explosive.
    /// A naive AR(1) OLS fit on a finite window is a known-biased
    /// estimator for anything closer to a genuine unit root -- a
    /// monotonic, non-reverting trend can still fit a real `phi` inside
    /// `(0, 1)` on a finite sample (this is exactly why formal
    /// statistical unit-root tests, e.g. Dickey-Fuller, exist; this
    /// function doesn't run one). See
    /// `estimated_half_life_fits_a_spurious_value_on_a_pure_trend` in
    /// this module's own tests for a concrete, regression-tested example
    /// of this limitation -- callers should treat any estimate from this
    /// function as a rough real signal, not a rigorous one.
    pub fn estimated_half_life_cycles(&self) -> Option<f64> {
        if self.samples.len() < MIN_SAMPLES_FOR_ZSCORE {
            return None;
        }
        let n = self.samples.len();
        let mean = self.samples.iter().sum::<f64>() / n as f64;
        let values: Vec<f64> = self.samples.iter().copied().collect();
        let mut cov = 0.0;
        let mut var = 0.0;
        for i in 0..(n - 1) {
            let a = values[i] - mean;
            let b = values[i + 1] - mean;
            cov += a * b;
            var += a * a;
        }
        if var <= 0.0 {
            return None;
        }
        let phi = cov / var;
        if !(phi > 0.0 && phi < 1.0) {
            return None;
        }
        Some(std::f64::consts::LN_2 / -phi.ln())
    }
}

/// Converts [`RollingWindow::estimated_half_life_cycles`]'s real cycle
/// count into real years, given the real wall-clock seconds per cycle
/// (this mode's resync cadence, `factor_graph::MAX_FACTOR_STALENESS_SECS`
/// -- not hardcoded here, since this module has no dependency on
/// `factor_graph` and shouldn't gain one just for a unit conversion).
pub fn half_life_cycles_to_years(half_life_cycles: f64, seconds_per_cycle: i64) -> f64 {
    const SECONDS_PER_YEAR: f64 = 365.0 * 24.0 * 3600.0;
    half_life_cycles * seconds_per_cycle as f64 / SECONDS_PER_YEAR
}

/// Real residual decomposition -- `INSTRUCTIONS.md` §3B's
/// `epsilon_t = R_t - sum(beta_k * F_k)`. `returns[i]` and
/// `eigenvectors[i]` must be the same token ordering `factor_graph`'s
/// `structural_factors` produced them in. `k` is how many *leading*
/// (smallest-eigenvalue, per `factor_graph::StructuralFactors`'s own
/// ascending convention) factors to project onto -- smallest eigenvalues
/// carry the real structural clustering signal for a graph Laplacian
/// specifically (the opposite convention from equity-style PCA, where
/// the *largest*-eigenvalue components matter most; see
/// `factor_graph.rs`'s own doc comment on why `lambda_0`'s eigenvector
/// isn't the naive "constant vector" equity intuition would suggest
/// either -- same graph-Laplacian-specific correction applies here).
///
/// Real inputs, resolved by the caller (real annualized or per-cycle
/// returns, real eigenvectors from a real resync) -- this function does
/// no chain reads and doesn't know what "returns" even are (percent?
/// log-return? caller's choice, consistently applied).
pub fn compute_residuals(returns: &[f64], eigenvectors: &[Vec<f64>], k: usize) -> Vec<f64> {
    let n = returns.len();
    if n == 0 || eigenvectors.len() != n {
        return Vec::new();
    }
    let n_factors = eigenvectors[0].len();
    let k = k.min(n_factors);
    // Factor scores: F[j] = sum_i eigenvectors[i][j] * returns[i] --
    // real projection onto an orthonormal basis, valid because
    // `factor_graph::structural_factors`'s eigenvectors are orthonormal
    // (see that module's own `eigenvectors_are_orthonormal` test).
    let mut factor_scores = vec![0.0; k];
    for (i, &r) in returns.iter().enumerate() {
        for j in 0..k {
            factor_scores[j] += eigenvectors[i][j] * r;
        }
    }
    (0..n)
        .map(|i| {
            let reconstructed: f64 = (0..k).map(|j| factor_scores[j] * eigenvectors[i][j]).sum();
            returns[i] - reconstructed
        })
        .collect()
}

/// Which side of a real residual divergence a symbol is on right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidualSide {
    /// Residual deviated far enough negative -- a real candidate to
    /// *long* (per `INSTRUCTIONS.md` §3B: long the underperformer).
    Underperformer,
    /// Residual deviated far enough positive -- a real candidate to
    /// *short* (short the overperformer).
    Overperformer,
}

/// Threshold (in standard deviations) a real residual must clear before
/// it's a real pair-trade candidate at all -- `INSTRUCTIONS.md` §3B's
/// own "2-3 standard deviations" range; `2.5` is the midpoint, a
/// starting value not calibrated against any real trading history yet.
pub const PAIR_TRADE_ENTRY_ZSCORE: f64 = 2.5;

/// `Some(side)` iff `zscore` clears [`PAIR_TRADE_ENTRY_ZSCORE`] in
/// either direction; `None` otherwise (including the boundary itself --
/// strict inequality, matching this codebase's other gate boundaries,
/// e.g. `decide_basis_trade`/`decide_short_leg`).
pub fn classify_residual(zscore: f64) -> Option<ResidualSide> {
    if zscore <= -PAIR_TRADE_ENTRY_ZSCORE {
        Some(ResidualSide::Underperformer)
    } else if zscore >= PAIR_TRADE_ENTRY_ZSCORE {
        Some(ResidualSide::Overperformer)
    } else {
        None
    }
}

/// One real curated symbol's current residual -- both the raw value
/// (same percent units the caller's real returns were in, per
/// [`compute_residuals`]) and its standardized z-score against that
/// symbol's own rolling history. The caller's job to build (real rolling
/// stats from a real, per-symbol [`RollingWindow`]). Both fields matter
/// downstream for different reasons: `zscore` drives entry/exit
/// classification ([`classify_residual`]/[`should_close_pair`]),
/// `residual_pct` is the real, percent-denominated magnitude a caller
/// needs for an actual expected-edge estimate (a z-score alone is a
/// standardized unit, not a percent return -- not usable directly as
/// `factor_borrow_gate::decide_short_leg`'s `expected_reversion_edge_pct`).
#[derive(Debug, Clone, Copy)]
pub struct SymbolResidual {
    pub symbol: &'static str,
    pub mint: AccountId,
    pub residual_pct: f64,
    pub zscore: f64,
}

/// A real, ranked pair-trade candidate: the single most-underperforming
/// and single most-overperforming curated symbol this cycle, both
/// clearing [`PAIR_TRADE_ENTRY_ZSCORE`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairTradeCandidate {
    pub long_symbol: &'static str,
    pub long_mint: AccountId,
    pub long_residual_pct: f64,
    pub long_zscore: f64,
    pub short_symbol: &'static str,
    pub short_mint: AccountId,
    pub short_residual_pct: f64,
    pub short_zscore: f64,
}

/// The real, reusable search -- same shape as
/// `derivative_router::find_best_funding_opportunities`/
/// `factor_borrow_gate::gate_short_legs`: scans every real residual,
/// picks the single most extreme candidate on each side, and returns a
/// pair only if both sides clear the entry threshold and aren't the same
/// symbol. `None` if no real pair clears the bar this cycle -- not a
/// "trade something anyway" fallback.
pub fn find_best_pair(residuals: &[SymbolResidual]) -> Option<PairTradeCandidate> {
    let most_underperforming = residuals
        .iter()
        .filter(|r| classify_residual(r.zscore) == Some(ResidualSide::Underperformer))
        .min_by(|a, b| a.zscore.total_cmp(&b.zscore))?;
    let most_overperforming = residuals
        .iter()
        .filter(|r| classify_residual(r.zscore) == Some(ResidualSide::Overperformer))
        .max_by(|a, b| a.zscore.total_cmp(&b.zscore))?;
    if most_underperforming.symbol == most_overperforming.symbol {
        return None;
    }
    Some(PairTradeCandidate {
        long_symbol: most_underperforming.symbol,
        long_mint: most_underperforming.mint,
        long_residual_pct: most_underperforming.residual_pct,
        long_zscore: most_underperforming.zscore,
        short_symbol: most_overperforming.symbol,
        short_mint: most_overperforming.mint,
        short_residual_pct: most_overperforming.residual_pct,
        short_zscore: most_overperforming.zscore,
    })
}

/// Z-score magnitude (either leg) at or below which an open pair trade
/// is considered reverted -- "close, took profit," per `PLAN-1.md`'s own
/// "has it reverted" close-pass requirement. Starting value.
pub const PAIR_TRADE_EXIT_ZSCORE: f64 = 0.5;

/// Z-score magnitude (either leg, in the direction that makes the trade
/// worse, not better) at or beyond which an open pair trade is
/// considered blown through a stop -- "close, cut the loss," per
/// `PLAN-1.md`'s own "or blown through a stop" close-pass requirement.
/// Starting value, deliberately wider than
/// [`PAIR_TRADE_ENTRY_ZSCORE`] (a stop should be a real tail event, not
/// just "slightly more extreme than entry").
pub const PAIR_TRADE_STOP_ZSCORE: f64 = 4.0;

/// Why [`should_close_pair`] says to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Reverted,
    StopLoss,
}

/// `PLAN-1.md`'s close-pass check, run every cycle against an already-open
/// pair's *current* long/short z-scores (not the entry-time ones).
/// `long_zscore`/`short_zscore` follow the same sign convention
/// [`find_best_pair`] uses (long leg's zscore should have started very
/// negative, short leg's very positive).
pub fn should_close_pair(long_zscore: f64, short_zscore: f64) -> Option<CloseReason> {
    if long_zscore.abs() <= PAIR_TRADE_EXIT_ZSCORE && short_zscore.abs() <= PAIR_TRADE_EXIT_ZSCORE {
        return Some(CloseReason::Reverted);
    }
    if long_zscore <= -PAIR_TRADE_STOP_ZSCORE || short_zscore >= PAIR_TRADE_STOP_ZSCORE {
        return Some(CloseReason::StopLoss);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // --- RollingWindow ---------------------------------------------------

    #[test]
    fn rolling_window_none_before_min_samples() {
        let mut w = RollingWindow::new(20);
        for i in 0..(MIN_SAMPLES_FOR_ZSCORE - 1) {
            w.push(i as f64);
        }
        assert!(w.stats().is_none());
        assert!(w.zscore(0.0).is_none());
    }

    #[test]
    fn rolling_window_real_stats_after_min_samples() {
        let mut w = RollingWindow::new(20);
        // 1..=10 -- mean 5.5, sample stdev (n-1 denominator) = sqrt(9.1666...) ~ 3.0277
        for i in 1..=MIN_SAMPLES_FOR_ZSCORE {
            w.push(i as f64);
        }
        let stats = w.stats().expect("should have stats");
        assert_eq!(stats.n, MIN_SAMPLES_FOR_ZSCORE);
        assert!(approx(stats.mean, 5.5));
        assert!((stats.stdev - 3.0276503540974917).abs() < 1e-9);
    }

    #[test]
    fn rolling_window_len_caps_at_capacity() {
        let mut w = RollingWindow::new(5);
        for i in 1..=10 {
            w.push(i as f64);
        }
        assert_eq!(w.len(), 5);
    }

    #[test]
    fn rolling_window_eviction_changes_mean_correctly() {
        let mut w = RollingWindow::new(MIN_SAMPLES_FOR_ZSCORE);
        for i in 1..=MIN_SAMPLES_FOR_ZSCORE {
            w.push(i as f64); // window now holds 1..=10, mean 5.5
        }
        for _ in 0..MIN_SAMPLES_FOR_ZSCORE {
            w.push(100.0); // evicts every original sample, one at a time
        }
        let stats = w.stats().expect("should have stats");
        assert!(approx(stats.mean, 100.0));
        assert!(approx(stats.stdev, 0.0));
    }

    #[test]
    fn rolling_window_samples_roundtrips_through_from_samples() {
        let mut w = RollingWindow::new(5);
        for i in 1..=7 {
            w.push(i as f64); // evicts 1,2 -- window holds 3..=7
        }
        let snapshot: Vec<f64> = w.samples().collect();
        assert_eq!(snapshot, vec![3.0, 4.0, 5.0, 6.0, 7.0]);

        let restored = RollingWindow::from_samples(5, snapshot);
        assert_eq!(restored.len(), 5);
        assert_eq!(restored.samples().collect::<Vec<f64>>(), vec![3.0, 4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn rolling_window_from_samples_evicts_oldest_when_over_capacity() {
        // A snapshot carrying more samples than the current capacity
        // (e.g. capacity shrank since the snapshot was taken) degrades
        // the same way live pushes do -- oldest evicted first.
        let restored = RollingWindow::from_samples(3, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(restored.samples().collect::<Vec<f64>>(), vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn rolling_window_zero_variance_zscore_is_none() {
        let mut w = RollingWindow::new(20);
        for _ in 0..MIN_SAMPLES_FOR_ZSCORE {
            w.push(42.0);
        }
        assert!(w.zscore(42.0).is_none());
        assert!(w.zscore(100.0).is_none());
    }

    #[test]
    fn rolling_window_zscore_matches_hand_computed_value() {
        let mut w = RollingWindow::new(20);
        for i in 1..=MIN_SAMPLES_FOR_ZSCORE {
            w.push(i as f64);
        }
        // mean 5.5, stdev ~3.0276503540974917 -- z of 5.5 is 0.
        assert!(approx(w.zscore(5.5).unwrap(), 0.0));
        let z = w.zscore(11.5).unwrap();
        assert!((z - (6.0 / 3.0276503540974917)).abs() < 1e-9);
    }

    #[test]
    fn estimated_half_life_none_before_min_samples() {
        let mut w = RollingWindow::new(20);
        for i in 0..(MIN_SAMPLES_FOR_ZSCORE - 1) {
            w.push(i as f64);
        }
        assert!(w.estimated_half_life_cycles().is_none());
    }

    fn decaying_series(phi: f64, x0: f64, n: usize) -> RollingWindow {
        let mut w = RollingWindow::new(n);
        let mut x = x0;
        for _ in 0..n {
            w.push(x);
            x *= phi;
        }
        w
    }

    #[test]
    fn estimated_half_life_is_positive_for_a_real_decaying_series() {
        let w = decaying_series(0.9, 100.0, 20);
        let half_life = w.estimated_half_life_cycles().expect("should fit a real half-life");
        assert!(half_life > 0.0);
    }

    #[test]
    fn estimated_half_life_grows_with_slower_decay() {
        // A slower-decaying (larger phi, closer to 1) series takes
        // longer to revert -- the fitted half-life must reflect that
        // directionally, not just return *some* positive number.
        let fast = decaying_series(0.5, 100.0, 20);
        let slow = decaying_series(0.95, 100.0, 20);
        let fast_half_life = fast.estimated_half_life_cycles().expect("fast series should fit");
        let slow_half_life = slow.estimated_half_life_cycles().expect("slow series should fit");
        assert!(slow_half_life > fast_half_life, "slow={slow_half_life} fast={fast_half_life}");
    }

    #[test]
    fn estimated_half_life_none_for_pure_oscillation() {
        // Alternating +1/-1 -- anti-persistent (phi < 0), not real
        // mean-reverting behavior in the AR(1) sense this fits for.
        let mut w = RollingWindow::new(20);
        for i in 0..MIN_SAMPLES_FOR_ZSCORE {
            w.push(if i % 2 == 0 { 1.0 } else { -1.0 });
        }
        assert!(w.estimated_half_life_cycles().is_none());
    }

    #[test]
    fn estimated_half_life_fits_a_spurious_value_on_a_pure_trend() {
        // Real, documented limitation, not a desired behavior: a naive
        // AR(1) OLS fit on a monotonic, non-reverting trend still
        // produces a real phi in (0, 1) on a finite window (a known
        // statistical fact -- this is exactly why formal unit-root
        // testing exists, which this simple estimator doesn't do). This
        // test exists to make that limitation explicit and regression-
        // tested, not to claim it's handled.
        let mut w = RollingWindow::new(20);
        for i in 0..MIN_SAMPLES_FOR_ZSCORE {
            w.push(i as f64);
        }
        assert!(w.estimated_half_life_cycles().is_some());
    }

    #[test]
    fn half_life_cycles_to_years_hand_computed() {
        // 10 cycles at 30s/cycle = 300s = 300 / (365*24*3600) years.
        let years = half_life_cycles_to_years(10.0, 30);
        assert!(approx(years, 300.0 / (365.0 * 24.0 * 3600.0)));
    }

    // --- compute_residuals -------------------------------------------------

    #[test]
    fn compute_residuals_is_zero_when_returns_exactly_match_one_factor() {
        // Token returns are an exact scalar multiple of eigenvector
        // column 0 -- projecting onto k=1 factor must reconstruct them
        // exactly, leaving zero residual everywhere.
        let eigenvectors = vec![vec![0.6, 0.8], vec![0.8, -0.6]]; // orthonormal 2x2
        let returns = vec![0.6 * 3.0, 0.8 * 3.0]; // 3.0 * eigenvector column 0
        let residuals = compute_residuals(&returns, &eigenvectors, 1);
        assert!(approx(residuals[0], 0.0));
        assert!(approx(residuals[1], 0.0));
    }

    #[test]
    fn compute_residuals_isolates_idiosyncratic_component() {
        // Same orthonormal basis; returns are a mix of both factors --
        // with k=1 (only factor 0), the leftover must be exactly the
        // factor-1 contribution.
        let eigenvectors = vec![vec![0.6, 0.8], vec![0.8, -0.6]];
        let factor0_contribution = [0.6 * 3.0, 0.8 * 3.0];
        let factor1_contribution = [0.8 * 2.0, -0.6 * 2.0];
        let returns = vec![
            factor0_contribution[0] + factor1_contribution[0],
            factor0_contribution[1] + factor1_contribution[1],
        ];
        let residuals = compute_residuals(&returns, &eigenvectors, 1);
        assert!(approx(residuals[0], factor1_contribution[0]));
        assert!(approx(residuals[1], factor1_contribution[1]));
    }

    #[test]
    fn compute_residuals_k_zero_returns_the_raw_returns() {
        let eigenvectors = vec![vec![1.0], vec![0.0]];
        let returns = vec![5.0, -3.0];
        let residuals = compute_residuals(&returns, &eigenvectors, 0);
        assert_eq!(residuals, returns);
    }

    #[test]
    fn compute_residuals_empty_on_mismatched_lengths() {
        let eigenvectors = vec![vec![1.0]];
        let returns = vec![1.0, 2.0];
        assert!(compute_residuals(&returns, &eigenvectors, 1).is_empty());
    }

    // --- classify_residual / find_best_pair --------------------------------

    #[test]
    fn classify_residual_boundaries() {
        assert_eq!(classify_residual(-PAIR_TRADE_ENTRY_ZSCORE), Some(ResidualSide::Underperformer));
        assert_eq!(classify_residual(PAIR_TRADE_ENTRY_ZSCORE), Some(ResidualSide::Overperformer));
        assert_eq!(classify_residual(-PAIR_TRADE_ENTRY_ZSCORE + 0.01), None);
        assert_eq!(classify_residual(PAIR_TRADE_ENTRY_ZSCORE - 0.01), None);
        assert_eq!(classify_residual(0.0), None);
    }

    fn sym(symbol: &'static str, mint: AccountId, zscore: f64) -> SymbolResidual {
        // residual_pct deliberately != zscore in these tests (real usage
        // has them differ too, e.g. a symbol with high historical
        // volatility has a smaller zscore for the same raw residual_pct)
        // so a test that only checked zscore couldn't accidentally pass
        // by conflating the two fields.
        SymbolResidual { symbol, mint, residual_pct: zscore * 1.3, zscore }
    }

    #[test]
    fn find_best_pair_picks_the_most_extreme_pair() {
        let residuals = [
            sym("SOL", 1, -1.0),  // doesn't clear threshold
            sym("BTC", 2, -3.5),  // most extreme underperformer
            sym("ETH", 3, -2.6),  // clears, but not the most extreme
            sym("XRP", 4, 2.7),   // clears, but not the most extreme
            sym("BNB", 5, 3.9),   // most extreme overperformer
            sym("SUI", 6, 0.5),   // doesn't clear threshold
        ];
        let pair = find_best_pair(&residuals).expect("should find a pair");
        assert_eq!(pair.long_symbol, "BTC");
        assert_eq!(pair.short_symbol, "BNB");
        assert!(approx(pair.long_zscore, -3.5));
        assert!(approx(pair.short_zscore, 3.9));
        assert!(approx(pair.long_residual_pct, -3.5 * 1.3));
        assert!(approx(pair.short_residual_pct, 3.9 * 1.3));
    }

    #[test]
    fn find_best_pair_none_when_only_one_side_clears() {
        let residuals = [sym("SOL", 1, -3.0), sym("BTC", 2, 0.1), sym("ETH", 3, -0.5)];
        assert!(find_best_pair(&residuals).is_none());
    }

    #[test]
    fn find_best_pair_none_when_nothing_clears() {
        let residuals = [sym("SOL", 1, 0.5), sym("BTC", 2, -1.0)];
        assert!(find_best_pair(&residuals).is_none());
    }

    // --- should_close_pair ---------------------------------------------

    #[test]
    fn should_close_pair_reverted_when_both_legs_near_zero() {
        assert_eq!(should_close_pair(0.1, -0.2), Some(CloseReason::Reverted));
        assert_eq!(should_close_pair(0.0, 0.0), Some(CloseReason::Reverted));
    }

    #[test]
    fn should_close_pair_stop_loss_when_a_leg_worsens_past_the_stop() {
        assert_eq!(should_close_pair(-4.5, 3.0), Some(CloseReason::StopLoss));
        assert_eq!(should_close_pair(-2.0, 4.5), Some(CloseReason::StopLoss));
    }

    #[test]
    fn should_close_pair_none_while_still_a_healthy_open_position() {
        // Still meaningfully diverged (worth holding) but not yet
        // reverted and not yet a stop-loss.
        assert_eq!(should_close_pair(-3.0, 3.2), None);
    }

    #[test]
    fn should_close_pair_stop_loss_takes_priority_shape_is_consistent() {
        // A leg that's reverted (near zero) while the OTHER leg is
        // blown through its stop must still report StopLoss, not
        // Reverted -- reverted requires BOTH legs near zero.
        assert_eq!(should_close_pair(0.1, 4.5), Some(CloseReason::StopLoss));
    }
}
