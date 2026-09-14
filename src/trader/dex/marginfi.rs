//! marginfi-v2 Bank account parser (credit-graph phase 2a — see
//! `cryptic-percolating-bunny.md`).
//!
//! `parse()`/`MarginfiBank` started out deliberately standalone (not
//! wired into `Updater`/`DexState`), feeding only
//! [`crate::trader::credit::CreditReserve`] directly. [`MarginfiState`]
//! (below) now wires bank + oracle subscriptions into `DexState` so that
//! data stays live -- see its own doc comment for the shape.
//!
//! # Account layout — marginfi-v2 Bank (Anchor, 8-byte discriminator)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        8   Anchor discriminator
//!   8       32   mint (Pubkey)
//!  40        1   mint_decimals (u8)
//!  41       32   group (Pubkey)
//!  73        7   _pad0
//!  80       16   asset_share_value (WrappedI80F48)
//!  96       16   liability_share_value (WrappedI80F48)
//! 256       16   total_liability_shares (WrappedI80F48)
//! 272       16   total_asset_shares (WrappedI80F48)
//! 288        8   last_update (i64)
//! 296       16   config.asset_weight_init (WrappedI80F48)  ← LTV equivalent
//! 312       16   config.asset_weight_maint (WrappedI80F48)
//! 328       16   config.liability_weight_init (WrappedI80F48)
//! 344       16   config.liability_weight_maint (WrappedI80F48)
//! 360        8   config.deposit_limit (u64)
//! 368      240   config.interest_rate_config (InterestRateConfig)
//!   368     16     placeholder0 (deprecated, unused)
//!   384     16     placeholder1 (deprecated, unused)
//!   400     16     placeholder2 (deprecated, unused)
//!   416     16     insurance_fee_fixed_apr (WrappedI80F48)
//!   432     16     insurance_ir_fee (WrappedI80F48)
//!   448     16     protocol_fixed_fee_apr (WrappedI80F48)
//!   464     16     protocol_ir_fee (WrappedI80F48)
//!   480     16     protocol_origination_fee (WrappedI80F48)
//!   496      4     zero_util_rate (u32, out of u32::MAX = 1000%)
//!   500      4     hundred_util_rate (u32, out of u32::MAX = 1000%)
//!   504     40     points: [RatePoint; 5], each {util: u32, rate: u32}
//!                  (util out of u32::MAX = 100%, rate out of u32::MAX = 1000%;
//!                  unused points are util=0/rate=0 and must be skipped)
//!   544      1     curve_type (u8; 1 = INTEREST_CURVE_SEVEN_POINT, the only
//!                  supported value -- 0 is deprecated/legacy)
//!   545     63     padding
//! 608        1   config.operational_state (u8 enum ordinal)
//! 609        1   config.oracle_setup (u8 enum ordinal)      ← NOT always Pyth
//! 610       32   config.oracle_keys[0] (Pubkey)
//! ```
//!
//! Offsets were confirmed against the public `0dotxyz/marginfi-v2`
//! `type-crate` source (`types/bank.rs`, `types/bank_config.rs`,
//! `types/interest_rate.rs` -- the repo moved from `mrgnlabs/marginfi-v2`)
//! *and* cross-checked against real mainnet Bank accounts:
//! `asset_share_value`/`liability_share_value` read back ~1.0 (an
//! interest-accrual multiplier that starts at 1.0 and drifts slightly
//! above it over time); `total_asset_shares`/`total_liability_shares` read
//! back as widely varying, plausible token quantities (not the near-
//! identical garbage values an earlier, wrong offset guess produced); and
//! `asset_weight_init` read back as a clean 0.0-1.0 fraction, with
//! `asset_weight_maint` consistently higher (looser) and
//! `liability_weight_init`/`liability_weight_maint` consistently ≥ 1.0 and
//! in the expected init ≥ maint order -- the same LTV-vs-threshold
//! relationship found empirically for Kamino. The interest-rate-curve
//! offsets (368-544) were independently confirmed by decoding SOL's real
//! main-group bank (`CCKtUs6Cgwo4aaQUmBPmyoApH2gUDErxNZCAntD6LYGh`): a
//! genuine monotonic jump-rate curve (0% at 0% utilization, rising through
//! 5%/7%/10% at 90%/98%/99%, up to 20% at 100%), `curve_type == 1`, and
//! plausible near-zero fee values -- not garbage.
//!
//! `WrappedI80F48` is a 128-bit fixed-point number: 80 integer bits + 48
//! fractional bits, i.e. `value = raw_i128 / 2^48`.
//!
//! **Not available from this account alone**: a USD price. Unlike Kamino's
//! Reserve (which stores `market_price_sf` directly), marginfi Banks only
//! reference an oracle account (`oracle_setup`/`oracle_key`, confirmed
//! against 10 real banks: setup values 1/3/4/6/8/16, NOT always Pyth --
//! see `dex::pyth` for the two formats parsed so far: legacy Pyth
//! (`Price`, magic-number-gated) and Pyth Push Oracle (`PriceUpdateV2`,
//! Anchor-discriminator-gated); real SOL banks in the tracked set use
//! push-oracle pricing (`oracle_setup == 4`), not legacy). [`MarginfiState`]
//! below subscribes to every bank's oracle account regardless of
//! `oracle_setup` and tries both parsers against whatever comes back --
//! each format's own self-describing discriminator decides which map (if
//! either) gets populated, so `CreditReserve::from_marginfi` can be priced
//! once an update has been seen -- still `0.0` for any other oracle type
//! (Switchboard, etc.) this bot doesn't parse.
//!
//! [`MarginfiState`] mirrors Kamino's `KaminoState`/`Updater` shape, but
//! implements every method for real -- Kamino's own `on_account` is an
//! empty stub and its `batch_router`/`on_tx` are `todo!()` (which would
//! panic, since `DexState::batch_router` calls every registered dex
//! unconditionally). `batch_router`/`on_tx`/`flush_pool` here are genuine
//! no-ops (a lending Bank has no swap price to contribute to
//! `TradeRouter`, matching the reasoning in `credit.rs`'s module doc
//! comment for why lending isn't unified with the AMM price graph) --
//! not stubs left to be finished later.

use crate::{
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionRequest},
    trader::{dex::pyth, dex::update::Updater, pricegraph::TradeRouter, types::TraderError},
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::{system_program, sysvar::instructions};
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

pub const MARGINFI_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA");

/// marginfi's real, confirmed main pool group -- marginfi currently only
/// maintains this one group for the main pool on its own app front page.
/// Confirmed against two independent real sources: DefiLlama's own
/// production TVL adapter (`projects/marginfi/index.js`, which defines
/// this exact same address under the identical constant name
/// `MARGINFI_MAIN_GROUP`), and live prefetch data -- it's the dominant
/// group by far (203 of 436 tracked banks; next-largest group has 17).
/// Same verification bar as `KAMINO_MAIN_MARKET`/`SOLEND_MAIN_MARKET`.
pub const MARGINFI_MAIN_GROUP: Pubkey =
    Pubkey::from_str_const("4qp6Fx6tnZkY5Wropq9wUYgtFxXKwE6viZxFHg3rdAG8");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// Anchor discriminators, sha256("global:<name>")[..8], cross-checked
/// against the live on-chain Anchor IDL fetched from
/// `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` (`anchor idl fetch`).
const DISC_INITIALIZE_ACCOUNT_PDA: [u8; 8] = [87, 177, 91, 80, 218, 119, 245, 31];
const DISC_LENDING_ACCOUNT_DEPOSIT: [u8; 8] = [171, 94, 235, 103, 82, 64, 212, 140];
const DISC_LENDING_ACCOUNT_BORROW: [u8; 8] = [4, 126, 116, 53, 48, 5, 212, 31];
const DISC_LENDING_ACCOUNT_REPAY: [u8; 8] = [79, 209, 172, 177, 222, 51, 173, 151];
const DISC_LENDING_ACCOUNT_WITHDRAW: [u8; 8] = [36, 72, 74, 19, 210, 210, 192, 192];

