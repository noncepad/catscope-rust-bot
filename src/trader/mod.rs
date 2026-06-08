//! Multi-DEX trading interface for Raydium, Orca, and Kamino.
//!
//! # Quick start
//!
//! ```rust,ignore
//! let mut trader = DexTrader::new();
//!
//! // Register the pools you want to trade through.
//! trader.register_raydium_amm(pool_account_id, Some(serum_cfg));
//! trader.register_orca_whirlpool(whirlpool_account_id);
//! trader.register_kamino_reserve(reserve_account_id);
//!
//! // Forward every on-chain account update you receive from the host.
//! if let Some(update) = trader.on_account(header, body) { /* price changed */ }
//!
//! // Forward SPL token-account events for vault reserve tracking.
//! if let Some(update) = trader.on_token(&tok) { /* reserves changed */ }
//!
//! // Get current price snapshot.
//! if let Some(p) = trader.price(pool_id) { println!("{}", p.price); }
//!
//! // Build and queue a swap instruction.
//! let params = SwapParams { pool: pool_id, input_mint, output_mint, amount_in,
//!                           min_amount_out, user_source_token_account,
//!                           user_destination_token_account, user_wallet };
//! trader.sell(&params, &mut wallet)?;
//! ```

pub mod arb;
pub mod dex;
pub mod portfolio;
pub mod pricegraph;
pub mod types;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    graph::AccountId,
    trader::{
        dex::{
            kamino::KaminoReserve,
            orca::OrcaWhirlpool,
            raydium::{RaydiumAmmPool, RaydiumAmmSwapConfig},
            DexPool,
        },
        types::{DexType, PoolPrice, PriceUpdate, SwapParams, TraderError},
    },
    wallet::Wallet,
};
use std::collections::HashMap;

/// Multi-DEX trading object.
///
/// Register pools once at startup, then feed it account updates via
/// [`on_account`](Self::on_account) and [`on_token`](Self::on_token).
/// Call [`buy`](Self::buy) / [`sell`](Self::sell) to append swap instructions
/// to a [`Wallet`].
pub struct DexTrader {
    /// Pool account ID → parsed DEX pool state.
    pools: HashMap<AccountId, DexPool>,
    /// Vault account ID → owning pool account ID.
    /// Routes `on_token` updates to the correct pool.
    vault_to_pool: HashMap<AccountId, AccountId>,
}

