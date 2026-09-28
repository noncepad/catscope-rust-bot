# multimodelv1 — Graph-Laplacian Multi-Factor Model: Design Plan

Status: **Phases 0-4 implemented** (`src/trader/factor_graph.rs` --
Laplacian construction, eigendecomposition, staleness gate;
`src/trader/factor_borrow_gate.rs` -- borrow-cost gating;
`src/trader/factor_sizing.rs` -- slippage-aware sizing;
`src/trader/factor_intent.rs` -- open-trade-intent data shape,
wire encode/decode, and reconciliation; `src/brain/multimodelv1/message.rs`
-- the Rust-side half of the intent wire protocol), unit-tested, compiling
clean on native and `wasm32-wasip2`. **Phase 4's Go-side half
(`prefetch.db` persistence + boot-time replay) is not built** -- see
Phase 4 below, it's a real gap, not an oversight. **Phase 5 sub-phase 5a
implemented and live-verified**: `brain::multimodelv1` is a real,
registered `EventHandler`, connected for real via `optimizer multi-model
<fee-payer>` (upload, handshake, wallet-key delivery, `on_load complete`,
continuous real commit processing, zero errors, idle -- no
decision/execution logic yet). **Phase 5 sub-phase 5b implemented and
live-verified**: real `factor_graph`/`factor_sizing`-driven live factor
computation, gated behind `TriggerEnableFactorLogging` (read-only) --
multiple real resync cycles confirmed live against real pool data, zero
errors. **Phase 5 sub-phase 5c implemented and live-tested** (two real
bugs found and fixed during the live test itself -- see Phase 5 below):
the pair/stat-arb trade type (`src/trader/factor_residual.rs`
-- real residual decomposition, rolling z-score gates; real Kamino
execution in `state.rs`, ported from `leveragedloopv1`'s own real,
live-tested basis-trade legs), gated behind `TriggerEnablePairTrading`.
Sends real transactions once enabled and a real candidate clears the
real gates -- explicit user choice, not the read-only-first pattern
every earlier sub-phase used. Along the way, found and fixed three real
gaps, none reachable until the capability that exposed them was actually
being built: sub-phase 5b found `TradeRouter` was never kept live (5a's
own gap); sub-phase 5c found `evaluate()` never sent queued transactions
at all, and found the Go side's standard bundler-tip broadcaster had no
Rust-side handler. **Real Solend execution added alongside Kamino (this
session)**, same pair-trade decision logic, per-leg protocol dispatch
(`LendingProtocol`/`best_supply_apy`/`best_borrow_apy`/
`holding_lending_protocol`) picking whichever protocol offers the best
real rate; found and fixed a real, independent pre-existing bug along the
way (`on_account` was never wired for the pair-trade Kamino obligation,
so it could never have actually opened a real position, ever, even before
Solend existed). Candidate universe now the union of Kamino's *and*
Solend's main-market mints (110 usable, up from 49). Live-tested for ~50
real minutes under `--enable-pair-trading` -- zero errors, several real
candidates correctly refused by real gates, but no real open yet (see
Phase 5 below for the full writeup and what's still needed). Marginfi
execution explicitly deferred. Directional-neutral and dispersion trade types, and
Phase 4's Go-side persistence, not started. Rewrite of the research
doc in `INSTRUCTIONS.md` (an eigenvector/graph-Laplacian multi-factor
model for trading tokens on this bot's arbitrage-free pricing graph),
restructured into an execution-shaped plan and extended with the four
gaps identified in `CRITIQUE-1.md`: borrow-cost gating, slippage-aware
sizing, staleness detection, and restart-safe state design. `INSTRUCTIONS.md`
is signal-generation research; this document is what turns it into a bot
mode this repo would actually trust with real money, following the same
phased, prove-idle-before-trading discipline as
[[leveraged_yield_farming_plan]] and `perpfundingv1`.

## What this bot mode does

Cluster tokens into structural factors using the real liquidity graph
(not historical price correlation), decompose each token's return into
factor exposure + idiosyncratic residual, and trade three ways:

1. **Factor-neutral directional** — long a token, short its factor-loading
   basket, isolating idiosyncratic alpha from systemic SOL/market moves.
2. **Stat-arb / pair trades** — long/short the residual when it deviates
   from its factor cluster, betting on mean reversion.
3. **Dispersion** — long a basket of high-idiosyncratic-vol tokens vs.
   short the SOL index, capturing a variance-risk-premium gap.

Structural factors come from the **Normalized Graph Laplacian**
(`L_sym = I − D⁻¹ᐟ² A D⁻¹ᐟ²`) built from real pool liquidity depth, updated
incrementally per-swap (rank-2 perturbation) with a periodic full
resync — see `INSTRUCTIONS.md` sections 1-4 for the underlying math,
unchanged here.

## Why the original design isn't tradeable as written

`CRITIQUE-1.md`'s five points, summarized as the four gaps this plan
closes:

- The Laplacian's edge weights (liquidity depth) are a *structural*
  proxy, never validated against what a real fill would actually cost —
  nothing sizes a trade against real slippage before sending it.
- The stat-arb section names borrow-rate bleed as a risk and stops
  there, with no gating mechanism, even though this exact problem is
  already solved elsewhere in this bot (`derivative_router::decide_basis_trade`,
  see [[feedback_derivative_router]]).
- Incremental rank-2 eigenvector tracking drifts between full resyncs,
  and nothing detects *how* stale the tracked factors are before a
  cycle trades on them.
- The whole design is signal-only — no position sizing, no unwind path,
  and critically, no answer for what a restart does to an open multi-leg
  basket trade whose "intent" (which basket, what entry residual) isn't
  itself represented on-chain the way a Kamino obligation or a Phoenix
  position is.

## Phase 0 — Structural factor engine (pure, standalone, no trading)

Following this codebase's established convention (`derivative_router.rs`,
`credit.rs`, `pricegraph.rs`): build the Laplacian/eigendecomposition
machinery as a **pure, host-import-free module**
(`src/trader/factor_graph.rs`), testable with plain unit tests, no wallet,
no chain reads.

- `GraphLaplacian` / `build_normalized_laplacian(edges: &[LiquidityEdge]) -> SymDense`-equivalent
  in Rust (this repo is native Rust + `wasm32-wasip2`, not Go — the
  `INSTRUCTIONS.md` Go/`gonum` snippet is a reference for the math only;
  re-implement over whatever this repo's existing linear-algebra story is,
  or a minimal in-house symmetric eigensolver if none exists yet — check
  `Cargo.toml` for an existing `nalgebra`/`ndarray` dependency before
  adding one).
- Edge weights sourced from real pool state already loaded by `DexState`
  (`o_dex`) — TVL/depth per pool, exponentially time-decayed
  (`INSTRUCTIONS.md` §"non-uniform time step", half-life a tunable
  constant, not hardcoded 30s without checking it against this bot's
  actual event cadence — `Event::Commit` arrives ~12s, not sub-second, so
  the decay/update cadence should be derived from real observed event
  timing, not assumed).
- Full eigendecomposition path first (correctness baseline); rank-2
  incremental tracking (Phase 3 below) only after the full-resync path is
  proven against known small test graphs (e.g. reproduce
  `INSTRUCTIONS.md`'s own 4-token SOL/mSOL/JUP/BONK example as a unit
  test with hand-checkable eigenvalues).

**Deliberately not wired to any bot mode or wallet in this phase.**

## Phase 1 — Staleness detection (new, closes gap 3)

Two independent triggers, either forces a full resync before the tracked
factors are used for anything that opens a real position:

1. **Time-based**: `now - last_full_resync_ts > MAX_FACTOR_STALENESS_SECS`
   (start conservative — the doc's own suggested 10-30s background-resync
   cadence, but treat that as an upper bound on *trust*, not just a
   resync schedule).
2. **Drift-based**: after every rank-2 incremental update, check
   subspace orthonormality drift on the tracked `V_k`:
   `‖V_kᵀV_k − I‖_F > MAX_ORTHOGONALITY_DRIFT` — cheap (O(N·k²), same
   cost class as the update itself), and a standard proxy for "the
   tracked eigenbasis has drifted enough to stop trusting it" without
   needing a full eigendecomposition to check.

Expose a single `factors_are_stale() -> bool` gate. Every trade-decision
path (Phase 4) checks this first and refuses to open (closes are still
allowed — never block an unwind on a data-quality gate) — same
"the state machine won't act until its own reads are ready" discipline
`leveragedloopv1`/`perpfundingv1` already use for on-chain data, applied
here to derived analytical state instead.

## Phase 2 — Borrow-cost gating (new, closes gap 2)

Reuse the existing gate, don't reinvent it. Every leg of every trade type
here that requires borrowing (short side of a factor-neutral hedge, short
side of a pair trade, the index-short leg of dispersion) must pass the
same shape of check `derivative_router::decide_basis_trade` already
enforces for the basis trade:

- Before opening: real `borrow_apy` for the mint being shorted, read live
  via `credit::CreditReserve::from_kamino`/`from_solend`/`from_marginfi`
  (already-live `DexState` accessors, no new subscriptions needed).
  Estimate an expected holding period from the residual's OU half-life
  (`INSTRUCTIONS.md` §3B), apply a conservative safety multiplier (short
  real-market history makes half-life estimates noisy — do not trust a
  raw point estimate), and require
  `expected_reversion_edge_pct / expected_holding_period_years > borrow_apy_pct`
  before sending the open.
- While open: re-check `borrow_apy` **every cycle**, not just at entry —
  mirror `run_basis_cycle`'s close-pass, which re-evaluates
  `decide_basis_trade` every cycle and closes if the answer flips. A
  borrow-rate spike mid-trade force-closes the short leg regardless of
  whether the residual has reverted yet.

## Phase 3 — Slippage-aware sizing (new, closes gap 1)

The Laplacian's edge weights are for **factor identification only** —
they never determine execution size. Every real order this mode sends
goes through this bot's existing depth-aware quoting, the same path
`TradeRouter`'s own arbitrage search uses to avoid trading on stale
summed weights:

- Size candidate notional against a real quote from
  `PriceGraph::route_slippage_aware` / `requote_edge`
  (`src/trader/pricegraph.rs`), not the Laplacian's `log(1+TVL)`-style
  edge weight — that weight is a structural proxy, not a price.
- Cap notional per leg so the slippage-aware quoted price impact stays
  under a `MAX_PRICE_IMPACT_BPS` constant (new — no equivalent exists yet
  since no prior bot mode here sizes against a factor-model signal).
- For multi-leg trades (factor-neutral hedge basket, dispersion basket):
  quote every leg first, size the **whole basket down to the
  shallowest-liquidity leg**, then apply that scale factor uniformly.
  A basket that's neutral on paper but has one leg too large for its
  pool isn't actually neutral once real slippage-adjusted fills land —
  do not size legs independently.
- Respect `Pool::mark_pool_cooldown` (already exists, used by the
  slippage-aware search to exclude recently-stale-quoted pools) — a
  factor-model trade must not route through a pool the rest of this bot
  has already flagged as unreliable.

## Phase 4 — Restart-safe state design (new, closes gap 4)

Split what's "state" here into two categories that need different
treatment, following this codebase's existing split (real on-chain
positions vs. derived local analytics):

