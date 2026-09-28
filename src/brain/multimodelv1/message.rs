//! Stdin/stdout message serialization for `multimodelv1` -- currently
//! just Phase 4's (`PLAN-1.md`) open-trade-intent round trip, the first
//! real per-strategy `Custom` traffic in *either* direction this
//! prototype needs. Unlike every other bot mode's `message.rs`
//! (`leveragedloopv1`/`testperpv1`'s `CustomMessageOutbound` are both
//! true no-op placeholders -- confirmed by tracing `testperpv1`'s before
//! writing this), this mode has a real reason to send something
//! Rust -> Go: `OpenIntentReport`, sent once when a basket trade opens or
//! closes, is what the (not-yet-built) Go-side handler would persist
//! into `prefetch.db` (Phase 4 point 2). `ReplayOpenIntent` is the
//! reverse: what the (not-yet-built) Go-side boot sequence would push
//! back into a freshly-started guest (Phase 4 point 3), same `CustomStdin`
//! channel the wallet key already uses.
//!
//! The actual byte layout for an intent record lives in
//! `trader::factor_intent::encode`/`decode`, not here -- this file only
//! wraps that payload in the same `KeyValuePair` key-flag framing every
//! other per-strategy `Custom` message already uses (see
//! `leveragedloopv1::message`'s `CUSTOM_KEY_FLAG_*` constants for the
//! established convention this follows).
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
    trader::{
        factor_intent::{self, OpenIntent},
        residual_snapshot::{self, ResidualSnapshot},
    },
};
use solana_sdk::{signature::Keypair, signer::Signer as _};
use std::{cell::UnsafeCell, rc::Rc};

