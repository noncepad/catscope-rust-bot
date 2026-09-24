//! Drift v2 SpotMarket account parser (credit-graph phase 2c — see
//! `~/cryptic-percolating-bunny.md`).
//!
//! [`DriftState`] wires spot-market subscriptions into `DexState` so this
//! data stays live, feeding [`crate::trader::credit::CreditReserve`] the
//! same as Kamino (phase 1), marginfi (phase 2a), and Solend (phase 2b).
//! Like Solend and unlike marginfi, no separate oracle subscription is
//! needed -- `price_usd` is already a field directly on the SpotMarket
//! account itself (a snapshot, see the caveat below), so `DriftState`
//! only ever tracks one account type.
//!
//! # Account layout — Drift v2 SpotMarket (Anchor, zero_copy, 8-byte discriminator)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        8   Anchor discriminator
//!   8       32   pubkey (self-referential; verified against own address)
//!  40       32   oracle
//!  72       32   mint
//! 104       32   vault
//! 136       32   name
//! 168        8   historical_oracle_data.last_oracle_price (i64)  ← TWAP-ish price
//! ...      ...   (historical_oracle_data/historical_index_data/revenue_pool/
//!                 spot_fee_pool/insurance_fund -- not needed here)
//! 432       16   deposit_balance (u128, scaled)
//! 448       16   borrow_balance (u128, scaled)
//! 464       16   cumulative_deposit_interest (u128)
//! 480       16   cumulative_borrow_interest (u128)
//! 640        4   initial_asset_weight (u32)         ← LTV equivalent
//! 644        4   maintenance_asset_weight (u32)
//! 648        4   initial_liability_weight (u32)
//! 652        4   maintenance_liability_weight (u32)
//! 668        4   optimal_utilization (u32)
//! 672        4   optimal_borrow_rate (u32)
//! 676        4   max_borrow_rate (u32)
//! 680        4   decimals (u32)
//! 728        1   min_borrow_rate (u8)
//! ```
//!
//! Offsets were computed from the field order in the public
//! `drift-labs/protocol-v2` source (`state/spot_market.rs`,
//! `state/oracle.rs`, `state/perp_market.rs` for `PoolBalance`) and cross-
//! checked against a real mainnet SpotMarket account: the running byte
//! count lands exactly on the account's real size (776 bytes) with no
//! slack, `initial_asset_weight`/`maintenance_asset_weight`/
//! `initial_liability_weight`/`maintenance_liability_weight` decoded to
//! 0.50/0.75/1.50/1.25 (the same LTV-below-maintenance,
//! liability-above-1.0 pattern found for every other protocol this
//! session), and `deposit_balance`/`cumulative_deposit_interest` combined
//! to a plausible ~5,385-token position -- of two candidate precision
//! formulas tried (dividing by `SPOT_BALANCE_PRECISION` as well as
//! `CUMULATIVE_INTEREST_PRECISION`, vs. only the latter), only
//! `scaled_balance * (cumulative_interest / 1e10)` produced a sane token
//! quantity; the other gave a physically-implausible fraction of a token.
//!
//! **Approximate, not independently verified**: `min_borrow_rate` is a
//! lone `u8` field (unlike the other `u32` rate fields at 1e6 precision)
//! and read back as `0` on the one real account sampled, so its scale
//! couldn't be empirically pinned down. Using the commonly-cited SDK
//! convention of `raw / 200.0` as a fraction; since it only matters at
//! zero utilization, this is a minor approximation. `price_usd` here is
//! `historical_oracle_data.last_oracle_price` -- a snapshot from the last
//! oracle update, not a live read of the actual oracle account.

