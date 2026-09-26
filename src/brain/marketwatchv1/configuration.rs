//! Runtime configuration for marketwatchv1. Unlike arbv1's, this carries
//! no wallet/signing state at all -- this bot never signs or sends a
//! transaction, only reads live pool prices -- just the two fixed mint
//! IDs `market_stats`'s USDC-quoting probe needs.
use std::time::Instant;

use solana_sdk::pubkey::Pubkey;

use crate::{graph::AccountId, util::account_id_from_pubkey};

#[derive(Debug)]
#[repr(C, align(8))]
pub struct Configuration {
    pub(crate) start: Instant,
    pub(crate) count: usize,
    pub(crate) mint_sol: AccountId,
    pub(crate) mint_usdc: AccountId,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: Default::default(),
            mint_sol: Default::default(),
            mint_usdc: Default::default(),
        }
    }
}

impl Configuration {
    /// Resolves the fixed mint constants to real `AccountId`s -- deferred
    /// out of `Default::default()` and called from `on_load` instead,
    /// same safety timing arbv1's own `Configuration::set` used:
    /// `account_id_from_pubkey` is a WIT host import that aborts outside
    /// the real WASM guest runtime, which isn't guaranteed to be live yet
    /// at `Default::default()` time (bot construction), only by `on_load`.
    pub fn resolve_mints(&mut self) {
        self.mint_sol = account_id_from_pubkey(&MINT_SOL);
        self.mint_usdc = account_id_from_pubkey(&MINT_USDC);
    }
}

const MINT_SOL: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");
const MINT_USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
