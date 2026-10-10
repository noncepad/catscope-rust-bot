//! Exact-quote re-verification for router-found routes. The router's
//! own quotes use a constant-product approximation for CLMM pools;
//! [`reverify_route_with_exact_quotes`] re-quotes every hop of a
//! [`Route`] against each dex's exact on-chain math before a caller
//! trusts it. Does **not** build or send any transaction.

use crate::{
    graph::AccountId,
    trader::{
        dex::DexState,
        pricegraph::{Hop, Route, TradeRouter},
        types::DexType,
    },
};

/// How long to exclude a pool that failed exact-quote re-verification
/// from the slippage-aware search
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

/// Re-quotes every hop of a point-to-point [`Route`] against exact
/// on-chain math (see [`reverify_hops`]). Returns `Err(`[`HopFailure`]`)`
/// when a hop's exact quote fails.
pub fn reverify_route_with_exact_quotes(
    route: &Route,
    amount_in: u64,
    router: &TradeRouter,
    dex: &DexState,
) -> Result<Route, HopFailure> {
    let corrected_hops = reverify_hops(&route.hops, amount_in, router, dex)?;
    Ok(Route { hops: corrected_hops })
}

/// Core of [`reverify_route_with_exact_quotes`] -- sequentially re-quotes every
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
