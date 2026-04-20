# INF Rust Bot

A Solana bot that automatically adjusts swap fees for the Sanctum INF liquid staking pool based on pool composition. Runs as a WebAssembly component inside the Catscope host.

## Overview

The Sanctum INF pool holds various liquid staking tokens (LSTs) plus native SOL. This bot monitors the pool composition and dynamically adjusts the SOL output fee to maintain optimal trading conditions.

**Fee Calculation:**
```
target_fee = slope × sol_weight + intercept
```

When the current on-chain fee drifts beyond a configured threshold, the bot automatically sends a transaction to update it.


## Architecture

**WASM Component**: Runs inside Catscope, a Solana account monitoring host.

**Key Modules**:
- `lib.rs` - WASM entry point
- `event_loop.rs` - Event dispatcher
- `graph.rs` - Account subscriptions
- `sanctum/` - Fee management logic
- `sanctum/flatslab/` - Vendored flatslab parsing

**Event Flow**:
1. Subscribe to LstStateList and Flatslab Slab accounts
2. Parse account updates to extract SOL weights and current fees
3. Calculate target fee and compare to current
4. Send update transaction if drift exceeds threshold

## Build
```bash
# Install target
rustup target add wasm32-wasip2

# Build for WASM
cargo build --target wasm32-wasip2 --release

# Check compilation
cargo check --target wasm32-wasip2
```

## Testing
```bash
cargo test --target x86_64-unknown-linux-gnu
```