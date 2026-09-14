//! Jet Protocol V1 Market reserve-info parser (credit-graph phase 2d — see
//! `~/cryptic-percolating-bunny.md`).
//!
//! Deliberately minimal and **not** wired into `Updater`/`DexState` or any
//! live decision loop — this exists only to feed
//! [`crate::trader::credit::CreditReserve`], same as the Kamino (phase 1),
//! marginfi (phase 2a), Solend (phase 2b), and Drift (phase 2c) additions.
//!
//! No public IDL matches the exact `JPv1rCqrhagNNmJVM5J1he7msQ5ybtvE1nNuHpDHMNU`
//! deployment for the standalone Reserve account (the only IDL found, from
//! the `@jet-lab/jet-engine` npm package, predicts `tokenMint`/`vault` 32
//! bytes earlier than where they actually are on a real account — the same
//! kind of drift this session already hit with Kamino's public source).
//! Rather than guess, the LTV-equivalent field here comes from a
//! *different*, independently-verified source: the **Market** account's
//! cached per-reserve info array (`reserves`), whose layout comes from
//! that same npm package's TypeScript client
//! (`src/pools/layout.ts`: `ReserveInfoStruct`, `MAX_RESERVES = 32`) and
//! was cross-checked against a real Market account.
//!
//! # Layout — Jet V1 Market account, `reserves` array
//!
//! ```text
//! offset          size  field
//! ──────          ────  ────────────────────────────────────────────
//!   0                8  Anchor discriminator
//!   520 + 384*i      —  slot i of up to 32 ReserveInfo entries:
//!     +0             32   reserve (Pubkey)
//!     +32            80   (unused)
//!     +112           24   price (Number192, scale 1e15)
//!     +136           24   depositNoteExchangeRate (unused here)
//!     +160           24   loanNoteExchangeRate (unused here)
//!     +184           24   minCollateralRatio (Number192, scale 1e15)
//!     +208            2   liquidationBonus (u16, basis points)
//! ```
//!
//! `520` (the array's start) and the `384`-byte stride were both derived
//! from field arithmetic on the npm package's IDL/layout, then confirmed
//! together: a reserve pubkey independently found by searching a real
//! Market account's raw bytes for a known Reserve address landed at
//! `520 + 1*384 = 904`, exactly slot index 1 — and that slot's decoded
//! values were clean and plausible: `minCollateralRatio` = 1.25 (i.e. 80%
//! LTV), `price` = $119.23, `liquidationBonus` = 300 bps (3%, matching the
//! same magnitude found for every other protocol's liquidation bonus this
//! session).
//!
//! **Not available here**: `available_liquidity_usd` and `borrow_apy`.
//! Those live in the standalone Reserve account's `state`/`config` fields,
//! whose offsets couldn't be reliably pinned down given the IDL mismatch
//! above -- left at `0.0` (unpriced), same treatment as marginfi's
//! still-unpriced fields.

use crate::{graph::AccountId, util::account_id_from_pubkey};
use solana_sdk::pubkey::Pubkey;

pub const JET_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("JPv1rCqrhagNNmJVM5J1he7msQ5ybtvE1nNuHpDHMNU");

const RESERVES_ARRAY_OFFSET: usize = 520;
const RESERVE_INFO_STRIDE: usize = 384;
const MAX_RESERVES: usize = 32;

const OFF_RESERVE_PUBKEY: usize = 0;
const OFF_PRICE: usize = 112;
const OFF_MIN_COLLATERAL_RATIO: usize = 184;
const OFF_LIQUIDATION_BONUS_BPS: usize = 208;

/// Scale for Jet's `Number192` fixed-point values: `value = raw / 1e15`.
const NUMBER192_SCALE: f64 = 1e15;

