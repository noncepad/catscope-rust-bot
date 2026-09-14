use std::{collections::HashSet, hash::BuildHasherDefault};

use solana_sdk::pubkey::Pubkey;
use twox_hash::XxHash64;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    raydium_amm_config, raydium_clmm_config, raydium_cpmm_config,
    trader::{
        dex::{
            raydium::{
                amm::{RaydiumAmm, RaydiumAmmSetup, RaydiumAmmSwapConfig},
                clmm::{RaydiumClmm, RaydiumClmmPoolSetup},
                cpmm::{RaydiumCpmm, RaydiumCpmmPoolSetup},
            },
            update::Updater,
        },
        pricegraph::TradeRouter,
    },
    util::account_id_from_pubkey,
};

pub mod amm;
pub mod clmm;
pub mod cpmm;

#[derive(Debug)]
pub struct RaydiumState {
    l_program: [AccountId; 3],
    cpmm: RaydiumCpmm,
    clmm: RaydiumClmm,
    amm: RaydiumAmm,
}

#[derive(Debug)]
pub struct Setup {
    pub l_raydium_clmm: Vec<RaydiumClmmPoolSetup>,
    pub l_raydium_amm: Vec<RaydiumAmmSetup>,
    pub l_raydium_cpmm: Vec<RaydiumCpmmPoolSetup>,
}
impl Default for Setup {
    fn default() -> Self {
        let mut z = [0u8; 32];
        // ── Raydium AMM ──────────────────────────────────────────────────────
        let mut l_raydium_amm = Vec::with_capacity(raydium_amm_config::RAYDIUM_AMM_POOLS.len());
        for raw in raydium_amm_config::RAYDIUM_AMM_POOLS {
            let mut to_id = |bytes: &[u8; 32]| {
                z.copy_from_slice(bytes);
                account_id_from_pubkey(&Pubkey::new_from_array(z))
            };
            l_raydium_amm.push(RaydiumAmmSetup {
                pubkey: to_id(&raw.pubkey),
                swap_config: RaydiumAmmSwapConfig {
                    market_bids: to_id(&raw.market_bids),
                    market_asks: to_id(&raw.market_asks),
                    market_event_queue: to_id(&raw.market_event_queue),
                    market_coin_vault: to_id(&raw.market_coin_vault),
                    market_pc_vault: to_id(&raw.market_pc_vault),
                    market_vault_signer: to_id(&raw.market_vault_signer),
                },
            });
        }

        // ── Raydium CLMM ─────────────────────────────────────────────────────
        let mut l_raydium_clmm = Vec::with_capacity(raydium_clmm_config::RAYDIUM_CLMM_POOLS.len());
        for raw in raydium_clmm_config::RAYDIUM_CLMM_POOLS {
            z.copy_from_slice(&raw.pubkey);
            let pubkey = account_id_from_pubkey(&Pubkey::new_from_array(z));
            z.copy_from_slice(&raw.mint_0);
            let mint_0 = account_id_from_pubkey(&Pubkey::new_from_array(z));
            z.copy_from_slice(&raw.mint_1);
            let mint_1 = account_id_from_pubkey(&Pubkey::new_from_array(z));
            l_raydium_clmm.push(RaydiumClmmPoolSetup {
                pubkey,
                mint_0,
                mint_1,
                fee_rate_pips: raw.fee_rate_pips,
            });
        }

        // ── Raydium CPMM ─────────────────────────────────────────────────────
        let mut l_raydium_cpmm = Vec::with_capacity(raydium_cpmm_config::RAYDIUM_CPMM_POOLS.len());
        for raw in raydium_cpmm_config::RAYDIUM_CPMM_POOLS {
            z.copy_from_slice(&raw.pubkey);
            let pubkey = account_id_from_pubkey(&Pubkey::new_from_array(z));
            z.copy_from_slice(&raw.mint_0);
            let mint_0 = account_id_from_pubkey(&Pubkey::new_from_array(z));
            z.copy_from_slice(&raw.mint_1);
            let mint_1 = account_id_from_pubkey(&Pubkey::new_from_array(z));
            l_raydium_cpmm.push(RaydiumCpmmPoolSetup {
                pubkey,
                mint_0,
                mint_1,
                trade_fee_rate: raw.trade_fee_rate,
            });
        }

        Self {
            l_raydium_clmm,
            l_raydium_amm,
            l_raydium_cpmm,
        }
    }
}
impl RaydiumState {
    /// Builds every sub-dex's live state and returns their combined
    /// pending subscription requests alongside it -- doesn't subscribe
    /// itself. See `amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new(
        hs_program_id: &mut HashSet<AccountId, BuildHasherDefault<XxHash64>>,
    ) -> (Self, Vec<SubscriptionRequest>) {
        let setup = Setup::default();
        let (amm, amm_reqs) = RaydiumAmm::new(&setup.l_raydium_amm);
        assert!(hs_program_id.insert(*amm.program_id()));
        let (clmm, clmm_reqs) = RaydiumClmm::new(&setup.l_raydium_clmm);
        assert!(hs_program_id.insert(*clmm.program_id()));
        let (cpmm, cpmm_reqs) = RaydiumCpmm::new(&setup.l_raydium_cpmm);
        assert!(hs_program_id.insert(*cpmm.program_id()));
        let l_program = [*amm.program_id(), *clmm.program_id(), *cpmm.program_id()];
        let mut l_req = Vec::with_capacity(amm_reqs.len() + clmm_reqs.len() + cpmm_reqs.len());
        l_req.extend(amm_reqs);
        l_req.extend(clmm_reqs);
        l_req.extend(cpmm_reqs);
        (Self { cpmm, clmm, amm, l_program }, l_req)
    }

    /// (amm, clmm, cpmm) pool counts.
    pub fn pool_counts(&self) -> (usize, usize, usize) {
        (
            self.amm.pool_count(),
            self.clmm.pool_count(),
            self.cpmm.pool_count(),
        )
    }

    /// TEMPORARY DIAGNOSTIC (2026-09-07): see
    /// `RaydiumClmm::tick_array_sub_queue_pending_count`'s own doc comment.
    pub fn clmm_tick_array_sub_queue_pending_count(&self) -> usize {
        self.clmm.tick_array_sub_queue_pending_count()
    }

    /// Thin delegation to `RaydiumAmm::plan_hop` -- see
    /// `DexState::execute_hop`'s dispatch doc.
    pub fn plan_amm_hop(
        &self,
        hop: &crate::trader::pricegraph::Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut crate::wallet::Wallet,
    ) -> Result<(), crate::trader::types::TraderError> {
        self.amm.plan_hop(hop, owner, source_ata, dest_ata, wallet)
    }

    /// Thin delegation to `RaydiumClmm::plan_hop`.
    pub fn plan_clmm_hop(
        &self,
        hop: &crate::trader::pricegraph::Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut crate::wallet::Wallet,
    ) -> Result<(), crate::trader::types::TraderError> {
        self.clmm.plan_hop(hop, owner, source_ata, dest_ata, wallet)
    }

    /// Thin delegation to `RaydiumClmm::exact_quote`.
    pub fn clmm_exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        self.clmm.exact_quote(pool_id, input_mint, amount_in)
    }

    /// Thin delegation to `RaydiumClmm::exact_quote_ready`.
    pub fn clmm_exact_quote_ready(&self, pool_id: AccountId, input_mint: AccountId) -> bool {
        self.clmm.exact_quote_ready(pool_id, input_mint)
    }

    /// Thin delegation to `RaydiumCpmm::plan_hop`.
    pub fn plan_cpmm_hop(
        &self,
        hop: &crate::trader::pricegraph::Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut crate::wallet::Wallet,
    ) -> Result<(), crate::trader::types::TraderError> {
        self.cpmm.plan_hop(hop, owner, source_ata, dest_ata, wallet)
    }
}
impl Updater for RaydiumState {
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        if self.amm.on_token(ta) {
            true
        } else if self.clmm.on_token(ta) {
            true
        } else if self.cpmm.on_token(ta) {
            true
        } else {
            false
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        let mut o_i = None;
        for (i, program_id) in self.l_program.iter().enumerate() {
            if *program_id == header.owner {
                o_i = Some(i);
            }
        }
        if o_i.is_none() {
            return;
        }
        match o_i.unwrap() {
            0 => {
                self.amm.on_account(header, body);
            }
            1 => {
                self.clmm.on_account(header, body);
            }
            2 => {
                self.cpmm.on_account(header, body);
            }
            x => panic!("bad i {x}"),
        };
    }

    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.amm.batch_router(router);
        self.clmm.batch_router(router);
        self.cpmm.batch_router(router);
    }

    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        // `on_account` above dispatches by `header.owner` (the program
        // id), which isn't available here -- only the touched account's
        // own id is. Each sub-module's m_pool is keyed by pool_id, which
        // is disjoint across amm/clmm/cpmm, so trying all three is just a
        // cheap HashMap miss for the two that don't own this pool_id.
        self.amm.refresh_account_router(account_id, router);
        self.clmm.refresh_account_router(account_id, router);
        self.cpmm.refresh_account_router(account_id, router);
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        self.amm.refresh_token_router(ta_id, router);
        self.clmm.refresh_token_router(ta_id, router);
        self.cpmm.refresh_token_router(ta_id, router);
    }

    fn on_tx(
        &mut self,
        ix: &crate::txview::CatscopeInstructionRead<'_>,
        slot: &solana_sdk::clock::Slot,
    ) {
        let mut o_i = None;
        for (i, program_id) in self.l_program.iter().enumerate() {
            if *program_id == *ix.program() {
                o_i = Some(i);
            }
        }
        let Some(i) = o_i else {
            return;
        };
        match i {
            0 => self.amm.on_tx(ix, slot),
            1 => self.clmm.on_tx(ix, slot),
            2 => self.cpmm.on_tx(ix, slot),
            x => panic!("bad i {x}"),
        };
    }

    fn flush_pool(&mut self, graph: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        // Only CLMM needs a real flush -- it queues PDA-derived tick-array
        // subscriptions (tick_array_sub_queue) that must be flushed the
        // same way Orca's tick-array subscriptions are; AMM/CPMM pools
        // don't have anything analogous to queue.
        self.clmm.flush_pool(graph, max_per_flush)
    }
}
