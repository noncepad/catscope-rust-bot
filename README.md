# Example Catscope Validator Runtime Bot

There are two sample modes:

* `./src/brain/helloworldv1` - print slot numbers to stderr
* `./src/brain/boucnerv1` - not yet complete

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

