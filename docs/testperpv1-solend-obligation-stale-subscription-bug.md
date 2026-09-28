# Bug: Solend obligation account subscription goes silent after one push

## Symptom

`testperpv1`'s mainnet smoke test gets through phases 1–10 (Solend/Kamino/
Marginfi deposit+withdraw round trips) and the new TSLAx/Kamino-xStocks legs
fine, then loops forever at `[11/16]` (opening a $1 SOL borrow-hedge position
on Solend), retrying every ~31s indefinitely and burning a real transaction
fee each time:

```
[WARN]  [332.476s] testperpv1: [11/16] opening $1.00 SOL borrow-hedge on solend
[WARN]  [363.418s] testperpv1: [11/16] opening $1.00 SOL borrow-hedge on solend
[WARN]  [394.355s] testperpv1: [11/16] opening $1.00 SOL borrow-hedge on solend
[WARN]  [425.844s] testperpv1: [11/16] opening $1.00 SOL borrow-hedge on solend
...  (repeats every ~31s indefinitely)
```

Confirming the actual transaction on-chain (`solana confirm -v <sig>`) shows
it's really failing, every time, with the same real program error:

```
Program So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo invoke [1]
Program log: Instruction: Refresh Obligation
Program log: Too many obligation deposit or borrow reserves provided
Program log: Invalid account input
Program So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo failed: custom program error: 0xd
```

## First hypothesis (fixed, but wasn't the real cause)

Solend's real `refresh_obligation` instruction requires the number of
"remaining accounts" (reserves) passed in to exactly match the obligation's
*current* on-chain deposit+borrow count. Our code builds that list from a
locally cached copy of the obligation (`o_solend_position`), fed by an
account-update subscription. The working theory was that a **late, out-of-
order "rooted" update** could roll that cache backward — this bot has two
separate account-update streams (a fast "low_latency" one and a slower,
~12s-delayed but finalized "rooted" one), and nothing guarded the rooted
stream from re-applying an older snapshot after a newer one had already
landed (a real, previously-fixed class of bug in this codebase's `arbv1`
mode, via `is_newer_than_low_latency`/`record_low_latency_slot`).

We ported that exact guard into `testperpv1` (see commit). It compiled
clean, and is a legitimate defensive fix worth keeping — but re-running on
mainnet with it in place hit **the exact same error again**, proving it
wasn't the actual cause of this specific failure.

## Real root cause (confirmed live, via added debug logging)

We added temporary instrumentation that logs every time the bot's Solend
obligation observer actually fires, and what decision `open_solend_borrow_leg`
makes from it. A fresh mainnet run showed:

```
[WARN]  [10.096s] testperpv1: [debug] rooted solend obligation update slot=447279892 body_len=1300 deposits=Some(1) borrows=Some(0)
```

— exactly **one** update, delivered at process start when the subscription
was first established. Then, for the rest of that run (we let it run past
1150 seconds / ~19 minutes), the exact same debug line **never fired again**
— not even once — despite this same run submitting a real, live Solend
withdraw transaction of its own in between:

```
[WARN]  [8435] [debug] open_solend_borrow_leg decision has_usdc_collateral=true deposits=Some([ObligationCollateral { deposit_reserve: 810506446, deposited_amount: 764976 }]) borrows=Some([])
[WARN]  [9237] [debug] open_solend_borrow_leg decision has_usdc_collateral=true deposits=Some([ObligationCollateral { deposit_reserve: 810506446, deposited_amount: 764976 }]) borrows=Some([])
```

Every single retry, for the entire run, reads the *identical* cached
`deposited_amount: 764976` — a real leftover USDC collateral deposit from
some earlier successful run of this same borrow-hedge test (repaying a loan
only clears the debt, it never withdraws the collateral that was posted for
it — a separate, known gap, not today's bug).

Meanwhile, direct independent verification against live mainnet RPC — run
three separate times across the investigation, most recently seconds after
the debug line above — shows the **real, current** obligation account is
completely empty:

```
$ solana account AemZeAW6nam7u4ots7Yj9zqPFSZyukaAjjK1xN8srAnt -u https://api.mainnet-beta.solana.com --output json
# decoded: owner=3hqu56Yw1aL4MKdYER8hGnJmZ4Q9EfctvCCB9mWYPP6X
# deposits_len = 0, borrows_len = 0
```

So the bot's in-memory copy of this one account is **frozen at whatever it
was the moment the subscription was first established**, and never receives
another push again — for the rest of the process's lifetime, regardless of
real on-chain activity, including this bot's own transactions touching that
exact account. Every other tracked account (Kamino/Marginfi obligations,
reserves, ATAs, the new xStocks TSLAx reserve/obligation) kept updating
normally in the same run; this is specific to this one Solend obligation
subscription.

**Conclusion:** this is a real subscription/observation-plumbing bug (most
likely in the `Graph`/`SubscriptionQueue` layer or the host-side geyser
delivery for this account), not a logic bug in `testperpv1`'s own
decision-making, and not caused by (or related to) the new TSLAx/Kamino
xStocks work added this session — it's pre-existing, and simply wasn't
visible before because no earlier phase's correctness actually depended on
a *second* update ever arriving for this specific account.

## Status

- The slot-ordering guard (`is_newer_than_low_latency`) is committed — a
  real, independently-justified fix (mirrors `arbv1`'s own precedent),
  just not sufficient on its own here.
- Diagnostic `[debug]` logging is left in place in `state.rs` to make this
  reproducible/traceable without re-instrumenting from scratch.
- Not yet fixed: why the subscription for this one account stops delivering
  after its first push. That's a deeper dig into the subscription layer,
  out of today's scope.
