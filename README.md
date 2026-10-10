# Catscope Rust bot

This is a trading bot that is compiled into web assembly and uploaded using [Solpipe](https://solpipe.io) to a validator running [the Catscope Geyser Plugin](https://catscope.io).

* `./src/brain/marketwatchv1` - print the volatility and correlation of some token pairs

Use [the optimizer project](https://github.com/noncepad/optimizer) to upload and run the bot in a validator.

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
