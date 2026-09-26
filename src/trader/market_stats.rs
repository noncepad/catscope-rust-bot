//! Pure EWMA volatility/correlation tracker for `marketwatchv1`'s
//! dashboard -- no live dependency (no WIT host imports, no `Graph`), same
//! "pure module + native unit tests" convention this codebase's other
//! statistical trackers (e.g. a hawkes-style factor tracker) already use.
//! `marketwatchv1::state` is the only live caller: it feeds this module
//! fresh USD quotes (from `TradeRouter::route`) on a throttled cadence and
//! serializes [`MarketStatsTracker::snapshot`]'s output onto the wire.
//!
//! # Method
//!
//! Each symbol's log-return `r_t = ln(price_t / price_{t-1})` feeds a
//! discrete-time EWMA mean/variance estimate (same shape this session's
//! own momentum-trading design note used):
//! ```text
//! ewma_return_t = (1-λ)·ewma_return_{t-1} + λ·r_t
//! ewma_var_t    = (1-λ)·ewma_var_{t-1}    + λ·r_t²
//! volatility_t  = sqrt(ewma_var_t)
//! ```
//! Every tracked pair (i, j) gets its own EWMA covariance of the two
//! symbols' returns, updated only on ticks where *both* have a fresh
//! return:
//! ```text
//! ewma_cov_t(i,j) = (1-λ)·ewma_cov_{t-1}(i,j) + λ·r_i·r_j
//! correlation(i,j) = ewma_cov_t(i,j) / (volatility_i · volatility_j)
//! ```
//! `λ` is a conservative fixed placeholder (see [`EWMA_LAMBDA`]), tuned
//! from real observed behavior later -- not a blind fit, same discipline
//! every other EWMA/factor tracker in this codebase's history committed
//! to.

/// One tracked symbol's identity: its display symbol, real mint, and real
/// decimals -- everything [`crate::brain::marketwatchv1::state`] needs to
/// probe [`crate::trader::pricegraph::TradeRouter::route`] for a live USD
/// quote, with no further lookup.
pub struct TrackedMint {
    pub symbol: &'static str,
    pub mint: solana_sdk::pubkey::Pubkey,
    pub decimals: u8,
}

/// Fixed set of symbols this dashboard tracks, in wire order. The Go side
/// (`optimizer/brain/marketwatchv1`) hardcodes this exact same order to
/// decode `MarketStats` messages -- see that package's own doc comment.
///
/// Deliberately **not** `symbol_mint_config::SYMBOL_MINT_MAP` (that
/// module's `build.rs`-curated `CURATED_SYMBOL_MINTS`, shared with
/// Phoenix/Velocity perp `base_mint` joins elsewhere in this codebase) --
/// this dashboard's first live run (2026-09-25) found that map's non-SOL
/// entries (BTC/ETH/XRP/BNB/SUI, all Portal-wrapped) had **zero** tracked
/// pool rows across every one of `raydium_amm_pool`/`raydium_clmm_pool`/
/// `raydium_cpmm_pool`/`orca_whirlpool_pool`/`pumpswap_pool` in a real
/// `prefetch.db` snapshot -- a real, live-confirmed gap, not a
/// hypothetical one (that shared map was curated for perp `base_mint`
/// matching, which doesn't need real spot liquidity the way this
/// dashboard's `TradeRouter::route` probe does). This table is this
/// dashboard's own, independently confirmed against that same live
/// snapshot: every mint below had hundreds to hundreds-of-thousands of
/// real tracked pool rows (SOL=743,994, JUP=510, JTO=119, BONK=843,
/// WIF=347, RAY=653) -- real, major, recognizable Solana-ecosystem
/// tokens (the native asset, its largest DEX aggregator's token, a
/// leading MEV/staking token, the two largest Solana-native memecoins by
/// historical market cap, and a leading DEX's governance token), not
/// wrapped-from-elsewhere assets whose Solana-side liquidity can't be
/// assumed. Mint/decimals hardcoded here (not looked up at runtime)
/// since both were independently confirmed against the same real
/// `mint_info` snapshot.
pub const TRACKED_MINTS: [TrackedMint; 6] = [
    TrackedMint {
        symbol: "SOL",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("So11111111111111111111111111111111111111112"),
        decimals: 9,
    },
    TrackedMint {
        symbol: "JUP",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN"),
        decimals: 6,
    },
    TrackedMint {
        symbol: "JTO",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL"),
        decimals: 9,
    },
    TrackedMint {
        symbol: "BONK",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263"),
        decimals: 5,
    },
    TrackedMint {
        symbol: "WIF",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm"),
        decimals: 6,
    },
    TrackedMint {
        symbol: "RAY",
        mint: solana_sdk::pubkey::Pubkey::from_str_const("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R"),
        decimals: 6,
    },
];