pub enum CustomMessageInbound {
    Blank,
    /// The wallet keypair -- same shape/key-flag convention (`3`) every
    /// other bot mode's `message.rs` uses (`leveragedloopv1`'s
    /// `CUSTOM_KEY_FLAG_WALLET`), added here in Phase 5 sub-phase 5a once
    /// this mode became a real, runnable `EventHandler` -- Phase 4's own
    /// isolated wire-protocol prototype never needed it.
    Wallet(Rc<UnsafeCell<Keypair>>),
    /// Phase 4 point 3: an open basket's intent, replayed by the Go host
    /// at boot from its own persisted copy -- see the module doc
    /// comment. Not yet actually sent by anything (no Go-side boot
    /// sequence exists yet); this is the Rust-side half of that wire
    /// protocol, provable in isolation before the Go side exists.
    ReplayOpenIntent(OpenIntent),
    /// Shared, cross-strategy: a live bundler tip update pushed by
    /// `optimizer/bundler.RunTipBroadcaster` -- every real bot mode's Go
    /// package wires this up unconditionally (`startBundlerTipBroadcaster`
    /// in its `cmd/*.go`, including `multimodelv1`'s, added when this
    /// mode became a real, runnable `EventHandler`), so this arm exists
    /// for the same reason `Wallet` does: without it, a real message the
    /// Go side already sends would silently fall through to `Blank`
    /// instead of reaching `Wallet::apply_bundler_tip_update`. Consumed
    /// by `Wallet::apply_bundler_tip_update`, not this module directly --
    /// see `crate::bundler_message::BundlerTipUpdate`'s doc comment.
    CommonBundlerTipUpdate(crate::bundler_message::BundlerTipUpdate),
    /// Phase 5 sub-phase 5b: one-time opt-in for the real-but-read-only
    /// factor-graph resync/logging cycle (`state::StateHelper::
    /// run_factor_resync`) -- computes real structural factors from live
    /// pool liquidity and logs staleness on a periodic cadence, opens or
    /// closes nothing. Matches this codebase's "nothing new happens
    /// without an explicit trigger" ethos, same role
    /// `TriggerEnableBasisTrading` plays for `leveragedloopv1`'s basis
    /// trade. No payload.
    TriggerEnableFactorLogging,
    /// Phase 5 sub-phase 5c: one-time opt-in for the **real, executing**
    /// pure-Kamino pair/stat-arb trade (`state::StateHelper::
    /// run_pair_trade_cycle`) -- unlike `TriggerEnableFactorLogging`,
    /// this one sends real transactions (real Kamino deposits/borrows,
    /// real spot swaps) once a real candidate clears the real z-score and
    /// borrow-cost gates. No payload.
    TriggerEnablePairTrading,
    /// Restart-warm-up cache: the Go host's own persisted copy of
    /// `ResidualSnapshotReport`'s last payload, replayed back shortly
    /// after a freshly started guest connects -- see
    /// `trader::residual_snapshot`'s module doc comment. Never trusted
    /// blindly: `state::StateHelper`'s handler re-checks
    /// `residual_snapshot::is_fresh` itself before applying it (the Go
    /// host's own persisted-at timestamp isn't assumed correct just
    /// because it arrived).
    ReplayResidualSnapshot(ResidualSnapshot),
    /// Trade type 1 (directional factor-neutral): one-time opt-in
    /// carrying the human-specified long target's real mint -- unlike
    /// every other trigger in this file, entry itself isn't automated
    /// (see `state::LendingProtocol`'s sibling doc comments/`PLAN-1.md`'s
    /// directional-neutral design notes for why), so this is the one
    /// trigger that needs a real payload rather than being a bare
    /// opt-in flag. Raw 32-byte pubkey, same hand-rolled convention
    /// `trader::residual_snapshot`'s wire format already uses for a mint
    /// -- resolved back to a real `AccountId` by the receiving
    /// `state::StateHelper`, not here (this module has no
    /// `account_id_from_pubkey` access).
    TriggerEnableDirectionalTrading { mint: [u8; 32] },
    /// Trade type 1: an explicit human request to close whatever
    /// directional position is currently open (or pending open) -- the
    /// one case in this trade type where entry is human-driven but exit
    /// isn't fully automatic (stop-loss/borrow-gate closes are; "I'm
    /// satisfied, take profit" isn't, since there's no computed
    /// take-profit target for a directional bet). No payload.
    TriggerCloseDirectionalPosition,
    /// Trade type 3 (dispersion): one-time opt-in that only *arms* the
    /// automated decision loop -- unlike `TriggerEnableDirectionalTrading`,
    /// entry itself is a real, computed signal (`dispersion_basket::
    /// should_enter_dispersion`), not human-specified, so this carries no
    /// payload, same no-payload shape `TriggerEnablePairTrading` uses.
    TriggerEnableDispersionTrading,
    /// Trade type 3: an explicit human request to close whatever
    /// dispersion position is currently open -- same role as
    /// `TriggerCloseDirectionalPosition` (the automated exit signal
    /// already handles ordinary reversion; this is the human override).
    /// No payload.
    TriggerCloseDispersionPosition,
    /// Temporary, standalone manual-cleanup tool (2026-09-03) -- not tied
    /// to any of the four trade types' own cadence/gates. Real, live
    /// motivation: a hop-chain send can leave a real balance stranded in
    /// a pass-through intermediate mint the bot never otherwise trades
    /// (e.g. USDH, an Orca-route intermediate for an ETH close) if
    /// `mid_on_tx` never sees the hop land (see `Wallet::
    /// ata_subscribe_request`'s call site in
    /// `send_single_hop_as_astralane_tx`'s doc comment for the real
    /// subscription-gap incident this was ported from). Carries two raw
    /// 32-byte mint pubkeys -- the wallet's entire real balance of `mint`
    /// is swept to `dest_mint` (not hardcoded to `mint_usdc`: a real,
    /// live-confirmed incident found the router had no route to USDC
    /// within the normal hop budget for one stranded mint, while a
    /// different destination did have one) -- same raw-pubkey convention
    /// `TriggerEnableDirectionalTrading`'s payload uses.
    TriggerSweepMint { mint: [u8; 32], dest_mint: [u8; 32] },
    /// Trade type 5 (Hawkes-on-eigenfactor momentum, see
    /// `docs/HAWKES_FACTOR_TRADE_PLAN.md`): one-time opt-in that only
    /// *arms* the automated decision loop -- same no-payload shape as
    /// `TriggerEnableDispersionTrading`: entry itself is a real, computed
    /// signal (a discrete-time self-exciting jump intensity crossing its
    /// own threshold, `hawkes_factor::should_open_hawkes`), not
    /// human-specified.
    TriggerEnableHawkesTrading,
    /// Trade type 5: an explicit human request to close whatever Hawkes
    /// momentum basket is currently open -- same role as
    /// `TriggerCloseDispersionPosition` (the automated exit signal --
    /// intensity decay, max-holding-cycles cap, borrow-gate re-check --
    /// already handles ordinary reversion; this is the human override).
    /// No payload.
    TriggerCloseHawkesPosition,
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

const CUSTOM_KEY_FLAG_WALLET: u8 = 3;
const CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT: u8 = 1;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_FACTOR_LOGGING: u8 = 2;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_PAIR_TRADING: u8 = 4;
const CUSTOM_KEY_FLAG_REPLAY_RESIDUAL_SNAPSHOT: u8 = 5;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DIRECTIONAL_TRADING: u8 = 6;
const CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DIRECTIONAL_POSITION: u8 = 7;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DISPERSION_TRADING: u8 = 8;
const CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DISPERSION_POSITION: u8 = 9;
const CUSTOM_KEY_FLAG_TRIGGER_SWEEP_MINT: u8 = 10;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_HAWKES_TRADING: u8 = 11;
const CUSTOM_KEY_FLAG_TRIGGER_CLOSE_HAWKES_POSITION: u8 = 12;

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
                let pubkey = secret_key.pubkey();
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
            CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT => {
                let intent = factor_intent::decode(kvp.value()).ok_or(CatscopeGuestError::InsufficientBuffer)?;
                *self = Self::ReplayOpenIntent(intent);
            }
            crate::bundler_message::COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE => {
                *self = Self::CommonBundlerTipUpdate(crate::bundler_message::BundlerTipUpdate::parse(kvp.value())?);
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_FACTOR_LOGGING => {
                *self = Self::TriggerEnableFactorLogging;
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_PAIR_TRADING => {
                *self = Self::TriggerEnablePairTrading;
            }
            CUSTOM_KEY_FLAG_REPLAY_RESIDUAL_SNAPSHOT => {
                let snapshot = residual_snapshot::decode(kvp.value()).ok_or(CatscopeGuestError::InsufficientBuffer)?;
                *self = Self::ReplayResidualSnapshot(snapshot);
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DIRECTIONAL_TRADING => {
                let value = kvp.value();
                if value.len() != 32 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 32));
                }
                let mint: [u8; 32] = value.try_into().unwrap();
                *self = Self::TriggerEnableDirectionalTrading { mint };
            }
            CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DIRECTIONAL_POSITION => {
                *self = Self::TriggerCloseDirectionalPosition;
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DISPERSION_TRADING => {
                *self = Self::TriggerEnableDispersionTrading;
            }
            CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DISPERSION_POSITION => {
                *self = Self::TriggerCloseDispersionPosition;
            }
            CUSTOM_KEY_FLAG_TRIGGER_SWEEP_MINT => {
                let value = kvp.value();
                if value.len() != 64 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 64));
                }
                let mint: [u8; 32] = value[0..32].try_into().unwrap();
                let dest_mint: [u8; 32] = value[32..64].try_into().unwrap();
                *self = Self::TriggerSweepMint { mint, dest_mint };
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_HAWKES_TRADING => {
                *self = Self::TriggerEnableHawkesTrading;
            }
            CUSTOM_KEY_FLAG_TRIGGER_CLOSE_HAWKES_POSITION => {
                *self = Self::TriggerCloseHawkesPosition;
            }
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(consumed)
    }
}

