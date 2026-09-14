use serde::Deserialize;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Decodes a base58 pubkey string (the encoding every Go-written JSON file
/// under target/ uses for pubkey/byte32 fields, matching sgo.PublicKey's
/// own MarshalJSON) into a fixed-size array -- the JSON-file counterpart
/// to blob32 above. Extracted from what was previously router.json's own
/// local closure, now shared by every table migrated off direct SQLite
/// access (see this file's other target/*.json readers).
fn bs58_32(s: &str, label: &str) -> [u8; 32] {
    let v = bs58::decode(s)
        .into_vec()
        .unwrap_or_else(|e| panic!("invalid base58 {label} {s:?}: {e}"));
    let len = v.len();
    v.try_into()
        .unwrap_or_else(|_| panic!("{label} {s:?} is not 32 bytes (got {len})"))
}

/// Reads a Go-written JSON file, looking first in `<manifest_dir>/target/`
/// (where `optimizer`'s Prefetcher.Build writes it) and falling back to
/// `<manifest_dir>/` (for local dev running `cargo build` directly against
/// a hand-placed file) -- the same two-location convention router.json
/// already established. Returns None if the file isn't found in either
/// location, so callers can choose their own missing-file behavior (some
/// tables have always-required data, e.g. router.json; others are meant to
/// degrade to empty, matching whatever their old "table not found in
/// prefetch.db" fallback already did).
fn read_target_json_file(manifest_dir: &Path, filename: &str) -> Option<String> {
    println!("cargo:rerun-if-changed=target/{filename}");
    println!("cargo:rerun-if-changed={filename}");
    let mut fp = manifest_dir.join("target").join(filename);
    let mut f = match File::open(&fp) {
        Ok(f) => f,
        Err(_) => {
            fp = manifest_dir.join(filename);
            match File::open(&fp) {
                Ok(f) => f,
                Err(_) => return None,
            }
        }
    };
    let mut s = String::new();
    f.read_to_string(&mut s)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", fp.display()));
    Some(s)
}

/// How many of each dex's top-liquidity pools to embed. Reads TOP_N, falls
/// back to 50 if unset or unparsable.
fn top_n() -> u32 {
    std::env::var("TOP_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

/// Minimum pool liquidity (in the router's raw-anchor-unit "USD", see
/// RouterPoolRaw's doc comment) for a pool's vaults to make it into
/// TRACKED_TOKEN_ACCOUNTS. Reads MIN_LIQUIDITY_USD, falls back to 10_000 if
/// unset or unparsable.
fn min_liquidity_usd() -> f64 {
    std::env::var("MIN_LIQUIDITY_USD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000.0)
}

/// Upper plausibility bound on a single pool's computed `liquidity_usd`.
/// Unlike `usd.is_finite()` below (which only catches literal NaN/inf),
/// this catches a quieter failure mode of the same first-hop-wins anchor-
/// price BFS: a pool on a token with an extreme raw supply/decimals
/// combination can produce a *finite* but economically-impossible dollar
/// figure (observed directly: a single real, live-on-chain Orca pool
/// computing to ~$514 trillion). Verified against a real prefetch.db
/// snapshot that this is a deliberate bimodal split, not a long organic
/// tail: real pools top out in the low hundreds of millions (Orca's own
/// flagship SOL/USDC pool computes to ~$600M here), then pool *count*
/// jumps a full order of magnitude at exactly the $1B mark and keeps
/// climbing into the quadrillions. 10_000_000_000 (10B) sits with ~15x
/// headroom above that known-real reference, comfortably inside the gap.
/// Reads MAX_LIQUIDITY_USD, falls back to 10_000_000_000.0 if unset or
/// unparsable.
fn max_liquidity_usd() -> f64 {
    std::env::var("MAX_LIQUIDITY_USD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000_000.0)
}

/// Minimum computed `liquidity_usd` for a Raydium AMM v4 pool to compete
/// for a ROUTER_POOLS mint-budget slot at all. Raydium AMM's router-graph
/// query has no floor of its own (`WHERE coin_balance > 0 AND pc_balance >
/// 0` admits ~602,000 real prefetch.db rows, most of them tiny/spam), so
/// its sheer candidate count let a flood of low-liquidity pools crowd out
/// every other DEX for the shared budget purely by volume, not quality --
/// confirmed directly: a diagnostic build showed RaydiumAmm claiming 4,667
/// of 5,000 admitted mints while Orca (7,613 real candidate pools of its
/// own) got only 297, and Orca contributed zero live trade-graph edges as
/// a result. Every other DEX's query is comparably small already (CLMM/
/// CPMM/Orca/Sanctum), so this floor is scoped to Raydium AMM specifically
/// for now. Reads RAYDIUM_AMM_ROUTER_MIN_LIQUIDITY_USD, falls back to
/// 10_000.0 (same default as `min_liquidity_usd`, an unrelated floor for a
/// different pass) if unset or unparsable.
fn raydium_amm_router_min_liquidity_usd() -> f64 {
    std::env::var("RAYDIUM_AMM_ROUTER_MIN_LIQUIDITY_USD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000.0)
}

/// Minimum real `liquidity_usd` (see `mint_liquidity_usd`'s doc comment)
/// for a mint to be admitted into TRADE_UNIVERSE via `mint_tracked`.
///
/// Real, live-confirmed gap this replaces (2026-09-05): `mint_tracked`
/// previously only checked `COUNT(*) > 0` against
/// raydium_amm/clmm/cpmm/orca_whirlpool_pool -- "does at least one pool
/// exist for this mint at all", not "does it have any real liquidity".
/// Since Solana lets anyone permissionlessly create a pool for any mint
/// pair (including zero/near-zero-liquidity spam or wash pools), that
/// check admitted plenty of mints with no real tradeable depth --
/// confirmed directly against a real curated-universe candidate
/// (`EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm`): dozens of matching
/// pool rows, every one either near-zero or an implausible placeholder-
/// looking balance, no real liquid market against any standard quote
/// asset. A pair-trading dispersion strategy formerly built against this
/// router sized real legs at roughly $10-20 notional each; its
/// live-observed real slippage-based sizing correctly refused to open
/// with such mints in the candidate set, converging basket size down to
/// single-digit cents (99%+ below intended) on essentially every cycle.
/// $2,000 gives real headroom above a $10-20 leg at the strategy's own
/// 50bps (`MAX_PRICE_IMPACT_BPS`) impact tolerance without requiring the
/// much higher bar (`min_liquidity_usd`'s $10,000 default, `router.rs`'s
/// own $50,000 `MIN_CLUSTER_LIQUIDITY`) those unrelated, larger-notional
/// passes use. Reads TRADE_UNIVERSE_MIN_LIQUIDITY_USD, falls back to
/// 2_000.0 if unset or unparsable.
fn trade_universe_min_liquidity_usd() -> f64 {
    std::env::var("TRADE_UNIVERSE_MIN_LIQUIDITY_USD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000.0)
}

/// Hard ceiling on TRACKED_TOKEN_ACCOUNTS's length -- the wasm bot has to
/// hold and iterate this list at runtime under real memory/compute limits,
/// so unlike MIN_LIQUIDITY_USD (a quality floor) this is a budget that must
/// never be exceeded, not just a preference. Reads TRACKED_ACCOUNTS_BUDGET,
/// falls back to 80_000 (raised from 50_000 alongside the per-dex pool
/// budgets below -- more live pools means more vault/token accounts to
/// track, roughly proportionally) if unset or unparsable.
fn tracked_accounts_budget() -> usize {
    std::env::var("TRACKED_ACCOUNTS_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80_000)
}

/// Hard ceiling on how many *distinct mints* feed ROUTER_POOLS. Unlike
/// TRACKED_ACCOUNTS_BUDGET (a flat list), this bounds `trader::router`'s
/// Tier-2 clustering, which allocates a dense ~448KB `ClusterMatrix` (8
/// buckets of 64x64 f32/u16/AccountId tables) per 64 tokens -- so mint
/// count, not pool count, is what has to stay bounded. 664,559 distinct
/// mints (the full real dataset, unfiltered) works out to >10,000
/// clusters, multiple GB of dense matrices, which is what was crashing
/// the wasm bot's allocator. Reads ROUTER_TOKEN_BUDGET, falls back to
/// 5_000 (~78 clusters, ~35MB) if unset or unparsable.
fn router_token_budget() -> usize {
    std::env::var("ROUTER_TOKEN_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000)
}

/// Hard ceiling on how many Orca pools ORCA_WHIRLPOOL_POOLS embeds.
/// `OrcaState::new` subscribes to every one of these at depth 2 ("pool +
/// corresponding tick arrays") -- each `ParsedTickArray` is itself ~10KB
/// (88 ticks x 113 bytes), and pools vary widely in how many tick arrays
/// they've accumulated over their trading history, so this is a much
/// steeper per-unit memory cost than a flat pubkey list. Unfiltered
/// (140,750 real pools, most of them illiquid/dead) is what overflowed
/// hashbrown's capacity in `OrcaState::m_tick_array` under the old
/// hold-everything-in-memory model -- since fixed by streaming accounts
/// per `Commit` instead, so this cap is raised (2_000 -> 4_000, smaller
/// than the other pool budgets' bump since the per-pool tick-array cost
/// is still real, just no longer an outright overflow) rather than left
/// alone. Reads ORCA_POOL_BUDGET, falls back to 4_000 if unset or
/// unparsable.
fn orca_pool_budget() -> u32 {
    std::env::var("ORCA_POOL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4_000)
}

/// Hard ceiling on how many Raydium AMM pools RAYDIUM_AMM_POOLS embeds,
/// highest-liquidity-first (`ORDER BY coin_balance + pc_balance`).
/// `RaydiumAmm::new` inserts every one of these into `m_pool`/`m_vault`
/// unconditionally at construction time (no lazy fill like Orca's tick
/// arrays -- see DexPoolStats's doc comment in trader::dex) and issues one
/// `bulk_subscribe` request per pool. Unfiltered, a real prefetch.db has
/// ~390,000 pools passing the `market_vault_signer IS NOT NULL` filter --
/// over 2.5x the unfiltered Orca pool count that overflowed hashbrown's
/// capacity (see `orca_pool_budget`) under the old hold-everything model,
/// since fixed by streaming accounts per `Commit` instead. Raised from
/// 2_000 -- a live bot was observed sitting exactly at this cap
/// (`raydium_amm=2000` in `dex pool stats`), meaning liquidity-vetted
/// pools were being cut purely by this budget, not by admission quality.
/// Reads RAYDIUM_AMM_POOL_BUDGET, falls back to 5_000 if unset or
/// unparsable.
fn raydium_amm_pool_budget() -> u32 {
    std::env::var("RAYDIUM_AMM_POOL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000)
}

/// Hard ceiling on how many Raydium CLMM pools RAYDIUM_CLMM_POOLS embeds,
/// highest-liquidity-first (`ORDER BY token0_balance + token1_balance`).
/// Same unconditional-`m_pool`-insert-plus-per-pool-subscription cost as
/// `raydium_amm_pool_budget`. Raised from 2_000 for the same reason as
/// that budget -- a live bot was observed sitting exactly at this cap
/// (`raydium_clmm=2000`) too. Reads RAYDIUM_CLMM_POOL_BUDGET, falls back
/// to 5_000 if unset or unparsable.
fn raydium_clmm_pool_budget() -> u32 {
    std::env::var("RAYDIUM_CLMM_POOL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000)
}

/// Hard ceiling on how many Raydium CPMM pools RAYDIUM_CPMM_POOLS embeds,
/// highest-liquidity-first (`ORDER BY token0_balance + token1_balance`).
/// See `raydium_amm_pool_budget`'s doc comment for why this needs a cap at
/// all. Raised more modestly than AMM/CLMM (2_000 -> 3_000) since a live
/// bot was observed *under* this cap already (`raydium_cpmm=1058`, not
/// 2000) -- fewer admitted CPMM pools exist than the old cap allowed, so
/// there's less headroom to gain here, just some safety margin as the
/// mint-level admission set shifts over time. Reads
/// RAYDIUM_CPMM_POOL_BUDGET, falls back to 3_000 if unset or unparsable.
fn raydium_cpmm_pool_budget() -> u32 {
    std::env::var("RAYDIUM_CPMM_POOL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3_000)
}

/// Hard ceiling on how many PumpSwap pools PUMPSWAP_POOLS embeds,
/// highest-liquidity-first. `PumpswapState::new` inserts every one of
/// these unconditionally at construction (same cost shape as Raydium
/// AMM/CPMM/CLMM, not Orca's lazier tick-array fill). Raised modestly
/// (2_000 -> 3_000, same reasoning as `raydium_cpmm_pool_budget` -- no
/// live-bot evidence yet that this one is actually at its cap the way
/// AMM/CLMM/Orca were, just headroom). Reads PUMPSWAP_POOL_BUDGET, falls
/// back to 3_000 if unset or unparsable.
fn pumpswap_pool_budget() -> u32 {
    std::env::var("PUMPSWAP_POOL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3_000)
}

/// Probe size (in raw lamports, i.e. 1e9 = 1 SOL) `arbv1`'s periodic
/// `trade router check` diagnostic uses to sanity-check that the live
/// `TradeRouter` is finding a real SOL->USDC route. Deliberately a raw
/// lamport integer, not a decimal SOL string -- avoids any float-parsing
/// precision question on either the Go or Rust side of this env var.
/// Lowered from a hardcoded 100 SOL to a configurable default of 0.1 SOL
/// (100_000_000 lamports) after a real live-bot finding: a 100 SOL probe
/// can legitimately crater against a thin-but-real pool's constant-product
/// curve (correct AMM slippage, not a bug), which made 100 SOL a
/// misleading stand-in for "is this route actually tradeable at the size
/// I'd use" -- 0.1 SOL better matches a realistic trade size. Reads
/// TRADE_ROUTER_PROBE_LAMPORTS, falls back to 100_000_000 if unset or
/// unparsable.
fn trade_router_probe_lamports() -> u64 {
    std::env::var("TRADE_ROUTER_PROBE_LAMPORTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000_000)
}

/// Temporary kill switch (2026-09-04): when set, `PHOENIX_MARKETS` is
/// generated empty regardless of what's in prefetch.db's `phoenix_market`
/// table, so no build of the wasm bot has any real Phoenix market to
/// find -- `PhoenixState::markets()` (and everything downstream of it,
/// including testperpv1's perp checks) sees an empty list and refuses
/// rather than attempting a trade. Added after this session's real, live-confirmed
/// incident: the trading wallet's Phoenix Eternal trader account was
/// found frozen on-chain (`TraderCapabilityFlags` denies
/// DepositCollateral/WithdrawCollateral/RiskIncreasingTrade -- see
/// `src/trader/dex/phoenix/accounts.rs`'s `OFF_TH_CAPABILITY_FLAGS` doc
/// comment), with no documented self-service unfreeze path, so every
/// Phoenix-touching cycle was doomed to fail. Reads DISABLE_PHOENIX,
/// treating "1"/"true"/"yes" (case-insensitive) as enabled; unset or any
/// other value leaves Phoenix on, matching every other real env var this
/// file reads. Revert (remove this gate, or set DISABLE_PHOENIX=false)
/// once the trader account is confirmed unfrozen.
fn phoenix_disabled() -> bool {
    std::env::var("DISABLE_PHOENIX")
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Which bundler `Wallet::drain_and_send()` requests from the host
/// (`transactionprocessor::batch`'s own `bundler: option<u8>` parameter)
/// when a single `evaluate()` tick produces more than one transaction --
/// real bundler identity (Jito vs Astralane, etc.) is a host-side
/// concern this Rust code never interprets, just plumbs through. Reads
/// BUNDLER, falls back to 0 if unset or unparsable -- the Go side (see
/// `optimizer/prefetch/rust.go`) sets this to 1 by default when it
/// invokes this build, so 0 here is only ever seen on an ad-hoc local
/// build outside that path.
fn bundler() -> u8 {
    std::env::var("BUNDLER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Shape of `router.json`, written by the Go side's `prefetch/liquidity`
/// package -- scalar config for `trader::router::Router` (not pool data;
/// the pool graph is derived from prefetch.db below).
#[derive(Deserialize)]
struct RouterJsonInput {
    lambda: f32,
    min_cluster_liquidity: f64,
    token_count: usize,
    core_mints: [String; 5],
}

/// Shape of `phoenix_market.json`, written by
/// `optimizer/prefetch/phoenix.ExportJSON` -- one row per tracked Phoenix
/// market, mirroring that package's own `Market` struct's json tags
/// exactly (`market_account`/`symbol`/`asset_id`).
#[derive(Deserialize)]
struct PhoenixMarketJson {
    symbol: String,
    asset_id: u32,
    market_account: String,
}

/// Shape of `kamino_reserve.json` (`optimizer/prefetch/kamino.ExportJSON`)
/// -- mirrors that package's own `Reserve` struct's json tags. `mint` is
/// unused by kamino_data.rs's own codegen but is read by
/// trade_universe_data.rs's Kamino∪Solend mint union below.
#[derive(Deserialize)]
struct KaminoReserveJson {
    pubkey: String,
    lending_market: String,
    mint: String,
    supply_vault: String,
    fee_vault: String,
}

/// Shape of `solend_reserve.json` (`optimizer/prefetch/solend.ExportJSON`).
#[derive(Deserialize)]
struct SolendReserveJson {
    pubkey: String,
    lending_market: String,
    mint: String,
    supply_vault: String,
}

/// Shape of `drift_spot_market.json` (`optimizer/prefetch/drift.ExportJSON`).
#[derive(Deserialize)]
struct DriftSpotMarketJson {
    pubkey: String,
    mint: String,
    vault: String,
}

/// Shape of `jet_reserve.json` (`optimizer/prefetch/jet.ExportJSON`).
#[derive(Deserialize)]
struct JetReserveJson {
    pubkey: String,
    market: String,
    mint: String,
    vault: String,
}

/// Shape of `sanctum_lst.json` (`optimizer/prefetch/sanctum.ExportJSON`).
#[derive(Deserialize)]
struct SanctumLstJson {
    mint: String,
    sol_value_calculator: String,
    sol_value: u64,
    pool_state: String,
    reserve: u64,
}

/// Shape of `marginfi_bank.json` (`optimizer/prefetch/marginfi.ExportJSON`).
#[derive(Deserialize)]
struct MarginfiBankJson {
    pubkey: String,
    group: String,
    mint: String,
    oracle_setup: u8,
    oracle_key: String,
}

/// Shape of `address_lookup_table.json` (`optimizer/prefetch/alt.ExportJSON`).
#[derive(Deserialize)]
struct AddressLookupTableJson {
    table_pubkey: String,
    account_pubkey: String,
}

/// Shape of one `mint_info.json` row (`optimizer/prefetch/mintinfo.ExportJSON`).
#[derive(Deserialize)]
struct MintInfoJson {
    mint: String,
    decimals: u8,
}

/// Shape of one `raydium_amm_pool.json` row (`optimizer/prefetch/raydium/
/// amm.ExportJSON`) -- mirrors that package's own `PoolRow` json tags.
/// Capped to the top `embedCap` highest-liquidity rows on the Go side (see
/// that constant's doc comment); the market_* fields are only present
/// together, and only for rows whose OpenBook market account has been
/// fetched (mirrors the original SQL's nullable market_* columns).
#[derive(Deserialize)]
struct RaydiumAmmPoolJson {
    pubkey: String,
    coin_vault: String,
    pc_vault: String,
    coin_mint: String,
    pc_mint: String,
    coin_balance: i64,
    pc_balance: i64,
    market_bids: Option<String>,
    market_asks: Option<String>,
    market_event_queue: Option<String>,
    market_coin_vault: Option<String>,
    market_pc_vault: Option<String>,
    market_vault_signer: Option<String>,
}

/// Shape of one `raydium_clmm_pool.json` row (`optimizer/prefetch/raydium/
/// clmm.ExportJSON`) -- already joined with its fee config on the Go side.
#[derive(Deserialize)]
struct RaydiumClmmPoolJson {
    pubkey: String,
    token_mint0: String,
    token_mint1: String,
    token_vault0: String,
    token_vault1: String,
    token0_balance: i64,
    token1_balance: i64,
    trade_fee_rate: u32,
}

/// Shape of one `raydium_cpmm_pool.json` row (`optimizer/prefetch/raydium/
/// cpmm.ExportJSON`) -- already joined with its fee config on the Go side.
#[derive(Deserialize)]
struct RaydiumCpmmPoolJson {
    pubkey: String,
    token0_mint: String,
    token1_mint: String,
    token0_vault: String,
    token1_vault: String,
    token0_balance: i64,
    token1_balance: i64,
    trade_fee_rate: u64,
}

/// Shape of one `orca_whirlpool_pool.json` row (`optimizer/prefetch/
/// orca.ExportJSON`) -- exported unfiltered; vault_a_balance/vault_b_balance
/// are the only nullable columns (NULL until the vault account is fetched).
#[derive(Deserialize)]
struct OrcaWhirlpoolPoolJson {
    pubkey: String,
    mint_a: String,
    mint_b: String,
    vault_a: String,
    vault_b: String,
    vault_a_balance: Option<i64>,
    vault_b_balance: Option<i64>,
    sqrt_price_lo: i64,
    sqrt_price_hi: i64,
    liquidity_lo: i64,
    liquidity_hi: i64,
}

/// Shape of one `pumpswap_pool.json` row (`optimizer/prefetch/
/// pumpswap.ExportJSON`) -- exported unfiltered, no nullable columns.
#[derive(Deserialize)]
struct PumpswapPoolJson {
    pool: String,
    base_mint: String,
    quote_mint: String,
    base_vault: String,
    quote_vault: String,
    base_balance: i64,
    quote_balance: i64,
}

/// Shape of one `pumpfun_bonding_curve.json` row (`optimizer/prefetch/
/// pumpfun.ExportJSON`) -- exported unfiltered, no nullable columns.
#[derive(Deserialize)]
struct PumpfunBondingCurveJson {
    mint: String,
    quote_mint: String,
    real_sol_reserves: i64,
    complete: bool,
}

/// Tell cargo at compile time to add trading configuration
/// data to the binary so that the bot in the validator
/// does not have to waste time fetching data.
fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Determine the output path in the build directory (OUT_DIR)
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // Per-mint *real* liquidity (max `liquidity_usd` across every pool
    // touching that mint), populated below alongside ROUTER_POOLS's own
    // per-edge `liquidity_usd` computation -- reused later by the
    // Kamino∪Solend trade-universe filter (`mint_tracked`) instead of
    // that filter's own naive "does any pool exist at all" check. See
    // that filter's own doc comment for the real incident this fixes.
    let mut mint_liquidity_usd: std::collections::HashMap<[u8; 32], f64> =
        std::collections::HashMap::new();

    // Real per-mint decimals, loaded once from mint_info.json and reused
    // by every one of this file's three separate decimals lookups (the
    // router-anchor lookup below, symbol_mint_data.rs's hard-panic
    // lookup, and trade_universe_data.rs's soft-skip lookup) -- avoids
    // re-parsing this file's largest JSON export (hundreds of thousands
    // of rows) three times. Missing file (older prefetch.db predating
    // mint_info, or none ever fetched) degrades to an empty map, same
    // "prepare returning Err -> treat as absent" precedent this file
    // already uses elsewhere -- each of the three call sites keeps its
    // own existing miss-handling policy (soft-empty / hard-panic /
    // soft-skip respectively) on top of this shared map.
    let mint_decimals: std::collections::HashMap<[u8; 32], u8> =
        match read_target_json_file(&manifest_dir, "mint_info.json") {
            Some(json_str) => {
                let rows: Vec<MintInfoJson> =
                    serde_json::from_str(&json_str).expect("failed to parse mint_info.json");
                rows.into_iter()
                    .map(|r| (bs58_32(&r.mint, "mint"), r.decimals))
                    .collect()
            }
            None => {
                println!(
                    "cargo:warning=mint_info.json not found -- re-run download-arb to \
                     populate mint decimals"
                );
                std::collections::HashMap::new()
            }
        };

    // Real per-pool rows for the five DEX pool tables, loaded once here and
    // reused by every one of this file's three consumers of each table
    // (the router-graph BFS just below, each dex's own priority-ordered
    // embed re-query further down, and top_pools_data.rs at the very end)
    // -- avoids three separate JSON round-trips per table. All filtering
    // that used to be a SQL WHERE clause now happens here, in Rust, against
    // these Vecs; see each type's own doc comment for which filter (if
    // any) the Go export already applied vs. what's still done here.
    //
    // raydium_clmm_pool.json/raydium_cpmm_pool.json are required (hard
    // panic if missing) since their own Go export already applies the same
    // filter this file's graph query always assumed -- an old prefetch.db
    // predating this migration simply doesn't exist as a JSON-backed build
    // input anymore. raydium_amm_pool.json is likewise required. Orca and
    // PumpSwap keep this file's pre-existing soft-degrade convention:
    // PumpSwap because pumpswap_pool itself may not exist yet on an older
    // prefetch.db (same as before), Orca because there's no reason to
    // change its established required-ness... actually Orca has always
    // been required (no prior soft-degrade existed for it), so it stays
    // required too.
    let amm_pools: Vec<RaydiumAmmPoolJson> = {
        let json_str = read_target_json_file(&manifest_dir, "raydium_amm_pool.json")
            .expect("failed to open raydium_amm_pool.json (required)");
        serde_json::from_str(&json_str).expect("failed to parse raydium_amm_pool.json")
    };
    let clmm_pools: Vec<RaydiumClmmPoolJson> = {
        let json_str = read_target_json_file(&manifest_dir, "raydium_clmm_pool.json")
            .expect("failed to open raydium_clmm_pool.json (required)");
        serde_json::from_str(&json_str).expect("failed to parse raydium_clmm_pool.json")
    };
    let cpmm_pools: Vec<RaydiumCpmmPoolJson> = {
        let json_str = read_target_json_file(&manifest_dir, "raydium_cpmm_pool.json")
            .expect("failed to open raydium_cpmm_pool.json (required)");
        serde_json::from_str(&json_str).expect("failed to parse raydium_cpmm_pool.json")
    };
    let orca_pools: Vec<OrcaWhirlpoolPoolJson> = {
        let json_str = read_target_json_file(&manifest_dir, "orca_whirlpool_pool.json")
            .expect("failed to open orca_whirlpool_pool.json (required)");
        serde_json::from_str(&json_str).expect("failed to parse orca_whirlpool_pool.json")
    };
    // pumpswap_pool may not exist yet against an older prefetch.db that
    // predates this table -- a missing file degrades to an empty list
    // (with a warning) instead of failing the build, same convention this
    // file always used for PumpSwap.
    let pumpswap_pools: Vec<PumpswapPoolJson> =
        match read_target_json_file(&manifest_dir, "pumpswap_pool.json") {
            Some(json_str) => {
                serde_json::from_str(&json_str).expect("failed to parse pumpswap_pool.json")
            }
            None => {
                println!(
                "cargo:warning=pumpswap_pool.json not found -- re-run download-arb with pumpswap \
                 enabled to populate it; PumpSwap router-pool registration and PUMPSWAP_POOLS \
                 will be empty"
            );
                Vec::new()
            }
        };
    // pumpfun_bonding_curve may not exist yet against an older prefetch.db
    // that predates this table -- same soft-degrade convention as PumpSwap.
    let pumpfun_curves: Vec<PumpfunBondingCurveJson> =
        match read_target_json_file(&manifest_dir, "pumpfun_bonding_curve.json") {
            Some(json_str) => {
                serde_json::from_str(&json_str).expect("failed to parse pumpfun_bonding_curve.json")
            }
            None => {
                println!(
                    "cargo:warning=pumpfun_bonding_curve.json not found -- re-run download-arb \
                     with pumpfun enabled to populate it; skipping Pump.fun router-pool \
                     registration"
                );
                Vec::new()
            }
        };

    // ── router.json + prefetch.db → router_config.rs / router_pools_data.rs ────
    // router.json carries trader::router::Router's scalar config (lambda,
    // min-cluster-liquidity threshold, the 5 core mints); the pool graph
    // used to seed LiquidityPartitioner is derived straight from
    // prefetch.db's raydium/orca reserves via a BFS price walk (see below).
    let (priority_amm, priority_cpmm, priority_clmm, priority_orca, priority_pumpswap) = {
        let json_str = read_target_json_file(&manifest_dir, "router.json")
            .expect("failed to open router.json file (required)");
        let router_input: RouterJsonInput =
            serde_json::from_str(&json_str).expect("failed to parse router.json");

        let core_mints: [[u8; 32]; 5] =
            std::array::from_fn(|i| bs58_32(&router_input.core_mints[i], "core_mints"));

        // ── router_config.rs ─────────────────────────────────────────────
        {
            let mints_lit: String = core_mints
                .iter()
                .map(|m| format!("        {m:?},\n"))
                .collect();
            let code = format!(
                "pub struct RouterConfig {{\n    \
             pub core_mints: [[u8; 32]; 5],\n    \
             pub lambda: f32,\n    \
             pub min_cluster_liquidity: f64,\n    \
             pub token_count: usize,\n\
             }}\n\
             pub static ROUTER_CONFIG: RouterConfig = RouterConfig {{\n    \
             core_mints: [\n{mints_lit}    ],\n    \
             lambda: f32::from_bits({lambda_bits}u32),\n    \
             min_cluster_liquidity: f64::from_bits({mcl_bits}u64),\n    \
             token_count: {token_count},\n\
             }};\n",
                lambda_bits = router_input.lambda.to_bits(),
                mcl_bits = router_input.min_cluster_liquidity.to_bits(),
                token_count = router_input.token_count,
            );
            File::create(out_dir.join("router_config.rs"))
                .expect("failed to create router_config.rs")
                .write_all(code.as_bytes())
                .expect("failed to write router_config.rs");
        }

        // ── router_pools_data.rs: BFS price graph over raydium + orca ────
        let priority_pools = {
            #[derive(Clone, Copy, PartialEq)]
            enum DexKind {
                RaydiumAmm,
                RaydiumCpmm,
                RaydiumClmm,
                Orca,
                Pumpswap,
            }
            struct Edge {
                // The pool account's own pubkey, plus which dex it came
                // from -- lets the priority-pool pass below (see
                // `priority_amm`/etc.) map a router_admitted edge back to
                // a specific row in that dex's own pool table.
                pubkey: [u8; 32],
                dex: DexKind,
                mint_a: [u8; 32],
                mint_b: [u8; 32],
                reserve_a: f64,
                reserve_b: f64,
                // Vaults carried alongside each edge purely so the
                // tracked-accounts pass below can reuse this same graph
                // (and its liquidity_usd) as the source of truth, instead
                // of computing pool liquidity twice. market_vault_a/b are
                // amm-only (the pool's OpenBook market's own vaults).
                vault_a: [u8; 32],
                vault_b: [u8; 32],
                market_vault_a: Option<[u8; 32]>,
                market_vault_b: Option<[u8; 32]>,
            }
            let mut edges: Vec<Edge> = Vec::new();

            // raydium_amm_pool.json's own Go-side filter (coin_balance > 0
            // AND pc_balance > 0 AND coin_mint/pc_mint NOT NULL) already
            // matches this graph's original SQL WHERE clause exactly, so
            // no further filtering is needed here.
            for p in &amm_pools {
                edges.push(Edge {
                    pubkey: bs58_32(&p.pubkey, "pubkey"),
                    dex: DexKind::RaydiumAmm,
                    mint_a: bs58_32(&p.coin_mint, "coin_mint"),
                    mint_b: bs58_32(&p.pc_mint, "pc_mint"),
                    reserve_a: p.coin_balance as f64,
                    reserve_b: p.pc_balance as f64,
                    vault_a: bs58_32(&p.coin_vault, "coin_vault"),
                    vault_b: bs58_32(&p.pc_vault, "pc_vault"),
                    market_vault_a: p
                        .market_coin_vault
                        .as_deref()
                        .map(|s| bs58_32(s, "market_coin_vault")),
                    market_vault_b: p
                        .market_pc_vault
                        .as_deref()
                        .map(|s| bs58_32(s, "market_pc_vault")),
                });
            }
            // raydium_cpmm_pool.json's own Go-side filter (lp_supply > 0
            // AND both balances > 0) already matches this graph's original
            // SQL WHERE clause exactly.
            for p in &cpmm_pools {
                edges.push(Edge {
                    pubkey: bs58_32(&p.pubkey, "pubkey"),
                    dex: DexKind::RaydiumCpmm,
                    mint_a: bs58_32(&p.token0_mint, "token0_mint"),
                    mint_b: bs58_32(&p.token1_mint, "token1_mint"),
                    reserve_a: p.token0_balance as f64,
                    reserve_b: p.token1_balance as f64,
                    vault_a: bs58_32(&p.token0_vault, "token0_vault"),
                    vault_b: bs58_32(&p.token1_vault, "token1_vault"),
                    market_vault_a: None,
                    market_vault_b: None,
                });
            }
            // raydium_clmm_pool.json's own Go-side filter (both balances >
            // 0) already matches this graph's original SQL WHERE clause
            // exactly.
            for p in &clmm_pools {
                edges.push(Edge {
                    pubkey: bs58_32(&p.pubkey, "pubkey"),
                    dex: DexKind::RaydiumClmm,
                    mint_a: bs58_32(&p.token_mint0, "token_mint0"),
                    mint_b: bs58_32(&p.token_mint1, "token_mint1"),
                    reserve_a: p.token0_balance as f64,
                    reserve_b: p.token1_balance as f64,
                    vault_a: bs58_32(&p.token_vault0, "token_vault0"),
                    vault_b: bs58_32(&p.token_vault1, "token_vault1"),
                    market_vault_a: None,
                    market_vault_b: None,
                });
            }
            {
                // Real vault token-account balances, same as every other
                // dex's edges above -- NOT the CLMM liquidity/sqrt_price
                // "virtual reserve" formula this used to use. That formula
                // is mathematically correct for its actual purpose (slippage
                // math at the current price, see OrcaWhirlpool::to_facade's
                // use of it in src/trader/dex/orca.rs) but isn't a valid
                // proxy for a pool's real economic size: virtual reserves
                // grow without bound as a position's price range narrows,
                // independent of how much real capital backs it. Verified
                // directly against live mainnet RPC: a real, current Orca
                // pool computed a virtual reserve of ~10 trillion raw units
                // of a token whose entire on-chain supply is ~1 trillion --
                // 10x more than could physically exist. vault_a_balance/
                // vault_b_balance (populated by optimizer's event.go
                // subscribing to the vault token accounts directly, same as
                // raydium's coin_balance/pc_balance) don't have this failure
                // mode. Coverage is partial while resyncs are ongoing --
                // pools without both balances yet are simply excluded here,
                // same as raydium pools missing balance data, rather than
                // falling back to the invalid virtual-reserve figure.
                //
                // orca_whirlpool_pool.json is exported unfiltered (unlike
                // the raydium tables above), so this filter -- originally a
                // SQL WHERE clause -- is applied here instead.
                for p in &orca_pools {
                    let (Some(bal_a), Some(bal_b)) = (p.vault_a_balance, p.vault_b_balance) else {
                        continue;
                    };
                    if bal_a <= 0 || bal_b <= 0 {
                        continue;
                    }
                    edges.push(Edge {
                        pubkey: bs58_32(&p.pubkey, "pubkey"),
                        dex: DexKind::Orca,
                        mint_a: bs58_32(&p.mint_a, "mint_a"),
                        mint_b: bs58_32(&p.mint_b, "mint_b"),
                        reserve_a: bal_a as f64,
                        reserve_b: bal_b as f64,
                        vault_a: bs58_32(&p.vault_a, "vault_a"),
                        vault_b: bs58_32(&p.vault_b, "vault_b"),
                        market_vault_a: None,
                        market_vault_b: None,
                    });
                }
            }
            {
                // PumpSwap -- a real two-sided AMM with real vault
                // balances (unlike Pump.fun's bonding curve, which lacks
                // real two-sided reserves and so competes in the separate
                // Candidate::Pumpfun side channel below instead). No
                // OpenBook-style separate market, so no market vaults.
                // pumpswap_pools is already empty (with a warning already
                // emitted at load time) if pumpswap_pool.json was missing --
                // same effective behavior as the old "prepare returning Err
                // -> skip PumpSwap entirely" convention, just detected at
                // file-load time instead of query time.
                //
                // pumpswap_pool.json is exported unfiltered, so this
                // filter -- originally a SQL WHERE clause -- is applied
                // here instead.
                for p in &pumpswap_pools {
                    if p.base_balance <= 0 || p.quote_balance <= 0 {
                        continue;
                    }
                    edges.push(Edge {
                        pubkey: bs58_32(&p.pool, "pool"),
                        dex: DexKind::Pumpswap,
                        mint_a: bs58_32(&p.base_mint, "base_mint"),
                        mint_b: bs58_32(&p.quote_mint, "quote_mint"),
                        reserve_a: p.base_balance as f64,
                        reserve_b: p.quote_balance as f64,
                        vault_a: bs58_32(&p.base_vault, "base_vault"),
                        vault_b: bs58_32(&p.quote_vault, "quote_vault"),
                        market_vault_a: None,
                        market_vault_b: None,
                    });
                }
            }
            edges.retain(|e| e.mint_a != e.mint_b);

            let mut adjacency: std::collections::HashMap<[u8; 32], Vec<usize>> =
                std::collections::HashMap::new();
            for (i, e) in edges.iter().enumerate() {
                adjacency.entry(e.mint_a).or_default().push(i);
                adjacency.entry(e.mint_b).or_default().push(i);
            }

            // Widest-path (maximum-bottleneck-capacity) walk from the anchor
            // mint (core_mints[0], USDC) to price every reachable mint in
            // raw anchor-units. This used to be a plain BFS -- first hop
            // found (by edge-table row order, arbitrary) wins, no
            // liquidity-weighted tie-breaking -- which let a single
            // near-empty pool set a token's price if it merely happened to
            // sit fewer hops from the anchor than every real, deep pool for
            // that same token. Confirmed against live mainnet data: a token
            // with a genuine ~$200K-liquidity pool (2 hops via SOL) got
            // priced instead from a 1-hop, dust-level direct-to-USDC pool
            // (19 raw units vs. 579 raw USDC units), inflating that token's
            // implied price by ~8 orders of magnitude and, with it, every
            // other pool's liquidity_usd for that token.
            //
            // Widest path replaces "fewest hops" with "highest-liquidity
            // path wins", Dijkstra-style: each edge's weight is its
            // trusted-side ("from") reserve valued in raw-anchor-units via
            // the already-resolved price of the node being expanded from --
            // the same raw-anchor-unit convention RouterPoolRaw's own
            // liquidity_usd uses below, just computed incrementally instead
            // of after the fact. A path's bottleneck is the minimum edge
            // weight seen along it; the node with the largest remaining
            // bottleneck is always expanded next (a max-heap in place of
            // the old FIFO queue), and -- same as ordinary Dijkstra --
            // finalizing a node the moment it's popped is correct because
            // every edge weight here (a positive reserve amount) is
            // non-negative.
            let anchor = core_mints[0];
            #[derive(Clone, Copy)]
            struct HeapEntry {
                bottleneck: f64,
                mint: [u8; 32],
            }
            impl PartialEq for HeapEntry {
                fn eq(&self, other: &Self) -> bool {
                    self.bottleneck == other.bottleneck
                }
            }
            impl Eq for HeapEntry {}
            impl PartialOrd for HeapEntry {
                fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                    Some(self.cmp(other))
                }
            }
            impl Ord for HeapEntry {
                fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                    // total_cmp so a NaN bottleneck (shouldn't happen given
                    // the is_finite guards below, but not structurally
                    // impossible) sorts as the minimum rather than panicking
                    // BinaryHeap's internal comparisons.
                    self.bottleneck.total_cmp(&other.bottleneck)
                }
            }
            let mut anchor_price: std::collections::HashMap<[u8; 32], f64> =
                std::collections::HashMap::new();
            let mut best_bottleneck: std::collections::HashMap<[u8; 32], f64> =
                std::collections::HashMap::new();
            let mut finalized: std::collections::HashSet<[u8; 32]> =
                std::collections::HashSet::new();
            anchor_price.insert(anchor, 1.0);
            best_bottleneck.insert(anchor, f64::INFINITY);
            let mut heap: std::collections::BinaryHeap<HeapEntry> =
                std::collections::BinaryHeap::new();
            heap.push(HeapEntry {
                bottleneck: f64::INFINITY,
                mint: anchor,
            });
            while let Some(HeapEntry {
                bottleneck,
                mint: m,
            }) = heap.pop()
            {
                if !finalized.insert(m) {
                    // Stale heap entry from before m's best bottleneck was
                    // improved (lazy deletion) -- already processed.
                    continue;
                }
                let price_m = anchor_price[&m];
                let Some(edge_ixs) = adjacency.get(&m) else {
                    continue;
                };
                for &i in edge_ixs {
                    let e = &edges[i];
                    let (to, reserve_from, reserve_to) = if e.mint_a == m {
                        (e.mint_b, e.reserve_a, e.reserve_b)
                    } else {
                        (e.mint_a, e.reserve_b, e.reserve_a)
                    };
                    if finalized.contains(&to) {
                        continue;
                    }
                    let gate = reserve_from * price_m;
                    let candidate_bottleneck = bottleneck.min(gate);
                    if !candidate_bottleneck.is_finite() || candidate_bottleneck <= 0.0 {
                        continue;
                    }
                    let improves = match best_bottleneck.get(&to) {
                        Some(&existing) => candidate_bottleneck > existing,
                        None => true,
                    };
                    if !improves {
                        continue;
                    }
                    let price_to = price_m * (reserve_from / reserve_to);
                    if !price_to.is_finite() || price_to <= 0.0 {
                        continue;
                    }
                    anchor_price.insert(to, price_to);
                    best_bottleneck.insert(to, candidate_bottleneck);
                    heap.push(HeapEntry {
                        bottleneck: candidate_bottleneck,
                        mint: to,
                    });
                }
            }

            // Real per-mint decimals -- see this outer function's own
            // mint_decimals (loaded once from mint_info.json, shared by
            // every decimals lookup in this file, including this one).

            // Only the anchor's own decimals matter for liquidity_usd (every
            // other mint's decimals cancel out along the multiplicative BFS
            // path) -- see the RouterPoolRaw doc comment below. Falls back
            // to the old hardcoded USDC assumption (with a warning) so the
            // build still works against a prefetch.db that predates
            // mint_info.
            let anchor_decimals = mint_decimals.get(&anchor).copied().unwrap_or_else(|| {
                println!(
                    "cargo:warning=no decimals known for anchor mint; assuming 6 (USDC) -- \
                     re-run download-arb to populate mint_info"
                );
                6
            });
            let anchor_correction = 10f64.powi(-(anchor_decimals as i32));

            // Computed once, up front, so both RouterPoolRaw's emission
            // below and the tracked-accounts liquidity filter further down
            // agree on every pool's dollar value instead of risking two
            // independent (and possibly diverging) computations.
            let liquidity_usd: Vec<f64> = edges
                .iter()
                .map(|e| {
                    let val_a = anchor_price
                        .get(&e.mint_a)
                        .map(|p| e.reserve_a * p * anchor_correction);
                    let val_b = anchor_price
                        .get(&e.mint_b)
                        .map(|p| e.reserve_b * p * anchor_correction);
                    let usd = match (val_a, val_b) {
                        (Some(a), Some(b)) => a + b,
                        (Some(a), None) => 2.0 * a,
                        (None, Some(b)) => 2.0 * b,
                        (None, None) => 0.0,
                    };
                    // The anchor-price BFS chains multiplications across
                    // however many hops separate a mint from the anchor --
                    // a handful of real, extreme-ratio pools (long-tail
                    // spam tokens with wildly disproportionate raw-unit
                    // reserves) are enough to overflow a node's price to
                    // inf, and a later inf * (a ratio that underflows to
                    // 0.0) is NaN. Treat non-finite the same as "no anchor
                    // price found": 0.0, i.e. unpriced/untrusted, rather
                    // than embedding NaN/inf into ROUTER_POOLS, where nothing
                    // downstream (Rust's f64::partial_cmp, this file's own
                    // TopPools ORDER BY, etc.) is safe to compare against it.
                    //
                    // Same treatment for a *finite* but implausible value --
                    // see max_liquidity_usd()'s doc comment for why real
                    // pools never legitimately cross this, and why it's
                    // the computed dollar figure that needs the bound, not
                    // the raw on-chain liquidity/reserve fields themselves
                    // (those are genuine chain state, verified against live
                    // RPC -- Solana permits arbitrary raw token supplies,
                    // so there's no analogous sanity bound on them).
                    if usd.is_finite() && usd <= max_liquidity_usd() {
                        usd
                    } else {
                        0.0
                    }
                })
                .collect();

            // Real per-mint liquidity (the best single pool seen for each
            // mint, across every edge) -- a mint can appear in many pools
            // of wildly different real depth (Solana lets anyone
            // permissionlessly create a pool for any mint pair, so most
            // long-tail mints accumulate a handful of near-empty/spam
            // pools alongside at most one or two real ones), so this
            // takes the max rather than a sum or the first match. Feeds
            // `mint_tracked` below.
            for (e, &usd) in edges.iter().zip(&liquidity_usd) {
                for m in [e.mint_a, e.mint_b] {
                    let entry = mint_liquidity_usd.entry(m).or_insert(0.0);
                    if usd > *entry {
                        *entry = usd;
                    }
                }
            }

            // Sanctum LSTs, as synthetic SOL↔LST edges -- extends
            // router::Router's known_mints (and thus TradeRouter's live
            // node set, see TradeRouter::from_router's doc comment) to
            // cover LSTs that have no Raydium/Orca pool of their own,
            // which was the reason `trade router edges by dex` never
            // showed Sanctum contributing anything: without a node for
            // both endpoints, SanctumState::batch_router's
            // add_generic_pair calls were silently skipped. Computed
            // *before* the admission pass below so these compete fairly,
            // by real liquidity, against every Raydium/Orca pool for the
            // shared ROUTER_TOKEN_BUDGET mint budget -- with ~600K+
            // candidate Raydium/Orca pool rows against a default 5,000
            // mint budget, admitting Sanctum only after that budget was
            // already exhausted (an earlier version of this code did
            // exactly that) starved it down to 0 admitted LSTs regardless
            // of how liquid any of them were.
            //
            // Sanctum isn't a two-sided AMM pool (see
            // src/trader/dex/sanctum.rs's module doc) -- there's no
            // per-LST raw-unit reserve in prefetch.db to compute a real
            // price_a_to_b from (the reserve token account balance is
            // only known live, via SanctumState's on_token vault
            // subscription). But `sanctum_lst.sol_value` -- a snapshot of
            // the on-chain LstState's own `sol_value` field -- IS
            // already the total SOL-denominated value of that LST's pool
            // reserves, which is exactly the liquidity figure this
            // router needs; price_a_to_b is left at 0.0 (not meaningful
            // without a raw reserve) since router::Pool's price_a_to_b
            // is never read by LiquidityPartitioner::partition, only
            // liquidity_usd/token_a/token_b are.
            let sol_mint = core_mints[1];
            struct SanctumRouterEdge {
                mint: [u8; 32],
                liquidity_usd: f64,
            }
            let sanctum_candidates: Vec<SanctumRouterEdge> =
                match anchor_price.get(&sol_mint).copied() {
                    Some(sol_price) => {
                        // sanctum_lst.json is re-read here (a second, small
                        // parse -- see tracked_accounts_data.rs's own
                        // add_from_json for the same "small file, cheap to
                        // re-parse" precedent) rather than threading Phase 2's
                        // sanctum_data.rs Vec across this much earlier point in
                        // the file. Required (hard panic if missing), matching
                        // the original SQL prepare's own hard panic on a
                        // missing table.
                        let json_str1 = read_target_json_file(&manifest_dir, "sanctum_lst.json")
                            .expect("failed to open sanctum_lst.json (required)");
                        let rows: Vec<SanctumLstJson> = serde_json::from_str(&json_str1)
                            .expect("failed to parse sanctum_lst.json");
                        rows.into_iter()
                            .filter(|r| r.sol_value > 0)
                            .map(|r| (bs58_32(&r.mint, "mint"), r.sol_value as f64))
                            .filter(|&(mint, _)| mint != sol_mint)
                            .filter_map(|(mint, sol_value)| {
                                let usd = sol_value * sol_price * anchor_correction;
                                (usd.is_finite() && usd > 0.0 && usd <= max_liquidity_usd())
                                    .then_some(SanctumRouterEdge {
                                        mint,
                                        liquidity_usd: usd,
                                    })
                            })
                            .collect()
                    }
                    None => {
                        println!(
                            "cargo:warning=sanctum: SOL has no anchor price from the raydium/orca \
                         router graph -- skipping Sanctum LST router-pool registration"
                        );
                        Vec::new()
                    }
                };

            // Pump.fun bonding curves -- same shape as Sanctum's candidates
            // above (a single self-contained pool account, no separate
            // vault to track), not Raydium/Orca's `edges: Vec<Edge>` model.
            // real_sol_reserves (not virtual) is used for this ranking
            // figure -- real economic size for admission, same reasoning as
            // Orca's vault balances; the Rust dex module's own live pricing
            // uses virtual reserves instead (see pumpfun.rs's module doc).
            struct PumpfunRouterEdge {
                mint: [u8; 32],
                liquidity_usd: f64,
            }
            // Pump.fun uses the all-zero Pubkey::default() as a "native
            // SOL" sentinel for quote_mint, NOT the WSOL mint address --
            // verified against 5 real, live bonding-curve rows fetched
            // this session: every one had quote_mint == 32 zero bytes
            // despite real, nonzero real_sol_reserves/virtual_sol_reserves
            // (a misaligned read would produce garbage, not a clean zero
            // across independent samples). Matches the common Solana
            // convention of distinguishing "native SOL" from the WSOL SPL
            // token via the zero pubkey rather than WSOL's own mint.
            let sol_quote_sentinel: [u8; 32] = [0u8; 32];
            // pumpfun_curves is already empty (with a warning already
            // emitted at load time) if pumpfun_bonding_curve.json was
            // missing -- same effective behavior as the old "prepare
            // returning Err -> empty candidate list" convention, just
            // detected at file-load time instead of query time. The
            // complete/quote_mint/real_sol_reserves filter -- originally a
            // SQL WHERE clause -- is applied here instead.
            let pumpfun_candidates: Vec<PumpfunRouterEdge> =
                match anchor_price.get(&sol_mint).copied() {
                    Some(sol_price) => pumpfun_curves
                        .iter()
                        .filter(|c| !c.complete)
                        .filter_map(|c| {
                            let quote_mint = bs58_32(&c.quote_mint, "quote_mint");
                            (quote_mint == sol_quote_sentinel && c.real_sol_reserves > 0)
                                .then(|| (bs58_32(&c.mint, "mint"), c.real_sol_reserves as f64))
                        })
                        .filter(|&(mint, _)| mint != sol_mint)
                        .filter_map(|(mint, real_sol_reserves)| {
                            let usd = real_sol_reserves * sol_price * anchor_correction;
                            (usd.is_finite() && usd > 0.0 && usd <= max_liquidity_usd()).then_some(
                                PumpfunRouterEdge {
                                    mint,
                                    liquidity_usd: usd,
                                },
                            )
                        })
                        .collect(),
                    None => {
                        println!(
                            "cargo:warning=pumpfun: SOL has no anchor price from the raydium/orca \
                         router graph -- skipping Pump.fun router-pool registration"
                        );
                        Vec::new()
                    }
                };

            // Admit pools and Sanctum LSTs highest-liquidity-first
            // together, tracking distinct mints seen so far, until
            // ROUTER_TOKEN_BUDGET would be exceeded -- see
            // router_token_budget() for why mint count (not pool count)
            // is the thing that has to stay bounded here. A pool's two
            // mints are admitted atomically (both or neither), same
            // reasoning as tracked_accounts_data.rs's per-pool vault
            // admission below; a Sanctum LST only ever adds one new mint
            // since its SOL side is already a registered core mint.
            let router_budget = router_token_budget();
            enum Candidate {
                Pool(usize),
                Sanctum(usize),
                Pumpfun(usize),
            }
            let candidate_liquidity = |c: &Candidate| match *c {
                Candidate::Pool(i) => liquidity_usd[i],
                Candidate::Sanctum(i) => sanctum_candidates[i].liquidity_usd,
                Candidate::Pumpfun(i) => pumpfun_candidates[i].liquidity_usd,
            };
            // See raydium_amm_router_min_liquidity_usd's doc comment --
            // only Raydium AMM candidates are floored; every other DEX's
            // query is already small enough not to need one.
            let raydium_amm_floor = raydium_amm_router_min_liquidity_usd();
            let mut router_order: Vec<Candidate> = (0..edges.len())
                .filter(|&i| {
                    edges[i].dex != DexKind::RaydiumAmm || raydium_amm_floor <= liquidity_usd[i]
                })
                .map(Candidate::Pool)
                .chain((0..sanctum_candidates.len()).map(Candidate::Sanctum))
                .chain((0..pumpfun_candidates.len()).map(Candidate::Pumpfun))
                .collect();
            router_order.sort_by(|a, b| {
                candidate_liquidity(b)
                    .partial_cmp(&candidate_liquidity(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut router_mints: std::collections::BTreeSet<[u8; 32]> =
                std::collections::BTreeSet::new();
            let mut router_admitted = vec![false; edges.len()];
            let mut sanctum_admitted: Vec<&SanctumRouterEdge> = Vec::new();
            let mut pumpfun_admitted: Vec<&PumpfunRouterEdge> = Vec::new();
            for c in router_order {
                match c {
                    Candidate::Pool(i) => {
                        let e = &edges[i];
                        let new_a = !router_mints.contains(&e.mint_a);
                        let new_b = !router_mints.contains(&e.mint_b);
                        let would_be = router_mints.len() + new_a as usize + new_b as usize;
                        if router_budget < would_be {
                            break;
                        }
                        router_mints.insert(e.mint_a);
                        router_mints.insert(e.mint_b);
                        router_admitted[i] = true;
                    }
                    Candidate::Sanctum(i) => {
                        let e = &sanctum_candidates[i];
                        let would_be =
                            router_mints.len() + !router_mints.contains(&e.mint) as usize;
                        if router_budget < would_be {
                            break;
                        }
                        router_mints.insert(e.mint);
                        sanctum_admitted.push(e);
                    }
                    Candidate::Pumpfun(i) => {
                        let e = &pumpfun_candidates[i];
                        let would_be =
                            router_mints.len() + !router_mints.contains(&e.mint) as usize;
                        if router_budget < would_be {
                            break;
                        }
                        router_mints.insert(e.mint);
                        pumpfun_admitted.push(e);
                    }
                }
            }
            if sanctum_admitted.len() < sanctum_candidates.len() {
                println!(
                    "cargo:warning=sanctum: router token budget only had room for {}/{} LSTs \
                     in ROUTER_POOLS",
                    sanctum_admitted.len(),
                    sanctum_candidates.len(),
                );
            }
            if pumpfun_admitted.len() < pumpfun_candidates.len() {
                println!(
                    "cargo:warning=pumpfun: router token budget only had room for {}/{} bonding \
                     curves in ROUTER_POOLS",
                    pumpfun_admitted.len(),
                    pumpfun_candidates.len(),
                );
            }
            // TEMP diagnostic: which DEX actually won the shared mint
            // budget. Sanctum's admission was previously getting starved
            // to 0/91 by sheer Raydium AMM candidate-pool volume even
            // though it was competing in the same pass as everything
            // else -- this checks whether Orca is suffering the same
            // fate, since it too shows 0 live edges after 30+ minutes of
            // runtime despite deep vault-balance activity.
            {
                let mut n_amm = 0usize;
                let mut n_cpmm = 0usize;
                let mut n_clmm = 0usize;
                let mut n_orca = 0usize;
                let mut n_pumpswap = 0usize;
                for (e, &admitted) in edges.iter().zip(&router_admitted) {
                    if !admitted {
                        continue;
                    }
                    match e.dex {
                        DexKind::RaydiumAmm => n_amm += 1,
                        DexKind::RaydiumCpmm => n_cpmm += 1,
                        DexKind::RaydiumClmm => n_clmm += 1,
                        DexKind::Orca => n_orca += 1,
                        DexKind::Pumpswap => n_pumpswap += 1,
                    }
                }
                println!(
                    "cargo:warning=router_pools admitted pools by dex: RaydiumAmm={n_amm} \
                     RaydiumCpmm={n_cpmm} RaydiumClmm={n_clmm} Orca={n_orca} \
                     Pumpswap={n_pumpswap} Sanctum={} Pumpfun={} / total_mints={}",
                    sanctum_admitted.len(),
                    pumpfun_admitted.len(),
                    router_mints.len(),
                );
            }

            // Per-dex pool pubkeys this widest-path liquidity analysis
            // actually admitted -- fed back into the live pool-subscription
            // queries below (RAYDIUM_AMM_POOLS/RAYDIUM_CLMM_POOLS/
            // RAYDIUM_CPMM_POOLS/ORCA_WHIRLPOOL_POOLS) so the bot actually
            // watches these pools live, not just a raw-balance-ranked set
            // that (verified against a real prefetch.db) can miss even the
            // canonical SOL/USDC pool entirely: raw coin_balance+pc_balance
            // has no price/decimals normalization, so a spam token with a
            // huge raw supply outranks genuinely deep, real pools.
            //
            // Ordered by real liquidity_usd (descending) and capped at each
            // dex's own embed budget -- router_admitted alone can hold far
            // more pools per dex than that budget (thousands, vs. a 2000
            // pool cap), and without this cap+order the subscription
            // query's raw-balance secondary sort was free to bump a
            // genuinely-liquid priority pool past the LIMIT in favor of
            // another priority pool with a merely larger raw balance sum --
            // verified directly: it silently dropped the very SOL/USDC pool
            // this fix exists for.
            let mut admitted_by_liquidity: Vec<usize> =
                (0..edges.len()).filter(|&i| router_admitted[i]).collect();
            admitted_by_liquidity.sort_by(|&i, &j| {
                liquidity_usd[j]
                    .partial_cmp(&liquidity_usd[i])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let (amm_budget, cpmm_budget, clmm_budget, orca_budget, pumpswap_budget) = (
                raydium_amm_pool_budget() as usize,
                raydium_cpmm_pool_budget() as usize,
                raydium_clmm_pool_budget() as usize,
                orca_pool_budget() as usize,
                pumpswap_pool_budget() as usize,
            );
            let mut priority_amm: Vec<[u8; 32]> = Vec::new();
            let mut priority_cpmm: Vec<[u8; 32]> = Vec::new();
            let mut priority_clmm: Vec<[u8; 32]> = Vec::new();
            let mut priority_orca: Vec<[u8; 32]> = Vec::new();
            let mut priority_pumpswap: Vec<[u8; 32]> = Vec::new();
            for &i in &admitted_by_liquidity {
                let e = &edges[i];
                match e.dex {
                    DexKind::RaydiumAmm if priority_amm.len() < amm_budget => {
                        priority_amm.push(e.pubkey)
                    }
                    DexKind::RaydiumCpmm if priority_cpmm.len() < cpmm_budget => {
                        priority_cpmm.push(e.pubkey)
                    }
                    DexKind::RaydiumClmm if priority_clmm.len() < clmm_budget => {
                        priority_clmm.push(e.pubkey)
                    }
                    DexKind::Orca if priority_orca.len() < orca_budget => {
                        priority_orca.push(e.pubkey)
                    }
                    DexKind::Pumpswap if priority_pumpswap.len() < pumpswap_budget => {
                        priority_pumpswap.push(e.pubkey)
                    }
                    _ => {}
                }
            }

            let mut code = String::from(
                "pub struct RouterPoolRaw {\n    \
             pub mint_a: [u8; 32],\n    \
             pub mint_b: [u8; 32],\n    \
             /// Pool liquidity valued in raw anchor-mint (USDC) units via a\n    \
             /// BFS price graph over raydium+orca reserves -- NOT real USD\n    \
             /// (no decimals normalization beyond the anchor's own decimals,\n    \
             /// no price oracle -- USDC is assumed pegged to $1). Used only\n    \
             /// to make router.rs's $50,000 MIN_CLUSTER_LIQUIDITY threshold\n    \
             /// meaningful.\n    \
             pub liquidity_usd: f64,\n    \
             /// Raw-unit ratio (mint_b raw units per mint_a raw unit) --\n    \
             /// meaningless without knowing both mints' decimals.\n    \
             pub price_a_to_b: f64,\n    \
             /// Decimals-normalized whole-token price (mint_b per mint_a),\n    \
             /// when both mints' decimals are known; None otherwise.\n    \
             pub price_a_to_b_normalized: Option<f64>,\n\
             }\n\
             pub static ROUTER_POOLS: &[RouterPoolRaw] = &[\n",
            );
            for ((e, &liquidity_usd1), &admitted) in
                edges.iter().zip(&liquidity_usd).zip(&router_admitted)
            {
                if !admitted {
                    continue;
                }
                let price_a_to_b = e.reserve_b / e.reserve_a;
                let price_a_to_b_normalized_lit =
                    match (mint_decimals.get(&e.mint_a), mint_decimals.get(&e.mint_b)) {
                        (Some(&da), Some(&db)) => {
                            let normalized =
                                price_a_to_b * 10f64.powi(da as i32) / 10f64.powi(db as i32);
                            format!("Some(f64::from_bits({}u64))", normalized.to_bits())
                        }
                        _ => "None".to_string(),
                    };
                code.push_str(&format!(
                    "    RouterPoolRaw {{ mint_a: {mint_a:?}, mint_b: {mint_b:?}, \
                 liquidity_usd: f64::from_bits({liq_bits}u64), \
                 price_a_to_b: f64::from_bits({price_bits}u64), \
                 price_a_to_b_normalized: {price_a_to_b_normalized_lit} }},\n",
                    mint_a = e.mint_a,
                    mint_b = e.mint_b,
                    liq_bits = liquidity_usd1.to_bits(),
                    price_bits = price_a_to_b.to_bits(),
                ));
            }
            for e in &sanctum_admitted {
                code.push_str(&format!(
                    "    RouterPoolRaw {{ mint_a: {mint_a:?}, mint_b: {mint_b:?}, \
                 liquidity_usd: f64::from_bits({liq_bits}u64), \
                 price_a_to_b: 0.0, \
                 price_a_to_b_normalized: None }},\n",
                    mint_a = sol_mint,
                    mint_b = e.mint,
                    liq_bits = e.liquidity_usd.to_bits(),
                ));
            }
            // price_a_to_b: 0.0, same as Sanctum's emission above -- not
            // meaningful from this static ranking snapshot (virtual
            // reserves aren't queried here), live pricing comes from
            // pumpfun.rs's on_account instead.
            for e in &pumpfun_admitted {
                code.push_str(&format!(
                    "    RouterPoolRaw {{ mint_a: {mint_a:?}, mint_b: {mint_b:?}, \
                 liquidity_usd: f64::from_bits({liq_bits}u64), \
                 price_a_to_b: 0.0, \
                 price_a_to_b_normalized: None }},\n",
                    mint_a = sol_mint,
                    mint_b = e.mint,
                    liq_bits = e.liquidity_usd.to_bits(),
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("router_pools_data.rs"))
                .expect("failed to create router_pools_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write router_pools_data.rs");

            // ── prefetch db (pumpfun_bonding_curve, admitted mints only) →
            // pumpfun_data.rs ────────────────────────────────────────────
            // Reuses pumpfun_admitted directly (already budget-capped by
            // router_token_budget() above) rather than a separate ranked
            // query + its own budget function -- unlike Orca (which needs
            // its own stricter tick-array-memory budget), Pump.fun's only
            // real per-account cost is one ~115-byte account, the same
            // order of magnitude as a Sanctum LST, so no extra cap is
            // needed beyond the shared router-mint budget.
            {
                let mut pf_code = String::from(
                    "pub struct PumpfunBondingCurveRaw {\n    \
                     pub mint: [u8; 32],\n\
                     }\n\
                     pub static PUMPFUN_BONDING_CURVES: &[PumpfunBondingCurveRaw] = &[\n",
                );
                for e in &pumpfun_admitted {
                    pf_code.push_str(&format!(
                        "    PumpfunBondingCurveRaw {{ mint: {mint:?} }},\n",
                        mint = e.mint,
                    ));
                }
                pf_code.push_str("];\n");
                File::create(out_dir.join("pumpfun_data.rs"))
                    .expect("failed to create pumpfun_data.rs")
                    .write_all(pf_code.as_bytes())
                    .expect("failed to write pumpfun_data.rs");
            }

            // ── tracked_accounts_data.rs: vaults of the highest-liquidity ──
            // pools, capped to a fixed account-count budget. The wasm bot
            // holds and iterates this list at runtime under real
            // memory/compute limits, so TRACKED_ACCOUNTS_BUDGET (see
            // tracked_accounts_budget()) is a hard ceiling that must never
            // be exceeded -- unlike MIN_LIQUIDITY_USD, which is only a
            // quality floor. Reuses this same BFS price graph (edges +
            // liquidity_usd) as the source of truth for ranking, rather
            // than a second, possibly-diverging computation.
            //
            // marginfi_bank and sanctum_lst aren't included: neither table
            // stores a vault column today (a known gap, not an oversight --
            // see the Go side's schema.sql). kamino_reserve/solend_reserve/
            // drift_spot_market/jet_reserve aren't part of this swap-pair
            // price graph at all (they're lending reserves, not AMM pools),
            // so there's no liquidity figure to rank them by -- their
            // vaults are added first, unconditionally (a few hundred rows
            // combined across all four tables), still counted against the
            // budget like everything else.
            {
                let budget = tracked_accounts_budget();
                let floor = min_liquidity_usd();
                let mut accounts: std::collections::BTreeSet<[u8; 32]> =
                    std::collections::BTreeSet::new();

                // Each of these four tables' own dedicated *_data.rs block
                // (further down in this file) re-reads and re-parses the
                // same JSON file independently -- these files are small
                // (a few hundred KB at most), so a second parse here is
                // cheap and far lower-risk than threading one shared,
                // already-parsed Vec across two widely separated blocks
                // of this file.
                let mut add_from_json =
                    |filename: &str, extract: &dyn Fn(&str) -> Vec<[u8; 32]>| {
                        let json_str1 = read_target_json_file(&manifest_dir, filename)
                            .unwrap_or_else(|| panic!("failed to open {filename} (required)"));
                        for v in extract(&json_str1) {
                            accounts.insert(v);
                        }
                    };
                add_from_json("kamino_reserve.json", &|s| {
                    let reserves: Vec<KaminoReserveJson> =
                        serde_json::from_str(s).expect("failed to parse kamino_reserve.json");
                    reserves
                        .iter()
                        .map(|r| bs58_32(&r.supply_vault, "supply_vault"))
                        .collect()
                });
                add_from_json("kamino_reserve.json", &|s| {
                    let reserves: Vec<KaminoReserveJson> =
                        serde_json::from_str(s).expect("failed to parse kamino_reserve.json");
                    reserves
                        .iter()
                        .map(|r| bs58_32(&r.fee_vault, "fee_vault"))
                        .collect()
                });
                add_from_json("solend_reserve.json", &|s| {
                    let reserves: Vec<SolendReserveJson> =
                        serde_json::from_str(s).expect("failed to parse solend_reserve.json");
                    reserves
                        .iter()
                        .map(|r| bs58_32(&r.supply_vault, "supply_vault"))
                        .collect()
                });
                add_from_json("drift_spot_market.json", &|s| {
                    let markets: Vec<DriftSpotMarketJson> =
                        serde_json::from_str(s).expect("failed to parse drift_spot_market.json");
                    markets.iter().map(|m| bs58_32(&m.vault, "vault")).collect()
                });
                add_from_json("jet_reserve.json", &|s| {
                    let reserves: Vec<JetReserveJson> =
                        serde_json::from_str(s).expect("failed to parse jet_reserve.json");
                    reserves
                        .iter()
                        .map(|r| bs58_32(&r.vault, "vault"))
                        .collect()
                });

                // Highest-liquidity pools first, stopping at whichever
                // limit is hit first: the account budget, or the liquidity
                // floor. The budget check runs before each pool's vaults
                // are added (atomically, all or none), so the final count
                // can land a few accounts past budget -- at most one
                // pool's worth (up to 4: two vaults plus amm's optional
                // market vaults) -- never a lot over.
                let mut order: Vec<usize> = (0..edges.len()).collect();
                order.sort_by(|&i, &j| {
                    liquidity_usd[j]
                        .partial_cmp(&liquidity_usd[i])
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for i in order {
                    if budget <= accounts.len() || liquidity_usd[i] < floor {
                        break;
                    }
                    let e = &edges[i];
                    accounts.insert(e.vault_a);
                    accounts.insert(e.vault_b);
                    if let Some(v) = e.market_vault_a {
                        accounts.insert(v);
                    }
                    if let Some(v) = e.market_vault_b {
                        accounts.insert(v);
                    }
                }

                let mut code1 =
                    String::from("pub static TRACKED_TOKEN_ACCOUNTS: &[[u8; 32]] = &[\n");
                for pk in &accounts {
                    code1.push_str(&format!("    {pk:?},\n"));
                }
                code1.push_str("];\n");
                File::create(out_dir.join("tracked_accounts_data.rs"))
                    .expect("failed to create tracked_accounts_data.rs")
                    .write_all(code1.as_bytes())
                    .expect("failed to write tracked_accounts_data.rs");
            }

            (
                priority_amm,
                priority_cpmm,
                priority_clmm,
                priority_orca,
                priority_pumpswap,
            )
        };
        priority_pools
    };

    // ── phoenix_market.json → phoenix_data.rs ────────────────────────────────
    // Migrated off the earlier phoenix.json-only path (a deliberate scope
    // reduction at the time, since Phoenix has a small fixed market count
    // unlike Raydium/Orca's thousands of pools) onto the same prefetch.db
    // pipeline every other dex uses, so market discovery is
    // live/refreshable via download-arb instead of a static hand-curated
    // snapshot -- see optimizer/prefetch/phoenix for the Go-side discovery
    // (a two-hop depth-1 cascade: GlobalConfiguration -> PerpAssetMap, no
    // edge-generator FilterEdges needed at all, unlike every other
    // prefetch package). As of this file's SQLite migration,
    // optimizer/prefetch/phoenix.ExportJSON writes phoenix_market.json
    // directly from prefetch.db (the same query this block used to run
    // itself); this block only reads that file now, never opens
    // prefetch.db.
    {
        let mut code = String::from(
            "pub struct PhoenixMarketRaw {\n    \
             pub symbol: [u8; 16],\n    \
             pub asset_id: u32,\n    \
             pub market_account: [u8; 32],\n\
             }\n\
             pub static PHOENIX_MARKETS: &[PhoenixMarketRaw] = &[\n",
        );
        let disable_phoenix = phoenix_disabled();
        if disable_phoenix {
            println!(
                "cargo:warning=DISABLE_PHOENIX is set -- PHOENIX_MARKETS will be empty regardless \
                 of phoenix_market.json"
            );
        }
        // Missing phoenix_market.json (e.g. local `cargo build` without
        // ever running `optimizer download-arb`) degrades to an empty
        // list with a warning, same convention this file's other
        // pumpswap/pumpfun fallbacks already use -- there is no "table
        // not found" case to consider anymore (optimizer's store.DB
        // always migrates this table before ExportJSON runs), only
        // "file not found".
        match (
            disable_phoenix,
            read_target_json_file(&manifest_dir, "phoenix_market.json"),
        ) {
            (true, _) => {}
            (false, Some(json_str)) => {
                let markets: Vec<PhoenixMarketJson> =
                    serde_json::from_str(&json_str).expect("failed to parse phoenix_market.json");
                for m in markets {
                    assert!(
                        m.symbol.len() <= 16,
                        "phoenix_market symbol {:?} longer than 16 bytes",
                        m.symbol
                    );
                    let mut symbol_arr = [0u8; 16];
                    symbol_arr[..m.symbol.len()].copy_from_slice(m.symbol.as_bytes());
                    let market_account = bs58_32(&m.market_account, "market_account");
                    code.push_str(&format!(
                        "    PhoenixMarketRaw {{ symbol: {symbol_arr:?}, asset_id: {}, \
                         market_account: {market_account:?} }},\n",
                        m.asset_id,
                    ));
                }
            }
            (false, None) => {
                println!(
                    "cargo:warning=phoenix_market.json not found -- re-run download-arb \
                     with phoenix enabled to populate it; PHOENIX_MARKETS will be empty"
                );
            }
        }
        code.push_str("];\n");
        File::create(out_dir.join("phoenix_data.rs"))
            .expect("failed to create phoenix_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write phoenix_data.rs");
    }

    // ── curated symbol -> mint join key → symbol_mint_data.rs ────────────────
    // Hand-curated, not database-derived: prefetch.db has no symbol/ticker
    // data anywhere (confirmed -- mint_info is just mint+decimals, no pool
    // table carries a ticker), so there's no way to mechanically derive
    // "this mint is BTC" the way ROUTER_POOLS/PHOENIX_MARKETS are derived.
    // Scoped to symbols with confirmed real perp coverage on both Phoenix
    // and Velocity (see `trader::dex::velocity::state::TRACKED_MARKETS`).
    // Each mint below was verified (this session) via Jupiter's live token
    // search for a genuinely liquid wrapped/bridged representation, then
    // cross-checked to actually appear in raydium_amm_pool/raydium_clmm_pool/
    // raydium_cpmm_pool/orca_whirlpool_pool with real tracked pools --
    // matches this build's own top-liquidity mint universe, not just an
    // external source trusted blindly. DOGE has no real wrapped Solana
    // representation with meaningful liquidity (checked multiple query
    // variants; only memecoin noise under a few thousand dollars came back)
    // and is deliberately omitted -- an absent entry here correctly
    // degrades to `base_mint: None` at the callsite (still fine for pure
    // inter-venue funding capture, just not spot-hedgeable).
    {
        const CURATED_SYMBOL_MINTS: &[(&str, &str)] = &[
            ("SOL", "So11111111111111111111111111111111111111112"),
            ("BTC", "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh"), // Wrapped BTC (Portal)
            ("ETH", "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"), // Ether (Portal)
            ("XRP", "6UpQcMAb5xMzxc7ZfPaVMgx3KqsvKZdT5U718BzD5We2"), // Wrapped XRP
            ("BNB", "9gP2kCy3wA1ctvYWQk75guqXuHfrEomqydHLtcTCqiLa"), // Binance Coin (Portal)
            ("SUI", "suifhC9gU1VbJAPYPTBkHJyyyStKGLLYPVDTmPoqbvA"),
        ];

        let decode_b58 = |s: &str, label: &str| -> [u8; 32] {
            let v = bs58::decode(s)
                .into_vec()
                .unwrap_or_else(|e| panic!("invalid base58 {label} {s:?}: {e}"));
            assert_eq!(v.len(), 32, "{label} {s:?} is not 32 bytes");
            v.try_into().unwrap()
        };

        // mint_liquidity_usd (populated by the router-graph BFS above from
        // the same 5 pool tables' real, filtered edges) already answers
        // "does this mint appear in any real, positive-balance pool" --
        // reusing it here instead of 4 fresh SQL COUNT queries actually
        // tightens this check slightly (real liquidity present, not just
        // any degenerate row mentioning the mint), which only matters for
        // this cosmetic build warning, not for anything load-bearing.
        let mint_tracked = |mint: &[u8; 32]| -> bool { mint_liquidity_usd.contains_key(mint) };
        // Real per-mint decimals for the curated set, from the same
        // mint_info table the router-anchor lookup above already reads.
        // Unlike that lookup's soft warning+fallback (a router-internal
        // valuation heuristic, low stakes), a missing decimals value
        // here is a hard build failure: decimals feed directly into
        // real trade sizing (testperpv1's rebalance math), and a
        // fabricated/guessed value would silently corrupt it -- same
        // discipline as Phoenix's mark_price_usd() returning `None`
        // rather than a guess. Verified (this session) all 6 curated
        // mints have a real mint_info row today, so this should never
        // actually fire.
        let lookup_decimals = |symbol: &str, mint: &[u8; 32]| -> u8 {
            *mint_decimals.get(mint).unwrap_or_else(|| {
                panic!(
                    "curated symbol_mint entry {symbol} has no mint_info decimals row -- \
                     re-run download-arb to populate mint_info before building"
                )
            })
        };

        let mut code = String::from(
            "pub struct SymbolMintRaw {\n    \
             pub symbol: [u8; 16],\n    \
             pub mint: [u8; 32],\n    \
             pub decimals: u8,\n\
             }\n\
             pub static SYMBOL_MINT_MAP: &[SymbolMintRaw] = &[\n",
        );
        for &(symbol, mint_str) in CURATED_SYMBOL_MINTS {
            let mint = decode_b58(mint_str, symbol);
            if !mint_tracked(&mint) {
                println!(
                    "cargo:warning=curated symbol_mint entry {symbol} ({mint_str}) not found in \
                     any tracked pool table -- check it's still a real, liquid mint"
                );
            }
            let decimals = lookup_decimals(symbol, &mint);
            assert!(
                symbol.len() <= 16,
                "symbol_mint symbol {symbol:?} longer than 16 bytes"
            );
            let mut symbol_arr = [0u8; 16];
            symbol_arr[..symbol.len()].copy_from_slice(symbol.as_bytes());
            code.push_str(&format!(
                "    SymbolMintRaw {{ symbol: {symbol_arr:?}, mint: {mint:?}, decimals: {decimals} }},\n",
            ));
        }
        code.push_str("];\n");
        File::create(out_dir.join("symbol_mint_data.rs"))
            .expect("failed to create symbol_mint_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write symbol_mint_data.rs");
    }

    // ── Kamino ∪ Solend reserve mints (all markets) → trade_universe_data.rs ─
    // Originally a pair-trading candidate universe, broadened from the
    // tiny Phoenix+Velocity-perp-gated `SYMBOL_MINT_MAP` above. That gate was
    // never actually load-bearing for this strategy: the pair trade's
    // legs are real Kamino *or* Solend deposit/borrow (protocol picked
    // per-leg at runtime, see `LendingProtocol`/`best_supply_apy`/
    // `best_borrow_apy` in `state.rs`), no perp hedge, so perp coverage
    // was just an accident of reusing another mode's curated list.
    // Union of both protocols' own real reserve mints, across *every*
    // lending market each protocol has (not just the designated
    // "main market") -- widened from a main-market-only union (111
    // mints) after live testing kept hitting "structurally isolated, no
    // real hedge basket buildable" for real, liquid tickers (BTC), since
    // main-market-only left too few real candidates for the directional
    // trade's factor-basket builder to find a good per-factor proxy from.
    // Kamino/Solend isolated markets are permissionless (anyone can list
    // anything), so unlike the main-market union, this can't trust "has a
    // reserve" alone as a liquidity signal -- each candidate mint is
    // additionally required to appear in a real tracked Raydium/Orca pool
    // (`mint_tracked` below, same real-liquidity check `CURATED_SYMBOL_
    // MINTS` above already uses) before being included, so spam/rug
    // isolated-market listings with no real spot liquidity are filtered
    // out rather than silently expanding the universe with noise.
    // Deduped by mint before the decimals join -- a mint covered by both
    // protocols (or multiple markets within one protocol) only needs one
    // row (which protocol(s)/market(s) actually cover a given mint is
    // answered live, via `dex.kamino()`/`dex.solend()`'s own
    // `reserve_by_mint`, not baked into this static table). Real decimals
    // from `mint_info`; unlike `SYMBOL_MINT_MAP`, this list is
    // broad/best-effort by construction (not hand-verified per entry),
    // so a mint with no `mint_info` decimals row is soft-skipped with a
    // build warning rather than a hard failure. No real ticker data
    // exists for most of these mints (same `prefetch.db` gap noted
    // above), so anything not already in `CURATED_SYMBOL_MINTS` gets a
    // real (not fabricated) label: its own truncated base58 mint address.
    {
        // Real per-mint liquidity, not a bare "does a pool exist" check --
        // see `mint_liquidity_usd`'s and `trade_universe_min_liquidity_usd`'s
        // own doc comments for the real incident this fixes (a bare
        // existence check let plenty of no-real-liquidity mints into
        // TRADE_UNIVERSE, since anyone can permissionlessly create a
        // spam/wash pool for any mint pair).
        let universe_liquidity_floor = trade_universe_min_liquidity_usd();
        let mint_tracked = |mint: &[u8; 32]| -> bool {
            mint_liquidity_usd.get(mint).copied().unwrap_or(0.0) >= universe_liquidity_floor
        };
        // Real hand-verified tickers stay attached to their real mints
        // (same source of truth as `CURATED_SYMBOL_MINTS` above) --
        // avoids regressing SOL/BTC/ETH's existing log labels to a
        // truncated address just because this list now generates them
        // mechanically.
        let known_tickers: &[(&str, &str)] = &[
            ("SOL", "So11111111111111111111111111111111111111112"),
            ("BTC", "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh"),
            ("ETH", "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"),
            ("XRP", "6UpQcMAb5xMzxc7ZfPaVMgx3KqsvKZdT5U718BzD5We2"),
            ("BNB", "9gP2kCy3wA1ctvYWQk75guqXuHfrEomqydHLtcTCqiLa"),
            ("SUI", "suifhC9gU1VbJAPYPTBkHJyyyStKGLLYPVDTmPoqbvA"),
        ];
        let known_ticker_for = |mint: &[u8; 32]| -> Option<&'static str> {
            known_tickers
                .iter()
                .find(|(_, m)| {
                    let v = bs58::decode(m)
                        .into_vec()
                        .unwrap_or_else(|e| panic!("invalid base58 known_ticker {m:?}: {e}"));
                    assert_eq!(v.len(), 32, "known_ticker {m:?} is not 32 bytes");
                    v.as_slice() == mint
                })
                .map(|(sym, _)| *sym)
        };

        // kamino_reserve.json/solend_reserve.json are re-read here (a
        // second, small parse -- same "cheap, low-risk" precedent as
        // tracked_accounts_data.rs's add_from_json) rather than threading
        // kamino_data.rs's/solend_data.rs's own Vecs across this much
        // earlier point in the file. DISTINCT + ORDER BY mint is
        // reproduced via sort+dedup below (matching the original SQL
        // exactly, just done in Rust).
        let distinct_sorted = |mut mints: Vec<[u8; 32]>| -> Vec<[u8; 32]> {
            mints.sort();
            mints.dedup();
            mints
        };
        let kamino_mints = distinct_sorted({
            let json_str = read_target_json_file(&manifest_dir, "kamino_reserve.json")
                .expect("failed to open kamino_reserve.json (required)");
            let reserves: Vec<KaminoReserveJson> =
                serde_json::from_str(&json_str).expect("failed to parse kamino_reserve.json");
            reserves.iter().map(|r| bs58_32(&r.mint, "mint")).collect()
        });
        let solend_mints = distinct_sorted({
            let json_str = read_target_json_file(&manifest_dir, "solend_reserve.json")
                .expect("failed to open solend_reserve.json (required)");
            let reserves: Vec<SolendReserveJson> =
                serde_json::from_str(&json_str).expect("failed to parse solend_reserve.json");
            reserves.iter().map(|r| bs58_32(&r.mint, "mint")).collect()
        });
        assert!(
            !kamino_mints.is_empty(),
            "no Kamino reserves found -- re-run download-arb with kamino enabled"
        );
        assert!(
            !solend_mints.is_empty(),
            "no Solend reserves found -- re-run download-arb with solend enabled"
        );

        let mut mints: Vec<[u8; 32]> = kamino_mints;
        for m in solend_mints {
            if !mints.contains(&m) {
                mints.push(m);
            }
        }
        mints.sort();

        let mut code = String::from(
            "pub struct TradeUniverseSymbolRaw {\n    \
             pub symbol: &'static str,\n    \
             pub mint: [u8; 32],\n    \
             pub decimals: u8,\n\
             }\n\
             pub static TRADE_UNIVERSE: &[TradeUniverseSymbolRaw] = &[\n",
        );
        let mut included = 0usize;
        let mut skipped_untracked = 0usize;
        let mut skipped_no_decimals = 0usize;
        for mint in &mints {
            if !mint_tracked(mint) {
                skipped_untracked += 1;
                continue;
            }
            let o_decimals: Option<u8> = mint_decimals.get(mint).copied();
            let Some(decimals) = o_decimals else {
                skipped_no_decimals += 1;
                println!(
                    "cargo:warning=trade_universe: mint {} has a real reserve and real pool liquidity but no \
                     mint_info decimals row -- skipping (real decimals data required for real sizing)",
                    bs58::encode(mint).into_string()
                );
                continue;
            };
            let symbol = match known_ticker_for(mint) {
                Some(s) => s.to_string(),
                None => {
                    let full = bs58::encode(mint).into_string();
                    format!(
                        "{}..{}",
                        &full[..4.min(full.len())],
                        &full[full.len().saturating_sub(4)..]
                    )
                }
            };
            code.push_str(&format!(
                "    TradeUniverseSymbolRaw {{ symbol: {symbol:?}, mint: {mint:?}, decimals: {decimals} }},\n",
            ));
            included += 1;
        }
        code.push_str("];\n");
        println!(
            "cargo:warning=trade_universe: {included} of {} real Kamino∪Solend reserve mints (all markets) included \
             ({skipped_untracked} skipped -- no real tracked pool liquidity; {skipped_no_decimals} skipped -- missing mint_info decimals)",
            mints.len()
        );
        File::create(out_dir.join("trade_universe_data.rs"))
            .expect("failed to create trade_universe_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write trade_universe_data.rs");
    }

    // ── perp funding target allocation default → target_allocation_data.rs ──
    // The optimizer (Go side) persists the latest target portfolio
    // allocation per symbol into `perp_funding_target_allocation`
    // whenever it computes/sends one via
    // `CustomMessageInbound::TargetAllocation` (see
    // `brain::testperpv1::message`'s doc) -- a fraction of total
    // portfolio value (0.0-1.0), e.g. `0.30` for "target 30% of the
    // portfolio in this symbol", remainder implicitly USD/stable. This
    // bakes in whatever's currently in that table as the compile-time
    // default a fresh bot starts with, before any runtime update
    // arrives -- same read-from-prefetch.db-at-build-time idea as
    // `symbol_mint_data.rs` just above, different table. Degrades
    // gracefully (defaults every symbol to `0.0`) against an older
    // `prefetch.db` that predates this table, or one the optimizer just
    // hasn't written an allocation into yet -- consistent with this
    // file's existing "prepare returning Err -> treat as absent"
    // precedent for forward/backward compatibility.
    {
        // Must match `CURATED_SYMBOL_MINTS`' symbol set above exactly --
        // a target allocation only makes sense for a symbol with a real
        // `Spot` graph node, which requires a curated mint.
        const TARGET_ALLOCATION_SYMBOLS: &[&str] = &["SOL", "BTC", "ETH", "XRP", "BNB", "SUI"];

        // perp_funding_target_allocation.json is a plain {symbol:
        // allocation_pct} object (optimizer/prefetch/perpfunding.
        // ExportJSON reuses GetAllTargetAllocations' own map shape
        // directly) -- missing file (predates this table, or the
        // optimizer hasn't written an allocation into it yet) degrades to
        // every symbol defaulting to 0.0, same as the old "prepare
        // returning Err -> treat as absent" precedent.
        let allocations: std::collections::HashMap<String, f64> =
            match read_target_json_file(&manifest_dir, "perp_funding_target_allocation.json") {
                Some(json_str) => serde_json::from_str(&json_str)
                    .expect("failed to parse perp_funding_target_allocation.json"),
                None => std::collections::HashMap::new(),
            };

        let mut code = String::from("pub static DEFAULT_TARGET_ALLOCATION: &[(&str, f64)] = &[\n");
        for &symbol in TARGET_ALLOCATION_SYMBOLS {
            let allocation_pct = allocations.get(symbol).copied().unwrap_or(0.0);
            code.push_str(&format!("    ({symbol:?}, {allocation_pct:?}),\n"));
        }
        code.push_str("];\n");
        File::create(out_dir.join("target_allocation_data.rs"))
            .expect("failed to create target_allocation_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write target_allocation_data.rs");
    }

    {
        // ── raydium_amm_pool.json → raydium_amm_data.rs ──────────────────────────
        {
            let mut code = String::from(
                "pub struct RaydiumAmmRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub market_bids: [u8; 32],\n    \
             pub market_asks: [u8; 32],\n    \
             pub market_event_queue: [u8; 32],\n    \
             pub market_coin_vault: [u8; 32],\n    \
             pub market_pc_vault: [u8; 32],\n    \
             pub market_vault_signer: [u8; 32],\n\
             }\n\
             pub static RAYDIUM_AMM_POOLS: &[RaydiumAmmRaw] = &[\n",
            );

            let priority_set: std::collections::HashSet<[u8; 32]> =
                priority_amm.iter().copied().collect();
            struct AmmCandidate {
                pubkey: [u8; 32],
                bid: [u8; 32],
                ask: [u8; 32],
                evq: [u8; 32],
                cvt: [u8; 32],
                pvt: [u8; 32],
                vsg: [u8; 32],
                secondary: i64,
            }
            // market_vault_signer IS NOT NULL was the only WHERE clause the
            // original SQL query had -- the other 5 market_* fields are
            // only ever populated together with it (see PoolRow's own doc
            // comment on the Go side), so `.expect()`ing them here (rather
            // than treating them as independently optional) preserves the
            // original query's own implicit assumption.
            let mut candidates: Vec<AmmCandidate> = amm_pools
                .iter()
                .filter_map(|p| {
                    let vsg = p.market_vault_signer.as_deref()?;
                    Some(AmmCandidate {
                        pubkey: bs58_32(&p.pubkey, "pubkey"),
                        bid: bs58_32(
                            p.market_bids
                                .as_deref()
                                .expect("market_bids missing alongside market_vault_signer"),
                            "market_bids",
                        ),
                        ask: bs58_32(
                            p.market_asks
                                .as_deref()
                                .expect("market_asks missing alongside market_vault_signer"),
                            "market_asks",
                        ),
                        evq: bs58_32(
                            p.market_event_queue
                                .as_deref()
                                .expect("market_event_queue missing alongside market_vault_signer"),
                            "market_event_queue",
                        ),
                        cvt: bs58_32(
                            p.market_coin_vault
                                .as_deref()
                                .expect("market_coin_vault missing alongside market_vault_signer"),
                            "market_coin_vault",
                        ),
                        pvt: bs58_32(
                            p.market_pc_vault
                                .as_deref()
                                .expect("market_pc_vault missing alongside market_vault_signer"),
                            "market_pc_vault",
                        ),
                        vsg: bs58_32(vsg, "market_vault_signer"),
                        // saturating_add: a handful of real high-supply
                        // tokens have raw balances close enough to
                        // i64::MAX that a plain `+` can overflow (SQLite's
                        // own arithmetic silently wrapped instead of
                        // panicking) -- saturating is safe here since this
                        // sum is only ever used as a relative ranking key,
                        // never a real quantity.
                        secondary: p.coin_balance.saturating_add(p.pc_balance),
                    })
                })
                .collect();
            candidates.sort_by(|a, b| {
                priority_set
                    .contains(&b.pubkey)
                    .cmp(&priority_set.contains(&a.pubkey))
                    .then_with(|| b.secondary.cmp(&a.secondary))
            });
            let budget = raydium_amm_pool_budget() as usize;
            for c in candidates.into_iter().take(budget) {
                code.push_str(&format!(
                    "    RaydiumAmmRaw {{ pubkey: {pk:?}, market_bids: {bid:?}, \
                 market_asks: {ask:?}, market_event_queue: {evq:?}, \
                 market_coin_vault: {cvt:?}, market_pc_vault: {pvt:?}, \
                 market_vault_signer: {vsg:?} }},\n",
                    pk = c.pubkey,
                    bid = c.bid,
                    ask = c.ask,
                    evq = c.evq,
                    cvt = c.cvt,
                    pvt = c.pvt,
                    vsg = c.vsg,
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("raydium_amm_data.rs"))
                .expect("failed to create raydium_amm_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write raydium_amm_data.rs");
        }
        // ── raydium_clmm_pool.json → raydium_clmm_data.rs ────────────────────────
        {
            let mut code = String::from(
                "pub struct RaydiumClmmRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub mint_0: [u8; 32],\n    \
             pub mint_1: [u8; 32],\n    \
             pub fee_rate_pips: u32,\n\
             }\n\
             pub static RAYDIUM_CLMM_POOLS: &[RaydiumClmmRaw] = &[\n",
            );

            let priority_set: std::collections::HashSet<[u8; 32]> =
                priority_clmm.iter().copied().collect();
            let mut ranked: Vec<(&RaydiumClmmPoolJson, [u8; 32])> = clmm_pools
                .iter()
                .map(|p| (p, bs58_32(&p.pubkey, "pubkey")))
                .collect();
            ranked.sort_by(|a, b| {
                priority_set
                    .contains(&b.1)
                    .cmp(&priority_set.contains(&a.1))
                    .then_with(|| {
                        b.0.token0_balance
                            .saturating_add(b.0.token1_balance)
                            .cmp(&a.0.token0_balance.saturating_add(a.0.token1_balance))
                    })
            });
            let budget = raydium_clmm_pool_budget() as usize;
            for (p, pk) in ranked.into_iter().take(budget) {
                let m0 = bs58_32(&p.token_mint0, "token_mint0");
                let m1 = bs58_32(&p.token_mint1, "token_mint1");
                code.push_str(&format!(
                    "    RaydiumClmmRaw {{ pubkey: {pk:?}, mint_0: {m0:?}, \
                 mint_1: {m1:?}, fee_rate_pips: {} }},\n",
                    p.trade_fee_rate,
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("raydium_clmm_data.rs"))
                .expect("failed to create raydium_clmm_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write raydium_clmm_data.rs");
        }
        // ── raydium_cpmm_pool.json → raydium_cpmm_data.rs ────────────────────────
        {
            let mut code = String::from(
                "pub struct RaydiumCpmmRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub mint_0: [u8; 32],\n    \
             pub mint_1: [u8; 32],\n    \
             pub trade_fee_rate: u64,\n\
             }\n\
             pub static RAYDIUM_CPMM_POOLS: &[RaydiumCpmmRaw] = &[\n",
            );

            let priority_set: std::collections::HashSet<[u8; 32]> =
                priority_cpmm.iter().copied().collect();
            let mut ranked: Vec<(&RaydiumCpmmPoolJson, [u8; 32])> = cpmm_pools
                .iter()
                .map(|p| (p, bs58_32(&p.pubkey, "pubkey")))
                .collect();
            ranked.sort_by(|a, b| {
                priority_set
                    .contains(&b.1)
                    .cmp(&priority_set.contains(&a.1))
                    .then_with(|| {
                        b.0.token0_balance
                            .saturating_add(b.0.token1_balance)
                            .cmp(&a.0.token0_balance.saturating_add(a.0.token1_balance))
                    })
            });
            let budget = raydium_cpmm_pool_budget() as usize;
            for (p, pk) in ranked.into_iter().take(budget) {
                let m0 = bs58_32(&p.token0_mint, "token0_mint");
                let m1 = bs58_32(&p.token1_mint, "token1_mint");
                code.push_str(&format!(
                    "    RaydiumCpmmRaw {{ pubkey: {pk:?}, mint_0: {m0:?}, \
                 mint_1: {m1:?}, trade_fee_rate: {} }},\n",
                    p.trade_fee_rate,
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("raydium_cpmm_data.rs"))
                .expect("failed to create raydium_cpmm_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write raydium_cpmm_data.rs");
        }
        // ── orca_whirlpool_pool.json → orca_data.rs ──────────────────────────────
        {
            let mut code = String::from(
                "pub struct OrcaWhirlpoolRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub mint_a: [u8; 32],\n    \
             pub mint_b: [u8; 32],\n\
             }\n\
             pub static ORCA_WHIRLPOOL_POOLS: &[OrcaWhirlpoolRaw] = &[\n",
            );

            // Ranked highest-active-liquidity-first and capped at
            // orca_pool_budget() -- see that function's doc comment for
            // why (OrcaState subscribes to every embedded pool's full
            // tick-array set, ~10KB each, at depth 2). Same
            // sign-bit-aware u128 ordering as orca/top.go's TopPools and
            // idx_orca_liquidity, since liquidity_lo/hi are
            // bit-reinterpreted i64 halves of a u128.
            let priority_set: std::collections::HashSet<[u8; 32]> =
                priority_orca.iter().copied().collect();
            let mut ranked: Vec<(&OrcaWhirlpoolPoolJson, [u8; 32])> = orca_pools
                .iter()
                .map(|p| (p, bs58_32(&p.pubkey, "pubkey")))
                .collect();
            ranked.sort_by(|a, b| {
                priority_set
                    .contains(&b.1)
                    .cmp(&priority_set.contains(&a.1))
                    .then_with(|| (b.0.liquidity_hi < 0).cmp(&(a.0.liquidity_hi < 0)))
                    .then_with(|| b.0.liquidity_hi.cmp(&a.0.liquidity_hi))
                    .then_with(|| (b.0.liquidity_lo < 0).cmp(&(a.0.liquidity_lo < 0)))
                    .then_with(|| b.0.liquidity_lo.cmp(&a.0.liquidity_lo))
            });
            let budget = orca_pool_budget() as usize;
            for (p, pk) in ranked.into_iter().take(budget) {
                let mint_a = bs58_32(&p.mint_a, "mint_a");
                let mint_b = bs58_32(&p.mint_b, "mint_b");
                code.push_str(&format!(
                    "    OrcaWhirlpoolRaw {{ pubkey: {pk:?}, mint_a: {mint_a:?}, \
                 mint_b: {mint_b:?} }},\n"
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("orca_data.rs"))
                .expect("failed to create orca_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write orca_data.rs");
        }
        // ── pumpswap_pool.json → pumpswap_data.rs ────────────────────────────────
        {
            let mut code = String::from(
                "pub struct PumpswapPoolRaw {\n    \
             pub pool: [u8; 32],\n    \
             pub base_mint: [u8; 32],\n    \
             pub quote_mint: [u8; 32],\n    \
             pub base_vault: [u8; 32],\n    \
             pub quote_vault: [u8; 32],\n\
             }\n\
             pub static PUMPSWAP_POOLS: &[PumpswapPoolRaw] = &[\n",
            );

            // Ranked by the real router-admission priority (real
            // dollar-liquidity, see the DexKind::Pumpswap router-graph
            // query above) first, raw base_balance as a tiebreak only --
            // same shape as ORCA_WHIRLPOOL_POOLS's emission just above.
            // pumpswap_pools is already empty (with a warning already
            // emitted at load time) if pumpswap_pool.json was missing --
            // same effective behavior as this file's old "table not found
            // -> empty list" convention, just detected at file-load time.
            let priority_set: std::collections::HashSet<[u8; 32]> =
                priority_pumpswap.iter().copied().collect();
            let mut ranked: Vec<(&PumpswapPoolJson, [u8; 32])> = pumpswap_pools
                .iter()
                .map(|p| (p, bs58_32(&p.pool, "pool")))
                .collect();
            ranked.sort_by(|a, b| {
                priority_set
                    .contains(&b.1)
                    .cmp(&priority_set.contains(&a.1))
                    .then_with(|| b.0.base_balance.cmp(&a.0.base_balance))
            });
            let budget = pumpswap_pool_budget() as usize;
            for (p, pool_pk) in ranked.into_iter().take(budget) {
                let base_mint = bs58_32(&p.base_mint, "base_mint");
                let quote_mint = bs58_32(&p.quote_mint, "quote_mint");
                let base_vault = bs58_32(&p.base_vault, "base_vault");
                let quote_vault = bs58_32(&p.quote_vault, "quote_vault");
                code.push_str(&format!(
                    "    PumpswapPoolRaw {{ pool: {pool_pk:?}, base_mint: {base_mint:?}, \
                 quote_mint: {quote_mint:?}, base_vault: {base_vault:?}, \
                 quote_vault: {quote_vault:?} }},\n"
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("pumpswap_data.rs"))
                .expect("failed to create pumpswap_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write pumpswap_data.rs");
        }
        // ── kamino_reserve.json → kamino_data.rs ─────────────────────────────────
        {
            let mut code = String::from(
                "pub struct KaminoReserveRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub lending_market: [u8; 32],\n    \
             pub supply_vault: [u8; 32],\n    \
             pub fee_vault: [u8; 32],\n\
             }\n\
             pub static KAMINO_RESERVES: &[KaminoReserveRaw] = &[\n",
            );

            let json_str = read_target_json_file(&manifest_dir, "kamino_reserve.json")
                .expect("failed to open kamino_reserve.json (required)");
            let reserves: Vec<KaminoReserveJson> =
                serde_json::from_str(&json_str).expect("failed to parse kamino_reserve.json");
            for r in reserves {
                let pk = bs58_32(&r.pubkey, "pubkey");
                let lending_market = bs58_32(&r.lending_market, "lending_market");
                let supply_vault = bs58_32(&r.supply_vault, "supply_vault");
                let fee_vault = bs58_32(&r.fee_vault, "fee_vault");
                code.push_str(&format!(
                    "    KaminoReserveRaw {{ pubkey: {pk:?}, \
                 lending_market: {lending_market:?}, supply_vault: {supply_vault:?}, \
                 fee_vault: {fee_vault:?} }},\n"
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("kamino_data.rs"))
                .expect("failed to create kamino_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write kamino_data.rs");
        }
        // ── prefetch db (marginfi_bank) → marginfi_data.rs ───────────────────────
        {
            let mut code = String::from(
                "pub struct MarginfiBankRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub group: [u8; 32],\n    \
             pub mint: [u8; 32],\n    \
             /// OracleSetup enum ordinal -- NOT always Pyth, see dex::marginfi.\n    \
             pub oracle_setup: u8,\n    \
             pub oracle_key: [u8; 32],\n\
             }\n\
             pub static MARGINFI_BANKS: &[MarginfiBankRaw] = &[\n",
            );

            // marginfi_bank.json is always written with the current schema
            // (oracle_setup/oracle_key always populated) -- optimizer's
            // store.DB always migrates the current schema before
            // marginfi.ExportJSON runs, so the old 2-tier "older prefetch.db
            // missing oracle columns" fallback this block used to have has
            // no equivalent here; only "file not found at all" degrades.
            match read_target_json_file(&manifest_dir, "marginfi_bank.json") {
                Some(json_str) => {
                    let banks: Vec<MarginfiBankJson> = serde_json::from_str(&json_str)
                        .expect("failed to parse marginfi_bank.json");
                    for b in banks {
                        let pk = bs58_32(&b.pubkey, "pubkey");
                        let group = bs58_32(&b.group, "group");
                        let mint = bs58_32(&b.mint, "mint");
                        let oracle_key = bs58_32(&b.oracle_key, "oracle_key");
                        code.push_str(&format!(
                            "    MarginfiBankRaw {{ pubkey: {pk:?}, group: {group:?}, mint: {mint:?}, \
                         oracle_setup: {}, oracle_key: {oracle_key:?} }},\n",
                            b.oracle_setup,
                        ));
                    }
                }
                None => {
                    println!(
                        "cargo:warning=marginfi_bank.json not found -- re-run download-arb to \
                         populate MarginFi bank data"
                    );
                }
            }
            code.push_str("];\n");
            File::create(out_dir.join("marginfi_data.rs"))
                .expect("failed to create marginfi_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write marginfi_data.rs");
        }
        // ── prefetch db (solend_reserve) → solend_data.rs ────────────────────────
        {
            let mut code = String::from(
                "pub struct SolendReserveRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub lending_market: [u8; 32],\n    \
             pub mint: [u8; 32],\n    \
             pub supply_vault: [u8; 32],\n\
             }\n\
             pub static SOLEND_RESERVES: &[SolendReserveRaw] = &[\n",
            );

            match read_target_json_file(&manifest_dir, "solend_reserve.json") {
                Some(json_str) => {
                    let reserves: Vec<SolendReserveJson> = serde_json::from_str(&json_str)
                        .expect("failed to parse solend_reserve.json");
                    for r in reserves {
                        let pk = bs58_32(&r.pubkey, "pubkey");
                        let lending_market = bs58_32(&r.lending_market, "lending_market");
                        let mint = bs58_32(&r.mint, "mint");
                        let supply_vault = bs58_32(&r.supply_vault, "supply_vault");
                        code.push_str(&format!(
                            "    SolendReserveRaw {{ pubkey: {pk:?}, \
                         lending_market: {lending_market:?}, mint: {mint:?}, \
                         supply_vault: {supply_vault:?} }},\n"
                        ));
                    }
                }
                None => {
                    println!(
                        "cargo:warning=solend_reserve.json not found -- re-run download-arb to \
                         populate Solend reserve data"
                    );
                }
            }
            code.push_str("];\n");
            File::create(out_dir.join("solend_data.rs"))
                .expect("failed to create solend_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write solend_data.rs");
        }
        // ── prefetch db (drift_spot_market) → drift_data.rs ──────────────────────
        {
            let mut code = String::from(
                "pub struct DriftSpotMarketRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub mint: [u8; 32],\n    \
             pub vault: [u8; 32],\n\
             }\n\
             pub static DRIFT_SPOT_MARKETS: &[DriftSpotMarketRaw] = &[\n",
            );

            match read_target_json_file(&manifest_dir, "drift_spot_market.json") {
                Some(json_str) => {
                    let markets: Vec<DriftSpotMarketJson> = serde_json::from_str(&json_str)
                        .expect("failed to parse drift_spot_market.json");
                    for m in markets {
                        let pk = bs58_32(&m.pubkey, "pubkey");
                        let mint = bs58_32(&m.mint, "mint");
                        let vault = bs58_32(&m.vault, "vault");
                        code.push_str(&format!(
                            "    DriftSpotMarketRaw {{ pubkey: {pk:?}, mint: {mint:?}, vault: {vault:?} }},\n"
                        ));
                    }
                }
                None => {
                    println!(
                        "cargo:warning=drift_spot_market.json not found -- re-run download-arb \
                         to populate Drift spot market data"
                    );
                }
            }
            code.push_str("];\n");
            File::create(out_dir.join("drift_data.rs"))
                .expect("failed to create drift_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write drift_data.rs");
        }
        // ── prefetch db (jet_reserve) → jet_data.rs ──────────────────────────────
        {
            let mut code = String::from(
                "pub struct JetReserveRaw {\n    \
             pub pubkey: [u8; 32],\n    \
             pub market: [u8; 32],\n    \
             pub mint: [u8; 32],\n    \
             pub vault: [u8; 32],\n\
             }\n\
             pub static JET_RESERVES: &[JetReserveRaw] = &[\n",
            );

            match read_target_json_file(&manifest_dir, "jet_reserve.json") {
                Some(json_str) => {
                    let reserves: Vec<JetReserveJson> =
                        serde_json::from_str(&json_str).expect("failed to parse jet_reserve.json");
                    for r in reserves {
                        let pk = bs58_32(&r.pubkey, "pubkey");
                        let market = bs58_32(&r.market, "market");
                        let mint = bs58_32(&r.mint, "mint");
                        let vault = bs58_32(&r.vault, "vault");
                        code.push_str(&format!(
                            "    JetReserveRaw {{ pubkey: {pk:?}, market: {market:?}, mint: {mint:?}, \
                         vault: {vault:?} }},\n"
                        ));
                    }
                }
                None => {
                    println!(
                        "cargo:warning=jet_reserve.json not found -- re-run download-arb to \
                         populate Jet Protocol reserve data"
                    );
                }
            }
            code.push_str("];\n");
            File::create(out_dir.join("jet_data.rs"))
                .expect("failed to create jet_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write jet_data.rs");
        }
        // ── prefetch db (sanctum_lst) → sanctum_data.rs ──────────────────────────
        {
            let mut code = String::from(
                "pub struct SanctumLstRaw {\n    \
             pub mint: [u8; 32],\n    \
             pub sol_value_calculator: [u8; 32],\n    \
             pub sol_value: u64,\n    \
             /// The LST's underlying stake-pool account, for the SanctumSpl/\n    \
             /// SanctumSplMulti/Spl SOL-value-calculator kinds -- all-zero\n    \
             /// means unknown (not in the vendored sanctum-lst-list.toml\n    \
             /// registry, or this LST uses a different calculator kind).\n    \
             pub pool_state: [u8; 32],\n    \
             /// Build-time snapshot of the pool-reserves ATA's raw token\n    \
             /// balance -- seeds SanctumState's live LstInfo.reserve so\n    \
             /// spot_price() doesn't have to wait for the first live\n    \
             /// on_token event. 0 means not yet fetched (predates this\n    \
             /// column) or genuinely empty.\n    \
             pub reserve: u64,\n\
             }\n\
             pub static SANCTUM_LSTS: &[SanctumLstRaw] = &[\n",
            );

            // sanctum_lst.json's pool_state is always a real (possibly
            // all-zero) base58 pubkey string, never a JSON null -- Go's
            // loadLsts already normalizes a NULL pool_state column to the
            // zero-value sgo.PublicKey before marshaling (see
            // optimizer/prefetch/sanctum/store.go), so decoding that
            // string back here always yields [0u8; 32] for the same
            // "unknown" case the old Option<Vec<u8>> handling covered --
            // no separate null case needed.
            let json_str = read_target_json_file(&manifest_dir, "sanctum_lst.json")
                .expect("failed to open sanctum_lst.json (required)");
            let lsts: Vec<SanctumLstJson> =
                serde_json::from_str(&json_str).expect("failed to parse sanctum_lst.json");
            for l in lsts {
                let mint = bs58_32(&l.mint, "mint");
                let sol_value_calculator = bs58_32(&l.sol_value_calculator, "sol_value_calculator");
                let pool_state = bs58_32(&l.pool_state, "pool_state");
                code.push_str(&format!(
                    "    SanctumLstRaw {{ mint: {mint:?}, \
                 sol_value_calculator: {sol_value_calculator:?}, sol_value: {}, \
                 pool_state: {pool_state:?}, reserve: {} }},\n",
                    l.sol_value, l.reserve,
                ));
            }
            code.push_str("];\n");
            File::create(out_dir.join("sanctum_data.rs"))
                .expect("failed to create sanctum_data.rs")
                .write_all(code.as_bytes())
                .expect("failed to write sanctum_data.rs");
        }
    }

    // ── prefetch db (address_lookup_table) → address_lookup_table.rs ─────────
    // Written by optimizer's `alt` subcommand (cmd/alt.go); schema in
    // optimizer/prefetch/alt/schema.sql. Rows are ordered (table_pubkey,
    // position) and grouped here into per-table runs, since position order
    // is load-bearing -- it's the exact on-chain lookup index each account
    // resolves to (see src/wallet.rs's AddressLookupTable::load_default).
    {
        // Emit: pub static ADDRESS_LOOKUP_TABLES: &[([u8;32], &[[u8;32]])] = &[...]
        // Each entry is (alt_pubkey, &[account_pubkey, ...])
        let mut code =
            String::from("pub static ADDRESS_LOOKUP_TABLES: &[([u8; 32], &[[u8; 32]])] = &[\n");

        // address_lookup_table.json is already written ordered by
        // (table_pubkey, position) -- optimizer/prefetch/alt.ExportJSON
        // preserves the exact ORDER BY the old direct query used, since
        // that order is load-bearing (see this block's own doc comment
        // above) -- so the grouping-by-adjacency reconstruction below is
        // unchanged, just fed from deserialized rows instead of rusqlite
        // rows.
        match read_target_json_file(&manifest_dir, "address_lookup_table.json") {
            Some(json_str) => {
                let entries: Vec<AddressLookupTableJson> = serde_json::from_str(&json_str)
                    .expect("failed to parse address_lookup_table.json");
                let mut current: Option<([u8; 32], String)> = None;
                for e in entries {
                    let table_pubkey = bs58_32(&e.table_pubkey, "table_pubkey");
                    let account_pubkey = bs58_32(&e.account_pubkey, "account_pubkey");
                    match &mut current {
                        Some((tpk, body)) if *tpk == table_pubkey => {
                            body.push_str(&format!("        {account_pubkey:?},\n"));
                        }
                        _ => {
                            if let Some((tpk, body)) = current.take() {
                                code.push_str(&format!("    ({tpk:?}, &[\n{body}    ]),\n"));
                            }
                            current =
                                Some((table_pubkey, format!("        {account_pubkey:?},\n")));
                        }
                    }
                }
                if let Some((tpk, body)) = current.take() {
                    code.push_str(&format!("    ({tpk:?}, &[\n{body}    ]),\n"));
                }
            }
            None => {
                println!(
                    "cargo:warning=address_lookup_table.json not found -- run `optimizer alt` \
                     to populate it, or ignore if you don't use lookup tables"
                );
            }
        }
        code.push_str("];\n");

        let dest = Path::new(&out_dir).join("address_lookup_table.rs");
        File::create(&dest)
            .expect("failed to create address_lookup_table.rs")
            .write_all(code.as_bytes())
            .expect("failed to write address_lookup_table.rs");
    }

    // ── trading_data.rs ──────────────────────────────────────────────────────
    // `src/lib.rs`'s `trading_config` module includes this unconditionally;
    // nothing currently populates `TRADING_PAIRS` with real data (a prior
    // refactor removed the generator for it -- see git history for "removing
    // old trading structs" -- without removing the `include!`), so this just
    // restores the always-empty static every existing OUT_DIR build artifact
    // already had, unblocking builds for target triples that don't happen to
    // have a stale copy lying around from before that refactor.
    {
        let dest = out_dir.join("trading_data.rs");
        File::create(&dest)
            .expect("failed to create trading_data.rs")
            .write_all(b"pub static TRADING_PAIRS: &[([u8; 32], [u8; 32])] = &[];\n")
            .expect("failed to write trading_data.rs");
    }

    // ── diagnostic_data.rs ────────────────────────────────────────────────
    // See trade_router_probe_lamports's doc comment.
    {
        let dest = out_dir.join("diagnostic_data.rs");
        let code = format!(
            "pub static TRADE_ROUTER_PROBE_LAMPORTS: u64 = {};\n",
            trade_router_probe_lamports(),
        );
        File::create(&dest)
            .expect("failed to create diagnostic_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write diagnostic_data.rs");
    }

    // ── bundler_data.rs ──────────────────────────────────────────────────
    // See bundler()'s doc comment.
    {
        let dest = out_dir.join("bundler_data.rs");
        let code = format!("pub static BUNDLER: u8 = {};\n", bundler());
        File::create(&dest)
            .expect("failed to create bundler_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write bundler_data.rs");
    }

    // ── top_pools_data.rs ────────────────────────────────────────────────────
    // Unified top-N-by-liquidity pools across every dex the optimizer
    // tracks, for generic ranking/routing code — unlike the per-program
    // sections above (which keep each dex's native fields for
    // instruction-building), this collapses everything to a common shape:
    // which two mints, and the spot price between them. Reuses
    // amm_pools/cpmm_pools/clmm_pools/orca_pools -- the same Vecs loaded
    // once near the top of main() for the router-graph BFS and each dex's
    // embed re-query above -- instead of a fourth SQL round-trip per table.
    println!("cargo:rerun-if-env-changed=TOP_N");
    {
        let n = top_n() as usize;

        let mut code = String::from(
            "pub struct TopPool {\n    \
             pub program: &'static str,\n    \
             pub pubkey: [u8; 32],\n    \
             pub mint_a: [u8; 32],\n    \
             pub mint_b: [u8; 32],\n    \
             /// Units of mint_b per unit of mint_a, raw token units (no\n    \
             /// decimals normalization, no fee adjustment).\n    \
             pub price_a_to_b: f64,\n\
             }\n\
             pub static TOP_POOLS: &[TopPool] = &[\n",
        );

        // amm: price = pc_balance / coin_balance (coin -> pc). Already
        // filtered by amm_pools' own Go-side query (coin_balance > 0 AND
        // pc_balance > 0 AND coin_mint/pc_mint NOT NULL), matching this
        // original SQL WHERE clause exactly.
        {
            let mut ranked: Vec<&RaydiumAmmPoolJson> = amm_pools.iter().collect();
            ranked.sort_by(|a, b| {
                b.coin_balance
                    .saturating_add(b.pc_balance)
                    .cmp(&a.coin_balance.saturating_add(a.pc_balance))
            });
            for p in ranked.into_iter().take(n) {
                let pk = bs58_32(&p.pubkey, "pubkey");
                let mint_a = bs58_32(&p.coin_mint, "coin_mint");
                let mint_b = bs58_32(&p.pc_mint, "pc_mint");
                let price = p.pc_balance as f64 / p.coin_balance as f64;
                code.push_str(&format!(
                    "    TopPool {{ program: \"amm\", pubkey: {pk:?}, mint_a: {mint_a:?}, \
                     mint_b: {mint_b:?}, price_a_to_b: f64::from_bits({bits}u64) }},\n",
                    bits = price.to_bits()
                ));
            }
        }

        // cpmm: price = token1_balance / token0_balance. Already filtered
        // by cpmm_pools' own Go-side query, matching this original SQL
        // WHERE clause exactly.
        {
            let mut ranked: Vec<&RaydiumCpmmPoolJson> = cpmm_pools.iter().collect();
            ranked.sort_by(|a, b| {
                b.token0_balance
                    .saturating_add(b.token1_balance)
                    .cmp(&a.token0_balance.saturating_add(a.token1_balance))
            });
            for p in ranked.into_iter().take(n) {
                let pk = bs58_32(&p.pubkey, "pubkey");
                let mint_a = bs58_32(&p.token0_mint, "token0_mint");
                let mint_b = bs58_32(&p.token1_mint, "token1_mint");
                let price = p.token1_balance as f64 / p.token0_balance as f64;
                code.push_str(&format!(
                    "    TopPool {{ program: \"cpmm\", pubkey: {pk:?}, mint_a: {mint_a:?}, \
                     mint_b: {mint_b:?}, price_a_to_b: f64::from_bits({bits}u64) }},\n",
                    bits = price.to_bits()
                ));
            }
        }

        // clmm: price = token1_balance / token0_balance. Already filtered
        // by clmm_pools' own Go-side query, matching this original SQL
        // WHERE clause exactly.
        {
            let mut ranked: Vec<&RaydiumClmmPoolJson> = clmm_pools.iter().collect();
            ranked.sort_by(|a, b| {
                b.token0_balance
                    .saturating_add(b.token1_balance)
                    .cmp(&a.token0_balance.saturating_add(a.token1_balance))
            });
            for p in ranked.into_iter().take(n) {
                let pk = bs58_32(&p.pubkey, "pubkey");
                let mint_a = bs58_32(&p.token_mint0, "token_mint0");
                let mint_b = bs58_32(&p.token_mint1, "token_mint1");
                let price = p.token1_balance as f64 / p.token0_balance as f64;
                code.push_str(&format!(
                    "    TopPool {{ program: \"clmm\", pubkey: {pk:?}, mint_a: {mint_a:?}, \
                     mint_b: {mint_b:?}, price_a_to_b: f64::from_bits({bits}u64) }},\n",
                    bits = price.to_bits()
                ));
            }
        }

        // orca: price = spot price derived from sqrt_price_x64, matching
        // Whirlpool.SpotPrice() on the optimizer's Go side. sqrt_price_lo/hi
        // are stored as bit-reinterpreted i64s of a u128 split in two, so
        // they're cast back through u64 before converting to f64. No
        // balance filter here, matching the original SQL query's own lack
        // of a WHERE clause.
        {
            let mut ranked: Vec<&OrcaWhirlpoolPoolJson> = orca_pools.iter().collect();
            ranked.sort_by(|a, b| {
                (b.liquidity_hi < 0)
                    .cmp(&(a.liquidity_hi < 0))
                    .then_with(|| b.liquidity_hi.cmp(&a.liquidity_hi))
                    .then_with(|| (b.liquidity_lo < 0).cmp(&(a.liquidity_lo < 0)))
                    .then_with(|| b.liquidity_lo.cmp(&a.liquidity_lo))
            });
            for p in ranked.into_iter().take(n) {
                let pk = bs58_32(&p.pubkey, "pubkey");
                let mint_a = bs58_32(&p.mint_a, "mint_a");
                let mint_b = bs58_32(&p.mint_b, "mint_b");
                let sqrt_price = (p.sqrt_price_hi as u64) as f64
                    + (p.sqrt_price_lo as u64) as f64 / 2f64.powi(64);
                let price = sqrt_price * sqrt_price;
                code.push_str(&format!(
                    "    TopPool {{ program: \"orca\", pubkey: {pk:?}, mint_a: {mint_a:?}, \
                     mint_b: {mint_b:?}, price_a_to_b: f64::from_bits({bits}u64) }},\n",
                    bits = price.to_bits()
                ));
            }
        }

        code.push_str("];\n");
        File::create(out_dir.join("top_pools_data.rs"))
            .expect("failed to create top_pools_data.rs")
            .write_all(code.as_bytes())
            .expect("failed to write top_pools_data.rs");
    }
}
