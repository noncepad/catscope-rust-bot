# Handoff: real cross-DEX arbitrage executor for xstockshealthv1

Written for a fresh session to pick this up with full context. The previous
session ran low on context budget specifically because this is real-fund-
moving code that deserves careful verification, not because the work itself
is blocked. Read this whole file before writing anything.

## Deadline context (check this first)

Stocklana hackathon deadline: **Friday 25 Sep 2026, 4:00pm ET** — confirmed
directly against `hackathons.solana.com/hackathons/stocklana` ("Edits are
allowed until submissions close"), not assumed. As of this handoff, the
submission had **not yet been made**. `SUBMISSION_TEXT.md` (this repo,
`docs/`) has a ready-to-paste title/short/full description. **Ask the user how much
time is actually left and whether continuing this is still the right use of
it** before diving in — a real, proven submission in beats a fancier one
that misses the window.

## What problem this solves, and what it explicitly does NOT solve

Earlier in the session, a "detected arb" feature was built comparing
Kamino's oracle price against Orca's live pool price, with a running
dollar total. **This was a mistake, caught by the user, not fixed until
they asked to actually execute it**: Kamino isn't a tradeable venue — it's
a lending protocol, its oracle price only values collateral, there's no
"buy at the Kamino price" instruction. That feature is still in the
dashboard, clearly labeled as a detected/estimated signal, not a trade —
leave it alone, it's honest as it stands, just not what this work is about.

Real arbitrage needs two places you can actually buy and sell the same
asset. That's Orca and Raydium (CLMM), confirmed via live
`getProgramAccounts` scans this session — real, liquid pools exist for all
10 xStock tickers on both.

## What's built and committed (safe — nothing executes yet)

Repo: `/home/naomi/work/catscope-rust-bot`, branch `t-22-stocklana`,
commit `9a2bd61` ("Add real swap-execution infrastructure for Orca +
Raydium CLMM xStock pools"). Companion Go repo:
`/home/naomi/work/optimizer`, branch `t-17-stocklana` — **nothing on the Go
side has been touched for this feature yet**.

Two new modules, both registered in `src/trader/dex/mod.rs`, **neither
wired into any brain mode** — no subscriptions are active, nothing fires,
until `xstockshealthv1::State` is updated to hold and drive them (see
"What's NOT built" below).

### `src/trader/dex/xstock_raydium_watcher.rs` — `XstockRaydiumWatcher`

All 10 xStock/USDC(or SOL) Raydium CLMM pools. The underlying mechanism —
subscribe to a pool, walk its `tick_array_bitmap` for real initialized
windows, subscribe to those, wait for a real on-chain confirmation
(`owner == RAYDIUM_CLMM_PROGRAM_ID && lamports > 0`) before trusting any of
them exist — is **not new risk**. It's a direct generalization of
`tslax::TslaxState`, which already fired a real, live-verified round-trip
swap against one hardcoded pool. Only the 9 additional pool *addresses* are
new; the approach is proven.

Pool addresses (picked via live scan, filtered to USDC/SOL-quoted pairs,
then the real on-chain `liquidity` field checked directly — several
first-found candidates had **zero liquidity**, dead pools, and were
discarded after checking, not assumed good):

```
AAPLx  CKwJZwm7oj3nu4653N1EpDrqXbXAYXoPFiPeEnLouF8y   (USDC)
CRCLx  G39wywquKbHK8F2wZZZFX3fcsyG91VCCbbr6WEVp5axy   (USDC)
GOOGLx B8YAwjGYk6qidWzGBXMAxP7nYfG8g74EZ3Y4gFSsobRw   (USDC)
HOODx  DXWbip5LducMAbDSSpLYz9Xik3253EPeAYQufQtx7LXs   (USDC)
METAx  3L7KbPVaAQA4UTecaGQYsm6UCq5F3sZM9zAYkxqYt63j   (USDC)
MSTRx  2ngTuP7xA581dqX9uJkGRqxmKuehY3k4SDfPebeoRG2J   (SOL)
NVDAx  49iMatQtoyabsYAQc8GafVq6aeBFVDxSRH44oiatyyw6   (USDC)
QQQx   GMjGLWzvK75LPetrgAmdeXnvxc4fUuQPwJxeQqTDU1aG   (USDC)
SPYx   6truu3rZuiB9rKQg4VYC3Dt3QwV7DgwGqXrYUcrvnDDE   (USDC)
TSLAx  8aDaBQkTrS6HVMjyc6EZebgdiaXhLYGriDWKWWp1NpFF   (USDC)
```

**Note on TSLAx**: this is a *different, more liquid* pool (~5x) than the
one `tslax.rs` itself was tested against
(`HHQUnUbmWLrYzkscDY1C3deEFbGtiGBGoHjpANogmvum`). The tick-array *mechanism*
is proven; this *specific address* is not yet itself real-trade-tested.
Chosen deliberately for lower slippage on a small trade — worth a first
real test against this exact pool before trusting it blind.

Public API: `XstockRaydiumWatcher::new() -> (Self, Vec<SubscriptionRequest>)`,
`.on_account(header, body)`, `.flush_subscriptions(g, max_per_flush)`,
`.pool(ticker) -> Option<(AccountId, &RaydiumClmmPool)>`,
`.ready_tick_arrays(ticker, zero_for_one) -> Option<Vec<AccountId>>`,
`.swap(ticker, params: &SwapParams, wallet: &mut Wallet) -> Result<(), TraderError>`.
Callers **must** check `ready_tick_arrays` before calling `swap` — it
doesn't re-check internally (matches `tslax::TslaxState::swap`'s own
contract).

### `src/trader/dex/xstock_orca_swap_watcher.rs` — `XstockOrcaSwapWatcher`

Same subscribe/confirm pattern, for Orca. **Deliberately scoped to TSLAx
only** (`XSTOCK_ORCA_SWAP_POOLS` has exactly one entry):

```
TSLAx  9p7abUFv31ycgu9kckvnoqMMvBy67dqTDM2m6HP9xokN   (USDC)
```

(Same pool `xstock_dex_watcher.rs` already uses for price-only tracking —
see that file's `XSTOCK_DEX_POOLS` for the other 9 tickers' Orca pool
addresses, already discovered, if/when this scales.)

**Why only one, and why this is the real open risk**: checked every call
site of Orca's swap-building code (`OrcaWhirlpool::build_swap_ix`/
`build_swap_ix_pda`) in this entire codebase — there are none. Only
`orca.rs` itself defines them. **No code path has ever fired a real Orca
swap.** Only price-reading (`spot_price()`) is proven. Worse: the existing
`build_swap_ix_pda` naively derives 3 consecutive tick-array PDAs by pure
arithmetic with **no check they exist on-chain** — the exact assumption
that already caused a real, live `NotEnoughTickArrayAccount` failure on a
different pool (documented in `raydium::clmm`'s and `byreal.rs`'s own doc
comments). `XstockOrcaSwapWatcher` fixes that the same way Raydium/Byreal
already had to: subscribe to the candidates, confirm each is real before
trusting it.

