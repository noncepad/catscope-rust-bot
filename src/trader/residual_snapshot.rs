//! Cross-restart persistence for `brain::multimodelv1`'s pair-trading
//! residual/z-score history (`state::State::m_residual_history`/
//! `m_last_price_usd`) -- **not** the same restart-safety concern
//! `factor_intent.rs` solves. That module persists *open-trade intent*
//! (what's actually on-chain, which has no real representation of "which
//! basket" a balance belongs to). This module persists a purely
//! statistical warm-up cache: the last `RESIDUAL_WINDOW_CAPACITY` real
//! residual samples per mint plus each mint's last observed price, so a
//! freshly restarted guest doesn't have to sit through the same
//! ~`RESIDUAL_WINDOW_CAPACITY`-cycle (~15 minute) warm-up every time the
//! process restarts (live-confirmed, repeatedly, during this session's
//! own testing -- every one of several redeploys needed a fresh ~10-cycle
//! climb from "not enough residual history" before real z-scores
//! reappeared). Losing this cache is never a correctness problem (the
//! bot already treats "not enough history" as a real, valid, fail-closed
//! state -- see `factor_residual::MIN_SAMPLES_FOR_ZSCORE`), only a
//! availability/latency one, which is exactly why this module treats a
//! too-old snapshot as worthless and discards it rather than replaying
//! stale statistics into a live trading decision (see [`is_fresh`]).
//!
//! Wire identity is the mint's own raw 32-byte pubkey, **not** an
//! `AccountId` -- unlike `factor_intent::IntentLeg::mint`, which persists
//! an `AccountId` on the (separate, still-unbuilt) assumption that the
//! host's pubkey<->id mapping is itself stable across a restart. This
//! module makes no such assumption: raw pubkey bytes are resolved back
//! to a fresh `AccountId` via `util::account_id_from_pubkey` only after
//! replay, on the guest's own new connection -- the same pattern
//! `trade_universe_config`'s build-time-embedded mints already use.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `factor_intent.rs`/`factor_graph.rs`. [`encode`]/[`decode`] are the
//! wire format `brain::multimodelv1::message` wraps in the same
//! `KeyValuePair` framing every other per-strategy `Custom` message uses.

/// One mint's persisted warm-up state: its real recent residual samples
/// (oldest-first, same order `factor_residual::RollingWindow::samples`
/// yields) and its last observed real price -- both needed to resume
/// immediately: the samples alone aren't enough to compute a real
/// z-score on the very first post-restart cycle without a `prev` price
/// to diff the next live price against.
#[derive(Debug, Clone, PartialEq)]
pub struct ResidualSnapshotEntry {
    pub mint: [u8; 32],
    pub samples: Vec<f64>,
    pub last_price_usd: f64,
}

/// The full persisted snapshot -- one real Unix timestamp (when it was
/// taken, for [`is_fresh`]'s staleness check) plus one entry per mint
/// that had real residual history at save time.
#[derive(Debug, Clone, PartialEq)]
pub struct ResidualSnapshot {
    pub saved_at_secs: i64,
    pub entries: Vec<ResidualSnapshotEntry>,
}

/// A replayed snapshot is only trustworthy if it's no older than the
/// window it's meant to seed -- `max_age_secs` should be the caller's
/// own real `RESIDUAL_WINDOW_CAPACITY * <real resync period>` (the same
/// bound the live window itself represents), not a separately-guessed
/// number. A snapshot from a bot that's been down for hours describes a
/// market that's moved on; replaying it would silently feed a stale
/// mean/stdev into a real z-score decision instead of the honest
/// "not enough history yet" state a cold start already handles
/// correctly. `now_secs < saved_at_secs` (clock skew, or a corrupt
/// record) is treated the same as "too old" -- never trusted.
pub fn is_fresh(saved_at_secs: i64, now_secs: i64, max_age_secs: i64) -> bool {
    let age = now_secs - saved_at_secs;
    (0..=max_age_secs).contains(&age)
}

const HEADER_WIRE_SIZE: usize = 8 + 2; // saved_at_secs (i64) + n_entries (u16)
const ENTRY_FIXED_WIRE_SIZE: usize = 32 + 1 + 8; // mint + n_samples (u8) + last_price_usd (f64)

