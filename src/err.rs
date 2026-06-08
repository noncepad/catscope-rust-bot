use crate::catscope::witbot::general::ErrorCode as GeneralErrorCode;
use crate::catscope::witbot::shooter::ErrorCode as ShooterErrorCode;
use crate::catscope::witbot::transactionprocessor::ErrorCode as TransactionErrorCode;
use crate::crypt::CryptError;
use crate::graph::AccountId;
use crate::trader::types::TraderError;

/// Error types for the Catscope bot system
///
/// Represents all possible error conditions that can occur during bot execution
#[derive(Debug)]
pub enum CatscopeGuestError {
    InsufficientBuffer,
    InsufficientBufferV2(usize, usize),
    // u32 that increments on every message sent and received
    BadNonce(u32, u32),
    /// Unknown or unclassified error
    Unknown(String),
    Shooter(ShooterErrorCode),
    General(GeneralErrorCode),
    Transaction(TransactionErrorCode),
    MissingEvent(u32),
    ProtocolVersionMismatch,
    MissingPool(AccountId, AccountId),
    BadMessageCommand,
    TransactionParse,
    UnalignedMemory,
    InvalidPrivateKey,
    Crypt(CryptError),
    Trade(TraderError),
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
            Self::InvalidPrivateKey => {
                write!(f, "invalid private key")
            }
            Self::Trade(e) => {
                write!(f, "trade error: {e}")
            }
            Self::MissingPool(a, b) => {
                write!(f, "missing pool {a} {b}")
            }
            Self::BadNonce(actual, expected) => {
                write!(f, "bad nonce; got {actual}, but expected {expected}")
            }
            Self::InsufficientBuffer => write!(f, "Insufficient buffer"),
            Self::InsufficientBufferV2(needs, has) => {
                write!(f, "Insufficient buffer; needs {needs}; has {has}")
            }
            CatscopeGuestError::Crypt(e) => write!(f, "Crypt: {e}"),
            CatscopeGuestError::Unknown(code) => write!(f, "Unknown: {code}"),
            CatscopeGuestError::Shooter(code) => write!(f, "Shooter: {code}"),
            CatscopeGuestError::General(code) => write!(f, "General: {}", general_code(code)),
            CatscopeGuestError::Transaction(code) => write!(f, "Transaction: {code}",),
            CatscopeGuestError::MissingEvent(event_id) => write!(f, "Missing event {event_id}",),
            CatscopeGuestError::ProtocolVersionMismatch => write!(f, "protocol version mismatch",),
            CatscopeGuestError::BadMessageCommand => write!(f, "bad message command",),
            CatscopeGuestError::TransactionParse => write!(f, "Transaction parse",),
        }
    }
}
impl From<TraderError> for CatscopeGuestError {
    fn from(value: TraderError) -> Self {
        CatscopeGuestError::Trade(value)
    }
}
impl From<CryptError> for CatscopeGuestError {
    fn from(value: CryptError) -> Self {
        CatscopeGuestError::Crypt(value)
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
