//! Kamino Lending reserve account parser, pricing, and flash-loan instruction builder.
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
//!  25        1   last_update.price_status (u8)
//!  26        6   last_update.placeholder ([u8; 6])
//!  32       32   lending_market (Pubkey)
//!  64       32   farm_collateral (Pubkey)
//!  96       32   farm_debt (Pubkey)
//! 128       32   liquidity.mint_pubkey (Pubkey)        ← token mint
//! 160       32   liquidity.supply_vault (Pubkey)       ← flash-loan source/dest
//! 192       32   liquidity.fee_vault (Pubkey)          ← flash-loan fee receiver
//! 224        8   liquidity.total_available_amount (u64) ← liquid supply
//! 232       16   liquidity.borrowed_amount_sf (u128)
//! 248       16   liquidity.market_price_sf (u128)      ← oracle price (SF)
//! 264        8   liquidity.market_price_last_updated_ts (u64)
//! 272        8   liquidity.mint_decimals (u64)
//! 4896       8   config.fees.origination_fee_sf (u64)
//! 4904       8   config.fees.flash_loan_fee_sf (u64)   ← flash-loan fee (SF)
//! ```
//!
//! Offsets 224 onward (and 4896/4904) were verified against live mainnet
//! reserve accounts, not just the public `Kamino-Finance/klend` source --
//! that source's field layout doesn't fully match the deployed binary (it's
//! missing a field that pushes everything from `total_available_amount`
//! onward earlier than the source alone would suggest). The previous
//! offsets here (224→256, 248→280, 272→304) were wrong by 32 bytes each,
//! producing garbage `available_amount`/`price_usd`/`mint_decimals`.
//!
//! # `market_price_sf` / `flash_loan_fee_sf` encoding
//!
//! ```text
//! actual_price_usd  = market_price_sf as f64 / 2^60
//! flash_loan_fee    = flash_loan_fee_sf as f64 / 2^60   (e.g. 0.0005 = 0.05%)
//! ```
//!
//! # Flash loans
//!
//! Kamino supports atomic flash borrows. The pair of instructions:
//! `flash_borrow_reserve_liquidity` + `flash_repay_reserve_liquidity`
//! must appear in the same transaction (Kamino validates via SYSVAR_INSTRUCTIONS).
//!
//! ```text
//! ix[0]  ComputeBudget::SetComputeUnitLimit
//! ix[1]  ComputeBudget::SetComputeUnitPrice
//! ix[2]  flash_borrow_reserve_liquidity  ← borrow_instruction_index = 2
//! ix[3…] your arbitrage / swap instructions
//! ix[N]  flash_repay_reserve_liquidity(amount, borrow_instruction_index=2)
//! ```

use crate::{
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionRequest},
    trader::{
        dex::update::Updater,
        types::{PoolPrice, TraderError},
    },
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::{system_program, sysvar::rent};
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

// ─── Program ID ───────────────────────────────────────────────────────────────

pub const KAMINO_LENDING_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");

/// Kamino's real main market -- confirmed via `api.kamino.finance/kamino-market`
/// -- the same lending market every currently-tracked reserve (SOL/BTC/ETH)
/// belongs to. Lets [`KaminoPosition`] derive this bot's obligation address
/// ([`obligation_pda`]) without needing a live reserve lookup first.
pub const KAMINO_MAIN_MARKET: Pubkey =
    Pubkey::from_str_const("7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF");

// ─── Well-known program IDs used in flash-loan accounts ───────────────────────

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const SYSVAR_INSTRUCTIONS_ID: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");

const KAMINO_FARMS_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr");

// ─── Reserve account offsets ──────────────────────────────────────────────────

const OFF_LENDING_MARKET: usize = 32;
/// `farm_collateral` -- live-verified against 4 real main-market reserves
/// (SOL/USDC have a real, Farms-program-owned account here; BTC/ETH are
/// null). Also cross-validated structurally: sits exactly between
/// `OFF_LENDING_MARKET` (32, size 32) and `OFF_FARM_DEBT` (96), both
/// already-verified neighbors.
const OFF_FARM_COLLATERAL: usize = 64;
/// `farm_debt` -- live-verified null on all 4 currently-tracked reserves;
/// see [`OFF_FARM_COLLATERAL`] for the verification method.
const OFF_FARM_DEBT: usize = 96;
const OFF_MINT: usize = 128;
const OFF_SUPPLY_VAULT: usize = 160;
const OFF_FEE_VAULT: usize = 192;
const OFF_AVAILABLE_AMOUNT: usize = 224;
const OFF_BORROWED_AMOUNT_SF: usize = 232;
const OFF_MARKET_PRICE_SF: usize = 248;
const OFF_MINT_DECIMALS: usize = 272;
/// `config.fees.flash_loan_fee_sf` -- see the account-layout doc comment
/// above for how this was empirically verified (real Reserve accounts are a
/// fixed 8624 bytes, so this offset is always in-bounds on a genuine one).
const OFF_FLASH_LOAN_FEE_SF: usize = 4904;
/// `config.loan_to_value_pct` -- verified empirically against 8 real
/// mainnet Reserve accounts: values came back as plausible round
/// percentages (0, 45, 70, 70, 70, 80, 90), and the adjacent byte
/// (`liquidation_threshold_pct`, offset+1) was consistently 5-10 points
/// higher, matching the expected LTV < liquidation-threshold relationship
/// for every real lending protocol.
const OFF_LOAN_TO_VALUE_PCT: usize = 4872;
/// `config.borrow_factor_pct` (u64) -- sits immediately after
/// `config.borrow_rate_curve` (`OFF_BORROW_RATE_CURVE` + 11 * 8 = 5008).
/// Live-verified against the real mainnet SOL reserve
/// (`d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q`): reads `125`, and the
/// `token_info.name` field 24 bytes later (offset 5032) reads `"SOL\0..."`,
/// confirming both the struct alignment and the reserve identity. See
/// [`KaminoReserve::borrow_factor_pct`] for the on-chain failure this
/// offset was found to explain.
const OFF_BORROW_FACTOR_PCT: usize = 5008;
/// `collateral.mint_pubkey` (the reserve's cToken mint) -- verified
/// empirically against a real
/// `withdraw_obligation_collateral_and_redeem_reserve_collateral_v2`
/// transaction (which names this exact account) and cross-checked against
/// 4 more mainnet Reserve accounts. NOT derivable via klend's
/// `pda::init_reserve_pdas` seed scheme (`["reserve_coll_mint", reserve]`)
/// -- that PDA does not match the address actually stored in real reserves,
/// so these were created as plain accounts at `init_reserve` time, not PDAs.
const OFF_COLLATERAL_MINT: usize = 2560;
/// `collateral.supply_vault` -- see `OFF_COLLATERAL_MINT` for verification
/// method; same "not actually a PDA in practice" caveat applies.
const OFF_COLLATERAL_SUPPLY_VAULT: usize = 2600;
/// `config.status` -- the first field of `ReserveConfig` (real klend
/// source: `status: u8` precedes every other config field), so this sits
/// at the same verified `config` struct base as
/// [`OFF_PROTOCOL_TAKE_RATE_PCT`] (4856 + 0). `0` = Active, `1` =
/// Obsolete, `2` = Hidden (`ReserveStatus` enum) -- live-verified against
/// real duplicate-mint reserves on the main market (e.g. 3 of 4 USDC
/// reserves on `KAMINO_MAIN_MARKET` are `Hidden`, only one `Active`).
const OFF_RESERVE_STATUS: usize = 4856;
/// `config.protocol_take_rate_pct` -- live-verified (this session's
/// oracle-account research pass), cross-checked two independent ways
/// against `OFF_LOAN_TO_VALUE_PCT`/`OFF_FLASH_LOAN_FEE_SF` (both agree the
/// real `config` struct base sits at absolute offset 4856, with no
/// binary-vs-source divergence found in this region, unlike the earlier
/// `ReserveLiquidity` bug).
const OFF_PROTOCOL_TAKE_RATE_PCT: usize = 4870;
/// `config.borrow_rate_curve: [CurvePoint { utilization_rate_bps: u32,
/// borrow_rate_bps: u32 }; 11]` -- live-verified, piecewise-linear jump-rate
/// curve, real annualized APR directly (not a per-slot rate).
const OFF_BORROW_RATE_CURVE: usize = 4920;
const BORROW_RATE_CURVE_POINTS: usize = 11;
/// `config.token_info.scope_configuration.price_feed` -- live-verified
/// (this session's oracle-account research): decoded bytes AND a real,
/// successful `refresh_reserve` transaction's account list both confirmed
/// this exact offset for SOL/BTC/ETH's real reserves (all three use Scope
/// only -- the other three oracle fields read as the null pubkey/`None`
/// for them today).
const OFF_SCOPE_PRICES: usize = 5112;
const OFF_SWITCHBOARD_PRICE_ORACLE: usize = 5160;
const OFF_SWITCHBOARD_TWAP_ORACLE: usize = 5192;
const OFF_PYTH_ORACLE: usize = 5224;

const MIN_RESERVE_LEN: usize = OFF_PYTH_ORACLE + 32;

/// Scaling factor for Kamino SF prices: `2^60`.
const SF_SCALE: f64 = (1u128 << 60) as f64;

// ─── Flash-loan instruction discriminators ────────────────────────────────────

/// sha256("global:flash_borrow_reserve_liquidity")[..8]
const DISC_FLASH_BORROW: [u8; 8] = [135, 231, 52, 167, 7, 52, 212, 193];

/// sha256("global:flash_repay_reserve_liquidity")[..8]
const DISC_FLASH_REPAY: [u8; 8] = [185, 117, 0, 203, 96, 245, 180, 186];

/// Compute-unit budget per flash-loan instruction (borrow or repay).
pub const KAMINO_FLASH_LOAN_CU: u32 = 200_000;

// ─── Regular lending instruction discriminators ───────────────────────────────
// All computed as sha256("global:<name>")[..8] and cross-checked directly
// against klend's handler source (github.com/Kamino-Finance/klend,
// programs/klend/src/lib.rs) -- the deprecated (undecorated) instructions are
// intentionally not used; klend's own SDK only builds the `_v2` variants.