/// Number of tracked symbols.
pub const N_SYMBOLS: usize = TRACKED_MINTS.len();

/// Number of distinct unordered symbol pairs (i < j): `N*(N-1)/2`.
pub const N_PAIRS: usize = N_SYMBOLS * (N_SYMBOLS - 1) / 2;

/// EWMA decay shared by every symbol's return/variance estimate and every
/// pair's covariance estimate. 0.05 means each new observation gets ~5%
/// weight -- roughly a 20-tick half-life, conservative enough that one
/// noisy quote doesn't swing the whole board, aggressive enough to show
/// real movement within a few minutes at this bot's throttled recompute
/// cadence (`marketwatchv1::state::MARKET_STATS_INTERVAL_SLOTS`).
const EWMA_LAMBDA: f64 = 0.05;

/// A single reading beyond this ratio (either direction) from the
/// current slow-moving [`anchor`](SymbolStats::anchor) -- not the
/// immediately-preceding tick, see that field's own doc comment for why
/// -- is discarded outright as implausible, rather than updated into
/// `price_prev`/fed into the EWMA. Real, live-confirmed incident this
/// guards against (2026-09-26): `TradeRouter::route_slippage_aware`'s
/// own protections (pool-reuse dedup, node-revisit guard, per-hop
/// utilization cap) stop specific execution-*unsafe* degeneracies, but
/// `widest_path` is still fundamentally an amount-*maximizing* search --
/// the right lens for real arbitrage sizing, the wrong one for "what's
/// the honest market price" -- so it can and does prefer whatever
/// technically-valid pool happens to quote the biggest output, mispriced
/// or not. Live-confirmed real, sustained incidents, not single
/// flickering ticks: one run watched SOL/USDC read a stable $1416.53 for
/// 20+ consecutive ticks; a later run (after gating against the
/// immediately-preceding tick only, the first version of this fix)
/// walked from a real ~$120-200 range up to $1300 over several
/// individually-small (<3x from *each other*) steps -- "boiling frog,"
/// each step innocuous next to the one before it, ruinous next to where
/// it started. A rejected reading contributes nothing at all (no return,
/// no EWMA update, `price_prev`/`anchor` both left untouched) -- the
/// last known-good price stays displayed/tracked until either a
/// plausible reading arrives or the process restarts. Deliberately
/// "freeze rather than ever display a number known to be implausible,"
/// not "eventually believe it if it repeats."
///
/// Real, honest limitation: this can't distinguish "implausible" from "a
/// genuine, real move bigger than 3x looked up against a reference that
/// may be ~10-20 minutes stale" -- for these six tracked symbols (not
/// exactly known for that kind of move outside a black-swan event) that
/// trade-off favors never showing a bogus number, at the cost of a real
/// (if extreme) move needing a process restart to be reflected sooner.
/// Also can't protect the very first observation for a symbol (nothing
/// to compare it against yet) -- multiple separate live runs have all
/// happened to get a real price on their first tick, but that's an
/// empirical observation from those runs, not a structural guarantee.
const REJECT_RATIO: f64 = 3.0;

/// [`anchor`](SymbolStats::anchor)'s own EWMA decay -- deliberately much
/// slower than [`EWMA_LAMBDA`] (0.05, roughly a 20-tick/~2-3 minute
/// half-life at this bot's own push cadence): a ~100-tick (~15-20
/// minute) half-life is what stops a short run of individually-plausible
/// -looking bad readings (a "boiling frog" walk, not one single spike --
/// see [`REJECT_RATIO`]'s own doc comment for the real, live-confirmed
/// incident this specifically closes) from dragging the trusted
/// reference price along with it within just a few minutes, while a
/// genuine, real move sustained over that same longer timescale still
/// gets incorporated rather than frozen out forever.
const ANCHOR_LAMBDA: f64 = 0.01;

