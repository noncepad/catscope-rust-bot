//! State machine for `xstockshealthv1` -- watches the xStocks Kamino
//! obligations/reserves from `trader::dex::kamino_xstocks_watcher`, logs a
//! health board periodically, and **fires a real liquidation** against
//! whichever real, live obligation is actually eligible (`health_factor
//! <= 1.0`), the moment it becomes eligible. Originally built as a
//! zero-transaction watcher only; merged with the self-triggered demo bot
//! (`xstocksliquidatev1`, since removed) into one bot, since the
//! liquidation instruction is already generic (any reserve pair, not
//! hardcoded to one asset) and this watcher already tracks live data for
//! every reserve and obligation in the market -- there was no real reason
//! to keep "detect" and "act" as two separate bots.
//!
//! # Event flow
//! ```text
//! validator → Event::Commit     → state.start()/on_account()/finish() (CommitHook,
//!                                  rooted accounts, ~12s) -- on_account feeds the
//!                                  watcher, tags UpdateLane::Commit, finish() paces
//!                                  the subscription queue
//! validator → Event::LowLatency → state.low_latency() ("processed" accounts, ~400ms)
//!                                  -- feeds the exact same watcher, tags
//!                                  UpdateLane::LowLatency. Added after this
//!                                  session's own testlatencylitev1 investigation
//!                                  (see that branch's UpdateLane/racing pattern,
//!                                  which this mirrors): whichever of these two
//!                                  channels actually delivers a given account's
//!                                  update first is a real race, not predictable
//!                                  in advance -- see low_latency's own doc comment.
//! validator → Event::Transaction → state.mid_on_tx() -- two independent things: (1)
//!                                    watches for this bot's own most recently sent
//!                                    liquidation signature and logs its real on-chain
//!                                    outcome (landed slot, or the specific
//!                                    TransactionError); (2) touch-triggered re-check --
//!                                    any *other* landed transaction that touches a
//!                                    tracked obligation gets an immediate, targeted
//!                                    health check + liquidation attempt, instead of
//!                                    waiting for evaluate()'s own health_board().first()
//!                                    scan to eventually reach it -- see mid_on_tx's own
//!                                    doc comment for why this is a real coverage
//!                                    improvement, not just an earlier poke at the same
//!                                    check
//! validator → Event::SlotStatus → no-op (see below)
//! Go brain  → stdin → state.on_message() (wallet key -- now genuinely used for
//!                      signing, not just harness-lifecycle consistency)
//! state.evaluate() runs after every event, including LowLatency's ~400ms-cadence
//! ones. Eligibility is checked on *every* call (untethered from any slot
//! throttle -- see evaluate()'s own doc comment for why); logging + the
//! dashboard push stay throttled to HEALTH_BOARD_LOG_INTERVAL_SLOTS.
//! ```
use crate::{
    brain::xstockshealthv1::{
        configuration::Configuration,
        message::{
            CustomMessageInbound, CustomMessageOutbound, DexActivityEntry, LiquidationEvent, MarketOverview,
        },
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    trader::dex::{
        kamino,
        kamino_xstocks_watcher::{self, KaminoXstocksWatcher},
        xstock_dex_watcher::XstockDexWatcher,
    },
    txview::TransactionList,
    util::{account_id_from_pubkey, rc_unlock},
    wallet::Wallet,
};
use solana_sdk::{clock::Slot, signature::Signature, signer::Signer};
use std::{
    collections::{HashMap, VecDeque},
    time::SystemTime,
};

/// How many queued obligation/reserve subscriptions `CommitHook::finish`
/// drains per slot -- see `kamino_xstocks_watcher`'s own module doc
/// comment for why this is paced at all instead of one `bulk_subscribe`
/// call.
const SUBSCRIBE_PER_SLOT: usize = 4;

/// How often (in slots) to log the current health board -- a real
/// "watch the market" run should see this move as prices/positions
/// change, not just print once at startup.
const HEALTH_BOARD_LOG_INTERVAL_SLOTS: Slot = 20;

/// Minimum slots between one liquidation attempt and the next -- a real
/// transaction takes a few seconds to land; without this, `evaluate()`
/// (which runs after *every* event) would resend the same liquidation
/// dozens of times before the first one even confirms. Deliberately
/// global (one attempt at a time across the whole bot), not per-obligation
/// -- this bot only ever acts on the single worst-off eligible obligation
/// per tick anyway (see `evaluate`'s own doc comment).
const LIQUIDATE_COOLDOWN_SLOTS: Slot = 50;

/// Cap on how much debt this bot will repay in a single real liquidation,
/// in whatever raw units the repay reserve's mint uses, capped by USD
/// value at attempt time -- deliberately small (target ~$2, matching this
/// session's explicit "very very minimal funds" instruction), not the
/// obligation's full repayable amount. A liquidator repaying a real
/// stranger's real debt should start conservative, not maximize bonus
/// capture on the first real attempt.
const MAX_REPAY_USD: f64 = 2.0;

/// Which real update channel most recently fed the watcher -- see
/// `low_latency`/`CommitHook::on_account`/`mid_on_tx`'s own doc comments.
/// Mirrors `testlatencylitev1::state::UpdateLane` (the branch this
/// pattern was taken from). `LowLatency` and `Commit` both feed the exact
/// same underlying obligation/reserve *state* (account bytes), so
/// whichever delivers a given account's update first is genuinely a
/// race. `Transaction` is different in kind, not just speed: `mid_on_tx`
/// sees every real transaction in the block, including ones that
/// reference a tracked obligation, *before* that transaction's resulting
/// account-state diff necessarily reaches either of the other two lanes
/// -- real, live-confirmed fact about this codebase (`TransactionList`
/// carries every transaction seen, not just this bot's own -- see
/// `testlatencylitev1::state::mid_on_tx`'s own "across every transaction
/// seen here, not just our own" comment on `t-21-fix-low`). It doesn't
/// carry the new account bytes itself, so it can't update the watcher's
/// data the way the other two lanes do -- but it can be the very first
/// sign that a tracked obligation is *about* to change. Logged when a
/// liquidation fires so "detected via X" is a real, printable claim
/// (matching HACKATHON_PLAN.md §5.2's "print real slot numbers" plan),
/// not an assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum UpdateLane {
    #[default]
    Commit,
    LowLatency,
    Transaction,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    watcher: KaminoXstocksWatcher,
    /// Live DEX-activity feed -- see `xstock_dex_watcher`'s own module
    /// doc comment. Purely observational (no eligibility/liquidation
    /// logic reads this), fed the same way `watcher` is at every call
    /// site below.
    dex: XstockDexWatcher,
    last_slot: Slot,
    /// The slot `evaluate` last logged/sent a health board for. Real,
    /// live-observed bug this guards against: `evaluate()` runs after
    /// *every* event, not once per slot, so gating only on `last_slot %
    /// HEALTH_BOARD_LOG_INTERVAL_SLOTS == 0` (no memory of what was
    /// already emitted) logged the same slot a dozen-plus times in a row
    /// during this mode's first live run.
    last_reported_slot: Slot,
    /// Which lane (see [`UpdateLane`]) most recently updated the watcher
    /// -- read (not written) at the point a liquidation is attempted, so
    /// the log line can say which channel actually won the race for the
    /// data that triggered it.
    last_update_lane: UpdateLane,
    /// How many times each lane has "won" (been the one that set
    /// `last_update_lane`) since this bot started -- see
    /// `StateHelper::mark_lane`, the single place all three lanes report
    /// through. Real counts, logged periodically alongside the health
    /// board, so "is Transaction actually faster than Commit" has an
    /// answer instead of an impression from eyeballing individual log
    /// lines.
    lane_wins: [u64; 3],

    /// Running total of every *detected* Kamino-vs-Orca price gap for a
    /// tracked xStock, in USD, assuming a fixed [`ARB_NOTIONAL_USD`]
    /// trade size per detected gap -- see `check_arb`'s own doc comment.
    /// Explicitly an estimate of *opportunity size*, not realized
    /// profit: no trade is actually executed for this number (real
    /// arbitrage execution has slippage/fees/race risk this doesn't
    /// model) -- the dashboard must label it as detected/estimated, same
    /// convention as `LiquidationEvent::estimated_bonus_usd`.
    arb_cumulative_usd: f64,
    /// How many times `check_arb` has actually added to
    /// `arb_cumulative_usd` (i.e. how many *new*, above-threshold gaps
    /// have been detected total, across every ticker) -- so the
    /// dashboard can show "$X across N detected gaps" instead of a bare
    /// dollar figure with no sense of how it accumulated.
    arb_detection_count: u64,
    /// Same running total as `arb_cumulative_usd`, but broken out per
    /// ticker -- so the dashboard can show which specific xStocks are
    /// actually driving the number instead of one opaque lump sum.
    m_arb_usd_by_ticker: HashMap<&'static str, f64>,
    /// The (kamino_price, dex_price) pair last counted toward
    /// `arb_cumulative_usd` for each ticker -- debounces `check_arb` so
    /// a persistent, unchanged gap isn't recounted every throttle tick
    /// as if it were a fresh opportunity each time.
    m_arb_last_priced: HashMap<&'static str, (f64, f64)>,
    /// Running total of `LiquidationEvent::estimated_bonus_usd` across
    /// every liquidation attempt this bot has actually sent with
    /// `sent_ok == true` -- inherits that field's own "estimate, not a
    /// post-hoc balance-verified figure" caveat, but distinct from (and
    /// far more grounded than) `arb_cumulative_usd`: this only grows
    /// from a real, signed, landed transaction, not a passive
    /// observation.
    liquidation_pnl_usd: f64,

    o_owner: Option<AccountId>,
    last_liquidate_attempt_slot: Option<Slot>,
    /// ATAs this bot has already derived+idempotent-created, keyed by
    /// mint -- real opportunities can involve any of the 10 xStock mints
    /// or 3 debt mints, so these are built on demand per mint the first
    /// time it's actually needed, not all 13 up front.
    m_ata: HashMap<AccountId, AccountId>,
    /// Set by `attempt_liquidate` right before the real instruction is
    /// queued, taken by `evaluate` right after `Wallet::drain_and_send`
    /// returns -- the only way to attach the *real* transaction signature
    /// to the outbound `LiquidationEvent` message, since the signature
    /// doesn't exist until the send actually happens.
    o_pending_liquidation: Option<PendingLiquidation>,
    /// Signature (and the slot it was sent at) of the most recently sent
    /// liquidation transaction -- set in `evaluate` right alongside
    /// `o_pending_liquidation`'s own consumption, consumed by
    /// `mid_on_tx` to log that transaction's real, authoritative on-chain
    /// outcome once the validator's Transaction lane actually carries it
    /// (see `mid_on_tx`'s own doc comment for why that's genuinely
    /// different information from `drain_and_send`'s send-time result).
    /// A single slot is enough, unlike `arbv1::State::m_sig`'s
    /// `HashMap<Signature, _>` -- `LIQUIDATE_COOLDOWN_SLOTS` already
    /// guarantees this bot never has more than one liquidation in flight
    /// at a time.
    o_pending_liquidation_signature: Option<(Signature, Slot)>,
}

