# Sanctum (INF) never showing up in the trade router — investigation & fix

## TL;DR

Sanctum LST↔LST edges never appeared in the live trade graph (`Sanctum=0` forever
in the `trade router edges by dex` log line), even after `sol_value` was correctly
wired up and Sanctum LSTs were admitted into `ROUTER_POOLS` as graph nodes. Root
cause: `ASSOCIATED_TOKEN_PROGRAM_ID` in `src/trader/dex/sanctum.rs` was a
**corrupted constant** — `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJe1bxr` instead of
the real `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` (same vanity prefix, wrong
tail). Every derived `pool_reserves_pk`/`protocol_fee_acc_pk` for all 126 LSTs
pointed at accounts that don't exist on-chain, so the bot's reserve-vault
subscriptions never delivered a single token event. Fixed by correcting the
constant. This also means `SanctumState::swap()` would have built transactions
with wrong accounts and failed on-chain if it had ever been invoked (it wasn't —
still dry-run-only elsewhere in the bot).

## Background

`TradeRouter` (`src/trader/pricegraph.rs`) is a live in-memory price graph, rebuilt
every commit from each DEX's `Updater::batch_router`. `SanctumState::batch_router`
(`src/trader/dex/sanctum.rs`) adds a directed edge for every LST pair whose
`spot_price()` resolves, which requires **both** LSTs to have a nonzero `reserve`
(live vault balance, from `on_token`) and nonzero `sol_value` (from the
`lst_state_list` account, via `on_account`).

Separately, an LST's mint has to be a registered node in `router::Router` (the
liquidity-tier classifier) for `TradeRouter::from_router` to even consider it —
that node set comes from the build-time `ROUTER_POOLS` snapshot in `build.rs`.

## Investigation timeline

