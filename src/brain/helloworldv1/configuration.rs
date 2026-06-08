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
    pub(crate) max_slippage: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            count: Default::default(),
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
const MINT_USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
