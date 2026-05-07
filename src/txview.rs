use crate::{err::CatscopeGuestError, graph::AccountId};
use solana_sdk::{
    clock::Slot, instruction::InstructionError, signature::SIGNATURE_BYTES,
    transaction::TransactionError,
};
use wincode::{SchemaRead, SchemaWrite};

pub struct TransactionList {
    i: usize,
    data: Vec<u8>,
    border: Vec<u32>,
}

impl TransactionList {
    pub(crate) fn new(data: Vec<u8>, border: Vec<u32>) -> Self {
        Self { data, border, i: 0 }
    }
    pub fn transaction<'a, 'b: 'a>(
        &'b mut self,
    ) -> Option<(
        CatscopeTransactionReadWrapper<'a>,
        Result<Slot, TransactionError>,
    )> {
        let i = self.i;
        if self.border.len() <= i {
            return None;
        }
        self.i += 1;
        let finish = self.border[i] as usize;
        let start = (if i == 0 { 0 } else { self.border[i - 1] }) as usize;
        let start_tx = start + 16;
        let r = {
            let subbuf = &self.data[start..start_tx];
            let z: [u8; 16] = subbuf.try_into().unwrap();
            result_from_bytes(&z).unwrap()
        };

        let tx_slice = &self.data[start_tx..finish];
        let tx = CatscopeTransactionReadWrapper::try_from(tx_slice).unwrap_or_else(|e| {
            let hdr_size = std::mem::size_of::<CatscopeTransactionHeader>();
            let hdr_info = if tx_slice.len() >= hdr_size {
                let outer_len = u32::from_le_bytes(tx_slice[72..76].try_into().unwrap());
                let inner_len = u32::from_le_bytes(tx_slice[76..80].try_into().unwrap());
                let account_len = u32::from_le_bytes(tx_slice[80..84].try_into().unwrap());
                let account_chunk_len = u32::from_le_bytes(tx_slice[84..88].try_into().unwrap());
                let data_chunk_len = u32::from_le_bytes(tx_slice[88..92].try_into().unwrap());
                let ix_count = outer_len.saturating_add(inner_len);
                let expected = (outer_len as usize) * 21
                    + (inner_len as usize) * 20
                    + (account_len as usize) * 8
                    + (account_chunk_len as usize) * 8
                    + (data_chunk_len as usize);
                // account_chunk_last + data_chunk_last: ix_count * 2 each
                let expected2 = expected + (ix_count as usize) * 4;
                format!(
                    "outer={outer_len} inner={inner_len} account={account_len} \
                     account_chunk={account_chunk_len} data_chunk={data_chunk_len} \
                     ix_count={ix_count} expected_payload={expected2} available={}",
                    tx_slice.len() - hdr_size
                )
            } else {
                format!(
                    "slice too small for header: {} < {hdr_size}",
                    tx_slice.len()
                )
            };
            panic!(
                "tx parse failed ({e:?}): start={start} start_tx={start_tx} finish={finish} \
                 data_len={} | {hdr_info}",
                self.data.len()
            )
        });
        Some((tx, r))
    }
}

/// Zero-copy representation of a Solana transaction.
#[derive(SchemaWrite, SchemaRead)]
#[repr(C, align(64))]
pub struct CatscopeTransaction {
    /// Transaction signature
    pub signature: [u8; SIGNATURE_BYTES],

    /// Index assigned by the runtime.
    pub index: u64,

    data_chunk_last: Vec<u16>,
    data_chunk: Vec<u8>,

    account_chunk_last: Vec<u16>,
    account_chunk: Vec<AccountId>,

    /// Top-level instructions.
    outer: Vec<CatscopeInstruction>,

    /// Per-outer-instruction inner instruction counts.
    l1_inner: Vec<u8>,

    /// Flattened inner instruction list.
    inner: Vec<CatscopeInstruction>,

    /// Sorted list of account IDs touched by this transaction.
    account: Vec<AccountId>,
}

/// A single CatScope instrcution
///
/// This is used to record what was executed during transaction processing,
/// both for instructions explicitly submitted by the client and instructions
/// invoked indirectly during execution.
#[derive(Clone, Copy, Default, SchemaWrite, SchemaRead)]
#[repr(C, align(8))]
struct CatscopeInstruction {
    /// AccountId of the program that executed this instruction.
    program: AccountId,
    data_chunk_i: u16,
    account_chunk_i: u16,
}

