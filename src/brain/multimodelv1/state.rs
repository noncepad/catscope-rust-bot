//! Reactive loop for `multimodelv1` -- Phase 5 of
//! `PLAN-1.md`, sub-phase 5a: the state skeleton and wallet/market-data
//! subscriptions only, no decision or execution logic yet. Same
//! milestone `leveragedloopv1`/`testperpv1` each reached before any real
//! trading logic existed ("idle-verified": connects, subscribes,
//! processes real account/token updates, sends nothing) -- see
//! `PLAN-1.md`'s own "phased, prove-idle-before-trading discipline" note.
//!
//! Deliberately mirrors `arbv1::state`'s shape, not
//! `leveragedloopv1`/`testperpv1`'s -- `arbv1` is the smallest existing
//! mode that already has a real `DexState` + `TradeRouter` (`helloworldv1`
//! doesn't use either), so it's the cleanest base to strip down rather
//! than trimming five bot-modes' worth of Kamino/basis-trade/DAG logic
//! back out of a heavier template. Tx-latency bookkeeping
//! (`m_sig`/`q_sig`/`tracker`/`tx_latency`/`log_stats`/`recycle_q` in
//! `arbv1::state::State`) is deliberately not carried over here --
//! `mid_on_tx` is a plain inherent method, not a trait requirement, so a
//! mode with nothing to do there yet can leave it empty; add that
//! bookkeeping back if/when this mode actually needs transaction-landing
//! latency data.
//!
//! Phase 5 sub-phase 5b adds Phase 0's real, live factor computation
//! (`run_factor_resync`/`build_live_factor_graph`), gated behind a new
//! `TriggerEnableFactorLogging` -- read-only: it computes and logs real
//! structural factors from live pool liquidity, opens or closes nothing.
//! `factor_borrow_gate`/`factor_sizing`'s sizing role/`factor_intent`
//! (Phases 2-4) are still not wired into any decision here -- there is no
//! decision yet, only the real data pipeline those phases will need.
//!
//! Sub-phase 5b also fixes a real gap in 5a's own skeleton, found while
//! building this: `self.state.router` (`TradeRouter`) was only ever
//! seeded with nodes in `on_load` and never actually kept live --
//! `arbv1::state` (this module's own base template) calls
//! `dex.refresh_token_router`/`dex.refresh_account_router` after every
//! real `on_token`/`on_account`, which this file's sub-phase 5a version
//! omitted. Without that, `TradeRouter::route_slippage_aware` (what
//! `factor_sizing::price_impact_bps` uses below) would never find a real
//! route -- fixed here via the same freshness-gated pattern `arbv1` uses
//! (`m_account_slot`/`record_low_latency_slot`/`is_newer_than_low_latency`),
//! not invented fresh.
//!
//! Sub-phase 5c adds the first real, executable strategy: a pure-Kamino
//! pair/stat-arb trade (`PLAN-1.md` Phase 5 point 2, `INSTRUCTIONS.md`
//! §3B) -- long the most-underperforming curated symbol (real Kamino
//! deposit), short the most-overperforming one (real Kamino borrow +
//! sell), when each clears `trader::factor_residual`'s real 2-3σ z-score
//! gate against a real rolling residual history. Gated behind
//! `TriggerEnablePairTrading`. Real execution -- not a read-only preview
//! -- ported almost verbatim from `leveragedloopv1`'s own real,
//! live-tested basis-trade Kamino legs (`open_kamino_deposit_leg`/
//! `open_kamino_borrow_leg`/close variants/`execute_spot_leg`/the
//! reserve-refresh-and-farm-bootstrap plumbing), retargeted at this
//! mode's own, fully independent `id=2` Kamino obligation (`id=0`/`id=1`
//! are `leveragedloopv1`'s leverage loop and basis trade respectively --
//! `id=2` stays safely independent of both even in the real, live-confirmed
//! case where the same fee-payer runs more than one bot mode and their
//! child wallets collide, see `o_pair_kamino_position`'s own doc
//! comment). Two more real gaps found and fixed while wiring this in,
//! neither anticipated by sub-phase 5a/5b (neither one ever built or sent
//! a real transaction, so neither gap was reachable before now): `evaluate()`
//! never drained/sent `self.wallet`'s queued instructions at all (every
//! other real bot mode's `evaluate()` ends with `self.wallet.drain_and_send()`);
//! and there was no `pending_route_pools` field for `execute_spot_leg`/
//! the drain tail's pool-cooldown-on-send-failure handling to share.
//!
//! Residual/rolling-stats tracking (`trader::factor_residual`) is
//! genuinely new -- no prior bot mode in this repo tracks a time series
//! of anything across resync cycles the way a real 2-3σ z-score gate
//! needs. Real per-symbol prices come from each curated symbol's own
//! Kamino reserve oracle price (`reserve.price_usd`, already loaded,
//! already used for every other real Kamino sizing calc in this
//! codebase) -- not a new oracle dependency.
use crate::{
    brain::multimodelv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate, SubscriptionQueue},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    router_config, router_pools_config,
    trade_universe_config,
    trader::{
        dex::{
            ember, kamino,
            phoenix::{self, ix::Side, PhoenixState},
            solend,
            update::Updater as _,
            DexState,
        },
        dispersion_basket, factor_basket, factor_borrow_gate, factor_graph, factor_residual, factor_sizing, hawkes_factor,
        pair_basket, planner,
        pricegraph::{self, TradeRouter},
        residual_snapshot, router,
    },
    txview::TransactionList,
    util::{account_id_from_pubkey, pubkey_from_account_id, rc_unlock},
    wallet::{PriorityLevel, Wallet},
};
use solana_sdk::{
    clock::Slot,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

/// Real, fully independent Kamino obligation id for this mode's pair
/// trade -- `leveragedloopv1`'s own leverage loop uses `id=0`, its basis
/// trade uses `id=1`; `id=2` stays independent of both. This matters
/// beyond just "don't share state with a different strategy" (the usual
/// reasoning `kamino::obligation_pda`'s own doc comment gives): this
/// mode's Go-side child-key derivation
/// (`common.DeriveChildKeyFromIndex(parentKey, 1)`, `eval.go`) uses the
/// *exact same index* `leveragedloopv1`'s own Go package uses -- if the
/// same fee-payer is ever used to run both `optimizer leveraged-loop` and
/// `optimizer multi-model`, they derive the identical child wallet, and
/// would otherwise collide on the same obligation id too. Not fixed at
/// the derivation-index level (out of scope here); `id=2` sidesteps the
/// consequence regardless of whether that collision is ever hit.
const PAIR_KAMINO_OBLIGATION_ID: u8 = 2;

/// This mode's own independent Solend obligation id, added when Solend
/// execution was added alongside Kamino for the pair trade. Same real
/// collision reasoning as [`PAIR_KAMINO_OBLIGATION_ID`]'s doc comment --
/// every bot mode's Go side derives its child wallet via the same
/// `common.DeriveChildKeyFromIndex(parentKey, 1)` index -- but the real
/// occupant is different here: `perpfundingv1`/`testperpv1` already use
/// Solend obligation `id=0` (unlike Kamino, where both `id=0` and `id=1`
/// were already taken by `leveragedloopv1`), so `id=1` is the first free
/// value, not `id=2`.
const PAIR_SOLEND_OBLIGATION_ID: u8 = 1;

/// This mode's own independent Kamino obligation id for trade type 1
/// (directional factor-neutral), separate from [`PAIR_KAMINO_OBLIGATION_ID`]
/// so the two trade types never share an obligation even if both are ever
/// enabled in the same process -- same real child-key-derivation
/// collision reasoning as [`PAIR_KAMINO_OBLIGATION_ID`]'s own doc
/// comment. `leveragedloopv1` already took `0`/`1`, the pair trade took
/// `2`, so `3` is the first free value.
const DIRECTIONAL_KAMINO_OBLIGATION_ID: u8 = 3;

/// This mode's own independent Solend obligation id for trade type 1 --
/// see [`DIRECTIONAL_KAMINO_OBLIGATION_ID`]'s doc comment. `perpfundingv1`/
/// `testperpv1` already use `0`, the pair trade took `1`, so `2` is the
/// first free value.
const DIRECTIONAL_SOLEND_OBLIGATION_ID: u8 = 2;

/// Same `LOOP_MAX_HOPS`-style routing-depth constant every other mode's
/// `execute_spot_leg` caller uses, sized down from `leveragedloopv1`'s
/// own `5` since this mode's curated symbols all route through common,
/// liquid hubs (SOL/USDC) -- revisit if a real route search comes back
/// empty for a curated pair that needs a longer real path.
const LOOP_MAX_HOPS: usize = 3;

/// Intended (pre-slippage-check) real USD notional per leg before a
/// pair-trade open is even attempted -- same conservative-sizing role as
/// `leveragedloopv1`'s own `BASIS_CYCLE_MIN_MARGIN_USD`, same value, not
/// recalibrated. The *actual* sent notional may end up smaller than this
/// once `size_pair_legs` (Phase 3) checks real slippage -- see
/// `MIN_SIZED_FRACTION_OF_INTENDED`.
const PAIR_CYCLE_MIN_NOTIONAL_USD: f64 = 10.0;

/// Floor on how far `size_pair_legs`'s real slippage-aware sizing is
/// allowed to shrink a leg below [`PAIR_CYCLE_MIN_NOTIONAL_USD`] before
/// the whole cycle just skips opening anything -- a real trade sized
/// down to a few cents by a shallow pool isn't worth the transaction
/// cost/complexity of sending it. Starting value, not calibrated.
const MIN_SIZED_FRACTION_OF_INTENDED: f64 = 0.5;

/// Intended (pre-slippage-check) real USD notional for the long leg of a
/// directional-neutral basket -- same role as [`PAIR_CYCLE_MIN_NOTIONAL_USD`]
/// for the pair trade, same starting value, not recalibrated. Each short
/// leg's own intended notional is this value times that leg's
/// `weight_fraction` (see `factor_basket::DirectionalBasketLeg`), so the
/// basket's short legs sum to the same total notional as the long.
const DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD: f64 = 10.0;

/// Intended (pre-slippage-check) real total USD notional for a dispersion
/// basket's long side -- same role as [`DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD`]
/// (a total, split across legs by each leg's own `weight_fraction`, not a
/// fixed amount per leg). Derived from [`PAIR_CYCLE_MIN_NOTIONAL_USD`]
/// (this mode's real per-leg floor everywhere else) times the basket's
/// target leg count, so a full, evenly-loaded basket gives each leg
/// roughly its own real minimum before vol-weighting skews individual
/// legs up or down -- not an independently chosen number.
const DISPERSION_CYCLE_MIN_NOTIONAL_USD: f64 = dispersion_basket::DISPERSION_BASKET_SIZE as f64 * PAIR_CYCLE_MIN_NOTIONAL_USD;

/// Trade type 5's (Hawkes-on-eigenfactor momentum) own independent
/// Kamino obligation id -- see [`PAIR_KAMINO_OBLIGATION_ID`]'s doc comment
/// for the real child-key-derivation collision reasoning every one of
/// these ids exists to sidestep. `leveragedloopv1` took `0`/`1`, pair took
/// `2`, directional took `3`, so `4` is the first free value.
const HAWKES_KAMINO_OBLIGATION_ID: u8 = 4;

/// Trade type 5's own independent Solend obligation id -- see
/// [`PAIR_SOLEND_OBLIGATION_ID`]'s doc comment. `perpfundingv1`/
/// `testperpv1` took `0`, pair took `1`, directional took `2`, so `3` is
/// the first free value.
const HAWKES_SOLEND_OBLIGATION_ID: u8 = 3;

/// Intended (pre-slippage-check) real total USD notional for a Hawkes
/// momentum basket -- same role/value as [`PAIR_CYCLE_MIN_NOTIONAL_USD`]/
/// [`DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD`], split across every leg (long
/// and short together) by that leg's own `weight_fraction` from
/// `hawkes_factor::factor_jump_basket` (which already sums to 1.0 across
/// the whole basket, not per side).
const HAWKES_CYCLE_MIN_NOTIONAL_USD: f64 = 10.0;

/// How many consecutive real resync cycles a Hawkes momentum position is
/// allowed to stay open before it's force-closed regardless of whether
/// `hawkes_factor::should_close_hawkes_on_decay` has fired yet -- a real
/// safety net against an untested `alpha`/`beta` calibration keeping a
/// position open indefinitely on stale excitation (see
/// `docs/HAWKES_FACTOR_TRADE_PLAN.md`'s exit-trigger design). Starting
/// value, not calibrated against any real trading history yet.
const HAWKES_MAX_HOLDING_CYCLES: i64 = 20;

/// One-time bootstrap deposit (USD) for dispersion's own Phoenix trader
/// account -- same small starting-budget role as `perpfundingv1::state`'s
/// own bootstrap deposit, just enough to register a funded trader account;
/// unlike that basis trade, dispersion's real short-index notional varies
/// basket to basket, so this is deliberately *not* sized to cover a real
/// position -- `top_up_dispersion_margin` (Phase 4) tops up the real
/// difference right before each open, using `dex::phoenix::margin::
/// required_margin_usd_for_notional`'s real formula rather than a second
/// guessed constant.
const DISPERSION_BOOTSTRAP_MARGIN_USD: f64 = 5.0;

/// Below this real USD value, a curated mint's wallet balance is treated
/// as leftover dust, not a real open dispersion leg -- see `current_open_
/// dispersion`'s doc comment for the real, live-confirmed incident this
/// closes (pre-existing small balances from earlier, unrelated trading
/// this same session blocked dispersion's open-pass from ever running).
/// Comfortably below any real leg's own intended notional (`DISPERSION_
/// CYCLE_MIN_NOTIONAL_USD / DISPERSION_BASKET_SIZE`, at least a few
/// dollars even at the smallest real weight), so this can't mistake a
/// real, deliberately-small real position for dust.
const DISPERSION_DUST_FLOOR_USD: f64 = 1.0;

/// Real minimum routable USD notional a curated mint must clear (via
/// `factor_sizing::max_safe_notional`, the exact same real function
/// `size_dispersion_long_legs`'s own sizing already uses) before it's
/// even eligible as a dispersion basket candidate -- live-confirmed
/// necessary, not a hypothetical: `factor_sizing::size_basket` scales
/// *every* leg in a basket down to match whichever single leg has the
/// worst real safe/intended ratio, so one genuinely illiquid candidate
/// silently drags perfectly liquid legs (SOL, live-observed sized down
/// to $0.15 of a $12.94 intended notional) down with it. Dispersion's own
/// candidate ranking is by highest idiosyncratic volatility
/// (`residual_stdev`), and for long-tail curated mints, high volatility
/// and low real liquidity tend to go together -- without this floor, the
/// ranking systematically prefers exactly the illiquid names that poison
/// every basket's sizing. Same reference amount as this mode's own
/// per-leg floor everywhere else (`PAIR_CYCLE_MIN_NOTIONAL_USD`) -- a
/// mint that can't even safely absorb $10 isn't a viable candidate
/// regardless of its eventual basket weight.
const DISPERSION_CANDIDATE_LIQUIDITY_FLOOR_USD: f64 = PAIR_CYCLE_MIN_NOTIONAL_USD;

/// Floor a Kamino/Solend reserve's `price_usd` must clear to be trusted
/// for the sell-direction reference-amount conversion below -- live-
/// confirmed necessary: a real curated mint (SRM, whose Kamino/Solend
/// market is effectively dead) had a nonzero but absurdly tiny
/// `price_usd`, which `DISPERSION_CANDIDATE_LIQUIDITY_FLOOR_USD /
/// price_usd` blew up into a nonsensical ~1e17 raw-unit reference amount
/// -- the old `p > 0.0` filter let it through since the price technically
/// wasn't exactly zero. `1e-9` is comfortably below any real curated
/// token's price (including cheap pump.fun-style mints, typically
/// $0.00001+) while still catching dead/uninitialized oracle data.
const MIN_SANE_RESERVE_PRICE_USD: f64 = 1e-9;

/// How many resync cycles of residual history each curated symbol's
/// `factor_residual::RollingWindow` keeps -- at this mode's ~30s resync
/// cadence (`factor_graph::MAX_FACTOR_STALENESS_SECS`), 30 samples is
/// real recent history (~15 minutes), not an arbitrary count, but still
/// a starting value, not calibrated against any real trading session.
/// Also the real history `real_expected_holding_period_years`'s AR(1)
/// half-life fit draws on -- no separate window for that.
const RESIDUAL_WINDOW_CAPACITY: usize = 30;

/// How many *leading* (smallest-eigenvalue, per `factor_graph::
/// StructuralFactors`'s own ascending convention) structural factors
/// `update_residual_history_and_get_current` projects each symbol's
/// return onto -- module-scoped (not function-local) so trade type 1's
/// basket construction (`build_directional_basket_for`) can share the
/// exact same `k`, keeping the residual/z-score machinery and the
/// hedge-basket construction consistent about how many factors "the
/// model" actually has. Starting value, not calibrated against any real
/// trading session.
const RESIDUAL_FACTOR_COUNT: usize = 3;

/// The Phoenix perp market shorted as trade type 3's (dispersion) market-
/// index hedge -- SOL is the real, most direct proxy for the leading
/// structural factor (`INSTRUCTIONS.md`'s "Eigenvector 1 (Market/Beta
/// Factor)"), and the only market this codebase's own basis trade
/// (`perpfundingv1`) already trades live on Phoenix, so its liquidity is
/// real-proven, not assumed.
const DISPERSION_INDEX_SYMBOL: &str = "SOL";

/// Real curated symbol/mint pairs Phase 5's factor graph evaluates --
/// `trade_universe_config::TRADE_UNIVERSE`, every real mint with a
/// reserve on Kamino's *or* Solend's main market (build-time,
/// database-derived, deduped by mint; see that module's doc comment),
/// excluding USDC itself (real, live-confirmed to appear in the raw
/// Kamino main-market reserve list -- it's this strategy's own
/// settlement currency, not a risky asset with its own residual
/// dynamics; a long/short leg landing on it would degenerate to a
/// USDC->USDC swap). Broadened (this session) from the original
/// 6-symbol `symbol_mint_config::SYMBOL_MINT_MAP` -- that list's
/// Phoenix+Velocity dual-perp-coverage gate was never actually load
/// -bearing here: this trade's legs are real Kamino *or* Solend
/// deposit/borrow (protocol picked per-leg at runtime, see
/// `LendingProtocol`), no perp hedge, so the gate only ever accidentally
/// capped the universe at whatever `perpfundingv1`/`leveragedloopv1`
/// happened to need for a different strategy. Real count is in the low
/// hundreds at most (Kamino ∪ Solend main markets), not thousands:
/// `factor_graph::structural_factors` is O(n^3) Jacobi plus an O(n^2)
/// real-router liquidity query per pair (`build_live_factor_graph`) --
/// fine at this scale, not fine for the thousands of mints
/// `TradeRouter`'s own node universe actually has.
fn curated_symbols() -> impl Iterator<Item = (&'static str, AccountId, u8)> {
    trade_universe_config::TRADE_UNIVERSE
        .iter()
        .filter(|raw| raw.mint != crate::brain::multimodelv1::configuration::MINT_USDC.to_bytes())
        .map(|raw| (raw.symbol, account_id_from_pubkey(&Pubkey::new_from_array(raw.mint)), raw.decimals))
}

#[derive(Debug)]
struct KeypairExtra {
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}

/// Build the 3-tier `trader::router::Router` from the build-time
/// `router_config`/`router_pools_config` snapshot embedded from
/// `prefetch.db`. Must run after the host has resolved accounts (i.e.
/// from `on_load`, like `DexState::new`) since `account_id_from_pubkey`
/// panics on an unknown pubkey. Private, per-mode copy, same as every
/// other mode's own -- see this module's doc comment on why small
/// per-mode boilerplate beats cross-module sharing here.
fn build_liquidity_router() -> router::Router {
    let cfg = &router_config::ROUTER_CONFIG;
    let mut r = router::Router::new(cfg.token_count, cfg.lambda);
    for core_mint in cfg.core_mints {
        r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(core_mint)));
    }
    let mut pools = Vec::with_capacity(router_pools_config::ROUTER_POOLS.len());
    for p in router_pools_config::ROUTER_POOLS {
        let token_a = r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(p.mint_a)));
        let token_b = r.register_mint(account_id_from_pubkey(&Pubkey::new_from_array(p.mint_b)));
        pools.push(router::Pool {
            token_a,
            token_b,
            liquidity_usd: p.liquidity_usd,
            price_a_to_b: p.price_a_to_b,
        });
    }
    r.rebuild_partitions(&pools);
    r
}

#[derive(Debug, Default)]
pub(crate) struct State {
    last_slot: Slot,
    o_rc_keypair: Option<KeypairExtra>,
    o_dex: Option<DexState>,
    /// Built once in `on_load` from the build-time router snapshot, not a
    /// live per-tick structure -- see `TradeRouter::from_router`'s doc
    /// comment (`router` below is the live one).
    o_liquidity_router: Option<router::Router>,
    router: TradeRouter,
    /// Per-account freshness gate for `router`'s incremental maintenance
    /// -- see this module's doc comment and `is_newer_than_low_latency`.
    m_account_slot: HashMap<AccountId, Slot>,
    /// Real UNIX-epoch seconds `run_factor_resync` last completed at --
    /// `None` until the first resync (`factor_graph::check_staleness`'s
    /// own "never resynced is always stale" rule applies before then).
    last_full_resync_secs: Option<i64>,
    /// Most recent real `factor_graph::structural_factors` result --
    /// read-only diagnostic state for now (sub-phase 5b logs it; no
    /// decision logic consumes it yet).
    o_factors: Option<factor_graph::StructuralFactors>,
    /// Gates `run_factor_resync` entirely -- `false` (the default) means
    /// `evaluate()` never computes real factors at all, matching this
    /// codebase's "nothing new happens without an explicit trigger"
    /// ethos. Set by `TriggerEnableFactorLogging`.
    factor_logging_enabled: bool,
    /// This mode's own, fully independent (`id=2`) Kamino obligation for
    /// the pair trade -- see [`PAIR_KAMINO_OBLIGATION_ID`]'s doc comment
    /// for why `id=2`, not `id=0`/`id=1`.
    o_pair_kamino_position: Option<kamino::KaminoPosition>,
    /// This mode's own, fully independent (`id=1`) Solend obligation for
    /// the pair trade -- see [`PAIR_SOLEND_OBLIGATION_ID`]'s doc comment
    /// for why `id=1`, not `id=0`.
    o_pair_solend_position: Option<solend::SolendPosition>,
    /// Gates `run_pair_trade_cycle` -- `false` (the default) means real
    /// pair-trade decisions/execution never run at all, same
    /// "nothing without an explicit trigger" ethos as
    /// `factor_logging_enabled`. Set by `TriggerEnablePairTrading`.
    /// Implies factor logging too (a pair trade needs real factors to
    /// compute residuals against) -- see `evaluate()`.
    pair_trading_enabled: bool,
    /// Trade type 1's own, fully independent (`id=3`) Kamino obligation --
    /// see [`DIRECTIONAL_KAMINO_OBLIGATION_ID`]'s doc comment.
    o_directional_kamino_position: Option<kamino::KaminoPosition>,
    /// Trade type 1's own, fully independent (`id=2`) Solend obligation --
    /// see [`DIRECTIONAL_SOLEND_OBLIGATION_ID`]'s doc comment.
    o_directional_solend_position: Option<solend::SolendPosition>,
    /// Gates `run_directional_trade_cycle` -- same "nothing without an
    /// explicit trigger" ethos as `pair_trading_enabled`. Set by
    /// `TriggerEnableDirectionalTrading`/`TriggerCloseDirectionalPosition`
    /// (the close trigger also implies this, so a close request is never
    /// silently ignored just because the enable trigger was never sent
    /// this process's lifetime).
    directional_trading_enabled: bool,
    /// The human-specified long target for trade type 1's next open pass
    /// -- set by `TriggerEnableDirectionalTrading`, cleared on every close
    /// (forced or human-requested). `None` means "nothing to open," not
    /// "nothing open" -- `current_open_directional` is the source of
    /// truth for what's actually open, re-derived from real on-chain
    /// state, same discipline as `current_open_pair`.
    o_directional_target_mint: Option<AccountId>,
    /// Set by `TriggerCloseDirectionalPosition`, consumed (and cleared)
    /// the next time `run_directional_trade_cycle`'s close-pass runs --
    /// whether or not anything was actually open to close, so a stale
    /// request from before the position closed on its own (stop-loss/
    /// borrow-gate) never lingers to affect a future, unrelated open.
    o_directional_close_requested: bool,
    /// Trade type 3's (dispersion) own Phoenix trader/margin state --
    /// this mode's first Phoenix usage; a fresh instance with its own
    /// wallet authority, same reasoning `perpfundingv1`/`phoenixperpsv1`
    /// each keep their own separate instance rather than sharing one (see
    /// `dex::phoenix::mod`'s own doc comment). Subscribed at `on_load`
    /// via `PhoenixState::new_and_subscribe`; the trader-account PDA
    /// itself needs `apply_authority` once the real wallet is known (see
    /// the `Wallet` message handler).
    o_phoenix: Option<PhoenixState>,
    /// Gates `run_dispersion_trade_cycle` -- unlike `pair_trading_enabled`/
    /// `directional_trading_enabled`, entry itself is automated once this
    /// is `true` (see `dispersion_basket::should_enter_dispersion`), not a
    /// human-specified target -- this flag only arms the automated
    /// decision loop, same "nothing without an explicit trigger" ethos
    /// applied to *arming* rather than to each individual open. Set by
    /// `TriggerEnableDispersionTrading`/`TriggerCloseDispersionPosition`
    /// (the close trigger also implies this, same reasoning as
    /// `directional_trading_enabled`'s own doc comment).
    dispersion_trading_enabled: bool,
    /// One-way latch: `true` once `graph::all_subscriptions_acked()` has
    /// been observed `true` at least once since this process started.
    ///
    /// Real, live-confirmed gap (2026-09-04): every trading decision in
    /// this file previously gated only on `State::wallet()`, which
    /// returns `Some` the instant the keypair loads -- saying nothing
    /// about whether this wallet's *real, existing* positions (curated-
    /// symbol ATAs, subscribed at `on_load`, per `ata_subscribe_request`'s
    /// doc comment) have actually delivered their first live balance
    /// yet. A subscription *request* going out at `on_load` is not the
    /// same as the *data* having arrived -- confirmed the same gap,
    /// repeatedly, all night for pool/tick-array data (see `planner::
    /// HopFailure`'s doc comment). A wallet carrying a real position from
    /// an earlier, interrupted process (exactly tonight's stranded-token
    /// incidents) would read as 0 to a trading decision made before its
    /// ATA subscription had delivered anything, risking a redundant open
    /// against a position the bot just doesn't know about yet.
    ///
    /// `all_subscriptions_acked()` is a coarse, process-wide signal (every
    /// pool/tick-array subscription too, not just this wallet's own ATAs)
    /// and flips back to `false` any time a new subscription is sent
    /// after being acked -- unsuitable as a live, continuously-rechecked
    /// gate. Latching it (checked once, then never un-set) gives a real
    /// "the startup burst is done" signal without re-blocking trading
    /// every time a routine new pool subscription goes out hours into a
    /// run. See `evaluate()`'s own gating of `run_factor_resync`/
    /// `execute_arbitrage_opportunity` on this.
    positions_loaded: bool,
    /// Set by `TriggerCloseDispersionPosition`, consumed (and cleared) the
    /// next time `run_dispersion_trade_cycle`'s close-pass runs -- same
    /// shape/reasoning as `o_directional_close_requested`.
    o_dispersion_close_requested: bool,
    /// The aggregate cross-sectional idiosyncratic-volatility series --
    /// one real number per resync cycle (the mean of every warmed-up
    /// curated symbol's own `m_residual_history` stdev), pushed into the
    /// *same* `factor_residual::RollingWindow` type `m_residual_history`
    /// uses per-symbol, reused here for a second, aggregate-level series.
    /// Its own z-score is dispersion's only automated entry/exit signal
    /// (`dispersion_basket::should_enter_dispersion`/`should_exit_
    /// dispersion`).
    m_dispersion_history: factor_residual::RollingWindow,
    /// Which curated mints are currently held as dispersion's open long
    /// basket -- genuinely new state this trade type needs that no other
    /// trade type in this file does: every other trade type's open legs
    /// are real lending-protocol obligations, re-derivable from a single
    /// on-chain account read on restart (`current_open_pair`/`current_
    /// open_directional`). Dispersion's long legs are plain spot holdings
    /// (`execute_spot_leg`, no lending deposit) -- there's no obligation
    /// account to read back, so `current_open_dispersion` derives this
    /// from real non-dust wallet SPL balances instead, and this field is
    /// only a same-process cache of that derivation's last result (safe
    /// to lose on restart, unlike `o_directional_target_mint`, since
    /// restart re-derives it from real chain state either way).
    dispersion_basket_legs: Vec<(&'static str, AccountId)>,
    /// `true` once the first real resync has run while `pair_trading_
    /// enabled` -- guards `apply_residual_snapshot` against a replayed
    /// snapshot arriving late (see that method's doc comment for why
    /// this dedicated flag, not a map-emptiness check, is what's needed
    /// once replay is split across multiple small per-mint messages).
    /// Never reset back to `false` -- once real live history exists,
    /// nothing should ever fall back to trusting a replay again this
    /// process's lifetime.
    residual_history_live: bool,
    /// Last real Kamino oracle price seen per curated symbol's mint --
    /// needed to compute a real period-over-period return each resync
    /// cycle (`run_pair_trade_cycle`). `None`/absent means "no prior
    /// observation yet," not "price is zero."
    m_last_price_usd: HashMap<AccountId, f64>,
    /// Real rolling residual history per curated symbol's mint -- the
    /// actual state `factor_residual::RollingWindow::zscore` needs;
    /// genuinely new time-series state, not re-derivable from a single
    /// on-chain snapshot the way `current_open_pair` below is.
    m_residual_history: HashMap<AccountId, factor_residual::RollingWindow>,
    /// Real, per-curated-symbol "spike vs. low-noise basket" spread
    /// history -- `residual_pct - basket_average_residual` each cycle
    /// (`pair_basket::basket_average_residual` over that cycle's shared
    /// calm-basket membership, excluding the symbol itself), pushed for
    /// *every* curated symbol with a real residual this cycle, not just
    /// whichever one is currently a spike candidate -- so warm-up
    /// (`factor_residual::MIN_SAMPLES_FOR_ZSCORE`) proceeds in the
    /// background with no extra cold-start delay once a symbol actually
    /// spikes, same reasoning `m_residual_history` itself already
    /// applies. This is the real series trade type 2's redesigned
    /// half-life gate fits AR(1) against (see `run_pair_trade_cycle`'s
    /// own doc comment for why this replaces fitting against a single
    /// leg's raw residual series -- live-confirmed negative phi, 6-for-6,
    /// on that old approach).
    m_pair_spread_history: HashMap<AccountId, factor_residual::RollingWindow>,
    /// Real previous-cycle z-score per curated symbol's mint -- needed to
    /// compute a real per-cycle `delta_zscore` for trade type 5's
    /// (Hawkes-on-eigenfactor momentum) jump-magnitude projection (see
    /// `hawkes_factor::SymbolJump`). Absent/missing means "no prior
    /// observation yet," same convention as `m_last_price_usd` -- a
    /// symbol's first-ever appearance in `m_residual_history` is
    /// correctly excluded from this cycle's jump computation rather than
    /// treated as a fabricated jump from an assumed zero baseline.
    m_last_zscore: HashMap<AccountId, f64>,
    /// Real per-factor discrete-time self-exciting jump-intensity state
    /// for trade type 5 (Hawkes-on-eigenfactor momentum) -- one
    /// `hawkes_factor::FactorIntensityState` per leading
    /// `RESIDUAL_FACTOR_COUNT` factor, same `[token][factor]`-adjacent
    /// indexing convention `factors.eigenvectors` itself uses. A `Vec`
    /// (not a fixed-size array) purely so `#[derive(Default)]` above
    /// keeps working without requiring `FactorIntensityState: Default`;
    /// real-initialized to length `RESIDUAL_FACTOR_COUNT` in `on_load`
    /// (same "junk default, immediately overwritten" pattern
    /// `m_dispersion_history`'s own doc comment documents for
    /// `RollingWindow`). See `docs/HAWKES_FACTOR_TRADE_PLAN.md` for the
    /// full design.
    m_factor_intensity: Vec<hawkes_factor::FactorIntensityState>,
    /// Real rolling history of each factor's own recursed `lambda` --
    /// gives `should_open_hawkes`/`should_close_hawkes_on_decay` a real
    /// `sigma_lambda` to gate on (the same z-scored-gate-on-a-rolling-stat
    /// shape every other entry/exit gate in this codebase uses), same
    /// `factor_residual::RollingWindow` type/capacity `m_residual_history`
    /// uses, just one window per factor instead of per curated symbol.
    /// Real-initialized to length `RESIDUAL_FACTOR_COUNT` in `on_load`,
    /// same reasoning as `m_factor_intensity` above.
    m_factor_intensity_lambda_history: Vec<factor_residual::RollingWindow>,
    /// This cycle's real per-factor jump list -- the same data
    /// `update_residual_history_and_get_current`'s own background block
    /// already builds to compute `hawkes_factor::factor_jump_magnitude`,
    /// persisted here (rather than dropped as a local) so
    /// `run_hawkes_trade_cycle` -- called separately, later in the same
    /// `run_factor_resync` -- can build a real momentum basket
    /// (`hawkes_factor::factor_jump_basket`) from the exact same jumps
    /// without recomputing them. Overwritten every cycle; stale by
    /// construction if `run_hawkes_trade_cycle` is ever called without
    /// this cycle's `update_residual_history_and_get_current` having run
    /// first (same ordering invariant every other shared-`residuals`
    /// trade type in this file already relies on).
    m_hawkes_jumps_this_cycle: Vec<Vec<hawkes_factor::SymbolJump>>,
    /// Trade type 5's own, fully independent (`id=4`) Kamino obligation --
    /// see [`HAWKES_KAMINO_OBLIGATION_ID`]'s doc comment.
    o_hawkes_kamino_position: Option<kamino::KaminoPosition>,
    /// Trade type 5's own, fully independent (`id=3`) Solend obligation --
    /// see [`HAWKES_SOLEND_OBLIGATION_ID`]'s doc comment.
    o_hawkes_solend_position: Option<solend::SolendPosition>,
    /// Gates `run_hawkes_trade_cycle` -- same "nothing without an explicit
    /// trigger" ethos as `pair_trading_enabled`/`directional_trading_
    /// enabled`. Set by `TriggerEnableHawkesTrading`/
    /// `TriggerCloseHawkesPosition` (the close trigger also implies this,
    /// same reasoning as `dispersion_trading_enabled`'s own doc comment).
    hawkes_trading_enabled: bool,
    /// Set by `TriggerCloseHawkesPosition`, consumed (and cleared) the
    /// next time `run_hawkes_trade_cycle`'s close-pass runs -- same role
    /// as `o_dispersion_close_requested`.
    o_hawkes_close_requested: bool,
    /// Which of the leading `RESIDUAL_FACTOR_COUNT` factors triggered the
    /// currently-open Hawkes basket, if any -- genuinely new state this
    /// trade type needs that no other trade type in this file does: unlike
    /// a basket's own long/short legs (re-derivable from real on-chain
    /// obligations, see `current_open_hawkes`), *which factor* justified
    /// opening it has no on-chain trace at all. `None` on a fresh restart
    /// with real legs already open (lost track of which factor) is treated
    /// as unrecoverable -- the close-pass force-closes rather than guess.
    o_hawkes_open_factor: Option<usize>,
    /// Real wall-clock time (unix seconds) the currently-open Hawkes basket
    /// was opened -- backs the [`HAWKES_MAX_HOLDING_CYCLES`] hard cap.
    /// `None` means nothing is currently open (or, same as
    /// `o_hawkes_open_factor`, a restart lost track of it).
    m_hawkes_opened_at_secs: Option<i64>,
    /// Real USDC reserved by an earlier trade type's open-pass *this same
    /// resync cycle* -- reset to 0.0 at the top of every
    /// `run_factor_resync`, incremented by each trade type right before
    /// it actually commits to opening (see `available_usdc_value`'s own
    /// doc comment for why this exists: multiple trade types' open-passes
    /// run sequentially against the same real balance snapshot within one
    /// cycle, before any of their transactions land on-chain and change
    /// it).
    m_usdc_reserved_this_cycle: f64,
    /// Guards against a real, live-confirmed bug (2026-09-07): a basket
    /// open commits multiple legs in one synchronous cycle (one spike/
    /// target leg plus N basket legs), and each leg independently checks
    /// its shared obligation's `registered()` before dispatching --
    /// `registered()` cannot flip true mid-cycle (it only updates from a
    /// real on-chain account read arriving on a later cycle), so every
    /// leg that still saw `registered()==false` re-ran the *same*
    /// `bootstrap_*_obligation` call, appending duplicate
    /// `CreateAccountWithSeed`/`InitObligation` instructions into the
    /// same accumulated transaction -- live-confirmed on-chain this
    /// always reverts the instant a second identical
    /// `CreateAccountWithSeed` for the same seed appears in one
    /// transaction ("already in use"). Reset to `false` alongside
    /// `m_usdc_reserved_this_cycle` at the top of every
    /// `run_factor_resync` (same "only ever see this same cycle's own
    /// state" reasoning); each `bootstrap_*_obligation` function checks
    /// its own flag first and only actually appends instructions once
    /// per cycle, no matter how many legs ask for it.
    m_pair_kamino_bootstrap_attempted_this_cycle: bool,
    m_pair_solend_bootstrap_attempted_this_cycle: bool,
    m_directional_kamino_bootstrap_attempted_this_cycle: bool,
    m_directional_solend_bootstrap_attempted_this_cycle: bool,
    m_hawkes_kamino_bootstrap_attempted_this_cycle: bool,
    m_hawkes_solend_bootstrap_attempted_this_cycle: bool,
    /// Guards against a real, live-confirmed bug (2026-09-07), same root
    /// cause as the bootstrap-dedup guards above but one step later: once
    /// an obligation is registered, a basket open still commits *multiple
    /// legs'* real deposit/borrow instructions in one synchronous cycle
    /// against the *same shared* obligation. Each leg's `refresh_all_
    /// reserves`/`refresh_obligation` call builds its on-chain reserve-
    /// account list from local, already-parsed obligation state -- fine
    /// across cycles (kept fresh by the real `on_account` low-latency
    /// dispatch), but stale *within* one cycle relative to an earlier
    /// leg's transaction sent moments before, since that transaction
    /// hasn't landed and been read back yet. Live-confirmed on-chain: leg
    /// 2's `RefreshObligation` gets built listing the *same* (stale)
    /// reserve count leg 1 saw, but by the time leg 2 actually executes,
    /// leg 1 has already landed and the *real* obligation now holds one
    /// more reserve than leg 2's instruction accounts for --
    /// `NotEnoughAccountKeys`, every time. Reset alongside the bootstrap
    /// flags above; each `open_*_long_leg_*`/`open_*_short_leg_*` pair
    /// (sharing one obligation) checks its own flag before committing to
    /// a real deposit/borrow this cycle, and only the first leg per
    /// cycle actually sends one -- the rest wait for next cycle's fresh,
    /// confirmed state.
    m_pair_kamino_leg_action_this_cycle: bool,
    m_pair_solend_leg_action_this_cycle: bool,
    m_directional_kamino_leg_action_this_cycle: bool,
    m_directional_solend_leg_action_this_cycle: bool,
    m_hawkes_kamino_leg_action_this_cycle: bool,
    m_hawkes_solend_leg_action_this_cycle: bool,
    /// Pools `execute_spot_leg`'s most recent route touched, not yet
    /// confirmed sent -- `evaluate()`'s drain-and-send tail marks these
    /// on cooldown if the real send fails. Same field/role as every
    /// other real bot mode's identically-named field (see
    /// `leveragedloopv1::state::State::pending_route_pools`'s doc
    /// comment for the real `transactionprocessor::send()`-rejection gap
    /// this closes) -- sub-phase 5a/5b never needed this since neither
    /// ever queued a real transaction.
    pending_route_pools: Vec<AccountId>,
    /// In-flight, genuinely cross-transaction hop chains -- keyed by the
    /// real signature of whichever hop was most recently sent. See
    /// [`PendingHopChain`]'s own doc comment for the full design; driven
    /// entirely by `mid_on_tx`, never polled.
    m_pending_hop_chains: HashMap<Signature, PendingHopChain>,
    /// Trade type 4 (arbitrage) -- ported from `arbv1::state::State`
    /// verbatim (same fields, same roles), not factored into shared
    /// `src/trader` code since `arbv1` itself keeps this logic in its own
    /// `state.rs`. Unlike every other trade type in this file, arbitrage
    /// has no `*_trading_enabled` gate -- it's always active once a real
    /// wallet is loaded, matching `arbv1`'s own real behavior (see this
    /// mode's own module doc comment / the design plan for why: no
    /// factor-model dependency, needs to react fast, and the user
    /// explicitly chose "always on like arbv1" over adding a trigger).
    ///
    /// Lifetime count of real arbitrage sends -- mirrors `arbv1::state::
    /// State::tx_count`'s hard, never-reset cap (`evaluate()` stops
    /// building/sending once this exceeds 10) -- a circuit breaker, not a
    /// cooldown.
    arb_tx_count: usize,
    /// Commits seen since this process started -- mirrors `arbv1::state::
    /// State::slot_delta_since_start`'s ~4-minute warm-up gate
    /// (`evaluate()` won't attempt a real send until this reaches 20),
    /// incremented once per `CommitHook::finish`.
    arb_slot_delta_since_start: Slot,
    /// Set by `detect_arbitrage_opportunity` (called from both
    /// `low_latency` and `CommitHook::finish`, mirroring `arbv1::state`'s
    /// own dual call sites) whenever a real profitable cycle is found;
    /// consumed (via `.take()`) by `execute_arbitrage_opportunity` from
    /// `evaluate()`. Mirrors `arbv1::state::State::o_pending_opportunity`.
    o_arb_pending_opportunity: Option<planner::ArbitrageOpportunity>,
    /// Lifetime count of `planner::find_opportunity` calls -- mirrors
    /// `arbv1::state::State::find_opportunity_call_count`, same
    /// ground-truth-vs-throttled-log-line reasoning as that field's own
    /// doc comment.
    arb_find_opportunity_call_count: u64,
    /// Temporary, standalone manual-cleanup tool -- see
    /// `CustomMessageInbound::TriggerSweepMint`'s doc comment. Consumed
    /// (via `.take()`) by `sweep_requested_mint`, called unconditionally
    /// from `evaluate()` every tick, independent of every other trade
    /// type's own enabled flag -- this is a one-shot human request, not
    /// part of any trade type's own decision loop.
    o_sweep_mint_requested: Option<(AccountId, AccountId)>,
}

/// The remaining hops of a route that didn't fit in one transaction,
/// waiting on real on-chain confirmation of the hop most recently sent
/// (tracked by the `Signature` this struct is keyed under in
/// `State::m_pending_hop_chains`) before the next one is sent.
///
/// This replaces the earlier "send hop 0 alone, decline everything else"
/// fallback (see `execute_spot_leg`'s own doc comment for that incident).
/// That fix was deliberately conservative because this codebase has no
/// *synchronous* host primitive to wait for a transaction's confirmation
/// (`wit/component.wit`'s `transactionprocessor` interface is
/// fire-and-forget from the guest's side) -- but `mid_on_tx` IS a real,
/// already-wired *asynchronous* delivery of exactly that confirmation --
/// `phoenixperpsv1::state`'s own `m_sig`/`mid_on_tx` proves the real
/// pattern: `Wallet::assemble`/`drain_and_send`
/// return each transaction's real `Signature` at send time, before any
/// confirmation; `mid_on_tx` later matches `Signature::from(*tx.
/// signature)` against tracked sends to learn the real landed/failed
/// result). Reusing that pattern here turns "wait for confirmation" from
/// an impossible synchronous call into a real, event-driven state
/// machine spanning multiple `evaluate()`/`mid_on_tx` cycles: send hop K
/// alone, register hops `K+1..` here under hop K's signature, and only
/// send hop `K+1` once `mid_on_tx` reports hop K's signature landed for
/// real -- so every hop after the first is only ever sent once its
/// prerequisite balance is *proven* to exist on-chain, not assumed.
///
/// Residual risk this does NOT eliminate: if a later hop's own send then
/// fails (unrelated liquidity/slippage/cooldown reasons, not a
/// dependency-ordering bug), `owner` is left holding a real balance of
/// whatever intermediate mint the last-landed hop produced -- an
/// untracked mint, invisible to this trade type's own curated-symbol
/// balance scan (`current_open_dispersion`). `mid_on_tx`'s failure arm
/// logs this loudly (mint + owner) rather than silently dropping it, but
/// does not attempt automatic recovery -- same fail-loud-not-fail-silent
/// discipline as every other real send failure in this file, and a much
/// better outcome than the original bug (2 of 3 batched transactions
/// failing with no visibility at all).
#[derive(Debug, Clone)]
struct PendingHopChain {
    /// Hops still to send, in order; `hops[0]` is sent next once the
    /// tracked signature confirms landed. Can legitimately be empty --
    /// `advance_pending_hop_chain` still registers a chain entry (under
    /// the just-sent hop's own signature) when that hop was the route's
    /// last one, purely so `CommitHook::start`'s expiry sweep below can
    /// still detect and log a missed final-hop confirmation. Never index
    /// this directly in code that must also run when it's empty -- see
    /// `produced_mint` below for what that code should use instead (a
    /// real, live-confirmed panic risk this field's addition fixed:
    /// `chain.hops[0]` on an empty chain aborts the whole guest process).
    hops: Vec<pricegraph::Hop>,
    owner: AccountId,
    /// Pool of the hop this chain entry is keyed by the signature of --
    /// cooled down if that signature is reported failed.
    sent_pool_id: AccountId,
    /// The mint the tracked signature's own hop was expected to *spend*
    /// -- real, live-confirmed bug fixed here (2026-09-04): the first
    /// version of this expiry-driven invalidation only cleared
    /// `produced_mint` (below), missing this side entirely. A real
    /// dispersion close-pass then kept re-selling an already-emptied
    /// position 5 times in a row, each one correctly failing on-chain
    /// (`insufficient funds`) -- the one genuine `mid_on_tx` miss (the
    /// hop that actually landed and drained this exact mint) never got
    /// invalidated on the side that mattered, since nothing ever
    /// invalidated the *input* side of a hop, only its output.
    consumed_mint: AccountId,
    /// The mint the tracked signature's own hop was expected to produce
    /// -- unlike `hops.first().map(|h| h.input_mint)`, this is always
    /// available, even once `hops` is empty (the route's final hop).
    /// `CommitHook::start`'s expiry sweep invalidates both this and
    /// `consumed_mint` together (see `TokenDatabase::invalidate`'s own
    /// doc comment for the real incident this closes -- a real send can
    /// land and change either mint's balance while `mid_on_tx` never
    /// reports it, so neither side's cached balance stays trustworthy
    /// once the tracked signature's real outcome is unknown).
    produced_mint: AccountId,
    /// Real slot the tracked hop was sent at -- `CommitHook::start` expires
    /// entries older than [`HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS`], same real
    /// gap `phoenixperpsv1::state`'s own `SIGNATURE_EXPIRY_SLOTS`/`m_sig`
    /// cleanup covers (a signature `mid_on_tx` never reports on at all --
    /// e.g. the transaction's blockhash simply expired unlanded -- must
    /// not pin a chain in memory, and a live-money position, forever).
    sent_slot: Slot,
    /// Real balance of `produced_mint` for `owner`, read at the moment
    /// this hop was sent (before any confirmation, landed or not).
    ///
    /// Real, live-confirmed gap (2026-09-05): `mid_on_tx`'s push-based
    /// confirmation stream is the *only* way this bot ever learns a
    /// tracked signature's outcome -- the host's `transactionprocessor`
    /// WIT interface exposes no way to directly poll a specific
    /// signature's status, only fire-and-forget submission. Confirmed
    /// live, repeatedly: a hop can genuinely land on-chain (verified
    /// independently via `solana confirm`/real balance change) while
    /// `mid_on_tx` never reports it, so `CommitHook::start`'s expiry
    /// sweep gives up and the resulting balance sits stranded in
    /// whatever mint the route happened to pass through -- invisible to
    /// any trade type whose own candidate/position scan doesn't happen
    /// to include that specific mint (e.g. a bridge/pass-through mint
    /// that isn't itself a curated trading candidate). `CommitHook::
    /// start` compares the real, current balance against this baseline
    /// before giving up -- a real increase is strong independent
    /// evidence the hop actually landed even though the dedicated
    /// confirmation never arrived, letting the chain advance instead of
    /// stranding.
    produced_mint_balance_before: u64,
}

impl PendingHopChain {
    /// This chain's real, ultimate destination mint -- `hops.last()`'s
    /// output if any hops remain, else `produced_mint` (the just-sent
    /// hop's own output, since that hop was the route's last one). Used
    /// by [`State::execute_spot_leg`]'s own pending-chain guard to tell
    /// whether an in-flight chain is already headed toward a given
    /// target, not just [`Self::produced_mint`] (which is only that
    /// *next* hop's output, an intermediate mint for any chain with
    /// hops still remaining).
    fn final_target_mint(&self) -> AccountId {
        self.hops.last().map_or(self.produced_mint, |h| h.output_mint)
    }
}

/// How many slots a [`PendingHopChain`] entry is allowed to wait for
/// `mid_on_tx` to report its tracked signature's real result before
/// `CommitHook::start` gives up on it -- same value/reasoning as
/// `phoenixperpsv1::state::SIGNATURE_EXPIRY_SLOTS` (roughly 5 commits'
/// worth, generous vs. typical confirmation latency).
const HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS: Slot = 300;

impl State {
    fn wallet(&self) -> Option<AccountId> {
        let ke = self.o_rc_keypair.as_ref()?;
        Some(ke.account_id)
    }
}

pub(crate) struct StateHelper<'a> {
    pub(crate) graph: &'a mut Graph,
    pub(crate) nonce: &'a mut u32,
    pub(crate) o_commit_slot: Option<Slot>,
    pub(crate) state: &'a mut State,
    pub(crate) wallet: &'a mut Wallet,
    pub(crate) configuration: &'a mut Configuration,
    pub(crate) q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
}

/// Which lending protocol backs a pair-trade leg -- Solend or Kamino,
/// whichever a given call site's rate/protocol selection picked (see
/// [`StateHelper::best_borrow_apy`]/[`StateHelper::best_supply_apy`]).
/// Marginfi execution is deferred (not yet ported into multimodelv1's pair
/// trade); ported from `perpfundingv1::state::LendingProtocol`'s 3-arm
/// version with the Marginfi arm dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LendingProtocol {
    Solend,
    Kamino,
}

impl<'a> StateHelper<'a> {
    /// Subscribe to relevant accounts. Same shape as every other mode's
    /// `on_load` -- `DexState::new()` queues (doesn't send) its startup
    /// subscription burst; `CommitHook::finish` below drains it paced,
    /// per slot, not all at once (see `SubscriptionQueue`'s own doc
    /// comment for the real, live-observed motivation for that pacing).
    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        assert!(self.state.o_dex.replace(DexState::new().expect("dex state")).is_none());
        assert!(self.state.o_liquidity_router.replace(build_liquidity_router()).is_none());
        self.state.router = TradeRouter::from_router(self.state.o_liquidity_router.as_ref().unwrap());
        assert!(self.state.o_pair_kamino_position.replace(kamino::KaminoPosition::default()).is_none());
        assert!(self.state.o_pair_solend_position.replace(solend::SolendPosition::default()).is_none());
        assert!(self.state.o_directional_kamino_position.replace(kamino::KaminoPosition::default()).is_none());
        assert!(self.state.o_directional_solend_position.replace(solend::SolendPosition::default()).is_none());
        assert!(self.state.o_phoenix.replace(PhoenixState::new_and_subscribe(self.graph).expect("phoenix state")).is_none());
        self.state.m_dispersion_history = factor_residual::RollingWindow::new(RESIDUAL_WINDOW_CAPACITY);
        // Trade type 5 (Hawkes-on-eigenfactor momentum) -- see
        // `m_factor_intensity`'s own doc comment for why this real
        // initialization happens here rather than via `#[derive(Default)]`.
        self.state.m_factor_intensity = vec![hawkes_factor::FactorIntensityState::starting(0.0); RESIDUAL_FACTOR_COUNT];
        self.state.m_factor_intensity_lambda_history =
            (0..RESIDUAL_FACTOR_COUNT).map(|_| factor_residual::RollingWindow::new(RESIDUAL_WINDOW_CAPACITY)).collect();
        self.state.m_hawkes_jumps_this_cycle = vec![Vec::new(); RESIDUAL_FACTOR_COUNT];
        assert!(self.state.o_hawkes_kamino_position.replace(kamino::KaminoPosition::default()).is_none());
        assert!(self.state.o_hawkes_solend_position.replace(solend::SolendPosition::default()).is_none());
        if let Some(x) = self.state.o_rc_keypair.as_ref() {
            self.configuration.set(&x.rc_keypair);
        }
        log_info!("multimodelv1: on_load complete");
    }

    fn record_low_latency_slot(&mut self, account_id: AccountId, slot: Slot) {
        self.state.m_account_slot.insert(account_id, slot);
    }

    /// Gate for `CommitHook::on_account`'s ~12s rooted stream only --
    /// same mechanism/reasoning as `arbv1::state`'s identically-named
    /// method (see this module's doc comment).
    fn is_newer_than_low_latency(&self, account_id: AccountId, slot: Slot) -> bool {
        match self.state.m_account_slot.get(&account_id) {
            Some(&last) => slot > last,
            None => true,
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        while let Some(ta) = llap.token() {
            self.wallet.token_mut().on_token(ta, false);
            if let Some(dex) = self.state.o_dex.as_mut() {
                _ = dex.on_token(ta);
                // Incrementally refresh just this pool's router edges --
                // see this module's doc comment for why sub-phase 5a
                // omitted this and why it matters now.
                dex.refresh_token_router(ta.id, &mut self.state.router);
            }
        }
        let zero = [];
        let mut account_count = 0u32;
        while let Some(account) = llap.account() {
            account_count += 1;
            let d = account.body.unwrap_or(&zero);
            self.record_low_latency_slot(account.header.accountid, account.header.slot);
            self.wallet.on_account(account.header, d);
            // Real, live-confirmed bug fixed this session: this dispatch
            // was entirely missing, meaning the pair trade's own Kamino
            // obligation could never register from a real on-chain read --
            // `registered()` would stay false forever, so an open attempt
            // would keep re-bootstrapping (which fails on-chain the second
            // time) instead of ever depositing/borrowing. Unconditional,
            // same as `wallet.on_account` right above -- not gated behind
            // the `is_newer_than_low_latency` freshness check below, which
            // exists for `dex`/`router` price-graph consistency, not
            // real account state (matches `leveragedloopv1`'s own
            // `kamino_position.on_account` call, the proven precedent this
            // was ported from).
            if let Some(pos) = self.state.o_pair_kamino_position.as_mut() {
                pos.on_account(account.header, d);
            }
            if let Some(pos) = self.state.o_pair_solend_position.as_mut() {
                pos.on_account(account.header, d);
            }
            // Same unconditional dispatch, same reasoning -- trade type
            // 1's obligations get the fix from day one instead of
            // repeating the pair trade's own original bug.
            if let Some(pos) = self.state.o_directional_kamino_position.as_mut() {
                pos.on_account(account.header, d);
            }
            if let Some(pos) = self.state.o_directional_solend_position.as_mut() {
                pos.on_account(account.header, d);
            }
            // Same unconditional dispatch -- trade type 5's (Hawkes)
            // obligations get the fix from day one too.
            if let Some(pos) = self.state.o_hawkes_kamino_position.as_mut() {
                pos.on_account(account.header, d);
            }
            if let Some(pos) = self.state.o_hawkes_solend_position.as_mut() {
                pos.on_account(account.header, d);
            }
            // Same unconditional dispatch -- trade type 3's (dispersion)
            // Phoenix trader/market state gets the fix from day one too.
            if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                phoenix.on_account(account.header, d);
            }
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(account.header, d);
                dex.refresh_account_router(account.header.accountid, &mut self.state.router);
            }
        }
        // Trade type 4 (arbitrage) -- reacts on this ~400ms processed
        // stream, not just the ~12s commit path (`CommitHook::finish`
        // below also calls this) -- mirrors `arbv1::state`'s own dual
        // call sites exactly (same `account_count > 0` gate, same
        // reasoning: skip the search on an empty batch).
        if account_count > 0 {
            self.detect_arbitrage_opportunity();
        }
    }

    /// Real, asynchronous transaction-confirmation delivery -- drives
    /// [`PendingHopChain`]'s state machine forward. Matches each landed
    /// transaction's real `Signature` against `state.m_pending_hop_
    /// chains`; anything not tracked there (ordinary `drain_and_send`
    /// batches, Kamino/Solend/Phoenix instructions, etc.) is ignored --
    /// this mode still doesn't need general transaction-landing latency
    /// bookkeeping (see this module's doc comment), only this one
    /// specific real use.
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while let Some((tx, result)) = transaction_list.transaction() {
            let signature = Signature::from(*tx.signature);
            let Some(chain) = self.state.m_pending_hop_chains.remove(&signature) else { continue };
            match result {
                Ok(slot) => self.advance_pending_hop_chain(signature, slot, chain),
                Err(e) => {
                    self.state.router.mark_pool_cooldown(chain.sent_pool_id, planner::POOL_COOLDOWN_SLOTS);
                    log_error!(
                        "multimodelv1: hop chain -- tracked tx {signature} FAILED on-chain: {e:?} -- {} hop(s) abandoned (cooling down pool {}); owner={} likely still holds a real balance of an intermediate mint from an earlier hop that DID land -- this trade type's own balance scan doesn't track non-curated mints, so this needs manual attention",
                        chain.hops.len(), chain.sent_pool_id, chain.owner,
                    );
                }
            }
        }
    }

    /// Sends the next hop of `chain` now that `landed_signature` (the hop
    /// immediately before it) is confirmed to have really landed at
    /// `landed_slot` -- see [`PendingHopChain`]'s own doc comment for the
    /// full design. Called only from `mid_on_tx`, never polled.
    ///
    /// Real, live-confirmed incident (2026-09-04): `chain.hops[0]`'s
    /// `amount_in` is whatever the *original* route search planned,
    /// computed before the previous hop had even been sent -- it is never
    /// reconciled against what the previous hop's real, on-chain execution
    /// actually delivered. Real slippage between quote-time and
    /// execution-time (ordinary, not a bug in itself) means the wallet's
    /// real balance of `next_hop.input_mint` can come in lower than the
    /// stale planned `amount_in`, and the swap instruction is built for
    /// the literal (too-large) planned amount regardless -- confirmed
    /// live twice in a row (two different chains, two different pools),
    /// both failing on-chain with `custom program error: 0x1`
    /// ("insufficient funds") on the second hop, stranding the wallet in
    /// yet another intermediate mint each time. Re-check the real balance
    /// here and re-quote the hop against it (mirrors `execute_spot_leg`'s
    /// own real-balance-first discipline) instead of trusting the plan.
    fn advance_pending_hop_chain(&mut self, landed_signature: Signature, landed_slot: Slot, chain: PendingHopChain) {
        if chain.hops.is_empty() {
            log_warn!(
                "multimodelv1: hop chain -- final hop (tx {landed_signature}) confirmed landed at slot {landed_slot}, chain complete",
            );
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            log_error!(
                "multimodelv1: hop chain -- tx {landed_signature} confirmed landed at slot {landed_slot}, but dex state isn't ready -- dropping {} remaining hop(s); owner={} likely holds an untracked intermediate-mint balance needing manual attention",
                chain.hops.len(), chain.owner,
            );
            return;
        };
        let planned_hop = chain.hops[0].clone();
        let real_balance: u64 = self
            .wallet
            .token_mut()
            .balance(&chain.owner, &planned_hop.input_mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if real_balance == 0 {
            log_error!(
                "multimodelv1: hop chain -- tx {landed_signature} confirmed landed at slot {landed_slot}, but real balance of {} is 0 -- dropping {} remaining hop(s); owner={} likely needs manual attention (TriggerSweepMint once the real balance is visible)",
                planned_hop.input_mint, chain.hops.len(), chain.owner,
            );
            return;
        }
        self.state.router.set_current_slot(self.state.last_slot);
        let next_hop = if real_balance == planned_hop.amount_in {
            planned_hop.clone()
        } else {
            let route = pricegraph::Route { hops: vec![planned_hop.clone()] };
            match planner::reverify_route_with_exact_quotes(&route, real_balance, &self.state.router, dex) {
                Ok(corrected) => {
                    log_warn!(
                        "multimodelv1: hop chain -- real balance of {} is {} (planned {}), re-quoted before sending",
                        planned_hop.input_mint, real_balance, planned_hop.amount_in,
                    );
                    corrected.hops[0].clone()
                }
                Err(failure) => {
                    if failure.coolable {
                        self.state.router.mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    }
                    log_error!(
                        "multimodelv1: hop chain -- tx {landed_signature} confirmed landed at slot {landed_slot}, but re-quoting the next hop against the real balance ({real_balance}) failed for pool {} (coolable={}) -- dropping {} remaining hop(s); owner={} likely holds a real, untracked balance of {} needing manual attention (TriggerSweepMint)",
                        failure.pool_id, failure.coolable, chain.hops.len(), chain.owner, planned_hop.input_mint,
                    );
                    return;
                }
            }
        };
        let produced_mint_balance_before: u64 = self
            .wallet
            .token_mut()
            .balance(&chain.owner, &next_hop.output_mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        match Self::send_single_hop_as_astralane_tx(self.graph, self.wallet, chain.owner, dex, &next_hop) {
            Ok((signature, _amount_out)) => {
                self.state.pending_route_pools = vec![next_hop.pool_id];
                let remaining: Vec<pricegraph::Hop> = chain.hops[1..].to_vec();
                log_warn!(
                    "multimodelv1: hop chain -- tx {landed_signature} confirmed landed at slot {landed_slot}, sent next hop (tx {signature}), {} more to follow",
                    remaining.len(),
                );
                self.state.m_pending_hop_chains.insert(
                    signature,
                    PendingHopChain {
                        hops: remaining,
                        owner: chain.owner,
                        sent_pool_id: next_hop.pool_id,
                        consumed_mint: next_hop.input_mint,
                        produced_mint: next_hop.output_mint,
                        sent_slot: self.state.last_slot,
                        produced_mint_balance_before,
                    },
                );
            }
            Err(e) => {
                // `send_single_hop_as_astralane_tx` only surfaces
                // `TraderError` as a formatted string, not the typed
                // error itself -- string-match its `PoolNotReady` Display
                // output (its `impl Display` renders it literally as
                // "PoolNotReady") for the same coolable-vs-not-ready
                // distinction used everywhere else (see `planner::
                // HopFailure`'s doc comment).
                // Fix (2026-09-07): live-confirmed a `PoolNotReady` pool
                // can fail identically for 30+ minutes across multiple
                // restarts -- not self-correcting. `note_pool_not_ready`
                // tracks the repeated case and reports once it's crossed
                // a real threshold, so we still cool down eventually
                // instead of retrying forever.
                let is_pool_not_ready = e.contains("PoolNotReady");
                let coolable = if is_pool_not_ready {
                    self.state.router.note_pool_not_ready(next_hop.pool_id)
                } else {
                    true
                };
                if coolable {
                    if is_pool_not_ready {
                        self.state.router.mark_pool_not_ready_cooldown(next_hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    } else {
                        self.state.router.mark_pool_cooldown(next_hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    }
                }
                log_error!(
                    "multimodelv1: hop chain -- tx {landed_signature} confirmed landed at slot {landed_slot}, but sending the next hop FAILED: {e} -- {} hop(s) abandoned (cooling down pool {}: {coolable}); owner={} likely holds an untracked intermediate-mint balance ({}) needing manual attention",
                    chain.hops.len() - 1, next_hop.pool_id, chain.owner, next_hop.input_mint,
                );
            }
        }
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        match status {
            SlotStatus::Processed => {}
            SlotStatus::Rooted => {}
            SlotStatus::Confirmed => {}
            SlotStatus::FirstShredReceived => {}
            SlotStatus::Completed => {}
            SlotStatus::CreatedBank => {}
            SlotStatus::Dead => log_info!("multimodelv1: slot {slot}; status dead"),
        }
    }

    /// Real-but-read-only Phase 0 wiring (sub-phase 5b): builds a live
    /// `factor_graph::FactorGraph` for the curated symbol universe
    /// ([`curated_symbols`]), using [`factor_sizing::price_impact_bps`]
    /// (Phase 3, already-tested) as the real, depth-aware liquidity
    /// signal for each pair instead of a fabricated weight -- shallower
    /// real quoted impact means a lower liquidity weight, not the other
    /// way around. A pair with no real route at all
    /// (`price_impact_bps` returns `None`) simply gets no edge, matching
    /// `factor_graph::FactorGraph`'s own "isolated node" handling.
    ///
    /// The reference notional (`FACTOR_GRAPH_REFERENCE_WHOLE_UNITS`
    /// whole units of the *from* token) is a structural simplification,
    /// not a real trade size -- real per-symbol dollar prices differ by
    /// orders of magnitude (BTC vs. XRP), so this isn't a fair
    /// USD-notional comparison across pairs. Fine for Phase 0's
    /// clustering purpose; Phase 3 owns real execution-time sizing and
    /// should never reuse this number.
    fn build_live_factor_graph(&self) -> factor_graph::FactorGraph {
        const FACTOR_GRAPH_REFERENCE_WHOLE_UNITS: u64 = 100;
        const FACTOR_GRAPH_MAX_HOPS: usize = 3;
        let symbols: Vec<(&str, AccountId, u8)> = curated_symbols().collect();
        let mut g = factor_graph::FactorGraph::new(symbols.len());
        for i in 0..symbols.len() {
            for j in (i + 1)..symbols.len() {
                let (_, mint_i, decimals_i) = symbols[i];
                let (_, mint_j, _) = symbols[j];
                let whole_unit = 10u64.saturating_pow(decimals_i as u32);
                let amount_in = whole_unit.saturating_mul(FACTOR_GRAPH_REFERENCE_WHOLE_UNITS);
                let Some(impact_bps) = factor_sizing::price_impact_bps(
                    &self.state.router,
                    mint_i,
                    mint_j,
                    amount_in,
                    whole_unit,
                    FACTOR_GRAPH_MAX_HOPS,
                ) else {
                    continue;
                };
                // `.abs()`: a near-zero or slightly negative reading
                // (integer-truncation noise on a very deep pool, see
                // `factor_sizing`'s own doc comment) means "very deep,
                // low impact" either way -- must map to a large
                // liquidity weight, not get dropped as non-positive.
                let liquidity = 1.0 / impact_bps.abs().max(0.01);
                g.add_edge(i, j, liquidity);
            }
        }
        g
    }

    /// Runs one real resync: rebuilds the live factor graph, computes
    /// `factor_graph::structural_factors`, checks staleness, and logs
    /// the result. Then, if `pair_trading_enabled`, runs the real pair
    /// trade decision/execution cycle against these same fresh factors.
    fn run_factor_resync(&mut self) {
        let graph = self.build_live_factor_graph();
        let factors = factor_graph::structural_factors(&graph);
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let staleness = factor_graph::check_staleness(now_secs, self.state.last_full_resync_secs, &factors.eigenvectors);
        // Live-observed (2026-09-07): this bot's real router-coverage-
        // built graph regularly has ~60 connected components out of 147
        // curated tokens, not 1 -- `factors.real_eigenvectors()` (used by
        // every real consumer below) accounts for this dynamically, but
        // the raw multiplicity is still worth surfacing here for live
        // visibility into how fragmented the real graph is each cycle.
        let trivial_multiplicity = factor_graph::trivial_eigenvalue_count(&factors.eigenvalues);
        log_warn!(
            "multimodelv1: factor resync -- {} eigenvalue(s), smallest={:.6}, was_stale_before_this_resync={}, \
             trivial_eigenvalue_multiplicity={trivial_multiplicity}",
            factors.eigenvalues.len(),
            factors.eigenvalues.first().copied().unwrap_or(f64::NAN),
            staleness.is_stale(),
        );
        self.state.last_full_resync_secs = Some(now_secs);
        // Reset every real resync -- see `m_usdc_reserved_this_cycle`'s
        // own doc comment. Must happen before any trade type's own cycle
        // function runs below, so each one only ever sees reservations
        // from trade types that ran *earlier this same cycle*, never a
        // stale reservation carried over from the previous one.
        self.state.m_usdc_reserved_this_cycle = 0.0;
        // Reset alongside it -- see `m_pair_kamino_bootstrap_attempted_
        // this_cycle`'s own doc comment.
        self.state.m_pair_kamino_bootstrap_attempted_this_cycle = false;
        self.state.m_pair_solend_bootstrap_attempted_this_cycle = false;
        self.state.m_directional_kamino_bootstrap_attempted_this_cycle = false;
        self.state.m_directional_solend_bootstrap_attempted_this_cycle = false;
        self.state.m_hawkes_kamino_bootstrap_attempted_this_cycle = false;
        self.state.m_hawkes_solend_bootstrap_attempted_this_cycle = false;
        // Reset alongside them -- see `m_pair_kamino_leg_action_this_
        // cycle`'s own doc comment.
        self.state.m_pair_kamino_leg_action_this_cycle = false;
        self.state.m_pair_solend_leg_action_this_cycle = false;
        self.state.m_directional_kamino_leg_action_this_cycle = false;
        self.state.m_directional_solend_leg_action_this_cycle = false;
        self.state.m_hawkes_kamino_leg_action_this_cycle = false;
        self.state.m_hawkes_solend_leg_action_this_cycle = false;
        // Computed once, shared by both trade types -- pushing into
        // `m_residual_history` twice per resync (once per trade type)
        // would corrupt every z-score/half-life fit, so this must never
        // be called more than once per real resync regardless of how
        // many trade types are enabled.
        if self.state.pair_trading_enabled
            || self.state.directional_trading_enabled
            || self.state.dispersion_trading_enabled
            || self.state.hawkes_trading_enabled
        {
            let residuals = self.update_residual_history_and_get_current(&factors);
            if self.state.pair_trading_enabled {
                self.run_pair_trade_cycle(&factors, &residuals);
            }
            if self.state.directional_trading_enabled {
                self.run_directional_trade_cycle(&factors, &residuals);
            }
            if self.state.dispersion_trading_enabled {
                let aggregate_zscore = self.update_dispersion_signal();
                self.run_dispersion_trade_cycle(&factors, aggregate_zscore);
            }
            if self.state.hawkes_trading_enabled {
                self.run_hawkes_trade_cycle(&factors, &residuals);
            }
            self.state.residual_history_live = true;
            self.send_residual_snapshot(now_secs);
        }
        self.state.o_factors = Some(factors);
    }

    /// Sends this cycle's real residual/z-score warm-up state (`m_
    /// residual_history`/`m_last_price_usd`) to the Go host for
    /// persistence, so a restart doesn't need the same ~`RESIDUAL_WINDOW_
    /// CAPACITY`-cycle climb back to real z-scores every time -- see
    /// `trader::residual_snapshot`'s module doc comment. `run_pair_trade_
    /// cycle` must already have run this cycle (it's what actually
    /// mutates `m_residual_history`); called right after it in
    /// `run_factor_resync`, not on a separate cadence, so every sent
    /// snapshot is this cycle's real, current state, never stale by
    /// construction (freshness at *replay* time is
    /// `apply_residual_snapshot`'s own separate concern).
    ///
    /// **One small message per mint, not one big one covering the whole
    /// universe** -- real, live-confirmed reason: the host's Custom
    /// message channel has a hard `MaxValueSize` cap (`catmsg.
    /// MaxValueSize` on the Go side, 3904 bytes) with no chunking of its
    /// own; a single message covering all ~48 curated mints at close to
    /// full `RESIDUAL_WINDOW_CAPACITY` genuinely exceeds it (confirmed
    /// live this session -- crashed the whole connection with a
    /// "bad value: 3904 vs 4035" deserialize error at cycle ~14, once
    /// enough mints had accumulated real history). One mint's own
    /// worst-case payload (30 samples) is ~291 bytes, comfortably under
    /// the cap with a wide margin regardless of how many curated symbols
    /// this mode ever grows to -- no batching/size-budget logic needed,
    /// just never batch multiple mints into one message.
    fn send_residual_snapshot(&mut self, now_secs: i64) {
        for (&mint, window) in &self.state.m_residual_history {
            let Some(&last_price_usd) = self.state.m_last_price_usd.get(&mint) else { continue };
            let Some(mint_pubkey) = pubkey_from_account_id(&mint) else { continue };
            let entry = residual_snapshot::ResidualSnapshotEntry { mint: mint_pubkey.to_bytes(), samples: window.samples().collect(), last_price_usd };
            let snapshot = residual_snapshot::ResidualSnapshot { saved_at_secs: now_secs, entries: vec![entry] };
            self.q_msg.push_back(MessageSend::Custom(CustomMessageOutbound::ResidualSnapshotReport(snapshot)));
        }
    }

    /// Inbound half of [`send_residual_snapshot`]: applies a replayed
    /// snapshot from the Go host's own persisted copy, but only if it's
    /// still real, trustworthy history -- `residual_snapshot::is_fresh`
    /// against this mode's own real resync cadence
    /// (`factor_graph::MAX_FACTOR_STALENESS_SECS`) times
    /// `RESIDUAL_WINDOW_CAPACITY`, the same real span the live window
    /// itself represents. A stale snapshot (bot down longer than that)
    /// describes a market that's moved on -- discarded, not replayed,
    /// same fail-closed reasoning `factor_borrow_gate`/`should_close_
    /// pair` already apply to missing-data cases elsewhere in this mode.
    /// Never overwrites real, already-accumulated live history -- the Go
    /// host sends one of these per persisted mint (see
    /// `send_residual_snapshot`'s doc comment for why it's one message
    /// per mint, not one big batch), as early as possible after connect,
    /// specifically to win the race against the first real resync. This
    /// is guarded by `residual_history_live` -- a dedicated flag, not a
    /// per-mint `entry`/`or_insert_with` or a "is the whole map still
    /// empty" check -- because multiple replay messages arrive in
    /// sequence, and after the first one applies, the map is no longer
    /// empty; a map-emptiness check would incorrectly refuse every
    /// message after the first. `residual_history_live` instead tracks
    /// "has a real resync cycle run yet" (set once, in
    /// `run_factor_resync`), which stays false across every one of the
    /// several replay messages that arrive before the first real resync,
    /// then correctly refuses any replay that arrives late.
    fn apply_residual_snapshot(&mut self, snapshot: residual_snapshot::ResidualSnapshot) {
        // Temporary diagnostic (2026-09-06): bracketing this call while
        // chasing a real, live-observed hang (bot's own elapsed-time log
        // stopped dead at a point where several of these replay messages
        // had just been processed, no panic/trap anywhere) -- this
        // function is otherwise untouched this session, but its own doc
        // comment says several replay messages arrive in a real
        // back-to-back burst, so a diagnostic here rules it in or out
        // conclusively. Remove once the real root cause is found.
        log_warn!("multimodelv1: apply_residual_snapshot -- entered (saved_at={} entries={})", snapshot.saved_at_secs, snapshot.entries.len());
        if self.state.residual_history_live {
            log_warn!(
                "multimodelv1: replayed residual snapshot arrived after real live history already started accumulating -- discarding the replay, keeping real live data",
            );
            return;
        }
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let max_age_secs = RESIDUAL_WINDOW_CAPACITY as i64 * factor_graph::MAX_FACTOR_STALENESS_SECS;
        if !residual_snapshot::is_fresh(snapshot.saved_at_secs, now_secs, max_age_secs) {
            log_warn!(
                "multimodelv1: replayed residual snapshot is too stale to trust (saved_at={} now={} max_age={}s) -- discarding, starting cold",
                snapshot.saved_at_secs, now_secs, max_age_secs,
            );
            return;
        }
        let mut applied = 0usize;
        for entry in snapshot.entries {
            let mint = account_id_from_pubkey(&Pubkey::new_from_array(entry.mint));
            self.state
                .m_residual_history
                .insert(mint, factor_residual::RollingWindow::from_samples(RESIDUAL_WINDOW_CAPACITY, entry.samples));
            self.state.m_last_price_usd.insert(mint, entry.last_price_usd);
            applied += 1;
        }
        log_warn!("multimodelv1: replayed real residual snapshot for {applied} mint(s) -- skipping the usual cold-start warm-up");
    }

    // --- Sub-phase 5c: pair/stat-arb trade (Phase 5 point 2) ---------------

    /// One-time bootstrap for this mode's own ([`PAIR_KAMINO_OBLIGATION_ID`])
    /// Kamino obligation. Ported from
    /// `leveragedloopv1::state::bootstrap_basis_kamino_obligation`.
    fn bootstrap_pair_kamino_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment: without this, every basket leg that dispatches here in
        // the same cycle re-appends the same init instructions.
        if self.state.m_pair_kamino_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_pair_kamino_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);
        let has_user_metadata =
            self.state.o_pair_kamino_position.as_ref().is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("multimodelv1: pair: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("multimodelv1: pair: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("multimodelv1: pair: bootstrap: registering pair-trade Kamino obligation (id={PAIR_KAMINO_OBLIGATION_ID})");
        if let Err(e) = kamino::init_obligation(owner, lending_market, PAIR_KAMINO_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: pair: bootstrap: kamino init_obligation failed: {e}");
        }
    }

    fn pair_kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    fn pair_kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    /// Ported from `leveragedloopv1::state::basis_kamino_refresh_all_reserves`.
    fn pair_kamino_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let deposit_reserves = self.pair_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.pair_kamino_obligation_borrow_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in deposit_reserves.iter().chain(borrow_reserves.iter()).chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.kamino().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(
                *reserve_id,
                reserve.pyth_oracle,
                reserve.switchboard_price_oracle,
                reserve.switchboard_twap_oracle,
                reserve.scope_prices,
                self.wallet,
            ) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    fn pair_kamino_refresh_obligation(&mut self, lending_market: AccountId) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("pair kamino obligation not resolved yet".to_string());
        };
        let deposit_reserves = self.pair_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.pair_kamino_obligation_borrow_reserves();
        kamino::refresh_obligation(lending_market, obligation_id, &deposit_reserves, &borrow_reserves, self.wallet)
            .map_err(|e| e.to_string())
    }

    /// Ported from `leveragedloopv1::state::ensure_basis_kamino_farm_ready`.
    fn ensure_pair_kamino_farm_ready(
        &mut self,
        reserve_id: AccountId,
        reserve_lending_market: AccountId,
        farm: Option<AccountId>,
        mode: u8,
    ) -> bool {
        let Some(farm) = farm else { return true };
        let Some(owner) = self.state.wallet() else { return false };
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return false;
        };
        let Some(farm_user_state_id) = kamino::farm_user_state_id(farm, obligation_id) else { return false };
        let Some(pair_kamino_position) = self.state.o_pair_kamino_position.as_mut() else { return false };
        if let Err(e) = pair_kamino_position.track_farm_user_state(farm_user_state_id, self.graph) {
            log_error!("multimodelv1: pair: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if pair_kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!("multimodelv1: pair: bootstrapping Kamino farm-user-state for reserve {reserve_id}");
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: kamino init_obligation_farms_for_reserve failed: {e}");
        }
        false
    }

    /// Solend reserves whose on-chain `RefreshReserve` instruction is
    /// currently broken and rejected by the Solend program itself --
    /// confirmed 2026-09-07 for PONKE by pulling several of the bot's own
    /// recent Solend txs directly from mainnet: every attempt fails with
    /// custom error 0x2a, logging "Could not find oracle type for
    /// nu11...1111" / "Switchboard oracle price is stale" / "Input oracle
    /// config is invalid". The reserve's on-chain `switchboard_oracle`
    /// field genuinely is that null sentinel (our code reads it straight
    /// off the reserve account, it isn't a bug on our side) -- this is a
    /// Solend-side reserve-config problem, not fixable here. Every
    /// attempt burns a real fee for nothing and was the root cause of the
    /// repeated "hop-chain already pending" retries seen against PONKE
    /// earlier this session, so new opens are steered away from Solend
    /// for these mints entirely rather than keep retrying a reserve
    /// that's structurally broken right now.
    const SOLEND_BROKEN_ORACLE_MINTS: &'static [Pubkey] =
        &[Pubkey::from_str_const("5z3EqYQo9HiCEs3R84RCDMu2n7anpDMxRhdK8PSWmrRC")]; // PONKE

    fn solend_reserve_blocked(mint: AccountId) -> bool {
        Self::SOLEND_BROKEN_ORACLE_MINTS.iter().any(|pk| account_id_from_pubkey(pk) == mint)
    }

    /// Cheapest real borrow APY for `mint` across Solend and Kamino
    /// (percent units, matching [`factor_borrow_gate::decide_short_leg`]'s
    /// expectation) -- `None` if neither protocol has a tracked, priced
    /// reserve for it yet. Used both for the borrow-gate signal and, once
    /// a short leg is opened, to decide which protocol to actually borrow
    /// from. Ported from `perpfundingv1::state::StateHelper::
    /// best_borrow_apy`, Marginfi arm dropped.
    fn best_borrow_apy(&self, mint: AccountId) -> Option<(LendingProtocol, f64)> {
        let dex = self.state.o_dex.as_ref()?;
        let candidates = [
            (!Self::solend_reserve_blocked(mint))
                .then(|| dex.solend().reserve_by_mint(mint))
                .flatten()
                .map(|(_, r)| (LendingProtocol::Solend, r.current_borrow_apy() * 100.0)),
            dex.kamino().reserve_by_mint(mint).map(|(_, r)| (LendingProtocol::Kamino, r.current_borrow_apy() * 100.0)),
        ];
        candidates.into_iter().flatten().min_by(|(_, a), (_, b)| a.total_cmp(b))
    }

    /// Highest real supply APY for `mint` across Solend and Kamino --
    /// used once deposit (long-leg) is already chosen (depositing always
    /// helps regardless of protocol, so this isn't part of the open/close
    /// signal, only which protocol to actually deposit into). Ported from
    /// `perpfundingv1::state::StateHelper::best_supply_apy`, Marginfi arm
    /// dropped.
    fn best_supply_apy(&self, mint: AccountId) -> Option<(LendingProtocol, f64)> {
        let dex = self.state.o_dex.as_ref()?;
        let candidates = [
            (!Self::solend_reserve_blocked(mint))
                .then(|| dex.solend().reserve_by_mint(mint))
                .flatten()
                .map(|(_, r)| (LendingProtocol::Solend, r.current_supply_apy() * 100.0)),
            dex.kamino().reserve_by_mint(mint).map(|(_, r)| (LendingProtocol::Kamino, r.current_supply_apy() * 100.0)),
        ];
        candidates.into_iter().flatten().max_by(|(_, a), (_, b)| a.total_cmp(b))
    }

    /// Real oracle price (USD) and mint decimals for `mint` from
    /// `protocol`'s own reserve -- used by [`Self::size_pair_legs`] so
    /// sizing math agrees with whichever protocol's reserve
    /// [`Self::best_supply_apy`]/[`Self::best_borrow_apy`] actually
    /// selected for execution, rather than assuming Kamino.
    fn reserve_price_and_decimals(&self, mint: AccountId, protocol: LendingProtocol) -> Option<(f64, u32)> {
        let dex = self.state.o_dex.as_ref()?;
        match protocol {
            LendingProtocol::Kamino => dex.kamino().reserve_by_mint(mint).map(|(_, r)| (r.price_usd, r.mint_decimals as u32)),
            LendingProtocol::Solend => dex.solend().reserve_by_mint(mint).map(|(_, r)| (r.price_usd, r.mint_decimals as u32)),
        }
    }

    /// Which lending protocol a currently-open pair-trade leg for `mint`
    /// actually used -- checks both `SolendPosition`/`KaminoPosition`'s
    /// live obligations for a deposit or borrow against that mint's
    /// reserve. Mutually exclusive by construction (a leg only ever opens
    /// on one protocol, decided once at open time -- see
    /// [`Self::best_borrow_apy`]/[`Self::best_supply_apy`]). `None` if
    /// neither protocol shows a position (nothing open, or reserve/
    /// obligation data not loaded yet). Ported from
    /// `perpfundingv1::state::StateHelper::holding_lending_protocol`,
    /// Marginfi arm dropped.
    fn holding_lending_protocol(&self, mint: AccountId) -> Option<LendingProtocol> {
        let dex = self.state.o_dex.as_ref()?;

        if let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Solend);
                }
            }
        }
        if let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Kamino);
                }
            }
        }
        None
    }

    /// Trade type 1's own twin to [`Self::holding_lending_protocol`] --
    /// **not a reuse**, since that function hardcodes the pair trade's
    /// own `o_pair_kamino_position`/`o_pair_solend_position` fields, not
    /// mint-only-generic despite `best_supply_apy`/`best_borrow_apy`
    /// being genuinely so. Checks `o_directional_kamino_position`/
    /// `o_directional_solend_position` instead, same shape otherwise.
    fn directional_holding_lending_protocol(&self, mint: AccountId) -> Option<LendingProtocol> {
        let dex = self.state.o_dex.as_ref()?;

        if let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Solend);
                }
            }
        }
        if let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Kamino);
                }
            }
        }
        None
    }

    /// Opens the "long" leg of a pair trade for `symbol` -- dispatches to
    /// whichever of Solend/Kamino currently offers the best real supply
    /// APY for `mint` (see [`Self::best_supply_apy`]).
    fn open_pair_long_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_supply_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_pair_long_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_pair_long_leg_solend(symbol, mint, notional_usd),
            None => log_warn!("multimodelv1: pair: open long leg {symbol} -- no real supply APY on either protocol yet"),
        }
    }

    /// Closes the "long" leg of a pair trade for `symbol` -- dispatches
    /// to whichever protocol actually holds the real deposit (see
    /// [`Self::holding_lending_protocol`]).
    fn close_pair_long_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_pair_long_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_pair_long_leg_solend(symbol, mint),
            None => log_warn!("multimodelv1: pair: close long leg {symbol} -- no open position found on either protocol"),
        }
    }

    /// Opens the "short" leg of a pair trade for `symbol` -- dispatches to
    /// whichever of Solend/Kamino currently offers the cheapest real
    /// borrow APY for `mint` (see [`Self::best_borrow_apy`]).
    fn open_pair_short_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_borrow_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_pair_short_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_pair_short_leg_solend(symbol, mint, notional_usd),
            None => log_warn!("multimodelv1: pair: open short leg {symbol} -- no real borrow APY on either protocol yet"),
        }
    }

    /// Closes the "short" leg of a pair trade for `symbol` -- dispatches
    /// to whichever protocol actually holds the real borrow (see
    /// [`Self::holding_lending_protocol`]).
    fn close_pair_short_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_pair_short_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_pair_short_leg_solend(symbol, mint),
            None => log_warn!("multimodelv1: pair: close short leg {symbol} -- no open position found on either protocol"),
        }
    }

    /// Real, live wallet USDC value -- ported verbatim from
    /// `leveragedloopv1::state::current_usdc_value`.
    ///
    /// `is_final=false`, matching every other real balance read in this
    /// file (e.g. `close_dispersion_long_leg`/`size_pair_legs`) -- reads
    /// the fast (~400ms) low-latency stream, falling back to the rooted
    /// (~12s+) one only if the fast stream has nothing yet. Real,
    /// live-confirmed bug fixed here (2026-09-04): this used to pass
    /// `true` (the exact bug already fixed for `close_dispersion_long_
    /// leg`/`current_open_dispersion`, just missed here), which
    /// `TokenDatabase::balance` reads *exclusively* from the rooted
    /// stream -- confirmed live, this read $0.00 for two consecutive
    /// cycles (32s+) right after a real close landed, while the real
    /// on-chain balance was $51.48, spuriously blocking the USDC floor
    /// check below `run_dispersion_trade_cycle`'s open pass.
    fn current_usdc_value(&mut self) -> f64 {
        let Some(owner) = self.state.wallet() else { return 0.0 };
        const USDC_DECIMALS: i32 = 6;
        let mint_usdc = self.configuration.mint_usdc;
        let usdc_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint_usdc, false).iter().map(|(_, a)| *a).sum();
        usdc_balance_raw as f64 / 10f64.powi(USDC_DECIMALS)
    }

    /// `current_usdc_value()` minus whatever an earlier trade type already
    /// reserved this same resync cycle -- see `m_usdc_reserved_this_cycle`'s
    /// own doc comment for the real race this closes: `run_factor_resync`
    /// runs pair/directional/dispersion's open-passes sequentially against
    /// the *same* real balance snapshot, before any of their transactions
    /// land on-chain and actually change it, so without this, two trade
    /// types enabled at once could each see the same full real balance as
    /// available and both commit against it. Open-pass floor checks (and
    /// anything that sizes real spend *within* an open-pass) must use this,
    /// not the raw `current_usdc_value()`, once more than one trade type
    /// can be enabled in the same process.
    fn available_usdc_value(&mut self) -> f64 {
        (self.current_usdc_value() - self.state.m_usdc_reserved_this_cycle).max(0.0)
    }

    /// Generic USDC<->underlying (or any mint<->mint) real spot swap,
    /// ported verbatim from `leveragedloopv1::state::execute_spot_leg` --
    /// see that function's own doc comments for the three real,
    /// live-confirmed bugs its atomic-group/checkpoint/cooldown handling
    /// fixes (cross-hop ordering, partial-route fund leaks, and
    /// oversized-atomic-group stalls). Not basis-trade-specific there,
    /// not pair-trade-specific here either.
    ///
    /// Real, live-confirmed incident (2026-09-04): this had no guard
    /// against starting a *second*, fully independent hop-chain toward a
    /// target this owner already has one in flight for. Dispersion's
    /// open-pass calls this once per leg every cycle the entry signal
    /// holds and `current_open_dispersion` hasn't yet seen the position
    /// as held -- and unlike a close (whose *source* balance drops the
    /// instant the chain's first hop lands, so a retry correctly sees
    /// nothing left to sell), an open only becomes visible once the
    /// *whole* chain reaches the real final target mint, which can take
    /// several cycles for a route too large for one transaction (see
    /// `PendingHopChain`'s own doc comment). With no guard, two (or
    /// more) real, separate hop-0 sends landed for the same nominal legs
    /// across two cycles, each spending its own real USDC into a shared
    /// intermediate mint -- ~$55 of real capital ended up stuck there,
    /// stalled short of any of the intended final positions, before the
    /// bot was stopped. Refusing here (rather than in every caller) is
    /// the fix: `execute_spot_leg` is the one real choke point every
    /// trade type's leg execution already goes through.
    fn execute_spot_leg(&mut self, mint_in: AccountId, mint_out: AccountId, amount_in: u64, max_hops: usize) -> Result<u64, String> {
        let Some(owner) = self.state.wallet() else {
            return Err("no wallet keypair yet".to_string());
        };
        if let Some(sig) = self.state.m_pending_hop_chains.iter().find_map(|(sig, chain)| {
            (chain.owner == owner && chain.final_target_mint() == mint_out).then_some(*sig)
        }) {
            return Err(format!(
                "a hop-chain toward {mint_out} is already pending (signature {sig}) -- refusing to start a redundant one",
            ));
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return Err("dex state not ready".to_string());
        };
        // Real, live-confirmed reliability gap (2026-09-04): `evaluate()`
        // resets priority to `Medium` at the top of every tick, and
        // nothing in this function (or any of pair/directional/
        // dispersion's leg-open/close callers) ever raised it back to
        // `High` -- only `execute_arbitrage_opportunity` did. Per
        // `Wallet::drain_and_send`'s own doc comment, only `High`
        // priority makes it try the real, durable-nonce-raced
        // `send_bundler_pair` mechanism at all; every other spot leg in
        // this file went out via the plain default route instead.
        // Confirmed live: of several consecutive real close sends built
        // this way for the same leg, most either landed-but-failed or
        // never appeared on-chain at all (`solana confirm` -> "Not
        // found"). `execute_spot_leg` is the one real choke point every
        // trade type's leg execution already goes through (see this
        // function's own doc comment above) -- fixing it here covers all
        // of them, not just arbitrage.
        self.wallet.set_priority_fee(PriorityLevel::High);
        self.state.router.set_current_slot(self.state.last_slot);

        // Real, live-confirmed gap (2026-09-08): a real Orca pool for
        // pSo1f9nQ.../WSOL sat `PoolNotReady` for 2+ hours straight during
        // a live unwind (confirmed dead on-chain: no real transaction in
        // weeks) while a genuinely live Raydium CLMM pool for the exact
        // same pair sat right there in the same price graph, untouched --
        // `widest_path` already ranks every DEX's edges uniformly by
        // `cp_quote`, so once the dead pool's stale-but-attractive cached
        // reserves are excluded it naturally falls through to whatever
        // real alternative (any DEX) is actually alive. The only thing
        // missing was giving *this call* a way to exclude a
        // just-failed pool before its very next attempt, instead of only
        // ever finding out next resync cycle (waiting on
        // `POOL_NOT_READY_COOLDOWN_THRESHOLD`, which is deliberately slow
        // to fire so a merely-still-subscribing pool isn't punished for a
        // single early failure). This bounded retry loop excludes a
        // failed pool with a same-slot-only cooldown (current_slot never
        // advances mid-call, so `mark_pool_cooldown(id, 1)` reliably
        // excludes it for the rest of *this* call without lingering into
        // future ticks or interfering with the real, escalating cooldown
        // logic below, which is unchanged) and re-runs `route_slippage_
        // aware`, giving the router an immediate chance to route around
        // it -- e.g. onto Raydium CLMM -- in the same call, rather than
        // giving up for an entire resync cycle.
        const MAX_ROUTE_ATTEMPTS: u32 = 3;
        let mut attempt = 0u32;
        let (route, checkpoint) = loop {
            attempt += 1;
            let Some(route) = self.state.router.route_slippage_aware(mint_in, mint_out, amount_in, max_hops) else {
                let (acked, sent) = crate::graph::subscription_ack_counts();
                log_error!(
                    "multimodelv1: route diagnostics for {mint_in} -> {mint_out} (subscriptions: {acked}/{sent} acked):\n{}",
                    self.state.router.route_diagnostics(mint_in, mint_out, amount_in, max_hops)
                );
                return Err(format!("no route found for {mint_in} -> {mint_out} amount_in={amount_in}"));
            };
            let route = match planner::reverify_route_with_exact_quotes(&route, amount_in, &self.state.router, dex) {
                Ok(route) => route,
                Err(failure) => {
                    if failure.coolable {
                        self.state.router.mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    } else {
                        // See `planner::HopFailure`'s doc comment -- this
                        // pool's live data (tick arrays) just isn't
                        // synced yet, not a real bad quote; only a
                        // trivial same-slot cooldown here (steers this
                        // call's own retry elsewhere), not the real,
                        // longer one a genuinely-bad pool gets.
                        self.state.router.mark_pool_cooldown(failure.pool_id, 1);
                    }
                    if attempt < MAX_ROUTE_ATTEMPTS {
                        log_warn!(
                            "multimodelv1: spot leg {mint_in} -> {mint_out}: pool {} invalidated during reverify (coolable={}) -- excluding it and retrying route (attempt {}/{MAX_ROUTE_ATTEMPTS})",
                            failure.pool_id, failure.coolable, attempt + 1,
                        );
                        continue;
                    }
                    let dex_type = route.hops.iter().find(|h| h.pool_id == failure.pool_id).map(|h| h.dex);
                    return Err(format!(
                        "pool {} (dex={dex_type:?}) invalidated during reverify (coolable={}) after {attempt} route attempt(s)",
                        failure.pool_id, failure.coolable,
                    ));
                }
            };
            log_warn!(
                "multimodelv1: spot leg @ slot {}: {} hop{} {} -> {} amount_in={}",
                self.state.last_slot,
                route.hops.len(),
                if route.hops.len() == 1 { "" } else { "s" },
                mint_in,
                mint_out,
                amount_in,
            );
            self.wallet.begin_atomic_group();
            let checkpoint = self.wallet.queue_checkpoint();
            let mut hop_failure: Option<String> = None;
            for (i, hop) in route.hops.iter().enumerate() {
                let (Some(source_ata), Some(dest_ata)) = (
                    self.wallet.append_create_ata(owner, hop.input_mint),
                    self.wallet.append_create_ata(owner, hop.output_mint),
                ) else {
                    self.wallet.rollback_to(checkpoint);
                    self.wallet.end_atomic_group();
                    return Err(format!("hop {i}: FAILED to derive token account(s) for owner={owner}"));
                };
                match dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet) {
                    Ok(()) => {
                        log_warn!(
                            "  hop {i}: OK dex={:?} pool={} {} -> {} amount_in={} amount_out={}",
                            hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                        );
                    }
                    Err(e) => {
                        self.wallet.rollback_to(checkpoint);
                        self.wallet.end_atomic_group();
                        // `PoolNotReady` means "not enough live data observed
                        // yet" (its own doc comment), the exact same "ask
                        // again shortly" situation as `HopFailure::coolable`
                        // -- not a genuine bad pool, so don't cool it down.
                        // Fix (2026-09-07): live-confirmed a `PoolNotReady`
                        // pool can fail identically for 30+ minutes across
                        // multiple restarts -- not self-correcting.
                        // `note_pool_not_ready` tracks the repeated case and
                        // reports once it's crossed a real threshold, so we
                        // still cool down eventually instead of retrying
                        // forever.
                        let is_pool_not_ready = matches!(e, crate::trader::types::TraderError::PoolNotReady);
                        let coolable = if is_pool_not_ready {
                            self.state.router.note_pool_not_ready(hop.pool_id)
                        } else {
                            true
                        };
                        if coolable {
                            if is_pool_not_ready {
                                self.state.router.mark_pool_not_ready_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                            } else {
                                self.state.router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                            }
                        }
                        // Fix (2026-09-08): a trivial same-slot cooldown
                        // regardless of `coolable` -- see this function's
                        // own doc comment above -- so this exact call's
                        // retry (below) excludes the failed pool even
                        // when it's not yet cool enough for the real,
                        // longer cooldown to apply.
                        self.state.router.mark_pool_cooldown(hop.pool_id, 1);
                        hop_failure = Some(format!(
                            "hop {i}: FAILED dex={:?} pool={} {} -> {}: {} (cooling down {} slots: {coolable})",
                            hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, e, planner::POOL_COOLDOWN_SLOTS,
                        ));
                    }
                }
            }
            if let Some(err) = hop_failure {
                if attempt < MAX_ROUTE_ATTEMPTS {
                    log_warn!(
                        "multimodelv1: spot leg {mint_in} -> {mint_out}: {err} -- excluding it and retrying route (attempt {}/{MAX_ROUTE_ATTEMPTS})",
                        attempt + 1,
                    );
                    continue;
                }
                return Err(err);
            }
            break (route, checkpoint);
        };
        if !self.wallet.atomic_group_fits(checkpoint) {
            self.wallet.rollback_to(checkpoint);
            self.wallet.end_atomic_group();
            // Real fallback, not an immediate failure. The combined route
            // doesn't fit in one 1232-byte transaction; `route.hops` is a
            // genuine sequential dependency chain (`Route::token_out()` =
            // `hops.last().output_mint`, confirmed in `pricegraph.rs`),
            // not independent parallel alternatives -- live-confirmed
            // this session (a real incident, not hypothetical): sending
            // every hop as its own separate transaction and submitting
            // them all together only worked when hop 0 alone already
            // reached the real destination; on most cycles it doesn't
            // (hop 0's real output is a genuine intermediate mint), and
            // sending the later, truly-dependent hops at the same time
            // risked them failing because the balance they need doesn't
            // exist until an earlier hop actually confirms.
            //
            // Fixed for real (not just made safe-but-limited) via
            // `PendingHopChain`/`mid_on_tx` -- see that struct's own doc
            // comment. Send hop 0 alone now; if more hops remain, register
            // them under hop 0's real signature so `mid_on_tx` sends hop 1
            // only once it learns (asynchronously, but for real) that hop
            // 0 actually landed, and so on. This is no longer "assumed
            // safe because hop 0 happens to reach the destination" -- it's
            // safe because every hop after the first is only ever sent
            // once its prerequisite balance is *proven* to exist.
            let hop0 = route.hops[0].clone();
            let hop0_produced_mint_balance_before: u64 = self
                .wallet
                .token_mut()
                .balance(&owner, &hop0.output_mint, false)
                .iter()
                .map(|(_, a)| *a)
                .sum();
            match Self::send_single_hop_as_astralane_tx(self.graph, self.wallet, owner, dex, &hop0) {
                Ok((signature, amount_out)) => {
                    self.state.pending_route_pools = vec![hop0.pool_id];
                    let remaining: Vec<pricegraph::Hop> = route.hops[1..].to_vec();
                    if remaining.is_empty() {
                        log_warn!(
                            "multimodelv1: spot leg -- route {mint_in} -> {mint_out}: {}-hop atomic group too large for one transaction, but hop 0 alone reaches the destination (tx {signature})",
                            route.hops.len(),
                        );
                    } else {
                        log_warn!(
                            "multimodelv1: spot leg -- route {mint_in} -> {mint_out}: {}-hop atomic group too large for one transaction -- sent hop 0 alone (tx {signature}), {} more hop(s) queued to follow once it's confirmed landed",
                            route.hops.len(),
                            remaining.len(),
                        );
                        self.state.m_pending_hop_chains.insert(
                            signature,
                            PendingHopChain {
                                hops: remaining,
                                owner,
                                sent_pool_id: hop0.pool_id,
                                consumed_mint: hop0.input_mint,
                                produced_mint: hop0.output_mint,
                                sent_slot: self.state.last_slot,
                                produced_mint_balance_before: hop0_produced_mint_balance_before,
                            },
                        );
                    }
                    return Ok(amount_out);
                }
                Err(e) => {
                    for hop in route.hops.iter() {
                        self.state.router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    }
                    return Err(format!(
                        "route {mint_in} -> {mint_out} amount_in={amount_in}: {}-hop atomic group too large for one transaction; hop 0 send failed: {e} (cooling down {} slots)",
                        route.hops.len(),
                        planner::POOL_COOLDOWN_SLOTS,
                    ));
                }
            }
        }
        self.wallet.end_atomic_group();
        self.state.pending_route_pools = route.hops.iter().map(|h| h.pool_id).collect();
        Ok(route.hops.last().map(|h| h.amount_out).unwrap_or(0))
    }

    /// Real single-hop send for a route too large to fit as one atomic
    /// group -- sends exactly `hop` and nothing else, as its own
    /// complete, tipped transaction (own atomic group covering its ATA
    /// creation(s) + the swap itself, own size check, own real
    /// `assemble()` call), via `Wallet::send_transaction_batch` (a
    /// one-element batch, still real Astralane tipping/routing for fast
    /// landing) rather than the ordinary queue/`drain_and_send` path, so
    /// it lands independently of whatever else this cycle queued.
    /// Returns the real `Signature` `Wallet::assemble` computed for this
    /// transaction (previously discarded) alongside `hop`'s expected
    /// amount-out -- the caller registers that signature in
    /// `state.m_pending_hop_chains` so `mid_on_tx` can drive sending
    /// whatever hop comes next once this one is confirmed landed (see
    /// [`PendingHopChain`]'s doc comment). **Deliberately an associated
    /// function, not a method** (no `self`/`&mut self` receiver) --
    /// `graph`/`wallet`/`dex` are passed in explicitly so this stays a
    /// disjoint borrow from the caller's own `self.state.router`/`self.
    /// state.m_pending_hop_chains` at each call site (`execute_spot_leg`/
    /// `advance_pending_hop_chain` already borrow `dex` from `self.state.
    /// o_dex`; a `&mut self` method here would conflict with that
    /// existing borrow).
    ///
    /// Real, live-confirmed root cause fixed here (2026-09-03,
    /// user-identified): `mid_on_tx`'s underlying transaction-notification
    /// delivery only ever surfaces a transaction if this bot has an active
    /// subscription on at least one account it touches. A hop chain's
    /// destination mint is very often a pass-through intermediate this
    /// mode never otherwise touches (not `mint_usdc`, not a curated
    /// symbol, so never included in the wallet-connect-time `ata_mints`
    /// batch) -- with no subscription anywhere on the transaction, a real,
    /// cleanly-landed send can be permanently invisible to `mid_on_tx`,
    /// which then always times out (`HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS`)
    /// regardless of how much slot budget is left. Confirmed live: a real
    /// hop 0 send landed at slot 443940750 (only 198 of its 300-slot
    /// budget used, sent at 443940552) and was still never reported,
    /// because its destination (USDH, an intermediate routing mint) had
    /// no subscription anywhere. Fixed by subscribing to the destination
    /// ATA before sending, every time -- not deduped against whatever's
    /// already subscribed, since this path isn't hot enough for that
    /// bookkeeping to be worth it, and an occasional harmless duplicate
    /// subscribe is a good trade for guaranteed `mid_on_tx` visibility.
    ///
    /// Real, live-confirmed reliability gap fixed here (2026-09-04): this
    /// used to *only* append a bare tip instruction (`append_bundler_tip`)
    /// and send one single-shot, regular-blockhash transaction via
    /// `assemble`/`send_transaction_batch` -- despite its name/log text
    /// ("tipped Astralane transaction"), that is *not* the real,
    /// durable-nonce-raced dual-transaction mechanism `Wallet::
    /// send_bundler_pair`'s own doc comment describes as "the intended
    /// way for ordinary `evaluate()` code to opt a real operation into
    /// Astralane landing" -- it's a materially weaker, single-attempt
    /// send. Confirmed live: of 5 consecutive real hop-chain sends built
    /// this way for the same close route, 1 landed (and failed on-chain
    /// for an unrelated reason), and the other 4 never appeared on-chain
    /// at all (`solana confirm` -> "Not found", not just slow). Now tries
    /// `send_bundler_pair` first (the real two-variant nonce race) and
    /// only falls back to the old single-shot path if it declines (e.g.
    /// the durable nonce isn't confirmed `Ready` yet, early in a
    /// process's life) -- `send_bundler_pair` leaves the queue untouched
    /// on decline (see its own doc comment), so appending the tip and
    /// falling back afterward is safe.
    fn send_single_hop_as_astralane_tx(
        graph: &Graph,
        wallet: &mut Wallet,
        owner: AccountId,
        dex: &DexState,
        hop: &pricegraph::Hop,
    ) -> Result<(Signature, u64), String> {
        if let Some(dest_sub_req) = wallet.ata_subscribe_request(owner, hop.output_mint) {
            match SubscriptionQueue::subscribe_now(graph, vec![dest_sub_req]) {
                Ok(subs) => wallet.keep_ata_subscriptions(subs),
                Err(e) => log_error!(
                    "multimodelv1: hop chain -- failed to subscribe to destination ATA for mint {}: {e} -- mid_on_tx may never see this send land",
                    hop.output_mint,
                ),
            }
        }
        let checkpoint = wallet.queue_checkpoint();
        wallet.begin_atomic_group();
        let (Some(source_ata), Some(dest_ata)) = (wallet.append_create_ata(owner, hop.input_mint), wallet.append_create_ata(owner, hop.output_mint)) else {
            wallet.rollback_to(checkpoint);
            wallet.end_atomic_group();
            return Err(format!("FAILED to derive token account(s) for owner={owner}"));
        };
        if let Err(e) = dex.execute_hop(hop, owner, source_ata, dest_ata, wallet) {
            wallet.rollback_to(checkpoint);
            wallet.end_atomic_group();
            return Err(format!("FAILED dex={:?} pool={} {} -> {}: {e}", hop.dex, hop.pool_id, hop.input_mint, hop.output_mint));
        }
        wallet.end_atomic_group();

        if let Some((signature, result)) = wallet.send_bundler_pair(Wallet::ASTRALANE_BUNDLER_CODE) {
            log_warn!(
                "multimodelv1: spot leg -- sending hop dex={:?} pool={} {} -> {} as a real durable-nonce Astralane bundler pair (tx {signature})",
                hop.dex, hop.pool_id, hop.input_mint, hop.output_mint,
            );
            log_warn!("multimodelv1: spot leg -- astralane bundler-pair send result: {result:?}");
            return result.map(|()| (signature, hop.amount_out)).map_err(|e| format!("astralane bundler-pair send failed: {e:?}"));
        }

        // Fallback: `send_bundler_pair` declined (nonce not `Ready` yet,
        // no tip data, or nothing queued) without touching the queue --
        // the swap instruction(s) appended above are still sitting there.
        // Same single-shot path this function used exclusively before.
        if !wallet.append_bundler_tip(owner, Wallet::ASTRALANE_BUNDLER_CODE) {
            wallet.rollback_to(checkpoint);
            return Err("no real Astralane tip data yet, refusing to send an untipped transaction".to_string());
        }
        if !wallet.atomic_group_fits(checkpoint) {
            wallet.rollback_to(checkpoint);
            return Err("even a single hop (with its own tip) is too large for one transaction".to_string());
        }
        let Some((signature, tx_bytes)) = wallet.assemble() else {
            return Err("failed to assemble its own transaction".to_string());
        };
        let tx_bytes = tx_bytes.to_vec();
        log_warn!(
            "multimodelv1: spot leg -- sending hop dex={:?} pool={} {} -> {} as its own tipped single-shot transaction (bundler pair unavailable)",
            hop.dex, hop.pool_id, hop.input_mint, hop.output_mint,
        );
        let result = wallet.send_transaction_batch(&[tx_bytes], Wallet::ASTRALANE_BUNDLER_CODE);
        // Real, explicit confirmation either way -- `Ok` here only means
        // the host *accepted* the send (same "queued, not yet landed"
        // semantics `drain_and_send`'s own "sent transaction" log has),
        // not that it actually confirmed on-chain; logged regardless so
        // a real accept-but-never-lands gap (e.g. a degraded Astralane
        // relay connection, live-observed this session) is visible here
        // instead of silently indistinguishable from success. The real
        // landed/failed result comes later, asynchronously, via
        // `mid_on_tx` matching this same `signature`.
        log_warn!("multimodelv1: spot leg -- astralane single-hop send result: {result:?}");
        result.map(|()| (signature, hop.amount_out)).map_err(|e| format!("astralane send failed: {e:?}"))
    }

    /// Kamino half of opening the "long" leg of a pair trade for
    /// `symbol` -- swaps `notional_usd` of USDC into the underlying,
    /// deposits it into this mode's own obligation (real Kamino supply
    /// APY on top of the expected price convergence). Ported from
    /// `leveragedloopv1::state::open_kamino_deposit_leg`. No-op if a
    /// deposit already exists for this reserve. Dispatched to by
    /// `open_pair_long_leg` based on `best_supply_apy`.
    fn open_pair_long_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_pair_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_pair_kamino_obligation();
            return;
        }
        // See `m_pair_kamino_leg_action_this_cycle`'s own doc comment --
        // at most one real deposit/borrow against this shared obligation
        // per cycle, regardless of how many legs ask for one. Checked
        // here (before the "already done" checks below, so a leg that's
        // genuinely a no-op never wastes the turn); *set* at each real
        // commit point further down, not here.
        if self.state.m_pair_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: pair: {symbol} has no Kamino oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_pair_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: pair: opening long leg {symbol} (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.pair_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_pair_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, self.wallet) {
            log_error!("multimodelv1: pair: long leg {symbol} kamino deposit failed: {e}");
        }
    }

    /// Kamino half of closing the long leg for `symbol` -- withdraws the
    /// real deposited amount, sells it back to USDC. Ported from
    /// `leveragedloopv1::state::close_kamino_deposit_leg`. Dispatched to
    /// by `close_pair_long_leg` based on `holding_lending_protocol`.
    fn close_pair_long_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let has_deposit = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if !has_deposit {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };

        log_warn!("multimodelv1: pair: closing long leg {symbol}");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.pair_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_pair_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: pair: close long leg {symbol} kamino withdraw failed: {e}");
            return;
        }
        let obligation_will_be_empty = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .is_some_and(|ob| ob.deposits.len() <= 1 && ob.borrows.is_empty());
        if obligation_will_be_empty {
            if let Some(pos) = self.state.o_pair_kamino_position.as_mut() {
                pos.mark_obligation_closing();
            }
        }
        let estimated_underlying_raw = ((PAIR_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of opening the "short" leg of a pair trade for
    /// `symbol` -- two-stage: deposits USDC collateral first (if not
    /// already enough), then borrows the underlying and sells it for
    /// USDC (synthetic short) once enough collateral is confirmed.
    /// Ported from `leveragedloopv1::state::open_kamino_borrow_leg`.
    /// Dispatched to by `open_pair_short_leg` based on `best_borrow_apy`.
    fn open_pair_short_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_pair_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_pair_kamino_obligation();
            return;
        }
        // See `m_pair_kamino_leg_action_this_cycle`'s own doc comment --
        // at most one real deposit/borrow against this shared obligation
        // per cycle, regardless of how many legs ask for one. Checked
        // here (before the "already done" checks below, so a leg that's
        // genuinely a no-op never wastes the turn); *set* at each real
        // commit point further down, not here.
        if self.state.m_pair_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let Some((_, borrow_reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let collateral_usd =
            notional_usd * borrow_reserve.borrow_factor_pct / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            self.state.m_pair_kamino_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: pair: depositing ${collateral_usd:.2} USDC collateral for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: pair: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            let (usdc_lending_market, usdc_farm_collateral) = (usdc_reserve.lending_market, usdc_reserve.farm_collateral);
            if let Err(e) = self.pair_kamino_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: pair: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.pair_kamino_refresh_obligation(usdc_lending_market) {
                log_error!("multimodelv1: pair: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            if !self.ensure_pair_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_collateral, 0) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) =
                usdc_reserve.deposit(usdc_reserve_id, obligation_id, usdc_amount_raw, owner, usdc_ata, self.wallet)
            {
                log_error!("multimodelv1: pair: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: pair: {symbol} has no Kamino oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_pair_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: pair: opening short leg {symbol} (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(
            usdc_reserve_id,
            usdc_reserve.pyth_oracle,
            usdc_reserve.switchboard_price_oracle,
            usdc_reserve.switchboard_twap_oracle,
            usdc_reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Err(e) = self.pair_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.pair_kamino_obligation_deposit_reserves();
        if let Err(e) = self.pair_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_pair_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.borrow(
            reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            underlying_ata,
            None,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: short leg {symbol} kamino borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of closing the short leg for `symbol` -- buys back the
    /// real, currently-borrowed amount with USDC, repays it. USDC
    /// collateral stays deposited for reuse. Ported from
    /// `leveragedloopv1::state::close_kamino_borrow_leg`. Dispatched to by
    /// `close_pair_short_leg` based on `holding_lending_protocol`.
    fn close_pair_short_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_pair_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: pair: closing short leg {symbol}");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: pair: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: pair: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.pair_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_pair_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: pair: close short leg {symbol} repay failed: {e}");
        }
    }

    // --- Sub-phase 5c point 2: Solend execution (added alongside Kamino,
    // same pair-trade decision logic, different lending protocol) --------

    /// Solend counterpart to `bootstrap_pair_kamino_obligation` -- simpler,
    /// no `init_user_metadata`/farm-equivalent pre-req for Solend. Real
    /// two-step account creation (`create_obligation_account` then
    /// `init_obligation`), unlike Kamino's single-step `init_obligation`
    /// -- Solend's Obligation isn't a PDA, see `solend::OBLIGATION_SEED`'s
    /// doc comment.
    fn bootstrap_pair_solend_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment (Solend twin of the same guard).
        if self.state.m_pair_solend_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_pair_solend_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((_, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            log_warn!("multimodelv1: pair: bootstrap: Solend USDC reserve not observed yet");
            return;
        };
        let lending_market = usdc_reserve.lending_market;
        log_warn!(
            "multimodelv1: pair: bootstrap: registering pair-trade Solend obligation (id={PAIR_SOLEND_OBLIGATION_ID})"
        );
        if let Err(e) = solend::create_obligation_account(owner, PAIR_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: pair: bootstrap: solend create_obligation_account failed: {e}");
            return;
        }
        if let Err(e) = solend::init_obligation(owner, lending_market, PAIR_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: pair: bootstrap: solend init_obligation failed: {e}");
        }
    }

    fn pair_solend_obligation_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).chain(ob.borrows.iter().map(|b| b.borrow_reserve)).collect()
    }

    fn pair_solend_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    /// Refreshes every reserve this cycle's leg touches, mirroring
    /// `pair_kamino_refresh_all_reserves` exactly -- and for a real,
    /// documented reason beyond just consistency: unlike `perpfundingv1`'s
    /// own Solend legs (which only refresh the *one* reserve being
    /// touched, a known simplification that bot's own doc comments flag
    /// as safe there because it typically holds one symbol at a time),
    /// this mode's pair-trade obligation genuinely holds multiple real
    /// reserves simultaneously (long deposit + short's USDC collateral +
    /// short's borrow, all on one obligation). `perpfundingv1`'s own
    /// `close_solend_borrow_leg` doc comment documents the real on-chain
    /// failure this causes if skipped (`refresh_obligation` reverts with
    /// a real `ReserveStale` if any currently-held reserve wasn't
    /// individually refreshed in the same transaction, live-confirmed
    /// there). Refreshing every currently-held reserve plus `extra` (the
    /// new reserve about to be touched, not yet in the obligation) avoids
    /// that failure mode entirely.
    fn pair_solend_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let reserves = self.pair_solend_obligation_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in reserves.iter().chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.solend().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(*reserve_id, self.wallet) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    /// Unlike Kamino's `refresh_obligation` (which takes `lending_market`
    /// as a real parameter), Solend's own `refresh_obligation` doesn't
    /// need it -- see `solend::refresh_obligation`'s own signature.
    /// Reserve list is deliberately the obligation's *current* real
    /// deposits+borrows only, no `extra` -- Solend's own program requires
    /// the remaining-accounts count to match the obligation's current
    /// on-chain state exactly (see `perpfundingv1::solend_refresh_reserves`'s
    /// doc comment for the real, live-verified reasoning); the new
    /// reserve about to be touched is added to the obligation *by* the
    /// deposit/borrow instruction, not before it.
    fn pair_solend_refresh_obligation(&mut self) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("pair solend obligation not resolved yet".to_string());
        };
        let reserves = self.pair_solend_obligation_reserves();
        solend::refresh_obligation(obligation_id, &reserves, self.wallet).map_err(|e| e.to_string())
    }

    /// Solend counterpart to `open_pair_long_leg_kamino` -- see that
    /// function's own doc comment for the shared long-leg logic (swap
    /// USDC into the underlying, deposit as obligation collateral). Real
    /// API difference: Solend's `deposit` needs a scratch
    /// `user_collateral_account` ATA (`reserve.collateral_mint`) the
    /// deposited cTokens pass through -- Kamino mints cTokens straight
    /// into the obligation, no such account needed there.
    fn open_pair_long_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_pair_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_pair_solend_obligation();
            return;
        }
        // See `m_pair_solend_leg_action_this_cycle`'s own doc comment.
        if self.state.m_pair_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_pair_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: pair: {symbol} has no Solend oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        self.state.m_pair_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: pair: opening long leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_obligation() {
            log_error!("multimodelv1: pair: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, collateral_ata, self.wallet)
        {
            log_error!("multimodelv1: pair: long leg {symbol} solend deposit failed: {e}");
        }
    }

    /// Solend counterpart to `close_pair_long_leg_kamino` -- withdraws the
    /// real deposited amount (already in collateral/cToken units, read
    /// directly from the real obligation -- same real value
    /// `perpfundingv1::close_solend_deposit_leg` passes, not
    /// `solend::SOLEND_AMOUNT_MAX`, matching its own tested behavior),
    /// sells it back to USDC. Real API differences from Kamino: needs a
    /// scratch collateral ATA plus the obligation's current
    /// `deposit_reserves` list (Solend's `withdraw` requires "borrow
    /// attribution" accounts Kamino's own withdraw doesn't).
    fn close_pair_long_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(collateral_amount) = self
            .state
            .o_pair_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .map(|d| d.deposited_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        log_warn!("multimodelv1: pair: closing long leg {symbol} via Solend");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_obligation() {
            log_error!("multimodelv1: pair: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let deposit_reserves = self.pair_solend_obligation_deposit_reserves();
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            underlying_ata,
            collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: pair: close long leg {symbol} solend withdraw failed: {e}");
            return;
        }
        let estimated_underlying_raw = ((PAIR_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend counterpart to `open_pair_short_leg_kamino` -- same
    /// two-stage shape (deposit USDC collateral first, borrow once
    /// confirmed). Real API difference: the USDC collateral deposit also
    /// needs a scratch collateral ATA, and the borrow needs the
    /// obligation's current `deposit_reserves` list.
    fn open_pair_short_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_pair_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_pair_solend_obligation();
            return;
        }
        // See `m_pair_solend_leg_action_this_cycle`'s own doc comment.
        if self.state.m_pair_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let usdc_collateral_mint = usdc_reserve.collateral_mint;

        const USDC_DECIMALS: i32 = 6;
        // Real, live-verified sizing (matches `perpfundingv1::
        // open_solend_borrow_leg`'s own tested formula exactly): Solend's
        // real reserve has no `borrow_factor_pct`-style risk-weighting
        // field the way Kamino's does, so this is deliberately simpler
        // than the Kamino short leg's own collateral calc, not an
        // oversight.
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let collateral_usd = notional_usd / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_pair_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            let Some(usdc_collateral_ata) = self.wallet.append_create_ata(owner, usdc_collateral_mint) else {
                return;
            };
            self.state.m_pair_solend_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: pair: depositing ${collateral_usd:.2} USDC collateral (Solend) for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: pair: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            if let Err(e) = self.pair_solend_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: pair: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.pair_solend_refresh_obligation() {
                log_error!("multimodelv1: pair: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) = usdc_reserve.deposit(
                usdc_reserve_id,
                obligation_id,
                usdc_amount_raw,
                owner,
                usdc_ata,
                usdc_collateral_ata,
                self.wallet,
            ) {
                log_error!("multimodelv1: pair: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_pair_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: pair: {symbol} has no Solend oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_pair_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: pair: opening short leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!("multimodelv1: pair: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.pair_solend_obligation_deposit_reserves();
        if let Err(e) = self.pair_solend_refresh_obligation() {
            log_error!("multimodelv1: pair: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.borrow(reserve_id, obligation_id, borrow_amount_raw, owner, underlying_ata, &deposit_reserves, self.wallet)
        {
            log_error!("multimodelv1: pair: short leg {symbol} solend borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: pair: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend counterpart to `close_pair_short_leg_kamino` -- buys back
    /// the real currently-borrowed amount, repays with
    /// `solend::SOLEND_AMOUNT_MAX`. USDC collateral stays deposited for
    /// reuse, matching the Kamino side's own precedent.
    fn close_pair_short_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_pair_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: pair: closing short leg {symbol} via Solend");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: pair: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: pair: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.pair_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.pair_solend_refresh_obligation() {
            log_error!("multimodelv1: pair: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, solend::SOLEND_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: pair: close short leg {symbol} repay failed: {e}");
        }
    }

    // --- Trade type 5 (Hawkes-on-eigenfactor momentum): execution ----------
    // Verbatim mechanical port of the pair trade's own Kamino+Solend
    // long/short leg execution above, retargeted at this trade type's own
    // (`o_hawkes_kamino_position`/`o_hawkes_solend_position`) obligations --
    // same real reasoning as directional's own leg-execution code, ported
    // rather than shared, since sharing would mean threading a position
    // field through every call site. See `docs/HAWKES_FACTOR_TRADE_PLAN.md`.

    /// One-time bootstrap for this mode's own ([`HAWKES_KAMINO_OBLIGATION_ID`])
    /// Kamino obligation. Ported from
    /// `leveragedloopv1::state::bootstrap_basis_kamino_obligation`.
    fn bootstrap_hawkes_kamino_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment (Hawkes twin of the same guard).
        if self.state.m_hawkes_kamino_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_hawkes_kamino_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);
        let has_user_metadata =
            self.state.o_hawkes_kamino_position.as_ref().is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("multimodelv1: hawkes: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("multimodelv1: hawkes: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("multimodelv1: hawkes: bootstrap: registering hawkes-trade Kamino obligation (id={HAWKES_KAMINO_OBLIGATION_ID})");
        if let Err(e) = kamino::init_obligation(owner, lending_market, HAWKES_KAMINO_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: hawkes: bootstrap: kamino init_obligation failed: {e}");
        }
    }

    fn hawkes_kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    fn hawkes_kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    /// Ported from `leveragedloopv1::state::basis_kamino_refresh_all_reserves`.
    fn hawkes_kamino_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let deposit_reserves = self.hawkes_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.hawkes_kamino_obligation_borrow_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in deposit_reserves.iter().chain(borrow_reserves.iter()).chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.kamino().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(
                *reserve_id,
                reserve.pyth_oracle,
                reserve.switchboard_price_oracle,
                reserve.switchboard_twap_oracle,
                reserve.scope_prices,
                self.wallet,
            ) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    fn hawkes_kamino_refresh_obligation(&mut self, lending_market: AccountId) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("hawkes kamino obligation not resolved yet".to_string());
        };
        let deposit_reserves = self.hawkes_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.hawkes_kamino_obligation_borrow_reserves();
        kamino::refresh_obligation(lending_market, obligation_id, &deposit_reserves, &borrow_reserves, self.wallet)
            .map_err(|e| e.to_string())
    }

    /// Ported from `leveragedloopv1::state::ensure_basis_kamino_farm_ready`.
    fn ensure_hawkes_kamino_farm_ready(
        &mut self,
        reserve_id: AccountId,
        reserve_lending_market: AccountId,
        farm: Option<AccountId>,
        mode: u8,
    ) -> bool {
        let Some(farm) = farm else { return true };
        let Some(owner) = self.state.wallet() else { return false };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return false;
        };
        let Some(farm_user_state_id) = kamino::farm_user_state_id(farm, obligation_id) else { return false };
        let Some(hawkes_kamino_position) = self.state.o_hawkes_kamino_position.as_mut() else { return false };
        if let Err(e) = hawkes_kamino_position.track_farm_user_state(farm_user_state_id, self.graph) {
            log_error!("multimodelv1: hawkes: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if hawkes_kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!("multimodelv1: hawkes: bootstrapping Kamino farm-user-state for reserve {reserve_id}");
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: kamino init_obligation_farms_for_reserve failed: {e}");
        }
        false
    }

    /// Which lending protocol a currently-open hawkes-trade leg for `mint`
    /// actually used -- checks both `SolendPosition`/`KaminoPosition`'s
    /// live obligations for a deposit or borrow against that mint's
    /// reserve. Mutually exclusive by construction (a leg only ever opens
    /// on one protocol, decided once at open time -- see
    /// [`Self::best_borrow_apy`]/[`Self::best_supply_apy`]). `None` if
    /// neither protocol shows a position (nothing open, or reserve/
    /// obligation data not loaded yet).
    fn hawkes_holding_lending_protocol(&self, mint: AccountId) -> Option<LendingProtocol> {
        let dex = self.state.o_dex.as_ref()?;

        if let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Solend);
                }
            }
        }
        if let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) {
            if let Some(ob) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation()) {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Kamino);
                }
            }
        }
        None
    }

    /// Opens the "long" leg of a hawkes trade for `symbol` -- dispatches to
    /// whichever of Solend/Kamino currently offers the best real supply
    /// APY for `mint` (see [`Self::best_supply_apy`]).
    fn open_hawkes_long_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_supply_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_hawkes_long_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_hawkes_long_leg_solend(symbol, mint, notional_usd),
            None => log_warn!("multimodelv1: hawkes: open long leg {symbol} -- no real supply APY on either protocol yet"),
        }
    }

    /// Closes the "long" leg of a hawkes trade for `symbol` -- dispatches
    /// to whichever protocol actually holds the real deposit (see
    /// [`Self::hawkes_holding_lending_protocol`]).
    fn close_hawkes_long_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.hawkes_holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_hawkes_long_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_hawkes_long_leg_solend(symbol, mint),
            None => log_warn!("multimodelv1: hawkes: close long leg {symbol} -- no open position found on either protocol"),
        }
    }

    /// Opens the "short" leg of a hawkes trade for `symbol` -- dispatches to
    /// whichever of Solend/Kamino currently offers the cheapest real
    /// borrow APY for `mint` (see [`Self::best_borrow_apy`]).
    fn open_hawkes_short_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_borrow_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_hawkes_short_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_hawkes_short_leg_solend(symbol, mint, notional_usd),
            None => log_warn!("multimodelv1: hawkes: open short leg {symbol} -- no real borrow APY on either protocol yet"),
        }
    }

    /// Closes the "short" leg of a hawkes trade for `symbol` -- dispatches
    /// to whichever protocol actually holds the real borrow (see
    /// [`Self::hawkes_holding_lending_protocol`]).
    fn close_hawkes_short_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.hawkes_holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_hawkes_short_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_hawkes_short_leg_solend(symbol, mint),
            None => log_warn!("multimodelv1: hawkes: close short leg {symbol} -- no open position found on either protocol"),
        }
    }

    /// Kamino half of opening the "long" leg of a hawkes trade for
    /// `symbol` -- swaps `notional_usd` of USDC into the underlying,
    /// deposits it into this mode's own obligation (real Kamino supply
    /// APY on top of the expected price convergence). Ported from
    /// `leveragedloopv1::state::open_kamino_deposit_leg`. No-op if a
    /// deposit already exists for this reserve. Dispatched to by
    /// `open_hawkes_long_leg` based on `best_supply_apy`.
    fn open_hawkes_long_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_hawkes_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_hawkes_kamino_obligation();
            return;
        }
        // See `m_hawkes_kamino_leg_action_this_cycle`'s own doc comment.
        if self.state.m_hawkes_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: hawkes: {symbol} has no Kamino oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_hawkes_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: hawkes: opening long leg {symbol} (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_hawkes_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, self.wallet) {
            log_error!("multimodelv1: hawkes: long leg {symbol} kamino deposit failed: {e}");
        }
    }

    /// Real, live-confirmed gap (2026-09-08): `current_open_hawkes` only
    /// recognizes an altcoin-denominated deposit/borrow (via
    /// `curated_symbols()`) as an open long/short leg -- a bare USDC
    /// deposit sitting in the USDC reserve is invisible to it (`mint ==
    /// mint_usdc` is explicitly skipped). This is exactly what's left
    /// behind when a short leg's atomic borrow-then-sell group fails
    /// before the borrow instruction itself lands: the USDC collateral
    /// (posted in an earlier, separate transaction) is stranded with no
    /// debt against it, and the normal close path never notices because
    /// it isn't looking at the USDC reserve at all. This narrow check
    /// (only Kamino's own reserves/obligation, no altcoin `mint`
    /// involved) withdraws that stranded USDC straight back to the
    /// wallet -- no repay (there's no debt) and no sell (it's already
    /// the target asset) -- whenever a USDC deposit exists with zero
    /// borrows anywhere in the obligation. A no-op otherwise, so it's
    /// safe to call unconditionally every cycle.
    fn close_hawkes_orphaned_usdc_collateral_kamino(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
        let Some(ob) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation()) else { return };
        if !ob.borrows.is_empty() {
            return; // real debt exists -- not orphaned, the normal close path owns this
        }
        if !ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount != 0) {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };

        log_warn!("multimodelv1: hawkes: withdrawing orphaned USDC collateral on Kamino (no debt against it)");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_hawkes_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, usdc_ata, self.wallet)
        {
            log_error!("multimodelv1: hawkes: orphaned USDC kamino withdraw failed: {e}");
        }
    }

    /// Solend counterpart to
    /// [`Self::close_hawkes_orphaned_usdc_collateral_kamino`] -- same
    /// reasoning, same "deposit with zero borrows anywhere" gate, just
    /// Solend's own reserve/obligation/withdraw shape (collateral-token
    /// ATA, `deposit_reserves` remaining-accounts list).
    fn close_hawkes_orphaned_usdc_collateral_solend(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let Some(ob) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation()) else { return };
        if !ob.borrows.is_empty() {
            return;
        }
        let Some(collateral_amount) = ob.deposit_for(reserve_id).map(|d| d.deposited_amount).filter(|&amt| amt != 0)
        else {
            return;
        };
        let collateral_mint = reserve.collateral_mint;
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        log_warn!("multimodelv1: hawkes: withdrawing orphaned USDC collateral on Solend (no debt against it)");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_obligation() {
            log_error!("multimodelv1: hawkes: orphaned USDC withdraw refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let deposit_reserves = self.hawkes_solend_obligation_deposit_reserves();
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            usdc_ata,
            collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: orphaned USDC solend withdraw failed: {e}");
        }
    }

    /// Kamino half of closing the long leg for `symbol` -- withdraws the
    /// real deposited amount, sells it back to USDC. Ported from
    /// `leveragedloopv1::state::close_kamino_deposit_leg`. Dispatched to
    /// by `close_hawkes_long_leg` based on `hawkes_holding_lending_protocol`.
    fn close_hawkes_long_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let has_deposit = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if !has_deposit {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };

        log_warn!("multimodelv1: hawkes: closing long leg {symbol}");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_hawkes_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: hawkes: close long leg {symbol} kamino withdraw failed: {e}");
            return;
        }
        let obligation_will_be_empty = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .is_some_and(|ob| ob.deposits.len() <= 1 && ob.borrows.is_empty());
        if obligation_will_be_empty {
            if let Some(pos) = self.state.o_hawkes_kamino_position.as_mut() {
                pos.mark_obligation_closing();
            }
        }
        let estimated_underlying_raw = ((HAWKES_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of opening the "short" leg of a hawkes trade for
    /// `symbol` -- two-stage: deposits USDC collateral first (if not
    /// already enough), then borrows the underlying and sells it for
    /// USDC (synthetic short) once enough collateral is confirmed.
    /// Ported from `leveragedloopv1::state::open_kamino_borrow_leg`.
    /// Dispatched to by `open_hawkes_short_leg` based on `best_borrow_apy`.
    fn open_hawkes_short_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_hawkes_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_hawkes_kamino_obligation();
            return;
        }
        // See `m_hawkes_kamino_leg_action_this_cycle`'s own doc comment.
        if self.state.m_hawkes_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let Some((_, borrow_reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let collateral_usd =
            notional_usd * borrow_reserve.borrow_factor_pct / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            self.state.m_hawkes_kamino_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: hawkes: depositing ${collateral_usd:.2} USDC collateral for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: hawkes: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            let (usdc_lending_market, usdc_farm_collateral) = (usdc_reserve.lending_market, usdc_reserve.farm_collateral);
            if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: hawkes: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.hawkes_kamino_refresh_obligation(usdc_lending_market) {
                log_error!("multimodelv1: hawkes: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            if !self.ensure_hawkes_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_collateral, 0) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) =
                usdc_reserve.deposit(usdc_reserve_id, obligation_id, usdc_amount_raw, owner, usdc_ata, self.wallet)
            {
                log_error!("multimodelv1: hawkes: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: hawkes: {symbol} has no Kamino oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_hawkes_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: hawkes: opening short leg {symbol} (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(
            usdc_reserve_id,
            usdc_reserve.pyth_oracle,
            usdc_reserve.switchboard_price_oracle,
            usdc_reserve.switchboard_twap_oracle,
            usdc_reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.hawkes_kamino_obligation_deposit_reserves();
        if let Err(e) = self.hawkes_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_hawkes_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.borrow(
            reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            underlying_ata,
            None,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: short leg {symbol} kamino borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of closing the short leg for `symbol` -- buys back the
    /// real, currently-borrowed amount with USDC, repays it. USDC
    /// collateral stays deposited for reuse. Ported from
    /// `leveragedloopv1::state::close_kamino_borrow_leg`. Dispatched to by
    /// `close_hawkes_short_leg` based on `hawkes_holding_lending_protocol`.
    fn close_hawkes_short_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_hawkes_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: hawkes: closing short leg {symbol}");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: hawkes: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: hawkes: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.hawkes_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_hawkes_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: hawkes: close short leg {symbol} repay failed: {e}");
        }
        // Real, live-confirmed repay-staleness bug (2026-09-08, Kamino
        // twin of the fix already applied to `close_hawkes_short_leg_
        // solend`): `underlying_balance_raw < borrowed_amount` above
        // trusts the fast-stream cached balance, which doesn't always
        // reflect that the immediately-preceding buy-back
        // (`execute_spot_leg`, an earlier cycle) actually drained/filled
        // this ATA. Confirmed on-chain: `RepayObligationLiquidityV2`
        // failing with `insufficient funds` (custom error 0x1) on every
        // attempt for 20+ minutes straight despite a real buy-back having
        // landed. `repay()` itself can't observe execution-time failures
        // (it only appends an instruction), so invalidate unconditionally
        // here -- forces a fresh re-derivation from the next real
        // account-update push instead of retrying forever against the
        // same wrong cached number.
        self.wallet.token_mut().invalidate(&owner, &mint);
    }

    // --- Trade type 5 point 2: Solend execution (added alongside Kamino,
    // same hawkes-trade decision logic, different lending protocol) --------

    /// Solend counterpart to `bootstrap_hawkes_kamino_obligation` -- simpler,
    /// no `init_user_metadata`/farm-equivalent pre-req for Solend. Real
    /// two-step account creation (`create_obligation_account` then
    /// `init_obligation`), unlike Kamino's single-step `init_obligation`
    /// -- Solend's Obligation isn't a PDA, see `solend::OBLIGATION_SEED`'s
    /// doc comment.
    fn bootstrap_hawkes_solend_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment (Hawkes/Solend twin of the same guard).
        if self.state.m_hawkes_solend_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_hawkes_solend_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((_, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            log_warn!("multimodelv1: hawkes: bootstrap: Solend USDC reserve not observed yet");
            return;
        };
        let lending_market = usdc_reserve.lending_market;
        log_warn!(
            "multimodelv1: hawkes: bootstrap: registering hawkes-trade Solend obligation (id={HAWKES_SOLEND_OBLIGATION_ID})"
        );
        if let Err(e) = solend::create_obligation_account(owner, HAWKES_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: hawkes: bootstrap: solend create_obligation_account failed: {e}");
            return;
        }
        if let Err(e) = solend::init_obligation(owner, lending_market, HAWKES_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: hawkes: bootstrap: solend init_obligation failed: {e}");
        }
    }

    fn hawkes_solend_obligation_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).chain(ob.borrows.iter().map(|b| b.borrow_reserve)).collect()
    }

    fn hawkes_solend_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    /// Refreshes every reserve this cycle's leg touches, mirroring
    /// `hawkes_kamino_refresh_all_reserves` exactly -- and for a real,
    /// documented reason beyond just consistency: this mode's hawkes-trade
    /// obligation genuinely holds multiple real reserves simultaneously
    /// (long deposit + short's USDC collateral + short's borrow, all on
    /// one obligation) -- see `pair_solend_refresh_all_reserves`'s own
    /// doc comment for the real, live-verified `ReserveStale` failure this
    /// avoids.
    fn hawkes_solend_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let reserves = self.hawkes_solend_obligation_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in reserves.iter().chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.solend().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(*reserve_id, self.wallet) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    /// Unlike Kamino's `refresh_obligation` (which takes `lending_market`
    /// as a real parameter), Solend's own `refresh_obligation` doesn't
    /// need it -- see `solend::refresh_obligation`'s own signature.
    fn hawkes_solend_refresh_obligation(&mut self) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("hawkes solend obligation not resolved yet".to_string());
        };
        let reserves = self.hawkes_solend_obligation_reserves();
        solend::refresh_obligation(obligation_id, &reserves, self.wallet).map_err(|e| e.to_string())
    }

    /// Solend counterpart to `open_hawkes_long_leg_kamino` -- see that
    /// function's own doc comment for the shared long-leg logic (swap
    /// USDC into the underlying, deposit as obligation collateral). Real
    /// API difference: Solend's `deposit` needs a scratch
    /// `user_collateral_account` ATA (`reserve.collateral_mint`) the
    /// deposited cTokens pass through -- Kamino mints cTokens straight
    /// into the obligation, no such account needed there.
    fn open_hawkes_long_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_hawkes_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_hawkes_solend_obligation();
            return;
        }
        // See `m_hawkes_solend_leg_action_this_cycle`'s own doc comment.
        if self.state.m_hawkes_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_hawkes_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: hawkes: {symbol} has no Solend oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        self.state.m_hawkes_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: hawkes: opening long leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_obligation() {
            log_error!("multimodelv1: hawkes: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, collateral_ata, self.wallet)
        {
            log_error!("multimodelv1: hawkes: long leg {symbol} solend deposit failed: {e}");
        }
    }

    /// Solend counterpart to `close_hawkes_long_leg_kamino` -- withdraws the
    /// real deposited amount (already in collateral/cToken units, read
    /// directly from the real obligation), sells it back to USDC. Real API
    /// differences from Kamino: needs a scratch collateral ATA plus the
    /// obligation's current `deposit_reserves` list (Solend's `withdraw`
    /// requires "borrow attribution" accounts Kamino's own withdraw
    /// doesn't).
    fn close_hawkes_long_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(collateral_amount) = self
            .state
            .o_hawkes_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .map(|d| d.deposited_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        log_warn!("multimodelv1: hawkes: closing long leg {symbol} via Solend");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_obligation() {
            log_error!("multimodelv1: hawkes: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let deposit_reserves = self.hawkes_solend_obligation_deposit_reserves();
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            underlying_ata,
            collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} solend withdraw failed: {e}");
            return;
        }
        let estimated_underlying_raw = ((HAWKES_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend counterpart to `open_hawkes_short_leg_kamino` -- same
    /// two-stage shape (deposit USDC collateral first, borrow once
    /// confirmed). Real API difference: the USDC collateral deposit also
    /// needs a scratch collateral ATA, and the borrow needs the
    /// obligation's current `deposit_reserves` list.
    fn open_hawkes_short_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_hawkes_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_hawkes_solend_obligation();
            return;
        }
        // See `m_hawkes_solend_leg_action_this_cycle`'s own doc comment.
        if self.state.m_hawkes_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let usdc_collateral_mint = usdc_reserve.collateral_mint;

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let collateral_usd = notional_usd / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_hawkes_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            let Some(usdc_collateral_ata) = self.wallet.append_create_ata(owner, usdc_collateral_mint) else {
                return;
            };
            self.state.m_hawkes_solend_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: hawkes: depositing ${collateral_usd:.2} USDC collateral (Solend) for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: hawkes: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: hawkes: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.hawkes_solend_refresh_obligation() {
                log_error!("multimodelv1: hawkes: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) = usdc_reserve.deposit(
                usdc_reserve_id,
                obligation_id,
                usdc_amount_raw,
                owner,
                usdc_ata,
                usdc_collateral_ata,
                self.wallet,
            ) {
                log_error!("multimodelv1: hawkes: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_hawkes_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: hawkes: {symbol} has no Solend oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_hawkes_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: hawkes: opening short leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.hawkes_solend_obligation_deposit_reserves();
        if let Err(e) = self.hawkes_solend_refresh_obligation() {
            log_error!("multimodelv1: hawkes: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.borrow(reserve_id, obligation_id, borrow_amount_raw, owner, underlying_ata, &deposit_reserves, self.wallet)
        {
            log_error!("multimodelv1: hawkes: short leg {symbol} solend borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: hawkes: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend counterpart to `close_hawkes_short_leg_kamino` -- buys back
    /// the real currently-borrowed amount, repays with
    /// `solend::SOLEND_AMOUNT_MAX`. USDC collateral stays deposited for
    /// reuse, matching the Kamino side's own precedent.
    fn close_hawkes_short_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_hawkes_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: hawkes: closing short leg {symbol} via Solend");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: hawkes: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: hawkes: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.hawkes_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.hawkes_solend_refresh_obligation() {
            log_error!("multimodelv1: hawkes: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, solend::SOLEND_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: hawkes: close short leg {symbol} repay failed: {e}");
        }
        // `repay` only appends an instruction -- it can't observe the
        // on-chain outcome, so `Err` above never fires for a real SPL
        // "insufficient funds" revert (confirmed live 2026-09-08: 20+
        // consecutive repay attempts failed this way over 30+ minutes,
        // each burning a real fee, because `underlying_balance_raw`
        // above kept reporting a stale nonzero WSOL balance -- left over
        // from the opening borrow -- long after the immediate follow-up
        // sell in `open_hawkes_short_leg_solend` had actually drained
        // that ATA back to zero on-chain). Forgetting the cached balance
        // unconditionally here breaks the "retry forever against the
        // same wrong cached number" loop: next cycle re-derives it fresh
        // (worst case treating it as 0 until a real update arrives,
        // which correctly routes back through the safe buy-back branch
        // above instead of another doomed direct repay).
        self.wallet.token_mut().invalidate(&owner, &mint);
    }

    // --- Trade type 1: directional factor-neutral execution (long a
    // human-specified token, short a real factor-loading basket) --
    // mirrors the pair trade's own Kamino/Solend leg plumbing above
    // line-for-line, retargeted at this trade type's own, fully
    // independent obligations. See `PLAN-1.md`'s directional-neutral
    // design notes for why entry is human-specified rather than
    // computed. -----------------------------------------------------

    fn bootstrap_directional_kamino_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment (directional twin of the same guard) -- this trade
        // type's open pass is exactly the shape that triggers it: one
        // target leg plus N basket legs dispatched in the same cycle.
        if self.state.m_directional_kamino_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_directional_kamino_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);
        let has_user_metadata =
            self.state.o_directional_kamino_position.as_ref().is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("multimodelv1: directional: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("multimodelv1: directional: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!(
            "multimodelv1: directional: bootstrap: registering directional obligation (id={DIRECTIONAL_KAMINO_OBLIGATION_ID})"
        );
        if let Err(e) = kamino::init_obligation(owner, lending_market, DIRECTIONAL_KAMINO_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: directional: bootstrap: kamino init_obligation failed: {e}");
        }
    }

    fn directional_kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    fn directional_kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    /// Ported from `pair_kamino_refresh_all_reserves` -- same reasoning
    /// (a basket obligation genuinely holds multiple simultaneous
    /// reserves: the long deposit plus every short leg's USDC collateral
    /// and borrow), generalizes with zero new design since it already
    /// refreshes *every* currently-held reserve, not a fixed count.
    fn directional_kamino_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let deposit_reserves = self.directional_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.directional_kamino_obligation_borrow_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in deposit_reserves.iter().chain(borrow_reserves.iter()).chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.kamino().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(
                *reserve_id,
                reserve.pyth_oracle,
                reserve.switchboard_price_oracle,
                reserve.switchboard_twap_oracle,
                reserve.scope_prices,
                self.wallet,
            ) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    fn directional_kamino_refresh_obligation(&mut self, lending_market: AccountId) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("directional kamino obligation not resolved yet".to_string());
        };
        let deposit_reserves = self.directional_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.directional_kamino_obligation_borrow_reserves();
        kamino::refresh_obligation(lending_market, obligation_id, &deposit_reserves, &borrow_reserves, self.wallet)
            .map_err(|e| e.to_string())
    }

    /// Ported from `ensure_pair_kamino_farm_ready`.
    fn ensure_directional_kamino_farm_ready(
        &mut self,
        reserve_id: AccountId,
        reserve_lending_market: AccountId,
        farm: Option<AccountId>,
        mode: u8,
    ) -> bool {
        let Some(farm) = farm else { return true };
        let Some(owner) = self.state.wallet() else { return false };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return false;
        };
        let Some(farm_user_state_id) = kamino::farm_user_state_id(farm, obligation_id) else { return false };
        let Some(directional_kamino_position) = self.state.o_directional_kamino_position.as_mut() else { return false };
        if let Err(e) = directional_kamino_position.track_farm_user_state(farm_user_state_id, self.graph) {
            log_error!("multimodelv1: directional: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if directional_kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!("multimodelv1: directional: bootstrapping Kamino farm-user-state for reserve {reserve_id}");
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: kamino init_obligation_farms_for_reserve failed: {e}");
        }
        false
    }

    /// Kamino half of opening trade type 1's long leg -- see
    /// `open_pair_long_leg_kamino`'s own doc comment for the shared
    /// logic; retargeted at this trade type's own obligation. Dispatched
    /// to by `open_directional_long_leg` based on `best_supply_apy`.
    fn open_directional_long_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_directional_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_directional_kamino_obligation();
            return;
        }
        // See `m_directional_kamino_leg_action_this_cycle`'s own doc
        // comment.
        if self.state.m_directional_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: directional: {symbol} has no Kamino oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_directional_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: directional: opening long leg {symbol} (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.directional_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_directional_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, self.wallet) {
            log_error!("multimodelv1: directional: long leg {symbol} kamino deposit failed: {e}");
        }
    }

    /// Directional counterpart to
    /// [`Self::close_hawkes_orphaned_usdc_collateral_kamino`] -- same
    /// gap (`current_open_directional` only recognizes altcoin-
    /// denominated deposits/borrows, not a bare USDC deposit), same fix:
    /// withdraw a stranded USDC deposit straight back to the wallet
    /// whenever no borrow exists anywhere in this obligation. No-op
    /// otherwise, safe to call unconditionally every cycle.
    fn close_directional_orphaned_usdc_collateral_kamino(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id())
        else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
        let Some(ob) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation()) else { return };
        if !ob.borrows.is_empty() {
            return;
        }
        if !ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount != 0) {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };

        log_warn!("multimodelv1: directional: withdrawing orphaned USDC collateral on Kamino (no debt against it)");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.directional_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_directional_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, usdc_ata, self.wallet)
        {
            log_error!("multimodelv1: directional: orphaned USDC kamino withdraw failed: {e}");
        }
    }

    /// Solend counterpart to
    /// [`Self::close_directional_orphaned_usdc_collateral_kamino`] --
    /// same reasoning, Solend's own reserve/obligation/withdraw shape.
    fn close_directional_orphaned_usdc_collateral_solend(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id())
        else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let Some(ob) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation()) else { return };
        if !ob.borrows.is_empty() {
            return;
        }
        let Some(collateral_amount) = ob.deposit_for(reserve_id).map(|d| d.deposited_amount).filter(|&amt| amt != 0)
        else {
            return;
        };
        let collateral_mint = reserve.collateral_mint;
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        log_warn!("multimodelv1: directional: withdrawing orphaned USDC collateral on Solend (no debt against it)");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_obligation() {
            log_error!("multimodelv1: directional: orphaned USDC withdraw refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let deposit_reserves = self.directional_solend_obligation_deposit_reserves();
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            usdc_ata,
            collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: orphaned USDC solend withdraw failed: {e}");
        }
    }

    /// Kamino half of closing trade type 1's long leg -- see
    /// `close_pair_long_leg_kamino`'s own doc comment. Dispatched to by
    /// `close_directional_long_leg` based on
    /// `directional_holding_lending_protocol`.
    fn close_directional_long_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let has_deposit = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if !has_deposit {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };

        log_warn!("multimodelv1: directional: closing long leg {symbol}");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.directional_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_directional_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: directional: close long leg {symbol} kamino withdraw failed: {e}");
            return;
        }
        let obligation_will_be_empty = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .is_some_and(|ob| ob.deposits.len() <= 1 && ob.borrows.is_empty());
        if obligation_will_be_empty {
            if let Some(pos) = self.state.o_directional_kamino_position.as_mut() {
                pos.mark_obligation_closing();
            }
        }
        let estimated_underlying_raw =
            ((DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of opening one of trade type 1's short (hedge-basket)
    /// legs -- see `open_pair_short_leg_kamino`'s own doc comment for the
    /// shared two-stage logic. Called once per basket leg by
    /// `run_directional_trade_cycle`'s open pass, each call independent
    /// (its own USDC-collateral sizing/check against this trade type's
    /// shared obligation) -- same pattern the pair trade's own single
    /// short leg already uses, just invoked in a loop here instead of
    /// once. Dispatched to by `open_directional_short_leg` based on
    /// `best_borrow_apy`.
    fn open_directional_short_leg_kamino(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_directional_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_directional_kamino_obligation();
            return;
        }
        // See `m_directional_kamino_leg_action_this_cycle`'s own doc
        // comment.
        if self.state.m_directional_kamino_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let Some((_, borrow_reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let collateral_usd =
            notional_usd * borrow_reserve.borrow_factor_pct / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            self.state.m_directional_kamino_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: directional: depositing ${collateral_usd:.2} USDC collateral for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: directional: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            let (usdc_lending_market, usdc_farm_collateral) = (usdc_reserve.lending_market, usdc_reserve.farm_collateral);
            if let Err(e) = self.directional_kamino_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: directional: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.directional_kamino_refresh_obligation(usdc_lending_market) {
                log_error!("multimodelv1: directional: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            if !self.ensure_directional_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_collateral, 0) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) =
                usdc_reserve.deposit(usdc_reserve_id, obligation_id, usdc_amount_raw, owner, usdc_ata, self.wallet)
            {
                log_error!("multimodelv1: directional: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: directional: {symbol} has no Kamino oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_directional_kamino_leg_action_this_cycle = true;
        log_warn!("multimodelv1: directional: opening short leg {symbol} (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(
            usdc_reserve_id,
            usdc_reserve.pyth_oracle,
            usdc_reserve.switchboard_price_oracle,
            usdc_reserve.switchboard_twap_oracle,
            usdc_reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Err(e) = self.directional_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.directional_kamino_obligation_deposit_reserves();
        if let Err(e) = self.directional_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_directional_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.borrow(
            reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            underlying_ata,
            None,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: short leg {symbol} kamino borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Kamino half of closing one of trade type 1's short legs -- see
    /// `close_pair_short_leg_kamino`'s own doc comment. Dispatched to by
    /// `close_directional_short_leg` based on
    /// `directional_holding_lending_protocol`.
    fn close_directional_short_leg_kamino(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_directional_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: directional: closing short leg {symbol}");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: directional: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("multimodelv1: directional: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.directional_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_kamino_refresh_obligation(lending_market) {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_directional_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: directional: close short leg {symbol} repay failed: {e}");
        }
        // Same repay-staleness fix as `close_hawkes_short_leg_kamino` --
        // see that function's own doc comment for the real, live-
        // confirmed failure mode this closes.
        self.wallet.token_mut().invalidate(&owner, &mint);
    }

    // --- Trade type 1 Solend legs ------------------------------------

    fn bootstrap_directional_solend_obligation(&mut self) {
        // See `m_pair_kamino_bootstrap_attempted_this_cycle`'s own doc
        // comment (directional/Solend twin of the same guard).
        if self.state.m_directional_solend_bootstrap_attempted_this_cycle {
            return;
        }
        self.state.m_directional_solend_bootstrap_attempted_this_cycle = true;
        let Some(owner) = self.state.wallet() else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((_, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            log_warn!("multimodelv1: directional: bootstrap: Solend USDC reserve not observed yet");
            return;
        };
        let lending_market = usdc_reserve.lending_market;
        log_warn!(
            "multimodelv1: directional: bootstrap: registering directional obligation (id={DIRECTIONAL_SOLEND_OBLIGATION_ID})"
        );
        if let Err(e) = solend::create_obligation_account(owner, DIRECTIONAL_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: directional: bootstrap: solend create_obligation_account failed: {e}");
            return;
        }
        if let Err(e) = solend::init_obligation(owner, lending_market, DIRECTIONAL_SOLEND_OBLIGATION_ID, self.wallet) {
            log_error!("multimodelv1: directional: bootstrap: solend init_obligation failed: {e}");
        }
    }

    fn directional_solend_obligation_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).chain(ob.borrows.iter().map(|b| b.borrow_reserve)).collect()
    }

    fn directional_solend_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    /// Ported from `pair_solend_refresh_all_reserves` -- same real
    /// reasoning (a basket obligation genuinely holds multiple
    /// simultaneous reserves), already generalizes with zero new design.
    fn directional_solend_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let reserves = self.directional_solend_obligation_reserves();
        let mut seen: HashSet<AccountId> = HashSet::new();
        for reserve_id in reserves.iter().chain(extra.iter()) {
            if !seen.insert(*reserve_id) {
                continue;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return Err("dex state not ready".to_string());
            };
            let Some(reserve) = dex.solend().reserve_by_id(*reserve_id) else {
                return Err(format!("reserve {reserve_id} not tracked -- cannot refresh"));
            };
            if let Err(e) = reserve.refresh_reserve(*reserve_id, self.wallet) {
                return Err(format!("refresh_reserve failed for {reserve_id}: {e}"));
            }
        }
        Ok(())
    }

    fn directional_solend_refresh_obligation(&mut self) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("directional solend obligation not resolved yet".to_string());
        };
        let reserves = self.directional_solend_obligation_reserves();
        solend::refresh_obligation(obligation_id, &reserves, self.wallet).map_err(|e| e.to_string())
    }

    /// Solend half of opening trade type 1's long leg -- see
    /// `open_pair_long_leg_solend`'s own doc comment. Dispatched to by
    /// `open_directional_long_leg` based on `best_supply_apy`.
    fn open_directional_long_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_directional_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_directional_solend_obligation();
            return;
        }
        // See `m_directional_solend_leg_action_this_cycle`'s own doc
        // comment.
        if self.state.m_directional_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_directional_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: directional: {symbol} has no Solend oracle price yet, skipping long leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 || usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        self.state.m_directional_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: directional: opening long leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: long leg {symbol} spot swap failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_obligation() {
            log_error!("multimodelv1: directional: long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, collateral_ata, self.wallet)
        {
            log_error!("multimodelv1: directional: long leg {symbol} solend deposit failed: {e}");
        }
    }

    /// Solend half of closing trade type 1's long leg -- see
    /// `close_pair_long_leg_solend`'s own doc comment. Dispatched to by
    /// `close_directional_long_leg` based on
    /// `directional_holding_lending_protocol`.
    fn close_directional_long_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(collateral_amount) = self
            .state
            .o_directional_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .map(|d| d.deposited_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let collateral_mint = reserve.collateral_mint;
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else { return };

        log_warn!("multimodelv1: directional: closing long leg {symbol} via Solend");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_obligation() {
            log_error!("multimodelv1: directional: close long leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let deposit_reserves = self.directional_solend_obligation_deposit_reserves();
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            underlying_ata,
            collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: close long leg {symbol} solend withdraw failed: {e}");
            return;
        }
        let estimated_underlying_raw =
            ((DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend half of opening one of trade type 1's short legs -- see
    /// `open_pair_short_leg_solend`'s own doc comment. Dispatched to by
    /// `open_directional_short_leg` based on `best_borrow_apy`.
    fn open_directional_short_leg_solend(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        if !self.state.o_directional_solend_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_directional_solend_obligation();
            return;
        }
        // See `m_directional_solend_leg_action_this_cycle`'s own doc
        // comment.
        if self.state.m_directional_solend_leg_action_this_cycle {
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
        let usdc_collateral_mint = usdc_reserve.collateral_mint;

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let collateral_usd = notional_usd / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_directional_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
            let Some(usdc_collateral_ata) = self.wallet.append_create_ata(owner, usdc_collateral_mint) else {
                return;
            };
            self.state.m_directional_solend_leg_action_this_cycle = true;
            log_warn!(
                "multimodelv1: directional: depositing ${collateral_usd:.2} USDC collateral (Solend) for {symbol} ${notional_usd:.2} short leg"
            );
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: directional: short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
            if let Err(e) = self.directional_solend_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("multimodelv1: directional: short leg {symbol} refresh_all_reserves failed: {e}");
                return;
            }
            if let Err(e) = self.directional_solend_refresh_obligation() {
                log_error!("multimodelv1: directional: short leg {symbol} refresh_obligation failed: {e}");
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) = usdc_reserve.deposit(
                usdc_reserve_id,
                obligation_id,
                usdc_amount_raw,
                owner,
                usdc_ata,
                usdc_collateral_ata,
                self.wallet,
            ) {
                log_error!("multimodelv1: directional: short leg {symbol} USDC collateral deposit failed: {e}");
            }
            return;
        }

        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_directional_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("multimodelv1: directional: {symbol} has no Solend oracle price yet, skipping short leg");
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        self.state.m_directional_solend_leg_action_this_cycle = true;
        log_warn!("multimodelv1: directional: opening short leg {symbol} via Solend (${notional_usd:.2})");
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: short leg {symbol} USDC refresh_reserve failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        let deposit_reserves = self.directional_solend_obligation_deposit_reserves();
        if let Err(e) = self.directional_solend_refresh_obligation() {
            log_error!("multimodelv1: directional: short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.borrow(
            reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            underlying_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("multimodelv1: directional: short leg {symbol} solend borrow failed: {e}");
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: directional: short leg {symbol} spot sell failed: {e}");
        }
    }

    /// Solend half of closing one of trade type 1's short legs -- see
    /// `close_pair_short_leg_solend`'s own doc comment. Dispatched to by
    /// `close_directional_short_leg` based on
    /// `directional_holding_lending_protocol`.
    fn close_directional_short_leg_solend(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_directional_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&amt| amt != 0)
        else {
            return;
        };
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw =
            ((borrowed_amount as f64 / 10f64.powi(decimals)) * price_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        let underlying_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!("multimodelv1: directional: closing short leg {symbol} via Solend");
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("multimodelv1: directional: close short leg {symbol} buy-back failed: {e}");
            }
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else { return };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_reserve failed: {e}");
            return;
        }
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!("multimodelv1: directional: close short leg {symbol} USDC refresh_reserve failed: {e}");
                return;
            }
        }
        if let Err(e) = self.directional_solend_refresh_all_reserves(&[reserve_id]) {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_all_reserves failed: {e}");
            return;
        }
        if let Err(e) = self.directional_solend_refresh_obligation() {
            log_error!("multimodelv1: directional: close short leg {symbol} refresh_obligation failed: {e}");
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, solend::SOLEND_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("multimodelv1: directional: close short leg {symbol} repay failed: {e}");
        }
    }

    /// Opens trade type 1's long leg -- dispatches to whichever of
    /// Solend/Kamino currently offers the best real supply APY for
    /// `mint` (see [`Self::best_supply_apy`]), same shape as
    /// `open_pair_long_leg`.
    fn open_directional_long_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_supply_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_directional_long_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_directional_long_leg_solend(symbol, mint, notional_usd),
            None => {
                log_warn!("multimodelv1: directional: open long leg {symbol} -- no real supply APY on either protocol yet")
            }
        }
    }

    /// Closes trade type 1's long leg -- dispatches to whichever
    /// protocol actually holds the real deposit (see
    /// [`Self::directional_holding_lending_protocol`]).
    fn close_directional_long_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.directional_holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_directional_long_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_directional_long_leg_solend(symbol, mint),
            None => {
                log_warn!("multimodelv1: directional: close long leg {symbol} -- no open position found on either protocol")
            }
        }
    }

    /// Opens one of trade type 1's short (hedge-basket) legs --
    /// dispatches to whichever of Solend/Kamino currently offers the
    /// cheapest real borrow APY for `mint` (see [`Self::best_borrow_apy`]),
    /// same shape as `open_pair_short_leg`.
    fn open_directional_short_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        match self.best_borrow_apy(mint) {
            Some((LendingProtocol::Kamino, _)) => self.open_directional_short_leg_kamino(symbol, mint, notional_usd),
            Some((LendingProtocol::Solend, _)) => self.open_directional_short_leg_solend(symbol, mint, notional_usd),
            None => {
                log_warn!("multimodelv1: directional: open short leg {symbol} -- no real borrow APY on either protocol yet")
            }
        }
    }

    /// Closes one of trade type 1's short legs -- dispatches to whichever
    /// protocol actually holds the real borrow (see
    /// [`Self::directional_holding_lending_protocol`]).
    fn close_directional_short_leg(&mut self, symbol: &str, mint: AccountId) {
        match self.directional_holding_lending_protocol(mint) {
            Some(LendingProtocol::Kamino) => self.close_directional_short_leg_kamino(symbol, mint),
            Some(LendingProtocol::Solend) => self.close_directional_short_leg_solend(symbol, mint),
            None => {
                log_warn!("multimodelv1: directional: close short leg {symbol} -- no open position found on either protocol")
            }
        }
    }

    /// Opens one of trade type 3's (dispersion) long-basket legs -- a real
    /// spot buy via `execute_spot_leg`, held as a plain wallet balance, no
    /// lending deposit (see the module's design notes on why dispersion's
    /// long legs deliberately skip the extra-yield deposit step every
    /// other trade type in this file takes -- up to `dispersion_basket::
    /// DISPERSION_BASKET_SIZE` legs at once is already more obligation
    /// surface than this mode wants to track). No open-once gate is
    /// needed the way `open_directional_long_leg` needs -- `run_
    /// dispersion_trade_cycle`'s open-pass only calls this once per leg
    /// per cycle, already guarded by `current_open_dispersion` being
    /// empty.
    fn open_dispersion_long_leg(&mut self, symbol: &str, mint: AccountId, notional_usd: f64) {
        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_amount_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        log_warn!("multimodelv1: dispersion: opening long leg {symbol} (${notional_usd:.2})");
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: dispersion: long leg {symbol} spot buy failed: {e}");
        }
    }

    /// Closes one of trade type 3's long-basket legs -- sells the real,
    /// currently-held wallet balance of `mint` back to USDC. Reads the
    /// real balance directly (same pattern `current_usdc_value` uses for
    /// USDC) rather than an intended/estimated amount -- there's no
    /// lending obligation recording how much was actually deposited, the
    /// wallet balance itself *is* the position (see `current_open_
    /// dispersion`'s own doc comment for why this trade type's restart-
    /// safety works this way).
    fn close_dispersion_long_leg(&mut self, symbol: &str, mint: AccountId) {
        let Some(owner) = self.state.wallet() else { return };
        // `is_final=false`, matching every other real balance read in this
        // file (e.g. size_pair_legs/size_directional_legs) -- reads the
        // fast (~400ms) low-latency stream, falling back to the rooted
        // (~12s+) one only if the fast stream has nothing yet. Real,
        // live-confirmed bug fixed here (2026-09-03): this used to pass
        // `true`, which `TokenDatabase::balance` reads *exclusively* from
        // the rooted stream (no fallback the other way) -- so this stayed
        // blind to a real, already-landed sell for as long as the rooted
        // stream took to catch up (observed: 66+ seconds, several close-pass
        // cycles), repeatedly re-selling the same stale amount and getting
        // real `insufficient funds` failures against a position that was
        // already gone.
        let held_raw: u64 = self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if held_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        log_warn!("multimodelv1: dispersion: closing long leg {symbol}");
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, held_raw, LOOP_MAX_HOPS) {
            log_error!("multimodelv1: dispersion: close long leg {symbol} spot sell failed: {e}");
        }
    }

    /// Temporary, standalone manual-cleanup tool -- see
    /// `CustomMessageInbound::TriggerSweepMint`'s doc comment for the
    /// real incident this exists to clean up (a hop-chain send stranding
    /// a real balance in a pass-through intermediate mint `mid_on_tx`
    /// never saw land). Consumes `state.o_sweep_mint_requested` via
    /// `.take()` -- one-shot: a failed sweep (e.g. no route, real balance
    /// still zero this tick) is not retried automatically, since this is
    /// an explicit human request, not part of any trade type's own
    /// automated retry cadence. Resend `TriggerSweepMint` to try again.
    ///
    /// Real, live-confirmed incident (2026-09-04): a stranded mint is, by
    /// definition, one this process may never have subscribed to before
    /// (a fresh process inherits no subscriptions from any prior one,
    /// same as it inherits no router edges -- see `TriggerSweepMint`'s
    /// own `--sweep-mint` doc comment) -- so `TokenDatabase::balance`
    /// correctly reported a real 0 for a mint the wallet actually held
    /// 7244+ real tokens of, because nothing had ever subscribed to that
    /// ATA in this process's lifetime. Subscribing here first (same
    /// `ata_subscribe_request`/`subscribe_now` pattern `send_single_hop_
    /// as_astralane_tx` already uses for the analogous mid_on_tx gap)
    /// doesn't make the balance available *this* tick -- a real
    /// subscription still needs real time for its first update to
    /// arrive -- but it means a second `TriggerSweepMint` shortly after
    /// will correctly see it, instead of every attempt reporting zero
    /// forever.
    fn sweep_requested_mint(&mut self) {
        let Some((mint, dest_mint)) = self.state.o_sweep_mint_requested.take() else { return };
        let Some(owner) = self.state.wallet() else { return };
        if let Some(sub_req) = self.wallet.ata_subscribe_request(owner, mint) {
            match SubscriptionQueue::subscribe_now(self.graph, vec![sub_req]) {
                Ok(subs) => self.wallet.keep_ata_subscriptions(subs),
                Err(e) => log_error!("multimodelv1: TriggerSweepMint -- failed to subscribe to mint {mint}'s ATA: {e}"),
            }
        }
        let held_raw: u64 = self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
        if held_raw == 0 {
            log_warn!(
                "multimodelv1: TriggerSweepMint -- real balance of mint {mint} is 0 (or not yet known -- just subscribed to its ATA, resend TriggerSweepMint shortly if this mint is real), nothing to sweep",
            );
            return;
        }
        log_warn!("multimodelv1: TriggerSweepMint -- sweeping {held_raw} raw unit(s) of mint {mint} to {dest_mint}");
        if let Err(e) = self.execute_spot_leg(mint, dest_mint, held_raw, LOOP_MAX_HOPS) {
            let (acked, sent) = crate::graph::subscription_ack_counts();
            log_error!(
                "multimodelv1: TriggerSweepMint -- sweep of mint {mint} to {dest_mint} failed: {e} \
                 (subscriptions: {acked}/{sent} acked)",
            );
        }
    }

    /// Live Phoenix position size for dispersion's own index market
    /// (`DISPERSION_INDEX_SYMBOL`), signed (`> 0` long, `< 0` short --
    /// dispersion only ever opens a short, but a real account read is
    /// checked as-is, not assumed). Mirrors `perpfundingv1::state::
    /// phoenix_position` exactly, hardcoded to the one market this trade
    /// type ever trades (that mode's own version is generic across
    /// symbols since it trades several).
    fn dispersion_phoenix_position(&self) -> Option<i64> {
        let phoenix = self.state.o_phoenix.as_ref()?;
        let market = phoenix.markets().iter().find(|m| m.symbol_str() == DISPERSION_INDEX_SYMBOL)?;
        let pos = phoenix.positions().iter().find(|p| p.asset_id as u32 == market.asset_id)?;
        (pos.base_lot_position != 0).then_some(pos.base_lot_position)
    }

    /// One-time bootstrap for dispersion's own Phoenix trader account:
    /// `register_trader`, convert USDC -> PhUSD via Ember, then `deposit_
    /// funds` as starting margin collateral -- batched into a single
    /// transaction, mirrored verbatim from `perpfundingv1::state::
    /// bootstrap_phoenix_trader` (same real reasoning: Solana executes a
    /// transaction's instructions sequentially, so `deposit_funds` can
    /// safely reference the account `register_trader` just created).
    /// Budget is `DISPERSION_BOOTSTRAP_MARGIN_USD`, bounded by real
    /// current USDC -- deliberately small; `top_up_dispersion_margin`
    /// covers the real gap to whatever a specific short actually needs,
    /// right before it's placed. Called instead of placing an order --
    /// see `open_dispersion_short_index_leg`'s call site -- so the first
    /// capital-feasible cycle after a fresh wallet is spent on setup, not
    /// a real position.
    fn bootstrap_dispersion_phoenix_trader(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        if self.state.o_phoenix.as_ref().and_then(|p| p.trader_account()).is_none() {
            log_warn!("multimodelv1: dispersion: bootstrap: phoenix trader_account PDA not known yet -- set_authority hasn't run");
            return;
        }
        let budget_usd = DISPERSION_BOOTSTRAP_MARGIN_USD.min(self.available_usdc_value());
        if budget_usd <= 0.0 {
            log_warn!("multimodelv1: dispersion: bootstrap: no spare USDC to fund the Phoenix trader account yet");
            return;
        }
        const USDC_DECIMALS: i32 = 6;
        let amount_raw = (budget_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        let mint_usdc = self.configuration.mint_usdc;
        let Some(phusd_mint_pk) = self.state.o_phoenix.as_ref().map(|p| p.canonical_mint()) else {
            return;
        };
        let phusd_mint_id = account_id_from_pubkey(&phusd_mint_pk);
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return };
        let Some(phusd_ata) = self.wallet.append_create_ata(owner, phusd_mint_id) else { return };

        log_warn!("multimodelv1: dispersion: bootstrap: registering + funding Phoenix trader account (${budget_usd:.2})");
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        if let Err(e) = phoenix.register_trader(owner, self.wallet) {
            log_error!("multimodelv1: dispersion: bootstrap: phoenix register_trader failed: {e}");
            return;
        }
        if let Err(e) = ember::deposit(owner, phusd_mint_id, usdc_ata, phusd_ata, amount_raw, self.wallet) {
            log_error!("multimodelv1: dispersion: bootstrap: ember deposit failed: {e}");
            return;
        }
        if let Err(e) = phoenix.deposit_funds(owner, phusd_ata, amount_raw, self.wallet) {
            log_error!("multimodelv1: dispersion: bootstrap: phoenix deposit_funds failed: {e}");
        }
    }

    /// Tops up dispersion's Phoenix margin (USDC -> PhUSD via Ember, then
    /// `deposit_funds`, same two-instruction shape `bootstrap_dispersion_
    /// phoenix_trader` uses for its own first-ever deposit) so real
    /// current collateral (`PhoenixState::collateral_quote_lots`, real
    /// on-chain state) covers `target_notional_usd` before a short is
    /// placed against it -- the real gap `bootstrap_dispersion_phoenix_
    /// trader`'s deliberately-small starting budget leaves. Uses `dex::
    /// phoenix::margin::required_margin_usd_for_notional` (Phase 2) for
    /// the real leverage-aware requirement rather than a guessed
    /// multiple of notional. Returns `true` once collateral already
    /// covers the requirement (including immediately after a real top-up
    /// attempt is sent -- the deposit itself lands asynchronously, same
    /// "queued, not yet confirmed" semantics every other real transaction
    /// in this file has); `false` if the requirement can't be computed
    /// yet (unpriced market) or there's no spare USDC to top up with.
    fn top_up_dispersion_margin(&mut self, target_notional_usd: f64) -> bool {
        let Some(owner) = self.state.wallet() else { return false };
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return false };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == DISPERSION_INDEX_SYMBOL) else {
            return false;
        };
        let Some(required_usd) = phoenix::margin::required_margin_usd_for_notional(market, target_notional_usd) else {
            log_warn!("multimodelv1: dispersion: margin top-up: {DISPERSION_INDEX_SYMBOL} not priced yet, can't size required margin");
            return false;
        };
        let current_usd = phoenix::margin::quote_lots_to_usd(phoenix.collateral_quote_lots());
        if current_usd >= required_usd {
            return true;
        }
        // Real, live-confirmed incident (2026-09-04): a frozen trader
        // account fails `DepositFunds` on-chain with `TradersViewError::
        // CapabilityDenied { capability: DepositCollateral }` /
        // `TradersViewError::TraderFrozen` -- the exact same failure
        // every single retry, burning a real transaction fee each time
        // with zero chance of success. Checking this up front (now that
        // `PhoenixState` actually parses the real capability-flags field
        // -- see `accounts::OFF_TH_CAPABILITY_FLAGS`'s doc comment) turns
        // that into one clear log line instead of an endless doomed
        // retry loop.
        if phoenix.is_trader_frozen() {
            log_warn!(
                "multimodelv1: dispersion: margin top-up: trader account is frozen (real on-chain capability flags deny DepositCollateral) -- refusing to attempt a deposit that would only fail",
            );
            return false;
        }
        // `phoenix`/`market` (borrowed from `self.state.o_phoenix`) must
        // be dropped before `current_usdc_value()` (needs `&mut self`) --
        // nothing left to read from either borrow past this point, so
        // re-derive `phusd_mint_pk` from a fresh borrow after.
        let phusd_mint_pk = phoenix.canonical_mint();
        let gap_usd = (required_usd - current_usd).min(self.available_usdc_value());
        if gap_usd <= 0.0 {
            log_warn!(
                "multimodelv1: dispersion: margin top-up: need ${:.2} more but no spare USDC (have ${:.2}, need ${required_usd:.2})",
                required_usd - current_usd,
                current_usd,
            );
            return false;
        }
        const USDC_DECIMALS: i32 = 6;
        let amount_raw = (gap_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_raw == 0 {
            return false;
        }
        let mint_usdc = self.configuration.mint_usdc;
        let phusd_mint_id = account_id_from_pubkey(&phusd_mint_pk);
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else { return false };
        let Some(phusd_ata) = self.wallet.append_create_ata(owner, phusd_mint_id) else { return false };
        log_warn!("multimodelv1: dispersion: margin top-up: depositing ${gap_usd:.2} more PhUSD (current ${current_usd:.2}, need ${required_usd:.2})");
        if let Err(e) = ember::deposit(owner, phusd_mint_id, usdc_ata, phusd_ata, amount_raw, self.wallet) {
            log_error!("multimodelv1: dispersion: margin top-up: ember deposit failed: {e}");
            return false;
        }
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return false };
        if let Err(e) = phoenix.deposit_funds(owner, phusd_ata, amount_raw, self.wallet) {
            log_error!("multimodelv1: dispersion: margin top-up: phoenix deposit_funds failed: {e}");
            return false;
        }
        true
    }

    /// Opens dispersion's short-index leg (always a short -- `Side::Ask`
    /// -- against `DISPERSION_INDEX_SYMBOL`). Mirrors `perpfundingv1::
    /// state::open_phoenix_leg`'s real shape (bootstrap-instead-of-trade
    /// if not registered, open-once gate via `dispersion_phoenix_
    /// position`, sizing via `mark_price_usd()`), with one addition that
    /// mode never needed: a real margin top-up (`top_up_dispersion_
    /// margin`) before placing the order, since dispersion's short
    /// notional varies basket to basket instead of perpfundingv1's own
    /// fixed, small basis-trade scale.
    fn open_dispersion_short_index_leg(&mut self, notional_usd: f64) {
        let Some(owner) = self.state.wallet() else { return };
        if !self.state.o_phoenix.as_ref().is_some_and(|p| p.trader_registered()) {
            self.bootstrap_dispersion_phoenix_trader();
            return;
        }
        if self.dispersion_phoenix_position().is_some() {
            return;
        }
        if !self.top_up_dispersion_margin(notional_usd) {
            log_warn!("multimodelv1: dispersion: short index leg -- real margin not yet sufficient, refusing to open this cycle");
            return;
        }
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == DISPERSION_INDEX_SYMBOL) else {
            return;
        };
        let Some(price_usd) = market.mark_price_usd() else {
            log_error!("multimodelv1: dispersion: {DISPERSION_INDEX_SYMBOL} has no oracle price yet, skipping short index leg");
            return;
        };
        let num_base_lots = ((notional_usd / price_usd) * 10f64.powi(market.base_lot_decimals as i32)).round() as u64;
        if num_base_lots == 0 {
            return;
        }
        let asset_id = market.asset_id;
        log_warn!(
            "multimodelv1: dispersion: opening short index leg {DISPERSION_INDEX_SYMBOL} num_base_lots={num_base_lots} (notional=${notional_usd:.2})"
        );
        if let Err(e) = phoenix.place_market_order(owner, asset_id, Side::Ask, num_base_lots, 0, 0, self.wallet) {
            log_error!("multimodelv1: dispersion: short index leg failed: {e}");
        }
    }

    /// Flattens dispersion's live short-index position to zero via an
    /// opposite-side (buy) market order -- mirrors `perpfundingv1::
    /// state::close_phoenix_leg` exactly. No-op if nothing's open.
    fn close_dispersion_short_index_leg(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == DISPERSION_INDEX_SYMBOL) else {
            return;
        };
        let Some(base_lot_position) = self.dispersion_phoenix_position() else { return };
        let asset_id = market.asset_id;
        let side = if base_lot_position > 0 { Side::Ask } else { Side::Bid };
        let size = base_lot_position.unsigned_abs();
        let client_order_id = self.state.last_slot as u128;
        log_warn!("multimodelv1: dispersion: closing short index leg {DISPERSION_INDEX_SYMBOL} size={size}");
        if let Err(e) = phoenix.place_market_order(owner, asset_id, side, size, 0, client_order_id, self.wallet) {
            log_error!("multimodelv1: dispersion: close short index leg failed: {e}");
        }
    }

    /// Real, on-chain-derived "what pair is currently open" -- re-derives
    /// identity from the obligation's own real deposit/borrow reserve
    /// lists (excluding USDC, which is short-leg collateral, not a
    /// "long" position) rather than tracking it as separate local state,
    /// same "re-derive from real reads" discipline every other bot mode
    /// here already follows for its own position state.
    ///
    /// **"Spike vs. basket" redesign**: generalized from "exactly 1 long
    /// + 1 short" to `Vec`s on both sides, same shape
    /// `current_open_directional` already uses for its own "1 long + N
    /// shorts" -- but here *either* side can be the multi-leg one
    /// (`find_spike_candidate`'s `side` decides which cycle to cycle: an
    /// `Underperformer` spike is long-1/short-N, an `Overperformer` spike
    /// is long-N/short-1). `Some((longs, shorts))` whenever at least one
    /// real leg exists on either side -- even a lopsided/empty one, since
    /// that's a real broken/partial open the caller's close-pass must
    /// force-close, same fail-closed discipline as before. `None` only
    /// when nothing at all is open.
    fn current_open_pair(&self) -> Option<(Vec<(&'static str, AccountId)>, Vec<(&'static str, AccountId)>)> {
        let dex = self.state.o_dex.as_ref()?;
        let kamino_ob = self.state.o_pair_kamino_position.as_ref().and_then(|s| s.obligation());
        let solend_ob = self.state.o_pair_solend_position.as_ref().and_then(|s| s.obligation());
        let mint_usdc = self.configuration.mint_usdc;
        let mut long_legs: Vec<(&'static str, AccountId)> = Vec::new();
        let mut short_legs: Vec<(&'static str, AccountId)> = Vec::new();
        for (symbol, mint, _) in curated_symbols() {
            if mint == mint_usdc {
                continue;
            }
            match self.holding_lending_protocol(mint) {
                Some(LendingProtocol::Kamino) => {
                    let Some(ob) = kamino_ob else { continue };
                    let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_legs.push((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                Some(LendingProtocol::Solend) => {
                    let Some(ob) = solend_ob else { continue };
                    let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_legs.push((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                None => {}
            }
        }
        if long_legs.is_empty() && short_legs.is_empty() {
            return None;
        }
        Some((long_legs, short_legs))
    }

    /// Trade type 5's (Hawkes) own twin to [`Self::current_open_pair`] --
    /// same real, re-derived-from-on-chain-state shape (arbitrary N long
    /// legs + M short legs, unlike directional's fixed "1 long + N
    /// shorts"), checking `o_hawkes_kamino_position`/`o_hawkes_solend_
    /// position` via [`Self::hawkes_holding_lending_protocol`] instead of
    /// the pair trade's own fields. Unlike the pair trade (which treats
    /// "not exactly 1 spike + a real basket" as broken), a Hawkes basket
    /// has no fixed long/short split -- an all-long or all-short cycle is
    /// a real, valid outcome of `hawkes_factor::factor_jump_basket`, not
    /// broken. `None` only when nothing at all is open on either side.
    fn current_open_hawkes(&self) -> Option<(Vec<(&'static str, AccountId)>, Vec<(&'static str, AccountId)>)> {
        let dex = self.state.o_dex.as_ref()?;
        let kamino_ob = self.state.o_hawkes_kamino_position.as_ref().and_then(|s| s.obligation());
        let solend_ob = self.state.o_hawkes_solend_position.as_ref().and_then(|s| s.obligation());
        let mint_usdc = self.configuration.mint_usdc;
        let mut long_legs: Vec<(&'static str, AccountId)> = Vec::new();
        let mut short_legs: Vec<(&'static str, AccountId)> = Vec::new();
        for (symbol, mint, _) in curated_symbols() {
            if mint == mint_usdc {
                continue;
            }
            match self.hawkes_holding_lending_protocol(mint) {
                Some(LendingProtocol::Kamino) => {
                    let Some(ob) = kamino_ob else { continue };
                    let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_legs.push((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                Some(LendingProtocol::Solend) => {
                    let Some(ob) = solend_ob else { continue };
                    let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_legs.push((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                None => {}
            }
        }
        if long_legs.is_empty() && short_legs.is_empty() {
            return None;
        }
        Some((long_legs, short_legs))
    }

    /// Trade type 1's own generalization of [`Self::current_open_pair`]
    /// -- same real, re-derived-from-on-chain-state discipline, but "1
    /// long + N shorts" instead of "1 long + 1 short": `shorts` is a
    /// `Vec`, not an `Option`, since a real hedge basket can (and
    /// usually will) have more than one short leg. `Some((long, shorts))`
    /// is returned whenever a real long deposit exists, **even if
    /// `shorts` is empty** -- an empty-shorts basket with a real long
    /// deposit is a broken/partial open (e.g. the long leg landed but a
    /// short leg's transaction failed), not "nothing open"; the caller's
    /// close-pass is responsible for treating that as broken and force
    /// -closing, matching this mode's fail-closed discipline everywhere
    /// else. `None` only when there's no real long deposit at all.
    fn current_open_directional(&self) -> Option<((&'static str, AccountId), Vec<(&'static str, AccountId)>)> {
        let dex = self.state.o_dex.as_ref()?;
        let kamino_ob = self.state.o_directional_kamino_position.as_ref().and_then(|s| s.obligation());
        let solend_ob = self.state.o_directional_solend_position.as_ref().and_then(|s| s.obligation());
        let mint_usdc = self.configuration.mint_usdc;
        let mut long_leg = None;
        let mut short_legs: Vec<(&'static str, AccountId)> = Vec::new();
        for (symbol, mint, _) in curated_symbols() {
            if mint == mint_usdc {
                continue;
            }
            match self.directional_holding_lending_protocol(mint) {
                Some(LendingProtocol::Kamino) => {
                    let Some(ob) = kamino_ob else { continue };
                    let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_leg = Some((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                Some(LendingProtocol::Solend) => {
                    let Some(ob) = solend_ob else { continue };
                    let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) else { continue };
                    if ob.deposit_for(reserve_id).is_some_and(|d| d.deposited_amount > 0) {
                        long_leg = Some((symbol, mint));
                    }
                    if ob.borrow_for(reserve_id).is_some_and(|b| b.borrowed_amount > 0) {
                        short_legs.push((symbol, mint));
                    }
                }
                None => {}
            }
        }
        long_leg.map(|l| (l, short_legs))
    }

    /// Real per-cycle update: for each curated symbol with a real Kamino
    /// oracle price, computes a real percent return since the last
    /// resync, decomposes it into a real residual via
    /// `factor_residual::compute_residuals` against `factors` (projected
    /// onto the leading `RESIDUAL_FACTOR_COUNT` factors -- see that
    /// constant's own doc comment), pushes it into that symbol's own
    /// rolling window, and returns the resulting current z-scores.
    /// Symbols without enough real history yet, or without a real price
    /// this cycle, are simply absent -- not a zero z-score.
    fn update_residual_history_and_get_current(
        &mut self,
        factors: &factor_graph::StructuralFactors,
    ) -> Vec<factor_residual::SymbolResidual> {
        let symbols: Vec<(&'static str, AccountId, u8)> = curated_symbols().collect();
        let Some(dex) = self.state.o_dex.as_ref() else { return Vec::new() };

        // Phase A: collect real prices while `dex` is borrowed, so this
        // borrow doesn't need to stay held across the `&mut self` writes
        // in Phase B below -- same discipline this codebase's Kamino-leg
        // porting already established for this exact shape.
        let mut prices: Vec<Option<f64>> = Vec::with_capacity(symbols.len());
        for (_, mint, _) in &symbols {
            // Kamino-first, Solend-fallback: the trade-universe union
            // includes mints that only have a Solend main-market reserve
            // (Kamino ∪ Solend, see `trade_universe_config`'s doc
            // comment), so a Kamino-only lookup would silently drop them
            // from real residual history entirely.
            let price = dex
                .kamino()
                .reserve_by_mint(*mint)
                .map(|(_, r)| r.price_usd)
                .or_else(|| dex.solend().reserve_by_mint(*mint).map(|(_, r)| r.price_usd));
            prices.push(price.filter(|&p| p > 0.0));
        }

        // Phase B: real percent returns against the last observed price.
        let mut returns = vec![0.0; symbols.len()];
        let mut have_return = vec![false; symbols.len()];
        for (i, (_, mint, _)) in symbols.iter().enumerate() {
            let Some(price) = prices[i] else { continue };
            if let Some(&prev) = self.state.m_last_price_usd.get(mint) {
                if prev > 0.0 {
                    returns[i] = ((price - prev) / prev) * 100.0;
                    have_return[i] = true;
                }
            }
            self.state.m_last_price_usd.insert(*mint, price);
        }
        if !have_return.iter().any(|&h| h) {
            return Vec::new();
        }

        // Fix (2026-09-07, revised): `factors.real_eigenvectors()` skips
        // every real trivial column, not a hardcoded single column --
        // see that method's own doc comment. This bot's real
        // router-coverage-built graph is live-observed to have *many*
        // disconnected components (60 of 147 curated tokens' own
        // components on a real cycle), so the Laplacian's trivial
        // eigenspace is far wider than the one column a *connected*
        // graph would have. Every consumer below used to read
        // `[0..RESIDUAL_FACTOR_COUNT]` directly, silently treating a
        // trivial mode as "factor 0" -- live-observed this systematically
        // favored thinly-connected/illiquid proxy candidates in
        // `build_directional_basket`'s max-|loading| selection (see
        // `docs/HAWKES_FACTOR_TRADE_PLAN.md`'s sibling investigation
        // notes).
        let real_eigenvectors: Vec<Vec<f64>> = factors.real_eigenvectors();

        let residuals = factor_residual::compute_residuals(&returns, &real_eigenvectors, RESIDUAL_FACTOR_COUNT);
        let mut out = Vec::new();
        for (i, (symbol, mint, _)) in symbols.iter().enumerate() {
            if !have_return[i] {
                continue;
            }
            let window = self
                .state
                .m_residual_history
                .entry(*mint)
                .or_insert_with(|| factor_residual::RollingWindow::new(RESIDUAL_WINDOW_CAPACITY));
            window.push(residuals[i]);
            if let Some(zscore) = window.zscore(residuals[i]) {
                out.push(factor_residual::SymbolResidual { symbol, mint: *mint, residual_pct: residuals[i], zscore });
            }
        }

        // Real background update for trade type 2's redesigned "spike
        // vs. basket" half-life gate -- see `m_pair_spread_history`'s own
        // doc comment. Computes each symbol's own basket-hedged spread
        // this cycle and pushes it, for every symbol with a real
        // residual this cycle, not just whichever one later turns out to
        // be a real spike candidate -- so warm-up proceeds with no extra
        // cold-start delay once one actually spikes.
        let basket_source: Vec<pair_basket::BasketMember> = curated_symbols()
            .filter_map(|(symbol, mint, _)| {
                let stdev = self.state.m_residual_history.get(&mint)?.stats()?.stdev;
                Some(pair_basket::BasketMember { symbol, mint, residual_stdev: stdev })
            })
            .collect();
        for r in &out {
            let Some(members) = pair_basket::select_pair_basket_members(&basket_source, r.mint) else { continue };
            let Some(basket_avg) = pair_basket::basket_average_residual(&out, &members) else { continue };
            let window = self
                .state
                .m_pair_spread_history
                .entry(r.mint)
                .or_insert_with(|| factor_residual::RollingWindow::new(RESIDUAL_WINDOW_CAPACITY));
            window.push(r.residual_pct - basket_avg);
        }

        // Real background update for trade type 5 (Hawkes-on-eigenfactor
        // momentum) -- see `hawkes_factor`'s own module doc comment /
        // `docs/HAWKES_FACTOR_TRADE_PLAN.md` for the full design.
        // Log-only this phase: no trading decision reads
        // `m_factor_intensity`/`m_factor_intensity_lambda_history` yet
        // (that's `run_hawkes_trade_cycle`, not yet wired into
        // `run_factor_resync`) -- this just observes real `lambda`
        // behavior live before any real position opens against it.
        let n_factors = self.state.m_factor_intensity.len();
        if n_factors > 0 {
            // Phase A: build each factor's real per-cycle jump list from
            // every symbol with BOTH a real residual this cycle AND a
            // real previous-cycle z-score (`m_last_zscore`) -- a symbol's
            // first-ever appearance has no real delta to compute yet, so
            // it's excluded rather than treated as a fabricated jump from
            // an assumed-zero baseline.
            let mut jumps_per_factor: Vec<Vec<hawkes_factor::SymbolJump>> = vec![Vec::new(); n_factors];
            for r in &out {
                let Some(prev_zscore) = self.state.m_last_zscore.get(&r.mint).copied() else { continue };
                let delta_zscore = r.zscore - prev_zscore;
                let Some(token_index) = self.curated_index_of(r.mint) else { continue };
                let Some(row) = real_eigenvectors.get(token_index) else { continue };
                for (f, jumps) in jumps_per_factor.iter_mut().enumerate().take(n_factors.min(row.len())) {
                    jumps.push(hawkes_factor::SymbolJump { symbol: r.symbol, mint: r.mint, loading: row[f], delta_zscore });
                }
            }
            // Phase B: only now overwrite `m_last_zscore` -- every jump
            // above must compare against the value from *before* this
            // cycle, not this cycle's own fresh zscore.
            for r in &out {
                self.state.m_last_zscore.insert(r.mint, r.zscore);
            }
            // Phase C: recurse each factor's own intensity exactly once
            // per cycle, and log the real result for live observation.
            for f in 0..n_factors {
                let magnitude = hawkes_factor::factor_jump_magnitude(&jumps_per_factor[f]);
                let updated = hawkes_factor::update_intensity(self.state.m_factor_intensity[f], magnitude);
                self.state.m_factor_intensity[f] = updated;
                self.state.m_factor_intensity_lambda_history[f].push(updated.lambda);
                let sigma_lambda = self.state.m_factor_intensity_lambda_history[f].stats().map(|s| s.stdev);
                log_warn!(
                    "multimodelv1: hawkes: factor {f} real lambda={:.6} mu={:.6} magnitude_this_cycle={:.6} \
                     sigma_lambda={sigma_lambda:?} real_jumps={}",
                    updated.lambda,
                    updated.mu,
                    magnitude,
                    jumps_per_factor[f].len(),
                );
            }
            // Fix (2026-09-07): `run_hawkes_trade_cycle`'s real open-pass
            // reads `self.state.m_hawkes_jumps_this_cycle` (see that
            // field's own doc comment), but this was the only place that
            // ever computed a real per-cycle jump list -- `jumps_per_
            // factor` was a local, discarded at the end of this function,
            // so `m_hawkes_jumps_this_cycle` stayed at its one-time
            // startup value (`on_load`'s `vec![Vec::new(); ...]`) for the
            // entire life of the process. Every real "cleared the entry
            // threshold but no real momentum basket buildable" refusal
            // was `factor_jump_basket`'s very first check
            // (`jumps.is_empty()`) hitting a permanently empty list, not
            // a genuine absence of real momentum signal. Persist the real
            // computation here so the open-pass actually sees it.
            self.state.m_hawkes_jumps_this_cycle = jumps_per_factor;
        }

        out
    }

    /// Real, per-symbol expected holding period for the borrow gate --
    /// `factor_residual::RollingWindow::estimated_half_life_cycles` (a
    /// real AR(1) fit against `mint`'s own real residual history, no
    /// fabricated fallback) converted to years via this mode's own real
    /// resync cadence, then padded by
    /// `factor_borrow_gate::HALF_LIFE_SAFETY_MULTIPLIER`. `None` if
    /// there isn't enough real history yet to fit one -- callers must
    /// refuse to gate-clear on `None`, not substitute a guessed number
    /// (same "unknown must never mean free" rule
    /// `factor_borrow_gate::decide_short_leg` already enforces for a
    /// zero/negative holding period). Used by trade type 1 (directional);
    /// trade type 2 (pair) uses [`Self::real_expected_pair_holding_period_years`]
    /// instead -- see that method's own doc comment for why.
    fn real_expected_holding_period_years(&self, mint: AccountId) -> Option<f64> {
        let cycles = self.state.m_residual_history.get(&mint)?.estimated_half_life_cycles()?;
        let raw_years = factor_residual::half_life_cycles_to_years(cycles, factor_graph::MAX_FACTOR_STALENESS_SECS);
        Some(factor_borrow_gate::expected_holding_period_years(raw_years))
    }

    /// Same real AR(1)-half-life-to-years conversion as
    /// [`Self::real_expected_holding_period_years`], but fit against
    /// `mint`'s own "spike vs. basket" spread series
    /// (`m_pair_spread_history`) rather than its raw residual series
    /// (`m_residual_history`) -- trade type 2's redesigned borrow-gate
    /// input. Real, live-confirmed reason this redesign exists: fitting
    /// AR(1) on a single symbol's raw residual (the old approach) came
    /// back negative phi on every one of 6+ real candidates observed live
    /// this session (classic microstructure-noise over-reaction/
    /// correction, not genuine mean-reversion) -- averaging a basket
    /// cancels exactly that kind of idiosyncratic noise, so the *spread*
    /// against a low-noise basket has a real chance of showing genuine
    /// `0 < phi < 1` decay where a single leg alone didn't.
    fn real_expected_pair_holding_period_years(&self, mint: AccountId) -> Option<f64> {
        let cycles = self.state.m_pair_spread_history.get(&mint)?.estimated_half_life_cycles()?;
        let raw_years = factor_residual::half_life_cycles_to_years(cycles, factor_graph::MAX_FACTOR_STALENESS_SECS);
        Some(factor_borrow_gate::expected_holding_period_years(raw_years))
    }

    /// Real, current-cycle "spike vs. basket" spread reading for `mint`
    /// -- `(latest raw spread pct, its z-score against that same
    /// window's rolling stats)`. `m_pair_spread_history` is pushed into
    /// every real factor resync cycle regardless of trade state (see
    /// that field's own doc comment), so this always reflects the
    /// *current* cycle's real value once
    /// `update_residual_history_and_get_current` has already run this
    /// cycle (which `run_factor_resync` guarantees before calling this
    /// trade type's own cycle function). `None` if there's no real
    /// spread history for `mint` at all yet.
    fn current_pair_spread(&self, mint: AccountId) -> Option<(f64, Option<f64>)> {
        let window = self.state.m_pair_spread_history.get(&mint)?;
        let latest = window.samples().last()?;
        Some((latest, window.zscore(latest)))
    }

    /// Real Phase 5 point 2 decision + execution driver, run every real
    /// factor resync once `pair_trading_enabled`.
    ///
    /// **"Spike vs. basket" redesign (2026-09-05)**: replaces the
    /// original "two independent single symbols cross the threshold
    /// simultaneously" design (`factor_residual::find_best_pair`) --
    /// live-confirmed over an extended real session that requiring both
    /// an underperformer *and* an overperformer to clear
    /// `PAIR_TRADE_ENTRY_ZSCORE` at once made a real candidate rare, and
    /// every one that *did* appear failed the half-life gate anyway (see
    /// [`Self::real_expected_pair_holding_period_years`]'s doc comment).
    /// Now: any single symbol crossing the threshold
    /// (`pair_basket::find_spike_candidate`) is a real candidate on its
    /// own, hedged against a small, low-noise basket of other curated
    /// symbols (`pair_basket::select_pair_basket_members`) rather than a
    /// second single symbol. Direction: an `Underperformer` spike is
    /// long-the-spike/short-the-basket; an `Overperformer` spike is
    /// short-the-spike/long-the-basket.
    ///
    /// Close pass first -- every cycle, re-derives whatever's currently
    /// open from real on-chain state ([`current_open_pair`]), identifies
    /// which side is "the spike" (whichever side has exactly one real
    /// leg) vs. "the basket" (the other side, expected to have at least
    /// `pair_basket::MIN_PAIR_BASKET_SIZE`), force-closes as broken/
    /// partial if that shape doesn't hold, and otherwise re-checks both
    /// the spread's own residual state (reverted or blown through a
    /// stop) and every currently-shorted leg's real borrow gate -- then
    /// an open pass only if nothing is currently open. `residuals` is
    /// computed once per resync by the caller (`run_factor_resync`) and
    /// shared with trade type 1's own cycle -- this function must never
    /// call `update_residual_history_and_get_current` itself, or the
    /// shared rolling windows would get pushed into twice per resync.
    /// `_factors` isn't used by this trade type's own logic (kept for
    /// signature symmetry with `run_directional_trade_cycle`, which does
    /// need it for basket construction).
    fn run_pair_trade_cycle(
        &mut self,
        _factors: &factor_graph::StructuralFactors,
        residuals: &[factor_residual::SymbolResidual],
    ) {
        if let Some((longs, shorts)) = self.current_open_pair() {
            let (spike_side, basket_side, spike_is_long) =
                if longs.len() == 1 { (&longs, &shorts, true) } else { (&shorts, &longs, false) };
            if spike_side.len() != 1 || basket_side.len() < pair_basket::MIN_PAIR_BASKET_SIZE {
                log_warn!(
                    "multimodelv1: pair: broken/partial open ({} long leg(s), {} short leg(s)) -- force-closing whatever is open",
                    longs.len(), shorts.len(),
                );
                for (symbol, mint) in &longs {
                    self.close_pair_long_leg(symbol, *mint);
                }
                for (symbol, mint) in &shorts {
                    self.close_pair_short_leg(symbol, *mint);
                }
                return;
            }
            let (spike_symbol, spike_mint) = spike_side[0];

            let spread = self.current_pair_spread(spike_mint);
            let residual_says_close = match spread {
                Some((_, Some(z))) => {
                    let reverted = z.abs() <= factor_residual::PAIR_TRADE_EXIT_ZSCORE;
                    let stopped_out = if spike_is_long {
                        z <= -factor_residual::PAIR_TRADE_STOP_ZSCORE
                    } else {
                        z >= factor_residual::PAIR_TRADE_STOP_ZSCORE
                    };
                    reverted || stopped_out
                }
                _ => false, // no real spread z-score yet this cycle -- don't close blind
            };

            let expected_edge_pct = spread.map(|(v, _)| v.abs());
            let holding_years = self.real_expected_pair_holding_period_years(spike_mint);
            let borrow_still_clears = match (expected_edge_pct, holding_years) {
                (Some(edge), Some(holding)) => shorts.iter().all(|(_, mint)| {
                    self.best_borrow_apy(*mint).is_some_and(|(_, apy)| factor_borrow_gate::decide_short_leg(edge, holding, apy))
                }),
                // No real holding-period estimate (or no real spread
                // data) this cycle -- fails closed: force a close rather
                // than assume the gate still clears on missing data.
                _ => false,
            };

            if residual_says_close || !borrow_still_clears {
                log_warn!(
                    "multimodelv1: pair: closing {spike_symbol} vs {}-leg basket -- residual_says_close={residual_says_close} borrow_still_clears={borrow_still_clears}",
                    basket_side.len(),
                );
                for (symbol, mint) in &longs {
                    self.close_pair_long_leg(symbol, *mint);
                }
                for (symbol, mint) in &shorts {
                    self.close_pair_short_leg(symbol, *mint);
                }
            }
            return;
        }

        let usdc_value = self.available_usdc_value();
        let usdc_floor = PAIR_CYCLE_MIN_NOTIONAL_USD * (1.0 + pair_basket::PAIR_BASKET_SIZE as f64);
        if usdc_value < usdc_floor {
            log_warn!(
                "multimodelv1: pair: skipping open pass -- wallet's real, currently-tracked USDC (${usdc_value:.2}) is below the ${usdc_floor:.2} floor needed for the spike leg plus every basket leg's collateral",
            );
            return;
        }
        let Some(candidate) = pair_basket::find_spike_candidate(residuals) else {
            // Real diagnostic, not just a silent skip -- without this,
            // "no candidate cleared the threshold" was indistinguishable
            // from every other silent early-return in this function (a
            // real gap noticed live-watching this cycle run for the
            // first time with real funds available and nothing
            // happening). Logs every real z-score this cycle actually
            // computed, not just that none cleared the bar.
            if residuals.is_empty() {
                log_warn!("multimodelv1: pair: no real z-scores yet this cycle (not enough residual history)");
            } else {
                let scores: Vec<String> = residuals.iter().map(|r| format!("{}={:.2}", r.symbol, r.zscore)).collect();
                log_warn!(
                    "multimodelv1: pair: no candidate cleared the {:.1}σ entry threshold this cycle -- {}",
                    factor_residual::PAIR_TRADE_ENTRY_ZSCORE,
                    scores.join(", "),
                );
            }
            return;
        };

        let basket_source: Vec<pair_basket::BasketMember> = curated_symbols()
            .filter_map(|(symbol, mint, _)| {
                let stdev = self.state.m_residual_history.get(&mint)?.stats()?.stdev;
                Some(pair_basket::BasketMember { symbol, mint, residual_stdev: stdev })
            })
            .collect();
        let Some(basket) = pair_basket::select_pair_basket_members(&basket_source, candidate.mint) else {
            log_warn!(
                "multimodelv1: pair: candidate {} has no real low-noise basket buildable this cycle (not enough warmed-up curated symbols) -- refusing to open",
                candidate.symbol,
            );
            return;
        };
        let Some(basket_avg) = pair_basket::basket_average_residual(residuals, &basket) else {
            log_warn!(
                "multimodelv1: pair: candidate {} -- none of this cycle's basket members have a real residual yet -- refusing to open",
                candidate.symbol,
            );
            return;
        };
        let spread_pct = candidate.residual_pct - basket_avg;

        let spike_is_long = candidate.side == factor_residual::ResidualSide::Underperformer;
        let short_mints: Vec<AccountId> =
            if spike_is_long { basket.iter().map(|m| m.mint).collect() } else { vec![candidate.mint] };

        let Some(holding_years) = self.real_expected_pair_holding_period_years(candidate.mint) else {
            // Temporary diagnostic (2026-09-05): mirrors RollingWindow::
            // estimated_half_life_cycles's own real AR(1) fit purely for
            // visibility -- that method is pure/no-log by design, so its
            // real phi (and *why* it's out of range) was otherwise
            // invisible across live cycles. Now fit against the spread
            // series, not a single leg's raw residual -- see this trade
            // type's own doc comment for why. Remove once this gate's
            // real live behavior is understood well enough not to need
            // it.
            if let Some(window) = self.state.m_pair_spread_history.get(&candidate.mint) {
                let samples: Vec<f64> = window.samples().collect();
                let n = samples.len();
                if n < factor_residual::MIN_SAMPLES_FOR_ZSCORE {
                    log_warn!(
                        "multimodelv1: pair: candidate {} vs basket half-life: only {n} real sample(s), need {}",
                        candidate.symbol, factor_residual::MIN_SAMPLES_FOR_ZSCORE,
                    );
                } else {
                    let mean = samples.iter().sum::<f64>() / n as f64;
                    let mut cov = 0.0;
                    let mut var = 0.0;
                    for i in 0..(n - 1) {
                        let a = samples[i] - mean;
                        let b = samples[i + 1] - mean;
                        cov += a * b;
                        var += a * a;
                    }
                    let phi = if var > 0.0 { cov / var } else { f64::NAN };
                    log_warn!(
                        "multimodelv1: pair: candidate {} vs basket half-life: real phi={phi:.6} (need 0 < phi < 1, n={n}, var={var:.6})",
                        candidate.symbol,
                    );
                }
            }
            log_warn!(
                "multimodelv1: pair: candidate {} vs basket found but no real half-life estimate yet -- refusing to open on an unknown holding period",
                candidate.symbol,
            );
            return;
        };

        let expected_edge_pct = spread_pct.abs();
        for &short_mint in &short_mints {
            let Some((_, apy)) = self.best_borrow_apy(short_mint) else {
                log_warn!("multimodelv1: pair: candidate {} vs basket -- a short leg has no real borrow APY yet", candidate.symbol);
                return;
            };
            if !factor_borrow_gate::decide_short_leg(expected_edge_pct, holding_years, apy) {
                log_warn!(
                    "multimodelv1: pair: candidate {} vs basket found but borrow gate refused (edge={expected_edge_pct:.2}% holding={holding_years:.4}yr apy={apy:.2}%)",
                    candidate.symbol,
                );
                return;
            }
        }

        // Real Phase 3 slippage-aware sizing: quote the spike leg plus
        // every basket leg together at their intended notionals and
        // scale the whole trade down together if any pool can't safely
        // absorb it -- never size legs independently (see
        // `factor_sizing::size_basket`'s own doc comment for why an
        // independently-sized "neutral" trade isn't actually neutral
        // once real slippage-adjusted fills land).
        let Some((sized_spike_usd, sized_basket)) = self.size_pair_legs(candidate.mint, spike_is_long, &basket) else {
            log_warn!("multimodelv1: pair: candidate {} vs basket found but no real priceable route yet", candidate.symbol);
            return;
        };
        let spike_floor = PAIR_CYCLE_MIN_NOTIONAL_USD * MIN_SIZED_FRACTION_OF_INTENDED;
        let leg_intended_usd = PAIR_CYCLE_MIN_NOTIONAL_USD / basket.len() as f64;
        let any_basket_leg_too_small =
            sized_basket.iter().any(|(_, _, sized_usd)| *sized_usd < leg_intended_usd * MIN_SIZED_FRACTION_OF_INTENDED);
        if sized_spike_usd < spike_floor || any_basket_leg_too_small {
            log_warn!(
                "multimodelv1: pair: candidate {} vs basket sized down too far by real slippage (spike=${sized_spike_usd:.2}) -- skipping this cycle",
                candidate.symbol,
            );
            return;
        }

        // Reserve this trade's real USDC commitment for the rest of this
        // resync cycle -- see `m_usdc_reserved_this_cycle`'s own doc
        // comment. Every gate has already passed at this point; nothing
        // past here can still refuse the open.
        self.state.m_usdc_reserved_this_cycle +=
            sized_spike_usd + sized_basket.iter().map(|(_, _, u)| u).sum::<f64>();

        log_warn!(
            "multimodelv1: pair: opening {} ({}, z={:.2}, ${sized_spike_usd:.2}) against a {}-leg low-noise basket",
            candidate.symbol, if spike_is_long { "long" } else { "short" }, candidate.zscore, sized_basket.len(),
        );
        if spike_is_long {
            self.open_pair_long_leg(candidate.symbol, candidate.mint, sized_spike_usd);
            for (symbol, mint, sized_usd) in &sized_basket {
                self.open_pair_short_leg(symbol, *mint, *sized_usd);
            }
        } else {
            self.open_pair_short_leg(candidate.symbol, candidate.mint, sized_spike_usd);
            for (symbol, mint, sized_usd) in &sized_basket {
                self.open_pair_long_leg(symbol, *mint, *sized_usd);
            }
        }
    }

    /// Real Phase 3 wiring for the "spike vs. basket" redesign: sizes the
    /// spike leg plus every real basket leg together against real,
    /// depth-aware liquidity (`factor_sizing::size_basket_exact` -- real
    /// tick-aware quotes for CLMM/Orca hops, not `cp_quote` on their full
    /// vault balance; live-confirmed necessary the same way dispersion's
    /// own basket sizing was, see `size_dispersion_long_legs`'s doc
    /// comment), generalizing the original fixed 2-element array to
    /// `1 + basket.len()` legs through the same *unmodified*
    /// `size_basket_exact` (already N-leg-generic, see
    /// `size_directional_legs`'s own doc comment for why every leg is
    /// scaled together, never independently).
    ///
    /// `spike_is_long` picks the direction: `true` means the spike leg is
    /// a real USDC->mint spot buy and every basket leg is a real
    /// mint->USDC sell (the underperformer case -- long the spike, short
    /// the basket); `false` is the mirror image (the overperformer case
    /// -- short the spike, long the basket). Each basket leg's own
    /// intended notional is `PAIR_CYCLE_MIN_NOTIONAL_USD / basket.len()`
    /// (equal-weighted, unlike `size_directional_legs`'s factor-loading
    /// -weighted basket -- there's no per-leg conviction to weight by
    /// here, see `pair_basket`'s own doc comment). Real price/decimals
    /// for each leg come from whichever protocol `open_pair_long_leg`/
    /// `open_pair_short_leg` will actually dispatch to, not hardcoded to
    /// Kamino. `None` if the spike leg or *any* basket leg has no real
    /// priceable route yet -- never opens a partial basket.
    fn size_pair_legs(
        &self,
        spike_mint: AccountId,
        spike_is_long: bool,
        basket: &[pair_basket::BasketMember],
    ) -> Option<(f64, Vec<(&'static str, AccountId, f64)>)> {
        let dex = self.state.o_dex.as_ref()?;
        let mint_usdc = self.configuration.mint_usdc;
        const USDC_DECIMALS: i32 = 6;

        let (spike_protocol, _) =
            if spike_is_long { self.best_supply_apy(spike_mint)? } else { self.best_borrow_apy(spike_mint)? };
        let (spike_price, spike_decimals) = self.reserve_price_and_decimals(spike_mint, spike_protocol)?;
        if spike_price <= 0.0 {
            return None;
        }

        let mut basket_prices_decimals: Vec<(f64, u32)> = Vec::with_capacity(basket.len());
        for member in basket {
            let (protocol, _) =
                if spike_is_long { self.best_borrow_apy(member.mint)? } else { self.best_supply_apy(member.mint)? };
            let (price, decimals) = self.reserve_price_and_decimals(member.mint, protocol)?;
            if price <= 0.0 {
                return None;
            }
            basket_prices_decimals.push((price, decimals));
        }

        let mut legs = Vec::with_capacity(1 + basket.len());
        if spike_is_long {
            let intended_raw = (PAIR_CYCLE_MIN_NOTIONAL_USD * 10f64.powi(USDC_DECIMALS)).round() as u64;
            if intended_raw == 0 {
                return None;
            }
            legs.push(factor_sizing::LegNotional { from_mint: mint_usdc, to_mint: spike_mint, intended_notional: intended_raw });
        } else {
            let intended_raw = ((PAIR_CYCLE_MIN_NOTIONAL_USD / spike_price) * 10f64.powi(spike_decimals as i32)).round() as u64;
            if intended_raw == 0 {
                return None;
            }
            legs.push(factor_sizing::LegNotional { from_mint: spike_mint, to_mint: mint_usdc, intended_notional: intended_raw });
        }
        let leg_intended_usd = PAIR_CYCLE_MIN_NOTIONAL_USD / basket.len() as f64;
        for (member, &(price, decimals)) in basket.iter().zip(&basket_prices_decimals) {
            if spike_is_long {
                let intended_raw = ((leg_intended_usd / price) * 10f64.powi(decimals as i32)).round() as u64;
                if intended_raw == 0 {
                    return None;
                }
                legs.push(factor_sizing::LegNotional { from_mint: member.mint, to_mint: mint_usdc, intended_notional: intended_raw });
            } else {
                let intended_raw = (leg_intended_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
                if intended_raw == 0 {
                    return None;
                }
                legs.push(factor_sizing::LegNotional { from_mint: mint_usdc, to_mint: member.mint, intended_notional: intended_raw });
            }
        }

        let sized = factor_sizing::size_basket_exact(&self.state.router, dex, &legs, LOOP_MAX_HOPS);
        let sized_spike_usd = if spike_is_long {
            sized[0].sized_notional as f64 / 10f64.powi(USDC_DECIMALS)
        } else {
            (sized[0].sized_notional as f64 / 10f64.powi(spike_decimals as i32)) * spike_price
        };
        let mut sized_basket = Vec::with_capacity(basket.len());
        for (i, member) in basket.iter().enumerate() {
            let (price, decimals) = basket_prices_decimals[i];
            let sized_usd = if spike_is_long {
                (sized[i + 1].sized_notional as f64 / 10f64.powi(decimals as i32)) * price
            } else {
                sized[i + 1].sized_notional as f64 / 10f64.powi(USDC_DECIMALS)
            };
            sized_basket.push((member.symbol, member.mint, sized_usd));
        }
        Some((sized_spike_usd, sized_basket))
    }

    /// Position of `mint` in `curated_symbols()`'s stable iteration order
    /// -- the same order `build_live_factor_graph` used to build
    /// `factors.eigenvectors` (both call `curated_symbols()` fresh, a
    /// pure/deterministic build-time list with no randomness, so the two
    /// calls always agree). This is what a caller needs to look up that
    /// mint's real factor loadings, `factors.eigenvectors[idx]`. `None`
    /// if `mint` isn't a curated symbol at all.
    fn curated_index_of(&self, mint: AccountId) -> Option<usize> {
        curated_symbols().position(|(_, m, _)| m == mint)
    }

    /// Trade type 1's real hedge-basket construction for `target_mint` --
    /// resolves the target's index into `factors.eigenvectors`, builds
    /// the candidate proxy list from every *other* curated symbol
    /// (excluding the target itself and USDC, neither a real hedge
    /// instrument), and delegates the actual per-factor proxy
    /// selection/weighting to `factor_basket::build_directional_basket`
    /// (see that function's own doc comment for the real algorithm).
    /// `None` if the target isn't a curated symbol, or if
    /// `build_directional_basket` itself refuses (structurally isolated
    /// target, or no real candidate loading survives on any factor).
    fn build_directional_basket_for(
        &self,
        target_mint: AccountId,
        factors: &factor_graph::StructuralFactors,
    ) -> Option<Vec<factor_basket::DirectionalBasketLeg>> {
        let mint_usdc = self.configuration.mint_usdc;
        let target_index = self.curated_index_of(target_mint)?;
        // Fix (2026-09-07): every basket leg is a real short (borrowed
        // against, see the open-pass's own per-leg `best_borrow_apy`
        // check right after this function returns) -- excluding
        // candidates with no real tracked Kamino/Solend reserve *before*
        // `build_directional_basket`'s own max-|loading| selection means
        // that selection naturally falls back to the next-best real
        // proxy per factor, instead of picking an ideal-on-paper proxy
        // that then always dies on the borrow-APY check downstream
        // (live-observed: the same single illiquid, uncovered proxy
        // blocked every real ETH open attempt this session).
        let candidates: Vec<factor_basket::CandidateToken> = curated_symbols()
            .enumerate()
            .filter(|(_, (_, mint, _))| *mint != target_mint && *mint != mint_usdc)
            .filter(|(_, (_, mint, _))| self.best_borrow_apy(*mint).is_some())
            .map(|(token_index, (symbol, mint, _))| factor_basket::CandidateToken { symbol, mint, token_index })
            .collect();
        // Fix (2026-09-07, revised): `factors.real_eigenvectors()` skips
        // every real trivial column (see that method's own doc comment
        // -- this bot's real graph has many disconnected components, not
        // just the one trivial column a connected graph would have).
        // Without this, max-|loading| proxy selection below
        // systematically favored thinly-connected/illiquid candidates.
        let real_eigenvectors: Vec<Vec<f64>> = factors.real_eigenvectors();
        factor_basket::build_directional_basket(&real_eigenvectors, target_index, RESIDUAL_FACTOR_COUNT, &candidates)
    }

    /// Trade type 1's real Phase 3 sizing -- generalizes `size_pair_legs`
    /// from a fixed 2-element array/index-unpack to `1 + basket.len()`
    /// legs (the long plus every real hedge-basket short), through the
    /// same *unmodified* `factor_sizing::size_basket_exact` (already N-leg
    /// -generic, see that function's own doc comment for why every leg
    /// is scaled together, never independently). Each short leg's own
    /// intended notional is `DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD *
    /// weight_fraction`, so the basket's short legs sum to the same
    /// total intended notional as the long. Real price/decimals for each
    /// leg come from whichever protocol `open_directional_long_leg`/
    /// `open_directional_short_leg` will actually dispatch to (same
    /// discipline `size_pair_legs` already established), not hardcoded
    /// to Kamino. `None` if the long leg or *any* short leg has no real
    /// priceable route yet -- never opens a partial basket.
    fn size_directional_legs(
        &self,
        target_mint: AccountId,
        basket: &[factor_basket::DirectionalBasketLeg],
    ) -> Option<(f64, Vec<(&'static str, AccountId, f64)>)> {
        let dex = self.state.o_dex.as_ref()?;
        let mint_usdc = self.configuration.mint_usdc;
        const USDC_DECIMALS: i32 = 6;

        let (long_protocol, _) = self.best_supply_apy(target_mint)?;
        let (long_price, _long_decimals) = self.reserve_price_and_decimals(target_mint, long_protocol)?;
        if long_price <= 0.0 {
            return None;
        }
        let long_intended_usdc_raw = (DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if long_intended_usdc_raw == 0 {
            return None;
        }

        // Real per-leg price/decimals for every short leg, resolved
        // before sizing -- `None` for any leg means "no real route yet,"
        // refusing the whole basket rather than opening a partial hedge.
        let mut short_prices_decimals: Vec<(f64, u32)> = Vec::with_capacity(basket.len());
        for leg in basket {
            let (protocol, _) = self.best_borrow_apy(leg.mint)?;
            let (price, decimals) = self.reserve_price_and_decimals(leg.mint, protocol)?;
            if price <= 0.0 {
                return None;
            }
            short_prices_decimals.push((price, decimals));
        }

        let mut legs = Vec::with_capacity(1 + basket.len());
        legs.push(factor_sizing::LegNotional { from_mint: mint_usdc, to_mint: target_mint, intended_notional: long_intended_usdc_raw });
        for (leg, &(price, decimals)) in basket.iter().zip(&short_prices_decimals) {
            let leg_intended_usd = DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
            let intended_raw = ((leg_intended_usd / price) * 10f64.powi(decimals as i32)).round() as u64;
            if intended_raw == 0 {
                return None;
            }
            legs.push(factor_sizing::LegNotional { from_mint: leg.mint, to_mint: mint_usdc, intended_notional: intended_raw });
        }

        let sized = factor_sizing::size_basket_exact(&self.state.router, dex, &legs, LOOP_MAX_HOPS);
        let sized_long_usd = sized[0].sized_notional as f64 / 10f64.powi(USDC_DECIMALS);
        let mut sized_shorts = Vec::with_capacity(basket.len());
        for (i, leg) in basket.iter().enumerate() {
            let (price, decimals) = short_prices_decimals[i];
            let sized_short_usd = (sized[i + 1].sized_notional as f64 / 10f64.powi(decimals as i32)) * price;
            sized_shorts.push((leg.symbol, leg.mint, sized_short_usd));
        }
        Some((sized_long_usd, sized_shorts))
    }

    /// Trade type 5's own generalization of `size_directional_legs`/
    /// `size_pair_legs` -- an arbitrary set of long+short legs (from
    /// `hawkes_factor::factor_jump_basket`, no fixed "spike" or "target"
    /// leg), each already carrying its own `weight_fraction` (summing to
    /// 1.0 across the *whole* basket, both sides together, unlike
    /// `size_directional_legs`'s short-only weights) and `MomentumSide`.
    /// Same *unmodified* `factor_sizing::size_basket_exact` every other
    /// trade type's own sizing uses. Real price/decimals for each *short*
    /// leg are resolved once and reused for both input sizing and output
    /// conversion (same discipline `size_pair_legs`/`size_directional_
    /// legs` already establish); a long leg needs no such lookup (already
    /// priced directly in USDC units, same as those two). `None` if any
    /// short leg has no real priceable route yet -- never opens a
    /// partial basket.
    fn size_hawkes_legs(
        &self,
        legs: &[hawkes_factor::FactorMomentumLeg],
    ) -> Option<Vec<(&'static str, AccountId, hawkes_factor::MomentumSide, f64)>> {
        let dex = self.state.o_dex.as_ref()?;
        let mint_usdc = self.configuration.mint_usdc;
        const USDC_DECIMALS: i32 = 6;

        // `Some` only for a Short leg (a Long leg needs no price lookup,
        // see this function's own doc comment) -- `None` for a Long leg
        // is "not applicable," never read back below.
        let mut short_prices_decimals: Vec<Option<(f64, u32)>> = Vec::with_capacity(legs.len());
        for leg in legs {
            match leg.side {
                hawkes_factor::MomentumSide::Long => short_prices_decimals.push(None),
                hawkes_factor::MomentumSide::Short => {
                    let (protocol, _) = self.best_borrow_apy(leg.mint)?;
                    let (price, decimals) = self.reserve_price_and_decimals(leg.mint, protocol)?;
                    if price <= 0.0 {
                        return None;
                    }
                    short_prices_decimals.push(Some((price, decimals)));
                }
            }
        }

        let mut sizing_legs = Vec::with_capacity(legs.len());
        for (leg, price_decimals) in legs.iter().zip(&short_prices_decimals) {
            let intended_usd = HAWKES_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
            match leg.side {
                hawkes_factor::MomentumSide::Long => {
                    let intended_raw = (intended_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
                    if intended_raw == 0 {
                        return None;
                    }
                    sizing_legs.push(factor_sizing::LegNotional {
                        from_mint: mint_usdc,
                        to_mint: leg.mint,
                        intended_notional: intended_raw,
                    });
                }
                hawkes_factor::MomentumSide::Short => {
                    let (price, decimals) = price_decimals.expect("short leg always has a resolved price");
                    let intended_raw = ((intended_usd / price) * 10f64.powi(decimals as i32)).round() as u64;
                    if intended_raw == 0 {
                        return None;
                    }
                    sizing_legs.push(factor_sizing::LegNotional {
                        from_mint: leg.mint,
                        to_mint: mint_usdc,
                        intended_notional: intended_raw,
                    });
                }
            }
        }

        let sized = factor_sizing::size_basket_exact(&self.state.router, dex, &sizing_legs, LOOP_MAX_HOPS);
        let mut out = Vec::with_capacity(legs.len());
        for (i, leg) in legs.iter().enumerate() {
            let sized_usd = match leg.side {
                hawkes_factor::MomentumSide::Long => sized[i].sized_notional as f64 / 10f64.powi(USDC_DECIMALS),
                hawkes_factor::MomentumSide::Short => {
                    let (price, decimals) = short_prices_decimals[i].expect("short leg always has a resolved price");
                    (sized[i].sized_notional as f64 / 10f64.powi(decimals as i32)) * price
                }
            };
            out.push((leg.symbol, leg.mint, leg.side, sized_usd));
        }
        Some(out)
    }

    /// Trade type 1's real Phase 5 decision + execution driver, run every
    /// real factor resync once `directional_trading_enabled`, sharing
    /// this cycle's `residuals` with `run_pair_trade_cycle` (both are
    /// computed from the exact same `update_residual_history_and_get_
    /// current` call in `run_factor_resync` -- never called twice per
    /// resync). Close pass first, unconditional every cycle: re-derives
    /// whatever's currently open from real on-chain state
    /// ([`current_open_directional`]) and force-closes on any of --  an
    /// empty-shorts broken basket, the real stop-loss
    /// (`factor_basket::should_stop_directional`, fails **open** on
    /// missing residual data, matching `run_pair_trade_cycle`'s own
    /// asymmetric close-pass precedent -- a stop-loss needs real evidence
    /// to fire, missing data alone never force-closes), any short leg's
    /// real borrow gate failing (fails **closed** -- missing borrow-rate
    /// data *does* force-close, same as the pair trade), or an explicit
    /// human close request. Open pass only if nothing is currently open
    /// and a human target is pending.
    fn run_directional_trade_cycle(&mut self, factors: &factor_graph::StructuralFactors, residuals: &[factor_residual::SymbolResidual]) {
        if let Some((long, shorts)) = self.current_open_directional() {
            if shorts.is_empty() {
                // Fix (2026-09-07): a basket open now takes more than one
                // cycle to complete -- see `m_directional_solend_leg_
                // action_this_cycle`'s own doc comment: only one real
                // deposit/borrow against a shared obligation lands per
                // cycle, so "long open, no shorts yet" is the *normal*
                // transient state for at least one full cycle right after
                // the long leg lands, not necessarily broken. Live-
                // confirmed this used to force-close (and re-pay real
                // spot-swap/slippage costs to reopen) every single cycle
                // before the short legs ever got a turn, an infinite
                // open/close loop. Try to continue the same real basket
                // this long leg was opened against before giving up --
                // only fall back to force-closing if it genuinely can't be
                // completed right now (no real basket buildable, missing
                // borrow-rate data, or the real borrow gate refuses --
                // same "never partial, fail closed" discipline every other
                // gate in this file already uses).
                let Some(basket) = self.build_directional_basket_for(long.1, factors) else {
                    log_warn!(
                        "multimodelv1: directional: {} has a real long deposit but no real short legs, and no real hedge basket buildable this cycle -- broken/partial basket, force-closing",
                        long.0,
                    );
                    self.close_directional_long_leg(long.0, long.1);
                    return;
                };
                let mut leg_apys: Vec<(&factor_basket::DirectionalBasketLeg, f64)> = Vec::with_capacity(basket.len());
                for leg in &basket {
                    let Some((_, apy)) = self.best_borrow_apy(leg.mint) else {
                        log_warn!(
                            "multimodelv1: directional: {} has a real long deposit but no real short legs, and basket leg {} has no real borrow APY yet -- waiting for next cycle",
                            long.0, leg.symbol,
                        );
                        return;
                    };
                    leg_apys.push((leg, apy));
                }
                let blended_short_borrow_apy: f64 = leg_apys.iter().map(|(leg, apy)| leg.weight_fraction * apy).sum();
                let basket_is_net_profitable = self.best_supply_apy(long.1).is_some_and(|(_, long_apy)| {
                    factor_borrow_gate::directional_basket_is_net_profitable(long_apy, blended_short_borrow_apy)
                });
                for (leg, apy) in &leg_apys {
                    if !factor_borrow_gate::decide_directional_short_leg_with_carry(*apy, basket_is_net_profitable) {
                        log_warn!(
                            "multimodelv1: directional: {} has a real long deposit but no real short legs, and basket leg {} borrow gate refused -- broken/partial basket, force-closing",
                            long.0, leg.symbol,
                        );
                        self.close_directional_long_leg(long.0, long.1);
                        return;
                    }
                }
                let Some((_, sized_shorts)) = self.size_directional_legs(long.1, &basket) else {
                    log_warn!(
                        "multimodelv1: directional: {} has a real long deposit but no real short legs, and the basket has no real priceable route yet -- waiting for next cycle",
                        long.0,
                    );
                    return;
                };
                let any_short_too_small = basket.iter().zip(&sized_shorts).any(|(leg, (_, _, sized_usd))| {
                    let intended_usd = DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
                    *sized_usd < intended_usd * MIN_SIZED_FRACTION_OF_INTENDED
                });
                if any_short_too_small {
                    log_warn!(
                        "multimodelv1: directional: {} has a real long deposit but no real short legs, and the basket sized down too far by real slippage -- waiting for next cycle",
                        long.0,
                    );
                    return;
                }
                self.state.m_usdc_reserved_this_cycle += sized_shorts.iter().map(|(_, _, u)| u).sum::<f64>();
                log_warn!(
                    "multimodelv1: directional: {} has a real long deposit but no real short legs yet -- continuing to open the {}-leg hedge basket",
                    long.0, sized_shorts.len(),
                );
                for (symbol, mint, sized_usd) in &sized_shorts {
                    self.open_directional_short_leg(symbol, *mint, *sized_usd);
                }
                return;
            }
            let long_r = residuals.iter().find(|r| r.mint == long.1);
            let residual_says_stop = long_r.is_some_and(|r| factor_basket::should_stop_directional(r.zscore));
            // Real net-carry-aware re-check (2026-09-06, user-directed):
            // same relaxed cap the open-pass below applies, re-derived
            // every cycle from real, current reserve data -- an
            // already-open basket that was profitable to open can stop
            // being profitable (rates move), and must be re-judged against
            // the *current* real carry, not the one at open time. Equal
            // weighting across `shorts` (real on-chain legs, no persisted
            // weight_fraction to re-derive) -- see this close-pass's own
            // reasoning for why that's the honest choice here.
            let short_apys: Vec<f64> = shorts.iter().filter_map(|(_, mint)| self.best_borrow_apy(*mint).map(|(_, apy)| apy)).collect();
            let borrow_still_clears_all = if short_apys.len() == shorts.len() {
                let blended_short_borrow_apy = short_apys.iter().sum::<f64>() / short_apys.len() as f64;
                let basket_is_net_profitable = self
                    .best_supply_apy(long.1)
                    .is_some_and(|(_, long_apy)| factor_borrow_gate::directional_basket_is_net_profitable(long_apy, blended_short_borrow_apy));
                short_apys.iter().all(|apy| factor_borrow_gate::decide_directional_short_leg_with_carry(*apy, basket_is_net_profitable))
            } else {
                // Missing real borrow-rate data on at least one leg --
                // fails closed, same discipline as every other missing-
                // data case in this file.
                false
            };
            let human_close_requested = self.state.o_directional_close_requested;
            if residual_says_stop || !borrow_still_clears_all || human_close_requested {
                log_warn!(
                    "multimodelv1: directional: closing {} + {} short leg(s) -- residual_says_stop={residual_says_stop} borrow_still_clears_all={borrow_still_clears_all} human_close_requested={human_close_requested}",
                    long.0, shorts.len(),
                );
                for (symbol, mint) in &shorts {
                    self.close_directional_short_leg(symbol, *mint);
                }
                self.close_directional_long_leg(long.0, long.1);
                self.state.o_directional_target_mint = None;
                self.state.o_directional_close_requested = false;
            }
            return;
        }
        // Directional counterpart to hawkes's own orphaned-USDC-collateral
        // gate above -- same reasoning, same "human close request only"
        // restriction (see `close_hawkes_orphaned_usdc_collateral_kamino`'s
        // doc comment for why this can't run unconditionally).
        if self.state.o_directional_close_requested {
            self.close_directional_orphaned_usdc_collateral_kamino();
            self.close_directional_orphaned_usdc_collateral_solend();
        }
        self.state.o_directional_close_requested = false;

        let Some(target_mint) = self.state.o_directional_target_mint else { return };
        let target_symbol = curated_symbols().find(|(_, m, _)| *m == target_mint).map(|(s, _, _)| s);
        let Some(target_symbol) = target_symbol else {
            log_error!("multimodelv1: directional: target mint is not a curated symbol -- refusing to open");
            self.state.o_directional_target_mint = None;
            return;
        };

        let usdc_value = self.available_usdc_value();
        let usdc_floor = DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * (1.0 + 2.0 * RESIDUAL_FACTOR_COUNT as f64);
        if usdc_value < usdc_floor {
            log_warn!(
                "multimodelv1: directional: skipping open pass -- wallet's real, currently-tracked USDC (${usdc_value:.2}) is below the ${usdc_floor:.2} floor needed for the long leg plus every basket short's collateral",
            );
            return;
        }

        // Temporary diagnostic (2026-09-05): mirrors build_directional_
        // basket's own total_loading computation purely for visibility --
        // that function is pure/no-log by design (factor_basket.rs's own
        // no-host-import discipline), so its real pass/fail number was
        // otherwise invisible across live cycles. Remove once trade type
        // 1's real live behavior is understood well enough not to need it.
        if let Some(target_index) = self.curated_index_of(target_mint) {
            // Fix (2026-09-07, revised): `factors.real_eigenvectors()`
            // skips every real trivial column, matching what
            // `build_directional_basket_for` (via its own identical fix)
            // actually gates on -- see that method's own doc comment.
            let real_eigenvectors = factors.real_eigenvectors();
            if let Some(real_row) = real_eigenvectors.get(target_index) {
                let n_factors = real_row.len().min(RESIDUAL_FACTOR_COUNT);
                let total_loading: f64 = real_row[0..n_factors].iter().map(|b| b.abs()).sum();
                log_warn!(
                    "multimodelv1: directional: {target_symbol} real total_loading={total_loading:.6} (need >= {:.6}, n_factors={n_factors}, eigenvectors.len()={})",
                    factor_basket::MIN_TOTAL_FACTOR_LOADING,
                    factors.eigenvectors.len(),
                );
            } else {
                log_warn!(
                    "multimodelv1: directional: {target_symbol} target_index={target_index} out of range for eigenvectors.len()={}",
                    factors.eigenvectors.len(),
                );
            }
        }

        // Temporary diagnostic (2026-09-06): mirrors build_live_factor_
        // graph's own price_impact_bps probe, restricted to target_mint
        // against every other curated symbol -- that function is
        // pure/no-log by design, so whether target_mint is genuinely
        // isolated (no real route to any curated symbol at all) or just
        // hasn't had its pools' live account updates arrive yet (the same
        // "static router snapshot has real edges, live router hasn't
        // populated them yet" gap already found for router coverage
        // earlier this session) was otherwise invisible -- only the
        // downstream total_loading=0.000000 symptom showed, not the real
        // cause. Remove once this trade type's real live connectivity is
        // understood well enough not to need it.
        {
            const FACTOR_GRAPH_REFERENCE_WHOLE_UNITS: u64 = 100;
            const FACTOR_GRAPH_MAX_HOPS: usize = 3;
            if let Some((_, _, target_decimals)) = curated_symbols().find(|(_, m, _)| *m == target_mint) {
                let whole_unit = 10u64.saturating_pow(target_decimals as u32);
                let amount_in = whole_unit.saturating_mul(FACTOR_GRAPH_REFERENCE_WHOLE_UNITS);
                let mut real_edges = 0usize;
                let mut tested = 0usize;
                let mut examples: Vec<String> = Vec::new();
                for (symbol, mint, _) in curated_symbols() {
                    if mint == target_mint {
                        continue;
                    }
                    tested += 1;
                    let got = factor_sizing::price_impact_bps(
                        &self.state.router,
                        target_mint,
                        mint,
                        amount_in,
                        whole_unit,
                        FACTOR_GRAPH_MAX_HOPS,
                    );
                    if let Some(impact_bps) = got {
                        real_edges += 1;
                        if examples.len() < 3 {
                            examples.push(format!("{symbol}={impact_bps:.2}bps"));
                        }
                    }
                }
                log_warn!(
                    "multimodelv1: directional: {target_symbol} real live-router edges: {real_edges}/{tested} curated pairs routable this cycle (reference={FACTOR_GRAPH_REFERENCE_WHOLE_UNITS} whole units, max_hops={FACTOR_GRAPH_MAX_HOPS}) -- examples: {}",
                    if examples.is_empty() { "none".to_string() } else { examples.join(", ") },
                );
            }
        }

        // Temporary diagnostic (2026-09-07): no pair/directional/Hawkes
        // short leg has ever successfully opened in this bot's history
        // (grepped every persisted log) -- every basket that gets this
        // far dies on "no real borrow APY yet" for at least one leg. This
        // checks `best_borrow_apy` against the *entire* curated universe
        // once per cycle to see whether the Kamino/Solend reserve data is
        // populated at all, or whether that's the real, structural
        // bottleneck regardless of the loading-gate loosening above.
        // Remove once that question is answered.
        {
            let mut have_apy = 0usize;
            let mut tested = 0usize;
            let mut examples: Vec<String> = Vec::new();
            for (symbol, mint, _) in curated_symbols() {
                tested += 1;
                if let Some((protocol, apy)) = self.best_borrow_apy(mint) {
                    have_apy += 1;
                    if examples.len() < 5 {
                        examples.push(format!("{symbol}={protocol:?}:{apy:.2}%"));
                    }
                }
            }
            log_warn!(
                "multimodelv1: directional: real borrow-APY coverage this cycle: {have_apy}/{tested} curated mints have a tracked Kamino/Solend reserve -- examples: {}",
                if examples.is_empty() { "none".to_string() } else { examples.join(", ") },
            );
        }

        let Some(basket) = self.build_directional_basket_for(target_mint, factors) else {
            log_warn!(
                "multimodelv1: directional: {target_symbol} has no real hedge basket buildable this cycle (structurally isolated, or no real factor data yet) -- refusing to open"
            );
            return;
        };

        // Real borrow gate on *every* short leg before sizing -- if any
        // leg fails, refuse the whole basket rather than opening a
        // partial hedge (same "never partial" discipline the close-pass
        // above and `size_directional_legs` below both already follow).
        //
        // Real net-carry-aware cap (2026-09-06, user-directed): resolve
        // every leg's real borrow APY first (never partial), then compute
        // the basket's real net carry -- the target's own real Kamino/
        // Solend supply APY while deposited, minus the basket's real
        // weight_fraction-blended borrow APY -- before applying the gate,
        // so every leg is judged against the same real, whole-basket
        // number, not a per-leg guess. See `factor_borrow_gate::
        // directional_basket_is_net_profitable`'s own doc comment for why
        // this is real, already-known reserve data, not a fabricated
        // edge.
        let mut leg_apys: Vec<(&factor_basket::DirectionalBasketLeg, f64)> = Vec::with_capacity(basket.len());
        for leg in &basket {
            let Some((_, apy)) = self.best_borrow_apy(leg.mint) else {
                log_warn!(
                    "multimodelv1: directional: {target_symbol} basket leg {} has no real borrow APY yet -- refusing to open",
                    leg.symbol,
                );
                return;
            };
            leg_apys.push((leg, apy));
        }
        let blended_short_borrow_apy: f64 = leg_apys.iter().map(|(leg, apy)| leg.weight_fraction * apy).sum();
        let basket_is_net_profitable = self
            .best_supply_apy(target_mint)
            .is_some_and(|(_, long_apy)| factor_borrow_gate::directional_basket_is_net_profitable(long_apy, blended_short_borrow_apy));
        for (leg, apy) in &leg_apys {
            if !factor_borrow_gate::decide_directional_short_leg_with_carry(*apy, basket_is_net_profitable) {
                log_warn!(
                    "multimodelv1: directional: {target_symbol} basket leg {} borrow gate refused (apy={apy:.2}% cap={:.2}% basket_net_profitable={basket_is_net_profitable} blended_short_borrow_apy={blended_short_borrow_apy:.2}%)",
                    leg.symbol,
                    if basket_is_net_profitable {
                        factor_borrow_gate::MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT_WHEN_PROFITABLE
                    } else {
                        factor_borrow_gate::MAX_DIRECTIONAL_SHORT_BORROW_APY_PCT
                    },
                );
                return;
            }
        }

        let Some((sized_long_usd, sized_shorts)) = self.size_directional_legs(target_mint, &basket) else {
            log_warn!("multimodelv1: directional: {target_symbol} found but the basket has no real priceable route yet");
            return;
        };
        // Each leg's own floor is `MIN_SIZED_FRACTION_OF_INTENDED` of
        // *that leg's own* intended notional -- the long's is the full
        // `DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD`, each short's is that
        // times its own `weight_fraction` (see `size_directional_legs`'s
        // own doc comment), never a single shared floor across legs of
        // very different intended sizes.
        let long_floor = DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * MIN_SIZED_FRACTION_OF_INTENDED;
        let any_short_too_small = basket.iter().zip(&sized_shorts).any(|(leg, (_, _, sized_usd))| {
            let intended_usd = DIRECTIONAL_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
            *sized_usd < intended_usd * MIN_SIZED_FRACTION_OF_INTENDED
        });
        if sized_long_usd < long_floor || any_short_too_small {
            log_warn!(
                "multimodelv1: directional: {target_symbol} sized down too far by real slippage (long=${sized_long_usd:.2}) -- skipping this cycle",
            );
            return;
        }

        // Reserve this trade's real USDC commitment for the rest of this
        // resync cycle -- see `m_usdc_reserved_this_cycle`'s own doc
        // comment. Every gate has already passed at this point; nothing
        // past here can still refuse the open.
        self.state.m_usdc_reserved_this_cycle += sized_long_usd + sized_shorts.iter().map(|(_, _, u)| u).sum::<f64>();

        log_warn!(
            "multimodelv1: directional: opening {target_symbol} (long, ${sized_long_usd:.2}) against a {}-leg hedge basket",
            sized_shorts.len(),
        );
        self.open_directional_long_leg(target_symbol, target_mint, sized_long_usd);
        for (symbol, mint, sized_usd) in &sized_shorts {
            self.open_directional_short_leg(symbol, *mint, *sized_usd);
        }
    }

    /// Trade type 5's (Hawkes-on-eigenfactor momentum) real decision +
    /// execution driver, run every real factor resync once
    /// `hawkes_trading_enabled`. See `docs/HAWKES_FACTOR_TRADE_PLAN.md`
    /// for the full design this implements.
    ///
    /// Close pass first, unconditional every cycle: re-derives whatever's
    /// currently open from real on-chain state ([`current_open_hawkes`]).
    /// Unlike the pair trade's fixed spike-vs-basket shape, a Hawkes
    /// basket has no fixed long/short split -- an all-long or all-short
    /// open is a real, valid outcome, not broken. What *is* unrecoverable
    /// is losing track of which factor justified the open
    /// (`o_hawkes_open_factor`) across a restart -- that has no on-chain
    /// trace at all, so real legs open with an unknown factor force-close
    /// immediately rather than guess. Otherwise checks three real
    /// conditions -- intensity decay (`hawkes_factor::
    /// should_close_hawkes_on_decay`), the hard max-holding-cycles cap
    /// (`HAWKES_MAX_HOLDING_CYCLES`), and a net-carry-aware borrow-gate
    /// re-check on every open short leg (mirrors directional's own
    /// close-pass re-check exactly, equal-weighting across real on-chain
    /// legs since there's no persisted `weight_fraction` to re-derive) --
    /// closing the whole basket if any fires. Open pass only if nothing
    /// is currently open.
    fn run_hawkes_trade_cycle(&mut self, _factors: &factor_graph::StructuralFactors, _residuals: &[factor_residual::SymbolResidual]) {
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;

        if let Some((longs, shorts)) = self.current_open_hawkes() {
            let Some(factor_idx) = self.state.o_hawkes_open_factor else {
                log_warn!(
                    "multimodelv1: hawkes: {} long leg(s) + {} short leg(s) open but no real factor context survived (restart?) -- force-closing",
                    longs.len(), shorts.len(),
                );
                for (symbol, mint) in &longs {
                    self.close_hawkes_long_leg(symbol, *mint);
                }
                for (symbol, mint) in &shorts {
                    self.close_hawkes_short_leg(symbol, *mint);
                }
                self.state.m_hawkes_opened_at_secs = None;
                self.state.o_hawkes_close_requested = false;
                return;
            };

            let intensity = self.state.m_factor_intensity[factor_idx];
            let sigma_lambda = self.state.m_factor_intensity_lambda_history[factor_idx].stats().map(|s| s.stdev);
            let decayed = match sigma_lambda {
                Some(sigma_lambda) => hawkes_factor::should_close_hawkes_on_decay(intensity.lambda, intensity.mu, sigma_lambda),
                None => true, // no real sigma yet -- fail closed, same discipline as every other missing-data case
            };

            let max_holding_secs = HAWKES_MAX_HOLDING_CYCLES * factor_graph::MAX_FACTOR_STALENESS_SECS;
            let holding_expired = match self.state.m_hawkes_opened_at_secs {
                Some(opened_at) => now_secs.saturating_sub(opened_at) > max_holding_secs,
                None => true, // lost track of open time -- fail closed
            };

            // Real net-carry-aware re-check, same shape as directional's
            // own close-pass re-check (equal weighting across real
            // on-chain legs -- no persisted `weight_fraction` to
            // re-derive here either). Moot (trivially true) if `shorts`
            // is empty -- an all-long basket has nothing to gate.
            let borrow_still_clears_all = if shorts.is_empty() {
                true
            } else {
                let short_apys: Vec<f64> = shorts.iter().filter_map(|(_, mint)| self.best_borrow_apy(*mint).map(|(_, apy)| apy)).collect();
                if short_apys.len() != shorts.len() {
                    false // missing real borrow-rate data on at least one leg -- fail closed
                } else {
                    let blended_short_borrow_apy = short_apys.iter().sum::<f64>() / short_apys.len() as f64;
                    let long_apys: Vec<f64> = longs.iter().filter_map(|(_, mint)| self.best_supply_apy(*mint).map(|(_, apy)| apy)).collect();
                    let basket_is_net_profitable = !longs.is_empty()
                        && long_apys.len() == longs.len()
                        && factor_borrow_gate::directional_basket_is_net_profitable(
                            long_apys.iter().sum::<f64>() / long_apys.len() as f64,
                            blended_short_borrow_apy,
                        );
                    short_apys.iter().all(|apy| factor_borrow_gate::decide_directional_short_leg_with_carry(*apy, basket_is_net_profitable))
                }
            };

            let human_close_requested = self.state.o_hawkes_close_requested;
            if decayed || holding_expired || !borrow_still_clears_all || human_close_requested {
                log_warn!(
                    "multimodelv1: hawkes: closing factor {factor_idx}'s basket ({} long, {} short) -- decayed={decayed} holding_expired={holding_expired} borrow_still_clears_all={borrow_still_clears_all} human_close_requested={human_close_requested}",
                    longs.len(), shorts.len(),
                );
                for (symbol, mint) in &longs {
                    self.close_hawkes_long_leg(symbol, *mint);
                }
                for (symbol, mint) in &shorts {
                    self.close_hawkes_short_leg(symbol, *mint);
                }
                self.state.o_hawkes_open_factor = None;
                self.state.m_hawkes_opened_at_secs = None;
                self.state.o_hawkes_close_requested = false;
            }
            return;
        }
        // Real, live-confirmed gap this covers: `current_open_hawkes`
        // above only sees altcoin-denominated legs, so a stranded USDC
        // deposit (no debt against it -- see these functions' own doc
        // comments) never reaches the close branch above. Gated strictly
        // behind an explicit human close request -- NOT run on every
        // ordinary cycle -- because `open_hawkes_short_leg_{kamino,
        // solend}` deposit USDC collateral and return, only borrowing
        // against it on a *later* cycle once the deposit is observed
        // (see those functions' own bodies): calling this unconditionally
        // would race a real, in-progress open and withdraw the collateral
        // before the borrow ever happens.
        if self.state.o_hawkes_close_requested {
            self.close_hawkes_orphaned_usdc_collateral_kamino();
            self.close_hawkes_orphaned_usdc_collateral_solend();
        }
        self.state.o_hawkes_close_requested = false;

        let usdc_value = self.available_usdc_value();
        // `* 2.0`: real headroom for short-leg collateral overhead (a
        // real LTV < 1.0 means posting collateral costs more real USDC
        // than the notional actually borrowed) -- `HAWKES_CYCLE_MIN_
        // NOTIONAL_USD` itself is already the whole basket's real total,
        // unlike `PAIR_CYCLE_MIN_NOTIONAL_USD`/`DIRECTIONAL_CYCLE_MIN_
        // NOTIONAL_USD` (both per-leg there).
        let usdc_floor = HAWKES_CYCLE_MIN_NOTIONAL_USD * 2.0;
        if usdc_value < usdc_floor {
            log_warn!(
                "multimodelv1: hawkes: skipping open pass -- wallet's real, currently-tracked USDC (${usdc_value:.2}) is below the ${usdc_floor:.2} floor",
            );
            return;
        }

        let mut best_factor: Option<(usize, f64)> = None;
        for f in 0..self.state.m_factor_intensity.len() {
            let intensity = self.state.m_factor_intensity[f];
            let Some(sigma_lambda) = self.state.m_factor_intensity_lambda_history[f].stats().map(|s| s.stdev) else {
                continue;
            };
            if !hawkes_factor::should_open_hawkes(intensity.lambda, intensity.mu, sigma_lambda) {
                continue;
            }
            let margin = if sigma_lambda > 0.0 { (intensity.lambda - intensity.mu) / sigma_lambda } else { f64::INFINITY };
            match best_factor {
                Some((_, best_margin)) if margin <= best_margin => {}
                _ => best_factor = Some((f, margin)),
            }
        }
        let Some((factor_idx, _)) = best_factor else {
            return;
        };

        let jumps = &self.state.m_hawkes_jumps_this_cycle[factor_idx];
        let Some(basket) = hawkes_factor::factor_jump_basket(jumps, hawkes_factor::HAWKES_BASKET_SIZE) else {
            log_warn!("multimodelv1: hawkes: factor {factor_idx} cleared the entry threshold but no real momentum basket buildable this cycle -- refusing to open");
            return;
        };

        let longs: Vec<&hawkes_factor::FactorMomentumLeg> =
            basket.iter().filter(|l| l.side == hawkes_factor::MomentumSide::Long).collect();
        let shorts: Vec<&hawkes_factor::FactorMomentumLeg> =
            basket.iter().filter(|l| l.side == hawkes_factor::MomentumSide::Short).collect();

        // Real borrow gate on every short leg before sizing -- same
        // "never partial" discipline directional's own open-pass follows.
        // Moot if `shorts` is empty (nothing to gate, basket is all-long).
        if !shorts.is_empty() {
            let mut short_apys: Vec<f64> = Vec::with_capacity(shorts.len());
            for leg in &shorts {
                let Some((_, apy)) = self.best_borrow_apy(leg.mint) else {
                    log_warn!("multimodelv1: hawkes: factor {factor_idx} basket leg {} has no real borrow APY yet -- refusing to open", leg.symbol);
                    return;
                };
                short_apys.push(apy);
            }
            let short_weight: f64 = shorts.iter().map(|l| l.weight_fraction).sum();
            let blended_short_borrow_apy =
                shorts.iter().zip(&short_apys).map(|(l, apy)| l.weight_fraction * apy).sum::<f64>() / short_weight;
            let basket_is_net_profitable = if longs.is_empty() {
                false
            } else {
                let mut long_apys: Vec<f64> = Vec::with_capacity(longs.len());
                let mut ok = true;
                for leg in &longs {
                    match self.best_supply_apy(leg.mint) {
                        Some((_, apy)) => long_apys.push(apy),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let long_weight: f64 = longs.iter().map(|l| l.weight_fraction).sum();
                    let blended_long_supply_apy =
                        longs.iter().zip(&long_apys).map(|(l, apy)| l.weight_fraction * apy).sum::<f64>() / long_weight;
                    factor_borrow_gate::directional_basket_is_net_profitable(blended_long_supply_apy, blended_short_borrow_apy)
                } else {
                    false
                }
            };
            for apy in &short_apys {
                if !factor_borrow_gate::decide_directional_short_leg_with_carry(*apy, basket_is_net_profitable) {
                    log_warn!(
                        "multimodelv1: hawkes: factor {factor_idx} basket borrow gate refused (blended_short_borrow_apy={blended_short_borrow_apy:.2}% basket_net_profitable={basket_is_net_profitable})",
                    );
                    return;
                }
            }
        }

        let Some(sized) = self.size_hawkes_legs(&basket) else {
            log_warn!("multimodelv1: hawkes: factor {factor_idx} basket found but no real priceable route yet");
            return;
        };
        let any_leg_too_small = basket.iter().zip(&sized).any(|(leg, (_, _, _, sized_usd))| {
            let intended_usd = HAWKES_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
            *sized_usd < intended_usd * MIN_SIZED_FRACTION_OF_INTENDED
        });
        if any_leg_too_small {
            log_warn!("multimodelv1: hawkes: factor {factor_idx} basket sized down too far by real slippage -- skipping this cycle");
            return;
        }

        // Reserve this trade's real USDC commitment for the rest of this
        // resync cycle -- see `m_usdc_reserved_this_cycle`'s own doc
        // comment. Every gate has already passed at this point; nothing
        // past here can still refuse the open.
        self.state.m_usdc_reserved_this_cycle += sized.iter().map(|(_, _, _, u)| u).sum::<f64>();

        log_warn!(
            "multimodelv1: hawkes: opening factor {factor_idx}'s momentum basket ({} long leg(s), {} short leg(s))",
            longs.len(), shorts.len(),
        );
        for (symbol, mint, side, sized_usd) in &sized {
            match side {
                hawkes_factor::MomentumSide::Long => self.open_hawkes_long_leg(symbol, *mint, *sized_usd),
                hawkes_factor::MomentumSide::Short => self.open_hawkes_short_leg(symbol, *mint, *sized_usd),
            }
        }
        self.state.o_hawkes_open_factor = Some(factor_idx);
        self.state.m_hawkes_opened_at_secs = Some(now_secs);
    }

    /// Real, restart-safe "what dispersion basket is currently open" --
    /// unlike every other trade type in this file, there's no lending
    /// obligation to read back: the long legs are plain spot wallet
    /// balances (`execute_spot_leg`, no deposit -- see `open_dispersion_
    /// long_leg`'s doc comment), so this scans every curated mint's real,
    /// current SPL balance instead, pairing it with the real short-index
    /// Phoenix position (`dispersion_phoenix_position`). A nonzero
    /// balance with no matching short position (or a short position with
    /// no long balances at all) is still `Some` -- treated as a
    /// broken/partial basket by the close-pass, same fail-closed
    /// convention `current_open_pair`/`current_open_directional` already
    /// use.
    ///
    /// **Real USD-value dust floor** (`DISPERSION_DUST_FLOOR_USD`) --
    /// live-confirmed necessary, not a hypothetical: this mode's wallet
    /// has been shared across many earlier tests/trade types this same
    /// session, and this file's own pair/directional trades' leg-opening
    /// code can leave small leftover curated-mint balances behind on a
    /// partial/interrupted cycle. Without a floor, the very first real
    /// scan after connecting found 5 such pre-existing dust balances
    /// (several worth well under $1, one literally $0.08 of ETH) and
    /// spent the entire live session stuck force-closing them every
    /// cycle -- `run_dispersion_trade_cycle`'s real open-pass was never
    /// once reached. Priced Kamino-first-Solend-fallback, matching
    /// `update_residual_history_and_get_current`'s own real price
    /// resolution exactly.
    ///
    /// **A mint with no real price available at all is also treated as
    /// dust (skipped)** -- changed from an earlier, more conservative
    /// version of this function that kept unpriced mints as real legs,
    /// after two more real leftover balances (from the same pre-existing-
    /// dust incident above) turned out to have neither a live price *nor*
    /// a live sell route, permanently stuck blocking every open-pass for
    /// the rest of that live session too. User-directed trade-off,
    /// accepted knowingly: a *real* dispersion leg whose price data
    /// flickers out for one resync cycle would also drop out of this scan
    /// for that one cycle (and, in the unlikely event the short-index
    /// leg's own state also looked absent that same cycle, the open-pass
    /// could in principle re-fire on top of a still-real position) --
    /// judged an acceptable residual risk against the alternative of
    /// dispersion never being able to trade at all whenever this wallet
    /// picks up any illiquid dust from elsewhere.
    fn current_open_dispersion(&mut self) -> Option<(Vec<(&'static str, AccountId)>, i64)> {
        let Some(owner) = self.state.wallet() else { return None };
        let mint_usdc = self.configuration.mint_usdc;
        let mut legs: Vec<(&'static str, AccountId)> = Vec::new();
        for (symbol, mint, decimals) in curated_symbols() {
            if mint == mint_usdc {
                continue;
            }
            // `is_final=false` -- see `close_dispersion_long_leg`'s doc
            // comment for the real bug this fixes (2026-09-03): `true`
            // reads only the rooted (~12s+) stream, with no fallback to
            // the fast low-latency one, so this scan could stay blind to
            // a real, already-landed balance change (a completed sell, or
            // a completed buy) for far longer than the rooted stream's
            // normal design lag.
            let raw: u64 = self.wallet.token_mut().balance(&owner, &mint, false).iter().map(|(_, a)| *a).sum();
            if raw == 0 {
                continue;
            }
            let price_usd = self.state.o_dex.as_ref().and_then(|dex| {
                dex.kamino()
                    .reserve_by_mint(mint)
                    .map(|(_, r)| r.price_usd)
                    .or_else(|| dex.solend().reserve_by_mint(mint).map(|(_, r)| r.price_usd))
            });
            // A curated mint with no real Kamino/Solend price at all --
            // not just below the dollar floor -- is also treated as dust
            // (skipped), at the user's explicit direction: two real,
            // live-held leftover balances (neither dispersion's own --
            // see this function's own doc comment) had no live price *and*
            // no live sell route, permanently blocking every open-pass
            // this whole live session. Trade-off accepted knowingly: a
            // *real* dispersion leg whose price data flickers out for one
            // cycle would also be dropped from this scan that cycle -- see
            // this function's own doc comment for the full reasoning.
            let is_dust = match price_usd {
                Some(price) if price > 0.0 => {
                    let value_usd = (raw as f64 / 10f64.powi(decimals as i32)) * price;
                    value_usd < DISPERSION_DUST_FLOOR_USD
                }
                _ => true,
            };
            if !is_dust {
                legs.push((symbol, mint));
            }
        }
        let short_position = self.dispersion_phoenix_position();
        if legs.is_empty() && short_position.is_none() {
            return None;
        }
        Some((legs, short_position.unwrap_or(0)))
    }

    /// Trade type 3's real sizing for the long basket -- each leg's own
    /// intended notional is `DISPERSION_CYCLE_MIN_NOTIONAL_USD *
    /// weight_fraction` (all in USDC, since every long leg is a real
    /// USDC->mint spot buy, unlike `size_directional_legs`'s short legs
    /// which sell mint->USDC and need a price/decimals conversion back).
    /// Sized together through `factor_sizing::size_basket_exact` (real
    /// tick-aware quotes for CLMM/Orca hops, not just `cp_quote` on their
    /// full vault balance -- live-confirmed necessary: pool `965066337`
    /// alone inflated quotes 2-4x across several real curated mints this
    /// session, see that function's own doc comment). `None` if any leg
    /// has no real priceable route yet -- never opens a partial basket.
    fn size_dispersion_long_legs(
        &self,
        basket: &[dispersion_basket::DispersionBasketLeg],
    ) -> Option<Vec<(&'static str, AccountId, f64)>> {
        let dex = self.state.o_dex.as_ref()?;
        let mint_usdc = self.configuration.mint_usdc;
        const USDC_DECIMALS: i32 = 6;
        let mut legs = Vec::with_capacity(basket.len());
        for leg in basket {
            let leg_intended_usd = DISPERSION_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
            let intended_raw = (leg_intended_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
            if intended_raw == 0 {
                return None;
            }
            legs.push(factor_sizing::LegNotional { from_mint: mint_usdc, to_mint: leg.mint, intended_notional: intended_raw });
        }
        let sized = factor_sizing::size_basket_exact(&self.state.router, dex, &legs, LOOP_MAX_HOPS);
        // `size_basket_exact` zeroes the *entire* basket if even one leg
        // has no real safe notional at all (`max_safe_notional_exact`
        // returns 0 when `price_impact_bps_exact` finds no route
        // whatsoever, not just high slippage -- see that function's own
        // doc comment). Live-observed this session: every leg in a real
        // basket showing $0.00 sized, cycle after cycle, with no way to
        // tell which single leg was the real culprit from that outcome
        // alone. This diagnostic pass (only run when the whole basket
        // comes back zeroed, so it never costs anything on the common
        // success path) recomputes each leg's own real
        // `max_safe_notional_exact` individually to name the actual
        // unroutable mint(s), not just report the symptom.
        if sized.iter().all(|s| s.sized_notional == 0) {
            let culprits: Vec<String> = basket
                .iter()
                .zip(&legs)
                .filter_map(|(leg, l)| {
                    let safe = factor_sizing::max_safe_notional_exact(&self.state.router, dex, l.from_mint, l.to_mint, l.intended_notional, LOOP_MAX_HOPS);
                    (safe == 0).then(|| leg.symbol.to_string())
                })
                .collect();
            log_warn!(
                "multimodelv1: dispersion: basket zeroed by size_basket_exact -- unroutable leg(s) with real max_safe_notional=0: {}",
                if culprits.is_empty() { "none found (transient?)".to_string() } else { culprits.join(", ") },
            );
        }
        let mut out = Vec::with_capacity(basket.len());
        for (i, leg) in basket.iter().enumerate() {
            let sized_usd = sized[i].sized_notional as f64 / 10f64.powi(USDC_DECIMALS);
            out.push((leg.symbol, leg.mint, sized_usd));
        }
        Some(out)
    }

    /// Real per-cycle update: pulls every curated mint's own current real
    /// idiosyncratic-volatility (`factor_residual::RollingWindow::stats().
    /// stdev`, from `m_residual_history` -- already populated this cycle
    /// by `run_factor_resync`'s shared `update_residual_history_and_get_
    /// current` call, never recomputed here), averages every mint with
    /// enough real samples into one aggregate cross-sectional number,
    /// pushes it into `m_dispersion_history` (a second, aggregate-level
    /// use of the same `RollingWindow` type `m_residual_history`'s own
    /// per-symbol windows use), and returns that series' own current
    /// z-score -- dispersion's only automated entry/exit signal. `None`
    /// if there isn't at least one real per-symbol stdev yet this cycle,
    /// or not enough aggregate history yet for a real z-score
    /// (`factor_residual::MIN_SAMPLES_FOR_ZSCORE` warm-up, same as every
    /// other real use of this type).
    fn update_dispersion_signal(&mut self) -> Option<f64> {
        let stdevs: Vec<f64> = self.state.m_residual_history.values().filter_map(|w| w.stats().map(|s| s.stdev)).collect();
        if stdevs.is_empty() {
            return None;
        }
        let mean_stdev = stdevs.iter().sum::<f64>() / stdevs.len() as f64;
        self.state.m_dispersion_history.push(mean_stdev);
        self.state.m_dispersion_history.zscore(mean_stdev)
    }

    /// Trade type 3's real decision + execution driver, run every real
    /// factor resync once `dispersion_trading_enabled`, sharing this
    /// cycle's `factors` with the pair/directional cycles. Close pass
    /// first, unconditional every cycle: re-derives whatever's currently
    /// open from real state (`current_open_dispersion`) and force-closes
    /// on any of -- a broken/partial basket (real long legs with no real
    /// short, or vice versa), the real exit signal (`dispersion_basket::
    /// should_exit_dispersion` on the aggregate z-score -- fails **open**
    /// on a missing signal, matching `run_pair_trade_cycle`'s own
    /// asymmetric close-pass precedent, since an exit needs real evidence
    /// to fire), or an explicit human close request. Open pass only if
    /// nothing is currently open and the real entry signal
    /// (`dispersion_basket::should_enter_dispersion`) clears.
    fn run_dispersion_trade_cycle(&mut self, factors: &factor_graph::StructuralFactors, aggregate_zscore: Option<f64>) {
        if let Some((legs, short_position)) = self.current_open_dispersion() {
            if legs.is_empty() || short_position == 0 {
                log_warn!(
                    "multimodelv1: dispersion: {} real long leg(s) but short_index_position={short_position} -- broken/partial basket, force-closing",
                    legs.len(),
                );
                for (symbol, mint) in &legs {
                    self.close_dispersion_long_leg(symbol, *mint);
                }
                self.close_dispersion_short_index_leg();
                self.state.dispersion_basket_legs.clear();
                return;
            }
            let signal_says_exit = aggregate_zscore.is_some_and(dispersion_basket::should_exit_dispersion);
            let human_close_requested = self.state.o_dispersion_close_requested;
            if signal_says_exit || human_close_requested {
                log_warn!(
                    "multimodelv1: dispersion: closing {} long leg(s) + short index -- signal_says_exit={signal_says_exit} human_close_requested={human_close_requested}",
                    legs.len(),
                );
                for (symbol, mint) in &legs {
                    self.close_dispersion_long_leg(symbol, *mint);
                }
                self.close_dispersion_short_index_leg();
                self.state.dispersion_basket_legs.clear();
                self.state.o_dispersion_close_requested = false;
            }
            return;
        }
        self.state.o_dispersion_close_requested = false;

        let Some(zscore) = aggregate_zscore else {
            log_warn!("multimodelv1: dispersion: no real aggregate z-score yet this cycle (not enough residual/dispersion history)");
            return;
        };
        if !dispersion_basket::should_enter_dispersion(zscore) {
            log_warn!(
                "multimodelv1: dispersion: aggregate z={zscore:.2} hasn't cleared the {:.1}σ entry threshold this cycle",
                dispersion_basket::DISPERSION_ENTRY_ZSCORE,
            );
            return;
        }

        // Short-leg-feasibility precheck -- live-confirmed necessary
        // (2026-09-05): without this, a basket that clears the entry
        // z-score but can never actually get hedged (Phoenix trader
        // frozen) still opens every long leg it can, pays real
        // spread/gas on each, and gets force-closed one cycle later by
        // `current_open_dispersion`'s own safety net -- a real, paid,
        // repeating round trip for zero net exposure ever taken. Check
        // the one condition `open_dispersion_short_index_leg` itself
        // already knows is fatal and unrecoverable (`is_trader_frozen`,
        // see `top_up_dispersion_margin`'s own doc comment) *before*
        // spending anything on long legs, not after.
        match self.state.o_phoenix.as_ref() {
            None => {
                log_warn!("multimodelv1: dispersion: skipping open pass -- phoenix state not ready yet, z={zscore:.2}");
                return;
            }
            Some(phoenix) if !phoenix.trader_registered() => {
                self.bootstrap_dispersion_phoenix_trader();
                return;
            }
            Some(phoenix) if phoenix.is_trader_frozen() => {
                log_warn!(
                    "multimodelv1: dispersion: skipping open pass -- phoenix trader account is frozen, short leg can never land this session, z={zscore:.2}",
                );
                return;
            }
            _ => {}
        }

        let usdc_value = self.available_usdc_value();
        // Rough pre-check floor: the long basket's own total plus a 25%
        // buffer for Phoenix margin -- not the real safety mechanism
        // (`top_up_dispersion_margin` is, checked for real right before
        // the short is placed), just a cheap early skip when there's
        // obviously not enough real USDC to bother building a basket at
        // all this cycle.
        let usdc_floor = DISPERSION_CYCLE_MIN_NOTIONAL_USD * 1.25;
        if usdc_value < usdc_floor {
            log_warn!(
                "multimodelv1: dispersion: skipping open pass -- wallet's real, currently-tracked USDC (${usdc_value:.2}) is below the ${usdc_floor:.2} floor, z={zscore:.2}",
            );
            return;
        }

        let Some(dex) = self.state.o_dex.as_ref() else {
            log_warn!("multimodelv1: dispersion: skipping open pass -- dex state not ready yet, z={zscore:.2}");
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let liquidity_floor_raw = (DISPERSION_CANDIDATE_LIQUIDITY_FLOOR_USD * 10f64.powi(6)).round() as u64;
        // Diagnostic counters -- real, live-requested breakdown of *why*
        // the candidate pool is small, not just that it is: how many
        // curated mints (excluding USDC) have real residual history at
        // all, how many of those clear the real *buy*-direction liquidity
        // floor, and how many of those *also* clear the real *sell*-
        // direction floor. Only costs anything when logged below (basket
        // refused).
        let mut total_curated = 0usize;
        let mut have_stdev = 0usize;
        let mut clear_buy_liquidity = 0usize;
        // Temporary, live-diagnostic-only: real per-mint reason a
        // buy-liquid candidate failed the *sell*-direction check --
        // 0-of-N has cleared it every single cycle across two live runs
        // now, which is suspicious enough (not just "illiquid") to need a
        // real root cause, not just the aggregate breakdown log below.
        let mut sell_fail_diag: Vec<String> = Vec::new();
        let candidates: Vec<dispersion_basket::DispersionCandidate> = curated_symbols()
            .enumerate()
            .filter(|(_, (_, mint, _))| *mint != mint_usdc)
            .filter_map(|(token_index, (symbol, mint, decimals))| {
                total_curated += 1;
                let stdev = self.state.m_residual_history.get(&mint)?.stats()?.stdev;
                have_stdev += 1;
                // Buy direction (USDC -> mint) -- what a real open actually
                // does. `_exact`: real tick-aware quotes for CLMM/Orca
                // hops, not `cp_quote` on their full vault balance --
                // live-confirmed necessary, see `factor_sizing::
                // max_safe_notional_exact`'s own doc comment.
                let buy_safe = factor_sizing::max_safe_notional_exact(&self.state.router, dex, mint_usdc, mint, liquidity_floor_raw, LOOP_MAX_HOPS);
                if buy_safe < liquidity_floor_raw {
                    return None;
                }
                clear_buy_liquidity += 1;
                // Sell direction (mint -> USDC) -- what a real close needs.
                // Live-confirmed necessary, not hypothetical: a real
                // candidate cleared the buy-direction floor, opened, and
                // then couldn't be sold back (real close attempts failed
                // "no route found" ~79% of the time) -- buy and sell
                // routes through this router are not guaranteed symmetric,
                // so both must be checked before a mint is eligible.
                // Needs a real price to convert the same $ floor into this
                // mint's own raw units; no real price yet is treated the
                // same as failing the floor (conservative, not optimistic).
                let price_usd = dex
                    .kamino()
                    .reserve_by_mint(mint)
                    .map(|(_, r)| r.price_usd)
                    .or_else(|| dex.solend().reserve_by_mint(mint).map(|(_, r)| r.price_usd));
                let Some(price_usd) = price_usd.filter(|&p| p > MIN_SANE_RESERVE_PRICE_USD) else {
                    sell_fail_diag.push(format!("{symbol}: no real (or sane) price yet"));
                    return None;
                };
                let sell_reference_raw = ((DISPERSION_CANDIDATE_LIQUIDITY_FLOOR_USD / price_usd) * 10f64.powi(decimals as i32)).round() as u64;
                if sell_reference_raw == 0 {
                    sell_fail_diag.push(format!("{symbol}: price=${price_usd:.6} decimals={decimals} -> sell_reference_raw rounds to 0"));
                    return None;
                }
                // `_exact`: same real tick-aware quoting as the
                // buy-direction check above -- this is the check that was
                // actually blocked by the CLMM approximation gap this
                // session (real candidates like ETH/bSo1/mSoL/jtoj showed
                // 2-58x inflated `cp_quote` outputs through CLMM hops,
                // making them look unsafe when they weren't).
                let sell_safe = factor_sizing::max_safe_notional_exact(&self.state.router, dex, mint, mint_usdc, sell_reference_raw, LOOP_MAX_HOPS);
                if sell_safe < sell_reference_raw {
                    sell_fail_diag.push(format!("{symbol}: price=${price_usd:.6} sell_reference_raw={sell_reference_raw} sell_safe={sell_safe}"));
                    return None;
                }
                Some(dispersion_basket::DispersionCandidate { symbol, mint, token_index, residual_stdev: stdev })
            })
            .collect();
        let Some(basket) = dispersion_basket::build_dispersion_basket(&candidates) else {
            log_warn!(
                "multimodelv1: dispersion: candidate breakdown -- {total_curated} curated mints, {have_stdev} with real residual history, {clear_buy_liquidity} clearing the real ${:.0} buy-direction floor, {} of those also clearing the real sell-direction floor",
                DISPERSION_CANDIDATE_LIQUIDITY_FLOOR_USD,
                candidates.len(),
            );
            if !sell_fail_diag.is_empty() {
                log_warn!("multimodelv1: dispersion: sell-direction failure detail -- {}", sell_fail_diag.join(" | "));
            }
            log_warn!(
                "multimodelv1: dispersion: no real basket buildable this cycle (fewer than {} warmed-up candidates) -- refusing to open, z={zscore:.2}",
                dispersion_basket::MIN_DISPERSION_BASKET_SIZE,
            );
            return;
        };

        let Some(sized_longs) = self.size_dispersion_long_legs(&basket) else {
            log_warn!("multimodelv1: dispersion: basket found but no real priceable route yet");
            return;
        };
        let too_small_legs: Vec<String> = basket
            .iter()
            .zip(&sized_longs)
            .filter_map(|(leg, (symbol, _, sized_usd))| {
                let intended_usd = DISPERSION_CYCLE_MIN_NOTIONAL_USD * leg.weight_fraction;
                if *sized_usd < intended_usd * MIN_SIZED_FRACTION_OF_INTENDED {
                    Some(format!("{symbol}(intended=${intended_usd:.2} sized=${sized_usd:.2})"))
                } else {
                    None
                }
            })
            .collect();
        if !too_small_legs.is_empty() {
            log_warn!(
                "multimodelv1: dispersion: basket sized down too far by real slippage -- skipping this cycle, z={zscore:.2}, culprit leg(s): {}",
                too_small_legs.join(", "),
            );
            return;
        }

        let basket_with_notionals: Vec<(dispersion_basket::DispersionBasketLeg, f64)> =
            basket.iter().zip(&sized_longs).map(|(leg, (_, _, sized_usd))| (*leg, *sized_usd)).collect();
        let short_notional_usd =
            dispersion_basket::aggregate_market_factor_exposure(&basket_with_notionals, &factors.eigenvectors, |mint| {
                self.curated_index_of(mint)
            })
            .abs();
        if short_notional_usd <= 0.0 {
            log_warn!("multimodelv1: dispersion: basket's aggregate market-factor exposure is ~0 -- nothing real to hedge, refusing to open");
            return;
        }

        // Reserve this trade's real USDC commitment for the rest of this
        // resync cycle -- see `m_usdc_reserved_this_cycle`'s own doc
        // comment. Every gate has already passed at this point; nothing
        // past here can still refuse the open. Only the long legs' own
        // spend counts here -- the short index leg's Phoenix margin is
        // funded separately by `top_up_dispersion_margin`, which already
        // reads `available_usdc_value()` itself at the point it actually
        // spends.
        self.state.m_usdc_reserved_this_cycle += sized_longs.iter().map(|(_, _, u)| u).sum::<f64>();

        log_warn!(
            "multimodelv1: dispersion: opening {}-leg basket (z={zscore:.2}) against a ${short_notional_usd:.2} short index leg",
            sized_longs.len(),
        );
        self.state.dispersion_basket_legs = sized_longs.iter().map(|(s, m, _)| (*s, *m)).collect();
        for (symbol, mint, sized_usd) in &sized_longs {
            self.open_dispersion_long_leg(symbol, *mint, *sized_usd);
        }
        self.open_dispersion_short_index_leg(short_notional_usd);
    }

    /// Trade type 4 (arbitrage) -- ported verbatim from `arbv1::state::
    /// StateHelper::detect_and_log_opportunity` (same reasoning throughout,
    /// see that method's own doc comment for the full rationale). Runs
    /// `planner::find_opportunity` against `state.router` and, on a hit,
    /// re-verifies any CLMM hop against the exact tick-aware quote before
    /// trusting it (`planner::reverify_with_exact_quotes`) -- a pool whose
    /// exact quote disagrees gets cooled down instead of repeatedly
    /// re-selected. Called from both `low_latency` (the ~400ms processed
    /// stream) and `CommitHook::finish` (the ~12s commit path), same dual
    /// call sites `arbv1` itself uses -- arbitrage has no factor-model
    /// dependency, so it deliberately does not go through `run_factor_
    /// resync`'s ~30s gate.
    fn detect_arbitrage_opportunity(&mut self) {
        if let Some(owner) = self.state.wallet() {
            self.state.arb_find_opportunity_call_count += 1;
            self.state.router.set_current_slot(self.state.last_slot);
            match planner::find_opportunity(&self.state.router, self.wallet, &owner) {
                Some(opp) => {
                    let opp = match self.state.o_dex.as_ref() {
                        Some(dex) => match planner::reverify_with_exact_quotes(&opp.cycle, &self.state.router, dex) {
                            planner::ReverifyOutcome::Ok(cycle) => {
                                planner::ArbitrageOpportunity { cycle, wallet_balance: opp.wallet_balance }
                            }
                            planner::ReverifyOutcome::HopFailed(failure) => {
                                if failure.coolable {
                                    self.state.router.mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                                }
                                log_warn!(
                                    "multimodelv1: arbitrage opportunity @ slot {}: rejected -- exact-quote re-verification invalidated \
                                     pool {} (cooling down {} slots: {}) (start_token={} amount_in={} hops={})",
                                    self.state.last_slot,
                                    failure.pool_id,
                                    planner::POOL_COOLDOWN_SLOTS,
                                    failure.coolable,
                                    opp.cycle.start_token(),
                                    opp.cycle.amount_in(),
                                    opp.cycle.hops.len(),
                                );
                                return;
                            }
                            planner::ReverifyOutcome::BelowThreshold(corrected) => {
                                log_warn!(
                                    "multimodelv1: arbitrage opportunity @ slot {}: rejected -- below profit threshold after \
                                     exact-quote correction (start_token={} amount_in={} amount_out={} profit_raw={} profit_bps={} hops={})",
                                    self.state.last_slot,
                                    corrected.start_token(),
                                    corrected.amount_in(),
                                    corrected.amount_out(),
                                    corrected.profit_raw(),
                                    corrected.profit_bps(),
                                    corrected.hops.len(),
                                );
                                for (i, hop) in corrected.hops.iter().enumerate() {
                                    log_warn!(
                                        "  hop {}: dex={:?} pool={} {} -> {} : amount_in={} amount_out={}",
                                        i, hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                                    );
                                }
                                return;
                            }
                        },
                        None => opp,
                    };
                    log_warn!(
                        "multimodelv1: arbitrage opportunity @ slot {}: start_token={} amount_in={} amount_out={} \
                         profit_raw={} profit_bps={} wallet_balance={} hops={}",
                        self.state.last_slot,
                        opp.cycle.start_token(),
                        opp.cycle.amount_in(),
                        opp.cycle.amount_out(),
                        opp.cycle.profit_raw(),
                        opp.cycle.profit_bps(),
                        opp.wallet_balance,
                        opp.cycle.hops.len(),
                    );
                    for (i, hop) in opp.cycle.hops.iter().enumerate() {
                        log_warn!(
                            "  hop {}: dex={:?} pool={} {} -> {} : amount_in={} amount_out={}",
                            i, hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                        );
                    }
                    // Handed off to evaluate()'s execute_arbitrage_opportunity --
                    // consumed there via .take(), not replanned on every event
                    // until the next call re-detects a cycle.
                    self.state.o_arb_pending_opportunity = Some(opp);
                }
                None if self.state.last_slot % 100 == 0 => {
                    log_warn!("multimodelv1: arbitrage check @ slot {}: no profitable cycle found", self.state.last_slot);
                }
                None => {}
            }
        } else if self.state.last_slot % 100 == 0 {
            log_warn!("multimodelv1: arbitrage check @ slot {}: no wallet keypair yet", self.state.last_slot);
        }
    }

    /// Trade type 4 (arbitrage) execution -- ported verbatim from
    /// `arbv1::state::StateHelper::build_execution_plan` (same reasoning
    /// throughout, see that method's own doc comment). Builds directly onto
    /// `self.wallet` inside a checkpoint + atomic group (single atomic
    /// transaction only -- deliberately NOT wired into this file's async
    /// multi-hop chain executor / `PendingHopChain`, since a cycle that
    /// lands hop 0 but not the rest leaves the wallet holding an unintended
    /// intermediate token with no guaranteed path back to the start token,
    /// unlike a one-way spot leg's partial completion). Net profit (after
    /// real transaction cost) is only computed when the cycle's start
    /// token is SOL -- see `ArbitrageCycle::net_profit_lamports`'s own doc
    /// comment for why a non-SOL start token can't be netted against a
    /// lamport fee without a separate price conversion; any other start
    /// token's cycle is unconditionally discarded regardless of gross
    /// profit. This is a real, known scope boundary inherited deliberately
    /// from `arbv1`, not silently -- generalizing it would need a real
    /// USD/SOL price conversion for arbitrary tokens.
    ///
    /// Consumes `state.o_arb_pending_opportunity` (set by `detect_
    /// arbitrage_opportunity`) via `.take()`, so a given opportunity is
    /// only planned once.
    fn execute_arbitrage_opportunity(&mut self) {
        let Some(opp) = self.state.o_arb_pending_opportunity.take() else { return };
        let Some(owner) = self.state.wallet() else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };

        log_warn!(
            "multimodelv1: arbitrage execution plan @ slot {}: {} hop{} for start_token={} amount_in={}",
            self.state.last_slot,
            opp.cycle.hops.len(),
            if opp.cycle.hops.len() == 1 { "" } else { "s" },
            opp.cycle.start_token(),
            opp.cycle.amount_in(),
        );

        let cu_before = self.wallet.cu();
        let checkpoint = self.wallet.queue_checkpoint();
        self.wallet.begin_atomic_group();
        for (i, hop) in opp.cycle.hops.iter().enumerate() {
            let (Some(source_ata), Some(dest_ata)) = (
                self.wallet.append_create_ata(owner, hop.input_mint),
                self.wallet.append_create_ata(owner, hop.output_mint),
            ) else {
                self.wallet.rollback_to(checkpoint);
                self.wallet.end_atomic_group();
                log_warn!("multimodelv1: arbitrage hop {i}: FAILED to derive token account(s) for owner={owner}");
                return;
            };
            match dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet) {
                Ok(()) => {
                    log_warn!(
                        "multimodelv1: arbitrage hop {i}: OK dex={:?} pool={} {} -> {} amount_in={} amount_out={}",
                        hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                    );
                }
                Err(e) => {
                    self.wallet.rollback_to(checkpoint);
                    self.wallet.end_atomic_group();
                    // Fix (2026-09-07): live-confirmed a `PoolNotReady`
                    // pool can fail identically for 30+ minutes across
                    // multiple restarts -- not self-correcting.
                    // `note_pool_not_ready` tracks the repeated case and
                    // reports once it's crossed a real threshold, so we
                    // still cool down eventually instead of retrying
                    // forever.
                    let is_pool_not_ready = matches!(e, crate::trader::types::TraderError::PoolNotReady);
                    let coolable = if is_pool_not_ready {
                        self.state.router.note_pool_not_ready(hop.pool_id)
                    } else {
                        true
                    };
                    if coolable {
                        if is_pool_not_ready {
                            self.state.router.mark_pool_not_ready_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        } else {
                            self.state.router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                    }
                    log_warn!(
                        "multimodelv1: arbitrage hop {i}: FAILED dex={:?} pool={} {} -> {}: {} (cooling down {} slots: {coolable})",
                        hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, e, planner::POOL_COOLDOWN_SLOTS,
                    );
                    return;
                }
            }
        }
        if !self.wallet.atomic_group_fits(checkpoint) {
            self.wallet.rollback_to(checkpoint);
            self.wallet.end_atomic_group();
            for hop in opp.cycle.hops.iter() {
                self.state.router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
            }
            log_warn!(
                "multimodelv1: arbitrage execution plan @ slot {}: {}-hop atomic group too large for one transaction (cooling down {} slots)",
                self.state.last_slot,
                opp.cycle.hops.len(),
                planner::POOL_COOLDOWN_SLOTS,
            );
            return;
        }
        self.wallet.end_atomic_group();
        let route_cu = self.wallet.cu().saturating_sub(cu_before);

        log_warn!(
            "multimodelv1: arbitrage execution plan @ slot {}: all {} hops built successfully; total_cu={} instruction_count={}",
            self.state.last_slot,
            opp.cycle.hops.len(),
            route_cu,
            self.wallet.instruction_count().saturating_sub(checkpoint),
        );

        if opp.cycle.start_token() == self.configuration.mint_sol {
            let priority_rate: u64 = PriorityLevel::Medium.into();
            let net_profit = opp.cycle.net_profit_lamports(route_cu, priority_rate, 1);
            log_warn!(
                "multimodelv1: arbitrage execution plan @ slot {}: net_profit={} lamports (gross_profit={} lamports, base_fee=5000, priority_fee_rate={} micro-lamports/cu, total_cu={})",
                self.state.last_slot,
                net_profit,
                opp.cycle.profit_raw(),
                priority_rate,
                route_cu,
            );
            if 0 < net_profit {
                // Real positive net profit (after fees) -- bid urgently
                // and opt this tick's send into trying real Astralane
                // landing (see Wallet::send_bundler_pair's own doc
                // comment). The route built above stays queued;
                // evaluate()'s own drain_and_send() call sends it.
                self.wallet.set_priority_fee(PriorityLevel::High);
                // Counted here, not against drain_and_send's own Ok(_)
                // results (unlike arbv1, where arbitrage is the *only*
                // thing that ever queues a transaction): multimodelv1's
                // other trade types can queue onto the same `self.wallet`
                // in the same tick, so a drain_and_send batch can't be
                // reliably attributed back to a specific trade type.
                // "Queued for a real send" is the countable event instead
                // of "confirmed sent" -- still a real, monotonic circuit
                // breaker against `evaluate()` repeatedly trying to build
                // more arbitrage routes than the lifetime cap allows.
                self.state.arb_tx_count += 1;
            } else {
                self.wallet.rollback_to(checkpoint);
                log_warn!(
                    "multimodelv1: arbitrage execution plan @ slot {}: net profit not positive, discarding route (not sent)",
                    self.state.last_slot,
                );
            }
        } else {
            self.wallet.rollback_to(checkpoint);
            log_warn!(
                "multimodelv1: arbitrage execution plan @ slot {}: net profit not computed -- start_token={} is not SOL, gross_profit={} raw units isn't directly comparable to lamport transaction fees; discarding route (not sent)",
                self.state.last_slot,
                opp.cycle.start_token(),
                opp.cycle.profit_raw(),
            );
        }
    }

    /// Sub-phase 5b/5c: real factor resync (and, if `pair_trading_enabled`,
    /// the real pair-trade cycle) on a periodic cadence, gated by
    /// `TriggerEnableFactorLogging`/`TriggerEnablePairTrading` (`false` by
    /// default -- see those fields' doc comments on `State`). Reuses
    /// `factor_graph::MAX_FACTOR_STALENESS_SECS` as both the staleness
    /// bound *and* the resync period -- consistent with that constant's
    /// own doc comment ("an upper bound on trust, not just a resync
    /// schedule"). Ends with the real drain-and-send tail every other
    /// real bot mode's `evaluate()` uses (see `pending_route_pools`'s own
    /// doc comment for the real send-rejection gap this closes) -- sub-phase
    /// 5a/5b never needed this since neither ever queued a real
    /// transaction.
    pub(crate) fn evaluate(&mut self) {
        // Ported from `arbv1::state::StateHelper::evaluate`'s own opening
        // (same idempotent/self-latching reasoning: `ensure_bundler_nonce_
        // created` no-ops after the first real call). Applied mode-wide,
        // every tick, same as every other real bot mode's `evaluate()` --
        // this mode previously never touched priority fee at all (every
        // send went out at the wallet default, `PriorityLevel::None`), so
        // this also brings the pre-existing pair/directional/dispersion
        // sends in line with the rest of the codebase, not just arbitrage.
        // `execute_arbitrage_opportunity` below may still raise this to
        // `High` for a specific tick's send; resetting to `Medium` here
        // unconditionally at the start of every tick keeps that bump from
        // ever leaking into an unrelated later tick's sends.
        self.wallet.set_priority_fee(PriorityLevel::Medium);
        if let Some(owner) = self.state.wallet() {
            self.wallet.ensure_bundler_nonce_created(owner);
        }
        // See `State::positions_loaded`'s doc comment -- one-way latch,
        // checked every tick until it flips, never re-checked after.
        if !self.state.positions_loaded && crate::graph::all_subscriptions_acked() {
            self.state.positions_loaded = true;
            log_warn!("multimodelv1: positions_loaded -- startup subscription burst acked, trading decisions now trust real balances");
        }

        // Temporary, standalone manual-cleanup tool -- see
        // `sweep_requested_mint`'s own doc comment. Independent of every
        // trade type's enabled flag, same reasoning as arbitrage's own
        // independence from the gate below: this is a one-shot human
        // request, not part of any trade type's decision loop -- and it
        // already does its own real-balance check immediately before
        // acting, so it doesn't need `positions_loaded` (a human
        // resending the trigger is the retry mechanism if it's too
        // early).
        self.sweep_requested_mint();

        if self.state.positions_loaded
            && (self.state.factor_logging_enabled
                || self.state.pair_trading_enabled
                || self.state.directional_trading_enabled
                || self.state.dispersion_trading_enabled)
        {
            let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
            let due = match self.state.last_full_resync_secs {
                None => true,
                Some(last) => now_secs.saturating_sub(last) > factor_graph::MAX_FACTOR_STALENESS_SECS,
            };
            if due {
                self.run_factor_resync();
            }
        }

        // Trade type 4 (arbitrage) -- deliberately independent of the
        // factor-logging/pair/directional/dispersion gate above: no
        // factor-model dependency, always active once a real wallet is
        // loaded (no `*_trading_enabled` flag), matching `arbv1`'s own
        // real behavior per the explicit design decision (see `State::
        // arb_tx_count`'s doc comment). Same warm-up (~20 commits) and
        // hard lifetime send cap (10, never reset) `arbv1::state::
        // StateHelper::evaluate` itself gates on. `positions_loaded` added
        // 2026-09-04 -- see that field's own doc comment.
        if self.state.positions_loaded
            && self.state.wallet().is_some()
            && self.state.arb_slot_delta_since_start >= 20
            && self.state.arb_tx_count <= 10
        {
            self.execute_arbitrage_opportunity();
        }

        let mut any_send_failed = false;
        for (sig, result) in self.wallet.drain_and_send() {
            match result {
                Ok(_) => log_warn!("multimodelv1: sent transaction {sig}"),
                Err(e) => {
                    log_error!("multimodelv1: failed to send transaction {sig}: {e}");
                    any_send_failed = true;
                }
            }
        }
        if any_send_failed && !self.state.pending_route_pools.is_empty() {
            for &pool_id in &self.state.pending_route_pools {
                self.state.router.mark_pool_cooldown(pool_id, planner::POOL_COOLDOWN_SLOTS);
            }
            self.state.pending_route_pools.clear();
        }
    }
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        self.o_commit_slot = Some(slot);
        self.state.last_slot = slot;
        // Real cleanup for a signature `mid_on_tx` never reports on at all
        // (e.g. its blockhash simply expired unlanded) -- see
        // `HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS`'s own doc comment. Whether the
        // tracked tx actually landed is checked against real evidence
        // before giving up -- see `PendingHopChain::produced_mint_balance_
        // before`'s own doc comment for why (the host gives no way to
        // directly poll a signature's status, so this is the only
        // independent signal available).
        //
        // Collected instead of acted on inline -- `Wallet::token_mut`/
        // `advance_pending_hop_chain` (disjoint-ish uses of `self`) don't
        // mix cleanly with iterating `self.state.m_pending_hop_chains`
        // directly, so this is a collect-then-apply pass, same shape the
        // old invalidate-only version already used.
        let now_slot = slot;
        let mut to_remove: Vec<Signature> = Vec::new();
        let mut to_advance: Vec<(Signature, PendingHopChain)> = Vec::new();
        let mut expired_mints: Vec<(AccountId, AccountId)> = Vec::new();
        for (signature, chain) in self.state.m_pending_hop_chains.iter() {
            if now_slot.saturating_sub(chain.sent_slot) < HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS {
                continue;
            }
            let current_balance: u64 = self
                .wallet
                .token_mut()
                .balance(&chain.owner, &chain.produced_mint, false)
                .iter()
                .map(|(_, a)| *a)
                .sum();
            if current_balance > chain.produced_mint_balance_before {
                // Real evidence the tracked hop actually landed (its
                // output mint's real balance grew since it was sent) even
                // though `mid_on_tx` never reported it -- advance the
                // chain for real instead of abandoning a live position.
                log_warn!(
                    "multimodelv1: hop chain -- tx {signature} (sent slot {}) never got a real confirmation from mid_on_tx within {HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS} slots, but real balance of {} grew ({} -> {}) since it was sent -- treating as landed and advancing the chain instead of stranding it",
                    chain.sent_slot, chain.produced_mint, chain.produced_mint_balance_before, current_balance,
                );
                to_advance.push((*signature, chain.clone()));
            } else {
                // Real, live-confirmed fix (2026-09-03): this used to
                // index `chain.hops[0]` directly, which panics (aborting
                // the whole guest) once `chain.hops` is legitimately
                // empty -- see `PendingHopChain::hops`'s own doc comment
                // for exactly when that happens (the route's own final
                // hop expiring). `consumed_mint`/`produced_mint` are
                // always available regardless, and are also the two
                // mints whose now-unverified cached balances get
                // invalidated below.
                //
                // Real, live-confirmed fix (2026-09-04): the first
                // version of this only invalidated `produced_mint`,
                // missing `consumed_mint` entirely -- see that field's
                // own doc comment for the real incident this caused (a
                // dispersion close-pass re-selling an already-emptied
                // position 5 times in a row against a `consumed_mint`
                // balance nothing had ever invalidated).
                log_error!(
                    "multimodelv1: hop chain -- tx {signature} (sent slot {}) never got a real confirmation from mid_on_tx within {HOP_CHAIN_SIGNATURE_EXPIRY_SLOTS} slots -- giving up on it with {} hop(s) still unsent; owner={} may or may not hold a real balance of {} or {} (whether this tracked tx actually landed is unknown -- invalidating this database's own cached balance for both rather than keep trusting now-unverified numbers)",
                    chain.sent_slot, chain.hops.len(), chain.owner, chain.consumed_mint, chain.produced_mint,
                );
                expired_mints.push((chain.owner, chain.consumed_mint));
                expired_mints.push((chain.owner, chain.produced_mint));
                to_remove.push(*signature);
            }
        }
        for signature in &to_remove {
            self.state.m_pending_hop_chains.remove(signature);
        }
        // Real, live-confirmed deadlock (2026-09-07): `invalidate` forgets
        // the balance on both streams and then just waits for the next
        // organic account-update push to repopulate it -- but every open
        // pass's own USDC floor check (`available_usdc_value`) reads
        // exactly this invalidated balance, so once USDC itself is the
        // invalidated mint, every trade type refuses to send anything that
        // would touch USDC, nothing ever does, and no update ever arrives:
        // a permanent $0.00 read despite a real, untouched, correct
        // balance sitting on-chain (confirmed live: 80.70 real USDC while
        // every trade type sat blocked for 5+ minutes). Forces a fresh
        // snapshot instead of waiting indefinitely, same
        // `ata_subscribe_request`/`subscribe_now`/`keep_ata_subscriptions`
        // pattern `send_single_hop_as_astralane_tx` already uses to track
        // a brand-new destination ATA.
        for (owner, mint) in expired_mints {
            self.wallet.token_mut().invalidate(&owner, &mint);
            if let Some(sub_req) = self.wallet.ata_subscribe_request(owner, mint) {
                match SubscriptionQueue::subscribe_now(self.graph, vec![sub_req]) {
                    Ok(subs) => self.wallet.keep_ata_subscriptions(subs),
                    Err(e) => log_error!(
                        "multimodelv1: hop chain -- failed to re-subscribe invalidated ATA for mint {mint}: {e} -- its balance may stay stuck at 0 until an unrelated update touches it",
                    ),
                }
            }
        }
        for (signature, chain) in to_advance {
            self.state.m_pending_hop_chains.remove(&signature);
            self.advance_pending_hop_chain(signature, now_slot, chain);
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.wallet.on_account(header, body);
        // Unconditional, same reasoning as `low_latency`'s identical
        // dispatch -- see that call site's doc comment for the real bug
        // this fixes.
        if let Some(pos) = self.state.o_pair_kamino_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(pos) = self.state.o_pair_solend_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(pos) = self.state.o_directional_kamino_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(pos) = self.state.o_directional_solend_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(pos) = self.state.o_hawkes_kamino_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(pos) = self.state.o_hawkes_solend_position.as_mut() {
            pos.on_account(header, body);
        }
        if let Some(phoenix) = self.state.o_phoenix.as_mut() {
            phoenix.on_account(header, body);
        }
        // Same freshness gate `arbv1::state`'s own `on_account` uses --
        // this rooted (~12s-late) stream must never roll `router` state
        // backwards past what `low_latency`'s ~400ms stream already
        // recorded. See this module's doc comment.
        if self.is_newer_than_low_latency(header.accountid, header.slot) {
            self.record_low_latency_slot(header.accountid, header.slot);
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(header, body);
                dex.refresh_account_router(header.accountid, &mut self.state.router);
            }
        }
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        let db = self.wallet.token_mut();
        db.on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        self.state.arb_slot_delta_since_start += 1;
        if let Some(mut dex) = self.state.o_dex.take() {
            dex.flush_pool(self.graph, planner::DEX_POOL_SUBSCRIPTION_FLUSH_BUDGET).expect("flush pool");
            if let Err(e) = dex.flush_subscriptions(self.graph, 128) {
                log_error!("multimodelv1: failed to flush dex subscription queue: {e}");
            }
            // TEMPORARY DIAGNOSTIC (2026-09-07): checking whether a
            // chronically `PoolNotReady` pool (real, live-confirmed
            // across 25+ restarts -- see `TradeRouter::mark_pool_not_
            // ready_cooldown`'s own doc comment) is stuck behind a real
            // backlog in this shared 128/cycle flush budget, or something
            // else. Same cadence as `arbv1::state`'s own periodic pool
            // stats log.
            if self.state.last_slot % 100 == 0 {
                log_warn!("multimodelv1: dex pool stats @ slot {}: {}", self.state.last_slot, dex.pool_stats());
            }
            self.state.o_dex.replace(dex);
        }
        // Trade type 4 (arbitrage) -- also detect on the ~12s commit path,
        // not just `low_latency`'s ~400ms stream, same dual call sites
        // `arbv1::state`'s own `CommitHook::finish` uses. Independent of
        // this mode's own factor-resync cadence.
        self.detect_arbitrage_opportunity();
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound> for StateHelper<'a> {
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(SystemTime::now()));
            }
            MessageAction::AdjustConfiguration(new_configuration) => {
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(x) => match x {
                CustomMessageInbound::Blank => {}
                CustomMessageInbound::Wallet(rc_keypair) => {
                    // Temporary diagnostic (2026-09-06): see
                    // `apply_residual_snapshot`'s own diagnostic doc
                    // comment for the real hang this is chasing -- this
                    // arm does the real wallet-authority-subscription
                    // batching (including trade type 5's newly added
                    // Hawkes obligations), a real candidate for a hang if
                    // `SubscriptionQueue::subscribe_now` or any
                    // `apply_authority` call below never returns. Remove
                    // once the real root cause is found.
                    log_warn!("multimodelv1: CustomMessageInbound::Wallet -- entered");
                    let keypair = rc_unlock(&rc_keypair);
                    let pubkey = keypair.pubkey();
                    let account_id = account_id_from_pubkey(&pubkey);
                    self.wallet.append_key(rc_keypair.clone(), self.graph).unwrap();
                    self.wallet.set_payer(account_id);
                    // Real, live-confirmed root cause (found chasing the
                    // ATA-subscription gap above): `Configuration::set`
                    // resolves `mint_usdc`/`mint_sol` from fixed
                    // constants, but was only ever called from
                    // `on_load` -- dead code on a fresh connection,
                    // since `o_rc_keypair` isn't set yet at `on_load`
                    // time. `self.configuration.mint_usdc` stayed `0`
                    // (an invalid `AccountId`) for the whole session,
                    // silently breaking `derive_ata`/`ata_subscribe_request`
                    // for USDC specifically (a `0` mint has no real
                    // reverse pubkey mapping, so it's dropped, not
                    // erred) and every real USDC balance/leg read
                    // downstream. Must run here, on the real connection
                    // path, not only on `on_load`'s reconnect-only path
                    // -- same fix `leveragedloopv1`'s own wallet-load
                    // handler already has.
                    self.configuration.set(&rc_keypair);
                    // Batched, not one-at-a-time: real, live-observed
                    // incident elsewhere in this codebase (see
                    // `kamino.rs`'s own doc comment) traced ~26s of stall
                    // to exactly this mistake -- one `subscribe_now` call
                    // covering this wallet's durable-nonce account, the
                    // pair-trade Kamino obligation's authority accounts
                    // (obligation PDA + user_metadata PDA), the pair-trade
                    // Solend obligation's authority account (its own
                    // `create_with_seed` address, `id=1`), and every ATA
                    // this mode's own real balance reads need.
                    let nonce_reqs: Vec<_> = self.wallet.nonce_subscribe_request(account_id).into_iter().collect();
                    let kamino_reqs = self
                        .state
                        .o_pair_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, PAIR_KAMINO_OBLIGATION_ID))
                        .unwrap_or_default();
                    let solend_reqs = self
                        .state
                        .o_pair_solend_position
                        .as_ref()
                        .map(|s| s.authority_subscribe_requests(pubkey, PAIR_SOLEND_OBLIGATION_ID))
                        .unwrap_or_default();
                    // Same batching for trade type 1's own, separate
                    // obligations -- see the batching note above for why
                    // this must never revert to a one-at-a-time subscribe.
                    let directional_kamino_reqs = self
                        .state
                        .o_directional_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, DIRECTIONAL_KAMINO_OBLIGATION_ID))
                        .unwrap_or_default();
                    let directional_solend_reqs = self
                        .state
                        .o_directional_solend_position
                        .as_ref()
                        .map(|s| s.authority_subscribe_requests(pubkey, DIRECTIONAL_SOLEND_OBLIGATION_ID))
                        .unwrap_or_default();
                    // Same batching for trade type 5's (Hawkes) own,
                    // separate obligations -- see the batching note above
                    // for why this must never revert to a one-at-a-time
                    // subscribe.
                    let hawkes_kamino_reqs = self
                        .state
                        .o_hawkes_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, HAWKES_KAMINO_OBLIGATION_ID))
                        .unwrap_or_default();
                    let hawkes_solend_reqs = self
                        .state
                        .o_hawkes_solend_position
                        .as_ref()
                        .map(|s| s.authority_subscribe_requests(pubkey, HAWKES_SOLEND_OBLIGATION_ID))
                        .unwrap_or_default();
                    // Trade type 3's (dispersion) own Phoenix trader
                    // account -- same batching discipline, one more real
                    // request folded into this same round-trip rather
                    // than a separate `set_authority` call.
                    let phoenix_reqs = self
                        .state
                        .o_phoenix
                        .as_ref()
                        .map(|p| p.authority_subscribe_requests(pubkey))
                        .unwrap_or_default();
                    // Real, live-confirmed gap (found running sub-phase
                    // 5c's own first real live test): without an
                    // explicit ATA subscription request per mint, this
                    // wallet's own SPL token accounts are never watched
                    // at all -- `current_usdc_value`/`open_pair_*_leg`'s
                    // own balance reads stayed at $0.00 forever (not a
                    // slow warm-up, a real subscription gap), even
                    // though the real on-chain balance was nonzero. Same
                    // fix `leveragedloopv1`'s own wallet-load handler
                    // already has: explicitly subscribe every mint this
                    // mode's own balance reads ever touch -- USDC plus
                    // every curated symbol -- at wallet-load time, not
                    // only once a specific candidate is chosen (an
                    // on-demand subscription only *registers*; the real
                    // balance arrives asynchronously later, so waiting
                    // until a candidate is picked would leave that first
                    // real decision blind to its own balance for however
                    // long the first update takes).
                    let mut ata_mints: HashSet<AccountId> = HashSet::new();
                    ata_mints.insert(self.configuration.mint_usdc);
                    for (_, mint, _) in curated_symbols() {
                        ata_mints.insert(mint);
                    }
                    let ata_reqs: Vec<_> =
                        ata_mints.into_iter().filter_map(|mint| self.wallet.ata_subscribe_request(account_id, mint)).collect();
                    let nonce_len = nonce_reqs.len();
                    let kamino_len = kamino_reqs.len();
                    let solend_len = solend_reqs.len();
                    let directional_kamino_len = directional_kamino_reqs.len();
                    let directional_solend_len = directional_solend_reqs.len();
                    let hawkes_kamino_len = hawkes_kamino_reqs.len();
                    let hawkes_solend_len = hawkes_solend_reqs.len();
                    let phoenix_len = phoenix_reqs.len();
                    let ata_len = ata_reqs.len();
                    let mut all_requests = Vec::with_capacity(
                        nonce_len
                            + kamino_len
                            + solend_len
                            + directional_kamino_len
                            + directional_solend_len
                            + hawkes_kamino_len
                            + hawkes_solend_len
                            + phoenix_len
                            + ata_len,
                    );
                    all_requests.extend(nonce_reqs);
                    all_requests.extend(kamino_reqs);
                    all_requests.extend(solend_reqs);
                    all_requests.extend(directional_kamino_reqs);
                    all_requests.extend(directional_solend_reqs);
                    all_requests.extend(hawkes_kamino_reqs);
                    all_requests.extend(hawkes_solend_reqs);
                    all_requests.extend(phoenix_reqs);
                    all_requests.extend(ata_reqs);
                    log_warn!(
                        "multimodelv1: CustomMessageInbound::Wallet -- about to subscribe_now ({} requests: nonce={nonce_len} kamino={kamino_len} solend={solend_len} dir_kamino={directional_kamino_len} dir_solend={directional_solend_len} hawkes_kamino={hawkes_kamino_len} hawkes_solend={hawkes_solend_len} phoenix={phoenix_len} ata={ata_len})",
                        nonce_len + kamino_len + solend_len + directional_kamino_len + directional_solend_len + hawkes_kamino_len + hawkes_solend_len + phoenix_len + ata_len,
                    );
                    match SubscriptionQueue::subscribe_now(self.graph, all_requests) {
                        Ok(subs) => {
                            log_warn!("multimodelv1: CustomMessageInbound::Wallet -- subscribe_now returned Ok ({} subs)", subs.len());
                            let mut it = subs.into_iter();
                            if nonce_len > 0 {
                                if let Some(sub) = it.next() {
                                    self.wallet.keep_nonce_subscription(sub);
                                }
                            }
                            if kamino_len > 0 {
                                let take: Vec<_> = (&mut it).take(kamino_len).collect();
                                if let Some(pos) = self.state.o_pair_kamino_position.as_mut() {
                                    pos.apply_authority(pubkey, PAIR_KAMINO_OBLIGATION_ID, take);
                                }
                            }
                            if solend_len > 0 {
                                let take: Vec<_> = (&mut it).take(solend_len).collect();
                                if let Some(pos) = self.state.o_pair_solend_position.as_mut() {
                                    pos.apply_authority(pubkey, PAIR_SOLEND_OBLIGATION_ID, take);
                                }
                            }
                            if directional_kamino_len > 0 {
                                let take: Vec<_> = (&mut it).take(directional_kamino_len).collect();
                                if let Some(pos) = self.state.o_directional_kamino_position.as_mut() {
                                    pos.apply_authority(pubkey, DIRECTIONAL_KAMINO_OBLIGATION_ID, take);
                                }
                            }
                            if directional_solend_len > 0 {
                                let take: Vec<_> = (&mut it).take(directional_solend_len).collect();
                                if let Some(pos) = self.state.o_directional_solend_position.as_mut() {
                                    pos.apply_authority(pubkey, DIRECTIONAL_SOLEND_OBLIGATION_ID, take);
                                }
                            }
                            log_warn!("multimodelv1: CustomMessageInbound::Wallet -- before hawkes apply_authority (kamino_len={hawkes_kamino_len} solend_len={hawkes_solend_len})");
                            if hawkes_kamino_len > 0 {
                                let take: Vec<_> = (&mut it).take(hawkes_kamino_len).collect();
                                if let Some(pos) = self.state.o_hawkes_kamino_position.as_mut() {
                                    pos.apply_authority(pubkey, HAWKES_KAMINO_OBLIGATION_ID, take);
                                }
                            }
                            if hawkes_solend_len > 0 {
                                let take: Vec<_> = (&mut it).take(hawkes_solend_len).collect();
                                if let Some(pos) = self.state.o_hawkes_solend_position.as_mut() {
                                    pos.apply_authority(pubkey, HAWKES_SOLEND_OBLIGATION_ID, take);
                                }
                            }
                            log_warn!("multimodelv1: CustomMessageInbound::Wallet -- after hawkes apply_authority");
                            if phoenix_len > 0 {
                                let take: Vec<_> = (&mut it).take(phoenix_len).collect();
                                if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                                    phoenix.apply_authority(pubkey, take);
                                }
                            }
                            let ata_subs: Vec<_> = (&mut it).take(ata_len).collect();
                            self.wallet.keep_ata_subscriptions(ata_subs);
                        }
                        Err(e) => {
                            log_error!("multimodelv1: failed to batch-subscribe wallet authority accounts: {e}");
                        }
                    }
                    self.state.o_rc_keypair.replace(KeypairExtra { rc_keypair, account_id });
                    log_warn!("multimodelv1: CustomMessageInbound::Wallet -- exited");
                }
                CustomMessageInbound::ReplayOpenIntent(_intent) => {
                    // Phase 4 point 3's inbound half -- wire this into
                    // `trader::factor_intent::reconcile_intent` once
                    // there's real trade state to reconcile it against
                    // (Phase 5 point 2 onward). Deliberately inert for
                    // now, not dropped: receiving this before there's
                    // anything to do with it is expected, not an error.
                }
                CustomMessageInbound::CommonBundlerTipUpdate(update) => {
                    self.wallet.apply_bundler_tip_update(self.graph, update);
                }
                CustomMessageInbound::TriggerEnableFactorLogging => {
                    self.state.factor_logging_enabled = true;
                    log_warn!("multimodelv1: TriggerEnableFactorLogging -- real factor resync/logging enabled");
                }
                CustomMessageInbound::TriggerEnablePairTrading => {
                    self.state.pair_trading_enabled = true;
                    log_warn!("multimodelv1: TriggerEnablePairTrading -- REAL pair-trade decisions/execution enabled");
                }
                CustomMessageInbound::ReplayResidualSnapshot(snapshot) => {
                    self.apply_residual_snapshot(snapshot);
                }
                CustomMessageInbound::TriggerEnableDirectionalTrading { mint } => {
                    self.state.directional_trading_enabled = true;
                    let target_mint = account_id_from_pubkey(&Pubkey::new_from_array(mint));
                    if self.current_open_directional().is_some() {
                        log_warn!(
                            "multimodelv1: TriggerEnableDirectionalTrading -- already have an open directional position, ignoring new target {target_mint}"
                        );
                    } else {
                        self.state.o_directional_target_mint = Some(target_mint);
                        log_warn!(
                            "multimodelv1: TriggerEnableDirectionalTrading -- REAL directional-neutral trading enabled, target {target_mint}"
                        );
                    }
                }
                CustomMessageInbound::TriggerCloseDirectionalPosition => {
                    self.state.directional_trading_enabled = true;
                    self.state.o_directional_close_requested = true;
                    log_warn!("multimodelv1: TriggerCloseDirectionalPosition -- close requested");
                }
                CustomMessageInbound::TriggerEnableDispersionTrading => {
                    self.state.dispersion_trading_enabled = true;
                    log_warn!("multimodelv1: TriggerEnableDispersionTrading -- REAL dispersion trading armed (automated entry)");
                }
                CustomMessageInbound::TriggerCloseDispersionPosition => {
                    self.state.dispersion_trading_enabled = true;
                    self.state.o_dispersion_close_requested = true;
                    log_warn!("multimodelv1: TriggerCloseDispersionPosition -- close requested");
                }
                CustomMessageInbound::TriggerEnableHawkesTrading => {
                    self.state.hawkes_trading_enabled = true;
                    log_warn!("multimodelv1: TriggerEnableHawkesTrading -- REAL Hawkes-on-eigenfactor momentum trading armed (automated entry)");
                }
                CustomMessageInbound::TriggerCloseHawkesPosition => {
                    self.state.hawkes_trading_enabled = true;
                    self.state.o_hawkes_close_requested = true;
                    log_warn!("multimodelv1: TriggerCloseHawkesPosition -- close requested");
                }
                CustomMessageInbound::TriggerSweepMint { mint, dest_mint } => {
                    let target_mint = account_id_from_pubkey(&Pubkey::new_from_array(mint));
                    let target_dest = account_id_from_pubkey(&Pubkey::new_from_array(dest_mint));
                    self.state.o_sweep_mint_requested = Some((target_mint, target_dest));
                    log_warn!("multimodelv1: TriggerSweepMint -- sweep requested for mint {target_mint} -> {target_dest}");
                }
            },
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(input_mint: AccountId, output_mint: AccountId) -> pricegraph::Hop {
        pricegraph::Hop {
            pool_id: 1,
            input_mint,
            output_mint,
            amount_in: 1,
            amount_out: 1,
            dex: crate::trader::types::DexType::OrcaWhirlpool,
        }
    }

    fn chain(hops: Vec<pricegraph::Hop>, produced_mint: AccountId) -> PendingHopChain {
        PendingHopChain {
            hops,
            owner: 0,
            sent_pool_id: 0,
            consumed_mint: 0,
            produced_mint,
            sent_slot: 0,
            produced_mint_balance_before: 0,
        }
    }

    #[test]
    fn final_target_mint_is_produced_mint_when_no_hops_remain() {
        // The just-sent hop was the route's last one -- produced_mint is
        // already the real, final destination.
        assert_eq!(chain(vec![], 42).final_target_mint(), 42);
    }

    #[test]
    fn final_target_mint_is_the_last_remaining_hops_output_when_hops_remain() {
        // produced_mint (99) is only the *next* hop's intermediate
        // output here -- the real final target is wherever the route's
        // last remaining hop actually lands.
        let c = chain(vec![hop(99, 7), hop(7, 55)], 99);
        assert_eq!(c.final_target_mint(), 55);
    }

    #[test]
    fn final_target_mint_single_remaining_hop() {
        let c = chain(vec![hop(99, 123)], 99);
        assert_eq!(c.final_target_mint(), 123);
    }
}
