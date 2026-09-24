pub mod drift;
/// Ember -- converts real USDC into Phoenix's native collateral mint
/// (PhUSD). Pure instruction-builder module, no live account state to
/// track, so (unlike every other module in this list) it's not part of
/// `DexState` -- see `ember.rs`'s own module doc.
pub mod ember;
pub mod jet;
pub mod kamino;
pub mod marginfi;
pub mod marinade;
pub mod orca;
/// Phoenix perpetuals market pricing/risk observation -- registered below
/// like every other dex, but `batch_router` is a documented placeholder
/// (see `phoenix/mod.rs`'s module doc and
/// `PhoenixState::add_to_pricing_router`): a perp position isn't a
/// `TradeRouter` graph edge the way a spot swap is, so this doesn't feed
/// the router yet. `brain::phoenixperpsv1` separately owns its own
/// `PhoenixState` instance for the full trader-account/margin lifecycle.
pub mod phoenix;
pub mod pumpfun;
pub mod pumpswap;
pub mod pyth;
pub mod raydium;
pub mod sanctum;
pub mod solend;
pub mod spl_stake_pool;
pub mod update;
/// Drift/Velocity Protocol perpetual futures -- pricing-only, deliberately
/// **not** wired into `DexState`/`Updater` below (unlike every other
/// module in this list). See `velocity/mod.rs`'s own doc comment: this
/// exists to feed `perp_router::PerpRouter`, a standalone framework module,
/// not a live subscription this bot maintains yet.
pub mod velocity;

use std::{collections::HashSet, hash::BuildHasherDefault};

use twox_hash::XxHash64;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionQueue},
    trader::{
        dex::{
            drift::DriftState, kamino::KaminoState, marginfi::MarginfiState,
            marinade::MarinadeState, orca::OrcaState, phoenix::PhoenixState,
            pumpfun::PumpfunState, pumpswap::PumpswapState, raydium::RaydiumState,
            sanctum::SanctumState, solend::SolendState, spl_stake_pool::SplStakePoolState,
            update::Updater,
        },
        pricegraph::Hop,
        types::{DexType, TraderError},
    },
    wallet::Wallet,
};

