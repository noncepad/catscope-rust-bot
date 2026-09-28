//! Runtime configuration for `midontxhealthv1` -- structurally required
//! by `message::Parser<Configuration, ..>`, but this bot needs none of
//! the usual `wallet`/`mint_sol`/`mint_usdc` fields (see `state`'s own
//! doc comment: passive listen-only, no wallet, no transactions). Kept
//! to just what `MessageAction::AdjustConfiguration` needs to exist at
//! all.
use std::time::Instant;

#[derive(Debug)]
#[repr(C, align(8))]
pub struct Configuration {
    pub(crate) start: Instant,
    pub(crate) count: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: 0,
        }
    }
}