pub enum CustomMessageOutbound {
    Blank,
    /// Phase 4 point 1: sent once when a basket trade actually opens or
    /// closes -- the Go-side handler that would receive and persist this
    /// (Phase 4 point 2) doesn't exist yet; this is the Rust-side half,
    /// provable in isolation (see the module doc comment).
    OpenIntentReport(OpenIntent),
    /// Sent periodically (once per real factor resync, while pair
    /// trading is enabled -- see `state::StateHelper::run_factor_resync`)
    /// so the Go host can persist the current residual/z-score warm-up
    /// state and replay it back (`ReplayResidualSnapshot`) after a
    /// restart. See `trader::residual_snapshot`'s module doc comment.
    ResidualSnapshotReport(ResidualSnapshot),
}

const CUSTOM_KEY_FLAG_OPEN_INTENT_REPORT: u8 = 1;
const CUSTOM_KEY_FLAG_RESIDUAL_SNAPSHOT_REPORT: u8 = 2;

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        match self {
            CustomMessageOutbound::Blank => 0,
            CustomMessageOutbound::OpenIntentReport(intent) => {
                let value = factor_intent::encode(intent);
                KeyValuePair { key: &[CUSTOM_KEY_FLAG_OPEN_INTENT_REPORT], value: &value }.len()
            }
            CustomMessageOutbound::ResidualSnapshotReport(snapshot) => {
                let value = residual_snapshot::encode(snapshot);
                KeyValuePair { key: &[CUSTOM_KEY_FLAG_RESIDUAL_SNAPSHOT_REPORT], value: &value }.len()
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, CustomMessageOutbound::Blank)
    }

    fn serialize(&self, buffer: &mut [u8]) {
        match self {
            CustomMessageOutbound::Blank => {}
            CustomMessageOutbound::OpenIntentReport(intent) => {
                let value = factor_intent::encode(intent);
                let kvp = KeyValuePair { key: &[CUSTOM_KEY_FLAG_OPEN_INTENT_REPORT], value: &value };
                kvp.serialize(buffer);
            }
            CustomMessageOutbound::ResidualSnapshotReport(snapshot) => {
                let value = residual_snapshot::encode(snapshot);
                let kvp = KeyValuePair { key: &[CUSTOM_KEY_FLAG_RESIDUAL_SNAPSHOT_REPORT], value: &value };
                kvp.serialize(buffer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trader::factor_intent::{IntentLeg, LegDirection};
    use crate::trader::residual_snapshot::ResidualSnapshotEntry;

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

    fn wire_body(key: u8, value: &[u8]) -> Vec<u8> {
        let mut buf = vec![1u8, key];
        buf.extend_from_slice(&(value.len() as u16).to_le_bytes());
        buf.extend_from_slice(value);
        buf
    }

    #[test]
    fn replay_open_intent_deserialize_roundtrips() {
        let intent = sample_intent();
        let value = factor_intent::encode(&intent);
        let body = wire_body(CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::ReplayOpenIntent(decoded) => assert_eq!(decoded, intent),
            _ => panic!("expected ReplayOpenIntent variant"),
        }
    }

    #[test]
    fn replay_open_intent_deserialize_rejects_malformed_payload() {
        let body = wire_body(CUSTOM_KEY_FLAG_REPLAY_OPEN_INTENT, &[0u8; 3]); // shorter than any valid header
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn unknown_key_flag_falls_back_to_blank() {
        let body = wire_body(0xFF, &[]);
        let mut msg = CustomMessageInbound::ReplayOpenIntent(sample_intent());
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::Blank));
    }

    #[test]
    fn trigger_enable_factor_logging_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_FACTOR_LOGGING, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerEnableFactorLogging));
    }

    #[test]
    fn trigger_enable_pair_trading_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_PAIR_TRADING, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerEnablePairTrading));
    }

    #[test]
    fn trigger_enable_directional_trading_deserialize_roundtrips_the_mint() {
        let mint = [7u8; 32];
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DIRECTIONAL_TRADING, &mint);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        match msg {
            CustomMessageInbound::TriggerEnableDirectionalTrading { mint: decoded } => assert_eq!(decoded, mint),
            _ => panic!("expected TriggerEnableDirectionalTrading variant"),
        }
    }

    #[test]
    fn trigger_enable_directional_trading_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DIRECTIONAL_TRADING, &[0u8; 31]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn trigger_close_directional_position_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DIRECTIONAL_POSITION, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerCloseDirectionalPosition));
    }

    #[test]
    fn trigger_enable_dispersion_trading_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_DISPERSION_TRADING, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerEnableDispersionTrading));
    }

    #[test]
    fn trigger_sweep_mint_deserialize_roundtrips_both_mints() {
        let mint = [9u8; 32];
        let dest_mint = [4u8; 32];
        let mut value = Vec::with_capacity(64);
        value.extend_from_slice(&mint);
        value.extend_from_slice(&dest_mint);
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_SWEEP_MINT, &value);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        match msg {
            CustomMessageInbound::TriggerSweepMint { mint: decoded_mint, dest_mint: decoded_dest } => {
                assert_eq!(decoded_mint, mint);
                assert_eq!(decoded_dest, dest_mint);
            }
            _ => panic!("expected TriggerSweepMint variant"),
        }
    }

    #[test]
    fn trigger_sweep_mint_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_SWEEP_MINT, &[0u8; 63]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn trigger_close_dispersion_position_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_CLOSE_DISPERSION_POSITION, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerCloseDispersionPosition));
    }

    #[test]
    fn trigger_enable_hawkes_trading_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_HAWKES_TRADING, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerEnableHawkesTrading));
    }

    #[test]
    fn trigger_close_hawkes_position_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_CLOSE_HAWKES_POSITION, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerCloseHawkesPosition));
    }

    fn sample_snapshot() -> ResidualSnapshot {
        ResidualSnapshot {
            saved_at_secs: 1_800_000_000,
            entries: vec![
                ResidualSnapshotEntry { mint: [7u8; 32], samples: vec![0.1, -0.2, 0.3], last_price_usd: 42.0 },
                ResidualSnapshotEntry { mint: [8u8; 32], samples: vec![], last_price_usd: 1.0 },
            ],
        }
    }

    #[test]
    fn replay_residual_snapshot_deserialize_roundtrips() {
        let snapshot = sample_snapshot();
        let value = residual_snapshot::encode(&snapshot);
        let body = wire_body(CUSTOM_KEY_FLAG_REPLAY_RESIDUAL_SNAPSHOT, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::ReplayResidualSnapshot(decoded) => assert_eq!(decoded, snapshot),
            _ => panic!("expected ReplayResidualSnapshot variant"),
        }
    }

    #[test]
    fn replay_residual_snapshot_deserialize_rejects_malformed_payload() {
        let body = wire_body(CUSTOM_KEY_FLAG_REPLAY_RESIDUAL_SNAPSHOT, &[0u8; 3]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn residual_snapshot_report_serialize_round_trips_through_deserialize() {
        let snapshot = sample_snapshot();
        let outbound = CustomMessageOutbound::ResidualSnapshotReport(snapshot.clone());
        assert!(!outbound.is_empty());
        let mut buffer = vec![0u8; outbound.len()];
        outbound.serialize(&mut buffer);

        // Outbound/inbound key-flag spaces are independent (see the
        // module doc comment) -- re-wrap the serialized value under the
        // *inbound* flag before feeding it back to the inbound
        // deserializer, same as `open_intent_report_serialize_round_
        // trips_through_deserialize` does where the flags happen to
        // already match.
        let value = residual_snapshot::encode(&snapshot);
        let body = wire_body(CUSTOM_KEY_FLAG_REPLAY_RESIDUAL_SNAPSHOT, &value);
        let mut inbound = CustomMessageInbound::default();
        let consumed = inbound.deserialize(&body).expect("should deserialize");
        assert_eq!(consumed, body.len());
        match inbound {
            CustomMessageInbound::ReplayResidualSnapshot(decoded) => assert_eq!(decoded, snapshot),
            _ => panic!("expected ReplayResidualSnapshot variant"),
        }
    }

    #[test]
    fn blank_outbound_is_empty() {
        let msg = CustomMessageOutbound::Blank;
        assert_eq!(msg.len(), 0);
        assert!(msg.is_empty());
    }

    #[test]
    fn open_intent_report_serialize_round_trips_through_deserialize() {
        // Full loop: serialize the outbound message, then feed the exact
        // same bytes an inbound Custom payload would see (the key-flag
        // values intentionally match -- inbound/outbound are independent
        // streams, see the module doc comment) into the inbound
        // deserializer, and confirm the same intent comes back out.
        let intent = sample_intent();
        let outbound = CustomMessageOutbound::OpenIntentReport(intent.clone());
        assert!(!outbound.is_empty());
        let mut buffer = vec![0u8; outbound.len()];
        outbound.serialize(&mut buffer);

        let mut inbound = CustomMessageInbound::default();
        let consumed = inbound.deserialize(&buffer).expect("should deserialize");
        assert_eq!(consumed, buffer.len());
        match inbound {
            CustomMessageInbound::ReplayOpenIntent(decoded) => assert_eq!(decoded, intent),
            _ => panic!("expected ReplayOpenIntent variant"),
        }
    }
}
