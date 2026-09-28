# Plan: add Lavarage (leveraged margin trading) to testperpv1

Status: fully spec'd and real-transaction-verified; not yet implemented.
Blocked on one real architectural decision (see "The blocker" below)
before writing code.

## What Lavarage is

A leveraged spot-margin-trading protocol (`github.com/pinedefi/lavarage-sdk`,
IDL at `idl/lavaragev2.ts`) -- different in kind from Solend/Kamino/
marginfi (general money markets). LP operators fund per-token "trading
pools"; traders open a single-token leveraged position against a pool in
one instruction. Two on-chain program deployments exist:

- **V1** `CRSeeBqjDnm3UPefJ9gxrtngTsnQRhEJiTA345Q83X3v` -- SOL-quote only,
  matches `idl/lavarage.ts`. 1,415 real pools, 115-byte `Pool` accounts.
- **V2** `1avaAUcjccXCjSZzwUvB2gS3DzkkieV2Mw8CjdN65uu` -- multi-quote
  (SOL, USDC, a few others via a per-pool `qt_type` field), matches
  `idl/lavaragev2.ts`. 4,054 real pools, 163-byte `Pool` accounts. **This
  is the one to build against** -- bigger, more active, handles both
  SOL and USDC in one deployment.

Neither program ID is published anywhere in Lavarage's docs or the SDK
itself -- both were found by pulling `app.lavarage.xyz`'s live JS bundle
and grepping for `programId:"..."` near known mint constants, then
independently confirmed on-chain (real, executable BPF programs) and
cross-validated by decoding real accounts.

## The public IDL is stale -- everything below is empirically verified

Both `Pool` and `Position` account layouts, and all three instructions'
real account lists/args, were verified against **real, successful mainnet
transactions and real on-chain accounts** -- not just read off the IDL,
which has measurably drifted from what's actually deployed (see each
section). Methodology: fetch every real `Pool`/`Position` account via
`getProgramAccounts` + the Anchor discriminator, then use per-byte-offset
entropy analysis across thousands of real accounts (pubkey fields show
~256 distinct values at every byte position; numeric fields taper to
all-zero at their high-order bytes) to find true field boundaries,
cross-checked against known real values (e.g. a decoded `node_wallet`
independently matched a value found in the app's own frontend bundle).

### `Pool` account layout (V2, 163 bytes, verified against 4,054 real accounts)

```
offset 0-7    discriminator (Anchor sha256("account:Pool")[:8] = f19a6d0411b16dbc)
offset 8      interest_rate (u8)
offset 9-40   collateral_type (Pubkey, 32B)
offset 41-48  max_borrow (u64 LE)
offset 49-80  node_wallet (Pubkey, 32B)
offset 81-88  max_exposure (u64 LE)
offset 89-96  current_exposure (u64 LE)
offset 97-128 qt_type (Pubkey, 32B) -- the quote/borrow currency, PER-POOL:
              2,068 pools = SOL, 1,964 = USDC, plus a handful of others (e.g. cbBTC)
offset 129-160 a per-pool-unique pubkey, full entropy, never a real funded
               account -- almost certainly the "randomAccountAsId" nonce
               tradingOpenBorrow's accounts reference
offset 161-162 2 bytes, unidentified (low entropy, minor -- not required
               for open/close instruction building)
```
(V1, 115 bytes: identical through offset 96 (`current_exposure`); has no
`qt_type`/trailing fields at all -- quote currency is implicit, always SOL.)

### `Position` account layout (V2, 178 bytes, verified against 16,084 real accounts)

```
offset 0-7     discriminator (sha256("account:Position")[:8])
offset 8-39    pool (Pubkey, 32B)
offset 40-47   close_status_recall_timestamp (u64 LE)
offset 48-55   amount (u64 LE)              -- == positionSize arg at open
offset 56-63   user_paid (u64 LE)           -- == userPays arg at open
offset 64-71   collateral_amount (u64 LE)
offset 72-79   timestamp (i64 LE)
offset 80-111  trader (Pubkey, 32B)
offset 112-143 seed (Pubkey, 32B)           -- == randomAccountAsId at open
offset 144-151 close_timestamp (i64 LE)
offset 152-159 closing_position_size (u64 LE)
offset 160     interest_rate (u8)
offset 161-168 last_interest_collect (i64 LE)
offset 169-177 (9 bytes, always zero across every sample -- reserved/unused)
```
Matches the IDL's 11 declared fields exactly, in order, plus 9 trailing
reserved bytes the IDL doesn't document.

### Position PDA

`findProgramAddress(["position", trader_pubkey, pool_pubkey, seed_pubkey], V2_program_id)`
-- confirmed against the SDK's `getPositionAccountPDA` (`index.ts`).
`seed_pubkey` is a **fresh random keypair generated per position**, not a
derived or reused value (confirmed live: `Position.seed` exactly equals
the `randomAccountAsId` account passed into `tradingOpenBorrow`). In this
bot, get one via the WIT host's `transactionprocessor::keygen()` (returns
a fresh `AccountId`/pubkey the host controls) -- unused anywhere else in
this codebase today, would be a first.