use crate::{
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    trader::{dex::update::Updater, pricegraph::TradeRouter, types::TraderError},
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::{system_program, sysvar::rent};
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

pub const DRIFT_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const OFF_ORACLE: usize = 40;
const OFF_MINT: usize = 72;
const OFF_VAULT: usize = 104;
const OFF_LAST_ORACLE_PRICE: usize = 168;
const OFF_DEPOSIT_BALANCE: usize = 432;
const OFF_BORROW_BALANCE: usize = 448;
const OFF_CUMULATIVE_DEPOSIT_INTEREST: usize = 464;
const OFF_CUMULATIVE_BORROW_INTEREST: usize = 480;
const OFF_INITIAL_ASSET_WEIGHT: usize = 640;
const OFF_OPTIMAL_UTILIZATION: usize = 668;
const OFF_OPTIMAL_BORROW_RATE: usize = 672;
const OFF_MAX_BORROW_RATE: usize = 676;
const OFF_DECIMALS: usize = 680;
/// Verified empirically: fetched real mainnet SpotMarket accounts via
/// `getProgramAccounts` (memcmp on the `SpotMarket` discriminator), decoded
/// the bytes at this offset as `u16` LE, and cross-checked against each
/// account's `name` field -- e.g. mSOL landed at `market_index = 2`, a
/// well-known low index for that market; sUSDe/PYUSD/USDS/TRUMP/POPCAT all
/// decoded to small, plausible, mutually-distinct indices (24/22/28/36/20).
const OFF_MARKET_INDEX: usize = 684;
const OFF_MIN_BORROW_RATE: usize = 728;

const MIN_MARKET_LEN: usize = OFF_MIN_BORROW_RATE + 1;

/// Drift's fixed precisions (`SPOT_WEIGHT_PRECISION`, `PERCENTAGE_PRECISION`,
/// `SPOT_CUMULATIVE_INTEREST_PRECISION`, `PRICE_PRECISION`).
const WEIGHT_PRECISION: f64 = 10_000.0;
const PERCENTAGE_PRECISION: f64 = 1_000_000.0;
const CUMULATIVE_INTEREST_PRECISION: f64 = 1e10;
const PRICE_PRECISION: f64 = 1_000_000.0;

// ─── User account (Anchor, zero_copy, repr(C), 8-byte discriminator) ────────
//
// Offsets computed from the real field order in
// `programs/drift/src/state/user.rs` (`velocity-exchange/protocol-v2`,
// tag `v2.162.0` -- Drift's GitHub org renamed to `velocity-exchange`,
// same protocol, matching the earlier Drift→Velocity rebrand), summing
// each field's own size under `repr(C)` layout rules (natural alignment,
// no reordering). **Live-verified this session** against a real trader's
// `User` account fetched from a real, on-chain `place_perp_order`
// transaction: `authority` (offset 8) decoded to the exact real signer
// pubkey from that transaction, `name` (offset 72) decoded to readable
// text ("Main Account"), and `spot_positions` (offset 104) decoded to
// two sane, real-looking entries (market_index 0/1 -- USDC/SOL, the
// well-known low indices -- both with plausible nonzero scaled
// balances). `perp_positions` (offset 424) wasn't verified against a
// nonzero example -- that specific transaction's order failed
// (`Custom: 101`), so no position was ever opened -- but its offset
// follows directly and sequentially from the same verified layout.
//
// ```text
// offset   size  field
// ──────   ────  ─────────────────────────────────────
//    0        8   Anchor discriminator
//    8       32   authority
//   40       32   delegate
//   72       32   name
//  104      320   spot_positions: [SpotPosition; 8]  (40 bytes each)
//  424      768   perp_positions: [PerpPosition; 8]  (96 bytes each)
//  ...            orders, and everything after -- not read by this bot
// ```
//
// `SpotPosition` (40 bytes, `repr(C)`), offsets relative to each
// position's own start (`104 + i * SPOT_POSITION_LEN`) -- source-confirmed
// against the same tagged release (`state/user.rs`), not yet
// independently live-verified against a nonzero example by this bot
// (the "two sane, real-looking entries" noted above were read directly
// off raw bytes during this session's research, not through this
// specific parsing code):
// ```text
// offset  size  field
// ──────  ────  ──────────────────────────
//    0      8   scaled_balance (u64)
//    8      8   open_bids (i64)
//   16      8   open_asks (i64)
//   24      8   cumulative_deposits (i64)
//   32      2   market_index (u16)
//   34      1   balance_type (0=Deposit, 1=Borrow)
//   35      1   open_orders (u8)
//   36      4   padding
// ```
//
// `PerpPosition` (96 bytes, `repr(C)`), offsets relative to each
// position's own start (`424 + i * PERP_POSITION_LEN`):
// ```text
// offset  size  field
// ──────  ────  ──────────────────────────
//    0      8   last_cumulative_funding_rate (i64)
//    8      8   base_asset_amount (i64)
//   16      8   quote_asset_amount (i64)
//   24      8   quote_break_even_amount (i64)
//   32      8   quote_entry_amount (i64)
//   40      8   open_bids (i64)
//   48      8   open_asks (i64)
//   56      8   settled_pnl (i64)
//   64      8   lp_shares (u64)
//   72      8   isolated_position_scaled_balance (u64)
//   80      8   last_quote_asset_amount_per_lp (i64)
//   88      2   padding
//   90      2   max_margin_ratio (u16)
//   92      2   market_index (u16)
//   94      1   open_orders (u8)
//   95      1   position_flag (u8)
// ```
const OFF_USER_AUTHORITY: usize = 8;
const OFF_USER_SPOT_POSITIONS: usize = 104;
const NUM_SPOT_POSITIONS: usize = 8;
const SPOT_POSITION_LEN: usize = 40;
const OFF_SPP_SCALED_BALANCE: usize = 0;
const OFF_SPP_MARKET_INDEX: usize = 32;
const OFF_SPP_BALANCE_TYPE: usize = 34;
const OFF_USER_PERP_POSITIONS: usize = 424;
const NUM_PERP_POSITIONS: usize = 8;
const PERP_POSITION_LEN: usize = 96;
const OFF_PP_LAST_CUMULATIVE_FUNDING_RATE: usize = 0;
const OFF_PP_BASE_ASSET_AMOUNT: usize = 8;
const OFF_PP_QUOTE_ENTRY_AMOUNT: usize = 32;
const OFF_PP_SETTLED_PNL: usize = 56;
const OFF_PP_MARKET_INDEX: usize = 92;

const MIN_USER_LEN: usize = OFF_USER_PERP_POSITIONS + NUM_PERP_POSITIONS * PERP_POSITION_LEN;

/// One entry from a `User` account's `perp_positions` array -- only the
/// fields needed to know "do I already have a position in this market,
/// and what's its size/entry," mirroring what `PhoenixPositionState`
/// already tracks for the Phoenix side of a funding-arb cycle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriftPerpPosition {
    pub market_index: u16,
    /// Precision: `BASE_PRECISION`. Positive = long, negative = short,
    /// zero = no position in this market.
    pub base_asset_amount: i64,
    /// Precision: `QUOTE_PRECISION`.
    pub quote_entry_amount: i64,
    /// Precision: `FUNDING_RATE_PRECISION`.
    pub last_cumulative_funding_rate: i64,
    /// Precision: `QUOTE_PRECISION`.
    pub settled_pnl: i64,
}

impl DriftPerpPosition {
    /// `true` when this slot holds a real open position (nonzero size) --
    /// mirrors the real SDK's `PerpPosition::is_for`'s core check, minus
    /// the open-orders/lp-shares nuance this bot doesn't need yet.
    pub fn is_open(&self) -> bool {
        self.base_asset_amount != 0
    }
}

/// One entry from a `User` account's `spot_positions` array -- e.g. the
/// USDC balance backing this bot's perp margin on Drift (unified
/// cross-margin: the same account backs both spot lending and perp
/// margin, see this module's own doc comment). `scaled_balance` needs a
/// market's `cumulative_deposit_interest`/`cumulative_borrow_interest`
/// (from that market's own parsed [`DriftSpotMarket`]) to convert to a
/// real token amount -- same formula already used for that market's own
/// aggregate `deposit_amount`/`borrow_amount` fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriftSpotPosition {
    pub market_index: u16,
    /// Precision: `SPOT_BALANCE_PRECISION`. Needs `is_borrow` to know
    /// which of a market's two interest ratios applies.
    pub scaled_balance: u64,
    /// `false` = Deposit (an asset this bot owns), `true` = Borrow (a
    /// liability). This bot never intentionally borrows against its
    /// USDC collateral -- a `true` here on the USDC market is
    /// unexpected and callers should treat it as unusable collateral,
    /// not assume it's spendable.
    pub is_borrow: bool,
}

/// Parsed subset of a Drift v2 `User` account -- this bot's own trader
/// account, not a market. `authority` plus every `spot_positions`/
/// `perp_positions` slot (`NUM_SPOT_POSITIONS`/`NUM_PERP_POSITIONS` = 8
/// each, matching the real account's fixed-size arrays) -- callers
/// needing "do I have a position in market X" should use
/// [`DriftUser::spot_position_for`]/[`DriftUser::perp_position_for`].
#[derive(Debug, Default, Clone)]
pub struct DriftUser {
    pub authority: AccountId,
    pub spot_positions: [DriftSpotPosition; NUM_SPOT_POSITIONS],
    pub perp_positions: [DriftPerpPosition; NUM_PERP_POSITIONS],
}