Orca has **no bitmap** the way Raydium's `PoolState` does, so there's no
way to search for "next initialized" — it just subscribes to a radius of
candidates (`CANDIDATE_RADIUS = 4` arrays each side) around current price
and confirms them, same candidates `build_swap_ix_pda` already assumes,
just verified instead of trusted blind.

Public API mirrors the Raydium watcher: `.new()`, `.on_account()`,
`.flush_subscriptions()`, `.pool(ticker)`, `.ready_tick_arrays(ticker,
a_to_b) -> Option<[AccountId; 3]>`, `.swap(ticker, tick_arrays: &[Pubkey;
3], params, wallet)`.

**Do not scale `XSTOCK_ORCA_SWAP_POOLS` past TSLAx until a real trade
against that one pool has actually been confirmed working live** — this
was an explicit user decision ("prove Orca on 1 pool first, then scale"),
made *after* the zero-call-sites finding above, not before it.

## What's NOT built yet (the actual executor — start here)

None of this exists. In rough dependency order:

1. **Wire both watchers into `xstockshealthv1::State`** (`src/brain/
   xstockshealthv1/state.rs`). Mirror exactly how `KaminoXstocksWatcher`
   and `XstockDexWatcher` are already wired — same file, look at: the
   `State` struct fields, `on_load` (construct + queue initial
   subscriptions), `low_latency` and `CommitHook::on_account` (route
   `on_account` calls), `finish` (flush subscriptions each slot). Do the
   same for `dex_raydium: XstockRaydiumWatcher` and
   `dex_orca_swap: XstockOrcaSwapWatcher`.

2. **Real price on the Raydium side.** `OrcaWhirlpool::spot_price()`
   already exists for Orca. Check whether `RaydiumClmmPool` has an
   equivalent — if not, add one (same `sqrt_price_x64` decode:
   `(sqrt_price_x64 / 2^64)^2`, scaled by `mint_decimals_0`/
   `mint_decimals_1`, same math already verified this session for both
   Orca and Kamino's Scope price). **Verify Raydium CLMM's real swap fee
   too** — it may live in a separate `AmmConfig` account (referenced by
   `RaydiumClmmPool::amm_config`), not the pool itself. Don't assume;
   check the real account layout before using it in a profitability
   calculation.

3. **Real profitability check.** A gap is only real money after: Orca's
   fee (`fee_rate` field, already parsed) + Raydium's fee (from step 2,
   verify the real source) + Solana tx fees (negligible, ~5000 lamports,
   fine to ignore) + a minimum safety margin. Given minimal position size
   (see next point), price impact on these liquid pools should be small,
   but check, don't assume.

