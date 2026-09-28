# multimodelv1 live-trading history: near-misses and actual trades

Covers real, live-money verification runs against mainnet for trade types
1 (directional factor-neutral) and 3 (dispersion), both built this
session per `PLAN-1.md`. All figures below are drawn either directly from
real process logs (`~/.optimizer/logs/multimodelv1-{directional,
dispersion}-live*.log`) or cross-checked against real on-chain
confirmations (`solana confirm`) -- not estimated or reconstructed from
memory. Trade type 2 (pair/stat-arb) is summarized briefly at the end
from this session's own earlier live-testing phase, ahead of this
document's own creation.

Wallet under test throughout: `Hg2p3cfmg3dratEVy94JArTVM7KzEywgNdmFnrfhroh9`.

## Trade type 1 (directional factor-neutral): no trades, all near-misses

Six live launches (`directional-live1.log` through `-live6.log`), several
hours of combined real connection time, targets tried: SOL, USDC
(explored then abandoned), BTC. **Zero real transactions were ever sent**
-- no `sent transaction` line appears in any directional log. Every cycle
ended in one of two real refusals:

| Refusal reason | Count (across all 6 runs) |
| --- | --- |
| `<target> has no real hedge basket buildable this cycle (structurally isolated, or no real factor data yet)` | 167 |
| `basket leg <mint> borrow gate refused (apy=20.00% cap=20.00%)` | 19 (across 4 distinct proxy mints: `5z3E..mrRC`, `Df6y..pump`, `CzLS..pump`, `2qEH..pump`) |
| `basket leg <mint> has no real borrow APY yet` | 2 |

Reading: the factor-basket builder frequently couldn't find any real,
sufficiently-loaded proxy for the target's leading structural factors at
all (the large majority of refusals). On the cycles it *did* build a
real basket, the proxy legs it picked were consistently illiquid
long-tail mints sitting at exactly Kamino/Solend's shared 20% max-rate
ceiling -- confirmed (this session) to be each protocol's real
configured rate-curve ceiling for thin listings, not a bot-side
artifact -- so `factor_borrow_gate::decide_directional_short_leg`'s
strict `< 20.0%` cap correctly refused every one. No directional position
was ever open when any of these six processes was killed.

## Trade type 3 (dispersion): 1 real near-miss signal, 6 real dust-cleanup transactions, 0 real dispersion opens (so far)

