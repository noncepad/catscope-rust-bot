# perpfundingv1 — Phoenix funding-rate vs. lending-rate basis trade

## What this bot does, in one sentence

Every hour, for each of 6 curated symbols (SOL/BTC/ETH/XRP/BNB/SUI), it
compares Phoenix perp's funding rate against the best real lending/
borrow rate available on Solend/Kamino/marginfi, and opens a
delta-neutral position that collects whichever side of that spread is
currently profitable — a classic cash-and-carry basis trade, not
directional speculation.

## History: this used to be a two-venue cycle search

This bot mode was originally built around comparing funding rates
**across two perp venues** (Phoenix vs. Velocity/Drift) using a
`FinancialGraph`/SPFA negative-cycle search — the same graph-cycle
machinery `arbv1` still uses for DEX-price arbitrage today. Once
Velocity/Drift perps were retired from this bot mode, there was no
second venue left to form a cycle against, so that machinery was
replaced with the much simpler direct comparison described below
(`log_basis_cycles`/`decide_basis_trade`) — "a single perp-vs-lending-
rate check per symbol doesn't need a cycle search." The only surviving
trace of the old mechanism is a synthetic self-test of the underlying
SPFA algorithm (checks the algorithm itself works correctly under
wasm32, not real market data) — running this bot today exercises the
basis-trade check, not any cycle search.

## The trade

Phoenix perp funding works like every perpetual: longs and shorts pay
each other periodically based on how far the perp price has drifted
from spot. This bot doesn't take a view on price — it captures the
funding payment while staying flat, using a lending-market position as
the offsetting leg instead of an opposite perp position:

- **Funding positive** (longs pay shorts): **short the perp**, and
  **deposit** the underlying asset on whichever lending protocol pays
  the best supply APY. The deposit's own yield stacks on top of the
  captured funding — this is worth doing whenever funding is positive
  at all, no threshold needed, since depositing never costs anything.
- **Funding negative** (shorts pay longs): **long the perp**, and
  **borrow** the underlying (sold for USDC, a synthetic short) to hedge
  delta. This only clears when the funding collected exceeds the real
  interest paid to borrow: `-funding_rate > borrow_apy`.
- **Otherwise** (funding is ~zero, or negative but not enough to clear
  the borrow cost): no position, no edge to capture.

See `decide_basis_trade` (`state.rs`) — pure, no host-import
dependency, natively `cargo test`-able — for the exact threshold logic,
and its own doc comment for the full economics.

## Picking a lending protocol

Solend, Kamino, and marginfi are all queried every epoch
(`best_borrow_apy`/`best_supply_apy`, `state.rs`), each returning real,
live on-chain rates for whichever protocols have a tracked reserve for
that symbol:

- Deposit-hedge: any protocol helps (deposits only ever earn), so the
  bot picks whichever pays the **highest supply APY**.
- Borrow-hedge: cost matters, so the bot picks whichever charges the
  **lowest borrow APY** — cheapest borrow gives the best chance of
  clearing the funding-collected bar.

The protocol choice is locked in at open time and read back from real
on-chain state thereafter (`holding_lending_protocol` checks Solend/
Kamino/marginfi's live obligations directly, not separate bookkeeping)
— there's no automatic reallocation to a newer, better-paying protocol
mid-position.

## Per-epoch cycle (`evaluate()` → `log_basis_cycles()`)

Runs once per hour (`SECONDS_PER_EPOCH = 3600`, gated on real
wall-clock time via `SystemTime::now()`, not slot/commit cadence):

1. **Close pass** — for every symbol with an open position,
   `close_basis_trade_if_needed` re-checks `decide_basis_trade` against
   this epoch's fresh rates. If the direction has flipped or the edge
   has disappeared, close it (`close_deposit_hedge_leg`/
   `close_borrow_hedge_leg`). Otherwise hold. This runs *before* any new
   opens, and independently of whether a new position will be opened
   this epoch.
2. **Open pass** — for every symbol without an already-open position,
   check `decide_basis_trade`; if it clears, open the appropriate leg
   pair (`open_deposit_hedge_leg`/`open_borrow_hedge_leg`) sized at
   `FUNDING_CYCLE_MIN_MARGIN_USD` ($10, both legs combined — a
   deliberately simple placeholder, not derived from real per-venue
   margin requirements). `spare_usdc` is decremented in-memory as each
   open is queued, greedy first-checked-first-funded, so the bot doesn't
   try to over-commit capital it's already spent earlier in the same
   epoch.
3. **Idle capital** — whatever USDC is left over after every symbol's
   been considered is genuinely idle this epoch, so it gets deployed at
   a real, ~0-market-risk baseline yield (`deploy_idle_usdc`, same best-
   supply-APY selection as the deposit-hedge leg) rather than sitting
   unclaimed in the wallet. This doesn't skip already-deposited
   collateral the way a borrow-hedge's own gate does — idle deployment
   keeps adding capital every epoch as more accumulates.

## What's read from real on-chain state, not tracked separately

Every open/close/holding decision reads real accounts, not internal
bookkeeping: which direction is "currently open" for a symbol comes from
the real Phoenix position's sign (`> 0` long = `BorrowHedge`, `< 0`
short = `DepositHedge`); which protocol backs it comes from checking
Solend/Kamino/marginfi's live obligation data directly
(`holding_lending_protocol`). If any of that data hasn't loaded yet, the
bot holds rather than acting on an incomplete read — same discipline
throughout this file.

## Known simplifications (flagged in-code, not silently assumed away)

- `FUNDING_CYCLE_MIN_MARGIN_USD` ($10/cycle) is a placeholder, not
  derived from Phoenix's real margin requirements.
- No fee/slippage/basis modeling on the spot leg.
- Deposit-hedge sizing is estimated from the lending reserve's own
  oracle price, not the spot swap's real (slippage-affected) output —
  both instructions land in the same transaction, so a bad estimate
  fails the deposit on-chain (insufficient balance) rather than
  depositing a wrong amount.
- One active symbol/position at a time is the common case this file is
  built around; obligation refresh on open only covers the specific
  reserve being touched, not every reserve the obligation might hold
  elsewhere.
- No automatic reallocation between lending protocols mid-position, even
  if a better rate appears elsewhere after open.
