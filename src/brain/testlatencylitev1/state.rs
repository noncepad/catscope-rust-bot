//! Reactive decision loop for a Phoenix-perp-funding-vs-lending-rate basis
//! trade -- mirrors `arbv1::state`/`phoenixperpsv1::state`'s shape
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
    brain::testlatencylitev1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::{
        shooter::{Header, Tokenaccountv1},
        transactionprocessor,
    },
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
    signature::{Keypair, Signature},
    signer::Signer,
};
use solana_system_interface::instruction::transfer as system_transfer;
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::{Instant, SystemTime, UNIX_EPOCH},
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
    /// Selected instead of `SwapToUsdc` (and everything after it) when
    /// `TestProtocol::Native` is chosen -- a self-contained loop, not a
    /// bootstrap/deposit/withdraw sequence, so it's a single phase rather
    /// than several. See [`StateHelper::test_native_transfer_loop`] for
    /// the real state machine (deliberately not modeled as more `TestPhase`
    /// variants, since there's no natural bootstrap/deposit/withdraw
    /// split for a plain System Program transfer -- just "send, then wait
    /// for a read, then send the other way").
    NativeTransferLoop,
    Done,
}

/// Which real update channel actually delivered a native-transfer's
/// confirmation first -- see [`StateHelper::test_native_transfer_loop`]'s
/// doc comment for why this can't be predicted in advance (all three
/// channels update the same underlying `Wallet::on_account`-tracked SOL
/// balance; whichever event happens to arrive first wins the race).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UpdateLane {
    /// `Event::LowLatency` -- accounts as soon as "processed" (this
    /// codebase's own doc comments elsewhere put this at ~400ms).
    LowLatency,
    /// `Event::Commit` -- accounts once "rooted"/finalized (~12s, per the
    /// same doc comments).
    Commit,
    /// `Event::Transaction` -- the transaction's own signature observed
    /// confirmed, independent of either account-update channel above.
    Transaction,
}

/// Send->confirm latency samples for one [`TestPhase`] -- microsecond
/// samples, same shape as `helloworldv1::state::TxLatencyStats`, but kept
/// per-phase here (one instance per `TestPhase` in `State::tx_latency`)
/// instead of a single aggregate bucket, since the whole point of this
/// module is comparing latency *across* phases (a plain Solend deposit vs.
/// a Kamino farm-gated borrow, say), not just an overall number.
/// Non-destructive: unlike `helloworldv1`'s version, [`Self::stats`]
/// doesn't clear `samples` on read, because a phase's samples are only
/// ever read once, at the point `evaluate_inner` advances past that
/// phase for good -- there's nothing left to reset for.
#[derive(Debug, Default)]
pub(crate) struct TxLatencyStats {
    samples: Vec<u64>,
}

impl TxLatencyStats {
    fn record(&mut self, elapsed: std::time::Duration) {
        self.samples.push(elapsed.as_micros() as u64);
    }
    /// `(n, p50_us, p99_us)` -- `(0, 0, 0)` if no samples were ever
    /// recorded for this phase (e.g. it advanced via the withdraw phases'
    /// fire-once-then-cooldown-advance path without ever landing a
    /// confirmed tx yet at report time).
    fn stats(&self) -> (u64, u64, u64) {
        percentiles(&self.samples)
    }
}