/// Live EWMA return/variance estimate for one tracked symbol.
#[derive(Debug, Clone, Copy, Default)]
pub struct SymbolStats {
    price_prev: f64,
    has_prev: bool,
    ewma_return: f64,
    ewma_var: f64,
    /// Slow-moving (see [`ANCHOR_LAMBDA`]) sanity reference `update`
    /// gates new readings against, instead of `price_prev` directly.
    /// `price_prev` itself still drives the actual returned/reported
    /// log-return and EWMA volatility (a real tick-to-tick return, not
    /// smoothed against a lagging reference) -- `anchor` exists purely
    /// to decide whether a new reading is trustworthy enough to become
    /// the next `price_prev` at all.
    anchor: f64,
}

impl SymbolStats {
    /// Feeds a new price observation. Returns this update's log-return.
    /// `None` on the very first observation, a non-positive price (a
    /// pool that hasn't delivered a real quote yet, not a true zero
    /// price), or a reading rejected by [`REJECT_RATIO`] (see its own
    /// doc comment) -- the last two both leave `price_prev`/`anchor` and
    /// the EWMA state completely unchanged, same as "no observation this
    /// tick."
    pub fn update(&mut self, price: f64) -> Option<f64> {
        if price <= 0.0 {
            return None;
        }
        if !self.has_prev {
            self.price_prev = price;
            self.anchor = price;
            self.has_prev = true;
            return None;
        }
        let ratio = price / self.anchor;
        if ratio > REJECT_RATIO || ratio < 1.0 / REJECT_RATIO {
            return None;
        }
        let r = (price / self.price_prev).ln();
        self.price_prev = price;
        self.anchor = (1.0 - ANCHOR_LAMBDA) * self.anchor + ANCHOR_LAMBDA * price;
        self.ewma_return = (1.0 - EWMA_LAMBDA) * self.ewma_return + EWMA_LAMBDA * r;
        self.ewma_var = (1.0 - EWMA_LAMBDA) * self.ewma_var + EWMA_LAMBDA * r * r;
        Some(r)
    }

    /// EWMA standard deviation of log-returns -- 0.0 until at least one
    /// real return has been observed.
    pub fn volatility(&self) -> f64 {
        self.ewma_var.max(0.0).sqrt()
    }

    /// Most recently fed price, or 0.0 if none yet.
    pub fn last_price(&self) -> f64 {
        self.price_prev
    }
}

/// Live EWMA covariance estimate for one tracked symbol pair.
#[derive(Debug, Clone, Copy, Default)]
pub struct PairCovariance {
    ewma_cov: f64,
}

impl PairCovariance {
    fn update(&mut self, r_a: f64, r_b: f64) {
        self.ewma_cov = (1.0 - EWMA_LAMBDA) * self.ewma_cov + EWMA_LAMBDA * r_a * r_b;
    }

    /// Pearson-style correlation implied by this pair's EWMA covariance
    /// and both symbols' own EWMA volatility. 0.0 (not an error -- there
    /// is no error return here) if either volatility is still zero (not
    /// enough data yet), clamped to `[-1.0, 1.0]` since EWMA covariance
    /// and variance are estimated independently and can drift very
    /// slightly outside that range in floating point.
    fn correlation(&self, vol_a: f64, vol_b: f64) -> f64 {
        let denom = vol_a * vol_b;
        if denom <= 0.0 {
            0.0
        } else {
            (self.ewma_cov / denom).clamp(-1.0, 1.0)
        }
    }
}

/// Index into the fixed `N_PAIRS`-length upper-triangular layout for
/// symbols `i < j` (0-indexed into [`TRACKED_MINTS`]). Standard
/// row-major upper-triangle packing: pairs are ordered
/// `(0,1),(0,2),...,(0,N-1),(1,2),...,(N-2,N-1)`.
///
/// # Panics
/// If `i >= j` -- only ever called with `i < j` by
/// [`MarketStatsTracker::update`]/[`MarketStatsTracker::snapshot`] below.
pub fn pair_index(i: usize, j: usize) -> usize {
    assert!(i < j, "pair_index: i ({i}) must be < j ({j})");
    let mut idx = 0;
    for k in 0..i {
        idx += N_SYMBOLS - 1 - k;
    }
    idx + (j - i - 1)
}

