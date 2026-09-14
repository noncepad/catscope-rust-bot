//! Token-2022 transfer-fee math.
//!
//! Mirrors `spl_token_2022_interface::extension::transfer_fee` byte-exactly
//! (`calculate_fee` and `calculate_pre_fee_amount`). The on-chain Token-2022
//! program runs this same arithmetic when it withholds a fee on transfer; we
//! reproduce it here so a swap simulator can route around fee-bearing mints
//! without round-tripping through the chain.
//!
//! ## Boundary
//!
//! This crate does **not** decode mint-extension TLV. The caller is
//! responsible for finding the `TransferFeeConfig` extension on a mint and
//! resolving the active [`TransferFee`] for the current epoch
//! (`older_transfer_fee` if `current_epoch < newer_transfer_fee.epoch`,
//! `newer_transfer_fee` otherwise — see
//! `spl_token_2022_interface::extension::transfer_fee::TransferFeeConfig::get_epoch_fee`).
//!
//! The [`TransferFee`] passed in here is that already-resolved fee, in the
//! same shape that consumers like `whirlpools-core` accept.
//!
//! ## Wiring into a swap
//!
//! Raydium's `swap_v2` instruction applies transfer fees on both legs
//! separately from the pool's own swap fee:
//!
//! 1. The user signs `amount_in_max`. Token-2022 withholds
//!    [`calculate_fee`] on the way in, so the pool sees only
//!    [`apply_transfer_fee`] of `amount_in_max`. The CLMM swap step (see
//!    [`crate::compute_swap_step`]) runs against that *post-fee* input.
//! 2. The pool produces `amount_out`. Token-2022 then withholds
//!    [`calculate_fee`] on the way out, so the user receives
//!    [`apply_transfer_fee`] of `amount_out`.
//!
//! For exact-out routing, invert step 2 with [`reverse_apply_transfer_fee`]
//! to size the swap so the user receives a desired post-fee amount.

use crate::error::{ErrorCode, Result};

/// Maximum transfer-fee basis points (`10_000` = `100%`).
pub const MAX_FEE_BASIS_POINTS: u16 = 10_000;
const ONE_IN_BASIS_POINTS: u128 = MAX_FEE_BASIS_POINTS as u128;

/// Resolved Token-2022 transfer fee for a single epoch.
///
/// Construct from a mint's `TransferFeeConfig` extension by selecting
/// `older_transfer_fee` or `newer_transfer_fee` based on the current epoch
/// (caller's responsibility — this crate doesn't decode mint TLV).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferFee {
    /// Fee in basis points of the transfer amount (`100 = 1%`). Capped at
    /// [`MAX_FEE_BASIS_POINTS`] by the on-chain program.
    pub transfer_fee_basis_points: u16,
    /// Absolute cap on the fee, in token base units. The fee is the lesser
    /// of `amount * basis_points / 10_000` (rounded up) and `maximum_fee`.
    pub maximum_fee: u64,
}

/// Transfer fee that Token-2022 withholds when transferring `pre_fee_amount`.
///
/// Mirrors `TransferFee::calculate_fee` in spl-token-2022.
pub fn calculate_fee(fee: &TransferFee, pre_fee_amount: u64) -> Result<u64> {
    let bps = fee.transfer_fee_basis_points as u128;
    if bps == 0 || pre_fee_amount == 0 {
        return Ok(0);
    }
    let numerator = (pre_fee_amount as u128)
        .checked_mul(bps)
        .ok_or(ErrorCode::MaxTokenOverflow)?;
    let raw_fee = ceil_div(numerator, ONE_IN_BASIS_POINTS).ok_or(ErrorCode::MaxTokenOverflow)?;
    // raw_fee <= u64::MAX: numerator <= u64::MAX * 10_000, so
    // ceil(numerator / 10_000) <= u64::MAX. spl-token-2022 marks the
    // try_into() here as "guaranteed to be okay" for the same reason.
    let raw_fee = u64::try_from(raw_fee).map_err(|_| ErrorCode::MaxTokenOverflow)?;
    Ok(core::cmp::min(raw_fee, fee.maximum_fee))
}

/// `pre_fee_amount` minus the Token-2022 transfer fee — the amount the
/// recipient actually sees on a transfer of `pre_fee_amount`.
///
/// Mirrors `TransferFee::calculate_post_fee_amount` in spl-token-2022.
pub fn apply_transfer_fee(fee: &TransferFee, pre_fee_amount: u64) -> Result<u64> {
    pre_fee_amount
        .checked_sub(calculate_fee(fee, pre_fee_amount)?)
        .ok_or(ErrorCode::MaxTokenOverflow)
}

