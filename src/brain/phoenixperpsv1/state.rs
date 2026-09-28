//! Reactive decision loop for the Phoenix perpetuals strategy -- mirrors
//! `arbv1::state`'s shape (same `StateHelper`/`CommitHook`/`evaluate`
//! pattern), scoped down to margin/position lifecycle management instead
//! of arbitrage-cycle detection.
use crate::{
    brain::phoenixperpsv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate, SubscriptionQueue},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    trader::dex::phoenix::{ix, margin, PhoenixState},
    txview::TransactionList,
    util::{account_id_from_pubkey, rc_unlock},
    wallet::{PriorityLevel, Wallet},
};
use solana_sdk::{clock::Slot, signature::{Keypair, Signature}, signer::Signer};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Instant,
};

/// How many slots an in-flight signature is allowed to sit unmatched
/// before it's dropped from `m_sig` as expired -- roughly 5 commits' worth
/// (~60s), generous given Phoenix order/margin instructions aren't as
/// latency-sensitive as arbv1's arbitrage sends.
const SIGNATURE_EXPIRY_SLOTS: Slot = 300;

#[derive(Debug)]
struct KeypairExtra {
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}

/// One position flagged as needing risk action by [`StateHelper::check_margin_health`].
struct AtRiskPosition {
    asset_id: u32,
    base_lot_position: i64,
}

/// Placeholder for a future entry signal -- see
/// [`StateHelper::should_open_position`]'s doc.
#[allow(dead_code)]
pub(crate) struct OpenPositionIntent {
    pub asset_id: u32,
    pub side: ix::Side,
    pub num_base_lots: u64,
}

#[derive(Debug)]
pub(crate) struct State {
    slot_delta_since_start: Slot,
    last_slot: Slot,
    last_margin_check_slot: Slot,
    tx_count: usize,
    o_rc_keypair: Option<KeypairExtra>,
    o_phoenix: Option<PhoenixState>,
    m_sig: HashMap<Signature, (Slot, Instant)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            slot_delta_since_start: 0,
            last_slot: 0,
            last_margin_check_slot: 0,
            tx_count: 0,
            o_rc_keypair: None,
            o_phoenix: None,
            m_sig: HashMap::default(),
        }
    }
}

