//! Runtime configuration for `multimodelv1`, populated from the Go brain
//! via stdin messages -- mirrors `leveragedloopv1::configuration`'s shape
//! exactly (same fields, same reasoning: `wallet` set once
//! `CustomMessageInbound::Wallet` arrives; `mint_sol`/`mint_usdc` fixed
//! constants resolved to `AccountId`). Both mints matter here beyond just
//! "a stable settlement currency": `mint_usdc` sizes every strategy's
//! margin/spot legs (`PLAN-1.md` Phase 2/3), `mint_sol` is the dispersion
//! trade's own index leg (`PLAN-1.md` Phase 5, point 3) -- not carried
//! over by habit, both are real, load-bearing constants for the
//! decision/execution logic later phases add on top of this skeleton.
use std::{cell::UnsafeCell, rc::Rc, time::Instant};

use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer as _};

use crate::{
    graph::AccountId,
    util::{account_id_from_pubkey, rc_unlock},
};

#[derive(Debug)]
#[repr(C, align(8))]
pub struct Configuration {
    pub(crate) start: Instant,
    pub(crate) count: usize,
    pub(crate) wallet: AccountId,
    pub(crate) mint_sol: AccountId,
    pub(crate) mint_usdc: AccountId,
    #[allow(dead_code)]
    pub(crate) max_slippage: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: 0,
            wallet: Default::default(),
            mint_sol: Default::default(),
            mint_usdc: Default::default(),
            max_slippage: 0.01,
        }
    }
}

impl Configuration {
    pub fn set(&mut self, rc_keypair: &Rc<UnsafeCell<Keypair>>) {
        let keypair = rc_unlock(rc_keypair);
        let pubkey = keypair.pubkey();
        self.wallet = account_id_from_pubkey(&pubkey);
        self.mint_sol = account_id_from_pubkey(&MINT_SOL);
        self.mint_usdc = account_id_from_pubkey(&MINT_USDC);
    }
}

const MINT_SOL: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");
/// `pub(crate)`, not private: `state::curated_symbols` (a free function,
/// no `self`/`Configuration` access) also needs this real constant, to
/// exclude USDC from the pair-trading candidate universe -- it's the
/// strategy's own settlement currency, not a risky asset with its own
/// residual dynamics, and picking it as a leg would degenerate to a
/// USDC->USDC swap.
pub(crate) const MINT_USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
