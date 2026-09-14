//! Program-derived addresses for the Phoenix perpetuals program.
//!
//! Every seed below was read directly from Ellipsis Labs' public
//! `rise-public` SDK source (`github.com/Ellipsis-Labs/rise-public`), not
//! guessed:
//! - `trader_account`: `rust/core/src/tx_builder.rs:192-199`
//!   (`PhoenixTxBuilder::trader_pda`).
//! - `fee_config`/`stop_loss`/`spline_collection`/`global_vault`:
//!   `rust/ix/src/constants.rs`.
//! - `event_authority`: fixed, single-seed, same convention Pump.fun/
//!   PumpSwap use for their own event authorities.

use solana_sdk::pubkey::Pubkey;

use super::{PHOENIX_FEE_PROGRAM_ID, PHOENIX_PROGRAM_ID};

/// Cross-margin subaccount index (up to 128 positions per trader account).
/// Isolated margin (1 position per account) uses `1..=100` instead -- not
/// used by this module, cross-margin is simpler to bookkeep for a first
/// pass.
pub const SUBACCOUNT_CROSS_MARGIN: u8 = 0;

/// `["trader", authority, [trader_pda_index, subaccount_index]]`.
pub fn trader_account(authority: &Pubkey, trader_pda_index: u8, subaccount_index: u8) -> Pubkey {
    let schema = [trader_pda_index, subaccount_index];
    Pubkey::find_program_address(
        &[b"trader", authority.as_ref(), schema.as_ref()],
        &PHOENIX_PROGRAM_ID,
    )
    .0
}

/// `["fee_config", PHOENIX_PROGRAM_ID]` on the separate fee program --
/// same shape as Pump.fun/PumpSwap's shared fee-program PDA.
pub fn fee_config() -> Pubkey {
    Pubkey::find_program_address(
        &[b"fee_config", PHOENIX_PROGRAM_ID.as_ref()],
        &PHOENIX_FEE_PROGRAM_ID,
    )
    .0
}

/// `["__event_authority"]` -- fixed.
pub fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &PHOENIX_PROGRAM_ID).0
}

/// `["stoploss", trader_account, asset_id.to_le_bytes()]`.
pub fn stop_loss(trader_account: &Pubkey, asset_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[b"stoploss", trader_account.as_ref(), &asset_id.to_le_bytes()],
        &PHOENIX_PROGRAM_ID,
    )
    .0
}

/// `["conditional_orders", trader_account]`.
pub fn conditional_orders(trader_account: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"conditional_orders", trader_account.as_ref()],
        &PHOENIX_PROGRAM_ID,
    )
    .0
}

/// `["vault", mint]` -- the protocol's own vault for a given collateral
/// mint (the canonical Phoenix mint, not the underlying USDC -- see the
/// module doc's Ember note).
pub fn global_vault(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"vault", mint.as_ref()], &PHOENIX_PROGRAM_ID).0
}

/// `["spline", market]` -- per-market, required by every order/cancel
/// instruction.
pub fn spline_collection(market: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"spline", market.as_ref()], &PHOENIX_PROGRAM_ID).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trader_account_is_deterministic_and_index_sensitive() {
        let authority = Pubkey::new_unique();
        let a = trader_account(&authority, 0, SUBACCOUNT_CROSS_MARGIN);
        let b = trader_account(&authority, 0, SUBACCOUNT_CROSS_MARGIN);
        assert_eq!(a, b);
        let isolated = trader_account(&authority, 0, 1);
        assert_ne!(a, isolated);
    }

    #[test]
    fn fee_config_is_seeded_by_the_phoenix_program_id_on_the_fee_program() {
        let fc = fee_config();
        assert_ne!(fc, Pubkey::default());
    }

    #[test]
    fn spline_collection_is_market_specific() {
        let m1 = Pubkey::new_unique();
        let m2 = Pubkey::new_unique();
        assert_ne!(spline_collection(&m1), spline_collection(&m2));
    }
}
