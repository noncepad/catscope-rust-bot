use crate::{
    brain::{bouncerv1::BouncerV1Hook, helloworldv1::HelloWorldV1Hook},
    event_loop::EventHandler,
};

pub mod bouncerv1;
pub mod helloworldv1;

pub struct Merged {
    inner: Option<BotMode>,
}

enum BotMode {
    Bouncer(Box<BouncerV1Hook>),
    HelloWorld(Box<HelloWorldV1Hook>),
}

impl Default for Merged {
    fn default() -> Self {
        let mode = match std::env::var("MODE") {
            Ok(x) => x,
            Err(_e) => panic!("env var MODE not set"),
        };
        let inner = match mode.as_str() {
            "BOUNCER" => BotMode::Bouncer(Box::default()),
            "HELLOWORLD" => BotMode::HelloWorld(Box::default()),
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
    ) -> Result<(), crate::err::CatscopeGuestError> {
        let mut inner = self.inner.take().unwrap();
        let r;
        match inner {
            BotMode::Bouncer(mut x) => {
                r = x.on_load(poller, rc_edgemgr, args);
                inner = BotMode::Bouncer(x);
            }
            BotMode::HelloWorld(mut x) => {
                r = x.on_load(poller, rc_edgemgr, args);
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
            BotMode::Bouncer(mut x) => {
                r = x.on_unload();
                inner = BotMode::Bouncer(x);
            }
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
            BotMode::Bouncer(mut x) => {
                r = x.on_event(event);
                inner = BotMode::Bouncer(x);
            }
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
            BotMode::Bouncer(mut x) => {
                r = x.flush();
                inner = BotMode::Bouncer(x);
            }
            BotMode::HelloWorld(mut x) => {
                r = x.flush();
                inner = BotMode::HelloWorld(x);
            }
        };
        self.inner.replace(inner);
        r
    }
}