impl DriftUser {
    /// The position (if any) in spot `market_index`, if that slot's
    /// `scaled_balance` is nonzero -- deposit or borrow, see
    /// [`DriftSpotPosition::is_borrow`]'s doc.
    pub fn spot_position_for(&self, market_index: u16) -> Option<&DriftSpotPosition> {
        self.spot_positions.iter().find(|p| p.market_index == market_index && p.scaled_balance != 0)
    }

    /// The open position (if any) in `market_index`, if that slot's
    /// `base_asset_amount` is nonzero.
    pub fn perp_position_for(&self, market_index: u16) -> Option<&DriftPerpPosition> {
        self.perp_positions.iter().find(|p| p.market_index == market_index && p.is_open())
    }
}

/// Parse a Drift v2 `User` account from raw body bytes (including the
/// 8-byte Anchor discriminator). See the offset table in this module's
/// `User account` section comment above.
pub fn parse_user(body: &[u8]) -> Option<DriftUser> {
    if body.len() < MIN_USER_LEN {
        return None;
    }
    let read_u16 = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_i64 = |off: usize| i64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let authority = account_id_from_pubkey(&Pubkey::new_from_array(
        body[OFF_USER_AUTHORITY..OFF_USER_AUTHORITY + 32].try_into().unwrap(),
    ));

    let mut spot_positions = [DriftSpotPosition::default(); NUM_SPOT_POSITIONS];
    for (i, slot) in spot_positions.iter_mut().enumerate() {
        let base = OFF_USER_SPOT_POSITIONS + i * SPOT_POSITION_LEN;
        *slot = DriftSpotPosition {
            market_index: read_u16(base + OFF_SPP_MARKET_INDEX),
            scaled_balance: read_u64(base + OFF_SPP_SCALED_BALANCE),
            is_borrow: body[base + OFF_SPP_BALANCE_TYPE] != 0,
        };
    }

    let mut perp_positions = [DriftPerpPosition::default(); NUM_PERP_POSITIONS];
    for (i, slot) in perp_positions.iter_mut().enumerate() {
        let base = OFF_USER_PERP_POSITIONS + i * PERP_POSITION_LEN;
        *slot = DriftPerpPosition {
            market_index: read_u16(base + OFF_PP_MARKET_INDEX),
            base_asset_amount: read_i64(base + OFF_PP_BASE_ASSET_AMOUNT),
            quote_entry_amount: read_i64(base + OFF_PP_QUOTE_ENTRY_AMOUNT),
            last_cumulative_funding_rate: read_i64(base + OFF_PP_LAST_CUMULATIVE_FUNDING_RATE),
            settled_pnl: read_i64(base + OFF_PP_SETTLED_PNL),
        };
    }

    Some(DriftUser { authority, spot_positions, perp_positions })
}

/// Parsed subset of a Drift v2 SpotMarket account -- only the fields
/// needed for [`crate::trader::credit::CreditReserve::from_drift`].
#[derive(Debug, Default, Clone)]
pub struct DriftSpotMarket {
    pub mint: AccountId,
    pub vault: AccountId,
    /// Price oracle for this market -- needed as part of the
    /// `[oracle][spot market]` remaining_accounts pairs `deposit`/`withdraw`
    /// require for every open position.
    pub oracle: AccountId,
    /// This market's index -- part of `deposit`/`withdraw`'s instruction
    /// data (which market to act on) and the seed for its own PDA-derived
    /// accounts (`spot_market_vault`).
    pub market_index: u16,
    pub mint_decimals: u32,
    /// Oracle price snapshot in USD (see module doc comment -- not a live read).
    pub price_usd: f64,
    /// Max loan-to-value ratio (0.0-1.0) when this market's asset is
    /// deposited as collateral -- Drift's `initial_asset_weight`.
    pub initial_asset_weight: f64,
    /// Currently deposited, in native token units (scaled balance × cumulative interest).
    pub deposit_amount: f64,
    /// Currently borrowed, in native token units.
    pub borrow_amount: f64,
    /// `raw_cumulative_deposit_interest / CUMULATIVE_INTEREST_PRECISION` --
    /// the same ratio already used to compute `deposit_amount` above,
    /// exposed on its own so a *per-user* `DriftSpotPosition::scaled_balance`
    /// (Deposit-type) can be converted to a real token amount with the
    /// identical, already-live-verified formula (`scaled_balance as f64 *
    /// this_ratio`) rather than the market's own aggregate balance.
    pub cumulative_deposit_interest: f64,
    /// Same as `cumulative_deposit_interest`, for Borrow-type positions.
    pub cumulative_borrow_interest: f64,
    pub optimal_utilization: f64,
    pub optimal_borrow_rate: f64,
    pub max_borrow_rate: f64,
    pub min_borrow_rate: f64,
}

impl DriftSpotMarket {
    /// Current utilization (0.0-1.0): borrowed / deposited.
    pub fn utilization(&self) -> f64 {
        if self.deposit_amount <= 0.0 {
            return 0.0;
        }
        (self.borrow_amount / self.deposit_amount).min(1.0)
    }

    /// Available liquidity, in native token units.
    pub fn available_amount(&self) -> f64 {
        (self.deposit_amount - self.borrow_amount).max(0.0)
    }

    /// Estimate the current borrow APY (0.0-1.0 fraction) via the same
    /// two-segment jump-rate model as Solend: linear from
    /// `min_borrow_rate` to `optimal_borrow_rate` up to
    /// `optimal_utilization`, then linear from `optimal_borrow_rate` to
    /// `max_borrow_rate` beyond it.
    pub fn current_borrow_apy(&self) -> f64 {
        let util = self.utilization();
        if util <= self.optimal_utilization {
            if self.optimal_utilization <= 0.0 {
                return self.min_borrow_rate;
            }
            let frac = util / self.optimal_utilization;
            self.min_borrow_rate + (self.optimal_borrow_rate - self.min_borrow_rate) * frac
        } else {
            let denom = 1.0 - self.optimal_utilization;
            let frac = if denom <= 0.0 {
                1.0
            } else {
                (util - self.optimal_utilization) / denom
            };
            self.optimal_borrow_rate + (self.max_borrow_rate - self.optimal_borrow_rate) * frac
        }
    }

