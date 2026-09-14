//! Shared wire codec for the optimizer's live bundler-tip broadcast --
//! `optimizer/bundler.RunTipBroadcaster` polls each registered
//! `bundler.Bundler`'s `Tip()`/`Distribution()` and pushes a
//! [`BundlerTipUpdate`] down to whichever bot mode is running via the
//! same `COMMAND_CUSTOM`/`MessageAction::Custom` pipe every per-strategy
//! `CustomMessageInbound` already rides (see `message::KeyValuePair`),
//! keyed by [`COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE`] -- a shared, reserved
//! key flag every brain module's own `CustomMessageInbound::deserialize`
//! recognizes identically (mirrors `message::COMMON_KEY_FLAG_ACCOUNT_USAGE`'s
//! own reserved-namespace convention, just for the opposite direction:
//! that one is bot->optimizer, this one is optimizer->bot). Defined once
//! here and reused by every module's `message.rs` instead of duplicated
//! six times, since the wire shape and parsing are identical regardless
//! of which strategy receives it -- only `Wallet` (not any one brain
//! module) actually consumes the result, via
//! `Wallet::apply_bundler_tip_update`.
use crate::err::CatscopeGuestError;
use solana_sdk::pubkey::Pubkey;

/// Reserved `KeyValuePair` key for a [`BundlerTipUpdate`], inbound
/// (optimizer -> bot) direction. Must match
/// `optimizer/bundler.KeyFlagBundlerTipUpdate` exactly. Deliberately in
/// the same 200+ reserved range `message::COMMON_KEY_FLAG_ACCOUNT_USAGE`
/// established for shared, non-per-strategy keys (every per-strategy
/// scheme in this codebase stays below 100) -- picked 201, not 200,
/// since 200 is already taken by that outbound message; any future
/// shared key in either direction should keep climbing from here.
pub const COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE: u8 = 201;

/// A live update for one bundler's tip state -- see this module's doc
/// comment. Consumed by `Wallet::apply_bundler_tip_update`.
pub struct BundlerTipUpdate {
    /// Matches `optimizer/bundler.BundlerCode` (0=default/none,
    /// 1=astralane, 2=jito) and, downstream, `bundler_config::BUNDLER`'s
    /// own u8 convention.
    pub bundler: u8,
    /// `false` whenever Go's `Bundler.Tip()`/`Distribution()` returned an
    /// error -- `tip_addresses`/`distribution` are meaningless and
    /// callers should treat this bundler as unusable until the next
    /// `up == true` update.
    pub up: bool,
    pub tip_addresses: Vec<Pubkey>,
    /// 25th/50th/75th/95th/99th percentile lamports required to land in
    /// a block, same order as `optimizer/bundler.Bundler::Distribution`'s
    /// `[5]graph.Lamports`.
    pub distribution: [u64; 5],
}

impl BundlerTipUpdate {
    /// Parses the `KeyValuePair` value bytes `optimizer/bundler.
    /// DoBundlerTipUpdate` produces: `[bundler u8][up u8][n_addrs
    /// u16][addr0 32B]...[addrN-1 32B][p25 u64][p50 u64][p75 u64][p95
    /// u64][p99 u64]`, all little-endian.
    pub fn parse(value: &[u8]) -> Result<Self, CatscopeGuestError> {
        if value.len() < 4 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let bundler = value[0];
        let up = value[1] != 0;
        let n_addrs = u16::from_le_bytes(value[2..4].try_into().unwrap()) as usize;
        let addrs_end = 4 + n_addrs * 32;
        if value.len() != addrs_end + 40 {
            return Err(CatscopeGuestError::InsufficientBufferV2(
                value.len(),
                addrs_end + 40,
            ));
        }
        let mut tip_addresses = Vec::with_capacity(n_addrs);
        for chunk in value[4..addrs_end].chunks_exact(32) {
            tip_addresses.push(Pubkey::new_from_array(chunk.try_into().unwrap()));
        }
        let mut distribution = [0u64; 5];
        for (i, chunk) in value[addrs_end..].chunks_exact(8).enumerate() {
            distribution[i] = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        Ok(Self {
            bundler,
            up,
            tip_addresses,
            distribution,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(bundler: u8, up: bool, addrs: &[Pubkey], distribution: [u64; 5]) -> Vec<u8> {
        let mut v = vec![bundler, if up { 1 } else { 0 }];
        v.extend_from_slice(&(addrs.len() as u16).to_le_bytes());
        for a in addrs {
            v.extend_from_slice(a.as_array());
        }
        for d in distribution {
            v.extend_from_slice(&d.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_roundtrips() {
        let addrs = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let dist = [1, 2, 3, 4, 5];
        let body = encode(1, true, &addrs, dist);
        let update = BundlerTipUpdate::parse(&body).expect("should parse");
        assert_eq!(update.bundler, 1);
        assert!(update.up);
        assert_eq!(update.tip_addresses, addrs);
        assert_eq!(update.distribution, dist);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(BundlerTipUpdate::parse(&[0u8; 3]).is_err());
    }

    #[test]
    fn parse_rejects_length_mismatch() {
        let mut body = encode(1, true, &[Pubkey::new_unique()], [0; 5]);
        body.pop();
        assert!(BundlerTipUpdate::parse(&body).is_err());
    }

    #[test]
    fn parse_down_update_with_no_addresses() {
        let body = encode(2, false, &[], [0; 5]);
        let update = BundlerTipUpdate::parse(&body).expect("should parse");
        assert!(!update.up);
        assert!(update.tip_addresses.is_empty());
    }
}
