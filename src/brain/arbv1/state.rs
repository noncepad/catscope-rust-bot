use crate::{
    brain::arbv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        strategy, Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate, SubscriptionQueue},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    router_config, router_pools_config,
    trader::{
        dex::{update::Updater as _, DexState},
        planner::{self, ArbitrageOpportunity},
        pricegraph::TradeRouter,
        router,
    },
    txview::TransactionList,
    util::{account_id_from_pubkey, rc_unlock},
    wallet::{PriorityLevel, Wallet},
};
use solana_sdk::{
    clock::Slot,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

/// Minimum wSOL input to avoid LiquidityUnderflow (0.01 SOL, 9 decimals).
const MIN_SWAP_SOL: u64 = 10_000_000;
/// Minimum USDC input to avoid LiquidityUnderflow (0.10 USDC, 6 decimals).
const MIN_SWAP_USDC: u64 = 100_000;
/// Rent-exempt minimum for an SPL token account (165 bytes). Kept in wSOL ATA to hold it open.
const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;

#[derive(Debug)]
enum Direction {
    ToSol,
    ToUSD,
}
#[derive(Debug)]
pub(crate) struct State {
    tx_count: usize,
    read_tx_count: usize,
    slot_delta_since_start: Slot,
    direction: Direction,
    last_slot: Slot,
    last_print: Instant,
    o_rc_keypair: Option<KeypairExtra>,
    o_bounce: Option<BounceStatus>,
    o_dex: Option<DexState>,
    /// 3-tier liquidity router (see ROUTER.md / trader::router) -- built once
    /// from the build-time router_pools_config snapshot in on_load(), not a
    /// live per-tick structure like `router` below.
    o_liquidity_router: Option<router::Router>,
    m_sig: HashMap<Signature, (Slot, Instant)>,
    tx_latency: TxLatencyStats,
    q_sig: VecDeque<SlotSignatureSet>,
    tracker: TransactionTracker,
    pub(crate) echo: bool,
    recycle_q: VecDeque<SlotSignatureSet>,
    o_ata_sol: Option<AccountId>,
    o_ata_usd: Option<AccountId>,
    log_stats: LatencyStatistics,
    router: TradeRouter,
    /// Lifetime count of pool/market/reserve account updates seen via the
    /// `Event::Commit` path (rooted accounts, ~12s latency) -- incremented
    /// once per `CommitHook::on_account` call.
    commit_pool_count: u64,
    /// Lifetime count of pool/market/reserve account updates seen via the
    /// `Event::LowLatency` path (processed accounts, ~400ms latency) --
    /// incremented once per account in `low_latency`'s account loop.
    low_latency_pool_count: u64,
    /// Lifetime count of every item (token accounts + regular accounts)
    /// read out of a `LowLatencyAccountUpdate` -- a superset of
    /// `low_latency_pool_count` (which counts only the account, not
    /// token, side), so comparing the two shows whether the low-latency
    /// channel is alive at all vs. simply never carrying pool accounts.
    low_latency_account_count: u64,
    /// Lifetime count of `planner::find_opportunity` calls -- incremented
    /// unconditionally in `detect_and_log_opportunity` every time it
    /// actually runs, regardless of the `% 100` gate that throttles that
    /// function's "no profitable cycle found"/"no wallet keypair yet" log
    /// lines (a real earlier miscount: those log lines are a ~1%-ish
    /// sample of calls, not the call count itself -- this field is the
    /// ground truth). Compare against `low_latency_account_count` (via
    /// the periodic `pool account counts` log line) to see how many
    /// account/token updates each `find_opportunity` call is effectively
    /// running against on average.
    find_opportunity_call_count: u64,
    /// Set by `CommitHook::finish` whenever `planner::find_opportunity`
    /// finds a cycle, consumed (via `.take()`) by
    /// `StateHelper::build_execution_plan` from `evaluate()` -- see that
    /// method's doc comment for why the plan-building step lives in
    /// `evaluate()` rather than inline in `finish()`.
    o_pending_opportunity: Option<ArbitrageOpportunity>,
    /// Latest `Slot` `low_latency`'s ~400ms processed-account stream has
    /// recorded for each account id -- lets
    /// `StateHelper::is_newer_than_low_latency` reject a
    /// `CommitHook::on_account` (~12s rooted) update when `low_latency`
    /// has already delivered the same or newer data for that account,
    /// instead of letting the (finalized, but consequently often
    /// stale-by-the-time-it-arrives) rooted stream roll pool state
    /// backwards. Only ever gates the rooted stream -- `low_latency`
    /// itself always writes here unconditionally, never gated by it (see
    /// `record_low_latency_slot`'s doc comment for why gating
    /// `low_latency` against its own past updates was a real bug: a
    /// single `Slot` can carry multiple separate updates for the same
    /// account).
    m_account_slot: HashMap<AccountId, Slot>,
}

#[derive(Debug)]
struct LatencyStatistics {
    commit_start: SystemTime,
    commit_finish: SystemTime,
    account_read: usize,
    account_filtered: usize,
    tx_read: usize,
    tx_filtered: usize,
    report: LatencyReportV1,
}
impl Default for LatencyStatistics {
    fn default() -> Self {
        let t = SystemTime::now();
        Self {
            commit_start: t,
            commit_finish: t,
            account_read: 0,
            account_filtered: 0,
            tx_read: 0,
            tx_filtered: 0,
            report: LatencyReportV1::default(),
        }
    }
}
impl LatencyStatistics {
    fn on_commit_start(&mut self) {
        self.commit_start = SystemTime::now();
        self.report.processed_diff = self
            .commit_start
            .duration_since(self.commit_finish)
            .unwrap();
    }
    fn on_account_processed(&mut self, count: usize) {
        self.report.account_processed += count;
    }
    fn on_account_root(&mut self, count: usize) {
        self.report.account_root += count;
    }
    fn on_tx_processed_filter(&mut self, count: usize) {
        self.report.tx_processed_filter += count;
    }
    fn on_tx_processed(&mut self, count: usize) {
        self.report.tx_processed += count;
    }
    fn on_commit_finish(&mut self) -> LatencyReportV1 {
        self.commit_finish = SystemTime::now();
        self.report.root_diff = self
            .commit_finish
            .duration_since(self.commit_start)
            .unwrap();
        let report = self.report.clone();
        self.report.reset();
        report
    }
}

#[derive(Debug, Clone, Default)]
pub struct LatencyReportV1 {
    pub processed_diff: std::time::Duration,
    pub root_diff: std::time::Duration,
    pub account_processed: usize,
    pub account_root: usize,
    pub tx_processed_filter: usize,
    pub tx_processed: usize,
    pub tx_n: u64,
    pub tx_p50_us: u64,
    pub tx_p99_us: u64,
}
impl LatencyReportV1 {
    fn reset(&mut self) {
        self.account_processed = 0;
        self.account_root = 0;
        self.tx_processed_filter = 0;
        self.tx_processed = 0;
        self.tx_n = 0;
        self.tx_p50_us = 0;
        self.tx_p99_us = 0;
    }
}

/// make sure we do not send duplicate transactions
#[derive(Debug, Default)]
struct TransactionTracker {
    // swap from SOL to USD
    o_tx_sol_to_usd: Option<Slot>,
}

#[derive(Debug, Default)]
struct TxLatencyStats {
    samples: Vec<u64>,
}

impl TxLatencyStats {
    fn record(&mut self, elapsed: Duration) {
        self.samples.push(elapsed.as_micros() as u64);
    }
    fn take_stats(&mut self) -> (u64, u64, u64) {
        if self.samples.is_empty() {
            return (0, 0, 0);
        }
        self.samples.sort_unstable();
        let n = self.samples.len() as u64;
        let p50 = self.samples[(self.samples.len().saturating_sub(1) * 50) / 100];
        let p99 = self.samples[(self.samples.len().saturating_sub(1) * 99) / 100];
        self.samples.clear();
        (n, p50, p99)
    }
}

#[derive(Debug)]
struct SignatureWithSlot {
    sent_slot: Slot,
    signature: Signature,
}

#[derive(Debug, Default)]
struct SlotSignatureSet {
    slot: Slot,
    hs_sig: HashSet<Signature>,
}
#[derive(Debug)]
struct BounceStatus {
    last_slot: Slot,
}

#[derive(Debug)]
struct KeypairExtra {
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}
impl State {
    fn wallet(&self) -> Option<AccountId> {
        let ke = self.o_rc_keypair.as_ref()?;
        Some(ke.account_id)
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            log_stats: LatencyStatistics::default(),
            slot_delta_since_start: 0,
            tx_count: 0,
            read_tx_count: 0,
            commit_pool_count: 0,
            low_latency_pool_count: 0,
            low_latency_account_count: 0,
            find_opportunity_call_count: 0,
            direction: Direction::ToUSD,
            o_dex: None,
            o_liquidity_router: None,
            o_bounce: None,
            last_slot: 0,
            last_print: Instant::now(),
            o_rc_keypair: None,
            m_sig: HashMap::default(),
            tx_latency: TxLatencyStats::default(),
            q_sig: VecDeque::default(),
            tracker: TransactionTracker::default(),
            recycle_q: VecDeque::default(),
            echo: false,
            o_ata_sol: None,
            o_ata_usd: None,
            router: TradeRouter::default(),
            o_pending_opportunity: None,
            m_account_slot: HashMap::default(),
        }
    }
}

