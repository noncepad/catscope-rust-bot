//! Dry-run arbitrage-cycle detector -- phase 1 of the trade-execution
//! planner. Cheaply discovers a candidate start token via
//! [`crate::trader::pricegraph::TradeRouter::find_arbitrage`], then
//! re-searches for a genuinely profitable cycle at real wallet-balance
//! size via
//! [`crate::trader::pricegraph::TradeRouter::find_arbitrage_slippage_aware`]
//! -- see `find_opportunity`'s doc comment for why a second real-size
//! *search*, not just a resized re-check of the same cycle, is needed.
//! Does **not** build or send any transaction yet.
//!
//! Deliberately depends only on `TradeRouter`/`Wallet`/`AccountId`, no
//! dex-specific state, so it stays easy to reason about and test in
//! isolation -- same standalone-module style as [`super::credit`].

use crate::{
    graph::AccountId,
    trader::{
        dex::DexState,
        pricegraph::{ArbitrageCycle, Hop, Route, TradeRouter},
        types::DexType,
    },
    wallet::Wallet,
};

/// Probe amount used only to discover whether a cycle exists and which
/// token it starts from -- discarded in favor of a real-balance-sized
/// re-check below. `find_arbitrage`'s Bellman-Ford path selection depends
/// only on `-ln(rate)` edge weights, not on `amount_in`, so *which* cycle
/// is found is independent of this value; it only needs to survive
/// integer fee-rounding through a few hops without a cp_quote hop
/// truncating to zero. 1_000_000 raw units is far above that rounding
/// floor for realistic fee_bps.
const PROBE_AMOUNT_RAW: u64 = 1_000_000;

/// Fraction of the wallet's real balance (in the cycle's start token) to
/// risk-size the honest profitability check at.
const REAL_SIZE_FRACTION_BPS: u32 = 1_000; // 10%

/// Defensive ceiling, independent of `REAL_SIZE_FRACTION_BPS` -- never
/// size more than a quarter of the wallet's balance into one cycle.
const REAL_SIZE_MAX_FRACTION_BPS: u32 = 2_500; // 25%

/// Skip sizing if the computed real amount is below this floor -- matches
/// `PROBE_AMOUNT_RAW`, since anything smaller isn't a meaningfully
/// different check from the probe itself.
const REAL_SIZE_MIN_RAW: u64 = PROBE_AMOUNT_RAW;

/// Max hops for the real-size, slippage-aware re-search below. Matches
/// the `4` used throughout the rest of this codebase for point-to-point
/// `route()` calls (e.g. `arbv1`'s diagnostic). Unlike `find_arbitrage`
/// (which can consider cycles up to the full node count, since its
/// log-space search is cheap regardless of length), each additional hop
/// here costs a real `cp_quote` call per edge per relaxation round, and a
/// cycle needing more than a handful of hops is increasingly unlikely to
/// stay profitable after that many hops' worth of fees regardless.
const REAL_SIZE_MAX_HOPS: usize = 4;

/// Minimum profit, in bps of amount_in, to report a cycle as an
/// opportunity -- covers compute-unit/priority-fee cost and cp_quote
/// estimation error (CLMM quotes are a constant-product approximation).
///
/// Reverted (2026-09-03) from a temporary debug value of `0` back to its
/// own intended `20` -- the `0` value was only ever meant to confirm the
/// incremental-router path could find *any* cycle end-to-end (done, real
/// cycles landed live this session), not to stay in place for real
/// sends: a 0bps *gross* "profit" cycle is very likely a net loss once
/// real fees (`ArbitrageCycle::net_profit_lamports`'s own base-fee +
/// priority-fee deduction) are subtracted. `find_arbitrage`'s own
/// negative-cycle detection is still the real structural gate regardless
/// of this value (it only ever returns a cycle whose product of
/// fee-adjusted rates exceeds 1.0) -- this just additionally filters out
/// cycles too thin to be worth sending in practice.
const MIN_PROFIT_BPS: u64 = 20;

