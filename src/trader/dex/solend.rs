//! Solend/Save Reserve account parser (credit-graph phase 2b — see
//! `~/cryptic-percolating-bunny.md`).
//!
//! [`SolendState`] wires reserve subscriptions into `DexState` so this data
//! stays live, feeding [`crate::trader::credit::CreditReserve`] the same as
//! Kamino (phase 1) and marginfi (phase 2a). Unlike marginfi, no separate
//! oracle subscription is needed here -- `price_usd` and the full
//! jump-rate curve are already fields directly on the Reserve account
//! itself (see `parse()`), so `SolendState` only ever tracks one account
//! type.
//!
//! # Account layout — Solend Reserve (no Anchor discriminator, size 619)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        1   version
//!   1        8   last_update_slot
//!   9        1   last_update_stale
//!  10       32   lending_market
//!  42       32   liquidity.mint_pubkey
//!  74        1   liquidity.mint_decimals
//!  75       32   liquidity.supply_vault
//! 107       32   liquidity.pyth_oracle_pubkey
//! 139       32   liquidity.switchboard_oracle_pubkey
//! 171        8   liquidity.available_amount            ← raw liquid supply
//! 179       16   liquidity.borrowed_amount_wads         ← WAD (1e18) scaled
//! 195       16   liquidity.cumulative_borrow_rate_wads
//! 211       16   liquidity.market_price                 ← WAD-scaled USD price
//! 227       32   collateral.mint_pubkey
//! 259        8   collateral.mint_total_supply
//! 267       32   collateral.supply_vault
//! 299        1   config.optimal_utilization_rate (0-100)
//! 300        1   config.loan_to_value_ratio (0-100)     ← LTV equivalent
//! 301        1   config.liquidation_bonus
//! 302        1   config.liquidation_threshold
//! 303        1   config.min_borrow_rate
//! 304        1   config.optimal_borrow_rate
//! 305        1   config.max_borrow_rate
//! 306        8   config.fees.borrow_fee_wad
//! 314        8   config.fees.flash_loan_fee_wad
//! 322        1   config.fees.host_fee_percentage
//! 323        8   config.deposit_limit
//! 331        8   config.borrow_limit
//! 339       32   config.fee_receiver
//! 371        1   config.protocol_liquidation_fee
//! 372        1   config.protocol_take_rate
//! 470        1   config.max_utilization_rate
//! 471        8   config.super_max_borrow_rate (percentage, not capped at 255)
//! ```
//!
//! Every field above was cross-checked against a real mainnet Reserve
//! account (a USDT reserve): `market_price` read back as `$0.9987` (right
//! at the USDT peg), `loan_to_value_ratio` = 70 with `liquidation_threshold`
//! = 77 (the expected LTV-below-threshold relationship), and
//! `available_amount`/`borrowed_amount_wads` combined to a plausible ~54%
//! pool utilization. The fields from offset 306 onward were added in a
//! later pass and independently cross-checked against a live SOL reserve
//! (`fee_receiver`, `protocol_take_rate=20`, `max_utilization_rate=90`,
//! `super_max_borrow_rate=300` -- all real, non-default values) -- see
//! `credit-graph`'s Solend verification notes for the transaction/account
//! trail. Unlike Kamino (whose public source drifted from the deployed
//! binary), Solend's public `solend-sdk`/`spl-token-lending` source
//! matched the real account byte-for-byte for the Reserve layout itself --
//! see the equivalent verification already done for the Go-side fetcher
//! (`optimizer/prefetch/solend`). **Important caveat, found the hard way**:
//! matching the *Reserve layout* is not the same as matching the
//! *instruction account lists* -- the `master` branch of that same source
//! (what the instruction builders below were originally built against) is
//! stale relative to what's actually deployed; the `mainnet` branch is the
//! one that matches real, successful transactions (see each instruction
//! builder's own doc comment).
//!
//! "Wads" are `Decimal` fixed-point values: `value = raw_u128 / 1e18`.

use crate::{
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionRequest},
    trader::{dex::update::Updater, pricegraph::TradeRouter, types::TraderError},
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
};
use solana_sdk_ids::sysvar::rent as rent_sysvar;
use solana_system_interface::instruction::create_account_with_seed;
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

pub const SOLEND_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo");

/// Solend's real main pool -- confirmed via `api.solend.fi/v1/markets/configs`
/// (`name: "main", isPrimary: true`), the one with real, liquid SOL/USDC/ETH
/// reserves (`8PbodeaosQP19SjYFx855UMqWxH2HynZLdBXmsrbac36`/
/// `BgxfHJDzm44T7XG68MYKx7YisTjZu73tVovyZSjJMpmw`/
/// `CPDiKagfozERtJ33p7HHhEfJERjvfk1VAjMXAFLrvrKP` respectively -- no BTC
/// reserve exists on it at all). Solend runs dozens of other markets (long-tail/
/// meme-coin pools, per the same API) that also happen to have reserves for
/// these same mints -- `reserve_by_mint` must filter to this address, same
/// "multi-market ambiguity" reasoning as Kamino's `KAMINO_MAIN_MARKET`.
pub const SOLEND_MAIN_MARKET: Pubkey =
    Pubkey::from_str_const("4UpD2fh7xH3VP9QQaXtsS1YY3bxzWhtfpks7FatyKvdY");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const OFF_LENDING_MARKET: usize = 10;
const OFF_MINT: usize = 42;
const OFF_MINT_DECIMALS: usize = 74;
const OFF_SUPPLY_VAULT: usize = 75;
const OFF_PYTH_ORACLE: usize = 107;
const OFF_SWITCHBOARD_ORACLE: usize = 139;
const OFF_AVAILABLE_AMOUNT: usize = 171;
const OFF_BORROWED_AMOUNT_WADS: usize = 179;
const OFF_MARKET_PRICE: usize = 211;
const OFF_COLLATERAL_MINT: usize = 227;
const OFF_COLLATERAL_SUPPLY_VAULT: usize = 267;
const OFF_OPTIMAL_UTILIZATION_RATE: usize = 299;
const OFF_LOAN_TO_VALUE_RATIO: usize = 300;
const OFF_LIQUIDATION_THRESHOLD: usize = 302;
const OFF_MIN_BORROW_RATE: usize = 303;
const OFF_OPTIMAL_BORROW_RATE: usize = 304;
const OFF_MAX_BORROW_RATE: usize = 305;
const OFF_FEE_RECEIVER: usize = 339;
const OFF_PROTOCOL_TAKE_RATE: usize = 372;
const OFF_MAX_UTILIZATION_RATE: usize = 470;
const OFF_SUPER_MAX_BORROW_RATE: usize = 471;

const MIN_RESERVE_LEN: usize = OFF_SUPER_MAX_BORROW_RATE + 8;

