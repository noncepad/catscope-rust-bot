use crate::{
    brain::bouncerv1::{
        configuration::Configuration,
        message::{CustomMessageInbound, CustomMessageOutbound},
        state::{State, StateHelper},
    },
    err::CatscopeGuestError,
    event::Event,
    event_loop::EventHandler,
    graph::{CommitHook, Graph},
    log_debug,
    message::{
        InboundMesasgeHandler, MessageAction, MessageDeserializer, MessageSend, MessageSerializer,
    },
    stdio::{MessageInbound, MessageOutbound, StdioPacket},
    util::{as_bytes, as_bytes_mut, rc_unlock, rc_unlock_mut},
    wallet::Wallet,
};
use solana_sdk::{pubkey::Pubkey, signer::Signer as _};
use std::{cell::UnsafeCell, collections::VecDeque, rc::Rc, time::SystemTime};

pub(crate) mod configuration;
pub(crate) mod message;
pub(crate) mod state;

pub struct BouncerV1Hook {
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_wallet: Rc<UnsafeCell<Wallet>>,
    rc_state: Rc<UnsafeCell<State>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    msg_outbound: Option<MessageOutbound>,
    o_poller: Option<crate::event_loop::EventPoller>,
    o_rc_edgemgr: Option<std::rc::Rc<std::cell::UnsafeCell<crate::graph::EdgeManager>>>,
    o_msg_parser: Option<MessageInbound>,
}

impl Default for BouncerV1Hook {
    fn default() -> Self {
        Self {
            tmp_q_msg: Rc::new(UnsafeCell::new(VecDeque::with_capacity(100))),
            rc_configuration: Rc::new(UnsafeCell::new(Configuration::default())),
            o_rc_graph: None,
            rc_state: Rc::new(UnsafeCell::new(State::default())),
            rc_wallet: Rc::new(UnsafeCell::new(Wallet::new())),
            msg_outbound: Some(MessageOutbound::default()),
            o_msg_parser: Some(MessageInbound::default()),
            o_rc_edgemgr: None,
            o_poller: None,
        }
    }
}
impl BouncerV1Hook {
    fn helper<'a, 'b: 'a>(&'b mut self) -> StateHelper<'a> {
        let g = rc_unlock_mut(self.o_rc_graph.as_ref().unwrap());
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        StateHelper {
            configuration: rc_unlock_mut(&self.rc_configuration),
            q_msg,
            o_graph: Some(g),
            o_commit_slot: None,
            state: rc_unlock_mut(&self.rc_state),
            wallet: rc_unlock_mut(&self.rc_wallet),
        }
    }
}

impl EventHandler for BouncerV1Hook {
    fn on_load(
        &mut self,
        poller: crate::event_loop::EventPoller,
        rc_edgemgr: std::rc::Rc<std::cell::UnsafeCell<crate::graph::EdgeManager>>,
    ) -> Result<(), crate::err::CatscopeGuestError> {
        assert!(self.o_poller.replace(poller.clone()).is_none());
        let g = Graph::new(poller)?;
        assert!(self.o_rc_graph.replace(g).is_none());
        assert!(self.o_rc_edgemgr.replace(rc_edgemgr).is_none());
        let mut helper = self.helper();
        helper.on_load();
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let mut outbound = self.msg_outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        self.msg_outbound.replace(outbound);
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut parser = self.o_msg_parser.take().unwrap();
        let mut helper = self.helper();
        match event {
            Event::Stdin(data) => {
                // Boiler plate: the user can customize Stdin data.
                log_debug!("Sample::event - 1");
                parser.parse(&data).unwrap();
                let action: MessageAction<Configuration, CustomMessageInbound> = {
                    let x = &parser;
                    x.try_into()?
                };
                helper.on_message(action);
            }
            Event::Commit(commit) => {
                log_debug!("Sample::event - 2");
                commit.process(&mut helper);
            }
            Event::Account(account_wrapper) => {
                // low latency update
                log_debug!("Sample::event - 3");
                helper.mid_on_account(account_wrapper);
            }
            Event::Token(tokenaccountv1s) => {
                // low latency update
                log_debug!("Sample::event - 4");
                helper.mid_on_token(tokenaccountv1s);
            }
            Event::Transaction(transaction_list) => {
                // low latency update
                log_debug!("Sample::event - 5");
                helper.mid_on_tx(transaction_list);
            }
        };
        helper.evaluate();

        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let mut outbound = self.msg_outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
            outbound.flush();
        }
        self.o_msg_parser.replace(parser);
        self.msg_outbound.replace(outbound);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