/// Smallest `pre_fee_amount` whose Token-2022 transfer-fee deduction yields
/// the given `post_fee_amount` — used for exact-out swap routing.
///
/// Mirrors `TransferFee::calculate_pre_fee_amount` in spl-token-2022.
///
/// Returns `MaxTokenOverflow` if no `u64` `pre_fee_amount` exists (e.g.
/// `post_fee_amount` near `u64::MAX` with a positive fee).
///
/// Note the round-trip is *not* exact in general: rounding-up of the fee
/// means `calculate_fee(reverse_apply_transfer_fee(x))` can be one unit
/// less than `calculate_fee(y)` where `y - calculate_fee(y) == x`. Only the
/// inequality `calculate_fee(y) >= calculate_fee(reverse(x))` holds. This
/// matches Token-2022's documented behavior.
pub fn reverse_apply_transfer_fee(fee: &TransferFee, post_fee_amount: u64) -> Result<u64> {
    let bps = fee.transfer_fee_basis_points as u128;
    let max_fee = fee.maximum_fee;
    match (bps, post_fee_amount) {
        // No fee, identity.
        (0, _) => Ok(post_fee_amount),
        // Zero in, zero out.
        (_, 0) => Ok(0),
        // 100% fee: fee is always capped at `maximum_fee`, so
        // pre_fee = post_fee + max_fee exactly.
        (b, _) if b == ONE_IN_BASIS_POINTS => post_fee_amount
            .checked_add(max_fee)
            .ok_or(ErrorCode::MaxTokenOverflow),
        _ => {
            let numerator = (post_fee_amount as u128)
                .checked_mul(ONE_IN_BASIS_POINTS)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
            // bps < ONE_IN_BASIS_POINTS by the arms above, so this never underflows.
            let denominator = ONE_IN_BASIS_POINTS - bps;
            let raw_pre_fee =
                ceil_div(numerator, denominator).ok_or(ErrorCode::MaxTokenOverflow)?;

            // If the implied fee at `raw_pre_fee` would exceed the cap, the
            // true pre-fee is just `post_fee + max_fee` (cap is flat).
            let implied_fee = raw_pre_fee
                .checked_sub(post_fee_amount as u128)
                .ok_or(ErrorCode::MaxTokenOverflow)?;
            if implied_fee >= max_fee as u128 {
                post_fee_amount
                    .checked_add(max_fee)
                    .ok_or(ErrorCode::MaxTokenOverflow)
            } else {
                u64::try_from(raw_pre_fee).map_err(|_| ErrorCode::MaxTokenOverflow)
            }
        }
    }
}

/// `ceil(numerator / denominator)`, matching spl-token-2022's helper.
fn ceil_div(numerator: u128, denominator: u128) -> Option<u128> {
    numerator
        .checked_add(denominator)?
        .checked_sub(1)?
        .checked_div(denominator)
}

#[cfg(test)]
mod tests {
    //! Test vectors ported verbatim from
    //! `spl_token_2022_interface::extension::transfer_fee::tests`. If any of
    //! these regress, byte-exact parity with the on-chain program is broken.

    use super::*;

    fn fee(bps: u16, maximum_fee: u64) -> TransferFee {
        TransferFee {
            transfer_fee_basis_points: bps,
            maximum_fee,
        }
    }

    #[test]
    fn calculate_fee_max() {
        let one = MAX_FEE_BASIS_POINTS as u64;
        let f = fee(1, 5_000);
        let max = f.maximum_fee;
        // hits maximum fee
        assert_eq!(max, calculate_fee(&f, u64::MAX).unwrap());
        // exactly at max
        assert_eq!(max, calculate_fee(&f, max * one).unwrap());
        // one above — round-up still capped
        assert_eq!(max, calculate_fee(&f, max * one + 1).unwrap());
        // one below — rounds up to the cap
        assert_eq!(max, calculate_fee(&f, max * one - 1).unwrap());
    }

    #[test]
    fn calculate_fee_min() {
        let one = MAX_FEE_BASIS_POINTS as u64;
        let f = fee(1, 5_000);
        // round-up means even 1 token incurs the 1-unit floor
        assert_eq!(1, calculate_fee(&f, 1).unwrap());
        assert_eq!(1, calculate_fee(&f, 2).unwrap());
        assert_eq!(1, calculate_fee(&f, one).unwrap());
        // 2-unit fee at one+1
        assert_eq!(2, calculate_fee(&f, one + 1).unwrap());
        // zero is always zero
        assert_eq!(0, calculate_fee(&f, 0).unwrap());
    }