/// Solend's Obligation is a *plain* account (`assert_uninitialized`, no
/// on-chain seed derivation), unlike Kamino's PDA-based obligation -- but
/// deriving its address via `create_account_with_seed` (rather than a fresh
/// keypair) keeps it just as deterministic: recomputable from `owner` alone,
/// nothing to remember separately. `id` (see [`obligation_seed`]) lets one
/// owner hold more than one independent obligation -- same real reason
/// Kamino's own `obligation_pda` takes an `id: u8`: every bot mode's Go side
/// derives its child wallet via the same index
/// (`common.DeriveChildKeyFromIndex(parentKey, 1)`), so two bot modes
/// sharing a fee-payer would otherwise collide on the same obligation
/// address.
const OBLIGATION_SEED: &str = "solend-obligation";

/// `id=0` produces the exact same seed string as before this was
/// parameterized -- critical: `testperpv1` (and the now-retired
/// perpfundingv1 before it) may have real, currently-open mainnet
/// obligations at that exact address, and this must never change under
/// them. `create_with_seed`'s real limit is 32 bytes;
/// worst case (`id=255`) is `"solend-obligation-255"`, 22 bytes -- safe with
/// room to spare for any `u8`.
fn obligation_seed(id: u8) -> String {
    if id == 0 { OBLIGATION_SEED.to_string() } else { format!("{OBLIGATION_SEED}-{id}") }
}

/// Solend's Obligation account is a fixed 1300 bytes
/// (`token-lending/program/src/state/obligation.rs::OBLIGATION_LEN`).
const OBLIGATION_LEN: u64 = 1300;

// ─── Obligation account layout (fixed-slot, not Borsh -- same manual
// scheme as Reserve) -- live-verified against a real Obligation account,
// cross-checked field-for-field against a real BorrowObligationLiquidity
// transaction that touched it (its deposit_reserve/borrow_reserve pubkeys
// matched the transaction's own account list exactly). ─────────────────
const OFF_OB_OWNER: usize = 42;
const OFF_OB_DEPOSITS_LEN: usize = 202;
const OFF_OB_BORROWS_LEN: usize = 203;
const OFF_OB_DATA_FLAT: usize = 204;
const OBLIGATION_COLLATERAL_LEN: usize = 88;
const OBLIGATION_LIQUIDITY_LEN: usize = 112;
const OFF_OC_DEPOSIT_RESERVE: usize = 0;
const OFF_OC_DEPOSITED_AMOUNT: usize = 32;
const OFF_OL_BORROW_RESERVE: usize = 0;
const OFF_OL_BORROWED_AMOUNT_WADS: usize = 48;

const MIN_OBLIGATION_LEN: usize = OFF_OB_DATA_FLAT;

/// Anchor-style discriminators don't apply here -- Solend (a
/// `spl-token-lending` fork) is a plain Borsh instruction enum with a
/// leading `u8` tag, confirmed against
/// `solendprotocol/solana-program-library`'s `instruction.rs`.
const TAG_REFRESH_RESERVE: u8 = 3;
const TAG_INIT_OBLIGATION: u8 = 6;
const TAG_REFRESH_OBLIGATION: u8 = 7;
const TAG_BORROW_OBLIGATION_LIQUIDITY: u8 = 10;
const TAG_REPAY_OBLIGATION_LIQUIDITY: u8 = 11;
const TAG_DEPOSIT_RESERVE_LIQUIDITY_AND_OBLIGATION_COLLATERAL: u8 = 14;
const TAG_WITHDRAW_OBLIGATION_COLLATERAL_AND_REDEEM_RESERVE_COLLATERAL: u8 = 15;

pub const SOLEND_CREATE_OBLIGATION_CU: u32 = 30_000;
pub const SOLEND_INIT_OBLIGATION_CU: u32 = 30_000;
pub const SOLEND_REFRESH_RESERVE_CU: u32 = 40_000;
pub const SOLEND_REFRESH_OBLIGATION_BASE_CU: u32 = 20_000;
pub const SOLEND_REFRESH_OBLIGATION_PER_RESERVE_CU: u32 = 12_000;
pub const SOLEND_DEPOSIT_CU: u32 = 120_000;
pub const SOLEND_BORROW_CU: u32 = 120_000;
pub const SOLEND_REPAY_CU: u32 = 90_000;
pub const SOLEND_WITHDRAW_CU: u32 = 120_000;

/// `u64::MAX` signals "borrow up to 100% of borrowing power" / "repay 100%
/// of borrowed amount" / "withdraw up to 100% of deposited amount" -- per
/// the doc comments on `LendingInstruction::BorrowObligationLiquidity`/
/// `RepayObligationLiquidity`/`WithdrawObligationCollateral`.
pub const SOLEND_AMOUNT_MAX: u64 = u64::MAX;

/// Scale for Solend's `Decimal` "wad" fixed-point values: `1e18`.
const WAD_SCALE: f64 = 1_000_000_000_000_000_000.0;

/// Parsed subset of a Solend Reserve account -- only the fields needed for
/// [`crate::trader::credit::CreditReserve::from_solend`] and a jump-rate
/// borrow APY estimate.
#[derive(Debug, Default, Clone)]
pub struct SolendReserve {
    pub lending_market: AccountId,
    pub mint: AccountId,
    pub mint_decimals: u8,
    /// Vault holding this reserve's liquid supply -- source/destination for
    /// deposit/borrow/repay/withdraw.
    pub supply_vault: AccountId,
    /// Primary price oracle. Blank (`Pubkey::default()`) on reserves that
    /// only use Switchboard.
    pub pyth_oracle: AccountId,
    /// Secondary/fallback price oracle. Often blank on reserves that only
    /// use Pyth -- that's expected, not a parse error.
    pub switchboard_oracle: AccountId,
    /// This reserve's cToken mint.
    pub collateral_mint: AccountId,
    /// This reserve's cToken supply vault.
    pub collateral_supply_vault: AccountId,
    /// Raw liquid supply available to borrow (native token units).
    pub available_amount: u64,
    /// Currently borrowed (native token units, converted down from the WAD
    /// fixed-point `borrowed_amount_wads`).
    pub borrowed_amount: f64,
    /// Oracle price in USD.
    pub price_usd: f64,
    /// Max loan-to-value ratio (0.0-1.0) when this reserve's asset is
    /// deposited as collateral.
    pub loan_to_value_pct: f64,
    pub liquidation_threshold_pct: f64,
    pub optimal_utilization_rate: f64,
    pub min_borrow_rate: f64,
    pub optimal_borrow_rate: f64,
    pub max_borrow_rate: f64,
    /// Utilization (0-100) above which the real deployed curve switches to
    /// a third, steep "emergency" segment up to `super_max_borrow_rate` --
    /// see [`Self::current_borrow_apy`]'s doc comment.
    pub max_utilization_rate: f64,
    /// Borrow rate (percent, e.g. `300.0` = 300%) at 100% utilization --
    /// not capped at 255 like the other rate fields, hence `u64` on-chain.
    pub super_max_borrow_rate: f64,
    /// This reserve's origination-fee vault -- destination for the
    /// borrow-fee cut on every `BorrowObligationLiquidity`.
    pub fee_receiver: AccountId,
    /// Protocol's cut (0-100) of borrower interest -- see
    /// [`Self::current_supply_apy`].
    pub protocol_take_rate: f64,
}

