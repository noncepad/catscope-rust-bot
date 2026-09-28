//! Stdin/stdout message serialization for `xstockshealthv1`. Mirrors
//! `testperplatencyv1lite::message`'s shape, trimmed further: `Wallet`
//! inbound (kept for harness-lifecycle consistency with every other bot
//! mode, unused for real sends since this mode doesn't transact yet), and
//! one real outbound message -- `HealthBoard` -- so the Go side
//! (`optimizer/brain/xstockshealthv1`) can serve the current board on a
//! local HTTP page instead of a person reading log lines.
use crate::{
    err::CatscopeGuestError,
    graph::AccountId,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
    trader::dex::kamino_xstocks_watcher::ObligationHealth,
};
use solana_sdk::signature::Keypair;
use std::{cell::UnsafeCell, rc::Rc};

pub enum CustomMessageInbound {
    Blank,
    Wallet(Rc<UnsafeCell<Keypair>>),
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

const CUSTOM_KEY_FLAG_WALLET: u8 = 3;

impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, body: &[u8]) -> Result<usize, CatscopeGuestError> {
        let kvp = KeyValuePair::try_from(body)?;
        let consumed = 1 + kvp.key().len() + 2 + kvp.value().len();
        let key = kvp.key();
        if key.len() != 1 {
            return Err(CatscopeGuestError::InsufficientBufferV2(key.len(), 1));
        }
        match key[0] {
            CUSTOM_KEY_FLAG_WALLET => {
                let value = kvp.value();
                if value.len() != 64 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 64));
                }
                let secret_key = {
                    let subbuf = &value[0..32];
                    Keypair::new_from_array(subbuf.try_into().unwrap())
                };
                let pubkey = solana_sdk::signer::Signer::pubkey(&secret_key);
                {
                    let subbuf = &value[32..];
                    let check_array = pubkey.as_array();
                    for i in 0..32 {
                        if subbuf[i] != check_array[i] {
                            return Err(CatscopeGuestError::InvalidPrivateKey);
                        }
                    }
                }
                *self = Self::Wallet(Rc::new(UnsafeCell::new(secret_key)));
            }
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(consumed)
    }
}

/// Wire key for [`CustomMessageOutbound::HealthBoard`] -- must match
/// `optimizer/brain/xstockshealthv1/message.go`'s `KeyFlagHealthBoard`
/// exactly. Per-strategy (this mode only), so it stays in the same
/// below-100 range every other `CUSTOM_KEY_FLAG_*`/`KeyFlag*` scheme in
/// this codebase uses (see `message.rs`'s own doc comment on
/// `COMMON_KEY_FLAG_ACCOUNT_USAGE` for why 100+ is reserved for
/// cross-strategy messages instead).
const KEY_FLAG_HEALTH_BOARD: u8 = 10;

/// Bytes per [`CustomMessageOutbound::HealthBoard`] entry on the wire:
/// account id (u64) + collateral_usd/debt_usd/health_factor (f64 each) +
/// collateral_ticker_id/debt_ticker_id (u8 each -- see
/// `kamino_xstocks_watcher::reserve_ticker_id`'s doc comment for why
/// these are a 1-byte index and not a ticker string), all little-endian
/// where it applies. `health_factor` is sent as-is, including a literal
/// IEEE-754 infinity for a debt-free obligation (see
/// `ObligationHealth::health_factor`'s own doc comment) -- the Go side
/// must handle that, not assume a finite float.
const HEALTH_BOARD_ENTRY_SIZE: usize = 8 + 8 + 8 + 8 + 1 + 1;

/// Wire key for [`CustomMessageOutbound::LiquidationEvent`] -- must match
/// `optimizer/brain/xstockshealthv1/message.go`'s `KeyFlagLiquidationEvent`
/// exactly. See [`KEY_FLAG_HEALTH_BOARD`]'s own doc comment for the
/// per-strategy key-range convention this follows.
const KEY_FLAG_LIQUIDATION_EVENT: u8 = 11;

/// Bytes for one [`CustomMessageOutbound::LiquidationEvent`]: obligation
/// id (u64) + repay/withdraw ticker (8 bytes each, ASCII, NUL-padded) +
/// repay_usd/health_factor_before/estimated_bonus_usd (f64 each) + slot
/// (u64) + the real transaction signature (64 raw bytes, not base58) +
/// sent_ok (1 byte, 0/1) -- all little-endian where it applies. One event
/// per message (unlike `HealthBoard`'s array); this bot only ever
/// attempts one liquidation per tick, see
/// `state.rs`'s `LIQUIDATE_COOLDOWN_SLOTS` doc comment.
const LIQUIDATION_EVENT_SIZE: usize = 8 + 8 + 8 + 8 + 8 + 8 + 8 + 64 + 1;