/// sha256("global:init_user_metadata")[..8]
const DISC_INIT_USER_METADATA: [u8; 8] = [117, 169, 176, 69, 197, 23, 15, 162];
/// sha256("global:init_obligation")[..8]
const DISC_INIT_OBLIGATION: [u8; 8] = [251, 10, 231, 76, 27, 11, 159, 96];
/// sha256("global:refresh_reserve")[..8]
const DISC_REFRESH_RESERVE: [u8; 8] = [2, 218, 138, 235, 79, 201, 25, 102];
/// sha256("global:refresh_obligation")[..8]
const DISC_REFRESH_OBLIGATION: [u8; 8] = [33, 132, 147, 228, 151, 192, 72, 89];
/// sha256("global:deposit_reserve_liquidity_and_obligation_collateral_v2")[..8]
const DISC_DEPOSIT_V2: [u8; 8] = [216, 224, 191, 27, 204, 151, 102, 175];
/// sha256("global:borrow_obligation_liquidity_v2")[..8]
const DISC_BORROW_V2: [u8; 8] = [161, 128, 143, 245, 171, 199, 194, 6];
/// sha256("global:repay_obligation_liquidity_v2")[..8]
const DISC_REPAY_V2: [u8; 8] = [116, 174, 213, 76, 180, 53, 210, 144];
/// sha256("global:withdraw_obligation_collateral_and_redeem_reserve_collateral_v2")[..8]
const DISC_WITHDRAW_V2: [u8; 8] = [235, 52, 119, 152, 149, 197, 20, 7];
/// sha256("global:init_obligation_farms_for_reserve")[..8]
const DISC_INIT_OBLIGATION_FARMS_FOR_RESERVE: [u8; 8] = [136, 63, 15, 186, 211, 152, 168, 164];

/// Conservative, unmeasured compute-unit budgets -- tune later against real
/// `simulateTransaction` results. `KAMINO_REFRESH_OBLIGATION_PER_RESERVE_CU`
/// scales with `deposit_reserves.len() + borrow_reserves.len()` since
/// `refresh_obligation`'s cost grows with the number of reserves it touches.
pub const KAMINO_INIT_USER_METADATA_CU: u32 = 40_000;
pub const KAMINO_INIT_OBLIGATION_CU: u32 = 40_000;
pub const KAMINO_REFRESH_RESERVE_CU: u32 = 60_000;
pub const KAMINO_REFRESH_OBLIGATION_BASE_CU: u32 = 30_000;
pub const KAMINO_REFRESH_OBLIGATION_PER_RESERVE_CU: u32 = 15_000;
pub const KAMINO_DEPOSIT_CU: u32 = 180_000;
pub const KAMINO_BORROW_CU: u32 = 180_000;
pub const KAMINO_REPAY_CU: u32 = 120_000;
pub const KAMINO_WITHDRAW_CU: u32 = 180_000;
pub const KAMINO_INIT_OBLIGATION_FARMS_FOR_RESERVE_CU: u32 = 50_000;

/// `u64::MAX` signals "repay the obligation's full outstanding debt" /
/// "withdraw the reserve's entire deposited amount" -- confirmed against
/// klend's `Reserve::calculate_repay`/`withdraw_obligation_collateral`.
pub const KAMINO_AMOUNT_MAX: u64 = u64::MAX;

// ─── Parsed reserve state ─────────────────────────────────────────────────────

/// Build-time setup for one Kamino Lending reserve.
#[derive(Debug, Clone, Default)]
struct Setup {
    l_setup: Vec<KaminoSetupReserve>,
}