/// How long to exclude a pool that failed exact-quote re-verification
/// (`ReverifyOutcome::HopFailed`) from the slippage-aware search
/// (`TradeRouter::mark_pool_cooldown`), in slots. ~400ms/slot observed
/// this session -> 30 minutes ≈ 1800s / 0.4s ≈ 4500 slots. Approximate by
/// design (real slot time varies) -- this cooldown running somewhat
/// shorter or longer than 30 real minutes doesn't matter for its purpose
/// (stop repeatedly re-selecting, and re-rejecting, a pool this bot's own
/// exact-quote check already found untradeable this session).
pub const POOL_COOLDOWN_SLOTS: u64 = 4_500;

/// Per-cycle cap on how many queued subscribe requests each dex's own
/// `flush_pool` sends in one `bulk_subscribe` call (tick arrays, vaults,
/// etc. -- see `OrcaState::token_sub_queue`/`RaydiumClmm::tick_array_sub_
/// queue`'s own doc comments).
///
/// Real, live-confirmed diagnostic (2026-09-07) caught Orca's own queue
/// backlog spike to 37,798 pending after a single startup burst of
/// ~4,754 fresh pool updates, taking ~4 minutes at `128`/cycle to fully
/// drain -- during which any pool whose tick-array request landed late
/// in that backlog stayed `PoolNotReady` for the whole window, across
/// every restart. Raising this to `512` did shrink and speed up that
/// specific drain (confirmed live) -- but also broke something more
/// important: `crate::graph::all_subscriptions_acked()` (which gates
/// `positions_loaded`, and therefore every real pair/directional/hawkes
/// trading decision) requires the *global* sent-vs-acked subscription
/// count to fully catch up, across every dex combined, not just this
/// one queue. At `512`/cycle the ongoing tick-array resubscription churn
/// (continuous, not just a one-time startup burst -- new pools drift
/// into new price windows constantly) pushed the global "sent" count up
/// faster than the host could ack in aggregate: `positions_loaded` never
/// flipped at all in a 500+-second live run, versus reliably flipping
/// within 284-359s at `128`/cycle across three separate runs. That's a
/// far worse outcome (real trading never starts, permanently, not just
/// intermittent `PoolNotReady` on a few pools) -- reverted back to `128`
/// until a fix that doesn't trip this global latch is found (e.g.
/// per-queue pacing that stays high only while the *specific* queue has
/// a deep backlog, or decoupling `positions_loaded` from ongoing
/// resubscription traffic rather than just the one-time startup burst).
pub const DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET: usize = 128;

/// A profitable arbitrage cycle sized against the bot's actual balance.
#[derive(Debug)]
pub struct ArbitrageOpportunity {
    pub cycle: ArbitrageCycle,
    /// The wallet's balance (raw units) in `cycle.start_token()` at the
    /// time of this check.
    pub wallet_balance: u64,
}

/// `wallet_balance * REAL_SIZE_FRACTION_BPS / 10_000`, clamped to
/// `REAL_SIZE_MAX_FRACTION_BPS` and floored at `REAL_SIZE_MIN_RAW`.
fn real_amount_in(wallet_balance: u64) -> Option<u64> {
    let fraction_bps = REAL_SIZE_FRACTION_BPS.min(REAL_SIZE_MAX_FRACTION_BPS) as u128;
    let amount = (wallet_balance as u128 * fraction_bps / 10_000) as u64;
    (amount >= REAL_SIZE_MIN_RAW).then_some(amount)
}

