# Leveraged Yield Farming — Design Plan (Phases 0-2 implemented; Phase 2 proven live end-to-end)

Status: **Phase 0 (LST staking-yield estimator) is implemented and
live-verified**, 2026-08-27 -- `optimizer/prefetch/lst-yield` (schema +
`RecordSnapshot`/`EstimateAPY`) and `optimizer watch-lst-yield` (the
poller, see the concrete design below), tracking jitoSOL/bSOL/mSOL.
Real rows confirmed landing in `~/.optimizer/prefetch.db` against live
mainnet accounts (rates matched independently-computed values exactly:
mSOL=1.40265, bSOL=1.31252, jitoSOL=1.29811 SOL-per-LST). `EstimateAPY`
needs a real time span between snapshots (`minEstimateSpan`, 6h) before
it returns anything -- run `watch-lst-yield` for a while before expecting
a real number back.

**Phase 1 (read-only projection) folded into the Phase 0 wiring**,
same day -- see [[project_testperpv1]]: `testperpv1`'s Rust side stores
each `LstApy` update and logs a `net_apy(L)` projection at 1x/2x/3x
using a real, live Kamino USDC borrow rate, no fabricated inputs, no
transactions.

**Phase 2 (the first real leverage-loop mechanics) is implemented as a
new bot mode, `leveragedloopv1`, and live-verified in idle mode** --
2026-08-27, same day. `catscope-rust-bot/src/brain/leveragedloopv1/`
(full `LoopPhase` state machine: `DepositCollateral` ->
`BorrowAndRedeposit` -> `Open` -> `DeleverageWithdrawAndRepay` ->
`DeleverageWithdrawRest` -> `Closed`, manual `TriggerOpen`/`TriggerClose`
only) + `optimizer/brain/leveragedloopv1` + `optimizer leveraged-loop`
(`--open <usd>`/`--close` flags -- the only way a real trigger is ever
sent). Compiles clean on both `wasm32-wasip2` and native targets, 8 new
Rust unit tests passing (211 total), Go side builds/vets/gofmt clean.
**Live-verified end to end in idle mode only** (no `--open`/`--close`
flag passed): real handshake, real wallet-key delivery, real Kamino/dex
state loading (183,146/183,146 build-time pubkeys resolved), zero errors
-- confirms the whole new Rust+Go pipeline works before any real trigger
is ever sent.

**Phase 2 fully proven live, 2026-08-27, same day -- a complete real
open→redeposit→close round trip, independently verified.** Full detail
(the ~16 real bugs found and fixed to get here) in
[[project_leveragedloopv1]]; summary:
- A real `--open 50` deposited real jitoSOL as Kamino collateral and
  borrowed real USDC against it. The loop's own automatic "redeposit"
  sub-step failed safely (a stale-quote pool cooldown, no funds lost)
  and wasn't retried automatically by design (`LoopPhase::Open` is a
  no-op) -- completed manually via a new `TriggerRedepositUsdc` trigger
  instead.
- An unrelated real-money leak from one of the bugs found along the way
  (a partial multi-hop swap landing into an unintended meme token) was
  fully recovered via a generalized `TriggerRecoverToken(mint)` trigger.
- `--close` fully unwound the position for real: repaid the USDC debt in
  full, withdrew all jitoSOL collateral back to the wallet. Verified via
  direct RPC (`getAccountInfo` on the real obligation PDA): debt and
  collateral both back to exactly zero, only the wallet's small
  pre-existing unrelated USDC deposit (from before this bot ever existed)
  remains.
- Final real wallet state (child key
  `Hg2p3cfmg3dratEVy94JArTVM7KzEywgNdmFnrfhroh9`,
  `$HOME/work/optimizer/fee-payer.json`'s child): ~34.49 USDC + ~0.493
  jitoSOL liquid, ~0.375 SOL, no leveraged position, no stray tokens.
- The bot mode is now proven correct through a full real-money
  round-trip, not just idle-verified -- every gap found along the way
  (swap/deposit sizing, tick-array guessing, atomic-group ordering and
  sizing, cross-process trigger phase resets, a shared-utility panic,
  and more) is fixed. See [[project_leveragedloopv1]] for the complete,
  numbered bug list.

See "Phase 2 concrete
design" below for the full mechanics as actually implemented (mirrors
what's described there closely; a few sub-steps were collapsed during
implementation for cleaner on-chain-state-driven phase transitions --
notably borrow+redeposit became one phase/one function, not two, since
Kamino's obligation touches the same jitoSOL reserve on both steps and
a two-phase split would have needed extra local bookkeeping to tell them
apart).