pub const MARGINFI_INITIALIZE_ACCOUNT_CU: u32 = 40_000;
/// **Live-corrected, not a guess**: an original `60_000` budget here was
/// too low -- a real mainnet `testperpv1` USDC deposit
/// (`4bf8MkKKR4FCwxGFBUyG2tCtYH6L2V3KhpurhSDNk7jpxXDJffu3qn6NHny9hyrpjLFHYMRLww7UhSWfFb7B4KPs`)
/// failed with `ProgramFailedToComplete` / "exceeded CUs meter at BPF
/// instruction" after consuming exactly the 59_850 CU it had left (60_000
/// tx-wide minus the `SetComputeUnitLimit` instruction's own overhead) --
/// `LendingAccountDeposit` genuinely needs more than that (real work:
/// bank state update, an oracle price fetch for the deposit-limit check,
/// a token CPI). Bumped generously rather than to a bare minimum -- CU
/// budget headroom is cheap, a repeat of this failure is not (a real,
/// wasted mainnet transaction).
pub const MARGINFI_DEPOSIT_CU: u32 = 150_000;
pub const MARGINFI_REPAY_CU: u32 = 150_000;
/// Borrow/withdraw scale with the number of other active positions on the
/// account (each contributes one more `[bank, oracle]` pair to
/// `remaining_accounts` for the health check). Both also run a full
/// account health check (unlike deposit/repay), so the base budget is
/// higher than [`MARGINFI_DEPOSIT_CU`]'s -- bumped alongside it per the
/// same live incident (that fix was for deposit specifically, but the
/// same "60_000 is too tight for a real marginfi instruction" lesson
/// applies at least as much here, since these do more work, not less).
pub const MARGINFI_BORROW_BASE_CU: u32 = 250_000;
pub const MARGINFI_BORROW_PER_BANK_CU: u32 = 40_000;
pub const MARGINFI_WITHDRAW_BASE_CU: u32 = 250_000;
pub const MARGINFI_WITHDRAW_PER_BANK_CU: u32 = 40_000;

const OFF_MINT: usize = 8;
const OFF_MINT_DECIMALS: usize = 40;
const OFF_GROUP: usize = 41;
const OFF_ASSET_SHARE_VALUE: usize = 80;
const OFF_LIABILITY_SHARE_VALUE: usize = 96;
const OFF_TOTAL_LIABILITY_SHARES: usize = 256;
const OFF_TOTAL_ASSET_SHARES: usize = 272;
const OFF_ASSET_WEIGHT_INIT: usize = 296;
const OFF_LIABILITY_WEIGHT_INIT: usize = 328;
const OFF_LIABILITY_WEIGHT_MAINT: usize = 344;
const OFF_INSURANCE_FEE_FIXED_APR: usize = 416;
const OFF_INSURANCE_IR_FEE: usize = 432;
const OFF_PROTOCOL_FIXED_FEE_APR: usize = 448;
const OFF_PROTOCOL_IR_FEE: usize = 464;
const OFF_ZERO_UTIL_RATE: usize = 496;
const OFF_HUNDRED_UTIL_RATE: usize = 500;
const OFF_CURVE_POINTS: usize = 504;
const CURVE_POINTS_COUNT: usize = 5;
const OFF_ORACLE_SETUP: usize = 609;
const OFF_ORACLE_KEY: usize = 610;
/// `config.oracle_max_age` (u16, seconds) -- "Time window in seconds for
/// the oracle price feed to be considered live" (real source comment,
/// `0dotxyz/marginfi-v2`'s `type-crate/src/types/bank_config.rs`). This
/// is the exact per-bank threshold marginfi's own risk engine compares
/// a Switchboard/Pyth-Push oracle's staleness against
/// (`SwitchboardStalePrice`/similar, see `pyth::OraclePrice::
/// last_update_timestamp`'s doc comment for the live-confirmed incident
/// this exists to prevent). Offset derived from `BankConfig`'s real,
/// asserted total size (544 bytes) and field order in the same source
/// file, cross-checked two independent ways against this file's own
/// already-verified `OFF_ORACLE_SETUP`/`OFF_ORACLE_KEY` anchor points:
/// (1) forward from `BankConfig`'s start (`609 - 313 = 296`, where 313 is
/// `oracle_setup`'s real relative offset) through every field up to
/// `oracle_max_age`, and (2) backward from `BankConfig`'s real 544-byte
/// end through its known trailing field order. Both agree: absolute
/// offset `296 + 504 = 800`.
const OFF_ORACLE_MAX_AGE: usize = 800;

const MIN_BANK_LEN: usize = OFF_ORACLE_MAX_AGE + 2;

/// Scale for marginfi's `WrappedI80F48`: 80 integer bits, 48 fractional.
const I80F48_SCALE: f64 = (1u128 << 48) as f64;

/// Parsed subset of a marginfi-v2 Bank account -- only the fields needed
/// for [`crate::trader::credit::CreditReserve::from_marginfi`] and this
/// bot's own basis-trade protocol selection (`current_borrow_apy`/
/// `current_supply_apy`).
#[derive(Debug, Default, Clone)]
pub struct MarginfiBank {
    pub mint: AccountId,
    pub mint_decimals: u8,
    pub group: AccountId,
    pub asset_share_value: f64,
    pub liability_share_value: f64,
    pub total_liability_shares: f64,
    pub total_asset_shares: f64,
    /// Max loan-to-value ratio (0.0-1.0) when this bank's asset is
    /// deposited as collateral -- marginfi's `config.asset_weight_init`.
    pub asset_weight_init: f64,
    /// Liability-side premium factors (>= 1.0, `init` >= `maint`) --
    /// not currently used by this bot's own logic, parsed for parity with
    /// `SolendReserve`/`KaminoReserve`'s shape and in case a future risk
    /// check needs them.
    pub liability_weight_init: f64,
    pub liability_weight_maint: f64,
    /// Real `InterestRateConfig` fields (`interest_rate.rs`, live-verified
    /// against SOL's main-group bank -- see the module doc comment): a
    /// 7-point piecewise-linear curve from `zero_util_rate` through up to
    /// 5 interior `(utilization, rate)` points to `hundred_util_rate`.
    /// Rates/utilizations are raw `u32` fractions of `u32::MAX` (rate:
    /// `u32::MAX` = 1000%, utilization: `u32::MAX` = 100%) -- kept raw
    /// here, converted in `current_borrow_apy`.
    pub zero_util_rate: u32,
    pub hundred_util_rate: u32,
    /// Interior curve points in ascending utilization order; unused slots
    /// are `(0, 0)` and must be skipped (`points where util = 0 are
    /// unused`, per real source doc comment).
    pub curve_points: [(u32, u32); CURVE_POINTS_COUNT],
    /// Fixed APR fees borrowers pay on top of the base curve rate (added,
    /// not netted against supply).
    pub insurance_fee_fixed_apr: f64,
    pub protocol_fixed_fee_apr: f64,
    /// Fractional cuts of the base curve rate taken before it reaches
    /// depositors (subtracted from the supply side).
    pub insurance_ir_fee: f64,
    pub protocol_ir_fee: f64,
    /// Raw `OracleSetup` enum ordinal -- NOT always Pyth, see the module
    /// doc comment before assuming which oracle type this is.
    pub oracle_setup: u8,
    /// The primary oracle account to read for this bank's price
    /// (`config.oracle_keys[0]`). Blank for "Fixed" setups, which have no
    /// real oracle account at all.
    pub oracle_key: AccountId,
    /// `config.oracle_max_age`, in seconds -- see [`OFF_ORACLE_MAX_AGE`]'s
    /// doc comment.
    pub oracle_max_age: u16,
}

impl MarginfiBank {
    /// Liquidity actually available to borrow, in whole tokens (raw
    /// `total_asset_shares - total_liability_shares`, converted via the
    /// share-value multipliers and `mint_decimals`). Not USD -- see the
    /// module doc comment for why a price isn't available here.
    pub fn available_liquidity_tokens(&self) -> f64 {
        let assets_raw = self.total_asset_shares * self.asset_share_value;
        let liabilities_raw = self.total_liability_shares * self.liability_share_value;
        (assets_raw - liabilities_raw).max(0.0) / 10f64.powi(self.mint_decimals as i32)
    }

    /// Fraction (0.0-1.0) of this bank's assets currently borrowed out --
    /// `0.0` if there are no assets at all (avoids a division by zero,
    /// matching Solend/Kamino's same-shape guard). Raw share amounts, not
    /// human units -- share values cancel out of the ratio so converting
    /// first isn't necessary.
    fn utilization(&self) -> f64 {
        let assets_raw = self.total_asset_shares * self.asset_share_value;
        if assets_raw <= 0.0 {
            return 0.0;
        }
        let liabilities_raw = self.total_liability_shares * self.liability_share_value;
        (liabilities_raw / assets_raw).clamp(0.0, 1.0)
    }

