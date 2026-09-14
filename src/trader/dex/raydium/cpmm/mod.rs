//! Raydium CPMM (constant-product market maker) parser and swap builder.
//!
//! # Account layout — Raydium CPMM PoolState (Anchor)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ─────────────────────────────────────────
//!    0       8   Anchor discriminator
//!    8      32   amm_config (Pubkey)
//!   40      32   pool_creator (Pubkey)
//!   72      32   token_0_vault (Pubkey)
//!  104      32   token_1_vault (Pubkey)
//!  136      32   lp_mint (Pubkey)
//!  168      32   token_0_mint (Pubkey)
//!  200      32   token_1_mint (Pubkey)
//!  232      32   token_0_program (Pubkey)
//!  264      32   token_1_program (Pubkey)
//!  296      32   observation_key (Pubkey)
//!  328       1   auth_bump (u8)
//!  329       1   status (u8)
//!  330       1   lp_mint_decimals (u8)
//!  331       1   mint_0_decimals (u8)
//!  332       1   mint_1_decimals (u8)
//! ```

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    trader::{
        dex::update::Updater,
        pricegraph::{cp_quote, Hop, TradeRouter},
        types::{DexType, PoolPrice, SwapParams, TraderError, RAYDIUM_HOP_MAX_SLIPPAGE},
    },
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const RAYDIUM_CPMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

// ─── Anchor discriminator ─────────────────────────────────────────────────────

pub const DISC_POOL_STATE: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

/// Anchor discriminator for `swap_base_input`: sha256("global:swap_base_input")[0..8].
pub const SWAP_BASE_INPUT_DISC: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];

// ─── Pool state offsets (absolute, including 8-byte discriminator) ────────────

const OFF_AMM_CONFIG: usize = 8;
const OFF_VAULT_0: usize = 72;
const OFF_VAULT_1: usize = 104;
const OFF_MINT_0: usize = 168;
const OFF_MINT_1: usize = 200;
const OFF_TOKEN_0_PROGRAM: usize = 232;
const OFF_TOKEN_1_PROGRAM: usize = 264;
const OFF_OBSERVATION_KEY: usize = 296;
const OFF_DECIMALS_0: usize = 331;
const OFF_DECIMALS_1: usize = 332;

const MIN_POOL_LEN: usize = OFF_DECIMALS_1 + 1; // 333

#[derive(Debug, Default)]
pub struct RaydiumCpmm {
    program_id: AccountId,
    m_pool: HashMap<AccountId, RaydiumCpmmPoolWrapper, BuildHasherDefault<XxHash64>>,
    /// vault account -> the pool it belongs to. Populated as vaults are
    /// discovered from parsed pool accounts (not known up front -- unlike
    /// the pool pubkeys themselves, `RaydiumCpmmPoolSetup` doesn't carry
    /// vault addresses).
    m_vault: HashMap<AccountId, AccountId, BuildHasherDefault<XxHash64>>,
}

