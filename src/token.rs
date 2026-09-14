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
    ///
    /// Real, live-confirmed bug fixed here (2026-09-03): the old fallback
    /// (`is_final=false` and `self.buffer.is_empty()`) couldn't tell "the
    /// fast (`is_final=false`) low-latency stream has never seen this
    /// (owner, mint) pair yet" apart from "the fast stream has seen it
    /// and the real, fresh balance is exactly zero" -- both leave the
    /// result buffer empty (a zero-amount entry is never pushed into it,
    /// a few lines below), so both used to fall back to the rooted
    /// (~12s+ lag) map, which can still be holding a stale *nonzero*
    /// balance from before the real change was rooted. Confirmed live: a
    /// real dispersion close-pass kept re-selling a position for a full
    /// resync cycle after the sell that emptied it had already finalized
    /// on-chain, because this fallback discarded the fast stream's
    /// already-correct zero and substituted the stale rooted value.
    /// Fixed by checking real presence (`contains_key`) instead of
    /// result-buffer emptiness to decide whether the fast stream
    /// genuinely has no data yet.
    pub fn balance(
        &mut self,
        owner: &AccountId,
        mint: &AccountId,
        is_final: bool,
    ) -> &[(AccountId, u64)] {
        self.buffer.clear();
        let use_root = is_final
            || !self.m_processed_owner.get(owner).is_some_and(|ton| ton.m_token.contains_key(mint));
        let m_root = if use_root { &self.m_root_owner } else { &self.m_processed_owner };
        if let Some(ton) = m_root.get(owner) {
            if let Some(m_token) = ton.m_token.get(mint) {
                for (account_id, amount) in m_token.iter() {
                    if 0 < *amount {
                        self.buffer.push((*account_id, *amount));
                    }
                }
            }
        }
        self.buffer.sort_unstable_by_key(|&(_, amount)| amount);
        &self.buffer
    }

    /// Forget everything this database thinks it knows about `mint` for
    /// `owner`, on both the fast and rooted maps.
    ///
    /// Real motivation (2026-09-03): a hop-chain send can genuinely land
    /// on-chain (changing this exact mint's real balance) while its
    /// `mid_on_tx` confirmation never arrives (see `HOP_CHAIN_SIGNATURE_
    /// EXPIRY_SLOTS`'s own doc comment) -- when that tracked signature
    /// expires, whichever balance this database is currently holding for
    /// the mint that send was expected to produce is provably unverified:
    /// it might be correct, or it might be exactly the kind of stale
    /// value `balance`'s own 2026-09-03 fix was written to stop trusting.
    /// Since there's no way to know which without a real update this
    /// process may never receive, the safe move is to forget the old
    /// value entirely rather than keep letting a caller retry against a
    /// number that's no longer trustworthy -- the next real update
    /// (whichever stream delivers it first) repopulates it properly.
    pub fn invalidate(&mut self, owner: &AccountId, mint: &AccountId) {
        if let Some(ton) = self.m_processed_owner.get_mut(owner) {
            ton.m_token.remove(mint);
        }
        if let Some(ton) = self.m_root_owner.get_mut(owner) {
            ton.m_token.remove(mint);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: AccountId, owner: AccountId, mint: AccountId, amount: u64) -> Tokenaccountv1 {
        Tokenaccountv1 { id, owner, mint, amount, slot: 0, version: 0 }
    }

    #[test]
    fn processed_balance_is_visible_immediately() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), false);
        assert_eq!(db.balance(&10, &20, false), &[(1, 500)]);
    }

    #[test]
    fn processed_query_falls_back_to_rooted_when_never_seen_on_the_fast_stream() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), true);
        assert_eq!(db.balance(&10, &20, false), &[(1, 500)]);
    }

    #[test]
    fn a_real_fresh_zero_on_the_fast_stream_is_not_shadowed_by_a_stale_rooted_balance() {
        // Real, live-confirmed bug this guards against (2026-09-03): a
        // stale rooted balance must never resurface once the fast stream
        // has genuinely reported the real, current balance is zero --
        // see `TokenDatabase::balance`'s own doc comment.
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), true); // stale rooted balance
        db.on_token(&token(1, 10, 20, 0), false); // fast stream: real sell landed
        assert_eq!(db.balance(&10, &20, false), &[] as &[(AccountId, u64)]);
    }

    #[test]
    fn final_query_never_sees_the_fast_stream_even_when_present() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), true);
        db.on_token(&token(1, 10, 20, 0), false);
        // is_final=true intentionally only ever trusts the rooted map.
        assert_eq!(db.balance(&10, &20, true), &[(1, 500)]);
    }

    #[test]
    fn invalidate_clears_both_the_fast_and_rooted_balance() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), true);
        db.on_token(&token(1, 10, 20, 500), false);
        db.invalidate(&10, &20);
        assert!(db.balance(&10, &20, false).is_empty());
        assert!(db.balance(&10, &20, true).is_empty());
    }

    #[test]
    fn invalidate_only_touches_the_given_owner_and_mint() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), false);
        db.on_token(&token(2, 10, 21, 300), false);
        db.on_token(&token(3, 11, 20, 700), false);
        db.invalidate(&10, &20);
        assert!(db.balance(&10, &20, false).is_empty());
        assert_eq!(db.balance(&10, &21, false), &[(2, 300)]);
        assert_eq!(db.balance(&11, &20, false), &[(3, 700)]);
    }

    #[test]
    fn unknown_owner_or_mint_returns_empty() {
        let mut db = TokenDatabase::default();
        db.on_token(&token(1, 10, 20, 500), false);
        assert!(db.balance(&11, &20, false).is_empty());
        assert!(db.balance(&10, &21, false).is_empty());
    }
}