    /// Interpolate marginfi's real 7-point curve (`zero_util_rate` ->
    /// up to 5 interior `curve_points` -> `hundred_util_rate`) at the
    /// current utilization, converting the raw `u32/u32::MAX` fractions to
    /// real percentages (rate: out of 1000%, i.e. `/u32::MAX*10.0`;
    /// utilization: out of 100%, i.e. `/u32::MAX`). Unused interior points
    /// (`util == 0`) are skipped -- see the module doc comment's live
    /// verification (SOL's real curve: 0% at 0%, rising through
    /// 5%/7%/10% at 90%/98%/99%, to 20% at 100%).
    fn interpolate_curve_pct(&self, utilization: f64) -> f64 {
        const MAX_U32: f64 = u32::MAX as f64;
        let to_util_frac = |raw: u32| raw as f64 / MAX_U32;
        let to_rate_pct = |raw: u32| raw as f64 / MAX_U32 * 1000.0;

        let mut prev_util = 0.0;
        let mut prev_rate = to_rate_pct(self.zero_util_rate);
        for &(util_raw, rate_raw) in self.curve_points.iter() {
            if util_raw == 0 {
                continue;
            }
            let util = to_util_frac(util_raw);
            let rate = to_rate_pct(rate_raw);
            if utilization <= util {
                let span = (util - prev_util).max(f64::EPSILON);
                let t = (utilization - prev_util) / span;
                return prev_rate + t * (rate - prev_rate);
            }
            prev_util = util;
            prev_rate = rate;
        }
        let hundred_rate = to_rate_pct(self.hundred_util_rate);
        let span = (1.0 - prev_util).max(f64::EPSILON);
        let t = ((utilization - prev_util) / span).clamp(0.0, 1.0);
        prev_rate + t * (hundred_rate - prev_rate)
    }

    /// Real borrow APY (percent units, matching `SolendReserve`/
    /// `KaminoReserve::current_borrow_apy`'s convention of returning a
    /// 0.0-1.0-ish fraction -- see `best_borrow_apy`'s `* 100.0` call
    /// sites): the interpolated curve rate plus the fixed fees borrowers
    /// pay on top (`protocol_fixed_fee_apr`/`insurance_fee_fixed_apr` are
    /// already fractions, e.g. `0.0001` = 0.01%, not percent -- multiply
    /// by 100 to match the curve rate's percent units before summing).
    pub fn current_borrow_apy(&self) -> f64 {
        let curve_pct = self.interpolate_curve_pct(self.utilization());
        (curve_pct + (self.protocol_fixed_fee_apr + self.insurance_fee_fixed_apr) * 100.0) / 100.0
    }

    /// Real supply APY: the base curve rate (before fixed fees, which
    /// borrowers pay on top but depositors never see), scaled by
    /// utilization (only the borrowed fraction of assets earns interest),
    /// net of the protocol/insurance `_ir_fee` cuts -- same formula shape
    /// Solend/Kamino already use (`borrow_apy * utilization * (1 -
    /// take_rate)`), with marginfi's two separate fee cuts summed into one
    /// take rate.
    pub fn current_supply_apy(&self) -> f64 {
        let utilization = self.utilization();
        let curve_pct = self.interpolate_curve_pct(utilization) / 100.0;
        let take_rate = (self.protocol_ir_fee + self.insurance_ir_fee).clamp(0.0, 1.0);
        curve_pct * utilization * (1.0 - take_rate)
    }
}

/// Parse a marginfi-v2 Bank account from raw body bytes (including the
/// 8-byte Anchor discriminator).
pub fn parse(body: &[u8]) -> Option<MarginfiBank> {
    if body.len() < MIN_BANK_LEN {
        return None;
    }

    let read_i80f48 = |off: usize| -> f64 {
        let raw = i128::from_le_bytes(body[off..off + 16].try_into().unwrap());
        raw as f64 / I80F48_SCALE
    };
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };
    let read_u32 = |off: usize| -> u32 { u32::from_le_bytes(body[off..off + 4].try_into().unwrap()) };

    let mut curve_points = [(0u32, 0u32); CURVE_POINTS_COUNT];
    for (i, point) in curve_points.iter_mut().enumerate() {
        let off = OFF_CURVE_POINTS + i * 8;
        *point = (read_u32(off), read_u32(off + 4));
    }

    Some(MarginfiBank {
        mint: read_pk(OFF_MINT),
        mint_decimals: body[OFF_MINT_DECIMALS],
        group: read_pk(OFF_GROUP),
        asset_share_value: read_i80f48(OFF_ASSET_SHARE_VALUE),
        liability_share_value: read_i80f48(OFF_LIABILITY_SHARE_VALUE),
        total_liability_shares: read_i80f48(OFF_TOTAL_LIABILITY_SHARES),
        total_asset_shares: read_i80f48(OFF_TOTAL_ASSET_SHARES),
        asset_weight_init: read_i80f48(OFF_ASSET_WEIGHT_INIT),
        liability_weight_init: read_i80f48(OFF_LIABILITY_WEIGHT_INIT),
        liability_weight_maint: read_i80f48(OFF_LIABILITY_WEIGHT_MAINT),
        zero_util_rate: read_u32(OFF_ZERO_UTIL_RATE),
        hundred_util_rate: read_u32(OFF_HUNDRED_UTIL_RATE),
        curve_points,
        insurance_fee_fixed_apr: read_i80f48(OFF_INSURANCE_FEE_FIXED_APR),
        protocol_fixed_fee_apr: read_i80f48(OFF_PROTOCOL_FIXED_FEE_APR),
        insurance_ir_fee: read_i80f48(OFF_INSURANCE_IR_FEE),
        protocol_ir_fee: read_i80f48(OFF_PROTOCOL_IR_FEE),
        oracle_setup: body[OFF_ORACLE_SETUP],
        oracle_key: read_pk(OFF_ORACLE_KEY),
        oracle_max_age: u16::from_le_bytes(body[OFF_ORACLE_MAX_AGE..OFF_ORACLE_MAX_AGE + 2].try_into().unwrap()),
    })
}

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

/// `["marginfi_account", group, authority, account_index_le, third_party_id_le]`
/// -- confirmed against `marginfi-v2`'s `MARGINFI_ACCOUNT_SEED` constant and
/// the `MarginfiAccountInitializePda` accounts struct. This bot always uses
/// `account_index = 0` (its one and only sub-account per group).
fn marginfi_account_pda(group: &Pubkey, authority: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"marginfi_account",
            group.as_ref(),
            authority.as_ref(),
            &0u16.to_le_bytes(), // account_index
            &0u16.to_le_bytes(), // third_party_id.unwrap_or(0)
        ],
        &MARGINFI_PROGRAM_ID,
    )
    .0
}

/// `["liquidity_vault_auth", bank]` -- the PDA that signs token transfers
/// out of a bank's liquidity vault (needed for borrow/withdraw).
fn bank_liquidity_vault_authority(bank: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"liquidity_vault_auth", bank.as_ref()], &MARGINFI_PROGRAM_ID).0
}

/// `["liquidity_vault", bank]` -- confirmed against `marginfi-v2`'s
/// `LIQUIDITY_VAULT_SEED` constant (`type-crate/src/constants.rs`). Every
/// bank's `has_one = liquidity_vault` constraint just checks the account
/// list against this same value stored on the Bank account, so deriving it
/// here (rather than parsing it out of the account) is equivalent and
/// avoids needing another live-parsed field.
fn bank_liquidity_vault(bank: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"liquidity_vault", bank.as_ref()], &MARGINFI_PROGRAM_ID).0
}