No code in this repo implements Phases 3-4 yet. Written after
shipping `perpfundingv1`'s Phoenix-perp-vs-lending-rate
basis trade (Solend + Kamino) and its idle-USDC yield deployment
(`StateHelper::deploy_idle_usdc`) — this plan reuses most of that
infrastructure but is a **different risk category**: everything shipped so
far is either delta-neutral (the basis trade) or pure supply-only with no
borrow (idle-USDC deployment). A leverage loop is directional, single-asset
exposure with real liquidation risk that nothing in this bot currently
manages.

## What "leveraged yield farming" means here

Deposit an asset as collateral, borrow a second asset against it, swap the
borrowed asset back into the collateral asset, redeposit, repeat — until
some target leverage (total deposited / starting equity) is reached. The
position then earns the collateral asset's supply yield on the *full
leveraged amount*, while paying the debt asset's borrow rate on the
*borrowed portion*. Net APY at leverage `L`:

```
net_apy(L) = L * supply_apr(collateral) - (L - 1) * borrow_apr(debt)
```

This is only profitable when `supply_apr(collateral) > borrow_apr(debt)` —
otherwise every unit of added leverage makes returns *worse*, not better,
since you're paying more on the borrowed unit than you earn on the
redeposited unit.

## Real numbers (live-checked, not estimated) — this is the load-bearing finding

Checked Kamino's main-market reserves directly (same method already used
throughout this session — real account bytes, not third-party APIs).
**Updated 2026-08-26**, re-checked against the 2026-08-17 table below —
kept both on purpose, because the delta between them is itself the
important finding:

| Asset (2026-08-17) | LTV | Utilization | Borrow APR | Supply APR |
|-------|-----|--------------|------------|------------|
| SOL   | 74% | 89.7%        | 4.93%      | 3.76%      |
| USDC  | 80% | 89.3%        | 4.33%      | 3.48%      |
| BTC   | 70% | 7.7%         | 0.34%      | 0.02%      |
| ETH   | 75% | 19.5%        | 0.84%      | 0.13%      |

| Asset (2026-08-26, live) | LTV | Utilization | Borrow APR | Supply APR |
|-------|-----|--------------|------------|------------|
| SOL   | 74% | 94.2%        | 10.668%    | 8.541%     |
| USDC (active reserve) | 80% | 89.2% | 3.786% | 3.041% |
| WBTC(portal) | 70% | 7.7%   | 0.338%     | 0.021%     |
| WETH(portal) | 75% | 20.8%  | 0.899%     | 0.150%     |
| jitoSOL | 63% | 0.60%     | 0.043%     | 0.000%     |
| mSOL    | 59% | 2.49%     | 0.145%     | 0.003%     |
| bSOL    | 45% | 5.77%     | 0.324%     | 0.015%     |

