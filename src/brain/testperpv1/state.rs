//! Reactive decision loop for a Phoenix-perp-funding-vs-lending-rate basis
//! trade -- mirrors `arbv1::state`'s shape
//! (same `StateHelper`/`CommitHook`/`evaluate` pattern). Feeds
//! `trader::perp_router::PerpRouter` from Phoenix's existing read-only
//! pricing data (`trader::dex::phoenix::PhoenixState`), closes out a
//! `GraphLayer` on real hourly epoch boundaries (`SystemTime::now()` --
//! confirmed already used and working in every other bot mode this
//! session, not a new/unverified capability), and logs the result.
//!
//! **Not** a Phoenix-vs-Velocity/Drift funding-rate arb (an earlier pass
//! this file went through) -- Drift's real order flow moved to an
//! off-chain "Swift" relayer this bot's WIT host interface can't reach,
//! so that path is retired. The second leg of every position here is a
//! deposit or borrow against a lending protocol -- Solend
//! (`trader::dex::solend`) or Kamino (`trader::dex::kamino`), whichever
//! offers the better rate for a given symbol (marginfi deferred -- its
//! cached prefetch data looked stale and a live re-fetch needs
//! infrastructure this environment doesn't have configured) -- not a
//! second perp venue. See `decide_basis_trade`'s doc comment for the real
//! trade structure.
//!
//! Also carries a spot-market execution hook (`o_dex`/`spot_router`,
//! `execute_spot_leg`), mirroring `arbv1::state`'s `o_dex`/`router`/
//! `build_execution_plan` pattern -- builds and can send a real
//! transaction. Used both by `rebalance_portfolio` and by this file's
//! Solend-hedge legs (swapping the underlying asset in/out of USDC).
use crate::{
    atl_config,
    brain::testperpv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    drift_config,
    err::CatscopeGuestError,
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate, SubscriptionQueue},
    jet_config, kamino_config, log_error, log_info, log_warn, marginfi_config,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    orca_config, phoenix_config, pumpfun_config, pumpswap_config, raydium_amm_config,
    raydium_clmm_config, raydium_cpmm_config, router_config, router_pools_config, sanctum_config,
    solend_config, symbol_mint_config, target_allocation_config, top_pools_config,
    tracked_accounts_config,
    trader::{
        dex::{
            ember, kamino, marginfi,
            phoenix::{ix::Side, PhoenixState},
            solend,
            update::Updater as _,
            DexState,
        },
        perp_router::{PerpRouter, PerpVenue},
        planner,
        pricegraph::TradeRouter,
        router,
    },
    trading_config,
    txview::TransactionList,
    util::{
        account_id_from_pubkey, pubkey_from_account_id, rc_unlock, resolve_symbol_decimals,
        resolve_symbol_mint,
    },
    wallet::{PriorityLevel, Wallet},
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use solana_system_interface::instruction::transfer as system_transfer;
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

/// Both Phoenix and Velocity settle funding on a real hourly cadence --
/// verified from each protocol's own source this session (see
/// `trader::perp_router`'s module doc), not a bot-side choice.
const SECONDS_PER_EPOCH: i64 = 3600;

/// Builds the 3-tier build-time liquidity router from `router_pools_config`
/// (the `ROUTER_POOLS` snapshot embedded at compile time) -- copied
/// verbatim from `arbv1::state::build_liquidity_router` rather than
/// shared, matching this codebase's established convention of copying
/// small per-mode boilerplate instead of factoring it out. Only used to
/// seed `State::spot_router`'s node set once, in `StateHelper::on_load`
/// -- see `TradeRouter::from_router`'s doc comment for why that seeding
/// step is required at all.
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

#[derive(Debug)]
struct KeypairExtra {
    #[allow(dead_code)]
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}

/// One symbol's target allocation, join-keyed to its mint via
/// `resolve_symbol_mint` -- `account_id` is `None` until resolved.
/// Resolution can't happen at `State::default()` time: `resolve_symbol_mint`
/// bottoms out in `account_id_from_pubkey`, a WIT host import that only
/// works inside the real WASM guest runtime (see `util::resolve_symbol_mint`'s
/// own doc and its native-test caveat) -- `Default::default()` must stay
/// safe to construct in a native unit test, so resolution instead happens
/// in `on_message` at the same point `Configuration::set`'s own
/// `mint_sol`/`mint_usdc` resolution already does (the `Wallet` arm --
/// the first point in this file's message flow confirmed to be inside a
/// live WASM guest), plus per-entry in the `TargetAllocation` arm itself
/// for runtime updates.
#[derive(Debug, Clone, Copy)]
struct TargetAllocationEntry {
    account_id: Option<AccountId>,
    allocation_pct: f64,
}

/// The real-transaction smoke test this whole module exists to run --
/// see `mod.rs`'s doc comment for why. A strict linear sequence, driven
/// by `StateHelper::evaluate` on every event: swap enough native SOL
/// (the child wallet's only funded asset -- see `eval.go`'s boot
/// transfer on the Go side) into USDC via `TradeRouter`/
/// `execute_spot_leg` to cover all three protocols' deposit tests, then
/// bootstrap each lending protocol's obligation, deposit a small amount
/// of USDC, withdraw it, move on. `Solend` before `Kamino` before
/// `Marginfi` only because that's the order the user asked for -- no
/// other significance. Once every protocol's deposit/withdraw pair has
/// been exercised, a second pass borrows and repays a small SOL position
/// against each -- the other half of the real basis-trade hedge legs
/// (`open_*_borrow_leg`/`close_*_borrow_leg`), never exercised by the
/// deposit/withdraw pass above and previously never verified against a
/// real transaction at all, for any of the three protocols.
///
/// **`SwapToUsdc` wraps native SOL into wSOL first.** `execute_spot_leg`
/// routes over SPL token balances (ATAs), not raw native lamports, and
/// nothing else in this codebase funds USDC directly -- so this phase
/// idempotently creates the wSOL ATA, moves raw lamports into it via a
/// System Program transfer, and issues `SyncNative` (hand-built --
/// the pinned `spl-token` crate has no client-side builder for it) to
/// make the wrapped balance visible to the token database, all batched
/// into the same transaction as the swap itself.
///
/// **Bootstrap/deposit phases retry-until-confirmed** (check real
/// on-chain state via the already-tracked `SolendPosition`/
/// `KaminoPosition`/`MarginfiPosition` on every event; if not yet true and
/// a cooldown has elapsed, retry the action) -- reliable here because
/// those accounts stay open with real, non-empty state throughout.
///
/// **Withdraw phases do NOT wait for confirmed on-chain absence.** A full
/// withdrawal closes the obligation account on Solend/Kamino (real,
/// live-verified behavior from the manual testing this module replaces)
/// -- and this bot's `on_account` tracking has no reliable "closed"
/// signal (a closed/deallocated account's update either never arrives or
/// parses too short to overwrite the last-known, still-showing-a-deposit
/// state), so polling for "the deposit is gone" here would retry
/// forever, resending a withdraw against an obligation that no longer
/// exists. Instead: fire once, wait out a fixed cooldown, then advance
/// unconditionally -- real success is verified by reading the resulting
/// on-chain state afterward (same way the manual testing this replaces
/// was verified), not by this state machine's own polling. marginfi's
/// `MarginfiAccount` PDA is different (it isn't closed by a full
/// withdrawal -- marginfi has a separate, explicit `close` instruction
/// this bot never calls), but `WithdrawMarginfi` still uses the same
/// fire-once-then-cooldown-advance shape for consistency with the other
/// two withdraw phases, not because it's strictly required here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TestPhase {
    #[default]
    SwapToUsdc,
    BootstrapSolend,
    DepositSolend,
    WithdrawSolend,
    BootstrapKamino,
    DepositKamino,
    WithdrawKamino,
    BootstrapMarginfi,
    DepositMarginfi,
    WithdrawMarginfi,
    /// Borrow/repay phases run *after* every protocol's own
    /// bootstrap/deposit/withdraw sequence completes, not interleaved
    /// with it -- each is self-contained (mirrors `open_solend_borrow_leg`
    /// et al.'s own real, two-stage "deposit USDC collateral if missing,
    /// then borrow" design), so it doesn't need to reuse any leftover
    /// state from the deposit/withdraw phases above. Unlike withdraw,
    /// borrow/repay don't close any account (the position just gains or
    /// loses a liability), so both use the same retry-until-confirmed
    /// shape as the deposit phases, not withdraw's fire-once-then-
    /// advance-unconditionally shape.
    BorrowSolend,
    RepaySolend,
    BorrowKamino,
    RepayKamino,
    BorrowMarginfi,
    RepayMarginfi,
    Done,
}

#[derive(Debug)]
pub(crate) struct State {
    last_slot: Slot,
    slot_delta_since_start: Slot,
    /// Paces every large startup subscription burst (Raydium/Orca/
    /// lending-reserve pools, ~32,000 requests total) across many slots
    /// instead of firing them all in one blocking `bulk_subscribe` call
    /// -- see [`SubscriptionQueue`]'s own doc comment. Drained by
    /// [`MAX_SUBSCRIBES_PER_SLOT`] requests per slot from
    /// `CommitHook::finish`.
    subscription_queue: SubscriptionQueue,
    /// Wall-clock instant `CommitHook::start` fired for the slot
    /// currently being processed -- `finish()` measures the elapsed time
    /// against this and logs a warning if it exceeds
    /// `SLOT_TIMING_TARGET_MS`. Diagnostic added while investigating this
    /// session's real `stdio timeout` disconnects -- whether slow
    /// guest-side per-slot processing (not just log volume, already
    /// fixed) is a contributing factor. Covers only this commit's own
    /// `start()`-through-`finish()` span (every `on_account`/`on_token`
    /// call plus `finish()`'s own flush work) -- see
    /// `o_prev_commit_start_instant` for the *other* number, the gap
    /// between one slot's `start()` and the next.
    o_commit_start_instant: Option<std::time::Instant>,
    /// Wall-clock instant the *previous* `CommitHook::start` fired --
    /// `start()` measures the gap against this every time it's called,
    /// before overwriting it for next time. Unlike `o_commit_start_instant`,
    /// this captures everything that happens *between* commits too:
    /// `evaluate()`'s own work, other event types (`LowLatency`, `Stdin`,
    /// `SlotStatus`), and genuine idle time waiting for the next event --
    /// the fuller picture of end-to-end guest responsiveness, not just
    /// this commit's own processing.
    o_prev_commit_start_instant: Option<std::time::Instant>,
    /// Cumulative wall-clock time spent inside `low_latency()` since the
    /// previous `CommitHook::start()` call -- `start()` logs and resets
    /// this every time, alongside the gap-since-previous-start number, to
    /// isolate how much of that gap (if any) is actually low-latency
    /// account-update processing versus something else. Diagnostic added
    /// to test a specific hypothesis: that low-latency updates (much
    /// higher volume/frequency than rooted commits) are the real
    /// contributor to this session's `stdio timeout` disconnects.
    low_latency_elapsed_since_last_start: std::time::Duration,
    /// How many low-latency account/token updates were processed in the
    /// same window as `low_latency_elapsed_since_last_start` -- reported
    /// alongside it so a large elapsed time can be told apart from "a
    /// genuinely huge batch arrived" vs. "processing got slow".
    low_latency_count_since_last_start: u64,
    /// Cumulative wall-clock time spent inside `evaluate()` since the
    /// previous `CommitHook::start()` call -- same accumulate-and-reset
    /// shape as `low_latency_elapsed_since_last_start`, reported alongside
    /// it. `evaluate()` runs after *every* event (`Commit`, `LowLatency`,
    /// `Stdin`, `Transaction`, `SlotStatus` -- see `on_event`'s tail call),
    /// so this is the other real candidate (besides `low_latency()`) for
    /// where the gap-since-previous-start time is actually going. Added
    /// to test a specific hypothesis: a negative-cycle/route search inside
    /// `evaluate()`'s call graph growing superlinearly as more of the
    /// ~32,000-request subscription burst drains in and the live pool/
    /// router graph grows.
    evaluate_elapsed_since_last_start: std::time::Duration,
    /// How many `evaluate()` calls happened in the same window as
    /// `evaluate_elapsed_since_last_start` -- same role as
    /// `low_latency_count_since_last_start`.
    evaluate_count_since_last_start: u64,
    /// Drives the real-transaction smoke test -- see [`TestPhase`]'s doc
    /// comment.
    test_phase: TestPhase,
    /// The slot the current phase's action (bootstrap/deposit/withdraw)
    /// was last sent at, if any -- gates retries/advancement so
    /// `evaluate()` (called on *every* event, which can fire many times
    /// per second) doesn't resend the same instruction before the
    /// previous one has had a real chance to confirm. Reset to `None` on
    /// every phase transition.
    test_last_action_slot: Option<Slot>,
    o_rc_keypair: Option<KeypairExtra>,
    o_phoenix: Option<PhoenixState>,
    /// This bot's own Solend lending position (the second leg of every
    /// basis trade) -- `None` until `on_load`, same lifecycle as
    /// `o_phoenix`. Reserve pricing/instruction-building come from the
    /// separate, shared, read-only `SolendState` inside `o_dex` --  this
    /// only tracks *this bot's own* obligation (mirrors
    /// `dex::velocity::VelocityState`'s old role for Drift's User
    /// account, scoped to one account instead of a market list).
    o_solend_position: Option<solend::SolendPosition>,
    /// This bot's own Kamino lending position -- the second lending
    /// protocol the basis trade can hedge through (SOL/BTC/ETH all have
    /// real Kamino reserves, vs. Solend's SOL-only), otherwise identical
    /// role/lifecycle to `o_solend_position`.
    o_kamino_position: Option<kamino::KaminoPosition>,
    /// This bot's own marginfi lending position -- the third lending
    /// protocol exercised by this smoke test, otherwise identical
    /// role/lifecycle to `o_solend_position`/`o_kamino_position`.
    o_marginfi_position: Option<marginfi::MarginfiPosition>,
    router: PerpRouter,
    /// The epoch currently being accumulated -- `None` until the first
    /// `evaluate()` call after `on_load`. Distinct from `PerpRouter`'s
    /// own internal pending buffers: this just tracks *when* to call
    /// `close_epoch`.
    pending_epoch_ts: Option<i64>,
    /// Spot-market execution hook -- see this module's doc comment and
    /// `StateHelper::execute_spot_leg`. `None` until `on_load`, same
    /// lifecycle as `o_phoenix`/`o_solend_position`.
    o_dex: Option<DexState>,
    /// Bellman-Ford spot price graph, fed incrementally by `low_latency`/
    /// `CommitHook::on_account` exactly like `arbv1::state`'s `router`
    /// field -- named `spot_router` here since `router` above is already
    /// taken by `PerpRouter`.
    spot_router: TradeRouter,
    /// Target portfolio allocation -- fraction of total portfolio value
    /// (0.0-1.0) to hold in each symbol, e.g. `0.30` for "target 30% of
    /// the portfolio in this symbol". The remainder is implicitly
    /// USD/stable (no explicit USD entry). Rebalancing toward this
    /// target is what realizes profit/loss. Seeded from
    /// `target_allocation_config::DEFAULT_TARGET_ALLOCATION`
    /// (build.rs-baked, itself read from the optimizer's own
    /// `prefetch.db` at compile time) and live-updated at runtime by
    /// `CustomMessageInbound::TargetAllocation` (see `on_message`
    /// below). Each entry is join-keyed to its mint via
    /// `resolve_symbol_mint` -- see `TargetAllocationEntry`'s doc for
    /// why that resolution is deferred, not done here at construction.
    /// Not yet consumed by any rebalance/selection logic -- see this
    /// module's plan doc for why that's a separate follow-up.
    target_allocation_pct: HashMap<String, TargetAllocationEntry>,
    /// Real annualized SOL-per-LST staking yield per liquid-staking-token
    /// symbol (e.g. `"jitoSOL" -> 0.073`), pushed periodically by
    /// `optimizer watch-lst-yield` via `CustomMessageInbound::LstApy` (see
    /// `on_message` below) -- this bot can't compute it itself (no
    /// persistent storage across restarts, see
    /// `leveraged_yield_farming_plan.md`'s "Phase 0"). Refreshed in place
    /// per symbol, not accumulated; absent until at least one real update
    /// has arrived. Read by `log_lst_loop_projection` for the
    /// Phase-1-style read-only leverage projection -- nothing in this bot
    /// opens a real leveraged position from this yet.
    lst_staking_apy: HashMap<String, f64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_slot: 0,
            slot_delta_since_start: 0,
            subscription_queue: SubscriptionQueue::default(),
            o_commit_start_instant: None,
            o_prev_commit_start_instant: None,
            low_latency_elapsed_since_last_start: std::time::Duration::ZERO,
            low_latency_count_since_last_start: 0,
            evaluate_elapsed_since_last_start: std::time::Duration::ZERO,
            evaluate_count_since_last_start: 0,
            test_phase: TestPhase::default(),
            test_last_action_slot: None,
            o_rc_keypair: None,
            o_phoenix: None,
            o_solend_position: None,
            o_kamino_position: None,
            o_marginfi_position: None,
            router: PerpRouter::default(),
            pending_epoch_ts: None,
            o_dex: None,
            spot_router: TradeRouter::default(),
            target_allocation_pct: target_allocation_config::DEFAULT_TARGET_ALLOCATION
                .iter()
                .map(|&(s, allocation_pct)| {
                    (
                        s.to_string(),
                        TargetAllocationEntry {
                            account_id: None,
                            allocation_pct,
                        },
                    )
                })
                .collect(),
            lst_staking_apy: HashMap::new(),
        }
    }
}

impl State {
    fn wallet(&self) -> Option<AccountId> {
        let ke = self.o_rc_keypair.as_ref()?;
        Some(ke.account_id)
    }

