//! Instruction builders for `register_trader`, `deposit_funds`,
//! `withdraw_funds`, `place_limit_order`, `place_market_order`, and
//! `cancel_orders_by_id`.
//!
//! Account lists and data encodings below were read directly from
//! Ellipsis Labs' public `rise-public` SDK source
//! (`rust/ix/src/{register_trader,deposit_funds,withdraw_funds,limit_order,market_order,cancel_orders,order_packet,types}.rs`),
//! not guessed. Instruction discriminants are
//! `sha256("global:<snake_case_name>")[..8]` (confirmed from
//! `rust/ix/src/discriminants.rs`'s `define_instruction_discriminants!`
//! macro -- the exact same Anchor-sighash convention Pump.fun/PumpSwap use
//! elsewhere in this codebase), recomputed and asserted in this module's
//! tests rather than trusted blindly.
//!
//! Order-packet data (`place_limit_order`/`place_market_order`) is Borsh-
//! encoded by hand here (field order and enum discriminants copied
//! directly from `OrderPacketKind`/`Side`/`SelfTradeBehavior`/`OrderFlags`
//! in `order_packet.rs`/`types.rs`, cross-checked against that file's own
//! unit tests, e.g. `Limit`'s enum discriminant byte is asserted `1` and
//! `ImmediateOrCancel`'s is asserted `2` there) rather than depending on
//! the `phoenix-rise-ix`/`borsh` crates directly -- this codebase never
//! vendors an on-chain-program's own crate, it hand-parses/hand-encodes,
//! matching every other dex module's convention.
//!
//! `global_trader_index`/`active_trader_buffer` remaining accounts use
//! [`PhoenixState::global_trader_index_accounts`]/
//! [`PhoenixState::active_trader_buffer_accounts`], which assert the
//! verified-live `num_arenas == 1` invariant rather than silently
//! constructing a wrong instruction if the exchange ever scales past it.

use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{
    pda, GLOBAL_CONFIGURATION_PK, PHOENIX_LOG_AUTHORITY, PHOENIX_PROGRAM_ID, PhoenixState,
    SPL_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID,
};
use crate::{
    graph::AccountId,
    trader::types::TraderError,
    util::pubkey_from_account_id,
    wallet::Wallet,
};

pub const REGISTER_TRADER_CU: u32 = 40_000;
pub const DEPOSIT_FUNDS_CU: u32 = 40_000;
pub const WITHDRAW_FUNDS_CU: u32 = 50_000;
pub const PLACE_LIMIT_ORDER_CU: u32 = 120_000;
pub const PLACE_MARKET_ORDER_CU: u32 = 150_000;
pub const CANCEL_ORDERS_BY_ID_CU: u32 = 60_000;

/// `sha256("global:register_trader")[..8]`.
const DISC_REGISTER_TRADER: [u8; 8] = [75, 243, 224, 167, 1, 5, 51, 32];
/// `sha256("global:deposit_funds")[..8]`.
const DISC_DEPOSIT_FUNDS: [u8; 8] = [202, 39, 52, 211, 53, 20, 250, 88];
/// `sha256("global:withdraw_funds")[..8]`.
const DISC_WITHDRAW_FUNDS: [u8; 8] = [241, 36, 29, 111, 208, 31, 104, 217];
/// `sha256("global:place_limit_order")[..8]`.
const DISC_PLACE_LIMIT_ORDER: [u8; 8] = [108, 176, 33, 186, 146, 229, 1, 197];
/// `sha256("global:place_market_order")[..8]`.
const DISC_PLACE_MARKET_ORDER: [u8; 8] = [90, 118, 192, 252, 192, 99, 39, 145];
/// `sha256("global:cancel_orders_by_id")[..8]`.
const DISC_CANCEL_ORDERS_BY_ID: [u8; 8] = [234, 204, 126, 94, 222, 22, 141, 24];

/// Mirrors `phoenix_rise_math::Side` (not depended on directly -- see
/// module doc). `Bid = 0`/`Ask = 1`, confirmed from `order_packet.rs`'s own
/// serialization tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

/// Mirrors `phoenix_rise_ix::types::SelfTradeBehavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SelfTradeBehavior {
    Abort = 0,
    CancelProvide = 1,
    DecrementTake = 2,
}

/// Mirrors `phoenix_rise_ix::types::OrderFlags` (`#[borsh(use_discriminant = true)]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderFlags {
    None = 0,
    ReduceOnly = 128,
}

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