**The spread flipped in 9 days.** SOL's utilization crept from 89.7% to
94.2% — enough to push it past the kink in Kamino's jump-rate curve, more
than doubling its borrow APR (4.93%→10.668%) and, because supply APR is
derived from borrow APR × utilization, its supply APR nearly did the same
(3.76%→8.541%). **Right now**, SOL-collateral/USDC-debt is a large
*positive* spread (8.541% supply − 3.786% borrow = +4.76%, before this
plan's own multi-step/flash-loan complexity):
`net_apy(L) = L*8.541% - (L-1)*3.786%`, e.g. **+22.1% at max leverage
(3.85x, 74% LTV)**.

**This is exactly the rate-flip risk the original draft warned about, now
demonstrated with real data, not hypothesized.** Two reasons *not* to
chase this specific spread as-is: (1) it's a single-asset directional bet —
SOL-denominated collateral against USDC-denominated debt has real price
risk with no LST-style correlation hedge, unlike the loop this plan is
actually built around; (2) a reserve sitting at 94.2% utilization, right at
a jump-rate curve's kink, is a *volatile* place for a spread to live — the
same curve shape that just pushed the rate up 2x in 9 days can push it back
down just as fast the moment borrow demand eases, and adding more borrow
demand (this loop's own USDC borrowing) pushes measured SOL utilization
*further* up only if SOL itself were the debt side — here USDC is the debt
side, so this loop's own activity doesn't directly self-correct the SOL
rate, but it's still a curve position with no safety margin. Treat this as
confirmation that live-rechecking before every open (not just at
position-open time) is a hard requirement, not a nice-to-have.

**Where the durable opportunity still is: LST collateral.** jitoSOL/mSOL/
bSOL's lending supply APR is confirmed near-zero again this check
(0.000%/0.003%/0.015%) — almost nobody borrows LSTs on Kamino — while their
real *staking* yield (baked into the SOL:LST exchange rate) is invisible to
any Reserve account and structurally different from SOL's own volatile
borrow-driven rate: staking yield tracks Solana's real network inflation/
validator commission, not Kamino's local supply-demand curve, so it doesn't
whipsaw the way the SOL-collateral loop above just demonstrated it can. A
loop of `deposit LST → borrow USDC (3.786% now, was 4.33%) → buy more LST →
redeposit` captures `L * (LST_staking_apr + LST_lending_apr) - (L-1) *
usdc_borrow_apr` — this is the loop worth building the estimator for, not
the SOL/USDC one, precisely *because* its profitability doesn't hinge on a
fragile curve position. Still not something this bot can verify for itself
without Phase 0.

## What already exists and can be reused as-is

Everything from the Solend/Kamino integration work is directly reusable,
already verified against real on-chain behavior (live `simulateTransaction`
checks, not just source):

- **Instruction builders**: `SolendReserve`/`KaminoReserve`'s
  `deposit`/`borrow`/`repay`/`withdraw`, `refresh_reserve`,
  `refresh_obligation` — all already correct, including the real
  `refresh_obligation` exact-match requirement (no "extra reserve" padding)
  and Kamino's Farms-account requirement for SOL/USDC specifically
  (`ensure_kamino_farm_ready`).
- **Obligation bootstrap + tracking**: `bootstrap_solend_obligation`/
  `bootstrap_kamino_obligation`, `SolendPosition`/`KaminoPosition` (live
  obligation state via `on_account`, not separate bookkeeping).
- **Protocol selection**: `best_borrow_apy`/`best_supply_apy`/
  `best_usdc_supply_apy` (pick the cheapest borrow / best supply venue).
- **Multi-hop swap execution**: `execute_spot_leg` (already does real
  routing up to 4 hops via `TradeRouter::route_slippage_aware`, already
  confirmed to reach Sanctum's LST pools with no new routing code needed).
- **LTV/liquidation-threshold data**: `KaminoReserve`/`SolendReserve`
  already parse `loan_to_value_pct`. Liquidation threshold is *not* yet a
  parsed field but its offset is already known and documented
  (`loan_to_value_pct`'s offset + 1 byte, per `kamino.rs`'s
  `OFF_LOAN_TO_VALUE_PCT` doc comment) — trivial to add.

## What's genuinely new

1. **LST staking-yield estimator** (shared dependency with the deferred
   Opportunity B). Exchange-rate-delta sampling over time, no external
   oracle dependency (consistent with this bot's on-chain-only stance) —
   needs persisted state across epochs (a rolling window of `sol_value`
   snapshots per LST) since a single point-in-time read can't produce a
   rate.

   **Concrete design (2026-08-26), chosen over relaying samples through the
   WASM bot**: the Rust/WASM guest has no persistent storage across
   restarts and no way to compute a rate-of-change on its own — but
   `optimizer` (Go) already has both, and already independently reads
   real on-chain state on its own schedule (same architecture split
   `pnl`/`watch-pnl` and `alt`'s planned account-usage reporting both rely
   on: WASM = no networking/no persistence, Go = real RPC + `prefetch.db`).
   So this should be a **Go-side-only poller**, not a new Rust→Go wire
   message:
   - New `optimizer prefetch/lst-yield` package (schema.sql +
     `RecordIfChanged`-style writer), mirroring `optimizer/prefetch/pnl`'s
     shape exactly: one row per `(lst_mint, time, sol_per_lst)` snapshot.
   - New `optimizer watch-lst-yield` command (mirrors `watch-pnl`): on a
     timer, reads each tracked LST's real SOL-value ratio directly —
     Marinade's `State.msol_price` (see `trader/dex/marinade.rs`'s account
     layout doc comment), Sanctum S Controller's `LstState.sol_value /
     reserve` (see `sanctum.rs`), and the generic SPL stake-pool ratio (see
     `spl_stake_pool.rs`) for jitoSOL/bSOL/others — all three layouts are
     already fully documented in this repo from prior sessions, so this is
     read-only account decoding Go-side, not new reverse-engineering.
   - A reader function (`EstimateAPY(db, mint, window) (float64, error)`)
     computing annualized yield from the oldest-vs-newest ratio in a
     rolling window (e.g. 7d), same "first-vs-latest snapshot" method
     `pnl.Positions` already uses — deliberately not a fitted curve or
     EWMA, for the same simplicity-over-precision reason `pnl` chose plain
     mark-to-market.
   - **Feeding it back to the strategy**: send the current estimate(s) down
     to the bot over the *existing* inbound message channel each strategy
     already has (the same path wallet keys arrive over) as a new inbound
     custom message, refreshed periodically (e.g. once per epoch) —
     `StateHelper` stores it as plain in-memory state, same as any other
     `on_message`-delivered value. No new Rust→Go outbound message type is
     needed at all, unlike the separate account-usage-reporting plan
     (`compressed-rolling-wirth.md`) which *does* need one because that
     data only exists inside the WASM guest's own `Wallet` state — LST
     exchange rates don't have that constraint, since Go can read them
     independently.
   - Bootstrapping problem: a freshly-started `watch-lst-yield` has no
     history yet, so `EstimateAPY` must return "unknown" (not zero, not a
     fabricated guess) until the window has at least two real snapshots
     spanning a meaningful amount of time — mirrors `pnl.Positions`'
     existing nil-until-known-price discipline exactly.
2. **Loop sequencing.** Deposit → borrow → swap → redeposit, repeated N
   times to reach a target leverage. Each step needs a fresh
   `refresh_reserve`+`refresh_obligation` (obligation state changes after
   every deposit/borrow), so this is realistically several transactions
   across several epochs, not one atomic sequence — unless built on flash
   loans (see "Future optimization" below).
3. **Leverage/LTV target with a safety buffer.** Never loop to the
   protocol's max LTV — that's zero buffer, certain liquidation on any
   adverse price tick. Needs an explicit, configurable target (e.g. 50-60%
   of max LTV) and a hard stop that refuses to add another loop step past
   it.
4. **Health-factor monitoring.** `health_factor = (deposited_value *
   liquidation_threshold) / borrowed_value` — needs live price data for
   both assets (already available via each reserve's `price_usd`) recomputed
   every epoch, with alerting/deleveraging triggers well before
   `health_factor` approaches 1.0.
5. **Deleverage/unwind path.** Both a user-requested full exit (withdraw →
   repay, unwound in the reverse order of the open, since you can't
   withdraw collateral that's securing an outstanding debt beyond the LTV
   line) and an automatic partial-deleverage triggered by health-factor
   degradation — this is the piece that turns "leverage loop" from a
   one-way bet into an actually-managed position, and is the least optional
   part of this whole plan.
6. **Rate-flip detection.** Before adding any new loop step (or holding an
   existing one across epochs), recheck the live spread
   (`current_supply_apy(collateral) + est_staking_apy` vs.
   `current_borrow_apy(debt)`) — a profitable loop can go underwater purely
   from utilization drift, with no price move at all.

## Risk summary (real, not hypothetical)

- **Liquidation risk** — the core, protocol-enforced risk this plan exists
  to manage, not eliminate. A large enough adverse price move between
  collateral and debt assets liquidates the position regardless of how
  conservative the leverage target was, just at a smaller loss.
- **Rate/carry risk** — the profitable spread this strategy depends on is
  not guaranteed to persist; utilization-driven rate curves can flip a
  profitable loop negative with no warning beyond what this bot itself
  monitors.
- **LST-specific risk** — depeg (LST trading below its redeemable value)
  and stake-pool/validator risk, on top of ordinary lending-protocol risk.
- **Cascading-liquidation cost** — a forced liquidation includes a real
  penalty plus slippage on the liquidator's forced sale; the loss is worse
  than the LTV math alone suggests.
- **Smart-contract/protocol risk** — same standing exposure as everything
  else this bot already does against Solend/Kamino, just now with borrowed
  capital amplifying the consequence.

## Phased approach

- **Phase 0** — LST staking-yield estimator: a Go-side-only poller
  (`optimizer prefetch/lst-yield` + `watch-lst-yield`, see the concrete
  design above), fed back to the strategy over the existing inbound
  message channel. Nothing else in this plan is safe to act on without
  this; it's the actual profitability signal.
- **Phase 1** — read-only: given live rates + the Phase 0 estimator, compute
  and log what a loop *would* return at a few candidate leverage levels,
  no transactions sent. Validates the math against real, live data before
  any capital is at risk.
- **Phase 2** — single conservative step, jitoSOL/USDC, 1.3x leverage,
  50%-of-max-LTV safety ceiling, new dedicated `leveragedloopv1` bot mode,
  manual/explicit trigger only, deleverage path built and tested *before*
  the loop-in path ships. **Complete: proven live end-to-end 2026-08-27**
  (see the Status section at the top) -- a real position was opened,
  manually completed, and fully closed again, all independently verified
  on-chain. `--open`/`--close`/`--recover-token`/`--redeposit-usdc` are
  all real, working, live-tested triggers now.
- **Phase 3** — multi-step looping up to a configurable target leverage,
  automatic health-factor monitoring and deleveraging.
- **Phase 4** — monitoring/alerting refinement, position-sizing relative to
  total portfolio (this should never be 100% of capital, independent of how
  safe any single position looks).

## Explicitly not in scope (this plan, this pass)

- Flash-loan-based single-transaction atomic looping — `KaminoReserve::
  flash_borrow`/`flash_repay` already exist in this codebase but have a
  known, previously-flagged, unfixed bug (missing `referrer_token_state`/
  `referrer_account` placeholder accounts) — real future optimization once
  that's fixed, not a Phase 1-4 dependency.
- Cross-protocol leverage netting (e.g. collateral on Kamino, debt on
  Solend) — every existing leg in this bot keeps a symbol's position on a
  single protocol by construction; this plan keeps that constraint.
- Any of this running against the real funded wallet without a separate,
  explicit go-ahead — standing discipline, unchanged, and *more* load-bearing
  here than anywhere else in this bot given the liquidation risk involved.

## Phase 2 concrete design (scoped and implemented 2026-08-27; proven live)

Decided (user's own choices, not defaulted): **jitoSOL** collateral /
**USDC** debt, **1.3x leverage** via a single loop step, **50% of jitoSOL's
real max LTV** as a hard safety ceiling, and this runs as a **new
dedicated bot mode (`leveragedloopv1`)** rather than inside testperpv1 --
testperpv1 already runs its own repeating Solend/Kamino $1 deposit/
withdraw smoke-test cycle against the same wallet's obligation, and
layering a real leveraged position onto that same obligation risked the
two interfering with each other (the smoke test's withdraw phases don't
expect extra real collateral/debt sitting on "their" obligation).

**The math**: a *single* deposit→borrow→swap→redeposit loop produces
`leverage = 1 + borrow_fraction`, where `borrow_fraction` is the portion
of deposited collateral's USD value borrowed. For 1.3x: `borrow_fraction
= 0.3` (borrow 30% of collateral value). jitoSOL's real Kamino LTV is
63% (live-checked 2026-08-26), so 50%-of-max-LTV gives a 31.5% ceiling --
the 30% target already sits *under* that ceiling, so the two chosen
parameters don't conflict; the ceiling is a secondary hard-cap check
(protects against the price moving between "decide to open" and "the
transaction lands"), not the binding constraint.

**Sequence** (mirrors `open_kamino_deposit_leg`'s existing shape closely,
reused where the direction matches):
1. **Fund + deposit**: swap USDC (already held by the wallet) → jitoSOL
   via `execute_spot_leg`, deposit into Kamino as obligation collateral.
   Reuses `open_kamino_deposit_leg`'s logic almost as-is -- it already
   does exactly this swap-then-deposit sequence -- but needs a **mint
   parameter instead of `resolve_symbol_mint(symbol)`**, since jitoSOL
   isn't in the curated 6-symbol perp universe that resolver serves.
   Minimal, mechanical change: take the mint directly, let callers
   resolve however's appropriate for them (basis-trade call sites keep
   using `resolve_symbol_mint`; this new mode uses its own small
   jitoSOL/bSOL/mSOL mint table instead, not `SYMBOL_MINT_MAP` --
   deliberately kept separate since these LSTs have no Phoenix perp
   market and would never be used by `decide_basis_trade`).
2. **Borrow**: new function (no existing leg does this direction --
   `open_kamino_borrow_leg` borrows the *symbol* against *USDC*
   collateral, the opposite pairing). Borrow amount = `min(
   deposited_jitosol_usd_value * 0.30, deposited_jitosol_usd_value *
   0.315 / LTV_SAFETY_FACTOR)` in USDC terms, applying the same
   `borrow_factor_pct`/`LTV_SAFETY_FACTOR = 0.9` over-collateralization
   pattern `open_kamino_borrow_leg` already established (live-confirmed
   necessary there -- a real `BorrowTooLarge` revert otherwise).
3. **Swap + redeposit**: `execute_spot_leg` the borrowed USDC → jitoSOL,
   then deposit *again* into the same obligation. The deposit leg's
   existing `already_deposited` early-exit (built for the basis trade's
   one-shot deposit) needs to allow a second top-up deposit here, not
   skip it -- a real behavioral difference from how the reused function
   is called elsewhere, not just a parameter change.
4. Done -- no further loop iterations. Phase 2 is explicitly one step,
   not the general N-step case (that's Phase 3).

**Deleverage path (built and tested *before* step 1-3 above is ever
allowed to run for real, per this plan's own standing requirement)**:
1. Withdraw the *excess* jitoSOL collateral -- the amount above what's
   needed to keep remaining collateral covering outstanding debt with a
   safety margin (real math: `max_safe_withdrawal_usd = deposited_usd -
   debt_usd / (jitosol_ltv * LTV_SAFETY_FACTOR)`).
2. Swap that withdrawn jitoSOL → USDC (`execute_spot_leg`, reverse
   direction from step 3 above).
3. Repay the USDC debt in full (reuses the existing `repay` builder --
   same one `close_borrow_hedge_leg` already uses).
4. Withdraw all remaining jitoSOL collateral (now debt-free) -- leave as
   jitoSOL, or swap back to USDC; either is safe once debt is zero.
This ordering (withdraw-excess → swap → repay → withdraw-rest) avoids
ever needing outside capital to unwind -- the position pays for its own
exit -- and never withdraws collateral while it's still needed to cover
outstanding debt, which Kamino's own LTV check would reject on-chain
anyway.

**What's still not decided**: the exact starting notional (how much USDC
this first real position actually risks) -- a separate, smaller
risk-tolerance call from leverage/LTV, deliberately not defaulted here
either. Ask before the first real `leveragedloopv1` run.

## Open questions for whoever picks this up

- ~~Target leverage range and safety-buffer size~~ — **resolved
  2026-08-27**: 1.3x leverage, 50% of jitoSOL's real max LTV as a hard
  ceiling. See "Phase 2 concrete design" above.
- ~~Starting notional for the first real position~~ — **resolved
  2026-08-27**: $50, the user's own choice, used for the real `--open`
  that's now been through a full open→close round trip.
- Which asset pairs are actually in scope — LST-vs-USDC is the target,
  confirmed by the 2026-08-26 re-check: it's structurally durable (staking
  yield tracks network inflation, not Kamino's local utilization curve),
  unlike the SOL-vs-USDC spread the same re-check found sitting at
  +22.1%/max-leverage today — real, but living right at a jump-rate curve's
  kink after 2x-ing in 9 days, which is a rate-flip risk this plan
  deliberately doesn't want to hold, not an opportunity to chase. Still
  contingent on Phase 0's estimator actually showing a positive net spread
  once built, not assumed here.
- Whether/when to invest in the flash-loan path once `flash_borrow`/
  `flash_repay`'s existing bug is fixed, given the failed-mid-loop risk a
  multi-transaction loop carries that an atomic one wouldn't.