/// Build the 3-tier `trader::router::Router` from the build-time
/// `router_config`/`router_pools_config` snapshot embedded from prefetch.db.
/// Must run after the host has resolved accounts (i.e. from `on_load`, like
/// `DexState::new`) since `account_id_from_pubkey` panics on unknown pubkeys.
fn build_liquidity_router() -> router::Router {
    let cfg = &router_config::ROUTER_CONFIG;
    let mut r = router::Router::new(cfg.token_count, cfg.lambda);
    for core_mint in cfg.core_mints {
        r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(core_mint)));
    }
    let mut pools = Vec::with_capacity(router_pools_config::ROUTER_POOLS.len());
    for p in router_pools_config::ROUTER_POOLS {
        let token_a = r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(p.mint_a)));
        let token_b = r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(p.mint_b)));
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
    pub(crate) graph: &'a mut Graph,
    pub(crate) nonce: &'a mut u32,
    pub(crate) o_commit_slot: Option<Slot>,
    pub(crate) state: &'a mut State,
    pub(crate) wallet: &'a mut Wallet,
    pub(crate) configuration: &'a mut Configuration,
    pub(crate) q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn nonce_check(&mut self, other_nonce: u32) -> Result<(), CatscopeGuestError> {
        if *self.nonce != other_nonce {
            return Err(CatscopeGuestError::BadNonce(*self.nonce, other_nonce));
        }
        *self.nonce += 1;
        Ok(())
    }
    /// Subscribe to relevant accounts.
    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        self.state.echo = std::env::args().any(|a| a == "--echo");
        assert!(self
            .state
            .o_dex
            .replace(DexState::new().expect("dex state"))
            .is_none());
        assert!(self
            .state
            .o_liquidity_router
            .replace(build_liquidity_router())
            .is_none());
        // Seed the live trade graph's node set from the liquidity router's
        // classified mint universe -- see TradeRouter::from_router's doc
        // comment. Must run after o_liquidity_router is built (just above).
        self.state.router =
            TradeRouter::from_router(self.state.o_liquidity_router.as_ref().unwrap());
        if let Some(x) = self.state.o_rc_keypair.as_ref() {
            self.configuration.set(&x.rc_keypair);
        }
        log_info!("bot has been successfully uploaded to validator");
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        match status {
            SlotStatus::Processed => {}
            SlotStatus::Rooted => {}
            SlotStatus::Confirmed => {}
            SlotStatus::FirstShredReceived => {}
            SlotStatus::Completed => {}
            SlotStatus::CreatedBank => {}
            SlotStatus::Dead => log_info!("slot {slot}; status dead"),
        }
    }

    /// Records `slot` as the latest slot `low_latency` has processed for
    /// `account_id` -- unconditional, never a gate. `Slot` is the coarse
    /// Solana block slot (~400-600ms); a single slot routinely carries
    /// *multiple* separate low-latency updates for the same account (e.g.
    /// two swaps against the same pool in one slot), each strictly newer
    /// than the last even though `header.slot` doesn't change between
    /// them. Gating on `slot <= last` here (an earlier version of this
    /// code did) silently dropped every update after the first per
    /// account per slot -- confirmed live: it stalled pool state and
    /// broke a previously-working SOL/USDC route within a few commits.
    /// `low_latency` is the primary driver of `router` now, so every call
    /// it makes must always be applied.
    fn record_low_latency_slot(&mut self, account_id: AccountId, slot: Slot) {
        self.state.m_account_slot.insert(account_id, slot);
    }

    /// Gate for `CommitHook::on_account`'s ~12s rooted stream only: true
    /// iff `slot` is strictly newer than whatever `record_low_latency_slot`
    /// has already recorded for `account_id`. The rooted stream is
    /// finalized but arrives late enough that `low_latency` has almost
    /// always already delivered the same or newer data for a given
    /// account by the time it shows up -- this stops a stale rooted
    /// duplicate from ever rolling pool/router state backwards, while
    /// still letting a genuinely-ahead rooted update (the rare case
    /// `low_latency` hasn't caught up to yet) through.
    fn is_newer_than_low_latency(&self, account_id: AccountId, slot: Slot) -> bool {
        match self.state.m_account_slot.get(&account_id) {
            Some(&last) => slot > last,
            None => true,
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        let do_stuff = *self.nonce == 0;
        if do_stuff {
            log_warn!(
                "low_latency hit!!!!!!!!!!!!!!! {} {}",
                llap.token_len(),
                llap.account_len()
            );
        }
        *self.nonce += 1;
        let mut account_count = 0;
        while let Some(ta) = llap.token() {
            account_count += 1;
            {
                let db = self.wallet.token_mut();
                db.on_token(ta, false);
            }
            if let Some(dex) = self.state.o_dex.as_mut() {
                _ = dex.on_token(ta);
                // Incrementally refresh just this pool's router edges --
                // O(pool out-degree), not a full rebuild -- this is the
                // only source of truth `router` has now; there's no
                // periodic full-rebuild safety net to fall back on (see
                // CommitHook::finish's doc comment for why that was
                // removed). No freshness gate needed here: unlike
                // on_account below, CommitHook never calls dex.on_token,
                // so low_latency is the only writer of token-driven pool
                // state and can't race against a rooted duplicate.
                dex.refresh_token_router(ta.id, &mut self.state.router);
            }
        }
        let zero = [];
        while let Some(account) = llap.account() {
            account_count += 1;
            self.state.low_latency_pool_count += 1;
            let d = if let Some(x) = account.body { x } else { &zero };
            self.record_low_latency_slot(account.header.accountid, account.header.slot);
            self.wallet.on_account(account.header, d);
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(account.header, d);
                dex.refresh_account_router(account.header.accountid, &mut self.state.router);
            }
        }
        self.state.low_latency_account_count += account_count as u64;
        self.state.log_stats.on_account_processed(account_count);
        if account_count > 0 {
            self.detect_and_log_opportunity();
        }
    }

    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        let mut l_a_size;
        let mut l_account_id = [0u64; 256];
        let mut signature;
        let mut l_program_id = [0u64; 256];
        let mut prog_i;
        let mut tx_processed = 0;
        let mut tx_processed_filter = 0;
        while let Some((mut tx, result)) = transaction_list.transaction() {
            tx_processed += 1;
            if result.is_err() {
                continue;
            }
            let slot = result.unwrap();
            l_a_size = tx.account.len();
            let inner_n = tx.ix_inner_len();
            let outer_n = tx.ix_outer_len();
            let l_a_subbuf = &mut l_account_id[0..l_a_size];
            l_a_subbuf.copy_from_slice(tx.account);
            l_a_subbuf.sort_unstable();
            {
                prog_i = 0;
                // track inner instructions
                for k in 0..inner_n {
                    let ix = tx.ix_inner(k);
                    l_program_id[prog_i] = *ix.program();
                    prog_i += 1;
                    tx = ix.into();
                }
                let l_prog = {
                    let subbuf = &mut l_program_id[0..prog_i];
                    subbuf.sort_unstable();
                    &l_program_id[0..prog_i]
                };
                // feed program_id index to various smart contract handlers
                if let Some(dex) = self.state.o_dex.as_mut() {
                    let mut source_l_p = [0; 100];
                    let n = dex.program_id_set(&mut source_l_p);
                    let l_p = &source_l_p[0..n];
                    for p_id in l_p {
                        if let Ok(i) = l_prog.binary_search(p_id) {
                            'doneix1: for k in i..inner_n {
                                let ix = tx.ix_inner(k);
                                if *ix.program() != *p_id {
                                    tx = ix.into();
                                    break 'doneix1;
                                }
                                tx_processed_filter += 1;
                                dex.on_tx(&ix, &slot);
                                tx = ix.into();
                            }
                        }
                    }
                }
            }
            {
                prog_i = 0;
                // track outer instructions
                for k in 0..outer_n {
                    let ix = tx.ix_outer(k);
                    l_program_id[prog_i] = *ix.program();
                    prog_i += 1;
                    tx = ix.into();
                }
                let l_prog = {
                    let subbuf = &mut l_program_id[0..prog_i];
                    subbuf.sort_unstable();
                    &l_program_id[0..prog_i]
                };
                // feed program_id index to various smart contract handlers --
                // a plain, non-CPI-wrapped swap is a top-level (outer)
                // instruction, so this needs the same dispatch the inner
                // loop above already does, or on_tx never sees it.
                if let Some(dex) = self.state.o_dex.as_mut() {
                    let mut source_l_p = [0; 100];
                    let n = dex.program_id_set(&mut source_l_p);
                    let l_p = &source_l_p[0..n];
                    for p_id in l_p {
                        if let Ok(i) = l_prog.binary_search(p_id) {
                            'doneix2: for k in i..outer_n {
                                let ix = tx.ix_outer(k);
                                if *ix.program() != *p_id {
                                    tx = ix.into();
                                    break 'doneix2;
                                }
                                tx_processed_filter += 1;
                                dex.on_tx(&ix, &slot);
                                tx = ix.into();
                            }
                        }
                    }
                }
            }

            signature = Signature::from(*tx.signature);
            self.state.read_tx_count += 1;
            if self.state.read_tx_count % 50_000 == 0 {
                log_warn!("read_tx_count {}", self.state.read_tx_count);
            }
            if let Some((slot2, sent_at)) = self.state.m_sig.remove(&signature) {
                let elapsed = sent_at.elapsed();
                self.state.tx_latency.record(elapsed);
                log_warn!(
                    "got transaction result {} {} {:?}; latency {}µs",
                    slot2,
                    signature,
                    slot2,
                    elapsed.as_micros()
                );
            }
        }
        self.state.log_stats.on_tx_processed(tx_processed);
        self.state
            .log_stats
            .on_tx_processed_filter(tx_processed_filter);
    }

    /// Run `planner::find_opportunity` against the current `state.router`
    /// and log the result -- factored out of `CommitHook::finish` so the
    /// same ~40-line find-and-log block also runs from the end of
    /// `low_latency()`'s incremental-refresh batch, not just once per
    /// ~12s commit. Sets `state.o_pending_opportunity` on a hit, exactly
    /// as before; the `% 100` gate on the "no cycle"/"no wallet" log
    /// lines is unchanged and applies regardless of which caller invokes
    /// this (it only throttles logging, not detection).
    pub(crate) fn detect_and_log_opportunity(&mut self) {
        if let Some(owner) = self.state.wallet() {
            self.state.find_opportunity_call_count += 1;
            self.state.router.set_current_slot(self.state.last_slot);
            match planner::find_opportunity(&self.state.router, self.wallet, &owner) {
                Some(opp) => {
                    // Re-verify any Orca CLMM hop against the exact,
                    // tick-aware quote before trusting this cycle -- see
                    // planner::reverify_with_exact_quotes's doc comment
                    // for the incident (a real pool's liquidity structure
                    // near the current tick made a hop the router's
                    // constant-product approximation priced as profitable
                    // actually untradeable) that made this necessary. A
                    // pool whose exact quote disagrees gets cooled down
                    // (excluded from the slippage-aware search for
                    // planner::POOL_COOLDOWN_SLOTS) so it stops being
                    // repeatedly re-selected and re-rejected every check.
                    let opp = match self.state.o_dex.as_ref() {
                        Some(dex) => match planner::reverify_with_exact_quotes(
                            &opp.cycle,
                            &self.state.router,
                            dex,
                        ) {
                            planner::ReverifyOutcome::Ok(cycle) => planner::ArbitrageOpportunity {
                                cycle,
                                wallet_balance: opp.wallet_balance,
                            },
                            planner::ReverifyOutcome::HopFailed(failure) => {
                                if failure.coolable {
                                    self.state
                                        .router
                                        .mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                                }
                                log_warn!(
                                    "arbitrage opportunity @ slot {}: rejected -- exact-quote re-verification invalidated \
                                     pool {} (cooling down {} slots: {}) (start_token={} amount_in={} hops={})",
                                    self.state.last_slot,
                                    failure.pool_id,
                                    planner::POOL_COOLDOWN_SLOTS,
                                    failure.coolable,
                                    opp.cycle.start_token(),
                                    opp.cycle.amount_in(),
                                    opp.cycle.hops.len(),
                                );
                                return;
                            }
                            planner::ReverifyOutcome::BelowThreshold(corrected) => {
                                log_warn!(
                                    "arbitrage opportunity @ slot {}: rejected -- below profit threshold after \
                                     exact-quote correction (start_token={} amount_in={} amount_out={} profit_raw={} profit_bps={} hops={})",
                                    self.state.last_slot,
                                    corrected.start_token(),
                                    corrected.amount_in(),
                                    corrected.amount_out(),
                                    corrected.profit_raw(),
                                    corrected.profit_bps(),
                                    corrected.hops.len(),
                                );
                                for (i, hop) in corrected.hops.iter().enumerate() {
                                    log_warn!(
                                        "  hop {}: dex={:?} pool={} {} -> {} : amount_in={} amount_out={}",
                                        i, hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                                    );
                                }
                                return;
                            }
                        },
                        None => opp,
                    };
                    // arbv1's own strategy filter (strategy.rs) -- the one
                    // place a coding agent should edit to change what this
                    // bot trades. Runs after the shared structural gates
                    // above (Bellman-Ford negative-cycle detection, exact-
                    // quote re-verification), never replacing them.
                    if !strategy::accept(&opp) {
                        log_warn!(
                            "arbitrage opportunity @ slot {}: rejected by arbv1 strategy filter \
                             (start_token={} amount_in={} profit_bps={} hops={})",
                            self.state.last_slot,
                            opp.cycle.start_token(),
                            opp.cycle.amount_in(),
                            opp.cycle.profit_bps(),
                            opp.cycle.hops.len(),
                        );
                        return;
                    }
                    log_warn!(
                        "arbitrage opportunity @ slot {}: start_token={} amount_in={} amount_out={} \
                         profit_raw={} profit_bps={} wallet_balance={} hops={}",
                        self.state.last_slot,
                        opp.cycle.start_token(),
                        opp.cycle.amount_in(),
                        opp.cycle.amount_out(),
                        opp.cycle.profit_raw(),
                        opp.cycle.profit_bps(),
                        opp.wallet_balance,
                        opp.cycle.hops.len(),
                    );
                    for (i, hop) in opp.cycle.hops.iter().enumerate() {
                        log_warn!(
                            "  hop {}: dex={:?} pool={} {} -> {} : amount_in={} amount_out={}",
                            i,
                            hop.dex,
                            hop.pool_id,
                            hop.input_mint,
                            hop.output_mint,
                            hop.amount_in,
                            hop.amount_out,
                        );
                    }
                    // Handed off to evaluate()'s build_execution_plan --
                    // consumed there via .take(), not replanned on every
                    // event until the next commit re-detects a cycle.
                    self.state.o_pending_opportunity = Some(opp);
                }
                None if self.state.last_slot % 100 == 0 => {
                    log_warn!(
                        "arbitrage check @ slot {}: no profitable cycle found",
                        self.state.last_slot,
                    );
                }
                None => {}
            }
        } else if self.state.last_slot % 100 == 0 {
            log_warn!(
                "arbitrage check @ slot {}: no wallet keypair yet",
                self.state.last_slot,
            );
        }
    }

    /// Build the real per-hop swap instructions for the most recently
    /// found arbitrage cycle (if any), against a throwaway scratch
    /// `Wallet` -- proves the price-graph-detected route is genuinely
    /// buildable (catching `WrongMints`/`PoolNotReady`/unknown-pool
    /// failures a pure price calculation can't see), but never touches
    /// `self.wallet` and never sends anything: `scratch` is never
    /// `assemble()`d, so `evaluate()`'s send loop can never pick these
    /// instructions up.
    ///
    /// Consumes `state.o_pending_opportunity` (set by `CommitHook::finish`)
    /// via `.take()`, so a given opportunity is only planned once, not
    /// replanned on every event until the next commit finds a new one.
    ///
    /// Only ever called from the real WASM guest runtime (same as
    /// `evaluate()` itself) -- `Wallet::new()`/`derive_ata`/every
    /// `plan_hop` adapter transitively call the `account_id_from_pubkey`/
    /// `pubkey_from_account_id` WIT host imports, which abort the process
    /// outside that runtime. This is why none of it is covered by native
    /// `cargo test` unit tests, matching this codebase's established
    /// testing boundary for every other `Wallet`-touching code path.
    pub(crate) fn build_execution_plan(&mut self) {
        let Some(opp) = self.state.o_pending_opportunity.take() else {
            return;
        };
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };

        log_warn!(
            "execution plan @ slot {}: {} hop{} for start_token={} amount_in={}",
            self.state.last_slot,
            opp.cycle.hops.len(),
            if opp.cycle.hops.len() == 1 { "" } else { "s" },
            opp.cycle.start_token(),
            opp.cycle.amount_in(),
        );

        // Real execution (2026-08-28, explicit instruction -- previously
        // this built everything onto a throwaway `scratch = Wallet::new()`
        // purely to compute/log what a real send *would* cost, and never
        // queued anything onto `self.wallet` -- arbv1 never actually
        // traded). Builds directly onto `self.wallet` now, inside a
        // checkpoint + atomic group (mirrors testperpv1::state::StateHelper::
        // execute_spot_leg's already-battle-tested discipline exactly, for the same reasons:
        // one atomic group ties every hop together so `assemble()` can't
        // split them across separate, unordered transactions; the
        // checkpoint lets any failure discard only *this* route's
        // instructions, not the whole queue). Whether the built route
        // actually gets sent is decided below, after real net profit is
        // known -- an unprofitable route is rolled back here, same as a
        // failed hop.
        let cu_before = self.wallet.cu();
        let checkpoint = self.wallet.queue_checkpoint();
        self.wallet.begin_atomic_group();
        for (i, hop) in opp.cycle.hops.iter().enumerate() {
            let (Some(source_ata), Some(dest_ata)) = (
                self.wallet.append_create_ata(owner, hop.input_mint),
                self.wallet.append_create_ata(owner, hop.output_mint),
            ) else {
                self.wallet.rollback_to(checkpoint);
                self.wallet.end_atomic_group();
                log_warn!("  hop {i}: FAILED to derive token account(s) for owner={owner}");
                return;
            };
            match dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet) {
                Ok(()) => {
                    log_warn!(
                        "  hop {i}: OK dex={:?} pool={} {} -> {} amount_in={} amount_out={}",
                        hop.dex,
                        hop.pool_id,
                        hop.input_mint,
                        hop.output_mint,
                        hop.amount_in,
                        hop.amount_out,
                    );
                }
                Err(e) => {
                    self.wallet.rollback_to(checkpoint);
                    self.wallet.end_atomic_group();
                    // `PoolNotReady` means "not enough live data observed
                    // yet" (its own doc comment), the exact same "ask
                    // again shortly" situation as `HopFailure::coolable`
                    // -- not a genuine bad pool on the *first* failure, so
                    // don't cool it down immediately. Fix (2026-09-07):
                    // live-confirmed a pool can fail this way identically
                    // for 30+ minutes across multiple restarts -- not
                    // self-correcting. `note_pool_not_ready` tracks the
                    // repeated case and reports once it's crossed a real
                    // threshold, so we still cool down eventually instead
                    // of retrying forever.
                    let is_pool_not_ready = matches!(e, crate::trader::types::TraderError::PoolNotReady);
                    let coolable = if is_pool_not_ready {
                        self.state.router.note_pool_not_ready(hop.pool_id)
                    } else {
                        true
                    };
                    if coolable {
                        if is_pool_not_ready {
                            self.state
                                .router
                                .mark_pool_not_ready_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        } else {
                            self.state
                                .router
                                .mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                    }
                    log_warn!(
                        "  hop {i}: FAILED dex={:?} pool={} {} -> {}: {} (cooling down {} slots: {coolable})",
                        hop.dex,
                        hop.pool_id,
                        hop.input_mint,
                        hop.output_mint,
                        e,
                        planner::POOL_COOLDOWN_SLOTS,
                    );
                    return;
                }
            }
        }
        if !self.wallet.atomic_group_fits(checkpoint) {
            self.wallet.rollback_to(checkpoint);
            self.wallet.end_atomic_group();
            for hop in opp.cycle.hops.iter() {
                self.state
                    .router
                    .mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
            }
            log_warn!(
                "execution plan @ slot {}: {}-hop atomic group too large for one transaction (cooling down {} slots)",
                self.state.last_slot,
                opp.cycle.hops.len(),
                planner::POOL_COOLDOWN_SLOTS,
            );
            return;
        }
        self.wallet.end_atomic_group();
        let route_cu = self.wallet.cu().saturating_sub(cu_before);

        log_warn!(
            "execution plan @ slot {}: all {} hops built successfully; total_cu={} instruction_count={}",
            self.state.last_slot,
            opp.cycle.hops.len(),
            route_cu,
            self.wallet.instruction_count().saturating_sub(checkpoint),
        );

        // Net profit after real transaction costs -- only meaningful when
        // start_token is SOL (net_profit_lamports's own doc comment
        // explains why a non-SOL start_token can't be netted against a
        // lamport fee without a separate price conversion). Uses the
        // REAL, built total_cu (not an estimate) and the same priority
        // level evaluate() actually bids at.
        if opp.cycle.start_token() == self.configuration.mint_sol {
            let priority_rate: u64 = PriorityLevel::Medium.into();
            let net_profit = opp.cycle.net_profit_lamports(route_cu, priority_rate, 1);
            log_warn!(
                "execution plan @ slot {}: net_profit={} lamports (gross_profit={} lamports, base_fee=5000, priority_fee_rate={} micro-lamports/cu, total_cu={})",
                self.state.last_slot,
                net_profit,
                opp.cycle.profit_raw(),
                priority_rate,
                route_cu,
            );
            if 0 < net_profit {
                // Explicit, 2026-08-28: a real positive net profit (after
                // fees, not just gross) is worth bidding urgently for --
                // set_priority_fee(High) also opts this tick's send into
                // trying real Astralane landing via
                // Wallet::send_bundler_pair (see that method's doc
                // comment), not just a higher compute-unit price. The
                // route built above stays queued; `evaluate()`'s own
                // `drain_and_send()` call sends it for real.
                self.wallet.set_priority_fee(PriorityLevel::High);
            } else {
                // Not profitable after real costs -- discard the route
                // built above rather than send a real loss.
                self.wallet.rollback_to(checkpoint);
                log_warn!(
                    "execution plan @ slot {}: net profit not positive, discarding route (not sent)",
                    self.state.last_slot,
                );
            }
        } else {
            // start_token isn't SOL, so there's no reliable real-cost-netted
            // profit signal to execute on (see net_profit_lamports's doc
            // comment) -- conservative default: discard rather than send
            // based on gross, incomparable-to-fees raw units alone.
            self.wallet.rollback_to(checkpoint);
            log_warn!(
                "execution plan @ slot {}: net profit not computed -- start_token={} is not SOL, gross_profit={} raw units isn't directly comparable to lamport transaction fees; discarding route (not sent)",
                self.state.last_slot,
                opp.cycle.start_token(),
                opp.cycle.profit_raw(),
            );
        }
    }

    pub(crate) fn evaluate(&mut self) {
        self.wallet.set_priority_fee(PriorityLevel::Medium);
        // Idempotent/self-latching (2026-08-28): only actually queues the
        // real create-nonce transaction once (Wallet::ensure_bundler_nonce_created
        // no-ops on every call after the first, whether still unconfirmed
        // or already Ready) -- safe to call unconditionally every tick so
        // the durable-nonce account this wallet's Astralane landing path
        // (Wallet::send_bundler_pair, driven by set_priority_fee(High) --
        // see its own doc comment) needs is bootstrapped automatically at
        // wallet load time instead of requiring a manual trigger.
        if let Some(owner) = self.state.wallet() {
            self.wallet.ensure_bundler_nonce_created(owner);
        }
        if self.configuration.wallet == 0 {
            if let Some(x) = self.state.o_rc_keypair.as_ref() {
                self.configuration.set(&x.rc_keypair);
                let db = self.wallet.token_mut();
                let owner = self.state.wallet().unwrap();
                let l_a = db.balance(&owner, &self.configuration.mint_sol, true);
                if !l_a.is_empty() {
                    log_warn!("wallet already has SOL balance at init: {l_a:?}");
                }
            } else {
                return;
            }
        }
        if self.state.slot_delta_since_start < 20 {
            return;
        }
        if 10 < self.state.tx_count {
            return;
        }

        self.build_execution_plan();

        for (sig, result) in self.wallet.drain_and_send() {
            match result {
                Ok(_) => {
                    self.state.tx_count += 1;
                    let mut ss = if self
                        .state
                        .q_sig
                        .front()
                        .map_or(false, |s| s.slot == self.state.last_slot)
                    {
                        self.state.q_sig.pop_front().unwrap()
                    } else {
                        let mut ss2 = self.state.recycle_q.pop_front().unwrap_or_default();
                        ss2.hs_sig.clear();
                        ss2.slot = self.state.last_slot;
                        ss2
                    };
                    ss.hs_sig.insert(sig);
                    self.state
                        .m_sig
                        .insert(sig, (self.state.last_slot, Instant::now()));
                    self.state.q_sig.push_front(ss);
                    self.state.direction = match self.state.direction {
                        Direction::ToUSD => Direction::ToSol,
                        Direction::ToSol => Direction::ToUSD,
                    };
                }
                Err(e) => log_error!("failed to send tx {e}"),
            };
        }
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
                log_warn!("StateHelper::on_message------------------------------- adjust config");
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(x) => {
                log_warn!("StateHelper::on_message------------------------------- custom");
                match x {
                    CustomMessageInbound::Blank => {
                        log_warn!("StateHelper::on_message------------------------------- blank");
                    }
                    CustomMessageInbound::EchoRequest(s) => {
                        log_warn!("StateHelper::on_message-------------------------------echo request {s}");
                        self.q_msg.push_back(MessageSend::Custom(
                            CustomMessageOutbound::EchoResponse(s.clone()),
                        ));
                    }
                    CustomMessageInbound::Wallet(rc_keypair) => {
                        let keypair = rc_unlock(&rc_keypair);
                        let pubkey = keypair.pubkey();
                        let account_id = account_id_from_pubkey(&pubkey);
                        self.state.o_ata_sol = None;
                        self.state.o_ata_usd = None;
                        log_warn!(
                            "StateHelper::on_message-------------------------------got keypair {} {}",
                            pubkey,account_id
                        );
                        self.wallet
                            .append_key(rc_keypair.clone(), self.graph)
                            .unwrap();
                        self.wallet.set_payer(account_id);
                        // This wallet's own durable-nonce account (see
                        // `Wallet::send_bundler_pair`'s doc comment).
                        if let Some(req) = self.wallet.nonce_subscribe_request(account_id) {
                            match SubscriptionQueue::subscribe_now(self.graph, vec![req]) {
                                Ok(mut subs) => {
                                    if let Some(sub) = subs.pop() {
                                        self.wallet.keep_nonce_subscription(sub);
                                    }
                                }
                                Err(e) => {
                                    log_error!(
                                        "arbv1: failed to subscribe to bundler nonce account: {e}"
                                    );
                                }
                            }
                        }
                        if let Some(x2) = self.state.o_rc_keypair.replace(KeypairExtra {
                            rc_keypair,
                            account_id,
                        }) {
                            let old_keypair = rc_unlock(&x2.rc_keypair);
                            log_warn!(
                                "StateHelper::on_message-------------------------------deleting keypair {} {}",
                                old_keypair.pubkey(),x2.account_id
                            );
                        }
                    }
                    CustomMessageInbound::CommonBundlerTipUpdate(update) => {
                        self.wallet.apply_bundler_tip_update(self.graph, update);
                    }
                }
            }
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

/// How often (in slots) to report this wallet's most-referenced accounts
/// back to the optimizer via `MessageSend::CommonAddressUpdate`, so
/// `optimizer alt` can rank real Address Lookup Table candidates without
/// an RPC history scan.
const ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS: Slot = 500;
/// Caps each report comfortably under `MESSAGE_MAX_SIZE` (4096 bytes):
/// `28 + 100*36 = 3628`.
const ACCOUNT_USAGE_REPORT_MAX_ENTRIES: usize = 100;

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        self.state.log_stats.on_commit_start();
        assert!(self.o_commit_slot.replace(slot).is_none());
        if slot % 100 == 0 {
            log_warn!(
                "CommitHook - start {slot}; orca {} {}; read_tx_count {}",
                0,
                0,
                self.state.read_tx_count,
            );
            log_warn!(
                "pool account counts @ slot {slot}: commit_pools={} low_latency_pools={} low_latency_accounts={} find_opportunity_calls={}",
                self.state.commit_pool_count,
                self.state.low_latency_pool_count,
                self.state.low_latency_account_count,
                self.state.find_opportunity_call_count,
            );
            if let Some(router) = self.state.o_liquidity_router.as_ref() {
                log_warn!("price graph stats @ slot {slot}: {}", router.stats());
            }
            if let Some(dex) = self.state.o_dex.as_ref() {
                log_warn!("dex pool stats @ slot {slot}: {}", dex.pool_stats());
                log_warn!(
                    "marginfi oracle status @ slot {slot}: {}",
                    dex.debug_marginfi_oracle_status()
                );
            }
        } else {
            log_warn!(
                "CommitHook - start {}; slot_delta_since_start {}; orca {} {}; read_tx_count {}",
                slot,
                self.state.slot_delta_since_start,
                0,
                0,
                self.state.read_tx_count,
            );
        }
        self.state.last_slot = slot;
        'done1: while let Some(mut ss) = self.state.q_sig.pop_front() {
            if slot < ss.slot {
                self.state.q_sig.push_front(ss);
                break 'done1;
            }
            for sig in ss.hs_sig.drain() {
                if let Some((old_slot, _)) = self.state.m_sig.remove(&sig) {
                    log_warn!(
                        "transaction expired: slot {} -> {}; signature {}",
                        old_slot,
                        slot,
                        sig
                    );
                }
            }
            ss.slot = 0;
            assert!(ss.hs_sig.is_empty());
            self.state.recycle_q.push_back(ss);
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.wallet.on_account(header, body);
        self.state.log_stats.on_account_root(1);
        self.state.commit_pool_count += 1;
        // This rooted stream is finalized but arrives ~12s late --
        // low_latency's ~400ms processed stream has almost always already
        // delivered the same or newer data for this account by the time
        // this fires. Only apply it (and refresh the account's router
        // edges) when it's genuinely newer than what low_latency already
        // recorded, so a late rooted duplicate can never roll pool or
        // router state backwards. See `is_newer_than_low_latency`'s doc
        // comment and `finish`'s (the periodic full-rebuild this replaced
        // is gone -- router is now maintained purely incrementally).
        if self.is_newer_than_low_latency(header.accountid, header.slot) {
            self.record_low_latency_slot(header.accountid, header.slot);
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(header, body);
                dex.refresh_account_router(header.accountid, &mut self.state.router);
            }
        }
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        self.state.log_stats.on_account_root(1);
        let db = self.wallet.token_mut();
        db.on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        self.state.slot_delta_since_start += 1;
        if let Some(mut dex) = self.state.o_dex.take() {
            dex.flush_pool(self.graph, planner::DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET).expect("flush pool");
            // Drains a bounded slice of the queued startup subscription
            // burst per slot (~32,000 requests total across every
            // sub-dex), instead of one giant blocking bulk_subscribe call
            // at DexState::new() time -- see SubscriptionQueue's own doc
            // comment for the real, live-observed motivation (found while
            // debugging `testperpv1`'s real-transaction smoke test).
            if let Err(e) = dex.flush_subscriptions(self.graph, 128) {
                log_error!("arbv1: failed to flush dex subscription queue: {e}");
            }
            self.state.o_dex.replace(dex);
        }
        // No periodic full clear()+batch_router() rebuild here anymore --
        // that rebuilt from *rooted* pool state, which is ~12s stale by
        // definition (that's how long it takes an account to finalize),
        // so a full rebuild would periodically stomp router edges
        // low_latency's ~400ms processed stream had already moved past.
        // `router` is now maintained purely incrementally: every
        // low_latency account/token update refreshes its own pool's
        // edges directly (see low_latency's doc comments), and
        // CommitHook::on_account above does the same but only when a
        // rooted update turns out to be genuinely fresher than what
        // low_latency already delivered (via is_newer_than_low_latency).
        self.detect_and_log_opportunity();
        if self.state.last_slot % 100 == 0 {
            if let Some(router) = self.state.o_liquidity_router.as_ref() {
                let stats = router.stats();
                log_warn!(
                    "liquidity tiers @ slot {}: tier1 sovereign-core={} tokens; \
                     tier2 liquid-clusters={} clusters / {} tokens; \
                     tier3 long-tail-spokes={} tokens; total={} tokens",
                    self.state.last_slot,
                    stats.tier1_count,
                    stats.tier2_cluster_count,
                    stats.tier2_token_count,
                    stats.tier3_spoke_count,
                    stats.token_count,
                );
            }
            // Diagnostic: price a configurable probe size (default 0.1
            // SOL, see build.rs's trade_router_probe_lamports doc comment)
            // -> USDC through the live TradeRouter so we can watch whether
            // it's actually finding routes. Sized to roughly the user's
            // real willing trade size -- a much larger probe (this used to
            // be hardcoded to 100 SOL) can legitimately crater against a
            // thin-but-real pool's constant-product curve (correct
            // slippage, not a bug), which made 100 SOL a misleading
            // stand-in for "is this route actually tradeable at the size
            // I'd use."
            //
            // Uses route_slippage_aware, not route -- this diagnostic was
            // deliberately left on the amount-blind route() through this
            // session's Orca pricing investigation so its long-tracked
            // baseline values stayed directly comparable across restarts;
            // now that that investigation (and the m_pool indexing bug it
            // surfaced) is settled, there's no more reason to keep it on
            // the spot-price-only path selection instead of the same
            // slippage-aware search planner::find_opportunity now uses for
            // real opportunity detection.
            const USDC_DECIMALS: u32 = 6;
            let amount_in = crate::diagnostic_config::TRADE_ROUTER_PROBE_LAMPORTS;
            let probe_sol = amount_in as f64 / 1_000_000_000.0;
            self.state.router.set_current_slot(self.state.last_slot);
            match self.state.router.route_slippage_aware(
                self.configuration.mint_sol,
                self.configuration.mint_usdc,
                amount_in,
                4,
            ) {
                Some(route) => {
                    // Re-verify any Orca CLMM hop against the exact,
                    // tick-aware quote before trusting this route -- see
                    // planner::reverify_route_with_exact_quotes's doc
                    // comment for the incident (a real pool's liquidity
                    // structure near the current tick made a hop the
                    // router's constant-product approximation price as
                    // profitable actually untradeable) that made this
                    // necessary. A pool whose exact quote disagrees gets
                    // cooled down (excluded from the slippage-aware
                    // search for planner::POOL_COOLDOWN_SLOTS) so it
                    // stops being repeatedly re-selected and re-rejected
                    // every check. Matched (not `return` on rejection)
                    // since there's more unrelated periodic diagnostic
                    // logging after this whole match.
                    let corrected = match self.state.o_dex.as_ref() {
                        Some(dex) => planner::reverify_route_with_exact_quotes(
                            &route,
                            amount_in,
                            &self.state.router,
                            dex,
                        ),
                        None => Ok(route),
                    };
                    if let Ok(route) = corrected {
                        let usdc_out = route.amount_out() as f64 / 10f64.powi(USDC_DECIMALS as i32);
                        log_warn!(
                            "trade router check @ slot {}: {:.9} SOL -> {:.6} USDC ({} hop{})",
                            self.state.last_slot,
                            probe_sol,
                            usdc_out,
                            route.n_hops(),
                            if route.n_hops() == 1 { "" } else { "s" },
                        );
                        // Independent ground truth: compare the router's
                        // own implied SOL/USD price against Pyth's own
                        // SOL/USD feed, subscribed to directly (not
                        // mediated through any lending protocol's bank
                        // config -- marginfi's tracked bank set turned out
                        // to have no SOL bank at all, see
                        // `pyth::PythFeedState`'s doc comment) -- turns
                        // "this looks roughly right" into a measured
                        // delta.
                        if probe_sol > 0.0 {
                            let router_price_usd = usdc_out / probe_sol;
                            if let Some(dex) = self.state.o_dex.as_ref() {
                                if let Some(pyth_price) = dex.pyth_sol_usd_price() {
                                    let delta_pct = (router_price_usd - pyth_price.price_usd)
                                        / pyth_price.price_usd
                                        * 100.0;
                                    log_warn!(
                                        "trade router check @ slot {}: router SOL/USD={:.4} pyth SOL/USD={:.4} (conf={:.4}) delta={:+.2}%",
                                        self.state.last_slot,
                                        router_price_usd,
                                        pyth_price.price_usd,
                                        pyth_price.confidence_usd,
                                        delta_pct,
                                    );
                                }
                            }
                        }
                        // TEMPORARY DEBUG: per-hop pool/dex breakdown --
                        // every reading since switching this diagnostic to
                        // route_slippage_aware has quoted noticeably above
                        // real market rate (never below), suspected
                        // "winner's curse" from always picking the single
                        // best-looking price among many now-thinner pools
                        // (post pool-budget increase), not a bug --
                        // confirm by checking whether the picked pool(s)
                        // are consistently thin/rarely-traded. Remove once
                        // settled.
                        for (i, hop) in route.hops.iter().enumerate() {
                            log_warn!(
                                "  route hop {}: dex={:?} pool={} {} -> {} : amount_in={} amount_out={}",
                                i,
                                hop.dex,
                                hop.pool_id,
                                hop.input_mint,
                                hop.output_mint,
                                hop.amount_in,
                                hop.amount_out,
                            );
                            // TEMPORARY DEBUG: raw pool pricing fields, see
                            // OrcaState::debug_pool_state's doc comment.
                            if hop.dex == crate::trader::types::DexType::OrcaWhirlpool {
                                if let Some(dex) = self.state.o_dex.as_ref() {
                                    if let Some(info) = dex.debug_orca_pool_state(hop.pool_id) {
                                        log_warn!("    orca pool state: {info}");
                                    }
                                    // TEMPORARY DEBUG: exact tick-aware
                                    // quotes across a size range, see
                                    // OrcaState::debug_quote_range's doc
                                    // comment -- answers "how much could
                                    // actually be sold before slippage/
                                    // tick-crossing eats the edge" using
                                    // real quote math instead of a hand-
                                    // rolled estimate.
                                    const PROBE_SIZES_SOL_RAW: [u64; 8] = [
                                        100_000_000,    // 0.1 SOL
                                        1_000_000_000,  // 1
                                        5_000_000_000,  // 5
                                        10_000_000_000, // 10
                                        20_000_000_000, // 20
                                        30_000_000_000, // 30
                                        40_000_000_000, // 40
                                        50_000_000_000, // 50
                                    ];
                                    if let Some(info) = dex.debug_orca_quote_range(
                                        hop.pool_id,
                                        hop.input_mint,
                                        &PROBE_SIZES_SOL_RAW,
                                    ) {
                                        log_warn!("    orca exact quote range: {info}");
                                    }
                                    // TEMPORARY DEBUG: see
                                    // OrcaState::debug_tick_array_status's
                                    // doc comment -- disambiguates real
                                    // zero liquidity from an incomplete
                                    // local tick-array cache.
                                    if let Some(info) = dex
                                        .debug_orca_tick_array_status(hop.pool_id, hop.input_mint)
                                    {
                                        log_warn!("    orca tick array status: {info}");
                                    }
                                }
                            }
                        }
                    } else if let Err(failure) = corrected {
                        if failure.coolable {
                            self.state
                                .router
                                .mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                        log_warn!(
                            "trade router check @ slot {}: rejected -- exact CLMM quote invalidated pool {} \
                             (cooling down {} slots: {}) for {:.9} SOL -> USDC",
                            self.state.last_slot,
                            failure.pool_id,
                            planner::POOL_COOLDOWN_SLOTS,
                            failure.coolable,
                            probe_sol,
                        );
                    }
                }
                None => {
                    log_warn!(
                        "trade router check @ slot {}: no route found for {:.9} SOL -> USDC",
                        self.state.last_slot,
                        probe_sol,
                    );
                    let diag = self.state.router.route_diagnose(
                        self.configuration.mint_sol,
                        self.configuration.mint_usdc,
                        amount_in,
                        4,
                    );
                    log_warn!(
                        "trade router diagnose @ slot {}: {diag}",
                        self.state.last_slot
                    );
                }
            }
            log_warn!(
                "trade router graph @ slot {}: total_edges={} sol_out_degree={} usdc_out_degree={}",
                self.state.last_slot,
                self.state.router.edge_count(),
                self.state.router.out_degree(self.configuration.mint_sol),
                self.state.router.out_degree(self.configuration.mint_usdc),
            );
            let by_dex = self.state.router.edge_counts_by_dex();
            let by_dex_str = by_dex
                .iter()
                .map(|(dex, n)| format!("{dex:?}={n}"))
                .collect::<Vec<_>>()
                .join(" ");
            log_warn!(
                "trade router edges by dex @ slot {}: {by_dex_str}",
                self.state.last_slot,
            );
            // Informational only -- reads the live router for its best-
            // instant-price leg but doesn't feed anything back into it
            // (see MarinadeState::compare_unstake_paths's doc comment).
            // 1 whole mSOL (9 decimals, same scale as lamports), not
            // planner::PROBE_AMOUNT_RAW's tiny probe size -- this is for a
            // human-readable comparison, not path selection.
            const ONE_MSOL_RAW: u64 = 1_000_000_000;
            if let Some(dex) = self.state.o_dex.as_ref() {
                if let Some(cmp) = dex.marinade().compare_unstake_paths(
                    ONE_MSOL_RAW,
                    self.state.last_slot,
                    &self.state.router,
                ) {
                    log_warn!(
                        "marinade unstake comparison @ slot {}: 1 mSOL -> instant={} ({} hop{} via {:?}) \
                         delayed={} (~{} epoch{}) lamports",
                        self.state.last_slot,
                        cmp.instant_out,
                        cmp.instant_route.len(),
                        if cmp.instant_route.len() == 1 { "" } else { "s" },
                        cmp.instant_route,
                        cmp.delayed_out,
                        cmp.wait_epochs,
                        if cmp.wait_epochs == 1 { "" } else { "s" },
                    );
                }
            }
        }
        let mut report = self.state.log_stats.on_commit_finish();
        let (tx_n, tx_p50_us, tx_p99_us) = self.state.tx_latency.take_stats();
        report.tx_n = tx_n;
        report.tx_p50_us = tx_p50_us;
        report.tx_p99_us = tx_p99_us;
        self.q_msg
            .push_back(MessageSend::Custom(CustomMessageOutbound::LatencyReportV1(
                report,
            )));
        // Periodically report this wallet's most-referenced accounts back
        // to the optimizer -- see ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS.
        if self.state.last_slot % ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS == 0 {
            let top = self
                .wallet
                .top_account_usage(ACCOUNT_USAGE_REPORT_MAX_ENTRIES);
            if !top.is_empty() {
                self.q_msg.push_back(MessageSend::CommonAddressUpdate(top));
            }
        }
    }
}
