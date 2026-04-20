use solana_sdk::pubkey::Pubkey;

use crate::{
    err::CatscopeGuestError,
    message::{MessageDeserializer, MessageSerializer},
    util::{as_bytes, as_bytes_mut},
};

pub(crate) enum CustomMessageInbound {
    ReturnWallet(Pubkey),
    Blank,
}
impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}
impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, key: &[u8], data: &[u8]) -> Result<(), CatscopeGuestError> {
        if key.is_empty() {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        match key[0] {
            2 => {
                let mut pubkey = Pubkey::default();
                let pubkey_slice = as_bytes_mut(&mut pubkey);
                if data.len() != pubkey_slice.len() {
                    return Err(CatscopeGuestError::BufferTooSmall);
                }
                pubkey_slice.copy_from_slice(data);
                *self = Self::ReturnWallet(pubkey)
            }
            _ => {
                *self = Self::Blank;
            }
        };
        Ok(())
    }
}

#[derive(Debug, Default)]
#[repr(C, align(8))]
pub struct Latency {
    pub tx_first_sig: u64,
}

pub(crate) enum CustomMessageOutbound {
    Latency(Latency),
}
impl CustomMessageOutbound {
    fn flag(&self) -> u8 {
        match self {
            CustomMessageOutbound::Latency(_latency) => 1,
        }
    }
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        let payload = match self {
            CustomMessageOutbound::Latency(_latency) => std::mem::size_of::<Latency>(),
        };
        1 + payload
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn serialize(&self, buffer: &mut [u8]) {
        let mut i = 0;
        buffer[i] = self.flag();
        i += 1;
        let n = self.len();
        if buffer.len() - i < n {
            panic!("buffer too small: {} {}", buffer.len(), 1 + n)
        }
        let subbuf = &mut buffer[i..(i + n)];
        match self {
            CustomMessageOutbound::Latency(latency) => {
                let slice = as_bytes(latency);
                subbuf.copy_from_slice(slice);
            }
        }
    }
}