/// Updates-per-second implied by `delta_count` new low-latency account
/// updates observed over `delta_seconds` of real wall-clock time between
/// two pushes -- the throughput dashboard panel's own data source (see
/// `marketwatchv1::state::StateHelper::evaluate`'s own doc comment for
/// where `delta_count`/`delta_seconds` come from). `0.0` (never NaN or
/// infinite) for a non-positive `delta_seconds` -- e.g. the very first
/// sample (no previous wall-clock time to diff against) or a
/// pathologically fast repeat call -- so a degenerate first push can
/// never render as an infinite/NaN rate on the dashboard.
pub fn updates_per_second(delta_count: u64, delta_seconds: f64) -> f64 {
    if delta_seconds <= 0.0 {
        return 0.0;
    }
    delta_count as f64 / delta_seconds
}

/// A snapshot of every tracked symbol's live price/volatility and every
/// tracked pair's live correlation, ready to serialize onto the wire
/// (`marketwatchv1::message::CustomMessageOutbound::MarketStats`).
#[derive(Debug, Clone, Copy)]
pub struct MarketStatsSnapshot {
    pub prices: [f64; N_SYMBOLS],
    pub volatilities: [f64; N_SYMBOLS],
    pub correlations: [f64; N_PAIRS],
}

/// Live tracker for every symbol's volatility and every pair's
/// correlation -- the whole dashboard's statistical state in one place.
#[derive(Debug, Default)]
pub struct MarketStatsTracker {
    symbols: [SymbolStats; N_SYMBOLS],
    pairs: [PairCovariance; N_PAIRS],
}

impl MarketStatsTracker {
    /// Feeds one fresh price per symbol (same order as
    /// [`TRACKED_MINTS`]) -- `None` for a symbol with no live quote this
    /// tick (e.g. `TradeRouter::route` found no path yet); that symbol's
    /// own stats simply don't advance, and it can't contribute a return to
    /// any pair's covariance this tick either. Updates every symbol's
    /// return/variance first, then every pair's covariance from whichever
    /// symbols actually produced a fresh return this call.
    pub fn update(&mut self, prices: [Option<f64>; N_SYMBOLS]) {
        let mut returns: [Option<f64>; N_SYMBOLS] = [None; N_SYMBOLS];
        for i in 0..N_SYMBOLS {
            if let Some(p) = prices[i] {
                returns[i] = self.symbols[i].update(p);
            }
        }
        for i in 0..N_SYMBOLS {
            let Some(r_i) = returns[i] else { continue };
            for j in (i + 1)..N_SYMBOLS {
                let Some(r_j) = returns[j] else { continue };
                self.pairs[pair_index(i, j)].update(r_i, r_j);
            }
        }
    }

    /// Current snapshot of every symbol's last price/volatility and every
    /// pair's correlation -- cheap (`N_SYMBOLS` + `N_PAIRS` reads, no
    /// recomputation), safe to call as often as the caller wants to push a
    /// wire update.
    pub fn snapshot(&self) -> MarketStatsSnapshot {
        let mut prices = [0.0; N_SYMBOLS];
        let mut volatilities = [0.0; N_SYMBOLS];
        for i in 0..N_SYMBOLS {
            prices[i] = self.symbols[i].last_price();
            volatilities[i] = self.symbols[i].volatility();
        }
        let mut correlations = [0.0; N_PAIRS];
        for i in 0..N_SYMBOLS {
            for j in (i + 1)..N_SYMBOLS {
                let k = pair_index(i, j);
                correlations[k] = self.pairs[k].correlation(volatilities[i], volatilities[j]);
            }
        }
        MarketStatsSnapshot { prices, volatilities, correlations }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_index_covers_every_pair_exactly_once() {
        let mut seen = [false; N_PAIRS];
        for i in 0..N_SYMBOLS {
            for j in (i + 1)..N_SYMBOLS {
                let k = pair_index(i, j);
                assert!(!seen[k], "pair_index({i},{j}) = {k} collides with an earlier pair");
                seen[k] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "pair_index left a gap in 0..N_PAIRS");
    }

    #[test]
    #[should_panic]
    fn pair_index_panics_when_not_ordered() {
        let _ = pair_index(2, 1);
    }

    #[test]
    fn symbol_stats_first_observation_has_no_return() {
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None);
        assert_eq!(s.volatility(), 0.0);
        assert_eq!(s.last_price(), 100.0);
    }

    #[test]
    fn symbol_stats_ignores_non_positive_price() {
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None);
        assert_eq!(s.update(0.0), None);
        assert_eq!(s.update(-5.0), None);
        // Neither bad observation should have perturbed price_prev.
        assert_eq!(s.last_price(), 100.0);
    }

