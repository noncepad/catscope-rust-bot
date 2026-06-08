use crate::{
    catscope::witbot::shooter::Tokenaccountv1,
    graph::{AccountId, TokenAmount},
};
use std::collections::HashMap;

#[derive(Default)]
pub struct TokenDatabase {
    buffer: Vec<(AccountId, u64)>,
    /// owner -> id
    m_root_owner: HashMap<AccountId, MintNode>,
    /// owner -> id
    m_processed_owner: HashMap<AccountId, MintNode>,
}
impl TokenDatabase {
    /// update the token balance
    pub fn on_token(&mut self, account: &Tokenaccountv1, is_final: bool) {
        let ton = if is_final {
            self.m_root_owner.entry(account.owner).or_default()
        } else {
            self.m_processed_owner.entry(account.owner).or_default()
        };

        let m_token = ton.m_token.entry(account.mint).or_default();
        m_token.insert(account.id, account.amount);
    }
    /// get the balance
    pub fn balance(
        &mut self,
        owner: &AccountId,
        mint: &AccountId,
        is_final: bool,
    ) -> &[(AccountId, u64)] {
        let mut balance = 0;
        self.buffer.clear();
        let m_root = if is_final {
            &self.m_root_owner
        } else {
            &self.m_processed_owner
        };
        if let Some(ton) = m_root.get(owner) {
            if let Some(m_token) = ton.m_token.get(mint) {
                for (account_id, amount) in m_token.iter() {
                    balance += amount;
                    if 0 < *amount {
                        self.buffer.push((*account_id, *amount));
                    }
                }
            }
        }
        if !is_final && self.buffer.is_empty() {
            if let Some(ton) = self.m_root_owner.get(owner) {
                if let Some(m_token) = ton.m_token.get(mint) {
                    for (account_id, amount) in m_token.iter() {
                        balance += amount;
                        if 0 < *amount {
                            self.buffer.push((*account_id, *amount));
                        }
                    }
                }
            }
        }
        self.buffer.sort_unstable_by_key(|&(_, amount)| amount);
        &self.buffer
    }
}

#[derive(Debug)]
pub struct TradeAmount {
    pub mint: AccountId,
    pub amount: u64,
}

#[derive(Debug)]
pub struct PairAmount {
    // in
    pub a_leg: TradeAmount,
    // out
    pub b_leg: TradeAmount,
}

impl PairAmount {
    /// `price` = mint_a raw units per mint_b raw unit.
    /// `volume_b` = mint_b raw units to trade.
    /// Derives mint_a amount as `volume_b * price`.
    pub fn new(price: f64, volume_b: u64, mint_a: AccountId, mint_b: AccountId) -> Self {
        assert_ne!(mint_a, mint_b);
        Self {
            a_leg: TradeAmount {
                mint: mint_a,
                amount: (volume_b as f64 * price) as u64,
            },
            b_leg: TradeAmount {
                mint: mint_b,
                amount: volume_b,
            },
        }
    }

    #[inline]
    pub fn lookup_id(&self) -> [AccountId; 2] {
        let mut id = [self.a_leg.mint, self.b_leg.mint];
        id.sort();
        id
    }
}

#[derive(Default)]
struct MintNode {
    interim_balance: TokenAmount,
    /// mint -> id -> balance
    m_token: HashMap<AccountId, HashMap<AccountId, TokenAmount>>,
}
