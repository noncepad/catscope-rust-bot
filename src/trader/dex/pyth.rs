//! Pyth account parsers (legacy `Price` and Push Oracle `PriceUpdateV2`)
//! -- originally added to parse whatever oracle format a marginfi Bank's
//! `oracle_key` happened to reference (see `dex::marginfi`), and now also
//! used directly: [`PythFeedState`] subscribes straight to Pyth's own
//! fixed, well-known SOL/USD Price account, independent of any lending
//! protocol's bank config (the tracked `marginfi_bank` set turned out to
//! have no SOL bank at all -- see
//! `~/compressed-rolling-wirth.md`'s Pyth Push Oracle plan section for
//! how that was discovered).
//!
//! `parse_legacy`/`parse_push_oracle` are pure functions (just parse
//! bytes you already have); [`PythFeedState`] is the one place in this
//! module that owns a live subscription.
//!
//! # Account layout — Pyth legacy Price account (VERSION_2)
//!
//! Byte offsets verified directly against the authoritative
//! `pyth-network/pyth-client-py` source (`pythaccounts.py`'s
//! `PythPriceAccount.update_from`/`PythPriceInfo.deserialise`), not
//! guessed -- an earlier version of this parser had `agg.conf` as a
//! `u32` and `agg.status` at offset 220, which was silently reading the
//! (usually-zero) high 4 bytes of `agg.conf`'s real `u64` value as
//! "status". That bug happened to leave `price`/`confidence_usd` numerically
//! correct for accounts with a small confidence value (fits in the low 32
//! bits), but made `status`/`pub_slot`-based staleness checks impossible --
//! see the incident below for why that mattered.
//!
//! ```text
//! offset   size  field
//! ──────   ────  ────────────────────────────────────────────────────
//!   0        4   magic (u32)      0xa1b2c3d4 -- Pyth's well-known magic number
//!   4        4   ver (u32)        2 (VERSION_2) for every account checked so far
//!   8        4   atype (u32)
//!  12        4   size (u32)
//!  16        4   ptype (u32)      1 = Price
//!  20        4   expo (i32)
//! 208        8   agg.price (i64)
//! 216        8   agg.conf (u64)   -- NOT u32, see above
//! 224        4   agg.status (u32) 0=Unknown 1=Trading 2=Halted 3=Auction 4=Ignored
//! 232        8   agg.pub_slot (u64) -- the slot this price was last published at
//! ```
//!
//! Confirmed against two real mainnet oracle accounts: (1) an account
//! referenced by two `oracle_setup == 1` marginfi Banks --
//! magic/ver/atype/size/ptype decoded exactly as expected (`size` even
//! matched the account's own real byte length, 3312), and
//! `agg.price * 10^expo` came out to exactly `$1.00`, consistent with the
//! banks using it being stablecoin (6-decimal) markets; (2) the
//! widely-cited "SOL/USD" account `H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG`
//! -- **this one turned out to be stale by roughly 728 days**
//! (`agg.status == 0`/Unknown, `agg.pub_slot` ~140M slots behind the
//! current slot when checked), i.e. Pyth's crank has stopped updating
//! this specific legacy-format account (they've migrated most feeds to
//! the push-oracle model -- see `parse_push_oracle` below), so its frozen
//! `agg.price` (~$119) was badly wrong versus the real ~$75 market price
//! at the time. **A caller MUST check `status == Trading` (and ideally
//! `pub_slot` recency) before trusting `price_usd` for anything real --
//! a well-known/fixed address alone is not evidence of liveness.** Both
//! accounts' owner, `FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH`, is
//! Pyth's mainnet oracle program, confirming these are real Pyth accounts
//! and not a coincidental byte pattern.

use crate::{
    graph::{AccountId, SubscriptionRequest},
    util::account_id_from_pubkey,
};
use solana_sdk::pubkey::Pubkey;