/// `(n, p50, p99)` over `samples` -- `(0, 0, 0)` for an empty slice.
/// Shared by [`TxLatencyStats::stats`] and the native-transfer write/read
/// breakdown below (`StateHelper::report_native_stats`), which needs the
/// exact same n/p50/p99 shape over plain `u64` distributions that aren't
/// always microsecond durations (slot counts, in one case).
fn percentiles(samples: &[u64]) -> (u64, u64, u64) {
    if samples.is_empty() {
        return (0, 0, 0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len() as u64;
    let p50 = sorted[(sorted.len().saturating_sub(1) * 50) / 100];
    let p99 = sorted[(sorted.len().saturating_sub(1) * 99) / 100];
    (n, p50, p99)
}

/// Which protocol's deposit<->withdraw cycle to actually run, read once
/// (via [`Self::from_env`]) from the `TEST_PROTOCOL` env var
/// (`solend`/`kamino`/`marginfi`, case-insensitive) -- lets a real run
/// spend real transaction fees against just one protocol at a time
/// instead of all three plus every borrow/repay leg in one go. Unset (or
/// an unrecognized value) preserves the original full-sequence behavior
/// via `State::o_target_protocol` being `None` -- see that field's doc
/// comment for how the two call sites that check it fall back.
/// One account a pending native transfer can be confirmed through --
/// `check_native_transfer_arrival` races *both* of a transfer's
/// participants independently (see `NativePending::watches`), not just
/// the recipient, so whichever one's own update channel happens to
/// deliver first still gets to claim the confirmation.
///
/// Real, live-verified 2026-09-03: watching only the recipient used to
/// mean an owner-recipient transfer could *only* ever be confirmed via
/// the Transaction (signature) lane, never Account/Commit, no matter how
/// promptly those delivered -- `owner`'s own balance delta on that side
/// bundles together the amount received *and* the fee it pays as this
/// wallet's permanent fee payer, and the old fee estimate silently
/// undercounted the real cost (missing the priority fee entirely -- see
/// `Wallet::priority_fee_lamports`), so the exact-match threshold below
/// was permanently unreachable. `wallet2`, by contrast, never pays a fee
/// in either direction (see the funding-wait check in
/// `test_native_transfer_loop`, which requires `owner` alone to cover
/// `owner_overhead`), so its delta is always exactly `±amount` -- no fee
/// model needed at all on that side. Watching both means: the `owner`
/// leg still races (now with the *exact* real fee folded in, not an
/// estimate), and the `wallet2` leg is a second, fee-free shot at the
/// same confirmation that can win first regardless of whether the
/// `owner`-side math is ever exactly right.
#[derive(Debug, Clone, Copy)]
struct NativeWatch {
    account: AccountId,
    /// This account's own balance (raw lamports) at the moment this
    /// transfer was sent.
    baseline_lamports: u64,
    /// The exact number of lamports this account's balance is expected to
    /// change by once this transfer lands -- positive for a gain,
    /// negative for a loss. Always the real, exact value, never an
    /// estimate (see this struct's own doc comment for why that's
    /// achievable for both participants). `check_native_transfer_arrival`
    /// requires the *exact* threshold this implies to be reached, not
    /// just any change in the expected direction -- necessary, not just
    /// defensive: real, live-verified 2026-09-01 that a *bare*
    /// `now > baseline` check lets a stray/unrelated lamport increase on
    /// the recipient (observed: origin still unidentified, but real
    /// on-chain confirmations showed a small excess crediting the wrong
    /// pending transfer) falsely satisfy the claim -- which let the loop
    /// advance and re-issue a *second* same-direction send before the
    /// first one's real effect had landed, producing two genuine
    /// `insufficient lamports` on-chain failures once the sender's true
    /// balance ran out.
    delta_lamports: i64,
}

/// One in-flight native SOL transfer, from the moment it's sent until
/// whichever [`UpdateLane`] notices either participant's balance change
/// first. See `State::o_native_pending`'s doc comment for the
/// take()-as-claim pattern this is used with.
#[derive(Debug, Clone, Copy)]
struct NativePending {
    sent_at: std::time::Instant,
    /// Both participants' independent arrival watches -- always exactly
    /// `[from, to]`, in that order, but checked without regard to order:
    /// `check_native_transfer_arrival` just looks for whichever one
    /// matches `account_id`. See [`NativeWatch`]'s doc comment for why
    /// both are worth watching instead of just the recipient.
    watches: [NativeWatch; 2],
    /// This transfer's own transaction signature, filled in by
    /// `evaluate_inner`'s send loop right after `Wallet::assemble()`
    /// produces it (unknown at the moment `test_native_transfer_loop`
    /// queues the instruction -- signing happens later, in `assemble()`).
    /// `mid_on_tx` checks a confirming signature against this before
    /// crediting the Transaction lane -- see that check's own doc comment
    /// for the bug this guards against (a *different*, older native
    /// transfer's delayed confirmation arriving while this one is still
    /// the pending one, and getting misattributed to it).
    sig: Option<Signature>,
    /// The freshest slot number this guest had observed (via
    /// `Event::SlotStatus`, tracked in `State::slot_clock`) at the moment
    /// this transfer was sent -- approximates the first slot this
    /// transaction could possibly have been included in (its first real
    /// leader opportunity). Compared against the transfer's actual
    /// inclusion slot (from the winning lane's own account/commit/
    /// transaction-result slot) to separate write/landing delay from
    /// read-propagation delay -- see [`NativeTransferSample`].
    send_slot: Slot,
}

/// One completed native transfer's full write->read breakdown -- built by
/// `StateHelper::record_native_read`, one per confirmed transfer, kept in
/// `State::native_samples`. Answers the question this instrumentation
/// exists for: is a slow write→read cycle caused by the transaction being
/// slow to *land* (write delay: send -> actual on-chain inclusion), or by
/// slow *propagation back to the guest* once it's already landed (read
/// delay: inclusion -> the winning lane observing it)?
#[derive(Debug, Clone, Copy)]
struct NativeTransferSample {
    send_slot: Slot,
    /// The slot this transfer's transaction actually landed in --
    /// read directly off whichever real source resolved it: an account
    /// update's own `header.slot` (LowLatency/Commit) or the confirmed
    /// transaction's own `Ok(slot)` result (Transaction lane, from
    /// `TransactionList::transaction()` -- previously discarded here,
    /// see `mid_on_tx`).
    inclusion_slot: Slot,
    slots_until_inclusion: u64,
    /// Wall-clock time from send until this guest's own validator first
    /// received evidence of `inclusion_slot`'s block at all
    /// (`SlotStatus::FirstShredReceived`), looked up against this guest's
    /// own local slot clock (`State::slot_clock`) -- a real measured
    /// duration, not `slots_until_inclusion` times an assumed ~350ms/slot
    /// constant (see `SLOT_TIMING_TARGET_MS`'s own doc comment on why
    /// that constant isn't a safe stand-in for real slot timing).
    ///
    /// This guest has no channel exposing the leader's own internal
    /// clock -- `FirstShredReceived` (the earliest evidence *this*
    /// validator, a downstream observer, ever gets that the leader built
    /// and started broadcasting the block) is the best available proxy
    /// for "the leader saw/included this transfer," not a literal
    /// timestamp of the leader's own inclusion decision. A genuine
    /// *lower* bound: the leader must have already included the transfer
    /// by the time any shred of that block reaches us.
    ///
    /// `None` whenever this can't be resolved to a real positive
    /// duration -- either `inclusion_slot` aged out of `slot_clock`
    /// entirely (a write delay long enough to exceed
    /// `SLOT_CLOCK_CAPACITY` slots), or `slot_clock`'s recorded
    /// `FirstShredReceived` instant for `inclusion_slot` turned out to be
    /// at or before `sent_at` (a same-slot sample -- this guest was
    /// already mid-slot when it sent). Real, live-user-caught bug
    /// 2026-09-02: this used to `map` straight to `Instant::duration_since`,
    /// which silently saturates a same-or-earlier instant to
    /// `Duration::ZERO` instead of `None` -- producing a fake
    /// `write_delay = 0`, which then made `read_delay = total_latency - 0
    /// = total_latency`, misattributing the *entire* unresolvable
    /// same-slot write time onto the read lane. See
    /// `write_delay_upper_bound` for what this reports instead when a
    /// real value can't be resolved at all.
    write_delay: Option<std::time::Duration>,
    /// Only meaningful when `write_delay` is `None` (always `None`
    /// itself otherwise). The wall-clock instant this guest first
    /// observed *some* slot strictly after `inclusion_slot` reach
    /// `FirstShredReceived` (already sitting in `slot_clock` by confirm
    /// time regardless of this transfer, since the chain keeps producing
    /// slots continuously in the background) minus `sent_at` -- a real,
    /// honest upper bound for a same-slot sample with no resolvable point
    /// estimate at all: the transaction landed in `inclusion_slot`, so
    /// this guest can't have seen its first shred any later than the
    /// first shred of the *next* slot it saw. Not a point estimate --
    /// reported separately from resolved `write_delay`/`read_delay`
    /// percentiles, never blended into them. `None` if no later slot has
    /// been observed to reach `FirstShredReceived` yet either (e.g. this
    /// was the very last transfer of the run).
    write_delay_upper_bound: Option<std::time::Duration>,
    /// `total_latency - write_delay` -- the remaining time it took this
    /// guest to notice the transfer once the leader had (at least)
    /// started broadcasting it. Covers everything real that happens
    /// after `FirstShredReceived`: the rest of block delivery
    /// (`shred_to_completed`), this validator's own local replay
    /// (`completed_to_processed`), cluster confirmation
    /// (`processed_to_confirmed`), and this guest's own dispatch/
    /// backpressure overhead -- none of those are subtracted back out,
    /// since from this guest's send-to-observe perspective they're all
    /// genuinely part of "the remaining time for us to see it." `None`
    /// whenever `write_delay` is `None` -- see that field's doc comment.
    /// Never derived as `total_latency - 0` as a stand-in for an
    /// unresolved write delay.
    read_delay: Option<std::time::Duration>,
    /// Full send -> observed latency, same quantity `native_read_latency`
    /// buckets by lane -- kept here too so this sample is a complete,
    /// standalone record of the transfer. Always real and exact,
    /// regardless of whether `write_delay`/`read_delay` resolved --
    /// unaffected by the same-slot ambiguity those two can hit.
    total_latency: std::time::Duration,
    lane: UpdateLane,
    /// Real, measured breakdown of what makes up `read_delay` -- `None`
    /// whenever either endpoint's timestamp never resolved. All three are
    /// independent, supplementary stats: real sub-stages of the time
    /// between `FirstShredReceived` and this guest's own observation, but
    /// never subtracted from `read_delay` (which already, deliberately,
    /// counts all of them as real "time for us to see it" -- see that
    /// field's own doc comment).
    /// `shred_to_completed`: how long after the first shred arrived did
    /// the rest of this slot's block finish arriving (real, measured
    /// answer to "how spread out is block delivery here").
    shred_to_completed: Option<std::time::Duration>,
    /// `completed_to_processed`: this validator's own local replay time
    /// once the block was fully assembled -- expected to be small (own
    /// CPU work).
    completed_to_processed: Option<std::time::Duration>,
    /// `processed_to_confirmed`: real cluster-wide supermajority
    /// vote-confirmation lag after this validator already replayed the
    /// slot locally. Real, corrected 2026-09-04: originally assumed to
    /// often be `None` because confirmation "hasn't happened yet" by
    /// observation time -- live data showed the opposite (17/20 samples
    /// in one run already had this resolved). Purely informational, like
    /// the other two above -- not subtracted from `read_delay`.
    processed_to_confirmed: Option<std::time::Duration>,
    /// This transfer's own real ordinal position within `inclusion_slot`'s
    /// block (Agave's real `ReplicaTransactionInfo::index`, already flowing
    /// end-to-end through this pipeline -- see
    /// `State::m_native_tx_index`'s doc comment for how this gets
    /// backfilled). `None` until `mid_on_tx` happens to see this signature's
    /// own transaction data -- which, since the Account/LowLatency lane
    /// wins essentially every race, normally happens strictly *after* this
    /// sample is first pushed, not at push time. Not yet used for anything
    /// beyond transparency -- turning this into a real position-within-block
    /// estimate (normalizing against how many transactions the block
    /// actually had) is a separate, later step.
    tx_index: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestProtocol {
    Solend,
    Kamino,
    Marginfi,
    /// Plain System Program SOL transfers, bouncing between two wallets --
    /// deliberately protocol-agnostic (no lending-protocol accounts, no
    /// bootstrap step, no USDC conversion needed at all). See
    /// [`StateHelper::test_native_transfer_loop`].
    Native,
}

impl TestProtocol {
    fn from_env() -> Option<Self> {
        match std::env::var("TEST_PROTOCOL").ok()?.to_lowercase().as_str() {
            "solend" => Some(Self::Solend),
            "kamino" => Some(Self::Kamino),
            "marginfi" => Some(Self::Marginfi),
            "native" => Some(Self::Native),
            _ => None,
        }
    }
    /// The `Bootstrap*` phase `test_swap_to_usdc` should jump to once
    /// there's enough USDC, when this protocol is the one selected to
    /// run in isolation -- skips the other two protocols' bootstrap
    /// phases entirely (no transactions sent against them at all), not
    /// just their deposit/withdraw cycles. `Native` never calls this --
    /// it bypasses `SwapToUsdc` entirely from `State::default()` instead,
    /// since it doesn't need USDC (or any bootstrap step) at all.
    fn bootstrap_phase(self) -> TestPhase {
        match self {
            Self::Solend => TestPhase::BootstrapSolend,
            Self::Kamino => TestPhase::BootstrapKamino,
            Self::Marginfi => TestPhase::BootstrapMarginfi,
            Self::Native => TestPhase::NativeTransferLoop,
        }
    }
}

/// Per-slot wall-clock timestamps for the real `SlotStatus` transitions
/// this test's write/propagate decomposition cares about -- each set
/// once, at the first time this guest sees that specific status for
/// that slot number. Real signals from the validator's own geyser
/// plugin, not something this guest infers or estimates -- traced
/// 2026-09-04: `catscope-bot`'s `apifs.rs` imports `SlotStatus` directly
/// from `agave_geyser_plugin_interface::geyser_plugin_interface` (the
/// real Agave validator's own geyser type), and
/// `catscope_zerohop::store::convert_slot_status_from`/`_to` carry every
/// one of these six variants across the wire unstubbed -- nothing here
/// is fabricated or dropped before it reaches this guest.
///
/// - `first_shred`: the first real network shred for this slot arrived
///   at this validator (Solana's Turbine propagation) -- the earliest
///   signal available, likely well before the block is complete, since
///   a leader streams shreds continuously through its ~400ms slot
///   rather than all at once at the end.
/// - `completed`: this validator has received every shred for this slot
///   -- the block is now fully assembled locally. `first_shred` ->
///   `completed` is the real, measured answer to "how spread out is
///   block delivery on this validator" -- previously only guessed at.
/// - `processed`: this validator has locally replayed (executed) the
///   slot -- the real "landed and its effects are visible" boundary.
/// - `confirmed`: real cluster-wide supermajority vote acknowledgment.
///   Originally assumed to routinely arrive *after* this guest already
///   knows the answer via the Account lane (which only needs local
///   `processed` state, not cluster consensus) -- real, live-measured
///   data corrected that 2026-09-04: 17/20 samples in one run already
///   had this resolved, meaning `Confirmed` had genuinely landed
///   *before* the observing lane claimed the transfer in most cases.
///   See `record_native_read`'s own doc comment for why this is
///   subtracted from `read_delay` when resolved (and treated as zero,
///   not unknown, when it isn't -- an unresolved `confirmed` means
///   confirmation demonstrably hasn't happened yet within the window
///   already measured, not that its contribution is unknown).
/// One sample's resolved result from `StateHelper::tx_index_estimate` --
/// see that method's doc comment. Carries the real inputs used
/// (`tx_index`, `slot_max_tx_index`) alongside the derived estimate so a
/// per-sample log line (and, downstream, a real per-row report column)
/// can show its own actual numbers, not just an aggregate percentile.
#[derive(Debug, Clone, Copy)]
struct TxIndexEstimate {
    tx_index: u64,
    slot_max_tx_index: u64,
    write_delay_estimate: std::time::Duration,
}

#[derive(Debug, Clone, Copy, Default)]
struct SlotTimestamps {
    first_shred: Option<std::time::Instant>,
    completed: Option<std::time::Instant>,
    processed: Option<std::time::Instant>,
    confirmed: Option<std::time::Instant>,
    /// Largest real Agave `ReplicaTransactionInfo::index` (`tx.index`,
    /// this transaction's exact ordinal position within its block) seen
    /// by `mid_on_tx` for this slot, across *every* transaction that
    /// passed through it -- not just our own native transfers. A real,
    /// honest lower bound on "how many transactions this block actually
    /// had," never a guarantee we saw the block's true last transaction
    /// (see `TX_INDEX_ESTIMATE_PLAN.md`'s "one real approximation left"
    /// section). Feeds Phase 3's fractional position-within-block
    /// estimate (`NativeTransferSample::tx_index` / this value); not yet
    /// consulted anywhere -- Phase 2 only captures it.
    max_tx_index: Option<u64>,
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
    /// When this guest first started waiting on the bundler tip
    /// broadcaster's first status, if it's currently waiting -- a real
    /// wall-clock `Instant`, not `test_last_action_slot`. Real, live bug
    /// 2026-09-04: this used to reuse `test_last_action_slot`/
    /// `test_cooldown_active` (slot-number-based), which silently never
    /// waited at all -- `last_slot` (rooted/Commit tier) is still `0`
    /// this early in a run, so the moment the first real Commit event
    /// arrived and jumped it to a real slot number in the hundreds of
    /// millions, the cooldown was blown through instantly. See
    /// `test_native_transfer_loop`'s own doc comment on the bundler-wait
    /// gate for the real wall-clock budget this uses instead.
    o_bundler_wait_started: Option<std::time::Instant>,
    /// Every signature this test has sent but not yet observed on-chain,
    /// tagged with the [`TestPhase`] that sent it and the `Instant` it was
    /// sent at -- populated at the send point in `evaluate_inner`'s
    /// `assemble()`/send loop, consumed by `mid_on_tx` on a match against
    /// a real `Event::Transaction`. Mirrors `helloworldv1::State::m_sig`
    /// (see that module's `TxLatencyStats`/`m_sig` doc comments for the
    /// same pattern), tagged by `TestPhase` instead of `Slot` since the
    /// signal this module cares about is per-phase latency, not
    /// slot correlation.
    m_sig: HashMap<Signature, (TestPhase, Instant)>,
    /// Send->confirm latency samples, bucketed by [`TestPhase`] -- see
    /// [`TxLatencyStats`]'s own doc comment. One entry per phase that has
    /// sent at least one transaction so far; a phase not yet reached (or
    /// one that never landed a confirmed tx) simply has no entry.
    tx_latency: HashMap<TestPhase, TxLatencyStats>,
    /// How many deposit<->withdraw cycles have completed for the protocol
    /// currently in its Deposit/Withdraw phase pair -- see
    /// `StateHelper::CYCLE_TARGET`. Reset to 0 every time a fresh
    /// protocol's Deposit phase starts a new cycle sequence.
    cycle_count: u32,
    /// Wall-clock instant the write half of the *current* deposit or
    /// withdraw attempt (within a cycle) was sent -- `None` once its
    /// matching low-latency read has been observed and recorded (see
    /// `StateHelper::record_cycle_read`), so a stray/duplicate
    /// `on_account` update found after that can't double-count.
    /// Distinct from `m_sig`/`tx_latency` above: this measures
    /// write->real-state-visible-via-`on_account` latency, not
    /// write->tx-confirmed latency -- seeing a tx's signature go by in
    /// `Event::Transaction` doesn't mean the *account* update carrying
    /// its effect has arrived yet, and that gap is exactly what this
    /// field is tracking.
    cycle_write_sent_at: Option<Instant>,
    /// The deposited-collateral amount (raw units) observed at the
    /// moment the current cycle's write was sent -- the read-confirmed
    /// condition is "moved away from this baseline in the expected
    /// direction", not a plain zero/nonzero check, because
    /// `test_withdraw_solend`/`test_withdraw_kamino` deliberately leave a
    /// dust remainder behind (see `StateHelper::CYCLE_DUST_RAW`), so a
    /// plain "is there a deposit at all" check would already be true from
    /// leftover dust before the new write even lands.
    cycle_write_baseline_amount: u64,
    /// Write->low-latency-read latency samples, bucketed by [`TestPhase`]
    /// -- only ever populated for the Deposit*/Withdraw* phases (see
    /// `cycle_write_sent_at`'s doc). Distinct from `tx_latency` above
    /// (tx-confirm-based); this is the write->`on_account`-visible-update
    /// latency the `CYCLE_TARGET`-repeat exists to measure. Reported via
    /// `StateHelper::report_cycle_stats` once a protocol's `CYCLE_TARGET`
    /// cycles complete.
    cycle_read_latency: HashMap<TestPhase, TxLatencyStats>,
    /// `Some` to run only one protocol's deposit<->withdraw cycle and stop
    /// (skipping the other two protocols' bootstrap phases and every
    /// borrow/repay leg entirely), `None` for the original full 16-phase
    /// sequence -- see [`TestProtocol`]'s doc comment. Read once from the
    /// `TEST_PROTOCOL` env var at construction; checked at the two places
    /// that need to route differently: `test_swap_to_usdc` (which
    /// `Bootstrap*` phase to jump to) and each protocol's withdraw-cycle
    /// completion (advance to `Done` instead of the next protocol's
    /// bootstrap).
    o_target_protocol: Option<TestProtocol>,
    /// The second wallet for `TestProtocol::Native`'s back-and-forth SOL
    /// transfers -- derived deterministically from the primary child
    /// wallet's own secret seed (HKDF-SHA256, see
    /// `StateHelper::ensure_native_second_wallet`), not communicated by
    /// the Go host at all. A locally-generated keypair needs no on-chain
    /// bootstrap to *receive* a plain SOL transfer, so this is simpler
    /// than deriving a second child key on the Go side and sending it
    /// over -- zero Go-side changes needed. `None` until
    /// `ensure_native_second_wallet` runs (lazily, the first time
    /// `test_native_transfer_loop` needs it).
    o_second_wallet: Option<AccountId>,
    /// How many of the 100 native transfers have completed (0..100). The
    /// very first transfer (count still 0) doubles as wallet 2's own
    /// funding -- it starts at zero balance and this is what gives it
    /// enough SOL to make its own return transfers later.
    native_transfer_count: u32,
    /// The in-flight native transfer, if any -- `None` means the next
    /// `evaluate()` should send the next transfer (or, once
    /// `native_transfer_count` reaches 100, report and stop).
    /// `Option::take()` on this is the atomic "claim" a winning update
    /// lane uses so only the *first* of the three lanes to notice the
    /// balance change gets to record it and trigger the next send --
    /// same pattern `cycle_write_sent_at` uses for the protocol-specific
    /// cycles above.
    o_native_pending: Option<NativePending>,
    /// Write->read latency samples for native transfers, bucketed by
    /// [`UpdateLane`] -- which of the three real update channels
    /// (`LowLatency`/`Commit`/`Transaction`) actually delivered the
    /// winning confirmation for a given transfer. Distinct from
    /// `tx_latency` above (which also gets a sample for every native
    /// transfer, under the generic `TestPhase::NativeTransferLoop` key,
    /// via the same `Event::Transaction` correlation every other phase
    /// uses) -- this is the per-lane breakdown the native test exists to
    /// produce.
    native_read_latency: HashMap<UpdateLane, TxLatencyStats>,
    /// This guest's own local slot clock: per-slot wall-clock timestamps
    /// for the real `SlotStatus` transitions this test's decomposition
    /// cares about, in increasing-slot order. Lets a native transfer's
    /// actual inclusion slot (discovered only once it confirms,
    /// potentially several slots after it was sent) be looked up against
    /// real wall-clock time -- see `NativeTransferSample::write_delay`'s
    /// doc comment for why this exists instead of assuming a fixed
    /// ms/slot constant. Bounded at `StateHelper::SLOT_CLOCK_CAPACITY`
    /// entries (a `VecDeque` so trimming the oldest is O(1)).
    slot_clock: VecDeque<(Slot, SlotTimestamps)>,
    /// Largest `header.slot` seen on *any* account/token update passing
    /// through `low_latency()` -- real, live-user-caught fix 2026-09-08
    /// for `current_slot()`'s own staleness: `slot_clock.back()` only
    /// advances when this guest's event loop has actually gotten around
    /// to processing a `SlotStatus` event, which can lag well behind the
    /// real chain tip under backpressure (e.g. right at boot, behind a
    /// queued burst of subscription/funding traffic -- see the real
    /// `slots_until_inclusion` anomalies this was diagnosed from).
    /// Account/token updates arrive at far higher frequency than
    /// `SlotStatus` events and each carries its own real slot number, so
    /// the max of the two is always at least as fresh as `slot_clock`
    /// alone, never less. Used only by `current_slot()` -- everything
    /// keyed on `inclusion_slot` (the real Write/Read split) is
    /// unaffected by this.
    freshest_account_slot: Slot,
    /// One entry per confirmed native transfer, in completion order --
    /// the full write->read breakdown this instrumentation exists to
    /// produce. See [`NativeTransferSample`].
    native_samples: Vec<NativeTransferSample>,
    /// Signature -> index into `native_samples`, for backfilling
    /// [`NativeTransferSample::tx_index`] once `mid_on_tx` happens to see
    /// that signature's own transaction data (which carries the real
    /// `tx.index` -- this validator's own Agave geyser plugin's
    /// `ReplicaTransactionInfo::index`, i.e. this transfer's real ordinal
    /// position within its landing block).
    ///
    /// Deliberately separate from `o_native_pending`/`m_sig`'s own
    /// signature tracking, not a reuse of either: `mid_on_tx`'s existing
    /// `is_current_pending` check (which gates *who wins the race* --
    /// `record_native_read` only ever runs once per transfer, whichever
    /// lane gets there first) is correctness-critical and must not change.
    /// But since the Account/LowLatency lane wins essentially every race
    /// live-verified this session, `o_native_pending` is almost always
    /// already consumed (`.take()`'d) by the time this same signature's
    /// Transaction-lane confirmation arrives later -- gating `tx_index`
    /// capture on that same check would mean it almost never fires. This
    /// map has no such gate: entries are added the moment a sample is
    /// pushed (any lane), and consulted for *every* signature `mid_on_tx`
    /// sees, regardless of whether it's the currently-pending transfer --
    /// removed once backfilled so it can't grow unbounded over a run.
    m_native_tx_index: HashMap<Signature, usize>,
    /// `true` once the end-of-run sweep (both wallets' remaining SOL back
    /// to the real mothership/parent wallet) has been queued -- guards
    /// against re-queuing it every subsequent `evaluate()` tick once
    /// `native_transfer_count` reaches `NATIVE_TRANSFER_TARGET` (that
    /// branch runs on every tick from then on, not just once). Real, live
    /// bug 2026-09-04: before this existed, the run just sat idle in
    /// `TestPhase::Done` forever with both test wallets' SOL still on
    /// them -- recoverable only because `Wallet 1`'s key happens to be
    /// deterministically derivable from the real parent key (see
    /// `common.DeriveChildKeyFromIndex` on the Go side), not because
    /// anything here returned it automatically.
    native_swept: bool,
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
}

impl Default for State {
    fn default() -> Self {
        let o_target_protocol = TestProtocol::from_env();
        // Native bypasses SwapToUsdc (and the whole
        // bootstrap/deposit/withdraw shape below it) entirely -- it needs
        // no USDC conversion and no protocol account to bootstrap, just
        // two wallets and plain System Program transfers. Every other
        // selection (or none) keeps the original TestPhase::default()
        // (SwapToUsdc) starting point unchanged.
        let test_phase = match o_target_protocol {
            Some(TestProtocol::Native) => TestPhase::NativeTransferLoop,
            _ => TestPhase::default(),
        };
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
            test_phase,
            test_last_action_slot: None,
            o_bundler_wait_started: None,
            m_sig: HashMap::default(),
            tx_latency: HashMap::default(),
            cycle_count: 0,
            cycle_write_sent_at: None,
            cycle_write_baseline_amount: 0,
            cycle_read_latency: HashMap::default(),
            o_target_protocol,
            o_second_wallet: None,
            native_transfer_count: 0,
            native_swept: false,
            o_native_pending: None,
            native_read_latency: HashMap::default(),
            slot_clock: VecDeque::default(),
            freshest_account_slot: 0,
            native_samples: Vec::default(),
            m_native_tx_index: HashMap::default(),
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
                        "perpfundingv1: target allocation symbol {} has no curated mint -- cannot resolve to AccountId",
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
        log_warn!("testlatencylitev1: batch-resolved {n_ids}/{n_pk} build-time pubkeys to account ids at startup");
        // THE experimental change this whole module exists for: the real
        // `testperplatencyv1::on_load` does all five of these
        // unconditionally, for every protocol -- see this module's own
        // doc comment for the real, live-measured evidence
        // (~42,000-account DEX/lending subscription, 8,271 updates/tick)
        // that this native-transfer test never needed any of it.
        // `o_phoenix`/`o_solend_position`/`o_kamino_position`/
        // `o_marginfi_position`/`o_dex` are simply left `None` here --
        // `evaluate_inner`'s own gate is relaxed below to match, since it
        // otherwise refuses to dispatch to *any* `TestPhase` (including
        // `NativeTransferLoop`) until all five are populated.
        //
        // Not skipped: `spot_router`'s seeding just below, which builds a
        // static routing graph from build-time config data -- no live
        // subscription, no account IDs, nothing this experiment is
        // trying to avoid -- kept as-is so nothing else in this copied
        // module trips over an unexpectedly-empty router.
        self.state.spot_router = TradeRouter::from_router(&build_liquidity_router());
        log_info!("testlatencylitev1: bot has been successfully uploaded to validator (dex/solend/kamino/marginfi subscriptions skipped -- native-transfer latency experiment)");
    }

    /// How many recent (slot, Instant) pairs `State::slot_clock` keeps.
    /// Generous relative to any write delay actually observed on this
    /// test so far (at most a couple dozen slots) -- comfortably covers
    /// looking up an inclusion slot even under unusually heavy write
    /// delay, without keeping the whole run's slot history around.
    const SLOT_CLOCK_CAPACITY: usize = 500;

    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        if status == SlotStatus::Dead {
            log_info!("perpfundingv1: slot {slot}; status dead");
        }
        // Ensure this slot has an entry, in increasing-slot order (real
        // slot numbers only ever go up, so a plain "is this newer than
        // the last entry" check is enough to dedupe without a full scan).
        if self.state.slot_clock.back().is_none_or(|&(s, _)| s < slot) {
            self.state
                .slot_clock
                .push_back((slot, SlotTimestamps::default()));
            if self.state.slot_clock.len() > Self::SLOT_CLOCK_CAPACITY {
                self.state.slot_clock.pop_front();
            }
        }
        // FirstShredReceived/Completed/Processed for `slot` always
        // arrive while `slot` is at or very near `slot_clock.back()`
        // (real-time signals about the current tip); Confirmed routinely
        // arrives well after slot_clock has advanced many entries past
        // it (cluster confirmation lags local processing -- see
        // `SlotTimestamps::confirmed`'s doc comment). Search backward
        // for the matching entry rather than assuming it's still
        // `back()` -- bounded by `SLOT_CLOCK_CAPACITY`, so this never
        // scans further than that.
        let now = std::time::Instant::now();
        if let Some((_, ts)) = self
            .state
            .slot_clock
            .iter_mut()
            .rev()
            .find(|(s, _)| *s == slot)
        {
            match status {
                SlotStatus::FirstShredReceived => {
                    ts.first_shred.get_or_insert(now);
                }
                SlotStatus::Completed => {
                    ts.completed.get_or_insert(now);
                }
                SlotStatus::Processed => {
                    ts.processed.get_or_insert(now);
                }
                SlotStatus::Confirmed => {
                    ts.confirmed.get_or_insert(now);
                }
                SlotStatus::Rooted | SlotStatus::CreatedBank | SlotStatus::Dead => {}
            }
        }
    }

    /// Best-effort wall-clock `Instant` for the first time this guest saw
    /// `slot`'s first shred (`SlotStatus::FirstShredReceived`). Exact
    /// match when available; otherwise falls back to the closest earlier
    /// recorded slot that has one (not every slot necessarily produces
    /// its own `SlotStatus` event to this guest), which slightly
    /// *understates* the true write delay for that sample rather than
    /// overstating it. `None` only if `slot` predates everything left in
    /// `slot_clock` (aged out under an unusually large write delay, or no
    /// `SlotStatus` event has ever arrived yet).
    fn instant_for_slot(&self, slot: Slot) -> Option<std::time::Instant> {
        self.state
            .slot_clock
            .iter()
            .rev()
            .filter(|&&(s, _)| s <= slot)
            .find_map(|&(_, ts)| ts.first_shred)
    }

    /// Wall-clock `Instant` this guest first observed *some* slot
    /// strictly after `slot` reach `FirstShredReceived` -- used only as
    /// `NativeTransferSample::write_delay_upper_bound`'s real, honest
    /// upper bound when `instant_for_slot(slot)` resolved to an instant
    /// at or before the send (see that field's doc comment for why that
    /// happens and why a point estimate isn't recoverable there). Scans
    /// forward from the front (oldest first) since `slot_clock` is
    /// stored in increasing-slot order -- correct regardless of slot
    /// gaps (not every slot necessarily produces its own `SlotStatus`
    /// event to this guest). `None` if no later slot has been observed
    /// yet either.
    fn instant_after_slot(&self, slot: Slot) -> Option<std::time::Instant> {
        self.state
            .slot_clock
            .iter()
            .filter(|&&(s, _)| s > slot)
            .find_map(|&(_, ts)| ts.first_shred)
    }

    /// Same lookup as `instant_for_slot`, against `SlotTimestamps::completed`
    /// -- used for the `shred_to_completed`/`completed_to_processed`
    /// transparency stats.
    fn completed_instant_for_slot(&self, slot: Slot) -> Option<std::time::Instant> {
        self.state
            .slot_clock
            .iter()
            .rev()
            .filter(|&&(s, _)| s <= slot)
            .find_map(|&(_, ts)| ts.completed)
    }

    /// Same lookup as `instant_for_slot`, against `SlotTimestamps::processed`
    /// -- used only for the `completed_to_processed`/`processed_to_confirmed`
    /// transparency stats (this validator's own local replay time and the
    /// cluster-confirmation lag after it), not as a write/read anchor.
    fn processed_instant_for_slot(&self, slot: Slot) -> Option<std::time::Instant> {
        self.state
            .slot_clock
            .iter()
            .rev()
            .filter(|&&(s, _)| s <= slot)
            .find_map(|&(_, ts)| ts.processed)
    }

    /// Same lookup as `instant_for_slot`, against `SlotTimestamps::confirmed`
    /// -- used only for the `processed_to_confirmed` transparency stat.
    /// Real, corrected 2026-09-04: this used to claim cluster confirmation
    /// "routinely hasn't happened yet" by observation time -- real,
    /// live-measured data contradicted that (17/20 samples in one run
    /// had `processed_to_confirmed` already resolved, meaning `Confirmed`
    /// genuinely landed *before* the observing lane claimed the transfer
    /// in most cases). Purely informational either way -- never
    /// subtracted from `read_delay` (see that field's own doc comment).
    fn confirmed_instant_for_slot(&self, slot: Slot) -> Option<std::time::Instant> {
        self.state
            .slot_clock
            .iter()
            .rev()
            .filter(|&&(s, _)| s <= slot)
            .find_map(|&(_, ts)| ts.confirmed)
    }

    /// Records that `mid_on_tx` saw a transaction with this real `tx.index`
    /// included in `slot` -- called for *every* transaction it sees, not
    /// just our own native transfers, so `SlotTimestamps::max_tx_index`
    /// tracks the largest ordinal position observed for that slot. Same
    /// dedup/capacity discipline as `on_slot_status`: creates a new
    /// `slot_clock` entry when `slot` is newer than everything currently
    /// tracked; otherwise updates the matching existing entry in place.
    /// If `slot` predates everything left in `slot_clock` (aged out) or
    /// falls in a gap this guest never saw a `SlotStatus` event for, this
    /// is a no-op -- same honest-lower-bound tradeoff as everywhere else
    /// `slot_clock` is consulted, not a correctness bug.
    fn record_tx_index_for_slot(&mut self, slot: Slot, index: u64) {
        if self.state.slot_clock.back().is_none_or(|&(s, _)| s < slot) {
            self.state
                .slot_clock
                .push_back((slot, SlotTimestamps::default()));
            if self.state.slot_clock.len() > Self::SLOT_CLOCK_CAPACITY {
                self.state.slot_clock.pop_front();
            }
        }
        if let Some((_, ts)) = self
            .state
            .slot_clock
            .iter_mut()
            .rev()
            .find(|(s, _)| *s == slot)
        {
            ts.max_tx_index = Some(ts.max_tx_index.map_or(index, |m| m.max(index)));
        }
    }

    /// Best-effort largest `tx.index` observed for `slot` (see
    /// `SlotTimestamps::max_tx_index`'s doc comment) -- a real, honest
    /// lower bound on how many transactions `slot`'s block actually had.
    /// Exact match when available; otherwise `None` (unlike
    /// `instant_for_slot`, falling back to an earlier slot's count would
    /// be a meaningless estimate here, not a conservative one, so this
    /// doesn't do it). Used by `write_delay_estimate`'s fractional
    /// position-within-block estimate.
    fn slot_max_tx_index(&self, slot: Slot) -> Option<u64> {
        self.state
            .slot_clock
            .iter()
            .rev()
            .find(|&&(s, _)| s == slot)
            .and_then(|&(_, ts)| ts.max_tx_index)
    }

    /// The freshest slot number this guest has observed, or `0` before
    /// the first `SlotStatus` event ever arrives -- used as
    /// `NativePending::send_slot`, the "first leader opportunity" a
    /// just-sent transfer is measured against.
    ///
    /// Takes the max of `slot_clock`'s own bookkeeping (fed only by
    /// `SlotStatus` events) and `State::freshest_account_slot` (fed by
    /// every account/token update this guest sees, far higher frequency)
    /// -- see that field's doc comment for the real staleness bug this
    /// fixes: `slot_clock.back()` alone can lag the true chain tip when
    /// this guest's `SlotStatus` processing specifically falls behind,
    /// most visibly right at boot behind a queued subscription/funding
    /// burst, inflating `slots_until_inclusion` without `send_slot` ever
    /// having reflected the real slot at send time.
    fn current_slot(&self) -> Slot {
        let from_slot_status = self.state.slot_clock.back().map_or(0, |&(s, _)| s);
        from_slot_status.max(self.state.freshest_account_slot)
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
            // Real-time freshest-slot signal for `current_slot()` -- see
            // `State::freshest_account_slot`'s doc comment. Every account
            // update carries its own real slot number regardless of
            // whether this guest cares about the account itself.
            self.state.freshest_account_slot =
                self.state.freshest_account_slot.max(account.header.slot);
            self.wallet.on_account(account.header, d);
            self.check_native_transfer_arrival(
                account.header.accountid,
                account.header.slot,
                UpdateLane::LowLatency,
            );
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

    /// Correlates every transaction this test has sent (tracked in
    /// `m_sig` at the send point in `evaluate_inner`'s `assemble()`/send
    /// loop) against real on-chain confirmations, recording send->confirm
    /// latency bucketed by the `TestPhase` that sent it -- see this
    /// module's doc comment and `helloworldv1::state::mid_on_tx`'s own
    /// `m_sig`/`tx_latency` pattern, which this mirrors. Unlike
    /// `helloworldv1`, no instruction-level filtering is needed here
    /// (nothing in this module inspects a specific program's
    /// instructions inside a transaction), so `tx.signature` is read
    /// directly instead of walking `ix_inner`/`ix_outer` first.
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while let Some((tx, result)) = transaction_list.transaction() {
            // `Ok(slot)` is the real slot this transaction was included
            // in -- previously discarded here (`if result.is_err() {
            // continue; }`), now the source of
            // `NativeTransferSample::inclusion_slot` for the Transaction
            // lane. Behavior for an errored transaction is unchanged:
            // skip it, same as before.
            let Ok(inclusion_slot) = result else {
                continue;
            };
            // Track this slot's largest `tx.index` across *every*
            // transaction seen here, not just our own native transfers --
            // see `SlotTimestamps::max_tx_index`'s doc comment. Phase 2 of
            // `TX_INDEX_ESTIMATE_PLAN.md`: not yet consulted anywhere,
            // reserved for Phase 3's fractional estimate.
            self.record_tx_index_for_slot(inclusion_slot, tx.index);
            let signature = Signature::from(*tx.signature);
            // Backfill `NativeTransferSample::tx_index` for this signature,
            // if it's one of ours -- independent of `m_sig`/`o_native_pending`
            // below and of which lane actually won this transfer's race
            // (almost always Account/LowLatency, live-verified this
            // session, so `o_native_pending` is usually already consumed by
            // the time this signature's own transaction data shows up
            // here). See `State::m_native_tx_index`'s doc comment.
            if let Some(sample_i) = self.state.m_native_tx_index.remove(&signature) {
                if let Some(sample) = self.state.native_samples.get_mut(sample_i) {
                    sample.tx_index = Some(tx.index);
                }
            }
            let Some((phase, sent_at)) = self.state.m_sig.remove(&signature) else {
                continue;
            };
            let elapsed = sent_at.elapsed();
            self.state
                .tx_latency
                .entry(phase)
                .or_default()
                .record(elapsed);
            log_warn!(
                "testlatencylitev1: {phase:?} tx {signature} confirmed on-chain -- latency {}µs",
                elapsed.as_micros(),
            );
            // NOTE: this is NOT necessarily the current pending native
            // transfer's own signature, even though only one is ever
            // in-flight at a time -- confirmations for OLDER native
            // transfers can (and, live-verified, routinely do) arrive
            // late, well after a faster lane already advanced the loop
            // past them. Real, confirmed live 2026-09-01: without the
            // `pending.sig == Some(signature)` check below, a stale
            // confirmation for an already-claimed earlier transfer was
            // misattributing itself to whatever transfer happened to be
            // pending *now* -- 39 of 60 "Transaction lane" wins in that
            // run turned out to be exactly this, verified by cross-
            // referencing each claimed latency against every signature's
            // own independently-logged confirm time and finding no
            // match. Checking the signature itself (not just the phase)
            // is what makes this correct: only the confirmation for the
            // transfer `o_native_pending` is actually still tracking
            // gets to claim the Transaction lane.
            if phase == TestPhase::NativeTransferLoop {
                let is_current_pending = self
                    .state
                    .o_native_pending
                    .is_some_and(|p| p.sig == Some(signature));
                if is_current_pending {
                    self.record_native_read(UpdateLane::Transaction, inclusion_slot);
                }
            }
        }
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
                "perpfundingv1: epoch {} closed @ slot {} -- no funding spread edges (no symbol had data from both venues, or rates were equal)",
                layer.epoch_ts,
                layer.slot,
            );
            return;
        }
        for edge in &layer.edges {
            log_warn!(
                "perpfundingv1: epoch {} @ slot {}: {} long={:?} short={:?} spread_annualized={:.3}%",
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
                "perpfundingv1: spot price probe @ slot {}: {} router SOL/USD={:.4} pyth SOL/USD={:.4} (conf={:.4}) delta={:+.2}%",
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
                    "perpfundingv1: spot price probe @ slot {}: {} hop {}: dex={:?} pool={} ({:?}) {} -> {} amount_in={} amount_out={}",
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
                                "perpfundingv1: spot price probe @ slot {}: SOL->USDC {:.9} SOL -> {:.6} USDC (price={:.4} USD/SOL, {} hop{})",
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
                            log_warn!(
                                "perpfundingv1: spot price probe @ slot {}: SOL->USDC rejected -- exact quote invalidated pool {} (cooling down {} slots)",
                                self.state.last_slot,
                                failure.pool_id,
                                planner::POOL_COOLDOWN_SLOTS,
                            );
                        } else {
                            log_warn!(
                                "perpfundingv1: spot price probe @ slot {}: SOL->USDC rejected -- pool {} not ready yet (no cooldown)",
                                self.state.last_slot,
                                failure.pool_id,
                            );
                        }
                    }
                }
            }
            None => {
                log_warn!(
                    "perpfundingv1: spot price probe @ slot {}: no SOL->USDC route found",
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
                                "perpfundingv1: spot price probe @ slot {}: USDC->SOL {:.6} USDC -> {:.9} SOL (price={:.4} USD/SOL, {} hop{})",
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
                            log_warn!(
                                "perpfundingv1: spot price probe @ slot {}: USDC->SOL rejected -- exact quote invalidated pool {} (cooling down {} slots)",
                                self.state.last_slot,
                                failure.pool_id,
                                planner::POOL_COOLDOWN_SLOTS,
                            );
                        } else {
                            log_warn!(
                                "perpfundingv1: spot price probe @ slot {}: USDC->SOL rejected -- pool {} not ready yet (no cooldown)",
                                self.state.last_slot,
                                failure.pool_id,
                            );
                        }
                    }
                }
            }
            None => {
                log_warn!(
                    "perpfundingv1: spot price probe @ slot {}: no USDC->SOL route found",
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
                "perpfundingv1: spfa smoke test OK -- found expected synthetic negative cycle: path={:?} weight={}",
                cycle.path,
                cycle.total_log_weight,
            ),
            None => log_error!(
                "perpfundingv1: spfa smoke test FAILED -- no cycle found in a graph built with a guaranteed negative cycle (real wasm32 SIMD bug?)",
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
            log_warn!("perpfundingv1: bootstrap: phoenix trader_account PDA not known yet -- set_authority hasn't run");
            return;
        }
        let budget_usd = (FUNDING_CYCLE_MIN_MARGIN_USD / 2.0).min(self.current_usdc_value());
        if budget_usd <= 0.0 {
            log_warn!(
                "perpfundingv1: bootstrap: no spare USDC to fund the Phoenix trader account yet"
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
            "perpfundingv1: bootstrap: registering + funding Phoenix trader account (${:.2})",
            budget_usd,
        );
        let Some(phoenix) = self.state.o_phoenix.as_ref() else {
            return;
        };
        if let Err(e) = phoenix.register_trader(owner, self.wallet) {
            log_error!("perpfundingv1: bootstrap: phoenix register_trader failed: {e}");
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
            log_error!("perpfundingv1: bootstrap: ember deposit failed: {e}");
            return;
        }
        if let Err(e) = phoenix.deposit_funds(owner, phusd_ata, amount_raw, self.wallet) {
            log_error!("perpfundingv1: bootstrap: phoenix deposit_funds failed: {e}");
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
            log_warn!("perpfundingv1: bootstrap: Solend USDC reserve not observed yet");
            return;
        };
        let lending_market = usdc_reserve.lending_market;

        // `id=0`, named (not a bare literal) so this real, currently-live
        // Solend obligation address is easy to grep for -- must never
        // change, since a different id would derive a different, unfunded
        // account, silently orphaning any real position already open here.
        const SOLEND_OBLIGATION_ID: u8 = 0;
        log_warn!("perpfundingv1: bootstrap: registering Solend obligation");
        if let Err(e) = solend::create_obligation_account(owner, SOLEND_OBLIGATION_ID, self.wallet)
        {
            log_error!("perpfundingv1: bootstrap: solend create_obligation_account failed: {e}");
            return;
        }
        if let Err(e) =
            solend::init_obligation(owner, lending_market, SOLEND_OBLIGATION_ID, self.wallet)
        {
            log_error!("perpfundingv1: bootstrap: solend init_obligation failed: {e}");
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
            log_warn!("perpfundingv1: bootstrap: registering Kamino user metadata");
            if let Err(e) = kamino::init_user_metadata(owner, self.wallet) {
                log_error!("perpfundingv1: bootstrap: kamino init_user_metadata failed: {e}");
                return;
            }
        }
        log_warn!("perpfundingv1: bootstrap: registering Kamino obligation");
        if let Err(e) = kamino::init_obligation(owner, lending_market, 0, self.wallet) {
            log_error!("perpfundingv1: bootstrap: kamino init_obligation failed: {e}");
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

        log_warn!("perpfundingv1: bootstrap: registering marginfi account");
        if let Err(e) = marginfi::initialize_account_pda(group, owner, self.wallet) {
            log_error!("perpfundingv1: bootstrap: marginfi initialize_account_pda failed: {e}");
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
            log_error!("perpfundingv1: basis trade: kamino track_farm_user_state failed: {e}");
            return false;
        }
        if kamino_position.farm_user_state_registered(farm_user_state_id) {
            return true;
        }
        log_warn!(
            "perpfundingv1: basis trade: bootstrapping Kamino farm-user-state for reserve {}",
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
                "perpfundingv1: basis trade: kamino init_obligation_farms_for_reserve failed: {e}"
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
                "perpfundingv1: funding cycle: {} has no oracle price yet, skipping Phoenix leg",
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
            "perpfundingv1: funding cycle: opening Phoenix leg {} side={:?} num_base_lots={} (notional=${:.2})",
            symbol,
            side,
            num_base_lots,
            notional_usd,
        );
        if let Err(e) =
            phoenix.place_market_order(owner, asset_id, side, num_base_lots, 0, 0, self.wallet)
        {
            log_error!(
                "perpfundingv1: funding cycle: Phoenix leg {} failed: {}",
                symbol,
                e
            );
        }
    }

    /// Flattens the live Phoenix position for `symbol` to zero via an
    /// opposite-side market order -- same call shape as
    /// `phoenixperpsv1::check_margin_health`'s proven-live
    /// liquidation-avoidance close. No-op if nothing's open.
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
            "perpfundingv1: funding cycle: closing Phoenix leg {} side={:?} size={}",
            symbol,
            side,
            size,
        );
        if let Err(e) =
            phoenix.place_market_order(owner, asset_id, side, size, 0, client_order_id, self.wallet)
        {
            log_error!(
                "perpfundingv1: funding cycle: Phoenix close {} failed: {}",
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
            log_error!("perpfundingv1: basis trade: {} has no Solend oracle price yet, skipping deposit-hedge", symbol);
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
            "perpfundingv1: basis trade: opening Solend deposit-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw) {
            log_error!(
                "perpfundingv1: basis trade: deposit-hedge {} spot swap failed: {}",
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
                "perpfundingv1: basis trade: deposit-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: deposit-hedge {} refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: deposit-hedge {} solend deposit failed: {}",
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
                "perpfundingv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} borrow-hedge",
                collateral_usd,
                symbol,
                notional_usd,
            );
            if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
                log_error!(
                    "perpfundingv1: basis trade: borrow-hedge {} refresh_reserve failed: {}",
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
                    "perpfundingv1: basis trade: borrow-hedge {} refresh_obligation failed: {}",
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
                log_error!("perpfundingv1: basis trade: borrow-hedge {} USDC collateral deposit failed: {}", symbol, e);
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
            log_error!("perpfundingv1: basis trade: {} has no Solend oracle price yet, skipping borrow-hedge", symbol);
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
            "perpfundingv1: basis trade: opening Solend borrow-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: borrow-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        if let Err(e) = usdc_reserve.refresh_reserve(usdc_reserve_id, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: borrow-hedge {} USDC refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: borrow-hedge {} refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: borrow-hedge {} solend borrow failed: {}",
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
                "perpfundingv1: basis trade: borrow-hedge {} spot sell failed: {}",
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
            log_error!("perpfundingv1: basis trade: {} has no Kamino oracle price yet, skipping deposit-hedge", symbol);
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
            "perpfundingv1: basis trade: opening Kamino deposit-hedge {} (${:.2})",
            symbol,
            notional_usd,
        );
        if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_amount_raw) {
            log_error!(
                "perpfundingv1: basis trade: deposit-hedge {} kamino spot swap failed: {}",
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
                "perpfundingv1: basis trade: deposit-hedge {} kamino refresh_reserve failed: {}",
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
                "perpfundingv1: basis trade: deposit-hedge {} kamino refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: deposit-hedge {} kamino deposit failed: {}",
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
                "perpfundingv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} Kamino borrow-hedge",
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
                log_error!("perpfundingv1: basis trade: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
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
                log_error!("perpfundingv1: basis trade: borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
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
                log_error!("perpfundingv1: basis trade: borrow-hedge {} kamino USDC collateral deposit failed: {}", symbol, e);
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
            log_error!("perpfundingv1: basis trade: {} has no Kamino oracle price yet, skipping borrow-hedge", symbol);
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
            "perpfundingv1: basis trade: opening Kamino borrow-hedge {} (${:.2})",
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
                "perpfundingv1: basis trade: borrow-hedge {} kamino refresh_reserve failed: {}",
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
            log_error!("perpfundingv1: basis trade: borrow-hedge {} kamino USDC refresh_reserve failed: {}", symbol, e);
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
                "perpfundingv1: basis trade: borrow-hedge {} kamino refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: borrow-hedge {} kamino borrow failed: {}",
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
                "perpfundingv1: basis trade: borrow-hedge {} kamino spot sell failed: {}",
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
                "perpfundingv1: basis trade: depositing ${:.2} USDC collateral for {} ${:.2} marginfi borrow-hedge",
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
                log_error!("perpfundingv1: basis trade: borrow-hedge {} marginfi USDC collateral deposit failed: {}", symbol, e);
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
            log_error!("perpfundingv1: basis trade: {} has no marginfi oracle price yet, skipping borrow-hedge", symbol);
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
                    "perpfundingv1: basis trade: {} marginfi oracle stale ({}s old, max age {}s), skipping borrow-hedge",
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
            "perpfundingv1: basis trade: opening marginfi borrow-hedge {} (${:.2})",
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
                "perpfundingv1: basis trade: borrow-hedge {} marginfi borrow failed: {}",
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
                "perpfundingv1: basis trade: borrow-hedge {} marginfi spot sell failed: {}",
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
            "perpfundingv1: basis trade: closing Solend deposit-hedge {}",
            symbol
        );
        if let Err(e) = reserve.refresh_reserve(reserve_id, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: close deposit-hedge {} refresh_reserve failed: {}",
                symbol,
                e
            );
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: close deposit-hedge {} refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: close deposit-hedge {} solend withdraw failed: {}",
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
                "perpfundingv1: basis trade: close deposit-hedge {} spot sell failed: {}",
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
                "perpfundingv1: basis trade: closing Solend borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "perpfundingv1: basis trade: close borrow-hedge {} buy-back failed: {}",
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
                "perpfundingv1: basis trade: close borrow-hedge {} refresh_reserve failed: {}",
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
                    "perpfundingv1: basis trade: close borrow-hedge {} USDC refresh_reserve failed: {}",
                    symbol,
                    e
                );
                return;
            }
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!(
                "perpfundingv1: basis trade: close borrow-hedge {} refresh_obligation failed: {}",
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
                "perpfundingv1: basis trade: close borrow-hedge {} solend repay failed: {}",
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
            "perpfundingv1: basis trade: closing Kamino deposit-hedge {}",
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
            log_error!("perpfundingv1: basis trade: close deposit-hedge {} kamino refresh_reserve failed: {}", symbol, e);
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
            log_error!("perpfundingv1: basis trade: close deposit-hedge {} kamino refresh_obligation failed: {}", symbol, e);
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
                "perpfundingv1: basis trade: close deposit-hedge {} kamino withdraw failed: {}",
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
                "perpfundingv1: basis trade: close deposit-hedge {} kamino spot sell failed: {}",
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
                "perpfundingv1: basis trade: closing Kamino borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "perpfundingv1: basis trade: close borrow-hedge {} kamino buy-back failed: {}",
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
            log_error!("perpfundingv1: basis trade: close borrow-hedge {} kamino refresh_reserve failed: {}", symbol, e);
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
                    "perpfundingv1: basis trade: close borrow-hedge {} kamino USDC refresh_reserve failed: {}",
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
            log_error!("perpfundingv1: basis trade: close borrow-hedge {} kamino refresh_obligation failed: {}", symbol, e);
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
                "perpfundingv1: basis trade: close borrow-hedge {} kamino repay failed: {}",
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
                "perpfundingv1: basis trade: closing marginfi borrow-hedge {}",
                symbol
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, mint, usdc_needed_raw) {
                log_error!(
                    "perpfundingv1: basis trade: close borrow-hedge {} marginfi buy-back failed: {}",
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
                "perpfundingv1: basis trade: close borrow-hedge {} marginfi repay failed: {}",
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
            "perpfundingv1: basis trade: closing {} -- direction reversed or no longer profitable",
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
            "perpfundingv1: idle capital: depositing ${:.2} USDC at {:?} (supply_apy={:.3}%)",
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
            log_error!("perpfundingv1: idle capital: solend refresh_reserve failed: {e}");
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!("perpfundingv1: idle capital: solend refresh_obligation failed: {e}");
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
            log_error!("perpfundingv1: idle capital: solend USDC deposit failed: {e}");
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
            log_error!("perpfundingv1: idle capital: kamino refresh_reserve failed: {e}");
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
            log_error!("perpfundingv1: idle capital: kamino refresh_obligation failed: {e}");
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
            log_error!("perpfundingv1: idle capital: kamino USDC deposit failed: {e}");
        }
    }

    /// marginfi half of [`Self::deploy_idle_usdc`] -- same bootstrap-gate/
    /// deposit shape as the borrow-hedge USDC-collateral stage in
    /// `perpfundingv1::state`, minus its `has_usdc_collateral` skip. No
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
            log_error!("perpfundingv1: idle capital: marginfi USDC deposit failed: {e}");
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
                "perpfundingv1: basis trade: opening {} direction={:?} (phoenix_funding={:.3}% best_borrow_apy={:.3}%)",
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
    /// comment for the full state machine this drives through. Replaces
    /// `perpfundingv1::evaluate`'s Phoenix/epoch/basis-trade logic
    /// entirely: this mode never touches Phoenix or the funding-rate
    /// router, only Solend/Kamino/marginfi deposit+withdraw.
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
        // Real, live 2026-09-04: `o_dex`/`o_solend_position`/
        // `o_kamino_position`/`o_marginfi_position` are never populated
        // in this module -- see `on_load`'s own comment for why. Gating
        // on them unconditionally (the real module's original check)
        // would mean `NativeTransferLoop`, the only phase this module
        // ever legitimately reaches, could never dispatch at all.
        // Bypassed only for `TestProtocol::Native` -- if `TEST_PROTOCOL`
        // isn't set to `native` (misconfiguration, since this module is
        // Native-only), this still correctly refuses to run any
        // Solend/Kamino/Marginfi phase against permanently-uninitialized
        // state, same as the real module would for genuinely-missing
        // state.
        if self.state.o_target_protocol != Some(TestProtocol::Native)
            && (self.state.o_dex.is_none()
                || self.state.o_solend_position.is_none()
                || self.state.o_kamino_position.is_none()
                || self.state.o_marginfi_position.is_none())
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
            TestPhase::NativeTransferLoop => self.test_native_transfer_loop(),
            TestPhase::Done => {}
        }

        // Drains whatever this evaluate() call (or `on_message`'s Wallet
        // arm) built onto self.wallet and actually sends it -- same tail
        // arbv1::state::evaluate/perpfundingv1::evaluate both use.
        // Without this, queued instructions would sit on the wallet
        // forever and never reach the chain.
        while let Some((sig, data)) = self.wallet.assemble() {
            match transactionprocessor::send(sig.as_array(), data) {
                Ok(_) => {
                    log_warn!("testlatencylitev1: sent transaction {sig}");
                    // Tag this signature with the phase that sent it (the
                    // state machine is strictly linear, so "current phase"
                    // is unambiguous) and the send instant, so `mid_on_tx`
                    // can compute real send->confirm latency once this
                    // signature is observed on-chain -- see `m_sig`'s doc
                    // comment. A stale/overwritten entry (the same
                    // signature sent twice) can't happen: `Signature` is a
                    // hash of the fully-signed transaction bytes, which
                    // change on every attempt (fresh recent blockhash).
                    let phase = self.state.test_phase;
                    self.state.m_sig.insert(sig, (phase, Instant::now()));
                    // Stash this send's own signature onto the pending
                    // native transfer it belongs to, if any -- see
                    // `NativePending::sig`'s doc comment for why
                    // `mid_on_tx` needs this instead of just trusting any
                    // NativeTransferLoop-phase confirmation. `is_none()`
                    // guards against overwriting it with some *later*
                    // tick's unrelated signature; harmless either way
                    // since only one native transfer is ever queued/
                    // assembled per tick.
                    if phase == TestPhase::NativeTransferLoop {
                        if let Some(pending) = self.state.o_native_pending.as_mut() {
                            if pending.sig.is_none() {
                                pending.sig = Some(sig);
                            }
                        }
                    }
                }
                Err(e) => log_error!("testlatencylitev1: failed to send transaction {sig}: {e}"),
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
    /// Real wall-clock budget for `test_native_transfer_loop`'s bundler-
    /// tip-status wait (see `o_bundler_wait_started`) -- generous
    /// relative to the Go-side `bundler.RunTipBroadcaster`'s own real
    /// 15-second poll interval (`tipBroadcastInterval`), so a normal
    /// deployment always gets at least one full broadcast cycle to
    /// deliver real status before this gives up and sends without it.
    const BUNDLER_STATUS_WAIT_SECS: u64 = 20;
    /// Deliberately small -- this only needs to prove the code path
    /// works, not move meaningful capital. Matches the conservative end
    /// of what was manually tested for real this session (5 USDC).
    const TEST_AMOUNT_USD: f64 = 1.0;

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
    /// from the phase just finished. Also emits a one-line latency
    /// report for the phase being left -- see `TxLatencyStats`'s doc
    /// comment -- so every phase's send->confirm numbers surface exactly
    /// once, at the point there's nothing more left to add to them,
    /// rather than needing to be scraped out of the full per-tx log.
    fn test_advance(&mut self, next: TestPhase) {
        let finished = self.state.test_phase;
        let (n, p50_us, p99_us) = self
            .state
            .tx_latency
            .get(&finished)
            .map(|s| s.stats())
            .unwrap_or((0, 0, 0));
        log_warn!(
            "testlatencylitev1: {finished:?} complete -- {n} confirmed tx, p50={p50_us}µs p99={p99_us}µs -- advancing to {next:?}",
        );
        self.state.test_phase = next;
        self.state.test_last_action_slot = None;
    }

    /// Number of full deposit<->withdraw cycles to run per protocol before
    /// moving on -- a single pass (the old behavior) only ever produces
    /// 1-3 samples, nowhere near enough for a meaningful p50/p99. Applies
    /// only to the Deposit/Withdraw phase pairs, not Bootstrap (one-shot,
    /// account creation isn't repeatable) or Borrow/Repay (out of scope
    /// for this pass -- see the gitlab issue this closes).
    const CYCLE_TARGET: u32 = 100;
    /// Raw units (not USD) deliberately left behind by every Solend/Kamino
    /// withdraw in a cycle, so the obligation's deposited balance never
    /// hits exactly zero. Necessary because a full/`_AMOUNT_MAX` withdrawal
    /// is documented elsewhere in this file as closing the obligation
    /// account (live-verified against mainnet) -- but *what specifically*
    /// triggers that closure (the `_AMOUNT_MAX` sentinel itself, vs. any
    /// withdrawal that empties the balance to zero) isn't something this
    /// codebase has verified. Leaving a ~$0.000001 dust remainder sidesteps
    /// that ambiguity entirely rather than betting on one theory of it --
    /// **this still needs live validation on the first real run**: if the
    /// obligation closes anyway, `cycle_write_baseline_amount`-based
    /// detection below will simply stop seeing confirmations and the
    /// phase will stall against its cooldown, which is at least a safe,
    /// visible failure rather than a silent wrong number. Not needed for
    /// Marginfi -- its `MarginfiAccount` is documented as NOT closed by a
    /// full withdrawal (no separate `close` instruction is ever called).
    const CYCLE_DUST_RAW: u64 = 1;

    /// Records one write->low-latency-read latency sample for `phase` --
    /// see `State::cycle_write_sent_at`'s doc comment for what "read"
    /// means here (an `on_account` update making the write's effect
    /// visible, not just seeing the transaction's signature confirmed).
    /// No-op if no write is currently pending (`cycle_write_sent_at` is
    /// `None`) -- guards against a stray/duplicate `on_account` update
    /// double-counting a sample that was already recorded.
    fn record_cycle_read(&mut self, phase: TestPhase) {
        let Some(sent_at) = self.state.cycle_write_sent_at.take() else {
            return;
        };
        let elapsed = sent_at.elapsed();
        self.state
            .cycle_read_latency
            .entry(phase)
            .or_default()
            .record(elapsed);
    }

    /// Logs the accumulated `CYCLE_TARGET`-cycle report for one protocol's
    /// deposit/withdraw pair once both are done -- separate n/p50/p99 for
    /// the deposit-write->read and withdraw-write->read latency (not one
    /// pooled number), so a difference between the two is visible.
    fn report_cycle_stats(
        &self,
        protocol: &str,
        deposit_phase: TestPhase,
        withdraw_phase: TestPhase,
    ) {
        let (dn, dp50, dp99) = self
            .state
            .cycle_read_latency
            .get(&deposit_phase)
            .map(|s| s.stats())
            .unwrap_or((0, 0, 0));
        let (wn, wp50, wp99) = self
            .state
            .cycle_read_latency
            .get(&withdraw_phase)
            .map(|s| s.stats())
            .unwrap_or((0, 0, 0));
        log_warn!(
            "testlatencylitev1: {protocol} cycle report ({} cycles) -- \
             deposit write->read: n={dn} p50={dp50}µs p99={dp99}µs; \
             withdraw write->read: n={wn} p50={wp50}µs p99={wp99}µs",
            Self::CYCLE_TARGET,
        );
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
            log_warn!("testlatencylitev1: [1/16] USDC balance already covers both deposit tests");
            let next = self
                .state
                .o_target_protocol
                .map(TestProtocol::bootstrap_phase)
                .unwrap_or(TestPhase::BootstrapSolend);
            self.test_advance(next);
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
            log_warn!("testlatencylitev1: [1/16] swapping {wsol_balance_raw} lamports of already-wrapped SOL to USDC");
            if let Err(e) = self.execute_spot_leg(mint_sol, mint_usdc, wsol_balance_raw) {
                log_error!("testlatencylitev1: [1/16] swap to USDC failed: {e}");
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
                "testlatencylitev1: [1/16] insufficient native SOL to wrap ({sol_balance} lamports, need {needed}) -- \
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
            "testlatencylitev1: [1/16] wrapping {} lamports SOL and swapping to USDC",
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
            log_error!("testlatencylitev1: [1/16] swap to USDC failed: {e}");
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
            log_warn!("testlatencylitev1: [2/16] solend obligation confirmed registered on-chain");
            self.test_advance(TestPhase::DepositSolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [2/16] bootstrapping solend obligation");
        self.bootstrap_solend_obligation();
        self.test_mark_action();
    }

    /// Current deposited-collateral amount (raw units, 0 if no deposit or
    /// the reserve isn't tracked yet) for `usdc_reserve_id` -- shared by
    /// the deposit/withdraw cycle logic on both sides (baseline capture
    /// at write time, read-confirmed check on every subsequent call).
    fn solend_deposited_amount(&self, usdc_reserve_id: AccountId) -> u64 {
        self.state
            .o_solend_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .map(|d| d.deposited_amount)
            .unwrap_or(0)
    }

    /// Runs [`Self::CYCLE_TARGET`] real deposit<->withdraw cycles against
    /// Solend, each one a write (deposit)->read->write (withdraw)->read
    /// round trip -- see this module's doc comment and `State::cycle_count`
    /// for why a single pass isn't enough for a meaningful p50/p99.
    /// "Read confirmed" means the deposited amount moved past the
    /// baseline captured when this cycle's write was sent (not a plain
    /// zero/nonzero check -- see `State::cycle_write_baseline_amount`'s
    /// doc for why).
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
            log_warn!("testlatencylitev1: [3/16] solend USDC reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let deposited_amount = self.solend_deposited_amount(usdc_reserve_id);
        if self.state.cycle_write_sent_at.is_some()
            && deposited_amount > self.state.cycle_write_baseline_amount
        {
            self.record_cycle_read(TestPhase::DepositSolend);
            log_warn!(
                "testlatencylitev1: [3/16] solend USDC deposit confirmed on-chain (cycle {}/{})",
                self.state.cycle_count + 1,
                Self::CYCLE_TARGET,
            );
            self.test_advance(TestPhase::WithdrawSolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [3/16] depositing ${:.2} USDC into solend (cycle {}/{})",
            Self::TEST_AMOUNT_USD,
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.state.cycle_write_baseline_amount = deposited_amount;
        self.deploy_idle_usdc_solend(Self::TEST_AMOUNT_USD);
        self.state.cycle_write_sent_at = Some(Instant::now());
        self.test_mark_action();
    }

    /// Withdraw half of the Solend deposit<->withdraw cycle -- see
    /// [`Self::test_deposit_solend`]'s doc comment. Withdraws down to a
    /// [`Self::CYCLE_DUST_RAW`] remainder rather than the full balance
    /// (see that constant's doc for why), and is read-confirmed the same
    /// baseline-comparison way as the deposit half, just in the opposite
    /// direction. Once [`Self::CYCLE_TARGET`] cycles complete, logs the
    /// aggregated report and moves on to Kamino; otherwise loops back to
    /// `DepositSolend` for another cycle.
    fn test_withdraw_solend(&mut self) {
        let Some(usdc_reserve_id) = self.solend_usdc_reserve_id() else {
            return;
        };
        let deposited_amount = self.solend_deposited_amount(usdc_reserve_id);
        if self.state.cycle_write_sent_at.is_some()
            && deposited_amount < self.state.cycle_write_baseline_amount
        {
            self.record_cycle_read(TestPhase::WithdrawSolend);
            self.state.cycle_count += 1;
            log_warn!(
                "testlatencylitev1: [4/16] solend USDC withdraw confirmed on-chain (cycle {}/{})",
                self.state.cycle_count,
                Self::CYCLE_TARGET,
            );
            if self.state.cycle_count >= Self::CYCLE_TARGET {
                self.report_cycle_stats(
                    "solend",
                    TestPhase::DepositSolend,
                    TestPhase::WithdrawSolend,
                );
                self.state.cycle_count = 0;
                // A single-protocol run (`o_target_protocol` set) stops
                // here rather than cascading into Kamino -- see
                // `TestProtocol`'s doc comment.
                let next = if self.state.o_target_protocol.is_some() {
                    TestPhase::Done
                } else {
                    TestPhase::BootstrapKamino
                };
                self.test_advance(next);
            } else {
                self.test_advance(TestPhase::DepositSolend);
            }
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [4/16] withdrawing USDC from solend (cycle {}/{})",
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.state.cycle_write_baseline_amount = deposited_amount;
        let withdraw_amount = deposited_amount.saturating_sub(Self::CYCLE_DUST_RAW);
        self.test_withdraw_usdc_solend(withdraw_amount);
        self.state.cycle_write_sent_at = Some(Instant::now());
        self.test_mark_action();
    }

    fn test_withdraw_usdc_solend(&mut self, collateral_amount: u64) {
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
            log_error!("testlatencylitev1: solend withdraw refresh_reserve failed: {e}");
            return;
        }
        let refresh_reserves = self.solend_refresh_reserves();
        if let Err(e) = solend::refresh_obligation(obligation_id, &refresh_reserves, self.wallet) {
            log_error!("testlatencylitev1: solend withdraw refresh_obligation failed: {e}");
            return;
        }
        let deposit_reserves = self.solend_obligation_deposit_reserves();
        if let Err(e) = usdc_reserve.withdraw(
            usdc_reserve_id,
            obligation_id,
            collateral_amount,
            owner,
            usdc_ata,
            usdc_collateral_ata,
            &deposit_reserves,
            self.wallet,
        ) {
            log_error!("testlatencylitev1: solend withdraw failed: {e}");
        }
    }

    fn test_bootstrap_kamino(&mut self) {
        if self
            .state
            .o_kamino_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            log_warn!("testlatencylitev1: [5/16] kamino obligation confirmed registered on-chain");
            self.test_advance(TestPhase::DepositKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [5/16] bootstrapping kamino obligation");
        self.bootstrap_kamino_obligation();
        self.test_mark_action();
    }

    /// See [`Self::solend_deposited_amount`]'s doc comment -- identical
    /// role, Kamino side.
    fn kamino_deposited_amount(&self, usdc_reserve_id: AccountId) -> u64 {
        self.state
            .o_kamino_position
            .as_ref()
            .and_then(|s| s.obligation())
            .and_then(|ob| ob.deposit_for(usdc_reserve_id))
            .map(|d| d.deposited_amount)
            .unwrap_or(0)
    }

    /// See [`Self::test_deposit_solend`]'s doc comment -- identical shape,
    /// Kamino side.
    fn test_deposit_kamino(&mut self) {
        let Some(usdc_reserve_id) = self.kamino_usdc_reserve_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testlatencylitev1: [6/16] kamino USDC reserve not observed yet");
            self.test_mark_action(); // see test_swap_to_usdc's doc comment for why this matters
            return;
        };
        let deposited_amount = self.kamino_deposited_amount(usdc_reserve_id);
        if self.state.cycle_write_sent_at.is_some()
            && deposited_amount > self.state.cycle_write_baseline_amount
        {
            self.record_cycle_read(TestPhase::DepositKamino);
            log_warn!(
                "testlatencylitev1: [6/16] kamino USDC deposit confirmed on-chain (cycle {}/{})",
                self.state.cycle_count + 1,
                Self::CYCLE_TARGET,
            );
            self.test_advance(TestPhase::WithdrawKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [6/16] depositing ${:.2} USDC into kamino (cycle {}/{})",
            Self::TEST_AMOUNT_USD,
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.state.cycle_write_baseline_amount = deposited_amount;
        self.deploy_idle_usdc_kamino(Self::TEST_AMOUNT_USD);
        self.state.cycle_write_sent_at = Some(Instant::now());
        self.test_mark_action();
    }

    /// See [`Self::test_withdraw_solend`]'s doc comment -- identical
    /// shape, Kamino side.
    fn test_withdraw_kamino(&mut self) {
        let Some(usdc_reserve_id) = self.kamino_usdc_reserve_id() else {
            return;
        };
        let deposited_amount = self.kamino_deposited_amount(usdc_reserve_id);
        if self.state.cycle_write_sent_at.is_some()
            && deposited_amount < self.state.cycle_write_baseline_amount
        {
            self.record_cycle_read(TestPhase::WithdrawKamino);
            self.state.cycle_count += 1;
            log_warn!(
                "testlatencylitev1: [7/16] kamino USDC withdraw confirmed on-chain (cycle {}/{})",
                self.state.cycle_count,
                Self::CYCLE_TARGET,
            );
            if self.state.cycle_count >= Self::CYCLE_TARGET {
                self.report_cycle_stats(
                    "kamino",
                    TestPhase::DepositKamino,
                    TestPhase::WithdrawKamino,
                );
                self.state.cycle_count = 0;
                // See test_withdraw_solend's identical single-protocol-run
                // check.
                let next = if self.state.o_target_protocol.is_some() {
                    TestPhase::Done
                } else {
                    TestPhase::BootstrapMarginfi
                };
                self.test_advance(next);
            } else {
                self.test_advance(TestPhase::DepositKamino);
            }
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [7/16] withdrawing USDC from kamino (cycle {}/{})",
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.state.cycle_write_baseline_amount = deposited_amount;
        let withdraw_amount = deposited_amount.saturating_sub(Self::CYCLE_DUST_RAW);
        self.test_withdraw_usdc_kamino(withdraw_amount);
        self.state.cycle_write_sent_at = Some(Instant::now());
        self.test_mark_action();
    }

    fn test_withdraw_usdc_kamino(&mut self, collateral_amount: u64) {
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
            log_error!("testlatencylitev1: kamino withdraw refresh_reserve failed: {e}");
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
            log_error!("testlatencylitev1: kamino withdraw refresh_obligation failed: {e}");
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
            collateral_amount,
            owner,
            usdc_ata,
            self.wallet,
        ) {
            log_error!("testlatencylitev1: kamino withdraw failed: {e}");
        }
        // Unlike the old full/`KAMINO_AMOUNT_MAX` withdrawal this replaced,
        // this leaves `CYCLE_DUST_RAW` behind on purpose (see that
        // constant's doc comment) specifically so the obligation does NOT
        // close -- so, unlike that old code, this must NOT call
        // `mark_obligation_closing()`. Doing so would zero out the tracked
        // `o_obligation` immediately, making `kamino_deposited_amount`
        // read 0 regardless of real on-chain state and falsely "confirm"
        // the withdraw before it actually lands -- corrupting the very
        // latency number this cycle exists to measure.
    }

    fn test_bootstrap_marginfi(&mut self) {
        if self
            .state
            .o_marginfi_position
            .as_ref()
            .is_some_and(|s| s.registered())
        {
            log_warn!("testlatencylitev1: [8/16] marginfi account confirmed registered on-chain");
            self.test_advance(TestPhase::DepositMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [8/16] bootstrapping marginfi account");
        self.bootstrap_marginfi_account();
        self.test_mark_action();
    }

    /// See [`Self::test_deposit_solend`]'s doc comment for the general
    /// cycle shape. Marginfi is simpler than Solend/Kamino here: its
    /// `MarginfiAccount` is documented as NOT closed by a full withdrawal
    /// (no explicit `close` instruction is ever called against it), so
    /// there's no dust/baseline dance needed -- a plain "is there a
    /// deposit at all" check is already unambiguous across repeated
    /// cycles, same as the original single-pass version used.
    fn test_deposit_marginfi(&mut self) {
        let Some(usdc_bank_id) = self.marginfi_usdc_bank_id() else {
            // See test_deposit_solend's identical branch for why this
            // cooldown check has to come before the log line, not just
            // test_mark_action after it.
            if self.test_cooldown_active() {
                return;
            }
            log_warn!("testlatencylitev1: [9/16] marginfi USDC bank not observed yet");
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
        if self.state.cycle_write_sent_at.is_some() && has_deposit {
            self.record_cycle_read(TestPhase::DepositMarginfi);
            log_warn!(
                "testlatencylitev1: [9/16] marginfi USDC deposit confirmed on-chain (cycle {}/{})",
                self.state.cycle_count + 1,
                Self::CYCLE_TARGET,
            );
            self.test_advance(TestPhase::WithdrawMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [9/16] depositing ${:.2} USDC into marginfi (cycle {}/{})",
            Self::TEST_AMOUNT_USD,
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.deploy_idle_usdc_marginfi(Self::TEST_AMOUNT_USD);
        self.state.cycle_write_sent_at = Some(Instant::now());
        self.test_mark_action();
    }

    /// See [`Self::test_withdraw_solend`]'s doc comment for the general
    /// cycle shape -- no dust remainder needed here (see
    /// [`Self::test_deposit_marginfi`]'s doc comment), so this withdraws
    /// the full deposit every cycle, same as the original single-pass
    /// version.
    fn test_withdraw_marginfi(&mut self) {
        let Some(usdc_bank_id) = self.marginfi_usdc_bank_id() else {
            return;
        };
        let has_deposit = self
            .state
            .o_marginfi_position
            .as_ref()
            .and_then(|s| s.lending_account())
            .and_then(|la| la.deposit_for(usdc_bank_id))
            .is_some();
        if self.state.cycle_write_sent_at.is_some() && !has_deposit {
            self.record_cycle_read(TestPhase::WithdrawMarginfi);
            self.state.cycle_count += 1;
            log_warn!(
                "testlatencylitev1: [10/16] marginfi USDC withdraw confirmed on-chain (cycle {}/{})",
                self.state.cycle_count,
                Self::CYCLE_TARGET,
            );
            if self.state.cycle_count >= Self::CYCLE_TARGET {
                self.report_cycle_stats(
                    "marginfi",
                    TestPhase::DepositMarginfi,
                    TestPhase::WithdrawMarginfi,
                );
                self.state.cycle_count = 0;
                // See test_withdraw_solend's identical single-protocol-run
                // check. Marginfi is last in the deposit/withdraw chain,
                // so the unset/full-sequence case still falls through into
                // the (out-of-scope-for-cycling, single-pass) borrow/repay
                // phases exactly as before.
                let next = if self.state.o_target_protocol.is_some() {
                    TestPhase::Done
                } else {
                    TestPhase::BorrowSolend
                };
                self.test_advance(next);
            } else {
                self.test_advance(TestPhase::DepositMarginfi);
            }
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [10/16] withdrawing USDC from marginfi (cycle {}/{})",
            self.state.cycle_count + 1,
            Self::CYCLE_TARGET,
        );
        self.test_withdraw_usdc_marginfi();
        self.state.cycle_write_sent_at = Some(Instant::now());
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
            log_error!("testlatencylitev1: marginfi withdraw failed: {e}");
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
            log_warn!("testlatencylitev1: [11/16] solend SOL reserve not observed yet");
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
            log_warn!("testlatencylitev1: [11/16] solend SOL borrow confirmed on-chain");
            self.test_advance(TestPhase::RepaySolend);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [11/16] opening ${:.2} SOL borrow-hedge on solend",
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
            log_warn!("testlatencylitev1: [12/16] solend SOL repay confirmed on-chain");
            self.test_advance(TestPhase::BorrowKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [12/16] repaying SOL borrow on solend");
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
            log_warn!("testlatencylitev1: [13/16] kamino SOL reserve not observed yet");
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
            log_warn!("testlatencylitev1: [13/16] kamino SOL borrow confirmed on-chain");
            self.test_advance(TestPhase::RepayKamino);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!(
            "testlatencylitev1: [13/16] opening ${:.2} SOL borrow-hedge on kamino",
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
            log_warn!("testlatencylitev1: [14/16] kamino SOL repay confirmed on-chain");
            self.test_advance(TestPhase::BorrowMarginfi);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [14/16] repaying SOL borrow on kamino");
        self.close_kamino_borrow_leg("SOL");
        self.test_mark_action();
    }

    /// [15/16] marginfi counterpart of [`Self::test_borrow_solend`], via
    /// [`Self::open_marginfi_borrow_leg`].
    ///
    /// TEMPORARILY SKIPPED (test scope only -- `open_marginfi_borrow_leg`
    /// itself, still exercised by production `perpfundingv1`, keeps its
    /// full real staleness gate). The gate is verified correct: marginfi's
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
        log_warn!("testlatencylitev1: [15/16] skipping marginfi SOL borrow-hedge (depends on an external Switchboard crank, see doc comment above)");
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
                "testlatencylitev1: [16/16] marginfi SOL repay confirmed on-chain -- test complete (verify via real \
                 on-chain state, not this state machine)"
            );
            self.test_advance(TestPhase::Done);
            return;
        }
        if self.test_cooldown_active() {
            return;
        }
        log_warn!("testlatencylitev1: [16/16] repaying SOL borrow on marginfi");
        self.close_marginfi_borrow_leg("SOL");
        self.test_mark_action();
    }

    /// Lamports moved on every native-transfer round trip after the
    /// first. Deliberately small, same reasoning as `TEST_AMOUNT_USD`
    /// elsewhere in this file -- this only needs to prove the round trip
    /// works and produce a measurable balance delta, not move meaningful
    /// value.
    const NATIVE_TRANSFER_LAMPORTS: u64 = 1_000_000;
    /// Extra lamports wallet 2 gets on top of `NATIVE_TRANSFER_LAMPORTS`
    /// in its one-time initial funding transfer (transfer #1), kept
    /// permanently (never sent back) as a fee cushion for the ~50
    /// transfers wallet 2 will send as the *sender* over the full 100-
    /// transfer run. 50 transfers x a real Solana base fee (5,000
    /// lamports) is 250,000 lamports; this is a generous multiple of
    /// that, not a tight budget.
    const NATIVE_WALLET2_FEE_BUFFER_LAMPORTS: u64 = 2_000_000;
    /// Real Solana base fee, lamports per required signature on a
    /// transaction -- same value `NATIVE_WALLET2_FEE_BUFFER_LAMPORTS`'s
    /// own doc comment already cites. Used to correct
    /// `NativePending::expected_lamports` when `owner` (this wallet's
    /// permanent fee payer, see that field's doc comment) is the
    /// transfer's recipient: `owner` also pays this fee on that same
    /// transaction, so its real balance gain is short of the raw transfer
    /// amount by `signature_count * SOLANA_BASE_FEE_LAMPORTS`.
    const SOLANA_BASE_FEE_LAMPORTS: u64 = 5_000;
    /// Total transfers to run (see this module's doc comment / the
    /// gitlab issue this closes) -- the *first* of these is wallet 2's
    /// own funding transfer, not a separate bootstrap step.
    ///
    /// Temporarily lowered 2026-09-04 (was 100) for a quick real-run
    /// check of the same-slot write/read fix + tip-aware balance check,
    /// without spending a full 100-transfer's worth of real Astralane
    /// tips on it. Restore to 100 for the real full run.
    const NATIVE_TRANSFER_TARGET: u32 = 20;

    /// Lazily derives and registers the second native-transfer wallet the
    /// first time it's needed. Deterministic (HKDF-SHA256 over the
    /// primary child wallet's own secret seed, domain-separated by a
    /// fixed info string), not random -- so it needs no host-side
    /// coordination or persistence: recomputing it from the same child
    /// key always yields the same keypair, same reasoning
    /// `contrib/derive-child-key` relies on for the parent->child
    /// derivation on the Go side (see that tool's doc comment). Returns
    /// the registered `AccountId`, or `None` if the primary wallet keypair
    /// hasn't arrived from the Go host yet (`Configuration::set`'s
    /// `Wallet` message arm).
    fn ensure_native_second_wallet(&mut self) -> Option<AccountId> {
        if let Some(id) = self.state.o_second_wallet {
            return Some(id);
        }
        let primary = self.state.o_rc_keypair.as_ref()?;
        let seed = {
            let kp = rc_unlock(&primary.rc_keypair);
            let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, kp.secret_bytes());
            let mut seed = [0u8; 32];
            hk.expand(b"testlatencylitev1-native-wallet2", &mut seed)
                .expect("32-byte HKDF expand output is always valid");
            seed
        };
        let keypair = Rc::new(UnsafeCell::new(Keypair::new_from_array(seed)));
        let id = self.wallet.append_key(keypair, self.graph).ok()?;
        self.state.o_second_wallet = Some(id);
        log_warn!(
            "testlatencylitev1: native transfer test -- derived second wallet {} for real SOL round trips",
            id
        );
        Some(id)
    }

    /// Queues one final transaction draining both native-transfer test
    /// wallets' remaining SOL back to the real mothership/parent wallet
    /// (the account this whole run was funded from) -- called exactly
    /// once, from `test_native_transfer_loop`'s `native_transfer_count >=
    /// NATIVE_TRANSFER_TARGET` branch (see `State::native_swept`'s doc
    /// comment for why this run previously just left both wallets funded
    /// forever instead).
    ///
    /// Two transfers in one transaction, in order: `wallet2 -> owner`
    /// (drains wallet2's leftover -- normally just
    /// `NATIVE_WALLET2_FEE_BUFFER_LAMPORTS`, the funding buffer from
    /// transfer 1 that an even `NATIVE_TRANSFER_TARGET` never sends back),
    /// then `owner -> mothership` for everything owner now has, less this
    /// transaction's own real fee. Both amounts are computed off-chain
    /// from this guest's own cached balances (`Wallet::balance_sol`), not
    /// derived on-chain -- correct as long as those caches are fresh,
    /// which they are here (nothing else touches either wallet between
    /// the 20th confirmed transfer and this call). Best-effort: no
    /// confirmation tracking, no retry -- if it fails to land, the same
    /// manual recovery this was added to avoid (deterministic re-
    /// derivation of `owner`'s key from the real parent key, see this
    /// field's own doc comment) is still available as a fallback.
    fn sweep_native_wallets(&mut self) {
        let Ok(mothership_str) = std::env::var(crate::message::ENV_MOTHERSHIP_PUBKEY) else {
            log_warn!(
                "testlatencylitev1: native transfer sweep skipped -- {} env var not set",
                crate::message::ENV_MOTHERSHIP_PUBKEY,
            );
            return;
        };
        let Ok(mothership_pk) = Pubkey::try_from(mothership_str.as_str()) else {
            log_warn!(
                "testlatencylitev1: native transfer sweep skipped -- couldn't parse {} ({mothership_str})",
                crate::message::ENV_MOTHERSHIP_PUBKEY,
            );
            return;
        };
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(owner_pk) = pubkey_from_account_id(&owner) else {
            return;
        };
        let owner_balance = self.wallet.balance_sol(&owner).unwrap_or(0);
        let wallet2 = self.state.o_second_wallet;
        let (wallet2_balance, wallet2_pk) = match wallet2 {
            Some(id) => (
                self.wallet.balance_sol(&id).unwrap_or(0),
                pubkey_from_account_id(&id),
            ),
            None => (0, None),
        };
        // 2 signatures (owner as fee payer + wallet2 as the first
        // instruction's own required signer) whenever wallet2 has
        // anything to sweep, else just 1 (owner alone). 10,000 CU total
        // (5,000 per transfer instruction) when both legs run, else
        // 5,000 for owner's alone -- same `priority_fee_lamports` formula
        // `owner_overhead` above uses, no Astralane tip (this sweep
        // doesn't need bundled speed).
        let sweep_wallet2 = wallet2_balance > 0 && wallet2_pk.is_some();
        let signature_count = if sweep_wallet2 { 2 } else { 1 };
        let transfer_compute = if sweep_wallet2 { 10_000 } else { 5_000 };
        let reserve = signature_count * Self::SOLANA_BASE_FEE_LAMPORTS
            + self.wallet.priority_fee_lamports(transfer_compute);
        let combined = owner_balance + wallet2_balance;
        if combined <= reserve {
            log_warn!(
                "testlatencylitev1: native transfer sweep skipped -- {} owner + {} wallet2 lamports isn't enough to cover this transaction's own {} lamport fee/priority-fee reserve",
                owner_balance,
                wallet2_balance,
                reserve,
            );
            return;
        }
        let sweep_amount = combined - reserve;
        if sweep_wallet2 {
            let wallet2_pk = wallet2_pk.expect("checked by sweep_wallet2 above");
            self.wallet.require_signer(
                wallet2.expect("sweep_wallet2 implies wallet2 is Some"),
            );
            self.wallet
                .append_ix(system_transfer(&wallet2_pk, &owner_pk, wallet2_balance), 5_000);
        }
        self.wallet.require_signer(owner);
        self.wallet.append_ix(
            system_transfer(&owner_pk, &mothership_pk, sweep_amount),
            5_000,
        );
        log_warn!(
            "testlatencylitev1: native transfer sweep queued -- {} lamports ({} owner + {} wallet2, less {} reserved for this tx's own fee) -> mothership {}",
            sweep_amount,
            owner_balance,
            wallet2_balance,
            reserve,
            mothership_pk,
        );
    }

    /// Records one native-transfer write->read sample under `lane` and
    /// hands back the elapsed time -- shared by all three detection
    /// points (`low_latency`, the `CommitHook::on_account` rooted path,
    /// and `mid_on_tx`'s `Event::Transaction` path), each passing the
    /// real on-chain slot their own update carries as `inclusion_slot`
    /// (an account's `header.slot`, or the confirmed transaction's own
    /// `Ok(slot)` result). Takes `state.o_native_pending` (see that
    /// field's doc comment for why `take()` is the right primitive here:
    /// only the first caller to observe the balance change gets a `Some`
    /// back, so a transfer can never be double-counted across lanes or
    /// advance the loop twice).
    ///
    /// Beyond the total send->observed latency (unchanged from before),
    /// this now also splits that total into write delay (send ->
    /// `inclusion_slot`, looked up against this guest's own real slot
    /// clock -- see `State::slot_clock`) and read delay (`inclusion_slot`
    /// -> observed) -- see [`NativeTransferSample`] for what each field
    /// means and why this split exists.
    fn record_native_read(
        &mut self,
        lane: UpdateLane,
        inclusion_slot: Slot,
    ) -> Option<std::time::Duration> {
        let pending = self.state.o_native_pending.take()?;
        let elapsed = pending.sent_at.elapsed();
        self.state
            .native_read_latency
            .entry(lane)
            .or_default()
            .record(elapsed);
        self.state.native_transfer_count += 1;

        let slots_until_inclusion = inclusion_slot.saturating_sub(pending.send_slot);
        // Anchored on `FirstShredReceived` -- the earliest evidence this
        // guest (a downstream observer, not the leader) ever gets that
        // `inclusion_slot`'s block exists at all. `write_delay` answers
        // "how long from send until the leader saw/included this
        // transfer" as best this guest can measure it -- a genuine
        // *lower* bound, since the leader must have already included it
        // by the time any shred reaches us. Only a real, *positive*
        // measured duration counts as resolved -- `Instant::duration_since`
        // silently saturates a same-or-earlier instant to `Duration::ZERO`
        // instead of signaling "unresolvable", which previously produced
        // a fake `write_delay = Some(0)` here (see
        // `NativeTransferSample::write_delay`'s doc comment for the real
        // user-caught bug this was). `None` here is a real "we can't
        // resolve this", not "it took zero time".
        let write_delay = self
            .instant_for_slot(inclusion_slot)
            .and_then(|t| (t > pending.sent_at).then(|| t.duration_since(pending.sent_at)));
        // Only computed (and only meaningful) when `write_delay` didn't
        // resolve at all -- see `NativeTransferSample::write_delay_upper_bound`'s
        // doc comment for what this actually bounds and why.
        let write_delay_upper_bound = if write_delay.is_none() {
            self.instant_after_slot(inclusion_slot)
                .map(|t| t.saturating_duration_since(pending.sent_at))
        } else {
            None
        };
        // Real, measured, purely-informational breakdown of what happens
        // between `FirstShredReceived` and this guest's own observation
        // -- none of these three are subtracted from `read_delay` (see
        // that field's doc comment); they're sub-stages of it, reported
        // here for transparency into what it's made of.
        let shred_to_completed = match (
            self.instant_for_slot(inclusion_slot),
            self.completed_instant_for_slot(inclusion_slot),
        ) {
            (Some(a), Some(b)) if b > a => Some(b.duration_since(a)),
            _ => None,
        };
        let completed_to_processed = match (
            self.completed_instant_for_slot(inclusion_slot),
            self.processed_instant_for_slot(inclusion_slot),
        ) {
            (Some(a), Some(b)) if b > a => Some(b.duration_since(a)),
            _ => None,
        };
        let processed_to_confirmed = match (
            self.processed_instant_for_slot(inclusion_slot),
            self.confirmed_instant_for_slot(inclusion_slot),
        ) {
            (Some(a), Some(b)) if b > a => Some(b.duration_since(a)),
            _ => None,
        };
        // Never derived as `total - 0` for an unresolved write delay --
        // `None` propagates straight through, so an unresolved write
        // delay always yields an unresolved read delay too. `checked_sub`
        // (not `saturating_sub`) -- a real sanity check: if a write delay
        // somehow exceeded `elapsed`, that's a genuine inconsistency
        // worth surfacing as `None`, not silently clamping to zero.
        let read_delay = write_delay.and_then(|w| elapsed.checked_sub(w));
        // Register this sample for `tx_index` backfill (see
        // `State::m_native_tx_index`'s doc comment for why this can't just
        // reuse `o_native_pending`/`is_current_pending`) -- before the push
        // below, so the recorded index (`native_samples.len()`) is exactly
        // where the new sample is about to land.
        if let Some(sig) = pending.sig {
            self.state
                .m_native_tx_index
                .insert(sig, self.state.native_samples.len());
        }
        self.state.native_samples.push(NativeTransferSample {
            send_slot: pending.send_slot,
            inclusion_slot,
            slots_until_inclusion,
            write_delay,
            write_delay_upper_bound,
            read_delay,
            total_latency: elapsed,
            lane,
            shred_to_completed,
            completed_to_processed,
            processed_to_confirmed,
            tx_index: None,
        });

        log_warn!(
            "testlatencylitev1: native transfer {}/{} confirmed via {:?} -- sig={} send_slot={} inclusion_slot={} slots={} write={} read={} total={}µs shred->completed={} completed->processed={} processed->confirmed={}",
            self.state.native_transfer_count,
            Self::NATIVE_TRANSFER_TARGET,
            lane,
            pending
                .sig
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            pending.send_slot,
            inclusion_slot,
            slots_until_inclusion,
            match (write_delay, write_delay_upper_bound) {
                (Some(d), _) => format!("{}µs", d.as_micros()),
                (None, Some(b)) => format!("unknown (<={}µs)", b.as_micros()),
                (None, None) => "unknown".to_string(),
            },
            read_delay
                .map(|d| format!("{}µs", d.as_micros()))
                .unwrap_or_else(|| "unknown".to_string()),
            elapsed.as_micros(),
            shred_to_completed
                .map(|d| format!("{}µs", d.as_micros()))
                .unwrap_or_else(|| "unknown".to_string()),
            completed_to_processed
                .map(|d| format!("{}µs", d.as_micros()))
                .unwrap_or_else(|| "unknown".to_string()),
            processed_to_confirmed
                .map(|d| format!("{}µs", d.as_micros()))
                .unwrap_or_else(|| "unknown".to_string()),
        );
        Some(elapsed)
    }

    /// Checks whether `account_id`'s new balance satisfies a pending
    /// native transfer's recipient, and if so, claims it under `lane`.
    /// Called from all three real update paths with that path's own
    /// `UpdateLane` tag and the slot the update itself carries (`slot` --
    /// see [`NativeTransferSample::inclusion_slot`]'s doc comment for why
    /// this is the real on-chain inclusion slot, not just a timestamp). A
    /// no-op (cheap `Option`/equality checks only) whenever there's no
    /// pending native transfer or `account_id` isn't its recipient, so
    /// this is safe to call unconditionally from the hot
    /// `low_latency`/`on_account` paths without gating on `test_phase`
    /// first.
    ///
    /// Also requires `slot >= pending.send_slot` -- necessary, not just
    /// defensive: real, live-verified 2026-09-02 that every single one of
    /// 20 `Commit`-lane "wins" in one run had an `inclusion_slot` *before*
    /// `send_slot`, and (cross-checked against the transaction's own
    /// independently-logged real landing time in `mid_on_tx`) every one
    /// of them claimed the transfer before it had actually landed
    /// on-chain. Cause: `Commit`'s rooted-tier snapshot is *always* a few
    /// dozen slots stale by construction (Solana needs ~32 confirmations
    /// to root a block) -- an old, already-superseded balance reading
    /// that happens to already clear `expected_lamports` (via oscillation
    /// or drift from unrelated transfers) gets credited as if it were a
    /// fresh confirmation of the transfer just sent. Requiring the
    /// update's own slot to be no earlier than the send is what actually
    /// rules that out; the amount check alone (below) can't.
    fn check_native_transfer_arrival(
        &mut self,
        account_id: AccountId,
        slot: Slot,
        lane: UpdateLane,
    ) {
        let Some(pending) = self.state.o_native_pending else {
            return;
        };
        let Some(watch) = pending.watches.iter().find(|w| w.account == account_id) else {
            return;
        };
        if slot < pending.send_slot {
            return;
        }
        let Some(now_lamports) = self.wallet.balance_sol(&account_id) else {
            return;
        };
        // Direction-aware exact match -- see `NativeWatch::delta_lamports`'s
        // doc comment for why this must be the *exact* threshold, not just
        // "moved in the right direction".
        let matched = if watch.delta_lamports >= 0 {
            now_lamports >= watch.baseline_lamports.saturating_add(watch.delta_lamports as u64)
        } else {
            now_lamports
                <= watch
                    .baseline_lamports
                    .saturating_sub(watch.delta_lamports.unsigned_abs())
        };
        if !matched {
            return;
        }
        self.record_native_read(lane, slot);
    }

    /// The real state machine for `TestProtocol::Native`: send, wait for
    /// a read via whichever real update channel wins the race, send the
    /// other way, repeat -- for `NATIVE_TRANSFER_TARGET` (100) real
    /// System Program transfers total, alternating direction between two
    /// wallets. Deliberately protocol-agnostic: no lending-protocol
    /// accounts, no USDC, no bootstrap step -- just `Wallet`'s own SOL-
    /// balance tracking (`Wallet::on_account`, already fed by both
    /// `low_latency` and the `CommitHook::on_account` rooted path in this
    /// module) and a plain `solana_system_interface::instruction::transfer`.
    ///
    /// Why the winning lane can't be predicted in advance: `evaluate()`
    /// (which is what would notice "the recipient's balance went up")
    /// runs after *every* event type, and both `Event::LowLatency` and
    /// `Event::Commit` feed the exact same `Wallet::on_account` update --
    /// whichever event happens to deliver the change first is the one
    /// `check_native_transfer_arrival` sees it through. `Event::Transaction`
    /// is a third, independent path (this module's existing
    /// `m_sig`/`tx_latency` signature-correlation, extended below to also
    /// claim `o_native_pending` when it wins). See `UpdateLane`'s own doc
    /// comment.
    fn test_native_transfer_loop(&mut self) {
        // A transfer is still in flight. Normally one of `low_latency`,
        // the commit hook, or `mid_on_tx` will claim `o_native_pending`
        // and this phase picks back up on the next `evaluate()` call
        // after that happens -- but if it never gets confirmed (dropped,
        // expired blockhash, or landed with an on-chain error, which
        // `mid_on_tx` currently skips silently rather than clearing
        // `o_native_pending`), nothing else will ever un-stick this
        // phase. So: same cooldown-gated resend the Solend/Kamino/
        // Marginfi deposit/withdraw phases already use for exactly this
        // reason -- wait `TEST_ACTION_COOLDOWN_SLOTS`, then fall through
        // and resend (recomputing the identical from/to/amount below,
        // since `native_transfer_count` only advances on confirmation).
        if self.state.o_native_pending.is_some() {
            if self.test_cooldown_active() {
                return;
            }
            log_warn!(
                "testlatencylitev1: native transfer {}/{} unconfirmed after {} slots -- retrying",
                self.state.native_transfer_count + 1,
                Self::NATIVE_TRANSFER_TARGET,
                Self::TEST_ACTION_COOLDOWN_SLOTS,
            );
        }
        if self.state.native_transfer_count >= Self::NATIVE_TRANSFER_TARGET {
            if !self.state.native_swept {
                self.sweep_native_wallets();
                self.state.native_swept = true;
            }
            self.report_native_stats();
            self.test_advance(TestPhase::Done);
            return;
        }
        // Wait for the Astralane bundler tip broadcaster's first real
        // status (up or down) before ever sending transfer 1 -- real,
        // live-verified 2026-09-04: this guest's very first tip update
        // attempt always fails Go-side ("bot not connected yet", an
        // inherent chicken/egg at boot -- see `Wallet::has_bundler_status`'s
        // doc comment), so if this phase sends transfer 1 fast enough, it
        // wins a race it has no reason to run: the transfer falls back to
        // a slower, non-bundled send purely because it looked before the
        // broadcaster's next scheduled push had a chance to land, not
        // because Astralane was actually unavailable. Only gates the
        // very first transfer -- once real status exists (or this wait
        // times out, for a deployment with no bundler configured at
        // all), every later send already sees whatever's current.
        //
        // A real wall-clock `Instant` budget (`BUNDLER_STATUS_WAIT_SECS`),
        // not the slot-based `test_cooldown_active`/`test_last_action_slot`
        // this used at first -- real, live bug 2026-09-04: `last_slot`
        // (rooted/Commit tier) is still `0` this early in a run, and the
        // moment the first real Commit event arrives and jumps it to a
        // real slot number in the hundreds of millions, a slot-based
        // cooldown gets blown through instantly, so that version never
        // actually waited at all. See `o_bundler_wait_started`'s own doc
        // comment.
        if self.state.native_transfer_count == 0
            && self.state.o_native_pending.is_none()
            && !self
                .wallet
                .has_bundler_status(Wallet::ASTRALANE_BUNDLER_CODE)
        {
            match self.state.o_bundler_wait_started {
                None => {
                    log_warn!(
                        "testlatencylitev1: native transfer test -- waiting up to {}s for the bundler tip broadcaster's first status before sending transfer 1/{}",
                        Self::BUNDLER_STATUS_WAIT_SECS,
                        Self::NATIVE_TRANSFER_TARGET,
                    );
                    self.state.o_bundler_wait_started = Some(std::time::Instant::now());
                    return;
                }
                Some(started)
                    if started.elapsed()
                        < std::time::Duration::from_secs(Self::BUNDLER_STATUS_WAIT_SECS) =>
                {
                    return;
                }
                Some(_) => {
                    log_warn!(
                        "testlatencylitev1: native transfer test -- gave up waiting on bundler tip status after {}s, sending transfer 1/{} without it",
                        Self::BUNDLER_STATUS_WAIT_SECS,
                        Self::NATIVE_TRANSFER_TARGET,
                    );
                }
            }
        }
        let Some(owner) = self.state.wallet() else {
            return;
        };
        let Some(wallet2) = self.ensure_native_second_wallet() else {
            return;
        };
        // Even count so far (0, 2, 4, ...) -> next send is wallet1->wallet2;
        // odd -> wallet2->wallet1. Transfer #1 (count still 0) is wallet
        // 2's own funding transfer, not a separate bootstrap step -- see
        // this module's doc comment.
        let (from, to) = if self.state.native_transfer_count % 2 == 0 {
            (owner, wallet2)
        } else {
            (wallet2, owner)
        };
        let amount = if self.state.native_transfer_count == 0 {
            Self::NATIVE_TRANSFER_LAMPORTS + Self::NATIVE_WALLET2_FEE_BUFFER_LAMPORTS
        } else {
            Self::NATIVE_TRANSFER_LAMPORTS
        };
        let (Some(from_pk), Some(to_pk)) =
            (pubkey_from_account_id(&from), pubkey_from_account_id(&to))
        else {
            return;
        };
        // `owner` is always this wallet's fee payer (`Wallet::assemble`'s
        // `self.payer`) and, once real Astralane tip data exists, also
        // pays a real tip on every transaction it sends -- both
        // regardless of whether it's `from` for *this* transfer.
        // Computed once, up front, so the funding-wait check below and
        // `expected_lamports` further down agree on the exact same real
        // overhead (and both actually reflect whether this send will use
        // Astralane, checked exactly once rather than risking two calls
        // to `select_tip_account` disagreeing within the same tick).
        // Signature count is 1 (just `owner`, as payer) when `owner` is
        // also the sender, else 2 (`owner` as payer + `from` as the
        // System Program instruction's own required signer).
        let use_astralane = self
            .wallet
            .select_tip_account(Wallet::ASTRALANE_BUNDLER_CODE)
            .is_some();
        let signature_count = if from == owner { 1 } else { 2 };
        // Real compute-unit total for the transaction this send actually
        // assembles -- one 5,000 CU transfer instruction plain, or that
        // plus a second 5,000 CU tip transfer when Astralane-bundled (see
        // `send_native_transfer_via_astralane`/`Wallet::append_bundler_tip`,
        // both of which use the same `Wallet::TRANSFER_CU` value). Needed
        // to compute the *real* priority fee below, not just the base fee.
        let transfer_compute = if use_astralane { 10_000 } else { 5_000 };
        // Real, live-verified 2026-09-03: this used to be just the flat
        // per-signature base fee, silently missing the priority fee
        // (`evaluate_inner` unconditionally sets `PriorityLevel::Medium`
        // before every phase runs) -- see `Wallet::priority_fee_lamports`'s
        // doc comment for the exact 50-lamport gap that left uncounted,
        // and why it made owner-recipient transfers structurally
        // unconfirmable via the Account/Commit lanes.
        let owner_overhead = signature_count * Self::SOLANA_BASE_FEE_LAMPORTS
            + self.wallet.priority_fee_lamports(transfer_compute)
            + if use_astralane {
                self.wallet
                    .tip_lamports(Wallet::ASTRALANE_BUNDLER_CODE)
                    .unwrap_or(0)
            } else {
                0
            };
        // Wait for `from` (and, when `from != owner`, `owner` too -- see
        // `owner_overhead` above) to actually have the funds before ever
        // broadcasting -- real, live-verified need (not just defensive):
        // this phase can start running before the Go host's own boot
        // transfer (parent -> wallet 1) has landed, since that depends on
        // a separate subscription this guest has no visibility into. A
        // fixed startup delay can't be sized correctly (that subscription
        // has no fixed upper bound), so this waits on the actual
        // precondition instead: exactly as long as it takes, and no
        // wasted broadcasts that can't even pay their own way. Real,
        // live-confirmed 2026-09-03: before `owner_overhead` accounted
        // for the Astralane tip too (originally just the network fee),
        // `owner` repeatedly sent a transfer it couldn't actually afford
        // once the tip was added -- landing on-chain and failing with a
        // real `custom program error: 0x1` (System Program "insufficient
        // lamports") four times in a row, each attempt burning a real
        // network fee, before this same underlying balance-vs-required
        // gap started throttling it (by coincidence, not by design).
        let from_balance = self.wallet.balance_sol(&from).unwrap_or(0);
        let from_required = amount + if from == owner { owner_overhead } else { 0 };
        if from_balance < from_required {
            if self.test_cooldown_active() {
                return;
            }
            log_warn!(
                "testlatencylitev1: native transfer {}/{} -- waiting for {} to be funded ({} of {} lamports needed)",
                self.state.native_transfer_count + 1,
                Self::NATIVE_TRANSFER_TARGET,
                from,
                from_balance,
                from_required,
            );
            self.test_mark_action();
            return;
        }
        if from != owner {
            let owner_balance = self.wallet.balance_sol(&owner).unwrap_or(0);
            if owner_balance < owner_overhead {
                if self.test_cooldown_active() {
                    return;
                }
                log_warn!(
                    "testlatencylitev1: native transfer {}/{} -- waiting for {} (fee payer) to cover its own fee/tip overhead ({} of {} lamports needed)",
                    self.state.native_transfer_count + 1,
                    Self::NATIVE_TRANSFER_TARGET,
                    owner,
                    owner_balance,
                    owner_overhead,
                );
                self.test_mark_action();
                return;
            }
        }
        let baseline_lamports = self.wallet.balance_sol(&to).unwrap_or(0);
        log_warn!(
            "testlatencylitev1: native transfer {}/{} -- sending {} lamports {} -> {}",
            self.state.native_transfer_count + 1,
            Self::NATIVE_TRANSFER_TARGET,
            amount,
            from,
            to,
        );
        self.wallet.require_signer(from);
        // Real Astralane bundled send, only ever attempted once live tip
        // data actually existed at the `use_astralane` check above --
        // see `send_native_transfer_via_astralane`'s own doc comment for
        // why that's a read-only check rather than just trying
        // `append_bundler_tip` and rolling back on failure. Falls back
        // to the plain unbundled send below on any real send failure
        // (not just missing tip data) -- a native transfer always goes
        // out exactly once per tick either way, so this can't turn into
        // an unthrottled retry loop the way silently skipping the send
        // entirely would.
        let astralane_sig = if use_astralane {
            match self.send_native_transfer_via_astralane(owner, &from_pk, &to_pk, amount) {
                Ok(signature) => Some(signature),
                Err(e) => {
                    log_error!(
                        "testlatencylitev1: native transfer {}/{} -- astralane bundled send failed, falling back to plain send: {e}",
                        self.state.native_transfer_count + 1,
                        Self::NATIVE_TRANSFER_TARGET,
                    );
                    None
                }
            }
        } else {
            None
        };
        if astralane_sig.is_none() {
            self.wallet
                .append_ix(system_transfer(&from_pk, &to_pk, amount), 5_000);
        }
        // `owner` is always this wallet's fee payer (`Wallet::assemble`'s
        // `self.payer`), on *both* legs of this transfer regardless of
        // which one it is -- so it's the only side that ever needs
        // `owner_overhead` netted out. `wallet2` (whichever leg isn't
        // `owner`) never pays a fee either way, so its delta is always
        // the raw `amount`, exactly. See [`NativeWatch`]'s doc comment for
        // why both legs are watched, not just `to`.
        let from_delta = -(amount as i64) - if from == owner { owner_overhead as i64 } else { 0 };
        let to_delta = amount as i64 - if to == owner { owner_overhead as i64 } else { 0 };
        // Fresh `sent_at` on every (re)send, including retries -- this
        // measures the latency of whichever attempt actually gets
        // confirmed, not the doomed one(s) before it. Same convention
        // `test_deposit_solend`/friends use for `cycle_write_sent_at` on
        // their own cooldown-gated resend.
        //
        // Diagnostic for `current_slot()`'s own staleness fix
        // (`State::freshest_account_slot`'s doc comment): logs both raw
        // signals separately, right before they're combined into the
        // `send_slot` actually used below, so a real run can show exactly
        // how far the old `SlotStatus`-only view (`from_slot_status`) had
        // fallen behind the newer, higher-frequency account-update signal
        // (`from_account`) at this exact send -- not just infer it from
        // whether `slots_until_inclusion` looks more plausible afterward.
        let from_slot_status = self.state.slot_clock.back().map_or(0, |&(s, _)| s);
        let from_account = self.state.freshest_account_slot;
        log_warn!(
            "testlatencylitev1: native transfer test -- send_slot sources: slot_status={} freshest_account={} gap={}",
            from_slot_status,
            from_account,
            from_account.saturating_sub(from_slot_status),
        );
        self.state.o_native_pending = Some(NativePending {
            sent_at: Instant::now(),
            watches: [
                NativeWatch {
                    account: from,
                    baseline_lamports: from_balance,
                    delta_lamports: from_delta,
                },
                NativeWatch {
                    account: to,
                    baseline_lamports,
                    delta_lamports: to_delta,
                },
            ],
            // `Some` already when `send_native_transfer_via_astralane`
            // just sent it directly above; `None` for the plain path --
            // filled in moments later this same tick, once
            // `evaluate_inner`'s own tail send loop actually
            // assembles/signs the instruction queued above -- see that
            // call site.
            sig: astralane_sig,
            send_slot: self.current_slot(),
        });
        self.test_mark_action();
    }

    /// Sends this transfer's own `system_transfer` as a real, individually
    /// tipped Astralane bundle (`Wallet::append_bundler_tip` +
    /// `Wallet::send_transaction_batch`) instead of letting
    /// `evaluate_inner`'s own tail drain loop send it plain via
    /// `transactionprocessor::send`. Mirrors
    /// `multimodelv1::state::send_single_hop_as_astralane_tx`'s real,
    /// live-confirmed pattern: tip + transfer built as one atomic group,
    /// assembled and sent right here rather than left for the generic
    /// queue drain, since Astralane requires a tip on every transaction
    /// it routes -- an ordinary queue drain elsewhere in this same tick
    /// could otherwise split the tip from the transfer across separate
    /// transactions.
    ///
    /// Callers must already have confirmed real tip data exists (see
    /// `Wallet::select_tip_account`) -- this only exists once the Go
    /// host has pushed at least one `CommonBundlerTipUpdate` (see
    /// `on_message`'s own arm for that), which needs no explicit
    /// subscription on this module's part beyond handling the message.
    /// A tip-append failure here despite that prior check is treated as
    /// a real error (a live race -- the tip data went down between the
    /// check and now), not silently retried.
    ///
    /// Still records into `self.state.m_sig` itself (`evaluate_inner`'s
    /// tail loop does this for the plain path) so `mid_on_tx`'s existing
    /// per-phase `tx_latency` stats keep covering this transaction the
    /// same as any other.
    fn send_native_transfer_via_astralane(
        &mut self,
        owner: AccountId,
        from_pk: &Pubkey,
        to_pk: &Pubkey,
        amount: u64,
    ) -> Result<Signature, String> {
        let checkpoint = self.wallet.queue_checkpoint();
        self.wallet.begin_atomic_group();
        if !self
            .wallet
            .append_bundler_tip(owner, Wallet::ASTRALANE_BUNDLER_CODE)
        {
            self.wallet.rollback_to(checkpoint);
            self.wallet.end_atomic_group();
            return Err(
                "append_bundler_tip declined despite select_tip_account succeeding moments earlier"
                    .to_string(),
            );
        }
        self.wallet
            .append_ix(system_transfer(from_pk, to_pk, amount), 5_000);
        self.wallet.end_atomic_group();
        if !self.wallet.atomic_group_fits(checkpoint) {
            self.wallet.rollback_to(checkpoint);
            return Err("tip + transfer atomic group too large for one transaction".to_string());
        }
        let Some((signature, tx_bytes)) = self.wallet.assemble() else {
            return Err("failed to assemble tip + transfer transaction".to_string());
        };
        let tx_bytes = tx_bytes.to_vec();
        self.wallet
            .send_transaction_batch(&[tx_bytes], Wallet::ASTRALANE_BUNDLER_CODE)
            .map_err(|e| format!("{e:?}"))?;
        log_warn!("testlatencylitev1: sent transaction {signature} via Astralane bundle");
        self.state
            .m_sig
            .insert(signature, (self.state.test_phase, Instant::now()));
        Ok(signature)
    }

    /// Real per-transaction position-within-block estimate, refining
    /// `write_delay` beyond its `FirstShredReceived`-anchored lower bound
    /// using the real Agave `tx.index` (this transfer's own ordinal
    /// position in its landing block) and `SlotTimestamps::max_tx_index`
    /// (the largest ordinal position observed for that same slot -- a
    /// real, honest lower bound on the block's true transaction count,
    /// see that field's doc comment). `fraction = tx_index / slot_max`
    /// estimates how far into the block's ordering this transfer landed;
    /// multiplying that against the real measured `shred_to_completed +
    /// completed_to_processed` window (`FirstShredReceived` ->
    /// `Processed`, i.e. this validator's own block-delivery-plus-replay
    /// span) places the transfer's estimated real execution instant
    /// somewhere inside that window, instead of only knowing it happened
    /// somewhere between the two endpoints.
    ///
    /// An estimate, not a measurement: Sealevel can execute
    /// non-conflicting transactions within a block in parallel, not
    /// strictly in `tx.index` order, so "later index -> later execution"
    /// is a reasonable approximation, not a guarantee. Always report this
    /// clearly labeled as an estimate, never merged into `write_delay`
    /// itself (same discipline as `write_delay_upper_bound` already
    /// follows).
    ///
    /// Called per-sample from `report_native_stats` -- both to build the
    /// aggregate percentiles *and* to log each sample's own individual
    /// result (real, live-user-caught gap 2026-09-08: the first version of
    /// this only ever logged an aggregate percentile line, with no way to
    /// tell which specific samples resolved or why one didn't -- exactly
    /// the kind of per-transaction detail the decomposition report's table
    /// is supposed to carry). `Err` names *which* required input was
    /// missing, instead of a bare `None`, so the per-sample log line can
    /// say why.
    fn tx_index_estimate(&self, sample: &NativeTransferSample) -> Result<TxIndexEstimate, &'static str> {
        let write_delay = sample
            .write_delay
            .ok_or("same-slot sample, no resolvable write/read split at all")?;
        let tx_index = sample.tx_index.ok_or(
            "tx.index never backfilled -- this signature's own Transaction-lane data never arrived",
        )?;
        let slot_max = self
            .slot_max_tx_index(sample.inclusion_slot)
            .filter(|&m| m > 0)
            .ok_or("no resolvable per-slot max tx.index for this slot")?;
        let shred_to_completed = sample
            .shred_to_completed
            .ok_or("missing shred->completed timestamp for this slot")?;
        let completed_to_processed = sample
            .completed_to_processed
            .ok_or("missing completed->processed timestamp for this slot")?;
        let fraction = (tx_index as f64 / slot_max as f64).clamp(0.0, 1.0);
        let window = shred_to_completed + completed_to_processed;
        Ok(TxIndexEstimate {
            tx_index,
            slot_max_tx_index: slot_max,
            write_delay_estimate: write_delay + window.mul_f64(fraction),
        })
    }

    /// Logs the final report once all `NATIVE_TRANSFER_TARGET` transfers
    /// complete: per-lane total latency (same n/p50/p99 shape as
    /// `report_cycle_stats`, keyed by [`UpdateLane`] instead of
    /// protocol/phase, since the whole point of this test is comparing
    /// latency *across* update channels, not across phases), plus the
    /// write/slots/read breakdown across all `State::native_samples` --
    /// see [`NativeTransferSample`] for what write delay and read delay
    /// mean and why they're reported separately from the total. This is
    /// the number that actually answers "is a slow write→read cycle the
    /// transaction being slow to land, or the read path being slow to
    /// notice it once it has."
    fn report_native_stats(&self) {
        for lane in [
            UpdateLane::LowLatency,
            UpdateLane::Commit,
            UpdateLane::Transaction,
        ] {
            let (n, p50, p99) = self
                .state
                .native_read_latency
                .get(&lane)
                .map(|s| s.stats())
                .unwrap_or((0, 0, 0));
            log_warn!(
                "testlatencylitev1: native transfer report -- {:?}: n={n} p50={p50}µs p99={p99}µs",
                lane,
            );
        }
        let write_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| s.write_delay)
            .map(|d| d.as_micros() as u64)
            .collect();
        let read_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| s.read_delay)
            .map(|d| d.as_micros() as u64)
            .collect();
        let slots: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .map(|s| s.slots_until_inclusion)
            .collect();
        let unresolved: Vec<&NativeTransferSample> = self
            .state
            .native_samples
            .iter()
            .filter(|s| s.write_delay.is_none())
            .collect();
        // Real, honest upper bounds -- see
        // `NativeTransferSample::write_delay_upper_bound`'s doc comment.
        // Deliberately reported on their own, never merged into `write_us`
        // above: a bound is not a point estimate, and blending the two
        // would silently reintroduce the same kind of misattribution this
        // whole split exists to avoid.
        let write_bound_us: Vec<u64> = unresolved
            .iter()
            .filter_map(|s| s.write_delay_upper_bound)
            .map(|d| d.as_micros() as u64)
            .collect();
        let (wn, wp50, wp99) = percentiles(&write_us);
        let (sn, sp50, sp99) = percentiles(&slots);
        let (rn, rp50, rp99) = percentiles(&read_us);
        log_warn!(
            "testlatencylitev1: native transfer report -- write delay (send->FirstShredReceived): n={wn} p50={wp50}µs p99={wp99}µs",
        );
        log_warn!(
            "testlatencylitev1: native transfer report -- slots until inclusion: n={sn} p50={sp50} p99={sp99}",
        );
        log_warn!(
            "testlatencylitev1: native transfer report -- read delay (FirstShredReceived->observed): n={rn} p50={rp50}µs p99={rp99}µs",
        );
        // Real, measured breakdown of what makes up read delay above --
        // purely informational, never subtracted from it. See
        // `NativeTransferSample`'s doc comment.
        let shred_completed_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| s.shred_to_completed)
            .map(|d| d.as_micros() as u64)
            .collect();
        let completed_processed_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| s.completed_to_processed)
            .map(|d| d.as_micros() as u64)
            .collect();
        let processed_confirmed_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| s.processed_to_confirmed)
            .map(|d| d.as_micros() as u64)
            .collect();
        let (scn, scp50, scp99) = percentiles(&shred_completed_us);
        let (cpn, cpp50, cpp99) = percentiles(&completed_processed_us);
        let (pcn, pcp50, pcp99) = percentiles(&processed_confirmed_us);
        log_warn!(
            "testlatencylitev1: native transfer report -- shred->completed (block delivery spread on this validator): n={scn} p50={scp50}µs p99={scp99}µs",
        );
        log_warn!(
            "testlatencylitev1: native transfer report -- completed->processed (this validator's own local replay): n={cpn} p50={cpp50}µs p99={cpp99}µs",
        );
        log_warn!(
            "testlatencylitev1: native transfer report -- processed->confirmed (cluster confirmation lag): n={pcn} p50={pcp50}µs p99={pcp99}µs",
        );
        // Phase 3 of TX_INDEX_ESTIMATE_PLAN.md: a real per-transaction
        // position-within-block estimate, using the actual Agave `tx.index`
        // plus each slot's observed max index -- refines `write_delay`
        // beyond its `FirstShredReceived`-anchored lower bound. See
        // `tx_index_estimate`'s doc comment for the full derivation and why
        // it's an estimate, not a measurement. Deliberately its own report
        // block, never merged into the real `write_delay`/`read_delay`
        // percentiles above -- same discipline as `write_delay_upper_bound`.
        //
        // Logged per-sample (below) as well as in aggregate here -- a real,
        // live-user-caught gap 2026-09-08: an aggregate-only percentile line
        // gives no way to tell which specific samples resolved an estimate,
        // or why one didn't. Both loops call the same `tx_index_estimate`,
        // so the aggregate and the per-sample lines are always consistent
        // with each other.
        let write_estimate_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| self.tx_index_estimate(s).ok())
            .map(|e| e.write_delay_estimate.as_micros() as u64)
            .collect();
        let read_estimate_us: Vec<u64> = self
            .state
            .native_samples
            .iter()
            .filter_map(|s| {
                let est = self.tx_index_estimate(s).ok()?;
                s.total_latency.checked_sub(est.write_delay_estimate)
            })
            .map(|d| d.as_micros() as u64)
            .collect();
        let (wen, wep50, wep99) = percentiles(&write_estimate_us);
        let (ren, rep50, rep99) = percentiles(&read_estimate_us);
        log_warn!(
            "testlatencylitev1: native transfer report -- write delay ESTIMATE (send->estimated real execution instant, via tx.index position-within-block -- NOT an exact measurement): n={wen} p50={wep50}µs p99={wep99}µs",
        );
        log_warn!(
            "testlatencylitev1: native transfer report -- read delay ESTIMATE (estimated real execution instant->observed -- NOT an exact measurement): n={ren} p50={rep50}µs p99={rep99}µs",
        );
        // Per-sample breakdown -- the actual point of this session's fix.
        // `i + 1` matches the `N/NATIVE_TRANSFER_TARGET` ordinal each
        // sample was already given in its own `record_native_read` log
        // line (samples are pushed in confirm order and never removed, so
        // this numbering is stable).
        for (i, s) in self.state.native_samples.iter().enumerate() {
            match self.tx_index_estimate(s) {
                Ok(est) => {
                    let read_estimate = s.total_latency.checked_sub(est.write_delay_estimate);
                    log_warn!(
                        "testlatencylitev1: native transfer {}/{} tx.index estimate -- tx_index={} slot_max_tx_index={} write_estimate={}µs read_estimate={} (NOT an exact measurement)",
                        i + 1,
                        Self::NATIVE_TRANSFER_TARGET,
                        est.tx_index,
                        est.slot_max_tx_index,
                        est.write_delay_estimate.as_micros(),
                        read_estimate
                            .map(|d| format!("{}µs", d.as_micros()))
                            .unwrap_or_else(|| "unknown".to_string()),
                    );
                }
                Err(reason) => {
                    log_warn!(
                        "testlatencylitev1: native transfer {}/{} tx.index estimate -- unresolved ({reason})",
                        i + 1,
                        Self::NATIVE_TRANSFER_TARGET,
                    );
                }
            }
        }
        if !unresolved.is_empty() {
            log_warn!(
                "testlatencylitev1: native transfer report -- {} sample(s) had no write/read split (the transaction landed in a slot already known to this guest before it was even sent, most commonly slots_until_inclusion=0 -- see NativeTransferSample::write_delay's doc comment) -- excluded from the write/read percentiles above; total_latency for these is still exact and included wherever total latency is reported elsewhere",
                unresolved.len(),
            );
            if write_bound_us.is_empty() {
                log_warn!(
                    "testlatencylitev1: native transfer report -- none of these had a computable write-delay upper bound (no later slot observed yet, e.g. the run ended right after)",
                );
            } else {
                let (bn, bp50, bp99) = percentiles(&write_bound_us);
                log_warn!(
                    "testlatencylitev1: native transfer report -- of those, {bn} had a real upper bound on write delay (time until this guest observed the next slot after inclusion): p50<={bp50}µs p99<={bp99}µs -- an upper bound, not a point estimate",
                );
            }
        }
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
                "testlatencylitev1: route diagnostics for {mint_in} -> {mint_out}:\n{}",
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
                    "exact quote invalidated pool {} (not ready yet, no cooldown)",
                    failure.pool_id,
                ));
            }
        };

        log_warn!(
            "perpfundingv1: spot leg @ slot {}: {} hop{} {} -> {} amount_in={}",
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
            let (Some(source_ata), Some(dest_ata)) = (
                self.wallet.append_create_ata(owner, hop.input_mint),
                self.wallet.append_create_ata(owner, hop.output_mint),
            ) else {
                return Err(format!(
                    "hop {i}: FAILED to derive token account(s) for owner={owner}"
                ));
            };
            match dex.execute_hop(hop, owner, source_ata, dest_ata, self.wallet) {
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
            log_warn!("perpfundingv1: rebalance skipped -- no wallet keypair yet");
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
                            "perpfundingv1: rebalance: no route to value {} ({}), skipping this pass",
                            symbol,
                            account_id,
                        );
                        continue;
                    }
                }
            };
            let decimals = resolve_symbol_decimals(&symbol).unwrap_or(0);
            log_warn!(
                "perpfundingv1: rebalance holding {}: balance={:.9} (raw {}) value=${:.2} target_pct={:.4}",
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
            "perpfundingv1: rebalance @ slot {}: total portfolio value=${:.2} (usdc=${:.2}, {} priced asset{})",
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
                "perpfundingv1: rebalance: desired buys exceed available capital -- scaled to {:.4} \
                 (reserving ${:.2} for funding-arb margin)",
                scale,
                FUNDING_CYCLE_MIN_MARGIN_USD,
            );
        }
        for (symbol, account_id, amount_in) in sells {
            log_warn!(
                "perpfundingv1: rebalance SELL {} amount_in={}",
                symbol,
                amount_in
            );
            if let Err(e) = self.execute_spot_leg(account_id, mint_usdc, amount_in) {
                log_error!("perpfundingv1: rebalance SELL {} failed: {}", symbol, e);
            }
        }
        for (symbol, account_id, amount_in) in buys {
            log_warn!(
                "perpfundingv1: rebalance BUY {} amount_in={}",
                symbol,
                amount_in
            );
            if let Err(e) = self.execute_spot_leg(mint_usdc, account_id, amount_in) {
                log_error!("perpfundingv1: rebalance BUY {} failed: {}", symbol, e);
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
                    log_warn!(
                        "testlatencylitev1: got wallet keypair {} {}",
                        pubkey,
                        account_id
                    );
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

                    let (phoenix_len, solend_len, kamino_len, marginfi_len, ata_len) = (
                        phoenix_reqs.len(),
                        solend_reqs.len(),
                        kamino_reqs.len(),
                        marginfi_reqs.len(),
                        ata_reqs.len(),
                    );
                    let mut all_requests = Vec::with_capacity(
                        phoenix_len + solend_len + kamino_len + marginfi_len + ata_len,
                    );
                    all_requests.extend(phoenix_reqs);
                    all_requests.extend(solend_reqs);
                    all_requests.extend(kamino_reqs);
                    all_requests.extend(marginfi_reqs);
                    all_requests.extend(ata_reqs);

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
                        }
                        Err(e) => {
                            log_error!("testlatencylitev1: failed to batch-subscribe wallet authority accounts: {e}");
                        }
                    }
                }
                CustomMessageInbound::TargetAllocation(symbol, allocation_pct) => {
                    let account_id = resolve_symbol_mint(&symbol);
                    if account_id.is_none() {
                        log_error!(
                            "perpfundingv1: target allocation symbol {} has no curated mint -- cannot resolve to AccountId",
                            symbol,
                        );
                    }
                    log_warn!(
                        "perpfundingv1: target allocation update: {}={} ({:?})",
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
/// start-to-finish span, and `start()`'s gap-since-previous-start. This
/// is a pure logging threshold -- it decides which real, already-measured
/// gaps are noteworthy enough to warn about; it plays no part in
/// computing the gap itself (that's a plain `Instant::duration_since`).
/// Originally set to 200ms; corrected to 400ms after live validator-side
/// data (root-slot `total=` timings gathered this session) showed real
/// root-slot intervals cluster around 230-250ms median with normal
/// spikes into the 400s -- 200ms wasn't an achievable target given
/// Solana's own real block cadence, not a guest-side problem to chase.
/// Tightened to 350ms 2026-09-04 after two independent real
/// measurements: mainnet's own `getRecentPerformanceSamples` over the
/// last 10 minutes averaged ~317ms/slot, and this guest's own rooted-
/// slot gaps that same session measured median 267ms / mean 340ms (heavy
/// right tail from real network jitter and occasional guest-side
/// backpressure) -- 400ms had drifted loose enough to miss some of that
/// tail as "normal".
const SLOT_TIMING_TARGET_MS: u128 = 350;

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
                    "testlatencylitev1: {}ms since previous slot's start() (target: <{}ms) -- \
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
                "perpfundingv1 stats @ slot {slot}: phoenix_ready={} solend_registered={} kamino_registered={} marginfi_registered={} pending_epoch={:?}",
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
        self.check_native_transfer_arrival(header.accountid, header.slot, UpdateLane::Commit);
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
                log_error!("perpfundingv1: failed to flush phoenix subscriptions: {e}");
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
                "testlatencylitev1: slot {} entering subscription flush (dex pending={:?}/active={:?})",
                self.state.last_slot,
                self.state.o_dex.as_ref().map(|d| d.subscription_pending_count()),
                self.state.o_dex.as_ref().map(|d| d.subscription_active_count()),
            );
        }
        let t_dex_flush = std::time::Instant::now();
        if let Some(mut dex) = self.state.o_dex.take() {
            if let Err(e) = dex.flush_pool(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("perpfundingv1: failed to flush dex subscriptions: {e}");
            }
            // Drains a bounded slice of DexState's own queued startup
            // subscription burst (~32,000 requests across every sub-dex)
            // -- same 128/slot pacing as `subscription_queue` below, just
            // a separate queue instance owned by `DexState` itself.
            if let Err(e) = dex.flush_subscriptions(self.graph, MAX_SUBSCRIBES_PER_SLOT) {
                log_error!("testlatencylitev1: failed to flush dex subscription queue: {e}");
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
            "testlatencylitev1: slot {} dex flush done ({dex_flush_ms}ms), entering wallet subscription_queue flush",
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
                    "testlatencylitev1: subscription_queue flushed {n} requests ({} still pending, {} active)",
                    self.state.subscription_queue.pending_count(),
                    self.state.subscription_queue.active_count(),
                );
            }
            Err(e) => {
                log_error!("testlatencylitev1: subscription_queue flush failed: {e}");
            }
        }
        let queue_flush_ms = t_queue_flush.elapsed().as_millis();
        // Unconditional, once per commit -- bookends "dex flush done" so
        // a hang inside `subscription_queue.flush` (present) vs. one
        // that happens *after* `finish()` returns entirely (absent) can
        // be told apart -- closes the last gap in `finish()` itself.
        log_warn!(
            "testlatencylitev1: slot {} finish() returning (queue_flush {queue_flush_ms}ms)",
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
                    "testlatencylitev1: slot {} took {}ms to process (target: <{}ms) -- \
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
