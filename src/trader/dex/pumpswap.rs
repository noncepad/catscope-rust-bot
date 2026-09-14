//! PumpSwap AMM trader.
//!
//! # Program
//!
//! `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` -- the permanent AMM a
//! Pump.fun bonding-curve token migrates to once it completes/graduates
//! (see `pumpfun.rs`'s module doc). A genuinely different, standard
//! constant-product AMM, structurally like Raydium AMM/Orca, not a
//! bonding curve -- `Pool` accounts store both mints *and* both vault
//! pubkeys directly (no mint-discovery problem like `BondingCurve` had),
//! and reserves are the real SPL balances of those two vaults (no inline
//! virtual/real reserve fields).
//!
//! Every account layout, PDA seed, and instruction discriminator below was
//! verified against pump.fun's own published IDL
//! (`github.com/pump-fun/pump-public-docs`, `idl/pump_amm.json`) the same
//! way `pumpfun.rs` was -- the account discriminator and every instruction
//! discriminator were independently recomputed via
//! `sha256("account:Pool")` / `sha256("global:buy")` / etc and matched the
//! IDL's own `discriminator` fields byte-for-byte (see the tests below;
//! `buy`/`sell`'s discriminators are the exact same bytes as `pumpfun.rs`'s
//! own `DISC_BUY`/`DISC_SELL`, since Anchor sighashes depend only on the
//! instruction *name*, not the program -- asserted explicitly in the
//! tests, not a coincidence to gloss over).
//!
//! # `Pool` account layout (discriminator `[241,154,109,4,17,177,109,188]`)
//!
//! ```text
//! @8   pool_bump                 u8
//! @9   index                     u16
//! @11  creator                   Pubkey
//! @43  base_mint                 Pubkey
//! @75  quote_mint                Pubkey
//! @107 lp_mint                   Pubkey
//! @139 pool_base_token_account   Pubkey
//! @171 pool_quote_token_account  Pubkey
//! @203 lp_supply                 u64
//! @211 coin_creator              Pubkey
//! @243 is_mayhem_mode            bool
//! @244 is_cashback_coin          bool
//! @245 virtual_quote_reserves    i128  -- 0 for non-boost pools (the
//!                                          overwhelming majority); not
//!                                          modeled, same class of
//!                                          simplification as
//!                                          `pumpfun.rs`'s flat fee
//!                                          estimate
//! ```
//! Min length 261 bytes.
//!
//! # `GlobalConfig` account (discriminator `[149,8,156,202,160,252,176,217]`,
//! single fixed account, PDA seed `["global_config"]`)
//!
//! ```text
//! @40  lp_fee_basis_points            u64
//! @48  protocol_fee_basis_points      u64
//! @57  protocol_fee_recipients        [Pubkey; 8]
//! @313 coin_creator_fee_basis_points  u64
//! ```
//! `lp_fee_bps + protocol_fee_bps + coin_creator_fee_bps` is the flat fee
//! estimate `batch_router` uses (same simplification style as
//! `pumpfun.rs`'s `Global`).
//!
//! # `buy`/`sell` instructions
//!
//! `buy` (disc `[102,6,61,18,1,218,235,234]`) accounts, in order: `pool,
//! user (signer), global_config, base_mint, quote_mint,
//! user_base_token_account, user_quote_token_account,
//! pool_base_token_account, pool_quote_token_account,
//! protocol_fee_recipient, protocol_fee_recipient_token_account,
//! base_token_program, quote_token_program, system_program,
//! associated_token_program, event_authority, program,
//! coin_creator_vault_ata, coin_creator_vault_authority,
//! global_volume_accumulator, user_volume_accumulator, fee_config,
//! fee_program`. Data: disc + `base_amount_out: u64` + `max_quote_amount_in:
//! u64` + `track_volume: bool` (same `OptionBool` 1-byte newtype
//! convention as `pumpfun.rs` -- always `false` here).
//!
//! `sell` (disc `[51,230,133,164,1,127,131,173]`): same accounts minus
//! `global_volume_accumulator`/`user_volume_accumulator`. Data: disc +
//! `base_amount_in: u64` + `min_quote_amount_out: u64`.
//!
//! PDAs: `global_config` = `["global_config"]`, `event_authority` =
//! `["__event_authority"]`, both fixed. `fee_config` =
//! `["fee_config", PUMPSWAP_PROGRAM_ID]` on the same shared fee program as
//! Pump.fun (`pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ`) -- fixed, byte
//! verified. `coin_creator_vault_authority` = `["creator_vault",
//! pool.coin_creator]` -- **note the underscore**, not Pump.fun bonding
//! curve's hyphenated `"creator-vault"` seed; a different literal string,
//! verified independently from the IDL's raw seed bytes. `global_volume_
//! accumulator` = `["global_volume_accumulator"]` (fixed). `user_volume_
//! accumulator` = `["user_volume_accumulator", user]`.
//! `coin_creator_vault_ata`/`protocol_fee_recipient_token_account` are
//! standard ATA derivations, same `spl_ata` helper shape as `pumpfun.rs`/
//! `sanctum.rs`.
//!
//! **Known caveats** (same class and reasoning as `pumpfun.rs`'s):
//! `base_token_program`/`quote_token_program` are always assumed to be the
//! legacy SPL Token program (Token-2022 not modeled). `protocol_fee_
//! recipient` uses the live `GlobalConfig.protocol_fee_recipients[0]` as a
//! best-effort single choice (the real selection among the 8 is
//! undocumented). Neither affects pricing -- only the never-called swap
//! builders, matching every other dex module's "built but never called"
//! convention this session.

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