/// Look for a profitable arbitrage cycle sized against the bot's real
/// wallet balance. Dry-run only -- does not build or send a transaction.
///
/// Two-pass, probe-then-*re-search* (not probe-then-resize): the cheap
/// first pass (`find_arbitrage(PROBE_AMOUNT_RAW)`) only discovers a
/// candidate `start_token` -- its cycle *selection* is amount-independent
/// (log-space Bellman-Ford over `-ln(rate)` edges), which is exactly the
/// property that makes it too blind to slippage to trust for the real
/// check: it can only ever consider the single cycle with the best
/// *spot*-rate product, even when a different, spot-worse-but-deeper-
/// liquidity cycle would be genuinely more profitable at real trade size
/// (confirmed directly: `find_arbitrage_slippage_aware_finds_a_cycle_
/// find_arbitrage_misses` in `pricegraph.rs`'s tests constructs exactly
/// this case). So the second pass uses
/// [`TradeRouter::find_arbitrage_slippage_aware`], which re-searches from
/// `start_token` using real `cp_quote`-composed amounts throughout
/// (rounds capped at `REAL_SIZE_MAX_HOPS`, not the full node count `n`
/// `find_arbitrage` uses -- see that constant's doc comment for the
/// tradeoff), rather than re-cascading the exact same cycle the probe
/// found at a bigger size.
///
/// Native SOL is not special-cased via `Wallet::balance_sol` -- pools
/// trade wrapped SOL as an SPL mint, so `TokenDatabase::balance` against
/// the wSOL mint is the correct, uniform query for every possible
/// `start_token`, SOL included.
pub fn find_opportunity(
    router: &TradeRouter,
    wallet: &mut Wallet,
    wallet_owner: &AccountId,
) -> Option<ArbitrageOpportunity> {
    let probe = router.find_arbitrage(PROBE_AMOUNT_RAW)?;
    let start_token = probe.start_token();

    let wallet_balance: u64 = wallet
        .token_mut()
        .balance(wallet_owner, &start_token, true)
        .iter()
        .map(|(_, amount)| *amount)
        .sum();

    let amount_in = real_amount_in(wallet_balance)?;
    let real_cycle = router.find_arbitrage_slippage_aware(start_token, amount_in, REAL_SIZE_MAX_HOPS)?;
    if real_cycle.profit_bps() < MIN_PROFIT_BPS {
        return None;
    }

    Some(ArbitrageOpportunity {
        cycle: real_cycle,
        wallet_balance,
    })
}

/// Re-verify a found cycle's Orca CLMM hops against the exact, tick-aware
/// on-chain quote (`dex::orca::OrcaState::exact_quote`), rather than
/// trusting `find_arbitrage_slippage_aware`'s constant-product-within-
/// current-tick approximation for those hops.
///
/// Why this matters, concretely (found live this session): a real Orca
/// pool's CLMM position can have unusual liquidity structure right at the
/// current tick (a large `liquidity_net` boundary sitting almost exactly
/// at the current price) that the constant-product approximation has no
/// way to see -- it reported a plausible-looking `amount_out` for a pool
/// that, per the *exact* tick-walking math, can't actually be traded
/// through at all (`exact_quote` returned `0`). A cycle whose reported
/// profit depends on a hop like that isn't real and must not be surfaced
/// as a found opportunity.
///
/// Deliberately kept separate from `find_opportunity` itself (not folded
/// into its second pass) -- `find_opportunity` is explicitly dex-state-
/// free by design (see the module doc comment), and this needs
/// `&DexState` to call the real Orca quote. Callers apply this as a
/// post-processing step on whatever `find_opportunity` returns.
///
/// Sequentially re-quotes every hop with the corrected running amount:
/// Orca hops use `exact_quote`; every other hop re-runs the router's own
/// `cp_quote` (already *exact*, not an approximation, for plain
/// constant-product pools -- only CLMM pools have this specific gap) via
/// `TradeRouter::requote_edge`, so a correction to an early hop's output
/// correctly propagates through the rest of the chain instead of only
/// checking the first affected hop in isolation.
///
/// Outcome of re-verifying a cycle -- distinguishes "a specific pool's
/// exact quote disagreed badly, blame that pool" (worth a cooldown, see
/// `pricegraph::TradeRouter::mark_pool_cooldown`) from "every hop
/// re-quoted fine, but the corrected result isn't profitable enough" (not
/// any single pool's fault, don't cool anything down for it).
pub enum ReverifyOutcome<T> {
    Ok(T),
    /// The hop whose exact quote returned zero or otherwise couldn't be
    /// re-quoted.
    HopFailed(HopFailure),
    /// Only reachable for the cycle variant -- a one-way `Route` has no
    /// profitability concept to fall below. Carries the corrected cycle
    /// (every hop re-quoted fine) even though it's being rejected --
    /// 2026-09-03, added so a caller logging this rejection can show the
    /// real, corrected per-hop breakdown (dex/pool/amounts) instead of
    /// just a start_token/amount_in/hop-count summary, which was all a
    /// caller had to go on for a persistent, always-just-below-threshold
    /// cycle otherwise.
    BelowThreshold(T),
}

