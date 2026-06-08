use std::{cell::UnsafeCell, rc::Rc};

use crate::{
    brain::helloworldv1::{
        message::{
            CustomMessageInbound as HelloCustomMessageInbound,
            CustomMessageOutbound as HelloCustomMessageOutbound,
        },
        HelloWorldV1Hook,
    },
    event_loop::EventHandler,
    message::{MessageDeserializer, MessageSend, MessageSerializer, Parser},
    TradingSetup,
};

pub mod helloworldv1;

pub struct Merged {
    inner: Option<BotMode>,
}

enum BotMode {
    HelloWorld(Box<HelloWorldV1Hook>),
}

impl Default for Merged {
    fn default() -> Self {
        let mode = match std::env::var("MODE") {
            Ok(x) => x,
            Err(_e) => panic!("env var MODE not set"),
        };
        let rc_parser = Rc::new(UnsafeCell::new(Parser::default()));

        let inner = match mode.as_str() {
            "helloworldv1" => BotMode::HelloWorld(Box::new(HelloWorldV1Hook::new(rc_parser))),
            _ => panic!("unknown mode {mode}",),
        };
        Self { inner: Some(inner) }
    }
}

impl EventHandler for Merged {
    fn on_load(
        &mut self,
        poller: crate::event_loop::EventPoller,
        rc_edgemgr: std::rc::Rc<std::cell::UnsafeCell<crate::graph::EdgeManager>>,
        args: &[String],
        trading: &TradingSetup,
    ) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::HelloWorld(mut x) => {
                r = x.on_load(poller, rc_edgemgr, args, trading);
                inner = BotMode::HelloWorld(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn on_unload(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::HelloWorld(mut x) => {
                r = x.on_unload();
                inner = BotMode::HelloWorld(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn on_event(
        &mut self,
        event: crate::event::Event,
    ) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::HelloWorld(mut x) => {
                r = x.on_event(event);
                inner = BotMode::HelloWorld(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn flush(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::HelloWorld(mut x) => {
                r = x.flush();
                inner = BotMode::HelloWorld(x);
            }
        };
        self.inner.replace(inner);
        r
    }
}
