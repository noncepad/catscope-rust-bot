# Plan: real per-transaction block-position estimate via `tx.index`

Supersedes the earlier `write_version`/dependency-bump plan (deleted). That
approach turned out to be both riskier and worse: `write_version` orders
account *writes*, not transactions, and getting it into `catscope-bot` required
a real cross-repo dependency bump plus a validator plugin reload. This plan
needs none of that.

## Why

`Write` (in `testperplatencyv1`/`testperplatencyv1lite`'s native-transfer
latency test) is anchored on `FirstShredReceived` — the earliest evidence a
block exists for a slot. But that says nothing about whether *our* transaction
is in the block yet, or where in its ordering it falls — shreds stream out
continuously as the leader builds the block, so a transaction landing late in
the block's ordering could have really executed much later than
`FirstShredReceived`. Today we have no way to tell; `Write` is a real lower
bound, not a point estimate.

## The signal — already flowing to the guest today, no cross-repo work needed

Solana's real geyser interface exposes `index: usize` on
`ReplicaTransactionInfo` — the transaction's *exact* ordinal position within
its block (not an indirect proxy like `write_version`). Traced end-to-end,
already wired, already running:

- `catscope-geyser/primitive/src/txproc.rs:243` — `catx.index = tx.index as u64;`
  — the real Agave field, captured today.
- `catscope-rust-bot/src/txview.rs:377` —
  `CatscopeTransactionReadWrapper.pub index: u64` — delivered to the guest,
  publicly readable, right now.
- `testperplatencyv1/state.rs`'s `mid_on_tx` (~line 1325) already loops over
  every transaction in every `Event::Transaction` batch, already matches our
  own sent signatures via `self.state.m_sig`, and has `tx.index` sitting in
  scope, currently unused.

So: no `catscope-bot` changes, no `catscope-zerohop` dependency bump, no
validator restart. Everything needed lives in `catscope-rust-bot`, on the
current branch.

## The real implementation wrinkle

