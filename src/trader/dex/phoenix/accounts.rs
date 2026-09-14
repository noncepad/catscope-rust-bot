//! Raw byte-offset parsing for Phoenix perpetuals account layouts.
//!
//! Every offset below was derived from Ellipsis Labs' public `rise-public`
//! SDK source (`rust/accounts/src/{global_config,perp_asset_map/*,trader/mod,multi_arena}.rs`)
//! by summing each struct's fields in declaration order using the crate's
//! own `const_assert_eq!(size_of::<...>(), N)` totals as a cross-check --
//! every intermediate and final size below was verified to add up exactly
//! to the source's own assertions. The derivation was then independently
//! validated against real, live mainnet account bytes (not just the
//! source): decoding `GlobalConfiguration` at
//! `2zskx2iyCvb6Stg7RBZkt1f6MrF4dpYtMG3yMvKwqtUZ` round-trips its own
//! embedded `account_key` back to the queried pubkey, and decoding
//! `PerpAssetMap` at `2nHGAaEw3D5dd4hVueaUNoygkQFmoeKqRQWnSPqSMFUC` produced
//! 65 real, sane entries (symbols SOL/BTC/ETH/XRP/... with ascending
//! `asset_id`s 0..64, plausible per-symbol max-leverage tiers: SOL 25x,
//! BTC 40x, ETH 25x) -- ruling out an off-by-a-field misalignment, since a
//! wrong offset would not decode into coherent symbols/leverage/ascending
//! IDs by chance.
//!
//! `phoenix_rise_math`'s quantity newtypes (`Ticks`, `BasisPoints`,
//! `BaseLots`, `Constant`, `SignedQuoteLotsPerBaseLot`, ...) all wrap a
//! plain `u64`/`i64` (verified from `rust/math/src/quantities/types.rs`'s
//! `basic_u64_struct!`/`basic_i64_struct!` macro invocations) -- so every
//! field below is read as a plain 8-byte little-endian integer, not a
//! bit-packed type, with one exception: `TraderPosition`'s
//! `accumulated_funding_for_active_position` is a 7-byte (56-bit) signed
//! packed integer (`SignedQuoteLotsI56` -- `TraderPositionRaw` is exactly
//! 32 bytes total and every other field accounts for 25 of them).
//!
//! **Known gap**: converting a raw `Ticks` value into a human quote-per-base
//! price requires the `Market`/orderbook account's own base-lot/quote-lot
//! conversion factors, which were not read in this pass (out of scope --
//! margin math here works directly in raw ticks/lots, matching every other
//! quantity Phoenix itself uses internally).

use solana_sdk::pubkey::Pubkey;

// ─── GlobalConfiguration (discriminator sha256("account:global_configuration")[..8]) ──
// Prefix is 776 bytes (`GlobalConfigPrefixRaw`, verified via const_assert).
pub const GLOBAL_CONFIG_LEN: usize = 776;
const OFF_GC_CANONICAL_TOKEN_MINT: usize = 296;
const OFF_GC_GLOBAL_VAULT: usize = 328;
const OFF_GC_PERP_ASSET_MAP: usize = 360;
const OFF_GC_GLOBAL_TRADER_INDEX_HEADER: usize = 392;
const OFF_GC_ACTIVE_TRADER_BUFFER_HEADER: usize = 424;
const OFF_GC_WITHDRAW_QUEUE: usize = 472;
const OFF_GC_EXCHANGE_STATUS: usize = 504;
const OFF_GC_QUOTE_DECIMALS: usize = 505;

#[derive(Debug, Clone, Copy)]
pub struct GlobalConfigView {
    pub canonical_token_mint: Pubkey,
    pub global_vault: Pubkey,
    pub perp_asset_map: Pubkey,
    pub global_trader_index_header: Pubkey,
    pub active_trader_buffer_header: Pubkey,
    pub withdraw_queue: Pubkey,
    pub exchange_status: u8,
    pub quote_decimals: u8,
}