impl Setup {
    /// Build the reserve list from `kamino_config::KAMINO_RESERVES`
    /// (build.rs's SQL-generated address book), mirroring how
    /// `raydium::Setup::default` reads its own `_config::*_POOLS` statics.
    /// Was previously deriving to an always-empty `Vec` -- `KaminoState`
    /// had zero reserves configured at runtime despite `parse()` working
    /// correctly.
    fn from_config() -> Self {
        let mut l_setup = Vec::with_capacity(crate::kamino_config::KAMINO_RESERVES.len());
        let mut z = [0u8; 32];
        for raw in crate::kamino_config::KAMINO_RESERVES {
            let mut to_id = |bytes: &[u8; 32]| {
                z.copy_from_slice(bytes);
                account_id_from_pubkey(&Pubkey::new_from_array(z))
            };
            l_setup.push(KaminoSetupReserve {
                reserve: to_id(&raw.pubkey),
                supply_vault: to_id(&raw.supply_vault),
                fee_vault: to_id(&raw.fee_vault),
                lending_market: to_id(&raw.lending_market),
            });
        }
        Self { l_setup }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KaminoSetupReserve {
    /// Reserve state account pubkey.
    pub reserve: AccountId,
    pub supply_vault: AccountId,
    pub fee_vault: AccountId,
    pub lending_market: AccountId,
}

struct KaminioReserveWrapper {
    reserve: KaminoReserve,
    coin_vault_balance: u64,
    fee_vault_balance: u64,
}

/// Parsed Kamino Lending reserve account.
#[derive(Debug, Default, Clone)]
pub struct KaminoReserve {
    /// Mint of the token this reserve holds.
    pub token_mint: AccountId,
    /// Liquidity available for borrowing (raw units).
    pub available_amount: u64,
    /// Currently borrowed (raw native token units, SF-decoded: `/2^60`)
    /// -- same scale as `available_amount as f64`, for utilization math.
    pub borrowed_amount: f64,
    /// Oracle market price in USD (SF-decoded: `market_price_sf / 2^60`).
    pub price_usd: f64,
    /// Token decimal places as reported in the reserve.
    pub mint_decimals: u64,
    /// Flash-loan fee as a fraction (SF-decoded: `flash_loan_fee_sf / 2^60`),
    /// e.g. `0.0005` = 0.05%. Compare against expected profit before
    /// deciding a flash loan is worth taking -- see `flash_loan_fee_raw` for
    /// the fee on a specific borrow amount.
    pub flash_loan_fee_fraction: f64,
    /// Max loan-to-value ratio (0.0-1.0) when this reserve's asset is
    /// deposited as collateral -- e.g. `0.7` means $1 of collateral here
    /// unlocks $0.70 of borrowing power elsewhere in the same lending
    /// market. `0.0` means this reserve can't be used as collateral at all
    /// (e.g. an isolated/borrow-only market).
    pub loan_to_value_pct: f64,
    /// `config.borrow_factor_pct` -- risk-adjustment multiplier applied to
    /// this reserve's asset when it's *borrowed* (not deposited): a $1.00
    /// borrow counts as `borrow_factor_pct / 100.0` dollars against the
    /// depositing reserve's max-borrow-value limit. Live-verified against
    /// the real mainnet SOL reserve (`1.25`, i.e. 125%) after a real
    /// `BorrowObligationLiquidityV2` transaction failed on-chain with
    /// `BorrowTooLarge` ("Borrow value 1.2500 cannot exceed maximum borrow
    /// value 1.1110") for a $1.00 notional borrow against $1.3888 USDC
    /// collateral at 80% LTV (`1.3888 * 0.80 = 1.1110`) -- the borrow side
    /// was never scaled by this factor, only the deposit side's LTV was.
    /// USDC's own reserve reads `1.00` (no risk premium), consistent with
    /// stablecoins typically being exempt. Must be folded into borrow-hedge
    /// collateral sizing: `collateral_usd = notional_usd * borrow_factor_pct
    /// / (deposit_reserve.loan_to_value_pct * SAFETY_FACTOR)`.
    pub borrow_factor_pct: f64,
    /// The lending market that owns this reserve.
    pub lending_market: AccountId,
    /// Vault that holds the reserve's liquid supply (source for flash borrows).
    pub supply_vault: AccountId,
    /// Vault that collects flash-loan fees.
    pub fee_vault: AccountId,
    /// This reserve's cToken mint (`collateral.mint_pubkey`) -- needed by
    /// deposit/withdraw, which move cTokens as obligation collateral.
    pub collateral_mint: AccountId,
    /// This reserve's cToken supply vault (`collateral.supply_vault`).
    pub collateral_supply_vault: AccountId,
    /// Real oracle accounts `refresh_reserve` needs -- `None` when the
    /// real bytes are the null pubkey (that oracle system isn't used by
    /// this reserve). All of SOL/BTC/ETH's real reserves currently use
    /// Scope only (`scope_prices: Some`, the other three `None`), but all
    /// four are parsed for real rather than assuming that stays true.
    pub pyth_oracle: Option<AccountId>,
    pub switchboard_price_oracle: Option<AccountId>,
    pub switchboard_twap_oracle: Option<AccountId>,
    pub scope_prices: Option<AccountId>,
    /// Protocol's cut (0-100) of borrower interest -- see
    /// [`Self::current_supply_apy`].
    pub protocol_take_rate_pct: u8,
    /// `(utilization_rate_bps, borrow_rate_bps)` control points -- real
    /// deployed jump-rate curve, piecewise-linear interpolation between
    /// points gives the real annualized borrow APR directly. Unused
    /// trailing slots are copies of the last real point (klend's own
    /// `BorrowRateCurve::from_points` padding convention).
    pub borrow_rate_curve: [(u32, u32); BORROW_RATE_CURVE_POINTS],
    /// `config.status` -- `0` = Active, `1` = Obsolete, `2` = Hidden. Real
    /// markets can carry multiple reserves for the same mint (e.g. after a
    /// migration); only the `Active` one should ever be deposited/borrowed
    /// into -- see [`Self::is_active`].
    pub status: u8,
    /// Real Kamino Farms account (owned by `KAMINO_FARMS_PROGRAM_ID`)
    /// attached to this reserve's collateral side, if any -- `None` when
    /// the real bytes are the null pubkey. Live-verified: SOL/USDC's
    /// main-market reserves have one, BTC/ETH don't. When `Some`,
    /// `deposit`/`withdraw` must reference the real farm accounts (see
    /// their doc comments) instead of the `None` placeholder -- confirmed
    /// via `simulateTransaction` (`Custom(6120) FarmAccountsMissing`
    /// otherwise).
    pub farm_collateral: Option<AccountId>,
    /// Same as [`Self::farm_collateral`] but for the reserve's debt side
    /// -- only relevant to `borrow`/`repay`. Live-verified null on every
    /// currently-tracked reserve, parsed for real anyway rather than
    /// assuming that stays true.
    pub farm_debt: Option<AccountId>,
}

impl KaminoReserve {
    /// `true` only for `config.status == 0` (Active) -- live-verified this
    /// matters in practice: `KAMINO_MAIN_MARKET` currently carries 4
    /// separate USDC reserves, 3 of them `Hidden`, only one `Active`.
    /// [`KaminoState::reserve_by_mint`] filters on this.
    pub fn is_active(&self) -> bool {
        self.status == 0
    }

    /// Build a synthetic [`PoolPrice`] denominated in USD.
    pub fn pool_price(&self) -> PoolPrice {
        PoolPrice {
            token_a: self.token_mint,
            token_b: 0,
            price: self.price_usd,
            reserve_a: self.available_amount,
            reserve_b: 0,
            fee_bps: 0,
        }
    }

    /// Flash-loan fee (raw token units) for borrowing `amount` from this
    /// reserve. Use this to decide whether a flash loan is worth taking:
    /// only borrow when `amount_out - amount_in > flash_loan_fee_raw(amount_in)`
    /// (plus tx costs) -- otherwise trading owned capital is strictly
    /// cheaper.
    pub fn flash_loan_fee_raw(&self, amount: u64) -> u64 {
        (amount as f64 * self.flash_loan_fee_fraction) as u64
    }

    /// Current utilization (0.0-1.0): borrowed / (available + borrowed).
    pub fn utilization(&self) -> f64 {
        let total = self.available_amount as f64 + self.borrowed_amount;
        if total <= 0.0 {
            return 0.0;
        }
        (self.borrowed_amount / total).min(1.0)
    }

    /// Real deployed borrow-rate curve: piecewise-linear interpolation
    /// across `borrow_rate_curve`'s control points on utilization --
    /// already an annualized APR directly (not a per-slot rate), confirmed
    /// against real on-chain values and `klend-sdk`'s own
    /// `interpolate`/`getBorrowRate`. A genuinely different curve shape
    /// than Solend's two/three-segment model, not reused from there.
    pub fn current_borrow_apy(&self) -> f64 {
        let util_bps = (self.utilization() * 10_000.0) as u32;
        let points = &self.borrow_rate_curve;
        if util_bps <= points[0].0 {
            return points[0].1 as f64 / 10_000.0;
        }
        for w in points.windows(2) {
            let (lo_util, lo_rate) = w[0];
            let (hi_util, hi_rate) = w[1];
            if util_bps <= hi_util {
                if hi_util == lo_util {
                    return hi_rate as f64 / 10_000.0;
                }
                let frac = (util_bps - lo_util) as f64 / (hi_util - lo_util) as f64;
                return (lo_rate as f64 + (hi_rate as f64 - lo_rate as f64) * frac) / 10_000.0;
            }
        }
        points[points.len() - 1].1 as f64 / 10_000.0
    }

    /// Real supply (lender) APR: `utilization * borrow_apy * (1 -
    /// protocol_take_rate)` -- same formula shape Solend already uses,
    /// independently confirmed for Kamino specifically (real on-chain
    /// `protocol_take_rate_pct` plus `klend-sdk`'s `reserve.ts`).
    pub fn current_supply_apy(&self) -> f64 {
        self.current_borrow_apy() * self.utilization() * (1.0 - self.protocol_take_rate_pct as f64 / 100.0)
    }

    // ─── Flash-loan instruction builders ─────────────────────────────────────

    /// Append a `flash_borrow_reserve_liquidity` instruction to `wallet`.
    ///
    /// `reserve_id`: this reserve's `AccountId`.
    /// `user_liquidity_account`: the user's token account that will receive the
    ///   borrowed tokens.
    /// The matching [`flash_repay`](Self::flash_repay) must be the last
    /// instruction in the same transaction.
    pub fn flash_borrow(
        &self,
        reserve_id: AccountId,
        amount: u64,
        user_wallet: AccountId,
        user_liquidity_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (
            reserve_pk,
            lm_pk,
            token_mint_pk,
            supply_vault_pk,
            user_liq_pk,
            fee_vault_pk,
            user_wallet_pk,
        ) = self.resolve_accounts(reserve_id, user_wallet, user_liquidity_account)?;
        let lm_authority = lending_market_authority(&lm_pk);

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_FLASH_BORROW);
        data.extend_from_slice(&amount.to_le_bytes());

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts: flash_accounts(
                    user_wallet_pk,
                    lm_authority,
                    lm_pk,
                    reserve_pk,
                    token_mint_pk,
                    supply_vault_pk,
                    user_liq_pk,
                    fee_vault_pk,
                ),
                data,
            },
            KAMINO_FLASH_LOAN_CU,
        );
        Ok(())
    }

    /// Append a `flash_repay_reserve_liquidity` instruction to `wallet`.
    ///
    /// `borrow_instruction_index`: the position of the `flash_borrow` instruction
    ///   in the final transaction. When two compute-budget instructions are
    ///   prepended by the wallet, this is typically `2`.
    /// `user_liquidity_account`: the user's token account that will repay.
    pub fn flash_repay(
        &self,
        reserve_id: AccountId,
        amount: u64,
        borrow_instruction_index: u8,
        user_wallet: AccountId,
        user_liquidity_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let (
            reserve_pk,
            lm_pk,
            token_mint_pk,
            supply_vault_pk,
            user_liq_pk,
            fee_vault_pk,
            user_wallet_pk,
        ) = self.resolve_accounts(reserve_id, user_wallet, user_liquidity_account)?;
        let lm_authority = lending_market_authority(&lm_pk);

        let mut data = Vec::with_capacity(17);
        data.extend_from_slice(&DISC_FLASH_REPAY);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(borrow_instruction_index);

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                // repay swaps supply_vault and user_liq_pk directions vs borrow
                accounts: flash_accounts(
                    user_wallet_pk,
                    lm_authority,
                    lm_pk,
                    reserve_pk,
                    token_mint_pk,
                    supply_vault_pk,
                    user_liq_pk,
                    fee_vault_pk,
                ),
                data,
            },
            KAMINO_FLASH_LOAN_CU,
        );
        Ok(())
    }

    // ─── Regular lending instruction builders ────────────────────────────────

    /// Append a `refresh_reserve` instruction to `wallet`. Must immediately
    /// precede (in the same slot/transaction) any deposit/borrow/repay/
    /// withdraw that touches this reserve or an obligation referencing it --
    /// see each instruction's own doc comment for exactly when full
    /// price-freshness (as opposed to just slot-freshness) is required.
    ///
    /// Pass this reserve's own `pyth_oracle`/`switchboard_price_oracle`/
    /// `switchboard_twap_oracle`/`scope_prices` fields (parsed off
    /// `config.token_info` -- see [`KaminoReserve`]'s fields), `None` for
    /// whichever ones this reserve doesn't use; klend accepts "None" via
    /// the program-id placeholder convention (see [`resolve_optional`]).
    /// All four currently-tracked reserves (SOL/BTC/ETH) use Scope only.
    pub fn refresh_reserve(
        &self,
        reserve_id: AccountId,
        pyth_oracle: Option<AccountId>,
        switchboard_price_oracle: Option<AccountId>,
        switchboard_twap_oracle: Option<AccountId>,
        scope_prices: Option<AccountId>,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let reserve_pk = resolve(reserve_id)?;
        let lm_pk = resolve(self.lending_market)?;

        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new_readonly(resolve_optional(pyth_oracle)?, false),
                    AccountMeta::new_readonly(resolve_optional(switchboard_price_oracle)?, false),
                    AccountMeta::new_readonly(resolve_optional(switchboard_twap_oracle)?, false),
                    AccountMeta::new_readonly(resolve_optional(scope_prices)?, false),
                ],
                data: DISC_REFRESH_RESERVE.to_vec(),
            },
            KAMINO_REFRESH_RESERVE_CU,
        );
        Ok(())
    }

    /// Append a `deposit_reserve_liquidity_and_obligation_collateral_v2`
    /// instruction to `wallet` -- deposits `liquidity_amount` of this
    /// reserve's asset and, in the same step, deposits the resulting cTokens
    /// as obligation collateral. `obligation` must already exist (see
    /// [`init_obligation`]). No farms are configured for this bot's
    /// reserves, so the V2 farm-tracking accounts are passed as "None".
    ///
    /// Requires this reserve and `obligation` to have been refreshed in the
    /// same slot (see [`Self::refresh_reserve`]/[`refresh_obligation`]) --
    /// deposit only needs slot-freshness, not full oracle-price freshness.
    pub fn deposit(
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
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let token_mint_pk = resolve(self.token_mint)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let collateral_mint_pk = resolve(self.collateral_mint)?;
        let collateral_supply_pk = resolve(self.collateral_supply_vault)?;
        let user_source_liquidity_pk = resolve(user_source_liquidity)?;
        let (obligation_farm_user_state, reserve_farm_state) =
            farm_accounts_metas(self.farm_collateral, obligation_pk)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_DEPOSIT_V2);
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(owner_pk, true),
                    AccountMeta::new(obligation_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new_readonly(lma_pk, false),
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new_readonly(token_mint_pk, false),
                    AccountMeta::new(supply_vault_pk, false),
                    AccountMeta::new(collateral_mint_pk, false),
                    AccountMeta::new(collateral_supply_pk, false),
                    AccountMeta::new(user_source_liquidity_pk, false),
                    AccountMeta::new_readonly(KAMINO_LENDING_PROGRAM_ID, false), // placeholder_user_destination_collateral: None
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // collateral_token_program
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // liquidity_token_program (Token-2022 mints unsupported)
                    AccountMeta::new_readonly(SYSVAR_INSTRUCTIONS_ID, false),
                    obligation_farm_user_state, // see `farm_accounts_metas`'s doc comment
                    reserve_farm_state,
                    AccountMeta::new_readonly(KAMINO_FARMS_PROGRAM_ID, false),
                ],
                data,
            },
            KAMINO_DEPOSIT_CU,
        );
        Ok(())
    }

    /// Append a `borrow_obligation_liquidity_v2` instruction to `wallet`.
    /// `deposit_reserves_for_elevation` should list the obligation's active
    /// deposit reserves (only actually consumed on-chain if the obligation
    /// is in an elevation group; harmless to include otherwise -- see
    /// klend's `update_elevation_group_debt_trackers_on_borrow`).
    ///
    /// Requires this reserve AND every deposit/borrow reserve on the
    /// obligation to have been refreshed in the same slot with full
    /// oracle-price freshness (`refresh_reserve` with real oracle accounts,
    /// then [`refresh_obligation`] listing all of them) -- borrowing (unlike
    /// deposit/repay) always needs `PriceStatusFlags::ALL_CHECKS`.
    pub fn borrow(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        liquidity_amount: u64,
        owner: AccountId,
        user_destination_liquidity: AccountId,
        referrer_token_state: Option<AccountId>,
        deposit_reserves_for_elevation: &[AccountId],
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let token_mint_pk = resolve(self.token_mint)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let fee_vault_pk = resolve(self.fee_vault)?;
        let dest_liq_pk = resolve(user_destination_liquidity)?;
        let (obligation_farm_user_state, reserve_farm_state) =
            farm_accounts_metas(self.farm_debt, obligation_pk)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_BORROW_V2);
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        let mut accounts = Vec::with_capacity(15 + deposit_reserves_for_elevation.len());
        accounts.extend_from_slice(&[
            AccountMeta::new_readonly(owner_pk, true),
            AccountMeta::new(obligation_pk, false),
            AccountMeta::new_readonly(lm_pk, false),
            AccountMeta::new_readonly(lma_pk, false),
            AccountMeta::new(reserve_pk, false),
            AccountMeta::new_readonly(token_mint_pk, false),
            AccountMeta::new(supply_vault_pk, false),
            AccountMeta::new(fee_vault_pk, false),
            AccountMeta::new(dest_liq_pk, false),
            AccountMeta::new_readonly(resolve_optional(referrer_token_state)?, false),
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSVAR_INSTRUCTIONS_ID, false),
            obligation_farm_user_state, // see `farm_accounts_metas`'s doc comment (debt-side farm here)
            reserve_farm_state,
            AccountMeta::new_readonly(KAMINO_FARMS_PROGRAM_ID, false),
        ]);
        for &id in deposit_reserves_for_elevation {
            accounts.push(AccountMeta::new(resolve(id)?, false));
        }

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts,
                data,
            },
            KAMINO_BORROW_CU,
        );
        Ok(())
    }

    /// Append a `repay_obligation_liquidity_v2` instruction to `wallet`.
    /// Pass [`KAMINO_AMOUNT_MAX`] to repay the obligation's full outstanding
    /// debt on this reserve exactly (not an overpay). Repay only needs
    /// slot-freshness (not full oracle-price freshness), but
    /// [`refresh_obligation`]'s remaining-accounts requirement (list every
    /// reserve on the obligation) still applies whenever it's called.
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
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let token_mint_pk = resolve(self.token_mint)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let user_source_liquidity_pk = resolve(user_source_liquidity)?;
        let (obligation_farm_user_state, reserve_farm_state) =
            farm_accounts_metas(self.farm_debt, obligation_pk)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_REPAY_V2);
        data.extend_from_slice(&liquidity_amount.to_le_bytes());

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new_readonly(owner_pk, true),
                    AccountMeta::new(obligation_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new_readonly(token_mint_pk, false),
                    AccountMeta::new(supply_vault_pk, false),
                    AccountMeta::new(user_source_liquidity_pk, false),
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
                    AccountMeta::new_readonly(SYSVAR_INSTRUCTIONS_ID, false),
                    obligation_farm_user_state, // see `farm_accounts_metas`'s doc comment (debt-side farm here)
                    reserve_farm_state,
                    AccountMeta::new_readonly(lma_pk, false),
                    AccountMeta::new_readonly(KAMINO_FARMS_PROGRAM_ID, false),
                ],
                data,
            },
            KAMINO_REPAY_CU,
        );
        Ok(())
    }

    /// Append a `withdraw_obligation_collateral_and_redeem_reserve_collateral_v2`
    /// instruction to `wallet`. `collateral_amount` is denominated in
    /// cTokens, not underlying liquidity; pass [`KAMINO_AMOUNT_MAX`] to
    /// withdraw this reserve's entire deposited amount. If this empties the
    /// obligation entirely (no deposits or borrows left), klend closes the
    /// obligation account and refunds rent to `owner`.
    ///
    /// Freshness requirement depends on the obligation's state: if it has no
    /// active borrows, only slot-freshness is required (like deposit/repay);
    /// if it has active borrows, full oracle-price freshness is required for
    /// every deposit/borrow reserve, same as [`Self::borrow`].
    pub fn withdraw(
        &self,
        reserve_id: AccountId,
        obligation: AccountId,
        collateral_amount: u64,
        owner: AccountId,
        user_destination_liquidity: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let owner_pk = resolve(owner)?;
        let obligation_pk = resolve(obligation)?;
        let lm_pk = resolve(self.lending_market)?;
        let lma_pk = lending_market_authority(&lm_pk);
        let reserve_pk = resolve(reserve_id)?;
        let token_mint_pk = resolve(self.token_mint)?;
        let supply_vault_pk = resolve(self.supply_vault)?;
        let collateral_mint_pk = resolve(self.collateral_mint)?;
        let collateral_supply_pk = resolve(self.collateral_supply_vault)?;
        let dest_liq_pk = resolve(user_destination_liquidity)?;
        let (obligation_farm_user_state, reserve_farm_state) =
            farm_accounts_metas(self.farm_collateral, obligation_pk)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_WITHDRAW_V2);
        data.extend_from_slice(&collateral_amount.to_le_bytes());

        wallet.require_signer(owner);
        wallet.append_ix(
            Instruction {
                program_id: KAMINO_LENDING_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(owner_pk, true),
                    AccountMeta::new(obligation_pk, false),
                    AccountMeta::new_readonly(lm_pk, false),
                    AccountMeta::new_readonly(lma_pk, false),
                    AccountMeta::new(reserve_pk, false),
                    AccountMeta::new_readonly(token_mint_pk, false),
                    AccountMeta::new(collateral_supply_pk, false),
                    AccountMeta::new(collateral_mint_pk, false),
                    AccountMeta::new(supply_vault_pk, false),
                    AccountMeta::new(dest_liq_pk, false),
                    AccountMeta::new_readonly(KAMINO_LENDING_PROGRAM_ID, false), // placeholder_user_destination_collateral: None
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // collateral_token_program
                    AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // liquidity_token_program (Token-2022 mints unsupported)
                    AccountMeta::new_readonly(SYSVAR_INSTRUCTIONS_ID, false),
                    obligation_farm_user_state, // see `farm_accounts_metas`'s doc comment
                    reserve_farm_state,
                    AccountMeta::new_readonly(KAMINO_FARMS_PROGRAM_ID, false),
                ],
                data,
            },
            KAMINO_WITHDRAW_CU,
        );
        Ok(())
    }

    fn resolve_accounts(
        &self,
        reserve_id: AccountId,
        user_wallet: AccountId,
        user_liquidity_account: AccountId,
    ) -> Result<(Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey), TraderError> {
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        Ok((
            resolve(reserve_id)?,
            resolve(self.lending_market)?,
            resolve(self.token_mint)?,
            resolve(self.supply_vault)?,
            resolve(user_liquidity_account)?,
            resolve(self.fee_vault)?,
            resolve(user_wallet)?,
        ))
    }
}