/// Identifies which hop's pool failed exact re-quoting, and whether that
/// pool is actually to blame.
///
/// Real, live-confirmed incident (2026-09-04): `OrcaState::exact_quote`/
/// `RaydiumClmm::exact_quote` both silently degrade a tick-array window
/// with no subscribed/decoded data yet into empty (zero-liquidity) ticks
/// -- indistinguishable, from their `None`/`Some(0)` return alone, from a
/// pool that's genuinely untradeable. A pool whose *main* account had
/// just delivered its first live update (a real, live router edge) still
/// had none of its tick arrays synced -- treating that the same as a real
/// bad quote cooled the pool down for `POOL_COOLDOWN_SLOTS` (~30-40 real
/// minutes), repeatedly locking out the only live route to a stranded
/// mint even though the actual problem was "ask again in a few more
/// seconds." `coolable` lets a caller skip `mark_pool_cooldown` for a
/// data-not-ready failure while still cooling down a genuine mismatch.
pub struct HopFailure {
    pub pool_id: AccountId,
    /// `false` when the failure is because required live data (tick
    /// arrays) hasn't arrived yet, not because the pool's real quote is
    /// genuinely bad -- see this struct's own doc comment.
    pub coolable: bool,
}

/// Returns [`ReverifyOutcome::HopFailed`] if any hop can no longer be
/// quoted at all (pool disappeared, zero liquidity) --
/// [`ReverifyOutcome::BelowThreshold`] if every hop re-quoted fine but the
/// corrected cycle no longer clears `MIN_PROFIT_BPS`. Either way the
/// caller should treat this exactly like `find_arbitrage_slippage_aware`
/// returning `None` itself (don't log a corrected-but-invalid number as a
/// found opportunity) -- `HopFailed` additionally identifies which pool
/// to cool down (see [`HopFailure`]).
pub fn reverify_with_exact_quotes(
    cycle: &ArbitrageCycle,
    router: &TradeRouter,
    dex: &DexState,
) -> ReverifyOutcome<ArbitrageCycle> {
    let corrected_hops = match reverify_hops(&cycle.hops, cycle.amount_in(), router, dex) {
        Ok(hops) => hops,
        Err(failure) => return ReverifyOutcome::HopFailed(failure),
    };
    let corrected = ArbitrageCycle { hops: corrected_hops };
    if corrected.profit_bps() < MIN_PROFIT_BPS {
        return ReverifyOutcome::BelowThreshold(corrected);
    }
    ReverifyOutcome::Ok(corrected)
}

/// Same correction as [`reverify_with_exact_quotes`], for a point-to-point
/// [`Route`] (e.g. `arbv1`'s `trade router check` diagnostic's
/// SOL->USDC probe via `route_slippage_aware`) rather than a closed
/// arbitrage cycle -- no profit gate, since a one-way route has no
/// "profit" concept to check. Returns `Err(`[`HopFailure`]`)` (not
/// `ReverifyOutcome`, since `BelowThreshold` never applies here) when a
/// hop's exact quote fails.
pub fn reverify_route_with_exact_quotes(
    route: &Route,
    amount_in: u64,
    router: &TradeRouter,
    dex: &DexState,
) -> Result<Route, HopFailure> {
    let corrected_hops = reverify_hops(&route.hops, amount_in, router, dex)?;
    Ok(Route { hops: corrected_hops })
}

