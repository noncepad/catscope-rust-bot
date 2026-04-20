# Catscope Rust Bot - Comprehensive Walkthrough

## Table of Contents

1. [Big Picture](#1-big-picture)
2. [Architecture Overview](#2-architecture-overview)
3. [The Host/Guest Boundary (WIT)](#3-the-hostguest-boundary-wit)
4. [File-by-File Walkthrough](#4-file-by-file-walkthrough)
5. [Data Flow: End to End](#5-data-flow-end-to-end)
6. [The State Machine](#6-the-state-machine)
7. [What Is Stagnant vs. Adjustable](#7-what-is-stagnant-vs-adjustable)
8. [Rust Basics for This Codebase](#8-rust-basics-for-this-codebase)
9. [Quick Reference](#9-quick-reference)

---

## 1. Big Picture

This is a **Solana bot** that runs inside a **WebAssembly (WASM) sandbox**. Its job:

> Monitor the Sanctum INF liquid staking pool on Solana. When the pool's SOL
> composition drifts, automatically adjust the swap fee by sending an on-chain
> transaction.

The key formula:

```
target_fee = slope * sol_weight + intercept
```

If the current on-chain fee differs from the target by more than a threshold
percentage, the bot sends a `SetLstFee` transaction to the FlatSlab program.

### The Players

| Name | What it is |
|------|-----------|
| **Catscope Host** | A **Solana validator plugin** (geyser plugin) that runs inside the Agave validator process. It syncs account data directly from the validator with zero network hops via the **zerohop interface**, and runs your bot as a WASM guest inside its sandbox. Think of it as "the operating system" for your bot — but one that lives inside the validator itself. |
| **This Bot (Guest)** | A WASM component that receives account updates and decides whether to send transactions. It cannot talk to the network directly -- it asks the host. |
| **Mothership** | An external orchestration process that communicates with bots via stdin/stdout. The bot has a message protocol (`stdio.rs`) for this, but the mothership itself is NOT implemented in this repo. **TODO:** The mothership should be able to manage multiple bots deployed across different hosts — listening to all of them and using their output to coordinate. Currently only the bot side of the stdin/stdout pipe exists (receiving `"flatslab"` config, sending messages back via `write_message()`). The mothership process that sits on the other end of these pipes needs to be built. |
| **Sanctum INF Pool** | A Solana program (infinity pool) that holds multiple liquid staking tokens (LSTs) plus native SOL. |
| **FlatSlab** | A Sanctum pricing program. It stores per-LST fees in a "slab" account. This bot reads and writes to that slab. |

---

## 2. Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────┐
│              Agave Validator Process (Solana Validator)               │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │  Validator Runtime                                             │  │
│  │  (consensus, block production, account storage, etc.)          │  │
│  │                                                                │  │
│  │  On-chain accounts:                                            │  │
│  │  ┌──────────┐  ┌──────────────┐  ┌───────────────────┐        │  │
│  │  │PoolState │  │LstStateList  │  │FlatSlab Slab Acct │        │  │
│  │  │(PDA)     │  │(PDA)         │  │(PDA)              │        │  │
│  │  └──────────┘  └──────────────┘  └───────────────────┘        │  │
│  └────────────────────────┬───────────────────────────────────────┘  │
│                           │ Geyser plugin API                        │
│                           │ (in-process, zero network hops)          │
│                           ▼                                          │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │            Catscope Host (Geyser Plugin)                       │  │
│  │                                                                │  │
│  │  - Syncs account data via geyser plugin interface (zerohop)    │  │
│  │  - Runs bots as WASM components in a sandbox                   │  │
│  │  - Provides the WIT interface (functions the bot calls)        │  │
│  │  - Delivers account updates to the bot                         │  │
│  │  - Sends transactions on behalf of the bot                     │  │
│  │  - Provides stdin/stdout pipes for external communication      │  │
│  │                                                                │  │
│  │  ┌──────────────────────────────────────────────────────────┐  │  │
│  │  │          WASM Sandbox                                    │  │  │
│  │  │                                                          │  │  │
│  │  │  ┌───────────────────────────┐                           │  │  │
│  │  │  │   This Bot (WASM Guest)   │                           │  │  │
│  │  │  │                           │                           │  │  │
│  │  │  │  Communicates with host   │                           │  │  │
│  │  │  │  via WIT imports/exports  │                           │  │  │
│  │  │  └───────────────────────────┘                           │  │  │
│  │  └──────────────────────────────────────────────────────────┘  │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                              │ stdin/stdout pipes
                              │ (message protocol)
                              ▼
               ┌───────────────────────────────┐
               │  Mothership (NOT YET BUILT)   │
               │                                │
               │  TODO: External orchestrator   │
               │  that connects to multiple     │
               │  hosts/bots via stdin/stdout,  │
               │  listens to their output, and  │
               │  manages them collectively.    │
               │                                │
               │  Currently only the bot side   │
               │  of the pipes exists (stdio.rs │
               │  message protocol).            │
               └───────────────────────────────┘

Inside the Bot (WASM Guest):
┌──────────────────────────────────────────────────────────────┐
│              This Bot (WASM Guest)                            │
│                                                          │
│  lib.rs          ── WASM entry point                     │
│  event_loop.rs   ── Event pump                           │
│  graph.rs        ── Account subscriptions                │
│  sanctum/bot.rs  ── Main bot logic (EventHandler)        │
│  sanctum/state.rs── State + fee evaluation               │
│  sanctum/decision.rs ── Transaction building             │
│  sanctum/config.rs   ── Configuration & pubkeys          │
│  sanctum/message.rs  ── Message action/send protocol     │
│  sanctum/flatslab/   ── Slab account parsing             │
│  wallet.rs       ── Key management + tx assembly         │
│  tx.rs           ── Transaction types                    │
│  token.rs        ── Token balance tracking               │
│  stdio.rs        ── Message protocol over stdin/stdout   │
│  util.rs         ── AccountId/Pubkey conversion helpers  │
│  err.rs          ── Error types                          │
└─────────────────────────────────────────────────────────┘
```

---

## 3. The Host/Guest Boundary (WIT)

**File: `wit/component.wit`**

WIT (WebAssembly Interface Types) defines the contract between host and guest.
Think of it as an API specification. The bot `import`s functions (calls INTO the
host), and `export`s functions (the host calls INTO the bot).

### What the bot IMPORTS (calls on the host):

#### `general` interface - basic I/O
| Function | Purpose |
|----------|---------|
| `ready() -> option<u32>` | **The main loop driver.** Blocks until the host has something for the bot. Returns an event ID, or None when the program should exit. |
| `stdin(channel) -> list<u8>` | Read bytes from a numbered stdin channel. Channel 0 is the config/message pipe. |
| `stdout(channel, data)` | Write bytes to a numbered stdout channel. Used for logging and sending messages back. |
| `finish() -> bool` | Check if the host wants the bot to shut down. |

#### `transactionprocessor` interface - building & sending transactions
| Function | Purpose |
|----------|---------|
| `assembly` (resource) | Builder pattern for constructing transactions. Has methods: `ix()` to add instructions, `compute()` for CU limit, `priority()` for priority fee, `payer()` to set fee payer, `lookup()` to set an address lookup table. |
| `accidlookup` (resource) | Async account ID lookup. Has methods: `poll()` returns an event ID to wait on, `read()` returns `option<u64>` with the resolved AccountId. |
| `send(signature, txdata)` | Send a raw serialized transaction to the network. |
| `blockhash()` | Get a recent blockhash for transaction construction. |
| `accountid(pubkey)` | Convert a 32-byte pubkey to an `accidlookup` resource (async lookup — poll then read to get the u64 AccountId). |
| `pubkey(key)` | Reverse: convert a u64 AccountId back to a 32-byte pubkey. |
| `keygen()` | Generate a new keypair, returns the AccountId. |
| `rent(size)` | Calculate rent exemption for an account of given size. |
| `txflush(list)` | Sign and send a batch of assembled transactions. |
| `txsend(assembly)` | Sign and send a single assembled transaction. |
| `txcancel(assembly)` | Discard a transaction without sending. |

#### `shooter` interface - account streaming & data
| Function | Purpose |
|----------|---------|
| `connect()` | Open a streaming connection to the host's account store. Returns a `client` resource. |
| `disconnect(client)` | Close a streaming connection. |
| `client.subscribe(id, filter, depth)` | Subscribe to account updates. `id` is the root account, `filter` controls which related accounts to include, `depth` controls how many levels of related accounts to follow. |
| `client.cancel(subid)` | Cancel a subscription by its ID. |
| `client.poll()` | Get an event ID to wait on. When `ready()` returns this ID, call `read()`. |
| `client.read()` | Read pending data: committed account snapshots, token accounts, individual account updates, and transaction results. Returns a big tuple of everything available. |
| `client.low()` | Get a `lowlatencyfeed` resource for low-latency account updates (if available). |
| `client.lowstop(feed)` | Stop a low-latency feed. |
| `lowlatencyfeed` (resource) | Low-latency account data stream. Has methods: `poll()` returns an event ID to wait on, `account()` returns account data. |
| `pubkey-map-by-pubkey()` | Look up a host AccountId by pubkey. |
| `pubkey-map-by-id()` | Look up a pubkey by host AccountId. |
| `account-by-id(id)` | Look up raw account data by AccountId. |
| `account-by-pubkey(pubkey)` | Look up raw account data by pubkey. |
| `neighbor(id, downstream)` | Get neighboring edges for an account in the account graph. |

### What the bot EXPORTS (host calls into the bot):

The bot exports `wasi:cli/run` — basically a `run()` function. The host calls
this once to start the bot. The bot then loops internally using `ready()`.

### Key Concept: AccountId vs Pubkey

The host assigns every Solana account a **u64 AccountId**. This is an internal
identifier that's cheaper to pass around than a 32-byte pubkey. The bot uses
`util.rs` functions to convert between the two:

- `account_id_from_pubkey(pubkey)` → u64 (panics on failure)
- `pubkey_from_account_id(id)` → Option<Pubkey> (returns None if host doesn't
  know the ID)

**Rule: Inside the bot, always use AccountId (u64). Only convert to Pubkey when
building Solana instructions.**

---

## 4. File-by-File Walkthrough

### `Cargo.toml` — Dependencies

```toml
[lib]
crate-type = ["cdylib"]     # Compiles to a shared library (.wasm)
```

Key dependencies:
- `wit-bindgen` — generates Rust types from the WIT file
- `solana-sdk` — Solana transaction building, pubkeys, signatures
- `wincode` / `bincode` — binary serialization for transactions
- `paste` — macro helper

The build target is `wasm32-wasip2` (set in `.cargo/config.toml`).

---

### `src/lib.rs` — Entry Point

**Read this first. It's where everything starts.**

```rust
wit_bindgen::generate!({          // (1) Generate Rust bindings from WIT
    world: "catscopevalidator",
    path: "wit",
    generate_all,
});

struct Component;                  // (2) Empty struct to hang the trait on

impl Guest for Component {         // (3) Implement the WASM export
    fn run() -> Result<(), ()> {
        let sampler = Rc::new(RefCell::new(SanctumHook::default()));
        let r = run(sampler);      // (4) Enter the event loop
        // ...
    }
}

export!(Component);               // (5) Wire it up to WASM exports
```

The flow:
1. `wit_bindgen::generate!` reads `wit/component.wit` and creates a Rust module
   `crate::catscope::witbot` with all the types and function stubs.
2. A `Component` struct implements the `Guest` trait (the exported `run()`
   function).
3. `SanctumHook` is the actual bot logic. It gets wrapped in `Rc<RefCell<_>>`
   for shared ownership (explained in Rust Basics section).
4. `run()` from `event_loop.rs` is called — this is the main loop.

**Rust note:** `Rc::new(RefCell::new(...))` is a very common Rust pattern. `Rc`
gives you reference-counted shared ownership (like a shared pointer). `RefCell`
gives you runtime-checked mutable borrowing. Together they let multiple parts
of the code share and mutate the same data.

---

### `src/event_loop.rs` — The Event Pump

This is the **heart of the runtime**. It's a polling loop.

#### The Main Loop

```
on_load() called once
    │
    ▼
┌──────────────────────────────┐
│ general::ready() → event_id  │ ◄── blocks until host has data
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│ Look up callback by event_id │
│ callback.on_event()          │ ◄── reads data from host, pushes Events
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│ Drain event queue            │
│ handler.on_event(event)      │ ◄── bot processes each Event
│ handler.flush()              │
└──────────────┬───────────────┘
               │
               ▼
           loop back to ready()
```

#### Key Types

- **`EventHandler`** trait: The interface your bot implements. Four methods:
  - `on_load(poller)` — called once at startup. Set up subscriptions here.
  - `on_unload()` — called once when the host signals shutdown (after the
    main loop exits). Clean up resources here.
  - `on_event(event)` — called for each event. Process data here.
  - `flush()` — called after all events in a batch. Flush output buffers.

- **`EventPoller`**: The registration system. It maps event IDs (u32) to
  callback objects. When the host wakes you up with an event ID, the poller
  finds the right callback and calls it.

- **`EventID`**: A u32. ID 0 is hardcoded for stdin. Other IDs come from
  `client.poll()` in the graph module.

#### Special: Event ID 0

Event ID 0 is always stdin. When `ready()` returns 0, the event loop reads
from `stdin(0)` and pushes an `Event::Stdin(data)` into the queue.

---

### `src/event.rs` — Event Types

The events that flow through the system:

```rust
pub enum Event {
    Stdin(Vec<u8>),              // Config/message data from the host
    Commit(Commit),              // Finalized account snapshot (batch)
    Account(AccountWrapper),     // Individual account updates (mid-stream)
    Token(Vec<Tokenaccountv1>),  // Token account updates
    Transaction(TransactionList),// Transaction results
}
```

- **Commit**: A batch of finalized account data at a specific slot. The most
  reliable data — it's been confirmed by the network.
- **Account**: Individual account updates that may not be finalized yet.
  "Mid-stream" means you're seeing data as it arrives, before full confirmation.
- **Token**: SPL token account updates (balance changes).
- **Transaction**: Results of transactions the bot sent.

The `EventCallback` trait is what Graph implements to convert raw host data
into these Event variants.

---

### `src/graph.rs` — Account Subscriptions

The **Graph** connects to the host's account store and subscribes to accounts.

#### How Subscriptions Work

```rust
let client = shooter::connect();         // Open connection
let event_id = client.poll();            // Get an event ID for this connection
poller.register(event_id, graph_clone);  // Register for wake-ups

// Subscribe to a specific account and its related accounts
let sub = graph.subscribe(
    account_id,    // Root account to watch
    u32::MAX,      // Filter: u32::MAX means "all related accounts"
    0,             // Depth: how many hops to follow 
)?;
```

When the host has data for this subscription, it wakes the bot via `ready()`,
which returns the event_id. The event loop calls `Graph::on_event()`, which:

1. Calls `client.read()` to get all pending data
2. Pushes appropriate Events into the queue (Token, Account, Commit, Transaction)

#### The Commit Processing Pipeline

`Commit.process(hook)` iterates over raw binary data using `border` markers:

```
data:   [───header+body───|───header+body───|───token_account───]
border: [        72       ,       140       ,        172        ]
```

Each segment is either:
- A full account (header + body) → calls `hook.on_account(header, body)`
- A token account (32 bytes) → calls `hook.on_token(token_account)`

The size of each segment determines which type it is.

#### Subscription Lifecycle

`Subscription` implements `Drop`. When it goes out of scope, it automatically
cancels the subscription with the host. This is RAII (Resource Acquisition Is
Initialization) — a core Rust pattern.

---

### `src/sanctum/bot.rs` — Main Bot Logic (THE IMPORTANT ONE)

`SanctumHook` is where everything comes together. This is **an example
implementation** — your bot would have its own struct implementing these same
traits. It implements four traits:

1. **`EventHandler`** — receives events from the event loop (defined in
   `event_loop.rs`). The event loop waits for data from the host and delivers
   it to your bot by calling `on_event()`. Think of the event loop as a
   mailroom that sorts incoming deliveries and hands them to your bot one at
   a time.
2. **`CommitHook`** — processes finalized account snapshots (batch data that
   has been confirmed by the network). Requires `std::fmt::Write` supertrait.
3. **`MessageHook`** — handles stdin messages from the host (configuration,
   commands, etc.).
4. **`std::fmt::Write`** — required by `CommitHook`. The bot implements this
   by delegating to its `Logger`.

#### Startup (`on_load`)

Every bot follows this general pattern in `on_load`:

```
1. Store the EventPoller (the mechanism that wakes you up when data arrives)
2. Create a Graph connection (connects to the host's account store)
3. Set up signing keys in the Wallet
4. Subscribe to a graph subset (the accounts your bot needs to monitor)
5. Store the Graph to prevent it from being dropped (Rust cleanup)
6. Call eval() — initial evaluation
```

**Sanctum example:** In this bot, step 4 subscribes to the INF program ID
(`5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx`), which gives the host
the root to discover and deliver the pool's downstream accounts
(pool_state, LstStateList, token accounts, etc.).

**Note:** The keypair is currently `Keypair::new()` (random). The `//TODO: add
wallet private key` comment shows this needs to be replaced with an actual
funded key for production.

#### Event Handling (`on_event`)

Every bot receives the same event types. Your `on_event` implementation decides
what to do with each one:

| Event | What It Is (Generic) | Handler | Sanctum Example |
|-------|---------------------|---------|-----------------|
| `Stdin(data)` | Binary messages from the host (config, commands) | `MessageInbound.parse()` → `MessageAction` → `on_message()` | Handles `AdjustConfiguration` (new config pushed from host) and `Shutdown` |
| `Commit(commit)` | Finalized account data (confirmed by the network) | `commit.process(self)` → `CommitHook` methods: `start(slot)` → `on_account()`/`on_token()` → `finish()` | Routes accounts to state logic based on pubkey (pool_state, lst_state_list, flatslab_slab, wallet) |
| `Account(wrapper)` | Mid-stream account updates (not yet finalized — faster but less certain) | Loop through accounts | `state.mid_on_account()` |
| `Token(list)` | Mid-stream SPL token balance updates | Loop through tokens | `state.mid_on_token()` |
| `Transaction(list)` | Results of transactions touching subscribed accounts | Loop through transactions | Currently iterates but takes no action |

#### The CommitHook Implementation

When a Commit arrives (finalized data), the framework calls these methods in
order. Every bot implements the same sequence — only the logic inside differs.
Note: `CommitHook` requires `std::fmt::Write` as a supertrait, so your bot
must also implement `Write` (used for logging during commit processing).

```
start(slot)     → Record the current slot (generic)
on_account()    → Process each account update (your logic here)
on_token()      → Process each token update (your logic here)
finish()        → Run any end-of-batch logic (your logic here)
```

**Sanctum example:**
```
start(slot)     → state.slot = slot
on_account()    → state.on_account(config, wallet, header, body)
                   Routes by pubkey: pool_state, lst_state_list, flatslab_slab, wallet
on_token()      → state.on_token(config, token_account)
finish()        → (currently empty — potential place for eval)
```

---

### `src/sanctum/state.rs` — State Machine & Fee Logic (THE BRAIN)

This is where the actual decision-making happens.

#### StateV1 Fields

```rust
pub(crate) struct StateV1 {
    logger: Logger,                            // Logger for diagnostics
    pub(crate) slot: Slot,                    // Current Solana slot
    pub(crate) funding: Funding,              // SOL balance + token balances
    pub(crate) sol_weight: Option<f64>,        // SOL as fraction of pool
    pub(crate) curr_input_fee: Option<FeeNanos>, // Current input fee from slab
    pub(crate) curr_output_fee: Option<f64>,   // Current output fee (what we adjust)
    pub(crate) ix_builder: InstructionBuilder,  // Accumulates instructions
}
```

`StateV1::new(logger)` takes a `Logger` instance (cloned from the bot's logger).

#### Account Routing (`on_account`)

When an account update arrives, the account's pubkey (as AccountId) determines
what to do:

```
header.pubkey matches:
  ├── wallet account     → update funding.sol (SOL balance)
  ├── pool_state         → log only (TODO: might need later)
  ├── lst_state_list     → IMPORTANT: parse, calculate SOL weight, check fees
  └── flatslab_slab      → cache current input/output fees
```

#### The Fee Evaluation Pipeline

When `lst_state_list` updates (meaning a trade happened):

```
1. parse_lst_state_list(body)
   └── Splits 80-byte entries: each has {is_disabled, sol_value, mint, calculator}

2. calculate_sol_weight(lst_data, sol_mint)
   └── sol_weight = sol_entry.sol_value / sum(all_entries.sol_value)

3. check_update_fees(config, wallet)
   └── eval_fee_update(config)
       ├── target_fee = slope * sol_weight + intercept
       ├── diff_pct = |target - current| / current
       └── if diff_pct > threshold → FeeUpdateAction
```

#### LstStateList Binary Layout (80 bytes per entry)

```
Offset  Size  Field
0       1     is_input_disabled (bool)
1       1     pool_reserves_bump
2       1     protocol_fee_accumulator_bump
3       5     padding
8       8     sol_value (u64 LE) ← the important one
16      32    mint pubkey
48      32    sol_value_calculator program
```

#### State Machine (documented in the code)

```
S0  Boot               → subscribe to inf program
S1  Idle               → waiting for host to deliver account update
S2  Trade detected     → LstStateList changed
S3  Slab update        → parse current fee and update state cache
S4  Calculate weight   → parse LstStateList, compute sol_value / total
S5  Calculate target   → linear equation: target_fee = slope * weight + intercept
S6  Compare fee        → target_fee vs current_fee
S7  Send update        → craft, sign, send SetLstFee tx
S8  Output monitoring  → emit tx sig + slot
S9  Shutdown           → terminate / flush
```

Normal flow: `S0 → S1 → (S2 or S3) → S4 → S5 → S6 → (S1 or S7) → S8 → S1`

---

### `src/sanctum/decision.rs` — Transaction Construction

Converts a `Decision` into Solana instructions.

```rust
pub(crate) enum Decision {
    Nothing,
    AdjustFee(FeeNanos, FeeNanos),  // (input_fee, output_fee)
}
```

#### `ix_slab_adjustment` — The SetLstFee instruction

```
Instruction data (9 bytes):
  [0]     discriminator = 253 (SET_LST_FEE_IX_DISCM)
  [1..5]  input_fee_nanos (i32 LE)
  [5..9]  output_fee_nanos (i32 LE)

Accounts (in order):
  1. admin   (signer, writable)
  2. payer   (signer, writable)
  3. slab    (writable)
  4. mint    (writable)
  5. system_program (writable)
```

Also includes `ix_slab_set_admin` and `ix_remove_lst` — these are NOT used by
the bot's automated logic. They're governance operations included for
completeness.

---

### `src/sanctum/config.rs` — Configuration

Hardcoded Solana addresses:

| Account | Pubkey | Purpose |
|---------|--------|---------|
| INF Program | `5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx` | Sanctum infinity pool controller |
| Pool State | `AYhux5gJzCoeoc1PoJ1VxwPDe22RwcvpHviLDD1oCGvW` | Pool state PDA |
| LstStateList | `Gb7m4daakbVbrFLR33FKMDVMHAprRZ66CSYt4bpFwUgS` | List of all LSTs in the pool |
| FlatSlab Slab | `4T9YzXnmQFMyYi2nrxyXjhtUANavmCkxGCsU3GKaNjwT` | Fee slab account |
| FlatSlab Program | `s1b6NRXj6ygNu1QMKXh2H9LUR2aPApAAm1UQ2DjdhNV` | The program that processes SetLstFee |
| SOL Mint | `[0u8; 32]` (all zeros) | Represents native SOL in Sanctum |

Fee parameters (defaults):
- `fee_slope: 0.0`
- `fee_intercept: 0.0`
- `update_threshold: 0.01` (1%)

The `Configuration` struct is `#[repr(C, align(8))]` so the host can send it
as raw bytes via `MessageAction::AdjustConfiguration`. It also stores:
- `o_wallet: Option<AccountId>` — the bot's wallet (set after keygen)
- `m_setting: HashMap<Vec<u8>, Vec<u8>>` — generic key-value settings
- `inf_program_id: AccountId` — the INF program ID as an AccountId

The `FlatSlabConfiguration` comes from stdin messages (the host sends it):
- `admin` — who signs SetLstFee transactions
- `payer` — who pays rent
- `slab` — the slab PDA

---

### `src/sanctum/flatslab/` — Slab Account Parsing

Vendored from `igneous-labs/inf-1.5`. Reads the FlatSlab slab account.

#### Slab Account Layout

```
[0..32]    admin pubkey (32 bytes)
[32..]     array of 40-byte entries (sorted by mint):

  Each entry (SlabEntryPacked, 40 bytes):
  [0..32]   mint pubkey
  [32..36]  input_fee_nanos  (i32 LE)
  [36..40]  output_fee_nanos (i32 LE)
```

#### FeeNanos

Fees are in "nanos" (billionths of 1.0):
- `10_000_000` nanos = 1% = 100 bps
- `100_000` nanos = 1 bps (0.01%)
- Range: `[-50_000_000, 1_000_000_000]` = [-5%, 100%]
- Negative = rebate (incentive for trading in that direction)

The slab entries are sorted by mint, so lookups use binary search.

---

### `src/wallet.rs` — Key Management & Transaction Assembly

Manages signing keys and builds transactions.

```rust
pub struct Wallet {
    m_key: HashMap<AccountId, SignerStatus>,  // All signing keys
    q_ix: VecDeque<Instruction>,              // Queued instructions
    compute: ComputeUnit,                      // Total CU budget
    sys_id: AccountId,                         // System program AccountId
    tx_data: Box<[u8; 4 * 1024]>,             // Pre-allocated tx serialization buffer
    payer: Option<AccountId>,                  // Who pays tx fees
}
```

Flow:
1. `append_key(keypair, graph)` — registers a key AND subscribes to its
   account (to track SOL balance for fee payment).
2. `append_ix(instruction, compute)` — queue an instruction.
3. `assemble()` — builds the final transaction:
   - Gets a recent blockhash from the host
   - Prepends a `set_compute_unit_limit` instruction
   - Signs with the payer's keypair
   - Serializes to bytes

---

### `src/tx.rs` — Transaction Types

Two sides:

1. **`TransactionSender`** — sends pre-built transactions to the host via
   `transactionprocessor::send()`.
2. **`TransactionList`** / **`CatscopeTransaction`** — represents transactions
   received FROM the host (other people's transactions that touched subscribed
   accounts). Used for monitoring what's happening on-chain.

`CatscopeTransaction` is a C-repr struct read directly from raw bytes (zero-copy
parsing). Contains account lists and instructions.

---

### `src/stdio.rs` — Message Protocol

The bot communicates with the host via a structured message protocol over
stdin/stdout channels.

#### Message Format (v1)

Inbound and outbound messages use the same binary format:

```
[1 byte]   command type (1=Pair, 2=Dump)
[4 bytes]  version (u32 LE, must equal 1)
[1 byte]   key size
[N bytes]  key (variable length)
[2 bytes]  value size (u16 LE)
[M bytes]  value (variable length, max 4KB)
```

#### Key Types

- **`Logger`** — cloneable logging wrapper around a stderr `StdioPacket`.
  Implements `std::fmt::Write` so you can use `write!(logger, "...")`.
- **`StdioPacket`** — buffered writer. Accumulates data and flushes via
  `general::stdout(channel, data)`.
- **`MessageInbound`** — parses incoming binary messages. Validates version
  and extracts command type, key, and value.
- **`MessageOutbound`** — builds outgoing binary messages. Tracks a nonce
  (incremented per message) for ordering.
- **`MessageHandler`** — legacy streaming parser for the older key-value
  message format. Still present but the bot now uses `MessageInbound`.
- **`MessageHook`** trait — your bot implements this to handle parsed messages.
- **`MessageType`** enum — `Pair` (single key-value) or `Dump` (full store).

Messages are interpreted by `sanctum/message.rs`, which converts the raw
binary key/value into a `MessageAction` enum (`AdjustConfiguration` or
`Shutdown`).

---

### `src/token.rs` — Token Balance Tracking

```rust
pub struct TokenDatabase {
    m_owner: HashMap<AccountId, MintNode>,
}
```

Tracks SPL token balances organized as:
```
owner → mint → token_account_id → balance
```

An owner can have multiple token accounts for the same mint. `balance()` sums
them all.

---

### `src/util.rs` — Utility Functions

- `pubkey_from_account_id() -> Option<Pubkey>` / `account_id_from_pubkey()` —
  convert between the host's u64 IDs and Solana 32-byte pubkeys. Note:
  `pubkey_from_account_id` returns `Option` (fails if the host doesn't know
  the ID). `account_id_from_pubkey` panics on failure.
- `as_bytes()` / `as_bytes_mut()` — reinterpret any struct as a byte slice
  (unsafe, used for zero-copy binary parsing).
- `rc_unlock()` / `rc_unlock_mut()` — get references from `Rc<UnsafeCell<T>>`.
  This is unsafe because it bypasses Rust's borrow checker — the programmer
  must ensure no aliased mutable references exist.

---

### `src/err.rs` — Error Types

A unified error enum wrapping errors from all three WIT interfaces:

```rust
pub enum CatscopeGuestError {
    InsufficientBuffer,                        // Message too short to parse
    BadVersion,                                // Message version mismatch
    Unknown(String),
    Shooter(ShooterErrorCode),
    General(GeneralErrorCode),
    Transaction(TransactionErrorCode),
    MissingEvent(u32),
    BufferTooSmall,
    TransactionParse,
}
```

Implements `From` for automatic conversion (the `?` operator).

---

### `src/sanctum/message.rs` — Message Actions

Defines the message protocol between the host and the Sanctum bot:

```rust
pub(crate) enum MessageAction {
    AdjustConfiguration(Configuration),  // Host pushes new config
    Shutdown,                            // Host tells bot to stop
}

pub(crate) enum MessageSend {
    Wallet(Pubkey),  // Bot sends its wallet pubkey back to host
}
```

`MessageAction` is parsed from `MessageInbound` by matching on the key byte:
key `1` = `AdjustConfiguration` (value is the raw `Configuration` struct),
key `2` = `Shutdown`. `MessageSend` serializes outbound messages via
`MessageOutbound`.

---

## 5. Data Flow: End to End

### Startup Flow

This flow is the same for every bot — only the EventHandler implementation
changes.

```
Host starts the WASM component
  │
  ▼
lib.rs: Component::run()
  │  Creates your EventHandler (any struct that implements the trait)
  │  Example: SanctumHook with default config and empty state
  │
  ▼
event_loop.rs: run(handler)
  │  Creates EventPoller
  │  Calls handler.on_load(poller)
  │
  ▼
Your bot's on_load():
  │  1. Creates Graph (connects to host's account store)
  │  2. Sets up signing keys in Wallet
  │  3. Subscribes to the graph subset it needs to monitor
  │  4. Runs any initial logic
  │
  │  Sanctum example: subscribes to the INF program ID (the root
  │  for discovering downstream pool accounts), generates a keypair,
  │  calls eval()
  │
  ▼
event_loop.rs: main loop begins
  │  general::ready() blocks until host has data
```

### Steady-State Data Flow

The event loop is generic infrastructure — it works the same for any bot.
Your bot's logic only runs inside the event handlers.

```
Validator processes a slot (account changes committed)
  │
  ▼
Geyser plugin API notifies Catscope host (in-process, zerohop)
  │
  ▼
general::ready() returns event_id
  │
  ▼
Event loop looks up callback for event_id
  │
  ├── event_id == 0 → stdin message
  │     ├── Parse key-value message
  │     └── Your bot handles the message via MessageHook
  │
  └── event_id == graph_event_id → account data
        │
        ▼
      graph.on_event()
        │  Calls client.read()
        │  Pushes Events into queue
        │
        ▼
      Event queue drained — your bot's on_event() handles each:
        │
        ├── Event::Commit(commit)
        │     commit.process(your_hook)
        │     For each account: your on_account() routes by pubkey
        │
        ├── Event::Account(wrapper) → mid-stream account updates
        ├── Event::Token(list) → mid-stream token updates
        └── Event::Transaction(list) → transaction results
```

**Sanctum example** of what happens inside `on_event`:
- **Stdin**: parsed via `MessageInbound` → `MessageAction::AdjustConfiguration`
  (new config from host) or `MessageAction::Shutdown`
- **Commit accounts**: route by pubkey — pool_state (log), lst_state_list
  (parse → calculate sol_weight → check fees), flatslab_slab (cache fees),
  wallet (update SOL balance)
- **Account/Token**: `mid_on_account()` / `mid_on_token()` for unconfirmed updates
- **Transaction**: iterates but currently takes no action

### Fee Update Flow — Sanctum Example (When Threshold Exceeded)

```
LstStateList updated (trade happened)
  │
  ▼
parse_lst_state_list(body)
  │  Split into 80-byte entries
  │  Extract sol_value and mint for each
  │
  ▼
calculate_sol_weight(lst_data, sol_mint)
  │  sol_weight = sol_entry.sol_value / total_sol_value
  │
  ▼
check_update_fees(config, wallet)
  │
  ▼
eval_fee_update(config)
  │  target = slope * sol_weight + intercept
  │  diff = |target - current| / current
  │
  ├── diff <= threshold → return None (no action)
  │
  └── diff > threshold → return FeeUpdateAction
        │
        ▼
      (Currently: Decision::Nothing is always set — the actual
       transaction sending via decision::process() is not yet
       wired up. This is the main TODO.)
```

---

## 6. The State Machine

Every bot built on this framework will have its own state machine — the
specific states depend on what the bot does. Below is the **Sanctum bot's
state machine** as an example. Your bot would define its own states based on
the accounts it monitors and the decisions it makes.

Documented at the bottom of `state.rs`:

```
S0 ─── Boot ──────────────► subscribe to accounts
│
▼
S1 ◄── Idle ──────────────► waiting for host events
│
├──── Slab update ────────► S3: cache fees → back to S1
│
└──── Trade detected ─────► S2: LstStateList changed
                              │
                              ▼
                            S4: Calculate SOL weight
                              │
                              ▼
                            S5: Calculate target fee
                              │
                              ▼
                            S6: Compare target vs current
                              │
                    ┌─────────┴──────────┐
                    │                    │
              within threshold     exceeds threshold
                    │                    │
                    ▼                    ▼
                   S1                   S7: Send SetLstFee tx
                                         │
                                         ▼
                                        S8: Emit monitoring data
                                         │
                                         ▼
                                        S1
```

The tests in `state_transition_tests.rs` verify this state machine using a
`simulate_on_account()` function that mirrors the real `on_account()` routing.

---

## 7. What Is Stagnant vs. Adjustable

### STAGNANT (Infrastructure — don't change per use case)

| File | Why |
|------|-----|
| `event_loop.rs` | Generic event pump. Works for any bot. |
| `event.rs` | Event type definitions. Stable. |
| `graph.rs` | Account subscription system. Reusable. |
| `stdio.rs` | Message protocol. Reusable. |
| `tx.rs` | Transaction types. Reusable. |
| `util.rs` | Conversion helpers. Reusable. |
| `err.rs` | Error types. Stable. |
| `token.rs` | Token tracking. Reusable. |
| `wallet.rs` | Key management. Reusable. |
| `wit/component.wit` | Host/guest contract. Defined by Catscope, not by you. |
| `sanctum/flatslab/` | Vendored types. Only change if upstream changes. |

### ADJUSTABLE (Per Use Case — this is where you customize)

| File | What to change |
|------|---------------|
| `lib.rs` | Line 26: Replace `SanctumHook` with your own `EventHandler` impl. |
| `sanctum/config.rs` | **Account addresses** (which accounts to watch), **fee formula** (slope, intercept), **threshold** (how much drift to tolerate). For a different bot, you'd replace these entirely. |
| `sanctum/state.rs` | **The core logic.** `on_account()` routing, `eval_fee_update()`, and `check_update_fees()`. This is where you define what the bot does when it sees data. |
| `sanctum/decision.rs` | **Transaction building.** What instructions to send. For a different protocol, you'd write different `ix_*` functions. |
| `sanctum/bot.rs` | **Event wiring.** How events map to state changes. The `on_load()` subscriptions, `on_event()` routing, and `on_message()` config handling. |
| `sanctum/message.rs` | **Message actions.** What stdin messages the bot expects (`AdjustConfiguration`, `Shutdown`) and what it sends back (`Wallet`). For a different bot, define your own `MessageAction`/`MessageSend` enums. |

### NOT YET WIRED UP (TODOs)

1. **`bot.rs:109`** — `Keypair::new()` generates a random key. Production needs
   a real funded keypair (probably received via stdin message).
2. **`state.rs:173`** — `check_update_fees()` always sets `Decision::Nothing`.
   The actual `decision::process()` call is missing. This is the gap between
   "deciding to update" and "actually sending the transaction."
3. **`state.rs:75,79`** — `mid_on_account` and `mid_on_token` are commented
   out. These would let the bot react to unconfirmed data (faster but riskier).
4. **`bot.rs:243-245`** — `CommitHook::finish()` is empty. Could trigger
   `eval()` here for end-of-commit evaluation.
5. **`bot.rs:84`** — BUG: `let state = unsafe { &mut *self.rc_wallet.get() }`
   should be `self.rc_state.get()`. Currently dereferences `rc_wallet` instead
   of `rc_state`.

---

## 8. Rust Basics for This Codebase

### Ownership & Borrowing Patterns Used Here

#### `Rc<RefCell<T>>` — Shared mutable ownership (safe)

Used for `SanctumHook` in `lib.rs`. Multiple parts of the code can hold a
reference and borrow it mutably at runtime.

```rust
let sampler = Rc::new(RefCell::new(SanctumHook::default()));
let mut h = handler.borrow_mut();  // Runtime-checked mutable borrow
```

#### `Rc<UnsafeCell<T>>` — Shared mutable ownership (unsafe)

Used for `Graph`, `Wallet`, `StateV1`, `Configuration`. Like `RefCell` but
without runtime checks. Faster but you can cause undefined behavior if you
create overlapping mutable references.

```rust
let config = unsafe { &*self.rc_config.get() };  // Get raw pointer, deref
```

The helper `rc_unlock_mut()` in `util.rs` wraps this pattern.

**Why unsafe here?** In WASM single-threaded code, the runtime borrow checking
of `RefCell` is unnecessary overhead. The programmer guarantees no aliasing.

#### `Option<T>` — Maybe a value

Used heavily. Rust has no null — `Option` is the replacement.

```rust
pub(crate) sol_weight: Option<f64>,  // Might not have this data yet

match self.sol_weight {
    Some(w) => /* use w */,
    None => /* don't have it yet */,
}
```

#### `Drop` trait — Cleanup on destruction

`Subscription` and `InnerGraph` implement `Drop`. When they go out of scope,
cleanup happens automatically (cancel subscription, unregister event).

### Common Patterns

#### The `?` operator — Early return on error

```rust
let client = shooter::connect()?;  // If Err, return the error immediately
```

This works because `CatscopeGuestError` implements `From<ShooterErrorCode>`,
so the conversion happens automatically.

#### `#[repr(C)]` — C-compatible memory layout

Used on `CatscopeTransaction`, `CatscopeInstruction`, `SlabEntryPacked`.
Guarantees the struct's fields are laid out in memory exactly as written,
with no Rust-specific reordering or padding. Essential for zero-copy parsing
of binary data.

#### `unsafe` blocks

This codebase uses `unsafe` for:
1. **Raw pointer derefs** via `UnsafeCell` — bypassing borrow checker
2. **`as_bytes_mut()`** — reinterpreting structs as byte slices
3. **`from_raw_parts()`** in flatslab — casting byte arrays to struct arrays

These are common patterns in systems code. The safety guarantees are maintained
by the programmer, not the compiler.

#### Traits as interfaces

Rust uses traits where other languages use interfaces:
- `EventHandler` — what a bot must implement
- `CommitHook` — what a commit processor must implement
- `EventCallback` — what an event source must implement
- `MessageHook` — what a message receiver must implement
- `IxBuilder` — what an instruction builder must implement (currently unused)

### WASM-Specific Notes

- No threads. Everything is single-threaded, which is why `Rc` (not `Arc`) and
  `UnsafeCell` (not `Mutex`) are used.
- No direct network access. All I/O goes through the WIT imports. The bot runs
  in a WASM sandbox inside the catscope host, which itself runs inside the
  Agave validator process as a geyser plugin.
- `crate-type = ["cdylib"]` — builds a C-compatible dynamic library, which is
  what the WASM host expects.
- The `wasm32-wasip2` target includes WASI Preview 2 support (filesystem,
  clocks, random, sockets interfaces in the `wit/deps/` folder).
- **Zerohop advantage:** Because the host is a validator plugin, account data
  doesn't traverse the network. The geyser plugin API delivers account updates
  directly in-process, giving bots the lowest possible latency.

---

## 9. Quick Reference

### Build Commands

```bash
# Set up (one time)
rustup target add wasm32-wasip2

# Build for WASM (what gets deployed)
cargo build --target wasm32-wasip2 --release

# Run tests (native, not WASM)
cargo test --target x86_64-unknown-linux-gnu

# Lint
cargo clippy -- -D clippy::shadow_unrelated -D clippy::shadow_reuse \
  -D unused_variables -D needless_borrow -D large_enum_variant
```

### Key Account Addresses (Solana Mainnet)

| Account | Pubkey |
|---------|--------|
| INF Program | `5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx` |
| Pool State | `AYhux5gJzCoeoc1PoJ1VxwPDe22RwcvpHviLDD1oCGvW` |
| LstStateList | `Gb7m4daakbVbrFLR33FKMDVMHAprRZ66CSYt4bpFwUgS` |
| FlatSlab Slab | `4T9YzXnmQFMyYi2nrxyXjhtUANavmCkxGCsU3GKaNjwT` |
| FlatSlab Program | `s1b6NRXj6ygNu1QMKXh2H9LUR2aPApAAm1UQ2DjdhNV` |
| SOL Mint | `11111111111111111111111111111111` (all zeros) |

### Fee Math Cheat Sheet

```
1 nano       = 1 billionth        = 0.0000001%
100_000      = 1 basis point      = 0.01%
10_000_000   = 100 bps            = 1%
1_000_000_000 = 10_000 bps        = 100%
-50_000_000  = -500 bps           = -5% (max rebate)
```

### Reading Order for New Developers

1. This study guide (you're here)
2. `README.md` — quick overview
3. `wit/component.wit` — understand the host/guest contract
4. `src/lib.rs` — entry point
5. `src/event_loop.rs` — how events flow
6. `src/event.rs` — what events look like
7. `src/graph.rs` — how account data arrives
8. `src/sanctum/config.rs` — what accounts we watch
9. `src/sanctum/bot.rs` — how events route to logic
10. `src/sanctum/state.rs` — the actual decision logic
11. `src/sanctum/decision.rs` — what transactions we build
12. `src/sanctum/message.rs` — host message protocol (actions & sends)
13. `src/sanctum/flatslab/` — slab account parsing
14. `src/wallet.rs` — how transactions are assembled and signed
15. `src/stdio.rs` — low-level message framing