impl SolendReserve {
    /// Current utilization (0.0-1.0): borrowed / (available + borrowed).
    pub fn utilization(&self) -> f64 {
        let total = self.available_amount as f64 + self.borrowed_amount;
        if total <= 0.0 {
            return 0.0;
        }
        self.borrowed_amount / total
    }

    /// Estimate the current borrow APY (0.0-1.0 fraction) from the real
    /// deployed **three-segment** jump-rate model (the public source this
    /// file was originally built against only has two -- confirmed via
    /// live verification that the deployed curve has a third, steep
    /// "emergency" tier between `max_utilization_rate` and 100%, e.g. real
    /// SOL-reserve values `optimal_utilization_rate=80,
    /// max_utilization_rate=90, max_borrow_rate=10%,
    /// super_max_borrow_rate=300%`): linear `min_borrow_rate` ->
    /// `optimal_borrow_rate` up to `optimal_utilization_rate`, then linear
    /// `optimal_borrow_rate` -> `max_borrow_rate` up to
    /// `max_utilization_rate`, then linear `max_borrow_rate` ->
    /// `super_max_borrow_rate` up to 100%.
    pub fn current_borrow_apy(&self) -> f64 {
        let util_pct = self.utilization() * 100.0;
        let (lo, hi, seg_start, seg_end) = if util_pct <= self.optimal_utilization_rate {
            (self.min_borrow_rate, self.optimal_borrow_rate, 0.0, self.optimal_utilization_rate)
        } else if util_pct <= self.max_utilization_rate {
            (self.optimal_borrow_rate, self.max_borrow_rate, self.optimal_utilization_rate, self.max_utilization_rate)
        } else {
            (self.max_borrow_rate, self.super_max_borrow_rate, self.max_utilization_rate, 100.0)
        };
        let denom = seg_end - seg_start;
        let frac = if denom <= 0.0 { 1.0 } else { ((util_pct - seg_start) / denom).clamp(0.0, 1.0) };
        (lo + (hi - lo) * frac) / 100.0
    }

    /// Real supply (lender) APY, derived directly from the deployed
    /// accrual mechanics (not assumed): each period lenders' claim on the
    /// reserve grows by `net_new_debt * (1 - protocol_take_rate)`, so
    /// `supply_apy = borrow_apy * utilization * (1 - protocol_take_rate)`
    /// -- the standard lending-market identity, with `protocol_take_rate`
    /// a real, live, per-reserve field (not a guessed constant -- e.g.
    /// live-verified `20%` on the SOL reserve, don't assume it's uniform
    /// across reserves).
    pub fn current_supply_apy(&self) -> f64 {
        self.current_borrow_apy() * self.utilization() * (1.0 - self.protocol_take_rate / 100.0)
    }

    // ─── Instruction builders ─────────────────────────────────────────────