/// One real liquidation attempt's outcome -- see `state.rs`'s
/// `attempt_liquidate`/`evaluate` for the only producer.
/// `estimated_bonus_usd` is exactly that: computed from the reserve's
/// `max_liquidation_bonus_bps` at attempt time, not the real amount the
/// liquidation actually returned (which needs a post-hoc balance/tx-result
/// check this bot doesn't do yet) -- the dashboard must label it as an
/// estimate, not a confirmed realized profit.
pub struct LiquidationEvent {
    pub obligation: AccountId,
    pub repay_ticker: [u8; 8],
    pub withdraw_ticker: [u8; 8],
    pub repay_usd: f64,
    pub health_factor_before: f64,
    pub estimated_bonus_usd: f64,
    pub slot: u64,
    pub signature: [u8; 64],
    pub sent_ok: bool,
}

/// Wire key for [`CustomMessageOutbound::DexActivity`] -- must match
/// `optimizer/brain/xstockshealthv1/message.go`'s `KeyFlagDexActivity`
/// exactly.
const KEY_FLAG_DEX_ACTIVITY: u8 = 13;

/// Bytes per [`CustomMessageOutbound::DexActivity`] entry: ticker_id (u8,
/// via `kamino_xstocks_watcher::ticker_to_id` -- same table
/// `HealthBoard`'s ticker fields use) + price_usd (f64) + up (u8, 0/1) +
/// slot (u64), all little-endian where it applies.
const DEX_ACTIVITY_ENTRY_SIZE: usize = 1 + 8 + 1 + 8;

/// One real DEX price tick -- see `xstock_dex_watcher::DexTick`, the
/// only producer. Purely observational (see that module's own doc
/// comment): a live-activity signal, not something eligibility/
/// liquidation logic reads.
pub struct DexActivityEntry {
    pub ticker_id: u8,
    pub price_usd: f64,
    pub up: bool,
    pub slot: u64,
}

/// Wire key for [`CustomMessageOutbound::MarketOverview`] -- must match
/// `optimizer/brain/xstockshealthv1/message.go`'s `KeyFlagMarketOverview`
/// exactly.
const KEY_FLAG_MARKET_OVERVIEW: u8 = 14;

/// Fixed-header bytes for [`CustomMessageOutbound::MarketOverview`]:
/// reserves_monitored (u16) + pools_monitored (u16) + risk_tier_counts
/// (4 * u64) + arb_detected_usd (f64) + liquidation_pnl_usd (f64) +
/// arb_detection_count (u64), all little-endian. Followed by a variable
/// number of `arb_by_ticker` entries -- see
/// [`MARKET_OVERVIEW_TICKER_ENTRY_SIZE`].
const MARKET_OVERVIEW_FIXED_SIZE: usize = 2 + 2 + 4 * 8 + 8 + 8 + 8;

/// Bytes per `arb_by_ticker` entry: ticker_id (u8, same table
/// `HealthBoard`'s ticker fields use) + usd (f64).
const MARKET_OVERVIEW_TICKER_ENTRY_SIZE: usize = 1 + 8;

/// Coverage/PnL summary for the dashboard's overview section -- see
/// `state.rs`'s `check_arb`/`risk_tier_counts` for the two numbers that
/// most need an honesty label: `arb_detected_usd` is a *detected
/// opportunity size*, not realized profit (no trade executed);
/// `liquidation_pnl_usd` is real (only grows from an actual sent,
/// landed transaction) but still inherits `LiquidationEvent::
/// estimated_bonus_usd`'s own "estimate, not balance-verified" caveat.
pub struct MarketOverview {
    pub reserves_monitored: u16,
    pub pools_monitored: u16,
    /// `[eligible_now, at_risk, watch, safe]` -- see
    /// `KaminoXstocksWatcher::risk_tier_counts`'s own doc comment for
    /// the exact tier boundaries.
    pub risk_tier_counts: [u64; 4],
    pub arb_detected_usd: f64,
    pub liquidation_pnl_usd: f64,
    /// How many times `arb_detected_usd` has actually been added to
    /// (i.e. how many new, above-threshold gaps detected total) -- so
    /// the dashboard can show "$X across N detected gaps," not a bare
    /// dollar figure with no sense of how it accumulated.
    pub arb_detection_count: u64,
    /// `arb_detected_usd`'s running total broken out per ticker (ticker
    /// index, usd) -- at most 10 entries (one per xStock), only tickers
    /// with a nonzero total included.
    pub arb_by_ticker: Vec<(u8, f64)>,
}

pub enum CustomMessageOutbound {
    Blank,
    /// `total_tracked`: how many obligations the watcher actually has
    /// live data for right now (`KaminoXstocksWatcher::tracked_count`) --
    /// real, live-observed bug this fixes: the dashboard previously had
    /// no way to know this and showed `entries.len()` (capped at 100,
    /// see `HEALTH_BOARD_MAX_ENTRIES`) labeled as "tracked," which was
    /// simply wrong once the real market (thousands of obligations)
    /// exceeded that cap. `entries`: the top-100 worst (lowest
    /// `health_factor`) complete obligations -- see
    /// `KaminoXstocksWatcher::health_board`, the only producer.
    HealthBoard { total_tracked: u64, entries: Vec<(AccountId, ObligationHealth)> },
    /// One real liquidation attempt's outcome, pushed right after
    /// `Wallet::drain_and_send` returns the real transaction signature --
    /// see [`LiquidationEvent`]'s own doc comment.
    LiquidationEvent(LiquidationEvent),
    /// Recent real DEX price ticks -- see `xstock_dex_watcher`'s own
    /// module doc comment and [`DexActivityEntry`].
    DexActivity(Vec<DexActivityEntry>),
    /// Coverage/PnL summary -- see [`MarketOverview`]'s own doc comment.
    MarketOverview(MarketOverview),
}

