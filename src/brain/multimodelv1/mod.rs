//! `multimodelv1` -- Phase 5 of `PLAN-1.md`, sub-phase 5a: the
//! `EventHandler` skeleton and wallet/market-data subscriptions only,
//! same "idle-verified" milestone `leveragedloopv1`/`testperpv1` each
//! reached before any real trading logic existed -- see `state`'s own
//! doc comment for the full reasoning and what's deliberately not here
//! yet (Phases 0-4's `trader::factor_graph`/`factor_borrow_gate`/
//! `factor_sizing`/`factor_intent` aren't wired into `evaluate()` yet).
//!
//! Same `EventHandler`/`StateHelper`/module shape as every other bot
//! mode -- see `leveragedloopv1::mod`'s own doc comment for the general
//! pattern this copies.
//!
//! # Event flow
//! ```text
//! validator → Event::LowLatency  → state.low_latency()   (processed accounts, ~400 ms)
//! validator → Event::Commit      → state.mid_on_account() (rooted accounts, ~12 s)
//! validator → Event::Transaction → state.mid_on_tx()      (drained, currently a no-op)
//! validator → Event::SlotStatus  → state.on_slot_status()
//! Go brain  → stdin              → state.on_message()  (wallet key; ReplayOpenIntent
//!                                   received but inert until Phase 5 wires real trade state)
//! state.evaluate() runs after every event -- currently a no-op (sub-phase 5a).
//! ```
use crate::{
    brain::multimodelv1::{
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
    wallet::Wallet,
};
use std::{cell::UnsafeCell, collections::VecDeque, rc::Rc};

pub(crate) mod configuration;
pub(crate) mod message;
pub(crate) mod state;

pub struct MultiModelV1Hook {
    nonce: Rc<UnsafeCell<u32>>,
    rc_parser: Rc<UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>>,
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    rc_wallet: Rc<UnsafeCell<Wallet>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    o_poller: Option<crate::event_loop::EventPoller>,
}

impl MultiModelV1Hook {
    pub fn new(
        rc_parser: Rc<
            UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>,
        >,
    ) -> Self {
        Self {
            rc_parser,
            rc_wallet: Rc::new(UnsafeCell::new(Wallet::new())),
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
        let wallet = rc_unlock_mut(&self.rc_wallet);
        let graph = rc_unlock_mut(self.o_rc_graph.as_ref().unwrap());
        StateHelper {
            graph,
            nonce,
            wallet,
            configuration: rc_unlock_mut(&self.rc_configuration),
            q_msg,
            o_commit_slot: None,
            state: rc_unlock_mut(&self.rc_state),
        }
    }
}

impl EventHandler for MultiModelV1Hook {
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
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let parser = rc_unlock_mut(&self.rc_parser);
        let mut outbound = parser.outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        parser.outbound.replace(outbound);
        log_info!("multimodelv1: on_load complete");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("multimodelv1: on_unload");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut msg_in = {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.take().unwrap()
        };
        let mut helper = self.helper();
        log_debug!("multimodelv1: event - +++++");
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
            log_warn!("multimodelv1: writing {n_written} outbound message(s), flushing stdout");
        }
        outbound.flush();
        if n_written > 0 {
            log_warn!("multimodelv1: outbound flush returned");
        }
        parser.outbound.replace(outbound);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
