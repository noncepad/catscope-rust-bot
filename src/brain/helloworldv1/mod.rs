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
    log_debug, log_info,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    stdio::{MessageInbound, MessageOutbound},
    util::rc_unlock_mut,
};
use std::{cell::UnsafeCell, collections::VecDeque, rc::Rc};

pub(crate) mod configuration;
pub(crate) mod message;
pub(crate) mod state;

pub struct HelloWorldV1Hook {
    rc_configuration: Rc<UnsafeCell<Configuration>>,
    rc_state: Rc<UnsafeCell<State>>,
    tmp_q_msg: Rc<UnsafeCell<VecDeque<MessageSend<CustomMessageOutbound>>>>,
    o_rc_graph: Option<Rc<UnsafeCell<Graph>>>,
    msg_outbound: Option<MessageOutbound>,
    o_poller: Option<crate::event_loop::EventPoller>,
    o_msg_parser: Option<MessageInbound>,
}

impl Default for HelloWorldV1Hook {
    fn default() -> Self {
        Self {
            tmp_q_msg: Rc::new(UnsafeCell::new(VecDeque::with_capacity(10))),
            rc_configuration: Rc::new(UnsafeCell::new(Configuration::default())),
            o_rc_graph: None,
            rc_state: Rc::new(UnsafeCell::new(State::default())),
            msg_outbound: Some(MessageOutbound::default()),
            o_msg_parser: Some(MessageInbound::default()),
            o_poller: None,
        }
    }
}

impl HelloWorldV1Hook {
    fn helper<'a, 'b: 'a>(&'b mut self) -> StateHelper<'a> {
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        StateHelper {
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
    ) -> Result<(), CatscopeGuestError> {
        assert!(self.o_poller.replace(poller.clone()).is_none());
        let g = Graph::new(poller)?;
        assert!(self.o_rc_graph.replace(g).is_none());
        let mut helper = self.helper();
        helper.on_load();
        log_info!("on_load - 1");
        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let mut outbound = self.msg_outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
        }
        outbound.flush();
        self.msg_outbound.replace(outbound);
        log_info!("on_load - 2");
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), CatscopeGuestError> {
        log_info!("on_unload - 1");
        Ok(())
    }

    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError> {
        let mut parser = self.o_msg_parser.take().unwrap();
        let mut helper = self.helper();
        log_debug!("HelloWorldV1Hook::event - 1");
        match event {
            Event::Stdin(data) => {
                //log_info!("HelloWorldV1Hook::event - stdin");
                parser.parse(&data).unwrap();
                let action: MessageAction<Configuration, CustomMessageInbound> = {
                    let x = &parser;
                    x.try_into()?
                };
                helper.on_message(action);
            }
            Event::Commit(commit) => {
                //log_info!("HelloWorldV1Hook::event - commit");
                commit.process(&mut helper);
            }
            Event::Account(account_wrapper) => {
                helper.mid_on_account(account_wrapper);
            }
            Event::Token(tokenaccountv1s) => {
                helper.mid_on_token(tokenaccountv1s);
            }
            Event::Transaction(transaction_list) => {
                helper.mid_on_tx(transaction_list);
            }
            Event::SlotStatus(slot, status) => {
                helper.on_slot_status(slot, status);
            }
        };
        //log_warn!("HelloWorldV1Hook::event - 2");
        helper.evaluate();
        //log_warn!("HelloWorldV1Hook::event - 3");

        let q_msg = rc_unlock_mut(&self.tmp_q_msg);
        let mut outbound = self.msg_outbound.take().unwrap();
        while let Some(message) = q_msg.pop_front() {
            outbound.write(message);
            outbound.flush();
        }
        //log_warn!("HelloWorldV1Hook::event - 4");
        self.o_msg_parser.replace(parser);
        self.msg_outbound.replace(outbound);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}