/// Append a `marginfi_account_initialize_pda` instruction to `wallet`,
/// creating `authority`'s (one and only) MarginfiAccount for `group`. Uses
/// the newer PDA-based initializer rather than marginfi's original
/// plain-keypair one -- fits this bot's single-signing-wallet model with no
/// extra keypair to generate/track, and the address is deterministic
/// ([`marginfi_account_pda`]), so nothing needs to be remembered separately
/// from `(group, authority)`. Passes `third_party_id = None`, which also
/// skips the CPI-authorization check `Some(id)` would trigger.
pub fn initialize_account_pda(
    group: AccountId,
    authority: AccountId,
    wallet: &mut Wallet,
) -> Result<AccountId, TraderError> {
    let group_pk = resolve(group)?;
    let authority_pk = resolve(authority)?;
    let account_pk = marginfi_account_pda(&group_pk, &authority_pk);

    let mut data = Vec::with_capacity(11);
    data.extend_from_slice(&DISC_INITIALIZE_ACCOUNT_PDA);
    data.extend_from_slice(&0u16.to_le_bytes()); // account_index
    data.push(0u8); // third_party_id: None

    wallet.require_signer(authority);
    wallet.append_ix(
        Instruction {
            program_id: MARGINFI_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new_readonly(group_pk, false),
                AccountMeta::new(account_pk, false),
                AccountMeta::new_readonly(authority_pk, true),
                AccountMeta::new(authority_pk, true), // fee_payer (same signer)
                AccountMeta::new_readonly(instructions::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        },
        MARGINFI_INITIALIZE_ACCOUNT_CU,
    );
    Ok(account_id_from_pubkey(&account_pk))
}

/// Real `OracleSetup` enum ordinals (`0dotxyz/marginfi-v2`'s
/// `type-crate/src/types/bank.rs`, confirmed via source -- the repo moved
/// from `mrgnlabs/marginfi-v2`): `PythPushOracle` and `SwitchboardPull`
/// are the two "plain" oracle types a general-purpose bank uses (every
/// other variant -- `KaminoPythPush`, `DriftPythPull`, `StakedWithPythPush`,
/// etc. -- is a protocol-partner collateral-integration bank, not a bank
/// this bot trades against; see `reserve_by_mint`). Both need exactly one
/// remaining account for the health check, and it's `oracle_keys[0]`
/// directly (confirmed via `price.rs`'s real `load_oracle_context_with_max_age`)
/// -- the same field already parsed here as `oracle_key`, no extra
/// derivation needed.
const ORACLE_SETUP_PYTH_PUSH: u8 = 3;
const ORACLE_SETUP_SWITCHBOARD_PULL: u8 = 4;

/// Live marginfi-v2 state: subscribes to every bank in
/// `marginfi_config::MARGINFI_BANKS` (the build-time address book, same
/// pattern as `KaminoState::new` reading `kamino_config::KAMINO_RESERVES`)
/// and, for banks using Pyth legacy pricing, their oracle account too.
///
/// Implements every `Updater` method for real -- see the module doc
/// comment for why `batch_router`/`on_tx`/`flush_pool` are genuine no-ops
/// rather than the `todo!()`/empty-stub shape `KaminoState` left them in.
pub struct MarginfiState {
    program_id: AccountId,
    m_bank: HashMap<AccountId, MarginfiBank, BuildHasherDefault<XxHash64>>,
    /// oracle account -> the bank it prices. Populated only for banks this
    /// bot actually trades (`group == MARGINFI_MAIN_GROUP`, plain
    /// `oracle_setup` 3/4, and a mint in `SYMBOL_MINT_MAP` or USDC -- see
    /// `MarginfiState::new()`) with a non-blank oracle key. Every
    /// subscribed oracle account is still tried against all three parsers
    /// in `on_account` (rather than trusting `oracle_setup` alone) since a
    /// format this bot doesn't parse just never produces a hit in any of
    /// the three maps below, safely -- but the subscription list itself is
    /// no longer "every bank in the protocol regardless of relevance" (was
    /// ~870 requests across all 436 real marginfi banks; live-confirmed
    /// this session to plausibly starve a host/validator-side subscription
    /// budget, leaving a bank this bot actually needed un-updated).
    m_oracle_to_bank: HashMap<AccountId, AccountId, BuildHasherDefault<XxHash64>>,
    /// bank -> last parsed legacy Pyth `Price` account price.
    m_price: HashMap<AccountId, pyth::PythPrice, BuildHasherDefault<XxHash64>>,
    /// bank -> last parsed Pyth Push Oracle (`PriceUpdateV2`) price.
    m_push_price: HashMap<AccountId, pyth::PythPushPrice, BuildHasherDefault<XxHash64>>,
    /// bank -> last parsed Switchboard On-Demand pull-feed price. Needed
    /// for banks like SOL's plain bank (`oracle_setup == 4`), whose oracle
    /// isn't a Pyth account at all -- see `pyth::parse_switchboard_pull`'s
    /// doc comment for the live-verified account layout.
    m_switchboard_price: HashMap<AccountId, pyth::SwitchboardPullPrice, BuildHasherDefault<XxHash64>>,
}

impl std::fmt::Debug for MarginfiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarginfiState")
            .field("bank_count", &self.m_bank.len())
            .field("priced_count", &self.m_price.len())
            .field("push_priced_count", &self.m_push_price.len())
            .finish()
    }
}

