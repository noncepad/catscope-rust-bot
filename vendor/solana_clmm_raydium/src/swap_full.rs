//! Multi-tick swap orchestrator.
//!
//! [`compute_swap_full`] is the "one call to swap" function: given pre-decoded
//! pool state and a sorted flat view of the pool's initialized ticks, it
//! walks tick arrays direction-aware, calls [`compute_swap_step`] for each
//! range, applies tick crossings via [`cross`], and accumulates the result.
//!
//! This is the orchestration layer that every v0.1 consumer had to write
//! themselves. It mirrors `raydium-clmm`'s on-chain
//! `instructions/swap.rs::swap_internal` loop and is byte-exact against
//! all 17 mainnet replay fixtures.
//!
//! # Boundary
//!
//! This crate does not decode pool or tick-array accounts. The caller is
//! responsible for:
//!
//! - Decoding their `PoolState` into [`SwapPool`].
//! - Flattening every initialized tick across the pool's tick arrays into
//!   a single ascending-by-tick `&[InitializedTick]` slice.
//!
//! For Token-2022 pools, wrap the call with
//! [`apply_transfer_fee`](crate::apply_transfer_fee) on input and output
//! — see the README's Token-2022 section.

use crate::error::{ErrorCode, Result};
use crate::liquidity_math::cross;
use crate::swap_math::compute_swap_step;
use crate::tick_math::{
    get_sqrt_price_at_tick, get_tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MAX_TICK,
    MIN_SQRT_PRICE_X64, MIN_TICK,
};

/// Pre-decoded pool fields the swap math needs. Caller fills these from
/// their decoded `PoolState`.
#[derive(Clone, Copy, Debug)]
pub struct SwapPool {
    /// Current sqrt price (Q64.64).
    pub sqrt_price_x64: u128,
    /// Current active liquidity.
    pub liquidity: u128,
    /// Current tick.
    pub tick_current: i32,
    /// Tick spacing (e.g. 1, 10, 60).
    pub tick_spacing: u16,
    /// Pool's fee in pips (parts per million). For Raydium CLMM, this comes
    /// from `fee_rate` in the pool's `AmmConfig`.
    pub fee_rate_pips: u32,
}

/// One initialized tick from the pool's tick arrays. Caller produces a
/// sorted (ascending by `tick`) slice of these from their decoded
/// `TickArrayState`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitializedTick {
    pub tick: i32,
    pub liquidity_net: i128,
}

/// Result of a multi-tick swap.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapResult {
    /// Gross input the pool consumed (= what the user paid, before transfer fees).
    /// Includes pool fees.
    pub amount_in: u64,
    /// Gross output the pool produced (= what the user receives, before transfer fees).
    pub amount_out: u64,
    /// Total pool fees accumulated across all steps.
    pub fee_amount: u64,
    /// Sqrt price after the swap settled.
    pub final_sqrt_price_x64: u128,
    /// Tick after the swap settled.
    pub final_tick: i32,
    /// Active liquidity after the swap settled.
    pub final_liquidity: u128,
    /// Number of `compute_swap_step` iterations executed (tick-arrays
    /// touched). Useful for cost estimation and debugging.
    pub steps: u32,
}

/// Hard cap on swap iterations. A real CLMM swap never traverses more than
/// a few dozen tick crossings even at the extremes; hitting this limit
/// indicates a logic bug rather than an extreme-but-valid swap.
const MAX_STEPS: u32 = 256;

