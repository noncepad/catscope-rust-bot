use std::collections::HashMap;

use crate::{
    catscope::witbot::shooter::Tokenaccountv1,
    graph::{AccountId, TokenAmount},
};

#[derive(Default)]
pub struct TokenDatabase {
    /// owner -> id
    m_owner: HashMap<AccountId, MintNode>,
}
impl TokenDatabase {
    /// update the token balance
    pub fn on_token(&mut self, account: &Tokenaccountv1) {
        let ton = self.m_owner.entry(account.owner).or_default();
        let m_token = ton.m_token.entry(account.mint).or_default();
        m_token.insert(account.id, account.amount);
    }
    /// get the balance
    pub fn balance(&self, owner: &AccountId, mint: &AccountId) -> TokenAmount {
        let mut balance = 0;
        if let Some(ton) = self.m_owner.get(owner) {
            if let Some(m_token) = ton.m_token.get(mint) {
                for (_, amount) in m_token.iter() {
                    balance += amount;
                }
            }
        }
        balance
    }
}

#[derive(Default)]
struct MintNode {
    interim_balance: TokenAmount,
    /// mint -> id -> balance
    m_token: HashMap<AccountId, HashMap<AccountId, TokenAmount>>,
}
