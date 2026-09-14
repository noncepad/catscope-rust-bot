//! Marinade Finance instant liquid-unstake trader.
//!
//! # Program
//!
//! Marinade (`MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD`) redeems mSOL for
//! SOL immediately through its own liquidity pool (the "liq_pool"), for a
//! fee that scales linearly with how much SOL-side liquidity remains after
//! the withdrawal. This is a completely different program/pool from
//! Sanctum's S Controller -- `sanctum.rs`'s Marinade references are only a
//! pricing CPI target, not a real instruction.
//!
//! # Account layout
//!
//! `State` (`8szGkuLTAux9XMgZ2vtY39jVSowEcpBfFfD8hXSEqdGC`) is a standard
//! Anchor account (8-byte discriminator), Borsh serialized. Every field we
//! need is at a **fixed** byte offset -- `StakeSystem`/`ValidatorSystem`
//! (which precede `liq_pool` in the struct) each hold only a
//! `List{account, item_size, count, ..}` reference to a *separate* account,
//! not an inline `Vec`, so nothing before `msol_price` has a variable size.
//! Verified against a live fetch of the real `State` account: decoded
//! `msol_price` matched Sanctum's independently-tracked mSOL/SOL ratio to
//! within ~0.6%, and `liq_pool.msol_leg`/the derived sol-leg PDA both
//! resolved to real, live accounts holding real mSOL/SOL balances.
//!
//! ```text
//! @104  treasury_msol_account       Pubkey (32 bytes)
//! @138  rent_exempt_for_token_acc   u64
//! @420  liq_pool.msol_leg           Pubkey (32 bytes)
//! @452  liq_pool.lp_liquidity_target u64
//! @460  liq_pool.lp_max_fee         u32 (basis points)
//! @464  liq_pool.lp_min_fee         u32 (basis points)
//! @512  msol_price                  u64 (divide by 2^32 for SOL-per-mSOL)
//! ```
//!
//! # Fee formula (`LiqPool::linear_fee`, exact)
//!
//! ```text
//! available = sol_leg_lamports - rent_exempt_for_token_acc
//! after     = available - user_remove_lamports
//! fee_bps = if after >= lp_liquidity_target { lp_min_fee }
//!           else { lp_max_fee - (lp_max_fee - lp_min_fee) * after / lp_liquidity_target }
//! ```
//! The graph edge uses the marginal fee at `user_remove_lamports = 0`
//! (`after = available`) -- a point price, same convention every other edge
//! in this codebase uses; amount-dependent slippage is left to the existing
//! generic `cp_quote` treatment downstream (see `reserve_in`'s doc comment
//! on [`MarinadeState::batch_router`]).
//!
//! # `liquid_unstake` instruction layout
//!
//! Anchor `#[derive(Accounts)]` order: state, msol_mint, liq_pool_sol_leg_pda,
//! liq_pool_msol_leg, treasury_msol_account, get_msol_from,
//! get_msol_from_authority (signer), transfer_sol_to, system_program,
//! token_program. Data: 8-byte Anchor sighash (`DISC_LIQUID_UNSTAKE`) then
//! `msol_amount: u64`. No on-chain min-out param on this instruction --
//! callers must check `min_amount_out` themselves before calling
//! [`MarinadeState::swap`].
//!
//! # Delayed unstake (`order_unstake` + `claim`)
//!
//! The other redemption path: burn mSOL now for a `Ticket` account, wait
//! for it to mature, then claim SOL later -- no dynamic liquidity-curve
//! fee (unlike `liquid_unstake`), just a flat `delayed_unstake_fee`
//! (currently the protocol max, 0.2%, live-verified against the real
//! `State` account: `delayed_unstake_fee.bp_cents == 2000`, matching
//! source's own `MAX_DELAYED_UNSTAKE_FEE` constant exactly).
//!
//! Maturity (`instructions/delayed_unstake/claim.rs`):
//! ```text
//! claimable when: clock.epoch >= ticket.created_epoch + 1
//!   AND (clock.epoch > ticket.created_epoch + 1
//!        OR clock.unix_timestamp - clock.epoch_start_timestamp >= 1800)
//! ```
//! Also, operationally: `Claim` additionally requires Marinade's own crank
//! bot to have already moved enough SOL into the reserve
//! (`reserve_pda.lamports() - rent_exempt_for_token_acc >=
//! ticket.lamports_amount`) -- "epoch-mature" doesn't strictly guarantee
//! "claimable this instant".
//!
//! `created_epoch = clock.epoch + (1 if clock.epoch ==
//! stake_system.last_stake_delta_epoch else 0)` -- set by `order_unstake`
//! at ticket-creation time.
//!
//! `new_ticket_account` is **not** a PDA (Anchor's `#[account(zero)]`, a
//! plain pre-allocated account) and this bot has no ephemeral-keypair
//! infrastructure (`Wallet::append_key` is for the bot's own pre-loaded
//! signing keys) nor verified WASM-guest secure randomness for a fresh
//! `Keypair`. Same problem `solend.rs`'s Obligation account has (also not
//! a PDA) -- same fix: `Pubkey::create_with_seed` +
//! `system_instruction::create_account_with_seed` +
//! `Rent::default().minimum_balance(..)`, deterministic, no ephemeral key
//! needed. Unlike Solend's one-fixed-seed-per-owner Obligation, Marinade
//! tickets are one-shot and can have many concurrent, so the seed passed
//! to [`MarinadeState::order_unstake`] must vary per call (e.g. include
//! the current slot) to avoid colliding with a still-pending ticket.
//!
//! `TicketAccountData` (`state/delayed_unstake_ticket.rs`):
//! `state_address(32) + beneficiary(32) + lamports_amount(8) +
//! created_epoch(8)` = 80 bytes + 8-byte Anchor discriminator = 88 bytes
//! total (`TICKET_ACCOUNT_DATA_LEN`).
//!
//! `order_unstake` accounts (Anchor order): state, msol_mint,
//! burn_msol_from, burn_msol_authority (signer), new_ticket_account,
//! clock sysvar, rent sysvar, token_program. Data: 8-byte sighash
//! (`DISC_ORDER_UNSTAKE`) + `msol_amount: u64`.
//!
//! `claim` accounts: state, reserve_pda (PDA: seeds=[state,
//! `State::RESERVE_SEED`=b"reserve"], same derivation shape as
//! `sol_leg_pda`), ticket_account, transfer_sol_to (must equal
//! `ticket.beneficiary`), clock sysvar, system_program. Data: just the
//! 8-byte sighash (`DISC_CLAIM`), no args.
//!
//! The authoritative SOL-per-mSOL rate used for `order_unstake`'s payout
//! (`msol_to_sol`, via `total_virtual_staked_lamports / msol_supply`) was
//! checked directly against the cached `msol_price` already used for the
//! live liquid-unstake edge: they agreed to 9 decimal places on a live
//! snapshot, so [`MarinadeState::compare_unstake_paths`]'s delayed leg
//! reuses the same cached `msol_price_raw` rather than computing the
//! fuller formula -- not worth the extra fields for a sub-1e-8 difference.
//! Its instant leg, by contrast, doesn't use this pool's own price/fee at
//! all -- it queries the shared `TradeRouter` for the best route from
//! mSOL to SOL across every registered venue (Sanctum, Raydium, Orca, and
//! this pool's own `liquid_unstake` edge, all on equal footing), since
//! this pool's own reserve isn't necessarily the best place to redeem
//! instantly.

