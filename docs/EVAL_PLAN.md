# Eval Pipeline Plan

## The Pipeline Today

```
on_account(LstStateList)
  → check_update_fees(config, wallet)        ← state.rs:164
    → eval_fee_update(config)                ← state.rs:178 (WORKS: returns FeeUpdateAction)
    → Decision::Nothing                      ← state.rs:173 (STUB: always nothing)
    → (dead end)

eval()                                       ← bot.rs:81
  → grabs config, wallet, state
  → wallet.assemble() + send                 ← WORKS, but nothing is ever in the queue
```

Two halves exist but they're not connected. `check_update_fees` computes the answer but doesn't
produce instructions. `eval` sends instructions but nobody feeds them in.

---

## What Needs to Happen

The gap is in `check_update_fees` at `state.rs:173`. Here's the full chain:

```
eval_fee_update(config) → Some(FeeUpdateAction)
                                    │
                                    ▼
              Decision::AdjustFee(input_fee, target_fee)
                                    │
                                    ▼
    decision::process(decision, flatslab_config, sol_mint, ...)
                                    │
                                    ▼
                          produces (Instruction, CU)
                                    │
                                    ▼
                      wallet.append_ix(ix, cu)
                                    │
                                    ▼
                        wallet.assemble() + send
```

All the pieces already exist. `decision::process()` (`decision.rs:23-40`) is fully implemented —
give it `Decision::AdjustFee(inp, out)` and it calls `ix_slab_adjustment`, which builds the
complete 9-byte `SetLstFee` instruction with all 5 accounts.

---

## Three Things to Fill In

### 1. `check_update_fees` → produce the Decision and instructions (`state.rs:164-174`)

Currently:
```rust
let inp_fee = self.curr_input_fee.unwrap_or(FeeNanos::ZERO);
let decision = Decision::Nothing;  // ← always nothing
```

Should become:
```rust
let decision = Decision::AdjustFee(inp_fee, action.target_fee_nanos);
```

Then call `decision::process()` to get the instruction, and call `wallet.append_ix()` directly
(you already have `wallet: &mut Wallet` in scope here).

### 2. `eval()` needs to actually call state logic (`bot.rs:81-99`)

Right now `eval()` doesn't talk to state at all. Two approaches:

- **Option A** (keep current separation): `check_update_fees` feeds instructions into the wallet,
  and `eval()` just does `assemble()` + send (it already does this). Call `eval()` after account
  processing — either at the end of `on_account`, or in `CommitHook::finish()`.

- **Option B** (centralize): Move the decision logic into `eval()` itself. Have `eval()` read
  `state.sol_weight` and `state.curr_output_fee`, compute the decision, build instructions, and
  send. `check_update_fees` would just be a helper or go away.

**Option A** keeps the current separation (state computes, eval sends). **Option B** centralizes
everything.

### 3. Fix the bug on line 84 (`bot.rs:84`)

```rust
let state = unsafe { &mut *self.rc_wallet.get() };  // ← wrong Rc
```
Should be `self.rc_state.get()`.

---

## One Prerequisite: `config.flatslab`

`decision::process()` needs a `FlatSlabConfiguration` (admin, payer, slab `AccountId`s). That
comes from `config.flatslab`, which is `Option<FlatSlabConfiguration>`. It's `None` by default.
The host pushes this via `MessageAction::AdjustConfiguration` — so the `Configuration` the host
sends needs to have `flatslab` populated. You'll need a guard:

```rust
let flatslab = match &config.flatslab {
    Some(f) => f,
    None => return,  // can't send without flatslab config
};
```

---

## Summary: Minimal Path

1. Fix `bot.rs:84` (`rc_wallet` → `rc_state`)
2. In `check_update_fees`: replace `Decision::Nothing` with `Decision::AdjustFee(inp_fee, action.target_fee_nanos)`, call `decision::process()`, call `wallet.append_ix()` for each resulting instruction
3. Guard on `config.flatslab.is_some()`
4. Call `self.eval()` after `check_update_fees` succeeds (or call it from `finish()`)

The `InstructionBuilder` on `StateV1` can be your staging area if you want state to stay pure and
not touch the wallet directly, but the simplest path is to go straight to `wallet.append_ix()`
since you already have `&mut Wallet` in `check_update_fees`.
