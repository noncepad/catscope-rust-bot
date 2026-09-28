//! Reactive loop for `leveragedloopv1` -- Phase 2 of
//! `leveraged_yield_farming_plan.md`: deposit jitoSOL as Kamino
//! collateral, borrow USDC against it, swap the borrow back to jitoSOL
//! and redeposit once (a single loop step, 1.3x leverage target), then
//! hold until a real `TriggerClose` unwinds it (withdraw the excess
//! collateral, swap it to USDC, repay the debt, withdraw the rest).
//! Manual/explicit trigger only -- see [`LoopPhase`]'s doc comment for
//! the full state machine, and `message::CustomMessageInbound` for the
//! two trigger messages.
//!
//! Mirrors `testperpv1::state`'s shape (same `StateHelper`/`CommitHook`/
//! `evaluate` pattern, same Kamino bootstrap/farm-ready/refresh helpers
//! -- those are Kamino-generic, not basis-trade-specific, so they're
//! copied rather than shared per this codebase's established convention
//! of small per-mode boilerplate over cross-module sharing), trimmed to
//! only what this mode needs: no Phoenix, no Solend, no marginfi, no
//! `PerpRouter`, no target allocation. Deliberately a **separate bot
//! mode and obligation** from testperpv1 -- see this module's doc
//! comment (`mod.rs`) for why.
use crate::{
    brain::leveragedloopv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate, SubscriptionQueue},
    log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    kamino_config, marginfi_config, orca_config, phoenix_config, pumpfun_config, pumpswap_config,
    raydium_amm_config, raydium_clmm_config, raydium_cpmm_config, router_config,
    router_pools_config, sanctum_config, solend_config, symbol_mint_config, top_pools_config,
    tracked_accounts_config, trading_config, atl_config,
    trader::{
        credit,
        derivative_router,
        dex::{
            ember,
            kamino,
            phoenix::{ix::Side, PhoenixState},
            update::Updater as _,
            DexState,
        },
        perp_router::{PerpRouter, PerpVenue},
        planner,
        pricegraph::TradeRouter,
        router,
        timegraph,
    },
    txview::TransactionList,
    util::{account_id_from_pubkey, pubkey_from_account_id, rc_unlock, resolve_symbol_mint},
    wallet::{PriorityLevel, Wallet},
};
use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

/// jitoSOL -- the LST Phase 2 targets (highest real Kamino TVL/liquidity
/// of the three named in the plan; user's own choice, 2026-08-27).
/// `loop_deposit_collateral`/`loop_borrow_and_redeposit`/deleverage still
/// execute against this single mint -- see [`LST_CANDIDATES`]'s doc
/// comment for the generalization boundary.
const JITOSOL_MINT: Pubkey = Pubkey::from_str_const("J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn");

/// Real LST candidates this bot mode's Time-Expanded DAG
/// (`src/trader/TIME.md`) evaluates -- every real Sanctum-tracked LST
/// (`sanctum_lst` table) with a real lending-protocol reserve on Kamino,
/// Solend, or marginfi's main markets, found by cross-referencing the
/// live `prefetch.db` (2026-08-29): 37 of Sanctum's 128 tracked LSTs.
/// Expanded from an original hand-picked 3 (jitoSOL/bSOL/mSOL, the only
/// ones with runtime yield data at the time) once
/// `optimizer/prefetch/lst-yield` switched to a generic, Sanctum-based
/// rate source (`FetchRates`/`TrackedLSTs`) covering all 37 instead of 3
/// hardcoded per-protocol decoders -- see that package's doc comment.
///
/// The label is a real, confidently-known friendly name for the handful
/// this codebase is sure about (jitoSOL/bSOL/mSOL/wSOL/jupSOL/bonkSOL --
/// vanity mint prefixes that unambiguously match well-known real LSTs),
/// otherwise the first 8 base58 characters of the LST's own real mint
/// address -- accurate and non-misleading (it's literally part of the
/// real mint, not a guessed name), and, critically, still **unique**:
/// this label doubles as `timegraph::Node`'s `token` identity
/// (`&'static str`, compared by value, not by pointer) in
/// `dag_best_lst_path` below, so two different candidates sharing a
/// label would silently merge into the same DAG node.
///
/// Only `dag_best_lst_path`'s read-only-by-default projection and
/// `TriggerOpenAuto`'s real decision (see that variant's doc comment) use
/// this table -- `TriggerOpen`'s own original, real-money-proven behavior
/// (always jitoSOL) is completely unchanged. Kamino-only for now, not
/// cross-protocol (Solend/marginfi) -- this bot mode has never subscribed
/// to Solend/marginfi reserve accounts (see `build_time_pubkeys`' own doc
/// comment); most of these 37 have no Kamino reserve at all (only 26 of
/// the 37 do -- some only have a Solend or marginfi reserve instead), so
/// `kamino_reserve_for_lst` returning `None` for those is the expected,
/// common case, not an error -- `dag_best_lst_path`'s loop just skips
/// them (`continue`), same as a candidate with no `LstApy` data yet.
const LST_CANDIDATES: &[(&str, Pubkey)] = &[
    ("BNso1VUJ", Pubkey::from_str_const("BNso1VUJnh4zcfpZa6986Ea66P6TCp59hvtNJ8b1X85")),
    ("bonkSOL", Pubkey::from_str_const("BonK1YhkXEGLZzwtcvRTip3gAL9nCeQD7ppZBLXhtTs")),
    ("Dso1bDeD", Pubkey::from_str_const("Dso1bDeDjCQxTrWHqUUi63oBvV7Mdm6WaobLbQ7gnPQ")),
    ("LAinEtNL", Pubkey::from_str_const("LAinEtNLgpmCP9Rvsf5Hn8W6EhNiKLZQti1xfWMLy6X")),
    ("LSTxxxnJ", Pubkey::from_str_const("LSTxxxnJzKDFSLr4dUkPcmCf5VyryEqzPLz5j4bpxFp")),
    ("LnTRntk2", Pubkey::from_str_const("LnTRntk2kTfWEY6cVB8K9649pgJbt6dJLS1Ns1GZCWg")),
    ("wSOL", Pubkey::from_str_const("So11111111111111111111111111111111111111112")),
    ("bSOL", Pubkey::from_str_const("bSo13r4TkiE4KumL71LsHTPpL2euBYLFx6h9HP3piy1")),
    ("cPQPBN7W", Pubkey::from_str_const("cPQPBN7WubB3zyQDpzTK2ormx1BMdAym9xkrYUJsctm")),
    ("haSo1Vz5", Pubkey::from_str_const("haSo1Vz5aTsqEnz8nisfnEsipvbAAWpgzRDh2WhhMEh")),
    ("he1iusmf", Pubkey::from_str_const("he1iusmfkpAdwvxLNGV8Y1iSbj4rUy6yMhEA3fotn9A")),
    ("hy1oXYgr", Pubkey::from_str_const("hy1oXYgrBW6PVcJ4s6s2FKavRdwgWTXdfE69AxT7kPT")),
    ("jucy5XJ7", Pubkey::from_str_const("jucy5XJ76pHVvtPZb5TKRcGQExkwit2P5s4vY8UzmpC")),
    ("jupSOL", Pubkey::from_str_const("jupSoLaHXQiZZTSfEWMTRRgpnyFm8f6sZdosWBjx93v")),
    ("mSOL", Pubkey::from_str_const("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So")),
    ("pSo1f9nQ", Pubkey::from_str_const("pSo1f9nQXWgXibFtKf7NWYxb5enAM4qfP6UJSiXRQfL")),
    ("phaseZSf", Pubkey::from_str_const("phaseZSfPxTDBpiVb96H4XFSD8xHeHxZre5HerehBJG")),
    ("picobAEv", Pubkey::from_str_const("picobAEvs6w7QEknPce34wAE4gknZA9v5tTonnmHYdX")),
    ("rkubjTrZ", Pubkey::from_str_const("rkubjTrZYioRSeXwDnhwGQzvW3qkcin72JSxUt3WMVp")),
    ("sctmB7GP", Pubkey::from_str_const("sctmB7GPi5L2Q5G9tUSzXvhZ4YiDMEGcRov9KfArQpx")),
    ("sctmTAsD", Pubkey::from_str_const("sctmTAsDn4tLUcemqoqYijfuRkiEfAMPi84PNq2EueR")),
    ("sctmY8fJ", Pubkey::from_str_const("sctmY8fJucsJatwHz6P48RuWBBkdBMNmSMuBYrWFdrw")),
    ("stke7uu3", Pubkey::from_str_const("stke7uu3fXHsGqKVVjKnkmj65LRPVrqr4bLG2SJg7rh")),
    ("strng7mq", Pubkey::from_str_const("strng7mqqc1MBJJV6vMzYbEqnwVGvKKGKedeCvtktWA")),
    ("vSoLxydx", Pubkey::from_str_const("vSoLxydx6akxyMD9XEcPvGYNGq6Nn66oqVb3UkGkei7")),
    ("7Q2afV64", Pubkey::from_str_const("7Q2afV64in6N6SeZsAAB81TJzwDoD6zpqmHkzi9Dcavn")),
    ("7dHbWXmc", Pubkey::from_str_const("7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj")),
    ("BULKoNSG", Pubkey::from_str_const("BULKoNSGzxtCqzwTvg5hFJg8fx6dqZRScyXe5LYMfxrn")),
    ("Bybit2vB", Pubkey::from_str_const("Bybit2vBJGhPF52GBdNaQfUJ6ZpThSgHBobjWZpLPb4B")),
    ("CDCSoLck", Pubkey::from_str_const("CDCSoLckzozyktpAp9FWT3w92KFJVEUxAU7cNu2Jn3aX")),
    ("CgnTSoL3", Pubkey::from_str_const("CgnTSoL3DgY9SFHxcLj6CgCgKKoTBr6tp4CPAEWy25DE")),
    ("Comp4ssD", Pubkey::from_str_const("Comp4ssDzXcLeu2MnLuGNNFC4cmLPMng8qWHPvzAMU1h")),
    ("CorvuSSo", Pubkey::from_str_const("CorvuSSoLxPKLoXWXSfn8pFSMhCRHhe7Uwqe874cmwvg")),
    ("EPCz5LK3", Pubkey::from_str_const("EPCz5LK372vmvCkZH3HgSuGNKACJJwwxsofW6fypCPZL")),
    ("Gekfj7SL", Pubkey::from_str_const("Gekfj7SL2fVpTDxJZmeC46cTYxinjB6gkAnb6EGT6mnn")),
    ("HUBsveNp", Pubkey::from_str_const("HUBsveNpjo5pWqNkH57QzxjQASdTVXcSK7bVKTSZtcSX")),
    ("jitoSOL", JITOSOL_MINT),
];

/// Result of `StateHelper::dag_best_lst_path` -- see that function's doc
/// comment for exactly what it means and its known simplifications.
struct DagLstDecision {
    /// The winning path's second node's token -- `"USDC"` means "do
    /// nothing" won; any `LST_CANDIDATES` symbol means that LST's
    /// unlevered loop beat both "do nothing" and every other candidate.
    winner: &'static str,
    profit_ratio: f64,
    usdc_borrow_apy: f64,
    /// Every candidate that had enough real data to be considered at all
    /// (not necessarily the winner) -- for logging/diagnostics.
    candidates_used: Vec<&'static str>,
}

impl DagLstDecision {
    /// Holding-period the DAG projects over -- see `dag_best_lst_path`'s
    /// doc comment. A year is a reasonable default horizon for an APY
    /// comparison; this isn't tied to how long `TriggerOpenAuto` will
    /// actually leave a position open (it doesn't auto-close on any
    /// schedule).
    const HORIZON_YEARS: f64 = 1.0;
    /// Nominal round-trip swap-fee approximation -- see
    /// `dag_best_lst_path`'s doc comment on why this isn't a live
    /// `TradeRouter` quote yet.
    const NOMINAL_SWAP_RATE: f64 = 0.997;
}

/// Borrow 30% of deposited collateral's USD value -- `leverage = 1 +
/// borrow_fraction`, so 0.30 targets the user's chosen 1.3x. See the
/// plan's "Phase 2 concrete design" for the derivation.
const TARGET_BORROW_FRACTION: f64 = 0.30;
/// Hard ceiling, independent of the target above: never borrow more than
/// this fraction of jitoSOL's real max LTV (50%, user's own choice).
/// Protects against the target drifting past the ceiling if jitoSOL's
/// real LTV ever changes; currently non-binding (30% < 0.5*63%=31.5%).
const LTV_SAFETY_CEILING_OF_MAX_LTV: f64 = 0.5;
/// Same over-collateralization margin `open_kamino_borrow_leg`
/// (testperpv1/perpfundingv1) already established against a real
/// `BorrowTooLarge` revert -- applies on top of the ceiling above, not
/// instead of it.
const LTV_SAFETY_FACTOR: f64 = 0.9;
/// Small buffer added when sizing the withdraw-for-repay amount during
/// deleverage, so a real price tick between "decide how much to
/// withdraw" and "the withdraw+swap lands" doesn't leave the repay short
/// by a few raw units.
const DELEVERAGE_WITHDRAW_BUFFER: f64 = 1.02;
/// Same cooldown testperpv1's phase machine uses -- generous enough for
/// a real transaction to land and confirm, short enough not to stall a
/// multi-step sequence for long. `evaluate()` fires on every event
/// (many times/second), so this is what stops it from spamming
/// duplicate transactions before a previous one has had a chance to
/// confirm.
const LOOP_ACTION_COOLDOWN_SLOTS: Slot = 100;
/// How often `log_dag_lst_projection` retries/re-logs, from `finish()`
/// (called every tick, same as `ensure_bundler_nonce_created`) -- real,
/// live-confirmed gap (2026-08-28): calling the projection only
/// reactively from the `LstApy` message handler meant it silently
/// no-oped forever whenever real Kamino reserve data hadn't streamed in
/// yet at that exact moment (confirmed live: `LstApy` arrived at 7.3s
/// into a fresh connection, well before Kamino's own commit-stream data
/// had caught up, and nothing ever retried). ~200 slots (~80s) is
/// deliberately longer than `LOOP_ACTION_COOLDOWN_SLOTS` -- this is a
/// status log, not a real-money retry, no need for it to be as tight.
const DAG_PROJECTION_LOG_COOLDOWN_SLOTS: Slot = 200;
/// `max_hops` for the loop's own USDC<->jitoSOL legs -- see
/// `execute_spot_leg`'s doc comment. Temporarily raised 2 -> 5
/// (2026-08-28, user-requested) to real-test the new
/// `Wallet::drain_and_send`/`transactionprocessor::batch` bundling path:
/// a 5-hop route still lands as one atomic group (fits-or-rejected, per
/// `atomic_group_fits`) rather than itself being split across a bundle,
/// but real hop counts this high are more likely to accumulate multiple
/// real transactions in one `evaluate()` tick (the leading ATA-creation
/// instruction landing separately from the swap's own atomic group, or
/// multiple independent legs in the same tick) than `LOOP_MAX_HOPS=2`
/// ever did. Revert to 2 once bundling is confirmed working live -- see
/// [[project_leveragedloopv1]] for the real tx-size-vs-hop-count
/// tradeoff this reverts back into.
const LOOP_MAX_HOPS: usize = 5;
/// `max_hops` for `check_pending_recover`'s one-off recovery swaps --
/// see `execute_spot_leg`'s doc comment.
const RECOVER_MAX_HOPS: usize = 4;
/// Basis-trade strategy (2026-08-29): assumed capital needed to margin
/// one funding-rate cycle (Phoenix leg + Kamino leg combined) -- same
/// deliberately simple, tunable placeholder `perpfundingv1`'s own
/// `FUNDING_CYCLE_MIN_MARGIN_USD` is (not derived from real per-venue
/// margin requirements -- see that constant's doc comment). Named
/// differently to avoid confusion with this file's own
/// `TARGET_BORROW_FRACTION`-style leverage-loop constants, which this is
/// unrelated to.
const BASIS_CYCLE_MIN_MARGIN_USD: f64 = 10.0;
/// Real Phoenix funding-epoch length -- both venues settle hourly (see
/// `trader::perp_router`'s own module doc for the live-verified
/// derivation), same constant `perpfundingv1::state` uses.
const SECONDS_PER_EPOCH: i64 = 3600;