    // ─── Instruction builders ─────────────────────────────────────────────

    /// Append a `deposit` instruction to `wallet`. There is no separate
    /// "borrow" instruction in Drift v2 -- see [`Self::withdraw`].
    ///
    /// `other_open_markets` must list, as `(oracle, spot_market)` pairs, every
    /// OTHER spot market this user currently has an open position in (this
    /// market's own oracle/account are appended automatically). Drift needs
    /// all of them present in `remaining_accounts` to compute margin/health,
    /// laid out as `[oracles...][spot markets...]` -- confirmed against
    /// `handle_deposit`'s `load_maps` call
    /// (`programs/drift/src/instructions/user.rs`). This bot doesn't track a
    /// user's actual open positions, so the caller must supply the list (see
    /// the credit-graph module's non-goals). Perp positions aren't
    /// supported -- this bot only models spot lending anywhere.
    #[allow(clippy::too_many_arguments)]
    pub fn deposit(
        &self,
        market_id: AccountId,
        authority: AccountId,
        sub_account_id: u16,
        amount: u64,
        reduce_only: bool,
        user_token_account: AccountId,
        other_open_markets: &[(AccountId, AccountId)],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let authority_pk = resolve(authority)?;
        let user_pk = user_pda(&authority_pk, sub_account_id);
        let user_stats_pk = user_stats_pda(&authority_pk);
        let vault_pk = spot_market_vault_pda(self.market_index);
        let user_token_pk = resolve(user_token_account)?;

        let mut data = Vec::with_capacity(19);
        data.extend_from_slice(&DISC_DEPOSIT);
        data.extend_from_slice(&self.market_index.to_le_bytes());
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(reduce_only as u8);

        let mut accounts = vec![
            AccountMeta::new_readonly(drift_state(), false),
            AccountMeta::new(user_pk, false),
            AccountMeta::new(user_stats_pk, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(vault_pk, false),
            AccountMeta::new(user_token_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        push_remaining_accounts(&mut accounts, other_open_markets, market_id, self.oracle)?;

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: DRIFT_PROGRAM_ID,
                accounts,
                data,
            },
            DRIFT_DEPOSIT_BASE_CU
                + other_open_markets.len() as u32 * DRIFT_DEPOSIT_PER_MARKET_CU,
        );
        Ok(())
    }

    /// Append a `withdraw` instruction to `wallet`. Withdrawing more than
    /// the user's deposited balance in this market *is* how borrowing works
    /// in Drift v2 -- confirmed against the live on-chain instruction list
    /// (no separate `borrow`/`borrowSpot`-style entry exists). Unlike
    /// deposit, `other_open_markets` (same `(oracle, spot_market)` shape) is
    /// *always* required here, not just for an edge case -- `handle_withdraw`
    /// unconditionally checks margin across every open position.
    #[allow(clippy::too_many_arguments)]
    pub fn withdraw(
        &self,
        market_id: AccountId,
        authority: AccountId,
        sub_account_id: u16,
        amount: u64,
        reduce_only: bool,
        user_token_account: AccountId,
        other_open_markets: &[(AccountId, AccountId)],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let authority_pk = resolve(authority)?;
        let user_pk = user_pda(&authority_pk, sub_account_id);
        let user_stats_pk = user_stats_pda(&authority_pk);
        let vault_pk = spot_market_vault_pda(self.market_index);
        let user_token_pk = resolve(user_token_account)?;

        let mut data = Vec::with_capacity(19);
        data.extend_from_slice(&DISC_WITHDRAW);
        data.extend_from_slice(&self.market_index.to_le_bytes());
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(reduce_only as u8);

        let mut accounts = vec![
            AccountMeta::new_readonly(drift_state(), false),
            AccountMeta::new(user_pk, false),
            AccountMeta::new(user_stats_pk, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(vault_pk, false),
            AccountMeta::new_readonly(drift_signer(), false),
            AccountMeta::new(user_token_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        push_remaining_accounts(&mut accounts, other_open_markets, market_id, self.oracle)?;

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: DRIFT_PROGRAM_ID,
                accounts,
                data,
            },
            DRIFT_WITHDRAW_BASE_CU
                + other_open_markets.len() as u32 * DRIFT_WITHDRAW_PER_MARKET_CU,
        );
        Ok(())
    }
}

// ─── PDA derivation ─────────────────────────────────────────────────────────

/// `["user", authority, sub_account_id_le]` -- a user's sub-account.
pub fn user_pda(authority: &Pubkey, sub_account_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[b"user", authority.as_ref(), &sub_account_id.to_le_bytes()],
        &DRIFT_PROGRAM_ID,
    )
    .0
}

/// `["user_stats", authority]` -- must exist (via [`initialize_user_stats`])
/// before [`initialize_user`] will succeed.
pub fn user_stats_pda(authority: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"user_stats", authority.as_ref()], &DRIFT_PROGRAM_ID).0
}

/// `["drift_state"]` -- the single global Drift `State` account.
fn drift_state() -> Pubkey {
    Pubkey::find_program_address(&[b"drift_state"], &DRIFT_PROGRAM_ID).0
}

/// `["drift_signer"]` -- the PDA that signs token transfers out of vaults.
fn drift_signer() -> Pubkey {
    Pubkey::find_program_address(&[b"drift_signer"], &DRIFT_PROGRAM_ID).0
}

/// `["spot_market_vault", market_index_le]`.
fn spot_market_vault_pda(market_index: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[b"spot_market_vault", &market_index.to_le_bytes()],
        &DRIFT_PROGRAM_ID,
    )
    .0
}

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

/// Appends the `[oracles...][spot markets...]` remaining-accounts layout
/// `deposit`/`withdraw` need: every `other_open_markets` pair, then this
/// market's own `(market_id, oracle)` last.
fn push_remaining_accounts(
    accounts: &mut Vec<AccountMeta>,
    other_open_markets: &[(AccountId, AccountId)],
    market_id: AccountId,
    oracle: AccountId,
) -> Result<(), TraderError> {
    let mut spot_markets = Vec::with_capacity(other_open_markets.len() + 1);
    for &(oracle_id, market) in other_open_markets {
        accounts.push(AccountMeta::new_readonly(resolve(oracle_id)?, false));
        spot_markets.push(resolve(market)?);
    }
    accounts.push(AccountMeta::new_readonly(resolve(oracle)?, false));
    spot_markets.push(resolve(market_id)?);
    for pk in spot_markets {
        accounts.push(AccountMeta::new(pk, false));
    }
    Ok(())
}