    /// Append a `RefreshReserve` (tag 3) instruction to `wallet`. Must
    /// immediately precede any deposit/borrow/repay/withdraw touching this
    /// reserve or an obligation referencing it, in the same slot.
    ///
    /// Real, live-verified account list (via a real, successful mainnet
    /// transaction -- the deployed program reads Clock via the `Clock::get()`
    /// syscall, NOT as an explicit account, unlike the stale `master`-branch
    /// reference this builder was originally written against): `[reserve,
    /// pyth_oracle, switchboard_oracle]`. An optional 4th "extra oracle"
    /// account exists on the real program (only used when a reserve's
    /// `config.extra_oracle_pubkey` is set) but wasn't observed live on any
    /// reserve sampled -- not modeled here.
    pub fn refresh_reserve(&self, reserve_id: AccountId, wallet: &mut Wallet) -> Result<(), TraderError> {
        let reserve_pk = resolve(reserve_id)?;
        wallet.append_ix(
            Instruction {
                program_id: SOLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new_readonly(resolve(self.pyth_oracle)?, false),
                    AccountMeta::new_readonly(resolve(self.switchboard_oracle)?, false),
                ],
                data: vec![TAG_REFRESH_RESERVE],
            },
            SOLEND_REFRESH_RESERVE_CU,
        );
        Ok(())
    }

    /// Append a `DepositReserveLiquidityAndObligationCollateral` (tag 14)
    /// instruction to `wallet`. `obligation` must already exist (see
    /// [`init_obligation`]). `user_collateral_account` is a scratch token
    /// account of `collateral_mint` -- the deposited cTokens pass through it
    /// on their way into obligation collateral (Solend's own client SDKs
    /// typically reuse the user's associated token account for the
    /// collateral mint here).
    ///
    /// Real, live-verified 13-account list (via 2 real, successful mainnet
    /// transactions -- no Clock account; the deployed program reads it via
    /// syscall, unlike the stale `master`-branch reference this builder was
    /// originally written against).
    pub fn deposit(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        liquidity_amount: u64,
        owner: AccountId,
        user_source_liquidity: AccountId,
        user_collateral_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let collateral_mint_pk = resolve(self.collateral_mint)?;
        let user_source_liquidity_pk = resolve(user_source_liquidity)?;
        let user_collateral_pk = resolve(user_collateral_account)?;

        let mut data = vec![TAG_DEPOSIT_RESERVE_LIQUIDITY_AND_OBLIGATION_COLLATERAL];
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: SOLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(user_source_liquidity_pk, false),
                    AccountMeta::new(user_collateral_pk, false),
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new(supply_vault_pk, false),
                    AccountMeta::new(collateral_mint_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new_readonly(lma_pk, false),
                    AccountMeta::new(resolve(self.collateral_supply_vault)?, false),
                    AccountMeta::new(obligation_pk, false),
                    AccountMeta::new_readonly(owner_pk, true), // obligation_owner
                    AccountMeta::new_readonly(resolve(self.pyth_oracle)?, false),
                    AccountMeta::new_readonly(resolve(self.switchboard_oracle)?, false),
                    AccountMeta::new_readonly(owner_pk, true), // user_transfer_authority
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
                ],
                data,
            },
            SOLEND_DEPOSIT_CU,
        );
        Ok(())
    }

    /// Append a `BorrowObligationLiquidity` (tag 10) instruction to
    /// `wallet`. Pass [`SOLEND_AMOUNT_MAX`] to borrow up to 100% of
    /// borrowing power. Requires this reserve and `obligation` to have been
    /// refreshed in the same slot ([`Self::refresh_reserve`] +
    /// [`refresh_obligation`], listing every reserve on the obligation).
    /// No host fee receiver (this bot isn't a Solend referrer).
    ///
    /// `deposit_reserves` must list every reserve `obligation` currently
    /// has a *deposit* in (writable "borrow attribution" accounts the
    /// deployed program requires, one per `obligation.deposits[i]` -- this
    /// bot doesn't track an obligation's contents yet, so the caller must
    /// supply the list, same reasoning as [`refresh_obligation`]'s existing
    /// `reserves` parameter).
    ///
    /// Real, live-verified 9(+N)-account list (`fee_receiver` -- now parsed
    /// directly from this reserve's own account data at a live-verified
    /// offset, no longer caller-supplied -- confirmed at its real position;
    /// no Clock account, unlike the stale `master`-branch reference this
    /// builder was originally written against). `lending_market` is
    /// **writable** here (unlike every other Solend instruction in this
    /// file, and unlike what a first read of the public `master`-branch
    /// source suggests) -- confirmed against a real, successful mainnet
    /// `BorrowObligationLiquidity` transaction
    /// (`MH3wpwEdbvcb9M6tPtmEDy5BYeMrufw2fD4FyyJTrfo7Fz3n2kK6wFcS24Jab78a1yvwq7GpdgU95BxvYWj2osH`):
    /// `lending_market` appears in that transaction's account list exactly
    /// once, inside this instruction, so its writable flag can't be a
    /// side effect of some other instruction in the same transaction (or
    /// of fee-payer status) -- it's this instruction's own requirement.
    /// Marking it read-only was the real cause of a live "instruction
    /// modified data of a read-only account" failure.
    pub fn borrow(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        liquidity_amount: u64,
        owner: AccountId,
        user_destination_liquidity: AccountId,
        deposit_reserves: &[AccountId],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let dest_liq_pk = resolve(user_destination_liquidity)?;
        let fee_receiver_pk = resolve(self.fee_receiver)?;

        let mut data = vec![TAG_BORROW_OBLIGATION_LIQUIDITY];
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new(supply_vault_pk, false), // source_liquidity (reserve's own vault)
            AccountMeta::new(dest_liq_pk, false),
            AccountMeta::new(reserve_pk, false),
            AccountMeta::new(fee_receiver_pk, false),
            AccountMeta::new(obligation_pk, false),
            AccountMeta::new(lm_pk, false),
            AccountMeta::new_readonly(lma_pk, false),
            AccountMeta::new_readonly(owner_pk, true),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        for &id in deposit_reserves {
            accounts.push(AccountMeta::new(resolve(id)?, false));
        }

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: SOLEND_PROGRAM_ID,
                accounts,
                data,
            },
            SOLEND_BORROW_CU,
        );
        Ok(())
    }

    /// Append a `RepayObligationLiquidity` (tag 11) instruction to
    /// `wallet`. Pass [`SOLEND_AMOUNT_MAX`] to repay 100% of the borrowed
    /// amount.
    ///
    /// Real, live-verified 7-account list (via a real, successful mainnet
    /// transaction -- no Clock account, unlike the stale `master`-branch
    /// reference this builder was originally written against).
    pub fn repay(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        liquidity_amount: u64,
        owner: AccountId,
        user_source_liquidity: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let reserve_pk = resolve(reserve_id)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let user_source_liquidity_pk = resolve(user_source_liquidity)?;

        let mut data = vec![TAG_REPAY_OBLIGATION_LIQUIDITY];
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: SOLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(user_source_liquidity_pk, false),
                    AccountMeta::new(supply_vault_pk, false), // destination_liquidity (reserve's own vault)
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new(obligation_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new_readonly(owner_pk, true), // user_transfer_authority
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
                ],
                data,
            },
            SOLEND_REPAY_CU,
        );
        Ok(())
    }

    /// Append a `WithdrawObligationCollateralAndRedeemReserveCollateral`
    /// (tag 15) instruction to `wallet`. Pass [`SOLEND_AMOUNT_MAX`] to
    /// withdraw 100% of the deposited amount. `user_collateral_account` is a
    /// scratch token account of `collateral_mint` -- the withdrawn cTokens
    /// pass through it on their way to being redeemed (see
    /// [`Self::deposit`]'s doc comment for the same pattern in reverse).
    /// Requires this reserve and `obligation` refreshed in the same slot;
    /// full oracle-price freshness only matters if the obligation has
    /// active borrows (see klend's equivalent, `Self::withdraw` doc comment
    /// -- Solend enforces the same slot-freshness-only-unless-borrowing
    /// rule via `is_stale`).
    ///
    /// `deposit_reserves` -- see [`Self::borrow`]'s doc comment for why
    /// this is required (same mandatory "borrow attribution" accounts,
    /// appended here after `token_program`).
    ///
    /// Real, live-verified 12(+N)-account list (via 4 real, successful
    /// mainnet transactions -- no Clock account, unlike the stale
    /// `master`-branch reference this builder was originally written
    /// against).
    pub fn withdraw(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        collateral_amount: u64,
        owner: AccountId,
        user_destination_liquidity: AccountId,
        user_collateral_account: AccountId,
        deposit_reserves: &[AccountId],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let collateral_supply_pk = resolve(self.collateral_supply_vault)?;
        let collateral_mint_pk = resolve(self.collateral_mint)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let user_collateral_pk = resolve(user_collateral_account)?;
        let user_liquidity_pk = resolve(user_destination_liquidity)?;

        let mut data = vec![TAG_WITHDRAW_OBLIGATION_COLLATERAL_AND_REDEEM_RESERVE_COLLATERAL];
        data.extend_from_slice(&collateral_amount.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new(collateral_supply_pk, false), // reserve_collateral (source)
            AccountMeta::new(user_collateral_pk, false),
            AccountMeta::new(reserve_pk, false),
            AccountMeta::new(obligation_pk, false),
            // Real, live-verified: a successful mainnet
            // WithdrawObligationCollateralAndRedeemReserveCollateral
            // transaction (55TYjzes7AksVYr98nNeWE9DL4tR6qCn3dtwW89oHujbJN7kuiaPENb8S94wiNKRUihX5ccnr2c8NZVW5aMu492u)
            // has `lending_market` writable here -- marking it read-only
            // (as this builder did before) makes the real program fail
            // with `ReadonlyDataModified`, confirmed live via
            // testperpv1's own withdraw attempt. Deposit/borrow/repay
            // (elsewhere in this file) keep `lm_pk` read-only -- untouched,
            // since only withdraw has been live-confirmed to need this.
            AccountMeta::new(lm_pk, false),
            AccountMeta::new_readonly(lma_pk, false),
            AccountMeta::new(user_liquidity_pk, false),
            AccountMeta::new(collateral_mint_pk, false),
            AccountMeta::new(supply_vault_pk, false), // reserve_liquidity_supply
            AccountMeta::new_readonly(owner_pk, true), // obligation_owner
            AccountMeta::new_readonly(owner_pk, true), // user_transfer_authority
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        for &id in deposit_reserves {
            accounts.push(AccountMeta::new(resolve(id)?, false));
        }

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: SOLEND_PROGRAM_ID,
                accounts,
                data,
            },
            SOLEND_WITHDRAW_CU,
        );
        Ok(())
    }
}

// ─── PDA / seeded-address derivation ───────────────────────────────────────────