/// Build the 10-account list shared by both flash-borrow and flash-repay.
///
/// For borrow: `vault` = source, `user_liq` = destination.
/// For repay:  `vault` = destination, `user_liq` = source.
/// Both directions use the same account ordering; the instruction discriminator
/// tells the program which direction to move tokens.
fn flash_accounts(
    user_wallet: Pubkey,
    lm_authority: Pubkey,
    lending_market: Pubkey,
    reserve: Pubkey,
    reserve_liquidity_mint: Pubkey,
    reserve_vault: Pubkey,
    user_liquidity: Pubkey,
    fee_receiver: Pubkey,
) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new(user_wallet, true), // userTransferAuthority
        AccountMeta::new_readonly(lm_authority, false), // lendingMarketAuthority
        AccountMeta::new(lending_market, false), // lendingMarket
        AccountMeta::new(reserve, false),    // reserve
        AccountMeta::new_readonly(reserve_liquidity_mint, false), // reserveLiquidityMint
        AccountMeta::new(reserve_vault, false), // reserveSource/DestLiquidity
        AccountMeta::new(user_liquidity, false), // userDest/SourceLiquidity
        AccountMeta::new(fee_receiver, false), // reserveLiquidityFeeReceiver
        // referrerTokenState and referrerAccount omitted (no referrer)
        AccountMeta::new_readonly(SYSVAR_INSTRUCTIONS_ID, false), // sysvarInfo
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),   // tokenProgram
    ]
}

// ─── Regular lending: PDA derivation ───────────────────────────────────────────
// All seeds confirmed against klend's `utils/seeds.rs` and `utils/seeds.rs::pda`
// (github.com/Kamino-Finance/klend, programs/klend/src).

/// `["lma", lending_market]` -- the lending market's authority PDA, used as
/// the signing authority over every vault this program controls. Was
/// previously (mis)derived without the `"lma"` prefix seed in `flash_borrow`/
/// `flash_repay`, which would have failed on-chain (klend's own handlers
/// enforce `seeds = [LENDING_MARKET_AUTH, lending_market], bump =
/// lending_market.bump_seed`).
fn lending_market_authority(lending_market: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"lma", lending_market.as_ref()], &KAMINO_LENDING_PROGRAM_ID).0
}

/// `["user_meta", owner]` -- must exist (via [`init_user_metadata`]) before
/// [`init_obligation`] can succeed.
fn user_metadata_pda(owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"user_meta", owner.as_ref()], &KAMINO_LENDING_PROGRAM_ID).0
}

/// `[[tag], [id], owner, lending_market, seed1, seed2]` -- this bot only
/// creates standard, single-market-slot obligations (`tag = 0`, always
/// `seed1 == seed2 == Pubkey::default()`, required by klend's
/// `check_obligation_seeds`). Leveraged/multiply obligations (`tag` 1-3) are
/// out of scope. `id` is a free byte the owner picks -- verified directly
/// against klend's own `handler_init_obligation.rs` (`seeds = [&[args.tag],
/// &[args.id], obligation_owner, lending_market, seed1_account,
/// seed2_account]`): a different `id` with the same owner/market/tag
/// produces a genuinely independent obligation PDA, not a collision. Every
/// bot mode's own single obligation uses `id = 0`; a second, independent
/// obligation for the same wallet in the same market (e.g.
/// `leveragedloopv1`'s basis-trade obligation, isolated from its own
/// leverage-loop obligation) uses `id = 1`.
pub fn obligation_pda(owner: &Pubkey, lending_market: &Pubkey, id: u8) -> Pubkey {
    let default = Pubkey::default();
    Pubkey::find_program_address(
        &[
            &[0u8],
            &[id],
            owner.as_ref(),
            lending_market.as_ref(),
            default.as_ref(),
            default.as_ref(),
        ],
        &KAMINO_LENDING_PROGRAM_ID,
    )
    .0
}

/// `["user", farm, delegatee]` under [`KAMINO_FARMS_PROGRAM_ID`] (not
/// klend's own program) -- the real Kamino Farms "farmer" PDA klend calls
/// `obligation_farm`/`obligation_farm_user_state`. Live-verified against
/// `Kamino-Finance/kfarms`'s real `handler_initialize_user.rs`
/// (`BASE_SEED_USER_STATE = b"user"`, seeds =
/// `[BASE_SEED_USER_STATE, farm_state, delegatee]`) cross-checked against
/// klend's own `farms_ixs::cpi_initialize_farmer_delegated` (passes this
/// bot's *obligation* as `delegatee` -- klend always registers the
/// obligation, never the owner wallet directly, as the farm's delegated
/// user). `farm` is a reserve's `farm_collateral`/`farm_debt` field (see
/// [`KaminoReserve`]'s fields); `delegatee` is this bot's own obligation.
pub fn farms_user_state_pda(farm: &Pubkey, delegatee: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"user", farm.as_ref(), delegatee.as_ref()], &KAMINO_FARMS_PROGRAM_ID).0
}