enum IxIndex {
    Outer(usize),
    Inner(usize),
}
pub struct CatscopeInstructionRead<'a> {
    i: IxIndex,
    tx: &'a CatscopeTransaction,
}

impl<'a> CatscopeInstructionRead<'a> {
    /// Returns the program that executed this instruction.
    #[inline]
    pub fn program(&self) -> &AccountId {
        match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i].program,
            IxIndex::Inner(i) => &self.tx.inner[i].program,
        }
    }

    /// Return the raw instruction data used during execution.
    pub fn data(&self) -> &[u8] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i],
            IxIndex::Inner(i) => &self.tx.inner[i],
        };
        let chunk_i = ix.data_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.tx.data_chunk_last[chunk_i])
        } else {
            (
                self.tx.data_chunk_last[chunk_i - 1],
                self.tx.data_chunk_last[chunk_i],
            )
        };
        &self.tx.data_chunk[(start as usize)..(finish as usize)]
    }

    /// Returns the accounts referenced by this instruction.
    pub fn account(&self) -> &[AccountId] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i],
            IxIndex::Inner(i) => &self.tx.inner[i],
        };
        let chunk_i = ix.account_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.tx.account_chunk_last[chunk_i])
        } else {
            (
                self.tx.account_chunk_last[chunk_i - 1],
                self.tx.account_chunk_last[chunk_i],
            )
        };
        &self.tx.account_chunk[(start as usize)..(finish as usize)]
    }
}

// ---------------------------------------------------------------------------
// Result<Slot, TransactionError> — 16-byte encoding
// ---------------------------------------------------------------------------
//
//  [0]     result_tag:  0 = Ok,  1 = Err
//  [1]     tx_disc:     TransactionError variant index (0-based, enum declaration order)
//  [2]     ie_disc:     InstructionError variant index (only when tx_disc == 8)
//  [3]     aux_u8:      • InstructionError(u8, _)               → instruction index
//                       • DuplicateInstruction(u8)              → instruction index
//                       • InsufficientFundsForRent{account_index}
//                       • ProgramExecutionTemporarilyRestricted{account_index}
//  [4..8]  custom_u32:  InstructionError::Custom(u32) payload, little-endian
//  [8..16] slot:        Ok(Slot) value, little-endian

/// Encode `Result<Slot, TransactionError>` into exactly 16 bytes.

/// Decode `Result<Slot, TransactionError>` from 16 bytes produced by [`result_to_bytes`].
pub fn result_from_bytes(
    buf: &[u8; 16],
) -> Result<Result<Slot, TransactionError>, CatscopeGuestError> {
    match buf[0] {
        0 => {
            let slot = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            Ok(Ok(slot))
        }
        1 => {
            let tx_disc = buf[1];
            let ie_disc = buf[2];
            let aux = buf[3];
            let custom = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            Ok(Err(decode_tx_error(tx_disc, aux, ie_disc, custom)?))
        }
        _ => Err(CatscopeGuestError::InsufficientBuffer),
    }
}
fn decode_tx_error(
    tx_disc: u8,
    aux: u8,
    ie_disc: u8,
    custom: u32,
) -> Result<TransactionError, CatscopeGuestError> {
    let e = match tx_disc {
        0 => TransactionError::AccountInUse,
        1 => TransactionError::AccountLoadedTwice,
        2 => TransactionError::AccountNotFound,
        3 => TransactionError::ProgramAccountNotFound,
        4 => TransactionError::InsufficientFundsForFee,
        5 => TransactionError::InvalidAccountForFee,
        6 => TransactionError::AlreadyProcessed,
        7 => TransactionError::BlockhashNotFound,
        8 => TransactionError::InstructionError(aux, decode_instruction_error(ie_disc, custom)?),
        9 => TransactionError::CallChainTooDeep,
        10 => TransactionError::MissingSignatureForFee,
        11 => TransactionError::InvalidAccountIndex,
        12 => TransactionError::SignatureFailure,
        13 => TransactionError::InvalidProgramForExecution,
        14 => TransactionError::SanitizeFailure,
        15 => TransactionError::ClusterMaintenance,
        16 => TransactionError::AccountBorrowOutstanding,
        17 => TransactionError::WouldExceedMaxBlockCostLimit,
        18 => TransactionError::UnsupportedVersion,
        19 => TransactionError::InvalidWritableAccount,
        20 => TransactionError::WouldExceedMaxAccountCostLimit,
        21 => TransactionError::WouldExceedAccountDataBlockLimit,
        22 => TransactionError::TooManyAccountLocks,
        23 => TransactionError::AddressLookupTableNotFound,
        24 => TransactionError::InvalidAddressLookupTableOwner,
        25 => TransactionError::InvalidAddressLookupTableData,
        26 => TransactionError::InvalidAddressLookupTableIndex,
        27 => TransactionError::InvalidRentPayingAccount,
        28 => TransactionError::WouldExceedMaxVoteCostLimit,
        29 => TransactionError::WouldExceedAccountDataTotalLimit,
        30 => TransactionError::DuplicateInstruction(aux),
        31 => TransactionError::InsufficientFundsForRent { account_index: aux },
        32 => TransactionError::MaxLoadedAccountsDataSizeExceeded,
        33 => TransactionError::InvalidLoadedAccountsDataSizeLimit,
        34 => TransactionError::ResanitizationNeeded,
        35 => TransactionError::ProgramExecutionTemporarilyRestricted { account_index: aux },
        36 => TransactionError::UnbalancedTransaction,
        37 => TransactionError::ProgramCacheHitMaxLimit,
        38 => TransactionError::CommitCancelled,
        _ => return Err(CatscopeGuestError::InsufficientBuffer),
    };
    Ok(e)
}

