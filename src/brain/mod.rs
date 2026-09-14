use std::{cell::UnsafeCell, rc::Rc};

use crate::{
    brain::{arbv1::ArbitrageV1Hook, testperpv1::TestPerpV1Hook},
    event_loop::EventHandler,
    message::Parser,
};

pub mod arbv1;
pub mod testperpv1;

pub struct Merged {
    inner: Option<BotMode>,
}

enum BotMode {
    Arbitrage(Box<ArbitrageV1Hook>),
    TestPerp(Box<TestPerpV1Hook>),
}

impl Default for Merged {
    fn default() -> Self {
        let mode = match std::env::var("MODE") {
            Ok(x) => x,
            Err(_e) => panic!("env var MODE not set"),
        };

        let inner = match mode.as_str() {
            "arbv1" => BotMode::Arbitrage(Box::new(ArbitrageV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "testperpv1" => BotMode::TestPerp(Box::new(TestPerpV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            _ => panic!("unknown mode {mode}",),
        };
        Self { inner: Some(inner) }
    }
}

impl EventHandler for Merged {
    fn on_load(
        &mut self,
        poller: crate::event_loop::EventPoller,
        args: &[String],
    ) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::Arbitrage(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::Arbitrage(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestPerp(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn on_unload(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::Arbitrage(mut x) => {
                r = x.on_unload();
                inner = BotMode::Arbitrage(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestPerp(x);
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
            BotMode::Arbitrage(mut x) => {
                r = x.on_event(event);
                inner = BotMode::Arbitrage(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestPerp(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn flush(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::Arbitrage(mut x) => {
                r = x.flush();
                inner = BotMode::Arbitrage(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.flush();
                inner = BotMode::TestPerp(x);
            }
        };
        self.inner.replace(inner);
        r
    }
}
