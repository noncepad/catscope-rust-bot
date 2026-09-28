//! Stdin/stdout message serialization for phoenixperpsv1.
//! Mirrors `arbv1::message`'s shape, trimmed to only what this strategy
//! needs: `Wallet` (the signing keypair) inbound, nothing outbound yet --
//! there's no Go-side `optimizer/brain/phoenixperpsv1` counterpart to talk
//! to, so `CustomMessageOutbound` is a true no-op placeholder for now.
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
};
use solana_sdk::{signature::Keypair, signer::Signer};
use std::{cell::UnsafeCell, rc::Rc};

pub enum CustomMessageInbound {
    Blank,
    Wallet(Rc<UnsafeCell<Keypair>>),
    /// Shared, cross-strategy: a live bundler tip update pushed by
    /// `optimizer/bundler.RunTipBroadcaster`. See
    /// `crate::bundler_message::BundlerTipUpdate`'s doc comment --
    /// consumed by `Wallet::apply_bundler_tip_update`, not this module
    /// directly.
    CommonBundlerTipUpdate(crate::bundler_message::BundlerTipUpdate),
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
