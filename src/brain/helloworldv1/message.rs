use crate::{
    err::CatscopeGuestError,
    message::{MessageDeserializer, MessageSerializer},
};

pub(crate) enum CustomMessageInbound {
    Blank,
    EchoRequest(String),
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, key: &[u8], data: &[u8]) -> Result<(), CatscopeGuestError> {
        match key {
            b"bot_echo_v1" => {
                let s = std::str::from_utf8(data).unwrap_or("").to_string();
                *self = Self::EchoRequest(s);
            }
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(())
    }
}

pub(crate) enum CustomMessageOutbound {
    EchoReply(String),
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        match self {
            Self::EchoReply(s) => s.len(),
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::EchoReply(s) => s.is_empty(),
        }
    }
    fn serialize(&self, buffer: &mut [u8]) {
        match self {
            Self::EchoReply(s) => buffer.copy_from_slice(s.as_bytes()),
        }
    }
}