`mid_on_tx` only calls `record_native_read(UpdateLane::Transaction, ...)` when
`self.state.o_native_pending` is *still* `Some` and its `sig` matches — i.e.
only when the Transaction lane is the one that *wins* the race. In every real
run so far, Account/LowLatency has won ~100% of the time, which means
`o_native_pending` is already consumed (`.take()`'d) by the time the Transaction
lane's own confirmation for that same signature arrives later. Gating
`tx.index` capture on that same check would mean it almost never fires.

Fix: decouple "does this signature belong to the still-active pending
transfer" (the existing, correctness-critical gate for *who wins the race* —
must not change, it's what the earlier stale-confirmation bug fix depends on)
from "does this signature belong to *any* past native transfer whose sample
should be backfilled with `tx.index`" (a new, separate, persistent lookup that
doesn't care about race-winning or ordering at all).

Concretely: add a small persistent map,
`state.m_native_tx_index: HashMap<Signature, usize>` (signature → index into
`native_samples`), populated inside `record_native_read` at the moment a
sample is pushed (the signature is already available via `pending.sig` before
`pending` is dropped). Then in `mid_on_tx`, for *every* transaction signature
seen — not just the currently-pending one — check this map; on a hit, backfill
`native_samples[i].tx_index = Some(tx.index)`. Remove the map entry once
backfilled so it doesn't grow unbounded over a run.

## The one real approximation left

We're never told "how many transactions were in this block," only our own
transaction's ordinal position in it. Estimate the denominator by tracking,
per slot, the largest `tx.index` observed across *everything* flowing through
`mid_on_tx` for that slot (not just our own transfers) — a real, honest lower
bound, not a guarantee we saw the block's true last transaction. Label
anything derived from it as an estimate (same dotted/dashed estimate-tier
convention already used elsewhere in the latency reports), never as an exact
measurement.

## Plan

### Phase 1 — Capture `tx.index` for every native transfer (both modules)
- Add `state.m_native_tx_index: HashMap<Signature, usize>` to `State`.
- In `record_native_read`, populate it (`signature → native_samples.len()`,
  i.e. the index the new sample is about to land at) right before pushing the
  new `NativeTransferSample`, if `pending.sig` is `Some`.
- Add `tx_index: Option<u64>` to `NativeTransferSample`.
- In `mid_on_tx`'s existing `while let Some((tx, result)) = transaction_list.transaction()`
  loop, for every signature seen (regardless of `is_current_pending`), check
  `state.m_native_tx_index`; on a hit, backfill `tx_index` on that sample and
  remove the map entry.
- Mirror identically in `testperplatencyv1lite`.
- `cargo build --lib`, confirm clean, same warning-baseline discipline as
  every other change this session.

### Phase 2 — Track the per-slot index range
- Alongside `slot_clock`, track the largest `tx.index` seen per slot from
  *any* transaction passing through `mid_on_tx` (not just our own), bounded
  the same way `slot_clock` already is.
- This gives, for any `inclusion_slot`, a real (lower-bound) estimate of how
  many transactions were in that block.

### Phase 3 — Turn `tx_index` + the per-slot range into a real estimate
- For a sample with both `tx_index` and a resolved per-slot max index:
  `fraction = tx_index / slot_max_index`.
- Multiply that fraction by the measured `shred_to_completed` +
  `completed_to_processed` window for that slot to place the transaction's
  real execution instant somewhere inside it, instead of only knowing it's
  "between `FirstShredReceived` and `Processed`."
- Surface this as a new, clearly-labeled estimate column in the decomposition
  report — dotted/dashed styling, matching the existing same-slot estimate
  tiers, never presented as an exact measurement.

### Phase 4 — Validate with a real run
- Same sweep discipline as every other run this session: sweep before,
  launch, watch, sweep after (until the auto-sweep is hardened with a retry —
  see the earlier, separate note on that), confirm balances.
- Check the new estimate against reality where possible (e.g. same-slot
  samples where `write_delay_upper_bound` already exists should be broadly
  consistent with the new fractional estimate).

## Addendum: `send_slot`/`current_slot()` staleness fix (2026-09-08)

Not part of the original tx.index plan, but found while validating it: the
second real run's own data showed `slots_until_inclusion` values that were
flatly inconsistent with real slot cadence (e.g. sample #1: `slots=5` but
`total=362,555µs` -- nowhere near enough real time for 5 slots at ~400ms/slot
each). Root-caused, from user questioning, to `current_slot()` (the source of
`NativePending::send_slot`): it only reads `slot_clock.back()`, which itself
only advances when this guest's own event loop has processed a `SlotStatus`
event -- if that processing lags the real chain tip (most visibly right at
boot, behind a queued subscription/funding burst), `send_slot` reports a
stale number, inflating `slots_until_inclusion` without any real elapsed time
to match it. `write_delay`/`read_delay`/`total_latency` were never affected
(none of them read `send_slot`), but the `slots` column itself was
misleading.

Fixed in both modules: added `State::freshest_account_slot`, updated to the
max `header.slot` seen on *any* account/token update passing through
`low_latency()` (far higher frequency than `SlotStatus`, and each one
already carries a real slot number). `current_slot()` now returns
`max(slot_clock.back(), freshest_account_slot)` -- strictly at least as
fresh as before, only ever tightens the staleness, never introduces a new
one. `current_slot()` is used only for `send_slot`, so this change is fully
isolated from the real Write/Read split. Verified with a clean
`cargo build --lib` (93 warnings, same baseline) in both modules. Not yet
validated against a real run.

## Status

- [x] Confirmed `write_version` was the wrong signal (orders account writes,
      not transactions).
- [x] Confirmed `tx.index` is the right signal, and traced it end-to-end —
      already flowing to the guest today, zero cross-repo work needed.
- [x] Identified the real implementation wrinkle (`mid_on_tx`'s existing gate
      only fires when Transaction lane wins, which is ~never in practice).
- [x] Phase 1 (capture + backfill `tx_index`) — done in both modules, verified
      with a clean `cargo build --lib` (93 warnings, same baseline). Not yet
      validated against a real run (that's Phase 4).
- [x] Phase 2 (per-slot index range tracking) — `SlotTimestamps::max_tx_index`
      added to both modules, updated for *every* transaction `mid_on_tx`
      sees (not just our own transfers) via `record_tx_index_for_slot`,
      same dedup/capacity discipline as `on_slot_status`. Read side
      (`slot_max_tx_index`) added but intentionally unused until Phase 3 --
      marked `#[allow(dead_code)]`. Verified with a clean `cargo build --lib`
      (93 warnings, same baseline).
- [x] Phase 3 (fractional estimate + report column) — new `write_delay_estimate`
      method in both modules: `fraction = tx_index / slot_max_tx_index`,
      multiplied against the real `shred_to_completed + completed_to_processed`
      window and added to `write_delay` to place the transfer's estimated real
      execution instant inside the `FirstShredReceived`->`Processed` span.
      Computed at report time only (not stored on `NativeTransferSample` --
      `tx_index` is backfilled after the sample is pushed, so it isn't known
      until later). `report_native_stats` now logs two new clearly-labeled
      "ESTIMATE" lines (write and read), kept fully separate from the real
      `write_delay`/`read_delay` percentiles, same discipline as
      `write_delay_upper_bound`. Verified with a clean `cargo build --lib`
      (93 warnings, same baseline). Not yet validated against a real run --
      that's Phase 4, and building the actual HTML report column comes out of
      that same run's logs.
- [x] Phase 4 (real-run validation) — real mainnet run, 2026-09-08,
      `optimizer testperp-latency c-wallet-5.json --protocol=native_lite`,
      freshly rebuilt `wasm32-wasip2` release binary (`--working-dir`
      pointed at this repo, not the default hosted wasm). 20/20 native
      transfers confirmed. New estimate resolved for 14/20 samples (the
      other 6 lacked one of: resolved `write_delay`, a backfilled
      `tx_index`, or a resolved per-slot `max_tx_index`):
      - write delay (lower bound, `send->FirstShredReceived`): p50=1653µs
      - write delay ESTIMATE (via tx.index): p50=88053µs p99=295641µs
      - read delay (raw): p50=336796µs
      - read delay ESTIMATE: p50=282332µs p99=398366µs
      Sanity check: estimate p50s sum to ~370µs, consistent with the raw
      total_latency samples in this run (264-559µs range) -- the estimate
      moves a real chunk of time from Read into Write versus the old
      lower-bound-only Write number, exactly what this whole plan set out
      to do. Auto-sweep (`sweep_native_wallets`) queued immediately after
      sample 20/20 and landed on-chain this time -- confirmed via
      `solana confirm -v` on the sweep signature (`Status: Ok`, mothership
      balance 0.079995 -> 0.260771846 SOL). The known sweep reliability gap
      (no retry) remains unfixed but wasn't hit this run.
    - **Real gap found after this run, via user questioning**: the first
      version of Phase 3 only ever logged the two aggregate percentile
      lines above -- there was no way to tell which specific samples
      resolved an estimate, or why one didn't (the user asked directly
      about sample #8 and this genuinely couldn't be answered from the
      logs). Fixed same day: `write_delay_estimate` refactored into
      `tx_index_estimate`, returning `Result<TxIndexEstimate, &'static str>`
      instead of a bare `Option` -- the `Err` names which specific input
      was missing. `report_native_stats` now also logs one line per sample
      (`native transfer N/20 tx.index estimate -- tx_index=... slot_max_tx_index=...
      write_estimate=...µs read_estimate=...` or `-- unresolved (<reason>)`),
      keyed by the same `N/TARGET` ordinal every sample already gets in its
      `record_native_read` confirmation line -- so a future artifact build
      can join per-sample estimates onto the existing per-row table instead
      of only showing an aggregate callout. Verified with a clean
      `cargo build --lib` (93 warnings, same baseline) in both modules.
      Not yet validated against a real run with this per-sample logging in
      place -- that requires another real mainnet run.
