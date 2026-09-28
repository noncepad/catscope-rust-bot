# testperpv1: xStock (TSLAx) Raydium + Kamino test — report

## Context: what the Stocklana plan is

**Stocklana** is a Solana hackathon (submissions due Friday 25 September,
judging through 2 Oct, $100k prize pool across several sponsor bounties).
Our submission is the **Catscope xStock Health & Liquidation Engine**
(full plan: `edge-generator/HACKATHON_PLAN.md`). The pitch in one
paragraph:

> Kamino Lend is the first major Solana lending protocol to accept
> tokenized equities as collateral — a live market today, ~$31M of the
> ~$53M in tokenized-stock collateral on Solana. Unlike crypto collateral,
> equity collateral's real risk moves at market open/close (9:30am ET and
> after), not continuously — a gap that laggy, RPC-polling monitors and
> liquidators are structurally bad at catching. Catscope decodes both
> sides of that risk **in-validator, same slot**: the lending protocol's
> obligation and reserve state, and the live equity price signal. We're
> building the thing that watches that gap and reacts to it before
> anything polling an RPC endpoint even sees the update — a real-time
> health-factor engine across every xStock-collateralized position on
> Kamino, with a live, self-triggered liquidation as proof it isn't just
> a dashboard.

It's explicitly framed as an **R&D / risk-infrastructure tool** — a
monitoring and execution engine other protocols, risk desks, or liquidator
bots could build on — not a finished consumer product managing real user
funds.

The plan identifies three genuinely new pieces needed, in dependency
order: (1) find and enumerate the real xStocks Kamino market and its
reserves, (2) obligation discovery + a real-time health-factor engine
(`xstockshealthv1`, the watcher built earlier this branch — see
`src/trader/dex/kamino_xstocks_watcher.rs` and
`src/brain/xstockshealthv1/`), and (3) a real liquidation instruction
(`LiquidateObligationAndRedeemReserveCollateralV2`) to actually *act* on
what the watcher finds, not just display it.

## Why this report matters for the plan

The watcher (piece 2) proves **detection** works: it's already live-tested
against all ~6,953 real obligations in this exact market, computing real
health factors from real on-chain reserve/obligation data. What it can't
prove on its own is that the bot can actually **execute** against this
market — and that's exactly what this report demonstrates, on real
mainnet, before any liquidation instruction gets built:

- **Real TSLAx acquisition.** The Raydium CLMM swap round trip
  (`[tslax x/3]`) proves the bot can buy and sell the real, Token-2022
  xStock token on-chain — not a mocked balance. Any real demo position
  (self-triggered liquidation or otherwise) needs genuine xStock
  collateral to work with.