use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
};
use solana_system_interface::instruction::create_account_with_seed;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    log_warn,
    trader::{
        dex::update::Updater,
        pricegraph::{Hop, TradeRouter},
        types::{DexType, TraderError},
    },
    txview::CatscopeInstructionRead,
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};

pub const MARINADE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD");

/// Marinade's single, fixed `State` account -- same address `sanctum.rs`
/// hardcodes as Marinade's SOL-value-calculator `pool_state` (that's a
/// coincidence of Marinade using the same account for both purposes, not a
/// shared abstraction between the two modules).
const MARINADE_STATE: Pubkey = Pubkey::from_str_const("8szGkuLTAux9XMgZ2vtY39jVSowEcpBfFfD8hXSEqdGC");

const MSOL_MINT: Pubkey = Pubkey::from_str_const("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So");

const WSOL_MINT: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

const SYSVAR_CLOCK_ID: Pubkey = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");
const SYSVAR_RENT_ID: Pubkey = Pubkey::from_str_const("SysvarRent111111111111111111111111111111111");

// ─── State account layout (see module doc) ────────────────────────────────────
const STATE_MIN_LEN: usize = 613; // through delayed_unstake_fee.bp_cents @609 (4 bytes)
const OFF_TREASURY_MSOL_ACCOUNT: usize = 104;
const OFF_RENT_EXEMPT_FOR_TOKEN_ACC: usize = 138;
const OFF_LAST_STAKE_DELTA_EPOCH: usize = 244;
const OFF_MSOL_LEG: usize = 420;
const OFF_LP_LIQUIDITY_TARGET: usize = 452;
const OFF_LP_MAX_FEE_BPS: usize = 460;
const OFF_LP_MIN_FEE_BPS: usize = 464;
const OFF_MSOL_PRICE: usize = 512;
const OFF_DELAYED_UNSTAKE_FEE_BP_CENTS: usize = 609;
/// `State::PRICE_DENOMINATOR` (2^32).
const PRICE_DENOMINATOR: f64 = 4_294_967_296.0;
/// `FeeCents::MAX_BP_CENTS` -- `delayed_unstake_fee`'s denominator (note:
/// *not* 10,000 like `spl_stake_pool.rs`'s `Fee`/`liq_pool`'s basis-point
/// fields -- Marinade's `FeeCents` is a finer, "basis-point-cents" unit).
/// `total_active_balance`/`available_reserve_balance`/
/// `circulating_ticket_balance`/`*_cooling_down` (needed for the fuller
/// `total_virtual_staked_lamports`-based rate) are deliberately not read
/// here -- see module doc's note on why the cached `msol_price` is reused
/// for both paths instead.
const FEE_CENTS_DENOMINATOR: u64 = 1_000_000;
/// `Claim`'s `WAIT_EPOCHS` (`instructions/delayed_unstake/claim.rs`).
const UNSTAKE_WAIT_EPOCHS: u64 = 1;
/// Mainnet `DEFAULT_SLOTS_PER_EPOCH` -- used only to estimate the current
/// epoch from `Header.slot` for the diagnostic comparison below, since
/// this bot doesn't track the `Clock` sysvar anywhere. Approximate (skips
/// any warmup-epoch schedule, irrelevant this far past mainnet genesis).
const SLOTS_PER_EPOCH: u64 = 432_000;
const TICKET_ACCOUNT_DATA_LEN: u64 = 88;

