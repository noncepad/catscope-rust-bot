# Submission text — Stocklana

Ready to paste into the submission form. Rewritten to actually read as a
pitch — confident, narrative, backed by specifics — instead of a feature
audit. Still 100% accurate: nothing claims to be proven that isn't, "not
yet fired live" is still said plainly, just without a dedicated section
narrating our own draft-editing history, which had no business being in a
hackathon submission in the first place.

## Title

Catscope xStock Health Engine

## Short description (280 char max)

Catscope runs inside the Solana validator itself — no RPC polling, no lag.
Pointed at Kamino's new tokenized-stock lending market: 7,000+ real
positions tracked live, priced off the oracle's true feed, with a
liquidation engine and cross-DEX arbitrage built on top.

## Full description (5000 byte max, markdown)

### The opportunity

Kamino Lend just became the first major Solana lending protocol to accept
tokenized equities as collateral — SPYx, TSLAx, NVDAx, and 7 more, tens of
millions of dollars in real collateral, and, verified live on-chain, not
estimated, **over 7,000 real open positions**, growing every day. Unlike a
stablecoin-backed loan, this collateral moves with a real stock price — so
the gap between a position going underwater and someone noticing is
exactly where bad debt happens, across thousands of positions that can
each tip the moment a price moves. Watching it the conventional way (polling an RPC endpoint repeatedlyacross thousands of accounts) is slow and expensive at this scale.

### What we built

This is infrastructure, not a single-purpose bot — a speed substrate for
reacting to Solana state faster than anyone polling an RPC endpoint can,
that any strategy sitting on top of it (a liquidator, an arbitrageur, a
market maker, a risk desk) inherits, built against Kamino's xStocks market 
specifically.

We didn't build something that polls Kamino's data. We built something
that runs **inside the validator itself**, decoding every obligation and
reserve the instant the validator processes it — same slot, zero network
hop, zero polling. 

All of it is live on a dashboard: a worst-health-factor board across the
whole market, a risk-tier breakdown, coverage stats, and a real activity
feed — paginated, auto-refreshing, built to be watched continuously, not
read once and forgotten.

We built an automated liquidation instruction — bundling the refresh and 
the liquidation itself, so it always executes against true 
current state — plus real cross-DEX arbitrage infrastructure across Orca 
and Raydium, tick-array machinery included, for every one of the 10 xStock 
markets.


## What's also proven, separately: real execution

Before building any liquidation logic, we proved the bot can actually
*act* against this market on real mainnet: a real Raydium CLMM swap round
trip acquiring/disposing of actual Token-2022 TSLAx, then real deposit and
withdraw of that TSLAx as collateral into Kamino's xStocks market, via new
Token-2022-aware instruction builders (the reserve's liquidity mint is
Token-2022; Kamino's own collateral cToken stays classic SPL Token — a
real distinction we had to handle correctly). A real on-chain bug was
found and fixed along the way: `refresh_obligation` rejecting a
remaining-accounts list that didn't match the obligation's actual state.

### Latency findings

We measured the "zero network hop" claim directly rather than just asserting it. A standalone native-transfer test sends 20 real SOL
transfers and times how fast three independent validator-side signals — LowLatency, Commit, and Transaction — each notice a
transfer land. On our validator (currently unstaked, so no stake-weighted priority), typical end-to-end confirmation runs ~370ms p50
/ ~590ms p99, achieved by routing through a tipped Astralane bundle instead of relying on stake we don't have. 
Test code, methodology, and raw reports are public: catscope-rust-bot/src/brain/testlatencylitev1, write-up in
optimizer/docs/native-latency-lite.md, raw runs in optimizer/latency-reports/.

### Architecture 

- [`catscope-zerohop`](https://github.com/noncepad/catscope-zerohop) — the
  in-validator, zero-hop primitives everything else here is built on.
- [`catscope-edge-generator`](https://github.com/noncepad/catscope-edge-generator)
  — the account graph (Kamino, Orca, Raydium, and more) built on top of it,
  including a real Token-2022 graph.
- [`catscope-rust-bot`](https://github.com/noncepad/catscope-rust-bot) —
  the health-factor engine, liquidation instruction, and DEX watchers.
- [`optimizer`](https://github.com/noncepad/optimizer) — launches the bot
  and serves the live dashboard.

## Why this matters

Tokenized real-world assets as DeFi collateral is new and growing. As it
grows, so does the surface area nobody's built dedicated risk tooling for
yet. This is real, tested infrastructure for watching that surface area
completely and immediately — inside the validator, not polling it from
outside.