    #[test]
    fn calculate_fee_zero() {
        let one = MAX_FEE_BASIS_POINTS as u64;
        // 0 bps with any cap → 0 fee
        let f = fee(0, u64::MAX);
        assert_eq!(0, calculate_fee(&f, 0).unwrap());
        assert_eq!(0, calculate_fee(&f, u64::MAX).unwrap());
        assert_eq!(0, calculate_fee(&f, 1).unwrap());
        assert_eq!(0, calculate_fee(&f, one).unwrap());
        // 100% bps with 0 cap → 0 fee (cap dominates)
        let f = fee(MAX_FEE_BASIS_POINTS, 0);
        assert_eq!(0, calculate_fee(&f, 0).unwrap());
        assert_eq!(0, calculate_fee(&f, u64::MAX).unwrap());
        assert_eq!(0, calculate_fee(&f, 1).unwrap());
        assert_eq!(0, calculate_fee(&f, one).unwrap());
    }

    #[test]
    fn reverse_apply_max() {
        let one = MAX_FEE_BASIS_POINTS as u64;
        let f = fee(1, 5_000);
        let max = f.maximum_fee;
        // post_fee = u64::MAX - max → cap dominates → fee == max
        let pre = reverse_apply_transfer_fee(&f, u64::MAX - max).unwrap();
        assert_eq!(max, calculate_fee(&f, pre).unwrap());
        // post = max*one - max → exactly at the cap boundary
        let pre = reverse_apply_transfer_fee(&f, max * one - max).unwrap();
        assert_eq!(max, calculate_fee(&f, pre).unwrap());
        // one above
        let pre = reverse_apply_transfer_fee(&f, max * one - max + 1).unwrap();
        assert_eq!(max, calculate_fee(&f, pre).unwrap());
        // one below
        let pre = reverse_apply_transfer_fee(&f, max * one - max - 1).unwrap();
        assert_eq!(max, calculate_fee(&f, pre).unwrap());
    }

    #[test]
    fn reverse_apply_edge_cases() {
        // 100% bps with finite cap: pre = post + max_fee
        let f = fee(MAX_FEE_BASIS_POINTS, 5_000);
        assert_eq!(0, reverse_apply_transfer_fee(&f, 0).unwrap());
        assert_eq!(1 + 5_000, reverse_apply_transfer_fee(&f, 1).unwrap());
        // 0 bps: identity
        let f = fee(0, 5_000);
        assert_eq!(1, reverse_apply_transfer_fee(&f, 1).unwrap());
    }

    #[test]
    fn reverse_apply_min() {
        let one = MAX_FEE_BASIS_POINTS as u64;
        let f = fee(1, 5_000);
        // post=1: minimum 1-unit fee
        let pre = reverse_apply_transfer_fee(&f, 1).unwrap();
        assert_eq!(1, calculate_fee(&f, pre).unwrap());
        // post=2: still 1-unit fee
        let pre = reverse_apply_transfer_fee(&f, 2).unwrap();
        assert_eq!(1, calculate_fee(&f, pre).unwrap());
        // post=one-1: still 1-unit fee
        let pre = reverse_apply_transfer_fee(&f, one - 1).unwrap();
        assert_eq!(1, calculate_fee(&f, pre).unwrap());
        // post=one: 2-unit fee at this boundary
        let pre = reverse_apply_transfer_fee(&f, one).unwrap();
        assert_eq!(2, calculate_fee(&f, pre).unwrap());
        // post=0: zero
        assert_eq!(0, reverse_apply_transfer_fee(&f, 0).unwrap());
    }

    #[test]
    fn apply_transfer_fee_is_subtraction() {
        let f = fee(30, 1_000); // 0.3% with 1k cap
        for amount in [1u64, 100, 10_000, 1_000_000, u64::MAX] {
            let post = apply_transfer_fee(&f, amount).unwrap();
            let fee_amt = calculate_fee(&f, amount).unwrap();
            assert_eq!(post, amount - fee_amt);
        }
    }

    #[test]
    fn round_trip_inequality() {
        // Documented Token-2022 invariant:
        //   calculate_fee(reverse(post)) <= calculate_fee(pre) where pre
        //   is any value with apply(pre) == post. We verify on a small grid.
        for &bps in &[1u16, 30, 100, 1_000, 5_000, 9_999] {
            for &cap in &[1u64, 5_000, 1_000_000, u64::MAX] {
                let f = fee(bps, cap);
                for amount_in in [1u64, 100, 10_000, 1_000_000, 1_000_000_000_000] {
                    let fee_in = calculate_fee(&f, amount_in).unwrap();
                    let post = amount_in - fee_in;
                    let fee_inv =
                        calculate_fee(&f, reverse_apply_transfer_fee(&f, post).unwrap()).unwrap();
                    assert!(
                        fee_in >= fee_inv,
                        "bps={bps} cap={cap} amt={amount_in} fee_in={fee_in} fee_inv={fee_inv}",
                    );
                }
            }
        }
    }
}