Five live launches (`dispersion-live1.log` through `-live5.log`,
`-live5` still running as of this writing). Two real, live-discovered
bugs were found and fixed mid-session (see `state.rs`'s `current_open_
dispersion` doc comment for the full detail):

1. **Pre-existing dust misidentified as an open position.** `current_
   open_dispersion`'s restart-safety balance scan has no lending
   obligation to read (dispersion's long legs are plain spot holdings),
   so on first connect it found up to 8 pre-existing, unrelated leftover
   curated-mint balances (residue from earlier pair/directional testing
   this same session) and spent the entire first live run stuck
   force-closing them -- the real open-pass was never reached.
2. Fixed with a real USD-value dust floor (`DISPERSION_DUST_FLOOR_USD =
   $1.00`, Kamino-first-Solend-fallback pricing) plus, after two of the
   five leftover balances turned out to have **no live price and no live
   sell route at all** (permanently stuck even under the first fix),
   treating any curated mint with no resolvable price as dust too
   (user-directed trade-off, accepted knowingly).

### Real transactions sent (dust cleanup, not dispersion opens)

All 6 succeeded on-chain except one, all confirmed via `solana confirm`:

| Time (JST) | Signature | Venue | Outcome | Run |
| --- | --- | --- | --- | --- |
| 16:37:59 | `5o9RbLn3Bav...poXtR` | Orca Whirlpool | Finalized | live3 |
| 16:37:59 | `4346CxJ5sgN...WNV6dTr` | -- | **Failed** (`custom program error: 0x1782`) | live3 |
| 16:38:31 | `GGZ7zgL6avP...Smt9baG` | Raydium CLMM | Finalized | live3 |
| 16:40:15 | `Rh27cEgJ2Qu...aXBoQmqHU` | Orca Whirlpool | Finalized | live3 |
| 17:15:00 | `3bhyzXNKupv...uaJBvKfQPGJ` | Orca Whirlpool | Finalized (confirmed: sold the curated ETH mint's ~$0.08 dust balance back to USDC) | live4 |
| 18:08:45 | `4yMKnv8pPdL...59D6W8PDPW` | Raydium CLMM (2-hop) | Finalized | live5 |
| 19:03:55 | `3wPeDF8S9GE...uY38885rPYBg` | Raydium CPMM | Finalized | live5 |

None of these were a real dispersion basket open or close -- every one
was the close-pass selling off a pre-existing stray balance it correctly
identified as broken/partial (no matching short-index position). By
~3350s into live5 (after the second fix landed), the last of these
leftover legs cleared and `current_open_dispersion` has returned nothing
open since -- the real open-pass has been reachable and running cleanly
from that point on.

### Real near-miss: aggregate z-score cleared the entry threshold, sizing refused every attempt

`DISPERSION_ENTRY_ZSCORE` was temporarily lowered from its real starting
value of 2.0 to 0.6 (user-directed, for live verification only -- see
`dispersion_basket.rs`'s own `TEMPORARY` doc comment, revert once a real
open/close has been observed) to make a real signal more likely to be
seen sooner.

In live5, the real aggregate idiosyncratic-volatility z-score rose from
roughly -1.6 up through **+5.13σ** (comfortably clearing 0.6σ), then
decayed back down over the following ~15 minutes:

```
z=5.13 -> 3.58 -> 2.91 -> 2.48 -> 2.18 -> 1.94 -> 1.96 -> 1.75 ->
1.61 -> 1.49 -> 1.38 -> 1.28 -> 1.20 -> 1.12 -> 1.04 -> ... -> negative
```

On **every single cycle** while `z >= 0.6`, the real basket-construction
step succeeded (a real, buildable candidate set existed), but real
per-leg slippage-aware sizing (`size_dispersion_long_legs` /
`factor_sizing::size_basket`) refused the basket every time:
`"basket sized down too far by real slippage -- skipping this cycle"`.
The entry signal faded back below threshold before a basket ever sized
well enough to open. **No dispersion position has been opened as of this
writing.**

### Current status (as of this document's creation)

- `dispersion-live5.log` still running, connected, cycling normally.
- Aggregate z-score has decayed to roughly -1.1, well below the (lowered)
  0.6 entry threshold -- no open expected imminently.
- Wallet balance: **$126.46 USDC**, **0.3657 SOL**.
- No dispersion position currently open; no Phoenix trader account has
  been bootstrapped yet (never reached -- `open_dispersion_short_index_
  leg` has never been called, since no basket has sized successfully
  enough to attempt a real open).

## Trade type 2 (pair/stat-arb): earlier this session, before this document existed

Per this session's own earlier live-testing phase (Solend execution
added alongside Kamino for the pair trade): extensive live-money testing
across many hours, watching for a real open. **None occurred** -- every
real candidate the residual/z-score engine surfaced was correctly
refused by the real borrow-cost/holding-period gate before a position was
ever opened. No pair-trade position has been observed open at any point
this session.

## Summary

| Trade type | Real opens | Real near-misses (signal cleared, execution refused) | Real transactions sent |
| --- | --- | --- | --- |
| 1: Directional | 0 | 0 (never even built a passing basket) | 0 |
| 2: Pair | 0 | Yes (candidates found, borrow gate refused) | 0 |
| 3: Dispersion | 0 | Yes (z=5.13, sizing refused every cycle) | 6 (all dust cleanup, 1 failed) |

Across all three trade types this session, every real safety gate this
codebase has built -- basket-buildability checks, borrow-APY caps,
slippage-aware sizing floors -- has fired correctly and prevented a
real position from opening on a signal that didn't clear it. The one
real entry signal that *did* clear its threshold (dispersion, z=5.13)
was still correctly refused by the downstream real-liquidity sizing
check rather than opening a poorly-filled basket.