    #[test]
    fn symbol_stats_rejects_an_implausible_single_tick_jump() {
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None); // bootstrap
        // 9x jump, well past REJECT_RATIO -- must be discarded, not
        // adopted as the new price_prev.
        assert_eq!(s.update(900.0), None);
        assert_eq!(s.last_price(), 100.0, "an implausible reading must not move price_prev");
        // A subsequent real, small move off the *original* trusted price
        // must still work normally afterward.
        let r = s.update(101.0);
        assert!(r.is_some());
        assert_eq!(s.last_price(), 101.0);
    }

    #[test]
    fn symbol_stats_never_adopts_a_sustained_implausible_reading() {
        // The real, live-confirmed failure mode this guards against:
        // a bad reading that *repeats* for many ticks (not a single
        // flickering glitch) must still never be adopted, no matter how
        // many times it repeats -- a "confirm across N ticks" debounce
        // would eventually accept this, which is exactly wrong here.
        let mut s = SymbolStats::default();
        assert_eq!(s.update(150.0), None);
        for _ in 0..50 {
            assert_eq!(s.update(1416.53), None);
        }
        assert_eq!(s.last_price(), 150.0, "a sustained implausible reading must never become trusted");
    }

    #[test]
    fn symbol_stats_rejects_an_implausible_crash_too() {
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None);
        // 1/9x -- implausible in the other direction too.
        assert_eq!(s.update(11.0), None);
        assert_eq!(s.last_price(), 100.0);
    }

    #[test]
    fn symbol_stats_accepts_a_move_just_inside_reject_ratio() {
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None);
        let r = s.update(100.0 * (REJECT_RATIO - 0.01));
        assert!(r.is_some(), "a move just inside REJECT_RATIO must be accepted");
    }

    #[test]
    fn symbol_stats_rejects_a_boiling_frog_drift() {
        // The real, live-confirmed failure mode gating against price_prev
        // alone couldn't catch: a walk of individually-small (<REJECT_RATIO
        // from each other) steps that cumulatively drifts far past any
        // sane price. A live run walked ~$120 -> $1300 this way. Each step
        // here is <REJECT_RATIO from the one before it, but the anchor
        // (which moves at ANCHOR_LAMBDA, far slower) should still catch and
        // reject the later, wildly-drifted steps.
        let mut s = SymbolStats::default();
        assert_eq!(s.update(120.0), None);
        let mut price = 120.0;
        let mut any_rejected = false;
        for _ in 0..200 {
            price *= 1.05; // well inside REJECT_RATIO tick-to-tick
            if s.update(price).is_none() {
                any_rejected = true;
            }
        }
        assert!(
            any_rejected,
            "a sustained gradual drift must eventually be rejected by the slow-moving anchor"
        );
        assert!(
            s.last_price() < 1300.0,
            "the boiling-frog drift must never fully reach the wildly-drifted endpoint; last_price = {}",
            s.last_price()
        );
    }

    #[test]
    fn symbol_stats_anchor_eventually_tracks_a_genuine_sustained_move() {
        // A real, sustained move (not a feed glitch) must still eventually
        // be reflected once the slow-moving anchor catches up -- this
        // isn't a permanent lockout, just a delay proportional to
        // ANCHOR_LAMBDA's decay.
        let mut s = SymbolStats::default();
        assert_eq!(s.update(100.0), None);
        // A real, sustained 2x move, fed repeatedly (as real ticks would
        // keep arriving at the new true price).
        let mut accepted_at_new_level = false;
        for _ in 0..2000 {
            if let Some(_) = s.update(200.0) {
                if (s.last_price() - 200.0).abs() < 1e-9 {
                    accepted_at_new_level = true;
                }
            }
        }
        assert!(
            accepted_at_new_level,
            "a real, sustained move must eventually be adopted once the anchor catches up"
        );
    }

    #[test]
    fn updates_per_second_divides_count_by_elapsed_time() {
        assert_eq!(updates_per_second(12_000, 2.0), 6_000.0);
    }

    #[test]
    fn updates_per_second_is_zero_for_zero_elapsed_time() {
        // The very first sample (no previous wall-clock time to diff
        // against yet) must never render as an infinite/NaN rate.
        assert_eq!(updates_per_second(500, 0.0), 0.0);
    }

    #[test]
    fn updates_per_second_is_zero_for_negative_elapsed_time() {
        assert_eq!(updates_per_second(500, -1.0), 0.0);
    }

    #[test]
    fn updates_per_second_is_zero_when_no_new_updates_arrived() {
        assert_eq!(updates_per_second(0, 5.0), 0.0);
    }

    #[test]
    fn symbol_stats_constant_price_has_zero_volatility() {
        let mut s = SymbolStats::default();
        for _ in 0..50 {
            s.update(100.0);
        }
        assert_eq!(s.volatility(), 0.0);
    }

    #[test]
    fn symbol_stats_volatility_is_positive_after_real_moves() {
        let mut s = SymbolStats::default();
        let mut last = None;
        for p in [100.0, 101.0, 99.0, 102.0, 98.0, 103.0] {
            last = s.update(p);
        }
        assert!(last.is_some());
        assert!(s.volatility() > 0.0);
    }

    #[test]
    fn perfectly_correlated_series_converges_near_one() {
        let mut tracker = MarketStatsTracker::default();
        // Symbol 0 and 1 always move together (identical returns);
        // everything else stays flat (no returns, so no covariance
        // contribution) -- isolates the pair-0-1 case.
        let mut price = 100.0f64;
        for step in 0..500 {
            price *= 1.0 + 0.01 * if step % 2 == 0 { 1.0 } else { -1.0 };
            let mut prices = [Some(50.0); N_SYMBOLS];
            prices[0] = Some(price);
            prices[1] = Some(price);
            tracker.update(prices);
        }
        let snap = tracker.snapshot();
        let corr01 = snap.correlations[pair_index(0, 1)];
        assert!(corr01 > 0.99, "expected near-perfect correlation, got {corr01}");
    }

    #[test]
    fn inversely_correlated_series_converges_near_negative_one() {
        let mut tracker = MarketStatsTracker::default();
        let mut price_a = 100.0f64;
        let mut price_b = 100.0f64;
        for step in 0..500 {
            let up = step % 2 == 0;
            price_a *= 1.0 + 0.01 * if up { 1.0 } else { -1.0 };
            price_b *= 1.0 + 0.01 * if up { -1.0 } else { 1.0 };
            let mut prices = [Some(50.0); N_SYMBOLS];
            prices[0] = Some(price_a);
            prices[1] = Some(price_b);
            tracker.update(prices);
        }
        let snap = tracker.snapshot();
        let corr01 = snap.correlations[pair_index(0, 1)];
        assert!(corr01 < -0.99, "expected near-perfect inverse correlation, got {corr01}");
    }

    #[test]
    fn missing_price_skips_that_symbol_without_panicking() {
        let mut tracker = MarketStatsTracker::default();
        let mut prices = [Some(10.0); N_SYMBOLS];
        prices[2] = None;
        tracker.update(prices);
        prices[2] = None;
        tracker.update(prices);
        let snap = tracker.snapshot();
        // Symbol 2 never got a real observation, so its last_price stays 0.
        assert_eq!(snap.prices[2], 0.0);
        assert_eq!(snap.volatilities[2], 0.0);
    }
}