pub fn parse_global_config(data: &[u8]) -> Option<GlobalConfigView> {
    if data.len() < GLOBAL_CONFIG_LEN {
        return None;
    }
    let pk = |off: usize| Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
    Some(GlobalConfigView {
        canonical_token_mint: pk(OFF_GC_CANONICAL_TOKEN_MINT),
        global_vault: pk(OFF_GC_GLOBAL_VAULT),
        perp_asset_map: pk(OFF_GC_PERP_ASSET_MAP),
        global_trader_index_header: pk(OFF_GC_GLOBAL_TRADER_INDEX_HEADER),
        active_trader_buffer_header: pk(OFF_GC_ACTIVE_TRADER_BUFFER_HEADER),
        withdraw_queue: pk(OFF_GC_WITHDRAW_QUEUE),
        exchange_status: data[OFF_GC_EXCHANGE_STATUS],
        quote_decimals: data[OFF_GC_QUOTE_DECIMALS],
    })
}

// ─── MultiArenaHeader + SuperblockView (shared prefix of GlobalTraderIndex
// and ActiveTraderBuffer accounts) ──────────────────────────────────────
// `num_arenas` tells the caller how many accounts (including this header)
// to pass as `global_trader_index`/`active_trader_buffer` remaining
// accounts. Verified live: both are `1` on current mainnet, so today the
// remaining-accounts list is just `[header_key]` -- but this must be read
// live every time, not hardcoded, since it can grow.
const OFF_ARENA_NUM_ARENAS: usize = 52;

pub fn parse_num_arenas(data: &[u8]) -> Option<u16> {
    if data.len() < 54 {
        return None;
    }
    Some(u16::from_le_bytes(data[OFF_ARENA_NUM_ARENAS..OFF_ARENA_NUM_ARENAS + 2].try_into().unwrap()))
}

// ─── PerpAssetMap (discriminator sha256("account:perp_asset_map")[..8]) ──
// Single shared account for the whole exchange (not per-market). Header is
// 48 bytes (`PerpAssetMapHeader`), followed by up to 1024 fixed-size 1584-
// byte entries (`symbol: [u8;16]` + `metadata: PerpAssetMetadataLayout`,
// 1568 bytes).
pub const PERP_ASSET_MAP_HEADER_LEN: usize = 48;
pub const PERP_ASSET_MAP_ENTRY_LEN: usize = 1584;
pub const PERP_ASSET_MAP_MAX_ENTRIES: usize = 1024;
const OFF_PAM_SLOTS_USED: usize = 32;

/// Live entry count (`StableIndexedShortMapHeader.slots_used`) -- the
/// number of populated slots to scan, out of the fixed 1024-slot capacity.
pub fn parse_perp_asset_map_slots_used(data: &[u8]) -> Option<u32> {
    if data.len() < PERP_ASSET_MAP_HEADER_LEN {
        return None;
    }
    Some(u32::from_le_bytes(data[OFF_PAM_SLOTS_USED..OFF_PAM_SLOTS_USED + 4].try_into().unwrap()))
}