impl MarginfiState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&MARGINFI_PROGRAM_ID);
        let banks = crate::marginfi_config::MARGINFI_BANKS;
        let main_group = MARGINFI_MAIN_GROUP.to_bytes();
        // The real basis-trade symbol set (`SYMBOL_MINT_MAP`) plus USDC
        // (always needed as collateral/quote, but not itself a traded
        // symbol so it's not in that map) -- every mint this bot could
        // ever call `reserve_by_mint` with. Restricting subscriptions to
        // just these (instead of all 436 real marginfi banks) matters:
        // subscribing to every bank + oracle regardless of relevance was
        // sending ~870 subscription requests for a protocol this bot only
        // ever trades ~7 mints on, live-confirmed this session to
        // correlate with a specific traded oracle's updates never
        // arriving -- plausibly a host/validator-side subscription
        // capacity limit being exhausted by the other ~860 requests this
        // bot never needed (same class of issue already found and fixed
        // once this session for obligation-account subscriptions,
        // elsewhere in the stack).
        let usdc_mint = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").to_bytes();
        let is_traded_mint = |mint: &[u8; 32]| {
            *mint == usdc_mint
                || crate::symbol_mint_config::SYMBOL_MINT_MAP
                    .iter()
                    .any(|e| &e.mint == mint)
        };
        let traded_banks: Vec<_> = banks
            .iter()
            .filter(|raw| {
                raw.group == main_group
                    && (raw.oracle_setup == ORACLE_SETUP_PYTH_PUSH
                        || raw.oracle_setup == ORACLE_SETUP_SWITCHBOARD_PULL)
                    && is_traded_mint(&raw.mint)
            })
            .collect();
        let mut m_bank =
            HashMap::with_capacity_and_hasher(traded_banks.len(), BuildHasherDefault::default());
        let mut m_oracle_to_bank =
            HashMap::with_capacity_and_hasher(traded_banks.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(traded_banks.len() * 2);

        for raw in traded_banks {
            let bank_id = account_id_from_pubkey(&Pubkey::new_from_array(raw.pubkey));
            l_req.push(SubscriptionRequest {
                root: bank_id,
                filter_weight: 0,
                depth: 1,
            });
            m_bank.insert(bank_id, MarginfiBank::default());

            if raw.oracle_key != [0u8; 32] {
                let oracle_id = account_id_from_pubkey(&Pubkey::new_from_array(raw.oracle_key));
                l_req.push(SubscriptionRequest {
                    root: oracle_id,
                    filter_weight: 0,
                    depth: 1,
                });
                m_oracle_to_bank.insert(oracle_id, bank_id);
            }
        }

        let m_price = HashMap::with_capacity_and_hasher(
            m_oracle_to_bank.len(),
            BuildHasherDefault::default(),
        );
        let m_push_price = HashMap::with_capacity_and_hasher(
            m_oracle_to_bank.len(),
            BuildHasherDefault::default(),
        );
        let m_switchboard_price = HashMap::with_capacity_and_hasher(
            m_oracle_to_bank.len(),
            BuildHasherDefault::default(),
        );

        let state = Self {
            program_id,
            m_bank,
            m_oracle_to_bank,
            m_price,
            m_push_price,
            m_switchboard_price,
        };
        (state, l_req)
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn bank_count(&self) -> usize {
        self.m_bank.len()
    }

    /// Find the *plain, general-purpose* bank backing `mint` on marginfi's
    /// real main group -- mirrors `SolendState`/`KaminoState::reserve_by_mint`'s
    /// shape, but marginfi's disambiguation problem is different in kind:
    /// the main group genuinely has multiple banks for the same mint (SOL
    /// has 6, USDC has 7 in live data), but they're not interchangeable
    /// duplicates -- most are protocol-partner collateral-integration
    /// banks (`KaminoPythPush`, `DriftPythPull`, etc., see
    /// `ORACLE_SETUP_PYTH_PUSH`'s doc comment). Filters to
    /// `group == MARGINFI_MAIN_GROUP` and `oracle_setup` in
    /// `{PythPushOracle, SwitchboardPull}` (the two plain types), then
    /// tie-breaks by highest TVL (`total_asset_shares * asset_share_value`)
    /// if more than one still matches -- live-verified this correctly
    /// isolates a single bank for SOL (only one of its 6 banks uses
    /// `oracle_setup == 4`).
    pub fn reserve_by_mint(&self, mint: AccountId) -> Option<(AccountId, &MarginfiBank)> {
        let main_group = account_id_from_pubkey(&MARGINFI_MAIN_GROUP);
        self.m_bank
            .iter()
            .filter(|(_, b)| {
                b.mint == mint
                    && b.group == main_group
                    && (b.oracle_setup == ORACLE_SETUP_PYTH_PUSH
                        || b.oracle_setup == ORACLE_SETUP_SWITCHBOARD_PULL)
            })
            .max_by(|(_, a), (_, b)| {
                let tvl_a = a.total_asset_shares * a.asset_share_value;
                let tvl_b = b.total_asset_shares * b.asset_share_value;
                tvl_a.total_cmp(&tvl_b)
            })
            .map(|(id, b)| (*id, b))
    }

    /// How many bank oracle accounts are currently subscribed to,
    /// regardless of whether either parser has produced a price from them
    /// yet -- TEMPORARY DEBUG, added to confirm the push-oracle wiring is
    /// actually live (subscription count vs. parsed-price counts below).
    pub fn oracle_subscribed_count(&self) -> usize {
        self.m_oracle_to_bank.len()
    }

    /// How many banks currently have a live legacy Pyth price. TEMPORARY DEBUG.
    pub fn legacy_priced_count(&self) -> usize {
        self.m_price.len()
    }

    /// How many banks currently have a live Pyth Push Oracle price. TEMPORARY DEBUG.
    pub fn push_priced_count(&self) -> usize {
        self.m_push_price.len()
    }

    /// TEMPORARY DEBUG: one line per bank with a subscribed oracle --
    /// mint pubkey, oracle_setup, and which parser (if any) has produced a
    /// price for it. Added to pin down exactly which bank the SOL/USD
    /// diagnostic should be matching, since `bank_count`/`priced_count`
    /// alone don't say which specific bank is or isn't priced.
    pub fn debug_oracle_status(&self) -> String {
        let mut lines = Vec::new();
        for (&oracle_id, &bank_id) in self.m_oracle_to_bank.iter() {
            let Some(bank) = self.m_bank.get(&bank_id) else { continue };
            let mint_str = resolve(bank.mint)
                .map(|pk| pk.to_string())
                .unwrap_or_else(|_| "?".to_string());
            let status = if self.m_price.contains_key(&bank_id) {
                "legacy"
            } else if self.m_push_price.contains_key(&bank_id) {
                "push"
            } else if self.m_switchboard_price.contains_key(&bank_id) {
                "switchboard"
            } else {
                "none"
            };
            lines.push(format!(
                "mint={} oracle_setup={} oracle_id={:?} priced={}",
                mint_str, bank.oracle_setup, oracle_id, status
            ));
        }
        lines.join(" | ")
    }

    /// Latest parsed oracle price for whichever bank tracks `mint` (e.g.
    /// wrapped SOL), if any -- `None` if no bank for that mint exists, or
    /// its oracle uses a format this bot doesn't parse, or hasn't
    /// delivered a price yet. Checks legacy Pyth, then Pyth Push Oracle,
    /// then Switchboard pull-feed -- a given bank's oracle only ever
    /// populates one of the three, so order doesn't matter in practice. An
    /// independent, oracle-sourced ground truth for validating the
    /// router's own AMM-derived pricing against (see `arbv1::state.rs`'s
    /// `trade router check` diagnostic).
    pub fn price_for_mint(&self, mint: AccountId) -> Option<pyth::OraclePrice> {
        let (&bank_id, _) = self.m_bank.iter().find(|(_, b)| b.mint == mint)?;
        self.price_for_bank(bank_id)
    }

    /// Same as [`Self::price_for_mint`], but for a specific, already-known
    /// `bank_id` rather than re-searching by mint. Needed because a mint
    /// like SOL has multiple marginfi banks (6 real ones: one plain bank
    /// this bot trades, plus 5 protocol-partner collateral-integration
    /// banks -- `KaminoPythPush`, `DriftPythPull`, etc. -- whose oracles
    /// use formats this bot doesn't parse). `price_for_mint`'s
    /// `m_bank.iter().find(|b| b.mint == mint)` has no way to prefer the
    /// plain bank over those others (`HashMap` iteration order is
    /// arbitrary), so it can silently return `None` forever even once the
    /// *correct* bank (the one `reserve_by_mint` picks) is fully priced --
    /// live-confirmed this session for marginfi's SOL borrow-hedge test
    /// phase. Callers that already have `bank_id` from `reserve_by_mint`
    /// (i.e. anything about to trade against a specific bank) should use
    /// this instead.
    pub fn price_for_bank(&self, bank_id: AccountId) -> Option<pyth::OraclePrice> {
        if let Some(&price) = self.m_price.get(&bank_id) {
            return Some(price.into());
        }
        if let Some(&price) = self.m_push_price.get(&bank_id) {
            return Some(price.into());
        }
        self.m_switchboard_price.get(&bank_id).copied().map(Into::into)
    }

    /// Build a priced [`crate::trader::credit::CreditReserve`] for `bank_id`,
    /// using the latest oracle price seen so far (`None` if this bank's
    /// oracle type isn't parsed yet, or no update has arrived).
    pub fn credit_reserve(
        &self,
        bank_id: AccountId,
    ) -> Option<crate::trader::credit::CreditReserve> {
        let bank = self.m_bank.get(&bank_id)?;
        let price_usd = self
            .m_price
            .get(&bank_id)
            .map(|p| p.price_usd)
            .or_else(|| self.m_push_price.get(&bank_id).map(|p| p.price_usd))
            .or_else(|| self.m_switchboard_price.get(&bank_id).map(|p| p.price_usd));
        Some(crate::trader::credit::CreditReserve::from_marginfi(
            bank_id, bank, price_usd,
        ))
    }

    // ─── Instruction builders ─────────────────────────────────────────────

    /// Build the `[bank, oracle]` `remaining_accounts` health-check pairs
    /// `borrow`/`withdraw` need for every active position, confirmed
    /// against `MarginfiAccount::sort_balances`/`get_health_components`
    /// (`state/marginfi_account.rs`): the account's balances are sorted
    /// **descending by bank pubkey**, then each active balance contributes
    /// its bank followed by its oracle account(s).
    ///
    /// This bot only trades banks using `PythPushOracle`/`SwitchboardPull`
    /// (see [`ORACLE_SETUP_PYTH_PUSH`]'s doc comment -- `reserve_by_mint`
    /// only ever returns banks of these two types in the first place) --
    /// a bank using any other oracle setup in `bank_ids` returns `Err`
    /// rather than silently building an incomplete/misordered
    /// remaining_accounts list (which the real account's health check
    /// would reject anyway, but failing here is clearer about why). Both
    /// supported types need exactly one remaining account, and it's
    /// `oracle_keys[0]` directly (confirmed via real source -- see the
    /// same doc comment), i.e. the already-parsed `oracle_key` field.
    fn build_remaining_accounts(&self, bank_ids: &[AccountId]) -> Result<Vec<AccountMeta>, TraderError> {
        let mut entries: Vec<(Pubkey, Pubkey)> = Vec::with_capacity(bank_ids.len());
        for &id in bank_ids {
            let bank = self
                .m_bank
                .get(&id)
                .ok_or(TraderError::PubkeyResolutionFailed(id))?;
            if bank.oracle_setup != ORACLE_SETUP_PYTH_PUSH && bank.oracle_setup != ORACLE_SETUP_SWITCHBOARD_PULL {
                return Err(TraderError::MissingConfig(
                    "marginfi: bank uses an oracle_setup this bot doesn't parse yet, can't build remaining_accounts",
                ));
            }
            entries.push((resolve(id)?, resolve(bank.oracle_key)?));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0));

        let mut accounts = Vec::with_capacity(entries.len() * 2);
        for (bank_pk, oracle_pk) in entries {
            accounts.push(AccountMeta::new_readonly(bank_pk, false));
            accounts.push(AccountMeta::new_readonly(oracle_pk, false));
        }
        Ok(accounts)
    }

    /// Append a `lending_account_deposit` instruction to `wallet`.
    /// `deposit_up_to_limit` clamps `amount` down to the bank's remaining
    /// deposit capacity instead of erroring if it would be exceeded.
    /// Append a `lending_account_deposit` instruction to `wallet`.
    /// `deposit_up_to_limit` clamps `amount` down to the bank's remaining
    /// deposit capacity instead of erroring if it would be exceeded.
    pub fn deposit(
        &self,
        bank_id: AccountId,
        group: AccountId,
        marginfi_account: AccountId,
        authority: AccountId,
        signer_token_account: AccountId,
        amount: u64,
        deposit_up_to_limit: bool,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let group_pk = resolve(group)?;
        let account_pk = resolve(marginfi_account)?;
        let authority_pk = resolve(authority)?;
        let bank_pk = resolve(bank_id)?;
        let vault_pk = bank_liquidity_vault(&bank_pk);

        let mut data = Vec::with_capacity(18);
        data.extend_from_slice(&DISC_LENDING_ACCOUNT_DEPOSIT);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(1u8); // deposit_up_to_limit: Some(..)
        data.push(deposit_up_to_limit as u8);

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: MARGINFI_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new_readonly(group_pk, false),
                    AccountMeta::new(account_pk, false),
                    AccountMeta::new_readonly(authority_pk, true),
                    AccountMeta::new(bank_pk, false),
                    AccountMeta::new(resolve(signer_token_account)?, false),
                    AccountMeta::new(vault_pk, false),
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
                ],
                data,
            },
            MARGINFI_DEPOSIT_CU,
        );
        Ok(())
    }

    /// Append a `lending_account_repay` instruction to `wallet`. Pass
    /// `repay_all = true` to repay the account's full liability on this
    /// bank exactly (`amount` is ignored by the program in that case).
    pub fn repay(
        &self,
        bank_id: AccountId,
        group: AccountId,
        marginfi_account: AccountId,
        authority: AccountId,
        signer_token_account: AccountId,
        amount: u64,
        repay_all: bool,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let group_pk = resolve(group)?;
        let account_pk = resolve(marginfi_account)?;
        let authority_pk = resolve(authority)?;
        let bank_pk = resolve(bank_id)?;
        let vault_pk = bank_liquidity_vault(&bank_pk);

        let mut data = Vec::with_capacity(18);
        data.extend_from_slice(&DISC_LENDING_ACCOUNT_REPAY);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(1u8); // repay_all: Some(..)
        data.push(repay_all as u8);

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: MARGINFI_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new_readonly(group_pk, false),
                    AccountMeta::new(account_pk, false),
                    AccountMeta::new_readonly(authority_pk, true),
                    AccountMeta::new(bank_pk, false),
                    AccountMeta::new(resolve(signer_token_account)?, false),
                    AccountMeta::new(vault_pk, false),
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
                ],
                data,
            },
            MARGINFI_REPAY_CU,
        );
        Ok(())
    }

    /// Append a `lending_account_borrow` instruction to `wallet`.
    /// `other_active_banks` must list every OTHER bank this account
    /// currently has an active balance in (this bank is appended
    /// automatically, since borrowing from it makes it active too) -- this
    /// bot doesn't track a user's actual open positions, so the caller must
    /// supply the list (see the credit-graph module's non-goals). See
    /// [`Self::build_remaining_accounts`] for the health-check accounts this
    /// produces, and its constraint that every listed bank must be
    /// Pyth-legacy-priced.
    pub fn borrow(
        &self,
        bank_id: AccountId,
        group: AccountId,
        marginfi_account: AccountId,
        authority: AccountId,
        destination_token_account: AccountId,
        amount: u64,
        other_active_banks: &[AccountId],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let group_pk = resolve(group)?;
        let account_pk = resolve(marginfi_account)?;
        let authority_pk = resolve(authority)?;
        let bank_pk = resolve(bank_id)?;
        let vault_pk = bank_liquidity_vault(&bank_pk);
        let vault_authority_pk = bank_liquidity_vault_authority(&bank_pk);

        let mut all_banks = Vec::with_capacity(other_active_banks.len() + 1);
        all_banks.extend_from_slice(other_active_banks);
        all_banks.push(bank_id);
        let remaining_accounts = self.build_remaining_accounts(&all_banks)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_LENDING_ACCOUNT_BORROW);
        data.extend_from_slice(&amount.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new_readonly(group_pk, false),
            AccountMeta::new(account_pk, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(bank_pk, false),
            AccountMeta::new(resolve(destination_token_account)?, false),
            AccountMeta::new_readonly(vault_authority_pk, false),
            AccountMeta::new(vault_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        accounts.extend(remaining_accounts);

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: MARGINFI_PROGRAM_ID,
                accounts,
                data,
            },
            MARGINFI_BORROW_BASE_CU + all_banks.len() as u32 * MARGINFI_BORROW_PER_BANK_CU,
        );
        Ok(())
    }

    /// Append a `lending_account_withdraw` instruction to `wallet`. Pass
    /// `withdraw_all = true` to withdraw the account's full deposit on this
    /// bank exactly (`amount` is ignored by the program in that case). See
    /// [`Self::borrow`] for `other_active_banks`/remaining_accounts.
    pub fn withdraw(
        &self,
        bank_id: AccountId,
        group: AccountId,
        marginfi_account: AccountId,
        authority: AccountId,
        destination_token_account: AccountId,
        amount: u64,
        withdraw_all: bool,
        other_active_banks: &[AccountId],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let group_pk = resolve(group)?;
        let account_pk = resolve(marginfi_account)?;
        let authority_pk = resolve(authority)?;
        let bank_pk = resolve(bank_id)?;
        let vault_pk = bank_liquidity_vault(&bank_pk);
        let vault_authority_pk = bank_liquidity_vault_authority(&bank_pk);

        let mut all_banks = Vec::with_capacity(other_active_banks.len() + 1);
        all_banks.extend_from_slice(other_active_banks);
        all_banks.push(bank_id);
        let remaining_accounts = self.build_remaining_accounts(&all_banks)?;

        let mut data = Vec::with_capacity(18);
        data.extend_from_slice(&DISC_LENDING_ACCOUNT_WITHDRAW);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(1u8); // withdraw_all: Some(..)
        data.push(withdraw_all as u8);

        let mut accounts = vec![
            AccountMeta::new_readonly(group_pk, false),
            AccountMeta::new(account_pk, false),
            AccountMeta::new_readonly(authority_pk, true),
            AccountMeta::new(bank_pk, false),
            AccountMeta::new(resolve(destination_token_account)?, false),
            AccountMeta::new_readonly(vault_authority_pk, false),
            AccountMeta::new(vault_pk, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        ];
        accounts.extend(remaining_accounts);

        wallet.require_signer(authority);
        wallet.append_ix(
            Instruction {
                program_id: MARGINFI_PROGRAM_ID,
                accounts,
                data,
            },
            MARGINFI_WITHDRAW_BASE_CU + all_banks.len() as u32 * MARGINFI_WITHDRAW_PER_BANK_CU,
        );
        Ok(())
    }
}