fn push_option_u64(data: &mut Vec<u8>, v: Option<u64>) {
    match v {
        None => data.push(0),
        Some(x) => {
            data.push(1);
            data.extend_from_slice(&x.to_le_bytes());
        }
    }
}

impl PhoenixState {
    /// Build a `register_trader` instruction (cross-margin, up to 128
    /// positions) and append it to `wallet`. Call once, before any other
    /// instruction on a fresh authority -- must run after
    /// [`PhoenixState::set_authority`].
    pub fn register_trader(&self, payer: AccountId, wallet: &mut Wallet) -> Result<(), TraderError> {
        let (_, trader_account_pk) = self.trader_account().ok_or(TraderError::MissingConfig("trader_account"))?;
        // Real, live-confirmed bug fixed this session: this used to
        // `resolve(trader_id)` -- `trader_id` is the *trader PDA's own*
        // account id (from `trader_account()`, used only for subscribing
        // to that PDA's on-chain data), which resolves right back to
        // `trader_account_pk` itself. That put the trader PDA in both the
        // `authority` and `trader_account` account slots instead of the
        // real wallet authority pubkey `set_authority` already stored --
        // confirmed live: a real `register_trader` transaction sent with
        // this bug failed on-chain with "invalid account data for
        // instruction", and the referenced pubkey in the `authority` slot
        // didn't even exist as a real account. `o_authority_pk` (set by
        // `set_authority`, the real wallet authority) is the correct
        // value for this slot.
        let authority_pk = self.o_authority_pk.ok_or(TraderError::MissingConfig("authority"))?;
        let payer_pk = resolve(payer)?;

        let mut data = Vec::with_capacity(18);
        data.extend_from_slice(&DISC_REGISTER_TRADER);
        data.extend_from_slice(&128u32.to_le_bytes()); // max_positions
        data.extend_from_slice(&0u32.to_le_bytes()); // trader_preference_bits
        data.push(0); // trader_pda_index
        data.push(pda::SUBACCOUNT_CROSS_MARGIN);

        let accounts = vec![
            AccountMeta::new_readonly(PHOENIX_PROGRAM_ID, false),
            AccountMeta::new_readonly(PHOENIX_LOG_AUTHORITY, false),
            AccountMeta::new_readonly(GLOBAL_CONFIGURATION_PK, false),
            AccountMeta::new(payer_pk, true),
            AccountMeta::new_readonly(authority_pk, false),
            AccountMeta::new(trader_account_pk, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ];

        wallet.require_signer(payer);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, REGISTER_TRADER_CU);
        Ok(())
    }