/// Pyth's well-known magic number, present at the start of every legacy
/// Price/Product/Mapping account.
const PYTH_MAGIC: u32 = 0xa1b2c3d4;

const OFF_EXPO: usize = 20;
const OFF_AGG_PRICE: usize = 208;
const OFF_AGG_CONF: usize = 216;
const OFF_AGG_STATUS: usize = 224;
const OFF_AGG_PUB_SLOT: usize = 232;

const MIN_LEN: usize = OFF_AGG_PUB_SLOT + 8;

/// Pyth's `PriceStatus` enum ordinal for "actively trading" -- the only
/// status under which `price_usd` should be trusted. See the incident in
/// the module doc comment for why this matters: a well-known account
/// address is not evidence a price is current.
pub const PYTH_STATUS_TRADING: u32 = 1;

/// A parsed Pyth legacy aggregate price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PythPrice {
    pub price_usd: f64,
    pub confidence_usd: f64,
    /// Raw `agg.status` enum ordinal: 0=Unknown 1=Trading 2=Halted
    /// 3=Auction 4=Ignored. Compare against [`PYTH_STATUS_TRADING`]
    /// before trusting `price_usd`.
    pub status: u32,
    /// The slot this price was last published at -- compare against the
    /// current slot to detect a crank that has stopped updating this
    /// account entirely (an abandoned legacy account can sit at
    /// `status == Trading`... in practice the one incident found so far
    /// showed `status == Unknown`, but `pub_slot` is the more direct
    /// staleness signal and doesn't rely on trusting the status field).
    pub pub_slot: u64,
}

/// Parse a Pyth legacy Price account. Returns `None` if the account
/// doesn't start with Pyth's magic number (wrong account, or not a Pyth
/// account at all) or is too short.
pub fn parse_legacy(body: &[u8]) -> Option<PythPrice> {
    if body.len() < MIN_LEN {
        return None;
    }
    let magic = u32::from_le_bytes(body[0..4].try_into().unwrap());
    if magic != PYTH_MAGIC {
        return None;
    }

    let expo = i32::from_le_bytes(body[OFF_EXPO..OFF_EXPO + 4].try_into().unwrap());
    let agg_price = i64::from_le_bytes(body[OFF_AGG_PRICE..OFF_AGG_PRICE + 8].try_into().unwrap());
    let agg_conf = u64::from_le_bytes(body[OFF_AGG_CONF..OFF_AGG_CONF + 8].try_into().unwrap());
    let agg_status =
        u32::from_le_bytes(body[OFF_AGG_STATUS..OFF_AGG_STATUS + 4].try_into().unwrap());
    let agg_pub_slot =
        u64::from_le_bytes(body[OFF_AGG_PUB_SLOT..OFF_AGG_PUB_SLOT + 8].try_into().unwrap());

    let scale = 10f64.powi(expo);
    Some(PythPrice {
        price_usd: agg_price as f64 * scale,
        confidence_usd: agg_conf as f64 * scale,
        status: agg_status,
        pub_slot: agg_pub_slot,
    })
}

