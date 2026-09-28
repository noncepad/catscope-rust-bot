//! leveragedloopv1 -- Phase 2 of `leveraged_yield_farming_plan.md`:
//! a single, conservative jitoSOL-collateral/USDC-debt leverage loop on
//! Kamino (deposit jitoSOL, borrow USDC against it, swap the borrow back
//! to jitoSOL, redeposit once), manual/explicit trigger only, with a
//! deleverage/unwind path built and tested before the open path is ever
//! run for real. **Not** testperpv1 with jitoSOL bolted on -- a
//! deliberately separate bot mode/obligation, since testperpv1 already
//! runs its own repeating Solend/Kamino $1 deposit-withdraw smoke-test
//! cycle and layering a real leveraged position onto that same
//! obligation risked the two interfering with each other. See the plan
//! file's "Phase 2 concrete design" section for the full mechanics this
//! module implements.
//!
//! Same `EventHandler`/`StateHelper`/module shape as every other bot mode
//! (`testperpv1`/`perpfundingv1`) -- originally trimmed to just wallet
//! handling, Kamino, and spot-swap execution, but as of 2026-08-29 this
//! mode also runs a **second, fully independent** real strategy: a
//! Phoenix-perp-funding-vs-Kamino-rate basis trade (see
//! `state::StateHelper::run_basis_cycle`'s doc comment), wired through
//! the standalone `trader::derivative_router` search module. Enabled only
//! after an explicit `TriggerEnableBasisTrading`; runs autonomously every
//! real funding epoch after that (delta-neutral by construction, unlike
//! the leverage loop above, so it doesn't need a human pulling the
//! trigger every cycle -- same real, proven `perpfundingv1` behavior this
//! was ported from). Deliberately isolated from the leverage loop's own
//! `id=0` Kamino obligation via a second, independent `id=1` obligation
//! (see `kamino::obligation_pda`'s doc comment) -- same "don't share a
//! real obligation between two independent strategies" reasoning as the
//! testperpv1 separation above, just within this one bot mode instead of
//! across two.
//!
//! # Event flow
//! ```text
//! validator → Event::LowLatency  → state.low_latency()   (processed accounts, ~400 ms)
//! validator → Event::Commit      → state.mid_on_account() (rooted accounts, ~12 s)
//! validator → Event::Transaction → state.mid_on_tx()      (drained, unused)
//! validator → Event::SlotStatus  → state.on_slot_status()
//! Go brain  → stdin              → state.on_message()  (wallet key, TriggerOpen/TriggerClose,
//!                                   TriggerEnableBasisTrading/TriggerCloseAllBasisPositions)
//! state.evaluate() runs after every event -- cooldown-gated LoopPhase
//! advancement (see state::LoopPhase's doc comment), entirely inert until
//! a real TriggerOpen/TriggerClose arrives; independently, the basis-trade
//! cycle (real funding-epoch-gated, see run_basis_cycle) is equally inert
//! until TriggerEnableBasisTrading arrives.
//! ```
use crate::{
    brain::leveragedloopv1::{
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

pub struct LeveragedLoopV1Hook {
    nonce: Rc<UnsafeCell<u32>>,
    rc_parser: Rc<UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>>,
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    rc_wallet: Rc<UnsafeCell<Wallet>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    o_poller: Option<crate::event_loop::EventPoller>,
}

impl LeveragedLoopV1Hook {
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

impl EventHandler for LeveragedLoopV1Hook {
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
        log_info!("leveragedloopv1: on_load complete");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("leveragedloopv1: on_unload");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut msg_in = {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.take().unwrap()
        };
        let mut helper = self.helper();
        log_debug!("leveragedloopv1: event - +++++");
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
            log_warn!("leveragedloopv1: writing {n_written} outbound message(s), flushing stdout");
        }
        outbound.flush();
        if n_written > 0 {
            log_warn!("leveragedloopv1: outbound flush returned");
        }
        parser.outbound.replace(outbound);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