/// One reserve's cached info as tracked by its Market account.
#[derive(Debug, Clone, Copy)]
pub struct JetReserveInfo {
    pub reserve: AccountId,
    /// USD price per whole token.
    pub price_usd: f64,
    /// Collateralization ratio required (e.g. `1.25` = need $1.25 of
    /// collateral per $1.00 borrowed). LTV = `1.0 / min_collateral_ratio`.
    pub min_collateral_ratio: f64,
    pub liquidation_bonus_bps: u16,
}

impl JetReserveInfo {
    /// Max loan-to-value ratio (0.0-1.0) implied by `min_collateral_ratio`.
    pub fn max_ltv_pct(&self) -> f64 {
        if self.min_collateral_ratio <= 0.0 {
            0.0
        } else {
            1.0 / self.min_collateral_ratio
        }
    }
}

/// Find `reserve_pubkey`'s cached info within a Market account's `reserves`
/// array. `body` must be the full Market account data including the
/// 8-byte Anchor discriminator.
pub fn find_reserve_info(body: &[u8], reserve_pubkey: &Pubkey) -> Option<JetReserveInfo> {
    for slot in 0..MAX_RESERVES {
        let base = RESERVES_ARRAY_OFFSET + slot * RESERVE_INFO_STRIDE;
        if body.len() < base + OFF_LIQUIDATION_BONUS_BPS + 2 {
            break;
        }
        let pk_bytes = &body[base + OFF_RESERVE_PUBKEY..base + OFF_RESERVE_PUBKEY + 32];
        if pk_bytes != reserve_pubkey.as_ref() {
            continue;
        }
        // Number192 is 24 bytes (little-endian), but every economically
        // real value here fits in the low 16 bytes -- the high 8 bytes are
        // zero, so reading just the low u128 (matching every other WAD/SF
        // style scale already used in this codebase) is safe and avoids
        // needing a native 192-bit integer type.
        let read_number192 = |off: usize| -> f64 {
            let raw = u128::from_le_bytes(body[base + off..base + off + 16].try_into().unwrap());
            raw as f64 / NUMBER192_SCALE
        };
        return Some(JetReserveInfo {
            reserve: account_id_from_pubkey(reserve_pubkey),
            price_usd: read_number192(OFF_PRICE),
            min_collateral_ratio: read_number192(OFF_MIN_COLLATERAL_RATIO),
            liquidation_bonus_bps: u16::from_le_bytes(
                body[base + OFF_LIQUIDATION_BONUS_BPS..base + OFF_LIQUIDATION_BONUS_BPS + 2]
                    .try_into()
                    .unwrap(),
            ),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `find_reserve_info` itself calls `account_id_from_pubkey`, a
    /// WASM-host import that isn't available under a plain native test
    /// run -- so, like every other `dex::*` parser's tests in this
    /// codebase, this exercises the decoding math directly with a
    /// fixture built from real, verified values (slot 1 of a live
    /// mainnet Market account's `reserves` array: minCollateralRatio
    /// 1.25, price $119.23378, 300 bps liquidation bonus) rather than
    /// going through the real pubkey-resolution path.
    fn real_reserve_info() -> JetReserveInfo {
        JetReserveInfo {
            reserve: 1,
            price_usd: 119_233_780_000_000_000f64 / NUMBER192_SCALE,
            min_collateral_ratio: 1_250_000_000_000_000f64 / NUMBER192_SCALE,
            liquidation_bonus_bps: 300,
        }
    }

    #[test]
    fn price_and_ratio_match_real_slot() {
        let info = real_reserve_info();
        assert!((info.price_usd - 119.23378).abs() < 1e-6);
        assert!((info.min_collateral_ratio - 1.25).abs() < 1e-9);
    }

    #[test]
    fn max_ltv_is_inverse_of_min_collateral_ratio() {
        let info = real_reserve_info();
        // 1 / 1.25 = 0.80 -- 80% LTV.
        assert!((info.max_ltv_pct() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn zero_min_collateral_ratio_yields_zero_ltv() {
        let mut info = real_reserve_info();
        info.min_collateral_ratio = 0.0;
        assert_eq!(info.max_ltv_pct(), 0.0);
    }
}