/// Pyth Push Oracle (`pyth-solana-receiver`) `PriceUpdateV2` account
/// parser -- the newer oracle format some marginfi banks reference
/// instead of the legacy `Price` account above (confirmed live this
/// session: marginfi's real SOL banks use `oracle_setup == 4`, not `1`
/// (`PythLegacy`), and `dex::pyth`'s own module doc comment already
/// flagged this as separate, not-yet-done work).
///
/// # Account layout — `PriceUpdateV2` (Anchor)
///
/// Fields verified directly against the authoritative source
/// (`pyth-network/pyth-crosschain`, `target_chains/solana/
/// pyth_solana_receiver_sdk/src/price_update.rs` for `PriceUpdateV2` and
/// `pythnet/pythnet_sdk/src/messages.rs` for `PriceFeedMessage`'s field
/// order -- **not independently verified against real captured mainnet
/// account bytes** the way the legacy parser above was (no live RPC
/// access in this session to do that cross-check) -- treat this as
/// unverified until confirmed against a real account, same as any other
/// not-yet-checked offset in this codebase.
///
/// ```text
/// offset          size  field
/// ──────          ────  ────────────────────────────────────────────────
///   0               8   Anchor discriminator (sha256("account:PriceUpdateV2")[..8])
///   8              32   write_authority (Pubkey)
///  40               1   verification_level discriminant (0 = Partial, 1 = Full)
///  41    0 or 1 (Partial only)   num_signatures (u8) -- only present for Partial
/// price_message starts at 41 (Full) or 42 (Partial):
///  +0              32   feed_id
///  +32              8   price (i64)
///  +40              8   conf (u64)
///  +48              4   exponent (i32)
///  +52              8   publish_time (i64)
///  +60              8   prev_publish_time (i64)
///  +68              8   ema_price (i64)
///  +76              8   ema_conf (u64)
/// price_message ends at +84, followed by:
///                   8   posted_slot (u64)
/// ```
///
/// `VerificationLevel` is a Borsh enum (`Partial { num_signatures: u8 }`
/// declared first = discriminant 0, `Full` declared second = discriminant
/// 1) -- **not** a fixed-size field, so `price_message`'s real offset
/// depends on which variant is actually present; `PriceUpdateV2::LEN`'s
/// own published constant (134 bytes) reflects the *larger* (`Partial`)
/// variant's allocated space, not a fixed offset for every account.
const PYTH_PUSH_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];

const OFF_VERIFICATION_LEVEL: usize = 40;
const VERIFICATION_LEVEL_PARTIAL: u8 = 0;
const VERIFICATION_LEVEL_FULL: u8 = 1;

/// A parsed Pyth Push Oracle (`PriceUpdateV2`) price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PythPushPrice {
    pub price_usd: f64,
    pub confidence_usd: f64,
    /// The feed this account claims to be pricing -- callers that need
    /// to be sure they're reading the *right* feed (this parser doesn't
    /// know what feed_id to expect) should check this against a known
    /// value, same as Pyth's own SDK does in its `get_price_no_older_than`
    /// helpers.
    pub feed_id: [u8; 32],
    pub publish_time: i64,
}

/// Parse a Pyth Push Oracle `PriceUpdateV2` account. Returns `None` if
/// the account doesn't start with `PriceUpdateV2`'s Anchor discriminator
/// (wrong account, or not this format at all), has an unrecognized
/// `verification_level` discriminant, or is too short for whichever
/// variant it claims to be.
pub fn parse_push_oracle(body: &[u8]) -> Option<PythPushPrice> {
    if body.len() < 8 + 32 + 1 {
        return None;
    }
    if body[0..8] != PYTH_PUSH_DISCRIMINATOR {
        return None;
    }

    let verification_discriminant = body[OFF_VERIFICATION_LEVEL];
    let price_message_off = match verification_discriminant {
        VERIFICATION_LEVEL_FULL => OFF_VERIFICATION_LEVEL + 1,
        VERIFICATION_LEVEL_PARTIAL => OFF_VERIFICATION_LEVEL + 2,
        _ => return None,
    };

    if body.len() < price_message_off + 84 + 8 {
        return None;
    }

    let read_i64 = |off: usize| i64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());

    let feed_id: [u8; 32] = body[price_message_off..price_message_off + 32].try_into().unwrap();
    let price = read_i64(price_message_off + 32);
    let conf = read_u64(price_message_off + 40);
    let exponent = i32::from_le_bytes(
        body[price_message_off + 48..price_message_off + 52].try_into().unwrap(),
    );
    let publish_time = read_i64(price_message_off + 52);

    let scale = 10f64.powi(exponent);
    Some(PythPushPrice {
        price_usd: price as f64 * scale,
        confidence_usd: conf as f64 * scale,
        feed_id,
        publish_time,
    })
}

