use std::time::{Duration, SystemTime, UNIX_EPOCH};

use solana_sdk::{pubkey::Pubkey, signature::Keypair};

use crate::{
    err::CatscopeGuestError,
    stdio::{MessageInbound, MessageOutbound, MessageType},
    util::{as_bytes, as_bytes_mut},
};

pub trait InboundMesasgeHandler<C: Sized + Default, M_IN: MessageDeserializer, M_OUT> {
    fn on_message(&mut self, action: MessageAction<C, M_IN>);
    fn message_send(&mut self, message: MessageSend<M_OUT>);
}
pub enum MessageSend<M> {
    Wallet(Pubkey),
    Pong(SystemTime),
    Custom(M),
}
pub trait MessageDeserializer: Default {
    fn deserialize(&mut self, key: &[u8], data: &[u8]) -> Result<(), CatscopeGuestError>;
}

pub trait MessageSerializer {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn serialize(&self, buffer: &mut [u8]);
}

pub enum MessageAction<C: Sized + Default, M_IN: MessageDeserializer> {
    PrivateKey(Keypair),
    Ping(SystemTime),
    Custom(M_IN),
    AdjustConfiguration(C),
    Shutdown,
}

impl<C: Sized + Default, M_IN: MessageDeserializer> TryFrom<&MessageInbound>
    for MessageAction<C, M_IN>
{
    type Error = CatscopeGuestError;

    fn try_from(msg: &MessageInbound) -> Result<Self, Self::Error> {
        let key = msg.key();
        match key {
            b"mothership_config_v1" => {
                let mut config = C::default();
                let slice = as_bytes_mut(&mut config);
                let v = msg.value();
                if slice.len() != v.len() {
                    return Err(CatscopeGuestError::Unknown(format!(
                        "bad private key size: {} vs {}",
                        v.len(),
                        slice.len()
                    )));
                }
                slice.copy_from_slice(v);
                Ok(MessageAction::AdjustConfiguration(config))
            }
            b"mothership_privkey_v1" => {
                let v = msg.value();
                if v.len() != 32 {
                    return Err(CatscopeGuestError::Unknown(format!(
                        "bad private key size: {} vs 64",
                        v.len()
                    )));
                }
                let x: [u8; 32] = v.try_into().unwrap();
                let y = Keypair::new_from_array(x);
                Ok(MessageAction::PrivateKey(y))
            }
            b"mothership_ping_v1" => {
                let v = msg.value();
                if v.len() != 8 {
                    return Err(CatscopeGuestError::Unknown(format!(
                        "bad message size: {} vs 8",
                        v.len()
                    )));
                }
                let x: [u8; 8] = v.try_into().unwrap();
                let st = UNIX_EPOCH + Duration::from_secs(u64::from_le_bytes(x));
                Ok(MessageAction::Ping(st))
            }
            b"mothership_shutdown_v1" => Ok(MessageAction::Shutdown),
            _ => {
                let mut x = M_IN::default();
                x.deserialize(key, msg.value())?;
                Ok(MessageAction::Custom(x))
            }
        }
    }
}
