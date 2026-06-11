use std::{cell::UnsafeCell, rc::Rc};

use solana_sdk::{signature::Keypair, signer::Signer};

use crate::{
    brain::helloworldv1::state::LatencyReportV1,
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
const CUSTOM_KEY_FLAG_LATENCY_REPORT_V1: u8 = 5;

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
    LatencyReportV1(LatencyReportV1),
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
            Self::LatencyReportV1(_) => {
                // key_len(1) + key(1) + value_len(2) + 9×u64(72)
                1 + 1 + 2 + 72
            }
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::EchoResponse(s) => s.is_empty(),
            Self::LatencyReportV1(_) => false,
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
            Self::LatencyReportV1(report) => {
                let key = [CUSTOM_KEY_FLAG_LATENCY_REPORT_V1];
                let mut value = [0u8; 72];
                let mut i = 0;
                value[i..(i + 8)]
                    .copy_from_slice(&(report.processed_diff.as_nanos() as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)]
                    .copy_from_slice(&(report.root_diff.as_nanos() as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&(report.account_processed as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&(report.account_root as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)]
                    .copy_from_slice(&(report.tx_processed_filter as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&(report.tx_processed as u64).to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&report.tx_n.to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&report.tx_p50_us.to_le_bytes());
                i += 8;
                value[i..(i + 8)].copy_from_slice(&report.tx_p99_us.to_le_bytes());
                let kvp = KeyValuePair {
                    key: &key,
                    value: &value,
                };
                kvp.serialize(buffer);
            }
        }
    }
}