fn decode_instruction_error(
    ie_disc: u8,
    custom: u32,
) -> Result<InstructionError, CatscopeGuestError> {
    let ie = match ie_disc {
        0 => InstructionError::GenericError,
        1 => InstructionError::InvalidArgument,
        2 => InstructionError::InvalidInstructionData,
        3 => InstructionError::InvalidAccountData,
        4 => InstructionError::AccountDataTooSmall,
        5 => InstructionError::InsufficientFunds,
        6 => InstructionError::IncorrectProgramId,
        7 => InstructionError::MissingRequiredSignature,
        8 => InstructionError::AccountAlreadyInitialized,
        9 => InstructionError::UninitializedAccount,
        10 => InstructionError::UnbalancedInstruction,
        11 => InstructionError::ModifiedProgramId,
        12 => InstructionError::ExternalAccountLamportSpend,
        13 => InstructionError::ExternalAccountDataModified,
        14 => InstructionError::ReadonlyLamportChange,
        15 => InstructionError::ReadonlyDataModified,
        16 => InstructionError::DuplicateAccountIndex,
        17 => InstructionError::ExecutableModified,
        18 => InstructionError::RentEpochModified,
        19 => InstructionError::NotEnoughAccountKeys,
        20 => InstructionError::AccountDataSizeChanged,
        21 => InstructionError::AccountNotExecutable,
        22 => InstructionError::AccountBorrowFailed,
        23 => InstructionError::AccountBorrowOutstanding,
        24 => InstructionError::DuplicateAccountOutOfSync,
        25 => InstructionError::Custom(custom),
        26 => InstructionError::InvalidError,
        27 => InstructionError::ExecutableDataModified,
        28 => InstructionError::ExecutableLamportChange,
        29 => InstructionError::ExecutableAccountNotRentExempt,
        30 => InstructionError::UnsupportedProgramId,
        31 => InstructionError::CallDepth,
        32 => InstructionError::MissingAccount,
        33 => InstructionError::ReentrancyNotAllowed,
        34 => InstructionError::MaxSeedLengthExceeded,
        35 => InstructionError::InvalidSeeds,
        36 => InstructionError::InvalidRealloc,
        37 => InstructionError::ComputationalBudgetExceeded,
        38 => InstructionError::PrivilegeEscalation,
        39 => InstructionError::ProgramEnvironmentSetupFailure,
        40 => InstructionError::ProgramFailedToComplete,
        41 => InstructionError::ProgramFailedToCompile,
        42 => InstructionError::Immutable,
        43 => InstructionError::IncorrectAuthority,
        44 => InstructionError::BorshIoError,
        45 => InstructionError::AccountNotRentExempt,
        46 => InstructionError::InvalidAccountOwner,
        47 => InstructionError::ArithmeticOverflow,
        48 => InstructionError::UnsupportedSysvar,
        49 => InstructionError::IllegalOwner,
        50 => InstructionError::MaxAccountsDataAllocationsExceeded,
        51 => InstructionError::MaxAccountsExceeded,
        52 => InstructionError::MaxInstructionTraceLengthExceeded,
        53 => InstructionError::BuiltinProgramsMustConsumeComputeUnits,
        _ => return Err(CatscopeGuestError::InsufficientBuffer),
    };
    Ok(ie)
}

