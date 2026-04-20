use solana_sdk::pubkey::Pubkey;

use crate::{
    catscope::witbot::general,
    err::CatscopeGuestError,
    event_loop::EventHandler,
    log_debug,
    message::{MessageSend, MessageSerializer},
    util::as_bytes,
};
use std::{cell::UnsafeCell, collections::HashMap, rc::Rc, time::UNIX_EPOCH};

/// Create a fixed size data packet to send out on stdout or stderr.
pub struct StdioPacket {
    size: usize,
    channel: u8,
    buffer: Box<[u8; STDIO_PACKET_BUFFER_MAX]>,
    do_debug: bool,
}

impl StdioPacket {
    /// Create a data packet for stdout.
    pub fn stdout() -> Self {
        Self::new(1)
    }

    /// Create a data packet for stderr.
    pub fn stderr() -> Self {
        Self::new(2)
    }
    /// Send messages out via stdio pipe.
    pub fn write_message(&mut self, key: &MessageKey, value: &[u8]) {
        assert_eq!(self.channel, 1, "channel must be stdout");
        assert!(value.len() <= MESSAGE_MAX_SIZE);
        let t = value.len() as u16;
        let t1: [u8; 2] = t.to_le_bytes();
        // make sure we flush the message and do not run out of buffer.
        self.flush();
        self.append(&t1);
        self.append(key);
        self.append(value);
        self.flush();
    }

    /// Create a data packet.
    pub fn new(channel: u8) -> Self {
        let buffer = Box::new([0u8; STDIO_PACKET_BUFFER_MAX]);
        let do_debug = std::env::var("DEBUG").is_ok();
        Self {
            channel,
            size: 0,
            buffer,
            do_debug,
        }
    }
    pub fn write(&mut self, data: &[u8]) {
        assert!(
            data.len() < self.buffer.len(),
            "data too big {} vs {}",
            data.len(),
            self.buffer.len(),
        );
        let subbuf = &mut self.buffer[0..data.len()];
        subbuf.copy_from_slice(data);
        self.size = data.len();
    }

    pub fn append(&mut self, data: &[u8]) {
        if self.buffer.len() < self.size + data.len() {
            self.flush();
        }
        let start = self.size;
        self.size += data.len();
        let subbuf = &mut self.buffer[start..self.size];
        subbuf.copy_from_slice(data);
    }

    pub fn data(&self) -> &[u8] {
        &self.buffer[0..self.size]
    }

    ///  send the message out
    pub fn flush(&mut self) {
        if self.size == 0 {
            return;
        }
        general::stdout(self.channel, &self.buffer[0..self.size]);
        self.size = 0;
    }
}

const STDIO_PACKET_BUFFER_MAX: usize = 2 * MESSAGE_MAX_SIZE;

pub trait MessageHook {
    fn on_message(&mut self, data: &[u8]);
}

pub type MessageKey = [u8; MESSAGE_KEY_SIZE];

pub fn message_key_from_str(keystr: &str) -> MessageKey {
    let mut output = [0u8; MESSAGE_KEY_SIZE];
    let data = keystr.as_bytes();
    assert!(
        data.len() < output.len(),
        "buffer overflow: {} {}",
        output.len(),
        data.len()
    );
    output[..data.len()].copy_from_slice(data);
    output
}

pub const MESSAGE_MAX_SIZE: usize = 4 * 1024;
pub const MESSAGE_KEY_SIZE: usize = 64;
enum MessageStage {
    Type,
    Key,
    Value,
}