/// `[lending_market]` -- Solend's lending market authority PDA. Unlike
/// Kamino's (which needs a `"lma"` prefix seed), Solend derives this with no
/// extra prefix, confirmed against `spl-token-lending`'s
/// `deposit_reserve_liquidity`/every other builder in `instruction.rs`.
fn lending_market_authority(lending_market: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[lending_market.as_ref()], &SOLEND_PROGRAM_ID).0
}

/// The deterministic, `create_account_with_seed`-derived address of
/// `owner`'s `id`-th Solend obligation (see [`OBLIGATION_SEED`]'s doc
/// comment for why this is used instead of a fresh keypair or a PDA --
/// Solend's Obligation isn't a PDA on-chain -- and for what `id` is for).
pub fn obligation_address(owner: &Pubkey, id: u8) -> Pubkey {
    Pubkey::create_with_seed(owner, &obligation_seed(id), &SOLEND_PROGRAM_ID)
        .expect("obligation_seed(id) is always a valid create-with-seed seed")
}

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

// ─── Position-account lifecycle ────────────────────────────────────────────────

/// Append a `system_instruction::create_account_with_seed` instruction to
/// `wallet`, allocating (but not yet initializing) `owner`'s `id`-th
/// obligation account. Must succeed before [`init_obligation`]. `owner`
/// acts as both the seed base and the fee payer -- this bot operates with a
/// single signing wallet.
pub fn create_obligation_account(owner: AccountId, id: u8, wallet: &mut Wallet) -> Result<AccountId, TraderError> {
    let owner_pk = resolve(owner)?;
    let obligation_pk = obligation_address(&owner_pk, id);
    let lamports = Rent::default().minimum_balance(OBLIGATION_LEN as usize);

    wallet.require_signer(owner);
    wallet.append_ix(
        create_account_with_seed(
            &owner_pk,
            &obligation_pk,
            &owner_pk,
            &obligation_seed(id),
            lamports,
            OBLIGATION_LEN,
            &SOLEND_PROGRAM_ID,
        ),
        SOLEND_CREATE_OBLIGATION_CU,
    );
    Ok(account_id_from_pubkey(&obligation_pk))
}

/// Append an `InitObligation` (tag 6) instruction to `wallet`. Requires
/// [`create_obligation_account`] to have already succeeded for `owner`.
///
/// **Verified** via `simulateTransaction` against real mainnet state (no
/// real historical transaction was found -- it's a rare, one-time-per-
/// wallet call -- so this was confirmed by simulating this exact account
/// list/data alongside a real `create_account_with_seed` in the same
/// transaction, using a real funded owner with no prior obligation):
/// `err: null`, program logs show `Instruction: Init Obligation` ->
/// `success`, 4699 compute units consumed. Same account list this
/// codebase already uses (no Clock account -- read via syscall on the
/// deployed program, unlike the stale `master`-branch reference every
/// builder in this file was originally written against; Rent still
/// explicit).
pub fn init_obligation(
    owner: AccountId,
    lending_market: AccountId,
    id: u8,
    wallet: &mut Wallet,
) -> Result<AccountId, TraderError> {
    let owner_pk = resolve(owner)?;
    let lm_pk = resolve(lending_market)?;
    let obligation_pk = obligation_address(&owner_pk, id);

    wallet.require_signer(owner);
    wallet.append_ix(
        Instruction {
            program_id: SOLEND_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(obligation_pk, false),
                AccountMeta::new_readonly(lm_pk, false),
                AccountMeta::new_readonly(owner_pk, true),
                AccountMeta::new_readonly(rent_sysvar::ID, false),
                AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
            ],
            data: vec![TAG_INIT_OBLIGATION],
        },
        SOLEND_INIT_OBLIGATION_CU,
    );
    Ok(account_id_from_pubkey(&obligation_pk))
}

/// Append a `RefreshObligation` (tag 7) instruction to `wallet`.
/// `reserves` must list every reserve this obligation currently has a
/// deposit or borrow position in (order doesn't matter to Solend, unlike
/// Kamino) -- this bot doesn't track a user's obligation contents, so the
/// caller must supply the list (see the credit-graph module's non-goals).
///
/// Real, live-verified account list (via a real, successful mainnet
/// transaction -- no Clock account, unlike the stale `master`-branch
/// reference this builder was originally written against): `[obligation,
/// ...reserves]`.
pub fn refresh_obligation(
    obligation: AccountId,
    reserves: &[AccountId],
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let obligation_pk = resolve(obligation)?;

    let mut accounts = Vec::with_capacity(1 + reserves.len());
    accounts.push(AccountMeta::new(obligation_pk, false));
    for &id in reserves {
        accounts.push(AccountMeta::new_readonly(resolve(id)?, false));
    }

    let cu = SOLEND_REFRESH_OBLIGATION_BASE_CU
        + reserves.len() as u32 * SOLEND_REFRESH_OBLIGATION_PER_RESERVE_CU;

    wallet.append_ix(
        Instruction {
            program_id: SOLEND_PROGRAM_ID,
            accounts,
            data: vec![TAG_REFRESH_OBLIGATION],
        },
        cu,
    );
    Ok(())
}

/// Parse a Solend Reserve account. `body` must be the full, untouched
/// account data (no discriminator to strip).
pub fn parse(body: &[u8]) -> Option<SolendReserve> {
    if body.len() < MIN_RESERVE_LEN {
        return None;
    }

    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_wad = |off: usize| -> f64 {
        u128::from_le_bytes(body[off..off + 16].try_into().unwrap()) as f64 / WAD_SCALE
    };
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };

    Some(SolendReserve {
        lending_market: read_pk(OFF_LENDING_MARKET),
        mint: read_pk(OFF_MINT),
        mint_decimals: body[OFF_MINT_DECIMALS],
        supply_vault: read_pk(OFF_SUPPLY_VAULT),
        pyth_oracle: read_pk(OFF_PYTH_ORACLE),
        switchboard_oracle: read_pk(OFF_SWITCHBOARD_ORACLE),
        collateral_mint: read_pk(OFF_COLLATERAL_MINT),
        collateral_supply_vault: read_pk(OFF_COLLATERAL_SUPPLY_VAULT),
        available_amount: read_u64(OFF_AVAILABLE_AMOUNT),
        borrowed_amount: read_wad(OFF_BORROWED_AMOUNT_WADS),
        price_usd: read_wad(OFF_MARKET_PRICE),
        loan_to_value_pct: body[OFF_LOAN_TO_VALUE_RATIO] as f64 / 100.0,
        liquidation_threshold_pct: body[OFF_LIQUIDATION_THRESHOLD] as f64 / 100.0,
        optimal_utilization_rate: body[OFF_OPTIMAL_UTILIZATION_RATE] as f64,
        min_borrow_rate: body[OFF_MIN_BORROW_RATE] as f64,
        optimal_borrow_rate: body[OFF_OPTIMAL_BORROW_RATE] as f64,
        max_borrow_rate: body[OFF_MAX_BORROW_RATE] as f64,
        max_utilization_rate: body[OFF_MAX_UTILIZATION_RATE] as f64,
        super_max_borrow_rate: read_u64(OFF_SUPER_MAX_BORROW_RATE) as f64,
        fee_receiver: read_pk(OFF_FEE_RECEIVER),
        protocol_take_rate: body[OFF_PROTOCOL_TAKE_RATE] as f64,
    })
}