/// sha256("global:liquid_unstake")[..8], cross-checked in this module's
/// tests below (same convention as `marginfi.rs`/`kamino.rs`).
const DISC_LIQUID_UNSTAKE: [u8; 8] = [30, 30, 119, 240, 191, 227, 12, 16];
/// sha256("global:order_unstake")[..8], cross-checked below.
const DISC_ORDER_UNSTAKE: [u8; 8] = [97, 167, 144, 107, 117, 190, 128, 36];
/// sha256("global:claim")[..8], cross-checked below.
const DISC_CLAIM: [u8; 8] = [62, 198, 214, 193, 213, 159, 108, 210];

pub const MARINADE_LIQUID_UNSTAKE_CU: u32 = 60_000;
pub const MARINADE_ORDER_UNSTAKE_CU: u32 = 60_000;
pub const MARINADE_CLAIM_CU: u32 = 40_000;
pub const MARINADE_CREATE_TICKET_ACCOUNT_CU: u32 = 5_000;

#[derive(Debug)]
pub struct MarinadeState {
    program_id: AccountId,
    state_id: AccountId,
    sol_leg_id: AccountId,
    sol_leg_pda: Pubkey,
    mint_id: AccountId,
    /// `WSOL_MINT`'s `AccountId`, resolved once in `new()` -- reused by
    /// `batch_router`/`compare_unstake_paths` instead of re-resolving it
    /// on every call, since `account_id_from_pubkey` is a host import with
    /// no implementation outside the real wasm32-wasip2 guest runtime
    /// (calling it from a native unit test aborts the process).
    sol_mint_id: AccountId,

    /// Parsed from `State` -- zero (default `Pubkey`) until the first
    /// account update arrives.
    msol_leg_pk: Pubkey,
    treasury_msol_account: Pubkey,
    rent_exempt_for_token_acc: u64,
    lp_liquidity_target: u64,
    lp_max_fee_bps: u32,
    lp_min_fee_bps: u32,
    /// Raw `msol_price` -- divide by [`PRICE_DENOMINATOR`] for SOL-per-mSOL.
    msol_price_raw: u64,
    /// Live lamport balance of `sol_leg_pda`, from `header.lamports` (a
    /// native System-owned account, not SPL -- no `on_token` needed).
    sol_leg_lamports: u64,
    /// `stake_system.last_stake_delta_epoch` -- needed to replicate
    /// `order_unstake`'s own `created_epoch` epoch-adjustment rule.
    last_stake_delta_epoch: u64,
    /// `delayed_unstake_fee.bp_cents` -- divide by [`FEE_CENTS_DENOMINATOR`].
    delayed_unstake_fee_bp_cents: u32,

    /// TEMP diagnostic: log the first delivery of each subscribed account
    /// exactly once, to check whether on_account is firing at all for
    /// these two IDs -- see the same pattern used to debug Sanctum's
    /// reserve-tracking earlier this session.
    state_logged: bool,
    sol_leg_logged: bool,
}

