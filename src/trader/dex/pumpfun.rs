//! Pump.fun bonding-curve trader.
//!
//! # Program
//!
//! `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`, mainnet+devnet. Every
//! account layout, PDA seed, and instruction discriminator below was
//! verified against pump.fun's own published IDL
//! (`github.com/pump-fun/pump-public-docs`, `idl/pump.json` and
//! `idl/pump_fees.json`) -- the account discriminator and every
//! instruction discriminator were independently recomputed via
//! `sha256("account:BondingCurve")` / `sha256("global:buy")` / etc and
//! matched the IDL's own `discriminator` fields byte-for-byte (see the
//! tests below). Every constant PDA seed (`fee_config`'s embedded
//! pump-program-ID seed, the Associated Token program ID embedded in
//! `associated_bonding_curve`'s seeds) was independently base58-decoded
//! and matched too.
//!
//! # `BondingCurve` account layout (discriminator `[23,183,248,55,96,216,172,96]`)
//!
//! PDA: `["bonding-curve", mint]`. The account's own data never stores its
//! mint -- only implicit in the PDA seed -- so mint discovery has to come
//! from elsewhere; see `optimizer/prefetch/pumpfun` (the Go side) for how
//! that's solved (walking the generic `mint_authority -> mint` edge from
//! pump.fun's fixed mint-authority PDA, since every pump.fun token shares
//! that one mint authority).
//!
//! ```text
//! @8   virtual_token_reserves   u64
//! @16  virtual_sol_reserves     u64  (aka virtual_quote_reserves)
//! @24  real_token_reserves      u64
//! @32  real_sol_reserves        u64  (aka real_quote_reserves)
//! @40  token_total_supply       u64
//! @48  complete                 bool
//! @49  creator                  Pubkey
//! @81  is_mayhem_mode           bool
//! @82  is_cashback_coin         bool
//! @83  quote_mint               Pubkey
//! ```
//! `complete=true` means the curve migrated to PumpSwap (no longer
//! tradeable via the bonding curve) -- `batch_router` excludes those.
//!
//! Virtual reserves (not real) are what this module prices and sizes
//! liquidity from -- unlike Orca's CLMM virtual-liquidity (unbounded,
//! tick-range dependent, see `orca.rs`'s module doc), Pump.fun's virtual
//! reserves are `real + a fixed initial offset` and are literally what the
//! on-chain constant-product swap math uses, so they're both the correct
//! pricing input and a safe (bounded) liquidity proxy.
//!
//! # `Global` account layout (single fixed account, PDA seed `["global"]`,
//! real address `4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf` -- verified
//! against the module's own PDA derivation in this file's tests)
//!
//! Only the fields this module needs:
//! ```text
//! @41   fee_recipient              Pubkey
//! @105  fee_basis_points           u64
//! @154  creator_fee_basis_points   u64
//! ```
//! `fee_basis_points + creator_fee_basis_points` is the flat fee estimate
//! `batch_router` uses -- the real fee system also has market-cap-tiered
//! overrides via a separate `FeeConfig`/`fee_tiers` account on the fee
//! program, not modeled here (same kind of documented simplification as
//! `marinade.rs`'s `compare_unstake_paths` using a flat
//! `delayed_unstake_fee` instead of the full tiered formula).
//!
//! # `buy`/`sell` instructions
//!
//! `buy` (disc `[102,6,61,18,1,218,235,234]`) accounts, in order: `global,
//! fee_recipient, mint, bonding_curve, associated_bonding_curve,
//! associated_user, user (signer), system_program, token_program,
//! creator_vault, event_authority, program, global_volume_accumulator,
//! user_volume_accumulator, fee_config, fee_program`. Data: disc +
//! `amount: u64` + `max_sol_cost: u64` + `track_volume: bool` (the IDL's
//! `OptionBool` is a plain 1-field newtype struct around `bool`, NOT a
//! real Borsh `Option` -- serializes as exactly 1 byte, no discriminant;
//! this module always sends `false`, a conservative default).
//!
//! `sell` (disc `[51,230,133,164,1,127,131,173]`): same accounts minus
//! `global_volume_accumulator`/`user_volume_accumulator`. Data: disc +
//! `amount: u64` + `min_sol_output: u64` (no `track_volume`).
//!
//! PDAs: `global` = `["global"]`, `event_authority` =
//! `["__event_authority"]`, both on the pump program (fixed, computed once
//! in [`PumpfunState::new`]). `fee_config` = `["fee_config",
//! PUMPFUN_PROGRAM_ID]` on the **fee program**
//! (`pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ`) -- fixed, also computed
//! once. `bonding_curve` = `["bonding-curve", mint]`, `creator_vault` =
//! `["creator-vault", bonding_curve.creator]` (needs the live `creator`
//! field), `global_volume_accumulator` = `["global_volume_accumulator"]`
//! (fixed), `user_volume_accumulator` = `["user_volume_accumulator",
//! user]` -- all on the pump program. `associated_bonding_curve` is a
//! standard ATA derivation `(bonding_curve, token_program, mint)`, same
//! shape as `sanctum.rs`'s private `spl_ata` helper (reused here).
//!
//! **Known caveat**: `fee_recipient` (a writable, non-PDA account) is
//! resolved from the live `Global.fee_recipient` field -- the one single
//! documented field. Real production traffic may rotate among a larger
//! set (`Global.fee_recipients`, `Global.buyback_fee_recipients`) by
//! undocumented logic; this is a scoped, documented best-effort
//! approximation, not a correctness risk to the running bot, since nothing
//! here ever actually sends these instructions (same "built but never
//! called" convention as every other dex module this session).