### Instructions (all three verified against real, successful mainnet transactions)

**`tradingOpenBorrow`** -- discriminator `[53,88,1,144,58,65,182,60]`
(`sha256("global:trading_open_borrow")[:8]`, confirmed byte-for-byte
against a real transaction). 18 accounts, verified 1:1 against the IDL's
declared order/names (no drift) -- `positionAccount`(w) `trader`(w,signer)
`tradingPool`(w) `nodeWallet`(w) `instructions`(r) `systemProgram`(r)
`clock`(r) `randomAccountAsId`(r, **not** a signer) `feeTokenAccount`(w)
`fromTokenAccount`(w) `toTokenAccount`(w) `tokenProgram`(r)
`positionTokenAccount`(w) `collateralTokenProgram`(r)
`associatedTokenProgram`(r) `collateralMint`(r) `quoteMint`(r)
`feeRecipient`(r, always `6JfTobDvwuwZxZP6FR5JPmjdvQ4h4MovkEVH2FPsMSrF`
in every real example seen).

Args (real deployed signature has grown beyond the IDL's declared 2):
`positionSize: u64, userPays: u64, unknown: Option<u64>, trailing: u8`.
Two real examples decoded: one with `unknown=Some(10000)`, one with
`unknown=None` -- **use `None` (a 1-byte `0x00` tag) for a minimal test,
it's a proven, real, successfully-executed encoding**; the `trailing` u8
was `0x00` in both. `positionSize`/`userPays` land exactly in
`Position.amount`/`Position.user_paid` respectively -- confirmed by
decoding the resulting Position account after a real transaction.

**`tradingCloseBorrowCollateral`** -- discriminator
`[160,104,113,179,42,80,8,16]`. 12 accounts, verified 1:1 against the IDL
(no drift): `positionAccount`(w) `trader`(w,signer) `tradingPool`(w)
`instructions`(r) `systemProgram`(r) `mint`(r, **collateral** mint)
`fromTokenAccount`(w) `toTokenAccount`(w) `tokenProgram`(r, may be
Token-2022 `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` depending on the
collateral mint's own token standard) `clock`(r) `randomAccountAsId`(r,
same value as at open) `associatedTokenProgram`(r). Zero args.

**`tradingCloseRepaySol`** -- discriminator `[74,90,5,111,35,171,135,56]`.
Real account list has **2 more accounts than the IDL declares** (14, not
12): the first 12 match the IDL 1:1 (`positionAccount`(w) `trader`
(w,signer) `tradingPool`(w) `nodeWallet`(w) `systemProgram`(r) `clock`(r)
`randomAccountAsId`(r) `mint`(r, **quote** mint -- SOL/USDC/etc, distinct
from the collateral mint above) `fromTokenAccount`(w) `toTokenAccount`(w)
`feeTokenAccount`(w) `tokenProgram`(r)), followed by 2 more:
`B2qRnMny4fwzPidibptciHbsMjhvREPquAiSjX4WbFjR`(w) and
`63maV9RL2i8Ydt259qyV9Vu9QjNoxFdzo48bkdGe4Las`(r, doesn't exist on-chain).
**Confirmed constant across 5 real transactions spanning 3 different
traders and 3 different pools** -- not per-trader/referral accounts as
first suspected, just fixed protocol-level addresses; safe to hardcode.
Args: `closingPositionSize: u64, closeType: u64, trailing: 2 bytes` --
use `0x00,0x00` for the trailing bytes (matches the same pattern as
`tradingOpenBorrow`'s trailing byte).

Real close flow is two Lavarage instructions with something in between:
`TradingCloseBorrowCollateral` → (external swap, see below) →
`TradingCloseRepaySol`, all in one transaction.

## The blocker

A **real** (non-zero) `tradingOpenBorrow` needs a genuine swap from quote
currency into the collateral token, to actually fund the leveraged
position. Traced via a real transaction's full CPI log: this swap is a
**separate, top-level Jupiter aggregator instruction**
(`JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4`, `invoke [1]` -- not a CPI
made by the Lavarage program itself) sitting between `TradingOpenBorrow`
and `TradingOpenAddCollateral`. Building that instruction requires
calling Jupiter's off-chain quote/swap HTTP API. The real close flow has
the same shape (a `Route`/`Sell` swap between the two close instructions).

`catscope-rust-bot`'s WASM guest has **no networking capability at all**
(checked `wit/component.wit`: only `transactionprocessor`, `shooter`,
`general` host interfaces exist -- no HTTP/outbound networking of any
kind). This is the exact same architectural wall as this session's
Switchboard-oracle-cranking investigation (see
`optimizer/prefetch/marginfi/switchboard-crank-plan.md`) -- the WASM
sandbox structurally cannot make the off-chain call a real trade needs.

A `tradingOpenBorrow` with `userPays=0, positionSize=0` does **not** hit
this wall (confirmed: a real transaction using exactly this had no
Jupiter instruction at all, just plain bookkeeping) -- it creates a real
Position account and exercises the real instruction/account plumbing
end-to-end, but isn't a real leveraged position.

## Options, not yet decided between

1. **Zero-notional open/close.** `positionSize=0, userPays=0` at open.
   Real instructions, real accounts, real Position account created and
   later closed -- but no actual leveraged exposure, no swap needed, no
   networking blocker. Matches testperpv1's own precedent (a smoke test
   of the integration surface, not a trading strategy).
2. **Bridge the swap through `optimizer` (Go)**, which already has real
   network access -- fetch a real Jupiter route/instruction there and
   pass it down to the WASM bot to include in the transaction, mirroring
   how `eval.go`'s boot transfer already bridges a capability (real SOL
   transfer) the WASM guest can't originate on its own. More plumbing
   across both repos; enables a real leveraged position.
3. Do nothing further until one of the above is chosen.

## What's needed to implement option 1 (fastest path)

- `src/trader/dex/lavarage.rs` (new): `Pool`/`Position` parsers per the
  verified layouts above, `position_pda()`, and instruction builders for
  `open_borrow`/`close_borrow_collateral`/`close_repay_sol` using the
  verified account lists/args -- mirroring `src/trader/dex/solend.rs`'s
  conventions (`resolve(AccountId) -> Result<Pubkey, TraderError>`,
  `Instruction { program_id, accounts: vec![AccountMeta::...], data }`,
  `wallet.append_ix(ix, compute_units)`, `wallet.require_signer(...)`).
- A real, currently-live pool to target -- e.g. V2 USDC-quote
  `FKT4n59C1VHeNqzdiKpcoMGRCXxaCKn2EmKJ9r4nsBow` (~$6,692 real exposure
  at time of writing) or V2 SOL-quote
  `Ey5wpVFjXtTBiWg4bhrhDUhrvgRvxyjLpPwAccG5Tn7u` (~26.6 SOL) -- re-verify
  it's still live before using, pool state changes over time.
- Two new `TestPhase` variants in `src/brain/testperpv1/state.rs`
  (`OpenLavarage`/`CloseLavarage`), following the existing
  Bootstrap/Deposit/Withdraw phase-pair convention, each gated on
  `test_cooldown_active()` the same way every other phase is.
- `transactionprocessor::keygen()` wrapper for the position seed --
  no existing call site in this codebase to copy from, would be new.

## What's additionally needed for option 2

Everything in option 1, plus:
- A Go-side Jupiter quote/swap-instruction fetch in `optimizer` (new).
- A wire path to hand that built instruction (or its raw bytes) down to
  the WASM guest to include in the same transaction as the Lavarage
  instructions -- no existing precedent in this codebase for the Go side
  handing a *pre-built instruction* to the WASM bot (the boot-transfer
  precedent hands down a *value*, not an instruction); would need new
  `wit/component.wit` surface or a different bridging mechanism.
