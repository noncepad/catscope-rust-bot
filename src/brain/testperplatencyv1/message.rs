//! Stdin/stdout message serialization for perpfundingv1.
//! Mirrors `phoenixperpsv1::message`'s shape exactly, trimmed to only
//! what this strategy needs: `Wallet` (kept for harness-lifecycle
//! consistency, unused for real sends) and `TargetAllocation` (real
//! target-portfolio-allocation updates pushed at runtime by
//! `optimizer/brain/perpfundingv1`'s `SendTargetAllocation`, see that
//! module's doc) inbound, nothing outbound yet -- there's no Go-side
//! reply this strategy needs to send, so `CustomMessageOutbound` is a
//! true no-op placeholder.
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
};
use solana_sdk::{signature::Keypair, signer::Signer};
use std::{cell::UnsafeCell, rc::Rc};

pub enum CustomMessageInbound {
    Blank,
    Wallet(Rc<UnsafeCell<Keypair>>),
    /// `(symbol, allocation_pct)` -- target fraction of total portfolio
    /// value (0.0-1.0) to hold in `symbol`, e.g. `0.30` means "target
    /// 30% of the portfolio in this symbol". The remainder is
    /// implicitly USD/stable -- no explicit USD entry. Rebalancing
    /// toward this target is what realizes profit/loss; this message
    /// only carries the target, not the rebalance itself. Mirrors the
    /// wire format `optimizer/brain/perpfundingv1/message.go`'s
    /// `DoTargetAllocation` encodes: a 24-byte value, 16-byte
    /// zero-padded symbol + 8-byte little-endian `f64`.
    TargetAllocation(String, f64),
    /// Shared, cross-strategy: a live bundler tip update pushed by
    /// `optimizer/bundler.RunTipBroadcaster`. See
    /// `crate::bundler_message::BundlerTipUpdate`'s doc comment --
    /// consumed by `Wallet::apply_bundler_tip_update`, not this module
    /// directly. Needed for the native-transfer loop's real Astralane
    /// bundled-send path (see `StateHelper::test_native_transfer_loop`'s
    /// doc comment) -- without this, `Wallet::select_tip_account`/
    /// `append_bundler_tip` never see any tip data and that path always
    /// falls back to a plain unbundled send.
    CommonBundlerTipUpdate(crate::bundler_message::BundlerTipUpdate),
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

const CUSTOM_KEY_FLAG_WALLET: u8 = 3;
const CUSTOM_KEY_FLAG_TARGET_ALLOCATION: u8 = 4;

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
            CUSTOM_KEY_FLAG_TARGET_ALLOCATION => {
                let value = kvp.value();
                if value.len() != 24 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 24));
                }
                let symbol = std::str::from_utf8(&value[0..16])
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
                let allocation_pct = f64::from_le_bytes(value[16..24].try_into().unwrap());
                *self = Self::TargetAllocation(symbol, allocation_pct);
            }
            crate::bundler_message::COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE => {
                *self = Self::CommonBundlerTipUpdate(
                    crate::bundler_message::BundlerTipUpdate::parse(kvp.value())?,
                );
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
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        0
    }
    fn is_empty(&self) -> bool {
        true
    }
    fn serialize(&self, _buffer: &mut [u8]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-builds a raw `KeyValuePair` wire body
    /// (`[1-byte key_len][key][2-byte LE value_len][value]`, per
    /// `message::KeyValuePair`'s `TryFrom`/`MessageSerializer` impls)
    /// for a single-byte key, so these tests exercise the exact bytes
    /// `optimizer/brain/perpfundingv1/message.go`'s
    /// `DoTargetAllocation`/`DoWallet` produce on the wire, not just the
    /// Rust-side struct.
    fn wire_body(key: u8, value: &[u8]) -> Vec<u8> {
        let mut buf = vec![1u8, key];
        buf.extend_from_slice(&(value.len() as u16).to_le_bytes());
        buf.extend_from_slice(value);
        buf
    }

    #[test]
    fn target_allocation_deserialize_roundtrips_symbol_and_pct() {
        let mut value = [0u8; 24];
        value[..3].copy_from_slice(b"SOL");
        value[16..24].copy_from_slice(&(0.30f64).to_le_bytes());
        let body = wire_body(CUSTOM_KEY_FLAG_TARGET_ALLOCATION, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::TargetAllocation(symbol, allocation_pct) => {
                assert_eq!(symbol, "SOL");
                assert_eq!(allocation_pct, 0.30);
            }
            _ => panic!("expected TargetAllocation variant"),
        }
    }

    #[test]
    fn target_allocation_deserialize_trims_trailing_zero_padding() {
        let mut value = [0u8; 24];
        value[..3].copy_from_slice(b"SUI"); // shorter than 16 bytes, zero-padded
        value[16..24].copy_from_slice(&0.0f64.to_le_bytes());
        let body = wire_body(CUSTOM_KEY_FLAG_TARGET_ALLOCATION, &value);

        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");

        match msg {
            CustomMessageInbound::TargetAllocation(symbol, _) => assert_eq!(symbol, "SUI"),
            _ => panic!("expected TargetAllocation variant"),
        }
    }

    #[test]
    fn target_allocation_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TARGET_ALLOCATION, &[0u8; 23]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn unknown_key_flag_falls_back_to_blank() {
        let body = wire_body(0xFF, &[]);
        let mut msg = CustomMessageInbound::TargetAllocation("SOL".to_string(), 1.0);
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::Blank));
    }
}
