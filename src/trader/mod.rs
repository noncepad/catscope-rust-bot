//! Multi-DEX trading interface for Raydium, Orca, and Kamino.
//!
//! # Quick start
//!
//! ```rust,ignore
//! let mut trader = DexTrader::new();
//!
//! // Register the pools you want to trade through.
//! trader.register_raydium_amm(pool_account_id, Some(serum_cfg));
//! trader.register_orca_whirlpool(whirlpool_account_id);
//! trader.register_kamino_reserve(reserve_account_id);
//!
//! // Forward every on-chain account update you receive from the host.
//! if let Some(update) = trader.on_account(header, body) { /* price changed */ }
//!
//! // Forward SPL token-account events for vault reserve tracking.
//! if let Some(update) = trader.on_token(&tok) { /* reserves changed */ }
//!
//! // Get current price snapshot.
//! if let Some(p) = trader.price(pool_id) { println!("{}", p.price); }
//!
//! // Build and queue a swap instruction.
//! let params = SwapParams { pool: pool_id, input_mint, output_mint, amount_in,
//!                           min_amount_out, user_source_token_account,
//!                           user_destination_token_account, user_wallet };
//! trader.sell(&params, &mut wallet)?;
//! ```

pub mod bundler;
pub mod credit;
pub mod dex;
pub mod perp_router;
pub mod planner;
pub mod pricegraph;
pub mod router;
pub mod spfa;
pub mod types;
