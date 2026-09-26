//! Stdin/stdout message serialization for marketwatchv1.
//! Inbound (stdin): EchoRequest only -- this bot has no wallet/signing
//! state to receive (see `configuration.rs`'s own doc comment), so
//! there's no `KeyFlagWallet`/bundler-tip handling here, unlike every
//! trading bot's message.rs.
//! Outbound (stdout): EchoResponse, MarketStats.
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
    trader::market_stats::{MarketStatsSnapshot, N_PAIRS, N_SYMBOLS},
};

pub enum CustomMessageInbound {
    Blank,
    EchoRequest(String),
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

const CUSTOM_KEY_FLAG_ECHO_REQUEST: u8 = 1;
const CUSTOM_KEY_FLAG_ECHO_RESPONSE: u8 = 2;
const CUSTOM_KEY_FLAG_MARKET_STATS: u8 = 3;

/// `MarketStats`'s fixed wire-value size: 8-byte slot, 8-byte
/// updates-per-second (f64), 8-byte lifetime total update count (u64),
/// then `N_SYMBOLS` f64 prices, `N_SYMBOLS` f64 volatilities, `N_PAIRS`
/// f64 correlations, all little-endian. The Go side
/// (`optimizer/brain/marketwatchv1/message.go`) decodes this exact same
/// fixed layout -- see that file's own doc comment.
const MARKET_STATS_VALUE_SIZE: usize = 8 + 8 + 8 + N_SYMBOLS * 8 + N_SYMBOLS * 8 + N_PAIRS * 8;

impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, body: &[u8]) -> Result<usize, CatscopeGuestError> {
        let kvp = KeyValuePair::try_from(body)?;
        let consumed = 1 + kvp.key().len() + 2 + kvp.value().len();
        let key = kvp.key();
        if key.len() != 1 {
            return Err(CatscopeGuestError::InsufficientBufferV2(key.len(), 1));
        }
        match key[0] {
            CUSTOM_KEY_FLAG_ECHO_REQUEST => {
                let s = std::str::from_utf8(kvp.value()).unwrap_or("").to_string();
                *self = Self::EchoRequest(s);
            }
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(consumed)
    }
}

pub enum CustomMessageOutbound {
    EchoResponse(String),
    /// `slot` is when this snapshot was pushed -- lets the Go side/TUI
    /// show real data-age instead of assuming every push is instant.
    /// `updates_per_sec`/`total_updates` are throughput diagnostics (see
    /// `trader::market_stats::updates_per_second`'s own doc comment) --
    /// deliberately siblings of `snapshot`, not fields inside
    /// `MarketStatsSnapshot` itself, since they describe this bot's own
    /// data pipeline, not per-symbol market data. `snapshot` is boxed
    /// (real, CI-confirmed: `MarketStatsSnapshot`'s 6 prices + 6
    /// volatilities + 15 correlations, all `f64`, is 216 bytes, which
    /// blew this enum's smallest variant (`EchoResponse`'s ~24 bytes)
    /// past clippy's `large_enum_variant` threshold denied in CI) --
    /// every other variant, and every caller matching on this one,
    /// stays exactly as-is; only the one field that was actually large
    /// moved onto the heap.
    MarketStats {
        slot: u64,
        snapshot: Box<MarketStatsSnapshot>,
        updates_per_sec: f64,
        total_updates: u64,
    },
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        match self {
            Self::EchoResponse(s) => {
                let key = [CUSTOM_KEY_FLAG_ECHO_RESPONSE];
                let kvp = KeyValuePair { key: &key, value: s.as_bytes() };
                kvp.len()
            }
            Self::MarketStats { .. } => {
                // key_len(1) + key(1) + value_len(2) + value
                1 + 1 + 2 + MARKET_STATS_VALUE_SIZE
            }
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::EchoResponse(s) => s.is_empty(),
            Self::MarketStats { .. } => false,
        }
    }
    fn serialize(&self, buffer: &mut [u8]) {
        match self {
            Self::EchoResponse(s) => {
                let key = [CUSTOM_KEY_FLAG_ECHO_RESPONSE];
                let kvp = KeyValuePair { key: &key, value: s.as_bytes() };
                kvp.serialize(buffer);
            }
            Self::MarketStats { slot, snapshot, updates_per_sec, total_updates } => {
                let key = [CUSTOM_KEY_FLAG_MARKET_STATS];
                let mut value = [0u8; MARKET_STATS_VALUE_SIZE];
                let mut i = 0;
                value[i..i + 8].copy_from_slice(&slot.to_le_bytes());
                i += 8;
                value[i..i + 8].copy_from_slice(&updates_per_sec.to_le_bytes());
                i += 8;
                value[i..i + 8].copy_from_slice(&total_updates.to_le_bytes());
                i += 8;
                for p in snapshot.prices {
                    value[i..i + 8].copy_from_slice(&p.to_le_bytes());
                    i += 8;
                }
                for v in snapshot.volatilities {
                    value[i..i + 8].copy_from_slice(&v.to_le_bytes());
                    i += 8;
                }
                for c in snapshot.correlations {
                    value[i..i + 8].copy_from_slice(&c.to_le_bytes());
                    i += 8;
                }
                debug_assert_eq!(i, MARKET_STATS_VALUE_SIZE);
                let kvp = KeyValuePair { key: &key, value: &value };
                kvp.serialize(buffer);
            }
        }
    }
}