**Factor loadings / eigenvectors — never persisted.** Pure derived
analytics from Phase 0, cheap to fully recompute from a fresh full
resync on every process start (`on_load`). No restart risk here by
construction — treat a restart identically to any other staleness event
(Phase 1's `factors_are_stale()` starts `true` until the first resync
completes).

**Open trade intent — the real gap, and the one `INSTRUCTIONS.md` never
addresses.** A single-obligation leverage loop or a single Phoenix perp
position is *fully described* by its own on-chain account — every other
bot mode here re-derives position state from real reads and needs no
local database because of that. A multi-leg factor-neutral or dispersion
basket is different: the wallet's post-trade token balances alone don't
tell you *which* basket a given balance belongs to, what the entry
residual/z-score was, or what "neutral" was supposed to mean for this
specific open trade. That intent has no on-chain representation.

Design, matching this repo's existing host/guest split. **Correction
(traced directly against `testperpv1` before starting Phase 4):** this
was originally written as "the same mechanism `testperpv1` already uses
for its own persisted target allocation" — that undersold the work.
`testperpv1`'s allocation persistence is not a usable precedent for this:
it runs backwards (Go computes and pushes the allocation into Rust, Rust
never sends anything outbound — `testperpv1`'s `CustomMessageOutbound` is
in fact a literal no-op placeholder enum, nothing to copy), and its
"survives a restart" property is not a runtime read-back at all — Go
writes to `prefetch.db`, but the only consumer is `catscope-rust-bot`'s
own `build.rs`, which reads the table *at compile time* and bakes it into
the binary as a const. A live process restart on the same binary gets
nothing from the database; only a rebuild picks up a persisted value.
That can't work here — trade intent (entry residual, entry timestamp)
doesn't exist until a real trade opens, so it's inherently runtime data,
not something a rebuild could ever bake in. So Phase 4 is four genuinely
new pieces, not a reuse of an existing path:

1. A real `CustomMessageOutbound` variant carrying the intent record
   (basket membership, entry residual, entry timestamp, target neutral
   weights, per-leg expected notional) — Rust → Go, sent once when a
   basket actually opens/closes. No existing bot mode has a working
   example of this direction to copy from.