impl RaydiumCpmm {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `RaydiumAmm::new`'s doc comment for why (paced through a shared
    /// [`crate::graph::SubscriptionQueue`] owned by `DexState` instead).
    pub fn new(setups: &[RaydiumCpmmPoolSetup]) -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&RAYDIUM_CPMM_PROGRAM_ID);
        let mut m_pool =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(setups.len());
        for setup in setups {
            l_req.push(SubscriptionRequest {
                root: setup.pubkey,
                filter_weight: u32::MAX,
                depth: 1,
            });
            m_pool.insert(
                setup.pubkey,
                RaydiumCpmmPoolWrapper {
                    pool: RaydiumCpmmPool {
                        token_mint_0: setup.mint_0,
                        token_mint_1: setup.mint_1,
                        ..RaydiumCpmmPool::default()
                    },
                    vault_0_balance: 0,
                    vault_1_balance: 0,
                    trade_fee_rate: setup.trade_fee_rate,
                },
            );
        }
        let state = Self {
            program_id,
            m_pool,
            m_vault: HashMap::with_capacity_and_hasher(
                setups.len() * 2,
                BuildHasherDefault::default(),
            ),
        };
        (state, l_req)
    }
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }
    pub fn pool_count(&self) -> usize {
        self.m_pool.len()
    }

    pub fn populate_router(&self, router: &mut TradeRouter) {
        for &pool_id in self.m_pool.keys() {
            self.upsert_pool_edge(pool_id, router);
        }
    }

    /// Re-derive and upsert one pool's router edge(s) -- `add_generic_pair`'s
    /// own `price_b_per_a <= 0.0` gate already removes any stale edge when
    /// `reserve_0`/`reserve_1` is zero (`spot_price()` returns `0.0` in
    /// that case), so no separate reserve check is needed here.
    fn upsert_pool_edge(&self, pool_id: AccountId, router: &mut TradeRouter) {
        let Some(wrapper) = self.m_pool.get(&pool_id) else {
            return;
        };
        let state = &wrapper.pool;
        let fee_frac = wrapper.trade_fee_rate as f64 / 1_000_000.0;
        router.add_generic_pair(
            pool_id,
            state.token_mint_0,
            state.token_mint_1,
            state.spot_price(),
            fee_frac,
            state.reserve_0,
            state.reserve_1,
            DexType::RaydiumCpmm,
        );
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let wrapper = self.m_pool.get(&hop.pool_id).ok_or(TraderError::UnknownPool(hop.pool_id))?;
        let mut params = SwapParams {
            pool: hop.pool_id,
            input_mint: hop.input_mint,
            output_mint: hop.output_mint,
            amount_in: hop.amount_in,
            min_amount_out: hop.amount_out,
            user_source_token_account: source_ata,
            user_destination_token_account: dest_ata,
            user_wallet: owner,
        };
        // See SwapParams::apply_slippage_tolerance's doc comment -- without
        // this, real on-chain price movement between quote and landing
        // reliably trips the pool's own minimum-output check.
        params.apply_slippage_tolerance(RAYDIUM_HOP_MAX_SLIPPAGE);
        build_swap_ix(hop.pool_id, &wrapper.pool, &params, wallet)
    }
}

impl Updater for RaydiumCpmm {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.owner != self.program_id {
            return;
        }
        let Some(wrapper) = self.m_pool.get_mut(&header.accountid) else {
            return;
        };
        let Some(mut new_state) = parse(body) else {
            return;
        };

        // Preserve live reserves from token events -- parse() always
        // zeroes them since they don't live in the account bytes.
        new_state.reserve_0 = wrapper.pool.reserve_0;
        new_state.reserve_1 = wrapper.pool.reserve_1;

        let (old_v0, old_v1) = (wrapper.pool.token_vault_0, wrapper.pool.token_vault_1);
        let (vault_0, vault_1) = (new_state.token_vault_0, new_state.token_vault_1);
        let pool_id = header.accountid;
        wrapper.pool = new_state;

        if old_v0 == 0 && vault_0 != 0 {
            self.m_vault.insert(vault_0, pool_id);
        }
        if old_v1 == 0 && vault_1 != 0 {
            self.m_vault.insert(vault_1, pool_id);
        }
    }

    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        let Some(&pool_id) = self.m_vault.get(&ta.id) else {
            return false;
        };
        let Some(wrapper) = self.m_pool.get_mut(&pool_id) else {
            return false;
        };
        if ta.id == wrapper.pool.token_vault_0 {
            wrapper.pool.reserve_0 = ta.amount;
        } else if ta.id == wrapper.pool.token_vault_1 {
            wrapper.pool.reserve_1 = ta.amount;
        }
        true
    }

    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.populate_router(router);
    }

    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        self.upsert_pool_edge(account_id, router);
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&pool_id) = self.m_vault.get(&ta_id) {
            self.upsert_pool_edge(pool_id, router);
        }
    }

    fn on_tx(
        &mut self,
        ix: &crate::txview::CatscopeInstructionRead<'_>,
        _slot: &solana_sdk::clock::Slot,
    ) {
        // Decode a landed swap_base_input instruction and provisionally
        // ("fudge") adjust the pool's reserves ahead of the real account
        // update that will arrive ~50ms later. Safe to mutate in place:
        // reserve_0/reserve_1 are the exact same fields on_token writes,
        // so the next real vault-balance update overwrites this estimate
        // with ground truth -- no separate expiry needed.
        if *ix.program() != self.program_id {
            return;
        }
        let data = ix.data();
        let accounts = ix.account();
        if data.len() < 24 || data[..8] != SWAP_BASE_INPUT_DISC || accounts.len() <= 7 {
            return;
        }
        let amount_in = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let pool_id = accounts[3];
        let input_vault = accounts[6];
        let Some(wrapper) = self.m_pool.get_mut(&pool_id) else {
            return;
        };
        let fee_bps = (wrapper.trade_fee_rate / 100) as u16;
        let pool = &mut wrapper.pool;
        let zero_for_one = if input_vault == pool.token_vault_0 {
            true
        } else if input_vault == pool.token_vault_1 {
            false
        } else {
            return;
        };
        let (reserve_in, reserve_out) = if zero_for_one {
            (pool.reserve_0, pool.reserve_1)
        } else {
            (pool.reserve_1, pool.reserve_0)
        };
        if reserve_in == 0 || reserve_out == 0 {
            return;
        }
        let amount_out = cp_quote(amount_in, reserve_in, reserve_out, fee_bps);
        let new_in = reserve_in.saturating_sub(amount_in);
        let new_out = reserve_out.saturating_add(amount_out);
        if zero_for_one {
            pool.reserve_0 = new_in;
            pool.reserve_1 = new_out;
        } else {
            pool.reserve_1 = new_in;
            pool.reserve_0 = new_out;
        }
    }

    fn flush_pool(&mut self, _graph: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        // Not currently called by DexState::flush_pool (only Orca is).
        Ok(())
    }
}