// ─── Per-DEX registration metadata ───────────────────────────────────────────
#[derive(Debug)]
pub struct DexState {
    hs_program_id: HashSet<AccountId, BuildHasherDefault<XxHash64>>,
    orca: OrcaState,
    raydium: RaydiumState,
    kamino: KaminoState,
    sanctum: SanctumState,
    marginfi: MarginfiState,
    solend: SolendState,
    drift: DriftState,
    marinade: MarinadeState,
    spl_stake_pool: SplStakePoolState,
    pumpfun: PumpfunState,
    pumpswap: PumpswapState,
    phoenix: PhoenixState,
    /// Direct subscription to Pyth's own SOL/USD feed -- see
    /// `pyth::PythFeedState`'s doc comment for why this exists alongside
    /// (not instead of) `marginfi`'s bank-mediated oracle lookup.
    pyth_feed: pyth::PythFeedState,
    /// Every sub-dex's startup subscription request (real, live-observed
    /// total: ~32,000 across Raydium/Orca/lending reserves/etc.), paced
    /// out via [`Self::flush_subscriptions`] instead of one blocking
    /// `bulk_subscribe` call per sub-dex at construction time -- see
    /// [`SubscriptionQueue`]'s own doc comment for why.
    subscription_queue: SubscriptionQueue,
}
impl DexState {
    /// Builds every sub-dex's live state. Doesn't subscribe to anything
    /// itself -- every sub-dex's pending subscription requests are
    /// collected into `subscription_queue` instead, paced out over many
    /// slots by [`Self::flush_subscriptions`] (call once per slot,
    /// typically from `CommitHook::finish`). No longer takes `&Graph` --
    /// nothing in this constructor touches the host anymore.
    pub fn new() -> Result<Self, CatscopeGuestError> {
        let mut hs_program_id =
            HashSet::with_capacity_and_hasher(10, BuildHasherDefault::default());
        let mut subscription_queue = SubscriptionQueue::default();

        let (raydium, raydium_reqs) = RaydiumState::new(&mut hs_program_id);
        subscription_queue.extend(raydium_reqs);
        let (orca, orca_reqs) = OrcaState::new();
        subscription_queue.extend(orca_reqs);
        hs_program_id.insert(*orca.program_id());
        let (kamino, kamino_reqs) = KaminoState::new();
        subscription_queue.extend(kamino_reqs);
        hs_program_id.insert(*kamino.program_id());
        let (sanctum, sanctum_reqs) = SanctumState::new();
        subscription_queue.extend(sanctum_reqs);
        hs_program_id.insert(*sanctum.program_id());
        let (marginfi, marginfi_reqs) = MarginfiState::new();
        subscription_queue.extend(marginfi_reqs);
        hs_program_id.insert(*marginfi.program_id());
        let (solend, solend_reqs) = SolendState::new();
        subscription_queue.extend(solend_reqs);
        hs_program_id.insert(*solend.program_id());
        let (drift, drift_reqs) = DriftState::new();
        subscription_queue.extend(drift_reqs);
        hs_program_id.insert(*drift.program_id());
        let (marinade, marinade_reqs) = MarinadeState::new();
        subscription_queue.extend(marinade_reqs);
        hs_program_id.insert(*marinade.program_id());
        let (spl_stake_pool, spl_stake_pool_reqs) = SplStakePoolState::new();
        subscription_queue.extend(spl_stake_pool_reqs);
        hs_program_id.insert(*spl_stake_pool.program_id());
        let (pumpfun, pumpfun_reqs) = PumpfunState::new();
        subscription_queue.extend(pumpfun_reqs);
        hs_program_id.insert(*pumpfun.program_id());
        let (pumpswap, pumpswap_reqs) = PumpswapState::new();
        subscription_queue.extend(pumpswap_reqs);
        hs_program_id.insert(*pumpswap.program_id());
        let (phoenix, phoenix_reqs) = PhoenixState::new();
        subscription_queue.extend(phoenix_reqs);
        hs_program_id.insert(*phoenix.program_id());
        // Not a dex program -- Pyth's SOL/USD account is a plain data
        // account, not owned by any program this bot otherwise cares
        // about -- so it's deliberately not added to hs_program_id
        // (that set is unrelated to whether an explicitly-subscribed
        // account receives updates; every other single-account
        // subscription in this file, e.g. marginfi's oracle accounts,
        // already works the same way).
        let (pyth_feed, pyth_req) = pyth::PythFeedState::new();
        subscription_queue.push(pyth_req);
        Ok(Self {
            hs_program_id,
            raydium,
            orca,
            kamino,
            sanctum,
            marginfi,
            solend,
            drift,
            marinade,
            spl_stake_pool,
            pumpfun,
            pumpswap,
            phoenix,
            pyth_feed,
            subscription_queue,
        })
    }

    /// Drains up to `max_per_flush` queued startup subscription requests
    /// into one bounded `bulk_subscribe` call -- call once per slot,
    /// typically from `CommitHook::finish`. No-op (`Ok(0)`) once the
    /// queue is empty. See [`SubscriptionQueue::flush`].
    pub fn flush_subscriptions(&mut self, g: &Graph, max_per_flush: usize) -> Result<usize, CatscopeGuestError> {
        self.subscription_queue.flush(g, max_per_flush)
    }
    /// How many of the startup subscription burst's requests are still
    /// queued/already active -- diagnostic accessor for callers timing
    /// `flush_subscriptions` (e.g. to log a breadcrumb right before a
    /// call that could block for a while on the validator side).
    pub fn subscription_pending_count(&self) -> usize {
        self.subscription_queue.pending_count()
    }
    pub fn subscription_active_count(&self) -> usize {
        self.subscription_queue.active_count()
    }
    /// Exposed directly (rather than through `pool_stats()`'s flat
    /// snapshot) because `compare_unstake_paths` is parameterized
    /// (amount, slot) -- state.rs's diagnostic block calls it directly.
    pub fn marinade(&self) -> &MarinadeState {
        &self.marinade
    }

    /// Exposed directly, same reasoning as [`Self::marinade`] --
    /// `perpfundingv1`'s Solend-basis-trade strategy needs a
    /// `SolendReserve` handle (via `SolendState::reserve_by_mint`) to
    /// build deposit/borrow/withdraw/repay instructions, and this
    /// `DexState`-owned instance is the only place that data lives (the
    /// shared, read-only pricing instance -- `perpfundingv1`'s own
    /// `SolendPosition` only tracks its own obligation, not reserve
    /// data).
    pub fn solend(&self) -> &SolendState {
        &self.solend
    }