/// One entry from an Obligation's `deposits` array -- collateral backing
/// the obligation in a given reserve.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ObligationCollateral {
    pub deposit_reserve: AccountId,
    /// Raw token units, already native -- no WAD conversion needed.
    pub deposited_amount: u64,
}

/// One entry from an Obligation's `borrows` array -- an outstanding loan
/// against a given reserve.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ObligationLiquidity {
    pub borrow_reserve: AccountId,
    /// WAD-scaled (`/1e18`) -- already converted here, unlike
    /// `ObligationCollateral::deposited_amount`.
    pub borrowed_amount: u64,
}

/// Parsed subset of a Solend Obligation account -- this bot's own lending
/// position, not a reserve. Mirrors `dex::drift::DriftUser`'s shape
/// (fixed-slot struct, find-by-key accessors) for the equivalent role on
/// the Drift side.
#[derive(Debug, Default, Clone)]
pub struct SolendObligation {
    pub owner: AccountId,
    pub deposits: Vec<ObligationCollateral>,
    pub borrows: Vec<ObligationLiquidity>,
}

impl SolendObligation {
    /// The deposit (if any) in `reserve`.
    pub fn deposit_for(&self, reserve: AccountId) -> Option<&ObligationCollateral> {
        self.deposits.iter().find(|d| d.deposit_reserve == reserve)
    }

    /// The borrow (if any) against `reserve`.
    pub fn borrow_for(&self, reserve: AccountId) -> Option<&ObligationLiquidity> {
        self.borrows.iter().find(|b| b.borrow_reserve == reserve)
    }
}

/// Parse a Solend Obligation account from raw body bytes. Live-verified,
/// fixed-slot layout (not Borsh, despite Solend using Borsh for
/// instruction data) -- see this module's "Obligation account layout"
/// section comment for the full offset table. `deposits_len`/
/// `borrows_len` entries are read from a single flat byte run starting at
/// [`OFF_OB_DATA_FLAT`]: all `ObligationCollateral` entries first, then
/// all `ObligationLiquidity` entries.
pub fn parse_obligation(body: &[u8]) -> Option<SolendObligation> {
    if body.len() < MIN_OBLIGATION_LEN {
        return None;
    }
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_wad_u64 = |off: usize| -> u64 {
        (u128::from_le_bytes(body[off..off + 16].try_into().unwrap()) as f64 / WAD_SCALE) as u64
    };

    let deposits_len = body[OFF_OB_DEPOSITS_LEN] as usize;
    let borrows_len = body[OFF_OB_BORROWS_LEN] as usize;
    let deposits_end = OFF_OB_DATA_FLAT + deposits_len * OBLIGATION_COLLATERAL_LEN;
    let borrows_end = deposits_end + borrows_len * OBLIGATION_LIQUIDITY_LEN;
    if body.len() < borrows_end {
        return None;
    }

    let mut deposits = Vec::with_capacity(deposits_len);
    for i in 0..deposits_len {
        let base = OFF_OB_DATA_FLAT + i * OBLIGATION_COLLATERAL_LEN;
        deposits.push(ObligationCollateral {
            deposit_reserve: read_pk(base + OFF_OC_DEPOSIT_RESERVE),
            deposited_amount: read_u64(base + OFF_OC_DEPOSITED_AMOUNT),
        });
    }

    let mut borrows = Vec::with_capacity(borrows_len);
    for i in 0..borrows_len {
        let base = deposits_end + i * OBLIGATION_LIQUIDITY_LEN;
        borrows.push(ObligationLiquidity {
            borrow_reserve: read_pk(base + OFF_OL_BORROW_RESERVE),
            borrowed_amount: read_wad_u64(base + OFF_OL_BORROWED_AMOUNT_WADS),
        });
    }

    Some(SolendObligation {
        owner: read_pk(OFF_OB_OWNER),
        deposits,
        borrows,
    })
}

/// This bot's own Solend lending position -- tracks whether *this bot's*
/// obligation is initialized and its current contents. Mirrors
/// `dex::velocity::VelocityState`'s old `set_authority`/`drift_user()`
/// pattern (deposit/borrow/withdraw/repay instructions already take
/// `owner`/`obligation` as plain `AccountId` params, so they don't need
/// an authority-bearing `self` -- this struct exists only to know "is my
/// obligation initialized, and what does it currently hold"), scoped to
/// one account instead of a market list. Reserve pricing/instruction-
/// building keep coming from the separate, shared, read-only
/// `SolendState` (e.g. via `DexState::solend()`).
#[derive(Debug, Default)]
pub struct SolendPosition {
    o_authority_pk: Option<Pubkey>,
    o_obligation_id: Option<AccountId>,
    o_obligation: Option<SolendObligation>,
    subscriptions: Vec<Subscription>,
}

impl SolendPosition {
    /// Pure-derivation half of the old single-shot `set_authority`
    /// (removed -- only ever called from `testperpv1`'s
    /// `Wallet` message handler, alongside Phoenix/Kamino/marginfi's own
    /// three-to-four subscription calls). Returns the subscription
    /// request this authority needs (empty if already set), without
    /// making the host `subscribe` call itself -- paired with
    /// [`Self::apply_authority`] so all of that handler's requests can be
    /// batched into one `bulk_subscribe` round-trip instead of five
    /// separate ones. Real, live-observed incident: those five
    /// one-at-a-time calls accounted for ~26 seconds of stall in one run
    /// (traced via `CommitHook::start`'s own timing diagnostics).
    pub fn authority_subscribe_requests(&self, authority: Pubkey, id: u8) -> Vec<SubscriptionRequest> {
        if self.o_authority_pk == Some(authority) {
            return Vec::new();
        }
        let obligation_pk = obligation_address(&authority, id);
        vec![SubscriptionRequest { root: account_id_from_pubkey(&obligation_pk), filter_weight: 0, depth: 1 }]
    }

    /// Apply `authority` plus its already-resolved subscription (from
    /// [`Self::authority_subscribe_requests`]) -- the second half of the
    /// split described there. No-op if `subs` is empty (either already
    /// set, or nothing to apply). `id` must match the same value passed to
    /// `authority_subscribe_requests`.
    pub fn apply_authority(&mut self, authority: Pubkey, id: u8, subs: Vec<Subscription>) {
        let Some(sub) = subs.into_iter().next() else { return };
        let obligation_pk = obligation_address(&authority, id);
        let obligation_id = account_id_from_pubkey(&obligation_pk);
        self.subscriptions.push(sub);
        self.o_authority_pk = Some(authority);
        self.o_obligation_id = Some(obligation_id);
    }