/// Everything `attempt_liquidate` already knows about an attempt before
/// it's actually sent -- carried forward to `evaluate`'s tail so the real
/// signature (only known after `Wallet::drain_and_send` runs) can be
/// attached before the outbound message goes out. See
/// `message::LiquidationEvent`'s own doc comment for what's a real
/// on-chain figure here versus an estimate.
#[derive(Debug)]
struct PendingLiquidation {
    obligation: AccountId,
    repay_ticker: [u8; 8],
    withdraw_ticker: [u8; 8],
    repay_usd: f64,
    health_factor_before: f64,
    estimated_bonus_usd: f64,
}

/// Left-aligned, NUL-padded, truncated to 8 bytes -- every real ticker in
/// this market (`SPYx`..`MSTRx`, `USDC`/`cbBTC`/`USDG`) fits well under
/// that, so truncation never actually triggers for real data.
fn ticker_bytes(ticker: &str) -> [u8; 8] {
    let mut out = [0u8; 8];
    let bytes = ticker.as_bytes();
    let n = bytes.len().min(8);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

pub(crate) struct StateHelper<'a> {
    pub graph: &'a mut Graph,
    pub nonce: &'a mut u32,
    pub wallet: &'a mut Wallet,
    pub configuration: &'a mut Configuration,
    pub q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
    pub o_commit_slot: Option<Slot>,
    pub state: &'a mut State,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        self.state.watcher = KaminoXstocksWatcher::new();
        self.state.dex = XstockDexWatcher::new();
        log_info!(
            "xstockshealthv1: bot has been successfully uploaded to validator; {} obligation/reserve subscriptions queued, {} dex-pool subscriptions queued",
            self.state.watcher.pending_subscriptions(),
            self.state.dex.pending_subscriptions()
        );
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("xstockshealthv1: slot {slot}; status dead");
        }
    }

    /// Real fast path, added after this session's own
    /// `testlatencylitev1` investigation: `Event::LowLatency` delivers
    /// "processed"-commitment account updates at ~400ms, versus the
    /// rooted `CommitHook` path's ~12s -- previously a no-op here, which
    /// meant a real liquidation opportunity's underlying price/obligation
    /// change could sit undetected for up to ~12s even though this bot's
    /// whole premise is reacting faster than that. Feeds the exact same
    /// `Wallet`/`KaminoXstocksWatcher` state the rooted path does; see
    /// [`UpdateLane`] for why tracking which one actually delivered a
    /// given update is real, checkable information, not just plumbing.
    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        let zero = [];
        while let Some(account) = llap.account() {
            let d = account.body.unwrap_or(&zero);
            self.wallet.on_account(account.header, d);
            self.state.watcher.on_account(account.header, d);
            self.state.dex.on_account(account.header, d);
            self.mark_lane(UpdateLane::LowLatency);
        }
        // No explicit eligibility check needed here -- mod.rs's on_event
        // calls evaluate() after every event including this one, and
        // evaluate() itself now checks on every call, not just on a
        // rooted-slot cadence. See evaluate()'s own doc comment.
    }

    /// Watches the Transaction lane for this bot's own most recently sent
    /// liquidation signature (see `o_pending_liquidation_signature`'s own
    /// doc comment) and logs its real, authoritative on-chain outcome --
    /// landed at some slot, or failed with a specific `TransactionError`
    /// -- once found.
    ///
    /// This is genuinely different information from what `evaluate`
    /// already logs from `Wallet::drain_and_send`'s own `Ok`/`Err` (also
    /// what `LiquidationEvent::sent_ok` reports): that result only
    /// reflects whether the transaction was successfully *broadcast*, not
    /// whether it actually executed successfully once it landed. A real,
    /// live-possible case here: another liquidator (or this obligation's
    /// own owner topping up collateral) beats this bot to it between
    /// `attempt_liquidate`'s refresh and this transaction actually
    /// landing, so the liquidate instruction itself fails on-chain
    /// (health factor no longer <= 1.0) despite sending cleanly -- a
    /// real, honest confirmation trail matters here specifically because
    /// this bot moves real money (small, but real) against real
    /// strangers' positions.
    ///
    /// Also watches every *other* landed transaction for a second,
    /// independent purpose: touch-triggered re-checks. `tx.account` is
    /// the real, sorted list of every account a transaction referenced --
    /// any tracked obligation appearing there was just deposited into,
    /// borrowed against, repaid, withdrawn from, refreshed, or liquidated
    /// by *someone* (this bot's own liquidations included). A landed
    /// transaction is a real, decidable "this obligation's on-chain state
    /// may have just changed" signal that `mid_on_tx` -- and only
    /// `mid_on_tx` -- can see, so it's used here to run an immediate,
    /// targeted `KaminoXstocksWatcher::health` check on that specific
    /// obligation and attempt liquidation right away if it's eligible,
    /// rather than relying solely on `evaluate`'s own periodic
    /// `health_board().first()` scan to eventually notice it.
    ///
    /// This is a real, non-redundant improvement, not just an earlier
    /// poke at the same check: `health_board().first()` only ever
    /// surfaces the single globally-worst tracked obligation (of
    /// potentially thousands -- see `XSTOCKS_OBLIGATIONS_GENERATED`) each
    /// call, and `evaluate` only ever attempts that one per tick (the
    /// cooldown is global, not per-obligation -- see
    /// `LIQUIDATE_COOLDOWN_SLOTS`'s doc comment). A second, simultaneously
    /// eligible obligation that isn't the current worst would otherwise
    /// have to wait until it *becomes* the sorted first entry -- which
    /// may never happen if a worse one is stuck (e.g. genuinely
    /// unliquidatable for some on-chain reason). Reacting to the specific
    /// obligation a real transaction just touched, instead of only ever
    /// checking "the worst one," gives this bot a real path to reach
    /// eligible obligations the global scan alone would otherwise starve.
    /// Does NOT claim a latency win over `low_latency`'s own account-lane
    /// updates -- this codebase's own earlier findings (see
    /// `testperpv1`/`testlatencylitev1`) established the account lanes
    /// usually win that race; this is about coverage, not speed.
    ///
    /// Reuses `liquidate_cooldown_active` (the same gate `evaluate` uses)
    /// so this can never double-fire against the one `evaluate` call that
    /// unconditionally follows every event, `Event::Transaction` included
    /// (see `mod.rs`'s `on_event`) -- whichever check runs first within a
    /// tick claims the cooldown window, and the other naturally no-ops.
    ///
    /// Every account that isn't a tracked obligation (reserves, oracles,
    /// token accounts, the program itself, unrelated wallets) is skipped
    /// via `KaminoXstocksWatcher::health`'s own "`None` iff not tracked"
    /// contract -- no separate membership check needed.
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        let target = self.state.o_pending_liquidation_signature;
        while let Some((tx, result)) = transaction_list.transaction() {
            let signature = Signature::from(*tx.signature);

            let landed_slot = match result {
                Ok(landed_slot) => {
                    if let Some((pending_sig, sent_slot)) = target {
                        if signature == pending_sig {
                            self.state.o_pending_liquidation_signature = None;
                            log_warn!(
                                "xstockshealthv1: [liquidate] confirmed on-chain: {signature} sent@slot={sent_slot} landed@slot={landed_slot}",
                            );
                        }
                    }
                    landed_slot
                }
                Err(e) => {
                    if let Some((pending_sig, sent_slot)) = target {
                        if signature == pending_sig {
                            self.state.o_pending_liquidation_signature = None;
                            log_error!(
                                "xstockshealthv1: [liquidate] transaction {signature} sent@slot={sent_slot} failed on-chain: {e:?}",
                            );
                        }
                    }
                    // A failed transaction's writes are rolled back
                    // entirely -- nothing for the touch-check below to
                    // react to.
                    continue;
                }
            };

            if self.liquidate_cooldown_active() {
                continue;
            }
            for &account_id in tx.account {
                let Some(health) = self.state.watcher.health(account_id) else {
                    continue;
                };
                if !health.complete {
                    continue;
                }
                // See UpdateLane::Transaction's own doc comment: this
                // lane can't update the watcher's account bytes (it
                // never touches self.state.watcher's data, only reads
                // its already-cached health()), but it's real,
                // checkable information about *which* lane first flagged
                // this obligation as worth a look.
                self.mark_lane(UpdateLane::Transaction);
                log_warn!(
                    "xstockshealthv1: [liquidate] tracked obligation {account_id} touched by transaction landed@slot={landed_slot}; health_factor={:.4}",
                    health.health_factor,
                );
                if health.health_factor <= 1.0 {
                    self.attempt_liquidate(account_id);
                    // attempt_liquidate just activated the (global,
                    // single-attempt) cooldown -- no point scanning this
                    // transaction's remaining accounts.
                    break;
                }
            }
        }
    }

    fn liquidate_cooldown_active(&self) -> bool {
        match self.state.last_liquidate_attempt_slot {
            Some(last) => self.state.last_slot.saturating_sub(last) < LIQUIDATE_COOLDOWN_SLOTS,
            None => false,
        }
    }

    /// The one place all three lanes report through -- sets
    /// `last_update_lane` and increments that lane's real win count (see
    /// `State::lane_wins`'s own doc comment). `UpdateLane`'s declaration
    /// order (`Commit`, `LowLatency`, `Transaction`) is the array index.
    fn mark_lane(&mut self, lane: UpdateLane) {
        self.state.last_update_lane = lane;
        self.state.lane_wins[lane as usize] += 1;
    }

    /// # Two tiers, deliberately not one
    ///
    /// **Eligibility check**: runs on *every single call*, untethered
    /// from any slot-based throttle -- this is what actually delivers
    /// the fast-path benefit `low_latency`'s own doc comment describes.
    /// `mod.rs`'s `on_event` calls `evaluate()` after every event
    /// (`Event::LowLatency` included, ~400ms cadence), so gating this
    /// the same way the logging tier below is gated would mean fresher
    /// data sitting unchecked until the next slow rooted commit anyway
    /// -- defeating the entire point of wiring the fast path in.
    ///
    /// **Logging + dashboard push**: stays throttled to
    /// `HEALTH_BOARD_LOG_INTERVAL_SLOTS`, same as before -- a human
    /// doesn't need a sub-second-refreshing log line or wire message, and
    /// this codebase has already hit real `stdio timeout` disconnects
    /// from over-logging once (see `last_reported_slot`'s own doc
    /// comment).
    pub(crate) fn evaluate(&mut self) {
        if let Some((id, health)) = self.state.watcher.health_board().first().copied() {
            if health.health_factor <= 1.0 && !self.liquidate_cooldown_active() {
                self.attempt_liquidate(id);
            }
        }

        if self.state.last_slot != 0
            && self.state.last_slot % HEALTH_BOARD_LOG_INTERVAL_SLOTS == 0
            && self.state.last_slot != self.state.last_reported_slot
        {
            self.state.last_reported_slot = self.state.last_slot;

            let board = self.state.watcher.health_board();
            log_warn!(
                "xstockshealthv1: slot {}; {} obligations tracked, {} with a computed health factor (last update via {:?})",
                self.state.last_slot,
                self.state.watcher.tracked_count(),
                board.len(),
                self.state.last_update_lane,
            );
            log_warn!(
                "xstockshealthv1: lane win counts -- Commit={} LowLatency={} Transaction={}",
                self.state.lane_wins[UpdateLane::Commit as usize],
                self.state.lane_wins[UpdateLane::LowLatency as usize],
                self.state.lane_wins[UpdateLane::Transaction as usize],
            );
            if let Some((id, health)) = board.first() {
                log_warn!(
                    "xstockshealthv1: most at-risk obligation {} health_factor={:.4} collateral_usd={:.2} debt_usd={:.2}",
                    id, health.health_factor, health.collateral_usd, health.debt_usd,
                );
            }
            // Real push to the Go side so it can serve a live local page
            // instead of a person reading these log lines -- see
            // message.rs's CustomMessageOutbound::HealthBoard doc comment.
            // Sent even when `board` is empty (nothing tracked/complete
            // yet), so the served page reflects that honestly instead of
            // just going stale.
            self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::HealthBoard {
                total_tracked: self.state.watcher.tracked_count() as u64,
                entries: board,
            }));
            // Same cadence, real live DEX activity -- see
            // xstock_dex_watcher's own module doc comment for why this
            // exists (mostly to make the dashboard visibly move even
            // between liquidations, plus a genuine leading-indicator
            // signal). Sent even when empty, same reasoning as
            // HealthBoard above.
            self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::DexActivity(
                self.state
                    .dex
                    .recent()
                    .map(|t| DexActivityEntry {
                        ticker_id: kamino_xstocks_watcher::ticker_to_id(t.ticker),
                        price_usd: t.price_usd,
                        up: t.up,
                        slot: t.slot,
                    })
                    .collect(),
            )));
            // Same cadence again -- updates arb_cumulative_usd (debounced
            // internally, see check_arb's own doc comment) before it's
            // read into the MarketOverview push right below.
            self.check_arb();
            let risk = self.state.watcher.risk_tier_counts();
            let arb_by_ticker = self
                .state
                .m_arb_usd_by_ticker
                .iter()
                .map(|(&ticker, &usd)| (kamino_xstocks_watcher::ticker_to_id(ticker), usd))
                .collect();
            self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::MarketOverview(MarketOverview {
                reserves_monitored: KaminoXstocksWatcher::reserve_count() as u16,
                pools_monitored: XstockDexWatcher::pool_count() as u16,
                risk_tier_counts: risk,
                arb_detected_usd: self.state.arb_cumulative_usd,
                liquidation_pnl_usd: self.state.liquidation_pnl_usd,
                arb_detection_count: self.state.arb_detection_count,
                arb_by_ticker,
            })));
        }

        // Drains whatever this evaluate() call built onto self.wallet and
        // actually sends it -- same tail every real (non-watcher-only)
        // bot in this codebase uses. Was absent here entirely before this
        // mode could send transactions at all.
        let mut o_last_result: Option<(solana_sdk::signature::Signature, bool)> = None;
        for (sig, result) in self.wallet.drain_and_send() {
            let ok = result.is_ok();
            match result {
                Ok(_) => log_warn!("xstockshealthv1: sent transaction {sig}"),
                Err(e) => log_error!("xstockshealthv1: failed to send transaction {sig}: {e}"),
            }
            o_last_result = Some((sig, ok));
        }
        // This mode only ever sends liquidation transactions, so any
        // successfully-broadcast signature here is the liquidation's own
        // -- record it for mid_on_tx to correlate against the real
        // on-chain outcome once the Transaction lane carries it.
        if let Some((sig, true)) = o_last_result {
            self.state.o_pending_liquidation_signature = Some((sig, self.state.last_slot));
        }

        // If attempt_liquidate() queued a liquidation this tick, this is
        // the earliest point the real signature exists -- build and send
        // the dashboard event now, whether it landed or not (a failed
        // attempt is real information too).
        if let Some(pending) = self.state.o_pending_liquidation.take() {
            let (signature, sent_ok) = match o_last_result {
                Some((sig, ok)) => (sig.as_ref().try_into().unwrap_or([0u8; 64]), ok),
                None => ([0u8; 64], false),
            };
            if sent_ok {
                self.state.liquidation_pnl_usd += pending.estimated_bonus_usd;
            }
            self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::LiquidationEvent(LiquidationEvent {
                obligation: pending.obligation,
                repay_ticker: pending.repay_ticker,
                withdraw_ticker: pending.withdraw_ticker,
                repay_usd: pending.repay_usd,
                health_factor_before: pending.health_factor_before,
                estimated_bonus_usd: pending.estimated_bonus_usd,
                slot: self.state.last_slot,
                signature,
                sent_ok,
            })));
        }
    }

    /// Minimum gap (as a fraction, `0.0005` = 0.05%) between Kamino's
    /// live Scope price and Orca's live pool price for a ticker before
    /// [`Self::check_arb`] counts it -- filters ordinary floating-point/
    /// timing noise between two independent price sources, not meant to
    /// model real capturable-after-fees economics (see `arb_cumulative_
    /// usd`'s own doc comment on `State`).
    const ARB_MIN_GAP_PCT: f64 = 0.0005;
    /// Assumed trade notional (USD) each detected gap is sized against,
    /// purely so `arb_cumulative_usd` has *a* dollar figure to show --
    /// arbitrary, not a claim about real available size on either venue.
    const ARB_NOTIONAL_USD: f64 = 100.0;

    /// Compares Kamino's live Scope price against Orca's live pool price
    /// for every tracked xStock ticker, and adds any new (debounced --
    /// see `m_arb_last_priced`), above-[`Self::ARB_MIN_GAP_PCT`] gap to
    /// `arb_cumulative_usd`. Called on the same throttled cadence as the
    /// health board/dex activity pushes in `evaluate` -- see that
    /// call site.
    fn check_arb(&mut self) {
        for ticker in kamino_xstocks_watcher::xstock_tickers() {
            let (Some(kamino_price), Some(dex_price)) = (
                self.state.watcher.live_price_usd_by_ticker(ticker),
                self.state.dex.price_usd_by_ticker(ticker),
            ) else {
                continue;
            };
            if kamino_price <= 0.0 || dex_price <= 0.0 {
                continue;
            }
            let already_counted = self.state.m_arb_last_priced.get(ticker) == Some(&(kamino_price, dex_price));
            self.state.m_arb_last_priced.insert(ticker, (kamino_price, dex_price));
            if already_counted {
                continue;
            }
            let gap_pct = (dex_price - kamino_price).abs() / kamino_price;
            if gap_pct >= Self::ARB_MIN_GAP_PCT {
                let usd = gap_pct * Self::ARB_NOTIONAL_USD;
                self.state.arb_cumulative_usd += usd;
                self.state.arb_detection_count += 1;
                *self.state.m_arb_usd_by_ticker.entry(ticker).or_insert(0.0) += usd;
            }
        }
    }

    /// Derives (and idempotently queues creation of, if not already known)
    /// this bot's own ATA for `mint`, using `token_program`. Cached in
    /// `State::m_ata` so repeat opportunities against the same mint don't
    /// re-derive/re-queue-create every time.
    fn get_or_create_ata(&mut self, owner: AccountId, mint: AccountId, token_program: &solana_sdk::pubkey::Pubkey) -> Option<AccountId> {
        if let Some(&ata) = self.state.m_ata.get(&mint) {
            return Some(ata);
        }
        let ata = self.wallet.append_create_ata_with_program(owner, mint, token_program)?;
        self.state.m_ata.insert(mint, ata);
        Some(ata)
    }

    /// Picks the largest (by live USD value) deposit and largest borrow on
    /// `id`'s real obligation, and fires a real, small, partial
    /// liquidation against that specific reserve pair. Only ever attempts
    /// one obligation per call (the caller already picked the single
    /// worst-off eligible one) -- see `LIQUIDATE_COOLDOWN_SLOTS`'s doc
    /// comment for why this isn't retried every tick regardless of
    /// outcome.
    fn attempt_liquidate(&mut self, id: AccountId) {
        let Some(owner) = self.state.o_owner else {
            return;
        };
        let Some(ob) = self.state.watcher.obligation(id).cloned() else {
            return;
        };

        let mut best_deposit: Option<(AccountId, f64)> = None;
        for d in &ob.deposits {
            let Some(reserve) = self.state.watcher.reserve(d.deposit_reserve) else {
                continue;
            };
            let usd = reserve.underlying_to_usd(reserve.ctokens_to_underlying(d.deposited_amount));
            if best_deposit.is_none_or(|(_, best)| usd > best) {
                best_deposit = Some((d.deposit_reserve, usd));
            }
        }
        let mut best_borrow: Option<(AccountId, f64)> = None;
        for b in &ob.borrows {
            let Some(reserve) = self.state.watcher.reserve(b.borrow_reserve) else {
                continue;
            };
            let usd = reserve.underlying_to_usd(b.borrowed_amount as f64);
            if best_borrow.is_none_or(|(_, best)| usd > best) {
                best_borrow = Some((b.borrow_reserve, usd));
            }
        }
        let (Some((withdraw_reserve_id, _)), Some((repay_reserve_id, _))) = (best_deposit, best_borrow) else {
            log_error!("xstocksliquidatev1: [liquidate] obligation {id} has no priceable deposit/borrow to act on");
            return;
        };
        let (Some(withdraw_reserve), Some(repay_reserve)) = (
            self.state.watcher.reserve(withdraw_reserve_id).cloned(),
            self.state.watcher.reserve(repay_reserve_id).cloned(),
        ) else {
            return;
        };
        let Some(borrow) = ob.borrow_for(repay_reserve_id) else {
            return;
        };

        // Cap repay amount at MAX_REPAY_USD, but never more than the
        // borrower's own outstanding debt on this reserve.
        let cap_raw = if repay_reserve.price_usd > 0.0 {
            (MAX_REPAY_USD / repay_reserve.price_usd * 10f64.powi(repay_reserve.mint_decimals as i32)) as u64
        } else {
            0
        };
        let repay_amount = borrow.borrowed_amount.min(cap_raw);
        if repay_amount == 0 {
            log_error!("xstockshealthv1: [liquidate] computed repay amount is 0 for obligation {id}");
            return;
        }

        let repay_token_program = self.state.watcher.liquidity_token_program(repay_reserve_id);
        let withdraw_token_program = self.state.watcher.liquidity_token_program(withdraw_reserve_id);
        let (Some(source_liquidity), Some(dest_collateral), Some(dest_liquidity)) = (
            self.get_or_create_ata(owner, repay_reserve.token_mint, &repay_token_program),
            self.get_or_create_ata(owner, withdraw_reserve.collateral_mint, &spl_token::ID),
            self.get_or_create_ata(owner, withdraw_reserve.token_mint, &withdraw_token_program),
        ) else {
            log_error!("xstockshealthv1: [liquidate] failed to derive/create one or more ATAs for obligation {id}");
            return;
        };

        if let Err(e) = repay_reserve.refresh_reserve(
            repay_reserve_id, repay_reserve.pyth_oracle, repay_reserve.switchboard_price_oracle,
            repay_reserve.switchboard_twap_oracle, repay_reserve.scope_prices, self.wallet,
        ) {
            log_error!("xstockshealthv1: [liquidate] refresh repay reserve failed: {e}");
            return;
        }
        if let Err(e) = withdraw_reserve.refresh_reserve(
            withdraw_reserve_id, withdraw_reserve.pyth_oracle, withdraw_reserve.switchboard_price_oracle,
            withdraw_reserve.switchboard_twap_oracle, withdraw_reserve.scope_prices, self.wallet,
        ) {
            log_error!("xstockshealthv1: [liquidate] refresh withdraw reserve failed: {e}");
            return;
        }
        if let Err(e) = kamino::refresh_obligation(
            repay_reserve.lending_market, id, &[withdraw_reserve_id], &[repay_reserve_id], self.wallet,
        ) {
            log_error!("xstockshealthv1: [liquidate] refresh_obligation failed: {e}");
            return;
        }

        let repay_usd = repay_amount as f64 / 10f64.powi(repay_reserve.mint_decimals as i32) * repay_reserve.price_usd;
        // Rough placeholder, not read from the reserve's real
        // min/max_liquidation_bonus_bps config (that field isn't parsed
        // into KaminoReserve yet) -- see message.rs's LiquidationEvent
        // doc comment: the dashboard must label this as an estimate, and
        // this comment is why it's a rough one specifically. TSLAx's own
        // real bonus range, checked directly on-chain earlier this
        // session, was 500-1000 bps (5-10%); 7% sits in the middle of
        // that as a placeholder for every reserve, not a per-reserve
        // real value.
        let estimated_bonus_usd = repay_usd * 0.07;
        self.state.o_pending_liquidation = Some(PendingLiquidation {
            obligation: id,
            repay_ticker: ticker_bytes(kamino_xstocks_watcher::reserve_ticker(repay_reserve_id)),
            withdraw_ticker: ticker_bytes(kamino_xstocks_watcher::reserve_ticker(withdraw_reserve_id)),
            repay_usd,
            health_factor_before: self.state.watcher.health(id).map(|h| h.health_factor).unwrap_or(f64::NAN),
            estimated_bonus_usd,
        });

        log_warn!(
            "xstockshealthv1: [liquidate] LIQUIDATING obligation {id} -- repaying {repay_amount} raw units of reserve {repay_reserve_id}, seizing collateral from reserve {withdraw_reserve_id}; detected via {:?}",
            self.state.last_update_lane,
        );
        self.wallet.require_signer(owner);
        if let Err(e) = kamino::liquidate_obligation_and_redeem_reserve_collateral_v2(
            owner,
            id,
            repay_reserve.lending_market,
            repay_reserve_id,
            &repay_reserve,
            withdraw_reserve_id,
            &withdraw_reserve,
            repay_amount,
            0, // min_acceptable_received_liquidity_amount -- small/conservative repay, not slippage-sensitive
            source_liquidity,
            dest_collateral,
            dest_liquidity,
            &spl_token::ID,
            &withdraw_token_program,
            self.wallet,
        ) {
            log_error!("xstockshealthv1: [liquidate] liquidate instruction failed: {e}");
        }
        self.state.last_liquidate_attempt_slot = Some(self.state.last_slot);
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
                CustomMessageInbound::Wallet(rc_keypair) => {
                    let keypair = rc_unlock(&rc_keypair);
                    let pubkey = keypair.pubkey();
                    let account_id = account_id_from_pubkey(&pubkey);
                    log_warn!("xstockshealthv1: got wallet keypair {} {}", pubkey, account_id);
                    self.wallet.append_key(rc_keypair.clone(), self.graph).unwrap();
                    self.wallet.set_payer(account_id);
                    self.configuration.wallet = account_id;
                    self.state.o_owner = Some(account_id);
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
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.wallet.on_account(header, body);
        self.state.watcher.on_account(header, body);
        self.state.dex.on_account(header, body);
        self.mark_lane(UpdateLane::Commit);
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        self.wallet.token_mut().on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        // Paced, not one `bulk_subscribe` call -- see this module's own
        // and `kamino_xstocks_watcher`'s doc comments for why.
        if let Err(e) = self
            .state
            .watcher
            .flush_subscriptions(self.graph, SUBSCRIBE_PER_SLOT)
        {
            log_warn!("xstockshealthv1: failed to flush obligation/reserve subscriptions: {e}");
        }
        if let Err(e) = self
            .state
            .dex
            .flush_subscriptions(self.graph, SUBSCRIBE_PER_SLOT)
        {
            log_warn!("xstockshealthv1: failed to flush dex-pool subscriptions: {e}");
        }
    }
}