    /// Exposed directly, same reasoning as [`Self::solend`] --
    /// `perpfundingv1`'s basis-trade strategy needs a `KaminoReserve`
    /// handle (via `KaminoState::reserve_by_mint`) to build
    /// deposit/borrow/withdraw/repay instructions, and this
    /// `DexState`-owned instance is the only place that data lives.
    pub fn kamino(&self) -> &KaminoState {
        &self.kamino
    }

    /// Exposed directly, same reasoning as [`Self::solend`]/[`Self::kamino`]
    /// -- `perpfundingv1`'s basis-trade strategy needs a `MarginfiBank`
    /// handle (via `MarginfiState::reserve_by_mint`) to build
    /// deposit/borrow/withdraw/repay instructions.
    pub fn marginfi(&self) -> &MarginfiState {
        &self.marginfi
    }

    /// Exposed directly, same reasoning as [`Self::solend`]/[`Self::kamino`]
    /// -- `leveragedloopv1`'s funding-rate basis-trade strategy needs real
    /// Phoenix market data (mark price, funding accumulator) to feed
    /// `trader::perp_router::PerpRouter`. This is the shared, read-only
    /// pricing instance only -- a real trader account/margin lifecycle
    /// needs its own separate `PhoenixState` instance with a wallet
    /// authority (see `trader::dex::phoenix::mod`'s module doc comment),
    /// this accessor doesn't provide that.
    pub fn phoenix(&self) -> &PhoenixState {
        &self.phoenix
    }

