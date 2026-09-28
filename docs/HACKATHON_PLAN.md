# Stocklana submission plan — Catscope xStock Health Engine

Deadline: **Friday 25 September, 4:00pm ET** (submissions), judging through
2 Oct. **This revision written 24 September, the day before the deadline.**

**Revision note (24 Sep):** the original pitch below claimed Kamino's
xStocks collateral is priced on a market-hours-only feed that freezes
overnight and "catches up" at 9:30am ET — a specific, checkable technical
claim. We checked it directly on-chain: it's false. Kamino's real reserve
price for TSLAx updates every few tens of seconds, 24/7, including at 3am ET
with the real market closed (traced through Kamino's actual oracle
aggregator, Scope, to the live on-chain price account — not assumed, not
taken from marketing). The pitch below is rewritten to drop that claim
entirely and stand on what's actually true instead. See
`catscope-rust-bot/docs/testperpv1-xstock-kamino-report.md` for the full
trace.

**Revision note (25 Sep, submission day):** everything in §4/§5 below is
now stale — it describes plans, not what was actually built. Real status,
split honestly between proven and unproven (see `SUBMISSION_TEXT.md` for
the version written for the actual submission form):

- **Proven, live-verified**: full obligation discovery (~7,000+ real
  obligations, growing); a real-time health-factor engine now priced off
  Scope's *live* oracle feed directly, not a reserve's own cached price
  (a reserve's cache can lag the live oracle by minutes on a quiet
  reserve — measured, not assumed); 3-lane detection racing (Commit/
  LowLatency/Transaction) with real per-lane win counters; a live DEX
  activity feed tracking real Orca swaps per xStock; a real dashboard
  (not the htmgo/SSE design in §5 — see below).