1. **Added a per-DEX edge-count diagnostic.** `trade router edges by dex` in
   `src/brain/arbv1/state.rs`, backed by a new `TradeRouter::edge_counts_by_dex()`
   in `pricegraph.rs`. This was the tool that made the whole investigation
   possible — up to this point the bot only logged a single combined
   `total_edges` number, so there was no way to tell which DEX was (or wasn't)
   contributing.

2. **First finding: Sanctum LSTs were never graph nodes at all.** `build.rs`'s
   `ROUTER_POOLS` snapshot was built only from `raydium_amm_pool`,
   `raydium_cpmm_pool`, `raydium_clmm_pool`, and `orca_whirlpool_pool` — the
   `sanctum_lst` table never contributed a node. Fixed by adding synthetic
   SOL↔LST edges to the same widest-path liquidity graph, using each LST's
   `sol_value` (already the total SOL-denominated value of that LST's pool
   reserves — no raw per-LST reserve is available at build time, so this was
   the right proxy) as the liquidity figure.

3. **Second finding: sequential admission starves the loser.** The first version
   of the fix admitted Raydium/Orca pools into the 5,000-mint `ROUTER_TOKEN_BUDGET`
   *first*, then gave Sanctum whatever budget was left — which turned out to be
   zero (0/91 LSTs admitted), because `prefetch.db` has **602,068** raydium_amm_pool
   rows passing a floor-less `balance > 0` filter. Fixed by merging Sanctum
   candidates into the *same* single ranked admission pass, so everything
   competes purely by real liquidity. Result: 15/91 LSTs admitted.

   This also surfaced a related, **still-unfixed** issue: `Orca=0` too, the whole
   run, despite `orca_whirlpool_updates` climbing into the tens of thousands. A
   temporary build-time diagnostic (`router_pools admitted pools by dex: ...`)
   showed why: `RaydiumAmm=4667 RaydiumCpmm=337 RaydiumClmm=33 Orca=297 Sanctum=15`
   — Raydium AMM's sheer *pool count* (not liquidity quality) crowds out Orca's
   share of the shared mint budget too. Orca only got 297 of its ~7,613 real
   candidate pools registered as nodes, even though this predates any of the
   Sanctum work. **This is a legitimate open bug, not yet fixed** — see
   "Follow-up" below.

4. **Third finding: even with real nodes, still zero edges.** After the admission
   fix, Sanctum edges were *still* 0 after 30+ minutes of runtime. Added two
   one-shot diagnostic log lines to `sanctum.rs` (`sanctum: lst_state_list
   delivered: ...` and `sanctum: first reserve token event: ...`) to see which
   half of `spot_price()`'s two preconditions was failing.

   - `lst_state_list delivered: len=10080 lst_state_size=80 len%size=0
     n_entries=126 known_lsts=126` — **`sol_value` parsing was fine.** Exact
     match, no discriminator/padding issue.
   - `first reserve token event` — **never fired, at all.** Zero reserve-vault
     token events across the entire run.

5. **Root-caused the missing reserve events by decoding a real transaction.**
   Rather than keep guessing at PDA derivation, fetched a real, recent
   `swap_exact_in` transaction against the S Controller program
   (`5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx`) from mainnet via public RPC
   (`getSignaturesForAddress` → `getTransaction`), and inspected its actual
   account list. The real wSOL pool-reserves account
   (`F2AETMoKjZgb3965ee9DiSriVmFDMA9Uf1ebuWuVzjUu`, ~29,142 SOL) is owned by our
   correctly-derived `pool_state` PDA (`AYhux5gJzCoeoc1PoJ1VxwPDe22RwcvpHviLDD1oCGvW`,
   confirmed via `find_program_address([b"state"], S_CONTROLLER_PROGRAM_ID)`),
   but does **not** match the standard ATA(pool_state, TOKEN_PROGRAM, wSOL) address
   our code was deriving.

   Diffed the ATA program ID our code used against the real
   `sanctum-ata-sdk`/`s-controller-lib` source
   ([igneous-labs/sanctum-ata-sdk](https://github.com/igneous-labs/sanctum-ata-sdk),
   `core/src/lib.rs`) and found the mismatch: `...LJe1bxr` (ours) vs. `...LJA8knL`
   (real, and also directly visible as an account in the decoded transaction).
   Re-deriving with the corrected program ID landed exactly on
   `F2AETMoKjZgb3965ee9DiSriVmFDMA9Uf1ebuWuVzjUu` — confirmed root cause.

## The fix

`src/trader/dex/sanctum.rs`:

```rust
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"); // was ...LJe1bxr
```

Grepped the rest of `src/` — this constant wasn't duplicated anywhere else, so the
bug was isolated to this one file.

## Verification methodology (for reuse)

When an on-chain address derivation is suspect, don't guess — decode a real
transaction:

```bash
# Find a recent transaction against the program in question
curl -s https://solana-rpc.publicnode.com -X POST -H "Content-Type: application/json" -d '{
  "jsonrpc":"2.0","id":1,"method":"getSignaturesForAddress",
  "params":["<PROGRAM_ID>",{"limit":5}]
}'

# Pull its full account list (and inner instructions, if it's CPI'd into)
curl -s https://solana-rpc.publicnode.com -X POST -H "Content-Type: application/json" -d '{
  "jsonrpc":"2.0","id":1,"method":"getTransaction",
  "params":["<SIGNATURE>",{"encoding":"jsonParsed","maxSupportedTransactionVersion":0}]
}'
```

Then cross-reference specific addresses with `getAccountInfo` (owner, parsed
SPL-token mint/authority/amount) to identify what each account actually is.
`api.mainnet-beta.solana.com` rate-limits aggressively from this environment;
`solana-rpc.publicnode.com` worked reliably as a fallback.

`solana find-program-derived-address <PROGRAM_ID> pubkey:<A> pubkey:<B> ...` (CLI,
already installed) is the quickest way to test a candidate PDA derivation without
writing throwaway Rust/Python.

## Diagnostics left in the code

Two one-shot (fire-once, no spam) `log_warn!` calls remain in `sanctum.rs`,
gated by `lst_state_list_logged` / `first_reserve_logged` fields on
`SanctumState`:

- `sanctum: lst_state_list delivered: ...`
- `sanctum: first reserve token event: ...`

These are cheap and harmless to leave in — they're useful confirmation the next
time this pipeline is touched — but can be deleted along with their two `bool`
fields if they're judged to be clutter.

A similar temporary diagnostic in `build.rs` (`router_pools admitted pools by
dex: ...`) is **not** one-shot (it prints every build) and is more clearly meant
to be removed once the follow-up issue below is resolved.

## Follow-up (not yet fixed)

`ROUTER_POOLS`' 5,000-mint budget (`ROUTER_TOKEN_BUDGET`) is dominated by sheer
Raydium AMM pool *count*, not quality — the `raydium_amm_pool` query has no
minimum-liquidity floor and 602K+ rows pass it. Confirmed via the build.rs
diagnostic: `RaydiumAmm=4667 RaydiumCpmm=337 RaydiumClmm=33 Orca=297 Sanctum=15`
out of 5,000 total mints. Orca in particular is left with only 297 of ~7,613 real
candidate pools registered as router nodes, which likely explains persistently
low/zero live Orca edges independent of anything Sanctum-related. Worth either:

- adding a minimum-liquidity floor to the Raydium AMM candidate query, or
- giving each DEX its own budget slice instead of one global pool.
