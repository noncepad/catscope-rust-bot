//! Stdin/stdout message plumbing for `midontxhealthv1`. Originally a
//! pure no-op (this bot was purely passive/listen-only) -- now handles
//! the real `Wallet` key too, since this module also sends a small
//! number of real signed transfers (see `state`'s own doc comment for
//! why: checking whether *our own* transaction is ever seen via the
//! Transaction lane, and at what latency, needs a real signer).
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
};
use solana_sdk::{signature::Keypair, signer::Signer};
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
        if key.len() == 1 && key[0] == CUSTOM_KEY_FLAG_WALLET {
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
        } else {
            *self = Self::Blank;
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
