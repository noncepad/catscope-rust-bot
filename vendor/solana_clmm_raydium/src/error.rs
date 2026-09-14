// Replacement for `crate::error::ErrorCode` from raydium-clmm program.
// Variants are the subset actually referenced from libraries/.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    TickUpperOverflow,
    SqrtPriceX64,
    LiquiditySubValueErr,
    LiquidityAddValueErr,
    MaxTokenOverflow,
    SqrtPriceLimitOverflow,
    InvalidTickIndex,
}

impl ErrorCode {
    /// Stable, human-readable reason. Stable in the sense that adding new
    /// variants is a minor bump but renaming an existing variant's reason
    /// string is breaking — keep these wording-stable across patch releases.
    pub const fn reason(self) -> &'static str {
        match self {
            ErrorCode::TickUpperOverflow => "tick_upper exceeds MAX_TICK",
            ErrorCode::SqrtPriceX64 => {
                "sqrt_price_x64 out of [MIN_SQRT_PRICE_X64, MAX_SQRT_PRICE_X64)"
            }
            ErrorCode::LiquiditySubValueErr => "liquidity subtraction underflow",
            ErrorCode::LiquidityAddValueErr => "liquidity addition overflow",
            ErrorCode::MaxTokenOverflow => "u64 token amount overflow",
            ErrorCode::SqrtPriceLimitOverflow => "sqrt_price_limit beyond domain bound",
            ErrorCode::InvalidTickIndex => "tick index out of [MIN_TICK, MAX_TICK]",
        }
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.reason())
    }
}

impl core::error::Error for ErrorCode {}

pub type Result<T> = core::result::Result<T, ErrorCode>;

#[macro_export]
macro_rules! require {
    ($cond:expr, $err:expr $(,)?) => {
        if !($cond) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gt {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a > $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gte {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a >= $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! err {
    ($err:expr $(,)?) => {
        Err($err)
    };
}