/// Shared core of [`reverify_with_exact_quotes`]/
/// [`reverify_route_with_exact_quotes`] -- sequentially re-quotes every
/// hop with the corrected running amount: Orca/RaydiumClmm hops use
/// their own `exact_quote`, `SplStakePoolWithdrawSol` hops use
/// `SplStakePoolState::exact_quote` (real, live-confirmed 2026-09-04: the
/// generic edge quote below isn't bit-exact for a stake-pool withdrawal's
/// ceil()-rounded fee, and a real overestimate made the next hop's swap
/// fail on-chain with `insufficient funds`), and every other hop re-runs
/// the router's own `cp_quote` (already *exact*, not an approximation,
/// for plain constant-product pools) via `TradeRouter::requote_edge`, so
/// a correction to an early hop's output correctly propagates through
/// the rest of the chain instead of only checking the first affected hop
/// in isolation. `Err(`[`HopFailure`]`)` identifies exactly which hop's
/// pool failed to quote (or quoted zero), and whether that's actually the
/// pool's fault -- see [`HopFailure`]'s doc comment.
fn reverify_hops(hops: &[Hop], amount_in: u64, router: &TradeRouter, dex: &DexState) -> Result<Vec<Hop>, HopFailure> {
    let mut corrected_hops = Vec::with_capacity(hops.len());
    let mut amount_in = amount_in;
    for hop in hops {
        let (amount_out, ready) = if hop.dex == DexType::OrcaWhirlpool {
            (
                dex.orca_exact_quote(hop.pool_id, hop.input_mint, amount_in),
                dex.orca_exact_quote_ready(hop.pool_id, hop.input_mint),
            )
        } else if hop.dex == DexType::RaydiumClmm {
            (
                dex.raydium_clmm_exact_quote(hop.pool_id, hop.input_mint, amount_in),
                dex.raydium_clmm_exact_quote_ready(hop.pool_id, hop.input_mint),
            )
        } else if hop.dex == DexType::SplStakePoolWithdrawSol {
            // Real, live-confirmed incident (2026-09-04): unlike genuine
            // constant-product pools (where the generic `requote_edge`
            // fallback below really is exact), a stake-pool withdrawal's
            // real payout has its own ceil()-rounded fee subtracted
            // before a separate integer division -- the generic linear
            // edge quote isn't bit-exact against that, and a real
            // overestimate (even 1 lamport) makes the very next hop's
            // swap fail on-chain with a real `insufficient funds` (SPL
            // token transfers are exact-amount, no partial fill). See
            // `SplStakePoolState::exact_quote`'s doc comment for the
            // full incident and the real formula this reproduces.
            (dex.spl_stake_pool_exact_quote(hop.pool_id, hop.input_mint, amount_in), true)
        } else {
            (router.requote_edge(hop.input_mint, hop.pool_id, hop.output_mint, amount_in), true)
        };
        let amount_out = match amount_out {
            Some(v) if v > 0 => v,
            _ => return Err(HopFailure { pool_id: hop.pool_id, coolable: ready }),
        };
        corrected_hops.push(Hop {
            pool_id: hop.pool_id,
            input_mint: hop.input_mint,
            output_mint: hop.output_mint,
            amount_in,
            amount_out,
            dex: hop.dex,
        });
        amount_in = amount_out;
    }
    Ok(corrected_hops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_amount_in_takes_the_configured_fraction() {
        // 10% of 1_000_000_000 = 100_000_000, well above the floor.
        assert_eq!(real_amount_in(1_000_000_000), Some(100_000_000));
    }

    #[test]
    fn real_amount_in_floors_out_small_balances() {
        // 10% of 1000 = 100, below REAL_SIZE_MIN_RAW (1_000_000).
        assert_eq!(real_amount_in(1_000), None);
    }

    #[test]
    fn real_amount_in_zero_balance_is_none() {
        assert_eq!(real_amount_in(0), None);
    }
}