    /// Resolves every not-yet-resolved `target_allocation_pct` entry's
    /// mint via `resolve_symbol_mint`. Safe to call at any call site
    /// confirmed to run inside the live WASM guest (see
    /// `TargetAllocationEntry`'s doc) -- currently only the `Wallet`
    /// arm, which fires once per bot lifetime, so a symbol with no
    /// curated mint (not in `SYMBOL_MINT_MAP`) logging on every call
    /// isn't a practical spam risk; re-check if a future call site
    /// invokes this on a tighter loop.
    fn resolve_target_allocation_mints(&mut self) {
        for (symbol, entry) in self.target_allocation_pct.iter_mut() {
            if entry.account_id.is_some() {
                continue;
            }
            match resolve_symbol_mint(symbol) {
                Some(account_id) => entry.account_id = Some(account_id),
                None => {
                    log_error!(
                        "testperpv1: target allocation symbol {} has no curated mint -- cannot resolve to AccountId",
                        symbol,
                    );
                }
            }
        }
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

    /// Every pubkey baked in by build.rs's generated tables, across every
    /// `[u8; 32]` field of every generated struct -- not just each
    /// table's own "root" pool/reserve/market pubkey, but secondary
    /// references too (vaults, oracle keys, lending markets, etc.).
    /// Gathered once at boot and batch-resolved to `AccountId`s via
    /// `PubkeyAccountIdCache::account_ids` (called from `on_load`,
    /// before `DexState::new()`/`PhoenixState::new_and_subscribe` run),
    /// so every one of those constructors' own individual
    /// `account_id_from_pubkey` calls become cache hits instead of a
    /// fresh host round-trip each -- see `account_ids`'s own doc comment
    /// for why this is still "one host call per miss" rather than a
    /// single round-trip (`pubkey-map-by-pubkey` never gained a batched
    /// parameter, unlike the accountid-to-pubkey direction).
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
        for p in phoenix_config::PHOENIX_MARKETS {
            push!(p.market_account);
        }
        for p in drift_config::DRIFT_SPOT_MARKETS {
            push!(p.pubkey);
            push!(p.mint);
            push!(p.vault);
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
        for p in jet_config::JET_RESERVES {
            push!(p.pubkey);
            push!(p.market);
            push!(p.mint);
            push!(p.vault);
        }
        for p in top_pools_config::TOP_POOLS {
            push!(p.pubkey);
            push!(p.mint_a);
            push!(p.mint_b);
        }
        for p in symbol_mint_config::SYMBOL_MINT_MAP {
            push!(p.mint);
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

    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        // Batch-resolve every build-time-baked pubkey to an AccountId
        // before any of the constructors below run their own individual
        // lookups -- see `build_time_pubkeys`'s doc comment.
        let l_pk = Self::build_time_pubkeys();
        let n_pk = l_pk.len();
        let n_ids = crate::util::pubkey_account_id_cache()
            .account_ids(&l_pk)
            .len();
        log_warn!("testperpv1: batch-resolved {n_ids}/{n_pk} build-time pubkeys to account ids at startup");
        assert!(self
            .state
            .o_phoenix
            .replace(PhoenixState::new_and_subscribe(self.graph).expect("phoenix state"))
            .is_none());
        assert!(self
            .state
            .o_solend_position
            .replace(solend::SolendPosition::default())
            .is_none());
        assert!(self
            .state
            .o_kamino_position
            .replace(kamino::KaminoPosition::default())
            .is_none());
        assert!(self
            .state
            .o_marginfi_position
            .replace(marginfi::MarginfiPosition::default())
            .is_none());
        assert!(self
            .state
            .o_dex
            .replace(DexState::new().expect("dex state"))
            .is_none());
        // Seed spot_router's node set from the build-time liquidity
        // router's classified mint universe -- see
        // TradeRouter::from_router's doc comment: live pool registration
        // (refresh_account_router/refresh_token_router) only *looks up*
        // nodes, it never creates them, so route_slippage_aware would
        // silently return None for every mint forever without this.
        // Mirrors arbv1::state::on_load exactly (same
        // build_liquidity_router helper below).
        self.state.spot_router = TradeRouter::from_router(&build_liquidity_router());
        log_info!("testperpv1: bot has been successfully uploaded to validator");
    }

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("testperpv1: slot {slot}; status dead");
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        let t0 = std::time::Instant::now();
        let mut count: u64 = 0;
        while let Some(ta) = llap.token() {
            count += 1;
            self.wallet.token_mut().on_token(ta, false);
            if let Some(dex) = self.state.o_dex.as_mut() {
                _ = dex.on_token(ta);
                dex.refresh_token_router(ta.id, &mut self.state.spot_router);
            }
        }
        let zero = [];
        while let Some(account) = llap.account() {
            count += 1;
            let d = account.body.unwrap_or(&zero);
            self.wallet.on_account(account.header, d);
            if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                phoenix.on_account(account.header, d);
            }
            if let Some(solend_position) = self.state.o_solend_position.as_mut() {
                solend_position.on_account(account.header, d);
            }
            if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
                kamino_position.on_account(account.header, d);
            }
            if let Some(marginfi_position) = self.state.o_marginfi_position.as_mut() {
                marginfi_position.on_account(account.header, d);
            }
            if let Some(dex) = self.state.o_dex.as_mut() {
                dex.on_account(account.header, d);
                dex.refresh_account_router(account.header.accountid, &mut self.state.spot_router);
            }
        }
        // Accumulated (not logged here) -- see `CommitHook::start`'s doc
        // comment on `low_latency_elapsed_since_last_start` for why: this
        // fires far more often than a commit, so logging every call here
        // would reintroduce the exact log-volume problem already found to
        // contribute to real `stdio timeout` disconnects this session.
        self.state.low_latency_elapsed_since_last_start += t0.elapsed();
        self.state.low_latency_count_since_last_start += count;
    }

    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        // This mode never sends a transaction -- nothing to correlate,
        // just drain the iterator rather than assuming it's safe to skip
        // entirely.
        while transaction_list.transaction().is_some() {}
    }

    fn current_epoch_ts() -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs() as i64;
        (now / SECONDS_PER_EPOCH) * SECONDS_PER_EPOCH
    }

    /// Feed every currently-known market into `PerpRouter` for the given
    /// epoch. Called every `evaluate()`, not just at the epoch boundary,
    /// so the router's pending buffers always reflect the freshest
    /// reading by the time the epoch actually closes -- matches
    /// `PerpRouter::close_epoch`'s existing "closes out whatever's
    /// pending" contract, no change needed there.
    fn observe_all(&mut self, epoch_ts: i64) {
        if let Some(phoenix) = self.state.o_phoenix.as_ref() {
            for market in phoenix.markets() {
                self.state
                    .router
                    .observe_phoenix(market, market.mark_price_usd(), epoch_ts);
            }
        }
    }

    fn log_latest_layer(&self) {
        let Some(layer) = self.state.router.latest_layer() else {
            return;
        };
        if layer.edges.is_empty() {
            log_warn!(
                "testperpv1: epoch {} closed @ slot {} -- no funding spread edges (no symbol had data from both venues, or rates were equal)",
                layer.epoch_ts,
                layer.slot,
            );
            return;
        }
        for edge in &layer.edges {
            log_warn!(
                "testperpv1: epoch {} @ slot {}: {} long={:?} short={:?} spread_annualized={:.3}%",
                layer.epoch_ts,
                layer.slot,
                edge.asset,
                edge.from_venue,
                edge.to_venue,
                edge.spread_annualized_pct,
            );
        }
    }

    /// Diagnostic-only spot SOL/USD price probe, both directions --
    /// mirrors `arbv1::state::evaluate`'s periodic "trade router check"
    /// (same `route_slippage_aware` + `reverify_route_with_exact_quotes`
    /// pattern, same CLMM-quote safety check and pool-cooldown-on-
    /// rejection), except run both SOL->USDC and USDC->SOL so the two
    /// implied prices can be compared against each other. Read-only:
    /// never touches `execute_spot_leg`/`self.wallet`, so it can never
    /// build or send anything -- purely confirms `spot_router` is being
    /// fed live data and can find a route in *this* process.
    fn log_spot_price_probe(&mut self) {
        let (mint_sol, mint_usdc) = (self.configuration.mint_sol, self.configuration.mint_usdc);
        if mint_sol == 0 || mint_usdc == 0 {
            // Configuration::set() hasn't run yet -- no wallet keypair
            // received, so the mint AccountIds aren't resolved.
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        self.state
            .spot_router
            .set_current_slot(self.state.last_slot);
        // Independent ground truth for both directions' router-implied
        // prices below -- same `pyth_sol_usd_price` cross-check
        // `arbv1::state::evaluate`'s own "trade router check" diagnostic
        // uses, not a new lookup path.
        let o_pyth = dex.pyth_sol_usd_price();
        let last_slot = self.state.last_slot;
        let log_pyth_delta = |direction: &str, router_price_usd: f64| {
            let Some(pyth_price) = o_pyth.as_ref() else {
                return;
            };
            let delta_pct =
                (router_price_usd - pyth_price.price_usd) / pyth_price.price_usd * 100.0;
            log_warn!(
                "testperpv1: spot price probe @ slot {}: {} router SOL/USD={:.4} pyth SOL/USD={:.4} (conf={:.4}) delta={:+.2}%",
                last_slot,
                direction,
                router_price_usd,
                pyth_price.price_usd,
                pyth_price.confidence_usd,
                delta_pct,
            );
        };
        // Per-hop breakdown -- only for multi-hop routes (a 1-hop route's
        // top-level "amount_in -> amount_out" line above already says
        // everything). Added to debug a real observed case: a 4-hop
        // USDC->SOL route passed reverify_route_with_exact_quotes (every
        // hop individually re-quoted fine) yet the end-to-end price was
        // ~18x too low vs Pyth -- this surfaces which specific hop's
        // amount_in/amount_out ratio is the culprit.
        let log_route_hops = |direction: &str, route: &crate::trader::pricegraph::Route| {
            if route.hops.len() <= 1 {
                return;
            }
            for (i, hop) in route.hops.iter().enumerate() {
                // Temporary: resolve the pool's real pubkey for offline
                // RPC verification of the Raydium CLMM tick-array fix --
                // AccountId is a runtime-assigned host mapping
                // (shooter::pubkey_map_by_id), unrecoverable outside the
                // live WASM guest, so this is the only way to get it.
                let pool_pubkey = crate::util::pubkey_from_account_id(&hop.pool_id);
                log_warn!(
                    "testperpv1: spot price probe @ slot {}: {} hop {}: dex={:?} pool={} ({:?}) {} -> {} amount_in={} amount_out={}",
                    last_slot,
                    direction,
                    i,
                    hop.dex,
                    hop.pool_id,
                    pool_pubkey,
                    hop.input_mint,
                    hop.output_mint,
                    hop.amount_in,
                    hop.amount_out,
                );
            }
        };

        const MAX_HOPS: usize = 4;
        const SOL_DECIMALS: i32 = 9;
        const USDC_DECIMALS: i32 = 6;
        // Diagnostic-only USDC probe size for the reverse direction --
        // no established constant for this direction elsewhere in the
        // codebase (arbv1's own probe only ever quotes SOL->USDC), so
        // this is a round $10 pick, comparable in spirit to arbv1's
        // build-time-configurable SOL-side probe.
        const USDC_PROBE_RAW: u64 = 10_000_000;
        let sol_probe_raw = crate::diagnostic_config::TRADE_ROUTER_PROBE_LAMPORTS;

        match self.state.spot_router.route_slippage_aware(
            mint_sol,
            mint_usdc,
            sol_probe_raw,
            MAX_HOPS,
        ) {
            Some(route) => {
                match planner::reverify_route_with_exact_quotes(
                    &route,
                    sol_probe_raw,
                    &self.state.spot_router,
                    dex,
                ) {
                    Ok(route) => {
                        let sol_in = sol_probe_raw as f64 / 10f64.powi(SOL_DECIMALS);
                        let usdc_out = route.amount_out() as f64 / 10f64.powi(USDC_DECIMALS);
                        if sol_in > 0.0 {
                            let router_price_usd = usdc_out / sol_in;
                            log_warn!(
                                "testperpv1: spot price probe @ slot {}: SOL->USDC {:.9} SOL -> {:.6} USDC (price={:.4} USD/SOL, {} hop{})",
                                self.state.last_slot,
                                sol_in,
                                usdc_out,
                                router_price_usd,
                                route.n_hops(),
                                if route.n_hops() == 1 { "" } else { "s" },
                            );
                            log_pyth_delta("SOL->USDC", router_price_usd);
                            log_route_hops("SOL->USDC", &route);
                        }
                    }
                    Err(failure) => {
                        if failure.coolable {
                            self.state
                                .spot_router
                                .mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                        log_warn!(
                            "testperpv1: spot price probe @ slot {}: SOL->USDC rejected -- exact quote invalidated pool {} (cooling down {} slots: {})",
                            self.state.last_slot,
                            failure.pool_id,
                            planner::POOL_COOLDOWN_SLOTS,
                            failure.coolable,
                        );
                    }
                }
            }
            None => {
                log_warn!(
                    "testperpv1: spot price probe @ slot {}: no SOL->USDC route found",
                    self.state.last_slot,
                );
            }
        }

        match self.state.spot_router.route_slippage_aware(
            mint_usdc,
            mint_sol,
            USDC_PROBE_RAW,
            MAX_HOPS,
        ) {
            Some(route) => {
                match planner::reverify_route_with_exact_quotes(
                    &route,
                    USDC_PROBE_RAW,
                    &self.state.spot_router,
                    dex,
                ) {
                    Ok(route) => {
                        let usdc_in = USDC_PROBE_RAW as f64 / 10f64.powi(USDC_DECIMALS);
                        let sol_out = route.amount_out() as f64 / 10f64.powi(SOL_DECIMALS);
                        if sol_out > 0.0 {
                            let router_price_usd = usdc_in / sol_out;
                            log_warn!(
                                "testperpv1: spot price probe @ slot {}: USDC->SOL {:.6} USDC -> {:.9} SOL (price={:.4} USD/SOL, {} hop{})",
                                self.state.last_slot,
                                usdc_in,
                                sol_out,
                                router_price_usd,
                                route.n_hops(),
                                if route.n_hops() == 1 { "" } else { "s" },
                            );
                            log_pyth_delta("USDC->SOL", router_price_usd);
                            log_route_hops("USDC->SOL", &route);
                        }
                    }
                    Err(failure) => {
                        if failure.coolable {
                            self.state
                                .spot_router
                                .mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
                        }
                        log_warn!(
                            "testperpv1: spot price probe @ slot {}: USDC->SOL rejected -- exact quote invalidated pool {} (cooling down {} slots: {})",
                            self.state.last_slot,
                            failure.pool_id,
                            planner::POOL_COOLDOWN_SLOTS,
                            failure.coolable,
                        );
                    }
                }
            }
            None => {
                log_warn!(
                    "testperpv1: spot price probe @ slot {}: no USDC->SOL route found",
                    self.state.last_slot,
                );
            }
        }
    }

    /// Builds a tiny synthetic graph with a negative cycle guaranteed by
    /// construction (not derived from real market data) and confirms
    /// `FinancialGraph::detect_negative_cycle` finds it -- the only way
    /// to exercise `relax_chunk_simd`'s real `wasm32` SIMD-128
    /// intrinsics at all, since native `cargo test` only runs the
    /// portable scalar fallback added alongside them when fixing
    /// `spfa.rs`'s compile errors earlier this session. 4 nodes:
    /// 0->1->2->0 is the real cycle (rate 1.01 per hop, so `1.01^3 > 1`,
    /// i.e. `-ln(rate)` summed around the loop is negative); node 3 is
    /// an inert weight-0-edge target padding each `add_edge_pair` call
    /// to 2 lanes (SIMD needs pairs; a 3-edge cycle is odd) that can
    /// never itself trigger a relaxation. `spfa` isn't wired into any
    /// real strategy yet (no `NodeMeta`/asset-universe integration), so
    /// this is purely a runtime/SIMD-correctness check, not a strategy
    /// test -- see `trader::spfa`'s own state for that gap.
    fn log_spfa_smoke_test(&self) {
        let mut g = crate::trader::spfa::FinancialGraph::new(4);
        g.add_edge_pair(0, (1, 1.01), (3, 1.0));
        g.add_edge_pair(1, (2, 1.01), (3, 1.0));
        g.add_edge_pair(2, (0, 1.01), (3, 1.0));
        match g.detect_negative_cycle() {
            Some(cycle) => log_warn!(
                "testperpv1: spfa smoke test OK -- found expected synthetic negative cycle: path={:?} weight={}",
                cycle.path,
                cycle.total_log_weight,
            ),
            None => log_error!(
                "testperpv1: spfa smoke test FAILED -- no cycle found in a graph built with a guaranteed negative cycle (real wasm32 SIMD bug?)",
            ),
        }
    }

    /// Builds `FinancialGraph`'s real live topology from `PerpRouter`'s
    /// current-epoch pending rates (must be called *before*
    /// `close_epoch` clears them) and logs every profitable cycle found
    /// -- the real integration the SIMD smoke test's synthetic graph
    /// was standing in for. Asset universe:
    /// `symbol_mint_config::SYMBOL_MINT_MAP`'s 6 entries (SOL/BTC/ETH/
    /// XRP/BNB/SUI -- DOGE has no curated mint, gets no graph presence
    /// here, same asset universe the `base_mint` join key already
    /// committed to). 2 nodes per asset:
    /// - `Home`: self-loops to capture the inter-venue funding spread,
    ///   weight reused directly from `PerpRouter`'s own already-verified
    ///   rate computation via `pending_rate` -- net-delta-zero by
    ///   construction, since a long-cheap/short-expensive pair cancels.
    ///   Only added when both venues have reported a rate this epoch,
    ///   mirroring `PerpRouter::close_epoch`'s own "a symbol only one
    ///   venue reported produces no edge" rule exactly.
    /// - `Spot`: real spot holding. `Home<->Spot` edges are where
    ///   directional exposure actually changes -- weight 0 for this
    ///   pass (no fee/slippage/basis modeling yet, deliberately
    ///   flagged, not silently assumed accurate) -- structural, added
    ///   unconditionally regardless of live funding data.
    ///
    /// One-time bootstrap for the Phoenix trader account: `register_trader`,
    /// convert USDC -> PhUSD via Ember (`dex::ember`), then `deposit_funds`
    /// as margin collateral -- batched into a single transaction (Solana
    /// executes instructions within one transaction sequentially, so
    /// `deposit_funds` can safely reference the account `register_trader`
    /// just created earlier in the same tx, same as how Drift's own
    /// frontend batches `initialize_user_stats`+`initialize_user`+
    /// `deposit`). Budget is half of `FUNDING_CYCLE_MIN_MARGIN_USD` (that
    /// constant is documented as "capital for one cycle, both legs
    /// combined"), bounded by `current_usdc_value()` so this never tries
    /// to spend USDC that isn't there. Called instead of placing an order
    /// -- see `open_phoenix_leg`'s call site -- so the first
    /// capital-feasible cycle found after a fresh wallet is spent on
    /// setup, not a real position; the next one proceeds normally once
    /// `PhoenixState::trader_registered` flips true from a real
    /// `on_account` update.
    fn bootstrap_phoenix_trader(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        if self
            .state
            .o_phoenix
            .as_ref()
            .and_then(|p| p.trader_account())
            .is_none()
        {
            log_warn!("testperpv1: bootstrap: phoenix trader_account PDA not known yet -- set_authority hasn't run");
            return;
        }
        let budget_usd = (FUNDING_CYCLE_MIN_MARGIN_USD / 2.0).min(self.current_usdc_value());
        if budget_usd <= 0.0 {
            log_warn!(
                "testperpv1: bootstrap: no spare USDC to fund the Phoenix trader account yet"
            );
            return;
        }
        const USDC_DECIMALS: i32 = 6;
        let amount_raw = (budget_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        let mint_usdc = self.configuration.mint_usdc;
        let Some(phusd_mint_pk) = self.state.o_phoenix.as_ref().map(|p| p.canonical_mint()) else {
            return;
        };
        let phusd_mint_id = account_id_from_pubkey(&phusd_mint_pk);
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        let Some(phusd_ata) = self.wallet.append_create_ata(owner, phusd_mint_id) else {
            return;
        };

        log_warn!(
            "testperpv1: bootstrap: registering + funding Phoenix trader account (${:.2})",
            budget_usd,
        );
        let Some(phoenix) = self.state.o_phoenix.as_ref() else {
            return;
        };
        if let Err(e) = phoenix.register_trader(owner, self.wallet) {
            log_error!("testperpv1: bootstrap: phoenix register_trader failed: {e}");
            return;
        }
        if let Err(e) = ember::deposit(
            owner,
            phusd_mint_id,
            usdc_ata,
            phusd_ata,
            amount_raw,
            self.wallet,
        ) {
            log_error!("testperpv1: bootstrap: ember deposit failed: {e}");
            return;
        }
        if let Err(e) = phoenix.deposit_funds(owner, phusd_ata, amount_raw, self.wallet) {
            log_error!("testperpv1: bootstrap: phoenix deposit_funds failed: {e}");
        }
    }

    /// One-time bootstrap for this bot's own Solend obligation:
    /// `create_obligation_account` + `init_obligation`, batched into a
    /// single transaction -- same reasoning as
    /// [`Self::bootstrap_phoenix_trader`]. No deposit here -- collateral
    /// sizing/asset choice is direction-specific (deposit-hedge deposits
    /// the underlying, borrow-hedge deposits USDC), decided at open time
    /// in `open_deposit_hedge_leg`/`open_borrow_hedge_leg`, not bootstrap
    /// time. `lending_market` is read off the USDC reserve (any tracked
    /// reserve's `lending_market` field works -- they all share Solend's
    /// one main pool -- USDC is just guaranteed to be tracked). Gated by
    /// `!solend_position.registered()`, queued instead of opening a leg,
    /// deferred to next epoch once confirmed via a real `on_account`
    /// update -- identical precedent to `bootstrap_phoenix_trader`.
    fn bootstrap_solend_obligation(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((_, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            log_warn!("testperpv1: bootstrap: Solend USDC reserve not observed yet");
            return;
        };
        let lending_market = usdc_reserve.lending_market;

        // `id=0`, named (not a bare literal) so this real, currently-live
        // Solend obligation address is easy to grep for -- must never
        // change, since a different id would derive a different, unfunded
        // account, silently orphaning any real position already open here.
        const SOLEND_OBLIGATION_ID: u8 = 0;
        log_warn!("testperpv1: bootstrap: registering Solend obligation");
        if let Err(e) = solend::create_obligation_account(owner, SOLEND_OBLIGATION_ID, self.wallet)
        {
            log_error!("testperpv1: bootstrap: solend create_obligation_account failed: {e}");
            return;
        }
        if let Err(e) =
            solend::init_obligation(owner, lending_market, SOLEND_OBLIGATION_ID, self.wallet)
        {
            log_error!("testperpv1: bootstrap: solend init_obligation failed: {e}");
        }
    }

    /// One-time bootstrap for this bot's own Kamino obligation:
    /// `init_user_metadata` + `init_obligation`, batched into a single
    /// transaction -- same reasoning as [`Self::bootstrap_solend_obligation`],
    /// but Kamino's own two-step order (`init_user_metadata` must exist
    /// before `init_obligation` will succeed, unlike Solend's
    /// create-account-then-init). `lending_market` is
    /// [`kamino::KAMINO_MAIN_MARKET`] directly -- Kamino's real, fixed
    /// main market every currently-tracked reserve belongs to, so unlike
    /// Solend's bootstrap this needs no live reserve lookup first. Gated
    /// by `!kamino_position.registered()`, queued instead of opening a
    /// leg, deferred to next epoch once confirmed via a real `on_account`
    /// update -- identical precedent to `bootstrap_solend_obligation`.
    fn bootstrap_kamino_obligation(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let lending_market = account_id_from_pubkey(&kamino::KAMINO_MAIN_MARKET);

        // `user_metadata` is per-owner, not per-obligation -- it survives
        // a full withdrawal closing the obligation, and `init_user_metadata`
        // fails (Anchor `init` constraint) if called a second time. Only
        // (re-)create it when a real `on_account` update hasn't confirmed
        // it exists yet; a truly fresh wallet still gets both batched into
        // one transaction exactly as before.
        let has_user_metadata = self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.user_metadata_registered());
        if !has_user_metadata {
            log_warn!("testperpv1: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("testperpv1: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("testperpv1: bootstrap: registering Kamino obligation");
        if let Err(e) = kamino::init_obligation(owner, lending_market, 0, self.wallet) {
            log_error!("testperpv1: bootstrap: kamino init_obligation failed: {e}");
        }
    }

    /// One-time bootstrap for this bot's own marginfi `MarginfiAccount`:
    /// a single `marginfi_account_initialize_pda` instruction -- simpler
    /// than Solend/Kamino's bootstrap (no separate obligation-account or
    /// user-metadata step; the PDA itself *is* the account, see
    /// [`marginfi::initialize_account_pda`]'s doc comment). Always scoped
    /// to [`marginfi::MARGINFI_MAIN_GROUP`], this bot's only group. Gated
    /// by `!marginfi_position.registered()`, queued instead of opening a
    /// leg, deferred to next epoch once confirmed via a real `on_account`
    /// update -- identical precedent to `bootstrap_solend_obligation`/
    /// `bootstrap_kamino_obligation`.
    fn bootstrap_marginfi_account(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let group = account_id_from_pubkey(&marginfi::MARGINFI_MAIN_GROUP);

        log_warn!("testperpv1: bootstrap: registering marginfi account");
        if let Err(e) = marginfi::initialize_account_pda(group, owner, self.wallet) {
            log_error!("testperpv1: bootstrap: marginfi initialize_account_pda failed: {e}");
        }
    }

    /// Ensures the Kamino Farms "farmer" account for `reserve_id` is ready
    /// before a deposit/withdraw (`mode = 0`) or borrow/repay (`mode = 1`)
    /// against it -- `true` immediately if `farm` (that reserve's
    /// `farm_collateral`/`farm_debt`, matching `mode`) is `None`, i.e. no
    /// farm is attached (BTC/ETH today). Otherwise: subscribes to the
    /// derived farmer PDA ([`kamino::farm_user_state_id`]), and if it
    /// hasn't been confirmed to exist yet, queues
    /// `init_obligation_farms_for_reserve` and returns `false` -- same
    /// bootstrap-then-defer-to-next-epoch pattern as
    /// [`Self::bootstrap_kamino_obligation`]. Returns `true` only once a
    /// real `on_account` update has confirmed the farmer account exists.
    /// Found via `simulateTransaction` (`Custom(6120) FarmAccountsMissing`
    /// without this) -- see `KaminoReserve`'s `farm_collateral`/
    /// `farm_debt` fields' doc comments.
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
            log_error!("testperpv1: basis trade: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!(
            "testperpv1: basis trade: bootstrapping Kamino farm-user-state for reserve {}",
            reserve_id
        );
        if let Err(e) = kamino::init_obligation_farms_for_reserve(
            owner,
            obligation_id,
            reserve_lending_market,
            reserve_id,
            farm,
            mode,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: kamino init_obligation_farms_for_reserve failed: {e}"
            );
        }
        false
    }

    /// Live Phoenix position size for `symbol`, signed (`> 0` long,
    /// `< 0` short), `None` if flat or the market/position isn't known
    /// yet. Real on-chain state, not separate bookkeeping -- shared by
    /// the open-gate, the close-decision, and `close_phoenix_leg`.
    fn phoenix_position(&self, symbol: &str) -> Option<i64> {
        let phoenix = self.state.o_phoenix.as_ref()?;
        let market = phoenix
            .markets()
            .iter()
            .find(|m| m.symbol_str() == symbol)?;
        let pos = phoenix
            .positions()
            .iter()
            .find(|p| p.asset_id as u32 == market.asset_id)?;
        (pos.base_lot_position != 0).then_some(pos.base_lot_position)
    }

    /// Every reserve this bot's obligation currently has a deposit OR
    /// borrow position in, from the last real `on_account` update.
    /// Solend's own staleness check is enforced across the *entire*
    /// obligation, not just whichever reserve(s) a given instruction
    /// touches -- not a concurrency/threading concern (this bot is
    /// single-threaded), a protocol-level requirement: if this bot ever
    /// holds two symbols' positions on the same obligation at once (one
    /// opened epochs before the other), an action that only refreshes
    /// its own reserve(s) can still get rejected on-chain as stale with
    /// respect to the *other*, unrefreshed position. Empty if
    /// unregistered or no positions yet.
    fn solend_obligation_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
        else {
            return Vec::new();
        };
        ob.deposits
            .iter()
            .map(|d| d.deposit_reserve)
            .chain(ob.borrows.iter().map(|b| b.borrow_reserve))
            .collect()
    }

    /// Every reserve this bot's obligation currently has a *deposit* in
    /// -- the "borrow attribution" accounts `SolendReserve::borrow`/
    /// `withdraw` require, one per `obligation.deposits[i]` (narrower
    /// than [`Self::solend_obligation_reserves`]: borrows don't need
    /// attribution accounts, only deposits do).
    fn solend_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
        else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    /// [`Self::solend_obligation_reserves`], for `refresh_obligation`'s
    /// account list.
    ///
    /// **Deliberately does NOT add "this call's own extra reserve"** (an
    /// earlier version of this method did, via an `extra: &[AccountId]`
    /// param, removed 2026-08-17). Real Solend's `process_refresh_obligation`
    /// requires the remaining-accounts count to match the obligation's
    /// *current* on-chain deposit+borrow count exactly -- live-verified
    /// against the real `solendprotocol` mainnet source (cross-checked
    /// against the identical requirement in Kamino's `refresh_obligation`,
    /// confirmed there via `simulateTransaction`): `if
    /// account_info_iter.next().is_some() { msg!("Too many obligation
    /// deposit or borrow reserves provided"); return
    /// Err(LendingError::InvalidAccountInput.into()); }`. The reserve
    /// being newly deposited/borrowed into is added to the obligation *by*
    /// that deposit/borrow instruction, not before it -- callers must not
    /// pre-include it.
    fn solend_refresh_reserves(&self) -> Vec<AccountId> {
        self.solend_obligation_reserves()
    }

    /// Cheapest real borrow APY for `symbol` across every lending
    /// protocol with a tracked, priced reserve for it (percent units,
    /// matching [`decide_basis_trade`]'s expectation) -- `None` if
    /// neither Solend nor Kamino has one, or the ones that do haven't
    /// reported an account update yet. Used both for the borrow-hedge
    /// profitability signal (cheapest borrow = best chance of clearing
    /// the funding-collected bar) and, once borrow-hedge is chosen, to
    /// know which protocol to actually borrow from.
    fn best_borrow_apy(&self, symbol: &str) -> Option<(LendingProtocol, f64)> {
        let mint = resolve_symbol_mint(symbol)?;
        let dex = self.state.o_dex.as_ref()?;
        let solend_apy = dex
            .solend()
            .reserve_by_mint(mint)
            .map(|(_, r)| r.current_borrow_apy() * 100.0);
        let kamino_apy = dex
            .kamino()
            .reserve_by_mint(mint)
            .map(|(_, r)| r.current_borrow_apy() * 100.0);
        match (solend_apy, kamino_apy) {
            (Some(s), Some(k)) if k < s => Some((LendingProtocol::Kamino, k)),
            (Some(s), _) => Some((LendingProtocol::Solend, s)),
            (None, Some(k)) => Some((LendingProtocol::Kamino, k)),
            (None, None) => None,
        }
    }

    /// Highest real supply APY for `symbol` across every lending protocol
    /// with a tracked, priced reserve for it -- used once deposit-hedge
    /// is already chosen (depositing always helps regardless of
    /// protocol, so this isn't part of the open/close signal, only which
    /// protocol to actually deposit into).
    fn best_supply_apy(&self, symbol: &str) -> Option<(LendingProtocol, f64)> {
        let mint = resolve_symbol_mint(symbol)?;
        let dex = self.state.o_dex.as_ref()?;
        let solend_apy = dex
            .solend()
            .reserve_by_mint(mint)
            .map(|(_, r)| r.current_supply_apy() * 100.0);
        let kamino_apy = dex
            .kamino()
            .reserve_by_mint(mint)
            .map(|(_, r)| r.current_supply_apy() * 100.0);
        match (solend_apy, kamino_apy) {
            (Some(s), Some(k)) if k > s => Some((LendingProtocol::Kamino, k)),
            (Some(s), _) => Some((LendingProtocol::Solend, s)),
            (None, Some(k)) => Some((LendingProtocol::Kamino, k)),
            (None, None) => None,
        }
    }

    /// [`Self::best_borrow_apy`]/[`Self::best_supply_apy`]'s USDC-specific
    /// counterpart -- those two are keyed by a curated perp symbol
    /// (`resolve_symbol_mint`), but USDC isn't one of the 6 entries in
    /// `SYMBOL_MINT_MAP`, so idle-USDC deployment ([`Self::deploy_idle_usdc`])
    /// needs its own lookup using `self.configuration.mint_usdc` directly.
    fn best_usdc_supply_apy(&self) -> Option<(LendingProtocol, f64)> {
        let mint_usdc = self.configuration.mint_usdc;
        let dex = self.state.o_dex.as_ref()?;
        let solend_apy = dex
            .solend()
            .reserve_by_mint(mint_usdc)
            .map(|(_, r)| r.current_supply_apy() * 100.0);
        let kamino_apy = dex
            .kamino()
            .reserve_by_mint(mint_usdc)
            .map(|(_, r)| r.current_supply_apy() * 100.0);
        match (solend_apy, kamino_apy) {
            (Some(s), Some(k)) if k > s => Some((LendingProtocol::Kamino, k)),
            (Some(s), _) => Some((LendingProtocol::Solend, s)),
            (None, Some(k)) => Some((LendingProtocol::Kamino, k)),
            (None, None) => None,
        }
    }

    /// [`Self::best_usdc_supply_apy`]'s borrow-side counterpart -- the
    /// debt cost side of an LST-collateral leverage loop (deposit LST,
    /// borrow USDC), used only by [`Self::log_lst_loop_projection`]'s
    /// read-only projection. Same Solend/Kamino cheapest-wins selection
    /// as [`Self::best_borrow_apy`], just keyed by `mint_usdc` directly
    /// instead of a curated symbol.
    fn best_usdc_borrow_apy(&self) -> Option<(LendingProtocol, f64)> {
        let mint_usdc = self.configuration.mint_usdc;
        let dex = self.state.o_dex.as_ref()?;
        let solend_apy = dex
            .solend()
            .reserve_by_mint(mint_usdc)
            .map(|(_, r)| r.current_borrow_apy() * 100.0);
        let kamino_apy = dex
            .kamino()
            .reserve_by_mint(mint_usdc)
            .map(|(_, r)| r.current_borrow_apy() * 100.0);
        match (solend_apy, kamino_apy) {
            (Some(s), Some(k)) if k < s => Some((LendingProtocol::Kamino, k)),
            (Some(s), _) => Some((LendingProtocol::Solend, s)),
            (None, Some(k)) => Some((LendingProtocol::Kamino, k)),
            (None, None) => None,
        }
    }

    /// Phase 1 of `leveraged_yield_farming_plan.md`: read-only, no
    /// transactions sent. Logs what an LST-collateral/USDC-debt leverage
    /// loop would return at a few candidate leverage levels, using
    /// `net_apy(L) = L*lst_staking_apy - (L-1)*usdc_borrow_apy` (the
    /// plan's own formula -- LST *lending* supply APR is omitted, not
    /// approximated as zero: it's confirmed near-zero in the plan's
    /// live-checked numbers, and this bot doesn't currently track
    /// jitoSOL/bSOL/mSOL Kamino reserves at all, so there's nothing to
    /// read even if it mattered). No-ops for a symbol until a real
    /// `LstApy` update has arrived for it and a real USDC borrow rate is
    /// available -- never fabricates either input.
    fn log_lst_loop_projection(&self, symbol: &str, staking_apy_fraction: f64) {
        let Some((protocol, usdc_borrow_apy_pct)) = self.best_usdc_borrow_apy() else {
            return;
        };
        let staking_apy_pct = staking_apy_fraction * 100.0;
        let levels = [1.0, 2.0, 3.0];
        let projections: Vec<String> = levels
            .iter()
            .map(|&l| {
                let net_apy = l * staking_apy_pct - (l - 1.0) * usdc_borrow_apy_pct;
                format!("{l:.1}x={net_apy:.2}%")
            })
            .collect();
        log_warn!(
            "testperpv1: LST loop projection {symbol}: staking_apy={staking_apy_pct:.3}% usdc_borrow_apy={usdc_borrow_apy_pct:.3}% ({protocol:?}) -> {}",
            projections.join(" "),
        );
    }

    /// Every reserve this bot's Kamino obligation currently has a
    /// *deposit* in, from the last real `on_account` update -- one of the
    /// two lists Kamino's `refresh_obligation` needs (unlike Solend's one
    /// combined list, Kamino keeps deposit/borrow reserves separate). Also
    /// the `deposit_reserves_for_elevation` list `KaminoReserve::borrow`
    /// wants. Empty if unregistered or no deposits yet.
    fn kamino_obligation_deposit_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
        else {
            return Vec::new();
        };
        ob.deposits.iter().map(|d| d.deposit_reserve).collect()
    }

    /// Every reserve this bot's Kamino obligation currently has a
    /// *borrow* against, from the last real `on_account` update -- the
    /// other of the two lists Kamino's `refresh_obligation` needs. Empty
    /// if unregistered or no borrows yet.
    fn kamino_obligation_borrow_reserves(&self) -> Vec<AccountId> {
        let Some(ob) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
        else {
            return Vec::new();
        };
        ob.borrows.iter().map(|b| b.borrow_reserve).collect()
    }

    /// [`Self::kamino_obligation_deposit_reserves`]/
    /// [`Self::kamino_obligation_borrow_reserves`] as a pair, for
    /// `refresh_obligation`'s two-list signature.
    ///
    /// **Deliberately does NOT add "this call's own extra reserve" the
    /// way `solend_refresh_reserves` does.** Real klend requires
    /// `refresh_obligation`'s remaining-accounts count to match the
    /// obligation's *current* on-chain deposit+borrow count exactly --
    /// live-verified via `simulateTransaction`: including a reserve not
    /// yet in the obligation (e.g. the target of a brand-new first
    /// deposit/borrow) fails with `Custom(6006) InvalidAccountInput`
    /// (`expected_remaining_accounts=0, actual_remaining_accounts=1` for
    /// a fresh obligation). The reserve being newly deposited/borrowed
    /// into is added to the obligation *by* that deposit/borrow
    /// instruction, not before it -- callers must not pre-include it.
    fn kamino_refresh_reserves(&self) -> (Vec<AccountId>, Vec<AccountId>) {
        (
            self.kamino_obligation_deposit_reserves(),
            self.kamino_obligation_borrow_reserves(),
        )
    }

    /// Opens (or does nothing, if `symbol` isn't a Phoenix-tracked
    /// market) the Phoenix leg: `long` = buy (`Side::Bid`), else sell
    /// (`Side::Ask`). Sizes `notional_usd` via `mark_price_usd()` (this
    /// session's fix -- see `dex::phoenix::PhoenixMarketState`) and
    /// `base_lot_decimals`. Bootstraps (registers + funds) the trader
    /// account instead of placing an order if it isn't registered yet --
    /// see `bootstrap_phoenix_trader`'s doc comment. Does nothing if a
    /// position is already open for `symbol` -- open once, hold, and let
    /// the close-decision handle the exit; without this gate the same
    /// symbol being selected epoch after epoch would keep adding to the
    /// position instead of leaving it alone.
    fn open_phoenix_leg(&mut self, symbol: &str, long: bool, notional_usd: f64) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        if !self
            .state
            .o_phoenix
            .as_ref()
            .is_some_and(|p| p.trader_registered())
        {
            self.bootstrap_phoenix_trader();
            return;
        }
        if self.phoenix_position(symbol).is_some() {
            return;
        }
        let Some(phoenix) = self.state.o_phoenix.as_ref() else {
            return;
        };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == symbol) else {
            return;
        };
        let Some(price_usd) = market.mark_price_usd() else {
            log_error!(
                "testperpv1: funding cycle: {} has no oracle price yet, skipping Phoenix leg",
                symbol
            );
            return;
        };
        let num_base_lots = ((notional_usd / price_usd)
            * 10f64.powi(market.base_lot_decimals as i32))
        .round() as u64;
        if num_base_lots == 0 {
            return;
        }
        let asset_id = market.asset_id;
        let side = if long { Side::Bid } else { Side::Ask };

        log_warn!(
            "testperpv1: funding cycle: opening Phoenix leg {} side={:?} num_base_lots={} (notional=${:.2})",
            symbol,
            side,
            num_base_lots,
            notional_usd,
        );
        if let Err(e) =
            phoenix.place_market_order(owner, asset_id, side, num_base_lots, 0, 0, self.wallet)
        {
            log_error!(
                "testperpv1: funding cycle: Phoenix leg {} failed: {}",
                symbol,
                e
            );
        }
    }

    /// Flattens the live Phoenix position for `symbol` to zero via an
    /// opposite-side market order -- a proven-live liquidation-avoidance
    /// close. No-op if nothing's open.
    fn close_phoenix_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(phoenix) = self.state.o_phoenix.as_ref() else {
            return;
        };
        let Some(market) = phoenix.markets().iter().find(|m| m.symbol_str() == symbol) else {
            return;
        };
        let Some(base_lot_position) = self.phoenix_position(symbol) else {
            return;
        };
        let asset_id = market.asset_id;
        let side = if base_lot_position > 0 {
            Side::Ask
        } else {
            Side::Bid
        };
        let size = base_lot_position.unsigned_abs();
        let client_order_id = self.state.last_slot as u128;

        log_warn!(
            "testperpv1: funding cycle: closing Phoenix leg {} side={:?} size={}",
            symbol,
            side,
            size,
        );
        if let Err(e) =
            phoenix.place_market_order(owner, asset_id, side, size, 0, client_order_id, self.wallet)
        {
            log_error!(
                "testperpv1: funding cycle: Phoenix close {} failed: {}",
                symbol,
                e
            );
        }
    }

    /// Opens the deposit-hedge direction of the basis trade for `symbol`:
    /// short the Phoenix perp (`open_phoenix_leg`'s existing core --
    /// bootstrap/already-open gating/sizing/`place_market_order`, unchanged)
    /// plus a deposit of the underlying asset on `protocol` to stay
    /// delta-neutral -- see [`decide_basis_trade`]'s doc comment for why
    /// this direction never needs to compare against any lending rate.
    /// `protocol` should come from [`Self::best_supply_apy`] at the call
    /// site (whichever protocol pays the best yield on this deposit).
    fn open_deposit_hedge_leg(
        &mut self,
        symbol: &str,
        protocol: LendingProtocol,
        notional_usd: f64,
    ) {
        self.open_phoenix_leg(symbol, false, notional_usd);
        match protocol {
            LendingProtocol::Solend => self.open_solend_deposit_leg(symbol, notional_usd),
            LendingProtocol::Kamino => self.open_kamino_deposit_leg(symbol, notional_usd),
        }
    }

    /// Solend half of the deposit-hedge direction: swap `notional_usd`
    /// worth of USDC into the underlying, then deposit it as obligation
    /// collateral -- must be the *same* asset as the perp leg to actually
    /// hedge delta (USDC collateral wouldn't offset a SOL perp's delta).
    /// Bootstraps the obligation instead of depositing if it isn't
    /// registered yet -- see `bootstrap_solend_obligation`'s doc comment.
    /// Does nothing if a deposit already exists in this reserve (open
    /// once, hold, let the close-decision handle the exit -- same
    /// reasoning as `open_phoenix_leg`'s own already-open gate).
    ///
    /// The deposit amount is estimated from `reserve.price_usd`, not the
    /// spot swap's real (slippage-affected) output -- both instructions
    /// land in the same transaction (queued onto the same `self.wallet`,
    /// assembled together in `evaluate()`'s tail), so if the estimate
    /// overshoots what the swap actually produced, the deposit simply
    /// fails on-chain (insufficient balance) rather than depositing a
    /// wrong amount. Only refreshes *this* reserve/obligation pair before
    /// depositing, not every reserve the obligation might hold a position
    /// in elsewhere -- a known simplification for the common case of one
    /// active symbol at a time, flagged rather than silently assumed
    /// complete.
    fn open_solend_deposit_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self
            .state
            .o_solend_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_solend_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };

        let already_deposited = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("testperpv1: basis trade: {} has no Solend oracle price yet, skipping deposit-hedge", symbol);
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
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else {
            return;
        };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: opening Solend deposit-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} spot swap failed: {}",
                symbol,
                e
            );
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        if let Err(e) = reserve.deposit(
            reserve_id,
            obligation_id,
            amount_raw,
            owner,
            underlying_ata,
            collateral_ata,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} solend deposit failed: {}",
                symbol,
                e
            );
        }
    }

    /// Opens the borrow-hedge direction of the basis trade for `symbol`:
    /// long the Phoenix perp plus a borrow of the underlying on
    /// `protocol`, immediately sold for USDC (synthetic short) -- only
    /// reached when [`decide_basis_trade`] confirms the funding collected
    /// exceeds the real borrow APY. `protocol` should come from
    /// [`Self::best_borrow_apy`] at the call site (the same protocol
    /// whose rate cleared the profitability bar).
    fn open_borrow_hedge_leg(
        &mut self,
        symbol: &str,
        protocol: LendingProtocol,
        notional_usd: f64,
    ) {
        self.open_phoenix_leg(symbol, true, notional_usd);
        match protocol {
            LendingProtocol::Solend => self.open_solend_borrow_leg(symbol, notional_usd),
            LendingProtocol::Kamino => self.open_kamino_borrow_leg(symbol, notional_usd),
        }
    }

    /// Solend half of the borrow-hedge direction, split across two
    /// stages (same "confirm via a real `on_account` update before the
    /// next step" discipline as `bootstrap_phoenix_trader`/
    /// `bootstrap_solend_obligation" -- never batches a fresh deposit
    /// and a borrow against it in the same transaction):
    ///
    /// 1. If the obligation has no USDC collateral yet, deposit
    ///    `notional_usd` worth of USDC (no swap needed -- USDC in, USDC
    ///    deposited) and return, deferring the borrow to the next epoch.
    /// 2. Once USDC collateral is confirmed, borrow `notional_usd` worth
    ///    of the underlying against it and immediately sell the borrowed
    ///    amount for USDC in the *same* transaction -- unlike the
    ///    deposit-hedge leg, this needs no estimate: the borrowed amount
    ///    is a parameter this code chooses itself, not a swap's output,
    ///    so the sell step already knows the exact amount to move.
    fn open_solend_borrow_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self
            .state
            .o_solend_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_solend_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            return;
        };
        let usdc_collateral_mint = usdc_reserve.collateral_mint;

        let has_usdc_collateral = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some();

        const USDC_DECIMALS: i32 = 6;

        if !has_usdc_collateral {
            // Solend requires deposited collateral to be worth strictly
            // more than what's later borrowed against it (LTV < 1.0, see
            // `SolendReserve::loan_to_value_pct`) -- depositing exactly
            // `notional_usd` and then borrowing `notional_usd` against it
            // always reverts with `BorrowTooLarge` (live-confirmed via
            // `solana confirm`, custom program error 0x1a, during this
            // phase, [11/16]). Target 90% of the reserve's actual LTV, not
            // the raw boundary, for headroom against oracle-price drift
            // between this calc and the on-chain check.
            const LTV_SAFETY_FACTOR: f64 = 0.9;
            let collateral_usd =
                notional_usd / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
            let usdc_amount_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
                return;
            };
            let Some(usdc_collateral_ata) =
                self.wallet.append_create_ata(owner, usdc_collateral_mint)
            else {
                return;
            };
            log_warn!(
                "testperpv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} borrow-hedge",
                collateral_usd,
                symbol,
                notional_usd,
            );
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!(
                    "testperpv1: basis trade: borrow-hedge {} refresh_reserve failed: {}",
                    symbol,
                    e
                );
                return;
            }
            let refresh_reserves = self.solend_refresh_reserves();
            if let Err(e) =
                solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet)
            {
                log_error!(
                    "testperpv1: basis trade: borrow-hedge {} refresh_obligation failed: {}",
                    symbol,
                    e
                );
                return;
            }
            if let Err(e) = usdc_reserve.deposit(
                usdc_reserve_id,
                obligation_id,
                usdc_amount_raw,
                owner,
                usdc_ata,
                usdc_collateral_ata,
                self.wallet,
            ) {
                log_error!("testperpv1: basis trade: borrow-hedge {} USDC collateral deposit failed: {}", symbol, e);
            }
            return;
        }

        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };
        let already_borrowed = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("testperpv1: basis trade: {} has no Solend oracle price yet, skipping borrow-hedge", symbol);
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: opening Solend borrow-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} USDC refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        let deposit_reserves = self.solend_obligation_deposit_reserves();
        if let Err(e) = reserve.borrow(
            reserve_id,
            obligation_id,
            borrow_amount_raw,
            owner,
            underlying_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} solend borrow failed: {}",
                symbol,
                e
            );
            return;
        }
        // Sell the borrowed underlying for USDC -- realizes the
        // synthetic short. Exact known amount, no estimate needed (see
        // this method's doc comment).
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} spot sell failed: {}",
                symbol,
                e
            );
        }
    }

    /// Kamino half of the deposit-hedge direction: swap `notional_usd`
    /// worth of USDC into the underlying, then deposit it as obligation
    /// collateral -- same role as [`Self::open_solend_deposit_leg`], but
    /// Kamino's real, simpler API: no separate collateral-mint ATA needed
    /// (Kamino mints cTokens straight into the obligation, confirmed via
    /// the verified account list), and `refresh_reserve` needs this
    /// reserve's own oracle account set threaded through (see
    /// `KaminoReserve::refresh_reserve`'s doc comment). Bootstraps the
    /// obligation instead of depositing if it isn't registered yet. Does
    /// nothing if a deposit already exists in this reserve -- same
    /// open-once-hold reasoning as `open_solend_deposit_leg`.
    fn open_kamino_deposit_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_kamino_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };

        let already_deposited = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(reserve_id))
            .is_some();
        if already_deposited {
            return;
        }

        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("testperpv1: basis trade: {} has no Kamino oracle price yet, skipping deposit-hedge", symbol);
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
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: opening Kamino deposit-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} kamino spot swap failed: {}",
                symbol,
                e
            );
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} kamino refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} kamino refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        if !self.ensure_kamino_farm_ready(
            reserve_id,
            reserve.lending_market,
            reserve.farm_collateral,
            0,
        ) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        if let Err(e) = reserve.deposit(
            reserve_id,
            obligation_id,
            amount_raw,
            owner,
            underlying_ata,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: deposit-hedge {} kamino deposit failed: {}",
                symbol,
                e
            );
        }
    }

    /// Opens the borrow-hedge direction of the basis trade for `symbol`
    /// via Kamino -- same two-stage split as [`Self::open_solend_borrow_leg`]
    /// (deposit USDC collateral first epoch if none yet, else borrow +
    /// sell in one tx), but with Kamino's real API: no separate
    /// collateral ATA on deposit, `borrow` needs a `referrer_token_state`
    /// (always `None` -- this bot never sets one up) and a
    /// `deposit_reserves_for_elevation` list (reused from
    /// [`Self::kamino_obligation_deposit_reserves`]; harmless to include
    /// even outside an elevation group).
    fn open_kamino_borrow_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_kamino_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };

        const USDC_DECIMALS: i32 = 6;

        // Same over-collateralization requirement (and same
        // `BorrowTooLarge`-equivalent revert if violated) as
        // `open_solend_borrow_leg`'s identical fix -- see that function's
        // doc comment for the live-confirmed root cause. Also scaled by
        // the *borrow* side's `borrow_factor_pct` (see
        // `KaminoReserve::borrow_factor_pct`'s doc comment) -- SOL's real
        // reserve is 1.25x, so a $1 borrow counts as $1.25 against the
        // USDC deposit's max-borrow-value limit. Without this, a borrow
        // sized only against the deposit-side LTV reverts on-chain with
        // `BorrowTooLarge` every time, live-confirmed.
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some((_, borrow_reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        let collateral_usd = notional_usd * borrow_reserve.borrow_factor_pct
            / (usdc_reserve.loan_to_value_pct * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        // `deposited_amount` is in cToken units, not raw USDC (see
        // `KaminoCollateral::deposited_amount`'s doc comment) -- Kamino's
        // cToken exchange rate only ever rises above 1:1 as interest
        // accrues, so treating the raw cToken count as a lower bound on
        // underlying USDC value is conservative (never under-collateralizes;
        // worst case is a harmless extra top-up deposit). This also self-
        // heals a stale on-chain obligation that was under-collateralized
        // by an earlier version of this formula (live-confirmed: a prior
        // deposit sized without `borrow_factor_pct` persists across
        // restarts on this obligation's deterministic PDA and otherwise
        // reverts with `BorrowTooLarge` forever, since presence alone was
        // treated as "enough").
        let has_enough_usdc_collateral = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount >= required_usdc_raw);

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
                return;
            };
            log_warn!(
                "testperpv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} Kamino borrow-hedge",
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
                log_error!("testperpv1: basis trade: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
                return;
            }
            let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
            if let Err(e) = kamino::refresh_obligation(
                usdc_reserve.lending_market,
                obligation_id,
                &deposit_reserves,
                &borrow_reserves,
                self.wallet,
            ) {
                log_error!("testperpv1: basis trade: borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
                return;
            }
            if !self.ensure_kamino_farm_ready(
                usdc_reserve_id,
                usdc_reserve.lending_market,
                usdc_reserve.farm_collateral,
                0,
            ) {
                return;
            }
            let Some(dex) = self.state.o_dex.as_ref() else {
                return;
            };
            let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc)
            else {
                return;
            };
            if let Err(e) = usdc_reserve.deposit(
                usdc_reserve_id,
                obligation_id,
                usdc_amount_raw,
                owner,
                usdc_ata,
                self.wallet,
            ) {
                log_error!("testperpv1: basis trade: borrow-hedge {} kamino USDC collateral deposit failed: {}", symbol, e);
            }
            return;
        }

        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        let already_borrowed = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let price_usd = reserve.price_usd;
        if price_usd <= 0.0 {
            log_error!("testperpv1: basis trade: {} has no Kamino oracle price yet, skipping borrow-hedge", symbol);
            return;
        }
        let decimals = reserve.mint_decimals as i32;
        let borrow_amount_raw = ((notional_usd / price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: opening Kamino borrow-hedge {} (${:.2})",
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
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} kamino refresh_reserve failed: {}",
                symbol,
                e
            );
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
            log_error!("testperpv1: basis trade: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} kamino refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        if !self.ensure_kamino_farm_ready(reserve_id, reserve.lending_market, reserve.farm_debt, 1)
        {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
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
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} kamino borrow failed: {}",
                symbol,
                e
            );
            return;
        }
        // Sell the borrowed underlying for USDC -- realizes the
        // synthetic short. Exact known amount, no estimate needed (see
        // `open_solend_borrow_leg`'s doc comment for why).
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} kamino spot sell failed: {}",
                symbol,
                e
            );
        }
    }

    /// marginfi half of the borrow-hedge direction, split across the same
    /// two stages as [`Self::open_solend_borrow_leg`]/[`Self::
    /// open_kamino_borrow_leg`], but marginfi's real, simpler API (no
    /// refresh step; `other_active_banks` supplied directly from
    /// [`marginfi::MarginfiPosition::other_active_banks`], which this bot
    /// doesn't otherwise track):
    ///
    /// 1. If the account has no USDC balance yet, deposit `notional_usd`
    ///    worth of USDC and return, deferring the borrow to the next
    ///    epoch.
    /// 2. Once USDC collateral is confirmed, borrow `notional_usd` worth
    ///    of the underlying against it and immediately sell it for USDC
    ///    in the same transaction.
    fn open_marginfi_borrow_leg(&mut self, symbol: &str, notional_usd: f64) {
        if !self
            .state
            .o_marginfi_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_marginfi_account();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(marginfi_account) = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.account_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_bank_id, usdc_bank)) = dex.marginfi().reserve_by_mint(mint_usdc) else {
            return;
        };
        let group = account_id_from_pubkey(&marginfi::MARGINFI_MAIN_GROUP);

        const USDC_DECIMALS: i32 = 6;

        // Same over-collateralization requirement (and same
        // `BorrowTooLarge`-equivalent revert if violated) as
        // `open_solend_borrow_leg`'s identical fix -- see that function's
        // doc comment for the live-confirmed root cause. Also scaled by
        // the *borrowed* asset's `liability_weight_init` (marginfi's own
        // risk-weight multiplier, see `MarginfiBank`'s doc comment --
        // consistently >= 1.0, same role as Kamino's `borrow_factor_pct`):
        // a $1 SOL borrow counts as more than $1 against the USDC
        // deposit's health-check limit. Without this, marginfi's risk
        // engine reverts on-chain with `RiskEngineInitRejected`
        // ("bad health or stale oracles"), live-confirmed this session.
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some((_, borrow_bank)) = dex.marginfi().reserve_by_mint(mint) else {
            return;
        };
        const LTV_SAFETY_FACTOR: f64 = 0.9;
        let collateral_usd = notional_usd * borrow_bank.liability_weight_init
            / (usdc_bank.asset_weight_init * LTV_SAFETY_FACTOR);
        let required_usdc_raw = (collateral_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;

        // `asset_shares` is a share count, not raw USDC (see
        // `MarginfiBalance`'s doc comment) -- multiply by the bank's
        // current `asset_share_value` (grows over time via accrued
        // interest, so this is the real current underlying value, not
        // just a bound) to compare against the required raw amount. This
        // also self-heals a stale on-chain deposit sized without
        // `liability_weight_init` by an earlier version of this formula
        // (same precedent as Kamino's identical top-up fix).
        let has_enough_usdc_collateral = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.deposit_for(usdc_bank_id))
            .is_some_and(|b| {
                (b.asset_shares * usdc_bank.asset_share_value).round() as u64 >= required_usdc_raw
            });

        if !has_enough_usdc_collateral {
            let usdc_amount_raw = required_usdc_raw;
            if usdc_amount_raw == 0 {
                return;
            }
            let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
                return;
            };
            log_warn!(
                "testperpv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} marginfi borrow-hedge",
                collateral_usd,
                symbol,
                notional_usd,
            );
            if let Err(e) = dex.marginfi().deposit(
                usdc_bank_id,
                group,
                marginfi_account,
                owner,
                usdc_ata,
                usdc_amount_raw,
                false,
                self.wallet,
            ) {
                log_error!("testperpv1: basis trade: borrow-hedge {} marginfi USDC collateral deposit failed: {}", symbol, e);
            }
            return;
        }

        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some((bank_id, bank)) = dex.marginfi().reserve_by_mint(mint) else {
            return;
        };
        let already_borrowed = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.borrow_for(bank_id))
            .is_some();
        if already_borrowed {
            return;
        }
        let Some(price) = dex.marginfi().price_for_bank(bank_id) else {
            log_error!("testperpv1: basis trade: {} has no marginfi oracle price yet, skipping borrow-hedge", symbol);
            return;
        };
        if price.price_usd <= 0.0 {
            return;
        }
        // Reject a stale oracle price *before* ever building the borrow
        // instruction, using marginfi's own real per-bank threshold
        // (`bank.oracle_max_age`) against the Switchboard feed's own
        // recorded update time -- matches what marginfi's on-chain
        // `Clock::unix_timestamp` check will see, so a doomed transaction
        // never gets sent. See `pyth::OraclePrice::last_update_timestamp`'s
        // doc comment for the live-confirmed incident this prevents
        // (`SwitchboardStalePrice`, a real feed observed ~40+ minutes
        // stale between external cranks).
        if let Some(oracle_ts) = price.last_update_timestamp {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_secs() as i64;
            if now.saturating_sub(oracle_ts) > bank.oracle_max_age as i64 {
                log_error!(
                    "testperpv1: basis trade: {} marginfi oracle stale ({}s old, max age {}s), skipping borrow-hedge",
                    symbol,
                    now.saturating_sub(oracle_ts),
                    bank.oracle_max_age,
                );
                return;
            }
        }
        let decimals = bank.mint_decimals as i32;
        let borrow_amount_raw =
            ((notional_usd / price.price_usd) * 10f64.powi(decimals)).round() as u64;
        if borrow_amount_raw == 0 {
            return;
        }
        let Some(underlying_ata) = self.wallet.append_create_ata(owner, mint) else {
            return;
        };
        let other_active_banks = self
            .state
            .o_marginfi_position
            .as_ref()
            .map(|s| s.other_active_banks(bank_id))
            .unwrap_or_default();

        log_warn!(
            "testperpv1: basis trade: opening marginfi borrow-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = dex.marginfi().borrow(
            bank_id,
            group,
            marginfi_account,
            owner,
            underlying_ata,
            borrow_amount_raw,
            &other_active_banks,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} marginfi borrow failed: {}",
                symbol,
                e
            );
            return;
        }
        // Sell the borrowed underlying for USDC -- realizes the
        // synthetic short. Exact known amount, no estimate needed (see
        // `open_solend_borrow_leg`'s doc comment for why).
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, borrow_amount_raw) {
            log_error!(
                "testperpv1: basis trade: borrow-hedge {} marginfi spot sell failed: {}",
                symbol,
                e
            );
        }
    }

    /// Closes the deposit-hedge direction for `symbol`: flattens the
    /// Phoenix leg (`close_phoenix_leg`, unchanged) and withdraws + sells
    /// the deposit on whichever `protocol` actually holds it -- should
    /// come from [`Self::holding_lending_protocol`] at the call site.
    fn close_deposit_hedge_leg(&mut self, symbol: &str, protocol: LendingProtocol) {
        self.close_phoenix_leg(symbol);
        match protocol {
            LendingProtocol::Solend => self.close_solend_deposit_leg(symbol),
            LendingProtocol::Kamino => self.close_kamino_deposit_leg(symbol),
        }
    }

    /// Withdraws the real, currently-deposited collateral amount (from
    /// the live obligation, not an estimate) and sells it back to USDC.
    /// The withdrawal's real underlying payout can't be known exactly
    /// ahead of time (depends on Solend's live exchange rate, which this
    /// bot doesn't track), so the sell step reuses `FUNDING_CYCLE_MIN_MARGIN_USD`
    /// at the current price as an estimate -- same "same transaction,
    /// fails safely if wrong" reasoning as the open-side deposit
    /// estimate. No-op if nothing's deposited in this reserve.
    fn close_solend_deposit_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };

        let Some(collateral_amount) = self
            .state
            .o_solend_position
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
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else {
            return;
        };
        let Some(collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: closing Solend deposit-hedge {}",
            symbol
        );
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        let deposit_reserves = self.solend_obligation_deposit_reserves();
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
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} solend withdraw failed: {}",
                symbol,
                e
            );
            return;
        }
        let estimated_underlying_raw =
            ((FUNDING_CYCLE_MIN_MARGIN_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw) {
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} spot sell failed: {}",
                symbol,
                e
            );
        }
    }

    /// Closes the borrow-hedge direction for `symbol`: flattens the
    /// Phoenix leg (`close_phoenix_leg`, unchanged) and buys back +
    /// repays the loan on whichever `protocol` actually holds it -- should
    /// come from [`Self::holding_lending_protocol`] at the call site.
    fn close_borrow_hedge_leg(&mut self, symbol: &str, protocol: LendingProtocol) {
        self.close_phoenix_leg(symbol);
        match protocol {
            LendingProtocol::Solend => self.close_solend_borrow_leg(symbol),
            LendingProtocol::Kamino => self.close_kamino_borrow_leg(symbol),
        }
    }

    /// Buys back the real, currently-borrowed amount (from the live
    /// obligation) with USDC, then repays the loan with
    /// [`solend::SOLEND_AMOUNT_MAX`] (repay everything owed, robust to
    /// small over/under-buys from the swap's own slippage) -- USDC
    /// collateral stays deposited (matches the established "collateral
    /// stays deployed for reuse" precedent), not withdrawn. No-op if
    /// nothing's borrowed against this reserve.
    fn close_solend_borrow_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };

        let Some(borrowed_amount) = self
            .state
            .o_solend_position
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
        let usdc_needed_raw = ((borrowed_amount as f64 / 10f64.powi(decimals))
            * price_usd
            * 10f64.powi(USDC_DECIMALS))
        .round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        // Skip the buy-back if a prior (already-confirmed) attempt already
        // left enough of the underlying sitting in the wallet -- same
        // reasoning and precedent as `close_kamino_borrow_leg`'s identical
        // fix: this function gets retried on the standard cooldown until
        // the repay itself confirms, and without this check every retry
        // re-buys the full amount again even though an earlier swap
        // already landed. Also self-heals the case where a single swap's
        // real slippage came up just short of `borrowed_amount` (an exact
        // repay reverts on-chain with `insufficient funds`, live-confirmed
        // this session) -- the next retry tops up the shortfall instead
        // of repeating the same undersized swap.
        let underlying_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!(
                "testperpv1: basis trade: closing Solend borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "testperpv1: basis trade: close borrow-hedge {} buy-back failed: {}",
                    symbol,
                    e
                );
            }
            // Defer refresh+repay to the next cycle regardless of whether
            // the swap above succeeded or failed -- `execute_spot_leg`
            // only queues instructions, it doesn't wait for on-chain
            // confirmation, so falling through into refresh+repay in this
            // same call races the just-queued swap: both end up sent in
            // the same batch with no ordering guarantee between them,
            // and a repay that lands before its own swap reverts with the
            // same `insufficient funds` this whole fix was for
            // (live-confirmed this session: the swap finalized fine, the
            // repay sent alongside it still failed). The next retry will
            // see the swap's real, landed balance and proceed straight to
            // refresh+repay.
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.solend().reserve_by_mint(mint) else {
            return;
        };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else {
            return;
        };
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "testperpv1: basis trade: close borrow-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        // `refresh_obligation` (below) requires *every* reserve currently
        // in the obligation -- not just the one being repaid -- to have
        // been individually refreshed in this same transaction, or it
        // fails with its own `ReserveStale` (live-confirmed: `solana
        // confirm` on a real repay attempt here returned `custom program
        // error: 0x16`). `open_solend_borrow_leg` already refreshes both
        // the USDC collateral reserve and the target reserve before its
        // own `refresh_obligation` call for exactly this reason -- this
        // close-side leg was missing the USDC half.
        let mint_usdc = self.configuration.mint_usdc;
        if let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) {
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!(
                    "testperpv1: basis trade: close borrow-hedge {} USDC refresh_reserve failed: {}",
                    symbol,
                    e
                );
                return;
            }
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "testperpv1: basis trade: close borrow-hedge {} refresh_obligation failed: {}",
                symbol,
                e
            );
            return;
        }
        if let Err(e) = reserve.repay(
            reserve_id,
            obligation_id,
            solend::SOLEND_AMOUNT_MAX,
            owner,
            underlying_ata,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: close borrow-hedge {} solend repay failed: {}",
                symbol,
                e
            );
        }
    }

    /// Kamino half of closing the deposit-hedge direction -- same role as
    /// [`Self::close_solend_deposit_leg`], but withdraws via
    /// [`kamino::KAMINO_AMOUNT_MAX`] rather than a read amount:
    /// `KaminoCollateral::deposited_amount` is in cToken units (see its
    /// doc comment), and `KaminoReserve::withdraw` accepts
    /// `KAMINO_AMOUNT_MAX` for "this reserve's entire deposited amount"
    /// directly, sidestepping the cToken-to-underlying exchange-rate
    /// conversion this bot doesn't track. No-op if nothing's deposited in
    /// this reserve.
    fn close_kamino_deposit_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };

        let has_deposit = self
            .state
            .o_kamino_position
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
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else {
            return;
        };

        log_warn!(
            "testperpv1: basis trade: closing Kamino deposit-hedge {}",
            symbol
        );
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("testperpv1: basis trade: close deposit-hedge {} kamino refresh_reserve failed: {}", symbol, e);
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("testperpv1: basis trade: close deposit-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_kamino_farm_ready(
            reserve_id,
            reserve.lending_market,
            reserve.farm_collateral,
            0,
        ) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        if let Err(e) = reserve.withdraw(
            reserve_id,
            obligation_id,
            kamino::KAMINO_AMOUNT_MAX,
            owner,
            underlying_ata,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} kamino withdraw failed: {}",
                symbol,
                e
            );
            return;
        }
        let estimated_underlying_raw =
            ((FUNDING_CYCLE_MIN_MARGIN_USD / price_usd) * 10f64.powi(decimals)).round() as u64;
        if estimated_underlying_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;
        if let Err(e) = self.execute_spot_leg(mint, mint_usdc, estimated_underlying_raw) {
            log_error!(
                "testperpv1: basis trade: close deposit-hedge {} kamino spot sell failed: {}",
                symbol,
                e
            );
        }
    }

    /// Kamino half of closing the borrow-hedge direction -- same role as
    /// [`Self::close_solend_borrow_leg`]: buys back the real,
    /// currently-borrowed amount with USDC, then repays with
    /// [`kamino::KAMINO_AMOUNT_MAX`] (repay everything owed). USDC
    /// collateral stays deposited, not withdrawn -- same "collateral
    /// stays deployed for reuse" precedent. No-op if nothing's borrowed
    /// against this reserve.
    fn close_kamino_borrow_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };

        let Some(borrowed_amount) = self
            .state
            .o_kamino_position
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
        let usdc_needed_raw = ((borrowed_amount as f64 / 10f64.powi(decimals))
            * price_usd
            * 10f64.powi(USDC_DECIMALS))
        .round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        // Skip the buy-back if a prior (already-confirmed) attempt already
        // left enough of the underlying sitting in the wallet -- this
        // function gets retried on the standard cooldown until the repay
        // itself confirms, and without this check every retry re-buys the
        // full amount again even though an earlier swap already landed.
        // Also matters structurally: keeping this stage's instruction set
        // small (just the two refreshes + repay, no swap) makes it much
        // less likely to get split across two transactions by
        // `Wallet::assemble()`'s size limit -- live-confirmed this
        // session that a split here is a real bug, not just extra fees:
        // Solana doesn't guarantee same-slot transactions execute in send
        // order, so a repay landing in a *different* transaction than its
        // own refresh can see stale reserve data even when both land in
        // the same slot, reverting with `ReserveStale`.
        let underlying_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if underlying_balance_raw < borrowed_amount {
            log_warn!(
                "testperpv1: basis trade: closing Kamino borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "testperpv1: basis trade: close borrow-hedge {} kamino buy-back failed: {}",
                    symbol,
                    e
                );
            }
            // Defer refresh+repay to the next cycle regardless of whether
            // the swap above succeeded or failed -- see
            // `close_solend_borrow_leg`'s identical fix for why: falling
            // through into refresh+repay in this same call races the
            // just-queued swap (no ordering guarantee between separately-
            // queued instruction batches), live-confirmed this session to
            // still revert with `insufficient funds` even when the swap
            // itself finalizes fine.
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else {
            return;
        };
        if let Err(e) = reserve.refresh_reserve(
            reserve_id,
            reserve.pyth_oracle,
            reserve.switchboard_price_oracle,
            reserve.switchboard_twap_oracle,
            reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("testperpv1: basis trade: close borrow-hedge {} kamino refresh_reserve failed: {}", symbol, e);
            return;
        }
        // Same reasoning as `close_solend_borrow_leg`'s identical fix:
        // `refresh_obligation` needs every reserve in the obligation --
        // not just the one being repaid -- individually refreshed in this
        // same transaction first, and `open_kamino_borrow_leg` already
        // does this for USDC; this close-side leg was missing it.
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
                    "testperpv1: basis trade: close borrow-hedge {} kamino USDC refresh_reserve failed: {}",
                    symbol,
                    e
                );
                return;
            }
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("testperpv1: basis trade: close borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
            return;
        }
        if !self.ensure_kamino_farm_ready(reserve_id, reserve.lending_market, reserve.farm_debt, 1)
        {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((reserve_id, reserve)) = dex.kamino().reserve_by_mint(mint) else {
            return;
        };
        if let Err(e) = reserve.repay(
            reserve_id,
            obligation_id,
            kamino::KAMINO_AMOUNT_MAX,
            owner,
            underlying_ata,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: close borrow-hedge {} kamino repay failed: {}",
                symbol,
                e
            );
        }
    }

    /// marginfi half of closing the borrow-hedge direction -- same role as
    /// [`Self::close_solend_borrow_leg`]/[`Self::close_kamino_borrow_leg`]:
    /// buys back the real, currently-borrowed amount with USDC (converted
    /// from raw liability shares via the bank's own
    /// `liability_share_value`, the same conversion
    /// `MarginfiBank::utilization` uses), then repays with `repay_all =
    /// true` (`amount` ignored by the program in that case). USDC
    /// collateral stays deposited, not withdrawn -- same "collateral stays
    /// deployed for reuse" precedent. No-op if nothing's borrowed against
    /// this bank.
    fn close_marginfi_borrow_leg(&mut self, symbol: &str) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(marginfi_account) = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.account_id())
        else {
            return;
        };
        let Some(mint) = resolve_symbol_mint(symbol) else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((bank_id, bank)) = dex.marginfi().reserve_by_mint(mint) else {
            return;
        };

        let Some(liability_shares) = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.borrow_for(bank_id))
            .map(|b| b.liability_shares)
            .filter(|&s| s > 0.0)
        else {
            return;
        };
        let Some(price) = dex.marginfi().price_for_bank(bank_id) else {
            return;
        };
        if price.price_usd <= 0.0 {
            return;
        }
        let decimals = bank.mint_decimals as i32;
        let borrowed_amount_raw = liability_shares * bank.liability_share_value;
        const USDC_DECIMALS: i32 = 6;
        let usdc_needed_raw = ((borrowed_amount_raw / 10f64.powi(decimals))
            * price.price_usd
            * 10f64.powi(USDC_DECIMALS))
        .round() as u64;
        if usdc_needed_raw == 0 {
            return;
        }
        let mint_usdc = self.configuration.mint_usdc;

        // Skip the buy-back if a prior (already-confirmed) attempt already
        // left enough of the underlying sitting in the wallet -- same
        // reasoning and precedent as `close_kamino_borrow_leg`'s identical
        // fix: this function gets retried until the repay itself confirms,
        // and without this check every retry re-buys the full amount
        // again even though an earlier swap already landed. Also
        // self-heals the case where a single swap's real slippage came up
        // just short of `borrowed_amount_raw` (an exact repay would
        // revert on-chain with `insufficient funds`, same live-confirmed
        // failure mode as Solend's identical leg) -- the next retry tops
        // up the shortfall instead of repeating the same undersized swap.
        let underlying_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if (underlying_balance_raw as f64) < borrowed_amount_raw {
            log_warn!(
                "testperpv1: basis trade: closing marginfi borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "testperpv1: basis trade: close borrow-hedge {} marginfi buy-back failed: {}",
                    symbol,
                    e
                );
            }
            // Defer repay to the next cycle regardless of whether the
            // swap above succeeded or failed -- see
            // `close_solend_borrow_leg`'s identical fix for why: falling
            // through in this same call races the just-queued swap.
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((bank_id, _)) = dex.marginfi().reserve_by_mint(mint) else {
            return;
        };
        let Some(underlying_ata) = self.wallet.derive_ata(owner, mint) else {
            return;
        };
        let group = account_id_from_pubkey(&marginfi::MARGINFI_MAIN_GROUP);
        if let Err(e) = dex.marginfi().repay(
            bank_id,
            group,
            marginfi_account,
            owner,
            underlying_ata,
            0, // ignored -- repay_all = true
            true,
            self.wallet,
        ) {
            log_error!(
                "testperpv1: basis trade: close borrow-hedge {} marginfi repay failed: {}",
                symbol,
                e
            );
        }
    }

    /// Which lending protocol a currently-open basis-trade position for
    /// `symbol` used -- checks both `SolendPosition`/`KaminoPosition`'s
    /// live obligations for a deposit or borrow against that symbol's
    /// reserve. Mutually exclusive by construction (a position only ever
    /// opens on one protocol, decided once at open time -- see
    /// [`Self::best_borrow_apy`]/[`Self::best_supply_apy`]). `None` if
    /// neither protocol shows a position (nothing open, or reserve/
    /// obligation data not loaded yet).
    fn holding_lending_protocol(&self, symbol: &str) -> Option<LendingProtocol> {
        let mint = resolve_symbol_mint(symbol)?;
        let dex = self.state.o_dex.as_ref()?;

        if let Some((reserve_id, _)) = dex.solend().reserve_by_mint(mint) {
            if let Some(ob) = self
                .state
                .o_solend_position
                .as_ref()
                .and_then(|s| s.obligation())
            {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Solend);
                }
            }
        }
        if let Some((reserve_id, _)) = dex.kamino().reserve_by_mint(mint) {
            if let Some(ob) = self
                .state
                .o_kamino_position
                .as_ref()
                .and_then(|s| s.obligation())
            {
                if ob.deposit_for(reserve_id).is_some() || ob.borrow_for(reserve_id).is_some() {
                    return Some(LendingProtocol::Kamino);
                }
            }
        }
        None
    }

    /// Checks whether an open basis-trade position for `symbol` should
    /// close this epoch, and if so closes it. Which direction is
    /// "currently open" is read from the real Phoenix position sign
    /// (`> 0` = long = `BorrowHedge`, `< 0` = short = `DepositHedge`) --
    /// same "real on-chain state, not separate bookkeeping" discipline as
    /// the rest of this file. No-op if nothing's open for `symbol`, or if
    /// this epoch is missing rate/reserve data (hold rather than act on
    /// an incomplete read).
    fn close_basis_trade_if_needed(&mut self, symbol: &str) {
        let Some(phoenix_pos) = self.phoenix_position(symbol) else {
            return;
        };
        let currently_open = if phoenix_pos > 0 {
            BasisDirection::BorrowHedge
        } else {
            BasisDirection::DepositHedge
        };

        let Some(phoenix_rate) = self.state.router.pending_rate(PerpVenue::Phoenix, symbol) else {
            return;
        };
        let Some((_, borrow_apy_pct)) = self.best_borrow_apy(symbol) else {
            return;
        };

        if decide_basis_trade(phoenix_rate, borrow_apy_pct) == Some(currently_open) {
            return; // still profitable in the same direction, hold
        }
        // Which protocol the open position actually used -- read from
        // real on-chain state (`holding_lending_protocol`), not assumed.
        // Hold rather than close blindly if that can't be determined yet
        // (e.g. obligation data hasn't loaded), same "hold on incomplete
        // read" discipline as the rate/reserve checks above.
        let Some(protocol) = self.holding_lending_protocol(symbol) else {
            return;
        };
        log_warn!(
            "testperpv1: basis trade: closing {} -- direction reversed or no longer profitable",
            symbol,
        );
        match currently_open {
            BasisDirection::DepositHedge => self.close_deposit_hedge_leg(symbol, protocol),
            BasisDirection::BorrowHedge => self.close_borrow_hedge_leg(symbol, protocol),
        }
    }

    /// Deploys `amount_usd` of otherwise-idle USDC into whichever lending
    /// protocol currently pays the best USDC supply APY ([`Self::
    /// best_usdc_supply_apy`]) -- real, ~0-market-risk yield Solend/Kamino
    /// already pay on any deposited collateral (not just capital already
    /// committed to a borrow-hedge's USDC stage), that this bot previously
    /// left unclaimed whenever no symbol cleared the funding-rate bar this
    /// epoch. Called from [`Self::log_basis_cycles`]'s tail with whatever
    /// `spare_usdc` remains after this epoch's basis-trade opens. Below
    /// [`REBALANCE_DUST_THRESHOLD_USD`] is treated as noise, not deployed
    /// (same threshold `plan_rebalance_legs` already uses).
    ///
    /// Deliberately does **not** skip already-deposited collateral the way
    /// `open_solend_borrow_leg`/`open_kamino_borrow_leg`'s stage-1 does
    /// (`has_usdc_collateral` gate) -- idle deployment should keep adding
    /// capital every epoch as more accumulates, not stop after the first
    /// deposit. This still composes for free with the existing borrow-hedge
    /// logic: if a later epoch's borrow-hedge signal picks the *same*
    /// protocol, its own `has_usdc_collateral` check already treats this
    /// deposit as the collateral it needs and skips straight to borrowing.
    /// If a later signal needs *liquid* USDC (a deposit-hedge's spot swap)
    /// or collateral on the *other* protocol, this deposit isn't reachable
    /// that epoch -- deliberately out of scope for this pass (no automatic
    /// withdraw-and-reallocate); that position simply won't open until
    /// enough new liquid USDC arrives, same "hold on insufficient data"
    /// discipline the rest of this file already uses.
    fn deploy_idle_usdc(&mut self, amount_usd: f64) {
        if amount_usd < REBALANCE_DUST_THRESHOLD_USD {
            return;
        }
        let Some((protocol, apy_pct)) = self.best_usdc_supply_apy() else {
            return;
        };
        log_warn!(
            "testperpv1: idle capital: depositing ${:.2} USDC at {:?} (supply_apy={:.3}%)",
            amount_usd,
            protocol,
            apy_pct,
        );
        match protocol {
            LendingProtocol::Solend => self.deploy_idle_usdc_solend(amount_usd),
            LendingProtocol::Kamino => self.deploy_idle_usdc_kamino(amount_usd),
        }
    }

    /// Solend half of [`Self::deploy_idle_usdc`] -- same bootstrap-gate/
    /// refresh/deposit shape as `open_solend_borrow_leg`'s stage-1, minus
    /// its `has_usdc_collateral` skip (see [`Self::deploy_idle_usdc`]'s doc
    /// comment for why).
    fn deploy_idle_usdc_solend(&mut self, amount_usd: f64) {
        if !self
            .state
            .o_solend_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_solend_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            return;
        };

        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (amount_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_amount_raw == 0 {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        let collateral_mint = usdc_reserve.collateral_mint;
        let Some(usdc_collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint)
        else {
            return;
        };

        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!("testperpv1: idle capital: solend refresh_reserve failed: {e}");
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!("testperpv1: idle capital: solend refresh_obligation failed: {e}");
            return;
        }
        if let Err(e) = usdc_reserve.deposit(
            usdc_reserve_id,
            obligation_id,
            usdc_amount_raw,
            owner,
            usdc_ata,
            usdc_collateral_ata,
            self.wallet,
        ) {
            log_error!("testperpv1: idle capital: solend USDC deposit failed: {e}");
        }
    }

    /// Kamino half of [`Self::deploy_idle_usdc`] -- same shape as
    /// `open_kamino_borrow_leg`'s stage-1, minus its `has_usdc_collateral`
    /// skip. Also gated on [`Self::ensure_kamino_farm_ready`] -- Kamino's
    /// USDC reserve has a real Farms attachment (see `KaminoReserve::
    /// farm_collateral`'s doc comment), so a fresh obligation's very first
    /// idle deposit may need to bootstrap the farmer account first, same
    /// as any other Kamino USDC deposit.
    fn deploy_idle_usdc_kamino(&mut self, amount_usd: f64) {
        if !self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_kamino_obligation();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };

        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (amount_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_amount_raw == 0 {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };

        if let Err(e) = usdc_reserve.refresh_reserve(
            usdc_reserve_id,
            usdc_reserve.pyth_oracle,
            usdc_reserve.switchboard_price_oracle,
            usdc_reserve.switchboard_twap_oracle,
            usdc_reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("testperpv1: idle capital: kamino refresh_reserve failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            usdc_reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("testperpv1: idle capital: kamino refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(
            usdc_reserve_id,
            usdc_reserve.lending_market,
            usdc_reserve.farm_collateral,
            0,
        ) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        if let Err(e) = usdc_reserve.deposit(
            usdc_reserve_id,
            obligation_id,
            usdc_amount_raw,
            owner,
            usdc_ata,
            self.wallet,
        ) {
            log_error!("testperpv1: idle capital: kamino USDC deposit failed: {e}");
        }
    }

    /// marginfi half of [`Self::deploy_idle_usdc`] -- same bootstrap-gate/
    /// deposit shape as this file's own borrow-hedge USDC-collateral
    /// stage, minus its `has_usdc_collateral` skip. No
    /// refresh step needed -- marginfi has no `refresh_reserve`/
    /// `refresh_obligation` analog at all (confirmed: no such instruction
    /// exists -- `MarginfiState::deposit` takes the bank/account
    /// directly).
    fn deploy_idle_usdc_marginfi(&mut self, amount_usd: f64) {
        if !self
            .state
            .o_marginfi_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            self.bootstrap_marginfi_account();
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(marginfi_account) = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.account_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_bank_id, _)) = dex.marginfi().reserve_by_mint(mint_usdc) else {
            return;
        };

        const USDC_DECIMALS: i32 = 6;
        let usdc_amount_raw = (amount_usd * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if usdc_amount_raw == 0 {
            return;
        }
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        let group = account_id_from_pubkey(&marginfi::MARGINFI_MAIN_GROUP);

        if let Err(e) = dex.marginfi().deposit(
            usdc_bank_id,
            group,
            marginfi_account,
            owner,
            usdc_ata,
            usdc_amount_raw,
            false,
            self.wallet,
        ) {
            log_error!("testperpv1: idle capital: marginfi USDC deposit failed: {e}");
        }
    }

    /// Per-epoch entry point for the Phoenix-vs-lending-rate basis trade --
    /// replaces the old Phoenix-vs-Velocity `log_funding_graph_cycles`/
    /// `FinancialGraph`/SPFA cycle search (purpose-built for comparing
    /// *two perp venues*; a single perp-vs-lending-rate check per symbol
    /// doesn't need a cycle search). Close-checks every tracked symbol
    /// first (independent of whether a new position would be opened this
    /// epoch), then opens whatever's newly profitable, capital
    /// permitting -- `spare_usdc` is decremented in-memory as each open
    /// is queued (same greedy, most-recently-iterated-first discipline
    /// `select_capital_feasible_cycles` used to provide), since queuing
    /// an instruction doesn't change the wallet's real on-chain balance
    /// `current_usdc_value()` would otherwise keep re-reading as
    /// unspent. Protocol choice on open: borrow-hedge reuses
    /// `best_borrow_apy`'s own winner directly (same rate the
    /// profitability check already used); deposit-hedge asks
    /// `best_supply_apy` separately since it wasn't consulted for the
    /// open/close decision (see `decide_basis_trade`'s doc comment for
    /// why). Whatever `spare_usdc` is left once every symbol's been
    /// considered is genuinely idle this epoch -- deployed at a baseline
    /// yield via [`Self::deploy_idle_usdc`] rather than left unclaimed.
    fn log_basis_cycles(&mut self) {
        let assets = crate::symbol_mint_config::SYMBOL_MINT_MAP;
        for entry in assets.iter() {
            let symbol = std::str::from_utf8(&entry.symbol)
                .unwrap_or("")
                .trim_end_matches('\0');
            self.close_basis_trade_if_needed(symbol);
        }

        let mut spare_usdc = self.current_usdc_value();
        for entry in assets.iter() {
            if spare_usdc < FUNDING_CYCLE_MIN_MARGIN_USD {
                break;
            }
            let symbol = std::str::from_utf8(&entry.symbol)
                .unwrap_or("")
                .trim_end_matches('\0');
            if self.phoenix_position(symbol).is_some() {
                continue;
            }
            let Some(phoenix_rate) = self.state.router.pending_rate(PerpVenue::Phoenix, symbol)
            else {
                continue;
            };
            let Some((borrow_protocol, borrow_apy_pct)) = self.best_borrow_apy(symbol) else {
                continue;
            };

            let Some(direction) = decide_basis_trade(phoenix_rate, borrow_apy_pct) else {
                continue;
            };
            log_warn!(
                "testperpv1: basis trade: opening {} direction={:?} (phoenix_funding={:.3}% best_borrow_apy={:.3}%)",
                symbol,
                direction,
                phoenix_rate,
                borrow_apy_pct,
            );
            match direction {
                BasisDirection::DepositHedge => {
                    let Some((deposit_protocol, _)) = self.best_supply_apy(symbol) else {
                        continue;
                    };
                    self.open_deposit_hedge_leg(
                        symbol,
                        deposit_protocol,
                        FUNDING_CYCLE_MIN_MARGIN_USD,
                    );
                }
                BasisDirection::BorrowHedge => self.open_borrow_hedge_leg(
                    symbol,
                    borrow_protocol,
                    FUNDING_CYCLE_MIN_MARGIN_USD,
                ),
            }
            spare_usdc -= FUNDING_CYCLE_MIN_MARGIN_USD;
        }

        // Whatever's left over after this epoch's basis-trade opens is
        // genuinely idle -- put it to work at a real, ~0-risk baseline
        // yield instead of leaving it unclaimed in the wallet.
        self.deploy_idle_usdc(spare_usdc);
    }

    /// Real-transaction smoke test entry point -- see [`TestPhase`]'s doc
    /// comment for the full state machine this drives through. This mode
    /// never touches Phoenix or the funding-rate router, only
    /// Solend/Kamino/marginfi deposit+withdraw.
    pub(crate) fn evaluate(&mut self) {
        let t0 = std::time::Instant::now();
        self.evaluate_inner();
        // Accumulated (not logged here) -- same reasoning as
        // `low_latency()`'s own accumulate-don't-log comment: this fires
        // after every event, so logging every call would reintroduce the
        // log-volume problem already found to contribute to real
        // `stdio timeout` disconnects. `start()` logs and resets this.
        self.state.evaluate_elapsed_since_last_start += t0.elapsed();
        self.state.evaluate_count_since_last_start += 1;
    }

    fn evaluate_inner(&mut self) {
        self.wallet.set_priority_fee(PriorityLevel::Medium);
        // Idempotent/self-latching (2026-08-28): only actually queues the
        // real create-nonce transaction once (Wallet::ensure_bundler_nonce_created
        // no-ops on every call after the first, whether still unconfirmed
        // or already Ready) -- safe to call unconditionally every tick so
        // the durable-nonce account this wallet's Astralane landing path
        // (Wallet::send_bundler_pair, driven by set_priority_fee(High) --
        // see its own doc comment) needs is bootstrapped automatically at
        // wallet load time instead of requiring a manual trigger.
        if let Some(owner) = self.state.wallet() {
            self.wallet.ensure_bundler_nonce_created(owner);
        }
        if self.state.o_dex.is_none()
            || self.state.o_solend_position.is_none()
            || self.state.o_kamino_position.is_none()
            || self.state.o_marginfi_position.is_none()
        {
            return;
        }
        match self.state.test_phase {
            TestPhase::SwapToUsdc => self.test_swap_to_usdc(),
            TestPhase::BootstrapSolend => self.test_bootstrap_solend(),
            TestPhase::DepositSolend => self.test_deposit_solend(),
            TestPhase::WithdrawSolend => self.test_withdraw_solend(),
            TestPhase::BootstrapKamino => self.test_bootstrap_kamino(),
            TestPhase::DepositKamino => self.test_deposit_kamino(),
            TestPhase::WithdrawKamino => self.test_withdraw_kamino(),
            TestPhase::BootstrapMarginfi => self.test_bootstrap_marginfi(),
            TestPhase::DepositMarginfi => self.test_deposit_marginfi(),
            TestPhase::WithdrawMarginfi => self.test_withdraw_marginfi(),
            TestPhase::BorrowSolend => self.test_borrow_solend(),
            TestPhase::RepaySolend => self.test_repay_solend(),
            TestPhase::BorrowKamino => self.test_borrow_kamino(),
            TestPhase::RepayKamino => self.test_repay_kamino(),
            TestPhase::BorrowMarginfi => self.test_borrow_marginfi(),
            TestPhase::RepayMarginfi => self.test_repay_marginfi(),
            TestPhase::Done => {}
        }

        // Drains whatever this evaluate() call (or `on_message`'s Wallet
        // arm) built onto self.wallet and actually sends it -- same tail
        // `arbv1::state::evaluate` also uses. Without this, queued
        // instructions would sit on the wallet forever and never reach
        // the chain.
        for (sig, result) in self.wallet.drain_and_send() {
            match result {
                Ok(_) => {
                    log_warn!("testperpv1: sent transaction {sig}");
                }
                Err(e) => log_error!("testperpv1: failed to send transaction {sig}: {e}"),
            }
        }
    }

    /// Minimum slots to wait between an action and either retrying it
    /// (bootstrap/deposit) or advancing past it (withdraw) -- generous
    /// enough for a real transaction to land and confirm (real
    /// confirmations observed manually this session landed within a few
    /// seconds; this is deliberately several times that). `evaluate()`
    /// fires on every event, which can be many times per second, so this
    /// is what stops that from spamming duplicate transactions.
    const TEST_ACTION_COOLDOWN_SLOTS: Slot = 100;
    /// Deliberately small -- this only needs to prove the code path
    /// works, not move meaningful capital. Matches the conservative end
    /// of what was manually tested for real this session (5 USDC).
    const TEST_AMOUNT_USD: f64 = 1.0;
    /// How often (in slots) to report this wallet's most-referenced
    /// accounts back to the optimizer via `MessageSend::
    /// CommonAddressUpdate`, so `optimizer alt` can rank real Address
    /// Lookup Table candidates without an RPC history scan.
    const ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS: Slot = 500;
    /// Caps each report comfortably under `MESSAGE_MAX_SIZE` (4096
    /// bytes): `28 + 100*36 = 3628`.
    const ACCOUNT_USAGE_REPORT_MAX_ENTRIES: usize = 100;

    /// `true` while still within the cooldown window of the current
    /// phase's last action -- see [`Self::TEST_ACTION_COOLDOWN_SLOTS`].
    fn test_cooldown_active(&self) -> bool {
        match self.state.test_last_action_slot {
            Some(last) => {
                self.state.last_slot.saturating_sub(last) < Self::TEST_ACTION_COOLDOWN_SLOTS
            }
            None => false,
        }
    }

    /// Records that the current phase just sent a real action, starting
    /// its cooldown.
    fn test_mark_action(&mut self) {
        self.state.test_last_action_slot = Some(self.state.last_slot);
    }

    /// Moves to `next`, clearing the cooldown so the new phase starts
    /// its own action fresh rather than inheriting whatever was left
    /// from the phase just finished.
    fn test_advance(&mut self, next: TestPhase) {
        self.state.test_phase = next;
        self.state.test_last_action_slot = None;
    }

    /// Native lamports wrapped into wSOL and swapped to USDC per attempt
    /// -- deliberately small like [`Self::TEST_AMOUNT_USD`], just enough
    /// (at any plausible SOL price) to cover both protocols' $1 deposits
    /// plus routing slippage, while leaving the rest of the child
    /// wallet's SOL for transaction fees and obligation/collateral
    /// account rent across the whole remaining sequence.
    const TEST_WRAP_SOL_LAMPORTS: u64 = 50_000_000;
    /// Lamports the child wallet must keep unwrapped as a fee/rent
    /// buffer -- checked before wrapping so a low balance skips the
    /// swap (and logs why) instead of leaving the wallet unable to pay
    /// for its own transactions.
    const TEST_SOL_FEE_RESERVE_LAMPORTS: u64 = 10_000_000;

    /// [1/16] Wrap native SOL into wSOL and swap it to USDC via
    /// `execute_spot_leg`, so the deposit phases below have something to
    /// deposit -- see [`TestPhase`]'s doc comment for why this needs to
    /// happen first at all (the child wallet only ever receives raw
    /// native SOL, never USDC, from `eval.go`'s boot transfer).
    fn test_swap_to_usdc(&mut self) {
        let usdc_target = 2.0 * Self::TEST_AMOUNT_USD;
        if self.current_usdc_value() >= usdc_target {
            log_warn!("testperpv1: [1/16] USDC balance already covers both deposit tests");
            self.test_advance(TestPhase::BootstrapSolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };

        // A previous attempt may have already wrapped SOL into wSOL and
        // then failed at the swap step (e.g. a routing quote invalidated
        // between building and sending -- live-observed: pool put on a
        // multi-thousand-slot cooldown, real transaction only contained
        // the wrap instructions since execute_spot_leg errored before
        // appending any swap ones). Retry the swap against that leftover
        // wSOL directly instead of blindly wrapping more native SOL on
        // top of it -- the balance may no longer be enough to wrap again
        // (it wasn't, live-observed: 0.077 SOL - 0.05 wrapped = 0.027
        // left, below the 0.07 SOL this fn requires to wrap once more).
        let mint_sol = self.configuration.mint_sol;
        let mint_usdc = self.configuration.mint_usdc;
        let wsol_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint_sol, false)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        if wsol_balance_raw > 0 {
            log_warn!("testperpv1: [1/16] swapping {wsol_balance_raw} lamports of already-wrapped SOL to USDC");
            if let Err(e) = self.execute_spot_leg(mint_sol, mint_usdc, wsol_balance_raw) {
                log_error!("testperpv1: [1/16] swap to USDC failed: {e}");
            }
            self.test_mark_action();
            return;
        }

        let Some(sol_balance) = self.wallet.balance_sol(&owner) else {
            return;
        };
        let needed = Self::TEST_WRAP_SOL_LAMPORTS + Self::TEST_SOL_FEE_RESERVE_LAMPORTS;
        if sol_balance < needed {
            log_warn!(
                "testperpv1: [1/16] insufficient native SOL to wrap ({sol_balance} lamports, need {needed}) -- \
                 waiting for boot transfer"
            );
            // Without this, `test_cooldown_active()` never engages (it
            // only gates once `test_last_action_slot` has been set at
            // least once) -- a wallet that starts at 0 SOL would log this
            // completely unthrottled on *every* `evaluate()` call (fires
            // on every event) until funds arrive. Real, live-observed
            // incident: ~3,900 repeats of this exact line in 114 seconds,
            // correlated with the bot's stdout pipe to the host stalling
            // out ("stdio timeout").
            self.test_mark_action();
            return;
        }
        log_warn!(
            "testperpv1: [1/16] wrapping {} lamports SOL and swapping to USDC",
            Self::TEST_WRAP_SOL_LAMPORTS
        );
        self.test_wrap_and_swap_sol(owner, Self::TEST_WRAP_SOL_LAMPORTS);
        self.test_mark_action();
    }

    /// Idempotently creates the wSOL ATA, moves `lamports` of native SOL
    /// into it, issues `SyncNative` to make the wrapped balance visible
    /// to the token database, then routes it to USDC -- all queued onto
    /// `self.wallet` so `evaluate()`'s tail sends it as one transaction.
    fn test_wrap_and_swap_sol(&mut self, owner: AccountId, lamports: u64) {
        let mint_sol = self.configuration.mint_sol;
        let mint_usdc = self.configuration.mint_usdc;
        let Some(wsol_ata) = self.wallet.append_create_ata(owner, mint_sol) else {
            return;
        };
        let (Some(owner_pk), Some(wsol_ata_pk)) = (
            pubkey_from_account_id(&owner),
            pubkey_from_account_id(&wsol_ata),
        ) else {
            return;
        };
        self.wallet.require_signer(owner);
        self.wallet
            .append_ix(system_transfer(&owner_pk, &wsol_ata_pk, lamports), 5_000);
        // SyncNative: SPL Token discriminator 17, one writable account, no
        // signer, no further data -- hand-built because the pinned
        // spl-token crate (v9.0.0) only has the on-chain processor for
        // this, no client-side instruction builder (see [`TestPhase`]'s
        // doc comment).
        self.wallet.append_ix(
            Instruction {
                program_id: spl_token::ID,
                accounts: vec![AccountMeta::new(wsol_ata_pk, false)],
                data: vec![17],
            },
            5_000,
        );
        if let Err(e) = self.execute_spot_leg(mint_sol, mint_usdc, lamports) {
            log_error!("testperpv1: [1/16] swap to USDC failed: {e}");
        }
    }

    fn solend_usdc_reserve_id(&self) -> Option<AccountId> {
        let mint_usdc = self.configuration.mint_usdc;
        let dex = self.state.o_dex.as_ref()?;
        dex.solend().reserve_by_mint(mint_usdc).map(|(id, _)| id)
    }

    fn kamino_usdc_reserve_id(&self) -> Option<AccountId> {
        let mint_usdc = self.configuration.mint_usdc;
        let dex = self.state.o_dex.as_ref()?;
        dex.kamino().reserve_by_mint(mint_usdc).map(|(id, _)| id)
    }

    fn marginfi_usdc_bank_id(&self) -> Option<AccountId> {
        let mint_usdc = self.configuration.mint_usdc;
        let dex = self.state.o_dex.as_ref()?;
        dex.marginfi().reserve_by_mint(mint_usdc).map(|(id, _)| id)
    }

    fn test_bootstrap_solend(&mut self) {
        if self
            .state
            .o_solend_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            log_warn!("testperpv1: [2/16] solend obligation confirmed registered on-chain");
            self.test_advance(TestPhase::DepositSolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [2/16] bootstrapping solend obligation");
        self.bootstrap_solend_obligation();
        self.test_mark_action();
    }

    fn test_deposit_solend(&mut self) {
        let Some(usdc_reserve_id) = self.solend_usdc_reserve_id() else {
            // Cooldown check *before* logging, not just `test_mark_action`
            // after -- `evaluate()` fires many times per slot (once per
            // event, not just per commit), and `last_slot` (what the
            // cooldown compares against) only advances once per commit,
            // so without this check every one of those same-slot calls
            // logs again before the mark ever takes effect. Real,
            // live-observed: this exact line repeated 3x within the same
            // millisecond in a real run.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testperpv1: [3/16] solend USDC reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let has_deposit = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if has_deposit {
            log_warn!("testperpv1: [3/16] solend USDC deposit confirmed on-chain");
            self.test_advance(TestPhase::WithdrawSolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        if let Some(owner) = self.state.wallet() {
            let sol_lamports = self.wallet.balance_sol(&owner).unwrap_or(0);
            let usdc_raw: u64 = self
                .wallet
                .token_mut()
                .balance(&owner, &self.configuration.mint_usdc, true)
                .iter()
                .map(|(_, a)| *a)
                .sum();
            log_warn!(
                "testperpv1: [3/16] wallet balances before deposit attempt: {sol_lamports} SOL lamports, {usdc_raw} USDC (raw units)"
            );
        }
        log_warn!(
            "testperpv1: [3/16] depositing ${:.2} USDC into solend",
            Self::TEST_AMOUNT_USD
        );
        self.deploy_idle_usdc_solend(Self::TEST_AMOUNT_USD);
        self.test_mark_action();
    }

    /// See [`TestPhase`]'s doc comment for why this advances
    /// unconditionally after a cooldown rather than polling for
    /// confirmed absence.
    fn test_withdraw_solend(&mut self) {
        if self.state.test_last_action_slot.is_some() {
            if self.test_cooldown_active() {
                return;
            }
            log_warn!(
                "testperpv1: [4/16] solend withdraw cooldown elapsed -- proceeding (verify via real \
                 on-chain state, not this state machine)"
            );
            self.test_advance(TestPhase::BootstrapKamino);
            return;
        }
        log_warn!("testperpv1: [4/16] withdrawing USDC from solend");
        self.test_withdraw_usdc_solend();
        self.test_mark_action();
    }

    fn test_withdraw_usdc_solend(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.solend().reserve_by_mint(mint_usdc) else {
            return;
        };
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        let collateral_mint = usdc_reserve.collateral_mint;
        let Some(usdc_collateral_ata) = self.wallet.append_create_ata(owner, collateral_mint)
        else {
            return;
        };

        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!("testperpv1: solend withdraw refresh_reserve failed: {e}");
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!("testperpv1: solend withdraw refresh_obligation failed: {e}");
            return;
        }
        let deposit_reserves = self.solend_obligation_deposit_reserves();
        if let Err(e) = usdc_reserve.withdraw(
            usdc_reserve_id,
            obligation_id,
            solend::SOLEND_AMOUNT_MAX,
            owner,
            usdc_ata,
            usdc_collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("testperpv1: solend withdraw failed: {e}");
        }
    }

    fn test_bootstrap_kamino(&mut self) {
        if self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            log_warn!("testperpv1: [5/16] kamino obligation confirmed registered on-chain");
            self.test_advance(TestPhase::DepositKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [5/16] bootstrapping kamino obligation");
        self.bootstrap_kamino_obligation();
        self.test_mark_action();
    }

    fn test_deposit_kamino(&mut self) {
        let Some(usdc_reserve_id) = self.kamino_usdc_reserve_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testperpv1: [6/16] kamino USDC reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let has_deposit = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .is_some_and(|d| d.deposited_amount != 0);
        if has_deposit {
            log_warn!("testperpv1: [6/16] kamino USDC deposit confirmed on-chain");
            self.test_advance(TestPhase::WithdrawKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testperpv1: [6/16] depositing ${:.2} USDC into kamino",
            Self::TEST_AMOUNT_USD
        );
        self.deploy_idle_usdc_kamino(Self::TEST_AMOUNT_USD);
        self.test_mark_action();
    }

    /// See [`TestPhase`]'s doc comment for why this advances
    /// unconditionally after a cooldown rather than polling for
    /// confirmed absence.
    fn test_withdraw_kamino(&mut self) {
        if self.state.test_last_action_slot.is_some() {
            if self.test_cooldown_active() {
                return;
            }
            log_warn!(
                "testperpv1: [7/16] kamino withdraw cooldown elapsed -- proceeding (verify via real \
                 on-chain state, not this state machine)"
            );
            self.test_advance(TestPhase::BootstrapMarginfi);
            return;
        }
        log_warn!("testperpv1: [7/16] withdrawing USDC from kamino");
        self.test_withdraw_usdc_kamino();
        self.test_mark_action();
    }

    fn test_withdraw_usdc_kamino(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(obligation_id) = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };

        if let Err(e) = usdc_reserve.refresh_reserve(
            usdc_reserve_id,
            usdc_reserve.pyth_oracle,
            usdc_reserve.switchboard_price_oracle,
            usdc_reserve.switchboard_twap_oracle,
            usdc_reserve.scope_prices,
            self.wallet,
        ) {
            log_error!("testperpv1: kamino withdraw refresh_reserve failed: {e}");
            return;
        }
        let (deposit_reserves, borrow_reserves) = self.kamino_refresh_reserves();
        if let Err(e) = kamino::refresh_obligation(
            usdc_reserve.lending_market,
            obligation_id,
            &deposit_reserves,
            &borrow_reserves,
            self.wallet,
        ) {
            log_error!("testperpv1: kamino withdraw refresh_obligation failed: {e}");
            return;
        }
        if !self.ensure_kamino_farm_ready(
            usdc_reserve_id,
            usdc_reserve.lending_market,
            usdc_reserve.farm_collateral,
            0,
        ) {
            return;
        }
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let Some((usdc_reserve_id, usdc_reserve)) = dex.kamino().reserve_by_mint(mint_usdc) else {
            return;
        };
        if let Err(e) = usdc_reserve.withdraw(
            usdc_reserve_id,
            obligation_id,
            kamino::KAMINO_AMOUNT_MAX,
            owner,
            usdc_ata,
            self.wallet,
        ) {
            log_error!("testperpv1: kamino withdraw failed: {e}");
            return;
        }
        // Withdrawing the entire (only) deposit with no borrows
        // outstanding closes the real Kamino obligation account -- see
        // `KaminoPosition::mark_obligation_closing`'s doc comment for why
        // this can't be detected reactively via `on_account` instead.
        if let Some(pos) = self.state.o_kamino_position.as_mut() {
            pos.mark_obligation_closing();
        }
    }

    fn test_bootstrap_marginfi(&mut self) {
        if self
            .state
            .o_marginfi_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            log_warn!("testperpv1: [8/16] marginfi account confirmed registered on-chain");
            self.test_advance(TestPhase::DepositMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [8/16] bootstrapping marginfi account");
        self.bootstrap_marginfi_account();
        self.test_mark_action();
    }

    fn test_deposit_marginfi(&mut self) {
        let Some(usdc_bank_id) = self.marginfi_usdc_bank_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testperpv1: [9/16] marginfi USDC bank not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let has_deposit = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.deposit_for(usdc_bank_id))
            .is_some();
        if has_deposit {
            log_warn!("testperpv1: [9/16] marginfi USDC deposit confirmed on-chain");
            self.test_advance(TestPhase::WithdrawMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testperpv1: [9/16] depositing ${:.2} USDC into marginfi",
            Self::TEST_AMOUNT_USD
        );
        self.deploy_idle_usdc_marginfi(Self::TEST_AMOUNT_USD);
        self.test_mark_action();
    }

    /// See [`TestPhase`]'s doc comment for why this advances
    /// unconditionally after a cooldown rather than polling for confirmed
    /// absence -- kept consistent with Solend/Kamino's withdraw phases,
    /// even though marginfi's own `MarginfiAccount` isn't actually closed
    /// by a full withdrawal (see the same doc comment's note on this).
    fn test_withdraw_marginfi(&mut self) {
        if self.state.test_last_action_slot.is_some() {
            if self.test_cooldown_active() {
                return;
            }
            log_warn!(
                "testperpv1: [10/16] marginfi withdraw cooldown elapsed -- proceeding (verify via real \
                 on-chain state, not this state machine)"
            );
            self.test_advance(TestPhase::BorrowSolend);
            return;
        }
        log_warn!("testperpv1: [10/16] withdrawing USDC from marginfi");
        self.test_withdraw_usdc_marginfi();
        self.test_mark_action();
    }

    fn test_withdraw_usdc_marginfi(&mut self) {
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(marginfi_account) = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.account_id())
        else {
            return;
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return;
        };
        let mint_usdc = self.configuration.mint_usdc;
        let Some((usdc_bank_id, _)) = dex.marginfi().reserve_by_mint(mint_usdc) else {
            return;
        };
        let Some(usdc_ata) = self.wallet.derive_ata(owner, mint_usdc) else {
            return;
        };
        let group = account_id_from_pubkey(&marginfi::MARGINFI_MAIN_GROUP);
        let other_active_banks = self
            .state
            .o_marginfi_position
            .as_ref()
            .map(|s| s.other_active_banks(usdc_bank_id))
            .unwrap_or_default();

        if let Err(e) = dex.marginfi().withdraw(
            usdc_bank_id,
            group,
            marginfi_account,
            owner,
            usdc_ata,
            0, // ignored -- withdraw_all = true
            true,
            &other_active_banks,
            self.wallet,
        ) {
            log_error!("testperpv1: marginfi withdraw failed: {e}");
        }
    }

    fn solend_sol_reserve_id(&self) -> Option<AccountId> {
        let mint_sol = self.configuration.mint_sol;
        let dex = self.state.o_dex.as_ref()?;
        dex.solend().reserve_by_mint(mint_sol).map(|(id, _)| id)
    }

    fn kamino_sol_reserve_id(&self) -> Option<AccountId> {
        let mint_sol = self.configuration.mint_sol;
        let dex = self.state.o_dex.as_ref()?;
        dex.kamino().reserve_by_mint(mint_sol).map(|(id, _)| id)
    }

    fn marginfi_sol_bank_id(&self) -> Option<AccountId> {
        let mint_sol = self.configuration.mint_sol;
        let dex = self.state.o_dex.as_ref()?;
        dex.marginfi().reserve_by_mint(mint_sol).map(|(id, _)| id)
    }

    /// [11/16] Opens a real SOL borrow-hedge position on Solend --
    /// reuses [`Self::open_solend_borrow_leg`] directly (not
    /// `open_borrow_hedge_leg`, which would also place a Phoenix order --
    /// this mode never touches Phoenix, see this module's doc comment).
    /// That function's own two-stage design (deposit USDC collateral
    /// first if missing, then borrow) means calling it repeatedly here is
    /// enough -- no separate collateral-deposit phase needed. Retries
    /// until a real borrow is confirmed on-chain, same shape as the
    /// deposit phases above (not withdraw's fire-once-then-advance
    /// shape -- see [`TestPhase::BorrowSolend`]'s doc comment for why).
    fn test_borrow_solend(&mut self) {
        let Some(reserve_id) = self.solend_sol_reserve_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testperpv1: [11/16] solend SOL reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let has_borrow = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some_and(|b| b.borrowed_amount != 0);
        if has_borrow {
            log_warn!("testperpv1: [11/16] solend SOL borrow confirmed on-chain");
            self.test_advance(TestPhase::RepaySolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testperpv1: [11/16] opening ${:.2} SOL borrow-hedge on solend",
            Self::TEST_AMOUNT_USD
        );
        self.open_solend_borrow_leg("SOL", Self::TEST_AMOUNT_USD);
        self.test_mark_action();
    }

    /// [12/16] Buys back and repays the real Solend SOL borrow via
    /// [`Self::close_solend_borrow_leg`] -- retries until the liability
    /// is confirmed cleared on-chain (repaying doesn't close any account,
    /// unlike a full withdrawal, so polling for confirmed absence is
    /// reliable here -- see [`TestPhase::BorrowSolend`]'s doc comment).
    fn test_repay_solend(&mut self) {
        let Some(reserve_id) = self.solend_sol_reserve_id() else {
            return;
        };
        let has_borrow = self
            .state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some_and(|b| b.borrowed_amount != 0);
        if !has_borrow {
            log_warn!("testperpv1: [12/16] solend SOL repay confirmed on-chain");
            self.test_advance(TestPhase::BorrowKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [12/16] repaying SOL borrow on solend");
        self.close_solend_borrow_leg("SOL");
        self.test_mark_action();
    }

    /// [13/16] Kamino counterpart of [`Self::test_borrow_solend`], via
    /// [`Self::open_kamino_borrow_leg`] (which also handles Kamino's Farms
    /// bootstrap gate internally, same as the deposit phase).
    fn test_borrow_kamino(&mut self) {
        let Some(reserve_id) = self.kamino_sol_reserve_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testperpv1: [13/16] kamino SOL reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let has_borrow = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some_and(|b| b.borrowed_amount != 0);
        if has_borrow {
            log_warn!("testperpv1: [13/16] kamino SOL borrow confirmed on-chain");
            self.test_advance(TestPhase::RepayKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testperpv1: [13/16] opening ${:.2} SOL borrow-hedge on kamino",
            Self::TEST_AMOUNT_USD
        );
        self.open_kamino_borrow_leg("SOL", Self::TEST_AMOUNT_USD);
        self.test_mark_action();
    }

    /// [14/16] Kamino counterpart of [`Self::test_repay_solend`], via
    /// [`Self::close_kamino_borrow_leg`].
    fn test_repay_kamino(&mut self) {
        let Some(reserve_id) = self.kamino_sol_reserve_id() else {
            return;
        };
        let has_borrow = self
            .state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.borrow_for(reserve_id))
            .is_some_and(|b| b.borrowed_amount != 0);
        if !has_borrow {
            log_warn!("testperpv1: [14/16] kamino SOL repay confirmed on-chain");
            self.test_advance(TestPhase::BorrowMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [14/16] repaying SOL borrow on kamino");
        self.close_kamino_borrow_leg("SOL");
        self.test_mark_action();
    }

    /// [15/16] marginfi counterpart of [`Self::test_borrow_solend`], via
    /// [`Self::open_marginfi_borrow_leg`].
    ///
    /// TEMPORARILY SKIPPED (test scope only -- `open_marginfi_borrow_leg`
    /// itself, still exercised by this mode's own production basis-trade
    /// path, keeps its full real staleness gate). The gate is verified
    /// correct: marginfi's
    /// real per-bank `oracle_max_age` (`70` seconds for this bank) and the
    /// Switchboard feed's own `last_update_timestamp` were both
    /// live-verified against real source and cross-checked independently
    /// against direct on-chain RPC reads (not just this bot's own
    /// subscription) over a 45+ minute stretch -- the feed genuinely
    /// wasn't cranked by any external consumer in that entire window, so
    /// there was nothing left to detect. This bot has no way to crank
    /// Switchboard itself from inside this sandboxed WASM guest, so this
    /// phase's success now depends entirely on an external, irregular
    /// cranking cadence outside this bot's control -- skip straight to
    /// [`TestPhase::RepayMarginfi`] (which already treats "no borrow" as
    /// a valid, complete state) so the rest of the test can still
    /// confirm end-to-end, per explicit direction.
    fn test_borrow_marginfi(&mut self) {
        log_warn!("testperpv1: [15/16] skipping marginfi SOL borrow-hedge (depends on an external Switchboard crank, see doc comment above)");
        self.test_advance(TestPhase::RepayMarginfi);
    }

    /// [16/16] marginfi counterpart of [`Self::test_repay_solend`], via
    /// [`Self::close_marginfi_borrow_leg`] -- the last phase; advances to
    /// [`TestPhase::Done`] once confirmed.
    fn test_repay_marginfi(&mut self) {
        let Some(bank_id) = self.marginfi_sol_bank_id() else {
            return;
        };
        let has_borrow = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.borrow_for(bank_id))
            .is_some();
        if !has_borrow {
            log_warn!(
                "testperpv1: [16/16] marginfi SOL repay confirmed on-chain -- test complete (verify via real \
                 on-chain state, not this state machine)"
            );
            self.test_advance(TestPhase::Done);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testperpv1: [16/16] repaying SOL borrow on marginfi");
        self.close_marginfi_borrow_leg("SOL");
        self.test_mark_action();
    }

    /// Build and send a single spot swap leg (`mint_in` -> `mint_out`,
    /// `amount_in` raw units) through `TradeRouter`/`DexState`, onto the
    /// real `self.wallet` (not a scratch dry-run, unlike
    /// `arbv1::build_execution_plan`) -- built instructions are picked
    /// up by `evaluate()`'s `assemble()`/send loop above.
    ///
    /// This is a general-purpose hook, not a strategy of its own -- it
    /// was built so a future decision could be wired in without first
    /// re-deriving this plumbing, and `rebalance_portfolio` below is
    /// now that decision (portfolio-target rebalancing); it stays
    /// available for others too (collateral top-up, a cash-and-carry
    /// spot leg against a Phoenix/Velocity funding edge, etc.). Same
    /// untestable-outside-the-WASM-guest-runtime boundary
    /// as `build_execution_plan` (see that method's doc comment): every
    /// `Wallet`/`account_id_from_pubkey`-touching call here transitively
    /// hits a WIT host import, so this has no native `cargo test`
    /// coverage by design, matching the rest of this codebase's
    /// `Wallet`-touching code.
    pub(crate) fn execute_spot_leg(
        &mut self,
        mint_in: AccountId,
        mint_out: AccountId,
        amount_in: u64,
    ) -> Result<(), String> {
        let Some(owner) = self.state.wallet() else {
            return Err("no wallet keypair yet".to_string());
        };
        let Some(dex) = self.state.o_dex.as_ref() else {
            return Err("dex state not ready".to_string());
        };
        const MAX_HOPS: usize = 4;
        self.state
            .spot_router
            .set_current_slot(self.state.last_slot);
        let Some(route) = self
            .state
            .spot_router
            .route_slippage_aware(mint_in, mint_out, amount_in, MAX_HOPS)
        else {
            log_error!(
                "testperpv1: route diagnostics for {mint_in} -> {mint_out}:\n{}",
                self.state
                    .spot_router
                    .route_diagnostics(mint_in, mint_out, amount_in, MAX_HOPS)
            );
            return Err(format!(
                "no route found for {mint_in} -> {mint_out} amount_in={amount_in}"
            ));
        };
        let route = match planner::reverify_route_with_exact_quotes(
            &route,
            amount_in,
            &self.state.spot_router,
            dex,
        ) {
            Ok(route) => route,
            Err(failure) => {
                if failure.coolable {
                    self.state
                        .spot_router
                        .mark_pool_cooldown(failure.pool_id, planner::POOL_COOLDOWN_SLOTS);
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
            "testperpv1: spot leg @ slot {}: {} hop{} {} -> {} amount_in={}",
            self.state.last_slot,
            route.hops.len(),
            if route.hops.len() == 1 { "" } else { "s" },
            mint_in,
            mint_out,
            amount_in,
        );
        for (i, hop) in route.hops.iter().enumerate() {
            // `append_create_ata` (not `derive_ata`) -- a hop's
            // intermediate mint (unlike the route's overall input/output,
            // which are almost always mints the wallet already holds) may
            // never have been touched by this wallet before, so its ATA
            // may not exist yet. `CreateIdempotent` is a safe no-op when
            // it already does. Live-confirmed this session: a
            // marginfi borrow-hedge sell routed through an unfamiliar
            // intermediate mint and reverted on-chain with
            // `AccountNotInitialized` because only the address was
            // derived, never actually created.
            // Glue this hop's ATA-creation + swap instructions together --
            // see `Wallet::begin_atomic_group`'s doc comment for the real,
            // live-confirmed `AccountNotInitialized` bug this prevents
            // (assemble()'s size-based splitter previously could, and
            // did, send a hop's swap in a different, unordered
            // transaction than its own destination-ATA-creation
            // instruction).
            self.wallet.begin_atomic_group();
            let (Some(source_ata), Some(dest_ata)) = (
                self.wallet.append_create_ata(owner, hop.input_mint),
                self.wallet.append_create_ata(owner, hop.output_mint),
            ) else {
                self.wallet.end_atomic_group();
                return Err(format!(
                    "hop {i}: FAILED to derive token account(s) for owner={owner}"
                ));
            };
            let hop_result = dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet);
            self.wallet.end_atomic_group();
            match hop_result {
                Ok(()) => {
                    log_warn!(
                        "  hop {i}: OK dex={:?} pool={} {} -> {} amount_in={} amount_out={}",
                        hop.dex,
                        hop.pool_id,
                        hop.input_mint,
                        hop.output_mint,
                        hop.amount_in,
                        hop.amount_out,
                    );
                }
                Err(e) => {
                    return Err(format!(
                        "hop {i}: FAILED dex={:?} pool={} {} -> {}: {}",
                        hop.dex, hop.pool_id, hop.input_mint, hop.output_mint, e,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Current USDC balance, valued at $1 (no price oracle -- USDC is
    /// assumed pegged, same convention `build.rs` and every other USDC
    /// valuation in this file already use). `0.0` if there's no wallet
    /// keypair yet, matching the rest of this file's precondition
    /// style. Shared by `rebalance_portfolio` (values the whole
    /// portfolio) and `log_funding_graph_cycles`'s capital-feasibility
    /// selection (how much spare USDC is available to margin a funding
    /// cycle) -- same live `TokenDatabase` lookup, not two data
    /// sources that could disagree.
    fn current_usdc_value(&mut self) -> f64 {
        let Some(owner) = self.state.wallet() else {
            return 0.0;
        };
        const USDC_DECIMALS: i32 = 6;
        let mint_usdc = self.configuration.mint_usdc;
        let usdc_balance_raw: u64 = self
            .wallet
            .token_mut()
            .balance(&owner, &mint_usdc, true)
            .iter()
            .map(|(_, a)| *a)
            .sum();
        usdc_balance_raw as f64 / 10f64.powi(USDC_DECIMALS)
    }

    /// Full-portfolio rebalance toward `target_allocation_pct`: values
    /// current holdings (each tracked asset + implicit USDC) in USD via
    /// `spot_router.route_slippage_aware` (the same per-asset quoting
    /// `log_spot_price_probe` above already uses -- `TradeRouter` has
    /// no bulk "value my whole wallet" API), computes each asset's
    /// delta against `allocation_pct * total_value`
    /// (`plan_rebalance_legs`), then executes every sell before every
    /// buy via `execute_spot_leg` (sells free the USDC buys need to
    /// spend). Triggered by every `CustomMessageInbound::
    /// TargetAllocation` -- a *full* portfolio rebalance, not just the
    /// one symbol that changed, since the implicit USDC remainder is
    /// defined as `1.0` minus every tracked asset's `allocation_pct`:
    /// changing one asset's target always implicitly changes every
    /// other asset's effective target too. Buy sizing reserves
    /// `FUNDING_CYCLE_MIN_MARGIN_USD` off the top before spending
    /// anything on directional purchases -- see `plan_rebalance_legs`'s
    /// doc comment for why.
    pub(crate) fn rebalance_portfolio(&mut self) {
        let Some(owner) = self.state.wallet() else {
            log_warn!("testperpv1: rebalance skipped -- no wallet keypair yet");
            return;
        };
        const MAX_HOPS: usize = 4;
        const USDC_DECIMALS: i32 = 6;
        let mint_usdc = self.configuration.mint_usdc;
        let usdc_value = self.current_usdc_value();

        self.state
            .spot_router
            .set_current_slot(self.state.last_slot);

        // Snapshot (symbol, account_id, target_pct) first -- avoids
        // holding an immutable borrow of self.state across the mutable
        // self.wallet/self.state.spot_router calls in the loop below.
        let entries: Vec<(String, AccountId, f64)> = self
            .state
            .target_allocation_pct
            .iter()
            .filter_map(|(symbol, entry)| {
                entry
                    .account_id
                    .map(|id| (symbol.clone(), id, entry.allocation_pct))
            })
            .collect();

        let mut holdings: Vec<(String, AssetHolding)> = Vec::new();
        let mut total_value = usdc_value;
        for (symbol, account_id, target_pct) in entries {
            // No SOL special-case: `Wallet::balance_sol` (native
            // lamports) is deliberately unused for trading-relevant
            // balance anywhere in this codebase -- pools only ever
            // trade *wrapped* SOL as an SPL mint, so `TokenDatabase::
            // balance` against the wSOL mint (which `account_id`
            // already resolves to) is the correct, uniform query for
            // every tracked asset including SOL. Matches
            // `planner::find_opportunity`'s own explicit doc comment on
            // this exact question. Un-wrapped native SOL sitting in the
            // wallet is real value this bot can't act on without a
            // wrap step this codebase doesn't have -- correctly
            // excluded, not a bug.
            let balance_raw: u64 = self
                .wallet
                .token_mut()
                .balance(&owner, &account_id, true)
                .iter()
                .map(|(_, a)| *a)
                .sum();
            // A zero balance needs no quote -- it's worth $0 regardless
            // of whether a route exists. A *nonzero* balance with no
            // route found is excluded from both the total and this
            // pass's trading (logged below), not silently treated as
            // $0 -- that would wrongly inflate its "needs buying"
            // signal.
            let value_usd = if balance_raw == 0 {
                0.0
            } else {
                match self.state.spot_router.route_slippage_aware(
                    account_id,
                    mint_usdc,
                    balance_raw,
                    MAX_HOPS,
                ) {
                    Some(route) => route.amount_out() as f64 / 10f64.powi(USDC_DECIMALS),
                    None => {
                        log_error!(
                            "testperpv1: rebalance: no route to value {} ({}), skipping this pass",
                            symbol,
                            account_id,
                        );
                        continue;
                    }
                }
            };
            let decimals = resolve_symbol_decimals(&symbol).unwrap_or(0);
            log_warn!(
                "testperpv1: rebalance holding {}: balance={:.9} (raw {}) value=${:.2} target_pct={:.4}",
                symbol,
                balance_raw as f64 / 10f64.powi(decimals as i32),
                balance_raw,
                value_usd,
                target_pct,
            );
            total_value += value_usd;
            holdings.push((
                symbol,
                AssetHolding {
                    account_id,
                    balance_raw,
                    value_usd,
                    target_pct,
                },
            ));
        }

        log_warn!(
            "testperpv1: rebalance @ slot {}: total portfolio value=${:.2} (usdc=${:.2}, {} priced asset{})",
            self.state.last_slot,
            total_value,
            usdc_value,
            holdings.len(),
            if holdings.len() == 1 { "" } else { "s" },
        );

        let (sells, buys, scale) = plan_rebalance_legs(
            &holdings,
            total_value,
            usdc_value,
            FUNDING_CYCLE_MIN_MARGIN_USD,
        );
        if scale < 1.0 {
            log_warn!(
                "testperpv1: rebalance: desired buys exceed available capital -- scaled to {:.4} \
                 (reserving ${:.2} for funding-arb margin)",
                scale,
                FUNDING_CYCLE_MIN_MARGIN_USD,
            );
        }
        for (symbol, account_id, amount_in) in sells {
            log_warn!(
                "testperpv1: rebalance SELL {} amount_in={}",
                symbol,
                amount_in
            );
            if let Err(e) = self.execute_spot_leg(account_id, mint_usdc, amount_in) {
                log_error!("testperpv1: rebalance SELL {} failed: {}", symbol, e);
            }
        }
        for (symbol, account_id, amount_in) in buys {
            log_warn!(
                "testperpv1: rebalance BUY {} amount_in={}",
                symbol,
                amount_in
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, account_id, amount_in) {
                log_error!("testperpv1: rebalance BUY {} failed: {}", symbol, e);
            }
        }
    }
}

/// Minimum `|delta_usd|` a rebalance leg must clear to be worth trading
/// -- a deliberately simple, tunable placeholder to avoid dust trades
/// from float noise or a few cents of drift, not derived from any
/// cost-of-trading analysis.
const REBALANCE_DUST_THRESHOLD_USD: f64 = 1.0;

/// One asset's valuation snapshot going into `plan_rebalance_legs` --
/// deliberately holds only plain data (no `Wallet`/`TradeRouter`
/// references), so the actual buy/sell-sizing math is natively
/// testable even though *gathering* these values (real wallet
/// balances, real route quotes) needs the live WASM guest runtime.
#[derive(Debug, Clone, Copy)]
struct AssetHolding {
    account_id: AccountId,
    balance_raw: u64,
    value_usd: f64,
    target_pct: f64,
}

/// Pure planning step (no host-import dependency, natively testable):
/// for each `(symbol, holding)` compares `holding.value_usd` to
/// `holding.target_pct * total_value`, skips deltas under
/// `REBALANCE_DUST_THRESHOLD_USD`, and returns sells and buys as two
/// separately ordered lists (both sorted by symbol for determinism --
/// `holdings` itself has no defined order). Sell amounts are sized as a
/// proportional fraction of the current raw balance, reusing this
/// pass's own valuation quote as an implied per-unit price -- an
/// approximation, not an exact-price computation;
/// `execute_spot_leg`'s own `reverify_route_with_exact_quotes` still
/// re-checks the real price before actually sending anything, so this
/// only affects the requested trade *size*, not the executed price.
///
/// Two passes: sells are sized and returned immediately (selling only
/// ever frees USDC, so nothing constrains it), while buys are collected
/// as desired USD amounts first, then capped in a second pass against
/// `usdc_value + sell_proceeds - capital_reserve_usd` -- what's
/// actually going to be on hand after this pass's sells settle, minus a
/// standing floor (`capital_reserve_usd`, e.g.
/// `FUNDING_CYCLE_MIN_MARGIN_USD` -- shared with
/// `select_capital_feasible_cycles` so the two systems don't both treat
/// the same dollar as theirs to spend) that's reserved off the top
/// regardless of which system runs first. If total desired buys exceed
/// what's available, every buy is scaled down by the same ratio --
/// proportional, not first-come-first-served, so capital scarcity
/// doesn't arbitrarily favor whichever symbol sorts first. The third
/// return value is that scale factor (`1.0` when every desired buy fit
/// without scaling), so a caller can log when a capital shortfall
/// actually bit.
fn plan_rebalance_legs(
    holdings: &[(String, AssetHolding)],
    total_value: f64,
    usdc_value: f64,
    capital_reserve_usd: f64,
) -> (
    Vec<(String, AccountId, u64)>,
    Vec<(String, AccountId, u64)>,
    f64,
) {
    const USDC_DECIMALS: i32 = 6;
    let mut sorted: Vec<&(String, AssetHolding)> = holdings.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut sells = Vec::new();
    let mut desired_buys: Vec<(String, AccountId, f64)> = Vec::new();
    let mut sell_proceeds_usd = 0.0;
    for (symbol, holding) in sorted {
        let target_usd = holding.target_pct * total_value;
        let delta_usd = target_usd - holding.value_usd;
        if delta_usd.abs() < REBALANCE_DUST_THRESHOLD_USD {
            continue;
        }
        if delta_usd > 0.0 {
            desired_buys.push((symbol.clone(), holding.account_id, delta_usd));
        } else if holding.value_usd > 0.0 {
            let sell_fraction = (delta_usd.abs() / holding.value_usd).min(1.0);
            let amount_in = (holding.balance_raw as f64 * sell_fraction).round() as u64;
            if amount_in > 0 {
                sells.push((symbol.clone(), holding.account_id, amount_in));
                sell_proceeds_usd += delta_usd.abs();
            }
        }
    }

    let available_for_buys = (usdc_value + sell_proceeds_usd - capital_reserve_usd).max(0.0);
    let total_desired_usd: f64 = desired_buys.iter().map(|(_, _, d)| *d).sum();
    let scale = if total_desired_usd > available_for_buys && total_desired_usd > 0.0 {
        available_for_buys / total_desired_usd
    } else {
        1.0
    };
    let mut buys = Vec::new();
    for (symbol, account_id, delta_usd) in desired_buys {
        let amount_in = ((delta_usd * scale) * 10f64.powi(USDC_DECIMALS)).round() as u64;
        if amount_in > 0 {
            buys.push((symbol, account_id, amount_in));
        }
    }
    (sells, buys, scale)
}

/// Assumed capital needed to margin one funding-arb cycle (both legs
/// combined) -- a deliberately simple, tunable placeholder, not derived
/// from real per-venue margin requirements (Phoenix/Velocity margin
/// ratios aren't modeled anywhere in this bot yet). Same
/// "flag the simplification, don't hide it" discipline as
/// `REBALANCE_DUST_THRESHOLD_USD`.
/// Assumed capital needed to margin one basis-trade cycle (both legs
/// combined) -- a deliberately simple, tunable placeholder, not derived
/// from real per-venue margin requirements. Same "flag the
/// simplification, don't hide it" discipline as
/// `REBALANCE_DUST_THRESHOLD_USD`.
const FUNDING_CYCLE_MIN_MARGIN_USD: f64 = 10.0;

/// Which side of the basis trade is profitable for a symbol right now.
/// See [`decide_basis_trade`]'s doc comment for the real economics of
/// each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BasisDirection {
    /// Funding positive (longs pay shorts on Phoenix): short the perp,
    /// hedge with a lending-protocol deposit of the underlying (no
    /// borrowing).
    DepositHedge,
    /// Funding negative (shorts pay longs): long the perp, hedge with a
    /// lending-protocol borrow of the underlying, sold for USDC
    /// (synthetic short).
    BorrowHedge,
}

/// Which lending protocol backs a basis-trade hedge leg -- Solend and
/// Kamino are the only two this bot uses (marginfi deferred, see this
/// module's doc comment: its cached prefetch data looked stale and a live
/// re-fetch needs infrastructure this environment doesn't have configured
/// yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LendingProtocol {
    Solend,
    Kamino,
}

/// Pure (no host-import dependency, natively testable): decides which
/// side of the Phoenix-perp-funding-vs-lending-rate basis trade is
/// profitable for a symbol, given this epoch's Phoenix funding rate and
/// the cheapest real borrow APY available across every lending protocol
/// with a reserve for this symbol (both **percent** units --
/// `SolendReserve`/`KaminoReserve::current_borrow_apy()` each return a
/// 0.0-1.0 fraction, multiply by 100 before calling this -- see
/// [`StateHelper::best_borrow_apy`]).
///
/// - `phoenix_funding_pct > 0.0` (longs pay shorts): short the perp,
///   deposit the underlying on whichever protocol pays the best supply
///   APY to stay delta-neutral -- this *adds* the deposit's yield on top
///   of the captured funding, so it's worth doing whenever funding is
///   positive at all, no threshold against any lending rate needed
///   (depositing never costs anything, only ever earns).
/// - `phoenix_funding_pct < 0.0` (shorts pay longs): long the perp,
///   borrow the underlying and sell it for a synthetic short hedge --
///   only profitable if the funding collected exceeds the real interest
///   paid to borrow, i.e. `-phoenix_funding_pct > borrow_apy_pct`.
/// - Otherwise (funding is exactly zero, or negative but not enough to
///   clear the borrow cost): `None`, no capturable edge.
fn decide_basis_trade(phoenix_funding_pct: f64, borrow_apy_pct: f64) -> Option<BasisDirection> {
    if phoenix_funding_pct > 0.0 {
        Some(BasisDirection::DepositHedge)
    } else if phoenix_funding_pct < 0.0 && -phoenix_funding_pct > borrow_apy_pct {
        Some(BasisDirection::BorrowHedge)
    } else {
        None
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg
                    .push_back(MessageSend::Pong(std::time::SystemTime::now()));
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
                    log_warn!("testperpv1: got wallet keypair {} {}", pubkey, account_id);
                    self.wallet
                        .append_key(rc_keypair.clone(), self.graph)
                        .unwrap();
                    self.wallet.set_payer(account_id);
                    self.configuration.set(&rc_keypair);
                    self.state.o_rc_keypair.replace(KeypairExtra {
                        rc_keypair,
                        account_id,
                    });
                    // First point in this file's message flow confirmed
                    // to be inside the live WASM guest (mirrors
                    // Configuration::set's own mint_sol/mint_usdc
                    // resolution immediately above) -- safe to resolve
                    // the build-time-default target allocation entries'
                    // mints now.
                    self.state.resolve_target_allocation_mints();
                    // Derive+subscribe each venue's own trader/position
                    // account PDA -- batched into a single bulk_subscribe
                    // call instead of four separate set_authority calls
                    // (five subscribe round-trips, Kamino needs two).
                    // Real, live-observed incident: those five
                    // one-at-a-time calls accounted for ~26 seconds of
                    // stall in one run (18.4s + 7.3s), traced via
                    // CommitHook::start's own timing diagnostics.
                    let phoenix_reqs = self
                        .state
                        .o_phoenix
                        .as_ref()
                        .map(|p| p.authority_subscribe_requests(pubkey))
                        .unwrap_or_default();
                    let solend_reqs = self
                        .state
                        .o_solend_position
                        .as_ref()
                        .map(|s| s.authority_subscribe_requests(pubkey, 0))
                        .unwrap_or_default();
                    let kamino_reqs = self
                        .state
                        .o_kamino_position
                        .as_ref()
                        .map(|k| k.authority_subscribe_requests(pubkey, 0))
                        .unwrap_or_default();
                    let marginfi_reqs = self
                        .state
                        .o_marginfi_position
                        .as_ref()
                        .map(|m| m.authority_subscribe_requests(pubkey))
                        .unwrap_or_default();
                    // This wallet's own SPL token accounts (USDC, wSOL,
                    // every currently-tracked target-allocation mint) --
                    // batched into the same call for the same reason as
                    // the four venues above. Real, live-observed
                    // motivation: before this, nothing in this codebase
                    // ever subscribed to a wallet's own ATAs at all (only
                    // its native SOL account, via `append_key`) --
                    // confirmed live via `solana confirm -v`: a real swap
                    // succeeded on-chain (22+ USDC landed in the wallet's
                    // real ATA) while `current_usdc_value()` stayed at 0
                    // the entire time, since no `on_token`/low-latency
                    // update for that ATA was ever received. See
                    // `Wallet::l_ata_sub`'s own doc comment.
                    let mut hs_ata_mints: std::collections::HashSet<AccountId> =
                        std::collections::HashSet::new();
                    hs_ata_mints.insert(self.configuration.mint_usdc);
                    hs_ata_mints.insert(self.configuration.mint_sol);
                    for entry in self.state.target_allocation_pct.values() {
                        if let Some(mint) = entry.account_id {
                            hs_ata_mints.insert(mint);
                        }
                    }
                    let ata_reqs: Vec<_> = hs_ata_mints
                        .into_iter()
                        .filter_map(|mint| self.wallet.ata_subscribe_request(account_id, mint))
                        .collect();
                    // This wallet's own durable-nonce account (see
                    // `Wallet::send_bundler_pair`'s doc comment) --
                    // batched into the same subscribe_now call as
                    // everything else above, not a separate round-trip.
                    let nonce_reqs: Vec<_> = self
                        .wallet
                        .nonce_subscribe_request(account_id)
                        .into_iter()
                        .collect();

                    let (phoenix_len, solend_len, kamino_len, marginfi_len, ata_len, nonce_len) = (
                        phoenix_reqs.len(),
                        solend_reqs.len(),
                        kamino_reqs.len(),
                        marginfi_reqs.len(),
                        ata_reqs.len(),
                        nonce_reqs.len(),
                    );
                    let mut all_requests = Vec::with_capacity(
                        phoenix_len + solend_len + kamino_len + marginfi_len + ata_len + nonce_len,
                    );
                    all_requests.extend(phoenix_reqs);
                    all_requests.extend(solend_reqs);
                    all_requests.extend(kamino_reqs);
                    all_requests.extend(marginfi_reqs);
                    all_requests.extend(ata_reqs);
                    all_requests.extend(nonce_reqs);

                    match SubscriptionQueue::subscribe_now(self.graph, all_requests) {
                        Ok(subs) => {
                            let mut it = subs.into_iter();
                            if let Some(phoenix) = self.state.o_phoenix.as_mut() {
                                let take: Vec<_> = (&mut it).take(phoenix_len).collect();
                                phoenix.apply_authority(pubkey, take);
                            }
                            if let Some(solend_position) = self.state.o_solend_position.as_mut() {
                                let take: Vec<_> = (&mut it).take(solend_len).collect();
                                solend_position.apply_authority(pubkey, 0, take);
                            }
                            if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
                                let take: Vec<_> = (&mut it).take(kamino_len).collect();
                                kamino_position.apply_authority(pubkey, 0, take);
                            }
                            if let Some(marginfi_position) = self.state.o_marginfi_position.as_mut()
                            {
                                let take: Vec<_> = (&mut it).take(marginfi_len).collect();
                                marginfi_position.apply_authority(pubkey, take);
                            }
                            let ata_subs: Vec<_> = (&mut it).take(ata_len).collect();
                            self.wallet.keep_ata_subscriptions(ata_subs);
                            if let Some(sub) = (&mut it).take(nonce_len).next() {
                                self.wallet.keep_nonce_subscription(sub);
                            }
                        }
                        Err(e) => {
                            log_error!("testperpv1: failed to batch-subscribe wallet authority accounts: {e}");
                        }
                    }
                }
                CustomMessageInbound::TargetAllocation(symbol, allocation_pct) => {
                    let account_id = resolve_symbol_mint(&symbol);
                    if account_id.is_none() {
                        log_error!(
                            "testperpv1: target allocation symbol {} has no curated mint -- cannot resolve to AccountId",
                            symbol,
                        );
                    }
                    log_warn!(
                        "testperpv1: target allocation update: {}={} ({:?})",
                        symbol,
                        allocation_pct,
                        account_id
                    );
                    self.state.target_allocation_pct.insert(
                        symbol,
                        TargetAllocationEntry {
                            account_id,
                            allocation_pct,
                        },
                    );
                    self.rebalance_portfolio();
                }
                CustomMessageInbound::LstApy(symbol, staking_apy) => {
                    log_warn!(
                        "testperpv1: LST staking APY update: {}={:.3}%",
                        symbol,
                        staking_apy * 100.0,
                    );
                    self.state
                        .lst_staking_apy
                        .insert(symbol.clone(), staking_apy);
                    self.log_lst_loop_projection(&symbol, staking_apy);
                }
                CustomMessageInbound::TriggerTestAstralane => {
                    let Some(owner) = self.state.wallet() else {
                        log_warn!("testperpv1: TriggerTestAstralane ignored -- no wallet yet");
                        return;
                    };
                    match self.wallet.test_send_astralane_tip_batch(owner) {
                        Some(Ok(())) => {
                            log_warn!("testperpv1: TriggerTestAstralane -- tip + self-transfer sent via transactionprocessor::batch (bundler=astralane)");
                        }
                        Some(Err(e)) => {
                            log_error!("testperpv1: TriggerTestAstralane -- batch failed: {e:?}");
                        }
                        None => {
                            log_warn!(
                                "testperpv1: TriggerTestAstralane ignored -- wallet key not loaded yet, or something else is already queued this tick"
                            );
                        }
                    }
                }
                CustomMessageInbound::CommonBundlerTipUpdate(update) => {
                    self.wallet.apply_bundler_tip_update(self.graph, update);
                }
            },
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

/// Target for both slot-timing diagnostics below: `finish()`'s
/// start-to-finish span, and `start()`'s gap-since-previous-start.
/// Originally set to 200ms; corrected to 400ms after live validator-side
/// data (root-slot `total=` timings gathered this session) showed real
/// root-slot intervals cluster around 230-250ms median with normal
/// spikes into the 400s -- 200ms wasn't an achievable target given
/// Solana's own real block cadence, not a guest-side problem to chase.
const SLOT_TIMING_TARGET_MS: u128 = 400;

/// Cap on how many queued subscription requests `subscription_queue`
/// sends per slot (one bounded `bulk_subscribe` call in `finish()`) --
/// keeps each slot's own blocking subscribe cost small and predictable
/// instead of one giant call for the whole ~32,000-request startup
/// burst. Real, live-observed motivation: `subscribe`/`bulk_subscribe`
/// are blocking calls on the validator side.
const MAX_SUBSCRIBES_PER_SLOT: usize = 128;

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        assert!(self.o_commit_slot.replace(slot).is_none());
        self.state.last_slot = slot;
        let now = std::time::Instant::now();
        // Gap since the *previous* slot's start() -- covers evaluate()'s
        // own work, other event types, and idle time between commits,
        // not just this commit's own on_account/on_token/finish span
        // (see o_prev_commit_start_instant's doc comment). `None` on the
        // very first commit -- nothing to compare against yet.
        // Reset every call regardless of whether it logs below, so the
        // measurement window always means exactly "since the previous
        // start()" -- see the field's own doc comment.
        let ll_elapsed_ms =
            std::mem::take(&mut self.state.low_latency_elapsed_since_last_start).as_millis();
        let ll_count = std::mem::take(&mut self.state.low_latency_count_since_last_start);
        let ev_elapsed_ms =
            std::mem::take(&mut self.state.evaluate_elapsed_since_last_start).as_millis();
        let ev_count = std::mem::take(&mut self.state.evaluate_count_since_last_start);
        // Testing a new hypothesis: every death this session left its
        // last log line looking completely unremarkable -- no bookend
        // ever caught the freeze itself, even after every known blocking
        // call (subscribe/bulk_subscribe) was paced and bookended. A log
        // call's own internal buffer-overflow flush is a real host stdio
        // write that neither `log_warn!` nor any caller can bookend (the
        // flush happens *inside* the call trying to log something) --
        // see `stdio::take_flush_stats`'s doc comment.
        let (flush_elapsed, flush_count) = crate::stdio::take_flush_stats();
        let flush_elapsed_ms = flush_elapsed.as_millis();
        if let Some(prev) = self.state.o_prev_commit_start_instant.replace(now) {
            let gap_ms = now.duration_since(prev).as_millis();
            if gap_ms > SLOT_TIMING_TARGET_MS {
                log_warn!(
                    "testperpv1: {}ms since previous slot's start() (target: <{}ms) -- \
                     of that, {}ms was spent in low_latency() processing {} account/token \
                     updates, {}ms was spent across {} evaluate() calls, and {}ms was spent \
                     across {} stdio flush() calls, in the window",
                    gap_ms,
                    SLOT_TIMING_TARGET_MS,
                    ll_elapsed_ms,
                    ll_count,
                    ev_elapsed_ms,
                    ev_count,
                    flush_elapsed_ms,
                    flush_count,
                );
            }
        }
        self.state.o_commit_start_instant = Some(now);
        if slot % 100 == 0 {
            let phoenix_ready = self
                .state
                .o_phoenix
                .as_ref()
                .map(|p| p.ready_count())
                .unwrap_or(0);
            let solend_registered = self
                .state
                .o_solend_position
                .as_ref()
                .is_some_and(|s| s.registered());
            let kamino_registered = self
                .state
                .o_kamino_position
                .as_ref()
                .is_some_and(|s| s.registered());
            let marginfi_registered = self
                .state
                .o_marginfi_position
                .as_ref()
                .is_some_and(|s| s.registered());
            log_warn!(
                "testperpv1 stats @ slot {slot}: phoenix_ready={} solend_registered={} kamino_registered={} marginfi_registered={} pending_epoch={:?}",
                phoenix_ready,
                solend_registered,
                kamino_registered,
                marginfi_registered,
                self.state.pending_epoch_ts,
            );
            self.log_spot_price_probe();
            self.log_spfa_smoke_test();
        }
    }

    fn on_account(&mut self, header: &Header, body: &[u8]) {
        // Real, live-confirmed gap: `Wallet::on_account` (the child
        // wallet's own SOL-balance tracking, used by `balance_sol()`,
        // which `test_swap_to_usdc` depends on) was never called from
        // anywhere in this codebase -- not here, not in `low_latency`,
        // not in any other bot mode. `balance_sol()` could therefore
        // never return anything but its zero-initialized default,
        // regardless of subscription depth or how long a run waited.
        self.wallet.on_account(header, body);
        if let Some(phoenix) = self.state.o_phoenix.as_mut() {
            phoenix.on_account(header, body);
        }
        if let Some(solend_position) = self.state.o_solend_position.as_mut() {
            solend_position.on_account(header, body);
        }
        if let Some(kamino_position) = self.state.o_kamino_position.as_mut() {
            kamino_position.on_account(header, body);
        }
        if let Some(marginfi_position) = self.state.o_marginfi_position.as_mut() {
            marginfi_position.on_account(header, body);
        }
        // No low_latency-freshness gate here (unlike arbv1's
        // is_newer_than_low_latency) -- this mode doesn't track a
        // per-account low-latency slot map, and nothing yet depends on
        // spot_router being perfectly fresh (execute_spot_leg isn't
        // auto-triggered). A late rooted duplicate can only make it
        // briefly less stale, never wrong.
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
        self.state.slot_delta_since_start += 1;
        if let Some(phoenix) = self.state.o_phoenix.as_mut() {
            if let Err(e) = phoenix.flush_pending(self.graph) {
                log_error!("testperpv1: failed to flush phoenix subscriptions: {e}");
            }
        }
        // Breadcrumb, throttled to every 20 slots -- `bulk_subscribe`
        // (called inside both flush calls below) is a confirmed blocking
        // host call. If the guest ever goes permanently silent (no more
        // commit/gap logs at all, as happened this session), whichever
        // flush call's own timing never got logged below is the one that
        // never returned -- this line is the last-known-position marker
        // for that case, so it needs to land *before* the risky calls,
        // not be reconstructed after the fact.
        if self.state.last_slot % 20 == 0 {
            log_warn!(
                "testperpv1: slot {} entering subscription flush (dex pending={:?}/active={:?})",
                self.state.last_slot,
                self.state
                    .o_dex
                    .as_ref()
                    .map(|d| d.subscription_pending_count()),
                self.state
                    .o_dex
                    .as_ref()
                    .map(|d| d.subscription_active_count()),
            );
        }
        let t_dex_flush = std::time::Instant::now();
        if let Some(mut dex) = self.state.o_dex.take() {
            if let Err(e) = dex.flush_pool(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("testperpv1: failed to flush dex subscriptions: {e}");
            }
            // Drains a bounded slice of DexState's own queued startup
            // subscription burst (~32,000 requests across every sub-dex)
            // -- same 128/slot pacing as `subscription_queue` below, just
            // a separate queue instance owned by `DexState` itself.
            if let Err(e) = dex.flush_subscriptions(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("testperpv1: failed to flush dex subscription queue: {e}");
            }
            self.state.o_dex.replace(dex);
        }
        let dex_flush_ms = t_dex_flush.elapsed().as_millis();
        // Unconditional, once per commit -- same per-commit rate as
        // graph.rs's `commit:border`/`commit:done` bookends, not a new
        // log-volume source. Localizes a future hang to one specific
        // flush call: if this line is missing for a commit whose
        // `commit:done` did print, the freeze is in `dex.flush_pool`/
        // `dex.flush_subscriptions` above; if this line prints but
        // nothing ever follows, it's in `subscription_queue.flush`
        // below instead.
        log_warn!(
            "testperpv1: slot {} dex flush done ({dex_flush_ms}ms), entering wallet subscription_queue flush",
            self.state.last_slot,
        );
        // Drain a bounded slice of the queued startup subscription burst
        // per slot, instead of one giant blocking bulk_subscribe call --
        // see SubscriptionQueue's own doc comment for the real,
        // live-observed motivation.
        let t_queue_flush = std::time::Instant::now();
        match self
            .state
            .subscription_queue
            .flush(self.graph, MAX_SUBSCRIBES_PER_SLOT)
        {
            Ok(0) => {}
            Ok(n) => {
                log_warn!(
                    "testperpv1: subscription_queue flushed {n} requests ({} still pending, {} active)",
                    self.state.subscription_queue.pending_count(),
                    self.state.subscription_queue.active_count(),
                );
            }
            Err(e) => {
                log_error!("testperpv1: subscription_queue flush failed: {e}");
            }
        }
        let queue_flush_ms = t_queue_flush.elapsed().as_millis();
        // Unconditional, once per commit -- bookends "dex flush done" so
        // a hang inside `subscription_queue.flush` (present) vs. one
        // that happens *after* `finish()` returns entirely (absent) can
        // be told apart -- closes the last gap in `finish()` itself.
        log_warn!(
            "testperpv1: slot {} finish() returning (queue_flush {queue_flush_ms}ms)",
            self.state.last_slot,
        );
        // Real-time budget check -- this commit's own start()-to-here
        // span (every on_account/on_token call plus the flush work
        // above), distinct from start()'s gap-since-previous-start check.
        // Deliberately only logs when over budget, not every slot: this
        // fires on every single commit, and unconditional per-slot
        // logging is exactly the kind of volume that was already found
        // to contribute to real `stdio timeout` disconnects this session
        // (see `test_swap_to_usdc`'s doc comment on `test_mark_action`
        // for that incident).
        if let Some(start) = self.state.o_commit_start_instant.take() {
            let elapsed_ms = start.elapsed().as_millis();
            if elapsed_ms > SLOT_TIMING_TARGET_MS {
                log_warn!(
                    "testperpv1: slot {} took {}ms to process (target: <{}ms) -- \
                     of that, {}ms was in dex pool/subscription flush and {}ms was in \
                     wallet subscription_queue flush",
                    self.state.last_slot,
                    elapsed_ms,
                    SLOT_TIMING_TARGET_MS,
                    dex_flush_ms,
                    queue_flush_ms,
                );
            }
        }
        // Periodically report this wallet's most-referenced accounts back
        // to the optimizer -- see Self::ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS.
        if self.state.last_slot % Self::ACCOUNT_USAGE_REPORT_INTERVAL_SLOTS == 0 {
            let top = self
                .wallet
                .top_account_usage(Self::ACCOUNT_USAGE_REPORT_MAX_ENTRIES);
            if !top.is_empty() {
                self.q_msg.push_back(MessageSend::CommonAddressUpdate(top));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(
        symbol: &str,
        account_id: AccountId,
        balance_raw: u64,
        value_usd: f64,
        target_pct: f64,
    ) -> (String, AssetHolding) {
        (
            symbol.to_string(),
            AssetHolding {
                account_id,
                balance_raw,
                value_usd,
                target_pct,
            },
        )
    }

    #[test]
    fn plan_rebalance_legs_buys_when_underweight() {
        // $1000 total, SOL target 30% ($300), currently worth $100 --
        // needs a $200 buy.
        let holdings = vec![holding("SOL", 1, 1_000_000_000, 100.0, 0.30)];
        let (sells, buys, scale) = plan_rebalance_legs(&holdings, 1000.0, 1_000_000.0, 0.0);

        assert!(sells.is_empty());
        assert_eq!(scale, 1.0, "ample capital -- no scaling expected");
        assert_eq!(buys.len(), 1);
        let (symbol, account_id, amount_in) = &buys[0];
        assert_eq!(symbol, "SOL");
        assert_eq!(*account_id, 1);
        assert_eq!(*amount_in, 200_000_000); // $200 -> raw USDC (1e6 scale)
    }

    #[test]
    fn plan_rebalance_legs_sells_when_overweight() {
        // $1000 total, BTC target 10% ($100), currently worth $250 --
        // needs to sell 60% of the current raw balance ($150 / $250).
        let holdings = vec![holding("BTC", 2, 1_000_000, 250.0, 0.10)];
        let (sells, buys, _) = plan_rebalance_legs(&holdings, 1000.0, 1_000_000.0, 0.0);

        assert!(buys.is_empty());
        assert_eq!(sells.len(), 1);
        let (symbol, account_id, amount_in) = &sells[0];
        assert_eq!(symbol, "BTC");
        assert_eq!(*account_id, 2);
        assert_eq!(*amount_in, 600_000); // 60% of 1_000_000 raw
    }

    #[test]
    fn plan_rebalance_legs_skips_deltas_under_dust_threshold() {
        // $1000 total, ETH target 10% ($100), currently worth $100.50 --
        // $0.50 delta, below the $1 dust threshold.
        let holdings = vec![holding("ETH", 3, 500_000, 100.50, 0.10)];
        let (sells, buys, _) = plan_rebalance_legs(&holdings, 1000.0, 1_000_000.0, 0.0);

        assert!(
            sells.is_empty(),
            "expected no sell leg for a sub-dust delta: {sells:?}"
        );
        assert!(
            buys.is_empty(),
            "expected no buy leg for a sub-dust delta: {buys:?}"
        );
    }

    #[test]
    fn plan_rebalance_legs_sorts_by_symbol_for_determinism() {
        // All three underweight (all buys) -- HashMap iteration order is
        // arbitrary, so feed them in reverse-alphabetical order and
        // confirm the output is still alphabetical.
        let holdings = vec![
            holding("XRP", 3, 0, 0.0, 0.10),
            holding("ETH", 2, 0, 0.0, 0.10),
            holding("BTC", 1, 0, 0.0, 0.10),
        ];
        let (_, buys, _) = plan_rebalance_legs(&holdings, 1000.0, 1_000_000.0, 0.0);

        let symbols: Vec<&str> = buys.iter().map(|(s, _, _)| s.as_str()).collect();
        assert_eq!(symbols, vec!["BTC", "ETH", "XRP"]);
    }

    #[test]
    fn plan_rebalance_legs_produces_sells_and_buys_together() {
        // SOL overweight (sell), BTC underweight (buy), in one pass.
        let holdings = vec![
            holding("SOL", 1, 1_000_000_000, 400.0, 0.10), // target $100, sell $300 worth
            holding("BTC", 2, 1_000_000, 50.0, 0.30),      // target $300, buy $250 worth
        ];
        let (sells, buys, scale) = plan_rebalance_legs(&holdings, 1000.0, 1_000_000.0, 0.0);

        assert_eq!(scale, 1.0, "ample capital -- no scaling expected");
        assert_eq!(sells.len(), 1);
        assert_eq!(sells[0].0, "SOL");
        assert_eq!(buys.len(), 1);
        assert_eq!(buys[0].0, "BTC");
        assert_eq!(buys[0].2, 250_000_000);
    }

    #[test]
    fn plan_rebalance_legs_empty_holdings_produce_no_legs() {
        let (sells, buys, _) = plan_rebalance_legs(&[], 1000.0, 1_000_000.0, 0.0);
        assert!(sells.is_empty());
        assert!(buys.is_empty());
    }

    #[test]
    fn plan_rebalance_legs_scales_buys_proportionally_when_capital_is_short() {
        // BTC and ETH each want a $200 buy ($400 total desired), but
        // only $100 USDC is on hand and nothing is being sold this
        // pass -- capital covers 25% of demand, so both buys should be
        // scaled to 25%, not one fully funded and the other starved.
        let holdings = vec![
            holding("BTC", 1, 0, 0.0, 0.20), // target $200
            holding("ETH", 2, 0, 0.0, 0.20), // target $200
        ];
        let (sells, buys, scale) = plan_rebalance_legs(&holdings, 1000.0, 100.0, 0.0);

        assert!(sells.is_empty());
        assert_eq!(scale, 0.25);
        assert_eq!(buys.len(), 2);
        for (_, _, amount_in) in &buys {
            assert_eq!(*amount_in, 50_000_000); // $50 (25% of $200) -> raw USDC
        }
    }

    #[test]
    fn plan_rebalance_legs_suppresses_buys_when_reserve_exceeds_available_usdc() {
        // Mirrors the real wallet's situation this session: $3.95 USDC
        // on hand, but the funding-arb margin reserve alone ($10)
        // already exceeds it -- available_for_buys clamps to 0, so no
        // buy should be sized at all, not a tiny/rounded one.
        let holdings = vec![holding("SOL", 1, 0, 0.0, 0.30)]; // target $300
        let (sells, buys, scale) = plan_rebalance_legs(&holdings, 1000.0, 3.95, 10.0);

        assert!(sells.is_empty());
        assert_eq!(scale, 0.0);
        assert!(
            buys.is_empty(),
            "expected no buy when the reserve exceeds available USDC: {buys:?}"
        );
    }

    #[test]
    fn plan_rebalance_legs_sells_are_unaffected_by_the_capital_reserve() {
        // Same overweight-BTC scenario as
        // plan_rebalance_legs_sells_when_overweight, but with a reserve
        // far larger than usdc_value -- selling only ever frees USDC,
        // so it must produce the identical sell leg regardless.
        let holdings = vec![holding("BTC", 2, 1_000_000, 250.0, 0.10)];
        let (sells, buys, _) = plan_rebalance_legs(&holdings, 1000.0, 0.0, 1000.0);

        assert!(buys.is_empty());
        assert_eq!(sells.len(), 1);
        let (symbol, account_id, amount_in) = &sells[0];
        assert_eq!(symbol, "BTC");
        assert_eq!(*account_id, 2);
        assert_eq!(*amount_in, 600_000);
    }

    #[test]
    fn decide_basis_trade_deposit_hedge_on_positive_funding() {
        // Positive funding is always worth a deposit-hedge -- no
        // threshold against Solend's rate, since depositing only ever
        // earns, never costs.
        assert_eq!(
            decide_basis_trade(5.0, 20.0),
            Some(BasisDirection::DepositHedge)
        );
        assert_eq!(
            decide_basis_trade(0.01, 0.0),
            Some(BasisDirection::DepositHedge)
        );
    }

    #[test]
    fn decide_basis_trade_borrow_hedge_only_when_funding_exceeds_borrow_cost() {
        // -20% funding vs 5% borrow APY -- funding collected comfortably
        // exceeds the interest paid.
        assert_eq!(
            decide_basis_trade(-20.0, 5.0),
            Some(BasisDirection::BorrowHedge)
        );
        // -3% funding vs 5% borrow APY -- borrowing would cost more than
        // the funding collected, not profitable.
        assert_eq!(decide_basis_trade(-3.0, 5.0), None);
    }

    #[test]
    fn decide_basis_trade_none_for_zero_funding() {
        assert_eq!(decide_basis_trade(0.0, 5.0), None);
    }

    #[test]
    fn decide_basis_trade_none_at_exact_borrow_cost_boundary() {
        // Exactly equal to the borrow cost -- no edge after paying it.
        assert_eq!(decide_basis_trade(-5.0, 5.0), None);
    }
}