    /// Build the swap instruction for one arbitrage-cycle `Hop`, appending
    /// it to `wallet`. Single dispatch point across every dex -- matches
    /// `Updater`'s existing pattern of `DexState` being the one place that
    /// knows about every sub-dex. Used by `brain::arbv1::state::
    /// StateHelper::build_execution_plan` and `brain::multimodelv1::
    /// state::StateHelper::execute_arbitrage_opportunity` (a port of the
    /// former) to build a price-graph-detected cycle's real swap
    /// instructions directly onto the bot's own wallet, inside a
    /// checkpoint + atomic group that gets rolled back on any failure or
    /// non-positive net profit (see either caller's own doc comment).
    pub fn execute_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        match hop.dex {
            DexType::RaydiumAmm => self.raydium.plan_amm_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::RaydiumClmm => self.raydium.plan_clmm_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::RaydiumCpmm => self.raydium.plan_cpmm_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::OrcaWhirlpool => self.orca.plan_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::Sanctum => self.sanctum.plan_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::MarinadeLiquidUnstake => self.marinade.plan_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::SplStakePoolWithdrawSol => {
                self.spl_stake_pool.plan_hop(hop, owner, source_ata, dest_ata, wallet)
            }
            DexType::PumpfunBondingCurve => self.pumpfun.plan_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::PumpswapAmm => self.pumpswap.plan_hop(hop, owner, source_ata, dest_ata, wallet),
            DexType::KaminoLending => {
                // Defensive/unreachable in practice: `KaminoState::batch_router`
                // never adds an edge (a lending Bank has no swap price to
                // contribute to TradeRouter), so `find_arbitrage` can never
                // produce a `Hop` with this `DexType`. Kept as an explicit
                // arm rather than a wildcard so a future real `DexType`
                // variant can't silently fall through unhandled.
                Err(TraderError::MissingConfig("KaminoLending has no swap path"))
            }
        }
    }

    pub fn program_id_set(&self, l_program_id: &mut [AccountId]) -> usize {
        let mut i = 0;
        for p_id in self.hs_program_id.iter() {
            l_program_id[i] = *p_id;
            i += 1;
        }
        i
    }

    /// TEMPORARY DEBUG: see `OrcaState::debug_pool_state`'s doc comment.
    pub fn debug_orca_pool_state(&self, pool_id: AccountId) -> Option<String> {
        self.orca.debug_pool_state(pool_id)
    }

    /// TEMPORARY DEBUG: see `OrcaState::debug_quote_range`'s doc comment.
    pub fn debug_orca_quote_range(&self, pool_id: AccountId, input_mint: AccountId, amounts_in: &[u64]) -> Option<String> {
        self.orca.debug_quote_range(pool_id, input_mint, amounts_in)
    }

    /// TEMPORARY DEBUG: see `OrcaState::debug_tick_array_status`'s doc comment.
    pub fn debug_orca_tick_array_status(&self, pool_id: AccountId, input_mint: AccountId) -> Option<String> {
        self.orca.debug_tick_array_status(pool_id, input_mint)
    }

    /// See `OrcaState::exact_quote`'s doc comment.
    pub fn orca_exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        self.orca.exact_quote(pool_id, input_mint, amount_in)
    }

    /// See `OrcaState::exact_quote_ready`'s doc comment.
    pub fn orca_exact_quote_ready(&self, pool_id: AccountId, input_mint: AccountId) -> bool {
        self.orca.exact_quote_ready(pool_id, input_mint)
    }

    /// See `RaydiumClmm::exact_quote`'s doc comment.
    pub fn raydium_clmm_exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        self.raydium.clmm_exact_quote(pool_id, input_mint, amount_in)
    }

    /// See `RaydiumClmm::exact_quote_ready`'s doc comment.
    pub fn raydium_clmm_exact_quote_ready(&self, pool_id: AccountId, input_mint: AccountId) -> bool {
        self.raydium.clmm_exact_quote_ready(pool_id, input_mint)
    }

    /// See `SplStakePoolState::exact_quote`'s doc comment.
    pub fn spl_stake_pool_exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        self.spl_stake_pool.exact_quote(pool_id, input_mint, amount_in)
    }

    /// See `MarginfiState::price_for_mint`'s doc comment.
    pub fn pyth_price_for_mint(&self, mint: AccountId) -> Option<pyth::OraclePrice> {
        self.marginfi.price_for_mint(mint)
    }

    /// Direct Pyth SOL/USD price -- see `pyth::PythFeedState`'s doc
    /// comment for why this exists separately from `pyth_price_for_mint`
    /// (marginfi's tracked bank set has no SOL bank to mediate through).
    pub fn pyth_sol_usd_price(&self) -> Option<pyth::OraclePrice> {
        self.pyth_feed.sol_usd_price().map(Into::into)
    }

    /// TEMPORARY DEBUG: see `MarginfiState::debug_oracle_status`.
    pub fn debug_marginfi_oracle_status(&self) -> String {
        self.marginfi.debug_oracle_status()
    }

    /// Snapshot of how many pools/reserves/markets each sub-dex is
    /// currently tracking -- for the periodic "pool stats" log (see
    /// StateHelper::start in brain/arbv1/state.rs).
    pub fn pool_stats(&self) -> DexPoolStats {
        let (raydium_amm, raydium_clmm, raydium_cpmm) = self.raydium.pool_counts();
        let (orca_whirlpool_updates, _parsed_count, _tx_count) = self.orca.count();
        DexPoolStats {
            raydium_amm,
            raydium_clmm,
            raydium_cpmm,
            orca_pools: self.orca.pool_count(),
            orca_tick_arrays: self.orca.tick_array_count(),
            orca_whirlpool_updates,
            kamino_reserves: self.kamino.reserve_count(),
            sanctum_lsts: self.sanctum.lst_count(),
            marginfi_banks: self.marginfi.bank_count(),
            marginfi_oracles_subscribed: self.marginfi.oracle_subscribed_count(),
            marginfi_legacy_priced: self.marginfi.legacy_priced_count(),
            marginfi_push_priced: self.marginfi.push_priced_count(),
            solend_reserves: self.solend.reserve_count(),
            drift_markets: self.drift.market_count(),
            marinade_ready: self.marinade.is_ready(),
            spl_stake_pool_ready: self.spl_stake_pool.ready_count(),
            pumpfun_curves: self.pumpfun.ready_count(),
            pumpswap_pools_ready: self.pumpswap.ready_count(),
            phoenix_markets_ready: self.phoenix.ready_count(),
            orca_token_sub_queue_pending: self.orca.token_sub_queue_pending_count(),
            raydium_clmm_tick_array_sub_queue_pending: self.raydium.clmm_tick_array_sub_queue_pending_count(),
        }
    }
}