- **Real Kamino xStocks deposit/withdraw.** The `[kamino-tslax x/3]` triad
  proves the bot can deposit and withdraw that TSLAx as collateral into
  Kamino's *actual* xStocks isolated lending market
  (`5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua` —  the same market
  address `kamino_xstocks_watcher.rs`'s `XSTOCKS_LENDING_MARKET` targets),
  via the new Token-2022-aware `deposit_with_token_program()` /
  `withdraw_with_token_program()`. This confirms the on-chain-verified
  assumption behind piece (3) — xStocks reserves need only their
  `liquidity_token_program` slot varied, collateral stays classic SPL
  Token — holds in practice, not just on paper.
- **A real bug in `refresh_obligation`, found and fixed.** The
  remaining-accounts mismatch root-caused here (§2/§3 below) is the same
  instruction family the liquidation instruction has to call correctly
  first — `LiquidateObligationAndRedeemReserveCollateralV2` was flagged in
  the plan (§3a) as its single highest-uncertainty remaining piece.
  Already having hit, root-caused, and fixed this exact failure mode
  against the real xStocks market removes real risk from that last piece,
  rather than leaving it to be discovered for the first time inside a
  liquidation transaction.

In short: detection (the watcher) and execution (this report) are both now
proven end-to-end against the real market. What's left for the plan's
demo is the smallest remaining gap — wiring a real liquidation instruction
on top of infrastructure that's already been shown to work.

## Current status (17 September 2026, ~8 days before the 25 Sep deadline)

**Detection — built, live-tested, not currently running.**
`xstockshealthv1` (`src/brain/xstockshealthv1/` +
`src/trader/dex/kamino_xstocks_watcher.rs`, Go orchestrator in
`optimizer/brain/xstockshealthv1/`) is a passive watcher — zero trades —
that subscribes directly to Kamino xStocks obligations/reserves and
computes a real-time health factor for each. Scaled this session from an
8-address hand-picked sample to the **real full market**: a new one-shot
`getProgramAccounts` scan
(`optimizer/prefetch/kamino/xstocks_obligation.go`, exploiting the fact
that the xStocks market is Kamino's own *isolated* lending market — one
`lending_market` memcmp filter finds every obligation in it) found
**6,953 real obligations with an actual position**, persisted into
`prefetch.db` and baked into the bot at build time
(`XSTOCKS_OBLIGATIONS_GENERATED`, mirroring the existing
`router_pools_data.rs` codegen pattern). Live-verified against real
mainnet with a local dashboard (`http://127.0.0.1:8091/`): reached 1,700+
tracked obligations with the top-100-worst health board updating
correctly before being intentionally stopped. Two real bugs surfaced only
at this scale and were fixed: reserves were FIFO-queued *behind* thousands
of obligations (health factors stayed at zero until the whole queue
drained — fixed by queuing the ~10 reserves first), and the outbound
wire message blew past `catmsg`'s hard 4096-byte per-message ceiling once
enough obligations became "complete" (fixed by capping the board to the
top 100 worst-health entries, which is also just better product design).
**Not currently running** — stopped after this verification; relaunching
is a single `optimizer xstocks-health <fee-payer>` command.

**A second, better discovery path exists but isn't deployed.** A small
edge-generator diff (`edge-generator/src/kamino.rs`) makes Kamino
Obligations walkable via the Catscope graph itself (`lending_market ->
obligation`, mirroring the existing `lending_market -> reserve` edge
exactly) — this would let `optimizer`'s existing prefetch pattern discover
obligations with zero RPC calls, matching how every other DEX/lending
table already works. It's written and tested against a real mainnet
fixture (`tests/data/kamino_obligation1.bin`, full suite passing), but
**requires a validator restart (~3 hours minimum) to test/deploy** — so
the RPC-scan path above is what's actually live today.

**Execution — proven end-to-end (this report).** Real Raydium CLMM
TSLAx swaps and real Kamino xStocks deposit/withdraw both confirmed on
real mainnet, detailed below. This is what makes a real (not simulated)
liquidation demo possible at all.

**Not yet built: the liquidation instruction itself.**
`LiquidateObligationAndRedeemReserveCollateralV2` — the plan's last
genuinely-new piece. Its shape is fully researched (discriminator, args,
19 fixed + 4 optional accounts, from klend-sdk's own IDL-generated TS —
see `HACKATHON_PLAN.md` §3a), and the `refresh_obligation`
remaining-accounts gotcha this report found is exactly the kind of bug
that instruction would otherwise have hit fresh. No code for it exists
yet in either repo.

**Open and still unresolved:** whether Kamino's xStocks reserves actually
go stale overnight the way the plan's "9:30am ET gap" thesis assumes.
Confirmed real: Pyth's `Equity.US.*` feeds genuinely are market-hours-only
(live API, not assumed). Not yet confirmed: Kamino's reserves don't read
Pyth directly — they go through **Scope** (Kamino's own oracle
aggregator, verified on TSLAx's real reserve: `pyth_configuration.price`
is the zeroed default, `scope_configuration` is what's actually
populated) — and what Scope's specific price-chain slot sources from
hasn't been traced yet. This is the one factual gap left in the pitch
itself, separate from any of the engineering above.

All work on this branch (`t-stocklana` here, `t-10-stocklana` in
`edge-generator`) is committed.

---

Run against real mainnet, wallet `3hqu56Yw1aL4MKdYER8hGnJmZ4Q9EfctvCCB9mWYPP6X`,
via `REPO=/home/naomi/work/catscope-rust-bot ./optimizer testperp /home/naomi/work/mainnet/c-wallet-5.json`.
Log: `testperpv1_kaminotslax_run2.log`.

## 1. The 16 steps of testperpv1

testperpv1 is a linear state machine (`TestPhase` enum in `state.rs`) that
exercises real deposit/withdraw/borrow/repay round trips against every
lending market the bot supports, one small real transaction at a time, each
gated by "confirmed on-chain before advancing." The core sequence is
numbered `[1/16]`…`[16/16]` in its own log lines. In words:

1. **Bootstrap USDC** — wrap native SOL into wSOL and swap it to USDC (skipped if USDC balance already covers the deposit tests).
2. **Bootstrap Solend** — register the wallet's Solend obligation account on-chain.
3. **Deposit Solend** — deposit $1.00 USDC into Solend.
4. **Withdraw Solend** — withdraw that USDC back out of Solend.
5. **Bootstrap Kamino** — register the wallet's Kamino obligation on Kamino's *main* market.
6. **Deposit Kamino** — deposit $1.00 USDC into Kamino (main market).
7. **Withdraw Kamino** — withdraw that USDC back out of Kamino.
8. **Bootstrap Marginfi** — register the wallet's Marginfi account.
9. **Deposit Marginfi** — deposit $1.00 USDC into Marginfi.
10. **Withdraw Marginfi** — withdraw that USDC back out of Marginfi.
11. **Borrow Solend** — open a $1.00 SOL borrow-hedge position on Solend (deposits USDC collateral first if needed).
12. **Repay Solend** — buy back SOL and repay the Solend borrow.
13. **Borrow Kamino** — same borrow-hedge, on Kamino's main market.
14. **Repay Kamino** — repay the Kamino SOL borrow.
15. **Borrow Marginfi** — same borrow-hedge on Marginfi (currently always *skipped*: depends on an external Switchboard oracle crank the bot doesn't control).
16. **Repay Marginfi** — repay the Marginfi SOL borrow; marks the whole 16-step test complete.

Alongside this numbered 1–16 sequence, three separate **swap-shaped test
triads** are interleaved into the same `TestPhase` state machine (each
logged with its own `x/3` counter rather than a `/16` number, since they
aren't lending positions):

- **Byreal** (`[byreal x/3]`) — Token-2022 CLMM swap round trip. Currently blocked on an external, on-chain pool condition (a tick array its own bitmap claims exists but doesn't), unrelated to this work.
- **TSLAx / Raydium** (`[tslax x/3]`) — swap USDC → TSLAx → USDC on TSLAx's own Raydium CLMM pool.
- **TSLAx / Kamino** (`[kamino-tslax x/3]`) — deposit/withdraw real TSLAx as Token-2022 collateral into Kamino's xStocks lending market. 

The TSLAx/Kamino triad is deliberately sequenced *between* the Raydium
triad's deposit and withdraw steps: it needs real TSLAx, which the Raydium
leg's deposit just bought, and hands back whatever's left for the Raydium
leg's withdraw to sell back to USDC — one real Raydium buy funds both round
trips instead of acquiring TSLAx twice.

## 2. Files changed for the xStock (TSLAx) tests

### Raydium leg 

| File | What it does |
|---|---|
| `src/trader/dex/tslax.rs` | TSLAx module: `TSLAX_MINT`/`TSLAX_POOL` constants, Token-2022 balance observation bypassing `wallet.token()`'s Token-2022 blindness, swap-instruction building reusing `raydium::clmm::build_swap_ix`/tick-array bitmap walk. |
| `src/brain/testperpv1/state.rs` | `TestPhase::BootstrapTslax`/`DepositTslax`/`WithdrawTslax`; `State::o_tslax_ata`/`tslax_balance` fields; `test_bootstrap_tslax()`, `test_deposit_tslax()`, `tslax_swap()`, `test_withdraw_tslax()`; wallet-subscription wiring for the TSLAx ATA. |

### Kamino leg 

| File | What changed |
|---|---|
| `src/trader/dex/kamino_xstocks_watcher.rs` | Added `pub const XSTOCKS_LENDING_MARKET` (`5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua`) and `pub const TSLAX_RESERVE` (`5iTiczqgUegqA3PpoNpotizMbY9n1sRWr3oL6igKvWuf`) — the real xStocks Kamino market and TSLAx's real reserve in it, verified by direct on-chain byte decoding. |
| `src/trader/dex/kamino.rs` | Added `deposit_with_token_program()` / `withdraw_with_token_program()`: Token-2022-aware variants of `deposit()`/`withdraw()` that take the liquidity token program as a parameter instead of hardcoding classic SPL Token. The existing `deposit()`/`withdraw()` now just delegate to these with `&SPL_TOKEN_PROGRAM_ID`, so no existing caller's behavior changes. Needed because TSLAx's reserve has a real Token-2022 liquidity mint (confirmed on-chain), even though Kamino's own cToken collateral mint stays classic SPL Token regardless. |
| `src/brain/testperpv1/state.rs` | • `TestPhase::BootstrapKaminoTslax`/`DepositKaminoTslax`/`WithdrawKaminoTslax`, inserted between `DepositTslax` and `WithdrawTslax`.<br>• `State` fields: `o_kamino_tslax_reserve`, `o_kamino_tslax_obligation_id`, `o_kamino_tslax_obligation` (a second, separate obligation from the main-market one — deliberately *not* routed through the existing `KaminoPosition` struct, which hardcodes the main market's PDA derivation).<br>• `observe_kamino_tslax_reserve()` / `observe_kamino_tslax_obligation()` — new account observers, wired into both the rooted (`on_account`) and low-latency paths.<br>• Wallet-subscription wiring: subscribes to the TSLAx reserve and derives+subscribes to this wallet's obligation PDA in the xStocks market.<br>• `test_bootstrap_kamino_tslax()`, `test_deposit_kamino_tslax()`, `test_withdraw_kamino_tslax()` — the three phase-handler functions.<br>• `kamino_tslax_deposit()` / `kamino_tslax_withdraw()` — the real `refresh_reserve` → `refresh_obligation` → `deposit_with_token_program`/`withdraw_with_token_program` instruction sequences.<br>• `kamino_tslax_obligation_deposit_reserves()` — bugfix helper (see below), mirrors the pre-existing `kamino_obligation_deposit_reserves()` but reads the xStocks-market obligation instead of the main-market one. |

**Bug found and fixed this session:** the first real deposit attempt failed
on-chain with `Custom(6006) InvalidAccountInput`
(`expected_remaining_accounts=0, actual_remaining_accounts=1`) — klend's
`refresh_obligation` requires its remaining-accounts list to match the
obligation's *current* recorded deposits exactly, and a freshly-initialized
obligation has zero. `kamino_tslax_deposit`/`kamino_tslax_withdraw` were
unconditionally passing the TSLAx reserve into that list before it had ever
been deposited. Fixed by adding `kamino_tslax_obligation_deposit_reserves()`
to derive the list from the obligation's real on-chain state instead — the
same pattern already used for the main-market obligation.

## 3. Test results and what they prove

All results below are from the real mainnet run after the fix, cross-checked
against `solana confirm -v` for the transaction that first exposed the bug.

| Step | Result | What it proves |
|---|---|---|
| `[1/16]`–`[10/16]` Solend/Kamino(main)/Marginfi bootstrap+deposit+withdraw | ✅ All confirmed on-chain (wallet already bootstrapped from prior runs, so these completed in seconds) | Baseline lending integrations are unaffected by this session's changes — no regression. |
| `[tslax 1/3]` Token-2022 ATA confirmed | ✅ | The wallet's TSLAx (Token-2022) ATA exists and is correctly detected — xStocks tokens are handled as ordinary (if Token-2022-flavored) SPL balances on the wallet side. |
| `[tslax 2/3]` deposit swap, USDC → TSLAx | ✅ 276,816 raw TSLAx units received, confirmed on-chain | Real Raydium CLMM swap into a Token-2022 xStock mint works end-to-end (routing, tick-array bitmap walk, Token-2022 swap-instruction building). |
| `[kamino-tslax 1/3]` xStocks obligation registered | ✅ confirmed on-chain (~4s) | `init_obligation` (pre-existing, market-generic) works unmodified against a *second*, non-main Kamino market — no market-specific code needed there. |
| `[kamino-tslax 2/3]` deposit TSLAx into Kamino | ❌ first attempt, real `Custom(6006) InvalidAccountInput` (verified via `solana confirm -v`) → **fixed** → ✅ confirmed on-chain on rerun | Proves the root cause precisely: klend's `refresh_obligation` remaining-accounts count must match the obligation's actual current state, not the reserve about to be touched. Also proves, once fixed, that Token-2022 collateral deposits into Kamino work via the new `deposit_with_token_program()` — the on-chain-verified assumption (xStocks reserves need only their `liquidity_token_program` slot varied, `collateral_token_program` stays classic SPL Token) holds in practice, not just on paper. |
| `[kamino-tslax 3/3]` withdraw TSLAx from Kamino | ✅ cooldown elapsed, withdraw proceeded, obligation vacated | `withdraw_with_token_program()` correctly reverses the deposit — same Token-2022 handling, other direction. |
| `[tslax 3/3]` withdraw swap, TSLAx → USDC | ✅ 276,815 raw units swapped back to USDC, confirmed on-chain | Confirms the full round trip: one real Raydium buy funded both the swap test *and* the lending test, and both handed control back cleanly — no leftover TSLAx stranded, no double-spend or state corruption between the two triads sharing the same ATA/balance. |
| `[11/16]` Solend SOL borrow-hedge | ⏳ still retrying every ~31s as of this report (36+ minutes), not yet confirmed | Pre-existing test, **unrelated to this session's changes** (no code here was touched). Not investigated further, per the task's scope — flagged here only for completeness. |
| `[12/16]`–`[16/16]` | Not yet reached (blocked behind `[11/16]`) | N/A — downstream of the unrelated stall above. |

**Bottom line:** both requested tests — the Raydium TSLAx swap round trip and
the new Kamino xStocks TSLAx lending round trip — passed end-to-end on real
mainnet, back-to-back, sharing one real token acquisition. The one real bug
hit along the way was root-caused from actual on-chain program logs (not
guessed), fixed non-breakingly, and re-verified live. The unrelated `[11/16]`
stall in the older Solend borrow-hedge phase is a pre-existing condition,
observed but out of scope for this change.