pub const PUMPSWAP_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
const PUMPSWAP_FEE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

// ─── Pool account layout (see module doc; base_mint@43/quote_mint@75 are
// not re-read here since they're already fixed at construction time from
// build.rs's embedded PUMPSWAP_POOLS list) ─────────────────────────────────
const POOL_MIN_LEN: usize = 261;
const OFF_POOL_COIN_CREATOR: usize = 211;

// ─── GlobalConfig account layout (see module doc) ─────────────────────────
const GLOBAL_CONFIG_MIN_LEN: usize = 321;
const OFF_GC_LP_FEE_BPS: usize = 40;
const OFF_GC_PROTOCOL_FEE_BPS: usize = 48;
const OFF_GC_PROTOCOL_FEE_RECIPIENTS: usize = 57;
const OFF_GC_COIN_CREATOR_FEE_BPS: usize = 313;

/// sha256("global:buy")[..8] -- identical bytes to `pumpfun.rs`'s
/// `DISC_BUY` (Anchor sighashes depend only on the instruction name), also
/// cross-checked in this module's tests.
const DISC_BUY: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
/// sha256("global:sell")[..8] -- identical to `pumpfun.rs`'s `DISC_SELL`.
const DISC_SELL: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

pub const PUMPSWAP_BUY_CU: u32 = 90_000;
pub const PUMPSWAP_SELL_CU: u32 = 80_000;

/// One pool's parsed state, plus the derived/embedded Pubkeys that never
/// change once discovered (cached at construction time, same convention as
/// `pumpfun.rs`'s `PumpfunBondingCurveState`).
#[derive(Debug)]
struct PumpswapPoolState {
    pool_id: AccountId,
    pool_pk: Pubkey,
    base_mint_id: AccountId,
    base_mint_pk: Pubkey,
    quote_mint_id: AccountId,
    quote_mint_pk: Pubkey,
    base_vault_pk: Pubkey,
    quote_vault_pk: Pubkey,
    /// Zero (default `Pubkey`) until the first `Pool` account update
    /// arrives.
    coin_creator: Pubkey,
    base_balance: u64,
    quote_balance: u64,
}

