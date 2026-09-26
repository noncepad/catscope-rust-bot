//! State machine for `marketwatchv1` -- a read-only market-statistics
//! observer, forked from `arbv1`'s chassis. See `mod.rs`'s own doc
//! comment for the event-flow diagram and what's genuinely different from
//! `arbv1` (no wallet, no cycle search, no execution planning -- only the
//! live pool-price feed and a throttled volatility/correlation push).
use crate::{
    brain::marketwatchv1::{
        configuration::Configuration,
        message::{CustomMessageInbound, CustomMessageOutbound},
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    router_config, router_pools_config,
    trader::{
        dex::{update::Updater as _, DexState},
        market_stats::{updates_per_second, MarketStatsTracker, TrackedMint, N_SYMBOLS, TRACKED_MINTS},
        planner::DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET,
        pricegraph::TradeRouter,
        router,
    },
    txview::TransactionList,
    util::account_id_from_pubkey,
};
use solana_sdk::clock::Slot;
use std::{
    collections::{HashMap, VecDeque},
    time::SystemTime,
};

/// How often (in slots) to recompute+push the market-stats snapshot.
/// Same reasoning `xstockshealthv1::HEALTH_BOARD_LOG_INTERVAL_SLOTS`'s
/// own doc comment gives: a human watching a dashboard doesn't need a
/// sub-second-refreshing wire message, and this codebase has already hit
/// real `stdio timeout` disconnects from over-logging/over-sending once.
/// Also bounds `quote_symbol_usd`'s own per-tracked-symbol
/// `liquidity_weighted_median_price` cost to a periodic tick instead of
/// every ~400ms `LowLatency` event.
const MARKET_STATS_INTERVAL_SLOTS: Slot = 10;

#[derive(Debug, Default)]
pub(crate) struct State {
    last_slot: Slot,
    o_dex: Option<DexState>,
    /// 3-tier liquidity router (see arbv1's own doc comment on the
    /// identical field) -- built once from the build-time
    /// router_pools_config snapshot in on_load(), not a live per-tick
    /// structure like `router` below.
    o_liquidity_router: Option<router::Router>,
    router: TradeRouter,
    tracker: MarketStatsTracker,
    /// The slot `evaluate` last pushed a `MarketStats` snapshot for --
    /// see `MARKET_STATS_INTERVAL_SLOTS`'s own doc comment for why this
    /// is throttled, and `xstockshealthv1::last_reported_slot`'s doc
    /// comment for the real, live-observed bug this specific "already
    /// pushed this slot" guard prevents (evaluate() runs after *every*
    /// event, not once per slot).
    last_stats_push_slot: Slot,
    /// Lifetime count of pool account updates seen via `Event::Commit`
    /// (rooted, ~12s) -- diagnostic only, logged in `CommitHook::start`.
    commit_pool_count: u64,
    /// Lifetime count of pool account updates seen via `Event::LowLatency`
    /// (processed, ~400ms) -- diagnostic only.
    low_latency_pool_count: u64,
    /// Lifetime count of every item (accounts + token accounts) read out
    /// of a `LowLatencyAccountUpdate` -- a superset of
    /// `low_latency_pool_count`, diagnostic only.
    low_latency_account_count: u64,
    /// Latest `Slot` `low_latency`'s ~400ms processed-account stream has
    /// recorded for each account id -- gates `CommitHook::on_account`
    /// (~12s rooted) so it never rolls pool state backwards. Identical
    /// purpose/reasoning to `arbv1::State::m_account_slot`'s own doc
    /// comment.
    m_account_slot: HashMap<AccountId, Slot>,
    /// Real wall-clock time of the previous `evaluate()` throughput
    /// sample -- `None` before the first one. Wall-clock (not slot
    /// count) on purpose: slot production speed isn't perfectly steady
    /// (backfill/catch-up can replay slots faster than real time), and
    /// the dashboard's "updates per second" figure should mean real
    /// seconds, not an assumed-constant slot cadence.
    last_throughput_sample_time: Option<SystemTime>,
    /// `low_latency_account_count`'s value as of the previous throughput
    /// sample -- diffed against the current value in `evaluate()` to get
    /// this period's `delta_count` for `market_stats::updates_per_second`.
    last_throughput_sample_count: u64,
}

/// Build the 3-tier `trader::router::Router` from the build-time
/// `router_config`/`router_pools_config` snapshot embedded from
/// prefetch.db -- identical to `arbv1::build_liquidity_router`, copied
/// rather than shared since each bot mode owns its own `State`/
/// `StateHelper` wiring independently in this codebase's established
/// pattern.
fn build_liquidity_router() -> router::Router {
    let cfg = &router_config::ROUTER_CONFIG;
    let mut r = router::Router::new(cfg.token_count, cfg.lambda);
    for core_mint in cfg.core_mints {
        r.register_mint(account_id_from_pubkey(&solana_sdk::pubkey::Pubkey::new_from_array(core_mint)));
    }
    let mut pools = Vec::with_capacity(router_pools_config::ROUTER_POOLS.len());
    for p in router_pools_config::ROUTER_POOLS {
        let token_a = r.register_mint(account_id_from_pubkey(&solana_sdk::pubkey::Pubkey::new_from_array(p.mint_a)));
        let token_b = r.register_mint(account_id_from_pubkey(&solana_sdk::pubkey::Pubkey::new_from_array(p.mint_b)));
        pools.push(router::Pool {
            token_a,
            token_b,
            liquidity_usd: p.liquidity_usd,
            price_a_to_b: p.price_a_to_b,
        });
    }
    r.rebuild_partitions(&pools);
    r
}

pub(crate) struct StateHelper<'a> {
    pub graph: &'a mut Graph,
    pub nonce: &'a mut u32,
    pub configuration: &'a mut Configuration,
    pub q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
    pub o_commit_slot: Option<Slot>,
    pub state: &'a mut State,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        self.configuration.resolve_mints();
        assert!(self.state.o_dex.replace(DexState::new().expect("dex state")).is_none());
        assert!(self
            .state
            .o_liquidity_router
            .replace(build_liquidity_router())
            .is_none());
        // Seed the live trade graph's node set from the liquidity
        // router's classified mint universe -- see arbv1's identical
        // wiring (TradeRouter::from_router's own doc comment).
        self.state.router = TradeRouter::from_router(self.state.o_liquidity_router.as_ref().unwrap());
        log_info!(
            "marketwatchv1: bot has been successfully uploaded to validator; tracking {:?}",
            TRACKED_MINTS.map(|t| t.symbol),
        );
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("marketwatchv1: slot {slot}; status dead");
        }
    }

    /// Records `slot` as the latest slot `low_latency` has processed for
    /// `account_id` -- unconditional, never a gate. See
    /// `arbv1::StateHelper::record_low_latency_slot`'s own doc comment
    /// for why (a single slot can carry multiple separate updates for the
    /// same account).
    fn record_low_latency_slot(&mut self, account_id: AccountId, slot: Slot) {
        self.state.m_account_slot.insert(account_id, slot);
    }

    /// Gate for `CommitHook::on_account`'s ~12s rooted stream only -- see
    /// `arbv1::StateHelper::is_newer_than_low_latency`'s own doc comment.
    fn is_newer_than_low_latency(&self, account_id: AccountId, slot: Slot) -> bool {
        match self.state.m_account_slot.get(&account_id) {
            Some(&last) => slot > last,
            None => true,
        }
    }

    /// Feeds `DexState`/`TradeRouter` from the ~400ms processed-account
    /// stream -- identical purpose to `arbv1::StateHelper::low_latency`,
    /// minus every `self.wallet.*` call (this bot has no wallet; see
    /// `configuration.rs`'s own doc comment).
    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        let mut account_count = 0;
        while let Some(ta) = llap.token() {
            account_count += 1;
            if let Some(dex) = self.state.o_dex.as_mut() {
                _ = dex.on_token(ta);
                dex.refresh_token_router(ta.id, &mut self.state.router);
            }
        }
        let zero = [];
        while let Some(account) = llap.account() {
            account_count += 1;
            self.state.low_latency_pool_count += 1;
            let d = account.body.unwrap_or(&zero);
            self.record_low_latency_slot(account.header.accountid, account.header.slot);
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(account.header, d);
                dex.refresh_account_router(account.header.accountid, &mut self.state.router);
            }
        }
        self.state.low_latency_account_count += account_count as u64;
    }

    /// No trading, no cycle search -- `DexState`/`TradeRouter` get
    /// everything they need from `on_account`/`low_latency`'s account and
    /// token updates (see this module's own doc comment). Still drained,
    /// not skipped, matching the same defensive convention every other
    /// non-trading bot's `mid_on_tx` uses (see e.g.
    /// `testperpv1::State::mid_on_tx`'s own doc comment).
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while transaction_list.transaction().is_some() {}
    }

    /// USDC's fixed decimals -- not looked up, matching every other
    /// USDC-denominated calculation already baked into this codebase's
    /// `Configuration::mint_usdc` convention.
    const USDC_DECIMALS: i32 = 6;

    /// Quotes `tracked` against USDC via
    /// `TradeRouter::liquidity_weighted_median_price` -- deliberately
    /// *not* `route_slippage_aware`/`route` (real, live-confirmed bug
    /// this avoids, in two escalating forms: `route`'s Bellman-Ford picks
    /// a path by comparing raw log-space spot rates with no protection at
    /// all against a thin/near-drained pool looking artificially cheap
    /// purely as a `cp_quote`-formula artifact; `route_slippage_aware`'s
    /// `widest_path` fixes *that* specific failure mode via
    /// `MAX_HOP_POOL_UTILIZATION_BPS`'s per-hop cap and the
    /// pool-reuse/node-revisit guards, but is still fundamentally an
    /// amount-*maximizing* single-path search -- live-confirmed
    /// 2026-09-25 it can still settle on one technically-valid path that
    /// quotes ~58% above the real price (SOL/USDC read a stable $190.23
    /// for 150+ seconds against a real, independently-confirmed ~$120),
    /// and stay there deterministically every cycle since the search
    /// keeps landing on the same "best" answer against an unchanged
    /// graph. Both failure modes share one root cause: trusting a single
    /// selected path at all. `liquidity_weighted_median_price` instead
    /// combines *every* live direct pool for the pair (or, lacking any,
    /// composes two such medians through
    /// `configuration.mint_sol` as a bridge) -- see its own and
    /// `TradeRouter::direct_quotes`'s doc comments for why a
    /// liquidity-weighted median of independent quotes is robust to
    /// exactly this "one path looked best but wasn't representative"
    /// failure in a way no single-path search, however guarded, can be.
    /// `None` if no live direct or bridged pool data exists yet (pool
    /// subscriptions still warming up).
    fn quote_symbol_usd(&self, tracked: &TrackedMint) -> Option<f64> {
        let mint = account_id_from_pubkey(&tracked.mint);
        let raw_price = self.state.router.liquidity_weighted_median_price(
            mint,
            self.configuration.mint_usdc,
            self.configuration.mint_sol,
        )?;
        // raw_price is USDC-raw-units per mint-raw-unit; convert to
        // USDC-UI-units per mint-UI-unit.
        let scale = 10f64.powi(tracked.decimals as i32 - Self::USDC_DECIMALS);
        Some(raw_price * scale)
    }

    /// Throttled (see `MARKET_STATS_INTERVAL_SLOTS`) recompute + push of
    /// the market-stats snapshot: quote every tracked symbol against
    /// USDC, feed the results into `MarketStatsTracker`, sample this
    /// period's low-latency account-update throughput (see
    /// `market_stats::updates_per_second`'s own doc comment -- this
    /// exists to give the dashboard something concrete to show *how
    /// fast* this bot's data pipeline really runs, not just what price
    /// it currently sees), and send the resulting snapshot to the Go
    /// brain.
    pub(crate) fn evaluate(&mut self) {
        if self.state.last_slot == 0
            || self.state.last_slot % MARKET_STATS_INTERVAL_SLOTS != 0
            || self.state.last_slot == self.state.last_stats_push_slot
        {
            return;
        }
        self.state.last_stats_push_slot = self.state.last_slot;

        let mut prices: [Option<f64>; N_SYMBOLS] = [None; N_SYMBOLS];
        for (i, tracked) in TRACKED_MINTS.iter().enumerate() {
            prices[i] = self.quote_symbol_usd(tracked);
        }
        self.state.tracker.update(prices);
        let snapshot = self.state.tracker.snapshot();

        let total_updates = self.state.low_latency_account_count;
        let now = SystemTime::now();
        let rate = match self.state.last_throughput_sample_time {
            Some(prev_time) => {
                let delta_seconds = now.duration_since(prev_time).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                let delta_count = total_updates.saturating_sub(self.state.last_throughput_sample_count);
                updates_per_second(delta_count, delta_seconds)
            }
            None => 0.0,
        };
        self.state.last_throughput_sample_time = Some(now);
        self.state.last_throughput_sample_count = total_updates;

        let n_priced = prices.iter().filter(|p| p.is_some()).count();
        log_warn!(
            "marketwatchv1: slot {} -- {}/{} symbols priced; prices={:?} volatilities={:?}; \
             throughput={:.0} updates/sec ({total_updates} total)",
            self.state.last_slot,
            n_priced,
            N_SYMBOLS,
            snapshot.prices,
            snapshot.volatilities,
            rate,
        );

        self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::MarketStats {
            slot: self.state.last_slot,
            snapshot: Box::new(snapshot),
            updates_per_sec: rate,
            total_updates,
        }));
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(SystemTime::now()));
            }
            MessageAction::AdjustConfiguration(new_configuration) => {
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(x) => match x {
                CustomMessageInbound::Blank => {}
                CustomMessageInbound::EchoRequest(s) => {
                    self.q_msg
                        .push_back(MessageSend::Custom(CustomMessageOutbound::EchoResponse(s)));
                }
            },
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        assert!(self.o_commit_slot.replace(slot).is_none());
        self.state.last_slot = slot;
        if slot % 100 == 0 {
            log_warn!(
                "marketwatchv1: pool account counts @ slot {slot}: commit_pools={} low_latency_pools={} low_latency_accounts={}",
                self.state.commit_pool_count,
                self.state.low_latency_pool_count,
                self.state.low_latency_account_count,
            );
            if let Some(router) = self.state.o_liquidity_router.as_ref() {
                log_warn!("marketwatchv1: price graph stats @ slot {slot}: {}", router.stats());
            }
            if let Some(dex) = self.state.o_dex.as_ref() {
                log_warn!("marketwatchv1: dex pool stats @ slot {slot}: {}", dex.pool_stats());
            }
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.state.commit_pool_count += 1;
        // This rooted stream is finalized but arrives ~12s late --
        // low_latency's ~400ms processed stream has almost always
        // already delivered the same or newer data for this account by
        // the time this fires. Only apply it (and refresh the account's
        // router edges) when it's genuinely newer than what low_latency
        // already recorded, matching arbv1's own gating exactly.
        if self.is_newer_than_low_latency(header.accountid, header.slot) {
            self.record_low_latency_slot(header.accountid, header.slot);
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(header, body);
                dex.refresh_account_router(header.accountid, &mut self.state.router);
            }
        }
    }

    fn on_token(&mut self, _token_account: &Tokenaccountv1) {
        // No wallet to feed, and (matching arbv1's own asymmetry) pool
        // vault-balance token updates are only ever fed to DexState via
        // the low_latency fast path in this bot's design, not the rooted
        // CommitHook path -- see low_latency's own doc comment.
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        if let Some(mut dex) = self.state.o_dex.take() {
            dex.flush_pool(self.graph, DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET)
                .expect("flush pool");
            // Drains a bounded slice of the queued startup subscription
            // burst per slot, instead of one giant blocking
            // bulk_subscribe call -- identical reasoning to arbv1's own
            // finish() (see SubscriptionQueue's own doc comment).
            if let Err(e) = dex.flush_subscriptions(self.graph, DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET) {
                log_error!("marketwatchv1: failed to flush dex subscription queue: {e}");
            }
            self.state.o_dex.replace(dex);
        }
    }
}