/// Execute a full multi-tick swap.
///
/// `initialized_ticks` MUST be sorted ascending by `tick` and MUST contain
/// every initialized tick that the swap could cross. Typically the caller
/// flattens all of `pool.tick_array_bitmap`'s active arrays into one slice;
/// for a constrained limit the caller can pre-filter.
///
/// `sqrt_price_limit_x64 = 0` means "no limit" (= ±1 inside the absolute
/// domain bound, matching the on-chain remap).
pub fn compute_swap_full(
    pool: &SwapPool,
    initialized_ticks: &[InitializedTick],
    amount_specified: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
    zero_for_one: bool,
) -> Result<SwapResult> {
    let mut sp_current = pool.sqrt_price_x64;
    let mut tick_current = pool.tick_current;
    let mut liquidity = pool.liquidity;
    let mut amount_remaining = amount_specified;
    let mut amount_calculated: u64 = 0;
    let mut fee_total: u64 = 0;

    // Resolve user limit: 0 → ±1 inside the domain bound (matches on-chain remap).
    let sp_limit = if sqrt_price_limit_x64 == 0 {
        if zero_for_one {
            MIN_SQRT_PRICE_X64 + 1
        } else {
            MAX_SQRT_PRICE_X64 - 1
        }
    } else {
        sqrt_price_limit_x64
    };

    let mut steps: u32 = 0;
    while amount_remaining > 0 && sp_current != sp_limit {
        if steps >= MAX_STEPS {
            return Err(ErrorCode::SqrtPriceLimitOverflow);
        }
        steps += 1;

        // Next initialized tick in the swap direction. If none in the
        // provided slice, fall back to the domain bound (the swap will hit
        // sp_limit before reaching it).
        let next_tick_opt: Option<i32> = if zero_for_one {
            // Largest initialized tick <= tick_current (on-chain uses `<=`
            // for the first cross-search; mirror that exactly).
            initialized_ticks
                .iter()
                .rev()
                .find(|t| t.tick <= tick_current)
                .map(|t| t.tick)
        } else {
            // Smallest initialized tick > tick_current.
            initialized_ticks
                .iter()
                .find(|t| t.tick > tick_current)
                .map(|t| t.tick)
        };
        let next_tick = next_tick_opt.unwrap_or(if zero_for_one { MIN_TICK } else { MAX_TICK });
        let next_tick = next_tick.clamp(MIN_TICK, MAX_TICK);
        let sp_next_tick = get_sqrt_price_at_tick(next_tick)?;

        // Target = whichever of sp_next_tick or sp_limit is hit first.
        let target_price = if zero_for_one {
            // moving down: closer-to-current = higher = max(sp_next_tick, sp_limit)
            sp_next_tick.max(sp_limit)
        } else {
            // moving up: closer-to-current = lower = min(sp_next_tick, sp_limit)
            sp_next_tick.min(sp_limit)
        };

        let step = compute_swap_step(
            sp_current,
            target_price,
            liquidity,
            amount_remaining,
            pool.fee_rate_pips,
            is_base_input,
            zero_for_one,
        )?;

        sp_current = step.sqrt_price_next_x64;
        fee_total = fee_total
            .checked_add(step.fee_amount)
            .ok_or(ErrorCode::MaxTokenOverflow)?;
        if is_base_input {
            amount_remaining = amount_remaining
                .checked_sub(step.amount_in + step.fee_amount)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
            amount_calculated = amount_calculated
                .checked_add(step.amount_out)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
        } else {
            amount_remaining = amount_remaining
                .checked_sub(step.amount_out)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
            amount_calculated = amount_calculated
                .checked_add(step.amount_in + step.fee_amount)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
        }

        // Did we hit the next initialized tick exactly? If so, cross it.
        if sp_current == target_price && target_price == sp_next_tick {
            let crossed = initialized_ticks
                .iter()
                .find(|t| t.tick == next_tick)
                .copied()
                .ok_or(ErrorCode::InvalidTickIndex)?;
            liquidity = cross(liquidity, crossed.liquidity_net, zero_for_one)?;
            tick_current = if zero_for_one {
                next_tick - 1
            } else {
                next_tick
            };
        } else if sp_current == target_price && target_price == sp_limit {
            break;
        } else {
            tick_current = get_tick_at_sqrt_price(sp_current).unwrap_or(tick_current);
        }
    }

    let consumed = amount_specified - amount_remaining;
    let (amount_in, amount_out) = if is_base_input {
        (consumed, amount_calculated)
    } else {
        (amount_calculated, consumed)
    };

    Ok(SwapResult {
        amount_in,
        amount_out,
        fee_amount: fee_total,
        final_sqrt_price_x64: sp_current,
        final_tick: tick_current,
        final_liquidity: liquidity,
        steps,
    })
}