#[derive(Debug)]
pub struct DexPoolStats {
    pub raydium_amm: usize,
    pub raydium_clmm: usize,
    pub raydium_cpmm: usize,
    pub orca_pools: usize,
    /// Live tick-array accounts in memory, across every Orca pool -- watch
    /// this over time, it's exactly what overflowed hashbrown's capacity
    /// before ORCA_WHIRLPOOL_POOLS was capped (see build.rs's
    /// orca_pool_budget()).
    pub orca_tick_arrays: usize,
    /// Lifetime count of Whirlpool (pool, not tick-array) accounts actually
    /// parsed in `on_account` -- `OrcaState::count`, previously untracked
    /// here. If this stays at 0 despite `commit_pools` climbing, Orca pool
    /// accounts simply aren't among what's being delivered to `on_account`
    /// at all, which would explain `orca_tick_arrays` never growing (the
    /// tick-array subscribe logic only runs from inside this same branch).
    pub orca_whirlpool_updates: usize,
    pub kamino_reserves: usize,
    pub sanctum_lsts: usize,
    pub marginfi_banks: usize,
    /// TEMPORARY DEBUG (Pyth Push Oracle wiring verification): how many
    /// bank oracle accounts are subscribed vs. how many have delivered a
    /// price via each parser -- see `MarginfiState::oracle_subscribed_count`.
    pub marginfi_oracles_subscribed: usize,
    pub marginfi_legacy_priced: usize,
    pub marginfi_push_priced: usize,
    pub solend_reserves: usize,
    pub drift_markets: usize,
    /// `1` once Marinade's liquid-unstake pricing is live, `0` otherwise.
    pub marinade_ready: usize,
    /// How many of the tracked "Spl"-kind SPL Stake Pool LSTs (JitoSOL,
    /// bSOL, and others -- see spl_stake_pool.rs's module doc) have live
    /// pricing.
    pub spl_stake_pool_ready: usize,
    /// How many tracked Pump.fun bonding curves (see build.rs's
    /// PUMPFUN_BONDING_CURVES) have live, tradeable pricing data.
    pub pumpfun_curves: usize,
    /// How many tracked PumpSwap pools (see build.rs's PUMPSWAP_POOLS)
    /// have live pricing data (both vault balances nonzero).
    pub pumpswap_pools_ready: usize,
    /// How many tracked Phoenix perp markets (see build.rs's
    /// PHOENIX_MARKETS, sourced from the `phoenix_market` prefetch.db
    /// table) have live pricing data from `PerpAssetMap` -- observation
    /// only, not yet fed into `TradeRouter`, see
    /// `PhoenixState::add_to_pricing_router`.
    pub phoenix_markets_ready: usize,
    /// TEMPORARY DIAGNOSTIC (2026-09-07): see `OrcaState::token_sub_queue_
    /// pending_count`'s own doc comment.
    pub orca_token_sub_queue_pending: usize,
    /// TEMPORARY DIAGNOSTIC (2026-09-07): see `RaydiumClmm::tick_array_
    /// sub_queue_pending_count`'s own doc comment.
    pub raydium_clmm_tick_array_sub_queue_pending: usize,
}

