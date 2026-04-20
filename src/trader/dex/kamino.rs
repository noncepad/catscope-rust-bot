//! Kamino Lending reserve account parser (pricing only).
//!
//! Kamino does not expose a simple AMM swap; this module reads the
//! `market_price_sf` field from a Kamino Lending reserve account so that
//! the trader can obtain the oracle-sourced USD price for any supported token.
//!
//! # Account layout — Kamino Lending Reserve (Anchor, 8-byte discriminator)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        8   Anchor discriminator
//!   8        8   version (u64)
//!  16        8   last_update.slot (u64)
//!  24        1   last_update.stale (u8)
//!  25        7   last_update.placeholder ([u8; 7])
//!  32       32   lending_market (Pubkey)
//!  64       32   farm_collateral (Pubkey)
//!  96       32   farm_debt (Pubkey)
//! 128       32   liquidity.mint_pubkey (Pubkey)        ← token mint
//! 160       32   liquidity.supply_vault (Pubkey)
//! 192       32   liquidity.fee_vault (Pubkey)
//! 224       32   liquidity.auth_pubkey (Pubkey)
//! 256        8   liquidity.available_amount (u64)      ← liquid supply
//! 264       16   liquidity.borrowed_amount_sf (u128)
//! 280       16   liquidity.market_price_sf (u128)      ← oracle price (SF)
//! 296        8   liquidity.market_price_last_updated_ts (u64)
//! 304        8   liquidity.mint_decimals (u64)
//! ```
//!
//! # `market_price_sf` encoding
//!
//! Prices are stored in Kamino's "Scaled Fraction" (SF) format:
//!
//! ```text
//! actual_price_usd = market_price_sf as f64 / 2^60
//! ```
//!
//! For example, SOL at $150 USD is stored as approximately
//! `150 * 2^60 ≈ 1.73 × 10^20`.

use crate::{
    graph::AccountId,
    trader::types::PoolPrice,
    util::account_id_from_pubkey,
};
use solana_sdk::pubkey::Pubkey;

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const KAMINO_LENDING_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");

// ─── Reserve account offsets ──────────────────────────────────────────────────

const OFF_MINT: usize = 128;
const OFF_AVAILABLE_AMOUNT: usize = 256;
const OFF_MARKET_PRICE_SF: usize = 280;
const OFF_MINT_DECIMALS: usize = 304;

const MIN_RESERVE_LEN: usize = OFF_MINT_DECIMALS + 8;

/// Scaling factor for Kamino SF prices: `2^60`.
const SF_SCALE: f64 = (1u128 << 60) as f64;

// ─── Parsed reserve state ─────────────────────────────────────────────────────

/// Parsed Kamino Lending reserve account.
///
/// Used for reading the oracle price of a token. No swap instructions are
/// generated; pair this with another DEX pool for actual trade execution.
#[derive(Debug, Clone)]
pub struct KaminoReserve {
    /// Mint of the token this reserve represents.
    pub token_mint: AccountId,
    /// Amount of the token currently available for borrowing (raw units).
    pub available_amount: u64,
    /// Oracle market price in USD, converted from SF format.
    /// `price_usd = market_price_sf / 2^60`
    pub price_usd: f64,
    /// Token decimal places as reported in the reserve.
    pub mint_decimals: u64,
}

impl KaminoReserve {
    /// Build a synthetic [`PoolPrice`] denominated in USD.
    ///
    /// `token_b` is set to `0` (USDC / USD placeholder) and `price` is the
    /// oracle USD price per raw token unit (before decimal adjustment).
    /// Multiply `price` by `10^-mint_decimals` to obtain the human price.
    pub fn pool_price(&self) -> PoolPrice {
        PoolPrice {
            token_a: self.token_mint,
            token_b: 0,     // USD / USDC placeholder
            price: self.price_usd,
            reserve_a: self.available_amount,
            reserve_b: 0,
            fee_bps: 0,
        }
    }
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a Kamino Lending reserve account from raw body bytes.
///
/// `body` must include the Anchor discriminator at offset 0.
///
/// Returns `None` if `body` is too short.
pub fn parse(body: &[u8]) -> Option<KaminoReserve> {
    if body.len() < MIN_RESERVE_LEN {
        return None;
    }

    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
    let read_pk = |off: usize| -> Pubkey {
        Pubkey::new_from_array(body[off..off + 32].try_into().unwrap())
    };

    let market_price_sf = read_u128(OFF_MARKET_PRICE_SF);
    let price_usd = market_price_sf as f64 / SF_SCALE;

    Some(KaminoReserve {
        token_mint: account_id_from_pubkey(&read_pk(OFF_MINT)),
        available_amount: read_u64(OFF_AVAILABLE_AMOUNT),
        price_usd,
        mint_decimals: read_u64(OFF_MINT_DECIMALS),
    })
}