impl State {
    fn wallet(&self) -> Option<AccountId> {
        Some(self.o_rc_keypair.as_ref()?.account_id)
    }
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

    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        assert!(
            self.state
                .o_phoenix
                .replace(PhoenixState::new_and_subscribe(self.graph).expect("phoenix state"))
                .is_none()
        );
        log_info!("phoenixperpsv1: bot has been successfully uploaded to validator");
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("phoenixperpsv1: slot {slot}; status dead");
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        while let Some(ta) = llap.token() {
            self.wallet.token_mut().on_token(ta, false);
        }
        let zero = [];
        while let Some(account) = llap.account() {
            let d = account.body.unwrap_or(&zero);
            self.wallet.on_account(account.header, d);
            if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                phoenix.on_account(account.header, d);
            }
        }
    }

    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        // Unlike arbv1's mid_on_tx, this does NOT skip on `result.is_err()`
        // -- a failed margin top-up/reduce-risk close needs to be noticed,
        // not silently dropped in with "never landed" (see the plan's
        // rationale: this strategy manages real liquidation risk).
        while let Some((tx, result)) = transaction_list.transaction() {
            let signature = Signature::from(*tx.signature);
            match result {
                Ok(slot) => {
                    if let Some((sent_slot, sent_at)) = self.state.m_sig.remove(&signature) {
                        log_warn!(
                            "phoenixperpsv1: tx {} landed at slot {} (sent at slot {}); latency {}us",
                            signature,
                            slot,
                            sent_slot,
                            sent_at.elapsed().as_micros(),
                        );
                    }
                }
                Err(e) => {
                    if let Some((sent_slot, _)) = self.state.m_sig.remove(&signature) {
                        log_error!(
                            "phoenixperpsv1: tracked tx {} FAILED (sent at slot {}): {:?}",
                            signature,
                            sent_slot,
                            e,
                        );
                    }
                }
            }
        }
    }

    /// Placeholder entry-signal hook -- always returns `None` for now. The
    /// user explicitly asked for a placeholder here rather than a
    /// designed-on-the-spot entry signal (manual command, basis-vs-spot
    /// divergence, or anything else): filling this in is future work, not
    /// part of this pass. Everything downstream of a `Some(...)` return
    /// (register-if-needed -> deposit -> place order, via
    /// `PhoenixState::register_trader`/`deposit_funds`/`place_market_order`/
    /// `place_limit_order`) is already wired and ready to use once this
    /// returns real intents.
    fn should_open_position(&self) -> Option<OpenPositionIntent> {
        None
    }

    /// Re-check every open position's margin health and queue a reducing
    /// market order for anything past the configured liquidation-avoidance
    /// threshold. Called from `evaluate()` on a `margin_check_interval`-slot
    /// cadence (mirrors `arbv1`'s `last_slot % 100` idiom).
    fn check_margin_health(&mut self) {
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        if !phoenix.global_ready() {
            return;
        }
        let collateral = phoenix.collateral_quote_lots();
        let mut at_risk = Vec::new();
        for pos in phoenix.positions() {
            if pos.base_lot_position == 0 {
                continue;
            }
            let Some(market) = phoenix.market(pos.asset_id as u32) else { continue };
            if margin::is_at_liquidation_risk(collateral, market, pos, self.configuration.liquidation_threshold) {
                at_risk.push(AtRiskPosition { asset_id: pos.asset_id as u32, base_lot_position: pos.base_lot_position });
            }
        }
        if at_risk.is_empty() {
            return;
        }
        let Some(wallet_id) = self.state.wallet() else { return };
        for p in at_risk {
            log_warn!(
                "phoenixperpsv1: position asset_id={} base_lots={} at liquidation risk @ slot {} \
                 (threshold={}) -- queuing reduce-to-flat market order",
                p.asset_id,
                p.base_lot_position,
                self.state.last_slot,
                self.configuration.liquidation_threshold,
            );
            let side = if p.base_lot_position > 0 { ix::Side::Ask } else { ix::Side::Bid };
            let size = p.base_lot_position.unsigned_abs();
            let client_order_id = self.state.last_slot as u128;
            let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
            if let Err(e) =
                phoenix.place_market_order(wallet_id, p.asset_id, side, size, 0, client_order_id, self.wallet)
            {
                log_error!(
                    "phoenixperpsv1: failed to build liquidation-avoidance close for asset_id={}: {e}",
                    p.asset_id,
                );
            }
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
            match self.state.wallet() {
                Some(id) => self.configuration.wallet = id,
                None => return,
            }
        }
        if self.state.slot_delta_since_start < 20 {
            return;
        }

        if self.state.last_slot >= self.state.last_margin_check_slot + self.configuration.margin_check_interval {
            self.state.last_margin_check_slot = self.state.last_slot;
            self.check_margin_health();
        }

        let _ = self.should_open_position();

        for (sig, result) in self.wallet.drain_and_send() {
            match result {
                Ok(_) => {
                    self.state.tx_count += 1;
                    self.state.m_sig.insert(sig, (self.state.last_slot, Instant::now()));
                }
                Err(e) => log_error!("phoenixperpsv1: failed to send tx {e}"),
            }
        }
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(std::time::SystemTime::now()));
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
                    log_warn!("phoenixperpsv1: got wallet keypair {} {}", pubkey, account_id);
                    self.wallet.append_key(rc_keypair.clone(), self.graph).unwrap();
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
                                log_error!("phoenixperpsv1: failed to subscribe to bundler nonce account: {e}");
                            }
                        }
                    }
                    if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                        if let Err(e) = phoenix.set_authority(pubkey, self.graph) {
                            log_error!("phoenixperpsv1: failed to set phoenix authority: {e}");
                        }
                    }
                    self.state.o_rc_keypair.replace(KeypairExtra { rc_keypair, account_id });
                }
                CustomMessageInbound::CommonBundlerTipUpdate(update) => {
                    self.wallet.apply_bundler_tip_update(self.graph, update);
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
        self.state.m_sig.retain(|_, (sent_slot, _)| slot.saturating_sub(*sent_slot) < SIGNATURE_EXPIRY_SLOTS);
        if slot % 100 == 0 {
            if let Some(phoenix) = self.state.o_phoenix.as_ref() {
                log_warn!(
                    "phoenixperpsv1 stats @ slot {slot}: markets_ready={}/{} collateral_quote_lots={} \
                     open_positions={} global_ready={}",
                    phoenix.ready_count(),
                    phoenix.markets().len(),
                    phoenix.collateral_quote_lots(),
                    phoenix.positions().len(),
                    phoenix.global_ready(),
                );
            }
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.wallet.on_account(header, body);
        if let Some(phoenix) = self.state.o_phoenix.as_mut() {
            phoenix.on_account(header, body);
        }
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        self.wallet.token_mut().on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        self.state.slot_delta_since_start += 1;
        if let Some(phoenix) = self.state.o_phoenix.as_mut() {
            if let Err(e) = phoenix.flush_pending(self.graph) {
                log_error!("phoenixperpsv1: failed to flush pending subscriptions: {e}");
            }
        }
    }
}
