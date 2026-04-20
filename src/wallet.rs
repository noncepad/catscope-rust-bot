use crate::{
    catscope::witbot::{shooter::Header, transactionprocessor},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription},
    log_debug,
    tx::ComputeUnit,
    util::account_id_from_pubkey,
};
use bincode;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    hash::Hash,
    message::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use solana_sdk_ids::system_program::ID as SystemProgramID;
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    u32,
};

pub struct Wallet {
    m_key: HashMap<AccountId, SignerStatus>,
    q_ix: VecDeque<Instruction>,
    compute: ComputeUnit,
    sys_id: AccountId,
    tx_data: Box<[u8; 4 * 1024]>,
    payer: Option<AccountId>,
    hs_required: HashSet<AccountId>,
}

struct SignerStatus {
    key: Keypair,
    header: Header,
    sub: Subscription,
}

impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}

impl Wallet {
    pub fn new() -> Self {
        Self {
            payer: None,
            tx_data: Box::new([0u8; 4 * 1024]),
            m_key: HashMap::default(),
            q_ix: VecDeque::default(),
            compute: 0,
            sys_id: account_id_from_pubkey(&SystemProgramID),
            hs_required: HashSet::new(),
        }
    }
    pub fn require_signer(&mut self, account_id: AccountId) {
        self.hs_required.insert(account_id);
    }
    pub fn set_payer(&mut self, payer: AccountId) {
        self.payer = Some(payer);
    }

    /// Add a private key to the wallet.
    /// This also includes doing a graph subscription to get Lamport updates.
    pub fn append_key(
        &mut self,
        key: Keypair,
        g1: &mut Graph,
    ) -> Result<AccountId, CatscopeGuestError> {
        let pubkey = key.pubkey();
        let signer = account_id_from_pubkey(&pubkey);
        let sub = g1.subscribe(signer, u32::MAX, 1)?;
        self.m_key.insert(
            signer,
            SignerStatus {
                key,
                header: Header {
                    slot: 0,
                    version: 0,
                    lamports: 0,
                    rentepoch: 0,
                    accountid: 0,
                    owner: 0,
                    datasize: 0,
                },
                sub,
            },
        );
        Ok(signer)
    }
    pub fn has_key(&self, l_account_id: &[AccountId]) -> Option<&Header> {
        for account_id in l_account_id {
            if let Some(ss) = self.m_key.get(account_id) {
                return Some(&ss.header);
            }
        }
        None
    }

    pub fn payer_pubkey(&self) -> Option<Pubkey> {
        let payer_id = self.payer?;
        Some(self.m_key.get(&payer_id)?.key.pubkey())
    }

    /// Update the signer system account status (includes SOL balance).
    pub fn on_account(&mut self, header: &Header) -> bool {
        if header.owner != self.sys_id {
            return false;
        }
        if let Some(status) = self.m_key.get_mut(&header.accountid) {
            status.header = *header;
            true
        } else {
            false
        }
    }

    /// Append an instruction.
    pub fn append_ix(&mut self, ix: Instruction, compute: ComputeUnit) {
        self.compute += compute;
        self.q_ix.push_back(ix);
    }

    /// Get the current compute unit for currently appended instructions.
    pub fn cu(&self) -> ComputeUnit {
        self.compute
    }

    /// Assemble and export transactions based on currently appended instructions.
    pub fn assemble(&mut self) -> Option<(Signature, &[u8])> {
        if self.q_ix.is_empty() {
            return None;
        }
        let blockhash;
        {
            let bh: [u8; 32] = transactionprocessor::blockhash()
                .unwrap()
                .try_into()
                .unwrap();
            blockhash = Hash::from(bh);
        }
        let mut l_ix = Vec::with_capacity(10);
        if 0 < self.compute {
            let ix_cu = ComputeBudgetInstruction::set_compute_unit_limit(self.compute);
            l_ix.push(ix_cu);
        }
        while let Some(ix) = self.q_ix.pop_front() {
            l_ix.push(ix);
        }
        self.compute = 0;
        //let _payer = self.payer.as_ref().unwrap();
        let ss = self.m_key.get(self.payer.as_ref().unwrap()).unwrap();
        let keypair = &ss.key;
        let signer_pubkey = keypair.pubkey();
        let mut tx =
            Transaction::new_signed_with_payer(&l_ix, Some(&signer_pubkey), &[keypair], blockhash);
        let signature = *tx.signatures.first().unwrap();
        let size = bincode::serde::encode_into_slice(
            &mut tx,
            &mut self.tx_data[0..],
            bincode::config::standard(),
        )
        .unwrap();

        Some((signature, &self.tx_data[0..size]))
    }
}
