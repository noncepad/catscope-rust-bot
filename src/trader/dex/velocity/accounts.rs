//! Raw byte-offset parsing for Drift/Velocity Protocol's `PerpMarket`
//! account (Drift rebranded to "Velocity Exchange" -- same on-chain
//! program, confirmed live this session; see project memory
//! `project_drift_rebrand.md`). This bot's existing `dex::drift` module
//! only parses `SpotMarket` (lending) accounts -- this is a new, separate
//! module for perpetual futures market data, used by `perp_router`.
//!
//! # Account layout -- PerpMarket (Anchor, zero_copy, 8-byte discriminator)
//!
//! Offsets were derived by summing `protocol-v2`'s own `PerpMarket`/`AMM`/
//! `HistoricalOracleData`/`PoolBalance` struct field sizes in declaration
//! order (`programs/drift/src/state/perp_market.rs`,
//! `programs/drift/src/state/oracle.rs`, both fetched directly from
//! `drift-labs/protocol-v2`'s `master` branch this session -- not
//! guessed), then independently verified against a real, live mainnet
//! account: `market_index=0`'s PDA (`["perp_market", 0u16.to_le_bytes()]`
//! against `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH`) resolves to
//! `8UJgxaiQx5nTrdDgph5FiahMmzduuLTLf5WmsPegYA6W`, a real 1216-byte
//! account owned by that program.
//!
//! Two independent checks confirmed the derived offsets: (1) `name`'s
//! offset was found by scanning the raw bytes for a printable ASCII run
//! rather than trusted from arithmetic alone -- landed at byte 1000,
//! containing `"SOL-PERP"` (space-padded, not zero-padded, to 32 bytes);
//! (2) with that anchor confirmed, the funding/price fields decoded to
//! internally-consistent, plausible values: `funding_period` = exactly
//! `3600` (1 hour, matching Drift/Velocity's own published docs),
//! `last_funding_rate_ts` decodes to a 2026 Unix timestamp,
//! `last_mark_price_twap` ≈ $84 (right order of magnitude for SOL), and
//! `last_funding_rate` implies roughly -10.6% annualized (a plausible
//! perp funding magnitude, not an absurd value) -- computed as
//! `(last_funding_rate / FUNDING_RATE_PRECISION) / mark_price_usd *
//! (24 * 365)` periods/year, `FUNDING_RATE_PRECISION = PRICE_PRECISION
//! (1e6) * FUNDING_RATE_BUFFER (1e3) = 1e9`, both constants confirmed
//! directly from `protocol-v2`'s own `math/constants.rs`. An earlier,
//! wrong offset attempt (before this second check) produced nonsensical
//! values (mark price ~$1775, funding_period in the trillions) --
//! flagging this because a plausible-looking wrong offset is exactly the
//! failure mode this two-part verification is meant to catch.
//!
//! **Not verified**: the sign convention of `last_funding_rate` (does
//! negative mean longs pay shorts, or the reverse?). Ellipsis Labs'
//! Phoenix `rise-public` SDK (a different protocol, but useful reference)
//! describes its own funding accumulator as `∑ (mark - index) * dt`
//! (positive = mark trading above index/premium); Velocity's own precise
//! "who pays whom" wasn't independently re-confirmed against real
//! observed market behavior this pass. Treat the raw sign as unconfirmed
//! until checked.
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        8   Anchor discriminator
//!   8       32   pubkey (self-referential; not independently checked)
//!  40       32   amm.oracle (Pubkey)
//! 480        8   amm.last_funding_rate (i64, FUNDING_RATE_PRECISION = 1e9)
//! 488        8   amm.last_funding_rate_long (i64)
//! 496        8   amm.last_funding_rate_short (i64)
//! 504        8   amm.last_24h_avg_funding_rate (i64)
//! 752        8   amm.last_mark_price_twap (u64, PRICE_PRECISION = 1e6)
//! 760        8   amm.last_mark_price_twap_5min (u64)
//! 768        8   amm.last_update_slot (u64)
//! 792        8   amm.last_funding_rate_ts (i64, unix seconds)
//! 800        8   amm.funding_period (i64, seconds)
//! 1000      32   name ([u8; 32], space-padded ASCII, e.g. "SOL-PERP")
//! ```

use crate::graph::AccountId;
use solana_sdk::clock::Slot;