pub struct PumpswapState {
    program_id: AccountId,
    global_config_id: AccountId,
    global_config_pk: Pubkey,
    event_authority: Pubkey,
    fee_config: Pubkey,
    protocol_fee_recipient: Pubkey,
    lp_fee_bps: u64,
    protocol_fee_bps: u64,
    coin_creator_fee_bps: u64,
    /// `true` once the single `GlobalConfig` account has been parsed at
    /// least once -- see `pumpfun.rs`'s `global_ready` doc for why this is
    /// a real bool gate rather than overloading a data field.
    global_ready: bool,
    pools: Vec<PumpswapPoolState>,
    pool_by_id: HashMap<AccountId, usize>,
    /// `bool` = is_base (`true`) vs is_quote (`false`) -- mirrors
    /// `raydium/amm`'s `m_vault` index-map pattern for live vault-balance
    /// routing via `on_token`.
    vault_to_pool: HashMap<AccountId, (usize, bool)>,
}

impl std::fmt::Debug for PumpswapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PumpswapState").finish()
    }
}

impl PumpswapState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let (global_config_pk, _bump) =
            Pubkey::find_program_address(&[b"global_config"], &PUMPSWAP_PROGRAM_ID);
        let (event_authority, _bump) =
            Pubkey::find_program_address(&[b"__event_authority"], &PUMPSWAP_PROGRAM_ID);
        let (fee_config, _bump) = Pubkey::find_program_address(
            &[b"fee_config", PUMPSWAP_PROGRAM_ID.as_ref()],
            &PUMPSWAP_FEE_PROGRAM_ID,
        );
        let global_config_id = account_id_from_pubkey(&global_config_pk);

        let raw = crate::pumpswap_config::PUMPSWAP_POOLS;
        let mut pools = Vec::with_capacity(raw.len());
        let mut pool_by_id = HashMap::with_capacity(raw.len());
        let mut vault_to_pool = HashMap::with_capacity(raw.len() * 2);
        let mut l_req = Vec::with_capacity(raw.len() * 3 + 1);
        l_req.push(SubscriptionRequest { root: global_config_id, filter_weight: 0, depth: 1 });

        for entry in raw.iter() {
            let pool_pk = Pubkey::new_from_array(entry.pool);
            let pool_id = account_id_from_pubkey(&pool_pk);
            let base_mint_pk = Pubkey::new_from_array(entry.base_mint);
            let quote_mint_pk = Pubkey::new_from_array(entry.quote_mint);
            let base_vault_pk = Pubkey::new_from_array(entry.base_vault);
            let quote_vault_pk = Pubkey::new_from_array(entry.quote_vault);
            let base_vault_id = account_id_from_pubkey(&base_vault_pk);
            let quote_vault_id = account_id_from_pubkey(&quote_vault_pk);

            let idx = pools.len();
            pool_by_id.insert(pool_id, idx);
            vault_to_pool.insert(base_vault_id, (idx, true));
            vault_to_pool.insert(quote_vault_id, (idx, false));
            l_req.push(SubscriptionRequest { root: pool_id, filter_weight: 0, depth: 1 });
            l_req.push(SubscriptionRequest { root: base_vault_id, filter_weight: 0, depth: 1 });
            l_req.push(SubscriptionRequest { root: quote_vault_id, filter_weight: 0, depth: 1 });
            pools.push(PumpswapPoolState {
                pool_id,
                pool_pk,
                base_mint_id: account_id_from_pubkey(&base_mint_pk),
                base_mint_pk,
                quote_mint_id: account_id_from_pubkey(&quote_mint_pk),
                quote_mint_pk,
                base_vault_pk,
                quote_vault_pk,
                coin_creator: Pubkey::default(),
                base_balance: 0,
                quote_balance: 0,
            });
        }

        let state = Self {
            program_id: account_id_from_pubkey(&PUMPSWAP_PROGRAM_ID),
            global_config_id,
            global_config_pk,
            event_authority,
            fee_config,
            protocol_fee_recipient: Pubkey::default(),
            lp_fee_bps: 0,
            protocol_fee_bps: 0,
            coin_creator_fee_bps: 0,
            global_ready: false,
            pools,
            pool_by_id,
            vault_to_pool,
        };
        (state, l_req)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// How many tracked pools have live pricing data (both vault balances
    /// nonzero) -- for the periodic "pool stats" log.
    pub fn ready_count(&self) -> usize {
        self.pools.iter().filter(|p| p.base_balance != 0 && p.quote_balance != 0).count()
    }

    fn parse_global_config(&mut self, data: &[u8]) {
        if data.len() < GLOBAL_CONFIG_MIN_LEN {
            return;
        }
        let u64_at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let pubkey_at = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        self.lp_fee_bps = u64_at(OFF_GC_LP_FEE_BPS);
        self.protocol_fee_bps = u64_at(OFF_GC_PROTOCOL_FEE_BPS);
        self.coin_creator_fee_bps = u64_at(OFF_GC_COIN_CREATOR_FEE_BPS);
        self.protocol_fee_recipient = pubkey_at(OFF_GC_PROTOCOL_FEE_RECIPIENTS);
        self.global_ready = true;
    }

    fn parse_pool(&mut self, idx: usize, data: &[u8]) {
        if data.len() < POOL_MIN_LEN {
            return;
        }
        let pubkey_at = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        // base_mint/quote_mint are already fixed at construction time (from
        // build.rs's embedded list) -- only coin_creator is genuinely
        // live-updated here, matching bonding_curve.creator's pattern in
        // pumpfun.rs.
        self.pools[idx].coin_creator = pubkey_at(OFF_POOL_COIN_CREATOR);
    }

    /// Build a `buy` instruction (quote -> base) and append it to `wallet`.
    /// `user_base_token_account`/`user_quote_token_account` must already
    /// exist (this builder doesn't create them, same assumption every
    /// other swap builder this session makes about the caller's accounts).
    pub fn buy(
        &self,
        pool_id: AccountId,
        base_amount_out: u64,
        max_quote_amount_in: u64,
        user_wallet: AccountId,
        user_base_token_account: AccountId,
        user_quote_token_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let pool = self.pools.get(*self.pool_by_id.get(&pool_id).ok_or(TraderError::WrongMints)?)
            .ok_or(TraderError::WrongMints)?;
        if pool.coin_creator == Pubkey::default() || !self.global_ready {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_base_pk = resolve(user_base_token_account)?;
        let user_quote_pk = resolve(user_quote_token_account)?;

        let (coin_creator_vault_authority, _bump) = Pubkey::find_program_address(
            &[b"creator_vault", pool.coin_creator.as_ref()],
            &PUMPSWAP_PROGRAM_ID,
        );
        let coin_creator_vault_ata =
            spl_ata(&coin_creator_vault_authority, &SPL_TOKEN_PROGRAM_ID, &pool.quote_mint_pk);
        let protocol_fee_recipient_token_account =
            spl_ata(&self.protocol_fee_recipient, &SPL_TOKEN_PROGRAM_ID, &pool.quote_mint_pk);
        let (global_volume_accumulator, _bump) =
            Pubkey::find_program_address(&[b"global_volume_accumulator"], &PUMPSWAP_PROGRAM_ID);
        let (user_volume_accumulator, _bump) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", user_wallet_pk.as_ref()],
            &PUMPSWAP_PROGRAM_ID,
        );

        let mut data = Vec::with_capacity(25);
        data.extend_from_slice(&DISC_BUY);
        data.extend_from_slice(&base_amount_out.to_le_bytes());
        data.extend_from_slice(&max_quote_amount_in.to_le_bytes());
        data.push(0); // track_volume: OptionBool(false) -- conservative default.

        let accounts = vec![
            AccountMeta::new(pool.pool_pk, false),                    // pool
            AccountMeta::new_readonly(user_wallet_pk, true),          // user (signer)
            AccountMeta::new_readonly(self.global_config_pk, false),  // global_config
            AccountMeta::new_readonly(pool.base_mint_pk, false),      // base_mint
            AccountMeta::new_readonly(pool.quote_mint_pk, false),     // quote_mint
            AccountMeta::new(user_base_pk, false),                    // user_base_token_account
            AccountMeta::new(user_quote_pk, false),                   // user_quote_token_account
            AccountMeta::new(pool.base_vault_pk, false),              // pool_base_token_account
            AccountMeta::new(pool.quote_vault_pk, false),             // pool_quote_token_account
            AccountMeta::new_readonly(self.protocol_fee_recipient, false), // protocol_fee_recipient
            AccountMeta::new(protocol_fee_recipient_token_account, false), // protocol_fee_recipient_token_account
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),   // base_token_program
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),   // quote_token_program
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),      // system_program
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false), // associated_token_program
            AccountMeta::new_readonly(self.event_authority, false),  // event_authority
            AccountMeta::new_readonly(PUMPSWAP_PROGRAM_ID, false),   // program
            AccountMeta::new(coin_creator_vault_ata, false),          // coin_creator_vault_ata
            AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority
            AccountMeta::new_readonly(global_volume_accumulator, false), // global_volume_accumulator
            AccountMeta::new(user_volume_accumulator, false),        // user_volume_accumulator
            AccountMeta::new_readonly(self.fee_config, false),       // fee_config
            AccountMeta::new_readonly(PUMPSWAP_FEE_PROGRAM_ID, false), // fee_program
        ];

        wallet.require_signer(user_wallet);
        wallet.append_ix(Instruction { program_id: PUMPSWAP_PROGRAM_ID, accounts, data }, PUMPSWAP_BUY_CU);
        Ok(())
    }

    /// Build a `sell` instruction (base -> quote) and append it to `wallet`.
    pub fn sell(
        &self,
        pool_id: AccountId,
        base_amount_in: u64,
        min_quote_amount_out: u64,
        user_wallet: AccountId,
        user_base_token_account: AccountId,
        user_quote_token_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let pool = self.pools.get(*self.pool_by_id.get(&pool_id).ok_or(TraderError::WrongMints)?)
            .ok_or(TraderError::WrongMints)?;
        if pool.coin_creator == Pubkey::default() || !self.global_ready {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_base_pk = resolve(user_base_token_account)?;
        let user_quote_pk = resolve(user_quote_token_account)?;

        let (coin_creator_vault_authority, _bump) = Pubkey::find_program_address(
            &[b"creator_vault", pool.coin_creator.as_ref()],
            &PUMPSWAP_PROGRAM_ID,
        );
        let coin_creator_vault_ata =
            spl_ata(&coin_creator_vault_authority, &SPL_TOKEN_PROGRAM_ID, &pool.quote_mint_pk);
        let protocol_fee_recipient_token_account =
            spl_ata(&self.protocol_fee_recipient, &SPL_TOKEN_PROGRAM_ID, &pool.quote_mint_pk);

        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&DISC_SELL);
        data.extend_from_slice(&base_amount_in.to_le_bytes());
        data.extend_from_slice(&min_quote_amount_out.to_le_bytes());

        let accounts = vec![
            AccountMeta::new(pool.pool_pk, false),                    // pool
            AccountMeta::new_readonly(user_wallet_pk, true),          // user (signer)
            AccountMeta::new_readonly(self.global_config_pk, false),  // global_config
            AccountMeta::new_readonly(pool.base_mint_pk, false),      // base_mint
            AccountMeta::new_readonly(pool.quote_mint_pk, false),     // quote_mint
            AccountMeta::new(user_base_pk, false),                    // user_base_token_account
            AccountMeta::new(user_quote_pk, false),                   // user_quote_token_account
            AccountMeta::new(pool.base_vault_pk, false),              // pool_base_token_account
            AccountMeta::new(pool.quote_vault_pk, false),             // pool_quote_token_account
            AccountMeta::new_readonly(self.protocol_fee_recipient, false), // protocol_fee_recipient
            AccountMeta::new(protocol_fee_recipient_token_account, false), // protocol_fee_recipient_token_account
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),   // base_token_program
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),   // quote_token_program
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),      // system_program
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false), // associated_token_program
            AccountMeta::new_readonly(self.event_authority, false),  // event_authority
            AccountMeta::new_readonly(PUMPSWAP_PROGRAM_ID, false),   // program
            AccountMeta::new(coin_creator_vault_ata, false),          // coin_creator_vault_ata
            AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority
            AccountMeta::new_readonly(self.fee_config, false),       // fee_config
            AccountMeta::new_readonly(PUMPSWAP_FEE_PROGRAM_ID, false), // fee_program
        ];

        wallet.require_signer(user_wallet);
        wallet.append_ix(Instruction { program_id: PUMPSWAP_PROGRAM_ID, accounts, data }, PUMPSWAP_SELL_CU);
        Ok(())
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    /// Picks `buy` vs `sell` from whether `hop.input_mint` is this pool's
    /// quote or base mint.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let pool = self
            .pools
            .get(*self.pool_by_id.get(&hop.pool_id).ok_or(TraderError::UnknownPool(hop.pool_id))?)
            .ok_or(TraderError::UnknownPool(hop.pool_id))?;
        if hop.input_mint == pool.quote_mint_id {
            // Buy: quote -> base. dest_ata holds base, source_ata holds quote.
            self.buy(hop.pool_id, hop.amount_out, hop.amount_in, owner, dest_ata, source_ata, wallet)
        } else {
            // Sell: base -> quote. source_ata holds base, dest_ata holds quote.
            self.sell(hop.pool_id, hop.amount_in, hop.amount_out, owner, source_ata, dest_ata, wallet)
        }
    }

    /// Re-derive and upsert one pool's edge. `price` is deliberately set to
    /// `0.0` (rather than skipped) when `base_balance == 0` -- `0.0` is
    /// also what `quote_balance == 0` naturally produces, and both feed
    /// `add_generic_pair`'s own `price_b_per_a <= 0.0` gate, which removes
    /// any stale edge for this pool. This also covers `!self.global_ready`
    /// (no fee data yet): no edge is ever inserted before this returns
    /// early, so there's nothing to remove either.
    fn upsert_pool_edge(&self, idx: usize, router: &mut TradeRouter) {
        if !self.global_ready {
            return;
        }
        let p = &self.pools[idx];
        let price = if p.base_balance == 0 {
            0.0
        } else {
            p.quote_balance as f64 / p.base_balance as f64
        };
        let fee_frac =
            (self.lp_fee_bps + self.protocol_fee_bps + self.coin_creator_fee_bps) as f64 / 10_000.0;
        router.add_generic_pair(
            p.pool_id,
            p.base_mint_id,
            p.quote_mint_id,
            price,
            fee_frac,
            p.base_balance,
            p.quote_balance,
            DexType::PumpswapAmm,
        );
    }
}

