# Catscope Rust bot

This is a trading bot that is compiled into web assembly and uploaded using [Solpipe](https://solpipe.io) to a validator running the Catscope Geyser Plugin.

* `./src/brain/helloworldv1` - print slot numbers to stderr and swap USDC and SOL on Orca

For an example of how to upload and run a web assembly bot, [please see this repository](https://github.com/noncepad/optimizer).

This are some arbitrage related information in [TRADE.md](./TRADE.md), but that functionality
has not been built into this example.

## Build

```bash
# Install target
rustup target add wasm32-wasip2

# Build for WASM
cargo build --target wasm32-wasip2 --release

# Check compilation
cargo check --target wasm32-wasip2
```

The binary will be at `./target/wasm32-wasip2/release/catscope_rust_bot.wasm`

## Test

The default build target is `wasm32-wasip2`, so tests must be run against a native target explicitly:

```bash
# x86-64 Linux
cargo test --target x86_64-unknown-linux-gnu

# ARM64 Linux
cargo test --target aarch64-unknown-linux-gnu
```