/// One market's parsed metadata entry.
#[derive(Debug, Clone, Copy)]
pub struct PerpAssetEntryView {
    pub symbol: [u8; 16],
    pub asset_id: u32,
    /// The `Market`/orderbook account for this asset -- `market_account` in
    /// the SDK, `orderbook` in every order/cancel instruction's account
    /// list. Same account, two names depending on context.
    pub market_account: Pubkey,
    pub tick_size: u64,
    pub base_lot_decimals: i8,
    /// `leverage_tiers[0]` only (the most permissive/first tier) -- the
    /// other 3 tiers exist for position-size-scaled leverage reduction,
    /// not modeled here; see the module doc's documented-simplification
    /// convention.
    pub tier0_upper_bound_size: u64,
    pub tier0_max_leverage: u64,
    pub cumulative_funding_rate: i64,
    pub open_interest: u64,
    pub open_interest_cap: u64,
    /// Continuously-updated, oracle-blended mark price in raw `Ticks` --
    /// `oracle_price.mark_price.price.ticks` in Ellipsis Labs' real
    /// `rise-public` SDK (`PerpAssetMetadataLayout.oracle_price:
    /// PriceComponent`, `PriceComponent.mark_price: MarkPrice`,
    /// `MarkPrice.price: TicksAtSlot`). **Not** the field this bot used
    /// before this session's fix (`finalized_mark_price`) -- that one is
    /// a *different* field, only ever written by `MarketClosedEvent`
    /// when a market permanently closes/delists (confirmed from the
    /// real SDK source, `rust/events/src/market_events/admin.rs`'s doc
    /// comment: "Event emitted when a market is closed with its
    /// finalized settlement price"), so it reads `0` forever for any
    /// actively-trading market -- which is exactly what SOL/BTC/ETH's
    /// real live entries showed. This field, by contrast, was
    /// live-verified this session: real ticks for SOL/BTC/ETH converted
    /// (via `PhoenixMarketState::mark_price_usd`'s formula) to $75.62 /
    /// $63,118 / $1,884, and its own recorded slot was ~38 slots behind
    /// the real current mainnet slot (~15s old) -- genuinely live, not
    /// stale or zero.
    pub oracle_mark_price_ticks: u64,
    pub is_tombstoned: bool,
}

// Offsets are relative to the start of a 1584-byte entry (symbol[16] +
// metadata[1568]).
const OFF_ENTRY_SYMBOL: usize = 0;
const OFF_ENTRY_METADATA: usize = 16;
// Offsets below are relative to OFF_ENTRY_METADATA (i.e. add OFF_ENTRY_METADATA).
// oracle_price: PriceComponent { price_sequence_number: SequenceNumber (16
// bytes), mark_price: MarkPrice { price: TicksAtSlot { slot: u64, ticks:
// Ticks }, ... } } -- ticks sits at 16 (past price_sequence_number) + 8
// (past TicksAtSlot.slot) = 24. Cross-checked against this file's own
// already-verified OFF_MD_STATIC_MARKET_PARAMS below: PriceComponent is
// 888 bytes in the real SDK (`const_assert_eq!(size_of::<PriceComponent>(),
// 888)`), and 888 + 16 bytes of padding0 = 904, exactly matching.
const OFF_MD_ORACLE_MARK_PRICE: usize = 24;
const OFF_MD_STATIC_MARKET_PARAMS: usize = 904;
const OFF_SMP_MARKET_ACCOUNT: usize = 0;
const OFF_SMP_TICK_SIZE: usize = 32;
const OFF_SMP_ASSET_ID_LOWER: usize = 40;
const OFF_SMP_BASE_LOT_DECIMALS: usize = 42;
const OFF_SMP_ASSET_ID_UPPER: usize = 44;
const OFF_MD_RISK_PARAMS: usize = 1016;
const OFF_MD_FUNDING_ACCUMULATOR: usize = 1280;
const OFF_FA_CUMULATIVE_FUNDING_RATE: usize = 32;
const OFF_MD_OPEN_INTEREST_PARAMS: usize = 1424;
const OFF_MD_SHORT_MAP_METADATA: usize = 1448;