const OFF_LAST_FUNDING_RATE: usize = 480;
const OFF_LAST_FUNDING_RATE_LONG: usize = 488;
const OFF_LAST_FUNDING_RATE_SHORT: usize = 496;
const OFF_LAST_24H_AVG_FUNDING_RATE: usize = 504;
const OFF_LAST_MARK_PRICE_TWAP: usize = 752;
const OFF_LAST_MARK_PRICE_TWAP_5MIN: usize = 760;
const OFF_LAST_UPDATE_SLOT: usize = 768;
const OFF_LAST_FUNDING_RATE_TS: usize = 792;
const OFF_FUNDING_PERIOD: usize = 800;
const OFF_NAME: usize = 1000;
const NAME_LEN: usize = 32;

const MIN_LEN: usize = OFF_NAME + NAME_LEN;

/// `FUNDING_RATE_PRECISION` from `protocol-v2`'s `math/constants.rs`:
/// `PRICE_PRECISION (1e6) * FUNDING_RATE_BUFFER (1e3)`.
const FUNDING_RATE_PRECISION: f64 = 1_000_000_000.0;
/// `PRICE_PRECISION` from the same source.
const PRICE_PRECISION: f64 = 1_000_000.0;

/// A parsed Drift/Velocity `PerpMarket` account, pricing-only (no
/// position/authority fields -- same scope boundary as
/// `dex::phoenix::PhoenixMarketState`).
#[derive(Debug, Clone, Copy)]
pub struct VelocityPerpMarketView {
    /// Space-padded market name, e.g. `"SOL-PERP                        "`
    /// -- use [`Self::name_str`] for a trimmed view.
    pub name: [u8; 32],
    /// Raw funding rate for the last completed funding period,
    /// `FUNDING_RATE_PRECISION`-scaled -- see
    /// [`Self::funding_rate_usd_per_base`].
    pub last_funding_rate: i64,
    pub last_funding_rate_long: i64,
    pub last_funding_rate_short: i64,
    pub last_24h_avg_funding_rate: i64,
    /// Raw mark price TWAP, `PRICE_PRECISION`-scaled -- see
    /// [`Self::mark_price_usd`].
    pub last_mark_price_twap: u64,
    pub last_mark_price_twap_5min: u64,
    /// The slot this account was last updated at -- compare against the
    /// current slot to detect a stale/uncranked market, same idiom as
    /// this session's Pyth staleness check.
    pub last_update_slot: Slot,
    pub last_funding_rate_ts: i64,
    /// Seconds between funding updates (3600 = hourly, confirmed live).
    pub funding_period: i64,
    /// The market's underlying spot mint, resolved from
    /// `symbol_mint_config::SYMBOL_MINT_MAP` by [`VelocityState`](super::state::VelocityState)
    /// -- genuinely not present in the account bytes (a perp only needs an
    /// oracle price, not a token), so this is set by the caller, not
    /// [`parse_perp_market`], and only ever `None` for a symbol with no
    /// curated entry (e.g. no real liquid Solana mint at all).
    pub base_mint: Option<AccountId>,
}

impl VelocityPerpMarketView {
    /// Trimmed market name (trailing ASCII spaces removed), e.g. `"SOL-PERP"`.
    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.name).unwrap_or("").trim_end()
    }

    /// `last_mark_price_twap` converted to USD.
    pub fn mark_price_usd(&self) -> f64 {
        self.last_mark_price_twap as f64 / PRICE_PRECISION
    }

    /// `last_funding_rate` converted to USD per 1 whole base unit, for
    /// one `funding_period`. Divide by [`Self::mark_price_usd`] and
    /// annualize by `(365.0 * 86_400.0 / funding_period as f64)`
    /// periods/year to get an annualized rate comparable across venues
    /// -- left as a caller responsibility (`perp_router`) rather than
    /// baked in here, since annualization/edge-building policy belongs
    /// one layer up.
    pub fn funding_rate_usd_per_base(&self) -> f64 {
        self.last_funding_rate as f64 / FUNDING_RATE_PRECISION
    }
}