/// sha256("global:initialize_user_stats")[..8]
const DISC_INITIALIZE_USER_STATS: [u8; 8] = [254, 243, 72, 98, 251, 130, 168, 213];
/// sha256("global:initialize_user")[..8]
const DISC_INITIALIZE_USER: [u8; 8] = [111, 17, 185, 250, 60, 122, 38, 254];
/// sha256("global:deposit")[..8]
const DISC_DEPOSIT: [u8; 8] = [242, 35, 198, 137, 82, 225, 242, 182];
/// sha256("global:withdraw")[..8]
const DISC_WITHDRAW: [u8; 8] = [183, 18, 70, 156, 148, 109, 161, 34];
/// sha256("global:place_perp_order")[..8] -- live-verified this session:
/// matches the exact discriminator bytes decoded from a real, on-chain
/// `place_perp_order` transaction's instruction data.
const DISC_PLACE_PERP_ORDER: [u8; 8] = [69, 161, 93, 202, 120, 126, 76, 185];

pub const DRIFT_INITIALIZE_USER_STATS_CU: u32 = 40_000;
pub const DRIFT_INITIALIZE_USER_CU: u32 = 40_000;
pub const DRIFT_DEPOSIT_BASE_CU: u32 = 60_000;
pub const DRIFT_DEPOSIT_PER_MARKET_CU: u32 = 20_000;
pub const DRIFT_WITHDRAW_BASE_CU: u32 = 80_000;
pub const DRIFT_WITHDRAW_PER_MARKET_CU: u32 = 20_000;
/// Placeholder, not benchmarked against a real compute-unit trace --
/// same discipline as the other `DRIFT_*_CU` constants above, just
/// without a real deposit/withdraw precedent to match.
pub const DRIFT_PLACE_PERP_ORDER_CU: u32 = 120_000;

/// `OrderType` -- real variant order from the authoritative IDL
/// (`sdk/src/idl/drift.json`, tag `v2.162.0`); the `u8` discriminant is
/// each variant's position, standard Anchor/Borsh enum encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderType {
    Market = 0,
    Limit = 1,
    TriggerMarket = 2,
    TriggerLimit = 3,
    Oracle = 4,
}

/// `MarketType` -- real variant order from the IDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MarketType {
    Spot = 0,
    Perp = 1,
}

/// `PositionDirection` -- real variant order from the IDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PositionDirection {
    Long = 0,
    Short = 1,
}

/// `PostOnlyParam` -- real variant order from the IDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PostOnlyParam {
    None = 0,
    MustPostOnly = 1,
    TryPostOnly = 2,
    Slide = 3,
}

/// `OrderTriggerCondition` -- real variant order from the IDL. Unused by
/// a plain market order ([`place_perp_order`]) but still part of
/// `OrderParams`' fixed encoding, so it must be present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderTriggerCondition {
    Above = 0,
    Below = 1,
    TriggeredAbove = 2,
    TriggeredBelow = 3,
}

// ─── Position-account lifecycle ────────────────────────────────────────────────