fn parse_entry(entry: &[u8]) -> PerpAssetEntryView {
    let u64_at = |off: usize| u64::from_le_bytes(entry[off..off + 8].try_into().unwrap());
    let i64_at = |off: usize| i64::from_le_bytes(entry[off..off + 8].try_into().unwrap());
    let u16_at = |off: usize| u16::from_le_bytes(entry[off..off + 2].try_into().unwrap());
    let pk_at = |off: usize| Pubkey::new_from_array(entry[off..off + 32].try_into().unwrap());

    let md = OFF_ENTRY_METADATA;
    let smp = md + OFF_MD_STATIC_MARKET_PARAMS;
    let asset_id_lower = u16_at(smp + OFF_SMP_ASSET_ID_LOWER) as u32;
    let asset_id_upper = u16_at(smp + OFF_SMP_ASSET_ID_UPPER) as u32;
    let risk = md + OFF_MD_RISK_PARAMS;
    let funding = md + OFF_MD_FUNDING_ACCUMULATOR;
    let oi = md + OFF_MD_OPEN_INTEREST_PARAMS;

    let mut symbol = [0u8; 16];
    symbol.copy_from_slice(&entry[OFF_ENTRY_SYMBOL..OFF_ENTRY_SYMBOL + 16]);

    PerpAssetEntryView {
        symbol,
        asset_id: asset_id_lower | (asset_id_upper << 16),
        market_account: pk_at(smp + OFF_SMP_MARKET_ACCOUNT),
        tick_size: u64_at(smp + OFF_SMP_TICK_SIZE),
        base_lot_decimals: entry[smp + OFF_SMP_BASE_LOT_DECIMALS] as i8,
        tier0_upper_bound_size: u64_at(risk),
        tier0_max_leverage: u64_at(risk + 8),
        cumulative_funding_rate: i64_at(funding + OFF_FA_CUMULATIVE_FUNDING_RATE),
        open_interest: u64_at(oi),
        open_interest_cap: u64_at(oi + 8),
        oracle_mark_price_ticks: u64_at(md + OFF_MD_ORACLE_MARK_PRICE),
        is_tombstoned: entry[md + OFF_MD_SHORT_MAP_METADATA + 2] != 0,
    }
}

/// Scan the map's populated slots (`0..slots_used`, capped defensively at
/// the fixed 1024 capacity) for the first active (non-tombstoned) entry
/// matching `asset_id`. O(slots_used) -- fine at today's live scale (65
/// entries) and bounded even at the theoretical max (1024).
pub fn find_perp_asset_by_id(data: &[u8], asset_id: u32) -> Option<PerpAssetEntryView> {
    let slots_used = parse_perp_asset_map_slots_used(data)? as usize;
    let n = slots_used.min(PERP_ASSET_MAP_MAX_ENTRIES);
    for i in 0..n {
        let start = PERP_ASSET_MAP_HEADER_LEN + i * PERP_ASSET_MAP_ENTRY_LEN;
        let end = start + PERP_ASSET_MAP_ENTRY_LEN;
        if data.len() < end {
            break;
        }
        let entry = parse_entry(&data[start..end]);
        if !entry.is_tombstoned && entry.asset_id == asset_id {
            return Some(entry);
        }
    }
    None
}

// ─── Trader / TraderPosition (discriminator sha256("account:trader")[..8]) ──
// Fixed 224-byte header (`TraderHeader`), then a 16-byte position-map
// prefix (`len: u64, capacity: u64`), then `len` fixed-size 40-byte
// `TraderPositionEntry` records.
pub const TRADER_HEADER_LEN: usize = 224;
const OFF_TH_AUTHORITY: usize = 56;
const OFF_TH_QUOTE_LOT_COLLATERAL: usize = 88;
// `TraderState.flags: u32` (`rust/accounts/src/trader/mod.rs`) -- the real
// on-chain `TraderCapabilityFlags` bitmask. `TraderState` starts right
// after `authority` (56+32=88, matching OFF_TH_QUOTE_LOT_COLLATERAL) and
// is `quote_lot_collateral: i64 (8) + flags: u32 (4) + ...`, so flags
// sits at 88+8=96. Cross-checked: this derivation also correctly
// predicts OFF_TH_MAX_POSITIONS=112 three fields later
// (`TraderState`'s remaining 4 bytes + 4-byte padding + withdraw_queue_
// node: u32 = 96+4+1+1+1+1+4+4=112), so all three pre-existing offsets
// and this new one agree on one single, consistent field layout -- not
// independently guessed. Real, live-confirmed need (2026-09-04): a
// dispersion margin top-up failed on-chain with `TradersViewError::
// CapabilityDenied { capability: DepositCollateral }` /
// `TradersViewError::TraderFrozen` -- this trader account's flags were
// live-confirmed to be `0x00000006` (CAN_PLACE_LIMIT|CAN_PLACE_MARKET
// only), which exactly matches `TraderCapabilityFlags::frozen()`'s own
// real definition upstream. Reading this field lets the caller check
// `TraderHeaderView::is_frozen()`/`can_deposit()` before attempting a
// deposit, instead of discovering a frozen account only by a real,
// fee-costing failed transaction every single cycle.
const OFF_TH_CAPABILITY_FLAGS: usize = 96;
const OFF_TH_MAX_POSITIONS: usize = 112;
const OFF_TH_POSITION_MAP_LEN: usize = 224;

