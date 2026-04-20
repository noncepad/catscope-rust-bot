pub mod kamino;
pub mod orca;
pub mod raydium;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    graph::AccountId,
    trader::types::{DexType, PoolPrice, PriceUpdate},
};

// ─── Per-DEX registration metadata ───────────────────────────────────────────

/// All per-DEX state that the trader tracks for a single registered pool.
pub(crate) enum DexPool {
    RaydiumAmm {
        state: raydium::RaydiumAmmPool,
        swap_config: Option<raydium::RaydiumAmmSwapConfig>,
    },
    OrcaWhirlpool {
        state: orca::OrcaWhirlpool,
    },
    KaminoLending {
        state: kamino::KaminoReserve,
    },
}

impl DexPool {
    pub(crate) fn dex_type(&self) -> DexType {
        match self {
            DexPool::RaydiumAmm { .. } => DexType::RaydiumAmm,
            DexPool::OrcaWhirlpool { .. } => DexType::OrcaWhirlpool,
            DexPool::KaminoLending { .. } => DexType::KaminoLending,
        }
    }

    /// Extract a price snapshot from the current state, if available.
    pub(crate) fn pool_price(&self) -> Option<PoolPrice> {
        match self {
            DexPool::RaydiumAmm { state, .. } => state.pool_price(),
            DexPool::OrcaWhirlpool { state } => Some(state.pool_price()),
            DexPool::KaminoLending { state } => Some(state.pool_price()),
        }
    }

    /// Update state from a raw account body, returning a new price if it changed.
    pub(crate) fn on_account(
        &mut self,
        _header: &Header,
        body: &[u8],
        pool_id: AccountId,
    ) -> Option<PriceUpdate> {
        match self {
            DexPool::RaydiumAmm { state, .. } => {
                if let Some(new_state) = raydium::parse(body) {
                    // Preserve cached reserves
                    let rc = state.reserve_coin;
                    let rp = state.reserve_pc;
                    *state = new_state;
                    state.reserve_coin = rc;
                    state.reserve_pc = rp;
                }
            }
            DexPool::OrcaWhirlpool { state } => {
                if let Some(new_state) = orca::parse(body) {
                    let ra = state.reserve_a;
                    let rb = state.reserve_b;
                    *state = new_state;
                    state.reserve_a = ra;
                    state.reserve_b = rb;
                }
            }
            DexPool::KaminoLending { state } => {
                if let Some(new_state) = kamino::parse(body) {
                    *state = new_state;
                }
            }
        }
        self.pool_price().map(|price| PriceUpdate { pool: pool_id, price })
    }

    /// Update vault reserves from an SPL token account event.
    ///
    /// Returns a price update if this token account belongs to one of this
    /// pool's vaults and the price state changed.
    pub(crate) fn on_token(
        &mut self,
        token: &Tokenaccountv1,
        pool_id: AccountId,
    ) -> Option<PriceUpdate> {
        let changed = match self {
            DexPool::RaydiumAmm { state, .. } => {
                if token.id == state.coin_vault {
                    state.reserve_coin = token.amount;
                    true
                } else if token.id == state.pc_vault {
                    state.reserve_pc = token.amount;
                    true
                } else {
                    false
                }
            }
            DexPool::OrcaWhirlpool { state } => {
                if token.id == state.vault_a {
                    state.reserve_a = token.amount;
                    true
                } else if token.id == state.vault_b {
                    state.reserve_b = token.amount;
                    true
                } else {
                    false
                }
            }
            DexPool::KaminoLending { .. } => false,
        };

        if changed {
            self.pool_price().map(|price| PriceUpdate { pool: pool_id, price })
        } else {
            None
        }
    }

    /// Returns the vault account IDs that should be tracked for this pool.
    pub(crate) fn vault_accounts(&self) -> (Option<AccountId>, Option<AccountId>) {
        match self {
            DexPool::RaydiumAmm { state, .. } => {
                (Some(state.coin_vault), Some(state.pc_vault))
            }
            DexPool::OrcaWhirlpool { state } => {
                (Some(state.vault_a), Some(state.vault_b))
            }
            DexPool::KaminoLending { .. } => (None, None),
        }
    }
}