/// Switchboard On-Demand `PullFeedAccountData` parser -- the other "plain"
/// oracle format a marginfi Bank's `oracle_key` can reference
/// (`OracleSetup::SwitchboardPull`, confirmed live this session: marginfi's
/// real SOL bank on the main group uses this format, not Pyth). Owner
/// program `SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv` (Switchboard's
/// on-demand program).
///
/// # Account layout — `PullFeedAccountData` (Anchor)
///
/// Only the current price is parsed -- the rest of the account (oracle
/// submissions, historical results, feed metadata) isn't needed by any
/// caller here.
///
/// ```text
/// offset   size  field
/// ──────   ────  ────────────────────────────────────────────────────
///   0        8   Anchor discriminator (sha256("account:PullFeedAccountData")[..8])
///  56       16   result.value (i128, fixed-point, scale 10^18)
/// ```
///
/// **Live-verified, not derived from public source** (no on-chain Rust
/// program source was found for Switchboard's on-demand program -- only a
/// TypeScript client, which decodes via Anchor's IDL rather than exposing
/// byte offsets directly): confirmed against a real mainnet account
/// (`4Hmd6PdjVA9auCoScE12iaBogfwS4ZXQ6VZoBeqanwWW`, the oracle referenced
/// by marginfi's real SOL bank) two independent ways -- (1) its first 8
/// bytes exactly equal `sha256("account:PullFeedAccountData")[..8]`,
/// confirming the account type; (2) the i128 at offset 56, scaled by
/// `10^18`, reads back as `$75.778`, matching real-time SOL/USD
/// (`$75.87` per a public price API at the same time) far more closely
/// than any other candidate offset found by scanning the account for
/// plausible SOL-price-range values.
const SWITCHBOARD_PULL_DISCRIMINATOR: [u8; 8] = [196, 27, 108, 196, 10, 215, 219, 40];

const OFF_SWITCHBOARD_VALUE: usize = 56;
const SWITCHBOARD_VALUE_SCALE: f64 = 1e18;

/// `PullFeedAccountData::last_update_timestamp` (i64, unix seconds) --
/// this is the exact field marginfi's own `SwitchboardPullPriceFeed::
/// load_checked` compares against `Clock::unix_timestamp` for its
/// `SwitchboardStalePrice` check (`current_timestamp.saturating_sub(
/// last_updated) > max_age`, confirmed via `0dotxyz/marginfi-v2`'s real
/// `programs/marginfi/src/state/price.rs`). Live-verified this session:
/// the real TS SDK's `PullFeedAccountData` field order
/// (`switchboard-xyz/on-demand`'s `src/accounts/pullFeed.ts`) gives no
/// byte offsets directly (Anchor/Borsh-decoded on that side, not
/// fixed-offset), so this was found empirically instead -- scanned the
/// whole account for an i64 within an hour of the real current unix time
/// and found exactly one real candidate (plus a duplicate a bit further
/// into the account, part of the historical-results ring buffer), which
/// cross-validated against an independent estimate derived from a
/// separate slot-like field found the same way and resolved via
/// `getBlockTime` -- both agreed the feed was ~40-48 minutes stale at
/// the time, not a coincidence.
const OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP: usize = 2216;

/// A parsed Switchboard On-Demand pull-feed price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchboardPullPrice {
    pub price_usd: f64,
    /// See [`OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP`]'s doc comment.
    pub last_update_timestamp: i64,
}