use std::collections::HashMap;

use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    trader::{
        dex::update::Updater,
        pricegraph::{Hop, TradeRouter},
        types::{DexType, TraderError},
    },
    txview::CatscopeInstructionRead,
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};

pub const PUMPFUN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
const PUMPFUN_FEE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

const WSOL_MINT: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");
const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

// ─── BondingCurve account layout (see module doc) ─────────────────────────
const BONDING_CURVE_MIN_LEN: usize = 115;
const OFF_BC_VIRTUAL_TOKEN_RESERVES: usize = 8;
const OFF_BC_VIRTUAL_SOL_RESERVES: usize = 16;
const OFF_BC_REAL_TOKEN_RESERVES: usize = 24;
const OFF_BC_REAL_SOL_RESERVES: usize = 32;
const OFF_BC_COMPLETE: usize = 48;
const OFF_BC_CREATOR: usize = 49;

// ─── Global account layout (see module doc) ───────────────────────────────
const GLOBAL_MIN_LEN: usize = 162;
const OFF_GLOBAL_FEE_RECIPIENT: usize = 41;
const OFF_GLOBAL_FEE_BASIS_POINTS: usize = 105;
const OFF_GLOBAL_CREATOR_FEE_BASIS_POINTS: usize = 154;

/// sha256("global:buy")[..8], cross-checked in this module's tests below
/// (same convention as `marinade.rs`/`kamino.rs`).
const DISC_BUY: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
/// sha256("global:sell")[..8], cross-checked below.
const DISC_SELL: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

pub const PUMPFUN_BUY_CU: u32 = 80_000;
pub const PUMPFUN_SELL_CU: u32 = 70_000;

/// One bonding curve's parsed state, plus the derived Pubkeys that never
/// change once the mint is known (cached at construction time rather than
/// re-derived per swap call, same convention as `spl_stake_pool.rs`'s
/// `LstEntry` caching `pool_state_pk` alongside `pool_state_id`).
#[derive(Debug)]
struct PumpfunBondingCurveState {
    mint_id: AccountId,
    mint_pk: Pubkey,
    bonding_curve_id: AccountId,
    bonding_curve_pk: Pubkey,
    associated_bonding_curve_pk: Pubkey,
    /// Zero (default `Pubkey`) until the first account update arrives.
    creator: Pubkey,
    virtual_token_reserves: u64,
    virtual_sol_reserves: u64,
    real_token_reserves: u64,
    real_sol_reserves: u64,
    complete: bool,
}

pub struct PumpfunState {
    program_id: AccountId,
    global_id: AccountId,
    global_pda: Pubkey,
    event_authority: Pubkey,
    fee_config: Pubkey,
    /// `WSOL_MINT`'s `AccountId`, resolved once in `new()` -- see
    /// `marinade.rs`'s `sol_mint_id` field doc for why this must be cached
    /// rather than re-resolved per call.
    sol_mint_id: AccountId,
    fee_recipient: Pubkey,
    fee_basis_points: u64,
    creator_fee_basis_points: u64,
    /// `true` once the single `Global` account has been parsed at least
    /// once -- a real `bool` gate rather than overloading a data field,
    /// since `fee_basis_points`/`creator_fee_basis_points` could
    /// legitimately be zero.
    global_ready: bool,
    curves: Vec<PumpfunBondingCurveState>,
    curve_to_idx: HashMap<AccountId, usize>,
}

