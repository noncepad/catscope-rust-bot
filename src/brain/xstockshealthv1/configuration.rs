//! Runtime configuration for `xstockshealthv1` -- mirrors
//! `testperplatencyv1lite::configuration`'s shape, trimmed to just what
//! this mode needs: a wallet identity (required by the standard
//! handshake even though this mode sends no transactions yet -- see
//! `state::StateHelper::on_message`'s `Wallet` arm).
use std::time::Instant;

use crate::graph::AccountId;

#[derive(Debug)]
#[repr(C, align(8))]
pub struct Configuration {
    pub(crate) start: Instant,
    pub(crate) count: usize,
    pub(crate) wallet: AccountId,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: 0,
            wallet: Default::default(),
        }
    }
}
