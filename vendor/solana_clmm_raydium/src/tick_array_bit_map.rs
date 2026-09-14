///! Helper functions to get most and least significant non-zero bits
use super::big_num::U1024;
use crate::err;
use crate::error::{ErrorCode, Result};
use crate::state_helpers::{
    is_tick_out_of_boundary, is_valid_tick_array_start_index, tick_count_in_array, TICK_ARRAY_SIZE,
};

pub const TICK_ARRAY_BITMAP_SIZE: i32 = 512;

pub type TickArryBitmap = [u64; 8];

/// Pool's tick-array bitmap as 1024 bits packed into 16 little-endian u64
/// limbs. Bit `i` lives in `limbs[i / 64]` at position `i % 64`. Bit 512
/// represents `tick_array_start_index = 0`; bit `512 + k` represents
/// `start_index = +k * tick_spacing * TICK_ARRAY_SIZE`; bit `512 - k`
/// represents the corresponding negative.
///
/// Decode from `pool.tick_array_bitmap` (raw `[u64; 16]` on-chain). This
/// type's only purpose is to keep the internal `U1024` big-int out of
/// the public API surface (issue #4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolTickBitmap(pub [u64; 16]);

impl PoolTickBitmap {
    /// All zeros — no tick-arrays initialized.
    pub const EMPTY: Self = Self([0u64; 16]);

    /// Construct from raw on-chain limb layout.
    pub const fn new(limbs: [u64; 16]) -> Self {
        Self(limbs)
    }

    /// Borrow the underlying limbs.
    pub const fn limbs(&self) -> &[u64; 16] {
        &self.0
    }
}

impl From<[u64; 16]> for PoolTickBitmap {
    fn from(limbs: [u64; 16]) -> Self {
        Self(limbs)
    }
}

pub fn max_tick_in_tickarray_bitmap(tick_spacing: u16) -> i32 {
    i32::from(tick_spacing) * TICK_ARRAY_SIZE * TICK_ARRAY_BITMAP_SIZE
}

pub fn get_bitmap_tick_boundary(tick_array_start_index: i32, tick_spacing: u16) -> (i32, i32) {
    let ticks_in_one_bitmap: i32 = max_tick_in_tickarray_bitmap(tick_spacing);
    let mut m = tick_array_start_index.abs() / ticks_in_one_bitmap;
    if tick_array_start_index < 0 && tick_array_start_index.abs() % ticks_in_one_bitmap != 0 {
        m += 1;
    }
    let min_value: i32 = ticks_in_one_bitmap * m;
    if tick_array_start_index < 0 {
        (-min_value, -min_value + ticks_in_one_bitmap)
    } else {
        (min_value, min_value + ticks_in_one_bitmap)
    }
}

// Demoted to crate-private: these take `U1024` and would re-leak it
// otherwise (#6). Used internally by `next_initialized_tick_array_start_index`.
pub(crate) fn most_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(u16::try_from(x.leading_zeros()).unwrap())
    }
}

pub(crate) fn least_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(u16::try_from(x.trailing_zeros()).unwrap())
    }
}

/// Given a tick, calculate whether the tickarray it belongs to has been initialized.
/// Note: The caller of the function should ensure that tick_current is within the range represented by bit_map.
/// Currently, this function is only called when `bit_map = pool.tick_array_bitmap`.
pub fn check_current_tick_array_is_initialized(
    bit_map: &PoolTickBitmap,
    tick_current: i32,
    tick_spacing: u16,
) -> Result<(bool, i32)> {
    let bit_map = U1024(bit_map.0);
    if is_tick_out_of_boundary(tick_current) {
        return err!(ErrorCode::InvalidTickIndex);
    }
    let multiplier = i32::from(tick_spacing) * TICK_ARRAY_SIZE;
    let mut compressed = tick_current / multiplier + 512;
    if tick_current < 0 && tick_current % multiplier != 0 {
        // round towards negative infinity
        compressed -= 1;
    }
    let bit_pos = compressed.abs();
    // set current bit
    let mask = U1024::one() << bit_pos.try_into().unwrap();
    let masked = bit_map & mask;
    // check the current bit whether initialized
    let initialized = masked != U1024::default();
    if initialized {
        return Ok((true, (compressed - 512) * multiplier));
    }
    // the current bit is not initialized
    return Ok((false, (compressed - 512) * multiplier));
}

/// The function is only called when `bit_map = pool.tick_array_bitmap`.
pub fn next_initialized_tick_array_start_index(
    bit_map: &PoolTickBitmap,
    last_tick_array_start_index: i32,
    tick_spacing: u16,
    zero_for_one: bool,
) -> (bool, i32) {
    let bit_map = U1024(bit_map.0);
    assert!(is_valid_tick_array_start_index(
        last_tick_array_start_index,
        tick_spacing
    ));
    let tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
    let next_tick_array_start_index = if zero_for_one {
        last_tick_array_start_index - tick_count_in_array(tick_spacing)
    } else {
        last_tick_array_start_index + tick_count_in_array(tick_spacing)
    };

    if next_tick_array_start_index < -tick_boundary || next_tick_array_start_index >= tick_boundary
    {
        return (false, last_tick_array_start_index);
    }

    let multiplier = i32::from(tick_spacing) * TICK_ARRAY_SIZE;
    let mut compressed = next_tick_array_start_index / multiplier + 512;
    if next_tick_array_start_index < 0 && next_tick_array_start_index % multiplier != 0 {
        // round towards negative infinity
        compressed -= 1;
    }
    let bit_pos = compressed.abs();
    if zero_for_one {
        // tick from upper to lower
        // find from highter bits to lower bits
        let offset_bit_map = bit_map << (1024 - bit_pos - 1).try_into().unwrap();
        let next_bit = most_significant_bit(offset_bit_map);
        if next_bit.is_some() {
            let next_array_start_index =
                (bit_pos - i32::from(next_bit.unwrap()) - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // not found til to the end
            (false, -tick_boundary)
        }
    } else {
        // tick from lower to upper
        // find from lower bits to highter bits
        let offset_bit_map = bit_map >> (bit_pos).try_into().unwrap();
        let next_bit = least_significant_bit(offset_bit_map);
        if next_bit.is_some() {
            let next_array_start_index =
                (bit_pos + i32::from(next_bit.unwrap()) - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // not found til to the end
            (false, tick_boundary - tick_count_in_array(tick_spacing))
        }
    }
}