4. **Minimal real position size.** Define a constant the same way
   `MAX_REPAY_USD = 2.0` already does in `state.rs` for liquidations.
   **Confirm the actual dollar amount with the user before writing real
   funds into this** — it was never pinned down before this handoff.

5. **ATA management on both venues.** The xStock side is Token-2022
   (reuse `state.rs`'s existing `get_or_create_ata` helper, already
   Token-2022-aware). USDC is classic SPL. Both venues need source/dest
   token accounts.

6. **Bundle both swap legs atomically.** Build both swap instructions
   (buy on the cheap venue, sell on the expensive one) into the *same*
   transaction before draining the wallet — exactly the pattern
   `attempt_liquidate()` already uses (refresh_reserve × 2 +
   refresh_obligation + liquidate, all queued before one
   `drain_and_send`). This matters: it eliminates "legged" risk (one side
   fills, price moves, the other doesn't) — either both swaps land or the
   whole transaction reverts and nothing is lost but gas.

7. **Real PnL tracking**, distinct from the existing "detected" arb
   number. This needs real captured profit — ideally a post-trade balance
   check, or at minimum a real signature + sent/failed status, honestly
   labeled the same way `LiquidationEvent` already is (estimate vs.
   confirmed, spelled out, not implied).

8. **New wire message** (`message.rs` + Go `message.go`), shaped like
   `LiquidationEvent`. **Check the 4096-byte `catmsg::BUFMAX` cap** before
   picking any per-entry size or array length — this has bitten this
   feature area twice already (`HEALTH_BOARD_MAX_ENTRIES`'s own doc
   comment has the math; reuse that approach, don't eyeball it).

9. **Dashboard section** for real arb trades — keep it visually and
   textually distinct from the existing "Detected arb opportunity" card,
   so a real executed trade is never confused with a passive estimate.

## Conventions this session established — follow these

- **Never guess an on-chain layout or address.** Every offset/discriminator/
  pool address in this codebase (and everything referenced above) was
  verified against real `getAccountInfo`/`getProgramAccounts` calls or
  primary source (program source, IDL), not assumed. Keep that discipline
  for the Raydium fee/AmmConfig lookup and anything else new.
- **Wire messages have a hard 4096-byte ceiling**, unconfigurable. Compute
  the real max entry count before picking a cap; leave real headroom, don't
  sit at the theoretical edge (see `HEALTH_BOARD_MAX_ENTRIES`'s doc comment
  for the worked example and why 110 was chosen over the true max of 120).
- **Label estimates as estimates, everywhere** — code comments and UI text
  both. The existing "detected" arb number and `LiquidationEvent::
  estimated_bonus_usd` are the house style to match.
- **Real fund-moving Write/Bash calls tend to get blocked by the Claude
  Code classifier.** Expect it; explain what was being attempted and defer
  to the user running it themselves rather than trying to route around it.
- **Confirm the execution/sizing plan with the user before firing anything
  real.** This session was corrected twice for building ahead of an
  explicit go-ahead on scope — for the part that actually moves funds,
  don't repeat that; confirm size and go/no-go before the first live send.
- **`optimizer` (Go) does not self-rebuild.** `go build -o optimizer
  ./cmd/...` must be run manually before every relaunch after a Go-side
  change. The WASM bot *does* rebuild fresh automatically every launch.

## Where everything else lives

- Plan doc: `HACKATHON_PLAN.md`, right alongside this file — originally
  lived in `edge-generator`, moved here (this repo, `t-22-stocklana`) so
  every project doc lives in one place. Has a 25 Sep revision note
  giving the real current status (splits proven-live-verified work from
  built-but-unfired work).
- Submission text: `SUBMISSION_TEXT.md`, also right here now. A later
  revision fixed a real overclaim ("self-liquidation tested, firing real
  signed transactions" was false) before it went out — worth re-reading
  before assuming anything in it is proven rather than just built.
- Ideas doc: `HACKATHON_PLAN_ADDITIONS.md`, also here.
- Dashboard code: `/home/naomi/work/optimizer/brain/xstockshealthv1/` —
  `dashboard.go` has the existing "Detected arb opportunity" card and
  pagination; `message.go` has the wire-format conventions to match.