/// [`farms_user_state_pda`], operating on `AccountId`s directly (this
/// bot's usual currency) instead of raw `Pubkey`s -- convenience wrapper
/// for callers (`perpfundingv1::state`) that don't otherwise need to
/// resolve pubkeys themselves. `None` if either id fails to resolve to a
/// real pubkey (shouldn't happen for tracked, live accounts).
pub fn farm_user_state_id(farm: AccountId, obligation: AccountId) -> Option<AccountId> {
    let farm_pk = pubkey_from_account_id(&farm)?;
    let obligation_pk = pubkey_from_account_id(&obligation)?;
    Some(account_id_from_pubkey(&farms_user_state_pda(&farm_pk, &obligation_pk)))
}

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

/// Anchor 0.29 optional-account convention: an absent `Option<AccountInfo>`
/// is represented on the wire by passing the *program's own* address rather
/// than shrinking the account list. Confirmed against klend's client SDK
/// (`libs/klend-interface/src/util.rs`).
fn resolve_optional(id: Option<AccountId>) -> Result<Pubkey, TraderError> {
    match id {
        Some(id) => resolve(id),
        None => Ok(KAMINO_LENDING_PROGRAM_ID),
    }
}

/// Real `farms_accounts` account pair (`obligation_farm_user_state`,
/// `reserve_farm_state`, in that order -- matches klend's
/// `OptionalObligationFarmsAccounts` field order exactly) for a
/// `deposit`/`withdraw` V2 instruction. `None` when `farm_collateral` is
/// unset (BTC/ETH today): both accounts fall back to the usual
/// program-id placeholder, read-only -- klend treats the farm step as a
/// no-op in that case. `Some`: both real accounts, **writable** (klend's
/// `OptionalObligationFarmsAccounts` marks both `#[account(mut)]`) --
/// confirmed via `simulateTransaction` that the read-only placeholder
/// fails with `Custom(6120) FarmAccountsMissing` once a farm is attached.
fn farm_accounts_metas(
    farm_collateral: Option<AccountId>,
    obligation_pk: Pubkey,
) -> Result<(AccountMeta, AccountMeta), TraderError> {
    match farm_collateral {
        Some(farm_id) => {
            let farm_pk = resolve(farm_id)?;
            let user_state_pk = farms_user_state_pda(&farm_pk, &obligation_pk);
            Ok((AccountMeta::new(user_state_pk, false), AccountMeta::new(farm_pk, false)))
        }
        None => Ok((
            AccountMeta::new_readonly(KAMINO_LENDING_PROGRAM_ID, false),
            AccountMeta::new_readonly(KAMINO_LENDING_PROGRAM_ID, false),
        )),
    }
}

// ─── Regular lending: position-account lifecycle ───────────────────────────────

/// Append an `init_user_metadata` instruction to `wallet`. Must succeed
/// before [`init_obligation`] will (klend requires `owner_user_metadata` to
/// already exist). `owner` acts as both the metadata's owner and the fee
/// payer -- this bot operates with a single signing wallet.
///
/// This bot never sets up a referrer or an address-lookup-table entry for
/// the metadata, so `referrer_user_metadata` is passed as "None" and
/// `user_lookup_table` is `Pubkey::default()`.
pub fn init_user_metadata(owner: AccountId, wallet: &mut Wallet) -> Result<(), TraderError> {
    let owner_pk = resolve(owner)?;
    let user_metadata = user_metadata_pda(&owner_pk);

    let mut data = Vec::with_capacity(40);
    data.extend_from_slice(&DISC_INIT_USER_METADATA);
    data.extend_from_slice(&Pubkey::default().to_bytes());

    wallet.require_signer(owner);
    wallet.append_ix(
        Instruction {
            program_id: KAMINO_LENDING_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new_readonly(owner_pk, true), // owner
                AccountMeta::new(owner_pk, true),          // fee_payer (same signer)
                AccountMeta::new(user_metadata, false),
                AccountMeta::new_readonly(KAMINO_LENDING_PROGRAM_ID, false), // referrer_user_metadata: None
                AccountMeta::new_readonly(rent::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        },
        KAMINO_INIT_USER_METADATA_CU,
    );
    Ok(())
}

/// Append an `init_obligation` instruction to `wallet`, creating this
/// owner's standard (`tag = 0`) obligation for `lending_market` at the
/// given `id` (see [`obligation_pda`]'s doc comment -- `0` for a bot
/// mode's own single/primary obligation, a different value for a second,
/// independent obligation in the same market). Returns the obligation's
/// `AccountId` so callers can chain it into deposit/borrow/repay/withdraw
/// without a second lookup -- the address is deterministic
/// ([`obligation_pda`]), so it never needs to be remembered separately
/// from `(owner, lending_market, id)`.
///
/// Requires [`init_user_metadata`] to have already succeeded for `owner`
/// (shared, per-owner, not per-obligation -- do not call it again just
/// because `id` differs).
pub fn init_obligation(
    owner: AccountId,
    lending_market: AccountId,
    id: u8,
    wallet: &mut Wallet,
) -> Result<AccountId, TraderError> {
    let owner_pk = resolve(owner)?;
    let lm_pk = resolve(lending_market)?;
    let obligation = obligation_pda(&owner_pk, &lm_pk, id);
    let user_metadata = user_metadata_pda(&owner_pk);
    let default = Pubkey::default();

    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&DISC_INIT_OBLIGATION);
    data.push(0u8); // tag
    data.push(id);

    wallet.require_signer(owner);
    wallet.append_ix(
        Instruction {
            program_id: KAMINO_LENDING_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new_readonly(owner_pk, true), // obligation_owner
                AccountMeta::new(owner_pk, true),          // fee_payer (same signer)
                AccountMeta::new(obligation, false),
                AccountMeta::new_readonly(lm_pk, false),
                AccountMeta::new_readonly(default, false), // seed1_account (tag 0 => default)
                AccountMeta::new_readonly(default, false), // seed2_account (tag 0 => default)
                AccountMeta::new_readonly(user_metadata, false),
                AccountMeta::new_readonly(rent::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        },
        KAMINO_INIT_OBLIGATION_CU,
    );
    Ok(account_id_from_pubkey(&obligation))
}

/// Append an `init_obligation_farms_for_reserve` instruction to `wallet`,
/// creating the Kamino Farms "farmer" PDA ([`farms_user_state_pda`]) this
/// obligation needs before any `deposit`/`withdraw` (`mode = 0`,
/// `ReserveFarmKind::Collateral`) or `borrow`/`repay` (`mode = 1`,
/// `ReserveFarmKind::Debt`) against a reserve with a real
/// `farm_collateral`/`farm_debt` attached -- see [`KaminoReserve`]'s
/// fields. Real account order/constraints live-verified against klend's
/// `handler_init_obligation_farms_for_reserve.rs`. Anchor's `init`
/// constraint on the farmer account means this fails if called a second
/// time for the same `(reserve, obligation)` pair -- callers must gate on
/// whether it already succeeded (e.g. a real `on_account` update for the
/// derived address), same discipline as [`init_obligation`] itself.
pub fn init_obligation_farms_for_reserve(
    owner: AccountId,
    obligation: AccountId,
    lending_market: AccountId,
    reserve_id: AccountId,
    farm: AccountId,
    mode: u8,
    wallet: &mut Wallet,
) -> Result<AccountId, TraderError> {
    let owner_pk = resolve(owner)?;
    let obligation_pk = resolve(obligation)?;
    let lm_pk = resolve(lending_market)?;
    let lma_pk = lending_market_authority(&lm_pk);
    let reserve_pk = resolve(reserve_id)?;
    let farm_pk = resolve(farm)?;
    let obligation_farm = farms_user_state_pda(&farm_pk, &obligation_pk);

    let mut data = Vec::with_capacity(9);
    data.extend_from_slice(&DISC_INIT_OBLIGATION_FARMS_FOR_RESERVE);
    data.push(mode);

    wallet.require_signer(owner);
    wallet.append_ix(
        Instruction {
            program_id: KAMINO_LENDING_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(owner_pk, true),          // payer
                AccountMeta::new_readonly(owner_pk, true), // owner (checked against obligation.owner)
                AccountMeta::new(obligation_pk, false),
                AccountMeta::new_readonly(lma_pk, false),
                AccountMeta::new(reserve_pk, false),
                AccountMeta::new(farm_pk, false), // reserve_farm_state
                AccountMeta::new(obligation_farm, false),
                AccountMeta::new_readonly(lm_pk, false),
                AccountMeta::new_readonly(KAMINO_FARMS_PROGRAM_ID, false),
                AccountMeta::new_readonly(rent::ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        },
        KAMINO_INIT_OBLIGATION_FARMS_FOR_RESERVE_CU,
    );
    Ok(account_id_from_pubkey(&obligation_farm))
}

/// Append a `refresh_obligation` instruction to `wallet`. `deposit_reserves`
/// must list every reserve the obligation currently has a collateral
/// deposit in, in order, followed by every reserve it has a debt in
/// (`borrow_reserves`) -- klend rejects the instruction if the count
/// doesn't match the obligation's actual deposit/borrow arrays exactly. This
/// bot doesn't track a user's obligation contents, so the caller must supply
/// these lists (see the credit-graph module's non-goals: no automatic
/// position discovery). Assumes the obligation has no referrer (this bot
/// never sets one up via [`init_user_metadata`]).
pub fn refresh_obligation(
    lending_market: AccountId,
    obligation: AccountId,
    deposit_reserves: &[AccountId],
    borrow_reserves: &[AccountId],
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let lm_pk = resolve(lending_market)?;
    let obligation_pk = resolve(obligation)?;

    let mut accounts = Vec::with_capacity(2 + deposit_reserves.len() + borrow_reserves.len());
    accounts.push(AccountMeta::new_readonly(lm_pk, false));
    accounts.push(AccountMeta::new(obligation_pk, false));
    for &id in deposit_reserves.iter().chain(borrow_reserves.iter()) {
        accounts.push(AccountMeta::new(resolve(id)?, false));
    }

    let cu = KAMINO_REFRESH_OBLIGATION_BASE_CU
        + (deposit_reserves.len() + borrow_reserves.len()) as u32
            * KAMINO_REFRESH_OBLIGATION_PER_RESERVE_CU;

    wallet.append_ix(
        Instruction {
            program_id: KAMINO_LENDING_PROGRAM_ID,
            accounts,
            data: DISC_REFRESH_OBLIGATION.to_vec(),
        },
        cu,
    );
    Ok(())
}

// ─── Obligation account layout (fixed-slot, mirrors `solend::Obligation`'s
// approach) -- live-verified against a real Kamino Obligation account.
// Unlike Solend, there is no explicit deposits_len/borrows_len byte: an
// unused slot's `deposit_reserve`/`borrow_reserve` simply decodes as the
// default (all-zero) pubkey, confirmed by directly inspecting live account
// bytes past the obligation's real position count. ─────────────────────────
const OFF_OB_DEPOSITS: usize = 96;
const OBLIGATION_DEPOSITS_COUNT: usize = 8;
const OBLIGATION_COLLATERAL_LEN: usize = 136;
const OFF_OC_DEPOSIT_RESERVE: usize = 0;
const OFF_OC_DEPOSITED_AMOUNT: usize = 32;

/// Only 5 slots, not 8 -- real, confirmed, genuinely differs from Solend's
/// `borrows` array (which has 8, matching its `deposits`).
const OFF_OB_BORROWS: usize = 1208;
const OBLIGATION_BORROWS_COUNT: usize = 5;
const OBLIGATION_LIQUIDITY_LEN: usize = 200;
const OFF_OL_BORROW_RESERVE: usize = 0;
const OFF_OL_BORROWED_AMOUNT_SF: usize = 88;

const MIN_OBLIGATION_LEN: usize =
    OFF_OB_BORROWS + OBLIGATION_BORROWS_COUNT * OBLIGATION_LIQUIDITY_LEN;

/// One entry from a Kamino Obligation's `deposits` array -- collateral
/// backing the obligation in a given reserve.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KaminoCollateral {
    pub deposit_reserve: AccountId,
    /// cToken units, NOT raw underlying -- unlike Solend's
    /// `ObligationCollateral::deposited_amount`, converting this to an
    /// underlying amount needs the reserve's live collateral exchange
    /// rate. [`KaminoReserve::withdraw`] accepts [`KAMINO_AMOUNT_MAX`] for
    /// a full withdrawal, which sidesteps needing that conversion for this
    /// bot's own close-leg flow.
    pub deposited_amount: u64,
}

/// One entry from a Kamino Obligation's `borrows` array -- an outstanding
/// loan against a given reserve.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KaminoLiquidity {
    pub borrow_reserve: AccountId,
    /// Raw native token units -- SF-scaled (`/2^60`) in the source
    /// account, already converted here, same convention as
    /// `solend::ObligationLiquidity::borrowed_amount` (WAD-scaled there
    /// instead).
    pub borrowed_amount: u64,
}