impl MarinadeState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let state_id = account_id_from_pubkey(&MARINADE_STATE);
        let (sol_leg_pda, _bump) =
            Pubkey::find_program_address(&[MARINADE_STATE.as_ref(), b"liq_sol"], &MARINADE_PROGRAM_ID);
        let sol_leg_id = account_id_from_pubkey(&sol_leg_pda);
        let mint_id = account_id_from_pubkey(&MSOL_MINT);

        let l_req = vec![
            SubscriptionRequest { root: state_id, filter_weight: 0, depth: 1 },
            SubscriptionRequest { root: sol_leg_id, filter_weight: 0, depth: 1 },
        ];

        let state = Self {
            program_id: account_id_from_pubkey(&MARINADE_PROGRAM_ID),
            state_id,
            sol_leg_id,
            sol_leg_pda,
            mint_id,
            sol_mint_id: account_id_from_pubkey(&WSOL_MINT),
            msol_leg_pk: Pubkey::default(),
            treasury_msol_account: Pubkey::default(),
            rent_exempt_for_token_acc: 0,
            lp_liquidity_target: 0,
            lp_max_fee_bps: 0,
            lp_min_fee_bps: 0,
            msol_price_raw: 0,
            sol_leg_lamports: 0,
            last_stake_delta_epoch: 0,
            delayed_unstake_fee_bp_cents: 0,
            state_logged: false,
            sol_leg_logged: false,
        };
        (state, l_req)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// `1` once live pricing data has arrived, `0` otherwise -- for the
    /// periodic "pool stats" log (mirrors `SanctumState::lst_count`).
    pub fn is_ready(&self) -> usize {
        (self.msol_price_raw != 0 && self.sol_leg_lamports != 0) as usize
    }

    fn parse_state(&mut self, data: &[u8]) {
        if data.len() < STATE_MIN_LEN {
            return;
        }
        let u64_at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let u32_at = |off: usize| u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        let pubkey_at = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());

        self.treasury_msol_account = pubkey_at(OFF_TREASURY_MSOL_ACCOUNT);
        self.rent_exempt_for_token_acc = u64_at(OFF_RENT_EXEMPT_FOR_TOKEN_ACC);
        self.msol_leg_pk = pubkey_at(OFF_MSOL_LEG);
        self.lp_liquidity_target = u64_at(OFF_LP_LIQUIDITY_TARGET);
        self.lp_max_fee_bps = u32_at(OFF_LP_MAX_FEE_BPS);
        self.lp_min_fee_bps = u32_at(OFF_LP_MIN_FEE_BPS);
        self.msol_price_raw = u64_at(OFF_MSOL_PRICE);
        self.last_stake_delta_epoch = u64_at(OFF_LAST_STAKE_DELTA_EPOCH);
        self.delayed_unstake_fee_bp_cents = u32_at(OFF_DELAYED_UNSTAKE_FEE_BP_CENTS);
    }

    /// `LiqPool::linear_fee`, evaluated at `user_remove_lamports = 0` (the
    /// marginal/best-case fee) -- see module doc for the exact formula.
    fn marginal_fee_bps(&self, available: u64) -> u32 {
        if self.lp_liquidity_target == 0 {
            return self.lp_min_fee_bps;
        }
        if available >= self.lp_liquidity_target {
            return self.lp_min_fee_bps;
        }
        let delta = self.lp_max_fee_bps.saturating_sub(self.lp_min_fee_bps);
        let reduction =
            (delta as u128 * available as u128 / self.lp_liquidity_target as u128) as u32;
        self.lp_max_fee_bps.saturating_sub(reduction)
    }

    /// Re-derive and upsert the single mSOL->SOL edge. `price` is
    /// deliberately computed even when `msol_price_raw`/`sol_leg_lamports`
    /// is `0` (yielding `0.0`, or a saturated `reserve_in` -- both numerically
    /// safe, Rust float-to-int casts saturate rather than panic) rather than
    /// skipped, so `add_directed_edge`'s own `price_out_per_in <= 0.0` gate
    /// removes any stale edge left over from when the pool was last valid.
    fn upsert_edge(&self, router: &mut TradeRouter) {
        let price = self.msol_price_raw as f64 / PRICE_DENOMINATOR;
        let available = self.sol_leg_lamports.saturating_sub(self.rent_exempt_for_token_acc);
        let fee_bps = self.marginal_fee_bps(available);
        let fee_frac = fee_bps as f64 / 10_000.0;
        // msol_leg's real balance isn't tracked live (not needed for pricing
        // -- see module doc), so reserve_in is a notional mSOL-equivalent of
        // the real SOL-side reserve, just to give the existing generic
        // cp_quote-based amount-dependent quoting a sane order-of-magnitude
        // shape (same simplification this codebase already applies to every
        // non-AMM edge, e.g. Sanctum's).
        let reserve_in = (available as f64 / price) as u64;
        router.add_directed_edge(
            self.sol_leg_id,
            self.mint_id,
            self.sol_mint_id,
            price,
            fee_frac,
            reserve_in,
            available,
            DexType::MarinadeLiquidUnstake,
        );
    }

    /// Build a `liquid_unstake` instruction and append it to `wallet`.
    pub fn swap(
        &self,
        amount_in: u64,
        user_wallet: AccountId,
        user_msol_account: AccountId,
        user_sol_destination: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        if self.msol_leg_pk == Pubkey::default() {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_msol_pk = resolve(user_msol_account)?;
        let user_sol_pk = resolve(user_sol_destination)?;

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_LIQUID_UNSTAKE);
        data.extend_from_slice(&amount_in.to_le_bytes());

        let accounts = vec![
            AccountMeta::new(MARINADE_STATE, false),           // state
            AccountMeta::new_readonly(MSOL_MINT, false),       // msol_mint
            AccountMeta::new(self.sol_leg_pda, false),         // liq_pool_sol_leg_pda
            AccountMeta::new(self.msol_leg_pk, false),         // liq_pool_msol_leg
            AccountMeta::new(self.treasury_msol_account, false), // treasury_msol_account
            AccountMeta::new(user_msol_pk, false),              // get_msol_from
            AccountMeta::new_readonly(user_wallet_pk, true),    // get_msol_from_authority (signer)
            AccountMeta::new(user_sol_pk, false),               // transfer_sol_to
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false), // system_program
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // token_program
        ];

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            Instruction { program_id: MARINADE_PROGRAM_ID, accounts, data },
            MARINADE_LIQUID_UNSTAKE_CU,
        );
        Ok(())
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    /// One-directional (mSOL -> SOL only), so `hop.input_mint`/`output_mint`
    /// aren't consulted here beyond what `DexState::execute_hop` already
    /// implied by matching `DexType::MarinadeLiquidUnstake`.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        self.swap(hop.amount_in, owner, source_ata, dest_ata, wallet)
    }

    /// The deterministic, `create_account_with_seed`-derived address of a
    /// delayed-unstake ticket for `owner` under `seed` -- see module doc
    /// for why this (not a PDA, not a fresh keypair) is the right tool
    /// here. `seed` must vary per call (e.g. include the current slot) to
    /// avoid colliding with a still-pending ticket's address; max 32 bytes
    /// (`Pubkey::create_with_seed`'s own limit).
    pub fn order_unstake_ticket_address(owner: &Pubkey, seed: &str) -> Pubkey {
        Pubkey::create_with_seed(owner, seed, &MARINADE_PROGRAM_ID)
            .expect("seed must be <= 32 bytes")
    }

    /// Append a `create_account_with_seed` instruction (allocating the new
    /// ticket account) followed by `order_unstake` (burn mSOL, initialize
    /// the ticket) to `wallet`. Returns the new ticket account's
    /// `AccountId` -- the caller is responsible for remembering it if it
    /// wants to `claim` later (this module holds no position state, see
    /// module doc's scope note).
    pub fn order_unstake(
        &self,
        amount_in: u64,
        seed: &str,
        user_wallet: AccountId,
        user_msol_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<AccountId, TraderError> {
        if self.msol_leg_pk == Pubkey::default() {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_msol_pk = resolve(user_msol_account)?;
        let ticket_pk = Self::order_unstake_ticket_address(&user_wallet_pk, seed);

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            create_account_with_seed(
                &user_wallet_pk,
                &ticket_pk,
                &user_wallet_pk,
                seed,
                Rent::default().minimum_balance(TICKET_ACCOUNT_DATA_LEN as usize),
                TICKET_ACCOUNT_DATA_LEN,
                &MARINADE_PROGRAM_ID,
            ),
            MARINADE_CREATE_TICKET_ACCOUNT_CU,
        );

        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_ORDER_UNSTAKE);
        data.extend_from_slice(&amount_in.to_le_bytes());

        let accounts = vec![
            AccountMeta::new(MARINADE_STATE, false),            // state
            AccountMeta::new(MSOL_MINT, false),                 // msol_mint
            AccountMeta::new(user_msol_pk, false),               // burn_msol_from
            AccountMeta::new_readonly(user_wallet_pk, true),     // burn_msol_authority (signer)
            AccountMeta::new(ticket_pk, false),                  // new_ticket_account
            AccountMeta::new_readonly(SYSVAR_CLOCK_ID, false),   // clock
            AccountMeta::new_readonly(SYSVAR_RENT_ID, false),    // rent
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // token_program
        ];

        wallet.append_ix(
            Instruction { program_id: MARINADE_PROGRAM_ID, accounts, data },
            MARINADE_ORDER_UNSTAKE_CU,
        );
        Ok(account_id_from_pubkey(&ticket_pk))
    }

    /// Build a `claim` instruction and append it to `wallet` -- redeems a
    /// matured ticket (from [`Self::order_unstake`]) for SOL.
    /// `user_wallet` must be the ticket's original beneficiary (`Claim`
    /// checks `ticket_account.beneficiary == transfer_sol_to` on-chain).
    /// Callers are responsible for having checked maturity themselves (see
    /// module doc's claimable condition) -- this builder doesn't verify it.
    ///
    /// Deliberately no `wallet.require_signer` call here -- `Claim`'s real
    /// Anchor accounts struct has no `Signer<'info>` at all (verified
    /// against source): the SOL can only ever go to the ticket's stored
    /// `beneficiary` (enforced on-chain via an `address = ..` constraint),
    /// so claiming is permissionless by design, not an oversight here.
    pub fn claim(
        &self,
        ticket_account: AccountId,
        user_wallet: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let ticket_pk = resolve(ticket_account)?;
        let user_wallet_pk = resolve(user_wallet)?;
        let (reserve_pda, _bump) =
            Pubkey::find_program_address(&[MARINADE_STATE.as_ref(), b"reserve"], &MARINADE_PROGRAM_ID);

        let accounts = vec![
            AccountMeta::new(MARINADE_STATE, false), // state
            AccountMeta::new(reserve_pda, false),    // reserve_pda
            AccountMeta::new(ticket_pk, false),      // ticket_account
            AccountMeta::new(user_wallet_pk, false), // transfer_sol_to
            AccountMeta::new_readonly(SYSVAR_CLOCK_ID, false), // clock
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false), // system_program
        ];

        wallet.append_ix(
            Instruction { program_id: MARINADE_PROGRAM_ID, accounts, data: DISC_CLAIM.to_vec() },
            MARINADE_CLAIM_CU,
        );
        Ok(())
    }

    /// Best-available instant redemption (via `router` -- any registered
    /// venue, not just this pool's own `liquid_unstake`) vs. delayed
    /// native-unstake, for `amount_in` raw mSOL -- informational only (see
    /// module doc's scope note: no position tracking here, nothing calls
    /// [`Self::order_unstake`]/[`Self::claim`] for real).
    ///
    /// `router` should be the same live `TradeRouter` `batch_router`
    /// populates -- Marinade's own `liquid_unstake` edge is registered on
    /// it same as everything else, so `router.route` already considers it
    /// alongside Sanctum/Raydium/Orca; this doesn't duplicate
    /// `marginal_fee_bps`'s pool-specific fee math (that's still used by
    /// `batch_router` for the edge itself, just not here anymore).
    ///
    /// `current_slot` is the caller's current `Header.slot` -- this bot
    /// doesn't track the `Clock` sysvar anywhere, so the epoch is
    /// approximated via [`SLOTS_PER_EPOCH`] (see its doc comment).
    ///
    /// `None` when either leg isn't ready: no live State data yet (same
    /// gate as [`Updater::batch_router`]), or `router` has no route from
    /// mSOL to SOL yet (e.g. graph still warming up).
    pub fn compare_unstake_paths(
        &self,
        amount_in: u64,
        current_slot: u64,
        router: &TradeRouter,
    ) -> Option<UnstakeComparison> {
        if self.msol_price_raw == 0 {
            return None;
        }
        let route = router.route(self.mint_id, self.sol_mint_id, amount_in, 3)?;
        let instant_out = route.amount_out();
        let instant_route: Vec<DexType> = route.hops.iter().map(|h| h.dex).collect();

        // Delayed: flat delayed_unstake_fee (FeeCents, /1_000_000 not
        // /10_000 -- see FEE_CENTS_DENOMINATOR's doc comment), against the
        // cached msol_price (see module doc's precision note on why the
        // fuller total_virtual_staked_lamports formula isn't used here).
        let price = self.msol_price_raw as f64 / PRICE_DENOMINATOR;
        let delayed_fee_msol =
            (amount_in as u128 * self.delayed_unstake_fee_bp_cents as u128 / FEE_CENTS_DENOMINATOR as u128) as u64;
        let delayed_out = (amount_in.saturating_sub(delayed_fee_msol) as f64 * price) as u64;

        // order_unstake's own created_epoch rule, then Claim's WAIT_EPOCHS.
        let current_epoch = current_slot / SLOTS_PER_EPOCH;
        let created_epoch =
            current_epoch + (current_epoch == self.last_stake_delta_epoch) as u64;
        let claimable_epoch = created_epoch + UNSTAKE_WAIT_EPOCHS;
        let wait_epochs = claimable_epoch.saturating_sub(current_epoch);

        Some(UnstakeComparison { instant_out, instant_route, delayed_out, wait_epochs })
    }
}

