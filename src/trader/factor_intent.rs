//! Restart-safe open-trade intent -- Phase 4 of
//! `brain::multimodelv1::PLAN-1.md`.
//!
//! A single Kamino obligation or a single Phoenix perp position is
//! *fully described* by its own on-chain account -- every other bot mode
//! in this repo re-derives position state from real reads on every
//! restart and needs no local database because of that. A multi-leg
//! factor-neutral or dispersion basket is different: the wallet's
//! post-trade token balances alone don't say *which* basket a balance
//! belongs to, what the entry residual was, or what "neutral" was
//! supposed to mean for this specific trade -- that intent has no
//! on-chain representation. [`OpenIntent`] is the minimal record of that
//! intent; [`reconcile_intent`] is the rule for whether replayed intent
//! (from `prefetch.db`, via the Go host, after a restart -- Phase 4
//! points 2-3, not yet built) should ever be trusted again.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `factor_graph.rs`/`factor_borrow_gate.rs` -- callers resolve real
//! on-chain balances themselves and hand them to [`reconcile_intent`].
//! [`encode`]/[`decode`] are the wire format
//! `brain::multimodelv1::message` (not yet built as a full bot mode,
//! only its `message.rs` -- see that module) uses to actually carry an
//! `OpenIntent` between the Rust guest and the Go host; defined here,
//! not duplicated there, so the byte layout has exactly one owner.

use crate::graph::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegDirection {
    Long,
    Short,
}

/// One leg of a basket trade's intent -- what the strategy expects to
/// find on-chain for this mint if the basket is still genuinely open the
/// way it was left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentLeg {
    pub mint: AccountId,
    pub direction: LegDirection,
    /// Raw-unit notional expected for this leg at entry -- same unit
    /// `reconcile_intent`'s `real_balances` input must use (this module
    /// doesn't do USD conversion; that's the caller's job, same as every
    /// other pure module in this crate).
    pub expected_notional: u64,
}

/// The full record of one open basket trade -- basket membership, entry
/// context, and every leg's expected footprint. Sent outbound once when
/// a basket opens (or closes, at which point the Go host should drop its
/// persisted copy -- not this module's concern, see the module doc
/// comment), and replayed back inbound on a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenIntent {
    /// Caller-assigned identifier for this basket, opaque to this
    /// module -- distinguishes concurrently open baskets from each
    /// other, nothing more.
    pub basket_id: u64,
    pub legs: Vec<IntentLeg>,
    /// The residual (percent) that justified opening this basket, at
    /// entry -- diagnostic/audit value, not re-derived or re-checked by
    /// this module.
    pub entry_residual_pct: f64,
    pub entry_timestamp_secs: i64,
}

/// Why [`reconcile_intent`] refused to trust a specific leg. Structured,
/// not a string -- callers (logging, a future `TriggerCloseAllFactor
/// Positions`-style safety valve) need to branch on *which* problem this
/// is, not just that there was one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UntrustedReason {
    /// `real_balances` had no entry at all for this leg's mint -- the
    /// wallet doesn't even hold the asset the intent record claims it
    /// should.
    MissingLeg { mint: AccountId },
    /// `real_balances` had an entry, but it's outside
    /// [`RECONCILIATION_TOLERANCE_PCT`] of what the intent expected.
    NotionalMismatch { mint: AccountId, expected: u64, real: u64 },
}

/// Result of reconciling a replayed [`OpenIntent`] against real on-chain
/// balances. See [`reconcile_intent`]'s doc comment for what a caller is
/// expected to do with each variant.
#[derive(Debug, Clone, PartialEq)]
pub enum IntentTrust {
    Trusted,
    /// Every leg that failed reconciliation, not just the first -- a
    /// caller logging or surfacing this wants the full picture in one
    /// shot, not one problem per retry.
    Untrusted(Vec<UntrustedReason>),
}

/// Max tolerated deviation (percent) between an intent leg's expected
/// notional and the real on-chain balance found for its mint. `1.0` is a
/// starting value, not a calibrated one -- no real basket trade has ever
/// gone through a restart yet to measure real drift (partial fills,
/// price movement between entry and restart, etc.) against.
pub const RECONCILIATION_TOLERANCE_PCT: f64 = 1.0;