impl Updater for MarginfiState {
    fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if let Some(bank) = self.m_bank.get_mut(&header.accountid) {
            if let Some(parsed) = parse(body) {
                *bank = parsed;
            }
            return;
        }
        if let Some(&bank_id) = self.m_oracle_to_bank.get(&header.accountid) {
            if let Some(price) = pyth::parse_legacy(body) {
                self.m_price.insert(bank_id, price);
            } else if let Some(price) = pyth::parse_push_oracle(body) {
                self.m_push_price.insert(bank_id, price);
            } else if let Some(price) = pyth::parse_switchboard_pull(body) {
                self.m_switchboard_price.insert(bank_id, price);
            }
        }
    }

    fn on_token(&mut self, _ta: &crate::catscope::witbot::shooter::Tokenaccountv1) -> bool {
        // MarginFi's available liquidity comes directly from a Bank's own
        // total_asset_shares/total_liability_shares (already read in
        // parse()), not from watching a vault's SPL Token balance the way
        // Kamino's supply_vault/fee_vault tracking does -- there is
        // nothing for this hook to track.
        false
    }

    fn batch_router(&mut self, _router: &mut TradeRouter) {
        // A lending Bank has no swap price to contribute to TradeRouter --
        // see the module doc comment. Genuine no-op, not unimplemented.
    }

    fn on_tx(&mut self, _ix: &crate::txview::CatscopeInstructionRead<'_>, _slot: &Slot) {}

    fn flush_pool(&mut self, _g: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}

// ─── This bot's own position (`MarginfiAccount`) ───────────────────────────

/// One active balance within this bot's own `MarginfiAccount` -- raw share
/// units (multiply by the bank's own `asset_share_value`/
/// `liability_share_value`, e.g. via [`MarginfiBank`], to get real token
/// amounts, same conversion `MarginfiBank::utilization` already applies).
/// Mirrors `solend::SolendCollateral`/`kamino::KaminoCollateral`'s role,
/// but marginfi tracks a deposit and a borrow in the *same* slot rather
/// than separate arrays -- real source's `Balance::get_side` asserts only
/// one side is ever nonzero per slot, confirmed live (see
/// [`parse_marginfi_lending_account`]'s doc comment).
#[derive(Debug, Clone, Copy, Default)]
pub struct MarginfiBalance {
    pub bank_id: AccountId,
    pub asset_shares: f64,
    pub liability_shares: f64,
}

/// Parsed subset of a real `MarginfiAccount` -- this bot's own lending
/// position (every bank it currently has a deposit or borrow in), not a
/// Bank itself. Mirrors `solend::SolendObligation`/`kamino::KaminoObligation`'s
/// shape (fixed-slot struct, find-by-bank accessors).
#[derive(Debug, Default, Clone)]
pub struct MarginfiLendingAccount {
    pub balances: Vec<MarginfiBalance>,
}

impl MarginfiLendingAccount {
    /// The deposit (if any) in `bank_id`.
    pub fn deposit_for(&self, bank_id: AccountId) -> Option<&MarginfiBalance> {
        self.balances.iter().find(|b| b.bank_id == bank_id && b.asset_shares > 0.0)
    }

    /// The borrow (if any) against `bank_id`.
    pub fn borrow_for(&self, bank_id: AccountId) -> Option<&MarginfiBalance> {
        self.balances.iter().find(|b| b.bank_id == bank_id && b.liability_shares > 0.0)
    }

    /// Every bank this account currently has an active balance in, other
    /// than `exclude` -- exactly what `MarginfiState::borrow`/`withdraw`'s
    /// `other_active_banks` param needs (this bot doesn't track a user's
    /// open positions any other way, see those methods' doc comments).
    pub fn other_active_banks(&self, exclude: AccountId) -> Vec<AccountId> {
        self.balances.iter().filter(|b| b.bank_id != exclude).map(|b| b.bank_id).collect()
    }
}

/// Absolute offset of `lending_account.balances[0]` within a real
/// `MarginfiAccount`'s raw body (including its 8-byte Anchor
/// discriminator): `group: Pubkey @8` + `authority: Pubkey @40` = 72.
const OFF_MFA_BALANCES: usize = 72;
/// `Balance` is `#[repr(C)]`, `assert_struct_size!(Balance, 104)` in real
/// source -- confirmed live (see [`parse_marginfi_lending_account`]).
const MFA_BALANCE_STRIDE: usize = 104;
/// `LendingAccount::balances: [Balance; MAX_LENDING_ACCOUNT_BALANCES]`,
/// `MAX_LENDING_ACCOUNT_BALANCES = 16` in real source.
const MFA_BALANCES_COUNT: usize = 16;
const OFF_BAL_ACTIVE: usize = 0;
const OFF_BAL_BANK_PK: usize = 1;
const OFF_BAL_ASSET_SHARES: usize = 40;
const OFF_BAL_LIABILITY_SHARES: usize = 56;
const MIN_MARGINFI_ACCOUNT_LEN: usize = OFF_MFA_BALANCES + MFA_BALANCES_COUNT * MFA_BALANCE_STRIDE;

/// Parse a real `MarginfiAccount`'s `lending_account.balances[]` from raw
/// body bytes (including the 8-byte Anchor discriminator). An unused slot
/// has `active == 0` and is filtered out here (no length byte the way
/// `LendingAccount` itself works -- `active` is the per-slot flag).
///
/// **Layout confirmed two ways**: (1) against the public
/// `0dotxyz/marginfi-v2` source (`type-crate/src/types/user_account.rs`)
/// -- `MarginfiAccount { group: Pubkey, authority: Pubkey, lending_account:
/// LendingAccount, .. }` (`assert_struct_size!(MarginfiAccount, 2304)`),
/// `LendingAccount { balances: [Balance; 16], .. }`
/// (`assert_struct_size!(LendingAccount, 1728)`), `Balance { active: u8,
/// bank_pk: Pubkey, bank_asset_tag: u8, tag: u16, _pad0: [u8; 4],
/// asset_shares: WrappedI80F48, liability_shares: WrappedI80F48, .. }`
/// (`assert_struct_size!(Balance, 104)`) -- all `#[repr(C)]`, so the field
/// order directly gives `bank_pk@+1`, `asset_shares@+40`,
/// `liability_shares@+56` within each 104-byte slot; (2) *live*, against a
/// real mainnet account with active balances
/// (`1145HABcj6DXMMd4j75bcEKJksHE9qxxUPJgMU9Ag4v`, found via
/// `getProgramAccounts` filtered to the real `MarginfiAccount` Anchor
/// discriminator `[67,178,130,109,126,114,28,42]` -- confirmed to equal
/// `discriminators::ACCOUNT` in the same real source's `constants.rs`, an
/// exact match, not `sha256`-derived): both of its two active balance
/// slots decoded to bank pubkeys that are themselves real, live Bank
/// accounts on-chain (`discriminators::BANK` match, plausible mint), with
/// plausible nonzero share values (~90k and ~479k) on the expected side
/// only in each case -- slot 0 had only `asset_shares` nonzero, slot 1
/// only `liability_shares`, matching real source's invariant that a
/// balance is never simultaneously an asset and a liability.
pub fn parse_marginfi_lending_account(body: &[u8]) -> Option<MarginfiLendingAccount> {
    if body.len() < MIN_MARGINFI_ACCOUNT_LEN {
        return None;
    }
    let mut balances = Vec::new();
    for i in 0..MFA_BALANCES_COUNT {
        let base = OFF_MFA_BALANCES + i * MFA_BALANCE_STRIDE;
        if body[base + OFF_BAL_ACTIVE] == 0 {
            continue;
        }
        let bank_id = account_id_from_pubkey(&Pubkey::new_from_array(
            body[base + OFF_BAL_BANK_PK..base + OFF_BAL_BANK_PK + 32]
                .try_into()
                .unwrap(),
        ));
        let asset_shares = i128::from_le_bytes(
            body[base + OFF_BAL_ASSET_SHARES..base + OFF_BAL_ASSET_SHARES + 16]
                .try_into()
                .unwrap(),
        ) as f64
            / I80F48_SCALE;
        let liability_shares = i128::from_le_bytes(
            body[base + OFF_BAL_LIABILITY_SHARES..base + OFF_BAL_LIABILITY_SHARES + 16]
                .try_into()
                .unwrap(),
        ) as f64
            / I80F48_SCALE;
        balances.push(MarginfiBalance {
            bank_id,
            asset_shares,
            liability_shares,
        });
    }
    Some(MarginfiLendingAccount { balances })
}

/// This bot's own marginfi lending position -- tracks whether *this bot's*
/// `MarginfiAccount` ([`initialize_account_pda`]'s PDA) is initialized and
/// its current contents. Mirrors `solend::SolendPosition`'s shape
/// (single-step bootstrap, no per-owner metadata account the way Kamino
/// needs) -- always scoped to [`MARGINFI_MAIN_GROUP`], this bot's only
/// group, so no live group lookup is needed to know it. Reserve
/// pricing/instruction-building keep coming from the separate, shared,
/// read-only `MarginfiState` (e.g. via `DexState::marginfi()`).
#[derive(Debug, Default)]
pub struct MarginfiPosition {
    o_authority_pk: Option<Pubkey>,
    o_account_id: Option<AccountId>,
    o_lending_account: Option<MarginfiLendingAccount>,
    subscriptions: Vec<Subscription>,
}

impl MarginfiPosition {
    /// Pure-derivation half of the old single-shot `set_authority`
    /// (removed -- only ever called from `testperpv1`'s
    /// `Wallet` message handler, alongside Phoenix/Solend/Kamino's own
    /// subscription calls). Returns the subscription request this
    /// authority needs -- this bot's own `MarginfiAccount`
    /// ([`marginfi_account_pda`] against [`MARGINFI_MAIN_GROUP`]) --
    /// empty if already set, without making the host `subscribe` call
    /// itself. Paired with [`Self::apply_authority`] so every venue's
    /// requests can be batched into one `bulk_subscribe` round-trip
    /// instead of five separate ones. Real, live-observed incident: those
    /// five one-at-a-time calls accounted for ~26 seconds of stall in one
    /// run (traced via `CommitHook::start`'s own timing diagnostics).
    pub fn authority_subscribe_requests(&self, authority: Pubkey) -> Vec<SubscriptionRequest> {
        if self.o_authority_pk == Some(authority) {
            return Vec::new();
        }
        let account_pk = marginfi_account_pda(&MARGINFI_MAIN_GROUP, &authority);
        vec![SubscriptionRequest { root: account_id_from_pubkey(&account_pk), filter_weight: 0, depth: 1 }]
    }

    /// Apply `authority` plus its already-resolved subscription (from
    /// [`Self::authority_subscribe_requests`]) -- the second half of the
    /// split described there. No-op if `subs` is empty (either already
    /// set, or nothing to apply).
    pub fn apply_authority(&mut self, authority: Pubkey, subs: Vec<Subscription>) {
        let Some(sub) = subs.into_iter().next() else { return };
        let account_pk = marginfi_account_pda(&MARGINFI_MAIN_GROUP, &authority);
        let account_id = account_id_from_pubkey(&account_pk);
        self.subscriptions.push(sub);
        self.o_authority_pk = Some(authority);
        self.o_account_id = Some(account_id);
    }

    pub fn account_id(&self) -> Option<AccountId> {
        self.o_account_id
    }

    /// `true` only once a real update for the account has been parsed --
    /// means it actually exists on-chain ([`initialize_account_pda`]
    /// already succeeded), not just "we know its address and subscribed."
    /// Same reasoning as `solend::SolendPosition::registered`.
    pub fn registered(&self) -> bool {
        self.o_lending_account.is_some()
    }

    /// Last successfully parsed lending account, if any real update has
    /// arrived yet.
    pub fn lending_account(&self) -> Option<&MarginfiLendingAccount> {
        self.o_lending_account.as_ref()
    }

    /// Every bank this position currently has an active balance in, other
    /// than `exclude` -- see [`MarginfiLendingAccount::other_active_banks`].
    /// Empty if [`Self::registered`] is `false`.
    pub fn other_active_banks(&self, exclude: AccountId) -> Vec<AccountId> {
        self.o_lending_account
            .as_ref()
            .map(|la| la.other_active_banks(exclude))
            .unwrap_or_default()
    }

    pub fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if Some(header.accountid) == self.o_account_id {
            if let Some(la) = parse_marginfi_lending_account(body) {
                self.o_lending_account = Some(la);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    // Pure, host-independent PDA/discriminator checks only -- see the same
    // note in `kamino.rs`'s test module for why the instruction-builder
    // methods themselves (which call `pubkey_from_account_id`) aren't
    // unit-tested here.

    fn disc(name: &str) -> [u8; 8] {
        let hash = Sha256::digest(format!("global:{name}").as_bytes());
        hash[..8].try_into().unwrap()
    }

    #[test]
    fn discriminators_match_their_instruction_names() {
        assert_eq!(disc("marginfi_account_initialize_pda"), DISC_INITIALIZE_ACCOUNT_PDA);
        assert_eq!(disc("lending_account_deposit"), DISC_LENDING_ACCOUNT_DEPOSIT);
        assert_eq!(disc("lending_account_borrow"), DISC_LENDING_ACCOUNT_BORROW);
        assert_eq!(disc("lending_account_repay"), DISC_LENDING_ACCOUNT_REPAY);
        assert_eq!(disc("lending_account_withdraw"), DISC_LENDING_ACCOUNT_WITHDRAW);
    }

    #[test]
    fn marginfi_account_pda_is_deterministic_and_authority_specific() {
        let group = Pubkey::new_unique();
        let authority_a = Pubkey::new_unique();
        let authority_b = Pubkey::new_unique();
        assert_eq!(
            marginfi_account_pda(&group, &authority_a),
            marginfi_account_pda(&group, &authority_a)
        );
        assert_ne!(
            marginfi_account_pda(&group, &authority_a),
            marginfi_account_pda(&group, &authority_b)
        );
    }

    #[test]
    fn bank_liquidity_vault_and_authority_differ_and_are_bank_specific() {
        let bank_a = Pubkey::new_unique();
        let bank_b = Pubkey::new_unique();
        assert_ne!(bank_liquidity_vault(&bank_a), bank_liquidity_vault_authority(&bank_a));
        assert_ne!(bank_liquidity_vault(&bank_a), bank_liquidity_vault(&bank_b));
    }

    fn bank_with_oracle(oracle_setup: u8) -> MarginfiBank {
        MarginfiBank {
            oracle_setup,
            ..Default::default()
        }
    }

    #[test]
    fn build_remaining_accounts_rejects_unsupported_oracle_setups() {
        // This is the one `build_remaining_accounts` path that returns
        // *before* calling `resolve()` (a WASM-host import that panics
        // under native `cargo test`), so it's the only part of that method
        // safely exercisable here -- see `kamino.rs`'s test module note.
        let mut m_bank = HashMap::default();
        m_bank.insert(1u64, bank_with_oracle(1)); // PythLegacy -- not PythPushOracle/SwitchboardPull
        let state = MarginfiState {
            program_id: 0,
            m_bank,
            m_oracle_to_bank: HashMap::default(),
            m_price: HashMap::default(),
            m_push_price: HashMap::default(),
            m_switchboard_price: HashMap::default(),
        };
        let err = state.build_remaining_accounts(&[1]).unwrap_err();
        assert!(matches!(err, TraderError::MissingConfig(_)));
    }

    // `parse_marginfi_lending_account` itself isn't unit-tested here -- it
    // calls `account_id_from_pubkey` internally (a WIT host import that
    // aborts outside the real WASM guest runtime), same established
    // boundary as `kamino::parse_kamino_obligation`.
    // `MarginfiLendingAccount::deposit_for`/`borrow_for`/`other_active_banks`
    // have no such dependency and are fully testable directly.
    fn lending_account_with_positions() -> MarginfiLendingAccount {
        MarginfiLendingAccount {
            balances: vec![
                MarginfiBalance { bank_id: 7, asset_shares: 90_051.3, liability_shares: 0.0 },
                MarginfiBalance { bank_id: 9, asset_shares: 0.0, liability_shares: 478_903.3 },
            ],
        }
    }

    #[test]
    fn deposit_for_finds_position_by_bank() {
        let la = lending_account_with_positions();
        let d = la.deposit_for(7).expect("expected a deposit in bank 7");
        assert!((d.asset_shares - 90_051.3).abs() < 1e-6);
        assert!(la.deposit_for(9).is_none()); // bank 9 only has a borrow
        assert!(la.deposit_for(8).is_none()); // not present at all
    }

    #[test]
    fn borrow_for_finds_position_by_bank() {
        let la = lending_account_with_positions();
        let b = la.borrow_for(9).expect("expected a borrow against bank 9");
        assert!((b.liability_shares - 478_903.3).abs() < 1e-6);
        assert!(la.borrow_for(7).is_none()); // bank 7 only has a deposit
        assert!(la.borrow_for(8).is_none()); // not present at all
    }

    #[test]
    fn other_active_banks_excludes_the_given_bank() {
        let la = lending_account_with_positions();
        let mut others = la.other_active_banks(7);
        others.sort();
        assert_eq!(others, vec![9]);
        let mut others_all = la.other_active_banks(1); // 1 isn't held at all
        others_all.sort();
        assert_eq!(others_all, vec![7, 9]);
    }

    #[test]
    fn marginfi_position_other_active_banks_empty_before_registered() {
        let pos = MarginfiPosition::default();
        assert!(pos.other_active_banks(7).is_empty());
        assert!(!pos.registered());
    }
}