#[repr(C)]
struct CatscopeTransactionHeader {
    signature: [u8; SIGNATURE_BYTES], // offset  0, 64 bytes
    index: u64,                       // offset 64
    outer_len: u32,                   // offset 72
    inner_len: u32,                   // offset 76
    account_len: u32,                 // offset 80
    account_chunk_len: u32,           // offset 84
    data_chunk_len: u32,              // offset 88
    _pad: u32,                        // offset 92 → total 96, align 8
}

/// Zero-copy view of a serialized [`CatscopeTransaction`].
///
/// Constructed via `TryFrom<&'a [u8]>` — no allocation, all fields are
/// slices into the original buffer.
pub struct CatscopeTransactionReadWrapper<'a> {
    pub signature: &'a [u8; SIGNATURE_BYTES],
    pub index: u64,
    pub account: &'a [AccountId],
    pub l1_inner: &'a [u8],
    pub data_chunk: &'a [u8],
    outer: &'a [CatscopeInstruction],
    inner: &'a [CatscopeInstruction],
    account_chunk: &'a [AccountId],
    account_chunk_last: &'a [u16],
    data_chunk_last: &'a [u16],
}

impl<'a> TryFrom<&'a [u8]> for CatscopeTransactionReadWrapper<'a> {
    type Error = CatscopeGuestError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        use std::mem::{align_of, size_of};

        // --- helper: advance `offset` by `count` elements of type T,
        //     returning a typed slice aliasing `data` ---
        unsafe fn take_slice<'b, T>(
            data: &'b [u8],
            offset: &mut usize,
            count: usize,
        ) -> Result<&'b [T], CatscopeGuestError> {
            let byte_len = count
                .checked_mul(size_of::<T>())
                .ok_or(CatscopeGuestError::InsufficientBuffer)?;
            let start = *offset;
            let end = start
                .checked_add(byte_len)
                .ok_or(CatscopeGuestError::InsufficientBuffer)?;
            if end > data.len() {
                return Err(CatscopeGuestError::InsufficientBuffer);
            }
            let ptr = data[start..end].as_ptr() as *const T;
            if (ptr as usize) % align_of::<T>() != 0 {
                return Err(CatscopeGuestError::InsufficientBuffer);
            }
            *offset = end;
            Ok(std::slice::from_raw_parts(ptr, count))
        }

        let hdr_size = std::mem::size_of::<CatscopeTransactionHeader>();
        if data.len() < hdr_size {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }

        // SAFETY: header is repr(C) with all valid bit patterns; we checked
        // both the length and alignment of the buffer.
        let hdr = unsafe {
            let ptr = data.as_ptr() as *const CatscopeTransactionHeader;
            if (ptr as usize) % std::mem::align_of::<CatscopeTransactionHeader>() != 0 {
                return Err(CatscopeGuestError::UnalignedMemory);
            }
            &*ptr
        };

        let outer_len = hdr.outer_len as usize;
        let inner_len = hdr.inner_len as usize;
        let account_len = hdr.account_len as usize;
        let account_chunk_len = hdr.account_chunk_len as usize;
        let data_chunk_len = hdr.data_chunk_len as usize;
        let ix_count = outer_len
            .checked_add(inner_len)
            .ok_or(CatscopeGuestError::InsufficientBuffer)?;

        let mut off = hdr_size;
        // SAFETY: each take_slice call checks bounds and alignment.
        unsafe {
            let outer = take_slice::<CatscopeInstruction>(data, &mut off, outer_len)?;
            let inner = take_slice::<CatscopeInstruction>(data, &mut off, inner_len)?;
            let account = take_slice::<AccountId>(data, &mut off, account_len)?;
            let account_chunk = take_slice::<AccountId>(data, &mut off, account_chunk_len)?;
            let account_chunk_last = take_slice::<u16>(data, &mut off, ix_count)?;
            let data_chunk_last = take_slice::<u16>(data, &mut off, ix_count)?;
            let l1_inner = take_slice::<u8>(data, &mut off, outer_len)?;
            let data_chunk = take_slice::<u8>(data, &mut off, data_chunk_len)?;

            // signature lives inside the header, which is borrowed from `data`
            let signature = &*(hdr.signature.as_ptr() as *const [u8; SIGNATURE_BYTES]);

            Ok(Self {
                signature,
                index: hdr.index,
                outer,
                inner,
                account,
                account_chunk,
                account_chunk_last,
                data_chunk_last,
                l1_inner,
                data_chunk,
            })
        }
    }
}
