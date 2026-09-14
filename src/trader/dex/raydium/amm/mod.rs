//! Raydium AMM v4 (constant-product, OpenBook-backed) parser and swap builder.
//!
//! # Account layout — Raydium AMM v4 pool state (no Anchor discriminator)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ─────────────────────────────────────────
//!    0       8   status (u64)
//!    8       8   nonce (u64) — used to derive amm_authority PDA
//!   32       8   coin_decimals (u64)
//!   40       8   pc_decimals (u64)
//!  176       8   swap_fee_numerator (u64)
//!  184       8   swap_fee_denominator (u64)
//!  336      32   coin_vault (Pubkey)
//!  368      32   pc_vault (Pubkey)
//!  400      32   coin_mint (Pubkey)
//!  432      32   pc_mint (Pubkey)
//!  496      32   open_orders (Pubkey)
//!  528      32   market (Pubkey)
//!  560      32   market_program (Pubkey)
//!  592      32   target_orders (Pubkey)
//! ```

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    trader::{
        dex::update::Updater,
        pricegraph::{Hop, TradeRouter},
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

pub const RAYDIUM_AMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

pub const RAYDIUM_AMM_AUTHORITY: Pubkey =
    Pubkey::from_str_const("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ─── Pool state offsets ───────────────────────────────────────────────────────

const OFF_SWAP_FEE_NUM: usize = 176;
const OFF_SWAP_FEE_DEN: usize = 184;
const OFF_COIN_VAULT: usize = 336;
const OFF_PC_VAULT: usize = 368;
const OFF_COIN_MINT: usize = 400;
const OFF_PC_MINT: usize = 432;
const OFF_OPEN_ORDERS: usize = 496;
const OFF_MARKET_ID: usize = 528;
const OFF_MARKET_PROGRAM: usize = 560;
const OFF_TARGET_ORDERS: usize = 592;
const MIN_POOL_STATE_LEN: usize = OFF_TARGET_ORDERS + 32;

struct RaydiumAmmPoolWrapper {
    pool: RaydiumAmmPool,
    /// Retained from `RaydiumAmmSetup` at construction time -- `build_swap_ix`
    /// needs the pool's OpenBook market accounts, which never appear in the
    /// pool account's own data (see `RaydiumAmmSwapConfig`'s doc comment).
    swap_config: RaydiumAmmSwapConfig,
}

// ─── Parsed pool state ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RaydiumAmmPool {
    pub coin_mint: AccountId,
    pub pc_mint: AccountId,
    pub coin_vault: AccountId,
    pub pc_vault: AccountId,
    pub open_orders: AccountId,
    pub target_orders: AccountId,
    pub market_id: AccountId,
    pub market_program: AccountId,
    pub swap_fee_numerator: u64,
    pub swap_fee_denominator: u64,
    pub reserve_coin: u64,
    pub reserve_pc: u64,
}

impl RaydiumAmmPool {
    pub fn fee_bps(&self) -> u16 {
        if self.swap_fee_denominator == 0 {
            return 0;
        }
        ((self.swap_fee_numerator * 10_000 / self.swap_fee_denominator) as u16).min(10_000)
    }

    pub fn pool_price(&self) -> Option<PoolPrice> {
        if self.reserve_coin == 0 || self.reserve_pc == 0 {
            return None;
        }
        Some(PoolPrice {
            token_a: self.coin_mint,
            token_b: self.pc_mint,
            price: self.reserve_pc as f64 / self.reserve_coin as f64,
            reserve_a: self.reserve_coin,
            reserve_b: self.reserve_pc,
            fee_bps: self.fee_bps(),
        })
    }
}

/// Static config loaded from `raydium_amm.json` at build time.
#[derive(Debug, Clone)]
pub struct RaydiumAmmSetup {
    pub pubkey: AccountId,
    pub swap_config: RaydiumAmmSwapConfig,
}

/// OpenBook/Serum market accounts required to build a swap instruction.
#[derive(Debug, Clone)]
pub struct RaydiumAmmSwapConfig {
    pub market_bids: AccountId,
    pub market_asks: AccountId,
    pub market_event_queue: AccountId,
    pub market_coin_vault: AccountId,
    pub market_pc_vault: AccountId,
    pub market_vault_signer: AccountId,
}

// ─── Parser ───────────────────────────────────────────────────────────────────

pub fn parse(body: &[u8]) -> Option<RaydiumAmmPool> {
    if body.len() < MIN_POOL_STATE_LEN {
        return None;
    }
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_pk = |off: usize| Pubkey::new_from_array(body[off..off + 32].try_into().unwrap());
    let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);
    Some(RaydiumAmmPool {
        coin_mint: pk_id(read_pk(OFF_COIN_MINT)),
        pc_mint: pk_id(read_pk(OFF_PC_MINT)),
        coin_vault: pk_id(read_pk(OFF_COIN_VAULT)),
        pc_vault: pk_id(read_pk(OFF_PC_VAULT)),
        open_orders: pk_id(read_pk(OFF_OPEN_ORDERS)),
        target_orders: pk_id(read_pk(OFF_TARGET_ORDERS)),
        market_id: pk_id(read_pk(OFF_MARKET_ID)),
        market_program: pk_id(read_pk(OFF_MARKET_PROGRAM)),
        swap_fee_numerator: read_u64(OFF_SWAP_FEE_NUM),
        swap_fee_denominator: read_u64(OFF_SWAP_FEE_DEN),
        reserve_coin: 0,
        reserve_pc: 0,
    })
}

