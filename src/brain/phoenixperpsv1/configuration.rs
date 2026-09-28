//! Runtime configuration for the Phoenix perpetuals strategy, populated
//! from the Go brain via stdin messages (mirrors `arbv1::configuration`'s
//! shape). The wallet field is set once when the `Wallet` message arrives;
//! everything else is a tunable with a conservative default.
use std::time::Instant;

use crate::graph::AccountId;

#[derive(Debug)]
#[repr(C, align(8))]
pub struct Configuration {
    pub(crate) start: Instant,
    pub(crate) count: usize,
    pub(crate) wallet: AccountId,
    /// Re-check margin health every N commits (~12s each) -- mirrors
    /// `arbv1::state`'s `last_slot % 100` idiom, see `state.rs::evaluate`.
    pub(crate) margin_check_interval: u64,
    /// Close/flag a position once `equity < liquidation_threshold *
    /// maintenance_margin` -- see `margin::is_at_liquidation_risk`.
    /// Deliberately generous (well above the bare 1.0 liquidation line) so
    /// action happens before the protocol itself would liquidate.
    pub(crate) liquidation_threshold: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: 0,
            wallet: Default::default(),
            margin_check_interval: 100,
            liquidation_threshold: 1.5,
        }
    }
}