/// The real gate Phase 4 exists for: does `intent`, replayed from
/// persisted storage after a restart, still match reality closely enough
/// to resume autonomous management of it? `real_balances` is the
/// caller's own resolved real on-chain reads -- `(mint, raw_balance)`
/// pairs, same units as [`IntentLeg::expected_notional`].
///
/// **Caller contract** (this module can't enforce it, only document it,
/// same as every other pure module in this crate): [`IntentTrust::Trusted`]
/// means it's safe to resume normal management of this basket (close-pass
/// re-evaluation, further borrow-cost/staleness gating, etc.).
/// [`IntentTrust::Untrusted`] means the opposite of "retry" -- per
/// `PLAN-1.md` Phase 4, an untrusted basket must not be silently acted on
/// at all; it should be surfaced and require an explicit manual
/// safety-valve trigger, the same way `TriggerCloseAllBasisPositions`
/// exists for `leveragedloopv1`'s basis trade.
pub fn reconcile_intent(intent: &OpenIntent, real_balances: &[(AccountId, u64)]) -> IntentTrust {
    let mut reasons = Vec::new();
    for leg in &intent.legs {
        match real_balances.iter().find(|(mint, _)| *mint == leg.mint) {
            None => reasons.push(UntrustedReason::MissingLeg { mint: leg.mint }),
            Some(&(_, real)) => {
                let expected = leg.expected_notional;
                let diff_pct = if expected == 0 {
                    if real == 0 { 0.0 } else { f64::INFINITY }
                } else {
                    ((real as f64 - expected as f64).abs() / expected as f64) * 100.0
                };
                if diff_pct > RECONCILIATION_TOLERANCE_PCT {
                    reasons.push(UntrustedReason::NotionalMismatch { mint: leg.mint, expected, real });
                }
            }
        }
    }
    if reasons.is_empty() {
        IntentTrust::Trusted
    } else {
        IntentTrust::Untrusted(reasons)
    }
}

const LEG_WIRE_SIZE: usize = 8 + 1 + 8; // mint (u64) + direction (u8) + expected_notional (u64)
const HEADER_WIRE_SIZE: usize = 8 + 8 + 8 + 2; // basket_id + entry_residual_pct + entry_timestamp_secs + n_legs

/// Real byte layout for an `OpenIntent`, little-endian throughout,
/// matching this crate's existing hand-rolled wire framing convention
/// (`message.rs`'s `KeyValuePair`/`MessageSend::CommonAddressUpdate` --
/// no `bincode`/`wincode`, both already crate dependencies but reserved
/// for transaction encoding, not this message pipe):
/// `[basket_id u64][entry_residual_pct f64][entry_timestamp_secs i64]
/// [n_legs u16][(mint u64, direction u8, expected_notional u64) x n_legs]`.
/// Defined once here; `brain::multimodelv1::message` wraps this `value`
/// inside the same `KeyValuePair` framing every other per-strategy
/// `Custom` message already uses, it does not re-encode the intent
/// itself.
pub fn encode(intent: &OpenIntent) -> Vec<u8> {
    let mut out = vec![0u8; HEADER_WIRE_SIZE + intent.legs.len() * LEG_WIRE_SIZE];
    out[0..8].copy_from_slice(&intent.basket_id.to_le_bytes());
    out[8..16].copy_from_slice(&intent.entry_residual_pct.to_le_bytes());
    out[16..24].copy_from_slice(&intent.entry_timestamp_secs.to_le_bytes());
    out[24..26].copy_from_slice(&(intent.legs.len() as u16).to_le_bytes());
    for (i, leg) in intent.legs.iter().enumerate() {
        let off = HEADER_WIRE_SIZE + i * LEG_WIRE_SIZE;
        out[off..(off + 8)].copy_from_slice(&leg.mint.to_le_bytes());
        out[off + 8] = match leg.direction {
            LegDirection::Long => 0,
            LegDirection::Short => 1,
        };
        out[(off + 9)..(off + 17)].copy_from_slice(&leg.expected_notional.to_le_bytes());
    }
    out
}