#[derive(Debug)]
struct RaydiumCpmmPoolWrapper {
    pub pool: RaydiumCpmmPool,
    pub vault_0_balance: u64,
    pub vault_1_balance: u64,
    pub trade_fee_rate: u64,
}
// ─── Parsed pool state ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct RaydiumCpmmPool {
    pub token_mint_0: AccountId,
    pub token_mint_1: AccountId,
    pub token_vault_0: AccountId,
    pub token_vault_1: AccountId,
    pub amm_config: AccountId,
    pub observation_key: AccountId,
    pub token_0_program: AccountId,
    pub token_1_program: AccountId,
    pub mint_0_decimals: u8,
    pub mint_1_decimals: u8,
    /// Token vault 0 balance (raw). Updated via token events.
    pub reserve_0: u64,
    /// Token vault 1 balance (raw). Updated via token events.
    pub reserve_1: u64,
}

impl RaydiumCpmmPool {
    /// Spot price: raw token_1 units per raw token_0 unit (from vault reserves).
    pub fn spot_price(&self) -> f64 {
        if self.reserve_0 == 0 {
            return 0.0;
        }
        self.reserve_1 as f64 / self.reserve_0 as f64
    }

    pub fn pool_price(&self, trade_fee_rate: u64) -> Option<PoolPrice> {
        if self.reserve_0 == 0 || self.reserve_1 == 0 {
            return None;
        }
        let fee_bps = (trade_fee_rate / 100).min(10_000) as u16;
        Some(PoolPrice {
            token_a: self.token_mint_0,
            token_b: self.token_mint_1,
            price: self.spot_price(),
            reserve_a: self.reserve_0,
            reserve_b: self.reserve_1,
            fee_bps,
        })
    }
}

// ─── Setup loaded from raydium_cpmm.json ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RaydiumCpmmPoolSetup {
    pub pubkey: AccountId,
    pub mint_0: AccountId,
    pub mint_1: AccountId,
    pub trade_fee_rate: u64,
}

// ─── Parser ───────────────────────────────────────────────────────────────────

pub fn parse(body: &[u8]) -> Option<RaydiumCpmmPool> {
    if body.len() < MIN_POOL_LEN {
        return None;
    }
    if body[..8] != DISC_POOL_STATE {
        return None;
    }
    let read_u8 = |off: usize| body[off];
    let read_pk = |off: usize| Pubkey::new_from_array(body[off..off + 32].try_into().unwrap());
    let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);

    Some(RaydiumCpmmPool {
        amm_config: pk_id(read_pk(OFF_AMM_CONFIG)),
        token_vault_0: pk_id(read_pk(OFF_VAULT_0)),
        token_vault_1: pk_id(read_pk(OFF_VAULT_1)),
        token_mint_0: pk_id(read_pk(OFF_MINT_0)),
        token_mint_1: pk_id(read_pk(OFF_MINT_1)),
        token_0_program: pk_id(read_pk(OFF_TOKEN_0_PROGRAM)),
        token_1_program: pk_id(read_pk(OFF_TOKEN_1_PROGRAM)),
        observation_key: pk_id(read_pk(OFF_OBSERVATION_KEY)),
        mint_0_decimals: read_u8(OFF_DECIMALS_0),
        mint_1_decimals: read_u8(OFF_DECIMALS_1),
        reserve_0: 0,
        reserve_1: 0,
    })
}

