use std::{cell::UnsafeCell, rc::Rc, sync::OnceLock, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

pub fn log_level() -> LogLevel {
    static LEVEL: OnceLock<LogLevel> = OnceLock::new();
    *LEVEL.get_or_init(|| match std::env::var("LOG_LEVEL").as_deref() {
        Ok("DEBUG") => LogLevel::Debug,
        Ok("WARN") => LogLevel::Warn,
        Ok("ERROR") => LogLevel::Error,
        _ => LogLevel::Info,
    })
}

pub fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub struct PacketHolder {
    pub p: StdioPacket,
}

impl PacketHolder {
    pub fn log(&mut self, data: &[u8]) {
        self.p.append(data);
    }
}

struct SyncCell(UnsafeCell<PacketHolder>);
unsafe impl Sync for SyncCell {}
unsafe impl Send for SyncCell {}

static GLOBAL_PACKET: OnceLock<SyncCell> = OnceLock::new();

pub fn packet_holder() -> &'static mut PacketHolder {
    let cell = GLOBAL_PACKET.get_or_init(|| {
        SyncCell(UnsafeCell::new(PacketHolder {
            p: StdioPacket::stderr(),
        }))
    });
    unsafe { &mut *cell.0.get() }
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Debug {
            let msg = format!("[DEBUG] [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Info {
            let msg = format!("[INFO]  [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Warn {
            let msg = format!("[WARN]  [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Error {
            let msg = format!("[ERROR] [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

use crate::{catscope::witbot::shooter, graph::AccountId, stdio::StdioPacket};
use solana_sdk::pubkey::Pubkey;

pub fn pubkey_from_account_id(account_id: &AccountId) -> Option<Pubkey> {
    let data = match shooter::pubkey_map_by_id(*account_id) {
        Ok(x) => x,
        Err(_) => return None,
    };
    let y: [u8; 32] = data.try_into().unwrap();
    Some(Pubkey::from(y))
}

pub fn account_id_from_pubkey(pubkey: &Pubkey) -> AccountId {
    match shooter::pubkey_map_by_pubkey(pubkey.as_array()) {
        Ok(x) => x,
        Err(e) => panic!("failed to get account_id {e}"),
    }
}

#[inline]
pub fn as_bytes_mut<T: Sized>(val: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut((val as *mut T) as *mut u8, std::mem::size_of::<T>()) }
}

#[inline]
pub fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T) as *const u8, std::mem::size_of::<T>()) }
}

#[inline]
pub fn rc_unlock_mut<'a, 'b: 'a, T>(object: &'b Rc<UnsafeCell<T>>) -> &'a mut T {
    unsafe { &mut *object.get() }
}

#[inline]
pub fn rc_unlock<'a, 'b: 'a, T>(object: &'b Rc<UnsafeCell<T>>) -> &'a T {
    unsafe { &*object.get() }
}
