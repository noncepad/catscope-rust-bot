use crate::{
    brain::helloworldv1::{
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
    util::{rc_unlock, rc_unlock_mut},
    wallet::Wallet,
    TradingSetup,
};
use std::{cell::UnsafeCell, collections::VecDeque, rc::Rc};

pub(crate) mod configuration;
pub(crate) mod message;
pub(crate) mod state;

pub struct HelloWorldV1Hook {
    llap_account_count: usize,
    llap_token_count: usize,
    nonce: Rc<UnsafeCell<u32>>,
    rc_parser: Rc<UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>>,
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    rc_wallet: Rc<UnsafeCell<Wallet>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    o_poller: Option<crate::event_loop::EventPoller>,
}

impl HelloWorldV1Hook {
    pub fn new(
        rc_parser: Rc<
            UnsafeCell<Parser<Configuration, CustomMessageInbound, CustomMessageOutbound>>,
        >,
    ) -> Self {
        Self {
            llap_account_count: 0,
            llap_token_count: 0,
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

impl EventHandler for HelloWorldV1Hook {
    fn on_load(
        &mut self,
        poller: crate::event_loop::EventPoller,
        _rc_edgemgr: std::rc::Rc<std::cell::UnsafeCell<crate::graph::EdgeManager>>,
        _l_args: &[String],
        trading: &TradingSetup,
    ) -> Result<(), CatscopeGuestError> {
        assert!(self.o_poller.replace(poller.clone()).is_none());
        let g = Graph::new(poller)?;
        assert!(self.o_rc_graph.replace(g).is_none());
        let mut helper = self.helper();
        helper.on_load(trading);
        log_info!("++++++on_load - 1");
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let parser = rc_unlock_mut(&self.rc_parser);
        let mut outbound = parser.outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        parser.outbound.replace(outbound);
        log_info!("on_load - 2+++++");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("on_unload - 1");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        //let mut parser = self.o_msg_parser.take().unwrap();
        let mut msg_in = {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.take().unwrap()
        };
        let mut llap_account_count = self.llap_account_count;
        let mut llap_token_count = self.llap_token_count;
        let mut helper = self.helper();
        log_debug!("HelloWorldV1Hook::event - 1 - +++++");
        match event {
            Event::Stdin(data) => {
                msg_in.on_data(&data, |action| {
                    helper.on_message(action);
                })?;
            }
            Event::Commit(commit) => {
                log_debug!("HelloWorldV1Hook::event - commit - 1 - llap {llap_token_count} {llap_account_count}",);
                commit.process(&mut helper);
                log_debug!("HelloWorldV1Hook::event - commit - 2");
            }
            Event::LowLatency(llap) => {
                log_debug!("HelloWorldV1Hook::event - account_wrapper");
                llap_account_count += llap.account_len();
                llap_token_count += llap.token_len();
                helper.low_latency(llap);
            }
            Event::Transaction(transaction_list) => {
                log_debug!("HelloWorldV1Hook::event - tx");
                helper.mid_on_tx(transaction_list);
            }
            Event::SlotStatus(slot, status) => {
                log_debug!("HelloWorldV1Hook::event - slot {slot} - status {status:?}");
                helper.on_slot_status(slot, status);
            }
        };
        //log_warn!("HelloWorldV1Hook::event - 2");
        helper.evaluate();
        {
            let parser = rc_unlock_mut(&self.rc_parser);
            parser.inbound.replace(msg_in);
        }
        //log_warn!("HelloWorldV1Hook::event - 3");

        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let parser = rc_unlock_mut(&self.rc_parser);
        let mut outbound = parser.outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        //log_warn!("HelloWorldV1Hook::event - 4");
        parser.outbound.replace(outbound);
        self.llap_account_count = llap_account_count;
        self.llap_token_count = llap_token_count;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