/// Parse a Switchboard On-Demand `PullFeedAccountData` account. Returns
/// `None` if the account doesn't start with this format's Anchor
/// discriminator (wrong account, or not this format at all) or is too
/// short.
pub fn parse_switchboard_pull(body: &[u8]) -> Option<SwitchboardPullPrice> {
    if body.len() < OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP + 8 {
        return None;
    }
    if body[0..8] != SWITCHBOARD_PULL_DISCRIMINATOR {
        return None;
    }
    let raw = i128::from_le_bytes(body[OFF_SWITCHBOARD_VALUE..OFF_SWITCHBOARD_VALUE + 16].try_into().unwrap());
    let last_update_timestamp = i64::from_le_bytes(
        body[OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP..OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP + 8]
            .try_into()
            .unwrap(),
    );
    Some(SwitchboardPullPrice {
        price_usd: raw as f64 / SWITCHBOARD_VALUE_SCALE,
        last_update_timestamp,
    })
}

/// Unified price, regardless of which on-chain Pyth account format
/// (legacy `Price` vs. push-oracle `PriceUpdateV2`) actually produced it
/// -- callers that only need `price_usd`/`confidence_usd` (e.g. the
/// `arbv1` diagnostic comparing against the router's own AMM-derived
/// price) shouldn't need to care which format a given bank's oracle
/// happens to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OraclePrice {
    pub price_usd: f64,
    pub confidence_usd: f64,
    /// Unix seconds this price was last updated on-chain, if the source
    /// format records one this way (only `SwitchboardPullPrice` does --
    /// legacy Pyth/Pyth Push use their own separate slot/timestamp
    /// staleness signals, `pub_slot`/`publish_time`, not routed through
    /// here). `None` when not applicable. See
    /// [`SwitchboardPullPrice::last_update_timestamp`]'s doc comment for
    /// why this matters: a genuinely stale Switchboard on-demand feed
    /// (live-confirmed this session: ~40+ minutes since its last real
    /// crank) will still deliver a value here, and callers that skip this
    /// check will send a doomed transaction straight into whatever
    /// on-chain staleness check the consuming protocol enforces.
    pub last_update_timestamp: Option<i64>,
}

impl From<PythPrice> for OraclePrice {
    fn from(p: PythPrice) -> Self {
        Self {
            price_usd: p.price_usd,
            confidence_usd: p.confidence_usd,
            last_update_timestamp: None,
        }
    }
}

impl From<PythPushPrice> for OraclePrice {
    fn from(p: PythPushPrice) -> Self {
        Self {
            price_usd: p.price_usd,
            confidence_usd: p.confidence_usd,
            last_update_timestamp: None,
        }
    }
}

impl From<SwitchboardPullPrice> for OraclePrice {
    fn from(p: SwitchboardPullPrice) -> Self {
        // `result.stdDev` (the feed's own confidence interval) isn't
        // parsed -- only the current price value was needed by any caller
        // so far -- so this is left at 0.0 rather than a fabricated number.
        Self {
            price_usd: p.price_usd,
            confidence_usd: 0.0,
            last_update_timestamp: Some(p.last_update_timestamp),
        }
    }
}

/// Pyth's Push Oracle program (`DEFAULT_PUSH_ORACLE_PROGRAM_ID` in Pyth's
/// own `pyth_solana_receiver` SDK, `address.ts`) -- the program a
/// `PriceUpdateV2` "price feed account" PDA is derived against. Distinct
/// from the Receiver program (`rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`),
/// which owns the account's actual bytes but isn't part of the PDA seeds.
const PYTH_PUSH_ORACLE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT");

/// Pyth's SOL/USD price feed ID, looked up live from Pyth's own Hermes
/// API this session (`hermes.pyth.network/v2/price_feeds?query=SOL/USD`),
/// not guessed or copied from a search-engine summary (an earlier attempt
/// at copying it that way silently gained an extra trailing hex digit).
const SOL_USD_FEED_ID: [u8; 32] = [
    0xef, 0x0d, 0x8b, 0x6f, 0xda, 0x2c, 0xeb, 0xa4, 0x1d, 0xa1, 0x5d, 0x40, 0x95, 0xd1, 0xda, 0x39,
    0x2a, 0x0d, 0x2f, 0x8e, 0xd0, 0xc6, 0xc7, 0xbc, 0x0f, 0x4c, 0xfa, 0xc8, 0xc2, 0x80, 0xb5, 0x6d,
];