// ─── Swap instruction ─────────────────────────────────────────────────────────

pub const RAYDIUM_AMM_SWAP_CU: u32 = 200_000;

pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &RaydiumAmmPool,
    swap_cfg: &RaydiumAmmSwapConfig,
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let (a_to_b, _) = if params.input_mint == pool.coin_mint && params.output_mint == pool.pc_mint {
        (true, ())
    } else if params.input_mint == pool.pc_mint && params.output_mint == pool.coin_mint {
        (false, ())
    } else {
        return Err(TraderError::WrongMints);
    };
    let _ = a_to_b;

    let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
        pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
    };

    let pool_pk = resolve(pool_id)?;
    let open_orders_pk = resolve(pool.open_orders)?;
    let target_orders_pk = resolve(pool.target_orders)?;
    let coin_vault_pk = resolve(pool.coin_vault)?;
    let pc_vault_pk = resolve(pool.pc_vault)?;
    let market_program_pk = resolve(pool.market_program)?;
    let market_id_pk = resolve(pool.market_id)?;
    let market_bids_pk = resolve(swap_cfg.market_bids)?;
    let market_asks_pk = resolve(swap_cfg.market_asks)?;
    let market_evq_pk = resolve(swap_cfg.market_event_queue)?;
    let market_cv_pk = resolve(swap_cfg.market_coin_vault)?;
    let market_pv_pk = resolve(swap_cfg.market_pc_vault)?;
    let market_vs_pk = resolve(swap_cfg.market_vault_signer)?;
    let user_source_pk = resolve(params.user_source_token_account)?;
    let user_dest_pk = resolve(params.user_destination_token_account)?;
    let user_wallet_pk = resolve(params.user_wallet)?;

    let mut data = [0u8; 17];
    data[0] = 9; // SwapBaseIn
    data[1..9].copy_from_slice(&params.amount_in.to_le_bytes());
    data[9..17].copy_from_slice(&params.min_amount_out.to_le_bytes());

    let accounts = vec![
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        AccountMeta::new(pool_pk, false),
        AccountMeta::new_readonly(RAYDIUM_AMM_AUTHORITY, false),
        AccountMeta::new(open_orders_pk, false),
        AccountMeta::new(target_orders_pk, false),
        AccountMeta::new(coin_vault_pk, false),
        AccountMeta::new(pc_vault_pk, false),
        AccountMeta::new_readonly(market_program_pk, false),
        AccountMeta::new(market_id_pk, false),
        AccountMeta::new(market_bids_pk, false),
        AccountMeta::new(market_asks_pk, false),
        AccountMeta::new(market_evq_pk, false),
        AccountMeta::new(market_cv_pk, false),
        AccountMeta::new(market_pv_pk, false),
        AccountMeta::new_readonly(market_vs_pk, false),
        AccountMeta::new(user_source_pk, false),
        AccountMeta::new(user_dest_pk, false),
        AccountMeta::new_readonly(user_wallet_pk, true),
    ];

    wallet.require_signer(params.user_wallet);
    wallet.append_ix(
        Instruction {
            program_id: RAYDIUM_AMM_PROGRAM_ID,
            accounts,
            data: data.to_vec(),
        },
        RAYDIUM_AMM_SWAP_CU,
    );
    Ok(())
}

// ─── Live state tracker ───────────────────────────────────────────────────────

pub struct RaydiumAmm {
    program_id: AccountId,
    m_pool: HashMap<AccountId, RaydiumAmmPoolWrapper, BuildHasherDefault<XxHash64>>,
    /// map token account to pool
    m_vault: HashMap<AccountId, AccountId, BuildHasherDefault<XxHash64>>,
}

impl std::fmt::Debug for RaydiumAmm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaydiumAmmState")
            .field("pool_count", &self.m_pool.len())
            .finish()
    }
}

