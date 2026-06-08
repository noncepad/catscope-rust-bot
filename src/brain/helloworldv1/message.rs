use std::{cell::UnsafeCell, rc::Rc};

use solana_sdk::{signature::Keypair, signer::Signer};

use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
};

pub(crate) enum CustomMessageInbound {
    Blank,
    EchoRequest(String),
    Wallet(Rc<UnsafeCell<Keypair>>),
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}
const CUSTOM_KEY_FLAG_ECHO_REQUEST: u8 = 1;
const CUSTOM_KEY_FLAG_ECHO_RESPONSE: u8 = 2;
const CUSTOM_KEY_FLAG_WALLET: u8 = 3;
const CUSTOM_KEY_FLAG_TX_LATENCY: u8 = 4;

impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, body: &[u8]) -> Result<usize, CatscopeGuestError> {
        let kvp = KeyValuePair::try_from(body)?;
        let consumed = 1 + kvp.key().len() + 2 + kvp.value().len();
        let key = kvp.key();
        if key.len() != 1 {
            return Err(CatscopeGuestError::InsufficientBufferV2(key.len(), 1));
        }
        match key[0] {
            CUSTOM_KEY_FLAG_ECHO_REQUEST => {
                let s = std::str::from_utf8(kvp.value()).unwrap_or("").to_string();
                *self = Self::EchoRequest(s);
            }
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
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(consumed)
    }
}

pub(crate) enum CustomMessageOutbound {
    EchoResponse(String),
    /// p50 and p99 round-trip latency in microseconds for outbound transactions.
    /// Fields: (n, p50_us, p99_us). p50/p99 are 0 when n == 0.
    TxLatencyReport {
        n: u64,
        p50_us: u64,
        p99_us: u64,
    },
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        match self {
            Self::EchoResponse(s) => {
                let key = [CUSTOM_KEY_FLAG_ECHO_RESPONSE];
                let kvp = KeyValuePair {
                    key: &key,
                    value: s.as_bytes(),
                };
                kvp.len()
            }
            Self::TxLatencyReport { .. } => {
                // key: 1 byte flag; value: 3 × u64 = 24 bytes
                1 + 1 + 2 + 24
            }
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::EchoResponse(s) => s.is_empty(),
            Self::TxLatencyReport { .. } => false,
        }
    }
    fn serialize(&self, buffer: &mut [u8]) {
        match self {
            Self::EchoResponse(s) => {
                let key = [CUSTOM_KEY_FLAG_ECHO_RESPONSE];
                let kvp = KeyValuePair {
                    key: &key,
                    value: s.as_bytes(),
                };
                kvp.serialize(buffer);
            }
            Self::TxLatencyReport { n, p50_us, p99_us } => {
                let key = [CUSTOM_KEY_FLAG_TX_LATENCY];
                let mut value = [0u8; 24];
                value[0..8].copy_from_slice(&n.to_le_bytes());
                value[8..16].copy_from_slice(&p50_us.to_le_bytes());
                value[16..24].copy_from_slice(&p99_us.to_le_bytes());
                let kvp = KeyValuePair {
                    key: &key,
                    value: &value,
                };
                kvp.serialize(buffer);
            }
        }
    }
}