/// Real byte layout, little-endian throughout, matching this crate's
/// existing hand-rolled wire framing convention (see `factor_intent::
/// encode`'s doc comment for why -- no `bincode`/`wincode` on this pipe):
/// `[saved_at_secs i64][n_entries u16]
/// [(mint [u8;32], n_samples u8, samples f64 x n_samples, last_price_usd f64) x n_entries]`.
/// `n_samples` fits `u8` with room to spare -- real
/// `RESIDUAL_WINDOW_CAPACITY` is 30, far under 256; a snapshot entry
/// carrying more than 255 samples (can't happen from a real
/// `RollingWindow`, whose own capacity gates this) is silently
/// truncated to its most recent 255 rather than panicking a live
/// trading bot on an internal invariant that isn't caller-facing input.
pub fn encode(snapshot: &ResidualSnapshot) -> Vec<u8> {
    let body_len: usize = snapshot.entries.iter().map(|e| ENTRY_FIXED_WIRE_SIZE + e.samples.len().min(u8::MAX as usize) * 8).sum();
    let mut out = Vec::with_capacity(HEADER_WIRE_SIZE + body_len);
    out.extend_from_slice(&snapshot.saved_at_secs.to_le_bytes());
    out.extend_from_slice(&(snapshot.entries.len() as u16).to_le_bytes());
    for entry in &snapshot.entries {
        out.extend_from_slice(&entry.mint);
        let n = entry.samples.len().min(u8::MAX as usize);
        out.push(n as u8);
        let start = entry.samples.len() - n; // keep the most recent `n`
        for &s in &entry.samples[start..] {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out.extend_from_slice(&entry.last_price_usd.to_le_bytes());
    }
    out
}

/// Inverse of [`encode`]. `None` on any malformed/truncated/trailing
/// -garbage input -- never partially decodes, same discipline
/// `factor_intent::decode` establishes: a caller that can't fully parse
/// a persisted snapshot must treat that the same as "no snapshot",
/// never act on a partial one.
pub fn decode(bytes: &[u8]) -> Option<ResidualSnapshot> {
    if bytes.len() < HEADER_WIRE_SIZE {
        return None;
    }
    let saved_at_secs = i64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let n_entries = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as usize;
    let mut entries = Vec::with_capacity(n_entries);
    let mut off = HEADER_WIRE_SIZE;
    for _ in 0..n_entries {
        if off + 32 + 1 > bytes.len() {
            return None;
        }
        let mint: [u8; 32] = bytes[off..off + 32].try_into().ok()?;
        off += 32;
        let n_samples = bytes[off] as usize;
        off += 1;
        let samples_bytes = n_samples * 8;
        if off + samples_bytes + 8 > bytes.len() {
            return None;
        }
        let mut samples = Vec::with_capacity(n_samples);
        for i in 0..n_samples {
            let s_off = off + i * 8;
            samples.push(f64::from_le_bytes(bytes[s_off..(s_off + 8)].try_into().ok()?));
        }
        off += samples_bytes;
        let last_price_usd = f64::from_le_bytes(bytes[off..(off + 8)].try_into().ok()?);
        off += 8;
        entries.push(ResidualSnapshotEntry { mint, samples, last_price_usd });
    }
    if off != bytes.len() {
        return None;
    }
    Some(ResidualSnapshot { saved_at_secs, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> ResidualSnapshot {
        ResidualSnapshot {
            saved_at_secs: 1_800_000_000,
            entries: vec![
                ResidualSnapshotEntry { mint: [1u8; 32], samples: vec![0.1, -0.2, 0.35], last_price_usd: 123.45 },
                ResidualSnapshotEntry { mint: [2u8; 32], samples: vec![], last_price_usd: 1.0 },
            ],
        }
    }

    #[test]
    fn encode_decode_roundtrips() {
        let snapshot = sample_snapshot();
        let bytes = encode(&snapshot);
        let decoded = decode(&bytes).expect("should decode");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn encode_decode_roundtrips_zero_entries() {
        let snapshot = ResidualSnapshot { saved_at_secs: 42, entries: vec![] };
        let bytes = encode(&snapshot);
        let decoded = decode(&bytes).expect("should decode");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn decode_rejects_truncated_header() {
        assert!(decode(&[0u8; 5]).is_none());
    }

    #[test]
    fn decode_rejects_truncated_entry() {
        let snapshot = sample_snapshot();
        let mut bytes = encode(&snapshot);
        bytes.truncate(bytes.len() - 3); // chop mid-way through the last entry
        assert!(decode(&bytes).is_none());
    }

    #[test]
    fn decode_rejects_trailing_garbage() {
        let snapshot = sample_snapshot();
        let mut bytes = encode(&snapshot);
        bytes.push(0xFF);
        assert!(decode(&bytes).is_none());
    }

    #[test]
    fn encode_truncates_oversized_sample_count_to_most_recent_255() {
        let many: Vec<f64> = (0..300).map(|i| i as f64).collect();
        let snapshot =
            ResidualSnapshot { saved_at_secs: 1, entries: vec![ResidualSnapshotEntry { mint: [9u8; 32], samples: many, last_price_usd: 1.0 }] };
        let decoded = decode(&encode(&snapshot)).expect("should decode");
        assert_eq!(decoded.entries[0].samples.len(), 255);
        // most recent 255 of 0..300 is 45..300
        assert_eq!(decoded.entries[0].samples[0], 45.0);
        assert_eq!(decoded.entries[0].samples[254], 299.0);
    }

    // --- is_fresh ----------------------------------------------------

    #[test]
    fn is_fresh_true_within_bound() {
        assert!(is_fresh(1000, 1000 + 900, 900));
        assert!(is_fresh(1000, 1000, 900)); // zero age
    }

    #[test]
    fn is_fresh_false_past_bound() {
        assert!(!is_fresh(1000, 1000 + 901, 900));
    }

    #[test]
    fn is_fresh_false_for_future_saved_at() {
        // saved_at in the future relative to now -- clock skew or a
        // corrupt record, never trusted either way.
        assert!(!is_fresh(2000, 1000, 900));
    }
}