/// Direct subscription to Pyth's own canonical SOL/USD push-oracle price
/// feed account -- the PDA `PublicKey::find_program_address([shard_id_le,
/// feed_id], PYTH_PUSH_ORACLE_PROGRAM_ID)` with `shard_id = 0` (Pyth's own
/// crank keeps shard 0 continuously updated for popular feeds; other
/// shards are ephemeral, created ad hoc by SDK callers -- see
/// `getPriceFeedAccountForProgram` in `pyth_solana_receiver`'s
/// `address.ts`). Verified live this session: this exact derivation
/// resolves to `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE`, which on a
/// direct `getAccountInfo` RPC call was `space=134` (matches
/// `PriceUpdateV2::LEN`), owned by the Receiver program, decoded to the
/// right discriminator/feed_id, and had `publish_time` only ~31 seconds
/// behind wall-clock `now` -- genuinely live, unlike the legacy Price
/// account originally tried here (see the module doc comment's incident
/// writeup: that "well-known" address turned out to be abandoned by
/// Pyth's crank, frozen ~728 days stale).
///
/// Unlike going through `MarginfiState::price_for_mint`, this doesn't
/// depend on marginfi tracking a SOL bank at all (as of this writing, the
/// tracked `marginfi_bank` set has none -- see
/// `~/compressed-rolling-wirth.md`'s Pyth Push Oracle plan section). An
/// independent ground truth by construction, for validating the
/// router's own AMM-derived pricing against (see `arbv1::state.rs`'s
/// `trade router check` diagnostic).
pub struct PythFeedState {
    sol_usd_account_id: AccountId,
    sol_usd_price: Option<PythPushPrice>,
}

impl std::fmt::Debug for PythFeedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PythFeedState")
            .field("has_sol_usd_price", &self.sol_usd_price.is_some())
            .finish()
    }
}

