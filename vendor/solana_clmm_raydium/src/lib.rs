// Math is extracted byte-for-byte from raydium-io/raydium-clmm and we want
// to keep it diffable against upstream. We therefore turn off most clippy
// lints on the lib itself (the extracted code) — tests are linted normally.
#![allow(clippy::all)]

//! Pure-Rust, no-RPC swap math for the Raydium concentrated-liquidity AMM
//! (CLMM) on Solana.
//!
//! This crate contains the deterministic integer arithmetic that the on-chain
//! Raydium CLMM program executes — extracted unchanged into a library that has
//! no dependency on `anchor-lang`, `solana-program`, the Solana runtime, or
//! the Anchor account model. Given pre-decoded pool state and tick-array data,
//! every function here is a pure function of its inputs.
//!
//! # Scope
//!
//! - Tick ↔ sqrt-price conversions ([`get_sqrt_price_at_tick`],
//!   [`get_tick_at_sqrt_price`])
//! - Liquidity ↔ token-amount conversions
//!   ([`get_liquidity_from_amounts`], [`get_delta_amounts_signed`], …)
//! - Single-tick swap step ([`compute_swap_step`])
//! - Multi-tick swap orchestration
//!   ([`compute_swap_full`])
//! - Tick-array bitmap navigation
//!   ([`next_initialized_tick_array_start_index`])
//! - Token-2022 transfer-fee math ([`calculate_fee`],
//!   [`apply_transfer_fee`], [`reverse_apply_transfer_fee`]) — mirrors
//!   `spl_token_2022_interface::extension::transfer_fee` byte-exactly
//!
//! # Out of scope
//!
//! - Pool / tick-array account decoding (this crate takes pre-decoded state;
//!   caller flattens decoded `TickArrayState`s into the
//!   [`InitializedTick`] slice that [`compute_swap_full`] consumes)
//! - Token-2022 mint-extension TLV decoding and transfer-hook execution
//!   (caller resolves the active [`TransferFee`] and CPIs hook programs)
//! - Position fee and reward accumulation beyond `liquidity_from_amounts`
//!
//! # Provenance
//!
//! Math is extracted from
//! `raydium-io/raydium-clmm/programs/amm/src/libraries/`. The arithmetic
//! itself is byte-for-byte identical to the on-chain implementation; the only
//! changes are import paths, an internal `ErrorCode` enum that replaces
//! `anchor_lang::error::Error`, and free-function rehosting of three static
//! methods that used no struct fields.

// `core_` alias is referenced by the `construct_bignum!` macro in `big_num`.
pub use core as core_;

#[doc(hidden)]
pub mod big_num;
#[doc(hidden)]
pub mod fixed_point_64;
#[doc(hidden)]
pub mod full_math;
#[doc(hidden)]
pub mod unsafe_math;

pub mod error;
pub mod liquidity_math;
pub mod sqrt_price_math;
pub mod state_helpers;
pub mod swap_full;
pub mod swap_math;
pub mod tick_array_bit_map;
pub mod tick_math;
pub mod transfer_fee;

// ---- curated public API ----

pub use error::ErrorCode;

pub use tick_math::{
    get_sqrt_price_at_tick, get_tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MAX_TICK,
    MIN_SQRT_PRICE_X64, MIN_TICK,
};

pub use liquidity_math::{
    add_delta, cross, get_delta_amount_0_signed, get_delta_amount_0_unsigned,
    get_delta_amount_1_signed, get_delta_amount_1_unsigned, get_delta_amounts_signed,
    get_liquidity_from_amount_0, get_liquidity_from_amount_1, get_liquidity_from_amounts,
    get_liquidity_from_single_amount_0, get_liquidity_from_single_amount_1,
};

pub use sqrt_price_math::{
    get_next_sqrt_price_from_amount_0_rounding_up, get_next_sqrt_price_from_amount_1_rounding_down,
    get_next_sqrt_price_from_input, get_next_sqrt_price_from_output,
};

pub use swap_math::{compute_swap_step, SwapStep};

pub use swap_full::{compute_swap_full, InitializedTick, SwapPool, SwapResult};

pub use tick_array_bit_map::{
    check_current_tick_array_is_initialized, next_initialized_tick_array_start_index,
    PoolTickBitmap, TICK_ARRAY_BITMAP_SIZE,
};

pub use state_helpers::{
    array_start_index_for_tick, is_tick_out_of_boundary, is_valid_tick_array_start_index,
    tick_count_in_array, FEE_RATE_DENOMINATOR_VALUE, TICK_ARRAY_SIZE,
};

pub use transfer_fee::{
    apply_transfer_fee, calculate_fee, reverse_apply_transfer_fee, TransferFee,
    MAX_FEE_BASIS_POINTS,
};