/// Parsed subset of a Kamino Obligation account -- this bot's own lending
/// position, not a reserve. Mirrors `solend::SolendObligation`'s shape
/// (fixed-slot struct, find-by-key accessors), but with Kamino's real,
/// different array sizes: 8 deposit slots, only 5 borrow slots.
#[derive(Debug, Default, Clone)]
pub struct KaminoObligation {
    pub deposits: Vec<KaminoCollateral>,
    pub borrows: Vec<KaminoLiquidity>,
}

impl KaminoObligation {
    /// The deposit (if any) in `reserve`.
    pub fn deposit_for(&self, reserve: AccountId) -> Option<&KaminoCollateral> {
        self.deposits.iter().find(|d| d.deposit_reserve == reserve)
    }

    /// The borrow (if any) against `reserve`.
    pub fn borrow_for(&self, reserve: AccountId) -> Option<&KaminoLiquidity> {
        self.borrows.iter().find(|b| b.borrow_reserve == reserve)
    }
}

/// Parse a Kamino Obligation account from raw body bytes. Live-verified,
/// fixed-slot layout: `deposits` is a fixed 8-slot array at
/// [`OFF_OB_DEPOSITS`] (stride [`OBLIGATION_COLLATERAL_LEN`]), `borrows` a
/// fixed 5-slot array at [`OFF_OB_BORROWS`] (stride
/// [`OBLIGATION_LIQUIDITY_LEN`], 5 slots not 8 -- genuinely differs from
/// Solend). There is no explicit length byte; an unused slot's
/// `deposit_reserve`/`borrow_reserve` decodes as the default (all-zero)
/// pubkey and is filtered out here.
pub fn parse_kamino_obligation(body: &[u8]) -> Option<KaminoObligation> {
    if body.len() < MIN_OBLIGATION_LEN {
        return None;
    }
    let is_default_pk = |off: usize| body[off..off + 32].iter().all(|&b| b == 0);
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_sf_u64 = |off: usize| -> u64 {
        (u128::from_le_bytes(body[off..off + 16].try_into().unwrap()) as f64 / SF_SCALE) as u64
    };

    let mut deposits = Vec::with_capacity(OBLIGATION_DEPOSITS_COUNT);
    for i in 0..OBLIGATION_DEPOSITS_COUNT {
        let base = OFF_OB_DEPOSITS + i * OBLIGATION_COLLATERAL_LEN;
        if is_default_pk(base + OFF_OC_DEPOSIT_RESERVE) {
            continue;
        }
        deposits.push(KaminoCollateral {
            deposit_reserve: read_pk(base + OFF_OC_DEPOSIT_RESERVE),
            deposited_amount: read_u64(base + OFF_OC_DEPOSITED_AMOUNT),
        });
    }

    let mut borrows = Vec::with_capacity(OBLIGATION_BORROWS_COUNT);
    for i in 0..OBLIGATION_BORROWS_COUNT {
        let base = OFF_OB_BORROWS + i * OBLIGATION_LIQUIDITY_LEN;
        if is_default_pk(base + OFF_OL_BORROW_RESERVE) {
            continue;
        }
        borrows.push(KaminoLiquidity {
            borrow_reserve: read_pk(base + OFF_OL_BORROW_RESERVE),
            borrowed_amount: read_sf_u64(base + OFF_OL_BORROWED_AMOUNT_SF),
        });
    }

    Some(KaminoObligation { deposits, borrows })
}

/// This bot's own Kamino lending position -- tracks whether *this bot's*
/// obligation is initialized and its current contents. Mirrors
/// `solend::SolendPosition`'s exact shape (deposit/borrow/withdraw/repay
/// instructions already take `owner`/`obligation` as plain `AccountId`
/// params, so they don't need an authority-bearing `self` -- this struct
/// exists only to know "is my obligation initialized, and what does it
/// currently hold"), scoped to one account instead of a market list.
/// Reserve pricing/instruction-building keep coming from the separate,
/// shared, read-only `KaminoState` (e.g. via `DexState::kamino()`).
#[derive(Debug, Default)]
pub struct KaminoPosition {
    o_authority_pk: Option<Pubkey>,
    o_obligation_id: Option<AccountId>,
    o_obligation: Option<KaminoObligation>,
    /// This owner's `user_metadata` PDA -- unlike the obligation, this is
    /// per-*owner*, not per-obligation: it survives an obligation being
    /// closed (e.g. a full withdrawal that empties it), so it must be
    /// tracked separately rather than assumed to follow `registered()`.
    /// Real gap this closes: `init_user_metadata` uses Anchor's `init`
    /// constraint, which fails if called a second time -- re-bootstrapping
    /// after an obligation closes must skip it, not blindly retry it.
    o_user_metadata_id: Option<AccountId>,
    user_metadata_registered: bool,
    subscriptions: Vec<Subscription>,
    /// Kamino Farms "farmer" PDAs ([`farms_user_state_pda`]) this bot has
    /// asked to track, keyed by their own `AccountId` -- `true` once a
    /// real `on_account` update confirms the account exists (i.e.
    /// [`init_obligation_farms_for_reserve`] already succeeded for it).
    /// Populated on demand by [`Self::track_farm_user_state`] -- unlike
    /// the obligation itself, which reserve needs a farm isn't known at
    /// `set_authority` time (that's a property of `KaminoReserve`, not
    /// this struct).
    m_farm_user_state_seen: HashMap<AccountId, bool>,
}

impl KaminoPosition {
    /// Pure-derivation half of the old single-shot `set_authority`
    /// (removed -- only ever called from `perpfundingv1`/`testperpv1`'s
    /// `Wallet` message handler, alongside Phoenix/Solend/marginfi's own
    /// subscription calls). Returns the two subscription requests this
    /// authority needs -- this bot's own Kamino obligation
    /// ([`obligation_pda`] against [`KAMINO_MAIN_MARKET`], this bot's
    /// only market, at the given obligation `id` -- see [`obligation_pda`]'s
    /// doc comment) and its `user_metadata` PDA ([`user_metadata_pda`])
    /// -- empty if already set, without making the host `subscribe` call
    /// itself. Paired with [`Self::apply_authority`] so every venue's
    /// requests can be batched into one `bulk_subscribe` round-trip
    /// instead of five separate ones. Real, live-observed incident: those
    /// five one-at-a-time calls accounted for ~26 seconds of stall in one
    /// run (traced via `CommitHook::start`'s own timing diagnostics).
    pub fn authority_subscribe_requests(&self, authority: Pubkey, id: u8) -> Vec<SubscriptionRequest> {
        if self.o_authority_pk == Some(authority) {
            return Vec::new();
        }
        let obligation_pk = obligation_pda(&authority, &KAMINO_MAIN_MARKET, id);
        let user_metadata_pk = user_metadata_pda(&authority);
        vec![
            SubscriptionRequest { root: account_id_from_pubkey(&obligation_pk), filter_weight: 0, depth: 1 },
            SubscriptionRequest { root: account_id_from_pubkey(&user_metadata_pk), filter_weight: 0, depth: 1 },
        ]
    }

    /// Apply `authority` plus its already-resolved subscriptions (from
    /// [`Self::authority_subscribe_requests`], in the same order: `[0]`
    /// obligation, `[1]` user_metadata) -- the second half of the split
    /// described there. `id` must match whatever was passed to
    /// `authority_subscribe_requests` for this same call. No-op if `subs`
    /// is empty (either already set, or nothing to apply).
    pub fn apply_authority(&mut self, authority: Pubkey, id: u8, subs: Vec<Subscription>) {
        if subs.is_empty() {
            return;
        }
        assert_eq!(subs.len(), 2, "authority_subscribe_requests always returns exactly 2 requests");
        let obligation_pk = obligation_pda(&authority, &KAMINO_MAIN_MARKET, id);
        let obligation_id = account_id_from_pubkey(&obligation_pk);
        let user_metadata_pk = user_metadata_pda(&authority);
        let user_metadata_id = account_id_from_pubkey(&user_metadata_pk);
        self.subscriptions.extend(subs);
        self.o_authority_pk = Some(authority);
        self.o_obligation_id = Some(obligation_id);
        self.o_user_metadata_id = Some(user_metadata_id);
    }

    pub fn obligation_id(&self) -> Option<AccountId> {
        self.o_obligation_id
    }

    /// `true` only once a real update for the obligation account has been
    /// parsed -- means the account actually exists on-chain
    /// (`init_user_metadata` + `init_obligation` already succeeded), not
    /// just "we know its address and subscribed." Same reasoning as
    /// `solend::SolendPosition::registered`.
    pub fn registered(&self) -> bool {
        self.o_obligation.is_some()
    }

