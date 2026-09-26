//! marketwatchv1 -- WASM-side bot brain for a real-time market-statistics
//! dashboard. Its counterpart on the Go side is
//! `optimizer/brain/marketwatchv1`, which feeds a terminal dashboard
//! (`optimizer marketwatch`) instead of an actual trading loop.
//!
//! This is a read-only observer, forked from `arbv1`'s chassis (same
//! `EventHandler`/`StateHelper`/`CommitHook` wiring, same `Graph::new` in
//! `on_load`, same live-pool price feed via `DexState`/`TradeRouter`) with
//! every trading-specific piece removed: no `wallet::Wallet` (this bot
//! never signs or sends a transaction, so there's no balance/ATA/
//! transaction-building state to carry), no Bellman-Ford cycle search, no
//! execution planning.
//!
//! # What it does
//! - Tracks DEX pool state the same way `arbv1` does, by processing live
//!   account and token-balance updates from the validator into the same
//!   `DexState`/`TradeRouter` live price graph.
//! - Every `MARKET_STATS_INTERVAL_SLOTS` slots, quotes a small, fixed set
//!   of well-known symbols (`trader::market_stats::TRACKED_MINTS`)
//!   against USDC via `TradeRouter::route`, feeds each quote into an EWMA
//!   volatility/correlation tracker (`trader::market_stats`), and pushes
//!   the resulting snapshot to the Go brain as a `MarketStats` message.
//!
//! # Event flow
//! ```text
//! validator → Event::LowLatency  → state.low_latency()   (processed accounts, ~400 ms)
//! validator → Event::Commit      → state.mid_on_account() (rooted accounts, ~12 s)
//! validator → Event::Transaction → state.mid_on_tx()      (drained, not otherwise used --
//!                                    see mid_on_tx's own doc comment)
//! validator → Event::SlotStatus  → state.on_slot_status()
//! state.evaluate() runs after every event; the real stats recompute+push is
//! throttled to MARKET_STATS_INTERVAL_SLOTS, same convention xstockshealthv1's
//! health-board push uses.
//! ```
use crate::{
    brain::marketwatchv1::{
        configuration::Configuration,
        message::{CustomMessageInbound, CustomMessageOutbound},
        state::{State, StateHelper},
    },
    err::CatscopeGuestError,
    event::Event,
    event_loop::EventHandler,
    graph::Graph,
    log_debug, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageSend, Parser},
    util::rc_unlock_mut,
};
use std::{cell::UnsafeCell, collections::VecDeque, rc::Rc};

pub(crate) mod configuration;
pub(crate) mod message;
pub(crate) mod state;

pub struct MarketWatchV1Hook {
    nonce: Rc<UnsafeCell<u32>>,
    rc_parser: Rc<UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>>,
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    o_poller: Option<crate::event_loop::EventPoller>,
}

impl MarketWatchV1Hook {
    pub fn new(
        rc_parser: Rc<
            UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>,
        >,
    ) -> Self {
        Self {
            rc_parser,
            nonce: Rc::new(UnsafeCell::new(1)),
            tmp_q_msg: Rc::new(UnsafeCell::new(VecDeque::with_capacity(10))),
            rc_configuration: Rc::new(UnsafeCell::new(Configuration::default())),
            o_rc_graph: None,
            rc_state: Rc::new(UnsafeCell::new(State::default())),
            o_poller: None,
        }
    }
    fn helper<'a, 'b: 'a>(&'b mut self) -> StateHelper<'a> {
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let nonce = unsafe { &mut *self.nonce.get() };
        let graph = rc_unlock_mut(self.o_rc_graph.as_ref().unwrap());
        StateHelper {
            graph,
            nonce,
            configuration: rc_unlock_mut(&self.rc_configuration),
            q_msg,
            o_commit_slot: None,
            state: rc_unlock_mut(&self.rc_state),
        }
    }
}

impl EventHandler for MarketWatchV1Hook {
    fn on_load(
        &mut self,
        poller: crate::event_loop::EventPoller,
        _l_args: &[String],
    ) -> Result<(), CatscopeGuestError> {
        assert!(self.o_poller.replace(poller.clone()).is_none());
        let g = Graph::new(poller)?;
        assert!(self.o_rc_graph.replace(g).is_none());
        let mut helper = self.helper();
        helper.on_load();
        log_info!("marketwatchv1: on_load - 1");
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let parser = rc_unlock_mut(&self.rc_parser);
        let mut outbound = parser.outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        parser.outbound.replace(outbound);
        log_info!("marketwatchv1: on_load - 2 complete");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("marketwatchv1: on_unload");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut msg_in = {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.take().unwrap()
        };
        let mut helper = self.helper();
        log_debug!("marketwatchv1: event - +++++");
        match event {
            Event::Stdin(data) => {
                msg_in.on_data(&data, |action| {
                    helper.on_message(action);
                })?;
            }
            Event::Commit(commit) => {
                commit.process(&mut helper);
            }
            Event::LowLatency(llap) => {
                helper.low_latency(llap);
            }
            Event::Transaction(transaction_list) => {
                helper.mid_on_tx(transaction_list);
            }
            Event::SlotStatus(slot, status) => {
                helper.on_slot_status(slot, status);
            }
        };
        helper.evaluate();
        {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.replace(msg_in);
        }

        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let parser = rc_unlock_mut(&self.rc_parser);
        let mut outbound = parser.outbound.take().unwrap();
        let mut n_written = 0u32;
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
            n_written += 1;
        }
        if n_written > 0 {
            log_warn!("marketwatchv1: writing {n_written} outbound message(s), flushing stdout");
        }
        outbound.flush();
        parser.outbound.replace(outbound);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