impl PythFeedState {
    /// Builds this dex's live state and returns its pending subscription
    /// request alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, SubscriptionRequest) {
        let shard_id_le = 0u16.to_le_bytes();
        let (pda, _bump) = Pubkey::find_program_address(
            &[&shard_id_le, &SOL_USD_FEED_ID],
            &PYTH_PUSH_ORACLE_PROGRAM_ID,
        );
        let sol_usd_account_id = account_id_from_pubkey(&pda);
        let req = SubscriptionRequest { root: sol_usd_account_id, filter_weight: 0, depth: 1 };
        (Self { sol_usd_account_id, sol_usd_price: None }, req)
    }

    pub fn sol_usd_price(&self) -> Option<PythPushPrice> {
        self.sol_usd_price
    }

    pub fn on_account(&mut self, account_id: AccountId, body: &[u8]) {
        if account_id == self.sol_usd_account_id {
            if let Some(price) = parse_push_oracle(body) {
                self.sol_usd_price = Some(price);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Price/confidence values captured from a live mainnet Pyth Price
    /// account (`Gnt27xtC473ZT2Mw5u8wZ68Z3gULkSTb5DuxJy7eJotD`, a USDC
    /// oracle) during offset verification -- `status`/`pub_slot` are
    /// synthetic (a plausible "Trading" reading), not from that capture:
    /// they were added after fixing the `agg.conf`/`agg.status` offset
    /// bug (see the module doc comment's incident writeup), and this
    /// fixture predates re-checking those two fields' real values at the
    /// corrected offsets.
    fn real_account_bytes() -> Vec<u8> {
        let mut body = vec![0u8; MIN_LEN];
        body[0..4].copy_from_slice(&PYTH_MAGIC.to_le_bytes());
        body[4..8].copy_from_slice(&2u32.to_le_bytes()); // ver
        body[8..12].copy_from_slice(&3u32.to_le_bytes()); // atype
        body[12..16].copy_from_slice(&3312u32.to_le_bytes()); // size
        body[16..20].copy_from_slice(&1u32.to_le_bytes()); // ptype
        body[OFF_EXPO..OFF_EXPO + 4].copy_from_slice(&(-8i32).to_le_bytes());
        body[OFF_AGG_PRICE..OFF_AGG_PRICE + 8].copy_from_slice(&100_000_000i64.to_le_bytes());
        body[OFF_AGG_CONF..OFF_AGG_CONF + 8].copy_from_slice(&106_896u64.to_le_bytes());
        body[OFF_AGG_STATUS..OFF_AGG_STATUS + 4].copy_from_slice(&PYTH_STATUS_TRADING.to_le_bytes());
        body[OFF_AGG_PUB_SLOT..OFF_AGG_PUB_SLOT + 8].copy_from_slice(&200_000_000u64.to_le_bytes());
        body
    }

    #[test]
    fn parses_real_account_price() {
        let price = parse_legacy(&real_account_bytes()).expect("should parse");
        assert!((price.price_usd - 1.0).abs() < 1e-9);
        assert!((price.confidence_usd - 0.00106896).abs() < 1e-9);
        assert_eq!(price.status, PYTH_STATUS_TRADING);
        assert_eq!(price.pub_slot, 200_000_000);
    }

    #[test]
    fn confidence_wider_than_u32_is_not_truncated() {
        // Regression test for the offset bug: agg.conf is a u64, not a
        // u32 -- a confidence value that doesn't fit in 32 bits must
        // still round-trip correctly (the old, wrong offset/width would
        // have silently truncated this).
        let mut body = real_account_bytes();
        let big_conf: u64 = 5_000_000_000; // > u32::MAX
        body[OFF_AGG_CONF..OFF_AGG_CONF + 8].copy_from_slice(&big_conf.to_le_bytes());
        let price = parse_legacy(&body).expect("should parse");
        assert!((price.confidence_usd - 50.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut body = real_account_bytes();
        body[0] = 0; // corrupt the magic number
        assert!(parse_legacy(&body).is_none());
    }

    #[test]
    fn rejects_too_short() {
        assert!(parse_legacy(&[0u8; 10]).is_none());
    }

    /// Builds a synthetic PriceUpdateV2 account (either verification
    /// level variant) with a known price/conf/exponent, so the offset
    /// arithmetic can be checked without live account bytes.
    fn synthetic_push_oracle_bytes(full: bool, price: i64, conf: u64, exponent: i32) -> Vec<u8> {
        let mut body = PYTH_PUSH_DISCRIMINATOR.to_vec(); // 0..8
        body.extend_from_slice(&[0u8; 32]); // 8..40 write_authority (unused)
        if full {
            body.push(VERIFICATION_LEVEL_FULL); // 40
        } else {
            body.push(VERIFICATION_LEVEL_PARTIAL); // 40
            body.push(7); // 41: num_signatures (Partial-only extra byte)
        }
        let feed_id = [9u8; 32];
        body.extend_from_slice(&feed_id); // feed_id
        body.extend_from_slice(&price.to_le_bytes()); // price
        body.extend_from_slice(&conf.to_le_bytes()); // conf
        body.extend_from_slice(&exponent.to_le_bytes()); // exponent
        body.extend_from_slice(&1_700_000_000i64.to_le_bytes()); // publish_time
        body.extend_from_slice(&1_699_999_999i64.to_le_bytes()); // prev_publish_time
        body.extend_from_slice(&price.to_le_bytes()); // ema_price
        body.extend_from_slice(&conf.to_le_bytes()); // ema_conf
        body.extend_from_slice(&123_456_789u64.to_le_bytes()); // posted_slot
        body
    }

    #[test]
    fn parses_push_oracle_full_verification() {
        // price=15000000000, exponent=-8 -> $150.00, matching Pyth's
        // real SOL/USD exponent convention (-8).
        let body = synthetic_push_oracle_bytes(true, 15_000_000_000, 5_000_000, -8);
        let price = parse_push_oracle(&body).expect("should parse (Full variant)");
        assert!((price.price_usd - 150.0).abs() < 1e-9);
        assert!((price.confidence_usd - 0.05).abs() < 1e-9);
        assert_eq!(price.feed_id, [9u8; 32]);
        assert_eq!(price.publish_time, 1_700_000_000);
    }

    #[test]
    fn parses_push_oracle_partial_verification() {
        // Same values, but the Partial variant's extra num_signatures
        // byte shifts price_message's real offset by one -- this is the
        // exact case that would silently misparse everything after it if
        // the offset were treated as fixed instead of branching on the
        // verification_level discriminant.
        let body = synthetic_push_oracle_bytes(false, 15_000_000_000, 5_000_000, -8);
        let price = parse_push_oracle(&body).expect("should parse (Partial variant)");
        assert!((price.price_usd - 150.0).abs() < 1e-9);
        assert!((price.confidence_usd - 0.05).abs() < 1e-9);
    }

    #[test]
    fn push_oracle_rejects_wrong_discriminator() {
        let mut body = synthetic_push_oracle_bytes(true, 1, 1, 0);
        body[0] = 0; // corrupt the Anchor discriminator
        assert!(parse_push_oracle(&body).is_none());
    }

    #[test]
    fn push_oracle_rejects_unknown_verification_level() {
        let mut body = synthetic_push_oracle_bytes(true, 1, 1, 0);
        body[OFF_VERIFICATION_LEVEL] = 2; // neither Partial (0) nor Full (1)
        assert!(parse_push_oracle(&body).is_none());
    }

    #[test]
    fn push_oracle_rejects_too_short() {
        assert!(parse_push_oracle(&[0u8; 10]).is_none());
        // Right discriminator, but truncated before price_message ends.
        let mut body = synthetic_push_oracle_bytes(true, 1, 1, 0);
        body.truncate(body.len() - 1);
        assert!(parse_push_oracle(&body).is_none());
    }

    /// Builds a synthetic `PullFeedAccountData` account with a known
    /// `result.value`, exercising the exact offset/scale live-verified
    /// against SOL's real bank this session.
    fn synthetic_switchboard_bytes(value: i128, last_update_timestamp: i64) -> Vec<u8> {
        let mut body = SWITCHBOARD_PULL_DISCRIMINATOR.to_vec(); // 0..8
        body.resize(OFF_SWITCHBOARD_VALUE, 0);
        body.extend_from_slice(&value.to_le_bytes()); // 56..72
        body.resize(OFF_SWITCHBOARD_LAST_UPDATE_TIMESTAMP, 0);
        body.extend_from_slice(&last_update_timestamp.to_le_bytes()); // 2216..2224
        body
    }

    #[test]
    fn parses_switchboard_pull_price() {
        // 75_778_000_000_000_000_000 / 1e18 == 75.778, matching the live
        // SOL/USD value read from the real oracle account this session.
        let body = synthetic_switchboard_bytes(75_778_000_000_000_000_000, 1_700_000_000);
        let price = parse_switchboard_pull(&body).expect("should parse");
        assert!((price.price_usd - 75.778).abs() < 1e-9);
        assert_eq!(price.last_update_timestamp, 1_700_000_000);
    }

    #[test]
    fn switchboard_pull_rejects_wrong_discriminator() {
        let mut body = synthetic_switchboard_bytes(1_000_000_000_000_000_000, 1_700_000_000);
        body[0] = 0;
        assert!(parse_switchboard_pull(&body).is_none());
    }

    #[test]
    fn switchboard_pull_rejects_too_short() {
        assert!(parse_switchboard_pull(&[0u8; 10]).is_none());
    }
}