pub struct MessageOutbound {
    stdout: StdioPacket,
    stdio_packet_buffer: Option<Box<[u8; MESSAGE_MAX_SIZE]>>,
    cmdtype: MessageType,
    /// increment this value by 1 for every message
    nonce: u32,
    key: Box<[u8; MESSAGE_KEY_SIZE]>,
    key_size: usize,
    value: Box<[u8; MESSAGE_MAX_SIZE]>,
    value_size: usize,
}
impl Default for MessageOutbound {
    fn default() -> Self {
        Self {
            stdout: StdioPacket::stdout(),
            nonce: 0,
            cmdtype: MessageType::Pair,
            key: Box::new([0u8; MESSAGE_KEY_SIZE]),
            key_size: 0,
            value: Box::new([0u8; MESSAGE_MAX_SIZE]),
            value_size: 0,
            stdio_packet_buffer: Some(Box::new([0u8; MESSAGE_MAX_SIZE])),
        }
    }
}
impl MessageOutbound {
    pub fn write<M: MessageSerializer>(&mut self, message: MessageSend<M>) {
        log_debug!("MessageOutboud::write - nonce {}", self.nonce);
        match message {
            MessageSend::Wallet(address) => {
                self.cmd(MessageType::Pubkey);
                let p_len = std::mem::size_of::<Pubkey>();
                {
                    let key = self.key_export(1).unwrap();
                    key[0] = 1;
                }
                {
                    let value = self.value_export(p_len).unwrap();
                    let inbuf = as_bytes(&address);
                    value.copy_from_slice(inbuf);
                }
                log_debug!(
                    "MessageOutboud::write - nonce {}; pubkey {}",
                    self.nonce,
                    address
                );
            }
            MessageSend::Pong(t) => {
                self.cmd(MessageType::Pong);
                {
                    let key = self.key_export(1).unwrap();
                    key[0] = 1;
                }
                let z = match t.duration_since(UNIX_EPOCH) {
                    Ok(n) => n.as_secs(),
                    Err(_) => panic!("SystemTime before UNIX EPOCH!"),
                };
                let x: [u8; 8] = z.to_le_bytes();
                {
                    let value = self.value_export(8).unwrap();
                    value.copy_from_slice(&x);
                }
            }
            MessageSend::Custom(c) => {
                self.cmd(MessageType::Custom);
                {
                    let key = self.key_export(1).unwrap();
                    key[0] = 1;
                }
                {
                    let value = self.value_export(c.len()).unwrap();
                    c.serialize(value);
                }
            }
        }
        self.save();
    }

    pub fn flush(&mut self) {
        self.stdout.flush();
    }

    fn cmd(&mut self, cmd: MessageType) {
        self.cmdtype = cmd;
    }
    fn key_export(&mut self, size: usize) -> Option<&mut [u8]> {
        self.key_size = size;
        Some(&mut self.key[0..size])
    }

    fn value_export(&mut self, size: usize) -> Option<&mut [u8]> {
        self.value_size = size;
        Some(&mut self.value[0..size])
    }

    fn save(&mut self) {
        let mut outbound_buffer = self.stdio_packet_buffer.take().unwrap();
        assert!(0 < self.key_size);
        // [CMD,1B][version,4B][key size, 1B][key, ?B][value size, 2B][value, ?B]
        let mut index = 0;
        outbound_buffer[index] = self.cmdtype.msg_type();
        index += 1;
        {
            let outbuf = &mut outbound_buffer[index..(index + 4)];
            index += 4;
            let inbuf = self.nonce.to_le_bytes();
            outbuf.copy_from_slice(&inbuf);
        }
        outbound_buffer[index] = self.key_size as u8;
        index += 1;
        {
            let outbuf = &mut outbound_buffer[index..(index + self.key_size)];
            index += self.key_size;
            outbuf.copy_from_slice(&self.key[0..self.key_size]);
        }
        {
            let outbuf = &mut outbound_buffer[index..(index + 2)];
            index += 2;
            let vz = self.value_size as u16;
            let x = vz.to_le_bytes();
            outbuf.copy_from_slice(&x);
        }
        {
            let outbuf = &mut outbound_buffer[index..(index + self.value_size)];
            index += self.value_size;
            outbuf.copy_from_slice(&self.value[0..self.value_size]);
        }

        // clear out current values
        self.key_size = 0;
        self.value_size = 0;
        log_debug!("send - nonce {}", self.nonce);
        self.nonce += 1;
        self.stdout.append(&outbound_buffer[0..index]);
        self.stdio_packet_buffer.replace(outbound_buffer);
    }
}

pub struct MessageInbound {
    nonce: u32,
    cmdtype: MessageType,
    key: Box<[u8; MESSAGE_KEY_SIZE]>,
    key_size: usize,
    value: Box<[u8; MESSAGE_MAX_SIZE]>,
    value_size: usize,
}

impl Default for MessageInbound {
    fn default() -> Self {
        Self {
            nonce: 0,
            cmdtype: MessageType::Pair,
            key: Box::new([0u8; MESSAGE_KEY_SIZE]),
            key_size: 0,
            value: Box::new([0u8; MESSAGE_MAX_SIZE]),
            value_size: 0,
        }
    }
}

