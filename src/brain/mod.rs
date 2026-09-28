use std::{cell::UnsafeCell, rc::Rc};

use crate::{
    brain::{
        arbv1::ArbitrageV1Hook, helloworldv1::HelloWorldV1Hook,
        leveragedloopv1::LeveragedLoopV1Hook, marketwatchv1::MarketWatchV1Hook,
        midontxhealthv1::MidOnTxHealthV1Hook, multimodelv1::MultiModelV1Hook,
        perpfundingv1::PerpFundingV1Hook, phoenixperpsv1::PhoenixPerpsV1Hook,
        testlatencylitev1::TestLatencyLiteV1Hook, testperplatencyv1::TestPerpLatencyV1Hook,
        testperplatencyv1lite::TestPerpLatencyV1LiteHook, testperpv1::TestPerpV1Hook,
        xstockshealthv1::XstocksHealthV1Hook,
    },
    event_loop::EventHandler,
    message::Parser,
};

pub mod arbv1;
pub mod helloworldv1;
pub mod leveragedloopv1;
// Read-only market-statistics dashboard feed -- see that module's own
// doc comment. No wallet, no trading; its counterpart is
// `optimizer/brain/marketwatchv1`'s terminal dashboard.
pub mod marketwatchv1;
// Passive, listen-only diagnostic: checks whether the Transaction-lane
// event stream (Event::Transaction -> mid_on_tx) delivers every real
// transaction per slot, or silently drops some -- see that module's own
// doc comment for the real, live-measured finding motivating it. Sends
// no transactions, needs no wallet. Delete this module (and its BotMode
// wiring below) once that question is settled, same convention
// `testperplatencyv1lite` itself follows.
pub mod midontxhealthv1;
// PLAN-1.md Phase 5 sub-phase 5a: idle-verified only (real EventHandler,
// wallet + DexState/TradeRouter subscriptions, no decision/execution
// logic yet) -- see `multimodelv1::state`'s own doc comment.
pub mod multimodelv1;
pub mod perpfundingv1;
pub mod phoenixperpsv1;
// Byte-for-byte copy of `testperplatencyv1` with the unconditional
// DEX/lending subscription setup stripped out -- see that module's own
// doc comment for the real, live-measured evidence motivating it.
pub mod testlatencylitev1;
pub mod testperplatencyv1;
// Experimental copy of `testperplatencyv1` with the DEX/lending
// subscription setup stripped out -- see that module's own doc comment
// for why. Delete this module (and its `BotMode` wiring below) once the
// native-transfer-latency question it exists to answer is settled.
pub mod testperplatencyv1lite;
pub mod testperpv1;
// Stocklana hackathon plan (HACKATHON_PLAN.md, edge-generator repo):
// watches Kamino xStocks obligations/reserves and logs a health-factor
// board. Passive, listen-only, sends no transactions -- see that
// module's own doc comment.
pub mod xstockshealthv1;

pub struct Merged {
    inner: Option<BotMode>,
}

enum BotMode {
    HelloWorld(Box<HelloWorldV1Hook>),
    Arbitrage(Box<ArbitrageV1Hook>),
    PhoenixPerps(Box<PhoenixPerpsV1Hook>),
    PerpFunding(Box<PerpFundingV1Hook>),
    TestPerp(Box<TestPerpV1Hook>),
    TestPerpLatency(Box<TestPerpLatencyV1Hook>),
    TestPerpLatencyLite(Box<TestPerpLatencyV1LiteHook>),
    TestLatencyLiteV1(Box<TestLatencyLiteV1Hook>),
    LeveragedLoop(Box<LeveragedLoopV1Hook>),
    MultiModel(Box<MultiModelV1Hook>),
    MarketWatchV1(Box<MarketWatchV1Hook>),
    MidOnTxHealth(Box<MidOnTxHealthV1Hook>),
    XstocksHealth(Box<XstocksHealthV1Hook>),
}

