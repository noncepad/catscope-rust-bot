//! Drift/Velocity Protocol perpetual futures -- pricing-only, minimal.
//!
//! Unlike every other `dex::*` module, this is **not** wired into
//! `DexState`/`Updater` -- it exists purely to feed `perp_router::PerpRouter`
//! raw parsed account data. `VelocityState` (below) mirrors how
//! `dex::phoenix::PhoenixState`'s authority-bearing instance is
//! independent of the read-only one `DexState` owns. `dex::drift` (this
//! bot's existing module) only parses Drift's `SpotMarket` (lending)
//! accounts and is unrelated to this one.

pub mod accounts;
pub mod state;

pub use accounts::{parse_perp_market, VelocityPerpMarketView};
pub use state::VelocityState;