/// Append an `initialize_user_stats` instruction to `wallet`. Must succeed
/// before [`initialize_user`] will (its `user_stats` account isn't `init`,
/// just `has_one = authority` on an existing one). `authority` acts as both
/// the account's authority and the fee payer -- this bot operates with a
/// single signing wallet.
pub fn initialize_user_stats(authority: AccountId, wallet: &mut Wallet) -> Result<(), TraderError> {
    let authority_pk = resolve(authority)?;
    let user_stats_pk = user_stats_pda(&authority_pk);

    wallet.require_signer(authority);
    wallet.append_ix(
        Instruction {
            program_id: DRIFT_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(user_stats_pk, false),
                AccountMeta::new(drift_state(), false),
                AccountMeta::new_readonly(authority_pk, true),
                AccountMeta::new(authority_pk, true), // payer (same signer)
                AccountMeta::new_readonly(rent::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data: DISC_INITIALIZE_USER_STATS.to_vec(),
        },
        DRIFT_INITIALIZE_USER_STATS_CU,
    );
    Ok(())
}

/// Append an `initialize_user` instruction to `wallet`, creating sub-account
/// `sub_account_id` for `authority`. `name` is the account's display name
/// (Drift pads/truncates it to 32 bytes; pass e.g. `*b"main account\0\0\0..."`
/// or all-zero for no name). Requires [`initialize_user_stats`] to have
/// already succeeded for `authority`. Returns the sub-account's `AccountId`
/// -- its address is deterministic ([`user_pda`]), nothing to remember
/// separately from `(authority, sub_account_id)`.
pub fn initialize_user(
    authority: AccountId,
    sub_account_id: u16,
    name: [u8; 32],
    wallet: &mut Wallet,
) -> Result<AccountId, TraderError> {
    let authority_pk = resolve(authority)?;
    let user_pk = user_pda(&authority_pk, sub_account_id);
    let user_stats_pk = user_stats_pda(&authority_pk);

    let mut data = Vec::with_capacity(42);
    data.extend_from_slice(&DISC_INITIALIZE_USER);
    data.extend_from_slice(&sub_account_id.to_le_bytes());
    data.extend_from_slice(&name);

    wallet.require_signer(authority);
    wallet.append_ix(
        Instruction {
            program_id: DRIFT_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(user_pk, false),
                AccountMeta::new(user_stats_pk, false),
                AccountMeta::new(drift_state(), false),
                AccountMeta::new_readonly(authority_pk, true),
                AccountMeta::new(authority_pk, true), // payer (same signer)
                AccountMeta::new_readonly(rent::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        },
        DRIFT_INITIALIZE_USER_CU,
    );
    Ok(account_id_from_pubkey(&user_pk))
}

// ─── Perp order placement ───────────────────────────────────────────────────

/// `place_perp_order`'s named accounts (`[state, user, authority]`) and
/// `OrderParams` argument shape are from the real, authoritative IDL
/// (`sdk/src/idl/drift.json`, `velocity-exchange/protocol-v2` tag
/// `v2.162.0`) -- not guessed from the Rust source's `#[derive(Accounts)]`
/// struct alone, which only lists 3 accounts and resolves the rest via
/// `remaining_accounts` (`optional_accounts.rs::load_maps`). The
/// `remaining_accounts` *shape* is live-verified this session against a
/// real, on-chain `place_perp_order` transaction targeting SOL-PERP
/// (market_index 0) -- decoded via its instruction discriminator
/// (`sha256("global:place_perp_order")[..8]`, matches `DISC_PLACE_PERP_ORDER`
/// below exactly) -- and was, for that specific market:
/// `[PythLazerOracle cache, legacy oracle, spot market (SOL), spot market
/// (USDC), perp market (SOL-PERP)]`, five accounts. The two oracle-related
/// entries are specific to SOL-PERP using `OracleSource::PythLazer`
/// (confirmed via `perpMarkets.ts`'s `oracleSource`/`pythLazerId` fields);
/// a market using a different oracle source may need a different
/// oracle-account shape. This bot doesn't track enough per-market oracle
/// metadata to derive the right list automatically -- same reasoning as
/// `deposit`/`withdraw`'s `other_open_markets` parameter -- so, like
/// those, the caller supplies `remaining_accounts` directly.
///
/// Builds a plain, non-auctioned market order (`auction_duration: None`)
/// -- real Drift market orders typically use a short Dutch auction for
/// better execution/MEV protection; this is a deliberately simple
/// placeholder, not modeled here yet. `price` is `0` (best-effort fill,
/// no limit) and `user_order_id`/`bit_flags` are `0` (no custom
/// tracking id, no special flags).
#[allow(clippy::too_many_arguments)]
pub fn place_perp_order(
    authority: AccountId,
    sub_account_id: u16,
    market_index: u16,
    direction: PositionDirection,
    base_asset_amount: u64,
    reduce_only: bool,
    remaining_accounts: &[AccountId],
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let authority_pk = resolve(authority)?;
    let user_pk = user_pda(&authority_pk, sub_account_id);

    let mut data = Vec::with_capacity(8 + 64);
    data.extend_from_slice(&DISC_PLACE_PERP_ORDER);
    data.push(OrderType::Market as u8);
    data.push(MarketType::Perp as u8);
    data.push(direction as u8);
    data.push(0u8); // user_order_id
    data.extend_from_slice(&base_asset_amount.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes()); // price -- best-effort, no limit
    data.extend_from_slice(&market_index.to_le_bytes());
    data.push(reduce_only as u8);
    data.push(PostOnlyParam::None as u8);
    data.push(0u8); // bit_flags
    data.push(0u8); // max_ts: Option<i64> = None
    data.push(0u8); // trigger_price: Option<u64> = None
    data.push(OrderTriggerCondition::Above as u8); // unused for a plain market order, still encoded
    data.push(0u8); // oracle_price_offset: Option<i32> = None
    data.push(0u8); // auction_duration: Option<u8> = None
    data.push(0u8); // auction_start_price: Option<i64> = None
    data.push(0u8); // auction_end_price: Option<i64> = None

    let mut accounts = vec![
        AccountMeta::new_readonly(drift_state(), false),
        AccountMeta::new(user_pk, false),
        AccountMeta::new_readonly(authority_pk, true),
    ];
    for &id in remaining_accounts {
        accounts.push(AccountMeta::new_readonly(resolve(id)?, false));
    }

    wallet.require_signer(authority);
    wallet.append_ix(
        Instruction {
            program_id: DRIFT_PROGRAM_ID,
            accounts,
            data,
        },
        DRIFT_PLACE_PERP_ORDER_CU,
    );
    Ok(())
}

/// Parse a Drift v2 SpotMarket account from raw body bytes (including the
/// 8-byte Anchor discriminator).
pub fn parse(body: &[u8]) -> Option<DriftSpotMarket> {
    if body.len() < MIN_MARKET_LEN {
        return None;
    }

    let read_u16 = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
    let read_u32 = |off: usize| u32::from_le_bytes(body[off..off + 4].try_into().unwrap());
    let read_i64 = |off: usize| i64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };

    let cumulative_deposit_interest =
        read_u128(OFF_CUMULATIVE_DEPOSIT_INTEREST) as f64 / CUMULATIVE_INTEREST_PRECISION;
    let cumulative_borrow_interest =
        read_u128(OFF_CUMULATIVE_BORROW_INTEREST) as f64 / CUMULATIVE_INTEREST_PRECISION;

    Some(DriftSpotMarket {
        mint: read_pk(OFF_MINT),
        vault: read_pk(OFF_VAULT),
        oracle: read_pk(OFF_ORACLE),
        market_index: read_u16(OFF_MARKET_INDEX),
        mint_decimals: read_u32(OFF_DECIMALS),
        price_usd: read_i64(OFF_LAST_ORACLE_PRICE) as f64 / PRICE_PRECISION,
        initial_asset_weight: read_u32(OFF_INITIAL_ASSET_WEIGHT) as f64 / WEIGHT_PRECISION,
        deposit_amount: read_u128(OFF_DEPOSIT_BALANCE) as f64 * cumulative_deposit_interest,
        borrow_amount: read_u128(OFF_BORROW_BALANCE) as f64 * cumulative_borrow_interest,
        cumulative_deposit_interest,
        cumulative_borrow_interest,
        optimal_utilization: read_u32(OFF_OPTIMAL_UTILIZATION) as f64 / PERCENTAGE_PRECISION,
        optimal_borrow_rate: read_u32(OFF_OPTIMAL_BORROW_RATE) as f64 / PERCENTAGE_PRECISION,
        max_borrow_rate: read_u32(OFF_MAX_BORROW_RATE) as f64 / PERCENTAGE_PRECISION,
        min_borrow_rate: body[OFF_MIN_BORROW_RATE] as f64 / 200.0,
    })
}

/// Live Drift state: subscribes to every spot market in
/// `drift_config::DRIFT_SPOT_MARKETS` (the build-time address book, same
/// pattern as `MarginfiState::new` reading `marginfi_config::MARGINFI_BANKS`).
/// No oracle subscription is needed -- see the module doc comment.
///
/// Implements every `Updater` method for real, mirroring
/// `MarginfiState`/`SolendState`: `batch_router`/`on_tx`/`flush_pool` are
/// genuine no-ops (a lending SpotMarket has no swap price to contribute to
/// `TradeRouter`, matching the reasoning in `credit.rs`'s module doc
/// comment for why lending isn't unified with the AMM price graph), not
/// stubs left to be finished later.
pub struct DriftState {
    program_id: AccountId,
    m_market: HashMap<AccountId, DriftSpotMarket, BuildHasherDefault<XxHash64>>,
}

impl std::fmt::Debug for DriftState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriftState")
            .field("market_count", &self.m_market.len())
            .finish()
    }
}

