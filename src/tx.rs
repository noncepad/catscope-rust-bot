use crate::catscope::witbot::transactionprocessor::{self};
use crate::err::CatscopeGuestError;
use bincode::config::Configuration as BincodeConfiguration;
use solana_sdk::message::Instruction;
use solana_sdk::transaction::VersionedTransaction;

/// Send out transactions with this struct.
#[derive(Clone)]
pub struct TransactionSender {
    buffer: Box<[u8; TX_MAX]>,
    config: BincodeConfiguration,
}
impl Default for TransactionSender {
    fn default() -> Self {
        Self {
            buffer: Box::new([0u8; TX_MAX]),
            config: bincode::config::standard(),
        }
    }
}
impl TransactionSender {
    /// Send a single transaction out
    pub fn send(&mut self, tx: &VersionedTransaction) -> Result<(), CatscopeGuestError> {
        let signature = match tx.signatures.first() {
            Some(x) => x,
            None => return Err(CatscopeGuestError::TransactionParse),
        };
        let tx_size =
            bincode::serde::encode_into_slice(tx, self.buffer.as_mut_slice(), self.config).unwrap();
        match transactionprocessor::send(signature.as_array(), &self.buffer[0..tx_size]) {
            Ok(_) => Ok(()),
            Err(code) => Err(CatscopeGuestError::Transaction(code)),
        }
    }
}

const TX_MAX: usize = 4 * 1024;

pub type ComputeUnit = u32;
pub trait IxBuilder {
    fn build(&self) -> (Instruction, ComputeUnit);
}
