use std::{cell::UnsafeCell, rc::Rc};

use crate::{
    brain::{
        marketwatchv1::MarketWatchV1Hook, testlatencylitev1::TestLatencyLiteV1Hook,
        testperplatencyv1::TestPerpLatencyV1Hook, xstockshealthv1::XstocksHealthV1Hook,
    },
    event_loop::EventHandler,
    message::Parser,
};

pub mod marketwatchv1;
pub mod testlatencylitev1;
pub mod testperplatencyv1;
pub mod xstockshealthv1;

pub struct Merged {
    inner: Option<BotMode>,
}

enum BotMode {
    //    PhoenixPerps(Box<PhoenixPerpsV1Hook>),
    //   TestPerp(Box<TestPerpV1Hook>),
    TestPerpLatency(Box<TestPerpLatencyV1Hook>),
    TestLatencyLiteV1(Box<TestLatencyLiteV1Hook>),
    //    LeveragedLoop(Box<LeveragedLoopV1Hook>),
    //   MultiModel(Box<MultiModelV1Hook>),
    MarketWatchV1(Box<MarketWatchV1Hook>),
    //   MidOnTxHealth(Box<MidOnTxHealthV1Hook>),
    XstocksHealth(Box<XstocksHealthV1Hook>),
}

impl Default for Merged {
    fn default() -> Self {
        let mode = match std::env::var("MODE") {
            Ok(x) => x,
            Err(_e) => panic!("env var MODE not set"),
        };

        let inner = match mode.as_str() {
            "testperplatencyv1" => BotMode::TestPerpLatency(Box::new(TestPerpLatencyV1Hook::new(
                Rc::new(UnsafeCell::new(Parser::default())),
            ))),
            "testlatencylitev1" => BotMode::TestLatencyLiteV1(Box::new(
                TestLatencyLiteV1Hook::new(Rc::new(UnsafeCell::new(Parser::default()))),
            )),
            "marketwatchv1" => BotMode::MarketWatchV1(Box::new(MarketWatchV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "xstockshealthv1" => BotMode::XstocksHealth(Box::new(XstocksHealthV1Hook::new(
                Rc::new(UnsafeCell::new(Parser::default())),
            ))),
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
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::XstocksHealth(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::XstocksHealth(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn on_unload(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_unload();
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::XstocksHealth(mut x) => {
                r = x.on_unload();
                inner = BotMode::XstocksHealth(x);
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
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_event(event);
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::XstocksHealth(mut x) => {
                r = x.on_event(event);
                inner = BotMode::XstocksHealth(x);
            }
        };
        self.inner.replace(inner);
        r
    }

    fn flush(&mut self) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::TestPerpLatency(mut x) => {
                r = x.flush();
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.flush();
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.flush();
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::XstocksHealth(mut x) => {
                r = x.flush();
                inner = BotMode::XstocksHealth(x);
            }
        };
        self.inner.replace(inner);
        r
    }
}