// Real `TraderCapabilityFlags` bit values (`rust/accounts/src/trader/
// capabilities.rs`) -- only the ones this codebase actually needs to
// check are reproduced here, not the whole upstream bitflags API.
const TRADER_CAPABILITY_CAN_PLACE_MARKET: u32 = 1 << 2;
const TRADER_CAPABILITY_CAN_RISK_INCREASE: u32 = 1 << 3;
const TRADER_CAPABILITY_CAN_DEPOSIT: u32 = 1 << 4;
const TRADER_CAPABILITY_CAN_WITHDRAW: u32 = 1 << 5;
pub const TRADER_POSITION_ENTRIES_START: usize = 240;
pub const TRADER_POSITION_ENTRY_LEN: usize = 40;

#[derive(Debug, Clone, Copy)]
pub struct TraderHeaderView {
    pub authority: Pubkey,
    /// Signed quote-lot collateral balance -- negative would indicate the
    /// account owes the protocol (shouldn't happen under normal operation,
    /// but read as signed to match the real on-chain type exactly).
    pub quote_lot_collateral: i64,
    /// Raw `TraderCapabilityFlags` bitmask -- see [`Self::is_frozen`]/
    /// [`Self::can_deposit`] for the checks callers actually need instead
    /// of hand-rolling bit tests against this field directly.
    pub capability_flags: u32,
    pub max_positions: u32,
    pub position_count: u64,
}

/// `true` once `DepositFunds` is real safe to attempt for a trader account
/// with these raw `TraderCapabilityFlags` bits -- callers (including
/// [`TraderHeaderView::can_deposit`] and `PhoenixState::can_deposit`, which
/// share this so neither can drift from the other) must check this before
/// sending a deposit, not just on a caught on-chain failure; a frozen
/// account fails the exact same way on every single retry (real,
/// live-confirmed this session).
pub fn capability_can_deposit(flags: u32) -> bool {
    flags & TRADER_CAPABILITY_CAN_DEPOSIT != 0
}

/// Mirrors the real upstream `is_trader_frozen` predicate exactly: can
/// still place market orders (to reduce risk) but has lost
/// deposit/withdraw/risk-increase -- the real state the protocol itself
/// calls "frozen" (confirmed live via the account's real
/// `TradersViewError::TraderFrozen` on-chain log).
pub fn capability_is_frozen(flags: u32) -> bool {
    flags & TRADER_CAPABILITY_CAN_PLACE_MARKET != 0
        && flags & TRADER_CAPABILITY_CAN_RISK_INCREASE == 0
        && flags & TRADER_CAPABILITY_CAN_WITHDRAW == 0
        && flags & TRADER_CAPABILITY_CAN_DEPOSIT == 0
}

impl TraderHeaderView {
    /// See [`capability_can_deposit`].
    pub fn can_deposit(&self) -> bool {
        capability_can_deposit(self.capability_flags)
    }

    /// See [`capability_is_frozen`].
    pub fn is_frozen(&self) -> bool {
        capability_is_frozen(self.capability_flags)
    }
}