    pub fn obligation_id(&self) -> Option<AccountId> {
        self.o_obligation_id
    }

    /// `true` only once a real update for the obligation account has been
    /// parsed -- means the account actually exists on-chain
    /// (`init_obligation` already succeeded), not just "we know its
    /// address and subscribed." Reliable because subscriptions are
    /// push-based: an account that doesn't exist yet simply never
    /// produces an `on_account` update. Same reasoning as
    /// `PhoenixState::trader_registered`.
    pub fn registered(&self) -> bool {
        self.o_obligation.is_some()
    }

    /// Last successfully parsed obligation, if any real update has
    /// arrived yet.
    pub fn obligation(&self) -> Option<&SolendObligation> {
        self.o_obligation.as_ref()
    }

    pub fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if Some(header.accountid) == self.o_obligation_id {
            if let Some(ob) = parse_obligation(body) {
                self.o_obligation = Some(ob);
            }
        }
    }
}

/// Live Solend state: subscribes to every reserve in
/// `solend_config::SOLEND_RESERVES` (the build-time address book, same
/// pattern as `MarginfiState::new` reading `marginfi_config::MARGINFI_BANKS`).
/// No oracle subscription is needed -- see the module doc comment.
///
/// Implements every `Updater` method for real, mirroring `MarginfiState`:
/// `batch_router`/`on_tx`/`flush_pool` are genuine no-ops (a lending
/// Reserve has no swap price to contribute to `TradeRouter`, matching the
/// reasoning in `credit.rs`'s module doc comment for why lending isn't
/// unified with the AMM price graph), not stubs left to be finished later.
pub struct SolendState {
    program_id: AccountId,
    m_reserve: HashMap<AccountId, SolendReserve, BuildHasherDefault<XxHash64>>,
}

impl std::fmt::Debug for SolendState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolendState")
            .field("reserve_count", &self.m_reserve.len())
            .finish()
    }
}

impl SolendState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&SOLEND_PROGRAM_ID);
        let reserves = crate::solend_config::SOLEND_RESERVES;
        let mut m_reserve =
            HashMap::with_capacity_and_hasher(reserves.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(reserves.len());

        for raw in reserves {
            let reserve_id = account_id_from_pubkey(&Pubkey::new_from_array(raw.pubkey));
            l_req.push(SubscriptionRequest {
                root: reserve_id,
                filter_weight: 0,
                depth: 1,
            });
            m_reserve.insert(reserve_id, SolendReserve::default());
        }

        (Self { program_id, m_reserve }, l_req)
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn reserve_count(&self) -> usize {
        self.m_reserve.len()
    }

    /// Direct reserve lookup by its own account id -- needed to refresh an
    /// obligation's *existing* deposit/borrow reserves before
    /// `refresh_obligation`, same reasoning as `KaminoState::reserve_by_id`.
    pub fn reserve_by_id(&self, reserve_id: AccountId) -> Option<&SolendReserve> {
        self.m_reserve.get(&reserve_id)
    }

    /// Find the *main-pool* reserve backing `mint` (e.g. real SOL's mint
    /// -> the SOL reserve). Solend runs dozens of separate markets (see
    /// [`SOLEND_MAIN_MARKET`]'s doc comment -- live-verified via Solend's
    /// own API: `SOLEND_RESERVES`' build-time snapshot has real reserves
    /// for the same mints on other, non-main markets too), so a plain
    /// mint match isn't enough -- must also check `lending_market`, same
    /// reasoning as `KaminoState::reserve_by_mint`. Linear scan is fine --
    /// `m_reserve` only ever holds a handful of entries (see
    /// `SOLEND_RESERVES`' build-time size), same reasoning as
    /// `DriftState::market_by_index`. `None` if `mint` isn't tracked on
    /// the main pool specifically, or is but no account update has
    /// arrived yet.
    pub fn reserve_by_mint(&self, mint: AccountId) -> Option<(AccountId, &SolendReserve)> {
        let main_market = account_id_from_pubkey(&SOLEND_MAIN_MARKET);
        self.m_reserve
            .iter()
            .find(|(_, r)| r.mint == mint && r.lending_market == main_market)
            .map(|(id, r)| (*id, r))
    }

    /// Build a priced [`crate::trader::credit::CreditReserve`] for
    /// `reserve_id`, using the latest parsed reserve state -- unlike
    /// `MarginfiState::credit_reserve`, there's no separate oracle update
    /// to wait for, so this is `None` only before the reserve's own first
    /// account update has arrived.
    pub fn credit_reserve(
        &self,
        reserve_id: AccountId,
    ) -> Option<crate::trader::credit::CreditReserve> {
        let reserve = self.m_reserve.get(&reserve_id)?;
        Some(crate::trader::credit::CreditReserve::from_solend(
            reserve_id, reserve,
        ))
    }
}