impl CustomMessageOutbound {
    fn value_len(&self) -> usize {
        match self {
            Self::Blank => 0,
            // +8 for the leading total_tracked u64.
            Self::HealthBoard { entries, .. } => 8 + entries.len() * HEALTH_BOARD_ENTRY_SIZE,
            Self::LiquidationEvent(_) => LIQUIDATION_EVENT_SIZE,
            Self::DexActivity(entries) => entries.len() * DEX_ACTIVITY_ENTRY_SIZE,
            Self::MarketOverview(m) => {
                MARKET_OVERVIEW_FIXED_SIZE + m.arb_by_ticker.len() * MARKET_OVERVIEW_TICKER_ENTRY_SIZE
            }
        }
    }

    fn write_value(&self, buf: &mut Vec<u8>) {
        match self {
            Self::HealthBoard { total_tracked, entries } => {
                buf.extend_from_slice(&total_tracked.to_le_bytes());
                for (id, h) in entries {
                    buf.extend_from_slice(&id.to_le_bytes());
                    buf.extend_from_slice(&h.collateral_usd.to_le_bytes());
                    buf.extend_from_slice(&h.debt_usd.to_le_bytes());
                    buf.extend_from_slice(&h.health_factor.to_le_bytes());
                    buf.push(h.collateral_ticker_id);
                    buf.push(h.debt_ticker_id);
                }
            }
            Self::LiquidationEvent(ev) => {
                buf.extend_from_slice(&ev.obligation.to_le_bytes());
                buf.extend_from_slice(&ev.repay_ticker);
                buf.extend_from_slice(&ev.withdraw_ticker);
                buf.extend_from_slice(&ev.repay_usd.to_le_bytes());
                buf.extend_from_slice(&ev.health_factor_before.to_le_bytes());
                buf.extend_from_slice(&ev.estimated_bonus_usd.to_le_bytes());
                buf.extend_from_slice(&ev.slot.to_le_bytes());
                buf.extend_from_slice(&ev.signature);
                buf.push(if ev.sent_ok { 1 } else { 0 });
            }
            Self::DexActivity(entries) => {
                for e in entries {
                    buf.push(e.ticker_id);
                    buf.extend_from_slice(&e.price_usd.to_le_bytes());
                    buf.push(if e.up { 1 } else { 0 });
                    buf.extend_from_slice(&e.slot.to_le_bytes());
                }
            }
            Self::MarketOverview(m) => {
                buf.extend_from_slice(&m.reserves_monitored.to_le_bytes());
                buf.extend_from_slice(&m.pools_monitored.to_le_bytes());
                for c in &m.risk_tier_counts {
                    buf.extend_from_slice(&c.to_le_bytes());
                }
                buf.extend_from_slice(&m.arb_detected_usd.to_le_bytes());
                buf.extend_from_slice(&m.liquidation_pnl_usd.to_le_bytes());
                buf.extend_from_slice(&m.arb_detection_count.to_le_bytes());
                for (ticker_id, usd) in &m.arb_by_ticker {
                    buf.push(*ticker_id);
                    buf.extend_from_slice(&usd.to_le_bytes());
                }
            }
            Self::Blank => {}
        }
    }
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        let key = match self {
            Self::Blank => return 0,
            Self::HealthBoard { .. } => KEY_FLAG_HEALTH_BOARD,
            Self::LiquidationEvent(_) => KEY_FLAG_LIQUIDATION_EVENT,
            Self::DexActivity(_) => KEY_FLAG_DEX_ACTIVITY,
            Self::MarketOverview(_) => KEY_FLAG_MARKET_OVERVIEW,
        };
        // Matches KeyValuePair::len's framing exactly (1-byte key len +
        // key + 2-byte value len + value) -- delegated to below rather
        // than hand-duplicated, so the two can't drift.
        KeyValuePair { key: &[key], value: &[] }.len() + self.value_len()
    }
    fn is_empty(&self) -> bool {
        matches!(self, Self::Blank)
    }
    fn serialize(&self, buffer: &mut [u8]) {
        let key = match self {
            Self::Blank => return,
            Self::HealthBoard { .. } => KEY_FLAG_HEALTH_BOARD,
            Self::LiquidationEvent(_) => KEY_FLAG_LIQUIDATION_EVENT,
            Self::DexActivity(_) => KEY_FLAG_DEX_ACTIVITY,
            Self::MarketOverview(_) => KEY_FLAG_MARKET_OVERVIEW,
        };
        let mut value = Vec::with_capacity(self.value_len());
        self.write_value(&mut value);
        let kvp = KeyValuePair { key: &[key], value: &value };
        kvp.serialize(buffer);
    }
}