/// See [`MarinadeState::compare_unstake_paths`].
#[derive(Debug, Clone)]
pub struct UnstakeComparison {
    /// Best estimated SOL out right now, across every venue `router` knows
    /// about (not just this pool's own `liquid_unstake`).
    pub instant_out: u64,
    /// The full chain of DEXes the best instant route swaps through, in
    /// order (e.g. `[Sanctum, OrcaWhirlpool]` for a 2-hop mSOL -> LST ->
    /// SOL path) -- `route()` allows up to 3 hops, so this can be more
    /// than the entry venue alone.
    pub instant_route: Vec<DexType>,
    /// Estimated SOL out via [`MarinadeState::order_unstake`] +
    /// [`MarinadeState::claim`], once mature.
    pub delayed_out: u64,
    /// Estimated epochs until a ticket ordered now would be claimable.
    pub wait_epochs: u64,
}

impl Updater for MarinadeState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        // sol_leg_pda is System-owned, not Marinade-owned -- match by
        // accountid directly rather than gating on header.owner first (see
        // module doc; this differs from sanctum.rs's owner-first check).
        if header.accountid == self.state_id {
            if !self.state_logged {
                self.state_logged = true;
                log_warn!(
                    "marinade: first State account delivery: len={} owner={}",
                    body.len(),
                    header.owner,
                );
            }
            self.parse_state(body);
        } else if header.accountid == self.sol_leg_id {
            if !self.sol_leg_logged {
                self.sol_leg_logged = true;
                log_warn!(
                    "marinade: first sol_leg delivery: lamports={} owner={}",
                    header.lamports,
                    header.owner,
                );
            }
            self.sol_leg_lamports = header.lamports;
        }
    }

    fn on_token(&mut self, _ta: &Tokenaccountv1) -> bool {
        false
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &solana_sdk::clock::Slot) {}

    /// Add the mSOL → SOL liquid-unstake edge for arbitrage detection.
    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.upsert_edge(router);
    }

    /// `state_id` and `sol_leg_id` both feed the single mSOL->SOL edge, so
    /// either one changing re-derives it -- there's only ever one edge to
    /// touch, no lookup needed.
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if account_id == self.state_id || account_id == self.sol_leg_id {
            self.upsert_edge(router);
        }
    }

    fn flush_pool(&mut self, _g: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn disc(name: &str) -> [u8; 8] {
        let hash = Sha256::digest(format!("global:{name}").as_bytes());
        hash[..8].try_into().unwrap()
    }

    #[test]
    fn discriminator_matches_instruction_name() {
        assert_eq!(disc("liquid_unstake"), DISC_LIQUID_UNSTAKE);
        assert_eq!(disc("order_unstake"), DISC_ORDER_UNSTAKE);
        assert_eq!(disc("claim"), DISC_CLAIM);
    }

    #[test]
    fn ticket_address_is_deterministic_and_seed_specific() {
        let owner = Pubkey::new_unique();
        let a = MarinadeState::order_unstake_ticket_address(&owner, "ticket-1");
        let b = MarinadeState::order_unstake_ticket_address(&owner, "ticket-1");
        let c = MarinadeState::order_unstake_ticket_address(&owner, "ticket-2");
        assert_eq!(a, b, "same owner+seed must derive the same address");
        assert_ne!(a, c, "different seeds must derive different addresses");
    }

    #[test]
    fn marginal_fee_interpolates_linearly() {
        let mut m = MarinadeState {
            program_id: 0,
            state_id: 0,
            sol_leg_id: 0,
            sol_leg_pda: Pubkey::default(),
            mint_id: 0,
            sol_mint_id: 1,
            msol_leg_pk: Pubkey::default(),
            treasury_msol_account: Pubkey::default(),
            rent_exempt_for_token_acc: 0,
            lp_liquidity_target: 10_000,
            lp_max_fee_bps: 300,
            lp_min_fee_bps: 30,
            msol_price_raw: 0,
            sol_leg_lamports: 0,
            last_stake_delta_epoch: 0,
            delayed_unstake_fee_bp_cents: 0,
            state_logged: false,
            sol_leg_logged: false,
        };
        // At/above target: min fee.
        assert_eq!(m.marginal_fee_bps(10_000), 30);
        assert_eq!(m.marginal_fee_bps(20_000), 30);
        // Empty: max fee.
        assert_eq!(m.marginal_fee_bps(0), 300);
        // Halfway: halfway between min and max.
        assert_eq!(m.marginal_fee_bps(5_000), 165);
        // Zero target doesn't divide-by-zero.
        m.lp_liquidity_target = 0;
        assert_eq!(m.marginal_fee_bps(5_000), 30);
    }

    #[test]
    fn compare_unstake_paths_math() {
        let m = MarinadeState {
            program_id: 0,
            state_id: 0,
            sol_leg_id: 0,
            sol_leg_pda: Pubkey::default(),
            mint_id: 0,
            // Matches `sol_id` below -- a synthetic id standing in for the
            // real WSOL mint, since `account_id_from_pubkey` can't run
            // outside the real wasm32-wasip2 guest (see field doc).
            sol_mint_id: 1,
            msol_leg_pk: Pubkey::default(),
            treasury_msol_account: Pubkey::default(),
            rent_exempt_for_token_acc: 0,
            lp_liquidity_target: 10_000,
            lp_max_fee_bps: 300,
            // price = 1.5 SOL/mSOL exactly (1.5 * 2^32).
            lp_min_fee_bps: 30,
            msol_price_raw: 6_442_450_944,
            sol_leg_lamports: 20_000, // >= lp_liquidity_target -> min fee
            last_stake_delta_epoch: 5,
            delayed_unstake_fee_bp_cents: 2_000, // 0.2%
            state_logged: false,
            sol_leg_logged: false,
        };

        // A real TradeRouter with exactly one mSOL->SOL edge, tagged
        // Sanctum -- proves the instant leg comes from the router (any
        // registered venue), not from this pool's own price/fee fields.
        // A synthetic id stands in for the real WSOL mint here: the real
        // `account_id_from_pubkey` calls a WIT-imported host function with
        // no implementation under a native test binary (only the real
        // wasm32-wasip2 guest runtime provides one), so calling it from a
        // unit test aborts the whole process.
        let sol_id: AccountId = 1;
        let mut r = crate::trader::router::Router::new(4, 0.01);
        r.register_mint(m.mint_id);
        r.register_mint(sol_id);
        let mut router = TradeRouter::from_router(&r);
        router.add_directed_edge(
            999,
            m.mint_id,
            sol_id,
            1.5,   // price -- consistent with the 1e9:1.5e9 reserves below
            0.003, // 0.3% fee
            1_000_000_000,
            1_500_000_000,
            DexType::Sanctum,
        );
        let expected_instant =
            crate::trader::pricegraph::cp_quote(1_000, 1_000_000_000, 1_500_000_000, 30);

        // current_epoch(5) == last_stake_delta_epoch(5) -> created_epoch
        // bumps by one extra epoch -> wait_epochs = 2.
        let mut m = m;
        m.msol_price_raw = 6_442_450_944; // 1.5 SOL/mSOL, for the delayed leg
        let cmp = m.compare_unstake_paths(1_000, 5 * SLOTS_PER_EPOCH, &router).unwrap();
        assert_eq!(cmp.instant_out, expected_instant);
        assert_eq!(cmp.instant_route, vec![DexType::Sanctum]);
        assert_eq!(cmp.delayed_out, 1_497); // (1000 - 2000bp-cents fee=2) * 1.5 = 1497
        assert_eq!(cmp.wait_epochs, 2);

        // current_epoch(6) != last_stake_delta_epoch(5) -> no extra bump
        // -> wait_epochs = 1. Prices/fees unaffected by current_slot.
        let cmp2 = m.compare_unstake_paths(1_000, 6 * SLOTS_PER_EPOCH, &router).unwrap();
        assert_eq!(cmp2.instant_out, expected_instant);
        assert_eq!(cmp2.delayed_out, 1_497);
        assert_eq!(cmp2.wait_epochs, 1);

        // Not ready when the router has no route yet, even with live
        // State data.
        let empty_router = TradeRouter::from_router(&r);
        assert!(m.compare_unstake_paths(1_000, 0, &empty_router).is_none());

        // Not ready until live State data has arrived, even with a route
        // available.
        let mut not_ready = m;
        not_ready.msol_price_raw = 0;
        assert!(not_ready.compare_unstake_paths(1_000, 0, &router).is_none());
    }
}