impl DriftState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&DRIFT_PROGRAM_ID);
        let markets = crate::drift_config::DRIFT_SPOT_MARKETS;
        let mut m_market =
            HashMap::with_capacity_and_hasher(markets.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(markets.len());

        for raw in markets {
            let market_id = account_id_from_pubkey(&Pubkey::new_from_array(raw.pubkey));
            l_req.push(SubscriptionRequest {
                root: market_id,
                filter_weight: 0,
                depth: 1,
            });
            m_market.insert(market_id, DriftSpotMarket::default());
        }

        (Self { program_id, m_market }, l_req)
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn market_count(&self) -> usize {
        self.m_market.len()
    }

    /// Find the spot market with the given `market_index` (e.g. `0` is
    /// USDC) -- linear scan is fine, `m_market` only ever holds a
    /// handful of entries (see `DRIFT_SPOT_MARKETS`' build-time size).
    /// `None` if not tracked, or tracked but no account update has
    /// arrived yet (`market_index` defaults to `0` on `DriftSpotMarket::
    /// default()`, same as every other field, so an unparsed USDC-index
    /// market and an unparsed *any other* market are indistinguishable
    /// by index alone until their first real update -- acceptable here
    /// since the caller (`perpfundingv1`'s bootstrap) only proceeds once
    /// `mint`/`oracle` are non-default, checked at the call site).
    pub fn market_by_index(&self, market_index: u16) -> Option<(AccountId, &DriftSpotMarket)> {
        self.m_market.iter().find(|(_, m)| m.market_index == market_index).map(|(id, m)| (*id, m))
    }

    /// Build a priced [`crate::trader::credit::CreditReserve`] for
    /// `market_id`, using the latest parsed market state -- unlike
    /// `MarginfiState::credit_reserve`, there's no separate oracle update
    /// to wait for, so this is `None` only before the market's own first
    /// account update has arrived.
    pub fn credit_reserve(
        &self,
        market_id: AccountId,
    ) -> Option<crate::trader::credit::CreditReserve> {
        let market = self.m_market.get(&market_id)?;
        Some(crate::trader::credit::CreditReserve::from_drift(
            market_id, market,
        ))
    }
}

