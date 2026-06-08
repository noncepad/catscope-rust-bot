wit_bindgen::generate!({
    world: "catscopevalidator",
    path: "wit",
    generate_all,
});
use crate::{
    brain::Merged, event_loop::run, graph::AccountId, message::Parser,
    trader::dex::orca::OrcaPoolSetup, util::account_id_from_pubkey,
};
use exports::wasi::cli::run::Guest;
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};

pub mod brain;
pub mod crypt;
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
        let args = std::env::args();
        let mut l_arg = Vec::new();
        for x in args {
            l_arg.push(x);
        }
        let b = Merged::default();
        let sampler = Rc::new(RefCell::new(b));

        let r = run(sampler, l_arg);
        if let Err(e) = r {
            panic!("program exited with error: {e}")
        }
        Ok(())
    }
}
pub mod trading_config {
    include!(concat!(env!("OUT_DIR"), "/trading_data.rs"));
}
pub mod orca_config {
    include!(concat!(env!("OUT_DIR"), "/orca_data.rs"));
}
pub struct TradingSetup {
    l_pair: Vec<[AccountId; 2]>,
    l_orca: Vec<OrcaPoolSetup>,
}
impl TradingSetup {
    pub fn pairs(&self) -> &[[AccountId; 2]] {
        &self.l_pair
    }
    pub fn orca(&self) -> &[OrcaPoolSetup] {
        &self.l_orca
    }
}
impl Default for TradingSetup {
    fn default() -> Self {
        let count = trading_config::TRADING_PAIRS.len();
        let mut l_pair = vec![[0, 0]; count];
        let mut z = [0u8; 32];

        for (i, (t_a, t_b)) in trading_config::TRADING_PAIRS.iter().enumerate() {
            z.copy_from_slice(t_a);
            let a = Pubkey::new_from_array(z);
            z.copy_from_slice(t_b);
            let b = Pubkey::new_from_array(z);
            l_pair[i] = [account_id_from_pubkey(&a), account_id_from_pubkey(&b)];
        }

        let mut l_orca = Vec::with_capacity(orca_config::ORCA_POOLS.len());
        {
            let mut z1 = [0u8; 32];
            let mut p;
            for (pubkey_id, mint_a_id, mint_b_id) in orca_config::ORCA_POOLS.iter() {
                //                let pk = decode(&pool.pubkey, "pubkey");
                //               let ma = decode(&pool.mint_a, "mint_a");
                //              let mb = decode(&pool.mint_b, "mint_b");
                z1.copy_from_slice(pubkey_id);
                p = Pubkey::new_from_array(z1);
                let pubkey = account_id_from_pubkey(&p);

                z1.copy_from_slice(mint_a_id);
                p = Pubkey::new_from_array(z1);
                let mint_a = account_id_from_pubkey(&p);

                z1.copy_from_slice(mint_b_id);
                p = Pubkey::new_from_array(z1);
                let mint_b = account_id_from_pubkey(&p);
                l_orca.push(OrcaPoolSetup {
                    pubkey,
                    mint_a,
                    mint_b,
                });
            }
        }
        Self { l_pair, l_orca }
    }
}

export!(Component);