impl Default for Merged {
    fn default() -> Self {
        let mode = match std::env::var("MODE") {
            Ok(x) => x,
            Err(_e) => panic!("env var MODE not set"),
        };

        let inner = match mode.as_str() {
            "helloworldv1" => BotMode::HelloWorld(Box::new(HelloWorldV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "arbv1" => BotMode::Arbitrage(Box::new(ArbitrageV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "phoenixperpsv1" => BotMode::PhoenixPerps(Box::new(PhoenixPerpsV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "perpfundingv1" => BotMode::PerpFunding(Box::new(PerpFundingV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "testperpv1" => BotMode::TestPerp(Box::new(TestPerpV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "testperplatencyv1" => BotMode::TestPerpLatency(Box::new(
                TestPerpLatencyV1Hook::new(Rc::new(UnsafeCell::new(Parser::default()))),
            )),
            "testperplatencyv1lite" => BotMode::TestPerpLatencyLite(Box::new(
                TestPerpLatencyV1LiteHook::new(Rc::new(UnsafeCell::new(Parser::default()))),
            )),
            "testlatencylitev1" => BotMode::TestLatencyLiteV1(Box::new(
                TestLatencyLiteV1Hook::new(Rc::new(UnsafeCell::new(Parser::default()))),
            )),
            "leveragedloopv1" => BotMode::LeveragedLoop(Box::new(LeveragedLoopV1Hook::new(
                Rc::new(UnsafeCell::new(Parser::default())),
            ))),
            "multimodelv1" => BotMode::MultiModel(Box::new(MultiModelV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "marketwatchv1" => BotMode::MarketWatchV1(Box::new(MarketWatchV1Hook::new(Rc::new(
                UnsafeCell::new(Parser::default()),
            )))),
            "midontxhealthv1" => BotMode::MidOnTxHealth(Box::new(MidOnTxHealthV1Hook::new(
                Rc::new(UnsafeCell::new(Parser::default())),
            ))),
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
            BotMode::HelloWorld(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::HelloWorld(x);
            }
            BotMode::Arbitrage(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::Arbitrage(x);
            }
            BotMode::PhoenixPerps(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::PhoenixPerps(x);
            }
            BotMode::PerpFunding(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::PerpFunding(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestPerp(x);
            }
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestPerpLatencyLite(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestPerpLatencyLite(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::LeveragedLoop(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::LeveragedLoop(x);
            }
            BotMode::MultiModel(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::MultiModel(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::MidOnTxHealth(mut x) => {
                r = x.on_load(poller, args);
                inner = BotMode::MidOnTxHealth(x);
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
            BotMode::HelloWorld(mut x) => {
                r = x.on_unload();
                inner = BotMode::HelloWorld(x);
            }
            BotMode::Arbitrage(mut x) => {
                r = x.on_unload();
                inner = BotMode::Arbitrage(x);
            }
            BotMode::PhoenixPerps(mut x) => {
                r = x.on_unload();
                inner = BotMode::PhoenixPerps(x);
            }
            BotMode::PerpFunding(mut x) => {
                r = x.on_unload();
                inner = BotMode::PerpFunding(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestPerp(x);
            }
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestPerpLatencyLite(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestPerpLatencyLite(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_unload();
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::LeveragedLoop(mut x) => {
                r = x.on_unload();
                inner = BotMode::LeveragedLoop(x);
            }
            BotMode::MultiModel(mut x) => {
                r = x.on_unload();
                inner = BotMode::MultiModel(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_unload();
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::MidOnTxHealth(mut x) => {
                r = x.on_unload();
                inner = BotMode::MidOnTxHealth(x);
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
            BotMode::HelloWorld(mut x) => {
                r = x.on_event(event);
                inner = BotMode::HelloWorld(x);
            }
            BotMode::Arbitrage(mut x) => {
                r = x.on_event(event);
                inner = BotMode::Arbitrage(x);
            }
            BotMode::PhoenixPerps(mut x) => {
                r = x.on_event(event);
                inner = BotMode::PhoenixPerps(x);
            }
            BotMode::PerpFunding(mut x) => {
                r = x.on_event(event);
                inner = BotMode::PerpFunding(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestPerp(x);
            }
            BotMode::TestPerpLatency(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestPerpLatencyLite(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestPerpLatencyLite(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.on_event(event);
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::LeveragedLoop(mut x) => {
                r = x.on_event(event);
                inner = BotMode::LeveragedLoop(x);
            }
            BotMode::MultiModel(mut x) => {
                r = x.on_event(event);
                inner = BotMode::MultiModel(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.on_event(event);
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::MidOnTxHealth(mut x) => {
                r = x.on_event(event);
                inner = BotMode::MidOnTxHealth(x);
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
            BotMode::HelloWorld(mut x) => {
                r = x.flush();
                inner = BotMode::HelloWorld(x);
            }
            BotMode::Arbitrage(mut x) => {
                r = x.flush();
                inner = BotMode::Arbitrage(x);
            }
            BotMode::PhoenixPerps(mut x) => {
                r = x.flush();
                inner = BotMode::PhoenixPerps(x);
            }
            BotMode::PerpFunding(mut x) => {
                r = x.flush();
                inner = BotMode::PerpFunding(x);
            }
            BotMode::TestPerp(mut x) => {
                r = x.flush();
                inner = BotMode::TestPerp(x);
            }
            BotMode::TestPerpLatency(mut x) => {
                r = x.flush();
                inner = BotMode::TestPerpLatency(x);
            }
            BotMode::TestPerpLatencyLite(mut x) => {
                r = x.flush();
                inner = BotMode::TestPerpLatencyLite(x);
            }
            BotMode::TestLatencyLiteV1(mut x) => {
                r = x.flush();
                inner = BotMode::TestLatencyLiteV1(x);
            }
            BotMode::LeveragedLoop(mut x) => {
                r = x.flush();
                inner = BotMode::LeveragedLoop(x);
            }
            BotMode::MultiModel(mut x) => {
                r = x.flush();
                inner = BotMode::MultiModel(x);
            }
            BotMode::MarketWatchV1(mut x) => {
                r = x.flush();
                inner = BotMode::MarketWatchV1(x);
            }
            BotMode::MidOnTxHealth(mut x) => {
                r = x.flush();
                inner = BotMode::MidOnTxHealth(x);
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
