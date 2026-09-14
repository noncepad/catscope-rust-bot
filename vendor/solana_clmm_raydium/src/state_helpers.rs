//! Pure-arithmetic helpers that the upstream Raydium program defines as
//! static methods on `TickState` / `TickArrayState`. The methods use no
//! struct fields, so we re-host them as free functions here.

use crate::tick_math;

/// Number of ticks per tick-array account. Raydium-CLMM-wide constant.
pub const TICK_ARRAY_SIZE: i32 = 60;

/// Fee-rate denominator: fees are stored as parts-per-million.
pub const FEE_RATE_DENOMINATOR_VALUE: u32 = 1_000_000;

/// Number of ticks covered by a single tick-array at the given spacing.
#[inline]
pub fn tick_count_in_array(tick_spacing: u16) -> i32 {
    TICK_ARRAY_SIZE * i32::from(tick_spacing)
}

/// Start tick of the tick-array containing `tick_index`.
#[inline]
pub fn array_start_index_for_tick(tick_index: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = tick_count_in_array(tick_spacing);
    let mut start = tick_index / ticks_in_array;
    if tick_index < 0 && tick_index % ticks_in_array != 0 {
        start -= 1;
    }
    start * ticks_in_array
}

/// True if `tick` lies outside [`tick_math::MIN_TICK`, `tick_math::MAX_TICK`].
#[inline]
pub fn is_tick_out_of_boundary(tick: i32) -> bool {
    tick < tick_math::MIN_TICK || tick > tick_math::MAX_TICK
}

/// True if `tick_index` is the canonical start of a tick-array for `tick_spacing`.
pub fn is_valid_tick_array_start_index(tick_index: i32, tick_spacing: u16) -> bool {
    if is_tick_out_of_boundary(tick_index) {
        if tick_index > tick_math::MAX_TICK {
            return false;
        }
        let min_start_index = array_start_index_for_tick(tick_math::MIN_TICK, tick_spacing);
        return tick_index == min_start_index;
    }
    tick_index % tick_count_in_array(tick_spacing) == 0
}