    /// Call right after queuing a withdraw that empties this obligation's
    /// last deposit (no borrows outstanding) -- Kamino's real
    /// `WithdrawObligationCollateral` closes the underlying obligation
    /// account once it's fully empty (rent refunded), but unlike account
    /// *creation* (which reliably produces a fresh `on_account` push, see
    /// `on_account`'s doc comment), this bot's account subscription never
    /// observed a follow-up push reflecting that *closure* -- real,
    /// live-confirmed: `getAccountInfo` on the tracked obligation address
    /// returned `null` (and `getSignaturesForAddress` showed zero
    /// successful transactions since) immediately after a full withdrawal,
    /// yet `registered()` stayed stuck `true` and every later borrow
    /// attempt in the same run kept sending real `RefreshObligation`
    /// instructions against the now-nonexistent account, failing on-chain
    /// with Anchor's `AccountOwnedByWrongProgram` (custom program error
    /// 0xbbf). Reactive fixes in `on_account` alone can't catch this
    /// (nothing ever arrives to react to) -- the caller has to proactively
    /// tell this position its obligation is about to close so
    /// `registered()` correctly reports `false` again and the next
    /// `open_*_borrow_leg` call re-bootstraps instead of reusing a dead
    /// account. `o_obligation_id` (the PDA address itself) is left alone
    /// -- re-bootstrapping reuses the same deterministic address.
    pub fn mark_obligation_closing(&mut self) {
        self.o_obligation = None;
    }

    /// `true` only once a real update for the `user_metadata` account has
    /// arrived -- see the field doc comment on `o_user_metadata_id` for
    /// why this can be `true` even when [`Self::registered`] is `false`
    /// (a closed-then-reopened obligation).
    pub fn user_metadata_registered(&self) -> bool {
        self.user_metadata_registered
    }

    /// Last successfully parsed obligation, if any real update has
    /// arrived yet.
    pub fn obligation(&self) -> Option<&KaminoObligation> {
        self.o_obligation.as_ref()
    }

    /// Ensure `farm_user_state_id` (a [`farms_user_state_pda`]-derived
    /// address) is subscribed to, so a real `on_account` update can
    /// confirm whether [`init_obligation_farms_for_reserve`] has already
    /// succeeded for it. Idempotent -- a repeat call for an
    /// already-tracked id is a no-op.
    pub fn track_farm_user_state(&mut self, farm_user_state_id: AccountId, g: &Graph) -> Result<(), CatscopeGuestError> {
        if self.m_farm_user_state_seen.contains_key(&farm_user_state_id) {
            return Ok(());
        }
        let sub = g.subscribe(SubscriptionRequest { root: farm_user_state_id, filter_weight: 0, depth: 1 })?;
        self.subscriptions.push(sub);
        self.m_farm_user_state_seen.insert(farm_user_state_id, false);
        Ok(())
    }

    /// `true` only once a real account update has confirmed
    /// `farm_user_state_id` exists on-chain -- same "push-based, only
    /// real accounts produce updates" reasoning as [`Self::registered`].
    /// `false` if it isn't tracked yet (see [`Self::track_farm_user_state`]).
    pub fn farm_user_state_registered(&self, farm_user_state_id: AccountId) -> bool {
        self.m_farm_user_state_seen.get(&farm_user_state_id).copied().unwrap_or(false)
    }

    pub fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if Some(header.accountid) == self.o_obligation_id {
            // Unlike the "not-yet-created" case (where a real Kamino
            // obligation simply hasn't been bootstrapped yet, and no
            // update has ever set `o_obligation`), a *previously
            // registered* obligation can become un-parseable again if
            // Kamino's real withdraw instruction closes the account once
            // it's fully emptied (rent refunded, account deleted) --
            // real, live-confirmed: `open_kamino_borrow_leg` used a stale
            // `registered()==true` from an obligation created earlier in
            // the same run, skipped re-bootstrapping, and sent a real
            // `RefreshObligation` against an account `getAccountInfo`
            // confirms no longer exists on mainnet, failing on-chain with
            // Anchor's `AccountOwnedByWrongProgram` (custom program error
            // 0xbbf) -- the same error code the `user_metadata` fix below
            // guards against, but for the opposite direction (never
            // un-registering once registered, instead of registering too
            // early).
            //
            // `parse_kamino_obligation` alone isn't enough to detect this:
            // it only checks `body.len()`, not ownership, and a closed
            // account can retain the *same* byte length with its data
            // simply zeroed out -- which parses "successfully" as an
            // all-default-pubkeys obligation (empty deposits/borrows)
            // rather than failing, so the stale `Some(..)` never got
            // reset even after adding the reset-on-parse-failure logic
            // above (real, live-confirmed: the exact same 0xbbf recurred
            // after that fix alone). Require real Kamino-owned data here
            // too, matching the `user_metadata` check's rigor below.
            let kamino_owned = header.owner == account_id_from_pubkey(&KAMINO_LENDING_PROGRAM_ID);
            self.o_obligation = if kamino_owned { parse_kamino_obligation(body) } else { None };
        }
        // A subscription to an account that doesn't exist on-chain yet
        // still produces exactly one `on_account` push here (real,
        // live-verified: `owner` = System Program, empty `body`) --
        // contrary to this file's usual "no update ever arrives for a
        // nonexistent account" assumption (safe for the *parsed*-obligation
        // check above, since an empty/wrong-owner body fails to parse by
        // construction). Trusting arrival alone previously flagged
        // `user_metadata_registered = true` for a still-uninitialized
        // `user_metadata` PDA, which skipped `init_user_metadata` and sent
        // a real `init_obligation` that failed on-chain with Anchor's
        // AccountOwnedByWrongProgram (custom program error 0xbbf) --
        // confirmed via `solana confirm` on the failed signature. Require
        // real Kamino-owned data, matching the parsed-obligation check's
        // effective rigor.
        if Some(header.accountid) == self.o_user_metadata_id
            && header.owner == account_id_from_pubkey(&KAMINO_LENDING_PROGRAM_ID)
            && !body.is_empty()
        {
            self.user_metadata_registered = true;
        }
        // Farm-user-state accounts are owned by KAMINO_FARMS_PROGRAM_ID
        // (Kamino Farms), not KAMINO_LENDING_PROGRAM_ID (Kamino Lending)
        // -- confirmed via `solana confirm` on both the successful
        // `InitializeUser` create (invoked by `FarmsPZpWu9i7...`) and a
        // second, failed attempt ("already in use", custom program error
        // 0x0) caused by comparing against the wrong program id here and
        // never flipping this flag true despite the account genuinely
        // existing after the first attempt.
        if let Some(seen) = self.m_farm_user_state_seen.get_mut(&header.accountid) {
            if header.owner == account_id_from_pubkey(&KAMINO_FARMS_PROGRAM_ID) && !body.is_empty() {
                *seen = true;
            }
        }
    }
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a Kamino Lending reserve account from raw body bytes.
pub fn parse(body: &[u8]) -> Option<KaminoReserve> {
    if body.len() < MIN_RESERVE_LEN {
        return None;
    }

    let read_u32 = |off: usize| u32::from_le_bytes(body[off..off + 4].try_into().unwrap());
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
    let read_pk = |off: usize| -> AccountId {
        account_id_from_pubkey(&Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ))
    };
    // Absence is signaled by the null pubkey (all-zero bytes) -- confirmed
    // convention (this session's oracle-account research).
    let read_optional_pk = |off: usize| -> Option<AccountId> {
        if body[off..off + 32].iter().all(|&b| b == 0) {
            None
        } else {
            Some(read_pk(off))
        }
    };
    let mut borrow_rate_curve = [(0u32, 0u32); BORROW_RATE_CURVE_POINTS];
    for (i, point) in borrow_rate_curve.iter_mut().enumerate() {
        let base = OFF_BORROW_RATE_CURVE + i * 8;
        *point = (read_u32(base), read_u32(base + 4));
    }

    Some(KaminoReserve {
        lending_market: read_pk(OFF_LENDING_MARKET),
        token_mint: read_pk(OFF_MINT),
        supply_vault: read_pk(OFF_SUPPLY_VAULT),
        fee_vault: read_pk(OFF_FEE_VAULT),
        collateral_mint: read_pk(OFF_COLLATERAL_MINT),
        collateral_supply_vault: read_pk(OFF_COLLATERAL_SUPPLY_VAULT),
        available_amount: read_u64(OFF_AVAILABLE_AMOUNT),
        borrowed_amount: read_u128(OFF_BORROWED_AMOUNT_SF) as f64 / SF_SCALE,
        price_usd: read_u128(OFF_MARKET_PRICE_SF) as f64 / SF_SCALE,
        mint_decimals: read_u64(OFF_MINT_DECIMALS),
        flash_loan_fee_fraction: read_u64(OFF_FLASH_LOAN_FEE_SF) as f64 / SF_SCALE,
        loan_to_value_pct: body[OFF_LOAN_TO_VALUE_PCT] as f64 / 100.0,
        borrow_factor_pct: read_u64(OFF_BORROW_FACTOR_PCT) as f64 / 100.0,
        pyth_oracle: read_optional_pk(OFF_PYTH_ORACLE),
        switchboard_price_oracle: read_optional_pk(OFF_SWITCHBOARD_PRICE_ORACLE),
        switchboard_twap_oracle: read_optional_pk(OFF_SWITCHBOARD_TWAP_ORACLE),
        scope_prices: read_optional_pk(OFF_SCOPE_PRICES),
        protocol_take_rate_pct: body[OFF_PROTOCOL_TAKE_RATE_PCT],
        borrow_rate_curve,
        status: body[OFF_RESERVE_STATUS],
        farm_collateral: read_optional_pk(OFF_FARM_COLLATERAL),
        farm_debt: read_optional_pk(OFF_FARM_DEBT),
    })
}

// ─── Live state tracker ───────────────────────────────────────────────────────

/// Tracks all configured Kamino Lending reserves.
pub struct KaminoState {
    program_id: AccountId,
    m_reserve: HashMap<AccountId, KaminioReserveWrapper, BuildHasherDefault<XxHash64>>,
    m_lending_market: HashMap<AccountId, LendingMarket, BuildHasherDefault<XxHash64>>,
    /// map vault account id to reserve
    m_vault: HashMap<AccountId, AccountId, BuildHasherDefault<XxHash64>>,
}

#[derive(Debug, Default)]
pub struct LendingMarket {}

impl std::fmt::Debug for KaminoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KaminoState")
            .field("parsed_count", &self.m_reserve.len())
            .finish()
    }
}