impl Updater for SolendState {
    fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if let Some(reserve) = self.m_reserve.get_mut(&header.accountid) {
            if let Some(parsed) = parse(body) {
                *reserve = parsed;
            }
        }
    }

    fn on_token(&mut self, _ta: &crate::catscope::witbot::shooter::Tokenaccountv1) -> bool {
        // Solend's available liquidity/price come directly from a
        // Reserve's own fields (already read in parse()), not from
        // watching a vault's SPL Token balance -- there is nothing for
        // this hook to track, same reasoning as MarginfiState::on_token.
        false
    }

    fn batch_router(&mut self, _router: &mut TradeRouter) {
        // A lending Reserve has no swap price to contribute to
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

    /// Real values read from a live mainnet USDT Reserve during offset
    /// verification: price landed right at the USDT peg ($0.9987), and
    /// utilization sits below `optimal_utilization_rate` (92%), so the APY
    /// should fall on the `min_borrow_rate`-to-`optimal_borrow_rate` leg.
    fn real_usdt_reserve() -> SolendReserve {
        SolendReserve {
            lending_market: 1,
            mint: 100,
            mint_decimals: 6,
            available_amount: 2_312_263_037_631,
            borrowed_amount: 2_681_215_225_571.3774,
            price_usd: 0.99869278,
            loan_to_value_pct: 0.70,
            liquidation_threshold_pct: 0.77,
            optimal_utilization_rate: 92.0,
            min_borrow_rate: 1.0,
            optimal_borrow_rate: 4.0,
            max_borrow_rate: 12.0,
            // This reserve's real max_utilization_rate/super_max_borrow_rate
            // weren't independently verified (only SOL's were, see
            // real_sol_reserve_emergency_tier() below) -- 100.0 keeps this
            // fixture's tests exercising only the two segments the original
            // (pre-emergency-tier) model covered, not a claim about USDT's
            // real third segment.
            max_utilization_rate: 100.0,
            super_max_borrow_rate: 0.0,
            ..Default::default()
        }
    }

    /// Real, live-verified SOL reserve values (this session's Solend
    /// verification pass): `optimal_utilization_rate=80,
    /// max_utilization_rate=90, max_borrow_rate=10%,
    /// super_max_borrow_rate=300%, protocol_take_rate=20%`.
    /// `min_borrow_rate`/`optimal_borrow_rate` aren't needed for the
    /// emergency-tier assertions below (only matters once utilization
    /// exceeds `max_utilization_rate`), so left at a placeholder.
    fn real_sol_reserve_emergency_tier() -> SolendReserve {
        SolendReserve {
            lending_market: 1,
            mint: 100,
            mint_decimals: 9,
            available_amount: 100,
            borrowed_amount: 0.0, // set per-test to hit the desired utilization
            optimal_utilization_rate: 80.0,
            min_borrow_rate: 0.0,
            optimal_borrow_rate: 5.0,
            max_borrow_rate: 10.0,
            max_utilization_rate: 90.0,
            super_max_borrow_rate: 300.0,
            protocol_take_rate: 20.0,
            ..Default::default()
        }
    }

    #[test]
    fn utilization_matches_real_reserve() {
        let r = real_usdt_reserve();
        // ~53.7% -- below the 92% optimal point.
        assert!((r.utilization() - 0.5369).abs() < 0.001);
    }

    #[test]
    fn borrow_apy_interpolates_below_optimal() {
        let r = real_usdt_reserve();
        let apy = r.current_borrow_apy();
        // Should land between min (1%) and optimal (4%) rates.
        assert!(apy > 0.01 && apy < 0.04);
    }

    #[test]
    fn borrow_apy_at_max_beyond_full_utilization() {
        let mut r = real_usdt_reserve();
        r.borrowed_amount = r.available_amount as f64 * 20.0; // force >92% utilization
        let apy = r.current_borrow_apy();
        assert!(apy > 0.04 && apy <= 0.12);
    }

    #[test]
    fn borrow_apy_uses_emergency_tier_beyond_max_utilization() {
        let mut r = real_sol_reserve_emergency_tier();
        r.borrowed_amount = r.available_amount as f64 * 19.0; // 95% utilization, past max=90%
        let apy = r.current_borrow_apy();
        // Real segment: max_borrow_rate(10%) -> super_max_borrow_rate(300%).
        assert!(apy > 0.10 && apy < 3.0);
    }

    #[test]
    fn borrow_apy_stays_on_second_segment_at_max_utilization_boundary() {
        let mut r = real_sol_reserve_emergency_tier();
        r.borrowed_amount = r.available_amount as f64 * 9.0; // exactly 90% utilization
        let apy = r.current_borrow_apy();
        // At the boundary, should land at (or just under) max_borrow_rate(10%),
        // not already into the emergency tier.
        assert!((apy - 0.10).abs() < 0.001);
    }

    #[test]
    fn supply_apy_matches_real_accrual_formula() {
        let mut r = real_sol_reserve_emergency_tier();
        r.borrowed_amount = r.available_amount as f64 * 4.0; // 80% utilization -> exactly optimal
        let borrow_apy = r.current_borrow_apy();
        let expected = borrow_apy * r.utilization() * (1.0 - 0.20);
        assert!((r.current_supply_apy() - expected).abs() < 1e-9);
        // Real, live-verified take rate (20%) means supply APY is strictly
        // less than borrow_apy * utilization.
        assert!(r.current_supply_apy() < borrow_apy * r.utilization());
    }

    // Pure, host-independent PDA/seeded-address checks only -- see the note
    // in `kamino.rs`'s test module for why the instruction-builder methods
    // themselves aren't unit-tested here.

    #[test]
    fn lending_market_authority_has_no_prefix_seed() {
        // Regression check: Solend derives this with a single seed
        // (`[lending_market]`), unlike Kamino's `["lma", lending_market]` --
        // getting this wrong the other way (adding a prefix) would produce
        // a different, wrong PDA.
        let lending_market = Pubkey::new_unique();
        let expected = Pubkey::find_program_address(&[lending_market.as_ref()], &SOLEND_PROGRAM_ID).0;
        assert_eq!(lending_market_authority(&lending_market), expected);
    }

    #[test]
    fn obligation_address_is_deterministic_and_owner_specific() {
        let owner_a = Pubkey::new_unique();
        let owner_b = Pubkey::new_unique();
        assert_eq!(obligation_address(&owner_a, 0), obligation_address(&owner_a, 0));
        assert_ne!(obligation_address(&owner_a, 0), obligation_address(&owner_b, 0));
    }

    #[test]
    fn obligation_address_is_independent_per_id() {
        let owner = Pubkey::new_unique();
        assert_ne!(obligation_address(&owner, 0), obligation_address(&owner, 1));
        assert_ne!(obligation_address(&owner, 1), obligation_address(&owner, 2));
    }

    /// The one test that actually protects real, possibly-currently-open
    /// mainnet Solend obligations (`testperpv1`, and the now-retired
    /// perpfundingv1 before it, both real callers of `id=0`): confirms
    /// `obligation_address(owner, 0)` is
    /// byte-identical to what the *original*, unparameterized
    /// `Pubkey::create_with_seed(owner, "solend-obligation",
    /// &SOLEND_PROGRAM_ID)` call would have produced -- not just "id 0 and
    /// id 1 differ" (already covered above), but "id 0 didn't silently
    /// change" when this function was parameterized.
    #[test]
    fn obligation_address_id_zero_matches_original_unparameterized_seed() {
        let owner = Pubkey::new_unique();
        let expected = Pubkey::create_with_seed(&owner, "solend-obligation", &SOLEND_PROGRAM_ID).unwrap();
        assert_eq!(obligation_address(&owner, 0), expected);
    }

    // `parse_obligation` itself isn't unit-tested here -- it calls
    // `account_id_from_pubkey` internally (a WIT host import that aborts
    // outside the real WASM guest runtime), same established boundary as
    // every other `parse`-style function in this codebase (see
    // `dex::drift`'s `DriftUser`/`user_with_position` for the same
    // pattern). `SolendObligation::deposit_for`/`borrow_for` have no such
    // dependency and are fully testable directly.
    fn obligation_with_positions() -> SolendObligation {
        SolendObligation {
            owner: 0,
            deposits: vec![ObligationCollateral { deposit_reserve: 7, deposited_amount: 5_000_000_000 }],
            borrows: vec![ObligationLiquidity { borrow_reserve: 9, borrowed_amount: 1_200_000_000 }],
        }
    }

    #[test]
    fn deposit_for_finds_position_by_reserve() {
        let ob = obligation_with_positions();
        let d = ob.deposit_for(7).expect("expected a deposit in reserve 7");
        assert_eq!(d.deposited_amount, 5_000_000_000);
        assert!(ob.deposit_for(8).is_none());
    }

    #[test]
    fn borrow_for_finds_position_by_reserve() {
        let ob = obligation_with_positions();
        let b = ob.borrow_for(9).expect("expected a borrow against reserve 9");
        assert_eq!(b.borrowed_amount, 1_200_000_000);
        assert!(ob.borrow_for(10).is_none());
    }
}