impl DexTrader {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
            vault_to_pool: HashMap::new(),
        }
    }

    // ─── Registration ─────────────────────────────────────────────────────────

    /// Register a Raydium AMM v4 pool.
    ///
    /// `swap_config` holds the OpenBook market accounts required to build swap
    /// instructions. Pass `None` for price-only use; supply later via
    /// [`set_raydium_swap_config`](Self::set_raydium_swap_config).
    pub fn register_raydium_amm(
        &mut self,
        pool_id: AccountId,
        swap_config: Option<RaydiumAmmSwapConfig>,
    ) {
        let placeholder = RaydiumAmmPool {
            coin_mint: 0,
            pc_mint: 0,
            coin_vault: 0,
            pc_vault: 0,
            open_orders: 0,
            target_orders: 0,
            market_id: 0,
            market_program: 0,
            swap_fee_numerator: 25,
            swap_fee_denominator: 10_000,
            reserve_coin: 0,
            reserve_pc: 0,
        };
        self.pools.insert(
            pool_id,
            DexPool::RaydiumAmm {
                state: placeholder,
                swap_config,
            },
        );
    }

    /// Supply or replace the OpenBook market accounts for a Raydium AMM pool.
    pub fn set_raydium_swap_config(&mut self, pool_id: AccountId, cfg: RaydiumAmmSwapConfig) {
        if let Some(DexPool::RaydiumAmm { swap_config, .. }) = self.pools.get_mut(&pool_id) {
            *swap_config = Some(cfg);
        }
    }

    /// Register an Orca Whirlpool pool.
    pub fn register_orca_whirlpool(&mut self, pool_id: AccountId) {
        let placeholder = OrcaWhirlpool {
            token_mint_a: 0,
            token_mint_b: 0,
            vault_a: 0,
            vault_b: 0,
            liquidity: 0,
            sqrt_price_x64: 0,
            tick_current_index: 0,
            tick_spacing: 64,
            fee_rate: 3_000,
            reserve_a: 0,
            reserve_b: 0,
        };
        self.pools
            .insert(pool_id, DexPool::OrcaWhirlpool { state: placeholder });
    }

    /// Register a Kamino Lending reserve (price feed only; no swap IX).
    pub fn register_kamino_reserve(&mut self, reserve_id: AccountId) {
        let placeholder = KaminoReserve {
            token_mint: 0,
            available_amount: 0,
            price_usd: 0.0,
            mint_decimals: 0,
        };
        self.pools
            .insert(reserve_id, DexPool::KaminoLending { state: placeholder });
    }

    // ─── Account & token update handlers ─────────────────────────────────────

    /// Process an account update from the Catscope host.
    ///
    /// Call this from `CommitHook::on_account` for every account update.
    /// Returns `Some(PriceUpdate)` when a tracked pool's price changes.
    pub fn on_account(&mut self, header: &Header, body: &[u8]) -> Option<PriceUpdate> {
        let pool_id = header.accountid;
        let pool = self.pools.get_mut(&pool_id)?;
        let update = pool.on_account(header, body, pool_id)?;

        // Register vault accounts after we learn their IDs from the pool state.
        let (va, vb) = pool.vault_accounts();
        if let Some(v) = va {
            if v != 0 {
                self.vault_to_pool.insert(v, pool_id);
            }
        }
        if let Some(v) = vb {
            if v != 0 {
                self.vault_to_pool.insert(v, pool_id);
            }
        }

        Some(update)
    }

    /// Process an SPL token account update from the Catscope host.
    ///
    /// Call this from `CommitHook::on_token` to keep Raydium and Orca vault
    /// reserves current. Returns `Some(PriceUpdate)` if reserves changed.
    pub fn on_token(&mut self, token: &Tokenaccountv1) -> Option<PriceUpdate> {
        let pool_id = *self.vault_to_pool.get(&token.id)?;
        let pool = self.pools.get_mut(&pool_id)?;
        pool.on_token(token, pool_id)
    }

    // ─── Price queries ────────────────────────────────────────────────────────

    /// Current price snapshot for a registered pool, if available.
    pub fn price(&self, pool_id: AccountId) -> Option<PoolPrice> {
        self.pools.get(&pool_id)?.pool_price()
    }

    /// DEX type for a registered pool.
    pub fn dex_type(&self, pool_id: AccountId) -> Option<DexType> {
        Some(self.pools.get(&pool_id)?.dex_type())
    }

    /// Constant-product quote (no IX built).
    ///
    /// Returns `None` if the pool is unregistered or reserves are unknown.
    pub fn quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        Some(self.price(pool_id)?.quote(input_mint, amount_in))
    }

    // ─── Swap instruction builders ────────────────────────────────────────────

    /// Append a swap instruction to `wallet` based on [`SwapParams`].
    ///
    /// The DEX protocol is selected automatically from which pool was registered
    /// under `params.pool`. `params.min_amount_out` is the slippage guard.
    pub fn swap(&mut self, params: &SwapParams, wallet: &mut Wallet) -> Result<(), TraderError> {
        let pool = self
            .pools
            .get(&params.pool)
            .ok_or(TraderError::UnknownPool(params.pool))?;

        match pool {
            DexPool::RaydiumAmm { state, swap_config } => {
                let cfg = swap_config.as_ref().ok_or(TraderError::MissingConfig(
                    "RaydiumAmmSwapConfig not configured",
                ))?;
                dex::raydium::build_swap_ix(params.pool, state, cfg, params, wallet)
            }
            DexPool::OrcaWhirlpool { state } => {
                state.build_swap_ix_pda(params.pool, params, wallet)
            }
            DexPool::KaminoLending { .. } => Err(TraderError::MissingConfig(
                "Kamino reserves do not support direct swap instructions; \
                 use Kamino's router program instead",
            )),
        }
    }

    /// Sell `params.input_mint` for `params.output_mint`. Alias for [`swap`](Self::swap).
    #[inline]
    pub fn sell(&mut self, params: &SwapParams, wallet: &mut Wallet) -> Result<(), TraderError> {
        self.swap(params, wallet)
    }

    /// Buy `params.output_mint` by spending `params.input_mint`. Alias for [`swap`](Self::swap).
    #[inline]
    pub fn buy(&mut self, params: &SwapParams, wallet: &mut Wallet) -> Result<(), TraderError> {
        self.swap(params, wallet)
    }
}

impl Default for DexTrader {
    fn default() -> Self {
        Self::new()
    }
}