/// Inverse of [`encode`]. `None` on any malformed/truncated input
/// (including an unrecognized direction byte) -- never partially
/// decodes; a caller receiving replayed intent it can't fully parse must
/// treat that the same as [`IntentTrust::Untrusted`], not act on a
/// partial record.
pub fn decode(bytes: &[u8]) -> Option<OpenIntent> {
    if bytes.len() < HEADER_WIRE_SIZE {
        return None;
    }
    let basket_id = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let entry_residual_pct = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let entry_timestamp_secs = i64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let n_legs = u16::from_le_bytes(bytes[24..26].try_into().ok()?) as usize;
    let expected_len = HEADER_WIRE_SIZE + n_legs * LEG_WIRE_SIZE;
    if bytes.len() != expected_len {
        return None;
    }
    let mut legs = Vec::with_capacity(n_legs);
    for i in 0..n_legs {
        let off = HEADER_WIRE_SIZE + i * LEG_WIRE_SIZE;
        let mint = u64::from_le_bytes(bytes[off..(off + 8)].try_into().ok()?);
        let direction = match bytes[off + 8] {
            0 => LegDirection::Long,
            1 => LegDirection::Short,
            _ => return None,
        };
        let expected_notional = u64::from_le_bytes(bytes[(off + 9)..(off + 17)].try_into().ok()?);
        legs.push(IntentLeg { mint, direction, expected_notional });
    }
    Some(OpenIntent { basket_id, legs, entry_residual_pct, entry_timestamp_secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_intent() -> OpenIntent {
        OpenIntent {
            basket_id: 42,
            legs: vec![
                IntentLeg { mint: 1, direction: LegDirection::Long, expected_notional: 1_000_000 },
                IntentLeg { mint: 2, direction: LegDirection::Short, expected_notional: 500_000 },
            ],
            entry_residual_pct: 2.35,
            entry_timestamp_secs: 1_735_000_000,
        }
    }

    // --- encode/decode -----------------------------------------------

    #[test]
    fn encode_decode_roundtrips() {
        let intent = sample_intent();
        let bytes = encode(&intent);
        let decoded = decode(&bytes).expect("should decode");
        assert_eq!(decoded, intent);
    }

    #[test]
    fn encode_decode_roundtrips_zero_legs() {
        let intent = OpenIntent { basket_id: 7, legs: vec![], entry_residual_pct: 0.0, entry_timestamp_secs: 0 };
        let bytes = encode(&intent);
        assert_eq!(bytes.len(), HEADER_WIRE_SIZE);
        let decoded = decode(&bytes).expect("should decode");
        assert_eq!(decoded, intent);
    }

    #[test]
    fn decode_rejects_truncated_header() {
        let bytes = vec![0u8; HEADER_WIRE_SIZE - 1];
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn decode_rejects_length_mismatched_legs() {
        let intent = sample_intent();
        let mut bytes = encode(&intent);
        bytes.pop(); // one byte short of the last leg
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn decode_rejects_unknown_direction_byte() {
        let intent = sample_intent();
        let mut bytes = encode(&intent);
        bytes[HEADER_WIRE_SIZE + 8] = 0xFF; // first leg's direction byte
        assert_eq!(decode(&bytes), None);
    }

    // --- reconcile_intent ----------------------------------------------

    #[test]
    fn reconcile_trusted_on_exact_match() {
        let intent = sample_intent();
        let balances = [(1, 1_000_000), (2, 500_000)];
        assert_eq!(reconcile_intent(&intent, &balances), IntentTrust::Trusted);
    }

    #[test]
    fn reconcile_trusted_within_tolerance() {
        let intent = sample_intent();
        // 1_000_000 leg, real balance 0.5% off -- inside the 1% band.
        let balances = [(1, 1_005_000), (2, 500_000)];
        assert_eq!(reconcile_intent(&intent, &balances), IntentTrust::Trusted);
    }

    #[test]
    fn reconcile_untrusted_beyond_tolerance() {
        let intent = sample_intent();
        // 1_000_000 leg, real balance 5% off -- well beyond the 1% band.
        let balances = [(1, 1_050_000), (2, 500_000)];
        let trust = reconcile_intent(&intent, &balances);
        assert_eq!(
            trust,
            IntentTrust::Untrusted(vec![UntrustedReason::NotionalMismatch { mint: 1, expected: 1_000_000, real: 1_050_000 }])
        );
    }

    #[test]
    fn reconcile_untrusted_when_leg_balance_missing() {
        let intent = sample_intent();
        let balances = [(1, 1_000_000)]; // mint 2 entirely absent
        let trust = reconcile_intent(&intent, &balances);
        assert_eq!(trust, IntentTrust::Untrusted(vec![UntrustedReason::MissingLeg { mint: 2 }]));
    }

    #[test]
    fn reconcile_reports_every_mismatched_leg_not_just_first() {
        let intent = sample_intent();
        let balances = [(1, 2_000_000)]; // mint 1 badly mismatched, mint 2 missing entirely
        let trust = reconcile_intent(&intent, &balances);
        assert_eq!(
            trust,
            IntentTrust::Untrusted(vec![
                UntrustedReason::NotionalMismatch { mint: 1, expected: 1_000_000, real: 2_000_000 },
                UntrustedReason::MissingLeg { mint: 2 },
            ])
        );
    }

    #[test]
    fn reconcile_trusted_for_a_basket_with_no_legs() {
        let intent = OpenIntent { basket_id: 1, legs: vec![], entry_residual_pct: 0.0, entry_timestamp_secs: 0 };
        assert_eq!(reconcile_intent(&intent, &[]), IntentTrust::Trusted);
    }

    #[test]
    fn reconcile_trusted_when_expected_and_real_are_both_zero() {
        let intent = OpenIntent {
            basket_id: 1,
            legs: vec![IntentLeg { mint: 9, direction: LegDirection::Long, expected_notional: 0 }],
            entry_residual_pct: 0.0,
            entry_timestamp_secs: 0,
        };
        assert_eq!(reconcile_intent(&intent, &[(9, 0)]), IntentTrust::Trusted);
    }

    #[test]
    fn reconcile_untrusted_when_expected_zero_but_real_nonzero() {
        let intent = OpenIntent {
            basket_id: 1,
            legs: vec![IntentLeg { mint: 9, direction: LegDirection::Long, expected_notional: 0 }],
            entry_residual_pct: 0.0,
            entry_timestamp_secs: 0,
        };
        let trust = reconcile_intent(&intent, &[(9, 100)]);
        assert_eq!(trust, IntentTrust::Untrusted(vec![UntrustedReason::NotionalMismatch { mint: 9, expected: 0, real: 100 }]));
    }
}