impl KaminoState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let setup = Setup::from_config();
        let setups = &setup.l_setup;
        let program_id = account_id_from_pubkey(&KAMINO_LENDING_PROGRAM_ID);
        let mut m_reserve =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let mut m_vault =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let mut m_lending_market =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(setups.len());
        for setup in setups {
            if m_lending_market
                .insert(setup.lending_market, LendingMarket::default())
                .is_none()
            {
                // unique lending market
                l_req.push(SubscriptionRequest {
                    root: setup.lending_market,
                    filter_weight: 0,
                    depth: 1,
                });
            }
            let mut r = KaminoReserve::default();
            r.supply_vault = setup.supply_vault;
            r.fee_vault = setup.fee_vault;
            if m_reserve
                .insert(
                    setup.reserve,
                    KaminioReserveWrapper {
                        reserve: r,
                        coin_vault_balance: 0,
                        fee_vault_balance: 0,
                    },
                )
                .is_none()
            {
                // unique reserve
                l_req.push(SubscriptionRequest {
                    root: setup.reserve,
                    filter_weight: 0,
                    depth: 1,
                });
                l_req.push(SubscriptionRequest {
                    root: setup.supply_vault,
                    filter_weight: 0,
                    depth: 1,
                });
                m_vault.insert(setup.supply_vault, setup.reserve);
                l_req.push(SubscriptionRequest {
                    root: setup.fee_vault,
                    filter_weight: 0,
                    depth: 1,
                });
                m_vault.insert(setup.fee_vault, setup.reserve);
            }
        }
        (Self { program_id, m_reserve, m_lending_market, m_vault }, l_req)
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn reserve_count(&self) -> usize {
        self.m_reserve.len()
    }

    /// Find the reserve backing `mint` (e.g. real SOL's mint -> the SOL
    /// reserve). Linear scan is fine -- `m_reserve` only ever holds a
    /// handful of entries, same reasoning as `SolendState::reserve_by_mint`.
    /// `None` if `mint` isn't tracked, or tracked but no account update has
    /// arrived yet.
    /// Find the `Active`, main-market reserve backing `mint`. Kamino's
    /// prefetch coverage spans reserves across every isolated market, not
    /// just [`KAMINO_MAIN_MARKET`] (e.g. real, live-verified: SOL has 37
    /// tracked reserves total, only one on the main market; USDC has 78,
    /// with 4 on the main market alone -- 3 `Hidden`, only 1 `Active`), so
    /// both filters are required, not just a plain mint match -- matching
    /// any other market's reserve, or a `Hidden`/`Obsolete` one, would
    /// mismatch against this bot's own main-market obligation on-chain.
    /// `None` if `mint` isn't tracked on the main market, or is but no
    /// account update for it has arrived yet.
    pub fn reserve_by_mint(&self, mint: AccountId) -> Option<(AccountId, &KaminoReserve)> {
        let main_market = account_id_from_pubkey(&KAMINO_MAIN_MARKET);
        self.m_reserve
            .iter()
            .find(|(_, w)| w.reserve.token_mint == mint && w.reserve.lending_market == main_market && w.reserve.is_active())
            .map(|(id, w)| (*id, &w.reserve))
    }

    /// Direct reserve lookup by its own account id -- needed to refresh an
    /// obligation's *existing* deposit/borrow reserves (from
    /// `KaminoObligation::deposits`/`borrows`, real on-chain state) before
    /// `refresh_obligation`, which klend requires for every reserve
    /// currently in the obligation, not just the one a given instruction
    /// directly touches (real, live-confirmed `ReserveStale` revert
    /// otherwise -- an obligation can carry an unrelated pre-existing
    /// deposit/borrow from before this bot ever touched it).
    pub fn reserve_by_id(&self, reserve_id: AccountId) -> Option<&KaminoReserve> {
        self.m_reserve.get(&reserve_id).map(|w| &w.reserve)
    }
}
impl Updater for KaminoState {
    fn on_account(&mut self, header: &crate::catscope::witbot::shooter::Header, body: &[u8]) {
        if self.program_id != header.owner {
            return;
        }
        // LendingMarket itself has no fields worth tracking today (see its
        // definition above) -- reserves are the only account type with
        // live numeric state to refresh.
        if let Some(wrapper) = self.m_reserve.get_mut(&header.accountid) {
            if let Some(parsed) = parse(body) {
                wrapper.reserve = parsed;
            }
        }
    }

    fn on_token(&mut self, ta: &crate::catscope::witbot::shooter::Tokenaccountv1) -> bool {
        let reserve_id = match self.m_vault.get(&ta.id) {
            Some(x) => *x,
            None => return false,
        };
        let wrapper = self.m_reserve.get_mut(&reserve_id).unwrap();
        if wrapper.reserve.supply_vault == ta.id {
            wrapper.coin_vault_balance = ta.amount;
        } else if wrapper.reserve.fee_vault == ta.id {
            wrapper.fee_vault_balance = ta.amount;
        }
        true
    }

    fn batch_router(&mut self, _router: &mut crate::trader::pricegraph::TradeRouter) {
        // A lending Reserve has no swap price to contribute to
        // TradeRouter -- same reasoning as MarginfiState::batch_router
        // (see credit.rs's module doc comment for why lending isn't
        // unified with the AMM price graph). Genuine no-op, not
        // unimplemented -- DexState::batch_router calls every registered
        // dex unconditionally, so a `todo!()` here would panic.
    }

    fn on_tx(
        &mut self,
        _ix: &crate::txview::CatscopeInstructionRead<'_>,
        _slot: &solana_sdk::clock::Slot,
    ) {
    }

    fn flush_pool(&mut self, _graph: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        // Not currently called by DexState::flush_pool (only Orca is), and
        // Kamino has no per-slot pool state that needs flushing. Genuine
        // no-op for trait-completeness.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    // PDA-derivation and discriminator-byte tests only -- these are pure,
    // host-independent computation (`Pubkey::find_program_address` and
    // `sha2::Sha256` don't touch the WASM host). The instruction-builder
    // methods themselves call `pubkey_from_account_id`/`account_id_from_pubkey`
    // (WASM host imports) and can't be unit-tested under native `cargo test`
    // for the same reason Kamino's pre-existing flash-loan builders aren't --
    // see the project's established testing convention.

    fn disc(name: &str) -> [u8; 8] {
        let hash = Sha256::digest(format!("global:{name}").as_bytes());
        hash[..8].try_into().unwrap()
    }

    #[test]
    fn discriminators_match_their_instruction_names() {
        assert_eq!(disc("init_user_metadata"), DISC_INIT_USER_METADATA);
        assert_eq!(disc("init_obligation"), DISC_INIT_OBLIGATION);
        assert_eq!(disc("refresh_reserve"), DISC_REFRESH_RESERVE);
        assert_eq!(disc("refresh_obligation"), DISC_REFRESH_OBLIGATION);
        assert_eq!(
            disc("deposit_reserve_liquidity_and_obligation_collateral_v2"),
            DISC_DEPOSIT_V2
        );
        assert_eq!(disc("borrow_obligation_liquidity_v2"), DISC_BORROW_V2);
        assert_eq!(disc("repay_obligation_liquidity_v2"), DISC_REPAY_V2);
        assert_eq!(
            disc("withdraw_obligation_collateral_and_redeem_reserve_collateral_v2"),
            DISC_WITHDRAW_V2
        );
        assert_eq!(
            disc("init_obligation_farms_for_reserve"),
            DISC_INIT_OBLIGATION_FARMS_FOR_RESERVE
        );
    }

    #[test]
    fn farms_user_state_pda_is_deterministic_and_farm_obligation_specific() {
        let farm_a = Pubkey::new_unique();
        let farm_b = Pubkey::new_unique();
        let obligation_a = Pubkey::new_unique();
        let obligation_b = Pubkey::new_unique();
        assert_eq!(
            farms_user_state_pda(&farm_a, &obligation_a),
            farms_user_state_pda(&farm_a, &obligation_a)
        );
        assert_ne!(
            farms_user_state_pda(&farm_a, &obligation_a),
            farms_user_state_pda(&farm_b, &obligation_a)
        );
        assert_ne!(
            farms_user_state_pda(&farm_a, &obligation_a),
            farms_user_state_pda(&farm_a, &obligation_b)
        );
    }

    #[test]
    fn lending_market_authority_uses_the_lma_prefix_seed() {
        let lending_market = Pubkey::new_unique();
        let expected =
            Pubkey::find_program_address(&[b"lma", lending_market.as_ref()], &KAMINO_LENDING_PROGRAM_ID).0;
        assert_eq!(lending_market_authority(&lending_market), expected);
        // Regression check for the bug this session found and fixed: the
        // seed must NOT be just `[lending_market]`.
        let wrong =
            Pubkey::find_program_address(&[lending_market.as_ref()], &KAMINO_LENDING_PROGRAM_ID).0;
        assert_ne!(lending_market_authority(&lending_market), wrong);
    }

    #[test]
    fn obligation_pda_is_deterministic_and_owner_specific() {
        let lending_market = Pubkey::new_unique();
        let owner_a = Pubkey::new_unique();
        let owner_b = Pubkey::new_unique();
        assert_eq!(
            obligation_pda(&owner_a, &lending_market, 0),
            obligation_pda(&owner_a, &lending_market, 0)
        );
        assert_ne!(
            obligation_pda(&owner_a, &lending_market, 0),
            obligation_pda(&owner_b, &lending_market, 0)
        );
    }

    #[test]
    fn obligation_pda_is_independent_per_id() {
        // Verified against klend's own handler_init_obligation.rs: `id` is
        // part of the PDA seed, so the same owner/market/tag with a
        // different id is a genuinely independent obligation, not a
        // collision -- what leveragedloopv1's basis-trade obligation
        // (id=1) relies on to stay isolated from its own leverage-loop
        // obligation (id=0).
        let lending_market = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        assert_ne!(
            obligation_pda(&owner, &lending_market, 0),
            obligation_pda(&owner, &lending_market, 1)
        );
    }

    #[test]
    fn user_metadata_pda_is_deterministic_and_owner_specific() {
        let owner_a = Pubkey::new_unique();
        let owner_b = Pubkey::new_unique();
        assert_eq!(user_metadata_pda(&owner_a), user_metadata_pda(&owner_a));
        assert_ne!(user_metadata_pda(&owner_a), user_metadata_pda(&owner_b));
    }

    // `parse_kamino_obligation` itself isn't unit-tested here -- it calls
    // `account_id_from_pubkey` internally (a WIT host import that aborts
    // outside the real WASM guest runtime), same established boundary as
    // `solend::parse_obligation`. `KaminoObligation::deposit_for`/
    // `borrow_for` have no such dependency and are fully testable directly.
    fn obligation_with_positions() -> KaminoObligation {
        KaminoObligation {
            deposits: vec![KaminoCollateral { deposit_reserve: 7, deposited_amount: 5_000_000_000 }],
            borrows: vec![KaminoLiquidity { borrow_reserve: 9, borrowed_amount: 1_200_000_000 }],
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

    #[test]
    fn is_active_true_only_for_status_zero() {
        // Real, live-verified: KAMINO_MAIN_MARKET's USDC reserves are
        // status=0 (Active, exactly one) or status=2 (Hidden, three).
        let mut r = KaminoReserve { status: 0, ..Default::default() };
        assert!(r.is_active());
        r.status = 1; // Obsolete
        assert!(!r.is_active());
        r.status = 2; // Hidden
        assert!(!r.is_active());
    }
}