impl std::fmt::Display for DexPoolStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "raydium_amm={} raydium_clmm={} raydium_cpmm={} orca_pools={} orca_tick_arrays={} \
             orca_whirlpool_updates={} \
             kamino_reserves={} sanctum_lsts={} marginfi_banks={} marginfi_oracles_subscribed={} \
             marginfi_legacy_priced={} marginfi_push_priced={} solend_reserves={} drift_markets={} \
             marinade_ready={} spl_stake_pool_ready={} pumpfun_curves={} pumpswap_pools_ready={} \
             phoenix_markets_ready={} orca_token_sub_queue_pending={} \
             raydium_clmm_tick_array_sub_queue_pending={}",
            self.raydium_amm,
            self.raydium_clmm,
            self.raydium_cpmm,
            self.orca_pools,
            self.orca_tick_arrays,
            self.orca_whirlpool_updates,
            self.kamino_reserves,
            self.sanctum_lsts,
            self.marginfi_banks,
            self.marginfi_oracles_subscribed,
            self.marginfi_legacy_priced,
            self.marginfi_push_priced,
            self.solend_reserves,
            self.drift_markets,
            self.marinade_ready,
            self.spl_stake_pool_ready,
            self.pumpfun_curves,
            self.pumpswap_pools_ready,
            self.phoenix_markets_ready,
            self.orca_token_sub_queue_pending,
            self.raydium_clmm_tick_array_sub_queue_pending,
        )
    }
}
impl Updater for DexState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.raydium.on_account(header, body);
        self.kamino.on_account(header, body);
        self.orca.on_account(header, body);
        self.sanctum.on_account(header, body);
        self.marginfi.on_account(header, body);
        self.solend.on_account(header, body);
        self.drift.on_account(header, body);
        self.marinade.on_account(header, body);
        self.spl_stake_pool.on_account(header, body);
        self.pumpfun.on_account(header, body);
        self.pumpswap.on_account(header, body);
        self.phoenix.on_account(header, body);
        self.pyth_feed.on_account(header.accountid, body);
    }

    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        self.raydium.on_token(ta)
            || self.orca.on_token(ta)
            || self.kamino.on_token(ta)
            || self.sanctum.on_token(ta)
            || self.marginfi.on_token(ta)
            || self.solend.on_token(ta)
            || self.drift.on_token(ta)
            || self.marinade.on_token(ta)
            || self.spl_stake_pool.on_token(ta)
            || self.pumpfun.on_token(ta)
            || self.pumpswap.on_token(ta)
            || self.phoenix.on_token(ta)
    }

    fn batch_router(&mut self, router: &mut super::pricegraph::TradeRouter) {
        self.raydium.batch_router(router);
        self.orca.batch_router(router);
        self.kamino.batch_router(router);
        self.sanctum.batch_router(router);
        self.marginfi.batch_router(router);
        self.solend.batch_router(router);
        self.drift.batch_router(router);
        self.marinade.batch_router(router);
        self.spl_stake_pool.batch_router(router);
        self.pumpfun.batch_router(router);
        self.pumpswap.batch_router(router);
        self.phoenix.batch_router(router);
    }

    /// Unconditional dispatch to all 12 (same shape as `batch_router` --
    /// unlike `on_token`'s short-circuit `||`, every sub-dex must get a
    /// chance to react, not just the first one that claims the account;
    /// the 5 that never feed router edges -- Kamino, Marginfi, Solend,
    /// Drift, Phoenix -- just inherit `Updater`'s default no-op).
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut super::pricegraph::TradeRouter) {
        self.raydium.refresh_account_router(account_id, router);
        self.orca.refresh_account_router(account_id, router);
        self.kamino.refresh_account_router(account_id, router);
        self.sanctum.refresh_account_router(account_id, router);
        self.marginfi.refresh_account_router(account_id, router);
        self.solend.refresh_account_router(account_id, router);
        self.drift.refresh_account_router(account_id, router);
        self.marinade.refresh_account_router(account_id, router);
        self.spl_stake_pool.refresh_account_router(account_id, router);
        self.pumpfun.refresh_account_router(account_id, router);
        self.pumpswap.refresh_account_router(account_id, router);
        self.phoenix.refresh_account_router(account_id, router);
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut super::pricegraph::TradeRouter) {
        self.raydium.refresh_token_router(ta_id, router);
        self.orca.refresh_token_router(ta_id, router);
        self.kamino.refresh_token_router(ta_id, router);
        self.sanctum.refresh_token_router(ta_id, router);
        self.marginfi.refresh_token_router(ta_id, router);
        self.solend.refresh_token_router(ta_id, router);
        self.drift.refresh_token_router(ta_id, router);
        self.marinade.refresh_token_router(ta_id, router);
        self.spl_stake_pool.refresh_token_router(ta_id, router);
        self.pumpfun.refresh_token_router(ta_id, router);
        self.pumpswap.refresh_token_router(ta_id, router);
        self.phoenix.refresh_token_router(ta_id, router);
    }

    fn on_tx(
        &mut self,
        ix: &crate::txview::CatscopeInstructionRead<'_>,
        slot: &solana_sdk::clock::Slot,
    ) {
        // Every sub-dex's on_tx is a safe no-op except Orca's, which does
        // real per-pool slot/tx-count bookkeeping -- dispatch to all of
        // them now that none can panic.
        self.raydium.on_tx(ix, slot);
        self.orca.on_tx(ix, slot);
        self.kamino.on_tx(ix, slot);
        self.sanctum.on_tx(ix, slot);
        self.marginfi.on_tx(ix, slot);
        self.solend.on_tx(ix, slot);
        self.drift.on_tx(ix, slot);
        self.marinade.on_tx(ix, slot);
        self.spl_stake_pool.on_tx(ix, slot);
        self.pumpfun.on_tx(ix, slot);
        self.pumpswap.on_tx(ix, slot);
        self.phoenix.on_tx(ix, slot);
    }

    fn flush_pool(&mut self, g: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        self.orca.flush_pool(g, max_per_flush)?;
        self.spl_stake_pool.flush_pool(g, max_per_flush)?;
        // Needed for Phoenix's two-hop GlobalConfiguration -> PerpAssetMap/
        // index-header discovery -- see `phoenix/mod.rs`'s module doc.
        self.phoenix.flush_pool(g, max_per_flush)?;
        // Needed for Raydium CLMM's PDA-derived tick-array subscriptions --
        // see `raydium::clmm`'s module doc.
        self.raydium.flush_pool(g, max_per_flush)?;
        Ok(())
    }
}
