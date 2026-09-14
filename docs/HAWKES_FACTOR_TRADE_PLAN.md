# Hawkes-on-eigenfactor momentum trade — design

## Motivation

Every existing trade type in this design bets on **reversion**: pair trades a
spike back toward its basket, directional/dispersion bet a factor-loading
position back toward zero. This trade type bets the opposite — that a
cluster of jumps on names sharing an eigenvector **predicts more jumps in
the same direction**, for a short window, before it decays. It targets
illiquid tokens specifically: a single illiquid token's jump is noise, but
jumps pooled across many names sharing a factor are a higher-SNR signal,
the same reasoning that already justifies basket sizing over single-leg
sizing elsewhere in this codebase.

This is a new trade type (5), separate from pair/directional/dispersion/
arbitrage, reusing existing factor/basket/sizing infrastructure rather than
duplicating it.

## Why discrete-time, not textbook continuous Hawkes

Textbook Hawkes needs a real point-process event log (exact jump
timestamps) and MLE fitting. This bot's factor resync cadence is already
effectively fixed-interval (~30s between cycles, observed directly in live
verification). Fighting that to recover continuous time is wasted
complexity — instead this models a **discrete-time self-exciting
intensity** (a Poisson-autoregression / INGARCH-style recursion), the
standard discretization of Hawkes when observations arrive on a regular
grid:

```
λ_t = μ + α · λ_{t-1} + β · event_magnitude_{t-1}
```

`λ_t` is this cycle's expected jump intensity for one factor; `μ` is
baseline; `α` controls how much last cycle's elevated intensity persists
(decay); `β` controls how strongly an actual observed jump excites next-
cycle intensity. This updates in O(1) per cycle — no event-history buffer
needed, which matters because the whole motivation here is illiquid names
with sparse real data.

## Event definition and factor-level pooling

1. Every resync cycle, for each curated symbol already tracked in
   `m_residual_history`, compute `Δz_i = current_zscore_i -
   previous_zscore_i` (a one-cycle diff of data already computed — no new
   data source).
2. For each of the `n_factors` (3) eigenvectors already computed for
   directional loading, project:
   `factor_jump_magnitude = Σ_i loadings[factor][i] · |Δz_i|`,
   restricted to symbols with non-trivial loading on that factor (same
   loading data directional already reads — no new computation, just a new
   reduction over it).
3. Feed that scalar into the factor's own `λ_t` recursion above. Three
   independent intensity trackers, one per factor, each a tiny piece of
   state (`μ, α, β, λ_prev`).

## Entry trigger

Fire when `λ_t` clears a threshold measured in the factor's own historical
intensity distribution — e.g. `λ_t > μ + k·σ_λ` (mirrors the existing
`total_loading >= 0.05` and pair's `±2.5σ` pattern: a z-scored gate on a
rolling stat, not a hardcoded absolute). Direction is signed: take the
position in the direction of the sign-weighted average `Δz` across the
factor's top-loaded symbols this cycle (momentum, so same sign as the
jump, not opposite).

Basket selection mirrors `select_pair_basket_members` but inverted — rank
by **highest** absolute loading on the triggered factor (you want the
names most responsible for the jump, not the calmest ones), the same shape
as dispersion's existing highest-stdev ranking. Sizing reuses
`size_basket_exact` unmodified.

## Exit trigger

Two independent conditions, whichever fires first:
- **Intensity decay**: `λ_t` has fallen back within some band of `μ` (the
  self-exciting effect has worn off — the thesis was "more jumps coming
  soon," and soon has passed).
- **Max holding period**: a hard cap, since without decay data yet, an
  untested `α`/`β` calibration could keep a position "not yet decayed"
  indefinitely — analogous to the borrow-cost cap acting as a hard stop
  elsewhere.
- (Optional stop-loss, same shape as directional's, if the move reverses
  hard instead of continuing.)

## New files/state (mirrors existing pattern)

- `src/trader/hawkes_factor.rs` (new pure module, like `pair_basket.rs`/
  `factor_basket.rs`): `FactorIntensityState { mu, alpha, beta, lambda }`,
  `update_intensity(...)`, `factor_jump_basket(...)`, unit tests on the
  recursion and basket selection in isolation — no live dependency, same as
  the others.
- `state.rs`: `m_factor_intensity: [FactorIntensityState; N_FACTORS]`,
  updated once per resync inside the existing residual-history loop (no
  new subscription, no new data feed); `run_hawkes_trade_cycle()` following
  the same open-pass/close-pass shape as pair/directional; a reservation
  increment against `available_usdc_value()` like the other three trade
  types.
- `factor_borrow_gate.rs`: not needed — this is a spot-basket momentum
  trade, no borrow leg (simpler than directional).
- Go CLI (`optimizer/cmd/multimodel.go`): `--enable-hawkes-trading`, same
  independent-`if` treatment as the other three flags.

## Open questions (revisit before wiring into `state.rs`)

1. **Calibrating `μ, α, β` with zero event history**: start with
   conservative fixed placeholders (e.g. `α` small enough that intensity
   decays within a few cycles) and tune from live-observed `λ_t` behavior,
   the same way the pair trade's 2.5σ threshold got tuned from real data —
   not a blind MLE fit day one.
2. **Which factor(s) to target**: all 3, or start with just the factor
   ETH/illiquid names load most heavily on, to keep the first live
   verification narrow?
3. **Overlap with pair trading**: a strong factor-level jump could
   simultaneously look like a pair spike-candidate (reversion) and a
   Hawkes candidate (momentum) on the same symbols in the same cycle,
   opposite directions — needs a same-symbol conflict check before opening
   either, the same category of problem the USDC-reservation fix solved
   for capital, not signal conflicts.

## Implementation phases

1. `hawkes_factor.rs` pure module (intensity recursion + basket selection)
   with unit tests — no `state.rs` wiring yet.
2. Wire per-cycle intensity update into `state.rs` (background loop,
   mirrors `m_pair_spread_history`'s wiring) — logging only, no trading
   yet, to observe real `λ_t` behavior live before opening real positions.
3. `run_hawkes_trade_cycle()` open/close logic + reservation wiring.
4. Go CLI flag + trigger dispatch.
5. Compile, test, build wasm, redeploy live, verify.
