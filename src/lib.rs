wit_bindgen::generate!({
    world: "catscopevalidator",
    path: "wit",
    generate_all,
});
use crate::{brain::bouncerv1::BouncerV1Hook, event_loop::run};
use exports::wasi::cli::run::Guest;
use std::{cell::RefCell, rc::Rc};

pub mod brain;
pub mod err;
pub(crate) mod event;
pub mod event_loop;
pub(crate) mod graph;
pub mod message;
pub(crate) mod stdio;
pub mod token;
pub mod trader;
pub mod tx;
pub mod txview;
pub(crate) mod util;
pub mod wallet;

struct Component;

impl Guest for Component {
    fn run() -> Result<(), ()> {
        let sampler = Rc::new(RefCell::new(BouncerV1Hook::default()));
        //let sampler = Rc::new(RefCell::new(ArbV1Hook::default()));
        let r = run(sampler);
        if let Err(e) = r {
            panic!("program exited with error: {e}")
        }
        Ok(())
    }
}

export!(Component);