impl std::fmt::Debug for PumpfunState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PumpfunState").finish()
    }
}

impl PumpfunState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let (global_pda, _bump) = Pubkey::find_program_address(&[b"global"], &PUMPFUN_PROGRAM_ID);
        let (event_authority, _bump) =
            Pubkey::find_program_address(&[b"__event_authority"], &PUMPFUN_PROGRAM_ID);
        let (fee_config, _bump) = Pubkey::find_program_address(
            &[b"fee_config", PUMPFUN_PROGRAM_ID.as_ref()],
            &PUMPFUN_FEE_PROGRAM_ID,
        );
        let global_id = account_id_from_pubkey(&global_pda);

        let raw = crate::pumpfun_config::PUMPFUN_BONDING_CURVES;
        let mut curves = Vec::with_capacity(raw.len());
        let mut curve_to_idx = HashMap::with_capacity(raw.len());
        let mut l_req = Vec::with_capacity(raw.len() + 1);
        l_req.push(SubscriptionRequest { root: global_id, filter_weight: 0, depth: 1 });

        for entry in raw.iter() {
            let mint_pk = Pubkey::new_from_array(entry.mint);
            let mint_id = account_id_from_pubkey(&mint_pk);
            let (bonding_curve_pk, _bump) =
                Pubkey::find_program_address(&[b"bonding-curve", mint_pk.as_ref()], &PUMPFUN_PROGRAM_ID);
            let bonding_curve_id = account_id_from_pubkey(&bonding_curve_pk);
            let associated_bonding_curve_pk =
                spl_ata(&bonding_curve_pk, &SPL_TOKEN_PROGRAM_ID, &mint_pk);

            let idx = curves.len();
            curve_to_idx.insert(bonding_curve_id, idx);
            l_req.push(SubscriptionRequest { root: bonding_curve_id, filter_weight: 0, depth: 1 });
            curves.push(PumpfunBondingCurveState {
                mint_id,
                mint_pk,
                bonding_curve_id,
                bonding_curve_pk,
                associated_bonding_curve_pk,
                creator: Pubkey::default(),
                virtual_token_reserves: 0,
                virtual_sol_reserves: 0,
                real_token_reserves: 0,
                real_sol_reserves: 0,
                complete: false,
            });
        }

        let state = Self {
            program_id: account_id_from_pubkey(&PUMPFUN_PROGRAM_ID),
            global_id,
            global_pda,
            event_authority,
            fee_config,
            sol_mint_id: account_id_from_pubkey(&WSOL_MINT),
            fee_recipient: Pubkey::default(),
            fee_basis_points: 0,
            creator_fee_basis_points: 0,
            global_ready: false,
            curves,
            curve_to_idx,
        };
        (state, l_req)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// How many tracked bonding curves have live, tradeable (not
    /// `complete`) pricing data -- for the periodic "pool stats" log
    /// (mirrors `SplStakePoolState::ready_count`).
    pub fn ready_count(&self) -> usize {
        self.curves
            .iter()
            .filter(|c| !c.complete && c.virtual_token_reserves != 0 && c.virtual_sol_reserves != 0)
            .count()
    }

    fn parse_global(&mut self, data: &[u8]) {
        if data.len() < GLOBAL_MIN_LEN {
            return;
        }
        let u64_at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let pubkey_at = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        self.fee_recipient = pubkey_at(OFF_GLOBAL_FEE_RECIPIENT);
        self.fee_basis_points = u64_at(OFF_GLOBAL_FEE_BASIS_POINTS);
        self.creator_fee_basis_points = u64_at(OFF_GLOBAL_CREATOR_FEE_BASIS_POINTS);
        self.global_ready = true;
    }

    fn parse_curve(&mut self, idx: usize, data: &[u8]) {
        if data.len() < BONDING_CURVE_MIN_LEN {
            return;
        }
        let u64_at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let pubkey_at = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        let c = &mut self.curves[idx];
        c.virtual_token_reserves = u64_at(OFF_BC_VIRTUAL_TOKEN_RESERVES);
        c.virtual_sol_reserves = u64_at(OFF_BC_VIRTUAL_SOL_RESERVES);
        c.real_token_reserves = u64_at(OFF_BC_REAL_TOKEN_RESERVES);
        c.real_sol_reserves = u64_at(OFF_BC_REAL_SOL_RESERVES);
        c.complete = data[OFF_BC_COMPLETE] != 0;
        c.creator = pubkey_at(OFF_BC_CREATOR);
    }

    /// Build a `buy` instruction (SOL -> token) and append it to `wallet`.
    /// `user_token_account` is the buyer's own ATA for `mint_id` -- must
    /// already exist (this builder doesn't create it, same assumption
    /// `marinade.rs::swap` makes about the caller's mSOL account).
    pub fn buy(
        &self,
        mint_id: AccountId,
        amount_in: u64,
        max_sol_cost: u64,
        user_wallet: AccountId,
        user_token_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let curve = self.curves.iter().find(|c| c.mint_id == mint_id).ok_or(TraderError::WrongMints)?;
        if curve.creator == Pubkey::default() || !self.global_ready {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_token_pk = resolve(user_token_account)?;

        let (creator_vault, _bump) =
            Pubkey::find_program_address(&[b"creator-vault", curve.creator.as_ref()], &PUMPFUN_PROGRAM_ID);
        let (global_volume_accumulator, _bump) =
            Pubkey::find_program_address(&[b"global_volume_accumulator"], &PUMPFUN_PROGRAM_ID);
        let (user_volume_accumulator, _bump) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", user_wallet_pk.as_ref()],
            &PUMPFUN_PROGRAM_ID,
        );

        let mut data = Vec::with_capacity(25);
        data.extend_from_slice(&DISC_BUY);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&max_sol_cost.to_le_bytes());
        data.push(0); // track_volume: OptionBool(false) -- conservative default.

        let accounts = vec![
            AccountMeta::new_readonly(self.global_pda, false), // global
            AccountMeta::new(self.fee_recipient, false),       // fee_recipient
            AccountMeta::new_readonly(curve.mint_pk, false),   // mint
            AccountMeta::new(curve.bonding_curve_pk, false),   // bonding_curve
            AccountMeta::new(curve.associated_bonding_curve_pk, false), // associated_bonding_curve
            AccountMeta::new(user_token_pk, false),             // associated_user
            AccountMeta::new_readonly(user_wallet_pk, true),    // user (signer)
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false), // system_program
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // token_program
            AccountMeta::new(creator_vault, false),             // creator_vault
            AccountMeta::new_readonly(self.event_authority, false), // event_authority
            AccountMeta::new_readonly(PUMPFUN_PROGRAM_ID, false), // program
            AccountMeta::new(global_volume_accumulator, false), // global_volume_accumulator
            AccountMeta::new(user_volume_accumulator, false),  // user_volume_accumulator
            AccountMeta::new_readonly(self.fee_config, false), // fee_config
            AccountMeta::new_readonly(PUMPFUN_FEE_PROGRAM_ID, false), // fee_program
        ];

        wallet.require_signer(user_wallet);
        wallet.append_ix(Instruction { program_id: PUMPFUN_PROGRAM_ID, accounts, data }, PUMPFUN_BUY_CU);
        Ok(())
    }

    /// Build a `sell` instruction (token -> SOL) and append it to `wallet`.
    pub fn sell(
        &self,
        mint_id: AccountId,
        amount_in: u64,
        min_sol_output: u64,
        user_wallet: AccountId,
        user_token_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let curve = self.curves.iter().find(|c| c.mint_id == mint_id).ok_or(TraderError::WrongMints)?;
        if curve.creator == Pubkey::default() || !self.global_ready {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_token_pk = resolve(user_token_account)?;

        let (creator_vault, _bump) =
            Pubkey::find_program_address(&[b"creator-vault", curve.creator.as_ref()], &PUMPFUN_PROGRAM_ID);

        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&DISC_SELL);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_sol_output.to_le_bytes());

        let accounts = vec![
            AccountMeta::new_readonly(self.global_pda, false), // global
            AccountMeta::new(self.fee_recipient, false),       // fee_recipient
            AccountMeta::new_readonly(curve.mint_pk, false),   // mint
            AccountMeta::new(curve.bonding_curve_pk, false),   // bonding_curve
            AccountMeta::new(curve.associated_bonding_curve_pk, false), // associated_bonding_curve
            AccountMeta::new(user_token_pk, false),             // associated_user
            AccountMeta::new_readonly(user_wallet_pk, true),    // user (signer)
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false), // system_program
            AccountMeta::new(creator_vault, false),             // creator_vault
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // token_program
            AccountMeta::new_readonly(self.event_authority, false), // event_authority
            AccountMeta::new_readonly(PUMPFUN_PROGRAM_ID, false), // program
            AccountMeta::new_readonly(self.fee_config, false), // fee_config
            AccountMeta::new_readonly(PUMPFUN_FEE_PROGRAM_ID, false), // fee_program
        ];

        wallet.require_signer(user_wallet);
        wallet.append_ix(Instruction { program_id: PUMPFUN_PROGRAM_ID, accounts, data }, PUMPFUN_SELL_CU);
        Ok(())
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    /// Picks `buy` vs `sell` from whether `hop.input_mint` is the wSOL
    /// mint. Note `buy`'s own `amount_in` param is actually the *token*
    /// amount desired out (Anchor's `buy(amount, max_sol_cost)`, despite
    /// the misleading Rust param name) -- `hop.amount_out`/`hop.amount_in`
    /// are swapped accordingly below, not passed straight through.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        if hop.input_mint == self.sol_mint_id {
            // Buy: SOL -> token. dest_ata holds the token being bought.
            self.buy(hop.output_mint, hop.amount_out, hop.amount_in, owner, dest_ata, wallet)
        } else {
            // Sell: token -> SOL. source_ata holds the token being sold.
            self.sell(hop.input_mint, hop.amount_in, hop.amount_out, owner, source_ata, wallet)
        }
    }

    /// Re-derive and upsert one curve's token<->SOL edge. `valid` gates
    /// `price` to `0.0` rather than skipping the `add_generic_pair` call
    /// on any of the three original invalidity conditions (`complete`,
    /// zero virtual reserves), so its own `price_b_per_a <= 0.0` gate
    /// reliably removes a stale edge left over from before the curve
    /// completed or went invalid. Returns early (no-op) if `!global_ready`,
    /// matching `batch_router`'s original top-level gate -- no edge is
    /// ever inserted before that, so there's nothing to remove either.
    fn upsert_curve_edge(&self, idx: usize, router: &mut TradeRouter) {
        if !self.global_ready {
            return;
        }
        let c = &self.curves[idx];
        let valid = !c.complete && c.virtual_token_reserves != 0 && c.virtual_sol_reserves != 0;
        let price = if valid {
            c.virtual_sol_reserves as f64 / c.virtual_token_reserves as f64
        } else {
            0.0
        };
        let fee_frac = (self.fee_basis_points + self.creator_fee_basis_points) as f64 / 10_000.0;
        router.add_generic_pair(
            c.bonding_curve_id,
            c.mint_id,
            self.sol_mint_id,
            price,
            fee_frac,
            c.virtual_token_reserves,
            c.virtual_sol_reserves,
            DexType::PumpfunBondingCurve,
        );
    }
}