impl RaydiumAmm {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. Paired with a
    /// shared [`crate::graph::SubscriptionQueue`] owned by `DexState`,
    /// which paces the actual `bulk_subscribe` calls across many slots
    /// instead of one blocking call for this dex's own (real,
    /// live-observed) ~10,000-pool startup burst. See
    /// `SubscriptionQueue`'s own doc comment.
    pub fn new(setups: &[RaydiumAmmSetup]) -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&RAYDIUM_AMM_PROGRAM_ID);

        let mut m_pool =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let m_vault =
            HashMap::with_capacity_and_hasher(2 * setups.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(setups.len());
        for setup in setups {
            l_req.push(SubscriptionRequest {
                root: setup.pubkey,
                filter_weight: u32::MAX,
                depth: 1,
            });

            let pool = RaydiumAmmPool {
                coin_mint: 0,
                pc_mint: 0,
                // Real values, unknown until the pool account itself is
                // read (see on_account) -- NOT setup.swap_config's market
                // vaults, which belong to the OpenBook market, not this
                // AMM pool, and were never a valid reserve-balance proxy.
                coin_vault: 0,
                pc_vault: 0,
                open_orders: 0,
                target_orders: 0,
                market_id: 0,
                market_program: 0,
                swap_fee_numerator: 0,
                swap_fee_denominator: 0,
                reserve_coin: 0,
                reserve_pc: 0,
            };
            m_pool.insert(
                setup.pubkey,
                RaydiumAmmPoolWrapper { pool, swap_config: setup.swap_config.clone() },
            );
        }
        (Self { program_id, m_pool, m_vault }, l_req)
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn pool_count(&self) -> usize {
        self.m_pool.len()
    }

    pub fn populate_router(&self, router: &mut crate::trader::pricegraph::TradeRouter) {
        for (&pool_id, state) in &self.m_pool {
            router.add_raydium_amm_pool(pool_id, &state.pool, DexType::RaydiumAmm);
        }
    }

    /// Build the swap instruction for one `Hop` routed through this dex,
    /// using already-resolved token accounts, and append it to `wallet` --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    /// Just wraps `build_swap_ix` unchanged with a `m_pool` lookup.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let state = self.m_pool.get(&hop.pool_id).ok_or(TraderError::UnknownPool(hop.pool_id))?;
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
        build_swap_ix(hop.pool_id, &state.pool, &state.swap_config, &params, wallet)
    }
}
impl Updater for RaydiumAmm {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.owner != self.program_id {
            return;
        }
        let new_state = match self.m_pool.get(&header.accountid) {
            None => return,
            Some(_) => match parse(body) {
                None => return,
                Some(s) => s,
            },
        };

        let pool_id = header.accountid;
        let (old_cv, old_pv, new_cv, new_pv) = {
            let s = match self.m_pool.get_mut(&pool_id) {
                Some(x) => x,
                None => return,
            };
            let rc = s.pool.reserve_coin;
            let rp = s.pool.reserve_pc;
            let (ocv, opv) = (s.pool.coin_vault, s.pool.pc_vault);
            let (ncv, npv) = (new_state.coin_vault, new_state.pc_vault);
            s.pool = new_state;
            s.pool.reserve_coin = rc;
            s.pool.reserve_pc = rp;
            (ocv, opv, ncv, npv)
        };
        // Register the pool's real vaults the moment they're learned (only
        // knowable once the pool account itself has been parsed) so
        // on_token can route their balance updates back to this pool --
        // same pattern as raydium::clmm/cpmm's on_account.
        if old_cv == 0 && new_cv != 0 {
            self.m_vault.insert(new_cv, pool_id);
        }
        if old_pv == 0 && new_pv != 0 {
            self.m_vault.insert(new_pv, pool_id);
        }
    }
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        if let Some(&pool_id) = self.m_vault.get(&ta.id) {
            if let Some(s) = self.m_pool.get_mut(&pool_id) {
                if ta.id == s.pool.coin_vault {
                    s.pool.reserve_coin = ta.amount;
                } else if ta.id == s.pool.pc_vault {
                    s.pool.reserve_pc = ta.amount;
                }
                return true;
            }
        }
        false
    }
    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.populate_router(router);
    }

    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if let Some(state) = self.m_pool.get(&account_id) {
            router.add_raydium_amm_pool(account_id, &state.pool, DexType::RaydiumAmm);
        }
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&pool_id) = self.m_vault.get(&ta_id) {
            if let Some(state) = self.m_pool.get(&pool_id) {
                router.add_raydium_amm_pool(pool_id, &state.pool, DexType::RaydiumAmm);
            }
        }
    }

    fn on_tx(
        &mut self,
        _ix: &crate::txview::CatscopeInstructionRead<'_>,
        _slot: &solana_sdk::clock::Slot,
    ) {
        // Deliberately not decoded, unlike CLMM/CPMM/Orca's on_tx: a
        // SwapBaseIn instruction always carries both coin_vault and
        // pc_vault at fixed account positions regardless of direction --
        // the on-chain program infers direction from the *mint* of the
        // user's source token account, which isn't something we can look
        // up for an arbitrary third party's ATA. Fudging reserves without
        // knowing which side is being sold would be as likely to move the
        // price the wrong way as the right one, so this is left as a
        // documented gap rather than a guess.
    }

    fn flush_pool(&mut self, _graph: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        // Not currently called by DexState::flush_pool (only Orca is).
        Ok(())
    }
}
