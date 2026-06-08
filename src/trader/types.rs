use crate::graph::AccountId;

/// Identifies which DEX protocol owns a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DexType {
    /// Raydium AMM v4 (constant-product, OpenBook-backed)
    RaydiumAmm,
    /// Orca Whirlpool (concentrated liquidity)
    OrcaWhirlpool,
    /// Kamino lending reserve (price feed only, no swap IX produced)
    KaminoLending,
}

/// Price and liquidity snapshot for a pool.
#[derive(Debug, Clone, Copy)]
pub struct PoolPrice {
    /// First token mint (coin / base / token A)
    pub token_a: AccountId,
    /// Second token mint (pc / quote / token B)
    pub token_b: AccountId,
    /// Spot price: `token_b_raw_units / token_a_raw_units`.
    /// Not decimal-adjusted; multiply by `10^(decimals_a - decimals_b)` for display.
    pub price: f64,
    /// Token A reserve in raw units (0 for CLMM pools until vaults are observed).
    pub reserve_a: u64,
    /// Token B reserve in raw units (0 for CLMM pools until vaults are observed).
    pub reserve_b: u64,
    /// Swap fee in basis points (e.g. 25 = 0.25%).
    pub fee_bps: u16,
}

impl PoolPrice {
    /// Constant-product AMM quote (`x * y = k`).
    ///
    /// Given `amount_in` of `input_mint`, returns the expected output after
    /// the pool fee is applied. Returns `0` when reserves are unknown.
    pub fn quote(&self, input_mint: AccountId, amount_in: u64) -> u64 {
        let (reserve_in, reserve_out) = if input_mint == self.token_a {
            (self.reserve_a, self.reserve_b)
        } else {
            (self.reserve_b, self.reserve_a)
        };
        if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
            return 0;
        }
        let fee_mult = 10_000u64.saturating_sub(self.fee_bps as u64);
        let amount_after_fee = amount_in.saturating_mul(fee_mult) / 10_000;
        // dy = y * dx / (x + dx)
        let num = (amount_after_fee as u128) * (reserve_out as u128);
        let den = (reserve_in as u128) + (amount_after_fee as u128);
        (num / den) as u64
    }

    /// Apply a slippage tolerance (in basis points) to a quoted amount.
    ///
    /// Returns `quote * (10_000 - slippage_bps) / 10_000`.
    pub fn apply_slippage(quote: u64, slippage_bps: u16) -> u64 {
        let mult = 10_000u64.saturating_sub(slippage_bps as u64);
        quote.saturating_mul(mult) / 10_000
    }
}

/// Parameters for a single-hop swap instruction.
#[derive(Debug)]
pub struct SwapParams {
    /// Pool account to route through.
    pub pool: AccountId,
    /// Mint of the token you are sending.
    pub input_mint: AccountId,
    /// Mint of the token you want to receive.
    pub output_mint: AccountId,
    /// Exact amount to send (raw token units).
    pub amount_in: u64,
    /// Minimum acceptable output (slippage guard).
    pub min_amount_out: u64,
    /// Your SPL token account that holds `input_mint`.
    pub user_source_token_account: AccountId,
    /// Your SPL token account that will receive `output_mint`.
    pub user_destination_token_account: AccountId,
    /// Your wallet (must be a signer in the assembled transaction).
    pub user_wallet: AccountId,
}

impl SwapParams {
    pub fn pool_lookup_id(&self) -> [AccountId; 2] {
        let mut id = [self.input_mint, self.output_mint];
        id.sort();
        id
    }

    /// Set `min_amount_out` from a spot price and a maximum slippage tolerance.
    ///
    /// `spot_price` — expected output raw units per input raw unit for this
    ///   swap direction (for A→B use `pool.price`; for B→A use `1.0 / pool.price`).
    /// `max_slippage` — fractional tolerance, e.g. `0.005` for 0.5%.
    ///
    /// `min_amount_out = floor(amount_in * spot_price * (1 - max_slippage))`
    pub fn set_min_amount_out(&mut self, spot_price: f64, max_slippage: f64) {
        let expected = self.amount_in as f64 * spot_price;
        self.min_amount_out = (expected * (1.0 - max_slippage)) as u64;
    }
}
/// Emitted whenever a tracked pool's pricing state changes.
#[derive(Debug, Clone)]
pub struct PriceUpdate {
    /// Pool account that changed.
    pub pool: AccountId,
    /// Updated price snapshot.
    pub price: PoolPrice,
}

/// Errors from `DexTrader` operations.
#[derive(Debug, Clone)]
pub enum TraderError {
    /// No pool registered under that account ID.
    UnknownPool(AccountId),
    /// Pool has not received enough on-chain state to trade (e.g., reserves
    /// not yet observed for Raydium vault accounts).
    PoolNotReady,
    /// The `input_mint` / `output_mint` in `SwapParams` do not match the pool.
    WrongMints,
    /// `pubkey_from_account_id` returned `None` for this account.
    PubkeyResolutionFailed(AccountId),
    /// A required DEX-specific configuration field is absent.
    MissingConfig(&'static str),
}
impl std::fmt::Display for TraderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPool(arg0) => f.debug_tuple("UnknownPool").field(arg0).finish(),
            Self::PoolNotReady => write!(f, "PoolNotReady"),
            Self::WrongMints => write!(f, "WrongMints"),
            Self::PubkeyResolutionFailed(arg0) => {
                f.debug_tuple("PubkeyResolutionFailed").field(arg0).finish()
            }
            Self::MissingConfig(arg0) => f.debug_tuple("MissingConfig").field(arg0).finish(),
        }
    }
}