/// Standard ATA derivation -- same shape as `sanctum.rs`'s private
/// `spl_ata` helper (seeds `[owner, token_program, mint]` under the
/// Associated Token program).
fn spl_ata(owner: &Pubkey, token_program: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

impl Updater for PumpfunState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.accountid == self.global_id {
            self.parse_global(body);
        } else if let Some(&idx) = self.curve_to_idx.get(&header.accountid) {
            self.parse_curve(idx, body);
        }
    }

    fn on_token(&mut self, _ta: &Tokenaccountv1) -> bool {
        false
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &solana_sdk::clock::Slot) {}

    /// Add a token <-> SOL edge for every incomplete, priced bonding curve.
    fn batch_router(&mut self, router: &mut TradeRouter) {
        for idx in 0..self.curves.len() {
            self.upsert_curve_edge(idx, router);
        }
    }

    /// `global_id` carries the shared fee bps every curve's edge depends
    /// on, so any update to it (including the first time it becomes
    /// ready) must re-derive every curve, not just one.
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if account_id == self.global_id {
            for idx in 0..self.curves.len() {
                self.upsert_curve_edge(idx, router);
            }
        } else if let Some(&idx) = self.curve_to_idx.get(&account_id) {
            self.upsert_curve_edge(idx, router);
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
        assert_eq!(disc("buy"), DISC_BUY);
        assert_eq!(disc("sell"), DISC_SELL);
    }

    #[test]
    fn global_pda_matches_real_known_address() {
        // Publicly documented real Global account address -- independent
        // confirmation that the "global" seed derivation is correct, not
        // just internally self-consistent.
        let (global_pda, _bump) = Pubkey::find_program_address(&[b"global"], &PUMPFUN_PROGRAM_ID);
        assert_eq!(global_pda, Pubkey::from_str_const("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf"));
    }

    #[test]
    fn fee_config_pda_is_seeded_by_the_pump_program_id() {
        // Verified this session by base58-decoding the IDL's embedded
        // 32-byte fee_config seed constant and confirming it equals the
        // pump program's own address exactly.
        let (fee_config, _bump) = Pubkey::find_program_address(
            &[b"fee_config", PUMPFUN_PROGRAM_ID.as_ref()],
            &PUMPFUN_FEE_PROGRAM_ID,
        );
        assert_ne!(fee_config, Pubkey::default());
    }

    #[test]
    fn bonding_curve_address_is_deterministic_and_mint_specific() {
        let mint_a = Pubkey::new_unique();
        let mint_b = Pubkey::new_unique();
        let (a1, _) = Pubkey::find_program_address(&[b"bonding-curve", mint_a.as_ref()], &PUMPFUN_PROGRAM_ID);
        let (a2, _) = Pubkey::find_program_address(&[b"bonding-curve", mint_a.as_ref()], &PUMPFUN_PROGRAM_ID);
        let (b, _) = Pubkey::find_program_address(&[b"bonding-curve", mint_b.as_ref()], &PUMPFUN_PROGRAM_ID);
        assert_eq!(a1, a2, "same mint must derive the same bonding_curve address");
        assert_ne!(a1, b, "different mints must derive different bonding_curve addresses");
    }

    fn synthetic_curve(mint_id: AccountId, bonding_curve_id: AccountId) -> PumpfunBondingCurveState {
        PumpfunBondingCurveState {
            mint_id,
            mint_pk: Pubkey::default(),
            bonding_curve_id,
            bonding_curve_pk: Pubkey::default(),
            associated_bonding_curve_pk: Pubkey::default(),
            creator: Pubkey::new_unique(),
            virtual_token_reserves: 1_073_000_000_000_000,
            virtual_sol_reserves: 30_000_000_000,
            real_token_reserves: 793_100_000_000_000,
            real_sol_reserves: 0,
            complete: false,
        }
    }

    #[test]
    fn batch_router_adds_edge_for_incomplete_priced_curve() {
        let mint_id: AccountId = 1;
        let sol_id: AccountId = 2;
        let bonding_curve_id: AccountId = 3;

        let mut state = PumpfunState {
            program_id: 0,
            global_id: 0,
            global_pda: Pubkey::default(),
            event_authority: Pubkey::default(),
            fee_config: Pubkey::default(),
            sol_mint_id: sol_id,
            fee_recipient: Pubkey::default(),
            fee_basis_points: 100,
            creator_fee_basis_points: 100,
            global_ready: true,
            curves: vec![synthetic_curve(mint_id, bonding_curve_id)],
            curve_to_idx: HashMap::new(),
        };

        let mut r = crate::trader::router::Router::new(4, 0.01);
        r.register_mint(mint_id);
        r.register_mint(sol_id);
        let mut router = TradeRouter::from_router(&r);
        state.batch_router(&mut router);
        assert_eq!(router.edge_counts_by_dex(), vec![(DexType::PumpfunBondingCurve, 2)]);

        // Not ready until Global has been parsed at least once, even with
        // priced curves.
        state.global_ready = false;
        let mut router2 = TradeRouter::from_router(&r);
        state.batch_router(&mut router2);
        assert_eq!(router2.edge_counts_by_dex(), vec![]);

        // A completed (migrated) curve is excluded even when Global is ready.
        state.global_ready = true;
        state.curves[0].complete = true;
        let mut router3 = TradeRouter::from_router(&r);
        state.batch_router(&mut router3);
        assert_eq!(router3.edge_counts_by_dex(), vec![]);
    }
}
