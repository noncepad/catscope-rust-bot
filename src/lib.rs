wit_bindgen::generate!({
    world: "catscopevalidator",
    path: "wit",
    generate_all,
});
use crate::{
    brain::Merged, event_loop::run,
};
use exports::wasi::cli::run::Guest;
use std::{
    cell::{RefCell},
    rc::Rc,
};

pub mod brain;
pub mod bundler_message;
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
    /// This is the entry point for the bot.
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

/// Load in configuration data during build time to save on memory
/// during runtime.
pub mod atl_config {
    include!(concat!(env!("OUT_DIR"), "/address_lookup_table.rs"));
}
/// Load in configuration data during build time to save on memory
/// during runtime.
pub mod trading_config {
    include!(concat!(env!("OUT_DIR"), "/trading_data.rs"));
}
/// `TRADE_ROUTER_PROBE_LAMPORTS` -- see build.rs's
/// `trade_router_probe_lamports` doc comment.
pub mod diagnostic_config {
    include!(concat!(env!("OUT_DIR"), "/diagnostic_data.rs"));
}
/// `BUNDLER` -- which bundler `Wallet::drain_and_send()` requests from
/// the host when batching. See build.rs's `bundler` doc comment.
pub mod bundler_config {
    include!(concat!(env!("OUT_DIR"), "/bundler_data.rs"));
}
/// Orca Whirlpool pool list (`OrcaWhirlpoolRaw`), generated at build time
/// from the `orca_whirlpool_pool` table in the unified prefetch db (SQL_PATH).
pub mod orca_config {
    include!(concat!(env!("OUT_DIR"), "/orca_data.rs"));
}
/// Load in configuration data during build time to save on memory
/// during runtime.
pub mod raydium_amm_config {
    include!(concat!(env!("OUT_DIR"), "/raydium_amm_data.rs"));
}
/// Load in configuration data during build time to save on memory
/// during runtime.
pub mod raydium_clmm_config {
    include!(concat!(env!("OUT_DIR"), "/raydium_clmm_data.rs"));
}
/// Load in configuration data during build time to save on memory
/// during runtime.
pub mod raydium_cpmm_config {
    include!(concat!(env!("OUT_DIR"), "/raydium_cpmm_data.rs"));
}
/// Kamino Lending reserve list (`KaminoReserveRaw`), generated at build time
/// from the `kamino_reserve` table in the unified prefetch db (SQL_PATH).
pub mod kamino_config {
    include!(concat!(env!("OUT_DIR"), "/kamino_data.rs"));
}
/// Sanctum S Controller LST list (`SanctumLstRaw`), generated at build time
/// from the `sanctum_lst` table in the unified prefetch db (SQL_PATH).
pub mod sanctum_config {
    include!(concat!(env!("OUT_DIR"), "/sanctum_data.rs"));
}
/// De-duplicated list of the highest-liquidity SPL token vaults this bot
/// needs a live balance for, across all protocols -- the source for the
/// validator's batch AccountId subscription list. Generated at build time
/// from the same BFS liquidity-USD graph as `router_pools_config`, capped
/// to a fixed account-count budget (TRACKED_ACCOUNTS_BUDGET, default
/// 50_000) since this list is held and iterated by the wasm bot at
/// runtime; see build.rs's tracked_accounts_budget()/min_liquidity_usd()
/// for the exact ranking and cutoff, and why marginfi/sanctum are
/// excluded.
pub mod tracked_accounts_config {
    include!(concat!(env!("OUT_DIR"), "/tracked_accounts_data.rs"));
}
/// marginfi-v2 Bank list (`MarginfiBankRaw`), generated at build time from
/// the `marginfi_bank` table in the unified prefetch db (SQL_PATH).
pub mod marginfi_config {
    include!(concat!(env!("OUT_DIR"), "/marginfi_data.rs"));
}
/// Solend/Save Reserve list (`SolendReserveRaw`), generated at build time
/// from the `solend_reserve` table in the unified prefetch db (SQL_PATH).
pub mod solend_config {
    include!(concat!(env!("OUT_DIR"), "/solend_data.rs"));
}
/// Drift v2 SpotMarket list (`DriftSpotMarketRaw`), generated at build time
/// from the `drift_spot_market` table in the unified prefetch db (SQL_PATH).
pub mod drift_config {
    include!(concat!(env!("OUT_DIR"), "/drift_data.rs"));
}
/// Jet Protocol V1 Reserve list (`JetReserveRaw`), generated at build time
/// from the `jet_reserve` table in the unified prefetch db (SQL_PATH).
pub mod jet_config {
    include!(concat!(env!("OUT_DIR"), "/jet_data.rs"));
}
/// Scalar config for `trader::router::Router` (lambda, min-cluster-liquidity
/// threshold, the 5 core mints), generated at build time from router.json.
pub mod router_config {
    include!(concat!(env!("OUT_DIR"), "/router_config.rs"));
}
/// Raw-anchor-unit liquidity graph across Raydium + Orca pools, used to seed
/// `trader::router::Router`'s tier partitioning at startup — see
/// `RouterPoolRaw` in build.rs for field semantics and caveats.
pub mod router_pools_config {
    include!(concat!(env!("OUT_DIR"), "/router_pools_data.rs"));
}
/// Unified top-N-by-liquidity pools across every dex the optimizer tracks
/// (Raydium AMM v4/CPMM/CLMM, Orca Whirlpools) — see TopPool in build.rs for
/// field semantics.
pub mod top_pools_config {
    include!(concat!(env!("OUT_DIR"), "/top_pools_data.rs"));
}
/// Pump.fun bonding-curve mint list (`PumpfunBondingCurveRaw`), generated
/// at build time from the `pumpfun_bonding_curve` table in the unified
/// prefetch db (SQL_PATH) — just the mint, since `bonding_curve` is a
/// fixed deterministic PDA of `(program, mint)` and `PumpfunState::new()`
/// derives it directly.
pub mod pumpfun_config {
    include!(concat!(env!("OUT_DIR"), "/pumpfun_data.rs"));
}
/// PumpSwap pool list (`PumpswapPoolRaw`), generated at build time from
/// the `pumpswap_pool` table in the unified prefetch db (SQL_PATH) --
/// embeds vault pubkeys directly (unlike Orca) so `PumpswapState::new()`
/// can subscribe to a pool and both vaults in one shot.
pub mod pumpswap_config {
    include!(concat!(env!("OUT_DIR"), "/pumpswap_data.rs"));
}
/// Phoenix perpetuals market list (`PhoenixMarketRaw`), generated at build
/// time from the `phoenix_market` table in the unified prefetch db
/// (SQL_PATH) -- see `optimizer/prefetch/phoenix` for the Go-side
/// discovery (a two-hop depth-1 cascade, no edge-generator FilterEdges
/// needed) and `trader::dex::phoenix`'s module doc.
pub mod phoenix_config {
    include!(concat!(env!("OUT_DIR"), "/phoenix_data.rs"));
}
/// Curated symbol -> mint join key (`SymbolMintRaw`), hand-maintained in
/// `build.rs` (not database-derived -- prefetch.db has no symbol/ticker
/// data anywhere) for the handful of assets with confirmed real perp
/// coverage on both Phoenix and Velocity. See
/// `trader::dex::velocity::state::TRACKED_MARKETS` and
/// `trader::dex::phoenix::PhoenixMarketState::base_mint`'s doc comments.
pub mod symbol_mint_config {
    include!(concat!(env!("OUT_DIR"), "/symbol_mint_data.rs"));
}
/// A pair-trading candidate universe (`TradeUniverseSymbolRaw`), generated
/// at build time from every distinct mint with a reserve on
/// `trader::dex::kamino::KAMINO_MAIN_MARKET` *or*
/// `trader::dex::solend::SOLEND_MAIN_MARKET` in the unified prefetch db
/// (SQL_PATH), deduped by mint, joined against `mint_info` for real
/// decimals. Unlike `symbol_mint_config` above (hand-curated,
/// Phoenix+Velocity-perp-gated), this list is database-derived and not
/// gated on perp coverage -- the pair trade itself is real Kamino *or*
/// Solend deposit/borrow (protocol picked per-leg at runtime), no perp
/// hedge leg.
pub mod trade_universe_config {
    include!(concat!(env!("OUT_DIR"), "/trade_universe_data.rs"));
}
/// Default target portfolio allocation (`(symbol, allocation_pct)`,
/// fraction of total portfolio value 0.0-1.0) per curated symbol, baked
/// in at build time from `perp_funding_target_allocation` -- the same
/// table `optimizer/brain/testperpv1`'s `SendTargetAllocation`
/// writes into at runtime. See
/// `brain::testperpv1::state::State::target_allocation_pct`.
pub mod target_allocation_config {
    include!(concat!(env!("OUT_DIR"), "/target_allocation_data.rs"));
}

export!(Component);
