use crate::{catscope::witbot::general, message::MessageSerializer};
use solana_sdk::pubkey::Pubkey;
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

/// Cumulative time spent inside `general::stdout` (the real host stdio
/// write) since the last [`take_flush_stats`] call, across every channel
/// (stdout message writes *and* the log macros' own stderr writes both
/// go through [`StdioPacket::flush`]). Real, live-observed motivation:
/// every death this session left its last log line looking completely
/// unremarkable -- no bookend ever caught the moment of the freeze, even
/// after every known blocking host call (subscribe/bulk_subscribe) was
/// paced and bookended. A log call's *own* internal buffer-overflow
/// flush is a host write neither `log_warn!` nor any caller can bookend
/// (the flush happens *inside* the call trying to log something), so if
/// it's ever slow under pipe backpressure, the very message meant to
/// report it is what's lost. This is a passive counter instead of a
/// log-on-entry for exactly that reason -- reported later via the
/// existing gap-since-previous-start log, the same "accumulate, don't
/// log every call" pattern already used for `low_latency`/`evaluate`.
static FLUSH_ELAPSED_NANOS: AtomicU64 = AtomicU64::new(0);
static FLUSH_COUNT: AtomicU32 = AtomicU32::new(0);

/// Drains and returns the accumulated stdio-flush time/count since the
/// last call -- see [`FLUSH_ELAPSED_NANOS`]'s doc comment.
pub fn take_flush_stats() -> (std::time::Duration, u32) {
    let nanos = FLUSH_ELAPSED_NANOS.swap(0, Ordering::Relaxed);
    let count = FLUSH_COUNT.swap(0, Ordering::Relaxed);
    (std::time::Duration::from_nanos(nanos), count)
}

/// Create a fixed size data packet to send out on stdout or stderr.
pub struct StdioPacket {
    pub(crate) size: usize,
    pub(crate) channel: u8,
    pub(crate) buffer: Box<[u8; STDIO_PACKET_BUFFER_MAX]>,
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
        Self {
            channel,
            size: 0,
            buffer,
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
        //assert_eq!(self.channel, 1);
        let t0 = std::time::Instant::now();
        general::stdout(self.channel, &self.buffer[0..self.size]);
        FLUSH_ELAPSED_NANOS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        FLUSH_COUNT.fetch_add(1, Ordering::Relaxed);
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
