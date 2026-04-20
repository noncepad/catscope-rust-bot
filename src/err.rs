use crate::catscope::witbot::general::ErrorCode as GeneralErrorCode;
use crate::catscope::witbot::shooter::ErrorCode as ShooterErrorCode;
use crate::catscope::witbot::transactionprocessor::ErrorCode as TransactionErrorCode;

/// Error types for the Catscope bot system
///
/// Represents all possible error conditions that can occur during bot execution
#[derive(Debug)]
pub enum CatscopeGuestError {
    InsufficientBuffer,
    // u32 that increments on every message sent and received
    BadNonce(u32, u32),
    /// Unknown or unclassified error
    Unknown(String),
    Shooter(ShooterErrorCode),
    General(GeneralErrorCode),
    Transaction(TransactionErrorCode),
    MissingEvent(u32),
    BufferTooSmall,
    TransactionParse,
    UnalignedMemory,
}

fn general_code(code: &GeneralErrorCode) -> usize {
    match code {
        GeneralErrorCode::Unknown => 1,
        GeneralErrorCode::Timeout => 2,
    }
}

impl std::fmt::Display for CatscopeGuestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnalignedMemory => {
                write!(f, "unaligned memory")
            }
            Self::BadNonce(actual, expected) => {
                write!(f, "bad nonce; got {actual}, but expected {expected}")
            }
            Self::InsufficientBuffer => write!(f, "Insufficient buffer"),
            CatscopeGuestError::Unknown(code) => write!(f, "Unknown: {code}"),
            CatscopeGuestError::Shooter(code) => write!(f, "Shooter: {code}"),
            CatscopeGuestError::General(code) => write!(f, "General: {}", general_code(code)),
            CatscopeGuestError::Transaction(code) => write!(f, "Transaction: {code}",),
            CatscopeGuestError::MissingEvent(event_id) => write!(f, "Missing event {event_id}",),
            CatscopeGuestError::BufferTooSmall => write!(f, "Buffer too small",),
            CatscopeGuestError::TransactionParse => write!(f, "Transaction parse",),
        }
    }
}

impl From<ShooterErrorCode> for CatscopeGuestError {
    fn from(value: ShooterErrorCode) -> Self {
        CatscopeGuestError::Shooter(value)
    }
}

impl From<GeneralErrorCode> for CatscopeGuestError {
    fn from(value: GeneralErrorCode) -> Self {
        CatscopeGuestError::General(value)
    }
}
