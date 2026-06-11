use crate::{
    catscope::witbot::{
        shooter::{Header, Tokenaccountv1},
        transactionprocessor,
    },
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Lamports, Subscription},
    token::TokenDatabase,
    tx::ComputeUnit,
    util::{account_id_from_pubkey, pubkey_from_account_id, rc_unlock},
};
use bincode;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    hash::Hash,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use solana_sdk_ids::system_program::ID as SystemProgramID;
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};
pub struct Wallet {
    m_key: HashMap<AccountId, SignerStatus>,
    q_ix: VecDeque<Instruction>,
    compute: ComputeUnit,
    token: TokenDatabase,
    sys_id: AccountId,
    tx_data: Box<[u8; 4 * 1024]>,
    payer: Option<AccountId>,
    m_cache_pubkey: HashMap<AccountId, Pubkey>,
    hs_required: HashSet<AccountId>,
}

struct SignerStatus {
    key: Rc<UnsafeCell<Keypair>>,
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
            m_cache_pubkey: HashMap::default(),
            payer: None,
            token: TokenDatabase::default(),
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
        keypair: Rc<UnsafeCell<Keypair>>,
        g1: &mut Graph,
    ) -> Result<AccountId, CatscopeGuestError> {
        let key = rc_unlock(&keypair);
        let pubkey = key.pubkey();
        let signer = account_id_from_pubkey(&pubkey);
        // get SOL and token accounts
        let sub = g1.subscribe(crate::graph::SubscriptionRequest {
            root: signer,
            filter_weight: u32::MAX,
            depth: 2,
        })?;

        self.m_key.insert(
            signer,
            SignerStatus {
                key: keypair,
                header: Header {
                    slot: 0,
                    version: 0,
                    lamports: 0,
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
        let ss = self.m_key.get(&payer_id)?;
        let keypair = rc_unlock(&ss.key);
        Some(keypair.pubkey())
    }

    /// Update the signer system account status (includes SOL balance).
    pub fn on_token(&mut self, a: &Tokenaccountv1, is_final: bool) -> bool {
        let x = self.m_key.contains_key(&a.owner);
        if x {
            self.token.on_token(a, is_final);
        }
        x
    }

    pub fn token(&self) -> &TokenDatabase {
        &self.token
    }
    pub fn token_mut(&mut self) -> &mut TokenDatabase {
        &mut self.token
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

    pub fn balance_sol(&self, signer_account_id: &AccountId) -> Option<Lamports> {
        let ss = self.m_key.get(signer_account_id)?;
        Some(ss.header.lamports)
    }
    fn pubkey_from_account_id(&mut self, account_id: &AccountId) -> Option<Pubkey> {
        if let Some(pubkey) = self.m_cache_pubkey.get(account_id) {
            Some(*pubkey)
        } else {
            let pubkey = pubkey_from_account_id(account_id)?;
            self.m_cache_pubkey.insert(*account_id, pubkey);
            Some(pubkey)
        }
    }
    /// Derive the ATA for `owner`+`mint`, append a `CreateIdempotent` instruction,
    /// and return the ATA pubkey. Returns `None` if no payer is set.
    pub fn append_create_ata(&mut self, owner: AccountId, mint: AccountId) -> Option<AccountId> {
        let owner_pubkey = self.pubkey_from_account_id(&owner)?;
        let mint_pubkey = self.pubkey_from_account_id(&mint)?;
        let ix = make_ata_instruction(&owner_pubkey, &owner_pubkey, &mint_pubkey);
        let ata_address: Pubkey = get_associated_token_address(&owner_pubkey, &mint_pubkey);
        self.require_signer(owner);
        self.append_ix(ix, 5_000);
        Some(account_id_from_pubkey(&ata_address))
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
        let mut l_keypair = Vec::with_capacity(self.m_key.len());
        for (_, ss) in self.m_key.iter() {
            let keypair = rc_unlock(&ss.key);
            l_keypair.push(keypair);
        }
        let ss = self.m_key.get(self.payer.as_ref().unwrap()).unwrap();
        let keypair = rc_unlock(&ss.key);
        let signer_pubkey = keypair.pubkey();
        let mut tx =
            Transaction::new_signed_with_payer(&l_ix, Some(&signer_pubkey), &l_keypair, blockhash);
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

fn make_ata_instruction(payer: &Pubkey, wallet_owner: &Pubkey, token_mint: &Pubkey) -> Instruction {
    create_associated_token_account_idempotent(
        payer,
        wallet_owner,
        token_mint,
        &spl_token::ID,
    )
}