impl MessageInbound {
    pub fn msg_type(&self) -> &MessageType {
        &self.cmdtype
    }
    pub fn key(&self) -> &[u8] {
        &self.key[0..self.key_size]
    }
    pub fn value(&self) -> &[u8] {
        &self.value[0..self.value_size]
    }
    // [CMD,1B][version,4B][key size, 1B][key, ?B][value size, 2B][value, ?B]
    pub fn parse(&mut self, data: &[u8]) -> Result<(), CatscopeGuestError> {
        //log_debug!("stdin parse - nonce {}", self.nonce);
        if data.len() <= 1 + 4 + 1 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let mut index = 0;
        self.cmdtype = data[index].try_into()?;
        index += 1;
        let check_nonce = {
            let subbuf = &data[index..(index + 4)];
            index += 4;
            let y: [u8; 4] = subbuf.try_into().unwrap();
            u32::from_le_bytes(y)
        };
        if check_nonce != self.nonce {
            return Err(CatscopeGuestError::BadNonce(check_nonce, self.nonce));
        }
        self.nonce += 1;
        self.key_size = data[index] as usize;
        index += 1;
        if data.len() - index <= self.key_size {
            return Err(CatscopeGuestError::InsufficientBuffer);
        } else {
            let subbuf = &data[index..(index + self.key_size)];
            index += self.key_size;
            let buf = &mut self.key[0..self.key_size];
            buf.copy_from_slice(subbuf);
        }
        if data.len() - index < 2 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        self.value_size = {
            let subbuf = &data[index..(index + 2)];
            index += 2;
            let y: [u8; 2] = subbuf.try_into().unwrap();
            u16::from_le_bytes(y) as usize
        };
        if data.len() - index < self.value_size {
            return Err(CatscopeGuestError::InsufficientBuffer);
        } else {
            let subbuf = &data[index..(index + self.value_size)];
            //index += self.value_size;
            let buf = &mut self.value[0..self.value_size];
            buf.copy_from_slice(subbuf);
        }
        Ok(())
    }
}

pub struct MessageHandlerV2 {}

impl MessageHandlerV2 {
    pub fn new() -> Self {
        Self {}
    }
}
pub struct MessageHandler {
    i: usize,
    index: usize,
    t2: usize,
    t2_key: MessageKey,
    buffer: Box<[u8; MESSAGE_MAX_SIZE]>,
    stage: MessageStage,
    m_map: HashMap<MessageKey, Vec<u8>>,
    l_id: Vec<MessageKey>,
}
impl MessageHandler {
    pub fn new() -> Self {
        Self {
            i: 0,
            l_id: Vec::new(),
            index: 0,
            t2: 0,
            t2_key: [0u8; MESSAGE_KEY_SIZE],
            buffer: Box::new([0u8; MESSAGE_MAX_SIZE]),
            stage: MessageStage::Type,
            m_map: HashMap::default(),
        }
    }
    pub fn get(&self, key: &MessageKey) -> Option<&Vec<u8>> {
        self.m_map.get(key)
    }
    pub fn message(&mut self) -> Option<(&MessageKey, &[u8])> {
        if self.l_id.len() <= self.i {
            return None;
        }
        let key: &MessageKey = &self.l_id[self.i];
        let value = self.m_map.get(key).unwrap();
        Some((key, value))
    }
    pub fn on_data(&mut self, _data: &[u8]) {}
}

pub enum MessageType {
    // outbound: sending back a key value pair
    Pair,
    // outbound; dumping the whole key/value store
    Dump,
    // outbound
    Custom,
    Pong,
    Pubkey,
}
impl TryFrom<u8> for MessageType {
    type Error = CatscopeGuestError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(MessageType::Pair),
            2 => Ok(MessageType::Dump),
            3 => Ok(MessageType::Pong),
            4 => Ok(MessageType::Custom),
            5 => Ok(MessageType::Pubkey),
            _ => Err(CatscopeGuestError::BufferTooSmall),
        }
    }
}

impl MessageType {
    pub fn msg_type(&self) -> u8 {
        match self {
            MessageType::Pair => 1,
            MessageType::Dump => 2,
            MessageType::Pong => 3,
            MessageType::Custom => 4,
            MessageType::Pubkey => 5,
        }
    }
}