impl Updater for DriftState {
    fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if let Some(market) = self.m_market.get_mut(&header.accountid) {
            if let Some(parsed) = parse(body) {
                *market = parsed;
            }
        }
    }

    fn on_token(&mut self, _ta: &crate::catscope::witbot::shooter::Tokenaccountv1) -> bool {
        // Drift's available liquidity/price come directly from a
        // SpotMarket's own fields (already read in parse()), not from
        // watching a vault's SPL Token balance -- there is nothing for
        // this hook to track, same reasoning as MarginfiState::on_token.
        false
    }

    fn batch_router(&mut self, _router: &mut TradeRouter) {
        // A lending SpotMarket has no swap price to contribute to
        // TradeRouter -- see the module doc comment. Genuine no-op, not
        // unimplemented.
    }

    fn on_tx(&mut self, _ix: &crate::txview::CatscopeInstructionRead<'_>, _slot: &Slot) {}

    fn flush_pool(&mut self, _g: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real values read from a live mainnet SpotMarket (market_index 24)
    /// during offset verification.
    fn real_market() -> DriftSpotMarket {
        let cumulative_deposit_interest = 10_188_996_166f64 / CUMULATIVE_INTEREST_PRECISION;
        let cumulative_borrow_interest = 10_907_786_873f64 / CUMULATIVE_INTEREST_PRECISION;
        DriftSpotMarket {
            mint: 1,
            vault: 2,
            mint_decimals: 9,
            price_usd: 1_223_778f64 / PRICE_PRECISION,
            initial_asset_weight: 5000f64 / WEIGHT_PRECISION,
            deposit_amount: 5_285_092_133_951f64 * cumulative_deposit_interest,
            borrow_amount: 4_928_904_471_568f64 * cumulative_borrow_interest,
            cumulative_deposit_interest,
            cumulative_borrow_interest,
            optimal_utilization: 700_000f64 / PERCENTAGE_PRECISION,
            optimal_borrow_rate: 250_000f64 / PERCENTAGE_PRECISION,
            max_borrow_rate: 5_000_000f64 / PERCENTAGE_PRECISION,
            min_borrow_rate: 0.0,
            oracle: 3,
            market_index: 24,
        }
    }

    #[test]
    fn price_matches_real_market() {
        let m = real_market();
        assert!((m.price_usd - 1.223778).abs() < 1e-6);
    }

    #[test]
    fn ltv_matches_real_market() {
        let m = real_market();
        assert_eq!(m.initial_asset_weight, 0.5);
    }

    #[test]
    fn utilization_is_near_full_for_real_market() {
        let m = real_market();
        // Deposits (~5385 tokens) and borrows (~5377 tokens) are nearly
        // equal for this real, illiquid long-tail market -- utilization
        // should be pinned near 1.0.
        assert!(m.utilization() > 0.99);
    }

    #[test]
    fn borrow_apy_near_max_beyond_optimal_utilization() {
        let m = real_market();
        // Utilization (~99.9%) is far beyond optimal (70%), so the APY
        // should sit close to max_borrow_rate (500%).
        let apy = m.current_borrow_apy();
        assert!(apy > 4.5 && apy <= 5.0);
    }

    // Pure, host-independent PDA/discriminator checks only -- see the note
    // in `kamino.rs`'s test module for why the instruction-builder methods
    // themselves aren't unit-tested here.

    fn disc(name: &str) -> [u8; 8] {
        use sha2::Digest;
        let hash = sha2::Sha256::digest(format!("global:{name}").as_bytes());
        hash[..8].try_into().unwrap()
    }

    #[test]
    fn discriminators_match_their_instruction_names() {
        assert_eq!(disc("initialize_user_stats"), DISC_INITIALIZE_USER_STATS);
        assert_eq!(disc("initialize_user"), DISC_INITIALIZE_USER);
        assert_eq!(disc("deposit"), DISC_DEPOSIT);
        assert_eq!(disc("withdraw"), DISC_WITHDRAW);
        assert_eq!(disc("place_perp_order"), DISC_PLACE_PERP_ORDER);
    }

    /// `DISC_PLACE_PERP_ORDER` matches `disc("place_perp_order")` above
    /// (the general "any instruction's discriminator is computable from
    /// its name" check) -- this test additionally pins it to the exact
    /// literal bytes decoded live this session from a real, on-chain
    /// `place_perp_order` transaction's instruction data, so a future
    /// accidental edit to the constant's value (not just its derivation)
    /// would still be caught.
    #[test]
    fn place_perp_order_discriminator_matches_real_onchain_transaction() {
        assert_eq!(DISC_PLACE_PERP_ORDER, [69, 161, 93, 202, 120, 126, 76, 185]);
    }

    /// Pure, host-independent check of `place_perp_order`'s `OrderParams`
    /// byte encoding -- reconstructs the data buffer the same way
    /// `place_perp_order` does, inline, and checks each field lands at
    /// the right byte offset/value per the real IDL's field order
    /// (`sdk/src/idl/drift.json`, `OrderParams` type). Doesn't call
    /// `place_perp_order` itself (that needs a `Wallet`, which needs the
    /// WIT host import boundary this file's other instruction builders
    /// already stay clear of in tests).
    #[test]
    fn order_params_encoding_matches_idl_field_order() {
        let mut data = Vec::new();
        data.extend_from_slice(&DISC_PLACE_PERP_ORDER);
        data.push(OrderType::Market as u8);
        data.push(MarketType::Perp as u8);
        data.push(PositionDirection::Short as u8);
        data.push(0u8);
        data.extend_from_slice(&2_500_000_000u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&7u16.to_le_bytes());
        data.push(true as u8);
        data.push(PostOnlyParam::None as u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);
        data.push(OrderTriggerCondition::Above as u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);

        assert_eq!(&data[0..8], &DISC_PLACE_PERP_ORDER);
        assert_eq!(data[8], OrderType::Market as u8);
        assert_eq!(data[9], MarketType::Perp as u8);
        assert_eq!(data[10], PositionDirection::Short as u8);
        assert_eq!(data[11], 0); // user_order_id
        assert_eq!(u64::from_le_bytes(data[12..20].try_into().unwrap()), 2_500_000_000);
        assert_eq!(u64::from_le_bytes(data[20..28].try_into().unwrap()), 0); // price
        assert_eq!(u16::from_le_bytes(data[28..30].try_into().unwrap()), 7); // market_index
        assert_eq!(data[30], 1); // reduce_only = true
        assert_eq!(data[31], PostOnlyParam::None as u8);
        // bit_flags, maxTs(None), triggerPrice(None) tags
        assert_eq!(&data[32..35], &[0, 0, 0]);
        assert_eq!(data[35], OrderTriggerCondition::Above as u8);
        // oraclePriceOffset/auctionDuration/auctionStartPrice/auctionEndPrice, all None tags
        assert_eq!(&data[36..40], &[0, 0, 0, 0]);
        assert_eq!(data.len(), 40);
    }

    #[test]
    fn user_pda_is_deterministic_and_sub_account_specific() {
        let authority = Pubkey::new_unique();
        assert_eq!(user_pda(&authority, 0), user_pda(&authority, 0));
        assert_ne!(user_pda(&authority, 0), user_pda(&authority, 1));
    }

    #[test]
    fn spot_market_vault_pda_is_market_index_specific() {
        assert_ne!(spot_market_vault_pda(0), spot_market_vault_pda(1));
    }

    /// `parse_user`/`parse` themselves aren't unit-tested here -- both
    /// call `account_id_from_pubkey` internally (a WIT host import that
    /// aborts outside the real WASM guest runtime), same established
    /// boundary as every other `parse`-style function in this file (see
    /// `real_market()` above: it constructs `DriftSpotMarket` directly
    /// rather than calling `parse()`). `DriftUser`/`DriftPerpPosition`'s
    /// own logic (`is_open`, `perp_position_for`) has no such dependency
    /// and is fully testable directly.
    fn user_with_position(market_index: u16, base_asset_amount: i64, quote_entry_amount: i64) -> DriftUser {
        let mut perp_positions = [DriftPerpPosition::default(); NUM_PERP_POSITIONS];
        perp_positions[3] =
            DriftPerpPosition { market_index, base_asset_amount, quote_entry_amount, ..Default::default() };
        DriftUser { authority: 0, spot_positions: [DriftSpotPosition::default(); NUM_SPOT_POSITIONS], perp_positions }
    }

    fn user_with_spot_position(market_index: u16, scaled_balance: u64, is_borrow: bool) -> DriftUser {
        let mut spot_positions = [DriftSpotPosition::default(); NUM_SPOT_POSITIONS];
        spot_positions[1] = DriftSpotPosition { market_index, scaled_balance, is_borrow };
        DriftUser { authority: 0, spot_positions, perp_positions: [DriftPerpPosition::default(); NUM_PERP_POSITIONS] }
    }

    #[test]
    fn spot_position_for_finds_position_by_market_index() {
        let user = user_with_spot_position(0, 5_000_000_000, false);
        let pos = user.spot_position_for(0).expect("expected a spot position in market 0");
        assert_eq!(pos.scaled_balance, 5_000_000_000);
        assert!(!pos.is_borrow);
        assert!(user.spot_position_for(1).is_none(), "market 1 has no position");
    }

    #[test]
    fn spot_position_for_ignores_zero_scaled_balance() {
        let user = user_with_spot_position(2, 0, false);
        assert!(user.spot_position_for(2).is_none());
    }

    #[test]
    fn perp_position_for_finds_open_position_by_market_index() {
        // market_index=0 (SOL-PERP), long 2.5 SOL-equivalent base units,
        // entered at $150 notional -- arbitrary but internally consistent
        // synthetic values.
        let user = user_with_position(0, 2_500_000_000, 150_000_000);

        let pos = user.perp_position_for(0).expect("expected an open position in market 0");
        assert_eq!(pos.base_asset_amount, 2_500_000_000);
        assert_eq!(pos.quote_entry_amount, 150_000_000);
        assert!(pos.is_open());

        assert!(user.perp_position_for(1).is_none(), "market 1 has no position");
    }

    #[test]
    fn perp_position_for_ignores_zero_base_asset_amount() {
        // A slot with market_index set but base_asset_amount still 0
        // (e.g. a fully-closed position whose slot hasn't been reused
        // yet) must not be reported as an open position.
        let user = user_with_position(5, 0, 0);
        assert!(user.perp_position_for(5).is_none());
    }

    #[test]
    fn parse_user_rejects_too_short_body() {
        assert!(parse_user(&[0u8; 10]).is_none());
    }
}