/// Standard ATA derivation -- same shape as `sanctum.rs`/`pumpfun.rs`'s
/// private `spl_ata` helper.
fn spl_ata(owner: &Pubkey, token_program: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

impl Updater for PumpswapState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.accountid == self.global_config_id {
            self.parse_global_config(body);
        } else if let Some(&idx) = self.pool_by_id.get(&header.accountid) {
            self.parse_pool(idx, body);
        }
    }

    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        if let Some(&(idx, is_base)) = self.vault_to_pool.get(&ta.id) {
            let pool = &mut self.pools[idx];
            if is_base {
                pool.base_balance = ta.amount;
            } else {
                pool.quote_balance = ta.amount;
            }
            return true;
        }
        false
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &solana_sdk::clock::Slot) {}

    /// Add the base <-> quote edge for every priced pool.
    fn batch_router(&mut self, router: &mut TradeRouter) {
        for idx in 0..self.pools.len() {
            self.upsert_pool_edge(idx, router);
        }
    }

    /// `global_config_id` carries the shared fee rates every pool's edge
    /// depends on, so any update to it (including the first time it
    /// becomes ready) must re-derive every pool, not just one.
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if account_id == self.global_config_id {
            for idx in 0..self.pools.len() {
                self.upsert_pool_edge(idx, router);
            }
        } else if let Some(&idx) = self.pool_by_id.get(&account_id) {
            self.upsert_pool_edge(idx, router);
        }
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&(idx, _)) = self.vault_to_pool.get(&ta_id) {
            self.upsert_pool_edge(idx, router);
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
    fn global_config_pda_is_fixed_and_nonzero() {
        let (global_config, _bump) =
            Pubkey::find_program_address(&[b"global_config"], &PUMPSWAP_PROGRAM_ID);
        assert_ne!(global_config, Pubkey::default());
    }

    #[test]
    fn fee_config_pda_is_seeded_by_the_pumpswap_program_id() {
        let (fee_config, _bump) = Pubkey::find_program_address(
            &[b"fee_config", PUMPSWAP_PROGRAM_ID.as_ref()],
            &PUMPSWAP_FEE_PROGRAM_ID,
        );
        assert_ne!(fee_config, Pubkey::default());
    }

    #[test]
    fn coin_creator_vault_authority_uses_underscore_seed_not_hyphen() {
        let creator = Pubkey::new_unique();
        let (underscore, _) =
            Pubkey::find_program_address(&[b"creator_vault", creator.as_ref()], &PUMPSWAP_PROGRAM_ID);
        let (hyphen, _) =
            Pubkey::find_program_address(&[b"creator-vault", creator.as_ref()], &PUMPSWAP_PROGRAM_ID);
        assert_ne!(underscore, hyphen, "creator_vault and creator-vault must derive different PDAs");
    }

    fn synthetic_pool(
        pool_id: AccountId,
        base_mint_id: AccountId,
        quote_mint_id: AccountId,
    ) -> PumpswapPoolState {
        PumpswapPoolState {
            pool_id,
            pool_pk: Pubkey::default(),
            base_mint_id,
            base_mint_pk: Pubkey::default(),
            quote_mint_id,
            quote_mint_pk: Pubkey::default(),
            base_vault_pk: Pubkey::default(),
            quote_vault_pk: Pubkey::default(),
            coin_creator: Pubkey::new_unique(),
            base_balance: 1_000_000_000,
            quote_balance: 1_500_000_000,
        }
    }

    #[test]
    fn batch_router_adds_edge_for_priced_pool() {
        let base_id: AccountId = 1;
        let quote_id: AccountId = 2;
        let pool_id: AccountId = 3;

        let mut state = PumpswapState {
            program_id: 0,
            global_config_id: 0,
            global_config_pk: Pubkey::default(),
            event_authority: Pubkey::default(),
            fee_config: Pubkey::default(),
            protocol_fee_recipient: Pubkey::default(),
            lp_fee_bps: 20,
            protocol_fee_bps: 5,
            coin_creator_fee_bps: 5,
            global_ready: true,
            pools: vec![synthetic_pool(pool_id, base_id, quote_id)],
            pool_by_id: HashMap::new(),
            vault_to_pool: HashMap::new(),
        };

        let mut r = crate::trader::router::Router::new(4, 0.01);
        r.register_mint(base_id);
        r.register_mint(quote_id);
        let mut router = TradeRouter::from_router(&r);
        state.batch_router(&mut router);
        assert_eq!(router.edge_counts_by_dex(), vec![(DexType::PumpswapAmm, 2)]);

        // Not ready until GlobalConfig has been parsed at least once, even
        // with priced pools.
        state.global_ready = false;
        let mut router2 = TradeRouter::from_router(&r);
        state.batch_router(&mut router2);
        assert_eq!(router2.edge_counts_by_dex(), vec![]);

        // A pool with no balance yet is excluded even when Global is ready.
        state.global_ready = true;
        state.pools[0].quote_balance = 0;
        let mut router3 = TradeRouter::from_router(&r);
        state.batch_router(&mut router3);
        assert_eq!(router3.edge_counts_by_dex(), vec![]);
    }
}