pub fn parse_trader_header(data: &[u8]) -> Option<TraderHeaderView> {
    if data.len() < TRADER_HEADER_LEN {
        return None;
    }
    Some(TraderHeaderView {
        authority: Pubkey::new_from_array(
            data[OFF_TH_AUTHORITY..OFF_TH_AUTHORITY + 32].try_into().unwrap(),
        ),
        quote_lot_collateral: i64::from_le_bytes(
            data[OFF_TH_QUOTE_LOT_COLLATERAL..OFF_TH_QUOTE_LOT_COLLATERAL + 8]
                .try_into()
                .unwrap(),
        ),
        capability_flags: u32::from_le_bytes(
            data[OFF_TH_CAPABILITY_FLAGS..OFF_TH_CAPABILITY_FLAGS + 4].try_into().unwrap(),
        ),
        max_positions: u32::from_le_bytes(
            data[OFF_TH_MAX_POSITIONS..OFF_TH_MAX_POSITIONS + 4].try_into().unwrap(),
        ),
        position_count: u64::from_le_bytes(
            data[OFF_TH_POSITION_MAP_LEN..OFF_TH_POSITION_MAP_LEN + 8].try_into().unwrap(),
        ),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct TraderPositionView {
    pub asset_id: u64,
    pub base_lot_position: i64,
    pub virtual_quote_lot_position: i64,
    pub cumulative_funding_snapshot: i64,
    /// `accumulated_funding_for_active_position` is a packed 56-bit (7
    /// byte) signed integer on-chain (`SignedQuoteLotsI56`) -- read as 7
    /// little-endian bytes and sign-extended to i64 here.
    pub accumulated_funding_for_active_position: i64,
}

fn sign_extend_i56(bytes: &[u8]) -> i64 {
    debug_assert_eq!(bytes.len(), 7);
    let mut buf = [0u8; 8];
    buf[..7].copy_from_slice(bytes);
    let unsigned = i64::from_le_bytes(buf);
    // Sign bit is bit 55 (the top bit of the 7th byte).
    if bytes[6] & 0x80 != 0 {
        unsigned | (!0i64 << 56)
    } else {
        unsigned
    }
}

/// Parse the position entry at index `i` (`0..position_count` from
/// [`TraderHeaderView::position_count`]). Returns `None` if `data` doesn't
/// extend far enough to cover it.
pub fn parse_trader_position(data: &[u8], i: usize) -> Option<TraderPositionView> {
    let start = TRADER_POSITION_ENTRIES_START + i * TRADER_POSITION_ENTRY_LEN;
    let end = start + TRADER_POSITION_ENTRY_LEN;
    if data.len() < end {
        return None;
    }
    let e = &data[start..end];
    let i64_at = |off: usize| i64::from_le_bytes(e[off..off + 8].try_into().unwrap());
    Some(TraderPositionView {
        asset_id: u64::from_le_bytes(e[0..8].try_into().unwrap()),
        base_lot_position: i64_at(8),
        virtual_quote_lot_position: i64_at(16),
        cumulative_funding_snapshot: i64_at(24),
        accumulated_funding_for_active_position: sign_extend_i56(&e[33..40]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn disc(name: &str) -> [u8; 8] {
        Sha256::digest(format!("account:{name}").as_bytes())[..8].try_into().unwrap()
    }

    #[test]
    fn account_discriminants_are_well_formed() {
        // Not gated on at runtime (routing is done by known pubkey identity,
        // same convention as pumpswap.rs/pumpfun.rs), but worth asserting
        // these compute to something non-degenerate.
        assert_ne!(disc("global_configuration"), [0u8; 8]);
        assert_ne!(disc("perp_asset_map"), [0u8; 8]);
        assert_ne!(disc("trader"), [0u8; 8]);
        assert_ne!(disc("global_configuration"), disc("trader"));
    }

    fn synthetic_global_config() -> Vec<u8> {
        let mut d = vec![0u8; GLOBAL_CONFIG_LEN];
        let canonical = Pubkey::new_unique();
        let vault = Pubkey::new_unique();
        let pam = Pubkey::new_unique();
        let gti = Pubkey::new_unique();
        let atb = Pubkey::new_unique();
        d[OFF_GC_CANONICAL_TOKEN_MINT..OFF_GC_CANONICAL_TOKEN_MINT + 32]
            .copy_from_slice(canonical.as_ref());
        d[OFF_GC_GLOBAL_VAULT..OFF_GC_GLOBAL_VAULT + 32].copy_from_slice(vault.as_ref());
        d[OFF_GC_PERP_ASSET_MAP..OFF_GC_PERP_ASSET_MAP + 32].copy_from_slice(pam.as_ref());
        d[OFF_GC_GLOBAL_TRADER_INDEX_HEADER..OFF_GC_GLOBAL_TRADER_INDEX_HEADER + 32]
            .copy_from_slice(gti.as_ref());
        d[OFF_GC_ACTIVE_TRADER_BUFFER_HEADER..OFF_GC_ACTIVE_TRADER_BUFFER_HEADER + 32]
            .copy_from_slice(atb.as_ref());
        d[OFF_GC_QUOTE_DECIMALS] = 6;
        d
    }

    #[test]
    fn parses_global_config_fields_at_the_right_offsets() {
        let d = synthetic_global_config();
        let v = parse_global_config(&d).unwrap();
        assert_eq!(v.quote_decimals, 6);
        assert_ne!(v.canonical_token_mint, v.global_vault);
        assert_ne!(v.perp_asset_map, v.global_trader_index_header);
    }

    fn synthetic_entry(asset_id: u32, symbol: &str, tombstoned: bool) -> Vec<u8> {
        let mut e = vec![0u8; PERP_ASSET_MAP_ENTRY_LEN];
        e[0..symbol.len()].copy_from_slice(symbol.as_bytes());
        let smp = OFF_ENTRY_METADATA + OFF_MD_STATIC_MARKET_PARAMS;
        let market = Pubkey::new_unique();
        e[smp + OFF_SMP_MARKET_ACCOUNT..smp + OFF_SMP_MARKET_ACCOUNT + 32]
            .copy_from_slice(market.as_ref());
        e[smp + OFF_SMP_ASSET_ID_LOWER..smp + OFF_SMP_ASSET_ID_LOWER + 2]
            .copy_from_slice(&((asset_id & 0xFFFF) as u16).to_le_bytes());
        e[smp + OFF_SMP_ASSET_ID_UPPER..smp + OFF_SMP_ASSET_ID_UPPER + 2]
            .copy_from_slice(&(((asset_id >> 16) & 0xFFFF) as u16).to_le_bytes());
        e[OFF_ENTRY_METADATA + OFF_MD_SHORT_MAP_METADATA + 2] = tombstoned as u8;
        e
    }

    #[test]
    fn finds_active_asset_by_id_and_skips_tombstoned() {
        let mut data = vec![0u8; PERP_ASSET_MAP_HEADER_LEN];
        data[OFF_PAM_SLOTS_USED..OFF_PAM_SLOTS_USED + 4].copy_from_slice(&3u32.to_le_bytes());
        data.extend(synthetic_entry(0, "SOL", false));
        data.extend(synthetic_entry(1, "BTC", true)); // tombstoned
        data.extend(synthetic_entry(2, "ETH", false));

        assert_eq!(find_perp_asset_by_id(&data, 0).unwrap().symbol[..3], *b"SOL");
        assert!(find_perp_asset_by_id(&data, 1).is_none());
        assert_eq!(find_perp_asset_by_id(&data, 2).unwrap().symbol[..3], *b"ETH");
        assert!(find_perp_asset_by_id(&data, 99).is_none());
    }

    #[test]
    fn sign_extends_negative_i56_correctly() {
        // -1 in 56-bit two's complement is all-ones.
        assert_eq!(sign_extend_i56(&[0xFF; 7]), -1i64);
        // 0 stays 0.
        assert_eq!(sign_extend_i56(&[0x00; 7]), 0i64);
        // Positive value with the sign bit clear.
        assert_eq!(sign_extend_i56(&[0x01, 0, 0, 0, 0, 0, 0]), 1i64);
    }

    #[test]
    fn parses_trader_header_and_positions() {
        let mut d = vec![0u8; TRADER_POSITION_ENTRIES_START + TRADER_POSITION_ENTRY_LEN];
        let authority = Pubkey::new_unique();
        d[OFF_TH_AUTHORITY..OFF_TH_AUTHORITY + 32].copy_from_slice(authority.as_ref());
        d[OFF_TH_QUOTE_LOT_COLLATERAL..OFF_TH_QUOTE_LOT_COLLATERAL + 8]
            .copy_from_slice(&1_000_000i64.to_le_bytes());
        // hot_active: every capability bit set (real "fully ready" state).
        d[OFF_TH_CAPABILITY_FLAGS..OFF_TH_CAPABILITY_FLAGS + 4]
            .copy_from_slice(&0x0000_003Fu32.to_le_bytes());
        d[OFF_TH_MAX_POSITIONS..OFF_TH_MAX_POSITIONS + 4].copy_from_slice(&128u32.to_le_bytes());
        d[OFF_TH_POSITION_MAP_LEN..OFF_TH_POSITION_MAP_LEN + 8]
            .copy_from_slice(&1u64.to_le_bytes());
        let pos_off = TRADER_POSITION_ENTRIES_START;
        d[pos_off..pos_off + 8].copy_from_slice(&0u64.to_le_bytes()); // asset_id = 0
        d[pos_off + 8..pos_off + 16].copy_from_slice(&500i64.to_le_bytes()); // base_lot_position

        let header = parse_trader_header(&d).unwrap();
        assert_eq!(header.authority, authority);
        assert_eq!(header.quote_lot_collateral, 1_000_000);
        assert_eq!(header.capability_flags, 0x0000_003F);
        assert!(header.can_deposit());
        assert!(!header.is_frozen());
        assert_eq!(header.position_count, 1);

        let pos = parse_trader_position(&d, 0).unwrap();
        assert_eq!(pos.asset_id, 0);
        assert_eq!(pos.base_lot_position, 500);
    }

    fn view_with_flags(capability_flags: u32) -> TraderHeaderView {
        TraderHeaderView {
            authority: Pubkey::new_unique(),
            quote_lot_collateral: 0,
            capability_flags,
            max_positions: 128,
            position_count: 0,
        }
    }

    #[test]
    fn is_frozen_true_for_the_real_live_confirmed_frozen_bit_pattern() {
        // 0x00000006 = CAN_PLACE_LIMIT | CAN_PLACE_MARKET only -- the
        // exact real on-chain value read from this session's live-
        // confirmed frozen trader account (see this module's own
        // OFF_TH_CAPABILITY_FLAGS doc comment).
        let view = view_with_flags(0x0000_0006);
        assert!(view.is_frozen());
        assert!(!view.can_deposit());
    }

    #[test]
    fn is_frozen_false_for_hot_active() {
        let view = view_with_flags(0x0000_003F); // every real capability bit set
        assert!(!view.is_frozen());
        assert!(view.can_deposit());
    }

    #[test]
    fn is_frozen_false_for_uninitialized() {
        // All-zero flags fails is_frozen's own CAN_PLACE_MARKET
        // requirement -- "never registered" is a different state from
        // "registered then frozen".
        let view = view_with_flags(0);
        assert!(!view.is_frozen());
        assert!(!view.can_deposit());
    }

    #[test]
    fn can_deposit_true_whenever_the_deposit_bit_is_set_regardless_of_other_bits() {
        assert!(view_with_flags(TRADER_CAPABILITY_CAN_DEPOSIT).can_deposit());
        assert!(view_with_flags(TRADER_CAPABILITY_CAN_DEPOSIT | TRADER_CAPABILITY_CAN_PLACE_MARKET).can_deposit());
    }
}
