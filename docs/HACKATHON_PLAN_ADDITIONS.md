# Stocklana plan — additions (not part of today's submission)

Captured so they're not lost, but explicitly **out of scope for today** —
ship the current plan (title, short/full description, the honest
"Catscope xStock Health Engine" pitch, the live monitor, and the optional
self-triggered liquidation stretch goal) as-is first. These are ideas to
layer on afterward.

## 1. Dashboard: PnL / self-liquidation tracker — DONE (25 Sep)

Built as the "Bot PnL & market overview" dashboard section
(`optimizer/brain/xstockshealthv1/dashboard.go`): a real coverage/PnL
summary, split honestly into "Realized" (from actual liquidation
attempts — real, but $0 until one actually fires) and "Detected" (an
arbitrage-opportunity estimate, explicitly labeled as not a trade), plus
a risk-tier donut and a "which markets are moving" bar list. See
`HACKATHON_PLAN.md`'s 25 Sep revision note for the fuller picture,
including what's still unfired. Original plan below, kept for reference:

Add a section to the local dashboard
(`optimizer/brain/xstockshealthv1/dashboard.go`) that tracks real
financial outcomes, not just live health factors:

- **If/when the self-triggered liquidation demo runs:** before/after
  health factor, repay amount, collateral seized, realized profit in USD
  — essentially the original plan's §5.3 "Liquidation proof card," but
  shown live on the dashboard itself rather than only as a one-off
  screenshot.
- **More generally, a running PnL tracker:** if the bot keeps running
  past the demo (not just fired once), accumulate every liquidation
  event (self-triggered or, eventually, real) it executes over time —
  cumulative profit/loss, not just the most recent event.

**Where this hooks in:**
- Rust side: a new outbound message type from
  `src/brain/xstockshealthv1/message.rs`, mirroring
  `CustomMessageOutbound::HealthBoard` — e.g. `LiquidationEvent { obligation,
  repay_amount, collateral_seized, health_factor_before, health_factor_after,
  realized_profit_usd, slot }`, pushed whenever `kamino.rs`'s liquidation
  instruction (once built) actually lands.
- Go side: a new accumulator alongside `boardStore` in
  `optimizer/brain/xstockshealthv1/dashboard.go` — parse the new message
  in `instance.go` (mirroring `ParseHealthBoard`), store a running list/
  cumulative total, and add an HTML section to `boardPageTemplate`.

## 2. Submission description: open invitation — DONE (25 Sep)

A "Try it yourself" section is now in `SUBMISSION_TEXT.md`'s full
description, covering both points below. Original plan kept for
reference:

Add a short section (or closing call-to-action) to the full description —
something like a new `## Try it yourself` section — along these lines:

- **Right now, anyone can test this against our own validator pipeline**
  (the free, non-staked Catscope tier this whole build ran against this
  session) — the invitation is open, not just an assertion to take on
  faith.
- **For teams interested in running this against their own staked
  validator** (a real, revenue-generating Solana validator, not
  Catscope's free test tier), invite them to reach out and talk — frames
  this as infrastructure other validators/protocols could actually adopt,
  not only a hackathon demo.

Exact wording/contact method (Telegram, email, X, etc.) still needs
deciding — flag as an open item when this actually gets added.

## 3. Validator restart — bundle everything into one, and confirm the deployed baseline first

Confirmed on-chain fact: Kamino Obligation addresses are real PDAs
(`seeds = [tag, id, obligation_owner, lending_market, seed1, seed2]`,
verified against klend's own `handler_init_obligation.rs`) -- but that
doesn't help discovery, since a PDA is only computable if every seed
(including the owner) is already known. There's no way to "list all
PDAs derived from unknown owners," which is exactly why finding every
real obligation in the market needs either the `getProgramAccounts` scan
(already built, live today) or the `lending_market -> obligation` graph
edge (built, tested, not deployed).

**Constraint as of 25 Sep, stated earlier in the day:** the validator
couldn't be restarted for at least ~5 hours from when that was said —
significant session time has passed since, so treat that window as
stale and re-confirm current status before acting on it. Until
re-confirmed, assume the RPC-scan path is still the live one and the
graph edge stays built-but-dormant.

**Before that restart actually happens, resolve one open question:**
which commit/branch is the validator's currently-running edge-generator
plugin actually built from? `main` in this repo is just one commit
(`bd382f9`, "ready for upload to Github") sitting apart from the real
development history -- it does not look like the deploy branch, and
there's no way to tell from git history alone what's actually live.
Once that's known, diff it against `t-10-stocklana` to get a *complete*
list of what a restart would activate -- almost certainly more than
just the obligation edge (Byreal `a9c7094` and the Token-2022 filter
`a628134` are both real candidates for "also not deployed yet," sitting
right next to the obligation edge in the same unmerged history). The
whole point of checking this first is the one thing that was explicitly
asked for: only pay the ~3 hour cost once, for everything that's
pending, not per-change.