// ─── Swap instruction ─────────────────────────────────────────────────────────

pub const RAYDIUM_CPMM_SWAP_CU: u32 = 200_000;

/// Derive the CPMM vault authority PDA (global, same for all pools).
pub fn cpmm_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"vault_and_lp_mint_auth_seed"], &RAYDIUM_CPMM_PROGRAM_ID).0
}

pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &RaydiumCpmmPool,
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let zero_for_one = if params.input_mint == pool.token_mint_0
        && params.output_mint == pool.token_mint_1
    {
        true
    } else if params.input_mint == pool.token_mint_1 && params.output_mint == pool.token_mint_0 {
        false
    } else {
        return Err(TraderError::WrongMints);
    };

    let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
        pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
    };

    let pool_pk = resolve(pool_id)?;
    let amm_config_pk = resolve(pool.amm_config)?;
    let observation_pk = resolve(pool.observation_key)?;
    let vault_0_pk = resolve(pool.token_vault_0)?;
    let vault_1_pk = resolve(pool.token_vault_1)?;
    let mint_0_pk = resolve(pool.token_mint_0)?;
    let mint_1_pk = resolve(pool.token_mint_1)?;
    let user_source_pk = resolve(params.user_source_token_account)?;
    let user_dest_pk = resolve(params.user_destination_token_account)?;
    let user_wallet_pk = resolve(params.user_wallet)?;

    // Resolve per-mint token programs (stored in pool state)
    let tok_prog_0 = pubkey_from_account_id(&pool.token_0_program).unwrap_or(SPL_TOKEN_PROGRAM_ID);
    let tok_prog_1 = pubkey_from_account_id(&pool.token_1_program).unwrap_or(SPL_TOKEN_PROGRAM_ID);

    let (input_vault_pk, output_vault_pk, input_mint_pk, output_mint_pk, in_prog, out_prog) =
        if zero_for_one {
            (
                vault_0_pk, vault_1_pk, mint_0_pk, mint_1_pk, tok_prog_0, tok_prog_1,
            )
        } else {
            (
                vault_1_pk, vault_0_pk, mint_1_pk, mint_0_pk, tok_prog_1, tok_prog_0,
            )
        };

    let authority_pk = cpmm_authority();

    // swap_base_input: disc(8) + amount_in(8) + minimum_amount_out(8) = 24 bytes
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&SWAP_BASE_INPUT_DISC);
    data.extend_from_slice(&params.amount_in.to_le_bytes());
    data.extend_from_slice(&params.min_amount_out.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(user_wallet_pk, true),           // payer
        AccountMeta::new_readonly(authority_pk, false),   // authority
        AccountMeta::new_readonly(amm_config_pk, false),  // amm_config
        AccountMeta::new(pool_pk, false),                 // pool_state
        AccountMeta::new(user_source_pk, false),          // input_token_account
        AccountMeta::new(user_dest_pk, false),            // output_token_account
        AccountMeta::new(input_vault_pk, false),          // input_vault
        AccountMeta::new(output_vault_pk, false),         // output_vault
        AccountMeta::new_readonly(in_prog, false),        // input_token_program
        AccountMeta::new_readonly(out_prog, false),       // output_token_program
        AccountMeta::new_readonly(input_mint_pk, false),  // input_token_mint
        AccountMeta::new_readonly(output_mint_pk, false), // output_token_mint
        AccountMeta::new(observation_pk, false),          // observation_state
    ];

    wallet.require_signer(params.user_wallet);
    wallet.append_ix(
        Instruction {
            program_id: RAYDIUM_CPMM_PROGRAM_ID,
            accounts,
            data,
        },
        RAYDIUM_CPMM_SWAP_CU,
    );
    Ok(())
}

