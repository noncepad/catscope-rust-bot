//! midontxhealthv1 -- a diagnostic bot for the Transaction-lane event
//! stream (`Event::Transaction` -> `mid_on_tx`). Two real questions, both
//! covered in full by `state`'s own doc comment:
//!
//! 1. Does `mid_on_tx` deliver *every* real non-vote transaction in a
//!    block, or does it silently drop some? (Checked passively -- every
//!    distinct signature seen per slot, spot-checked against
//!    `solana block <slot>`'s real non-vote signature list. Confirmed
//!    real, substantial gaps -- see `state`'s doc comment for the full
//!    history, including a first version of this check that got the
//!    methodology wrong.)
//! 2. When our *own* signed transaction lands, does the Transaction lane
//!    actually notice it, and how fast? (Checked actively -- this module
//!    sends a small number of its own real, self-signed native transfers,
//!    each straight back to the parent/mothership wallet, and tracks only
//!    the Transaction lane's own confirmation for each one -- no other
//!    lane competes for the same signature here, unlike the full
//!    native-transfer latency tests where Account/LowLatency wins
//!    essentially every race.)
//!
//! Real, live-measured motivation (2026-09-09): see `TX_INDEX_ESTIMATE_PLAN.md`'s
//! addendum for the fuller investigation this grew out of. Delete this
//! whole module once both questions are answered, same convention
//! `testperplatencyv1lite` itself follows.
//!
//! # Event flow
//! ```text
//! validator → Event::Transaction → state.mid_on_tx()      (both checks above)
//! validator → Event::SlotStatus  → state.on_slot_status() (no-op, kept for dispatch parity)
//! validator → Event::LowLatency  → state.low_latency()    (no-op, drained and ignored)
//! validator → Event::Commit      → state.mid_on_account() (no-op, drained and ignored)
//! Go brain  → stdin              → state.on_message()     (only the real `Wallet` key is used)
//! state.evaluate() runs after every event -- sends this module's own small number of
//! real transfers once the wallet arrives, gated by NATIVE_TRANSFER_TARGET/SEND_COOLDOWN.
//! ```
use crate::{
    brain::midontxhealthv1::{
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

pub struct MidOnTxHealthV1Hook {
    nonce: Rc<UnsafeCell<u32>>,
    rc_parser: Rc<UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>>,
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    rc_wallet: Rc<UnsafeCell<Wallet>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    o_poller: Option<crate::event_loop::EventPoller>,
}

impl MidOnTxHealthV1Hook {
    pub fn new(
        rc_parser: Rc<
            UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>,
        >,
    ) -> Self {
        Self {
            rc_parser,
            // Real -- this module sends a small number of its own real
            // signed transfers (see `state`'s doc comment). `Wallet::new()`
            // itself does no network I/O; the real key arrives later via
            // `on_message`.
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

impl EventHandler for MidOnTxHealthV1Hook {
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
        log_info!("midontxhealthv1: on_load complete");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("midontxhealthv1: on_unload");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut msg_in = {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.take().unwrap()
        };
        let mut helper = self.helper();
        log_debug!("midontxhealthv1: event - +++++");
        match event {
            Event::Stdin(data) => {
                log_warn!("midontxhealthv1: stdin event, {} bytes", data.len());
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
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