/// Parse a Drift/Velocity `PerpMarket` account. Returns `None` if the
/// account is too short to contain every field this parser reads.
///
/// Unlike `dex::phoenix`'s parsers, this does **not** check an Anchor
/// discriminator against a known constant -- the exact 8-byte
/// discriminator for `PerpMarket` wasn't independently computed this
/// pass (would need `sha256("account:PerpMarket")[..8]`, same method
/// used for Pyth's push-oracle discriminator earlier this session).
/// Callers must only pass bytes from an account already known to be a
/// `PerpMarket` (e.g. by subscribing to the PDA directly), not
/// arbitrary/untrusted account data.
pub fn parse_perp_market(body: &[u8]) -> Option<VelocityPerpMarketView> {
    if body.len() < MIN_LEN {
        return None;
    }
    let i64_at = |off: usize| i64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let u64_at = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());

    Some(VelocityPerpMarketView {
        name: body[OFF_NAME..OFF_NAME + NAME_LEN].try_into().unwrap(),
        last_funding_rate: i64_at(OFF_LAST_FUNDING_RATE),
        last_funding_rate_long: i64_at(OFF_LAST_FUNDING_RATE_LONG),
        last_funding_rate_short: i64_at(OFF_LAST_FUNDING_RATE_SHORT),
        last_24h_avg_funding_rate: i64_at(OFF_LAST_24H_AVG_FUNDING_RATE),
        last_mark_price_twap: u64_at(OFF_LAST_MARK_PRICE_TWAP),
        last_mark_price_twap_5min: u64_at(OFF_LAST_MARK_PRICE_TWAP_5MIN),
        last_update_slot: u64_at(OFF_LAST_UPDATE_SLOT),
        last_funding_rate_ts: i64_at(OFF_LAST_FUNDING_RATE_TS),
        funding_period: i64_at(OFF_FUNDING_PERIOD),
        base_mint: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real bytes captured from a live mainnet `PerpMarket` account
    /// (SOL-PERP, market_index=0,
    /// `8UJgxaiQx5nTrdDgph5FiahMmzduuLTLf5WmsPegYA6W`) during offset
    /// verification -- only the fields this parser reads, zero-padded
    /// out to `MIN_LEN` except `name`, which keeps its real space-padding.
    fn real_account_bytes() -> Vec<u8> {
        let mut body = vec![0u8; MIN_LEN];
        body[OFF_LAST_FUNDING_RATE..OFF_LAST_FUNDING_RATE + 8]
            .copy_from_slice(&(-1_024_958i64).to_le_bytes());
        body[OFF_LAST_FUNDING_RATE_LONG..OFF_LAST_FUNDING_RATE_LONG + 8]
            .copy_from_slice(&(-1_024_958i64).to_le_bytes());
        body[OFF_LAST_FUNDING_RATE_SHORT..OFF_LAST_FUNDING_RATE_SHORT + 8]
            .copy_from_slice(&(-1_024_958i64).to_le_bytes());
        body[OFF_LAST_24H_AVG_FUNDING_RATE..OFF_LAST_24H_AVG_FUNDING_RATE + 8]
            .copy_from_slice(&(-550_707i64).to_le_bytes());
        body[OFF_LAST_MARK_PRICE_TWAP..OFF_LAST_MARK_PRICE_TWAP + 8]
            .copy_from_slice(&84_321_048u64.to_le_bytes());
        body[OFF_LAST_MARK_PRICE_TWAP_5MIN..OFF_LAST_MARK_PRICE_TWAP_5MIN + 8]
            .copy_from_slice(&83_711_596u64.to_le_bytes());
        body[OFF_LAST_UPDATE_SLOT..OFF_LAST_UPDATE_SLOT + 8]
            .copy_from_slice(&410_366_402u64.to_le_bytes());
        body[OFF_LAST_FUNDING_RATE_TS..OFF_LAST_FUNDING_RATE_TS + 8]
            .copy_from_slice(&1_775_066_400i64.to_le_bytes());
        body[OFF_FUNDING_PERIOD..OFF_FUNDING_PERIOD + 8].copy_from_slice(&3600i64.to_le_bytes());
        let name = b"SOL-PERP                        ";
        body[OFF_NAME..OFF_NAME + NAME_LEN].copy_from_slice(&name[..NAME_LEN]);
        body
    }

    #[test]
    fn parses_real_account() {
        let market = parse_perp_market(&real_account_bytes()).expect("should parse");
        assert_eq!(market.name_str(), "SOL-PERP");
        assert_eq!(market.funding_period, 3600);
        assert!((market.mark_price_usd() - 84.321048).abs() < 1e-6);
        // -1_024_958 / 1e9 = -0.001024958 USD per base per funding period.
        assert!((market.funding_rate_usd_per_base() - (-0.001024958)).abs() < 1e-9);
    }

    #[test]
    fn rejects_too_short() {
        assert!(parse_perp_market(&[0u8; 10]).is_none());
    }
}