    /// Build a `deposit_funds` instruction (collateral -> margin account)
    /// and append it to `wallet`. `trader_token_account` must already hold
    /// the canonical Phoenix mint ([`PhoenixState::canonical_mint`]) -- see
    /// the module doc's Ember note, this builder does not convert USDC.
    pub fn deposit_funds(
        &self,
        authority: AccountId,
        trader_token_account: AccountId,
        amount: u64,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (_, trader_account_pk) = self.trader_account().ok_or(TraderError::MissingConfig("trader_account"))?;
        if !self.global_ready() {
            return Err(TraderError::PoolNotReady);
        }
        let authority_pk = resolve(authority)?;
        let token_account_pk = resolve(trader_token_account)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_DEPOSIT_FUNDS);
        data.extend_from_slice(&amount.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new_readonly(PHOENIX_PROGRAM_ID, false),
            AccountMeta::new_readonly(PHOENIX_LOG_AUTHORITY, false),
            AccountMeta::new(GLOBAL_CONFIGURATION_PK, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(token_account_pk, false),
            AccountMeta::new(trader_account_pk, false),
            AccountMeta::new(self.global_vault_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        for pk in self.global_trader_index_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }
        for pk in self.active_trader_buffer_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }

        wallet.require_signer(authority);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, DEPOSIT_FUNDS_CU);
        Ok(())
    }

    /// Build a `withdraw_funds` instruction (margin account -> collateral)
    /// and append it to `wallet`.
    pub fn withdraw_funds(
        &self,
        authority: AccountId,
        trader_token_account: AccountId,
        amount: u64,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (_, trader_account_pk) = self.trader_account().ok_or(TraderError::MissingConfig("trader_account"))?;
        if !self.global_ready() {
            return Err(TraderError::PoolNotReady);
        }
        let authority_pk = resolve(authority)?;
        let token_account_pk = resolve(trader_token_account)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_WITHDRAW_FUNDS);
        data.extend_from_slice(&amount.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new_readonly(PHOENIX_PROGRAM_ID, false),
            AccountMeta::new_readonly(PHOENIX_LOG_AUTHORITY, false),
            AccountMeta::new(GLOBAL_CONFIGURATION_PK, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(trader_account_pk, false),
            AccountMeta::new(self.perp_asset_map_pk, false),
            AccountMeta::new(self.global_vault_pk, false),
            AccountMeta::new(token_account_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        for pk in self.global_trader_index_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }
        for pk in self.active_trader_buffer_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }
        accounts.push(AccountMeta::new(self.withdraw_queue_pk, false));

        wallet.require_signer(authority);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, WITHDRAW_FUNDS_CU);
        Ok(())
    }

    fn order_action_accounts(&self, asset_id: u32) -> Result<(Pubkey, Vec<AccountMeta>), TraderError> {
        let (_, trader_account_pk) = self.trader_account().ok_or(TraderError::MissingConfig("trader_account"))?;
        let market = self.market(asset_id).ok_or(TraderError::MissingConfig("market"))?;
        if !market.priced || !self.global_ready() {
            return Err(TraderError::PoolNotReady);
        }
        let mut accounts = vec![
            AccountMeta::new_readonly(PHOENIX_PROGRAM_ID, false),
            AccountMeta::new_readonly(PHOENIX_LOG_AUTHORITY, false),
            AccountMeta::new(GLOBAL_CONFIGURATION_PK, false),
        ];
        // trader (signer) placeholder pushed by the caller once the
        // authority pubkey is resolved -- see call sites below.
        accounts.push(AccountMeta::new(trader_account_pk, false));
        accounts.push(AccountMeta::new(self.perp_asset_map_pk, false));
        for pk in self.global_trader_index_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }
        for pk in self.active_trader_buffer_accounts() {
            accounts.push(AccountMeta::new(pk, false));
        }
        accounts.push(AccountMeta::new(market.market_pk, false));
        accounts.push(AccountMeta::new(market.spline_collection_pk, false));
        Ok((market.market_pk, accounts))
    }

    /// Build a `place_limit_order` instruction and append it to `wallet`.
    /// `client_order_id` is caller-supplied (any value works; the protocol
    /// only uses it for the caller's own order tracking).
    #[allow(clippy::too_many_arguments)]
    pub fn place_limit_order(
        &self,
        authority: AccountId,
        asset_id: u32,
        side: Side,
        price_in_ticks: u64,
        num_base_lots: u64,
        client_order_id: u128,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (_market_pk, mut accounts) = self.order_action_accounts(asset_id)?;
        let authority_pk = resolve(authority)?;
        // Insert the signer account right after global_configuration (index 3).
        accounts.insert(3, AccountMeta::new_readonly(authority_pk, true));

        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&DISC_PLACE_LIMIT_ORDER);
        data.push(1); // OrderPacketKind::Limit discriminant
        data.push(side as u8);
        data.extend_from_slice(&price_in_ticks.to_le_bytes());
        data.extend_from_slice(&num_base_lots.to_le_bytes());
        data.push(SelfTradeBehavior::CancelProvide as u8);
        push_option_u64(&mut data, None); // match_limit
        data.extend_from_slice(&client_order_id.to_le_bytes());
        push_option_u64(&mut data, None); // last_valid_slot
        data.push(OrderFlags::None as u8);
        data.push(0); // cancel_existing

        wallet.require_signer(authority);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, PLACE_LIMIT_ORDER_CU);
        Ok(())
    }

    /// Build a `place_market_order` instruction (immediate-or-cancel) and
    /// append it to `wallet`. `min_base_lots_to_fill = 0` accepts any
    /// partial fill down to zero -- set higher for an all-or-nothing
    /// guard.
    #[allow(clippy::too_many_arguments)]
    pub fn place_market_order(
        &self,
        authority: AccountId,
        asset_id: u32,
        side: Side,
        num_base_lots: u64,
        min_base_lots_to_fill: u64,
        client_order_id: u128,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (_market_pk, mut accounts) = self.order_action_accounts(asset_id)?;
        let authority_pk = resolve(authority)?;
        accounts.insert(3, AccountMeta::new_readonly(authority_pk, true));

        let mut data = Vec::with_capacity(72);
        data.extend_from_slice(&DISC_PLACE_MARKET_ORDER);
        data.push(2); // OrderPacketKind::ImmediateOrCancel discriminant
        data.push(side as u8);
        push_option_u64(&mut data, None); // price_in_ticks (no limit)
        data.extend_from_slice(&num_base_lots.to_le_bytes());
        push_option_u64(&mut data, None); // num_quote_lots
        data.extend_from_slice(&min_base_lots_to_fill.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes()); // min_quote_lots_to_fill
        data.push(SelfTradeBehavior::CancelProvide as u8);
        push_option_u64(&mut data, None); // match_limit
        data.extend_from_slice(&client_order_id.to_le_bytes());
        push_option_u64(&mut data, None); // last_valid_slot
        data.push(OrderFlags::None as u8);
        data.push(0); // cancel_existing

        wallet.require_signer(authority);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, PLACE_MARKET_ORDER_CU);
        Ok(())
    }

    /// Build a `cancel_orders_by_id` instruction and append it to `wallet`.
    /// Each order is identified by `(price_in_ticks, order_sequence_number)`
    /// -- both come from the order's `FifoOrderId`, which the caller must
    /// already have (e.g. from the response to the `place_*_order` call
    /// that created it; not tracked by this module).
    pub fn cancel_orders_by_id(
        &self,
        authority: AccountId,
        asset_id: u32,
        order_ids: &[(u64, u64)],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (_market_pk, mut accounts) = self.order_action_accounts(asset_id)?;
        let authority_pk = resolve(authority)?;
        accounts.insert(3, AccountMeta::new_readonly(authority_pk, true));

        let mut data = Vec::with_capacity(8 + 4 + order_ids.len() * 20);
        data.extend_from_slice(&DISC_CANCEL_ORDERS_BY_ID);
        data.extend_from_slice(&(order_ids.len() as u32).to_le_bytes());
        for &(price_in_ticks, order_sequence_number) in order_ids {
            data.extend_from_slice(&0u32.to_le_bytes()); // node_pointer
            data.extend_from_slice(&price_in_ticks.to_le_bytes());
            data.extend_from_slice(&order_sequence_number.to_le_bytes());
        }

        wallet.require_signer(authority);
        wallet.append_ix(Instruction { program_id: PHOENIX_PROGRAM_ID, accounts, data }, CANCEL_ORDERS_BY_ID_CU);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn disc(name: &str) -> [u8; 8] {
        Sha256::digest(format!("global:{name}").as_bytes())[..8].try_into().unwrap()
    }

    #[test]
    fn instruction_discriminants_match_their_snake_case_names() {
        assert_eq!(disc("register_trader"), DISC_REGISTER_TRADER);
        assert_eq!(disc("deposit_funds"), DISC_DEPOSIT_FUNDS);
        assert_eq!(disc("withdraw_funds"), DISC_WITHDRAW_FUNDS);
        assert_eq!(disc("place_limit_order"), DISC_PLACE_LIMIT_ORDER);
        assert_eq!(disc("place_market_order"), DISC_PLACE_MARKET_ORDER);
        assert_eq!(disc("cancel_orders_by_id"), DISC_CANCEL_ORDERS_BY_ID);
    }

    #[test]
    fn option_u64_encodes_none_as_single_zero_byte() {
        let mut d = Vec::new();
        push_option_u64(&mut d, None);
        assert_eq!(d, vec![0]);
    }

    #[test]
    fn option_u64_encodes_some_as_disc_plus_le_bytes() {
        let mut d = Vec::new();
        push_option_u64(&mut d, Some(50_000));
        assert_eq!(d[0], 1);
        assert_eq!(u64::from_le_bytes(d[1..9].try_into().unwrap()), 50_000);
    }

    #[test]
    fn side_and_order_flags_use_the_verified_discriminant_bytes() {
        // Matches order_packet.rs's own serialization tests exactly.
        assert_eq!(Side::Bid as u8, 0);
        assert_eq!(Side::Ask as u8, 1);
        assert_eq!(OrderFlags::None as u8, 0);
        assert_eq!(OrderFlags::ReduceOnly as u8, 128);
        assert_eq!(SelfTradeBehavior::Abort as u8, 0);
        assert_eq!(SelfTradeBehavior::CancelProvide as u8, 1);
        assert_eq!(SelfTradeBehavior::DecrementTake as u8, 2);
    }
}