2. A new Go-side handler that receives that message and writes it into a
   new `prefetch.db` table — no existing "Rust asks Go to persist
   something" handler exists yet to model this on.
3. A genuine **runtime** boot-time step: Go reads the open-intent
   table and pushes any live record(s) into the freshly-booted Rust guest
   via `CustomStdin` — same *channel* the wallet key already uses
   (`DoWallet`, sent from `eval.go`'s boot sequence), but a new message
   type and a new boot-time DB read; nothing in this codebase does this
   today, including `testperpv1`.
4. The Rust side **never trusts replayed intent blindly**: on receipt,
   reconcile it against real on-chain balances for every leg's mint. If
   actual balances don't match the intent record's expected footprint
   within a tolerance, treat the record as untrustworthy — do not resume
   autonomous management of that trade. Surface it and require a manual
   `TriggerCloseAllFactorPositions`-equivalent safety valve (mirroring
   `TriggerCloseAllBasisPositions`) rather than silently acting on stale
   intent. No existing precedent does this reconciliation step either.

Still the highest-uncertainty part of this plan, more so now that it's
confirmed as net-new rather than reused — prototype it in isolation (a
trivial single-field intent record) before building the full
basket-trade version on top of it.

## Phase 5 — Trade execution, gated by Phases 1-3

The full bot mode -- large enough that it's broken into sub-phases rather
than landed in one pass, same discipline as every earlier phase here.

**Sub-phase 5a -- state skeleton, implemented.** `brain::multimodelv1` is
now a real, registered `EventHandler` (`BotMode::MultiModel` in
`brain::mod`), reaching the same "idle-verified" milestone
`leveragedloopv1`/`testperpv1` each hit before any real trading logic
existed: real wallet-key delivery (`CustomMessageInbound::Wallet`, added
here -- Phase 4's own wire-protocol prototype never needed it),
`DexState`/`TradeRouter` construction and paced subscription flushing,
real `on_account`/`on_token`/`low_latency` handling. Deliberately mirrors
`arbv1::state`'s shape, not `leveragedloopv1`/`testperpv1`'s -- `arbv1` is
the smallest existing mode with a real `DexState`+`TradeRouter`, cheaper
to strip down than trimming a heavier mode's Kamino/basis-trade/DAG logic
back out. Tx-latency bookkeeping is deliberately not carried over (an
empty `mid_on_tx` for now -- inherent method, not a trait requirement, so
this is safe to defer). `evaluate()` is still a no-op; Phases 0-4's
`trader::factor_graph`/`factor_borrow_gate`/`factor_sizing`/
`factor_intent` are not wired in yet. Compiles clean on native and
`wasm32-wasip2` (real `cargo build`, not just `check`) -- not yet
live-tested against a real validator connection.

**Sub-phase 5a's Go half -- implemented.** `optimizer/brain/multimodelv1/`
(`multimodelv1.go`/`eval.go`/`init.go`/`instance.go`/`message.go`) and
`optimizer/cmd/multimodel.go` (`optimizer multimodel <fee-payer>`,
registered in `cmd/main.go`) now exist, mirroring `leveragedloopv1`'s Go
package shape with every `SendTrigger*` method/CLI flag removed (nothing
to trigger yet) -- same idle-only scope as the Rust side. `go build ./...`
and `go vet ./...` both pass. One real, necessary deviation from the
template found and fixed during review: every other mode's Go package
sends a `DoEchoRequest` stdin liveness ping on key flag `1` -- that
collides with Phase 4's real `CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT` (whose
Rust-side deserializer hard-errors on a non-`OpenIntent` payload instead
of the usual silent fallback), so the echo ping is dropped entirely here
rather than risk corrupting that parse path (diagnostic-only, not
required for "connects and idles cleanly"). A second real gap found and
fixed *on the Rust side* during the same review: `startBundlerTipBroadcaster`
is wired unconditionally in `cmd/multimodel.go` (every mode's cmd file
does this), but `multimodelv1::message.rs` had no
`CustomMessageInbound::CommonBundlerTipUpdate` arm yet -- real tip
updates would have silently fallen through to `Blank`. Added the variant
+ `state.rs`'s `on_message` arm (`Wallet::apply_bundler_tip_update`),
mirroring every other mode's identical handling. Not yet live-tested
against a real validator connection.

**Sub-phase 5a's real live idle test -- done.** `optimizer multi-model
<fee-payer>` (the CLI's actual kebab-cased command name -- `cmd:"multimodel"`
in `cmd/main.go` doesn't override the framework's own field-name-derived
naming, a real surprise found only by trying it) connected for real:
upload, handshake, wallet-key delivery, `on_load complete`, continuous
real commit processing, zero errors, no triggers sent. Confirms the idle
milestone live, not just compiled.

**Sub-phase 5b -- implemented.** Wires Phase 0 in for real, gated behind
a new one-time `TriggerEnableFactorLogging` (read-only: computes and
logs real structural factors on a periodic cadence, opens/closes
nothing -- same "prove it read-only first" testing plan agreed before
starting this sub-phase). Concretely:
- `state::StateHelper::build_live_factor_graph` builds a
  `factor_graph::FactorGraph` over `curated_symbols()` (at the time this
  sub-phase was written: the 6-symbol, Phoenix+Velocity-perp-gated
  `symbol_mint_config::SYMBOL_MINT_MAP` -- SOL/BTC/ETH/XRP/BNB/SUI;
  broadened to 49 real Kamino-main-market symbols later, see the
  sub-phase 5c live-test writeup below), using **Phase 3's own
  `factor_sizing::price_impact_bps`**
  as the real, depth-aware liquidity signal per pair (`liquidity ∝
  1/impact_bps`) instead of inventing a second pricing path -- a pair
  with no real route just gets no edge.
- `run_factor_resync` computes `factor_graph::structural_factors`,
  evaluates `factor_graph::check_staleness`, and logs the result.
  `evaluate()` calls it on a periodic cadence reusing
  `factor_graph::MAX_FACTOR_STALENESS_SECS` as the resync period (same
  number Phase 1 already uses as the trust bound -- consistent with that
  constant's own "not just a resync schedule" doc comment).
- New `CustomMessageInbound::TriggerEnableFactorLogging` (Rust) /
  `KeyFlagTriggerEnableFactorLogging`+`DoTriggerEnableFactorLogging`
  (Go) / `Hook.SendTriggerEnableFactorLogging` / `--enable-factor-logging`
  CLI flag on `optimizer multi-model` -- full round trip, both sides.

**A real gap in sub-phase 5a's own skeleton, found and fixed while
building this.** `self.state.router` (`TradeRouter`) was seeded with
nodes in `on_load` but never actually kept live -- 5a's `low_latency`/
`CommitHook::on_account` never called `dex.refresh_token_router`/
`dex.refresh_account_router`, unlike `arbv1::state` (this file's own base
template). Without that, `route_slippage_aware` (what
`factor_sizing::price_impact_bps` depends on) could never have found a
real route, no matter how correct sub-phase 5b's own new code was. Fixed
by porting `arbv1`'s exact freshness-gated maintenance pattern
(`m_account_slot`/`record_low_latency_slot`/`is_newer_than_low_latency`,
gating `CommitHook::on_account`'s ~12s-late rooted stream so it can never
roll `low_latency`'s ~400ms-fresher router state backwards). Not yet
live-tested against a real validator connection (unlike sub-phase 5a's
skeleton, which was).

**Sub-phase 5b's real live test -- done.** `optimizer multi-model
<fee-payer> --enable-factor-logging` connected, the trigger landed and
was acknowledged, and multiple real resync cycles ran back-to-back on
the expected 30s cadence, each producing 6 real eigenvalues (correct
`smallest≈0.000000` λ₀ property) against real live pool data. Zero
errors.

**Sub-phase 5c -- implemented, real execution, NOT yet live-tested.**
The pair/stat-arb trade type (Phase 5 point 2), gated behind a new
`TriggerEnablePairTrading`. Explicit user choice: real execution, not a
read-only preview first (every earlier sub-phase used the read-only-first
pattern; this one was asked for directly). Concretely:

- `trader::factor_residual.rs` (new, pure, 19 tests): real residual
  decomposition (`compute_residuals`, `INSTRUCTIONS.md` §3B's
  `epsilon_t = R_t - sum(beta_k F_k)`, projected onto the leading 3 of
  the 6 curated symbols' factors -- `RESIDUAL_FACTOR_COUNT`, a real,
  undocumented-elsewhere scope call, not derived from any formal rule),
  a real bounded rolling window (`RollingWindow`, old samples evicted,
  `MIN_SAMPLES_FOR_ZSCORE=10` before any z-score is trusted), and the
  real 2-3σ entry/exit/stop-loss gates (`find_best_pair`/
  `should_close_pair`, `PAIR_TRADE_ENTRY_ZSCORE=2.5`/`_EXIT_ZSCORE=0.5`/
  `_STOP_ZSCORE=4.0`, all starting values). A pair trade here means
  literally two legs (the single most-underperforming vs. single
  most-overperforming curated symbol), not "long one, short the whole
  basket" -- that's trade type 1, a separate, larger undertaking.
- `state.rs`'s new pair-trade execution path -- **ported almost verbatim
  from `leveragedloopv1`'s own real, live-tested basis-trade Kamino
  legs** (`open`/`close_kamino_deposit_leg`, `open`/`close_kamino_borrow_leg`,
  `execute_spot_leg`, the reserve-refresh/farm-bootstrap plumbing),
  retargeted at this mode's own, fully independent `id=2` Kamino
  obligation (`id=0`/`id=1` are `leveragedloopv1`'s leverage loop/basis
  trade -- `id=2` stays safe even in the real, confirmed case where the
  same fee-payer runs both bot modes and their child wallets collide,
  since both packages derive from the same child-key index). "Which pair
  is currently open" is re-derived from the real obligation's own
  deposit/borrow reserve lists every cycle (`current_open_pair`), not
  tracked as separate local state -- same "re-derive from real reads"
  discipline every other bot mode here already follows.
- **Two more real gaps found and fixed while wiring this in**, neither
  reachable before now (sub-phase 5a/5b never queued or sent a real
  transaction): `evaluate()` never drained/sent `self.wallet`'s queued
  instructions at all -- every other real bot mode's `evaluate()` ends
  with `self.wallet.drain_and_send()`, this mode's never did. And there
  was no `pending_route_pools` field for `execute_spot_leg`/the drain
  tail's pool-cooldown-on-send-failure handling to share.
- **Both flagged gaps closed (same session, on request).** The fixed
  `PAIR_TRADE_ASSUMED_RAW_HALF_LIFE_YEARS` placeholder is gone --
  `trader::factor_residual::RollingWindow::estimated_half_life_cycles`
  now fits a real AR(1) mean-reversion half-life from each symbol's own
  real residual history (standard `phi = cov(x_t,x_{t-1})/var(x_{t-1})`
  OLS fit, `half_life = ln(2)/-ln(phi)`), converted to years via this
  mode's real resync cadence
  (`state::StateHelper::real_expected_holding_period_years`). No
  history yet means no estimate, which now means "refuse to open /
  force a close" rather than "assume a guessed number" -- consistent
  with `factor_borrow_gate::decide_short_leg`'s own "unknown must never
  mean free" rule, and free of extra warm-up cost since it reuses the
  same `MIN_SAMPLES_FOR_ZSCORE` bar the z-score gate already required.
  Honestly documented real limitation: a naive AR(1) OLS fit on a finite
  window can still fit a spurious `phi` inside `(0, 1)` for a genuinely
  non-reverting trend (a known statistical fact -- catching that
  properly needs a real unit-root test, e.g. Dickey-Fuller, not
  implemented here; a concrete regression test documents the limitation
  rather than hiding it). Sizing is no longer fixed at $10/leg either --
  `state::StateHelper::size_pair_legs` now runs both legs through
  `factor_sizing::size_basket` (Phase 3) against real quoted liquidity
  before opening, shrinking both legs together (never independently) if
  either pool can't safely absorb the intended notional, and skipping
  the cycle entirely if slippage shrinks either leg below
  `MIN_SIZED_FRACTION_OF_INTENDED` (50%) of what was intended.
- Both sides (Rust `cargo test`/`cargo check`/`cargo build` on native and
  `wasm32-wasip2`, 325 tests passing; Go `go build`/`go vet`/`gofmt`)
  verified clean after these fixes.

**First real live test -- two more real bugs found and fixed, exactly
the "needs real live-test rounds" pattern flagged above.** Both were
invisible until real funds and a real wallet-load handshake were
actually exercised -- no unit test could have caught either:

1. **The wallet's real USDC balance read as $0.00 the entire session**,
   even though the real on-chain wallet held $55.42 -- not a slow
   warm-up (confirmed by watching it stay at exactly $0.00 across 20+
   real resync cycles / ~10 minutes). Root cause, found by adding
   diagnostic logging and comparing directly against
   `leveragedloopv1`'s own proven wallet-load handler: `Configuration::set`
   (resolves `mint_usdc`/`mint_sol` from fixed constants) was only ever
   called from `on_load`, which is dead code for this on a *fresh*
   connection (`o_rc_keypair` isn't set yet at `on_load` time) -- it was
   never called from the `CustomMessageInbound::Wallet` handler itself,
   where it actually needed to run. `self.configuration.mint_usdc` sat
   at `0` (an invalid `AccountId`) for the whole session, which silently
   broke `derive_ata`/`ata_subscribe_request` for USDC (an invalid mint
   has no real reverse-pubkey mapping, so it's just dropped, not erred)
   and every downstream USDC balance/leg read. Fixed by calling
   `self.configuration.set(&rc_keypair)` from the `Wallet` handler
   itself, matching `leveragedloopv1`'s own working pattern exactly.
2. Compounding gap, found first while chasing bug 1: this mode's
   wallet-load handler never explicitly subscribed to the wallet's own
   ATAs (USDC + every curated symbol) at all -- only the durable-nonce
   account and the pair-trade Kamino obligation's own authority accounts
   were subscribed. Without an explicit `wallet.ata_subscribe_request`
   per mint, a wallet's own SPL token account is never watched, so its
   balance never updates locally regardless of the real on-chain state.
   Fixed the same way `leveragedloopv1`'s own wallet-load handler
   already does it -- explicit ATA subscription requests for USDC and
   every curated symbol, batched into the same `subscribe_now` call.
3. Diagnostic logging added along the way (kept, not temporary): a real
   log line when the open pass is skipped for insufficient tracked USDC
   (reports the real tracked value, not just a silent skip), and a real
   log line when no candidate clears the entry threshold (reports every
   real z-score computed that cycle, not just that none cleared the
   bar) -- both real, permanent observability improvements, requested
   directly while live-watching cycle after cycle produce no signal and
   no way to tell why.

After both fixes: confirmed live -- the wallet's real USDC balance
resolved correctly, the open-pass USDC floor check now passes, and the
cycle now correctly reports "not enough residual history yet" (the
real, expected state for the first ~10 cycles/~5 minutes after
enabling) instead of silently doing nothing. Still watching for the
first real candidate/open once enough real history accumulates.

Extended live watch (same run, attempt 6): real SOL/BTC/ETH z-scores
oscillated for ~30 minutes, produced one more real BTC/SOL candidate
correctly refused for the same "no real half-life estimate yet" reason,
then a real 5.25σ ETH move -- still correctly produced no candidate,
since `find_best_pair` needs a real clearing symbol on *both* sides
(over- and under-performer) and nothing else cleared the opposite side
that cycle. No real trade opened this whole run. This, plus "only
SOL/BTC/ETH of the original 6 curated symbols have any real Kamino
main-market price data at all" (confirmed earlier via direct
`kamino_reserve` query), motivated broadening the candidate universe
below rather than continuing to wait on a 3-symbol pool.

**Candidate universe broadened (this session).** `curated_symbols()`
(`state.rs`) no longer reads the hand-curated, Phoenix+Velocity-perp
-gated `symbol_mint_config::SYMBOL_MINT_MAP` (6 symbols, half without
real Kamino price data). It now reads a new build-time-generated,
database-derived list, `kamino_pair_universe_config::KAMINO_PAIR_UNIVERSE`
(see `build.rs`'s "Kamino main-market reserve mints" section): every
distinct mint with a real reserve on `kamino::KAMINO_MAIN_MARKET`, real
decimals joined from `mint_info`. Real count: 49 (of 55 real
main-market mints; 6 skipped for missing `mint_info` decimals, logged
as build warnings, not fabricated). The Phoenix/Velocity dual-perp-
coverage gate was never actually load-bearing for this trade type --
the pair trade's legs are pure Kamino deposit/borrow, no perp hedge --
so it was only ever an accidental cap inherited from reusing
`perpfundingv1`/`leveragedloopv1`'s own curated list. SOL/BTC/ETH keep
their real ticker labels (from the same hand-verified source); the
other ~46 real mints get a real (not fabricated) label -- their own
truncated base58 address -- since `prefetch.db` has no ticker data for
them. `factor_graph::structural_factors`'s O(n^3) Jacobi and
`build_live_factor_graph`'s O(n^2) real-router liquidity query both
stay cheap at n=49 (a couple hundred thousand ops/resync, not the
billions n=2000 would have cost) -- **not yet live-timed**, so watch
real resync cadence closely after redeploying this change; no
correctness risk either way since a slow resync only delays trading
further; `KAMINO_PAIR_UNIVERSE`'s own doc comment flags this ceiling if
it ever needs revisiting.

Redeployed (attempt 7): connected, `TriggerEnablePairTrading` sent,
factor resync confirmed real (49 eigenvalues), cadence unchanged (~31s,
same as the 6-symbol version -- the O(n^2) router-liquidity query is
genuinely cheap, confirmed live, not just by the earlier order-of
-magnitude estimate). **Real bug found live, same cycle it would have
mattered**: USDC's own mint (`EPjFWdd5...`) is itself a Kamino
main-market reserve, so it was included in the broadened universe --
and by cycle ~10 its real z-score hit -2.67σ, past the entry threshold.
USDC is this strategy's own settlement currency (every leg's notional
is priced/swapped through it), so a leg landing on it would degenerate
to a USDC->USDC swap -- never actually exercised live (no opposite-side
candidate cleared the same cycle), but a real, live-confirmed gap, not
a theoretical one. Fixed: `configuration::MINT_USDC` made `pub(crate)`,
`curated_symbols()` filters it out. Redeployed again (attempt 8) with
the fix; watching for the same z-score computation on the remaining 48
real symbols, USDC no longer among the candidates.

**Real infra disconnect (unrelated to any of the above), then a
restart-latency feature.** The attempt-8 process later exited on its
own with a real "stdio timeout" from the validator connection (brief
astralane tip-stream disconnect just before it) -- no panic, no bug in
this mode's own logic, nothing was ever open so nothing needed
unwinding. Relaunching hit a second, genuinely external blocker: the
local `solpipe bidder proxy` daemon (a separate, user-managed process)
had lost its own unix socket, so the bot couldn't even connect --
resolved by the user restarting that proxy themselves, not a code fix.

While waiting on that, built a real cross-restart warm-up cache, since
every one of this session's several redeploys needed the same ~10-cycle
(~5 minute) climb back from "not enough residual history" before real
z-scores reappeared -- a real, repeatedly-observed cost, not a
hypothetical one:
- `src/trader/residual_snapshot.rs` (new, pure): `ResidualSnapshot`/
  `encode`/`decode` -- the wire format for a mint's real recent residual
  samples + last observed price, and `is_fresh` (a real trust-bound
  check: `RESIDUAL_WINDOW_CAPACITY * MAX_FACTOR_STALENESS_SECS` = 900s,
  the same real span the live window itself represents -- a replay
  older than that describes a market that's moved on and is discarded,
  same fail-closed discipline as everywhere else in this mode).
- `factor_residual::RollingWindow` gained `samples()`/`from_samples()`
  for snapshotting/reconstructing a window's real contents.
- `message.rs`: new `CustomMessageOutbound::ResidualSnapshotReport`
  (sent once per real resync while pair trading is enabled) and
  `CustomMessageInbound::ReplayResidualSnapshot` (received at boot).
- `state.rs`: `send_residual_snapshot` (fires right after
  `run_pair_trade_cycle`, so every sent snapshot is that cycle's real,
  current state) and `apply_residual_snapshot` -- guarded by the *whole*
  `m_residual_history` map being empty, not a per-mint check, so a
  replay either applies atomically and completely or not at all; a
  replay arriving after even one live price has already ticked for any
  mint is discarded outright rather than partially applied (see that
  function's own doc comment for why a per-mint `entry`/`or_insert_with`
  guard would have been wrong).
- Go side (new `prefetch/multimodel` package): per-mint rows in
  `multimodel_residual_snapshot`, payload **opaque** to Go for its
  residual data (never parsed, only ferried) but with `mint` extracted
  from a fixed byte offset to key each row; `instance.go`'s
  `loopInstance` persists on receipt (`KeyFlagResidualSnapshotReport`,
  key flag 2 on the bot->Go stream -- **had to be added to
  `instance.go`'s `switch` before ever redeploying this**, since that
  switch's `default` case hard-errors and tears down the whole
  connection on any unrecognized key, unlike every other mode's
  silent-ignore); `cmd/multimodel.go`'s new `pushResidualSnapshots`
  loads and sends every persisted mint's snapshot immediately at boot
  (no `--trigger-at` delay, unlike `TriggerEnablePairTrading`) so it
  wins the race against the first real resync.
- Verified: 339/339 Rust tests (up from 325), both Rust targets clean;
  Go build/vet/gofmt clean.

**Real bug found live, attempt 10 (first live test of this feature).**
Original design sent one big `ResidualSnapshotReport` per resync cycle,
covering every curated mint with real history in a single message.
Crashed the whole bot connection at cycle ~14 with a real deserialize
error, `"bad value: 3904 vs 4035"` -- `github.com/noncepad/catmsg` (the
shared Go<->bot wire library, used by *every* bot mode including
`leveragedloopv1`'s real, currently-running basis trades) has a hard
`MaxValueSize` cap (3904 bytes) with no chunking of its own, and the
snapshot payload genuinely exceeded it once enough of the 48 curated
mints had accumulated close to full `RESIDUAL_WINDOW_CAPACITY` history.
User's own first suggestion (raise `MaxValueSize` to 500KB in the local
`catmsg` sibling repo) was considered and rejected: that constant is
global, shared infrastructure other real-money bots depend on right now,
and nothing in this system has ever carried a message anywhere near
that size -- real, poorly-understood risk for zero benefit. Fixed
instead by redesigning to one small message per mint (~291 bytes
worst-case, far under the cap regardless of how many curated symbols
this mode ever grows to) on both the send (`send_residual_snapshot`)
and replay (`apply_residual_snapshot`, Go's `LoadResidualSnapshots`/
`pushResidualSnapshots`) sides -- no batching/size-budget logic needed
anywhere, just never combine multiple mints into one message.
`apply_residual_snapshot`'s guard also had to change from "is the whole
map still empty" to a dedicated `residual_history_live` flag, since
multiple small replay messages now arrive in sequence and a map
-emptiness check would incorrectly refuse every one after the first.
Re-verified after the redesign: 339/339 Rust tests, Go build/vet/gofmt
clean. **Not yet live-tested** -- redeploying (attempt 11) next; the
full replay round trip (save on one run, restart, confirm real z-scores
reappear immediately instead of after the usual warm-up) still needs a
real live pass.

**Real Solend execution added alongside Kamino (this session), implemented
and live-tested; no real open observed yet.** User asked to trade through
Solend and Marginfi; direct DB queries disproved the "same liquidity as
Kamino" assumption (Kamino 55/49 usable main-market mints, Solend 89/82,
Marginfi 163/99, only 21 mints overlapping all three) -- confirmed real,
meaningful upside in candidate count from adding Solend, so Solend went
first, Marginfi explicitly deferred. Real, tested Solend execution
infrastructure already existed in `perpfundingv1`/`testperpv1`
(`src/trader/dex/solend.rs`, verified against real mainnet tx hashes in
its own doc comments) -- this was a port of proven code, not new protocol
integration, mirroring how sub-phase 5c's own Kamino legs were ported from
`leveragedloopv1`.

- **A real, independent pre-existing bug was found and fixed along the
  way, unrelated to Solend**: `o_pair_kamino_position.on_account(...)` was
  never wired into either `low_latency` or `CommitHook::on_account` in
  `state.rs` -- every other real bot mode with a position struct of this
  shape (`perpfundingv1`, `leveragedloopv1`) dispatches `on_account`
  unconditionally from both paths; this mode never did. That meant the
  pair-trade Kamino obligation's `registered()` could never become `true`
  from a real on-chain read, so `open_pair_long_leg`/`open_pair_short_leg`
  would keep re-bootstrapping forever instead of ever depositing/borrowing
  -- **the pair trade could never have actually opened a real position
  this whole engagement, even on a fully-cleared candidate**. Not caught
  earlier because no live run had ever reached the open path (every real
  candidate so far was refused first, by the half-life or borrow gate).
  Fixed by adding the same unconditional dispatch `leveragedloopv1`
  already uses, for both the Kamino and new Solend obligations.
- `solend::obligation_address`/`create_obligation_account`/
  `init_obligation`/`SolendPosition::authority_subscribe_requests`/
  `apply_authority` parameterized with an `id: u8` (mirroring Kamino's own
  already-`id`-parameterized shape) -- `id=0` produces the byte-identical
  seed/address `perpfundingv1`/`testperpv1` already use on real mainnet
  obligations (protected by a dedicated regression test); this mode's own
  pair-trade obligation uses `id=1` (`perpfundingv1`/`testperpv1` share
  `id=0`; Kamino's equivalent `id=2` since `leveragedloopv1` already took
  `0`/`1` there).
- Candidate universe broadened again: `kamino_pair_universe_config`/
  `KAMINO_PAIR_UNIVERSE` renamed to `trade_universe_config`/
  `TRADE_UNIVERSE` (`build.rs`), now the union of Kamino's *and* Solend's
  main-market mints (deduped by mint), not Kamino-only. Real count: 123
  union mints, 110 usable after the same `mint_info`-decimals join
  (13 skipped, logged as build warnings). `update_residual_history_and_get_current`'s
  price lookup is Kamino-first/Solend-fallback so Solend-only mints get
  real residual history too.
- New `LendingProtocol` enum (`Solend`/`Kamino`, Marginfi arm dropped) and
  `best_supply_apy`/`best_borrow_apy`/`holding_lending_protocol` --
  direct ports of `perpfundingv1::state.rs`'s own proven shape. The four
  leg functions (`open`/`close_pair_long_leg`, `open`/`close_pair_short_leg`)
  are now thin dispatchers over `_kamino`/`_solend` bodies, matching on
  `best_supply_apy`/`best_borrow_apy` (open) or `holding_lending_protocol`
  (close) -- real rate comparison decides which protocol executes each
  leg, re-derived from live reserve/obligation reads every cycle, never
  tracked as separate local state. `size_pair_legs`/`current_open_pair`
  updated to be protocol-aware the same way.
- Verified after every phase: full `cargo test --lib` (341/341, up from
  339) and both native/`wasm32-wasip2` release builds clean.
- **Live-tested, real `--enable-pair-trading`, ~50 minutes across two
  runs** (41 min + 9 min; both ended by real, unrelated infra
  disconnects -- a validator-side gRPC stream closing, then the local
  `solpipe bidder proxy` daemon losing its socket again, same real,
  user-managed-process gap sub-phase 5c's own live test hit earlier).
  Stable resync cadence at the broadened n=109/110 the whole time (no
  measurable cost from the O(n²)/O(n³) growth flagged when the universe
  was first broadened). Seven real candidates reached the borrow-gate/
  sizing pipeline and were refused correctly every time -- five for "no
  real half-life estimate yet" (a real, pre-existing AR(1) mean-reversion
  constraint, unrelated to this change), two for real slippage sizing
  a leg down below `MIN_SIZED_FRACTION_OF_INTENDED`. Zero crashes, zero
  dispatch errors, zero incorrect executions. **No real deposit/borrow
  has actually fired yet** -- every candidate was refused by a real,
  correct gate before reaching execution, so the new Solend leg functions
  themselves (`open`/`close_pair_long_leg_solend`,
  `open`/`close_pair_short_leg_solend`) are still only compile/type
  -verified and code-reviewed against `perpfundingv1`'s own live-verified
  originals, not yet exercised by a real transaction. Next live pass
  should watch specifically for: both obligations' `registered()`
  becoming `true` from a real on-chain read (confirms the `on_account` fix
  above), a real leg actually landing on Solend, and a real close.

**Still ahead:**
1. Directional factor-neutral: long the target token (spot buy, no
   borrow), short the factor basket sized per Phase 3, against Phase
   2's borrow gate on every shorted mint.
2. Dispersion: long basket + short SOL index (perp, via the existing
   `PerpRouter`/Phoenix infra this bot already has from the basis-trade
   work), same Phase 3 basket-sizing discipline -- would need Phoenix/
   PerpRouter wired into `multimodelv1` from scratch, the biggest lift of
   the three trade types.
3. Wire Phase 4's `OpenIntentReport`/`ReplayOpenIntent` into real trade
   state (currently received but inert, see `state::on_message`'s
   `ReplayOpenIntent` arm) -- now that sub-phase 5c has a real position
   to persist, this is no longer blocked on "nothing to persist yet."
4. Phase 4 points 2-3 on the Go side: the `prefetch.db` handler that
   would receive/persist `OpenIntentReport`, and the boot-time step that
   would read it back and send `ReplayOpenIntent` -- still genuinely
   unbuilt.
5. Sub-phase 5c's real live test is done (50 minutes, see above) but never
   reached a real open -- keep watching (once the local `solpipe bidder
   proxy` is back up) until a real candidate actually clears every gate,
   confirming both the Kamino path (still never live-fired, despite the
   `on_account` fix) and the new Solend path end-to-end, including a real
   close.
6. Marginfi execution, explicitly deferred by the user in favor of
   Solend first -- same `LendingProtocol`/`best_supply_apy`/
   `best_borrow_apy`/`holding_lending_protocol` shape has a clear slot for
   a third arm whenever this is picked up (`perpfundingv1::state.rs`'s own
   3-arm version is the reference).

Manual, explicit enable trigger only for a first live version — same
"nothing happens without an explicit trigger" ethos as every other real
mode here — even though, like the basis trade, this strategy could in
principle run autonomously once proven.

## Explicitly out of scope for v1

- Full Rayleigh-Ritz incremental tracking correctness proof — start with
  full-resync-only (accept the O(N³) cost at a conservative interval)
  and add the rank-2 incremental path only once Phase 1's staleness
  detection is proven to actually catch drift in practice, not just in
  theory.
- Options/variance-swap-style dispersion — `INSTRUCTIONS.md`'s dispersion
  trade is synthetic (spot basket vs. perp index), not real options; stays
  that way, matching the doc's own reasoning (no liquid Solana altcoin
  options market).
- Cross-strategy capital budgeting against the leverage loop / basis
  trade if this ever runs in the same wallet as `leveragedloopv1` — same
  unresolved, explicitly-flagged gap that mode already carries for its
  own two strategies.

## Critical files (once implementation starts)

- `src/trader/factor_graph.rs` — **implemented**, pure (Phase 0-1):
  Laplacian construction, eigendecomposition, staleness gate
  (`check_staleness`/`orthogonality_drift`).
- `src/trader/factor_borrow_gate.rs` — **implemented**, pure (Phase 2):
  `decide_short_leg`/`gate_short_legs`, same
  reuse-the-gate-shape-not-the-function relationship to
  `derivative_router::decide_basis_trade` its own doc comment describes.
- `src/trader/credit.rs` — consumed as-is (Phase 2), not modified.
- `src/trader/factor_sizing.rs` — **implemented**, Phase 3:
  `price_impact_bps`/`max_safe_notional`/`size_basket`. Not host-import-
  free like Phases 0-2 (deliberately depends on `TradeRouter`) -- see its
  own module doc comment.
- `src/trader/pricegraph.rs` — consumed as-is (Phase 3;
  `route_slippage_aware`/`requote_edge`/`mark_pool_cooldown`), not
  modified.
- `src/trader/factor_intent.rs` — **implemented**, pure (Phase 4):
  `OpenIntent`/`IntentLeg`/`reconcile_intent`/`encode`/`decode`. The wire
  byte layout is owned here, not duplicated in `message.rs`.
- `src/trader/factor_residual.rs` — **implemented**, pure (Phase 5
  sub-phase 5c, 24 tests): `compute_residuals`/`RollingWindow`
  (`zscore`/`estimated_half_life_cycles`, the latter a real AR(1)
  half-life fit added when the borrow-gate placeholder was fixed)/
  `half_life_cycles_to_years`/`classify_residual`/`find_best_pair`/
  `should_close_pair`. Genuinely new statistical infrastructure -- no
  prior bot mode here tracks a time series across resync cycles.
- `src/trader/dex/solend.rs` — consumed as-is for pricing since sub-phase
  5b; **this session**, `obligation_address`/`create_obligation_account`/
  `init_obligation`/`SolendPosition::authority_subscribe_requests`/
  `apply_authority` parameterized with a real `id: u8` (mirroring
  Kamino's own shape), plus `SolendState::reserve_by_id` -- `id=0` stays
  byte-identical to the original unparameterized seed (regression-tested,
  protects `perpfundingv1`/`testperpv1`'s real, possibly-open mainnet
  obligations from a silent address change).
- `build.rs`/`src/lib.rs` — **this session**: the "Kamino main-market
  reserve mints" build-time table renamed `kamino_pair_universe_config`/
  `KAMINO_PAIR_UNIVERSE` → `trade_universe_config`/`TRADE_UNIVERSE`, now
  the real, deduped union of Kamino's *and* Solend's main-market mints
  (123 union / 110 usable), not Kamino-only.
- `src/brain/multimodelv1/message.rs` — **implemented**: Phase 4's
  `CustomMessageOutbound::OpenIntentReport`/
  `CustomMessageInbound::ReplayOpenIntent`, Phase 5 sub-phase 5a's
  `CustomMessageInbound::Wallet`/`CommonBundlerTipUpdate` (the latter a
  real gap found during Go-side review, not planned up front — see
  sub-phase 5a's Go notes below), sub-phase 5b's
  `TriggerEnableFactorLogging`, and sub-phase 5c's real
  `TriggerEnablePairTrading`.
- `src/brain/multimodelv1/configuration.rs` — **implemented** (Phase 5
  sub-phase 5a): mirrors `leveragedloopv1::configuration` exactly
  (`wallet`/`mint_sol`/`mint_usdc`/`max_slippage`).
- `src/brain/multimodelv1/state.rs` — **implemented**: sub-phase 5a's
  `State`/`StateHelper`/`on_load`/`CommitHook` impl/`on_message`'s
  `Wallet` arm (mirrors `arbv1::state`'s shape, stripped of arb-specific
  fields/tx-latency bookkeeping); sub-phase 5b's real router-liveness
  maintenance plus `build_live_factor_graph`/`run_factor_resync`; sub-phase
  5c's real pair-trade execution path (`open`/`close_pair_long_leg`,
  `open`/`close_pair_short_leg`, `execute_spot_leg`, the Kamino
  reserve-refresh/farm-bootstrap plumbing -- all ported from
  `leveragedloopv1`'s own real, live-tested basis-trade legs, retargeted
  at this mode's own `id=2` obligation), `current_open_pair` (real,
  on-chain-derived, not locally tracked), `run_pair_trade_cycle`, and
  `evaluate()`'s real `drain_and_send` tail (a real gap found while
  building this -- see Phase 5's sub-phase 5c notes above). **This
  session**: fixed the `on_account` wiring bug described above (both
  `o_pair_kamino_position.on_account`/`o_pair_solend_position.on_account`
  now dispatch unconditionally from `low_latency` and
  `CommitHook::on_account`); added `o_pair_solend_position`, the parallel
  Solend leg functions (`open`/`close_pair_long_leg_solend`,
  `open`/`close_pair_short_leg_solend`, `bootstrap_pair_solend_obligation`,
  `pair_solend_refresh_all_reserves`/`pair_solend_refresh_obligation`);
  `open`/`close_pair_long_leg`/`open`/`close_pair_short_leg` are now thin
  `LendingProtocol` dispatchers over the `_kamino`/`_solend` bodies;
  `size_pair_legs`/`current_open_pair` are protocol-aware.
- `src/brain/multimodelv1/mod.rs` — **implemented**: full `EventHandler`
  (`MultiModelV1Hook`), registered in `brain::mod::BotMode` as
  `BotMode::MultiModel` — `MODE=multimodelv1` is real, runnable, and
  live-verified through sub-phase 5b; sub-phase 5c's real trigger not yet
  live-tested.
- `optimizer/brain/multimodelv1/` (Go) — **implemented through sub-phase
  5c**: `multimodelv1.go`/`eval.go`/`init.go`/`instance.go`/`message.go`,
  mirroring `leveragedloopv1`'s shape. `go build`/`go vet` clean. Real
  gaps found and fixed during review (not anticipated up front): every
  other mode's stdin liveness ping (key flag 1) collides with Phase 4's
  real `CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT`, so it's dropped here
  entirely rather than risk corrupting that parse path (see
  `instance.go`'s doc comment); `cmd/multimodel.go` wires the standard
  bundler-tip broadcaster unconditionally like every mode, but the Rust
  side had no handler for it yet -- added `CommonBundlerTipUpdate`.
  **Still not built**: Phase 4 points 2-3 (a new Go handler that
  receives `OpenIntentReport` and writes a new `prefetch.db` table; a
  new boot-time step that reads it back and pushes `ReplayOpenIntent`
  into the guest) -- not reuse of `testperpv1`'s allocation persistence
  (traced directly before starting Phase 4: that mechanism runs the
  opposite direction and its "restart durability" is a compile-time
  `build.rs` bake, not a runtime read-back — doesn't apply here, see
  Phase 4's own corrected text above).
- `optimizer/cmd/multimodel.go` — **implemented**: registered as
  `optimizer multi-model <fee-payer>` (the CLI framework kebab-cases the
  Go field name regardless of the `cmd:"multimodel"` tag string — a real
  surprise hit live-testing sub-phase 5a, not a bug).
  `--enable-factor-logging` (5b, read-only) and `--enable-pair-trading`
  (5c, REAL execution) are mutually exclusive trigger flags.

## Verification

- Phase 0: unit tests against small hand-checkable graphs (start with
  `INSTRUCTIONS.md`'s own 4-token example).
- Phase 1: unit test that a synthetic large edge-weight jump (simulating
  a drained pool) trips the orthogonality-drift gate.
- Phase 2: unit test mirroring
  `decide_basis_trade_*`'s existing table-driven style, confirming the
  edge/holding-period/borrow_apy inequality gates correctly at the
  boundary.
- Phase 3: unit test confirming basket sizing scales *all* legs down to
  the shallowest leg's slippage-safe notional, not just the shallow leg
  itself.
- Phase 4: the highest-risk phase — verify via a real restart-mid-trade
  drill in a low-notional live test before trusting it with real basket
  size: open a minimal single-pair position, kill the process, restart,
  confirm the reconciliation-against-real-balances path either resumes
  correctly or safely refuses rather than double-acting.
- Same compile/test discipline as every other mode in this repo:
  `cargo check`/`cargo test --lib` on both native and `wasm32-wasip2`
  targets before any live trigger is ever sent.