/// Builds the 3-tier build-time liquidity router from `router_pools_config`
/// -- copied verbatim from `testperpv1::state::build_liquidity_router`
/// rather than shared, matching this codebase's established convention.
/// Only used to seed `State::spot_router`'s node set once, in
/// `StateHelper::on_load`.
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

/// Real curated symbol/mint pairs the basis trade evaluates -- same 6
/// entries `perpfundingv1`'s own basis-trade strategy uses
/// (`symbol_mint_config::SYMBOL_MINT_MAP`: SOL/BTC/ETH/XRP/BNB/SUI).
/// `'static` lifetime is real, not asserted -- both `symbol` and `mint`
/// are borrowed from/derived from the build-time-baked `'static`
/// `SYMBOL_MINT_MAP` table itself.
fn basis_symbols() -> impl Iterator<Item = (&'static str, AccountId)> {
    symbol_mint_config::SYMBOL_MINT_MAP.iter().filter_map(|raw| {
        let symbol = std::str::from_utf8(&raw.symbol).ok()?.trim_end_matches('\0');
        let mint = account_id_from_pubkey(&Pubkey::new_from_array(raw.mint));
        Some((symbol, mint))
    })
}

#[derive(Debug)]
struct KeypairExtra {
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}

/// The real state machine this bot mode runs. Every transition either
/// rechecks real on-chain state first (advancing only once confirmed --
/// never assumed from having just sent an instruction) or is driven by
/// an explicit `TriggerOpen`/`TriggerClose` message; nothing here fires
/// automatically without one of those two triggers having arrived at
/// some point.
///
/// ```text
/// Idle/Closed --TriggerOpen(notional_usd)--> DepositCollateral
///   DepositCollateral: swap USDC->jitoSOL (notional_usd), deposit as
///     Kamino collateral. Advances once a real deposit is confirmed.
///   -> BorrowAndRedeposit: borrow USDC against the jitoSOL collateral
///     (sized at TARGET_BORROW_FRACTION, capped by
///     LTV_SAFETY_CEILING_OF_MAX_LTV), then immediately swap the
///     borrowed USDC back to jitoSOL and deposit again -- both batched
///     into the same instruction sequence (same "estimate now, on-chain
///     safety net" precedent testperpv1's own Kamino legs already use;
///     see this function's own doc comment). Advances once the borrow
///     itself is confirmed.
///   -> Open: holding. Waits for a real TriggerClose.
/// Open --TriggerClose--> DeleverageWithdrawAndRepay
///   DeleverageWithdrawAndRepay: withdraws just enough jitoSOL collateral
///     to cover the outstanding USDC debt (real math, never touches
///     capital beyond what's needed), swaps it to USDC, repays in full.
///     Advances once the debt is confirmed gone.
///   -> DeleverageWithdrawRest: withdraws all remaining jitoSOL
///     collateral (now debt-free) -- left as jitoSOL, not swapped back,
///     since either is safe once debt is zero. Advances once the
///     deposit is confirmed gone.
///   -> Closed. Accepts a fresh TriggerOpen just like Idle.
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LoopPhase {
    #[default]
    Idle,
    DepositCollateral,
    BorrowAndRedeposit,
    Open,
    DeleverageWithdrawAndRepay,
    DeleverageWithdrawRest,
    Closed,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    last_slot: Slot,
    subscription_queue: SubscriptionQueue,
    o_rc_keypair: Option<KeypairExtra>,
    /// This bot's own Kamino lending position -- the only lending
    /// protocol Phase 2 uses (jitoSOL/USDC collateral+debt).
    o_kamino_position: Option<kamino::KaminoPosition>,
    /// Spot-market execution hook -- see `StateHelper::execute_spot_leg`.
    /// `None` until `on_load`.
    o_dex: Option<DexState>,
    /// Bellman-Ford spot price graph, fed incrementally by
    /// `low_latency`/`CommitHook::on_account`, same role as every other
    /// bot mode's own `spot_router`/`router` field.
    spot_router: TradeRouter,
    loop_phase: LoopPhase,
    /// The slot the current phase's last action was sent at, if any --
    /// gates retries so `evaluate()` (called on every event) doesn't
    /// resend the same instruction before a previous one has had a real
    /// chance to confirm. Reset to `None` on every phase transition.
    loop_last_action_slot: Option<Slot>,
    /// Notional USD requested by the `TriggerOpen` that started the
    /// current open sequence -- consumed by `loop_deposit_collateral`,
    /// not read again afterward (borrow/redeposit sizing comes from the
    /// real deposited collateral value, not this original request).
    requested_notional_usd: Option<f64>,
    /// Pool IDs of the most recently queued `execute_spot_leg` route,
    /// cleared once `evaluate()`'s send loop has run -- lets that loop
    /// mark every pool in the route on cooldown if
    /// `transactionprocessor::send()` itself rejects the transaction
    /// (any reason, not just the oversized-route case
    /// `atomic_group_fits` already catches earlier). Real motivation
    /// (2026-08-27): send()-time rejection is the only *real* failure
    /// signal this bot mode can act on today -- it never reads back
    /// asynchronous on-chain confirmation results at all (see
    /// `execute_spot_leg`'s own doc comment on the separate, still-open
    /// gap that leaves: a transaction that sends fine but reverts
    /// on-chain, e.g. a real Raydium CLMM `BorrowError` panic hit twice
    /// live on pool 492876181, currently has no cooldown feedback path).
    pending_route_pools: Vec<AccountId>,
    /// Set by a `TriggerRecoverToken(mint)` message, cleared once
    /// `evaluate()`'s `check_pending_recover` actually acts on it (finds
    /// a real nonzero balance and attempts the swap) -- real, live-
    /// confirmed reason this can't just check the balance immediately in
    /// `on_message`: subscribing to a mint on demand
    /// (`Wallet::ata_subscribe_request` + `SubscriptionQueue::
    /// subscribe_now`) only registers the subscription -- the real
    /// account data arrives *asynchronously* later via the normal
    /// commit-event stream, same as every other subscription in this
    /// codebase, so a balance check in the same synchronous call always
    /// saw zero. Checked on every `evaluate()` tick instead, so it picks
    /// up the real balance whenever the subscription actually delivers
    /// it.
    o_pending_recover_mint: Option<Pubkey>,
    /// Cooldown gate for `check_pending_recover`'s own retries, separate
    /// from `loop_last_action_slot` (the loop's own phase machine) since
    /// recovery is deliberately independent of `LoopPhase`. Real,
    /// live-confirmed reason a retry needed a gate at all, not just
    /// "clear on first attempt regardless of outcome": a real recovery
    /// swap can fail pre-flight (e.g. `PoolNotReady`) with zero funds at
    /// risk, and the router needs its own cooldown on the bad pool to
    /// actually take effect before the next attempt, or it just
    /// reselects the same doomed route every tick.
    recover_last_action_slot: Option<Slot>,
    /// Set by a `TriggerRedepositUsdc(notional_usd)` message (raw USDC
    /// amount, already converted), cleared once
    /// `check_pending_redeposit` succeeds. Retried (cooldown-gated via
    /// `redeposit_last_action_slot`) rather than a single attempt --
    /// same real, live-confirmed reasoning as `o_pending_recover_mint`:
    /// a pre-flight route failure is safe but needs the router's own
    /// cooldown on the bad pool to actually take effect before a retry
    /// picks something else.
    o_pending_redeposit_usdc_raw: Option<u64>,
    /// Cooldown gate for `check_pending_redeposit`'s own retries,
    /// separate from `loop_last_action_slot`/`recover_last_action_slot`.
    redeposit_last_action_slot: Option<Slot>,
    /// Real annualized SOL-per-LST exchange-rate growth per real mint
    /// (e.g. jitoSOL's mint -> `0.073`), pushed periodically by
    /// `CustomMessageInbound::LstApy` -- see that variant's doc comment.
    /// Mint-keyed, not symbol-keyed (changed 2026-08-29 alongside
    /// `LstApy`'s own wire-format change -- see that variant's doc
    /// comment for why); unlike `testperpv1::State::lst_staking_apy`
    /// (still symbol-keyed, a different, untouched module). Refreshed in
    /// place on receipt, not accumulated.
    lst_staking_apy: HashMap<Pubkey, f64>,
    /// Cooldown gate for `log_dag_lst_projection`'s periodic retry from
    /// `finish()` -- see `DAG_PROJECTION_LOG_COOLDOWN_SLOTS`'s doc
    /// comment for why a retry is needed at all, not just the reactive
    /// call from the `LstApy` handler.
    dag_projection_last_log_slot: Option<Slot>,
    /// The LST candidate (symbol, mint) the *current* loop position
    /// actually targets -- `None` means "use the default" (jitoSOL,
    /// this mode's original single-LST target, unchanged since Phase 2).
    /// Set once by `TriggerOpenAuto`'s DAG decision (see that variant's
    /// doc comment) right before it starts the loop, exactly like
    /// `TriggerOpen` already does with `requested_notional_usd`. Every
    /// phase-handler function (`loop_deposit_collateral`,
    /// `loop_borrow_and_redeposit`, the deleverage functions) reads this
    /// via `State::active_lst()` instead of referencing `JITOSOL_MINT`
    /// directly now -- their own local variable is still named
    /// `jitosol_mint` throughout (not renamed, to keep this a minimal,
    /// low-risk change to already real-money-proven code) even though it
    /// may now hold a different LST's mint. `TriggerOpen`'s own,
    /// original real-money-proven behavior is completely unchanged
    /// unless `TriggerOpenAuto` is the one that actually opened the
    /// position.
    active_lst: Option<(&'static str, Pubkey)>,
    /// Set by a `TriggerOpenAuto(notional_usd)` message, cleared once
    /// `check_pending_open_auto` actually resolves it (opens for real,
    /// declines, or the loop phase changes out from under it). Real,
    /// live-confirmed reason a retry is needed at all (2026-08-28, same
    /// class of gap as `o_pending_recover_mint`/
    /// `o_pending_redeposit_usdc_raw`): `TriggerOpenAuto` used to decide
    /// inline, once, in `on_message` -- if it arrived before real Kamino
    /// reserve data and `LstApy` updates had both loaded (a real ~80-90s
    /// window observed live), `dag_best_lst_path` had nothing to work
    /// with and the trigger silently declined for the wrong reason ("no
    /// data yet") instead of ever getting a real answer. Checked on
    /// every `evaluate()` tick instead, so it retries until real data is
    /// actually available.
    o_pending_open_auto: Option<f64>,
    /// Cooldown gate for `check_pending_open_auto`'s own retries,
    /// separate from `loop_last_action_slot`/`recover_last_action_slot`/
    /// `redeposit_last_action_slot`.
    open_auto_last_action_slot: Option<Slot>,
    /// Real Phoenix-perp-funding-vs-Kamino-rate basis-trade strategy
    /// (2026-08-29), second and fully independent of the jitoSOL leverage
    /// loop above -- see `run_basis_cycle`'s doc comment. Own `PhoenixState`
    /// instance (needs a wallet authority for the full trader/margin
    /// lifecycle -- registration, deposits, positions -- same
    /// "intentionally independent, not shared" reasoning
    /// `trader::dex::phoenix::mod`'s own doc comment gives for why this
    /// can't just be `o_dex`'s shared read-only instance), mirroring
    /// `perpfundingv1::State`'s own `o_phoenix` field exactly.
    o_phoenix: Option<PhoenixState>,
    /// This basis trade's own, fully independent Kamino obligation --
    /// `id=1` (see `kamino::obligation_pda`'s doc comment), isolated from
    /// the jitoSOL leverage loop's own `id=0` obligation
    /// (`o_kamino_position` above) so the two strategies' deposits/
    /// borrows never mix in one obligation.
    o_basis_kamino_position: Option<kamino::KaminoPosition>,
    /// Real Phoenix funding-rate capture, feeding
    /// `derivative_router::find_best_funding_opportunities` -- see
    /// `run_basis_cycle`'s doc comment.
    router: PerpRouter,
    /// Set once by a `TriggerEnableBasisTrading` message -- gates whether
    /// `evaluate()` ever calls `run_basis_cycle` at all. `false` (the
    /// default) means this bot behaves exactly as it did before this
    /// strategy existed -- matches `leveragedloopv1`'s own "nothing new
    /// happens without an explicit trigger" ethos for turning a feature
    /// on in the first place, even though `run_basis_cycle` itself then
    /// runs autonomously every real funding epoch once enabled (matching
    /// `perpfundingv1`'s own real, proven behavior for this same
    /// strategy -- delta-neutral by construction, not a directional
    /// leverage decision that needs a human pulling the trigger every
    /// cycle).
    basis_trading_enabled: bool,
    /// Real epoch bookkeeping for `self.router` -- the timestamp
    /// `close_epoch` was last called with, so `run_basis_cycle` can
    /// detect a new real hourly funding-epoch boundary the same way
    /// `perp_router.rs`'s own module doc establishes.
    basis_last_epoch_ts: Option<i64>,
    /// Cooldown gate for `run_basis_cycle`'s own action-sending, separate
    /// from every other `*_last_action_slot` field above (this strategy
    /// is fully independent of `LoopPhase`). Also gates
    /// `check_pending_close_all_basis`'s retries -- same concern
    /// (basis-trade action pacing), one shared cooldown.
    basis_last_action_slot: Option<Slot>,
    /// Set by a `TriggerCloseAllBasisPositions` message, cleared once
    /// `check_pending_close_all_basis` confirms no symbol has a real open
    /// Phoenix position left. Retried (cooldown-gated via
    /// `basis_last_action_slot`) rather than a single attempt, same
    /// pending+retry discipline as `o_pending_recover_mint`.
    o_pending_close_all_basis: bool,
}

impl State {
    fn wallet(&self) -> Option<AccountId> {
        let ke = self.o_rc_keypair.as_ref()?;
        Some(ke.account_id)
    }

    /// `(symbol, mint)` of the LST the current loop position actually
    /// targets -- see [`State::active_lst`]'s (the field's) doc comment.
    fn active_lst(&self) -> (&'static str, Pubkey) {
        self.active_lst.unwrap_or(("jitoSOL", JITOSOL_MINT))
    }

    /// Every pubkey baked in by build.rs's generated tables this mode's
    /// `spot_router`/Kamino state actually touch, batch-resolved once at
    /// startup rather than one at a time as each constructor below needs
    /// them -- same startup-cost optimization testperpv1 established
    /// (see its own `build_time_pubkeys` doc comment), trimmed to the
    /// config tables this mode's build-time router/Kamino setup actually
    /// reads (no Phoenix/Solend/marginfi/Drift/symbol_mint tables --
    /// this mode never touches those).
    fn build_time_pubkeys() -> Vec<Pubkey> {
        let mut out = Vec::new();
        macro_rules! push {
            ($bytes:expr) => {
                out.push(Pubkey::new_from_array($bytes));
            };
        }
        for p in raydium_amm_config::RAYDIUM_AMM_POOLS {
            push!(p.pubkey);
            push!(p.market_bids);
            push!(p.market_asks);
            push!(p.market_event_queue);
            push!(p.market_coin_vault);
            push!(p.market_pc_vault);
            push!(p.market_vault_signer);
        }
        for p in raydium_clmm_config::RAYDIUM_CLMM_POOLS {
            push!(p.pubkey);
            push!(p.mint_0);
            push!(p.mint_1);
        }
        for p in raydium_cpmm_config::RAYDIUM_CPMM_POOLS {
            push!(p.pubkey);
            push!(p.mint_0);
            push!(p.mint_1);
        }
        for p in orca_config::ORCA_WHIRLPOOL_POOLS {
            push!(p.pubkey);
            push!(p.mint_a);
            push!(p.mint_b);
        }
        for p in kamino_config::KAMINO_RESERVES {
            push!(p.pubkey);
            push!(p.lending_market);
            push!(p.supply_vault);
            push!(p.fee_vault);
        }
        for p in sanctum_config::SANCTUM_LSTS {
            push!(p.mint);
            push!(p.sol_value_calculator);
            push!(p.pool_state);
        }
        // Basis-trade strategy (2026-08-29) -- mirrors testperpv1's own
        // build_time_pubkeys exactly for these four tables.
        for p in phoenix_config::PHOENIX_MARKETS {
            push!(p.market_account);
        }
        for p in marginfi_config::MARGINFI_BANKS {
            push!(p.pubkey);
            push!(p.group);
            push!(p.mint);
            push!(p.oracle_key);
        }
        for p in solend_config::SOLEND_RESERVES {
            push!(p.pubkey);
            push!(p.lending_market);
            push!(p.mint);
            push!(p.supply_vault);
        }
        for p in symbol_mint_config::SYMBOL_MINT_MAP {
            push!(p.mint);
        }
        for p in pumpfun_config::PUMPFUN_BONDING_CURVES {
            push!(p.mint);
        }
        for p in pumpswap_config::PUMPSWAP_POOLS {
            push!(p.pool);
            push!(p.base_mint);
            push!(p.quote_mint);
            push!(p.base_vault);
            push!(p.quote_vault);
        }
        for p in top_pools_config::TOP_POOLS {
            push!(p.pubkey);
            push!(p.mint_a);
            push!(p.mint_b);
        }
        for p in router_pools_config::ROUTER_POOLS {
            push!(p.mint_a);
            push!(p.mint_b);
        }
        for m in router_config::ROUTER_CONFIG.core_mints {
            push!(m);
        }
        for bytes in tracked_accounts_config::TRACKED_TOKEN_ACCOUNTS {
            push!(*bytes);
        }
        for entry in atl_config::ADDRESS_LOOKUP_TABLES {
            let (table, addrs) = *entry;
            push!(table);
            for a in addrs {
                push!(*a);
            }
        }
        for (a, b) in trading_config::TRADING_PAIRS {
            push!(*a);
            push!(*b);
        }
        out
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

impl<'a> StateHelper<'a> {
    pub(crate) fn nonce_check(&mut self, other_nonce: u32) -> Result<(), CatscopeGuestError> {
        if *self.nonce != other_nonce {
            return Err(CatscopeGuestError::BadNonce(*self.nonce, other_nonce));
        }
        *self.nonce += 1;
        Ok(())
    }

    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        let l_pk = State::build_time_pubkeys();
        let n_pk = l_pk.len();
        let n_ids = crate::util::pubkey_account_id_cache().account_ids(&l_pk).len();
        log_warn!("leveragedloopv1: batch-resolved {n_ids}/{n_pk} build-time pubkeys to account ids at startup");
        assert!(self
            .state
            .o_kamino_position
            .replace(kamino::KaminoPosition::default())
            .is_none());
        assert!(self.state.o_dex.replace(DexState::new().expect("dex state")).is_none());
        self.state.spot_router = TradeRouter::from_router(&build_liquidity_router());
        // Basis-trade strategy (2026-08-29) -- own PhoenixState instance
        // and own, independent (id=1) Kamino obligation, mirroring
        // `perpfundingv1::state::on_load`'s exact pattern.
        assert!(self
            .state
            .o_phoenix
            .replace(PhoenixState::new_and_subscribe(self.graph).expect("phoenix state"))
            .is_none());
        assert!(self
            .state
            .o_basis_kamino_position
            .replace(kamino::KaminoPosition::default())
            .is_none());
        log_info!("leveragedloopv1: bot has been successfully uploaded to validator");
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("leveragedloopv1: slot {slot}; status dead");
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        while let Some(ta) = llap.token() {
            self.wallet.token_mut().on_token(ta, false);
            if let Some(dex) = self.state.o_dex.as_mut() {
                _ = dex.on_token(ta);
                dex.refresh_token_router(ta.id, &mut self.state.spot_router);
            }
        }
        let zero = [];
        while let Some(account) = llap.account() {
            let d = account.body.unwrap_or(&zero);
            self.wallet.on_account(account.header, d);
            if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
                kamino_position.on_account(account.header, d);
            }
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(account.header, d);
                dex.refresh_account_router(account.header.accountid, &mut self.state.spot_router);
            }
        }
    }

    /// Real on-chain confirmation feedback for `pending_route_pools` --
    /// closes the gap `execute_spot_leg`'s doc comment flags:
    /// `transactionprocessor::send()` only reports *send-time*
    /// rejection, never whether a transaction that sent fine later
    /// reverted on-chain (e.g. the real Raydium CLMM `BorrowError` panic
    /// hit twice live on pool 492876181). This is the validator's own
    /// real confirmation result, decoded with the full real
    /// `TransactionError` (see `txview::result_from_bytes`) -- for any
    /// transaction whose real touched-account set overlaps
    /// `pending_route_pools` and which came back `Err`, cool those pools
    /// down exactly like every other real failure mode already does.
    /// `pending_route_pools` is cleared on the first matching
    /// transaction either way (success or failure) -- it's only ever
    /// meant to answer "how did the most recently queued route turn
    /// out," not accumulate across unrelated later routes.
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while let Some((tx, result)) = transaction_list.transaction() {
            if self.state.pending_route_pools.is_empty() {
                continue;
            }
            let touches_pending_route = self
                .state
                .pending_route_pools
                .iter()
                .any(|pool_id| tx.account.binary_search(pool_id).is_ok());
            if !touches_pending_route {
                continue;
            }
            if let Err(e) = result {
                log_error!(
                    "leveragedloopv1: real on-chain confirmation shows the last queued route's transaction REVERTED: {e:?} -- cooling down its {} pool(s)",
                    self.state.pending_route_pools.len(),
                );
                for &pool_id in &self.state.pending_route_pools {
                    self.state.spot_router.mark_pool_cooldown(pool_id, planner::POOL_COOLDOWN_SLOTS);
                }
            }
            self.state.pending_route_pools.clear();
        }
    }

    /// Real Kamino main-market reserve for `candidate_mint`, if this bot
    /// has one tracked -- `None` is the common case for most of
    /// [`LST_CANDIDATES`] (only 26 of the 37 real candidates have a
    /// Kamino main-market reserve at all; the rest only have a Solend or
    /// marginfi reserve, which this Kamino-only helper can't see -- see
    /// `LST_CANDIDATES`'s own doc comment). Thin wrapper over `KaminoState::reserve_by_mint`
    /// (already mint-generic) so DAG code (see `src/trader/timegraph.rs`)
    /// has one place to call rather than reaching into `o_dex` directly.
    fn kamino_reserve_for_lst(&self, candidate_mint: Pubkey) -> Option<(AccountId, &kamino::KaminoReserve)> {
        let dex = self.state.o_dex.as_ref()?;
        let mint = account_id_from_pubkey(&candidate_mint);
        dex.kamino().reserve_by_mint(mint)
    }

    /// Real, current Kamino USDC borrow APY -- shared across every LST
    /// candidate (Kamino's own USDC reserve is one reserve per lending
    /// market, independent of which asset is posted as collateral), so
    /// unlike `kamino_reserve_for_lst` this isn't per-candidate.
    fn kamino_usdc_borrow_apy(&self) -> Option<f64> {
        let dex = self.state.o_dex.as_ref()?;
        let (_, reserve) = dex.kamino().reserve_by_mint(self.configuration.mint_usdc)?;
        Some(reserve.current_borrow_apy())
    }

    /// Phase 3 (2026-08-28) read-only projection -- mirrors
    /// `testperpv1::log_lst_loop_projection`'s "no transactions sent"
    /// precedent exactly, using the real `timegraph` DAG
    /// (`src/trader/timegraph.rs`/`TIME.md`) instead of that function's
    /// simpler `net_apy(L) = L*yield - (L-1)*borrow` formula, so it can
    /// compare every real candidate (`LST_CANDIDATES`) at once and report
    /// which one the DAG actually picks, not just project one symbol in
    /// isolation. Called once per real `LstApy` update (already
    /// naturally rate-limited to the Go-side poller's hourly cadence --
    /// no separate cooldown gate needed).
    ///
    /// Path shape per candidate: `USDC(t0) -> LST(t0)` [spot swap] ->
    /// `LST(t1)` [1-year yield accrual: real staking APY
    /// (`lst_staking_apy`) + real Kamino supply APY, both live data] ->
    /// `USDC(t1)` [spot swap back], compared against a flat `USDC(t0) ->
    /// USDC(t1)` "hold USDC, do nothing" baseline (weight 0, ratio
    /// exactly 1.0). The swap legs use a nominal 0.3%-each-way fee
    /// approximation, **not** a live `TradeRouter` quote yet -- the yield
    /// and borrow rates driving the actual profitability signal are 100%
    /// real; only the round-trip transaction-cost estimate is a
    /// placeholder, left for a later refinement once this projection has
    /// been observed live. This function reports the *unlevered*
    /// (1x) comparison across candidates -- folding this bot's real
    /// leverage sizing (`TARGET_BORROW_FRACTION`) into the graph itself
    /// is Phase 3.7's job, once a real trigger is wired to consume the
    /// DAG's answer instead of just logging it.
    fn log_dag_lst_projection(&self) {
        let Some(decision) = self.dag_best_lst_path() else {
            return;
        };
        log_warn!(
            "leveragedloopv1: DAG LST projection ({:.0}yr horizon, {} real candidate(s): {}) -- best path: USDC -> {} -> USDC, profit_ratio={:.5} ({:+.3}%), usdc_borrow_apy={:.3}%",
            DagLstDecision::HORIZON_YEARS,
            decision.candidates_used.len(),
            decision.candidates_used.join(","),
            decision.winner,
            decision.profit_ratio,
            (decision.profit_ratio - 1.0) * 100.0,
            decision.usdc_borrow_apy * 100.0,
        );
    }

    /// Real decision, shared by `log_dag_lst_projection` (logging only)
    /// and `TriggerOpenAuto`'s handler (an actual open decision): builds
    /// the `USDC(t0) -> LST(t0) -> LST(t1) -> USDC(t1)` graph across
    /// every real candidate (`LST_CANDIDATES`) with both live yield data
    /// (`lst_staking_apy`) and a real Kamino reserve, compared against a
    /// flat `USDC(t0) -> USDC(t1)` "hold USDC, do nothing" baseline, and
    /// returns the DAG's chosen best path. `None` if no candidate has
    /// real data yet (nothing to decide) or the Kamino USDC borrow rate
    /// isn't available yet either. `decision.winner == "USDC"` means "do
    /// nothing" won -- no real LST candidate clears its own round-trip
    /// swap cost right now.
    ///
    /// Real, known simplification (2026-08-28, carried over from
    /// `log_dag_lst_projection`'s original doc comment): the swap legs
    /// use a nominal 0.3%-each-way fee approximation, not a live
    /// `TradeRouter` quote -- the yield and borrow rates driving the
    /// actual profitability signal are 100% real; only the round-trip
    /// transaction-cost estimate is a placeholder. `TriggerOpenAuto`
    /// relies on this decision for real money, so this simplification is
    /// a real, deliberate risk this function's callers should be aware
    /// of, not silently trust as fully precise.
    fn dag_best_lst_path(&self) -> Option<DagLstDecision> {
        let usdc_borrow_apy = self.kamino_usdc_borrow_apy()?;
        const USDC: &str = "USDC";
        let usdc_t0 = timegraph::Node::new(USDC, 0);
        let usdc_t1 = timegraph::Node::new(USDC, 1);

        let mut g = timegraph::TimeGraph::new();
        g.add_edge(usdc_t0, usdc_t1, 0.0);

        let mut candidates_used: Vec<&'static str> = Vec::new();
        for &(label, mint) in LST_CANDIDATES {
            let Some(staking_apy) = self.state.lst_staking_apy.get(&mint).copied() else {
                continue;
            };
            let Some((_, reserve)) = self.kamino_reserve_for_lst(mint) else {
                continue;
            };
            let total_yield = staking_apy + reserve.current_supply_apy();
            let lst_t0 = timegraph::Node::new(label, 0);
            let lst_t1 = timegraph::Node::new(label, 1);
            g.add_edge(usdc_t0, lst_t0, timegraph::spot_edge_weight(DagLstDecision::NOMINAL_SWAP_RATE));
            g.add_edge(
                lst_t0,
                lst_t1,
                timegraph::yield_edge_weight(total_yield, DagLstDecision::HORIZON_YEARS),
            );
            g.add_edge(lst_t1, usdc_t1, timegraph::spot_edge_weight(DagLstDecision::NOMINAL_SWAP_RATE));
            candidates_used.push(label);
        }
        if candidates_used.is_empty() {
            return None;
        }
        let (weight, path) = g.shortest_path(usdc_t0, usdc_t1)?;
        let profit_ratio = timegraph::profit_ratio(weight);
        let winner = path.get(1).map(|n| n.token).unwrap_or(USDC);
        Some(DagLstDecision { winner, profit_ratio, usdc_borrow_apy, candidates_used })
    }

    /// One-time bootstrap for this bot's own Kamino obligation --
    /// identical to `testperpv1::state::bootstrap_kamino_obligation`
    /// (Kamino-generic, not basis-trade-specific).
    fn bootstrap_kamino_obligation(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);
        let has_user_metadata = self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("leveragedloopv1: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("leveragedloopv1: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("leveragedloopv1: bootstrap: registering Kamino obligation");
        if let Err(e) = kamino::init_obligation(owner, lending_market, 0, self.wallet) {
            log_error!("leveragedloopv1: bootstrap: kamino init_obligation failed: {e}");
        }
    }

    /// Real USDC value of this wallet's own token balance, `0.0` if
    /// there's no wallet keypair yet -- same live `TokenDatabase` lookup
    /// `perpfundingv1::state::current_usdc_value` uses, ported verbatim
    /// for the basis-trade strategy's own capital sizing
    /// (`bootstrap_phoenix_trader`/`run_basis_cycle`). USDC assumed
    /// pegged, same convention every other USDC valuation in this
    /// codebase already uses.
    fn current_usdc_value(&mut self) -> f64 {
        let Some(owner) = self.state.wallet() else {
            return 0.0;
        };
        const USDC_DECIMALS: i32 = 6;
        let mint_usdc = self.configuration.mint_usdc;
        let usdc_balance_raw: u64 =
            self.wallet.token_mut().balance(&owner, &mint_usdc, true).iter().map(|(_, a)| *a).sum();
        usdc_balance_raw as f64 / 10f64.powi(USDC_DECIMALS)
    }

    /// One-time bootstrap for the basis trade's Phoenix trader account:
    /// `register_trader`, convert USDC -> PhUSD via Ember (`dex::ember`,
    /// Phoenix's margin collateral is its own canonical mint, not USDC
    /// directly -- see `trader::dex::phoenix::mod`'s module doc), then
    /// `deposit_funds` as margin collateral -- batched into a single
    /// transaction (Solana executes instructions within one transaction
    /// sequentially, so `deposit_funds` can safely reference the account
    /// `register_trader` just created earlier in the same tx). Ported
    /// verbatim from `perpfundingv1::state::bootstrap_phoenix_trader`.
    /// Budget is half of `BASIS_CYCLE_MIN_MARGIN_USD`, bounded by
    /// `current_usdc_value()` so this never tries to spend USDC that
    /// isn't there. Called instead of placing an order (see
    /// `open_phoenix_leg`'s call site) -- so the first capital-feasible
    /// cycle found after a fresh wallet is spent on setup, not a real
    /// position; the next one proceeds normally once
    /// `PhoenixState::trader_registered` flips true from a real
    /// `on_account` update.
    fn bootstrap_phoenix_trader(&mut self) {
        let Some(owner) = self.state.wallet() else { return };
        if self.state.o_phoenix.as_ref().and_then(|p| p.trader_account()).is_none() {
            log_warn!("leveragedloopv1: basis: bootstrap: phoenix trader_account PDA not known yet -- set_authority hasn't run");
            return;
        }
        let budget_usd = (BASIS_CYCLE_MIN_MARGIN_USD / 2.0).min(self.current_usdc_value());
        if budget_usd <= 0.0 {
            log_warn!("leveragedloopv1: basis: bootstrap: no spare USDC to fund the Phoenix trader account yet");
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

        log_warn!(
            "leveragedloopv1: basis: bootstrap: registering + funding Phoenix trader account (${:.2})",
            budget_usd,
        );
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        if let Err(e) = phoenix.register_trader(owner, self.wallet) {
            log_error!("leveragedloopv1: basis: bootstrap: phoenix register_trader failed: {e}");
            return;
        }
        if let Err(e) = ember::deposit(owner, phusd_mint_id, usdc_ata, phusd_ata, amount_raw, self.wallet) {
            log_error!("leveragedloopv1: basis: bootstrap: ember deposit failed: {e}");
            return;
        }
        if let Err(e) = phoenix.deposit_funds(owner, phusd_ata, amount_raw, self.wallet) {
            log_error!("leveragedloopv1: basis: bootstrap: phoenix deposit_funds failed: {e}");
        }
    }

    /// Identical to `testperpv1::state::ensure_kamino_farm_ready` --
    /// see that function's doc comment for the real `FarmAccountsMissing`
    /// bug this guards against.
    fn ensure_kamino_farm_ready(
        &mut self,
        reserve_id: AccountId,
        reserve_lending_market: AccountId,
        farm: Option<AccountId>,
        mode: u8,
    ) -> bool {
        let Some(farm) = farm else { return true };
        let Some(owner) = self.state.wallet() else {
            return false;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return false;
        };
        let Some(farm_user_state_id) = kamino::farm_user_state_id(farm, obligation_id) else {
            return false;
        };
        let Some(kamino_position) = self.state.o_kamino_position.as_mut() else {
            return false;
        };
        if let Err(e) = kamino_position.track_farm_user_state(farm_user_state_id, self.graph) {
            log_error!("leveragedloopv1: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!("leveragedloopv1: bootstrapping Kamino farm-user-state for reserve {reserve_id}");
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: kamino init_obligation_farms_for_reserve failed: {e}");
        }
        false
    }

    fn kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    fn kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    fn kamino_refresh_reserves(&self) -> (Vec<AccountId>, Vec<AccountId>) {
        (
            self.kamino_obligation_deposit_reserves(),
            self.kamino_obligation_borrow_reserves(),
        )
    }

    /// Refreshes every reserve `refresh_obligation` will need in this same
    /// transaction: every reserve *currently* in the obligation (real
    /// on-chain state, via [`Self::kamino_refresh_reserves`]) plus
    /// `extra` -- reserve(s) this specific step is about to touch for the
    /// first time (e.g. the collateral reserve on an initial deposit),
    /// which aren't yet obligation members but still need a fresh
    /// `refresh_reserve` of their own before the deposit/borrow
    /// instruction that adds them.
    ///
    /// Real, live-confirmed gap this closes (2026-08-27, first real
    /// `leveragedloopv1` open attempt): this wallet's obligation already
    /// had an unrelated pre-existing USDC deposit from before this bot
    /// ever touched it, and only refreshing the reserve *this step*
    /// directly cared about (jitoSOL) left that USDC reserve stale,
    /// reverting `refresh_obligation` with a real on-chain
    /// `ReserveStale` error every time. Same root cause already fixed
    /// once for testperpv1/perpfundingv1's own Kamino legs, but for a
    /// *fixed, hardcoded* second reserve (USDC) they always deposit into
    /// -- this bot has no such fixed set, so it has to discover the real
    /// membership every time instead.
    fn kamino_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        // Real, live-confirmed gap (2026-08-27): a real `ReserveStale`
        // (6009) landed on-chain here even with this function in place --
        // the obligation's own pre-existing collateral reserve wasn't in
        // `deposit_reserves` at refresh time, only `extra` got refreshed.
        // Logged at warn (not debug) specifically so a real run makes
        // this directly visible without needing to re-instrument later --
        // if `deposit_reserves`/`borrow_reserves` ever come back emptier
        // than the real on-chain obligation actually has, this is where
        // that would first become observable.
        log_warn!(
            "leveragedloopv1: kamino_refresh_all_reserves: deposit_reserves={deposit_reserves:?} borrow_reserves={borrow_reserves:?} extra={extra:?} obligation_loaded={}",
            self.state.o_kamino_position.as_ref().and_then(|s| s.obligation()).is_some(),
        );
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

    /// One-time bootstrap for the basis trade's own, independent
    /// (`id=1`) Kamino obligation -- identical shape to
    /// `bootstrap_kamino_obligation` (the leverage loop's own `id=0`
    /// one), just targeting `o_basis_kamino_position`/`id=1` so the two
    /// never share a real obligation.
    fn bootstrap_basis_kamino_obligation(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);
        let has_user_metadata = self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.user_metadata_registered())
            || self
                .state
                .o_basis_kamino_position
                .as_ref()
                .is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("leveragedloopv1: basis: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("leveragedloopv1: basis: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("leveragedloopv1: basis: bootstrap: registering basis-trade Kamino obligation (id=1)");
        if let Err(e) = kamino::init_obligation(owner, lending_market, 1, self.wallet) {
            log_error!("leveragedloopv1: basis: bootstrap: kamino init_obligation (id=1) failed: {e}");
        }
    }

    fn basis_kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    fn basis_kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation()) else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    /// Basis-trade counterpart of `kamino_refresh_all_reserves`, scoped
    /// to `o_basis_kamino_position`/`id=1` -- same "refresh every reserve
    /// the real obligation currently has, plus whatever this step is
    /// about to touch for the first time" discipline, same
    /// `ReserveStale`-avoidance reasoning.
    fn basis_kamino_refresh_all_reserves(&mut self, extra: &[AccountId]) -> Result<(), String> {
        let deposit_reserves = self.basis_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.basis_kamino_obligation_borrow_reserves();
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

    fn basis_kamino_refresh_obligation(&mut self, lending_market: AccountId) -> Result<(), String> {
        let Some(obligation_id) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return Err("basis kamino obligation not resolved yet".to_string());
        };
        let deposit_reserves = self.basis_kamino_obligation_deposit_reserves();
        let borrow_reserves = self.basis_kamino_obligation_borrow_reserves();
        kamino::refresh_obligation(lending_market, obligation_id, &deposit_reserves, &borrow_reserves, self.wallet)
            .map_err(|e| e.to_string())
    }

    /// Basis-trade counterpart of `ensure_kamino_farm_ready`, scoped to
    /// `o_basis_kamino_position`/`id=1`.
    fn ensure_basis_kamino_farm_ready(
        &mut self,
        reserve_id: AccountId,
        reserve_lending_market: AccountId,
        farm: Option<AccountId>,
        mode: u8,
    ) -> bool {
        let Some(farm) = farm else { return true };
        let Some(owner) = self.state.wallet() else {
            return false;
        };
        let Some(obligation_id) = self
            .state
            .o_basis_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return false;
        };
        let Some(farm_user_state_id) = kamino::farm_user_state_id(farm, obligation_id) else {
            return false;
        };
        let Some(basis_kamino_position) = self.state.o_basis_kamino_position.as_mut() else {
            return false;
        };
        if let Err(e) = basis_kamino_position.track_farm_user_state(farm_user_state_id, self.graph) {
            log_error!("leveragedloopv1: basis: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if basis_kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!("leveragedloopv1: basis: bootstrapping Kamino farm-user-state for reserve {reserve_id}");
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: basis: kamino init_obligation_farms_for_reserve failed: {e}");
        }
        false
    }

    /// Opens the deposit-hedge direction of the basis trade for `symbol`
    /// via Kamino -- swaps `notional_usd` of USDC into the underlying,
    /// deposits it into the basis trade's own (`id=1`) obligation. Ported
    /// (Kamino-only, per the user's explicit scope choice -- Solend/
    /// marginfi stay read-only comparison data, see `run_basis_cycle`'s
    /// doc comment) from `perpfundingv1::state::open_kamino_deposit_leg`.
    /// No-op if a deposit already exists for this reserve (open-once-hold).
    fn open_kamino_deposit_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self.state.o_basis_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_basis_kamino_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let already_deposited = self
            .state
            .o_basis_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("leveragedloopv1: basis: {} has no Kamino oracle price yet, skipping deposit-hedge", symbol);
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

        log_warn!(
            "leveragedloopv1: basis: opening Kamino deposit-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw, LOOP_MAX_HOPS) {
            log_error!("leveragedloopv1: basis: deposit-hedge {} kamino spot swap failed: {}", symbol, e);
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
            log_error!("leveragedloopv1: basis: deposit-hedge {} kamino refresh_reserve failed: {}", symbol, e);
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.basis_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: basis: deposit-hedge {} kamino refresh_all_reserves failed: {}", symbol, e);
            return;
        }
        if let Err(e) = self.basis_kamino_refresh_obligation(lending_market) {
            log_error!("leveragedloopv1: basis: deposit-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_basis_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) = reserve.deposit(reserve_id, obligation_id, amount_raw, owner, underlying_ata, self.wallet) {
            log_error!("leveragedloopv1: basis: deposit-hedge {} kamino deposit failed: {}", symbol, e);
        }
    }

    /// Opens the borrow-hedge direction of the basis trade for `symbol`
    /// via Kamino -- two-stage: deposit USDC collateral first (if none
    /// yet), then borrow the underlying + sell it for USDC (synthetic
    /// short) once enough collateral is confirmed. Ported (Kamino-only)
    /// from `perpfundingv1::state::open_kamino_borrow_leg` -- see that
    /// function's doc comment for the real `BorrowTooLarge`-avoidance
    /// `LTV_SAFETY_FACTOR`/`borrow_factor_pct` sizing this reuses exactly.
    fn open_kamino_borrow_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self.state.o_basis_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            self.bootstrap_basis_kamino_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };

        const USDC_DECIMALS: i32 = 6;
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let Some(mint) = resolve_symbol_mint(symbol) else { return };
        let Some((_, borrow_reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let collateral_usd = notional_usd * borrow_reserve.borrow_factor_pct
            / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        let has_enough_usdc_collateral = self
            .state
            .o_basis_kamino_position
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
            log_warn!(
                "leveragedloopv1: basis: depositing ${:.2} USDC collateral for {} ${:.2} Kamino borrow-hedge",
                collateral_usd,
                symbol,
                notional_usd,
            );
            if let Err(e) = usdc_reserve.refresh_reserve(
                usdc_reserve_id,
                usdc_reserve.pyth_oracle,
                usdc_reserve.switchboard_price_oracle,
                usdc_reserve.switchboard_twap_oracle,
                usdc_reserve.scope_prices,
                self.wallet,
            ) {
                log_error!("leveragedloopv1: basis: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
                return;
            }
            let (usdc_lending_market, usdc_farm_collateral) = (usdc_reserve.lending_market, usdc_reserve.farm_collateral);
            if let Err(e) = self.basis_kamino_refresh_all_reserves(&[usdc_reserve_id]) {
                log_error!("leveragedloopv1: basis: borrow-hedge {} kamino refresh_all_reserves failed: {}", symbol, e);
                return;
            }
            if let Err(e) = self.basis_kamino_refresh_obligation(usdc_lending_market) {
                log_error!("leveragedloopv1: basis: borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
                return;
            }
            if !self.ensure_basis_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_collateral, 0) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else { return };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else { return };
            if let Err(e) =
                usdc_reserve.deposit(usdc_reserve_id, obligation_id, usdc_amount_raw, owner, usdc_ata, self.wallet)
            {
                log_error!("leveragedloopv1: basis: borrow-hedge {} kamino USDC collateral deposit failed: {}", symbol, e);
            }
            return;
        }

        let Some(mint) = resolve_symbol_mint(symbol) else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        let already_borrowed = self
            .state
            .o_basis_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("leveragedloopv1: basis: {} has no Kamino oracle price yet, skipping borrow-hedge", symbol);
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else { return };

        log_warn!(
            "leveragedloopv1: basis: opening Kamino borrow-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino refresh_reserve failed: {}", symbol, e);
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
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
            return;
        }
        let (lending_market, farm_debt) = (reserve.lending_market, reserve.farm_debt);
        if let Err(e) = self.basis_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino refresh_all_reserves failed: {}", symbol, e);
            return;
        }
        let deposit_reserves = self.basis_kamino_obligation_deposit_reserves();
        if let Err(e) = self.basis_kamino_refresh_obligation(lending_market) {
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_basis_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
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
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino borrow failed: {}", symbol, e);
            return;
        }
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw, LOOP_MAX_HOPS) {
            log_error!("leveragedloopv1: basis: borrow-hedge {} kamino spot sell failed: {}", symbol, e);
        }
    }

    /// Closes the deposit-hedge direction for `symbol` via Kamino --
    /// withdraws the real deposited amount, sells it back to USDC. Ported
    /// (Kamino-only) from `perpfundingv1::state::close_kamino_deposit_leg`.
    fn close_kamino_deposit_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let has_deposit = self
            .state
            .o_basis_kamino_position
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

        log_warn!("leveragedloopv1: basis: closing Kamino deposit-hedge {}", symbol);
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: basis: close deposit-hedge {} kamino refresh_reserve failed: {}", symbol, e);
            return;
        }
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.basis_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: basis: close deposit-hedge {} kamino refresh_all_reserves failed: {}", symbol, e);
            return;
        }
        if let Err(e) = self.basis_kamino_refresh_obligation(lending_market) {
            log_error!("leveragedloopv1: basis: close deposit-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_basis_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("leveragedloopv1: basis: close deposit-hedge {} kamino withdraw failed: {}", symbol, e);
            return;
        }
        let obligation_will_be_empty = self
            .state
            .o_basis_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .is_some_and(|ob| ob.deposits.len() <= 1 && ob.borrows.is_empty());
        if obligation_will_be_empty {
            if let Some(pos) = self.state.o_basis_kamino_position.as_mut() {
                pos.mark_obligation_closing();
            }
        }
        let estimated_underlying_raw = ((BASIS_CYCLE_MIN_MARGIN_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw, LOOP_MAX_HOPS) {
            log_error!("leveragedloopv1: basis: close deposit-hedge {} kamino spot sell failed: {}", symbol, e);
        }
    }

    /// Closes the borrow-hedge direction for `symbol` via Kamino -- buys
    /// back the real, currently-borrowed amount with USDC, repays
    /// everything owed. USDC collateral stays deposited for reuse. Ported
    /// (Kamino-only) from `perpfundingv1::state::close_kamino_borrow_leg`.
    fn close_kamino_borrow_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(obligation_id) = self.state.o_basis_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else { return };
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };

        let Some(borrowed_amount) = self
            .state
            .o_basis_kamino_position
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
            log_warn!("leveragedloopv1: basis: closing Kamino borrow-hedge {}", symbol);
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw, LOOP_MAX_HOPS) {
                log_error!("leveragedloopv1: basis: close borrow-hedge {} kamino buy-back failed: {}", symbol, e);
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
            log_error!("leveragedloopv1: basis: close borrow-hedge {} kamino refresh_reserve failed: {}", symbol, e);
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
                log_error!(
                    "leveragedloopv1: basis: close borrow-hedge {} kamino USDC refresh_reserve failed: {}",
                    symbol,
                    e
                );
                return;
            }
        }
        if let Err(e) = self.basis_kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: basis: close borrow-hedge {} kamino refresh_all_reserves failed: {}", symbol, e);
            return;
        }
        if let Err(e) = self.basis_kamino_refresh_obligation(lending_market) {
            log_error!("leveragedloopv1: basis: close borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_basis_kamino_farm_ready(reserve_id, lending_market, farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else { return };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else { return };
        if let Err(e) =
            reserve.repay(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, underlying_ata, self.wallet)
        {
            log_error!("leveragedloopv1: basis: close borrow-hedge {} kamino repay failed: {}", symbol, e);
        }
    }

    /// Live Phoenix position size for `symbol`, signed (`> 0` long, `< 0`
    /// short), `None` if flat or the market/position isn't known yet.
    /// Real on-chain state, not separate bookkeeping -- shared by the
    /// open-gate, the close-decision, and `close_phoenix_leg`. Ported
    /// verbatim from `perpfundingv1::state::phoenix_position`.
    fn phoenix_position(&self, symbol: &str) -> Option<i64> {
        let phoenix = self.state.o_phoenix.as_ref()?;
        let market = phoenix.markets().iter().find(|m| m.symbol_str() == symbol)?;
        let pos = phoenix.positions().iter().find(|p| p.asset_id as u32 == market.asset_id)?;
        (pos.base_lot_position != 0).then_some(pos.base_lot_position)
    }

    /// Places a real Phoenix market order for `symbol` -- `long=false`
    /// (short) for the deposit-hedge direction, `long=true` for the
    /// borrow-hedge direction. Bootstraps the trader account instead of
    /// ordering if it isn't registered yet (see `bootstrap_phoenix_trader`'s
    /// doc comment); no-ops if a position is already open (prevents
    /// pyramiding). Sized from `notional_usd / mark_price_usd()`. Ported
    /// verbatim from `perpfundingv1::state::open_phoenix_leg`.
    fn open_phoenix_leg(&mut self, symbol: &str, long: bool, notional_usd: f64) {
        let Some(owner) = self.state.wallet() else { return };
        if !self.state.o_phoenix.as_ref().is_some_and(|p| p.trader_registered()) {
            self.bootstrap_phoenix_trader();
            return;
        }
        if self.phoenix_position(symbol).is_some() {
            return;
        }
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == symbol) else {
            return;
        };
        let Some(price_usd) = market.mark_price_usd() else {
            log_error!("leveragedloopv1: basis: {} has no oracle price yet, skipping Phoenix leg", symbol);
            return;
        };
        let num_base_lots = ((notional_usd / price_usd) * 10f64.powi(market.base_lot_decimals as i32)).round() as u64;
        if num_base_lots == 0 {
            return;
        }
        let asset_id = market.asset_id;
        let side = if long { Side::Bid } else { Side::Ask };

        log_warn!(
            "leveragedloopv1: basis: opening Phoenix leg {} side={:?} num_base_lots={} (notional=${:.2})",
            symbol, side, num_base_lots, notional_usd,
        );
        if let Err(e) = phoenix.place_market_order(owner, asset_id, side, num_base_lots, 0, 0, self.wallet) {
            log_error!("leveragedloopv1: basis: Phoenix leg {} failed: {}", symbol, e);
        }
    }

    /// Closes any real open Phoenix position for `symbol` with an
    /// opposite-side market order sized at the exact real position (real
    /// on-chain state, not an estimate). Ported verbatim from
    /// `perpfundingv1::state::close_phoenix_leg`.
    fn close_phoenix_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else { return };
        let Some(phoenix) = self.state.o_phoenix.as_ref() else { return };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == symbol) else { return };
        let Some(base_lot_position) = self.phoenix_position(symbol) else { return };
        let asset_id = market.asset_id;
        let side = if base_lot_position > 0 { Side::Ask } else { Side::Bid };
        let size = base_lot_position.unsigned_abs();
        let client_order_id = self.state.last_slot as u128;
        log_warn!("leveragedloopv1: basis: closing Phoenix leg {} side={:?} size={}", symbol, side, size);
        if let Err(e) = phoenix.place_market_order(owner, asset_id, side, size, 0, client_order_id, self.wallet) {
            log_error!("leveragedloopv1: basis: Phoenix close {} failed: {}", symbol, e);
        }
    }

    /// Real `CreditReserve` for `symbol`'s underlying mint, Kamino only
    /// (per the user's explicit scope choice) -- `None` if this bot
    /// hasn't tracked a real Kamino main-market reserve for it. Owned
    /// return value (not a borrowed reference), deliberately, so callers
    /// never hold `self.state.o_dex`'s borrow across a later `&mut self`
    /// action call (real borrow-checker conflict hit and fixed during
    /// `open_kamino_deposit_leg`/etc.'s own porting -- see those
    /// functions' `let (lending_market, ...) = (reserve.lending_market, ...)`
    /// pattern for the same fix applied there).
    fn basis_kamino_credit_reserve(&self, mint: AccountId) -> Option<credit::CreditReserve> {
        let dex = self.state.o_dex.as_ref()?;
        let (reserve_id, reserve) = dex.kamino().reserve_by_mint(mint)?;
        Some(credit::CreditReserve::from_kamino(reserve_id, reserve))
    }

    /// Real wall-clock funding-epoch timestamp, rounded down to the
    /// current `SECONDS_PER_EPOCH` boundary -- identical to
    /// `perpfundingv1::state::current_epoch_ts`.
    fn current_basis_epoch_ts() -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs() as i64;
        (now / SECONDS_PER_EPOCH) * SECONDS_PER_EPOCH
    }

    /// Feed every currently-known Phoenix market into `self.state.router`
    /// for the given epoch. Called every `evaluate()` tick basis trading
    /// is enabled, not just at the epoch boundary, so the router's
    /// pending buffers always reflect the freshest reading by the time
    /// the epoch actually closes. Identical to
    /// `perpfundingv1::state::observe_all`.
    fn basis_observe_all(&mut self, epoch_ts: i64) {
        if let Some(phoenix) = self.state.o_phoenix.as_ref() {
            for market in phoenix.markets() {
                self.state.router.observe_phoenix(market, market.mark_price_usd(), epoch_ts);
            }
        }
    }

    /// Real, reusable decision driver for the basis trade -- generalizes
    /// `perpfundingv1::state::log_basis_cycles`' private open-pass loop
    /// into a call through the standalone `derivative_router` module
    /// instead of a per-file-private per-symbol loop. Close pass first
    /// (re-checks every symbol currently holding a real Phoenix position,
    /// closes both legs if `decide_basis_trade` no longer agrees -- same
    /// logic as `perpfundingv1::state::close_basis_trade_if_needed`,
    /// generalized across all 6 symbols instead of one call per symbol),
    /// then open pass (opens the best-ranked real opportunity per
    /// available USDC budget). Kamino-only lending data throughout, per
    /// the user's explicit scope choice (2026-08-29) -- Solend/marginfi
    /// reserves are never read here; this strategy's own real deposit/
    /// borrow only ever targets its own `id=1` Kamino obligation. Called
    /// from `evaluate()` on every real funding-epoch boundary, once
    /// `TriggerEnableBasisTrading` has fired -- see that variant's doc
    /// comment for why this runs autonomously rather than needing a
    /// trigger every cycle.
    fn run_basis_cycle(&mut self) {
        // Close pass.
        for (symbol, mint) in basis_symbols() {
            let Some(phoenix_pos) = self.phoenix_position(symbol) else { continue };
            let currently_open = if phoenix_pos > 0 {
                derivative_router::BasisDirection::BorrowHedge
            } else {
                derivative_router::BasisDirection::DepositHedge
            };
            let Some(phoenix_rate) = self.state.router.pending_rate(PerpVenue::Phoenix, symbol) else { continue };
            let Some(borrow_apy_pct) = self.basis_kamino_credit_reserve(mint).map(|cr| cr.borrow_apy * 100.0) else {
                continue;
            };
            if derivative_router::decide_basis_trade(phoenix_rate, borrow_apy_pct) == Some(currently_open) {
                continue;
            }
            log_warn!(
                "leveragedloopv1: basis: closing {} -- direction reversed or no longer profitable",
                symbol,
            );
            self.close_phoenix_leg(symbol);
            match currently_open {
                derivative_router::BasisDirection::DepositHedge => self.close_kamino_deposit_leg(symbol),
                derivative_router::BasisDirection::BorrowHedge => self.close_kamino_borrow_leg(symbol),
            }
        }

        // Open pass: build real inputs for every symbol not already open,
        // search once via derivative_router, open the best-ranked real
        // opportunities up to available budget.
        let mut per_symbol: Vec<(&'static str, f64, Vec<credit::CreditReserve>)> = Vec::new();
        for (symbol, mint) in basis_symbols() {
            if self.phoenix_position(symbol).is_some() {
                continue;
            }
            let Some(phoenix_rate) = self.state.router.pending_rate(PerpVenue::Phoenix, symbol) else { continue };
            let reserves: Vec<credit::CreditReserve> = self.basis_kamino_credit_reserve(mint).into_iter().collect();
            per_symbol.push((symbol, phoenix_rate, reserves));
        }
        let inputs: Vec<derivative_router::SymbolFundingInput> = per_symbol
            .iter()
            .map(|(symbol, rate, reserves)| derivative_router::SymbolFundingInput {
                symbol,
                phoenix_funding_pct: *rate,
                reserves,
            })
            .collect();
        let opportunities = derivative_router::find_best_funding_opportunities(&inputs);

        let mut spare_usdc = self.current_usdc_value();
        for opp in opportunities {
            if spare_usdc < BASIS_CYCLE_MIN_MARGIN_USD {
                break;
            }
            log_warn!(
                "leveragedloopv1: basis: opening {} direction={:?} (phoenix_funding={:.3}% borrow_apy={:.3}% net_edge={:.3}%)",
                opp.symbol, opp.direction, opp.phoenix_funding_pct, opp.borrow_apy_pct, opp.net_edge_pct,
            );
            match opp.direction {
                derivative_router::BasisDirection::DepositHedge => {
                    self.open_phoenix_leg(opp.symbol, false, BASIS_CYCLE_MIN_MARGIN_USD);
                    self.open_kamino_deposit_leg(opp.symbol, BASIS_CYCLE_MIN_MARGIN_USD);
                }
                derivative_router::BasisDirection::BorrowHedge => {
                    self.open_phoenix_leg(opp.symbol, true, BASIS_CYCLE_MIN_MARGIN_USD);
                    self.open_kamino_borrow_leg(opp.symbol, BASIS_CYCLE_MIN_MARGIN_USD);
                }
            }
            spare_usdc -= BASIS_CYCLE_MIN_MARGIN_USD;
        }
    }

    /// See `o_pending_close_all_basis`'s doc comment. Called on every
    /// `evaluate()` tick; a no-op unless a `TriggerCloseAllBasisPositions`
    /// is pending. Force-closes every symbol with a real open Phoenix
    /// position, independent of what `decide_basis_trade` currently says
    /// -- a manual override, not a normal cycle decision. Clears the
    /// pending flag only once a real on-chain check confirms nothing is
    /// open left, same "confirm via real state, not assumption" style as
    /// `close_basis_trade_if_needed`'s hold-vs-close comparison.
    fn check_pending_close_all_basis(&mut self) {
        if !self.state.o_pending_close_all_basis {
            return;
        }
        if let Some(last) = self.state.basis_last_action_slot {
            if self.state.last_slot.saturating_sub(last) < LOOP_ACTION_COOLDOWN_SLOTS {
                return;
            }
        }
        self.state.basis_last_action_slot = Some(self.state.last_slot);
        let mut any_open = false;
        for (symbol, _) in basis_symbols() {
            let Some(phoenix_pos) = self.phoenix_position(symbol) else { continue };
            any_open = true;
            log_warn!("leveragedloopv1: basis: TriggerCloseAllBasisPositions -- closing {}", symbol);
            self.close_phoenix_leg(symbol);
            if phoenix_pos > 0 {
                self.close_kamino_borrow_leg(symbol);
            } else {
                self.close_kamino_deposit_leg(symbol);
            }
        }
        if !any_open {
            log_warn!("leveragedloopv1: basis: TriggerCloseAllBasisPositions -- confirmed nothing open");
            self.state.o_pending_close_all_basis = false;
        }
    }

    /// Same multi-hop spot-swap execution every other bot mode uses --
    /// copied verbatim from `testperpv1::state::execute_spot_leg`
    /// (generic, not basis-trade-specific).
    /// Returns the route's real, exact-quote-verified total output
    /// amount on success -- real, live-confirmed motivation (2026-08-27):
    /// callers used to size a *following* instruction (a Kamino deposit,
    /// a redeposit) from an independent pre-swap oracle-price estimate
    /// instead of what the swap actually produced. When the real swap
    /// output came in even slightly under that estimate, the deposit
    /// tried to move more than was actually received and failed with a
    /// genuine on-chain `insufficient funds` -- observed twice in a row
    /// on a real single-hop USDC->jitoSOL swap (requested deposit
    /// 380362195 raw vs. real swap output 380187463 raw). Since the
    /// whole transaction (swap included) is atomic, the failure was safe
    /// (full revert, no funds lost) but the deposit could never succeed
    /// this way. Callers should use this return value, not a separate
    /// estimate, to size whatever comes next.
    /// `max_hops` -- stop-gap, not a fundamental limit (2026-08-27):
    /// the loop's own USDC<->jitoSOL legs pass 2 here after real,
    /// live-repeated evidence showed 3- and 4-hop routes for that pair
    /// are routinely too large for one transaction under
    /// `Wallet::MAX_TX_SIZE`=1232 bytes (observed 1986-2219 bytes across
    /// many real attempts, `atomic_group_fits` catching and cooling
    /// every one down before send) -- with `route_slippage_aware`'s
    /// widest-path search always preferring the longest chain it's
    /// allowed to search first, that ceiling meant this bot almost never
    /// got offered a route with any real chance of fitting. 2-hop routes
    /// have reliably fit in one transaction every time observed live.
    /// `check_pending_recover`'s one-off token-recovery calls pass a
    /// larger value instead -- real, live-confirmed: `max_hops=2` alone
    /// left zero live routes for a real, real-money CARDS->USDC recovery
    /// (an obscure token without a short direct path), and a recovery
    /// call is a rare, deliberately-triggered one-shot where an
    /// oversized-route rejection (safe, just wasted) is a fine trade for
    /// having a chance to find a route at all. The real fix for the
    /// tx-size ceiling itself is per-hop-transaction execution via a
    /// bundle relay (Jito/Astralane), guaranteeing atomic ordered
    /// execution across multiple transactions without needing everything
    /// in one -- planned, not yet built.
    pub(crate) fn execute_spot_leg(
        &mut self,
        mint_in: AccountId,
        mint_out: AccountId,
        amount_in: u64,
        max_hops: usize,
    ) -> Result<u64, String> {
        let Some(owner) = self.state.wallet() else {
            return Err("no wallet keypair yet".to_string());
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return Err("dex state not ready".to_string());
        };
        self.state.spot_router.set_current_slot(self.state.last_slot);
        let Some(route) = self
            .state
            .spot_router
            .route_slippage_aware(mint_in, mint_out, amount_in, max_hops)
        else {
            log_error!(
                "leveragedloopv1: route diagnostics for {mint_in} -> {mint_out}:\n{}",
                self.state.spot_router.route_diagnostics(mint_in, mint_out, amount_in, max_hops)
            );
            return Err(format!("no route found for {mint_in} -> {mint_out} amount_in={amount_in}"));
        };
        let route = match planner::reverify_route_with_exact_quotes(&route, amount_in, &self.state.spot_router, dex) {
            Ok(route) => route,
            Err(failure) => {
                if failure.coolable {
                    self.state.spot_router.mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                    return Err(format!(
                        "exact quote invalidated pool {} (cooling down {} slots)",
                        failure.pool_id,
                        planner::POOL_COOLDOWN_SLOTS,
                    ));
                }
                return Err(format!(
                    "pool {} isn't ready to quote yet (tick-array data still syncing) -- try again shortly",
                    failure.pool_id,
                ));
            }
        };
        log_warn!(
            "leveragedloopv1: spot leg @ slot {}: {} hop{} {} -> {} amount_in={}",
            self.state.last_slot,
            route.hops.len(),
            if route.hops.len() == 1 { "" } else { "s" },
            mint_in,
            mint_out,
            amount_in,
        );
        // Real, live-confirmed bug (2026-08-27): this used to open a
        // *separate* atomic group per hop, which left nothing tying hop
        // N to hop N+1 -- `Wallet::assemble()` was free to split them
        // across separate transactions, and Solana gives no guarantee
        // those land in submission order (already documented on
        // `MAX_TX_SIZE`'s own doc comment, for the same reason). Real
        // outcome: a later hop's swap failed with a genuine on-chain
        // `insufficient funds` because the *earlier* hop's output
        // hadn't actually landed (or landed after) at execution time.
        // One atomic group for the *whole* route closes this: either
        // every hop lands together in one transaction (real ordering
        // guaranteed), or -- if the whole route is too large to fit --
        // `assemble()` sends it as a single oversized transaction that
        // the network cleanly rejects outright (an obvious, immediate
        // failure) rather than silently splitting into a race that
        // sometimes works and sometimes doesn't.
        self.wallet.begin_atomic_group();
        // Real, live-confirmed bug (2026-08-27): `end_atomic_group()`
        // alone does NOT discard instructions already appended by
        // earlier, successful hops when a later hop fails -- it only
        // stops *new* instructions from being glued to the group. A real
        // 4-hop route had hop 0 (a real Raydium CLMM swap) succeed and
        // get queued, then hop 1 fail (`PoolNotReady`); this function
        // returned `Err` and the caller believed nothing happened, but
        // hop 0's swap instruction was still sitting in the queue and
        // landed for real on the next `assemble()` drain -- a genuine
        // $50 swapped into an unintended token. `checkpoint` + a
        // `rollback_to` call on every error path closes this: either the
        // whole route's instructions land together, or none of them do.
        let checkpoint = self.wallet.queue_checkpoint();
        for (i, hop) in route.hops.iter().enumerate() {
            let (Some(source_ata), Some(dest_ata)) = (
                self.wallet.append_create_ata(owner, hop.input_mint),
                self.wallet.append_create_ata(owner, hop.output_mint),
            ) else {
                self.wallet.rollback_to(checkpoint);
                self.wallet.end_atomic_group();
                return Err(format!("hop {i}: FAILED to derive token account(s) for owner={owner}"));
            };
            let hop_result = dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet);
            match hop_result {
                Ok(()) => {
                    log_warn!(
                        "  hop {i}: OK dex={:?} pool={} {} -> {} amount_in={} amount_out={}",
                        hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out,
                    );
                }
                Err(e) => {
                    // Real, live-confirmed gap (2026-08-27): a hop can
                    // fail here (e.g. `TraderError::PoolNotReady` from a
                    // pool with no real tick-array data) even after
                    // `reverify_route_with_exact_quotes` above let it
                    // through -- that pre-flight check doesn't cover
                    // every real failure mode this call does. Without
                    // marking cooldown here too, the next retry's
                    // `route_slippage_aware` would likely pick this same
                    // bad pool again and stall indefinitely rather than
                    // ever trying an alternate route.
                    //
                    // Refined (2026-09-04): `PoolNotReady` specifically
                    // means "not enough live data observed yet" (its own
                    // doc comment), not a genuine bad pool -- a real,
                    // live-confirmed incident found this could be the
                    // *only* viable pool for a route (e.g. a stranded
                    // token with a single tracked pool), where cooling it
                    // down forecloses the only path instead of avoiding a
                    // stall. Skip the cooldown for that specific case;
                    // every other error still gets one, same as before.
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
                        self.state.spot_router.note_pool_not_ready(hop.pool_id)
                    } else {
                        true
                    };
                    if coolable {
                        if is_pool_not_ready {
                            self.state.spot_router.mark_pool_not_ready_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        } else {
                            self.state.spot_router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                    }
                    return Err(format!(
                        "hop {i}: FAILED dex={:?} pool={} {} -> {}: {} (cooling down {} slots: {coolable})",
                        hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, e, planner::POOL_COOLDOWN_SLOTS,
                    ));
                }
            }
        }
        // Real, live-confirmed gap (2026-08-27): a route whose hops all
        // succeed can still be too big to fit in one transaction once
        // every hop's swap + ATA-creation instructions are combined --
        // `Wallet::assemble()` correctly refuses to split an atomic
        // group and the network correctly rejects the oversized attempt,
        // but neither of those marks the route's pools as bad, so the
        // exact same doomed route got reselected on every retry
        // indefinitely (observed for real: the identical 4-hop route,
        // same pool IDs, every ~35s for several minutes straight).
        // Checking fit here, before committing, lets this function reuse
        // the same cooldown-and-return-Err pattern as every other
        // failure mode above.
        if !self.wallet.atomic_group_fits(checkpoint) {
            self.wallet.rollback_to(checkpoint);
            self.wallet.end_atomic_group();
            for hop in route.hops.iter() {
                self.state.spot_router.mark_pool_cooldown(hop.pool_id, planner::POOL_COOLDOWN_SLOTS);
            }
            return Err(format!(
                "route {mint_in} -> {mint_out} amount_in={amount_in}: {}-hop atomic group too large for one transaction (cooling down {} slots)",
                route.hops.len(),
                planner::POOL_COOLDOWN_SLOTS,
            ));
        }
        self.wallet.end_atomic_group();
        // See `pending_route_pools`'s doc comment -- `evaluate()`'s send
        // loop cools these down if `transactionprocessor::send()` itself
        // rejects the transaction this route ends up in.
        self.state.pending_route_pools = route.hops.iter().map(|h| h.pool_id).collect();
        Ok(route.hops.last().map(|h| h.amount_out).unwrap_or(0))
    }

    fn loop_cooldown_active(&self) -> bool {
        match self.state.loop_last_action_slot {
            Some(last) => self.state.last_slot.saturating_sub(last) < LOOP_ACTION_COOLDOWN_SLOTS,
            None => false,
        }
    }

    /// See `o_pending_recover_mint`'s doc comment. Called on every
    /// `evaluate()` tick; a no-op unless a `TriggerRecoverToken` is
    /// waiting on a real balance update for its mint. Independent of
    /// `LoopPhase`/`loop_cooldown_active` (its own separate
    /// `recover_last_action_slot` gate instead) -- this is a one-shot
    /// recovery utility, not part of the loop's own state machine, so it
    /// doesn't participate in either. Retries (cooldown-gated) rather
    /// than giving up after one failed attempt -- real, live-confirmed:
    /// a pre-flight route failure (e.g. `PoolNotReady`) is safe (zero
    /// funds moved) but leaves `execute_spot_leg`'s own cooldown on the
    /// bad pool as the only way the *next* attempt picks something else.
    fn check_pending_recover(&mut self) {
        let Some(mint_pubkey) = self.state.o_pending_recover_mint else {
            return;
        };
        if let Some(last) = self.state.recover_last_action_slot {
            if self.state.last_slot.saturating_sub(last) < LOOP_ACTION_COOLDOWN_SLOTS {
                return;
            }
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let mint = account_id_from_pubkey(&mint_pubkey);
        let balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if balance_raw == 0 {
            return;
        }
        self.state.recover_last_action_slot = Some(self.state.last_slot);
        log_warn!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) real balance found -- swapping {balance_raw} raw back to USDC");
        let mint_usdc = self.configuration.mint_usdc;
        match self.execute_spot_leg(mint, mint_usdc, balance_raw, RECOVER_MAX_HOPS) {
            Ok(usdc_out) => {
                log_warn!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) swap submitted (expect ~{usdc_out} raw USDC)");
                self.state.o_pending_recover_mint = None;
            }
            Err(e) => {
                log_error!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) swap FAILED: {e} -- will retry");
            }
        }
    }

    /// See `o_pending_redeposit_usdc_raw`'s doc comment. Called on every
    /// `evaluate()` tick; a no-op unless a `TriggerRedepositUsdc` is
    /// pending. Independent of `LoopPhase`/`loop_cooldown_active`, its
    /// own `redeposit_last_action_slot` gate instead -- same reasoning
    /// as `check_pending_recover`.
    fn check_pending_redeposit(&mut self) {
        let Some(usdc_amount_raw) = self.state.o_pending_redeposit_usdc_raw else {
            return;
        };
        if let Some(last) = self.state.redeposit_last_action_slot {
            if self.state.last_slot.saturating_sub(last) < LOOP_ACTION_COOLDOWN_SLOTS {
                return;
            }
        }
        self.state.redeposit_last_action_slot = Some(self.state.last_slot);
        if self.redeposit_usdc_as_jitosol_collateral(usdc_amount_raw, RECOVER_MAX_HOPS) {
            log_warn!("leveragedloopv1: TriggerRedepositUsdc deposit submitted");
            self.state.o_pending_redeposit_usdc_raw = None;
        } else {
            log_error!("leveragedloopv1: TriggerRedepositUsdc attempt failed -- will retry");
        }
    }

    /// See `o_pending_open_auto`'s doc comment. Called on every
    /// `evaluate()` tick, same as `check_pending_recover`/
    /// `check_pending_redeposit`; a no-op unless a `TriggerOpenAuto` is
    /// pending. Re-checks `loop_phase` at retry time (not just at
    /// message-receipt time in `on_message`) in case a `TriggerOpen`/
    /// `TriggerClose` raced it while it was waiting on real data --
    /// abandons rather than blindly opening into a phase that's no
    /// longer `Idle`/`Closed`.
    fn check_pending_open_auto(&mut self) {
        let Some(notional_usd) = self.state.o_pending_open_auto else {
            return;
        };
        if let Some(last) = self.state.open_auto_last_action_slot {
            if self.state.last_slot.saturating_sub(last) < LOOP_ACTION_COOLDOWN_SLOTS {
                return;
            }
        }
        if !matches!(self.state.loop_phase, LoopPhase::Idle | LoopPhase::Closed) {
            log_warn!(
                "leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) abandoned -- loop phase changed to {:?} while it was waiting on real data",
                self.state.loop_phase,
            );
            self.state.o_pending_open_auto = None;
            return;
        }
        self.state.open_auto_last_action_slot = Some(self.state.last_slot);
        let Some(decision) = self.dag_best_lst_path() else {
            log_warn!("leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) still waiting on real candidate data -- will retry");
            return;
        };
        if decision.winner == "USDC" || decision.profit_ratio <= 1.0 {
            log_warn!(
                "leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) declined -- DAG says \"do nothing\" beats every real candidate right now (best={} profit_ratio={:.5}, considered: {})",
                decision.winner,
                decision.profit_ratio,
                decision.candidates_used.join(","),
            );
            self.state.o_pending_open_auto = None;
            return;
        }
        let Some(&(symbol, mint)) = LST_CANDIDATES.iter().find(|&&(s, _)| s == decision.winner) else {
            log_error!(
                "leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) -- DAG picked {} but it isn't in LST_CANDIDATES (should never happen)",
                decision.winner,
            );
            self.state.o_pending_open_auto = None;
            return;
        };
        log_warn!(
            "leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) -- DAG picked {symbol} (profit_ratio={:.5}, usdc_borrow_apy={:.3}%) -- opening",
            decision.profit_ratio,
            decision.usdc_borrow_apy * 100.0,
        );
        self.state.active_lst = Some((symbol, mint));
        self.state.requested_notional_usd = Some(notional_usd);
        self.state.o_pending_open_auto = None;
        self.loop_advance(LoopPhase::DepositCollateral);
    }

    fn loop_mark_action(&mut self) {
        self.state.loop_last_action_slot = Some(self.state.last_slot);
    }

    /// Moves to `next`, clearing the cooldown so the new phase starts
    /// its own action fresh.
    fn loop_advance(&mut self, next: LoopPhase) {
        log_warn!("leveragedloopv1: phase {:?} -> {:?}", self.state.loop_phase, next);
        self.state.loop_phase = next;
        self.state.loop_last_action_slot = None;
    }

    /// See [`LoopPhase::DepositCollateral`]. Sizing is estimated from the
    /// Kamino reserve's own oracle price, not the spot swap's real
    /// (slippage-affected) output -- both instructions land in the same
    /// transaction, so a bad estimate fails the deposit on-chain
    /// (insufficient balance) rather than depositing a wrong amount.
    /// Same precedent testperpv1/perpfundingv1's own Kamino deposit legs
    /// already established.
    fn loop_deposit_collateral(&mut self) {
        if !self.state.o_kamino_position.as_ref().is_some_and(|s| s.registered()) {
            if self.loop_cooldown_active() {
                return;
            }
            self.bootstrap_kamino_obligation();
            self.loop_mark_action();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let jitosol_mint = account_id_from_pubkey(&self.state.active_lst().1);
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        let already_deposited = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if already_deposited {
            log_warn!("leveragedloopv1: jitoSOL collateral confirmed deposited on-chain");
            self.loop_advance(LoopPhase::BorrowAndRedeposit);
            return;
        }
        if self.loop_cooldown_active() {
            return;
        }
        // Real, live-confirmed gap (2026-08-27): a real swap can land
        // (USDC -> jitoSOL) but the follow-on Kamino deposit fail for an
        // unrelated reason (e.g. `ReserveStale`), leaving real jitoSOL
        // sitting in the wallet's own ATA, undeposited. Without this
        // check, the next retry would try to swap *again* from USDC --
        // at best wasteful (perfectly good jitoSOL already on hand), at
        // worst impossible: real, live-confirmed, this happened after a
        // real $50 swap left only ~$1.30 USDC, nowhere near enough for a
        // second full-notional swap, permanently stalling this phase.
        // Checking the real ATA balance first and depositing it directly
        // when present unblocks exactly that stuck state.
        let owner_jitosol_balance: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &jitosol_mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, jitosol_mint) else {
            return;
        };
        let real_jitosol_out = if owner_jitosol_balance > 0 {
            log_warn!(
                "leveragedloopv1: real jitoSOL already sitting in the wallet ({owner_jitosol_balance} raw) from an earlier swap -- depositing that instead of swapping again"
            );
            self.loop_mark_action();
            owner_jitosol_balance
        } else {
            let Some(notional_usd) = self.state.requested_notional_usd else {
                log_error!("leveragedloopv1: DepositCollateral phase with no requested notional -- should not happen");
                return;
            };
            let Some(dex) = self.state.o_dex.as_ref() else {
                return;
            };
            let Some((_, reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
                return;
            };
            let price_usd = reserve.price_usd;
            if price_usd <= 0.0 {
                log_error!("leveragedloopv1: jitoSOL has no Kamino oracle price yet, waiting");
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
            log_warn!("leveragedloopv1: opening jitoSOL collateral deposit (${:.2})", notional_usd);
            self.loop_mark_action();
            match self.execute_spot_leg(mint_usdc, jitosol_mint, usdc_amount_raw, LOOP_MAX_HOPS) {
                Ok(amount_out) => amount_out,
                Err(e) => {
                    log_error!("leveragedloopv1: collateral deposit swap failed: {e}");
                    return;
                }
            }
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: collateral deposit refresh failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: collateral deposit refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        if let Err(e) = reserve.deposit(reserve_id, obligation_id, real_jitosol_out, owner, underlying_ata, self.wallet) {
            log_error!("leveragedloopv1: collateral deposit failed: {e}");
        }
    }

    /// See [`LoopPhase::BorrowAndRedeposit`]. Borrows USDC against the
    /// jitoSOL collateral just deposited, sized at
    /// `min(TARGET_BORROW_FRACTION, LTV_SAFETY_CEILING_OF_MAX_LTV *
    /// jitosol_ltv) * LTV_SAFETY_FACTOR / usdc_borrow_factor_pct` of its
    /// real USD value, then immediately swaps the borrowed USDC back to
    /// jitoSOL and deposits again -- both halves batched into this one
    /// function call/phase (mirrors `open_kamino_borrow_leg`'s own
    /// borrow-then-immediately-sell pattern; the redeposit amount is
    /// estimated from oracle price, same "on-chain safety net" precedent
    /// as the initial deposit).
    fn loop_borrow_and_redeposit(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let jitosol_mint = account_id_from_pubkey(&self.state.active_lst().1);
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, _)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        let already_borrowed = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(usdc_reserve_id))
            .is_some_and(|b| b.borrowed_amount != 0);
        if already_borrowed {
            log_warn!("leveragedloopv1: USDC borrow confirmed on-chain, leverage loop step complete");
            self.state.requested_notional_usd = None;
            self.loop_advance(LoopPhase::Open);
            return;
        }
        if self.loop_cooldown_active() {
            return;
        }
        let Some((jitosol_reserve_id, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        let Some((_, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        let Some(deposited_raw) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(jitosol_reserve_id))
            .map(|d| d.deposited_amount)
        else {
            return;
        };
        if deposited_raw == 0 {
            return;
        }
        let price_usd = jitosol_reserve.price_usd;
        if price_usd <= 0.0 {
            return;
        }
        let decimals = jitosol_reserve.mint_decimals as i32;
        let deposited_usd = deposited_raw as f64 / 10f64.powi(decimals) * price_usd;
        let borrow_fraction = borrow_fraction_capped(jitosol_reserve.loan_to_value_pct);
        let target_borrow_usd = deposited_usd * borrow_fraction;
        let protocol_max_borrow_usd =
            deposited_usd * jitosol_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR / usdc_reserve.borrow_factor_pct;
        let borrow_usd = target_borrow_usd.min(protocol_max_borrow_usd);
        const USDC_DECIMALS: i32 = 6;
        let borrow_amount_raw = (borrow_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(usdc_ata) = self.wallet.append_create_ata(owner, mint_usdc) else {
            return;
        };

        log_warn!(
            "leveragedloopv1: borrowing ${borrow_usd:.2} USDC against ${deposited_usd:.2} jitoSOL collateral"
        );
        self.loop_mark_action();
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((jitosol_reserve_id, _)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        let (usdc_lending_market, usdc_farm_debt) = (usdc_reserve.lending_market, usdc_reserve.farm_debt);
        if let Err(e) = self.kamino_refresh_all_reserves(&[jitosol_reserve_id, usdc_reserve_id]) {
            log_error!("leveragedloopv1: borrow refresh failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            usdc_lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: borrow refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        if let Err(e) = usdc_reserve.borrow(
            usdc_reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            usdc_ata,
            None,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: borrow failed: {e}");
            return;
        }
        // Immediately swap the borrowed USDC back to jitoSOL and
        // redeposit, batched into this same call -- see
        // `redeposit_usdc_as_jitosol_collateral`'s doc comment.
        self.redeposit_usdc_as_jitosol_collateral(borrow_amount_raw, LOOP_MAX_HOPS);
    }

    /// Swap `usdc_amount_raw` USDC to jitoSOL and deposit it as
    /// additional Kamino collateral -- the "redeposit" half of
    /// `loop_borrow_and_redeposit`'s own borrow-then-redeposit pattern,
    /// factored out so `TriggerRedepositUsdc` (a manual one-shot
    /// action, independent of `LoopPhase`) can reuse it to finish a
    /// redeposit that failed automatically. Real, live-confirmed need
    /// (2026-08-27): a real `BorrowAndRedeposit` cycle's automatic
    /// redeposit failed on a stale-quote pool cooldown; since
    /// `already_borrowed` short-circuits straight to `Open` on every
    /// later `evaluate()` tick, nothing ever retried it, leaving real
    /// USDC un-redeposited. Checks for jitoSOL already sitting in the
    /// wallet first (same fix `loop_deposit_collateral` needed) -- if a
    /// *previous* call's swap landed but its own deposit step then
    /// failed, retrying should deposit that, not swap more USDC.
    /// Refreshes reserves/obligation again right before depositing (this
    /// exact deposit path had never actually landed live before this
    /// trigger existed -- unlike the rest of this file's Kamino deposit
    /// call sites, which all learned the hard way that skipping this
    /// produces a real `ReserveStale`). Returns `true` only once the
    /// deposit instruction itself has been queued without error --
    /// callers use this to decide whether it's safe to stop retrying.
    fn redeposit_usdc_as_jitosol_collateral(&mut self, usdc_amount_raw: u64, max_hops: usize) -> bool {
        let Some(owner) = self.state.wallet() else {
            return false;
        };
        let Some(obligation_id) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return false;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let jitosol_mint = account_id_from_pubkey(&self.state.active_lst().1);
        let owner_jitosol_balance: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &jitosol_mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        let real_jitosol_out = if owner_jitosol_balance > 0 {
            log_warn!(
                "leveragedloopv1: redeposit: real jitoSOL already sitting in the wallet ({owner_jitosol_balance} raw) -- depositing that instead of swapping again"
            );
            owner_jitosol_balance
        } else {
            match self.execute_spot_leg(mint_usdc, jitosol_mint, usdc_amount_raw, max_hops) {
                Ok(amount_out) => amount_out,
                Err(e) => {
                    log_error!("leveragedloopv1: redeposit swap failed: {e}");
                    return false;
                }
            }
        };
        if real_jitosol_out == 0 {
            return false;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return false;
        };
        let Some((jitosol_reserve_id, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return false;
        };
        let (lending_market, farm_collateral) = (jitosol_reserve.lending_market, jitosol_reserve.farm_collateral);
        if let Err(e) = self.kamino_refresh_all_reserves(&[jitosol_reserve_id]) {
            log_error!("leveragedloopv1: redeposit refresh failed: {e}");
            return false;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: redeposit refresh_obligation failed: {e}");
            return false;
        }
        if !self.ensure_kamino_farm_ready(jitosol_reserve_id, lending_market, farm_collateral, 0) {
            return false;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return false;
        };
        let Some((jitosol_reserve_id, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return false;
        };
        if let Err(e) = jitosol_reserve.deposit(
            jitosol_reserve_id,
            obligation_id,
            real_jitosol_out,
            owner,
            self.wallet.derive_ata(owner, jitosol_mint).unwrap_or_default(),
            self.wallet,
        ) {
            log_error!("leveragedloopv1: redeposit failed: {e}");
            return false;
        }
        true
    }

    /// See [`LoopPhase::DeleverageWithdrawAndRepay`]. Same two-stage
    /// split `close_kamino_borrow_leg` (testperpv1/perpfundingv1) uses:
    /// if the wallet doesn't already hold enough USDC to cover the real
    /// outstanding debt, withdraw just enough jitoSOL collateral to
    /// cover the shortfall (a real, live-computed amount -- never more
    /// than needed), swap it to USDC, and defer the repay to the next
    /// cycle rather than racing the just-queued swap (same ordering bug
    /// `close_solend_borrow_leg`/`close_kamino_borrow_leg` already fixed
    /// for real). Once enough USDC is confirmed on hand, repay in full.
    fn loop_deleverage_withdraw_and_repay(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let jitosol_mint = account_id_from_pubkey(&self.state.active_lst().1);
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, _)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        // Real, live-confirmed gap (2026-08-27): `TriggerClose` can now
        // fire from `LoopPhase::Idle` (see its own `on_message` doc
        // comment), which -- unlike the old `Open`-only path -- gives no
        // guarantee a real obligation account update has arrived yet.
        // The `else` branch below intentionally treats "no borrow
        // recorded for this reserve" as "debt repaid," but that's only
        // true once `obligation()` itself is confirmed `Some` -- if it's
        // still `None` (data simply hasn't arrived), the old code raced
        // straight to `DeleverageWithdrawRest`/`Closed` believing
        // nothing needed to be done, without ever sending a real
        // repay/withdraw. Waiting here for real data first closes that.
        if self.state.o_kamino_position.as_ref().and_then(|s| s.obligation()).is_none() {
            return;
        }
        let Some(borrowed_amount_raw) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(usdc_reserve_id))
            .map(|b| b.borrowed_amount)
            .filter(|&a| a != 0)
        else {
            log_warn!("leveragedloopv1: USDC debt confirmed repaid on-chain");
            self.loop_advance(LoopPhase::DeleverageWithdrawRest);
            return;
        };
        let usdc_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint_usdc, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if usdc_balance_raw < borrowed_amount_raw {
            if self.loop_cooldown_active() {
                return;
            }
            let Some((_, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
                return;
            };
            let jitosol_price = jitosol_reserve.price_usd;
            if jitosol_price <= 0.0 {
                return;
            }
            const USDC_DECIMALS: i32 = 6;
            let shortfall_usd =
                (borrowed_amount_raw - usdc_balance_raw) as f64 / 10f64.powi(USDC_DECIMALS) * DELEVERAGE_WITHDRAW_BUFFER;
            let decimals = jitosol_reserve.mint_decimals as i32;
            let withdraw_raw = ((shortfall_usd / jitosol_price) * 10f64.powi(decimals)).round() as u64;
            if withdraw_raw == 0 {
                return;
            }
            let Some(jitosol_ata) = self.wallet.derive_ata(owner, jitosol_mint) else {
                return;
            };
            log_warn!("leveragedloopv1: withdrawing ${shortfall_usd:.2} jitoSOL collateral to repay USDC debt");
            self.loop_mark_action();
            let Some(dex) = self.state.o_dex.as_ref() else {
                return;
            };
            let Some((jitosol_reserve_id, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
                return;
            };
            let (jitosol_lending_market, jitosol_farm_collateral) =
                (jitosol_reserve.lending_market, jitosol_reserve.farm_collateral);
            if let Err(e) = self.kamino_refresh_all_reserves(&[jitosol_reserve_id]) {
                log_error!("leveragedloopv1: deleverage withdraw refresh failed: {e}");
                return;
            }
            let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
            if let Err(e) = kamino::refresh_obligation(
                jitosol_lending_market,
                obligation_id,
                &deposit_reserves,
                &borrow_reserves,
                self.wallet,
            ) {
                log_error!("leveragedloopv1: deleverage withdraw refresh_obligation failed: {e}");
                return;
            }
            if !self.ensure_kamino_farm_ready(jitosol_reserve_id, jitosol_lending_market, jitosol_farm_collateral, 0) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return;
            };
            let Some((jitosol_reserve_id, jitosol_reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
                return;
            };
            if let Err(e) = jitosol_reserve.withdraw(jitosol_reserve_id, obligation_id, withdraw_raw, owner, jitosol_ata, self.wallet) {
                log_error!("leveragedloopv1: deleverage withdraw failed: {e}");
                return;
            }
            if let Err(e) = self.execute_spot_leg(jitosol_mint, mint_usdc, withdraw_raw, LOOP_MAX_HOPS) {
                log_error!("leveragedloopv1: deleverage withdraw-to-USDC swap failed: {e}");
            }
            return;
        }
        if self.loop_cooldown_active() {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        log_warn!("leveragedloopv1: repaying USDC debt in full");
        self.loop_mark_action();
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        let (usdc_lending_market, usdc_farm_debt) = (usdc_reserve.lending_market, usdc_reserve.farm_debt);
        if let Err(e) = self.kamino_refresh_all_reserves(&[usdc_reserve_id]) {
            log_error!("leveragedloopv1: repay refresh failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            usdc_lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: repay refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(usdc_reserve_id, usdc_lending_market, usdc_farm_debt, 1) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        if let Err(e) = usdc_reserve.repay(usdc_reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, usdc_ata, self.wallet) {
            log_error!("leveragedloopv1: repay failed: {e}");
        }
    }

    /// See [`LoopPhase::DeleverageWithdrawRest`]. Withdraws all
    /// remaining jitoSOL collateral once the debt is confirmed gone --
    /// left as jitoSOL, not swapped back to USDC (either is safe once
    /// debt is zero; this bot doesn't need the USDC for anything else).
    fn loop_deleverage_withdraw_rest(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self.state.o_kamino_position.as_ref().and_then(|s| s.obligation_id()) else {
            return;
        };
        let jitosol_mint = account_id_from_pubkey(&self.state.active_lst().1);
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        // See `loop_deleverage_withdraw_and_repay`'s identical guard's
        // doc comment -- same real gap, same fix: don't treat "no real
        // obligation data yet" as "confirmed empty."
        if self.state.o_kamino_position.as_ref().and_then(|s| s.obligation()).is_none() {
            return;
        }
        let has_deposit = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if !has_deposit {
            log_warn!("leveragedloopv1: jitoSOL collateral confirmed fully withdrawn -- position closed");
            // Defensive reset (2026-08-28): in practice every real
            // trigger this whole session has been sent to a fresh
            // process (`active_lst` would already be `None` on a fresh
            // `State`), but reset explicitly anyway so a long-running
            // process that closed a TriggerOpenAuto-opened non-jitoSOL
            // position can't leak that choice into a later plain
            // TriggerOpen.
            self.state.active_lst = None;
            self.loop_advance(LoopPhase::Closed);
            return;
        }
        if self.loop_cooldown_active() {
            return;
        }
        let Some(jitosol_ata) = self.wallet.derive_ata(owner, jitosol_mint) else {
            return;
        };
        log_warn!("leveragedloopv1: withdrawing all remaining jitoSOL collateral");
        self.loop_mark_action();
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        let (lending_market, farm_collateral) = (reserve.lending_market, reserve.farm_collateral);
        if let Err(e) = self.kamino_refresh_all_reserves(&[reserve_id]) {
            log_error!("leveragedloopv1: withdraw-rest refresh failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("leveragedloopv1: withdraw-rest refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(reserve_id, lending_market, farm_collateral, 0) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(jitosol_mint) else {
            return;
        };
        if let Err(e) = reserve.withdraw(reserve_id, obligation_id, kamino::KAMINO_AMOUNT_MAX, owner, jitosol_ata, self.wallet) {
            log_error!("leveragedloopv1: withdraw-rest failed: {e}");
        }
    }

    pub(crate) fn evaluate(&mut self) {
        if self.state.o_dex.is_none() || self.state.o_kamino_position.is_none() {
            return;
        }
        self.check_pending_recover();
        self.check_pending_redeposit();
        self.check_pending_open_auto();
        self.check_pending_close_all_basis();
        // Basis-trade strategy (2026-08-29) -- fully independent of
        // LoopPhase below, gated on TriggerEnableBasisTrading having
        // fired (see that variant's doc comment) and o_phoenix actually
        // being ready. Same real epoch-boundary dispatch shape as
        // perpfundingv1::state::evaluate's own tail.
        if self.state.basis_trading_enabled && self.state.o_phoenix.is_some() {
            let epoch_ts = Self::current_basis_epoch_ts();
            match self.state.basis_last_epoch_ts {
                None => {
                    self.state.basis_last_epoch_ts = Some(epoch_ts);
                    self.basis_observe_all(epoch_ts);
                }
                Some(pending) if epoch_ts > pending => {
                    self.run_basis_cycle();
                    self.state.router.close_epoch(self.state.last_slot, pending);
                    self.state.basis_last_epoch_ts = Some(epoch_ts);
                    self.basis_observe_all(epoch_ts);
                }
                Some(_) => {
                    self.basis_observe_all(epoch_ts);
                }
            }
        }
        match self.state.loop_phase {
            LoopPhase::Idle => {}
            LoopPhase::DepositCollateral => self.loop_deposit_collateral(),
            LoopPhase::BorrowAndRedeposit => self.loop_borrow_and_redeposit(),
            LoopPhase::Open => {}
            LoopPhase::DeleverageWithdrawAndRepay => self.loop_deleverage_withdraw_and_repay(),
            LoopPhase::DeleverageWithdrawRest => self.loop_deleverage_withdraw_rest(),
            LoopPhase::Closed => {}
        }
        // Drains whatever this evaluate() call (or on_message's Wallet
        // arm) built onto self.wallet and actually sends it -- same tail
        // every other bot mode's evaluate() uses. Real, live-confirmed
        // gap this closes (2026-08-27): `transactionprocessor::send()`
        // rejecting a transaction (any reason -- an oversized atomic
        // group used to be the only one ever observed, now pre-empted by
        // `atomic_group_fits`, but nothing guarantees it's the only real
        // one) never fed back into `mark_pool_cooldown` at all, so a
        // route whose transaction the host rejects at send time could
        // get reselected and resent unchanged forever. On any send
        // failure this cycle, `pending_route_pools` is cooled down AND
        // cleared right here -- a transaction that never made it to the
        // network will never produce a real confirmation event, so
        // there's nothing for `mid_on_tx` to wait for. If every send
        // succeeds, `pending_route_pools` is deliberately left alone --
        // `mid_on_tx` (see its own doc comment) is what answers whether
        // the *sent* transaction actually landed or reverted on-chain,
        // asynchronously, once the validator's real result arrives.
        let mut any_send_failed = false;
        for (sig, result) in self.wallet.drain_and_send() {
            match result {
                Ok(_) => log_warn!("leveragedloopv1: sent transaction {sig}"),
                Err(e) => {
                    log_error!("leveragedloopv1: failed to send transaction {sig}: {e}");
                    any_send_failed = true;
                }
            }
        }
        if any_send_failed && !self.state.pending_route_pools.is_empty() {
            for &pool_id in &self.state.pending_route_pools {
                self.state.spot_router.mark_pool_cooldown(pool_id, planner::POOL_COOLDOWN_SLOTS);
            }
            self.state.pending_route_pools.clear();
        }
    }
}

/// Caps `TARGET_BORROW_FRACTION` at `LTV_SAFETY_CEILING_OF_MAX_LTV *
/// jitosol_ltv` -- pure function so its behavior at the boundary (and
/// with a hypothetically lower real LTV than today's 63%) is directly
/// unit-testable without any live chain state.
fn borrow_fraction_capped(jitosol_ltv: f64) -> f64 {
    TARGET_BORROW_FRACTION.min(LTV_SAFETY_CEILING_OF_MAX_LTV * jitosol_ltv)
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(std::time::SystemTime::now()));
            }
            MessageAction::AdjustConfiguration(new_configuration) => {
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(x) => match x {
                CustomMessageInbound::Blank => {}
                CustomMessageInbound::Wallet(rc_keypair) => {
                    let keypair = rc_unlock(&rc_keypair);
                    let pubkey = keypair.pubkey();
                    let account_id = account_id_from_pubkey(&pubkey);
                    log_warn!("leveragedloopv1: got wallet keypair {pubkey} {account_id}");
                    self.wallet.append_key(rc_keypair.clone(), self.graph).unwrap();
                    self.wallet.set_payer(account_id);
                    self.configuration.set(&rc_keypair);
                    self.state.o_rc_keypair.replace(KeypairExtra { rc_keypair, account_id });

                    let kamino_reqs = self
                        .state
                        .o_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, 0))
                        .unwrap_or_default();
                    // Basis-trade strategy's own two authority-scoped
                    // items -- its PhoenixState trader/margin account and
                    // its independent (id=1) Kamino obligation, batched
                    // into this same subscribe_now call for the same
                    // ~26s-stall-avoidance reason documented on
                    // `KaminoPosition::authority_subscribe_requests`.
                    let phoenix_reqs = self
                        .state
                        .o_phoenix
                        .as_ref()
                        .map(|p| p.authority_subscribe_requests(pubkey))
                        .unwrap_or_default();
                    let basis_kamino_reqs = self
                        .state
                        .o_basis_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, 1))
                        .unwrap_or_default();
                    let mut hs_ata_mints: HashSet<AccountId> = HashSet::new();
                    hs_ata_mints.insert(self.configuration.mint_usdc);
                    // Every real LST_CANDIDATES entry, not just jitoSOL --
                    // whichever one TriggerOpenAuto's DAG later picks
                    // needs its ATA already subscribed at wallet-load
                    // time (this codebase's own established gotcha: an
                    // on-demand subscription only registers, the real
                    // balance arrives asynchronously later -- see
                    // `o_pending_recover_mint`'s doc comment -- so
                    // subscribing only once a candidate is chosen would
                    // leave that phase blind to its own real balance for
                    // however long the first update takes to arrive).
                    // Batched into the same subscribe_now call as
                    // everything else below -- 2 extra mints costs
                    // nothing extra in round-trips.
                    for &(_, mint) in LST_CANDIDATES {
                        hs_ata_mints.insert(account_id_from_pubkey(&mint));
                    }
                    let ata_reqs: Vec<_> = hs_ata_mints
                        .into_iter()
                        .filter_map(|mint| self.wallet.ata_subscribe_request(account_id, mint))
                        .collect();
                    // This wallet's own durable-nonce account (see
                    // `Wallet::send_bundler_pair`'s doc comment) --
                    // batched into the same subscribe_now call as
                    // everything else above, not a separate round-trip.
                    let nonce_reqs: Vec<_> =
                        self.wallet.nonce_subscribe_request(account_id).into_iter().collect();
                    let kamino_len = kamino_reqs.len();
                    let phoenix_len = phoenix_reqs.len();
                    let basis_kamino_len = basis_kamino_reqs.len();
                    let ata_len = ata_reqs.len();
                    let nonce_len = nonce_reqs.len();
                    let mut all_requests = Vec::with_capacity(
                        kamino_len + phoenix_len + basis_kamino_len + ata_len + nonce_len,
                    );
                    all_requests.extend(kamino_reqs);
                    all_requests.extend(phoenix_reqs);
                    all_requests.extend(basis_kamino_reqs);
                    all_requests.extend(ata_reqs);
                    all_requests.extend(nonce_reqs);
                    match SubscriptionQueue::subscribe_now(self.graph, all_requests) {
                        Ok(subs) => {
                            let mut it = subs.into_iter();
                            if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
                                let take: Vec<_> = (&mut it).take(kamino_len).collect();
                                kamino_position.apply_authority(pubkey, 0, take);
                            }
                            if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                                let take: Vec<_> = (&mut it).take(phoenix_len).collect();
                                phoenix.apply_authority(pubkey, take);
                            }
                            if let Some(basis_kamino_position) = self.state.o_basis_kamino_position.as_mut() {
                                let take: Vec<_> = (&mut it).take(basis_kamino_len).collect();
                                basis_kamino_position.apply_authority(pubkey, 1, take);
                            }
                            let ata_subs: Vec<_> = (&mut it).take(ata_len).collect();
                            self.wallet.keep_ata_subscriptions(ata_subs);
                            if let Some(sub) = (&mut it).take(nonce_len).next() {
                                self.wallet.keep_nonce_subscription(sub);
                            }
                        }
                        Err(e) => {
                            log_error!("leveragedloopv1: failed to batch-subscribe wallet authority accounts: {e}");
                        }
                    }
                }
                CustomMessageInbound::TriggerOpen(notional_usd) => match self.state.loop_phase {
                    LoopPhase::Idle | LoopPhase::Closed => {
                        log_warn!("leveragedloopv1: TriggerOpen received: ${notional_usd:.2} notional -- opening");
                        self.state.requested_notional_usd = Some(notional_usd);
                        self.loop_advance(LoopPhase::DepositCollateral);
                    }
                    other => {
                        log_warn!("leveragedloopv1: TriggerOpen ignored -- loop already in phase {other:?}");
                    }
                },
                CustomMessageInbound::TriggerOpenAuto(notional_usd) => match self.state.loop_phase {
                    LoopPhase::Idle | LoopPhase::Closed => {
                        log_warn!(
                            "leveragedloopv1: TriggerOpenAuto(${notional_usd:.2}) received -- waiting for real candidate data to decide"
                        );
                        self.state.o_pending_open_auto = Some(notional_usd);
                    }
                    other => {
                        log_warn!("leveragedloopv1: TriggerOpenAuto ignored -- loop already in phase {other:?}");
                    }
                },
                CustomMessageInbound::TriggerClose => match self.state.loop_phase {
                    // Real, live-confirmed gap (2026-08-27): `LoopPhase`
                    // is purely in-memory and never persists across
                    // process restarts -- every real trigger this whole
                    // session has been sent to a *fresh* process (launch,
                    // send one trigger, kill), so a real, fully-open
                    // position's phase is `Idle` again by the time
                    // `--close` runs in its own new process, and
                    // `TriggerClose` was unconditionally ignored. Safe to
                    // accept from `Idle` too: `loop_deleverage_withdraw_
                    // and_repay`/`loop_deleverage_withdraw_rest` already
                    // re-derive everything from real on-chain obligation
                    // state (never assume anything about how the phase
                    // got here), so this is idempotent even if triggered
                    // with nothing actually open -- it just confirms
                    // "already repaid"/"already withdrawn" and reaches
                    // `Closed` with zero real actions.
                    LoopPhase::Open | LoopPhase::Idle => {
                        log_warn!("leveragedloopv1: TriggerClose received -- closing");
                        self.loop_advance(LoopPhase::DeleverageWithdrawAndRepay);
                    }
                    other => {
                        log_warn!("leveragedloopv1: TriggerClose ignored -- loop not in Open/Idle phase (currently {other:?})");
                    }
                },
                CustomMessageInbound::TriggerRecoverToken(mint_pubkey) => {
                    let Some(owner) = self.state.wallet() else {
                        log_warn!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) ignored -- no wallet yet");
                        return;
                    };
                    let mint = account_id_from_pubkey(&mint_pubkey);
                    // Real, live-confirmed gap (2026-08-27, first version
                    // of this trigger, hardcoded to mSOL): a mint this
                    // bot never otherwise touches has no subscribed ATA
                    // data, so `token_mut().balance()` always reports
                    // zero even when a real balance sits in the wallet.
                    // Subscribing on demand here (same pattern the
                    // `Wallet` arm above uses at connect time, just for
                    // one mint instead of the fixed set) is what makes
                    // this trigger genuinely generic over *any* mint,
                    // not just ones anticipated in advance. The real
                    // balance check happens later, in
                    // `check_pending_recover` -- see
                    // `o_pending_recover_mint`'s doc comment for why it
                    // can't happen synchronously right here.
                    if let Some(req) = self.wallet.ata_subscribe_request(owner, mint) {
                        match SubscriptionQueue::subscribe_now(self.graph, vec![req]) {
                            Ok(subs) => self.wallet.keep_ata_subscriptions(subs),
                            Err(e) => {
                                log_error!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) failed to subscribe: {e}");
                                return;
                            }
                        }
                    }
                    log_warn!("leveragedloopv1: TriggerRecoverToken({mint_pubkey}) received -- subscribed, waiting for a real balance update");
                    self.state.o_pending_recover_mint = Some(mint_pubkey);
                }
                CustomMessageInbound::TriggerRedepositUsdc(notional_usd) => {
                    if self.state.wallet().is_none() {
                        log_warn!("leveragedloopv1: TriggerRedepositUsdc(${notional_usd:.2}) ignored -- no wallet yet");
                        return;
                    }
                    const USDC_DECIMALS: i32 = 6;
                    let usdc_amount_raw = (notional_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
                    if usdc_amount_raw == 0 {
                        log_warn!("leveragedloopv1: TriggerRedepositUsdc(${notional_usd:.2}) ignored -- rounds to zero raw USDC");
                        return;
                    }
                    log_warn!("leveragedloopv1: TriggerRedepositUsdc(${notional_usd:.2}) received -- will swap to jitoSOL and deposit as collateral");
                    self.state.o_pending_redeposit_usdc_raw = Some(usdc_amount_raw);
                }
                CustomMessageInbound::TriggerTestBundler => {
                    let Some(owner) = self.state.wallet() else {
                        log_warn!("leveragedloopv1: TriggerTestBundler ignored -- no wallet yet");
                        return;
                    };
                    if self.wallet.ensure_bundler_nonce_created(owner) {
                        log_warn!(
                            "leveragedloopv1: TriggerTestBundler -- durable-nonce account not ready yet, queued its (real, one-time) creation transaction"
                        );
                        return;
                    }
                    let Some(owner_pubkey) = pubkey_from_account_id(&owner) else {
                        log_warn!("leveragedloopv1: TriggerTestBundler ignored -- could not resolve wallet pubkey");
                        return;
                    };
                    // Trivial, deliberately inert real instruction (a
                    // self-transfer) -- this trigger only exists to prove
                    // the durable-nonce dual-transaction pipe end-to-end
                    // before wiring it into any real trading operation.
                    //
                    // Real, live-confirmed gap (2026-08-28): `send_bundler_pair`
                    // declining does NOT drain/clear whatever's already
                    // queued -- that's this call site's job (same
                    // checkpoint/rollback discipline `execute_spot_leg`
                    // already uses), since `send_bundler_pair` has no way
                    // to tell "caller-owned instructions I should leave
                    // alone" apart from "instructions meant only for this
                    // bundle." Without the checkpoint/rollback below, a
                    // decline left this self-transfer sitting in the
                    // queue and the very next ordinary `drain_and_send()`
                    // call sent it anyway -- harmless for this trivial
                    // instruction, but exactly the kind of silent,
                    // untipped, non-bundled fallback a real caller must
                    // never risk.
                    let checkpoint = self.wallet.queue_checkpoint();
                    self.wallet.require_signer(owner);
                    self.wallet.append_ix(
                        solana_system_interface::instruction::transfer(&owner_pubkey, &owner_pubkey, 5_000),
                        150,
                    );
                    match self.wallet.send_bundler_pair(crate::bundler_config::BUNDLER) {
                        Some((sig, Ok(()))) => {
                            log_warn!("leveragedloopv1: TriggerTestBundler -- real dual-transaction bundle sent ({sig})");
                        }
                        Some((sig, Err(e))) => {
                            self.wallet.rollback_to(checkpoint);
                            log_error!("leveragedloopv1: TriggerTestBundler -- send_bundler_pair failed ({sig}): {e:?}");
                        }
                        None => {
                            self.wallet.rollback_to(checkpoint);
                            log_warn!(
                                "leveragedloopv1: TriggerTestBundler -- send_bundler_pair declined (nonce not ready yet, or bundler {} has no known tip account)",
                                crate::bundler_config::BUNDLER,
                            );
                        }
                    }
                }
                CustomMessageInbound::TriggerTestBatch => {
                    let Some(owner) = self.state.wallet() else {
                        log_warn!("leveragedloopv1: TriggerTestBatch ignored -- no wallet yet");
                        return;
                    };
                    match self.wallet.test_send_two_system_transfers(owner, Some(crate::bundler_config::BUNDLER)) {
                        Some(Ok(())) => {
                            log_warn!("leveragedloopv1: TriggerTestBatch -- 2 separate transactions sent via transactionprocessor::batch");
                        }
                        Some(Err(e)) => {
                            log_error!("leveragedloopv1: TriggerTestBatch -- batch failed: {e:?}");
                        }
                        None => {
                            log_warn!(
                                "leveragedloopv1: TriggerTestBatch ignored -- wallet key not loaded yet, or something else is already queued this tick"
                            );
                        }
                    }
                }
                CustomMessageInbound::LstApy(mint, staking_apy) => {
                    let label = LST_CANDIDATES
                        .iter()
                        .find(|&&(_, m)| m == mint)
                        .map(|&(label, _)| label);
                    log_warn!(
                        "leveragedloopv1: LST staking APY update: {}={:.3}%",
                        label.unwrap_or("(untracked mint)"),
                        staking_apy * 100.0,
                    );
                    self.state.lst_staking_apy.insert(mint, staking_apy);
                    self.log_dag_lst_projection();
                }
                CustomMessageInbound::CommonBundlerTipUpdate(update) => {
                    self.wallet.apply_bundler_tip_update(self.graph, update);
                }
                CustomMessageInbound::TriggerEnableBasisTrading => {
                    if self.state.basis_trading_enabled {
                        log_warn!("leveragedloopv1: basis: TriggerEnableBasisTrading ignored -- already enabled");
                    } else {
                        log_warn!("leveragedloopv1: basis: TriggerEnableBasisTrading -- real funding-rate basis trading enabled");
                        self.state.basis_trading_enabled = true;
                    }
                }
                CustomMessageInbound::TriggerCloseAllBasisPositions => {
                    log_warn!("leveragedloopv1: basis: TriggerCloseAllBasisPositions received");
                    self.state.o_pending_close_all_basis = true;
                }
            },
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        assert!(self.o_commit_slot.replace(slot).is_none());
        self.state.last_slot = slot;
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        self.wallet.on_account(header, body);
        if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
            kamino_position.on_account(header, body);
        }
        if let Some(dex) = self.state.o_dex.as_mut() {
            dex.on_account(header, body);
            dex.refresh_account_router(header.accountid, &mut self.state.spot_router);
        }
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        self.wallet.token_mut().on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        self.wallet.set_priority_fee(PriorityLevel::Medium);
        // Idempotent/self-latching (2026-08-28): only actually queues the
        // real create-nonce transaction once (Wallet::ensure_bundler_nonce_created
        // no-ops on every call after the first, whether still unconfirmed
        // or already Ready) -- safe to call unconditionally every tick so
        // the durable-nonce account this wallet's Astralane landing path
        // (Wallet::send_bundler_pair, driven by set_priority_fee(High) --
        // see its own doc comment) needs is bootstrapped automatically at
        // wallet load time instead of requiring a manual trigger.
        //
        // Kept: TriggerTestBundler's own explicit
        // ensure_bundler_nonce_created call elsewhere in this file --
        // redundant with this now (both just no-op once Ready/CreationQueued),
        // left as-is since removing it isn't necessary for correctness.
        if let Some(owner) = self.state.wallet() {
            self.wallet.ensure_bundler_nonce_created(owner);
        }
        if !self.state.lst_staking_apy.is_empty() {
            let ready = match self.state.dag_projection_last_log_slot {
                None => true,
                Some(last) => {
                    DAG_PROJECTION_LOG_COOLDOWN_SLOTS <= self.state.last_slot.saturating_sub(last)
                }
            };
            if ready {
                self.state.dag_projection_last_log_slot = Some(self.state.last_slot);
                self.log_dag_lst_projection();
            }
        }
        if let Some(mut dex) = self.state.o_dex.take() {
            if let Err(e) = dex.flush_pool(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("leveragedloopv1: failed to flush dex subscriptions: {e}");
            }
            if let Err(e) = dex.flush_subscriptions(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("leveragedloopv1: failed to flush dex subscription queue: {e}");
            }
            self.state.o_dex.replace(dex);
        }
        match self.state.subscription_queue.flush(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
            Ok(0) => {}
            Ok(n) => {
                log_warn!(
                    "leveragedloopv1: subscription_queue flushed {n} requests ({} still pending, {} active)",
                    self.state.subscription_queue.pending_count(),
                    self.state.subscription_queue.active_count(),
                );
            }
            Err(e) => log_error!("leveragedloopv1: subscription_queue flush failed: {e}"),
        }
    }
}

/// Cap on how many queued subscription requests `subscription_queue`
/// sends per slot -- same rationale/value as every other bot mode's own
/// constant of the same name.
const MAX_SUBSCRIBES_PER_SLOT: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_fraction_uses_target_when_under_ceiling() {
        // jitoSOL's real LTV (63%) -> ceiling 31.5%, above the 30% target.
        assert_eq!(borrow_fraction_capped(0.63), TARGET_BORROW_FRACTION);
    }

    #[test]
    fn borrow_fraction_caps_when_ltv_drops_low_enough() {
        // A hypothetical lower LTV (50%) -> ceiling 25%, below the 30% target.
        let capped = borrow_fraction_capped(0.50);
        assert!((capped - 0.25).abs() < 1e-9);
        assert!(capped < TARGET_BORROW_FRACTION);
    }

    #[test]
    fn leverage_from_single_step_borrow_fraction() {
        // leverage = 1 + borrow_fraction; 0.30 -> 1.3x, matching the
        // user's chosen target.
        let leverage = 1.0 + TARGET_BORROW_FRACTION;
        assert!((leverage - 1.3).abs() < 1e-9);
    }

    #[test]
    fn loop_phase_default_is_idle() {
        assert_eq!(LoopPhase::default(), LoopPhase::Idle);
    }
}