- **Built, not yet fired live**: the real liquidation instruction
  (bundles refresh+refresh+liquidate atomically, compiles clean, never
  actually executed — confirmed via direct code search, not assumed:
  `kamino.rs` has no deposit/borrow instruction builder at all, only
  `init_obligation`/`refresh_obligation`/`liquidate`, so there's no way
  yet to create a demo position to trigger a real liquidation against.
  Building one would need the same rigor the liquidate instruction took
  — real account layout verified against Kamino's SDK, not guessed);
  real cross-DEX arbitrage
  infrastructure (tick-array-aware pool tracking across Orca and Raydium
  CLMM for all 10 tickers, liquidity-verified, not wired into a live
  trigger yet). See `catscope-rust-bot/docs/ARB_EXECUTOR_HANDOFF.md` for
  the detailed state of this work if picking it back up.
- **An earlier draft of the actual submission text (`SUBMISSION_TEXT.md`)
  briefly overclaimed the liquidation had fired live — caught and
  corrected before submitting**, same discipline as the gap-thesis
  correction above.
- The real dashboard (Go, polling-refreshed, not the SSE/htmgo design
  §5 describes) has: a paginated worst-to-best health board, a
  paginated liquidation-attempt log, a "Bot PnL & market overview"
  section (coverage stats, a risk-tier donut, a "which markets are
  moving" bar list, and PnL split honestly into "Realized" vs.
  "Detected" — the latter an estimated opportunity size, not a trade),
  and the live DEX activity feed.

## 1. The pitch, in one paragraph

Kamino Lend is the first major Solana lending protocol to accept tokenized
equities as collateral — a live, real market today, 10 xStock reserves
(SPYx, QQQx, HOODx, GOOGLx, CRCLx, TSLAx, NVDAx, AAPLx, METAx, MSTRx), tens
of millions of dollars of real collateral, with thousands of real open
positions — we found and verified **6,953 real obligations** with an actual
deposit or borrow in this one market. Nobody has built dedicated, complete
risk-monitoring infrastructure for this specific, fast-growing collateral
type yet, and watching it by polling an RPC endpoint across thousands of
accounts repeatedly is slow and expensive at this scale. Catscope decodes
this entire market **in-validator, same slot** — every obligation's live
deposits/borrows and every reserve's live price and liquidation terms —
with zero RPC polling, computing a real-time health factor for every
position simultaneously. This is built and live-verified against the real
market end to end: full discovery of every real obligation (not a sample),
a working health-factor engine, and a live dashboard, tested on mainnet.

This is explicitly framed as an **R&D / risk-infrastructure tool**, not a
finished consumer product — a monitoring engine other protocols, risk
desks, or liquidator bots could build on, not something we're claiming
manages real user funds today. Live execution (a self-triggered
liquidation, proving the engine can act and not just watch) is a stretch
goal for the time remaining, not a claim the core submission depends on —
see §4.

## 2. What we already have (verified against the actual code, not memory)

| Piece | Where | Status |
|---|---|---|
| In-validator account graph (Kamino, Kamino2, Kvault, Orca/Orca2, Raydium amm/clmm/cpmm, Raydium2, Pumpfun/Pumpswap, Sanctum, Meteora DLMM, Jupiter, Safejar, Solpipe, Token/Token-2022, Ondo GM, Byreal) | `edge-generator` (separate repo -- this doc now lives in `catscope-rust-bot`) | Built, unit-tested |
| Kamino `Reserve` account parsing — including `config.loan_to_value_pct` and the adjacent `config.liquidation_threshold_pct`, both live-verified against 8 real mainnet reserves (values came back as plausible round percentages with LTV consistently 5–10 points under liquidation threshold, matching every real lending protocol's LTV-vs-threshold relationship) | `edge-generator/src/kamino.rs` (graph) and `catscope-rust-bot/src/trader/dex/kamino.rs` (bot, byte-offset verified independently) | **Real, mainnet-verified.** This is the field liquidation eligibility is computed from — it's already correct. |
| Kamino `Obligation` parsing (`deposits`/`borrows` arrays) | `catscope-rust-bot/src/trader/dex/kamino.rs` | Real, live-verified — currently tracks only the bot's own obligation |
| Kamino instruction building: refresh reserve/obligation, deposit, withdraw, borrow | `catscope-rust-bot/src/trader/dex/kamino.rs` | Real, tested (bot's own position, deposit→withdraw round-trip) |
| Live xStock/Raydium-CLMM pool support (proves Token-2022 xStock trading end-to-end: real freeze authority / permanent delegate / pausable extensions, ATA program-awareness, CU budgeting all handled) | `catscope-rust-bot/src/trader/dex/tslax.rs` | Real, mainnet-verified against `HHQUnUbmWLrYzkscDY1C3deEFbGtiGBGoHjpANogmvum` (TSLAx/USDC). Treat as **one proof point for the xStock pattern**, not the product — see §7. |
| Live server-push web app for Catscope data — htmgo (server-rendered HTML fragments), SSE for live updates, a `state.Client` service already streaming decoded Catscope state into pages | `catscope-htmx` ("Catscope Explorer") | Exists, but its current pages (market/pipeline) are Solpipe bandwidth-marketplace views, not DeFi/lending — the architecture (live push) is reusable, the content isn't. See §5 and open item 4. |
| Pyth price parsing — legacy `Price` *and* Push Oracle `PriceUpdateV2`, with a working feed-subscription pattern (`PythFeedState`) | `catscope-rust-bot/src/trader/dex/pyth.rs` | Real, mainnet-verified, currently wired to SOL/USD only. Adding a feed is "add a feed-ID constant." |
| Real bugs already hit and fixed in Token-2022 / CLMM trading (tick-array existence assumptions, Token-2022 ATA program-awareness, Token-2022 CU underbudgeting) | see the previous plan revision / git history | Fixed — good evidence for "quality of execution," not directly this idea's path but proof the team ships correct code against these exact asset types |

**What does not exist anywhere yet:** obligation *discovery* beyond the
bot's own position (i.e. scanning the xStocks market for other borrowers'
obligations), any health-factor computation, and a liquidation instruction
builder (`LiquidateObligationAndRedeemReserveCollateralV2` or equivalent —
grepped for, confirmed absent in both repos). That's the real, new work.
Reserve/obligation parsing and the LTV/threshold fields it depends on are
already done and already correct.

## 3. What's genuinely new — three pieces, in dependency order

1. **Identify the xStocks market.** Kamino's xStocks collateral lives in its
   own isolated lending market (not the main SOL/BTC/ETH/USDC market this
   codebase's existing offsets were verified against) — a different
   `lending_market` pubkey with its own Reserve accounts per xStock asset.
   First task: find that market's address and enumerate its reserves (one
   per supported xStock), the same way Byreal's real test pool was found —
   `getProgramAccounts` against Kamino's program, filtered and decoded with
   the already-verified `Reserve` layout, cross-checked against a block
   explorer. This is research, not new plumbing.
2. **Obligation discovery + health-factor engine.** Scan the xStocks
   market for obligations, computing for each: collateral value (from the
   relevant xStock reserve's price + Pyth's live equity/xStock feed) ×
   `liquidation_threshold_pct`, versus outstanding debt value. This is
   arithmetic on fields that are already parsed correctly — the genuinely
   new code is the obligation-enumeration/subscription layer (not just the
   bot's own one obligation) and the health-factor formula itself.
3. **Liquidation execution.** Build the actual liquidation instruction
   (repay debt, seize discounted collateral) — new instruction-building
   work, no existing precedent in either repo to lean on the way swaps did.
   Kamino's IDL/SDK should have the exact account layout; this is the
   highest-uncertainty new-code item in the plan.

## 3a. Research results (open items 1–3, resolved)

All three verified against primary sources (Kamino's own API cross-checked
against raw `getAccountInfo` on mainnet; Pyth's Hermes feed list; Kamino's
own IDL-generated SDK) — not guessed, matching this codebase's own standing
practice.

**Kamino xStocks market.** Two markets exist; the one to build against is
the primary, uncurated one — 10 real xStock reserves, cross-checked
on-chain (owner = `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`, Kamino's
real lending program; account sizes match this codebase's own
already-verified Reserve/LendingMarket layouts):

- **`lendingMarket = 5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua`**
  ("xStocks Market")

| Ticker | Reserve | Mint | maxLtv |
|---|---|---|---|
| SPYx | `UvXjBuC7YZYaGB9Rn1PpBD1GySmjzunXgE8Zev9ua8d` | `XsoCS1TfEyfFhfvj8EtZ528L3CaKBDBRqRapnBbDF2W` | 0.73 |
| QQQx | `2jerdAXR8r2B6z3P7P6VgSiePQX7wqcpbEqdDbm8mgeB` | `Xs8S1uUs1zvS2p7iwtsG3b6fkhpvmwz4GYU3gWAmWHZ` | 0.70 |
| HOODx | `4UBJu5Xp1aziV9frBQBhc1RnKrgXHAWHYejQytkYr8gq` | `XsvNBAYkrDRNhA7wPHQfX3ZUXZyZLdnCQDfHZ56bzpg` | 0.30 |
| GOOGLx | `4wg6rEkGgHaEuxMduP46C1xFZ24Lnp5YgdNkZAHxFzsN` | `XsCPL9dNWBMvFtTmwcCA5v3xWPSMEBCszbQdiLLq6aN` | 0.60 |
| CRCLx | `57qagnQFuWw1seEqi6Z5JBvkm5xH5svdmq9dtqxG1rYy` | `XsueG8BtpquVJX9LVLLEGuViXUungE6WmK5YZ3p3bd1` | 0.30 |
| TSLAx | `5iTiczqgUegqA3PpoNpotizMbY9n1sRWr3oL6igKvWuf` | `XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB` | 0.55 |
| NVDAx | `7B66Az3tJhAo4bLkX8PzTixQ9ZGyHkkjxfVLhF26sP5q` | `Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh` | 0.55 |
| AAPLx | `CKJbqakbPGyhziowm19LPYz636UszuezfkitmpRtcLSH` | `XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp` | 0.40 |
| METAx | `AJPrye7NZGex2rUZhRwiAPJYxai1Ptb7DNWR3yYjtk3G` | `Xsa62P5mvPszXL1krVUnU5ar38bBSVcWAB6fmPCo5Zu` | 0.35 |
| MSTRx | `Cwy2WJoswCMyfPtWTrmiaDLXC3phz3qwr1TaT4kaSAyD` | `XsP7xzNPvEHS1m6qfanPUGjNmdnmsLKEoNAnHjdxxyZ` | 0.30 |

TSLAx's mint matches `tslax.rs`'s already-verified constant exactly — good
cross-check that the API data is current. Borrow-side reserves in the same
market: USDC (`97zoywd8mPZsGTg8q1wdD2Wgkdrs2tqusp1Qqcxbyj7E`), USDG, cbBTC.
`liquidation_threshold_pct` isn't in this API response — no need, it's
already read directly on-chain by the bot's existing Reserve parser.

A second, smaller **"Sentora xStocks Market"**
(`8BNUWRSibVasaAmhYpBCFpGgMisGKfVAf9ho3Cmf6vjr`, curated, only 4 reserves:
SPYx/QQQx/NVDAx/PYUSD) also exists — likely the Kraken-partnered vault
market. Secondary target at best; the primary market above is where the
real volume and the full 10-asset story is.

**Pyth feed IDs**, all confirmed live on Hermes (`asset_type=equity` for
the real-market feed, `asset_type=crypto` for the xStock and — bonus,
directly useful for the Pyth bounty's own named example — Ondo synthetic
feeds):

| Ticker | `Equity.US.*` (real market) | `Crypto.*X/USD` (xStock) | `Crypto.*ON/USD` (Ondo, where it exists) |
|---|---|---|---|
| AAPL | `49f6b65cb1de6b10eaf75e7c03ca029c306d0357e91b5311b175084a5ad55688` | `978e6cc68a119ce066aa830017318563a9ed04ec3a0a6439010fc11296a58675` | `e6734de88a83d9d2fb33072adab319004700aefd069653aba30ba9e3cac056f2` |
| TSLA | `16dad506d7db8da01c87581c87ca897a012a153557d4d578c3b9c9e1bc0632f1` | `47a156470288850a440df3a6ce85a55917b813a19bb5b31128a33a986566a362` | `c09ef687ed07091c047da444f1499f2da52cdc1c085104643ec565a9eb1af514` |
| NVDA | `b1073854ed24cbc755dc527418f52b7d271f6cc967bbf8d8129112b18860a593` | `4244d07890e4610f46bbde67de8f43a4bf8b569eebe904f136b469f148503b7f` | `207ddea2a443d30b7e13a7c88a9e3f106765deb97049afc65a18cede50fffc82` |
| GOOGL | `5a48c03e9b9cb337801073ed9d166817473697efff0d138874e0f6a33d6d5aa6` | `b911b0329028cd0283e4259c33809d62942bd2716a58084e5f31d64c00b5424e` | `ad79b3487bef87ff8f8ab31c0b779ad08d931fdfa5436f7e92a234bb82bff7e4` |
| HOOD | `306736a4035846ba15a3496eed57225b64cc19230a50d14f3ed20fd7219b7849` | `dd49a9ac6df5cbfa9d8fc6371f7ae927a74d5c6763c1c01b4220d70314c647f9` | `8d61af9bd7c39d9503d7d99b7e9e59cc7b0bd707341ecc3ed3e3eda1a411f4de` |
| META | `78a3e3b8e676a8f73c439f5d749737034b139bbbe899ba5775216fba596607fe` | `bf3e5871be3f80ab7a4d1f1fd039145179fb58569e159aee1ccd472868ea5900` | — |
| MSTR | `e1e80251e5f5184f2195008382538e847fafc36f751896889dd3d1b1f6111f09` | `53f95ba4e23ed15ea56083e2ee9a5eec48055d6f59033d4bb95f1ca2a2349c28` | `89a131faf74b5298981e3d25bbce60a25c6f452004da31ddd3c5805cdaa9b6ab` |
| CRCL | `92b8527aabe59ea2b12230f7b532769b133ffb118dfbd48ff676f14b273f1365` | `c13184461c0c80d98ffcd89be627c2220b94a96c7c67f0c4b16bc12fd3b17758` | `25edf380836977c4fea6d1e8df3567192c4e11deeeb566c8e0bd8022068860df` |
| SPY | `19e09bb805456ada3979a7d1cbb4b6d63babc3a0f8e8a9509f68afa5c4c11cd5` | `2817b78438c769357182c04346fddaad1178c82f4048828fe0997c3c64624e14` | — |
| QQQ | `9695e2b96ea7b3859da9ed25b7a46a920a776e2fdae19a7bcfdf2b219230452d` | `178a6f73a5aede9d0d682e86b0047c9f333ed0efe5c6537ca937565219c4054d` | — |

Note: several equity symbols have a second, duplicate-looking feed under
`Equity.Index.*` (Pyth's own description: "PYTH PRICE IN USD FOR X 24/7") —
a synthetic always-on feed, not the real market-hours one. Use
`Equity.US.*` (the ones tabled above) as the reference — the whole thesis
depends on the market-hours gap, so the 24/7 synthetic feed is the wrong
one for this.

**Kamino's liquidation instruction**, from `klend-sdk`'s own IDL-generated
TypeScript (`liquidateObligationAndRedeemReserveCollateralV2.ts` —
authoritative, not reconstructed by hand):

- Discriminator: `[162, 161, 35, 143, 30, 187, 185, 103]`
- Args (Borsh): `liquidityAmount: u64`, `minAcceptableReceivedLiquidityAmount: u64`, `maxAllowedLtvOverridePercent: u64`
- Accounts: 19 fixed (liquidator signer, obligation, lending market +
  authority, repay-side reserve/mint/supply, withdraw-side
  reserve/mint/collateral-mint/collateral-supply/liquidity-supply/fee-receiver,
  user's source-liquidity/destination-collateral/destination-liquidity,
  3 token programs, instructions sysvar) + 4 optional farms accounts
  (default to the program ID itself when unused, per the SDK) + the farms
  program + `remainingAccounts`.
- No existing precedent for this exact instruction in either repo (§2 already
  flagged this as the highest-uncertainty item) — but the shape is now fully
  known rather than needing reverse-engineering, which removes most of that
  uncertainty. Signer/writable roles are visible in the SDK source too;
  worth confirming against `@solana/kit`'s `AccountRole` enum at
  implementation time rather than trusting a quick read of the numbers.

## 4. The demo

Real strangers' obligations may or may not be near their liquidation
threshold when we happen to be recording — we shouldn't depend on market
timing for the demo. Two layers, ship both if time allows, but #1 alone is
already a complete, honest submission:

1. **Live monitor (always shippable):** a dashboard — I'd build this as a
   published Artifact — showing every xStock-collateralized obligation on
   Kamino's xStocks market, live health factors, updating in-validator.
   This alone satisfies "a real problem, a working end-to-end demo" even
   with zero trades fired.
2. **Self-triggered live liquidation (the "real trade" proof):** deposit
   xStock collateral into our own obligation, borrow up close to the
   threshold deliberately (small size — this is a controlled, self-inflicted
   demo, not real risk-taking), then either wait for/force a price move that
   crosses it, and have the engine detect and fire the liquidation live,
   on camera, the instant it's eligible. This is the "in-validator, faster
   than an RPC poller" claim made concrete and controllable — we don't need
   to wait for a real stranger's position to blow up to prove the engine
   works.

## 5. UI/UX

One page, three parts. Everything on it is a direct display of data the
engine (§3) already computes — no separate design/content effort, just
surfacing it.

**5.1 xStock Health Board (the home view)**
- Header strip: total xStock collateral value, total debt, obligations
  tracked live, count currently "at risk."
- Per-asset row, one per xStock reserve (8 total): ticker, on-chain pool
  price, Pyth reference price, divergence in bps, a small live sparkline —
  the market-pulse view, shows breadth of coverage at a glance.
- Obligation table, sorted health-factor ascending (worst first): borrower
  (truncated pubkey), collateral asset + $ value, debt $ value, health
  factor as a color-coded bar (green → yellow → red), last-updated
  timestamp. Rows flash on live (SSE) update — the flash is the point: it's
  visible proof state is moving in real time, not on a poll timer.

**5.2 Live Event Log ("the latency ledger")**
A scrolling, slot-numbered feed:

```
[slot 123456789] Pyth TSLAx feed update → $410.22
[slot 123456789] obligation 7xKp…9F1 health factor → 1.04 (below threshold)
[slot 123456789] liquidation instruction built
[slot 123456791] tx confirmed — liquidation executed
```

The single highest-value part of the UI for judging: printing real slot
numbers from detection to execution turns "we have a latency advantage"
from a claim into something a judge can literally count, using data the
engine already has.

**5.3 Liquidation proof card**
Appears only when a liquidation actually fires: before/after health factor,
collateral seized, repay amount, profit, and a direct Solana Explorer link
to the real signature. This is the screenshot for the submission and the
freeze-frame for the video.

5.1+5.2 are the whole page and don't depend on a liquidation ever firing;
5.3 is additive. So the UI is fully demoable even if §4's stretch goal (live
liquidation) slips — same "ship the sure thing first" principle as §4 and
§7 already apply to the engine itself.

**Hosting: undecided, see open item 4.** Either extend `catscope-htmx`
(reuses its existing SSE/live-push architecture, but content and its
`state.Client` wiring would be new, and it currently depends on an
`eflam`-fork bot package, not the `np`/noncepad one used elsewhere in this
plan) or ship as a standalone page (faster to stand up in isolation, no
integration risk, but doesn't reuse existing infra or read as "part of the
product" the way extending the real Explorer would).

## 6. Bounty mapping

| Bounty | Fit | Why |
|---|---|---|
| **Stocklana main track** ($100k pool) | Core | The whole submission. |
| **Pyth market data** | High confidence | Pyth's equity + xStock feeds are the price input the health-factor computation runs on — not decorative, structurally required. |
| **Meteora DBC / Clawpump** | Weaker fit for this idea, optional | Nothing about a lending/liquidation engine naturally touches a bonding-curve launch. With 8 days instead of 2 it's viable as an independent side-track if there's spare bandwidth, but it shouldn't distort scope or timeline on the core plan. Revisit after §8's core milestones are hit, not before. |
| **PreStocks / Tessera** | Skip | Same reasoning as before — no code or narrative overlap. |

## 7. What NOT to do

- **Don't center the pitch on TSLAx, or on any single ticker.** TSLAx is one
  proof point that the xStock pattern (Token-2022 extensions, pool trading)
  works end-to-end — the product is the engine across all 8 xStock reserves
  on Kamino's xStocks market, not a TSLA-specific tool.
- **Don't build this around `bot/txbuilder` (Go).** Same reasoning as
  before — it's real, tested code for a different bot entirely
  (Solpipe/Safejar treasury), not the in-validator runtime this pitch's
  latency claim depends on.
- **Don't risk real capital for the demo.** The self-triggered liquidation
  leg should use small, deliberately-sized positions — an R&D tool proving
  the mechanism, not a fund.
- **Don't let the liquidation-instruction unknown block the monitor.** Build
  and ship §4.1 (the live health-factor dashboard) first and completely —
  it's a valid, complete submission on its own. Treat §4.2 (live liquidation)
  as the stretch goal it is; if Kamino's liquidation instruction turns out
  to be more involved than expected, the dashboard still stands as a real,
  working, judged submission.

## 8. Timeline (8 days)

**Days 1–2 (17–18 Sep):**
- ~~Identify the xStocks lending market + enumerate its reserves~~ ~~Confirm
  Pyth feed IDs~~ ~~Research Kamino's liquidation instruction~~ — all done,
  see §3a.
- Start the obligation-discovery/subscription layer against the real market
  (`5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua`) and reserves in §3a.
- Land the UI hosting decision (open item 4) so §5 isn't blocked later.

**Days 3–5 (19–21 Sep):**
- Health-factor engine complete, computing real numbers against real live
  obligations.
- Ship the live dashboard (§5) — this is the point at which the submission
  is already complete and defensible.
- Start the liquidation instruction builder using §3a's real account/arg
  shape (no more research needed there, just implementation).

**Days 6–7 (22–23 Sep):**
- Build and test the liquidation instruction builder.
- Run the self-triggered live-liquidation demo, recorded.
- If there's real spare bandwidth: look at Meteora DBC/Clawpump as an
  independent addition — only after the core is done and recorded.

**Day 8 (24–25 Sep):**
- Buffer for the inevitable last-mile issue.
- Write the README / submission text: real user (risk desks, liquidator
  bots, protocols), real problem (market-hours volatility vs continuous
  on-chain monitoring), why Solana (the in-validator latency claim, made
  concrete), quality of execution (point at what's already real and
  verified in §2).
- Register, submit (GitHub + video — video is the strongest option given
  "real trade" is explicitly judged), invite teammates on the submit form.

## 9. Submission checklist

- [ ] Title/short/full description ready to paste — see
      `SUBMISSION_TEXT.md` (this repo)
- [ ] Register on the Stocklana site
- [ ] At least one link: GitHub (this repo + `catscope-rust-bot`), live
      demo, or video
- [ ] Invite teammates from the submit form
- [ ] Disclose open-source components (Pyth SDK, Kamino IDL/SDK, this
      repo's graph work)
- [ ] Submitted before 25 Sep 4:00pm ET (edits allowed until then)

## 10. Open items

Items 1–3 are resolved — see §3a. Items 4–5 block §5 (UI) but not the
engine itself, so they can proceed independently of §3's build.

4. UI hosting: extend `catscope-htmx` or ship standalone (§5) — depends on
   how heavy it'd be to wire a new data source into its existing
   `state.Client`/SSE plumbing, which needs someone who knows that repo to
   weigh in.
5. Which bot-state package is canonical: `catscope-htmx` currently imports
   `gitlab.noncepad.com/eflam/bot/state`, not the `git.noncepad.com/pkg/bot`
   used elsewhere in this plan — confirm whether those are the same thing
   under different remotes, forks that have diverged, or intentionally
   separate, before building the UI against either.
